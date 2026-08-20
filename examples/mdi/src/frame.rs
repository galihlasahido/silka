//! One internal frame: chrome, titlebar, eight resize edges, and the node that
//! owns the window's keyboard and accessibility contract.
//!
//! ## Why the window is a **focus scope**, and only when it is in front
//!
//! The hard requirement of an MDI desktop is that the keyboard belongs to the
//! window in front. The framework already has the mechanism —
//! [`FocusPolicy::scope`] is what keeps Tab inside a dialog — and this module
//! uses it with one twist that dialogs never need, because a dialog is always
//! the frontmost thing there is:
//!
//! | The window is… | `focusable` | `scope` | `skip_subtree` |
//! |---|---|---|---|
//! | active (frontmost, not minimized) | ✓ — a click on its background lands here | ✓ — Tab cycles inside it | ✗ |
//! | visible but behind | ✓ — so a click can still bring it forward | ✗ | ✓ — **Tab never walks into it** |
//! | minimized | ✗ | ✗ | ✓ |
//!
//! `skip_subtree` on a background window is the whole of "Tab must not reach
//! the window behind": [`tab_order`](silka_core::input::tab_order) abandons a
//! subtree the moment it sees that flag, so the controls of a window that is
//! not in front are not in the order at all — not disabled, not skipped one by
//! one, simply not there.
//!
//! It also covers the case the framework cannot: focus that is *already* inside
//! a window when that window drops to the back. Dropping `scope` at the same
//! moment means [`enclosing_scope`](silka_core::input::enclosing_scope) walks
//! straight past it to the root, and the root's tab order skips the window —
//! so the next Tab leaves the background window and cannot come back. (See the
//! notes in `main.rs`: an application has no way to *move* focus itself, so
//! this has to be arranged rather than commanded.)

use silka_core::access::{AccessNode, AccessRole};
use silka_core::input::{
    CursorIcon, DragPhase, DragUpdate, Event, EventCtx, FocusPolicy, HitBehavior, PointerButton,
    PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::{Key, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, LayoutCtx, MainAlign, RenderNode};
use silka_core::view::{
    column, constrained, draggable, draggable_area, expanded, row, Builder, View, ViewNode,
};
use silka_paint::{Point, Size};
use silka_theme::{ColorToken, FontToken, RadiusToken, ShadowToken, SpaceToken, Theme};
use silka_widgets::overlay::{
    overlay, Align, Anchor, Barrier, Dismiss, OverlayBuilder, Placement, Side,
};
use silka_widgets::{button, icon_button, spacer, text, text_field, ButtonVariant, IconName};

use crate::model::{DragKind, Edge, Frame, FrameId, FrameState, Mdi};

/// How wide the draggable band along each edge is.
///
/// Six points: wide enough to hit with a mouse on the first try, narrow enough
/// that it never steals a click meant for the content.
pub const RESIZE_BAND: f32 = 6.0;

/// The a11y name of a window's titlebar.
pub fn titlebar_label(title: &str) -> String {
    format!("{title} title bar")
}

/// The a11y name of one resize edge.
pub fn edge_label(title: &str, edge: Edge) -> String {
    format!("{title} resize {}", edge.name())
}

/// The a11y name of the minimize button.
pub fn minimize_label(title: &str) -> String {
    format!("Minimize {title}")
}

/// The a11y name of the maximize/restore button.
pub fn maximize_label(title: &str, maximized: bool) -> String {
    if maximized {
        format!("Restore {title}")
    } else {
        format!("Maximize {title}")
    }
}

/// The a11y name of the close button.
pub fn close_label(title: &str) -> String {
    format!("Close {title}")
}

/// The a11y name of a window's editable line.
pub fn note_label(title: &str) -> String {
    format!("{title} note")
}

// ---------------------------------------------------------------------------
// The shell node
// ---------------------------------------------------------------------------

/// The node that **is** the window as far as focus and assistive technology are
/// concerned.
///
/// It draws nothing: the chrome is an ordinary decorated box inside it. What it
/// owns is the table in the module docs, plus one behaviour — a press on the
/// window's own background takes focus, which is how a click anywhere in a
/// background window brings it forward without every button inside it having to
/// know that windows can be stacked.
pub struct FrameShell {
    /// The window's name, announced by a screen reader.
    title: String,
    /// Frontmost and not minimized.
    active: bool,
    /// Collapsed to the taskbar.
    minimized: bool,
}

impl FrameShell {
    /// The window's title — how the desktop maps a focused node back to the
    /// window it belongs to.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The focus policy this window has right now (see the module docs).
    pub fn policy(&self) -> FocusPolicy {
        if self.minimized {
            return FocusPolicy::NONE.skip_subtree();
        }
        if self.active {
            FocusPolicy {
                focusable: true,
                scope: true,
                ..FocusPolicy::NONE
            }
        } else {
            FocusPolicy {
                focusable: true,
                ..FocusPolicy::NONE
            }
            .skip_subtree()
        }
    }
}

impl RenderNode for FrameShell {
    fn type_name(&self) -> &'static str {
        "FrameShell"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        size
    }

    fn access(&self, node: &mut AccessNode) {
        // Every internal frame is a window in its own right, and says so. This
        // is the node a screen reader's window chooser lands on, which is why
        // it carries the title rather than leaving it to the label inside the
        // titlebar.
        node.role = AccessRole::Window;
        node.label = Some(self.title.clone());
        // Minimized windows are already hidden by the closed overlay around
        // this node; saying it here too keeps the node honest when it is
        // mounted anywhere else.
        node.hidden = self.minimized;
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Opaque, so a press on the window's own background reaches this node.
        // Children are deeper in the hit path and are still offered the event
        // first, so nothing is stolen from a button.
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        self.policy()
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else { return };
        if p.phase == PointerPhase::Down && p.button == Some(PointerButton::Primary) {
            // Click-to-front, with focus as the messenger: the desktop reads
            // back which window holds focus and raises that one. It has to work
            // this way round because there is no ancestor-first (capture) phase
            // in the router — a parent cannot see a press that a child handled.
            ctx.request_focus();
            ctx.request_paint();
        }
    }
}

impl core::fmt::Debug for FrameShell {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FrameShell")
            .field("title", &self.title)
            .field("active", &self.active)
            .field("minimized", &self.minimized)
            .finish()
    }
}

/// The props of [`FrameShell`].
#[derive(Debug, Clone, PartialEq)]
pub struct ShellProps {
    title: String,
    active: bool,
    minimized: bool,
}

impl ViewNode for ShellProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(FrameShell {
            title: self.title.clone(),
            active: self.active,
            minimized: self.minimized,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<FrameShell>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;
        if n.title != self.title {
            n.title.clone_from(&self.title);
        }
        if n.active != self.active || n.minimized != self.minimized {
            n.active = self.active;
            n.minimized = self.minimized;
            // A window that changes places in the stack changes what the
            // keyboard can reach, and that is a paint-visible change too: the
            // chrome behind it is dimmer.
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

/// Wrap `child` in a window shell.
fn shell(title: &str, active: bool, minimized: bool, child: impl Into<View>) -> View {
    Builder::new(ShellProps {
        title: title.to_string(),
        active,
        minimized,
    })
    .child(child)
    .into()
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

/// One internal frame, as an overlay ready to be pushed onto the desktop layer.
///
/// The position is not computed here and not computed by the caller either: it
/// is handed to the overlay system as an [`Anchor`], the same way a popover
/// hands it the rect of its button. See [`exact`] for the one place where that
/// system has to be talked into an absolute coordinate.
pub fn internal_frame(
    t: &Theme,
    state: Signal<Mdi>,
    f: &Frame,
    active: bool,
    dragging: bool,
) -> OverlayBuilder {
    let id = f.id;
    let minimized = !f.is_visible();
    let title = f.title.clone();

    let content = column([
        titlebar(t, state, f, active),
        View::from(expanded(body(t, state, f))),
    ])
    .cross(CrossAlign::Stretch)
    .bg(ColorToken::Surface)
    .rounded(RadiusToken::Lg)
    .border_1()
    .border_color(if active {
        ColorToken::Accent
    } else {
        ColorToken::Border
    })
    // Depth is the only thing that says which window is in front on a still
    // screenshot, so the active one really is lifted further off the desktop —
    // and a window being dragged lifts further still, which is the whole of
    // "picked up" as a visual idea.
    .elevation(match (active, dragging) {
        (_, true) => ShadowToken::Xl,
        (true, false) => ShadowToken::Lg,
        (false, false) => ShadowToken::Md,
    });

    let panel = constrained(
        BoxConstraints::tight(f.rect.size),
        shell(&title, active, minimized, resize_lattice(state, f, content)),
    );

    overlay(panel)
        .key(Key::num(i64::from(id)))
        // `Panel`: only the window itself takes the pointer. Everything outside
        // it — the desktop, the windows below — keeps receiving clicks, which
        // is the difference between a floating window and a modal dialog.
        .barrier(Barrier::Panel)
        .no_backdrop()
        // Esc must not close a document window: that is a dialog's contract,
        // not a window's.
        .dismiss(Dismiss::NONE)
        .anchor(exact(f))
        .placement(exact_placement())
        // Minimize and restore are this spring: the window sinks towards the
        // taskbar and rises back out of it, and pressing minimize again halfway
        // down reverses the motion **carrying its velocity** rather than
        // starting a new animation (§3.5).
        .travel(t.space(12.0))
        // The overlay is a plain container: the window's accessible identity is
        // the `FrameShell` inside it, which is the node that also owns focus.
        .role(AccessRole::Container)
        .open(!minimized)
}

/// The anchor that puts a window at an exact desktop coordinate.
///
/// **This is a workaround, and the most concrete gap this example found.**
/// [`Placement`] can centre a panel, attach it to an anchor, or hug an edge —
/// there is no "put it exactly here", because until now nothing needed one:
/// every overlay in the framework hangs off a widget. A window hangs off
/// nothing but its own remembered rectangle.
///
/// The trick: a zero-height anchor along the window's **bottom** edge, with the
/// panel placed on [`Side::Top`] of it and no gap, resolves to precisely the
/// window's own origin. `Side::Top` rather than `Side::Bottom` because the side
/// also chooses the transition direction, and a window should sink towards the
/// taskbar when it is minimized, not fly up out of the screen.
pub fn exact(f: &Frame) -> Anchor {
    Anchor::Rect(silka_paint::Rect::new(
        f.rect.min_x(),
        f.rect.max_y(),
        f.rect.size.width,
        0.0,
    ))
}

/// The placement that goes with [`exact`].
pub fn exact_placement() -> Placement {
    Placement::anchored(Side::Top)
        .gap(0.0)
        .align(Align::Start)
        // No flipping and no shifting: the model has already clamped this
        // rectangle to the desktop, and a second opinion here would fight it.
        .flip(false)
        .shift(false)
}

/// The 3×3 lattice of resize bands with the window's content in the middle.
///
/// A lattice rather than eight absolutely positioned strips: the eight edges
/// then fall out of ordinary flex layout, they cannot overlap each other, and
/// the corners are unambiguous — the thing that makes hand-placed resize
/// handles fiddly is deciding who owns the corner.
fn resize_lattice(state: Signal<Mdi>, f: &Frame, content: impl Into<View>) -> View {
    // A maximized window has no edges to drag: it is already the desktop.
    let live = f.state == FrameState::Normal;
    // Straight out of `Edge::ALL`, in its reading order: the top row, the two
    // sides, the bottom row.
    let e = Edge::ALL;
    column([
        band_row(state, f, live, e[0], e[1], e[2]),
        View::from(expanded(
            row([
                side_band(state, f, live, e[3]),
                View::from(expanded(content)),
                side_band(state, f, live, e[4]),
            ])
            .cross(CrossAlign::Stretch),
        )),
        band_row(state, f, live, e[5], e[6], e[7]),
    ])
    .cross(CrossAlign::Stretch)
    .into()
}

/// The top or bottom row of the lattice: corner, edge, corner.
fn band_row(
    state: Signal<Mdi>,
    f: &Frame,
    live: bool,
    start: Edge,
    middle: Edge,
    end: Edge,
) -> View {
    row([
        corner_band(state, f, live, start),
        View::from(expanded(edge_handle(state, f, live, middle))),
        corner_band(state, f, live, end),
    ])
    .cross(CrossAlign::Stretch)
    .into()
}

/// A corner: a fixed square of grab.
fn corner_band(state: Signal<Mdi>, f: &Frame, live: bool, edge: Edge) -> View {
    constrained(
        BoxConstraints::tight(Size::new(RESIZE_BAND, RESIZE_BAND)),
        edge_handle(state, f, live, edge),
    )
    .into()
}

/// A left or right band: fixed width, whatever height the row has.
fn side_band(state: Signal<Mdi>, f: &Frame, live: bool, edge: Edge) -> View {
    constrained(
        BoxConstraints::new(RESIZE_BAND, RESIZE_BAND, 0.0, f32::INFINITY),
        edge_handle(state, f, live, edge),
    )
    .into()
}

/// One resize edge.
fn edge_handle(state: Signal<Mdi>, f: &Frame, live: bool, edge: Edge) -> View {
    let id = f.id;
    let mut handle = draggable_area()
        .key(Key::text(edge.name()))
        // The cursor vocabulary has a horizontal and a vertical resize arrow
        // and nothing diagonal, so a corner borrows the axis it mostly moves
        // along. A real toolkit has `ResizeNwSe`/`ResizeNeSw`; see `main.rs`.
        .cursor(if edge.moves_left() || edge.moves_right() {
            CursorIcon::ResizeHorizontal
        } else {
            CursorIcon::ResizeVertical
        })
        .label(edge_label(&f.title, edge));
    if live {
        handle = handle.on_drag(move |g| drag(state, id, DragKind::Resize(edge), g));
    }
    handle.into()
}

/// The titlebar: a name, a drag surface, and three buttons.
fn titlebar(t: &Theme, state: Signal<Mdi>, f: &Frame, active: bool) -> View {
    let id = f.id;
    let maximized = f.state == FrameState::Maximized;
    let title = f.title.clone();

    let bar = row([
        View::from(
            text(f.title.clone())
                .font(FontToken::Headline)
                .color(if active {
                    t.color.label
                } else {
                    t.color.secondary_label
                })
                .single_line(),
        ),
        View::from(expanded(spacer())),
        View::from(
            // A chevron pointing down, matching the direction the window
            // actually travels when it is minimized.
            icon_button(IconName::ChevronDown, minimize_label(&title))
                .key("minimize")
                .variant(ButtonVariant::Ghost)
                .on_press(move || state.update(|m| m.minimize(id))),
        ),
        View::from(
            icon_button(
                if maximized {
                    IconName::Minus
                } else {
                    IconName::Plus
                },
                maximize_label(&title, maximized),
            )
            .key("maximize")
            .variant(ButtonVariant::Ghost)
            .on_press(move || state.update(|m| m.toggle_maximize(id))),
        ),
        View::from(
            icon_button(IconName::Close, close_label(&title))
                .key("close")
                .variant(ButtonVariant::Ghost)
                .on_press(move || {
                    state.update(|m| m.close(id));
                }),
        ),
    ])
    .cross(CrossAlign::Center)
    .main(MainAlign::Start)
    .spacing(t.space(1.0))
    .padding(silka_paint::Insets::symmetric(t.space(3.0), 0.0))
    .bg(if active {
        ColorToken::SurfaceHover
    } else {
        ColorToken::Surface
    });

    // The whole bar is the drag surface; the buttons sit inside it and are
    // offered the pointer first, because they are deeper in the hit path.
    draggable(bar)
        .key("titlebar")
        .cursor(CursorIcon::Grab)
        .label(titlebar_label(&title))
        // The one tab stop that moves a window: arrows nudge it, and that is
        // the whole keyboard story for direct manipulation.
        .focusable(true)
        .keyboard_step(crate::model::KEY_STEP)
        .on_drag(move |g| drag(state, id, DragKind::Move, g))
        .into()
}

/// The window's content.
fn body(t: &Theme, state: Signal<Mdi>, f: &Frame) -> View {
    let id = f.id;
    let title = f.title.clone();
    column([
        View::from(
            text(f.body.clone())
                .font(FontToken::Callout)
                .color(t.color.secondary_label),
        ),
        View::from(
            text_field(f.note.clone())
                .key("note")
                .label(note_label(&title))
                .placeholder("Type here")
                .on_change(move |s| state.update(|m| m.set_note(id, s))),
        ),
        View::from(
            button(format!("Duplicate {title}"))
                .key("duplicate")
                .variant(ButtonVariant::Secondary)
                .on_press(move || {
                    state.update(|m| {
                        let (title, body) = m
                            .get(id)
                            .map(|f| (f.title.clone(), f.body.clone()))
                            .unwrap_or_default();
                        m.open(format!("{title} copy"), body);
                    })
                }),
        ),
    ])
    .cross(CrossAlign::Start)
    .spacing(t.space(3.0))
    .p(SpaceToken::S4)
    .into()
}

/// Route one gesture phase into the model.
///
/// Every window in the desktop shares this one function, which is why "a drag
/// is measured from the rectangle it started on" is a property of the desktop
/// rather than of nine separate handles.
fn drag(state: Signal<Mdi>, id: FrameId, kind: DragKind, g: DragUpdate) {
    match g.phase {
        DragPhase::Down => state.update(|m| m.begin_drag(id, kind)),
        DragPhase::Start | DragPhase::Update => state.update(|m| m.drag_to(g.delta)),
        DragPhase::End => {
            state.update(|m| m.end_drag(g.delta, g.velocity));
        }
        DragPhase::Cancel => state.update(|m| m.cancel_drag()),
    }
}
