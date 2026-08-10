//! Hit-testing over the render tree — **squircle-aware** (REKOMENDASI §3.6).
//!
//! The binding rule from §3.6: "corner geometry propagates into hit-testing".
//! If a button is drawn as a squircle but tested as a rectangle, each corner
//! gets a band a few pixels wide that looks empty yet is clickable — the kind
//! of flaw nobody ever reports but that makes an application feel cheap. So
//! the shape tested here is **exactly the same** superellipse that is sent to
//! the shader ([`silka_paint::Corners::contains`]).
//!
//! The traversal follows Flutter: the last child is checked first (whatever is
//! drawn on top wins), and the result is a **path from the innermost node up
//! to the root** so that events can bubble.
//!
//! ```
//! use silka_core::input::hit_test;
//! use silka_core::tree::{BoxConstraints, RenderTree};
//! use silka_core::view::{column, fixed, reconcile};
//! use silka_paint::{Point, Size};
//!
//! let mut tree = RenderTree::new();
//! reconcile(&mut tree, column([fixed(100.0, 20.0), fixed(100.0, 20.0)]));
//! tree.layout(BoxConstraints::loose(Size::new(200.0, 200.0)));
//!
//! // A default leaf does not "catch" anything (DeferToChild) — only nodes
//! // that claim to cover their area are hit.
//! assert!(hit_test(&tree, Point::new(50.0, 30.0)).is_empty());
//! ```

use silka_paint::{Corners, Point, Size};

use crate::tree::{NodeId, RenderTree};

/// The shape of a node's touch area.
///
/// It comes from [`crate::tree::RenderNode::hit_shape`] and **must** match the
/// shape that is drawn: the same theme token feeds both.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum HitShape {
    /// The node's whole box.
    #[default]
    Rect,
    /// A box with rounded corners — arcs in the Tailwind preset, squircles in
    /// the Cupertino preset, both through the same [`Corners`].
    Rounded(Corners),
}

impl HitShape {
    /// True when `local` (relative to the node's top-left corner) lies inside
    /// the shape.
    pub fn contains(self, size: Size, local: Point) -> bool {
        match self {
            HitShape::Rect => {
                local.x >= 0.0 && local.y >= 0.0 && local.x < size.width && local.y < size.height
            }
            HitShape::Rounded(corners) => corners.contains(size, local),
        }
    }
}

/// How a node behaves towards pointer events.
///
/// The counterpart of Flutter's `HitTestBehavior`, with one addition
/// ([`HitBehavior::Ignore`]) for decorative layers such as shadows or overlays
/// that must not steal clicks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HitBehavior {
    /// Joins the path **only** when one of its children is hit.
    ///
    /// The default for every structural node (padding, flex, align): they have
    /// no interest of their own in a click.
    #[default]
    DeferToChild,
    /// Covers its area: hit even without children, and blocks siblings
    /// underneath.
    Opaque,
    /// Hit, but does **not** block nodes underneath (a see-through overlay).
    Translucent,
    /// Never hit, and its children are not examined at all.
    Ignore,
}

/// One entry on a hit-test path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitEntry {
    /// The node that was hit.
    pub node: NodeId,
    /// The event position in the node's local coordinates (relative to its
    /// top-left corner).
    pub local: Point,
}

/// The result of one hit test: the path from the innermost node to the root.
///
/// The order matters — it is the order of delivery: the target first, then its
/// ancestors, until something marks the event as handled.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HitTestResult {
    entries: Vec<HitEntry>,
}

impl HitTestResult {
    /// An empty result.
    pub fn new() -> Self {
        Self::default()
    }

    /// The full path, innermost first.
    pub fn path(&self) -> &[HitEntry] {
        &self.entries
    }

    /// The innermost node hit (the event target).
    pub fn target(&self) -> Option<NodeId> {
        self.entries.first().map(|e| e.node)
    }

    /// True when no node at all was hit.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The number of nodes on the path.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when `node` is on the path.
    pub fn contains(&self, node: NodeId) -> bool {
        self.entries.iter().any(|e| e.node == node)
    }

    /// The event's local coordinates within `node`, if that node is on the
    /// path.
    pub fn local_of(&self, node: NodeId) -> Option<Point> {
        self.entries
            .iter()
            .find(|e| e.node == node)
            .map(|e| e.local)
    }

    /// Just the nodes, innermost first.
    pub fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.entries.iter().map(|e| e.node)
    }

    fn push(&mut self, node: NodeId, local: Point) {
        self.entries.push(HitEntry { node, local });
    }
}

/// What happened to one branch as it was walked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Not hit at all.
    Miss,
    /// Hit, but does not block siblings underneath.
    Pass,
    /// Hit and absorbing: siblings underneath need not be examined.
    Absorb,
}

/// Test one global point (in logical points) against the whole tree.
///
/// A node that clips its contents
/// ([`crate::tree::RenderNode::clips_children`], e.g. a viewport) stops the
/// search as soon as the point falls outside its box — that is what makes rows
/// scrolled off-screen unclickable.
pub fn hit_test(tree: &RenderTree, point: Point) -> HitTestResult {
    let mut result = HitTestResult::new();
    let root = tree.root();
    let local = Point::new(point.x - tree.offset(root).x, point.y - tree.offset(root).y);
    hit_node(tree, root, local, &mut result);
    result
}

/// Test one point against the subtree rooted at `node`; `local` is relative to
/// that node's top-left corner.
///
/// Used by overlays/popups that have their own root, and by [`hit_test`].
pub fn hit_test_subtree(
    tree: &RenderTree,
    node: NodeId,
    local: Point,
    result: &mut HitTestResult,
) -> bool {
    hit_node(tree, node, local, result) != Outcome::Miss
}

fn hit_node(tree: &RenderTree, id: NodeId, local: Point, out: &mut HitTestResult) -> Outcome {
    let Some(render) = tree.render(id) else {
        return Outcome::Miss;
    };
    let behavior = render.hit_behavior();
    if behavior == HitBehavior::Ignore {
        return Outcome::Miss;
    }
    let size = tree.size(id);
    let di_dalam = render.hit_shape().contains(size, local);
    // A node that clips its contents: outside its box, its children do not
    // exist.
    if render.clips_children() && !di_dalam {
        return Outcome::Miss;
    }

    let mut anak = Outcome::Miss;
    // Reversed: whatever is drawn last sits on top, so it wins.
    for child in tree.children(id).iter().rev() {
        let offset = tree.offset(*child);
        let child_local = Point::new(local.x - offset.x, local.y - offset.y);
        match hit_node(tree, *child, child_local, out) {
            Outcome::Absorb => {
                anak = Outcome::Absorb;
                break;
            }
            Outcome::Pass => anak = Outcome::Pass,
            Outcome::Miss => {}
        }
    }

    let diri = di_dalam && matches!(behavior, HitBehavior::Opaque | HitBehavior::Translucent);
    if anak == Outcome::Miss && !diri {
        return Outcome::Miss;
    }
    // Children first, then ourselves → the path comes out innermost-first for
    // free.
    out.push(id, local);
    if anak == Outcome::Absorb || (diri && behavior == HitBehavior::Opaque) {
        Outcome::Absorb
    } else {
        Outcome::Pass
    }
}
