//! `icon()` — the Tier 0 monochrome symbol (`KOMPONEN.md`).
//!
//! An icon here is **coverage, not colour** — exactly like a glyph. One filled
//! SVG path is rasterised on the CPU once per size ([`silka_paint::svg`]),
//! stored in the same atlas as every other bitmap, and coloured at draw time by
//! a theme token. So a single chevron bitmap serves `label`, `secondary_label`
//! and `accent`, and an icon beside a label costs no extra draw call.
//!
//! ```
//! use silka_theme::ColorToken;
//! use silka_widgets::{icon, IconName};
//!
//! let chevron = icon(IconName::ChevronRight)
//!     .md()
//!     .color(ColorToken::TertiaryLabel);
//! # let _ = chevron;
//! ```
//!
//! # Size and colour are tokens, never numbers
//!
//! The size comes from the spacing scale ([`silka_theme::SpaceToken`]) and the
//! colour from a role ([`silka_theme::ColorToken`]), so the same call site is
//! right in both presets and both appearances (§2.6, §2.7). The escape hatches
//! are spelled `_raw` so they show up in review.
//!
//! # The built-in set, and your own
//!
//! [`IconName`] is a small set of the symbols a business application cannot
//! avoid — chevrons, a check, a close, a search. It is an enum on purpose: a
//! typo is a compile error rather than a silently blank square, which is the
//! same discipline the colour vocabulary follows.
//!
//! An application with its own artwork uses [`icon_path`], which takes the path
//! data directly and caches it under a key of your choosing:
//!
//! ```
//! use silka_widgets::icon_path;
//!
//! let brand = icon_path("brand/logo", "M4 4 H20 V20 H4 Z", 24.0);
//! # let _ = brand;
//! ```
//!
//! Elliptical arcs (`A`/`a`) are refused by the rasteriser: an arc converted
//! wrongly is a silently misshapen icon, so the artwork is converted to curves
//! by its exporter instead. That is why every path in the built-in set is
//! written with lines and curves only.
//!
//! # Where the built-in artwork comes from
//!
//! The set is **Material Symbols Rounded** (filled, 24dp), used unmodified under
//! Apache-2.0. `crates/widgets/ICONS.md` carries the upstream commit, the
//! per-symbol mapping, and the statement of modification the licence requires.
//!
//! Upstream draws in a `0 -960 960 960` grid — a thousand units square whose Y
//! is **negative** — not the `0 0 24 24` a hand-drawn icon would use. Rather
//! than rewriting every coordinate (one mistyped decimal there is artwork that is
//! subtly wrong and passes every test), the grid travels with the path:
//! [`IconName::view_box`] reports it, and [`silka_paint::ViewBox`] applies it.
//!
//! This matters at the call site only when you add a symbol from the same
//! upstream set: use [`icon_path_in_box`] with [`MATERIAL_SYMBOLS_VIEW_BOX`], not
//! [`icon_path`]. Handing a negative-Y path to [`icon_path`] parses perfectly and
//! draws **nothing**, because the artwork lands above the canvas.
//!
//! # Arrows that mean "back" and "forward" (§9.8)
//!
//! [`IconName::ChevronLeft`] and [`IconName::ChevronRight`] are **physical**:
//! they say which way the artwork points and nothing else. An arrow that means
//! *the previous month* or *the next page* is not physical — it points
//! backward or forward along the reading direction, and in a right-to-left
//! document that is the other way round.
//!
//! Mirroring it is not the caller's job, because a call site has no idea which
//! direction is in force. It is the icon node's, which learns the direction
//! from its [`LayoutCtx`] — the same root
//! `AUDIT.md` P-6 closed for every other self-drawing widget. So the vocabulary
//! is [`chevron_back`]/[`chevron_forward`] (and [`Icon::mirrored`] for your own
//! artwork), never a constant chosen at the call site:
//!
//! ```
//! use silka_widgets::chevron_back;
//!
//! // Points left while the document reads left-to-right, right while it
//! // reads right-to-left — decided by the node, not here.
//! let previous = chevron_back();
//! # let _ = previous;
//! ```
//!
//! # Definition of done
//!
//! | Line | How it is met |
//! |---|---|
//! | Both presets, dark mode | size and colour are tokens |
//! | Interactive states, keyboard, hit target | none: an icon is content. `icon_button` (Tier 2) is what makes one pressable, and it is what owns the ≥ 44pt target |
//! | AccessKit node | decorative by default — an icon inside a button must not be announced twice — and [`Icon::label`] promotes it to a named [`silka_core::access::AccessRole::Image`] when it stands alone |
//! | Reduced motion | nothing moves |
//! | Resolution | rasterised at `size × scale_factor` pixels, so it is sharp on a retina display and re-rasterised when the window moves to another one (§3.3) |

use std::rc::Rc;

use silka_core::access::{AccessNode, AccessRole};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, ImageId, ImageQuad, Rect, Size, ViewBox};
use silka_theme::{ColorToken, SpaceToken, Theme};

use crate::ambient::active_theme;
use crate::images::{active_images, Images};

/// The side of a `0 0 w h` icon grid — the default for **your own** artwork.
///
/// The built-in set does not use this: Material Symbols draws in
/// [`MATERIAL_SYMBOLS_VIEW_BOX`], and each symbol carries its own grid through
/// [`IconName::view_box`].
pub const ICON_VIEWPORT: f32 = 24.0;

/// The grid Material Symbols draws in: a thousand units square, Y running
/// **negative** from the baseline.
///
/// Kept public because it is the one number an application needs in order to add
/// a symbol from the same upstream set through [`icon_path_in_box`] — see
/// `crates/widgets/ICONS.md`.
pub const MATERIAL_SYMBOLS_VIEW_BOX: ViewBox = ViewBox::new(0.0, -960.0, 960.0);

/// The built-in monochrome symbol set.
///
/// Small on purpose: these are the symbols a business application cannot avoid,
/// not a competitor to an icon library. Anything else comes in through
/// [`icon_path`].
///
/// ```
/// use silka_widgets::IconName;
///
/// // Every symbol has a stable name (used as the atlas cache key) and a path
/// // the rasteriser accepts, drawn in the grid the symbol names itself.
/// for name in IconName::ALL {
///     assert!(!name.name().is_empty());
///     let d = name.path();
///     assert!(d.starts_with('M') || d.starts_with('m'));
///     assert_eq!(name.view_box().side, 960.0);
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconName {
    /// A check mark — confirmation, a selected row.
    Check,
    /// A chevron pointing up — a collapsed disclosure, a sort direction.
    ChevronUp,
    /// A chevron pointing down — a dropdown, an expandable row.
    ChevronDown,
    /// A chevron pointing left.
    ChevronLeft,
    /// A chevron pointing right — a disclosure indicator, a breadcrumb step.
    ChevronRight,
    /// A cross — close, clear, remove.
    Close,
    /// A plus — add.
    Plus,
    /// A minus — remove, collapse.
    Minus,
    /// A magnifier — search.
    Search,
    /// Three stacked bars — a menu.
    Menu,
    /// Three dots — an overflow menu.
    Ellipsis,
    /// A circled "i" — information.
    Info,
    /// A triangle with an exclamation — a warning.
    Warning,
    /// A waste bin — delete.
    Trash,
    /// A five-pointed star — favourite, rating.
    Star,
    /// A heart — like.
    Heart,
    /// A person — account, profile.
    User,
    /// A calendar page — a date.
    Calendar,
    /// An arrow into a tray — download, import.
    Download,
    /// An arrow out of a tray — upload, export.
    Upload,
    /// A sun — switch to light mode.
    Sun,
    /// A crescent moon — switch to dark mode.
    Moon,
    /// A bell — notifications.
    Bell,
}

impl IconName {
    /// Every built-in symbol — for the gallery and for completeness tests.
    pub const ALL: [IconName; 23] = [
        IconName::Check,
        IconName::ChevronUp,
        IconName::ChevronDown,
        IconName::ChevronLeft,
        IconName::ChevronRight,
        IconName::Close,
        IconName::Plus,
        IconName::Minus,
        IconName::Search,
        IconName::Menu,
        IconName::Ellipsis,
        IconName::Info,
        IconName::Warning,
        IconName::Trash,
        IconName::Star,
        IconName::Heart,
        IconName::User,
        IconName::Calendar,
        IconName::Download,
        IconName::Upload,
        IconName::Sun,
        IconName::Moon,
        IconName::Bell,
    ];

    /// The symbol's stable name — the atlas cache key, and what a debug dump
    /// shows.
    pub const fn name(self) -> &'static str {
        match self {
            IconName::Check => "check",
            IconName::ChevronUp => "chevron.up",
            IconName::ChevronDown => "chevron.down",
            IconName::ChevronLeft => "chevron.left",
            IconName::ChevronRight => "chevron.right",
            IconName::Close => "close",
            IconName::Plus => "plus",
            IconName::Minus => "minus",
            IconName::Search => "search",
            IconName::Menu => "menu",
            IconName::Ellipsis => "ellipsis",
            IconName::Info => "info",
            IconName::Warning => "warning",
            IconName::Trash => "trash",
            IconName::Star => "star",
            IconName::Heart => "heart",
            IconName::User => "user",
            IconName::Calendar => "calendar",
            IconName::Download => "download",
            IconName::Upload => "upload",
            IconName::Sun => "sun",
            IconName::Moon => "moon",
            IconName::Bell => "bell",
        }
    }

    /// The filled path, in a `0 0 24 24` viewBox.
    ///
    /// Lines and cubics only — see the module docs for why there are no arcs
    /// and no strokes.
    /// The filled path, drawn in the grid [`IconName::view_box`] names.
    ///
    /// Material Symbols Rounded, filled, 24dp, taken unmodified from the
    /// upstream `google/material-design-icons` (Apache-2.0). The coordinates
    /// are therefore in a `0 -960 960 960` grid with **negative Y**, not the
    /// `0 0 24 24` a hand-drawn icon would use — see
    /// `crates/widgets/ICONS.md` for the per-symbol mapping and the licence.
    ///
    /// Lines and curves only: not one of these paths needs an elliptical arc,
    /// which is what lets them stay byte-for-byte upstream (the rasteriser
    /// refuses `A`/`a`).
    pub const fn path(self) -> &'static str {
        match self {
            IconName::Check => {
                "m382-354 339-339q12-12 28-12t28 12q12 12 12 28.5T777-636L410-268 \
                 q-12 12-28 12t-28-12L182-440q-12-12-11.5-28.5T183-497q12-12 28.5-12t28.5 12 \
                 l142 143Z"
            }
            IconName::ChevronUp => {
                "M480-528 324-372q-11 11-28 11t-28-11q-11-11-11-28t11-28l184-184q12-12 28-12 \
                 t28 12l184 184q11 11 11 28t-11 28q-11 11-28 11t-28-11L480-528Z"
            }
            IconName::ChevronDown => {
                "M480-361q-8 0-15-2.5t-13-8.5L268-556q-11-11-11-28t11-28q11-11 28-11t28 11 \
                 l156 156 156-156q11-11 28-11t28 11q11 11 11 28t-11 28L508-372q-6 6-13 8.5 \
                 t-15 2.5Z"
            }
            IconName::ChevronLeft => {
                "m432-480 156 156q11 11 11 28t-11 28q-11 11-28 11t-28-11L348-452q-6-6-8.5-13 \
                 t-2.5-15q0-8 2.5-15t8.5-13l184-184q11-11 28-11t28 11q11 11 11 28t-11 28 \
                 L432-480Z"
            }
            IconName::ChevronRight => {
                "M504-480 348-636q-11-11-11-28t11-28q11-11 28-11t28 11l184 184q6 6 8.5 13 \
                 t2.5 15q0 8-2.5 15t-8.5 13L404-268q-11 11-28 11t-28-11q-11-11-11-28t11-28 \
                 l156-156Z"
            }
            IconName::Close => {
                "M480-424 284-228q-11 11-28 11t-28-11q-11-11-11-28t11-28l196-196-196-196 \
                 q-11-11-11-28t11-28q11-11 28-11t28 11l196 196 196-196q11-11 28-11t28 11 \
                 q11 11 11 28t-11 28L536-480l196 196q11 11 11 28t-11 28q-11 11-28 11t-28-11 \
                 L480-424Z"
            }
            IconName::Plus => {
                "M440-440H240q-17 0-28.5-11.5T200-480q0-17 11.5-28.5T240-520h200v-200 \
                 q0-17 11.5-28.5T480-760q17 0 28.5 11.5T520-720v200h200q17 0 28.5 11.5 \
                 T760-480q0 17-11.5 28.5T720-440H520v200q0 17-11.5 28.5T480-200 \
                 q-17 0-28.5-11.5T440-240v-200Z"
            }
            IconName::Minus => {
                "M240-440q-17 0-28.5-11.5T200-480q0-17 11.5-28.5T240-520h480q17 0 28.5 11.5 \
                 T760-480q0 17-11.5 28.5T720-440H240Z"
            }
            IconName::Search => {
                "M380-320q-109 0-184.5-75.5T120-580q0-109 75.5-184.5T380-840q109 0 184.5 75.5 \
                 T640-580q0 44-14 83t-38 69l224 224q11 11 11 28t-11 28q-11 11-28 11t-28-11 \
                 L532-372q-30 24-69 38t-83 14Zm0-80q75 0 127.5-52.5T560-580q0-75-52.5-127.5 \
                 T380-760q-75 0-127.5 52.5T200-580q0 75 52.5 127.5T380-400Z"
            }
            IconName::Menu => {
                "M160-240q-17 0-28.5-11.5T120-280q0-17 11.5-28.5T160-320h640q17 0 28.5 11.5 \
                 T840-280q0 17-11.5 28.5T800-240H160Zm0-200q-17 0-28.5-11.5T120-480 \
                 q0-17 11.5-28.5T160-520h640q17 0 28.5 11.5T840-480q0 17-11.5 28.5T800-440 \
                 H160Zm0-200q-17 0-28.5-11.5T120-680q0-17 11.5-28.5T160-720h640 \
                 q17 0 28.5 11.5T840-680q0 17-11.5 28.5T800-640H160Z"
            }
            IconName::Ellipsis => {
                "M240-400q-33 0-56.5-23.5T160-480q0-33 23.5-56.5T240-560q33 0 56.5 23.5 \
                 T320-480q0 33-23.5 56.5T240-400Zm240 0q-33 0-56.5-23.5T400-480 \
                 q0-33 23.5-56.5T480-560q33 0 56.5 23.5T560-480q0 33-23.5 56.5T480-400Zm240 0 \
                 q-33 0-56.5-23.5T640-480q0-33 23.5-56.5T720-560q33 0 56.5 23.5T800-480 \
                 q0 33-23.5 56.5T720-400Z"
            }
            IconName::Info => {
                "M480-280q17 0 28.5-11.5T520-320v-160q0-17-11.5-28.5T480-520q-17 0-28.5 11.5 \
                 T440-480v160q0 17 11.5 28.5T480-280Zm0-320q17 0 28.5-11.5T520-640 \
                 q0-17-11.5-28.5T480-680q-17 0-28.5 11.5T440-640q0 17 11.5 28.5T480-600Z \
                 m0 520q-83 0-156-31.5T197-197q-54-54-85.5-127T80-480q0-83 31.5-156T197-763 \
                 q54-54 127-85.5T480-880q83 0 156 31.5T763-763q54 54 85.5 127T880-480 \
                 q0 83-31.5 156T763-197q-54 54-127 85.5T480-80Z"
            }
            IconName::Warning => {
                "M109-120q-11 0-20-5.5T75-140q-5-9-5.5-19.5T75-180l370-640q6-10 15.5-15 \
                 t19.5-5q10 0 19.5 5t15.5 15l370 640q6 10 5.5 20.5T885-140q-5 9-14 14.5 \
                 t-20 5.5H109Zm371-120q17 0 28.5-11.5T520-280q0-17-11.5-28.5T480-320 \
                 q-17 0-28.5 11.5T440-280q0 17 11.5 28.5T480-240Zm0-120q17 0 28.5-11.5 \
                 T520-400v-120q0-17-11.5-28.5T480-560q-17 0-28.5 11.5T440-520v120 \
                 q0 17 11.5 28.5T480-360Z"
            }
            IconName::Trash => {
                "M280-120q-33 0-56.5-23.5T200-200v-520q-17 0-28.5-11.5T160-760q0-17 11.5-28.5 \
                 T200-800h160q0-17 11.5-28.5T400-840h160q17 0 28.5 11.5T600-800h160 \
                 q17 0 28.5 11.5T800-760q0 17-11.5 28.5T760-720v520q0 33-23.5 56.5T680-120 \
                 H280Zm120-160q17 0 28.5-11.5T440-320v-280q0-17-11.5-28.5T400-640 \
                 q-17 0-28.5 11.5T360-600v280q0 17 11.5 28.5T400-280Zm160 0q17 0 28.5-11.5 \
                 T600-320v-280q0-17-11.5-28.5T560-640q-17 0-28.5 11.5T520-600v280 \
                 q0 17 11.5 28.5T560-280Z"
            }
            IconName::Star => {
                "M480-269 314-169q-11 7-23 6t-21-8q-9-7-14-17.5t-2-23.5l44-189-147-127 \
                 q-10-9-12.5-20.5T140-571q4-11 12-18t22-9l194-17 75-178q5-12 15.5-18t21.5-6 \
                 q11 0 21.5 6t15.5 18l75 178 194 17q14 2 22 9t12 18q4 11 1.5 22.5T809-528 \
                 L662-401l44 189q3 13-2 23.5T690-171q-9 7-21 8t-23-6L480-269Z"
            }
            IconName::Heart => {
                "M480-147q-14 0-28.5-5T426-168l-69-63q-106-97-191.5-192.5T80-634q0-94 63-157 \
                 t157-63q53 0 100 22.5t80 61.5q33-39 80-61.5T660-854q94 0 157 63t63 157 \
                 q0 115-85 211T602-230l-68 62q-11 11-25.5 16t-28.5 5Z"
            }
            IconName::User => {
                "M480-480q-66 0-113-47t-47-113q0-66 47-113t113-47q66 0 113 47t47 113 \
                 q0 66-47 113t-113 47ZM160-240v-32q0-34 17.5-62.5T224-378q62-31 126-46.5 \
                 T480-440q66 0 130 15.5T736-378q29 15 46.5 43.5T800-272v32q0 33-23.5 56.5 \
                 T720-160H240q-33 0-56.5-23.5T160-240Z"
            }
            IconName::Calendar => {
                "M200-80q-33 0-56.5-23.5T120-160v-560q0-33 23.5-56.5T200-800h40v-40 \
                 q0-17 11.5-28.5T280-880q17 0 28.5 11.5T320-840v40h320v-40q0-17 11.5-28.5 \
                 T680-880q17 0 28.5 11.5T720-840v40h40q33 0 56.5 23.5T840-720v560 \
                 q0 33-23.5 56.5T760-80H200Zm0-80h560v-400H200v400Zm280-240q-17 0-28.5-11.5 \
                 T440-440q0-17 11.5-28.5T480-480q17 0 28.5 11.5T520-440q0 17-11.5 28.5 \
                 T480-400Zm-160 0q-17 0-28.5-11.5T280-440q0-17 11.5-28.5T320-480 \
                 q17 0 28.5 11.5T360-440q0 17-11.5 28.5T320-400Zm320 0q-17 0-28.5-11.5 \
                 T600-440q0-17 11.5-28.5T640-480q17 0 28.5 11.5T680-440q0 17-11.5 28.5 \
                 T640-400ZM480-240q-17 0-28.5-11.5T440-280q0-17 11.5-28.5T480-320 \
                 q17 0 28.5 11.5T520-280q0 17-11.5 28.5T480-240Zm-160 0q-17 0-28.5-11.5 \
                 T280-280q0-17 11.5-28.5T320-320q17 0 28.5 11.5T360-280q0 17-11.5 28.5 \
                 T320-240Zm320 0q-17 0-28.5-11.5T600-280q0-17 11.5-28.5T640-320 \
                 q17 0 28.5 11.5T680-280q0 17-11.5 28.5T640-240Z"
            }
            IconName::Download => {
                "M480-337q-8 0-15-2.5t-13-8.5L308-492q-12-12-11.5-28t11.5-28q12-12 28.5-12.5 \
                 T365-549l75 75v-286q0-17 11.5-28.5T480-800q17 0 28.5 11.5T520-760v286l75-75 \
                 q12-12 28.5-11.5T652-548q11 12 11.5 28T652-492L508-348q-6 6-13 8.5t-15 2.5Z \
                 M240-160q-33 0-56.5-23.5T160-240v-80q0-17 11.5-28.5T200-360q17 0 28.5 11.5 \
                 T240-320v80h480v-80q0-17 11.5-28.5T760-360q17 0 28.5 11.5T800-320v80 \
                 q0 33-23.5 56.5T720-160H240Z"
            }
            IconName::Upload => {
                "M240-160q-33 0-56.5-23.5T160-240v-80q0-17 11.5-28.5T200-360q17 0 28.5 11.5 \
                 T240-320v80h480v-80q0-17 11.5-28.5T760-360q17 0 28.5 11.5T800-320v80 \
                 q0 33-23.5 56.5T720-160H240Zm200-486-75 75q-12 12-28.5 11.5T308-572 \
                 q-11-12-11.5-28t11.5-28l144-144q6-6 13-8.5t15-2.5q8 0 15 2.5t13 8.5l144 144 \
                 q12 12 11.5 28T652-572q-12 12-28.5 12.5T595-571l-75-75v286q0 17-11.5 28.5 \
                 T480-320q-17 0-28.5-11.5T440-360v-286Z"
            }
            IconName::Sun => {
                "M480-280q-83 0-141.5-58.5T280-480q0-83 58.5-141.5T480-680q83 0 141.5 58.5 \
                 T680-480q0 83-58.5 141.5T480-280ZM80-440q-17 0-28.5-11.5T40-480 \
                 q0-17 11.5-28.5T80-520h80q17 0 28.5 11.5T200-480q0 17-11.5 28.5T160-440H80Z \
                 m720 0q-17 0-28.5-11.5T760-480q0-17 11.5-28.5T800-520h80q17 0 28.5 11.5 \
                 T920-480q0 17-11.5 28.5T880-440h-80ZM480-760q-17 0-28.5-11.5T440-800v-80 \
                 q0-17 11.5-28.5T480-920q17 0 28.5 11.5T520-880v80q0 17-11.5 28.5T480-760Z \
                 m0 720q-17 0-28.5-11.5T440-80v-80q0-17 11.5-28.5T480-200q17 0 28.5 11.5 \
                 T520-160v80q0 17-11.5 28.5T480-40ZM226-678l-43-42q-12-11-11.5-28t11.5-29 \
                 q12-12 29-12t28 12l42 43q11 12 11 28t-11 28q-11 12-27.5 11.5T226-678Z \
                 m494 495-42-43q-11-12-11-28.5t11-27.5q11-12 27.5-11.5T734-282l43 42 \
                 q12 11 11.5 28T777-183q-12 12-29 12t-28-12Zm-42-495q-12-11-11.5-27.5T678-734 \
                 l42-43q11-12 28-11.5t29 11.5q12 12 12 29t-12 28l-43 42q-12 11-28 11t-28-11Z \
                 M183-183q-12-12-12-29t12-28l43-42q12-11 28.5-11t27.5 11q12 11 11.5 27.5 \
                 T282-226l-42 43q-11 12-28 11.5T183-183Z"
            }
            IconName::Moon => {
                "M480-120q-151 0-255.5-104.5T120-480q0-138 90-239.5T440-838q13-2 23 3.5 \
                 t16 14.5q6 9 6.5 21t-7.5 23q-17 26-25.5 55t-8.5 61q0 90 63 153t153 63 \
                 q31 0 61.5-9t54.5-25q11-7 22.5-6.5T819-479q10 5 15.5 15t3.5 24 \
                 q-14 138-117.5 229T480-120Z"
            }
            IconName::Bell => {
                "M200-200q-17 0-28.5-11.5T160-240q0-17 11.5-28.5T200-280h40v-280 \
                 q0-83 50-147.5T420-792v-28q0-25 17.5-42.5T480-880q25 0 42.5 17.5T540-820v28 \
                 q80 20 130 84.5T720-560v280h40q17 0 28.5 11.5T800-240q0 17-11.5 28.5T760-200 \
                 H200ZM480-80q-33 0-56.5-23.5T400-160h160q0 33-23.5 56.5T480-80Z"
            }
        }
    }

    /// The grid this symbol's path is drawn in.
    ///
    /// Every built-in symbol answers [`MATERIAL_SYMBOLS_VIEW_BOX`], because
    /// the whole set comes from one upstream. It is a method rather than a
    /// constant so that adding a symbol from a different set later is a change
    /// to one arm instead of a change to the contract.
    pub const fn view_box(self) -> ViewBox {
        MATERIAL_SYMBOLS_VIEW_BOX
    }
}

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// The icon leaf: a square of `size` points holding one tinted coverage mask.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_widgets::{icon, IconName};
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, icon(IconName::Check).size_raw(16.0));
/// tree.layout(BoxConstraints::loose(Size::new(200.0, 200.0)));
/// assert_eq!(
///     tree.size(tree.children(tree.root())[0]),
///     Size::new(16.0, 16.0)
/// );
/// ```
pub struct IconBox {
    key: Rc<str>,
    path: Rc<str>,
    /// The artwork to use instead while the document reads right-to-left;
    /// `None` = this symbol looks the same either way (§9.8).
    mirror: Option<(Rc<str>, Rc<str>)>,
    view_box: ViewBox,
    size: f32,
    color: Color,
    label: Option<String>,
    images: Images,

    // -- derived (always a product of the fields above) --
    /// The atlas handle for the current pixel size, once rasterised.
    image: Option<ImageId>,
    /// The pixel size the mask above was rasterised at (`0` = never).
    rasterized_px: u32,
    /// The reading direction the mask above was chosen for.
    rasterized_rtl: bool,
}

impl IconBox {
    /// Make sure a mask exists for the size, the resolution **and the reading
    /// direction** in force.
    ///
    /// There are exactly three reasons to rasterise again, and all three are
    /// here: the point size changed, the display scale factor changed (a
    /// coverage mask is tied to a pixel grid, §3.3), and — for a directional
    /// symbol — the document flipped between LTR and RTL (§9.8). The two
    /// artworks are separate atlas entries under separate keys, so flipping
    /// back costs nothing the second time.
    fn ensure_mask(&mut self, rtl: bool) {
        let px = (self.size * self.images.scale_factor()).round().max(0.0) as u32;
        if self.image.is_some() && self.rasterized_px == px && self.rasterized_rtl == rtl {
            return;
        }
        let (key, path) = self.artwork(rtl);
        self.rasterized_px = px;
        self.rasterized_rtl = rtl;
        self.image = self.images.icon_in(&key, &path, self.view_box, px);
    }

    /// The artwork this icon draws in a given reading direction.
    fn artwork(&self, rtl: bool) -> (Rc<str>, Rc<str>) {
        match (&self.mirror, rtl) {
            (Some((key, path)), true) => (key.clone(), path.clone()),
            _ => (self.key.clone(), self.path.clone()),
        }
    }

    /// The atlas handle currently in use, for tests and inspectors.
    pub fn image_id(&self) -> Option<ImageId> {
        self.image
    }

    /// True when this symbol swaps its artwork in a right-to-left document.
    pub fn is_directional(&self) -> bool {
        self.mirror.is_some()
    }
}

impl std::fmt::Debug for IconBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IconBox")
            .field("key", &self.key)
            .field("size", &self.size)
            .field("px", &self.rasterized_px)
            .finish()
    }
}

impl RenderNode for IconBox {
    fn type_name(&self) -> &'static str {
        "Icon"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // Rasterisation happens here rather than in `paint`, which only has a
        // shared reference — and here is also where the scale factor **and the
        // reading direction** in force are known to be current. A "back" arrow
        // is mirrored here, at the node, rather than by a caller who cannot
        // know which direction the document is in (§9.8).
        self.ensure_mask(ctx.direction().is_rtl());
        constraints.constrain(Size::new(self.size, self.size))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let Some(id) = self.image else {
            // A path the rasteriser refused: nothing is drawn, and nothing
            // wrong is drawn either.
            return;
        };
        if self.color.a <= 0.0 {
            return;
        }
        let box_size = ctx.size();
        // The mask is square; if layout squeezed the box, the icon shrinks
        // inside it rather than being stretched.
        let side = self.size.min(box_size.width).min(box_size.height);
        let rect = Rect::new(
            (box_size.width - side) * 0.5,
            (box_size.height - side) * 0.5,
            side,
            side,
        );
        if rect.size.is_empty() {
            return;
        }
        // Coverage in, colour from a token: the same trick that lets one glyph
        // bitmap serve every text colour.
        ctx.image(ImageQuad::new(rect, id).tint(self.color));
    }

    fn access(&self, node: &mut AccessNode) {
        match &self.label {
            Some(label) => {
                node.role = AccessRole::Image;
                node.label = Some(label.clone());
            }
            // Decorative by default: the chevron inside a disclosure row must
            // not be announced after the row has already said "expandable".
            None => node.role = AccessRole::Container,
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props for the icon leaf.
#[derive(Debug, Clone, PartialEq)]
pub struct IconProps {
    key: Rc<str>,
    path: Rc<str>,
    mirror: Option<(Rc<str>, Rc<str>)>,
    view_box: ViewBox,
    size: f32,
    color: Color,
    label: Option<String>,
    images: Images,
}

impl ViewNode for IconProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(IconBox {
            key: self.key.clone(),
            path: self.path.clone(),
            mirror: self.mirror.clone(),
            view_box: self.view_box,
            size: self.size,
            color: self.color,
            label: self.label.clone(),
            images: self.images.clone(),
            image: None,
            rasterized_px: 0,
            rasterized_rtl: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<IconBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        let artwork_changed = n.key != self.key
            || n.path != self.path
            || n.mirror != self.mirror
            || n.view_box != self.view_box;
        if artwork_changed || n.size != self.size || n.images != self.images {
            n.key.clone_from(&self.key);
            n.path.clone_from(&self.path);
            n.mirror.clone_from(&self.mirror);
            n.view_box = self.view_box;
            n.size = self.size;
            n.images = self.images.clone();
            // Force a fresh rasterisation; layout is where it happens, because
            // that is where the scale factor is read.
            n.image = None;
            n.rasterized_px = 0;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.color != self.color {
            n.color = self.color;
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

/// A Dart-style icon builder (§2.5).
///
/// Created through [`icon()`] or [`icon_path`]; becomes a [`View`] as soon as
/// it is placed into any container.
#[derive(Debug, Clone, PartialEq)]
pub struct Icon {
    props: IconProps,
    theme: Theme,
    key: Option<Key>,
}

/// One symbol from the built-in set — the `icon` component (`KOMPONEN.md`
/// Tier 0).
///
/// The atlas comes from [`active_images`] and the tokens from the ambient
/// theme, so neither appears at the call site.
///
/// ```
/// use silka_theme::ColorToken;
/// use silka_widgets::{icon, IconName};
///
/// let disclosure = icon(IconName::ChevronRight).color(ColorToken::TertiaryLabel);
/// # let _ = disclosure;
/// ```
pub fn icon(name: IconName) -> Icon {
    icon_in(&active_images(), &active_theme(), name)
}

/// [`icon()`] with the atlas and the theme passed explicitly — for views built
/// outside a build pass.
pub fn icon_in(images: &Images, theme: &Theme, name: IconName) -> Icon {
    build(images, theme, name.name(), name.path(), name.view_box())
}

/// An icon from **your own** path data, cached under `key`.
///
/// `viewport` is the side of the artwork's `viewBox` in user units (24.0 for a
/// `0 0 24 24` set). Pick a `key` that is stable and unique per artwork: it is
/// what stops the same path being rasterised twice.
///
/// ```
/// use silka_widgets::icon_path;
///
/// let logo = icon_path("brand/mark", "M2 12 L12 2 L22 12 L12 22 Z", 24.0);
/// # let _ = logo;
/// ```
pub fn icon_path(key: impl AsRef<str>, path: impl AsRef<str>, viewport: f32) -> Icon {
    icon_path_in(
        &active_images(),
        &active_theme(),
        key.as_ref(),
        path.as_ref(),
        viewport,
    )
}

/// [`icon_path`] with the atlas and the theme passed explicitly.
pub fn icon_path_in(images: &Images, theme: &Theme, key: &str, path: &str, viewport: f32) -> Icon {
    build(images, theme, key, path, ViewBox::square(viewport))
}

/// [`icon_path`] for artwork whose `viewBox` does **not** start at `0 0`.
///
/// This is what adding another symbol from the same upstream set as the built-ins
/// needs, because Material Symbols draws in a negative-Y grid
/// ([`MATERIAL_SYMBOLS_VIEW_BOX`]). Passing such a path to [`icon_path`] parses
/// perfectly and draws **nothing** — the grid has to come with it.
///
/// ```
/// use silka_widgets::{icon_path_in_box, MATERIAL_SYMBOLS_VIEW_BOX};
///
/// // `bookmark` from Material Symbols Rounded, filled — upstream, unmodified.
/// let saved = icon_path_in_box(
///     "material/bookmark",
///     "m480-240-168 72q-40 17-76-6.5T200-241v-519q0-33 23.5-56.5T280-840h400q33 0 \
///      56.5 23.5T760-760v519q0 43-36 66.5t-76 6.5l-168-72Z",
///     MATERIAL_SYMBOLS_VIEW_BOX,
/// );
/// # let _ = saved;
/// ```
pub fn icon_path_in_box(key: impl AsRef<str>, path: impl AsRef<str>, view_box: ViewBox) -> Icon {
    build(
        &active_images(),
        &active_theme(),
        key.as_ref(),
        path.as_ref(),
        view_box,
    )
}

/// [`icon_path_in_box`] with the atlas and the theme passed explicitly.
pub fn icon_path_in_box_in(
    images: &Images,
    theme: &Theme,
    key: &str,
    path: &str,
    view_box: ViewBox,
) -> Icon {
    build(images, theme, key, path, view_box)
}

/// The chevron that points **backward** along the reading direction — the
/// previous month, the previous page (§9.8).
///
/// Left in a left-to-right document, right in a right-to-left one, and the
/// swap happens inside the node: see [`IconBox::layout`].
pub fn chevron_back() -> Icon {
    chevron_back_in(&active_images(), &active_theme())
}

/// [`chevron_back`] with the atlas and the theme passed explicitly.
pub fn chevron_back_in(images: &Images, theme: &Theme) -> Icon {
    icon_in(images, theme, IconName::ChevronLeft).mirrored(IconName::ChevronRight)
}

/// The chevron that points **forward** along the reading direction — the next
/// month, the next page, a disclosure indicator (§9.8).
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree, TextDirection};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{chevron_forward_in, icon_in, IconBox, IconName, Images};
///
/// let images = Images::new();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let mask = |view, direction| {
///     let mut tree = RenderTree::new();
///     reconcile(&mut tree, view);
///     tree.set_direction(direction);
///     tree.layout(BoxConstraints::loose(Size::new(64.0, 64.0)));
///     let id = tree.children(tree.root())[0];
///     tree.node_ref::<IconBox>(id).unwrap().image_id()
/// };
///
/// // Forward is the right-pointing artwork while reading left-to-right…
/// assert_eq!(
///     mask(chevron_forward_in(&images, &theme), TextDirection::Ltr),
///     mask(icon_in(&images, &theme, IconName::ChevronRight), TextDirection::Ltr),
/// );
/// // …and the left-pointing one while reading right-to-left.
/// assert_eq!(
///     mask(chevron_forward_in(&images, &theme), TextDirection::Rtl),
///     mask(icon_in(&images, &theme, IconName::ChevronLeft), TextDirection::Ltr),
/// );
/// ```
pub fn chevron_forward() -> Icon {
    chevron_forward_in(&active_images(), &active_theme())
}

/// [`chevron_forward`] with the atlas and the theme passed explicitly.
pub fn chevron_forward_in(images: &Images, theme: &Theme) -> Icon {
    icon_in(images, theme, IconName::ChevronRight).mirrored(IconName::ChevronLeft)
}

fn build(images: &Images, theme: &Theme, key: &str, path: &str, view_box: ViewBox) -> Icon {
    Icon {
        props: IconProps {
            key: Rc::from(key),
            path: Rc::from(path),
            mirror: None,
            view_box: if view_box.side.is_finite() && view_box.side > 0.0 {
                view_box
            } else {
                ViewBox::square(ICON_VIEWPORT)
            },
            size: theme.space_of(SpaceToken::S4),
            color: theme.color_of(ColorToken::Label),
            label: None,
            images: images.clone(),
        },
        theme: *theme,
        key: None,
    }
}

impl Icon {
    fn map(mut self, f: impl FnOnce(&mut IconProps)) -> Self {
        f(&mut self.props);
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The side of the icon, named by a spacing token — **the** way to size an
    /// icon, so a brand preset with a different scale moves them all together.
    pub fn size(self, token: SpaceToken) -> Self {
        let v = self.theme.space_of(token);
        self.map(move |p| p.size = v)
    }

    /// **Escape hatch**: a size that is not on the scale — an icon matched to a
    /// measured cap height, for instance.
    pub fn size_raw(self, size: f32) -> Self {
        let size = if size.is_finite() { size.max(0.0) } else { 0.0 };
        self.map(move |p| p.size = size)
    }

    /// 16pt — inline with body text, the default.
    pub fn sm(self) -> Self {
        self.size(SpaceToken::S4)
    }

    /// 20pt — toolbars and list rows.
    pub fn md(self) -> Self {
        self.size(SpaceToken::S5)
    }

    /// 24pt — a section header, a large button.
    pub fn lg(self) -> Self {
        self.size(SpaceToken::S6)
    }

    /// 32pt — an empty state, a feature tile.
    pub fn xl(self) -> Self {
        self.size(SpaceToken::S8)
    }

    /// The colour, named by its role.
    pub fn color(self, token: ColorToken) -> Self {
        let color = self.theme.color_of(token);
        self.map(move |p| p.color = color)
    }

    /// **Escape hatch**: a colour that is not a token.
    pub fn color_raw(self, color: Color) -> Self {
        self.map(move |p| p.color = color)
    }

    /// The name a screen reader announces.
    ///
    /// Without one the icon is **decorative** and disappears from the a11y
    /// tree — which is right for the chevron inside a row that has already
    /// announced itself, and wrong for an icon that is the only thing saying
    /// what a control does.
    pub fn label(self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.map(move |p| p.label = Some(label))
    }

    /// Mark the icon as decoration, dropping any name it had.
    pub fn decorative(self) -> Self {
        self.map(move |p| p.label = None)
    }

    /// The artwork to draw instead while the document reads right-to-left
    /// (§9.8).
    ///
    /// For arrows that mean **back**/**forward** rather than left/right. The
    /// swap is made by the render node, which reads the direction from its
    /// layout context — a call site cannot know it, and a call site that
    /// guesses is exactly the bug `AUDIT.md` P-6 is about.
    ///
    /// Leave it unset for symbols that mean a *physical* side ("collapse the
    /// left panel") and for symbols that are their own mirror image.
    pub fn mirrored(self, name: IconName) -> Self {
        self.mirrored_path(name.name(), name.path())
    }

    /// [`Icon::mirrored`] with your own artwork.
    ///
    /// `key` must differ from this icon's own key: the two forms are two
    /// entries in the atlas cache, and reusing one key would draw the first
    /// bitmap for both.
    pub fn mirrored_path(self, key: impl AsRef<str>, path: impl AsRef<str>) -> Self {
        let pair = (Rc::from(key.as_ref()), Rc::from(path.as_ref()));
        self.map(move |p| p.mirror = Some(pair))
    }

    /// True when this icon swaps its artwork in a right-to-left document.
    pub fn is_directional(&self) -> bool {
        self.props.mirror.is_some()
    }

    /// The side this icon will take, in logical points.
    pub fn size_value(&self) -> f32 {
        self.props.size
    }

    /// The colour this icon will be tinted with.
    pub fn color_value(&self) -> Color {
        self.props.color
    }
}

impl From<Icon> for View {
    fn from(i: Icon) -> View {
        let mut b = Builder::new(i.props);
        if let Some(key) = i.key {
            b = b.key(key);
        }
        b.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::tree::{RenderTree, TextDirection};
    use silka_core::view::reconcile;
    use silka_paint::{Command, Scene};
    use silka_theme::{Appearance, Preset};

    const BOX: Size = Size::new(200.0, 200.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn tree_of(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(BOX));
        tree
    }

    fn quad(tree: &mut RenderTree) -> Option<ImageQuad> {
        let mut scene = Scene::new(Color::BLACK);
        tree.paint_into(&mut scene);
        scene.commands().iter().find_map(|c| match c {
            Command::Image(q) => Some(*q),
            _ => None,
        })
    }

    #[test]
    fn every_built_in_path_rasterises() {
        let images = Images::new();
        for name in IconName::ALL {
            assert!(
                images
                    .icon_in(name.name(), name.path(), name.view_box(), 24)
                    .is_some(),
                "'{}' is not a path the rasteriser accepts",
                name.name()
            );
        }
    }

    /// Accepting the path is not the same as drawing something, and the
    /// difference is the whole reason [`ViewBox`] exists: a Material Symbols path
    /// mapped in a `0 0 24 24` grid parses perfectly and rasterises to an empty
    /// mask. `is_some()` would pass. Coverage is what actually proves it.
    #[test]
    fn every_built_in_icon_actually_draws_pixels() {
        use silka_paint::{rasterize_path_in, FillRule};

        for name in IconName::ALL {
            let mask = rasterize_path_in(name.path(), name.view_box(), 48, FillRule::NonZero)
                .unwrap_or_else(|| panic!("'{}' should rasterise", name.name()));
            let ink: u32 = mask.alpha().iter().map(|&a| u32::from(a)).sum();
            assert!(
                ink > 0,
                "'{}' rasterised to a blank mask — the viewBox is wrong, not the path",
                name.name()
            );

            // A symbol that covered almost nothing would also be a bug worth
            // catching: a stray decimal point shrinks artwork rather than
            // erasing it.
            let lit = mask.alpha().iter().filter(|&&a| a > 32).count();
            assert!(
                lit >= 48,
                "'{}' covers only {lit} px of 48x48 — suspiciously small",
                name.name()
            );
        }
    }

    /// The trap, stated as a test: drop the grid and every icon goes blank.
    #[test]
    fn built_in_paths_are_blank_without_their_view_box() {
        use silka_paint::{rasterize_path, FillRule};

        let blank = IconName::ALL
            .iter()
            .filter(|name| {
                rasterize_path(name.path(), ICON_VIEWPORT, 48, FillRule::NonZero)
                    .map(|m| m.alpha().iter().all(|&a| a == 0))
                    .unwrap_or(true)
            })
            .count();
        assert_eq!(
            blank,
            IconName::ALL.len(),
            "the built-in set is drawn in a negative-Y grid, so ICON_VIEWPORT \
             alone must not be able to render it"
        );
    }

    #[test]
    fn every_built_in_name_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for name in IconName::ALL {
            assert!(
                seen.insert(name.name()),
                "'{}' is used twice — the second one would draw the first one's \
                 bitmap out of the cache",
                name.name()
            );
        }
    }

    #[test]
    fn an_icon_is_a_square_of_the_size_token() {
        let t = theme();
        let images = Images::new();
        let mut tree = tree_of(icon_in(&images, &t, IconName::Check).md());
        let size = tree.size(tree.children(tree.root())[0]);
        assert_eq!(
            size,
            Size::new(t.space_of(SpaceToken::S5), t.space_of(SpaceToken::S5))
        );
        assert!(quad(&mut tree).is_some(), "an icon has to draw something");
    }

    #[test]
    fn the_colour_comes_from_a_token_in_every_preset_and_appearance() {
        let images = Images::new();
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut tree = tree_of(
                    icon_in(&images, &t, IconName::Check).color(ColorToken::SecondaryLabel),
                );
                let q = quad(&mut tree).expect("an image command");
                assert_eq!(
                    q.tint,
                    t.color_of(ColorToken::SecondaryLabel),
                    "{preset:?} {appearance:?}"
                );
            }
        }
    }

    #[test]
    fn one_bitmap_serves_every_colour() {
        // The whole reason an icon is coverage and not colour: two tints must
        // not cost two entries in the atlas.
        let images = Images::new();
        let t = theme();
        let mut a = tree_of(icon_in(&images, &t, IconName::Check).color(ColorToken::Label));
        let mut b = tree_of(icon_in(&images, &t, IconName::Check).color(ColorToken::Accent));
        let qa = quad(&mut a).unwrap();
        let qb = quad(&mut b).unwrap();
        assert_eq!(qa.image, qb.image);
        assert_ne!(qa.tint, qb.tint);
    }

    #[test]
    fn a_higher_scale_factor_rasterises_a_bigger_bitmap() {
        let images = Images::new();
        let t = theme();
        let mut one_x = tree_of(icon_in(&images, &t, IconName::Check).size_raw(16.0));
        let a = quad(&mut one_x).unwrap().image;

        images.set_scale_factor(2.0);
        let mut two_x = tree_of(icon_in(&images, &t, IconName::Check).size_raw(16.0));
        let b = quad(&mut two_x).unwrap().image;
        assert_ne!(a, b, "a coverage mask is tied to the pixel grid (§3.3)");
    }

    #[test]
    fn a_refused_path_draws_nothing_rather_than_a_blank_square() {
        let images = Images::new();
        let t = theme();
        // Elliptical arcs are exactly what the rasteriser refuses.
        let mut tree = tree_of(icon_path_in(
            &images,
            &t,
            "broken",
            "M0 0 A1 1 0 0 1 2 2",
            24.0,
        ));
        assert!(quad(&mut tree).is_none());
    }

    #[test]
    fn a_named_icon_is_content_and_an_unnamed_one_is_decoration() {
        let images = Images::new();
        let t = theme();
        let tree = tree_of(icon_in(&images, &t, IconName::Search).label("Search"));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Search")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Image);

        let quiet = tree_of(icon_in(&images, &t, IconName::Search));
        let a11y = quiet.access_tree(None);
        assert!(
            a11y.entries()
                .iter()
                .all(|e| e.node.role != AccessRole::Image),
            "an icon inside a control must not be announced twice:\n{}",
            a11y.dump()
        );
    }

    #[test]
    fn rebuilding_an_identical_icon_does_nothing() {
        let images = Images::new();
        let t = theme();
        let build = || icon_in(&images, &t, IconName::Plus).md();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, build());
        tree.layout(BoxConstraints::loose(BOX));
        assert!(reconcile(&mut tree, build()).is_noop());
    }

    #[test]
    fn a_custom_path_is_cached_under_its_own_key() {
        let images = Images::new();
        let t = theme();
        let d = "M2 12 L12 2 L22 12 L12 22 Z";
        let mut a = tree_of(icon_path_in(&images, &t, "brand", d, 24.0).size_raw(16.0));
        let mut b = tree_of(icon_path_in(&images, &t, "brand", d, 24.0).size_raw(16.0));
        assert_eq!(quad(&mut a).unwrap().image, quad(&mut b).unwrap().image);
    }

    // -- RTL (§9.8, `AUDIT.md` P-6) ------------------------------------------

    /// Build one icon in one reading direction and return the bitmap it draws.
    fn mask_in(view: impl Into<View>, direction: TextDirection) -> Option<ImageId> {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.set_direction(direction);
        tree.layout(BoxConstraints::loose(BOX));
        quad(&mut tree).map(|q| q.image)
    }

    #[test]
    fn a_back_arrow_swaps_its_artwork_between_ltr_and_rtl() {
        let images = Images::new();
        let t = theme();
        let ltr = mask_in(chevron_back_in(&images, &t).sm(), TextDirection::Ltr);
        let rtl = mask_in(chevron_back_in(&images, &t).sm(), TextDirection::Rtl);
        assert!(ltr.is_some() && rtl.is_some());
        assert_ne!(
            ltr, rtl,
            "'back' points the other way in an RTL document (§9.8)"
        );

        // And it is not just *some* other bitmap: it is exactly the opposite
        // chevron, i.e. what a reader of that document expects to see.
        let kiri = mask_in(
            icon_in(&images, &t, IconName::ChevronLeft).sm(),
            TextDirection::Ltr,
        );
        let kanan = mask_in(
            icon_in(&images, &t, IconName::ChevronRight).sm(),
            TextDirection::Ltr,
        );
        assert_eq!(ltr, kiri);
        assert_eq!(rtl, kanan);
    }

    #[test]
    fn back_and_forward_point_at_each_other_in_both_directions() {
        let images = Images::new();
        let t = theme();
        for direction in [TextDirection::Ltr, TextDirection::Rtl] {
            let mundur = mask_in(chevron_back_in(&images, &t).sm(), direction);
            let maju = mask_in(chevron_forward_in(&images, &t).sm(), direction);
            assert_ne!(
                mundur, maju,
                "previous and next must never draw the same arrow ({direction:?})"
            );
        }
        // Forward in RTL is backward in LTR: the pair simply swaps.
        assert_eq!(
            mask_in(chevron_forward_in(&images, &t).sm(), TextDirection::Rtl),
            mask_in(chevron_back_in(&images, &t).sm(), TextDirection::Ltr),
        );
    }

    #[test]
    fn a_physical_arrow_is_left_alone() {
        // "Collapse the left panel" means the left panel in both directions —
        // mirroring it would be a bug, so only icons that asked for it move.
        let images = Images::new();
        let t = theme();
        let plain = icon_in(&images, &t, IconName::ChevronLeft).sm();
        assert!(!plain.is_directional());
        assert_eq!(
            mask_in(plain.clone(), TextDirection::Ltr),
            mask_in(plain, TextDirection::Rtl),
        );
        assert!(chevron_back_in(&images, &t).is_directional());
    }

    #[test]
    fn flipping_the_direction_of_a_live_tree_re_rasterises() {
        // The direction can change after the tree exists (a language switch),
        // and the node caches its bitmap — so the cache has to be keyed by
        // direction too, not only by pixel size.
        let images = Images::new();
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, chevron_forward_in(&images, &t).sm());
        tree.layout(BoxConstraints::loose(BOX));
        let sebelum = quad(&mut tree).unwrap().image;

        tree.set_direction(TextDirection::Rtl);
        tree.layout(BoxConstraints::loose(BOX));
        let sesudah = quad(&mut tree).unwrap().image;
        assert_ne!(sebelum, sesudah, "the arrow is stuck at its old artwork");
    }
}
