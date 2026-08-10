//! The editing body: the node that owns the document, the caret, and the
//! glyphs.
//!
//! It is the multi-line twin of [`crate::text_field::TextFieldBox`], and the
//! parts they share are genuinely shared rather than copied:
//!
//! - the document itself is [`silka_text::TextEdit`] in
//!   [`TextEdit::multiline`] mode — the same graphemes, the same words, the
//!   same undo coalescing, the same IME preedit;
//! - the keymap is [`crate::editing::handle_key`] — the same arrows, the same
//!   ⌘Z, the same typing path;
//! - the geometry comes from [`silka_text::TextLayout`] — the same caret and
//!   selection rectangles.
//!
//! What genuinely belongs to a multi-line editor, and therefore lives here:
//! soft wrapping against the width, ↑/↓ across **visual** lines with a goal
//! column, Home/End per visual line, ⌘/Ctrl+Home/End across the document,
//! PageUp/PageDown by a viewport, Enter as a new line, a configurable Tab, and
//! the line-number gutter.

use silka_core::access::{AccessActions, AccessNode, AccessRole, AccessTextSelection};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, ImeEvent, KeyCode,
    KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent, PointerPhase,
};
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_paint::{Color, GlyphRun, Insets, Point, Quad, Rect, Size};
use silka_text::{Caret, LineMetrics, Selection, TextConstraints, TextEdit, TextLayout, TextStyle};

use crate::editing::{self, EditCaps, TextCallback};
use crate::fonts::Fonts;

use super::link::AreaLink;
use super::TabBehavior;

/// Colours the body draws with — every one of them **already resolved from
/// tokens** one level up, so this node holds no opinion about colour (§2.7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyColors {
    /// The text itself.
    pub text: Color,
    /// The placeholder shown while the area is empty.
    pub placeholder: Color,
    /// The text of a disabled area.
    pub disabled: Color,
    /// The selection highlight.
    pub selection: Color,
    /// The caret.
    pub caret: Color,
    /// Line numbers in the gutter.
    pub gutter: Color,
    /// The gutter's background.
    pub gutter_background: Color,
    /// The hairline between gutter and text.
    pub gutter_separator: Color,
}

/// The `text_area` editing body.
pub struct TextAreaBody {
    // -- configuration -------------------------------------------------------
    pub(super) fonts: Fonts,
    pub(super) style: TextStyle,
    pub(super) placeholder: String,
    pub(super) padding: Insets,
    pub(super) caret_width: f32,
    pub(super) line_numbers: bool,
    pub(super) gutter_gap: f32,
    pub(super) label: Option<String>,
    pub(super) disabled: bool,
    pub(super) read_only: bool,
    pub(super) tab: TabBehavior,
    pub(super) colors: BodyColors,
    pub(super) on_change: Option<TextCallback>,
    pub(super) on_submit: Option<TextCallback>,
    pub(super) link: AreaLink,

    // -- state owned by the node (diffing never overwrites it) ---------------
    pub(super) edit: TextEdit,
    /// The value that last **came from props**, and only from there: typing
    /// never changes it (the controlled-component rule, exactly as in
    /// `text_field`).
    pub(super) props_value: String,
    focused: bool,
    dragging: bool,
    /// The x the caret **wants** while walking up and down — a real editor's
    /// goal column. `None` means the next vertical move sets it.
    goal_x: Option<f32>,
    size: Size,

    // -- derived: always a consequence of the above --------------------------
    layout: Option<TextLayout>,
    shaped: String,
    shaped_scale: f32,
    shaped_width: f32,
    showing_placeholder: bool,
    lines: Vec<LineMetrics>,
    gutter_width: f32,
    run: GlyphRun,
    gutter_runs: Vec<(Rect, GlyphRun)>,
    caret: Rect,
    selection: Vec<Rect>,
    preedit: Vec<Rect>,
}

/// Everything a body needs to exist, with tokens already resolved.
///
/// A named struct rather than a long argument list: a text area has fifteen
/// settings, and fifteen positional arguments is how two colours end up
/// swapped without the compiler noticing.
pub(super) struct BodyConfig {
    pub fonts: Fonts,
    pub style: TextStyle,
    pub placeholder: String,
    pub padding: Insets,
    pub caret_width: f32,
    pub line_numbers: bool,
    pub gutter_gap: f32,
    pub label: Option<String>,
    pub disabled: bool,
    pub read_only: bool,
    pub tab: TabBehavior,
    pub colors: BodyColors,
    pub on_change: Option<TextCallback>,
    pub on_submit: Option<TextCallback>,
    pub link: AreaLink,
    pub value: String,
}

impl TextAreaBody {
    /// Build the body from already-resolved values.
    pub(super) fn new(cfg: BodyConfig) -> Self {
        let colors = cfg.colors;
        Self {
            fonts: cfg.fonts,
            style: cfg.style,
            placeholder: cfg.placeholder,
            padding: cfg.padding,
            caret_width: cfg.caret_width,
            line_numbers: cfg.line_numbers,
            gutter_gap: cfg.gutter_gap,
            label: cfg.label,
            disabled: cfg.disabled,
            read_only: cfg.read_only,
            tab: cfg.tab,
            colors,
            on_change: cfg.on_change,
            on_submit: cfg.on_submit,
            link: cfg.link,
            // **Multiline** is what makes newlines and tabs real content
            // instead of characters to be filtered away.
            edit: TextEdit::new(cfg.value.clone()).multiline(true),
            props_value: cfg.value,
            focused: false,
            dragging: false,
            goal_x: None,
            size: Size::ZERO,
            layout: None,
            shaped: String::new(),
            shaped_scale: f32::NAN,
            shaped_width: f32::NAN,
            showing_placeholder: false,
            lines: Vec::new(),
            gutter_width: 0.0,
            run: GlyphRun::new(colors.text),
            gutter_runs: Vec::new(),
            caret: Rect::default(),
            selection: Vec::new(),
            preedit: Vec::new(),
        }
    }

    /// The **committed** contents — without the preedit being composed.
    pub fn text(&self) -> &str {
        self.edit.text()
    }

    /// The current selection (byte indices).
    pub fn selection(&self) -> Selection {
        self.edit.selection()
    }

    /// True while an IME is composing here.
    pub fn is_composing(&self) -> bool {
        self.edit.is_composing()
    }

    /// The caret rectangle in **content** coordinates (from the last layout).
    pub fn caret_rect(&self) -> Rect {
        self.caret
    }

    /// The selection highlight rectangles, in content coordinates.
    pub fn selection_rects(&self) -> &[Rect] {
        &self.selection
    }

    /// The IME preedit underlines, in content coordinates.
    pub fn preedit_rects(&self) -> &[Rect] {
        &self.preedit
    }

    /// True when what is on screen is the placeholder (an empty area).
    pub fn shows_placeholder(&self) -> bool {
        self.showing_placeholder
    }

    /// The visual lines of the current layout — one entry per **wrapped** line.
    pub fn lines(&self) -> &[LineMetrics] {
        &self.lines
    }

    /// How many visual lines the content currently occupies.
    pub fn line_count(&self) -> usize {
        self.lines.len().max(1)
    }

    /// Width reserved for the line-number gutter (0 when it is off).
    pub fn gutter_width(&self) -> f32 {
        self.gutter_width
    }

    /// The 1-based source line the caret sits on — what a screen reader and a
    /// status bar both want.
    pub fn caret_line(&self) -> usize {
        self.caret_teks().line + 1
    }

    /// True while this body holds keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The seam to the frame and the scroll view around it.
    pub(super) fn link(&self) -> &AreaLink {
        &self.link
    }

    // -- geometry -----------------------------------------------------------

    /// Editable at all?
    fn bisa_sunting(&self) -> bool {
        !self.disabled && !self.read_only
    }

    /// The top-left corner of the text block, in node-local coordinates.
    fn asal_teks(&self) -> Point {
        Point::new(self.padding.left + self.gutter_width, self.padding.top)
    }

    /// The width the text may occupy — what soft wrapping breaks against.
    fn lebar_teks(&self) -> f32 {
        (self.size.width - self.padding.horizontal() - self.gutter_width).max(1.0)
    }

    /// How wide the gutter has to be for the **largest** line number it will
    /// ever show at this length.
    fn hitung_gutter(&mut self) -> f32 {
        if !self.line_numbers {
            return 0.0;
        }
        let baris = self.edit.display_text().matches('\n').count() + 1;
        let digit = baris.to_string().len().max(2);
        let contoh = "0".repeat(digit);
        let gaya = self.style.clone();
        let lebar = self.fonts.with(|m| m.measure_line(&contoh, &gaya).width);
        lebar + self.gutter_gap * 2.0
    }

    /// Re-shape when the visible text, the wrap width, or the screen resolution
    /// changes — and never otherwise.
    fn pastikan_bentuk(&mut self) {
        let tampil = self.edit.display_text().into_owned();
        let kosong = tampil.is_empty();
        let yang_dishape = if kosong {
            self.placeholder.clone()
        } else {
            tampil
        };
        let lebar = self.lebar_teks();
        let scale = self.fonts.scale_factor();
        if self.layout.is_some()
            && self.shaped == yang_dishape
            && self.shaped_scale == scale
            && self.shaped_width == lebar
            && self.showing_placeholder == kosong
        {
            return;
        }
        let gaya = &self.style;
        let hasil = self
            .fonts
            .with(|m| m.layout(&yang_dishape, gaya, TextConstraints::width(lebar)));
        self.lines = hasil.lines();
        self.layout = Some(hasil);
        self.shaped = yang_dishape;
        self.shaped_scale = scale;
        self.shaped_width = lebar;
        self.showing_placeholder = kosong;
    }

    /// The caret according to the shaping result — zeroed while the placeholder
    /// is showing.
    fn caret_teks(&self) -> Caret {
        let kosong = Caret {
            x: 0.0,
            top: 0.0,
            height: self.style.line_height_px(),
            line: 0,
            rtl: false,
        };
        if self.showing_placeholder {
            return kosong;
        }
        match &self.layout {
            Some(l) => l.caret(self.edit.display_selection().focus),
            None => kosong,
        }
    }

    /// The visual line the caret currently sits on.
    fn baris_caret(&self) -> usize {
        let c = self.caret_teks();
        self.lines
            .iter()
            .position(|l| c.top < l.top + l.height * 0.5)
            .unwrap_or(self.lines.len().saturating_sub(1))
    }

    /// Recompute caret, selection, preedit, glyphs, and the gutter.
    ///
    /// The one place coordinates are born; `paint` only draws what was computed
    /// here, because rasterizing needs `&mut` on the text engine and the paint
    /// pass does not have it.
    fn perbarui_geometri(&mut self) {
        let asal = self.asal_teks();
        let caret = self.caret_teks();
        self.caret = Rect::new(
            asal.x + caret.x,
            asal.y + caret.top,
            self.caret_width,
            caret.height,
        );

        let geser = |r: Rect| {
            Rect::new(
                r.origin.x + asal.x,
                r.origin.y + asal.y,
                r.size.width,
                r.size.height,
            )
        };

        // A selection crossing lines comes back as one rectangle per visual run
        // — `TextLayout` already splits it that way, so bidi text never
        // highlights letters that are not selected (§9.8).
        self.selection = match (&self.layout, self.showing_placeholder) {
            (Some(l), false) => l
                .selection_rects(self.edit.display_selection().range())
                .into_iter()
                .map(geser)
                .collect(),
            _ => Vec::new(),
        };

        // Preedit underline: as thick as the caret, hugging the baseline — the
        // shape every OS uses to mark "this isn't final yet" (§3.8).
        self.preedit = match (&self.layout, self.edit.preedit_range()) {
            (Some(l), Some(r)) => l
                .selection_rects(r)
                .into_iter()
                .map(|k| {
                    let g = geser(k);
                    Rect::new(
                        g.origin.x,
                        g.max_y() - self.caret_width,
                        g.size.width,
                        self.caret_width,
                    )
                })
                .collect(),
            _ => Vec::new(),
        };

        let warna = if self.showing_placeholder {
            self.colors.placeholder
        } else if self.disabled {
            // The `disabled_label` token, not the text colour dimmed by hand:
            // "dimmed" is a theme decision, not a widget decision (§2.7).
            self.colors.disabled
        } else {
            self.colors.text
        };
        self.run = match &self.layout {
            Some(l) => self.fonts.with(|m| m.rasterize(l, asal, warna)),
            None => GlyphRun::new(warna),
        };

        self.perbarui_gutter();
    }

    /// Rasterize the line numbers: one number per **source** line, on the first
    /// visual line of that paragraph — a wrapped continuation stays blank,
    /// exactly as in every editor.
    fn perbarui_gutter(&mut self) {
        self.gutter_runs.clear();
        if !self.line_numbers || self.gutter_width <= 0.0 {
            return;
        }
        let gaya = self.style.clone().single_line();
        let warna = if self.disabled {
            self.colors.disabled
        } else {
            self.colors.gutter
        };
        let kanan = self.padding.left + self.gutter_width - self.gutter_gap;
        let atas = self.padding.top;

        // One number per **source** line: a soft-wrapped continuation shares
        // its paragraph's index and gets no number of its own.
        let mut daftar: Vec<(usize, f32)> = Vec::new();
        let mut sebelumnya: Option<usize> = None;
        for l in &self.lines {
            if sebelumnya != Some(l.line) {
                daftar.push((l.line, l.top));
                sebelumnya = Some(l.line);
            }
        }
        // An empty area still shows the number 1: the gutter counts lines that
        // exist, and an empty document has one.
        if daftar.is_empty() {
            daftar.push((0, 0.0));
        }

        for (indeks, top) in daftar {
            let teks = (indeks + 1).to_string();
            let (lebar, tata) = self.fonts.with(|m| {
                let l = m.layout(&teks, &gaya, TextConstraints::UNBOUNDED);
                (l.measure().content_size.width, l)
            });
            let asal = Point::new(kanan - lebar, atas + top);
            let run = self.fonts.with(|m| m.rasterize(&tata, asal, warna));
            let kotak = Rect::new(asal.x, asal.y, lebar, self.style.line_height_px());
            self.gutter_runs.push((kotak, run));
        }
    }

    /// The byte index under the point `local` (node-local coordinates).
    fn indeks_di(&self, local: Point) -> usize {
        if self.showing_placeholder {
            return 0;
        }
        let asal = self.asal_teks();
        let titik = Point::new(local.x - asal.x, local.y - asal.y);
        self.layout.as_ref().map_or(0, |l| l.hit(titik))
    }

    // -- reacting to changes -------------------------------------------------

    /// After the text changed: re-shape, recompute geometry, tell the app.
    fn setelah_teks_berubah(&mut self, ctx: &mut EventCtx<'_>) {
        self.goal_x = None;
        self.pastikan_bentuk();
        self.perbarui_geometri();
        // `props_value` is deliberately **not** touched: it records what the
        // app last handed us, not what the user typed. That is what keeps an
        // uncontrolled area typable while a controlled one still accepts new
        // values — the same rule as `text_field`.
        //
        // The reported value **never** contains the preedit (§3.8).
        if let Some(cb) = self.on_change.clone() {
            cb.call(self.edit.text());
        }
        // Text that changed may be text that grew: the frame's height and the
        // scroll extent both follow from it, and neither can find out on its
        // own (see `AreaLink::request_relayout`).
        self.link.request_relayout();
        ctx.request_layout();
        self.minta_reveal();
        self.perbarui_ime(ctx);
    }

    /// After the caret/selection changed but the text did not.
    fn setelah_caret_berubah(&mut self, ctx: &mut EventCtx<'_>) {
        self.perbarui_geometri();
        ctx.request_paint();
        self.minta_reveal();
        self.perbarui_ime(ctx);
    }

    /// Ask the scroll view (through [`super::sync`]) to bring the caret back
    /// into view.
    fn minta_reveal(&self) {
        if !self.focused {
            return;
        }
        self.link.request_reveal();
    }

    /// Tell the shell where the IME candidate window should stand.
    fn perbarui_ime(&self, ctx: &mut EventCtx<'_>) {
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

    /// Replace the contents at the request of **assistive technology** (voice
    /// dictation, autofill).
    pub(super) fn setel_nilai_bantu(&mut self, nilai: &str) -> bool {
        if !self.bisa_sunting() || self.edit.text() == nilai {
            return false;
        }
        self.edit.set_text(nilai.to_string());
        self.pastikan_bentuk();
        self.perbarui_geometri();
        self.link.request_relayout();
        if let Some(cb) = self.on_change.clone() {
            cb.call(self.edit.text());
        }
        true
    }

    // -- vertical movement ---------------------------------------------------

    /// Move the caret `arah` visual lines, keeping the **goal column**.
    ///
    /// The goal column is what makes a real editor feel right: walking down
    /// through a short line and out the other side puts the caret back where
    /// the eye expects it, instead of clinging to the short line's end.
    fn pindah_baris(&mut self, arah: i32, extend: bool) -> bool {
        let Some(l) = &self.layout else { return false };
        if self.lines.is_empty() {
            return false;
        }
        let caret = self.caret_teks();
        let x = *self.goal_x.get_or_insert(caret.x);
        let sekarang = self.baris_caret() as i32;
        let tujuan = sekarang + arah;
        let indeks = if tujuan < 0 {
            // Above the first line: the start of the document, the AppKit rule.
            0
        } else if tujuan as usize >= self.lines.len() {
            self.edit.text().len()
        } else {
            let baris = self.lines[tujuan as usize];
            l.hit(Point::new(x, baris.top + baris.height * 0.5))
        };
        let berubah = self.edit.place_caret(indeks, extend);
        // `place_caret` clears nothing of ours: the goal column survives the
        // whole run of ↑/↓ presses, and only a horizontal move resets it.
        berubah
    }

    /// Move the caret one viewport up or down (PageUp/PageDown).
    fn pindah_halaman(&mut self, arah: f32, extend: bool) -> bool {
        let Some(l) = &self.layout else { return false };
        let caret = self.caret_teks();
        let x = *self.goal_x.get_or_insert(caret.x);
        let tinggi = self.link.viewport().height.max(caret.height * 3.0);
        let y = caret.top + caret.height * 0.5 + arah * tinggi;
        let indeks = l.hit(Point::new(x, y));
        self.edit.place_caret(indeks, extend)
    }

    /// The start/end of the **visual** line the caret is on — what Home and End
    /// mean once soft wrapping is in play.
    fn ujung_baris_visual(&self, akhir: bool) -> usize {
        match self.lines.get(self.baris_caret()) {
            Some(l) => {
                if akhir {
                    l.end
                } else {
                    l.start
                }
            }
            None => {
                if akhir {
                    self.edit.text().len()
                } else {
                    0
                }
            }
        }
    }

    // -- keyboard ------------------------------------------------------------

    fn tombol(&mut self, ctx: &mut EventCtx<'_>, k: &KeyEvent) {
        // **While an IME is composing, the normal key path is held back**
        // (§3.8): the letters being picked in the candidate window must not
        // also land as ordinary keystrokes.
        if self.edit.is_composing() {
            ctx.handled();
            return;
        }

        let m = k.modifiers;
        let shift = m.contains(Modifiers::SHIFT);
        let cmd = m.contains(Modifiers::COMMAND);
        let ctrl = m.contains(Modifiers::CONTROL);
        let dokumen = cmd || ctrl;
        let sebelum = self.edit.text().to_string();
        let seleksi_sebelum = self.edit.selection();
        let mut tertangani = true;
        // Only a vertical move keeps the goal column; everything else drops it.
        let mut jaga_goal = false;

        match &k.code {
            KeyCode::Named(NamedKey::ArrowUp) if dokumen => {
                self.edit.place_caret(0, shift);
            }
            KeyCode::Named(NamedKey::ArrowDown) if dokumen => {
                let akhir = self.edit.text().len();
                self.edit.place_caret(akhir, shift);
            }
            KeyCode::Named(NamedKey::ArrowUp) => {
                self.pindah_baris(-1, shift);
                jaga_goal = true;
            }
            KeyCode::Named(NamedKey::ArrowDown) => {
                self.pindah_baris(1, shift);
                jaga_goal = true;
            }
            KeyCode::Named(NamedKey::PageUp) => {
                self.pindah_halaman(-1.0, shift);
                jaga_goal = true;
            }
            KeyCode::Named(NamedKey::PageDown) => {
                self.pindah_halaman(1.0, shift);
                jaga_goal = true;
            }
            // ⌘/Ctrl+Home/End cross the whole document; bare Home/End stay on
            // the **visual** line, which is what soft wrapping makes them mean.
            KeyCode::Named(NamedKey::Home) if dokumen => {
                self.edit.place_caret(0, shift);
            }
            KeyCode::Named(NamedKey::End) if dokumen => {
                let akhir = self.edit.text().len();
                self.edit.place_caret(akhir, shift);
            }
            KeyCode::Named(NamedKey::Home) => {
                let tujuan = self.ujung_baris_visual(false);
                self.edit.place_caret(tujuan, shift);
            }
            KeyCode::Named(NamedKey::End) => {
                let tujuan = self.ujung_baris_visual(true);
                self.edit.place_caret(tujuan, shift);
            }
            KeyCode::Named(NamedKey::Enter) => {
                if dokumen {
                    // ⌘Enter submits — the habit of every chat box and comment
                    // field, and the only way `on_submit` can exist at all when
                    // plain Enter has to insert a line.
                    match self.on_submit.clone() {
                        Some(cb) => cb.call(self.edit.text()),
                        None => tertangani = false,
                    }
                } else if self.bisa_sunting() {
                    self.edit.insert("\n");
                } else {
                    tertangani = false;
                }
            }
            KeyCode::Named(NamedKey::Tab) => {
                // **The default moves focus, on purpose.** A Tab swallowed by a
                // text box is a keyboard trap: a keyboard-only user who lands
                // in it can never get out again (`KOMPONEN.md` DoD, §3.8).
                // Indentation is opt-in, and ⇧Tab always walks focus backwards
                // so there is an escape hatch even then.
                if self.tab == TabBehavior::InsertTab && self.bisa_sunting() && !shift {
                    self.edit.insert("\t");
                } else {
                    tertangani = false;
                }
            }
            _ => {
                // Everything else is the **shared** keymap: the very same
                // function `text_field` runs (`crate::editing`).
                let caps = EditCaps::new(self.bisa_sunting());
                tertangani = editing::handle_key(&mut self.edit, k, caps);
            }
        }

        if !tertangani {
            return;
        }
        if !jaga_goal {
            self.goal_x = None;
        }
        ctx.handled();
        if self.edit.text() != sebelum {
            self.setelah_teks_berubah(ctx);
            // A vertical move never changes the text, so this cannot undo the
            // goal column we just kept.
        } else if self.edit.selection() != seleksi_sebelum || matches!(&k.code, KeyCode::Named(_)) {
            self.setelah_caret_berubah(ctx);
        }
    }

    // -- IME -----------------------------------------------------------------

    fn ime(&mut self, ctx: &mut EventCtx<'_>, e: &ImeEvent) {
        if !self.bisa_sunting() {
            return;
        }
        let sebelum = self.edit.text().to_string();
        let berubah = match e {
            ImeEvent::Enabled => false,
            ImeEvent::Preedit { text, cursor } => self.edit.set_preedit(text, *cursor),
            ImeEvent::Commit(teks) => self.edit.commit(teks),
            ImeEvent::Disabled => self.edit.clear_preedit(),
        };
        if !berubah {
            return;
        }
        ctx.handled();
        if self.edit.text() != sebelum {
            self.setelah_teks_berubah(ctx);
        } else {
            // A changed preedit means the **visible** text changed, so
            // re-shaping is still needed — but the app is told nothing.
            self.pastikan_bentuk();
            self.perbarui_geometri();
            self.link.request_relayout();
            ctx.request_layout();
            self.minta_reveal();
            self.perbarui_ime(ctx);
        }
    }

    // -- pointer -------------------------------------------------------------

    fn penunjuk(&mut self, ctx: &mut EventCtx<'_>, p: &PointerEvent) {
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
                let indeks = self.indeks_di(ctx.local());
                match p.click_count {
                    // Double-click = one word. Triple-click = the **paragraph**,
                    // not the whole document: in a multi-line editor "select
                    // everything" is ⌘A, and a triple click that swallowed a
                    // long note would be a trap.
                    2 => {
                        self.edit.select_word_at(indeks);
                    }
                    n if n >= 3 => {
                        let teks = self.edit.text();
                        let mulai = teks[..indeks.min(teks.len())]
                            .rfind('\n')
                            .map_or(0, |i| i + 1);
                        let akhir = teks[indeks.min(teks.len())..]
                            .find('\n')
                            .map_or(teks.len(), |i| indeks + i);
                        self.edit.set_selection(Selection::new(mulai, akhir));
                    }
                    _ => {
                        self.edit
                            .place_caret(indeks, p.modifiers.contains(Modifiers::SHIFT));
                    }
                }
                self.setelah_caret_berubah(ctx);
                ctx.handled();
            }
            PointerPhase::Move if self.dragging => {
                // Drag-select: the pointer is already captured, so dragging
                // outside the area still extends the selection — across lines
                // included.
                let indeks = self.indeks_di(ctx.local());
                if self.edit.place_caret(indeks, true) {
                    self.setelah_caret_berubah(ctx);
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

impl RenderNode for TextAreaBody {
    fn type_name(&self) -> &'static str {
        "TextAreaBody"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // The scroll view above hands down a **tight width and an unbounded
        // height**: the width is what soft wrapping breaks against, the height
        // is ours to decide.
        let lebar = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            constraints.min_width.max(1.0)
        };
        self.size = Size::new(lebar, self.size.height);
        self.gutter_width = self.hitung_gutter();
        self.pastikan_bentuk();

        let isi = self
            .layout
            .as_ref()
            .map_or(self.style.line_height_px(), |l| {
                l.measure().content_size.height
            });
        let alami = isi + self.padding.vertical();
        // The frame needs the **natural** height for auto-grow, so publish that
        // one — not the stretched height below.
        self.link.set_content(alami);

        // Fill the viewport when the text is shorter than it: a click in the
        // empty space below the last line has to land in the document, the way
        // it does in every text editor.
        let tinggi = alami.max(self.link.viewport().height);
        self.size = constraints.constrain(Size::new(lebar, tinggi));
        self.perbarui_geometri();
        self.size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        // The gutter first: it is the background of its own column.
        if self.line_numbers && self.gutter_width > 0.0 {
            let lebar = self.padding.left + self.gutter_width;
            if self.colors.gutter_background.a > 0.0 {
                ctx.quad(
                    Quad::new(Rect::new(0.0, 0.0, lebar, self.size.height))
                        .background(self.colors.gutter_background),
                );
            }
            if self.colors.gutter_separator.a > 0.0 {
                ctx.quad(
                    Quad::new(Rect::new(lebar - 1.0, 0.0, 1.0, self.size.height))
                        .background(self.colors.gutter_separator),
                );
            }
        }

        // The selection highlight goes **under** the text; its alpha follows
        // focus so an unfocused area does not still look "live".
        if !self.selection.is_empty() && !self.disabled {
            let hidup = if self.focused { 1.0 } else { 0.35 };
            let warna = self
                .colors
                .selection
                .with_alpha(self.colors.selection.a * hidup);
            for r in &self.selection {
                ctx.quad(Quad::new(*r).background(warna));
            }
        }

        if !self.run.is_empty() {
            ctx.glyph_run(self.run.clone());
        }

        // Line numbers are drawn only where they are actually visible: the
        // document may be thousands of lines long, the window is not.
        for (kotak, run) in &self.gutter_runs {
            if !run.is_empty() && ctx.is_visible(*kotak) {
                ctx.glyph_run(run.clone());
            }
        }

        // Preedit underline: on top of the text, because it marks that text.
        for r in &self.preedit {
            ctx.quad(Quad::new(*r).background(self.colors.text));
        }

        if self.focused && !self.disabled && self.edit.display_selection().is_collapsed() {
            ctx.quad(Quad::new(self.caret).background(self.colors.caret));
        }
    }

    fn access(&self, node: &mut AccessNode) {
        // The **multiline** role, not `TextInput`: it is what tells a screen
        // reader that ↑/↓ move between lines here and that Enter does not
        // submit (`KOMPONEN.md` DoD).
        node.role = AccessRole::MultilineTextInput;
        node.label.clone_from(&self.label);
        // The value read out is the committed one — never a half-formed
        // preedit.
        node.value = Some(self.edit.text().to_string());
        // Caret and selection, in characters: without them a screen reader can
        // only re-read the whole document after every keystroke instead of
        // following the caret from line to line (§3.8).
        let s = self.edit.selection();
        node.text_selection = Some(AccessTextSelection::from_bytes(
            self.edit.text(),
            s.anchor,
            s.focus,
        ));
        node.disabled = self.disabled;
        if !self.disabled {
            node.actions |= AccessActions::CLICK | AccessActions::FOCUS;
            if !self.read_only {
                // Voice dictation and autofill belong to assistive tech.
                node.actions |= AccessActions::SET_VALUE;
            }
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rect
    }

    fn hit_behavior(&self) -> HitBehavior {
        // A disabled area still absorbs: a click on it must not fall through to
        // whatever is behind it.
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
            Event::Pointer(p) => self.penunjuk(ctx, p),
            Event::Key(k) if k.is_pressed() => self.tombol(ctx, k),
            Event::Ime(e) => self.ime(ctx, e),
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                self.link.set_focused(self.focused);
                if !self.focused {
                    self.dragging = false;
                    // A composition left dangling when focus leaves is thrown
                    // away: the IME will never send its commit now.
                    self.edit.clear_preedit();
                    self.pastikan_bentuk();
                } else {
                    self.minta_reveal();
                }
                self.perbarui_geometri();
                ctx.request_paint();
                ctx.request_animation();
                if self.focused {
                    self.perbarui_ime(ctx);
                } else {
                    ctx.disable_ime();
                }
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for TextAreaBody {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextAreaBody")
            .field("text", &self.edit.text())
            .field("selection", &self.edit.selection())
            .field("lines", &self.lines.len())
            .field("focused", &self.focused)
            .field("composing", &self.edit.is_composing())
            .finish()
    }
}

/// The pieces of the body that props may replace on a rebuild.
///
/// Everything else the props touch is a plain `pub(super)` field: this module
/// is the only one that may reach in, and two methods is fewer moving parts
/// than twenty setters.
impl TextAreaBody {
    /// Replace the colours and force the glyph run to be produced again — the
    /// text colour is baked into it.
    pub(super) fn set_colors(&mut self, colors: BodyColors) {
        self.colors = colors;
        self.invalidate_shape();
    }

    /// Throw away the cached shaping result.
    pub(super) fn invalidate_shape(&mut self) {
        self.shaped_scale = f32::NAN;
    }
}
