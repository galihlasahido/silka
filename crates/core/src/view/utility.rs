//! **The utility vocabulary of §2.6** — Tailwind's spelling, HIG's numbers, and
//! a front door that only design tokens fit through.
//!
//! The binding example from REKOMENDASI §2.6:
//!
//! ```
//! use silka_core::view::{div, fixed};
//! use silka_theme::{ColorToken, RadiusToken};
//!
//! let _ = div()
//!     .flex()
//!     .items_center()
//!     .gap_3()
//!     .px_4()
//!     .rounded_lg()
//!     .bg(ColorToken::Surface)
//!     .shadow_md()
//!     .child(fixed(64.0, 20.0).label("Save"));
//! ```
//!
//! # The one rule this module exists to enforce
//!
//! §2.6 discipline #1 reads *"values are locked to design tokens"*. Before this
//! module that promise was kept by a doc-comment: [`Builder::background`] takes
//! a [`Color`], so `.background(Color::hex(0x1E90FF))` type-checked and no
//! reviewer was ever warned.
//!
//! Here the normal path takes **only** a token — [`ColorToken`],
//! [`RadiusToken`], [`ShadowToken`], [`SpaceToken`], [`FontToken`] — and a
//! literal simply does not compile:
//!
//! ```compile_fail
//! use silka_core::view::div;
//! use silka_paint::Color;
//!
//! // error[E0308]: expected `ColorToken`, found `Color`
//! let _ = div().bg(Color::hex(0x1E90FF));
//! ```
//!
//! A brand color that genuinely is not a token still has a way out, spelled so
//! that it stands out in a diff: [`Builder::bg_raw`], [`Builder::rounded_raw`],
//! [`Builder::shadow_raw`], [`Builder::p_raw`]. The low-level methods
//! ([`Builder::background`], [`Builder::corners`], [`Builder::border`],
//! [`Builder::shadow`]) keep working as the layer underneath — they are what
//! the token methods call.
//!
//! # Where the theme comes from
//!
//! Tokens are inert values; they need the active [`Theme`] to become numbers.
//! Rather than thread a `theme` argument through every call
//! (`.bg(theme.color.surface)`), the theme is **ambient for the duration of a
//! build**: the shell wraps the build in [`with_theme`], and every utility
//! resolves against it.
//!
//! ```
//! use silka_core::view::{active_theme, div, with_theme};
//! use silka_theme::{Appearance, Preset, RadiusToken, Theme};
//!
//! let cupertino = Theme::cupertino(Appearance::Light);
//! with_theme(cupertino, || {
//!     assert_eq!(active_theme().preset, Preset::Cupertino);
//!     // `rounded_lg()` is a squircle here and an arc under the Tailwind
//!     // preset — the call site never changes.
//!     let _ = div().rounded_lg();
//! });
//! ```
//!
//! Resolution happens **while the view is built**, so the values that reach
//! [`crate::tree::Decoration`] are already concrete — the paint pass and the
//! renderer stay entirely theme-free (§3.2), exactly as they are today.
//!
//! # The 4pt scale
//!
//! `p_4()` is 4 **steps**, not 4 points: 16pt on both first-party presets
//! (§2.6). Every spacing utility goes through [`SpaceToken`], so a custom brand
//! preset that sets a different unit moves the whole UI at once.

use std::cell::RefCell;

use silka_paint::{Color, Corners, Insets, ShadowPair};
use silka_text::{FontWeight, TextStyle};
use silka_theme::typography::weight;
use silka_theme::{ColorToken, FontToken, RadiusToken, ShadowToken, SpaceToken, Theme, Token};

use crate::tree::{Axis, CrossAlign, FlexWrap, FocusRing, LayoutMode, MainAlign, StateStyle};

use super::primitives::{Decorated, ItemProps, LayoutProps, PadProps};
use super::{Builder, View, ViewNode};

// ---------------------------------------------------------------------------
// The ambient theme
// ---------------------------------------------------------------------------

thread_local! {
    /// The theme utilities resolve against, for this thread.
    ///
    /// A thread-local rather than an argument for the same reason
    /// [`crate::signals::current_scope`] is one: the value is constant for the
    /// whole of one build pass, and threading it through every constructor
    /// would put `theme` in front of every second line of application code
    /// (§2.5 — the code has to *read* like Dart).
    static TEMA_AKTIF: RefCell<Theme> = RefCell::new(Theme::default());
}

/// The theme styling utilities are currently resolving against.
///
/// Outside a [`with_theme`] block this is [`Theme::default`] (Cupertino,
/// light) — utilities never panic and never resolve to nothing.
///
/// ```
/// use silka_core::view::{active_theme, with_theme};
/// use silka_theme::{Appearance, ColorToken, Preset, Theme};
///
/// // There is always an answer, even with no shell running.
/// assert_eq!(active_theme().preset, Preset::Cupertino);
///
/// // The shell wraps one whole frame; every utility inside resolves against
/// // this theme without anyone passing it down by hand.
/// let dark = Theme::tailwind(Appearance::Dark);
/// with_theme(dark, || {
///     assert_eq!(active_theme().preset, Preset::Tailwind);
///
///     // Nesting works, which is what lets the gallery show both presets side
///     // by side in one window.
///     with_theme(Theme::cupertino(Appearance::Light), || {
///         assert_eq!(active_theme().appearance, Appearance::Light);
///     });
///
///     // …and the outer theme is restored on the way out.
///     assert_eq!(active_theme().appearance, Appearance::Dark);
/// });
///
/// assert_eq!(active_theme().preset, Preset::Cupertino);
/// # let _ = ColorToken::Accent;
/// ```
pub fn active_theme() -> Theme {
    TEMA_AKTIF.with(|t| *t.borrow())
}

/// Run `f` with `theme` installed as the ambient theme.
///
/// The shell wraps **one whole frame** in it — every component rebuild happens
/// synchronously inside [`crate::app::AppRuntime::frame`], so this one line is
/// the entire integration:
///
/// ```
/// use silka_core::app::app;
/// use silka_core::view::{div, with_theme, View};
/// use silka_theme::{Appearance, ColorToken, Theme};
///
/// let mut ui = app(|_cx| View::from(div().bg(ColorToken::Accent))).sized(200.0, 100.0);
/// let theme = Theme::tailwind(Appearance::Dark);
/// with_theme(theme, || ui.frame());
/// ```
///
/// Nesting is supported (a subtree under a different preset — the gallery's
/// side-by-side comparison), and the previous theme is restored even if `f`
/// panics.
///
/// A theme change must still be paired with a rebuild: the values utilities
/// produce are resolved at build time, so a component that is not rebuilt keeps
/// the colors it was built with. Injecting the theme as a
/// `Signal<Theme>` ([`crate::app::Env`]) is what marks the readers dirty.
pub fn with_theme<R>(theme: Theme, f: impl FnOnce() -> R) -> R {
    struct Pulihkan(Theme);

    impl Drop for Pulihkan {
        fn drop(&mut self) {
            TEMA_AKTIF.with(|t| *t.borrow_mut() = self.0);
        }
    }

    let _pulih = Pulihkan(TEMA_AKTIF.with(|t| {
        let lama = *t.borrow();
        *t.borrow_mut() = theme;
        lama
    }));
    f()
}

/// Resolve one token against the ambient theme.
///
/// Private on purpose: outside this module a token should be *handed to* a
/// utility, not resolved by hand.
fn resolusi<T: Token>(token: T) -> T::Value {
    TEMA_AKTIF.with(|t| token.resolve(&t.borrow()))
}

// ---------------------------------------------------------------------------
// div / container
// ---------------------------------------------------------------------------

/// The generic styled box — the `div()` of §2.6.
///
/// It is a flex container that stacks its children downward and stretches them
/// across, i.e. what a web `div` does before anyone says `display: flex`. Call
/// [`Builder::flex`] to lay children out in a row instead.
///
/// It is the same node type as [`super::column()`]/[`super::row()`]/[`super::grid`],
/// so switching between them keeps the node and its state — only the style
/// changes.
///
/// ```
/// use silka_core::view::{div, fixed};
/// use silka_theme::ColorToken;
///
/// let _ = div()
///     .flex()
///     .items_center()
///     .justify_between()
///     .px_4()
///     .py_2()
///     .rounded_md()
///     .bg(ColorToken::Surface)
///     .child(fixed(80.0, 20.0).label("Left"))
///     .child(fixed(80.0, 20.0).label("Right"));
/// ```
pub fn div() -> Builder<LayoutProps> {
    super::column(Vec::<View>::new()).cross(CrossAlign::Stretch)
}

/// [`div`] under a name that does not come from the web.
///
/// Identical in every respect; pick whichever reads better in your codebase.
///
/// ```
/// use silka_core::view::{container, div, fixed};
/// use silka_theme::ColorToken;
///
/// // The same node, the same utilities, a name that carries no web baggage.
/// let card = container()
///     .p_4()
///     .gap_2()
///     .rounded_lg()
///     .bg(ColorToken::Surface)
///     .child(fixed(120.0, 16.0).label("Title"))
///     .child(fixed(120.0, 32.0).label("Body"));
/// # let _ = (card, div());
/// ```
pub fn container() -> Builder<LayoutProps> {
    div()
}

// ---------------------------------------------------------------------------
// Traits: which props accept which family of utilities
// ---------------------------------------------------------------------------

/// Props that carry padding — the entry point for `p_*`/`px_*`/`pt_*`.
///
/// The counterpart of [`Decorated`]: implement it once and the whole spacing
/// vocabulary appears on that view's method chain.
///
/// ```
/// use silka_core::view::{div, pad, Padded};
/// use silka_paint::Insets;
///
/// // Implementing the trait is what makes `p_*`/`px_*`/`pt_*` appear; the
/// // vocabulary itself is written once, not once per view type.
/// let padded = div().px_4().py_2();
/// let also_padded = pad(Insets::ZERO, div()).p_3();
/// # let _ = (padded, also_padded);
///
/// // A custom props type joins the vocabulary by answering one question.
/// struct MyProps {
///     insets: Insets,
/// }
///
/// impl Padded for MyProps {
///     fn padding_mut(&mut self) -> &mut Insets {
///         &mut self.insets
///     }
/// }
///
/// let mut props = MyProps { insets: Insets::ZERO };
/// *props.padding_mut() = Insets::all(16.0);
/// assert_eq!(props.insets.left, 16.0);
/// ```
pub trait Padded {
    /// The insets these props apply **inside** their own edges.
    fn padding_mut(&mut self) -> &mut Insets;
}

impl Padded for LayoutProps {
    fn padding_mut(&mut self) -> &mut Insets {
        &mut self.style.padding
    }
}

impl Padded for PadProps {
    fn padding_mut(&mut self) -> &mut Insets {
        &mut self.insets
    }
}

/// Props that carry a margin — the entry point for `m_*`/`mx_*`/`mt_*`.
///
/// Only flex/grid **items** have one ([`super::item`], [`super::expanded`],
/// [`super::flexible`]): a margin is a statement about a child's place among
/// its siblings, which is precisely what an item style is. Everything else uses
/// the parent's `gap_*` or its own `p_*`, the way Tailwind's own layout advice
/// goes.
///
/// ```
/// use silka_core::view::{expanded, fixed, item, row, Margined};
/// use silka_paint::Insets;
///
/// // Only flex/grid *items* carry one, because a margin is a statement about
/// // a child's place among its siblings.
/// let spaced = row([
///     item(fixed(80.0, 20.0)).mr_2(),
///     expanded(fixed(80.0, 20.0)).ml_2(),
/// ]);
/// # let _ = spaced;
///
/// // Everything else spaces its children with the parent's `gap_*`, which is
/// // one decision in one place instead of a margin on every child.
/// let gapped = row([fixed(80.0, 20.0), fixed(80.0, 20.0)]).gap_2();
/// # let _ = gapped;
/// # let _: fn(&mut dyn Margined) -> &mut Insets = |m| m.margin_mut();
/// ```
pub trait Margined {
    /// The insets these props keep **outside** their own edges.
    fn margin_mut(&mut self) -> &mut Insets;
}

impl Margined for ItemProps {
    fn margin_mut(&mut self) -> &mut Insets {
        &mut self.style.margin
    }
}

/// Props that carry a text style — the entry point for `text_*`/`font_*`.
///
/// The text leaf itself lives in `silka-widgets` (it needs the font stack),
/// so this trait is how that layer plugs into the typography vocabulary
/// without the vocabulary having to be written twice.
///
/// ```
/// use silka_core::view::TextStyled;
/// use silka_paint::Color;
/// use silka_text::{FontWeight, TextStyle};
///
/// // Any props type that owns a text style and a colour can join the
/// // `text_*`/`font_*` vocabulary by answering these two questions.
/// struct Caption {
///     style: TextStyle,
///     color: Color,
/// }
///
/// impl TextStyled for Caption {
///     fn text_style_mut(&mut self) -> &mut TextStyle {
///         &mut self.style
///     }
///
///     fn text_color_mut(&mut self) -> &mut Color {
///         &mut self.color
///     }
/// }
///
/// let mut caption = Caption {
///     style: TextStyle::new(),
///     color: Color::WHITE,
/// };
/// *caption.text_style_mut() = TextStyle::new().size(11.0).weight(FontWeight::MEDIUM);
/// *caption.text_color_mut() = Color::WHITE.with_alpha(0.6);
///
/// assert_eq!(caption.style.size, 11.0);
/// assert_eq!(caption.color.a, 0.6);
/// ```
pub trait TextStyled {
    /// The text style these props shape.
    fn text_style_mut(&mut self) -> &mut TextStyle;

    /// The color the glyphs are painted in.
    fn text_color_mut(&mut self) -> &mut Color;
}

// ---------------------------------------------------------------------------
// Macros — one vocabulary, several targets
// ---------------------------------------------------------------------------

/// Generate the `<side>_<step>()` shorthands for one spacing setter.
macro_rules! langkah_spasi {
    ($setter:ident, $sisi:literal, $($nama:ident => $token:ident),+ $(,)?) => {
        $(
            #[doc = concat!(
                "`", stringify!($nama), "` — ", $sisi,
                " of [`SpaceToken::", stringify!($token), "`] on the 4pt scale."
            )]
            pub fn $nama(self) -> Self {
                self.$setter(SpaceToken::$token)
            }
        )+
    };
}

/// Generate `rounded_*()` for a target that already has `rounded()`.
macro_rules! pintasan_radius {
    () => {
        /// Square corners.
        pub fn rounded_none(self) -> Self {
            self.rounded(RadiusToken::None)
        }

        /// The `sm` radius token (6pt Cupertino, 4pt Tailwind).
        pub fn rounded_sm(self) -> Self {
            self.rounded(RadiusToken::Sm)
        }

        /// The `md` radius token (10pt Cupertino, 6pt Tailwind).
        pub fn rounded_md(self) -> Self {
            self.rounded(RadiusToken::Md)
        }

        /// The `lg` radius token — a 14pt squircle under Cupertino, an 8pt arc
        /// under Tailwind (§2.7).
        pub fn rounded_lg(self) -> Self {
            self.rounded(RadiusToken::Lg)
        }

        /// The `xl` radius token (20pt Cupertino, 12pt Tailwind).
        pub fn rounded_xl(self) -> Self {
            self.rounded(RadiusToken::Xl)
        }

        /// Pill/circle: the radius is clamped to half the box by the shader.
        pub fn rounded_full(self) -> Self {
            self.rounded(RadiusToken::Full)
        }
    };
}

// ---------------------------------------------------------------------------
// Color, radius, border, shadow — everything that decorates a box
// ---------------------------------------------------------------------------

impl<V: ViewNode + Decorated> Builder<V> {
    /// The background color, named by its **role** (§2.6).
    ///
    /// ```
    /// use silka_core::view::div;
    /// use silka_theme::ColorToken;
    ///
    /// let _ = div().bg(ColorToken::Surface);
    /// ```
    pub fn bg(self, token: ColorToken) -> Self {
        self.background(resolusi(token))
    }

    /// **Escape hatch**: a background color that is not a token.
    ///
    /// Legitimate for content colors an application owns and the design system
    /// cannot know — a brand logo plate, a user-picked label color, a swatch in
    /// a color picker. It is deliberately spelled `_raw` so that it shows up in
    /// review; a UI *chrome* color that reaches for this is a missing token,
    /// not a special case (§2.6, §2.7).
    pub fn bg_raw(self, color: Color) -> Self {
        self.background(color)
    }

    /// The corner geometry of one radius token — squircle under Cupertino, arc
    /// under Tailwind, straight through to the shader and to hit-testing
    /// (§3.6).
    pub fn rounded(self, token: RadiusToken) -> Self {
        self.corners(resolusi(token))
    }

    pintasan_radius!();

    /// **Escape hatch**: corner geometry computed rather than named — e.g. half
    /// a control's height. Prefer [`Theme::corners`], which keeps the preset's
    /// corner *shape* even when the radius is your own number.
    pub fn rounded_raw(self, corners: Corners) -> Self {
        self.corners(corners)
    }

    /// The border color, named by its role (`Separator`, `Border`,
    /// `FocusRing`). Width comes from `border_1()`/`border_2()`/`border_4()`.
    pub fn border_color(self, token: ColorToken) -> Self {
        let color = resolusi(token);
        self.map(move |p| p.decoration_mut().border_color = color)
    }

    /// **Escape hatch**: a border color that is not a token. See
    /// [`Builder::bg_raw`].
    pub fn border_color_raw(self, color: Color) -> Self {
        self.map(move |p| p.decoration_mut().border_color = color)
    }

    /// No border.
    pub fn border_0(self) -> Self {
        self.border_lebar(0.0)
    }

    /// A **hairline** border: 1pt, the [`SpaceToken::Px`] token.
    ///
    /// This is the separator weight across the whole HIG — a border that stays
    /// one point regardless of the spacing scale, because it is about edge
    /// crispness rather than layout rhythm.
    pub fn border_1(self) -> Self {
        self.border_lebar(resolusi(SpaceToken::Px))
    }

    /// A 2pt border (half a step on the 4pt scale).
    pub fn border_2(self) -> Self {
        self.border_lebar(resolusi(SpaceToken::S0_5))
    }

    /// A 4pt border (one full step) — heavy emphasis, selection frames.
    pub fn border_4(self) -> Self {
        self.border_lebar(resolusi(SpaceToken::S1))
    }

    /// The width half of the border utilities.
    fn border_lebar(self, width: f32) -> Self {
        self.map(move |p| p.decoration_mut().border_width = width.max(0.0))
    }

    /// The paired ambient + key shadow of one elevation token (§3.6).
    pub fn elevation(self, token: ShadowToken) -> Self {
        self.shadow(resolusi(token))
    }

    /// Flush with the surface: no shadow.
    pub fn shadow_none(self) -> Self {
        self.elevation(ShadowToken::None)
    }

    /// Low elevation — controls, flush cards.
    pub fn shadow_sm(self) -> Self {
        self.elevation(ShadowToken::Sm)
    }

    /// Medium elevation — raised cards, popovers.
    pub fn shadow_md(self) -> Self {
        self.elevation(ShadowToken::Md)
    }

    /// High elevation — sheets, dialogs.
    pub fn shadow_lg(self) -> Self {
        self.elevation(ShadowToken::Lg)
    }

    /// Highest elevation — drag previews, floating windows.
    pub fn shadow_xl(self) -> Self {
        self.elevation(ShadowToken::Xl)
    }

    /// **Escape hatch**: a shadow recipe built by hand instead of named.
    pub fn shadow_raw(self, shadows: ShadowPair) -> Self {
        self.shadow(shadows)
    }
}

// ---------------------------------------------------------------------------
// Spacing — inside the edges
// ---------------------------------------------------------------------------

impl<V: ViewNode + Padded> Builder<V> {
    /// Padding on all four sides.
    pub fn p(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.map(move |props| *props.padding_mut() = Insets::all(v))
    }

    /// Padding on the left and right.
    pub fn px(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.map(move |props| {
            let i = props.padding_mut();
            i.left = v;
            i.right = v;
        })
    }

    /// Padding on the top and bottom.
    pub fn py(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.map(move |props| {
            let i = props.padding_mut();
            i.top = v;
            i.bottom = v;
        })
    }

    /// Padding at the top.
    pub fn pt(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.map(move |props| props.padding_mut().top = v)
    }

    /// Padding on the right edge.
    ///
    /// Physical, not "end": mirroring for RTL happens in layout (§9.8), so a
    /// value written here is the value that is drawn.
    pub fn pr(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.map(move |props| props.padding_mut().right = v)
    }

    /// Padding at the bottom.
    pub fn pb(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.map(move |props| props.padding_mut().bottom = v)
    }

    /// Padding on the left edge.
    pub fn pl(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.map(move |props| props.padding_mut().left = v)
    }

    /// **Escape hatch**: padding that is not on the scale — asymmetric optical
    /// corrections, a value measured from a glyph. See [`Builder::bg_raw`].
    pub fn p_raw(self, insets: Insets) -> Self {
        self.map(move |props| *props.padding_mut() = insets)
    }

    langkah_spasi!(p, "padding on all sides",
        p_0 => None, p_px => Px, p_1 => S1, p_2 => S2, p_3 => S3, p_4 => S4, p_5 => S5,
        p_6 => S6, p_8 => S8, p_10 => S10, p_12 => S12, p_16 => S16, p_20 => S20, p_24 => S24);

    langkah_spasi!(px, "horizontal padding",
        px_0 => None, px_px => Px, px_1 => S1, px_2 => S2, px_3 => S3, px_4 => S4, px_5 => S5,
        px_6 => S6, px_8 => S8, px_10 => S10, px_12 => S12, px_16 => S16, px_20 => S20,
        px_24 => S24);

    langkah_spasi!(py, "vertical padding",
        py_0 => None, py_px => Px, py_1 => S1, py_2 => S2, py_3 => S3, py_4 => S4, py_5 => S5,
        py_6 => S6, py_8 => S8, py_10 => S10, py_12 => S12, py_16 => S16, py_20 => S20,
        py_24 => S24);

    langkah_spasi!(pt, "top padding",
        pt_0 => None, pt_px => Px, pt_1 => S1, pt_2 => S2, pt_3 => S3, pt_4 => S4, pt_5 => S5,
        pt_6 => S6, pt_8 => S8, pt_10 => S10, pt_12 => S12, pt_16 => S16);

    langkah_spasi!(pr, "right padding",
        pr_0 => None, pr_px => Px, pr_1 => S1, pr_2 => S2, pr_3 => S3, pr_4 => S4, pr_5 => S5,
        pr_6 => S6, pr_8 => S8, pr_10 => S10, pr_12 => S12, pr_16 => S16);

    langkah_spasi!(pb, "bottom padding",
        pb_0 => None, pb_px => Px, pb_1 => S1, pb_2 => S2, pb_3 => S3, pb_4 => S4, pb_5 => S5,
        pb_6 => S6, pb_8 => S8, pb_10 => S10, pb_12 => S12, pb_16 => S16);

    langkah_spasi!(pl, "left padding",
        pl_0 => None, pl_px => Px, pl_1 => S1, pl_2 => S2, pl_3 => S3, pl_4 => S4, pl_5 => S5,
        pl_6 => S6, pl_8 => S8, pl_10 => S10, pl_12 => S12, pl_16 => S16);
}

// ---------------------------------------------------------------------------
// Spacing — outside the edges
// ---------------------------------------------------------------------------

impl<V: ViewNode + Margined> Builder<V> {
    /// Margin on all four sides.
    pub fn m(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.map(move |props| *props.margin_mut() = Insets::all(v))
    }

    /// Margin on the left and right.
    pub fn mx(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.map(move |props| {
            let i = props.margin_mut();
            i.left = v;
            i.right = v;
        })
    }

    /// Margin on the top and bottom.
    pub fn my(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.map(move |props| {
            let i = props.margin_mut();
            i.top = v;
            i.bottom = v;
        })
    }

    /// Margin above.
    pub fn mt(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.map(move |props| props.margin_mut().top = v)
    }

    /// Margin to the right.
    pub fn mr(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.map(move |props| props.margin_mut().right = v)
    }

    /// Margin below.
    pub fn mb(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.map(move |props| props.margin_mut().bottom = v)
    }

    /// Margin to the left.
    pub fn ml(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.map(move |props| props.margin_mut().left = v)
    }

    /// **Escape hatch**: a margin that is not on the scale.
    pub fn m_raw(self, insets: Insets) -> Self {
        self.map(move |props| *props.margin_mut() = insets)
    }

    langkah_spasi!(m, "margin on all sides",
        m_0 => None, m_1 => S1, m_2 => S2, m_3 => S3, m_4 => S4, m_6 => S6, m_8 => S8,
        m_12 => S12);

    langkah_spasi!(mx, "horizontal margin",
        mx_0 => None, mx_1 => S1, mx_2 => S2, mx_3 => S3, mx_4 => S4, mx_6 => S6, mx_8 => S8);

    langkah_spasi!(my, "vertical margin",
        my_0 => None, my_1 => S1, my_2 => S2, my_3 => S3, my_4 => S4, my_6 => S6, my_8 => S8);

    langkah_spasi!(mt, "top margin",
        mt_0 => None, mt_1 => S1, mt_2 => S2, mt_3 => S3, mt_4 => S4, mt_6 => S6, mt_8 => S8);

    langkah_spasi!(mr, "right margin",
        mr_0 => None, mr_1 => S1, mr_2 => S2, mr_3 => S3, mr_4 => S4, mr_6 => S6, mr_8 => S8);

    langkah_spasi!(mb, "bottom margin",
        mb_0 => None, mb_1 => S1, mb_2 => S2, mb_3 => S3, mb_4 => S4, mb_6 => S6, mb_8 => S8);

    langkah_spasi!(ml, "left margin",
        ml_0 => None, ml_1 => S1, ml_2 => S2, ml_3 => S3, ml_4 => S4, ml_6 => S6, ml_8 => S8);
}

// ---------------------------------------------------------------------------
// Layout — flex direction, alignment, wrapping
// ---------------------------------------------------------------------------

impl Builder<LayoutProps> {
    /// Lay children out in a **row** — `display: flex` with the web's default
    /// direction (§2.6).
    pub fn flex(self) -> Self {
        self.sumbu(Axis::Horizontal)
    }

    /// Lay children out in a row. The explicit spelling of [`Builder::flex`].
    pub fn flex_row(self) -> Self {
        self.sumbu(Axis::Horizontal)
    }

    /// Stack children downward — a [`div`]'s default.
    pub fn flex_col(self) -> Self {
        self.sumbu(Axis::Vertical)
    }

    /// Switch a container to flex mode along `axis`.
    fn sumbu(self, axis: Axis) -> Self {
        self.map(move |p| {
            p.style.mode = LayoutMode::Flex;
            p.style.axis = axis;
        })
    }

    /// Children packed at the start of the cross axis.
    pub fn items_start(self) -> Self {
        self.cross(CrossAlign::Start)
    }

    /// Children centered on the cross axis.
    pub fn items_center(self) -> Self {
        self.cross(CrossAlign::Center)
    }

    /// Children packed at the end of the cross axis.
    pub fn items_end(self) -> Self {
        self.cross(CrossAlign::End)
    }

    /// Children stretched across the cross axis.
    pub fn items_stretch(self) -> Self {
        self.cross(CrossAlign::Stretch)
    }

    /// Children aligned on their text baselines.
    pub fn items_baseline(self) -> Self {
        self.cross(CrossAlign::Baseline)
    }

    /// Children packed at the start of the main axis.
    pub fn justify_start(self) -> Self {
        self.main(MainAlign::Start)
    }

    /// Children centered on the main axis.
    pub fn justify_center(self) -> Self {
        self.main(MainAlign::Center)
    }

    /// Children packed at the end of the main axis.
    pub fn justify_end(self) -> Self {
        self.main(MainAlign::End)
    }

    /// Leftover space split between the children; the first and last touch the
    /// edges.
    pub fn justify_between(self) -> Self {
        self.main(MainAlign::SpaceBetween)
    }

    /// Leftover space split evenly, with half a gap at each edge.
    pub fn justify_around(self) -> Self {
        self.main(MainAlign::SpaceAround)
    }

    /// Leftover space split perfectly evenly, edges included.
    pub fn justify_evenly(self) -> Self {
        self.main(MainAlign::SpaceEvenly)
    }

    /// Keep every child on one line, overflowing if it must (the default, and
    /// Flutter's `Row`/`Column` behavior).
    pub fn nowrap(self) -> Self {
        self.wrap_mode(FlexWrap::NoWrap)
    }

    /// Wrap onto new lines with the line order reversed.
    pub fn wrap_reverse(self) -> Self {
        self.wrap_mode(FlexWrap::WrapReverse)
    }

    /// The gap between children on both axes, named by its spacing token.
    ///
    /// The general form behind `gap_1()`…`gap_12()`, for the rare case where
    /// the step is computed rather than written.
    pub fn gap_token(self, token: SpaceToken) -> Self {
        let v = resolusi(token);
        self.gap(v, v)
    }
}

// ---------------------------------------------------------------------------
// Flex items
// ---------------------------------------------------------------------------

impl Builder<ItemProps> {
    /// Grow and shrink from a zero basis — Tailwind's `flex-1`, and the same
    /// thing [`super::expanded`] does.
    pub fn flex_1(self) -> Self {
        self.grow(1.0).shrink(1.0).basis(0.0)
    }

    /// Grow and shrink from the content's natural size — Tailwind's
    /// `flex-auto`, the same thing [`super::flexible`] does.
    pub fn flex_auto(self) -> Self {
        self.grow(1.0).shrink(1.0)
    }

    /// Neither grow nor shrink: the item keeps its natural size.
    pub fn flex_none(self) -> Self {
        self.grow(0.0).shrink(0.0)
    }
}

// ---------------------------------------------------------------------------
// Typography
// ---------------------------------------------------------------------------

impl<V: ViewNode + TextStyled> Builder<V> {
    /// The full text style of one typography token: size, line height, weight,
    /// and tracking together.
    ///
    /// This is the preferred spelling — a **role**, which each preset sizes for
    /// its own density (§2.7). The `text_*()` shorthands below are aliases onto
    /// the same roles for readers who think in Tailwind.
    pub fn font(self, token: FontToken) -> Self {
        let gaya = resolusi(token);
        self.map(move |p| {
            let ts = p.text_style_mut();
            ts.size = gaya.size;
            ts.line_height = gaya.line_height;
            ts.weight = FontWeight(gaya.weight);
            ts.tracking = gaya.tracking;
        })
    }

    /// The smallest text — [`FontToken::Caption2`].
    pub fn text_xs(self) -> Self {
        self.font(FontToken::Caption2)
    }

    /// Small supporting text — [`FontToken::Footnote`].
    pub fn text_sm(self) -> Self {
        self.font(FontToken::Footnote)
    }

    /// The UI default — [`FontToken::Body`].
    pub fn text_base(self) -> Self {
        self.font(FontToken::Body)
    }

    /// A small title — [`FontToken::Title3`].
    pub fn text_lg(self) -> Self {
        self.font(FontToken::Title3)
    }

    /// A medium title — [`FontToken::Title2`].
    pub fn text_xl(self) -> Self {
        self.font(FontToken::Title2)
    }

    /// A large title — [`FontToken::Title1`].
    pub fn text_2xl(self) -> Self {
        self.font(FontToken::Title1)
    }

    /// A page title — [`FontToken::LargeTitle`].
    pub fn text_3xl(self) -> Self {
        self.font(FontToken::LargeTitle)
    }

    /// Weight 400 — body text.
    pub fn font_regular(self) -> Self {
        self.bobot(weight::REGULAR)
    }

    /// Weight 500 — control labels.
    pub fn font_medium(self) -> Self {
        self.bobot(weight::MEDIUM)
    }

    /// Weight 600 — HIG-style titles.
    pub fn font_semibold(self) -> Self {
        self.bobot(weight::SEMIBOLD)
    }

    /// Weight 700 — large titles.
    pub fn font_bold(self) -> Self {
        self.bobot(weight::BOLD)
    }

    /// Italic text (synthesized when the font has no italic master).
    pub fn italic(self) -> Self {
        self.map(move |p| p.text_style_mut().italic = true)
    }

    fn bobot(self, w: u16) -> Self {
        self.map(move |p| p.text_style_mut().weight = FontWeight(w))
    }

    /// The glyph color, named by its role (`Label`, `SecondaryLabel`,
    /// `OnAccent`, …).
    pub fn text_color(self, token: ColorToken) -> Self {
        let color = resolusi(token);
        self.map(move |p| *p.text_color_mut() = color)
    }

    /// **Escape hatch**: a glyph color that is not a token — syntax
    /// highlighting, a user-chosen label color. See [`Builder::bg_raw`].
    pub fn text_color_raw(self, color: Color) -> Self {
        self.map(move |p| *p.text_color_mut() = color)
    }
}

// ---------------------------------------------------------------------------
// Interactive
// ---------------------------------------------------------------------------

impl Builder<super::InteractiveProps> {
    /// The resting background color, named by its role.
    pub fn bg(self, token: ColorToken) -> Self {
        self.background(resolusi(token))
    }

    /// **Escape hatch**: a resting background that is not a token. See
    /// [`Builder::bg_raw`].
    pub fn bg_raw(self, color: Color) -> Self {
        self.background(color)
    }

    /// The background while the pointer is over it — the `SurfaceHover` /
    /// `AccentHover` roles.
    pub fn hover_bg(self, token: ColorToken) -> Self {
        self.hover_background(resolusi(token))
    }

    /// The background while pressed — the `SurfacePressed` / `AccentPressed`
    /// roles.
    pub fn press_bg(self, token: ColorToken) -> Self {
        self.press_background(resolusi(token))
    }

    /// The corner geometry of one radius token — **and therefore the shape of
    /// the touch area**, since hit-testing reads the same corners (§3.6).
    pub fn rounded(self, token: RadiusToken) -> Self {
        self.corners(resolusi(token))
    }

    pintasan_radius!();

    /// The paired ambient + key shadow of one elevation token.
    pub fn elevation(self, token: ShadowToken) -> Self {
        self.shadow(resolusi(token))
    }

    /// A hairline border in the color of one role.
    pub fn border_1(self, token: ColorToken) -> Self {
        self.border(resolusi(SpaceToken::Px), resolusi(token))
    }

    /// The keyboard focus ring, in the color of one role — normally
    /// [`ColorToken::FocusRing`].
    ///
    /// The width is the HIG's 2pt ([`SpaceToken::S0_5`]), the same value every
    /// first-party control uses. The ring **grows and fades in** on a spring
    /// when focus arrives; nothing here has to ask for that.
    pub fn ring(self, token: ColorToken) -> Self {
        self.focus_ring(resolusi(SpaceToken::S0_5), resolusi(token))
    }
}

// ---------------------------------------------------------------------------
// Interaction states — the closure form of §2.6
// ---------------------------------------------------------------------------

/// The same vocabulary, one state deep.
///
/// [`StateStyle`] is what `hover(|s| …)`, `pressed(|s| …)`, `focused(|s| …)` and
/// `disabled_style(|s| …)` hand to their closure. The methods here mirror the
/// resting ones — `bg`, `border_color`, `border_1` — so a state is written the
/// way the resting style is written, and every value still goes through a token:
///
/// ```
/// use silka_core::view::{fixed, interactive};
/// use silka_theme::ColorToken;
///
/// let _ = interactive(fixed(160.0, 44.0))
///     .bg(ColorToken::Surface)
///     .hover(|s| s.bg(ColorToken::SurfaceHover))
///     .pressed(|s| s.bg(ColorToken::SurfacePressed).scale(0.97));
/// ```
///
/// What is **not** here is any way to say how long the transition takes: the
/// spring is the system's, not the call site's (§3.5). That is the whole point
/// of the milestone — an application cannot write a hover that snaps, and cannot
/// write one that lags either.
impl StateStyle {
    /// The background color in this state, named by its role.
    pub fn bg(mut self, token: ColorToken) -> Self {
        self.background = Some(resolusi(token));
        self
    }

    /// **Escape hatch**: a background in this state that is not a token. See
    /// [`Builder::bg_raw`].
    pub fn bg_raw(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// The border color in this state, named by its role.
    pub fn border_color(mut self, token: ColorToken) -> Self {
        self.border_color = Some(resolusi(token));
        self
    }

    /// **Escape hatch**: a border color in this state that is not a token.
    pub fn border_color_raw(mut self, color: Color) -> Self {
        self.border_color = Some(color);
        self
    }

    /// No border in this state — the width animates down to zero rather than
    /// vanishing between two frames.
    pub fn border_0(self) -> Self {
        self.border_lebar(0.0)
    }

    /// A hairline (1pt) border in this state.
    pub fn border_1(self) -> Self {
        self.border_lebar(resolusi(SpaceToken::Px))
    }

    /// A 2pt border in this state.
    pub fn border_2(self) -> Self {
        self.border_lebar(resolusi(SpaceToken::S0_5))
    }

    /// A 4pt border in this state.
    pub fn border_4(self) -> Self {
        self.border_lebar(resolusi(SpaceToken::S1))
    }

    fn border_lebar(mut self, width: f32) -> Self {
        self.border_width = Some(width.max(0.0));
        self
    }

    /// The focus ring, in the color of one role.
    ///
    /// Only read from the `focused` state — the ring is what focus *is*, so it
    /// would mean nothing on `hover`.
    pub fn ring(mut self, token: ColorToken) -> Self {
        self.ring = Some(FocusRing::new(resolusi(SpaceToken::S0_5), resolusi(token)));
        self
    }

    /// **Escape hatch**: a focus ring whose width and color are both given by
    /// hand.
    pub fn ring_raw(mut self, width: f32, color: Color) -> Self {
        self.ring = Some(FocusRing::new(width, color));
        self
    }

    /// Scale the drawn box: `0.97` is the classic press shrink, `1.02` a hover
    /// lift.
    ///
    /// **Decorative motion** (§3.5): under reduced motion it does not happen at
    /// all — not "instantly", but never, because a box that blinks smaller
    /// within one frame is exactly the flicker the setting exists to remove.
    /// Whatever the state also changes about *colour* keeps running, so the
    /// control still answers the pointer.
    ///
    /// The box shrinks into itself, so no neighbour ever moves: a press cannot
    /// trigger a relayout.
    pub fn scale(mut self, factor: f32) -> Self {
        self.scale = Some(if factor.is_finite() { factor } else { 1.0 });
        self
    }
}
