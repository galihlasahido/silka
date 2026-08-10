//! # `tabs` — a tab row (`KOMPONEN.md` Tier 3)
//!
//! Three variants as the catalog requires, one engine: **segmented** (the
//! `NSSegmentedControl` feel), **underline** (the shadcn/ui feel), and
//! **enclosed** (folder tabs that merge into their panel). All that separates
//! them is the tokens resolved by [`TabsStyle::from_theme`] and the shape of
//! the indicator rect ([`TabsStyle::indicator_rect`]) — not one of them has a
//! layout, input, or a11y path of its own.
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_theme::{Appearance, Theme};
//! # use silka_widgets::Fonts;
//! use silka_widgets::tabs::{tab, tabs, TabsVariant};
//!
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! # let rt = Runtime::new();
//! let terpilih = rt.signal(0usize);
//!
//! let _ = tabs(
//!     &fonts,
//!     &t,
//!     [tab("Umum"), tab("Tampilan"), tab("Lanjutan").disabled(true)],
//! )
//! .variant(TabsVariant::Segmented)
//! .selected(terpilih.get())
//! .label("Pengaturan")
//! .on_select(move |i| terpilih.set(i));
//! ```
//!
//! ## Controlled component
//!
//! `tabs` does **not** select on its own: `selected` comes from the app and
//! `on_select` reports the user's intent — the same pattern as `open` on
//! [`overlay`](mod@crate::overlay). That keeps the panel below it purely
//! declarative: build only the active panel's view, and the inactive ones are
//! not in the tree at all (not reachable by Tab, not announced by a screen
//! reader, not laid out).
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Item | Where |
//! |---|---|
//! | Correct in both presets | [`TabsStyle::from_theme`] — not a single color value lives outside that file |
//! | Interactive state on springs | [`TabBox`] (hover/press) + [`TabListBox`] (indicator) |
//! | Full keyboard + focus ring | One row = one Tab stop; arrows/Home/End inside it, focus ring around the active tab |
//! | AccessKit node | [`AccessRole::TabList`] + [`AccessRole::Tab`] carrying selected state |
//! | Dark mode | Follows the tokens, without a single `if` branch |
//! | Hit target ≥ 44pt | [`TabsStyle::min_height`] forced onto every tab during layout |
//! | Reduced-motion | Indicator [`Essential`](silka_core::animation::MotionRole::Essential) (loses its bounce), hover highlight [`Decorative`](silka_core::animation::MotionRole::Decorative) (disappears entirely) |
//!
//! ## Who ticks the springs
//!
//! Just like [`crate::overlay::advance`]: the shell calls [`advance`] once per
//! frame, and that function answers whether anything is still moving
//! (§3.5, "render only when dirty"). Until the
//! [`AnimationDriver`](silka_core::animation::AnimationDriver) is wired into
//! the app's frame loop, a shell may never call it at all — and the nodes here
//! **do not freeze** when that happens: before the first tick arrives,
//! transitions run as jumps. Once the ticks start coming, those same
//! transitions become springs without a single line changing in the app.

pub mod item;
pub mod list;
pub mod style;
#[cfg(test)]
mod tests;

use silka_core::animation::{Spring, Tick};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{AccessRole, CrossAlign, MainAlign, NodeId, RenderTree};
use silka_core::view::{row, Builder, View};
use silka_core::Callback;
use silka_text::FontWeight;
use silka_theme::Theme;

use crate::fonts::Fonts;
use crate::text::text;

pub use item::{TabBox, TabProps, TAB_TINT_MOTION};
pub use list::{OnSelect, TabListBox, TabListProps};
pub use style::{TabsStyle, TabsVariant};

// ---------------------------------------------------------------------------
// A single tab
// ---------------------------------------------------------------------------

/// Description of one tab: its label, state, and identity key.
///
/// Deliberately **not** a [`View`]: the row needs to read `disabled` before the
/// tree is assembled (arrow navigation skips disabled tabs), and the moment
/// something becomes a `View` its props are buried behind `dyn ViewNode`. The
///
/// ```
/// use silka_core::signals::Key;
/// use silka_widgets::tab;
///
/// let general = tab("General");
/// assert_eq!(general.label_text(), "General");
/// assert!(!general.is_disabled());
///
/// // A disabled tab is still announced — dimmed, and skipped by the arrow
/// // keys rather than silently absent.
/// let advanced = tab("Advanced").disabled(true).key(Key::from("advanced"));
/// assert!(advanced.is_disabled());
/// assert_eq!(advanced.label_text(), "Advanced");
/// ```
/// same reason gives [`crate::overlay::OverlayBuilder`] its own type.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    label: String,
    disabled: bool,
    key: Option<Key>,
}

/// A single tab labeled `label`.
///
/// ```
/// use silka_widgets::tab;
///
/// // Tabs are plain values, so a tab row can be built from data.
/// let items: Vec<_> = ["General", "Network", "Advanced"]
///     .into_iter()
///     .map(tab)
///     .collect();
/// assert_eq!(items.len(), 3);
/// assert_eq!(items[1].label_text(), "Network");
/// ```
pub fn tab(label: impl Into<String>) -> Tab {
    Tab {
        label: label.into(),
        disabled: false,
        key: None,
    }
}

impl Tab {
    /// A tab that cannot be selected (still announced, as dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Identity key — required for tab lists whose contents change (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The label a screen reader announces.
    pub fn label_text(&self) -> &str {
        &self.label
    }

    /// True when this tab cannot be selected.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Dart-style builder for a tab row (§2.5).
///
/// Its own type rather than a [`Builder`], because it has to **assemble its
/// children** from the [`Tab`] list at the moment it becomes a [`View`]: label
/// colors, font weights, and the per-index callbacks are all derived from
/// `selected` and `style`, which are only known once the whole method chain has
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{tab, tabs, Fonts, TabsVariant};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
/// let rt = Runtime::new();
/// let page = rt.signal(0usize);
///
/// let row = tabs(&fonts, &theme, [tab("General"), tab("Network")])
///     .segmented()
///     .selected(page.get())
///     .label("Settings sections")
///     .on_select(move |i| page.set(i));
///
/// assert_eq!(row.active_index(), 0);
/// assert_eq!(row.resolved_style().variant, TabsVariant::Segmented);
///
/// // A selected index past the end is clamped rather than panicking: a tab
/// // list whose contents shrank must not take the application down.
/// let clamped = tabs(&fonts, &theme, [tab("Only")]).selected(99);
/// assert_eq!(clamped.active_index(), 0);
///
/// // Switching variant swaps the tokens, not the engine.
/// let underlined = tabs(&fonts, &theme, [tab("A"), tab("B")]).underline();
/// assert_eq!(underlined.resolved_style().variant, TabsVariant::Underline);
/// ```
/// been written out.
pub struct Tabs {
    fonts: Fonts,
    theme: Theme,
    items: Vec<Tab>,
    variant: TabsVariant,
    style: Option<TabsStyle>,
    equal_widths: Option<bool>,
    selected: usize,
    label: Option<String>,
    on_select: Option<OnSelect>,
    spring: Spring,
    key: Option<Key>,
}

/// A tab row holding `items`.
///
/// `fonts` is the app's text engine and `theme` the source of every value —
/// not a single number originates in application code (§2.6).
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{tab, tabs, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
/// let rt = Runtime::new();
/// let selected = rt.signal(1usize);
///
/// // The whole row is one Tab stop; the arrow keys move *within* it, which
/// // is what the platform conventions expect of a tab list.
/// let row = tabs(
///     &fonts,
///     &theme,
///     [tab("General"), tab("Network"), tab("Advanced").disabled(true)],
/// )
/// .selected(selected.get())
/// .on_select(move |i| selected.set(i));
///
/// assert_eq!(row.active_index(), 1);
/// ```
pub fn tabs(fonts: &Fonts, theme: &Theme, items: impl IntoIterator<Item = Tab>) -> Tabs {
    Tabs {
        fonts: fonts.clone(),
        theme: *theme,
        items: items.into_iter().collect(),
        variant: TabsVariant::default(),
        style: None,
        equal_widths: None,
        selected: 0,
        label: None,
        on_select: None,
        // `snappy` is the preset closest to how picking a segment feels on
        // macOS: arrives fast, with barely any bounce (WWDC23).
        spring: Spring::snappy(),
        key: None,
    }
}

impl Tabs {
    /// Visual variant (defaults to [`TabsVariant::Segmented`]).
    pub fn variant(mut self, variant: TabsVariant) -> Self {
        self.variant = variant;
        self
    }

    /// The [`TabsVariant::Segmented`] variant.
    pub fn segmented(self) -> Self {
        self.variant(TabsVariant::Segmented)
    }

    /// The [`TabsVariant::Underline`] variant.
    pub fn underline(self) -> Self {
        self.variant(TabsVariant::Underline)
    }

    /// The [`TabsVariant::Enclosed`] variant.
    pub fn enclosed(self) -> Self {
        self.variant(TabsVariant::Enclosed)
    }

    /// Index of the currently active tab (controlled component).
    pub fn selected(mut self, index: usize) -> Self {
        self.selected = index;
        self
    }

    /// What runs when the user picks a different tab.
    pub fn on_select(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_select = Some(OnSelect::new(f));
        self
    }

    /// The row's name for screen readers.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Force every tab to the widest tab's width (default: segmented only).
    pub fn equal_widths(mut self, equal: bool) -> Self {
        self.equal_widths = Some(equal);
        self
    }

    /// The spring driving the indicator and the highlights (`smooth`/`snappy`/
    /// `bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Replace every visual value at once — an escape hatch for custom brands
    /// that swapping theme tokens alone cannot express (§2.7).
    pub fn style(mut self, style: TabsStyle) -> Self {
        self.variant = style.variant;
        self.style = Some(style);
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The visual values that will be used — the already-resolved tokens.
    pub fn resolved_style(&self) -> TabsStyle {
        let mut style = self
            .style
            .unwrap_or_else(|| TabsStyle::from_theme(&self.theme, self.variant));
        if let Some(equal) = self.equal_widths {
            style.equal_widths = equal;
        }
        style
    }

    /// The active index that actually applies: clamped to the current list.
    ///
    /// An out-of-range index does **not** panic and does not make the indicator
    /// vanish — a tab list that shrinks one frame ahead of the signal holding
    /// the selection is normal, not an application bug.
    pub fn active_index(&self) -> usize {
        if self.items.is_empty() {
            return 0;
        }
        self.selected.min(self.items.len() - 1)
    }
}

impl From<Tabs> for View {
    fn from(t: Tabs) -> View {
        let style = t.resolved_style();
        let aktif = t.active_index();
        let props = TabListProps {
            style,
            selected: aktif,
            label: t.label.clone(),
            on_select: t.on_select.clone(),
            enabled: t.items.iter().map(|i| !i.disabled).collect(),
            spring: t.spring,
        };

        let mut builder = Builder::new(props);
        for (i, item) in t.items.iter().enumerate() {
            builder = builder.child(tab_view(&t, &style, i, item, i == aktif));
        }
        if let Some(key) = t.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

/// Assemble one tab into a view: highlight + label, both driven by tokens.
fn tab_view(t: &Tabs, style: &TabsStyle, index: usize, item: &Tab, selected: bool) -> View {
    let warna = if item.disabled {
        style.disabled_label
    } else if selected {
        style.selected_label
    } else {
        style.label
    };

    // The label sits in a flex container that centers it — no arithmetic here
    // (§3.4). Its role is `Container` so a screen reader does not announce the
    // tab's name twice: once from the tab node, once from its text.
    let isi = row([text(&t.fonts, &item.label)
        .size(style.label_size)
        .weight(if selected {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::MEDIUM
        })
        .color(warna)
        .single_line()
        .role(AccessRole::Container)])
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(style.tab_padding);

    let on_press = t.on_select.clone().map(|cb| {
        Callback::new(move || {
            cb.call(index);
        })
    });

    let mut b = Builder::new(TabProps {
        label: item.label.clone(),
        index,
        selected,
        disabled: item.disabled,
        corners: style.tab_corners,
        hover: style.hover,
        pressed: style.pressed,
        on_press,
        spring: t.spring,
    })
    .child(isi);
    if let Some(key) = item.key.clone() {
        b = b.key(key);
    }
    b.into()
}

// ---------------------------------------------------------------------------
// Ticking
// ---------------------------------------------------------------------------

/// Every `tabs` node in `tree`, in pre-order.
fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        if let Some(node) = tree.render(id) {
            if node.downcast_ref::<TabListBox>().is_some()
                || node.downcast_ref::<TabBox>().is_some()
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

/// Advance every `tabs` transition by one frame.
///
/// The shell calls this **once per frame**, unconditionally: this function is
/// what knows whether anything is still moving, and its answer decides whether
/// the next frame needs to be scheduled (§3.5). What it returns:
///
/// - [`Dirty::PAINT`] — an indicator or highlight **changed** this frame.
/// - [`Dirty::ANIMATION`] — a spring has yet to settle. Once this flag is gone,
///   the GPU may sleep.
/// - [`Dirty::NONE`] — this module produced no work.
///
/// The indicator moves **without** triggering layout: tab positions do not
/// depend on it, so an animating row never forces the window to be recomputed.
///
/// ```
/// # use silka_core::animation::{Motion, Tick};
/// # use silka_core::scheduler::Dirty;
/// # use silka_core::tree::{BoxConstraints, RenderTree};
/// # use silka_core::view::reconcile;
/// # use silka_paint::Size;
/// # use silka_theme::{Appearance, Theme};
/// # use silka_widgets::Fonts;
/// # use std::time::Duration;
/// use silka_widgets::tabs::{advance, tab, tabs};
///
/// # let fonts = Fonts::bundled_only();
/// # let t = Theme::tailwind(Appearance::Light);
/// let mut tree = RenderTree::new();
/// let tick = Tick::manual(Duration::from_millis(8), Motion::Full);
///
/// reconcile(&mut tree, tabs(&fonts, &t, [tab("Satu"), tab("Dua")]).selected(0));
/// tree.layout(BoxConstraints::tight(Size::new(400.0, 60.0)));
/// // A freshly built row is already in place: nothing is moving.
/// assert_eq!(advance(&mut tree, &tick), Dirty::NONE);
/// ```
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in nodes(tree) {
        let (berubah, bergerak) = if let Some(l) = tree.node_mut_ref::<TabListBox>(id) {
            (l.advance(tick), l.is_animating())
        } else if let Some(t) = tree.node_mut_ref::<TabBox>(id) {
            (t.advance(tick), t.is_animating())
        } else {
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

/// True while any `tabs` transition is still running.
///
/// ```
/// use silka_core::tree::RenderTree;
/// use silka_widgets::tabs::{is_animating, settle};
///
/// // A tree with no tab rows is trivially at rest, so an application may
/// // call this unconditionally.
/// let mut tree = RenderTree::new();
/// assert!(!is_animating(&tree));
/// settle(&mut tree);
/// assert!(!is_animating(&tree));
/// ```
/// True while any `tabs` transition is still running.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<TabListBox>(id)
            .is_some_and(TabListBox::is_animating)
            || tree
                .node_ref::<TabBox>(id)
                .is_some_and(TabBox::is_animating)
    })
}

/// Finish every `tabs` transition instantly (used by tests and snapshots).
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::tabs::{is_animating, settle};
/// use silka_widgets::{tab, tabs, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let mut tree = RenderTree::new();
/// reconcile(
///     &mut tree,
///     tabs(&fonts, &theme, [tab("General"), tab("Network")]).selected(1),
/// );
/// tree.layout(BoxConstraints::loose(Size::new(320.0, 44.0)));
///
/// // Jump the sliding indicator to its destination — a golden file should
/// // photograph the result, never a spring mid-flight.
/// settle(&mut tree);
/// assert!(!is_animating(&tree));
/// ```
/// Finish every `tabs` transition instantly (used by tests and snapshots).
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(l) = tree.node_mut_ref::<TabListBox>(id) {
            l.settle();
        } else if let Some(t) = tree.node_mut_ref::<TabBox>(id) {
            t.settle();
        }
        tree.mark_needs_paint(id);
    }
}
