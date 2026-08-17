//! **One tick for every chart in the tree** — the same door
//! [`silka_widgets::advance`] provides for the widget catalogue.
//!
//! There is no ticking timer anywhere in this framework (§3.5). What there is
//! is a single [`Tick`] per frame, shared across the tree; values still moving
//! flag themselves on it, and only that flag schedules the next frame. This
//! module is that sharing for charts.
//!
//! It is a **separate** function from `silka_widgets::advance` for the reason
//! this crate exists at all: `silka-widgets` must stay lean and must not learn
//! about charts. An application showing both calls both — which is one line,
//! and the honest price of the split:
//!
//! ```no_run
//! # use silka_core::tree::RenderTree;
//! # use silka_core::animation::Tick;
//! # use silka_core::scheduler::Dirty;
//! fn animate(tree: &mut RenderTree, tick: &Tick) -> Dirty {
//!     silka_widgets::advance(tree, tick) | silka_chart::advance(tree, tick)
//! }
//! ```

use silka_core::animation::Tick;
use silka_core::scheduler::Dirty;
use silka_core::tree::RenderTree;

use crate::node::{walk, ChartBox};

/// Advance every chart animation by one frame.
///
/// The returned reasons mean exactly what they do everywhere else:
///
/// - [`Dirty::PAINT`] — a value moved this frame.
/// - [`Dirty::ANIMATION`] — a spring has not settled, so another frame is
///   needed. Once this is gone the GPU may sleep.
/// - [`Dirty::NONE`] — nothing in this crate is moving.
///
/// [`Dirty::LAYOUT`] is deliberately **never** returned: a chart's size does
/// not depend on its values, so animating data must not make the page around it
/// re-measure sixty times a second.
///
/// ```
/// use std::time::Duration;
///
/// use silka_chart::{advance, is_animating, line_chart_in};
/// use silka_core::animation::{Motion, Tick};
/// use silka_core::scheduler::Dirty;
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::Fonts;
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let mut tree = RenderTree::new();
/// reconcile(
///     &mut tree,
///     line_chart_in(&fonts, &theme, [1.0f64, 4.0, 2.0, 8.0])
///         .numeric()
///         .y_named("Value", |v: &f64| *v)
///         .animated(true),
/// );
/// tree.layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
///
/// // Values spring in from their baseline, so the first frames report work
/// // left to do; the frame loop keeps calling until they do not.
/// let mut frames = 0;
/// let tick = Tick::manual(Duration::from_micros(8_333), Motion::Full);
/// while is_animating(&tree) && frames < 600 {
///     let dirty = advance(&mut tree, &tick);
///     // A chart's size never depends on its values, so animating data must
///     // never force the page around it to re-measure.
///     assert!(!dirty.contains(Dirty::LAYOUT));
///     frames += 1;
/// }
///
/// // Once every spring has settled the GPU is allowed to sleep: no repaint is
/// // requested and no further frame is scheduled.
/// assert!(!is_animating(&tree));
/// assert_eq!(advance(&mut tree, &tick), Dirty::NONE);
/// ```
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in walk(tree) {
        let hasil = tree
            .node_mut_ref::<ChartBox>(id)
            .map(|c| (c.advance(tick), c.is_animating()));
        if let Some((bergerak, belum_selesai)) = hasil {
            if bergerak {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if belum_selesai {
                dirty |= Dirty::ANIMATION;
            }
        }
    }
    dirty
}

/// True while any chart in the tree is still animating.
///
/// This is the question that decides whether the GPU may sleep, so it must be
/// asked of the whole tree rather than of one node: a single unsettled chart
/// keeps the frame loop alive.
///
/// ```
/// use silka_chart::{is_animating, settle};
/// use silka_core::tree::RenderTree;
///
/// // A tree with no charts in it is trivially at rest — which is what lets an
/// // application call this unconditionally.
/// let mut empty = RenderTree::new();
/// assert!(!is_animating(&empty));
/// settle(&mut empty);
/// assert!(!is_animating(&empty));
/// ```
pub fn is_animating(tree: &RenderTree) -> bool {
    walk(tree).into_iter().any(|id| {
        tree.node_ref::<ChartBox>(id)
            .is_some_and(ChartBox::is_animating)
    })
}

/// Finish every chart animation instantly — golden tests and snapshots, where
/// "halfway through a spring" is not a state worth photographing.
///
/// ```
/// use silka_chart::{bar_chart_in, is_animating, settle};
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::Fonts;
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let mut tree = RenderTree::new();
/// reconcile(
///     &mut tree,
///     bar_chart_in(&fonts, &theme, [3.0f64, 7.0, 5.0])
///         .numeric()
///         .y_named("Value", |v: &f64| *v)
///         .animated(true),
/// );
/// tree.layout(BoxConstraints::tight(Size::new(240.0, 160.0)));
///
/// // Jump straight to the end state instead of pumping frames — a golden file
/// // should photograph the result, never a spring mid-flight.
/// settle(&mut tree);
/// assert!(!is_animating(&tree));
/// ```
pub fn settle(tree: &mut RenderTree) {
    for id in walk(tree) {
        if tree
            .node_mut_ref::<ChartBox>(id)
            .map(ChartBox::settle)
            .is_some()
        {
            tree.mark_needs_paint(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bar_chart_in, line_chart_in};
    use silka_core::animation::Motion;
    use silka_core::tree::BoxConstraints;
    use silka_core::view::reconcile;
    use silka_paint::Size;
    use silka_theme::{Appearance, Theme};
    use silka_widgets::Fonts;
    use std::time::Duration;

    fn pohon(animated: bool) -> RenderTree {
        let f = Fonts::bundled_only();
        let t = Theme::cupertino(Appearance::Dark);
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            bar_chart_in(&f, &t, vec![3.0f64, 9.0, 5.0])
                .y_named("a", |v: &f64| *v)
                .animated(animated),
        );
        tree.layout(BoxConstraints::tight(Size::new(400.0, 240.0)));
        tree
    }

    fn tick() -> Tick {
        Tick::manual(Duration::from_millis(16), Motion::Full)
    }

    #[test]
    fn chart_baru_tumbuh_dari_garis_dasar() {
        // Growth is what makes a chart feel like it arrived rather than blinked
        // into existence.
        let mut tree = pohon(true);
        assert!(is_animating(&tree));
        let d = advance(&mut tree, &tick());
        assert!(d.contains(Dirty::ANIMATION), "{d:?}");
        assert!(d.contains(Dirty::PAINT), "{d:?}");
        assert!(
            !d.contains(Dirty::LAYOUT),
            "nilai bergerak bukan alasan layout ulang"
        );
    }

    #[test]
    fn spring_akhirnya_diam_dan_gpu_boleh_tidur() {
        let mut tree = pohon(true);
        let mut dirty = Dirty::ANIMATION;
        let mut frame = 0;
        while dirty.contains(Dirty::ANIMATION) && frame < 600 {
            dirty = advance(&mut tree, &tick());
            frame += 1;
        }
        assert!(frame < 600, "spring tidak pernah selesai");
        assert!(!is_animating(&tree));
        assert_eq!(advance(&mut tree, &tick()), Dirty::NONE);
    }

    #[test]
    fn nilai_sebesar_miliaran_tetap_bisa_selesai() {
        // The trap this crate walked into once and must not walk into again: a
        // `SpringValue` decides it has arrived by an **absolute** tolerance of
        // 1/512, which is a fraction of a pixel in logical points and a
        // physical impossibility in rupiah — `f32` cannot resolve 1/512 near
        // 1.5e9 at all. The spring would never settle, the scheduler would
        // never idle, and the GPU would spin forever on a chart that had
        // visibly stopped moving.
        let f = Fonts::bundled_only();
        let t = Theme::cupertino(Appearance::Dark);
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            bar_chart_in(&f, &t, vec![1.5e9f64, 2.4e9, 0.9e9]).y_named("Rupiah", |v: &f64| *v),
        );
        tree.layout(BoxConstraints::tight(Size::new(400.0, 240.0)));

        let mut frame = 0;
        while is_animating(&tree) && frame < 600 {
            advance(&mut tree, &tick());
            frame += 1;
        }
        assert!(frame < 600, "spring bernilai miliaran tidak pernah selesai");

        // …and it arrived at the right place, not merely gave up.
        let id = tree.children(tree.root())[0];
        let g = tree
            .node_ref::<crate::ChartBox>(id)
            .unwrap()
            .geometry()
            .unwrap();
        assert!(g.value.domain().1 >= 2.4e9);
    }

    #[test]
    fn settle_menyelesaikan_seketika() {
        let mut tree = pohon(true);
        settle(&mut tree);
        assert!(!is_animating(&tree));
        assert_eq!(advance(&mut tree, &tick()), Dirty::NONE);
    }

    #[test]
    fn tanpa_animasi_tidak_ada_frame_yang_diminta() {
        let mut tree = pohon(false);
        assert!(!is_animating(&tree));
        assert_eq!(advance(&mut tree, &tick()), Dirty::NONE);
    }

    #[test]
    fn pohon_tanpa_chart_tidak_menuntut_apa_pun() {
        let f = Fonts::bundled_only();
        let t = Theme::cupertino(Appearance::Light);
        let mut tree = RenderTree::new();
        reconcile(&mut tree, silka_core::view::fixed(100.0, 100.0));
        tree.layout(BoxConstraints::tight(Size::new(100.0, 100.0)));
        assert_eq!(advance(&mut tree, &tick()), Dirty::NONE);
        assert!(!is_animating(&tree));
        // …and one that does have a chart, does.
        let mut tree2 = RenderTree::new();
        reconcile(
            &mut tree2,
            line_chart_in(&f, &t, vec![1.0f64, 2.0])
                .y_named("a", |v: &f64| *v)
                .numeric(),
        );
        tree2.layout(BoxConstraints::tight(Size::new(200.0, 100.0)));
        assert!(is_animating(&tree2));
    }
}
