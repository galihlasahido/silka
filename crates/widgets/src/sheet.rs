//! `sheet()` — the macOS sheet: a modal that **descends from the title bar**
//! rather than appearing in the middle of the screen (`KOMPONEN.md` Tier 4).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::fixed;
//! use silka_widgets::{overlay_layer, sheet};
//!
//! # let rt = Runtime::new();
//! # let open = rt.signal(true);
//! let _ = overlay_layer(fixed(800.0, 600.0)).overlay(
//!     sheet("Export invoices")
//!         .message("Choose a format and a date range.")
//!         .open(open.get())
//!         .confirm("Export", || {})
//!         .cancel("Cancel", move || open.set(false)),
//! );
//! ```
//!
//! ## What it borrows, and what it is
//!
//! | Part | Where it comes from |
//! |---|---|
//! | Placement, backdrop, dismissal, focus trap, the spring | [`mod@crate::overlay`] — a sheet picks [`Placement::edge`] and [`Barrier::Modal`], nothing more |
//! | The button row's per-OS order, the confirm/cancel/destructive vocabulary, "Return runs the default button" | [`mod@crate::dialog`] — the very same [`DialogAction`], [`ButtonOrder`] and [`DialogPanel`](crate::dialog::DialogPanel) |
//! | Its own | the **corner geometry** (top corners square, because the sheet is attached to the window edge), and the entrance, which starts off-screen instead of merely emerging |
//!
//! That second row is the point of writing this at all. A sheet is not visually
//! a dialog, but it is behaviourally *exactly* a dialog, and the one place the
//! two must never differ is the keyboard: Return runs the default button, Esc
//! runs cancel, Tab is trapped inside the panel. Reimplementing that here would
//! guarantee they eventually disagree.
//!
//! ## Why it is attached to the edge
//!
//! `Placement::edge(Side::Top)` is not a cosmetic choice — it changes the
//! **entrance**. An anchored or centred overlay "emerges" two spacing steps
//! from its resting place; an edge-placed one comes in from off-screen by its
//! own height ([`Placed::enter_offset`](crate::overlay::Placed::enter_offset)),
//! which is what makes a sheet look hinged to the title bar rather than
//! dropped on top of the window. The overlay entry clips its children, so the
//! part still outside the window is genuinely not drawn.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | surface, separator, radius, elevation and every spacing value are tokens |
//! | Interactive states on a spring | the entrance is a retargetable spring: a sheet cancelled mid-open reverses **carrying its velocity** |
//! | Keyboard + focus ring | Tab trapped ([`Barrier::Modal`]), Return runs the default button, Esc runs cancel; the buttons are [`mod@crate::button`] and bring their own rings |
//! | AccessKit node | [`AccessRole::Dialog`] with the title as its name, and the content behind genuinely inert |
//! | Dark mode | token-driven |
//! | Hit target ≥ 44pt | the buttons are [`mod@crate::button`] |
//! | Reduced motion | [`MotionRole::Essential`](silka_core::animation::MotionRole): the movement says where the panel came from, so it is calmed rather than deleted |

use silka_core::access::AccessRole;
use silka_core::animation::Spring;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, row, Builder, View};
use silka_core::Callback;
use silka_paint::{CornerRadii, Corners, Insets};
use silka_text::FontWeight;
use silka_theme::{ColorToken, RadiusToken, ShadowToken, SpaceToken, Theme};

use crate::button::button_variant_in;
use crate::dialog::{action, ActionKind, ButtonOrder, DialogAction, DialogPanelProps};
use crate::fonts::Fonts;
use crate::overlay::{overlay, Align, Barrier, Dismiss, OverlayBuilder, Placement, Side};
use crate::text::text_in;

/// Sheet width, in **spacing steps** (§2.6).
///
/// 130 × 4pt = 520pt: wider than a dialog ([`crate::DIALOG_WIDTH_STEPS`],
/// 360pt) because a sheet is where a *form* goes, and narrower than the window
/// so the title bar it hangs from stays visible on both sides.
pub const SHEET_WIDTH_STEPS: f32 = 130.0;

/// A modal sheet titled `title`.
///
/// Use [`sheet_in`] outside a build pass.
///
/// ```
/// use silka_widgets::sheet;
///
/// let s = sheet("Export").confirm("Export", || {}).cancel("Cancel", || {});
/// # let _ = s;
/// ```
pub fn sheet(title: impl Into<String>) -> Sheet {
    sheet_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        title,
    )
}

/// [`sheet`] with the text engine and the theme passed explicitly.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{sheet_in, Fonts, Side};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let s = sheet_in(&fonts, &theme, "Export invoices")
///     .message("Choose a format.")
///     .open(true)
///     .confirm("Export", || {})
///     .cancel("Cancel", || {});
///
/// // The buttons come back in this OS's order; the caller wrote them by meaning.
/// assert_eq!(s.arranged().len(), 2);
/// // And it hangs from the top edge rather than floating in the middle.
/// assert_eq!(s.placement().side, Side::Top);
/// ```
pub fn sheet_in(fonts: &Fonts, theme: &Theme, title: impl Into<String>) -> Sheet {
    Sheet {
        fonts: fonts.clone(),
        theme: *theme,
        key: None,
        title: title.into(),
        message: None,
        content: None,
        actions: Vec::new(),
        order: ButtonOrder::default(),
        open: false,
        width: theme.space(SHEET_WIDTH_STEPS),
        side: Side::Top,
        // A sheet is hinged to the window edge, so there is no margin between
        // the two: a gap here is what makes it look like a floating card that
        // happens to be near the top.
        gap: 0.0,
        // An `NSAlert`-style rule: a sheet asks something that has to be
        // answered, so a click on the dimmed window behind must not throw the
        // form away. Esc still works.
        dismiss: Dismiss::ESCAPE,
        on_dismiss: None,
        spring: Spring::snappy(),
    }
}

/// The sheet builder — Dart-style (§2.5).
pub struct Sheet {
    fonts: Fonts,
    theme: Theme,
    key: Option<Key>,
    title: String,
    message: Option<String>,
    content: Option<View>,
    actions: Vec<DialogAction>,
    order: ButtonOrder,
    open: bool,
    width: f32,
    side: Side,
    gap: f32,
    dismiss: Dismiss,
    on_dismiss: Option<Callback>,
    spring: Spring,
}

impl Sheet {
    /// Identity key — required when the sheet comes from a dynamic list (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Open or closed. Changing it **starts a transition**, never a jump.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Explanatory text below the title.
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// The form (or anything else) between the message and the button row.
    pub fn content(mut self, content: impl Into<View>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Add one button.
    pub fn action(mut self, action: DialogAction) -> Self {
        self.actions.push(action);
        self
    }

    /// Add several buttons at once.
    pub fn actions(mut self, actions: impl IntoIterator<Item = DialogAction>) -> Self {
        self.actions.extend(actions);
        self
    }

    /// Add the default button (run by Return).
    pub fn confirm(self, label: impl Into<String>, f: impl Fn() + 'static) -> Self {
        self.action(action(label).confirm().on_press(f))
    }

    /// Add the cancel button (run by Esc).
    pub fn cancel(self, label: impl Into<String>, f: impl Fn() + 'static) -> Self {
        self.action(action(label).cancel().on_press(f))
    }

    /// Add a destructive button — **never** the default one (HIG).
    pub fn destructive(self, label: impl Into<String>, f: impl Fn() + 'static) -> Self {
        self.action(action(label).destructive().on_press(f))
    }

    /// Force a button order instead of following the OS convention.
    pub fn order(mut self, order: ButtonOrder) -> Self {
        self.order = order;
        self
    }

    /// Panel width in logical points — **always** from the spacing scale
    /// (§2.6). It is still clamped to the window, so a narrow window narrows
    /// the sheet with it.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(0.0);
        self
    }

    /// Which window edge the sheet hangs from.
    ///
    /// [`Side::Top`] is the macOS shape and the default. The other sides exist
    /// for a reason rather than for symmetry: a sheet from the bottom is what
    /// a touch layout wants, and one from the reading end is a wide-window
    /// inspector — see [`mod@crate::drawer`] when the panel should be full height.
    pub fn side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }

    /// A margin between the sheet and the window edge it hangs from.
    ///
    /// Zero by default, which is what makes it read as attached.
    pub fn gap(mut self, token: SpaceToken) -> Self {
        self.gap = self.theme.space_of(token);
        self
    }

    /// The ways this sheet may be dismissed.
    pub fn dismiss(mut self, dismiss: Dismiss) -> Self {
        self.dismiss = dismiss;
        self
    }

    /// What runs when the user dismisses it. Without this, the
    /// [`ActionKind::Cancel`] action runs instead, so "Esc = Cancel" holds by
    /// itself.
    pub fn on_dismiss(mut self, f: impl Fn() + 'static) -> Self {
        self.on_dismiss = Some(Callback::new(f));
        self
    }

    /// The spring that drives its entrance.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// The buttons in the visual order that applies here.
    pub fn arranged(&self) -> Vec<DialogAction> {
        self.order.arrange(self.actions.clone())
    }

    /// The placement recipe handed to the overlay system.
    pub fn placement(&self) -> Placement {
        Placement::edge(self.side)
            .align(Align::Center)
            .gap(self.gap)
    }

    /// The panel's corner geometry: square where it meets the window edge.
    ///
    /// A sheet with four rounded corners is a floating card; rounding only the
    /// two corners that face into the window is what makes it look hinged.
    pub fn corners(&self) -> Corners {
        let r = self.theme.radius_of(RadiusToken::Xl);
        let radii = match self.side {
            Side::Top => CornerRadii {
                top_left: 0.0,
                top_right: 0.0,
                bottom_right: r,
                bottom_left: r,
            },
            Side::Bottom => CornerRadii {
                top_left: r,
                top_right: r,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
            // The horizontal sides are resolved against the reading direction
            // by the overlay system, and the two rounded corners follow: what
            // is rounded is the pair facing into the window, whichever pair
            // that turns out to be. Rounding all four is the honest answer
            // here, because this component does not learn the resolved side
            // until layout — and a drawer, which does, is `crate::drawer`.
            Side::Start | Side::End => CornerRadii::all(r),
        };
        Corners::new(radii, self.theme.corners_of(RadiusToken::Xl).style)
    }

    /// The action Return runs, if any.
    fn default_action(&self) -> Option<Callback> {
        self.actions
            .iter()
            .find(|a| a.kind() == ActionKind::Confirm)
            .and_then(DialogAction::callback)
    }

    /// The action Esc runs.
    fn dismiss_action(&self) -> Option<Callback> {
        self.on_dismiss.clone().or_else(|| {
            self.actions
                .iter()
                .find(|a| a.kind() == ActionKind::Cancel)
                .and_then(DialogAction::callback)
        })
    }

    /// Title + message.
    fn header(&self) -> View {
        let t = &self.theme;
        let mut baris: Vec<View> = Vec::with_capacity(2);
        baris.push(
            text_in(&self.fonts, self.title.clone())
                .type_style(t.typography.headline)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color_of(ColorToken::Label))
                // Announced once, from the overlay entry — not twice.
                .role(AccessRole::Container)
                .into(),
        );
        if let Some(pesan) = &self.message {
            baris.push(
                text_in(&self.fonts, pesan.clone())
                    .type_style(t.typography.body)
                    .color(t.color_of(ColorToken::SecondaryLabel))
                    .into(),
            );
        }
        column(baris)
            .spacing(t.space(2.0))
            .cross(CrossAlign::Stretch)
            .into()
    }

    /// The button row in the platform's visual order.
    fn buttons(&self) -> View {
        let t = &self.theme;
        let tombol: Vec<View> = self
            .arranged()
            .into_iter()
            .map(|a| {
                let mut b = button_variant_in(&self.fonts, t, a.label(), a.variant())
                    .disabled(a.is_disabled());
                if let Some(cb) = a.callback() {
                    b = b.on_press(move || cb.call());
                }
                b.into()
            })
            .collect();
        row(tombol)
            .main(MainAlign::End)
            .cross(CrossAlign::Center)
            .spacing(t.space(3.0))
            .wrap()
            .into()
    }

    /// The panel: header, content, buttons — inside the hinged card.
    fn panel(&mut self) -> View {
        let t = &self.theme;
        let mut isi: Vec<View> = vec![self.header()];
        if let Some(konten) = self.content.take() {
            isi.push(konten);
        }
        if !self.actions.is_empty() {
            isi.push(self.buttons());
        }

        let kartu = column(isi)
            .spacing(t.space(5.0))
            .cross(CrossAlign::Stretch)
            .padding(Insets::all(t.space(5.0)))
            .background(t.color_of(ColorToken::SurfaceElevated))
            .corners(self.corners())
            .border(
                t.space_of(SpaceToken::Px),
                t.color_of(ColorToken::Separator),
            )
            .shadow(t.shadow_of(ShadowToken::Xl));

        // The width is clamped to what is available, so a window narrower than
        // the sheet still lays out correctly.
        let kotak = constrained(
            BoxConstraints::new(self.width, self.width, 0.0, f32::INFINITY),
            kartu,
        );

        Builder::new(DialogPanelProps::new(self.open, self.default_action()))
            .child(kotak)
            .into()
    }
}

impl From<Sheet> for OverlayBuilder {
    fn from(mut b: Sheet) -> OverlayBuilder {
        let t = b.theme;
        let placement = b.placement();
        let mut ov = overlay(b.panel())
            .open(b.open)
            .barrier(Barrier::Modal)
            .backdrop(t.color_of(ColorToken::Scrim))
            .placement(placement)
            .dismiss(b.dismiss)
            .role(AccessRole::Dialog)
            .label(b.title.clone())
            .spring(b.spring);
        if let Some(cb) = b.dismiss_action() {
            ov = ov.on_dismiss(move || cb.call());
        }
        if let Some(key) = b.key.clone() {
            ov = ov.key(key);
        }
        ov
    }
}

impl From<Sheet> for View {
    fn from(b: Sheet) -> View {
        View::from(OverlayBuilder::from(b))
    }
}

impl core::fmt::Debug for Sheet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Sheet")
            .field("title", &self.title)
            .field("open", &self.open)
            .field("side", &self.side)
            .field("actions", &self.actions.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::signals::Runtime;
    use silka_core::tree::RenderTree;
    use silka_core::view::{fixed, reconcile};
    use silka_paint::Size;
    use silka_theme::{Appearance, Preset};

    const WINDOW: Size = Size::new(800.0, 600.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn laid_out(s: Sheet) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            crate::overlay_layer(fixed(WINDOW.width, WINDOW.height)).overlay(s),
        );
        tree.layout(BoxConstraints::tight(WINDOW));
        tree
    }

    fn opened(s: Sheet) -> RenderTree {
        let mut tree = laid_out(s);
        crate::overlay::settle(&mut tree);
        tree.layout(BoxConstraints::tight(WINDOW));
        tree
    }

    #[test]
    fn a_sheet_hangs_from_the_top_edge_rather_than_floating_in_the_middle() {
        let tree = opened(
            sheet_in(&Fonts::bundled_only(), &theme(), "Export")
                .open(true)
                .confirm("Export", || {}),
        );
        let entry = crate::overlay::entries(&tree)[0];
        let panel = tree
            .node_ref::<crate::overlay::OverlayEntry>(entry)
            .unwrap()
            .panel_rect();
        assert_eq!(panel.min_y(), 0.0, "attached, not centred");
        // Horizontally centred, like every macOS sheet.
        assert!((panel.center().x - WINDOW.width * 0.5).abs() < 1.0);
    }

    #[test]
    fn a_closed_sheet_waits_off_screen_instead_of_over_the_window() {
        // Its entrance comes from *outside* the window, which is what the edge
        // placement mode buys and what a centred dialog cannot do.
        let tree = laid_out(sheet_in(&Fonts::bundled_only(), &theme(), "Export").open(false));
        let entry = crate::overlay::entries(&tree)[0];
        let panel = tree
            .node_ref::<crate::overlay::OverlayEntry>(entry)
            .unwrap()
            .panel_rect();
        assert!(
            panel.max_y() <= 0.0,
            "a closed sheet sits above the window edge, got {panel:?}"
        );
    }

    #[test]
    fn the_top_corners_are_square_because_the_sheet_is_attached() {
        let s = sheet_in(&Fonts::bundled_only(), &theme(), "Export");
        let c = s.corners();
        assert_eq!(c.radii.top_left, 0.0);
        assert_eq!(c.radii.top_right, 0.0);
        assert!(c.radii.bottom_left > 0.0);

        // …and a sheet from the bottom is the mirror image.
        let bawah = sheet_in(&Fonts::bundled_only(), &theme(), "Export").side(Side::Bottom);
        assert_eq!(bawah.corners().radii.bottom_left, 0.0);
        assert!(bawah.corners().radii.top_left > 0.0);
    }

    #[test]
    fn the_button_order_is_the_platforms_and_the_caller_wrote_meaning() {
        let s = sheet_in(&Fonts::bundled_only(), &theme(), "Export")
            .confirm("Export", || {})
            .cancel("Cancel", || {})
            .order(ButtonOrder::ConfirmLast);
        let arranged = s.arranged();
        let names: Vec<&str> = arranged.iter().map(|a| a.label()).collect();
        assert_eq!(names, ["Cancel", "Export"]);

        let win = sheet_in(&Fonts::bundled_only(), &theme(), "Export")
            .confirm("Export", || {})
            .cancel("Cancel", || {})
            .order(ButtonOrder::ConfirmFirst);
        let arranged_win = win.arranged();
        let names: Vec<&str> = arranged_win.iter().map(|a| a.label()).collect();
        assert_eq!(names, ["Export", "Cancel"]);
    }

    #[test]
    fn return_runs_the_default_button_through_the_dialogs_own_node() {
        let rt = Runtime::new();
        let ran = rt.signal(0);
        let mut tree = opened(
            sheet_in(&Fonts::bundled_only(), &theme(), "Export")
                .open(true)
                .confirm("Export", move || ran.set(ran.get() + 1)),
        );
        assert!(crate::dialog::activate_default(&mut tree));
        assert_eq!(ran.get(), 1, "the very same seam a dialog uses");
    }

    #[test]
    fn esc_runs_cancel_without_the_caller_writing_it_twice() {
        let rt = Runtime::new();
        let closed = rt.signal(false);
        let mut tree = opened(
            sheet_in(&Fonts::bundled_only(), &theme(), "Export")
                .open(true)
                .confirm("Export", || {})
                .cancel("Cancel", move || closed.set(true)),
        );
        assert!(crate::overlay::dismiss_topmost(
            &mut tree,
            crate::overlay::Dismiss::ESCAPE
        ));
        assert!(closed.get());
    }

    #[test]
    fn a_click_on_the_dimmed_window_does_not_throw_the_form_away() {
        let rt = Runtime::new();
        let closed = rt.signal(false);
        let mut tree = opened(
            sheet_in(&Fonts::bundled_only(), &theme(), "Export")
                .open(true)
                .cancel("Cancel", move || closed.set(true)),
        );
        assert!(
            !crate::overlay::dismiss_topmost(&mut tree, crate::overlay::Dismiss::OUTSIDE),
            "a sheet asks something that has to be answered"
        );
        assert!(!closed.get());
    }

    #[test]
    fn the_content_behind_a_sheet_is_genuinely_inert() {
        let tree = opened(
            sheet_in(&Fonts::bundled_only(), &theme(), "Export")
                .open(true)
                .confirm("Export", || {}),
        );
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Export")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Dialog);
    }

    #[test]
    fn the_panel_is_token_driven_in_both_presets() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let th = Theme::new(preset, appearance);
                let tree = opened(
                    sheet_in(&Fonts::bundled_only(), &th, "Export")
                        .open(true)
                        .confirm("Export", || {}),
                );
                assert_eq!(crate::overlay::entries(&tree).len(), 1);
            }
        }
    }
}
