//! Deretan tab: penempatan, indikator ber-spring, keyboard, dan a11y.
//!
//! Node ini yang **memiliki** semua keputusan yang tidak bisa diambil satu tab
//! sendirian:
//!
//! - **Penempatan.** Tab diletakkan berurutan mengikuti arah baca (§9.8), dan
//!   varian [`Segmented`](super::TabsVariant::Segmented) menyamakan lebarnya seperti
//!   `NSSegmentedControl`. Semua tab menerima tinggi yang sama, minimal
//!   [`MIN_HIT_TARGET`](crate::MIN_HIT_TARGET) (HIG).
//! - **Indikator.** Satu [`SpringValue<Rect>`] berisi kotak tab yang sedang
//!   dipilih; bentuk yang digambar diturunkan darinya lewat
//!   [`TabsStyle::indicator_rect`]. Karena yang di-spring adalah **kotaknya**,
//!   thumb segmented dan garis underline memakai gerakan yang sama persis, dan
//!   pilihan yang berubah di tengah animasi **membawa kecepatannya** (§3.5).
//! - **Keyboard.** Satu deretan = satu perhentian Tab; di dalamnya panah
//!   kiri/kanan memindahkan pilihan (dicerminkan di RTL), Home/End melompat ke
//!   ujung, dan tab yang dimatikan dilewati. Cincin fokus digambar mengelilingi
//!   tab aktif, jadi ia ikut meluncur bersama indikator.

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    Event, EventCtx, FocusEvent, FocusPolicy, KeyCode, NamedKey, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::ViewNode;
use silka_paint::{CornerRadii, Corners, Insets, Quad, Rect, Size};

use super::style::TabsStyle;

// ---------------------------------------------------------------------------
// OnSelect
// ---------------------------------------------------------------------------

/// Aksi "tab ke-`index` dipilih" yang dititipkan aplikasi.
///
/// Sepupu [`silka_core::Callback`] yang membawa satu argumen; sifatnya sama
/// persis: `Clone` murah, `PartialEq` berdasarkan identitas, dan yang boleh
/// dilakukannya hanyalah **menulis signal** — struktur pohon adalah wewenang
/// view-diff (§2.5).
#[derive(Clone)]
pub struct OnSelect(std::rc::Rc<dyn Fn(usize)>);

impl OnSelect {
    /// Bungkus sebuah closure.
    pub fn new(f: impl Fn(usize) + 'static) -> Self {
        Self(std::rc::Rc::new(f))
    }

    /// Jalankan aksinya untuk tab ke-`index`.
    pub fn call(&self, index: usize) {
        (self.0)(index)
    }
}

impl PartialEq for OnSelect {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for OnSelect {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OnSelect")
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// Node render satu deretan tab.
pub struct TabListBox {
    /// Nilai visual yang sudah diresolusi dari token.
    pub style: TabsStyle,
    /// Indeks tab yang sedang aktif.
    pub selected: usize,
    /// Nama deretan bagi screen reader ("Bagian pengaturan").
    pub label: Option<String>,
    /// Apa yang dijalankan saat pengguna memilih tab lain.
    pub on_select: Option<OnSelect>,
    /// Tab mana yang masih bisa dipilih — panjangnya = jumlah tab.
    pub enabled: Vec<bool>,

    /// Kotak tab aktif; bentuk indikator diturunkan darinya saat menggambar.
    indicator: SpringValue<Rect>,
    /// Kotak setiap tab dari layout terakhir (koordinat lokal).
    placed: Vec<Rect>,
    /// Sudah pernah ada layout yang mengisi [`TabListBox::placed`].
    ready: bool,
    /// Sedang memegang fokus keyboard.
    focused: bool,
    /// Arah baca dari layout terakhir — panah kiri/kanan dicerminkan (§9.8).
    rtl: bool,
    /// Benar begitu ada yang pernah memanggil [`TabListBox::advance`].
    driven: bool,
}

impl TabListBox {
    /// Kotak indikator yang digambar frame ini (koordinat lokal).
    pub fn indicator_rect(&self) -> Rect {
        self.style.indicator_rect(self.indicator.position())
    }

    /// Kotak tab aktif yang sedang dianimasikan.
    pub fn active_rect(&self) -> Rect {
        self.indicator.position()
    }

    /// Kotak setiap tab dari layout terakhir.
    pub fn tab_rects(&self) -> &[Rect] {
        &self.placed
    }

    /// Benar bila indikatornya masih bergerak.
    pub fn is_animating(&self) -> bool {
        self.indicator.is_animating()
    }

    /// Sedang memegang fokus keyboard.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Spring yang menjalankan indikator.
    pub fn spring(&self) -> Spring {
        self.indicator.spring()
    }

    /// Jumlah tab.
    pub fn len(&self) -> usize {
        self.enabled.len()
    }

    /// Benar bila tidak ada tab sama sekali.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_empty()
    }

    /// Benar bila tab ke-`index` masih bisa dipilih.
    pub fn is_enabled(&self, index: usize) -> bool {
        self.enabled.get(index).copied().unwrap_or(false)
    }

    /// Arahkan indikator ke `kotak`.
    ///
    /// Tanpa penggerak frame (lihat [`super`]) transisi menjadi lompatan: lebih
    /// baik indikator langsung benar daripada membeku di posisi lama selamanya.
    fn arahkan(&mut self, kotak: Rect) {
        if self.driven {
            self.indicator.set_target(kotak);
        } else {
            self.indicator.jump_to(kotak);
        }
    }

    /// Pindahkan pilihan ke `index` — **retarget**, bukan animasi baru.
    pub fn set_selected(&mut self, index: usize) {
        if self.selected == index {
            return;
        }
        self.selected = index;
        if let Some(kotak) = self.placed.get(index).copied() {
            self.arahkan(kotak);
        }
    }

    /// Majukan indikator satu frame; benar bila kotaknya berubah.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        self.driven = true;
        if !self.indicator.is_animating() {
            return false;
        }
        let sebelum = self.indicator.position();
        tick.advance(&mut self.indicator);
        self.indicator.position() != sebelum
    }

    /// Selesaikan transisi seketika (uji dan snapshot).
    pub fn settle(&mut self) {
        self.indicator.settle();
    }

    /// Tab enabled berikutnya dari `dari` ke arah `langkah`, tanpa melingkar.
    ///
    /// Tidak melingkar karena itulah kebiasaan `NSSegmentedControl`: panah
    /// kanan di tab terakhir **tidak** melompat kembali ke tab pertama, jadi
    /// pengguna keyboard tidak pernah kehilangan jejak posisinya.
    pub fn tetangga(&self, dari: usize, langkah: i32) -> Option<usize> {
        let n = self.enabled.len();
        if n == 0 {
            return None;
        }
        let mut i = dari as i32;
        loop {
            i += langkah;
            if i < 0 || i >= n as i32 {
                return None;
            }
            if self.enabled[i as usize] {
                return Some(i as usize);
            }
        }
    }

    /// Tab enabled pertama (`langkah` positif) atau terakhir (negatif).
    pub fn ujung(&self, langkah: i32) -> Option<usize> {
        let n = self.enabled.len();
        if n == 0 {
            return None;
        }
        if langkah >= 0 {
            (0..n).find(|i| self.enabled[*i])
        } else {
            (0..n).rev().find(|i| self.enabled[*i])
        }
    }

    /// Minta pilihan berpindah ke `index`; benar bila ada yang dijalankan.
    ///
    /// Node **tidak** memindahkan pilihannya sendiri: `selected` datang dari
    /// aplikasi lewat props (komponen terkendali), persis seperti `open` pada
    /// [`crate::overlay::OverlayEntry`]. Yang dilakukan di sini hanya memanggil
    /// callback; frame berikutnya membawa pilihan barunya kembali ke sini.
    pub fn request_select(&mut self, index: usize) -> bool {
        if index == self.selected || !self.is_enabled(index) {
            return false;
        }
        let Some(cb) = self.on_select.clone() else {
            return false;
        };
        cb.call(index);
        true
    }
}

impl RenderNode for TabListBox {
    fn type_name(&self) -> &'static str {
        "TabList"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let n = ctx.child_count();
        if n == 0 {
            self.placed.clear();
            return constraints.smallest();
        }

        let pad = self.style.padding;
        let dalam = constraints.deflate(pad).loosen();

        // Pass 1 — "kamu maunya sebesar apa?". Tinggi minimum sudah dipaksa di
        // sini supaya hit target HIG tidak bergantung pada isi label.
        let ukur = BoxConstraints::new(
            0.0,
            dalam.max_width,
            self.style.min_height,
            dalam.max_height,
        );
        let mut lebar = Vec::with_capacity(n);
        let mut tinggi = self.style.min_height;
        for i in 0..n {
            let anak = ctx.child(i);
            let s = ctx.layout_child_measured(anak, ukur);
            lebar.push(s.width);
            tinggi = tinggi.max(s.height);
        }
        if self.style.equal_widths {
            let terlebar = lebar.iter().copied().fold(0.0f32, f32::max);
            lebar.iter_mut().for_each(|w| *w = terlebar);
        }

        let jarak = self.style.spacing * (n - 1) as f32;
        let isi_lebar: f32 = lebar.iter().sum::<f32>() + jarak;
        let size = constraints.constrain(Size::new(
            isi_lebar + pad.horizontal(),
            tinggi + pad.vertical(),
        ));
        let tinggi_isi = (size.height - pad.vertical()).max(0.0);

        // Pass 2 — setiap tab menerima kotaknya. Constraints tight di sini
        // **berasal dari hasil mengukur anak itu sendiri**, jadi ia tidak boleh
        // menjadi relayout boundary (alasan yang sama seperti `TaffyBox`).
        self.placed.clear();
        let mut x = pad.left;
        for (i, w) in lebar.iter().copied().enumerate() {
            let anak = ctx.child(i);
            ctx.layout_child_measured(anak, BoxConstraints::tight(Size::new(w, tinggi_isi)));
            // Mengikuti arah baca: di RTL tab pertama berada di kanan (§9.8).
            let kiri = if self.rtl { size.width - x - w } else { x };
            let kotak = Rect::new(kiri, pad.top, w, tinggi_isi);
            ctx.place_child(anak, kotak.origin);
            self.placed.push(kotak);
            x += w + self.style.spacing;
        }

        // Indikator disinkronkan ke geometri terbaru. Kalau ia sedang bergerak,
        // yang berubah cuma tujuannya — pilihan yang berganti di tengah animasi
        // membawa kecepatannya (§3.5). Kalau ia diam (mis. window di-resize),
        // ia ikut pindah tanpa animasi: bukan pilihan yang berubah, hanya
        // kotaknya.
        let aktif = self
            .placed
            .get(self.selected.min(self.placed.len().saturating_sub(1)))
            .copied();
        if let Some(kotak) = aktif {
            if !self.ready {
                // Deretan yang baru lahir tidak "meluncur masuk" dari sudut
                // kiri-atas: tab aktif memang sudah di sana.
                self.indicator.jump_to(kotak);
                self.ready = true;
            } else if self.indicator.is_animating() {
                self.indicator.set_target(kotak);
            } else {
                self.indicator.jump_to(kotak);
            }
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.style.track);

        // Garis rambut selebar deretan (underline & enclosed) — digambar
        // sebelum indikator supaya tab enclosed yang aktif menutupinya.
        if let Some(warna) = self.style.rail.filter(|c| c.a > 0.0) {
            let t = self.style.rail_thickness.max(0.0);
            if t > 0.0 {
                let b = ctx.local_bounds();
                ctx.quad(
                    Quad::new(Rect::new(b.min_x(), b.max_y() - t, b.size.width, t))
                        .background(warna),
                );
            }
        }

        if self.ready && self.style.indicator_is_visible() && !self.placed.is_empty() {
            let d = self.style.indicator;
            let kotak = self.indicator_rect();
            ctx.shadowed(
                Quad::new(kotak)
                    .background(d.background)
                    .corners(d.corners)
                    .border(d.border_width, d.border_color),
                d.shadows,
            );
        }

        ctx.paint_children();

        // Cincin fokus mengelilingi **tab aktif**, bukan seluruh deretan: ia
        // ikut meluncur bersama indikator, jadi keyboard selalu menunjuk ke
        // tempat yang sama dengan mata.
        if self.focused && self.ready && !self.placed.is_empty() {
            let ring = self.style.focus_ring;
            if ring.width > 0.0 && ring.color.a > 0.0 {
                let kotak = self.active_rect().deflate(Insets::all(-ring.width));
                let corners = Corners::new(
                    CornerRadii::all(self.style.tab_corners.radii.max() + ring.width),
                    self.style.tab_corners.style,
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
        node.role = AccessRole::TabList;
        node.label.clone_from(&self.label);
        if self.ujung(1).is_some() {
            node.actions |= AccessActions::FOCUS;
        }
    }

    /// Satu deretan = **satu** perhentian Tab; di dalamnya panah yang bekerja.
    fn focus_policy(&self) -> FocusPolicy {
        if self.ujung(1).is_some() {
            FocusPolicy::FOCUSABLE
        } else {
            FocusPolicy::NONE
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            // Klik pada salah satu tab menggelembung sampai ke sini (tab
            // sengaja tidak menandainya handled): fokus milik deretan, jadi
            // di sinilah ia diminta — persis seperti `NSSegmentedControl`,
            // yang setelah diklik langsung bisa dipakai dengan panah.
            Event::Pointer(p)
                if p.phase == PointerPhase::Down
                    && p.button == Some(PointerButton::Primary)
                    && self.ujung(1).is_some() =>
            {
                ctx.request_focus();
                ctx.handled();
            }
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                ctx.request_paint();
            }
            Event::Key(k) if k.is_pressed() && k.modifiers.is_empty() => {
                // Arah baca ikut menentukan arti panah: di RTL, "kanan" berarti
                // tab sebelumnya (§9.8).
                let maju = if self.rtl { -1 } else { 1 };
                let tujuan = match k.code {
                    KeyCode::Named(NamedKey::ArrowRight) => self.tetangga(self.selected, maju),
                    KeyCode::Named(NamedKey::ArrowLeft) => self.tetangga(self.selected, -maju),
                    // Panah atas/bawah sengaja tidak dipakai: deretan ini
                    // horizontal, dan menelan panah vertikal akan merampas
                    // guliran halaman di belakangnya.
                    KeyCode::Named(NamedKey::Home) => self.ujung(1),
                    KeyCode::Named(NamedKey::End) => self.ujung(-1),
                    _ => return,
                };
                // Panah di ujung tetap "handled": tanpa itu, Home di tab
                // pertama akan menggelembung dan menggulir halaman.
                ctx.handled();
                ctx.request_paint();
                if let Some(i) = tujuan {
                    self.request_select(i);
                }
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for TabListBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TabListBox")
            .field("variant", &self.style.variant)
            .field("selected", &self.selected)
            .field("tabs", &self.enabled.len())
            .field("indicator", &self.indicator.position())
            .field("focused", &self.focused)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props deretan tab — bentuk view dari [`TabListBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct TabListProps {
    pub(super) style: TabsStyle,
    pub(super) selected: usize,
    pub(super) label: Option<String>,
    pub(super) on_select: Option<OnSelect>,
    pub(super) enabled: Vec<bool>,
    pub(super) spring: Spring,
}

impl ViewNode for TabListProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TabListBox {
            style: self.style,
            selected: self.selected,
            label: self.label.clone(),
            on_select: self.on_select.clone(),
            enabled: self.enabled.clone(),
            indicator: SpringValue::new(Rect::new(0.0, 0.0, 0.0, 0.0)).with_spring(self.spring),
            placed: Vec::new(),
            ready: false,
            focused: false,
            rtl: false,
            driven: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TabListBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.enabled != self.enabled {
            n.enabled.clone_from(&self.enabled);
            dirty |= Dirty::PAINT;
        }
        if n.selected != self.selected {
            n.set_selected(self.selected);
            // Indikator bergeser: butuh gambar ulang **dan** frame berikutnya
            // selama springnya belum settle.
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.indicator.spring() != self.spring {
            n.indicator.set_spring(self.spring);
        }
        // Callback selalu diganti tanpa dibandingkan (lihat `InteractiveProps`).
        n.on_select.clone_from(&self.on_select);
        dirty
    }
}
