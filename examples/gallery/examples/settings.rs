//! # `settings` — a macOS System Settings-shaped window
//!
//! A sidebar of sections on the left, a form on the right, and a status line
//! that is honest about what is wrong — the shape almost every desktop
//! application needs once and then copies forever.
//!
//! ```text
//! cargo run -p silka-gallery --example settings
//! ```
//!
//! What it is here to show: the **form layout** of `KOMPONEN.md` Tier 2, built
//! from `row`/`constrained` rather than a bespoke form widget; **one settings
//! record in one signal**, so nothing is kept in sync by hand; **a popup needs
//! a layer, not a coordinate** — the accent picker's panel goes to
//! [`overlay_layer`], the only thing here that knows where a popup lands
//! (`KOMPONEN.md` rule #3); and **validation as a pure function** ([`issues`]),
//! which is the part this file unit-tests.

use silka_core::access::AccessRole;
use silka_core::app::{BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, expanded, row, View};
use silka_paint::Insets;
use silka_platform::{run_app_with, window, PlatformError};
use silka_theme::{ColorToken, FontToken, Preset, Theme};
use silka_widgets::{
    active_fonts, advance, button_variant, checkbox, overlay_layer, select, slider, switch, text,
    text_field, ButtonVariant, Fonts, Select, SelectState,
};

const TITLE: &str = "Settings";
/// The accent picker's name for screen readers.
const ACCENT: &str = "Accent";
const ACCENTS: [&str; 4] = ["Blue", "Purple", "Green", "Graphite"];
/// Column widths in spacing-scale steps (§2.6), never in raw points.
const SIDEBAR_STEPS: f32 = 44.0;
const LABEL_STEPS: f32 = 34.0;
const CONTROL_STEPS: f32 = 60.0;

// --- the model --------------------------------------------------------------

/// One page of the settings window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Section {
    #[default]
    General,
    Appearance,
    Network,
}

impl Section {
    /// Every section, in sidebar order.
    const ALL: [Section; 3] = [Section::General, Section::Appearance, Section::Network];

    /// The sidebar label, which is also the section's accessible name.
    fn title(self) -> &'static str {
        ["General", "Appearance", "Network"]
            [Section::ALL.iter().position(|x| *x == self).unwrap_or(0)]
    }
}

/// Everything the window edits, in one value. The port is a `String` on
/// purpose: a half-typed number is a validation message, not a panic.
#[derive(Clone, Debug, PartialEq)]
struct Settings {
    display_name: String,
    launch_at_login: bool,
    menu_bar_icon: bool,
    text_size: f32,
    reduce_motion: bool,
    use_proxy: bool,
    proxy_host: String,
    proxy_port: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            display_name: "Ada".to_string(),
            launch_at_login: true,
            menu_bar_icon: false,
            text_size: 13.0,
            reduce_motion: false,
            use_proxy: false,
            proxy_host: "127.0.0.1".to_string(),
            proxy_port: "8080".to_string(),
        }
    }
}

/// Everything wrong with `s`, in the order it should be read out. Empty means
/// the window can be closed with a clear conscience.
fn issues(s: &Settings) -> Vec<String> {
    let mut out = Vec::new();
    if s.display_name.trim().is_empty() {
        out.push("Display name cannot be empty".to_string());
    }
    if s.use_proxy {
        if s.proxy_host.trim().is_empty() {
            out.push("A proxy needs a host".to_string());
        }
        if !matches!(s.proxy_port.trim().parse::<u32>(), Ok(1..=65_535)) {
            out.push("Port must be a number between 1 and 65535".to_string());
        }
    }
    out
}

/// The status line under the form.
fn status(s: &Settings) -> String {
    match issues(s).first() {
        Some(first) => first.clone(),
        None => format!("Text size {:.0} pt · saved", s.text_size),
    }
}

// --- the view ---------------------------------------------------------------

/// The whole window — this is what `run_app_with` is handed.
fn app(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution (§3.3).
    active_fonts().set_scale_factor(cx.expect_env::<Signal<ScaleFactor>>().get().get());

    let section = use_signal(Section::default);
    let settings = use_signal(Settings::default);
    let accent = use_signal(|| SelectState::with_selected(0));

    // Built at the root because its panel belongs to the overlay layer: the
    // trigger travels down into the pane, the panel travels nowhere.
    let picker = select(ACCENTS).label(ACCENT).key("accent").bind(accent);

    // Switching section rebuilds this whole window, which is the honest shape:
    // a settings pane *is* a different screen, not a detail of one.
    let content = row([
        sidebar(&t, section),
        View::from(expanded(pane(&t, section.get(), settings, &picker))),
    ])
    .cross(CrossAlign::Stretch);

    overlay_layer(content).overlay(picker.popup()).into()
}

/// The source list on the left.
fn sidebar(t: &Theme, section: Signal<Section>) -> View {
    let items: Vec<View> = Section::ALL
        .into_iter()
        .map(|s| {
            button_variant(s.title(), ButtonVariant::Ghost)
                .key(s.title())
                .toggled(s == section.get())
                .on_press(move || section.set(s))
                .into()
        })
        .collect();
    let w = t.space(SIDEBAR_STEPS);

    constrained(
        BoxConstraints::new(w, w, 0.0, f32::INFINITY),
        column(items)
            .spacing(t.space(1.0))
            .cross(CrossAlign::Stretch)
            .padding(Insets::all(t.space(3.0))),
    )
    .background(t.color.surface_sunken)
    .border(t.space(0.25), t.color.separator)
    .into()
}

/// The form on the right: a heading, the section's fields, and the status line.
fn pane(t: &Theme, section: Section, settings: Signal<Settings>, picker: &Select) -> View {
    let s = settings.get();
    let fields: Vec<(&str, View)> = match section {
        Section::General => vec![
            (
                "Display name",
                text_field(s.display_name.clone())
                    .key("display-name")
                    .label("Display name")
                    .placeholder("Your name")
                    .on_change(move |v| settings.update(|s| s.display_name = v.to_string()))
                    .into(),
            ),
            (
                "Startup",
                switch("Launch at login")
                    .key("launch")
                    .on(s.launch_at_login)
                    .on_change(move |v| settings.update(|s| s.launch_at_login = v))
                    .into(),
            ),
            (
                "Menu bar",
                checkbox("Show the icon")
                    .key("menu-bar")
                    .checked(s.menu_bar_icon)
                    .on_toggle(move |v| settings.update(|s| s.menu_bar_icon = v))
                    .into(),
            ),
        ],
        Section::Appearance => vec![
            (ACCENT, picker.trigger()),
            (
                "Text size",
                slider(s.text_size)
                    .range(11.0..=20.0)
                    .step(1.0)
                    .label("Text size")
                    .on_change(move |v| settings.update(|s| s.text_size = v))
                    .into(),
            ),
            (
                "Motion",
                switch("Reduce motion")
                    .key("motion")
                    .on(s.reduce_motion)
                    .on_change(move |v| settings.update(|s| s.reduce_motion = v))
                    .into(),
            ),
        ],
        Section::Network => vec![
            (
                "Proxy",
                switch("Route through a proxy")
                    .key("proxy")
                    .on(s.use_proxy)
                    .on_change(move |v| settings.update(|s| s.use_proxy = v))
                    .into(),
            ),
            (
                "Host",
                text_field(s.proxy_host.clone())
                    .key("host")
                    .label("Host")
                    .disabled(!s.use_proxy)
                    .on_change(move |v| settings.update(|s| s.proxy_host = v.to_string()))
                    .into(),
            ),
            (
                "Port",
                text_field(s.proxy_port.clone())
                    .key("port")
                    .label("Port")
                    .disabled(!s.use_proxy)
                    .on_change(move |v| settings.update(|s| s.proxy_port = v.to_string()))
                    .into(),
            ),
        ],
    };

    let mut rows = vec![View::from(
        text(section.title())
            .font(FontToken::Title2)
            .text_color(ColorToken::Label)
            .single_line(),
    )];
    rows.extend(fields.into_iter().map(|(l, c)| field(t, l, c)));
    rows.push(View::from(
        text(status(&s))
            .text_color(match issues(&s).is_empty() {
                true => ColorToken::SecondaryLabel,
                false => ColorToken::Destructive,
            })
            .single_line(),
    ));

    column(rows)
        .spacing(t.space(4.0))
        .cross(CrossAlign::Start)
        .padding(Insets::all(t.space(8.0)))
        .into()
}

/// One form row: a right-aligned label, then the control. The label carries the
/// `Container` role on purpose — the control announces its own name, and a
/// screen reader must not hear it twice (§3.8).
fn field(t: &Theme, label: &str, control: View) -> View {
    let (l, c) = (t.space(LABEL_STEPS), t.space(CONTROL_STEPS));
    row([
        View::from(constrained(
            BoxConstraints::new(l, l, 0.0, f32::INFINITY),
            row([View::from(
                text(label)
                    .text_color(ColorToken::SecondaryLabel)
                    .single_line()
                    .role(AccessRole::Container),
            )])
            .main(MainAlign::End)
            .cross(CrossAlign::Center),
        )),
        View::from(constrained(
            BoxConstraints::new(c, c, 0.0, f32::INFINITY),
            control,
        )),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .into()
}

#[cfg_attr(test, allow(dead_code))]
fn main() -> Result<(), PlatformError> {
    // One text engine for the whole application, installed once: it is what
    // every `text("…")` in this file resolves against (§2.5, §3.3).
    let fonts = Fonts::new();
    silka_widgets::install_fonts(&fonts);

    run_app_with(
        window(TITLE)
            .size(760.0, 560.0)
            .min_size(620.0, 420.0)
            .preset(Preset::Cupertino)
            .follow_system_appearance()
            // Without this the `GlyphRun` commands carry no bitmaps and the
            // window renders blank — the atlas is what crosses to the GPU.
            .glyphs(fonts.shared()),
        app,
        advance,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_platform::headless_app;
    use silka_theme::Appearance;

    #[test]
    fn defaults_are_valid_and_a_blank_name_is_not() {
        assert!(issues(&Settings::default()).is_empty());
        assert_eq!(status(&Settings::default()), "Text size 13 pt · saved");

        let blank = Settings {
            display_name: "   ".to_string(),
            ..Settings::default()
        };
        assert_eq!(issues(&blank), vec!["Display name cannot be empty"]);
    }

    #[test]
    fn the_proxy_is_only_validated_when_it_is_used() {
        let off = Settings {
            proxy_host: String::new(),
            proxy_port: "nonsense".to_string(),
            ..Settings::default()
        };
        assert!(
            issues(&off).is_empty(),
            "an unused proxy is nobody's problem"
        );

        let on = Settings {
            use_proxy: true,
            ..off
        };
        assert_eq!(issues(&on).len(), 2);
        assert_eq!(status(&on), "A proxy needs a host");

        for port in ["0", "65536", "-1", "80.5", ""] {
            let s = Settings {
                proxy_host: "proxy.local".to_string(),
                proxy_port: port.to_string(),
                ..on.clone()
            };
            assert_eq!(issues(&s).len(), 1, "port {port:?} should be rejected");
        }
    }

    /// The window assembles and both columns announce themselves — the a11y
    /// tree is the contract, so it is what the test reads (§3.8).
    #[test]
    fn the_window_builds_and_announces_its_parts() {
        let mut ui = headless_app(Theme::cupertino(Appearance::Dark), app).sized(900.0, 620.0);
        ui.frame();

        let tree = ui.access_tree();
        for name in ["General", "Appearance", "Network", "Display name"] {
            assert!(
                tree.find_label(name).is_some(),
                "no node named {name:?}:\n{}",
                tree.dump()
            );
        }
        assert!(ui.is_idle(), "an idle application schedules no frame");
    }
}
