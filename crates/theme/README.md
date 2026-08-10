# silka-theme

Semantic design tokens and the two first-party presets of
[silka](../../README.md): **Cupertino** (Apple HIG, the default) and
**Tailwind/shadcn**.

A widget is written once against roles — `ColorToken::Surface`,
`RadiusToken::Md`, `FontToken::Body`, `SpaceToken::S4` — and is therefore
automatically correct under both presets, in light and in dark mode. There is
no CSS, no cascade, and no parser: a token is a value with no meaning until it
meets a `Theme`.

## Layers

| Layer | Module | Contents |
| --- | --- | --- |
| Raw palette | `palette` | Tailwind 50–950 ramps and HIG system colors. The **only** place color literals live. |
| Semantic tokens | `color`, `radius`, `shadow`, `spacing`, `typography` | Roles: `surface`, `accent`, `radius_md`, `shadow_md`, the 4pt scale, the type scale. |
| Resolution | `token` | The `Token` trait — one `Theme::resolve` for every kind of token. |
| Presets | `preset` | The only place tokens meet numbers. |
| OS settings | `system` | `Theme::with_accent`, `Transparency`, contrast-ratio arithmetic. |

## Example

```rust
use silka_theme::{Appearance, ColorToken, FontToken, Preset, RadiusToken, SpaceToken, Theme};

let theme = Theme::cupertino(Appearance::Dark);
assert_eq!(theme.preset, Preset::Cupertino);

// Widgets name roles, not numbers…
let background = theme.resolve(ColorToken::Surface);
let corners = theme.resolve(RadiusToken::Md);
let padding = theme.resolve(SpaceToken::S4);
let title = theme.resolve(FontToken::Title2);
# let _ = (background, corners, padding, title);

// …and the preset decides what comes out. Under Cupertino a corner is a
// G2-continuous squircle; under Tailwind it is a plain circular arc.
let same_widget_on_the_web = theme.with_preset(Preset::Tailwind);
assert_ne!(corners.style, same_widget_on_the_web.resolve(RadiusToken::Md).style);
```

## Corner geometry is a parameter, not a constant

`RadiusToken::Md` resolves to `silka_paint::Corners`, which carries **both** the
radius and the curve style. That value flows through the paint commands all the
way into the SDF shader *and* into hit-testing, so the shape that is drawn and
the shape that is clickable can never disagree.

## Reacting to the OS

A `Theme` is a pure value. When the OS changes — dark mode, accent color,
*reduce transparency* — the theme is rebuilt from `(Preset, Appearance)` rather
than invalidated in place, so there is no hidden state to get stale:

```rust
use silka_theme::{Appearance, ColorToken, Theme};
use silka_paint::Color;

let theme = Theme::cupertino(Appearance::Dark).with_accent(Color::hex(0xFF375F));
assert_eq!(theme.color_of(ColorToken::Accent), Color::hex(0xFF375F));

// Following the OS switching to light mode is one call, not a cache flush.
let light = theme.with_appearance(Appearance::Light);
assert_ne!(light.color_of(ColorToken::Background), theme.color_of(ColorToken::Background));
```

## A third preset

A custom brand fills in the same tokens — nothing else changes, and every
existing widget follows:

```rust
use silka_theme::{Appearance, ColorToken, Theme};
use silka_paint::Color;

let brand = Theme::cupertino(Appearance::Light)
    .map_colors(|token, value| match token {
        ColorToken::Accent => Color::hex(0x0A7D48),
        _ => value,
    });
assert_eq!(brand.color_of(ColorToken::Accent), Color::hex(0x0A7D48));
```

## License

MIT OR Apache-2.0
