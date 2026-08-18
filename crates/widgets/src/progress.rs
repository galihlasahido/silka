//! `progress_bar()` and `progress_circle()` — the two shapes of "this is
//! taking a while" (`KOMPONEN.md` Tier 4).
//!
//! ```
//! use silka_widgets::{progress_bar, progress_circle};
//!
//! // Determinate: the fraction is known, so say it.
//! let import = progress_bar(0.42).label("Importing invoices");
//!
//! // Indeterminate: the fraction is *not* known, and pretending otherwise
//! // ("stuck at 99%") is the oldest lie in software.
//! let waiting = progress_circle(0.0).indeterminate().label("Connecting");
//! # let _ = (import, waiting);
//! ```
//!
//! These are the first users of
//! [`AccessRole::ProgressIndicator`](silka_core::access::AccessRole::ProgressIndicator),
//! which has been in the vocabulary since the accessibility layer was written
//! and had no widget behind it.
//!
//! ## Determinate and indeterminate are one node, not two
//!
//! A progress indicator flips between the two states in real life — a download
//! spins while the server thinks, then fills once `Content-Length` arrives —
//! and a component that made them two widgets would force the application to
//! swap one for the other mid-transfer, losing whatever the first one had
//! animated to. Here it is [`ProgressBar::indeterminate`], one field, and the
//! spring survives the switch.
//!
//! ## What moves, and what reduced motion does to it
//!
//! | Motion | Role | Under reduced motion |
//! |---|---|---|
//! | The determinate fill catching up to a new value | essential — it *is* the information | keeps moving, loses its bounce |
//! | The indeterminate sweep | decorative — it carries no progress information at all | **stops**, and the indicator settles into a static partial band |
//!
//! That second row is not a shortcut. An indeterminate indicator is an
//! animation that loops forever, which is precisely the class of motion the OS
//! setting exists to switch off; freezing it while keeping the shape is what
//! lets the widget still say "busy" without moving.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | track, fill, thickness, diameter and corner geometry are all tokens |
//! | Interactive states on a spring | the value is a [`SpringValue`], retargeted rather than restarted |
//! | Keyboard + focus ring | a progress indicator is **not** a control: it takes no input and is not a tab stop |
//! | AccessKit node | [`AccessRole::ProgressIndicator`] with the percentage as its value, or no value at all when the fraction is unknown |
//! | Dark mode | token-driven |
//! | Hit target ≥ 44pt | not applicable |
//! | Reduced motion | see the table above |

use std::time::Duration;

use silka_core::access::{AccessNode, AccessRole};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{
    BoxConstraints, LayoutCtx, NodeId, PaintCtx, RenderNode, RenderTree, TextDirection,
};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, Corners, LineCap, Point, Quad, Rect, Size, Stroke};
use silka_theme::{ColorToken, RadiusToken, Theme};

/// How long one sweep of an indeterminate indicator takes.
///
/// 1.4 s is the interval both `NSProgressIndicator` and Material land on: slow
/// enough not to read as a flicker, fast enough that the widget is obviously
/// alive.
pub const INDETERMINATE_PERIOD: Duration = Duration::from_millis(1400);

/// The travelling band's length, as a fraction of the track.
pub const INDETERMINATE_BAND: f32 = 0.35;

/// The rotating arc's length, in turns.
pub const INDETERMINATE_SWEEP: f32 = 0.25;

/// Bar thickness, in **spacing steps** (§2.6) — 1 × 4pt.
pub const BAR_THICKNESS_STEPS: f32 = 1.0;

/// Ring diameter, in **spacing steps** — 6 × 4pt = 24pt.
pub const CIRCLE_DIAMETER_STEPS: f32 = 6.0;

/// Ring thickness, in **spacing steps** — 0.5 × 4pt = 2pt.
pub const CIRCLE_THICKNESS_STEPS: f32 = 0.5;

/// Segments per full turn when a ring is flattened into a polyline.
///
/// The stroke command takes a polyline (§3.2), so a circle is a many-sided
/// polygon; 64 per turn is below a tenth of a point of error at the sizes a
/// progress ring is actually drawn at.
pub const ARC_SEGMENTS: usize = 64;

// ---------------------------------------------------------------------------
// Pure geometry
// ---------------------------------------------------------------------------

/// The travelling band of an indeterminate bar: `(start, width)` along a track
/// of `length`.
///
/// A pure function, so "does the band leave the track cleanly at both ends?" is
/// a unit test rather than a video recording. The band enters from before the
/// start and leaves past the end, and both ends are clipped to the track —
/// which is what makes it grow out of one edge and shrink into the other
/// instead of appearing whole.
///
/// ```
/// use silka_widgets::progress::indeterminate_span;
///
/// // Just entering: clipped at the left, so it is still short.
/// let (start, width) = indeterminate_span(0.0, 200.0, 70.0);
/// assert_eq!(start, 0.0);
/// assert_eq!(width, 0.0);
///
/// // Mid-track: the whole band is visible.
/// let (_, mid) = indeterminate_span(0.5, 200.0, 70.0);
/// assert_eq!(mid, 70.0);
///
/// // Leaving: clipped at the right, so it is short again.
/// let (start, width) = indeterminate_span(0.9, 200.0, 70.0);
/// assert!((start + width - 200.0).abs() < 1e-3);
/// assert!(width < 70.0);
/// ```
pub fn indeterminate_span(phase: f32, length: f32, band: f32) -> (f32, f32) {
    let length = length.max(0.0);
    let band = band.clamp(0.0, length);
    if length <= 0.0 {
        return (0.0, 0.0);
    }
    let p = phase.rem_euclid(1.0);
    // The band's leading edge travels from `-band` to `length`.
    let head = p * (length + band) - band;
    let start = head.max(0.0);
    let end = (head + band).min(length);
    (start, (end - start).max(0.0))
}

/// A circular arc as a polyline, ready for [`Stroke`].
///
/// Angles are in **turns** (1.0 = a full circle) measured clockwise from
/// twelve o'clock, because that is how a progress ring is described — "a
/// quarter filled", not "π/2 radians from the positive x-axis".
///
/// ```
/// use silka_paint::Point;
/// use silka_widgets::progress::arc_points;
///
/// let pts = arc_points(Point::new(50.0, 50.0), 20.0, 0.0, 0.25, 8);
/// // Starts at twelve o'clock…
/// assert!((pts[0].x - 50.0).abs() < 0.01 && (pts[0].y - 30.0).abs() < 0.01);
/// // …and a quarter turn clockwise is three o'clock.
/// let last = pts[pts.len() - 1];
/// assert!((last.x - 70.0).abs() < 0.01 && (last.y - 50.0).abs() < 0.01);
/// ```
pub fn arc_points(
    center: Point,
    radius: f32,
    start_turns: f32,
    sweep_turns: f32,
    segments: usize,
) -> Vec<Point> {
    let radius = radius.max(0.0);
    let segments = segments.max(1);
    let mut out = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let t = start_turns + sweep_turns * (i as f32 / segments as f32);
        // Twelve o'clock is -y, and increasing turns go clockwise.
        let a = t * std::f32::consts::TAU;
        out.push(Point::new(
            center.x + radius * a.sin(),
            center.y - radius * a.cos(),
        ));
    }
    out
}

/// How many polyline segments an arc of `sweep_turns` deserves.
pub fn arc_segments(sweep_turns: f32) -> usize {
    ((sweep_turns.abs() * ARC_SEGMENTS as f32).ceil() as usize).clamp(2, ARC_SEGMENTS)
}

/// A fraction as the percentage a screen reader announces.
///
/// ```
/// use silka_widgets::progress::percent_text;
///
/// assert_eq!(percent_text(0.0), "0%");
/// assert_eq!(percent_text(0.425), "43%");
/// assert_eq!(percent_text(1.0), "100%");
/// // Out-of-range values are clamped rather than announced as nonsense.
/// assert_eq!(percent_text(2.5), "100%");
/// ```
pub fn percent_text(value: f32) -> String {
    let v = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    format!("{}%", (v * 100.0).round() as i32)
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every drawing value of a progress indicator, already resolved from tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProgressStyle {
    /// The unfilled part of the track.
    pub track: Color,
    /// The filled part.
    pub fill: Color,
    /// Bar height, or ring thickness.
    pub thickness: f32,
    /// The bar's corner geometry — a full radius makes it a capsule.
    pub corners: Corners,
    /// The ring's outer diameter (ignored by the bar).
    pub diameter: f32,
    /// The bar's natural length when nothing constrains it (ignored by the
    /// ring).
    pub length: f32,
}

impl ProgressStyle {
    /// The bar's style for a theme.
    pub fn bar(theme: &Theme) -> Self {
        Self {
            track: theme.color_of(ColorToken::SurfaceSunken),
            fill: theme.color_of(ColorToken::Accent),
            thickness: theme.space(BAR_THICKNESS_STEPS),
            corners: theme.corners_of(RadiusToken::Full),
            diameter: theme.space(CIRCLE_DIAMETER_STEPS),
            length: theme.space(40.0),
        }
    }

    /// The ring's style for a theme.
    pub fn circle(theme: &Theme) -> Self {
        Self {
            thickness: theme.space(CIRCLE_THICKNESS_STEPS),
            ..Self::bar(theme)
        }
    }
}

// ---------------------------------------------------------------------------
// Shared motion
// ---------------------------------------------------------------------------

/// The animated part both indicators share: a value spring plus the
/// indeterminate phase.
///
/// One struct rather than two copies, because "retarget, never restart" and
/// "freeze the sweep under reduced motion" are decisions that must not be able
/// to differ between the bar and the ring.
#[derive(Debug, Clone)]
struct Gerak {
    value: SpringValue<f32>,
    /// `None` = indeterminate.
    target: Option<f32>,
    phase: f32,
    /// True while the sweep is allowed to run (reduced motion clears it).
    sweeping: bool,
}

impl Gerak {
    fn new(target: Option<f32>, spring: Spring) -> Self {
        let awal = target.unwrap_or(0.0);
        Self {
            value: SpringValue::new(awal).with_spring(spring),
            target,
            phase: 0.0,
            sweeping: target.is_none(),
        }
    }

    fn set_target(&mut self, target: Option<f32>) -> bool {
        if self.target == target {
            return false;
        }
        self.target = target;
        if let Some(v) = target {
            // A retarget, not a new animation: a value corrected twice in a row
            // keeps its velocity (§3.5).
            self.value.set_target(v.clamp(0.0, 1.0));
        }
        true
    }

    fn advance(&mut self, tick: &Tick) -> bool {
        let mut bergeser = false;
        if self.target.is_some() {
            let sebelum = self.value.position();
            tick.advance(&mut self.value);
            bergeser |= self.value.position() != sebelum;
        }
        // The sweep is decorative — it says "busy", never "how far" — so
        // reduced motion switches it off entirely rather than merely calming
        // it. The band stays where it froze, which still reads as busy.
        self.sweeping = self.target.is_none() && !tick.motion().suppresses(MotionRole::Decorative);
        if self.sweeping {
            let dt = tick.dt().as_secs_f32();
            let period = INDETERMINATE_PERIOD.as_secs_f32().max(f32::EPSILON);
            self.phase = (self.phase + dt / period).rem_euclid(1.0);
            // Keep the frame loop alive: an indeterminate indicator is the one
            // thing in this framework that legitimately never settles.
            tick.keep_awake();
            bergeser = true;
        }
        bergeser
    }

    fn is_animating(&self) -> bool {
        self.value.is_animating() || self.sweeping
    }

    fn settle(&mut self) {
        self.value.settle();
    }

    fn position(&self) -> f32 {
        self.value.position().clamp(0.0, 1.0)
    }
}

// ---------------------------------------------------------------------------
// Bar node
// ---------------------------------------------------------------------------

/// The horizontal progress bar.
pub struct ProgressBarBox {
    /// Every resolved drawing value.
    pub style: ProgressStyle,
    /// The name a screen reader announces.
    pub label: Option<String>,
    gerak: Gerak,
}

impl ProgressBarBox {
    /// The fraction currently drawn (0..1); meaningless while indeterminate.
    pub fn progress(&self) -> f32 {
        self.gerak.position()
    }

    /// The fraction the application asked for, or `None` when indeterminate.
    pub fn target(&self) -> Option<f32> {
        self.gerak.target
    }

    /// True when the fraction is unknown.
    pub fn is_indeterminate(&self) -> bool {
        self.gerak.target.is_none()
    }

    /// The sweep's phase (0..1).
    pub fn phase(&self) -> f32 {
        self.gerak.phase
    }

    /// True while anything is still moving.
    pub fn is_animating(&self) -> bool {
        self.gerak.is_animating()
    }

    /// Advance by one frame; true when something moved.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        self.gerak.advance(tick)
    }

    /// Finish the value transition instantly (tests and snapshots).
    pub fn settle(&mut self) {
        self.gerak.settle();
    }

    /// The filled rect inside a track of `size`.
    ///
    /// Reading-relative: a determinate bar fills from the start of the line, so
    /// it grows leftwards in an RTL document (§9.8).
    pub fn fill_rect(&self, size: Size, direction: TextDirection) -> Rect {
        let (start, width) = match self.gerak.target {
            Some(_) => (0.0, size.width * self.gerak.position()),
            None => indeterminate_span(
                self.gerak.phase,
                size.width,
                size.width * INDETERMINATE_BAND,
            ),
        };
        let x = if direction.is_rtl() {
            size.width - start - width
        } else {
            start
        };
        Rect::new(x, 0.0, width.max(0.0), size.height)
    }
}

impl RenderNode for ProgressBarBox {
    fn type_name(&self) -> &'static str {
        "ProgressBar"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // As long as it is allowed to be — a progress bar is a horizontal rule
        // with a fill, and one that shrank to its content would be invisible.
        let width = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            self.style.length
        };
        constraints.constrain(Size::new(width, self.style.thickness))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let corners = self.style.corners.clamp_to(bounds.size);
        if self.style.track.a > 0.0 {
            ctx.quad(
                Quad::new(bounds)
                    .background(self.style.track)
                    .corners(corners),
            );
        }
        let isi = self.fill_rect(bounds.size, ctx.direction());
        if isi.size.width > 0.0 && self.style.fill.a > 0.0 {
            ctx.quad(
                Quad::new(isi)
                    .background(self.style.fill)
                    .corners(self.style.corners.clamp_to(isi.size)),
            );
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::ProgressIndicator;
        node.label.clone_from(&self.label);
        // No value at all while the fraction is unknown: `AccessKit` reads a
        // missing value as "busy", whereas a made-up 0% would be announced as
        // real progress that never moves.
        node.value = self.gerak.target.map(percent_text);
    }
}

impl core::fmt::Debug for ProgressBarBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProgressBarBox")
            .field("target", &self.gerak.target)
            .field("position", &self.gerak.position())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Circle node
// ---------------------------------------------------------------------------

/// The circular progress ring.
pub struct ProgressCircleBox {
    /// Every resolved drawing value.
    pub style: ProgressStyle,
    /// The name a screen reader announces.
    pub label: Option<String>,
    gerak: Gerak,
}

impl ProgressCircleBox {
    /// The fraction currently drawn (0..1).
    pub fn progress(&self) -> f32 {
        self.gerak.position()
    }

    /// The fraction the application asked for, or `None` when indeterminate.
    pub fn target(&self) -> Option<f32> {
        self.gerak.target
    }

    /// True when the fraction is unknown.
    pub fn is_indeterminate(&self) -> bool {
        self.gerak.target.is_none()
    }

    /// The sweep's phase (0..1).
    pub fn phase(&self) -> f32 {
        self.gerak.phase
    }

    /// True while anything is still moving.
    pub fn is_animating(&self) -> bool {
        self.gerak.is_animating()
    }

    /// Advance by one frame; true when something moved.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        self.gerak.advance(tick)
    }

    /// Finish the value transition instantly (tests and snapshots).
    pub fn settle(&mut self) {
        self.gerak.settle();
    }

    /// The arc actually drawn: `(start_turns, sweep_turns)`.
    ///
    /// A determinate ring starts at twelve o'clock and grows clockwise; an
    /// indeterminate one is a fixed arc that travels round.
    pub fn arc(&self) -> (f32, f32) {
        match self.gerak.target {
            Some(_) => (0.0, self.gerak.position()),
            None => (self.gerak.phase, INDETERMINATE_SWEEP),
        }
    }

    /// The ring's centre and radius inside a box of `size`.
    fn ring(&self, size: Size) -> (Point, f32) {
        let d = size.min_side();
        let r = (d - self.style.thickness).max(0.0) * 0.5;
        (Point::new(size.width * 0.5, size.height * 0.5), r)
    }
}

impl RenderNode for ProgressCircleBox {
    fn type_name(&self) -> &'static str {
        "ProgressCircle"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let d = self.style.diameter;
        constraints.constrain(Size::new(d, d))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let size = ctx.size();
        let (center, radius) = self.ring(size);
        let tebal = self.style.thickness;
        if radius <= 0.0 || tebal <= 0.0 {
            return;
        }
        if self.style.track.a > 0.0 {
            let mut jalur = arc_points(center, radius, 0.0, 1.0, ARC_SEGMENTS);
            // A closed ring joins its own ends, so the duplicated final point a
            // full turn produces would be a zero-length segment.
            jalur.pop();
            let mut goresan = Stroke::with_capacity(self.style.track, tebal, jalur.len())
                .closed(true)
                .cap(LineCap::Butt);
            goresan.extend(jalur);
            ctx.stroke(goresan);
        }
        let (start, sweep) = self.arc();
        if sweep > 0.0 && self.style.fill.a > 0.0 {
            let jalur = arc_points(center, radius, start, sweep, arc_segments(sweep));
            let mut goresan = Stroke::with_capacity(self.style.fill, tebal, jalur.len())
                // Round caps: the end of a progress arc is the tip of a pen,
                // not the corner of a surface (the same rule as the checkbox
                // tick).
                .cap(LineCap::Round);
            goresan.extend(jalur);
            ctx.stroke(goresan);
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::ProgressIndicator;
        node.label.clone_from(&self.label);
        node.value = self.gerak.target.map(percent_text);
    }
}

impl core::fmt::Debug for ProgressCircleBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProgressCircleBox")
            .field("target", &self.gerak.target)
            .field("position", &self.gerak.position())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

/// The props shared by both indicators.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressProps {
    style: ProgressStyle,
    value: Option<f32>,
    label: Option<String>,
    spring: Spring,
}

impl ProgressProps {
    fn terapkan(
        &self,
        gerak: &mut Gerak,
        label: &mut Option<String>,
        style: &mut ProgressStyle,
    ) -> Dirty {
        let mut dirty = Dirty::NONE;
        if gerak.set_target(self.value) {
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if *label != self.label {
            label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if style.thickness != self.style.thickness
            || style.diameter != self.style.diameter
            || style.length != self.style.length
        {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if *style != self.style {
            dirty |= Dirty::PAINT;
        }
        *style = self.style;
        if gerak.value.spring() != self.spring {
            gerak.value.set_spring(self.spring);
        }
        dirty
    }
}

/// The [`ProgressBarBox`] props.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressBarProps(ProgressProps);

impl ViewNode for ProgressBarProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ProgressBarBox {
            style: self.0.style,
            label: self.0.label.clone(),
            gerak: Gerak::new(self.0.value, self.0.spring),
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ProgressBarBox>()
            .expect("the same view type means the same render node type");
        self.0.terapkan(&mut n.gerak, &mut n.label, &mut n.style)
    }
}

/// The [`ProgressCircleBox`] props.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressCircleProps(ProgressProps);

impl ViewNode for ProgressCircleProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ProgressCircleBox {
            style: self.0.style,
            label: self.0.label.clone(),
            gerak: Gerak::new(self.0.value, self.0.spring),
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ProgressCircleBox>()
            .expect("the same view type means the same render node type");
        self.0.terapkan(&mut n.gerak, &mut n.label, &mut n.style)
    }
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// A determinate progress bar filled to `value` (0..1).
///
/// Use [`progress_bar_in`] outside a build pass.
pub fn progress_bar(value: f32) -> ProgressBar {
    progress_bar_in(&crate::ambient::active_theme(), value)
}

/// [`progress_bar`] with the theme passed explicitly.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::progress_bar_in;
///
/// let theme = Theme::cupertino(Appearance::Dark);
/// let b = progress_bar_in(&theme, 0.42).label("Importing");
/// assert_eq!(b.value_of(), Some(0.42));
/// assert_eq!(b.announced_value().as_deref(), Some("42%"));
///
/// // The same widget, with the fraction unknown.
/// let busy = progress_bar_in(&theme, 0.0).indeterminate();
/// assert_eq!(busy.value_of(), None);
/// assert_eq!(busy.announced_value(), None);
/// ```
pub fn progress_bar_in(theme: &Theme, value: f32) -> ProgressBar {
    ProgressBar {
        key: None,
        props: ProgressProps {
            style: ProgressStyle::bar(theme),
            value: Some(sane(value)),
            label: None,
            spring: Spring::smooth(),
        },
        theme: *theme,
    }
}

/// A determinate progress ring filled to `value` (0..1).
///
/// Use [`progress_circle_in`] outside a build pass.
pub fn progress_circle(value: f32) -> ProgressCircle {
    progress_circle_in(&crate::ambient::active_theme(), value)
}

/// [`progress_circle`] with the theme passed explicitly.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::progress_circle_in;
///
/// let theme = Theme::cupertino(Appearance::Dark);
/// let spinner = progress_circle_in(&theme, 0.0).indeterminate().label("Connecting");
/// assert!(spinner.is_indeterminate());
/// ```
pub fn progress_circle_in(theme: &Theme, value: f32) -> ProgressCircle {
    ProgressCircle {
        key: None,
        props: ProgressProps {
            style: ProgressStyle::circle(theme),
            value: Some(sane(value)),
            label: None,
            spring: Spring::smooth(),
        },
        theme: *theme,
    }
}

fn sane(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

macro_rules! progress_builder {
    ($nama:ident, $props:ident) => {
        impl $nama {
            /// Identity key among its siblings (§2.5).
            pub fn key(mut self, key: impl Into<silka_core::signals::Key>) -> Self {
                self.key = Some(key.into());
                self
            }

            /// The fraction complete (0..1).
            pub fn value(mut self, value: f32) -> Self {
                self.props.value = Some(sane(value));
                self
            }

            /// The fraction is **unknown**.
            ///
            /// Not the same as zero: a screen reader is told there is no value
            /// at all, which it announces as busy rather than as progress that
            /// never moves.
            pub fn indeterminate(mut self) -> Self {
                self.props.value = None;
                self
            }

            /// The name a screen reader announces.
            pub fn label(mut self, label: impl Into<String>) -> Self {
                self.props.label = Some(label.into());
                self
            }

            /// The track colour, named by its role.
            pub fn track(mut self, token: silka_theme::ColorToken) -> Self {
                self.props.style.track = self.theme.color_of(token);
                self
            }

            /// The fill colour, named by its role.
            pub fn fill(mut self, token: silka_theme::ColorToken) -> Self {
                self.props.style.fill = self.theme.color_of(token);
                self
            }

            /// Bar height, or ring thickness, named by a spacing token.
            pub fn thickness(mut self, token: silka_theme::SpaceToken) -> Self {
                self.props.style.thickness = self.theme.space_of(token);
                self
            }

            /// The spring that carries the value to a new target.
            pub fn spring(mut self, spring: Spring) -> Self {
                self.props.spring = spring;
                self
            }

            /// The fraction asked for, or `None` when indeterminate.
            pub fn value_of(&self) -> Option<f32> {
                self.props.value
            }

            /// True when the fraction is unknown.
            pub fn is_indeterminate(&self) -> bool {
                self.props.value.is_none()
            }

            /// What a screen reader will announce as the value.
            pub fn announced_value(&self) -> Option<String> {
                self.props.value.map(percent_text)
            }

            /// Every resolved drawing value.
            pub fn style(&self) -> ProgressStyle {
                self.props.style
            }
        }

        impl From<$nama> for View {
            fn from(b: $nama) -> View {
                let mut builder = Builder::new($props(b.props));
                if let Some(key) = b.key {
                    builder = builder.key(key);
                }
                builder.into()
            }
        }
    };
}

/// The progress bar builder — Dart-style (§2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressBar {
    key: Option<Key>,
    props: ProgressProps,
    theme: Theme,
}

/// The progress ring builder — Dart-style (§2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressCircle {
    key: Option<Key>,
    props: ProgressProps,
    theme: Theme,
}

progress_builder!(ProgressBar, ProgressBarProps);
progress_builder!(ProgressCircle, ProgressCircleProps);

impl ProgressCircle {
    /// The ring's outer diameter, named by a spacing token.
    pub fn diameter(mut self, token: silka_theme::SpaceToken) -> Self {
        self.props.style.diameter = self.theme.space_of(token);
        self
    }
}

impl ProgressBar {
    /// The bar's natural length when nothing constrains it.
    pub fn length(mut self, length: f32) -> Self {
        self.props.style.length = if length.is_finite() {
            length.max(0.0)
        } else {
            0.0
        };
        self
    }
}

// ---------------------------------------------------------------------------
// Frame door
// ---------------------------------------------------------------------------

/// Every progress node in `tree`, in pre-order.
fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        if let Some(node) = tree.render(id) {
            if node.downcast_ref::<ProgressBarBox>().is_some()
                || node.downcast_ref::<ProgressCircleBox>().is_some()
            {
                out.push(id);
            }
        }
        for anak in tree.children(id) {
            kumpulkan(tree, *anak, out);
        }
    }
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

/// Advance every progress indicator by one frame.
///
/// Nothing here changes geometry — a bar's fill and a ring's arc live entirely
/// inside a box whose size never depends on the value — so the answer is always
/// [`Dirty::PAINT`] plus, while something is still moving, [`Dirty::ANIMATION`].
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in nodes(tree) {
        // Two statements rather than one chain: the second lookup needs its own
        // mutable borrow, and interleaving them in a single expression is how a
        // borrow outlives the value it produced.
        let batang = tree
            .node_mut_ref::<ProgressBarBox>(id)
            .map(|b| (b.advance(tick), b.is_animating()));
        let hasil = match batang {
            Some(h) => Some(h),
            None => tree
                .node_mut_ref::<ProgressCircleBox>(id)
                .map(|c| (c.advance(tick), c.is_animating())),
        };
        if let Some((bergeser, bergerak)) = hasil {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
        }
    }
    dirty
}

/// True while any progress indicator is still moving.
///
/// An **indeterminate** indicator answers true for as long as it is on screen:
/// that is not a leak, it is the one animation in this framework that is
/// legitimately endless, and it is why closing the panel that holds it is what
/// lets the GPU sleep again.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<ProgressBarBox>(id)
            .is_some_and(ProgressBarBox::is_animating)
            || tree
                .node_ref::<ProgressCircleBox>(id)
                .is_some_and(ProgressCircleBox::is_animating)
    })
}

/// Finish every value transition instantly (tests and snapshots).
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(b) = tree.node_mut_ref::<ProgressBarBox>(id) {
            b.settle();
        } else if let Some(c) = tree.node_mut_ref::<ProgressCircleBox>(id) {
            c.settle();
        }
        tree.mark_needs_paint(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::view::reconcile;
    use silka_theme::{Appearance, Preset, SpaceToken};

    const BOX: Size = Size::new(200.0, 60.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn tick(ms: u64, motion: Motion) -> Tick {
        Tick::manual(Duration::from_millis(ms), motion)
    }

    fn laid_out(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(BOX));
        tree
    }

    // -- pure geometry ----------------------------------------------------

    #[test]
    fn the_indeterminate_band_grows_out_of_one_edge_and_shrinks_into_the_other() {
        let (s0, w0) = indeterminate_span(0.05, 200.0, 70.0);
        assert_eq!(s0, 0.0, "still entering, so pinned to the start");
        assert!(w0 > 0.0 && w0 < 70.0);

        let (_, wm) = indeterminate_span(0.5, 200.0, 70.0);
        assert_eq!(wm, 70.0, "fully on the track");

        let (se, we) = indeterminate_span(0.95, 200.0, 70.0);
        assert!(we > 0.0 && we < 70.0, "leaving, so clipped again");
        assert_eq!(se + we, 200.0);
    }

    #[test]
    fn the_band_never_leaves_the_track_whatever_the_phase() {
        for i in 0..=40 {
            let p = i as f32 / 10.0 - 2.0; // deliberately outside 0..1
            let (s, w) = indeterminate_span(p, 120.0, 40.0);
            assert!(s >= 0.0, "phase {p}");
            assert!(s + w <= 120.0 + 1e-3, "phase {p}");
        }
        // A degenerate track produces no band rather than a NaN.
        assert_eq!(indeterminate_span(0.5, 0.0, 40.0), (0.0, 0.0));
    }

    #[test]
    fn an_arc_starts_at_twelve_o_clock_and_runs_clockwise() {
        let c = Point::new(50.0, 50.0);
        let pts = arc_points(c, 20.0, 0.0, 0.5, 32);
        assert!((pts[0].x - 50.0).abs() < 1e-3 && (pts[0].y - 30.0).abs() < 1e-3);
        // A quarter of the way through half a turn is three o'clock.
        let quarter = pts[16];
        assert!((quarter.x - 70.0).abs() < 1e-3 && (quarter.y - 50.0).abs() < 1e-3);
        // Every point sits on the circle.
        for p in &pts {
            let d = ((p.x - c.x).powi(2) + (p.y - c.y).powi(2)).sqrt();
            assert!((d - 20.0).abs() < 1e-3);
        }
    }

    #[test]
    fn the_announced_value_is_a_percentage_and_never_a_lie() {
        assert_eq!(percent_text(0.0), "0%");
        assert_eq!(percent_text(0.505), "51%");
        assert_eq!(percent_text(f32::NAN), "0%");
        assert_eq!(percent_text(-3.0), "0%");
    }

    // -- bar --------------------------------------------------------------

    #[test]
    fn the_bar_takes_the_width_it_is_offered_and_a_token_of_height() {
        let t = theme();
        let tree = laid_out(progress_bar_in(&t, 0.5));
        let id = tree.children(tree.root())[0];
        assert_eq!(tree.size(id).width, BOX.width);
        assert_eq!(tree.size(id).height, t.space(BAR_THICKNESS_STEPS));
    }

    #[test]
    fn the_determinate_fill_grows_from_the_reading_start() {
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, progress_bar_in(&t, 0.25));
        tree.layout(BoxConstraints::loose(BOX));
        settle(&mut tree);
        let id = tree.children(tree.root())[0];
        let node = tree.node_ref::<ProgressBarBox>(id).unwrap();

        let ltr = node.fill_rect(Size::new(200.0, 4.0), TextDirection::Ltr);
        assert_eq!(ltr.min_x(), 0.0);
        assert_eq!(ltr.size.width, 50.0);

        // In an RTL document it grows from the right — the same 25%, mirrored.
        let rtl = node.fill_rect(Size::new(200.0, 4.0), TextDirection::Rtl);
        assert_eq!(rtl.max_x(), 200.0);
        assert_eq!(rtl.size.width, 50.0);
    }

    #[test]
    fn a_new_value_is_retargeted_rather_than_restarted() {
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, progress_bar_in(&t, 0.0));
        tree.layout(BoxConstraints::loose(BOX));
        let id = tree.children(tree.root())[0];

        reconcile(&mut tree, progress_bar_in(&t, 1.0));
        let tk = tick(16, Motion::Full);
        advance(&mut tree, &tk);
        let mid = tree.node_ref::<ProgressBarBox>(id).unwrap().progress();
        assert!(mid > 0.0 && mid < 1.0, "it travels rather than jumping");

        // Retargeting mid-flight keeps the motion in one piece: the value must
        // not snap back to zero and start again.
        reconcile(&mut tree, progress_bar_in(&t, 0.5));
        advance(&mut tree, &tk);
        let after = tree.node_ref::<ProgressBarBox>(id).unwrap().progress();
        assert!(after > 0.0, "a retarget must not throw the position away");
    }

    #[test]
    fn an_indeterminate_bar_keeps_asking_for_frames_and_a_determinate_one_does_not() {
        let t = theme();
        let mut tree = laid_out(progress_bar_in(&t, 0.0).indeterminate());
        let tk = tick(16, Motion::Full);
        for _ in 0..10 {
            assert!(advance(&mut tree, &tk).contains(Dirty::ANIMATION));
        }
        assert!(is_animating(&tree));

        let mut settled = laid_out(progress_bar_in(&t, 0.4));
        settle(&mut settled);
        advance(&mut settled, &tk);
        assert!(!is_animating(&settled), "a settled bar lets the GPU sleep");
    }

    #[test]
    fn reduced_motion_freezes_the_endless_sweep() {
        let t = theme();
        let mut tree = laid_out(progress_bar_in(&t, 0.0).indeterminate());
        let id = tree.children(tree.root())[0];
        advance(&mut tree, &tick(200, Motion::Full));
        let bergerak = tree.node_ref::<ProgressBarBox>(id).unwrap().phase();
        assert!(bergerak > 0.0);

        advance(&mut tree, &tick(200, Motion::Reduced));
        let beku = tree.node_ref::<ProgressBarBox>(id).unwrap().phase();
        assert_eq!(
            beku, bergerak,
            "a looping animation is what the setting is for"
        );
        assert!(!is_animating(&tree), "and it stops asking for frames");
    }

    #[test]
    fn a_screen_reader_hears_a_progress_indicator_with_a_percentage() {
        let tree = laid_out(progress_bar_in(&theme(), 0.42).label("Importing invoices"));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Importing invoices")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::ProgressIndicator);
        assert_eq!(e.node.value.as_deref(), Some("42%"));
    }

    #[test]
    fn an_indeterminate_indicator_announces_no_value_at_all() {
        let tree = laid_out(
            progress_bar_in(&theme(), 0.0)
                .indeterminate()
                .label("Connecting"),
        );
        let a11y = tree.access_tree(None);
        let e = a11y.find_label("Connecting").unwrap();
        assert_eq!(
            e.node.value, None,
            "a made-up 0% is announced as progress that never moves"
        );
    }

    // -- circle -----------------------------------------------------------

    #[test]
    fn the_ring_is_square_and_sized_by_a_token() {
        let t = theme();
        let tree = laid_out(progress_circle_in(&t, 0.5));
        let id = tree.children(tree.root())[0];
        let size = tree.size(id);
        assert_eq!(size.width, size.height);
        assert_eq!(size.width, t.space(CIRCLE_DIAMETER_STEPS));
    }

    #[test]
    fn the_determinate_arc_starts_at_the_top_and_the_indeterminate_one_travels() {
        let t = theme();
        let mut tree = laid_out(progress_circle_in(&t, 0.25));
        settle(&mut tree);
        let id = tree.children(tree.root())[0];
        assert_eq!(
            tree.node_ref::<ProgressCircleBox>(id).unwrap().arc(),
            (0.0, 0.25)
        );

        let mut spin = laid_out(progress_circle_in(&t, 0.0).indeterminate());
        advance(&mut spin, &tick(350, Motion::Full));
        let id = spin.children(spin.root())[0];
        let (start, sweep) = spin.node_ref::<ProgressCircleBox>(id).unwrap().arc();
        assert!(start > 0.0, "the arc has moved round");
        assert_eq!(sweep, INDETERMINATE_SWEEP, "its length never changes");
    }

    #[test]
    fn a_ring_draws_two_strokes_and_a_zero_progress_ring_draws_one() {
        use silka_paint::{Command, Scene};
        let t = theme();
        let strokes = |tree: &mut RenderTree| {
            let mut scene = Scene::new(Color::BLACK);
            tree.paint_into(&mut scene);
            scene
                .commands()
                .iter()
                .filter(|c| matches!(c, Command::Stroke(_)))
                .count()
        };

        let mut half = laid_out(progress_circle_in(&t, 0.5));
        settle(&mut half);
        assert_eq!(strokes(&mut half), 2, "the track and the arc");

        let mut zero = laid_out(progress_circle_in(&t, 0.0));
        settle(&mut zero);
        assert_eq!(strokes(&mut zero), 1, "an empty arc costs no command");
    }

    #[test]
    fn both_indicators_are_token_driven_in_every_preset_and_appearance() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let light = ProgressStyle::bar(&Theme::new(preset, Appearance::Light));
            let dark = ProgressStyle::bar(&Theme::new(preset, Appearance::Dark));
            assert_ne!(light.track, dark.track, "{preset:?}");
            assert_ne!(light.fill, dark.fill, "{preset:?}");
        }
        // And a caller can still name a role rather than a colour.
        let t = theme();
        let warn = progress_bar_in(&t, 0.3).fill(ColorToken::Warning);
        assert_eq!(warn.style().fill, t.color.warning);
        let thick = progress_bar_in(&t, 0.3).thickness(SpaceToken::S2);
        assert_eq!(thick.style().thickness, t.space_of(SpaceToken::S2));
    }
}
