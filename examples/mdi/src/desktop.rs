//! The desktop: the canvas windows float on, the toolbar and Window menu above
//! it, and the taskbar the minimized ones collapse into.
//!
//! ## Two layers, not one
//!
//! The application mounts **two** [`overlay_layer`]s, nested:
//!
//! ```text
//! overlay_layer                     <- the app's own layer: the Window menu
//!   column
//!     toolbar
//!     expanded(overlay_layer)       <- the desktop: one overlay per window
//!       DesktopCanvas
//!     taskbar
//! ```
//!
//! The inner one exists because an overlay is placed in **layer-local**
//! coordinates: making the desktop its own layer is what lets a window's
//! rectangle be stored in desktop coordinates — `(0, 0)` is the top-left of the
//! desktop, not of the window — and what stops a window from being dragged over
//! the toolbar, since the overlay system already clamps a panel to its layer.
//!
//! The outer one exists because the menu system anchors its panels with the
//! trigger's **global** rect ([`silka_widgets::menu()`]), which only lines up in
//! a layer that starts at the window's origin. Two layers is the honest answer;
//! it is also the first time anything in this repository has nested them.

use silka_core::access::{AccessNode, AccessRole};
use silka_core::input::{Event, EventCtx, KeyCode, Modifiers, NamedKey};
use silka_core::scheduler::Dirty;
use silka_core::signals::{Key, Signal};
use silka_core::tree::{
    BoxConstraints, CrossAlign, LayoutCtx, MainAlign, NodeId, PaintCtx, RenderNode, RenderTree,
};
use silka_core::view::{column, expanded, row, stack, Builder, View, ViewNode};
use silka_paint::{Insets, Point, Quad, Size};
use silka_theme::{ColorToken, FontToken, SpaceToken, Theme};
use silka_widgets::menu::{cmd, item, menu, separator, MenuEntry, MenuState};
use silka_widgets::overlay::overlay_layer;
use silka_widgets::{button, divider, spacer, text, ButtonVariant};

use crate::frame::internal_frame;
use crate::model::{FrameId, Mdi};

/// The a11y name of the desktop canvas.
pub const DESKTOP: &str = "Desktop";
/// The a11y name (and text) of the Window menu.
pub const WINDOW_MENU: &str = "Window";
/// The a11y name (and text) of the button that opens another window.
pub const NEW_WINDOW: &str = "New Window";
/// The a11y name of the taskbar.
pub const TASKBAR: &str = "Taskbar";

/// The id of the "cascade" row in the Window menu.
pub const CASCADE: &str = "window.cascade";
/// The id of the "tile" row.
pub const TILE: &str = "window.tile";
/// The id of the "minimize all" row.
pub const MINIMIZE_ALL: &str = "window.minimize_all";

/// The Window-menu row id that activates window `id`.
pub fn activate_id(id: FrameId) -> String {
    format!("window.activate.{id}")
}

/// The window id a row produced by [`activate_id`] refers to.
pub fn activated(row: &str) -> Option<FrameId> {
    row.strip_prefix("window.activate.")?.parse().ok()
}

/// The a11y name of a taskbar button.
pub fn taskbar_label(title: &str) -> String {
    format!("Restore {title} from taskbar")
}

// ---------------------------------------------------------------------------
// The canvas
// ---------------------------------------------------------------------------

/// The desktop surface itself: a sunken background, and the one node that knows
/// how big the desktop is.
///
/// Its size is read back out by [`sync`] after layout, because that is the only
/// moment it exists. A window's rectangle has to be clamped against it — and
/// layout is the only place the number is known, while clamping happens in an
/// event handler that runs long before the next layout.
pub struct DesktopCanvas {
    /// The size from the last layout.
    size: Size,
}

impl DesktopCanvas {
    /// The desktop's size in points.
    pub fn size(&self) -> Size {
        self.size
    }
}

impl RenderNode for DesktopCanvas {
    fn type_name(&self) -> &'static str {
        "DesktopCanvas"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let biggest = constraints.biggest();
        self.size = constraints.constrain(Size::new(
            if biggest.width.is_finite() {
                biggest.width
            } else {
                0.0
            },
            if biggest.height.is_finite() {
                biggest.height
            } else {
                0.0
            },
        ));
        self.size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        // One quad on the sunken token: the desktop is the floor windows sit
        // on, so it has to read as *below* every surface around it.
        let t = silka_widgets::active_theme();
        ctx.quad(Quad::new(ctx.local_bounds()).background(t.color.surface_sunken));
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Group;
        node.label = Some(DESKTOP.to_string());
    }
}

impl core::fmt::Debug for DesktopCanvas {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DesktopCanvas")
            .field("size", &self.size)
            .finish()
    }
}

/// The props of [`DesktopCanvas`] — it has none; its whole state is its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CanvasProps;

impl ViewNode for CanvasProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(DesktopCanvas { size: Size::ZERO })
    }

    fn update(&self, _node: &mut dyn RenderNode) -> Dirty {
        Dirty::NONE
    }
}

/// Publish the desktop's size from this frame's finished layout.
///
/// The same seam `silka_widgets::split_view` uses for its track length: an
/// `advance`-time pass that reads what only a finished layout knows and writes
/// it where the event handlers can reach it. `set_desktop` returns false when
/// nothing changed, which is what keeps this from writing a signal — and
/// rebuilding the whole desktop — every single frame.
pub fn sync(tree: &RenderTree, state: Signal<Mdi>) -> Dirty {
    fn find(tree: &RenderTree, id: NodeId) -> Option<Size> {
        if let Some(c) = tree
            .render(id)
            .and_then(|n| n.downcast_ref::<DesktopCanvas>())
        {
            return Some(c.size());
        }
        tree.children(id).iter().find_map(|c| find(tree, *c))
    }
    let Some(size) = find(tree, tree.root()) else {
        return Dirty::NONE;
    };
    if state.peek_with(|m| m.desktop()) == size {
        return Dirty::NONE;
    }
    state.update(|m| m.set_desktop(size));
    Dirty::LAYOUT | Dirty::PAINT
}

// ---------------------------------------------------------------------------
// Desktop-wide keyboard commands
// ---------------------------------------------------------------------------

/// The node that owns the desktop's keyboard commands.
///
/// It sits above everything, which is exactly what makes it work: key events
/// travel from the focused node **up** to the root, so whatever a window's
/// controls do not consume arrives here. Ctrl+Tab is safe to claim because the
/// router only turns *bare* Tab and Shift+Tab into focus navigation — modified
/// Tab is left to the application on purpose.
pub struct MdiKeys {
    /// Where the commands land.
    state: Option<Signal<Mdi>>,
}

impl MdiKeys {
    fn command(&self, code: &KeyCode, modifiers: Modifiers) -> bool {
        let Some(state) = self.state else {
            return false;
        };
        // Ctrl+Tab cycles, the way every tabbed and every MDI application does.
        if code.is(NamedKey::Tab) && modifiers.contains(Modifiers::CONTROL) {
            let forward = !modifiers.contains(Modifiers::SHIFT);
            state.update(|m| m.cycle(forward));
            return true;
        }
        if !modifiers.contains(Modifiers::COMMAND) {
            return false;
        }
        match code {
            KeyCode::Character('w') => {
                state.update(|m| {
                    if let Some(id) = m.active() {
                        m.close(id);
                    }
                });
                true
            }
            KeyCode::Character('m') => {
                state.update(|m| {
                    if let Some(id) = m.active() {
                        m.minimize(id);
                    }
                });
                true
            }
            _ => false,
        }
    }
}

impl RenderNode for MdiKeys {
    fn type_name(&self) -> &'static str {
        "MdiKeys"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        size
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Key(k) = event else { return };
        if k.is_pressed() && self.command(&k.code, k.modifiers) {
            ctx.request_layout();
            ctx.handled();
        }
    }
}

impl core::fmt::Debug for MdiKeys {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("MdiKeys")
    }
}

/// The props of [`MdiKeys`].
#[derive(Debug, Clone, PartialEq)]
pub struct KeysProps {
    state: Option<Signal<Mdi>>,
}

impl ViewNode for KeysProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(MdiKeys { state: self.state })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<MdiKeys>()
            .expect("same view type means same render node type");
        n.state = self.state;
        Dirty::NONE
    }
}

/// Wrap `child` in the desktop's keyboard commands.
pub fn with_commands(state: Signal<Mdi>, child: impl Into<View>) -> View {
    Builder::new(KeysProps { state: Some(state) })
        .child(child)
        .into()
}

// ---------------------------------------------------------------------------
// The chrome
// ---------------------------------------------------------------------------

/// The rows of the Window menu: the arrangements first, then every open window.
///
/// The list of windows in a menu is the one piece of MDI that macOS itself
/// keeps — the Window menu of any Mac application is exactly this list — which
/// makes it the right place to prove that a menu can be driven by application
/// state rather than by a fixed literal.
pub fn window_entries(m: &Mdi) -> Vec<MenuEntry> {
    let mut entries = vec![
        item(CASCADE, "Cascade").enabled(!m.is_empty()).into(),
        item(TILE, "Tile").enabled(!m.is_empty()).into(),
        item(MINIMIZE_ALL, "Minimize All")
            .shortcut(cmd(KeyCode::Character('m')))
            .enabled(!m.is_empty())
            .into(),
    ];
    if !m.is_empty() {
        entries.push(separator());
    }
    let active = m.active();
    // Front to back, the order a window chooser is read in.
    for f in m.frames().iter().rev() {
        let label = if f.is_visible() {
            f.title.clone()
        } else {
            format!("{} (minimized)", f.title)
        };
        entries.push(
            item(activate_id(f.id), label)
                .radio(Some(f.id) == active)
                .into(),
        );
    }
    entries
}

/// What the Window menu does with the row that was chosen.
///
/// Pure and separate from the view so the whole menu can be exercised without
/// opening one: the tests call this with a row id and assert on the model.
pub fn activate(m: &mut Mdi, row: &str) {
    match row {
        CASCADE => m.cascade(),
        TILE => m.tile(),
        MINIMIZE_ALL => m.minimize_all(),
        other => {
            if let Some(id) = activated(other) {
                m.restore(id);
            }
        }
    }
}

/// The toolbar: a button that opens windows, and the Window menu.
///
/// Returns the bar plus the menu's overlay panels — the panels belong to the
/// application's outer layer, because that order is the stacking order for the
/// whole window.
pub fn toolbar(
    t: &Theme,
    state: Signal<Mdi>,
    menu_state: Signal<MenuState>,
) -> (View, Vec<silka_widgets::overlay::OverlayBuilder>) {
    let m = state.get();
    let window_menu = menu(window_entries(&m))
        .label(WINDOW_MENU)
        .key("window-menu")
        .bind(menu_state)
        .on_activate(move |row| {
            let row = row.to_string();
            state.update(|m| activate(m, &row));
        });

    let bar = row([
        View::from(
            button(NEW_WINDOW)
                .key("new-window")
                .variant(ButtonVariant::Primary)
                .on_press(move || {
                    state.update(|m| {
                        let n = m.len() + 1;
                        m.open(
                            format!("Window {n}"),
                            "A window opened at runtime. Everything about it — \
                             chrome, drag, resize, focus — comes from this \
                             example, not from the widget catalogue.",
                        );
                    });
                }),
        ),
        window_menu.trigger(WINDOW_MENU),
        View::from(expanded(spacer())),
        View::from(
            text(status(&m))
                .font(FontToken::Footnote)
                .color(t.color.secondary_label)
                .single_line(),
        ),
    ])
    .cross(CrossAlign::Center)
    .spacing(t.space(2.0))
    .padding(Insets::symmetric(t.space(3.0), t.space(2.0)))
    .bg(ColorToken::Surface)
    .into();

    (bar, window_menu.overlays())
}

/// The line at the right of the toolbar.
pub fn status(m: &Mdi) -> String {
    let minimized = m.minimized().len();
    let front = m
        .active()
        .and_then(|id| m.get(id))
        .map(|f| f.title.clone())
        .unwrap_or_else(|| "nothing".to_string());
    format!("{} open · {minimized} minimized · front: {front}", m.len())
}

/// The taskbar: one button per minimized window.
pub fn taskbar(t: &Theme, state: Signal<Mdi>) -> View {
    let m = state.get();
    let mut items: Vec<View> = Vec::new();
    for id in m.minimized() {
        let Some(f) = m.get(id) else { continue };
        items.push(
            button(taskbar_label(&f.title))
                .key(Key::num(i64::from(id)))
                .variant(ButtonVariant::Ghost)
                .on_press(move || state.update(|m| m.restore(id)))
                .into(),
        );
    }
    if items.is_empty() {
        items.push(
            text("No minimized windows")
                .font(FontToken::Footnote)
                .color(t.color.tertiary_label)
                .single_line()
                .into(),
        );
    }
    column([
        View::from(divider()),
        // Wrapped in a `stack` purely to give the row a name: `row`/`column`
        // carry no a11y label of their own, so a bar of buttons cannot
        // announce itself as a toolbar without a box around it.
        stack([row(items)
            .cross(CrossAlign::Center)
            .main(MainAlign::Start)
            .spacing(t.space(2.0))
            .padding(Insets::symmetric(t.space(3.0), t.space(1.0)))])
        .label(TASKBAR)
        .role(AccessRole::Toolbar)
        .into(),
    ])
    .cross(CrossAlign::Stretch)
    .bg(ColorToken::Surface)
    .into()
}

/// The desktop layer: the canvas, with one overlay per window in z-order.
///
/// The order the overlays are pushed in **is** the z-order — that is the whole
/// of "bring to front" in this application, and not one window computes its own
/// stacking. Each is keyed by its window id, so reordering *moves* nodes
/// instead of destroying and rebuilding them: a window raised in the middle of
/// a drag keeps the pointer capture it grabbed a moment ago.
pub fn desktop(t: &Theme, state: Signal<Mdi>) -> View {
    let m = state.get();
    let mut layer = overlay_layer(Builder::new(CanvasProps));
    for f in m.frames() {
        layer = layer.overlay(internal_frame(
            t,
            state,
            f,
            m.is_active(f.id),
            m.is_dragging(f.id),
        ));
    }
    layer.into()
}

/// Toolbar, desktop, taskbar — the whole application surface below the menus.
pub fn surface(
    t: &Theme,
    state: Signal<Mdi>,
    menu_state: Signal<MenuState>,
) -> (View, Vec<silka_widgets::overlay::OverlayBuilder>) {
    let (bar, panels) = toolbar(t, state, menu_state);
    let body = column([
        bar,
        divider().into(),
        View::from(expanded(desktop(t, state))),
        taskbar(t, state),
    ])
    .cross(CrossAlign::Stretch)
    .bg(ColorToken::Background)
    .p(SpaceToken::None)
    .into();
    (body, panels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_id_survives_the_round_trip() {
        assert_eq!(activated(&activate_id(7)), Some(7));
        assert_eq!(activated(CASCADE), None);
        assert_eq!(activated("window.activate.nope"), None);
    }

    #[test]
    fn the_window_menu_lists_every_window_front_to_back() {
        let mut m = Mdi::new();
        m.set_desktop(Size::new(900.0, 600.0));
        m.open("A", "");
        m.open("B", "");
        m.minimize(1);

        let entries = window_entries(&m);
        let labels: Vec<&str> = entries
            .iter()
            .filter_map(|e| e.item())
            .map(|i| i.label())
            .collect();
        assert_eq!(
            labels,
            vec!["Cascade", "Tile", "Minimize All", "B", "A (minimized)"]
        );

        // Exactly one row carries the "this is the front window" mark.
        let marked: Vec<&str> = entries
            .iter()
            .filter_map(|e| e.item())
            .filter(|i| i.is_checked())
            .map(|i| i.label())
            .collect();
        assert_eq!(marked, vec!["B"]);
    }

    #[test]
    fn choosing_a_window_row_restores_and_raises_it() {
        let mut m = Mdi::new();
        m.set_desktop(Size::new(900.0, 600.0));
        m.open("A", "");
        m.open("B", "");
        m.minimize(1);
        assert_eq!(m.active(), Some(2));

        activate(&mut m, &activate_id(1));
        assert_eq!(m.active(), Some(1), "restored from the menu and raised");
    }

    #[test]
    fn the_arrangement_rows_do_what_they_say() {
        let mut m = Mdi::new();
        m.set_desktop(Size::new(900.0, 600.0));
        m.open("A", "");
        m.open("B", "");

        activate(&mut m, TILE);
        assert_ne!(m.get(1).unwrap().rect, m.get(2).unwrap().rect);

        activate(&mut m, MINIMIZE_ALL);
        assert_eq!(m.active(), None);
        assert_eq!(m.minimized().len(), 2);
    }

    #[test]
    fn the_status_line_names_the_front_window() {
        let mut m = Mdi::new();
        m.set_desktop(Size::new(900.0, 600.0));
        assert!(status(&m).contains("front: nothing"));
        m.open("Ledger", "");
        assert!(status(&m).contains("front: Ledger"));
        m.minimize(1);
        assert!(status(&m).contains("1 minimized"));
    }
}
