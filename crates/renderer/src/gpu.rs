//! The GPU context: instance, adapter, device, queue.

use std::sync::Arc;

use crate::error::RendererError;
use crate::geometry::SurfaceGeometry;
use crate::surface::WindowSurface;

/// A window target for surface creation.
///
/// Deliberately expressed through `raw-window-handle` rather than a winit type:
/// `silka-renderer` must not know which shell is driving it, and
/// `silka-platform` must not know which graphics API is in use.
///
/// Nothing implements this trait by hand: a blanket impl covers every type that
/// already satisfies the `raw-window-handle` bounds, so the trait is really a
/// name for "something a surface can be created from".
///
/// ```no_run
/// use std::sync::Arc;
/// use silka_paint::Size;
/// use silka_renderer::{Gpu, SurfaceGeometry, WindowTarget};
///
/// fn attach<W: WindowTarget>(window: Arc<W>) -> Result<(), Box<dyn std::error::Error>> {
///     let geometry = SurfaceGeometry::from_logical(Size::new(800.0, 600.0), 1.0);
///     let (_gpu, _surface) = Gpu::with_surface(window, geometry)?;
///     Ok(())
/// }
/// ```
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

/// The GPU context shared by the whole application.
///
/// A single `Gpu` serves many windows: additional surfaces use the same adapter
/// and device, so resources (glyph atlas, SDF pipeline) can be shared.
///
/// ```no_run
/// use silka_paint::{Color, Scene, Size};
/// use silka_renderer::{Gpu, OffscreenTarget, SurfaceGeometry};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Without a window: the entry point for golden tests and benchmarks.
/// let gpu = Gpu::headless()?;
/// println!("{} on {}", gpu.backend_name(), gpu.adapter_name());
///
/// let geometry = SurfaceGeometry::from_logical(Size::new(320.0, 200.0), 2.0);
/// let mut target = OffscreenTarget::new(&gpu, geometry)?;
/// let image = target.render(&gpu, &Scene::new(Color::hex(0x1C1C1E)))?;
/// assert_eq!(image.width(), 640);
/// # Ok(()) }
/// ```
///
/// With a window, the adapter is chosen after the surface exists, so it is
/// guaranteed to be able to present to it — see [`Gpu::with_surface`].
#[derive(Debug)]
pub struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Gpu {
    /// Build the GPU context together with its first surface.
    ///
    /// The adapter is picked **after** the surface exists so wgpu can guarantee
    /// that adapter really can present to that window (mandatory on Vulkan/GL,
    /// and the right thing to do on Metal).
    ///
    /// # Platform
    ///
    /// On macOS this call must happen on the main thread — a Metal
    /// restriction, not ours. `silka-platform` calls it from winit's
    /// `resumed()` event loop, so the requirement is met automatically.
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
            // A UI is not a game: the low-power adapter is the right fit, and
            // on dual-GPU Macs this is what avoids spinning up the discrete
            // GPU.
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .map_err(|e| RendererError::NoAdapter(e.to_string()))?;

        let gpu = Self::from_adapter(instance, adapter)?;
        let window_surface = WindowSurface::new(&gpu, surface, geometry)?;
        Ok((gpu, window_surface))
    }

    /// A GPU context **without a window** — for headless rendering.
    ///
    /// This is the entry point for visual golden/snapshot tests and frame-time
    /// benchmarks in CI (REKOMENDASI §9.5): the same scene that is drawn into a
    /// window can be drawn into a [`crate::OffscreenTarget`] and compared pixel
    /// by pixel, with no display server.
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
            // Downlevel defaults keep the Linux/GL path open (REKOMENDASI
            // §3.2), with the resolution raised to follow the adapter so large
            // or multi-monitor windows do not hit the texture limit.
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

    /// An additional surface, for the second window and beyond.
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

    /// The active backend's name, e.g. `"Metal"` on macOS — useful for logs and
    /// user bug reports.
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

    /// The name of the selected adapter/GPU.
    pub fn adapter_name(&self) -> String {
        self.adapter.get_info().name
    }

    pub(crate) fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    pub(crate) fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Raw access to the `wgpu::Device`.
    ///
    /// **For sibling backend crates only** (SDF pipeline, glyph atlas) — widget
    /// code must not call this, and never holds a `Gpu` reference with which it
    /// could (REKOMENDASI §3.2).
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }
}

fn preferred_backends() -> wgpu::Backends {
    // PRIMARY = Metal / Vulkan / D3D12 per §3.2; GL is included as a safety net
    // for older Linux, handled by wgpu itself without a separate tier.
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
        // `Backends::PRIMARY` also brings in BROWSER_WEBGPU; that is fine,
        // since the web backend is never compiled into a desktop target.
        assert_eq!(b, wgpu::Backends::PRIMARY | wgpu::Backends::GL);
    }
}
