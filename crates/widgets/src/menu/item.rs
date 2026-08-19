//! One row of a menu panel, and the separator between two groups of them.
//!
//! A row takes **no** part in focus navigation. Focus stays on the trigger that
//! opened the menu (see [`super::trigger`]), exactly as it does in a native
//! menu and in [`crate::select`]: there is no focus trap to install, no "focus
//! the panel that just opened" hook (which genuinely does not exist yet, see
//! [`crate::overlay`]), and not one keystroke lost between two frames.
//!
//! What a row does is therefore small and countable: report a highlight when
//! the pointer passes over it, report that its submenu should open, report an
//! activation when it is clicked, hand its own rect to the sync pass when a
//! submenu needs an anchor, and announce itself to assistive technology as a
//! menu item — checked, disabled, or with a submenu, as the case may be.

use silka_core::access::{AccessActions, AccessNode, AccessRole, AccessToggled};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, HitBehavior, HitShape, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::ViewNode;
use silka_paint::{Color, Corners, Insets, LineCap, LineJoin, Point, Quad, Rect, Size, Stroke};

use super::model::MenuMark;
use super::state::MenuIntent;
use super::MenuHandler;

/// Number of columns the submenu triangle is built from.
///
/// The paint layer knows quads, glyphs, and shadows (§3.2) — no paths and no
/// rotation — so the triangle is drawn as narrowing vertical columns. Five is
/// already smooth at 7pt.
const KOLOM: usize = 5;

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every drawing value of one menu row, **already resolved** from theme tokens.
///
/// The node holds no opinion about color (§2.6, §2.7): the Cupertino and
/// Tailwind presets swap by filling in this struct.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuRowStyle {
    /// Background at rest (normally transparent — the panel shows through).
    pub rest: Color,
    /// Background while highlighted, by keyboard or pointer.
    pub highlight: Color,
    /// Corner geometry of the row — also the shape of its hit area (§3.6).
    pub corners: Corners,
    /// Distance from the content to the row's edges.
    pub padding: Insets,
    /// Width reserved at the start of the line for the check/radio mark.
    pub leading: f32,
    /// Width reserved at the end of the line for the submenu triangle.
    pub trailing: f32,
    /// Color of the check/radio mark.
    pub mark: Color,
    /// Color of the submenu triangle.
    pub arrow: Color,
    /// Minimum row height — the HIG hit target.
    pub min_height: f32,
}

impl MenuRowStyle {
    /// The background this state resolves to — the spring's **target**, not
    /// what is drawn.
    pub fn background_for(&self, highlighted: bool) -> Color {
        if highlighted {
            self.highlight
        } else {
            self.rest
        }
    }

    /// Content insets, with both gutters accounted for.
    ///
    /// Which side each gutter grows on follows the reading direction (§9.8):
    /// the mark sits at the start of the line and the submenu triangle at its
    /// end, so an Arabic UI mirrors both without a single value being
    /// recomputed in the view layer.
    pub fn insets(&self, rtl: bool) -> Insets {
        let mut i = self.padding;
        if rtl {
            i.right += self.leading;
            i.left += self.trailing;
        } else {
            i.left += self.leading;
            i.right += self.trailing;
        }
        i
    }
}

// ---------------------------------------------------------------------------
// Geometry — pure functions, tested without a GPU
// ---------------------------------------------------------------------------

/// The columns of the submenu triangle inside `bounds`.
///
/// A pure function so the shape can be checked without a window: the columns
/// taper from full height at the base to nothing at the tip, and the tip points
/// towards the **end of the line** — right in LTR, left in RTL.
pub fn triangle_columns(bounds: Rect, rtl: bool) -> Vec<Rect> {
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return Vec::new();
    }
    let w = bounds.size.width / KOLOM as f32;
    (0..KOLOM)
        .filter_map(|i| {
            // 0 at the base, 1 at the tip.
            let t = (i as f32 + 0.5) / KOLOM as f32;
            let tinggi = bounds.size.height * (1.0 - t);
            if tinggi <= 0.0 {
                return None;
            }
            let x = if rtl {
                bounds.max_x() - (i as f32 + 1.0) * w
            } else {
                bounds.min_x() + i as f32 * w
            };
            Some(Rect::new(x, bounds.center().y - tinggi / 2.0, w, tinggi))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// The render node of one menu row.
pub struct MenuRowBox {
    style: MenuRowStyle,
    depth: usize,
    index: usize,
    label: Option<String>,
    enabled: bool,
    mark: Option<MenuMark>,
    checked: bool,
    has_submenu: bool,
    submenu_open: bool,
    highlighted: bool,
    /// The state is waiting for this row's rect (a submenu opened by keyboard).
    wants_anchor: bool,
    on_intent: Option<MenuHandler>,

    bg: SpringValue<Color>,
    hovered: bool,
    pressed: bool,
    rtl: bool,
}

impl MenuRowBox {
    fn new(props: &MenuRowProps) -> Self {
        Self {
            bg: SpringValue::new(props.style.background_for(props.highlighted))
                .with_spring(props.spring),
            style: props.style,
            depth: props.depth,
            index: props.index,
            label: props.label.clone(),
            enabled: props.enabled,
            mark: props.mark,
            checked: props.checked,
            has_submenu: props.has_submenu,
            submenu_open: props.submenu_open,
            highlighted: props.highlighted,
            wants_anchor: props.wants_anchor,
            on_intent: props.on_intent.clone(),
            hovered: false,
            pressed: false,
            rtl: false,
        }
    }

    /// This row's index within its level.
    pub fn index(&self) -> usize {
        self.index
    }

    /// The level this row lives in (0 = the root panel).
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// The background drawn this frame — the spring's position, not its target.
    pub fn background(&self) -> Color {
        self.bg.position()
    }

    /// The background the spring is heading for.
    pub fn background_target(&self) -> Color {
        self.bg.target()
    }

    /// Currently highlighted.
    pub fn is_highlighted(&self) -> bool {
        self.highlighted
    }

    /// True while the background spring is still moving.
    pub fn is_animating(&self) -> bool {
        self.bg.is_animating()
    }

    /// True when the state is still waiting for this row's rect.
    pub fn wants_anchor(&self) -> bool {
        self.wants_anchor && self.has_submenu
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
            .set_target(self.style.background_for(self.highlighted));
    }

    /// Send one intent to the application.
    ///
    /// The handler is **cloned out first**: it almost always writes a signal,
    /// and a signal write may trigger anything — what it may not do is run
    /// while this node is still borrowed `&mut`.
    pub(super) fn kirim(&self, intent: MenuIntent) {
        if let Some(h) = self.on_intent.clone() {
            h.emit(intent);
        }
    }

    /// Hand this row's rect to the state, in the coordinates of `layer`.
    ///
    /// Called by [`super::advance`] once per frame — the same seam
    /// [`crate::list::sync_virtual`] uses to publish geometry the view layer
    /// could not know when it was built.
    pub(super) fn kirim_anchor(&self, rect: Rect) {
        self.kirim(MenuIntent::SubmenuAnchor {
            depth: self.depth,
            anchor: super::state::anchor_of(rect),
        });
    }

    /// The rect of the check/radio mark, in local coordinates.
    pub fn mark_rect(&self, bounds: Rect) -> Rect {
        let d = (self.style.leading - self.style.padding.left * 0.5).clamp(0.0, bounds.size.height);
        let x = if self.rtl {
            bounds.max_x() - self.style.padding.right - d
        } else {
            bounds.min_x() + self.style.padding.left
        };
        Rect::new(x, bounds.center().y - d / 2.0, d, d)
    }

    /// The rect of the submenu triangle, in local coordinates.
    pub fn arrow_rect(&self, bounds: Rect) -> Rect {
        let w = (self.style.trailing * 0.5).max(0.0);
        let h = w * 1.6;
        let x = if self.rtl {
            bounds.min_x() + self.style.padding.left
        } else {
            bounds.max_x() - self.style.padding.right - w
        };
        Rect::new(x, bounds.center().y - h / 2.0, w, h)
    }
}

impl RenderNode for MenuRowBox {
    fn type_name(&self) -> &'static str {
        "MenuRowBox"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let insets = self.style.insets(self.rtl);
        if ctx.child_count() == 0 {
            return constraints.constrain(Size::new(constraints.min_width, self.style.min_height));
        }
        let child = ctx.child(0);
        let isi = ctx.layout_child(child, constraints.deflate(insets).loosen());
        // A row fills the panel's width (the panel hands down a tight width);
        // with no bound it falls back to the width of its own content.
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

        // The mark: a drawn check for a checkbox item, a filled dot for a radio
        // item — the same distinction AppKit makes, and the one a user reads as
        // "one of these" versus "any of these".
        if self.checked && self.style.mark.a > 0.0 {
            let kotak = self.mark_rect(bounds);
            match self.mark {
                Some(MenuMark::Check) => {
                    // The same path a checkbox draws, as one stroke command:
                    // the tick is a line, and now the paint layer can say so.
                    let tebal = (kotak.size.width / 7.0).max(1.0);
                    let jalur = crate::checkbox::check_path(kotak, 1.0);
                    if jalur.len() >= 2 {
                        let mut goresan =
                            Stroke::with_capacity(self.style.mark, tebal, jalur.len())
                                .cap(LineCap::Round)
                                .join(LineJoin::Round);
                        goresan.extend(jalur);
                        ctx.stroke(goresan);
                    }
                }
                Some(MenuMark::Radio) => {
                    let d = kotak.size.width * 0.5;
                    let dot =
                        Rect::new(kotak.center().x - d / 2.0, kotak.center().y - d / 2.0, d, d);
                    ctx.quad(
                        Quad::new(dot)
                            .background(self.style.mark)
                            .corners(Corners::uniform(d / 2.0, self.style.corners.style)),
                    );
                }
                None => {}
            }
        }

        // The submenu triangle, pointing towards the end of the line.
        if self.has_submenu && self.style.arrow.a > 0.0 {
            let kotak = self.arrow_rect(bounds);
            for kolom in triangle_columns(kotak, self.rtl) {
                ctx.quad(Quad::new(kolom).background(self.style.arrow));
            }
        }
    }

    /// Role `MenuItem`, with everything a screen reader needs to describe the
    /// row **without** looking at pixels.
    ///
    /// `toggled` is filled in only for checkable items: a plain item advertising
    /// "not checked" would make every row the user passes announce a state it
    /// does not have (the trap AccessKit documents on `Toggled`). A submenu
    /// parent advertises `Expand`/`Collapse` instead of a plain click, which is
    /// the honest description: choosing it opens a menu, it does not run
    /// anything.
    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::MenuItem;
        node.label.clone_from(&self.label);
        node.disabled = !self.enabled;
        if self.mark.is_some() {
            node.toggled = Some(AccessToggled::from(self.checked));
        }
        if self.enabled {
            if self.has_submenu {
                node.actions |= if self.submenu_open {
                    AccessActions::COLLAPSE
                } else {
                    AccessActions::EXPAND
                };
            }
            node.actions |= AccessActions::CLICK;
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.style.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        // A disabled row still absorbs the pointer: its click must not fall
        // through to the panel, let alone to the page behind it.
        HitBehavior::Opaque
    }

    fn cursor(&self) -> Option<CursorIcon> {
        self.enabled.then_some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else { return };
        let sebelum = (self.hovered, self.pressed, self.highlighted);
        match p.phase {
            PointerPhase::Enter | PointerPhase::Move => {
                self.hovered = true;
                if !self.enabled {
                    // A disabled row clears the highlight instead of taking it:
                    // native menus never highlight what cannot be chosen.
                    if self.highlighted {
                        self.highlighted = false;
                        self.retarget();
                        self.kirim(MenuIntent::Highlight {
                            depth: self.depth,
                            index: None,
                        });
                    }
                } else if !self.highlighted {
                    // Written locally **and** reported: without that, the next
                    // pointer move before the next frame would report the same
                    // thing all over again.
                    self.highlighted = true;
                    self.retarget();
                    self.kirim(MenuIntent::Highlight {
                        depth: self.depth,
                        index: Some(self.index),
                    });
                    if self.has_submenu {
                        // The pointer path already knows the rect, so a submenu
                        // opened by hover needs no extra frame.
                        self.kirim(MenuIntent::OpenSubmenu {
                            depth: self.depth,
                            index: self.index,
                            anchor: Some(super::state::anchor_of(ctx.bounds())),
                            focus_first: false,
                        });
                    }
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
                let aktif = self.pressed && di_dalam && self.enabled;
                self.pressed = false;
                ctx.release_pointer();
                ctx.handled();
                if aktif {
                    self.kirim(MenuIntent::Activate {
                        depth: self.depth,
                        index: self.index,
                    });
                }
            }
            PointerPhase::Cancel if self.pressed => self.pressed = false,
            _ => {}
        }
        if (self.hovered, self.pressed, self.highlighted) != sebelum {
            ctx.request_paint();
            // Without this the next frame never arrives and the spring freezes
            // where it stands (§3.5 "render only when dirty").
            ctx.request_animation();
        }
    }
}

impl core::fmt::Debug for MenuRowBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MenuRowBox")
            .field("depth", &self.depth)
            .field("index", &self.index)
            .field("label", &self.label)
            .field("enabled", &self.enabled)
            .field("highlighted", &self.highlighted)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props for one menu row — the view form of [`MenuRowBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct MenuRowProps {
    /// Drawing values, already resolved from tokens.
    pub style: MenuRowStyle,
    /// The level this row lives in (0 = the root panel).
    pub depth: usize,
    /// The row's index within that level.
    pub index: usize,
    /// The name a screen reader announces.
    pub label: Option<String>,
    /// Whether it can be chosen.
    pub enabled: bool,
    /// The kind of mark, for checkable items.
    pub mark: Option<MenuMark>,
    /// Whether a checkable item is on.
    pub checked: bool,
    /// Whether it owns a submenu.
    pub has_submenu: bool,
    /// Whether that submenu is open right now.
    pub submenu_open: bool,
    /// Whether it is highlighted.
    pub highlighted: bool,
    /// Whether the state is waiting for this row's rect.
    pub wants_anchor: bool,
    /// The spring that drives the background transition.
    pub spring: Spring,
    /// Where user intent is sent.
    pub on_intent: Option<MenuHandler>,
}

impl ViewNode for MenuRowProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(MenuRowBox::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<MenuRowBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        let berubah = n.style != self.style || n.highlighted != self.highlighted;
        n.style = self.style;
        n.highlighted = self.highlighted;
        if berubah {
            // New colors are **approached**, not jumped to: even a theme swap
            // travels on a spring.
            n.retarget();
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.depth != self.depth {
            n.depth = self.depth;
        }
        if n.index != self.index {
            n.index = self.index;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if (n.enabled, n.mark, n.checked, n.has_submenu, n.submenu_open)
            != (
                self.enabled,
                self.mark,
                self.checked,
                self.has_submenu,
                self.submenu_open,
            )
        {
            n.enabled = self.enabled;
            n.mark = self.mark;
            n.checked = self.checked;
            n.has_submenu = self.has_submenu;
            n.submenu_open = self.submenu_open;
            if !n.enabled {
                // A row that was just disabled must not stay stuck pressed:
                // its pointer is never coming back.
                n.pressed = false;
            }
            dirty |= Dirty::PAINT;
        }
        if self.wants_anchor && !n.wants_anchor {
            // The state is waiting for this row's rect, and only the sync pass
            // in `menu::advance` can supply it — so one more frame **must** be
            // scheduled. Without this the application goes idle believing there
            // is nothing left to do, and the submenu opened from the keyboard
            // would simply never appear.
            dirty |= Dirty::ANIMATION;
        }
        n.wants_anchor = self.wants_anchor;
        if n.bg.spring() != self.spring {
            n.bg.set_spring(self.spring);
        }
        // The handler is always replaced without comparison: the closure is
        // rebuilt on every rebuild and **captures fresh values**.
        n.on_intent.clone_from(&self.on_intent);
        dirty
    }
}

// ---------------------------------------------------------------------------
// Separator
// ---------------------------------------------------------------------------

/// The render node of a separator line between two groups of items.
#[derive(Debug)]
pub struct MenuSeparatorBox {
    color: Color,
    thickness: f32,
    inset: f32,
    height: f32,
}

impl RenderNode for MenuSeparatorBox {
    fn type_name(&self) -> &'static str {
        "MenuSeparatorBox"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        constraints.constrain(Size::new(constraints.max_width, self.height))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let b = ctx.local_bounds();
        if self.color.a <= 0.0 || self.thickness <= 0.0 {
            return;
        }
        let garis = Rect::new(
            b.min_x() + self.inset,
            b.center().y - self.thickness / 2.0,
            (b.size.width - self.inset * 2.0).max(0.0),
            self.thickness,
        );
        ctx.quad(Quad::new(garis).background(self.color));
    }

    /// A separator is announced as a separator — not as an empty menu item, and
    /// not skipped either: it is what tells a screen reader user that a group
    /// ended.
    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Separator;
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Part of the panel's surface: a click on the gap between two groups
        // must not fall through to whatever is behind the menu.
        HitBehavior::Opaque
    }
}

/// Props for a separator — the view form of [`MenuSeparatorBox`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuSeparatorProps {
    /// The line's color — always the `separator` token.
    pub color: Color,
    /// The line's thickness.
    pub thickness: f32,
    /// How far the line is inset from both ends of the row.
    pub inset: f32,
    /// The total height the separator occupies, line plus breathing room.
    pub height: f32,
}

impl ViewNode for MenuSeparatorProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(MenuSeparatorBox {
            color: self.color,
            thickness: self.thickness,
            inset: self.inset,
            height: self.height,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<MenuSeparatorBox>()
            .expect("same view type means same render node type");
        let baru = (self.color, self.thickness, self.inset, self.height);
        if (n.color, n.thickness, n.inset, n.height) == baru {
            return Dirty::NONE;
        }
        let tinggi_berubah = n.height != self.height;
        n.color = self.color;
        n.thickness = self.thickness;
        n.inset = self.inset;
        n.height = self.height;
        if tinggi_berubah {
            Dirty::LAYOUT | Dirty::PAINT
        } else {
            Dirty::PAINT
        }
    }
}
