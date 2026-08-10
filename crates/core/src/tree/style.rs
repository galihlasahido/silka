//! The layout style vocabulary for **flex/grid** containers (REKOMENDASI §3.4).
//!
//! The types in this module are **our own**. Taffy is the engine behind them,
//! but its name never leaks upwards: the mapping to `taffy::Style` lives in
//! exactly one place ([`super::taffy_box`]). The rule is identical to wgpu
//! (§3.2) and cosmic-text (§3.3) — widget code speaks the framework's
//! vocabulary, so the engine underneath can be swapped without touching a single
//! widget.
//!
//! Spacing values are locked to the **4pt scale** ([`SPACING_UNIT`]) per the
//! token discipline of §2.6/§2.7: `gap_3()` means three steps on the scale, not
//! "12 pixels that happen to look nice".

use silka_paint::Insets;

use super::primitives::Axis;

/// One step on the spacing scale, in logical points — the **fallback** unit.
///
/// The authority is `silka_theme::SpacingTokens::unit`, which the spacing
/// utilities ([`crate::view::div`] and friends) read from the ambient theme.
/// This constant is what they fall back to when no theme is installed, and it
/// is the value both first-party presets set (§2.7); a unit test below keeps
/// the two from drifting apart.
///
/// Layout code that needs a number without a theme in reach — a default gap, a
/// debug overlay — may use it directly. Application code should not: `p_4()` and
/// `gap_3()` say the same thing and follow a brand preset that changes the unit.
pub const SPACING_UNIT: f32 = 4.0;

/// The algorithm a container uses to arrange its children.
///
/// The whole public vocabulary of this module is ours, not Taffy's: `taffy::`
/// is confined to one module, exactly as wgpu is confined to one crate.
///
/// ```
/// use silka_core::tree::{Axis, ContainerStyle, LayoutMode};
///
/// assert_eq!(ContainerStyle::flex(Axis::Horizontal).mode, LayoutMode::Flex);
/// assert_eq!(ContainerStyle::grid().mode, LayoutMode::Grid);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LayoutMode {
    /// Flexbox — `row()` and `column()`.
    #[default]
    Flex,
    /// CSS Grid — `grid()`.
    Grid,
}

/// Whether children may move to a new line when they run out of room.
///
/// ```
/// use silka_core::tree::{Axis, ContainerStyle, FlexWrap};
///
/// // Not wrapping is the default — the Flutter `Row`/`Column` behaviour,
/// // not the web's.
/// assert_eq!(ContainerStyle::flex(Axis::Horizontal).wrap, FlexWrap::default());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FlexWrap {
    /// Stay on one line even when overflowing (Flutter's `Row`/`Column`
    /// behaviour).
    #[default]
    NoWrap,
    /// Move to the next line.
    Wrap,
    /// Wrap with the line order reversed.
    WrapReverse,
}

/// Alignment/space distribution along the main axis.
///
/// ```
/// use silka_core::tree::MainAlign;
///
/// // The utility vocabulary spells these out: `justify_between()` and
/// // friends set exactly this field.
/// assert_eq!(MainAlign::default(), MainAlign::Start);
/// assert_ne!(MainAlign::SpaceBetween, MainAlign::SpaceEvenly);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MainAlign {
    /// Packed at the start of the axis.
    #[default]
    Start,
    /// Centred.
    Center,
    /// Packed at the end of the axis.
    End,
    /// The leftover space is split between the children; the first and last
    /// touch the edges.
    SpaceBetween,
    /// The leftover space is split evenly, including half a gap at each edge.
    SpaceAround,
    /// The leftover space is split perfectly evenly, edges included.
    SpaceEvenly,
}

/// Alignment along the cross axis.
///
/// `Start` means *left in LTR and right in RTL* — direction-relative, not
/// physical, which is what makes an Arabic UI mirror without a widget changing.
///
/// ```
/// use silka_core::tree::CrossAlign;
///
/// assert_eq!(CrossAlign::default(), CrossAlign::Start);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CrossAlign {
    /// Packed at the start of the cross axis (left in LTR, right in RTL).
    #[default]
    Start,
    /// Centred.
    Center,
    /// Packed at the end of the cross axis.
    End,
    /// Forced to the container's width/height.
    Stretch,
    /// The children's text baselines are aligned.
    Baseline,
}

/// The cell-filling order for grid items that are not placed explicitly.
///
/// ```
/// use silka_core::tree::GridFlow;
///
/// // Rows first, which is what a form or a card grid wants.
/// assert_eq!(GridFlow::default(), GridFlow::Row);
/// ```
///
/// The `Dense` variants backfill holes left by explicitly placed items, at the
/// cost of items no longer appearing in source order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GridFlow {
    /// Fill rows first, moving right.
    #[default]
    Row,
    /// Fill columns first, moving down.
    Column,
    /// Like [`GridFlow::Row`], but holes left behind get filled in too.
    RowDense,
    /// Like [`GridFlow::Column`], but holes left behind get filled in too.
    ColumnDense,
}

/// The lower size bound of a grid track.
///
/// Half of a CSS `minmax()`; see [`Track`] for the constructors that cover the
/// common cases without naming either half.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrackMin {
    /// As small as the content can possibly be.
    Auto,
    /// A fixed size (logical points).
    Fixed(f32),
    /// A percentage of the container (`0.0..=1.0`).
    Percent(f32),
    /// The content's min-content size.
    MinContent,
    /// The content's max-content size.
    MaxContent,
}

/// The upper size bound of a grid track.
///
/// [`TrackMax::Fraction`] is the CSS `fr` unit: a share of whatever space is
/// left after the fixed tracks have taken theirs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrackMax {
    /// As large as the content.
    Auto,
    /// A fixed size (logical points).
    Fixed(f32),
    /// A percentage of the container (`0.0..=1.0`).
    Percent(f32),
    /// The content's min-content size.
    MinContent,
    /// The content's max-content size.
    MaxContent,
    /// A share of the leftover space (the CSS `fr` unit).
    Fraction(f32),
}

/// The size of one grid track (a row or a column).
///
/// Its shape is always `minmax(min, max)` as in CSS; short constructors are
/// provided for the common cases.
///
/// ```
/// use silka_core::tree::{repeat, Track, TrackMax, TrackMin};
///
/// // A sidebar of fixed width beside content that takes the rest.
/// let columns = [Track::fixed(240.0), Track::fr(1.0)];
/// assert_eq!(columns[0].min, TrackMin::Fixed(240.0));
/// assert_eq!(columns[1].max, TrackMax::Fraction(1.0));
///
/// // The general form is always available, and `repeat` covers the rest.
/// let responsive = Track::minmax(TrackMin::Fixed(160.0), TrackMax::Fraction(1.0));
/// assert_eq!(responsive.max, TrackMax::Fraction(1.0));
/// assert_eq!(repeat(3, Track::fr(1.0)).len(), 3);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Track {
    /// The lower bound.
    pub min: TrackMin,
    /// The upper bound.
    pub max: TrackMax,
}

impl Default for Track {
    fn default() -> Self {
        Track::AUTO
    }
}

impl Track {
    /// As large as its content.
    pub const AUTO: Track = Track {
        min: TrackMin::Auto,
        max: TrackMax::Auto,
    };

    /// A fixed width/height.
    pub const fn fixed(v: f32) -> Track {
        Track {
            min: TrackMin::Fixed(v),
            max: TrackMax::Fixed(v),
        }
    }

    /// A percentage of the container (`0.0..=1.0`).
    pub const fn percent(v: f32) -> Track {
        Track {
            min: TrackMin::Percent(v),
            max: TrackMax::Percent(v),
        }
    }

    /// A share of the leftover space — `fr(1.0)` is CSS `minmax(auto, 1fr)`.
    pub const fn fr(v: f32) -> Track {
        Track {
            min: TrackMin::Auto,
            max: TrackMax::Fraction(v),
        }
    }

    /// As small as possible without clipping the content.
    pub const fn min_content() -> Track {
        Track {
            min: TrackMin::MinContent,
            max: TrackMax::MinContent,
        }
    }

    /// As wide as the content with no line breaking.
    pub const fn max_content() -> Track {
        Track {
            min: TrackMin::MaxContent,
            max: TrackMax::MaxContent,
        }
    }

    /// The general `minmax(min, max)` form.
    pub const fn minmax(min: TrackMin, max: TrackMax) -> Track {
        Track { min, max }
    }
}

/// `count` identical tracks — the equivalent of CSS `repeat(count, track)`.
///
/// It deliberately returns a plain `Vec` rather than a dedicated type: `repeat()`
/// here is only sugar, and the resulting grid stays explicit.
pub fn repeat(count: usize, track: Track) -> Vec<Track> {
    vec![track; count]
}

/// One placement edge of a grid item.
///
/// ```
/// use silka_core::tree::{GridLine, GridSpan};
///
/// // Automatic placement follows the flow; explicit placement does not.
/// assert_eq!(GridSpan::AUTO.start, GridLine::Auto);
/// assert_eq!(GridSpan::line(2).start, GridLine::Line(2));
/// assert_eq!(GridSpan::span(3).end, GridLine::Span(3));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GridLine {
    /// Placed automatically, following [`GridFlow`].
    #[default]
    Auto,
    /// Line number `n` (1 = the first line; negatives count from the end).
    Line(i16),
    /// Spans `n` tracks from the opposite edge.
    Span(u16),
}

/// An item's placement along one grid axis (row or column).
///
/// ```
/// use silka_core::tree::{GridLine, GridSpan};
///
/// // "From line 1 to line 3" — a header spanning two columns.
/// let header = GridSpan::between(1, 3);
/// assert_eq!(header.start, GridLine::Line(1));
/// assert_eq!(header.end, GridLine::Line(3));
///
/// // Negative line numbers count from the end, as in CSS.
/// assert_eq!(GridSpan::between(1, -1).end, GridLine::Line(-1));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GridSpan {
    /// The start edge.
    pub start: GridLine,
    /// The end edge.
    pub end: GridLine,
}

impl GridSpan {
    /// Automatic placement.
    pub const AUTO: GridSpan = GridSpan {
        start: GridLine::Auto,
        end: GridLine::Auto,
    };

    /// Start at line `n`, one track wide.
    pub const fn line(n: i16) -> GridSpan {
        GridSpan {
            start: GridLine::Line(n),
            end: GridLine::Auto,
        }
    }

    /// Span `n` tracks from its automatic position.
    pub const fn span(n: u16) -> GridSpan {
        GridSpan {
            start: GridLine::Auto,
            end: GridLine::Span(n),
        }
    }

    /// From line `start` through to line `end`.
    pub const fn between(start: i16, end: i16) -> GridSpan {
        GridSpan {
            start: GridLine::Line(start),
            end: GridLine::Line(end),
        }
    }
}

/// The style of a flex/grid **container**.
///
/// Held by [`super::TaffyBox`]; the view layer copies it across verbatim from
/// the Dart-flavoured method chain (`row()`/`column()`/`grid()`).
///
/// ```
/// use silka_core::tree::{Axis, ContainerStyle, CrossAlign, MainAlign, SPACING_UNIT};
///
/// let mut style = ContainerStyle::flex(Axis::Horizontal);
/// style.main = MainAlign::SpaceBetween;
/// style.cross = CrossAlign::Center;
///
/// // Gaps are locked to the 4pt scale rather than to free-floating numbers.
/// // "Spacing" means the gap along the main axis — here, horizontally.
/// style.set_spacing(3.0 * SPACING_UNIT);
/// assert_eq!(style.gap_x, 12.0);
/// assert_eq!(style.gap_y, 0.0);
///
/// // A grid has no single main axis, so spacing sets both — which is what an
/// // application author means by the word.
/// let mut cards = ContainerStyle::grid();
/// cards.set_spacing(4.0 * SPACING_UNIT);
/// assert_eq!((cards.gap_x, cards.gap_y), (16.0, 16.0));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerStyle {
    /// Flexbox or Grid.
    pub mode: LayoutMode,
    /// The main axis (meaningful only for [`LayoutMode::Flex`]).
    pub axis: Axis,
    /// Reverse the main-axis order.
    pub reverse: bool,
    /// Wrap onto a new line when space runs out.
    pub wrap: FlexWrap,
    /// Space distribution along the main axis (flex) / the inline axis (grid).
    pub main: MainAlign,
    /// Child alignment along the cross axis.
    pub cross: CrossAlign,
    /// Space distribution between wrapped lines (flex) or between tracks along
    /// the block axis (grid). `None` = let the engine use its default (stretch).
    pub lines: Option<MainAlign>,
    /// The gap between children along the horizontal axis.
    pub gap_x: f32,
    /// The gap between children along the vertical axis.
    pub gap_y: f32,
    /// Space inside the container's edges.
    pub padding: Insets,
    /// Explicit row sizes (grid).
    pub rows: Vec<Track>,
    /// Explicit column sizes (grid).
    pub columns: Vec<Track>,
    /// The cell-filling order for items with no explicit placement.
    pub auto_flow: GridFlow,
}

impl Default for ContainerStyle {
    fn default() -> Self {
        ContainerStyle::flex(Axis::Vertical)
    }
}

impl ContainerStyle {
    /// A flex container along `axis`.
    pub fn flex(axis: Axis) -> Self {
        Self {
            mode: LayoutMode::Flex,
            axis,
            reverse: false,
            wrap: FlexWrap::NoWrap,
            main: MainAlign::Start,
            cross: CrossAlign::Start,
            lines: None,
            gap_x: 0.0,
            gap_y: 0.0,
            padding: Insets::ZERO,
            rows: Vec::new(),
            columns: Vec::new(),
            auto_flow: GridFlow::Row,
        }
    }

    /// A grid container.
    ///
    /// Its default is `cross = Stretch` — grid cells fill completely, as in CSS.
    pub fn grid() -> Self {
        Self {
            mode: LayoutMode::Grid,
            cross: CrossAlign::Stretch,
            ..Self::flex(Axis::Vertical)
        }
    }

    /// The gap between children **along the main axis**.
    ///
    /// For a grid (which has no single main axis) this sets both axes at once —
    /// that is what an application author expects "spacing" to mean.
    pub fn set_spacing(&mut self, v: f32) {
        match (self.mode, self.axis) {
            (LayoutMode::Grid, _) => {
                self.gap_x = v;
                self.gap_y = v;
            }
            (LayoutMode::Flex, Axis::Vertical) => self.gap_y = v,
            (LayoutMode::Flex, Axis::Horizontal) => self.gap_x = v,
        }
    }
}

/// The style of an **item** inside a flex/grid container.
///
/// The equivalent of Flutter's `ParentData` (`Expanded`/`Flexible`): the data
/// belongs to the child, but the parent is what reads it. Carried by
/// [`super::LayoutItem`] and picked up by the parent through
/// [`super::LayoutCtx::child_layout_style`].
///
/// ```
/// use silka_core::tree::ItemStyle;
///
/// // The default child does not grow and does not shrink — Flutter's
/// // behaviour, not the web's.
/// let plain = ItemStyle::DEFAULT;
/// assert_eq!(plain.grow, 0.0);
///
/// // `expanded()` is grow = 1 with a zero basis; `flexible()` keeps the
/// // content's natural size as its basis.
/// let expanded = ItemStyle { grow: 1.0, basis: Some(0.0), ..ItemStyle::DEFAULT };
/// assert!(expanded.grow > plain.grow);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemStyle {
    /// The share of leftover space requested (0 = does not grow).
    pub grow: f32,
    /// The willingness to shrink when space runs short (0 = never shrinks).
    pub shrink: f32,
    /// The initial size along the main axis; `None` = the content's natural
    /// size.
    pub basis: Option<f32>,
    /// A cross-axis alignment just for this item; `None` = follow the container.
    pub align_self: Option<CrossAlign>,
    /// Space outside the item's edges.
    pub margin: Insets,
    /// Placement along the grid's row axis.
    pub row: GridSpan,
    /// Placement along the grid's column axis.
    pub column: GridSpan,
}

impl ItemStyle {
    /// An ordinary item: does not grow, **does not shrink**, sized to its
    /// content.
    ///
    /// `shrink = 0` deliberately differs from CSS (which uses 1). The reason is
    /// the Flutter feel: a child of a `Row` keeps its natural size and overflows
    /// when it does not fit, rather than quietly collapsing until it is
    /// unreadable. Anyone who wants CSS behaviour just calls `.shrink(1.0)`.
    pub const DEFAULT: ItemStyle = ItemStyle {
        grow: 0.0,
        shrink: 0.0,
        basis: None,
        align_self: None,
        margin: Insets::ZERO,
        row: GridSpan::AUTO,
        column: GridSpan::AUTO,
    };
}

impl Default for ItemStyle {
    fn default() -> Self {
        ItemStyle::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_masuk_ke_sumbu_utama_saja() {
        let mut kolom = ContainerStyle::flex(Axis::Vertical);
        kolom.set_spacing(12.0);
        assert_eq!((kolom.gap_x, kolom.gap_y), (0.0, 12.0));

        let mut baris = ContainerStyle::flex(Axis::Horizontal);
        baris.set_spacing(12.0);
        assert_eq!((baris.gap_x, baris.gap_y), (12.0, 0.0));
    }

    #[test]
    fn spacing_grid_mengisi_kedua_sumbu() {
        let mut g = ContainerStyle::grid();
        g.set_spacing(8.0);
        assert_eq!((g.gap_x, g.gap_y), (8.0, 8.0));
        assert_eq!(g.cross, CrossAlign::Stretch, "sel grid mengisi penuh");
    }

    #[test]
    fn item_bawaan_tidak_tumbuh_dan_tidak_menyusut() {
        let s = ItemStyle::DEFAULT;
        assert_eq!(s.grow, 0.0);
        assert_eq!(s.shrink, 0.0, "rasa Flutter: anak Row tidak mengempis");
        assert!(s.basis.is_none());
    }

    #[test]
    fn repeat_menghasilkan_track_identik() {
        let t = repeat(3, Track::fr(1.0));
        assert_eq!(t.len(), 3);
        assert!(t.iter().all(|x| *x == Track::fr(1.0)));
    }

    #[test]
    fn track_fr_adalah_minmax_auto_fr() {
        let t = Track::fr(2.0);
        assert_eq!(t.min, TrackMin::Auto);
        assert_eq!(t.max, TrackMax::Fraction(2.0));
    }

    #[test]
    fn grid_span_punya_bentuk_pendek() {
        assert_eq!(GridSpan::span(2).end, GridLine::Span(2));
        assert_eq!(GridSpan::line(3).start, GridLine::Line(3));
        assert_eq!(
            GridSpan::between(1, 3),
            GridSpan {
                start: GridLine::Line(1),
                end: GridLine::Line(3)
            }
        );
    }

    #[test]
    fn skala_spacing_empat_poin() {
        assert_eq!(SPACING_UNIT, 4.0);
        assert_eq!(SPACING_UNIT * 3.0, 12.0);
    }

    #[test]
    fn fallback_sepakat_dengan_unit_setiap_preset() {
        // The constant is only a fallback; if a preset ever moved off 4pt, a
        // view built without a theme would silently disagree with one built
        // inside `with_theme`.
        for preset in silka_theme::Preset::ALL {
            let t = silka_theme::Theme::new(preset, silka_theme::Appearance::Light);
            assert_eq!(t.spacing.unit, SPACING_UNIT, "{preset:?}");
        }
    }
}
