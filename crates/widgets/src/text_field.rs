//! `text_field()` — **komponen tersulit di seluruh katalog** (`KOMPONEN.md`
//! Tier 2), dan karena itu yang dikerjakan paling awal: ia memaksa stack text,
//! IME, dan accessibility matang lebih cepat (REKOMENDASI §5 failure mode #1
//! dan #2).
//!
//! ```
//! # use rustui_core::signals::Runtime;
//! # use rustui_theme::{Appearance, Theme};
//! # use rustui_widgets::{text_field, Fonts};
//! # let rt = Runtime::new();
//! # let nama = rt.signal(String::new());
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! text_field(&fonts, &t, nama.get())
//!     .placeholder("Nama lengkap")
//!     .label("Nama")
//!     .on_change(move |s| nama.set(s.to_string()));
//! ```
//!
//! ## Yang membuatnya benar, bukan sekadar terlihat benar
//!
//! | Bagian | Di mana | Kenapa di sana |
//! |---|---|---|
//! | Caret/seleksi per grapheme, undo, preedit | [`rustui_text::edit`] | Aturan Unicode (UAX #29), bukan aturan tampilan — dan bisa diuji tanpa piksel |
//! | Geometri caret, hit-test, kotak seleksi | [`rustui_text::TextLayout`] | Hanya hasil shaping yang tahu di mana huruf berdiri |
//! | Fokus, capture penunjuk, klik beruntun | [`rustui_core::input`] | Sudah jadi kontrak input framework |
//! | Token warna/jarak/sudut | [`rustui_theme`] | Satu tulisan, dua preset (§2.7) |
//!
//! Node ini menempelkan keempatnya, dan **tidak menambahkan aturan Unicode
//! sendiri satu baris pun**.
//!
//! ## Definition of Done (`KOMPONEN.md`) yang dipenuhi
//!
//! - **Kedua preset** lewat token semantik; tidak ada satu angka warna pun di
//!   berkas ini, dan bentuk sudut adalah parameter ([`Corners`]), bukan
//!   konstanta (§2.7, §3.6).
//! - **Semua state interaktif bertransisi spring**: hover dan fokus adalah
//!   [`SpringValue`] yang bisa di-retarget di tengah gerakan — cincin fokus
//!   tidak pernah "menyala" mendadak (§3.5).
//! - **Keyboard penuh**: ←/→ (per grapheme), ⌥←/⌥→ (per kata), ⌘←/⌘→, Home/End,
//!   Shift untuk memperluas, Backspace/Delete (+⌥ per kata), ⌘A, ⌘Z/⇧⌘Z, Enter
//!   untuk `on_submit`. Tab **tidak** ditangkap: ia milik navigasi fokus.
//! - **Node AccessKit** dengan peran [`AccessRole::TextInput`], nama, **nilai**,
//!   dan aksi `SET_VALUE` (dikte suara) — lengkap dengan status disabled.
//! - **Dark mode** ikut token; **hit target ≥ 44pt** ([`MIN_HIT_TARGET`]) walau
//!   tinggi barisnya jauh lebih kecil; **reduced-motion** dihormati karena
//!   seluruh gerakan lewat [`Tick`].
//! - **IME preedit dirender inline** dengan garis bawah, dan selama komposisi
//!   berjalan jalur tombol normal ditahan (§3.8). Selama itu pula `on_change`
//!   **tidak** dipanggil: aplikasi tidak pernah menerima huruf setengah jadi.
//!
//! ## Utang teknis yang disadari
//!
//! - **Clipboard** (⌘C/⌘X/⌘V) belum tersambung: `arboard` hidup di
//!   `rustui-platform` (INTEGRASI-NATIVE §4) dan crate ini tidak boleh
//!   bergantung padanya. Pintasannya sengaja **dibiarkan menggelembung** ke
//!   atas, bukan ditelan diam-diam, supaya shell bisa melayaninya nanti tanpa
//!   satu baris pun berubah di sini.
//! - **Caret tidak berkedip.** Kedipan butuh timer yang berdetak terus, dan itu
//!   bertabrakan dengan janji "render hanya saat dirty" (§3.5) sampai scheduler
//!   punya jalur timer resmi.
//! - Satu baris saja; multi-baris + soft wrap adalah `text_area`, yang memakai
//!   [`rustui_text::TextEdit::multiline`] yang sudah ada.

use std::rc::Rc;

use rustui_core::access::{
    AccessAction, AccessActionRequest, AccessActions, AccessNode, AccessRole,
};
use rustui_core::animation::{Spring, SpringValue, Tick};
use rustui_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, ImeEvent, KeyCode,
    KeyEvent, Modifiers, NamedKey, PointerButton, PointerPhase,
};
use rustui_core::scheduler::Dirty;
use rustui_core::signals::Key;
use rustui_core::tree::{
    BoxConstraints, Decoration, FocusRing, LayoutCtx, NodeId, PaintCtx, RenderNode, RenderTree,
};
use rustui_core::view::{Builder, View, ViewNode};
use rustui_paint::{Color, Corners, GlyphRun, Insets, Point, Quad, Rect, Size};
use rustui_text::{Caret, Movement, TextConstraints, TextEdit, TextLayout, TextStyle};
use rustui_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;

// ---------------------------------------------------------------------------
// Callback pembawa teks
// ---------------------------------------------------------------------------

/// Aksi yang menerima **isi kolom** — bentuk `on_change`/`on_submit`.
///
/// [`rustui_core::Callback`] sengaja tidak membawa argumen (ia melayani
/// `on_press`); kolom teks butuh satu, dan hanya satu: teksnya. Sifatnya sama —
/// `Clone` murah lewat [`Rc`], dan `PartialEq` berdasarkan identitas karena
/// closure dibangun ulang tiap rebuild.
#[derive(Clone)]
pub struct TextCallback(Rc<dyn Fn(&str)>);

impl TextCallback {
    /// Bungkus sebuah closure.
    pub fn new(f: impl Fn(&str) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Jalankan dengan isi kolom.
    pub fn call(&self, text: &str) {
        (self.0)(text)
    }
}

impl PartialEq for TextCallback {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for TextCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TextCallback")
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// Node render sebuah kolom teks.
///
/// Ia menggambar teksnya sendiri (bukan lewat anak [`crate::text`]) karena
/// caret, seleksi, dan preedit harus berbagi **satu** hasil shaping dengan
/// glyph yang digambar. Dua sumber layout untuk satu baris teks = caret yang
/// meleset setengah piksel, dan itu terlihat.
pub struct TextFieldBox {
    // -- konfigurasi (token yang sudah diresolusi satu tingkat di atas) --
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

    // -- state milik node (tidak pernah ditimpa diffing) --
    edit: TextEdit,
    /// Nilai yang terakhir **datang dari props**, dan hanya dari sana: mengetik
    /// tidak pernah mengubahnya. Pembanding untuk tahu apakah aplikasi benar-
    /// benar mengganti isinya (lihat [`TextFieldProps::update`]).
    props_value: String,
    hovered: bool,
    focused: bool,
    dragging: bool,
    scroll: f32,
    size: Size,

    // -- animasi (§3.5) --
    hover_t: SpringValue<f32>,
    focus_t: SpringValue<f32>,

    // -- turunan: selalu hasil dari yang di atas --
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
    /// Isi kolom **tersimpan** — tanpa preedit yang sedang dikomposisi.
    pub fn text(&self) -> &str {
        self.edit.text()
    }

    /// Seleksi saat ini (indeks byte).
    pub fn selection(&self) -> rustui_text::Selection {
        self.edit.selection()
    }

    /// Benar bila IME sedang mengomposisi di kolom ini.
    pub fn is_composing(&self) -> bool {
        self.edit.is_composing()
    }

    /// Kotak caret dalam koordinat lokal node (hasil layout terakhir).
    pub fn caret_rect(&self) -> Rect {
        self.caret
    }

    /// Kotak-kotak sorot seleksi, koordinat lokal node.
    pub fn selection_rects(&self) -> &[Rect] {
        &self.selection
    }

    /// Kotak garis bawah preedit, koordinat lokal node.
    pub fn preedit_rects(&self) -> &[Rect] {
        &self.preedit
    }

    /// Benar bila yang tampil adalah placeholder (kolom kosong).
    pub fn shows_placeholder(&self) -> bool {
        self.showing_placeholder
    }

    /// Geseran horizontal isi, poin logis.
    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    /// Benar bila salah satu transisinya masih bergerak.
    pub fn is_animating(&self) -> bool {
        self.hover_t.is_animating() || self.focus_t.is_animating()
    }

    /// Majukan transisi satu frame; benar bila ada yang berubah.
    ///
    /// Dipanggil [`crate::advance`], satu-satunya tempat seluruh spring sebuah
    /// pohon dimajukan bersama-sama.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        if !self.is_animating() {
            return false;
        }
        let sebelum = (self.hover_t.position(), self.focus_t.position());
        tick.advance(&mut self.hover_t);
        tick.advance(&mut self.focus_t);
        (self.hover_t.position(), self.focus_t.position()) != sebelum
    }

    /// Selesaikan seluruh transisi seketika (uji dan snapshot).
    pub fn settle(&mut self) {
        self.hover_t.settle();
        self.focus_t.settle();
    }

    // -- geometri -----------------------------------------------------------

    /// Kotak isi: kotak node dikurangi padding.
    fn kotak_isi(&self) -> Rect {
        Rect::from_origin_size(Point::ZERO, self.size).deflate(self.padding)
    }

    /// Tepi atas teks: satu baris selalu **di tengah** secara vertikal, karena
    /// hit target 44pt hampir selalu lebih tinggi dari barisnya (HIG).
    fn atas_teks(&self) -> f32 {
        let baris = self.style.line_height_px();
        ((self.size.height - baris) / 2.0).max(self.padding.top)
    }

    /// Shape ulang bila teks yang terlihat atau resolusi layar berubah.
    ///
    /// Dua alasan sah, dan hanya dua — aturan yang sama dengan [`crate::text`]:
    /// isi berubah, atau scale factor berubah (bitmap glyph terikat resolusi).
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

    /// Caret menurut hasil shaping — nol bila yang tampil placeholder.
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

    /// Hitung ulang scroll, caret, seleksi, preedit, dan glyph run.
    ///
    /// Satu-satunya tempat koordinat lahir; `paint` hanya menggambar apa yang
    /// sudah dihitung di sini, karena rasterisasi butuh `&mut` mesin teks dan
    /// pass paint tidak punya itu.
    fn perbarui_geometri(&mut self) {
        let isi = self.kotak_isi();
        let atas = self.atas_teks();
        let caret = self.caret_teks();

        // Guliran horizontal: caret selalu terlihat, dan isi tidak pernah
        // digeser lebih jauh dari yang perlu.
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

        let asal = Point::new(isi.origin.x - self.scroll, atas);
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

        // Garis bawah preedit: setebal caret, menempel di dasar baris — bentuk
        // yang dipakai semua OS untuk menandai "ini belum jadi" (§3.8).
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
            // Token `disabled_label`, bukan warna teks yang diredupkan sendiri:
            // "dimmed" adalah keputusan theme, bukan keputusan widget (§2.7).
            self.disabled_color
        } else {
            self.color
        };
        self.run = match &self.layout {
            Some(l) => {
                let mut run = self.fonts.with(|m| m.rasterize(l, asal, warna));
                // Isi yang lebih panjang dari kolomnya dipotong di tepi isi,
                // bukan menabrak border.
                run.clip = Some(pandang);
                run
            }
            None => GlyphRun::new(warna),
        };
    }

    /// Indeks byte di bawah titik `local` (koordinat lokal node).
    fn indeks_di(&self, local: Point) -> usize {
        if self.showing_placeholder {
            return 0;
        }
        let isi = self.kotak_isi();
        let titik = Point::new(
            local.x - isi.origin.x + self.scroll,
            // Satu baris: apa pun tinggi kliknya, barisnya itu-itu juga.
            self.style.line_height_px() / 2.0,
        );
        self.layout.as_ref().map_or(0, |l| l.hit(titik))
    }

    /// Dekorasi untuk keadaan sekarang — hasil **interpolasi spring**, bukan
    /// lompatan antar tiga warna.
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
            shadows: rustui_paint::ShadowPair::NONE,
        }
    }

    // -- reaksi terhadap perubahan ------------------------------------------

    /// Setelah teks berubah: shape ulang, hitung geometri, lapor ke aplikasi.
    fn setelah_teks_berubah(&mut self, ctx: &mut EventCtx<'_>) {
        self.pastikan_bentuk();
        self.perbarui_geometri();
        // `props_value` sengaja **tidak** disentuh di sini: ia mencatat apa yang
        // terakhir diberikan aplikasi, bukan apa yang diketik pengguna. Itulah
        // yang membuat kolom tanpa `on_change` tetap bisa diketik (props-nya
        // tidak pernah berubah, jadi tidak pernah menimpa), sementara kolom
        // yang terkendali tetap menerima nilai baru dari aplikasi.
        //
        // Nilai yang dilaporkan **tidak pernah** memuat preedit: aplikasi hanya
        // melihat teks yang sudah jadi (§3.8).
        if let Some(cb) = self.on_change.clone() {
            cb.call(self.edit.text());
        }
        ctx.request_layout();
        self.perbarui_ime(ctx);
    }

    /// Setelah caret/seleksi berubah tapi teksnya tidak.
    fn setelah_caret_berubah(&mut self, ctx: &mut EventCtx<'_>) {
        self.perbarui_geometri();
        ctx.request_paint();
        self.perbarui_ime(ctx);
    }

    /// Beri tahu shell di mana jendela kandidat IME harus berdiri.
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

    /// Bisa disunting sama sekali?
    fn bisa_sunting(&self) -> bool {
        !self.disabled && !self.read_only
    }

    /// Ganti isi atas permintaan **teknologi bantu** (dikte suara, isi ulang
    /// field) — jalur masuk [`AccessAction::SetValue`].
    ///
    /// Sengaja terpisah dari jalur props: yang satu adalah aplikasi yang
    /// menyetel nilai, yang ini adalah *pengguna* yang mengetik lewat cara
    /// lain — jadi ia wajib memanggil `on_change`, persis seperti ketikan
    /// keyboard.
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
        // **Selama komposisi IME, jalur tombol normal ditahan** (§3.8): huruf
        // yang sedang dipilih di jendela kandidat tidak boleh juga masuk
        // sebagai ketikan biasa.
        if self.edit.is_composing() {
            ctx.handled();
            return;
        }

        let m = k.modifiers;
        let shift = m.contains(Modifiers::SHIFT);
        let cmd = m.contains(Modifiers::COMMAND);
        let alt = m.contains(Modifiers::ALT);
        let sebelum = self.edit.text().to_string();
        let mut tertangani = true;
        let mut caret_berubah = false;

        match &k.code {
            KeyCode::Named(n) => match n {
                NamedKey::ArrowLeft => {
                    let gerak = if cmd {
                        Movement::LineStart
                    } else if alt {
                        Movement::PrevWord
                    } else {
                        Movement::Prev
                    };
                    caret_berubah = self.edit.move_caret(gerak, shift);
                }
                NamedKey::ArrowRight => {
                    let gerak = if cmd {
                        Movement::LineEnd
                    } else if alt {
                        Movement::NextWord
                    } else {
                        Movement::Next
                    };
                    caret_berubah = self.edit.move_caret(gerak, shift);
                }
                // Satu baris: atas/bawah = ujung baris, kebiasaan AppKit.
                NamedKey::ArrowUp | NamedKey::Home => {
                    caret_berubah = self.edit.move_caret(Movement::LineStart, shift);
                }
                NamedKey::ArrowDown | NamedKey::End => {
                    caret_berubah = self.edit.move_caret(Movement::LineEnd, shift);
                }
                NamedKey::Backspace if self.bisa_sunting() => {
                    if alt || cmd {
                        self.edit.delete_word_backward();
                    } else {
                        self.edit.delete_backward();
                    }
                }
                NamedKey::Delete if self.bisa_sunting() => {
                    if alt || cmd {
                        self.edit.delete_word_forward();
                    } else {
                        self.edit.delete_forward();
                    }
                }
                NamedKey::Space if self.bisa_sunting() && !cmd => {
                    self.edit.insert(k.text.as_deref().unwrap_or(" "));
                }
                NamedKey::Enter => {
                    if let Some(cb) = self.on_submit.clone() {
                        cb.call(self.edit.text());
                    } else {
                        tertangani = false;
                    }
                }
                // Esc dan Tab sengaja dibiarkan lewat: yang pertama milik
                // overlay, yang kedua milik navigasi fokus.
                _ => tertangani = false,
            },

            KeyCode::Character(c) if cmd => match c.to_ascii_lowercase() {
                'a' => caret_berubah = self.edit.select_all(),
                'z' if self.bisa_sunting() => {
                    if shift {
                        self.edit.redo();
                    } else {
                        self.edit.undo();
                    }
                }
                // ⌘C/⌘X/⌘V dibiarkan menggelembung: clipboard hidup di
                // `rustui-platform` (lihat catatan modul).
                _ => tertangani = false,
            },

            KeyCode::Character(c) if self.bisa_sunting() && !m.contains(Modifiers::CONTROL) => {
                // Teks dari platform sudah melewati layout keyboard dan dead
                // key; `c` hanyalah cadangan untuk event sintetis (uji).
                let teks = k.text.clone().unwrap_or_else(|| c.to_string());
                self.edit.insert(&teks);
            }

            _ => tertangani = false,
        }

        if !tertangani {
            return;
        }
        ctx.handled();
        if self.edit.text() != sebelum {
            self.setelah_teks_berubah(ctx);
        } else if caret_berubah || matches!(&k.code, KeyCode::Named(_)) {
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
            // Preedit berubah = teks yang **terlihat** berubah, jadi shaping
            // ulang tetap perlu — tapi aplikasi tidak diberi tahu apa pun.
            self.pastikan_bentuk();
            self.perbarui_geometri();
            ctx.request_layout();
            self.perbarui_ime(ctx);
        }
    }

    // -- penunjuk -----------------------------------------------------------

    fn penunjuk(&mut self, ctx: &mut EventCtx<'_>, p: &rustui_core::input::PointerEvent) {
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
                    // Klik ganda = satu kata, klik tripel = seluruh isi —
                    // ambang waktunya milik framework (`ClickConfig`), sehingga
                    // sama di tiga OS.
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
                // Drag-select: penunjuk sudah ditangkap, jadi menyeret keluar
                // kolom pun tetap memperluas seleksi.
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

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
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

        // Cincin fokus **tumbuh** bersama spring: 0 saat diam, penuh saat
        // fokus. Digambar di luar kotak agar tidak menutupi isi (kebiasaan
        // AppKit, sama dengan `Interactive`).
        let fokus = self.focus_t.position().clamp(0.0, 1.0);
        if let Some(ring) = self.focus_ring.filter(|r| fokus > 0.0 && r.width > 0.0) {
            let tebal = ring.width * fokus;
            let kotak = ctx.local_bounds().deflate(Insets::all(-tebal));
            let corners = rustui_paint::Corners::new(
                rustui_paint::CornerRadii::all(self.corners.radii.max() + tebal),
                self.corners.style,
            );
            ctx.quad(
                Quad::new(kotak)
                    .corners(corners)
                    .border(tebal, ring.color.with_alpha(ring.color.a * fokus)),
            );
        }

        // Sorot seleksi **di bawah** teks; alpha-nya ikut fokus supaya kolom
        // yang kehilangan fokus tidak terlihat masih "aktif".
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

        // Garis bawah preedit: di atas teks, karena ia menandai teks itu.
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
        // Nilai yang dibacakan adalah nilai tersimpan — bukan preedit yang
        // masih setengah jadi.
        node.value = Some(self.edit.text().to_string());
        node.disabled = self.disabled;
        if !self.disabled {
            node.actions |= AccessActions::CLICK | AccessActions::FOCUS;
            if !self.read_only {
                // Dikte suara dan "isi ulang field" milik teknologi bantu.
                node.actions |= AccessActions::SET_VALUE;
            }
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Kolom mati tetap menyerap: klik padanya tidak boleh menembus ke
        // konten di belakangnya.
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
                    // Komposisi yang menggantung saat fokus pergi dibuang: IME
                    // tidak akan pernah mengirim commit-nya lagi.
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
// Detak
// ---------------------------------------------------------------------------

/// Layani permintaan teknologi bantu yang ditujukan ke sebuah kolom teks.
///
/// Node aksesibilitas kolom mengumumkan [`AccessActions::SET_VALUE`], dan
/// mengumumkan kemampuan yang tidak dilayani sama saja dengan berbohong kepada
/// screen reader. Inilah yang melayaninya; shell tinggal meneruskan apa yang
/// datang dari adapter platform:
///
/// ```no_run
/// # use rustui_core::access::AccessActionRequest;
/// # use rustui_core::tree::RenderTree;
/// # fn contoh(tree: &mut RenderTree, permintaan: &AccessActionRequest) {
/// // Di dalam `WindowConfig::on_access_action(...)`:
/// rustui_widgets::text_field::apply_access_action(tree, permintaan);
/// # }
/// ```
///
/// Mengembalikan `true` bila isinya benar-benar berubah — dan bila iya,
/// `on_change` sudah dipanggil, sama seperti ketikan keyboard.
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

/// Kolom teks pertama di `tree` — jalan pintas untuk uji dan gallery.
///
/// Spring-nya sendiri dimajukan [`crate::advance`], satu detak untuk seluruh
/// pohon: komponen baru cukup menambah satu cabang di sana, bukan menumbuhkan
/// loop frame kedua (§3.5).
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

/// Props sebuah kolom teks: **hanya token yang sudah diresolusi**.
#[derive(Debug, Clone, PartialEq)]
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
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        // **Isi hanya ditimpa kalau aplikasi memang mengubahnya.** Membandingkan
        // props dengan props (bukan dengan isi node) adalah bedanya antara
        // kolom yang bisa diketik dan kolom yang melempar caret ke belakang
        // setiap kali ada signal lain berubah — bug "controlled component"
        // klasik, sama yang dihindari `ViewportProps::scroll`.
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
            // Warna teks ikut warna node: run harus dirasterisasi ulang.
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
        // Callback selalu diganti tanpa dibandingkan: closure dibangun ulang
        // tiap rebuild dan menangkap nilai baru (pola yang sama dengan
        // `InteractiveProps::on_press`).
        n.on_change.clone_from(&self.on_change);
        n.on_submit.clone_from(&self.on_submit);
        dirty
    }
}

/// Builder kolom teks bergaya Dart (§2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct TextField {
    props: TextFieldProps,
    key: Option<Key>,
}

/// Kolom teks satu baris — komponen `text_field` (`KOMPONEN.md` Tier 2).
///
/// Seluruh nilainya datang dari `theme`; `fonts` adalah mesin teks aplikasi.
pub fn text_field(fonts: &Fonts, theme: &Theme, value: impl Into<String>) -> TextField {
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
            min_height: MIN_HIT_TARGET,
            // Setipis satu langkah spacing terkecil: caret HIG adalah garis
            // rambut, bukan balok.
            caret_width: t.space(0.25),
            label: None,
            disabled: false,
            read_only: false,
            color: t.color.label,
            placeholder_color: t.color.tertiary_label,
            disabled_color: t.color.disabled_label,
            selection_color: t.color.selection,
            caret_color: t.color.accent,
            background: t.color.surface,
            background_hover: t.color.surface_hover,
            background_focus: t.color.surface,
            border_width: t.space(0.25),
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

    /// Kunci identitas di antara saudara-saudaranya (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Teks samar saat kolom kosong.
    pub fn placeholder(self, placeholder: impl Into<String>) -> Self {
        let p = placeholder.into();
        self.map(move |x| x.placeholder = p)
    }

    /// Nama yang dibacakan screen reader (§3.8) — pasangan `label` visual.
    pub fn label(self, label: impl Into<String>) -> Self {
        let l = label.into();
        self.map(move |x| x.label = Some(l))
    }

    /// Matikan kolom: tidak menerima fokus maupun ketikan, tetap dibacakan.
    pub fn disabled(self, disabled: bool) -> Self {
        self.map(move |x| x.disabled = disabled)
    }

    /// Isinya bisa diseleksi dan disalin, tapi tidak bisa diubah.
    pub fn read_only(self, read_only: bool) -> Self {
        self.map(move |x| x.read_only = read_only)
    }

    /// Dipanggil setiap kali isi kolom berubah — **tanpa** preedit IME.
    pub fn on_change(self, f: impl Fn(&str) + 'static) -> Self {
        let cb = TextCallback::new(f);
        self.map(move |x| x.on_change = Some(cb))
    }

    /// Dipanggil saat Enter ditekan.
    pub fn on_submit(self, f: impl Fn(&str) + 'static) -> Self {
        let cb = TextCallback::new(f);
        self.map(move |x| x.on_submit = Some(cb))
    }

    /// Gaya teks lengkap (mis. yang sudah dirakit dari token typography).
    pub fn style(self, style: TextStyle) -> Self {
        self.map(move |x| x.style = style)
    }

    /// Jarak di dalam tepi kolom — **selalu** skala spacing token (§2.6).
    pub fn padding(self, padding: Insets) -> Self {
        self.map(move |x| x.padding = padding)
    }

    /// Bentuk sudut: squircle di Cupertino, arc di Tailwind — dua nilai yang
    /// sama sahnya, keduanya parameter shader (§3.6).
    pub fn corners(self, corners: Corners) -> Self {
        self.map(move |x| x.corners = corners)
    }

    /// Tinggi minimum; bawaannya [`MIN_HIT_TARGET`] (HIG).
    pub fn min_height(self, height: f32) -> Self {
        self.map(move |x| x.min_height = height.max(0.0))
    }

    /// Spring yang menjalankan transisi hover/fokus.
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
