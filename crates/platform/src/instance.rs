//! Single instance, with argument forwarding (INTEGRASI-NATIVE §5).
//!
//! The behaviour every document-based application is expected to have and
//! almost none get for free: double-clicking a second file must open a second
//! *window*, not a second copy of the application. That means the second
//! process has to notice the first one, hand over its arguments, and exit.
//!
//! ## Why a loopback socket rather than a named pipe or D-Bus
//!
//! The three platform-native answers — a named pipe on Windows, a Unix socket
//! on macOS/Linux, `NSRunningApplication` or D-Bus activation — are three
//! separate implementations, two of which need bindings this workspace does not
//! pin. A TCP listener on `127.0.0.1` with a port written into a lock file is
//! **one** implementation in `std`, works identically on all three, and is
//! testable without a window or a display server.
//!
//! What that costs, stated plainly rather than buried: a loopback port is
//! reachable by any process running as **any** user on the machine. The lock
//! file therefore carries a token that a forwarding instance must present, and
//! is created `0600` on Unix. That makes it a same-user handshake, not a
//! security boundary — do not forward anything a hostile local process must not
//! see.
//!
//! ```no_run
//! use silka_platform::instance::{single_instance, InstanceRole};
//!
//! let instance = single_instance("Editor");
//! match instance.acquire()? {
//!     InstanceRole::Primary(listener) => {
//!         // …open the window, then once per frame:
//!         for args in listener.poll() {
//!             println!("another launch asked for {args:?}");
//!         }
//!     }
//!     // The first instance has our arguments; this process should exit.
//!     InstanceRole::Secondary => std::process::exit(0),
//! }
//! # Ok::<(), silka_platform::instance::InstanceError>(())
//! ```

use core::fmt;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::lifecycle::{state_path, HostOs};

/// How long a forwarding instance waits for the first one to answer.
///
/// Short on purpose: a second launch that hangs for seconds because the first
/// instance is busy feels like the application failed to start, and falling
/// back to "become the primary" is the better failure.
pub const FORWARD_TIMEOUT: Duration = Duration::from_millis(500);

/// Why the single-instance handshake could not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum InstanceError {
    /// There is nowhere to put the lock file — no `HOME`, no `APPDATA`.
    NoStateDirectory,
    /// The filesystem refused.
    Io(String),
}

impl fmt::Display for InstanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstanceError::NoStateDirectory => {
                write!(f, "no directory to keep the instance lock in")
            }
            InstanceError::Io(m) => write!(f, "the instance lock could not be used: {m}"),
        }
    }
}

impl std::error::Error for InstanceError {}

fn io(e: std::io::Error) -> InstanceError {
    InstanceError::Io(e.to_string())
}

/// Which of the two this process turned out to be.
#[derive(Debug)]
pub enum InstanceRole {
    /// The only instance. Owns the listener the others forward to.
    Primary(InstanceListener),
    /// Another instance was already running and has been handed our arguments.
    /// This process should exit.
    Secondary,
}

impl InstanceRole {
    /// Whether this process should carry on and open a window.
    pub fn is_primary(&self) -> bool {
        matches!(self, InstanceRole::Primary(_))
    }
}

/// The single-instance handshake for one application.
///
/// A plain value: naming it touches nothing, and only
/// [`SingleInstance::acquire`] opens a socket.
///
/// ```
/// use silka_platform::instance::single_instance;
///
/// let instance = single_instance("Editor").arguments(["--new"]);
/// assert_eq!(instance.arguments_to_forward(), ["--new"]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleInstance {
    app: String,
    args: Vec<String>,
    lock_path: Option<PathBuf>,
}

/// Describe the handshake for an application.
///
/// The arguments forwarded default to this process's own `argv` minus the
/// executable, which is what a second launch always wants to hand over.
pub fn single_instance(app: impl Into<String>) -> SingleInstance {
    SingleInstance {
        app: app.into(),
        args: std::env::args().skip(1).collect(),
        lock_path: None,
    }
}

impl SingleInstance {
    /// Forward these arguments instead of this process's own.
    pub fn arguments<S: Into<String>>(mut self, args: impl IntoIterator<Item = S>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// Keep the lock file somewhere specific — mainly so a test can point at a
    /// temporary directory instead of the user's real application-support
    /// folder.
    pub fn lock_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.lock_path = Some(path.into());
        self
    }

    /// The arguments that will be forwarded.
    pub fn arguments_to_forward(&self) -> &[String] {
        &self.args
    }

    /// Where the lock file goes.
    pub fn lock_path(&self) -> Result<PathBuf, InstanceError> {
        if let Some(path) = &self.lock_path {
            return Ok(path.clone());
        }
        instance_path(&self.app, HostOs::CURRENT, |k| std::env::var(k).ok())
            .ok_or(InstanceError::NoStateDirectory)
    }

    /// Become the primary instance, or hand the arguments to the one that
    /// already exists.
    ///
    /// A **stale** lock file — one left behind by a crash — is not an error:
    /// nothing answers on the recorded port, so this process takes over and
    /// rewrites it. That is the difference between an application that survives
    /// a crash and one that has to be un-stuck by deleting a file nobody can
    /// find.
    pub fn acquire(&self) -> Result<InstanceRole, InstanceError> {
        let path = self.lock_path()?;

        if let Some(lock) = read_lock(&path) {
            if forward(&lock, &self.args).is_ok() {
                return Ok(InstanceRole::Secondary);
            }
            // Nobody answered: the recorded instance is gone.
            let _ = std::fs::remove_file(&path);
        }

        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).map_err(io)?;
        listener.set_nonblocking(true).map_err(io)?;
        let port = listener.local_addr().map_err(io)?.port();
        let token = new_token();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        write_lock(
            &path,
            &Lock {
                port,
                token: token.clone(),
            },
        )?;

        Ok(InstanceRole::Primary(InstanceListener {
            listener,
            token,
            path,
        }))
    }
}

/// Where an application's instance lock lives.
///
/// Beside the session state ([`state_path`]), because they belong to the same
/// application and are cleaned up together.
///
/// ```
/// use std::path::PathBuf;
/// use silka_platform::instance::instance_path;
/// use silka_platform::lifecycle::HostOs;
///
/// let path = instance_path("Editor", HostOs::MacOs, |k| {
///     (k == "HOME").then(|| "/Users/ana".to_string())
/// });
/// assert_eq!(
///     path,
///     Some(PathBuf::from("/Users/ana/Library/Application Support/Editor/instance.lock"))
/// );
/// ```
pub fn instance_path(
    app: &str,
    os: HostOs,
    get: impl Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    state_path(app, os, get).map(|p| p.with_file_name("instance.lock"))
}

/// What the lock file says.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Lock {
    port: u16,
    token: String,
}

/// The lock file's contents.
///
/// Two lines, port then token — a format chosen so a human debugging a stuck
/// application can read it with `cat`.
fn encode_lock(lock: &Lock) -> String {
    format!("{}\n{}\n", lock.port, lock.token)
}

fn decode_lock(text: &str) -> Option<Lock> {
    let mut lines = text.lines();
    let port = lines.next()?.trim().parse::<u16>().ok()?;
    let token = lines.next()?.trim().to_string();
    if port == 0 || token.is_empty() {
        return None;
    }
    Some(Lock { port, token })
}

fn read_lock(path: &Path) -> Option<Lock> {
    decode_lock(&std::fs::read_to_string(path).ok()?)
}

fn write_lock(path: &Path, lock: &Lock) -> Result<(), InstanceError> {
    std::fs::write(path, encode_lock(lock)).map_err(io)?;
    // The token is the whole handshake; on Unix it must not be world-readable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// A token that another process on this machine cannot guess by trying.
///
/// Not cryptographic, and not claimed to be: it exists so that a process which
/// merely scans loopback ports cannot inject arguments, and it is written to a
/// file only this user can read.
///
/// The unguessable half comes from [`RandomState`], which std seeds from the
/// operating system's randomness — the one source of entropy available here
/// without taking a dependency. The clock deliberately is **not** part of it:
/// `SystemTime::now` is microsecond-grained on macOS, so two calls in the same
/// process routinely read the same instant, and neither the process id nor the
/// address of a local ever differs between them. A counter is what actually
/// guarantees that no two tokens from one process collide; the process id stays
/// only so that a human reading a stuck lock file can see who wrote it.
fn new_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    let entropy = RandomState::new().build_hasher().finish();
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    format!("{pid:x}-{seq:x}-{entropy:016x}")
}

/// The wire format: the token, then one argument per line.
///
/// Newlines and backslashes inside an argument are escaped, because a file
/// called `weird\nname.md` would otherwise arrive as two arguments — and
/// filenames with newlines in them are legal on every Unix.
///
/// ```
/// use silka_platform::instance::{decode_message, encode_message};
///
/// let wire = encode_message("tok", &["a b".into(), "line\nbreak".into()]);
/// assert_eq!(decode_message(&wire), Some(("tok".to_string(), vec!["a b".to_string(), "line\nbreak".to_string()])));
/// ```
pub fn encode_message(token: &str, args: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&escape(token));
    out.push('\n');
    for arg in args {
        out.push_str(&escape(arg));
        out.push('\n');
    }
    out
}

/// Take the wire format apart; `None` when it is not one.
pub fn decode_message(text: &str) -> Option<(String, Vec<String>)> {
    let mut lines = text.lines();
    let token = unescape(lines.next()?);
    if token.is_empty() {
        return None;
    }
    Some((token, lines.map(unescape).collect()))
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Hand the arguments to the instance the lock file describes.
fn forward(lock: &Lock, args: &[String]) -> std::io::Result<()> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, lock.port));
    let mut stream = TcpStream::connect_timeout(&addr, FORWARD_TIMEOUT)?;
    stream.set_write_timeout(Some(FORWARD_TIMEOUT))?;
    stream.write_all(encode_message(&lock.token, args).as_bytes())?;
    stream.flush()
}

/// The primary instance's end of the handshake.
///
/// **Keep it alive.** Dropping it removes the lock file, which is exactly what
/// should happen when the application exits — and exactly what must not happen
/// while it is running.
#[derive(Debug)]
pub struct InstanceListener {
    listener: TcpListener,
    token: String,
    path: PathBuf,
}

impl InstanceListener {
    /// Every launch that has arrived since the last call.
    ///
    /// Never blocks: the listener is non-blocking, so this is safe to call once
    /// per frame from the event loop without a thread and without stalling a
    /// frame on a socket that has nothing to say.
    ///
    /// **Call it every frame, not once.** Accepting a loopback connection is
    /// not synchronous with the `connect` that made it: the kernel can finish
    /// the handshake after the forwarding process has already written its
    /// arguments and exited. A launch that a poll does not see is not lost —
    /// it is waiting in the accept queue for the next one, a frame later.
    ///
    /// A connection that does not present the right token is dropped in
    /// silence. Reporting it would let any local process make an application
    /// log whatever it liked.
    pub fn poll(&self) -> Vec<Vec<String>> {
        let mut launches = Vec::new();
        loop {
            let Ok((mut stream, _)) = self.listener.accept() else {
                // `WouldBlock` — nothing waiting — and every other error mean
                // the same thing here: stop for this frame.
                return launches;
            };
            // Whether an accepted socket inherits the listener's non-blocking
            // flag differs between Linux and the BSDs, so it is set explicitly
            // rather than assumed; the read timeout is what keeps a silent
            // peer from stalling a frame.
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(FORWARD_TIMEOUT));
            let mut text = String::new();
            if stream.read_to_string(&mut text).is_err() {
                continue;
            }
            let Some((token, args)) = decode_message(&text) else {
                continue;
            };
            if token != self.token {
                continue;
            }
            launches.push(args);
        }
    }

    /// The loopback port other instances forward to.
    pub fn port(&self) -> u16 {
        self.listener
            .local_addr()
            .map(|a| a.port())
            .unwrap_or_default()
    }

    /// The lock file this instance owns.
    pub fn lock_path(&self) -> &Path {
        &self.path
    }
}

impl Drop for InstanceListener {
    fn drop(&mut self) {
        // Leaving the file behind is survivable — the next launch finds nothing
        // listening and takes over — but cleaning up means the common case
        // never has to rely on that.
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Poll the way an event loop does: again next frame, until something
    /// arrives or the deadline passes.
    ///
    /// A test that polls exactly once after a forward is measuring how fast the
    /// kernel moved a finished handshake into the accept queue, not whether
    /// this module delivers — and on macOS that first poll routinely sees
    /// nothing.
    fn poll_until(listener: &InstanceListener, deadline: Duration) -> Vec<Vec<String>> {
        let start = std::time::Instant::now();
        loop {
            let launches = listener.poll();
            if !launches.is_empty() || start.elapsed() >= deadline {
                return launches;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Keep polling for the whole span and collect everything.
    ///
    /// What an assertion that *nothing* arrives needs: the connection has to be
    /// given time to land before its absence means anything.
    fn poll_throughout(listener: &InstanceListener, span: Duration) -> Vec<Vec<String>> {
        let start = std::time::Instant::now();
        let mut all = Vec::new();
        while start.elapsed() < span {
            all.extend(listener.poll());
            std::thread::sleep(Duration::from_millis(1));
        }
        all
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("silka-instance-{name}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn lock_hidup_di_samping_state_aplikasi() {
        let path = instance_path("Editor", HostOs::MacOs, |k| {
            (k == "HOME").then(|| "/Users/ana".to_string())
        });
        assert_eq!(
            path,
            Some(PathBuf::from(
                "/Users/ana/Library/Application Support/Editor/instance.lock"
            ))
        );
    }

    #[test]
    fn lock_bisa_dibaca_manusia_dan_dibaca_ulang() {
        let lock = Lock {
            port: 51234,
            token: "abc".into(),
        };
        let text = encode_lock(&lock);
        assert!(text.starts_with("51234\n"));
        assert_eq!(decode_lock(&text), Some(lock));
    }

    #[test]
    fn lock_rusak_ditolak_bukan_ditebak() {
        assert_eq!(decode_lock(""), None);
        assert_eq!(decode_lock("bukan-angka\ntok"), None);
        assert_eq!(decode_lock("0\ntok"), None);
        assert_eq!(decode_lock("51234\n"), None);
        assert_eq!(decode_lock("51234\n   \n"), None);
    }

    #[test]
    fn nama_berkas_dengan_baris_baru_tetap_satu_argumen() {
        // Legal on every Unix, and the classic way a line-based protocol
        // silently splits one file into two.
        let args = vec!["a b".to_string(), "weird\nname.md".to_string()];
        let wire = encode_message("tok", &args);
        assert_eq!(decode_message(&wire), Some(("tok".to_string(), args)));
    }

    #[test]
    fn backslash_tidak_dimakan() {
        let args = vec![r"C:\Users\a.md".to_string()];
        let wire = encode_message("tok", &args);
        assert_eq!(decode_message(&wire).unwrap().1, args);
    }

    #[test]
    fn pesan_tanpa_token_ditolak() {
        assert_eq!(decode_message(""), None);
        assert_eq!(decode_message("\n\n"), None);
    }

    #[test]
    fn peluncuran_tanpa_argumen_tetap_sah() {
        // "Open the application again" is a launch with nothing attached, and
        // it still has to reach the first instance so the window comes forward.
        let wire = encode_message("tok", &[]);
        assert_eq!(decode_message(&wire), Some(("tok".to_string(), Vec::new())));
    }

    #[test]
    fn token_dua_proses_tidak_pernah_sama() {
        // A batch rather than a pair: two calls close enough together to read
        // the same microsecond off the clock are exactly the case that used to
        // collide, and a pair only catches it some of the time.
        let tokens: std::collections::HashSet<String> = (0..1000).map(|_| new_token()).collect();
        assert_eq!(tokens.len(), 1000);
        assert!(tokens.iter().all(|t| !t.is_empty()));
        // The wire format is one token per line; a token that carried a newline
        // or a space would be a token the far end reads back differently.
        assert!(tokens.iter().all(|t| !t.contains(char::is_whitespace)));
    }

    #[test]
    fn instance_pertama_jadi_primary() {
        let dir = temp_dir("pertama");
        let lock = dir.join("instance.lock");
        let _ = std::fs::remove_file(&lock);

        let first = single_instance("Editor").lock_file(&lock);
        let role = first.acquire().expect("lock bisa ditulis");
        assert!(role.is_primary());
        assert!(lock.exists());

        drop(role);
        // The lock is cleaned up on the way out.
        assert!(!lock.exists());
    }

    #[test]
    fn instance_kedua_meneruskan_argumennya_lalu_mengalah() {
        let dir = temp_dir("kedua");
        let lock = dir.join("instance.lock");
        let _ = std::fs::remove_file(&lock);

        let role = single_instance("Editor")
            .lock_file(&lock)
            .acquire()
            .expect("lock bisa ditulis");
        let InstanceRole::Primary(listener) = role else {
            panic!("yang pertama harus primary");
        };

        let second = single_instance("Editor")
            .lock_file(&lock)
            .arguments(["/tmp/notes.md"])
            .acquire()
            .expect("penerusan berhasil");
        assert!(!second.is_primary());

        let launches = poll_until(&listener, Duration::from_secs(2));
        assert_eq!(launches, vec![vec!["/tmp/notes.md".to_string()]]);
        // …and nothing is delivered twice.
        assert!(poll_throughout(&listener, Duration::from_millis(50)).is_empty());
    }

    #[test]
    fn lock_basi_diambil_alih_bukan_menggantung() {
        // What is left behind by a crash: a lock file pointing at a port
        // nothing listens on.
        let dir = temp_dir("basi");
        let lock = dir.join("instance.lock");
        write_lock(
            &lock,
            &Lock {
                // Port 1 needs privileges to bind, so nothing of ours is there.
                port: 1,
                token: "stale".into(),
            },
        )
        .expect("lock bisa ditulis");

        let role = single_instance("Editor")
            .lock_file(&lock)
            .acquire()
            .expect("lock basi diambil alih");
        assert!(role.is_primary());
    }

    #[test]
    fn token_salah_diabaikan_dalam_diam() {
        let dir = temp_dir("token");
        let lock = dir.join("instance.lock");
        let _ = std::fs::remove_file(&lock);
        let role = single_instance("Editor")
            .lock_file(&lock)
            .acquire()
            .expect("lock bisa ditulis");
        let InstanceRole::Primary(listener) = role else {
            panic!("harus primary");
        };

        let bogus = Lock {
            port: listener.port(),
            token: "salah".into(),
        };
        let _ = forward(&bogus, &["/tmp/a.md".to_string()]);
        // Long enough that the connection has certainly been accepted, so the
        // empty result means the token was rejected rather than that nothing
        // had arrived yet.
        assert!(poll_throughout(&listener, Duration::from_millis(250)).is_empty());
    }

    #[test]
    fn argumen_bawaan_diambil_dari_argv() {
        // Not asserting the contents — a test binary's argv is its own — only
        // that the default is argv rather than nothing.
        let instance = single_instance("Editor");
        assert_eq!(
            instance.arguments_to_forward().len(),
            std::env::args().count() - 1
        );
    }
}
