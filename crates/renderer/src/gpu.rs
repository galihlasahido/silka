//! Konteks GPU: instance, adapter, device, queue.

use std::sync::Arc;

use crate::error::RendererError;
use crate::geometry::SurfaceGeometry;
use crate::surface::WindowSurface;

/// Target window untuk pembuatan surface.
///
/// Sengaja dinyatakan lewat `raw-window-handle`, bukan lewat tipe winit:
/// `silka-renderer` tidak boleh tahu shell mana yang memakainya, dan
/// `silka-platform` tidak boleh tahu API grafis mana yang dipakai.
pub trait WindowTarget:
    raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle + Send + Sync + 'static
{
}

impl<T> WindowTarget for T where
    T: raw_window_handle::HasWindowHandle
        + raw_window_handle::HasDisplayHandle
        + Send
        + Sync
        + 'static
{
}

/// Konteks GPU bersama untuk seluruh aplikasi.
///
/// Satu `Gpu` melayani banyak window: surface tambahan memakai adapter dan
/// device yang sama sehingga resource (atlas glyph, pipeline SDF) bisa
/// dipakai bersama.
#[derive(Debug)]
pub struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Gpu {
    /// Bangun konteks GPU sekaligus surface pertamanya.
    ///
    /// Adapter dipilih **setelah** surface ada supaya wgpu bisa menjamin
    /// adapter itu benar-benar bisa mempresentasikan ke window tersebut
    /// (syarat mutlak di Vulkan/GL, dan hal yang benar di Metal).
    ///
    /// # Platform
    ///
    /// Di macOS pemanggilan ini harus terjadi di main thread — batasan Metal,
    /// bukan batasan kita. `silka-platform` memanggilnya dari `resumed()`
    /// event loop winit, jadi syarat itu otomatis terpenuhi.
    pub fn with_surface<W: WindowTarget>(
        target: Arc<W>,
        geometry: SurfaceGeometry,
    ) -> Result<(Self, WindowSurface), RendererError> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = preferred_backends();
        let instance = wgpu::Instance::new(descriptor);

        let surface = instance
            .create_surface(target)
            .map_err(|e| RendererError::SurfaceCreation(e.to_string()))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            // UI bukan game: adapter hemat daya lebih tepat, dan di Mac
            // dual-GPU inilah yang menghindari menyalakan GPU diskrit.
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .map_err(|e| RendererError::NoAdapter(e.to_string()))?;

        let gpu = Self::from_adapter(instance, adapter)?;
        let window_surface = WindowSurface::new(&gpu, surface, geometry)?;
        Ok((gpu, window_surface))
    }

    /// Konteks GPU **tanpa window** — untuk rendering headless.
    ///
    /// Inilah pintu masuk golden/snapshot test visual dan benchmark frame-time
    /// di CI (REKOMENDASI §9.5): scene yang sama yang digambar ke window bisa
    /// digambar ke [`crate::OffscreenTarget`] dan dibandingkan piksel demi
    /// piksel, tanpa server tampilan.
    pub fn headless() -> Result<Self, RendererError> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = preferred_backends();
        let instance = wgpu::Instance::new(descriptor);

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .map_err(|e| RendererError::NoAdapter(e.to_string()))?;

        Self::from_adapter(instance, adapter)
    }

    fn from_adapter(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
    ) -> Result<Self, RendererError> {
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("silka.device"),
            required_features: wgpu::Features::empty(),
            // Downlevel defaults menjaga jalur Linux/GL tetap terbuka
            // (REKOMENDASI §3.2), dinaikkan resolusinya mengikuti adapter agar
            // window besar/multi-monitor tidak menabrak limit tekstur.
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| RendererError::DeviceUnavailable(e.to_string()))?;

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// Surface tambahan untuk window kedua dan seterusnya.
    pub fn create_surface<W: WindowTarget>(
        &self,
        target: Arc<W>,
        geometry: SurfaceGeometry,
    ) -> Result<WindowSurface, RendererError> {
        let surface = self
            .instance
            .create_surface(target)
            .map_err(|e| RendererError::SurfaceCreation(e.to_string()))?;
        WindowSurface::new(self, surface, geometry)
    }

    /// Nama backend aktif, mis. `"Metal"` di macOS — berguna untuk log dan
    /// laporan bug pengguna.
    pub fn backend_name(&self) -> &'static str {
        match self.adapter.get_info().backend {
            wgpu::Backend::Metal => "Metal",
            wgpu::Backend::Vulkan => "Vulkan",
            wgpu::Backend::Dx12 => "D3D12",
            wgpu::Backend::Gl => "OpenGL",
            wgpu::Backend::BrowserWebGpu => "WebGPU",
            wgpu::Backend::Noop => "Noop",
        }
    }

    /// Nama adapter/GPU yang terpilih.
    pub fn adapter_name(&self) -> String {
        self.adapter.get_info().name
    }

    pub(crate) fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    pub(crate) fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Akses mentah ke `wgpu::Device`.
    ///
    /// **Hanya untuk crate backend saudara** (pipeline SDF, atlas glyph) —
    /// kode widget tidak boleh memanggil ini, dan tidak pernah punya
    /// referensi `Gpu` untuk melakukannya (REKOMENDASI §3.2).
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }
}

fn preferred_backends() -> wgpu::Backends {
    // PRIMARY = Metal / Vulkan / D3D12 sesuai §3.2; GL disertakan sebagai
    // jaring pengaman Linux lama, ditangani wgpu sendiri tanpa tier terpisah.
    wgpu::Backends::PRIMARY | wgpu::Backends::GL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_pilihan_mencakup_metal_dan_gl() {
        let b = preferred_backends();
        assert!(b.contains(wgpu::Backends::METAL));
        assert!(b.contains(wgpu::Backends::VULKAN));
        assert!(b.contains(wgpu::Backends::DX12));
        assert!(b.contains(wgpu::Backends::GL));
        // `Backends::PRIMARY` juga membawa BROWSER_WEBGPU; itu tidak apa-apa
        // karena backend web tidak pernah ikut dikompilasi di target desktop.
        assert_eq!(b, wgpu::Backends::PRIMARY | wgpu::Backends::GL);
    }
}
