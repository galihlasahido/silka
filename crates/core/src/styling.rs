//! **How to style anything in silka** — the utility vocabulary, start to
//! finish.
//!
//! This module contains no code. It is the page to read before writing a
//! screen, because the vocabulary described here is not *a* way to arrange a
//! view — it is **the** way (REKOMENDASI §2.6). Everything else
//! ([`Builder::background`](crate::view::Builder::background),
//! [`Builder::corners`](crate::view::Builder::corners),
//! [`Builder::padding`](crate::view::Builder::padding)) is the layer
//! underneath, kept public because the vocabulary is built on it and because a
//! widget author occasionally needs it.
//!
//! ```
//! use silka_core::view::div;
//! use silka_theme::ColorToken;
//!
//! let _ = div()
//!     .flex()
//!     .items_center()
//!     .justify_between()
//!     .gap_3()
//!     .px_4()
//!     .py_2()
//!     .rounded_lg()
//!     .bg(ColorToken::Surface)
//!     .shadow_md();
//! ```
//!
//! Tailwind's spelling, Apple's numbers, and a front door only design tokens
//! fit through. Four things are worth knowing before the reference below:
//! **who supplies the values**, **what a number means**, **how a state is
//! written**, and **where the theme comes from**.
//!
//! # 1. Values are roles, not colors
//!
//! Every styling utility takes a **token** — a name for a role in the design
//! system — never a literal:
//!
//! ```
//! use silka_core::view::div;
//! use silka_theme::{ColorToken, RadiusToken, ShadowToken, SpaceToken};
//!
//! let _ = div()
//!     .bg(ColorToken::SurfaceElevated)   // "an elevated surface", not #1C1C1E
//!     .border_1()                        // the hairline weight
//!     .border_color(ColorToken::Separator)
//!     .rounded(RadiusToken::Lg)
//!     .elevation(ShadowToken::Md)
//!     .p(SpaceToken::S4);
//! ```
//!
//! A literal does not type-check, which is the entire point — before this
//! vocabulary the rule "values are locked to design tokens" was guarded by a
//! doc-comment, and a reviewer had to notice:
//!
//! ```compile_fail
//! use silka_core::view::div;
//! use silka_paint::Color;
//!
//! // error[E0308]: expected `ColorToken`, found `Color`
//! let _ = div().bg(Color::hex(0x1E90FF));
//! ```
//!
//! A color the design system genuinely cannot know — a brand plate, a
//! user-picked label color, a swatch in a color picker — still has a way out,
//! spelled so that it stands out in a diff:
//! [`bg_raw`](crate::view::Builder::bg_raw),
//! [`rounded_raw`](crate::view::Builder::rounded_raw),
//! [`shadow_raw`](crate::view::Builder::shadow_raw),
//! [`p_raw`](crate::view::Builder::p_raw). A UI *chrome* color that reaches for
//! one of these is a missing token, not a special case.
//!
//! # 2. `p_4()` is four steps, not four points
//!
//! Spacing is a scale, and the number counts **steps** on it: `p_4()` is 4 × the
//! preset's unit — 16pt on both first-party presets (§2.6). The same numbering
//! runs through `p_*`, `px_*`, `py_*`, `pt/pr/pb/pl_*`, `gap_*`, and `m_*` on
//! flex items.
//!
//! ```
//! use silka_core::view::{active_theme, div, with_theme};
//! use silka_theme::{Appearance, SpaceToken, Theme};
//!
//! with_theme(Theme::cupertino(Appearance::Light), || {
//!     // 4 steps of a 4pt unit.
//!     assert_eq!(active_theme().space(4.0), 16.0);
//!     let _ = div().p_4().gap_2();
//! });
//! ```
//!
//! Two spellings deliberately break the pattern because they are not layout
//! rhythm: `border_1()` is the **1pt hairline** every separator in the HIG uses,
//! and `p_px()` is that same single point as padding.
//!
//! # 3. States are closures, and they transition on their own
//!
//! ```
//! use silka_core::view::{fixed, interactive};
//! use silka_theme::ColorToken;
//!
//! let _ = interactive(fixed(240.0, 88.0))
//!     .label("Card")
//!     .bg(ColorToken::Surface)
//!     .rounded_lg()
//!     .hover(|s| s.bg(ColorToken::SurfaceHover))
//!     .pressed(|s| s.bg(ColorToken::SurfacePressed).scale(0.98))
//!     .focused(|s| s.ring(ColorToken::FocusRing))
//!     .disabled_style(|s| s.bg(ColorToken::SurfaceSunken));
//! ```
//!
//! Each closure receives a [`StateStyle`](crate::tree::StateStyle) and speaks
//! the same vocabulary as the resting style, so nothing new has to be learned.
//! What is **absent** is any way to say how long the change takes: the node
//! keeps one spring per animatable property and retargets it as the state
//! changes (§3.5). An application cannot write a hover that snaps, and cannot
//! write one that lags either.
//!
//! Retargeting carries velocity, so a pointer that leaves halfway reverses
//! without a seam. Under reduced motion colors land instantly, the focus ring
//! still grows, and `scale` does not happen at all — a box that blinks smaller
//! within one frame is exactly the flicker the setting exists to remove.
//!
//! # 4. The theme is ambient
//!
//! Tokens are inert; the active [`Theme`](silka_theme::Theme) turns them into
//! numbers. Rather than thread `theme` through every call, it is installed for
//! the duration of a build pass — [`AppRuntime::frame`](crate::app::AppRuntime::frame)
//! does it from the `Signal<Theme>` in [`Env`](crate::app::Env), so an
//! application never writes it:
//!
//! ```
//! use silka_core::app::app;
//! use silka_core::view::{div, View};
//! use silka_paint::{Command, Scene};
//! use silka_theme::{Appearance, ColorToken, Theme};
//!
//! let theme = Theme::tailwind(Appearance::Dark);
//! let mut ui = app(|_cx| View::from(div().bg(ColorToken::Surface)))
//!     .with_env(move |rt| rt.signal(theme))
//!     .sized(200.0, 120.0);
//! ui.frame();
//!
//! // The token became this preset's surface colour, in the paint pass, with
//! // no theme mentioned in the view at all.
//! let color = match &ui.scene().commands()[0] {
//!     Command::Quad(q) => q.background,
//!     other => panic!("not a quad: {other:?}"),
//! };
//! assert_eq!(color, theme.color.surface);
//! ```
//!
//! Resolution happens **while the view is built**, so the values that reach the
//! render tree are already concrete and the paint pass and renderer stay
//! entirely theme-free (§3.2). Two consequences worth remembering:
//!
//! - a component that is not rebuilt keeps the colors it was built with, which
//!   is why the theme is injected as a `Signal<Theme>` — writing it marks its
//!   readers dirty and they rebuild in the new palette;
//! - outside a frame (a unit test that builds a view by hand) the ambient theme
//!   is [`Theme::default`](silka_theme::Theme::default). Utilities never panic
//!   and never resolve to nothing; wrap the code in
//!   [`with_theme`](crate::view::with_theme) when a specific preset matters.
//!
//! # Reference
//!
//! | Family | Utilities |
//! |---|---|
//! | Container | [`div()`](crate::view::div), [`container()`](crate::view::container) |
//! | Direction | `flex()`, `flex_row()`, `flex_col()`, `wrap()`, `wrap_reverse()`, `nowrap()`, `reverse()` |
//! | Cross axis | `items_start()`, `items_center()`, `items_end()`, `items_stretch()`, `items_baseline()` |
//! | Main axis | `justify_start()`, `justify_center()`, `justify_end()`, `justify_between()`, `justify_around()`, `justify_evenly()` |
//! | Flex item | `flex_1()`, `flex_auto()`, `flex_none()`, [`expanded()`](crate::view::expanded), [`flexible()`](crate::view::flexible) |
//! | Gap | `gap_0()`…`gap_12()`, `gap_token(SpaceToken)` |
//! | Padding | `p_*`, `px_*`, `py_*`, `pt_*`, `pr_*`, `pb_*`, `pl_*`, `p_raw(Insets)` |
//! | Margin (items) | `m_*`, `mx_*`, `my_*`, `mt_*`, `mr_*`, `mb_*`, `ml_*` |
//! | Color | `bg(ColorToken)`, `border_color(ColorToken)`, `bg_raw(Color)` |
//! | Radius | `rounded_none/sm/md/lg/xl/full()`, `rounded(RadiusToken)`, `rounded_raw(Corners)` |
//! | Border | `border_0()`, `border_1()`, `border_2()`, `border_4()` |
//! | Elevation | `shadow_none/sm/md/lg/xl()`, `elevation(ShadowToken)`, `shadow_raw(ShadowPair)` |
//! | Typography | `font(FontToken)`, `text_xs/sm/base/lg/xl/2xl/3xl()`, `font_regular/medium/semibold/bold()`, `italic()`, `text_color(ColorToken)` |
//! | States | `hover(…)`, `pressed(…)`, `focused(…)`, `disabled_style(…)`, `ring(ColorToken)` |
//!
//! The typography row lives on the text leaf, which is in `silka-widgets`
//! (it needs the font stack); the trait that plugs it into this vocabulary is
//! [`TextStyled`](crate::view::TextStyled).
//!
//! # Adopting it in existing code
//!
//! The rewrite is mechanical, and it is worth doing in one pass per file rather
//! than gradually — a file in two dialects is harder to read than a file in
//! either one.
//!
//! | Written by hand | In the vocabulary |
//! |---|---|
//! | `.padding(Insets::all(t.space(6.0)))` | `.p_6()` |
//! | `.spacing(t.space(4.0))` | `.gap_4()` |
//! | `.cross(CrossAlign::Center)` | `.items_center()` |
//! | `.main(MainAlign::SpaceBetween)` | `.justify_between()` |
//! | `.background(t.color.surface)` | `.bg(ColorToken::Surface)` |
//! | `.border(t.space(0.25), t.color.separator)` | `.border_1().border_color(ColorToken::Separator)` |
//! | `.corners(t.radius.corners(RadiusToken::Lg))` | `.rounded_lg()` |
//! | `.shadow(t.shadow.md)` | `.shadow_md()` |
//! | `.size(t.typography.body_size).color(t.color.label)` | `.text_base().text_color(ColorToken::Label)` |
//!
//! Two things usually disappear along with the calls: the `let t: Theme = …`
//! binding used only for lookups, and the `Insets`/`CrossAlign`/`MainAlign`
//! imports. What is gained is not brevity but **reviewability** — a diff that
//! says `bg(ColorToken::Surface)` cannot quietly become a slightly different
//! grey.
//!
//! A worked example lives in the gallery: `examples/gallery/src/reactive.rs`
//! (a page rewritten from hand-assembled layout, with hover/press/focus gained
//! on the way) and `examples/gallery/src/utility.rs`, which is the vocabulary
//! itself as a running reference page.
