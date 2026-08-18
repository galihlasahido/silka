# silka-chart

Charts for [silka](../../README.md) — **a separate crate, not a separate set of
rules.**

It lives outside `silka-widgets` because a chart library is large and the widget
catalogue is the framework's frozen public contract. Charts can therefore grow
without every application paying for them. What the split explicitly does *not*
buy is a private set of conventions: every contract that binds a widget binds a
chart.

| Contract | Where it is honored here |
| --- | --- |
| Semantic tokens, dual preset, no hard-coded colors or sizes | `style::ChartStyle` — every value resolves through the theme |
| The paint abstraction; wgpu is never named | a series is one `silka-paint` `Stroke`; `stroke` fills areas with boxes |
| Spring animation, retargetable, reduced-motion aware | data transitions in `node::ChartBox::advance` |
| An AccessKit node as part of the widget contract | `node::ChartBox::summary` — a description, not a bare "image" |
| The overlay system is built once and ridden by all | `tooltip()` returns an `OverlayBuilder`; it computes no positions |
| Dart-style API: constructor plus method chain | `line_chart`, `bar_chart`, `area_chart`, `sparkline` |

## Example

```rust
use silka_chart::{bar_chart_in, format::{Locale, NumberFormat}};
use silka_theme::{Appearance, Theme};
use silka_widgets::Fonts;

struct Month { name: &'static str, income: f64, outgoing: f64 }

# let fonts = Fonts::bundled_only();
# let theme = Theme::cupertino(Appearance::Dark);
# let data = vec![Month { name: "Jan", income: 1.2e6, outgoing: 8.0e5 }];
bar_chart_in(&fonts, &theme, data)
    .x_label(|d: &Month| d.name.to_string())
    .y_named("Income", |d: &Month| d.income)
    .y_named("Outgoing", |d: &Month| d.outgoing)
    .stacked()
    .legend(true)
    .animated(true)
    .locale(Locale::ID_ID)
    .value_format(NumberFormat::Compact);
```

## What is in the box (v1)

Four marks — `line_chart`, `area_chart`, `bar_chart` (vertical or horizontal,
grouped or stacked), and `sparkline` — over one shared set of elements: axes
with ticks and labels, gridlines, a legend, a hover tooltip on the overlay
system, locale-aware number and date formatting, an empty state, and spring
transitions when the dataset changes.

## The two decisions worth arguing about

**Series colors do not come from the theme.** Every *other* color in a chart
does, but a series color encodes identity rather than role, and a role palette
has only one accent. So `palette` carries a categorical palette validated for
colorblind readers **by arithmetic in its own unit tests** (OKLab plus a
Machado 2009 protan/deutan simulation) — and it is the same under both presets,
because CVD safety is a promise to the reader, not a brand decision.

**A chart is one render node.** Axis space is circular: the value axis's width
depends on labels that depend on ticks that depend on the plot height that
depends on the category axis. Box constraints rightly forbid a node from
reading its sibling's measurements, so the circularity is resolved inside a
single node in two passes.

## Not in v1, deliberately

Pie and donut charts (a form worse than a bar chart at the job it is usually
given), scatter and bubble plots (they need the *all-pairs* palette gate, which
caps the series count at three), zoom and pan, and annotations. Each is an
addition to this crate rather than a change to it.

## Acknowledged debt

A line is a real `silka-paint` stroke — one command per series, rasterized from
a distance field — and what remains in `stroke` is the area *fill*, which is not
a stroke at all. The accessibility
role is `Image` with a description, because the role vocabulary in `silka-core`
has no chart role — adding one touches the platform adapter too, so it is a
change to make deliberately rather than in passing.

## License

MIT OR Apache-2.0
