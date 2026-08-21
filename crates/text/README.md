# silka-text

The text layer of [silka](../../README.md): shaping, a glyph atlas, measurement
for layout, and the editing model behind `text_field` / `text_area`.

It is a thin wrapper over [cosmic-text](https://github.com/pop-os/cosmic-text)
(fontdb + rustybuzz + swash). The binding rule behind that choice: **never
write your own shaper.** Text is the single most common cause of death for a
new GUI framework, and the parts that have to be right on day one — font
fallback per platform, bidi (UAX #9), ZWJ and color emoji, subpixel
*positioning*, per-grapheme carets, inline IME preedit — are exactly the parts
a hand-rolled shaper gets wrong.

## The boundary it keeps

cosmic-text types **never appear in this crate's public API**. Callers speak in
`TextStyle`, `TextConstraints`, `TextMeasure`, `TextLayout`, and
`silka_paint::GlyphRun`. Moving to `parley` later would therefore be work
confined to this crate, and widget code still has no idea what a font is.

## The three things it does

```rust
use silka_paint::{Color, Point, Scene};
use silka_text::{TextConstraints, TextEngine, TextStyle};

// One engine per application — one atlas, one measurement cache.
// `bundled_only` skips system fonts so tests stay deterministic.
let mut engine = TextEngine::bundled_only();
engine.set_scale_factor(2.0); // Retina

// Styles come from theme tokens, never from literal numbers in widget code.
let style = TextStyle::new().size(17.0);

// 1. Measure — this is what the box-constraints layout pass calls.
let measure = engine.measure("Hello, world", &style, TextConstraints::width(280.0));
assert!(measure.width() > 0.0);
assert_eq!(measure.line_count, 1);

// 2. Lay out — positions, line metrics, caret and hit geometry.
let layout = engine.layout("Hello, world", &style, TextConstraints::width(280.0));
assert_eq!(layout.line_count(), 1);

// 3. Draw — the result is a GlyphRun of atlas ids, not of fonts.
let mut scene = Scene::new(Color::hex(0x1C1C1E));
engine.draw(
    &mut scene,
    "Hello, world",
    &style,
    TextConstraints::width(280.0),
    Point::new(24.0, 24.0),
    Color::WHITE,
);
assert_eq!(scene.len(), 1);
```

## Measuring while a window is resized

A resize hands down a different `max_width` on every frame, but hardly any text
actually *wraps*: a file name, a button title or a column header measures the
same in a 900 pt column as in a 1200 pt one. So the measure cache keys that text
**without the width in the key**, and only text the width really broke stays
pinned to the width that broke it.

```rust
use silka_text::{TextConstraints, TextEngine, TextStyle};

let mut engine = TextEngine::bundled_only();
let style = TextStyle::new().size(13.0);

let before = engine.shape_count();
for w in 300..900 {
    engine.measure("annual-report.pdf", &style, TextConstraints::width(w as f32));
}
// Six hundred widths, one shaping.
assert_eq!(engine.shape_count() - before, 1);
```

Widgets that keep their own shaping result ask
`TextLayout::minimum_valid_width` the same question, and a full cache evicts its
least recently used entry instead of emptying itself — throwing everything away
in the middle of a gesture is the one thing a resize cannot afford.

## What the backend sees

A `GlyphRun` carries **atlas ids plus logical destination rects**. The backend
redeems those ids through `silka_paint::GlyphSource`, which `TextEngine`
implements — and that trait is the entire surface between the two:

```rust
use silka_paint::{GlyphFormat, GlyphSource};
use silka_text::{TextConstraints, TextEngine, TextStyle};

let mut engine = TextEngine::bundled_only();
# let style = TextStyle::new();
# let layout = engine.layout("Hi", &style, TextConstraints::UNBOUNDED);
# let run = engine.rasterize(&layout, silka_paint::Point::ZERO, silka_paint::Color::WHITE);

// What a backend does every frame — without ever saying "font":
let side = engine.atlas_size(GlyphFormat::Mask);
if let Some(dirty) = engine.take_dirty(GlyphFormat::Mask) {
    let _pixels = engine.atlas_pixels(GlyphFormat::Mask); // upload just this rect
    let _uv = dirty.uv(side);
}
let _placement = engine.placement(run.glyphs[0].image); // id → rect in the atlas
```

Only the atlas rects that actually changed are uploaded, so a frame whose text
did not change costs zero bytes.

## Editing

`edit` holds a pure editing model — no pixels: a document, a `Selection` that
moves per grapheme cluster and per word (UAX #29), undo/redo that coalesces a
run of typing, and `Preedit` for IME composition. `layout` supplies the
geometry that model needs (`TextLayout::hit`, `caret`, `selection_rects`).
`silka-widgets` builds both `text_field` and `text_area` on this one model.

## Known gaps

- Inter's `opsz` (optical size) axis is not yet driven automatically by font
  size; the `wght` axis through variable weight does work.
- Rich text within one paragraph and automatic ellipsis are not there yet;
  `max_lines` plus an `overflowed` flag are the foundation that is.
- A selection spanning lines does not yet highlight the line break itself.

## License

MIT OR Apache-2.0
