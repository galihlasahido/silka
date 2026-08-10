//! Session state: what the application writes down before it is closed, and
//! reads back when it opens again (INTEGRASI-NATIVE §6, last two rows).
//!
//! The OS never does this for us. What it *does* do is give us exactly one
//! moment to react — `NSApplicationDelegate`'s terminate, Windows'
//! `WM_QUERYENDSESSION`, a Wayland compositor closing the toplevel — and that
//! moment arrives at the shell as a quit event. So:
//!
//! 1. The shell fills a [`SessionState`] with the window geometry it has been
//!    tracking all along.
//! 2. The application's [`crate::WindowConfig::on_quit`] handler adds whatever
//!    else has to survive (the open document, the scroll offset, the selected
//!    tab).
//! 3. The [`StateStore`] writes it, atomically.
//!
//! The format is deliberately a flat, line-based text file rather than a
//! serialization framework: it is diffable, a user can delete one line of it
//! when something goes wrong, it adds no dependency, and — most importantly —
//! **decoding never fails**. A truncated or hand-edited state file loses the
//! lines that are broken and keeps the rest; an application that refuses to
//! start because its window-position file is corrupt is a worse bug than any
//! it was trying to avoid.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use silka_paint::Size;

use crate::error::PlatformError;

use super::restore::WindowPlacement;

/// The first line of the file — a version marker, so a future format change
/// can be recognised instead of silently misread.
const HEADER: &str = "# silka session v1";

const KEY_X: &str = "window.x";
const KEY_Y: &str = "window.y";
const KEY_WIDTH: &str = "window.width";
const KEY_HEIGHT: &str = "window.height";
const KEY_SCALE: &str = "window.scale";
const KEY_MAXIMIZED: &str = "window.maximized";

/// Prefix under which the application's own values are stored, keeping them
/// from ever colliding with the framework's `window.*` keys.
const APP_PREFIX: &str = "app.";

/// Everything one window remembers between runs.
///
/// The framework owns the geometry; the application owns everything else, and
/// the two can never collide because application keys are namespaced.
///
/// ```
/// use silka_paint::Size;
/// use silka_platform::{SessionState, WindowPlacement};
///
/// let mut state = SessionState::new();
/// assert!(state.is_empty());
///
/// state.set_placement(WindowPlacement::sized(Size::new(1024.0, 720.0)).at(40, 40));
/// state.set("page", "transactions");
///
/// // It round-trips through a plain text encoding — no serialization crate.
/// let restored = SessionState::decode(&state.encode());
/// assert_eq!(restored.get("page"), Some("transactions"));
/// assert_eq!(restored.placement(), state.placement());
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionState {
    placement: Option<WindowPlacement>,
    values: Vec<(String, String)>,
}

impl SessionState {
    /// An empty state — a first run.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when nothing at all has been remembered.
    pub fn is_empty(&self) -> bool {
        self.placement.is_none() && self.values.is_empty()
    }

    /// The saved window geometry, if there is one.
    pub fn placement(&self) -> Option<WindowPlacement> {
        self.placement
    }

    /// Record the window geometry (the shell does this; applications do not
    /// have to).
    pub fn set_placement(&mut self, placement: WindowPlacement) {
        self.placement = Some(placement);
    }

    /// One of the application's own values.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Store one of the application's own values, replacing any previous one.
    ///
    /// Insertion order is preserved so the file stays diffable between runs.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let (key, value) = (key.into(), value.into());
        match self.values.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => self.values.push((key, value)),
        }
    }

    /// Remove one value; returns what was there.
    pub fn remove(&mut self, key: &str) -> Option<String> {
        let i = self.values.iter().position(|(k, _)| k == key)?;
        Some(self.values.remove(i).1)
    }

    /// Every application value, in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Serialize to the on-disk form.
    pub fn encode(&self) -> String {
        let mut out = String::from(HEADER);
        out.push('\n');
        if let Some(p) = self.placement {
            if let Some((x, y)) = p.position {
                out.push_str(&format!("{KEY_X} = {x}\n"));
                out.push_str(&format!("{KEY_Y} = {y}\n"));
            }
            out.push_str(&format!("{KEY_WIDTH} = {}\n", p.size.width));
            out.push_str(&format!("{KEY_HEIGHT} = {}\n", p.size.height));
            out.push_str(&format!("{KEY_SCALE} = {}\n", p.scale));
            out.push_str(&format!("{KEY_MAXIMIZED} = {}\n", p.maximized));
        }
        for (k, v) in &self.values {
            out.push_str(&format!("{APP_PREFIX}{} = {}\n", escape(k), escape(v)));
        }
        out
    }

    /// Parse the on-disk form. **Never fails**: broken lines are dropped.
    pub fn decode(text: &str) -> Self {
        let mut out = Self::new();
        let mut x = None;
        let mut y = None;
        let mut width = None;
        let mut height = None;
        let mut scale = 1.0_f64;
        let mut maximized = false;

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match key {
                KEY_X => x = value.parse::<i32>().ok(),
                KEY_Y => y = value.parse::<i32>().ok(),
                KEY_WIDTH => width = value.parse::<f32>().ok().filter(|v| v.is_finite()),
                KEY_HEIGHT => height = value.parse::<f32>().ok().filter(|v| v.is_finite()),
                KEY_SCALE => {
                    if let Some(v) = value
                        .parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite() && *v > 0.0)
                    {
                        scale = v;
                    }
                }
                KEY_MAXIMIZED => maximized = super::parse_bool(value).unwrap_or(false),
                _ => {
                    if let Some(name) = key.strip_prefix(APP_PREFIX) {
                        if !name.is_empty() {
                            out.set(unescape(name), unescape(value));
                        }
                    }
                }
            }
        }

        // A size is what makes geometry meaningful; a position without one
        // cannot be validated against a monitor, so it is dropped with it.
        if let (Some(w), Some(h)) = (width, height) {
            out.placement = Some(WindowPlacement {
                position: x.zip(y),
                size: Size::new(w, h),
                scale,
                maximized,
            });
        }
        out
    }
}

/// Escape the two characters that would otherwise break the line format.
fn escape(raw: &str) -> String {
    raw.replace('\\', "\\\\").replace('\n', "\\n")
}

/// The inverse of [`escape`].
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
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

/// Where a [`SessionState`] is kept.
///
/// A trait rather than a concrete file, because the two interesting
/// implementations are not both files: an application that already has its own
/// preferences database wants to put the window geometry in there, and a test
/// wants it in memory.
///
/// ```
/// use silka_platform::{MemoryStore, SessionState, StateStore};
///
/// let store = MemoryStore::new();
///
/// // A first run — or an unreadable store — is an empty state, never an
/// // error: refusing to start because a preferences file is missing would be
/// // absurd.
/// assert!(store.load().is_empty());
///
/// let mut state = SessionState::new();
/// state.set("page", "dashboard");
/// store.save(&state).unwrap();
/// assert_eq!(store.load().get("page"), Some("dashboard"));
/// ```
pub trait StateStore {
    /// Read the stored state. A first run — or an unreadable store — is an
    /// empty state, never an error: failing to start because a preferences
    /// file is missing would be absurd.
    fn load(&self) -> SessionState;

    /// Write the state. Called once, at quit.
    fn save(&self, state: &SessionState) -> Result<(), PlatformError>;
}

/// A store that keeps the state in memory — for tests and for an application
/// that deliberately does not persist.
///
/// ```
/// use silka_platform::{MemoryStore, SessionState, StateStore};
///
/// // `with_state` is the shape of "the previous run" in a test.
/// let mut previous = SessionState::new();
/// previous.set("page", "chart");
/// let store = MemoryStore::with_state(previous);
///
/// assert_eq!(store.load().get("page"), Some("chart"));
/// assert_eq!(store.saved().get("page"), Some("chart"));
/// ```
#[derive(Debug, Default)]
pub struct MemoryStore {
    inner: RefCell<SessionState>,
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// A store pre-filled with a state — the shape of "the previous run".
    pub fn with_state(state: SessionState) -> Self {
        Self {
            inner: RefCell::new(state),
        }
    }

    /// What has been saved so far.
    pub fn saved(&self) -> SessionState {
        self.inner.borrow().clone()
    }
}

impl StateStore for MemoryStore {
    fn load(&self) -> SessionState {
        self.inner.borrow().clone()
    }

    fn save(&self, state: &SessionState) -> Result<(), PlatformError> {
        *self.inner.borrow_mut() = state.clone();
        Ok(())
    }
}

/// A store backed by one text file.
///
/// Written atomically (to a temporary file, then renamed), so a crash mid-quit
/// cannot leave a truncated file that costs the user their window position.
///
/// ```
/// use silka_platform::{FileStore, SessionState, StateStore};
///
/// let path = std::env::temp_dir().join("silka-doc-state.silka");
/// let store = FileStore::at(&path);
///
/// let mut state = SessionState::new();
/// state.set("page", "transactions");
/// store.save(&state).unwrap();
/// assert_eq!(store.load().get("page"), Some("transactions"));
/// # let _ = std::fs::remove_file(&path);
/// ```
///
/// [`FileStore::for_app`] picks the conventional per-user location for the
/// host OS instead of an explicit path.
#[derive(Debug, Clone)]
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    /// A store at an explicit path.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// A store at the conventional per-user location for `app`
    /// ([`state_path`]).
    ///
    /// Falls back to the current directory when the OS reports no home at all
    /// — a state file next to the binary is still better than losing the
    /// window position.
    pub fn for_app(app: &str) -> Self {
        let path = state_path(app, HostOs::CURRENT, |name| std::env::var(name).ok())
            .unwrap_or_else(|| PathBuf::from(format!("{}.silka", sanitize_app_name(app))));
        Self::at(path)
    }

    /// The file this store writes.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl StateStore for FileStore {
    fn load(&self) -> SessionState {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => SessionState::decode(&text),
            Err(_) => SessionState::new(),
        }
    }

    fn save(&self, state: &SessionState) -> Result<(), PlatformError> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| PlatformError::State(format!("{}: {e}", parent.display())))?;
            }
        }
        // Written to a sibling and renamed into place: a machine that loses
        // power mid-save must not come back with half a state file.
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, state.encode())
            .map_err(|e| PlatformError::State(format!("{}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| PlatformError::State(format!("{}: {e}", self.path.display())))?;
        Ok(())
    }
}

/// The host operating-system family, as far as the state path is concerned.
///
/// It is an enum rather than `cfg!` at the call site so that the path logic for
/// all three platforms can be tested on any one of them.
///
/// ```
/// use silka_platform::lifecycle::HostOs;
///
/// // What this binary was compiled for.
/// let here = HostOs::CURRENT;
/// assert!(matches!(here, HostOs::MacOs | HostOs::Windows | HostOs::Unix));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    /// macOS: `~/Library/Application Support`.
    MacOs,
    /// Windows: `%APPDATA%`.
    Windows,
    /// Everything else: `$XDG_CONFIG_HOME`, else `~/.config`.
    Unix,
}

impl HostOs {
    /// The family this binary was compiled for.
    pub const CURRENT: HostOs = if cfg!(target_os = "macos") {
        HostOs::MacOs
    } else if cfg!(target_os = "windows") {
        HostOs::Windows
    } else {
        HostOs::Unix
    };
}

/// Strip everything from an application name that must never reach a path.
///
/// An application called `../../etc` would otherwise write its window position
/// wherever it pleased.
pub fn sanitize_app_name(app: &str) -> String {
    let cleaned: String = app
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('-').to_string();
    if cleaned.is_empty() {
        "silka".to_string()
    } else {
        cleaned
    }
}

/// The conventional state-file path for an application, per OS.
///
/// `get` reads environment variables; passing it in is what makes every branch
/// testable from any machine.
pub fn state_path(app: &str, os: HostOs, get: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let app = sanitize_app_name(app);
    let dir = match os {
        HostOs::MacOs => {
            let home = get("HOME")?;
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join(&app)
        }
        HostOs::Windows => PathBuf::from(get("APPDATA")?).join(&app),
        HostOs::Unix => match get("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            Some(base) => PathBuf::from(base).join(&app),
            None => PathBuf::from(get("HOME")?).join(".config").join(&app),
        },
    };
    Some(dir.join("state.silka"))
}

/// Why the application is being closed.
///
/// ```
/// use silka_platform::QuitReason;
///
/// // A close button can still be refused — that is where "you have unsaved
/// // work" lives.
/// assert!(QuitReason::CloseRequested.can_cancel());
///
/// // By the time the event loop is ending, the decision has been made and
/// // only saving is still open.
/// assert!(!QuitReason::Exiting.can_cancel());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitReason {
    /// The user closed the window (red button, Cmd+W, Alt+F4).
    CloseRequested,
    /// The event loop is ending — the app quit through a menu, or the OS is
    /// logging the user out. **Cancelling is not possible here**: by this point
    /// the decision has been made and only saving is still open.
    Exiting,
}

impl QuitReason {
    /// Whether a handler may still veto the quit.
    pub fn can_cancel(self) -> bool {
        matches!(self, QuitReason::CloseRequested)
    }
}

/// What an [`crate::WindowConfig::on_quit`] handler is given.
///
/// It carries the state that is about to be written — with the window geometry
/// already filled in by the shell — and the ability to say "not yet" while the
/// user still has unsaved work.
///
/// ```
/// use silka_platform::{QuitContext, QuitReason, SessionState};
///
/// // The shell has already filled in the window geometry; the application
/// // adds whatever else it wants back next time.
/// let mut ctx = QuitContext::new(QuitReason::CloseRequested, SessionState::new());
/// ctx.remember("page", "transactions");
///
/// // Unsaved work: refuse the quit and show a dialog instead.
/// ctx.cancel();
/// assert!(ctx.is_cancelled());
///
/// let (state, cancelled) = ctx.finish();
/// assert!(cancelled);
/// assert_eq!(state.get("page"), Some("transactions"));
/// ```
#[derive(Debug)]
pub struct QuitContext {
    reason: QuitReason,
    state: SessionState,
    cancelled: bool,
}

impl QuitContext {
    /// Build a context around the state that is about to be saved.
    pub fn new(reason: QuitReason, state: SessionState) -> Self {
        Self {
            reason,
            state,
            cancelled: false,
        }
    }

    /// Why the application is closing.
    pub fn reason(&self) -> QuitReason {
        self.reason
    }

    /// The state that will be written.
    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// The state that will be written, to add to.
    pub fn state_mut(&mut self) -> &mut SessionState {
        &mut self.state
    }

    /// Shorthand for `state_mut().set(key, value)`.
    pub fn remember(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.state.set(key, value);
    }

    /// Keep the application open (an unsaved document, a confirmation sheet).
    ///
    /// Ignored once the quit can no longer be cancelled
    /// ([`QuitReason::can_cancel`]) — a handler that tries to veto an OS logout
    /// would otherwise leave the app in a state where the user cannot log out
    /// at all.
    pub fn cancel(&mut self) {
        if self.reason.can_cancel() {
            self.cancelled = true;
        }
    }

    /// True when the handler asked to stay open.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// Consume the context, returning the state to save and whether the quit
    /// was vetoed.
    pub fn finish(self) -> (SessionState, bool) {
        (self.state, self.cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contoh() -> SessionState {
        let mut s = SessionState::new();
        s.set_placement(
            WindowPlacement::sized(Size::new(820.0, 640.0))
                .at(120, 64)
                .scaled(2.0),
        );
        s.set("halaman", "chart");
        s.set("gulir", "12.5");
        s
    }

    #[test]
    fn encode_decode_bolak_balik() {
        let asal = contoh();
        let kembali = SessionState::decode(&asal.encode());
        assert_eq!(kembali, asal);
    }

    #[test]
    fn state_kosong_tetap_bolak_balik() {
        let kosong = SessionState::new();
        assert!(kosong.is_empty());
        assert_eq!(SessionState::decode(&kosong.encode()), kosong);
    }

    #[test]
    fn berkas_rusak_kehilangan_baris_bukan_aplikasinya() {
        // Half a file, a hand-edited line, a stray key: what survives must
        // survive, and nothing may panic.
        let teks = "# silka session v1\n\
                    window.x = 100\n\
                    window.y = bukan angka\n\
                    window.width = 800\n\
                    window.height = 600\n\
                    window.scale = -3\n\
                    baris tanpa tanda sama dengan\n\
                    app.halaman = tabel\n\
                    app. = tanpa nama\n\
                    kunci.asing = diabaikan\n\
                    window.hei";
        let s = SessionState::decode(teks);
        let p = s.placement().expect("ukuran masih utuh");
        assert_eq!(p.size, Size::new(800.0, 600.0));
        // x survived but y did not — a half position is no position.
        assert_eq!(p.position, None);
        // A nonsensical scale falls back to 1 rather than inverting geometry.
        assert_eq!(p.scale, 1.0);
        assert_eq!(s.get("halaman"), Some("tabel"));
        assert_eq!(s.get(""), None);
    }

    #[test]
    fn berkas_kosong_atau_sampah_menghasilkan_state_kosong() {
        assert!(SessionState::decode("").is_empty());
        assert!(SessionState::decode("\n\n# cuma komentar\n").is_empty());
        assert!(SessionState::decode("\u{0}\u{1}biner").is_empty());
    }

    #[test]
    fn ukuran_tanpa_posisi_tetap_dipulihkan() {
        let teks = "window.width = 640\nwindow.height = 480\n";
        let p = SessionState::decode(teks).placement().expect("ada ukuran");
        assert_eq!(p.size, Size::new(640.0, 480.0));
        assert_eq!(p.position, None);
    }

    #[test]
    fn posisi_tanpa_ukuran_dibuang() {
        // A position that cannot be validated against a monitor is worse than
        // no position at all.
        let teks = "window.x = 10\nwindow.y = 20\n";
        assert!(SessionState::decode(teks).placement().is_none());
    }

    #[test]
    fn nilai_dengan_tanda_sama_dengan_dan_baris_baru_selamat() {
        let mut s = SessionState::new();
        s.set("kueri", "a = b");
        s.set("catatan", "baris satu\nbaris dua");
        s.set("jalur", r"C:\Users\x");
        let kembali = SessionState::decode(&s.encode());
        assert_eq!(kembali.get("kueri"), Some("a = b"));
        assert_eq!(kembali.get("catatan"), Some("baris satu\nbaris dua"));
        assert_eq!(kembali.get("jalur"), Some(r"C:\Users\x"));
    }

    #[test]
    fn set_mengganti_bukan_menumpuk() {
        let mut s = SessionState::new();
        s.set("halaman", "satu");
        s.set("halaman", "dua");
        assert_eq!(s.iter().count(), 1);
        assert_eq!(s.get("halaman"), Some("dua"));
        assert_eq!(s.remove("halaman").as_deref(), Some("dua"));
        assert_eq!(s.get("halaman"), None);
    }

    #[test]
    fn urutan_kunci_stabil_agar_berkas_bisa_dibandingkan() {
        let a = contoh().encode();
        let b = contoh().encode();
        assert_eq!(a, b);
        assert!(a.starts_with(HEADER));
    }

    #[test]
    fn memory_store_menyimpan_dan_memuat() {
        let store = MemoryStore::new();
        assert!(store.load().is_empty());
        store.save(&contoh()).expect("simpan");
        assert_eq!(store.load(), contoh());
        assert_eq!(store.saved(), contoh());
    }

    #[test]
    fn file_store_bolak_balik_lewat_disk() {
        let dir = std::env::temp_dir().join(format!("silka-state-{}", std::process::id()));
        let store = FileStore::at(dir.join("sub").join("state.silka"));
        // A store whose file does not exist yet is a first run, not an error.
        assert!(store.load().is_empty());
        store.save(&contoh()).expect("simpan");
        assert_eq!(store.load(), contoh());
        // Saving twice overwrites rather than appending.
        store.save(&contoh()).expect("simpan lagi");
        assert_eq!(store.load(), contoh());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_store_yang_tidak_bisa_ditulis_melaporkan_kesalahan() {
        // A path whose parent is a file, not a directory.
        let dir = std::env::temp_dir().join(format!("silka-state-x-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("siapkan");
        let berkas = dir.join("bukan-direktori");
        std::fs::write(&berkas, "x").expect("siapkan");
        let store = FileStore::at(berkas.join("state.silka"));
        assert!(store.save(&contoh()).is_err());
        // …and loading from it is still merely empty.
        assert!(store.load().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jalur_state_mengikuti_konvensi_tiap_os() {
        let env = |pairs: Vec<(&'static str, &'static str)>| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.to_string())
            }
        };
        let mac =
            state_path("Galeri", HostOs::MacOs, env(vec![("HOME", "/Users/a")])).expect("ada HOME");
        assert_eq!(
            mac,
            PathBuf::from("/Users/a/Library/Application Support/Galeri/state.silka")
        );

        let win = state_path(
            "Galeri",
            HostOs::Windows,
            env(vec![("APPDATA", r"C:\Users\a\AppData\Roaming")]),
        )
        .expect("ada APPDATA");
        assert!(win.ends_with("Galeri/state.silka") || win.ends_with(r"Galeri\state.silka"));

        let xdg = state_path(
            "Galeri",
            HostOs::Unix,
            env(vec![("XDG_CONFIG_HOME", "/home/a/.cfg")]),
        )
        .expect("ada XDG");
        assert_eq!(xdg, PathBuf::from("/home/a/.cfg/Galeri/state.silka"));

        let unix =
            state_path("Galeri", HostOs::Unix, env(vec![("HOME", "/home/a")])).expect("ada HOME");
        assert_eq!(unix, PathBuf::from("/home/a/.config/Galeri/state.silka"));
    }

    #[test]
    fn tanpa_home_tidak_ada_jalur_bawaan() {
        assert_eq!(state_path("Galeri", HostOs::MacOs, |_| None), None);
        assert_eq!(state_path("Galeri", HostOs::Windows, |_| None), None);
        assert_eq!(state_path("Galeri", HostOs::Unix, |_| None), None);
        // An empty XDG_CONFIG_HOME is treated as unset, per the spec.
        let hanya_xdg_kosong = |name: &str| match name {
            "XDG_CONFIG_HOME" => Some(String::new()),
            "HOME" => Some("/home/a".into()),
            _ => None,
        };
        assert_eq!(
            state_path("Galeri", HostOs::Unix, hanya_xdg_kosong),
            Some(PathBuf::from("/home/a/.config/Galeri/state.silka"))
        );
    }

    #[test]
    fn nama_aplikasi_tidak_bisa_keluar_dari_direktorinya() {
        assert_eq!(sanitize_app_name("../../etc"), "etc");
        assert_eq!(sanitize_app_name("a/b\\c"), "a-b-c");
        assert_eq!(sanitize_app_name("  Galeri Silka  "), "Galeri Silka");
        assert_eq!(sanitize_app_name(""), "silka");
        assert_eq!(sanitize_app_name("///"), "silka");
        let p = state_path("../../etc", HostOs::Unix, |n| {
            (n == "HOME").then(|| "/home/a".to_string())
        })
        .expect("ada HOME");
        assert!(!p.to_string_lossy().contains(".."));
    }

    #[test]
    fn quit_bisa_dibatalkan_hanya_saat_masih_boleh() {
        let mut ctx = QuitContext::new(QuitReason::CloseRequested, SessionState::new());
        ctx.cancel();
        assert!(ctx.is_cancelled());

        // An OS logout is not up for negotiation.
        let mut ctx = QuitContext::new(QuitReason::Exiting, SessionState::new());
        ctx.cancel();
        assert!(!ctx.is_cancelled());
        assert!(!QuitReason::Exiting.can_cancel());
        assert!(QuitReason::CloseRequested.can_cancel());
    }

    #[test]
    fn handler_quit_menambahkan_state_di_atas_geometri_shell() {
        let mut awal = SessionState::new();
        awal.set_placement(WindowPlacement::sized(Size::new(800.0, 600.0)).at(0, 0));
        let mut ctx = QuitContext::new(QuitReason::Exiting, awal);
        ctx.remember("dokumen", "/tmp/a.txt");
        ctx.state_mut().set("tab", "2");
        let (state, dibatalkan) = ctx.finish();
        assert!(!dibatalkan);
        assert_eq!(state.get("dokumen"), Some("/tmp/a.txt"));
        assert_eq!(state.get("tab"), Some("2"));
        // The shell's geometry survived the application's additions.
        assert_eq!(
            state.placement().map(|p| p.size),
            Some(Size::new(800.0, 600.0))
        );
    }
}
