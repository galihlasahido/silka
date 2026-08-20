//! The server this example talks to, running inside the same process.
//!
//! An HTTP client with nothing to call is a screenshot. Rather than depend on
//! the public internet — which is offline in CI, slow in a demo, and cannot be
//! asked to stall for exactly 1.5 seconds — the binary starts a loopback server
//! on `127.0.0.1` and points the sample requests at it. Roughly two hundred
//! lines of `std::net`, and it buys the three conditions the framework claims
//! to handle and that a real endpoint will not perform on request:
//!
//! | Route | What it exists to produce |
//! |---|---|
//! | `/slow?ms=N` | a **loading state** that lasts long enough to see, and a request worth cancelling |
//! | `/status/500` | an **error** that is a response rather than a socket failure |
//! | (a port with nothing on it) | a **network error**, which must be a sentence and not a panic |
//!
//! Each connection is served on its own thread, deliberately: a `/slow` request
//! that blocked the accept loop would make "the UI stays live while one request
//! is in flight" untestable, because the *server* would be the thing serialising
//! them.
//!
//! It is not a web server. There is no keep-alive, no chunked encoding, no
//! HEAD-body suppression beyond what the routes do themselves, and no security
//! of any kind — it binds loopback only, and that is the whole of its threat
//! model.

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// How long the accept loop sleeps between polls of the stop flag.
const ACCEPT_POLL: Duration = Duration::from_millis(5);

/// The step a `/slow` route sleeps in, so shutdown is never blocked behind a
/// pending nap.
const SLOW_STEP: Duration = Duration::from_millis(10);

/// The longest `/slow?ms=` this server will honour.
const SLOW_CAP: u64 = 60_000;

/// A running loopback server. Dropping it stops it.
#[derive(Debug)]
pub struct DummyServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    served: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}

impl DummyServer {
    /// Bind `127.0.0.1:port` and start serving; `0` takes any free port.
    ///
    /// ```
    /// # use silka_api_client::serve::DummyServer;
    /// let server = DummyServer::start(0).expect("loopback is available");
    /// assert!(server.base_url().starts_with("http://127.0.0.1:"));
    /// ```
    pub fn start(port: u16) -> std::io::Result<DummyServer> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;

        let stop = Arc::new(AtomicBool::new(false));
        let served = Arc::new(AtomicUsize::new(0));
        let loop_stop = stop.clone();
        let loop_served = served.clone();

        let thread = thread::Builder::new()
            .name("silka-api-client-server".to_string())
            .spawn(move || {
                while !loop_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let stop = loop_stop.clone();
                            let served = loop_served.clone();
                            // One thread per connection: a nap on `/slow` must
                            // not hold up the next request.
                            let _ = thread::Builder::new()
                                .name("silka-api-client-conn".to_string())
                                .spawn(move || {
                                    served.fetch_add(1, Ordering::Relaxed);
                                    handle(stream, &stop);
                                });
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {
                            thread::sleep(ACCEPT_POLL);
                        }
                        Err(_) => break,
                    }
                }
            })?;

        Ok(DummyServer {
            addr,
            stop,
            served,
            thread: Some(thread),
        })
    }

    /// Where it is listening.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The prefix every sample request is built from.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// How many connections have been accepted since it started.
    pub fn served(&self) -> usize {
        self.served.load(Ordering::Relaxed)
    }
}

impl Drop for DummyServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The accept loop polls rather than blocks, so nothing has to be poked
        // awake; joining just makes the shutdown observable.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// ---------------------------------------------------------------------------
// One connection
// ---------------------------------------------------------------------------

/// Read one request and answer it. Every failure closes the socket quietly:
/// this is a fixture, and a fixture that panics fails the wrong test.
fn handle(mut stream: TcpStream, stop: &AtomicBool) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Some((head, body)) = read_request(&mut stream) else {
        return;
    };

    let request_line = head.lines().next().unwrap_or_default().to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.clone(), String::new()),
    };

    let reply = route(&method, &path, &query, &head, &body, stop);
    let _ = stream.write_all(&reply);
    let _ = stream.flush();
    // A polite close: the client is reading until EOF when there is no
    // `Content-Length` it trusts.
    let _ = stream.shutdown(std::net::Shutdown::Write);
}

/// Read head and body off the socket. `None` when the client vanished first.
fn read_request(stream: &mut TcpStream) -> Option<(String, String)> {
    let mut raw: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    let split = loop {
        if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => raw.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
        if raw.len() > 1024 * 1024 {
            return None;
        }
    };

    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let declared = head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);

    let mut body = raw[split + 4..].to_vec();
    while body.len() < declared {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    // Anything past the declared length belongs to a request this fixture will
    // never read, because it closes the connection after one.
    body.truncate(declared);
    Some((head, String::from_utf8_lossy(&body).into_owned()))
}

/// The routing table.
fn route(
    method: &str,
    path: &str,
    query: &str,
    head: &str,
    body: &str,
    stop: &AtomicBool,
) -> Vec<u8> {
    match path {
        "/" => text(200, "OK", ROUTES),
        "/ok" => json(
            200,
            "OK",
            r#"{"ok":true,"service":"silka-api-client","routes":6}"#,
        ),
        "/slow" => {
            let ms = param(query, "ms")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(1_000)
                .min(SLOW_CAP);
            let mut waited = 0u64;
            while waited < ms {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(SLOW_STEP);
                waited += SLOW_STEP.as_millis() as u64;
            }
            json(200, "OK", &format!(r#"{{"slept_ms":{waited}}}"#))
        }
        "/echo" => {
            let lines = head.lines().count().saturating_sub(1);
            json(
                200,
                "OK",
                &format!(
                    r#"{{"method":"{}","header_count":{lines},"body_bytes":{},"body":{}}}"#,
                    escape(method),
                    body.len(),
                    quote(body)
                ),
            )
        }
        "/headers" => {
            let names: Vec<String> = head
                .lines()
                .skip(1)
                .filter_map(|line| line.split_once(':'))
                .map(|(name, _)| quote(name.trim()))
                .collect();
            json(
                200,
                "OK",
                &format!(r#"{{"received":[{}]}}"#, names.join(",")),
            )
        }
        p if p.starts_with("/status/") => {
            let code: u16 = p.trim_start_matches("/status/").parse().unwrap_or(500);
            let reason = reason_for(code);
            json(
                code,
                reason,
                &format!(r#"{{"error":"the server was asked for {code}","code":{code}}}"#),
            )
        }
        other => json(
            404,
            "Not Found",
            &format!(
                r#"{{"error":"no route","path":{},"try":"/ok"}}"#,
                quote(other)
            ),
        ),
    }
}

/// What `/` prints — the same list a person needs when driving the app by hand.
const ROUTES: &str = "silka-api-client test server\n\n\
     GET  /ok               a small JSON document\n\
     GET  /slow?ms=1500     answers after a delay\n\
     GET  /headers          echoes the header names it received\n\
     GET  /status/500       any status you name\n\
     POST /echo             echoes the body back\n";

/// One query parameter, undecoded — nothing here needs percent-decoding yet.
fn param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

/// A reason phrase for the codes the `/status/` route is asked for.
fn reason_for(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        418 => "I'm a teapot",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

/// A JSON response with the length filled in.
fn json(status: u16, reason: &str, body: &str) -> Vec<u8> {
    respond(status, reason, "application/json; charset=utf-8", body)
}

/// A plain-text response.
fn text(status: u16, reason: &str, body: &str) -> Vec<u8> {
    respond(status, reason, "text/plain; charset=utf-8", body)
}

fn respond(status: u16, reason: &str, content_type: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Escape a string for use inside a JSON string literal.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// [`escape`] with the quotes around it.
fn quote(s: &str) -> String {
    format!("\"{}\"", escape(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{self, Method, RequestSpec};
    use silka_core::task::Cancel;
    use std::time::Instant;

    /// A real request over a real socket — the client and the server proving
    /// each other, which is the only thing either of them is here for.
    #[test]
    fn a_request_goes_out_over_a_socket_and_a_response_comes_back() {
        let server = DummyServer::start(0).expect("bind");
        let spec = RequestSpec::get(format!("{}/ok", server.base_url()));
        let response = http::send(&spec, &Cancel::detached()).expect("a response");

        assert_eq!(response.status, 200);
        assert_eq!(response.reason, "OK");
        assert_eq!(
            response.header("content-type"),
            Some("application/json; charset=utf-8")
        );
        assert!(response.body.contains("silka-api-client"));
        // Pretty-printed, because the server said it was JSON.
        assert!(response.display_body().contains("\n  \"ok\": true"));
        assert_eq!(server.served(), 1);
    }

    #[test]
    fn a_post_body_arrives_intact() {
        let server = DummyServer::start(0).expect("bind");
        let spec = RequestSpec {
            method: Method::Post,
            url: format!("{}/echo", server.base_url()),
            headers: "Content-Type: application/json".into(),
            body: r#"{"amount": 250000}"#.into(),
        };
        let response = http::send(&spec, &Cancel::detached()).expect("a response");
        assert_eq!(response.status, 200);
        assert!(
            response.body.contains(r#""body_bytes":18"#),
            "{}",
            response.body
        );
        assert!(response.body.contains(r#""method":"POST""#));
    }

    #[test]
    fn a_500_is_a_response_and_not_a_failure() {
        let server = DummyServer::start(0).expect("bind");
        let spec = RequestSpec::get(format!("{}/status/503", server.base_url()));
        let response = http::send(&spec, &Cancel::detached()).expect("still a response");
        assert_eq!(response.status, 503);
        assert_eq!(response.reason, "Service Unavailable");
        assert!(!response.is_success());
    }

    /// The claim cancellation rests on: raising the flag stops the read loop
    /// within a poll, not when the server finally answers.
    #[test]
    fn raising_the_cancel_flag_ends_the_send_long_before_the_server_replies() {
        let server = DummyServer::start(0).expect("bind");
        let spec = RequestSpec::get(format!("{}/slow?ms=4000", server.base_url()));
        // The token comes from a real `Tasks`, because `Cancel` can only be
        // raised through a `TaskHandle` — which is the framework saying that
        // cancellation belongs to whoever spawned the work.
        let (tx, rx) = std::sync::mpsc::channel();
        let tasks = silka_core::task::Tasks::new();

        let started = Instant::now();
        let handle = tasks.spawn_blocking(
            move |cancel| {
                let _ = tx.send(http::send(&spec, cancel));
            },
            |()| {},
        );
        // Long enough that the request is certainly on the wire, short enough
        // that the server is nowhere near answering.
        thread::sleep(Duration::from_millis(60));
        handle.cancel();
        tasks.wait_for_idle();

        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the worker answered");
        let elapsed = started.elapsed();
        assert_eq!(result.unwrap_err(), http::CANCELLED);
        assert!(
            elapsed < Duration::from_millis(1_500),
            "cancelling took {elapsed:?}, which is not cancelling"
        );
    }

    #[test]
    fn an_unknown_route_is_a_404_with_a_readable_body() {
        let server = DummyServer::start(0).expect("bind");
        let spec = RequestSpec::get(format!("{}/nowhere", server.base_url()));
        let response = http::send(&spec, &Cancel::detached()).expect("a response");
        assert_eq!(response.status, 404);
        assert!(response.body.contains("no route"));
    }

    #[test]
    fn a_dropped_server_stops_listening() {
        let addr = {
            let server = DummyServer::start(0).expect("bind");
            server.addr()
        };
        // The port is free again, which is the observable half of shutdown.
        let spec = RequestSpec::get(format!("http://{addr}/ok"));
        assert!(http::send(&spec, &Cancel::detached()).is_err());
    }
}
