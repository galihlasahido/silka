//! winit accessibility adapter — the bridge between the `silka-core` a11y pass
//! and the OS accessibility APIs (REKOMENDASI §3.8).
//!
//! The only thing that crosses from the framework into here is
//! [`AccessTree`]/[`AccessUpdate`](silka_core::AccessUpdate): one tree snapshot
//! plus its diff. The only
//! thing that crosses back is an already-validated [`AccessActionRequest`].
//! `accesskit_winit` handles the rest per platform: UIA on Windows,
//! NSAccessibility on macOS, AT-SPI on Linux.
//!
//! ## Three easily broken rules, locked down here
//!
//! 1. **The adapter is created before the window becomes visible.**
//!    `accesskit_winit` panics otherwise — so the shell creates the window
//!    hidden, attaches the adapter, and only then shows it.
//! 2. **Zero cost when no assistive technology is present.** The a11y pass runs
//!    only while the adapter is active ([`AccessAdapter::update_with`]); users
//!    without a screen reader pay nothing at all. This is a direct extension of
//!    "render only when dirty" (§3.5).
//! 3. **Re-activation always sends the full tree.** A screen reader that was
//!    just switched on has no history; sending it a delta would leave it with a
//!    tree that is never complete.
//!
//! ```
//! use silka_core::access::{AccessRole, AccessTree};
//! use silka_core::tree::{BoxConstraints, RenderTree};
//! use silka_core::view::{column, fixed, interactive, reconcile, View};
//! use silka_paint::Size;
//!
//! // What the shell hands the adapter each frame: a whole tree, from which
//! // the adapter sends only what changed.
//! let mut tree = RenderTree::new();
//! reconcile(
//!     &mut tree,
//!     column([View::from(
//!         interactive(fixed(120.0, 44.0))
//!             .role(AccessRole::Button)
//!             .label("Save"),
//!     )]),
//! );
//! tree.layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
//!
//! let snapshot = tree.access_tree(None);
//!
//! // The first update after a screen reader switches on is the full tree —
//! // a reader with no history cannot be handed a delta.
//! let full = snapshot.changes_since(None);
//! assert!(full.full);
//!
//! // From there it is deltas, and an unchanged frame produces nothing at
//! // all: a user without a screen reader pays nothing for any of this.
//! let quiet = snapshot.changes_since(Some(&snapshot));
//! assert!(quiet.is_empty());
//! ```

use accesskit_winit::Adapter;
use silka_core::access::{AccessActionRequest, AccessTree};
use winit::event::WindowEvent as WinitWindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

/// An accessibility event arriving from the OS through the winit event loop.
///
/// Used as the shell event loop's *user event*. A newtype rather than an alias,
/// so the application event loop can carry other events later without forcing
/// an API change.
///
/// It reaches the event loop wrapped in [`crate::ShellEvent`], alongside menu
/// and tray activations:
///
/// ```
/// use silka_platform::ShellEvent;
///
/// fn is_accessibility(event: &ShellEvent) -> bool {
///     matches!(event, ShellEvent::Access(_))
/// }
/// # let _ = is_accessibility;
/// ```
#[derive(Debug)]
pub struct AccessEvent(pub accesskit_winit::Event);

impl From<accesskit_winit::Event> for AccessEvent {
    fn from(event: accesskit_winit::Event) -> Self {
        Self(event)
    }
}

impl AccessEvent {
    /// The window this event refers to.
    pub fn window_id(&self) -> WindowId {
        self.0.window_id
    }
}

/// What the shell must do after an [`AccessEvent`] has been processed.
///
/// ```
/// use silka_platform::access::AccessOutcome;
///
/// fn needs_work(outcome: &AccessOutcome) -> bool {
///     // `Idle` is the common case — accessibility is off, and the a11y pass
///     // never runs. Users without a screen reader pay nothing at all.
///     !matches!(outcome, AccessOutcome::Idle)
/// }
///
/// assert!(!needs_work(&AccessOutcome::Idle));
/// assert!(needs_work(&AccessOutcome::NeedsFullTree));
/// ```
///
/// [`AccessOutcome::NeedsFullTree`] rather than a delta on activation is
/// deliberate: a screen reader that was just switched on has no history, and a
/// delta would leave it with a tree that is never complete.
#[derive(Debug, Clone, PartialEq)]
pub enum AccessOutcome {
    /// Assistive technology asked for the whole tree — send one full update.
    NeedsFullTree,
    /// Assistive technology requested an action on a node.
    Action(AccessActionRequest),
    /// Nothing to do (e.g. accessibility was switched off).
    Idle,
}

/// Accessibility adapter for a single window.
///
/// Applications do not build one: [`crate::WindowConfig::on_access`] wires it
/// up and the shell drives it. What the type enforces is three rules that are
/// easy to break and expensive to debug —
///
/// 1. it is attached **before** the window becomes visible (`accesskit_winit`
///    panics otherwise), which is why the shell creates the window hidden;
/// 2. the a11y pass runs only while assistive technology is actually present,
///    so the tree is never built for nobody ([`AccessAdapter::update_with`]
///    takes a closure precisely so the tree is not even constructed);
/// 3. re-activation sends the **whole** tree, never a delta.
///
/// ```no_run
/// use silka_core::view::fixed;
/// use silka_platform::{run_app, window};
///
/// run_app(window("Editor"), |_cx| fixed(120.0, 24.0).into()).unwrap();
/// // The a11y tree comes from the same render tree as layout and paint, so
/// // what a screen reader announces cannot diverge from what was drawn.
/// ```
pub struct AccessAdapter {
    inner: Adapter,
    /// The last snapshot actually sent — the basis for the delta **and** for
    /// resolving action requests. Deliberately not the newest tree: assistive
    /// technology always talks about the tree it has already seen.
    terkirim: Option<AccessTree>,
    /// True from activation until assistive technology is switched off again.
    aktif: bool,
}

impl AccessAdapter {
    /// Attach the adapter to a window.
    ///
    /// **Must be called before the window is shown** — create the window with
    /// `with_visible(false)`, call this, then `set_visible(true)`.
    ///
    /// The proxy's event type is only required to be *buildable from* an
    /// accessibility event, so a shell whose loop also carries menu and tray
    /// events (see [`crate::ShellEvent`]) can hand over the same proxy instead
    /// of running a second one.
    pub fn new<T: From<accesskit_winit::Event> + Send + 'static>(
        event_loop: &ActiveEventLoop,
        window: &Window,
        proxy: EventLoopProxy<T>,
    ) -> Self {
        Self {
            inner: Adapter::with_event_loop_proxy(event_loop, window, proxy),
            terkirim: None,
            aktif: false,
        }
    }

    /// True while some assistive technology is listening.
    pub fn is_active(&self) -> bool {
        self.aktif
    }

    /// Forward a window event to the adapter.
    ///
    /// Must be called for **every** window event, before the shell handles it
    /// itself: window focus and geometry are tracked from here.
    pub fn process_event(&mut self, window: &Window, event: &WinitWindowEvent) {
        self.inner.process_event(window, event);
    }

    /// Handle an accessibility event coming from the event loop.
    pub fn handle(&mut self, event: &AccessEvent) -> AccessOutcome {
        match &event.0.window_event {
            accesskit_winit::WindowEvent::InitialTreeRequested => {
                self.aktif = true;
                // Drop the history: a newly arrived consumer must receive the
                // full tree, not a slice of changes it has no basis for.
                self.terkirim = None;
                AccessOutcome::NeedsFullTree
            }
            accesskit_winit::WindowEvent::ActionRequested(request) => self
                .terkirim
                .as_ref()
                .and_then(|pohon| pohon.action_request(request))
                .map(AccessOutcome::Action)
                .unwrap_or(AccessOutcome::Idle),
            accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                self.aktif = false;
                self.terkirim = None;
                AccessOutcome::Idle
            }
        }
    }

    /// Build the a11y tree **only if someone is listening**, then send the
    /// diff.
    ///
    /// `scale_factor` is the window's scale factor: AccessKit demands physical
    /// pixel coordinates, whereas the whole framework above it speaks in
    /// logical points.
    pub fn update_with(&mut self, scale_factor: f64, build: impl FnOnce() -> AccessTree) {
        if !self.aktif {
            return;
        }
        let pohon = build();
        let update = pohon.changes_since(self.terkirim.as_ref());
        if update.is_empty() {
            return;
        }
        self.inner
            .update_if_active(|| update.to_tree_update(scale_factor));
        self.terkirim = Some(pohon);
    }

    /// Send one full tree, whatever the history says.
    ///
    /// Used when answering [`AccessOutcome::NeedsFullTree`].
    pub fn update_full(&mut self, scale_factor: f64, pohon: AccessTree) {
        self.aktif = true;
        self.inner
            .update_if_active(|| pohon.to_tree_update(scale_factor));
        self.terkirim = Some(pohon);
    }

    /// The last snapshot sent — the assistive technology's current view.
    pub fn last_sent(&self) -> Option<&AccessTree> {
        self.terkirim.as_ref()
    }
}

impl core::fmt::Debug for AccessAdapter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AccessAdapter")
            .field("aktif", &self.aktif)
            .field("node_terkirim", &self.terkirim.as_ref().map(|t| t.len()))
            .finish()
    }
}
