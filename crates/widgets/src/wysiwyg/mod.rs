//! `wysiwyg()` — the **rich text editor** (`KOMPONEN.md` Tier 6, pulled into
//! Phase 2b at the owner's request, 10 Aug 2026).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_theme::{Appearance, Theme};
//! # use silka_widgets::{wysiwyg::{wysiwyg, Document}, Fonts};
//! # let rt = Runtime::new();
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! let naskah = rt.signal(Document::from_plain("Halo dunia"));
//!
//! wysiwyg(&fonts, &t, naskah.get())
//!     .placeholder("Tulis sesuatu…")
//!     .label("Naskah")
//!     .auto_grow(6, 18)
//!     .on_change(move |d| naskah.set(d.clone()));
//! ```
//!
//! ## The heaviest component in the catalogue, and what makes it heavy
//!
//! Not the caret — `text_field` already solved that. What makes this one
//! heavier is that **the contents are no longer a string**:
//!
//! | Layer | Module | The question it answers |
//! |---|---|---|
//! | [`document`] | the model | which block, which style, what exactly was removed |
//! | [`history`] | undo | how do I run that backwards |
//! | [`editor`] | commands | what does Backspace *mean* at the start of a bullet |
//! | [`layout`] | geometry | where does a run of bold text sit, and what did that click hit |
//! | [`body`] | the render node | pixels, keys, IME, accessibility |
//! | [`state`] | the seam | how does a toolbar in another subtree reflect and command it |
//! | [`clipboard`] | copy/paste | styled inside the app, plain text outside it |
//! | [`mod@toolbar`] | the UI | toggles, the block-kind dropdown, the link dialog |
//!
//! ## What it does not write a second time
//!
//! The frame, the focus ring, auto-grow, momentum, rubber banding, the
//! scrollbar and the `SCROLL` action are all `text_area`'s and `scroll_view`'s,
//! reached through the very same [`TextAreaFrameProps`] and [`AreaLink`]. The
//! shaping, the bidi, the font fallback, the grapheme rules and the caret
//! geometry *inside* a run are `silka-text`'s. The dropdown is
//! [`mod@crate::select`] and the link dialog is [`mod@crate::dialog`] — this module
//! adds no second popup and no second modal.
//!
//! ## Definition of Done (`KOMPONEN.md`), satisfied
//!
//! - **Both presets** through semantic tokens ([`EditorStyle::from_theme`]);
//!   not one colour literal, and the code block's corner is a theme parameter.
//! - **Every interactive state on a spring** — the frame's hover/focus ring and
//!   the toolbar's buttons are the existing widgets, springs included.
//! - **Full keyboard**: the whole `text_area` keymap plus ⌘B/⌘I/⌘U/⌘K, ⌘E, and
//!   ⌘⌥0…3 — and a **Tab that always moves focus**, so the editor can never
//!   become a keyboard trap.
//! - **AccessKit**: [`AccessRole::MultilineTextInput`](silka_core::access::AccessRole::MultilineTextInput)
//!   with the document's plain text as its value, the caret and selection
//!   reported as character offsets, and `SET_VALUE` for dictation.
//! - **Dark mode** and **reduced motion** follow the tokens and the [`Tick`].
//!
//! ## Deliberately out of scope for v1 (`KOMPONEN.md` says so, and we agree)
//!
//! Tables inside the editor, images, collaboration/CRDT, and comments. Each of
//! them changes the *model*, not just the UI, and doing them badly now would be
//! harder to undo than doing them later.

pub mod body;
pub mod clipboard;
pub mod document;
pub mod editor;
pub mod history;
pub mod layout;
pub mod state;
#[cfg(test)]
mod tests;
pub mod toolbar;

use silka_core::access::{AccessAction, AccessActionRequest};
use silka_core::animation::{Spring, Tick};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{FocusRing, NodeId, RenderNode, RenderTree};
use silka_core::view::{Builder, View, ViewNode};
use silka_core::Callback;
use silka_paint::{Color, Corners, Insets};
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::scroll_view::{self, scroll_view, ScrollView, Scrollbar};
use crate::text_area::{AreaLink, FrameStyle, TextAreaFrameProps};

pub use body::WysiwygBody;
pub use clipboard::{decode, encode};
pub use document::{
    Block, BlockKind, DocPos, DocRange, DocSelection, Document, Fragment, InlineStyle, Marks,
    Piece, Span, StyleRuns,
};
pub use editor::{RichEdit, RichPreedit};
pub use history::{History, Op, Step};
pub use layout::{BlockLayout, DocLayout, EditorStyle, Segment, VisualLine};
pub use state::{
    ClipCallback, Clipping, DocumentCallback, EditorCommand, EditorHandle, EditorSnapshot,
    StateCallback,
};
pub use toolbar::{link_dialog, toolbar, LinkDialog, Toolbar};

use body::BodyConfig;

/// How much room is left around the caret when it is scrolled into view.
const REVEAL_PADDING: f32 = 6.0;

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

/// The editing body's props — **resolved tokens only**.
#[derive(Debug, Clone, PartialEq)]
pub struct WysiwygBodyProps {
    fonts: Fonts,
    document: Document,
    placeholder: String,
    style: EditorStyle,
    padding: Insets,
    caret_width: f32,
    label: Option<String>,
    disabled: bool,
    read_only: bool,
    link: AreaLink,
    handle: EditorHandle,
    on_change: Option<DocumentCallback>,
    on_state: Option<StateCallback>,
    on_copy: Option<ClipCallback>,
    on_paste: Option<Callback>,
    on_link: Option<Callback>,
}

impl ViewNode for WysiwygBodyProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(WysiwygBody::new(BodyConfig {
            fonts: self.fonts.clone(),
            style: self.style.clone(),
            placeholder: self.placeholder.clone(),
            padding: self.padding,
            caret_width: self.caret_width,
            label: self.label.clone(),
            disabled: self.disabled,
            read_only: self.read_only,
            document: self.document.clone(),
            link: self.link.clone(),
            handle: self.handle.clone(),
            on_change: self.on_change.clone(),
            on_state: self.on_state.clone(),
            on_copy: self.on_copy.clone(),
            on_paste: self.on_paste.clone(),
            on_link: self.on_link.clone(),
        }))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<WysiwygBody>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if !n.link.same(&self.link) {
            self.link.adopt(&n.link);
            n.link = self.link.clone();
        }
        if !n.handle.same(&self.handle) {
            self.handle.adopt(&n.handle);
            n.handle = self.handle.clone();
        }

        // **The document is overwritten only when the application actually
        // changed it.** Comparing props against props (never against the node's
        // contents) is the difference between an editor you can type in and one
        // that throws the caret backwards on every unrelated signal.
        if n.props_document != self.document {
            n.props_document = self.document.clone();
            if n.set_document(self.document.clone()) {
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
        }

        if *n.style() != self.style {
            n.set_style(self.style.clone());
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.placeholder != self.placeholder {
            n.placeholder.clone_from(&self.placeholder);
            n.invalidate();
            dirty |= Dirty::PAINT;
        }
        if n.padding != self.padding || n.caret_width != self.caret_width {
            n.padding = self.padding;
            n.caret_width = self.caret_width;
            n.invalidate();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.disabled != self.disabled || n.read_only != self.read_only {
            n.disabled = self.disabled;
            n.read_only = self.read_only;
            n.invalidate();
            dirty |= Dirty::PAINT;
        }
        if n.fonts != self.fonts {
            n.fonts = self.fonts.clone();
            n.invalidate();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        // Callbacks are replaced without comparison: closures are rebuilt on
        // every rebuild and capture fresh values.
        n.on_change.clone_from(&self.on_change);
        n.on_state.clone_from(&self.on_state);
        n.on_copy.clone_from(&self.on_copy);
        n.on_paste.clone_from(&self.on_paste);
        n.on_link.clone_from(&self.on_link);
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// A Dart-style `wysiwyg` builder (§2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct Wysiwyg {
    theme: Theme,
    body: WysiwygBodyProps,
    frame_style: FrameStyle,
    spring: Spring,
    min_rows: usize,
    max_rows: usize,
    auto_grow: bool,
    scrollbar: Scrollbar,
    key: Option<Key>,
    handle: EditorHandle,
}

/// A rich text editor — the `wysiwyg_editor` component (`KOMPONEN.md` Tier 6).
///
/// Every value comes from `theme`; `fonts` is the application's text engine.
/// The default is a ten-row box that scrolls, with a toolbar the application
/// mounts next to it ([`mod@toolbar`]).
pub fn wysiwyg(fonts: &Fonts, theme: &Theme, document: Document) -> Wysiwyg {
    let t = theme;
    let link = AreaLink::new();
    let handle = EditorHandle::new();
    let style = EditorStyle::from_theme(t);
    let min_rows = 10;

    let mut editor = Wysiwyg {
        theme: *t,
        body: WysiwygBodyProps {
            fonts: fonts.clone(),
            document,
            placeholder: String::new(),
            style,
            padding: Insets::symmetric(t.space(3.0), t.space(2.5)),
            // As thin as the smallest spacing step: the HIG caret is a hairline.
            caret_width: t.space(0.25),
            label: None,
            disabled: false,
            read_only: false,
            link: link.clone(),
            handle: handle.clone(),
            on_change: None,
            on_state: None,
            on_copy: None,
            on_paste: None,
            on_link: None,
        },
        frame_style: FrameStyle {
            background: t.color.surface,
            background_hover: t.color.surface_hover,
            background_focus: t.color.surface,
            border_width: t.space(0.25),
            border: t.color.border,
            border_focus: t.color.accent,
            corners: t.corners(t.radius.md),
            focus_ring: Some(FocusRing::new(t.space(0.5), t.color.focus_ring)),
        },
        spring: Spring::snappy(),
        min_rows,
        max_rows: min_rows,
        auto_grow: false,
        scrollbar: Scrollbar::default(),
        key: None,
        handle,
    };
    editor.body.link = link;
    editor
}

impl Wysiwyg {
    fn map(mut self, f: impl FnOnce(&mut Wysiwyg)) -> Self {
        f(&mut self);
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The faint text shown while the document is empty.
    pub fn placeholder(self, placeholder: impl Into<String>) -> Self {
        let p = placeholder.into();
        self.map(move |x| x.body.placeholder = p)
    }

    /// The name a screen reader announces (§3.8).
    pub fn label(self, label: impl Into<String>) -> Self {
        let l = label.into();
        self.map(move |x| x.body.label = Some(l))
    }

    /// Disable the editor: no focus, no keystrokes — still read out.
    pub fn disabled(self, disabled: bool) -> Self {
        self.map(move |x| x.body.disabled = disabled)
    }

    /// The document can be selected and copied, but not changed.
    pub fn read_only(self, read_only: bool) -> Self {
        self.map(move |x| x.body.read_only = read_only)
    }

    /// A **fixed** height of `rows` body lines; the content scrolls inside it.
    pub fn rows(self, rows: usize) -> Self {
        self.map(move |x| {
            x.min_rows = rows.max(1);
            x.max_rows = rows.max(1);
            x.auto_grow = false;
        })
    }

    /// Grow with the content between `min_rows` and `max_rows`, then scroll.
    pub fn auto_grow(self, min_rows: usize, max_rows: usize) -> Self {
        self.map(move |x| {
            x.min_rows = min_rows.max(1);
            x.max_rows = max_rows.max(min_rows.max(1));
            x.auto_grow = true;
        })
    }

    /// Share a command queue with a [`mod@toolbar`].
    ///
    /// Both pieces are usually born from the same builder, so this only has to
    /// be reached for when the toolbar is built somewhere else entirely.
    pub fn handle(self, handle: EditorHandle) -> Self {
        self.map(move |x| {
            x.handle = handle.clone();
            x.body.handle = handle;
        })
    }

    /// Called whenever the document changes — **never** with a preedit in it.
    pub fn on_change(self, f: impl Fn(&Document) + 'static) -> Self {
        let cb = DocumentCallback::new(f);
        self.map(move |x| x.body.on_change = Some(cb))
    }

    /// Called whenever what the toolbar reflects changes.
    pub fn on_state(self, f: impl Fn(&EditorSnapshot) + 'static) -> Self {
        let cb = StateCallback::new(f);
        self.map(move |x| x.body.on_state = Some(cb))
    }

    /// Called on ⌘C/⌘X with both flavours of the selection.
    pub fn on_copy(self, f: impl Fn(&Clipping) + 'static) -> Self {
        let cb = ClipCallback::new(f);
        self.map(move |x| x.body.on_copy = Some(cb))
    }

    /// Called on ⌘V: the shell reads the pasteboard and posts
    /// [`EditorCommand::InsertFragment`] or [`EditorCommand::InsertText`] back.
    pub fn on_paste(self, f: impl Fn() + 'static) -> Self {
        let cb = Callback::new(f);
        self.map(move |x| x.body.on_paste = Some(cb))
    }

    /// Called on ⌘K: the application opens its [`LinkDialog`].
    pub fn on_link(self, f: impl Fn() + 'static) -> Self {
        let cb = Callback::new(f);
        self.map(move |x| x.body.on_link = Some(cb))
    }

    /// The complete resolved visual style (rare — the theme already fills it).
    pub fn style(self, style: EditorStyle) -> Self {
        self.map(move |x| x.body.style = style)
    }

    /// Spacing inside the editor's edges — always the token scale (§2.6).
    pub fn padding(self, padding: Insets) -> Self {
        self.map(move |x| x.body.padding = padding)
    }

    /// Corner shape: squircle on Cupertino, arc on Tailwind (§3.6).
    pub fn corners(self, corners: Corners) -> Self {
        self.map(move |x| x.frame_style.corners = corners)
    }

    /// Background colour (the `surface` token).
    pub fn background(self, color: Color) -> Self {
        self.map(move |x| {
            x.frame_style.background = color;
            x.frame_style.background_focus = color;
        })
    }

    /// A border `width` thick in `color`.
    pub fn border(self, width: f32, color: Color) -> Self {
        self.map(move |x| {
            x.frame_style.border_width = width.max(0.0);
            x.frame_style.border = color;
        })
    }

    /// No keyboard focus ring (rare — an editor should keep it).
    pub fn no_focus_ring(self) -> Self {
        self.map(|x| x.frame_style.focus_ring = None)
    }

    /// When the scrollbar is visible.
    pub fn scrollbar(self, scrollbar: Scrollbar) -> Self {
        self.map(move |x| x.scrollbar = scrollbar)
    }

    /// The spring driving the hover/focus transitions.
    pub fn spring(self, spring: Spring) -> Self {
        self.map(move |x| x.spring = spring)
    }

    /// The command queue this editor listens on — hand it to [`mod@toolbar`].
    pub fn command_handle(&self) -> EditorHandle {
        self.handle.clone()
    }

    /// The height range the frame will clamp the content to, in points.
    pub fn height_range(&self) -> (f32, f32) {
        let baris = self.body.style.body.line_height_px();
        let sisip = self.body.padding.vertical();
        let min = (baris * self.min_rows.max(1) as f32 + sisip).max(MIN_HIT_TARGET);
        let max = (baris * self.max_rows.max(self.min_rows) as f32 + sisip).max(min);
        (min, max)
    }
}

impl From<Wysiwyg> for View {
    fn from(e: Wysiwyg) -> View {
        let baris = e.body.style.body.line_height_px();
        let (min_height, max_height) = e.height_range();
        // The link is taken **before** the body's props move into the view: the
        // frame and the body share exactly one of them, which is the whole
        // point of `AreaLink`.
        let link = e.body.link.clone();
        let isi: View = Builder::new(e.body).into();
        // **Riding `scroll_view`, not reimplementing it** — momentum, rubber
        // banding, the auto-hiding scrollbar, and the working `SCROLL` action
        // all come from the Tier 1 widget. It is not focusable here because the
        // editing body already is: one editor, one Tab stop.
        let gulir = scroll_view(&e.theme, isi)
            .vertical()
            .scrollbar(e.scrollbar)
            .line_height(baris)
            .focusable(false)
            .no_focus_ring();
        let frame = TextAreaFrameProps::new(
            e.frame_style,
            min_height,
            max_height,
            e.auto_grow,
            e.spring,
            link,
        );
        let mut b = Builder::new(frame).child(View::from(gulir));
        if let Some(key) = e.key {
            b = b.key(key);
        }
        b.into()
    }
}

// ---------------------------------------------------------------------------
// Tree-level operations
// ---------------------------------------------------------------------------

/// Every [`WysiwygBody`] in `tree`, in tree order.
pub fn bodies(tree: &RenderTree) -> Vec<NodeId> {
    fn jelajah(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        if tree.node_ref::<WysiwygBody>(id).is_some() {
            out.push(id);
        }
        for anak in tree.children(id) {
            jelajah(tree, *anak, out);
        }
    }
    let mut out = Vec::new();
    jelajah(tree, tree.root(), &mut out);
    out
}

/// The first editor body in `tree` — a shortcut for tests and the gallery.
pub fn first(tree: &RenderTree) -> Option<NodeId> {
    bodies(tree).into_iter().next()
}

/// Stitch every editor back together once a frame.
///
/// Three jobs, in this order and no other:
///
/// 1. **Serve the toolbar's queued commands.** They were posted during event
///    dispatch, when the render tree was already borrowed and this node might
///    not even have existed yet.
/// 2. **Re-layout when the document grew or shrank** — auto-grow and the scroll
///    extent both follow from the content, and the scroll view in between is a
///    relayout boundary that stops the change from reaching the frame.
/// 3. **Reveal the caret**, deliberately never in the same frame as the
///    relayout: revealing against a scroll extent that is about to change
///    clamps against the old maximum and stops one line short.
pub fn sync(tree: &mut RenderTree) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in bodies(tree) {
        let Some((link, handle)) = tree
            .node_ref::<WysiwygBody>(id)
            .map(|b| (b.link().clone(), b.handle().clone()))
        else {
            continue;
        };

        let perintah = handle.drain();
        if !perintah.is_empty() {
            let mut berubah = false;
            if let Some(b) = tree.node_mut_ref::<WysiwygBody>(id) {
                for p in perintah {
                    berubah |= b.apply_command(p);
                }
                b.after_external_change(berubah);
            }
            if berubah {
                tree.mark_needs_layout(id);
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            } else {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
        }

        if link.take_relayout() {
            tree.mark_needs_layout(id);
            if let Some(f) = crate::text_area::frames(tree)
                .into_iter()
                .find(|f| is_ancestor(tree, *f, id))
            {
                tree.mark_needs_layout(f);
            }
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
        let Some(caret) = tree.node_ref::<WysiwygBody>(id).map(|b| b.caret_rect()) else {
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

/// True when `ancestor` is above `node` in the tree.
fn is_ancestor(tree: &RenderTree, ancestor: NodeId, node: NodeId) -> bool {
    let mut cur = tree.parent(node);
    while let Some(id) = cur {
        if id == ancestor {
            return true;
        }
        cur = tree.parent(id);
    }
    false
}

/// Advance every editor by one frame.
///
/// The frame's own springs are advanced by [`crate::text_area::advance`] — the
/// frame really is the same node — so what is left here is the sync pass.
pub fn advance(tree: &mut RenderTree, _tick: &Tick) -> Dirty {
    sync(tree)
}

/// Serve an assistive-technology request aimed at an editor.
///
/// The body advertises `SET_VALUE`, and advertising a capability you do not
/// serve is lying to the screen reader. Dictated text arrives as plain text, so
/// it lands as plain paragraphs — the styling that was there is replaced, which
/// is exactly what "set the value of this field" means.
pub fn apply_access_action(tree: &mut RenderTree, request: &AccessActionRequest) -> bool {
    if request.action != AccessAction::SetValue {
        return false;
    }
    let Some(nilai) = request.value.clone() else {
        return false;
    };
    let berubah = tree
        .node_mut_ref::<WysiwygBody>(request.target)
        .is_some_and(|b| b.set_access_value(&nilai));
    if berubah {
        tree.mark_needs_layout(request.target);
    }
    berubah
}
