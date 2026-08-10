//! The lifecycle owner: one place that holds the signals runtime, the root view
//! builder closure, the render tree, and the frame scheduler.
//!
//! This module is the **seam** that was deliberately left empty until now:
//! [`crate::signals::Runtime::drain_dirty`] finally has a caller, and its
//! contract is honored exactly — rebuilding a scope **re-enters** every
//! retained child, because [`super::component`] builds its children eagerly
//! inside [`crate::signals::scope`].

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Instant;

use silka_paint::{Color, Scene, Size};

use crate::access::AccessTree;
use crate::animation::{AnimationDriver, Motion, Tick};
use crate::input::{Event, InputRouter, Response};
use crate::scheduler::{Dirty, FrameScheduler, FrameTiming, Wake};
use crate::signals::{current_scope, Runtime, ScopeId};
use crate::tree::{BoxConstraints, NodeId, RenderTree, TextDirection};
use crate::view::{reconcile_children, DiffStats, View};

use super::component::ComponentBox;

/// A component's builder closure: run **inside** its own scope.
pub(super) type ComponentBuilder = Rc<dyn Fn(&BuildCtx) -> View>;

/// Tells the shell that the scheduler accepted a frame request.
type WakeFn = Rc<dyn Fn(Wake)>;

// ---------------------------------------------------------------------------
// Env
// ---------------------------------------------------------------------------

/// Application-level injected values that every component can read while it
/// builds.
///
/// The contents are usually a **signal** rather than a plain value: the shell
/// puts a `Signal<Theme>` here once and updates it whenever OS dark mode
/// changes — and only the components that actually read the theme are rebuilt
/// (§2.7, §3.5). Injecting a raw value is legal too, it simply is not reactive.
///
/// Keyed by type: one type = one injected value. That is enough, and it closes
/// the whole "which one do I get" class of bugs that string keys invite.
#[derive(Default)]
pub struct Env {
    map: HashMap<TypeId, Box<dyn Any>>,
}

impl Env {
    /// An empty env.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inject a value; any earlier value of the same type is replaced.
    pub fn insert<T: 'static>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Borrow the injected value of type `T`.
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map.get(&TypeId::of::<T>())?.downcast_ref::<T>()
    }

    /// True when a value of type `T` has been injected.
    pub fn contains<T: 'static>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    /// How many values are injected.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when nothing at all has been injected.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl core::fmt::Debug for Env {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Env").field("len", &self.map.len()).finish()
    }
}

/// The scale factor of the display this application draws on (2.0 on Retina).
///
/// A standard [`Env`] injected value, and no luxury: **text must be rasterized
/// at the real screen resolution** (§3.3), so any component that measures or
/// draws text needs the number. The shell injects it as a
/// `Signal<ScaleFactor>` and refreshes it every frame, so moving the window to
/// another monitor rebuilds only the components that actually read it (§2.7,
/// §3.5).
///
/// Logical sizes never change because of it — only the resolution of the glyph
/// bitmaps does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScaleFactor(pub f32);

impl ScaleFactor {
    /// One physical pixel per logical point — a non-Retina display, and the
    /// default before the shell reports anything.
    pub const ONE: ScaleFactor = ScaleFactor(1.0);

    /// The value, always finite and positive.
    pub fn get(self) -> f32 {
        if self.0.is_finite() && self.0 > 0.0 {
            self.0
        } else {
            1.0
        }
    }
}

impl Default for ScaleFactor {
    fn default() -> Self {
        Self::ONE
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Maps scope → how to rebuild it, and where the result attaches.
///
/// This is what makes **per-component** rebuilds possible: `drain_dirty()`
/// hands back a `ScopeId`, and these two maps translate it into "which closure
/// to run" and "under which render node the result is diffed".
#[derive(Default)]
struct Registry {
    builders: HashMap<ScopeId, ComponentBuilder>,
    anchors: HashMap<ScopeId, NodeId>,
}

// ---------------------------------------------------------------------------
// HostShared
// ---------------------------------------------------------------------------

/// The part of [`AppRuntime`] that must be reachable from inside a build.
///
/// [`super::component`] is called in the middle of the user's closure, far from
/// any `&mut AppRuntime`; it finds the host through the thread-local stack
/// below.
pub(super) struct HostShared {
    runtime: Runtime,
    scheduler: RefCell<FrameScheduler>,
    wake: RefCell<Option<WakeFn>>,
    env: RefCell<Env>,
    reg: RefCell<Registry>,
}

impl HostShared {
    /// Record the builder closure of a component scope.
    pub(super) fn register(&self, scope: ScopeId, builder: ComponentBuilder) {
        self.reg.borrow_mut().builders.insert(scope, builder);
    }
}

thread_local! {
    /// The stack of hosts currently building on this thread.
    ///
    /// A stack rather than a single slot, because two windows = two
    /// [`AppRuntime`]s, and both live on the same UI thread.
    static HOSTS: RefCell<Vec<Rc<HostShared>>> = const { RefCell::new(Vec::new()) };
}

/// The host currently building on this thread, if any.
pub(super) fn current_host() -> Option<Rc<HostShared>> {
    HOSTS.with(|h| h.borrow().last().cloned())
}

/// Guards the host stack — stays correct even if the user closure panics.
struct HostGuard;

impl HostGuard {
    fn push(host: Rc<HostShared>) -> Self {
        HOSTS.with(|h| h.borrow_mut().push(host));
        HostGuard
    }
}

impl Drop for HostGuard {
    fn drop(&mut self) {
        let _ = HOSTS.try_with(|h| {
            h.borrow_mut().pop();
        });
    }
}

// ---------------------------------------------------------------------------
// BuildCtx
// ---------------------------------------------------------------------------

/// What a component sees while it builds.
///
/// Deliberately thin: local state comes from [`crate::signals::use_signal`],
/// children come from [`super::component`], and application-level injected
/// values from [`BuildCtx::env`]. There is no `setState`, and no widget tree
/// that can be poked at from here (§2.5).
pub struct BuildCtx {
    host: Rc<HostShared>,
}

impl BuildCtx {
    pub(super) fn new(host: Rc<HostShared>) -> Self {
        Self { host }
    }

    /// This application's signals runtime.
    pub fn runtime(&self) -> &Runtime {
        &self.host.runtime
    }

    /// The component scope currently building.
    ///
    /// Panics when called outside a build — just like
    /// [`crate::signals::use_signal`].
    pub fn scope(&self) -> ScopeId {
        current_scope().expect("BuildCtx::scope() hanya berlaku saat komponen dibangun")
    }

    /// Clone the application-level injected value of type `T` ([`Env`]).
    ///
    /// It returns a clone rather than a reference on purpose: what gets
    /// injected is almost always a `Copy` [`crate::signals::Signal`], and
    /// returning a reference would hold an `Env` borrow open for the whole
    /// build.
    pub fn env<T: Clone + 'static>(&self) -> Option<T> {
        self.host.env.borrow().get::<T>().cloned()
    }

    /// Like [`BuildCtx::env`], but panics when nothing was injected.
    pub fn expect_env<T: Clone + 'static>(&self) -> T {
        self.env::<T>().unwrap_or_else(|| {
            panic!(
                "tidak ada titipan bertipe {} di Env aplikasi — pasang lewat AppRuntime::with_env",
                std::any::type_name::<T>()
            )
        })
    }
}

impl core::fmt::Debug for BuildCtx {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BuildCtx")
            .field("scope", &current_scope())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// FrameReport
// ---------------------------------------------------------------------------

/// A summary of one [`AppRuntime::frame`] turn.
///
/// Not decoration: this is what tests use to prove that **only** the relevant
/// subtree was rebuilt, and what an inspector uses to explain jank.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameReport {
    /// Frame sequence number.
    pub index: u64,
    /// Why this frame was scheduled (empty = a frame the OS asked for).
    pub reason: Dirty,
    /// How many component scopes really were rebuilt.
    pub rebuilt: usize,
    /// The diff results of every rebuild this frame, summed.
    pub diff: DiffStats,
    /// How many relayout boundaries were queued when layout started.
    pub relayouts: usize,
    /// The tree's final size after layout.
    pub size: Size,
    /// Frame time measurements.
    pub timing: FrameTiming,
}

impl FrameReport {
    /// True when this frame changed neither structure nor any props.
    pub fn is_noop(&self) -> bool {
        self.rebuilt == 0 && self.diff.is_noop()
    }
}

// ---------------------------------------------------------------------------
// AppRuntime
// ---------------------------------------------------------------------------

/// The owner of one UI lifecycle: **signals → view → layout → paint →
/// scheduler**.
///
/// One instance per window. It holds the four parts that used to live apart and
/// stitches them into a single [`AppRuntime::frame`] turn:
///
/// 1. [`crate::signals::Runtime::drain_dirty`] → the list of scopes that must
///    be rebuilt (already ordered root→leaf and pruned).
/// 2. For each scope: re-run its closure **inside that scope**, then diff the
///    result against the children of that scope's anchor node.
/// 3. [`crate::tree::RenderTree::perform_layout`] — full when the window
///    constraints changed, otherwise only the dirty boundaries.
/// 4. [`crate::tree::RenderTree::paint`] → [`Scene`].
///
/// The wiring into the scheduler is installed once in [`AppRuntime::new`]:
/// [`crate::signals::Runtime::on_wake`] calls [`FrameScheduler::request`]
/// directly, so the §3.5 promise stays intact — a signal no component reads
/// schedules nothing, and without a signal change [`AppRuntime::is_idle`]
/// remains true.
///
/// ```
/// use silka_core::app::{app, component};
/// use silka_core::signals::use_signal;
/// use silka_core::view::{column, fixed};
/// use silka_paint::Color;
///
/// let mut ui = app(|_cx| {
///     let count = use_signal(|| 0i32);
///     column([component("angka", move |_| {
///         fixed(40.0, 20.0 + count.get() as f32).background(Color::WHITE).into()
///     })])
///     .into()
/// })
/// .sized(200.0, 200.0);
///
/// let laporan = ui.frame();
/// assert_eq!(laporan.rebuilt, 1);
/// assert_eq!(ui.scene().len(), 1);
/// // Without a signal change there is no next frame.
/// assert!(ui.is_idle());
/// ```
pub struct AppRuntime {
    host: Rc<HostShared>,
    tree: RenderTree,
    scene: Scene,
    router: InputRouter,
    root: ScopeId,
    constraints: BoxConstraints,
    mounted: bool,
    /// The animation clock + reduced-motion preference (§3.5). Used by
    /// [`AppRuntime::animate`], the only door through which springs advance.
    anim: AnimationDriver,
}

/// Create an application from a root view builder closure — the Dart-style
/// constructor (§2.5).
///
/// The closure runs inside the signals runtime's root scope, so
/// [`crate::signals::use_signal`] may be used directly within it.
pub fn app(build: impl Fn(&BuildCtx) -> View + 'static) -> AppRuntime {
    AppRuntime::new(build)
}

impl AppRuntime {
    /// The long form of [`app`].
    pub fn new(build: impl Fn(&BuildCtx) -> View + 'static) -> Self {
        let runtime = Runtime::new();
        let root = runtime.root();
        let host = Rc::new(HostShared {
            runtime: runtime.clone(),
            scheduler: RefCell::new(FrameScheduler::new()),
            wake: RefCell::new(None),
            env: RefCell::new(Env::new()),
            reg: RefCell::new(Registry::default()),
        });

        // **The signals → scheduler wiring** (§3.5). `Weak` so that the runtime
        // holding this closure does not keep its host alive forever.
        let lemah: Weak<HostShared> = Rc::downgrade(&host);
        runtime.on_wake(move |dirty| {
            let Some(host) = lemah.upgrade() else { return };
            let wake = host.scheduler.borrow_mut().request(dirty);
            // The borrow is released before the platform callback runs: it is
            // allowed to call back into here (e.g. to start the display link).
            let cb = host.wake.borrow().clone();
            if let Some(cb) = cb {
                cb(wake);
            }
        });

        let tree = RenderTree::new();
        {
            let mut reg = host.reg.borrow_mut();
            reg.builders.insert(root, Rc::new(build));
            // The root scope's anchor is the render tree root — that way a
            // single rebuild path serves both the root and any component.
            reg.anchors.insert(root, tree.root());
        }

        let clear = tree.clear_color();
        let app = Self {
            host,
            tree,
            scene: Scene::new(clear),
            router: InputRouter::new(),
            root,
            constraints: BoxConstraints::tight(Size::ZERO),
            mounted: false,
            anim: AnimationDriver::new(),
        };
        // The first frame is the only frame not triggered by a change.
        app.request(Dirty::LAYOUT | Dirty::PAINT);
        app
    }

    // -- configuration (method chaining, §2.5) ------------------------------

    /// The size of the drawing area in logical points.
    pub fn sized(mut self, width: f32, height: f32) -> Self {
        self.resize(Size::new(width, height));
        self
    }

    /// The frame's background color — always the `background` token, never a
    /// literal.
    pub fn clear_color(mut self, color: Color) -> Self {
        self.set_clear_color(color);
        self
    }

    /// The document's reading direction (§9.8).
    pub fn direction(mut self, direction: TextDirection) -> Self {
        self.set_direction(direction);
        self
    }

    /// Inject an application-level value into [`Env`].
    ///
    /// The closure receives the runtime so that the common case — a
    /// [`crate::signals::Signal`] — can be created on the spot:
    ///
    /// ```
    /// # use silka_core::app::app;
    /// # use silka_core::view::fixed;
    /// let ui = app(|cx| {
    ///     let judul: silka_core::signals::Signal<&'static str> = cx.expect_env();
    ///     fixed(10.0, 10.0).label(judul.get()).into()
    /// })
    /// .with_env(|rt| rt.signal("Beranda"));
    /// ```
    pub fn with_env<T: 'static>(self, f: impl FnOnce(&Runtime) -> T) -> Self {
        let value = f(&self.host.runtime);
        self.host.env.borrow_mut().insert(value);
        self
    }

    /// Install the shell's "a frame was scheduled" notifier.
    ///
    /// Called every time the scheduler accepts a request — [`Wake::Schedule`]
    /// means the vsync source must be woken, anything else means there is
    /// nothing to do.
    pub fn on_wake(&self, f: impl Fn(Wake) + 'static) {
        *self.host.wake.borrow_mut() = Some(Rc::new(f));
    }

    // -- accessors -----------------------------------------------------------

    /// This application's signals runtime.
    pub fn runtime(&self) -> &Runtime {
        &self.host.runtime
    }

    /// The render tree the last frame produced.
    pub fn tree(&self) -> &RenderTree {
        &self.tree
    }

    /// This application's input router.
    pub fn router(&self) -> &InputRouter {
        &self.router
    }

    /// The scene the last frame produced.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The root scope (the outermost component).
    pub fn root_scope(&self) -> ScopeId {
        self.root
    }

    /// The anchor render node of a component scope, while it is still alive.
    pub fn anchor(&self, scope: ScopeId) -> Option<NodeId> {
        self.host.reg.borrow().anchors.get(&scope).copied()
    }

    /// The window constraints in effect.
    pub fn constraints(&self) -> BoxConstraints {
        self.constraints
    }

    /// Clone the injected [`Env`] value of type `T` from outside a build.
    ///
    /// This is how the shell reaches back to a signal it injected itself (e.g.
    /// `Signal<Theme>` when OS dark mode changes).
    pub fn env<T: Clone + 'static>(&self) -> Option<T> {
        self.host.env.borrow().get::<T>().cloned()
    }

    /// The accessibility tree built from the last frame's geometry (§3.8).
    ///
    /// Focus is read from the [`InputRouter`] rather than stored twice.
    pub fn access_tree(&self) -> AccessTree {
        self.tree.access_tree(self.router.focus().focused())
    }

    // -- scheduler ------------------------------------------------------------

    /// Request one frame because of `dirty`.
    pub fn request(&self, dirty: Dirty) -> Wake {
        let wake = self.host.scheduler.borrow_mut().request(dirty);
        let cb = self.host.wake.borrow().clone();
        if let Some(cb) = cb {
            cb(wake);
        }
        wake
    }

    /// The reasons not yet served.
    pub fn pending(&self) -> Dirty {
        self.host.scheduler.borrow().pending()
    }

    /// True when nothing at all needs drawing — **idle = zero work**.
    pub fn is_idle(&self) -> bool {
        self.host.scheduler.borrow().is_idle()
    }

    /// The number of the next frame.
    pub fn frame_index(&self) -> u64 {
        self.host.scheduler.borrow().frame_index()
    }

    /// Report the display tick from the platform.
    pub fn set_vsync(&self, vsync: crate::scheduler::Vsync) {
        self.host.scheduler.borrow_mut().set_vsync(vsync);
    }

    /// Frame time summary (a clone, since the scheduler is shared).
    pub fn frame_stats(&self) -> crate::scheduler::FrameStats {
        self.host.scheduler.borrow().stats().clone()
    }

    // -- animation -------------------------------------------------------------

    /// The motion preference in effect (OS reduced-motion).
    pub fn motion(&self) -> Motion {
        self.anim.motion()
    }

    /// Report the OS reduced-motion setting.
    ///
    /// The shell is what reads it (`INTEGRASI-NATIVE` §6); here it is merely
    /// handed to the [`AnimationDriver`] and, when it changed, one frame is
    /// requested so that decorative motion already in flight can finish itself
    /// instead of freezing halfway.
    pub fn set_motion(&mut self, motion: Motion) -> Dirty {
        let dirty = self.anim.set_motion(motion);
        if !dirty.is_empty() {
            self.request(dirty);
        }
        dirty
    }

    /// **Advance animation by one frame** — the seam between springs and the
    /// frame cycle.
    ///
    /// This is the single door that used to be deliberately left empty: the
    /// animation system (§3.5) was complete, but nothing called it per frame.
    /// `f` receives the render tree and this frame's [`Tick`], and returns its
    /// dirty reasons — exactly the shape `silka_widgets::advance` satisfies.
    /// Those reasons are merged into the scheduler's requests, so as long as a
    /// spring is still moving the next frame schedules itself, and once
    /// everything settles the renderer goes back to sleep.
    ///
    /// Call it **before** [`AppRuntime::frame`] so that a moving value is
    /// already this frame's value rather than the next frame's.
    ///
    /// ```
    /// use silka_core::app::app;
    /// use silka_core::scheduler::Dirty;
    /// use silka_core::view::fixed;
    ///
    /// let mut ui = app(|_cx| fixed(80.0, 24.0).into()).sized(200.0, 100.0);
    /// // With no animation at all, advancing a frame creates no work.
    /// assert_eq!(ui.animate(|_tree, _tick| Dirty::NONE), Dirty::NONE);
    /// ui.frame();
    /// assert!(ui.is_idle());
    /// ```
    pub fn animate(&mut self, f: impl FnOnce(&mut RenderTree, &Tick) -> Dirty) -> Dirty {
        self.animate_at(Instant::now(), f)
    }

    /// [`AppRuntime::animate`] with a caller-supplied frame time.
    ///
    /// For tests that must be deterministic (§9.5) and for shells that already
    /// hold their own vsync timestamp — never invent 16.6 ms (§3.5).
    pub fn animate_at(
        &mut self,
        now: Instant,
        f: impl FnOnce(&mut RenderTree, &Tick) -> Dirty,
    ) -> Dirty {
        let tick = self.anim.begin_frame(now);
        let mut dirty = f(&mut self.tree, &tick);
        dirty |= self.anim.end_frame(tick);
        // Dirty flags raised by nodes that just moved are carried along too,
        // exactly as in `dispatch`.
        dirty |= self.tree.take_dirty();
        if !dirty.is_empty() {
            self.request(dirty);
        }
        dirty
    }

    /// True when the previous animation frame left something still moving.
    pub fn is_animating(&self) -> bool {
        self.anim.is_animating()
    }

    // -- changes from the outside ----------------------------------------------

    /// Resize the drawing area; true when it really changed.
    pub fn resize(&mut self, size: Size) -> bool {
        let baru = BoxConstraints::tight(size);
        if self.constraints == baru {
            return false;
        }
        self.constraints = baru;
        self.request(Dirty::SURFACE | Dirty::LAYOUT);
        true
    }

    /// Change the frame's background color; true when it really changed.
    pub fn set_clear_color(&mut self, color: Color) -> bool {
        if self.tree.clear_color() == color {
            return false;
        }
        self.tree.set_clear_color(color);
        self.request(Dirty::THEME | Dirty::PAINT);
        true
    }

    /// Change the document's reading direction; true when it really changed.
    pub fn set_direction(&mut self, direction: TextDirection) -> bool {
        if self.tree.direction() == direction {
            return false;
        }
        self.tree.set_direction(direction);
        self.request(Dirty::LAYOUT | Dirty::PAINT);
        true
    }

    /// Route one input event into the tree.
    ///
    /// The returned value already accounts for **signal writes** that happened
    /// inside the handler: its `dirty` is merged with whatever is pending in
    /// the scheduler, so the shell never has to tell the difference.
    pub fn dispatch(&mut self, event: &Event) -> Response {
        let mut hasil = self.router.dispatch(&mut self.tree, event);
        hasil.dirty |= self.tree.take_dirty();
        if !hasil.dirty.is_empty() {
            self.request(hasil.dirty);
        }
        hasil.dirty |= self.pending();
        hasil
    }

    // -- one frame ------------------------------------------------------------

    /// Run one full turn and return its summary.
    ///
    /// The order is fixed and must not be swapped: rebuild → diff → layout →
    /// paint. The scene can be read through [`AppRuntime::scene`].
    pub fn frame(&mut self) -> FrameReport {
        let mut start = self.host.scheduler.borrow_mut().begin_frame(Instant::now());

        // 1. Who has to be rebuilt.
        //
        // The first frame builds the root; after that the list comes from
        // signals — already ordered root→leaf and **pruned** (descendants of a
        // scope that is also dirty are dropped), so no subtree is worked twice.
        let antrean: Vec<ScopeId> = if self.mounted {
            self.host.runtime.drain_dirty()
        } else {
            self.mounted = true;
            vec![self.root]
        };

        // 2. Rebuild + diff, scope by scope.
        let mut diff = DiffStats::default();
        let mut rebuilt = 0usize;
        for scope in antrean {
            if let Some(stat) = self.rebuild_scope(scope) {
                diff.merge(stat);
                rebuilt += 1;
            }
        }
        if diff.removed > 0 {
            self.kumpulkan_sampah();
        }

        // 3. Layout: full when the constraints changed or the root is dirty,
        //    otherwise only the dirty boundaries.
        let relayouts = self.tree.pending_boundaries();
        let size = self.tree.perform_layout(self.constraints);

        // 4. Paint into a buffer that is reused across frames.
        self.tree.paint_into(&mut self.scene);

        // The tree's dirty flags were served by this frame too — if they were
        // allowed to pile up, the next frame would be scheduled for no reason
        // and "idle = zero" would stop holding.
        //
        // The one thing **not** finished by this frame is
        // [`Dirty::ANIMATION`]: a spring that the view-diff has just retargeted
        // (a dialog's `open` prop changed, a button entered its loading state)
        // has not moved at all yet — it will only move in the next frame's
        // `animate`. Dropping it here freezes the animation until the next
        // input event arrives, and that has happened before.
        let sisa = self.tree.take_dirty();
        if sisa.contains(Dirty::ANIMATION) || self.anim.is_animating() {
            self.request(Dirty::ANIMATION);
        }

        start.mark_built(Instant::now());
        let timing = self
            .host
            .scheduler
            .borrow_mut()
            .end_frame(start, Instant::now(), true);

        FrameReport {
            index: timing.index,
            reason: timing.reason,
            rebuilt,
            diff,
            relayouts,
            size,
            timing,
        }
    }

    /// Rebuild one scope and diff the result into its anchor.
    ///
    /// `None` when the scope is already dead or its anchor is gone — both
    /// happen normally when a list shrinks during the same frame.
    fn rebuild_scope(&mut self, scope: ScopeId) -> Option<DiffStats> {
        let (builder, anchor) = {
            let reg = self.host.reg.borrow();
            (
                reg.builders.get(&scope).cloned()?,
                reg.anchors.get(&scope).copied()?,
            )
        };
        if !self.tree.contains(anchor) {
            return None;
        }

        let cx = BuildCtx::new(self.host.clone());
        let view = {
            // The host is installed for the duration of the build so that
            // `component()` — called in the middle of the user's closure — can
            // find it.
            let _g = HostGuard::push(self.host.clone());
            if scope == self.root {
                Some(self.host.runtime.build_root(|| builder(&cx)))
            } else {
                self.host.runtime.rebuild(scope, || builder(&cx))
            }
        }?;

        let stats = reconcile_children(&mut self.tree, anchor, std::slice::from_ref(&view));
        // Component anchor nodes inside this subtree may have just been created
        // or replaced; the map is refreshed from the real tree, never guessed.
        self.segarkan_jangkar(anchor);
        Some(stats)
    }

    /// Re-record `scope → NodeId` for every component inside the subtree.
    fn segarkan_jangkar(&self, from: NodeId) {
        let mut tumpukan = vec![from];
        let mut reg = self.host.reg.borrow_mut();
        while let Some(id) = tumpukan.pop() {
            if let Some(node) = self.tree.render(id) {
                if let Some(k) = node.downcast_ref::<ComponentBox>() {
                    reg.anchors.insert(k.scope, id);
                }
            }
            tumpukan.extend_from_slice(self.tree.children(id));
        }
    }

    /// Drop the entries belonging to scopes that have died.
    ///
    /// Only run on frames that actually removed nodes, so a stable list pays
    /// nothing.
    fn kumpulkan_sampah(&self) {
        let rt = &self.host.runtime;
        let mut reg = self.host.reg.borrow_mut();
        reg.builders.retain(|s, _| rt.is_scope_alive(*s));
        reg.anchors
            .retain(|s, id| rt.is_scope_alive(*s) && self.tree.contains(*id));
    }
}

impl core::fmt::Debug for AppRuntime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AppRuntime")
            .field("runtime", &self.host.runtime)
            .field("nodes", &self.tree.len())
            .field("komponen", &self.host.reg.borrow().builders.len())
            .field("idle", &self.is_idle())
            .finish()
    }
}
