//! A single tab: hover/press highlights that transition by **spring**, plus an
//! AccessKit node with the [`AccessRole::Tab`] role.
//!
//! Why not reuse [`silka_core::tree::Interactive`], the way `button` does?
//! Because of three differences of contract, not of taste:
//!
//! 1. **A tab is not a Tab stop.** A tab row is **one** keyboard stop (the
//!    `FocusPolicy` belongs to the row, not to each tab) — the
//!    `NSSegmentedControl` habit and the ARIA "roving tabindex" pattern at
//!    once. `Interactive` is always focusable.
//! 2. **A tab has selected state**, which must surface in the a11y tree as
//!    [`AccessToggled`] — `Interactive` has no such concept.
//! 3. **Its transitions are springs** (`KOMPONEN.md` DoD), not the color jumps
//!    `Interactive` does today.
//!
//! What this node does **not** do: paint the selected state. The active tab's
//! background is the row's indicator ([`super::list::TabListBox`]), moved by a
//! single spring — if each tab painted its own background, you would see two
//! rectangles alternately lighting up rather than one thumb gliding across.

use silka_core::access::{AccessNode, AccessRole, AccessToggled};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick, Tolerance};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusPolicy, HitBehavior, HitShape, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::ViewNode;
use silka_core::Callback;
use silka_paint::{Color, Corners, Point, Quad, Size};

/// Render node for a single tab.
pub struct TabBox {
    /// The name a screen reader announces.
    pub label: String,
    /// Its position within the row — the argument handed to `on_select`.
    pub index: usize,
    /// Currently the active tab.
    pub selected: bool,
    /// Cannot be selected (still announced, as dimmed).
    pub disabled: bool,
    /// Corner shape of the highlight — **identical** to the hit-test shape
    /// (§3.6).
    pub corners: Corners,
    /// Hover highlight (token `surface_hover`).
    pub hover: Color,
    /// Pressed highlight (token `surface_pressed`).
    pub pressed_color: Color,
    /// What runs when the user selects this tab.
    pub on_press: Option<Callback>,

    hovered: bool,
    pressed: bool,
    /// The highlight color currently in effect — this is what gets sprung.
    tint: SpringValue<Color>,
    /// True as soon as anything has called [`TabBox::advance`].
    ///
    /// See [`super`]: without a frame driver, transitions run as jumps instead
    /// of freezing halfway.
    driven: bool,
}

impl TabBox {
    /// The highlight color that should apply to the current state.
    ///
    /// The resting state is not [`Color::TRANSPARENT`] but the hover color at
    /// zero alpha: only the alpha fades, so the highlight never appears to
    /// "darken first" mid-transition.
    fn target_tint(&self) -> Color {
        if self.disabled {
            return self.hover.with_alpha(0.0);
        }
        // `pressed` survives while the captured pointer wanders outside the
        // box; the "pressed" look only applies while it is still inside
        // (AppKit).
        if self.pressed && self.hovered {
            self.pressed_color
        } else if self.hovered {
            self.hover
        } else {
            self.hover.with_alpha(0.0)
        }
    }

    /// Aim the highlight at the current state.
    fn arahkan(&mut self) {
        let target = self.target_tint();
        if self.driven {
            self.tint.set_target(target);
        } else {
            self.tint.jump_to(target);
        }
    }

    /// The highlight color painted this frame.
    pub fn tint(&self) -> Color {
        self.tint.position()
    }

    /// True while the highlight is still moving.
    pub fn is_animating(&self) -> bool {
        self.tint.is_animating()
    }

    /// The pointer is over this tab.
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// This tab is being pressed.
    pub fn is_pressed(&self) -> bool {
        self.pressed
    }

    /// Advance the highlight by one frame; true if its color changed.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        // Recorded even when nothing is moving: what matters is knowing "this
        // app has a frame driver", not "an animation is running right now".
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

    /// Run `on_press` — split out so the callback is copied out first, exactly
    /// like [`silka_core::tree::Interactive`]: it almost always writes a
    /// signal, and a signal write must not run while this node is borrowed
    /// `&mut`.
    fn pilih(&mut self) {
        if self.disabled {
            return;
        }
        if let Some(cb) = self.on_press.clone() {
            cb.call();
        }
    }
}

impl RenderNode for TabBox {
    fn type_name(&self) -> &'static str {
        "Tab"
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
        node.role = AccessRole::Tab;
        node.label = Some(self.label.clone());
        node.disabled = self.disabled;
        // Our a11y vocabulary knows on/off/mixed; for a tab that is exactly
        // what a screen reader reads out as "selected".
        node.toggled = Some(AccessToggled::from(self.selected));
        if !self.disabled {
            node.actions |= silka_core::access::AccessActions::CLICK;
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        // A disabled tab still swallows the pointer: a click on it must not
        // fall through to the row behind and select something else.
        HitBehavior::Opaque
    }

    /// **One row = one Tab stop.** Focus is held by
    /// [`super::list::TabListBox`]; left/right arrows move the selection.
    fn focus_policy(&self) -> FocusPolicy {
        FocusPolicy::NONE
    }

    fn cursor(&self) -> Option<CursorIcon> {
        (!self.disabled).then_some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else {
            return;
        };
        if self.disabled {
            if matches!(p.phase, PointerPhase::Down | PointerPhase::Up) {
                ctx.handled();
            }
            return;
        }
        match p.phase {
            PointerPhase::Enter => {
                if !self.hovered {
                    self.hovered = true;
                    self.arahkan();
                    ctx.request_paint();
                    ctx.request_animation();
                }
            }
            PointerPhase::Leave => {
                if self.hovered || self.pressed {
                    self.hovered = false;
                    self.arahkan();
                    ctx.request_paint();
                    ctx.request_animation();
                }
            }
            PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                self.pressed = true;
                self.arahkan();
                ctx.capture_pointer();
                ctx.request_paint();
                ctx.request_animation();
                // **Deliberately not marked handled**: focus has to land on the
                // row, not on the tab (see `focus_policy`), and the only way
                // the row can get it is by letting Down bubble up to the
                // ancestor. The pointer still belongs to this tab — capture has
                // nothing to do with `handled`.
            }
            PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                let di_dalam = self.corners.contains(ctx.size(), ctx.local());
                let jadi = self.pressed && di_dalam;
                self.pressed = false;
                self.arahkan();
                ctx.release_pointer();
                ctx.request_paint();
                ctx.request_animation();
                ctx.handled();
                if jadi {
                    self.pilih();
                }
            }
            // Cancelled by the OS is not a release: nothing gets selected.
            PointerPhase::Cancel if self.pressed => {
                self.pressed = false;
                self.arahkan();
                ctx.request_paint();
                ctx.request_animation();
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for TabBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TabBox")
            .field("label", &self.label)
            .field("index", &self.index)
            .field("selected", &self.selected)
            .field("disabled", &self.disabled)
            .field("tint", &self.tint.position())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props for a single tab — the view form of [`TabBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct TabProps {
    pub(super) label: String,
    pub(super) index: usize,
    pub(super) selected: bool,
    pub(super) disabled: bool,
    pub(super) corners: Corners,
    pub(super) hover: Color,
    pub(super) pressed: Color,
    pub(super) on_press: Option<Callback>,
    pub(super) spring: Spring,
}

impl ViewNode for TabProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let diam = self.hover.with_alpha(0.0);
        Box::new(TabBox {
            label: self.label.clone(),
            index: self.index,
            selected: self.selected,
            disabled: self.disabled,
            corners: self.corners,
            hover: self.hover,
            pressed_color: self.pressed,
            on_press: self.on_press.clone(),
            hovered: false,
            pressed: false,
            tint: SpringValue::new(diam)
                .with_spring(self.spring)
                .with_tolerance(Tolerance::COLOR)
                // The hover highlight explains nothing — under reduced-motion
                // it disappears entirely, rather than merely losing its bounce
                // ([`MotionRole`]).
                .decorative(),
            driven: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TabBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.index != self.index {
            n.index = self.index;
        }
        if n.selected != self.selected {
            n.selected = self.selected;
            dirty |= Dirty::PAINT;
        }
        if n.corners != self.corners {
            n.corners = self.corners;
            dirty |= Dirty::PAINT;
        }
        if n.hover != self.hover || n.pressed_color != self.pressed {
            n.hover = self.hover;
            n.pressed_color = self.pressed;
            n.arahkan();
            dirty |= Dirty::PAINT;
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                // A tab that was just disabled must not freeze in the pressed
                // state: its pointer events will never arrive again.
                n.pressed = false;
                n.hovered = false;
            }
            n.arahkan();
            dirty |= Dirty::PAINT;
        }
        if n.tint.spring() != self.spring {
            n.tint.set_spring(self.spring);
        }
        // The callback is always replaced without comparison: the closure is
        // rebuilt on every rebuild and captures fresh values (see
        // `InteractiveProps`).
        n.on_press.clone_from(&self.on_press);
        dirty
    }
}

/// Motion role of the tab highlight with respect to reduced-motion.
///
/// A constant so tests can refer to it without prying into the node's innards.
pub const TAB_TINT_MOTION: MotionRole = MotionRole::Decorative;
