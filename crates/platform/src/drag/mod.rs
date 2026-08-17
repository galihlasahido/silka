//! **Drag source** — starting a drag that leaves the application
//! (INTEGRASI-NATIVE §4).
//!
//! This is the one P0 item in the whole native catalogue with no crate to lean
//! on. Receiving a drop is easy — winit already reports it. Being the *source*
//! of a drag, so that a file dragged out of a list lands in Finder, Explorer or
//! another application, is three unrelated per-platform APIs:
//!
//! | Platform | API | State here |
//! |---|---|---|
//! | macOS | `-[NSView beginDraggingSessionWithItems:event:source:]` | implemented (`drag::macos`) |
//! | Windows | `DoDragDrop` + `IDataObject`/`IDropSource` | [`DragError::Unsupported`], see below |
//! | Wayland | `wl_data_device.start_drag` | [`DragError::Unsupported`], see below |
//!
//! ## What is a value and what talks to the OS
//!
//! Everything above [`DragSource::begin`] is a plain value that can be built,
//! inspected and asserted with no OS involved: which items are on offer
//! ([`DragItem`]), which effects the source permits ([`DragEffects`]), and what
//! the pointer drags along ([`DragPreview`]). The rules that are easy to get
//! subtly wrong — how a modifier key picks copy vs move, how a path becomes a
//! `file://` URL, where the preview sits relative to the pointer — are pure
//! functions with tests, because they are the parts that would otherwise be
//! debugged by dragging things around by hand on three machines.
//!
//! ```
//! use silka_paint::Point;
//! use silka_platform::drag::{drag, DragEffect, DragEffects};
//!
//! let source = drag()
//!     .file("/tmp/report.pdf")
//!     .text("report.pdf")
//!     .allow(DragEffects::COPY.union(DragEffects::LINK));
//!
//! // Complete enough to hand to the OS…
//! assert!(source.check().is_ok());
//! // …and the effect it will report if the user holds nothing is knowable
//! // without starting a drag.
//! assert_eq!(source.allowed().preferred(), Some(DragEffect::Copy));
//! # let _ = Point::ZERO;
//! ```
//!
//! ## The two platforms that are not implemented yet, and why
//!
//! Neither is a shrug; both are blocked on something specific, and both are
//! reported as [`DragError::Unsupported`] with that reason in the message
//! rather than silently doing nothing:
//!
//! - **Windows.** `DoDragDrop` needs two COM objects implemented by the caller
//!   (`IDropSource` and `IDataObject`). windows-rs can do it, but only with its
//!   `implement` machinery and a `Win32_System_Com`/`Win32_System_Ole` feature
//!   set this workspace does not pin yet. The vocabulary here already carries
//!   what those objects need — [`DragItem::windows_format`] is the clipboard
//!   format name each item registers under.
//! - **Wayland.** `wl_data_device::start_drag` requires the seat and the
//!   **serial of the input event that started the drag**. winit exposes neither
//!   through its public API, so a correct implementation means either a winit
//!   change or reaching around it. [`DragItem::mime`] is the offer type each
//!   item would advertise.
//!
//! An application can tell which platforms are live without starting a drag:
//! [`is_supported`].

use core::fmt;
use std::path::{Path, PathBuf};

use silka_core::input::Modifiers;
use silka_paint::{Point, Rect, Size};

use crate::image::RgbaImage;
use crate::lifecycle::HostOs;
use crate::platform::NativeWindow;

/// The macOS backend: `NSDraggingSession` and everything it needs.
///
/// Public so the escape hatch (INTEGRASI-NATIVE §8) can see how the translation
/// is done — the same reason `platform::macos` exists.
#[cfg(target_os = "macos")]
pub mod macos;

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

/// What a drop is allowed to do with the dragged data.
///
/// Deliberately three and only three: every desktop platform has these, and
/// nothing else is portable. `Copy` leaves the source alone, `Move` means the
/// source must delete its copy once the drop reports success, and `Link` makes
/// a reference (an alias, a shortcut, a symlink).
///
/// ```
/// use silka_platform::drag::DragEffect;
///
/// // A move is the only one the source has to act on afterwards.
/// assert!(DragEffect::Move.source_must_remove());
/// assert!(!DragEffect::Copy.source_must_remove());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DragEffect {
    /// The receiver takes a copy; the source keeps its own.
    Copy,
    /// The receiver takes ownership; the source deletes its copy **after** the
    /// drop reports success.
    Move,
    /// The receiver makes a reference — an alias, a shortcut, a symlink.
    Link,
}

impl DragEffect {
    /// Whether the source must delete its own copy once the drop succeeds.
    ///
    /// The one question a drag source has to answer after the fact, and the one
    /// that loses a user's data when answered wrongly.
    pub const fn source_must_remove(self) -> bool {
        matches!(self, DragEffect::Move)
    }

    /// This single effect as a set.
    pub const fn as_set(self) -> DragEffects {
        match self {
            DragEffect::Copy => DragEffects::COPY,
            DragEffect::Move => DragEffects::MOVE,
            DragEffect::Link => DragEffects::LINK,
        }
    }
}

/// The set of effects a drag source permits.
///
/// A bitset rather than a `Vec` so it is `Copy`, comparable, and cheap to pass
/// into an OS call that wants a mask.
///
/// ```
/// use silka_platform::drag::{DragEffect, DragEffects};
///
/// let allowed = DragEffects::COPY.union(DragEffects::MOVE);
/// assert!(allowed.contains(DragEffects::MOVE));
/// assert!(!allowed.contains(DragEffects::LINK));
///
/// // With no modifier held, a source that allows both offers the
/// // non-destructive one.
/// assert_eq!(allowed.preferred(), Some(DragEffect::Copy));
///
/// // An empty set means "this cannot be dropped anywhere", which is a
/// // mistake worth catching before a drag starts.
/// assert!(DragEffects::NONE.is_empty());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DragEffects(u8);

impl DragEffects {
    /// Nothing is allowed — a drag that can never be dropped.
    pub const NONE: Self = Self(0);
    /// A copy is allowed.
    pub const COPY: Self = Self(1 << 0);
    /// A move is allowed.
    pub const MOVE: Self = Self(1 << 1);
    /// A link is allowed.
    pub const LINK: Self = Self(1 << 2);
    /// All three.
    pub const ALL: Self = Self(0b111);

    /// The raw bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// True when no effect at all is allowed.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True when every bit of `other` is present here.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The union of two sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The intersection of two sets.
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// The effect to use when the user holds no modifier at all.
    ///
    /// Copy first, then move, then link — the order every platform's own
    /// default follows, and the only order that is **non-destructive when it
    /// guesses wrong**: a copy the user did not want leaves a stray file, a
    /// move the user did not want loses the original.
    pub const fn preferred(self) -> Option<DragEffect> {
        if self.contains(Self::COPY) {
            Some(DragEffect::Copy)
        } else if self.contains(Self::MOVE) {
            Some(DragEffect::Move)
        } else if self.contains(Self::LINK) {
            Some(DragEffect::Link)
        } else {
            None
        }
    }
}

impl core::ops::BitOr for DragEffects {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl From<DragEffect> for DragEffects {
    fn from(effect: DragEffect) -> Self {
        effect.as_set()
    }
}

/// The effect the modifiers currently held ask for, narrowed to what the source
/// allows.
///
/// The modifier conventions are **not** the same on every platform and a user
/// notices immediately when they are wrong:
///
/// | Effect | macOS | Windows / Linux |
/// |---|---|---|
/// | Copy | ⌥ Option | Ctrl |
/// | Move | ⌘ Command | ⇧ Shift |
/// | Link | ⌘⌥ | Ctrl+⇧ |
///
/// A modifier that asks for something the source did not allow does not cancel
/// the drag; it falls back to [`DragEffects::preferred`], which is what the OS
/// itself does.
///
/// ```
/// use silka_core::input::Modifiers;
/// use silka_platform::drag::{effect_for_modifiers, DragEffect, DragEffects};
/// use silka_platform::lifecycle::HostOs;
///
/// let both = DragEffects::COPY.union(DragEffects::MOVE);
///
/// // ⌘ on macOS is move; Ctrl on Windows is copy. Same gesture, different key.
/// assert_eq!(
///     effect_for_modifiers(both, Modifiers::META, HostOs::MacOs),
///     Some(DragEffect::Move)
/// );
/// assert_eq!(
///     effect_for_modifiers(both, Modifiers::CONTROL, HostOs::Windows),
///     Some(DragEffect::Copy)
/// );
///
/// // Asking for a link from a source that does not offer one falls back
/// // rather than cancelling.
/// assert_eq!(
///     effect_for_modifiers(both, Modifiers::META.union(Modifiers::ALT), HostOs::MacOs),
///     Some(DragEffect::Copy)
/// );
/// ```
pub fn effect_for_modifiers(
    allowed: DragEffects,
    modifiers: Modifiers,
    host: HostOs,
) -> Option<DragEffect> {
    let requested = requested_effect(modifiers, host);
    match requested {
        Some(effect) if allowed.contains(effect.as_set()) => Some(effect),
        _ => allowed.preferred(),
    }
}

/// Which effect the modifier keys are asking for, before the source's own
/// permissions are taken into account.
///
/// Split out so the platform conventions can be tested on their own; the answer
/// is `None` when the user is holding nothing meaningful.
fn requested_effect(modifiers: Modifiers, host: HostOs) -> Option<DragEffect> {
    let (copy, moved) = match host {
        HostOs::MacOs => (Modifiers::ALT, Modifiers::META),
        // Windows and every Linux desktop follow the same convention here.
        HostOs::Windows | HostOs::Unix => (Modifiers::CONTROL, Modifiers::SHIFT),
    };
    let has_copy = modifiers.contains(copy);
    let has_move = modifiers.contains(moved);
    match (has_copy, has_move) {
        // Both keys together are the link gesture on both conventions.
        (true, true) => Some(DragEffect::Link),
        (true, false) => Some(DragEffect::Copy),
        (false, true) => Some(DragEffect::Move),
        (false, false) => None,
    }
}

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

/// One thing on offer in a drag.
///
/// A drag carries **several** representations of the same thing, best first:
/// dragging a row out of a file list offers the file itself to Finder, and its
/// name as text to a text editor. Whoever receives it takes the first
/// representation it understands, so the order items are added in is the order
/// of preference.
///
/// ```
/// use silka_platform::drag::DragItem;
///
/// // Each item knows what it is called on each platform — the three names an
/// // OS drag API asks for.
/// let text = DragItem::text("hello");
/// assert_eq!(text.uti(), "public.utf8-plain-text");
/// assert_eq!(text.mime(), "text/plain;charset=utf-8");
/// assert_eq!(text.windows_format(), "CF_UNICODETEXT");
///
/// // A custom payload keeps its own reverse-DNS name everywhere, which is what
/// // lets an application drag its own document type between its own windows.
/// let own = DragItem::custom("com.example.card", vec![1, 2, 3]);
/// assert_eq!(own.uti(), "com.example.card");
/// assert_eq!(own.mime(), "com.example.card");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DragItem {
    /// Plain UTF-8 text.
    Text(String),
    /// Rich text, with the plain-text alternative that a receiver which cannot
    /// read HTML must get instead.
    ///
    /// The alternative is not optional in practice: a drop into a terminal must
    /// not produce a wall of tags.
    Html {
        /// The markup.
        html: String,
        /// What a plain-text receiver gets.
        plain: String,
    },
    /// A link.
    Url(String),
    /// One or more files, by path.
    Files(Vec<PathBuf>),
    /// An application's own type: a reverse-DNS name plus opaque bytes.
    Custom {
        /// A reverse-DNS type name (`com.example.card`), used unchanged as the
        /// UTI, the MIME type and the registered clipboard format.
        kind: String,
        /// The payload.
        bytes: Vec<u8>,
    },
}

impl DragItem {
    /// Plain text.
    pub fn text(text: impl Into<String>) -> Self {
        DragItem::Text(text.into())
    }

    /// HTML with a plain-text alternative.
    pub fn html(html: impl Into<String>, plain: impl Into<String>) -> Self {
        DragItem::Html {
            html: html.into(),
            plain: plain.into(),
        }
    }

    /// A link.
    pub fn url(url: impl Into<String>) -> Self {
        DragItem::Url(url.into())
    }

    /// A single file.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        DragItem::Files(vec![path.into()])
    }

    /// Several files at once.
    pub fn files<P: Into<PathBuf>>(paths: impl IntoIterator<Item = P>) -> Self {
        DragItem::Files(paths.into_iter().map(Into::into).collect())
    }

    /// An application's own type.
    pub fn custom(kind: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        DragItem::Custom {
            kind: kind.into(),
            bytes: bytes.into(),
        }
    }

    /// The macOS pasteboard type (a Uniform Type Identifier).
    pub fn uti(&self) -> &str {
        match self {
            DragItem::Text(_) => "public.utf8-plain-text",
            DragItem::Html { .. } => "public.html",
            DragItem::Url(_) => "public.url",
            DragItem::Files(_) => "public.file-url",
            DragItem::Custom { kind, .. } => kind,
        }
    }

    /// The MIME type a Wayland or X11 offer advertises.
    pub fn mime(&self) -> &str {
        match self {
            DragItem::Text(_) => "text/plain;charset=utf-8",
            DragItem::Html { .. } => "text/html",
            // Both a URL and a file list travel as `text/uri-list` on X11 and
            // Wayland; there is no separate "one link" type there.
            DragItem::Url(_) | DragItem::Files(_) => "text/uri-list",
            DragItem::Custom { kind, .. } => kind,
        }
    }

    /// The Windows clipboard format this item is offered under.
    ///
    /// The three standard ones are named as their `CF_*` constants; everything
    /// else is a registered format name, which is what
    /// `RegisterClipboardFormat` takes verbatim.
    pub fn windows_format(&self) -> &str {
        match self {
            DragItem::Text(_) => "CF_UNICODETEXT",
            DragItem::Html { .. } => "HTML Format",
            DragItem::Url(_) => "UniformResourceLocatorW",
            DragItem::Files(_) => "CF_HDROP",
            DragItem::Custom { kind, .. } => kind,
        }
    }

    /// Whether this item carries nothing at all.
    ///
    /// An empty item is refused up front ([`DragError::EmptyItem`]): a drag
    /// whose payload is a zero-length string looks like it worked and drops
    /// nothing.
    pub fn is_empty(&self) -> bool {
        match self {
            DragItem::Text(t) => t.is_empty(),
            DragItem::Html { html, .. } => html.is_empty(),
            DragItem::Url(u) => u.is_empty(),
            DragItem::Files(paths) => paths.is_empty(),
            DragItem::Custom { kind, bytes } => kind.is_empty() || bytes.is_empty(),
        }
    }
}

/// A filesystem path as a `file://` URL.
///
/// The form every platform's drag API wants for a file, and a place with three
/// separate traps: a space must become `%20` or the URL truncates at the space;
/// a Windows path must have its separators flipped and gain a leading slash
/// (`C:\a` → `file:///C:/a`); and a non-UTF-8 path has no URL at all, which is
/// `None` rather than a lossy guess.
///
/// ```
/// use silka_platform::drag::file_url;
///
/// assert_eq!(file_url("/tmp/my report.pdf").as_deref(), Some("file:///tmp/my%20report.pdf"));
/// // A Windows path keeps its drive letter and colon, which are legal in a
/// // path segment, and flips its separators.
/// assert_eq!(file_url(r"C:\Users\a b.txt").as_deref(), Some("file:///C:/Users/a%20b.txt"));
/// // Already-absolute POSIX paths are not doubled up.
/// assert_eq!(file_url("/a").as_deref(), Some("file:///a"));
/// ```
pub fn file_url(path: impl AsRef<Path>) -> Option<String> {
    let raw = path.as_ref().to_str()?;
    if raw.is_empty() {
        return None;
    }
    // Windows separators are not legal in a URL path; flip them first so the
    // encoder below never sees one.
    let flipped: String = raw.replace('\\', "/");
    let mut out = String::from("file://");
    if !flipped.starts_with('/') {
        out.push('/');
    }
    for ch in flipped.chars() {
        if is_url_path_safe(ch) {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for byte in ch.encode_utf8(&mut buf).as_bytes() {
                out.push('%');
                out.push(hex_digit(byte >> 4));
                out.push(hex_digit(byte & 0x0F));
            }
        }
    }
    Some(out)
}

/// Whether a character may appear unencoded in a URL path.
///
/// The RFC 3986 unreserved set, plus the two separators a path is made of:
/// `/` between segments and `:` for a Windows drive letter (legal in a path
/// segment, and encoding it makes Explorer reject the URL).
fn is_url_path_safe(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_' | '~' | '/' | ':')
}

fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + nibble - 10) as char,
    }
}

// ---------------------------------------------------------------------------
// Preview
// ---------------------------------------------------------------------------

/// The image that follows the pointer during a drag.
///
/// Without one the drag is invisible: the pointer moves, nothing follows it,
/// and the user cannot tell a drag started at all. That is why [`DragSource`]
/// warns about it ([`DragError::NoPreview`]) instead of quietly producing a
/// ghost drag.
///
/// `hotspot` is where the pointer sits **inside** the image, in logical points
/// from its top-left corner. Getting it right is what makes the dragged thing
/// feel picked up rather than flung: the pointer should stay on the same part
/// of the row it grabbed.
///
/// ```
/// use silka_paint::Point;
/// use silka_platform::drag::DragPreview;
/// use silka_platform::image::RgbaImage;
///
/// let image = RgbaImage::solid(64, 32, [0, 0, 0, 128]).unwrap();
///
/// // Centred is the safe default when the grab point is not known.
/// let centred = DragPreview::centered(image.clone(), 2.0);
/// assert_eq!(centred.hotspot(), Point::new(16.0, 8.0));
///
/// // A hotspot outside the image is clamped rather than trusted: an OS asked
/// // to draw at a negative offset puts the image somewhere else entirely.
/// let odd = DragPreview::new(image, 2.0, Point::new(-40.0, 900.0));
/// assert_eq!(odd.hotspot(), Point::new(0.0, 16.0));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DragPreview {
    image: RgbaImage,
    scale: u32,
    hotspot_x_millis: i32,
    hotspot_y_millis: i32,
}

impl DragPreview {
    /// A preview with an explicit hotspot, in logical points.
    ///
    /// `scale` is the number of image pixels per logical point — pass the
    /// window's scale factor, so a preview rendered for a Retina display is not
    /// twice the size it should be.
    pub fn new(image: RgbaImage, scale: f32, hotspot: Point) -> Self {
        let scale = if scale.is_finite() && scale >= 1.0 {
            scale.round() as u32
        } else {
            1
        };
        let mut preview = Self {
            image,
            scale,
            hotspot_x_millis: 0,
            hotspot_y_millis: 0,
        };
        let clamped = clamp_hotspot(hotspot, preview.size());
        preview.hotspot_x_millis = (clamped.x * 1000.0) as i32;
        preview.hotspot_y_millis = (clamped.y * 1000.0) as i32;
        preview
    }

    /// A preview whose hotspot is its centre.
    pub fn centered(image: RgbaImage, scale: f32) -> Self {
        let mut preview = Self::new(image, scale, Point::ZERO);
        let size = preview.size();
        preview.hotspot_x_millis = (size.width * 500.0) as i32;
        preview.hotspot_y_millis = (size.height * 500.0) as i32;
        preview
    }

    /// The image.
    pub fn image(&self) -> &RgbaImage {
        &self.image
    }

    /// Image pixels per logical point.
    pub fn scale(&self) -> u32 {
        self.scale
    }

    /// The preview's size in **logical points** — pixels divided by the scale.
    pub fn size(&self) -> Size {
        Size::new(
            self.image.width() as f32 / self.scale as f32,
            self.image.height() as f32 / self.scale as f32,
        )
    }

    /// Where the pointer sits inside the image, in logical points from its
    /// top-left corner. Always inside the image.
    pub fn hotspot(&self) -> Point {
        Point::new(
            self.hotspot_x_millis as f32 / 1000.0,
            self.hotspot_y_millis as f32 / 1000.0,
        )
    }
}

/// Pull a hotspot back inside the image it belongs to.
///
/// An out-of-range hotspot is not a cosmetic problem: the OS positions the
/// whole preview by subtracting it from the pointer, so a hotspot of `-40`
/// puts the image forty points away from the pointer that is supposedly
/// holding it.
pub fn clamp_hotspot(hotspot: Point, size: Size) -> Point {
    let clamp = |v: f32, max: f32| {
        if !v.is_finite() {
            0.0
        } else {
            v.clamp(0.0, max.max(0.0))
        }
    };
    Point::new(clamp(hotspot.x, size.width), clamp(hotspot.y, size.height))
}

/// Where the preview image goes, given the pointer position.
///
/// The pointer holds the image at its hotspot, so the image's top-left corner
/// is the pointer minus the hotspot. In logical points, top-left origin — the
/// per-platform backends convert from here.
///
/// ```
/// use silka_paint::{Point, Rect, Size};
/// use silka_platform::drag::preview_frame;
///
/// // Grabbed 8 points in from the left edge: the image hangs to the left of
/// // the pointer by exactly that much.
/// let frame = preview_frame(Size::new(64.0, 24.0), Point::new(8.0, 12.0), Point::new(100.0, 200.0));
/// assert_eq!(frame, Rect::new(92.0, 188.0, 64.0, 24.0));
/// ```
pub fn preview_frame(size: Size, hotspot: Point, pointer: Point) -> Rect {
    let hotspot = clamp_hotspot(hotspot, size);
    Rect::from_origin_size(
        Point::new(pointer.x - hotspot.x, pointer.y - hotspot.y),
        size,
    )
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why a drag could not start.
///
/// The first four are refusals on principle — a drag that starts with no
/// payload, no permitted effect or no visible preview is worse than one that
/// never starts, because the user cannot tell it failed.
///
/// ```
/// use silka_platform::drag::{drag, DragEffects, DragError};
///
/// // Nothing on offer.
/// assert_eq!(drag().check(), Err(DragError::NoItems));
///
/// // Something on offer, but nothing anyone may do with it.
/// assert_eq!(
///     drag().text("x").allow(DragEffects::NONE).check(),
///     Err(DragError::NoEffects)
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DragError {
    /// Nothing was offered.
    NoItems,
    /// One of the offered items carries no payload.
    EmptyItem,
    /// No effect is permitted, so nothing could accept the drop.
    NoEffects,
    /// No preview image. A drag with no preview is invisible.
    NoPreview,
    /// The window handle could not be read.
    NoWindow,
    /// There is no mouse event to hang the drag on.
    ///
    /// macOS starts a dragging session from the event that began it; a drag
    /// kicked off from a timer or a menu has no such event and is refused
    /// rather than started at the wrong place.
    NoEvent,
    /// This platform has no drag source implementation yet. The message says
    /// which and why.
    Unsupported(String),
    /// The OS refused the drag.
    Os(String),
}

impl fmt::Display for DragError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DragError::NoItems => write!(f, "a drag with nothing on offer"),
            DragError::EmptyItem => write!(f, "one of the dragged items is empty"),
            DragError::NoEffects => write!(f, "no drop effect is permitted"),
            DragError::NoPreview => write!(f, "a drag with no preview image is invisible"),
            DragError::NoWindow => write!(f, "the window handle could not be read"),
            DragError::NoEvent => write!(f, "no mouse event to start the drag from"),
            DragError::Unsupported(m) => write!(f, "no drag source on this platform: {m}"),
            DragError::Os(m) => write!(f, "the OS refused the drag: {m}"),
        }
    }
}

impl std::error::Error for DragError {}

/// Whether this build can actually start a drag.
///
/// Answerable without a window, so an application can hide a "drag me" affordance
/// on a platform where it would do nothing.
///
/// ```
/// use silka_platform::drag::is_supported;
///
/// // True on macOS today; see the module documentation for the other two.
/// let _ = is_supported();
/// ```
pub const fn is_supported() -> bool {
    cfg!(target_os = "macos")
}

// ---------------------------------------------------------------------------
// The source
// ---------------------------------------------------------------------------

/// A drag description, built by method chaining.
///
/// A plain value until [`DragSource::begin`]: it can be assembled, inspected
/// and [`DragSource::check`]ed in a unit test with no OS anywhere near it.
///
/// ```
/// use silka_platform::drag::{drag, DragEffects, DragItem};
///
/// let source = drag()
///     // Best representation first: Finder takes the file, a text editor
///     // takes the name.
///     .file("/tmp/report.pdf")
///     .text("report.pdf")
///     .allow(DragEffects::COPY.union(DragEffects::MOVE));
///
/// assert_eq!(source.items().len(), 2);
/// assert_eq!(source.items()[0], DragItem::file("/tmp/report.pdf"));
/// assert!(source.check().is_ok());
/// ```
pub struct DragSource {
    items: Vec<DragItem>,
    allowed: DragEffects,
    preview: Option<DragPreview>,
    on_finish: Option<Box<dyn FnOnce(Option<DragEffect>)>>,
}

impl fmt::Debug for DragSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DragSource")
            .field("items", &self.items)
            .field("allowed", &self.allowed)
            .field("preview", &self.preview)
            .field("on_finish", &self.on_finish.is_some())
            .finish()
    }
}

/// Describe a drag.
///
/// Copy is the only effect allowed until [`DragSource::allow`] says otherwise:
/// a source that accidentally permitted a move would have its data deleted by a
/// drop it never intended to allow.
///
/// ```
/// use silka_platform::drag::{drag, DragEffects};
///
/// assert_eq!(drag().allowed(), DragEffects::COPY);
/// ```
pub fn drag() -> DragSource {
    DragSource {
        items: Vec::new(),
        // The non-destructive default, on purpose.
        allowed: DragEffects::COPY,
        preview: None,
        on_finish: None,
    }
}

impl DragSource {
    /// Offer an item. Order is preference order: best representation first.
    pub fn item(mut self, item: DragItem) -> Self {
        self.items.push(item);
        self
    }

    /// Offer plain text.
    pub fn text(self, text: impl Into<String>) -> Self {
        self.item(DragItem::text(text))
    }

    /// Offer HTML with a plain-text alternative.
    pub fn html(self, html: impl Into<String>, plain: impl Into<String>) -> Self {
        self.item(DragItem::html(html, plain))
    }

    /// Offer a link.
    pub fn url(self, url: impl Into<String>) -> Self {
        self.item(DragItem::url(url))
    }

    /// Offer a file.
    pub fn file(self, path: impl Into<PathBuf>) -> Self {
        self.item(DragItem::file(path))
    }

    /// Offer several files.
    pub fn files<P: Into<PathBuf>>(self, paths: impl IntoIterator<Item = P>) -> Self {
        self.item(DragItem::files(paths))
    }

    /// Offer an application's own type.
    pub fn custom(self, kind: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        self.item(DragItem::custom(kind, bytes))
    }

    /// Which effects a receiver may perform. Defaults to [`DragEffects::COPY`].
    pub fn allow(mut self, effects: DragEffects) -> Self {
        self.allowed = effects;
        self
    }

    /// The image that follows the pointer.
    pub fn preview(mut self, preview: DragPreview) -> Self {
        self.preview = Some(preview);
        self
    }

    /// Called once the drag ends, with the effect the receiver performed —
    /// `None` when the drop was cancelled or landed nowhere.
    ///
    /// This is where a **move** deletes the source's own copy, and it must
    /// happen here rather than when the drag starts: a drag dropped on the
    /// desktop and then cancelled would otherwise have already destroyed the
    /// original.
    pub fn on_finish(mut self, f: impl FnOnce(Option<DragEffect>) + 'static) -> Self {
        self.on_finish = Some(Box::new(f));
        self
    }

    /// The items on offer, in preference order.
    pub fn items(&self) -> &[DragItem] {
        &self.items
    }

    /// The permitted effects.
    pub fn allowed(&self) -> DragEffects {
        self.allowed
    }

    /// The preview, when one was given.
    pub fn preview_image(&self) -> Option<&DragPreview> {
        self.preview.as_ref()
    }

    /// Whether the description is complete enough to hand to the OS.
    ///
    /// Checked separately from [`DragSource::begin`] so an application can
    /// assert it in a test that never touches a window. The preview is **not**
    /// required here — [`DragError::NoPreview`] is only produced by `begin`,
    /// where an invisible drag would actually reach a user.
    pub fn check(&self) -> Result<(), DragError> {
        if self.items.is_empty() {
            return Err(DragError::NoItems);
        }
        if self.items.iter().any(DragItem::is_empty) {
            return Err(DragError::EmptyItem);
        }
        if self.allowed.is_empty() {
            return Err(DragError::NoEffects);
        }
        Ok(())
    }

    /// Start the drag.
    ///
    /// `pointer` is where the pointer is **in the window**, in logical points
    /// from its top-left corner — the same coordinate space the framework's own
    /// input events use, so a call site can pass the position straight out of
    /// the pointer event that started the gesture.
    ///
    /// Returns as soon as the OS owns the drag; the outcome arrives at
    /// [`DragSource::on_finish`], because a dragging session outlives the call
    /// that started it on every platform.
    ///
    /// # Errors
    ///
    /// Everything [`DragSource::check`] catches, plus [`DragError::NoPreview`]
    /// for an invisible drag and [`DragError::Unsupported`] on a platform with
    /// no implementation yet (see the module documentation).
    pub fn begin(self, window: &NativeWindow, pointer: Point) -> Result<(), DragError> {
        self.check()?;
        if self.preview.is_none() {
            return Err(DragError::NoPreview);
        }

        #[cfg(target_os = "macos")]
        {
            macos::begin(self, window, pointer)
        }

        #[cfg(target_os = "windows")]
        {
            let _ = (window, pointer);
            Err(DragError::Unsupported(
                "DoDragDrop needs IDropSource and IDataObject implemented through windows-rs \
                 `implement`, and the Win32_System_Ole feature is not pinned by this workspace yet"
                    .into(),
            ))
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (window, pointer);
            Err(DragError::Unsupported(
                "wl_data_device::start_drag needs the seat and the input-event serial, neither of \
                 which winit exposes"
                    .into(),
            ))
        }
    }

    /// Take the finish callback out of the description.
    ///
    /// Only the per-platform backends need this: the callback has to outlive
    /// `self`, since the drag does.
    #[allow(dead_code)]
    pub(crate) fn take_on_finish(&mut self) -> Option<Box<dyn FnOnce(Option<DragEffect>)>> {
        self.on_finish.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview() -> DragPreview {
        DragPreview::centered(
            RgbaImage::solid(32, 16, [0, 0, 0, 255]).expect("valid size"),
            1.0,
        )
    }

    #[test]
    fn efek_bawaan_adalah_copy_yang_tidak_merusak() {
        // A source that accidentally permitted a move would have its data
        // deleted by a drop it never meant to allow.
        assert_eq!(drag().allowed(), DragEffects::COPY);
        assert_eq!(DragEffects::COPY.preferred(), Some(DragEffect::Copy));
    }

    #[test]
    fn urutan_pilihan_efek_selalu_yang_paling_aman_dulu() {
        assert_eq!(DragEffects::ALL.preferred(), Some(DragEffect::Copy));
        assert_eq!(
            DragEffects::MOVE.union(DragEffects::LINK).preferred(),
            Some(DragEffect::Move)
        );
        assert_eq!(DragEffects::LINK.preferred(), Some(DragEffect::Link));
        assert_eq!(DragEffects::NONE.preferred(), None);
    }

    #[test]
    fn hanya_move_yang_mewajibkan_sumber_menghapus() {
        assert!(DragEffect::Move.source_must_remove());
        assert!(!DragEffect::Copy.source_must_remove());
        assert!(!DragEffect::Link.source_must_remove());
    }

    #[test]
    fn modifier_macos_dan_windows_memang_berbeda() {
        let all = DragEffects::ALL;
        // ⌥ copies on macOS; the same key means nothing on Windows.
        assert_eq!(
            effect_for_modifiers(all, Modifiers::ALT, HostOs::MacOs),
            Some(DragEffect::Copy)
        );
        assert_eq!(
            effect_for_modifiers(all, Modifiers::ALT, HostOs::Windows),
            Some(DragEffect::Copy) // falls back to preferred, not to "alt means copy"
        );
        // ⌘ moves on macOS; Shift moves on Windows.
        assert_eq!(
            effect_for_modifiers(all, Modifiers::META, HostOs::MacOs),
            Some(DragEffect::Move)
        );
        assert_eq!(
            effect_for_modifiers(all, Modifiers::SHIFT, HostOs::Windows),
            Some(DragEffect::Move)
        );
        // Both together is the link gesture on both conventions.
        assert_eq!(
            effect_for_modifiers(all, Modifiers::META.union(Modifiers::ALT), HostOs::MacOs),
            Some(DragEffect::Link)
        );
        assert_eq!(
            effect_for_modifiers(
                all,
                Modifiers::CONTROL.union(Modifiers::SHIFT),
                HostOs::Unix
            ),
            Some(DragEffect::Link)
        );
    }

    #[test]
    fn modifier_yang_meminta_efek_terlarang_tidak_membatalkan_drag() {
        // The OS itself falls back rather than cancelling, and a drag that
        // died because a user leaned on Ctrl would be baffling.
        let copy_only = DragEffects::COPY;
        assert_eq!(
            effect_for_modifiers(copy_only, Modifiers::META, HostOs::MacOs),
            Some(DragEffect::Copy)
        );
        // …but a source that allows nothing still allows nothing.
        assert_eq!(
            effect_for_modifiers(DragEffects::NONE, Modifiers::META, HostOs::MacOs),
            None
        );
    }

    #[test]
    fn setiap_item_punya_nama_di_tiga_platform() {
        let items = [
            DragItem::text("x"),
            DragItem::html("<b>x</b>", "x"),
            DragItem::url("https://example.com"),
            DragItem::file("/tmp/a"),
        ];
        for item in items {
            assert!(!item.uti().is_empty(), "{item:?}");
            assert!(!item.mime().is_empty(), "{item:?}");
            assert!(!item.windows_format().is_empty(), "{item:?}");
        }
    }

    #[test]
    fn tipe_kustom_memakai_nama_yang_sama_di_mana_mana() {
        // The whole point of a reverse-DNS name: an application dragging its
        // own document type between its own windows writes the name once.
        let own = DragItem::custom("com.example.card", vec![9]);
        assert_eq!(own.uti(), "com.example.card");
        assert_eq!(own.mime(), "com.example.card");
        assert_eq!(own.windows_format(), "com.example.card");
    }

    #[test]
    fn url_berkas_menyandikan_spasi_dan_membalik_pemisah_windows() {
        assert_eq!(
            file_url("/tmp/my report.pdf").as_deref(),
            Some("file:///tmp/my%20report.pdf")
        );
        assert_eq!(
            file_url(r"C:\Users\a b.txt").as_deref(),
            Some("file:///C:/Users/a%20b.txt")
        );
        // Non-ASCII is percent-encoded per UTF-8 byte, not per character.
        assert_eq!(file_url("/tmp/é").as_deref(), Some("file:///tmp/%C3%A9"));
        assert_eq!(file_url(""), None);
    }

    #[test]
    fn url_berkas_tidak_menyandikan_karakter_yang_sah() {
        let url = file_url("/a-b_c.d~e/f").expect("ascii path");
        assert_eq!(url, "file:///a-b_c.d~e/f");
        assert!(!url.contains('%'));
    }

    #[test]
    fn hotspot_selalu_di_dalam_gambar() {
        let size = Size::new(64.0, 32.0);
        assert_eq!(
            clamp_hotspot(Point::new(-10.0, 900.0), size),
            Point::new(0.0, 32.0)
        );
        assert_eq!(
            clamp_hotspot(Point::new(f32::NAN, 8.0), size),
            Point::new(0.0, 8.0)
        );
    }

    #[test]
    fn preview_dibagi_scale_supaya_retina_tidak_dobel() {
        // 64 physical pixels at 2× is 32 logical points, not 64.
        let img = RgbaImage::solid(64, 32, [1, 2, 3, 4]).expect("valid size");
        let p = DragPreview::centered(img, 2.0);
        assert_eq!(p.size(), Size::new(32.0, 16.0));
        assert_eq!(p.hotspot(), Point::new(16.0, 8.0));
        assert_eq!(p.scale(), 2);
    }

    #[test]
    fn scale_tidak_masuk_akal_dianggap_satu() {
        let img = RgbaImage::solid(4, 4, [0, 0, 0, 255]).expect("valid size");
        assert_eq!(DragPreview::centered(img.clone(), 0.0).scale(), 1);
        assert_eq!(DragPreview::centered(img, f32::NAN).scale(), 1);
    }

    #[test]
    fn bingkai_preview_menggantung_pada_hotspot() {
        let frame = preview_frame(
            Size::new(64.0, 24.0),
            Point::new(8.0, 12.0),
            Point::new(100.0, 200.0),
        );
        assert_eq!(frame, Rect::new(92.0, 188.0, 64.0, 24.0));
        // The pointer is inside the frame it is holding — the property that
        // makes a drag feel picked up rather than flung.
        assert!(frame.contains(Point::new(100.0, 200.0)));
    }

    #[test]
    fn drag_kosong_ditolak_sebelum_menyentuh_os() {
        assert_eq!(drag().check(), Err(DragError::NoItems));
        assert_eq!(drag().text("").check(), Err(DragError::EmptyItem));
        assert_eq!(
            drag().text("x").allow(DragEffects::NONE).check(),
            Err(DragError::NoEffects)
        );
        assert!(drag().text("x").check().is_ok());
    }

    #[test]
    fn deskripsi_lengkap_bisa_diperiksa_tanpa_window() {
        let source = drag()
            .file("/tmp/report.pdf")
            .text("report.pdf")
            .allow(DragEffects::COPY.union(DragEffects::MOVE))
            .preview(preview());
        assert!(source.check().is_ok());
        assert_eq!(source.items().len(), 2);
        assert!(source.preview_image().is_some());
        assert!(source.allowed().contains(DragEffects::MOVE));
    }

    #[test]
    fn callback_selesai_diambil_sekali_saja() {
        let mut source = drag().text("x").on_finish(|_| {});
        assert!(source.take_on_finish().is_some());
        assert!(source.take_on_finish().is_none());
    }

    #[test]
    fn galat_membawa_penjelasan_yang_bisa_dibaca() {
        assert!(DragError::NoPreview.to_string().contains("invisible"));
        let e = DragError::Unsupported("no serial".into());
        assert!(e.to_string().contains("no serial"));
    }

    #[test]
    fn dukungan_platform_bisa_ditanya_tanpa_window() {
        // An application hides its "drag me" affordance where this is false.
        assert_eq!(is_supported(), cfg!(target_os = "macos"));
    }
}
