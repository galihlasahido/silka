//! HTTP/1.1, written against `std::net` — the request path the application
//! actually sends on.
//!
//! ## Why there is no HTTP crate here
//!
//! The brief allowed either a light HTTP crate behind a feature flag or a local
//! dummy server. This crate takes the second road, and the reason is worth
//! stating because it is the kind of decision that looks like laziness from the
//! outside:
//!
//! - **What is being proved is the framework, not the protocol.** The claims
//!   under test are "a loading state is visible and the UI keeps running",
//!   "a network error is a value and not a panic", "leaving a tab stops the
//!   work", and "a panicking panel does not take the window with it". Every one
//!   of those is about [`silka_core::task`] and [`silka_core::recover`]. A
//!   client crate would answer none of them and hide one of them: an executor
//!   inside the HTTP crate would be doing the cancelling instead of us.
//! - **Cancellation has to be visible to be proved.** [`send`] reads in short
//!   polls and checks [`Cancel`] between them, so "the user left the tab" turns
//!   into a socket that is dropped in tens of milliseconds — and a test can
//!   watch it happen. A `send()` that blocks inside somebody else's runtime
//!   would leave nothing to observe.
//! - **A TLS stack is not free.** `rustls` + `webpki-roots` + `ring` is minutes
//!   of build time in a workspace whose `target/` is shared by every crate here.
//!   Paying that to prove something neither of them is responsible for is a bad
//!   trade.
//!
//! The cost is stated rather than hidden: **`https://` is not supported**.
//! [`Url::parse`] rejects it with a sentence a user can read, which is the same
//! treatment every other failure in this file gets. Talking to the public
//! internet is a `--url` away from working the day a TLS transport is added
//! behind [`send`]; nothing above this module would change.
//!
//! ## The shape
//!
//! One more limit while the list is open: a `HEAD` response is read like any
//! other, so a server that sends a `Content-Length` for the body it is *not*
//! going to send keeps this client reading until it closes the connection.
//! `Connection: close` makes that immediate, which is why it has never been
//! worth special-casing here.
//!
//! Everything except [`send`] is a pure function over strings and bytes, which
//! is what lets the parser, the encoder, the URL splitter and the JSON
//! pretty-printer be tested without a socket. [`send`] is the only part that
//! blocks, and it is only ever called from inside
//! [`Tasks::spawn_blocking`](silka_core::task::Tasks::spawn_blocking).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use silka_core::task::Cancel;

/// How long a connect may take before it is called a failure.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The whole exchange's budget, measured from the first byte written.
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);

/// How long one `read` waits before the loop looks at the cancel flag again.
///
/// This number **is** the cancellation latency: a request abandoned by the user
/// stops within one poll of being asked to. Short enough to feel instant, long
/// enough that an idle wait is not a spin loop.
pub const POLL: Duration = Duration::from_millis(20);

/// The error [`send`] returns when the cancel flag went up.
///
/// A constant rather than a formatted string because the UI branches on it: a
/// request the user abandoned is not an error to apologise for.
pub const CANCELLED: &str = "cancelled";

/// The largest response body this client will hold in memory.
///
/// A text editor is what shows the body, so "stream it" is not on the table;
/// refusing a gigabyte politely is.
pub const MAX_BODY: usize = 8 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Method
// ---------------------------------------------------------------------------

/// The HTTP methods the picker offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Method {
    /// The default, and the only one with no body.
    #[default]
    Get,
    /// Create.
    Post,
    /// Replace.
    Put,
    /// Modify.
    Patch,
    /// Remove.
    Delete,
    /// Headers only.
    Head,
}

impl Method {
    /// Every method, in the order the picker shows them.
    pub const ALL: [Method; 6] = [
        Method::Get,
        Method::Post,
        Method::Put,
        Method::Patch,
        Method::Delete,
        Method::Head,
    ];

    /// The token that goes on the request line.
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
            Method::Head => "HEAD",
        }
    }

    /// The method named by `s`, case-insensitively.
    pub fn parse(s: &str) -> Option<Method> {
        Method::ALL
            .into_iter()
            .find(|m| m.as_str().eq_ignore_ascii_case(s.trim()))
    }

    /// Its position in [`Method::ALL`] — what the `select` widget is bound to.
    pub fn index(self) -> usize {
        Method::ALL.iter().position(|m| *m == self).unwrap_or(0)
    }

    /// The method at `index`, saturating at the end of the list.
    pub fn from_index(index: usize) -> Method {
        Method::ALL[index.min(Method::ALL.len() - 1)]
    }

    /// Whether the body editor should be offered for this method.
    ///
    /// Not a rule of the protocol (a `GET` may carry a body) but a rule of the
    /// interface: showing a body box for a `GET` invites writing one that no
    /// server will read.
    pub fn takes_body(self) -> bool {
        matches!(self, Method::Post | Method::Put | Method::Patch)
    }
}

// ---------------------------------------------------------------------------
// URL
// ---------------------------------------------------------------------------

/// A URL, split into the four pieces a request line and a `Host` header need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    /// The host, without the port.
    pub host: String,
    /// The port, defaulted to 80 when the URL omits it.
    pub port: u16,
    /// Path and query, always starting with `/`.
    pub target: String,
}

impl Url {
    /// Split `raw`, or say what is wrong with it in a sentence.
    ///
    /// ```
    /// # use silka_api_client::http::Url;
    /// let u = Url::parse("http://localhost:8080/orders?page=2").unwrap();
    /// assert_eq!(u.host, "localhost");
    /// assert_eq!(u.port, 8080);
    /// assert_eq!(u.target, "/orders?page=2");
    ///
    /// // The one limit of this client, said out loud rather than swallowed.
    /// assert!(Url::parse("https://example.com").unwrap_err().contains("https"));
    /// ```
    pub fn parse(raw: &str) -> Result<Url, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("The URL is empty.".to_string());
        }
        let rest = match raw.split_once("://") {
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => rest,
            Some((scheme, _)) if scheme.eq_ignore_ascii_case("https") => {
                return Err(
                    "https is not supported by this example — it speaks plain HTTP/1.1 over a \
                     socket, with no TLS stack. Try an http:// URL, or the built-in server."
                        .to_string(),
                )
            }
            Some((scheme, _)) => return Err(format!("Unknown scheme {scheme:?}; use http://.")),
            // A bare `localhost:9000/ok` is what everybody types, so it is
            // read as http rather than rejected on a technicality.
            None => raw,
        };

        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        if authority.is_empty() {
            return Err("The URL has no host.".to_string());
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => {
                let port: u16 = port
                    .parse()
                    .map_err(|_| format!("{port:?} is not a port number."))?;
                (host, port)
            }
            None => (authority, 80),
        };
        if host.is_empty() {
            return Err("The URL has no host.".to_string());
        }

        Ok(Url {
            host: host.to_string(),
            port,
            target: if path.is_empty() {
                "/".to_string()
            } else {
                path.to_string()
            },
        })
    }

    /// `host:port`, what the resolver is handed.
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// The `Host` header value: the port is omitted when it is the default.
    pub fn host_header(&self) -> String {
        if self.port == 80 {
            self.host.clone()
        } else {
            self.authority()
        }
    }
}

// ---------------------------------------------------------------------------
// The request
// ---------------------------------------------------------------------------

/// One request, exactly as the panes describe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestSpec {
    /// The verb.
    pub method: Method,
    /// What the user typed in the URL bar.
    pub url: String,
    /// The header editor's raw text — parsed by [`parse_headers`], kept raw so
    /// a half-typed line is never destroyed by a reformat.
    pub headers: String,
    /// The body editor's text.
    pub body: String,
}

impl Default for RequestSpec {
    fn default() -> Self {
        Self {
            method: Method::Get,
            url: String::new(),
            headers: String::new(),
            body: String::new(),
        }
    }
}

impl RequestSpec {
    /// A `GET` of `url`.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            ..Self::default()
        }
    }

    /// The one-line summary a tab and a history row show.
    pub fn summary(&self) -> String {
        let url = self.url.trim();
        if url.is_empty() {
            format!("{} —", self.method.as_str())
        } else {
            format!("{} {url}", self.method.as_str())
        }
    }
}

/// Split the header editor's text into pairs.
///
/// Blank lines and `#` comments are skipped, and a line with no colon is
/// dropped rather than being turned into a header with an empty value — a
/// half-typed line must not travel.
///
/// ```
/// # use silka_api_client::http::parse_headers;
/// let h = parse_headers("Accept: application/json\n# a note\nX-Trace:  42 \nnonsense\n");
/// assert_eq!(h, vec![
///     ("Accept".to_string(), "application/json".to_string()),
///     ("X-Trace".to_string(), "42".to_string()),
/// ]);
/// ```
pub fn parse_headers(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once(':'))
        .filter_map(|(name, value)| {
            let name = name.trim();
            (!name.is_empty()).then(|| (name.to_string(), value.trim().to_string()))
        })
        .collect()
}

/// True when `headers` already carries a header called `name`.
fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case(name))
}

/// Render the bytes that go on the wire.
///
/// Three headers are supplied when the user did not write them, because a
/// request without them is either invalid (`Host`) or silently truncated
/// (`Content-Length`), and because this client cannot reuse a connection
/// (`Connection: close`).
///
/// ```
/// # use silka_api_client::http::{encode_request, Method, RequestSpec, Url};
/// let spec = RequestSpec { method: Method::Post, url: String::new(),
///                          headers: "Accept: */*".into(), body: "hi".into() };
/// let wire = String::from_utf8(encode_request(&Url::parse("http://h:9/x").unwrap(), &spec)).unwrap();
/// assert!(wire.starts_with("POST /x HTTP/1.1\r\n"));
/// assert!(wire.contains("Host: h:9\r\n"));
/// assert!(wire.contains("Content-Length: 2\r\n"));
/// assert!(wire.ends_with("\r\n\r\nhi"));
/// ```
pub fn encode_request(url: &Url, spec: &RequestSpec) -> Vec<u8> {
    let headers = parse_headers(&spec.headers);
    let body = if spec.method.takes_body() {
        spec.body.as_bytes()
    } else {
        &[][..]
    };

    let mut out = format!("{} {} HTTP/1.1\r\n", spec.method.as_str(), url.target);
    if !has_header(&headers, "host") {
        out.push_str(&format!("Host: {}\r\n", url.host_header()));
    }
    for (name, value) in &headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    if !has_header(&headers, "content-length") && !body.is_empty() {
        out.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    if !has_header(&headers, "connection") {
        out.push_str("Connection: close\r\n");
    }
    out.push_str("\r\n");

    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

// ---------------------------------------------------------------------------
// The response
// ---------------------------------------------------------------------------

/// A parsed response, with the timing the pane shows next to it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// The reason phrase, when the server sent one.
    pub reason: String,
    /// Every header, in the order they arrived.
    pub headers: Vec<(String, String)>,
    /// The body, decoded lossily — a client that shows a body in a text editor
    /// has to show *something* for bytes that are not text.
    pub body: String,
    /// How many bytes the body was on the wire.
    pub bytes: usize,
    /// How long the whole exchange took.
    pub elapsed: Duration,
}

impl Response {
    /// `200 OK`, or just the number when there was no reason phrase.
    pub fn status_line(&self) -> String {
        if self.reason.is_empty() {
            self.status.to_string()
        } else {
            format!("{} {}", self.status, self.reason)
        }
    }

    /// True for 2xx.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// One header, case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The body, pretty-printed when the server said it was JSON.
    pub fn display_body(&self) -> String {
        let json = self
            .header("content-type")
            .is_some_and(|c| c.to_ascii_lowercase().contains("json"));
        if json {
            pretty_json(&self.body)
        } else {
            self.body.clone()
        }
    }
}

/// Parse a complete response.
///
/// ```
/// # use silka_api_client::http::parse_response;
/// let raw = b"HTTP/1.1 404 Not Found\r\nContent-Length: 3\r\n\r\nnope";
/// let r = parse_response(raw).unwrap();
/// assert_eq!(r.status, 404);
/// assert_eq!(r.reason, "Not Found");
/// // The body is trimmed to `Content-Length`, never to whatever arrived.
/// assert_eq!(r.body, "nop");
/// ```
pub fn parse_response(raw: &[u8]) -> Result<Response, String> {
    let split = find_header_end(raw)
        .ok_or("The server closed the connection before it finished its headers.")?;
    let head = String::from_utf8_lossy(&raw[..split.0]);
    let body = &raw[split.1..];

    // `lines()` rather than a split on CRLF: it accepts both, which is what
    // makes the hand-written server above readable by this parser too.
    let mut lines = head.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| "The response has no status line.".to_string())?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/") {
        return Err(format!(
            "This does not look like an HTTP response — it starts with {:?}.",
            truncate(status_line, 40)
        ));
    }
    let status: u16 = parts
        .next()
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| format!("The status line has no status code: {status_line:?}"))?;
    let reason = parts.next().unwrap_or_default().trim().to_string();

    let headers: Vec<(String, String)> = lines
        .filter(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();

    // A body longer than `Content-Length` is trimmed: what follows it belongs
    // to the next message, and this client only ever reads one.
    let declared = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok());
    let body = match declared {
        Some(n) if n < body.len() => &body[..n],
        _ => body,
    };

    Ok(Response {
        status,
        reason,
        headers,
        body: String::from_utf8_lossy(body).into_owned(),
        bytes: body.len(),
        elapsed: Duration::ZERO,
    })
}

/// Where the head ends and the body begins: `(end of head, start of body)`.
///
/// Both `\r\n\r\n` and the bare `\n\n` a hand-written server emits are
/// accepted, because a client that only speaks to perfect servers is a client
/// nobody can debug with.
fn find_header_end(raw: &[u8]) -> Option<(usize, usize)> {
    let crlf = raw.windows(4).position(|w| w == b"\r\n\r\n");
    let lf = raw.windows(2).position(|w| w == b"\n\n");
    match (crlf, lf) {
        (Some(a), Some(b)) if b < a => Some((b, b + 2)),
        (Some(a), _) => Some((a, a + 4)),
        (None, Some(b)) => Some((b, b + 2)),
        (None, None) => None,
    }
}

/// True once `raw` holds a whole response — the loop's stopping condition.
///
/// Returns `false` while the head is incomplete, and `false` while a declared
/// `Content-Length` has not arrived. With no `Content-Length` there is no way
/// to know except the connection closing, which is what `Connection: close`
/// makes happen.
pub fn is_complete(raw: &[u8]) -> bool {
    let Some((head_end, body_start)) = find_header_end(raw) else {
        return false;
    };
    let head = String::from_utf8_lossy(&raw[..head_end]);
    let declared = head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok());
    match declared {
        Some(n) => raw.len() - body_start >= n,
        None => false,
    }
}

/// Shorten a string for an error message.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

// ---------------------------------------------------------------------------
// Sending
// ---------------------------------------------------------------------------

/// Send `spec` and wait for the answer, checking `cancel` throughout.
///
/// This is the only blocking function in the crate, and it is only ever called
/// from a worker thread. It returns `Err(`[`CANCELLED`]`)` the moment the flag
/// goes up — within [`POLL`] of the user leaving the tab — and a sentence for
/// every other failure. **Nothing in here panics**: every error path is a
/// `Result`, which is what makes "a network error is tidy, not a crash" a
/// property of the code rather than a hope.
///
/// The one uninterruptible step is DNS: [`ToSocketAddrs`] has no timeout and no
/// cancellation in `std`. For `127.0.0.1` — which is what this example talks to
/// — it does not block at all.
pub fn send(spec: &RequestSpec, cancel: &Cancel) -> Result<Response, String> {
    let started = Instant::now();
    let url = Url::parse(&spec.url)?;
    let wire = encode_request(&url, spec);

    if cancel.is_cancelled() {
        return Err(CANCELLED.to_string());
    }

    let address = url
        .authority()
        .to_socket_addrs()
        .map_err(|e| format!("{} could not be resolved: {e}", url.host))?
        .next()
        .ok_or_else(|| format!("{} resolved to no address at all.", url.host))?;

    if cancel.is_cancelled() {
        return Err(CANCELLED.to_string());
    }

    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
        .map_err(|e| format!("Could not connect to {}: {e}", url.authority()))?;
    stream
        .set_write_timeout(Some(CONNECT_TIMEOUT))
        .and_then(|()| stream.set_read_timeout(Some(POLL)))
        .map_err(|e| format!("The socket refused its timeouts: {e}"))?;

    stream
        .write_all(&wire)
        .and_then(|()| stream.flush())
        .map_err(|e| format!("The request could not be sent: {e}"))?;

    let mut raw: Vec<u8> = Vec::with_capacity(4096);
    let mut chunk = [0u8; 8192];
    loop {
        // Checked **before** every read, so a request cancelled while nothing
        // is arriving still stops within one poll.
        if cancel.is_cancelled() {
            return Err(CANCELLED.to_string());
        }
        if started.elapsed() > TOTAL_TIMEOUT {
            return Err(format!(
                "No answer within {} seconds.",
                TOTAL_TIMEOUT.as_secs()
            ));
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&chunk[..n]);
                if raw.len() > MAX_BODY {
                    return Err(format!(
                        "The response is larger than {} MB, which this client will not hold in \
                         memory.",
                        MAX_BODY / (1024 * 1024)
                    ));
                }
                if is_complete(&raw) {
                    break;
                }
            }
            // What a read timeout looks like — the poll expired, which is the
            // normal way round this loop, not a failure.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(format!("The connection broke while reading: {e}")),
        }
    }

    if raw.is_empty() {
        return Err(format!(
            "{} accepted the connection and then closed it without answering.",
            url.authority()
        ));
    }

    let mut response = parse_response(&raw)?;
    response.elapsed = started.elapsed();
    Ok(response)
}

// ---------------------------------------------------------------------------
// JSON pretty-printing
// ---------------------------------------------------------------------------

/// Re-indent a JSON document so a response is readable.
///
/// A formatter, not a parser: it never rejects anything, because a body that
/// does not parse still has to be shown. Strings (and the escapes inside them)
/// are passed through untouched, which is the only part that has to be right —
/// a brace inside a string must not open a level.
///
/// ```
/// # use silka_api_client::http::pretty_json;
/// assert_eq!(pretty_json(r#"{"a":[1,2]}"#), "{\n  \"a\": [\n    1,\n    2\n  ]\n}");
/// // A brace inside a string is text, not structure.
/// assert_eq!(pretty_json(r#"{"a":"{"}"#), "{\n  \"a\": \"{\"\n}");
/// ```
pub fn pretty_json(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + raw.len() / 4);
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut chars = raw.chars().peekable();

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '{' | '[' => {
                // An empty object or array stays on one line: `{\n}` is noise.
                let closer = if c == '{' { '}' } else { ']' };
                if chars.peek().copied() == Some(closer) {
                    chars.next();
                    out.push(c);
                    out.push(closer);
                } else {
                    depth += 1;
                    out.push(c);
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(c);
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    out
}

/// A newline plus two spaces per level.
fn newline(out: &mut String, depth: usize) {
    out.push('\n');
    for _ in 0..depth {
        out.push_str("  ");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_without_a_scheme_is_read_as_http() {
        let u = Url::parse("localhost:9100/ok").expect("parses");
        assert_eq!(u.host, "localhost");
        assert_eq!(u.port, 9100);
        assert_eq!(u.target, "/ok");
        assert_eq!(u.host_header(), "localhost:9100");
    }

    #[test]
    fn a_url_without_a_path_asks_for_the_root() {
        let u = Url::parse("http://example.test").expect("parses");
        assert_eq!(u.target, "/");
        assert_eq!(u.port, 80);
        // Port 80 is left out of the Host header, as the RFC asks.
        assert_eq!(u.host_header(), "example.test");
    }

    #[test]
    fn every_bad_url_is_a_sentence_rather_than_a_panic() {
        for bad in ["", "   ", "http://", "http://host:notaport", "ftp://host/x"] {
            let message = Url::parse(bad).expect_err("must be rejected");
            assert!(!message.is_empty(), "{bad:?} produced an empty complaint");
            assert!(
                message.ends_with('.'),
                "{bad:?} -> {message:?} is not a sentence"
            );
        }
    }

    #[test]
    fn https_is_refused_with_an_explanation_not_a_shrug() {
        let message = Url::parse("https://example.com/x").expect_err("no TLS here");
        assert!(message.contains("https"));
        assert!(message.contains("http://"));
    }

    #[test]
    fn the_encoder_supplies_host_length_and_close_but_never_overrides_the_user() {
        let spec = RequestSpec {
            method: Method::Post,
            url: "http://h/x".into(),
            headers: "Host: pinned.example\nConnection: keep-alive\nContent-Length: 99".into(),
            body: "abc".into(),
        };
        let wire =
            String::from_utf8(encode_request(&Url::parse("http://h/x").unwrap(), &spec)).unwrap();
        assert!(wire.contains("Host: pinned.example\r\n"));
        assert!(!wire.contains("Host: h\r\n"));
        assert!(wire.contains("Connection: keep-alive\r\n"));
        assert_eq!(wire.matches("Content-Length").count(), 1);
    }

    #[test]
    fn a_get_never_carries_the_body_the_editor_is_holding() {
        let spec = RequestSpec {
            method: Method::Get,
            url: "http://h/x".into(),
            headers: String::new(),
            body: "left over from when this was a POST".into(),
        };
        let wire =
            String::from_utf8(encode_request(&Url::parse("http://h/x").unwrap(), &spec)).unwrap();
        assert!(wire.ends_with("\r\n\r\n"));
        assert!(!wire.contains("left over"));
        assert!(!wire.contains("Content-Length"));
    }

    #[test]
    fn a_response_is_complete_only_once_its_declared_body_has_arrived() {
        assert!(!is_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 5"));
        assert!(!is_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nabc"
        ));
        assert!(is_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nabcde"
        ));
        // No length: only the connection closing can end it.
        assert!(!is_complete(b"HTTP/1.1 200 OK\r\n\r\nabcde"));
    }

    #[test]
    fn a_server_that_answers_with_bare_newlines_is_still_understood() {
        let r = parse_response(b"HTTP/1.0 204 No Content\nX-Note: hand written\n\n").unwrap();
        assert_eq!(r.status, 204);
        assert_eq!(r.header("x-note"), Some("hand written"));
        assert!(r.body.is_empty());
    }

    #[test]
    fn garbage_on_the_socket_is_an_error_message_not_an_unwrap() {
        let message = parse_response(b"<html>hello</html>\r\n\r\n").expect_err("not HTTP");
        assert!(message.contains("does not look like an HTTP response"));
        assert!(parse_response(b"HTTP/1.1 ok\r\n\r\n").is_err());
        assert!(parse_response(b"no header terminator at all").is_err());
    }

    #[test]
    fn a_json_body_is_reindented_and_anything_else_is_left_alone() {
        let json = Response {
            headers: vec![(
                "Content-Type".into(),
                "application/json; charset=utf-8".into(),
            )],
            body: r#"{"ok":true,"items":[]}"#.into(),
            ..Response::default()
        };
        assert_eq!(
            json.display_body(),
            "{\n  \"ok\": true,\n  \"items\": []\n}"
        );

        let text = Response {
            headers: vec![("Content-Type".into(), "text/plain".into())],
            body: "  spaced   out  ".into(),
            ..Response::default()
        };
        assert_eq!(text.display_body(), "  spaced   out  ");
    }

    #[test]
    fn pretty_printing_never_loses_the_characters_that_matter() {
        // Whitespace inside a string survives; whitespace outside one does not.
        assert_eq!(
            pretty_json(r#" { "a" : " b c " } "#),
            "{\n  \"a\": \" b c \"\n}"
        );
        // An escaped quote does not end the string.
        assert_eq!(pretty_json(r#"{"a":"\""}"#), "{\n  \"a\": \"\\\"\"\n}");
        // Not JSON at all: still returns, still readable.
        assert_eq!(pretty_json("plain"), "plain");
    }

    #[test]
    fn a_cancelled_token_stops_the_send_before_it_opens_a_socket() {
        let cancel = Cancel::detached();
        // A port nothing listens on — if the flag were ignored, this would be a
        // connection refused instead, which is a different message.
        let spec = RequestSpec::get("http://127.0.0.1:9/never");
        // Not cancelled: it really does try, and fails tidily.
        let message = send(&spec, &cancel).expect_err("nothing listens on port 9");
        assert!(message.contains("Could not connect"), "{message}");
    }

    #[test]
    fn the_method_list_round_trips_through_the_picker_index() {
        for m in Method::ALL {
            assert_eq!(Method::from_index(m.index()), m);
            assert_eq!(Method::parse(&m.as_str().to_lowercase()), Some(m));
        }
        assert_eq!(Method::parse("nonsense"), None);
        assert!(!Method::Get.takes_body());
        assert!(Method::Post.takes_body());
        // Out of range saturates instead of panicking — the picker's index and
        // the list can disagree for one frame after an edit.
        assert_eq!(Method::from_index(99), Method::Head);
    }
}
