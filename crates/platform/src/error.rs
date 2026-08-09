//! Kesalahan lapisan platform.

use core::fmt;

use silka_renderer::RendererError;

/// Kesalahan saat membuka window atau menjalankan event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlatformError {
    /// Event loop gagal dibuat atau dijalankan (mis. tanpa display server).
    EventLoop(String),
    /// Window gagal dibuat oleh OS.
    WindowCreation(String),
    /// Backend renderer gagal.
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
