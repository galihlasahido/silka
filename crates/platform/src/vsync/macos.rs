//! Vsync macOS lewat `CADisplayLink` (REKOMENDASI §3.5, INTEGRASI-NATIVE §8).
//!
//! `CADisplayLink` adalah satu-satunya cara benar mendapat detak layar Apple:
//! ia ikut **ProMotion** (24–120 Hz adaptif), ikut saat window dipindah ke
//! monitor dengan laju berbeda, dan melaporkan `timestamp`/`targetTimestamp`
//! sehingga interval sesungguhnya bisa dibaca — bukan diasumsikan 16,6 ms.
//!
//! Detail penting:
//!
//! - Link dibuat dari **NSView** window (`-[NSView displayLinkWithTarget:selector:]`,
//!   macOS 14+) sehingga ia terikat ke layar tempat window benar-benar berada.
//!   Bila selector itu tidak ada (macOS lebih tua), `attach` mengembalikan
//!   `None` dan pemanggil turun ke `request_redraw` winit.
//! - Link dipasang di `NSRunLoopCommonModes`, bukan default mode — kalau tidak,
//!   animasi berhenti saat window sedang di-resize atau menu sedang terbuka.
//! - Link **lahir dalam keadaan paused**. Ia hanya berdetak selama ada yang
//!   dirty (§3.5: idle harus benar-benar idle).
//! - `invalidate()` wajib saat drop: `CADisplayLink` me-retain target-nya, dan
//!   target kita memegang `Arc<Window>`.

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

/// Apa yang dilakukan saat layar berdetak. Di praktiknya:
/// `window.request_redraw()`.
type Notify = Box<dyn Fn()>;

struct TargetIvars {
    clock: Arc<VsyncClock>,
    notify: Notify,
}

define_class!(
    // SAFETY:
    // - Superclass NSObject tidak punya syarat subclassing.
    // - Tipe ini tidak mengimplementasikan `Drop`; ivars dibersihkan oleh
    //   `dealloc` yang dihasilkan macro.
    #[unsafe(super(NSObject))]
    // Callback display link dikirim ke run loop utama, dan `notify` menyentuh
    // window winit — keduanya main-thread-only.
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

/// Interval sampai vsync berikutnya menurut OS.
///
/// `targetTimestamp - timestamp` adalah angka yang benar-benar adaptif: di
/// ProMotion ia berubah saat sistem menaikkan/menurunkan laju layar.
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

/// Display link yang hidup selama window-nya hidup.
pub(super) struct DisplayLink {
    link: Retained<CADisplayLink>,
    _target: Retained<DisplayLinkTarget>,
}

impl DisplayLink {
    /// Pasang display link ke NSView milik `window`.
    ///
    /// Mengembalikan `None` — tanpa panic dan tanpa keributan — bila platform
    /// tidak menyediakan API-nya; pemanggil lalu memakai jalur fallback.
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
        // SAFETY: winit menjamin `ns_view` adalah NSView hidup selama `window`
        // hidup, dan `handle` meminjam window itu.
        let view: &AnyObject = unsafe { appkit.ns_view.cast::<AnyObject>().as_ref() };

        // `-[NSView displayLinkWithTarget:selector:]` baru ada di macOS 14.
        // SAFETY: `respondsToSelector:` ada di setiap NSObject.
        if !unsafe { responds_to(view, sel!(displayLinkWithTarget:selector:)) } {
            return None;
        }

        let target = DisplayLinkTarget::new(mtm, clock.clone(), Box::new(notify));

        // SAFETY: selector di atas sudah dipastikan ada; `target` merespons
        // `silkaDisplayLinkFired:` dengan tanda tangan yang cocok.
        let link: Option<Retained<CADisplayLink>> = unsafe {
            msg_send![
                view,
                displayLinkWithTarget: &*target,
                selector: sel!(silkaDisplayLinkFired:),
            ]
        };
        let link = link?;

        // SAFETY: dipanggil dari main thread, run loop utama.
        unsafe {
            link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes);
        }
        // Idle harus benar-benar idle: link baru berdetak saat ada yang dirty.
        link.setPaused(true);

        // Nilai awal supaya frame pertama pun punya budget yang benar; tick
        // pertama akan menggantinya dengan angka yang lebih tepat.
        if let Some(seed) = unsafe { screen_interval(view) } {
            clock.seed_interval(seed);
        }

        Some(Self {
            link,
            _target: target,
        })
    }

    /// Hentikan/lanjutkan detak.
    pub(super) fn set_paused(&self, paused: bool) {
        if self.link.isPaused() != paused {
            self.link.setPaused(paused);
        }
    }
}

impl Drop for DisplayLink {
    fn drop(&mut self) {
        // SAFETY: dipanggil dari main thread — `DisplayLink` tidak `Send`.
        unsafe {
            self.link
                .removeFromRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes);
        }
        // Memutus retain cycle CADisplayLink → target → Arc<Window>.
        self.link.invalidate();
    }
}

impl DisplayLinkTarget {
    fn new(mtm: MainThreadMarker, clock: Arc<VsyncClock>, notify: Notify) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { clock, notify });
        // SAFETY: `init` NSObject tanpa argumen.
        unsafe { msg_send![super(this), init] }
    }
}

/// Laju maksimum layar tempat window berada, sebagai nilai awal.
///
/// `-[NSScreen maximumFramesPerSecond]` memberi 120 di ProMotion dan 60 di
/// layar biasa — sudah benar sebelum tick pertama datang.
///
/// # Safety
///
/// `view` harus NSView yang hidup.
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

/// Apakah `obj` merespons `selector` — cara memeriksa ketersediaan API yang
/// lebih baru tanpa menaikkan deployment target seluruh aplikasi.
///
/// # Safety
///
/// `obj` harus objek Objective-C yang hidup.
unsafe fn responds_to(obj: &AnyObject, selector: Sel) -> bool {
    unsafe { msg_send![obj, respondsToSelector: selector] }
}
