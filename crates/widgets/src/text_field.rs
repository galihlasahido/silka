//! `text_field()` — **the hardest component in the whole catalogue**
//! (`KOMPONEN.md` Tier 2), and precisely why it was built first: it forces the
//! text, IME, and accessibility stacks to mature sooner (REKOMENDASI §5 failure
//! mode #1 and #2).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_theme::{Appearance, Theme};
//! # use silka_widgets::{text_field_in, Fonts};
//! # let rt = Runtime::new();
//! # let nama = rt.signal(String::new());
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! text_field_in(&fonts, &t, nama.get())
//!     .placeholder("Full name")
//!     .label("Name")
//!     .on_change(move |s| nama.set(s.to_string()));
//! ```
//!
//! ## What makes it correct, not merely correct-looking
//!
//! | Piece | Where it lives | Why there |
//! |---|---|---|
//! | Per-grapheme caret/selection, undo, preedit | [`silka_text::edit`] | Unicode rules (UAX #29), not drawing rules — and testable without pixels |
//! | Caret geometry, hit-testing, selection rects | [`silka_text::TextLayout`] | Only the shaping result knows where a letter stands |
//! | Focus, pointer capture, multi-click | [`silka_core::input`] | Already the framework's input contract |
//! | Color/spacing/corner tokens | [`silka_theme`] | Written once, two presets (§2.7) |
//!
//! This node bolts the four together, and **does not add a single line of
//! Unicode rules of its own**.
//!
//! ## Definition of Done (`KOMPONEN.md`), satisfied
//!
//! - **Both presets** through semantic tokens; not one color literal anywhere
//!   in this file, and corner shape is a parameter ([`Corners`]), not a
//!   constant (§2.7, §3.6).
//! - **Every interactive state transitions on a spring**: hover and focus are
//!   [`SpringValue`]s that can be retargeted mid-flight — the focus ring never
//!   snaps on out of nowhere (§3.5).
//! - **Full keyboard**: ←/→ (per grapheme), ⌥←/⌥→ (per word), ⌘←/⌘→, Home/End,
//!   Shift to extend, Backspace/Delete (+⌥ per word), ⌘A, ⌘Z/⇧⌘Z, Enter for
//!   `on_submit`. Tab is **not** captured: it belongs to focus navigation.
//! - **AccessKit node** with role [`AccessRole::TextInput`], a name, a
//!   **value**, and the `SET_VALUE` action (voice dictation) — disabled state
//!   included.
//! - **Dark mode** follows the tokens; the drawn height is
//!   [`ControlToken::Md`] while the **hit target
//!   stays ≥ 44pt** ([`MIN_HIT_TARGET`]) even though the line itself is far
//!   shorter;
//!   **reduced-motion** is honored because every motion goes through [`Tick`].
//! - **IME preedit is rendered inline** with an underline, and while
//!   composition is running the normal key path is held back (§3.8). For that
//!   whole time `on_change` is **not** called: the app never receives
//!   half-formed letters.
//!
//! ## Technical debt we know about
//!
//! - **Clipboard** (⌘C/⌘X/⌘V) is not wired up yet: `arboard` lives in
//!   `silka-platform` (INTEGRASI-NATIVE §4) and this crate must not depend on
//!   it. Those shortcuts are deliberately **left to bubble** upward rather than
//!   swallowed in silence, so the shell can serve them later without a single
//!   line changing here.
//! - **The caret does not blink.** Blinking needs a timer that ticks forever,
//!   and that collides with the "render only when dirty" promise (§3.5) until
//!   the scheduler grows a proper timer path.
//! - Single line only; multi-line + soft wrap is `text_area`, which uses the
//!   already-existing [`silka_text::TextEdit::multiline`].

use silka_core::access::{
    AccessAction, AccessActionRequest, AccessActions, AccessNode, AccessRole, AccessTextSelection,
};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, ImeEvent, KeyCode,
    KeyEvent, Modifiers, NamedKey, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{
    BoxConstraints, Decoration, FocusRing, LayoutCtx, NodeId, PaintCtx, RenderNode, RenderTree,
    TextDirection,
};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, Corners, GlyphRun, Insets, Point, Quad, Rect, Size};
use silka_text::{Caret, Movement, TextConstraints, TextEdit, TextLayout, TextStyle};
use silka_theme::{ControlToken, SpaceToken, Theme};

// Referenced by the doc links below rather than by code: the height now comes
// from `ControlToken::Md`, and `hit_target_of` is what applies the 44pt floor.
#[allow(unused_imports)]
use crate::button::MIN_HIT_TARGET;
use crate::editing::{self, EditCaps};
use crate::fonts::Fonts;

/// The callback that carries the field's contents.
///
/// It lives in [`crate::editing`] because `text_area` (and later `combo_box`)
/// hand back exactly the same thing; re-exported here so the name stays where
/// callers first meet it.
pub use crate::editing::TextCallback;

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// What ↑ and ↓ mean inside a single-line field.
///
/// A one-line field has nowhere to move the caret vertically, so the keys are
/// free — and two different controls want two different things from them. This
/// is that choice, made explicit rather than hard-wired.
///
/// ```
/// use silka_widgets::ArrowKeys;
///
/// // The AppKit habit, and what a field standing on its own should do.
/// assert_eq!(ArrowKeys::default(), ArrowKeys::Caret);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ArrowKeys {
    /// Move the caret to the start or the end of the content — the AppKit
    /// habit, and the default.
    #[default]
    Caret,
    /// Leave them alone, so they **bubble** to an enclosing control.
    ///
    /// What a [`mod@crate::combo_box`] needs: inside one, ↓ opens the suggestion
    /// list and walks it, and a field that swallowed the key first would make
    /// that impossible.
    Bubble,
}

/// The render node behind a text field.
///
/// It draws its own text (rather than through a [`crate::text()`] child) because
/// the caret, the selection, and the preedit must share **one** shaping result
/// with the glyphs on screen. Two layout sources for one line of text = a caret
/// off by half a pixel, and that shows.
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{text_field_in, Fonts, TextFieldBox};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let mut tree = RenderTree::new();
/// reconcile(
///     &mut tree,
///     text_field_in(&fonts, &theme, "Hello").placeholder("Type here"),
/// );
/// tree.layout(BoxConstraints::tight(Size::new(240.0, 44.0)));
///
/// let id = tree.children(tree.root())[0];
/// let node = tree.node_ref::<TextFieldBox>(id).expect("a text field node");
///
/// assert_eq!(node.text(), "Hello");
/// assert!(!node.shows_placeholder()); // there is content, so no placeholder
///
/// // The caret is real geometry, not a guess — it comes from the same
/// // `TextLayout` the glyphs came from.
/// assert!(node.caret_rect().size.height > 0.0);
///
/// // Nothing selected and no composition in flight, so the normal key path
/// // is live and the application is receiving whole characters.
/// assert!(node.selection_rects().is_empty());
/// assert!(!node.is_composing());
/// assert!(node.preedit_rects().is_empty());
/// ```
pub struct TextFieldBox {
    // -- configuration (tokens already resolved one level up) --
    fonts: Fonts,
    style: TextStyle,
    placeholder: String,
    padding: Insets,
    corners: Corners,
    min_height: f32,
    caret_width: f32,
    label: Option<String>,
    disabled: bool,
    read_only: bool,
    arrows: ArrowKeys,

    color: Color,
    placeholder_color: Color,
    disabled_color: Color,
    selection_color: Color,
    caret_color: Color,
    background: Color,
    background_hover: Color,
    background_focus: Color,
    border_width: f32,
    border_color: Color,
    border_focus_color: Color,
    focus_ring: Option<FocusRing>,

    on_change: Option<TextCallback>,
    on_submit: Option<TextCallback>,

    // -- state owned by the node (diffing never overwrites it) --
    edit: TextEdit,
    /// The value that last **came from props**, and only from there: typing
    /// never changes it. The yardstick for telling whether the app really did
    /// replace the contents (see [`TextFieldProps::update`]).
    props_value: String,
    hovered: bool,
    focused: bool,
    dragging: bool,
    scroll: f32,
    size: Size,
    /// The reading direction from the last layout (§9.8, `AUDIT.md` P-6).
    ///
    /// The field's box is mirrored by the layout system, but where the text sits
    /// **inside** that box is computed here, by hand — so this is the one value
    /// that has to be carried across from `LayoutCtx`.
    direction: TextDirection,

    // -- animation (§3.5) --
    hover_t: SpringValue<f32>,
    focus_t: SpringValue<f32>,

    // -- derived: always a consequence of the above --
    layout: Option<TextLayout>,
    shaped: String,
    shaped_scale: f32,
    showing_placeholder: bool,
    run: GlyphRun,
    caret: Rect,
    selection: Vec<Rect>,
    preedit: Vec<Rect>,
}

impl TextFieldBox {
    /// The field's **committed** contents — without the preedit being composed.
    pub fn text(&self) -> &str {
        self.edit.text()
    }

    /// The current selection (byte indices).
    pub fn selection(&self) -> silka_text::Selection {
        self.edit.selection()
    }

    /// True while an IME is composing in this field.
    pub fn is_composing(&self) -> bool {
        self.edit.is_composing()
    }

    /// The caret rect in node-local coordinates (from the last layout).
    pub fn caret_rect(&self) -> Rect {
        self.caret
    }

    /// The selection highlight rects, in node-local coordinates.
    pub fn selection_rects(&self) -> &[Rect] {
        &self.selection
    }

    /// The preedit underline rects, in node-local coordinates.
    pub fn preedit_rects(&self) -> &[Rect] {
        &self.preedit
    }

    /// True when what is on screen is the placeholder (an empty field).
    pub fn shows_placeholder(&self) -> bool {
        self.showing_placeholder
    }

    /// Horizontal offset of the contents, in logical points.
    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    /// True while any of its transitions is still moving.
    pub fn is_animating(&self) -> bool {
        self.hover_t.is_animating() || self.focus_t.is_animating()
    }

    /// Advance the transitions by one frame; true if anything changed.
    ///
    /// Called by [`crate::advance`], the one place where every spring in a tree
    /// is advanced together.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        if !self.is_animating() {
            return false;
        }
        let sebelum = (self.hover_t.position(), self.focus_t.position());
        tick.advance(&mut self.hover_t);
        tick.advance(&mut self.focus_t);
        (self.hover_t.position(), self.focus_t.position()) != sebelum
    }

    /// Finish every transition instantly (tests and snapshots).
    pub fn settle(&mut self) {
        self.hover_t.settle();
        self.focus_t.settle();
    }

    // -- geometry -----------------------------------------------------------

    /// The content box: the node's box minus padding.
    fn kotak_isi(&self) -> Rect {
        Rect::from_origin_size(Point::ZERO, self.size).deflate(self.padding)
    }

    /// Top edge of the text: a single line is always **vertically centered**,
    /// because a 44pt hit target is nearly always taller than its line (HIG).
    fn atas_teks(&self) -> f32 {
        let baris = self.style.line_height_px();
        ((self.size.height - baris) / 2.0).max(self.padding.top)
    }

    /// Re-shape when the visible text or the screen resolution changes.
    ///
    /// Two valid reasons, and only two — the same rule as [`crate::text`]: the
    /// contents changed, or the scale factor changed (glyph bitmaps are tied to
    /// resolution).
    fn pastikan_bentuk(&mut self) {
        let tampil = self.edit.display_text().into_owned();
        let kosong = tampil.is_empty();
        let yang_dishape = if kosong {
            self.placeholder.clone()
        } else {
            tampil
        };
        let scale = self.fonts.scale_factor();
        if self.layout.is_some()
            && self.shaped == yang_dishape
            && self.shaped_scale == scale
            && self.showing_placeholder == kosong
        {
            return;
        }
        let gaya = &self.style;
        let hasil = self
            .fonts
            .with(|m| m.layout(&yang_dishape, gaya, TextConstraints::UNBOUNDED));
        self.layout = Some(hasil);
        self.shaped = yang_dishape;
        self.shaped_scale = scale;
        self.showing_placeholder = kosong;
    }

    /// The caret according to the shaping result — zeroed when the placeholder
    /// is showing.
    fn caret_teks(&self) -> Caret {
        let baris = self.style.line_height_px();
        let kosong = Caret {
            x: 0.0,
            top: 0.0,
            height: baris,
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

    /// Recompute scroll, caret, selection, preedit, and the glyph run.
    ///
    /// The one place coordinates are born; `paint` only draws what was already
    /// computed here, because rasterization needs `&mut` on the text engine and
    /// the paint pass does not have it.
    fn perbarui_geometri(&mut self) {
        let isi = self.kotak_isi();
        let atas = self.atas_teks();
        let caret = self.caret_teks();

        // Horizontal scroll: the caret is always visible, and the contents are
        // never shifted further than they need to be.
        let lebar_isi = self
            .layout
            .as_ref()
            .map_or(0.0, |l| l.measure().content_size.width);
        let maksimum = (lebar_isi - isi.size.width).max(0.0);
        if !self.focused {
            self.scroll = 0.0;
        } else {
            if caret.x - self.scroll < 0.0 {
                self.scroll = caret.x;
            }
            let batas_kanan = isi.size.width - self.caret_width;
            if caret.x - self.scroll > batas_kanan {
                self.scroll = caret.x - batas_kanan;
            }
            self.scroll = self.scroll.clamp(0.0, maksimum);
        }

        // Where the text sits when it is **narrower** than the field: against
        // the leading edge, which is the right-hand one in an RTL document. Once
        // the content overflows there is no slack left and the caret-following
        // scroll above is what decides, exactly as before (§9.8).
        let sisip = if self.direction.is_rtl() {
            (isi.size.width - lebar_isi).max(0.0)
        } else {
            0.0
        };
        let asal = Point::new(isi.origin.x + sisip - self.scroll, atas);
        self.caret = Rect::new(
            asal.x + caret.x,
            atas + caret.top,
            self.caret_width,
            caret.height,
        );

        let geser = |r: Rect| {
            Rect::new(
                r.origin.x + asal.x,
                r.origin.y + atas,
                r.size.width,
                r.size.height,
            )
        };
        let pandang = Rect::from_origin_size(
            Point::new(isi.origin.x, 0.0),
            Size::new(isi.size.width, self.size.height),
        );

        self.selection = match (&self.layout, self.showing_placeholder) {
            (Some(l), false) => l
                .selection_rects(self.edit.display_selection().range())
                .into_iter()
                .filter_map(|r| geser(r).intersect(pandang))
                .collect(),
            _ => Vec::new(),
        };

        // Preedit underline: as thick as the caret, hugging the baseline — the
        // shape every OS uses to mark "this isn't final yet" (§3.8).
        self.preedit = match (&self.layout, self.edit.preedit_range()) {
            (Some(l), Some(r)) => l
                .selection_rects(r)
                .into_iter()
                .filter_map(|k| {
                    let g = geser(k);
                    let garis = Rect::new(
                        g.origin.x,
                        g.max_y() - self.caret_width,
                        g.size.width,
                        self.caret_width,
                    );
                    garis.intersect(pandang)
                })
                .collect(),
            _ => Vec::new(),
        };

        let warna = if self.showing_placeholder {
            self.placeholder_color
        } else if self.disabled {
            // The `disabled_label` token, not the text color dimmed by hand:
            // "dimmed" is a theme decision, not a widget decision (§2.7).
            self.disabled_color
        } else {
            self.color
        };
        self.run = match &self.layout {
            Some(l) => {
                let mut run = self.fonts.with(|m| m.rasterize(l, asal, warna));
                // Contents longer than the field are clipped at the content
                // edge, rather than running into the border.
                run.clip = Some(pandang);
                run
            }
            None => GlyphRun::new(warna),
        };
    }

    /// The byte index under point `local` (node-local coordinates).
    fn indeks_di(&self, local: Point) -> usize {
        if self.showing_placeholder {
            return 0;
        }
        let isi = self.kotak_isi();
        let titik = Point::new(
            local.x - isi.origin.x + self.scroll,
            // Single line: however high the click lands, it's the same line.
            self.style.line_height_px() / 2.0,
        );
        self.layout.as_ref().map_or(0, |l| l.hit(titik))
    }

    /// The decoration for the current state — the result of **spring
    /// interpolation**, not a jump between three colors.
    fn dekorasi_aktif(&self) -> Decoration {
        let hover = self.hover_t.position().clamp(0.0, 1.0);
        let fokus = self.focus_t.position().clamp(0.0, 1.0);
        let latar = if self.disabled {
            self.background
        } else {
            self.background
                .lerp(self.background_hover, hover)
                .lerp(self.background_focus, fokus)
        };
        let border = if self.disabled {
            self.border_color
        } else {
            self.border_color.lerp(self.border_focus_color, fokus)
        };
        Decoration {
            background: latar,
            corners: self.corners,
            border_width: self.border_width,
            border_color: border,
            shadows: silka_paint::ShadowPair::NONE,
        }
    }

    // -- reacting to changes -------------------------------------------------

    /// After the text changed: re-shape, recompute geometry, tell the app.
    fn setelah_teks_berubah(&mut self, ctx: &mut EventCtx<'_>) {
        self.pastikan_bentuk();
        self.perbarui_geometri();
        // `props_value` is deliberately **not** touched here: it records what
        // the app last handed us, not what the user typed. That is exactly what
        // keeps a field without `on_change` typable (its props never change, so
        // they never overwrite), while a controlled field still accepts new
        // values from the app.
        //
        // The reported value **never** contains the preedit: the app only ever
        // sees finished text (§3.8).
        if let Some(cb) = self.on_change.clone() {
            cb.call(self.edit.text());
        }
        ctx.request_layout();
        self.perbarui_ime(ctx);
    }

    /// After the caret/selection changed but the text did not.
    fn setelah_caret_berubah(&mut self, ctx: &mut EventCtx<'_>) {
        self.perbarui_geometri();
        ctx.request_paint();
        self.perbarui_ime(ctx);
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

    /// Editable at all?
    fn bisa_sunting(&self) -> bool {
        !self.disabled && !self.read_only
    }

    /// Replace the contents at the request of **assistive technology** (voice
    /// dictation, autofill) — the [`AccessAction::SetValue`] entry point.
    ///
    /// Deliberately separate from the props path: that one is the app setting a
    /// value, this one is the *user* typing by other means — so it must call
    /// `on_change`, exactly like a keystroke does.
    fn setel_nilai_bantu(&mut self, nilai: &str) -> bool {
        if !self.bisa_sunting() || self.edit.text() == nilai {
            return false;
        }
        self.edit.set_text(nilai.to_string());
        self.pastikan_bentuk();
        self.perbarui_geometri();
        if let Some(cb) = self.on_change.clone() {
            cb.call(self.edit.text());
        }
        true
    }

    // -- keyboard -----------------------------------------------------------

    fn tombol(&mut self, ctx: &mut EventCtx<'_>, k: &KeyEvent) {
        // **While an IME is composing, the normal key path is held back**
        // (§3.8): the letters being picked in the candidate window must not
        // also land as ordinary keystrokes.
        if self.edit.is_composing() {
            ctx.handled();
            return;
        }

        let shift = k.modifiers.contains(Modifiers::SHIFT);
        let sebelum = self.edit.text().to_string();
        let seleksi_sebelum = self.edit.selection();

        // The **shared** half of the keymap first (`crate::editing`): arrows,
        // words, Home/End, delete, undo/redo, plain typing. `text_area` runs
        // the very same function, which is what keeps the two widgets from
        // drifting apart key by key.
        let caps = EditCaps::new(self.bisa_sunting());
        let mut tertangani = editing::handle_key(&mut self.edit, k, caps);

        if !tertangani {
            match &k.code {
                // Single line: up/down = ends of the line, the AppKit habit.
                // This is exactly the key `text_area` reads differently, and
                // the reason it is not in the shared half. A field inside a
                // combo box gives them up entirely ([`ArrowKeys::Bubble`]) so
                // that the suggestion list can have them.
                KeyCode::Named(NamedKey::ArrowUp) if self.arrows == ArrowKeys::Caret => {
                    self.edit.move_caret(Movement::LineStart, shift);
                    tertangani = true;
                }
                KeyCode::Named(NamedKey::ArrowDown) if self.arrows == ArrowKeys::Caret => {
                    self.edit.move_caret(Movement::LineEnd, shift);
                    tertangani = true;
                }
                KeyCode::Named(NamedKey::Enter) => {
                    if let Some(cb) = self.on_submit.clone() {
                        cb.call(self.edit.text());
                        tertangani = true;
                    }
                }
                // Esc and Tab are deliberately let through: the first belongs
                // to overlays, the second to focus navigation.
                _ => {}
            }
        }

        if !tertangani {
            return;
        }
        ctx.handled();
        if self.edit.text() != sebelum {
            self.setelah_teks_berubah(ctx);
        } else if self.edit.selection() != seleksi_sebelum || matches!(&k.code, KeyCode::Named(_)) {
            self.setelah_caret_berubah(ctx);
        }
    }

    // -- IME ----------------------------------------------------------------

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
            ctx.request_layout();
            self.perbarui_ime(ctx);
        }
    }

    // -- pointer -------------------------------------------------------------

    fn penunjuk(&mut self, ctx: &mut EventCtx<'_>, p: &silka_core::input::PointerEvent) {
        match p.phase {
            PointerPhase::Enter => {
                if !self.hovered {
                    self.hovered = true;
                    self.hover_t.set_target(1.0);
                    ctx.request_paint();
                    ctx.request_animation();
                }
            }
            PointerPhase::Leave => {
                if self.hovered {
                    self.hovered = false;
                    self.hover_t.set_target(0.0);
                    ctx.request_paint();
                    ctx.request_animation();
                }
            }
            PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                ctx.request_focus();
                ctx.capture_pointer();
                self.dragging = true;
                let indeks = self.indeks_di(ctx.local());
                match p.click_count {
                    // Double-click = one word, triple-click = everything — the
                    // timing thresholds belong to the framework
                    // (`ClickConfig`), so they match across all three OSes.
                    2 => {
                        self.edit.select_word_at(indeks);
                    }
                    n if n >= 3 => {
                        self.edit.select_all();
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
                // outside the field still extends the selection.
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

impl RenderNode for TextFieldBox {
    fn type_name(&self) -> &'static str {
        "TextField"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.direction = ctx.direction();
        self.pastikan_bentuk();
        let baris = self.style.line_height_px();
        let tinggi = (baris + self.padding.vertical()).max(self.min_height);
        let lebar = if constraints.max_width.is_finite() {
            constraints.max_width
        } else {
            let isi = self
                .layout
                .as_ref()
                .map_or(0.0, |l| l.measure().content_size.width);
            isi + self.padding.horizontal() + self.caret_width
        };
        self.size = constraints.constrain(Size::new(lebar, tinggi));
        self.perbarui_geometri();
        self.size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.dekorasi_aktif());

        // The focus ring **grows** with the spring: 0 at rest, full when
        // focused. Drawn outside the box so it never covers the contents (the
        // AppKit habit, same as `Interactive`).
        let fokus = self.focus_t.position().clamp(0.0, 1.0);
        if let Some(ring) = self.focus_ring.filter(|r| fokus > 0.0 && r.width > 0.0) {
            let tebal = ring.width * fokus;
            let kotak = ctx.local_bounds().deflate(Insets::all(-tebal));
            let corners = silka_paint::Corners::new(
                silka_paint::CornerRadii::all(self.corners.radii.max() + tebal),
                self.corners.style,
            );
            ctx.quad(
                Quad::new(kotak)
                    .corners(corners)
                    .border(tebal, ring.color.with_alpha(ring.color.a * fokus)),
            );
        }

        // The selection highlight goes **under** the text; its alpha follows
        // focus so an unfocused field doesn't still look "live".
        if !self.selection.is_empty() && !self.disabled {
            let warna = self
                .selection_color
                .with_alpha(self.selection_color.a * (0.35 + 0.65 * fokus));
            for r in &self.selection {
                ctx.quad(Quad::new(*r).background(warna));
            }
        }

        if !self.run.is_empty() {
            ctx.glyph_run(self.run.clone());
        }

        // Preedit underline: on top of the text, because it marks that text.
        for r in &self.preedit {
            ctx.quad(Quad::new(*r).background(self.color));
        }

        if self.focused && !self.disabled && self.edit.display_selection().is_collapsed() {
            ctx.quad(Quad::new(self.caret).background(self.caret_color));
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::TextInput;
        node.label.clone_from(&self.label);
        // The value read out is the committed one — not a preedit that is
        // still half-formed.
        node.value = Some(self.edit.text().to_string());
        // Where the caret stands and how far the selection reaches — in
        // characters, the unit assistive technology counts in. Without this a
        // screen reader can only re-read the whole value after every keystroke
        // instead of following the caret (§3.8).
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
        HitShape::Rounded(self.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        // A disabled field still absorbs: a click on it must not fall through
        // to whatever is behind it.
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
                self.focus_t
                    .set_target(if self.focused { 1.0 } else { 0.0 });
                if !self.focused {
                    self.dragging = false;
                    // A composition left dangling when focus leaves is thrown
                    // away: the IME will never send its commit now.
                    self.edit.clear_preedit();
                    self.pastikan_bentuk();
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

impl core::fmt::Debug for TextFieldBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextFieldBox")
            .field("text", &self.edit.text())
            .field("selection", &self.edit.selection())
            .field("focused", &self.focused)
            .field("composing", &self.edit.is_composing())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Ticking
// ---------------------------------------------------------------------------

/// Serve an assistive-technology request aimed at a text field.
///
/// The field's accessibility node advertises [`AccessActions::SET_VALUE`], and
/// advertising a capability you don't actually serve is just lying to the
/// screen reader. This is what serves it; the shell only has to forward
/// whatever arrives from the platform adapter:
///
/// ```no_run
/// # use silka_core::access::AccessActionRequest;
/// # use silka_core::tree::RenderTree;
/// # fn contoh(tree: &mut RenderTree, permintaan: &AccessActionRequest) {
/// // Inside `WindowConfig::on_access_action(...)`:
/// silka_widgets::text_field::apply_access_action(tree, permintaan);
/// # }
/// ```
///
/// Returns `true` when the contents really did change — and when they did,
/// `on_change` has already been called, just as for a keystroke.
pub fn apply_access_action(tree: &mut RenderTree, request: &AccessActionRequest) -> bool {
    if request.action != AccessAction::SetValue {
        return false;
    }
    let Some(nilai) = request.value.clone() else {
        return false;
    };
    let berubah = tree
        .node_mut_ref::<TextFieldBox>(request.target)
        .is_some_and(|k| k.setel_nilai_bantu(&nilai));
    if berubah {
        tree.mark_needs_layout(request.target);
    }
    berubah
}

/// The first text field in `tree` — a shortcut for tests and the gallery.
///
/// Its springs are advanced by [`crate::advance`], one tick for the whole tree:
/// a new component only adds a branch there instead of growing a second frame
/// loop (§3.5).
pub fn first(tree: &RenderTree) -> Option<NodeId> {
    let mut tumpukan = vec![tree.root()];
    while let Some(id) = tumpukan.pop() {
        if tree.node_ref::<TextFieldBox>(id).is_some() {
            return Some(id);
        }
        tumpukan.extend_from_slice(tree.children(id));
    }
    None
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// A text field's props: **resolved tokens only**.
#[derive(Debug, Clone, PartialEq)]
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{text_field_in, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
/// let build = |v: &str| text_field_in(&fonts, &theme, v.to_string());
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, build("Hello"));
/// tree.layout(BoxConstraints::tight(Size::new(240.0, 44.0)));
///
/// // Rebuilding with the same value leaves the caret, the selection and the
/// // undo history exactly where they were.
/// assert!(reconcile(&mut tree, build("Hello")).is_noop());
///
/// // A new value updates the node rather than replacing it — replacing it
/// // would throw the undo history away mid-sentence.
/// let changed = reconcile(&mut tree, build("Hello there"));
/// assert_eq!(changed.replaced, 0);
/// assert!(changed.updated > 0);
/// ```
pub struct TextFieldProps {
    fonts: Fonts,
    value: String,
    placeholder: String,
    style: TextStyle,
    padding: Insets,
    corners: Corners,
    min_height: f32,
    caret_width: f32,
    label: Option<String>,
    disabled: bool,
    read_only: bool,
    arrows: ArrowKeys,

    color: Color,
    placeholder_color: Color,
    disabled_color: Color,
    selection_color: Color,
    caret_color: Color,
    background: Color,
    background_hover: Color,
    background_focus: Color,
    border_width: f32,
    border_color: Color,
    border_focus_color: Color,
    focus_ring: Option<FocusRing>,

    on_change: Option<TextCallback>,
    on_submit: Option<TextCallback>,
    spring: Spring,
}

impl ViewNode for TextFieldProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let edit = TextEdit::new(self.value.clone());
        Box::new(TextFieldBox {
            fonts: self.fonts.clone(),
            style: self.style.clone(),
            placeholder: self.placeholder.clone(),
            padding: self.padding,
            corners: self.corners,
            min_height: self.min_height,
            caret_width: self.caret_width,
            label: self.label.clone(),
            disabled: self.disabled,
            read_only: self.read_only,
            arrows: self.arrows,
            color: self.color,
            placeholder_color: self.placeholder_color,
            disabled_color: self.disabled_color,
            selection_color: self.selection_color,
            caret_color: self.caret_color,
            background: self.background,
            background_hover: self.background_hover,
            background_focus: self.background_focus,
            border_width: self.border_width,
            border_color: self.border_color,
            border_focus_color: self.border_focus_color,
            focus_ring: self.focus_ring,
            on_change: self.on_change.clone(),
            on_submit: self.on_submit.clone(),
            edit,
            props_value: self.value.clone(),
            hovered: false,
            focused: false,
            dragging: false,
            scroll: 0.0,
            size: Size::ZERO,
            direction: TextDirection::Ltr,
            hover_t: SpringValue::new(0.0).with_spring(self.spring),
            focus_t: SpringValue::new(0.0).with_spring(self.spring),
            layout: None,
            shaped: String::new(),
            shaped_scale: f32::NAN,
            showing_placeholder: false,
            run: GlyphRun::new(self.color),
            caret: Rect::default(),
            selection: Vec::new(),
            preedit: Vec::new(),
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TextFieldBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;

        // **The contents are overwritten only when the app actually changed
        // them.** Comparing props against props (not against the node's
        // contents) is the difference between a field you can type in and a
        // field that throws the caret backwards every time some unrelated
        // signal changes — the classic "controlled component" bug, the same one
        // `ViewportProps::scroll` sidesteps.
        if n.props_value != self.value {
            n.props_value.clone_from(&self.value);
            if n.edit.text() != self.value {
                n.edit.set_text(self.value.clone());
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
        }

        if n.style != self.style || n.placeholder != self.placeholder {
            n.style = self.style.clone();
            n.placeholder.clone_from(&self.placeholder);
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.padding != self.padding
            || n.min_height != self.min_height
            || n.caret_width != self.caret_width
        {
            n.padding = self.padding;
            n.min_height = self.min_height;
            n.caret_width = self.caret_width;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.corners != self.corners
            || n.color != self.color
            || n.placeholder_color != self.placeholder_color
            || n.disabled_color != self.disabled_color
            || n.selection_color != self.selection_color
            || n.caret_color != self.caret_color
            || n.background != self.background
            || n.background_hover != self.background_hover
            || n.background_focus != self.background_focus
            || n.border_width != self.border_width
            || n.border_color != self.border_color
            || n.border_focus_color != self.border_focus_color
            || n.focus_ring != self.focus_ring
        {
            n.corners = self.corners;
            n.color = self.color;
            n.placeholder_color = self.placeholder_color;
            n.disabled_color = self.disabled_color;
            n.selection_color = self.selection_color;
            n.caret_color = self.caret_color;
            n.background = self.background;
            n.background_hover = self.background_hover;
            n.background_focus = self.background_focus;
            n.border_width = self.border_width;
            n.border_color = self.border_color;
            n.border_focus_color = self.border_focus_color;
            n.focus_ring = self.focus_ring;
            // Text color follows the node's color: the run must be
            // re-rasterized.
            n.shaped_scale = f32::NAN;
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.read_only != self.read_only {
            n.read_only = self.read_only;
            dirty |= Dirty::PAINT;
        }
        // Nothing about the pixels changes, so nothing is marked dirty: this
        // only decides who gets ↑/↓ on the next keystroke.
        n.arrows = self.arrows;
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                n.hovered = false;
                n.dragging = false;
                n.hover_t.jump_to(0.0);
                n.focus_t.jump_to(0.0);
            }
            dirty |= Dirty::PAINT;
        }
        if n.fonts != self.fonts {
            n.fonts = self.fonts.clone();
            n.shaped_scale = f32::NAN;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.hover_t.spring() != self.spring {
            n.hover_t.set_spring(self.spring);
            n.focus_t.set_spring(self.spring);
        }
        // Callbacks are always replaced without comparison: closures are
        // rebuilt on every rebuild and capture fresh values (the same pattern
        // as `InteractiveProps::on_press`).
        n.on_change.clone_from(&self.on_change);
        n.on_submit.clone_from(&self.on_submit);
        dirty
    }
}

/// A Dart-style text field builder (§2.5).
///
/// ```
/// use silka_core::animation::Spring;
/// use silka_core::signals::Key;
/// use silka_paint::Insets;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{text_field_in, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // Everything optional is a method, and every value it takes comes from a
/// // token rather than from a literal in application code.
/// let field = text_field_in(&fonts, &theme, "")
///     .placeholder("name@example.com")
///     .label("Email")
///     .padding(Insets::symmetric(theme.space(3.0), theme.space(2.0)))
///     .corners(theme.corners_of(silka_theme::RadiusToken::Md))
///     .min_height(silka_widgets::MIN_HIT_TARGET)
///     .spring(Spring::smooth())
///     .key(Key::from("email"))
///     .on_change(|_| {});
/// # let _ = field;
/// ```
/// A Dart-style text field builder (§2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct TextField {
    props: TextFieldProps,
    key: Option<Key>,
}

/// A single-line text field — `text_field` (`KOMPONEN.md` Tier 2).
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_widgets::text_field;
///
/// let rt = Runtime::new();
/// let city = rt.signal(String::from("Ubud"));
///
/// let field = text_field(city.get())
///     .label("City")
///     .placeholder("Where to?")
///     .on_change(move |s| city.set(s.to_owned()));
/// # let _ = field;
/// ```
///
/// Use [`text_field_in`] outside a build pass.
pub fn text_field(value: impl Into<String>) -> TextField {
    text_field_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        value,
    )
}

/// A single-line text field — the `text_field` component (`KOMPONEN.md`
/// Tier 2).
///
/// Every value comes from `theme`; `fonts` is the app's text engine.
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{text_field_in, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
/// let rt = Runtime::new();
/// let query = rt.signal(String::new());
///
/// // The value is *owned by the caller*: the field is told what to display
/// // and reports what it has become, so there is only ever one copy of the
/// // text in the application.
/// let search = text_field_in(&fonts, &theme, query.get())
///     .placeholder("Search")
///     .label("Search documents")
///     .on_change(move |text| query.set(text.to_string()))
///     .on_submit(|text| println!("searching for {text}"));
/// # let _ = search;
///
/// // Read-only is not the same as disabled: the caret still moves, the text
/// // is still selectable, and it can still be copied out.
/// let readonly = text_field_in(&fonts, &theme, "abc-123").read_only(true);
/// # let _ = readonly;
/// ```
pub fn text_field_in(fonts: &Fonts, theme: &Theme, value: impl Into<String>) -> TextField {
    let t = theme;
    TextField {
        props: TextFieldProps {
            fonts: fonts.clone(),
            value: value.into(),
            placeholder: String::new(),
            style: TextStyle::new()
                .size(t.typography.body_size)
                .line_height(t.typography.body_line_height)
                .single_line(),
            padding: Insets::symmetric(t.space(3.0), t.space(1.5)),
            corners: t.corners(t.radius.md),
            // The control token, not the hit-target floor. A field used to be
            // exactly 44pt tall because that floor was the only number available,
            // which made it taller than every AppKit field beside it. The target
            // is still honoured — `hit_target_of` clamps it — but the two are now
            // separate questions with separate answers.
            min_height: t
                .control_of(ControlToken::Md)
                .max(t.hit_target_of(ControlToken::Md)),
            // As thin as the smallest spacing step: the HIG caret is a hairline,
            // not a slab.
            caret_width: t.space_of(SpaceToken::Px),
            label: None,
            disabled: false,
            read_only: false,
            arrows: ArrowKeys::Caret,
            color: t.color.label,
            placeholder_color: t.color.tertiary_label,
            disabled_color: t.color.disabled_label,
            selection_color: t.color.selection,
            caret_color: t.color.accent,
            background: t.color.surface,
            background_hover: t.color.surface_hover,
            background_focus: t.color.surface,
            border_width: t.space_of(SpaceToken::Px),
            border_color: t.color.border,
            border_focus_color: t.color.accent,
            focus_ring: Some(FocusRing::new(t.space(0.5), t.color.focus_ring)),
            on_change: None,
            on_submit: None,
            spring: Spring::snappy(),
        },
        key: None,
    }
}

impl TextField {
    fn map(mut self, f: impl FnOnce(&mut TextFieldProps)) -> Self {
        f(&mut self.props);
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The faint text shown while the field is empty.
    pub fn placeholder(self, placeholder: impl Into<String>) -> Self {
        let p = placeholder.into();
        self.map(move |x| x.placeholder = p)
    }

    /// The name a screen reader reads out (§3.8) — the visual `label`'s twin.
    pub fn label(self, label: impl Into<String>) -> Self {
        let l = label.into();
        self.map(move |x| x.label = Some(l))
    }

    /// Disable the field: it takes neither focus nor keystrokes, but is still
    /// read out.
    pub fn disabled(self, disabled: bool) -> Self {
        self.map(move |x| x.disabled = disabled)
    }

    /// The contents can be selected and copied, but not changed.
    pub fn read_only(self, read_only: bool) -> Self {
        self.map(move |x| x.read_only = read_only)
    }

    /// What ↑ and ↓ do — the caret by default, or nothing at all so that an
    /// enclosing control can have them ([`ArrowKeys`]).
    ///
    /// Set to [`ArrowKeys::Bubble`] by [`mod@crate::combo_box`], and by anything
    /// else that puts a list under a field.
    pub fn arrow_keys(self, arrows: ArrowKeys) -> Self {
        self.map(move |x| x.arrows = arrows)
    }

    /// Called every time the field's contents change — **without** the IME
    /// preedit.
    pub fn on_change(self, f: impl Fn(&str) + 'static) -> Self {
        let cb = TextCallback::new(f);
        self.map(move |x| x.on_change = Some(cb))
    }

    /// Called when Enter is pressed.
    pub fn on_submit(self, f: impl Fn(&str) + 'static) -> Self {
        let cb = TextCallback::new(f);
        self.map(move |x| x.on_submit = Some(cb))
    }

    /// A complete text style (e.g. one already assembled from typography
    /// tokens).
    pub fn style(self, style: TextStyle) -> Self {
        self.map(move |x| x.style = style)
    }

    /// Spacing inside the field's edges — **always** the token spacing scale
    /// (§2.6).
    pub fn padding(self, padding: Insets) -> Self {
        self.map(move |x| x.padding = padding)
    }

    /// Corner shape: squircle on Cupertino, arc on Tailwind — two equally valid
    /// values, both of them shader parameters (§3.6).
    pub fn corners(self, corners: Corners) -> Self {
        self.map(move |x| x.corners = corners)
    }

    /// Minimum height; defaults to the medium control token, clamped up by the
    /// [`MIN_HIT_TARGET`] floor (HIG).
    pub fn min_height(self, height: f32) -> Self {
        self.map(move |x| x.min_height = height.max(0.0))
    }

    /// The spring that drives the hover/focus transitions.
    pub fn spring(self, spring: Spring) -> Self {
        self.map(move |x| x.spring = spring)
    }
}

impl From<TextField> for View {
    fn from(t: TextField) -> View {
        let mut b = Builder::new(t.props);
        if let Some(key) = t.key {
            b = b.key(key);
        }
        b.into()
    }
}

#[cfg(test)]
mod tests;
