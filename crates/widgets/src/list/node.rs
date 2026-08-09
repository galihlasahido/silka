//! Node render daftar tervirtualisasi: [`ListBody`] dan [`ListRowBox`].
//!
//! `ListBody` sengaja **bukan** wadah bergulir. Ia tinggal di dalam
//! [`scroll_view`](mod@crate::scroll_view) dan hanya mengerjakan bagian yang memang milik
//! daftar:
//!
//! | Milik `scroll_view` | Milik `ListBody` |
//! |---|---|
//! | momentum OS, rubber band, pantulan spring | jendela baris + penempatannya |
//! | scrollbar overlay + auto-hide | sorotan seleksi/hover (spring) |
//! | Page/Home/End sebagai **guliran** | ↑/↓/Page/Home/End sebagai **seleksi** |
//! | peran a11y `ScrollView` + aksi scroll | peran `List` + `ListItem` per baris |
//!
//! Pembagian itu bukan selera: `KOMPONEN.md` aturan urutan #4 melarang
//! menumbuhkan sistem guliran (dan nanti virtualisasi) kedua — `table` akan
//! menumpang keduanya lagi.
//!
//! Yang membuat penempatan baris murah: node ini melapor setinggi **seluruh**
//! isinya (`header + count × extent`) tapi hanya memiliki node untuk baris di
//! dalam jendela, dan setiap baris ditempatkan pada koordinat isi yang dihitung
//! langsung dari indeksnya ([`ListMetrics::row_top`]). Baris ke-99.999 karena
//! itu bisa ditempatkan tanpa pernah membangun 99.998 node sebelumnya.

use std::rc::Rc;

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, KeyEvent, NamedKey, PointerButton,
    PointerEvent, PointerPhase,
};
use silka_core::tree::{BoxConstraints, Decoration, FocusRing, LayoutCtx, PaintCtx, RenderNode};
use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, Size};

use super::geometry::ListMetrics;
use super::state::ListState;

/// Aksi yang menerima nomor baris — `on_activate` gaya Dart (§2.5).
///
/// Bentuknya sama dengan [`silka_core::Callback`] (`Rc`, `PartialEq`
/// identitas), hanya saja membawa argumen; begitu core punya `Callback<T>`,
/// inilah yang pertama dihapus.
///
/// Publik karena [`table`](mod@crate::table) memakainya juga: "aksi yang
/// menerima nomor baris" adalah konsep yang sama di daftar dan di tabel, dan
/// menyalinnya ke sana hanya akan melahirkan dua tipe yang berperilaku identik.
#[derive(Clone)]
pub struct RowAction(Rc<dyn Fn(usize)>);

impl RowAction {
    /// Bungkus sebuah closure menjadi aksi baris.
    pub fn new(f: impl Fn(usize) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Jalankan aksi untuk baris `index`.
    pub fn call(&self, index: usize) {
        (self.0)(index)
    }
}

impl PartialEq for RowAction {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for RowAction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RowAction")
    }
}

// ---------------------------------------------------------------------------
// Gaya
// ---------------------------------------------------------------------------

/// Nilai token yang **sudah diresolusi** untuk isi sebuah daftar.
///
/// Tidak satu pun angka warna lahir di lapisan ini: semuanya datang dari
/// [`silka_theme::Theme`] satu tingkat di atas (§2.6, §2.7), sehingga preset
/// Cupertino dan Tailwind berganti tanpa satu baris pun berubah di sini.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListStyle {
    /// Latar isi daftar (biasanya transparan — latar milik wadahnya).
    pub decoration: Decoration,
    /// Bentuk sudut sorotan baris (squircle di Cupertino, arc di Tailwind).
    pub row_corners: Corners,
    /// Latar baris terpilih saat daftar memegang fokus (token `selection`).
    pub selection: Color,
    /// Latar baris terpilih saat fokus ada di tempat lain — kebiasaan macOS:
    /// seleksi tidak hilang, ia meredup.
    pub selection_idle: Color,
    /// Latar baris di bawah penunjuk (token `surface_hover`).
    pub hover: Color,
    /// Latar baris yang sedang ditekan (token `surface_pressed`).
    pub pressed: Color,
    /// Warna garis antar baris (token `separator`).
    pub separator: Color,
    /// Tebal garis antar baris; `0` = tanpa garis.
    pub separator_width: f32,
    /// Cincin fokus keyboard di sekeliling baris terpilih (token `focus_ring`).
    pub focus_ring: Option<FocusRing>,
}

impl Default for ListStyle {
    fn default() -> Self {
        Self {
            decoration: Decoration::NONE,
            row_corners: Corners::SHARP,
            selection: Color::TRANSPARENT,
            selection_idle: Color::TRANSPARENT,
            hover: Color::TRANSPARENT,
            pressed: Color::TRANSPARENT,
            separator: Color::TRANSPARENT,
            separator_width: 0.0,
            focus_ring: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ListBody
// ---------------------------------------------------------------------------

/// Node isi daftar tervirtualisasi.
pub struct ListBody {
    // -- properti (datang dari view) -------------------------------------
    pub(super) metrics: ListMetrics,
    /// Posisi guliran yang berlaku, dibaca dari [`ListState`] saat build.
    pub(super) offset: f32,
    pub(super) first: usize,
    pub(super) rows: usize,
    pub(super) has_header: bool,
    pub(super) has_empty: bool,
    pub(super) selectable: bool,
    pub(super) selected: Option<usize>,
    pub(super) label: Option<String>,
    pub(super) style: ListStyle,
    pub(super) state: Option<ListState>,
    pub(super) on_activate: Option<RowAction>,
    /// Lebar jalur scrollbar di tepi kanan yang **tidak** boleh menelan klik.
    pub(super) bar_inset: f32,

    // -- keadaan runtime (tidak pernah disentuh diffing) -----------------
    /// Tepi atas sorotan seleksi, koordinat isi — springnya yang membuat
    /// seleksi *meluncur* dari baris ke baris, bukan berkedip pindah.
    sel_y: SpringValue<f32>,
    /// Kepekatan sorotan seleksi (0 = tidak ada yang terpilih).
    sel_alpha: SpringValue<f32>,
    /// Tepi atas sorotan hover.
    hover_y: SpringValue<f32>,
    /// Kepekatan sorotan hover.
    hover_alpha: SpringValue<f32>,
    /// Kepekatan sorotan "sedang ditekan".
    press_alpha: SpringValue<f32>,

    /// Baris di bawah penunjuk.
    hovered: Option<usize>,
    /// Baris yang sedang ditekan; aktivasi hanya sah bila dilepas di baris yang sama.
    pressed: Option<usize>,
    /// Sedang memegang fokus keyboard.
    focused: bool,
    /// Baris yang menunggu digulirkan ke dalam layar (dilayani [`super::sync`]).
    reveal: Option<usize>,
    /// Lebar isi dari layout terakhir.
    width: f32,
}

/// Spring sorotan baris.
///
/// **Dekoratif** dengan sengaja: yang membawa informasi adalah baris mana yang
/// terpilih, bukan perjalanan sorotannya. Karena itu di bawah reduced-motion
/// sorotan langsung berada di tempatnya — tidak meluncur, tidak memudar (§3.5).
fn sorotan_spring(spring: Spring) -> SpringValue<f32> {
    SpringValue::new(0.0).with_spring(spring).decorative()
}

impl ListBody {
    /// Node baru dari props yang sudah diresolusi.
    pub(super) fn from_props(props: &super::view::ListProps) -> Self {
        let mut node = Self {
            metrics: props.metrics,
            offset: props.offset,
            first: props.first,
            rows: props.rows,
            has_header: props.has_header,
            has_empty: props.has_empty,
            selectable: props.selectable,
            selected: props.selected,
            label: props.label.clone(),
            style: props.style,
            state: Some(props.state),
            on_activate: props.on_activate.clone(),
            bar_inset: props.bar_inset,
            sel_y: sorotan_spring(props.spring),
            sel_alpha: sorotan_spring(props.spring),
            hover_y: sorotan_spring(props.spring),
            hover_alpha: sorotan_spring(props.spring),
            press_alpha: sorotan_spring(props.spring),
            hovered: None,
            pressed: None,
            focused: false,
            reveal: None,
            width: 0.0,
        };
        // Daftar yang lahir dengan seleksi (state yang dipulihkan) **tidak**
        // menganimasikan sorotannya masuk: itu bukan gerakan, itu keadaan awal.
        node.pasang_seleksi(props.selected, false);
        node
    }

    /// Ukuran-ukuran daftar yang berlaku.
    pub fn metrics(&self) -> ListMetrics {
        self.metrics
    }

    /// Baris yang sedang terpilih.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Baris di bawah penunjuk.
    pub fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// Benar bila daftar memegang fokus keyboard.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// State yang dipakai daftar ini, bila ada.
    pub fn state(&self) -> Option<ListState> {
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

    /// Kotak baris `index` dalam **koordinat isi**.
    pub fn row_rect(&self, index: usize) -> Rect {
        Rect::new(
            0.0,
            self.metrics.row_top(index),
            self.width,
            self.metrics.extent,
        )
    }

    // -- animasi ----------------------------------------------------------

    /// Benar bila masih ada sorotan yang bergerak.
    pub fn is_animating(&self) -> bool {
        self.sel_y.is_animating()
            || self.sel_alpha.is_animating()
            || self.hover_y.is_animating()
            || self.hover_alpha.is_animating()
            || self.press_alpha.is_animating()
    }

    /// Majukan sorotan satu frame; benar bila ada piksel yang berubah.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let sebelum = (
            self.sel_y.position(),
            self.sel_alpha.position(),
            self.hover_y.position(),
            self.hover_alpha.position(),
            self.press_alpha.position(),
        );
        tick.advance(&mut self.sel_y);
        tick.advance(&mut self.sel_alpha);
        tick.advance(&mut self.hover_y);
        tick.advance(&mut self.hover_alpha);
        tick.advance(&mut self.press_alpha);
        sebelum
            != (
                self.sel_y.position(),
                self.sel_alpha.position(),
                self.hover_y.position(),
                self.hover_alpha.position(),
                self.press_alpha.position(),
            )
    }

    /// Selesaikan seluruh gerakan sorotan seketika (uji, snapshot).
    pub fn settle(&mut self) {
        self.sel_y.settle();
        self.sel_alpha.settle();
        self.hover_y.settle();
        self.hover_alpha.settle();
        self.press_alpha.settle();
    }

    /// Ganti spring seluruh sorotan tanpa mengganggu gerakan yang berjalan.
    pub fn set_spring(&mut self, spring: Spring) {
        self.sel_y.set_spring(spring);
        self.sel_alpha.set_spring(spring);
        self.hover_y.set_spring(spring);
        self.hover_alpha.set_spring(spring);
        self.press_alpha.set_spring(spring);
    }

    /// Spring yang menjalankan sorotan.
    pub fn spring(&self) -> Spring {
        self.sel_y.spring()
    }

    /// Arahkan sorotan seleksi ke `index`.
    ///
    /// `animasi` salah berarti sorotan langsung berada di tempatnya — dipakai
    /// saat node lahir dan saat seleksi berpindah karena datanya yang berubah,
    /// bukan karena pengguna.
    fn pasang_seleksi(&mut self, index: Option<usize>, animasi: bool) {
        let Some(i) = index else {
            self.sel_alpha.set_target(0.0);
            if !animasi {
                self.sel_alpha.settle();
            }
            return;
        };
        let y = self.metrics.row_top(i);
        // Sorotan yang baru muncul **tidak** meluncur dari baris lama: ia
        // memudar masuk di tempatnya. Yang meluncur hanya perpindahan antar
        // baris saat sorotannya memang sudah terlihat.
        if self.sel_alpha.position() <= 0.0 || !animasi {
            self.sel_y.jump_to(y);
        } else {
            self.sel_y.set_target(y);
        }
        self.sel_alpha.set_target(1.0);
        if !animasi {
            self.sel_y.jump_to(y);
            self.sel_alpha.settle();
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

    /// Setel seleksi di node **dan** terbitkan ke [`ListState`].
    pub(super) fn pilih(&mut self, index: Option<usize>, animasi: bool) -> bool {
        if self.selected == index {
            return false;
        }
        self.selected = index;
        self.pasang_seleksi(index, animasi);
        if let Some(state) = self.state {
            state.publish_selection(index);
        }
        true
    }

    /// Ambil permintaan "gulirkan baris ini ke layar" yang tertunda.
    ///
    /// Dilayani [`super::sync`], bukan di sini: yang bisa menggulir adalah
    /// [`crate::scroll_view::ScrollView`] di atas node ini, dan sebuah render
    /// node tidak boleh meraba leluhurnya dari dalam `event` (aturan "node
    /// hanya boleh mengubah dirinya sendiri", [`silka_core::tree`]).
    pub(super) fn take_reveal(&mut self) -> Option<usize> {
        self.reveal.take()
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
    /// daftar.
    ///
    /// Hit-test menelusuri anak lebih dulu (Flutter), jadi tanpa penjaga ini
    /// baris akan menelan setiap klik yang sebenarnya ditujukan ke thumb —
    /// dan scrollbar sebuah daftar menjadi hiasan yang tidak bisa diseret.
    fn di_jalur_scrollbar(&self, p: Point) -> bool {
        self.bar_inset > 0.0
            && self.metrics.max_scroll() > 0.0
            && p.x >= self.width - self.bar_inset
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
                if self.selectable {
                    ctx.request_focus();
                    self.pilih(Some(baris), true);
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
                // Finder, Mail, dan setiap daftar macOS.
                //
                // `== 2`, bukan `>= 2`: router menaikkan `click_count` terus
                // selama rentetan masih rapat (dua, tiga, empat…), jadi `>= 2`
                // akan memanggil `on_activate` sekali lagi di setiap ketukan
                // berikutnya. Membuka satu baris tiga kali karena pengguna
                // gugup adalah bug, bukan fitur.
                if ditekan == baris && p.click_count == 2 {
                    if let (Some(i), Some(aksi)) = (baris, self.on_activate.clone()) {
                        aksi.call(i);
                    }
                }
                ctx.request_animation();
                ctx.request_paint();
                ctx.handled();
            }
            // Dibatalkan OS ≠ dilepas: tidak ada aktivasi, hanya sorotan
            // tekan yang memudar pulang.
            PointerPhase::Cancel if self.pressed.take().is_some() => {
                self.press_alpha.set_target(0.0);
                ctx.request_animation();
                ctx.request_paint();
            }
            _ => {}
        }
    }

    fn tombol(&mut self, ctx: &mut EventCtx<'_>, k: &KeyEvent) {
        // Tanpa seleksi, panah/Page/Home/End bukan urusan daftar: mereka
        // **menggelembung** ke `scroll_view` di atasnya dan menggulir isi.
        if !self.selectable || !k.modifiers.is_empty() || self.metrics.count == 0 {
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
            c if c.is(NamedKey::Enter) || c.is(NamedKey::Space) => {
                let (Some(i), Some(aksi)) = (self.selected, self.on_activate.clone()) else {
                    return;
                };
                aksi.call(i);
                ctx.handled();
                return;
            }
            _ => None,
        };
        let Some(index) = tujuan else { return };
        self.pilih(Some(index), true);
        // Guliran ke baris terpilih dijalankan `sync`, yang memegang pohon.
        self.reveal = Some(index);
        ctx.request_animation();
        ctx.request_paint();
        ctx.handled();
    }

    /// Baris tujuan setelah bergeser `delta` langkah dari seleksi sekarang.
    fn langkah(&self, delta: isize) -> usize {
        let terakhir = (self.metrics.count - 1) as isize;
        match self.selected {
            // Tanpa seleksi, tekanan pertama mendarat di ujung yang searah.
            None if delta > 0 => 0,
            None => terakhir as usize,
            Some(i) => (i as isize + delta).clamp(0, terakhir) as usize,
        }
    }
}

impl RenderNode for ListBody {
    fn type_name(&self) -> &'static str {
        "ListBody"
    }

    /// Baris ditempatkan sendiri, jadi node ini menyerap penunjuk yang tidak
    /// diambil isinya — tombol di dalam baris tetap menang karena hit-test
    /// menelusuri anak lebih dulu.
    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    /// Daftar yang bisa dipilih adalah satu perhentian Tab (pola listbox AppKit
    /// dan ARIA); daftar tampilan murni menyerahkan Tab ke wadah gulirnya.
    fn focus_policy(&self) -> FocusPolicy {
        if self.selectable && self.metrics.count > 0 {
            FocusPolicy::FOCUSABLE
        } else {
            FocusPolicy::NONE
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
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
            // aplikasinya sendiri; sebelum layout pertama ia seukuran isinya.
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
        // pembungkus clip kedua (lihat `paint`).
        if self.has_header && idx < jumlah_anak {
            let anak = ctx.child(idx);
            let c = BoxConstraints::new(lebar, lebar, self.metrics.header, self.metrics.header);
            ctx.layout_child_boundary(anak, c);
            // Menempel = tetap di tepi atas jendela, yaitu tepat di posisi
            // guliran; tidak menempel = ikut tergulir keluar bersama isi.
            let atas = if self.metrics.sticky {
                self.offset
                    .clamp(0.0, (tinggi - self.metrics.header).max(0.0))
            } else {
                0.0
            };
            ctx.place_child(anak, Point::new(0.0, atas));
        }

        // Node ini setinggi **seluruh** isi walau hanya sepersekiannya yang
        // dimaterialisasi: itulah yang membuat scrollbar dan `max_scroll` di
        // atas sana benar tanpa harus tahu apa pun tentang virtualisasi.
        let size = Size::new(lebar, constraints.constrain_height(tinggi));
        if let Some(state) = self.state {
            state.publish_content(tinggi, self.metrics.extent, self.metrics.header);
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.style.decoration);

        // `PaintCtx` sudah membuang apa pun di luar clip wadah gulir, jadi
        // sorotan yang tergulir keluar tidak menghasilkan perintah sama sekali.
        let mut sorot = |y: f32, warna: Color, alpha: f32| {
            if alpha <= 0.0 || warna.a <= 0.0 {
                return;
            }
            ctx.quad(
                Quad::new(Rect::new(0.0, y, self.width, self.metrics.extent))
                    .background(warna.with_alpha(warna.a * alpha.clamp(0.0, 1.0)))
                    .corners(self.style.row_corners),
            );
        };
        if self.selectable {
            let hover = self.hover_alpha.position();
            if self.hovered != self.selected {
                sorot(self.hover_y.position(), self.style.hover, hover);
            }
            let warna = if self.focused {
                self.style.selection
            } else {
                self.style.selection_idle
            };
            sorot(self.sel_y.position(), warna, self.sel_alpha.position());
            if let Some(i) = self.pressed {
                sorot(
                    self.metrics.row_top(i),
                    self.style.pressed,
                    self.press_alpha.position(),
                );
            }
        }

        if self.style.separator_width > 0.0 && self.style.separator.a > 0.0 {
            // Garis hanya untuk baris yang dimaterialisasi: seratus ribu baris
            // tetap menghasilkan belasan perintah gambar.
            for i in self.first.max(1)..(self.first + self.rows).min(self.metrics.count) {
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

        ctx.paint_children();

        // Cincin fokus digambar **di atas** isi baris dan di dalam kotak
        // barisnya: daftar yang terfokus harus terbaca walau seluruh baris
        // sudah berlatar warna seleksi.
        if self.focused && self.sel_alpha.position() > 0.0 {
            if let Some(ring) = self
                .style
                .focus_ring
                .filter(|r| r.width > 0.0 && r.color.a > 0.0)
            {
                let kotak = Rect::new(0.0, self.sel_y.position(), self.width, self.metrics.extent)
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
        node.role = AccessRole::List;
        node.label.clone_from(&self.label);
        if self.selectable && self.metrics.count > 0 {
            node.actions |= AccessActions::FOCUS;
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            Event::Pointer(p) => self.penunjuk(ctx, p),
            Event::Key(k) if k.is_pressed() => self.tombol(ctx, k),
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                // Daftar yang baru menerima fokus tanpa seleksi tidak punya
                // tempat untuk cincin fokusnya — dan pengguna keyboard tidak
                // punya petunjuk di mana ia berada. Kebiasaan AppKit: baris
                // pertama yang terlihat menjadi titik mulai.
                if self.focused
                    && self.selectable
                    && self.metrics.count > 0
                    && self.selected.is_none()
                {
                    let pertama = self.metrics.index_at(self.offset).unwrap_or(0);
                    self.pilih(Some(pertama), false);
                    self.reveal = Some(pertama);
                }
                ctx.request_animation();
                ctx.request_paint();
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for ListBody {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ListBody")
            .field("count", &self.metrics.count)
            .field("first", &self.first)
            .field("rows", &self.rows)
            .field("offset", &self.offset)
            .field("selected", &self.selected)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// ListRowBox
// ---------------------------------------------------------------------------

/// Node satu baris: transparan bagi layout, **berarti** bagi screen reader.
///
/// Ia tidak menggambar apa pun — sorotan seleksi milik [`ListBody`], yang tahu
/// geometri seluruh daftar — dan tidak mengubah ukuran apa pun. Yang ia
/// tambahkan hanya satu hal, dan hal itu wajib: peran `ListItem` beserta
/// keadaan terpilihnya, sehingga daftar dibaca teknologi bantu sebagai daftar,
/// bukan sebagai tumpukan kotak (§3.8).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListRowBox {
    /// Nomor baris ini di dalam data (bukan di dalam jendela).
    pub index: usize,
    /// Terpilih atau tidak; `None` = daftar ini memang tidak punya seleksi.
    pub selected: Option<bool>,
    /// Baris ini bisa diaktifkan (ketuk-ganda / Enter).
    pub activatable: bool,
}

impl RenderNode for ListRowBox {
    fn type_name(&self) -> &'static str {
        "ListRow"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(size)
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::ListItem;
        node.selected = self.selected;
        if self.activatable {
            node.actions |= AccessActions::CLICK;
        }
    }
}
