//! The macOS drag source: `NSDraggingSession` (INTEGRASI-NATIVE §4, §8).
//!
//! AppKit's drag API is built from four pieces, and this file is the only place
//! in the framework that names any of them:
//!
//! 1. **`NSPasteboardItem`** — one per representation on offer. Several items
//!    means "here is the same thing described several ways"; the receiver takes
//!    the first type it understands, which is why [`super::DragSource`] keeps
//!    the application's ordering.
//! 2. **`NSDraggingItem`** — a pasteboard item plus the rectangle its image
//!    occupies, in the **view's** coordinates.
//! 3. **A dragging source object** — an Objective-C object implementing
//!    `NSDraggingSource`. There is no way around defining a class here: the
//!    protocol's `draggingSession:sourceOperationMaskForDraggingContext:` is
//!    what tells AppKit which effects are allowed, and a source that does not
//!    answer it produces a drag nobody can accept.
//! 4. **`-[NSView beginDraggingSessionWithItems:event:source:]`**, which needs
//!    the mouse event the drag began from. A drag started from a timer has no
//!    such event and is refused ([`super::DragError::NoEvent`]) rather than
//!    started at the wrong place on screen.
//!
//! ## Two coordinate conversions, both easy to get wrong
//!
//! The framework speaks logical points with the origin at the **top left**;
//! AppKit's `NSView` may or may not agree, depending on `isFlipped`. Rather
//! than assume winit's view is one or the other, the flag is read and the
//! conversion follows from it. And the preview arrives as physical pixels with
//! a scale factor, so its `NSImage` is given the **logical** size — an image
//! sized in pixels on a Retina display is drawn at twice the size it should be.
//!
//! ## Who keeps the source object alive
//!
//! A dragging session outlives the call that started it, so the source object
//! must too. It is held in a thread-local list — main thread only, which is
//! also AppKit's own rule for all of this — and swept on the next drag rather
//! than removed from inside its own `draggingSession:endedAtPoint:operation:`
//! callback, because dropping the last reference to an object while executing
//! one of its methods is a use-after-free.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSBitmapFormat, NSBitmapImageRep, NSDeviceRGBColorSpace, NSDragOperation,
    NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource, NSImage,
    NSPasteboardItem, NSView,
};
use objc2_foundation::{NSArray, NSData, NSPoint, NSRect, NSSize, NSString};
use silka_paint::Point;

use super::{
    file_url, preview_frame, DragEffect, DragEffects, DragError, DragItem, DragPreview, DragSource,
};
use crate::platform::NativeWindow;

// ---------------------------------------------------------------------------
// Effect translation
// ---------------------------------------------------------------------------

/// Our effect set as AppKit's operation mask.
///
/// A free function rather than a method so it can be tested here without a
/// window: this is the value that decides whether a drop is possible at all.
pub(crate) fn operation_mask(effects: DragEffects) -> NSDragOperation {
    let mut mask = NSDragOperation::None;
    if effects.contains(DragEffects::COPY) {
        mask |= NSDragOperation::Copy;
    }
    if effects.contains(DragEffects::MOVE) {
        mask |= NSDragOperation::Move;
    }
    if effects.contains(DragEffects::LINK) {
        mask |= NSDragOperation::Link;
    }
    mask
}

/// What AppKit says actually happened, as one of ours.
///
/// AppKit reports a *mask*, and a drop that landed nowhere reports
/// `NSDragOperationNone` — which is `None` here, the signal a `Move` source
/// uses to **not** delete the original. `Generic` is AppKit's "some
/// application-defined operation"; the only honest reading of it from outside
/// that application is a copy.
pub(crate) fn effect_from_operation(operation: NSDragOperation) -> Option<DragEffect> {
    if operation.contains(NSDragOperation::Move) {
        Some(DragEffect::Move)
    } else if operation.contains(NSDragOperation::Copy) {
        Some(DragEffect::Copy)
    } else if operation.contains(NSDragOperation::Link) {
        Some(DragEffect::Link)
    } else if operation.contains(NSDragOperation::Generic) {
        Some(DragEffect::Copy)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// The source object
// ---------------------------------------------------------------------------

/// Everything the source object has to remember for the length of a drag.
struct SourceIvars {
    /// The mask handed back to AppKit on every request.
    allowed: NSDragOperation,
    /// The application's completion callback, taken out when the drag ends.
    on_finish: RefCell<Option<Box<dyn FnOnce(Option<DragEffect>)>>>,
    /// Set once the session has ended, so the next drag can sweep this object
    /// away safely.
    finished: Cell<bool>,
}

define_class!(
    // SAFETY:
    // - `NSObject` has no subclassing requirements.
    // - `Source` does not implement `Drop`; its ivars are dropped by objc2.
    // - The class is main-thread-only, which is what `NSDraggingSource`
    //   requires and what AppKit requires of all of this anyway.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = SourceIvars]
    struct Source;

    unsafe impl NSObjectProtocol for Source {}

    unsafe impl NSDraggingSource for Source {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn operation_mask_for_context(
            &self,
            _session: &NSDraggingSession,
            _context: NSDraggingContext,
        ) -> NSDragOperation {
            // The same mask inside and outside the application: a drag whose
            // rules changed halfway across the screen is the sort of thing a
            // user reads as a bug in their own hands.
            self.ivars().allowed
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn ended_at_point(
            &self,
            _session: &NSDraggingSession,
            _screen_point: NSPoint,
            operation: NSDragOperation,
        ) {
            self.ivars().finished.set(true);
            let callback = self.ivars().on_finish.borrow_mut().take();
            if let Some(callback) = callback {
                callback(effect_from_operation(operation));
            }
        }
    }
);

impl Source {
    fn new(
        mtm: MainThreadMarker,
        allowed: NSDragOperation,
        on_finish: Option<Box<dyn FnOnce(Option<DragEffect>)>>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SourceIvars {
            allowed,
            on_finish: RefCell::new(on_finish),
            finished: Cell::new(false),
        });
        // SAFETY: `NSObject`'s designated initializer, called exactly once on a
        // freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    /// Source objects for drags that may still be running.
    ///
    /// Swept at the start of the next drag rather than cleared from inside the
    /// end-of-drag callback: releasing the last reference to an object while
    /// one of its own methods is on the stack is a use-after-free.
    static ALIVE: RefCell<Vec<Retained<Source>>> = const { RefCell::new(Vec::new()) };
}

fn remember(source: Retained<Source>) {
    ALIVE.with(|alive| {
        let mut alive = alive.borrow_mut();
        alive.retain(|s| !s.ivars().finished.get());
        alive.push(source);
    });
}

// ---------------------------------------------------------------------------
// Pasteboard
// ---------------------------------------------------------------------------

/// One pasteboard item per representation, in the application's own order.
///
/// A file item becomes **one item per path**: that is what makes Finder accept
/// a multi-file drag as several files rather than one opaque blob.
fn pasteboard_items(items: &[DragItem]) -> Result<Vec<Retained<NSPasteboardItem>>, DragError> {
    let mut out = Vec::new();
    for item in items {
        match item {
            DragItem::Files(paths) => {
                for path in paths {
                    let url = file_url(path).ok_or(DragError::EmptyItem)?;
                    let pb = NSPasteboardItem::new();
                    let _ = pb.setString_forType(
                        &NSString::from_str(&url),
                        &NSString::from_str(item.uti()),
                    );
                    out.push(pb);
                }
            }
            DragItem::Text(text) => {
                let pb = NSPasteboardItem::new();
                let _ = pb
                    .setString_forType(&NSString::from_str(text), &NSString::from_str(item.uti()));
                out.push(pb);
            }
            DragItem::Url(url) => {
                let pb = NSPasteboardItem::new();
                let _ =
                    pb.setString_forType(&NSString::from_str(url), &NSString::from_str(item.uti()));
                out.push(pb);
            }
            DragItem::Html { html, plain } => {
                let pb = NSPasteboardItem::new();
                let _ = pb
                    .setString_forType(&NSString::from_str(html), &NSString::from_str(item.uti()));
                // The plain-text alternative rides along on the same item, so a
                // receiver that cannot read HTML gets prose instead of tags.
                let _ = pb.setString_forType(
                    &NSString::from_str(plain),
                    &NSString::from_str(DragItem::text("").uti()),
                );
                out.push(pb);
            }
            DragItem::Custom { kind, bytes } => {
                let pb = NSPasteboardItem::new();
                let _ = pb.setData_forType(&NSData::with_bytes(bytes), &NSString::from_str(kind));
                out.push(pb);
            }
        }
    }
    if out.is_empty() {
        return Err(DragError::NoItems);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Preview image
// ---------------------------------------------------------------------------

/// The preview as an `NSImage` sized in **logical points**.
///
/// The bitmap is created with AppKit owning the buffer (`planes` is null) and
/// the bytes are copied in row by row: the variant that borrows the caller's
/// buffer does **not** copy it, and our `RgbaImage` is about to go out of
/// scope.
fn preview_image(preview: &DragPreview) -> Option<Retained<NSImage>> {
    let width = preview.image().width() as isize;
    let height = preview.image().height() as isize;

    // SAFETY: an AppKit string constant, valid for the life of the process.
    let color_space = unsafe { NSDeviceRGBColorSpace };

    // SAFETY: `planes` is null, which asks AppKit to allocate and own the
    // buffer; every other argument describes the 8-bit RGBA layout `RgbaImage`
    // guarantees.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            core::ptr::null_mut(),
            width,
            height,
            8,
            4,
            true,
            false,
            color_space,
            // `RgbaImage` is documented as **not** premultiplied; saying so is
            // the difference between a clean preview and a dark halo.
            NSBitmapFormat::AlphaNonpremultiplied,
            0,
            0,
        )
    }?;

    let dest = rep.bitmapData();
    if dest.is_null() {
        return None;
    }
    let stride = rep.bytesPerRow() as usize;
    let row_bytes = preview.image().width() as usize * 4;
    let src = preview.image().rgba();
    for row in 0..preview.image().height() as usize {
        // SAFETY: `dest` is a buffer AppKit allocated for `height` rows of
        // `stride` bytes, and `row_bytes <= stride` because the bitmap has the
        // same width and 4 bytes per pixel.
        unsafe {
            core::ptr::copy_nonoverlapping(
                src[row * row_bytes..].as_ptr(),
                dest.add(row * stride),
                row_bytes,
            );
        }
    }

    let size = NSSize::new(preview.size().width as f64, preview.size().height as f64);
    rep.setSize(size);
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    image.addRepresentation(&rep);
    Some(image)
}

// ---------------------------------------------------------------------------
// Starting the drag
// ---------------------------------------------------------------------------

/// Start a dragging session (INTEGRASI-NATIVE §4).
///
/// Called by [`super::DragSource::begin`] once the description has been
/// checked; everything here is AppKit.
pub(crate) fn begin(
    mut source: DragSource,
    window: &NativeWindow,
    pointer: Point,
) -> Result<(), DragError> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err(DragError::Os(
            "a drag must be started on the main thread".into(),
        ));
    };
    let Some(view) = window.ns_view() else {
        return Err(DragError::NoWindow);
    };
    // SAFETY: winit hands out the `NSView` of a live window, and the borrowed
    // `window` keeps it alive for this call.
    let view: &NSView = unsafe { view.cast::<NSView>().as_ref() };

    // A dragging session hangs off the event that started it. Without one
    // AppKit would place the drag wherever the last event happened to be.
    let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else {
        return Err(DragError::NoEvent);
    };

    let preview = source.preview_image().ok_or(DragError::NoPreview)?.clone();
    let image = preview_image(&preview)
        .ok_or_else(|| DragError::Os("the preview bitmap could not be created".into()))?;

    // The frame the preview occupies, converted out of the framework's
    // top-left space into whatever the view uses.
    let frame = preview_frame(preview.size(), preview.hotspot(), pointer);
    let view_height = view.bounds().size.height;
    let y = if view.isFlipped() {
        frame.min_y() as f64
    } else {
        view_height - frame.min_y() as f64 - frame.size.height as f64
    };
    let ns_frame = NSRect::new(
        NSPoint::new(frame.min_x() as f64, y),
        NSSize::new(frame.size.width as f64, frame.size.height as f64),
    );

    let pasteboard_items = pasteboard_items(source.items())?;
    let mut dragging_items = Vec::with_capacity(pasteboard_items.len());
    for pb in &pasteboard_items {
        let item = NSDraggingItem::initWithPasteboardWriter(
            NSDraggingItem::alloc(),
            ProtocolObject::from_ref(&**pb),
        );
        let contents: &AnyObject = &image;
        // SAFETY: `contents` is an `NSImage`, which is what
        // `setDraggingFrame:contents:` documents itself as taking.
        unsafe { item.setDraggingFrame_contents(ns_frame, Some(contents)) };
        dragging_items.push(item);
    }

    let allowed = operation_mask(source.allowed());
    let on_finish = source.take_on_finish();
    let dragging_source = Source::new(mtm, allowed, on_finish);

    let _session = view.beginDraggingSessionWithItems_event_source(
        &NSArray::from_retained_slice(&dragging_items),
        &event,
        ProtocolObject::from_ref(&*dragging_source),
    );
    remember(dragging_source);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topeng_operasi_menyalin_setiap_bit() {
        assert_eq!(operation_mask(DragEffects::NONE), NSDragOperation::None);
        assert!(operation_mask(DragEffects::ALL).contains(NSDragOperation::Copy));
        assert!(operation_mask(DragEffects::ALL).contains(NSDragOperation::Move));
        assert!(operation_mask(DragEffects::ALL).contains(NSDragOperation::Link));
        assert!(!operation_mask(DragEffects::COPY).contains(NSDragOperation::Move));
    }

    #[test]
    fn drop_yang_mendarat_di_mana_mana_bukan_move() {
        // The property a `Move` source depends on: a cancelled drag must not
        // report an effect, or the original file is deleted for nothing.
        assert_eq!(effect_from_operation(NSDragOperation::None), None);
        assert_eq!(
            effect_from_operation(NSDragOperation::Move),
            Some(DragEffect::Move)
        );
        assert_eq!(
            effect_from_operation(NSDragOperation::Copy),
            Some(DragEffect::Copy)
        );
        assert_eq!(
            effect_from_operation(NSDragOperation::Link),
            Some(DragEffect::Link)
        );
        // AppKit's "some application-defined operation" reads as a copy from
        // outside that application — the only non-destructive answer.
        assert_eq!(
            effect_from_operation(NSDragOperation::Generic),
            Some(DragEffect::Copy)
        );
    }

    #[test]
    fn move_menang_atas_copy_dalam_satu_topeng() {
        // A receiver that reports both did the destructive one; treating it as
        // a copy would leave a duplicate behind.
        let both = NSDragOperation::Move | NSDragOperation::Copy;
        assert_eq!(effect_from_operation(both), Some(DragEffect::Move));
    }
}
