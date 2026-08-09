//! Node render tabel: [`TableBody`], [`TableHeaderBox`], [`TableRowBox`], dan
//! [`TableCellBox`].
//!
//! Pembagian kerjanya sengaja dibuat setipis mungkin, karena setiap node
//! tambahan adalah satu tempat baru di mana geometri kolom bisa berbeda dari
//! tetangganya:
//!
//! | Node | Yang benar-benar dikerjakannya |
//! |---|---|
//! | [`TableBody`] | jendela baris, seleksi, keyboard, sorotan — peran a11y `Table` |
//! | [`TableHeaderBox`] | seret resize, seret geser kolom, klik sort — peran a11y `Row` |
//! | [`TableRowBox`] | menempatkan sel pada kolomnya — peran a11y `Row` |
//! | [`TableCellBox`] | perataan + padding satu sel — peran a11y `Cell` |
//!
//! Ketiganya menyelesaikan lebar kolom lewat fungsi yang **sama**
//! ([`solve_widths`]) dari lebar layout masing-masing, jadi tidak ada satu pun
//! yang perlu bertanya kepada yang lain — dan tidak ada satu poin pun selisih
//! antara garis header dan garis barisnya.
//!
//! Guliran, pantulan, dan scrollbar tidak ada di berkas ini sama sekali:
//! semuanya milik [`scroll_view`](mod@crate::scroll_view), tempat tabel ini
//! tinggal. Aritmetika virtualisasinya pun bukan milik tabel — ia
//! [`ListMetrics`], objek yang sama persis dengan yang dipakai
//! [`list`](mod@crate::list) (`KOMPONEN.md` aturan urutan #4).

use std::rc::Rc;

use rustui_core::access::{AccessActions, AccessNode, AccessRole};
use rustui_core::animation::{Spring, SpringValue, Tick};
use rustui_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, KeyCode, KeyEvent,
    Modifiers, NamedKey, PointerButton, PointerEvent, PointerPhase,
};
use rustui_core::tree::{BoxConstraints, Decoration, FocusRing, LayoutCtx, PaintCtx, RenderNode};
use rustui_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, Size};

use crate::list::{ListMetrics, RowAction};

use super::column::{
    column_at, drop_index, handle_at, next_sort, offsets, reorder, solve_widths, CellAlign,
    ColumnLayout, SortBy,
};
use super::selection::{Selection, SelectionMode};
use super::state::TableState;

/// Sejauh apa penunjuk harus bergerak sebelum tekan pada judul kolom berubah
/// dari "klik untuk mengurutkan" menjadi "seret untuk memindahkan".
///
/// Tanpa ambang ini setiap klik sort yang tangannya sedikit bergetar akan
/// diam-diam menggeser kolom — kegagalan yang membuat tabel terasa licin.
pub const REORDER_THRESHOLD: f32 = 4.0;

/// Jumlah bilah penyusun segitiga penanda urutan.
const SORT_BARS: usize = 5;

// ---------------------------------------------------------------------------
// Gaya
// ---------------------------------------------------------------------------

/// Nilai token yang **sudah diresolusi** untuk isi sebuah tabel.
///
/// Tidak satu pun angka warna lahir di lapisan ini: semuanya datang dari
/// [`rustui_theme::Theme`] satu tingkat di atas (§2.6, §2.7), sehingga preset
/// Cupertino dan Tailwind berganti tanpa satu baris pun berubah di sini.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableStyle {
    /// Latar isi tabel.
    pub decoration: Decoration,
    /// Bentuk sudut sorotan baris.
    pub row_corners: Corners,
    /// Latar baris terpilih saat tabel memegang fokus (token `selection`).
    pub selection: Color,
    /// Latar baris terpilih saat fokus ada di tempat lain — kebiasaan macOS:
    /// seleksi tidak hilang, ia meredup.
    pub selection_idle: Color,
    /// Latar baris di bawah penunjuk (token `surface_hover`).
    pub hover: Color,
    /// Latar baris yang sedang ditekan (token `surface_pressed`).
    pub pressed: Color,
    /// Latar baris ganjil bila [`TableStyle::striped`] menyala.
    pub stripe: Color,
    /// Baris berselang-seling berlatar `stripe` — kebiasaan tabel data padat.
    pub striped: bool,
    /// Warna garis antar baris dan antar kolom (token `separator`).
    pub separator: Color,
    /// Tebal garis antar baris; `0` = tanpa garis.
    pub separator_width: f32,
    /// Tebal garis antar kolom; `0` = tanpa garis.
    pub grid_width: f32,
    /// Cincin fokus keyboard di sekeliling **sel** aktif (token `focus_ring`).
    pub focus_ring: Option<FocusRing>,
}

impl Default for TableStyle {
    fn default() -> Self {
        Self {
            decoration: Decoration::NONE,
            row_corners: Corners::SHARP,
            selection: Color::TRANSPARENT,
            selection_idle: Color::TRANSPARENT,
            hover: Color::TRANSPARENT,
            pressed: Color::TRANSPARENT,
            stripe: Color::TRANSPARENT,
            striped: false,
            separator: Color::TRANSPARENT,
            separator_width: 0.0,
            grid_width: 0.0,
            focus_ring: None,
        }
    }
}

/// Nilai token yang sudah diresolusi untuk **header** sebuah tabel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeaderStyle {
    /// Latar header — wajib buram: baris yang lewat di bawahnya tidak boleh
    /// tembus saat header menempel.
    pub background: Color,
    /// Latar judul kolom di bawah penunjuk.
    pub hover: Color,
    /// Latar judul kolom yang sedang ditekan.
    pub pressed: Color,
    /// Warna garis pemisah (bawah header dan antar kolom).
    pub separator: Color,
    /// Tebal garis pemisah.
    pub separator_width: f32,
    /// Warna segitiga penanda urutan dan garis penunjuk tujuan geser.
    pub indicator: Color,
    /// Lebar segitiga penanda urutan.
    pub indicator_size: f32,
    /// Warna pegangan resize saat penunjuk berada di atasnya.
    pub handle: Color,
    /// Tebal pegangan resize saat disorot.
    pub handle_width: f32,
}

impl Default for HeaderStyle {
    fn default() -> Self {
        Self {
            background: Color::TRANSPARENT,
            hover: Color::TRANSPARENT,
            pressed: Color::TRANSPARENT,
            separator: Color::TRANSPARENT,
            separator_width: 0.0,
            indicator: Color::TRANSPARENT,
            indicator_size: 8.0,
            handle: Color::TRANSPARENT,
            handle_width: 2.0,
        }
    }
}

// ---------------------------------------------------------------------------
// TableBody
// ---------------------------------------------------------------------------

/// Node isi tabel tervirtualisasi.
///
/// Seperti [`ListBody`](crate::list::ListBody), ia melapor setinggi **seluruh**
/// isi (`header + count × extent`) tapi hanya memiliki node untuk baris di
/// dalam jendela. Baris ke-99.999 karena itu bisa ditempatkan tanpa pernah
/// membangun 99.998 node sebelumnya.
pub struct TableBody {
    // -- properti (datang dari view) -------------------------------------
    pub(super) metrics: ListMetrics,
    pub(super) offset: f32,
    pub(super) first: usize,
    pub(super) rows: usize,
    pub(super) has_header: bool,
    pub(super) has_empty: bool,
    pub(super) mode: SelectionMode,
    pub(super) selection: Selection,
    pub(super) columns: Rc<[ColumnLayout]>,
    pub(super) active: usize,
    pub(super) label: Option<String>,
    pub(super) style: TableStyle,
    pub(super) state: Option<TableState>,
    pub(super) on_activate: Option<RowAction>,
    /// Lebar jalur scrollbar di tepi yang **tidak** boleh menelan klik.
    pub(super) bar_inset: f32,

    // -- keadaan runtime (tidak pernah disentuh diffing) -----------------
    /// Tepi atas sorotan baris aktif — springnya yang membuat seleksi
    /// *meluncur* antar baris alih-alih berkedip pindah.
    lead_y: SpringValue<f32>,
    /// Kepekatan sorotan baris aktif.
    lead_alpha: SpringValue<f32>,
    /// Kepekatan sorotan baris terpilih lainnya (seleksi jamak).
    sel_alpha: SpringValue<f32>,
    hover_y: SpringValue<f32>,
    hover_alpha: SpringValue<f32>,
    press_alpha: SpringValue<f32>,

    hovered: Option<usize>,
    pressed: Option<usize>,
    focused: bool,
    /// Baris yang menunggu digulirkan ke layar (dilayani [`super::sync`]).
    reveal: Option<usize>,
    width: f32,
    rtl: bool,
}

/// Spring sorotan baris.
///
/// **Dekoratif** dengan sengaja: yang membawa informasi adalah baris mana yang
/// terpilih, bukan perjalanan sorotannya. Di bawah reduced-motion sorotan
/// langsung berada di tempatnya (§3.5).
fn sorotan_spring(spring: Spring) -> SpringValue<f32> {
    SpringValue::new(0.0).with_spring(spring).decorative()
}

impl TableBody {
    /// Node baru dari props yang sudah diresolusi.
    pub(super) fn from_props(props: &super::view::TableProps) -> Self {
        let mut node = Self {
            metrics: props.metrics,
            offset: props.offset,
            first: props.first,
            rows: props.rows,
            has_header: props.has_header,
            has_empty: props.has_empty,
            mode: props.mode,
            selection: props.selection.clone(),
            columns: props.columns.clone(),
            active: props.active,
            label: props.label.clone(),
            style: props.style,
            state: Some(props.state),
            on_activate: props.on_activate.clone(),
            bar_inset: props.bar_inset,
            lead_y: sorotan_spring(props.spring),
            lead_alpha: sorotan_spring(props.spring),
            sel_alpha: sorotan_spring(props.spring),
            hover_y: sorotan_spring(props.spring),
            hover_alpha: sorotan_spring(props.spring),
            press_alpha: sorotan_spring(props.spring),
            hovered: None,
            pressed: None,
            focused: false,
            reveal: None,
            width: 0.0,
            rtl: false,
        };
        // Tabel yang lahir dengan seleksi (state yang dipulihkan) **tidak**
        // menganimasikan sorotannya masuk: itu bukan gerakan, itu keadaan awal.
        node.pasang_sorotan(false);
        node
    }

    /// Ukuran-ukuran tabel yang berlaku.
    pub fn metrics(&self) -> ListMetrics {
        self.metrics
    }

    /// Baris-baris yang sedang terpilih.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Baris aktif (pemegang cincin fokus).
    pub fn lead(&self) -> Option<usize> {
        self.selection.lead()
    }

    /// Kolom aktif, sebagai indeks **tampil**.
    pub fn active_column(&self) -> usize {
        self.active
    }

    /// Baris di bawah penunjuk.
    pub fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// Benar bila tabel memegang fokus keyboard.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// State yang dipakai tabel ini, bila ada.
    pub fn state(&self) -> Option<TableState> {
        self.state
    }

    /// Indeks baris pertama yang benar-benar dimaterialisasi.
    pub fn first(&self) -> usize {
        self.first
    }

    /// Berapa baris yang benar-benar dimaterialisasi menjadi node.
    pub fn materialized(&self) -> usize {
        self.rows
    }

    /// Kolom-kolom dalam urutan tampil.
    pub fn columns(&self) -> &[ColumnLayout] {
        &self.columns
    }

    /// Lebar tiap kolom pada lebar tabel hasil layout terakhir.
    pub fn column_widths(&self) -> Vec<f32> {
        solve_widths(&self.columns, self.width)
    }

    /// Kotak baris `index` dalam **koordinat isi**.
    pub fn row_rect(&self, index: usize) -> Rect {
        Rect::new(
            0.0,
            self.metrics.row_top(index),
            self.width,
            self.metrics.extent,
        )
    }

    /// Ambil permintaan "gulirkan baris ini ke layar" yang tertunda.
    pub(super) fn take_reveal(&mut self) -> Option<usize> {
        self.reveal.take()
    }

    // -- animasi ----------------------------------------------------------

    /// Benar bila masih ada sorotan yang bergerak.
    pub fn is_animating(&self) -> bool {
        self.lead_y.is_animating()
            || self.lead_alpha.is_animating()
            || self.sel_alpha.is_animating()
            || self.hover_y.is_animating()
            || self.hover_alpha.is_animating()
            || self.press_alpha.is_animating()
    }

    /// Majukan sorotan satu frame; benar bila ada piksel yang berubah.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let sebelum = self.snapshot();
        tick.advance(&mut self.lead_y);
        tick.advance(&mut self.lead_alpha);
        tick.advance(&mut self.sel_alpha);
        tick.advance(&mut self.hover_y);
        tick.advance(&mut self.hover_alpha);
        tick.advance(&mut self.press_alpha);
        sebelum != self.snapshot()
    }

    fn snapshot(&self) -> [f32; 6] {
        [
            self.lead_y.position(),
            self.lead_alpha.position(),
            self.sel_alpha.position(),
            self.hover_y.position(),
            self.hover_alpha.position(),
            self.press_alpha.position(),
        ]
    }

    /// Selesaikan seluruh gerakan sorotan seketika (uji, snapshot).
    pub fn settle(&mut self) {
        self.lead_y.settle();
        self.lead_alpha.settle();
        self.sel_alpha.settle();
        self.hover_y.settle();
        self.hover_alpha.settle();
        self.press_alpha.settle();
    }

    /// Ganti spring seluruh sorotan tanpa mengganggu gerakan yang berjalan.
    pub fn set_spring(&mut self, spring: Spring) {
        self.lead_y.set_spring(spring);
        self.lead_alpha.set_spring(spring);
        self.sel_alpha.set_spring(spring);
        self.hover_y.set_spring(spring);
        self.hover_alpha.set_spring(spring);
        self.press_alpha.set_spring(spring);
    }

    /// Spring yang menjalankan sorotan.
    pub fn spring(&self) -> Spring {
        self.lead_y.spring()
    }

    /// Arahkan sorotan ke keadaan seleksi sekarang.
    fn pasang_sorotan(&mut self, animasi: bool) {
        let ada = !self.selection.is_empty();
        self.sel_alpha.set_target(if ada { 1.0 } else { 0.0 });
        match self.selection.lead() {
            Some(i) => {
                let y = self.metrics.row_top(i);
                // Sorotan yang baru muncul **tidak** meluncur dari baris lama:
                // ia memudar masuk di tempatnya. Yang meluncur hanya
                // perpindahan saat sorotannya memang sudah terlihat.
                if self.lead_alpha.position() <= 0.0 || !animasi {
                    self.lead_y.jump_to(y);
                } else {
                    self.lead_y.set_target(y);
                }
                self.lead_alpha.set_target(1.0);
            }
            None => self.lead_alpha.set_target(0.0),
        }
        if !animasi {
            self.lead_alpha.settle();
            self.sel_alpha.settle();
            self.lead_y.settle();
        }
    }

    fn pasang_hover(&mut self, index: Option<usize>) {
        let Some(i) = index else {
            self.hover_alpha.set_target(0.0);
            return;
        };
        let y = self.metrics.row_top(i);
        if self.hover_alpha.position() <= 0.0 {
            self.hover_y.jump_to(y);
        } else {
            self.hover_y.set_target(y);
        }
        self.hover_alpha.set_target(1.0);
    }

    // -- seleksi ----------------------------------------------------------

    /// Setel seleksi di node **dan** terbitkan ke [`TableState`].
    pub(super) fn set_selection(&mut self, selection: Selection, animasi: bool) -> bool {
        if self.selection == selection {
            return false;
        }
        self.selection = selection;
        self.pasang_sorotan(animasi);
        if let Some(state) = self.state {
            state.set_selection(self.selection.clone());
        }
        true
    }

    fn set_active(&mut self, column: usize) {
        let batas = self.columns.len().saturating_sub(1);
        let baru = column.min(batas);
        if self.active == baru {
            return;
        }
        self.active = baru;
        if let Some(state) = self.state {
            state.set_active_column(baru);
        }
    }

    /// Berapa baris yang muat dalam satu layar penuh (Page Up/Down).
    fn sehalaman(&self) -> usize {
        if self.metrics.extent <= 0.0 {
            return 1;
        }
        let atap = if self.metrics.sticky {
            self.metrics.header
        } else {
            0.0
        };
        let muat = ((self.metrics.viewport - atap) / self.metrics.extent).floor();
        if muat >= 1.0 {
            muat as usize
        } else {
            1
        }
    }

    /// Baris tujuan setelah bergeser `delta` langkah dari baris aktif.
    fn langkah(&self, delta: isize) -> usize {
        let terakhir = (self.metrics.count - 1) as isize;
        match self.selection.lead() {
            None if delta > 0 => 0,
            None => terakhir as usize,
            Some(i) => (i as isize + delta).clamp(0, terakhir) as usize,
        }
    }

    /// Koordinat mendatar dalam **arah baca**: di RTL, kolom pertama ada di
    /// kanan, jadi seluruh aritmetika kolom bekerja pada nilai yang dicerminkan
    /// (§9.8).
    fn reading_x(&self, x: f32) -> f32 {
        if self.rtl {
            self.width - x
        } else {
            x
        }
    }

    /// Baris yang berada di titik lokal `p` (koordinat isi).
    fn baris_di(&self, p: Point) -> Option<usize> {
        // Header yang menempel menutupi baris di bawahnya: klik di atasnya
        // adalah klik pada header, bukan pada baris yang kebetulan lewat.
        if self.has_header && self.metrics.sticky {
            let atas = self.offset;
            if p.y >= atas && p.y < atas + self.metrics.header {
                return None;
            }
        }
        self.metrics.index_at(p.y)
    }

    /// Benar bila titik ini berada di jalur scrollbar yang melayang di atas
    /// tabel.
    fn di_jalur_scrollbar(&self, p: Point) -> bool {
        self.bar_inset > 0.0
            && self.metrics.max_scroll() > 0.0
            && p.x >= self.width - self.bar_inset
    }

    /// Kotak sel `(baris, kolom tampil)` dalam koordinat isi.
    pub fn cell_rect(&self, row: usize, column: usize) -> Rect {
        let widths = self.column_widths();
        let tepi = offsets(&widths);
        let (Some(w), Some(x)) = (widths.get(column), tepi.get(column)) else {
            return Rect::new(0.0, self.metrics.row_top(row), 0.0, self.metrics.extent);
        };
        let x = if self.rtl { self.width - x - w } else { *x };
        Rect::new(x, self.metrics.row_top(row), *w, self.metrics.extent)
    }

    // -- input ------------------------------------------------------------

    fn penunjuk(&mut self, ctx: &mut EventCtx<'_>, p: &PointerEvent) {
        match p.phase {
            PointerPhase::Enter | PointerPhase::Move => {
                let baris = self.baris_di(ctx.local());
                if self.hovered != baris {
                    self.hovered = baris;
                    self.pasang_hover(baris);
                    ctx.request_animation();
                    ctx.request_paint();
                }
            }
            PointerPhase::Leave => {
                if self.hovered.take().is_some() {
                    self.pasang_hover(None);
                    self.press_alpha.set_target(0.0);
                    ctx.request_animation();
                    ctx.request_paint();
                }
            }
            PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                if self.di_jalur_scrollbar(ctx.local()) {
                    return;
                }
                let Some(baris) = self.baris_di(ctx.local()) else {
                    return;
                };
                self.pressed = Some(baris);
                self.press_alpha.set_target(1.0);
                ctx.capture_pointer();
                if self.mode.is_selectable() {
                    ctx.request_focus();
                    // Kolom yang diklik menjadi sel aktif: navigasi keyboard
                    // berikutnya melanjutkan dari tempat jari berhenti.
                    let widths = self.column_widths();
                    if let Some(k) = column_at(&widths, self.reading_x(ctx.local().x)) {
                        self.set_active(k);
                    }
                    let mut seleksi = self.selection.clone();
                    if seleksi.apply_click(baris, p.modifiers, self.mode) {
                        self.set_selection(seleksi, true);
                    }
                }
                ctx.request_animation();
                ctx.request_paint();
                ctx.handled();
            }
            PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                let baris = self.baris_di(ctx.local());
                let ditekan = self.pressed.take();
                if ditekan.is_none() {
                    return;
                }
                self.press_alpha.set_target(0.0);
                ctx.release_pointer();
                // Ketuk-ganda membuka, ketuk tunggal hanya memilih — kebiasaan
                // setiap tabel macOS. `== 2` dan bukan `>= 2`: ketukan ketiga
                // dan keempat tidak boleh membuka baris yang sama lagi.
                if ditekan == baris && p.click_count == 2 {
                    if let (Some(i), Some(aksi)) = (baris, self.on_activate.clone()) {
                        aksi.call(i);
                    }
                }
                ctx.request_animation();
                ctx.request_paint();
                ctx.handled();
            }
            PointerPhase::Cancel if self.pressed.take().is_some() => {
                self.press_alpha.set_target(0.0);
                ctx.request_animation();
                ctx.request_paint();
            }
            _ => {}
        }
    }

    fn tombol(&mut self, ctx: &mut EventCtx<'_>, k: &KeyEvent) {
        if self.metrics.count == 0 || !self.mode.is_selectable() {
            return;
        }
        let m = k.modifiers;

        // ⌘A memilih seluruh baris — satu rentang, berapa pun jumlahnya.
        if self.mode == SelectionMode::Multiple
            && m.is_exactly(Modifiers::COMMAND)
            && matches!(&k.code, KeyCode::Character(c) if c.eq_ignore_ascii_case(&'a'))
        {
            let mut seleksi = self.selection.clone();
            seleksi.select_all(self.metrics.count);
            if self.set_selection(seleksi, true) {
                ctx.request_animation();
                ctx.request_paint();
            }
            ctx.handled();
            return;
        }

        // Esc melepas seleksi — jalan keluar yang selalu ada.
        if k.code.is(NamedKey::Escape) && m.is_empty() && !self.selection.is_empty() {
            if self.set_selection(Selection::default(), true) {
                ctx.request_animation();
                ctx.request_paint();
            }
            ctx.handled();
            return;
        }

        // Navigasi **antar sel**: kolom aktif bergeser, seleksi baris tidak
        // tersentuh. Di RTL panah kanan berarti kolom sebelumnya (§9.8).
        let maju = if self.rtl {
            NamedKey::ArrowLeft
        } else {
            NamedKey::ArrowRight
        };
        let mundur = if self.rtl {
            NamedKey::ArrowRight
        } else {
            NamedKey::ArrowLeft
        };
        if m.is_empty() && (k.code.is(maju) || k.code.is(mundur)) {
            let terakhir = self.columns.len().saturating_sub(1);
            let baru = if k.code.is(maju) {
                (self.active + 1).min(terakhir)
            } else {
                self.active.saturating_sub(1)
            };
            if baru != self.active {
                self.set_active(baru);
                ctx.request_paint();
            }
            ctx.handled();
            return;
        }

        let extend = m.is_exactly(Modifiers::SHIFT) && self.mode == SelectionMode::Multiple;
        if !m.is_empty() && !extend {
            return;
        }
        let sehalaman = self.sehalaman() as isize;
        let terakhir = self.metrics.count - 1;
        let tujuan = match &k.code {
            c if c.is(NamedKey::ArrowDown) => Some(self.langkah(1)),
            c if c.is(NamedKey::ArrowUp) => Some(self.langkah(-1)),
            c if c.is(NamedKey::PageDown) => Some(self.langkah(sehalaman)),
            c if c.is(NamedKey::PageUp) => Some(self.langkah(-sehalaman)),
            c if c.is(NamedKey::Home) => Some(0),
            c if c.is(NamedKey::End) => Some(terakhir),
            c if (c.is(NamedKey::Enter) || c.is(NamedKey::Space)) && m.is_empty() => {
                let (Some(i), Some(aksi)) = (self.selection.lead(), self.on_activate.clone())
                else {
                    return;
                };
                aksi.call(i);
                ctx.handled();
                return;
            }
            _ => None,
        };
        let Some(index) = tujuan else { return };
        let mut seleksi = self.selection.clone();
        seleksi.apply_move(index, extend, self.mode);
        self.set_selection(seleksi, true);
        // Guliran ke baris aktif dijalankan `sync`, yang memegang pohon.
        self.reveal = Some(index);
        ctx.request_animation();
        ctx.request_paint();
        ctx.handled();
    }

    // -- gambar -----------------------------------------------------------

    fn sorot(&self, ctx: &mut PaintCtx<'_>, y: f32, warna: Color, alpha: f32) {
        if alpha <= 0.0 || warna.a <= 0.0 {
            return;
        }
        ctx.quad(
            Quad::new(Rect::new(0.0, y, self.width, self.metrics.extent))
                .background(warna.with_alpha(warna.a * alpha.clamp(0.0, 1.0)))
                .corners(self.style.row_corners),
        );
    }

    fn warna_seleksi(&self) -> Color {
        if self.focused {
            self.style.selection
        } else {
            self.style.selection_idle
        }
    }
}

impl RenderNode for TableBody {
    fn type_name(&self) -> &'static str {
        "TableBody"
    }

    /// Baris ditempatkan sendiri, jadi node ini menyerap penunjuk yang tidak
    /// diambil isinya — tombol di dalam sel tetap menang karena hit-test
    /// menelusuri anak lebih dulu.
    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    /// Tabel yang bisa dipilih adalah **satu** perhentian Tab (pola NSTableView
    /// dan ARIA grid): di dalamnya panah yang berkuasa, bukan Tab.
    fn focus_policy(&self) -> FocusPolicy {
        if self.mode.is_selectable() && self.metrics.count > 0 {
            FocusPolicy::FOCUSABLE
        } else {
            FocusPolicy::NONE
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let lebar = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            constraints.min_width
        };
        self.width = lebar;

        let jumlah_anak = ctx.child_count();
        let baris = self.rows.min(jumlah_anak);
        for k in 0..baris {
            let anak = ctx.child(k);
            let c = BoxConstraints::new(lebar, lebar, self.metrics.extent, self.metrics.extent);
            ctx.layout_child_boundary(anak, c);
            ctx.place_child(anak, Point::new(0.0, self.metrics.row_top(self.first + k)));
        }

        let mut tinggi = self.metrics.content();
        let mut idx = baris;
        if self.has_empty && idx < jumlah_anak {
            let anak = ctx.child(idx);
            // Empty state mengisi jendela pandang bila tingginya sudah
            // diketahui, supaya isinya bisa diratakan di tengah oleh
            // aplikasinya sendiri.
            let ruang = (self.metrics.viewport - self.metrics.header).max(0.0);
            let c = if ruang > 0.0 {
                BoxConstraints::new(lebar, lebar, ruang, ruang)
            } else {
                BoxConstraints::new(lebar, lebar, 0.0, f32::INFINITY)
            };
            let ukuran = ctx.layout_child_boundary(anak, c);
            ctx.place_child(anak, Point::new(0.0, self.metrics.header));
            tinggi = tinggi.max(self.metrics.header + ukuran.height);
            idx += 1;
        }
        // Header **terakhir** supaya ia tergambar di atas baris tanpa perlu
        // pembungkus clip kedua.
        if self.has_header && idx < jumlah_anak {
            let anak = ctx.child(idx);
            let c = BoxConstraints::new(lebar, lebar, self.metrics.header, self.metrics.header);
            ctx.layout_child_boundary(anak, c);
            let atas = if self.metrics.sticky {
                self.offset
                    .clamp(0.0, (tinggi - self.metrics.header).max(0.0))
            } else {
                0.0
            };
            ctx.place_child(anak, Point::new(0.0, atas));
        }

        let size = Size::new(lebar, constraints.constrain_height(tinggi));
        if let Some(state) = self.state {
            state
                .scroll_state()
                .publish_content(tinggi, self.metrics.extent, self.metrics.header);
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.style.decoration);
        let akhir = (self.first + self.rows).min(self.metrics.count);

        // Zebra: hanya untuk baris yang dimaterialisasi — seratus ribu baris
        // tetap menghasilkan belasan perintah gambar.
        if self.style.striped && self.style.stripe.a > 0.0 {
            for i in self.first..akhir {
                if i % 2 == 1 {
                    self.sorot(ctx, self.metrics.row_top(i), self.style.stripe, 1.0);
                }
            }
        }

        if self.mode.is_selectable() {
            if self.hovered.is_some_and(|h| !self.selection.contains(h)) {
                self.sorot(
                    ctx,
                    self.hover_y.position(),
                    self.style.hover,
                    self.hover_alpha.position(),
                );
            }
            // Baris terpilih **selain** yang aktif: mereka tidak sedang
            // bergerak ke mana-mana, jadi mereka tidak meluncur — hanya
            // kepekatannya yang bertransisi.
            let warna = self.warna_seleksi();
            let lead = self.selection.lead();
            for (a, b) in self.selection.ranges_within(self.first, self.rows) {
                for i in a..=b {
                    if Some(i) == lead {
                        continue;
                    }
                    self.sorot(
                        ctx,
                        self.metrics.row_top(i),
                        warna,
                        self.sel_alpha.position(),
                    );
                }
            }
            // Baris aktif: inilah yang meluncur antar baris.
            self.sorot(
                ctx,
                self.lead_y.position(),
                warna,
                self.lead_alpha.position(),
            );
            if let Some(i) = self.pressed {
                self.sorot(
                    ctx,
                    self.metrics.row_top(i),
                    self.style.pressed,
                    self.press_alpha.position(),
                );
            }
        }

        // Garis antar baris.
        if self.style.separator_width > 0.0 && self.style.separator.a > 0.0 {
            for i in self.first.max(1)..akhir {
                ctx.quad(
                    Quad::new(Rect::new(
                        0.0,
                        self.metrics.row_top(i),
                        self.width,
                        self.style.separator_width,
                    ))
                    .background(self.style.separator),
                );
            }
        }

        // Garis antar kolom: satu perintah per kolom, membentang setinggi isi.
        // Clip wadah gulir yang memotongnya — bukan aritmetika di sini.
        if self.style.grid_width > 0.0 && self.style.separator.a > 0.0 && self.metrics.count > 0 {
            let widths = self.column_widths();
            let tepi = offsets(&widths);
            let atas = self.metrics.header;
            let tinggi = (self.metrics.content() - atas).max(0.0);
            for x in tepi.iter().skip(1).take(widths.len().saturating_sub(1)) {
                let x = if self.rtl {
                    self.width - x - self.style.grid_width
                } else {
                    *x
                };
                ctx.quad(
                    Quad::new(Rect::new(x, atas, self.style.grid_width, tinggi))
                        .background(self.style.separator),
                );
            }
        }

        ctx.paint_children();

        // Cincin fokus mengelilingi **sel** aktif, bukan seluruh baris: itulah
        // yang membuat navigasi ← → punya arti yang terlihat.
        if self.focused && self.lead_alpha.position() > 0.0 {
            if let (Some(ring), Some(baris)) = (
                self.style
                    .focus_ring
                    .filter(|r| r.width > 0.0 && r.color.a > 0.0),
                self.selection.lead(),
            ) {
                let mut kotak = self.cell_rect(baris, self.active);
                // Sel mengikuti sorotan yang sedang meluncur, bukan koordinat
                // statis barisnya — kalau tidak, cincin fokus akan mendahului
                // sorotan yang menyusulnya.
                kotak = Rect::new(
                    kotak.origin.x,
                    self.lead_y.position(),
                    kotak.size.width,
                    kotak.size.height,
                )
                .deflate(Insets::all(ring.width / 2.0));
                let corners = Corners::new(
                    CornerRadii::all((self.style.row_corners.radii.max() - ring.width).max(0.0)),
                    self.style.row_corners.style,
                );
                ctx.quad(
                    Quad::new(kotak)
                        .corners(corners)
                        .border(ring.width, ring.color),
                );
            }
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Table;
        node.label.clone_from(&self.label);
        if self.mode.is_selectable() && self.metrics.count > 0 {
            node.actions |= AccessActions::FOCUS;
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            Event::Pointer(p) => self.penunjuk(ctx, p),
            Event::Key(k) if k.is_pressed() => self.tombol(ctx, k),
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                // Tabel yang baru menerima fokus tanpa seleksi tidak punya
                // tempat untuk cincin fokusnya. Kebiasaan AppKit: baris pertama
                // yang terlihat menjadi titik mulai.
                if self.focused
                    && self.mode.is_selectable()
                    && self.metrics.count > 0
                    && self.selection.is_empty()
                {
                    let pertama = self.metrics.index_at(self.offset).unwrap_or(0);
                    self.set_selection(Selection::single(pertama), false);
                    self.reveal = Some(pertama);
                }
                ctx.request_animation();
                ctx.request_paint();
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for TableBody {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TableBody")
            .field("count", &self.metrics.count)
            .field("first", &self.first)
            .field("rows", &self.rows)
            .field("columns", &self.columns.len())
            .field("selected", &self.selection.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// TableHeaderBox
// ---------------------------------------------------------------------------

/// Apa yang sedang diseret di header.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    /// Menyeret batas kolom `boundary` (indeks tampil kolom di kirinya).
    Resize {
        /// Kolom yang lebarnya berubah.
        boundary: usize,
    },
    /// Menyeret judul kolom `from` ke posisi `to`.
    Reorder {
        /// Kolom yang diangkat, indeks tampil.
        from: usize,
        /// Posisi tampil tujuan saat ini.
        to: usize,
    },
}

/// Node baris judul kolom: sort, resize, dan geser kolom.
pub struct TableHeaderBox {
    pub(super) columns: Rc<[ColumnLayout]>,
    pub(super) sort: Option<SortBy>,
    pub(super) style: HeaderStyle,
    pub(super) state: Option<TableState>,
    pub(super) on_sort: Option<SortAction>,

    hovered: Option<usize>,
    /// Batas kolom yang sedang disorot penunjuk (pegangan resize).
    handle: Option<usize>,
    pressed: Option<usize>,
    /// Titik tekan awal, untuk membedakan klik sort dari seret geser.
    press_x: f32,
    drag: Option<Drag>,
    hover_alpha: SpringValue<f32>,
    hover_x: SpringValue<f32>,
    /// Posisi garis penunjuk tujuan geser, koordinat lokal.
    drop_x: SpringValue<f32>,
    size: Size,
    rtl: bool,
}

/// Aksi yang menerima kolom pengurut baru — `on_sort` gaya Dart (§2.5).
#[derive(Clone)]
pub struct SortAction(Rc<dyn Fn(SortBy)>);

impl SortAction {
    /// Bungkus sebuah closure menjadi aksi pengurutan.
    pub fn new(f: impl Fn(SortBy) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Jalankan aksi.
    pub fn call(&self, sort: SortBy) {
        (self.0)(sort)
    }
}

impl PartialEq for SortAction {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for SortAction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SortAction")
    }
}

impl TableHeaderBox {
    /// Node baru dari props yang sudah diresolusi.
    pub(super) fn from_props(props: &super::view::TableHeaderProps) -> Self {
        Self {
            columns: props.columns.clone(),
            sort: props.sort,
            style: props.style,
            state: Some(props.state),
            on_sort: props.on_sort.clone(),
            hovered: None,
            handle: None,
            pressed: None,
            press_x: 0.0,
            drag: None,
            hover_alpha: sorotan_spring(props.spring),
            hover_x: sorotan_spring(props.spring),
            drop_x: sorotan_spring(props.spring),
            size: Size::ZERO,
            rtl: false,
        }
    }

    /// Lebar tiap kolom pada lebar header hasil layout terakhir.
    pub fn column_widths(&self) -> Vec<f32> {
        solve_widths(&self.columns, self.size.width)
    }

    /// Kolom yang sedang di bawah penunjuk (indeks tampil).
    pub fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// Batas kolom yang siap diseret, bila penunjuk berada di atasnya.
    pub fn handle(&self) -> Option<usize> {
        self.handle
    }

    /// Benar bila lebar sebuah kolom sedang diseret.
    pub fn is_resizing(&self) -> bool {
        matches!(self.drag, Some(Drag::Resize { .. }))
    }

    /// Kolom yang sedang dipindahkan beserta tujuannya, bila ada.
    pub fn reordering(&self) -> Option<(usize, usize)> {
        match self.drag {
            Some(Drag::Reorder { from, to }) => Some((from, to)),
            _ => None,
        }
    }

    /// Benar bila masih ada sorotan header yang bergerak.
    pub fn is_animating(&self) -> bool {
        self.hover_alpha.is_animating() || self.hover_x.is_animating() || self.drop_x.is_animating()
    }

    /// Majukan sorotan satu frame; benar bila ada piksel yang berubah.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let sebelum = (
            self.hover_alpha.position(),
            self.hover_x.position(),
            self.drop_x.position(),
        );
        tick.advance(&mut self.hover_alpha);
        tick.advance(&mut self.hover_x);
        tick.advance(&mut self.drop_x);
        sebelum
            != (
                self.hover_alpha.position(),
                self.hover_x.position(),
                self.drop_x.position(),
            )
    }

    /// Selesaikan seluruh gerakan seketika (uji, snapshot).
    pub fn settle(&mut self) {
        self.hover_alpha.settle();
        self.hover_x.settle();
        self.drop_x.settle();
    }

    /// Ganti spring tanpa mengganggu gerakan yang berjalan.
    pub fn set_spring(&mut self, spring: Spring) {
        self.hover_alpha.set_spring(spring);
        self.hover_x.set_spring(spring);
        self.drop_x.set_spring(spring);
    }

    /// Spring yang menjalankan sorotan header.
    pub fn spring(&self) -> Spring {
        self.hover_alpha.spring()
    }

    fn reading_x(&self, x: f32) -> f32 {
        if self.rtl {
            self.size.width - x
        } else {
            x
        }
    }

    /// Tepi kiri kolom `k` dalam koordinat **lokal** (sudah dicerminkan).
    fn column_x(&self, widths: &[f32], k: usize) -> f32 {
        let tepi = offsets(widths);
        let x = tepi.get(k).copied().unwrap_or(0.0);
        if self.rtl {
            self.size.width - x - widths.get(k).copied().unwrap_or(0.0)
        } else {
            x
        }
    }

    fn pasang_hover(&mut self, index: Option<usize>, widths: &[f32]) {
        let Some(k) = index else {
            self.hover_alpha.set_target(0.0);
            return;
        };
        let x = self.column_x(widths, k);
        if self.hover_alpha.position() <= 0.0 {
            self.hover_x.jump_to(x);
        } else {
            self.hover_x.set_target(x);
        }
        self.hover_alpha.set_target(1.0);
    }

    fn penunjuk(&mut self, ctx: &mut EventCtx<'_>, p: &PointerEvent) {
        let widths = self.column_widths();
        let x = self.reading_x(ctx.local().x);
        match p.phase {
            PointerPhase::Enter | PointerPhase::Move => {
                if let Some(drag) = self.drag {
                    self.seret(ctx, drag, &widths, x);
                    return;
                }
                if let Some(k) = self.pressed {
                    // Ambang geser: di bawahnya ini masih calon klik sort.
                    if (ctx.local().x - self.press_x).abs() > REORDER_THRESHOLD
                        && self.columns.get(k).is_some_and(|c| c.movable)
                    {
                        self.drag = Some(Drag::Reorder { from: k, to: k });
                        self.drop_x.jump_to(self.column_x(&widths, k));
                        ctx.request_paint();
                    }
                    return;
                }
                let pegangan = handle_at(&self.columns, &widths, x);
                let kolom = column_at(&widths, x);
                if pegangan != self.handle {
                    self.handle = pegangan;
                    ctx.request_paint();
                }
                // Judul kolom yang sedang "menjadi pegangan" tidak ikut
                // disorot: dua umpan balik sekaligus di satu titik hanya
                // membingungkan.
                let sorot = if pegangan.is_some() { None } else { kolom };
                if self.hovered != sorot {
                    self.hovered = sorot;
                    self.pasang_hover(sorot, &widths);
                    ctx.request_animation();
                    ctx.request_paint();
                }
            }
            PointerPhase::Leave => {
                if self.hovered.take().is_some() || self.handle.take().is_some() {
                    self.pasang_hover(None, &widths);
                    ctx.request_animation();
                    ctx.request_paint();
                }
            }
            PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                if let Some(k) = handle_at(&self.columns, &widths, x) {
                    self.drag = Some(Drag::Resize { boundary: k });
                    self.handle = Some(k);
                    ctx.capture_pointer();
                    ctx.request_paint();
                    ctx.handled();
                    return;
                }
                if let Some(k) = column_at(&widths, x) {
                    self.pressed = Some(k);
                    self.press_x = ctx.local().x;
                    ctx.capture_pointer();
                    ctx.request_paint();
                    ctx.handled();
                }
            }
            PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                let drag = self.drag.take();
                let ditekan = self.pressed.take();
                ctx.release_pointer();
                match drag {
                    Some(Drag::Reorder { from, to }) => {
                        if from != to {
                            self.commit_reorder(from, to);
                        }
                        ctx.request_paint();
                        ctx.handled();
                    }
                    Some(Drag::Resize { .. }) => {
                        ctx.request_paint();
                        ctx.handled();
                    }
                    // Tanpa seret sama sekali: inilah klik sort.
                    None => {
                        if let Some(k) = ditekan {
                            if column_at(&widths, x) == Some(k) {
                                self.urutkan(k);
                            }
                            ctx.request_paint();
                            ctx.handled();
                        }
                    }
                }
            }
            PointerPhase::Cancel => {
                // Kedua `take` harus tetap dijalankan — dibatalkan OS berarti
                // seret **dan** tekanan sama-sama dilepas, bukan salah satu.
                let seret = self.drag.take().is_some();
                let tekan = self.pressed.take().is_some();
                if seret || tekan {
                    ctx.request_paint();
                }
            }
            _ => {}
        }
    }

    fn seret(&mut self, ctx: &mut EventCtx<'_>, drag: Drag, widths: &[f32], x: f32) {
        match drag {
            Drag::Resize { boundary } => {
                let Some(kolom) = self.columns.get(boundary) else {
                    return;
                };
                let lebar = super::column::width_for_handle(&self.columns, widths, boundary, x);
                if let Some(state) = self.state {
                    state.set_width(kolom.source, Some(lebar));
                }
                ctx.request_layout();
                ctx.request_paint();
                ctx.handled();
            }
            Drag::Reorder { from, .. } => {
                let tujuan = drop_index(&self.columns, widths, from, x);
                self.drag = Some(Drag::Reorder { from, to: tujuan });
                self.drop_x.set_target(self.column_x(widths, tujuan));
                ctx.request_animation();
                ctx.request_paint();
                ctx.handled();
            }
        }
    }

    fn commit_reorder(&mut self, from: usize, to: usize) {
        let Some(state) = self.state else { return };
        let mut order: Vec<usize> = self.columns.iter().map(|c| c.source).collect();
        reorder(&mut order, from, to);
        state.set_order(order);
        // Sel aktif ikut kolomnya, bukan tinggal di posisi lamanya.
        state.set_active_column(to);
    }

    fn urutkan(&mut self, k: usize) {
        let Some(kolom) = self.columns.get(k).filter(|c| c.sortable) else {
            return;
        };
        let baru = next_sort(self.sort, kolom.source);
        self.sort = Some(baru);
        if let Some(state) = self.state {
            state.set_sort(Some(baru));
        }
        if let Some(aksi) = &self.on_sort {
            aksi.call(baru);
        }
    }

    /// Segitiga penanda urutan di kotak `bounds`.
    fn gambar_indikator(&self, ctx: &mut PaintCtx<'_>, bounds: Rect, ascending: bool) {
        let w = self.style.indicator_size.max(0.0);
        if w <= 0.0 || self.style.indicator.a <= 0.0 {
            return;
        }
        let h = w * 0.55;
        let tinggi_bilah = h / SORT_BARS as f32;
        let cx = bounds.center().x;
        let y0 = bounds.center().y - h / 2.0;
        for i in 0..SORT_BARS {
            let t = i as f32 / (SORT_BARS - 1) as f32;
            let lebar = w * if ascending { t } else { 1.0 - t };
            if lebar <= 0.0 {
                continue;
            }
            ctx.quad(
                Quad::new(Rect::new(
                    cx - lebar / 2.0,
                    y0 + i as f32 * tinggi_bilah,
                    lebar,
                    tinggi_bilah,
                ))
                .background(self.style.indicator),
            );
        }
    }
}

impl RenderNode for TableHeaderBox {
    fn type_name(&self) -> &'static str {
        "TableHeader"
    }

    /// Header buram: klik di atasnya adalah klik pada header, bukan pada baris
    /// yang kebetulan lewat di bawahnya.
    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn cursor(&self) -> Option<CursorIcon> {
        if self.handle.is_some() || self.is_resizing() {
            Some(CursorIcon::ResizeHorizontal)
        } else if self.reordering().is_some() {
            Some(CursorIcon::Grabbing)
        } else {
            None
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let ukuran = Size::new(
            if constraints.has_bounded_width() {
                constraints.max_width
            } else {
                constraints.min_width
            },
            if constraints.has_bounded_height() {
                constraints.max_height
            } else {
                constraints.min_height
            },
        );
        self.size = ukuran;

        let widths = solve_widths(&self.columns, ukuran.width);
        let n = ctx.child_count().min(widths.len());
        for k in 0..n {
            let anak = ctx.child(k);
            let w = widths[k];
            let c = BoxConstraints::new(w, w, ukuran.height, ukuran.height);
            ctx.layout_child_boundary(anak, c);
            ctx.place_child(anak, Point::new(self.column_x(&widths, k), 0.0));
        }
        ukuran
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        if self.style.background.a > 0.0 {
            ctx.quad(Quad::new(bounds).background(self.style.background));
        }

        let widths = self.column_widths();

        // Sorotan judul kolom di bawah penunjuk / yang sedang ditekan.
        let alpha = self.hover_alpha.position();
        if alpha > 0.0 {
            let k = self.hovered.or(self.pressed);
            if let Some(w) = k.and_then(|k| widths.get(k)) {
                let warna = if self.pressed.is_some() {
                    self.style.pressed
                } else {
                    self.style.hover
                };
                if warna.a > 0.0 {
                    ctx.quad(
                        Quad::new(Rect::new(
                            self.hover_x.position(),
                            0.0,
                            *w,
                            bounds.size.height,
                        ))
                        .background(warna.with_alpha(warna.a * alpha.clamp(0.0, 1.0))),
                    );
                }
            }
        }

        ctx.paint_children();

        // Garis antar kolom + pegangan resize yang sedang disorot.
        if self.style.separator_width > 0.0 && self.style.separator.a > 0.0 {
            for k in 0..widths.len().saturating_sub(1) {
                let x = self.column_x(&widths, k) + if self.rtl { 0.0 } else { widths[k] }
                    - if self.rtl {
                        self.style.separator_width
                    } else {
                        0.0
                    };
                let disorot = self.handle == Some(k);
                let (warna, tebal) = if disorot {
                    (self.style.handle, self.style.handle_width)
                } else {
                    (self.style.separator, self.style.separator_width)
                };
                if warna.a > 0.0 {
                    ctx.quad(
                        Quad::new(Rect::new(x, 0.0, tebal, bounds.size.height)).background(warna),
                    );
                }
            }
            // Garis bawah header: batas antara judul dan datanya.
            ctx.quad(
                Quad::new(Rect::new(
                    0.0,
                    bounds.size.height - self.style.separator_width,
                    bounds.size.width,
                    self.style.separator_width,
                ))
                .background(self.style.separator),
            );
        }

        // Segitiga penanda urutan, di tepi akhir judul kolomnya.
        if let Some(sort) = self.sort {
            if let Some(k) = self.columns.iter().position(|c| c.source == sort.column) {
                if let Some(w) = widths.get(k) {
                    let x = self.column_x(&widths, k);
                    let sisi = self.style.indicator_size * 2.0;
                    let kotak = if self.rtl {
                        Rect::new(x, 0.0, sisi.min(*w), bounds.size.height)
                    } else {
                        Rect::new(
                            x + (*w - sisi).max(0.0),
                            0.0,
                            sisi.min(*w),
                            bounds.size.height,
                        )
                    };
                    self.gambar_indikator(ctx, kotak, sort.direction.is_ascending());
                }
            }
        }

        // Garis penunjuk tujuan saat kolom sedang dipindahkan.
        if let Some((from, _)) = self.reordering() {
            if let Some(w) = widths.get(from) {
                if self.style.indicator.a > 0.0 {
                    ctx.quad(
                        Quad::new(Rect::new(
                            self.drop_x.position(),
                            0.0,
                            *w,
                            bounds.size.height,
                        ))
                        .background(self.style.indicator.with_alpha(0.16)),
                    );
                    ctx.quad(
                        Quad::new(Rect::new(
                            self.drop_x.position(),
                            0.0,
                            self.style.handle_width.max(1.0),
                            bounds.size.height,
                        ))
                        .background(self.style.indicator),
                    );
                }
            }
        }
    }

    fn access(&self, node: &mut AccessNode) {
        // Baris judul **adalah** sebuah baris tabel bagi teknologi bantu —
        // sel-selnya yang menjadi judul kolom.
        node.role = AccessRole::Row;
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if let Event::Pointer(p) = event {
            self.penunjuk(ctx, p);
        }
    }
}

impl core::fmt::Debug for TableHeaderBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TableHeaderBox")
            .field("columns", &self.columns.len())
            .field("sort", &self.sort)
            .field("drag", &self.drag)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// TableRowBox
// ---------------------------------------------------------------------------

/// Node satu baris tabel: menempatkan sel pada kolomnya, dan mengumumkan
/// dirinya sebagai `Row` bagi teknologi bantu.
///
/// Ia tidak menggambar apa pun — sorotan seleksi milik [`TableBody`], yang tahu
/// geometri seluruh tabel.
#[derive(Debug, Clone, PartialEq)]
pub struct TableRowBox {
    /// Nomor baris ini di dalam data (bukan di dalam jendela).
    pub index: usize,
    /// Terpilih atau tidak; `None` = tabel ini memang tidak punya seleksi.
    pub selected: Option<bool>,
    /// Baris ini bisa diaktifkan (ketuk-ganda / Enter).
    pub activatable: bool,
    /// Kolom dalam urutan tampil.
    pub columns: Rc<[ColumnLayout]>,
    /// Arah baca hasil layout terakhir.
    rtl: bool,
}

impl TableRowBox {
    /// Baris baru.
    pub fn new(
        index: usize,
        selected: Option<bool>,
        activatable: bool,
        columns: Rc<[ColumnLayout]>,
    ) -> Self {
        Self {
            index,
            selected,
            activatable,
            columns,
            rtl: false,
        }
    }
}

impl RenderNode for TableRowBox {
    fn type_name(&self) -> &'static str {
        "TableRow"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let ukuran = Size::new(
            if constraints.has_bounded_width() {
                constraints.max_width
            } else {
                constraints.min_width
            },
            if constraints.has_bounded_height() {
                constraints.max_height
            } else {
                constraints.min_height
            },
        );
        let widths = solve_widths(&self.columns, ukuran.width);
        let tepi = offsets(&widths);
        let n = ctx.child_count().min(widths.len());
        for k in 0..n {
            let anak = ctx.child(k);
            let w = widths[k];
            let c = BoxConstraints::new(w, w, ukuran.height, ukuran.height);
            ctx.layout_child_boundary(anak, c);
            let x = if self.rtl {
                ukuran.width - tepi[k] - w
            } else {
                tepi[k]
            };
            ctx.place_child(anak, Point::new(x, 0.0));
        }
        ukuran
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Row;
        node.selected = self.selected;
        if self.activatable {
            node.actions |= AccessActions::CLICK;
        }
    }
}

// ---------------------------------------------------------------------------
// TableCellBox
// ---------------------------------------------------------------------------

/// Node satu sel: perataan + padding, dan peran `Cell` bagi teknologi bantu.
///
/// Isinya boleh view apa pun — teks, badge, tombol, sakelar — dan itulah yang
/// dimaksud "sel kustom" di `KOMPONEN.md`: tidak ada tipe sel khusus, hanya
/// sebuah kotak yang tahu cara meratakan isinya.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableCellBox {
    /// Perataan isi di dalam kolomnya.
    pub align: CellAlign,
    /// Jarak isi ke tepi sel.
    pub padding: Insets,
    /// Arah baca hasil layout terakhir.
    rtl: bool,
}

impl TableCellBox {
    /// Sel baru.
    pub fn new(align: CellAlign, padding: Insets) -> Self {
        Self {
            align,
            padding,
            rtl: false,
        }
    }
}

impl RenderNode for TableCellBox {
    fn type_name(&self) -> &'static str {
        "TableCell"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let ukuran = Size::new(
            if constraints.has_bounded_width() {
                constraints.max_width
            } else {
                constraints.min_width
            },
            if constraints.has_bounded_height() {
                constraints.max_height
            } else {
                constraints.min_height
            },
        );
        if ctx.child_count() == 0 {
            return ukuran;
        }
        let anak = ctx.child(0);
        let isi = ctx.layout_child(anak, constraints.deflate(self.padding).loosen());

        let ruang = (ukuran.width - self.padding.horizontal()).max(0.0);
        let sisa = (ruang - isi.width).max(0.0);
        // Perataan "start"/"end" mengikuti arah baca, bukan kiri/kanan layar
        // (§9.8): kolom angka tetap rata ke akhir baris di RTL.
        let geser = match (self.align, self.rtl) {
            (CellAlign::Start, false) | (CellAlign::End, true) => 0.0,
            (CellAlign::Center, _) => sisa / 2.0,
            (CellAlign::End, false) | (CellAlign::Start, true) => sisa,
        };
        let y = ((ukuran.height - isi.height) / 2.0).max(0.0);
        ctx.place_child(anak, Point::new(self.padding.left + geser, y));
        ukuran
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Cell;
    }
}
