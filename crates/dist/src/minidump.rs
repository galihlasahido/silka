//! The one place `minidump-writer` is named.
//!
//! Third-party types stop at this file, the same boundary `silka-platform`
//! draws around `keyring` and `arboard`. The rest of the crate sees a function
//! that takes an open file and says whether it was filled.
//!
//! Compiled only with the `minidump` feature on macOS and Windows. Everywhere
//! else [`write`] does not exist and `crash::write_minidump` reports
//! `Unsupported` with the reason.

use std::fs::File;

/// Whether a backend exists in this build.
pub(crate) const AVAILABLE: bool = cfg!(all(
    feature = "minidump",
    any(target_os = "macos", target_os = "windows")
));

/// Fill `file` with a minidump of the **current** process.
///
/// The error is the backend's own message; the caller decides how to wrap it.
#[cfg(all(feature = "minidump", target_os = "macos"))]
pub(crate) fn write(file: &mut File) -> Result<(), String> {
    use minidump_writer::minidump_writer::MinidumpWriter;

    // `None, None` is "this task, this thread": the thread is excluded from
    // the thread list as the handler, which is right for a dump taken from a
    // panic hook and merely harmless for one taken on demand.
    MinidumpWriter::new(None, None)
        .dump(file)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Fill `file` with a minidump of the **current** process.
///
/// The error is the backend's own message; the caller decides how to wrap it.
#[cfg(all(feature = "minidump", target_os = "windows"))]
pub(crate) fn write(file: &mut File) -> Result<(), String> {
    use minidump_writer::minidump_writer::MinidumpWriter;

    MinidumpWriter::dump_local_context(None, None, None, file).map_err(|error| error.to_string())
}

/// No backend in this build; unreachable because [`AVAILABLE`] is false.
#[cfg(not(all(feature = "minidump", any(target_os = "macos", target_os = "windows"))))]
pub(crate) fn write(_file: &mut File) -> Result<(), String> {
    Err(String::from("no minidump backend in this build"))
}
