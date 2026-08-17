//! `spacer()` — the Tier 0 gap (`KOMPONEN.md`).
//!
//! The smallest component in the catalogue, and the one whose absence was the
//! most visible: every page of the gallery and the dashboard had grown its own
//! `expanded(fixed(0.0, 0.0))`, which is the same idea spelled out three times
//! and explained in a comment each time.
//!
//! ```
//! use silka_core::view::{row, View};
//! use silka_widgets::{spacer, text};
//!
//! // Brand on the reading-start side, controls on the other. The gap belongs
//! // to the layout engine, not to a hand-computed number.
//! let bar = row([
//!     View::from(text("silka")),
//!     View::from(spacer()),
//!     View::from(text("Settings")),
//! ]);
//! # let _ = bar;
//! ```
//!
//! # Which gap to reach for
//!
//! | Situation | Reach for |
//! |---|---|
//! | Even spacing between **every** child | the container's `gap_*()` / `spacing()` |
//! | One child pushed to the far end | [`spacer()`] |
//! | Two groups sharing the leftover space | [`spacer_flex`] with different weights |
//! | A single fixed gap between two children | [`spacer_of`] |
//!
//! `gap_*()` is the first answer far more often than a spacer is: it is one
//! decision in one place, whereas a spacer between every pair is the same
//! decision repeated. The spacer earns its keep when the gap is **not** even —
//! which is exactly the top-bar case above.
//!
//! # Definition of done
//!
//! A spacer is not a control: it has no interactive state, no keyboard
//! behaviour, and no hit target of its own. What it does owe the contract is
//! silence — it emits [`silka_core::access::AccessRole::Container`], the
//! structural role assistive technology filters out, so a screen reader never
//! announces an empty box (§3.8).

use silka_core::view::{expanded, fixed, item, Builder, FixedProps, ItemProps};
use silka_theme::{SpaceToken, Theme};

use crate::ambient::active_theme;

/// A flexible gap that eats **all** the leftover space on the main axis.
///
/// The counterpart of Flutter's `Spacer`: a flex child of zero natural size
/// that grows into whatever is left, so whatever follows it is pushed to the
/// far end.
///
/// ```
/// use silka_core::view::{row, View};
/// use silka_widgets::{spacer, text};
///
/// let _ = row([
///     View::from(text("Title")),
///     View::from(spacer()),
///     View::from(text("Action")),
/// ]);
/// ```
pub fn spacer() -> Builder<ItemProps> {
    expanded(fixed(0.0, 0.0))
}

/// A flexible gap with an explicit weight.
///
/// Two spacers of weight `1.0` and `2.0` split the leftover space one third to
/// two thirds — which is how a control is centred *optically* rather than
/// geometrically.
///
/// ```
/// use silka_core::view::{row, View};
/// use silka_widgets::{spacer_flex, text};
///
/// let _ = row([
///     View::from(spacer_flex(1.0)),
///     View::from(text("Centred a third of the way in")),
///     View::from(spacer_flex(2.0)),
/// ]);
/// ```
pub fn spacer_flex(flex: f32) -> Builder<ItemProps> {
    let flex = if flex.is_finite() { flex.max(0.0) } else { 0.0 };
    item(fixed(0.0, 0.0)).grow(flex).shrink(1.0).basis(0.0)
}

/// A **fixed** gap of one spacing token, square on both axes.
///
/// The token is the point: the value is always a multiple of one scale step,
/// never an arbitrary number (§2.6). Square because a gap does not know which
/// axis it will end up on; in a row it contributes its width, and it only
/// affects the row's height if the row is shorter than the token — which, on
/// the 4pt scale, it essentially never is.
///
/// ```
/// use silka_core::view::{row, View};
/// use silka_theme::SpaceToken;
/// use silka_widgets::{spacer_of, text};
///
/// let _ = row([
///     View::from(text("Name")),
///     View::from(spacer_of(SpaceToken::S2)),
///     View::from(text("Value")),
/// ]);
/// ```
pub fn spacer_of(token: SpaceToken) -> Builder<FixedProps> {
    spacer_of_in(&active_theme(), token)
}

/// [`spacer_of`] with the theme passed explicitly — for views built outside a
/// build pass.
pub fn spacer_of_in(theme: &Theme, token: SpaceToken) -> Builder<FixedProps> {
    let v = theme.space_of(token);
    fixed(v, v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::tree::{BoxConstraints, CrossAlign, RenderTree};
    use silka_core::view::{reconcile, row, View};
    use silka_paint::Size;
    use silka_theme::Appearance;

    /// Tall enough that even the largest spacing token fits without the row
    /// having to squeeze it — the test is about the width.
    const BOX: Size = Size::new(300.0, 150.0);

    fn laid_out(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(BOX));
        tree
    }

    #[test]
    fn a_spacer_pushes_what_follows_to_the_far_end() {
        let tree = laid_out(
            row([
                View::from(fixed(40.0, 20.0)),
                View::from(spacer()),
                View::from(fixed(60.0, 20.0)),
            ])
            .cross(CrossAlign::Center),
        );
        let baris = tree.children(tree.root())[0];
        let anak = tree.children(baris).to_vec();
        assert_eq!(anak.len(), 3);
        let terakhir = tree.bounds(anak[2]);
        assert!(
            terakhir.max_x() >= BOX.width - 0.5,
            "the trailing child stopped at {} instead of the far edge",
            terakhir.max_x()
        );
        // …and the leading child did not move at all.
        assert_eq!(tree.bounds(anak[0]).min_x(), 0.0);
    }

    #[test]
    fn weights_split_the_leftover_space_in_proportion() {
        let tree = laid_out(
            row([
                View::from(spacer_flex(1.0)),
                View::from(fixed(60.0, 20.0)),
                View::from(spacer_flex(2.0)),
            ])
            .cross(CrossAlign::Center),
        );
        let baris = tree.children(tree.root())[0];
        let anak = tree.children(baris).to_vec();
        let kiri = tree.size(anak[0]).width;
        let kanan = tree.size(anak[2]).width;
        assert!(
            (kanan - kiri * 2.0).abs() < 1.0,
            "1:2 weights gave {kiri} and {kanan}"
        );
    }

    #[test]
    fn a_fixed_gap_is_always_a_multiple_of_the_scale_step() {
        for theme in [
            Theme::cupertino(Appearance::Light),
            Theme::tailwind(Appearance::Dark),
        ] {
            for token in SpaceToken::ALL {
                let expected = theme.space_of(token);
                let tree = laid_out(row([View::from(spacer_of_in(&theme, token))]));
                let baris = tree.children(tree.root())[0];
                let gap = tree.children(baris)[0];
                assert_eq!(tree.size(gap).width, expected, "{token:?}");
            }
        }
    }

    #[test]
    fn a_spacer_says_nothing_to_a_screen_reader() {
        use silka_core::access::AccessRole;

        let tree = laid_out(row([View::from(spacer()), View::from(fixed(40.0, 20.0))]));
        let a11y = tree.access_tree(None);
        assert!(
            a11y.entries()
                .iter()
                .all(|e| e.node.role != AccessRole::Label && e.node.label.is_none()),
            "a gap must be silent:\n{}",
            a11y.dump()
        );
    }
}
