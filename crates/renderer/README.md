# silka-renderer

The **wgpu** backend for [silka](../../README.md) — the only crate in the
workspace allowed to name a wgpu type.

It takes a `silka_paint::Scene` and rasterizes the entire box vocabulary with
one UI-specific SDF shader, in the spirit of GPUI: rounded rects and squircles,
borders, ambient + key shadows, glyphs from an atlas, and clipping.

## The single-draw-call design

Everything that differs between commands is *instance data*, not a shader
variant:

| What differs | How it is expressed |
| --- | --- |
| Arc vs squircle | per-instance superellipse exponent (2 vs ≈4) |
| Per-corner radius | four `f32` per instance, already scaled CPU-side |
| Border | per-instance width; a ring between two SDF isolines |
| Ambient + key shadow | two gaussian-blurred instances behind the box |
| Glyph | textured instance: atlas UV plus the run color |

Because it is all data, **no WGSL is ever assembled at runtime** — the lesson
Impeller paid for. Anti-aliasing comes from screen-space derivatives, so it is
correct on 2× Retina and on fractional Wayland scales without an extra
parameter.

`Command::PushClip`/`PopClip` become GPU scissor rects: the scene is split into
`(clip rect, instance range)` batches in command order, so a UI without
clipping stays a single draw call and one scroll view adds two.

## Example

```rust,no_run
use std::sync::Arc;
use silka_paint::{Color, Scene, Size};
use silka_renderer::{Gpu, SurfaceGeometry, WindowTarget};

fn draw_once<W: WindowTarget>(window: Arc<W>) -> Result<(), Box<dyn std::error::Error>> {
    let geometry = SurfaceGeometry::from_logical(Size::new(1024.0, 720.0), 2.0);
    let (gpu, mut surface) = Gpu::with_surface(window, geometry)?;

    // The background color always comes from a theme token, never a literal.
    let scene = Scene::new(Color::hex(0x1C1C1E));
    surface.render(&gpu, &scene)?;
    Ok(())
}
```

Headless rendering — the foundation of the golden-image tests — is the same
path without a window:

```rust,no_run
use silka_paint::{Color, Scene, Size};
use silka_renderer::{Gpu, OffscreenTarget, SurfaceGeometry};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let gpu = Gpu::headless()?;
let geometry = SurfaceGeometry::from_logical(Size::new(320.0, 200.0), 2.0);
let mut target = OffscreenTarget::new(&gpu, geometry)?;

let image = target.render(&gpu, &Scene::new(Color::hex(0x1C1C1E)))?;
assert_eq!(image.width(), 640); // 320 logical points at 2x
# Ok(()) }
```

## The boundaries it keeps

- Its public surface mentions only `silka-paint` and `raw-window-handle` types.
  It does **not** know what winit is, and its callers do **not** need to know
  what wgpu is.
- The single door into the wgpu world is `Gpu::device`, reserved for sibling
  backend crates.
- Glyphs arrive through `silka_paint::GlyphSource`, so the renderer never names
  `silka-text`.

## License

MIT OR Apache-2.0
