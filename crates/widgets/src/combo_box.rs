//! `combo_box()` — a text field with a list of suggestions under it
//! (`KOMPONEN.md` Tier 2, `NSComboBox`).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::column;
//! # use silka_widgets::overlay_layer;
//! use silka_widgets::{combo_box, MenuState};
//!
//! # let rt = Runtime::new();
//! let query = rt.signal(String::new());
//! let list = rt.signal(MenuState::new());
//!
//! // Filtering is the application's job — see below.
//! let cities = ["Bandung", "Denpasar", "Surabaya"];
//! let hits: Vec<&str> = cities
//!     .iter()
//!     .copied()
//!     .filter(|c| c.to_lowercase().contains(&query.get().to_lowercase()))
//!     .collect();
//!
//! let combo = combo_box(query.get())
//!     .label("City")
//!     .placeholder("Where to?")
//!     .suggestions(hits)
//!     .bind(list)
//!     .on_change(move |s| query.set(s.to_owned()))
//!     .on_select(move |_, s| query.set(s.to_owned()));
//!
//! // Two pieces in two places, exactly like `select` and `menu`.
//! let mut layer = overlay_layer(column([combo.field()]));
//! for panel in combo.overlays() {
//!     layer = layer.overlay(panel);
//! }
//! ```
//!
//! ## It is a composition, not a new control
//!
//! A combo box is the one place in the catalogue where two finished components
//! have to cooperate, and the whole design of this module is about **not**
//! writing a third one:
//!
//! | Part | Who does it |
//! |---|---|
//! | Typing, caret, selection, undo, IME | [`mod@crate::text_field`] — untouched |
//! | The suggestion panel, its rows, its highlight | [`mod@crate::menu`] — untouched |
//! | Placement, auto-flip, dismissal | [`mod@crate::overlay`], through the menu |
//! | Which key belongs to which of the two | this module, and only this module |
//!
//! What is genuinely new here is one node ([`ComboFieldBox`]) that sits **above**
//! the text field and takes the keys the field does not want: ↓ opens the list
//! and walks it, ↑ walks back, Return takes the highlighted suggestion, Esc
//! closes. Everything else — every letter, every ←/→, ⌘Z, the whole IME path —
//! reaches the field untouched, because the node never sees it: a key event
//! travels from the focused node outwards, so anything the field handles stops
//! there.
//!
//! The one thing that had to change elsewhere is ↑/↓. A single-line field moves
//! its caret to the ends of the content with them (the AppKit habit), which
//! would have swallowed the keys the list needs — so `text_field` grew
//! [`ArrowKeys`], and a combo box asks for [`ArrowKeys::Bubble`].
//!
//! ## The state is a `MenuState`
//!
//! Deliberately not a new type. Everything a suggestion list can get wrong —
//! a highlight running past the end, a list that fails to close after a choice,
//! Esc closing the wrong thing — is already settled in [`MenuState::apply`],
//! tested there, and shared with every menu in the application. A second copy
//! of those rules is a second set of bugs.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Requirement | Where |
//! |---|---|
//! | Both presets | every value belongs to `text_field` and `menu`, which resolve their own tokens |
//! | Interactive states on springs | the field's hover/focus ring, the panel's overlay transition, each row's background |
//! | Keyboard + focus ring | ↓/↑/Return/Esc here, everything else in the field — and **focus never leaves the field**, which is what keeps typing possible while the list is open |
//! | AccessKit node | a [`AccessRole::Group`] carrying `expanded` around the field's [`AccessRole::TextInput`], with the panel's `Menu`/`MenuItem` nodes |
//! | Dark mode | tokens only, in the two components underneath |
//! | Hit target ≥ 44pt | the field's `min_height` and the menu's row height, both already [`crate::MIN_HIT_TARGET`] |
//! | Reduced motion | inherited from both |
//!
//! ## Filtering is the application's job
//!
//! [`ComboBox::suggestions`] takes the list to show, already filtered. That is
//! not laziness: matching is where the domain lives — accent folding, aliases,
//! a remote lookup, "recently used first" — and a widget that guessed at it
//! would be wrong for most applications and impossible to correct for the rest.
//!
//! ## Deliberately not here yet
//!
//! - **Inline autocompletion** (typing "Ban" and having "dung" appear selected
//!   ahead of the caret). It needs the field to hold text the user did not type,
//!   which is a change to the editing model rather than to this file.
//! - **Re-anchoring on window resize.** The panel's anchor is published once,
//!   when the list opens ([`sync`]); a window resized while a combo box is open
//!   leaves it hanging where it was. The same limitation the menu trigger has,
//!   and worth fixing in one place rather than two.

use std::rc::Rc;

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::Spring;
use silka_core::input::{Event, EventCtx, FocusPolicy, HitBehavior, KeyCode, NamedKey};
use silka_core::scheduler::Dirty;
use silka_core::signals::Signal;
use silka_core::tree::{BoxConstraints, LayoutCtx, NodeId, PaintCtx, RenderNode, RenderTree};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Point, Size};
use silka_theme::Theme;

use crate::editing::TextCallback;
use crate::fonts::Fonts;
use crate::menu::{item, menu_in, MenuEntry, MenuHandler, MenuIntent, MenuModel, MenuState};
use crate::overlay::{anchor_rect, Anchor, OverlayBuilder, OverlayLayer};
use crate::text_field::{text_field_in, ArrowKeys};

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------

type PickFn = Rc<dyn Fn(usize, &str)>;

/// What runs when the user takes one of the suggestions.
///
/// It carries the index **and** the text, because callers need both: the index
/// to look the choice up in their own data, the text to put back into the
/// field.
///
/// ```
/// use std::cell::RefCell;
/// use std::rc::Rc;
///
/// use silka_widgets::PickCallback;
///
/// let seen = Rc::new(RefCell::new(String::new()));
/// let sink = seen.clone();
/// let on_select = PickCallback::new(move |i, s: &str| *sink.borrow_mut() = format!("{i}:{s}"));
///
/// on_select.call(2, "Surabaya");
/// assert_eq!(seen.borrow().as_str(), "2:Surabaya");
///
/// // Cheap to clone, equal only to itself.
/// assert_eq!(on_select.clone(), on_select);
/// assert_ne!(on_select, PickCallback::new(|_, _| {}));
/// ```
#[derive(Clone)]
pub struct PickCallback(PickFn);

impl PickCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(usize, &str) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run it for the suggestion at `index`.
    pub fn call(&self, index: usize, text: &str) {
        (self.0)(index, text)
    }
}

impl PartialEq for PickCallback {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for PickCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PickCallback")
    }
}

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// The node that sits **above** the text field and owns the list's keys.
///
/// It draws nothing, measures nothing of its own, and takes no focus: its
/// entire job is to receive the four keys the field lets through, and to
/// publish the field's rect once the list opens.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{column, reconcile};
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{combo_box_in, ComboFieldBox, Fonts, MIN_HIT_TARGET};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let combo = combo_box_in(&fonts, &theme, "Ban").suggestions(["Bandung"]);
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, column([combo.field()]));
/// tree.layout(BoxConstraints::loose(Size::new(320.0, 200.0)));
///
/// // The wrapper is the size of the field inside it: it adds no box of its
/// // own, which is what lets a combo box drop into a form row unchanged.
/// let wrapper = tree.children(tree.children(tree.root())[0])[0];
/// assert!(tree.node_ref::<ComboFieldBox>(wrapper).is_some());
/// assert!(tree.size(wrapper).height >= MIN_HIT_TARGET);
/// ```
pub struct ComboFieldBox {
    /// How many suggestions there are right now.
    count: usize,
    /// The list is open, according to the application.
    open: bool,
    /// The state already carries a rect for the panel to hang on.
    has_anchor: bool,
    /// The highlighted suggestion, if any.
    highlight: Option<usize>,
    disabled: bool,
    /// The field's contents, for `on_submit`.
    text: String,
    label: Option<String>,
    on_intent: Option<MenuHandler>,
    on_submit: Option<TextCallback>,
}

impl ComboFieldBox {
    /// True while the panel is waiting for the rect only the tree can supply.
    ///
    /// The same seam [`crate::menu::advance`] uses for a submenu opened by
    /// keyboard: a node may not look at its own position from inside layout, so
    /// it leaves a request behind and [`sync`] answers it one frame later. That
    /// one frame is why a suggestion list never flashes in the middle of the
    /// window before sliding under its field.
    pub fn wants_anchor(&self) -> bool {
        self.open && !self.has_anchor && self.count > 0 && !self.disabled
    }

    /// The list is open **and** ready to be shown.
    pub fn is_open(&self) -> bool {
        self.open && self.has_anchor && self.count > 0 && !self.disabled
    }

    /// The highlighted suggestion.
    pub fn highlight(&self) -> Option<usize> {
        self.highlight
    }

    /// How many suggestions are on offer.
    pub fn suggestion_count(&self) -> usize {
        self.count
    }

    /// Send one intent to the application.
    ///
    /// The handler is **cloned out first**: it almost always writes a signal,
    /// and a signal write may run anything — what it may not do is run while
    /// this node is borrowed `&mut`.
    pub(crate) fn emit(&self, intent: MenuIntent) {
        if let Some(h) = self.on_intent.clone() {
            h.emit(intent);
        }
    }
}

impl RenderNode for ComboFieldBox {
    fn type_name(&self) -> &'static str {
        "ComboBox"
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
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        // There is no `ComboBox` role in the vocabulary, so what is announced
        // is the honest shape of what is here: a group that can be opened and
        // closed, wrapped around the field's own `TextInput`.
        node.role = AccessRole::Group;
        node.label.clone_from(&self.label);
        node.disabled = self.disabled;
        // `Some(false)` and `None` are different statements: "closed" and "this
        // cannot be opened at all". With no suggestions the second is true.
        node.expanded = (self.count > 0 && !self.disabled).then_some(self.open);
        if !self.disabled && self.count > 0 {
            node.actions |= if self.open {
                AccessActions::COLLAPSE
            } else {
                AccessActions::EXPAND
            };
        }
    }

    fn hit_behavior(&self) -> HitBehavior {
        // A pure wrapper: the field underneath keeps every click it would have
        // had, including the one that places the caret.
        HitBehavior::DeferToChild
    }

    fn focus_policy(&self) -> FocusPolicy {
        // The field inside is the Tab stop. A wrapper that took focus would
        // make Tab visit the same control twice.
        FocusPolicy::NONE
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if self.disabled {
            return;
        }
        let Event::Key(k) = event else {
            return;
        };
        if !k.is_pressed() || !k.modifiers.is_empty() {
            return;
        }

        match &k.code {
            // ↓ opens a closed list and walks an open one — the NSComboBox
            // habit, and the one key everybody tries first.
            KeyCode::Named(NamedKey::ArrowDown) => {
                if self.count == 0 {
                    return;
                }
                ctx.handled();
                ctx.request_animation();
                if self.open {
                    self.emit(MenuIntent::Move(1));
                } else {
                    // Opened without an anchor: `sync` supplies the real rect
                    // one frame later, and until then the panel stays hidden.
                    self.emit(MenuIntent::Open(Anchor::None));
                }
            }
            KeyCode::Named(NamedKey::ArrowUp) => {
                if !self.open || self.count == 0 {
                    return;
                }
                ctx.handled();
                ctx.request_animation();
                self.emit(MenuIntent::Move(-1));
            }
            // Return takes the highlighted suggestion; with nothing highlighted
            // it means "I meant what I typed", which is a submit, not a pick.
            KeyCode::Named(NamedKey::Enter) => match (self.open, self.highlight) {
                (true, Some(index)) => {
                    ctx.handled();
                    ctx.request_animation();
                    self.emit(MenuIntent::Activate { depth: 0, index });
                }
                _ => {
                    if let Some(cb) = self.on_submit.clone() {
                        ctx.handled();
                        cb.call(&self.text);
                    }
                }
            },
            // Esc closes the list **without** clearing the field: what the user
            // typed is theirs, and a control that throws it away on Esc is the
            // fastest way to lose a sentence.
            KeyCode::Named(NamedKey::Escape) if self.open => {
                ctx.handled();
                ctx.request_animation();
                self.emit(MenuIntent::Close);
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for ComboFieldBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ComboBox")
            .field("suggestions", &self.count)
            .field("open", &self.open)
            .field("highlight", &self.highlight)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props of [`ComboFieldBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct ComboFieldProps {
    count: usize,
    open: bool,
    has_anchor: bool,
    highlight: Option<usize>,
    disabled: bool,
    text: String,
    label: Option<String>,
    on_intent: Option<MenuHandler>,
    on_submit: Option<TextCallback>,
}

impl ViewNode for ComboFieldProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ComboFieldBox {
            count: self.count,
            open: self.open,
            has_anchor: self.has_anchor,
            highlight: self.highlight,
            disabled: self.disabled,
            text: self.text.clone(),
            label: self.label.clone(),
            on_intent: self.on_intent.clone(),
            on_submit: self.on_submit.clone(),
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ComboFieldBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.open != self.open
            || n.count != self.count
            || n.highlight != self.highlight
            || n.disabled != self.disabled
            || n.has_anchor != self.has_anchor
        {
            n.open = self.open;
            n.count = self.count;
            n.highlight = self.highlight;
            n.disabled = self.disabled;
            n.has_anchor = self.has_anchor;
            // Nothing this node draws changes — the panel is a different
            // subtree — but the a11y `expanded` state does.
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        n.text.clone_from(&self.text);
        // Callbacks are replaced without comparison: closures are rebuilt every
        // rebuild and capture new values.
        n.on_intent.clone_from(&self.on_intent);
        n.on_submit.clone_from(&self.on_submit);
        dirty
    }
}

// ---------------------------------------------------------------------------
// Dart-style builder
// ---------------------------------------------------------------------------

/// Dart-style combo box builder (§2.5).
///
/// `Clone` is cheap and matters: the field and the panel come from the same
/// builder, so there is no way for the two to disagree about the state.
#[derive(Clone)]
pub struct ComboBox {
    fonts: Fonts,
    theme: Theme,
    value: String,
    suggestions: Vec<String>,
    state: MenuState,
    bound: Option<Signal<MenuState>>,
    placeholder: String,
    label: Option<String>,
    disabled: bool,
    spring: Spring,
    on_change: Option<TextCallback>,
    on_submit: Option<TextCallback>,
    on_select: Option<PickCallback>,
    on_intent: Option<MenuHandler>,
    key: String,
}

/// A text field with a list of suggestions — the `combo_box` component
/// (`KOMPONEN.md` Tier 2).
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_widgets::{combo_box, MenuState};
///
/// let rt = Runtime::new();
/// let query = rt.signal(String::from("Ban"));
/// let list = rt.signal(MenuState::new());
///
/// let city = combo_box(query.get())
///     .label("City")
///     .suggestions(["Bandung", "Banjarmasin"])
///     .bind(list)
///     .on_change(move |s| query.set(s.to_owned()));
/// # let _ = city;
/// ```
///
/// Use [`combo_box_in`] outside a build pass.
pub fn combo_box(value: impl Into<String>) -> ComboBox {
    combo_box_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        value,
    )
}

/// [`combo_box`] with the text engine and the theme passed explicitly.
pub fn combo_box_in(fonts: &Fonts, theme: &Theme, value: impl Into<String>) -> ComboBox {
    ComboBox {
        fonts: fonts.clone(),
        theme: *theme,
        value: value.into(),
        suggestions: Vec::new(),
        state: MenuState::new(),
        bound: None,
        placeholder: String::new(),
        label: None,
        disabled: false,
        spring: Spring::snappy(),
        on_change: None,
        on_submit: None,
        on_select: None,
        on_intent: None,
        key: String::from("silka-combo"),
    }
}

impl ComboBox {
    /// The suggestions to show, **already filtered** (see the module docs).
    pub fn suggestions<S: Into<String>>(mut self, items: impl IntoIterator<Item = S>) -> Self {
        self.suggestions = items.into_iter().map(Into::into).collect();
        self
    }

    /// The list's state, fully controlled by the application.
    pub fn state(mut self, state: MenuState) -> Self {
        self.state = state;
        self
    }

    /// Wire it to a single signal: reading it **and** writing to it.
    ///
    /// The shape 95% of applications want — one piece of state to keep, and
    /// every rule about the highlight and the closing already right, because it
    /// all goes through [`MenuState::apply`].
    pub fn bind(mut self, state: Signal<MenuState>) -> Self {
        // Read **during build**, so the component calling it subscribes.
        self.state = state.get();
        self.bound = Some(state);
        self
    }

    /// Force the list open or closed.
    pub fn open(mut self, open: bool) -> Self {
        self.state.open = open;
        self
    }

    /// The faint text shown while the field is empty.
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// The name a screen reader announces (§3.8) — given to the field **and**
    /// to the group around it.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Disable the whole control.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The spring that drives the field's and the panel's transitions.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Called every time the typed text changes — **without** the IME preedit.
    pub fn on_change(mut self, f: impl Fn(&str) + 'static) -> Self {
        self.on_change = Some(TextCallback::new(f));
        self
    }

    /// Called on Return when **nothing** is highlighted: "I meant what I typed".
    pub fn on_submit(mut self, f: impl Fn(&str) + 'static) -> Self {
        self.on_submit = Some(TextCallback::new(f));
        self
    }

    /// Called when one of the suggestions is taken, by click or by Return.
    pub fn on_select(mut self, f: impl Fn(usize, &str) + 'static) -> Self {
        self.on_select = Some(PickCallback::new(f));
        self
    }

    /// Receive every intent raw — the path for applications managing their own
    /// state instead of binding a signal.
    pub fn on_intent(mut self, f: impl Fn(MenuIntent) + 'static) -> Self {
        self.on_intent = Some(MenuHandler::new(f));
        self
    }

    /// The identity prefix for this combo box's nodes (§2.5).
    ///
    /// **Give every combo box on a page its own prefix**: two of them mounted
    /// in the same [`overlay_layer`](crate::overlay::overlay_layer) with the
    /// default prefix would hand their panels the same key, and diffing would
    /// happily reuse one's node for the other's panel.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }

    // -- readers -------------------------------------------------------------

    /// The suggestions currently on offer.
    pub fn suggestion_list(&self) -> &[String] {
        &self.suggestions
    }

    /// The state currently in effect.
    pub fn state_value(&self) -> &MenuState {
        &self.state
    }

    /// The highlighted suggestion, if any.
    pub fn highlight(&self) -> Option<usize> {
        self.state.highlight_at(0)
    }

    /// True when the panel should actually be on screen.
    ///
    /// Three conditions, and the anchor is the one that is easy to forget: a
    /// list opened by a keystroke has no rect until [`sync`] has run, and
    /// showing it before then would place it in the middle of the window.
    pub fn is_open(&self) -> bool {
        self.state.open
            && self.state.anchor.is_some()
            && !self.suggestions.is_empty()
            && !self.disabled
    }

    /// The suggestions as a menu model — the shape [`MenuState::apply`] reads.
    pub fn model(&self) -> MenuModel {
        MenuModel::new(
            self.suggestions
                .iter()
                .enumerate()
                .map(|(i, s)| MenuEntry::from(item(i.to_string(), s.clone())))
                .collect::<Vec<_>>(),
        )
    }

    // -- the pieces mounted in two places ------------------------------------

    /// The field, to be mounted in the page content.
    pub fn field(&self) -> View {
        let t = &self.theme;
        let mut field = text_field_in(&self.fonts, t, self.value.clone())
            .placeholder(self.placeholder.clone())
            .disabled(self.disabled)
            .spring(self.spring)
            // The one thing that had to change in `text_field`: ↑/↓ belong to
            // the list, so the field must not spend them on the caret.
            .arrow_keys(ArrowKeys::Bubble);
        if let Some(label) = &self.label {
            field = field.label(label.clone());
        }
        if let Some(cb) = &self.on_change {
            // Typing invalidates the highlight: the suggestion that was under
            // it is not the one under it now. Resetting here rather than in the
            // application is what stops Return from taking a stale choice.
            let outer = cb.clone();
            let handler = self.handler();
            field = field.on_change(move |text| {
                outer.call(text);
                handler.emit(MenuIntent::Highlight {
                    depth: 0,
                    index: None,
                });
            });
        }

        Builder::new(ComboFieldProps {
            count: self.suggestions.len(),
            open: self.state.open,
            has_anchor: self.state.anchor.is_some(),
            highlight: self.highlight(),
            disabled: self.disabled,
            text: self.value.clone(),
            label: self.label.clone(),
            on_intent: Some(self.handler()),
            on_submit: self.on_submit.clone(),
        })
        .key(format!("{}::field", self.key))
        .child(field)
        .into()
    }

    /// The suggestion panel, to be mounted in the
    /// [`overlay_layer`](crate::overlay::overlay_layer).
    ///
    /// A `Vec` rather than a single builder so the shape matches
    /// [`crate::menu::Menu::overlays`] exactly — a combo box has no submenus, so
    /// today it is always one panel.
    pub fn overlays(&self) -> Vec<OverlayBuilder> {
        let handler = self.handler();
        let mut state = self.state.clone();
        // A list with nothing in it is not a list: an empty panel that still
        // takes the pointer is how a click near a search box stops working.
        state.open = self.is_open();
        // The panel lines up with the field, which is the one thing a menu's
        // own "as wide as my longest label" rule cannot know. The width comes
        // from the anchor, so nothing has to be measured twice.
        let width = match state.anchor {
            Anchor::Rect(r) => Some(r.size.width),
            _ => None,
        };

        let mut m = menu_in(
            &self.fonts,
            &self.theme,
            self.suggestions
                .iter()
                .enumerate()
                .map(|(i, s)| MenuEntry::from(item(i.to_string(), s.clone())))
                .collect::<Vec<_>>(),
        )
        .state(state)
        .spring(self.spring)
        .key(self.key.clone())
        .on_intent(move |intent| handler.emit(intent));
        if let Some(label) = &self.label {
            m = m.label(label.clone());
        }
        if let Some(w) = width {
            m = m.min_width(w);
        }
        m.overlays()
    }

    /// The one handler both halves send their intents to.
    ///
    /// It does exactly three things, in an order that matters: resolve the
    /// chosen suggestion **before** applying (applying closes the list and
    /// clears the highlight), write the new state into the bound signal, and
    /// pass the raw intent on to an application that asked for it.
    fn handler(&self) -> MenuHandler {
        let model = self.model();
        let bound = self.bound;
        let picked = self.on_select.clone();
        let outer = self.on_intent.clone();
        let items: Rc<Vec<String>> = Rc::new(self.suggestions.clone());
        MenuHandler::new(move |intent| {
            if let (MenuIntent::Activate { depth: 0, index }, Some(f)) = (intent, &picked) {
                if let Some(text) = items.get(index) {
                    f.call(index, text);
                }
            }
            if let Some(sig) = bound {
                // `peek`, not `get`: the handler runs outside of build, and
                // subscribing from inside an event handler is never right.
                let mut next = sig.peek();
                if next.apply(intent, &model) {
                    sig.set(next);
                }
            }
            if let Some(h) = &outer {
                h.emit(intent);
            }
        })
    }
}

impl core::fmt::Debug for ComboBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ComboBox")
            .field("value", &self.value)
            .field("suggestions", &self.suggestions.len())
            .field("open", &self.state.open)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Frame pass
// ---------------------------------------------------------------------------

/// Every node of the tree, parent before child.
fn all(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    collect(tree, tree.root(), &mut out);
    out
}

fn collect(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    out.push(id);
    for child in tree.children(id) {
        collect(tree, *child, out);
    }
}

/// The nearest [`OverlayLayer`] above `id` — the coordinate space every anchor
/// is expressed in.
fn layer_of(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
    let mut current = tree.parent(id);
    while let Some(n) = current {
        if tree.node_ref::<OverlayLayer>(n).is_some() {
            return Some(n);
        }
        current = tree.parent(n);
    }
    None
}

/// Publish the field's rect for any combo box whose list has just opened.
///
/// This is the geometry the view layer could not know when it was built: a node
/// never learns its own position, so a list opened by ↓ leaves a request behind
/// ([`ComboFieldBox::wants_anchor`]) and it is answered here, after this frame's
/// layout has settled. The same seam [`crate::menu::advance`] and
/// [`crate::list::sync_virtual`] use, for the same reason.
///
/// Called once per frame by [`crate::advance`], so an application never has to
/// call it directly.
pub fn sync(tree: &mut RenderTree) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in all(tree) {
        if !tree
            .node_ref::<ComboFieldBox>(id)
            .is_some_and(ComboFieldBox::wants_anchor)
        {
            continue;
        }
        let anchor = match layer_of(tree, id) {
            Some(layer) => anchor_rect(tree, id, layer),
            // No overlay layer above us: the application forgot to mount one,
            // and the honest answer is "no anchor" rather than a rect in a
            // coordinate space that does not exist.
            None => Anchor::None,
        };
        if !anchor.is_some() {
            continue;
        }
        if let Some(n) = tree.node_ref::<ComboFieldBox>(id) {
            n.emit(MenuIntent::Open(anchor));
        }
        dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
    }
    dirty
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::input::{InputRouter, KeyEvent};
    use silka_core::signals::Runtime;
    use silka_core::tree::RenderTree;
    use silka_core::view::{column, reconcile};
    use silka_paint::Rect;
    use silka_theme::Appearance;
    use std::cell::RefCell;
    use std::time::Duration;

    const BOX: Size = Size::new(360.0, 240.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    fn cities() -> [&'static str; 3] {
        ["Bandung", "Banjarmasin", "Denpasar"]
    }

    /// The field mounted inside an overlay layer, exactly as an application
    /// would mount it — otherwise there is no coordinate space for the anchor.
    fn mounted(combo: &ComboBox) -> RenderTree {
        let mut layer = crate::overlay_layer(column([combo.field()]));
        for panel in combo.overlays() {
            layer = layer.overlay(panel);
        }
        let mut tree = RenderTree::new();
        reconcile(&mut tree, layer);
        tree.layout(BoxConstraints::tight(BOX));
        tree
    }

    fn find(tree: &RenderTree) -> NodeId {
        all(tree)
            .into_iter()
            .find(|id| tree.node_ref::<ComboFieldBox>(*id).is_some())
            .expect("the combo box wrapper is in the tree")
    }

    /// Focus the text field inside the wrapper and send it one key.
    fn key(tree: &mut RenderTree, named: NamedKey) {
        let wrapper = find(tree);
        let field = tree.children(wrapper)[0];
        let mut router = InputRouter::new();
        router.focus_node(tree, Some(field));
        router.dispatch(
            tree,
            &Event::Key(KeyEvent::pressed(KeyCode::Named(named), Duration::ZERO)),
        );
    }

    // -- keys ---------------------------------------------------------------

    #[test]
    fn the_down_arrow_opens_a_closed_list() {
        let rt = Runtime::new();
        let state = rt.signal(MenuState::new());
        let combo = combo_box_in(&fonts(), &theme(), "Ban")
            .suggestions(cities())
            .bind(state);
        let mut tree = mounted(&combo);

        key(&mut tree, NamedKey::ArrowDown);
        assert!(state.get().open, "↓ is the key everybody tries first");
    }

    #[test]
    fn the_arrows_walk_the_open_list() {
        let rt = Runtime::new();
        let state = rt.signal(MenuState {
            open: true,
            anchor: Anchor::Rect(Rect::new(0.0, 0.0, 200.0, 44.0)),
            ..MenuState::new()
        });
        let combo = combo_box_in(&fonts(), &theme(), "Ban")
            .suggestions(cities())
            .bind(state);
        let mut tree = mounted(&combo);

        key(&mut tree, NamedKey::ArrowDown);
        assert_eq!(state.get().highlight, Some(0));

        let combo = combo_box_in(&fonts(), &theme(), "Ban")
            .suggestions(cities())
            .bind(state);
        let mut tree = mounted(&combo);
        key(&mut tree, NamedKey::ArrowDown);
        assert_eq!(state.get().highlight, Some(1));
    }

    #[test]
    fn return_takes_the_highlighted_suggestion() {
        let rt = Runtime::new();
        let state = rt.signal(MenuState {
            open: true,
            anchor: Anchor::Rect(Rect::new(0.0, 0.0, 200.0, 44.0)),
            highlight: Some(1),
            ..MenuState::new()
        });
        let taken = Rc::new(RefCell::new(String::new()));
        let sink = taken.clone();
        let combo = combo_box_in(&fonts(), &theme(), "Ban")
            .suggestions(cities())
            .bind(state)
            .on_select(move |_, s| *sink.borrow_mut() = s.to_string());
        let mut tree = mounted(&combo);

        key(&mut tree, NamedKey::Enter);
        assert_eq!(taken.borrow().as_str(), "Banjarmasin");
        assert!(!state.get().open, "choosing closes the list");
    }

    #[test]
    fn return_with_nothing_highlighted_is_a_submit_not_a_pick() {
        let rt = Runtime::new();
        let state = rt.signal(MenuState {
            open: true,
            anchor: Anchor::Rect(Rect::new(0.0, 0.0, 200.0, 44.0)),
            ..MenuState::new()
        });
        let submitted = Rc::new(RefCell::new(String::new()));
        let sink = submitted.clone();
        let combo = combo_box_in(&fonts(), &theme(), "Ban")
            .suggestions(cities())
            .bind(state)
            .on_submit(move |s| *sink.borrow_mut() = s.to_string())
            .on_select(|_, _| panic!("nothing was highlighted"));
        let mut tree = mounted(&combo);

        key(&mut tree, NamedKey::Enter);
        assert_eq!(submitted.borrow().as_str(), "Ban");
    }

    #[test]
    fn escape_closes_the_list_and_keeps_what_was_typed() {
        let rt = Runtime::new();
        let state = rt.signal(MenuState {
            open: true,
            anchor: Anchor::Rect(Rect::new(0.0, 0.0, 200.0, 44.0)),
            ..MenuState::new()
        });
        let combo = combo_box_in(&fonts(), &theme(), "Ban")
            .suggestions(cities())
            .bind(state);
        let mut tree = mounted(&combo);

        key(&mut tree, NamedKey::Escape);
        assert!(!state.get().open);

        // The field still holds the text: Esc closes a list, it does not undo a
        // sentence.
        let wrapper = find(&tree);
        let field = tree.children(wrapper)[0];
        let text = tree
            .node_ref::<crate::text_field::TextFieldBox>(field)
            .expect("a text field node")
            .text()
            .to_string();
        assert_eq!(text, "Ban");
    }

    #[test]
    fn a_letter_still_reaches_the_field() {
        let rt = Runtime::new();
        let state = rt.signal(MenuState::new());
        let typed = Rc::new(RefCell::new(String::new()));
        let sink = typed.clone();
        let combo = combo_box_in(&fonts(), &theme(), "Ban")
            .suggestions(cities())
            .bind(state)
            .on_change(move |s| *sink.borrow_mut() = s.to_string());
        let mut tree = mounted(&combo);

        let wrapper = find(&tree);
        let field = tree.children(wrapper)[0];
        let mut router = InputRouter::new();
        router.focus_node(&mut tree, Some(field));
        let mut ev = KeyEvent::pressed(KeyCode::Character('d'), Duration::ZERO);
        ev.text = Some(String::from("d"));
        router.dispatch(&mut tree, &Event::Key(ev));

        assert_eq!(typed.borrow().as_str(), "Band");
    }

    // -- the anchor seam ----------------------------------------------------

    #[test]
    fn a_list_opened_by_keyboard_gets_its_rect_one_frame_later() {
        let rt = Runtime::new();
        let state = rt.signal(MenuState::new());
        let build = || {
            combo_box_in(&fonts(), &theme(), "Ban")
                .suggestions(cities())
                .bind(state)
        };
        let mut tree = mounted(&build());

        key(&mut tree, NamedKey::ArrowDown);
        assert!(state.get().open);
        assert!(
            !state.get().anchor.is_some(),
            "the rect cannot exist before a layout pass has placed the field"
        );

        // Rebuild with the new state, lay out, then run the sync pass — the
        // exact order a frame runs in.
        let combo = build();
        assert!(!combo.is_open(), "an unanchored panel must stay hidden");
        let mut tree2 = mounted(&combo);
        sync(&mut tree2);
        assert!(state.get().anchor.is_some());

        let combo = build();
        assert!(combo.is_open(), "with a rect it is finally shown");
    }

    #[test]
    fn a_list_with_no_suggestions_never_opens() {
        let rt = Runtime::new();
        let state = rt.signal(MenuState::new());
        let combo = combo_box_in(&fonts(), &theme(), "zzz")
            .suggestions(Vec::<String>::new())
            .bind(state);
        let mut tree = mounted(&combo);

        key(&mut tree, NamedKey::ArrowDown);
        assert!(!state.get().open);
        assert!(!combo.is_open());
    }

    // -- contract -----------------------------------------------------------

    #[test]
    fn a_screen_reader_hears_a_group_that_can_be_opened() {
        let combo = combo_box_in(&fonts(), &theme(), "Ban")
            .label("City")
            .suggestions(cities());
        let tree = mounted(&combo);
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("City")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Group);
        assert_eq!(e.node.expanded, Some(false));
        assert!(e.node.actions.contains(AccessActions::EXPAND));
    }

    #[test]
    fn with_no_suggestions_it_is_not_something_that_can_be_opened_at_all() {
        let combo = combo_box_in(&fonts(), &theme(), "zzz")
            .label("City")
            .suggestions(Vec::<String>::new());
        let tree = mounted(&combo);
        let a11y = tree.access_tree(None);
        let e = a11y.find_label("City").expect("the group is announced");
        assert_eq!(
            e.node.expanded, None,
            "`Some(false)` would make a screen reader say \"collapsed\" about \
             something that cannot be opened"
        );
    }

    #[test]
    fn the_wrapper_adds_no_tab_stop_and_no_box_of_its_own() {
        let combo = combo_box_in(&fonts(), &theme(), "Ban").suggestions(cities());
        let tree = mounted(&combo);
        let wrapper = find(&tree);
        assert!(!tree
            .render(wrapper)
            .map(|r| r.focus_policy().focusable)
            .unwrap_or(false));
        assert_eq!(tree.size(wrapper), tree.size(tree.children(wrapper)[0]));
    }

    #[test]
    fn the_panel_is_never_narrower_than_the_field() {
        let combo = combo_box_in(&fonts(), &theme(), "Ban")
            .suggestions(["a", "b"])
            .state(MenuState {
                open: true,
                anchor: Anchor::Rect(Rect::new(0.0, 0.0, 300.0, 44.0)),
                ..MenuState::new()
            });
        assert!(combo.is_open());
        assert_eq!(combo.overlays().len(), 1, "a combo box has no submenus");

        // The width travels through the anchor, so nothing is measured twice:
        // two one-letter suggestions would otherwise give a sliver of a panel.
        let entries: Vec<MenuEntry> = ["a", "b"]
            .iter()
            .enumerate()
            .map(|(i, s)| MenuEntry::from(item(i.to_string(), *s)))
            .collect();
        let bare = menu_in(&fonts(), &theme(), entries.clone()).level_width(0);
        let wide = menu_in(&fonts(), &theme(), entries)
            .min_width(300.0)
            .level_width(0);
        assert!(bare < 300.0);
        assert_eq!(wide, 300.0);
    }

    #[test]
    fn rebuilding_an_identical_combo_box_costs_nothing() {
        let combo = combo_box_in(&fonts(), &theme(), "Ban").suggestions(cities());
        let mut tree = RenderTree::new();
        reconcile(&mut tree, column([combo.field()]));
        tree.layout(BoxConstraints::loose(BOX));

        let same = combo_box_in(&fonts(), &theme(), "Ban").suggestions(cities());
        let stats = reconcile(&mut tree, column([same.field()]));
        assert_eq!(stats.created, 0);
        assert_eq!(stats.replaced, 0);
    }
}
