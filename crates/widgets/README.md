# silka-widgets

The component catalogue of [silka](../../README.md) — and at the same time the
framework's **public API surface**. This is the contract that has to be frozen
early; everything below it may change at will.

## Two rules for the shape of the API

1. **Dart style** — constructor functions plus method chaining, nesting like
   Flutter widgets; optional properties move into the chain. An `rsx!`-style
   DSL macro is deliberately rejected as the foundation.
2. **Tailwind-style utility styling as a method chain** — no CSS, no parser, no
   cascade. Values always resolve through `silka-theme` tokens, and the
   interactive utilities (`hover` / `pressed` / `focused`) **transition on a
   spring** rather than jumping the way CSS without `transition` does.

```rust
use silka_core::signals::Runtime;
use silka_core::view::{column, View};
use silka_theme::{Appearance, Theme};
use silka_widgets::{button_in, text_in, Fonts};

# let rt = Runtime::new();
# let count = rt.signal(0i32);
# let fonts = Fonts::bundled_only();
# let t = Theme::cupertino(Appearance::Dark);
column([
    View::from(text_in(&fonts, format!("Count: {}", count.get())).color(t.color.label)),
    View::from(button_in(&fonts, &t, "Increment").on_press(move || count.set(count.get() + 1))),
])
.spacing(t.space(3.0));
```

## The catalogue

| Tier | Components |
| --- | --- |
| 0 — primitives | `text` |
| 1 — layout | `scroll_view` (rubber banding, overlay scrollbars), `list` (virtualized) |
| 2 — controls | `button`, `icon_button`, `checkbox` (tri-state), `radio` / `radio_group`, `switch` / `toggle`, `slider` / `range_slider`, `stepper`, `select`, `combo_box`, `text_field`, `text_area`, `label` / `field` / `form` |
| 3 — navigation | `tabs` (segmented / underline / enclosed), `menu` and `context_menu` |
| 4 — overlay | `overlay` (infrastructure), `dialog` / `alert` |
| 5 — data | `table` (virtualized, sortable, resizable), `tree` (virtualized outline) |
| 6 — editors | `wysiwyg` |

Plus the shared seams: `Fonts` (one text engine and one atlas per application),
`editing` (the half of the text keymap that means the same thing in a one-line
field and a multi-line editor, written once), and `advance` (one tick that
drives every widget's springs and answers "is anything still moving?").

## Nothing is built twice

The catalogue's ordering rule is that a new component rides existing
infrastructure instead of growing a parallel one:

- `list` lives **inside** `scroll_view` — momentum, rubber banding, and
  scrollbars are not written twice.
- `table` and `tree` ride `list`'s virtualization and the same `ListMetrics`.
- `text_area` uses the very same `silka_text::TextEdit` document, graphemes,
  undo, and IME as `text_field`, in multiline mode.
- `wysiwyg` is built **on** `text_area`'s machinery — the frame, focus ring,
  auto-grow, and scroll view are literally the same nodes.
- `dialog`, `select`'s popup, `menu`, and the chart tooltip all ride `overlay`.
  Each one picks a `Placement` and a `Barrier`; not one of them computes its
  own position.
- `icon_button` is `button`'s own render node with an `icon` inside it, not a
  second interaction contract.
- `combo_box` is `text_field` plus `menu` — including the menu's *state*, so a
  suggestion list's rules about the highlight and the closing are the rules
  already tested for every menu in the application. What it adds is one node
  that takes the four keys the field lets through.

## Definition of done

A component is not finished until all of it is true:

- correct under **both presets**, light and dark;
- every interactive state transitions on a **spring**;
- full **keyboard** navigation plus a visible focus ring;
- an **AccessKit node** with role, name, and actions;
- a **≥ 44pt hit target**, even when the drawn control is 16pt;
- **reduced motion** honored.

## Known debt, not hidden

`Fonts` is still passed explicitly to every constructor because there is no
ambient context for application-level dependencies yet. "Scale on press" is
drawn as the background box deflating — the paint layer has no transform
command — so the label inside does not shrink with it. Overlays have no
"just opened" hook, so a freshly opened panel is not focused automatically.

Code in this crate must not touch wgpu types; only `silka-paint` drawing
commands.

## License

MIT OR Apache-2.0
