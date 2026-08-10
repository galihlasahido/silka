//! The accessibility node vocabulary: roles, actions and the content **filled
//! in by widgets**.
//!
//! This vocabulary is our own and maps 1:1 onto `accesskit` in
//! [`super::bridge`] — exactly the pattern `silka-paint` uses towards wgpu
//! (§3.2): widget code never touches a third-party type, so that library can
//! be swapped or deferred without touching a single widget.
//!
//! What is **not** here are `bounds` and the child list. Neither may ever be
//! filled in by a widget, because only the layout result knows the truth; they
//! belong on [`super::AccessEntry`], not on [`AccessNode`]. That rule is
//! enforced by the types, not by a comment.

use core::fmt;

/// A node's role for assistive technology (screen readers).
///
/// `#[non_exhaustive]`: the list of roles grows alongside `KOMPONEN.md`.
///
/// ```
/// use silka_core::access::AccessRole;
///
/// // The default is structural, and structural nodes are filtered out of the
/// // tree assistive technology sees — so a node that forgot to state its role
/// // can never make a screen reader announce an empty container.
/// assert_eq!(AccessRole::default(), AccessRole::Container);
/// assert!(AccessRole::Container.is_structural());
/// assert!(!AccessRole::Button.is_structural());
/// assert_eq!(AccessRole::Button.name(), "button");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum AccessRole {
    /// A purely structural container (padding, align, constrained box).
    ///
    /// This node is **filtered out** of the tree assistive technology sees —
    /// its children rise up to take its place. And it is the default: a node
    /// that forgot to state its role will never make a screen reader announce
    /// an empty container.
    #[default]
    Container,
    /// A window or the root of a tree.
    Window,
    /// A meaningful grouping (row/column/stack, fieldset).
    Group,
    /// Static text.
    Label,
    /// A pressable button.
    Button,
    /// A link.
    Link,
    /// A single-line text field.
    TextInput,
    /// A multi-line text field.
    MultilineTextInput,
    /// A checkbox (which may be `Mixed`/indeterminate).
    CheckBox,
    /// A radio button.
    RadioButton,
    /// An on/off switch.
    Switch,
    /// A value slider.
    Slider,
    /// A stepped increment/decrement control.
    Stepper,
    /// A scrollable container.
    ScrollView,
    /// A meaningful image/icon.
    Image,
    /// A list.
    List,
    /// A single list row.
    ListItem,
    /// A single tab.
    Tab,
    /// A row of tabs.
    TabList,
    /// A modal dialog.
    Dialog,
    /// A menu.
    Menu,
    /// A single menu item.
    MenuItem,
    /// A progress indicator.
    ProgressIndicator,
    /// A separator line.
    Separator,
    /// A toolbar.
    Toolbar,
    /// A tooltip.
    Tooltip,
    /// A table.
    Table,
    /// A table row.
    Row,
    /// A table cell.
    Cell,
    /// A hierarchical list (outline view).
    Tree,
    /// A single row of a hierarchical list.
    TreeItem,
}

impl AccessRole {
    /// A short name for tree dumps and the inspector.
    pub const fn name(self) -> &'static str {
        match self {
            AccessRole::Container => "container",
            AccessRole::Window => "window",
            AccessRole::Group => "group",
            AccessRole::Label => "label",
            AccessRole::Button => "button",
            AccessRole::Link => "link",
            AccessRole::TextInput => "text_input",
            AccessRole::MultilineTextInput => "text_area",
            AccessRole::CheckBox => "checkbox",
            AccessRole::RadioButton => "radio",
            AccessRole::Switch => "switch",
            AccessRole::Slider => "slider",
            AccessRole::Stepper => "stepper",
            AccessRole::ScrollView => "scroll_view",
            AccessRole::Image => "image",
            AccessRole::List => "list",
            AccessRole::ListItem => "list_item",
            AccessRole::Tab => "tab",
            AccessRole::TabList => "tab_list",
            AccessRole::Dialog => "dialog",
            AccessRole::Menu => "menu",
            AccessRole::MenuItem => "menu_item",
            AccessRole::ProgressIndicator => "progress",
            AccessRole::Separator => "separator",
            AccessRole::Toolbar => "toolbar",
            AccessRole::Tooltip => "tooltip",
            AccessRole::Table => "table",
            AccessRole::Row => "row",
            AccessRole::Cell => "cell",
            AccessRole::Tree => "tree",
            AccessRole::TreeItem => "tree_item",
        }
    }

    /// True when this role is structure only and should be filtered out by
    /// assistive technology (the counterpart of AccessKit's
    /// `GenericContainer` / ARIA's `role="none"`).
    pub const fn is_structural(self) -> bool {
        matches!(self, AccessRole::Container)
    }
}

impl fmt::Display for AccessRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// One action **requested** by assistive technology against a node.
///
/// The difference from [`AccessActions`]: this is a single incoming request
/// (VoiceOver pressing a button), whereas [`AccessActions`] is the set of
/// capabilities a node advertises outwards.
///
/// ```
/// use silka_core::access::{AccessAction, AccessActions};
///
/// // Every incoming action names the capability a node must advertise for it
/// // to be legal — which is what the platform adapter checks before
/// // delivering it.
/// assert_eq!(AccessAction::Click.capability(), AccessActions::CLICK);
///
/// let button = AccessActions::CLICK.union(AccessActions::FOCUS);
/// assert!(button.contains(AccessAction::Focus.capability()));
/// assert!(!button.contains(AccessAction::Increment.capability()));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AccessAction {
    /// Primary activation (pressing a button, selecting a row).
    Click,
    /// Move keyboard focus to this node.
    Focus,
    /// Remove keyboard focus from this node.
    Blur,
    /// Raise the value by one step (slider, stepper).
    Increment,
    /// Lower the value by one step.
    Decrement,
    /// Open/expand (disclosure, accordion, combo box).
    Expand,
    /// Close/collapse.
    Collapse,
    /// Replace the content (dictated text, a numeric value).
    SetValue,
    /// Open the context menu.
    ShowContextMenu,
    /// Scroll up by one unit.
    ScrollUp,
    /// Scroll down by one unit.
    ScrollDown,
    /// Scroll left by one unit.
    ScrollLeft,
    /// Scroll right by one unit.
    ScrollRight,
    /// Scroll whichever container is needed so this node becomes visible.
    ScrollIntoView,
}

impl AccessAction {
    /// The capability a node must advertise before this action may be
    /// requested.
    ///
    /// Used to **reject illegitimate requests** before they reach the widget:
    /// assistive technology works from a snapshot of the tree that may well be
    /// a frame out of date.
    pub const fn capability(self) -> AccessActions {
        match self {
            AccessAction::Click => AccessActions::CLICK,
            AccessAction::Focus | AccessAction::Blur => AccessActions::FOCUS,
            AccessAction::Increment => AccessActions::INCREMENT,
            AccessAction::Decrement => AccessActions::DECREMENT,
            AccessAction::Expand => AccessActions::EXPAND,
            AccessAction::Collapse => AccessActions::COLLAPSE,
            AccessAction::SetValue => AccessActions::SET_VALUE,
            AccessAction::ShowContextMenu => AccessActions::CONTEXT_MENU,
            AccessAction::ScrollUp
            | AccessAction::ScrollDown
            | AccessAction::ScrollLeft
            | AccessAction::ScrollRight
            | AccessAction::ScrollIntoView => AccessActions::SCROLL,
        }
    }

    /// A short name for debugging/dumps.
    pub const fn name(self) -> &'static str {
        match self {
            AccessAction::Click => "click",
            AccessAction::Focus => "focus",
            AccessAction::Blur => "blur",
            AccessAction::Increment => "increment",
            AccessAction::Decrement => "decrement",
            AccessAction::Expand => "expand",
            AccessAction::Collapse => "collapse",
            AccessAction::SetValue => "set_value",
            AccessAction::ShowContextMenu => "context_menu",
            AccessAction::ScrollUp => "scroll_up",
            AccessAction::ScrollDown => "scroll_down",
            AccessAction::ScrollLeft => "scroll_left",
            AccessAction::ScrollRight => "scroll_right",
            AccessAction::ScrollIntoView => "scroll_into_view",
        }
    }
}

impl fmt::Display for AccessAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// An action request from assistive technology, in our own vocabulary.
///
/// Produced by the platform adapter after two validations: the target node
/// still exists in the tree that was last sent, **and** the action really is
/// advertised by that node. Assistive technology works from a snapshot that
/// may be a frame out of date; without those checks, a click could land on the
/// wrong widget.
///
/// Assistive technology works from a snapshot that may be a frame out of date,
/// so an unvalidated request is a click landing on whatever moved into that
/// slot since.
///
/// ```
/// use silka_core::access::{AccessAction, AccessActionRequest, AccessRole};
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::{column, fixed, interactive, reconcile, View};
/// use silka_paint::Size;
///
/// let mut tree = RenderTree::new();
/// reconcile(
///     &mut tree,
///     column([View::from(
///         interactive(fixed(120.0, 44.0))
///             .role(AccessRole::Button)
///             .label("Save"),
///     )]),
/// );
/// tree.layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
///
/// let access = tree.access_tree(None);
/// let button = access.find_label("Save").expect("the button is announced");
///
/// // VoiceOver asks for the primary action on the node it last saw.
/// let request = AccessActionRequest {
///     target: button.id,
///     action: AccessAction::Click,
///     value: None,
/// };
///
/// // The adapter validates both halves before dispatching: the node still
/// // exists, *and* it really advertises that action. A button does not
/// // advertise `Increment`, so such a request is dropped rather than guessed at.
/// assert!(tree.contains(request.target));
/// assert!(button.node.actions.contains(AccessAction::Click.capability()));
/// assert!(!button.node.actions.contains(AccessAction::Increment.capability()));
///
/// // `value` only travels with `SetValue` — dictation into a text field.
/// let dictated = AccessActionRequest {
///     target: button.id,
///     action: AccessAction::SetValue,
///     value: Some("Hello".into()),
/// };
/// assert_eq!(dictated.value.as_deref(), Some("Hello"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessActionRequest {
    /// The target node.
    pub target: crate::tree::NodeId,
    /// The action requested.
    pub action: AccessAction,
    /// The new content for [`AccessAction::SetValue`] (voice dictation,
    /// refilling a field).
    pub value: Option<String>,
}

/// The set of capabilities a node **advertises**, as a bitset.
///
/// ```
/// use silka_core::access::AccessActions;
///
/// // A slider advertises what a screen reader may actually do to it.
/// let slider = AccessActions::FOCUS
///     .union(AccessActions::INCREMENT)
///     .union(AccessActions::DECREMENT);
///
/// assert!(slider.contains(AccessActions::INCREMENT));
/// assert!(!slider.contains(AccessActions::CLICK));
/// assert!(AccessActions::NONE.is_empty());
///
/// // The names are what a golden a11y dump prints.
/// assert!(slider.names().any(|n| n == "increment"));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct AccessActions(u16);

impl AccessActions {
    /// No actions.
    pub const NONE: Self = Self(0);
    /// Can be activated (click/tap/Enter).
    pub const CLICK: Self = Self(1 << 0);
    /// Can take keyboard focus.
    pub const FOCUS: Self = Self(1 << 1);
    /// Can be scrolled.
    pub const SCROLL: Self = Self(1 << 2);
    /// Its value can be increased.
    pub const INCREMENT: Self = Self(1 << 3);
    /// Its value can be decreased.
    pub const DECREMENT: Self = Self(1 << 4);
    /// Can be expanded.
    pub const EXPAND: Self = Self(1 << 5);
    /// Can be collapsed.
    pub const COLLAPSE: Self = Self(1 << 6);
    /// Its content can be replaced outright (voice dictation, refilling a
    /// field).
    pub const SET_VALUE: Self = Self(1 << 7);
    /// Has a context menu.
    pub const CONTEXT_MENU: Self = Self(1 << 8);

    const NAMES: [(Self, &'static str); 9] = [
        (Self::CLICK, "click"),
        (Self::FOCUS, "focus"),
        (Self::SCROLL, "scroll"),
        (Self::INCREMENT, "increment"),
        (Self::DECREMENT, "decrement"),
        (Self::EXPAND, "expand"),
        (Self::COLLAPSE, "collapse"),
        (Self::SET_VALUE, "set_value"),
        (Self::CONTEXT_MENU, "context_menu"),
    ];

    /// The raw bits.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// True when there are no actions at all.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The union of two action sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// True when every action in `other` is present here.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Add an action.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Remove an action.
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    /// The name of each bit that is set, in a stable order — used by tree
    /// dumps.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Self::NAMES
            .into_iter()
            .filter(move |(bit, _)| self.contains(*bit))
            .map(|(_, name)| name)
    }
}

impl core::ops::BitOr for AccessActions {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for AccessActions {
    fn bitor_assign(&mut self, rhs: Self) {
        self.insert(rhs);
    }
}

impl From<AccessAction> for AccessActions {
    fn from(action: AccessAction) -> Self {
        action.capability()
    }
}

impl fmt::Debug for AccessActions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("AccessActions(none)");
        }
        f.write_str("AccessActions(")?;
        for (i, name) in self.names().enumerate() {
            if i > 0 {
                f.write_str("|")?;
            }
            f.write_str(name)?;
        }
        f.write_str(")")
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// A three-state value for checkboxes/switches/menu items.
///
/// Three, not two: an indeterminate checkbox has to reach a screen reader as
/// "mixed" rather than as a button whose *name* keeps changing.
///
/// ```
/// use silka_core::access::AccessToggled;
///
/// assert_eq!(AccessToggled::Mixed.name(), "mixed");
/// assert_ne!(AccessToggled::Mixed, AccessToggled::On);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessToggled {
    /// Off.
    Off,
    /// On.
    On,
    /// Partially (an indeterminate checkbox, `KOMPONEN.md` Tier 2).
    Mixed,
}

impl AccessToggled {
    /// A short name for dumps.
    pub const fn name(self) -> &'static str {
        match self {
            AccessToggled::Off => "off",
            AccessToggled::On => "on",
            AccessToggled::Mixed => "mixed",
        }
    }
}

impl From<bool> for AccessToggled {
    fn from(v: bool) -> Self {
        if v {
            AccessToggled::On
        } else {
            AccessToggled::Off
        }
    }
}

/// Where the caret sits, and how far the selection reaches, inside an editable
/// node's [`AccessNode::value`].
///
/// Both ends are **character** offsets (not bytes): that is the unit assistive
/// technology counts in, and the unit `AccessKit` positions in. `anchor` is the
/// end that stays put while the selection is being extended, `focus` the end
/// that moves; the two being equal means "just a caret, nothing selected" —
/// what AccessKit calls a degenerate selection.
///
/// This is what lets a screen reader announce *"line 3, character 12"* and
/// follow a selection as it grows, instead of merely re-reading the whole
/// content after every keystroke.
///
/// Indices are in **characters**, not bytes, because that is what assistive
/// technology counts — [`AccessTextSelection::from_bytes`] does the conversion
/// from the editing model's byte offsets.
///
/// ```
/// use silka_core::access::AccessTextSelection;
///
/// assert!(AccessTextSelection::caret(3).is_collapsed());
///
/// // Selecting backwards is normal; `range` orders the pair.
/// let backwards = AccessTextSelection::new(7, 2);
/// assert_eq!(backwards.range(), 2..7);
///
/// // "café" is 5 bytes but 4 characters — the difference a screen reader
/// // would otherwise announce wrongly.
/// assert_eq!(AccessTextSelection::from_bytes("café", 0, 5).focus, 4);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccessTextSelection {
    /// The fixed end of the selection, in characters from the start of the
    /// value.
    pub anchor: usize,
    /// The moving end — the caret itself.
    pub focus: usize,
}

impl AccessTextSelection {
    /// A caret with nothing selected.
    pub const fn caret(at: usize) -> Self {
        Self {
            anchor: at,
            focus: at,
        }
    }

    /// A selection running from `anchor` to `focus`.
    pub const fn new(anchor: usize, focus: usize) -> Self {
        Self { anchor, focus }
    }

    /// True when nothing is selected.
    pub const fn is_collapsed(self) -> bool {
        self.anchor == self.focus
    }

    /// The selected range, ordered.
    pub fn range(self) -> core::ops::Range<usize> {
        let (a, b) = (self.anchor.min(self.focus), self.anchor.max(self.focus));
        a..b
    }

    /// Translate a **byte** range in `text` into character offsets.
    ///
    /// Widgets store byte indices (that is what `silka-text` edits in);
    /// assistive technology counts characters. Converting here, once, keeps
    /// every widget from getting it subtly wrong on its own.
    pub fn from_bytes(text: &str, anchor: usize, focus: usize) -> Self {
        // Counted by walking, not by slicing: an index that lands inside a
        // character must come back as a number, never as a panic (§9.7).
        let hitung = |batas: usize| text.char_indices().take_while(|(i, _)| *i < batas).count();
        Self {
            anchor: hitung(anchor),
            focus: hitung(focus),
        }
    }
}

/// The part of an accessibility node that is **filled in by the widget**.
///
/// This is half of the [`crate::tree::RenderNode::access`] contract. The other
/// half — `bounds`, parent and children — comes from the layout result and is
/// assembled by the engine in [`super::AccessEntry`], so it cannot go stale
/// with respect to what is drawn.
///
/// ```
/// use silka_core::access::{AccessActions, AccessNode, AccessRole, AccessToggled};
///
/// // What a checkbox fills in. Note what is absent: `bounds`. A widget never
/// // holds it, so it structurally cannot describe a box other than the one
/// // that was laid out.
/// let node = AccessNode::with_role(AccessRole::CheckBox)
///     .label("Sync automatically")
///     .with_actions(AccessActions::CLICK.union(AccessActions::FOCUS))
///     .toggled(AccessToggled::Mixed);
///
/// assert_eq!(node.role, AccessRole::CheckBox);
/// assert_eq!(node.label.as_deref(), Some("Sync automatically"));
/// assert_eq!(node.toggled, Some(AccessToggled::Mixed));
/// assert!(!node.hidden);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccessNode {
    /// The node's role.
    pub role: AccessRole,
    /// The name a screen reader announces.
    pub label: Option<String>,
    /// The current value (a text field's content, a slider position as text).
    pub value: Option<String>,
    /// The capabilities advertised.
    pub actions: AccessActions,
    /// Hide the node **and all its descendants** from assistive technology.
    ///
    /// For pure decoration (shadows, ornamental lines) and for content that is
    /// currently being animated off-screen.
    pub hidden: bool,
    /// Present but unusable — still announced, with a "dimmed" status.
    pub disabled: bool,
    /// The on/off/mixed state, where the concept applies.
    pub toggled: Option<AccessToggled>,
    /// Selected or not, where the concept applies (list rows, table rows,
    /// tabs, menu items).
    ///
    /// `None` means "the concept of 'selected' does not apply here" — and that
    /// is **not** the same thing as `Some(false)`: a node advertising
    /// `Some(false)` makes a screen reader announce "not selected" for every
    /// row the user passes. That is why only containers that genuinely have a
    /// selection fill it in (`AccessKit` documents the same trap on
    /// `Selected`).
    pub selected: Option<bool>,
    /// Caret and selection inside [`AccessNode::value`], for editable text.
    ///
    /// Only text-editing roles fill this in; everywhere else it stays `None`,
    /// which means "this node has no caret", not "the caret is at zero".
    pub text_selection: Option<AccessTextSelection>,
    /// Open or closed, where the concept applies (tree rows, disclosure
    /// triangles, combo boxes).
    ///
    /// The same trap as [`AccessNode::selected`]: `None` means "this node
    /// cannot be opened at all", while `Some(false)` makes a screen reader
    /// announce "collapsed" — which is exactly right for a branch and exactly
    /// wrong for a leaf.
    pub expanded: Option<bool>,
    /// Nesting depth, counted from 1 for a root row.
    ///
    /// A hierarchy is invisible to assistive technology unless every row says
    /// how deep it sits: without this a screen reader reads an outline as a
    /// flat list, and the user loses the only structure the widget has.
    pub level: Option<usize>,
    /// This node's 1-based position among its **siblings**.
    ///
    /// Deliberately not its position in the flattened list: "3 of 7" is what a
    /// screen reader announces, and it is about the group the row belongs to.
    pub position_in_set: Option<usize>,
    /// How many siblings the group holds in total.
    ///
    /// A virtualized container may only materialize a window of its rows, so
    /// this number cannot be inferred from the a11y tree — the widget is the
    /// only one that knows it.
    pub size_of_set: Option<usize>,
}

impl AccessNode {
    /// An empty node with the structural role.
    pub fn new() -> Self {
        Self::default()
    }

    /// A node with a particular role.
    pub fn with_role(role: AccessRole) -> Self {
        Self {
            role,
            ..Self::default()
        }
    }

    /// Set the role.
    pub fn role(mut self, role: AccessRole) -> Self {
        self.role = role;
        self
    }

    /// Set the announced name.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the current value.
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Add capabilities.
    pub fn with_actions(mut self, actions: AccessActions) -> Self {
        self.actions |= actions;
        self
    }

    /// Mark as unusable.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Hide from assistive technology (along with its descendants).
    pub fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    /// Set the on/off/mixed state.
    pub fn toggled(mut self, toggled: AccessToggled) -> Self {
        self.toggled = Some(toggled);
        self
    }

    /// Set the selected state (list/table rows, tabs, menu items).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    /// Set the caret/selection inside the value (editable text only).
    pub fn text_selection(mut self, selection: AccessTextSelection) -> Self {
        self.text_selection = Some(selection);
        self
    }

    /// Set the open/closed state (tree rows, disclosure triangles).
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    /// Place this node inside a hierarchy: depth (from 1), its 1-based
    /// position among its siblings, and how many siblings there are.
    pub fn at_level(mut self, level: usize, position: usize, size: usize) -> Self {
        self.level = Some(level);
        self.position_in_set = Some(position);
        self.size_of_set = Some(size);
        self
    }

    /// True when the node can take keyboard focus.
    pub fn is_focusable(&self) -> bool {
        self.actions.contains(AccessActions::FOCUS) && !self.disabled
    }
}
