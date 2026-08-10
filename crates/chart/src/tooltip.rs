//! **The hover tooltip** — and the reason it is not a popup.
//!
//! `KOMPONEN.md` working rule #3 is explicit: the overlay system is built once
//! and ten components ride it — dialog, sheet, popover, tooltip, menu, toast.
//! A chart tooltip that positioned itself would be the eleventh implementation
//! of edge auto-flip, and the first one to get it wrong at the right-hand edge
//! of the window.
//!
//! So this module produces two things and **no geometry at all**:
//!
//! 1. [`ChartHover`] — what the pointer is over, as data: which point, what its
//!    title is, one entry per series, and the **anchor rect** the panel should
//!    be placed against.
//! 2. [`tooltip`] — the panel's *content* as an ordinary view, and
//!    [`tooltip_overlay`] which hands that content to
//!    [`silka_widgets::overlay::overlay`] together with an [`Anchor`] and a
//!    [`Placement`]. Where the panel ends up, whether it flips above or below,
//!    and how it springs in are all the overlay system's answers, unchanged.
//!
//! ## Why the anchor is a global rect
//!
//! The chart node knows only its own local coordinates, and the overlay layer
//! wants layer-local ones. Rather than have the chart guess, [`ChartHover`]
//! carries the anchor in **global** (window) coordinates — the one frame of
//! reference both parties can compute without knowing about each other — and
//! [`ChartHover::anchor_in`] converts it for a particular layer. When the layer
//! is the window root, which it is in the common case, the conversion is the
//! identity.
//!
//! ## The barrier
//!
//! [`Barrier::None`]. A tooltip must never catch the mouse passing beneath it:
//! it would swallow the very pointer motion that keeps it alive, and the panel
//! would flicker at exactly the moment the reader moves toward it.

use silka_core::signals::Key;
use silka_core::tree::{NodeId, RenderTree};
use silka_core::view::View;
use silka_paint::{Color, Insets, Point, Rect};
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::overlay::{overlay, Anchor, Barrier, Dismiss, OverlayBuilder, Placement, Side};
use silka_widgets::{text, Fonts};

use crate::style::ChartStyle;

/// One series' contribution to what is under the pointer.
#[derive(Debug, Clone, PartialEq)]
pub struct HoverEntry {
    /// Which series (its index, so the color can be looked up again).
    pub series: usize,
    /// The series name.
    pub name: String,
    /// The raw value — the **target** value, not the animating one: a tooltip
    /// that counted up while the spring settled would be unreadable.
    pub value: f64,
    /// The value already formatted for this chart's locale.
    pub text: String,
    /// The series color, so the tooltip can carry the same identity the mark
    /// does.
    pub color: Color,
}

/// What the pointer is currently over.
///
/// A plain value handed to the application through
/// [`on_hover`](crate::ChartBuilder::on_hover), which stores it in a signal like
/// any other state. Nothing here is a widget and nothing here is positioned —
/// see the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartHover {
    /// The data point's index.
    pub index: usize,
    /// The point's title — its category name, or its date.
    pub title: String,
    /// One entry per series that has a value here.
    pub entries: Vec<HoverEntry>,
    /// The rect to anchor a panel against, in **global** (window) coordinates.
    pub anchor: Rect,
}

impl ChartHover {
    /// The anchor translated into a layer's local coordinates.
    ///
    /// Returns [`Anchor::None`] when the layer is no longer in the tree — a
    /// vanished layer means the panel falls back to the centre, not to garbage
    /// coordinates (the same contract as
    /// [`silka_widgets::overlay::anchor_rect`]).
    pub fn anchor_in(&self, tree: &RenderTree, layer: NodeId) -> Anchor {
        if !tree.contains(layer) {
            return Anchor::None;
        }
        let asal = tree.global_offset(layer);
        Anchor::Rect(self.anchor.translated(Point::new(-asal.x, -asal.y)))
    }

    /// The anchor as-is, for the common case where the overlay layer **is** the
    /// window root.
    pub fn anchor(&self) -> Anchor {
        Anchor::Rect(self.anchor)
    }

    /// A one-line summary — what a screen reader announces, and what a test can
    /// assert on without going near a pixel.
    pub fn summary(&self) -> String {
        let mut out = self.title.clone();
        for e in &self.entries {
            out.push_str("; ");
            out.push_str(&e.name);
            out.push_str(": ");
            out.push_str(&e.text);
        }
        out
    }
}

/// The tooltip panel's **content** — an ordinary view, built from tokens.
///
/// Kept separate from [`tooltip_overlay`] so an application that wants its own
/// panel (a sparkline showing a mini table, a chart showing an image) can place
/// that content through the same overlay path instead of inventing a second
/// one.
pub fn tooltip(fonts: &Fonts, theme: &Theme, style: &ChartStyle, hover: &ChartHover) -> View {
    use silka_core::tree::CrossAlign;
    use silka_core::view::{column, pad, row};

    let mut baris: Vec<View> = Vec::with_capacity(hover.entries.len() + 1);
    baris.push(
        text(fonts, hover.title.clone())
            .size(theme.typography.footnote.size)
            .weight(FontWeight::SEMIBOLD)
            .color(theme.color.label)
            .single_line()
            .into(),
    );
    for e in &hover.entries {
        baris.push(
            row([
                // The swatch carries the identity so the *text* does not have
                // to: values and names stay in ink colors, never in the series
                // color, which is unreadable at label size.
                View::from(
                    silka_core::view::fixed(style.swatch_size * 0.6, style.swatch_size * 0.6)
                        .background(e.color)
                        .corners(theme.corners(style.swatch_size * 0.3)),
                ),
                View::from(
                    text(fonts, e.name.clone())
                        .size(theme.typography.footnote.size)
                        .color(theme.color.secondary_label)
                        .single_line(),
                ),
                View::from(
                    text(fonts, e.text.clone())
                        .size(theme.typography.footnote.size)
                        .weight(FontWeight::MEDIUM)
                        .color(theme.color.label)
                        .single_line(),
                ),
            ])
            .spacing(theme.space(1.5))
            .cross(CrossAlign::Center)
            .into(),
        );
    }

    pad(
        Insets::symmetric(theme.space(2.5), theme.space(2.0)),
        column(baris).spacing(theme.space(1.0)),
    )
    .background(theme.color.surface_elevated)
    .corners(theme.corners_of(silka_theme::RadiusToken::Md))
    .border(
        theme.space_of(silka_theme::SpaceToken::Px),
        theme.color.separator,
    )
    .shadow(theme.shadow_of(silka_theme::ShadowToken::Lg))
    .into()
}

/// A ready-made tooltip overlay: the panel above the hovered point, flipping at
/// the window edge, springing in and out.
///
/// `hover` being `None` is not "do not build the overlay" — it is "the overlay
/// is closed". The entry stays in the tree so its **disappearance** animates
/// too; that is rule #2 of the overlay module, and skipping it is what makes
/// tooltips snap out of existence.
pub fn tooltip_overlay(
    fonts: &Fonts,
    theme: &Theme,
    style: &ChartStyle,
    hover: Option<&ChartHover>,
    anchor: Anchor,
) -> OverlayBuilder {
    let panel = match hover {
        Some(h) => tooltip(fonts, theme, style, h),
        // A closed tooltip still needs a panel to fade out; an empty box is the
        // cheapest one that keeps the transition honest.
        None => silka_core::view::fixed(0.0, 0.0).into(),
    };
    overlay(panel)
        .key(Key::from("chart-tooltip"))
        .open(hover.is_some())
        .anchor(anchor)
        .placement(Placement::anchored(Side::Top).gap(theme.space(2.0)))
        .no_backdrop()
        // A tooltip must never catch the mouse passing beneath it: it would
        // swallow the pointer motion that keeps it alive.
        .barrier(Barrier::None)
        .dismiss(Dismiss::NONE)
        .role(silka_core::access::AccessRole::Tooltip)
        .label(hover.map(ChartHover::summary).unwrap_or_default())
        // Decorative: a tooltip's motion explains nothing, so reduced-motion
        // removes it entirely rather than merely calming it (§3.5).
        .decorative()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::tree::{BoxConstraints, RenderTree};
    use silka_core::view::reconcile;
    use silka_paint::Size;
    use silka_theme::{Appearance, Theme};
    use silka_widgets::overlay::overlay_layer;

    fn hover() -> ChartHover {
        ChartHover {
            index: 2,
            title: "Q3".into(),
            entries: vec![
                HoverEntry {
                    series: 0,
                    name: "Pendapatan".into(),
                    value: 1_500_000.0,
                    text: "Rp 1.500.000".into(),
                    color: Color::hex(0x2A78D6),
                },
                HoverEntry {
                    series: 1,
                    name: "Biaya".into(),
                    value: 400_000.0,
                    text: "Rp 400.000".into(),
                    color: Color::hex(0xEB6834),
                },
            ],
            anchor: Rect::new(300.0, 120.0, 2.0, 180.0),
        }
    }

    #[test]
    fn ringkasan_membawa_judul_dan_setiap_deret() {
        let s = hover().summary();
        assert!(s.starts_with("Q3"));
        assert!(s.contains("Pendapatan: Rp 1.500.000"));
        assert!(s.contains("Biaya: Rp 400.000"));
    }

    #[test]
    fn anchor_diterjemahkan_ke_koordinat_lapisan() {
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            overlay_layer(silka_core::view::fixed(400.0, 300.0)),
        );
        tree.layout(BoxConstraints::tight(Size::new(400.0, 300.0)));
        let layer = tree.root();
        // The layer is the root here, so the conversion is the identity — and
        // that is exactly the case the common path takes.
        match hover().anchor_in(&tree, layer) {
            Anchor::Rect(r) => assert_eq!(r, hover().anchor),
            lain => panic!("anchor tak terduga: {lain:?}"),
        }
    }

    #[test]
    fn tooltip_menumpang_sistem_overlay_bukan_popup_sendiri() {
        // The check that this module never grows geometry of its own: what it
        // returns is an `OverlayBuilder`, so placement, flipping, and the
        // spring all belong to the overlay module.
        let fonts = Fonts::bundled_only();
        let t = Theme::cupertino(Appearance::Dark);
        let style = ChartStyle::from_theme(&t);
        let h = hover();
        let view = overlay_layer(silka_core::view::fixed(600.0, 400.0)).overlay(tooltip_overlay(
            &fonts,
            &t,
            &style,
            Some(&h),
            h.anchor(),
        ));

        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(Size::new(600.0, 400.0)));
        let entri = silka_widgets::overlay::entries(&tree);
        assert_eq!(entri.len(), 1, "tepat satu entri overlay");
    }

    #[test]
    fn tooltip_tertutup_tetap_berada_di_pohon_agar_bisa_menghilang_dengan_halus() {
        let fonts = Fonts::bundled_only();
        let t = Theme::cupertino(Appearance::Light);
        let style = ChartStyle::from_theme(&t);
        let view = overlay_layer(silka_core::view::fixed(600.0, 400.0)).overlay(tooltip_overlay(
            &fonts,
            &t,
            &style,
            None,
            Anchor::None,
        ));
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(Size::new(600.0, 400.0)));
        assert_eq!(silka_widgets::overlay::entries(&tree).len(), 1);
    }

    #[test]
    fn isi_tooltip_bisa_dibangun_tanpa_gpu() {
        let fonts = Fonts::bundled_only();
        let t = Theme::tailwind(Appearance::Dark);
        let style = ChartStyle::from_theme(&t);
        let mut tree = RenderTree::new();
        reconcile(&mut tree, tooltip(&fonts, &t, &style, &hover()));
        let ukuran = tree.layout(BoxConstraints::loose(Size::new(400.0, 300.0)));
        assert!(ukuran.width > 0.0 && ukuran.height > 0.0);
    }
}
