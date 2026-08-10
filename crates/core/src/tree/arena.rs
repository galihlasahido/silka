//! ID-based render tree arena + the box-constraints layout engine
//! (REKOMENDASI §2, §3.4).
//!
//! Why an arena instead of plain ownership: AccessKit and Taffy are both
//! ID/arena based, so everything lines up — and we never have to fight the
//! borrow checker over a tree whose nodes point at each other (parent ⇄ child).
//! The IDs are **generational**, exactly like the signals arena: a dead slot is
//! never confused with its new occupant.
//!
//! This module is an implementation detail. Application authors never touch it;
//! *widget* authors touch it through [`RenderNode`].

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use silka_paint::{Color, Point, Rect, Scene, Size};

use crate::access::{AccessNode, AccessRole, AccessTree};
use crate::input::{CursorIcon, Event, EventCtx, FocusPolicy, HitBehavior, HitShape};
use crate::scheduler::Dirty;
use crate::signals::Key;

use super::constraints::BoxConstraints;
use super::paint::{paint_tree, PaintCache, PaintCtx};
use super::style::ItemStyle;

// ---------------------------------------------------------------------------
// IDs
// ---------------------------------------------------------------------------

static NEXT_TREE: AtomicU32 = AtomicU32::new(0);

/// The identity of one render tree (one per window).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TreeId(u32);

/// The identity of one node in the render tree arena.
///
/// Generational: once a node dies, its old ID never matches the new node that
/// takes over the same slot. The ID also carries a [`TreeId`] so nodes from
/// another window are never silently mixed in.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId {
    tree: TreeId,
    index: u32,
    generation: u32,
}

impl NodeId {
    /// The tree that owns this node.
    pub fn tree(self) -> TreeId {
        self.tree
    }

    /// The arena slot number (stable only while the node is alive).
    pub fn index(self) -> u32 {
        self.index
    }

    /// The slot generation — what tells the old occupant from the new one.
    pub fn generation(self) -> u32 {
        self.generation
    }
}

impl core::fmt::Debug for NodeId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Node(#{}v{})", self.index, self.generation)
    }
}

// ---------------------------------------------------------------------------
// The node trait
// ---------------------------------------------------------------------------

/// Automatic downcasting for every `'static` type.
///
/// It exists so [`RenderNode`] authors never have to write `as_any`
/// boilerplate; the view layer uses it to apply props to an existing node
/// ("trait object + downcast", REKOMENDASI §2).
pub trait AsAny: 'static {
    /// An `Any` reference to self.
    fn as_any(&self) -> &dyn Any;
    /// A mutable `Any` reference to self.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The behaviour of one render node: layout, relayout boundaries, and a11y
/// emission.
///
/// Its contract is exactly the three box-constraints sentences (see
/// [`BoxConstraints`]): take constraints, return a size, and **position the
/// children** via [`LayoutCtx::place_child`]. A node never knows its own
/// position.
///
/// [`RenderNode::access`] **has no default implementation**, and that is
/// deliberate: accessibility is an output of the render tree, not an add-on
/// (§3.8). A new widget that forgot to think about screen readers does not
/// compile — the only defence against the "accessibility retrofitted later"
/// failure mode that has actually proven to work (§5, item 2). A node that
/// really is pure structure says so explicitly with [`AccessRole::Container`].
pub trait RenderNode: AsAny {
    /// A type name for debugging/inspectors.
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Compute this node's own size from `constraints`, and place the children.
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size;

    /// Draw this node into the scene (REKOMENDASI §3.2).
    ///
    /// Coordinates are **local**: `(0, 0)` is the node's top-left corner and
    /// [`PaintCtx`] lifts them into absolute coordinates — the same rule as
    /// layout, where a node likewise never knows its own position.
    ///
    /// The default **draws nothing** but still descends into the content
    /// ([`PaintCtx::paint_children`]), so purely structural nodes (padding,
    /// align, containers) are not forced to write anything and their subtrees do
    /// not vanish. A node that overrides this must call
    /// [`PaintCtx::paint_children`]/[`PaintCtx::paint_child`] itself — that is
    /// where it decides what sits below and what sits above its children.
    ///
    /// The vocabulary is `silka-paint` only; wgpu types never reach this far
    /// (§3.2).
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.paint_children();
    }

    /// True when this node is **always** a relayout boundary.
    ///
    /// For nodes whose size never depends on their content — a scroll viewport
    /// is the canonical case: the content may grow to any height, the viewport
    /// box stays put.
    fn is_relayout_boundary(&self) -> bool {
        false
    }

    /// This node's style **as an item** inside a flex/grid container.
    ///
    /// The equivalent of Flutter's `ParentData`: the data lives on the child,
    /// but the parent is what reads it ([`LayoutCtx::child_layout_style`]).
    /// Ordinary nodes need not care — the default is [`ItemStyle::DEFAULT`], and
    /// only [`super::LayoutItem`] (`expanded()`/`flexible()`) fills it in.
    fn layout_style(&self) -> ItemStyle {
        ItemStyle::DEFAULT
    }

    /// The contents of the accessibility node: role, name, value, actions,
    /// state.
    ///
    /// **Must be implemented.** `bounds`, the parent, and the child list do not
    /// appear in [`AccessNode`] at all — the engine assembles them from layout
    /// results ([`RenderTree::access_tree`]), so they cannot go stale and cannot
    /// be faked by a widget.
    ///
    /// Purely structural nodes (padding, align) simply declare
    /// [`AccessRole::Container`]: assistive technology filters them out and
    /// their children take their place.
    fn access(&self, node: &mut AccessNode);

    // -- input ------------------------------------------------------------
    //
    // The four hooks below are the input contract. All have a safe default
    // ("I stay out of this"), because the majority of nodes really are
    // structural — but an interactive node that forgets to fill them in is
    // noticed immediately: it cannot be clicked and cannot be tabbed to.

    /// The node's touch shape — **this is where the squircle reaches
    /// hit-testing** (REKOMENDASI §3.6).
    ///
    /// The default is the full rectangle. A node that draws itself with rounded
    /// corners must return [`HitShape::Rounded`] with **exactly the same**
    /// [`silka_paint::Corners`] it sends to the shader — otherwise there is a
    /// band a few points wide in every corner that looks empty but is
    /// clickable.
    fn hit_shape(&self) -> HitShape {
        HitShape::Rect
    }

    /// How the node behaves towards pointer events.
    ///
    /// The default is [`HitBehavior::DeferToChild`]: a structural container does
    /// not steal clicks from its content.
    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::DeferToChild
    }

    /// True when the node clips its content to its own box.
    ///
    /// A viewport answers yes: a row that has scrolled off screen must not stay
    /// clickable just because it is still in the tree.
    fn clips_children(&self) -> bool {
        false
    }

    /// The node's role in keyboard focus navigation (Tab/Shift+Tab).
    fn focus_policy(&self) -> FocusPolicy {
        FocusPolicy::NONE
    }

    /// The cursor shape while the pointer is over this node.
    ///
    /// `None` means "whatever the node below me says". The router asks this
    /// along the hover path, so no cursor state can go stale.
    fn cursor(&self) -> Option<CursorIcon> {
        None
    }

    /// Handle one input event.
    ///
    /// A node may only change **itself**; anything concerning the outside world
    /// (focus, capture, IME, repaint requests) goes through [`EventCtx`]. The
    /// tree structure must not change from here — that is the view-diff's
    /// prerogative.
    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let _ = (ctx, event);
    }
}

impl dyn RenderNode {
    /// Downcast to a concrete node type.
    pub fn downcast_ref<T: RenderNode>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }

    /// Mutably downcast to a concrete node type.
    pub fn downcast_mut<T: RenderNode>(&mut self) -> Option<&mut T> {
        self.as_any_mut().downcast_mut::<T>()
    }
}

impl core::fmt::Debug for dyn RenderNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.type_name())
    }
}

// ---------------------------------------------------------------------------
// Text direction
// ---------------------------------------------------------------------------

/// The document's reading direction — understood by the layout system **from
/// the start** (§9.8).
///
/// RTL mirroring is not a later addition: `row` reverses its main axis and the
/// cross axis of `column` flips with it, both inside the layout engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextDirection {
    /// Left to right (Latin, CJK).
    #[default]
    Ltr,
    /// Right to left (Arabic, Hebrew).
    Rtl,
}

impl TextDirection {
    /// True when the direction is right-to-left.
    pub fn is_rtl(self) -> bool {
        matches!(self, TextDirection::Rtl)
    }
}

// ---------------------------------------------------------------------------
// Nodes & slots
// ---------------------------------------------------------------------------

struct Node {
    key: Option<Key>,
    type_id: TypeId,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    depth: u32,
    /// `None` **only** while this node is running its own layout.
    render: Option<Box<dyn RenderNode>>,
    size: Size,
    /// Position relative to the parent — always written by the parent, never by
    /// the node itself.
    offset: Point,
    constraints: Option<BoxConstraints>,
    needs_layout: bool,
    needs_paint: bool,
    boundary: bool,
    parent_uses_size: bool,
    /// May tight constraints make this node a relayout boundary?
    ///
    /// Usually yes: if the parent already forced its size, the node's content
    /// cannot change anything above it. With one exception — a flex/grid
    /// container that **derives those tight numbers from measuring the child
    /// itself** ([`super::TaffyBox`]). There the tightness is only apparent: the
    /// content changes → the measurement changes → the whole flex must be
    /// recomputed, so dirty propagation must not stop at the child.
    tight_is_boundary: bool,
    /// An exact mirror of membership in `RenderTree::dirty_boundaries`.
    ///
    /// It exists so the queue can be kept duplicate-free **without** the
    /// "already marked means already queued" early-out that once made
    /// boundaries disappear from the queue forever.
    queued: bool,
    layout_count: u32,
    /// This subtree's draw commands from the last paint pass.
    ///
    /// Only filled in at relayout boundaries (other than the root) — see
    /// [`super::paint`]. `None` means "never painted yet, or not a place to
    /// keep a cache".
    paint_cache: Option<PaintCache>,
    paint_count: u32,
}

struct Slot {
    generation: u32,
    node: Option<Node>,
}

/// The root node: passes the window constraints through unchanged to its single
/// child.
///
/// To assistive technology it is the **window**, and its name (the window title)
/// is the first thing a screen reader announces when the application takes
/// focus — which is why the label is kept here.
#[derive(Default)]
struct Root {
    label: Option<String>,
}

impl RenderNode for Root {
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
        node.role = AccessRole::Window;
        node.label.clone_from(&self.label);
    }
}

// ---------------------------------------------------------------------------
// RenderTree
// ---------------------------------------------------------------------------

/// The retained, arena-backed render tree.
///
/// Its structure is mutated **only** by the view-diff layer ([`crate::view`]);
/// layout never adds or removes nodes. That is why `depth` is always correct and
/// the layout flush order can be relied upon.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{fixed, pad, reconcile};
/// use silka_paint::{Insets, Point, Size};
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, pad(Insets::all(8.0), fixed(100.0, 20.0)));
/// tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
///
/// let luar = tree.children(tree.root())[0];
/// let dalam = tree.children(luar)[0];
/// // Sizes come up: the padding is child + insets.
/// assert_eq!(tree.size(luar), Size::new(116.0, 36.0));
/// // The parent decides where the child goes.
/// assert_eq!(tree.offset(dalam), Point::new(8.0, 8.0));
/// ```
pub struct RenderTree {
    id: TreeId,
    slots: Vec<Slot>,
    free: Vec<u32>,
    root: NodeId,
    dirty_boundaries: Vec<NodeId>,
    dirty: Dirty,
    direction: TextDirection,
    clear_color: Color,
}

impl Default for RenderTree {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderTree {
    /// A new tree containing a single root node.
    pub fn new() -> Self {
        let id = TreeId(NEXT_TREE.fetch_add(1, Ordering::Relaxed));
        let mut tree = Self {
            id,
            slots: Vec::new(),
            free: Vec::new(),
            root: NodeId {
                tree: id,
                index: 0,
                generation: 0,
            },
            dirty_boundaries: Vec::new(),
            dirty: Dirty::NONE,
            direction: TextDirection::Ltr,
            clear_color: Color::TRANSPARENT,
        };
        let root = tree.alloc(None, None, TypeId::of::<Root>(), Box::<Root>::default());
        tree.root = root;
        if let Some(n) = tree.node_mut(root) {
            n.boundary = true;
        }
        tree
    }

    /// This tree's identity.
    pub fn id(&self) -> TreeId {
        self.id
    }

    /// The root node (always alive).
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// The reading direction in force for the whole tree.
    pub fn direction(&self) -> TextDirection {
        self.direction
    }

    /// Change the reading direction; the **entire** tree needs relayout.
    ///
    /// Reading direction is a layout input that is not part of the cache key, so
    /// marking only the root is not enough — children whose constraints did not
    /// change would hold on to their cache and refuse to mirror (§9.8).
    pub fn set_direction(&mut self, direction: TextDirection) {
        if self.direction == direction {
            return;
        }
        self.direction = direction;
        self.invalidate_all();
    }

    /// Invalidate the whole layout cache — used when a global input changes
    /// (reading direction, and later scale factor/theme where they affect size).
    pub fn invalidate_all(&mut self) {
        for slot in &mut self.slots {
            if let Some(n) = slot.node.as_mut() {
                n.needs_layout = true;
                n.needs_paint = true;
            }
        }
        self.dirty.insert(Dirty::LAYOUT | Dirty::PAINT);
        let root = self.root;
        self.enqueue_boundary(root);
    }

    // -- inspection -------------------------------------------------------

    /// The number of live nodes (including the root).
    pub fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    /// True when only the root is present.
    pub fn is_empty(&self) -> bool {
        self.len() <= 1
    }

    /// True when `id` still points at a live node in this tree.
    pub fn contains(&self, id: NodeId) -> bool {
        self.node(id).is_some()
    }

    /// A node's parent (`None` for the root or a dead id).
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id)?.parent
    }

    /// A node's children, in order.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.node(id).map(|n| n.children.as_slice()).unwrap_or(&[])
    }

    /// A node's identity key (from the view that built it).
    pub fn key(&self, id: NodeId) -> Option<Key> {
        self.node(id)?.key.clone()
    }

    /// The view type that built this node — used by diffing to decide between
    /// "update in place" and "replace".
    pub fn type_id_of(&self, id: NodeId) -> Option<TypeId> {
        Some(self.node(id)?.type_id)
    }

    /// Depth from the root (the root is 0).
    pub fn depth(&self, id: NodeId) -> Option<u32> {
        Some(self.node(id)?.depth)
    }

    /// The size produced by the last layout.
    pub fn size(&self, id: NodeId) -> Size {
        self.node(id).map(|n| n.size).unwrap_or(Size::ZERO)
    }

    /// The position relative to the parent (set by the parent).
    pub fn offset(&self, id: NodeId) -> Point {
        self.node(id).map(|n| n.offset).unwrap_or(Point::ZERO)
    }

    /// The absolute position within the tree.
    pub fn global_offset(&self, id: NodeId) -> Point {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(n) = self.node(c) else { break };
            x += n.offset.x;
            y += n.offset.y;
            cur = n.parent;
        }
        Point::new(x, y)
    }

    /// A node's absolute box — this is the `bounds` used by a11y and
    /// hit-testing.
    pub fn bounds(&self, id: NodeId) -> Rect {
        Rect::from_origin_size(self.global_offset(id), self.size(id))
    }

    /// The constraints used by the last layout.
    pub fn constraints(&self, id: NodeId) -> Option<BoxConstraints> {
        self.node(id)?.constraints
    }

    /// How many times this node actually ran its layout.
    ///
    /// It exists to prove the promise "layout work is bounded by relayout
    /// boundaries" — used by unit tests and the inspector, not by framework
    /// logic.
    pub fn layout_count(&self, id: NodeId) -> u32 {
        self.node(id).map(|n| n.layout_count).unwrap_or(0)
    }

    /// True when the node is waiting for layout.
    pub fn needs_layout(&self, id: NodeId) -> bool {
        self.node(id).map(|n| n.needs_layout).unwrap_or(false)
    }

    /// True when the node is waiting to be repainted.
    pub fn needs_paint(&self, id: NodeId) -> bool {
        self.node(id).map(|n| n.needs_paint).unwrap_or(false)
    }

    /// True when the node was a relayout boundary as of the last layout.
    pub fn is_relayout_boundary(&self, id: NodeId) -> bool {
        self.node(id).map(|n| n.boundary).unwrap_or(false)
    }

    /// How many boundaries are waiting in the relayout queue.
    pub fn pending_boundaries(&self) -> usize {
        self.dirty_boundaries.len()
    }

    /// A node's behaviour.
    pub fn render(&self, id: NodeId) -> Option<&dyn RenderNode> {
        self.node(id)?.render.as_deref()
    }

    /// A node's behaviour, mutable.
    pub fn render_mut(&mut self, id: NodeId) -> Option<&mut dyn RenderNode> {
        self.node_mut(id)?.render.as_deref_mut()
    }

    /// A node's behaviour, already downcast to a concrete type.
    pub fn node_ref<T: RenderNode>(&self, id: NodeId) -> Option<&T> {
        self.render(id)?.downcast_ref::<T>()
    }

    /// A node's behaviour, already downcast to a concrete type, mutable.
    ///
    /// Changes made through this path do **not** mark anything dirty — call
    /// [`RenderTree::mark_needs_layout`]/[`RenderTree::mark_needs_paint`]
    /// yourself. The normal path is the view-diff, which already does.
    pub fn node_mut_ref<T: RenderNode>(&mut self, id: NodeId) -> Option<&mut T> {
        self.render_mut(id)?.downcast_mut::<T>()
    }

    /// Temporarily take a node's behaviour out of the arena.
    ///
    /// Used by input routing for the same reason as layout: while a node handles
    /// an event it must not be able to see (let alone mutate) itself through the
    /// tree. It must be put back with [`RenderTree::put_render`].
    pub(crate) fn take_render(&mut self, id: NodeId) -> Option<Box<dyn RenderNode>> {
        self.node_mut(id)?.render.take()
    }

    /// Put back a node's behaviour taken by [`RenderTree::take_render`].
    pub(crate) fn put_render(&mut self, id: NodeId, render: Box<dyn RenderNode>) {
        if let Some(node) = self.node_mut(id) {
            node.render = Some(render);
        }
    }

    // -- structural mutation ----------------------------------------------

    /// Insert a new child at `index` (clamped to the current child count).
    ///
    /// Only the view-diff layer may call this.
    pub fn insert_child(
        &mut self,
        parent: NodeId,
        index: usize,
        key: Option<Key>,
        type_id: TypeId,
        render: Box<dyn RenderNode>,
    ) -> NodeId {
        assert!(self.contains(parent), "induk {parent:?} sudah mati");
        let child = self.alloc(Some(parent), key, type_id, render);
        let depth = self.node(parent).map(|n| n.depth).unwrap_or(0) + 1;
        if let Some(n) = self.node_mut(child) {
            n.depth = depth;
        }
        let n = self.node_mut(parent).expect("induk hidup");
        let at = index.min(n.children.len());
        n.children.insert(at, child);
        self.mark_needs_layout(parent);
        child
    }

    /// Remove a node together with all its descendants; returns how many nodes
    /// were removed.
    ///
    /// The root cannot be removed (it panics) — a tree always has a root.
    pub fn remove_subtree(&mut self, id: NodeId) -> usize {
        assert!(id != self.root, "akar render tree tidak boleh dibuang");
        let Some(node) = self.node(id) else { return 0 };
        let parent = node.parent;
        if let Some(p) = parent {
            if let Some(pn) = self.node_mut(p) {
                pn.children.retain(|c| *c != id);
            }
            self.mark_needs_layout(p);
        }
        let mut stack = vec![id];
        let mut removed = 0;
        while let Some(cur) = stack.pop() {
            let Some(idx) = self.index_of(cur) else {
                continue;
            };
            let node = self.slots[idx].node.take().expect("slot hidup");
            stack.extend(node.children);
            self.slots[idx].generation = self.slots[idx].generation.wrapping_add(1);
            self.free.push(idx as u32);
            removed += 1;
        }
        self.dirty.insert(Dirty::LAYOUT | Dirty::PAINT);
        removed
    }

    /// Reorder `parent`'s children into `order`.
    ///
    /// `order` must contain exactly the children that exist right now (same
    /// count, same set) — a violation panics rather than silently corrupting the
    /// tree.
    pub fn set_children(&mut self, parent: NodeId, order: &[NodeId]) {
        let current = self.children(parent);
        assert_eq!(
            current.len(),
            order.len(),
            "set_children harus memuat semua anak {parent:?}"
        );
        if current == order {
            return;
        }
        for id in order {
            assert_eq!(
                self.parent(*id),
                Some(parent),
                "{id:?} bukan anak {parent:?}"
            );
        }
        if let Some(n) = self.node_mut(parent) {
            n.children.clear();
            n.children.extend_from_slice(order);
        }
        self.mark_needs_layout(parent);
    }

    // -- dirty ------------------------------------------------------------

    /// Mark a node as needing relayout.
    ///
    /// The mark propagates upwards **until the nearest relayout boundary**, and
    /// it is that boundary which enters the queue. This is what keeps a small
    /// change inside a scroll view from ever forcing the whole window to
    /// relayout (§3.4).
    pub fn mark_needs_layout(&mut self, id: NodeId) {
        self.dirty.insert(Dirty::LAYOUT | Dirty::PAINT);
        // Paint propagation has its own rule (all the way to the root, without
        // stopping at relayout boundaries) — see [`RenderTree::mark_needs_paint`].
        self.mark_needs_paint(id);
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(node) = self.node_mut(c) else { return };
            // Deliberately **not** stopping just because `needs_layout` is
            // already true: that flag does not prove the boundary above is still
            // queued (the queue may already have been drained by an earlier
            // pass). Stopping there means a boundary that is never worked on
            // again, and a `needs_layout` that can never be cleared. The walk
            // stays short anyway: propagation always stops at the nearest
            // boundary.
            node.needs_layout = true;
            node.needs_paint = true;
            let boundary = node.boundary;
            let parent = node.parent;
            if boundary || parent.is_none() {
                self.enqueue_boundary(c);
                return;
            }
            cur = parent;
        }
    }

    /// Put a boundary into the relayout queue, exactly once.
    ///
    /// `Node::queued` mirrors queue membership, so repeated calls never pile up
    /// duplicates.
    fn enqueue_boundary(&mut self, id: NodeId) {
        let Some(node) = self.node_mut(id) else {
            return;
        };
        if node.queued {
            return;
        }
        node.queued = true;
        self.dirty_boundaries.push(id);
    }

    /// Add a dirty reason to the tree without touching any node.
    ///
    /// For reasons that are **not about geometry**, and there is only one such
    /// reason: [`Dirty::ANIMATION`]. A spring that has just been re-aimed (a
    /// dialog's `open` prop changed through the view-diff, a button entering its
    /// loading state) has not moved a single pixel this frame — what it needs is
    /// the **next** frame. Without this door that reason gets lost in transit and
    /// the animation freezes until the next input event arrives.
    pub fn mark_dirty(&mut self, dirty: Dirty) {
        self.dirty.insert(dirty);
    }

    /// Mark a node as needing a repaint (without layout).
    ///
    /// The mark propagates **all the way to the root**, and that is not waste:
    /// the paint pass stores a subtree's draw commands at relayout boundaries
    /// ([`RenderTree::paint`]). If the mark stopped halfway, a boundary above the
    /// changed node would believe itself clean and replay the old drawing — the
    /// change would vanish without a sound. "Clean" has to really mean "nothing
    /// inside me changed".
    ///
    /// Repaint boundaries (layers whose size does not propagate) belong to the
    /// layer/offscreen milestone, not here.
    pub fn mark_needs_paint(&mut self, id: NodeId) {
        self.dirty.insert(Dirty::PAINT);
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(node) = self.node_mut(c) else { return };
            // Deliberately **not** stopping just because `needs_paint` is already
            // true: that flag can come from another path (a newly allocated node,
            // a size change during layout) that did not propagate upwards.
            // Stopping there means an ancestor that believes itself clean. The
            // walk is short: one straight line to the root.
            node.needs_paint = true;
            cur = node.parent;
        }
    }

    /// Take the accumulated dirty reasons and clear them.
    ///
    /// This is what gets wired to
    /// [`crate::scheduler::FrameScheduler::request`] — rendering stays
    /// **dirty-driven only** (§3.5).
    pub fn take_dirty(&mut self) -> Dirty {
        core::mem::replace(&mut self.dirty, Dirty::NONE)
    }

    /// The accumulated dirty reasons, without clearing them.
    pub fn dirty(&self) -> Dirty {
        self.dirty
    }

    /// Mark the whole tree as painted.
    pub fn clear_paint(&mut self) {
        for slot in &mut self.slots {
            if let Some(n) = slot.node.as_mut() {
                n.needs_paint = false;
            }
        }
    }

    // -- paint ------------------------------------------------------------

    /// The frame's background colour — **always a theme token**
    /// (`theme.color.background`).
    pub fn clear_color(&self) -> Color {
        self.clear_color
    }

    /// Change the frame's background colour (e.g. after dark mode flips).
    ///
    /// Changing it marks the whole tree as needing a repaint: the background
    /// changed because the preset/appearance changed, and that changes every
    /// token colour already baked into the paint caches.
    pub fn set_clear_color(&mut self, color: Color) {
        if self.clear_color == color {
            return;
        }
        self.clear_color = color;
        self.dirty.insert(Dirty::PAINT);
        for slot in &mut self.slots {
            if let Some(n) = slot.node.as_mut() {
                n.needs_paint = true;
            }
        }
    }

    /// **The paint pass**: assemble this frame's [`Scene`] from the render tree
    /// (§3.2).
    ///
    /// A peer of layout and a11y. Clean subtrees are **not** re-run: their draw
    /// commands are copied from the cache at relayout boundaries, and
    /// `needs_paint` is cleared across the whole tree afterwards.
    ///
    /// Must be called after layout: the absolute positions used here come from
    /// layout results, exactly like the a11y `bounds`.
    ///
    /// ```
    /// use silka_core::tree::{BoxConstraints, RenderTree};
    /// use silka_core::view::{fixed, pad, reconcile};
    /// use silka_paint::{Color, Insets, Size};
    ///
    /// let mut tree = RenderTree::new();
    /// reconcile(
    ///     &mut tree,
    ///     pad(Insets::all(8.0), fixed(100.0, 20.0).background(Color::WHITE)),
    /// );
    /// tree.layout(BoxConstraints::loose(Size::new(200.0, 200.0)));
    /// let scene = tree.paint();
    /// assert_eq!(scene.len(), 1);
    /// ```
    pub fn paint(&mut self) -> Scene {
        let mut scene = Scene::new(self.clear_color);
        self.paint_into(&mut scene);
        scene
    }

    /// The variant of [`RenderTree::paint`] that reuses a scene buffer.
    ///
    /// This is the per-frame path: draw-command allocations are kept across
    /// frames, so repainting never touches the allocator.
    pub fn paint_into(&mut self, scene: &mut Scene) {
        scene.reset(self.clear_color);
        paint_tree(self, scene);
        self.clear_paint();
    }

    /// How many times this node actually ran its paint.
    ///
    /// The twin of [`RenderTree::layout_count`], and it exists for the same
    /// reason: to prove that clean subtrees really are skipped. Not framework
    /// logic.
    pub fn paint_count(&self, id: NodeId) -> u32 {
        self.node(id).map(|n| n.paint_count).unwrap_or(0)
    }

    /// The geometry the paint pass needs: relative offset, size, dirty flag, and
    /// boundary status.
    pub(super) fn paint_geometry(&self, id: NodeId) -> Option<(Point, Size, bool, bool)> {
        let n = self.node(id)?;
        Some((n.offset, n.size, n.needs_paint, n.boundary))
    }

    /// This subtree's draw commands from the previous frame, if any.
    pub(super) fn paint_cache(&self, id: NodeId) -> Option<&PaintCache> {
        self.node(id)?.paint_cache.as_ref()
    }

    /// Store the result of a node's paint pass and record that it really did
    /// draw.
    pub(super) fn finish_paint(&mut self, id: NodeId, cache: Option<PaintCache>) {
        if let Some(n) = self.node_mut(id) {
            n.paint_cache = cache;
            n.paint_count = n.paint_count.saturating_add(1);
        }
    }

    // -- layout -----------------------------------------------------------

    /// A full layout from the root with the window constraints.
    ///
    /// Called on the first frame and whenever the surface size changes. Same
    /// constraints + clean tree = no work at all.
    ///
    /// After the full pass the relayout queue is drained too
    /// ([`RenderTree::flush_layout`]): a queued boundary may have been skipped
    /// because its ancestor got a cache hit, and after this
    /// [`RenderTree::pending_boundaries`] is always zero for boundaries that
    /// have already been laid out once.
    pub fn layout(&mut self, constraints: BoxConstraints) -> Size {
        let root = self.root;
        let size = self.layout_node(root, constraints, true, true);
        // The queue **must not simply be discarded**. A full pass stops early on
        // every cache hit, so a boundary queued below a clean ancestor (e.g. a
        // scroll view inside a tightly sized box, while what actually changed is
        // its sibling) is never touched. Dropping its entry would leave its
        // `needs_layout` set forever and kill the scroll view silently.
        self.flush_layout();
        size
    }

    /// Relayout **only** the dirty subtrees, using the constraints stored at each
    /// boundary. Returns how many boundaries were processed.
    ///
    /// A boundary guarantees its own size does not change, so nothing has to
    /// propagate upwards.
    pub fn flush_layout(&mut self) -> usize {
        let mut queue = core::mem::take(&mut self.dirty_boundaries);
        for id in &queue {
            if let Some(n) = self.node_mut(*id) {
                n.queued = false;
            }
        }
        // Ancestors first: one boundary may clean up the boundaries below it, and
        // whatever is already clean is simply skipped.
        queue.sort_by_key(|id| self.depth(*id).unwrap_or(0));
        let mut done = 0;
        for id in queue {
            let Some((needs_layout, constraints, parent_uses_size, tight_is_boundary)) =
                self.node(id).map(|n| {
                    (
                        n.needs_layout,
                        n.constraints,
                        n.parent_uses_size,
                        n.tight_is_boundary,
                    )
                })
            else {
                continue;
            };
            if !needs_layout {
                continue;
            }
            let Some(constraints) = constraints else {
                // Never laid out before: it has to go through a full layout from
                // the root, so its entry goes back into the queue.
                self.enqueue_boundary(id);
                continue;
            };
            self.layout_node(id, constraints, parent_uses_size, tight_is_boundary);
            done += 1;
        }
        done
    }

    /// The normal per-frame layout path: a full layout when the constraints
    /// changed or the root is dirty, otherwise just the dirty subtrees.
    pub fn perform_layout(&mut self, constraints: BoxConstraints) -> Size {
        let root = self.root;
        let perlu_penuh =
            self.needs_layout(root) || self.constraints(root) != Some(constraints.normalized());
        if perlu_penuh {
            self.layout(constraints)
        } else {
            self.flush_layout();
            self.size(root)
        }
    }

    fn layout_node(
        &mut self,
        id: NodeId,
        constraints: BoxConstraints,
        parent_uses_size: bool,
        tight_is_boundary: bool,
    ) -> Size {
        let constraints = constraints.normalized();
        let (is_root, intrinsic) = {
            let node = self
                .node(id)
                .unwrap_or_else(|| panic!("layout node mati: {id:?}"));
            let render = node.render.as_ref().unwrap_or_else(|| {
                panic!("{id:?} sedang melakukan layout — layout rekursif tidak diizinkan")
            });
            (node.parent.is_none(), render.is_relayout_boundary())
        };
        // The Flutter rule: a boundary when our own size cannot be affected by
        // our content, or when the parent does not use our size anyway.
        // `tight_is_boundary` is the exception used by flex/grid containers — see
        // the `Node::tight_is_boundary` field.
        let boundary = is_root
            || intrinsic
            || (tight_is_boundary && constraints.is_tight())
            || !parent_uses_size;

        {
            let node = self.node_mut(id).expect("node hidup");
            if !node.needs_layout
                && node.constraints == Some(constraints)
                && node.boundary == boundary
            {
                return node.size;
            }
            node.boundary = boundary;
            node.parent_uses_size = parent_uses_size;
            node.tight_is_boundary = tight_is_boundary;
            node.constraints = Some(constraints);
        }

        let mut render = self
            .node_mut(id)
            .expect("node hidup")
            .render
            .take()
            .expect("render node tersedia");
        let size = {
            let mut ctx = LayoutCtx {
                tree: self,
                node: id,
            };
            render.layout(&mut ctx, constraints)
        };
        let size = constraints.constrain(size);
        debug_assert!(
            size.width.is_finite() && size.height.is_finite(),
            "{id:?} ({}) memilih ukuran tak hingga di bawah constraints tanpa batas",
            render.type_name()
        );

        let node = self
            .node_mut(id)
            .expect("struktur pohon tidak boleh berubah selama layout");
        node.render = Some(render);
        node.size = size;
        node.needs_layout = false;
        node.layout_count = node.layout_count.saturating_add(1);
        // A node that actually ran its layout **always** needs a repaint: we only
        // got here because it was dirty or its constraints changed, and either can
        // change its size. Marking it from here closes the paths that never go
        // through `mark_needs_layout` at all — e.g. a child relaid out purely
        // because its parent changed.
        self.mark_needs_paint(id);
        size
    }

    // -- a11y -------------------------------------------------------------

    /// Emit the whole accessibility tree — **a peer pass to layout and paint**,
    /// not an afterthought layer (§3.8).
    ///
    /// Each node's `bounds` comes from layout results, so what a screen reader
    /// announces and what gets drawn cannot disagree. An [`AccessNode::hidden`]
    /// node disappears together with all its descendants.
    ///
    /// `focus` is **deliberately passed in by the caller** rather than stored in
    /// the tree: the rightful owner of focus is [`crate::input::FocusManager`],
    /// and two places storing focus means sooner or later the two disagree.
    /// Usually `router.focus().focused()`; `None` means the window itself holds
    /// focus (the AccessKit rule).
    pub fn access_tree(&self, focus: Option<NodeId>) -> AccessTree {
        AccessTree::emit(self, focus)
    }

    /// The window's name for assistive technology (usually the window title).
    ///
    /// It is the first thing a screen reader announces when the application
    /// takes focus, so it belongs to the a11y tree — not merely to titlebar
    /// decoration.
    pub fn set_root_label(&mut self, label: impl Into<String>) {
        let root = self.root;
        let label = label.into();
        if let Some(r) = self.node_mut_ref::<Root>(root) {
            if r.label.as_deref() != Some(label.as_str()) {
                r.label = Some(label);
                self.dirty.insert(Dirty::PAINT);
            }
        }
    }

    // -- internals --------------------------------------------------------

    fn alloc(
        &mut self,
        parent: Option<NodeId>,
        key: Option<Key>,
        type_id: TypeId,
        render: Box<dyn RenderNode>,
    ) -> NodeId {
        let node = Node {
            key,
            type_id,
            parent,
            children: Vec::new(),
            depth: 0,
            render: Some(render),
            size: Size::ZERO,
            offset: Point::ZERO,
            constraints: None,
            needs_layout: true,
            needs_paint: true,
            boundary: false,
            parent_uses_size: true,
            tight_is_boundary: true,
            queued: false,
            layout_count: 0,
            paint_cache: None,
            paint_count: 0,
        };
        match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                slot.node = Some(node);
                NodeId {
                    tree: self.id,
                    index,
                    generation: slot.generation,
                }
            }
            None => {
                let index = self.slots.len() as u32;
                self.slots.push(Slot {
                    generation: 0,
                    node: Some(node),
                });
                NodeId {
                    tree: self.id,
                    index,
                    generation: 0,
                }
            }
        }
    }

    fn index_of(&self, id: NodeId) -> Option<usize> {
        if id.tree != self.id {
            return None;
        }
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation || slot.node.is_none() {
            return None;
        }
        Some(id.index as usize)
    }

    fn node(&self, id: NodeId) -> Option<&Node> {
        let idx = self.index_of(id)?;
        self.slots[idx].node.as_ref()
    }

    fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let idx = self.index_of(id)?;
        self.slots[idx].node.as_mut()
    }
}

impl core::fmt::Debug for RenderTree {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RenderTree")
            .field("id", &self.id)
            .field("nodes", &self.len())
            .field("direction", &self.direction)
            .field("dirty", &self.dirty)
            .field("pending_boundaries", &self.dirty_boundaries.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// LayoutCtx
// ---------------------------------------------------------------------------

/// Restricted access to the tree while a node runs its layout.
///
/// It deliberately offers **no** structural mutation: the tree only changes
/// through the view-diff. What a node may do is lay out its children and place
/// them.
pub struct LayoutCtx<'a> {
    tree: &'a mut RenderTree,
    node: NodeId,
}

impl LayoutCtx<'_> {
    /// The node currently being laid out.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// The reading direction in force (RTL mirroring, §9.8).
    pub fn direction(&self) -> TextDirection {
        self.tree.direction
    }

    /// This node's children.
    pub fn children(&self) -> &[NodeId] {
        self.tree.children(self.node)
    }

    /// The number of children.
    pub fn child_count(&self) -> usize {
        self.tree.children(self.node).len()
    }

    /// The child at `index`.
    ///
    /// Panics when out of range — child indices always come from
    /// [`LayoutCtx::child_count`].
    pub fn child(&self, index: usize) -> NodeId {
        self.tree.children(self.node)[index]
    }

    /// Lay out a child whose size may influence our own.
    pub fn layout_child(&mut self, child: NodeId, constraints: BoxConstraints) -> Size {
        debug_assert_eq!(
            self.tree.parent(child),
            Some(self.node),
            "hanya boleh melayout anak sendiri"
        );
        self.tree.layout_node(child, constraints, true, true)
    }

    /// Lay out a child with constraints that **were derived from measuring that
    /// very child**.
    ///
    /// There is only one difference from [`LayoutCtx::layout_child`], but it
    /// matters: tight constraints here do **not** make the child a relayout
    /// boundary. A flex/grid container ([`super::TaffyBox`]) hands its child an
    /// exact size after first asking it "how big do you want to be?" — if the
    /// child's content changes, the answer changes and the whole flex has to be
    /// recomputed. Making the child a boundary there would mean that change never
    /// reaches its container and layout freezes silently.
    pub fn layout_child_measured(&mut self, child: NodeId, constraints: BoxConstraints) -> Size {
        debug_assert_eq!(
            self.tree.parent(child),
            Some(self.node),
            "hanya boleh melayout anak sendiri"
        );
        self.tree.layout_node(child, constraints, true, false)
    }

    /// A child's style as a flex/grid item ([`RenderNode::layout_style`]).
    pub fn child_layout_style(&self, child: NodeId) -> ItemStyle {
        self.tree
            .render(child)
            .map(|n| n.layout_style())
            .unwrap_or(ItemStyle::DEFAULT)
    }

    /// Lay out a child whose size does **not** influence our own.
    ///
    /// The child automatically becomes a relayout boundary: changes inside it
    /// stop there. Its size is still returned so it can be placed, but must not
    /// be used to compute our own size.
    pub fn layout_child_boundary(&mut self, child: NodeId, constraints: BoxConstraints) -> Size {
        debug_assert_eq!(
            self.tree.parent(child),
            Some(self.node),
            "hanya boleh melayout anak sendiri"
        );
        self.tree.layout_node(child, constraints, false, true)
    }

    /// A child's size from the last layout.
    pub fn child_size(&self, child: NodeId) -> Size {
        self.tree.size(child)
    }

    /// **The parent decides the position**: place a child relative to this
    /// node's top-left corner.
    pub fn place_child(&mut self, child: NodeId, offset: Point) {
        debug_assert_eq!(
            self.tree.parent(child),
            Some(self.node),
            "hanya boleh menempatkan anak sendiri"
        );
        let berubah = match self.tree.node_mut(child) {
            Some(n) if n.offset != offset => {
                n.offset = offset;
                true
            }
            _ => false,
        };
        if berubah {
            // Moving a child shifts all of its descendants; what needs marking is
            // the path upwards, because the ancestors' paint caches contain this
            // child's drawing. The descendants take care of themselves: their
            // caches record the absolute position they were built at.
            self.tree.mark_needs_paint(child);
        }
    }
}

// ---------------------------------------------------------------------------
// Utilities for the view layer
// ---------------------------------------------------------------------------

/// A `key -> node` map for a node's children; used by keyed diffing.
///
/// Panics when two siblings share a key: the map would swallow one of them, and
/// the swallowed node would never be matched nor removed — which only surfaces a
/// frame later as an arena invariant blowing up. Better to be loud at the site
/// of the mistake (§9.7).
pub(crate) fn keyed_children(tree: &RenderTree, parent: NodeId) -> HashMap<Key, NodeId> {
    let mut map = HashMap::new();
    for id in tree.children(parent) {
        if let Some(key) = tree.key(*id) {
            if let Some(sebelumnya) = map.insert(key.clone(), *id) {
                panic!(
                    "kunci ganda di antara anak {parent:?}: {key:?} dipakai {sebelumnya:?} \
                     dan {id:?} — kunci wajib unik di antara saudara"
                );
            }
        }
    }
    map
}
