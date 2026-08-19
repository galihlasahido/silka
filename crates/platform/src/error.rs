//! Platform-layer errors.

use core::fmt;

use silka_renderer::RendererError;

/// Something went wrong opening a window or running the event loop.
///
/// ```no_run
/// use silka_platform::{window, PlatformError};
///
/// match window("Editor").run() {
///     Ok(()) => {}
///     // A CI runner with no display server is the common case here, and it
///     // deserves a message rather than a panic.
///     Err(PlatformError::EventLoop(reason)) => eprintln!("no display: {reason}"),
///     Err(e) => eprintln!("could not start: {e}"),
/// }
/// ```
///
/// Note what is *not* here: a failure to **read** session state. A missing or
/// unreadable state file is a first run, and an application that refused to
/// start because it could not remember its window position would be a worse
/// bug than the one being reported.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlatformError {
    /// The event loop could not be created or run (e.g. no display server).
    EventLoop(String),
    /// The OS refused to create the window.
    WindowCreation(String),
    /// The renderer backend failed.
    Renderer(RendererError),
    /// Session state could not be written (INTEGRASI-NATIVE §6).
    ///
    /// Deliberately **not** produced when *reading* fails: a missing or
    /// unreadable state file is a first run, and an application that refuses
    /// to start because it cannot remember its window position would be a
    /// worse bug than the one being reported.
    State(String),
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlatformError::EventLoop(m) => write!(f, "event loop failed: {m}"),
            PlatformError::WindowCreation(m) => write!(f, "could not create the window: {m}"),
            PlatformError::Renderer(e) => write!(f, "renderer failed: {e}"),
            PlatformError::State(m) => write!(f, "could not save the session state: {m}"),
        }
    }
}

impl std::error::Error for PlatformError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PlatformError::Renderer(e) => Some(e),
            _ => None,
        }
    }
}

impl From<RendererError> for PlatformError {
    fn from(e: RendererError) -> Self {
        PlatformError::Renderer(e)
    }
}
