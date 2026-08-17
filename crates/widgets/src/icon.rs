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
//! written with lines and cubics only.
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
use silka_paint::{Color, ImageId, ImageQuad, Rect, Size};
use silka_theme::{ColorToken, SpaceToken, Theme};

use crate::ambient::active_theme;
use crate::images::{active_images, Images};

/// The side of the `viewBox` every built-in path is drawn in.
pub const ICON_VIEWPORT: f32 = 24.0;

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
/// // the rasteriser accepts.
/// for name in IconName::ALL {
///     assert!(!name.name().is_empty());
///     assert!(name.path().starts_with('M'));
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
    pub const fn path(self) -> &'static str {
        match self {
            IconName::Check => {
                "M9.55 18.2 L3.4 12.05 L5.35 10.1 L9.55 14.3 L18.65 5.2 L20.6 7.15 Z"
            }
            IconName::ChevronUp => "M3 16 L12 7 L21 16 L18.5 18.5 L12 12 L5.5 18.5 Z",
            IconName::ChevronDown => "M3 8 L12 17 L21 8 L18.5 5.5 L12 12 L5.5 5.5 Z",
            IconName::ChevronLeft => "M16 3 L7 12 L16 21 L18.5 18.5 L12 12 L18.5 5.5 Z",
            IconName::ChevronRight => "M8 3 L17 12 L8 21 L5.5 18.5 L12 12 L5.5 5.5 Z",
            IconName::Close => {
                "M5.6 4 L12 10.4 L18.4 4 L20 5.6 L13.6 12 L20 18.4 L18.4 20 L12 13.6 \
                 L5.6 20 L4 18.4 L10.4 12 L4 5.6 Z"
            }
            IconName::Plus => {
                "M10.6 4 H13.4 V10.6 H20 V13.4 H13.4 V20 H10.6 V13.4 H4 V10.6 H10.6 Z"
            }
            IconName::Minus => "M4 10.6 H20 V13.4 H4 Z",
            IconName::Search => {
                "M10.5 3.5 C14.366 3.5 17.5 6.634 17.5 10.5 C17.5 14.366 14.366 17.5 10.5 17.5 \
                 C6.634 17.5 3.5 14.366 3.5 10.5 C3.5 6.634 6.634 3.5 10.5 3.5 Z \
                 M10.5 5.5 C7.739 5.5 5.5 7.739 5.5 10.5 C5.5 13.261 7.739 15.5 10.5 15.5 \
                 C13.261 15.5 15.5 13.261 15.5 10.5 C15.5 7.739 13.261 5.5 10.5 5.5 Z \
                 M14.5 16.3 L16.3 14.5 L20.8 19 L19 20.8 Z"
            }
            IconName::Menu => "M4 5 H20 V7.4 H4 Z M4 10.8 H20 V13.2 H4 Z M4 16.6 H20 V19 H4 Z",
            IconName::Ellipsis => {
                "M5 10.4 C5.883 10.4 6.6 11.117 6.6 12 C6.6 12.883 5.883 13.6 5 13.6 \
                 C4.117 13.6 3.4 12.883 3.4 12 C3.4 11.117 4.117 10.4 5 10.4 Z \
                 M12 10.4 C12.883 10.4 13.6 11.117 13.6 12 C13.6 12.883 12.883 13.6 12 13.6 \
                 C11.117 13.6 10.4 12.883 10.4 12 C10.4 11.117 11.117 10.4 12 10.4 Z \
                 M19 10.4 C19.883 10.4 20.6 11.117 20.6 12 C20.6 12.883 19.883 13.6 19 13.6 \
                 C18.117 13.6 17.4 12.883 17.4 12 C17.4 11.117 18.117 10.4 19 10.4 Z"
            }
            IconName::Info => {
                "M12 3 C16.971 3 21 7.029 21 12 C21 16.971 16.971 21 12 21 \
                 C7.029 21 3 16.971 3 12 C3 7.029 7.029 3 12 3 Z \
                 M12 5 C8.134 5 5 8.134 5 12 C5 15.866 8.134 19 12 19 \
                 C15.866 19 19 15.866 19 12 C19 8.134 15.866 5 12 5 Z \
                 M10.8 6.6 H13.2 V9 H10.8 Z M10.8 10.6 H13.2 V17.4 H10.8 Z"
            }
            IconName::Warning => {
                "M12 2.6 L22.4 20.6 L1.6 20.6 Z \
                 M10.9 8.5 L10.9 14.5 L13.1 14.5 L13.1 8.5 Z \
                 M10.9 16 L10.9 18.2 L13.1 18.2 L13.1 16 Z"
            }
            IconName::Trash => {
                "M9 2.8 H15 V4.6 H20 V6.8 H4 V4.6 H9 Z M5.8 8.4 H18.2 L17.2 21.2 H6.8 Z"
            }
            IconName::Star => {
                "M12 3 L14.29 8.84 L20.56 9.22 L15.71 13.21 L17.29 19.28 L12 15.9 \
                 L6.71 19.28 L8.29 13.21 L3.44 9.22 L9.71 8.84 Z"
            }
            IconName::Heart => {
                "M12 20.5 C12 20.5 3 14.6 3 8.9 C3 5.9 5.4 3.5 8.4 3.5 \
                 C10.1 3.5 11.3 4.4 12 5.4 C12.7 4.4 13.9 3.5 15.6 3.5 \
                 C18.6 3.5 21 5.9 21 8.9 C21 14.6 12 20.5 12 20.5 Z"
            }
            IconName::User => {
                "M12 4 C14.209 4 16 5.791 16 8 C16 10.209 14.209 12 12 12 \
                 C9.791 12 8 10.209 8 8 C8 5.791 9.791 4 12 4 Z \
                 M4.5 21 C4.5 16.9 7.9 13.6 12 13.6 C16.1 13.6 19.5 16.9 19.5 21 Z"
            }
            IconName::Calendar => {
                "M7 2.6 H9.2 V5 H14.8 V2.6 H17 V5 H20.4 V21.4 H3.6 V5 H7 Z \
                 M5.8 9 L5.8 19.2 L18.2 19.2 L18.2 9 Z"
            }
            IconName::Download => {
                "M10.6 3 H13.4 V12.2 H17.4 L12 18.2 L6.6 12.2 H10.6 Z M4 19.4 H20 V21.4 H4 Z"
            }
            IconName::Upload => {
                "M12 2.8 L17.4 8.8 H13.4 V18 H10.6 V8.8 H6.6 Z M4 19.4 H20 V21.4 H4 Z"
            }
            IconName::Sun => {
                "M12 7.4 C14.54 7.4 16.6 9.46 16.6 12 C16.6 14.54 14.54 16.6 12 16.6 \
                 C9.46 16.6 7.4 14.54 7.4 12 C7.4 9.46 9.46 7.4 12 7.4 Z \
                 M10.9 1.8 H13.1 V5 H10.9 Z M10.9 19 H13.1 V22.2 H10.9 Z \
                 M1.8 10.9 H5 V13.1 H1.8 Z M19 10.9 H22.2 V13.1 H19 Z \
                 M3.6 5.2 L5.2 3.6 L7.5 5.9 L5.9 7.5 Z \
                 M20.4 5.2 L18.8 3.6 L16.5 5.9 L18.1 7.5 Z \
                 M3.6 18.8 L5.2 20.4 L7.5 18.1 L5.9 16.5 Z \
                 M20.4 18.8 L18.8 20.4 L16.5 18.1 L18.1 16.5 Z"
            }
            IconName::Moon => {
                "M16 3.4 C9.6 4.7 4.5 7.9 4.5 12 C4.5 16.1 9.6 19.3 16 20.6 \
                 C11.6 18.6 8.5 15.6 8.5 12 C8.5 8.4 11.6 5.4 16 3.4 Z"
            }
            IconName::Bell => {
                "M12 2.6 C15.6 2.6 18.2 5.2 18.2 8.8 V14 L20 17.4 H4 L5.8 14 V8.8 \
                 C5.8 5.2 8.4 2.6 12 2.6 Z \
                 M9.6 18.8 H14.4 C14.4 20.5 13.3 21.6 12 21.6 \
                 C10.7 21.6 9.6 20.5 9.6 18.8 Z"
            }
        }
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
    viewport: f32,
    size: f32,
    color: Color,
    label: Option<String>,
    images: Images,

    // -- derived (always a product of the fields above) --
    /// The atlas handle for the current pixel size, once rasterised.
    image: Option<ImageId>,
    /// The pixel size the mask above was rasterised at (`0` = never).
    rasterized_px: u32,
}

impl IconBox {
    /// Make sure a mask exists for the size **and the resolution** in force.
    ///
    /// There are exactly two reasons to rasterise again, and both are here: the
    /// point size changed, and the display scale factor changed (a coverage
    /// mask is tied to a pixel grid, §3.3).
    fn ensure_mask(&mut self) {
        let px = (self.size * self.images.scale_factor()).round().max(0.0) as u32;
        if self.image.is_some() && self.rasterized_px == px {
            return;
        }
        self.rasterized_px = px;
        self.image = self.images.icon(&self.key, &self.path, self.viewport, px);
    }

    /// The atlas handle currently in use, for tests and inspectors.
    pub fn image_id(&self) -> Option<ImageId> {
        self.image
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

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // Rasterisation happens here rather than in `paint`, which only has a
        // shared reference — and here is also where the scale factor in force
        // is known to be current.
        self.ensure_mask();
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
    viewport: f32,
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
            viewport: self.viewport,
            size: self.size,
            color: self.color,
            label: self.label.clone(),
            images: self.images.clone(),
            image: None,
            rasterized_px: 0,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<IconBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        let artwork_changed =
            n.key != self.key || n.path != self.path || n.viewport != self.viewport;
        if artwork_changed || n.size != self.size || n.images != self.images {
            n.key.clone_from(&self.key);
            n.path.clone_from(&self.path);
            n.viewport = self.viewport;
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
    build(images, theme, name.name(), name.path(), ICON_VIEWPORT)
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
    build(images, theme, key, path, viewport)
}

fn build(images: &Images, theme: &Theme, key: &str, path: &str, viewport: f32) -> Icon {
    Icon {
        props: IconProps {
            key: Rc::from(key),
            path: Rc::from(path),
            viewport: if viewport.is_finite() && viewport > 0.0 {
                viewport
            } else {
                ICON_VIEWPORT
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
    use silka_core::tree::RenderTree;
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
                    .icon(name.name(), name.path(), ICON_VIEWPORT, 24)
                    .is_some(),
                "'{}' is not a path the rasteriser accepts",
                name.name()
            );
        }
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
}
