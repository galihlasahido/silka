//! Event routing: from one raw event to the nodes that handle it.
//!
//! Four rules determine everything in this module:
//!
//! 1. **Pointers follow geometry** — the route is the hit-test path
//!    ([`super::hit_test`]), from the innermost node to the root, stopping at
//!    the first node that declares the event handled.
//! 2. **The keyboard follows focus** — the route is the focus path up to the
//!    root, so window-level shortcuts still get their turn after a widget
//!    declines.
//! 3. **A press captures the pointer** — once a node presses a button and asks
//!    for capture, every movement until the button is released goes to that
//!    node even when the cursor has left its box. Without this, a slider
//!    dragged quickly would come loose halfway.
//! 4. **The IME belongs to whoever has focus** — preedit/commit are delivered
//!    only to the focused node, and the `set_ime_cursor_area` request flows
//!    back to the platform through [`Response`] (REKOMENDASI §3.8).
//!
//! A node may **not** change the tree structure from inside an event handler:
//! all it can do is change itself and leave requests behind through
//! [`EventCtx`]. Structure only changes through the view diff (§2) — that is
//! what keeps the arena consistent even when events arrive mid-frame.

use std::collections::HashMap;
use std::time::Duration;

use silka_paint::{Point, Rect, Size};

use crate::scheduler::Dirty;
use crate::tree::{NodeId, RenderTree};

use super::event::{
    Event, FocusEvent, ImeEvent, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent,
    PointerId, PointerPhase, ScrollEvent,
};
use super::focus::{FocusChange, FocusDirection, FocusManager};
use super::hit::{hit_test, HitEntry, HitTestResult};
use super::velocity::{Velocity, VelocityTracker};

// ---------------------------------------------------------------------------
// Cursor
// ---------------------------------------------------------------------------

/// The cursor shape a node asks for.
///
/// Our own vocabulary, mapped to `winit::window::CursorIcon` in
/// `silka-platform` — the same reason as for the whole input module: widget
/// code does not touch third-party types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum CursorIcon {
    /// The ordinary arrow.
    #[default]
    Default,
    /// A pointing hand (links, web-style buttons).
    Pointer,
    /// A text caret.
    Text,
    /// Busy.
    Wait,
    /// Grabbable (scroll pan, drag handle).
    Grab,
    /// Currently grabbed.
    Grabbing,
    /// Horizontal resize (a vertical split view).
    ResizeHorizontal,
    /// Vertical resize.
    ResizeVertical,
    /// The action is not allowed.
    NotAllowed,
}

// ---------------------------------------------------------------------------
// IME requests
// ---------------------------------------------------------------------------

/// An IME-related request to the platform shell.
///
/// `silka-platform` translates it into `set_ime_allowed` +
/// `set_ime_cursor_area` — the two winit calls that decide whether the CJK
/// candidate window appears in the right place (REKOMENDASI §3.8).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImeRequest {
    /// Enable the IME and put the candidate area at `area` (logical points,
    /// global).
    Enable {
        /// The caret/preedit box the candidate window anchors to.
        area: Rect,
    },
    /// The IME is already on; only its area moved (the caret moved).
    Update {
        /// The new caret box.
        area: Rect,
    },
    /// Turn the IME off — there is nothing left that could receive text.
    Disable,
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// The result of one dispatch: what the shell has to do afterwards.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Response {
    /// Why a next frame is needed — wired straight into
    /// [`crate::scheduler::FrameScheduler::request`]. Empty = nothing needs
    /// drawing, and the window stays genuinely idle (§3.5).
    pub dirty: Dirty,
    /// True when some node claimed this event as its own.
    pub handled: bool,
    /// The focus move that happened.
    pub focus: FocusChange,
    /// An IME request for the shell.
    pub ime: Option<ImeRequest>,
    /// A new cursor shape (only filled in when it changes).
    pub cursor: Option<CursorIcon>,
}

impl Response {
    /// True when this dispatch had no effect at all.
    pub fn is_noop(&self) -> bool {
        self.dirty.is_empty()
            && !self.handled
            && !self.focus.changed()
            && self.ime.is_none()
            && self.cursor.is_none()
    }
}

// ---------------------------------------------------------------------------
// EventCtx
// ---------------------------------------------------------------------------

/// What nodes leave behind through [`EventCtx`], gathered across one dispatch
/// and applied once at the end.
#[derive(Debug, Default)]
struct Sink {
    dirty: Dirty,
    /// `Some(Some(n))` = focus requested for n, `Some(None)` = drop focus.
    focus: Option<Option<NodeId>>,
    /// `Some(Some(n))` = capture the pointer for n, `Some(None)` = release it.
    capture: Option<Option<NodeId>>,
    ime: Option<(NodeId, Option<Rect>)>,
}

/// Limited access to the outside world while a node handles an event.
///
/// It deliberately does **not** carry a `&mut RenderTree`: a node may only
/// change itself (through `&mut self`) and leave requests here. As a
/// consequence the tree structure cannot possibly change mid-dispatch, and
/// there is no re-entrancy to guard against.
pub struct EventCtx<'a> {
    node: NodeId,
    local: Point,
    size: Size,
    bounds: Rect,
    focused: bool,
    handled: &'a mut bool,
    sink: &'a mut Sink,
}

impl EventCtx<'_> {
    /// The node currently handling the event.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// The event position in the node's local coordinates (logical points).
    ///
    /// For events without a position (keyboard, IME, focus) this is
    /// [`Point::ZERO`].
    pub fn local(&self) -> Point {
        self.local
    }

    /// The node's size from the last layout.
    pub fn size(&self) -> Size {
        self.size
    }

    /// The node's global box — used to compute the caret area for the IME.
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// True when this node currently holds keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Declare the event handled: delivery to ancestors stops here.
    pub fn handled(&mut self) {
        *self.handled = true;
    }

    /// True when a deeper node already handled the event.
    pub fn is_handled(&self) -> bool {
        *self.handled
    }

    /// Ask for the node to be repainted (hover, pressed, focus ring).
    pub fn request_paint(&mut self) {
        self.sink.dirty |= Dirty::PAINT;
    }

    /// Ask for a relayout (e.g. the scroll position changed).
    pub fn request_layout(&mut self) {
        self.sink.dirty |= Dirty::LAYOUT | Dirty::PAINT;
    }

    /// Ask for a next frame because an animation is running (a spring).
    pub fn request_animation(&mut self) {
        self.sink.dirty |= Dirty::ANIMATION;
    }

    /// Ask for keyboard focus to move to this node.
    pub fn request_focus(&mut self) {
        self.sink.focus = Some(Some(self.node));
    }

    /// Release focus from whoever is holding it.
    pub fn release_focus(&mut self) {
        self.sink.focus = Some(None);
    }

    /// Capture the pointer: every movement until the button is released comes
    /// here.
    pub fn capture_pointer(&mut self) {
        self.sink.capture = Some(Some(self.node));
    }

    /// Release the pointer capture.
    pub fn release_pointer(&mut self) {
        self.sink.capture = Some(None);
    }

    /// Ask for the IME to be enabled with `area` as the candidate area (global
    /// coordinates).
    ///
    /// Called by text widgets when they gain focus and every time the caret
    /// moves.
    pub fn request_ime(&mut self, area: Rect) {
        self.sink.ime = Some((self.node, Some(area)));
    }

    /// Turn the IME off (a text widget lost focus).
    pub fn disable_ime(&mut self) {
        self.sink.ime = Some((self.node, None));
    }
}

// ---------------------------------------------------------------------------
// Multi-click configuration
// ---------------------------------------------------------------------------

/// The thresholds for consecutive clicks (double/triple).
///
/// The numbers belong to the framework, not the platform: the three operating
/// systems report them in different ways (and Wayland not at all), while users
/// expect the same behaviour everywhere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClickConfig {
    /// The maximum gap between clicks.
    pub interval: Duration,
    /// The maximum drift between clicks, in logical points.
    pub distance: f32,
}

impl Default for ClickConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(500),
            distance: 4.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Per-pointer state
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct PointerState {
    position: Point,
    /// The path of nodes currently hovered, innermost first.
    hover: Vec<NodeId>,
    capture: Option<NodeId>,
    velocity: VelocityTracker,
    last_click: Option<(PointerButton, Point, Duration)>,
    click_count: u32,
}

// ---------------------------------------------------------------------------
// InputRouter
// ---------------------------------------------------------------------------

/// The event router for one render tree (one window).
///
/// It stores what genuinely has to be remembered between events: the last
/// modifiers, the buttons held, the hover path, capture, per-pointer velocity,
/// focus, and the IME state. Anything that can be re-read from the tree is
/// **not** stored.
#[derive(Debug, Default)]
pub struct InputRouter {
    modifiers: Modifiers,
    pointers: HashMap<PointerId, PointerState>,
    focus: FocusManager,
    click: ClickConfig,
    cursor: CursorIcon,
    /// The node that currently owns the IME session, plus its last caret area.
    ime: Option<(NodeId, Rect)>,
}

impl InputRouter {
    /// A new router: no focus, no hover, no capture.
    pub fn new() -> Self {
        Self::default()
    }

    /// The multi-click thresholds.
    pub fn click_config(&self) -> ClickConfig {
        self.click
    }

    /// Change the multi-click thresholds (e.g. to follow an OS setting).
    pub fn set_click_config(&mut self, config: ClickConfig) {
        self.click = config;
    }

    /// The last known keyboard modifiers.
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Update the modifiers without dispatching an event (winit reports them
    /// separately).
    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    /// The keyboard focus holder.
    pub fn focus(&self) -> &FocusManager {
        &self.focus
    }

    /// The cursor currently in effect.
    pub fn cursor(&self) -> CursorIcon {
        self.cursor
    }

    /// The node currently capturing pointer `id`.
    pub fn capture_of(&self, id: PointerId) -> Option<NodeId> {
        self.pointers.get(&id).and_then(|p| p.capture)
    }

    /// The hover path for pointer `id`, innermost first.
    pub fn hover_of(&self, id: PointerId) -> &[NodeId] {
        self.pointers
            .get(&id)
            .map(|p| p.hover.as_slice())
            .unwrap_or(&[])
    }

    /// The current velocity of pointer `id` — this is the value handed to a
    /// spring when the gesture is released (fling → spring, §3.5).
    pub fn velocity(&self, id: PointerId) -> Velocity {
        self.pointers
            .get(&id)
            .map(|p| p.velocity.velocity())
            .unwrap_or(Velocity::ZERO)
    }

    /// Focus a particular node from the outside (e.g. after a dialog opens).
    pub fn focus_node(&mut self, tree: &mut RenderTree, node: Option<NodeId>) -> Response {
        let mut out = Response::default();
        let change = self.focus.focus(tree, node);
        self.terapkan_fokus(tree, change, &mut out);
        out
    }

    /// Move focus one step (a programmatically triggered Tab / Shift+Tab).
    pub fn move_focus(&mut self, tree: &mut RenderTree, direction: FocusDirection) -> Response {
        let mut out = Response::default();
        let change = self.focus.move_focus(tree, direction);
        self.terapkan_fokus(tree, change, &mut out);
        out
    }

    /// Reconcile the input state with the tree after a view diff.
    ///
    /// Nodes can vanish at any moment; focus, capture, hover and IME sessions
    /// pointing at a grave must be cleaned up **before** the next event
    /// arrives — otherwise the keyboard goes completely dead and the IME
    /// candidate window hangs in the wrong place.
    pub fn sync(&mut self, tree: &mut RenderTree) -> Response {
        let mut out = Response::default();
        // A node that is **still alive** but stopped being focusable (e.g. a
        // button that was just disabled) is still told through `Focus::Lost`;
        // one that has vanished cannot be, and `kirim_satu` skips it quietly.
        let change = self.focus.prune(tree);
        self.terapkan_fokus(tree, change, &mut out);
        for state in self.pointers.values_mut() {
            if let Some(cap) = state.capture {
                if !tree.contains(cap) {
                    state.capture = None;
                }
            }
            state.hover.retain(|n| tree.contains(*n));
        }
        if let Some((owner, _)) = self.ime {
            if !tree.contains(owner) || !self.focus.is_focused(owner) {
                self.ime = None;
                out.ime = Some(ImeRequest::Disable);
            }
        }
        out
    }

    /// Route one event into the tree.
    pub fn dispatch(&mut self, tree: &mut RenderTree, event: &Event) -> Response {
        match event {
            Event::Pointer(e) => self.pointer(tree, e),
            Event::Scroll(e) => self.scroll(tree, e),
            Event::Key(e) => self.key(tree, e),
            Event::Ime(e) => self.ime_event(tree, e),
            // Focus events are born in the router, never injected from outside.
            Event::Focus(_) => Response::default(),
        }
    }

    // -- pointer ----------------------------------------------------------

    fn pointer(&mut self, tree: &mut RenderTree, event: &PointerEvent) -> Response {
        self.modifiers = event.modifiers;
        let mut out = Response::default();
        let click = self.click;

        // Movement history: the basis of the velocity tracker for the spring
        // handoff.
        {
            let state = self.pointers.entry(event.id).or_default();
            state.position = event.position;
            match event.phase {
                PointerPhase::Down => {
                    state.velocity.reset();
                    state.velocity.add(event.time, event.position);
                    state.click_count = hitung_klik(state, event, click);
                    state.last_click = event.button.map(|b| (b, event.position, event.time));
                }
                PointerPhase::Move | PointerPhase::Enter => {
                    state.velocity.add(event.time, event.position)
                }
                PointerPhase::Up => state.velocity.add(event.time, event.position),
                PointerPhase::Cancel | PointerPhase::Leave => state.velocity.reset(),
            }
        }

        // Hover is computed from geometry, not from capture — a button that is
        // pressed and then dragged away from really should stop looking
        // hovered.
        let hit = if event.phase == PointerPhase::Leave {
            HitTestResult::new()
        } else {
            hit_test(tree, event.position)
        };
        self.perbarui_hover(tree, event, &hit, &mut out);

        if event.phase == PointerPhase::Leave {
            return out;
        }

        let mut event = event.clone();
        event.click_count = self.pointers.get(&event.id).map_or(0, |s| s.click_count);

        let rute = match self.capture_of(event.id) {
            Some(node) if tree.contains(node) => rute_dari_node(tree, node, event.position),
            _ => hit.path().to_vec(),
        };

        let mut sink = Sink::default();
        let handled = self.kirim(tree, &rute, &Event::Pointer(event.clone()), &mut sink);
        out.handled = handled;

        // A release or a cancel always ends the capture, whatever the node
        // says — otherwise a pointer could stay stuck forever on a node that
        // forgot to let go.
        if matches!(event.phase, PointerPhase::Up | PointerPhase::Cancel)
            && sink.capture.is_none()
            && event.buttons.is_empty()
        {
            sink.capture = Some(None);
        }
        self.terapkan(tree, sink, Some(event.id), &mut out);

        // The cursor is asked for **after** the event reaches the node, not
        // before: a node whose cursor shape depends on where the pointer is
        // inside it (the column resize handle in `table`, later `split_view`)
        // only knows the answer once it has received that movement. Asking
        // first would mean the arrow cursor stays an arrow right on top of a
        // draggable handle — and the user never discovers it exists.
        self.perbarui_kursor(tree, event.id, &mut out);
        out
    }

    fn perbarui_hover(
        &mut self,
        tree: &mut RenderTree,
        event: &PointerEvent,
        hit: &HitTestResult,
        out: &mut Response,
    ) {
        let baru: Vec<NodeId> = hit.nodes().collect();
        let lama = std::mem::take(&mut self.pointers.entry(event.id).or_default().hover);
        if lama == baru {
            self.pointers.entry(event.id).or_default().hover = baru;
            return;
        }

        let mut sink = Sink::default();
        for node in lama.iter().filter(|n| !baru.contains(n)) {
            let mut e = event.clone();
            e.phase = PointerPhase::Leave;
            // The local coordinates still mean something even though the point
            // is now outside the node — a widget working out "which side did
            // it leave by" needs them.
            let origin = tree.global_offset(*node);
            let local = Point::new(e.position.x - origin.x, e.position.y - origin.y);
            self.kirim_satu(tree, *node, local, &Event::Pointer(e), &mut sink);
        }
        for entry in hit.path().iter().filter(|e| !lama.contains(&e.node)) {
            let mut e = event.clone();
            e.phase = PointerPhase::Enter;
            self.kirim_satu(tree, entry.node, entry.local, &Event::Pointer(e), &mut sink);
        }
        self.pointers.entry(event.id).or_default().hover = baru;
        self.terapkan(tree, sink, Some(event.id), out);
        self.perbarui_kursor(tree, event.id, out);
    }

    /// Re-ask the hover chain for the cursor shape, and report it when it
    /// changed.
    ///
    /// The cursor is **asked for** from the node, never cached in the router —
    /// so a node whose cursor shape depends on its own state (or on where the
    /// pointer is inside it) only has to update that state in `event`, and the
    /// answer here is already right within the same event.
    fn perbarui_kursor(&mut self, tree: &RenderTree, id: PointerId, out: &mut Response) {
        let kursor = self
            .hover_of(id)
            .iter()
            .find_map(|n| tree.render(*n).and_then(|r| r.cursor()))
            .unwrap_or_default();
        if kursor != self.cursor {
            self.cursor = kursor;
            out.cursor = Some(kursor);
        }
    }

    // -- scroll -----------------------------------------------------------

    fn scroll(&mut self, tree: &mut RenderTree, event: &ScrollEvent) -> Response {
        self.modifiers = event.modifiers;
        let mut out = Response::default();
        let rute = hit_test(tree, event.position).path().to_vec();
        let mut sink = Sink::default();
        out.handled = self.kirim(tree, &rute, &Event::Scroll(event.clone()), &mut sink);
        self.terapkan(tree, sink, None, &mut out);
        out
    }

    // -- keyboard ---------------------------------------------------------

    fn key(&mut self, tree: &mut RenderTree, event: &KeyEvent) -> Response {
        self.modifiers = event.modifiers;
        let mut out = Response::default();

        let rute: Vec<HitEntry> = self
            .focus
            .path(tree)
            .into_iter()
            .map(|node| HitEntry {
                node,
                local: Point::ZERO,
            })
            .collect();
        let rute = if rute.is_empty() {
            vec![HitEntry {
                node: tree.root(),
                local: Point::ZERO,
            }]
        } else {
            rute
        };

        let mut sink = Sink::default();
        out.handled = self.kirim(tree, &rute, &Event::Key(event.clone()), &mut sink);
        self.terapkan(tree, sink, None, &mut out);

        // Tab is focus navigation **only** when nobody else claims it (a text
        // area uses Tab for indentation) and only bare or with Shift — ⌘Tab
        // and Ctrl+Tab belong to the OS/application, not to widget traversal.
        if !out.handled && event.is_pressed() && event.code.is(NamedKey::Tab) {
            let arah = if event.modifiers.is_exactly(Modifiers::SHIFT) {
                Some(FocusDirection::Previous)
            } else if event.modifiers.is_exactly(Modifiers::NONE) {
                Some(FocusDirection::Next)
            } else {
                None
            };
            if let Some(arah) = arah {
                let change = self.focus.move_focus(tree, arah);
                self.terapkan_fokus(tree, change, &mut out);
                out.handled = true;
            }
        }
        out
    }

    // -- IME --------------------------------------------------------------

    fn ime_event(&mut self, tree: &mut RenderTree, event: &ImeEvent) -> Response {
        let mut out = Response::default();
        let Some(fokus) = self.focus.focused() else {
            // No destination for the composition: do not leave the IME on by
            // itself.
            if self.ime.take().is_some() {
                out.ime = Some(ImeRequest::Disable);
            }
            return out;
        };
        let rute = vec![HitEntry {
            node: fokus,
            local: Point::ZERO,
        }];
        let mut sink = Sink::default();
        out.handled = self.kirim(tree, &rute, &Event::Ime(event.clone()), &mut sink);
        self.terapkan(tree, sink, None, &mut out);
        out
    }

    // -- the delivery machinery -------------------------------------------

    /// Send an event along a route (innermost first) until something handles
    /// it.
    fn kirim(
        &mut self,
        tree: &mut RenderTree,
        rute: &[HitEntry],
        event: &Event,
        sink: &mut Sink,
    ) -> bool {
        let mut handled = false;
        for entry in rute {
            self.sampaikan(tree, entry.node, entry.local, event, sink, &mut handled);
            if handled {
                break;
            }
        }
        handled
    }

    /// Send to a single node only (enter/leave, focus, IME).
    fn kirim_satu(
        &mut self,
        tree: &mut RenderTree,
        node: NodeId,
        local: Point,
        event: &Event,
        sink: &mut Sink,
    ) {
        let mut handled = false;
        self.sampaikan(tree, node, local, event, sink, &mut handled);
    }

    fn sampaikan(
        &mut self,
        tree: &mut RenderTree,
        node: NodeId,
        local: Point,
        event: &Event,
        sink: &mut Sink,
        handled: &mut bool,
    ) {
        // The node is temporarily taken out of the arena — the same pattern as
        // layout, and for the same reason: a handler must not see itself in
        // the tree.
        let Some(mut render) = tree.take_render(node) else {
            return;
        };
        let mut ctx = EventCtx {
            node,
            local,
            size: tree.size(node),
            bounds: tree.bounds(node),
            focused: self.focus.is_focused(node),
            handled,
            sink,
        };
        render.event(&mut ctx, event);
        tree.put_render(node, render);
    }

    /// Apply what the nodes left behind: focus, capture, IME and dirty
    /// reasons.
    ///
    /// `pointer` is the pointer being processed; capture applies to it alone —
    /// a second finger on a touch screen must not get captured too.
    fn terapkan(
        &mut self,
        tree: &mut RenderTree,
        sink: Sink,
        pointer: Option<PointerId>,
        out: &mut Response,
    ) {
        out.dirty |= sink.dirty;

        if let (Some(capture), Some(id)) = (sink.capture, pointer) {
            self.pointers.entry(id).or_default().capture = capture;
        }

        if let Some(target) = sink.focus {
            let change = self.focus.focus(tree, target);
            self.terapkan_fokus(tree, change, out);
        }

        if let Some((node, area)) = sink.ime {
            self.terapkan_ime(node, area, out);
        }
    }

    fn terapkan_fokus(&mut self, tree: &mut RenderTree, change: FocusChange, out: &mut Response) {
        if !change.changed() {
            return;
        }
        out.focus = change;
        out.dirty |= Dirty::PAINT;
        let mut sink = Sink::default();
        if let Some(lost) = change.lost {
            self.kirim_satu(
                tree,
                lost,
                Point::ZERO,
                &Event::Focus(FocusEvent::Lost),
                &mut sink,
            );
        }
        if let Some(gained) = change.gained {
            self.kirim_satu(
                tree,
                gained,
                Point::ZERO,
                &Event::Focus(FocusEvent::Gained),
                &mut sink,
            );
        }
        out.dirty |= sink.dirty;
        // A node losing focus usually turns the IME off and one gaining focus
        // turns it on — both through the same request slot.
        if let Some((node, area)) = sink.ime {
            self.terapkan_ime(node, area, out);
        }
        // An IME session owned by a node that no longer has focus must not be
        // left hanging.
        if let Some((owner, _)) = self.ime {
            if !self.focus.is_focused(owner) {
                self.ime = None;
                out.ime = Some(ImeRequest::Disable);
            }
        }
    }

    fn terapkan_ime(&mut self, node: NodeId, area: Option<Rect>, out: &mut Response) {
        match area {
            Some(area) => {
                let permintaan = match self.ime {
                    Some((owner, sebelumnya)) if owner == node => {
                        if sebelumnya == area {
                            None
                        } else {
                            Some(ImeRequest::Update { area })
                        }
                    }
                    _ => Some(ImeRequest::Enable { area }),
                };
                self.ime = Some((node, area));
                if permintaan.is_some() {
                    out.ime = permintaan;
                }
            }
            None => {
                if matches!(self.ime, Some((owner, _)) if owner == node) {
                    self.ime = None;
                    out.ime = Some(ImeRequest::Disable);
                }
            }
        }
    }
}

/// The route from a node up to the root, with local coordinates computed from
/// the global offset — used while a pointer is captured.
fn rute_dari_node(tree: &RenderTree, node: NodeId, position: Point) -> Vec<HitEntry> {
    let mut rute = Vec::new();
    let mut cur = Some(node);
    while let Some(id) = cur {
        if !tree.contains(id) {
            break;
        }
        let origin = tree.global_offset(id);
        rute.push(HitEntry {
            node: id,
            local: Point::new(position.x - origin.x, position.y - origin.y),
        });
        cur = tree.parent(id);
    }
    rute
}

fn hitung_klik(state: &PointerState, event: &PointerEvent, config: ClickConfig) -> u32 {
    let Some(button) = event.button else { return 0 };
    match state.last_click {
        Some((sebelumnya, posisi, waktu))
            if sebelumnya == button
                && event.time.saturating_sub(waktu) <= config.interval
                && jarak(posisi, event.position) <= config.distance =>
        {
            state.click_count.saturating_add(1).max(2)
        }
        _ => 1,
    }
}

fn jarak(a: Point, b: Point) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}
