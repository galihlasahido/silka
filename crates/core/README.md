# silka-core

The engine underneath [silka](../../README.md)'s public API. Application code
normally talks to `silka-widgets`; this crate is what that catalogue is built
out of.

## The layers it hosts

| Layer | Module | What it is |
| --- | --- | --- |
| State | `signals` | Signals with per-scope dependency tracking — a write marks exactly the components that read it dirty |
| View | `view` | A lightweight Dart-style view tree, rebuilt on every update and **diffed** into the render tree |
| Render tree | `tree` | A retained tree in a generational-ID arena; layout, paint, hit-test, and accessibility all address it by id |
| Layout | `tree` | Flutter-style box constraints ("constraints go down, sizes come up"), with Taffy driving Flex/Grid inside a single module |
| Motion | `animation` | Springs storing `(position, velocity)` — always interruptible, always retargetable |
| Input | `input` | Hit-testing, focus and tab order, velocity tracking, IME |
| Accessibility | `access` | AccessKit nodes emitted as a render-tree pass, not a later layer |
| Frame | `scheduler` | Render **only when dirty**; the vsync interval is supplied by the platform, never assumed |
| Lifecycle | `app` | `AppRuntime` joining all of the above into one frame |

## One frame

```rust
use silka_core::scheduler::FrameScheduler;
use silka_core::tree::{BoxConstraints, RenderTree};
use silka_core::view::{column, fixed, reconcile};
use silka_paint::Size;

let mut scheduler = FrameScheduler::new();
let mut tree = RenderTree::new();

// 1. A component rebuilds → a new view → diff it into the render tree.
reconcile(&mut tree, column([fixed(120.0, 24.0)]).spacing(8.0));
// 2. What changed decides whether the renderer needs waking at all.
scheduler.request(tree.take_dirty());
// 3. Layout: full when the window resizes, subtree-only otherwise.
tree.perform_layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
```

## Render only when dirty

This is the promise the whole crate is arranged around. A signal write wakes
the scheduler; a spring that is still moving keeps it awake; the moment
everything settles, `AppRuntime::is_idle` is true again and **not one timer is
ticking**. There is no hardcoded 16.6 ms anywhere — while the display's
interval is unknown it is `None`, and nothing pretends to know better.

## Springs, not curves

```rust
use std::time::Duration;
use silka_core::animation::{Motion, Spring, SpringValue};

let mut value = SpringValue::new(0.0).with_spring(Spring::snappy());
value.set_target(100.0);
value.advance(Duration::from_millis(16), Motion::Full);
assert!(value.position() > 0.0);

// Retargeting mid-flight keeps the velocity, so there is no seam.
value.set_target(40.0);
assert!(value.is_animating());
```

Because a spring carries velocity, a fling gesture hands its measured velocity
straight to `set_target` — that is the gesture → animation handoff. `Motion`
carries the system *reduce motion* setting through the same call, so honoring
it is not a per-widget decision.

## Utility styling

The §2.6 vocabulary — `flex()`, `items_center()`, `p_4()`, `rounded_lg()`,
`bg()` — lives here, in Tailwind's spelling but on HIG's numbers. The point is
*who supplies the values*: the normal path takes only a token, so a stray
literal does not compile, and a brand color that genuinely is not a token has
to go through a conspicuously named escape hatch (`bg_raw`, `p_raw`).

```rust
use silka_core::view::{fixed, interactive};
use silka_theme::ColorToken;

let _ = interactive(fixed(240.0, 88.0))
    .bg(ColorToken::Surface)
    .rounded_lg()
    .hover(|s| s.bg(ColorToken::SurfaceHover))
    .pressed(|s| s.bg(ColorToken::SurfacePressed).scale(0.98))
    .focused(|s| s.ring(ColorToken::FocusRing));
```

Each state is a delta over the resting style, and the transition between them
is a spring owned by the node — so motion is a property of the system rather
than something every widget re-implements.

## Boundaries

- **No wgpu.** The paint pass speaks only `silka-paint` commands.
- **No winit.** The scheduler receives a vsync tick; it does not know where it
  came from.
- **`taffy::` never escapes one module.** The public layout vocabulary
  (`ContainerStyle`, `ItemStyle`, `Track`) is this crate's own.
- **`RenderNode::access` is a required method.** A widget that forgot to think
  about screen readers does not compile.

## License

MIT OR Apache-2.0
