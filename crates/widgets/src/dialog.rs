//! `dialog()` and `alert()` — the first Tier 4 components (`KOMPONEN.md`).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::fixed;
//! # use silka_theme::{Appearance, Theme};
//! use silka_widgets::{dialog_in, overlay_layer, Fonts};
//!
//! # let rt = Runtime::new();
//! # let terbuka = rt.signal(true);
//! # let f = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! overlay_layer(fixed(800.0, 600.0).background(t.color.background)).overlay(
//!     dialog_in(&f, &t, "Save changes?")
//!         .message("Unsaved changes will be lost.")
//!         .open(terbuka.get())
//!         .cancel("Cancel", move || terbuka.set(false))
//!         .confirm("Save", move || terbuka.set(false)),
//! );
//! ```
//!
//! `KOMPONEN.md` raises two specific notes for this component, and both are
//! answered in this file:
//!
//! 1. **"Modal with a dimmed backdrop"** — not a new node: a dialog is a preset
//!    on top of [`mod@crate::overlay`], which was built once to serve ten
//!    components (working rule #3). All that is picked here is
//!    [`Barrier::Modal`], [`Placement::center`], and the `scrim` backdrop
//!    token; geometry, dismissal, focus trapping, and the spring transition
//!    already exist.
//! 2. **"Default/cancel buttons follow the per-OS convention"** —
//!    [`ButtonOrder`]. The app writes its actions in order of **meaning**
//!    (confirm, cancel, other) and the platform decides their visual order: on
//!    macOS and GNOME the default button sits farthest right with Cancel to its
//!    left, on Windows it is exactly the other way around. An app never writes
//!    `#[cfg(target_os)]` for this.
//!
//! Definition of Done (`KOMPONEN.md`) satisfied here:
//!
//! - **Both presets** via semantic tokens — not a single color, radius, or
//!   spacing number is born in this file.
//! - **A retargetable spring transition**: a dialog dismissed mid-open reverses
//!   direction carrying its velocity ([`mod@crate::overlay`] §3.5).
//! - **Full keyboard + focus ring**: Tab is trapped inside the panel (modal =
//!   focus scope), Space activates the focused control, and **Esc** runs the
//!   cancel action.
//!
//!   The **Return** rule is spelled out here because it is the only one with
//!   two plausible answers: Return is offered to the focused node first, so a
//!   focused button wins over the default button (shadcn/web behavior). As soon
//!   as the focused node does **not** swallow Return — a text field inside
//!   [`DialogBuilder::content`], or nothing focused at all — Return bubbles up
//!   to [`DialogPanel`] and runs the default button (HIG behavior). What never
//!   happens: Return running a destructive action.
//! - **AccessKit nodes**: the panel takes the [`AccessRole::Dialog`] role with
//!   the title as its name, its contents are announced, and the content behind
//!   it is genuinely inert.
//! - **Dark mode**, **hit target ≥ 44pt** (the buttons are [`crate::button()`]),
//!   and **reduced-motion** (the transition is
//!   [`silka_core::animation::MotionRole`] `Essential`: the bounce is dropped,
//!   the motion that explains the change is kept).
//!
//! ## Enter with nothing focused
//!
//! Return's normal path is to bubble up from the focused node and pass through
//! [`DialogPanel`]. But when nothing is focused yet, a key event only reaches
//! the root of the tree — exactly the same hole that
//! [`crate::overlay::dismiss_topmost`] patches for Esc. [`activate_default`] is
//! its counterpart for Return, and the shell calls it under the same condition:
//! **only** when the router reports that nothing handled the event.

use silka_core::access::{AccessNode, AccessRole};
use silka_core::animation::Spring;
use silka_core::input::{Event, EventCtx, NamedKey};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{
    BoxConstraints, CrossAlign, LayoutCtx, MainAlign, NodeId, RenderNode, RenderTree,
};
use silka_core::view::{column, constrained, row, Builder, View, ViewNode};
use silka_core::Callback;
use silka_paint::{Insets, Point, Size};
use silka_text::FontWeight;
use silka_theme::{SpaceToken, Theme, TypeStyle};

use crate::button::{button_variant_in, ButtonVariant};
use crate::fonts::Fonts;
use crate::overlay::{overlay, Barrier, Dismiss, OverlayBuilder, OverlayEntry, Placement};
use crate::text::{text_in, Text};

/// Dialog panel width in **spacing scale steps** (§2.6).
///
/// 90 × 4pt = 360pt: between `NSAlert` (260pt, too narrow for explanatory
/// text) and shadcn's `Dialog` (512pt, too wide for an alert). The number is
/// still a multiple of the scale, not an arbitrary width.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::DIALOG_WIDTH_STEPS;
///
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // The width is a count of spacing steps, so it stays on the scale rather
/// // than becoming one more loose number in the codebase.
/// assert_eq!(theme.space(DIALOG_WIDTH_STEPS), 360.0);
///
/// // Between NSAlert (too narrow for explanatory text) and shadcn's Dialog
/// // (too wide for an alert).
/// assert!(theme.space(DIALOG_WIDTH_STEPS) > 260.0);
/// assert!(theme.space(DIALOG_WIDTH_STEPS) < 512.0);
/// ```
pub const DIALOG_WIDTH_STEPS: f32 = 90.0;

// ---------------------------------------------------------------------------
// Button order
// ---------------------------------------------------------------------------

/// Dialog button order — the only thing in this component that genuinely
/// differs between operating systems.
///
/// | Platform | Order (left → right) |
/// |---|---|
/// | macOS (HIG), GNOME | `[other…] [Cancel] [Default]` |
/// | Windows | `[Default] [Cancel] [other…]` |
///
/// The app writes its actions in order of **meaning**, not in pixel order;
/// [`ButtonOrder::Platform`] translates between the two. In an RTL interface
/// the row mirrors itself, because [`row`] follows the reading direction
/// (§9.8).
/// ```
/// use silka_widgets::{action, ButtonOrder};
///
/// // The application writes its actions in order of *meaning*.
/// let written = vec![
///     action("Save").confirm(),
///     action("Cancel").cancel(),
/// ];
///
/// // macOS and GNOME put the default button last…
/// let mac = ButtonOrder::ConfirmLast.arrange(written.clone());
/// assert_eq!(mac.last().unwrap().label(), "Save");
///
/// // …Windows puts it first. Neither ordering is written by the caller.
/// let windows = ButtonOrder::ConfirmFirst.arrange(written.clone());
/// assert_eq!(windows.first().unwrap().label(), "Save");
///
/// // `Platform` is decided at compile time, so no application asks its own
/// // operating system anything merely to lay out two buttons.
/// assert_ne!(ButtonOrder::PLATFORM, ButtonOrder::Platform);
/// assert!(matches!(
///     ButtonOrder::PLATFORM,
///     ButtonOrder::ConfirmFirst | ButtonOrder::ConfirmLast,
/// ));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ButtonOrder {
    /// Follow the convention of the OS this app is compiled for
    /// ([`ButtonOrder::PLATFORM`]).
    #[default]
    Platform,
    /// Default button last (macOS, GNOME).
    ConfirmLast,
    /// Default button first (Windows).
    ConfirmFirst,
}

impl ButtonOrder {
    /// The convention of this build's target OS.
    ///
    /// Decided at compile time, not at run time: no app should have to ask its
    /// own operating system anything just to lay out two buttons.
    pub const PLATFORM: ButtonOrder = if cfg!(target_os = "windows") {
        ButtonOrder::ConfirmFirst
    } else {
        ButtonOrder::ConfirmLast
    };

    /// The concrete order — [`ButtonOrder::Platform`] resolves to
    /// [`ButtonOrder::PLATFORM`].
    pub fn resolved(self) -> Self {
        match self {
            ButtonOrder::Platform => ButtonOrder::PLATFORM,
            lain => lain,
        }
    }

    /// Rearrange `actions` into their visual order.
    ///
    /// A pure function, deliberately: this is the one part of the "per-OS
    /// convention" that has a right and a wrong answer, so it must be testable
    /// without a tree, without a GPU, and for **both** platforms at once
    /// (§9.5).
    ///
    /// ```
    /// use silka_widgets::dialog::{action, ButtonOrder};
    ///
    /// let urut = ButtonOrder::ConfirmLast.arrange(vec![
    ///     action("Save").confirm(),
    ///     action("Cancel").cancel(),
    /// ]);
    /// let nama: Vec<&str> = urut.iter().map(|a| a.label()).collect();
    /// assert_eq!(nama, ["Cancel", "Save"]);
    /// ```
    pub fn arrange(self, actions: Vec<DialogAction>) -> Vec<DialogAction> {
        // One rule, not two: split into three role groups, then concatenate
        // the groups per the convention. What swaps places is the **group**,
        // not individual buttons — the order the app wrote within a group
        // stays its reading order on both platforms. (Reversing the whole
        // vector would also swap two "other" buttons that should stay
        // side by side in the order they were written.)
        let mut lainnya: Vec<DialogAction> = Vec::new();
        let mut batal: Vec<DialogAction> = Vec::new();
        let mut utama: Vec<DialogAction> = Vec::new();
        for a in actions {
            match a.kind {
                ActionKind::Plain => lainnya.push(a),
                ActionKind::Cancel => batal.push(a),
                ActionKind::Confirm | ActionKind::Destructive => utama.push(a),
            }
        }

        let mut out: Vec<DialogAction> =
            Vec::with_capacity(lainnya.len() + batal.len() + utama.len());
        if self.resolved() == ButtonOrder::ConfirmFirst {
            // Windows: `[Default] [Cancel] [other…]`.
            out.append(&mut utama);
            out.append(&mut batal);
            out.append(&mut lainnya);
        } else {
            // macOS/GNOME: `[other…] [Cancel] [Default]`.
            out.append(&mut lainnya);
            out.append(&mut batal);
            out.append(&mut utama);
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// The role of a dialog button — decides its position, visual variant, and the
/// key that runs it.
/// ```
/// use silka_widgets::{action, ActionKind, ButtonVariant};
///
/// // The role decides the position, the visual variant, and the key.
/// assert_eq!(action("Save").confirm().kind(), ActionKind::Confirm);
/// assert_eq!(action("Cancel").cancel().kind(), ActionKind::Cancel);
/// assert_eq!(action("Don't Save").kind(), ActionKind::Plain);
///
/// // A destructive action looks like the primary one…
/// let delete = action("Delete").destructive();
/// assert_eq!(delete.variant(), ButtonVariant::Destructive);
///
/// // …but it is never the Return-key default. The HIG forbids a destructive
/// // action being run by a stray keypress.
/// assert_ne!(delete.kind(), ActionKind::Confirm);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ActionKind {
    /// The primary action, run by **Return** from anywhere inside the dialog.
    Confirm,
    /// Cancel: run by **Esc**, and (when allowed) by a click outside the panel.
    Cancel,
    /// A destructive action (Delete, Discard). Takes the same position as
    /// [`ActionKind::Confirm`] but is **never** the default button — the HIG
    /// forbids a destructive action being run accidentally by Return.
    Destructive,
    /// An extra action with no special role ("Don't Save").
    #[default]
    Plain,
}

/// A single dialog button.
///
/// Written Dart-style (§2.5): [`action`] followed by method chaining.
///
/// ```
/// use silka_widgets::{action, ActionKind, ButtonVariant};
///
/// let save = action("Save").confirm().on_press(|| {});
/// assert_eq!(save.label(), "Save");
/// assert_eq!(save.kind(), ActionKind::Confirm);
/// assert_eq!(save.variant(), ButtonVariant::Primary);
/// assert!(!save.is_disabled());
///
/// // An action can be present but unavailable — still announced, so the
/// // reader learns the option exists and why it cannot be taken.
/// let publish = action("Publish").confirm().disabled(true);
/// assert!(publish.is_disabled());
/// ```
/// Written Dart-style (§2.5): [`action`] followed by method chaining.
#[derive(Debug, Clone)]
pub struct DialogAction {
    label: String,
    kind: ActionKind,
    on_press: Option<Callback>,
    disabled: bool,
}

/// A dialog button labeled `label`, with no special role.
///
/// ```
/// use silka_widgets::{action, ActionKind};
///
/// // No role until one is asked for, which is the right default for the
/// // third button in a three-button alert ("Don't Save").
/// let plain = action("Don't Save");
/// assert_eq!(plain.kind(), ActionKind::Plain);
/// ```
pub fn action(label: impl Into<String>) -> DialogAction {
    DialogAction {
        label: label.into(),
        kind: ActionKind::Plain,
        on_press: None,
        disabled: false,
    }
}

impl DialogAction {
    /// Make this the primary action (the default button; run by Return).
    pub fn confirm(mut self) -> Self {
        self.kind = ActionKind::Confirm;
        self
    }

    /// Make this the cancel action (run by Esc).
    pub fn cancel(mut self) -> Self {
        self.kind = ActionKind::Cancel;
        self
    }

    /// Make this a destructive action.
    pub fn destructive(mut self) -> Self {
        self.kind = ActionKind::Destructive;
        self
    }

    /// What runs when this button is activated.
    pub fn on_press(mut self, f: impl Fn() + 'static) -> Self {
        self.on_press = Some(Callback::new(f));
        self
    }

    /// Disable this button (still announced, as dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The button's name.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The button's role.
    pub fn kind(&self) -> ActionKind {
        self.kind
    }

    /// True when this button cannot be used.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// The [`mod@crate::button`] visual variant for this role.
    ///
    /// A mapping, not a color choice: every color still belongs to a token.
    pub fn variant(&self) -> ButtonVariant {
        match self.kind {
            ActionKind::Confirm => ButtonVariant::Primary,
            ActionKind::Destructive => ButtonVariant::Destructive,
            ActionKind::Cancel | ActionKind::Plain => ButtonVariant::Secondary,
        }
    }

    /// This action's callback, if any.
    ///
    /// A disabled action reports `None` rather than a callback nobody may run,
    /// so "is this button live?" is answered in one place instead of at every
    /// call site. Public because [`mod@crate::sheet`] builds the same button
    /// row from the same actions rather than growing a second vocabulary for
    /// confirm/cancel/destructive.
    pub fn callback(&self) -> Option<Callback> {
        self.on_press.clone().filter(|_| !self.disabled)
    }
}

// ---------------------------------------------------------------------------
// Panel node
// ---------------------------------------------------------------------------

/// The dialog panel node: **the default button is its only reason to exist**.
///
/// Beyond that it is transparent — layout is passed through untouched and its
/// role is structural, because the dialog's name and role are already
/// announced by the [`OverlayEntry`] above it (one dialog = one name, not two).
pub struct DialogPanel {
    /// The dialog is open (not animating out).
    pub open: bool,
    /// The action Return runs.
    pub default_action: Option<Callback>,
}

impl DialogPanel {
    /// Run the default button; true when something actually ran.
    ///
    /// The callback is cloned out first — it almost always writes a signal,
    /// and a signal write may trigger anything; what it must not do is run
    /// while this node is still borrowed `&mut` (the same pattern as
    /// [`silka_core::tree::Interactive`]).
    pub fn activate_default(&mut self) -> bool {
        if !self.open {
            return false;
        }
        let Some(cb) = self.default_action.clone() else {
            return false;
        };
        cb.call();
        true
    }
}

impl RenderNode for DialogPanel {
    fn type_name(&self) -> &'static str {
        "DialogPanel"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let anak = ctx.child(0);
        let ukuran = ctx.layout_child(anak, constraints);
        ctx.place_child(anak, Point::ZERO);
        constraints.constrain(ukuran)
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        // Return is only marked handled when this dialog actually has a
        // default button: a dialog with no primary action must **let** Return
        // bubble on, not swallow it silently (the same rule as Esc in
        // `OverlayEntry`).
        let Event::Key(k) = event else { return };
        if !k.is_pressed() || !k.code.is(NamedKey::Enter) || !k.modifiers.is_empty() {
            return;
        }
        if self.activate_default() {
            ctx.handled();
        }
    }
}

impl core::fmt::Debug for DialogPanel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DialogPanel")
            .field("open", &self.open)
            .field("default_action", &self.default_action.is_some())
            .finish()
    }
}

/// [`DialogPanel`] props.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DialogPanelProps {
    open: bool,
    default_action: Option<Callback>,
}

impl DialogPanelProps {
    /// The panel props for an overlay that is `open`, with `default_action` as
    /// the button Return runs.
    ///
    /// The seam [`mod@crate::sheet`] rides: a sheet is a dialog that arrives
    /// from the top edge rather than the middle, and "Return runs the default
    /// button, Esc runs cancel" must be the very same node in both — a second
    /// implementation is how the two drift apart in exactly the case nobody
    /// tests (a focused text field inside the panel).
    pub fn new(open: bool, default_action: Option<Callback>) -> Self {
        Self {
            open,
            default_action,
        }
    }
}

impl ViewNode for DialogPanelProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(DialogPanel {
            open: self.open,
            default_action: self.default_action.clone(),
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<DialogPanel>()
            .expect("same view type means same render node type");
        n.open = self.open;
        // The callback is always replaced without comparison: the closure is
        // rebuilt on every rebuild and captures fresh values (see
        // `InteractiveProps`).
        n.default_action.clone_from(&self.default_action);
        Dirty::NONE
    }
}

// ---------------------------------------------------------------------------
// Return safety net
// ---------------------------------------------------------------------------

/// Run the topmost dialog's default button; true when something ran.
///
/// The counterpart of [`crate::overlay::dismiss_topmost`] for Return, with
/// exactly the same usage condition — the shell calls it **only** when the
/// router reports that nothing handled the event:
///
/// ```
/// # use silka_core::input::{Event, InputRouter, KeyEvent, KeyCode, NamedKey};
/// # use silka_core::tree::RenderTree;
/// # use std::time::Duration;
/// # use silka_widgets::dialog::activate_default;
/// # let mut tree = RenderTree::new();
/// # let mut router = InputRouter::new();
/// let enter = Event::Key(KeyEvent::pressed(
///     KeyCode::Named(NamedKey::Enter),
///     Duration::ZERO,
/// ));
/// if !router.dispatch(&mut tree, &enter).handled {
///     activate_default(&mut tree);
/// }
/// ```
pub fn activate_default(tree: &mut RenderTree) -> bool {
    let Some(panel) = panel_teratas(tree) else {
        return false;
    };
    tree.node_mut_ref::<DialogPanel>(panel)
        .is_some_and(DialogPanel::activate_default)
}

/// The dialog panel belonging to the topmost open overlay.
fn panel_teratas(tree: &RenderTree) -> Option<NodeId> {
    crate::overlay::entries(tree)
        .into_iter()
        .rev()
        .filter(|id| {
            tree.node_ref::<OverlayEntry>(*id)
                .is_some_and(|o| o.open && o.is_visible())
        })
        .find_map(|id| cari_panel(tree, id))
}

fn cari_panel(tree: &RenderTree, akar: NodeId) -> Option<NodeId> {
    if tree.node_ref::<DialogPanel>(akar).is_some() {
        return Some(akar);
    }
    tree.children(akar)
        .iter()
        .find_map(|anak| cari_panel(tree, *anak))
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// A modal dialog — `dialog` (`KOMPONEN.md` Tier 4).
///
/// ```
/// use silka_widgets::dialog;
///
/// let d = dialog("Save changes?")
///     .message("Your edits will be lost otherwise.")
///     .confirm("Save", || {})
///     .cancel("Cancel", || {});
/// # let _ = d;
/// ```
///
/// Use [`dialog_in`] outside a build pass.
pub fn dialog(title: impl Into<String>) -> DialogBuilder {
    dialog_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        title,
    )
}

/// A modal dialog titled `title` — the equivalent of shadcn's `Dialog`.
///
/// By default it can be dismissed with Esc **and** by clicking outside the
/// panel; for an alert that must not disappear by accident, use [`alert`].
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{dialog_in, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
/// let rt = Runtime::new();
/// let open = rt.signal(true);
///
/// let sheet = dialog_in(&fonts, &theme, "Rename file")
///     .message("Choose a new name for this document.")
///     .open(open.get())
///     .confirm("Rename", || {})
///     .cancel("Cancel", move || open.set(false));
///
/// // The buttons come back in the order this OS puts them in — the caller
/// // wrote them in order of meaning.
/// let arranged = sheet.arranged();
/// assert_eq!(arranged.len(), 2);
/// ```
pub fn dialog_in(fonts: &Fonts, theme: &Theme, title: impl Into<String>) -> DialogBuilder {
    DialogBuilder {
        fonts: fonts.clone(),
        theme: *theme,
        key: None,
        title: title.into(),
        message: None,
        content: None,
        actions: Vec::new(),
        order: ButtonOrder::default(),
        open: false,
        width: theme.space(DIALOG_WIDTH_STEPS),
        dismiss: Dismiss::ALL,
        on_dismiss: None,
        spring: Spring::snappy(),
    }
}

/// A [`dialog`] an outside click cannot dismiss — the `NSAlert` shape.
///
/// ```
/// use silka_widgets::alert;
///
/// let a = alert("Delete 3 files?").destructive("Delete", || {});
/// # let _ = a;
/// ```
///
/// Use [`alert_in`] outside a build pass.
pub fn alert(title: impl Into<String>) -> DialogBuilder {
    dialog(title).dismiss(Dismiss::ESCAPE)
}

/// A modal alert — the equivalent of `NSAlert`.
///
/// It differs from [`dialog`] in exactly one way, and it is not a visual one:
/// clicking outside the panel does **not** dismiss it. An alert asks something
/// that has to be answered; making it vanish because the cursor slipped means
/// losing data (the same behavior as `NSAlert` and shadcn's `AlertDialog`).
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{alert_in, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // A question that has to be answered: Esc still works, but a click that
/// // lands outside the panel does not throw the work away.
/// let confirm = alert_in(&fonts, &theme, "Discard changes?")
///     .message("This cannot be undone.")
///     .destructive("Discard", || {})
///     .cancel("Keep editing", || {})
///     .open(true);
///
/// assert_eq!(confirm.arranged().len(), 2);
/// ```
pub fn alert_in(fonts: &Fonts, theme: &Theme, title: impl Into<String>) -> DialogBuilder {
    dialog_in(fonts, theme, title).dismiss(Dismiss::ESCAPE)
}

/// The dialog builder.
///
/// It becomes an [`OverlayBuilder`] when handed to [`crate::overlay_layer`],
/// so a dialog rides on the same overlay infrastructure as
/// popover/tooltip/menu/toast — no geometry, dismissal, or transition is
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{action, dialog_in, ButtonOrder, Dismiss, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let d = dialog_in(&fonts, &theme, "Export")
///     .message("Pick a format.")
///     .actions([
///         action("Export").confirm().on_press(|| {}),
///         action("Cancel").cancel().on_press(|| {}),
///         action("Help"),
///     ])
///     .order(ButtonOrder::ConfirmLast)
///     .dismiss(Dismiss::ESCAPE)
///     .open(true);
///
/// // Three buttons, and the confirm one has been moved to the end for us.
/// let arranged = d.arranged();
/// assert_eq!(arranged.len(), 3);
/// assert_eq!(arranged.last().unwrap().label(), "Export");
/// ```
/// recomputed here.
pub struct DialogBuilder {
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
    dismiss: Dismiss,
    on_dismiss: Option<Callback>,
    spring: Spring,
}

impl DialogBuilder {
    /// Extra content between the message and the button row — a form, a list
    /// of choices, or anything else.
    ///
    /// This is where the Return rule becomes visible: as long as focus sits on
    /// a control that does **not** swallow Return (a single-line text field,
    /// say), Return still runs the dialog's default button.
    pub fn content(mut self, content: impl Into<View>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Identity key — required when the dialog comes from a dynamic list
    /// (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Open or closed. Changing it **triggers a transition**, not a jump.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Explanatory text below the title.
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Add a single button.
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

    /// Add the cancel button (run by Esc, and by an outside click when
    /// allowed).
    pub fn cancel(self, label: impl Into<String>, f: impl Fn() + 'static) -> Self {
        self.action(action(label).cancel().on_press(f))
    }

    /// Add a destructive button — it is **not** the default button (HIG).
    pub fn destructive(self, label: impl Into<String>, f: impl Fn() + 'static) -> Self {
        self.action(action(label).destructive().on_press(f))
    }

    /// Force a button order instead of following the OS convention.
    ///
    /// For the gallery and cross-platform tests; ordinary apps do not use it.
    pub fn order(mut self, order: ButtonOrder) -> Self {
        self.order = order;
        self
    }

    /// Panel width in logical points — **always** derived from the spacing
    /// scale (§2.6).
    ///
    /// The value is still clamped to the available space: in a narrow window
    /// the panel narrows with it, never sticking out past the screen.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(0.0);
        self
    }

    /// The ways this dialog is allowed to be dismissed.
    pub fn dismiss(mut self, dismiss: Dismiss) -> Self {
        self.dismiss = dismiss;
        self
    }

    /// What runs when the user dismisses the dialog (Esc/outside click).
    ///
    /// Without this, the [`ActionKind::Cancel`] action runs instead — so
    /// "Esc = Cancel" holds by itself and never has to be written twice.
    pub fn on_dismiss(mut self, f: impl Fn() + 'static) -> Self {
        self.on_dismiss = Some(Callback::new(f));
        self
    }

    /// The spring that drives its transition (`smooth`/`snappy`/`bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// The buttons in the visual order that applies here.
    pub fn arranged(&self) -> Vec<DialogAction> {
        self.order.arrange(self.actions.clone())
    }

    /// The action Return runs, if any.
    fn default_action(&self) -> Option<Callback> {
        self.actions
            .iter()
            .find(|a| a.kind == ActionKind::Confirm)
            .and_then(DialogAction::callback)
    }

    /// The action Esc/an outside click runs.
    fn dismiss_action(&self) -> Option<Callback> {
        self.on_dismiss.clone().or_else(|| {
            self.actions
                .iter()
                .find(|a| a.kind == ActionKind::Cancel)
                .and_then(DialogAction::callback)
        })
    }

    /// The panel: title, message, extra content, then the button row.
    fn panel(&mut self) -> View {
        let t = &self.theme;
        let mut isi: Vec<View> = vec![self.header()];
        if let Some(konten) = self.content.take() {
            isi.push(konten);
        }
        if !self.actions.is_empty() {
            isi.push(self.tombol());
        }

        let kartu = column(isi)
            .spacing(t.space(5.0))
            .cross(CrossAlign::Stretch)
            .padding(Insets::all(t.space(5.0)))
            .background(t.color.surface_elevated)
            .corners(t.corners(t.radius.xl))
            // The hairline follows the spacing scale (0.25 step = 1pt): in
            // dark mode this is what separates the panel from the scrim
            // behind it.
            .border(t.space_of(SpaceToken::Px), t.color.separator)
            .shadow(t.shadow.xl);

        // The width is clamped to the available space by
        // `BoxConstraints::enforce`, so a window narrower than the dialog
        // still lays out correctly.
        let kotak = constrained(
            BoxConstraints::new(self.width, self.width, 0.0, f32::INFINITY),
            kartu,
        );

        Builder::new(DialogPanelProps {
            open: self.open,
            default_action: self.default_action(),
        })
        .child(kotak)
        .into()
    }

    /// Title + message.
    fn header(&self) -> View {
        let t = &self.theme;
        let mut baris: Vec<View> = Vec::with_capacity(2);
        baris.push(
            gaya(text_in(&self.fonts, &self.title), t.typography.headline)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color.label)
                // The title is announced once, from the dialog node — not
                // twice.
                .role(AccessRole::Container)
                .into(),
        );
        if let Some(pesan) = &self.message {
            baris.push(
                gaya(text_in(&self.fonts, pesan), t.typography.body)
                    .color(t.color.secondary_label)
                    .into(),
            );
        }
        column(baris)
            .spacing(t.space(2.0))
            .cross(CrossAlign::Stretch)
            .into()
    }

    /// The button row in the platform's visual order.
    fn tombol(&self) -> View {
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
            // Dialog buttons align to the end of the row on all three
            // operating systems; only their order differs (`ButtonOrder`). In
            // RTL the row mirrors itself.
            .main(MainAlign::End)
            .cross(CrossAlign::Center)
            .spacing(t.space(3.0))
            .wrap()
            .into()
    }
}

/// Apply a typography token to a piece of text.
fn gaya(teks: Text, style: TypeStyle) -> Text {
    teks.size(style.size)
        .line_height(style.line_height)
        .tracking(style.tracking)
        .weight(FontWeight(style.weight))
}

impl From<DialogBuilder> for OverlayBuilder {
    fn from(mut b: DialogBuilder) -> OverlayBuilder {
        let t = b.theme;
        let mut ov = overlay(b.panel())
            .open(b.open)
            .barrier(Barrier::Modal)
            .backdrop(t.color.scrim)
            .placement(Placement::center())
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

impl From<DialogBuilder> for View {
    fn from(b: DialogBuilder) -> View {
        View::from(OverlayBuilder::from(b))
    }
}

impl core::fmt::Debug for DialogBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DialogBuilder")
            .field("title", &self.title)
            .field("open", &self.open)
            .field("actions", &self.actions.len())
            .field("order", &self.order.resolved())
            .finish()
    }
}

#[cfg(test)]
mod tests;
