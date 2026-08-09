//! Pemicu select: kotak yang menampilkan pilihan sekarang, dan **satu-satunya
//! yang memegang fokus keyboard** selama popup terbuka.
//!
//! Kenapa fokus tidak pindah ke popup: itu justru yang dilakukan NSPopUpButton
//! dan `<select>` — panah, Home/End, Enter, Esc, dan typeahead semuanya sampai
//! ke kontrolnya, sementara menunya cuma menggambar. Konsekuensi praktisnya
//! besar: tidak ada perangkap fokus yang harus dipasang dan dilepas, tidak ada
//! "fokus otomatis ke panel yang baru terbuka" (kait yang memang belum ada,
//! lihat [`crate::overlay`]), dan tidak ada satu pun keystroke yang hilang di
//! antara dua frame.
//!
//! Empat gerakan node ini dan perannya terhadap reduced-motion:
//!
//! | Gerakan | Spring | Peran | Alasan |
//! |---|---|---|---|
//! | Latar hover/press/disabled | `snappy` | Essential | Menjelaskan keadaan kontrol |
//! | Cincin fokus tumbuh | `smooth` | Essential | Menjelaskan di mana fokus keyboard |
//! | Segitiga membalik saat popup buka/tutup | `snappy` | Essential | Menjelaskan popup terbuka |

use std::rc::Rc;
use std::time::Duration;

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode,
    Modifiers, NamedKey, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::ViewNode;
use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, ShadowPair, Size};

use super::{SelectHandler, SelectIntent};

/// Jumlah bilah penyusun segitiga penunjuk.
///
/// Lapisan paint hanya mengenal kotak, glyph, dan bayangan (§3.2) — tidak ada
/// perintah path dan tidak ada rotasi. Segitiga karena itu disusun dari bilah
/// horizontal yang menyempit; lima sudah cukup halus pada ukuran 8pt, dan
/// **membalikkan urutan lebarnya** adalah animasi buka/tutupnya.
const BILAH: usize = 5;

/// Jeda maksimum antar ketikan typeahead sebelum buffernya dilupakan.
const JEDA_KETIK: Duration = Duration::from_millis(900);

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Seluruh nilai gambar pemicu select, **sudah diresolusi** dari token theme.
///
/// Mesin tidak pernah punya pendapat tentang warna (§2.6, §2.7): preset
/// Cupertino dan Tailwind berganti dengan mengisi struct ini, tanpa satu baris
/// pun berubah di [`SelectTrigger`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectTriggerStyle {
    /// Latar keadaan diam.
    pub rest: Color,
    /// Latar saat penunjuk di atasnya.
    pub hover: Color,
    /// Latar saat ditekan (dan saat popup terbuka).
    pub pressed: Color,
    /// Latar saat tidak bisa dipakai.
    pub disabled: Color,
    /// Geometri sudut — sekaligus bentuk area sentuh (§3.6).
    pub corners: Corners,
    /// Tebal border (0 = tanpa border).
    pub border_width: f32,
    /// Warna border saat aktif.
    pub border: Color,
    /// Warna border saat mati.
    pub border_disabled: Color,
    /// Bayangan ganda ala HIG.
    pub shadows: ShadowPair,
    /// Tebal cincin fokus keyboard.
    pub focus_ring_width: f32,
    /// Warna cincin fokus.
    pub focus_ring: Color,
    /// Jarak isi ke tepi kotak.
    pub padding: Insets,
    /// Jarak antara label dan segitiga penunjuk.
    pub gap: f32,
    /// Lebar segitiga penunjuk.
    pub indicator: f32,
    /// Warna segitiga penunjuk.
    pub indicator_color: Color,
    /// Lebar minimum kotak (diukur dari pilihan terpanjang).
    pub min_width: f32,
    /// Tinggi minimum kotak — hit target HIG.
    pub min_height: f32,
}

impl SelectTriggerStyle {
    /// Latar yang seharusnya berlaku untuk kombinasi keadaan ini.
    ///
    /// Inilah **target** spring; yang digambar adalah posisinya, bukan ini.
    pub fn background_for(
        &self,
        hovered: bool,
        pressed: bool,
        open: bool,
        disabled: bool,
    ) -> Color {
        if disabled {
            return self.disabled;
        }
        // `pressed` bertahan saat penunjuk ditangkap keluar kotak, tapi tampilan
        // "ditekan" hanya berlaku selama penunjuknya masih di dalam — persis
        // AppKit/UIKit. Popup yang terbuka membuat kontrolnya tetap terlihat
        // aktif walau penunjuk sudah pergi ke daftarnya.
        if (pressed && hovered) || open {
            self.pressed
        } else if hovered {
            self.hover
        } else {
            self.rest
        }
    }

    /// Warna border yang berlaku.
    pub fn border_for(&self, disabled: bool) -> Color {
        if disabled {
            self.border_disabled
        } else {
            self.border
        }
    }

    /// Jarak isi ke tepi, sudah memperhitungkan ruang segitiga penunjuk.
    ///
    /// Sisi mana yang melebar mengikuti arah baca (§9.8): penunjuk selalu di
    /// **akhir** baris, jadi di RTL ia pindah ke kiri tanpa satu pun nilai
    /// dihitung ulang di lapisan view.
    pub fn insets(&self, rtl: bool) -> Insets {
        let ruang = self.gap + self.indicator;
        let mut i = self.padding;
        if rtl {
            i.left += ruang;
        } else {
            i.right += ruang;
        }
        i
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// Node render pemicu select.
pub struct SelectTrigger {
    style: SelectTriggerStyle,
    label: Option<String>,
    value: Option<String>,
    options: Rc<Vec<String>>,
    open: bool,
    highlight: usize,
    disabled: bool,
    focus: FocusPolicy,
    on_intent: Option<SelectHandler>,

    /// Latar yang benar-benar digambar frame ini.
    bg: SpringValue<Color>,
    /// 0 = tanpa cincin fokus, 1 = cincin penuh.
    ring_t: SpringValue<f32>,
    /// 0 = segitiga menunjuk ke bawah, 1 = ke atas.
    open_t: SpringValue<f32>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    rtl: bool,

    /// Buffer typeahead dan kapan huruf terakhir masuk.
    ketikan: String,
    ketikan_pada: Duration,
}

impl SelectTrigger {
    fn new(props: &SelectTriggerProps) -> Self {
        let bg = props
            .style
            .background_for(false, false, props.open, props.disabled);
        Self {
            bg: SpringValue::new(bg).with_spring(props.spring),
            ring_t: SpringValue::new(0.0).with_spring(Spring::smooth()),
            // Select yang lahir dalam keadaan terbuka tidak beranimasi masuk:
            // ia memang **sudah** terbuka, bukan baru saja dibuka.
            open_t: SpringValue::new(if props.open { 1.0 } else { 0.0 }).with_spring(props.spring),
            style: props.style,
            label: props.label.clone(),
            value: props.value.clone(),
            options: props.options.clone(),
            open: props.open,
            highlight: props.highlight,
            disabled: props.disabled,
            focus: props.focus,
            on_intent: props.on_intent.clone(),
            hovered: false,
            pressed: false,
            focused: false,
            rtl: false,
            ketikan: String::new(),
            ketikan_pada: Duration::ZERO,
        }
    }

    /// Nilai gambar yang sedang berlaku.
    pub fn style(&self) -> SelectTriggerStyle {
        self.style
    }

    /// Latar yang digambar frame ini — posisi spring, bukan targetnya.
    pub fn background(&self) -> Color {
        self.bg.position()
    }

    /// Target latar yang sedang dituju spring.
    pub fn background_target(&self) -> Color {
        self.bg.target()
    }

    /// Kemajuan cincin fokus 0..1.
    pub fn focus_progress(&self) -> f32 {
        self.ring_t.position()
    }

    /// Kemajuan buka 0..1 (arah segitiga penunjuk).
    pub fn open_progress(&self) -> f32 {
        self.open_t.position()
    }

    /// Popup sedang terbuka menurut props terakhir.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Penunjuk sedang di atasnya.
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// Sedang ditekan.
    pub fn is_pressed(&self) -> bool {
        self.pressed
    }

    /// Sedang memegang fokus keyboard.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Indeks yang sedang disorot.
    pub fn highlight(&self) -> usize {
        self.highlight
    }

    /// Benar bila masih ada spring yang bergerak.
    pub fn is_animating(&self) -> bool {
        self.bg.is_animating() || self.ring_t.is_animating() || self.open_t.is_animating()
    }

    /// Arahkan seluruh spring ke keadaan sekarang.
    ///
    /// **Retarget, bukan animasi baru** (§3.5): kontrol yang dilepas di tengah
    /// animasi tekan berbalik arah membawa kecepatannya.
    fn retarget(&mut self) {
        self.bg.set_target(self.style.background_for(
            self.hovered,
            self.pressed,
            self.open,
            self.disabled,
        ));
        self.ring_t.set_target(if self.focused && !self.disabled {
            1.0
        } else {
            0.0
        });
        self.open_t.set_target(if self.open { 1.0 } else { 0.0 });
    }

    /// Majukan seluruh spring satu frame; benar bila ada yang bergeser.
    ///
    /// Dipanggil [`crate::motion::advance`], satu tempat untuk seluruh pohon.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let mut bergeser = false;
        let bg0 = self.bg.position();
        tick.advance(&mut self.bg);
        bergeser |= self.bg.position() != bg0;

        let r0 = self.ring_t.position();
        tick.advance(&mut self.ring_t);
        bergeser |= self.ring_t.position() != r0;

        let o0 = self.open_t.position();
        tick.advance(&mut self.open_t);
        bergeser |= self.open_t.position() != o0;
        bergeser
    }

    /// Selesaikan seluruh gerakan seketika (uji, snapshot, reduced-motion).
    pub fn settle(&mut self) {
        self.bg.settle();
        self.ring_t.settle();
        self.open_t.settle();
    }

    /// Kirim satu niat ke aplikasi.
    ///
    /// Handler-nya **disalin keluar dulu**: ia hampir selalu menulis signal, dan
    /// tulisan signal boleh memicu apa saja — yang tidak boleh adalah ia
    /// berjalan sambil node ini masih dipinjam `&mut`.
    fn kirim(&mut self, intent: SelectIntent) {
        if let Some(h) = self.on_intent.clone() {
            h.emit(intent);
        }
    }

    /// Geser sorotan `delta` langkah, dijepit ke rentang yang sah.
    ///
    /// Sorotannya ikut disimpan di node, bukan hanya dikirim: dua tombol panah
    /// yang datang sebelum frame berikutnya harus menghasilkan dua langkah,
    /// bukan dua kali langkah yang sama.
    fn geser_sorotan(&mut self, delta: i32) {
        let n = self.options.len();
        if n == 0 {
            return;
        }
        let baru = (self.highlight as i64 + delta as i64).clamp(0, n as i64 - 1) as usize;
        self.sorot(baru);
    }

    fn sorot(&mut self, index: usize) {
        let n = self.options.len();
        if n == 0 {
            return;
        }
        let index = index.min(n - 1);
        self.highlight = index;
        self.kirim(SelectIntent::Highlight(index));
    }

    /// Cari pilihan yang cocok dengan huruf yang baru diketik.
    ///
    /// Aturannya sama dengan menu native: huruf berturut-turut menumpuk menjadi
    /// satu awalan selama jedanya pendek, dan awalan yang tidak cocok jatuh
    /// kembali ke satu huruf terakhir alih-alih diam saja.
    fn typeahead(&mut self, c: char, waktu: Duration) -> Option<usize> {
        if c.is_control() {
            return None;
        }
        if waktu.saturating_sub(self.ketikan_pada) > JEDA_KETIK {
            self.ketikan.clear();
        }
        self.ketikan_pada = waktu;
        self.ketikan.extend(c.to_lowercase());
        if let Some(i) = cari_awalan(&self.options, &self.ketikan) {
            return Some(i);
        }
        if self.ketikan.chars().count() > 1 {
            self.ketikan.clear();
            self.ketikan.extend(c.to_lowercase());
            return cari_awalan(&self.options, &self.ketikan);
        }
        None
    }

    /// Kotak segitiga penunjuk dalam koordinat lokal.
    pub fn indicator_rect(&self, bounds: Rect) -> Rect {
        let w = self.style.indicator.max(0.0);
        let h = w * 0.5;
        let x = if self.rtl {
            self.style.padding.left
        } else {
            bounds.size.width - self.style.padding.right - w
        };
        Rect::new(x, bounds.center().y - h / 2.0, w, h)
    }
}

/// Indeks pilihan pertama yang diawali `awalan` (tanpa peduli besar-kecil).
///
/// Fungsi murni, jadi typeahead bisa diuji tanpa satu pun event.
pub fn cari_awalan(options: &[String], awalan: &str) -> Option<usize> {
    if awalan.is_empty() {
        return None;
    }
    options
        .iter()
        .position(|o| o.to_lowercase().starts_with(awalan))
}

/// Lebar bilah ke-`i` segitiga penunjuk pada kemajuan buka `progress`.
///
/// Fungsi murni: pada `progress` 0 bilah teratas paling lebar (menunjuk ke
/// bawah), pada 1 kebalikannya (menunjuk ke atas).
pub fn bar_width(width: f32, index: usize, progress: f32) -> f32 {
    let t = if BILAH > 1 {
        index as f32 / (BILAH - 1) as f32
    } else {
        0.0
    };
    let p = progress.clamp(0.0, 1.0);
    width * ((1.0 - t) * (1.0 - p) + t * p)
}

impl RenderNode for SelectTrigger {
    fn type_name(&self) -> &'static str {
        "SelectTrigger"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let insets = self.style.insets(self.rtl);
        if ctx.child_count() == 0 {
            return constraints.constrain(Size::new(self.style.min_width, self.style.min_height));
        }
        let child = ctx.child(0);
        let isi = ctx.layout_child(child, constraints.deflate(insets).loosen());
        let size = constraints.constrain(Size::new(
            (isi.width + insets.horizontal()).max(self.style.min_width),
            (isi.height + insets.vertical()).max(self.style.min_height),
        ));
        // Label rata ke arah awal baris, dan tetap di tengah secara vertikal
        // walau kotaknya dipaksa setinggi hit target HIG.
        let x = if self.rtl {
            (size.width - insets.right - isi.width).max(insets.left)
        } else {
            insets.left
        };
        let y = ((size.height - isi.height) / 2.0).max(0.0);
        ctx.place_child(child, Point::new(x, y));
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let bg = self.bg.position();
        let border = self.style.border_for(self.disabled);
        let ada_border = self.style.border_width > 0.0 && border.a > 0.0;
        if bg.a > 0.0 || ada_border || self.style.shadows.is_visible() {
            let quad = Quad::new(bounds)
                .background(bg)
                .corners(self.style.corners)
                .border(self.style.border_width, border);
            ctx.shadowed(quad, self.style.shadows);
        }

        // Cincin fokus digambar **di luar** kotak node supaya tidak menutupi
        // label (kebiasaan AppKit), dan tumbuh lewat spring.
        let ring = self.ring_t.position().clamp(0.0, 1.0);
        let tebal = self.style.focus_ring_width * ring;
        if tebal > 0.0 && self.style.focus_ring.a > 0.0 {
            let luar = bounds.deflate(Insets::all(-tebal));
            let corners = Corners::new(
                CornerRadii::all(self.style.corners.radii.max() + tebal),
                self.style.corners.style,
            );
            ctx.quad(
                Quad::new(luar).corners(corners).border(
                    tebal,
                    self.style
                        .focus_ring
                        .with_alpha(self.style.focus_ring.a * ring),
                ),
            );
        }

        ctx.paint_children();

        // Segitiga penunjuk: membalik arah lewat spring saat popup buka/tutup.
        let kotak = self.indicator_rect(bounds);
        let warna = self.style.indicator_color;
        if warna.a > 0.0 && kotak.size.width > 0.0 {
            let p = self.open_t.position();
            let tinggi_bilah = kotak.size.height / BILAH as f32;
            let bentuk = Corners::uniform(tinggi_bilah / 2.0, self.style.corners.style);
            for i in 0..BILAH {
                let w = bar_width(kotak.size.width, i, p);
                if w < 0.5 {
                    continue;
                }
                let x = kotak.min_x() + (kotak.size.width - w) / 2.0;
                let y = kotak.min_y() + i as f32 * tinggi_bilah;
                ctx.quad(
                    Quad::new(Rect::new(x, y, w, tinggi_bilah))
                        .background(warna)
                        .corners(bentuk),
                );
            }
        }
    }

    /// Peran `Button` dengan nilai = pilihan sekarang.
    ///
    /// Inilah pemetaan pop-up button macOS (`AXPopUpButton` = tombol yang punya
    /// menu): namanya dibacakan sekali dari sini, nilainya adalah teks pilihan
    /// yang tampil, dan aksi `Expand`/`Collapse` mengumumkan bahwa ia punya
    /// daftar yang bisa dibuka — kosakata yang sudah disediakan
    /// [`AccessActions`] justru untuk kasus ini.
    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Button;
        node.label.clone_from(&self.label);
        node.value.clone_from(&self.value);
        node.disabled = self.disabled;
        if !self.disabled {
            node.actions |= AccessActions::CLICK;
            node.actions |= if self.open {
                AccessActions::COLLAPSE
            } else {
                AccessActions::EXPAND
            };
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.style.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Kontrol mati tetap **menyerap** penunjuk: kliknya tidak boleh menembus
        // ke konten di belakangnya.
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.disabled {
            FocusPolicy::NONE
        } else {
            self.focus
        }
    }

    fn cursor(&self) -> Option<CursorIcon> {
        (!self.disabled).then_some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if self.disabled {
            if matches!(event, Event::Pointer(p) if matches!(p.phase, PointerPhase::Down | PointerPhase::Up))
            {
                ctx.handled();
            }
            return;
        }

        let sebelum = (self.hovered, self.pressed, self.focused);
        match event {
            Event::Pointer(p) => match p.phase {
                PointerPhase::Enter => self.hovered = true,
                // Sengaja tidak membatalkan `pressed`: penunjuk yang ditangkap
                // boleh keluar-masuk selama tombol ditahan.
                PointerPhase::Leave => self.hovered = false,
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    self.pressed = true;
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let di_dalam = self.style.corners.contains(ctx.size(), ctx.local());
                    let aktif = self.pressed && di_dalam;
                    self.pressed = false;
                    ctx.release_pointer();
                    ctx.handled();
                    if aktif {
                        // Retarget dulu, baru kirim: handler boleh membangun
                        // ulang node ini seketika.
                        self.retarget();
                        let niat = if self.open {
                            SelectIntent::Close
                        } else {
                            // Kotak global pemicu = jangkar popup. Node tidak
                            // pernah tahu posisinya sendiri di dalam layout,
                            // tapi lapisan input memang tahu (`EventCtx`).
                            SelectIntent::Open(ctx.bounds())
                        };
                        self.kirim(niat);
                    }
                }
                PointerPhase::Cancel if self.pressed => self.pressed = false,
                _ => {}
            },

            Event::Key(k) if k.is_pressed() => {
                let polos = k.modifiers.is_empty();
                let boleh_ketik = polos || k.modifiers.is_exactly(Modifiers::SHIFT);
                let n = self.options.len();
                match &k.code {
                    KeyCode::Named(NamedKey::Escape) if self.open && polos => {
                        ctx.handled();
                        self.kirim(SelectIntent::Close);
                    }
                    KeyCode::Named(NamedKey::Enter) | KeyCode::Named(NamedKey::Space) if polos => {
                        ctx.handled();
                        if self.open {
                            self.kirim(SelectIntent::Commit(self.highlight));
                        } else {
                            self.retarget();
                            self.kirim(SelectIntent::Open(ctx.bounds()));
                        }
                    }
                    KeyCode::Named(NamedKey::ArrowDown) if polos => {
                        ctx.handled();
                        if self.open {
                            self.geser_sorotan(1);
                        } else {
                            self.kirim(SelectIntent::Open(ctx.bounds()));
                        }
                    }
                    KeyCode::Named(NamedKey::ArrowUp) if polos => {
                        ctx.handled();
                        if self.open {
                            self.geser_sorotan(-1);
                        } else {
                            self.kirim(SelectIntent::Open(ctx.bounds()));
                        }
                    }
                    KeyCode::Named(NamedKey::Home) if self.open && polos => {
                        ctx.handled();
                        self.sorot(0);
                    }
                    KeyCode::Named(NamedKey::End) if self.open && polos && n > 0 => {
                        ctx.handled();
                        self.sorot(n - 1);
                    }
                    KeyCode::Character(c) if boleh_ketik => {
                        let c = *c;
                        if let Some(i) = self.typeahead(c, k.time) {
                            ctx.handled();
                            if self.open {
                                self.sorot(i);
                            } else {
                                // Menu tertutup: mengetik langsung memilih,
                                // persis pop-up button macOS.
                                self.highlight = i;
                                self.kirim(SelectIntent::Commit(i));
                            }
                        }
                    }
                    _ => {}
                }
            }

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.pressed = false;
                }
            }

            _ => {}
        }

        if (self.hovered, self.pressed, self.focused) != sebelum {
            self.retarget();
            ctx.request_paint();
            // Tanpa ini frame berikutnya tidak akan pernah datang dan spring
            // membeku di tempat (§3.5 "render hanya saat dirty").
            ctx.request_animation();
        }
    }
}

impl core::fmt::Debug for SelectTrigger {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SelectTrigger")
            .field("value", &self.value)
            .field("open", &self.open)
            .field("highlight", &self.highlight)
            .field("disabled", &self.disabled)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props pemicu select — bentuk view dari [`SelectTrigger`].
#[derive(Debug, Clone, PartialEq)]
pub struct SelectTriggerProps {
    /// Nilai gambar, sudah diresolusi dari token.
    pub style: SelectTriggerStyle,
    /// Nama yang dibacakan screen reader.
    pub label: Option<String>,
    /// Nilai yang dibacakan screen reader (teks pilihan sekarang).
    pub value: Option<String>,
    /// Daftar pilihan — dipakai typeahead di dalam node.
    pub options: Rc<Vec<String>>,
    /// Popup sedang terbuka.
    pub open: bool,
    /// Indeks yang sedang disorot.
    pub highlight: usize,
    /// Tidak bisa dipakai.
    pub disabled: bool,
    /// Peran dalam navigasi fokus.
    pub focus: FocusPolicy,
    /// Spring yang menjalankan transisi state.
    pub spring: Spring,
    /// Ke mana niat pengguna dikirim.
    pub on_intent: Option<SelectHandler>,
}

impl ViewNode for SelectTriggerProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(SelectTrigger::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<SelectTrigger>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        let keadaan_berubah =
            n.style != self.style || n.open != self.open || n.disabled != self.disabled;
        if n.style != self.style {
            n.style = self.style;
        }
        if n.open != self.open {
            n.open = self.open;
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                // Kontrol yang baru saja dimatikan tidak boleh membeku dalam
                // keadaan ditekan/hover — penunjuknya tidak akan datang lagi.
                n.pressed = false;
                n.hovered = false;
            }
        }
        if keadaan_berubah {
            // Warna baru **dituju**, bukan dilompati: ganti theme pun lewat spring.
            n.retarget();
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.value != self.value {
            n.value.clone_from(&self.value);
            dirty |= Dirty::PAINT;
        }
        if n.options != self.options {
            n.options = self.options.clone();
        }
        if n.highlight != self.highlight {
            n.highlight = self.highlight;
        }
        if n.focus != self.focus {
            n.focus = self.focus;
            dirty |= Dirty::PAINT;
        }
        if n.bg.spring() != self.spring {
            // Ganti preset spring tanpa mengganggu gerakan yang sedang berjalan.
            n.bg.set_spring(self.spring);
            n.open_t.set_spring(self.spring);
        }
        // Handler selalu diganti tanpa dibandingkan: closure dibangun ulang tiap
        // rebuild dan **menangkap nilai baru**.
        n.on_intent.clone_from(&self.on_intent);
        dirty
    }
}
