//! # silka-core
//!
//! The framework engine: everything that sits beneath the Dart-style public
//! API (REKOMENDASI §2). This crate is **implementation detail** — the contract
//! application authors see lives in `silka-widgets`.
//!
//! The layers it hosts:
//!
//! - **Signals + per-component rebuild** (the Dioxus 0.7 pattern, §2.5): a
//!   signal write marks every component that read it dirty → rebuild that one
//!   small subtree → diff. Needs a dirty-marking scheduler + scope tracking.
//! - **View-diff → arena render tree** (§2): a lightweight view tree is rebuilt
//!   on every update and diffed into a retained tree backed by an ID-addressed
//!   arena/slotmap. The arena was chosen because AccessKit and Taffy are both
//!   ID-based.
//! - **Flutter-style box constraints** as the native layout protocol
//!   ("constraints go down, sizes come up"), single pass + relayout boundaries;
//!   Taffy drives the Flex/Grid widgets, with leaf measure functions riding on
//!   `silka-text` (§3.4). Layout must understand **RTL mirroring** from day
//!   one — retrofitting RTL is as hopeless as retrofitting a11y (§9.8).
//! - **Spring animation** (§3.5): an animated value stores
//!   `(position, velocity)` and is **always interruptible/retargetable** — a
//!   closed-form damped harmonic oscillator solution, perceptual parameters
//!   (duration + bounce), and the `smooth`/`snappy`/`bouncy` presets.
//!   Reduced-motion must be honored.
//! - **Input + hit-testing + velocity tracker** — velocity is what makes
//!   gesture handoff (fling → spring) possible.
//! - **Scheduler**: render **only when dirty**; vsync arrives over a
//!   per-platform display link, never a hardcoded 16.6 ms.
//!
//! **AccessKit is a first-class output of the render tree** (§3.8), not a layer
//! bolted on later: every node supplies role, name, bounds, and actions.
//!
//! ## What exists today
//!
//! **Milestone `frame-scheduling`** — [`scheduler`]: the **render-on-dirty**
//! engine together with frame time measurement. Pure logic: it knows nothing of
//! winit and nothing of wgpu. The platform supplies only the vsync tick and its
//! measured interval; `silka-platform` uses `CADisplayLink` on macOS
//! (ProMotion-aware) and winit's `request_redraw` everywhere else. **There is
//! no 16.6 ms anywhere** — while the interval is unknown it is `None`, and
//! nothing pretends to know better.
//!
//! **Milestone `signals`** — [`signals`]: the Dioxus-style state runtime.
//! [`signals::use_signal`] for component-local state, per-scope dependency
//! tracking, dirty marking + batching, and [`signals::Key`]-based scope
//! identity for dynamic lists. Its wiring into [`scheduler`] is a single line
//! ([`signals::Runtime::on_wake`]), which keeps the "render only when dirty"
//! promise intact: a signal that no component reads does **not** wake the GPU.
//!
//! **Milestone `arena-tree`** — [`tree`] and [`view`]: a retained render tree
//! backed by a generational ID arena, the **Flutter-style box constraints**
//! protocol ("constraints go down, sizes come up, the parent decides
//! position"), a layout cache plus **relayout boundaries**, and above it the
//! **view-diff** layer: a lightweight Dart-style view tree rebuilt on every
//! rebuild and then diffed into the render tree (§2). Child identity uses the
//! same [`signals::Key`] as component scopes, so there is exactly one key
//! discipline across the whole framework. [`tree::RenderNode::access`] is part
//! of the node contract from the start, with `bounds` coming from layout
//! results (§3.8).
//!
//! The single-frame flow the three of them assemble:
//!
//! ```
//! use silka_core::scheduler::FrameScheduler;
//! use silka_core::tree::{BoxConstraints, RenderTree};
//! use silka_core::view::{column, fixed, reconcile};
//! use silka_paint::Size;
//!
//! let mut scheduler = FrameScheduler::new();
//! let mut tree = RenderTree::new();
//!
//! // 1. A component rebuilds → new view → diff into the render tree.
//! reconcile(&mut tree, column([fixed(120.0, 24.0)]).spacing(8.0));
//! // 2. What changed decides whether the renderer needs waking at all.
//! scheduler.request(tree.take_dirty());
//! // 3. Layout: full when the window resizes, subtree-only otherwise.
//! tree.perform_layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
//! ```
//!
//! **Milestone `spring`** — [`animation`]: a spring animation system built on
//! the **closed-form damped harmonic oscillator** solution. A value stores
//! `(position, velocity)` ([`animation::SpringValue`]) and is therefore
//! **always interruptible**: [`animation::SpringValue::set_target`] may be
//! called at any moment and the velocity carries over (WWDC23), which doubles
//! as the fling → spring gesture handoff path. The parameters are perceptual
//! (duration + bounce) with `smooth`/`snappy`/`bouncy` presets, and
//! **reduced-motion** ([`animation::Motion`]) is part of the contract, not a
//! final coat of polish. Its wiring into [`scheduler`] follows the same rule as
//! signals: [`animation::AnimationDriver::end_frame`] returns
//! [`Dirty::ANIMATION`] only while something is genuinely moving — no timer
//! ticks, and the moment every spring settles the GPU goes back to sleep.
//!
//! **Milestone `accesskit`** — [`access`]: accessibility node emission as a
//! **render tree pass**, on equal footing with layout and paint (§3.8).
//! [`tree::RenderNode::access`] is a **required** method — a widget that forgot
//! to think about screen readers does not compile — and every node's `bounds`
//! comes from layout results rather than from the widget, so what assistive
//! technology announces cannot diverge from what was drawn.
//! [`access::AccessTree::dump`] gives a deterministic tree dump for golden
//! tests, and [`access::AccessTree::changes_since`] keeps the "only when dirty"
//! promise alive for screen readers too. The conversion to `accesskit` is
//! confined to a single file; the winit adapter lives in `silka-platform`.
//!
//! **Milestone `taffy-flex`** — [`tree::TaffyBox`]: Flexbox and CSS Grid run
//! with **Taffy as a widget inside the box constraints protocol** (§3.4).
//! Dart-style `row()`/`column()`/`grid()` ([`view::row`], [`view::column`],
//! [`view::grid`]) with `.spacing()`/`.gap_*()` locked to the 4pt scale
//! ([`tree::SPACING_UNIT`], §2.6), `expanded()`/`flexible()` as the
//! counterparts of Flutter's `Expanded`/`Flexible`, and RTL mirroring passed
//! straight through to Taffy (§9.8). The name `taffy::` never escapes a single
//! module: the public vocabulary is ours ([`tree::ContainerStyle`],
//! [`tree::ItemStyle`], [`tree::Track`]). **Text measurement enters through a
//! leaf measure function** — [`tree::MeasuredBox`] (`view::measured`) is the
//! only door, used identically by our own box-constraints engine and by Taffy.
//!
//! **Milestone `input-hittest`** — [`input`]: pointer/keyboard event routing,
//! hit-testing, focus, velocity tracking, and IME. Four promises from the
//! design docs are settled here:
//!
//! 1. **Squircle-aware hit-testing** (§3.6) — [`input::HitShape::Rounded`]
//!    tests **exactly** the superellipse that is handed to the shader
//!    ([`silka_paint::Corners::contains`]), so a corner that looks empty is not
//!    clickable, and vice versa. A viewport clips its contents, so a row that
//!    has scrolled out of view can no longer be touched.
//! 2. **Focus & tab order** ([`input::FocusManager`]) are computed from the
//!    same render tree as layout and a11y, complete with explicit ordering and
//!    **focus scopes** as dialog focus traps.
//! 3. **Velocity tracker** ([`input::VelocityTracker`]) — Flutter's
//!    second-degree least-squares regression; this is what supplies the initial
//!    `velocity` for [`animation::SpringValue::set_target`], i.e. the
//!    fling → spring handoff promised by §3.5.
//! 4. **IME** ([`input::ImeRequest`]) — preedit/commit flow only to the focused
//!    node, and caret area requests flow back out to the shell so that the CJK
//!    candidate window anchors in the right place (§3.8).
//!
//! The contract lives on [`tree::RenderNode`] (`hit_shape`, `hit_behavior`,
//! `focus_policy`, `cursor`, `event`) alongside `access` — not as a later
//! layer. [`tree::Interactive`] (`view::interactive`) is the first node to use
//! it in full, and `silka-platform` translates winit into this vocabulary in a
//! single file.
//!
//! **Milestone `paint-pass`** — [`tree::RenderTree::paint`]: assembling a
//! [`silka_paint::Scene`] from the render tree, a third pass on equal footing
//! with layout and a11y (§3.2). [`tree::RenderNode::paint`] is part of the node
//! contract, and its vocabulary is **only** `silka-paint` — quads, double
//! shadows, glyph runs: not a single wgpu type can reach widget code, so a new
//! backend (GL/CPU) later lands in exactly one place. Four properties:
//!
//! 1. **Nodes draw in local coordinates** — the mirror of the layout rule "a
//!    node never knows its own position"; [`tree::PaintCtx`] is what lifts them
//!    into absolute coordinates, and those absolutes are exactly the a11y
//!    `bounds`.
//! 2. **Parent before child**, so command order = stacking order.
//! 3. **Clipping** uses the same [`tree::RenderNode::clips_children`] answer
//!    hit-testing already uses: one answer for two passes, which makes a row
//!    that has scrolled off screen but is still clickable impossible.
//! 4. **Render only when dirty, down to subtree granularity** (§3.5): draw
//!    commands are cached at relayout boundaries, and a clean subtree that has
//!    not moved is not re-run at all.
//!
//! Color never originates in the engine: [`tree::Decoration`] carries values
//! **already resolved** from theme tokens one level up, so the
//! Cupertino/Tailwind presets (§2.7) can be swapped without a single line
//! changing here — including corner geometry, which stays a parameter rather
//! than a constant.
//!
//! **Milestone `reactive-glue`** — [`mod@app`]: the six layers above are
//! finally **joined into one lifecycle**. [`app::AppRuntime`] owns the signals
//! runtime, the root view builder closure, the render tree, and the
//! [`scheduler::FrameScheduler`]; [`app::AppRuntime::frame`] runs one full
//! turn:
//!
//! ```text
//! signals::Runtime::drain_dirty()          ← scopes that must be rebuilt
//!   → re-run their closure INSIDE that scope
//!   → view::reconcile_children(tree, anchor, [new view])
//!   → tree::RenderTree::perform_layout(window constraints)
//!   → tree::RenderTree::paint_into(scene)
//! ```
//!
//! Two seams that used to gape are closed here.
//! [`signals::Runtime::drain_dirty`] finally has a caller, and that caller
//! honors its contract exactly: [`app::component`] builds its body *eagerly*
//! inside [`signals::scope`], so rebuilding a scope **re-enters every retained
//! child** — the precondition that makes pruning descendants from the dirty
//! list sound. And [`signals::Runtime::on_wake`] is wired straight into
//! [`scheduler::FrameScheduler::request`], so the §3.5 promise holds
//! end-to-end: a signal write schedules exactly one frame, and once that frame
//! is done [`app::AppRuntime::is_idle`] is true again with not a single timer
//! ticking.
//!
//! Every component owns an **anchor node** ([`app::ComponentBox`]) —
//! transparent to layout and filtered out of the a11y tree — because without it
//! the only way to apply a rebuild's result is to diff from the root, and
//! "per-component rebuild" becomes a name and nothing more.
//!
//! **Milestone `demo-end-to-end`** — the last three pieces that make that chain
//! something you can **see and touch** rather than merely test:
//!
//! 1. [`Callback`] + [`tree::Interactive::on_press`] — the action an
//!    application hands to a node. This is the Dart-style `on_press` promised
//!    by §2.5, and it closes the `click → signal → rebuild` path: before it, an
//!    interactive node could only **count** activations, never tell anyone
//!    about them.
//! 2. **Per-state appearance** ([`tree::Interactive::decoration`] plus the
//!    [`tree::StateStyle`] deltas behind `hover(|s| …)` / `pressed(|s| …)` /
//!    `focused(|s| …)`, and [`tree::FocusRing`]) — the values are already
//!    resolved from tokens one level up (§2.6), the transition between them is
//!    a spring owned by the node rather than by each widget (§3.5), and the
//!    corner shape is guaranteed to match the shape hit-testing checks because
//!    both read the same [`tree::Interactive::corners`] (§3.6).
//! 3. [`app::ScaleFactor`] as a standard [`app::Env`] injected value — text
//!    must be rasterized at the real screen resolution (§3.3), and a window
//!    moved to another monitor rebuilds only the components that read it.
//!
//! The proof lives in the `counter` page of `examples/gallery`: a single click
//! simulated through the input layer ends up as different pixels on a
//! GPU-rendered texture.
//!
//! [`silka_paint::Command::PushClip`] is already executed by the backend as a
//! scissor rect per instance range, so this pass's clip contract holds all the
//! way down to the pixel.
//!
//! **Milestone `utility-vocab`** — [`view::div`] and the §2.6 utility
//! vocabulary: `flex()`, `items_center()`, `justify_between()`, `p_4()`,
//! `rounded_lg()`, `shadow_md()`, `bg()`, `text_sm()` as a method chain, in
//! Tailwind's spelling but on HIG's numbers.
//!
//! The point of the milestone is not the vocabulary but **who is allowed to
//! supply the values**. §2.6 discipline #1 ("values are locked to design
//! tokens") used to be guarded by a doc-comment: `background()` takes a
//! `Color`, so a literal type-checked. Now the normal path takes only a token
//! (`ColorToken`, `RadiusToken`, `ShadowToken`, `SpaceToken`, `FontToken`) and
//! `.bg(Color::hex(0x1E90FF))` **does not compile**; a brand color that really
//! is not a token goes through a conspicuously named escape hatch
//! (`bg_raw`, `rounded_raw`, `p_raw`). The older `background`/`corners`/
//! `border`/`shadow` remain as the layer underneath, which is what the token
//! methods call.
//!
//! Tokens meet numbers through the **ambient theme** ([`view::with_theme`]):
//! [`app::AppRuntime::frame`] installs it around the whole rebuild pass, taken
//! from the `Signal<Theme>` in [`app::Env`], so no call site has to name
//! `theme` (§2.5 — the code has to read like Dart) and a component rebuilt on
//! its own halfway down the tree resolves against the same theme as the root.
//! Resolution happens while
//! the view is built, so [`tree::Decoration`] still reaches the paint pass
//! already resolved and the renderer stays theme-free (§3.2). `rounded_lg()`
//! is therefore one call with two geometries: a 14pt squircle under Cupertino,
//! an 8pt arc under Tailwind (§2.7).
//!
//! **Milestone `utility-spring`** — §2.6 discipline #2: *"`hover(...)` /
//! `pressed(...)` / `focused(...)` transition through a spring animation
//! (§3.5), they do not jump the way CSS without `transition` does."*
//!
//! ```
//! # use silka_core::view::{fixed, interactive};
//! # use silka_theme::ColorToken;
//! let _ = interactive(fixed(240.0, 88.0))
//!     .bg(ColorToken::Surface)
//!     .rounded_lg()
//!     .hover(|s| s.bg(ColorToken::SurfaceHover))
//!     .pressed(|s| s.bg(ColorToken::SurfacePressed).scale(0.98))
//!     .focused(|s| s.ring(ColorToken::FocusRing));
//! ```
//!
//! Each state is a [`tree::StateStyle`] — a delta over the resting style,
//! written in the same utility vocabulary — and [`tree::Interactive`] keeps one
//! [`SpringValue`] per animatable property (background, border, focus ring,
//! scale). They are advanced by [`tree::RenderTree::advance`], the **one** pass
//! that ticks the whole tree, and retargeted mid-flight carrying their velocity,
//! so a pointer that leaves halfway reverses without a seam.
//!
//! The point is *where the spring lives*: before this milestone every widget
//! brought its own, which meant an `interactive(…)` written by an application
//! jumped. Motion is now a property of the system.
//!
//! **Milestone `utility-adopt`** — the vocabulary put to work. [`styling`] is
//! the page to read before writing a screen: it teaches the vocabulary as the
//! primary way to arrange a view, with the token rule, the 4pt scale, the
//! closure states, and a mechanical table for converting hand-styled code. The
//! gallery carries the worked examples — `reactive.rs` (a page rewritten out of
//! layout arithmetic, gaining hover/press/focus on the way) and `utility.rs`
//! (the vocabulary itself as a live reference).
//!
//! Springs are no longer the application's problem: [`app::AppRuntime`] owns an
//! [`animation::AnimationDriver`], every frame opens and closes with it
//! ([`app::AppRuntime::animate_at`]), and the driver is what answers "does this
//! need another frame?" — so an app that never calls `request_animation_frame`
//! still animates, and one that stops interacting still comes to a complete
//! stop ([`app::AppRuntime::is_idle`]).
//!
//! What is still missing, and what comes next: **incremental** repaint. Layers
//! exist all the way down — [`tree::PaintCtx::with_layer`],
//! [`silka_paint::Command::PushLayer`], and an offscreen target in the renderer
//! — and they already buy group opacity and blur. What they do not buy yet is
//! keeping a layer's texture **across** frames so an unchanged subtree is
//! composited rather than repainted.

#![warn(missing_docs)]
// Documentation is part of the public contract, so the checks rustdoc offers
// are turned on here rather than left to a reviewer's eye. A broken intra-doc
// link is an error: it means a rename silently orphaned a reference.
#![deny(rustdoc::broken_intra_doc_links)]
#![warn(
    rustdoc::private_intra_doc_links,
    rustdoc::invalid_codeblock_attributes,
    rustdoc::invalid_html_tags,
    rustdoc::bare_urls,
    rustdoc::unescaped_backticks
)]

pub mod access;
pub mod animation;
pub mod app;
mod callback;
pub mod date;
pub mod hot;
pub mod input;
pub mod locale;
pub mod recover;
pub mod scheduler;
pub mod signals;
pub mod styling;
pub mod task;
pub mod tree;
pub mod view;

pub use access::{
    AccessAction, AccessActionRequest, AccessActions, AccessEntry, AccessNode, AccessRole,
    AccessToggled, AccessTree, AccessUpdate,
};
pub use animation::{
    Animatable, AnimationDriver, Motion, MotionRole, Propagator, Spring, SpringValue, Tick,
    Tolerance,
};
pub use app::{app, component, current_tasks, AppRuntime, BuildCtx, Env, FrameReport, ScaleFactor};
pub use callback::Callback;
pub use date::{days_in_month, is_leap_year, Date, TimeUnit};
pub use hot::{patch_screen, register_screen, screen_view, HotTheme, ScreenFn};
pub use input::{
    hit_test, CursorIcon, Event, EventCtx, FocusDirection, FocusManager, FocusPolicy, HitBehavior,
    HitShape, ImeEvent, ImeRequest, InputResponse, InputRouter, KeyCode, KeyEvent, Modifiers,
    NamedKey, PointerButton, PointerEvent, PointerPhase, ScrollEvent, Velocity, VelocityTracker,
};
pub use locale::{CompactUnit, CurrencyPosition, DateOrder, Locale};
pub use recover::{catch, guard_view, guard_view_or, install_hook, on_crash, PanicReport};
pub use scheduler::{
    ClockSource, Dirty, FrameLogger, FrameScheduler, FrameStart, FrameStats, FrameTiming,
    RefreshEstimator, Vsync, Wake,
};
pub use signals::{
    current_scope, list, scope, untracked, use_signal, Key, Runtime, ScopeId, Signal, SignalId,
};
pub use task::{
    use_resource, Cancel, Load, Notifier, Spawner, TaskHandle, TaskId, Tasks, ThreadSpawner,
};
pub use tree::{
    Alignment, BoxConstraints, ContainerStyle, CrossAlign, Decoration, ItemStyle, LayoutCtx,
    MainAlign, NodeId, PaintCtx, RenderNode, RenderTree, StackFit, TextDirection, Track,
};
pub use view::{
    active_theme, align, aspect_ratio, center, container, div, reconcile, stack, with_theme,
    DiffStats, Margined, Padded, TextStyled, View, ViewNode,
};

/// Compiles and runs every Rust example in this crate's `README.md`.
///
/// The item only exists while rustdoc is collecting doctests, so it never
/// shows up in the rendered documentation. Its whole purpose is to stop the
/// README from drifting away from the API it advertises.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
