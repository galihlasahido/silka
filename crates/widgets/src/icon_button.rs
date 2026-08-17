//! `icon_button()` — a button whose whole content is a symbol
//! (`KOMPONEN.md` Tier 2: "hit area ≥ 44pt walau visual kecil").
//!
//! ```
//! # use silka_widgets::{icon_button, IconName};
//! icon_button(IconName::Bell, "Notifications").on_press(|| {});
//! ```
//!
//! ## The name is not optional
//!
//! [`icon_button`] takes the accessible name as a **required argument**, beside
//! the symbol, and that is the whole reason this component exists rather than
//! being a recipe. [`crate::button()`] takes one string and uses it as both the
//! drawn label and the a11y name; an icon-only button built that way announces
//! itself to VoiceOver as "☀". Here there is nothing to draw *and* nothing to
//! announce unless the caller says what the button does, so the API asks for it
//! in the one place it cannot be skipped: the constructor.
//!
//! The symbol itself stays **decorative** ([`crate::icon()`]'s default for an
//! unnamed icon), so the name is announced exactly once (§3.8).
//!
//! ## Small symbol, large target
//!
//! The drawn symbol is 20pt; the button is [`MIN_HIT_TARGET`] on both axes.
//! Those are two different numbers on purpose, and the gap between them is the
//! HIG rule this component exists to keep: a toolbar full of 20pt glyphs is
//! unusable with a finger and fine with a mouse, and the fix is an invisible
//! target rather than a bigger picture.
//!
//! The default variant is [`ButtonVariant::Ghost`] for the same reason: an icon
//! button draws no background until it is touched, so a row of them reads as a
//! row of symbols instead of a row of boxes — and the 44pt highlight that
//! appears on hover is exactly what a macOS toolbar does.
//!
//! ## What it is made of
//!
//! Nothing new. The interaction contract, the springs, the focus ring, the
//! keyboard and the a11y node are [`crate::button::ButtonBox`] — the same node,
//! not a copy of it — and the content is [`crate::icon()`]. What this module adds
//! is the assembly and the two rules above.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! Every line is inherited from [`crate::button()`] and [`crate::icon()`], which is
//! the point: both presets through [`ButtonVariant::style`], every state on a
//! spring, Space/Enter with a growing focus ring, an [`AccessRole::Button`]
//! node carrying the name (and `toggled` when it is a toggle), dark mode
//! through tokens, a ≥ 44pt target guaranteed here, and reduced motion honoured
//! by the springs underneath.

use silka_core::access::AccessRole;
use silka_core::animation::Spring;
use silka_core::input::FocusPolicy;
use silka_core::signals::Key;
use silka_core::tree::BoxConstraints;
use silka_core::view::{center, constrained, Builder, View};
use silka_core::Callback;
use silka_paint::{Color, CornerStyle, Corners};
use silka_theme::{ColorToken, SpaceToken, Theme};

use crate::button::{ButtonProps, ButtonState, ButtonStyle, ButtonVariant, MIN_HIT_TARGET};
use crate::icon::{icon_in, Icon, IconName};
use crate::images::{active_images, Images};

/// The side of an icon button's box, in logical points.
///
/// Equal to [`MIN_HIT_TARGET`], and deliberately not a smaller number with an
/// invisible band around it: a square target is easier to aim at than a wide
/// one, and two adjacent icon buttons whose targets overlapped would be one
/// button as far as a finger is concerned.
pub const ICON_BUTTON_SIDE: f32 = MIN_HIT_TARGET;

/// Dart-style icon button builder (§2.5).
#[derive(Debug, Clone)]
pub struct IconButton {
    theme: Theme,
    icon: Icon,
    label: String,
    variant: ButtonVariant,
    state: ButtonState,
    toggled: Option<bool>,
    tint: Option<Color>,
    side: f32,
    circular: bool,
    spring: Spring,
    focus: FocusPolicy,
    on_press: Option<Callback>,
    key: Option<Key>,
}

/// A button whose content is a symbol — the `icon_button` component
/// (`KOMPONEN.md` Tier 2).
///
/// `label` is what a screen reader announces, and it is required: see the
/// module docs.
///
/// ```
/// use silka_widgets::{icon_button, IconName};
///
/// let bell = icon_button(IconName::Bell, "Notifications").on_press(|| {});
/// # let _ = bell;
/// ```
///
/// Use [`icon_button_in`] outside a build pass.
pub fn icon_button(name: IconName, label: impl Into<String>) -> IconButton {
    icon_button_in(
        &active_images(),
        &crate::ambient::active_theme(),
        name,
        label,
    )
}

/// [`icon_button`] with the bitmap atlas and the theme passed explicitly.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{icon_button_in, ButtonVariant, IconName, Images, MIN_HIT_TARGET};
///
/// let images = Images::new();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // The symbol is small, the button is not.
/// let close = icon_button_in(&images, &theme, IconName::Close, "Close");
/// assert!(close.icon_size() < MIN_HIT_TARGET);
/// assert_eq!(close.side(), MIN_HIT_TARGET);
///
/// // Ghost by default — a toolbar of icon buttons must read as symbols, not
/// // as boxes.
/// assert_eq!(close.variant_value(), ButtonVariant::Ghost);
/// assert_eq!(close.style().rest.a, 0.0);
/// ```
pub fn icon_button_in(
    images: &Images,
    theme: &Theme,
    name: IconName,
    label: impl Into<String>,
) -> IconButton {
    icon_button_with_in(theme, icon_in(images, theme, name).md(), label)
}

/// An icon button from **your own** artwork ([`crate::icon_path`]).
///
/// The icon's colour is replaced by the variant's content colour: what a
/// button's contents look like belongs to the button, not to the artwork. Use
/// [`IconButton::color`] to override it.
///
/// ```
/// use silka_widgets::{icon_button_with, icon_path};
///
/// let brand = icon_button_with(
///     icon_path("brand/mark", "M2 12 L12 2 L22 12 L12 22 Z", 24.0),
///     "Open brand menu",
/// );
/// # let _ = brand;
/// ```
///
/// Use [`icon_button_with_in`] outside a build pass.
pub fn icon_button_with(icon: Icon, label: impl Into<String>) -> IconButton {
    icon_button_with_in(&crate::ambient::active_theme(), icon, label)
}

/// [`icon_button_with`] with the theme passed explicitly.
pub fn icon_button_with_in(theme: &Theme, icon: Icon, label: impl Into<String>) -> IconButton {
    IconButton {
        theme: *theme,
        icon,
        label: label.into(),
        // A symbol with a filled background reads as a box; ghost is what makes
        // a toolbar look like a toolbar.
        variant: ButtonVariant::Ghost,
        state: ButtonState::default(),
        toggled: None,
        tint: None,
        side: ICON_BUTTON_SIDE,
        circular: false,
        // `snappy` is the macOS control feel: arrives fast, almost no bounce.
        spring: Spring::snappy(),
        focus: FocusPolicy::FOCUSABLE,
        on_press: None,
        key: None,
    }
}

impl IconButton {
    /// Visual variant — [`ButtonVariant::Ghost`] by default.
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// The symbol's side, named by a spacing token (§2.6).
    pub fn size(mut self, token: SpaceToken) -> Self {
        self.icon = self.icon.size(token);
        self
    }

    /// 16pt — a dense table row.
    pub fn sm(self) -> Self {
        self.size(SpaceToken::S4)
    }

    /// 20pt — toolbars and list rows, the default.
    pub fn md(self) -> Self {
        self.size(SpaceToken::S5)
    }

    /// 24pt — a section header.
    pub fn lg(self) -> Self {
        self.size(SpaceToken::S6)
    }

    /// The symbol's colour, named by its role.
    ///
    /// Without this the colour comes from the variant, which is what keeps a
    /// destructive icon button red without the caller repeating the token.
    pub fn color(mut self, token: ColorToken) -> Self {
        self.tint = Some(self.theme.color_of(token));
        self
    }

    /// **Escape hatch**: a symbol colour that is not a token.
    pub fn color_raw(mut self, color: Color) -> Self {
        self.tint = Some(color);
        self
    }

    /// Draw it as a circle rather than a rounded square.
    ///
    /// The hit shape follows the drawing (§3.6), so a circular button really is
    /// unclickable in the corners it does not cover — which is the honest
    /// behaviour, and the reason the shape is a property rather than a
    /// decoration.
    pub fn circular(mut self, circular: bool) -> Self {
        self.circular = circular;
        self
    }

    /// The button's side, in logical points.
    ///
    /// Never smaller than [`MIN_HIT_TARGET`]: a caller may make a button
    /// bigger, and may not make it unhittable.
    pub fn side_of(mut self, side: f32) -> Self {
        self.side = if side.is_finite() {
            side.max(MIN_HIT_TARGET)
        } else {
            MIN_HIT_TARGET
        };
        self
    }

    /// What runs when the button is activated — a click **or** Space/Enter.
    pub fn on_press(mut self, f: impl Fn() + 'static) -> Self {
        self.on_press = Some(Callback::new(f));
        self
    }

    /// Mark the button as a **toggle** and give it a state.
    ///
    /// Used by a formatting toolbar, where "bold" is not an action but a
    /// switch: without this a screen reader announces the button and never says
    /// whether it is currently on (§3.8).
    pub fn toggled(mut self, on: bool) -> Self {
        self.toggled = Some(on);
        self
    }

    /// Disable the button (still announced, as dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.state.disabled = disabled;
        self
    }

    /// The spring that drives its state transitions.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Whether it can take keyboard focus.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focus.focusable = focusable;
        self
    }

    /// Explicit tab order (takes precedence over tree order).
    pub fn tab_order(mut self, order: i32) -> Self {
        self.focus.focusable = true;
        self.focus.order = Some(order);
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    // -- readers -------------------------------------------------------------

    /// The name a screen reader announces.
    pub fn label_text(&self) -> &str {
        &self.label
    }

    /// The symbol's side, in logical points.
    pub fn icon_size(&self) -> f32 {
        self.icon.size_value()
    }

    /// The button's side, in logical points.
    pub fn side(&self) -> f32 {
        self.side
    }

    /// The variant in effect.
    pub fn variant_value(&self) -> ButtonVariant {
        self.variant
    }

    /// The symbol colour that will be used.
    pub fn tint_value(&self) -> Color {
        self.tint
            .unwrap_or_else(|| self.variant.foreground(&self.theme, self.state))
    }

    /// The corner geometry that will be used — and with it the hit shape.
    pub fn corners(&self) -> Corners {
        if self.circular {
            Corners::uniform(self.side * 0.5, CornerStyle::Arc)
        } else {
            self.variant.style(&self.theme, self.state).corners
        }
    }

    /// The paint values that will be used — for the gallery and token tests.
    pub fn style(&self) -> ButtonStyle {
        ButtonStyle {
            corners: self.corners(),
            ..self.variant.style(&self.theme, self.state)
        }
    }
}

impl From<IconButton> for View {
    fn from(b: IconButton) -> View {
        let style = b.style();
        let tint = b.tint_value();
        let side = b.side;

        // The symbol is decorative: the name is announced once, by the button
        // node. An icon that named itself as well would make VoiceOver say the
        // same thing twice.
        let symbol = b.icon.decorative().color_raw(tint);

        // A **tight** square, not a floor: an icon button is a fixed target,
        // and a floor would let a flex parent stretch it into a bar. The side
        // is at least 44pt whatever the symbol's size — the one line of the
        // Definition of Done an icon-only button is most likely to break.
        let box_ = constrained(BoxConstraints::new(side, side, side, side), center(symbol));

        let mut builder = Builder::new(ButtonProps {
            style,
            label: Some(b.label),
            role: AccessRole::Button,
            toggled: b.toggled,
            focus: b.focus,
            spring: b.spring,
            on_press: b.on_press,
        })
        .child(box_);
        if let Some(key) = b.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::input::{
        Event, InputRouter, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_core::tree::RenderTree;
    use silka_core::view::reconcile;
    use silka_paint::{Command, Point, Scene, Size};
    use silka_theme::{Appearance, Preset};
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    const BOX: Size = Size::new(200.0, 200.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn laid_out(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(BOX));
        tree
    }

    fn build(label: &str) -> IconButton {
        icon_button_in(&Images::new(), &theme(), IconName::Bell, label)
    }

    #[test]
    fn the_target_is_at_least_44pt_on_both_axes() {
        let tree = laid_out(build("Notifications"));
        let id = tree.children(tree.root())[0];
        let size = tree.size(id);
        assert!(size.width >= MIN_HIT_TARGET);
        assert!(size.height >= MIN_HIT_TARGET);
    }

    #[test]
    fn the_symbol_stays_small_inside_it() {
        let b = build("Notifications");
        assert!(
            b.icon_size() < MIN_HIT_TARGET,
            "a 44pt glyph is not an icon, it is a picture"
        );
        assert_eq!(b.side(), MIN_HIT_TARGET);
    }

    #[test]
    fn a_bigger_button_is_allowed_a_smaller_one_is_not() {
        assert_eq!(build("x").side_of(64.0).side(), 64.0);
        assert_eq!(build("x").side_of(12.0).side(), MIN_HIT_TARGET);
        assert_eq!(build("x").side_of(f32::NAN).side(), MIN_HIT_TARGET);
    }

    #[test]
    fn it_draws_the_symbol_and_not_a_glyph() {
        let mut tree = laid_out(build("Notifications"));
        let mut scene = Scene::new(Color::BLACK);
        tree.paint_into(&mut scene);
        let images = scene
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::Image(_)))
            .count();
        let glyphs = scene
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::GlyphRun(_)))
            .count();
        assert_eq!(images, 1, "the symbol is a mask from the atlas");
        assert_eq!(glyphs, 0, "an icon button has no text in it at all");
    }

    #[test]
    fn a_screen_reader_hears_the_name_once_and_never_hears_the_symbol() {
        let tree = laid_out(build("Notifications"));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Notifications")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Button);
        // The icon inside is decorative, so no second node claims a name of its
        // own — the whole button is one thing to a screen reader.
        assert!(
            !a11y.dump().contains("image"),
            "the symbol must not announce itself as well:\n{}",
            a11y.dump()
        );
    }

    #[test]
    fn a_toggle_announces_its_state_and_a_plain_button_says_nothing_about_one() {
        let on = laid_out(build("Bold").toggled(true));
        let a11y = on.access_tree(None);
        let e = a11y.find_label("Bold").expect("announced");
        assert_eq!(e.node.toggled, Some(silka_core::access::AccessToggled::On));

        let plain = laid_out(build("Bold"));
        let a11y = plain.access_tree(None);
        let e = a11y.find_label("Bold").expect("announced");
        assert_eq!(
            e.node.toggled, None,
            "\"not pressed\" after every ordinary button is noise"
        );
    }

    #[test]
    fn a_click_runs_the_action_and_a_disabled_one_does_not() {
        for (disabled, expected) in [(false, 1u32), (true, 0)] {
            let hits = Rc::new(Cell::new(0u32));
            let sink = hits.clone();
            let mut tree = laid_out(
                build("Notifications")
                    .disabled(disabled)
                    .on_press(move || sink.set(sink.get() + 1)),
            );
            let mut router = InputRouter::new();
            let at = Point::new(20.0, 20.0);
            for e in [
                PointerEvent::new(PointerPhase::Move, at, Duration::ZERO),
                PointerEvent::new(PointerPhase::Down, at, Duration::from_millis(8))
                    .button(PointerButton::Primary),
                PointerEvent::new(PointerPhase::Up, at, Duration::from_millis(40))
                    .button(PointerButton::Primary),
            ] {
                router.dispatch(&mut tree, &Event::Pointer(e));
            }
            assert_eq!(hits.get(), expected, "disabled = {disabled}");
        }
    }

    #[test]
    fn space_activates_it_like_any_other_button() {
        let hits = Rc::new(Cell::new(0u32));
        let sink = hits.clone();
        let mut tree = laid_out(build("Notifications").on_press(move || sink.set(sink.get() + 1)));
        let id = tree.children(tree.root())[0];
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(id));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Space),
                Duration::ZERO,
            )),
        );
        assert_eq!(hits.get(), 1);
    }

    #[test]
    fn ghost_is_the_default_and_it_draws_nothing_until_it_is_touched() {
        let b = build("Notifications");
        assert_eq!(b.variant_value(), ButtonVariant::Ghost);
        assert_eq!(b.style().rest.a, 0.0);
        assert_ne!(b.style().hover.a, 0.0);
    }

    #[test]
    fn the_symbol_colour_comes_from_the_variant_unless_it_is_overridden() {
        let t = theme();
        let ghost = build("x");
        assert_eq!(ghost.tint_value(), t.color.label);

        let destructive = build("x").variant(ButtonVariant::Destructive);
        assert_eq!(destructive.tint_value(), t.color.on_destructive);

        let custom = build("x").color(ColorToken::Accent);
        assert_eq!(custom.tint_value(), t.color.accent);
    }

    #[test]
    fn a_circular_button_is_a_circle_in_both_presets() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let t = Theme::new(preset, Appearance::Dark);
            let b = icon_button_in(&Images::new(), &t, IconName::Close, "Close").circular(true);
            let corners = b.corners();
            assert_eq!(corners.style, CornerStyle::Arc, "{preset:?}");
            assert_eq!(corners.radii.max(), MIN_HIT_TARGET * 0.5, "{preset:?}");
        }

        // A square one keeps the preset's own corner shape, squircle included.
        let square = icon_button_in(
            &Images::new(),
            &Theme::cupertino(Appearance::Dark),
            IconName::Close,
            "Close",
        );
        assert_eq!(square.corners().style, CornerStyle::squircle());
    }

    #[test]
    fn every_colour_moves_with_the_preset_and_the_appearance() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let light = Theme::new(preset, Appearance::Light);
            let dark = Theme::new(preset, Appearance::Dark);
            let a = icon_button_in(&Images::new(), &light, IconName::Bell, "x").tint_value();
            let b = icon_button_in(&Images::new(), &dark, IconName::Bell, "x").tint_value();
            assert_ne!(
                a, b,
                "{preset:?}: a tint that survives dark mode is a literal"
            );
        }
    }

    #[test]
    fn rebuilding_an_identical_icon_button_costs_nothing() {
        let images = Images::new();
        let t = theme();
        let make = || icon_button_in(&images, &t, IconName::Bell, "Notifications");
        let mut tree = RenderTree::new();
        reconcile(&mut tree, make());
        tree.layout(BoxConstraints::loose(BOX));
        assert!(reconcile(&mut tree, make()).is_noop());
    }

    #[test]
    fn custom_artwork_takes_the_buttons_content_colour() {
        let t = theme();
        let art = crate::icon::icon_path_in(
            &Images::new(),
            &t,
            "brand/mark",
            "M2 12 L12 2 L22 12 L12 22 Z",
            24.0,
        );
        let b = icon_button_with_in(&t, art, "Open brand menu").variant(ButtonVariant::Destructive);
        assert_eq!(b.tint_value(), t.color.on_destructive);
    }
}
