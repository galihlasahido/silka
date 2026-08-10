//! macOS vsync through `CADisplayLink` (REKOMENDASI §3.5, INTEGRASI-NATIVE §8).
//!
//! `CADisplayLink` is the only correct way to obtain Apple's display clock: it
//! follows **ProMotion** (adaptive 24–120 Hz), follows the window when it moves
//! to a monitor with a different refresh rate, and reports
//! `timestamp`/`targetTimestamp` so the real interval can be read instead of
//! assumed to be 16.6 ms.
//!
//! Important details:
//!
//! - The link is created from the window's **NSView**
//!   (`-[NSView displayLinkWithTarget:selector:]`, macOS 14+) so that it is
//!   bound to the screen the window is really on. If that selector is missing
//!   (older macOS), `attach` returns `None` and the caller falls back to
//!   winit's `request_redraw`.
//! - The link is installed in `NSRunLoopCommonModes`, not the default mode —
//!   otherwise animation stops while the window is being resized or a menu is
//!   open.
//! - The link is **born paused**. It only ticks while something is dirty
//!   (§3.5: idle must really be idle).
//! - `invalidate()` is mandatory on drop: `CADisplayLink` retains its target,
//!   and our target holds an `Arc<Window>`.

use std::sync::Arc;
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes};
use objc2_quartz_core::CADisplayLink;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use super::VsyncClock;

/// What happens when the display ticks. In practice:
/// `window.request_redraw()`.
type Notify = Box<dyn Fn()>;

struct TargetIvars {
    clock: Arc<VsyncClock>,
    notify: Notify,
}

define_class!(
    // SAFETY:
    // - The NSObject superclass imposes no subclassing requirements.
    // - This type does not implement `Drop`; the ivars are cleaned up by the
    //   `dealloc` the macro generates.
    #[unsafe(super(NSObject))]
    // Display link callbacks are delivered on the main run loop, and `notify`
    // touches the winit window — both are main-thread-only.
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    struct DisplayLinkTarget;

    impl DisplayLinkTarget {
        #[unsafe(method(silkaDisplayLinkFired:))]
        fn fired(&self, link: &CADisplayLink) {
            let ivars = self.ivars();
            ivars.clock.tick(interval_of(link));
            (ivars.notify)();
        }
    }
);

/// The interval until the next vsync, according to the OS.
///
/// `targetTimestamp - timestamp` is a genuinely adaptive number: on ProMotion
/// it changes as the system raises or lowers the display rate.
fn interval_of(link: &CADisplayLink) -> Option<Duration> {
    let delta = link.targetTimestamp() - link.timestamp();
    let seconds = if delta.is_finite() && delta > 0.0 {
        delta
    } else {
        let d = link.duration();
        if d.is_finite() && d > 0.0 {
            d
        } else {
            return None;
        }
    };
    Some(Duration::from_secs_f64(seconds))
}

/// A display link that lives as long as its window does.
pub(super) struct DisplayLink {
    link: Retained<CADisplayLink>,
    _target: Retained<DisplayLinkTarget>,
}

impl DisplayLink {
    /// Attach a display link to `window`'s NSView.
    ///
    /// Returns `None` — without panicking and without fuss — when the platform
    /// does not provide the API; the caller then uses the fallback path.
    pub(super) fn attach(
        window: &Arc<Window>,
        clock: Arc<VsyncClock>,
        notify: impl Fn() + 'static,
    ) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;

        let handle = window.window_handle().ok()?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return None;
        };
        // SAFETY: winit guarantees `ns_view` is an NSView that stays alive as
        // long as `window` does, and `handle` borrows that window.
        let view: &AnyObject = unsafe { appkit.ns_view.cast::<AnyObject>().as_ref() };

        // `-[NSView displayLinkWithTarget:selector:]` only exists on macOS 14+.
        // SAFETY: `respondsToSelector:` exists on every NSObject.
        if !unsafe { responds_to(view, sel!(displayLinkWithTarget:selector:)) } {
            return None;
        }

        let target = DisplayLinkTarget::new(mtm, clock.clone(), Box::new(notify));

        // SAFETY: the selector above is known to exist; `target` responds to
        // `silkaDisplayLinkFired:` with a matching signature.
        let link: Option<Retained<CADisplayLink>> = unsafe {
            msg_send![
                view,
                displayLinkWithTarget: &*target,
                selector: sel!(silkaDisplayLinkFired:),
            ]
        };
        let link = link?;

        // SAFETY: called from the main thread, on the main run loop.
        unsafe {
            link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes);
        }
        // Idle must really be idle: the link only ticks once something is dirty.
        link.setPaused(true);

        // A seed value so even the first frame has a correct budget; the first
        // real tick replaces it with a more accurate number.
        if let Some(seed) = unsafe { screen_interval(view) } {
            clock.seed_interval(seed);
        }

        Some(Self {
            link,
            _target: target,
        })
    }

    /// Stop or resume the clock.
    pub(super) fn set_paused(&self, paused: bool) {
        if self.link.isPaused() != paused {
            self.link.setPaused(paused);
        }
    }
}

impl Drop for DisplayLink {
    fn drop(&mut self) {
        // SAFETY: called from the main thread — `DisplayLink` is not `Send`.
        unsafe {
            self.link
                .removeFromRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes);
        }
        // Breaks the CADisplayLink → target → Arc<Window> retain cycle.
        self.link.invalidate();
    }
}

impl DisplayLinkTarget {
    fn new(mtm: MainThreadMarker, clock: Arc<VsyncClock>, notify: Notify) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { clock, notify });
        // SAFETY: plain NSObject `init`, no arguments.
        unsafe { msg_send![super(this), init] }
    }
}

/// The maximum refresh rate of the screen the window is on, used as a seed.
///
/// `-[NSScreen maximumFramesPerSecond]` gives 120 on ProMotion and 60 on an
/// ordinary display — already correct before the first tick arrives.
///
/// # Safety
///
/// `view` must be a live NSView.
unsafe fn screen_interval(view: &AnyObject) -> Option<Duration> {
    let window: *mut AnyObject = unsafe { msg_send![view, window] };
    let window = unsafe { window.as_ref() }?;
    let screen: *mut AnyObject = unsafe { msg_send![window, screen] };
    let screen = unsafe { screen.as_ref() }?;
    if !unsafe { responds_to(screen, sel!(maximumFramesPerSecond)) } {
        return None;
    }
    let fps: isize = unsafe { msg_send![screen, maximumFramesPerSecond] };
    if fps <= 0 {
        return None;
    }
    Some(Duration::from_secs_f64(1.0 / fps as f64))
}

/// Whether `obj` responds to `selector` — how to probe for a newer API without
/// raising the deployment target of the whole application.
///
/// # Safety
///
/// `obj` must be a live Objective-C object.
unsafe fn responds_to(obj: &AnyObject, selector: Sel) -> bool {
    unsafe { msg_send![obj, respondsToSelector: selector] }
}
