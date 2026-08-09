//! Kesalahan yang bisa muncul dari backend.

use core::fmt;

/// Kesalahan backend renderer.
///
/// Tipe wgpu sengaja **tidak** ikut bocor keluar: varian menyimpan pesan yang
/// sudah diformat agar pemanggil (dan nanti backend GL/CPU) berbicara dalam
/// kosakata yang sama.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RendererError {
    /// Tidak ada adapter GPU yang cocok (mis. driver terlalu tua).
    NoAdapter(String),
    /// Perangkat GPU gagal dibuat atau limit yang diminta tidak tersedia.
    DeviceUnavailable(String),
    /// Surface gagal dibuat dari window handle.
    SurfaceCreation(String),
    /// Adapter yang terpilih tidak mendukung surface ini sama sekali.
    SurfaceUnsupported,
    /// Surface hilang dan harus dibuat ulang dari window (mis. GPU reset).
    SurfaceLost,
}

impl fmt::Display for RendererError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RendererError::NoAdapter(m) => write!(f, "tidak ada adapter GPU yang cocok: {m}"),
            RendererError::DeviceUnavailable(m) => write!(f, "perangkat GPU tidak tersedia: {m}"),
            RendererError::SurfaceCreation(m) => write!(f, "gagal membuat surface: {m}"),
            RendererError::SurfaceUnsupported => {
                write!(
                    f,
                    "adapter tidak mendukung format apa pun untuk surface ini"
                )
            }
            RendererError::SurfaceLost => write!(f, "surface hilang dan harus dibuat ulang"),
        }
    }
}

impl std::error::Error for RendererError {}
