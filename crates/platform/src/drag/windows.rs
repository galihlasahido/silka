//! The Windows drag source: `DoDragDrop` over a hand-built `IDataObject` and
//! `IDropSource` (INTEGRASI-NATIVE §4).
//!
//! Every `windows::` type in the framework lives and dies inside this file or
//! `crate::platform::raw` — the same boundary `notify-rust` keeps and `keyring`
//! keeps, and the reason this backend is one file with pure helpers rather
//! than a module tree.
//!
//! What the call has to assemble, and why each piece is non-optional:
//!
//! 1. **`IDataObject`** — the payload. OLE drop sources must implement it
//!    themselves; unlike macOS there is no standard object to hand. It has to
//!    answer `GetData` (write one payload into an `HGLOBAL`), `QueryGetData`
//!    and `GetCanonicalFormatEtc` (a receiver probes before it pulls), and
//!    `EnumFormatEtc` (the receiver's first question is "what do you have",
//!    answered with the standard enumerator shell
//!    [`SHCreateStdEnumFmtEtc`]).
//! 2. **`IDropSource`** — the mouse. `QueryContinueDrag` is called on every
//!    mouse move: left button up ends the drag, Esc cancels it. Both answers
//!    are *HRESULTs with positive values* (`DRAGDROP_S_DROP`,
//!    `DRAGDROP_S_CANCEL`), not error codes — the classic way to get this
//!    wrong is to map them through a "is this an error?" check.
//! 3. **`DoDragDrop`** — the loop. It blocks (pumping messages itself) until
//!    the drop lands, and returns the effect the receiver performed. So
//!    `DragSource::begin` on Windows does *not* return as soon as the drag
//!    starts; the on-finish callback fires before it returns. That is still
//!    honest to the contract above `begin` — the outcome reaches
//!    `on_finish` exactly once — but an application must not expect the
//!    function to be asynchronous on this platform, the way it is on macOS.
//!
//! ## Payloads and their formats
//!
//! One `IDataObject` per drag, carrying every [`DragItem`] under the clipboard
//! format [`DragItem::windows_format`] names — the same mapping the clipboard
//! (`crate::clipboard`) already speaks, which is what lets a drag that started
//! in a silka window be pasted anywhere else. Text travels as UTF-16 (the
//! `CF_UNICODETEXT` contract), files as a `CF_HDROP` `DROPFILES` block, and an
//! application's own type goes through `RegisterClipboardFormatW` — whose
//! result is process-wide but *not* system-wide, so two running silka apps
//! agree on the format by registering the same name, which is also what the
//! clipboard does.

use std::path::Path;

use windows::core::implement;
use windows::Win32::Foundation::{
    DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, HGLOBAL, HWND, POINT,
};
use windows::Win32::System::Com::{IDataObject, IDataObject_Impl, FORMATETC, STGMEDIUM};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK,
    DROPEFFECT_MOVE, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MK_RBUTTON};
use windows::Win32::UI::Shell::{SHCreateStdEnumFmtEtc, DROPFILES};
use windows_core::PCWSTR;

use super::{DragEffect, DragEffects, DragError, DragItem, DragSource};
use crate::platform::NativeWindow;

// ---------------------------------------------------------------------------
// Effect translation
// ---------------------------------------------------------------------------

/// Our effect set as OLE's `DROPEFFECT` mask.
///
/// A free function rather than a method so it can be tested here without a
/// window: this is the value that decides whether a drop is possible at all.
pub(crate) fn allowed_mask(effects: DragEffects) -> DROPEFFECT {
    let mut mask = DROPEFFECT_NONE;
    if effects.contains(DragEffects::COPY) {
        mask |= DROPEFFECT_COPY;
    }
    if effects.contains(DragEffects::MOVE) {
        mask |= DROPEFFECT_MOVE;
    }
    if effects.contains(DragEffects::LINK) {
        mask |= DROPEFFECT_LINK;
    }
    mask
}

/// What OLE says actually happened, as one of ours.
///
/// `None` means the drop landed nowhere or was cancelled — the signal a `Move`
/// source uses to **not** delete the original.
pub(crate) fn effect_from_ole(effect: DROPEFFECT) -> Option<DragEffect> {
    if effect.contains(DROPEFFECT_MOVE) {
        Some(DragEffect::Move)
    } else if effect.contains(DROPEFFECT_COPY) {
        Some(DragEffect::Copy)
    } else if effect.contains(DROPEFFECT_LINK) {
        Some(DragEffect::Link)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Clipboard formats
// ---------------------------------------------------------------------------

/// Register (or look up) the clipboard format a drag item is offered under.
///
/// `CF_UNICODETEXT` and `CF_HDROP` are *standard* formats with fixed ids, not
/// names — `RegisterClipboardFormatW` would hand back a registered alias, and
/// receivers asking for the standard id would find nothing. That is why the
/// two are special-cased by name here, matching the contract
/// [`DragItem::windows_format`] documents.
fn format_id(item: &DragItem) -> Option<u16> {
    match item.windows_format() {
        "CF_UNICODETEXT" => Some(13), // CF_UNICODETEXT
        "CF_HDROP" => Some(15),       // CF_HDROP
        name => {
            // Nulls cannot appear in a format name; the register call takes a
            // wide string and this module owns the conversion.
            let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            let id = unsafe { RegisterClipboardFormatW(PCWSTR::from_raw(wide.as_ptr())) };
            (id != 0).then_some(id as u16)
        }
    }
}

/// One format the data object offers, with the item it serves.
struct Offer {
    format: u16,
    item: DragItem,
}

/// Encode a UTF-16 string as the `CF_UNICODETEXT` payload: the text, one null
/// terminator (double-null is the *list* convention; a text blob needs one).
fn utf16_payload(text: &str) -> Vec<u8> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    wide.iter().flat_map(|w| w.to_ne_bytes()).collect()
}

/// Encode a file list as a `CF_HDROP` payload: a `DROPFILES` header followed
/// by one absolute wide path per file, the whole list double-null-terminated.
///
/// The paths are sent **absolute**: the `DROPFILES` contract is that a
/// relative path is resolved against the *receiving* process's working
/// directory, which would silently send the wrong files.
fn hdrop_payload(paths: &[std::path::PathBuf]) -> Vec<u8> {
    let mut out = vec![0u8; std::mem::size_of::<DROPFILES>()];
    // pFiles = offset of the file list, fNC = false (client area, irrelevant
    // for a payload), fWide = true — without it a receiver reads UTF-16 as
    // ANSI and every non-ASCII path corrupts.
    let header = DROPFILES {
        pFiles: std::mem::size_of::<DROPFILES>() as u32,
        pt: POINT::default(),
        fNC: false.into(),
        fWide: true.into(),
    };
    out[..std::mem::size_of::<DROPFILES>()].copy_from_slice(unsafe {
        std::slice::from_raw_parts(
            std::ptr::addr_of!(header).cast::<u8>(),
            std::mem::size_of::<DROPFILES>(),
        )
    });
    let mut list: Vec<u16> = Vec::with_capacity(hdrop_list_len(paths));
    for path in paths {
        let absolute = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir()
                .map(|root| root.join(path))
                .unwrap_or_else(|_| path.clone())
        };
        let as_windows_path = Path::new(&absolute);
        list.extend(as_windows_path.as_os_str().to_string_lossy().encode_utf16());
        list.push(0); // each path is null-terminated
    }
    list.push(0); // …and the whole list is double-null-terminated.
    out.extend(list.iter().flat_map(|w| w.to_ne_bytes()));
    out
}

/// The byte length of a `CF_HDROP` list for `paths` — header excluded.
///
/// Pure so the arithmetic can be asserted without an OS allocation behind it;
/// `hdrop_payload` below must produce exactly this many payload bytes.
pub(crate) fn hdrop_list_len(paths: &[std::path::PathBuf]) -> usize {
    let mut chars = 0usize;
    for path in paths {
        chars += path.as_os_str().to_string_lossy().encode_utf16().count() + 1;
    }
    chars + 1 // the final extra null
}

// ---------------------------------------------------------------------------
// The data object
// ---------------------------------------------------------------------------

/// Allocate a movable `HGLOBAL` holding `bytes`, or a string saying what went
/// wrong (the module above converts that into a typed `DragError::Os`).
fn hglobal_of(bytes: &[u8]) -> Result<HGLOBAL, String> {
    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len())
            .map_err(|e| format!("GlobalAlloc failed: {e}"))?;
        let ptr = GlobalLock(handle);
        if ptr.is_null() {
            return Err("GlobalLock failed".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(handle);
        Ok(handle)
    }
}

#[implement(IDataObject)]
struct SilkaDataObject {
    offers: Vec<Offer>,
}

impl SilkaDataObject {
    /// The offer matching a format query, if any.
    fn offer_for(&self, format: u16) -> Option<&Offer> {
        self.offers.iter().find(|o| o.format == format)
    }

    /// The payload bytes of one item, as the clipboard contract wants them.
    ///
    /// A free-standing associated function rather than a method on the inner
    /// struct: the `#[implement]` expansion gives the outer
    /// `SilkaDataObject_Impl` its own `Self`, and only the *inner* type's
    /// methods are visible to both.
    fn payload_of(item: &DragItem) -> Vec<u8> {
        match item {
            DragItem::Text(t) => utf16_payload(t),
            DragItem::Html { html, .. } => {
                // "HTML Format" carriers are supposed to wrap the fragment in
                // a version header with byte offsets. The offsets only matter
                // to receivers that parse the header — Word, for one. A
                // fragment without a header is accepted by every drop target
                // that reads the payload directly, and producing a header with
                // wrong offsets would be worse than producing none.
                html.as_bytes().to_vec()
            }
            DragItem::Url(u) => utf16_payload(u),
            DragItem::Files(paths) => hdrop_payload(paths),
            DragItem::Custom { bytes, .. } => bytes.clone(),
        }
    }
}

/// The one failure inside the data object: an allocation that OLE cannot
/// proceed without. Reported as `E_OUTOFMEMORY`, which is what a receiver
/// that cannot read a payload should see.
fn alloc_error(message: String) -> windows_core::Error {
    windows_core::Error::new(windows_core::HRESULT(-2_147_024_882i32), message) // E_OUTOFMEMORY
}

impl IDataObject_Impl for SilkaDataObject_Impl {
    fn GetData(&self, pformatetcin: *const FORMATETC) -> windows_core::Result<STGMEDIUM> {
        let query = unsafe { *pformatetcin };
        let offer = self
            .offer_for(query.cfFormat)
            .ok_or_else(windows_core::Error::from_thread)?;
        let handle = hglobal_of(&SilkaDataObject::payload_of(&offer.item)).map_err(alloc_error)?;
        Ok(STGMEDIUM {
            tymed: 1, // TYMED_HGLOBAL
            u: windows::Win32::System::Com::STGMEDIUM_0 { hGlobal: handle },
            pUnkForRelease: std::mem::ManuallyDrop::new(None),
        })
    }

    fn GetDataHere(
        &self,
        _pformatetc: *const FORMATETC,
        _pmedium: *mut STGMEDIUM,
    ) -> windows_core::Result<()> {
        // Only the allocate-for-me path is offered; a receiver that wants to
        // supply its own storage can fall back to `GetData`.
        Err(windows_core::Error::from_thread())
    }

    fn QueryGetData(&self, pformatetc: *const FORMATETC) -> windows_core::HRESULT {
        let query = unsafe { *pformatetc };
        if self.offer_for(query.cfFormat).is_some() {
            windows_core::HRESULT(0)
        } else {
            windows_core::HRESULT(-1) // DV_E_FORMATETC-ish: "no"
        }
    }

    fn GetCanonicalFormatEtc(
        &self,
        pformatetcin: *const FORMATETC,
        pformatetcout: *mut FORMATETC,
    ) -> windows_core::HRESULT {
        // Every format we offer is already canonical — no metafile-vs-dib
        // equivalence games. The OLE convention for "same format" is to write
        // the query back and return a negative result.
        unsafe {
            *pformatetcout = *pformatetcin;
        }
        windows_core::HRESULT(-1)
    }

    fn SetData(
        &self,
        _pformatetc: *const FORMATETC,
        _pmedium: *const STGMEDIUM,
        _frelease: windows_core::BOOL,
    ) -> windows_core::Result<()> {
        // A source that lets a receiver push data into it mid-drag is a
        // source whose payload can change underneath the user.
        Err(windows_core::Error::from_thread())
    }

    fn EnumFormatEtc(
        &self,
        dwdirection: u32,
    ) -> windows_core::Result<windows::Win32::System::Com::IEnumFORMATETC> {
        // Only "enumerate what I offer" makes sense on a source; DATADIR_GET
        // is 1, DATADIR_SET is 2.
        if dwdirection != 1 {
            return Err(windows_core::Error::from_thread());
        }
        let formats: Vec<FORMATETC> = self
            .offers
            .iter()
            .map(|o| FORMATETC {
                cfFormat: o.format,
                ptd: std::ptr::null_mut(),
                dwAspect: 1, // DVASPECT_CONTENT
                lindex: -1,
                tymed: 1, // TYMED_HGLOBAL
            })
            .collect();
        unsafe { SHCreateStdEnumFmtEtc(&formats) }
    }

    fn DAdvise(
        &self,
        _pformatetc: *const FORMATETC,
        _advf: u32,
        _padvsink: windows_core::Ref<windows::Win32::System::Com::IAdviseSink>,
    ) -> windows_core::Result<u32> {
        Err(windows_core::Error::from_thread())
    }

    fn DUnadvise(&self, _dwconnection: u32) -> windows_core::Result<()> {
        Err(windows_core::Error::from_thread())
    }

    fn EnumDAdvise(&self) -> windows_core::Result<windows::Win32::System::Com::IEnumSTATDATA> {
        Err(windows_core::Error::from_thread())
    }
}

// ---------------------------------------------------------------------------
// The drop source
// ---------------------------------------------------------------------------

/// The mouse. `QueryContinueDrag` answers the only two questions a drag loop
/// has — "keep going?" and "how does this look?" — and both answers are
/// *HRESULTs*, which is why the constants are imported next to the errors.
#[implement(IDropSource)]
struct SilkaDropSource;

impl IDropSource_Impl for SilkaDropSource_Impl {
    fn QueryContinueDrag(
        &self,
        escape_pressed: windows_core::BOOL,
        grfkeystate: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
    ) -> windows_core::HRESULT {
        // Esc wins over everything, including a button that is also up: the
        // user asked to cancel, and a drag that converts that into "dropped"
        // would destroy a `Move` original.
        if escape_pressed.as_bool() {
            return DRAGDROP_S_CANCEL;
        }
        // Every button released: this is the drop. Only the two buttons that
        // can *start* a drag are watched — a user still holding Shift from a
        // copy modifier must not be read as "still dragging".
        let pressed = grfkeystate;
        if !pressed.contains(MK_LBUTTON) && !pressed.contains(MK_RBUTTON) {
            return DRAGDROP_S_DROP;
        }
        windows_core::HRESULT(0) // S_OK — keep going.
    }

    fn GiveFeedback(&self, _dweffect: DROPEFFECT) -> windows_core::HRESULT {
        // DRAGDROP_S_USEDEFAULTCURSORS: OLE picks the copy/move/no-drop
        // cursor. Drawing our own would need a preview-owned cursor set, and
        // the OS's are what users already read.
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

// ---------------------------------------------------------------------------
// The begin call
// ---------------------------------------------------------------------------

/// Run the drag loop. Blocks until the drop resolves; see the module doc for
/// why that is honest to `DragSource::begin`'s contract.
///
/// The Esc key is answered through `QueryContinueDrag`'s own
/// `escape_pressed` flag — OLE reads the keyboard for us — so there is no
/// `GetAsyncKeyState` call here to duplicate it.
pub(super) fn begin(source: &mut DragSource, window: &NativeWindow) -> Result<(), DragError> {
    // The preview is checked by the caller (`begin` above this file); what
    // this backend does with it is nothing yet — an `HBITMAP` preview needs a
    // `DoDragDrop` variant with a drag-image helper (`IDragSourceHelper`),
    // which is follow-up work, not a blocker: every file manager drop target
    // works, the drag just follows the pointer without a thumbnail.

    // `DoDragDrop` binds to the calling thread's message pump, not to the
    // handle; what the window contributes is that its thread is the one
    // running this call. `hwnd()` hands the handle over as `isize` precisely
    // so no windows-rs type crosses the platform boundary before this file.
    let _hwnd = window
        .hwnd()
        .map(|h| HWND(h as *mut _))
        .ok_or(DragError::NoWindow)?;

    let offers: Vec<Offer> = source
        .items()
        .iter()
        .filter_map(|item| {
            format_id(item).map(|format| Offer {
                format,
                item: item.clone(),
            })
        })
        .collect();
    if offers.is_empty() {
        return Err(DragError::NoItems);
    }

    let data_object: IDataObject = SilkaDataObject { offers }.into();
    let drop_source: IDropSource = SilkaDropSource {}.into();

    let mut performed = DROPEFFECT_NONE;
    let result = unsafe {
        DoDragDrop(
            &data_object,
            &drop_source,
            allowed_mask(source.allowed()),
            &mut performed,
        )
    };

    // Both "success" codes are *positive* HRESULTs; treating them through a
    // generic is-error check would report a successful drop as a failure.
    let dropped = result == DRAGDROP_S_DROP;
    let cancelled = result == DRAGDROP_S_CANCEL;

    if let Some(f) = source.take_on_finish() {
        if dropped {
            f(effect_from_ole(performed));
        } else if cancelled {
            // A cancelled or nowhere drop: `None` tells a `Move` source to
            // keep its original.
            f(None);
        } else {
            // A genuine failure (OLE refused to start). Same silence as the
            // macOS path: `on_finish` is "how the drag ended", and a drag
            // that never started has not ended.
        }
    }
    Ok(())
}
