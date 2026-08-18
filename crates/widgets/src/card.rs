//! `card()` — the surface panel every group of content sits on
//! (`KOMPONEN.md` Tier 5).
//!
//! ```
//! use silka_core::view::View;
//! use silka_widgets::{card, card_body, card_header, text};
//!
//! let panel = card([
//!     View::from(card_header("Recent invoices").subtitle("Last 30 days")),
//!     View::from(card_body([View::from(text("…"))])),
//! ])
//! .label("Recent invoices");
//! # let _ = panel;
//! ```
//!
//! # What it replaced
//!
//! The ERP dashboard in `examples/dashboard/src/kit.rs` had grown two of these
//! by hand (`card` and `padded_card`) plus a `card_header` and an
//! `action_tile`, and the gallery a third. Every one of them repeated the same
//! four lines — surface, radius, hairline, elevation — and every one of them
//! got to invent its own idea of how much padding a card has.
//!
//! Four things the hand-rolled recipe never had, and this component does:
//!
//! 1. **A vocabulary for the surface.** [`CardVariant`] says whether a panel is
//!    *raised* off the page, *drawn* on it, or *sunk* into it. The hand-rolled
//!    version only knew one, so a nested card looked exactly like the card it
//!    was nested in.
//! 2. **A name for assistive technology.** A card is a landmark: a screen reader
//!    should be able to jump between "Recent invoices" and "Quick links" instead
//!    of walking every row of both. [`Card::label`] is where that name lives,
//!    and the role is [`AccessRole::Group`] rather than an anonymous box.
//! 3. **A clickable card that is still a card.** A shortcut tile is a card *and*
//!    a control at once, which is why the dashboard had to build one from
//!    `interactive` plus a column. Here [`Card::on_press`] turns the same panel
//!    into a real button — hover/press/focus springs, a focus ring, Space and
//!    Enter, a 44pt floor — without a second component.
//! 4. **Header, body and footer that agree with each other.** The inset of a
//!    header and the inset of the rows under it were two independent numbers in
//!    the application; here they are one token, and the hairline between them is
//!    [`crate::divider`] rather than a constrained empty box.
//!
//! # Why it owns a node
//!
//! It would have been shorter to return a decorated `column`, and it would have
//! been wrong: `Builder<LayoutProps>` has no way to declare an a11y role, so the
//! panel would have been invisible to assistive technology — which is the one
//! thing a landmark must not be. [`CardBox`] exists for `role` + `label`, and
//! it draws its own surface while it is there.
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | every colour is a [`ColorToken`], the radius a [`RadiusToken`], the elevation a [`ShadowToken`], the insets spacing steps |
//! | Interactive states on a spring | only when the card **is** a control ([`Card::on_press`]): then it is an `interactive`, which springs by construction. A static panel has no states to transition |
//! | Keyboard + focus ring | a pressable card is a Tab stop with Space/Enter and a ring; a static one is deliberately not |
//! | AccessKit node | [`AccessRole::Group`] with the card's name, or [`AccessRole::Button`] when pressable |
//! | Dark mode | token-driven |
//! | Hit target ≥ 44pt | [`MIN_HIT_TARGET`] is the floor of a pressable card |
//! | Reduced motion | the press shrink is decorative and the system drops it; the colour change survives |

use silka_core::access::{AccessNode, AccessRole};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{
    BoxConstraints, CrossAlign, Decoration, LayoutCtx, MainAlign, PaintCtx, RenderNode,
};
use silka_core::view::{column, interactive, row, Builder, View, ViewNode};
use silka_paint::{Color, Corners, Insets, Point, ShadowPair, Size};
use silka_text::FontWeight;
use silka_theme::{ColorToken, RadiusToken, ShadowToken, SpaceToken, Theme};

use crate::button::MIN_HIT_TARGET;
use crate::divider::divider_in;
use crate::fonts::Fonts;
use crate::spacer::spacer;
use crate::text::text_in;

/// The inset a card's own sections use, in **spacing steps** (§2.6) — 5 × 4pt.
///
/// One number for the header, the body and the footer, because a header inset
/// by 20pt above rows inset by 16pt is the single most common way a hand-built
/// card looks subtly broken.
pub const CARD_INSET_STEPS: f32 = 5.0;

/// The vertical inset of a header or a footer, in spacing steps — 3 × 4pt.
///
/// Deliberately smaller than [`CARD_INSET_STEPS`]: a header is a band, and a
/// band as tall as it is wide reads as an empty row.
pub const CARD_BAND_STEPS: f32 = 3.0;

// ---------------------------------------------------------------------------
// Variant
// ---------------------------------------------------------------------------

/// Where a card sits relative to the page — never what colour it is.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::CardVariant;
///
/// let t = Theme::cupertino(Appearance::Dark);
///
/// // A raised card casts a shadow; a drawn one answers with a hairline
/// // instead, which is what lets the two be nested without turning into soup.
/// assert!(CardVariant::Elevated.style(&t).shadows.is_visible());
/// assert!(!CardVariant::Outlined.style(&t).shadows.is_visible());
/// assert!(CardVariant::Outlined.style(&t).border_width > 0.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CardVariant {
    /// Raised off the page: a surface with a small shadow **and** a hairline.
    ///
    /// The default, and the one the ERP dashboard was already drawing by hand.
    /// The hairline matters as much as the shadow: on a dark appearance a
    /// shadow against a dark background is nearly invisible, and without the
    /// border the card would simply disappear.
    #[default]
    Elevated,
    /// Drawn on the page: a hairline and no shadow.
    ///
    /// The right answer for a card **inside** a card, where a second shadow
    /// only adds mud.
    Outlined,
    /// Sunk into the page: a quieter fill, no border, no shadow.
    ///
    /// For a well that holds something else — a code block, an empty state.
    Filled,
    /// No surface at all: spacing and grouping only.
    ///
    /// Still a real card as far as a screen reader is concerned, which is the
    /// point: a section can be a landmark without being a box.
    Ghost,
}

impl CardVariant {
    /// Every variant — for the gallery and the token sweep tests.
    pub const ALL: [CardVariant; 4] = [
        CardVariant::Elevated,
        CardVariant::Outlined,
        CardVariant::Filled,
        CardVariant::Ghost,
    ];

    /// A short name for dumps and gallery captions.
    pub const fn name(self) -> &'static str {
        match self {
            CardVariant::Elevated => "elevated",
            CardVariant::Outlined => "outlined",
            CardVariant::Filled => "filled",
            CardVariant::Ghost => "ghost",
        }
    }

    /// The surface this variant resolves to in `theme`.
    pub fn style(self, theme: &Theme) -> CardSurface {
        let hairline = theme.space_of(SpaceToken::Px);
        match self {
            CardVariant::Elevated => CardSurface {
                background: theme.color_of(ColorToken::Surface),
                border_width: hairline,
                border_color: theme.color_of(ColorToken::Separator),
                corners: theme.corners_of(RadiusToken::Lg),
                shadows: theme.shadow_of(ShadowToken::Sm),
            },
            CardVariant::Outlined => CardSurface {
                background: theme.color_of(ColorToken::Surface),
                border_width: hairline,
                border_color: theme.color_of(ColorToken::Border),
                corners: theme.corners_of(RadiusToken::Lg),
                shadows: ShadowPair::NONE,
            },
            CardVariant::Filled => CardSurface {
                background: theme.color_of(ColorToken::SurfaceSunken),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
                corners: theme.corners_of(RadiusToken::Lg),
                shadows: ShadowPair::NONE,
            },
            CardVariant::Ghost => CardSurface {
                background: Color::TRANSPARENT,
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
                corners: theme.corners_of(RadiusToken::Lg),
                shadows: ShadowPair::NONE,
            },
        }
    }
}

/// The surface one card draws, already resolved from tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardSurface {
    /// The panel fill.
    pub background: Color,
    /// The hairline width (0 for the variants that have none).
    pub border_width: f32,
    /// The hairline colour.
    pub border_color: Color,
    /// The corner geometry — squircle or arc, decided by the preset (§2.7).
    pub corners: Corners,
    /// The paired elevation shadows.
    pub shadows: ShadowPair,
}

impl CardSurface {
    /// The same values as a [`Decoration`], which is what actually paints them.
    pub fn decoration(self) -> Decoration {
        Decoration {
            background: self.background,
            corners: self.corners,
            border_width: self.border_width,
            border_color: self.border_color,
            shadows: self.shadows,
        }
    }
}

/// Every drawing and layout value of a card, already resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardStyle {
    /// The surface itself.
    pub surface: CardSurface,
    /// The inset between the panel edge and its contents.
    pub padding: Insets,
    /// The gap between the card's own sections.
    pub gap: f32,
    /// The floor on the panel's height (only a pressable card has one).
    pub min_height: f32,
}

impl CardStyle {
    /// The default style of `variant` in `theme`.
    ///
    /// The padding is **zero**: the sections ([`card_header`], [`card_body`],
    /// [`card_footer`]) carry their own inset, because a header that has to
    /// span the full width — for its own background or its own hairline —
    /// cannot live inside the panel's padding.
    pub fn from_theme(theme: &Theme, variant: CardVariant) -> Self {
        Self {
            surface: variant.style(theme),
            padding: Insets::ZERO,
            gap: 0.0,
            min_height: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// The panel leaf: one child, one surface, one a11y landmark.
///
/// It exists for the last of those three. Painting a rounded rectangle is
/// something `div().bg(…).rounded_lg()` already does; declaring "this box is a
/// group called *Recent invoices*" is not.
pub struct CardBox {
    /// Every resolved drawing value.
    pub style: CardStyle,
    /// The name a screen reader announces for the landmark.
    pub label: Option<String>,
    /// The role announced (default [`AccessRole::Group`]).
    pub role: AccessRole,
}

impl RenderNode for CardBox {
    fn type_name(&self) -> &'static str {
        "Card"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let p = self.style.padding;
        if ctx.child_count() == 0 {
            return constraints.constrain(Size::new(
                p.horizontal(),
                p.vertical().max(self.style.min_height),
            ));
        }
        let child = ctx.child(0);
        let inner = BoxConstraints::new(
            (constraints.min_width - p.horizontal()).max(0.0),
            (constraints.max_width - p.horizontal()).max(0.0),
            0.0,
            (constraints.max_height - p.vertical()).max(0.0),
        )
        .normalized();
        let isi = ctx.layout_child(child, inner);
        ctx.place_child(child, Point::new(p.left, p.top));
        constraints.constrain(Size::new(
            isi.width + p.horizontal(),
            (isi.height + p.vertical()).max(self.style.min_height),
        ))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let mut d = self.style.surface.decoration();
        // Clamping keeps the corner honest on a card that ended up shorter than
        // twice its radius — a KPI tile squeezed into a narrow column.
        d.corners = d.corners.clamp_to(ctx.size());
        ctx.decorate(&d);
        ctx.paint_children();
    }

    /// A landmark, not a box: [`AccessRole::Group`] carrying the card's name so
    /// a screen reader can jump *between* cards instead of through them.
    fn access(&self, node: &mut AccessNode) {
        // Without a name there is nothing to announce, and an unnamed group is
        // one more level of nesting for no information at all (the same trap
        // `AccessNode::selected` documents).
        if self.label.is_some() {
            node.role = self.role;
            node.label.clone_from(&self.label);
        } else {
            node.role = AccessRole::Container;
        }
    }

    /// The touch shape follows the drawn shape, so a squircle's corners are not
    /// clickable dead ground (§3.6).
    fn hit_shape(&self) -> silka_core::input::HitShape {
        silka_core::input::HitShape::Rounded(self.style.surface.corners)
    }
}

impl core::fmt::Debug for CardBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CardBox")
            .field("label", &self.label)
            .field("role", &self.role)
            .finish()
    }
}

/// The props of [`CardBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct CardProps {
    style: CardStyle,
    label: Option<String>,
    role: AccessRole,
}

impl ViewNode for CardProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(CardBox {
            style: self.style,
            label: self.label.clone(),
            role: self.role,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<CardBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.style.padding != self.style.padding || n.style.min_height != self.style.min_height {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if n.style != self.style {
            dirty |= Dirty::PAINT;
        }
        n.style = self.style;
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.role != self.role {
            n.role = self.role;
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// A surface panel holding `children`, stacked in a column.
///
/// Use [`card_in`] outside a build pass.
///
/// ```
/// use silka_core::view::View;
/// use silka_widgets::{card, text};
///
/// let panel = card([View::from(text("Total"))]).label("Total revenue");
/// # let _ = panel;
/// ```
pub fn card<C: Into<View>>(children: impl IntoIterator<Item = C>) -> Card {
    card_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        children,
    )
}

/// [`card`] with the text engine and the theme passed explicitly.
///
/// ```
/// use silka_core::view::View;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{card_in, text_in, CardVariant, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let nested = card_in(&fonts, &theme, [View::from(text_in(&fonts, "…"))]).outlined();
/// assert_eq!(nested.variant_value(), CardVariant::Outlined);
/// // A card inside a card drops the shadow rather than doubling it.
/// assert!(!nested.style().surface.shadows.is_visible());
/// ```
pub fn card_in<C: Into<View>>(
    fonts: &Fonts,
    theme: &Theme,
    children: impl IntoIterator<Item = C>,
) -> Card {
    Card {
        fonts: fonts.clone(),
        theme: *theme,
        key: None,
        children: children.into_iter().map(Into::into).collect(),
        variant: CardVariant::default(),
        padding: None,
        gap: None,
        label: None,
        role: AccessRole::Group,
        on_press: None,
        disabled: false,
        style: None,
    }
}

/// A card whose contents are inset by the standard card padding and spaced
/// apart — the "one block of content" case, with no header band.
///
/// ```
/// use silka_core::view::View;
/// use silka_widgets::{card_padded, text};
///
/// let tile = card_padded([View::from(text("Rp 128.400.000"))]);
/// # let _ = tile;
/// ```
pub fn card_padded<C: Into<View>>(children: impl IntoIterator<Item = C>) -> Card {
    card_padded_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        children,
    )
}

/// [`card_padded`] with the text engine and the theme passed explicitly.
pub fn card_padded_in<C: Into<View>>(
    fonts: &Fonts,
    theme: &Theme,
    children: impl IntoIterator<Item = C>,
) -> Card {
    card_in(fonts, theme, children)
        .padding(SpaceToken::S5)
        .gap(SpaceToken::S3)
}

/// The card builder — Dart-style (§2.5).
pub struct Card {
    /// Held, not read: a card draws no text of its own — its children arrive
    /// already built. The engine is still taken by `card_in` so that every
    /// `_in` constructor has one shape, and kept here so that a card which
    /// later grows a built-in header is not a breaking change.
    #[allow(dead_code)]
    fonts: Fonts,
    theme: Theme,
    key: Option<Key>,
    children: Vec<View>,
    variant: CardVariant,
    padding: Option<Insets>,
    gap: Option<f32>,
    label: Option<String>,
    role: AccessRole,
    on_press: Option<silka_core::Callback>,
    disabled: bool,
    style: Option<CardStyle>,
}

impl Card {
    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Where the card sits relative to the page.
    pub fn variant(mut self, variant: CardVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Raised off the page — the default.
    pub fn elevated(self) -> Self {
        self.variant(CardVariant::Elevated)
    }

    /// Drawn on the page: a hairline, no shadow.
    pub fn outlined(self) -> Self {
        self.variant(CardVariant::Outlined)
    }

    /// Sunk into the page: a quieter fill.
    pub fn filled(self) -> Self {
        self.variant(CardVariant::Filled)
    }

    /// No surface at all — grouping only.
    pub fn ghost(self) -> Self {
        self.variant(CardVariant::Ghost)
    }

    /// Inset the contents by one spacing token on all four sides.
    pub fn padding(mut self, token: SpaceToken) -> Self {
        self.padding = Some(Insets::all(self.theme.space_of(token)));
        self
    }

    /// Inset the contents by an explicit rectangle of space.
    pub fn padding_raw(mut self, insets: Insets) -> Self {
        self.padding = Some(insets);
        self
    }

    /// The gap between the card's sections.
    pub fn gap(mut self, token: SpaceToken) -> Self {
        self.gap = Some(self.theme.space_of(token));
        self
    }

    /// The name a screen reader announces for the landmark.
    ///
    /// Without it the panel is structural, and its rows are announced as if
    /// they belonged to the page — which is right for a decorative box and
    /// wrong for a section.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The role announced (default [`AccessRole::Group`]).
    pub fn role(mut self, role: AccessRole) -> Self {
        self.role = role;
        self
    }

    /// Make the whole panel a control.
    ///
    /// This is what a "shortcut tile" is, and building it out of a button would
    /// not work: [`crate::button`] takes one string and uses it as both the
    /// visible label and the accessible name, whereas a tile has a title, a
    /// detail line, and sometimes an icon. What it gets instead is
    /// [`silka_core::view::interactive`] — the same hover/press/focus springs
    /// every first-party control uses, plus a floor of [`MIN_HIT_TARGET`].
    pub fn on_press(mut self, f: impl Fn() + 'static) -> Self {
        self.on_press = Some(silka_core::Callback::new(f));
        self
    }

    /// Present but unusable (only meaningful with [`Card::on_press`]).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Replace every visual value at once (§2.7).
    pub fn style_with(mut self, style: CardStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// The variant in force.
    pub fn variant_value(&self) -> CardVariant {
        self.variant
    }

    /// True when this card is a control.
    pub fn is_pressable(&self) -> bool {
        self.on_press.is_some()
    }

    /// Every resolved drawing and layout value.
    pub fn style(&self) -> CardStyle {
        if let Some(style) = self.style {
            return style;
        }
        let mut style = CardStyle::from_theme(&self.theme, self.variant);
        if let Some(p) = self.padding {
            style.padding = p;
        }
        if let Some(g) = self.gap {
            style.gap = g;
        }
        if self.on_press.is_some() {
            style.min_height = MIN_HIT_TARGET;
        }
        style
    }
}

impl From<Card> for View {
    fn from(c: Card) -> View {
        let style = c.style();
        let isi = column(c.children)
            // Stretch, not Start: a card's sections span its width, and a
            // header that shrink-wrapped its title would leave its trailing
            // action floating in the middle of the panel.
            .cross(CrossAlign::Stretch)
            .spacing(style.gap);

        // A pressable card is an `interactive` **around** the panel rather than
        // inside it: the ring has to trace the card's own outline, and the
        // press shrink has to take the whole surface with it.
        if let Some(on_press) = c.on_press.clone() {
            let t = &c.theme;
            let mut builder = interactive(
                Builder::new(CardProps {
                    // The surface is drawn by the interactive wrapper (that is
                    // what makes it spring), so the panel underneath keeps only
                    // its geometry and its name.
                    style: CardStyle {
                        surface: CardSurface {
                            background: Color::TRANSPARENT,
                            border_width: 0.0,
                            border_color: Color::TRANSPARENT,
                            ..style.surface
                        },
                        ..style
                    },
                    label: None,
                    role: c.role,
                })
                .child(isi),
            )
            .role(AccessRole::Button)
            .disabled(c.disabled)
            .corners(style.surface.corners)
            .background(style.surface.background)
            .border(style.surface.border_width, style.surface.border_color)
            .shadow(style.surface.shadows)
            .hover(|s| s.bg(ColorToken::SurfaceHover))
            .pressed(|s| s.bg(ColorToken::SurfacePressed).scale(0.99))
            .disabled_style(|s| s.bg(ColorToken::SurfaceSunken))
            .focus_ring(t.space(0.5), t.color_of(ColorToken::FocusRing))
            .focusable(!c.disabled)
            .cursor(silka_core::input::CursorIcon::Pointer)
            .on_press(move || on_press.call());
            // The name is set only when there is one: an empty accessible name
            // is worse than none, because it stops the content underneath from
            // speaking for the control.
            if let Some(label) = c.label.clone() {
                builder = builder.label(label);
            }
            if let Some(key) = c.key.clone() {
                builder = builder.key(key);
            }
            return builder.into();
        }

        let mut builder = Builder::new(CardProps {
            style,
            label: c.label.clone(),
            role: c.role,
        })
        .child(isi);
        if let Some(key) = c.key.clone() {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for Card {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Card")
            .field("variant", &self.variant.name())
            .field("label", &self.label)
            .field("pressable", &self.on_press.is_some())
            .field("children", &self.children.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

/// A card's header band: a title (and optional subtitle) on the reading-start
/// side, something else on the other, and a hairline underneath.
///
/// ```
/// use silka_widgets::{button, card_header};
///
/// let head = card_header("Recent invoices")
///     .subtitle("Last 30 days")
///     .trailing(button("View all"));
/// # let _ = head;
/// ```
pub fn card_header(title: impl Into<String>) -> CardHeader {
    card_header_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        title,
    )
}

/// [`card_header`] with the text engine and the theme passed explicitly.
pub fn card_header_in(fonts: &Fonts, theme: &Theme, title: impl Into<String>) -> CardHeader {
    CardHeader {
        fonts: fonts.clone(),
        theme: *theme,
        key: None,
        title: title.into(),
        subtitle: None,
        leading: None,
        trailing: None,
        divider: true,
        inset: None,
    }
}

/// The header builder — Dart-style (§2.5).
pub struct CardHeader {
    fonts: Fonts,
    theme: Theme,
    key: Option<Key>,
    title: String,
    subtitle: Option<String>,
    leading: Option<View>,
    trailing: Option<View>,
    divider: bool,
    inset: Option<Insets>,
}

impl CardHeader {
    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// A quieter second line under the title.
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Something on the reading-start side of the title — an icon, an avatar.
    pub fn leading(mut self, leading: impl Into<View>) -> Self {
        self.leading = Some(leading.into());
        self
    }

    /// Something on the far side — a "View all" link, an overflow button.
    pub fn trailing(mut self, trailing: impl Into<View>) -> Self {
        self.trailing = Some(trailing.into());
        self
    }

    /// Draw the hairline under the band (on by default).
    ///
    /// Turn it off for a card whose body starts with its own separator, or the
    /// two will sit two points apart and read as a printing error.
    pub fn divider(mut self, divider: bool) -> Self {
        self.divider = divider;
        self
    }

    /// Replace the band's inset.
    pub fn inset(mut self, insets: Insets) -> Self {
        self.inset = Some(insets);
        self
    }

    /// The inset this band will use.
    pub fn resolved_inset(&self) -> Insets {
        self.inset.unwrap_or_else(|| {
            Insets::symmetric(
                self.theme.space(CARD_INSET_STEPS),
                self.theme.space(CARD_BAND_STEPS),
            )
        })
    }
}

impl From<CardHeader> for View {
    fn from(h: CardHeader) -> View {
        let t = &h.theme;
        let inset = h.resolved_inset();

        let mut judul = vec![View::from(
            text_in(&h.fonts, h.title.clone())
                .type_style(t.typography.headline)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color_of(ColorToken::Label))
                .single_line(),
        )];
        if let Some(sub) = h.subtitle.clone() {
            judul.push(View::from(
                text_in(&h.fonts, sub)
                    .type_style(t.typography.footnote)
                    .color(t.color_of(ColorToken::SecondaryLabel))
                    .single_line(),
            ));
        }

        let mut baris: Vec<View> = Vec::new();
        if let Some(leading) = h.leading {
            baris.push(leading);
        }
        baris.push(
            column(judul)
                .spacing(t.space(0.5))
                .cross(CrossAlign::Start)
                .into(),
        );
        // The gap belongs to the layout engine, not to a hand-computed number.
        baris.push(View::from(spacer()));
        if let Some(trailing) = h.trailing {
            baris.push(trailing);
        }

        let band = row(baris)
            .spacing(t.space(3.0))
            .cross(CrossAlign::Center)
            .padding(inset);

        let mut view: View = if h.divider {
            column([View::from(band), View::from(divider_in(t))])
                .cross(CrossAlign::Stretch)
                .into()
        } else {
            band.into()
        };
        if let Some(key) = h.key {
            // A keyed header survives being reordered inside a card; without a
            // key it is matched by position, which is right for the usual
            // single-header card.
            view = column([view]).cross(CrossAlign::Stretch).key(key).into();
        }
        view
    }
}

impl core::fmt::Debug for CardHeader {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CardHeader")
            .field("title", &self.title)
            .field("subtitle", &self.subtitle)
            .field("divider", &self.divider)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Body & footer
// ---------------------------------------------------------------------------

/// A card's body: the children stacked and inset by the card's own padding.
///
/// ```
/// use silka_core::view::View;
/// use silka_widgets::{card_body, text};
///
/// let body = card_body([View::from(text("One")), View::from(text("Two"))]);
/// # let _ = body;
/// ```
pub fn card_body<C: Into<View>>(
    children: impl IntoIterator<Item = C>,
) -> Builder<silka_core::view::LayoutProps> {
    card_body_in(&crate::ambient::active_theme(), children)
}

/// [`card_body`] with the theme passed explicitly.
pub fn card_body_in<C: Into<View>>(
    theme: &Theme,
    children: impl IntoIterator<Item = C>,
) -> Builder<silka_core::view::LayoutProps> {
    column(children)
        .spacing(theme.space(3.0))
        .cross(CrossAlign::Stretch)
        .padding(Insets::all(theme.space(CARD_INSET_STEPS)))
}

/// A card's footer: actions on the reading-end side, above a hairline.
///
/// ```
/// use silka_core::view::View;
/// use silka_widgets::{button, card_footer};
///
/// let foot = card_footer([View::from(button("Save"))]);
/// # let _ = foot;
/// ```
pub fn card_footer<C: Into<View>>(children: impl IntoIterator<Item = C>) -> View {
    card_footer_in(&crate::ambient::active_theme(), children)
}

/// [`card_footer`] with the theme passed explicitly.
pub fn card_footer_in<C: Into<View>>(theme: &Theme, children: impl IntoIterator<Item = C>) -> View {
    let band = row(children)
        .spacing(theme.space(2.0))
        .cross(CrossAlign::Center)
        // `End` and not `Right`: in an RTL document the buttons belong on the
        // left, and the layout engine already knows which side that is (§9.8).
        .main(MainAlign::End)
        .padding(Insets::symmetric(
            theme.space(CARD_INSET_STEPS),
            theme.space(CARD_BAND_STEPS),
        ));
    column([View::from(divider_in(theme)), View::from(band)])
        .cross(CrossAlign::Stretch)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::tree::{RenderTree, TextDirection};
    use silka_core::view::reconcile;
    use silka_paint::{Command, Quad, Scene};
    use silka_theme::{Appearance, Preset};

    const BOX: Size = Size::new(480.0, 320.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    fn laid_out(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(BOX));
        tree
    }

    fn quads(tree: &mut RenderTree) -> Vec<Quad> {
        let mut scene = Scene::new(Color::BLACK);
        tree.paint_into(&mut scene);
        scene
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::Quad(q) => Some(q.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_named_card_is_a_landmark_a_screen_reader_can_jump_to() {
        let f = fonts();
        let tree = laid_out(
            card_in(&f, &theme(), [View::from(text_in(&f, "Rp 128.400.000"))])
                .label("Total revenue"),
        );
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Total revenue")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Group);
    }

    #[test]
    fn an_unnamed_card_adds_no_level_of_nesting_at_all() {
        // An anonymous group is one more thing to walk past and zero
        // information — the same trap `AccessNode::selected` documents.
        let f = fonts();
        let tree = laid_out(card_in(&f, &theme(), [View::from(text_in(&f, "…"))]));
        // The card's own node, not "no group anywhere": the body is a column,
        // and a flex container is a meaningful grouping in its own right
        // (`silka_core::tree::TaffyBox`). What is being asserted is that the
        // card added nothing on top of it.
        let card = tree.children(tree.root())[0];
        let a11y = tree.access_tree(None);
        let e = a11y
            .entries()
            .iter()
            .find(|e| e.id == card)
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Container, "{}", a11y.dump());
        assert!(e.node.label.is_none(), "{}", a11y.dump());
    }

    #[test]
    fn every_variant_moves_with_the_preset_and_the_appearance() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let light = Theme::new(preset, Appearance::Light);
            let dark = Theme::new(preset, Appearance::Dark);
            for variant in CardVariant::ALL {
                if variant == CardVariant::Ghost {
                    continue; // nothing to compare: it draws no surface
                }
                assert_ne!(
                    variant.style(&light).background,
                    variant.style(&dark).background,
                    "{} kept its colour in dark mode",
                    variant.name()
                );
            }
        }
    }

    #[test]
    fn a_card_inside_a_card_drops_the_shadow_rather_than_doubling_it() {
        let t = theme();
        assert!(CardVariant::Elevated.style(&t).shadows.is_visible());
        assert!(!CardVariant::Outlined.style(&t).shadows.is_visible());
        // …but keeps the hairline, without which a dark card on a dark page has
        // no edge at all.
        assert!(CardVariant::Outlined.style(&t).border_width > 0.0);
    }

    #[test]
    fn a_ghost_card_draws_nothing_and_still_groups() {
        let f = fonts();
        let mut tree = laid_out(
            card_in(&f, &theme(), [View::from(text_in(&f, "x"))])
                .ghost()
                .label("Section"),
        );
        assert!(
            quads(&mut tree).is_empty(),
            "an invisible decoration must not produce a command"
        );
        let a11y = tree.access_tree(None);
        assert!(a11y.find_label("Section").is_some());
    }

    #[test]
    fn a_pressable_card_is_a_button_with_a_hit_target_of_44() {
        use std::cell::Cell;
        use std::rc::Rc;
        let f = fonts();
        let ditekan = Rc::new(Cell::new(0u32));
        let n = ditekan.clone();
        let tree = laid_out(
            card_in(&f, &theme(), [View::from(text_in(&f, "Buat faktur"))])
                .label("Buat faktur")
                .on_press(move || n.set(n.get() + 1)),
        );
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Buat faktur")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Button);
        let id = tree.children(tree.root())[0];
        assert!(
            tree.size(id).height >= MIN_HIT_TARGET,
            "a control shorter than the HIG floor is a control nobody can tap"
        );
    }

    #[test]
    fn a_static_card_is_not_a_tab_stop() {
        let f = fonts();
        let tree =
            laid_out(card_in(&f, &theme(), [View::from(text_in(&f, "x"))]).label("Statistik"));
        let a11y = tree.access_tree(None);
        let e = a11y.find_label("Statistik").unwrap();
        assert!(
            !e.node
                .actions
                .contains(silka_core::access::AccessActions::FOCUS),
            "a panel that is not a control must not be in the Tab order"
        );
    }

    #[test]
    fn the_header_puts_its_action_on_the_far_side_and_mirrors_in_rtl() {
        let f = fonts();
        let t = theme();
        let header = || {
            card_header_in(&f, &t, "Recent invoices")
                .subtitle("Last 30 days")
                .trailing(text_in(&f, "View all"))
        };

        let mut ltr = RenderTree::new();
        reconcile(&mut ltr, View::from(header()));
        ltr.layout(BoxConstraints::tight(BOX));

        let mut rtl = RenderTree::new();
        reconcile(&mut rtl, View::from(header()));
        rtl.set_direction(TextDirection::Rtl);
        rtl.layout(BoxConstraints::tight(BOX));

        // The two documents cannot place the trailing view at the same x, or
        // the row is not mirroring at all.
        let ambil = |tree: &RenderTree| -> Vec<f32> {
            fn walk(tree: &RenderTree, id: silka_core::tree::NodeId, out: &mut Vec<f32>) {
                out.push(tree.global_offset(id).x);
                for c in tree.children(id) {
                    walk(tree, *c, out);
                }
            }
            let mut out = Vec::new();
            walk(tree, tree.root(), &mut out);
            out
        };
        assert_ne!(ambil(&ltr), ambil(&rtl));
    }

    #[test]
    fn the_header_hairline_can_be_turned_off() {
        let f = fonts();
        let t = theme();
        let mut with = laid_out(View::from(card_header_in(&f, &t, "A")));
        let mut without = laid_out(View::from(card_header_in(&f, &t, "A").divider(false)));
        assert!(quads(&mut with).len() > quads(&mut without).len());
    }

    #[test]
    fn the_padded_card_insets_and_spaces_its_contents() {
        let t = theme();
        let f = fonts();
        let c = card_padded_in(&f, &t, [View::from(text_in(&f, "x"))]);
        assert_eq!(c.style().padding, Insets::all(t.space_of(SpaceToken::S5)));
        assert_eq!(c.style().gap, t.space_of(SpaceToken::S3));
    }

    #[test]
    fn rebuilding_an_identical_card_does_nothing_at_all() {
        let t = theme();
        let f = fonts();
        let build = || card_in(&f, &t, [View::from(text_in(&f, "Paid"))]).label("Status");
        let mut tree = RenderTree::new();
        reconcile(&mut tree, build());
        tree.layout(BoxConstraints::loose(BOX));
        let again = reconcile(&mut tree, build());
        assert_eq!(again.created, 0);
        assert!(again.is_noop(), "identical props must be free");
    }
}
