//! `toolbar()` — the window's action bar, and the first component in the
//! catalogue to use [`AccessRole::Toolbar`] (`KOMPONEN.md` Tier 3).
//!
//! ```
//! use silka_widgets::{button, icon_button, tool, toolbar, tool_space, IconName};
//!
//! let bar = toolbar([
//!     tool("new", "New", icon_button(IconName::Plus, "New")),
//!     tool("delete", "Delete", icon_button(IconName::Trash, "Delete")).priority(-1),
//!     tool_space(),
//!     tool("share", "Share", button("Share")),
//! ])
//! .label("Document actions");
//! # let _ = bar;
//! ```
//!
//! # Overflow is automatic, and it is decided in **layout**
//!
//! A toolbar is the one component whose contents genuinely do not fit: a window
//! narrowed to half its width has to give something up. Which items give way is
//! decided by [`fit_plan`] — a pure function over natural widths and
//! priorities — and it is run during layout, where the available width is
//! actually known.
//!
//! The item that overflows does **not** merely stop being drawn. Its wrapper
//! ([`ToolbarItemBox`]) is laid out to zero, and from that it concludes it is
//! collapsed and reports [`AccessNode::hidden`] — so it vanishes from the
//! accessibility tree together with its whole subtree, stops being a Tab stop,
//! and stops being clickable. A toolbar button that a screen reader still
//! announces while the eye cannot see it is worse than one that is simply gone.
//!
//! Because the plan is computed from the **natural** widths of every item —
//! including the collapsed ones, which are measured before anything is decided
//! — the answer does not depend on the previous frame's answer. That is what
//! stops the classic toolbar flip-flop, where hiding an item frees the space
//! that makes it fit again.
//!
//! # The overflow menu is the application's
//!
//! The `…` trigger is drawn here, but what it opens is not: a menu of the
//! hidden items belongs to [`menu`](mod@crate::menu), and only the application
//! knows what each item's action is. The seam is a signal — bind a
//! [`ToolbarState`] and the toolbar publishes the ids of whatever it collapsed
//! ([`sync`]), which is exactly the list the menu needs.
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_widgets::{button, tool, toolbar, use_toolbar_state};
//! # let rt = Runtime::new();
//! rt.build_root(|| {
//!     let state = use_toolbar_state();
//!     // Empty while everything fits; the ids of whatever gave way otherwise —
//!     // which is exactly the list a `menu` needs.
//!     let hidden: Vec<String> = state.hidden();
//!
//!     let bar = toolbar([tool("save", "Save", button("Save"))])
//!         .bind(state)
//!         .on_overflow(|| { /* open a menu built from `hidden` */ });
//!     let _ = (bar, hidden);
//! });
//! ```
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Item | Where |
//! |---|---|
//! | Correct in both presets | [`ToolbarStyle::from_theme`] |
//! | Interactive state on springs | the `…` trigger's tint; each item brings its own control's states |
//! | Full keyboard + focus ring | items keep their own Tab stops, the trigger has one, collapsed items are skipped entirely |
//! | AccessKit node | [`AccessRole::Toolbar`], at last |
//! | Dark mode | tokens only |
//! | Hit target ≥ 44pt | [`ToolbarStyle::min_height`] and the trigger's own square |
//! | Reduced motion | the trigger's tint is [`Decorative`](silka_core::animation::MotionRole::Decorative) |

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick, Tolerance};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::{use_signal, Key, Signal};
use silka_core::tree::{
    BoxConstraints, FocusRing, LayoutCtx, NodeId, PaintCtx, RenderNode, RenderTree,
};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, CornerStyle, Corners, Insets, Point, Quad, Rect, Size};
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;

// ---------------------------------------------------------------------------
// Fit plan
// ---------------------------------------------------------------------------

/// Which items survive a width, and whether the `…` trigger is needed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolbarFit {
    /// One flag per item, in item order.
    pub visible: Vec<bool>,
    /// True when at least one item was collapsed, so the trigger is shown.
    pub overflow: bool,
}

impl ToolbarFit {
    /// How many items are collapsed.
    pub fn hidden_count(&self) -> usize {
        self.visible.iter().filter(|v| !**v).count()
    }
}

/// Decide which toolbar items fit — **the whole overflow policy, as a pure
/// function**.
///
/// The rules, in the order they apply:
///
/// 1. If everything fits, everything stays and no trigger is needed. A toolbar
///    with room to spare must not grow a `…` it does not need.
/// 2. Otherwise the trigger is shown, and its width comes out of the budget
///    before anything else — so the answer never depends on whether it happened
///    to be visible last frame.
/// 3. Items give way by **lowest priority first**; ties are broken from the
///    trailing end, because the leading edge of a toolbar is where the primary
///    actions live.
/// 4. Flexible spaces never give way: they have no natural width to reclaim.
///
/// Being a pure function is not a nicety. It is what makes the plan independent
/// of the previous plan, and therefore what stops the toolbar from oscillating
/// between two layouts forever.
///
/// ```
/// use silka_widgets::toolbar::fit_plan;
///
/// let natural = [80.0, 80.0, 80.0];
/// let priority = [0, -1, 0];
/// let flexible = [false, false, false];
///
/// // Everything fits: no trigger.
/// let roomy = fit_plan(&natural, &priority, &flexible, 8.0, 500.0, 44.0);
/// assert_eq!(roomy.visible, vec![true, true, true]);
/// assert!(!roomy.overflow);
///
/// // Too narrow: the lowest-priority item is the one that leaves.
/// let tight = fit_plan(&natural, &priority, &flexible, 8.0, 230.0, 44.0);
/// assert_eq!(tight.visible, vec![true, false, true]);
/// assert!(tight.overflow);
/// ```
pub fn fit_plan(
    natural: &[f32],
    priority: &[i32],
    flexible: &[bool],
    spacing: f32,
    available: f32,
    overflow_width: f32,
) -> ToolbarFit {
    let n = natural.len();
    let mut visible = vec![true; n];
    if n == 0 {
        return ToolbarFit {
            visible,
            overflow: false,
        };
    }

    let jarak = spacing.max(0.0);
    let lebar = |visible: &[bool], dengan_pemicu: bool| -> f32 {
        let mut jumlah = 0.0f32;
        let mut hitung = 0usize;
        for (i, v) in visible.iter().enumerate() {
            if *v {
                jumlah += natural[i].max(0.0);
                hitung += 1;
            }
        }
        if dengan_pemicu {
            jumlah += overflow_width.max(0.0);
            hitung += 1;
        }
        jumlah + jarak * (hitung.saturating_sub(1)) as f32
    };

    if lebar(&visible, false) <= available {
        return ToolbarFit {
            visible,
            overflow: false,
        };
    }

    // Lowest priority first; on a tie the trailing item goes first.
    let mut urutan: Vec<usize> = (0..n)
        .filter(|i| !flexible.get(*i).copied().unwrap_or(false))
        .collect();
    urutan.sort_by(|a, b| {
        priority
            .get(*a)
            .copied()
            .unwrap_or(0)
            .cmp(&priority.get(*b).copied().unwrap_or(0))
            .then(b.cmp(a))
    });

    for i in urutan {
        if lebar(&visible, true) <= available {
            break;
        }
        visible[i] = false;
    }

    let overflow = visible.iter().any(|v| !*v);
    ToolbarFit { visible, overflow }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// What the toolbar publishes back to the application: the ids it collapsed.
///
/// A hook-owned signal, exactly like [`crate::use_list_state`]. It is optional
/// — a toolbar without one still collapses correctly; it simply cannot offer a
/// menu of what it gave up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToolbarState {
    hidden: Signal<Vec<String>>,
}

impl ToolbarState {
    /// A state owned by `runtime` — for tests, which have no build pass.
    pub fn new(runtime: &silka_core::signals::Runtime) -> Self {
        Self {
            hidden: runtime.signal(Vec::new()),
        }
    }

    /// The ids the toolbar collapsed, in item order — **tracks** when read
    /// during a build.
    pub fn hidden(&self) -> Vec<String> {
        self.hidden.get()
    }

    /// True when nothing had to give way.
    pub fn fits(&self) -> bool {
        self.hidden.get().is_empty()
    }

    /// True while the signal is still alive (its owning scope is not disposed).
    ///
    /// A render node can outlive the scope that built it for a moment; writing
    /// to a dead signal panics, so every write goes through this guard.
    pub fn is_alive(&self) -> bool {
        self.hidden.is_alive()
    }

    /// Publish what was collapsed; writes only when it actually changed.
    ///
    /// "Only when changed" is a requirement rather than an optimisation: every
    /// write schedules a frame, and republishing the same list on every layout
    /// would spin the application at 120 fps without a pixel moving (§3.5).
    fn publish(&self, ids: Vec<String>) -> bool {
        if !self.hidden.is_alive() {
            return false;
        }
        self.hidden.set_if_changed(ids)
    }
}

/// Toolbar state owned by the component being built (§2.5).
///
/// A hook: call it once per build, never inside an `if` or a loop.
pub fn use_toolbar_state() -> ToolbarState {
    ToolbarState {
        hidden: use_signal(Vec::new),
    }
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every visual value of a toolbar, already resolved from the tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToolbarStyle {
    /// Padding inside the bar's edges.
    pub padding: Insets,
    /// Gap between two items.
    pub spacing: f32,
    /// Minimum height of the bar — the HIG hit target.
    pub min_height: f32,
    /// Side of the `…` trigger's square.
    pub overflow_side: f32,
    /// Corner shape of the trigger — the tint **and** hit-testing (§3.6).
    pub overflow_corners: Corners,
    /// Diameter of one dot in the `…`.
    pub dot_size: f32,
    /// Gap between two dots.
    pub dot_gap: f32,
    /// Corner shape of a dot (always a circle, whatever the preset does).
    pub dot_corners: Corners,
    /// Colour of the dots.
    pub dot_color: Color,
    /// Hover tint over the trigger.
    pub hover: Color,
    /// Pressed tint over the trigger.
    pub pressed: Color,
    /// Keyboard focus ring (token `focus_ring`).
    pub focus_ring: FocusRing,
}

impl ToolbarStyle {
    /// Resolve every token.
    pub fn from_theme(theme: &Theme) -> Self {
        let titik = theme.space(1.0);
        Self {
            padding: Insets::symmetric(theme.space(2.0), theme.space(1.0)),
            spacing: theme.space(2.0),
            min_height: MIN_HIT_TARGET,
            overflow_side: MIN_HIT_TARGET,
            overflow_corners: theme.corners(theme.radius.sm),
            dot_size: titik,
            dot_gap: theme.space(0.75),
            // A dot is a circle in every preset: a squircle this small reads as
            // a smudge rather than as a shape.
            dot_corners: Corners::uniform(titik * 0.5, CornerStyle::Arc),
            dot_color: theme.color.secondary_label,
            hover: theme.color.surface_hover,
            pressed: theme.color.surface_pressed,
            focus_ring: FocusRing::new(theme.space(0.5), theme.color.focus_ring),
        }
    }

    /// The three dot rects inside `box_`, centred — a pure function so the
    /// glyph can be checked without a render tree.
    pub fn dot_rects(&self, box_: Rect) -> [Rect; 3] {
        let c = box_.center();
        let d = self.dot_size.max(0.0);
        let langkah = d + self.dot_gap.max(0.0);
        let kiri = c.x - langkah - d * 0.5;
        let atas = c.y - d * 0.5;
        [
            Rect::new(kiri, atas, d, d),
            Rect::new(kiri + langkah, atas, d, d),
            Rect::new(kiri + langkah * 2.0, atas, d, d),
        ]
    }
}

// ---------------------------------------------------------------------------
// Item
// ---------------------------------------------------------------------------

/// One entry in a toolbar: a control, plus what the bar needs to know about it.
///
/// The control itself is an ordinary [`View`] — a button, an icon button, a
/// segmented control, a search field. What the toolbar adds is the metadata it
/// cannot infer: an id (so the overflow menu can name the action), a label (so
/// that menu has something to read), and a priority (so the bar knows what to
/// give up first).
pub struct ToolbarItem {
    id: String,
    label: String,
    view: Option<View>,
    priority: i32,
    flexible: bool,
    key: Option<Key>,
}

/// A toolbar entry wrapping `view`.
///
/// ```
/// use silka_widgets::{button, tool};
///
/// let save = tool("save", "Save", button("Save")).priority(10);
/// assert_eq!(save.id(), "save");
/// assert_eq!(save.priority_value(), 10);
/// assert!(!save.is_space());
/// ```
pub fn tool(id: impl Into<String>, label: impl Into<String>, view: impl Into<View>) -> ToolbarItem {
    ToolbarItem {
        id: id.into(),
        label: label.into(),
        view: Some(view.into()),
        priority: 0,
        flexible: false,
        key: None,
    }
}

/// A flexible gap that pushes what follows to the far end.
///
/// The `NSToolbarFlexibleSpaceItem`, and the reason a toolbar rarely needs a
/// [`spacer`](fn@crate::spacer) of its own. A space is never collapsed: there
/// is no width to reclaim from something that has none.
///
/// ```
/// use silka_widgets::tool_space;
///
/// assert!(tool_space().is_space());
/// ```
pub fn tool_space() -> ToolbarItem {
    ToolbarItem {
        id: String::new(),
        label: String::new(),
        view: None,
        priority: i32::MAX,
        flexible: true,
        key: None,
    }
}

impl ToolbarItem {
    /// What gives way first: **lower** goes first, and the default is 0.
    pub fn priority(mut self, priority: i32) -> Self {
        if !self.flexible {
            self.priority = priority;
        }
        self
    }

    /// Identity key — required when the toolbar's contents change (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The id published when this item is collapsed.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The name the overflow menu shows.
    pub fn label_text(&self) -> &str {
        &self.label
    }

    /// This item's overflow priority.
    pub fn priority_value(&self) -> i32 {
        self.priority
    }

    /// True when this is a flexible space rather than a control.
    pub fn is_space(&self) -> bool {
        self.flexible
    }
}

impl core::fmt::Debug for ToolbarItem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToolbarItem")
            .field("id", &self.id)
            .field("priority", &self.priority)
            .field("flexible", &self.flexible)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// The item wrapper node
// ---------------------------------------------------------------------------

/// Render node for one toolbar entry.
///
/// It draws nothing. Its entire job is to know **whether it is collapsed**, and
/// it learns that the only honest way: from the constraints its parent hands
/// it. A zero-width box means "you did not fit", and from that single fact the
/// node derives everything a collapsed item owes — no a11y node, no Tab stop,
/// no hit area.
pub struct ToolbarItemBox {
    /// The id published when this item is collapsed.
    pub id: String,
    /// The name the overflow menu shows.
    pub label: String,
    /// What gives way first (lower goes first).
    pub priority: i32,
    /// True when this is a flexible space.
    pub flexible: bool,

    collapsed: bool,
}

impl ToolbarItemBox {
    /// True when the last layout gave this item no room.
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }
}

impl RenderNode for ToolbarItemBox {
    fn type_name(&self) -> &'static str {
        "ToolbarItem"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // The parent's verdict, read off the constraints rather than passed in
        // by a back channel: a node may only be told things through the
        // protocol it already has.
        self.collapsed = constraints.max_width <= 0.0;
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        if self.collapsed {
            // Still laid out, so nothing downstream keeps a stale size that
            // could be hit-tested or announced. Identical constraints on the
            // next frame cost nothing (the layout cache answers immediately).
            ctx.layout_child(child, BoxConstraints::tight(Size::ZERO));
            ctx.place_child(child, Point::ZERO);
            return Size::ZERO;
        }
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(size)
    }

    fn access(&self, node: &mut AccessNode) {
        // Structural while visible: the control inside already announces
        // itself, and a second wrapper node would make a screen reader stutter.
        node.role = AccessRole::Container;
        // Collapsed: gone, together with the whole subtree. A button the eye
        // cannot find must not be something a screen reader can still press.
        node.hidden = self.collapsed;
    }

    fn clips_children(&self) -> bool {
        // Belt and braces: a collapsed item is zero-sized *and* clipping, so
        // nothing inside it can be clicked even for one frame.
        self.collapsed
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.collapsed {
            FocusPolicy::NONE.skip_subtree()
        } else {
            FocusPolicy::NONE
        }
    }
}

impl core::fmt::Debug for ToolbarItemBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToolbarItemBox")
            .field("id", &self.id)
            .field("collapsed", &self.collapsed)
            .finish()
    }
}

/// Props for one entry — the view form of [`ToolbarItemBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct ToolbarItemProps {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) priority: i32,
    pub(crate) flexible: bool,
}

impl ViewNode for ToolbarItemProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ToolbarItemBox {
            id: self.id.clone(),
            label: self.label.clone(),
            priority: self.priority,
            flexible: self.flexible,
            collapsed: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ToolbarItemBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;
        if n.id != self.id {
            n.id.clone_from(&self.id);
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
        }
        if n.priority != self.priority || n.flexible != self.flexible {
            n.priority = self.priority;
            n.flexible = self.flexible;
            dirty |= Dirty::LAYOUT;
        }
        dirty
    }
}

// ---------------------------------------------------------------------------
// The overflow trigger node
// ---------------------------------------------------------------------------

/// Motion role of the trigger's tint under reduced-motion.
pub const OVERFLOW_TINT_MOTION: MotionRole = MotionRole::Decorative;

/// Render node for the `…` trigger.
///
/// Collapsed exactly the way an item is — a toolbar with room to spare must not
/// grow a trigger it does not need, and the way it learns that is the same:
/// zero constraints.
pub struct ToolbarOverflowBox {
    /// Visual values already resolved from the tokens.
    pub style: ToolbarStyle,
    /// The trigger's name for screen readers.
    pub label: String,
    /// What runs when the trigger is activated.
    pub on_press: Option<silka_core::Callback>,

    collapsed: bool,
    hovered: bool,
    pressed: bool,
    focused: bool,
    tint: SpringValue<Color>,
    driven: bool,
}

impl ToolbarOverflowBox {
    fn target_tint(&self) -> Color {
        if self.collapsed {
            return self.style.hover.with_alpha(0.0);
        }
        if self.pressed && self.hovered {
            self.style.pressed
        } else if self.hovered {
            self.style.hover
        } else {
            self.style.hover.with_alpha(0.0)
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

    /// True when the last layout gave the trigger no room (nothing overflowed).
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    /// Holding keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The tint painted this frame.
    pub fn tint(&self) -> Color {
        self.tint.position()
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

    fn jalankan(&mut self) {
        if self.collapsed {
            return;
        }
        if let Some(cb) = self.on_press.clone() {
            cb.call();
        }
    }
}

impl RenderNode for ToolbarOverflowBox {
    fn type_name(&self) -> &'static str {
        "ToolbarOverflow"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.collapsed = constraints.max_width <= 0.0;
        if self.collapsed {
            return Size::ZERO;
        }
        constraints.constrain(Size::new(
            self.style.overflow_side,
            self.style.overflow_side,
        ))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        if self.collapsed {
            return;
        }
        let b = ctx.local_bounds();
        let sorot = self.tint.position();
        if sorot.a > 0.0 {
            ctx.quad(
                Quad::new(b)
                    .background(sorot)
                    .corners(self.style.overflow_corners),
            );
        }
        if self.style.dot_color.a > 0.0 {
            for titik in self.style.dot_rects(b) {
                ctx.quad(
                    Quad::new(titik)
                        .background(self.style.dot_color)
                        .corners(self.style.dot_corners),
                );
            }
        }
        if self.focused && self.style.focus_ring.is_visible() {
            ctx.quad(
                Quad::new(b)
                    .corners(self.style.overflow_corners)
                    .border(self.style.focus_ring.width, self.style.focus_ring.color),
            );
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Button;
        node.label = Some(self.label.clone());
        node.hidden = self.collapsed;
        if !self.collapsed {
            node.actions |= AccessActions::CLICK | AccessActions::FOCUS;
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.style.overflow_corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        if self.collapsed {
            HitBehavior::Ignore
        } else {
            HitBehavior::Opaque
        }
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.collapsed {
            FocusPolicy::NONE
        } else {
            FocusPolicy::FOCUSABLE
        }
    }

    fn cursor(&self) -> Option<CursorIcon> {
        (!self.collapsed).then_some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if self.collapsed {
            return;
        }
        match event {
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                ctx.request_paint();
            }
            Event::Key(k)
                if k.is_pressed()
                    && k.modifiers.is_empty()
                    && matches!(
                        k.code,
                        KeyCode::Named(NamedKey::Enter) | KeyCode::Named(NamedKey::Space)
                    ) =>
            {
                ctx.handled();
                self.jalankan();
            }
            Event::Pointer(p) => match p.phase {
                PointerPhase::Enter if !self.hovered => {
                    self.hovered = true;
                    self.arahkan();
                    ctx.request_paint();
                    ctx.request_animation();
                }
                PointerPhase::Leave if self.hovered || self.pressed => {
                    self.hovered = false;
                    self.arahkan();
                    ctx.request_paint();
                    ctx.request_animation();
                }
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    self.pressed = true;
                    self.arahkan();
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.handled();
                    ctx.request_paint();
                    ctx.request_animation();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let di_dalam = self
                        .style
                        .overflow_corners
                        .contains(ctx.size(), ctx.local());
                    let jadi = self.pressed && di_dalam;
                    self.pressed = false;
                    self.arahkan();
                    ctx.release_pointer();
                    ctx.handled();
                    ctx.request_paint();
                    ctx.request_animation();
                    if jadi {
                        self.jalankan();
                    }
                }
                PointerPhase::Cancel if self.pressed => {
                    self.pressed = false;
                    self.arahkan();
                    ctx.request_paint();
                    ctx.request_animation();
                }
                _ => {}
            },
            _ => {}
        }
    }
}

impl core::fmt::Debug for ToolbarOverflowBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToolbarOverflowBox")
            .field("collapsed", &self.collapsed)
            .field("focused", &self.focused)
            .finish()
    }
}

/// Props for the `…` trigger — the view form of [`ToolbarOverflowBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct ToolbarOverflowProps {
    pub(crate) style: ToolbarStyle,
    pub(crate) label: String,
    pub(crate) on_press: Option<silka_core::Callback>,
    pub(crate) spring: Spring,
}

impl ViewNode for ToolbarOverflowProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ToolbarOverflowBox {
            style: self.style,
            label: self.label.clone(),
            on_press: self.on_press.clone(),
            collapsed: false,
            hovered: false,
            pressed: false,
            focused: false,
            tint: SpringValue::new(self.style.hover.with_alpha(0.0))
                .with_spring(self.spring)
                .with_tolerance(Tolerance::COLOR)
                .decorative(),
            driven: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ToolbarOverflowBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;
        if n.style != self.style {
            n.style = self.style;
            n.arahkan();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.tint.spring() != self.spring {
            n.tint.set_spring(self.spring);
        }
        n.on_press.clone_from(&self.on_press);
        dirty
    }
}

// ---------------------------------------------------------------------------
// The bar node
// ---------------------------------------------------------------------------

/// What the bar needs to know about one entry in order to lay it out.
///
/// It lives on the **bar**, not on the item, because layout has no way to read
/// a child node's own fields ([`LayoutCtx`] deliberately offers no such door)
/// — and because the bar is the only party that can compare items to each
/// other anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolbarMeta {
    /// The id published when this item is collapsed.
    pub id: String,
    /// What gives way first (lower goes first).
    pub priority: i32,
    /// True when this is a flexible space rather than a control.
    pub flexible: bool,
}

/// Render node for the bar itself.
///
/// The children are, in order: one [`ToolbarItemBox`] per entry, then exactly
/// one [`ToolbarOverflowBox`]. That "exactly one, always last" invariant is
/// what lets layout address the trigger without searching for it.
pub struct ToolbarBox {
    /// Visual values already resolved from the tokens.
    pub style: ToolbarStyle,
    /// The bar's name for screen readers.
    pub label: Option<String>,
    /// Where the collapsed ids are published (optional).
    pub state: Option<ToolbarState>,
    /// One entry's worth of layout metadata, in child order.
    pub items: Vec<ToolbarMeta>,

    fit: ToolbarFit,
    placed: Vec<Rect>,
    trigger: Rect,
    rtl: bool,
}

impl ToolbarBox {
    /// The plan the last layout arrived at.
    pub fn fit(&self) -> &ToolbarFit {
        &self.fit
    }

    /// Rect of every item from the last layout (collapsed ones are empty).
    pub fn item_rects(&self) -> &[Rect] {
        &self.placed
    }

    /// The `…` trigger's rect (empty when nothing overflowed).
    pub fn trigger_rect(&self) -> Rect {
        self.trigger
    }

    /// True when the last layout mirrored the bar.
    pub fn is_rtl(&self) -> bool {
        self.rtl
    }
}

impl RenderNode for ToolbarBox {
    fn type_name(&self) -> &'static str {
        "Toolbar"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let total = ctx.child_count();
        self.placed.clear();
        self.trigger = Rect::new(0.0, 0.0, 0.0, 0.0);
        if total == 0 {
            self.fit = ToolbarFit::default();
            return constraints.smallest();
        }
        // The last child is the trigger; everything before it is an item.
        let n = total - 1;

        let pad = self.style.padding;
        let dalam = constraints.deflate(pad).loosen();
        let ukur = BoxConstraints::new(0.0, f32::INFINITY, self.style.min_height, dalam.max_height);

        // Pass 1 — natural widths, for **every** item including the ones that
        // were collapsed last frame. Measuring them all is what keeps the plan
        // independent of the previous plan.
        let mut alami = Vec::with_capacity(n);
        let mut prioritas = Vec::with_capacity(n);
        let mut lentur = Vec::with_capacity(n);
        let mut tinggi = self.style.min_height;
        for i in 0..n {
            let anak = ctx.child(i);
            let meta = self.items.get(i);
            let p = meta.map(|m| m.priority).unwrap_or(0);
            let l = meta.map(|m| m.flexible).unwrap_or(false);
            prioritas.push(p);
            lentur.push(l);
            let s = ctx.layout_child_measured(anak, ukur);
            alami.push(if l { 0.0 } else { s.width });
            tinggi = tinggi.max(s.height);
        }
        let pemicu = ctx.child(n);
        let sp = ctx.layout_child_measured(pemicu, ukur);
        tinggi = tinggi.max(sp.height);

        let tersedia = if constraints.has_bounded_width() {
            (constraints.max_width - pad.horizontal()).max(0.0)
        } else {
            f32::INFINITY
        };
        self.fit = fit_plan(
            &alami,
            &prioritas,
            &lentur,
            self.style.spacing,
            tersedia,
            self.style.overflow_side,
        );

        let size = constraints.constrain(Size::new(
            if tersedia.is_finite() {
                constraints.max_width
            } else {
                alami.iter().sum::<f32>()
                    + self.style.spacing * n.saturating_sub(1) as f32
                    + pad.horizontal()
            },
            tinggi + pad.vertical(),
        ));
        let tinggi_isi = (size.height - pad.vertical()).max(0.0);
        let isi_lebar = (size.width - pad.horizontal()).max(0.0);

        // How much slack the flexible spaces share out.
        let terpakai: f32 = alami
            .iter()
            .enumerate()
            .filter(|(i, _)| self.fit.visible[*i])
            .map(|(_, w)| *w)
            .sum::<f32>()
            + if self.fit.overflow {
                self.style.overflow_side
            } else {
                0.0
            };
        let tampil =
            self.fit.visible.iter().filter(|v| **v).count() + usize::from(self.fit.overflow);
        let jarak_total = self.style.spacing * tampil.saturating_sub(1) as f32;
        let ruang_lentur = lentur
            .iter()
            .enumerate()
            .filter(|(i, l)| **l && self.fit.visible[*i])
            .count();
        let sisa = (isi_lebar - terpakai - jarak_total).max(0.0);
        let per_lentur = if ruang_lentur > 0 {
            sisa / ruang_lentur as f32
        } else {
            0.0
        };

        // Pass 2 — place. Tight constraints here come from measuring these very
        // children, so they must not turn them into relayout boundaries.
        let mut x = pad.left;
        let mut pertama = true;
        for i in 0..n {
            let anak = ctx.child(i);
            if !self.fit.visible[i] {
                ctx.layout_child_measured(anak, BoxConstraints::tight(Size::ZERO));
                ctx.place_child(anak, Point::new(pad.left, pad.top));
                self.placed.push(Rect::new(pad.left, pad.top, 0.0, 0.0));
                continue;
            }
            if !pertama {
                x += self.style.spacing;
            }
            pertama = false;
            let w = if lentur[i] { per_lentur } else { alami[i] };
            ctx.layout_child_measured(anak, BoxConstraints::tight(Size::new(w, tinggi_isi)));
            let kiri = if self.rtl { size.width - x - w } else { x };
            let kotak = Rect::new(kiri, pad.top, w, tinggi_isi);
            ctx.place_child(anak, kotak.origin);
            self.placed.push(kotak);
            x += w;
        }

        if self.fit.overflow {
            if !pertama {
                x += self.style.spacing;
            }
            let w = self.style.overflow_side;
            ctx.layout_child_measured(pemicu, BoxConstraints::tight(Size::new(w, tinggi_isi)));
            let kiri = if self.rtl { size.width - x - w } else { x };
            self.trigger = Rect::new(kiri, pad.top, w, tinggi_isi);
            ctx.place_child(pemicu, self.trigger.origin);
        } else {
            ctx.layout_child_measured(pemicu, BoxConstraints::tight(Size::ZERO));
            ctx.place_child(pemicu, Point::new(pad.left, pad.top));
        }
        size
    }

    fn access(&self, node: &mut AccessNode) {
        // The role that has existed in the vocabulary since day one and had no
        // user until now.
        node.role = AccessRole::Toolbar;
        node.label.clone_from(&self.label);
    }

    fn clips_children(&self) -> bool {
        // A bar squeezed below its own content must not leak buttons over the
        // window behind it.
        true
    }
}

impl core::fmt::Debug for ToolbarBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToolbarBox")
            .field("items", &self.placed.len())
            .field("hidden", &self.fit.hidden_count())
            .field("rtl", &self.rtl)
            .finish()
    }
}

/// Props for the bar — the view form of [`ToolbarBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct ToolbarProps {
    pub(crate) style: ToolbarStyle,
    pub(crate) label: Option<String>,
    pub(crate) state: Option<ToolbarState>,
    pub(crate) items: Vec<ToolbarMeta>,
}

impl ViewNode for ToolbarProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ToolbarBox {
            style: self.style,
            label: self.label.clone(),
            state: self.state,
            items: self.items.clone(),
            fit: ToolbarFit::default(),
            placed: Vec::new(),
            trigger: Rect::new(0.0, 0.0, 0.0, 0.0),
            rtl: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ToolbarBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;
        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.items != self.items {
            n.items.clone_from(&self.items);
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        n.state = self.state;
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Dart-style builder for a toolbar (§2.5).
pub struct Toolbar {
    theme: Theme,
    items: Vec<ToolbarItem>,
    style: Option<ToolbarStyle>,
    label: Option<String>,
    overflow_label: Option<String>,
    on_overflow: Option<silka_core::Callback>,
    state: Option<ToolbarState>,
    spring: Spring,
    key: Option<Key>,
}

/// A toolbar holding `items` — `toolbar` (`KOMPONEN.md` Tier 3).
///
/// ```
/// use silka_widgets::{button, tool, toolbar};
///
/// let bar = toolbar([tool("save", "Save", button("Save"))]).label("Actions");
/// # let _ = bar;
/// ```
///
/// Use [`toolbar_in`] outside a build pass.
pub fn toolbar(items: impl IntoIterator<Item = ToolbarItem>) -> Toolbar {
    toolbar_in(&crate::ambient::active_theme(), items)
}

/// [`toolbar`] with the theme passed explicitly.
///
/// No text engine is needed: a toolbar measures its items rather than reading
/// them, and every item brings its own fonts.
pub fn toolbar_in(theme: &Theme, items: impl IntoIterator<Item = ToolbarItem>) -> Toolbar {
    Toolbar {
        theme: *theme,
        items: items.into_iter().collect(),
        style: None,
        label: None,
        overflow_label: None,
        on_overflow: None,
        state: None,
        spring: Spring::snappy(),
        key: None,
    }
}

impl Toolbar {
    /// The bar's name for screen readers.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The `…` trigger's accessible name (default: "More toolbar items").
    pub fn overflow_label(mut self, label: impl Into<String>) -> Self {
        self.overflow_label = Some(label.into());
        self
    }

    /// What runs when the `…` trigger is activated — usually opening a
    /// [`menu`](mod@crate::menu) built from [`ToolbarState::hidden`].
    pub fn on_overflow(mut self, f: impl Fn() + 'static) -> Self {
        self.on_overflow = Some(silka_core::Callback::new(f));
        self
    }

    /// Publish the collapsed ids into `state` (see [`use_toolbar_state`]).
    pub fn bind(mut self, state: ToolbarState) -> Self {
        self.state = Some(state);
        self
    }

    /// The spring driving the trigger's tint.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Replace every visual value at once (§2.7).
    pub fn style(mut self, style: ToolbarStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The visual values that will be used.
    pub fn resolved_style(&self) -> ToolbarStyle {
        self.style
            .unwrap_or_else(|| ToolbarStyle::from_theme(&self.theme))
    }

    /// How many entries the bar has (flexible spaces included).
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when the bar is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl From<Toolbar> for View {
    fn from(t: Toolbar) -> View {
        let style = t.resolved_style();
        let mut builder = Builder::new(ToolbarProps {
            style,
            label: t.label.clone(),
            state: t.state,
            items: t
                .items
                .iter()
                .map(|i| ToolbarMeta {
                    id: i.id.clone(),
                    priority: i.priority,
                    flexible: i.flexible,
                })
                .collect(),
        });

        for item in t.items {
            let mut b = Builder::new(ToolbarItemProps {
                id: item.id.clone(),
                label: item.label.clone(),
                priority: item.priority,
                flexible: item.flexible,
            });
            if let Some(v) = item.view {
                b = b.child(v);
            }
            if let Some(k) = item.key {
                b = b.key(k);
            }
            builder = builder.child(b);
        }

        // Always present, always last: layout addresses it by position, and a
        // trigger that came and went would take its focus state with it.
        builder = builder.child(
            Builder::new(ToolbarOverflowProps {
                style,
                label: t
                    .overflow_label
                    .unwrap_or_else(|| "More toolbar items".to_string()),
                on_press: t.on_overflow,
                spring: t.spring,
            })
            .key(Key::text("toolbar-overflow")),
        );

        if let Some(key) = t.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

// ---------------------------------------------------------------------------
// Ticking & the publish seam
// ---------------------------------------------------------------------------

/// Every toolbar-owned node in `tree`, in pre-order.
fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        if let Some(node) = tree.render(id) {
            if node.downcast_ref::<ToolbarBox>().is_some()
                || node.downcast_ref::<ToolbarOverflowBox>().is_some()
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

/// The ids the toolbar at `id` collapsed, in item order.
///
/// Reads the item wrappers rather than the plan, so what comes back is what the
/// tree actually contains — the same numbers a screen reader sees.
pub fn collapsed_ids(tree: &RenderTree, id: NodeId) -> Vec<String> {
    tree.children(id)
        .iter()
        .filter_map(|c| tree.node_ref::<ToolbarItemBox>(*c))
        .filter(|it| it.collapsed && !it.flexible)
        .map(|it| it.id.clone())
        .collect()
}

/// Publish every toolbar's collapsed ids into its bound [`ToolbarState`].
///
/// The same seam as [`crate::list::sync_virtual`], and it exists for the same
/// reason: what has to be published only exists once **this** frame's layout
/// has finished, and a signal write is how it reaches the next rebuild.
///
/// Returns [`Dirty::NONE`] when nothing changed, which is the normal case: a
/// toolbar that is not being resized publishes nothing and wakes nobody.
pub fn sync(tree: &mut RenderTree) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in nodes(tree) {
        let Some(state) = tree.node_ref::<ToolbarBox>(id).and_then(|b| b.state) else {
            continue;
        };
        if !state.is_alive() {
            continue;
        }
        if state.publish(collapsed_ids(tree, id)) {
            dirty |= Dirty::PAINT;
        }
    }
    dirty
}

/// Advance every toolbar trigger's tint by one frame.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = sync(tree);
    for id in nodes(tree) {
        let Some((berubah, bergerak)) = tree
            .node_mut_ref::<ToolbarOverflowBox>(id)
            .map(|o| (o.advance(tick), o.is_animating()))
        else {
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

/// True while any toolbar trigger's tint is still moving.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<ToolbarOverflowBox>(id)
            .is_some_and(ToolbarOverflowBox::is_animating)
    })
}

/// Finish every toolbar transition instantly (tests and snapshots).
///
/// ```
/// use silka_core::tree::RenderTree;
/// use silka_widgets::toolbar::{is_animating, settle};
///
/// let mut tree = RenderTree::new();
/// assert!(!is_animating(&tree));
/// settle(&mut tree);
/// assert!(!is_animating(&tree));
/// ```
pub fn settle(tree: &mut RenderTree) {
    sync(tree);
    for id in nodes(tree) {
        if let Some(o) = tree.node_mut_ref::<ToolbarOverflowBox>(id) {
            o.settle();
            tree.mark_needs_paint(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::signals::Runtime;
    use silka_core::tree::TextDirection;
    use silka_core::view::{fixed, reconcile};
    use silka_theme::{Appearance, Preset};

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    /// Fixed-size stand-ins for real controls: this component is about widths,
    /// and a fixed box is a width with nothing else attached.
    fn bar(t: &Theme) -> Toolbar {
        toolbar_in(
            t,
            [
                tool("new", "New", fixed(80.0, 28.0)),
                tool("delete", "Delete", fixed(80.0, 28.0)).priority(-1),
                tool("share", "Share", fixed(80.0, 28.0)),
            ],
        )
    }

    fn built(view: impl Into<View>, width: f32) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(Size::new(width, 60.0)));
        tree
    }

    fn bar_id(tree: &RenderTree) -> NodeId {
        fn cari(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
            if tree.node_ref::<ToolbarBox>(id).is_some() {
                return Some(id);
            }
            tree.children(id).iter().find_map(|c| cari(tree, *c))
        }
        cari(tree, tree.root()).expect("toolbar ada di pohon")
    }

    #[test]
    fn semua_muat_berarti_tanpa_pemicu() {
        let t = theme();
        let tree = built(bar(&t), 800.0);
        let b = tree
            .node_ref::<ToolbarBox>(bar_id(&tree))
            .expect("node bar");
        assert_eq!(b.fit().visible, vec![true, true, true]);
        assert!(!b.fit().overflow);
        assert!(
            b.trigger_rect().size.is_empty(),
            "bar yang lapang tidak boleh menumbuhkan tombol … yang tidak dibutuhkan"
        );
    }

    #[test]
    fn yang_prioritasnya_terendah_mengalah_lebih_dulu() {
        let t = theme();
        // Room for roughly two items plus the trigger.
        let tree = built(bar(&t), 240.0);
        let b = tree
            .node_ref::<ToolbarBox>(bar_id(&tree))
            .expect("node bar");
        assert!(b.fit().overflow);
        assert!(
            !b.fit().visible[1],
            "item berprioritas -1 harus yang pertama mengalah: {:?}",
            b.fit().visible
        );
    }

    #[test]
    fn item_yang_menciut_hilang_dari_pohon_aksesibilitas() {
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            toolbar_in(
                &t,
                [
                    tool(
                        "a",
                        "A",
                        fixed(200.0, 28.0)
                            .label("Tombol A")
                            .role(AccessRole::Button),
                    ),
                    tool(
                        "b",
                        "B",
                        fixed(200.0, 28.0)
                            .label("Tombol B")
                            .role(AccessRole::Button),
                    ),
                ],
            )
            .label("Aksi dokumen"),
        );
        tree.layout(BoxConstraints::tight(Size::new(320.0, 60.0)));

        let id = bar_id(&tree);
        let b = tree.node_ref::<ToolbarBox>(id).expect("node bar");
        assert!(b.fit().overflow, "harus ada yang menciut pada 320pt");
        assert_eq!(collapsed_ids(&tree, id), vec!["b".to_string()]);

        // The point of the whole design: an item the eye cannot find is not
        // something a screen reader can still press.
        let a11y = tree.access_tree(None);
        assert!(
            a11y.find_label("Tombol A").is_some(),
            "yang masih tampak tetap diumumkan:\n{}",
            a11y.dump()
        );
        assert!(
            a11y.find_label("Tombol B").is_none(),
            "yang menciut harus hilang dari pohon a11y:\n{}",
            a11y.dump()
        );
    }

    #[test]
    fn bar_membawa_peran_toolbar() {
        let t = theme();
        let tree = built(bar(&t).label("Aksi dokumen"), 800.0);
        let a11y = tree.access_tree(None);
        let bar = a11y
            .find_role(AccessRole::Toolbar)
            .expect("bar punya peran toolbar");
        assert_eq!(bar.node.label.as_deref(), Some("Aksi dokumen"));
    }

    #[test]
    fn rencana_tidak_bergantung_pada_rencana_frame_sebelumnya() {
        // The flip-flop test. Laying out the same tree at the same width twice
        // must give the identical answer — if the plan read the previous plan,
        // the second pass would differ.
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, bar(&t));
        tree.layout(BoxConstraints::tight(Size::new(240.0, 60.0)));
        let pertama = tree
            .node_ref::<ToolbarBox>(bar_id(&tree))
            .expect("node bar")
            .fit()
            .clone();

        tree.invalidate_all();
        tree.layout(BoxConstraints::tight(Size::new(240.0, 60.0)));
        let kedua = tree
            .node_ref::<ToolbarBox>(bar_id(&tree))
            .expect("node bar")
            .fit()
            .clone();
        assert_eq!(
            pertama, kedua,
            "toolbar tidak boleh berayun antar dua tata letak"
        );
    }

    #[test]
    fn ruang_lentur_tidak_pernah_menciut() {
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            toolbar_in(
                &t,
                [
                    tool("a", "A", fixed(200.0, 28.0)),
                    tool_space(),
                    tool("b", "B", fixed(200.0, 28.0)),
                ],
            ),
        );
        tree.layout(BoxConstraints::tight(Size::new(200.0, 60.0)));
        let b = tree
            .node_ref::<ToolbarBox>(bar_id(&tree))
            .expect("node bar");
        assert!(
            b.fit().visible[1],
            "ruang lentur tidak punya lebar untuk direbut kembali"
        );
    }

    #[test]
    fn ruang_lentur_mendorong_isi_ke_ujung() {
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            toolbar_in(
                &t,
                [
                    tool("a", "A", fixed(60.0, 28.0)),
                    tool_space(),
                    tool("b", "B", fixed(60.0, 28.0)),
                ],
            ),
        );
        tree.layout(BoxConstraints::tight(Size::new(600.0, 60.0)));
        let b = tree
            .node_ref::<ToolbarBox>(bar_id(&tree))
            .expect("node bar");
        let r = b.item_rects();
        assert!(
            r[2].max_x() > 500.0,
            "item terakhir harus terdorong ke ujung: {:?}",
            r[2]
        );
    }

    #[test]
    fn rtl_menempatkan_item_pertama_di_kanan() {
        let t = theme();
        let mut tree = RenderTree::new();
        tree.set_direction(TextDirection::Rtl);
        reconcile(&mut tree, bar(&t));
        tree.layout(BoxConstraints::tight(Size::new(800.0, 60.0)));
        let b = tree
            .node_ref::<ToolbarBox>(bar_id(&tree))
            .expect("node bar");
        assert!(b.is_rtl());
        let r = b.item_rects();
        assert!(r[0].min_x() > r[2].min_x());
    }

    #[test]
    fn state_menerbitkan_id_yang_menciut() {
        let rt = Runtime::new();
        let state = ToolbarState::new(&rt);
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, bar(&t).bind(state));
        tree.layout(BoxConstraints::tight(Size::new(240.0, 60.0)));

        assert!(state.fits(), "belum ada yang diterbitkan sebelum sync");
        sync(&mut tree);
        assert_eq!(state.hidden(), vec!["delete".to_string()]);

        // Publishing the same list again must not wake another frame.
        assert_eq!(sync(&mut tree), Dirty::NONE);
    }

    #[test]
    fn state_dikosongkan_lagi_saat_bar_melebar() {
        let rt = Runtime::new();
        let state = ToolbarState::new(&rt);
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, bar(&t).bind(state));
        tree.layout(BoxConstraints::tight(Size::new(240.0, 60.0)));
        sync(&mut tree);
        assert!(!state.fits());

        tree.layout(BoxConstraints::tight(Size::new(900.0, 60.0)));
        sync(&mut tree);
        assert!(state.fits(), "yang kembali muat harus kembali diumumkan");
    }

    #[test]
    fn fit_plan_menjatuhkan_dari_ujung_saat_prioritas_seri() {
        let n = [80.0, 80.0, 80.0];
        let p = [0, 0, 0];
        let l = [false, false, false];
        let rencana = fit_plan(&n, &p, &l, 8.0, 230.0, 44.0);
        assert!(
            !rencana.visible[2],
            "prioritas seri: yang di ujung mengalah lebih dulu ({:?})",
            rencana.visible
        );
        assert!(rencana.visible[0], "tepi awal adalah rumah aksi utama");
    }

    #[test]
    fn fit_plan_kosong_tidak_panik() {
        let rencana = fit_plan(&[], &[], &[], 8.0, 100.0, 44.0);
        assert!(rencana.visible.is_empty());
        assert!(!rencana.overflow);
        assert_eq!(rencana.hidden_count(), 0);
    }

    #[test]
    fn fit_plan_terlalu_sempit_menyembunyikan_semuanya() {
        let n = [80.0, 80.0];
        let p = [0, 0];
        let l = [false, false];
        let rencana = fit_plan(&n, &p, &l, 8.0, 10.0, 44.0);
        assert_eq!(rencana.visible, vec![false, false]);
        assert!(
            rencana.overflow,
            "pemicu tetap ada agar isinya bisa dicapai"
        );
    }

    #[test]
    fn titik_ellipsis_tersusun_rapi_dan_terpusat() {
        let t = theme();
        let s = ToolbarStyle::from_theme(&t);
        let kotak = Rect::new(0.0, 0.0, 44.0, 44.0);
        let d = s.dot_rects(kotak);
        assert!((d[1].center().x - kotak.center().x).abs() < 0.01);
        assert!(d[0].max_x() < d[1].min_x());
        assert!(d[1].max_x() < d[2].min_x());
        for r in d {
            assert!((r.center().y - kotak.center().y).abs() < 0.01);
        }
    }

    #[test]
    fn benar_di_kedua_preset() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let s = ToolbarStyle::from_theme(&t);
                assert!(s.min_height >= MIN_HIT_TARGET);
                assert!(s.overflow_side >= MIN_HIT_TARGET);
                assert_eq!(s.overflow_corners.style, t.radius.style);
                assert_eq!(
                    s.dot_corners.style,
                    CornerStyle::Arc,
                    "titik selalu lingkaran, apa pun presetnya"
                );
                assert!(s.dot_color.a > 0.0);
            }
        }
    }

    #[test]
    fn bar_kosong_tidak_panik() {
        let t = theme();
        let kosong: Vec<ToolbarItem> = Vec::new();
        let b = toolbar_in(&t, kosong);
        assert!(b.is_empty());
        let tree = built(b, 400.0);
        let node = tree
            .node_ref::<ToolbarBox>(bar_id(&tree))
            .expect("node bar");
        assert!(node.item_rects().is_empty());
        assert!(!node.fit().overflow);
    }
}
