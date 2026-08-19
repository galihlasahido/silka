//! Errors the backend can surface.

use core::fmt;

/// A renderer backend error.
///
/// wgpu types deliberately do **not** leak out: each variant carries an
/// already-formatted message so callers (and later the GL/CPU backends) all
/// speak the same vocabulary.
///
/// ```
/// use silka_renderer::{Gpu, RendererError};
///
/// // A machine with no usable adapter is a normal condition in CI, not a
/// // panic: tests skip, and applications can fall back or explain themselves.
/// match Gpu::headless() {
///     Ok(_gpu) => {}
///     Err(RendererError::NoAdapter(reason)) => println!("no GPU: {reason}"),
///     Err(other) => println!("renderer unavailable: {other}"),
/// }
/// ```
///
/// [`RendererError::SurfaceLost`] is the one variant worth handling
/// specifically: it means the surface has to be recreated from the window
/// handle, typically after a GPU reset or a monitor change.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RendererError {
    /// No suitable GPU adapter (e.g. the driver is too old).
    NoAdapter(String),
    /// The GPU device failed to be created, or a requested limit is missing.
    DeviceUnavailable(String),
    /// The surface could not be created from the window handle.
    SurfaceCreation(String),
    /// The chosen adapter does not support this surface at all.
    SurfaceUnsupported,
    /// The surface was lost and must be recreated from the window (e.g. after
    /// a GPU reset).
    SurfaceLost,
}

impl fmt::Display for RendererError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RendererError::NoAdapter(m) => write!(f, "no suitable GPU adapter: {m}"),
            RendererError::DeviceUnavailable(m) => write!(f, "GPU device unavailable: {m}"),
            RendererError::SurfaceCreation(m) => write!(f, "could not create the surface: {m}"),
            RendererError::SurfaceUnsupported => {
                write!(f, "the adapter supports no format for this surface")
            }
            RendererError::SurfaceLost => write!(f, "the surface was lost and must be recreated"),
        }
    }
}

impl std::error::Error for RendererError {}
