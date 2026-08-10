//! Token resolution → concrete values for `tabs` (§2.6, §2.7).
//!
//! This is the only file in this component allowed to mention [`Theme`]. The
//! render nodes in [`super::list`] and [`super::item`] only ever receive a
//! [`TabsStyle`] that **already** holds finished colors, radii, and spacing —
//! the same rule as [`silka_core::tree::Decoration`]: the engine has no opinion
//! about color, so the Cupertino/Tailwind presets swap without a single line
//! changing in the node code.
//!
//! Corner geometry comes along as a **parameter** (squircle on Cupertino, arc
//! on Tailwind) — not a constant, because that shape flows all the way into the
//! shader *and* into hit-testing (§2.7, §3.6).

use silka_core::tree::{Decoration, FocusRing};
use silka_paint::{Color, CornerRadii, Corners, Insets, Rect, ShadowPair};
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;

/// Visual variant of a tab row (`KOMPONEN.md` Tier 3: segmented/underline/
/// enclosed).
///
/// All three share **one** engine: the only differences are the token values
/// resolved here and the shape of the indicator rect
/// ([`TabsStyle::indicator_rect`]). Not one of them has a layout or input path
/// of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TabsVariant {
    /// A thumb sliding inside a "well" — `NSSegmentedControl`/
    /// `UISegmentedControl`. Equal-width segments.
    #[default]
    Segmented,
    /// A thick bar under the active tab — the shadcn/ui and web-toolbar style.
    Underline,
    /// Folder-shaped tabs that merge into the panel below them.
    Enclosed,
}

impl TabsVariant {
    /// All three variants — used by the gallery and cross-variant tests.
    pub const ALL: [TabsVariant; 3] = [
        TabsVariant::Segmented,
        TabsVariant::Underline,
        TabsVariant::Enclosed,
    ];

    /// Short name for CLI/gallery/debug output.
    pub const fn name(self) -> &'static str {
        match self {
            TabsVariant::Segmented => "segmented",
            TabsVariant::Underline => "underline",
            TabsVariant::Enclosed => "enclosed",
        }
    }
}

/// Every visual value for `tabs`, **already resolved** from the theme tokens.
///
/// Split out from the builder so it can be tested without a render tree at all:
/// the question "does the underline indicator really sit flush against the
/// bottom edge of the selected tab" needs no GPU, no window, and no tree —
/// only [`TabsStyle::indicator_rect`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabsStyle {
    /// The variant that determines the indicator's shape.
    pub variant: TabsVariant,

    /// Background of the whole row (the segmented well; empty for the others).
    pub track: Decoration,
    /// Hairline spanning the row along its bottom edge (underline & enclosed).
    pub rail: Option<Color>,
    /// Thickness of that hairline, in logical points.
    pub rail_thickness: f32,

    /// Background, border, corners, and shadows of the moving indicator.
    pub indicator: Decoration,
    /// Inset of the indicator from the edges of the selected tab's rect.
    pub indicator_inset: Insets,
    /// Indicator thickness for the [`TabsVariant::Underline`] variant.
    pub indicator_thickness: f32,

    /// Keyboard focus ring (token `focus_ring`).
    pub focus_ring: FocusRing,

    /// Padding inside the row's edges.
    pub padding: Insets,
    /// Gap between tabs.
    pub spacing: f32,
    /// Minimum height of one tab — the HIG hit target (`KOMPONEN.md` DoD).
    pub min_height: f32,
    /// Every tab as wide as the widest one (the `NSSegmentedControl` feel).
    pub equal_widths: bool,

    /// Corner shape of one tab: used by the hover highlight **and** hit-testing
    /// (§3.6).
    pub tab_corners: Corners,
    /// Padding inside one tab's edges.
    pub tab_padding: Insets,

    /// Highlight while the pointer is over a tab.
    pub hover: Color,
    /// Highlight while a tab is pressed.
    pub pressed: Color,

    /// Label color of an unselected tab.
    pub label: Color,
    /// Label color of the selected tab.
    pub selected_label: Color,
    /// Label color of a disabled tab.
    pub disabled_label: Color,
    /// Label font size, in logical points.
    pub label_size: f32,
}

impl TabsStyle {
    /// Resolve every token for a given variant.
    ///
    /// Not a single color value originates here: everything derives from
    /// [`Theme`], so both presets are automatically correct and dark mode
    /// follows without an `if` branch.
    pub fn from_theme(theme: &Theme, variant: TabsVariant) -> Self {
        let rambut = theme.space(0.25);
        let dasar = Self {
            variant,
            track: Decoration::NONE,
            rail: None,
            rail_thickness: rambut,
            indicator: Decoration::NONE,
            indicator_inset: Insets::ZERO,
            indicator_thickness: theme.space(0.5),
            focus_ring: FocusRing::new(theme.space(0.5), theme.color.focus_ring),
            padding: Insets::ZERO,
            spacing: theme.space(1.0),
            min_height: MIN_HIT_TARGET,
            equal_widths: false,
            tab_corners: theme.corners(theme.radius.sm),
            tab_padding: Insets::symmetric(theme.space(3.0), theme.space(1.5)),
            hover: theme.color.surface_hover,
            pressed: theme.color.surface_pressed,
            label: theme.color.secondary_label,
            selected_label: theme.color.label,
            disabled_label: theme.color.disabled_label,
            label_size: theme.typography.body_size,
        };

        match variant {
            TabsVariant::Segmented => {
                let sumur = theme.space(0.5);
                let dalam = (theme.radius.md - sumur).max(0.0);
                Self {
                    track: Decoration::fill(theme.color.surface_sunken)
                        .corners(theme.corners(theme.radius.md))
                        .border(rambut, theme.color.separator),
                    indicator: Decoration::fill(theme.color.surface_elevated)
                        .corners(theme.corners(dalam))
                        .border(rambut, theme.color.separator)
                        .shadows(theme.shadow.sm),
                    tab_corners: theme.corners(dalam),
                    padding: Insets::all(sumur),
                    spacing: 0.0,
                    equal_widths: true,
                    ..dasar
                }
            }
            TabsVariant::Underline => Self {
                rail: Some(theme.color.separator),
                indicator: Decoration::fill(theme.color.accent)
                    .corners(theme.corners(theme.space(0.25))),
                spacing: theme.space(1.0),
                ..dasar
            },
            TabsVariant::Enclosed => Self {
                rail: Some(theme.color.separator),
                indicator: Decoration::fill(theme.color.surface_elevated)
                    .corners(Corners::new(
                        CornerRadii {
                            top_left: theme.radius.md,
                            top_right: theme.radius.md,
                            bottom_right: 0.0,
                            bottom_left: 0.0,
                        },
                        theme.radius.style,
                    ))
                    .border(rambut, theme.color.separator),
                spacing: theme.space(0.5),
                ..dasar
            },
        }
    }

    /// Indicator rect for the tab occupying `tab` (row-local coordinates).
    ///
    /// This is the one place where the three variants differ geometrically —
    /// and because it is a pure function, all of that difference can be tested
    /// without touching a render tree.
    pub fn indicator_rect(&self, tab: Rect) -> Rect {
        let kotak = tab.deflate(self.indicator_inset);
        match self.variant {
            TabsVariant::Segmented | TabsVariant::Enclosed => kotak,
            TabsVariant::Underline => {
                let tebal = self.indicator_thickness.min(kotak.size.height);
                Rect::new(
                    kotak.min_x(),
                    kotak.max_y() - tebal,
                    kotak.size.width,
                    tebal,
                )
            }
        }
    }

    /// True when the indicator contributes any pixels at all.
    pub fn indicator_is_visible(&self) -> bool {
        self.indicator.is_visible()
    }

    /// The indicator's shadows (empty for variants without elevation).
    pub fn indicator_shadows(&self) -> ShadowPair {
        self.indicator.shadows
    }
}
