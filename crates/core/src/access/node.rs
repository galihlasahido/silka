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

/// The part of an accessibility node that is **filled in by the widget**.
///
/// This is half of the [`crate::tree::RenderNode::access`] contract. The other
/// half — `bounds`, parent and children — comes from the layout result and is
/// assembled by the engine in [`super::AccessEntry`], so it cannot go stale
/// with respect to what is drawn.
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

    /// True when the node can take keyboard focus.
    pub fn is_focusable(&self) -> bool {
        self.actions.contains(AccessActions::FOCUS) && !self.disabled
    }
}
