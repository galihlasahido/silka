//! `segmented_control()` — `NSSegmentedControl` / `UISegmentedControl`
//! (`KOMPONEN.md` Tier 3).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! use silka_widgets::{segment, segmented_control};
//!
//! # let rt = Runtime::new();
//! let mode = rt.signal(0usize);
//!
//! let picker = segmented_control([segment("Day"), segment("Week"), segment("Month")])
//!     .selected(mode.get())
//!     .label("Calendar range")
//!     .on_select(move |i| mode.set(i));
//! # let _ = picker;
//! ```
//!
//! # Why this is not a `tabs` variant
//!
//! [`tabs`](mod@crate::tabs) already draws a segmented *look*, and for a while
//! that was the only segmented control in the catalogue. It is the wrong home
//! for this component for three reasons that are contract, not decoration:
//!
//! | | `tabs` | `segmented_control` |
//! |---|---|---|
//! | What it means | "which **page** am I looking at" | "which **value** did I pick" |
//! | AccessKit | [`AccessRole::TabList`] + [`AccessRole::Tab`] | [`AccessRole::Group`] + [`AccessRole::RadioButton`], each announced as "*n* of *m*" |
//! | Gesture | click | click **and drag** — the thumb follows the finger across segments, the iOS habit |
//!
//! A screen reader user hearing "tab, 2 of 3" in a settings row is being told
//! that pressing it navigates somewhere; it does not. That single sentence is
//! the whole reason this file exists — everything else follows from it.
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Item | Where |
//! |---|---|
//! | Correct in both presets | [`SegmentedStyle::from_theme`] — the one place in this module allowed to see a [`Theme`] |
//! | Interactive state on springs | the thumb ([`SegmentedBox`]) and each segment's hover tint ([`SegmentBox`]) |
//! | Full keyboard + focus ring | one control = one Tab stop; ←/→ move the selection (mirrored in RTL), Home/End jump to the ends, the ring rides the thumb |
//! | AccessKit node | a [`AccessRole::Group`] of [`AccessRole::RadioButton`]s carrying `toggled` plus position-in-set |
//! | Dark mode | tokens only, without a branch |
//! | Hit target ≥ 44pt | [`SegmentedStyle::min_height`], forced during layout |
//! | Reduced motion | thumb [`Essential`](silka_core::animation::MotionRole::Essential) (loses its bounce), hover tint [`Decorative`](silka_core::animation::MotionRole::Decorative) (disappears) |
//!
//! # Who ticks the springs
//!
//! The same door as everywhere else: the shell calls [`crate::advance`] once a
//! frame and it reaches [`advance`] here. Until an application wires up an
//! [`AnimationDriver`](silka_core::animation::AnimationDriver), transitions run
//! as jumps rather than freezing halfway.

use silka_core::access::{AccessActions, AccessNode, AccessRole, AccessToggled};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick, Tolerance};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{
    BoxConstraints, CrossAlign, Decoration, FocusRing, LayoutCtx, MainAlign, NodeId, PaintCtx,
    RenderNode, RenderTree,
};
use silka_core::view::{row, Builder, View, ViewNode};
use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, Size};
use silka_text::FontWeight;
use silka_theme::{SpaceToken, Theme};

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::icon::{icon_in, IconName};
use crate::images::Images;
use crate::text::text_in;

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every visual value of a segmented control, **already resolved** from the
/// theme tokens (§2.6, §2.7).
///
/// Split out from the builder for the same reason [`crate::tabs::TabsStyle`] is:
/// questions like "does the thumb keep clear of the well's border" are pure
/// geometry and deserve to be answerable without a render tree, a GPU or a
/// window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentedStyle {
    /// The well the thumb slides inside.
    pub track: Decoration,
    /// Background, border, corners and shadow of the sliding thumb.
    pub thumb: Decoration,
    /// Inset of the thumb from the edges of the selected segment.
    pub thumb_inset: Insets,
    /// Hairline drawn between two neighbouring *unselected* segments.
    pub divider: Color,
    /// Thickness of that hairline.
    pub divider_thickness: f32,
    /// Keyboard focus ring (token `focus_ring`).
    pub focus_ring: FocusRing,
    /// Padding inside the well.
    pub padding: Insets,
    /// Minimum height of the control — the HIG hit target.
    pub min_height: f32,
    /// Corner shape of one segment: the hover tint **and** hit-testing (§3.6).
    pub segment_corners: Corners,
    /// Padding inside one segment.
    pub segment_padding: Insets,
    /// Gap between an icon and its label.
    pub icon_gap: f32,
    /// Highlight while the pointer is over an unselected segment.
    pub hover: Color,
    /// Label colour of an unselected segment.
    pub label: Color,
    /// Label colour of the selected segment.
    pub selected_label: Color,
    /// Label colour of a disabled segment.
    pub disabled_label: Color,
    /// Label font size, in logical points.
    pub label_size: f32,
}

impl SegmentedStyle {
    /// Resolve every token.
    ///
    /// Not one colour originates here: both presets and dark mode are therefore
    /// automatically correct, without a single `if`.
    pub fn from_theme(theme: &Theme) -> Self {
        let rambut = theme.space_of(SpaceToken::Px);
        let sumur = theme.space(0.5);
        // The thumb's radius is the well's radius minus the padding, which is
        // what makes two concentric rounded rectangles look concentric instead
        // of merely nested (the "inner radius" rule).
        let dalam = (theme.radius.md - sumur).max(0.0);
        Self {
            track: Decoration::fill(theme.color.surface_sunken)
                .corners(theme.corners(theme.radius.md))
                .border(rambut, theme.color.separator),
            thumb: Decoration::fill(theme.color.surface_elevated)
                .corners(theme.corners(dalam))
                .border(rambut, theme.color.separator)
                .shadows(theme.shadow.sm),
            thumb_inset: Insets::ZERO,
            divider: theme.color.separator,
            divider_thickness: rambut,
            focus_ring: FocusRing::new(theme.space(0.5), theme.color.focus_ring),
            padding: Insets::all(sumur),
            min_height: MIN_HIT_TARGET,
            segment_corners: theme.corners(dalam),
            segment_padding: Insets::symmetric(theme.space(2.5), theme.space(1.0)),
            icon_gap: theme.space(1.5),
            hover: theme.color.surface_hover,
            label: theme.color.secondary_label,
            selected_label: theme.color.label,
            disabled_label: theme.color.disabled_label,
            label_size: theme.typography.body_size,
        }
    }

    /// The thumb rect for the segment occupying `segment` (control-local
    /// coordinates).
    ///
    /// A pure function, so every geometric promise this component makes can be
    /// tested without touching a tree.
    pub fn thumb_rect(&self, segment: Rect) -> Rect {
        segment.deflate(self.thumb_inset)
    }

    /// True when the thumb contributes any pixels at all.
    pub fn thumb_is_visible(&self) -> bool {
        self.thumb.is_visible()
    }
}

// ---------------------------------------------------------------------------
// Segment
// ---------------------------------------------------------------------------

/// One choice inside a segmented control.
///
/// Deliberately **not** a [`View`], for the same reason [`crate::Tab`] is not:
/// the control has to read `disabled` before the tree is assembled (the arrow
/// keys skip disabled choices), and the moment something becomes a `View` its
/// props are buried behind `dyn ViewNode`.
///
/// ```
/// use silka_widgets::{segment, IconName};
///
/// let day = segment("Day");
/// assert_eq!(day.label_text(), "Day");
/// assert!(!day.is_disabled());
///
/// // A segment may show an icon beside its label, or instead of it — an
/// // icon-only segment still carries the label as its accessible name.
/// let starred = segment("Starred").icon(IconName::Star).icon_only(true);
/// assert!(starred.is_icon_only());
/// assert_eq!(starred.label_text(), "Starred");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    label: String,
    icon: Option<IconName>,
    icon_only: bool,
    disabled: bool,
    key: Option<Key>,
}

/// One choice labelled `label`.
///
/// ```
/// use silka_widgets::segment;
///
/// // Segments are plain values, so a control can be built from data.
/// let items: Vec<_> = ["Day", "Week", "Month"].into_iter().map(segment).collect();
/// assert_eq!(items.len(), 3);
/// assert_eq!(items[2].label_text(), "Month");
/// ```
pub fn segment(label: impl Into<String>) -> Segment {
    Segment {
        label: label.into(),
        icon: None,
        icon_only: false,
        disabled: false,
        key: None,
    }
}

impl Segment {
    /// Show a symbol beside the label.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Draw **only** the symbol; the label survives as the accessible name.
    ///
    /// This is the one thing an icon-only control cannot borrow from what it
    /// draws, which is why the label is still required rather than optional.
    pub fn icon_only(mut self, icon_only: bool) -> Self {
        self.icon_only = icon_only;
        self
    }

    /// A choice that cannot be picked (still announced, as dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Identity key — required for controls whose contents change (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The name a screen reader announces.
    pub fn label_text(&self) -> &str {
        &self.label
    }

    /// The symbol shown, if any.
    pub fn icon_name(&self) -> Option<IconName> {
        self.icon
    }

    /// True when only the symbol is drawn.
    pub fn is_icon_only(&self) -> bool {
        self.icon_only && self.icon.is_some()
    }

    /// True when this choice cannot be picked.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }
}

// ---------------------------------------------------------------------------
// OnPick
// ---------------------------------------------------------------------------

/// The "segment `index` was picked" action the app entrusts to the control.
///
/// The same three properties as [`silka_core::Callback`]: cheap to `Clone`,
/// `PartialEq` by identity, and the only thing it may do is write a signal.
#[derive(Clone)]
pub struct OnPick(std::rc::Rc<dyn Fn(usize)>);

impl OnPick {
    /// Wrap a closure.
    pub fn new(f: impl Fn(usize) + 'static) -> Self {
        Self(std::rc::Rc::new(f))
    }

    /// Run the action for segment `index`.
    pub fn call(&self, index: usize) {
        (self.0)(index)
    }
}

impl PartialEq for OnPick {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for OnPick {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OnPick")
    }
}

// ---------------------------------------------------------------------------
// The segment node
// ---------------------------------------------------------------------------

/// Motion role of a segment's hover tint under reduced-motion.
///
/// A constant so tests can name it without prying into the node.
pub const SEGMENT_TINT_MOTION: MotionRole = MotionRole::Decorative;

/// Render node for one segment.
///
/// It owns exactly two things: a hover tint on a spring, and the accessibility
/// node. Everything else — pressing, dragging, the thumb, focus — belongs to
/// [`SegmentedBox`], because all of it is about the *relationship* between
/// segments and none of it can be decided by one segment alone.
pub struct SegmentBox {
    /// The name a screen reader announces.
    pub label: String,
    /// Position within the control (0-based); `position_in_set` is this plus 1.
    pub index: usize,
    /// How many segments the control holds — a screen reader says "2 of 3".
    pub count: usize,
    /// Currently the picked segment.
    pub selected: bool,
    /// Cannot be picked (still announced, as dimmed).
    pub disabled: bool,
    /// Corner shape of the tint — **identical** to the hit shape (§3.6).
    pub corners: Corners,
    /// Hover tint (token `surface_hover`).
    pub hover: Color,

    hovered: bool,
    tint: SpringValue<Color>,
    driven: bool,
}

impl SegmentBox {
    /// The tint that should apply to the current state.
    ///
    /// The resting value is the hover colour at zero alpha rather than
    /// [`Color::TRANSPARENT`], so only the alpha travels and the tint never
    /// appears to darken on its way out.
    fn target_tint(&self) -> Color {
        if self.hovered && !self.disabled && !self.selected {
            self.hover
        } else {
            self.hover.with_alpha(0.0)
        }
    }

    fn arahkan(&mut self) {
        let target = self.target_tint();
        if self.driven {
            self.tint.set_target(target);
        } else {
            self.tint.jump_to(target);
        }
    }

    /// The tint painted this frame.
    pub fn tint(&self) -> Color {
        self.tint.position()
    }

    /// The pointer is over this segment.
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// True while the tint is still moving.
    pub fn is_animating(&self) -> bool {
        self.tint.is_animating()
    }

    /// Advance the tint by one frame; true if its colour changed.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        self.driven = true;
        if !self.tint.is_animating() {
            return false;
        }
        let sebelum = self.tint.position();
        tick.advance(&mut self.tint);
        self.tint.position() != sebelum
    }

    /// Finish the transition instantly (tests and snapshots).
    pub fn settle(&mut self) {
        self.tint.settle();
    }
}

impl RenderNode for SegmentBox {
    fn type_name(&self) -> &'static str {
        "Segment"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(size)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let sorot = self.tint.position();
        if sorot.a > 0.0 {
            ctx.quad(
                Quad::new(ctx.local_bounds())
                    .background(sorot)
                    .corners(self.corners),
            );
        }
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        // A choice, not a page: `RadioButton` is what tells a screen reader that
        // picking this changes a value rather than navigating somewhere.
        node.role = AccessRole::RadioButton;
        node.label = Some(self.label.clone());
        node.disabled = self.disabled;
        node.toggled = Some(AccessToggled::from(self.selected));
        node.position_in_set = Some(self.index + 1);
        node.size_of_set = Some(self.count);
        if !self.disabled {
            node.actions |= AccessActions::CLICK;
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Opaque even when disabled: a click on a dimmed segment must not fall
        // through to whatever sits behind the control.
        HitBehavior::Opaque
    }

    /// **One control = one Tab stop.** Focus belongs to [`SegmentedBox`].
    fn focus_policy(&self) -> FocusPolicy {
        FocusPolicy::NONE
    }

    fn cursor(&self) -> Option<CursorIcon> {
        (!self.disabled).then_some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else { return };
        match p.phase {
            PointerPhase::Enter if !self.hovered => {
                self.hovered = true;
                self.arahkan();
                ctx.request_paint();
                ctx.request_animation();
            }
            PointerPhase::Leave if self.hovered => {
                self.hovered = false;
                self.arahkan();
                ctx.request_paint();
                ctx.request_animation();
            }
            // Down/Up are deliberately left alone — and deliberately **not**
            // marked handled: pressing, dragging and focus all belong to the
            // control above, and the only way it can see them is by letting
            // them bubble.
            _ => {}
        }
    }
}

impl core::fmt::Debug for SegmentBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SegmentBox")
            .field("label", &self.label)
            .field("index", &self.index)
            .field("selected", &self.selected)
            .field("disabled", &self.disabled)
            .finish()
    }
}

/// Props for one segment — the view form of [`SegmentBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentProps {
    pub(crate) label: String,
    pub(crate) index: usize,
    pub(crate) count: usize,
    pub(crate) selected: bool,
    pub(crate) disabled: bool,
    pub(crate) corners: Corners,
    pub(crate) hover: Color,
    pub(crate) spring: Spring,
}

impl ViewNode for SegmentProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(SegmentBox {
            label: self.label.clone(),
            index: self.index,
            count: self.count,
            selected: self.selected,
            disabled: self.disabled,
            corners: self.corners,
            hover: self.hover,
            hovered: false,
            tint: SpringValue::new(self.hover.with_alpha(0.0))
                .with_spring(self.spring)
                .with_tolerance(Tolerance::COLOR)
                // A hover tint explains nothing, so reduced-motion removes it
                // outright rather than merely taking its bounce away.
                .decorative(),
            driven: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<SegmentBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        n.index = self.index;
        n.count = self.count;
        if n.selected != self.selected {
            n.selected = self.selected;
            n.arahkan();
            dirty |= Dirty::PAINT;
        }
        if n.corners != self.corners {
            n.corners = self.corners;
            dirty |= Dirty::PAINT;
        }
        if n.hover != self.hover {
            n.hover = self.hover;
            n.arahkan();
            dirty |= Dirty::PAINT;
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                // A segment disabled mid-hover would never receive its Leave.
                n.hovered = false;
            }
            n.arahkan();
            dirty |= Dirty::PAINT;
        }
        if n.tint.spring() != self.spring {
            n.tint.set_spring(self.spring);
        }
        dirty
    }
}

// ---------------------------------------------------------------------------
// The control node
// ---------------------------------------------------------------------------

/// Render node for the whole control: placement, thumb, drag, keyboard, a11y.
pub struct SegmentedBox {
    /// Visual values already resolved from the tokens.
    pub style: SegmentedStyle,
    /// The picked segment (a controlled prop — the node never moves it itself).
    pub selected: usize,
    /// The control's name for screen readers.
    pub label: Option<String>,
    /// What runs when the user picks a different segment.
    pub on_select: Option<OnPick>,
    /// Which segments can still be picked — length = number of segments.
    pub enabled: Vec<bool>,

    /// Rect of the picked segment; the thumb shape derives from it at paint.
    thumb: SpringValue<Rect>,
    /// Rect of every segment from the last layout (control-local).
    placed: Vec<Rect>,
    /// A layout pass has filled [`SegmentedBox::placed`].
    ready: bool,
    /// Holding keyboard focus.
    focused: bool,
    /// A press is in flight and the pointer is captured.
    dragging: bool,
    /// Reading direction from the last layout — arrows are mirrored (§9.8).
    rtl: bool,
    /// True as soon as anything has called [`SegmentedBox::advance`].
    driven: bool,
}

impl SegmentedBox {
    /// The thumb rect painted this frame (control-local coordinates).
    pub fn thumb_rect(&self) -> Rect {
        self.style.thumb_rect(self.thumb.position())
    }

    /// The picked segment's rect as it is currently being animated.
    pub fn active_rect(&self) -> Rect {
        self.thumb.position()
    }

    /// Rect of every segment from the last layout.
    pub fn segment_rects(&self) -> &[Rect] {
        &self.placed
    }

    /// True while the thumb is still moving.
    pub fn is_animating(&self) -> bool {
        self.thumb.is_animating()
    }

    /// Holding keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// A press is in flight.
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// The spring driving the thumb.
    pub fn spring(&self) -> Spring {
        self.thumb.spring()
    }

    /// Number of segments.
    pub fn len(&self) -> usize {
        self.enabled.len()
    }

    /// True when there are no segments at all.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_empty()
    }

    /// True when segment `index` can still be picked.
    pub fn is_enabled(&self, index: usize) -> bool {
        self.enabled.get(index).copied().unwrap_or(false)
    }

    /// Which segment covers `local`, if any.
    ///
    /// Used by both the initial press and every drag step, so a finger sliding
    /// across the control is answered by exactly the same rule as a click.
    pub fn segment_at(&self, local: Point) -> Option<usize> {
        self.placed.iter().position(|r| {
            local.x >= r.min_x()
                && local.x < r.max_x()
                && local.y >= r.min_y()
                && local.y < r.max_y()
        })
    }

    /// The segment whose horizontal band contains `local.x`, clamped to the
    /// ends.
    ///
    /// The drag variant of [`SegmentedBox::segment_at`]: a finger that wanders
    /// above or below the control while sliding keeps steering the thumb rather
    /// than dropping it — the iOS behaviour, and the reason those two functions
    /// are not one.
    pub fn segment_near(&self, local: Point) -> Option<usize> {
        if self.placed.is_empty() {
            return None;
        }
        if let Some(i) = self
            .placed
            .iter()
            .position(|r| local.x >= r.min_x() && local.x < r.max_x())
        {
            return Some(i);
        }
        // Past one of the ends: the nearest band wins.
        let mut terbaik = 0usize;
        let mut jarak = f32::INFINITY;
        for (i, r) in self.placed.iter().enumerate() {
            let d = if local.x < r.min_x() {
                r.min_x() - local.x
            } else {
                local.x - r.max_x()
            };
            if d < jarak {
                jarak = d;
                terbaik = i;
            }
        }
        Some(terbaik)
    }

    /// Aim the thumb at `kotak`.
    fn arahkan(&mut self, kotak: Rect) {
        if self.driven {
            self.thumb.set_target(kotak);
        } else {
            self.thumb.jump_to(kotak);
        }
    }

    /// Move the picked segment to `index` — a **retarget**, not a new
    /// animation, so a selection changed mid-flight carries its velocity
    /// (§3.5).
    pub fn set_selected(&mut self, index: usize) {
        if self.selected == index {
            return;
        }
        self.selected = index;
        if let Some(kotak) = self.placed.get(index).copied() {
            self.arahkan(kotak);
        }
    }

    /// Advance the thumb by one frame; true if its rect changed.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        self.driven = true;
        if !self.thumb.is_animating() {
            return false;
        }
        let sebelum = self.thumb.position();
        tick.advance(&mut self.thumb);
        self.thumb.position() != sebelum
    }

    /// Finish the transition instantly (tests and snapshots).
    pub fn settle(&mut self) {
        self.thumb.settle();
    }

    /// The next enabled segment from `dari` in the direction of `langkah`.
    ///
    /// No wrapping: a right arrow on the last segment does not jump back to the
    /// first, which is the `NSSegmentedControl` habit and the reason a keyboard
    /// user never loses track of where they are.
    pub fn tetangga(&self, dari: usize, langkah: i32) -> Option<usize> {
        let n = self.enabled.len();
        if n == 0 {
            return None;
        }
        let mut i = dari as i32;
        loop {
            i += langkah;
            if i < 0 || i >= n as i32 {
                return None;
            }
            if self.enabled[i as usize] {
                return Some(i as usize);
            }
        }
    }

    /// The first enabled segment (positive `langkah`) or the last (negative).
    pub fn ujung(&self, langkah: i32) -> Option<usize> {
        let n = self.enabled.len();
        if n == 0 {
            return None;
        }
        if langkah >= 0 {
            (0..n).find(|i| self.enabled[*i])
        } else {
            (0..n).rev().find(|i| self.enabled[*i])
        }
    }

    /// Ask for the selection to move to `index`; true if the callback ran.
    ///
    /// The node does **not** move its own selection: `selected` arrives through
    /// props, exactly like `open` on [`crate::overlay::OverlayEntry`]. All that
    /// happens here is the callback; the next frame brings the answer back.
    pub fn request_select(&mut self, index: usize) -> bool {
        if index == self.selected || !self.is_enabled(index) {
            return false;
        }
        let Some(cb) = self.on_select.clone() else {
            return false;
        };
        cb.call(index);
        true
    }
}

impl RenderNode for SegmentedBox {
    fn type_name(&self) -> &'static str {
        "SegmentedControl"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let n = ctx.child_count();
        if n == 0 {
            self.placed.clear();
            return constraints.smallest();
        }

        let pad = self.style.padding;
        let dalam = constraints.deflate(pad).loosen();

        // Pass 1 — "how big would you like to be?". The minimum height is
        // forced here so the HIG hit target never depends on what a label
        // happens to contain.
        let ukur = BoxConstraints::new(
            0.0,
            dalam.max_width,
            self.style.min_height,
            dalam.max_height,
        );
        let mut terlebar = 0.0f32;
        let mut tinggi = self.style.min_height;
        for i in 0..n {
            let anak = ctx.child(i);
            let s = ctx.layout_child_measured(anak, ukur);
            terlebar = terlebar.max(s.width);
            tinggi = tinggi.max(s.height);
        }

        // Equal widths, always: that is what makes a segmented control a
        // *control* rather than a row of buttons, and it is what lets the thumb
        // travel a constant distance per step.
        let size = constraints.constrain(Size::new(
            terlebar * n as f32 + pad.horizontal(),
            tinggi + pad.vertical(),
        ));
        let isi_lebar = (size.width - pad.horizontal()).max(0.0);
        let lebar = isi_lebar / n as f32;
        let tinggi_isi = (size.height - pad.vertical()).max(0.0);

        // Pass 2 — every segment receives its rect. These tight constraints
        // were derived from measuring the very same children, so they must not
        // turn those children into relayout boundaries (the `TaffyBox` rule).
        self.placed.clear();
        for i in 0..n {
            let anak = ctx.child(i);
            ctx.layout_child_measured(anak, BoxConstraints::tight(Size::new(lebar, tinggi_isi)));
            let x = pad.left + lebar * i as f32;
            // Following the reading direction: in RTL the first segment sits on
            // the right (§9.8).
            let kiri = if self.rtl { size.width - x - lebar } else { x };
            let kotak = Rect::new(kiri, pad.top, lebar, tinggi_isi);
            ctx.place_child(anak, kotak.origin);
            self.placed.push(kotak);
        }

        let aktif = self
            .placed
            .get(self.selected.min(self.placed.len().saturating_sub(1)))
            .copied();
        if let Some(kotak) = aktif {
            if !self.ready {
                // A freshly built control does not glide in from the corner:
                // the picked segment is already where it belongs.
                self.thumb.jump_to(kotak);
                self.ready = true;
            } else if self.thumb.is_animating() {
                self.thumb.set_target(kotak);
            } else {
                // The window was resized: the rect moved, the selection did
                // not — so this is a move, not an animation.
                self.thumb.jump_to(kotak);
            }
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.style.track);

        // Hairlines between neighbouring segments, the `NSSegmentedControl`
        // detail. They are skipped next to the thumb — a divider running into
        // the thumb's shadow is what makes a segmented control look muddy.
        if self.style.divider.a > 0.0 && self.style.divider_thickness > 0.0 && self.placed.len() > 1
        {
            let t = self.style.divider_thickness;
            let thumb = self.thumb_rect();
            for pair in self.placed.windows(2) {
                let (kiri, kanan) = (pair[0], pair[1]);
                let x = (kiri.max_x() + kanan.min_x()) * 0.5 - t * 0.5;
                let dekat_thumb = self.ready
                    && self.style.thumb_is_visible()
                    && x + t > thumb.min_x() - t
                    && x < thumb.max_x() + t;
                if dekat_thumb {
                    continue;
                }
                let inset = kiri.size.height * 0.2;
                ctx.quad(
                    Quad::new(Rect::new(
                        x,
                        kiri.min_y() + inset,
                        t,
                        (kiri.size.height - inset * 2.0).max(0.0),
                    ))
                    .background(self.style.divider),
                );
            }
        }

        if self.ready && self.style.thumb_is_visible() && !self.placed.is_empty() {
            let d = self.style.thumb;
            ctx.shadowed(
                Quad::new(self.thumb_rect())
                    .background(d.background)
                    .corners(d.corners)
                    .border(d.border_width, d.border_color),
                d.shadows,
            );
        }

        ctx.paint_children();

        // The ring surrounds the **thumb**, so it glides with the selection:
        // the keyboard always points where the eye is already looking.
        if self.focused && self.ready && !self.placed.is_empty() {
            let ring = self.style.focus_ring;
            if ring.is_visible() {
                let kotak = self.active_rect().deflate(Insets::all(-ring.width));
                let corners = Corners::new(
                    CornerRadii::all(self.style.segment_corners.radii.max() + ring.width),
                    self.style.segment_corners.style,
                );
                ctx.quad(
                    Quad::new(kotak)
                        .corners(corners)
                        .border(ring.width, ring.color),
                );
            }
        }
    }

    fn access(&self, node: &mut AccessNode) {
        // A group of radio buttons, not a tab list: this control picks a value.
        node.role = AccessRole::Group;
        node.label.clone_from(&self.label);
        if self.ujung(1).is_some() {
            node.actions |= AccessActions::FOCUS;
        }
    }

    fn hit_behavior(&self) -> HitBehavior {
        // The well's padding belongs to the control: a click 2pt from the edge
        // must not fall through to the page behind.
        HitBehavior::Opaque
    }

    /// One control = **one** Tab stop; the arrows work inside it.
    fn focus_policy(&self) -> FocusPolicy {
        if self.ujung(1).is_some() {
            FocusPolicy::FOCUSABLE
        } else {
            FocusPolicy::NONE
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            Event::Pointer(p) => match p.phase {
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    if self.ujung(1).is_some() {
                        ctx.request_focus();
                    }
                    ctx.handled();
                    let Some(i) = self.segment_at(ctx.local()) else {
                        return;
                    };
                    // Capture *before* the callback: the app may rebuild
                    // synchronously, and a drag that lost its capture in the
                    // middle would leave the thumb stranded.
                    self.dragging = true;
                    ctx.capture_pointer();
                    ctx.request_paint();
                    ctx.request_animation();
                    self.request_select(i);
                }
                // The thumb tracks the finger: sliding across segments picks
                // them as it goes, which is what makes this feel like iOS
                // rather than like three buttons.
                PointerPhase::Move if self.dragging => {
                    let Some(i) = self.segment_near(ctx.local()) else {
                        return;
                    };
                    ctx.handled();
                    if self.request_select(i) {
                        ctx.request_paint();
                        ctx.request_animation();
                    }
                }
                PointerPhase::Up if self.dragging => {
                    self.dragging = false;
                    ctx.release_pointer();
                    ctx.handled();
                    ctx.request_paint();
                }
                // Cancelled by the OS is not a release, and it is not an undo
                // either: whatever the finger last slid onto stays picked, the
                // same as AppKit.
                PointerPhase::Cancel if self.dragging => {
                    self.dragging = false;
                    ctx.release_pointer();
                    ctx.request_paint();
                }
                _ => {}
            },
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                ctx.request_paint();
            }
            Event::Key(k) if k.is_pressed() && k.modifiers.is_empty() => {
                // The reading direction decides what the arrows mean: in RTL,
                // "right" is the previous segment (§9.8).
                let maju = if self.rtl { -1 } else { 1 };
                let tujuan = match k.code {
                    KeyCode::Named(NamedKey::ArrowRight) => self.tetangga(self.selected, maju),
                    KeyCode::Named(NamedKey::ArrowLeft) => self.tetangga(self.selected, -maju),
                    // Up/down are left alone on purpose: this control is
                    // horizontal, and swallowing them would steal scrolling
                    // from the page behind.
                    KeyCode::Named(NamedKey::Home) => self.ujung(1),
                    KeyCode::Named(NamedKey::End) => self.ujung(-1),
                    _ => return,
                };
                // Even an arrow that hits the end stays handled: without that,
                // Home on the first segment would bubble up and scroll the page
                // out from under the user.
                ctx.handled();
                ctx.request_paint();
                if let Some(i) = tujuan {
                    self.request_select(i);
                }
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for SegmentedBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SegmentedBox")
            .field("selected", &self.selected)
            .field("segments", &self.enabled.len())
            .field("thumb", &self.thumb.position())
            .field("focused", &self.focused)
            .field("dragging", &self.dragging)
            .finish()
    }
}

/// Props for the control — the view form of [`SegmentedBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentedProps {
    pub(crate) style: SegmentedStyle,
    pub(crate) selected: usize,
    pub(crate) label: Option<String>,
    pub(crate) on_select: Option<OnPick>,
    pub(crate) enabled: Vec<bool>,
    pub(crate) spring: Spring,
}

impl ViewNode for SegmentedProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(SegmentedBox {
            style: self.style,
            selected: self.selected,
            label: self.label.clone(),
            on_select: self.on_select.clone(),
            enabled: self.enabled.clone(),
            thumb: SpringValue::new(Rect::new(0.0, 0.0, 0.0, 0.0)).with_spring(self.spring),
            placed: Vec::new(),
            ready: false,
            focused: false,
            dragging: false,
            rtl: false,
            driven: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<SegmentedBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.enabled != self.enabled {
            n.enabled.clone_from(&self.enabled);
            dirty |= Dirty::PAINT;
        }
        if n.selected != self.selected {
            n.set_selected(self.selected);
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.thumb.spring() != self.spring {
            n.thumb.set_spring(self.spring);
        }
        // Callbacks are replaced without comparison: the closure is rebuilt on
        // every rebuild and captures fresh values (see `InteractiveProps`).
        n.on_select.clone_from(&self.on_select);
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Dart-style builder for a segmented control (§2.5).
///
/// Its own type rather than a [`Builder`] because it has to **assemble its
/// children** at the moment it becomes a [`View`]: label colours, weights and
/// per-index state all derive from `selected` and `style`, which are only known
/// once the whole chain has been written out.
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{segment, segmented_control_in, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
/// let rt = Runtime::new();
/// let mode = rt.signal(1usize);
///
/// let picker = segmented_control_in(
///     &fonts,
///     &theme,
///     [segment("Day"), segment("Week"), segment("Month")],
/// )
/// .selected(mode.get())
/// .label("Calendar range")
/// .on_select(move |i| mode.set(i));
///
/// assert_eq!(picker.active_index(), 1);
/// assert_eq!(picker.len(), 3);
///
/// // A selection past the end is clamped rather than panicking: a control
/// // whose choices shrank must not take the application down.
/// let clamped = segmented_control_in(&fonts, &theme, [segment("Only")]).selected(99);
/// assert_eq!(clamped.active_index(), 0);
/// ```
pub struct SegmentedControl {
    fonts: Fonts,
    images: Images,
    theme: Theme,
    items: Vec<Segment>,
    style: Option<SegmentedStyle>,
    selected: usize,
    label: Option<String>,
    on_select: Option<OnPick>,
    spring: Spring,
    key: Option<Key>,
}

/// A segmented control holding `items` — `segmented_control` (`KOMPONEN.md`
/// Tier 3).
///
/// ```
/// # use silka_core::signals::Runtime;
/// use silka_widgets::{segment, segmented_control};
///
/// # let rt = Runtime::new();
/// let mode = rt.signal(0usize);
/// let picker = segmented_control([segment("List"), segment("Grid")])
///     .selected(mode.get())
///     .on_select(move |i| mode.set(i));
/// # let _ = picker;
/// ```
///
/// Use [`segmented_control_in`] outside a build pass.
pub fn segmented_control(items: impl IntoIterator<Item = Segment>) -> SegmentedControl {
    segmented_control_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        items,
    )
}

/// [`segmented_control`] with the text engine and theme passed explicitly.
///
/// Not a single number originates in application code: every value comes from
/// `theme` (§2.6).
pub fn segmented_control_in(
    fonts: &Fonts,
    theme: &Theme,
    items: impl IntoIterator<Item = Segment>,
) -> SegmentedControl {
    SegmentedControl {
        fonts: fonts.clone(),
        images: crate::images::active_images(),
        theme: *theme,
        items: items.into_iter().collect(),
        style: None,
        selected: 0,
        label: None,
        on_select: None,
        // `snappy` is the preset closest to how picking a segment feels on
        // macOS and iOS: it arrives fast, with barely any bounce (WWDC23).
        spring: Spring::snappy(),
        key: None,
    }
}

impl SegmentedControl {
    /// The picked segment (a controlled component).
    pub fn selected(mut self, index: usize) -> Self {
        self.selected = index;
        self
    }

    /// What runs when the user picks a different segment.
    pub fn on_select(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_select = Some(OnPick::new(f));
        self
    }

    /// The control's name for screen readers.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The spring driving the thumb (`smooth`/`snappy`/`bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Replace every visual value at once — the escape hatch for brands that
    /// swapping theme tokens alone cannot express (§2.7).
    pub fn style(mut self, style: SegmentedStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// The images atlas used to rasterise segment icons.
    pub fn images(mut self, images: &Images) -> Self {
        self.images = images.clone();
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The visual values that will be used — the already-resolved tokens.
    pub fn resolved_style(&self) -> SegmentedStyle {
        self.style
            .unwrap_or_else(|| SegmentedStyle::from_theme(&self.theme))
    }

    /// How many segments there are.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when there are no segments at all.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The picked index that actually applies: clamped to the current list.
    ///
    /// An out-of-range index does not panic and does not make the thumb vanish
    /// — a control whose choices shrank one frame ahead of the signal holding
    /// the selection is normal, not an application bug.
    pub fn active_index(&self) -> usize {
        if self.items.is_empty() {
            return 0;
        }
        self.selected.min(self.items.len() - 1)
    }
}

impl From<SegmentedControl> for View {
    fn from(c: SegmentedControl) -> View {
        let style = c.resolved_style();
        let aktif = c.active_index();
        let n = c.items.len();
        let props = SegmentedProps {
            style,
            selected: aktif,
            label: c.label.clone(),
            on_select: c.on_select.clone(),
            enabled: c.items.iter().map(|i| !i.disabled).collect(),
            spring: c.spring,
        };

        let mut builder = Builder::new(props);
        for (i, item) in c.items.iter().enumerate() {
            builder = builder.child(segment_view(&c, &style, i, n, item, i == aktif));
        }
        if let Some(key) = c.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

/// Assemble one segment into a view: icon + label, both driven by tokens.
fn segment_view(
    c: &SegmentedControl,
    style: &SegmentedStyle,
    index: usize,
    count: usize,
    item: &Segment,
    selected: bool,
) -> View {
    let warna = if item.disabled {
        style.disabled_label
    } else if selected {
        style.selected_label
    } else {
        style.label
    };

    let mut isi: Vec<View> = Vec::with_capacity(2);
    if let Some(name) = item.icon {
        isi.push(View::from(
            icon_in(&c.images, &c.theme, name)
                .size_raw(style.label_size)
                .color_raw(warna)
                // The segment node already carries the accessible name; a
                // second one from the symbol would make a screen reader say it
                // twice.
                .decorative(),
        ));
    }
    if !item.is_icon_only() {
        isi.push(View::from(
            text_in(&c.fonts, &item.label)
                .size(style.label_size)
                .weight(if selected {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .color(warna)
                .single_line()
                // Same reason as the icon: the name lives on the segment.
                .role(AccessRole::Container),
        ));
    }

    // The content is centred by the layout engine — no arithmetic here (§3.4).
    let baris = row(isi)
        .main(MainAlign::Center)
        .cross(CrossAlign::Center)
        .spacing(style.icon_gap)
        .padding(style.segment_padding);

    let mut b = Builder::new(SegmentProps {
        label: item.label.clone(),
        index,
        count,
        selected,
        disabled: item.disabled,
        corners: style.segment_corners,
        hover: style.hover,
        spring: c.spring,
    })
    .child(baris);
    if let Some(key) = item.key.clone() {
        b = b.key(key);
    }
    b.into()
}

// ---------------------------------------------------------------------------
// Ticking
// ---------------------------------------------------------------------------

/// Every segmented-control node in `tree`, in pre-order.
fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        if let Some(node) = tree.render(id) {
            if node.downcast_ref::<SegmentedBox>().is_some()
                || node.downcast_ref::<SegmentBox>().is_some()
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

/// Advance every segmented-control transition by one frame.
///
/// The shell calls this once a frame, unconditionally, through
/// [`crate::advance`]. What comes back decides whether the next frame is
/// scheduled at all (§3.5):
///
/// - [`Dirty::PAINT`] — the thumb or a tint **changed** this frame.
/// - [`Dirty::ANIMATION`] — a spring has yet to settle. Once this flag is gone,
///   the GPU may sleep.
/// - [`Dirty::NONE`] — nothing here produced any work.
///
/// The thumb moves **without** triggering layout: segment positions do not
/// depend on it, so an animating control never forces the window to be
/// recomputed.
///
/// ```
/// # use silka_core::animation::{Motion, Tick};
/// # use silka_core::scheduler::Dirty;
/// # use silka_core::tree::{BoxConstraints, RenderTree};
/// # use silka_core::view::reconcile;
/// # use silka_paint::Size;
/// # use silka_theme::{Appearance, Theme};
/// # use silka_widgets::Fonts;
/// # use std::time::Duration;
/// use silka_widgets::segmented_control::{advance, segment, segmented_control_in};
///
/// # let fonts = Fonts::bundled_only();
/// # let t = Theme::tailwind(Appearance::Light);
/// let mut tree = RenderTree::new();
/// let tick = Tick::manual(Duration::from_millis(8), Motion::Full);
///
/// reconcile(
///     &mut tree,
///     segmented_control_in(&fonts, &t, [segment("One"), segment("Two")]).selected(0),
/// );
/// tree.layout(BoxConstraints::tight(Size::new(400.0, 60.0)));
/// // A freshly built control is already in place: nothing is moving.
/// assert_eq!(advance(&mut tree, &tick), Dirty::NONE);
/// ```
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in nodes(tree) {
        let (berubah, bergerak) = if let Some(c) = tree.node_mut_ref::<SegmentedBox>(id) {
            (c.advance(tick), c.is_animating())
        } else if let Some(s) = tree.node_mut_ref::<SegmentBox>(id) {
            (s.advance(tick), s.is_animating())
        } else {
            continue;
        };
        if berubah {
            tree.mark_needs_paint(id);
            dirty |= Dirty::PAINT;
        }
        if bergerak {
            dirty |= Dirty::ANIMATION;
        }
    }
    dirty
}

/// True while any segmented-control transition is still running.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<SegmentedBox>(id)
            .is_some_and(SegmentedBox::is_animating)
            || tree
                .node_ref::<SegmentBox>(id)
                .is_some_and(SegmentBox::is_animating)
    })
}

/// Finish every segmented-control transition instantly (tests and snapshots).
///
/// ```
/// use silka_core::tree::RenderTree;
/// use silka_widgets::segmented_control::{is_animating, settle};
///
/// // A tree without a segmented control is trivially at rest, so this may be
/// // called unconditionally.
/// let mut tree = RenderTree::new();
/// assert!(!is_animating(&tree));
/// settle(&mut tree);
/// assert!(!is_animating(&tree));
/// ```
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(c) = tree.node_mut_ref::<SegmentedBox>(id) {
            c.settle();
        } else if let Some(s) = tree.node_mut_ref::<SegmentBox>(id) {
            s.settle();
        }
        tree.mark_needs_paint(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::input::{InputRouter, KeyEvent, PointerEvent};
    use silka_core::signals::Runtime;
    use silka_core::tree::TextDirection;
    use silka_core::view::reconcile;
    use silka_theme::{Appearance, Preset};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    const BOX: Size = Size::new(360.0, 80.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn built(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(BOX));
        tree
    }

    fn control_id(tree: &RenderTree) -> NodeId {
        nodes(tree)
            .into_iter()
            .find(|id| tree.node_ref::<SegmentedBox>(*id).is_some())
            .expect("segmented control ada di pohon")
    }

    fn three(fonts: &Fonts, t: &Theme) -> SegmentedControl {
        segmented_control_in(
            fonts,
            t,
            [segment("Day"), segment("Week"), segment("Month")],
        )
    }

    #[test]
    fn semua_segmen_sama_lebar() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        // "Month" is wider than "Day"; a segmented control is still a grid of
        // equal cells, which is what keeps the thumb's travel constant.
        let tree = built(three(&fonts, &t));
        let c = tree
            .node_ref::<SegmentedBox>(control_id(&tree))
            .expect("node kontrol");
        let lebar: Vec<f32> = c.segment_rects().iter().map(|r| r.size.width).collect();
        assert_eq!(lebar.len(), 3);
        for w in &lebar {
            assert!(
                (w - lebar[0]).abs() < 0.01,
                "lebar segmen tidak seragam: {lebar:?}"
            );
        }
    }

    #[test]
    fn tinggi_minimal_memenuhi_hit_target_hig() {
        let fonts = Fonts::bundled_only();
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Light);
            let tree = built(three(&fonts, &t));
            let id = control_id(&tree);
            assert!(
                tree.size(id).height >= MIN_HIT_TARGET,
                "{preset:?}: tinggi {} < {MIN_HIT_TARGET}",
                tree.size(id).height
            );
        }
    }

    #[test]
    fn thumb_menempel_pada_segmen_terpilih() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(three(&fonts, &t).selected(2));
        let c = tree
            .node_ref::<SegmentedBox>(control_id(&tree))
            .expect("node kontrol");
        // No frame driver has run yet, so the thumb is already home rather than
        // gliding in from the corner.
        assert_eq!(c.active_rect(), c.segment_rects()[2]);
        assert!(!c.is_animating());
    }

    #[test]
    fn indeks_di_luar_jangkauan_dijepit_bukan_panik() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let picker = three(&fonts, &t).selected(99);
        assert_eq!(picker.active_index(), 2);
        let tree = built(picker);
        let c = tree
            .node_ref::<SegmentedBox>(control_id(&tree))
            .expect("node kontrol");
        assert_eq!(c.selected, 2);
    }

    #[test]
    fn panah_memindahkan_pilihan_dan_melewati_yang_nonaktif() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let dipilih = Rc::new(RefCell::new(Vec::<usize>::new()));
        let rekam = dipilih.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            segmented_control_in(
                &fonts,
                &t,
                [segment("A"), segment("B").disabled(true), segment("C")],
            )
            .selected(0)
            .on_select(move |i| rekam.borrow_mut().push(i)),
        );
        tree.layout(BoxConstraints::tight(BOX));

        let id = control_id(&tree);
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(id));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::ArrowRight),
                Duration::ZERO,
            )),
        );
        assert_eq!(
            *dipilih.borrow(),
            vec![2],
            "panah kanan harus melompati segmen nonaktif"
        );
    }

    #[test]
    fn panah_di_ujung_tetap_dianggap_ditangani() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, three(&fonts, &t).selected(2).on_select(|_| {}));
        tree.layout(BoxConstraints::tight(BOX));

        let id = control_id(&tree);
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(id));
        let resp = router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::ArrowRight),
                Duration::ZERO,
            )),
        );
        assert!(
            resp.handled,
            "kalau tidak ditangani, End/panah akan menggulirkan halaman di belakangnya"
        );
    }

    #[test]
    fn rtl_membalik_arti_panah() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let dipilih = Rc::new(RefCell::new(Vec::<usize>::new()));
        let rekam = dipilih.clone();
        let mut tree = RenderTree::new();
        tree.set_direction(TextDirection::Rtl);
        reconcile(
            &mut tree,
            three(&fonts, &t)
                .selected(1)
                .on_select(move |i| rekam.borrow_mut().push(i)),
        );
        tree.layout(BoxConstraints::tight(BOX));

        let id = control_id(&tree);
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(id));
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::ArrowRight),
                Duration::ZERO,
            )),
        );
        assert_eq!(
            *dipilih.borrow(),
            vec![0],
            "di RTL, panah kanan berarti segmen sebelumnya"
        );
    }

    #[test]
    fn rtl_menempatkan_segmen_pertama_di_kanan() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let mut tree = RenderTree::new();
        tree.set_direction(TextDirection::Rtl);
        reconcile(&mut tree, three(&fonts, &t));
        tree.layout(BoxConstraints::tight(BOX));
        let c = tree
            .node_ref::<SegmentedBox>(control_id(&tree))
            .expect("node kontrol");
        let r = c.segment_rects();
        assert!(
            r[0].min_x() > r[2].min_x(),
            "segmen pertama harus berada di sisi kanan pada dokumen RTL"
        );
    }

    #[test]
    fn menyeret_melintasi_segmen_ikut_memilih() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let dipilih = Rc::new(RefCell::new(Vec::<usize>::new()));
        let rekam = dipilih.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            three(&fonts, &t)
                .selected(0)
                .on_select(move |i| rekam.borrow_mut().push(i)),
        );
        tree.layout(BoxConstraints::tight(BOX));

        let id = control_id(&tree);
        let asal = tree.global_offset(id);
        let kotak: Vec<Rect> = tree
            .node_ref::<SegmentedBox>(id)
            .expect("node kontrol")
            .segment_rects()
            .to_vec();
        let titik = |r: Rect| Point::new(asal.x + r.center().x, asal.y + r.center().y);

        let mut router = InputRouter::new();
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Down, titik(kotak[0]), Duration::ZERO)
                    .button(PointerButton::Primary),
            ),
        );
        router.dispatch(
            &mut tree,
            &Event::Pointer(PointerEvent::new(
                PointerPhase::Move,
                titik(kotak[2]),
                Duration::from_millis(40),
            )),
        );
        assert_eq!(
            *dipilih.borrow(),
            vec![2],
            "jempol harus mengikuti jari melintasi segmen (rasa iOS)"
        );
    }

    #[test]
    fn setiap_segmen_adalah_radio_yang_menyebut_posisinya() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(three(&fonts, &t).selected(1).label("Rentang kalender"));
        let a11y = tree.access_tree(None);

        let grup = a11y
            .entries()
            .iter()
            .find(|e| e.node.role == AccessRole::Group)
            .expect("kontrol punya node grup");
        assert_eq!(grup.node.label.as_deref(), Some("Rentang kalender"));

        let radio: Vec<_> = a11y
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::RadioButton)
            .collect();
        assert_eq!(radio.len(), 3, "setiap pilihan diumumkan sendiri");
        assert_eq!(radio[1].node.toggled, Some(AccessToggled::On));
        assert_eq!(radio[0].node.toggled, Some(AccessToggled::Off));
        assert_eq!(radio[1].node.position_in_set, Some(2));
        assert_eq!(radio[1].node.size_of_set, Some(3));

        // The one thing that must *not* happen: being announced as a tab.
        assert!(
            a11y.entries()
                .iter()
                .all(|e| e.node.role != AccessRole::Tab && e.node.role != AccessRole::TabList),
            "kontrol segmen bukan tab list:\n{}",
            a11y.dump()
        );
    }

    #[test]
    fn tanpa_segmen_aktif_kontrol_tidak_bisa_difokus() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(segmented_control_in(
            &fonts,
            &t,
            [segment("A").disabled(true), segment("B").disabled(true)],
        ));
        let c = tree
            .node_ref::<SegmentedBox>(control_id(&tree))
            .expect("node kontrol");
        assert_eq!(c.focus_policy(), FocusPolicy::NONE);
    }

    #[test]
    fn benar_di_kedua_preset_dan_kedua_appearance() {
        // Not one value is hardcoded here: the test only asserts that every
        // cell of the (preset × appearance) matrix answers with something
        // usable, which is what "correct in both presets" means in practice.
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let s = SegmentedStyle::from_theme(&t);
                assert!(s.track.is_visible(), "{preset:?}/{appearance:?}");
                assert!(s.thumb_is_visible(), "{preset:?}/{appearance:?}");
                assert!(s.min_height >= MIN_HIT_TARGET);
                assert_eq!(
                    s.segment_corners.style, t.radius.style,
                    "bentuk sudut harus mengikuti preset (squircle vs arc)"
                );
                assert!(
                    s.thumb.corners.radii.max() <= t.radius.md,
                    "radius jempol harus lebih kecil dari radius sumur"
                );
            }
        }
    }

    #[test]
    fn thumb_rect_adalah_fungsi_murni() {
        let t = theme();
        let s = SegmentedStyle::from_theme(&t);
        let segmen = Rect::new(10.0, 4.0, 100.0, 36.0);
        assert_eq!(s.thumb_rect(segmen), segmen.deflate(s.thumb_inset));
    }

    #[test]
    fn mengganti_pilihan_saat_bergerak_adalah_retarget() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, three(&fonts, &t).selected(0));
        tree.layout(BoxConstraints::tight(BOX));

        let tick = Tick::manual(Duration::from_millis(8), Motion::Full);
        // The first tick tells the node an animation driver exists.
        advance(&mut tree, &tick);

        reconcile(&mut tree, three(&fonts, &t).selected(2));
        tree.layout(BoxConstraints::tight(BOX));
        let id = control_id(&tree);
        assert!(
            tree.node_ref::<SegmentedBox>(id)
                .expect("node kontrol")
                .is_animating(),
            "pilihan baru harus meluncur, bukan melompat"
        );

        // …and `settle` gets a snapshot to the final frame rather than a spring
        // caught mid-flight.
        settle(&mut tree);
        assert!(!is_animating(&tree));
        let c = tree.node_ref::<SegmentedBox>(id).expect("node kontrol");
        assert_eq!(c.active_rect(), c.segment_rects()[2]);
    }

    #[test]
    fn segmen_hanya_ikon_tetap_punya_nama() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(segmented_control_in(
            &fonts,
            &t,
            [
                segment("List view").icon(IconName::Menu).icon_only(true),
                segment("Grid view").icon(IconName::Check).icon_only(true),
            ],
        ));
        let a11y = tree.access_tree(None);
        assert!(
            a11y.find_label("List view").is_some(),
            "segmen hanya-ikon wajib tetap punya nama:\n{}",
            a11y.dump()
        );
    }

    #[test]
    fn kontrol_kosong_tidak_panik() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let kosong: Vec<Segment> = Vec::new();
        let tree = built(segmented_control_in(&fonts, &t, kosong));
        let c = tree
            .node_ref::<SegmentedBox>(control_id(&tree))
            .expect("node kontrol");
        assert!(c.is_empty());
        assert_eq!(c.tetangga(0, 1), None);
        assert_eq!(c.ujung(1), None);
        assert_eq!(c.segment_at(Point::new(1.0, 1.0)), None);
        assert_eq!(c.segment_near(Point::new(1.0, 1.0)), None);
    }

    #[test]
    fn memilih_ulang_segmen_yang_sama_tidak_memanggil_callback() {
        let rt = Runtime::new();
        let hitung = rt.signal(0i32);
        let fonts = Fonts::bundled_only();
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            three(&fonts, &t)
                .selected(1)
                .on_select(move |_| hitung.set(hitung.get() + 1)),
        );
        tree.layout(BoxConstraints::tight(BOX));
        let id = control_id(&tree);
        let c = tree.node_mut_ref::<SegmentedBox>(id).expect("node kontrol");
        assert!(!c.request_select(1), "pilihan yang sama bukan perubahan");
        assert_eq!(hitung.get(), 0);
    }
}
