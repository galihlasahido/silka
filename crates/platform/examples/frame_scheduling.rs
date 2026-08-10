//! Live proof of the render-on-dirty scheduler (REKOMENDASI §3.5).
//!
//! Run it, then watch stderr:
//!
//! ```text
//! cargo run -p silka-platform --example frame_scheduling
//! ```
//!
//! What you should see:
//!
//! 1. The opening line names the vsync source — on a ProMotion Mac it reads
//!    `vsync 120.0 Hz (display-link) (CADisplayLink)`, not 60 Hz and not a
//!    hardcoded constant.
//! 2. For the first three seconds the background pulses because every frame
//!    calls [`silka_platform::FrameContext::request_animation_frame`]; frame
//!    time logs keep flowing.
//! 3. Once the animation finishes, **the log stops completely** — not a single
//!    frame is drawn until the window is resized, the OS dark mode changes, or
//!    the window is closed. That is the whole point of "render only when
//!    dirty".

use std::time::Duration;

use silka_paint::Scene;
use silka_platform::{window, PlatformError};

/// How long the pulse runs before the window goes completely still again.
const DURASI_ANIMASI: Duration = Duration::from_secs(3);

fn main() -> Result<(), PlatformError> {
    window("silka — frame scheduling")
        .size(720.0, 480.0)
        .on_frame(|frame| {
            let t = frame.elapsed();
            if t >= DURASI_ANIMASI {
                // No further frame requested → the window goes to sleep.
                return Scene::new(frame.theme().color.background);
            }

            // While animating, each frame books the next one. Later on this
            // caller will be a spring that has not reached its target (§3.5).
            frame.request_animation_frame();

            let fase = (t.as_secs_f32() * 2.0).sin() * 0.5 + 0.5;
            Scene::new(
                frame
                    .theme()
                    .color
                    .background
                    .lerp(frame.theme().color.accent, fase * 0.35),
            )
        })
        // Summarise every 30 frames so even a short pulse is visible.
        .frame_log_every(30)
        .run()
}
