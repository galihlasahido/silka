# silka-paint

The drawing vocabulary for [silka](../../README.md): quads, corner geometry,
shadows, glyph runs, clips, strokes, bitmaps, transforms, and layers — expressed
as plain data, with **no GPU type anywhere in the public API**.

This crate says *what* to draw. It never says *how*. Executing a
[`Scene`](https://docs.rs/silka-paint) is the job of a backend
(`silka-renderer` on wgpu today; a GL or CPU backend later), and adding such a
backend must not require touching a single widget.

## What is in the vocabulary

| Module | Contents |
| --- | --- |
| `color` | `Color` in non-linear sRGB with straight alpha, plus the sRGB ⇄ linear conversion the backend applies at its own boundary |
| `geometry` | `Point`, `Size`, `Rect`, `Insets` — always in **logical points**, never physical pixels |
| `corner` | `CornerStyle` (arc vs squircle), `CornerRadii`, `Corners` — corner shape as a *parameter*, including the hit-test predicate |
| `shadow` | `Shadow` and the HIG-style `ShadowPair` (ambient + key) |
| `glyph` | `GlyphRun`, `Glyph`, `GlyphImageId` — text as opaque atlas ids, never as fonts |
| `atlas` | `GlyphSource`, the trait a backend uses to turn those ids into texels |
| `stroke` | `Stroke`, `LineCap`, `LineJoin` — a real line: a polyline with a width, caps and joins |
| `image` | `ImageQuad`, `ImageId`, `ImageSource`, `ImageAtlas` — bitmaps through an atlas, photos and tintable icons alike |
| `svg` | `rasterize_path` — one filled SVG path into a coverage mask, on the CPU, once at load time |
| `transform` | `Transform` — an affine matrix for a whole subtree of commands |
| `layer` | `Layer`, `LayerEffect` — render a subtree to a texture, then composite it (group opacity, blur) |
| `scene` | `Scene`, `Command`, `Quad`, `ShadowQuad` — one frame as a command list |

## Example

```rust
use silka_paint::{Color, CornerStyle, Corners, Quad, Rect, Scene, Shadow, ShadowPair};

let mut scene = Scene::new(Color::hex(0x1C1C1E));

let card = Quad::new(Rect::new(24.0, 24.0, 180.0, 96.0))
    .background(Color::hex(0x2C2C2E))
    // The radius and the curve style both arrive from a theme token.
    .corners(Corners::uniform(14.0, CornerStyle::squircle()))
    .normalized();

// The HIG recipe: a soft ambient layer plus a tighter, offset key layer.
scene.push_shadowed(
    card,
    ShadowPair::new(
        Shadow::new(Color::BLACK.with_alpha(0.06), 16.0).offset(0.0, 2.0),
        Shadow::new(Color::BLACK.with_alpha(0.12), 4.0).offset(0.0, 1.0),
    ),
);

// Two shadow layers plus the box itself.
assert_eq!(scene.len(), 3);
```

A pressed control, a tick, and a blurred panel — the three commands that used to
be worked around:

```rust
use silka_paint::{
    Color, Layer, LineCap, LineJoin, Point, Quad, Rect, Scene, Stroke, Transform,
};

let mut scene = Scene::new(Color::hex(0x1C1C1E));
let button = Rect::new(0.0, 0.0, 120.0, 44.0);

// Scale-on-press applies to the WHOLE subtree, label included. At rest the
// transform is the identity and no command is emitted at all.
scene.with_transform(Transform::scale_around(button.center(), 0.96, 0.96), |s| {
    s.push(Quad::new(button).background(Color::hex(0x0A84FF)));
});

// A tick is one stroke, not a dozen stamped quads.
let mut tick = Stroke::new(Color::WHITE, 2.0)
    .cap(LineCap::Round)
    .join(LineJoin::Round);
tick.extend([
    Point::new(4.0, 9.0),
    Point::new(7.0, 12.0),
    Point::new(13.0, 5.0),
]);
scene.push(tick);

// A material: the panel is rendered to a texture, blurred, then composited.
let sidebar = Rect::new(0.0, 0.0, 260.0, 720.0);
scene.with_layer(Layer::new(sidebar).blur(24.0).opacity(0.92), |s| {
    s.push(Quad::new(sidebar).background(Color::WHITE.with_alpha(0.6)));
});
```

## Two rules this crate exists to enforce

1. **Corner geometry is a parameter, not a constant.** `rounded_lg()` is a
   G2-continuous squircle under the Cupertino preset and a circular arc under
   the Tailwind preset. The difference travels all the way to the shader as
   per-command data, and `Corners::contains` gives hit-testing the *same*
   shape — a corner that looks empty is not clickable.
2. **Text crosses this boundary without a font.** A `GlyphRun` carries atlas
   ids and destination rects. The backend redeems those ids through
   `GlyphSource`, so it never learns what `silka-text` is, and `silka-text`
   never learns what wgpu is.

## Dependencies

None. This crate is deliberately dependency-free so that every layer above it,
and every backend below it, can agree on these types without inheriting a
graphics stack.

## License

MIT OR Apache-2.0
