//! The frame around a text area: its background, its border, its focus ring —
//! and **how tall it is**.
//!
//! Two jobs, and both of them are the reason the frame exists as a node of its
//! own instead of being folded into the scroll view:
//!
//! 1. **Auto-grow.** A growing text area is one whose height is a function of
//!    its content, but a scroll view is by definition a box that fills what it
//!    is given. So the frame measures — through [`AreaLink`], the body having
//!    published its natural height — and then hands the scroll view an exact
//!    size. Two passes are enough and never more: the content height depends
//!    only on the width, and the width does not change between them.
//! 2. **The ring belongs around the field, not around the text.** The node
//!    that takes focus is the body, and the body lives *inside* the scroll
//!    view's clip and is as tall as the whole document. A ring drawn there
//!    would be clipped away and would follow the scrolling. So the body
//!    records focus on the link and the frame draws the ring — on a spring, so
//!    it grows instead of snapping on (§3.5).

use silka_core::access::{AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{FocusPolicy, HitBehavior, HitShape};
use silka_core::tree::{BoxConstraints, Decoration, FocusRing, LayoutCtx, PaintCtx, RenderNode};
use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Size};

use super::link::AreaLink;

/// How the frame looks in each of its states — **already resolved from
/// tokens**, so the node itself holds no opinion about colour (§2.7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameStyle {
    /// Background at rest.
    pub background: Color,
    /// Background while hovered.
    pub background_hover: Color,
    /// Background while focused.
    pub background_focus: Color,
    /// Border width.
    pub border_width: f32,
    /// Border colour at rest.
    pub border: Color,
    /// Border colour while focused.
    pub border_focus: Color,
    /// Corner geometry: squircle in Cupertino, arc in Tailwind — a shader
    /// parameter, never a constant (§3.6).
    pub corners: Corners,
    /// The keyboard focus ring.
    pub focus_ring: Option<FocusRing>,
}

/// The render node that frames a text area.
pub struct TextAreaFrame {
    pub(super) style: FrameStyle,
    /// Height when the content is shorter than this, logical points.
    pub(super) min_height: f32,
    /// The height it will never grow past.
    pub(super) max_height: f32,
    /// Grow with the content between `min_height` and `max_height`.
    pub(super) auto_grow: bool,
    pub(super) link: AreaLink,

    hover_t: SpringValue<f32>,
    focus_t: SpringValue<f32>,
    size: Size,
}

impl TextAreaFrame {
    /// A frame with the given look and sizing rule.
    pub(super) fn new(
        style: FrameStyle,
        min_height: f32,
        max_height: f32,
        auto_grow: bool,
        link: AreaLink,
        spring: Spring,
    ) -> Self {
        Self {
            style,
            min_height,
            max_height,
            auto_grow,
            link,
            hover_t: SpringValue::new(0.0).with_spring(spring),
            focus_t: SpringValue::new(0.0).with_spring(spring),
            size: Size::ZERO,
        }
    }

    /// The height this frame wants for a content of `content` points.
    pub fn height_for(&self, content: f32) -> f32 {
        if self.auto_grow {
            content.clamp(self.min_height, self.max_height.max(self.min_height))
        } else {
            self.min_height
        }
    }

    /// True while any of its transitions is still moving.
    pub fn is_animating(&self) -> bool {
        self.hover_t.is_animating() || self.focus_t.is_animating()
    }

    /// Progress of the focus transition, 0..1.
    pub fn focus_progress(&self) -> f32 {
        self.focus_t.position().clamp(0.0, 1.0)
    }

    /// The spring driving hover and focus.
    pub(super) fn set_spring(&mut self, spring: Spring) {
        self.hover_t.set_spring(spring);
        self.focus_t.set_spring(spring);
    }

    /// The spring currently in use.
    pub(super) fn spring(&self) -> Spring {
        self.focus_t.spring()
    }

    /// Aim the transitions at what the body reported, then advance them by one
    /// frame; true when something moved.
    ///
    /// Reading the state here rather than pushing it from the body is what
    /// keeps the body free of any knowledge about who draws the ring.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let fokus = if self.link.focused() { 1.0 } else { 0.0 };
        let hover = if self.link.hovered() { 1.0 } else { 0.0 };
        if self.focus_t.target() != fokus {
            self.focus_t.set_target(fokus);
        }
        if self.hover_t.target() != hover {
            self.hover_t.set_target(hover);
        }
        if !self.is_animating() {
            return false;
        }
        let sebelum = (self.hover_t.position(), self.focus_t.position());
        tick.advance(&mut self.hover_t);
        tick.advance(&mut self.focus_t);
        (self.hover_t.position(), self.focus_t.position()) != sebelum
    }

    /// Finish every transition instantly (tests and snapshots).
    pub fn settle(&mut self) {
        self.focus_t
            .set_target(if self.link.focused() { 1.0 } else { 0.0 });
        self.hover_t
            .set_target(if self.link.hovered() { 1.0 } else { 0.0 });
        self.hover_t.settle();
        self.focus_t.settle();
    }

    /// The decoration for the current state — the result of **spring
    /// interpolation**, not a jump between three colours.
    fn dekorasi(&self) -> Decoration {
        let hover = self.hover_t.position().clamp(0.0, 1.0);
        let fokus = self.focus_progress();
        Decoration {
            background: self
                .style
                .background
                .lerp(self.style.background_hover, hover)
                .lerp(self.style.background_focus, fokus),
            corners: self.style.corners,
            border_width: self.style.border_width,
            border_color: self.style.border.lerp(self.style.border_focus, fokus),
            shadows: silka_paint::ShadowPair::NONE,
        }
    }
}

impl RenderNode for TextAreaFrame {
    fn type_name(&self) -> &'static str {
        "TextAreaFrame"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let lebar = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            constraints.min_width
        };
        if ctx.child_count() == 0 {
            self.size = constraints.constrain(Size::new(lebar, self.min_height));
            self.link.set_viewport(self.size);
            return self.size;
        }

        let anak = ctx.child(0);
        let mut tinggi = constraints
            .constrain(Size::new(lebar, self.height_for(self.link.content())))
            .height;
        self.link.set_viewport(Size::new(lebar, tinggi));
        ctx.layout_child(anak, BoxConstraints::tight(Size::new(lebar, tinggi)));

        // The body has now published its **real** content height. If that
        // changes ours, lay out once more — and exactly once more: the content
        // height is a function of the width alone, and the width did not move
        // between the two passes, so the second answer is final.
        let lagi = constraints
            .constrain(Size::new(lebar, self.height_for(self.link.content())))
            .height;
        if (lagi - tinggi).abs() > 0.01 {
            tinggi = lagi;
            self.link.set_viewport(Size::new(lebar, tinggi));
            ctx.layout_child(anak, BoxConstraints::tight(Size::new(lebar, tinggi)));
        }

        ctx.place_child(anak, Point::ZERO);
        self.size = Size::new(lebar, tinggi);
        self.size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.dekorasi());
        ctx.paint_children();

        // The focus ring **grows** with the spring: nothing at rest, full when
        // focused. Drawn outside the box so it never covers the text (the
        // AppKit habit, and the same code path as `text_field`).
        let fokus = self.focus_progress();
        if let Some(ring) = self
            .style
            .focus_ring
            .filter(|r| fokus > 0.0 && r.width > 0.0)
        {
            let tebal = ring.width * fokus;
            let kotak = ctx.local_bounds().deflate(Insets::all(-tebal));
            let corners = Corners::new(
                CornerRadii::all(self.style.corners.radii.max() + tebal),
                self.style.corners.style,
            );
            ctx.quad(
                Quad::new(kotak)
                    .corners(corners)
                    .border(tebal, ring.color.with_alpha(ring.color.a * fokus)),
            );
        }
    }

    fn access(&self, node: &mut AccessNode) {
        // Structural on purpose: the field a screen reader announces is the
        // body, which carries the role, the value, and the caret. A second
        // node here would make VoiceOver read the same field twice.
        node.role = AccessRole::Container;
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.style.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        // The body is the Tab stop, not the frame: one field, one stop.
        FocusPolicy::NONE
    }
}

impl core::fmt::Debug for TextAreaFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextAreaFrame")
            .field("size", &self.size)
            .field("auto_grow", &self.auto_grow)
            .field("min_height", &self.min_height)
            .field("max_height", &self.max_height)
            .field("focus", &self.focus_progress())
            .finish()
    }
}
