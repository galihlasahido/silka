# silka

A desktop GUI framework for Rust with a declarative widget model and
Apple-grade visual polish. Targets **macOS, Windows, and Linux** (X11/Wayland).

> Status: under active development. The API is still changing.

## Philosophy

Silka is built on three convictions:

1. **Writing UI should feel good.** The API is nested composition with method
   chaining — close to the feel of writing Flutter widgets, while staying
   idiomatic Rust and checked at compile time.
2. **Motion is part of the design.** Every animation is a spring that carries
   both position and velocity, so it can be retargeted mid-flight without
   snapping — not a rigid easing curve.
3. **Small details decide how it feels.** Squircle corners, layered shadows,
   and optical-size typography are not decoration; they are the reason an
   interface feels smooth.

## Example

```rust
let count = use_signal(|| 0);

column((
    text(format!("Count: {}", count.get())),
    button("Increment").on_press(move || count.set(count.get() + 1)),
))
.spacing(12.0)
.padding(16.0)
```

Styling follows a utility-first pattern like Tailwind, but as compiler-checked
methods — a typo is a build error, not a silent no-op.

## Theming

Every component is written once against semantic tokens, then renders according
to the active preset:

| Preset | Character |
| --- | --- |
| **Cupertino** (default) | Squircle corners, Apple HIG palette, layered ambient + key shadows |
| **Tailwind** | Plain arc corners, slate/blue palette, Tailwind-style shadows |

Both support light and dark mode, and both use the same spring animations —
smooth motion is the framework's identity, not a property of one theme.

## Architecture

| Crate | Responsibility |
| --- | --- |
| `paint` | Drawing command vocabulary (quads, shadows, glyphs, clips), free of GPU types |
| `renderer` | wgpu backend with SDF shaders; the whole scene renders in a single draw call |
| `text` | Text shaping, glyph atlas, and measurement for layout |
| `core` | Signals, arena-backed render tree, constraint layout, animation, input, accessibility |
| `theme` | Semantic tokens and theme presets |
| `widgets` | The component library |
| `platform` | Window shell, application lifecycle, and OS integration |

Widget code never touches GPU types directly. Everything goes through the
`paint` layer, so the rendering backend can be swapped without changing a
single component.

## Accessibility

Every component emits an accessibility node as part of its contract, not as a
later addition. Keyboard navigation, focus rings, and honoring the system
*reduce motion* setting are requirements for a component to be considered
done — not optional extras.

## Running the gallery

```bash
cargo run -p silka-gallery
```

The gallery showcases the available components and their variants, with a theme
switcher and dark mode toggle.

## License

MIT
