# Icon attribution

The built-in symbol set in [`silka_widgets::IconName`](src/icon.rs) is
**Material Symbols**, used under the Apache License 2.0.

| | |
| --- | --- |
| Upstream | [`google/material-design-icons`](https://github.com/google/material-design-icons) |
| Commit | `e083cc60a0828fdd3b404cea0cb8a5b900e9c23e` (2026-08-14) |
| Variant | Material Symbols **Rounded**, **filled** (`fill1`), **24dp** |
| Licence | [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0) — full text at [`LICENSE-APACHE`](../../LICENSE-APACHE) in the repository root |

Apache-2.0 is not the licence of silka itself (MIT OR Apache-2.0). It applies to
the path data listed below, and its conditions travel with any redistribution of
this crate.

## Statement of modification (Apache-2.0 §4(b))

**The path data is unmodified.** Each `d` attribute is byte-for-byte what the
upstream SVG contains, taken from
`symbols/web/<symbol>/materialsymbolsrounded/<symbol>_fill1_24px.svg`.

That is a deliberate choice rather than a convenience. Rewriting coordinates into
silka's own `0 0 24 24` grid would mean touching every number in every path, and
a single mistyped decimal produces artwork that is subtly wrong in a way no test
notices. Instead the grid travels with the path:
[`silka_paint::ViewBox`](../paint/src/svg.rs) carries the upstream
`0 -960 960 960` — a thousand units square with **negative Y** — and
`IconName::view_box()` reports it per symbol.

Two things were changed, neither of them the artwork:

1. **Names.** silka's variant names follow its own vocabulary, so they do not
   always match the upstream symbol name. The mapping is the table below.
2. **Packaging.** The `d` attribute is extracted from the SVG wrapper and stored
   as a Rust string constant, wrapped across source lines at path-command
   boundaries. Line continuations never split a number or a command.

No symbol required conversion: not one of the 23 paths uses an elliptical arc
(`A`/`a`), which the rasteriser refuses. They are upstream exactly as published.

## The symbols used

| `IconName` | Upstream symbol | Meaning in silka |
| --- | --- | --- |
| `Check` | `check` | confirmation, a selected row |
| `ChevronUp` | `keyboard_arrow_up` | a collapsed disclosure, a sort direction |
| `ChevronDown` | `keyboard_arrow_down` | a dropdown, an expandable row |
| `ChevronLeft` | `chevron_left` | physical left (see below) |
| `ChevronRight` | `chevron_right` | physical right (see below) |
| `Close` | `close` | close, clear, remove |
| `Plus` | `add` | add |
| `Minus` | `remove` | remove, collapse |
| `Search` | `search` | search |
| `Menu` | `menu` | a menu affordance |
| `Ellipsis` | `more_horiz` | an overflow menu |
| `Info` | `info` | informational status |
| `Warning` | `warning` | a warning |
| `Trash` | `delete` | destructive action |
| `Star` | `star` | a favourite, a rating |
| `Heart` | `favorite` | a like |
| `User` | `person` | a person, an account |
| `Calendar` | `calendar_month` | a date |
| `Download` | `download` | download |
| `Upload` | `upload` | upload |
| `Sun` | `light_mode` | light appearance |
| `Moon` | `dark_mode` | dark appearance |
| `Bell` | `notifications` | notifications |

Four names diverge from upstream on purpose. `ChevronUp`/`ChevronDown` come from
`keyboard_arrow_*` because the `expand_less`/`expand_more` pair reads as a
disclosure control rather than a direction, and silka uses these for both. `Plus`
and `Minus` keep the names a developer reaches for, not `add`/`remove`.

`ChevronLeft` and `ChevronRight` are **physical** directions. An arrow meaning
"the previous page" is not physical — it mirrors in a right-to-left document —
so that intent is spelled `chevron_back()` / `chevron_forward()` and the flip
happens inside the node. See the `icon` module docs (§9.8).

## Adding another symbol from the same set

Fetch the filled Rounded 24dp SVG, take the `d` attribute unchanged, and pass the
grid along with it:

```rust
use silka_widgets::{icon_path_in_box, MATERIAL_SYMBOLS_VIEW_BOX};

let saved = icon_path_in_box(
    "material/bookmark",
    "m480-240-168 72q-40 17-76-6.5T200-241v-519q0-33 23.5-56.5T280-840h400q33 0 \
     56.5 23.5T760-760v519q0 43-36 66.5t-76 6.5l-168-72Z",
    MATERIAL_SYMBOLS_VIEW_BOX,
);
```

Passing that same path to `icon_path()` — which assumes a `0 0 24 24` grid —
parses without complaint and draws **nothing at all**, because every Y
coordinate is negative and lands above the canvas. That failure is silent, which
is why the grid is a parameter and why
`icon::tests::built_in_paths_are_blank_without_their_view_box` exists to hold the
line.

When you add a symbol here, add its row to the table above. An icon whose
attribution is missing is a licence violation in a public repository, not an
untidy document.
