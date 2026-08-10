//! The editing body: the node that owns the document, the caret, and the
//! glyphs.
//!
//! It is the rich-text twin of [`crate::text_area::TextAreaBody`], and it
//! reuses everything about that widget that is not about *styling*:
//!
//! | Layer | Where |
//! |---|---|
//! | The frame, its focus ring, auto-grow | [`crate::text_area::TextAreaFrame`] — the same node, not a copy |
//! | Momentum, rubber band, scrollbar, the `SCROLL` action | [`mod@crate::scroll_view`] |
//! | The seam between frame, scroll view, and body | [`crate::text_area::AreaLink`] |
//! | Graphemes, words, IME preedit semantics | `silka_text` |
//! | Shaping, bidi, font fallback, caret geometry inside a run | [`silka_text::TextLayout`], through [`super::layout`] |
//!
//! What is genuinely new here is what a *document* needs and a string does not:
//! a caret that lives in a `(block, offset)` pair, a selection that crosses
//! block boundaries, styled runs that have to be shaped one by one, and block
//! decoration (bullets, numbers, the quote bar, the code tint).
//!
//! ## The keymap is written here, and why it is not shared
//!
//! [`crate::editing::handle_key`] is the shared keymap for every widget whose
//! document is a [`silka_text::TextEdit`] — a string. This one's document is a
//! tree, so the same function cannot serve it. What is shared is the *table*:
//! the meaning of every key below is identical to `text_field`/`text_area`, and
//! the tests pin it that way. The keys this widget adds are exactly the rich
//! ones: ⌘B/⌘I/⌘U/⌘K, ⌘E for inline code, and ⌘⌥0…3 for the block kind.

use silka_core::access::{AccessActions, AccessNode, AccessRole, AccessTextSelection};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, ImeEvent, KeyCode,
    KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent, PointerPhase,
};
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_paint::{Color, Insets, Point, Quad, Rect, Size};

use crate::fonts::Fonts;
use crate::text_area::AreaLink;

use super::clipboard;
use super::document::{BlockKind, DocPos, DocSelection, Document, Marks};
use super::editor::RichEdit;
use super::layout::{self, BlockInput, DocLayout, EditorStyle};
use super::state::{
    ClipCallback, Clipping, DocumentCallback, EditorCommand, EditorHandle, EditorSnapshot,
    StateCallback,
};

/// Everything the body needs to exist, tokens already resolved.
pub(super) struct BodyConfig {
    pub fonts: Fonts,
    pub style: EditorStyle,
    pub placeholder: String,
    pub padding: Insets,
    pub caret_width: f32,
    pub label: Option<String>,
    pub disabled: bool,
    pub read_only: bool,
    pub document: Document,
    pub link: AreaLink,
    pub handle: EditorHandle,
    pub on_change: Option<DocumentCallback>,
    pub on_state: Option<StateCallback>,
    pub on_copy: Option<ClipCallback>,
    pub on_paste: Option<silka_core::Callback>,
    pub on_link: Option<silka_core::Callback>,
}

/// The WYSIWYG editing body.
pub struct WysiwygBody {
    // -- configuration ------------------------------------------------------
    pub(super) fonts: Fonts,
    pub(super) style: EditorStyle,
    pub(super) placeholder: String,
    pub(super) padding: Insets,
    pub(super) caret_width: f32,
    pub(super) label: Option<String>,
    pub(super) disabled: bool,
    pub(super) read_only: bool,
    pub(super) link: AreaLink,
    pub(super) handle: EditorHandle,
    pub(super) on_change: Option<DocumentCallback>,
    pub(super) on_state: Option<StateCallback>,
    pub(super) on_copy: Option<ClipCallback>,
    pub(super) on_paste: Option<silka_core::Callback>,
    pub(super) on_link: Option<silka_core::Callback>,

    // -- state owned by the node (diffing never overwrites it) --------------
    pub(super) edit: RichEdit,
    /// The document that last **came from props**, and only from there: typing
    /// never changes it (the controlled-component rule, exactly as in
    /// `text_area`).
    pub(super) props_document: Document,
    focused: bool,
    dragging: bool,
    /// The x the caret **wants** while walking up and down.
    goal_x: Option<f32>,
    size: Size,

    // -- derived ------------------------------------------------------------
    layout: Option<DocLayout>,
    shaped_width: f32,
    shaped_scale: f32,
    shaped_revision: u64,
    revision: u64,
    showing_placeholder: bool,
    placeholder_run: Option<silka_paint::GlyphRun>,
    caret: Rect,
    selection: Vec<Rect>,
    preedit: Vec<Rect>,
    rules: Vec<(Rect, Color)>,
    snapshot: EditorSnapshot,
}

impl WysiwygBody {
    /// Build the body from already-resolved values.
    pub(super) fn new(cfg: BodyConfig) -> Self {
        let mut body = Self {
            fonts: cfg.fonts,
            style: cfg.style,
            placeholder: cfg.placeholder,
            padding: cfg.padding,
            caret_width: cfg.caret_width,
            label: cfg.label,
            disabled: cfg.disabled,
            read_only: cfg.read_only,
            link: cfg.link,
            handle: cfg.handle,
            on_change: cfg.on_change,
            on_state: cfg.on_state,
            on_copy: cfg.on_copy,
            on_paste: cfg.on_paste,
            on_link: cfg.on_link,
            edit: RichEdit::new(cfg.document.clone()),
            props_document: cfg.document,
            focused: false,
            dragging: false,
            goal_x: None,
            size: Size::ZERO,
            layout: None,
            shaped_width: f32::NAN,
            shaped_scale: f32::NAN,
            shaped_revision: u64::MAX,
            revision: 0,
            showing_placeholder: false,
            placeholder_run: None,
            caret: Rect::default(),
            selection: Vec::new(),
            preedit: Vec::new(),
            rules: Vec::new(),
            snapshot: EditorSnapshot::default(),
        };
        body.snapshot = body.build_snapshot();
        body
    }

    // -- reading -------------------------------------------------------------

    /// The document as it stands — **without** any IME preedit.
    pub fn document(&self) -> &Document {
        self.edit.document()
    }

    /// The editing model, for tests and for the tree-level helpers.
    pub fn edit(&self) -> &RichEdit {
        &self.edit
    }

    /// The current selection.
    pub fn selection(&self) -> DocSelection {
        self.edit.selection()
    }

    /// What the toolbar reflects.
    pub fn snapshot(&self) -> &EditorSnapshot {
        &self.snapshot
    }

    /// True while an IME is composing here.
    pub fn is_composing(&self) -> bool {
        self.edit.is_composing()
    }

    /// True while this body holds keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The caret rectangle in content coordinates (from the last layout).
    pub fn caret_rect(&self) -> Rect {
        self.caret
    }

    /// The selection highlight rectangles.
    pub fn selection_rects(&self) -> &[Rect] {
        &self.selection
    }

    /// True when what is on screen is the placeholder.
    pub fn shows_placeholder(&self) -> bool {
        self.showing_placeholder
    }

    /// How many visual lines the document currently occupies.
    pub fn line_count(&self) -> usize {
        self.layout
            .as_ref()
            .map_or(1, |l| l.flat_lines().len().max(1))
    }

    /// The document's laid-out geometry, when there is one.
    pub fn doc_layout(&self) -> Option<&DocLayout> {
        self.layout.as_ref()
    }

    /// The seam to the frame and the scroll view around it.
    pub(super) fn link(&self) -> &AreaLink {
        &self.link
    }

    /// The command queue shared with the toolbar.
    pub(super) fn handle(&self) -> &EditorHandle {
        &self.handle
    }

    // -- geometry ------------------------------------------------------------

    fn editable(&self) -> bool {
        !self.disabled && !self.read_only
    }

    /// The top-left corner of the text block, in node-local coordinates.
    fn origin(&self) -> Point {
        Point::new(self.padding.left, self.padding.top)
    }

    /// The width the text may occupy — what soft wrapping breaks against.
    fn text_width(&self) -> f32 {
        (self.size.width - self.padding.horizontal()).max(1.0)
    }

    /// Throw away the cached layout.
    pub(super) fn invalidate(&mut self) {
        self.shaped_scale = f32::NAN;
    }

    /// Mark the document as changed, so the next layout re-shapes it.
    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Re-shape when the document, the width, or the screen resolution changed
    /// — and never otherwise.
    fn ensure_layout(&mut self) {
        let lebar = self.text_width();
        let scale = self.fonts.scale_factor();
        if self.layout.is_some()
            && self.shaped_width == lebar
            && self.shaped_scale == scale
            && self.shaped_revision == self.revision
        {
            return;
        }
        let doc = self.edit.document();
        let mut tampil = Vec::with_capacity(doc.block_count());
        for i in 0..doc.block_count() {
            tampil.push((
                doc.block(i).kind,
                self.edit.display_spans(i).0,
                doc.list_number(i),
            ));
        }
        let masukan: Vec<BlockInput<'_>> = tampil
            .iter()
            .map(|(kind, spans, number)| BlockInput {
                kind: *kind,
                spans,
                number: *number,
            })
            .collect();
        let gaya = self.style.clone();
        let dinonaktifkan = self.disabled;
        let asal = self.origin();
        let mut hasil = self
            .fonts
            .with(|m| layout::build(m, &masukan, &gaya, lebar, dinonaktifkan));
        layout::place_runs(&mut hasil, asal);
        self.layout = Some(hasil);
        self.shaped_width = lebar;
        self.shaped_scale = scale;
        self.shaped_revision = self.revision;

        // The placeholder is drawn only for a genuinely empty document, and it
        // is a separate run because it is not part of the model.
        self.showing_placeholder = self.edit.document().is_empty() && !self.edit.is_composing();
        self.placeholder_run = if self.showing_placeholder && !self.placeholder.is_empty() {
            let teks = self.placeholder.clone();
            let gaya = self.style.body.clone();
            let warna = self.style.placeholder;
            Some(self.fonts.with(|m| {
                let l = m.layout(&teks, &gaya, silka_text::TextConstraints::width(lebar));
                m.rasterize(&l, asal, warna)
            }))
        } else {
            None
        };
    }

    /// Recompute caret, selection, preedit underline, and the decoration rules.
    fn update_geometry(&mut self) {
        let asal = self.origin();
        let Some(l) = self.layout.as_ref() else {
            return;
        };
        let tampil = self.edit.display_selection();
        let caret = l.caret(tampil.focus, self.caret_width);
        self.caret = Rect::new(
            caret.origin.x + asal.x,
            caret.origin.y + asal.y,
            caret.size.width,
            caret.size.height,
        );

        let geser = |r: Rect| {
            Rect::new(
                r.origin.x + asal.x,
                r.origin.y + asal.y,
                r.size.width,
                r.size.height,
            )
        };
        self.selection = if tampil.is_collapsed() {
            Vec::new()
        } else {
            l.selection_rects(tampil.range())
                .into_iter()
                .map(geser)
                .collect()
        };

        // The preedit underline: as thick as the caret, hugging the baseline —
        // the shape every OS uses for "this is not final yet" (§3.8).
        self.preedit = Vec::new();
        if let Some(p) = self.edit.preedit() {
            let r = super::document::DocRange::new(
                DocPos::new(p.at.block, p.at.offset),
                DocPos::new(p.at.block, p.at.offset + p.text.len()),
            );
            for k in l.selection_rects(r) {
                let g = geser(k);
                self.preedit.push(Rect::new(
                    g.origin.x,
                    g.max_y() - self.caret_width,
                    g.size.width,
                    self.caret_width,
                ));
            }
        }

        // Underlines and strikethroughs are quads, not font features: the paint
        // layer has no stroke command yet (§3.2), and a link that is only
        // coloured is not accessible enough on its own.
        self.rules.clear();
        let tebal = self.style.rule;
        for b in &l.blocks {
            for line in &b.lines {
                for seg in &line.segments {
                    let kiri = asal.x + b.content_x + seg.x;
                    let atas = asal.y + b.top + b.content_y + line.top;
                    let bergaris =
                        seg.style.marks.contains(Marks::UNDERLINE) || seg.style.is_link();
                    if bergaris {
                        self.rules.push((
                            Rect::new(kiri, atas + line.baseline + tebal, seg.width, tebal),
                            seg.color,
                        ));
                    }
                    if seg.style.marks.contains(Marks::STRIKE) {
                        self.rules.push((
                            Rect::new(kiri, atas + line.height * 0.5, seg.width, tebal),
                            seg.color,
                        ));
                    }
                }
            }
        }
    }

    // -- publishing ----------------------------------------------------------

    fn build_snapshot(&self) -> EditorSnapshot {
        EditorSnapshot {
            marks: self.edit.active_marks(),
            link: self.edit.active_link(),
            kind: self.edit.active_kind(),
            can_undo: self.edit.can_undo(),
            can_redo: self.edit.can_redo(),
            focused: self.focused,
            has_selection: !self.edit.selection().is_collapsed(),
            selected_text: self.edit.selected_plain_text(),
        }
    }

    /// Tell the application what the toolbar should look like — but only when
    /// it actually changed, so a mouse move does not rebuild the toolbar.
    fn publish_state(&mut self) {
        let baru = self.build_snapshot();
        if baru == self.snapshot {
            return;
        }
        self.snapshot = baru;
        if let Some(cb) = self.on_state.clone() {
            cb.call(&self.snapshot);
        }
    }

    /// After the document changed: re-shape, recompute geometry, tell the app.
    fn after_document_changed(&mut self, ctx: &mut EventCtx<'_>) {
        self.goal_x = None;
        self.touch();
        self.ensure_layout();
        self.update_geometry();
        // `props_document` is deliberately **not** touched: it records what the
        // application last handed us, not what the user typed.
        if let Some(cb) = self.on_change.clone() {
            cb.call(self.edit.document());
        }
        self.publish_state();
        self.link.request_relayout();
        ctx.request_layout();
        self.request_reveal();
        self.update_ime(ctx);
    }

    /// After the caret moved but the document did not.
    fn after_caret_changed(&mut self, ctx: &mut EventCtx<'_>) {
        self.update_geometry();
        self.publish_state();
        ctx.request_paint();
        self.request_reveal();
        self.update_ime(ctx);
    }

    fn request_reveal(&self) {
        if self.focused {
            self.link.request_reveal();
        }
    }

    fn update_ime(&self, ctx: &mut EventCtx<'_>) {
        if !self.focused || self.disabled {
            return;
        }
        let b = ctx.bounds();
        ctx.request_ime(Rect::from_origin_size(
            Point::new(
                b.origin.x + self.caret.origin.x,
                b.origin.y + self.caret.origin.y,
            ),
            self.caret.size,
        ));
    }

    // -- commands ------------------------------------------------------------

    /// Serve one queued toolbar command; true when something changed.
    pub(super) fn apply_command(&mut self, command: EditorCommand) -> bool {
        if !self.editable() {
            return false;
        }
        match command {
            EditorCommand::ToggleMark(m) => self.edit.toggle_mark(m),
            EditorCommand::SetBlockKind(k) => self.edit.set_block_kind(k),
            EditorCommand::SetLink(url) => self.edit.set_link(url),
            EditorCommand::Undo => self.edit.undo(),
            EditorCommand::Redo => self.edit.redo(),
            EditorCommand::InsertText(t) => self.edit.insert_text(&t),
            EditorCommand::InsertFragment(f) => self.edit.insert_fragment(f),
        }
    }

    /// Recompute everything after commands were served outside an event.
    pub(super) fn after_external_change(&mut self, document_changed: bool) {
        if document_changed {
            self.touch();
        }
        self.ensure_layout();
        self.update_geometry();
        if document_changed {
            if let Some(cb) = self.on_change.clone() {
                cb.call(self.edit.document());
            }
            self.link.request_relayout();
        }
        self.publish_state();
        self.request_reveal();
    }

    /// Replace the document at the application's request.
    pub(super) fn set_document(&mut self, doc: Document) -> bool {
        if *self.edit.document() == doc {
            return false;
        }
        self.edit.set_document(doc);
        self.touch();
        self.invalidate();
        true
    }

    /// Replace the whole document from assistive technology (dictation).
    pub(super) fn set_access_value(&mut self, value: &str) -> bool {
        if !self.editable() {
            return false;
        }
        let doc = Document::from_plain(value);
        if *self.edit.document() == doc {
            return false;
        }
        self.edit.set_document(doc);
        self.touch();
        self.ensure_layout();
        self.update_geometry();
        if let Some(cb) = self.on_change.clone() {
            cb.call(self.edit.document());
        }
        self.publish_state();
        self.link.request_relayout();
        true
    }

    // -- movement ------------------------------------------------------------

    /// Move the caret `direction` visual lines, keeping the goal column.
    fn move_line(&mut self, direction: i32, extend: bool) -> bool {
        let Some(l) = self.layout.as_ref() else {
            return false;
        };
        let baris = l.flat_lines();
        if baris.is_empty() {
            return false;
        }
        let fokus = self.edit.selection().focus;
        let sekarang = l.flat_index(fokus) as i32;
        let x = match self.goal_x {
            Some(x) => x,
            None => {
                let c = l.caret(fokus, self.caret_width);
                self.goal_x = Some(c.origin.x);
                c.origin.x
            }
        };
        let tujuan = sekarang + direction;
        let pos = if tujuan < 0 {
            DocPos::START
        } else if tujuan as usize >= baris.len() {
            self.edit.document().end()
        } else {
            l.position_on_line(tujuan as usize, x)
        };
        self.edit.place_caret(pos, extend)
    }

    /// Move the caret one viewport up or down.
    fn move_page(&mut self, direction: f32, extend: bool) -> bool {
        let Some(l) = self.layout.as_ref() else {
            return false;
        };
        let fokus = self.edit.selection().focus;
        let c = l.caret(fokus, self.caret_width);
        let x = *self.goal_x.get_or_insert(c.origin.x);
        let tinggi = self.link.viewport().height.max(c.size.height * 3.0);
        let y = c.origin.y + c.size.height * 0.5 + direction * tinggi;
        let pos = l.hit(Point::new(x, y));
        self.edit.place_caret(pos, extend)
    }

    /// The ends of the visual line the caret is on (Home/End).
    fn line_bounds(&self, end: bool) -> DocPos {
        let fokus = self.edit.selection().focus;
        match self.layout.as_ref() {
            Some(l) => {
                let (a, z) = l.visual_line_bounds(fokus);
                if end {
                    z
                } else {
                    a
                }
            }
            None => fokus,
        }
    }

    /// The document position under a node-local point.
    fn position_at(&self, local: Point) -> DocPos {
        let asal = self.origin();
        match self.layout.as_ref() {
            Some(l) => {
                let p = l.hit(Point::new(local.x - asal.x, local.y - asal.y));
                self.edit.model_position(p)
            }
            None => DocPos::START,
        }
    }

    // -- keyboard ------------------------------------------------------------

    fn key(&mut self, ctx: &mut EventCtx<'_>, k: &KeyEvent) {
        // **While an IME is composing, the ordinary key path is held back**
        // (§3.8): the letters being picked in the candidate window must not
        // also land as keystrokes.
        if self.edit.is_composing() {
            ctx.handled();
            return;
        }

        let m = k.modifiers;
        let shift = m.contains(Modifiers::SHIFT);
        let alt = m.contains(Modifiers::ALT);
        let dokumen = m.contains(Modifiers::COMMAND) || m.contains(Modifiers::CONTROL);
        let sunting = self.editable();
        let sebelum = self.edit.document().clone();
        let seleksi_sebelum = self.edit.selection();
        let mut tertangani = true;
        let mut jaga_goal = false;

        match &k.code {
            KeyCode::Named(NamedKey::ArrowUp) if dokumen => {
                self.edit.move_document_start(shift);
            }
            KeyCode::Named(NamedKey::ArrowDown) if dokumen => {
                self.edit.move_document_end(shift);
            }
            KeyCode::Named(NamedKey::ArrowUp) => {
                self.move_line(-1, shift);
                jaga_goal = true;
            }
            KeyCode::Named(NamedKey::ArrowDown) => {
                self.move_line(1, shift);
                jaga_goal = true;
            }
            KeyCode::Named(NamedKey::PageUp) => {
                self.move_page(-1.0, shift);
                jaga_goal = true;
            }
            KeyCode::Named(NamedKey::PageDown) => {
                self.move_page(1.0, shift);
                jaga_goal = true;
            }
            KeyCode::Named(NamedKey::ArrowLeft) => {
                if dokumen {
                    let p = self.line_bounds(false);
                    self.edit.place_caret(p, shift);
                } else if alt {
                    self.edit.move_prev_word(shift);
                } else {
                    self.edit.move_prev(shift);
                }
            }
            KeyCode::Named(NamedKey::ArrowRight) => {
                if dokumen {
                    let p = self.line_bounds(true);
                    self.edit.place_caret(p, shift);
                } else if alt {
                    self.edit.move_next_word(shift);
                } else {
                    self.edit.move_next(shift);
                }
            }
            KeyCode::Named(NamedKey::Home) if dokumen => {
                self.edit.move_document_start(shift);
            }
            KeyCode::Named(NamedKey::End) if dokumen => {
                self.edit.move_document_end(shift);
            }
            KeyCode::Named(NamedKey::Home) => {
                let p = self.line_bounds(false);
                self.edit.place_caret(p, shift);
            }
            KeyCode::Named(NamedKey::End) => {
                let p = self.line_bounds(true);
                self.edit.place_caret(p, shift);
            }
            KeyCode::Named(NamedKey::Enter) if sunting && !dokumen => {
                self.edit.split_block();
            }
            KeyCode::Named(NamedKey::Backspace) if sunting => {
                if alt || m.contains(Modifiers::COMMAND) {
                    self.edit.delete_word_backward();
                } else {
                    self.edit.delete_backward();
                }
            }
            KeyCode::Named(NamedKey::Delete) if sunting => {
                if alt || m.contains(Modifiers::COMMAND) {
                    self.edit.delete_word_forward();
                } else {
                    self.edit.delete_forward();
                }
            }
            KeyCode::Named(NamedKey::Space) if sunting && !dokumen => {
                self.edit.insert_text(k.text.as_deref().unwrap_or(" "));
            }
            // **Tab always moves focus.** A text box that swallows Tab is a
            // keyboard trap, and an editor with a toolbar has somewhere to go
            // (`KOMPONEN.md` DoD, §3.8).
            KeyCode::Named(NamedKey::Tab) => tertangani = false,
            KeyCode::Character(c) if dokumen => {
                tertangani = self.command_key(*c, shift, alt, sunting);
            }
            KeyCode::Character(c) if sunting && !m.contains(Modifiers::CONTROL) => {
                let teks = k.text.clone().unwrap_or_else(|| c.to_string());
                self.edit.insert_text(&teks);
            }
            _ => tertangani = false,
        }

        if !tertangani {
            return;
        }
        if !jaga_goal {
            self.goal_x = None;
        }
        ctx.handled();
        if *self.edit.document() != sebelum {
            self.after_document_changed(ctx);
        } else if self.edit.selection() != seleksi_sebelum || matches!(&k.code, KeyCode::Named(_)) {
            self.after_caret_changed(ctx);
        } else {
            // A style armed at a collapsed caret (⌘B with nothing selected)
            // changes neither document nor selection, and the toolbar still has
            // to light up.
            self.publish_state();
            ctx.request_paint();
        }
    }

    /// The ⌘-shortcuts. Returns false when the key is not ours and must bubble.
    fn command_key(&mut self, c: char, shift: bool, alt: bool, editable: bool) -> bool {
        match c.to_ascii_lowercase() {
            'a' => self.edit.select_all(),
            'z' if editable => {
                if shift {
                    self.edit.redo()
                } else {
                    self.edit.undo()
                }
            }
            'b' if editable && !alt => self.edit.toggle_mark(Marks::BOLD),
            'i' if editable && !alt => self.edit.toggle_mark(Marks::ITALIC),
            'u' if editable && !alt => self.edit.toggle_mark(Marks::UNDERLINE),
            'e' if editable && !alt => self.edit.toggle_mark(Marks::CODE),
            'x' if editable && shift => self.edit.toggle_mark(Marks::STRIKE),
            'k' if editable && !alt => {
                // The widget cannot open a dialog — it has no overlay layer of
                // its own — so it asks, and the application opens the one it
                // already mounted (`dialog`, Tier 4).
                self.publish_state();
                if let Some(cb) = self.on_link.clone() {
                    cb.call();
                }
                true
            }
            'c' | 'x' => {
                // Copy and cut are served here because the *content* is ours;
                // the pasteboard is not (INTEGRASI-NATIVE §4), so what leaves is
                // both flavours and the shell decides.
                let potongan = self.edit.selected_fragment();
                if self.edit.selection().is_collapsed() {
                    return false;
                }
                if let Some(cb) = self.on_copy.clone() {
                    cb.call(&Clipping {
                        rich: clipboard::encode(&potongan),
                        plain: potongan.plain_text(),
                    });
                }
                if c == 'x' && editable {
                    self.edit.delete_backward();
                }
                true
            }
            'v' if editable => {
                // The paste itself arrives back as a command once the shell has
                // read the pasteboard.
                match self.on_paste.clone() {
                    Some(cb) => {
                        cb.call();
                        true
                    }
                    None => false,
                }
            }
            // ⌘⌥0…3: paragraph and the three heading levels — the shortcut every
            // editor with headings has.
            '0' if editable && alt => self.edit.set_block_kind(BlockKind::Paragraph),
            '1' if editable && alt => self.edit.set_block_kind(BlockKind::Heading1),
            '2' if editable && alt => self.edit.set_block_kind(BlockKind::Heading2),
            '3' if editable && alt => self.edit.set_block_kind(BlockKind::Heading3),
            _ => false,
        }
    }

    // -- IME -----------------------------------------------------------------

    fn ime(&mut self, ctx: &mut EventCtx<'_>, e: &ImeEvent) {
        if !self.editable() {
            return;
        }
        let sebelum = self.edit.document().clone();
        let berubah = match e {
            ImeEvent::Enabled => false,
            ImeEvent::Preedit { text, cursor } => self.edit.set_preedit(text, *cursor),
            ImeEvent::Commit(teks) => self.edit.commit_text(teks),
            ImeEvent::Disabled => self.edit.clear_preedit(),
        };
        if !berubah {
            return;
        }
        ctx.handled();
        if *self.edit.document() != sebelum {
            self.after_document_changed(ctx);
        } else {
            // The **visible** text changed even though the document did not, so
            // it still has to be shaped again — and the application is told
            // nothing.
            self.touch();
            self.ensure_layout();
            self.update_geometry();
            self.link.request_relayout();
            ctx.request_layout();
            self.request_reveal();
            self.update_ime(ctx);
        }
    }

    // -- pointer -------------------------------------------------------------

    fn pointer(&mut self, ctx: &mut EventCtx<'_>, p: &PointerEvent) {
        match p.phase {
            PointerPhase::Enter => {
                if !self.link.hovered() {
                    self.link.set_hovered(true);
                    ctx.request_paint();
                    ctx.request_animation();
                }
            }
            PointerPhase::Leave => {
                if self.link.hovered() {
                    self.link.set_hovered(false);
                    ctx.request_paint();
                    ctx.request_animation();
                }
            }
            PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                ctx.request_focus();
                ctx.capture_pointer();
                self.dragging = true;
                self.goal_x = None;
                let pos = self.position_at(ctx.local());
                match p.click_count {
                    // Double click = one word. Triple click = the **block**, not
                    // the document: ⌘A is how you take everything.
                    2 => {
                        self.edit.select_word_at(pos);
                    }
                    n if n >= 3 => {
                        self.edit.select_block_at(pos);
                    }
                    _ => {
                        self.edit
                            .place_caret(pos, p.modifiers.contains(Modifiers::SHIFT));
                    }
                }
                self.after_caret_changed(ctx);
                ctx.handled();
            }
            PointerPhase::Move if self.dragging => {
                let pos = self.position_at(ctx.local());
                if self.edit.place_caret(pos, true) {
                    self.after_caret_changed(ctx);
                }
                ctx.handled();
            }
            PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                self.dragging = false;
                ctx.release_pointer();
                ctx.handled();
            }
            PointerPhase::Cancel => self.dragging = false,
            _ => {}
        }
    }
}

impl RenderNode for WysiwygBody {
    fn type_name(&self) -> &'static str {
        "WysiwygBody"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // The scroll view above hands down a tight width and an unbounded
        // height: the width is what wrapping breaks against, the height is ours.
        let lebar = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            constraints.min_width.max(1.0)
        };
        self.size = Size::new(lebar, self.size.height);
        self.ensure_layout();

        let isi = self
            .layout
            .as_ref()
            .map_or(self.style.body.line_height_px(), |l| l.size.height);
        let alami = isi + self.padding.vertical();
        self.link.set_content(alami);

        // Fill the viewport when the document is shorter than it, so a click in
        // the empty space below the last line still lands in the text.
        let tinggi = alami.max(self.link.viewport().height);
        self.size = constraints.constrain(Size::new(lebar, tinggi));
        self.update_geometry();
        self.size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let asal = self.origin();
        let Some(l) = self.layout.as_ref() else {
            return;
        };

        // 1. Block decoration: the code tint and the quote bar are the block's
        //    own background, so they go underneath everything.
        for b in &l.blocks {
            let kotak = Rect::new(
                asal.x + b.content_x - self.style.indent_of(b.kind),
                asal.y + b.top,
                (self.text_width() - b.content_x + self.style.indent_of(b.kind)).max(0.0),
                b.height,
            );
            match b.kind {
                BlockKind::Code if self.style.code_background.a > 0.0 => {
                    ctx.quad(
                        Quad::new(kotak)
                            .background(self.style.code_background)
                            .corners(self.style.code_corners),
                    );
                }
                BlockKind::Quote if self.style.quote_bar.a > 0.0 => {
                    ctx.quad(
                        Quad::new(Rect::new(
                            asal.x,
                            asal.y + b.top,
                            self.style.rule * 2.0,
                            b.height,
                        ))
                        .background(self.style.quote_bar),
                    );
                }
                _ => {}
            }
        }

        // 2. Inline code carries its own tint, drawn per run rather than per
        //    block: `like this` inside a sentence has to be legible without
        //    the sentence changing shape around it.
        if self.style.code_background.a > 0.0 {
            for b in &l.blocks {
                if b.kind == BlockKind::Code {
                    continue;
                }
                for line in &b.lines {
                    for seg in &line.segments {
                        if !seg.style.marks.contains(Marks::CODE) {
                            continue;
                        }
                        let kotak = Rect::new(
                            asal.x + b.content_x + seg.x,
                            asal.y + b.top + b.content_y + line.top,
                            seg.width,
                            line.height,
                        );
                        ctx.quad(
                            Quad::new(kotak)
                                .background(self.style.code_background)
                                .corners(self.style.code_corners),
                        );
                    }
                }
            }
        }

        // 3. The selection goes under the text; its alpha follows focus so an
        //    unfocused editor does not still look live.
        if !self.selection.is_empty() && !self.disabled {
            let hidup = if self.focused { 1.0 } else { 0.35 };
            let warna = self
                .style
                .selection
                .with_alpha(self.style.selection.a * hidup);
            for r in &self.selection {
                ctx.quad(Quad::new(*r).background(warna));
            }
        }

        // 4. The glyphs — markers first, then every styled run.
        for b in &l.blocks {
            if let Some(mk) = &b.marker {
                if !mk.run.is_empty() {
                    ctx.glyph_run(mk.run.clone());
                }
            }
            for line in &b.lines {
                for seg in &line.segments {
                    if seg.run.is_empty() {
                        continue;
                    }
                    let kotak = Rect::new(
                        asal.x + b.content_x + seg.x,
                        asal.y + b.top + b.content_y + line.top,
                        seg.width,
                        line.height,
                    );
                    // A long document is shaped whole but only drawn where it
                    // is visible — the scroll view's clip is the authority.
                    if ctx.is_visible(kotak) {
                        ctx.glyph_run(seg.run.clone());
                    }
                }
            }
        }

        if let Some(run) = &self.placeholder_run {
            if !run.is_empty() {
                ctx.glyph_run(run.clone());
            }
        }

        // 5. Underlines and strikethroughs, then the preedit underline on top of
        //    the text it marks.
        for (r, c) in &self.rules {
            ctx.quad(Quad::new(*r).background(*c));
        }
        for r in &self.preedit {
            ctx.quad(Quad::new(*r).background(self.style.text));
        }

        if self.focused && !self.disabled && self.edit.display_selection().is_collapsed() {
            ctx.quad(Quad::new(self.caret).background(self.style.caret));
        }
    }

    fn access(&self, node: &mut AccessNode) {
        // The **multiline** role: it is what tells a screen reader that ↑/↓ move
        // between lines here and that Return does not submit.
        node.role = AccessRole::MultilineTextInput;
        node.label.clone_from(&self.label);
        let teks = self.edit.document().access_text();
        let sel = self.edit.selection();
        let anchor = self.edit.document().flat_offset(sel.anchor);
        let focus = self.edit.document().flat_offset(sel.focus);
        node.text_selection = Some(AccessTextSelection::from_bytes(&teks, anchor, focus));
        node.value = Some(teks);
        node.disabled = self.disabled;
        if !self.disabled {
            node.actions |= AccessActions::CLICK | AccessActions::FOCUS;
            if !self.read_only {
                node.actions |= AccessActions::SET_VALUE;
            }
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rect
    }

    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.disabled {
            FocusPolicy::NONE
        } else {
            FocusPolicy::FOCUSABLE
        }
    }

    fn cursor(&self) -> Option<CursorIcon> {
        (!self.disabled).then_some(CursorIcon::Text)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if self.disabled {
            if matches!(event, Event::Pointer(p) if matches!(p.phase, PointerPhase::Down | PointerPhase::Up))
            {
                ctx.handled();
            }
            return;
        }
        match event {
            Event::Pointer(p) => self.pointer(ctx, p),
            Event::Key(k) if k.is_pressed() => self.key(ctx, k),
            Event::Ime(e) => self.ime(ctx, e),
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                self.link.set_focused(self.focused);
                if !self.focused {
                    self.dragging = false;
                    // A composition left dangling when focus leaves is thrown
                    // away: the IME will never send its commit now.
                    if self.edit.clear_preedit() {
                        self.touch();
                        self.ensure_layout();
                    }
                } else {
                    self.request_reveal();
                }
                self.update_geometry();
                self.publish_state();
                ctx.request_paint();
                ctx.request_animation();
                if self.focused {
                    self.update_ime(ctx);
                } else {
                    ctx.disable_ime();
                }
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for WysiwygBody {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WysiwygBody")
            .field("blocks", &self.edit.document().block_count())
            .field("selection", &self.edit.selection())
            .field("focused", &self.focused)
            .field("composing", &self.edit.is_composing())
            .finish()
    }
}

/// The visual style a body was built with — used by the props diff.
impl WysiwygBody {
    pub(super) fn style(&self) -> &EditorStyle {
        &self.style
    }

    /// Replace the resolved style (a theme change).
    pub(super) fn set_style(&mut self, style: EditorStyle) {
        self.style = style;
        self.invalidate();
    }
}
