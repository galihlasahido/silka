//! The window manager, as plain Rust.
//!
//! Everything an MDI desktop actually *decides* lives here: who is in front,
//! what a drag does to a rectangle, where cascade puts the next window, which
//! window a fling sticks to. Not one framework type appears in this module
//! beyond [`Rect`]/[`Size`]/[`Point`] and the pointer [`Velocity`] — which is
//! why all of it is tested without a window, exactly the way `silka-todo`
//! tests its task list.
//!
//! Two invariants hold the whole thing together, and both are asserted by the
//! tests below:
//!
//! 1. **The frame vector *is* the z-order**, back to front. Raising a window is
//!    a rotate, not a counter that some other list has to be kept in step with.
//! 2. **The active window is the frontmost one that is not minimized.** There
//!    is no separate `active` field to go stale when a window is closed or
//!    minimized out from under it.

use silka_core::input::Velocity;
use silka_paint::{Point, Rect, Size};

/// The identity of an internal frame.
///
/// Stable for the life of the window: it is the view [`Key`](silka_core::signals::Key)
/// of its overlay, so re-sorting the z-order **moves** a frame's nodes instead
/// of rebuilding them — which is what lets a window keep the pointer capture it
/// grabbed a moment ago while it is being raised (§2.5).
pub type FrameId = u32;

/// The smallest an internal frame may be dragged down to.
///
/// Not a taste decision: below this the titlebar buttons stop fitting, and a
/// window whose own close button is unreachable is a trap.
pub const MIN_FRAME: Size = Size::new(260.0, 140.0);

/// The size a freshly opened window gets.
pub const DEFAULT_FRAME: Size = Size::new(420.0, 280.0);

/// The diagonal step between cascaded windows.
pub const CASCADE_STEP: f32 = 28.0;

/// The desktop size assumed until a real layout publishes one.
///
/// A window opened before the first frame still has to go **somewhere**, and
/// clamping it against a desktop of size zero would stack every window in the
/// top-left corner — which is what happened the first time this was written.
/// The real size arrives one frame later and every window is clamped into it
/// then ([`Mdi::set_desktop`]).
pub const INITIAL_DESKTOP: Size = Size::new(1024.0, 640.0);

/// How fast the pointer must still be travelling at release for a drag to count
/// as a fling, in points per second.
pub const FLING_SPEED: f32 = 900.0;

/// How close to an edge the window must already be for a fling to stick to it.
pub const FLING_MARGIN: f32 = 48.0;

/// How far one arrow key press moves or resizes a window.
///
/// Six steps of the 4pt spacing scale: large enough to cross a desktop without
/// wearing out a keyboard, small enough to place a window precisely.
pub const KEY_STEP: f32 = 24.0;

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

/// What an internal frame is currently doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameState {
    /// A free-floating window at [`Frame::rect`].
    Normal,
    /// Filling the desktop; [`Frame::restore`] remembers where it came from.
    Maximized,
    /// Collapsed to the taskbar: no pixels, no pointer, no tab stops.
    Minimized,
}

impl FrameState {
    /// True when the window contributes a rectangle to the desktop.
    pub fn is_visible(self) -> bool {
        !matches!(self, FrameState::Minimized)
    }
}

/// One internal frame — the model half of a `JInternalFrame`.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// Stable identity; also the view key.
    pub id: FrameId,
    /// The titlebar text, and the accessible name of the whole window.
    pub title: String,
    /// The document this window shows — one paragraph, because the point of
    /// this example is the chrome around it.
    pub body: String,
    /// A line the user can actually type into.
    ///
    /// There is one per window on purpose: a text field is the cheapest proof
    /// that keyboard focus really is trapped in the window in front, because a
    /// keystroke that lands in the wrong document is impossible to miss.
    pub note: String,
    /// Where the window is right now, in desktop-local points.
    pub rect: Rect,
    /// Where an un-maximize puts it back.
    pub restore: Rect,
    /// Normal / maximized / minimized.
    pub state: FrameState,
}

impl Frame {
    /// True when this window is drawn on the desktop at all.
    pub fn is_visible(&self) -> bool {
        self.state.is_visible()
    }
}

// ---------------------------------------------------------------------------
// Drag
// ---------------------------------------------------------------------------

/// Which of the eight edges a resize drag has hold of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edge {
    /// Top edge.
    North,
    /// Bottom edge.
    South,
    /// Right edge.
    East,
    /// Left edge.
    West,
    /// Top-right corner.
    NorthEast,
    /// Top-left corner.
    NorthWest,
    /// Bottom-right corner.
    SouthEast,
    /// Bottom-left corner.
    SouthWest,
}

impl Edge {
    /// All eight, in **reading order across the frame**: the top row left to
    /// right, then the two sides, then the bottom row.
    ///
    /// This is not a convenience list — `frame::resize_lattice` builds the
    /// window's nine cells straight out of it, so the order a screen reader
    /// hears the edges in and the order they are laid out in cannot drift
    /// apart.
    pub const ALL: [Edge; 8] = [
        Edge::NorthWest,
        Edge::North,
        Edge::NorthEast,
        Edge::West,
        Edge::East,
        Edge::SouthWest,
        Edge::South,
        Edge::SouthEast,
    ];

    /// This edge's stable name — the view key and the a11y name both use it.
    pub const fn name(self) -> &'static str {
        match self {
            Edge::North => "top",
            Edge::South => "bottom",
            Edge::East => "right",
            Edge::West => "left",
            Edge::NorthEast => "top right",
            Edge::NorthWest => "top left",
            Edge::SouthEast => "bottom right",
            Edge::SouthWest => "bottom left",
        }
    }

    /// True when dragging this edge moves the window's left side.
    pub const fn moves_left(self) -> bool {
        matches!(self, Edge::West | Edge::NorthWest | Edge::SouthWest)
    }

    /// True when dragging this edge moves the window's right side.
    pub const fn moves_right(self) -> bool {
        matches!(self, Edge::East | Edge::NorthEast | Edge::SouthEast)
    }

    /// True when dragging this edge moves the window's top.
    pub const fn moves_top(self) -> bool {
        matches!(self, Edge::North | Edge::NorthEast | Edge::NorthWest)
    }

    /// True when dragging this edge moves the window's bottom.
    pub const fn moves_bottom(self) -> bool {
        matches!(self, Edge::South | Edge::SouthEast | Edge::SouthWest)
    }
}

/// What a drag in flight is doing to a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragKind {
    /// The titlebar: the whole window travels.
    Move,
    /// One of the eight edges.
    Resize(Edge),
}

/// A drag in flight.
///
/// The **origin rectangle** is snapshotted at the start and every update is
/// computed from it plus the total delta, rather than accumulated frame by
/// frame: accumulation drifts as soon as one update is clamped at an edge, and
/// the window then never catches up with the finger again.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drag {
    /// The window being dragged.
    pub id: FrameId,
    /// Move or resize.
    pub kind: DragKind,
    /// Its rectangle when the drag began.
    pub origin: Rect,
}

/// Where a fling stuck the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Snap {
    /// The left half of the desktop.
    Left,
    /// The right half.
    Right,
    /// The whole desktop (the window is maximized).
    Top,
}

// ---------------------------------------------------------------------------
// Geometry — pure functions
// ---------------------------------------------------------------------------

/// Keep `rect` fully inside a `bounds`-sized desktop, without resizing it.
///
/// A window bigger than the desktop is pinned to the origin rather than pushed
/// off the top-left: a title you cannot reach is worse than an edge you cannot
/// see.
pub fn clamp_inside(rect: Rect, bounds: Size) -> Rect {
    let x = (rect.min_x()).min(bounds.width - rect.size.width).max(0.0);
    let y = (rect.min_y())
        .min(bounds.height - rect.size.height)
        .max(0.0);
    Rect::from_origin_size(Point::new(x, y), rect.size)
}

/// The rectangle a move-drag of `delta` produces from `origin`.
pub fn dragged(origin: Rect, delta: Point, bounds: Size) -> Rect {
    clamp_inside(origin.translated(delta), bounds)
}

/// The rectangle a resize-drag of `delta` on `edge` produces from `origin`.
///
/// Every edge is clamped twice: against [`MIN_FRAME`], so a window cannot be
/// folded shut, and against the desktop, so an edge cannot be dragged out of
/// reach. The opposite edge never moves — that is the whole difference between
/// a resize and a move.
pub fn resized(origin: Rect, edge: Edge, delta: Point, bounds: Size) -> Rect {
    let mut left = origin.min_x();
    let mut right = origin.max_x();
    let mut top = origin.min_y();
    let mut bottom = origin.max_y();

    if edge.moves_left() {
        left = (left + delta.x).max(0.0).min(right - MIN_FRAME.width);
    }
    if edge.moves_right() {
        right = (right + delta.x)
            .min(bounds.width.max(left + MIN_FRAME.width))
            .max(left + MIN_FRAME.width);
    }
    if edge.moves_top() {
        top = (top + delta.y).max(0.0).min(bottom - MIN_FRAME.height);
    }
    if edge.moves_bottom() {
        bottom = (bottom + delta.y)
            .min(bounds.height.max(top + MIN_FRAME.height))
            .max(top + MIN_FRAME.height);
    }

    Rect::new(left, top, right - left, bottom - top)
}

/// The edge a released drag sticks to, if any.
///
/// Two conditions, both required: the pointer is still travelling fast
/// ([`FLING_SPEED`]) and the window is already *at* that edge
/// ([`FLING_MARGIN`]). Speed alone would snap every brisk drag; proximity alone
/// would snap every window merely parked near a border.
pub fn fling_snap(rect: Rect, velocity: Velocity, bounds: Size) -> Option<Snap> {
    if velocity.magnitude() < FLING_SPEED || bounds.is_empty() {
        return None;
    }
    let horizontal = velocity.x.abs() >= velocity.y.abs();
    if horizontal {
        if velocity.x < 0.0 && rect.min_x() <= FLING_MARGIN {
            return Some(Snap::Left);
        }
        if velocity.x > 0.0 && rect.max_x() >= bounds.width - FLING_MARGIN {
            return Some(Snap::Right);
        }
        return None;
    }
    if velocity.y < 0.0 && rect.min_y() <= FLING_MARGIN {
        return Some(Snap::Top);
    }
    None
}

/// The rectangle a [`Snap`] resolves to on a `bounds`-sized desktop.
pub fn snap_rect(snap: Snap, bounds: Size) -> Rect {
    let half = (bounds.width * 0.5).max(MIN_FRAME.width.min(bounds.width));
    match snap {
        Snap::Left => Rect::new(0.0, 0.0, half, bounds.height),
        Snap::Right => Rect::new(bounds.width - half, 0.0, half, bounds.height),
        Snap::Top => Rect::from_origin_size(Point::ZERO, bounds),
    }
}

// ---------------------------------------------------------------------------
// The desktop
// ---------------------------------------------------------------------------

/// Every window on the desktop, in z-order, plus the drag in flight.
#[derive(Debug, Clone, PartialEq)]
pub struct Mdi {
    /// Back to front: the last element is the frontmost window.
    frames: Vec<Frame>,
    /// The desktop's size in points, published by the layout pass.
    desktop: Size,
    /// The id the next window gets.
    next_id: FrameId,
    /// The drag in flight, if any.
    drag: Option<Drag>,
    /// The window whose traffic lights the pointer is resting on, published
    /// after layout by [`crate::traffic::sync`].
    ///
    /// It lives here rather than in the nodes that draw the lights because the
    /// three lights of one window light up **together**: the fact belongs to
    /// the group, and only the view can hand it to all three.
    lit: Option<FrameId>,
}

impl Default for Mdi {
    fn default() -> Self {
        Self::new()
    }
}

impl Mdi {
    /// An empty desktop, sized [`INITIAL_DESKTOP`] until layout says otherwise.
    pub fn new() -> Self {
        Self {
            frames: Vec::new(),
            desktop: INITIAL_DESKTOP,
            next_id: 1,
            drag: None,
            lit: None,
        }
    }

    /// The desktop the example opens with: three cascaded windows.
    pub fn demo() -> Self {
        let mut mdi = Mdi::new();
        mdi.open(
            "Ledger",
            "Every window here is an overlay entry in one layer. The layer's \
             child order is the z-order, so bringing a window to the front is a \
             rotate of a Vec — not a paint-order flag on nine widgets.",
        );
        mdi.open(
            "Journal",
            "Drag the titlebar to move, any edge or corner to resize. The drag \
             is computed from the rectangle the gesture started on, so a window \
             pushed against a wall snaps straight back to the finger.",
        );
        mdi.open(
            "Notes",
            "Minimize sends the window to the taskbar on a spring. Press it \
             again mid-flight and the spring reverses carrying its velocity — \
             it does not restart from zero.",
        );
        mdi
    }

    // -- reading ----------------------------------------------------------

    /// Every window, back to front.
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// How many windows are open.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// True when no window is open.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// One window by id.
    pub fn get(&self, id: FrameId) -> Option<&Frame> {
        self.frames.iter().find(|f| f.id == id)
    }

    /// The desktop's size in points.
    pub fn desktop(&self) -> Size {
        self.desktop
    }

    /// The window whose traffic lights are showing their glyphs.
    pub fn lit_lights(&self) -> Option<FrameId> {
        self.lit
    }

    /// Record which window's traffic lights the pointer is on.
    ///
    /// Returns false when nothing changed, exactly like [`Mdi::set_desktop`]:
    /// the pass that calls this runs after **every** frame, and a write that
    /// changes nothing would still rebuild the whole desktop.
    pub fn set_lit_lights(&mut self, id: Option<FrameId>) -> bool {
        if self.lit == id {
            return false;
        }
        self.lit = id;
        true
    }

    /// The drag in flight.
    pub fn drag(&self) -> Option<Drag> {
        self.drag
    }

    /// The frontmost window that is not minimized — **the active window**.
    ///
    /// Derived rather than stored: a field would have to be fixed up by close,
    /// minimize, cascade and tile alike, and one forgotten path is a desktop
    /// where the keyboard talks to a window nobody can see.
    pub fn active(&self) -> Option<FrameId> {
        self.frames
            .iter()
            .rev()
            .find(|f| f.is_visible())
            .map(|f| f.id)
    }

    /// True when `id` is the active window.
    pub fn is_active(&self, id: FrameId) -> bool {
        self.active() == Some(id)
    }

    /// True when `id` is the window currently being dragged or resized.
    pub fn is_dragging(&self, id: FrameId) -> bool {
        self.drag().map(|d| d.id) == Some(id)
    }

    /// The ids currently collapsed to the taskbar, in the order they were
    /// opened.
    pub fn minimized(&self) -> Vec<FrameId> {
        self.frames
            .iter()
            .filter(|f| !f.is_visible())
            .map(|f| f.id)
            .collect()
    }

    // -- z-order ----------------------------------------------------------

    /// Bring `id` to the front. Returns true when the order actually changed.
    pub fn raise(&mut self, id: FrameId) -> bool {
        let Some(i) = self.frames.iter().position(|f| f.id == id) else {
            return false;
        };
        if i + 1 == self.frames.len() {
            return false;
        }
        let frame = self.frames.remove(i);
        self.frames.push(frame);
        true
    }

    /// Send `id` to the very back of the stack.
    ///
    /// The exact inverse of [`Mdi::raise`], which is what lets Ctrl+Tab and
    /// Ctrl+Shift+Tab undo each other instead of ping-ponging between two
    /// windows.
    pub fn lower(&mut self, id: FrameId) -> bool {
        let Some(i) = self.frames.iter().position(|f| f.id == id) else {
            return false;
        };
        if i == 0 {
            return false;
        }
        let frame = self.frames.remove(i);
        self.frames.insert(0, frame);
        true
    }

    /// Rotate the stack — Ctrl+Tab.
    ///
    /// Forward brings the **hindmost** window to the front, so repeating it
    /// visits every window in turn and comes back round; a "swap with the
    /// previous one" rule would bounce between the same two forever. Backward
    /// is the exact inverse: the front window goes to the very back.
    ///
    /// Returns the window that ended up in front.
    pub fn cycle(&mut self, forward: bool) -> Option<FrameId> {
        let visible: Vec<FrameId> = self
            .frames
            .iter()
            .filter(|f| f.is_visible())
            .map(|f| f.id)
            .collect();
        if visible.len() < 2 {
            return visible.last().copied();
        }
        if forward {
            let target = visible[0];
            self.raise(target);
            Some(target)
        } else {
            let front = *visible.last().expect("checked non-empty above");
            self.lower(front);
            self.active()
        }
    }

    // -- opening and closing ----------------------------------------------

    /// Open a new window, cascaded past the frontmost one, and raise it.
    pub fn open(&mut self, title: impl Into<String>, body: impl Into<String>) -> FrameId {
        let id = self.next_id;
        self.next_id += 1;
        let step = self.frames.len() as f32 * CASCADE_STEP;
        let rect = clamp_inside(
            Rect::from_origin_size(Point::new(step, step), DEFAULT_FRAME),
            self.bounds(),
        );
        let title = title.into();
        self.frames.push(Frame {
            id,
            note: format!("Notes for {title}"),
            title,
            body: body.into(),
            rect,
            restore: rect,
            state: FrameState::Normal,
        });
        id
    }

    /// Replace a window's editable line.
    pub fn set_note(&mut self, id: FrameId, note: impl Into<String>) {
        if let Some(f) = self.frame_mut(id) {
            f.note = note.into();
        }
    }

    /// Close a window. Returns true when there was one to close.
    pub fn close(&mut self, id: FrameId) -> bool {
        let before = self.frames.len();
        self.frames.retain(|f| f.id != id);
        if self.drag.map(|d| d.id) == Some(id) {
            self.drag = None;
        }
        self.frames.len() != before
    }

    // -- window state -----------------------------------------------------

    /// Collapse a window to the taskbar.
    pub fn minimize(&mut self, id: FrameId) {
        if let Some(f) = self.frame_mut(id) {
            if f.state != FrameState::Minimized {
                f.state = FrameState::Minimized;
            }
        }
    }

    /// Bring a window back from the taskbar (or out of maximized) and raise it.
    pub fn restore(&mut self, id: FrameId) {
        let bounds = self.bounds();
        if let Some(f) = self.frame_mut(id) {
            match f.state {
                FrameState::Minimized => f.state = FrameState::Normal,
                FrameState::Maximized => {
                    f.state = FrameState::Normal;
                    f.rect = clamp_inside(f.restore, bounds);
                }
                FrameState::Normal => {}
            }
        }
        self.raise(id);
    }

    /// Maximize a window, or un-maximize an already maximized one.
    pub fn toggle_maximize(&mut self, id: FrameId) {
        let bounds = self.bounds();
        if let Some(f) = self.frame_mut(id) {
            match f.state {
                FrameState::Maximized => {
                    f.state = FrameState::Normal;
                    f.rect = clamp_inside(f.restore, bounds);
                }
                _ => {
                    if f.state == FrameState::Normal {
                        f.restore = f.rect;
                    }
                    f.state = FrameState::Maximized;
                    f.rect = Rect::from_origin_size(Point::ZERO, bounds);
                }
            }
        }
        self.raise(id);
    }

    /// Minimize every open window.
    pub fn minimize_all(&mut self) {
        for f in &mut self.frames {
            f.state = FrameState::Minimized;
        }
    }

    // -- arrangement ------------------------------------------------------

    /// Lay the visible windows out in a diagonal cascade, front-most last.
    ///
    /// The step wraps once the next window would fall off the bottom, which is
    /// what keeps a cascade of twelve windows from walking off the desktop.
    pub fn cascade(&mut self) {
        let bounds = self.bounds();
        if bounds.is_empty() {
            return;
        }
        let size = Size::new(
            DEFAULT_FRAME.width.min(bounds.width),
            DEFAULT_FRAME.height.min(bounds.height),
        );
        let per_column = (((bounds.height - size.height) / CASCADE_STEP).floor() as i32).max(1);
        let mut n = 0i32;
        for f in &mut self.frames {
            if !f.is_visible() {
                continue;
            }
            let column = n / per_column;
            let row = n % per_column;
            let origin = Point::new(
                column as f32 * CASCADE_STEP * 2.0 + row as f32 * CASCADE_STEP,
                row as f32 * CASCADE_STEP,
            );
            f.state = FrameState::Normal;
            f.rect = clamp_inside(Rect::from_origin_size(origin, size), bounds);
            f.restore = f.rect;
            n += 1;
        }
    }

    /// Tile the visible windows into a grid that fills the desktop.
    pub fn tile(&mut self) {
        let bounds = self.bounds();
        let count = self.frames.iter().filter(|f| f.is_visible()).count();
        if bounds.is_empty() || count == 0 {
            return;
        }
        let (cols, rows) = grid_for(count);
        let cell = Size::new(bounds.width / cols as f32, bounds.height / rows as f32);
        let mut n = 0usize;
        for f in &mut self.frames {
            if !f.is_visible() {
                continue;
            }
            let column = n % cols;
            let row = n / cols;
            f.state = FrameState::Normal;
            f.rect = Rect::new(
                column as f32 * cell.width,
                row as f32 * cell.height,
                cell.width,
                cell.height,
            );
            f.restore = f.rect;
            n += 1;
        }
    }

    // -- the desktop's own size -------------------------------------------

    /// Publish the desktop's size from the finished layout.
    ///
    /// Returns true when it changed, which is what keeps the frame loop from
    /// writing this signal — and rebuilding the whole desktop — every frame.
    /// Maximized windows follow the new size; the rest are only pulled back
    /// inside it.
    pub fn set_desktop(&mut self, size: Size) -> bool {
        if self.desktop == size {
            return false;
        }
        self.desktop = size;
        let bounds = self.bounds();
        for f in &mut self.frames {
            match f.state {
                FrameState::Maximized => f.rect = Rect::from_origin_size(Point::ZERO, bounds),
                _ => f.rect = clamp_inside(f.rect, bounds),
            }
        }
        true
    }

    // -- dragging ---------------------------------------------------------

    /// Begin a drag on `id`, snapshotting the rectangle it starts from.
    ///
    /// A maximized window is un-maximized by the act of dragging its titlebar,
    /// the way every desktop does it — otherwise the drag would silently do
    /// nothing.
    pub fn begin_drag(&mut self, id: FrameId, kind: DragKind) {
        self.raise(id);
        let bounds = self.bounds();
        if kind == DragKind::Move {
            if let Some(f) = self.frame_mut(id) {
                if f.state == FrameState::Maximized {
                    f.state = FrameState::Normal;
                    f.rect = clamp_inside(f.restore, bounds);
                }
            }
        }
        let Some(f) = self.get(id) else { return };
        self.drag = Some(Drag {
            id,
            kind,
            origin: f.rect,
        });
    }

    /// Apply the **total** delta since the drag began.
    pub fn drag_to(&mut self, delta: Point) {
        let Some(drag) = self.drag else { return };
        let bounds = self.bounds();
        let rect = match drag.kind {
            DragKind::Move => dragged(drag.origin, delta, bounds),
            DragKind::Resize(edge) => resized(drag.origin, edge, delta, bounds),
        };
        if let Some(f) = self.frame_mut(drag.id) {
            f.rect = rect;
            f.restore = rect;
        }
    }

    /// Finish the drag, letting a fling stick the window to an edge.
    ///
    /// Returns the snap that was applied, if any.
    pub fn end_drag(&mut self, delta: Point, velocity: Velocity) -> Option<Snap> {
        let drag = self.drag.take()?;
        self.drag = Some(drag);
        self.drag_to(delta);
        self.drag = None;

        if drag.kind != DragKind::Move {
            return None;
        }
        let bounds = self.bounds();
        let rect = self.get(drag.id)?.rect;
        let snap = fling_snap(rect, velocity, bounds)?;
        let target = snap_rect(snap, bounds);
        if let Some(f) = self.frame_mut(drag.id) {
            // The pre-snap rectangle is what an un-maximize returns to, so a
            // window flung to the top can be dragged straight back out.
            f.restore = drag.origin;
            f.rect = target;
            f.state = if snap == Snap::Top {
                FrameState::Maximized
            } else {
                FrameState::Normal
            };
        }
        Some(snap)
    }

    /// Abandon the drag: the OS took the gesture away.
    pub fn cancel_drag(&mut self) {
        let Some(drag) = self.drag.take() else { return };
        if let Some(f) = self.frame_mut(drag.id) {
            f.rect = drag.origin;
            f.restore = drag.origin;
        }
    }

    // -- internals --------------------------------------------------------

    fn frame_mut(&mut self, id: FrameId) -> Option<&mut Frame> {
        self.frames.iter_mut().find(|f| f.id == id)
    }

    /// The desktop rectangle windows are clamped to.
    ///
    /// Never smaller than one window: on a desktop narrower than
    /// [`MIN_FRAME`], clamping to the real size would produce negative widths.
    fn bounds(&self) -> Size {
        Size::new(
            self.desktop.width.max(MIN_FRAME.width),
            self.desktop.height.max(MIN_FRAME.height),
        )
    }
}

/// The grid a tile of `count` windows uses: as square as possible, wider than
/// tall — a screen is wider than it is tall, and so is a document.
pub fn grid_for(count: usize) -> (usize, usize) {
    if count == 0 {
        return (1, 1);
    }
    let cols = (count as f32).sqrt().ceil() as usize;
    let rows = count.div_ceil(cols);
    (cols.max(1), rows.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP: Size = Size::new(1000.0, 700.0);

    fn desktop() -> Mdi {
        let mut m = Mdi::new();
        m.set_desktop(DESKTOP);
        m
    }

    fn with_three() -> Mdi {
        let mut m = desktop();
        m.open("A", "a");
        m.open("B", "b");
        m.open("C", "c");
        m
    }

    // -- z-order ----------------------------------------------------------

    #[test]
    fn the_newest_window_is_the_active_one() {
        let m = with_three();
        assert_eq!(m.active(), Some(3));
        assert_eq!(
            m.frames().iter().map(|f| f.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn raising_moves_a_window_to_the_end_and_leaves_the_rest_in_order() {
        let mut m = with_three();
        assert!(m.raise(1));
        assert_eq!(
            m.frames().iter().map(|f| f.id).collect::<Vec<_>>(),
            vec![2, 3, 1]
        );
        assert_eq!(m.active(), Some(1));
        // Raising what is already in front is not a change — the view diff
        // would otherwise re-key three overlays for nothing.
        assert!(!m.raise(1));
    }

    #[test]
    fn a_minimized_window_is_never_the_active_one() {
        let mut m = with_three();
        m.minimize(3);
        assert_eq!(m.active(), Some(2));
        m.minimize(2);
        assert_eq!(m.active(), Some(1));
        m.minimize(1);
        assert_eq!(m.active(), None, "an empty desktop has no active window");
        assert_eq!(m.minimized(), vec![1, 2, 3]);
    }

    #[test]
    fn restoring_from_the_taskbar_raises() {
        let mut m = with_three();
        m.minimize(1);
        m.restore(1);
        assert_eq!(m.active(), Some(1));
        assert_eq!(m.get(1).unwrap().state, FrameState::Normal);
    }

    #[test]
    fn cycling_rotates_through_every_window_and_comes_back_round() {
        let mut m = with_three();
        assert_eq!(m.active(), Some(3));
        assert_eq!(m.cycle(true), Some(1));
        assert_eq!(m.cycle(true), Some(2));
        assert_eq!(m.cycle(true), Some(3), "three steps is a full turn");

        // Backward undoes forward exactly, rather than bouncing between two.
        let before: Vec<FrameId> = m.frames().iter().map(|f| f.id).collect();
        m.cycle(true);
        m.cycle(false);
        assert_eq!(m.frames().iter().map(|f| f.id).collect::<Vec<_>>(), before);
    }

    #[test]
    fn cycling_skips_the_windows_in_the_taskbar() {
        let mut m = with_three();
        m.minimize(1);
        assert_eq!(m.cycle(true), Some(2));
        assert_eq!(m.cycle(true), Some(3));
        assert_eq!(m.cycle(true), Some(2), "window 1 is not in the rotation");
    }

    #[test]
    fn closing_the_active_window_hands_the_desktop_to_the_next_one() {
        let mut m = with_three();
        assert!(m.close(3));
        assert_eq!(m.active(), Some(2));
        assert!(!m.close(3), "closing twice is not an error, just a no-op");
    }

    // -- geometry ---------------------------------------------------------

    #[test]
    fn a_move_drag_is_clamped_to_the_desktop() {
        let origin = Rect::new(100.0, 100.0, 400.0, 300.0);
        let far = dragged(origin, Point::new(-9_000.0, -9_000.0), DESKTOP);
        assert_eq!(far.min_x(), 0.0);
        assert_eq!(far.min_y(), 0.0);
        assert_eq!(far.size, origin.size, "a move never resizes");

        let other = dragged(origin, Point::new(9_000.0, 9_000.0), DESKTOP);
        assert_eq!(other.max_x(), DESKTOP.width);
        assert_eq!(other.max_y(), DESKTOP.height);
    }

    #[test]
    fn a_resize_holds_the_opposite_edge_still() {
        let origin = Rect::new(100.0, 100.0, 400.0, 300.0);
        let west = resized(origin, Edge::West, Point::new(-40.0, 0.0), DESKTOP);
        assert_eq!(west.min_x(), 60.0);
        assert_eq!(west.max_x(), origin.max_x(), "the right edge stayed put");
        assert_eq!(west.max_y(), origin.max_y());

        let corner = resized(origin, Edge::SouthEast, Point::new(50.0, 60.0), DESKTOP);
        assert_eq!(corner.min_x(), origin.min_x());
        assert_eq!(corner.min_y(), origin.min_y());
        assert_eq!(corner.size, Size::new(450.0, 360.0));
    }

    #[test]
    fn a_resize_can_never_fold_a_window_shut() {
        let origin = Rect::new(100.0, 100.0, 400.0, 300.0);
        for edge in Edge::ALL {
            for delta in [Point::new(9_000.0, 9_000.0), Point::new(-9_000.0, -9_000.0)] {
                let r = resized(origin, edge, delta, DESKTOP);
                assert!(
                    r.size.width >= MIN_FRAME.width - 0.01
                        && r.size.height >= MIN_FRAME.height - 0.01,
                    "{edge:?} with {delta:?} produced {r:?}"
                );
                assert!(
                    r.min_x() >= -0.01
                        && r.min_y() >= -0.01
                        && r.max_x() <= DESKTOP.width + 0.01
                        && r.max_y() <= DESKTOP.height + 0.01,
                    "{edge:?} left the desktop: {r:?}"
                );
            }
        }
    }

    #[test]
    fn every_edge_moves_exactly_the_sides_it_names() {
        for edge in Edge::ALL {
            assert!(
                !(edge.moves_left() && edge.moves_right()),
                "{edge:?} cannot own both vertical sides"
            );
            assert!(!(edge.moves_top() && edge.moves_bottom()));
            let horizontal = edge.moves_left() || edge.moves_right();
            let vertical = edge.moves_top() || edge.moves_bottom();
            assert!(horizontal || vertical, "{edge:?} moves nothing");
        }
    }

    // -- fling ------------------------------------------------------------

    #[test]
    fn a_fling_needs_both_speed_and_proximity() {
        let at_edge = Rect::new(10.0, 200.0, 400.0, 300.0);
        let middle = Rect::new(300.0, 200.0, 400.0, 300.0);
        let fast = Velocity::new(-1_500.0, 0.0);
        let slow = Velocity::new(-100.0, 0.0);

        assert_eq!(fling_snap(at_edge, fast, DESKTOP), Some(Snap::Left));
        assert_eq!(fling_snap(at_edge, slow, DESKTOP), None, "too slow");
        assert_eq!(fling_snap(middle, fast, DESKTOP), None, "too far away");
    }

    #[test]
    fn a_fling_upwards_maximizes_and_keeps_somewhere_to_come_back_to() {
        let mut m = with_three();
        let before = Rect::new(200.0, 20.0, 400.0, 300.0);
        m.frame_mut(2).unwrap().rect = before;
        m.begin_drag(2, DragKind::Move);
        let snap = m.end_drag(Point::ZERO, Velocity::new(0.0, -2_000.0));
        assert_eq!(snap, Some(Snap::Top));
        let f = m.get(2).unwrap();
        assert_eq!(f.state, FrameState::Maximized);
        assert_eq!(f.rect, Rect::new(0.0, 0.0, DESKTOP.width, DESKTOP.height));
        assert_eq!(f.restore, before, "the way back survived the snap");
    }

    #[test]
    fn a_fling_to_the_right_takes_the_right_half() {
        let at_edge = Rect::new(DESKTOP.width - 410.0, 200.0, 400.0, 300.0);
        assert_eq!(
            fling_snap(at_edge, Velocity::new(2_000.0, 0.0), DESKTOP),
            Some(Snap::Right)
        );
        let half = snap_rect(Snap::Right, DESKTOP);
        assert_eq!(half.max_x(), DESKTOP.width);
        assert_eq!(half.size.width, DESKTOP.width * 0.5);
        assert_eq!(half.size.height, DESKTOP.height);
        // The two halves meet exactly in the middle and cover the desktop.
        let left = snap_rect(Snap::Left, DESKTOP);
        assert_eq!(left.max_x(), half.min_x());
    }

    #[test]
    fn a_resize_fling_never_snaps() {
        let mut m = with_three();
        m.frame_mut(2).unwrap().rect = Rect::new(0.0, 0.0, 400.0, 300.0);
        m.begin_drag(2, DragKind::Resize(Edge::SouthEast));
        assert_eq!(
            m.end_drag(Point::new(10.0, 10.0), Velocity::new(-3_000.0, 0.0)),
            None
        );
    }

    // -- drag through the model -------------------------------------------

    #[test]
    fn a_drag_is_measured_from_where_it_started_not_from_the_last_frame() {
        let mut m = with_three();
        m.frame_mut(1).unwrap().rect = Rect::new(100.0, 100.0, 400.0, 300.0);
        m.begin_drag(1, DragKind::Move);
        // Push far past the left wall, then come back: an implementation that
        // accumulated per-update deltas would still be pinned to the wall here.
        m.drag_to(Point::new(-4_000.0, 0.0));
        assert_eq!(m.get(1).unwrap().rect.min_x(), 0.0);
        m.drag_to(Point::new(-40.0, 0.0));
        assert_eq!(m.get(1).unwrap().rect.min_x(), 60.0);
    }

    #[test]
    fn dragging_a_window_raises_it_and_un_maximizes_it() {
        let mut m = with_three();
        m.toggle_maximize(1);
        assert_eq!(m.get(1).unwrap().state, FrameState::Maximized);
        m.begin_drag(1, DragKind::Move);
        assert_eq!(m.active(), Some(1));
        assert_eq!(m.get(1).unwrap().state, FrameState::Normal);
    }

    #[test]
    fn a_cancelled_drag_puts_the_window_back() {
        let mut m = with_three();
        let before = m.get(2).unwrap().rect;
        m.begin_drag(2, DragKind::Move);
        m.drag_to(Point::new(120.0, 60.0));
        assert_ne!(m.get(2).unwrap().rect, before);
        m.cancel_drag();
        assert_eq!(m.get(2).unwrap().rect, before);
        assert_eq!(m.drag(), None);
    }

    // -- arrangement ------------------------------------------------------

    #[test]
    fn cascade_steps_diagonally_and_stays_on_the_desktop() {
        let mut m = with_three();
        m.cascade();
        let rects: Vec<Rect> = m.frames().iter().map(|f| f.rect).collect();
        assert_eq!(rects[0].origin, Point::ZERO);
        assert_eq!(rects[1].origin, Point::new(CASCADE_STEP, CASCADE_STEP));
        for r in &rects {
            assert!(r.max_x() <= DESKTOP.width + 0.01 && r.max_y() <= DESKTOP.height + 0.01);
        }
    }

    #[test]
    fn cascade_wraps_into_a_second_column_instead_of_walking_off_screen() {
        let mut m = desktop();
        for i in 0..24 {
            m.open(format!("W{i}"), "");
        }
        m.cascade();
        for f in m.frames() {
            assert!(
                f.rect.max_y() <= DESKTOP.height + 0.01,
                "{} fell off the bottom: {:?}",
                f.title,
                f.rect
            );
        }
    }

    #[test]
    fn tile_covers_the_desktop_without_overlapping() {
        let mut m = with_three();
        m.tile();
        let rects: Vec<Rect> = m.frames().iter().map(|f| f.rect).collect();
        let area: f32 = rects.iter().map(|r| r.size.width * r.size.height).sum();
        let (cols, rows) = grid_for(3);
        assert_eq!((cols, rows), (2, 2));
        // Three cells of a 2x2 grid: three quarters of the desktop.
        assert!((area - DESKTOP.width * DESKTOP.height * 0.75).abs() < 1.0);
        for (i, a) in rects.iter().enumerate() {
            for b in rects.iter().skip(i + 1) {
                assert!(!a.intersects(*b), "tiles overlap: {a:?} and {b:?}");
            }
        }
    }

    #[test]
    fn tile_and_cascade_both_bring_minimized_windows_back() {
        let mut m = with_three();
        m.minimize(2);
        m.tile();
        assert_eq!(
            m.get(2).unwrap().state,
            FrameState::Minimized,
            "a minimized window keeps out of the arrangement"
        );
        m.restore(2);
        m.cascade();
        assert_eq!(m.get(2).unwrap().state, FrameState::Normal);
    }

    // -- desktop size ------------------------------------------------------

    #[test]
    fn a_shrinking_desktop_pulls_windows_back_in_and_follows_maximized_ones() {
        let mut m = with_three();
        m.frame_mut(1).unwrap().rect = Rect::new(500.0, 300.0, 400.0, 300.0);
        m.toggle_maximize(3);

        assert!(m.set_desktop(Size::new(600.0, 400.0)));
        assert!(!m.set_desktop(Size::new(600.0, 400.0)), "idempotent");

        let one = m.get(1).unwrap().rect;
        assert!(one.max_x() <= 600.01 && one.max_y() <= 400.01);
        assert_eq!(m.get(3).unwrap().rect, Rect::new(0.0, 0.0, 600.0, 400.0));
    }

    #[test]
    fn maximize_remembers_the_rectangle_it_came_from() {
        let mut m = with_three();
        let before = m.get(2).unwrap().rect;
        m.toggle_maximize(2);
        assert_eq!(m.get(2).unwrap().rect.size, DESKTOP);
        m.toggle_maximize(2);
        assert_eq!(m.get(2).unwrap().rect, before);
    }
}
