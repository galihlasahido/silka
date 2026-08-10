# silka-paint

The drawing vocabulary for [silka](../../README.md): quads, corner geometry,
shadows, glyph runs, and clips — expressed as plain data, with **no GPU type
anywhere in the public API**.

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
