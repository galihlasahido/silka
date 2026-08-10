//! `component()` — the one place where a signals scope meets a render node.
//!
//! A component is a pair: a **scope** in [`crate::signals`] (owner of its
//! `use_signal` state and its subscriptions) and **one anchor node** in
//! [`crate::tree`] (where its build result is diffed). The anchor node is
//! transparent — it merely forwards constraints to its single child — so adding
//! a component does not change layout results at all.
//!
//! Why the anchor has to exist: `drain_dirty()` hands back a `ScopeId`, and a
//! per-component rebuild needs to know **under which node** its new view is
//! diffed. Without that node the only option left is diffing the whole tree
//! from the root on every signal change — precisely what §2.5 set out to avoid.

use silka_paint::{Point, Size};

use crate::access::{AccessNode, AccessRole};
use crate::scheduler::Dirty;
use crate::signals::{current_scope, scope as masuk_scope, Key, ScopeId};
use crate::tree::{BoxConstraints, LayoutCtx, RenderNode};
use crate::view::{View, ViewNode};

use super::host::{current_host, BuildCtx, ComponentBuilder};

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// A component's anchor node: transparent to layout, a marker for rebuilds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentBox {
    /// The signals scope that builds this node's contents.
    pub scope: ScopeId,
}

impl RenderNode for ComponentBox {
    fn type_name(&self) -> &'static str {
        "Component"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        // `layout_child_measured`, not `layout_child`: this node is
        // transparent, so the tightness of the constraints passing through here
        // is **not** its own. If it made its child a relayout boundary, changes
        // inside the component would never reach the flex/grid container above
        // it (see `LayoutCtx::layout_child_measured`).
        let size = ctx.layout_child_measured(child, constraints);
        ctx.place_child(child, Point::ZERO);
        size
    }

    /// Purely structural: assistive technology filters it out and its child
    /// takes its place (§3.8). Component boundaries are a framework concern,
    /// not something a screen reader should announce.
    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props for a component's anchor node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentProps {
    scope: ScopeId,
}

impl ComponentProps {
    /// The scope these props anchor.
    pub fn scope(&self) -> ScopeId {
        self.scope
    }
}

impl ViewNode for ComponentProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ComponentBox { scope: self.scope })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ComponentBox>()
            .expect("tipe view sama berarti tipe render node sama");
        if n.scope == self.scope {
            return Dirty::NONE;
        }
        // The same key with a different scope means the old scope died and a
        // new tenant took its place — the contents must be treated as entirely
        // new.
        n.scope = self.scope;
        Dirty::LAYOUT | Dirty::PAINT
    }
}

/// Build one `key`-identified component and turn the result into a [`View`].
///
/// This is the Dart-style shape for "a piece of UI that owns its state and
/// rebuilds on its own" (§2.5):
///
/// ```
/// use silka_core::app::{app, component};
/// use silka_core::signals::use_signal;
/// use silka_core::view::{column, fixed};
///
/// let mut ui = app(|_cx| {
///     column([
///         component("kiri", |_| fixed(40.0, 20.0).into()),
///         component("kanan", |_| {
///             let n = use_signal(|| 1i32);
///             fixed(40.0, 20.0 * n.get() as f32).into()
///         }),
///     ])
///     .into()
/// })
/// .sized(200.0, 200.0);
/// ui.frame();
/// ```
///
/// Three things happen at once, and all three are required:
///
/// 1. **Enter the scope** ([`crate::signals::scope`]) — the same `key` on the
///    next build = the same scope = the same state, even if its position moved.
/// 2. **Build the body right now.** This is what honors the
///    [`crate::signals::Runtime::drain_dirty`] contract: rebuilding a scope
///    **re-enters every retained child**, which keeps pruning descendants from
///    the dirty list sound.
/// 3. **Store the closure** in the host registry, so the next frame can rebuild
///    **only** this component.
///
/// Panics when called outside the build of a [`crate::app::AppRuntime`].
pub fn component<F>(key: impl Into<Key>, body: F) -> View
where
    F: Fn(&BuildCtx) -> View + 'static,
{
    let host = current_host().expect(
        "component() hanya boleh dipanggil saat komponen dibangun (di dalam AppRuntime::frame)",
    );
    let key: Key = key.into();
    let builder: ComponentBuilder = std::rc::Rc::new(body);

    let cx = BuildCtx::new(host.clone());
    let untuk_scope = builder.clone();
    let (scope, isi) = masuk_scope(key.clone(), move || {
        let id = current_scope().expect("scope() baru saja memasuki scope anak");
        (id, untuk_scope(&cx))
    });

    host.register(scope, builder);

    crate::view::Builder::new(ComponentProps { scope })
        .key(key)
        .child(isi)
        .into()
}
