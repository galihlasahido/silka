//! # silka-widgets
//!
//! The component catalogue (see `KOMPONEN.md`) and at the same time the
//! framework's **public API surface**. This is the contract that has to be
//! frozen early; internals may change at will (REKOMENDASI §4 "Kestabilan").
//!
//! Two BINDING rules for the shape of the API:
//!
//! 1. **Dart style** (§2.5) — constructor functions plus method chaining,
//!    nesting identical to Flutter; optional properties move into the method
//!    chain. An `rsx!`-style DSL macro is rejected as the foundation.
//! 2. **Tailwind-style utility styling as a method chain** (§2.6) — no CSS,
//!    no parser, no cascade. Values always resolve through `silka-theme`
//!    tokens, and interactive utilities (`hover`/`pressed`/`focused`)
//!    transition on a spring instead of jumping.
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::{column, View};
//! # use silka_theme::{Appearance, Theme};
//! use silka_widgets::{button, text, Fonts};
//!
//! # let rt = Runtime::new();
//! # let count = rt.signal(0i32);
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! column([
//!     View::from(text(&fonts, format!("Nilai: {}", count.get())).color(t.color.label)),
//!     View::from(button(&fonts, &t, "Tambah").on_press(move || count.set(count.get() + 1))),
//! ])
//! .spacing(t.space(3.0));
//! ```
//!
//! ## What already exists
//!
//! - [`mod@text`] (Tier 0) — a text leaf that **measures itself** through
//!   `silka-text` and draws glyphs from the atlas; wrapping follows the width
//!   handed down by the box constraints, and its content is the a11y node name.
//! - [`mod@button`] (Tier 2) — a complete control built on tokens:
//!   primary/secondary/ghost/destructive/link variants, hover/press/focus/
//!   disabled/loading states that **all transition on springs**, a focus ring
//!   that grows, Space/Enter, an AccessKit node, and a hit target ≥ 44pt.
//! - [`mod@checkbox`] (Tier 2) — a **tri-state** checkbox (indeterminate
//!   included): the check mark is genuinely *drawn* by a spring
//!   ([`check_dots`]), the label is clickable too and doubles as the a11y
//!   name, Space activates, and the hit target is ≥ 44pt even though the box
//!   is 16pt.
//! - [`mod@switch`] / [`toggle`] (Tier 2) — an on/off switch you can **drag**, not
//!   merely click: the thumb tracks the finger 1:1, the finger's velocity is
//!   handed to the spring on release (handoff §3.5), the track color crosses
//!   over exactly at the midpoint, Space plus left/right arrows, an AccessKit
//!   node carrying the on/off state, and a hit target ≥ 44pt even though the
//!   track is 32pt/24pt.
//! - [`mod@slider`] / [`range_slider`] (Tier 2) — value sliders: dragging that
//!   sticks to the finger, click-on-track, **snapping to steps**, full
//!   keyboard support (arrows/Home/End/PageUp), a two-thumb range variant, an
//!   AccessKit node with the value plus increment/decrement actions, and a
//!   ≥ 44pt touch band around a track only 4pt thick.
//! - [`tabs`](mod@tabs) (Tier 3) — a row of tabs with three variants
//!   (segmented/underline/enclosed) over **one** engine: an indicator that
//!   glides on a retargetable spring, a single Tab stop for the whole row
//!   (arrows/Home/End inside it, skipping disabled tabs, mirrored in RTL), a
//!   focus ring that glides along, and AccessKit `TabList`/`Tab` nodes
//!   complete with the selected state.
//! - [`mod@select`] (Tier 2) — a macOS pop-up button / shadcn Select: a popup that
//!   **rides the overlay system** (anchored to the trigger, auto-flipping at
//!   the screen edge), full keyboard support on the trigger, which keeps focus
//!   (Space/Enter/arrows/Home/End/Esc), plus native-menu-style **typeahead**,
//!   long lists with a window that follows the highlight, an AccessKit
//!   `Button` carrying a value plus marked `Menu`/`MenuItem` nodes, and a hit
//!   target ≥ 44pt on both the box and every row.
//! - [`scroll_view`](mod@scroll_view) (Tier 1) — a scrolling container with
//!   **macOS-style rubber banding**, a bounce that inherits the velocity of
//!   the OS inertia tail (momentum stays the OS's job, INTEGRASI-NATIVE §3),
//!   overlay scrollbars that widen on hover and fade out on their own when
//!   idle, thumb dragging, full keyboard navigation plus a focus ring,
//!   `scroll_to`/`scroll_into_view`, and an AccessKit `SCROLL` action that
//!   genuinely works.
//! - [`list`](mod@list) (Tier 1) — a **virtualized** list: `item` is called
//!   only for rows that are actually visible, so a hundred thousand rows still
//!   come out as a dozen-odd nodes. It lives **inside**
//!   [`scroll_view`](mod@scroll_view) — momentum, rubber banding, and
//!   scrollbars are not written twice — and adds what genuinely belongs to a
//!   list: sticky headers, a selection whose highlight *glides* on a spring,
//!   ↑/↓/Page/Home/End that move the selection while scrolling its row into
//!   view, and AccessKit `List`/`ListItem` nodes along with their selected
//!   state.
//! - [`table`](mod@table) (Tier 5) — a **virtualized** table that rides the
//!   `list` infrastructure instead of growing a second one
//!   (`KOMPONEN.md` ordering rule #4): its row window is computed by the same
//!   [`ListMetrics`], its scrolling and rubber banding belong to
//!   [`scroll_view`](mod@scroll_view), and the seam between the two is the
//!   same [`list::sync_virtual`]. What it adds is precisely what a list does
//!   not have: per-column sorting, column resize and reorder by dragging in
//!   the header, anchored multiple selection (⇧ extends, ⌘ picks, ⌘A takes
//!   everything) stored as **ranges** so a hundred thousand selected rows are
//!   still a single entry, keyboard navigation between **cells** with a focus
//!   ring around the active cell, custom cells (any widget inside a cell),
//!   sticky headers, an empty state, and AccessKit `Table`/`Row`/`Cell` nodes.
//! - [`mod@text_field`] (Tier 2, **the hardest component in the whole catalogue**)
//!   — a single-line text field: caret and selection **per grapheme cluster**
//!   (UAX #29), double-click by word, triple-click for the whole content,
//!   drag-select, undo/redo that coalesces consecutive typing, horizontal
//!   scrolling that keeps the caret visible, and **IME preedit rendered inline
//!   with an underline** — with the normal key path held back during
//!   composition, so the application never receives half-finished letters
//!   (§3.3, §3.8). Its editing model lives in [`silka_text::edit`], its
//!   geometry in [`silka_text::TextLayout`].
//! - [`advance`] (infrastructure) — one tick for the whole tree: this is where
//!   every widget's springs are advanced, once per frame, and where the answer
//!   to "is anything still moving?" comes from.
//! - [`Fonts`] — the shared handle to the application's text engine, one atlas
//!   for the whole application.
//! - [`dialog`](mod@dialog) / [`alert`] (Tier 4) — a backdropped modal on top
//!   of [`overlay`](mod@overlay): a title, a message, and a button row whose
//!   **order follows OS convention** ([`ButtonOrder`]), with Return running
//!   the default button and Esc running the cancel action.
//! - [`overlay`](mod@overlay) (Tier 4, **infrastructure**) — a layer above the
//!   content, anchored placement with auto-flip at the edges, a backdrop,
//!   dismissal (outside click/Esc), and retargetable spring transitions. Built
//!   exactly once, precisely as `KOMPONEN.md` rule #3 demands: dialog, sheet,
//!   popover, tooltip, menu, and toast will all **ride** this module — each
//!   one merely picks a [`Placement`] and a [`Barrier`], and not one of them
//!   may compute its own position.
//!
//! Technical debt we are aware of and deliberately do not hide: `Fonts` is
//! still passed explicitly to every constructor because there is no ambient
//! context yet for application-level dependencies, and "scale-on-press" is
//! drawn as the background box deflating (the paint layer has no transform
//! command yet, §3.2), so the label inside it does not shrink along with it.
//! For overlays, what is missing is **automatic focus** on a freshly opened
//! panel: [`overlay::topmost`] provides the node, but there is no "just
//! opened" hook in the frame cycle that calls it.
//!
//! The order of work follows the tiers in `KOMPONEN.md`: Tier 0 (primitives)
//! and Tier 1 (layout) until they are genuinely solid first, `text_field`
//! started earliest in Tier 2 because it forces the text/IME/a11y stack to
//! mature, and the overlay system built once for
//! dialog/popover/tooltip/menu/toast.
//!
//! **Definition of Done for every component** (KOMPONEN.md): correct in both
//! presets, every interactive state transitions on a spring, full keyboard
//! navigation plus a focus ring, an **AccessKit node** (role/name/actions),
//! dark mode, a minimum 44pt hit target, and respect for reduced-motion.
//!
//! Code in this crate **must not touch wgpu types** — only `silka-paint`
//! drawing commands (§3.2, §5 failure mode #7).

#![warn(missing_docs)]

pub mod button;
pub mod checkbox;
pub mod dialog;
pub mod fonts;
pub mod list;
pub mod motion;
pub mod overlay;
pub mod scroll_view;
pub mod select;
pub mod slider;
pub mod switch;
pub mod table;
pub mod tabs;
pub mod text;
pub mod text_field;

pub use button::{
    button, button_variant, Button, ButtonBox, ButtonProps, ButtonState, ButtonStyle,
    ButtonVariant, MIN_HIT_TARGET,
};
pub use checkbox::{
    check_dots, checkbox, checkbox_only, dash_rect, ChangeCallback, CheckState, Checkbox,
    CheckboxNode, CheckboxProps, CheckboxStyle,
};
pub use dialog::{
    action, activate_default, alert, dialog, ActionKind, ButtonOrder, DialogAction, DialogBuilder,
    DialogPanel, DialogPanelProps, DIALOG_WIDTH_STEPS,
};
pub use fonts::Fonts;
pub use list::{
    list, use_list_state, ListBody, ListBuilder, ListMetrics, ListRange, ListRowBox, ListScroll,
    ListState, ListStyle, RowAction, Virtualized,
};
pub use motion::{advance, is_animating, settle};
pub use overlay::{overlay, overlay_layer, Anchor, Barrier, Dismiss, Placement, Side};
pub use scroll_view::{
    scroll_view, ScrollBar, ScrollBuilder, ScrollProps, ScrollView, Scrollbar, ScrollbarStyle,
    Thumb,
};
pub use select::{
    select, Select, SelectHandler, SelectIntent, SelectOption, SelectOptionProps,
    SelectOptionStyle, SelectState, SelectTrigger, SelectTriggerProps, SelectTriggerStyle,
};
pub use slider::{
    range_slider, slider, Slider, SliderBuilder, SliderGeometry, SliderProps, SliderStyle,
};
pub use switch::{
    switch, switch_only, toggle, StateColors, Switch, SwitchCallback, SwitchNode, SwitchProps,
    SwitchStyle,
};
pub use table::{
    col, table, use_table_state, CellAlign, Column, ColumnLayout, ColumnWidth, HeaderStyle,
    Selection, SelectionMode, SortBy, SortDirection, TableBody, TableBuilder, TableCellBox,
    TableHeaderBox, TableRowBox, TableState, TableStyle,
};
pub use tabs::{tab, tabs, Tab, Tabs, TabsStyle, TabsVariant};
pub use text::{text, Text, TextBox, TextProps};
pub use text_field::{text_field, TextCallback, TextField, TextFieldBox, TextFieldProps};
