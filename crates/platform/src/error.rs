//! Platform-layer errors.

use core::fmt;

use silka_renderer::RendererError;

/// Something went wrong opening a window or running the event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlatformError {
    /// The event loop could not be created or run (e.g. no display server).
    EventLoop(String),
    /// The OS refused to create the window.
    WindowCreation(String),
    /// The renderer backend failed.
    Renderer(RendererError),
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlatformError::EventLoop(m) => write!(f, "event loop gagal: {m}"),
            PlatformError::WindowCreation(m) => write!(f, "window gagal dibuat: {m}"),
            PlatformError::Renderer(e) => write!(f, "renderer gagal: {e}"),
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
