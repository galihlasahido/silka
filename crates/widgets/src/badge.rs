//! `badge()` — the status pill (`KOMPONEN.md` Tier 4).
//!
//! ```
//! use silka_widgets::{badge, BadgeTone};
//!
//! let paid = badge("Paid").tone(BadgeTone::Success).soft();
//! let unread = silka_widgets::badge_count(12);
//! # let _ = (paid, unread);
//! ```
//!
//! # What it replaced
//!
//! Two applications in this repository had grown the same shape by hand — the
//! ERP dashboard in `examples/dashboard/src/kit.rs`, and the table page of the
//! gallery. Both wrote the identical five lines:
//!
//! ```text
//! pad(Insets::symmetric(t.space(2.0), t.space(0.5)), text_in(fonts, label)
//!         .size(t.typography.caption1.size)
//!         .weight(FontWeight::SEMIBOLD)
//!         .color(fg))
//!     .background(bg)
//!     .corners(t.corners_of(RadiusToken::Full))
//! ```
//!
//! Four things that recipe never had, and this component does:
//!
//! 1. **A floor on the size.** A pill holding `3` came out narrower than it was
//!    tall, so a row of them was a row of different shapes. Here the minimum
//!    width is the pill's own height, which makes a one-character badge a
//!    circle by construction.
//! 2. **A tone vocabulary.** The hand-rolled version took a colour pair, so
//!    every call site got to invent its own idea of what "warning" looks like —
//!    and the soft/solid distinction did not exist at all.
//! 3. **A name for assistive technology.** A screen reader met "Paid" as a
//!    stray label with no indication that it was a *status*; [`Badge::label`]
//!    is where "Status: Paid" belongs.
//! 4. **A count that cannot overflow its pill.** [`format_count`] caps at a
//!    maximum and writes `99+`, instead of letting a four-digit number stretch
//!    a notification dot across a toolbar.
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | every colour is a [`ColorToken`], the radius is [`RadiusToken::Full`], the padding and height are spacing steps |
//! | Interactive states on a spring | none exist: a badge is **not** a control. A clickable pill is [`mod@crate::button`] with [`crate::ButtonVariant::Ghost`], and conflating the two is how a status turns into a mystery button |
//! | Keyboard + focus ring | not a tab stop, by design |
//! | AccessKit node | [`AccessRole::Label`] when it carries its own name, otherwise structural so its text speaks once rather than twice |
//! | Dark mode | token-driven |
//! | Hit target ≥ 44pt | not applicable: nothing here is clickable |
//! | Reduced motion | nothing moves |

use silka_core::access::{AccessNode, AccessRole};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, Corners, Insets, Point, Quad, Rect, Size};
use silka_text::FontWeight;
use silka_theme::{ColorToken, RadiusToken, SpaceToken, Theme};

use crate::fonts::Fonts;
use crate::text::text_in;

/// Alpha of a **soft** badge's fill, as a fraction of its tone colour.
///
/// The one derivation in this file, and it is deliberate: the token set has a
/// muted companion for the accent ([`ColorToken::AccentMuted`]) and for nothing
/// else, so `success`/`warning`/`destructive` get their quiet background by
/// dropping the alpha of the tone itself. That keeps the hue identical to the
/// text on top of it, which is what makes a soft badge read as one object
/// rather than as text on a random tint.
pub const SOFT_TINT: f32 = 0.18;

/// Pill height, in **spacing steps** (§2.6) — 5 × 4pt = 20pt.
pub const BADGE_HEIGHT_STEPS: f32 = 5.0;

// ---------------------------------------------------------------------------
// Tone & variant
// ---------------------------------------------------------------------------

/// What a badge **means** — never what colour it is.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{BadgeTone, BadgeVariant};
///
/// let t = Theme::cupertino(Appearance::Dark);
///
/// // The same tone answers differently for a solid pill and a soft one, and
/// // in neither case does the caller name a colour.
/// let solid = BadgeTone::Success.colors(&t, BadgeVariant::Solid);
/// let soft = BadgeTone::Success.colors(&t, BadgeVariant::Soft);
/// assert_ne!(solid.background, soft.background);
/// assert_eq!(soft.foreground, t.color.success);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BadgeTone {
    /// No opinion: a category, a count, a tag.
    #[default]
    Neutral,
    /// The application's own accent — "new", "beta", a selected filter.
    Accent,
    /// Something completed successfully.
    Success,
    /// Something needs attention but is not broken.
    Warning,
    /// Something failed, expired, or was rejected.
    Danger,
}

impl BadgeTone {
    /// Every tone — for the gallery and for the token sweep tests.
    pub const ALL: [BadgeTone; 5] = [
        BadgeTone::Neutral,
        BadgeTone::Accent,
        BadgeTone::Success,
        BadgeTone::Warning,
        BadgeTone::Danger,
    ];

    /// A short name for dumps and gallery captions.
    pub const fn name(self) -> &'static str {
        match self {
            BadgeTone::Neutral => "neutral",
            BadgeTone::Accent => "accent",
            BadgeTone::Success => "success",
            BadgeTone::Warning => "warning",
            BadgeTone::Danger => "danger",
        }
    }

    /// The tone's own colour — the one a soft badge writes its text in.
    pub fn ink(self, theme: &Theme) -> Color {
        match self {
            BadgeTone::Neutral => theme.color_of(ColorToken::SecondaryLabel),
            BadgeTone::Accent => theme.color_of(ColorToken::Accent),
            BadgeTone::Success => theme.color_of(ColorToken::Success),
            BadgeTone::Warning => theme.color_of(ColorToken::Warning),
            BadgeTone::Danger => theme.color_of(ColorToken::Destructive),
        }
    }

    /// The three colours a badge of this tone and variant draws with.
    pub fn colors(self, theme: &Theme, variant: BadgeVariant) -> BadgeColors {
        let ink = self.ink(theme);
        match variant {
            BadgeVariant::Solid => BadgeColors {
                background: match self {
                    // A solid neutral badge is a *surface*, not ink: filling it
                    // with the secondary label colour would make a grey block
                    // as dark as the text meant to sit on it.
                    BadgeTone::Neutral => theme.color_of(ColorToken::SurfaceSunken),
                    _ => ink,
                },
                foreground: match self {
                    BadgeTone::Neutral => theme.color_of(ColorToken::Label),
                    BadgeTone::Danger => theme.color_of(ColorToken::OnDestructive),
                    _ => theme.color_of(ColorToken::OnAccent),
                },
                border: Color::TRANSPARENT,
            },
            BadgeVariant::Soft => BadgeColors {
                background: match self {
                    BadgeTone::Neutral => theme.color_of(ColorToken::SurfaceSunken),
                    // The accent is the only role with a real muted token; the
                    // rest derive theirs (see `SOFT_TINT`).
                    BadgeTone::Accent => theme.color_of(ColorToken::AccentMuted),
                    _ => ink.with_alpha(ink.a * SOFT_TINT),
                },
                foreground: ink,
                border: Color::TRANSPARENT,
            },
            BadgeVariant::Outline => BadgeColors {
                background: Color::TRANSPARENT,
                foreground: ink,
                border: match self {
                    BadgeTone::Neutral => theme.color_of(ColorToken::Border),
                    _ => ink,
                },
            },
        }
    }
}

/// How much of the tone a badge shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BadgeVariant {
    /// A filled pill — the loudest, for a status that has to be seen.
    Solid,
    /// A tinted pill with the tone as its text — the default, because a table
    /// full of solid pills is a table nobody can read.
    #[default]
    Soft,
    /// An outline only — for a tag among other tags.
    Outline,
}

impl BadgeVariant {
    /// Every variant.
    pub const ALL: [BadgeVariant; 3] = [
        BadgeVariant::Solid,
        BadgeVariant::Soft,
        BadgeVariant::Outline,
    ];

    /// A short name for dumps and gallery captions.
    pub const fn name(self) -> &'static str {
        match self {
            BadgeVariant::Solid => "solid",
            BadgeVariant::Soft => "soft",
            BadgeVariant::Outline => "outline",
        }
    }
}

/// The three colours one badge draws with, already resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BadgeColors {
    /// The pill fill.
    pub background: Color,
    /// The text (and dot) colour.
    pub foreground: Color,
    /// The outline; transparent for the filled variants.
    pub border: Color,
}

// ---------------------------------------------------------------------------
// Count formatting (pure)
// ---------------------------------------------------------------------------

/// A count as it should appear inside a pill.
///
/// A pure function, because "what does 1 234 unread look like?" has a right
/// answer that must not depend on a running app:
///
/// ```
/// use silka_widgets::badge::format_count;
///
/// assert_eq!(format_count(0, 99), "0");
/// assert_eq!(format_count(12, 99), "12");
/// assert_eq!(format_count(99, 99), "99");
/// // Past the cap the pill stops growing — a four-digit badge on a toolbar
/// // icon pushes everything else off the row.
/// assert_eq!(format_count(1_234, 99), "99+");
/// // A cap of zero means "never cap".
/// assert_eq!(format_count(1_234, 0), "1234");
/// ```
pub fn format_count(count: u64, max: u64) -> String {
    if max > 0 && count > max {
        format!("{max}+")
    } else {
        count.to_string()
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// Every drawing value of a badge, already resolved from tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BadgeStyle {
    /// The pill's colours.
    pub colors: BadgeColors,
    /// The corner geometry — [`RadiusToken::Full`] for a real pill.
    pub corners: Corners,
    /// The outline thickness (0 for the filled variants).
    pub border_width: f32,
    /// Padding around the text.
    pub padding: Insets,
    /// The pill's minimum height.
    pub height: f32,
    /// Diameter of the leading dot, or 0 for none.
    pub dot: f32,
    /// Gap between the dot and the text.
    pub dot_gap: f32,
}

/// The status-pill leaf.
///
/// It owns its own box rather than being a decorated `pad(…)` for one reason
/// that matters and one that follows from it: the **minimum width is the
/// height**, so a single character comes out as a circle instead of a squashed
/// oval; and the optional leading dot has to be laid out against that same box.
pub struct BadgeBox {
    /// Every resolved drawing value.
    pub style: BadgeStyle,
    /// The name a screen reader announces, if this badge speaks for itself.
    pub label: Option<String>,
}

impl BadgeBox {
    /// The room the leading dot takes on the main axis (0 when there is none).
    fn dot_extent(&self) -> f32 {
        if self.style.dot > 0.0 {
            self.style.dot + self.style.dot_gap
        } else {
            0.0
        }
    }

    /// The dot's rect inside a box of `size`, if there is a dot.
    ///
    /// Reading-relative: the dot leads the text, so it moves to the right-hand
    /// side of the pill in an RTL document (§9.8).
    pub fn dot_rect(&self, size: Size, rtl: bool) -> Option<Rect> {
        let d = self.style.dot;
        if d <= 0.0 {
            return None;
        }
        let x = if rtl {
            size.width - self.style.padding.right - d
        } else {
            self.style.padding.left
        };
        Some(Rect::new(x, (size.height - d) * 0.5, d, d))
    }
}

impl RenderNode for BadgeBox {
    fn type_name(&self) -> &'static str {
        "Badge"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let p = self.style.padding;
        let lead = self.dot_extent();
        if ctx.child_count() == 0 {
            return constraints.constrain(Size::new(
                (p.horizontal() + lead).max(self.style.height),
                self.style.height,
            ));
        }
        let child = ctx.child(0);
        let inner = BoxConstraints::new(
            0.0,
            (constraints.max_width - p.horizontal() - lead).max(0.0),
            0.0,
            f32::INFINITY,
        );
        let isi = ctx.layout_child(child, inner);
        // The floor is the pill's own height, which is what turns a
        // one-character badge into a circle rather than a narrow oval.
        let size = constraints.constrain(Size::new(
            (isi.width + p.horizontal() + lead).max(self.style.height),
            (isi.height + p.vertical()).max(self.style.height),
        ));
        let rtl = ctx.direction().is_rtl();
        let x = if rtl {
            (size.width - p.right - lead - isi.width).max(p.left)
        } else {
            p.left + lead
        };
        ctx.place_child(
            child,
            Point::new(x, ((size.height - isi.height) * 0.5).max(0.0)),
        );
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let c = self.style.colors;
        let ada_border = self.style.border_width > 0.0 && c.border.a > 0.0;
        if c.background.a > 0.0 || ada_border {
            ctx.quad(
                Quad::new(bounds)
                    // Clamping keeps a "full" radius a genuine half-height on
                    // any pill, whatever the preset's `full` token says.
                    .corners(self.style.corners.clamp_to(bounds.size))
                    .background(c.background)
                    .border(self.style.border_width, c.border),
            );
        }
        if let Some(dot) = self.dot_rect(bounds.size, ctx.is_rtl()) {
            if c.foreground.a > 0.0 {
                ctx.quad(
                    Quad::new(dot)
                        .background(c.foreground)
                        .corners(self.style.corners.clamp_to(dot.size)),
                );
            }
        }
        ctx.paint_children();
    }

    /// A badge with its own name is a [`AccessRole::Label`]; without one it is
    /// structural, so its text is announced exactly once.
    fn access(&self, node: &mut AccessNode) {
        match &self.label {
            Some(label) => {
                node.role = AccessRole::Label;
                node.label = Some(label.clone());
            }
            None => node.role = AccessRole::Container,
        }
    }
}

impl core::fmt::Debug for BadgeBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BadgeBox")
            .field("label", &self.label)
            .finish()
    }
}

/// The props of [`BadgeBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct BadgeProps {
    style: BadgeStyle,
    label: Option<String>,
}

impl ViewNode for BadgeProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(BadgeBox {
            style: self.style,
            label: self.label.clone(),
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<BadgeBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.style.padding != self.style.padding
            || n.style.height != self.style.height
            || n.style.dot != self.style.dot
            || n.style.dot_gap != self.style.dot_gap
        {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if n.style != self.style {
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        n.style = self.style;
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// A status pill reading `text`.
///
/// Use [`badge_in`] outside a build pass.
///
/// ```
/// use silka_widgets::{badge, BadgeTone};
///
/// let overdue = badge("Overdue").tone(BadgeTone::Danger);
/// # let _ = overdue;
/// ```
pub fn badge(text: impl Into<String>) -> Badge {
    badge_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        text,
    )
}

/// [`badge`] with the text engine and the theme passed explicitly.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{badge_in, BadgeTone, BadgeVariant, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let paid = badge_in(&fonts, &theme, "Paid")
///     .tone(BadgeTone::Success)
///     .solid();
/// assert_eq!(paid.colors().foreground, theme.color.on_accent);
/// assert_eq!(paid.variant_value(), BadgeVariant::Solid);
/// ```
pub fn badge_in(fonts: &Fonts, theme: &Theme, text: impl Into<String>) -> Badge {
    Badge {
        fonts: fonts.clone(),
        theme: *theme,
        key: None,
        text: text.into(),
        tone: BadgeTone::default(),
        variant: BadgeVariant::default(),
        dot: false,
        label: None,
    }
}

/// A count pill — the notification badge on a toolbar icon or a sidebar row.
///
/// Capped at 99 by default so the pill cannot stretch; see
/// [`Badge::max_count`] and [`format_count`].
///
/// ```
/// use silka_widgets::badge_count;
///
/// let b = badge_count(1_234);
/// assert_eq!(b.text(), "99+");
/// ```
pub fn badge_count(count: u64) -> Badge {
    badge_count_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        count,
    )
}

/// [`badge_count`] with the text engine and the theme passed explicitly.
pub fn badge_count_in(fonts: &Fonts, theme: &Theme, count: u64) -> Badge {
    badge_in(fonts, theme, format_count(count, 99))
        .tone(BadgeTone::Accent)
        .solid()
}

/// The badge builder — Dart-style (§2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct Badge {
    fonts: Fonts,
    theme: Theme,
    key: Option<Key>,
    text: String,
    tone: BadgeTone,
    variant: BadgeVariant,
    dot: bool,
    label: Option<String>,
}

impl Badge {
    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// What the badge **means**.
    pub fn tone(mut self, tone: BadgeTone) -> Self {
        self.tone = tone;
        self
    }

    /// How loudly it says it.
    pub fn variant(mut self, variant: BadgeVariant) -> Self {
        self.variant = variant;
        self
    }

    /// A filled pill.
    pub fn solid(self) -> Self {
        self.variant(BadgeVariant::Solid)
    }

    /// A tinted pill — the default.
    pub fn soft(self) -> Self {
        self.variant(BadgeVariant::Soft)
    }

    /// An outline only.
    pub fn outline(self) -> Self {
        self.variant(BadgeVariant::Outline)
    }

    /// Show a leading dot in the tone colour.
    ///
    /// The point of it is not decoration: it is the second channel a status
    /// needs so that "success" and "warning" are still distinguishable when
    /// the two hues are not (deuteranopia, a projector, a greyscale print).
    pub fn dot(mut self, dot: bool) -> Self {
        self.dot = dot;
        self
    }

    /// Re-cap an existing count badge.
    ///
    /// Only meaningful on a badge built by [`badge_count`]; it re-formats the
    /// number, so calling it with a text badge does nothing.
    pub fn max_count(mut self, count: u64, max: u64) -> Self {
        self.text = format_count(count, max);
        self
    }

    /// The name a screen reader announces.
    ///
    /// Without it, the badge's text is read as an ordinary label — which is
    /// right in a table cell whose column header already says "Status", and
    /// wrong on a toolbar icon where "12" means nothing on its own.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The text this badge will draw.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The variant in force.
    pub fn variant_value(&self) -> BadgeVariant {
        self.variant
    }

    /// The tone in force.
    pub fn tone_value(&self) -> BadgeTone {
        self.tone
    }

    /// The colours this badge resolves to.
    pub fn colors(&self) -> BadgeColors {
        self.tone.colors(&self.theme, self.variant)
    }

    /// Every resolved drawing value.
    pub fn style(&self) -> BadgeStyle {
        let t = &self.theme;
        let height = t.space(BADGE_HEIGHT_STEPS);
        BadgeStyle {
            colors: self.colors(),
            corners: t.corners_of(RadiusToken::Full),
            border_width: match self.variant {
                BadgeVariant::Outline => t.space_of(SpaceToken::Px),
                _ => 0.0,
            },
            // Horizontal padding is deliberately smaller than the vertical
            // rhythm would suggest: a one-character count has to fit inside the
            // pill's own height, or the width floor in `BadgeBox::layout` can
            // never win and a row of counts becomes a row of narrow ovals
            // instead of a row of circles.
            padding: Insets::symmetric(t.space(1.5), t.space(0.5)),
            height,
            dot: if self.dot { t.space(1.5) } else { 0.0 },
            dot_gap: t.space(1.0),
        }
    }
}

impl From<Badge> for View {
    fn from(b: Badge) -> View {
        let style = b.style();
        let t = &b.theme;
        let isi = text_in(&b.fonts, b.text.clone())
            .type_style(t.typography.caption1)
            .weight(FontWeight::SEMIBOLD)
            .color(style.colors.foreground)
            .single_line()
            // The badge speaks for itself when it was given a name; otherwise
            // this text is the one that speaks. Either way, exactly once.
            .role(if b.label.is_some() {
                AccessRole::Container
            } else {
                AccessRole::Label
            });
        let mut builder = Builder::new(BadgeProps {
            style,
            label: b.label.clone(),
        })
        .child(isi);
        if let Some(key) = b.key.clone() {
            builder = builder.key(key);
        }
        builder.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::tree::{RenderTree, TextDirection};
    use silka_core::view::reconcile;
    use silka_paint::{Command, Scene};
    use silka_theme::{Appearance, Preset};

    const BOX: Size = Size::new(320.0, 120.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
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
    fn a_single_character_badge_comes_out_a_circle() {
        let t = theme();
        let tree = laid_out(badge_in(&Fonts::bundled_only(), &t, "3"));
        let id = tree.children(tree.root())[0];
        let size = tree.size(id);
        assert_eq!(
            size.width, size.height,
            "the width floor is the pill height, or a row of counts is a row of shapes"
        );
        assert_eq!(size.height, t.space(BADGE_HEIGHT_STEPS));
    }

    #[test]
    fn a_longer_label_grows_past_the_floor() {
        let t = theme();
        let tree = laid_out(badge_in(&Fonts::bundled_only(), &t, "Awaiting approval"));
        let id = tree.children(tree.root())[0];
        assert!(tree.size(id).width > tree.size(id).height);
    }

    #[test]
    fn every_tone_and_variant_moves_with_the_preset_and_the_appearance() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let light = Theme::new(preset, Appearance::Light);
            let dark = Theme::new(preset, Appearance::Dark);
            for tone in BadgeTone::ALL {
                for variant in BadgeVariant::ALL {
                    let a = tone.colors(&light, variant);
                    let b = tone.colors(&dark, variant);
                    assert_ne!(
                        (a.background, a.foreground),
                        (b.background, b.foreground),
                        "{}/{} kept its colour in dark mode",
                        tone.name(),
                        variant.name()
                    );
                }
            }
        }
    }

    #[test]
    fn a_solid_badge_writes_on_its_fill_and_a_soft_one_in_its_tone() {
        let t = theme();
        let solid = BadgeTone::Danger.colors(&t, BadgeVariant::Solid);
        assert_eq!(solid.background, t.color.destructive);
        assert_eq!(solid.foreground, t.color.on_destructive);

        let soft = BadgeTone::Danger.colors(&t, BadgeVariant::Soft);
        assert_eq!(soft.foreground, t.color.destructive);
        assert!(
            soft.background.a < t.color.destructive.a,
            "a soft fill is the tone at a fraction of its alpha"
        );

        let outline = BadgeTone::Danger.colors(&t, BadgeVariant::Outline);
        assert_eq!(outline.background, Color::TRANSPARENT);
        assert_eq!(outline.border, t.color.destructive);
    }

    #[test]
    fn a_solid_neutral_badge_is_a_surface_rather_than_ink() {
        let t = theme();
        let c = BadgeTone::Neutral.colors(&t, BadgeVariant::Solid);
        assert_eq!(c.background, t.color.surface_sunken);
        assert_ne!(
            c.background, c.foreground,
            "filling it with the label colour would hide its own text"
        );
    }

    #[test]
    fn the_status_dot_is_a_second_channel_and_leads_the_text() {
        let t = theme();
        let mut tree = laid_out(
            badge_in(&Fonts::bundled_only(), &t, "Live")
                .tone(BadgeTone::Success)
                .dot(true),
        );
        let q = quads(&mut tree);
        assert_eq!(q.len(), 2, "the pill and its dot");
        // The dot leads the text, so it sits in the left half in LTR.
        assert!(q[1].rect.center().x < q[0].rect.center().x);
    }

    #[test]
    fn the_dot_mirrors_in_an_rtl_document() {
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            badge_in(&Fonts::bundled_only(), &t, "Live")
                .tone(BadgeTone::Success)
                .dot(true),
        );
        tree.set_direction(TextDirection::Rtl);
        tree.layout(BoxConstraints::loose(BOX));
        let q = quads(&mut tree);
        assert!(q[1].rect.center().x > q[0].rect.center().x);
    }

    #[test]
    fn a_named_badge_is_announced_once_as_a_label() {
        let tree =
            laid_out(badge_in(&Fonts::bundled_only(), &theme(), "12").label("12 unread messages"));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("12 unread messages")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Label);
        // The pill's own text must not be announced a second time.
        assert!(a11y.find_label("12").is_none());
    }

    #[test]
    fn an_unnamed_badge_lets_its_text_speak() {
        let tree = laid_out(badge_in(&Fonts::bundled_only(), &theme(), "Paid"));
        let a11y = tree.access_tree(None);
        assert!(a11y.find_label("Paid").is_some());
    }

    #[test]
    fn a_count_badge_never_stretches_past_its_cap() {
        assert_eq!(
            badge_count_in(&Fonts::bundled_only(), &theme(), 7).text(),
            "7"
        );
        assert_eq!(
            badge_count_in(&Fonts::bundled_only(), &theme(), 4_000).text(),
            "99+"
        );
        assert_eq!(
            badge_in(&Fonts::bundled_only(), &theme(), "")
                .max_count(1_500, 999)
                .text(),
            "999+"
        );
    }

    #[test]
    fn rebuilding_an_identical_badge_does_nothing_at_all() {
        let t = theme();
        let f = Fonts::bundled_only();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, badge_in(&f, &t, "Paid"));
        tree.layout(BoxConstraints::loose(BOX));
        let again = reconcile(&mut tree, badge_in(&f, &t, "Paid"));
        assert_eq!(again.created, 0);
        assert!(again.is_noop(), "identical props must be free");
    }
}
