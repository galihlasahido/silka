//! A single option row inside the select popup.
//!
//! Rows take **no** part in focus navigation: focus stays on the trigger (see
//! [`super::trigger`]), exactly like a native menu. A row does only three
//! things — report a highlight when the pointer passes over it, report a
//! selection when it is clicked, and announce itself to assistive technology as
//! a menu item that is checked or not.

use silka_core::access::{AccessActions, AccessNode, AccessRole, AccessToggled};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, HitBehavior, HitShape, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::ViewNode;
use silka_paint::{Color, Corners, Insets, Point, Quad, Rect, Size};

use super::{SelectHandler, SelectIntent};

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Paint values for one option row, **already resolved** from theme tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectOptionStyle {
    /// Resting background (usually transparent).
    pub rest: Color,
    /// Background while highlighted (keyboard or pointer).
    pub highlight: Color,
    /// Background of the row that is currently selected.
    pub selected: Color,
    /// Corner geometry of the row.
    pub corners: Corners,
    /// Distance from the content to the row's edges.
    pub padding: Insets,
    /// Color of the "selected" marker.
    pub marker: Color,
    /// Size of the "selected" marker.
    pub marker_size: f32,
    /// Minimum row height — the HIG hit target.
    pub min_height: f32,
}

impl SelectOptionStyle {
    /// The background that should apply — the spring's **target**, not what is
    /// drawn.
    pub fn background_for(&self, highlighted: bool, selected: bool) -> Color {
        if highlighted {
            self.highlight
        } else if selected {
            self.selected
        } else {
            self.rest
        }
    }

    /// Content insets, with room already reserved for the marker at the end of
    /// the row (§9.8).
    pub fn insets(&self, rtl: bool) -> Insets {
        let ruang = self.marker_size * 2.0;
        let mut i = self.padding;
        if rtl {
            i.left += ruang;
        } else {
            i.right += ruang;
        }
        i
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// The render node for one option row.
pub struct SelectOption {
    style: SelectOptionStyle,
    index: usize,
    label: Option<String>,
    selected: bool,
    highlighted: bool,
    on_intent: Option<SelectHandler>,

    bg: SpringValue<Color>,
    hovered: bool,
    pressed: bool,
    rtl: bool,
}

impl SelectOption {
    fn new(props: &SelectOptionProps) -> Self {
        Self {
            bg: SpringValue::new(
                props
                    .style
                    .background_for(props.highlighted, props.selected),
            )
            .with_spring(props.spring),
            style: props.style,
            index: props.index,
            label: props.label.clone(),
            selected: props.selected,
            highlighted: props.highlighted,
            on_intent: props.on_intent.clone(),
            hovered: false,
            pressed: false,
            rtl: false,
        }
    }

    /// This row's index within the list.
    pub fn index(&self) -> usize {
        self.index
    }

    /// The background drawn this frame — the spring's position, not its target.
    pub fn background(&self) -> Color {
        self.bg.position()
    }

    /// The background target the spring is heading for.
    pub fn background_target(&self) -> Color {
        self.bg.target()
    }

    /// Currently selected.
    pub fn is_selected(&self) -> bool {
        self.selected
    }

    /// Currently highlighted.
    pub fn is_highlighted(&self) -> bool {
        self.highlighted
    }

    /// True while the background spring is still moving.
    pub fn is_animating(&self) -> bool {
        self.bg.is_animating()
    }

    /// Advance the spring by one frame; true when the color moved.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let sebelum = self.bg.position();
        tick.advance(&mut self.bg);
        self.bg.position() != sebelum
    }

    /// Finish the motion immediately.
    pub fn settle(&mut self) {
        self.bg.settle();
    }

    fn retarget(&mut self) {
        self.bg
            .set_target(self.style.background_for(self.highlighted, self.selected));
    }

    fn kirim(&mut self, intent: SelectIntent) {
        if let Some(h) = self.on_intent.clone() {
            h.emit(intent);
        }
    }

    /// The "selected" marker's rect, in local coordinates.
    pub fn marker_rect(&self, bounds: Rect) -> Rect {
        let d = self.style.marker_size.max(0.0);
        let x = if self.rtl {
            self.style.padding.left
        } else {
            bounds.size.width - self.style.padding.right - d
        };
        Rect::new(x, bounds.center().y - d / 2.0, d, d)
    }
}

impl RenderNode for SelectOption {
    fn type_name(&self) -> &'static str {
        "SelectOption"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let insets = self.style.insets(self.rtl);
        if ctx.child_count() == 0 {
            return constraints.constrain(Size::new(constraints.min_width, self.style.min_height));
        }
        let child = ctx.child(0);
        let isi = ctx.layout_child(child, constraints.deflate(insets).loosen());
        // A row fills the panel's width (the list hands down a tight width);
        // with no bound, it falls back to the width of its own content.
        let lebar = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            isi.width + insets.horizontal()
        };
        let size = constraints.constrain(Size::new(
            lebar,
            (isi.height + insets.vertical()).max(self.style.min_height),
        ));
        let x = if self.rtl {
            (size.width - insets.right - isi.width).max(insets.left)
        } else {
            insets.left
        };
        let y = ((size.height - isi.height) / 2.0).max(0.0);
        ctx.place_child(child, Point::new(x, y));
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let bg = self.bg.position();
        if bg.a > 0.0 {
            ctx.quad(Quad::new(bounds).background(bg).corners(self.style.corners));
        }
        ctx.paint_children();

        // The "selected" marker: a dot shaped by the same corner preset, so the
        // Cupertino squircle and the Tailwind arc stay in step (§2.7).
        if self.selected && self.style.marker.a > 0.0 {
            let kotak = self.marker_rect(bounds);
            ctx.quad(
                Quad::new(kotak)
                    .background(self.style.marker)
                    .corners(Corners::uniform(
                        kotak.size.width / 2.0,
                        self.style.corners.style,
                    )),
            );
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::MenuItem;
        node.label.clone_from(&self.label);
        node.toggled = Some(AccessToggled::from(self.selected));
        node.actions |= AccessActions::CLICK;
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.style.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn cursor(&self) -> Option<CursorIcon> {
        Some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else { return };
        let sebelum = (self.hovered, self.pressed, self.highlighted);
        match p.phase {
            PointerPhase::Enter | PointerPhase::Move => {
                self.hovered = true;
                if !self.highlighted {
                    // The highlight is written locally **and** reported:
                    // without that, the next pointer move before the next frame
                    // would report the very same thing all over again.
                    self.highlighted = true;
                    self.retarget();
                    self.kirim(SelectIntent::Highlight(self.index));
                }
            }
            PointerPhase::Leave => self.hovered = false,
            PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                self.pressed = true;
                ctx.capture_pointer();
                ctx.handled();
            }
            PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                let di_dalam = self.style.corners.contains(ctx.size(), ctx.local());
                let aktif = self.pressed && di_dalam;
                self.pressed = false;
                ctx.release_pointer();
                ctx.handled();
                if aktif {
                    self.kirim(SelectIntent::Commit(self.index));
                }
            }
            PointerPhase::Cancel if self.pressed => self.pressed = false,
            _ => {}
        }
        if (self.hovered, self.pressed, self.highlighted) != sebelum {
            ctx.request_paint();
            ctx.request_animation();
        }
    }
}

impl core::fmt::Debug for SelectOption {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SelectOption")
            .field("index", &self.index)
            .field("label", &self.label)
            .field("selected", &self.selected)
            .field("highlighted", &self.highlighted)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props for one option row — the view form of [`SelectOption`].
#[derive(Debug, Clone, PartialEq)]
pub struct SelectOptionProps {
    /// Paint values, already resolved from tokens.
    pub style: SelectOptionStyle,
    /// This row's index.
    pub index: usize,
    /// The name a screen reader announces.
    pub label: Option<String>,
    /// Currently selected.
    pub selected: bool,
    /// Currently highlighted.
    pub highlighted: bool,
    /// The spring that drives the background transition.
    pub spring: Spring,
    /// Where user intent is sent.
    pub on_intent: Option<SelectHandler>,
}

impl ViewNode for SelectOptionProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(SelectOption::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<SelectOption>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        let berubah = n.style != self.style
            || n.selected != self.selected
            || n.highlighted != self.highlighted;
        n.style = self.style;
        n.selected = self.selected;
        n.highlighted = self.highlighted;
        if berubah {
            n.retarget();
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.index != self.index {
            n.index = self.index;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.bg.spring() != self.spring {
            n.bg.set_spring(self.spring);
        }
        n.on_intent.clone_from(&self.on_intent);
        dirty
    }
}
