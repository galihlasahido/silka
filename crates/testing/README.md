# silka-testing

The test harness for [silka](../../README.md). It exists to make one claim
checkable: **the UI still looks and behaves the way it did, on every operating
system, at 120 fps.**

Add it as a `dev-dependency`:

```toml
[dev-dependencies]
silka-testing = { path = "../silka/crates/testing" }
```

## Four tools, four questions

| Module | The question it answers |
| --- | --- |
| `headless` | Can this scene be drawn without a window? (texture + readback) |
| `golden` | Does it still look the same? (snapshot files, per preset) |
| `sim` | Does it still behave the same? (synthetic pointer / keyboard) |
| `bench` | Is it still fast enough? (frame time with a failing gate) |

`matrix` insists every visual test runs in all four cells of
preset × appearance, and `png` / `image` / `diff` are the arithmetic
underneath — pure, dependency-free, and unit-tested on their own.

## The shape of a visual test

```rust,no_run
use silka_core::view::fixed;
use silka_testing::{gpu_or_skip, matrix::for_each_case, Simulator, Tolerance};

#[test]
fn card_looks_the_same_in_every_preset() {
    let mut gpu = gpu_or_skip!();
    for_each_case(|case| {
        let mut sim = Simulator::case(case, |_cx| fixed(120.0, 48.0).into());
        sim.settle();
        let capture = sim.capture(&mut gpu);
        case.golden("card").tolerance(Tolerance::SHAPES).assert(&capture);
    });
}
```

`gpu_or_skip!` skips the test when the machine has no usable adapter — unless
`SILKA_REQUIRE_GPU=1`, which turns that skip into a failure, as CI should.

## Why a crate and not a `#[cfg(test)]` module

Before this crate, every gallery page reimplemented its own headless setup:
acquire a device, build an offscreen target, remember to skip when there is no
GPU, hand-roll a scale factor. Copies drift, and a drifted harness tests a
runtime nobody ships. One crate means the app under test is assembled by
`silka_platform::headless_app` — the same assembly `run_app` performs — every
single time.

## Environment variables

| Variable | Effect |
| --- | --- |
| `SILKA_GOLDEN=new\|update` | write missing goldens / overwrite all of them |
| `SILKA_GOLDEN_TOLERANCE` | per-channel allowance, for a noisier driver |
| `SILKA_GOLDEN_RATIO` | fraction of pixels allowed to differ |
| `SILKA_REQUIRE_GPU=1` | "no GPU" becomes a failure instead of a skip |
| `SILKA_BENCH_ITERATIONS` | frames measured per benchmark |
| `SILKA_BENCH_SCALE` | multiplies every frame budget |
| `SILKA_BENCH_FORCE=1` | enforce budgets in debug builds too |

## What it deliberately does not do

- **It does not compress.** Golden PNGs use DEFLATE stored blocks, so the
  project owes no compression dependency.
- **It does not time the GPU.** `bench` measures the CPU frame path, which is
  our code; GPU time on a CI runner measures the runner.
- **It does not chase bit-exactness.** `Tolerance` exists because three drivers
  rasterize the same SDF three slightly different ways, and a suite that
  demands equality is a suite that gets deleted.

## License

MIT OR Apache-2.0
