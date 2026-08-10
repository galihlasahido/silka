//! The tab row: placement, spring-driven indicator, keyboard, and a11y.
//!
//! This node **owns** every decision a single tab cannot make on its own:
//!
//! - **Placement.** Tabs are laid out in order following the reading direction
//!   (§9.8), and the [`Segmented`](super::TabsVariant::Segmented) variant
//!   equalizes their widths the way `NSSegmentedControl` does. Every tab gets
//!   the same height, at least
//!   [`MIN_HIT_TARGET`](crate::MIN_HIT_TARGET) (HIG).
//! - **Indicator.** A single [`SpringValue<Rect>`] holding the rect of the
//!   selected tab; the painted shape is derived from it via
//!   [`TabsStyle::indicator_rect`]. Because it is the **rect** that is sprung,
//!   the segmented thumb and the underline bar share exactly the same motion,
//!   and a selection that changes mid-animation **carries its velocity** over
//!   (§3.5).
//! - **Keyboard.** One row = one Tab stop; inside it, left/right arrows move
//!   the selection (mirrored in RTL), Home/End jump to the ends, and disabled
//!   tabs are skipped. The focus ring is drawn around the active tab, so it
//!   glides along with the indicator.

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    Event, EventCtx, FocusEvent, FocusPolicy, KeyCode, NamedKey, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::ViewNode;
use silka_paint::{CornerRadii, Corners, Insets, Quad, Rect, Size};

use super::style::TabsStyle;

// ---------------------------------------------------------------------------
// OnSelect
// ---------------------------------------------------------------------------

/// The "tab `index` was selected" action the app entrusts to the row.
///
/// A cousin of [`silka_core::Callback`] that carries one argument; its
/// properties are identical: cheap to `Clone`, `PartialEq` by identity, and the
/// only thing it may do is **write a signal** — tree structure is the
/// view-diff's authority (§2.5).
#[derive(Clone)]
pub struct OnSelect(std::rc::Rc<dyn Fn(usize)>);

impl OnSelect {
    /// Wrap a closure.
    pub fn new(f: impl Fn(usize) + 'static) -> Self {
        Self(std::rc::Rc::new(f))
    }

    /// Run the action for tab `index`.
    pub fn call(&self, index: usize) {
        (self.0)(index)
    }
}

impl PartialEq for OnSelect {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for OnSelect {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OnSelect")
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// Render node for a tab row.
pub struct TabListBox {
    /// Visual values already resolved from the tokens.
    pub style: TabsStyle,
    /// Index of the currently active tab.
    pub selected: usize,
    /// The row's name for screen readers ("Settings section").
    pub label: Option<String>,
    /// What runs when the user picks a different tab.
    pub on_select: Option<OnSelect>,
    /// Which tabs can still be selected — length = number of tabs.
    pub enabled: Vec<bool>,

    /// Rect of the active tab; the indicator shape is derived from it at paint
    /// time.
    indicator: SpringValue<Rect>,
    /// Rect of every tab from the last layout (local coordinates).
    placed: Vec<Rect>,
    /// A layout pass has already filled [`TabListBox::placed`].
    ready: bool,
    /// Currently holding keyboard focus.
    focused: bool,
    /// Reading direction from the last layout — left/right arrows are mirrored
    /// (§9.8).
    rtl: bool,
    /// True as soon as anything has called [`TabListBox::advance`].
    driven: bool,
}

impl TabListBox {
    /// The indicator rect painted this frame (local coordinates).
    pub fn indicator_rect(&self) -> Rect {
        self.style.indicator_rect(self.indicator.position())
    }

    /// The active tab's rect as it is currently being animated.
    pub fn active_rect(&self) -> Rect {
        self.indicator.position()
    }

    /// Rect of every tab from the last layout.
    pub fn tab_rects(&self) -> &[Rect] {
        &self.placed
    }

    /// True while the indicator is still moving.
    pub fn is_animating(&self) -> bool {
        self.indicator.is_animating()
    }

    /// Currently holding keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The spring driving the indicator.
    pub fn spring(&self) -> Spring {
        self.indicator.spring()
    }

    /// Number of tabs.
    pub fn len(&self) -> usize {
        self.enabled.len()
    }

    /// True when there are no tabs at all.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_empty()
    }

    /// True when tab `index` can still be selected.
    pub fn is_enabled(&self, index: usize) -> bool {
        self.enabled.get(index).copied().unwrap_or(false)
    }

    /// Aim the indicator at `kotak`.
    ///
    /// Without a frame driver (see [`super`]) the transition becomes a jump:
    /// better an immediately correct indicator than one frozen forever at its
    /// old position.
    fn arahkan(&mut self, kotak: Rect) {
        if self.driven {
            self.indicator.set_target(kotak);
        } else {
            self.indicator.jump_to(kotak);
        }
    }

    /// Move the selection to `index` — a **retarget**, not a fresh animation.
    pub fn set_selected(&mut self, index: usize) {
        if self.selected == index {
            return;
        }
        self.selected = index;
        if let Some(kotak) = self.placed.get(index).copied() {
            self.arahkan(kotak);
        }
    }

    /// Advance the indicator by one frame; true if its rect changed.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        self.driven = true;
        if !self.indicator.is_animating() {
            return false;
        }
        let sebelum = self.indicator.position();
        tick.advance(&mut self.indicator);
        self.indicator.position() != sebelum
    }

    /// Finish the transition instantly (tests and snapshots).
    pub fn settle(&mut self) {
        self.indicator.settle();
    }

    /// The next enabled tab from `dari` in the direction of `langkah`, no wrap.
    ///
    /// It does not wrap because that is the `NSSegmentedControl` habit: a right
    /// arrow on the last tab does **not** jump back to the first, so keyboard
    /// users never lose track of where they are.
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

    /// The first enabled tab (positive `langkah`) or the last one (negative).
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

    /// Request that the selection move to `index`; true if anything ran.
    ///
    /// The node does **not** move its own selection: `selected` arrives from
    /// the app through props (a controlled component), exactly like `open` on
    /// [`crate::overlay::OverlayEntry`]. All that happens here is the callback
    /// being invoked; the next frame brings the new selection back in.
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

impl RenderNode for TabListBox {
    fn type_name(&self) -> &'static str {
        "TabList"
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
        // already forced here so the HIG hit target does not depend on what the
        // labels happen to contain.
        let ukur = BoxConstraints::new(
            0.0,
            dalam.max_width,
            self.style.min_height,
            dalam.max_height,
        );
        let mut lebar = Vec::with_capacity(n);
        let mut tinggi = self.style.min_height;
        for i in 0..n {
            let anak = ctx.child(i);
            let s = ctx.layout_child_measured(anak, ukur);
            lebar.push(s.width);
            tinggi = tinggi.max(s.height);
        }
        if self.style.equal_widths {
            let terlebar = lebar.iter().copied().fold(0.0f32, f32::max);
            lebar.iter_mut().for_each(|w| *w = terlebar);
        }

        let jarak = self.style.spacing * (n - 1) as f32;
        let isi_lebar: f32 = lebar.iter().sum::<f32>() + jarak;
        let size = constraints.constrain(Size::new(
            isi_lebar + pad.horizontal(),
            tinggi + pad.vertical(),
        ));
        let tinggi_isi = (size.height - pad.vertical()).max(0.0);

        // Pass 2 — every tab receives its rect. The tight constraints here
        // **come from measuring that very child**, so this must not become a
        // relayout boundary (the same reasoning as `TaffyBox`).
        self.placed.clear();
        let mut x = pad.left;
        for (i, w) in lebar.iter().copied().enumerate() {
            let anak = ctx.child(i);
            ctx.layout_child_measured(anak, BoxConstraints::tight(Size::new(w, tinggi_isi)));
            // Following the reading direction: in RTL the first tab sits on the
            // right (§9.8).
            let kiri = if self.rtl { size.width - x - w } else { x };
            let kotak = Rect::new(kiri, pad.top, w, tinggi_isi);
            ctx.place_child(anak, kotak.origin);
            self.placed.push(kotak);
            x += w + self.style.spacing;
        }

        // The indicator is synced to the latest geometry. If it is moving, only
        // its target changes — a selection that switches mid-animation carries
        // its velocity (§3.5). If it is at rest (e.g. the window was resized),
        // it moves along without animating: the selection did not change, only
        // its rect did.
        let aktif = self
            .placed
            .get(self.selected.min(self.placed.len().saturating_sub(1)))
            .copied();
        if let Some(kotak) = aktif {
            if !self.ready {
                // A freshly built row does not "glide in" from the top-left
                // corner: the active tab is already right where it belongs.
                self.indicator.jump_to(kotak);
                self.ready = true;
            } else if self.indicator.is_animating() {
                self.indicator.set_target(kotak);
            } else {
                self.indicator.jump_to(kotak);
            }
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.style.track);

        // Hairline across the full row (underline & enclosed) — painted before
        // the indicator so the active enclosed tab covers it.
        if let Some(warna) = self.style.rail.filter(|c| c.a > 0.0) {
            let t = self.style.rail_thickness.max(0.0);
            if t > 0.0 {
                let b = ctx.local_bounds();
                ctx.quad(
                    Quad::new(Rect::new(b.min_x(), b.max_y() - t, b.size.width, t))
                        .background(warna),
                );
            }
        }

        if self.ready && self.style.indicator_is_visible() && !self.placed.is_empty() {
            let d = self.style.indicator;
            let kotak = self.indicator_rect();
            ctx.shadowed(
                Quad::new(kotak)
                    .background(d.background)
                    .corners(d.corners)
                    .border(d.border_width, d.border_color),
                d.shadows,
            );
        }

        ctx.paint_children();

        // The focus ring surrounds the **active tab**, not the whole row: it
        // glides along with the indicator, so the keyboard always points where
        // the eye is already looking.
        if self.focused && self.ready && !self.placed.is_empty() {
            let ring = self.style.focus_ring;
            if ring.width > 0.0 && ring.color.a > 0.0 {
                let kotak = self.active_rect().deflate(Insets::all(-ring.width));
                let corners = Corners::new(
                    CornerRadii::all(self.style.tab_corners.radii.max() + ring.width),
                    self.style.tab_corners.style,
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
        node.role = AccessRole::TabList;
        node.label.clone_from(&self.label);
        if self.ujung(1).is_some() {
            node.actions |= AccessActions::FOCUS;
        }
    }

    /// One row = **one** Tab stop; inside it, the arrow keys do the work.
    fn focus_policy(&self) -> FocusPolicy {
        if self.ujung(1).is_some() {
            FocusPolicy::FOCUSABLE
        } else {
            FocusPolicy::NONE
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            // A click on one of the tabs bubbles all the way up here (the tab
            // deliberately does not mark it handled): focus belongs to the row,
            // so this is where it is requested — exactly like
            // `NSSegmentedControl`, which is arrow-key usable right after a
            // click.
            Event::Pointer(p)
                if p.phase == PointerPhase::Down
                    && p.button == Some(PointerButton::Primary)
                    && self.ujung(1).is_some() =>
            {
                ctx.request_focus();
                ctx.handled();
            }
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                ctx.request_paint();
            }
            Event::Key(k) if k.is_pressed() && k.modifiers.is_empty() => {
                // The reading direction decides what the arrows mean: in RTL,
                // "right" means the previous tab (§9.8).
                let maju = if self.rtl { -1 } else { 1 };
                let tujuan = match k.code {
                    KeyCode::Named(NamedKey::ArrowRight) => self.tetangga(self.selected, maju),
                    KeyCode::Named(NamedKey::ArrowLeft) => self.tetangga(self.selected, -maju),
                    // Up/down arrows are deliberately unused: this row is
                    // horizontal, and swallowing vertical arrows would steal
                    // scrolling from the page behind it.
                    KeyCode::Named(NamedKey::Home) => self.ujung(1),
                    KeyCode::Named(NamedKey::End) => self.ujung(-1),
                    _ => return,
                };
                // Arrows at the ends stay "handled": without that, Home on the
                // first tab would bubble up and scroll the page.
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

impl core::fmt::Debug for TabListBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TabListBox")
            .field("variant", &self.style.variant)
            .field("selected", &self.selected)
            .field("tabs", &self.enabled.len())
            .field("indicator", &self.indicator.position())
            .field("focused", &self.focused)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props for a tab row — the view form of [`TabListBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct TabListProps {
    pub(super) style: TabsStyle,
    pub(super) selected: usize,
    pub(super) label: Option<String>,
    pub(super) on_select: Option<OnSelect>,
    pub(super) enabled: Vec<bool>,
    pub(super) spring: Spring,
}

impl ViewNode for TabListProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TabListBox {
            style: self.style,
            selected: self.selected,
            label: self.label.clone(),
            on_select: self.on_select.clone(),
            enabled: self.enabled.clone(),
            indicator: SpringValue::new(Rect::new(0.0, 0.0, 0.0, 0.0)).with_spring(self.spring),
            placed: Vec::new(),
            ready: false,
            focused: false,
            rtl: false,
            driven: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TabListBox>()
            .expect("tipe view sama berarti tipe render node sama");
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
            // The indicator shifts: it needs a repaint **and** a next frame for
            // as long as its spring has not settled.
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.indicator.spring() != self.spring {
            n.indicator.set_spring(self.spring);
        }
        // The callback is always replaced without comparison (see
        // `InteractiveProps`).
        n.on_select.clone_from(&self.on_select);
        dirty
    }
}
