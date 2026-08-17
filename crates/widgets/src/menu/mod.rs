//! `menu()` — the **in-app** menu: dropdown menus and context menus
//! (`KOMPONEN.md` Tier 3 `context_menu`).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::column;
//! # use silka_widgets::overlay_layer;
//! use silka_widgets::menu::{item, menu, separator, MenuState};
//!
//! # let rt = Runtime::new();
//! let state = rt.signal(MenuState::new());
//!
//! let m = menu([
//!     item("view.zoom_in", "Perbesar").into(),
//!     item("view.zoom_out", "Perkecil").into(),
//!     separator(),
//!     item("view.mode", "Tampilan")
//!         .submenu([item("view.list", "Daftar").radio(true)])
//!         .into(),
//! ])
//! .label("Tampilan")
//! .bind(state)
//! .on_activate(|id| println!("dipilih: {id}"));
//!
//! // The trigger stands in the content; every panel lives in the overlay layer.
//! let mut layer = overlay_layer(column([m.trigger("Tampilan")]));
//! for panel in m.overlays() {
//!     layer = layer.overlay(panel);
//! }
//! ```
//!
//! ## In-app menu or native menu?
//!
//! There are two menu systems in this framework and they are **not**
//! interchangeable. `INTEGRASI-NATIVE.md` §2 leaves the choice open per
//! component; this table is that choice, made:
//!
//! | | This module (`silka_widgets::menu`) | `silka_platform::menu` (planned) |
//! |---|---|---|
//! | Drawn by | us, on the GPU, inside the window | the OS |
//! | Menubar at the top of the macOS screen | ✗ | ✓ — **the only correct option** |
//! | ⌘C/⌘V through the responder chain | ✗ | ✓ (the standard Edit menu) |
//! | Tray / dock menus | ✗ | ✓ |
//! | May escape the window's bounds | ✗ (a layer inside the window) | ✓ |
//! | Spring transitions, themed rows, custom content | ✓ | ✗ |
//! | Identical on macOS/Windows/Linux | ✓ | follows each OS |
//! | Works on Wayland/X11 as a popup | ✓ | ✗ (`PopupMenu::show` has no path there) |
//!
//! The rule of thumb: **anything the operating system owns goes to
//! `silka_platform::menu`** — the menubar, the tray, the dock, and every
//! standard editing command. Everything that belongs to the application's own
//! surface — a "⋯" button on a row, a right-click on a canvas, a filter chip —
//! belongs here, where it can be themed, animated, and tested headlessly.
//!
//! ## How the pieces fit
//!
//! Like [`mod@crate::select`], a menu hands back **two kinds of piece mounted in
//! two places**: a [`Menu::trigger`] (or [`Menu::context_area`]) inside the
//! page content, and [`Menu::overlays`] inside
//! [`overlay_layer`](crate::overlay::overlay_layer). The panels must be free to
//! paint over everything and to spill past their parent's box, and the
//! infrastructure for that was built once for ten components
//! (`KOMPONEN.md` rule #3). **Not one coordinate is computed in this module**:
//! the root panel is `Placement::anchored(Side::Bottom)` and every submenu is
//! `Placement::anchored(Side::End)`, so opening upward at the bottom of the
//! screen and flipping to the other side at the right edge are behaviours the
//! overlay system already has — right once, instead of five times with five
//! bugs.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Requirement | Where |
//! |---|---|
//! | Correct in both presets | Every value flows through [`MenuTriggerStyle`]/[`MenuRowStyle`], filled from tokens; not one color literal in this module |
//! | Interactive states via springs | Trigger background, focus ring, disclosure triangle; row background; every panel's overlay transition |
//! | Full keyboard + focus ring | ↑/↓, Home/End, →/← for submenus (mirrored in RTL), Return/Space, Esc closing **one** level, and native-menu typeahead — all on the trigger, which keeps focus |
//! | AccessKit nodes | Panel = `Menu`, row = `MenuItem` (+ `toggled` for checkables, `Expand`/`Collapse` for submenu parents), separator = `Separator`, trigger = `Button` or a `Group` advertising `CONTEXT_MENU` |
//! | Dark mode | Tokens only |
//! | Hit target ≥ 44pt | `min_height` on the trigger **and** on every row |
//! | Reduced-motion | Every spring runs through [`Tick`], which carries [`Motion`](silka_core::animation::Motion) |
//!
//! ## Deliberately not here yet
//!
//! - **A hover delay ("safe triangle") before a submenu closes.** Native menus
//!   forgive a diagonal pointer path across sibling rows; we close immediately.
//!   The submenu therefore hangs with **no gap** against its parent panel,
//!   which is what keeps that path short. A delay needs a timer the widget
//!   layer does not own — only the frame [`Tick`]
//!   exists today.
//! - **Dispatching shortcuts.** The shortcut on a row is *shown*, not routed
//!   (see [`Shortcut`]): an application would otherwise have two places where
//!   ⌘S is defined.
//! - **Scrolling a menu taller than the screen.** The overlay system clamps a
//!   panel onto the screen, so a very long menu is clipped rather than
//!   scrollable; the fix is [`mod@crate::scroll_view`] inside the panel, and it
//!   waits until a real menu needs it.

mod item;
mod model;
mod state;
#[cfg(test)]
mod tests;
mod trigger;

use std::rc::Rc;

use silka_core::access::AccessRole;
use silka_core::animation::{Spring, Tick};
use silka_core::input::FocusPolicy;
use silka_core::scheduler::Dirty;
use silka_core::signals::Signal;
use silka_core::tree::{BoxConstraints, CrossAlign, NodeId, RenderTree};
use silka_core::view::{column, constrained, expanded, fixed, pad, row, Builder, View};
use silka_paint::{Insets, Point, Rect};
use silka_text::{FontWeight, TextConstraints, TextStyle};
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::overlay::{
    overlay, Align, Anchor, Barrier, Dismiss, OverlayBuilder, OverlayLayer, Placement, Side,
};
use crate::text::text_in;

pub use item::{
    triangle_columns, MenuRowBox, MenuRowProps, MenuRowStyle, MenuSeparatorBox, MenuSeparatorProps,
};
pub use model::{
    cmd, cmd_shift, first_selectable, item, last_selectable, separator, shortcut, step, typeahead,
    MenuEntry, MenuItem, MenuMark, MenuModel, Shortcut, ShortcutStyle,
};
pub use state::{MenuIntent, MenuState, SubmenuLevel};
pub use trigger::{MenuTriggerBox, MenuTriggerMode, MenuTriggerProps, MenuTriggerStyle};

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// Where a [`MenuIntent`] is sent.
///
/// Shaped exactly like [`Callback`](silka_core::Callback) — cheap `Clone`,
/// equality by identity — except that it carries one argument, which the core
/// has no equivalent for yet.
#[derive(Clone)]
pub struct MenuHandler(Rc<dyn Fn(MenuIntent)>);

impl MenuHandler {
    /// Wrap a closure.
    pub fn new(f: impl Fn(MenuIntent) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Send one intent.
    pub fn emit(&self, intent: MenuIntent) {
        (self.0)(intent)
    }
}

impl PartialEq for MenuHandler {
    /// Identity, not contents: the same `Rc` means the same handler.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for MenuHandler {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("MenuHandler")
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// What runs when the user chooses an item: the item's id, nothing more.
///
/// An id rather than an index, because indices shift the moment a menu grows a
/// line and `"file.save"` survives that.
pub type ActivateCallback = Rc<dyn Fn(&str)>;

/// Dart-style menu builder (§2.5).
///
/// It keeps the raw ingredients and only **resolves tokens** when it becomes a
/// view, so a method called late still changes the whole result. `Clone` is
/// cheap: the trigger and every panel come from the same builder, so there is
/// no way for them to drift apart.
#[derive(Clone)]
pub struct Menu {
    fonts: Fonts,
    theme: Theme,
    model: MenuModel,
    label: Option<String>,
    state: MenuState,
    disabled: bool,
    chip: bool,
    shortcut_style: ShortcutStyle,
    spring: Spring,
    focus: FocusPolicy,
    min_width: Option<f32>,
    bound: Option<Signal<MenuState>>,
    on_intent: Option<MenuHandler>,
    on_activate: Option<ActivateCallback>,
    key: String,
}

/// An in-app menu — `context_menu` (`KOMPONEN.md` Tier 3).
///
/// ```
/// use silka_widgets::menu::{item, menu, separator};
///
/// let m = menu([
///     item("view.zoom_in", "Zoom In").into(),
///     separator(),
///     item("view.mode", "View").into(),
/// ])
/// .label("View");
/// # let _ = m;
/// ```
///
/// Use [`menu_in`] outside a build pass.
pub fn menu<E: Into<MenuEntry>>(entries: impl IntoIterator<Item = E>) -> Menu {
    menu_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        entries,
    )
}

/// An in-app menu: a dropdown behind a button/chip, or a context menu behind a
/// right-click (`KOMPONEN.md` Tier 3).
///
/// `fonts` is the application's text engine, `theme` the source of every value.
pub fn menu_in<E: Into<MenuEntry>>(
    fonts: &Fonts,
    theme: &Theme,
    entries: impl IntoIterator<Item = E>,
) -> Menu {
    Menu {
        fonts: fonts.clone(),
        theme: *theme,
        model: MenuModel::new(entries),
        label: None,
        state: MenuState::new(),
        disabled: false,
        chip: false,
        shortcut_style: ShortcutStyle::PLATFORM,
        // `snappy` is how a macOS control feels: quick to arrive, almost no
        // bounce (WWDC23).
        spring: Spring::snappy(),
        focus: FocusPolicy::FOCUSABLE,
        min_width: None,
        bound: None,
        on_intent: None,
        on_activate: None,
        key: String::from("silka-menu"),
    }
}

impl Menu {
    /// The name a screen reader announces — for the trigger **and** the panel.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The state in effect, fully controlled by the application.
    pub fn state(mut self, state: MenuState) -> Self {
        self.state = state;
        self
    }

    /// Wire it to a single signal: reading it **and** writing to it.
    ///
    /// This is the shape 95% of applications use — one piece of state to keep,
    /// and every rule (the highlight skipping separators, submenus closing when
    /// the pointer moves away, the menu closing after a choice) is already
    /// right because it all goes through [`MenuState::apply`].
    pub fn bind(mut self, state: Signal<MenuState>) -> Self {
        // Read **during build**, so the component calling it subscribes:
        // choosing something rebuilds exactly that component (§2.5).
        self.state = state.get();
        self.bound = Some(state);
        self
    }

    /// Whether the menu is open.
    pub fn open(mut self, open: bool) -> Self {
        self.state.open = open;
        self
    }

    /// Disable the trigger (still announced by screen readers, as dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Draw the trigger as a **chip**: fully rounded, tighter, no shadow.
    pub fn chip(mut self, chip: bool) -> Self {
        self.chip = chip;
        self
    }

    /// How shortcuts are spelled (defaults to the convention of the target OS).
    pub fn shortcut_style(mut self, style: ShortcutStyle) -> Self {
        self.shortcut_style = style;
        self
    }

    /// The spring that drives state transitions (`smooth`/`snappy`/`bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// A floor under the **root** panel's width.
    ///
    /// Menus size themselves to their longest label, which is right for a menu
    /// and wrong for a list of suggestions hanging under a text field: there,
    /// the panel has an edge to line up with. [`mod@crate::combo_box`] passes the
    /// field's width here.
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = (width.is_finite() && width > 0.0).then_some(width);
        self
    }

    /// Whether the trigger can take keyboard focus.
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

    /// Receive every user intent raw — the path for applications that manage
    /// their own state.
    pub fn on_intent(mut self, f: impl Fn(MenuIntent) + 'static) -> Self {
        self.on_intent = Some(MenuHandler::new(f));
        self
    }

    /// Called with the item's id every time the user chooses one.
    ///
    /// A submenu parent never arrives here: choosing it opens its submenu, and
    /// that rule lives in [`MenuState::apply`], not in the application.
    pub fn on_activate(mut self, f: impl Fn(&str) + 'static) -> Self {
        self.on_activate = Some(Rc::new(f));
        self
    }

    /// The identity prefix for this menu's nodes among their siblings (§2.5).
    ///
    /// A `String` rather than a [`Key`](silka_core::signals::Key) because a
    /// menu produces **several** keyed nodes — the trigger plus one overlay per
    /// open level — and all of them are derived from this prefix.
    ///
    /// **Give every menu on a page its own prefix.** Two menus mounted in the
    /// same [`overlay_layer`](crate::overlay::overlay_layer) with the default
    /// prefix would hand their panels the same key, and diffing would then
    /// happily reuse one menu's node for the other's panel.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }

    // -- readers (used by the gallery, the tests, and the code below) --------

    /// The menu tree.
    pub fn model(&self) -> &MenuModel {
        &self.model
    }

    /// The state currently in effect.
    pub fn state_value(&self) -> &MenuState {
        &self.state
    }

    /// Height of one row — which is also the minimum hit target (HIG).
    pub fn row_height(&self) -> f32 {
        MIN_HIT_TARGET
    }

    /// The label of the item at `(depth, index)`, if it exists.
    pub fn item_label(&self, depth: usize, index: usize) -> Option<&str> {
        self.model
            .item_at(&self.state.path_at(depth), index)
            .map(MenuItem::label)
    }

    // -- token resolution ----------------------------------------------------

    fn text_style(&self) -> TextStyle {
        TextStyle::new()
            .size(self.theme.typography.body_size)
            .weight(FontWeight::REGULAR)
            .single_line()
    }

    fn measure(&self, s: &str) -> f32 {
        if s.is_empty() {
            return 0.0;
        }
        let gaya = self.text_style();
        self.fonts.with(|m| {
            m.measure(s, &gaya, TextConstraints::UNBOUNDED)
                .content_size
                .width
        })
    }

    /// Paint values for the trigger — used by the gallery and the token tests.
    pub fn trigger_style(&self) -> MenuTriggerStyle {
        self.trigger_style_for(MenuTriggerMode::Press)
    }

    fn trigger_style_for(&self, mode: MenuTriggerMode) -> MenuTriggerStyle {
        let t = &self.theme;
        // A context region draws nothing at all: it is the content underneath
        // that the user sees, and a box around it would be a lie about what is
        // interactive.
        if mode == MenuTriggerMode::Context {
            let bening = t.color.surface.with_alpha(0.0);
            return MenuTriggerStyle {
                rest: bening,
                hover: bening,
                pressed: bening,
                disabled: bening,
                corners: t.corners(0.0),
                border_width: 0.0,
                border: bening,
                border_disabled: bening,
                shadows: silka_paint::ShadowPair::NONE,
                focus_ring_width: t.space(0.5),
                focus_ring: t.color.focus_ring,
                padding: Insets::ZERO,
                gap: 0.0,
                indicator: 0.0,
                indicator_color: bening,
                min_height: 0.0,
            };
        }
        let radius = if self.chip {
            t.radius.full
        } else {
            t.radius.md
        };
        MenuTriggerStyle {
            rest: t.color.surface,
            hover: t.color.surface_hover,
            pressed: t.color.surface_pressed,
            // A disabled control **fades toward the page background** — the
            // same rule macOS uses, and the value stays derived from tokens.
            disabled: t.color.surface.lerp(t.color.background, 0.6),
            corners: t.corners(radius),
            border_width: t.space(0.25),
            border: t.color.border,
            border_disabled: t.color.separator,
            // A chip is lighter than a button in shape only: the same shadow
            // token keeps the two reading as one family.
            shadows: t.shadow.sm,
            focus_ring_width: t.space(0.5),
            focus_ring: t.color.focus_ring,
            padding: Insets::symmetric(t.space(3.0), t.space(1.5)),
            gap: t.space(2.0),
            indicator: t.space(2.0),
            indicator_color: if self.disabled {
                t.color.disabled_label
            } else {
                t.color.secondary_label
            },
            min_height: MIN_HIT_TARGET,
        }
    }

    /// Paint values for one row of the level at `depth`.
    ///
    /// The gutters are computed **per level**: a level where nothing is
    /// checkable reserves no room for a mark, and a level with no submenus
    /// reserves none for a triangle — but within one level every label starts
    /// at the same x, which is what makes a menu look like a menu instead of a
    /// list of buttons.
    pub fn row_style(&self, depth: usize) -> MenuRowStyle {
        let t = &self.theme;
        let entries = self
            .model
            .level(&self.state.path_at(depth))
            .unwrap_or(&[][..]);
        let ada_tanda = entries
            .iter()
            .filter_map(MenuEntry::item)
            .any(|i| i.mark().is_some());
        let ada_submenu = entries
            .iter()
            .filter_map(MenuEntry::item)
            .any(MenuItem::has_submenu);
        MenuRowStyle {
            // A resting row draws nothing: what you see is the panel surface
            // behind it.
            rest: t.color.surface_hover.with_alpha(0.0),
            // The same highlight `select` uses for its rows. Deliberately not
            // the macOS accent fill: that one also demands the label flip to
            // `on_accent`, and a label colour that jumps while its background
            // glides is worse than a quieter highlight.
            highlight: t.color.surface_hover,
            corners: t.corners(t.radius.sm),
            padding: Insets::symmetric(t.space(2.0), t.space(1.0)),
            leading: if ada_tanda { t.space(5.0) } else { 0.0 },
            trailing: if ada_submenu { t.space(4.0) } else { 0.0 },
            mark: t.color.accent,
            arrow: t.color.secondary_label,
            min_height: MIN_HIT_TARGET,
        }
    }

    /// The panel width of the level at `depth`, in logical points.
    ///
    /// Measured with the same text engine that will later draw the labels, so
    /// nowhere is a glyph width ever guessed (§3.3, §3.4). The **root** panel is
    /// additionally never narrower than [`Menu::min_width`] — what a
    /// [`mod@crate::combo_box`] uses to make its suggestion list line up with the
    /// field above it. Submenus are unaffected: they hang beside their parent
    /// and have nothing to line up with.
    pub fn level_width(&self, depth: usize) -> f32 {
        let t = &self.theme;
        let gaya = self.row_style(depth);
        let entries = self
            .model
            .level(&self.state.path_at(depth))
            .unwrap_or(&[][..]);
        let jarak = t.space(2.0);
        let mut isi: f32 = t.space(24.0); // a floor, so a one-word menu is not a sliver
        for it in entries.iter().filter_map(MenuEntry::item) {
            let ikon = it
                .icon_text()
                .map(|s| self.measure(s) + jarak)
                .unwrap_or(0.0);
            let pintasan = it
                .accelerator()
                .map(|s| self.measure(&s.display(self.shortcut_style)) + jarak * 2.0)
                .unwrap_or(0.0);
            isi = isi.max(ikon + self.measure(it.label()) + pintasan);
        }
        let lebar =
            (isi + gaya.leading + gaya.trailing + gaya.padding.horizontal() + t.space(2.0)).ceil();
        match self.min_width {
            Some(w) if depth == 0 => lebar.max(w),
            _ => lebar,
        }
    }

    /// The handler that turns intent into new state.
    fn handler(&self) -> MenuHandler {
        let model = self.model.clone();
        let state = self.state.clone();
        let bound = self.bound;
        let luar = self.on_intent.clone();
        let dipilih = self.on_activate.clone();
        MenuHandler::new(move |intent| {
            // Resolve the activation **before** applying: applying closes the
            // menu and clears the path the item was found through.
            if let (MenuIntent::Activate { depth, index }, Some(f)) = (intent, &dipilih) {
                if let Some(it) = state.activated(&model, depth, index) {
                    f(it.id());
                }
            }
            if let Some(sig) = bound {
                // `peek`, not `get`: the handler runs outside of build, and
                // subscribing from inside an event handler is never right.
                let mut baru = sig.peek();
                if baru.apply(intent, &model) {
                    sig.set(baru);
                }
            }
            if let Some(h) = &luar {
                h.emit(intent);
            }
        })
    }

    // -- the pieces mounted in two places ------------------------------------

    /// The trigger as a button or chip, with `label` written on it.
    pub fn trigger(&self, label: impl Into<String>) -> View {
        let t = &self.theme;
        let label = label.into();
        let warna = if self.disabled {
            t.color.disabled_label
        } else {
            t.color.label
        };
        let isi = text_in(&self.fonts, label)
            .size(t.typography.body_size)
            .weight(FontWeight::MEDIUM)
            .color(warna)
            .single_line()
            // The control's name is announced once, from the trigger node.
            .role(AccessRole::Container);
        self.trigger_with(isi)
    }

    /// The trigger with arbitrary content inside it.
    ///
    /// The content is drawn, not consulted: an interactive child would swallow
    /// the click that is supposed to open the menu, so what belongs in here is
    /// a label, an icon, or a row of both.
    pub fn trigger_with(&self, child: impl Into<View>) -> View {
        self.pemicu(MenuTriggerMode::Press, child.into())
    }

    /// A region whose **right-click** (or Shift+F10) opens this menu.
    ///
    /// Its content keeps every primary click it would otherwise have had: the
    /// region joins the hit path without absorbing anything
    /// ([`HitBehavior::Translucent`](silka_core::input::HitBehavior)).
    pub fn context_area(&self, child: impl Into<View>) -> View {
        self.pemicu(MenuTriggerMode::Context, child.into())
    }

    fn pemicu(&self, mode: MenuTriggerMode, child: View) -> View {
        Builder::new(MenuTriggerProps {
            style: self.trigger_style_for(mode),
            mode,
            model: self.model.clone(),
            state: self.state.clone(),
            label: self.label.clone(),
            disabled: self.disabled,
            focus: self.focus,
            spring: self.spring,
            on_intent: Some(self.handler()),
        })
        .key(format!("{}::trigger", self.key))
        .child(child)
        .into()
    }

    /// Every panel that should currently exist, **outermost first**.
    ///
    /// Mount them all in [`overlay_layer`](crate::overlay::overlay_layer), in
    /// the order they come back — that order *is* the stacking order, so a
    /// submenu paints over its parent.
    ///
    /// The root panel is always here, open or not: an overlay that stays in the
    /// tree is what lets the menu's disappearance be animated as smoothly as
    /// its arrival ([`mod@crate::overlay`]). Submenus come and go with the open
    /// chain, which matches how a native menu drops a submenu the instant the
    /// pointer moves elsewhere.
    pub fn overlays(&self) -> Vec<OverlayBuilder> {
        let t = &self.theme;
        let handler = self.handler();
        let mut out = Vec::new();

        let tutup = handler.clone();
        let mut akar = overlay(self.panel(0))
            .open(self.state.open)
            .anchor(self.state.anchor)
            .placement(
                Placement::anchored(Side::Bottom)
                    .align(Align::Start)
                    .gap(t.space(1.0)),
            )
            // A menu, not a dialog: the content behind stays alive for the
            // keyboard and for screen readers, but a click outside dismisses.
            .barrier(Barrier::Light)
            .dismiss(Dismiss::ALL)
            .no_backdrop()
            .role(AccessRole::Menu)
            .spring(self.spring)
            .key(format!("{}::level0", self.key))
            .on_dismiss(move || tutup.emit(MenuIntent::Close));
        if let Some(label) = &self.label {
            akar = akar.label(label.clone());
        }
        out.push(akar);

        for depth in 1..self.state.visible_levels() {
            let Some(anchor) = self.state.levels[depth - 1].anchor else {
                break;
            };
            let mut panel = overlay(self.panel(depth))
                .open(true)
                .anchor(anchor)
                .placement(
                    // Towards the end of the line, so an Arabic UI opens its
                    // submenus leftwards with no extra code (§9.8) — and the
                    // overlay system flips to the other side by itself when the
                    // screen edge gets in the way.
                    Placement::anchored(Side::End)
                        .align(Align::Start)
                        // No gap: the shorter the pointer's path from the
                        // parent row to the submenu, the fewer sibling rows it
                        // can brush against on the way (see the module docs).
                        .gap(0.0),
                )
                // Only the panel takes the pointer. A second *light* barrier
                // here would swallow the click meant for a row of the parent
                // panel behind it.
                .barrier(Barrier::Panel)
                .dismiss(Dismiss::NONE)
                .no_backdrop()
                .role(AccessRole::Menu)
                .spring(self.spring)
                .key(format!("{}::level{depth}", self.key));
            if let Some(label) = self.item_label(depth - 1, self.state.levels[depth - 1].index) {
                panel = panel.label(label.to_string());
            }
            out.push(panel);
        }
        out
    }

    /// One panel: the rows of the level at `depth`, on the elevated surface.
    fn panel(&self, depth: usize) -> View {
        let t = &self.theme;
        let handler = self.handler();
        let gaya = self.row_style(depth);
        let path = self.state.path_at(depth);
        let entries: Vec<MenuEntry> = self
            .model
            .level(&path)
            .map(<[MenuEntry]>::to_vec)
            .unwrap_or_default();
        let disorot = self.state.highlight_at(depth);

        let baris: Vec<View> = entries
            .iter()
            .enumerate()
            .map(|(i, entry)| match entry {
                MenuEntry::Separator => Builder::new(MenuSeparatorProps {
                    color: t.color.separator,
                    thickness: t.space(0.25).max(1.0),
                    inset: t.space(1.0),
                    height: t.space(2.0),
                })
                // Key discipline in a dynamic list (§2.5).
                .key(i)
                .into(),
                MenuEntry::Item(it) => {
                    let submenu_terbuka = self.state.is_submenu_open(depth, i);
                    Builder::new(MenuRowProps {
                        style: gaya,
                        depth,
                        index: i,
                        label: Some(it.label().to_string()),
                        enabled: it.is_enabled(),
                        mark: it.mark(),
                        checked: it.is_checked(),
                        has_submenu: it.has_submenu(),
                        submenu_open: submenu_terbuka,
                        highlighted: self.state.open && disorot == Some(i),
                        // The state opened this submenu without knowing where
                        // the row is; the sync pass fills that in.
                        wants_anchor: submenu_terbuka && self.state.levels[depth].anchor.is_none(),
                        spring: self.spring,
                        on_intent: Some(handler.clone()),
                    })
                    .key(i)
                    .child(self.isi_baris(it))
                    .into()
                }
            })
            .collect();

        let isi = column(baris).cross(CrossAlign::Stretch);
        let panel = pad(Insets::all(t.space(1.0)), isi)
            .background(t.color.surface_elevated)
            .corners(t.corners(t.radius.lg))
            .border(t.space(0.25), t.color.separator)
            .shadow(t.shadow.lg);
        let lebar = self.level_width(depth);
        constrained(BoxConstraints::new(lebar, lebar, 0.0, f32::INFINITY), panel).into()
    }

    /// The content of one row: icon, label, and the shortcut pushed to the end.
    fn isi_baris(&self, it: &MenuItem) -> View {
        let t = &self.theme;
        let warna = if it.is_enabled() {
            t.color.label
        } else {
            t.color.disabled_label
        };
        let mut anak: Vec<View> = Vec::with_capacity(4);
        if let Some(ikon) = it.icon_text() {
            anak.push(
                text_in(&self.fonts, ikon)
                    .size(t.typography.body_size)
                    .color(warna)
                    .single_line()
                    .role(AccessRole::Container)
                    .into(),
            );
        }
        anak.push(
            text_in(&self.fonts, it.label())
                .size(t.typography.body_size)
                .color(warna)
                .single_line()
                // The row's name is announced from the row node, not twice.
                .role(AccessRole::Container)
                .into(),
        );
        // The shortcut sits at the end of the line whatever the label's length:
        // it is the flex spacer that puts it there, not arithmetic (§3.4).
        anak.push(expanded(fixed(0.0, 0.0)).into());
        if let Some(s) = it.accelerator() {
            anak.push(
                text_in(&self.fonts, s.display(self.shortcut_style))
                    .size(t.typography.body_size)
                    .color(if it.is_enabled() {
                        t.color.tertiary_label
                    } else {
                        t.color.disabled_label
                    })
                    .single_line()
                    .role(AccessRole::Container)
                    .into(),
            );
        }
        row(anak)
            .spacing(t.space(2.0))
            .cross(CrossAlign::Center)
            .into()
    }
}

impl core::fmt::Debug for Menu {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Menu")
            .field("model", &self.model)
            .field("label", &self.label)
            .field("state", &self.state)
            .field("disabled", &self.disabled)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Frame pass
// ---------------------------------------------------------------------------

/// Every node of the tree, parent before child.
fn semua(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    out.push(id);
    for anak in tree.children(id) {
        kumpulkan(tree, *anak, out);
    }
}

/// The nearest [`OverlayLayer`] above `id` — the coordinate space every anchor
/// is expressed in.
fn layer_of(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
    let mut kini = tree.parent(id);
    while let Some(n) = kini {
        if tree.node_ref::<OverlayLayer>(n).is_some() {
            return Some(n);
        }
        kini = tree.parent(n);
    }
    None
}

/// `id`'s rect in the coordinates of its overlay layer.
///
/// This is the same conversion [`crate::overlay::anchor_rect`] performs, done
/// from inside the frame pass because that is the only place that may look at
/// another node's geometry (`silka_core::tree`: a node never knows its own
/// position).
fn rect_lokal(tree: &RenderTree, id: NodeId) -> Rect {
    let asal = layer_of(tree, id)
        .map(|l| tree.global_offset(l))
        .unwrap_or(Point::ZERO);
    let g = tree.global_offset(id);
    Rect::from_origin_size(Point::new(g.x - asal.x, g.y - asal.y), tree.size(id))
}

/// Advance every menu animation by one frame, then publish the geometry the
/// view layer could not know when it was built.
///
/// The second half is the same seam [`crate::list::sync_virtual`] uses: a
/// trigger that was clicked, and a submenu opened from the keyboard, both need
/// a rect **in layer coordinates**, and only the tree knows one. So they leave
/// a request behind and it is answered here, one frame later — which is why a
/// panel never flashes at the wrong place before sliding to the right one.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in semua(tree) {
        // 1. Springs.
        let gerak = if let Some(t) = tree.node_mut_ref::<MenuTriggerBox>(id) {
            Some((t.advance(tick), t.is_animating()))
        } else {
            tree.node_mut_ref::<MenuRowBox>(id)
                .map(|r| (r.advance(tick), r.is_animating()))
        };
        if let Some((bergeser, bergerak)) = gerak {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
        }

        // 2. A trigger waiting for its anchor.
        let tertunda = tree
            .node_mut_ref::<MenuTriggerBox>(id)
            .and_then(MenuTriggerBox::take_pending);
        if let Some(p) = tertunda {
            let kotak = rect_lokal(tree, id);
            let anchor = match p {
                trigger::Pending::Rect => Anchor::Rect(kotak),
                // A context menu opens where the cursor is, not where the
                // region is.
                trigger::Pending::At(lokal) => {
                    Anchor::Point(Point::new(kotak.min_x() + lokal.x, kotak.min_y() + lokal.y))
                }
            };
            if let Some(t) = tree.node_ref::<MenuTriggerBox>(id) {
                t.kirim(MenuIntent::Open(anchor));
            }
            dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
        }

        // 3. A row whose submenu was opened by keyboard and has no rect yet.
        if tree
            .node_ref::<MenuRowBox>(id)
            .is_some_and(MenuRowBox::wants_anchor)
        {
            let kotak = rect_lokal(tree, id);
            if let Some(r) = tree.node_ref::<MenuRowBox>(id) {
                r.kirim_anchor(kotak);
            }
            dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
        }
    }
    dirty
}

/// True while any menu transition is still running.
pub fn is_animating(tree: &RenderTree) -> bool {
    semua(tree).into_iter().any(|id| {
        tree.node_ref::<MenuTriggerBox>(id)
            .is_some_and(MenuTriggerBox::is_animating)
            || tree
                .node_ref::<MenuRowBox>(id)
                .is_some_and(MenuRowBox::is_animating)
    })
}

/// Finish every menu transition instantly (tests, snapshots, reduced-motion).
pub fn settle(tree: &mut RenderTree) {
    for id in semua(tree) {
        let kena = if let Some(t) = tree.node_mut_ref::<MenuTriggerBox>(id) {
            t.settle();
            true
        } else if let Some(r) = tree.node_mut_ref::<MenuRowBox>(id) {
            r.settle();
            true
        } else {
            false
        };
        if kena {
            tree.mark_needs_paint(id);
        }
    }
}
