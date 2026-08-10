//! `text_area()` — the **multi-line** text editor (`KOMPONEN.md` Tier 2,
//! "multiline + soft wrap; the foundation for an editor").
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_theme::{Appearance, Theme};
//! # use silka_widgets::{text_area, Fonts};
//! # let rt = Runtime::new();
//! # let catatan = rt.signal(String::new());
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! text_area(&fonts, &t, catatan.get())
//!     .placeholder("Tulis catatan…")
//!     .label("Catatan")
//!     .auto_grow(3, 12)
//!     .line_numbers(true)
//!     .on_change(move |s| catatan.set(s.to_string()));
//! ```
//!
//! ## It does not own a second editing engine — that is the whole point
//!
//! `text_field` is described in `KOMPONEN.md` as the hardest component in the
//! catalogue. Writing its caret, its selection, its undo, and its IME a second
//! time for the multi-line case is how the two quietly start disagreeing about
//! what ⌥Backspace does. So the layers are shared, and only the genuinely
//! multi-line parts are new:
//!
//! | Layer | Where | Shared with |
//! |---|---|---|
//! | Document, graphemes, words, undo, preedit | [`silka_text::TextEdit`] (in `multiline` mode) | `text_field` |
//! | Keyboard → document | [`crate::editing::handle_key`] | `text_field` |
//! | Caret/selection geometry, soft wrapping | [`silka_text::TextLayout`] | `text_field`, `text` |
//! | Momentum, rubber band, scrollbar, `SCROLL` action | [`mod@crate::scroll_view`] | `list`, `table` |
//! | Vertical navigation, gutter, auto-grow, frame | this module | — |
//!
//! ## What multi-line actually adds
//!
//! | Behaviour | Detail |
//! |---|---|
//! | Soft wrap | Breaks against the width the constraints hand down; resizing the window re-wraps, it does not scroll sideways |
//! | ↑/↓ | Across **visual** lines with a real **goal column**: walking down through a short line and out again puts the caret back under the eye |
//! | Home/End | The ends of the *visual* line — which is what they mean once a paragraph wraps |
//! | ⌘/Ctrl+Home/End, ⌘↑/⌘↓ | The ends of the document |
//! | PageUp/PageDown | One viewport, measured from the frame — not a guessed number of lines |
//! | Enter | A new line. ⌘Enter runs `on_submit`, the habit of every comment box |
//! | Tab | **Configurable, and moves focus by default** ([`TabBehavior`]): a Tab swallowed by a text box is a keyboard trap |
//! | Selection across lines | One highlight rectangle per visual run, so bidi text never highlights letters that are not selected |
//! | Auto-grow | The frame's height follows the content between `min_rows` and `max_rows`, then scrolls |
//! | Line numbers | Optional gutter; one number per **source** line, blank on wrapped continuations |
//! | Placeholder | Shown while the document is empty, in the `tertiary_label` token |
//!
//! ## Definition of Done (`KOMPONEN.md`), satisfied
//!
//! - **Both presets** through semantic tokens; not one colour literal in this
//!   module, and the corner shape is a parameter, not a constant (§2.7, §3.6).
//! - **Every interactive state transitions on a spring**: hover and focus are
//!   retargetable [`SpringValue`](silka_core::animation::SpringValue)s on the
//!   frame, scrolling is the scroll view's spring.
//! - **Full keyboard**, the table above — plus everything `text_field` has,
//!   because it is literally the same keymap.
//! - **AccessKit node** with the role
//!   [`MultilineTextInput`](silka_core::access::AccessRole::MultilineTextInput),
//!   a name, a value, `SET_VALUE`, and **caret/selection reporting**
//!   ([`AccessTextSelection`](silka_core::access::AccessTextSelection)) — the
//!   scroll container around it advertises `SCROLL` on its own.
//! - **Dark mode** follows the tokens; **reduced motion** is honoured because
//!   every motion goes through [`Tick`].
//!
//! ## Technical debt we know about
//!
//! - **Clipboard** (⌘C/⌘X/⌘V) is not wired up: `arboard` lives in
//!   `silka-platform` (INTEGRASI-NATIVE §4) and this crate must not depend on
//!   it. Those shortcuts are deliberately left to **bubble**.
//! - **The caret does not blink** — same reason as in `text_field`: blinking
//!   needs a timer that ticks forever, and that collides with "render only
//!   when dirty" (§3.5).
//! - **The whole document is shaped and rasterized**, not only the visible
//!   part. Correct at note length, wrong at novel length; virtualised text is
//!   what `code_editor` is for (`KOMPONEN.md` Tier 6).
//! - **No horizontal scrolling**: wrapping is always on. A no-wrap mode would
//!   need a second scroll axis, and every use we have wants wrapping.

mod body;
mod frame;
mod link;
#[cfg(test)]
mod tests;

use silka_core::access::{AccessAction, AccessActionRequest};
use silka_core::animation::{Spring, Tick};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{FocusRing, NodeId, RenderNode, RenderTree};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, Corners, Insets};
use silka_text::TextStyle;
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::editing::TextCallback;
use crate::fonts::Fonts;
use crate::scroll_view::{self, scroll_view, ScrollView, Scrollbar};

pub use body::{BodyColors, TextAreaBody};
pub use frame::{FrameStyle, TextAreaFrame};
pub use link::AreaLink;

use body::BodyConfig;

/// How much room is left around the caret when it is scrolled into view.
const REVEAL_PADDING: f32 = 6.0;

// ---------------------------------------------------------------------------
// Tab
// ---------------------------------------------------------------------------

/// What the Tab key does inside a text area.
///
/// The default is **not** the one that inserts a tab, and that is an
/// accessibility decision rather than a taste one: a text box that swallows Tab
/// is a keyboard trap — a keyboard-only user who lands in it can never leave
/// (`KOMPONEN.md` Definition of Done, §3.8). Indentation is therefore opt-in,
/// and even then ⇧Tab still walks focus backwards so an escape hatch always
/// exists.
/// ```
/// use silka_widgets::text_area::TabBehavior;
///
/// // The default moves focus, and that is an accessibility decision rather
/// // than a taste one: a text box that swallows Tab is a keyboard trap.
/// assert_eq!(TabBehavior::default(), TabBehavior::MoveFocus);
///
/// // Indentation is opt-in, for the cases that genuinely need it.
/// assert_ne!(TabBehavior::InsertTab, TabBehavior::default());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TabBehavior {
    /// Tab leaves the field and moves focus onward (the default).
    #[default]
    MoveFocus,
    /// Tab inserts a tab character; ⇧Tab still moves focus backwards.
    InsertTab,
}

// ---------------------------------------------------------------------------
// Props: the body
// ---------------------------------------------------------------------------

/// The editing body's props — **resolved tokens only**.
#[derive(Debug, Clone, PartialEq)]
pub struct TextAreaBodyProps {
    fonts: Fonts,
    value: String,
    placeholder: String,
    style: TextStyle,
    padding: Insets,
    caret_width: f32,
    line_numbers: bool,
    gutter_gap: f32,
    label: Option<String>,
    disabled: bool,
    read_only: bool,
    tab: TabBehavior,
    colors: BodyColors,
    on_change: Option<TextCallback>,
    on_submit: Option<TextCallback>,
    link: AreaLink,
}

impl ViewNode for TextAreaBodyProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TextAreaBody::new(BodyConfig {
            fonts: self.fonts.clone(),
            style: self.style.clone(),
            placeholder: self.placeholder.clone(),
            padding: self.padding,
            caret_width: self.caret_width,
            line_numbers: self.line_numbers,
            gutter_gap: self.gutter_gap,
            label: self.label.clone(),
            disabled: self.disabled,
            read_only: self.read_only,
            tab: self.tab,
            colors: self.colors,
            on_change: self.on_change.clone(),
            on_submit: self.on_submit.clone(),
            link: self.link.clone(),
            value: self.value.clone(),
        }))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TextAreaBody>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        // Every rebuild allocates a fresh link; the state on the old one is
        // carried over so a rebuild in the middle of typing does not blink the
        // focus ring off (see `AreaLink::adopt`).
        if !n.link.same(&self.link) {
            self.link.adopt(&n.link);
            n.link = self.link.clone();
        }

        // **The contents are overwritten only when the app actually changed
        // them.** Comparing props against props (not against the node's
        // contents) is the difference between an area you can type in and one
        // that throws the caret backwards on every unrelated signal — the
        // classic controlled-component bug.
        if n.props_value != self.value {
            n.props_value.clone_from(&self.value);
            if n.edit.text() != self.value {
                n.edit.set_text(self.value.clone());
                n.invalidate_shape();
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
        }

        if n.style != self.style || n.placeholder != self.placeholder {
            n.style = self.style.clone();
            n.placeholder.clone_from(&self.placeholder);
            n.invalidate_shape();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.padding != self.padding
            || n.caret_width != self.caret_width
            || n.line_numbers != self.line_numbers
            || n.gutter_gap != self.gutter_gap
        {
            n.padding = self.padding;
            n.caret_width = self.caret_width;
            n.line_numbers = self.line_numbers;
            n.gutter_gap = self.gutter_gap;
            n.invalidate_shape();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.colors != self.colors {
            // The text colour is baked into the rasterized run, so the glyphs
            // have to be produced again — not merely re-drawn.
            n.set_colors(self.colors);
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.disabled != self.disabled || n.read_only != self.read_only || n.tab != self.tab {
            n.disabled = self.disabled;
            n.read_only = self.read_only;
            n.tab = self.tab;
            dirty |= Dirty::PAINT;
        }
        if n.fonts != self.fonts {
            n.fonts = self.fonts.clone();
            n.invalidate_shape();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        // Callbacks are always replaced without comparison: closures are
        // rebuilt on every rebuild and capture fresh values.
        n.on_change.clone_from(&self.on_change);
        n.on_submit.clone_from(&self.on_submit);
        dirty
    }
}

// ---------------------------------------------------------------------------
// Props: the frame
// ---------------------------------------------------------------------------

/// The frame's props — background, border, ring, and the sizing rule.
#[derive(Debug, Clone, PartialEq)]
pub struct TextAreaFrameProps {
    style: FrameStyle,
    min_height: f32,
    max_height: f32,
    auto_grow: bool,
    spring: Spring,
    link: AreaLink,
}

impl TextAreaFrameProps {
    /// Build a frame around **any** editing body that rides this widget's
    /// three-node stack (frame → scroll view → body).
    ///
    /// The one caller outside this module is [`mod@crate::wysiwyg`], and that is
    /// the point: the rich text editor's frame, its focus ring, its auto-grow
    /// rule, and its seam to the scroll view are not written a second time
    /// (`KOMPONEN.md` ordering rule: one engine, not three).
    pub fn new(
        style: FrameStyle,
        min_height: f32,
        max_height: f32,
        auto_grow: bool,
        spring: Spring,
        link: AreaLink,
    ) -> Self {
        Self {
            style,
            min_height,
            max_height,
            auto_grow,
            spring,
            link,
        }
    }
}

impl ViewNode for TextAreaFrameProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TextAreaFrame::new(
            self.style,
            self.min_height,
            self.max_height,
            self.auto_grow,
            self.link.clone(),
            self.spring,
        ))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TextAreaFrame>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if !n.link.same(&self.link) {
            self.link.adopt(&n.link);
            n.link = self.link.clone();
        }
        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::PAINT;
        }
        if n.min_height != self.min_height
            || n.max_height != self.max_height
            || n.auto_grow != self.auto_grow
        {
            n.min_height = self.min_height;
            n.max_height = self.max_height;
            n.auto_grow = self.auto_grow;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.spring() != self.spring {
            n.set_spring(self.spring);
        }
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// A Dart-style `text_area` builder (§2.5).
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{text_area, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // `rows` fixes the height; `auto_grow` lets it breathe between two bounds.
/// // They answer the same question, so the later call wins.
/// let fixed = text_area(&fonts, &theme, "").rows(4);
/// let growing = text_area(&fonts, &theme, "").auto_grow(1, 6);
/// # let _ = (fixed, growing);
///
/// // Read-only keeps the caret, the selection and copying; only editing goes.
/// let readonly = text_area(&fonts, &theme, "log output").read_only(true);
/// # let _ = readonly;
/// ```
/// A Dart-style `text_area` builder (§2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct TextArea {
    theme: Theme,
    body: TextAreaBodyProps,
    frame: TextAreaFrameProps,
    /// Row heights are what the caller talks in; points are derived.
    min_rows: usize,
    max_rows: usize,
    scrollbar: Scrollbar,
    key: Option<Key>,
}

/// A multi-line text area — the `text_area` component (`KOMPONEN.md` Tier 2).
///
/// Every value comes from `theme`; `fonts` is the app's text engine. The
/// default is a fixed four-row box with soft wrapping, no gutter, and a Tab
/// that moves focus.
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::text_area::TabBehavior;
/// use silka_widgets::{text_area, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
/// let rt = Runtime::new();
/// let body = rt.signal(String::new());
///
/// // A comment box that grows with its contents up to a ceiling, then
/// // scrolls — the shape most multi-line inputs actually want.
/// let comment = text_area(&fonts, &theme, body.get())
///     .placeholder("Write a reply")
///     .label("Reply")
///     .auto_grow(2, 8)
///     .on_change(move |text| body.set(text.to_string()));
/// # let _ = comment;
///
/// // A code box: fixed height, a line-number gutter, and Tab that indents.
/// // Even then ⇧Tab still walks focus backwards, so it is never a trap.
/// let code = text_area(&fonts, &theme, "fn main() {}")
///     .rows(12)
///     .line_numbers(true)
///     .tab(TabBehavior::InsertTab);
/// # let _ = code;
/// ```
pub fn text_area(fonts: &Fonts, theme: &Theme, value: impl Into<String>) -> TextArea {
    let t = theme;
    let link = AreaLink::new();
    let style = TextStyle::new()
        .size(t.typography.body_size)
        .line_height(t.typography.body_line_height);
    let padding = Insets::symmetric(t.space(3.0), t.space(2.0));
    let min_rows = 4;

    let mut area = TextArea {
        theme: *t,
        body: TextAreaBodyProps {
            fonts: fonts.clone(),
            value: value.into(),
            placeholder: String::new(),
            style,
            padding,
            // As thin as the smallest spacing step: the HIG caret is a
            // hairline, not a slab.
            caret_width: t.space(0.25),
            line_numbers: false,
            gutter_gap: t.space(2.0),
            label: None,
            disabled: false,
            read_only: false,
            tab: TabBehavior::default(),
            colors: BodyColors {
                text: t.color.label,
                placeholder: t.color.tertiary_label,
                disabled: t.color.disabled_label,
                selection: t.color.selection,
                caret: t.color.accent,
                gutter: t.color.tertiary_label,
                gutter_background: t.color.surface_sunken,
                gutter_separator: t.color.separator,
            },
            on_change: None,
            on_submit: None,
            link: link.clone(),
        },
        frame: TextAreaFrameProps {
            style: FrameStyle {
                background: t.color.surface,
                background_hover: t.color.surface_hover,
                background_focus: t.color.surface,
                border_width: t.space(0.25),
                border: t.color.border,
                border_focus: t.color.accent,
                corners: t.corners(t.radius.md),
                focus_ring: Some(FocusRing::new(t.space(0.5), t.color.focus_ring)),
            },
            min_height: 0.0,
            max_height: 0.0,
            auto_grow: false,
            spring: Spring::snappy(),
            link,
        },
        min_rows,
        max_rows: min_rows,
        scrollbar: Scrollbar::default(),
        key: None,
    };
    area.hitung_tinggi();
    area
}

impl TextArea {
    fn map(mut self, f: impl FnOnce(&mut TextArea)) -> Self {
        f(&mut self);
        self.hitung_tinggi();
        self
    }

    /// Translate the row counts into point heights.
    ///
    /// Rows are the unit the caller thinks in ("a four-line box"); the frame
    /// works in points, and the conversion depends on the typography token, so
    /// it belongs here and nowhere else.
    fn hitung_tinggi(&mut self) {
        let baris = self.body.style.line_height_px();
        let sisip = self.body.padding.vertical();
        // Never below the HIG hit target, even for `rows(1)`: a control you
        // cannot reliably hit is not a control (`KOMPONEN.md` DoD).
        self.frame.min_height = (baris * self.min_rows.max(1) as f32 + sisip).max(MIN_HIT_TARGET);
        self.frame.max_height =
            (baris * self.max_rows.max(self.min_rows) as f32 + sisip).max(self.frame.min_height);
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The faint text shown while the area is empty.
    pub fn placeholder(self, placeholder: impl Into<String>) -> Self {
        let p = placeholder.into();
        self.map(move |x| x.body.placeholder = p)
    }

    /// The name a screen reader announces (§3.8) — the visual label's twin.
    pub fn label(self, label: impl Into<String>) -> Self {
        let l = label.into();
        self.map(move |x| x.body.label = Some(l))
    }

    /// Disable the area: it takes neither focus nor keystrokes, but is still
    /// read out.
    pub fn disabled(self, disabled: bool) -> Self {
        self.map(move |x| x.body.disabled = disabled)
    }

    /// The contents can be selected and copied, but not changed.
    pub fn read_only(self, read_only: bool) -> Self {
        self.map(move |x| x.body.read_only = read_only)
    }

    /// A **fixed** height of `rows` lines; the content scrolls inside it.
    pub fn rows(self, rows: usize) -> Self {
        self.map(move |x| {
            x.min_rows = rows.max(1);
            x.max_rows = rows.max(1);
            x.frame.auto_grow = false;
        })
    }

    /// **Grow with the content** between `min_rows` and `max_rows` lines, then
    /// scroll.
    ///
    /// The new height is applied by [`sync`] at the start of the next frame —
    /// which is the same frame the keystroke is painted in, so the box is never
    /// seen at the wrong size.
    pub fn auto_grow(self, min_rows: usize, max_rows: usize) -> Self {
        self.map(move |x| {
            x.min_rows = min_rows.max(1);
            x.max_rows = max_rows.max(min_rows.max(1));
            x.frame.auto_grow = true;
        })
    }

    /// Show the line-number gutter.
    pub fn line_numbers(self, on: bool) -> Self {
        self.map(move |x| x.body.line_numbers = on)
    }

    /// What the Tab key does — the default moves focus ([`TabBehavior`]).
    pub fn tab(self, tab: TabBehavior) -> Self {
        self.map(move |x| x.body.tab = tab)
    }

    /// Called every time the contents change — **without** the IME preedit.
    pub fn on_change(self, f: impl Fn(&str) + 'static) -> Self {
        let cb = TextCallback::new(f);
        self.map(move |x| x.body.on_change = Some(cb))
    }

    /// Called on ⌘/Ctrl+Enter (plain Enter inserts a line).
    pub fn on_submit(self, f: impl Fn(&str) + 'static) -> Self {
        let cb = TextCallback::new(f);
        self.map(move |x| x.body.on_submit = Some(cb))
    }

    /// A complete text style (e.g. one already assembled from typography
    /// tokens, or a monospace family for code).
    pub fn style(self, style: TextStyle) -> Self {
        self.map(move |x| x.body.style = style)
    }

    /// Spacing inside the area's edges — **always** the token spacing scale
    /// (§2.6).
    pub fn padding(self, padding: Insets) -> Self {
        self.map(move |x| x.body.padding = padding)
    }

    /// Corner shape: squircle on Cupertino, arc on Tailwind — two equally valid
    /// values, both of them shader parameters (§3.6).
    pub fn corners(self, corners: Corners) -> Self {
        self.map(move |x| x.frame.style.corners = corners)
    }

    /// Background colour of the field (the `surface` token).
    pub fn background(self, color: Color) -> Self {
        self.map(move |x| {
            x.frame.style.background = color;
            x.frame.style.background_focus = color;
        })
    }

    /// A border `width` thick in `color` (the `border`/`separator` token).
    pub fn border(self, width: f32, color: Color) -> Self {
        self.map(move |x| {
            x.frame.style.border_width = width.max(0.0);
            x.frame.style.border = color;
        })
    }

    /// No keyboard focus ring (rare — a form field should keep it).
    pub fn no_focus_ring(self) -> Self {
        self.map(|x| x.frame.style.focus_ring = None)
    }

    /// When the scrollbar is visible.
    pub fn scrollbar(self, scrollbar: Scrollbar) -> Self {
        self.map(move |x| x.scrollbar = scrollbar)
    }

    /// The spring that drives the hover/focus transitions.
    pub fn spring(self, spring: Spring) -> Self {
        self.map(move |x| x.frame.spring = spring)
    }

    /// The height this area currently asks for, in logical points — the value
    /// the frame will clamp the content to.
    pub fn height_range(&self) -> (f32, f32) {
        (self.frame.min_height, self.frame.max_height)
    }
}

impl From<TextArea> for View {
    fn from(a: TextArea) -> View {
        let baris = a.body.style.line_height_px();
        let isi: View = Builder::new(a.body).into();
        // **Riding `scroll_view`, not reimplementing it**: momentum, rubber
        // banding, the auto-hiding overlay scrollbar, and the working `SCROLL`
        // accessibility action all come from the Tier 1 widget. It is not
        // focusable here because the text body already is — one field, one Tab
        // stop.
        let gulir = scroll_view(&a.theme, isi)
            .vertical()
            .scrollbar(a.scrollbar)
            .line_height(baris)
            .focusable(false)
            .no_focus_ring();
        let mut b = Builder::new(a.frame).child(View::from(gulir));
        if let Some(key) = a.key {
            b = b.key(key);
        }
        b.into()
    }
}

// ---------------------------------------------------------------------------
// Tree-level operations
// ---------------------------------------------------------------------------

/// Every [`TextAreaBody`] in `tree`, in tree order.
pub fn bodies(tree: &RenderTree) -> Vec<NodeId> {
    kumpulkan::<TextAreaBody>(tree)
}

/// Every [`TextAreaFrame`] in `tree`, in tree order.
pub fn frames(tree: &RenderTree) -> Vec<NodeId> {
    kumpulkan::<TextAreaFrame>(tree)
}

fn kumpulkan<N: RenderNode>(tree: &RenderTree) -> Vec<NodeId> {
    fn jelajah<N: RenderNode>(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        if tree.node_ref::<N>(id).is_some() {
            out.push(id);
        }
        for anak in tree.children(id) {
            jelajah::<N>(tree, *anak, out);
        }
    }
    let mut out = Vec::new();
    // Depth-first in **child order**: two areas on a page come back in the
    // order they are read and Tabbed through, not in the order a stack happens
    // to pop them.
    jelajah::<N>(tree, tree.root(), &mut out);
    out
}

/// The first text area body in `tree` — a shortcut for tests and the gallery.
pub fn first(tree: &RenderTree) -> Option<NodeId> {
    bodies(tree).into_iter().next()
}

/// Stitch every text area back together once a frame.
///
/// Two jobs, and both of them exist because a widget's `request_layout` only
/// raises a flag on the event response — it does not mark the tree, and the
/// scroll view in between is a relayout boundary that stops any change from
/// reaching the frame:
///
/// 1. **The text grew or shrank** — the frame's auto-grow height and the scroll
///    view's extent both follow from the content, so both are marked for
///    layout. This is the same seam [`crate::list::sync_virtual`] uses, in the
///    same place in the frame cycle.
/// 2. **The caret moved** — the scroll view is asked to
///    [`reveal`](ScrollView::reveal) it. Deliberately *after* the relayout has
///    been requested and never in the same frame as it: revealing against a
///    scroll extent that is about to change would clamp against the old
///    maximum and stop one line short.
pub fn sync(tree: &mut RenderTree) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in bodies(tree) {
        let Some(link) = tree.node_ref::<TextAreaBody>(id).map(|b| b.link().clone()) else {
            continue;
        };
        let bingkai = frame_of(tree, id);

        if link.take_relayout() {
            tree.mark_needs_layout(id);
            if let Some(f) = bingkai {
                tree.mark_needs_layout(f);
            }
            // The caret reveal waits for the next frame, by which time the
            // scroll extent is the real one.
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
            if link.wants_reveal() {
                dirty |= Dirty::ANIMATION;
            }
            continue;
        }

        if !link.wants_reveal() {
            continue;
        }
        link.clear_reveal();
        let Some(wadah) = scroll_view::enclosing(tree, id) else {
            continue;
        };
        // The **current** caret rectangle, not one recorded a keystroke ago:
        // the layout in between may have re-wrapped the text.
        let Some(caret) = tree.node_ref::<TextAreaBody>(id).map(|b| b.caret_rect()) else {
            continue;
        };
        let berubah = tree
            .node_mut_ref::<ScrollView>(wadah)
            .is_some_and(|s| s.reveal(caret.origin.y, caret.size.height, REVEAL_PADDING));
        if berubah {
            tree.mark_needs_layout(wadah);
            dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
        }
    }
    dirty
}

/// The frame that owns `body`.
fn frame_of(tree: &RenderTree, body: NodeId) -> Option<NodeId> {
    let mut cur = tree.parent(body);
    while let Some(id) = cur {
        if tree.node_ref::<TextAreaFrame>(id).is_some() {
            return Some(id);
        }
        cur = tree.parent(id);
    }
    None
}

/// Advance every text area by one frame: first the caret reveal, then the
/// frame's hover/focus springs.
///
/// The order matters and is the same one the list uses: the reveal must read
/// **this** frame's scroll position, which is why it runs after
/// [`crate::scroll_view::advance`] in [`crate::advance`].
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = sync(tree);
    for id in frames(tree) {
        let hasil = tree
            .node_mut_ref::<TextAreaFrame>(id)
            .map(|f| (f.advance(tick), f.is_animating()));
        if let Some((bergeser, bergerak)) = hasil {
            if bergeser {
                // Only pixels: the frame's height comes from the content, and
                // hovering does not change the content.
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
        }
    }
    dirty
}

/// True while any text area transition is still running.
pub fn is_animating(tree: &RenderTree) -> bool {
    frames(tree).into_iter().any(|id| {
        tree.node_ref::<TextAreaFrame>(id)
            .is_some_and(TextAreaFrame::is_animating)
    })
}

/// Finish every text area transition instantly (tests and snapshots).
pub fn settle(tree: &mut RenderTree) {
    for id in frames(tree) {
        if let Some(f) = tree.node_mut_ref::<TextAreaFrame>(id) {
            f.settle();
        }
        tree.mark_needs_paint(id);
    }
}

/// Serve an assistive-technology request aimed at a text area.
///
/// The body advertises
/// [`SET_VALUE`](silka_core::access::AccessActions::SET_VALUE) — and
/// advertising a capability you do not serve is lying to the screen reader.
/// The shell forwards whatever arrives from the platform adapter; `true` when
/// the contents really did change, in which case `on_change` has already been
/// called, exactly as for a keystroke.
pub fn apply_access_action(tree: &mut RenderTree, request: &AccessActionRequest) -> bool {
    if request.action != AccessAction::SetValue {
        return false;
    }
    let Some(nilai) = request.value.clone() else {
        return false;
    };
    let berubah = tree
        .node_mut_ref::<TextAreaBody>(request.target)
        .is_some_and(|b| b.setel_nilai_bantu(&nilai));
    if berubah {
        tree.mark_needs_layout(request.target);
    }
    berubah
}
