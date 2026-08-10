//! `select()` — Tier 2 component (`KOMPONEN.md`): the macOS pop-up button /
//! shadcn Select.
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::column;
//! # use silka_theme::{Appearance, Theme};
//! # use silka_widgets::{overlay::overlay_layer, select, Fonts, SelectState};
//! # let rt = Runtime::new();
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! let state = rt.signal(SelectState::with_selected(0));
//!
//! let mata_uang = select(&fonts, &t, ["IDR", "USD", "EUR"])
//!     .label("Mata uang")
//!     .bind(state);
//!
//! // The trigger stands inside the content; the popup lives in the overlay layer.
//! let _ = overlay_layer(column([mata_uang.trigger()]))
//!     .overlay(mata_uang.popup());
//! ```
//!
//! ## Why two pieces instead of one view
//!
//! The popup **must not** live where its trigger stands: it has to paint over
//! other content and be free to spill past its parent's box. The infrastructure
//! for that already exists, built once for ten components ([`mod@crate::overlay`],
//! `KOMPONEN.md` rule #3), and it takes the shape of a layer at the root of the
//! page. Since there is no "portal" mechanism yet that could hand a panel from
//! deep in the tree up to that layer, select hands back two pieces mounted in
//! two places: [`Select::trigger`] inside the content and [`Select::popup`] in
//! the layer. Once a portal exists, the only thing that changes is this file —
//! not the apps using it, because both pieces are born from the same builder.
//!
//! ## Who owns the state
//!
//! All of the state lives in the application's [`SelectState`], and the render
//! nodes only **report intent** ([`SelectIntent`]). [`Select::bind`] wires the
//! two together through a single [`Signal`], so the ordinary case is one line;
//! an app that wants to drive things itself (validation, undo, syncing to a
//! server) reaches for [`Select::state`] + [`Select::on_intent`] and loses
//! nothing.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Requirement | Where |
//! |---|---|
//! | Correct in both presets | Every value flows through [`SelectTriggerStyle`]/[`SelectOptionStyle`], filled from tokens |
//! | Interactive states via springs | Background, focus ring, and disclosure triangle (`trigger`); row background (`option`) |
//! | Full keyboard + focus ring | Space/Enter/arrows/Home/End/Esc + typeahead, all on the trigger that holds focus |
//! | AccessKit nodes | Trigger = `Button` + value + `Expand`/`Collapse`; row = `MenuItem` + `toggled` |
//! | Dark mode | Tokens; not a single color literal in this file |
//! | Hit target ≥ 44pt | `min_height` on the trigger **and** on every row |
//! | Reduced-motion | Every spring runs through [`Tick`](silka_core::animation::Tick), which carries [`Motion`](silka_core::animation::Motion) |
//!
//! ## Deliberately not here yet
//!
//! - **A search box inside the popup** (`KOMPONEN.md`: "search/filter
//!   opsional") is waiting on `text_field`. What does exist, and covers the
//!   same need for medium-sized lists, is **typeahead** — typing letters jumps
//!   to the matching option, exactly like a native menu.
//! - **Nested/grouped options**, and disabling options one at a time.

mod option;
mod state;
#[cfg(test)]
mod tests;
mod trigger;

use std::rc::Rc;

use silka_core::access::AccessRole;
use silka_core::animation::Spring;
use silka_core::input::FocusPolicy;
use silka_core::signals::{Key, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign};
use silka_core::view::{column, constrained, pad, viewport, Builder, View};
use silka_paint::Insets;
use silka_text::{FontWeight, TextConstraints, TextStyle};
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::overlay::{overlay, Align, Anchor, Barrier, Dismiss, OverlayBuilder, Placement, Side};
use crate::text::text;

pub use option::{SelectOption, SelectOptionProps, SelectOptionStyle};
pub use state::{SelectIntent, SelectState};
pub use trigger::{bar_width, cari_awalan, SelectTrigger, SelectTriggerProps, SelectTriggerStyle};

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// Where a [`SelectIntent`] is sent.
///
/// Shaped exactly like [`Callback`](silka_core::Callback) — cheap `Clone`,
/// equality by identity — except that it carries one argument, which the core
/// has no equivalent for yet.
#[derive(Clone)]
pub struct SelectHandler(Rc<dyn Fn(SelectIntent)>);

impl SelectHandler {
    /// Wrap a closure.
    pub fn new(f: impl Fn(SelectIntent) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Send one intent.
    pub fn emit(&self, intent: SelectIntent) {
        (self.0)(intent)
    }
}

impl PartialEq for SelectHandler {
    /// Identity, not contents: the same `Rc` means the same handler.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for SelectHandler {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SelectHandler")
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Dart-style select builder (§2.5).
///
/// It keeps the raw ingredients and only **resolves tokens** when it becomes a
/// view, so a method called late still changes the whole result. `Clone` is
/// cheap: [`Select::trigger`] and [`Select::popup`] use the same builder, so
/// there is no way for the two to drift apart.
#[derive(Clone)]
pub struct Select {
    fonts: Fonts,
    theme: Theme,
    options: Rc<Vec<String>>,
    label: Option<String>,
    placeholder: String,
    state: SelectState,
    disabled: bool,
    width: Option<f32>,
    max_visible: usize,
    spring: Spring,
    focus: FocusPolicy,
    bound: Option<Signal<SelectState>>,
    on_intent: Option<SelectHandler>,
    on_select: Option<Rc<dyn Fn(usize)>>,
    key: Option<Key>,
}

/// A single choice out of a list — the `select` component (`KOMPONEN.md`).
///
/// `fonts` is the app's text engine, `theme` the source of every value.
pub fn select<S: Into<String>>(
    fonts: &Fonts,
    theme: &Theme,
    options: impl IntoIterator<Item = S>,
) -> Select {
    Select {
        fonts: fonts.clone(),
        theme: *theme,
        options: Rc::new(options.into_iter().map(Into::into).collect()),
        label: None,
        placeholder: String::from("Pilih…"),
        state: SelectState::new(),
        disabled: false,
        width: None,
        max_visible: 8,
        // `snappy` is how a macOS control feels: quick to arrive, almost no
        // bounce (WWDC23).
        spring: Spring::snappy(),
        focus: FocusPolicy::FOCUSABLE,
        bound: None,
        on_intent: None,
        on_select: None,
        key: None,
    }
}

impl Select {
    /// The name a screen reader announces (and the popup's title).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Text shown while nothing is selected.
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// The state in effect, fully controlled by the application.
    pub fn state(mut self, state: SelectState) -> Self {
        self.state = state;
        self
    }

    /// Wire it to a single signal: reading it **and** writing to it.
    ///
    /// This is the shape 95% of apps use — one piece of state to keep, and
    /// every rule (highlight clamped, scroll following along, popup closing
    /// after a choice) is already right because it all goes through
    /// [`SelectState::apply`].
    pub fn bind(mut self, state: Signal<SelectState>) -> Self {
        // Read **during build**, so the component calling it subscribes:
        // choosing something rebuilds exactly that component (§2.5).
        self.state = state.get();
        self.bound = Some(state);
        self
    }

    /// The selected index.
    pub fn selected(mut self, selected: Option<usize>) -> Self {
        self.state.selected = selected;
        self
    }

    /// Whether the popup is open.
    pub fn open(mut self, open: bool) -> Self {
        self.state.open = open;
        self
    }

    /// The popup's anchor, in the overlay layer's local coordinates.
    ///
    /// Rarely set by hand: [`SelectIntent::Open`] carries the trigger's rect
    /// and [`SelectState::apply`] is what stores it.
    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.state.anchor = anchor;
        self
    }

    /// Disable the control (still announced by screen readers, as dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Box width, in logical points. Without it the width is measured from the
    /// longest option — the NSPopUpButton habit, and what keeps the control
    /// from resizing every time the selection changes.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width.max(0.0));
        self
    }

    /// How many rows are visible before the popup starts to scroll.
    pub fn max_visible(mut self, rows: usize) -> Self {
        self.max_visible = rows.max(1);
        self
    }

    /// The spring that drives state transitions (`smooth`/`snappy`/`bouncy`).
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

    /// Receive every user intent raw — the path for apps that manage their own
    /// state.
    pub fn on_intent(mut self, f: impl Fn(SelectIntent) + 'static) -> Self {
        self.on_intent = Some(SelectHandler::new(f));
        self
    }

    /// Called every time the user picks a row.
    pub fn on_select(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    // -- readers (used by the gallery, the tests, and the code below) --------

    /// The list of options.
    pub fn options(&self) -> &[String] {
        &self.options
    }

    /// The state currently in effect.
    pub fn state_value(&self) -> SelectState {
        self.state
    }

    /// The current option's text, if there is one.
    pub fn selected_label(&self) -> Option<&str> {
        self.state
            .selected
            .and_then(|i| self.options.get(i))
            .map(String::as_str)
    }

    /// The text shown on the trigger: the current option, or the placeholder.
    pub fn display_text(&self) -> &str {
        self.selected_label().unwrap_or(&self.placeholder)
    }

    /// Height of one popup row — which is also the minimum hit target (HIG).
    pub fn row_height(&self) -> f32 {
        MIN_HIT_TARGET
    }

    /// How many rows are actually visible in the popup.
    pub fn visible_rows(&self) -> usize {
        self.options.len().clamp(1, self.max_visible.max(1))
    }

    /// True when the list is longer than its window.
    pub fn is_scrollable(&self) -> bool {
        self.options.len() > self.max_visible
    }

    /// The effective box width, in logical points.
    ///
    /// Deliberately **not** routed through [`Select::trigger_style`]: that
    /// style carries `min_width` itself, so asking it from here would be
    /// circular.
    pub fn width_value(&self) -> f32 {
        self.width.unwrap_or_else(|| {
            self.content_width() + self.padding().horizontal() + self.gap() + self.indicator()
        })
    }

    /// Distance from the content to the edges of the box.
    fn padding(&self) -> Insets {
        Insets::symmetric(self.theme.space(3.0), self.theme.space(1.5))
    }

    /// Gap between the label and the disclosure triangle.
    fn gap(&self) -> f32 {
        self.theme.space(2.0)
    }

    /// Width of the disclosure triangle.
    fn indicator(&self) -> f32 {
        self.theme.space(2.0)
    }

    /// Width of the longest text (the placeholder counts), in logical points.
    ///
    /// Measured with the same text engine that will later draw it, so nowhere
    /// is a glyph width ever guessed (§3.3, §3.4).
    pub fn content_width(&self) -> f32 {
        let gaya = self.text_style();
        self.fonts.with(|m| {
            let mut w = m
                .measure(&self.placeholder, &gaya, TextConstraints::UNBOUNDED)
                .content_size
                .width;
            for o in self.options.iter() {
                w = w.max(
                    m.measure(o, &gaya, TextConstraints::UNBOUNDED)
                        .content_size
                        .width,
                );
            }
            w.ceil()
        })
    }

    fn text_style(&self) -> TextStyle {
        TextStyle::new()
            .size(self.theme.typography.body_size)
            .weight(FontWeight::MEDIUM)
            .single_line()
    }

    /// Paint values for the trigger — used by the gallery and the token tests.
    pub fn trigger_style(&self) -> SelectTriggerStyle {
        let t = &self.theme;
        SelectTriggerStyle {
            rest: t.color.surface,
            hover: t.color.surface_hover,
            pressed: t.color.surface_pressed,
            // A disabled control **fades toward the page background** — the
            // same rule macOS uses, and the value stays derived from tokens.
            disabled: t.color.surface.lerp(t.color.background, 0.6),
            corners: t.corners(t.radius.md),
            border_width: t.space(0.25),
            border: t.color.border,
            border_disabled: t.color.separator,
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
            min_width: self.width_value(),
            min_height: MIN_HIT_TARGET,
        }
    }

    /// Paint values for a single popup row.
    pub fn option_style(&self) -> SelectOptionStyle {
        let t = &self.theme;
        SelectOptionStyle {
            // A resting row draws nothing: what you see is the panel surface
            // behind it.
            rest: t.color.surface_hover.with_alpha(0.0),
            highlight: t.color.surface_hover,
            selected: t.color.accent_muted,
            corners: t.corners(t.radius.sm),
            padding: Insets::symmetric(t.space(2.0), t.space(1.0)),
            marker: t.color.accent,
            marker_size: t.space(1.5),
            min_height: MIN_HIT_TARGET,
        }
    }

    /// The handler that turns intent into new state.
    fn handler(&self) -> SelectHandler {
        let count = self.options.len();
        let visible = self.visible_rows();
        let bound = self.bound;
        let luar = self.on_intent.clone();
        let dipilih = self.on_select.clone();
        SelectHandler::new(move |intent| {
            if let Some(sig) = bound {
                // `peek`, not `get`: the handler runs outside of build, and
                // subscribing from inside an event handler is never right.
                let mut baru = sig.peek();
                if baru.apply(intent, count, visible) {
                    sig.set(baru);
                }
            }
            if let Some(h) = &luar {
                h.emit(intent);
            }
            if let SelectIntent::Commit(i) = intent {
                if let Some(f) = &dipilih {
                    if count > 0 {
                        f(i.min(count - 1));
                    }
                }
            }
        })
    }

    // -- the two pieces mounted in two places --------------------------------

    /// The trigger box — mounted inside the page content.
    pub fn trigger(&self) -> View {
        let t = &self.theme;
        let warna = if self.disabled {
            t.color.disabled_label
        } else if self.state.selected.is_some() {
            t.color.label
        } else {
            // The placeholder is dimmer than real content.
            t.color.tertiary_label
        };
        let isi = text(&self.fonts, self.display_text())
            .size(t.typography.body_size)
            .weight(FontWeight::MEDIUM)
            .color(warna)
            .single_line()
            // The control's name is announced once, from the select node — not twice.
            .role(AccessRole::Container);

        let mut b = Builder::new(SelectTriggerProps {
            style: self.trigger_style(),
            label: self.label.clone(),
            value: self.selected_label().map(str::to_string),
            options: self.options.clone(),
            open: self.state.open,
            highlight: self.state.highlight,
            disabled: self.disabled,
            focus: self.focus,
            spring: self.spring,
            on_intent: Some(self.handler()),
        })
        .child(isi);
        if let Some(key) = &self.key {
            b = b.key(key.clone());
        }
        b.into()
    }

    /// The options panel — mounted in [`crate::overlay::overlay_layer`].
    ///
    /// Placement is left entirely to the overlay system: it sits below the
    /// trigger, aligned to the start of the line, and **flips upward on its
    /// own** when it runs into the bottom of the screen. Not a single
    /// coordinate is computed in this file (`KOMPONEN.md` rule #3).
    ///
    /// One known limit: on a scrollable list the scroll position is
    /// **controlled** by [`SelectState::first_visible`] so the keyboard
    /// highlight is always visible. Mouse-wheel scrolling still works, but the
    /// next rebuild snaps it back to the highlight's window. Reconciling the
    /// two needs a scroll position that can be read back from the node — a hook
    /// [`silka_core::tree::Viewport`] does not have yet.
    pub fn popup(&self) -> OverlayBuilder {
        let t = &self.theme;
        let handler = self.handler();
        let gaya_baris = self.option_style();

        let baris: Vec<View> = self
            .options
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let terpilih = self.state.selected == Some(i);
                let disorot = self.state.open && self.state.highlight == i;
                let isi = text(&self.fonts, label)
                    .size(t.typography.body_size)
                    .weight(FontWeight::REGULAR)
                    .color(if terpilih {
                        t.color.accent
                    } else {
                        t.color.label
                    })
                    .single_line()
                    // The row's name is announced from the row node, not twice.
                    .role(AccessRole::Container);
                Builder::new(SelectOptionProps {
                    style: gaya_baris,
                    index: i,
                    label: Some(label.clone()),
                    selected: terpilih,
                    highlighted: disorot,
                    spring: self.spring,
                    on_intent: Some(handler.clone()),
                })
                // Key discipline in a dynamic list (§2.5).
                .key(i)
                .child(isi)
                .into()
            })
            .collect();

        let daftar = column(baris).cross(CrossAlign::Stretch);
        let tinggi_baris = self.row_height();
        let isi: View = if self.is_scrollable() {
            let tinggi = tinggi_baris * self.visible_rows() as f32;
            // Scroll is **derived from the highlight**
            // ([`SelectState::first_visible`]): arrow-down past the last
            // visible row shifts the window by one row instead of jumping to
            // the middle.
            constrained(
                BoxConstraints::new(0.0, f32::INFINITY, tinggi, tinggi),
                viewport(daftar)
                    .scroll(self.state.scroll_offset(tinggi_baris))
                    .line_height(tinggi_baris),
            )
            .into()
        } else {
            daftar.into()
        };

        let panel = pad(Insets::all(t.space(1.0)), isi)
            .background(t.color.surface_elevated)
            .corners(t.corners(t.radius.lg))
            .border(t.space(0.25), t.color.separator)
            .shadow(t.shadow.lg);
        // The panel's width is locked to the trigger's: a list that "jumps
        // wider" as it opens is the first thing that makes a select feel cheap.
        let lebar = self.width_value();
        let panel = constrained(BoxConstraints::new(lebar, lebar, 0.0, f32::INFINITY), panel);

        let tutup = handler.clone();
        let mut b = overlay(panel)
            .open(self.state.open)
            .anchor(self.state.anchor)
            .placement(
                Placement::anchored(Side::Bottom)
                    .align(Align::Start)
                    .gap(t.space(1.0)),
            )
            // A popup, not a dialog: the content behind stays alive for the
            // keyboard and screen readers, but a click outside dismisses it.
            .barrier(Barrier::Light)
            .dismiss(Dismiss::ALL)
            .no_backdrop()
            .role(AccessRole::Menu)
            .spring(self.spring)
            .on_dismiss(move || tutup.emit(SelectIntent::Close));
        if let Some(label) = &self.label {
            b = b.label(label.clone());
        }
        if let Some(key) = &self.key {
            b = b.key(key.clone());
        }
        b
    }
}

impl core::fmt::Debug for Select {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Select")
            .field("options", &self.options.len())
            .field("label", &self.label)
            .field("state", &self.state)
            .field("disabled", &self.disabled)
            .finish()
    }
}
