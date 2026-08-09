//! `checkbox()` — kotak centang Tier 2 (`KOMPONEN.md`), **termasuk state
//! indeterminate dan animasi centang** seperti yang diminta catatan khususnya.
//!
//! ```
//! # use silka_widgets::{checkbox, Fonts};
//! # use silka_theme::{Appearance, Theme};
//! # use silka_core::signals::Runtime;
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! # let rt = Runtime::new();
//! let aktif = rt.signal(false);
//!
//! checkbox(&fonts, &t, "Sinkronkan otomatis")
//!     .checked(aktif.get())
//!     .on_toggle(move |v| aktif.set(v));
//! ```
//!
//! ## Kenapa ini node sendiri, bukan pembungkus `Interactive`
//!
//! Checkbox butuh tiga hal yang tidak ada di kontrak interaktif umum dan tidak
//! boleh dipalsukan:
//!
//! 1. **Keadaan tiga-nilai** ([`CheckState`]) yang sampai ke screen reader
//!    sebagai [`AccessToggled`] — bukan sebagai nama tombol yang berubah-ubah.
//! 2. **Centang yang digambar bertahap** (`KOMPONEN.md`: "animasi centang"),
//!    bukan simbol yang muncul tiba-tiba.
//! 3. **Kotak kecil di dalam area sentuh besar**: yang digambar 16pt, yang bisa
//!    diklik ≥ 44pt (HIG) — dan labelnya ikut bisa diklik, seperti `<label for>`
//!    di web dan `NSButton` bertipe switch di AppKit.
//!
//! ## Bagaimana centangnya digambar tanpa perintah "garis"
//!
//! `silka-paint` hari ini mengenal kotak bersudut, glyph, dan bayangan (§3.2)
//! — tidak ada primitif goresan, dan tidak ada rotasi. Centangnya karena itu
//! dirakit dari rantai kotak berujung bulat yang saling menindih
//! ([`check_dots`]): sebuah pena bundar yang dijejakkan rapat-rapat sepanjang
//! jalur. Hasilnya identik dengan goresan ber-round-cap, biayanya belasan quad,
//! dan geometrinya murni CPU sehingga bisa diuji tanpa GPU.
//!
//! Itu **utang teknis yang disadari**, bukan kecelakaan: begitu lapisan paint
//! punya perintah goresan SDF sendiri, [`check_dots`] menjadi satu perintah dan
//! tidak ada satu baris pun di luar berkas ini yang berubah. Yang sengaja tidak
//! ditempuh: merender glyph "✓" akan menyandera bentuk centang pada font yang
//! kebetulan terpasang, **dan** membuat animasi goresan mustahil.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! - **Kedua preset** — setiap angka lewat [`CheckboxStyle::from_theme`];
//!   sudut kotak adalah `radius.sm` yang di Cupertino squircle dan di Tailwind
//!   arc, keduanya parameter shader, bukan konstanta (§2.7, §3.6).
//! - **Semua state interaktif dengan spring** — latar, border, goresan, garis
//!   indeterminate, penyusutan tekan, dan cincin fokus masing-masing sebuah
//!   [`SpringValue`] yang di-retarget di tengah jalan, tidak pernah dimulai
//!   ulang (§3.5).
//! - **Keyboard + focus ring** — Space mengaktifkan (di HIG dan di web, Enter
//!   milik tombol default sebuah form); cincinnya tumbuh dengan spring.
//! - **Node AccessKit** — peran [`AccessRole::CheckBox`], nama dari labelnya,
//!   [`AccessToggled`] tiga-nilai, aksi klik + fokus.
//! - **Dark mode** — seluruh warna token, tanpa satu literal pun.
//! - **Hit target ≥ 44pt** — dijamin [`CheckboxNode::layout`], bukan pemanggil.
//! - **Reduced-motion** — gerakan yang *menjelaskan* (latar, goresan, garis)
//!   tetap berjalan tanpa pantulan; yang cuma menghias (penyusutan tekan,
//!   cincin fokus) ditandai [`MotionRole::Decorative`] dan hilang sepenuhnya.

use std::rc::Rc;

use silka_core::access::{AccessActions, AccessNode, AccessRole, AccessToggled};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, CornerRadii, CornerStyle, Corners, Insets, Point, Quad, Rect, Size};
use silka_text::FontWeight;
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::text::text;

// ---------------------------------------------------------------------------
// Keadaan
// ---------------------------------------------------------------------------

/// Keadaan sebuah kotak centang.
///
/// Tiga nilai, bukan dua: `Mixed` (indeterminate) adalah keadaan sah sebuah
/// checkbox induk yang anak-anaknya hanya sebagian tercentang — `KOMPONEN.md`
/// menyebutnya bagian komponen ini, bukan tambahan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CheckState {
    /// Tidak tercentang.
    #[default]
    Off,
    /// Tercentang.
    On,
    /// Sebagian — digambar sebagai garis, bukan centang.
    Mixed,
}

impl CheckState {
    /// Keadaan berikutnya saat pengguna mengaktifkan kotak ini.
    ///
    /// `Mixed` **tidak** ikut dalam siklus: pengguna tidak pernah memilih
    /// "sebagian" — itu keadaan yang lahir dari data, jadi mengaktifkannya
    /// berarti memutuskan, yaitu `On` (aturan yang sama di AppKit dan HTML).
    pub fn toggled(self) -> Self {
        match self {
            CheckState::On => CheckState::Off,
            CheckState::Off | CheckState::Mixed => CheckState::On,
        }
    }

    /// Benar bila tercentang penuh.
    pub fn is_on(self) -> bool {
        matches!(self, CheckState::On)
    }

    /// Benar bila kotaknya menggambar sesuatu di dalamnya (centang atau garis).
    pub fn is_filled(self) -> bool {
        !matches!(self, CheckState::Off)
    }

    /// Nama pendek untuk dump dan log.
    pub const fn name(self) -> &'static str {
        match self {
            CheckState::Off => "off",
            CheckState::On => "on",
            CheckState::Mixed => "mixed",
        }
    }
}

impl From<bool> for CheckState {
    fn from(v: bool) -> Self {
        if v {
            CheckState::On
        } else {
            CheckState::Off
        }
    }
}

impl From<CheckState> for AccessToggled {
    fn from(s: CheckState) -> Self {
        match s {
            CheckState::Off => AccessToggled::Off,
            CheckState::On => AccessToggled::On,
            CheckState::Mixed => AccessToggled::Mixed,
        }
    }
}

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------

/// Aksi yang dititipkan aplikasi untuk menerima keadaan **baru**.
///
/// Sengaja bukan [`silka_core::Callback`]: yang perlu diceritakan sebuah
/// checkbox bukan "aku ditekan" melainkan "aku sekarang begini". Tanpa argumen
/// itu setiap pemanggil terpaksa menghitung ulang keadaan berikutnya sendiri —
/// tempat paling gampang melahirkan dua sumber kebenaran. Tiga sifatnya sama
/// dengan `Callback`: `Clone` murah, `PartialEq` berdasarkan identitas, dan
/// tidak pernah menyentuh pohon.
#[derive(Clone)]
pub struct ChangeCallback(Rc<dyn Fn(CheckState)>);

impl ChangeCallback {
    /// Bungkus sebuah closure.
    pub fn new(f: impl Fn(CheckState) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Jalankan aksinya dengan keadaan baru.
    pub fn call(&self, state: CheckState) {
        (self.0)(state)
    }
}

impl PartialEq for ChangeCallback {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for ChangeCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ChangeCallback")
    }
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Seluruh nilai gambar sebuah checkbox, **sudah diresolusi** dari token theme.
///
/// Mesin tidak pernah punya pendapat tentang warna maupun ukuran (§2.6, §2.7):
/// preset Cupertino dan Tailwind berganti dengan mengisi struct ini, tanpa satu
/// baris pun berubah di [`CheckboxNode`]. Preset ketiga (brand kustom) tinggal
/// menyerahkan struct ini lewat [`Builder::style`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CheckboxStyle {
    /// Sisi kotak yang digambar, poin logis.
    pub box_size: f32,
    /// Bentuk sudut kotak — squircle di Cupertino, arc di Tailwind.
    pub corners: Corners,
    /// Tebal border kotak.
    pub border_width: f32,
    /// Tebal goresan centang dan garis indeterminate.
    pub stroke: f32,
    /// Jarak kotak ke label.
    pub gap: f32,
    /// Tebal cincin fokus keyboard.
    pub focus_ring_width: f32,
    /// Sisi minimum area sentuh (HIG).
    pub min_target: f32,
    /// Seberapa jauh kotak mengempis saat ditekan, poin logis.
    pub press_travel: f32,

    /// Latar diam, keadaan kosong.
    pub rest_off: Color,
    /// Latar diam, keadaan terisi.
    pub rest_on: Color,
    /// Latar saat di-hover, keadaan kosong.
    pub hover_off: Color,
    /// Latar saat di-hover, keadaan terisi.
    pub hover_on: Color,
    /// Latar saat ditekan, keadaan kosong.
    pub pressed_off: Color,
    /// Latar saat ditekan, keadaan terisi.
    pub pressed_on: Color,
    /// Border keadaan kosong.
    pub border_off: Color,
    /// Border keadaan terisi.
    pub border_on: Color,
    /// Latar saat tidak bisa dipakai.
    pub disabled_box: Color,
    /// Border saat tidak bisa dipakai.
    pub disabled_border: Color,
    /// Warna goresan centang.
    pub mark: Color,
    /// Warna goresan saat tidak bisa dipakai.
    pub disabled_mark: Color,
    /// Warna cincin fokus.
    pub focus_ring: Color,
}

impl CheckboxStyle {
    /// Nilai bawaan dari theme aktif.
    ///
    /// `space(4.0)` = 16pt di kedua preset — kebetulan yang bukan kebetulan:
    /// itu persis ukuran `h-4 w-4` milik checkbox shadcn/ui, dan sekitar ukuran
    /// checkbox AppKit pada teks body.
    pub fn from_theme(theme: &Theme) -> Self {
        let c = &theme.color;
        Self {
            box_size: theme.space(4.0),
            corners: theme.corners(theme.radius.sm),
            border_width: theme.space(0.25),
            stroke: theme.space(0.5),
            gap: theme.space(2.0),
            focus_ring_width: theme.space(0.5),
            min_target: MIN_HIT_TARGET,
            press_travel: theme.space(0.25),

            rest_off: c.surface,
            rest_on: c.accent,
            hover_off: c.surface_hover,
            hover_on: c.accent_hover,
            pressed_off: c.surface_pressed,
            pressed_on: c.accent_pressed,
            border_off: c.border,
            border_on: c.accent,
            disabled_box: c.surface_sunken,
            disabled_border: c.separator,
            mark: c.on_accent,
            disabled_mark: c.disabled_label,
            focus_ring: c.focus_ring,
        }
    }

    /// Latar yang seharusnya berlaku untuk kombinasi keadaan ini.
    ///
    /// Inilah **target** spring; yang digambar adalah posisi spring-nya, bukan
    /// nilai ini.
    pub fn background_for(
        &self,
        state: CheckState,
        disabled: bool,
        hovered: bool,
        pressed: bool,
    ) -> Color {
        if disabled {
            return self.disabled_box;
        }
        let terisi = state.is_filled();
        // `pressed` bertahan saat penunjuk ditangkap keluar kotak, tapi tampilan
        // "ditekan" hanya berlaku selama penunjuknya masih di dalam — persis
        // AppKit/UIKit.
        if pressed && hovered {
            if terisi {
                self.pressed_on
            } else {
                self.pressed_off
            }
        } else if hovered {
            if terisi {
                self.hover_on
            } else {
                self.hover_off
            }
        } else if terisi {
            self.rest_on
        } else {
            self.rest_off
        }
    }

    /// Warna border yang berlaku.
    pub fn border_for(&self, state: CheckState, disabled: bool) -> Color {
        if disabled {
            self.disabled_border
        } else if state.is_filled() {
            self.border_on
        } else {
            self.border_off
        }
    }

    /// Warna goresan yang berlaku.
    pub fn mark_for(&self, disabled: bool) -> Color {
        if disabled {
            self.disabled_mark
        } else {
            self.mark
        }
    }
}

// ---------------------------------------------------------------------------
// Geometri goresan — logika murni, diuji tanpa GPU
// ---------------------------------------------------------------------------

/// Jalur centang dalam kotak satuan (0..1): tiga titik, dua ruas.
///
/// Angkanya menyisakan ruang untuk ujung bundar pena: dengan tebal 1/8 sisi
/// kotak, tidak satu pun jejak keluar dari kotaknya (diuji).
const JALUR: [(f32, f32); 3] = [(0.22, 0.52), (0.42, 0.72), (0.78, 0.30)];

/// Batas atas jumlah jejak pena untuk satu centang.
///
/// Bukan soal kualitas melainkan soal jaminan: kotak yang (karena theme kustom
/// atau bug) berukuran ribuan poin tidak boleh mengubah satu widget kecil
/// menjadi ribuan perintah gambar.
const MAX_JEJAK: usize = 64;

/// Titik-titik pusat pena sepanjang goresan centang, sampai `progress`.
///
/// Ini seluruh "animasi centang" (`KOMPONEN.md`) dalam bentuk yang bisa diuji:
/// `progress` 0 tidak menghasilkan apa pun, 1 menghasilkan goresan penuh yang
/// **berakhir tepat** di ujung jalur, dan nilai di antaranya adalah goresan
/// yang sedang ditarik. Jarak antar-jejak tidak bergantung pada `progress`,
/// jadi jejak-jejak awal tidak pernah bergeser saat goresan memanjang — syarat
/// agar gerakannya terbaca sebagai satu tarikan, bukan kedipan.
pub fn check_dots(box_rect: Rect, stroke: f32, progress: f32) -> Vec<Point> {
    let p = progress.clamp(0.0, 1.0);
    if p <= 0.0 || stroke <= 0.0 || box_rect.size.is_empty() {
        return Vec::new();
    }
    let titik: Vec<Point> = JALUR
        .iter()
        .map(|(x, y)| {
            Point::new(
                box_rect.origin.x + box_rect.size.width * x,
                box_rect.origin.y + box_rect.size.height * y,
            )
        })
        .collect();

    let ruas: Vec<f32> = titik.windows(2).map(|w| jarak(w[0], w[1])).collect();
    let total: f32 = ruas.iter().sum();
    if total <= 0.0 {
        return vec![titik[0]];
    }

    let terlihat = total * p;
    let langkah = (stroke * 0.35).max(total / MAX_JEJAK as f32);

    let mut out = Vec::with_capacity((terlihat / langkah) as usize + 2);
    let mut d = 0.0;
    while d < terlihat {
        out.push(pada_jalur(&titik, &ruas, d));
        d += langkah;
    }
    // Ujungnya selalu tepat: tanpa ini panjang goresan melompat sebesar satu
    // langkah dan gerakannya terlihat bergetar.
    out.push(pada_jalur(&titik, &ruas, terlihat));
    out
}

/// Garis indeterminate: satu kotak berujung bulat yang tumbuh dari tengah.
///
/// `None` bila belum ada yang terlihat, sehingga keadaan `Off` benar-benar
/// gratis — tidak ada perintah gambar sama sekali.
pub fn dash_rect(box_rect: Rect, stroke: f32, progress: f32) -> Option<Rect> {
    let p = progress.clamp(0.0, 1.0);
    if p <= 0.0 || stroke <= 0.0 || box_rect.size.is_empty() {
        return None;
    }
    let lebar = box_rect.size.width * 0.5 * p;
    if lebar <= 0.0 {
        return None;
    }
    let tengah = box_rect.center();
    Some(Rect::new(
        tengah.x - lebar * 0.5,
        tengah.y - stroke * 0.5,
        lebar,
        stroke,
    ))
}

fn jarak(a: Point, b: Point) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    (dx * dx + dy * dy).sqrt()
}

/// Titik pada jalur setelah menempuh `d` satuan panjang.
fn pada_jalur(titik: &[Point], ruas: &[f32], d: f32) -> Point {
    let mut sisa = d.max(0.0);
    for (i, panjang) in ruas.iter().enumerate() {
        if sisa <= *panjang || i == ruas.len() - 1 {
            let t = if *panjang > 0.0 {
                (sisa / panjang).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let a = titik[i];
            let b = titik[i + 1];
            return Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
        }
        sisa -= panjang;
    }
    titik[titik.len() - 1]
}

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// Node render sebuah kotak centang: kontrak input penuh + enam spring.
///
/// Anak pertamanya, bila ada, adalah label yang diletakkan di samping kotak dan
/// **ikut bisa diklik**.
pub struct CheckboxNode {
    style: CheckboxStyle,
    /// Keadaan yang datang dari aplikasi.
    state: CheckState,
    /// Tidak bisa dipakai — tetap dibacakan screen reader sebagai dimmed.
    disabled: bool,
    label: Option<String>,
    focus: FocusPolicy,
    on_change: Option<ChangeCallback>,

    /// Latar yang benar-benar digambar frame ini.
    bg: SpringValue<Color>,
    /// Border yang benar-benar digambar frame ini.
    border: SpringValue<Color>,
    /// Panjang goresan centang (0..1).
    check: SpringValue<f32>,
    /// Panjang garis indeterminate (0..1).
    dash: SpringValue<f32>,
    /// 0 = lepas, 1 = kempis penuh (scale-on-press).
    press_t: SpringValue<f32>,
    /// 0 = tanpa cincin fokus, 1 = cincin penuh.
    ring_t: SpringValue<f32>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    /// Jumlah aktivasi (klik atau Space) sejak node dibuat.
    activations: u32,
    /// Kotak yang digambar, koordinat lokal — hasil layout terakhir.
    box_rect: Rect,
}

impl CheckboxNode {
    /// Node baru yang **sudah berada** di keadaan diamnya.
    ///
    /// Bedanya dengan overlay yang selalu beranimasi masuk: sebuah kontrol
    /// tidak sedang "muncul", ia sedang menampilkan data. Menganimasikan
    /// keadaan awal berarti setiap form yang dibuka akan berkedip.
    fn new(style: CheckboxStyle, state: CheckState, disabled: bool, spring: Spring) -> Self {
        Self {
            bg: SpringValue::new(style.background_for(state, disabled, false, false))
                .with_spring(spring),
            border: SpringValue::new(style.border_for(state, disabled)).with_spring(spring),
            check: SpringValue::new(if state.is_on() { 1.0 } else { 0.0 }).with_spring(spring),
            dash: SpringValue::new(if state == CheckState::Mixed { 1.0 } else { 0.0 })
                .with_spring(spring),
            press_t: SpringValue::new(0.0).with_spring(spring).decorative(),
            ring_t: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            style,
            state,
            disabled,
            label: None,
            focus: FocusPolicy::FOCUSABLE,
            on_change: None,
            hovered: false,
            pressed: false,
            focused: false,
            activations: 0,
            box_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
        }
    }

    /// Peran gerakan untuk nilai yang **menjelaskan keadaan** (latar, border,
    /// panjang centang, panjang garis indeterminate).
    ///
    /// `press_t`/`ring_t` sengaja tidak ikut: keduanya murni hiasan sehingga
    /// selalu [`MotionRole::Decorative`] apa pun yang diminta pemanggil.
    /// Dipakai `build` *dan* `update` agar rebuild yang mengubah `.decorative()`
    /// benar-benar berpengaruh, bukan cuma yang pertama.
    fn set_motion_role(&mut self, role: MotionRole) {
        self.bg.set_role(role);
        self.border.set_role(role);
        self.check.set_role(role);
        self.dash.set_role(role);
    }

    /// Peran gerakan yang sedang dipakai nilai-nilai penjelas keadaan.
    fn motion_role(&self) -> MotionRole {
        self.bg.role()
    }

    /// Keadaan yang datang dari aplikasi.
    pub fn state(&self) -> CheckState {
        self.state
    }

    /// Nilai gambar yang sedang berlaku.
    pub fn style(&self) -> CheckboxStyle {
        self.style
    }

    /// Tidak bisa dipakai.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Kotak yang digambar (koordinat lokal), hasil layout terakhir.
    ///
    /// Ukuran node bisa jauh lebih besar (area sentuh 44pt, label di
    /// sampingnya); inilah bagian yang benar-benar terlihat sebagai checkbox.
    pub fn box_rect(&self) -> Rect {
        self.box_rect
    }

    /// Latar yang digambar frame ini — posisi spring, bukan targetnya.
    pub fn background(&self) -> Color {
        self.bg.position()
    }

    /// Target latar yang sedang dituju spring.
    pub fn background_target(&self) -> Color {
        self.bg.target()
    }

    /// Border yang digambar frame ini.
    pub fn border_color(&self) -> Color {
        self.border.position()
    }

    /// Kemajuan goresan centang 0..1.
    pub fn check_progress(&self) -> f32 {
        self.check.position()
    }

    /// Kemajuan garis indeterminate 0..1.
    pub fn dash_progress(&self) -> f32 {
        self.dash.position()
    }

    /// Kemajuan tekanan 0..1 (0 = lepas).
    pub fn press_progress(&self) -> f32 {
        self.press_t.position()
    }

    /// Kemajuan cincin fokus 0..1.
    pub fn focus_progress(&self) -> f32 {
        self.ring_t.position()
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

    /// Jumlah aktivasi sejak node dibuat.
    pub fn activations(&self) -> u32 {
        self.activations
    }

    /// Benar bila masih ada spring yang bergerak.
    pub fn is_animating(&self) -> bool {
        self.bg.is_animating()
            || self.border.is_animating()
            || self.check.is_animating()
            || self.dash.is_animating()
            || self.press_t.is_animating()
            || self.ring_t.is_animating()
    }

    /// Arahkan seluruh spring ke keadaan sekarang.
    ///
    /// **Retarget, bukan animasi baru** (§3.5): centang yang dibatalkan di
    /// tengah goresan berbalik arah membawa kecepatannya. Satu fungsi untuk
    /// enam nilai, dipanggil setiap kali apa pun berubah — dengan begitu tidak
    /// mungkin ada satu spring yang lupa di-retarget dan tertinggal
    /// menampilkan keadaan kemarin.
    fn retarget(&mut self) {
        let aktif = !self.disabled;
        self.bg.set_target(self.style.background_for(
            self.state,
            self.disabled,
            self.hovered,
            self.pressed,
        ));
        self.border
            .set_target(self.style.border_for(self.state, self.disabled));
        self.check
            .set_target(if self.state.is_on() { 1.0 } else { 0.0 });
        self.dash.set_target(if self.state == CheckState::Mixed {
            1.0
        } else {
            0.0
        });
        self.press_t
            .set_target(if self.pressed && self.hovered && aktif {
                1.0
            } else {
                0.0
            });
        self.ring_t
            .set_target(if self.focused && aktif { 1.0 } else { 0.0 });
    }

    /// Majukan seluruh spring satu frame; benar bila ada yang bergeser.
    ///
    /// Dipanggil [`crate::advance`], satu tempat untuk seluruh pohon.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let mut bergeser = false;
        bergeser |= maju_warna(&mut self.bg, tick);
        bergeser |= maju_warna(&mut self.border, tick);
        bergeser |= maju(&mut self.check, tick);
        bergeser |= maju(&mut self.dash, tick);
        bergeser |= maju(&mut self.press_t, tick);
        bergeser |= maju(&mut self.ring_t, tick);
        bergeser
    }

    /// Selesaikan seluruh gerakan seketika (uji, snapshot, reduced-motion).
    pub fn settle(&mut self) {
        self.bg.settle();
        self.border.settle();
        self.check.settle();
        self.dash.settle();
        self.press_t.settle();
        self.ring_t.settle();
    }

    /// Aktifkan: hitung keadaan berikutnya lalu ceritakan ke aplikasi.
    ///
    /// Node **tidak** mengubah `state`-nya sendiri. Sumber kebenarannya ada di
    /// signal aplikasi, dan yang kembali ke sini adalah hasil rebuild lewat
    /// [`CheckboxProps::update`]. Kalau node menebak duluan, checkbox yang
    /// perubahannya ditolak aplikasi (validasi gagal) akan terlihat berubah
    /// selama satu frame — kebohongan kecil yang mahal.
    ///
    /// Callback-nya disalin keluar dulu: ia hampir selalu menulis signal, dan
    /// itu tidak boleh terjadi sambil node ini dipinjam `&mut` (pola yang sama
    /// dengan [`crate::button::ButtonBox`]).
    fn aktifkan(&mut self) {
        if self.disabled {
            return;
        }
        self.activations = self.activations.saturating_add(1);
        let berikutnya = self.state.toggled();
        if let Some(cb) = self.on_change.clone() {
            cb.call(berikutnya);
        }
    }

    /// Kotak yang benar-benar digambar frame ini: mengempis mengikuti spring
    /// tekanan, dan radiusnya ikut mengecil supaya bentuknya tidak melar.
    fn kotak_tergambar(&self) -> (Rect, Corners) {
        let kempis = (self.press_t.position() * self.style.press_travel)
            .clamp(0.0, self.box_rect.size.min_side() * 0.25);
        let kotak = self.box_rect.deflate(Insets::all(kempis));
        let radii = (self.style.corners.radii.max() - kempis).max(0.0);
        (
            kotak,
            Corners::new(CornerRadii::all(radii), self.style.corners.style),
        )
    }

    /// Pena bundar untuk goresan centang dan garis indeterminate.
    ///
    /// Ujungnya **selalu** busur, bukan `theme.radius.style`: yang dibulatkan
    /// di sini adalah ujung pena, bukan sudut sebuah permukaan — squircle milik
    /// preset mengatur kotaknya, bukan goresannya.
    fn pena(rect: Rect, warna: Color) -> Quad {
        Quad::new(rect).background(warna).corners(Corners::uniform(
            rect.size.min_side() * 0.5,
            CornerStyle::Arc,
        ))
    }
}

fn maju(value: &mut SpringValue<f32>, tick: &Tick) -> bool {
    let sebelum = value.position();
    tick.advance(value);
    value.position() != sebelum
}

fn maju_warna(value: &mut SpringValue<Color>, tick: &Tick) -> bool {
    let sebelum = value.position();
    tick.advance(value);
    value.position() != sebelum
}

impl RenderNode for CheckboxNode {
    fn type_name(&self) -> &'static str {
        "Checkbox"
    }

    /// Kotak di sisi awal baca, label mengikuti, dan **area sentuh ≥ 44pt**.
    ///
    /// RTL ditangani di sini dan hanya di sini: kotaknya pindah ke kanan
    /// bersama isinya, karena arah baca adalah urusan layout — bukan urusan
    /// tiap widget menghitungnya sendiri (§9.8).
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let s = self.style;
        let sisi = s.box_size.max(0.0);

        if ctx.child_count() == 0 {
            // Tanpa label, hit target-nya kotak itu sendiri yang dipaksa
            // membesar — yang digambar tetap `box_size` (HIG: area sentuh boleh
            // lebih besar dari yang terlihat).
            let target = sisi.max(s.min_target);
            let size = constraints.constrain(Size::new(target, target));
            self.box_rect = Rect::new(
                (size.width - sisi) * 0.5,
                (size.height - sisi) * 0.5,
                sisi,
                sisi,
            );
            return size;
        }

        let depan = sisi + s.gap;
        let anak = ctx.child(0);
        let ukuran_anak = ctx.layout_child(
            anak,
            constraints
                .deflate(Insets {
                    top: 0.0,
                    right: depan,
                    bottom: 0.0,
                    left: 0.0,
                })
                .loosen(),
        );

        let size = constraints.constrain(Size::new(
            depan + ukuran_anak.width,
            ukuran_anak.height.max(sisi).max(s.min_target),
        ));

        let y_kotak = (size.height - sisi) * 0.5;
        let y_anak = (size.height - ukuran_anak.height) * 0.5;
        if ctx.direction().is_rtl() {
            self.box_rect = Rect::new(size.width - sisi, y_kotak, sisi, sisi);
            ctx.place_child(
                anak,
                Point::new((size.width - depan - ukuran_anak.width).max(0.0), y_anak),
            );
        } else {
            self.box_rect = Rect::new(0.0, y_kotak, sisi, sisi);
            ctx.place_child(anak, Point::new(depan, y_anak));
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let s = self.style;
        let (kotak, corners) = self.kotak_tergambar();

        // Cincin fokus digambar **di luar** kotak supaya tidak menutupi
        // centangnya — kebiasaan AppKit, dan syarat agar kontrol sekecil ini
        // tetap terbaca saat difokuskan.
        let ring = self.ring_t.position().clamp(0.0, 1.0) * s.focus_ring_width;
        if ring > 0.01 && s.focus_ring.a > 0.0 && !self.disabled {
            ctx.quad(
                Quad::new(kotak.deflate(Insets::all(-ring)))
                    .corners(Corners::new(
                        CornerRadii::all(corners.radii.max() + ring),
                        corners.style,
                    ))
                    .border(ring, s.focus_ring),
            );
        }

        ctx.quad(
            Quad::new(kotak)
                .corners(corners)
                .background(self.bg.position())
                .border(s.border_width, self.border.position()),
        );

        // Goresan ikut mengempis bersama kotaknya supaya tidak menonjol keluar
        // saat ditekan.
        let skala = if s.box_size > 0.0 {
            (kotak.size.min_side() / s.box_size).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let tebal = s.stroke * skala;
        let warna = s.mark_for(self.disabled);

        for pusat in check_dots(kotak, tebal, self.check.position()) {
            let jejak = Rect::new(pusat.x - tebal * 0.5, pusat.y - tebal * 0.5, tebal, tebal);
            ctx.quad(Self::pena(jejak, warna));
        }
        if let Some(garis) = dash_rect(kotak, tebal, self.dash.position()) {
            ctx.quad(Self::pena(garis, warna));
        }

        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::CheckBox;
        node.label.clone_from(&self.label);
        node.disabled = self.disabled;
        // Keadaan tiga-nilai sampai ke screen reader sebagai **keadaan**, bukan
        // sebagai nama yang berubah-ubah (§3.8).
        node.toggled = Some(AccessToggled::from(self.state));
        if !self.disabled {
            node.actions |= AccessActions::CLICK;
            if self.focus.focusable {
                node.actions |= AccessActions::FOCUS;
            }
        }
    }

    /// Seluruh baris — kotak **dan** label — adalah area sentuhnya.
    fn hit_shape(&self) -> HitShape {
        HitShape::Rect
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Checkbox yang mati tetap menyerap penunjuk: klik di atasnya tidak
        // boleh menembus ke konten di belakangnya.
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
            // Tetap menyerap agar tidak tembus, tapi tidak mengubah apa pun.
            if matches!(event, Event::Pointer(p) if matches!(p.phase, PointerPhase::Down | PointerPhase::Up))
            {
                ctx.handled();
            }
            return;
        }

        match event {
            Event::Pointer(p) => match p.phase {
                PointerPhase::Enter => {
                    if !self.hovered {
                        self.hovered = true;
                        self.retarget();
                        ctx.request_animation();
                    }
                }
                PointerPhase::Leave => {
                    if self.hovered {
                        self.hovered = false;
                        // `pressed` sengaja dipertahankan: penunjuk yang
                        // ditangkap boleh keluar-masuk selama tombol ditahan.
                        self.retarget();
                        ctx.request_animation();
                    }
                }
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    self.pressed = true;
                    self.retarget();
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.request_animation();
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let di_dalam = HitShape::Rect.contains(ctx.size(), ctx.local());
                    let jadi = self.pressed && di_dalam;
                    self.pressed = false;
                    self.retarget();
                    ctx.release_pointer();
                    ctx.request_animation();
                    ctx.handled();
                    if jadi {
                        self.aktifkan();
                    }
                }
                // Dibatalkan OS ≠ dilepas: tidak ada aktivasi.
                PointerPhase::Cancel if self.pressed => {
                    self.pressed = false;
                    self.retarget();
                    ctx.request_animation();
                }
                _ => {}
            },

            // Space, bukan Enter: di HIG (dan di web) Enter milik tombol
            // default sebuah form, sedangkan Space adalah "aktifkan kontrol
            // yang sedang difokuskan".
            Event::Key(k)
                if k.is_pressed()
                    && k.code == KeyCode::Named(NamedKey::Space)
                    && k.modifiers.is_empty() =>
            {
                ctx.handled();
                self.aktifkan();
            }

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.pressed = false;
                }
                self.retarget();
                ctx.request_animation();
            }

            _ => {}
        }
    }
}

impl core::fmt::Debug for CheckboxNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Checkbox")
            .field("state", &self.state.name())
            .field("disabled", &self.disabled)
            .field("label", &self.label)
            .field("check", &self.check.position())
            .field("dash", &self.dash.position())
            .field("box_rect", &self.box_rect)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props [`CheckboxNode`] — bentuk view-nya.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckboxProps {
    style: CheckboxStyle,
    state: CheckState,
    disabled: bool,
    label: Option<String>,
    focus: FocusPolicy,
    spring: Spring,
    motion: MotionRole,
    on_change: Option<ChangeCallback>,
}

impl CheckboxProps {
    /// Props bawaan untuk theme aktif.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            style: CheckboxStyle::from_theme(theme),
            state: CheckState::Off,
            disabled: false,
            label: None,
            focus: FocusPolicy::FOCUSABLE,
            spring: Spring::snappy(),
            motion: MotionRole::Essential,
            on_change: None,
        }
    }
}

impl ViewNode for CheckboxProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut node = CheckboxNode::new(self.style, self.state, self.disabled, self.spring);
        node.label.clone_from(&self.label);
        node.focus = self.focus;
        node.on_change.clone_from(&self.on_change);
        // Aplikasi yang menyatakan gerakan ini sekadar hiasan: reduced-motion
        // mematikannya sepenuhnya, bukan cuma membuang pantulannya.
        node.set_motion_role(self.motion);
        Box::new(node)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<CheckboxNode>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.style != self.style {
            // `box_size`/`gap` ikut di sini, jadi theme yang berganti preset
            // memang harus di-layout ulang — bukan cuma digambar ulang.
            n.style = self.style;
            dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.focus != self.focus {
            n.focus = self.focus;
            dirty |= Dirty::PAINT;
        }
        if n.motion_role() != self.motion {
            // Tanpa diff ini, rebuild yang mengubah `.decorative()` diam-diam
            // mempertahankan peran lama — dan reduced-motion jadi salah.
            n.set_motion_role(self.motion);
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.bg.spring() != self.spring {
            n.bg.set_spring(self.spring);
            n.border.set_spring(self.spring);
            n.check.set_spring(self.spring);
            n.dash.set_spring(self.spring);
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                // Kontrol yang baru dimatikan tidak boleh membeku dalam keadaan
                // ditekan/hover: penunjuknya tidak akan pernah datang lagi.
                n.pressed = false;
                n.hovered = false;
            }
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.state != self.state {
            n.state = self.state;
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        // Selalu di-retarget: murah, dan menutup setiap kombinasi perubahan di
        // atas sekaligus. Yang tidak berubah tidak menghasilkan gerakan apa pun
        // karena `set_target` ke nilai yang sama tidak membangunkan spring.
        n.retarget();
        // Callback selalu diganti tanpa dibandingkan: closure dibangun ulang
        // tiap rebuild dan menangkap nilai baru (pola `InteractiveProps`).
        n.on_change.clone_from(&self.on_change);
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder gaya Dart
// ---------------------------------------------------------------------------

/// Kotak centang — komponen `checkbox` (`KOMPONEN.md` Tier 2).
///
/// Tipe builder tersendiri, bukan [`Builder<CheckboxProps>`], karena label
/// harus **sudah diketahui** saat pohon view dirakit: ia menjadi anak yang
/// digambar *dan* nama a11y sekaligus, jadi ia tidak bisa dititipkan lewat
/// `map` seperti properti biasa (pola yang sama dengan [`crate::button::Button`]).
pub struct Checkbox {
    fonts: Option<Fonts>,
    theme: Theme,
    label: Option<String>,
    style: CheckboxStyle,
    state: CheckState,
    disabled: bool,
    spring: Spring,
    motion: MotionRole,
    focus: FocusPolicy,
    on_change: Option<ChangeCallback>,
    key: Option<Key>,
}

/// Kotak centang berlabel.
///
/// Labelnya ikut bisa diklik **dan sekaligus** menjadi nama yang dibacakan
/// screen reader — satu sumber, jadi tidak mungkin yang terlihat dan yang
/// terdengar berbeda.
///
/// ```
/// # use silka_widgets::{checkbox, CheckState, Fonts};
/// # use silka_theme::{Appearance, Theme};
/// # let fonts = Fonts::bundled_only();
/// # let t = Theme::tailwind(Appearance::Light);
/// checkbox(&fonts, &t, "Semua item")
///     .state(CheckState::Mixed)
///     .on_change(|baru| println!("sekarang {}", baru.name()));
/// ```
pub fn checkbox(fonts: &Fonts, theme: &Theme, label: impl Into<String>) -> Checkbox {
    Checkbox {
        fonts: Some(fonts.clone()),
        label: Some(label.into()),
        ..checkbox_only(theme)
    }
}

/// Kotak centang tanpa label terlihat — di dalam sel tabel, di depan baris
/// daftar, atau di header "pilih semua".
///
/// Tetap **wajib** punya nama lewat [`Checkbox::label`]: kontrol tanpa nama
/// adalah kontrol yang tidak ada bagi screen reader (§3.8), dan itu bug, bukan
/// pilihan desain.
///
/// ```
/// # use silka_widgets::checkbox_only;
/// # use silka_theme::{Appearance, Theme};
/// # let t = Theme::cupertino(Appearance::Light);
/// checkbox_only(&t).label("Pilih semua").checked(true);
/// ```
pub fn checkbox_only(theme: &Theme) -> Checkbox {
    Checkbox {
        fonts: None,
        theme: *theme,
        label: None,
        style: CheckboxStyle::from_theme(theme),
        state: CheckState::Off,
        disabled: false,
        // `snappy` adalah rasa kontrol macOS: cepat sampai, nyaris tanpa
        // pantulan (WWDC23).
        spring: Spring::snappy(),
        motion: MotionRole::Essential,
        focus: FocusPolicy::FOCUSABLE,
        on_change: None,
        key: None,
    }
}

impl Checkbox {
    /// Keadaan dua-nilai.
    pub fn checked(self, checked: bool) -> Self {
        self.state(CheckState::from(checked))
    }

    /// Keadaan tiga-nilai (termasuk [`CheckState::Mixed`]).
    pub fn state(mut self, state: CheckState) -> Self {
        self.state = state;
        self
    }

    /// Nama yang dibacakan screen reader.
    ///
    /// Untuk [`checkbox`] ini juga mengganti teks yang tergambar — nama dan
    /// tulisan tidak pernah boleh berbeda.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Matikan interaksi (tetap dibacakan sebagai dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Bisa menerima fokus keyboard atau tidak.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focus.focusable = focusable;
        self
    }

    /// Urutan tab eksplisit (mendahului urutan pohon).
    pub fn tab_order(mut self, order: i32) -> Self {
        self.focus.focusable = true;
        self.focus.order = Some(order);
        self
    }

    /// Apa yang dijalankan saat pengguna mengubahnya — menerima keadaan
    /// **baru**, bukan yang lama.
    pub fn on_change(mut self, f: impl Fn(CheckState) + 'static) -> Self {
        self.on_change = Some(ChangeCallback::new(f));
        self
    }

    /// Bentuk dua-nilai dari [`Checkbox::on_change`], untuk checkbox yang
    /// memang tidak pernah `Mixed`.
    pub fn on_toggle(self, f: impl Fn(bool) + 'static) -> Self {
        self.on_change(move |s| f(s.is_on()))
    }

    /// Spring yang menjalankan perubahan keadaannya.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Tandai gerakannya **dekoratif**: reduced-motion mematikannya sepenuhnya
    /// alih-alih sekadar membuang pantulannya.
    pub fn decorative(mut self) -> Self {
        self.motion = MotionRole::Decorative;
        self
    }

    /// Nilai gambar kustom (preset brand ketiga, §2.7).
    pub fn style(mut self, style: CheckboxStyle) -> Self {
        self.style = style;
        self
    }

    /// Kunci identitas di antara saudara-saudaranya (§2.5) — wajib untuk
    /// anggota daftar dinamis.
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Nilai gambar yang akan dipakai — dipakai gallery dan uji token.
    pub fn resolved_style(&self) -> CheckboxStyle {
        self.style
    }
}

impl From<Checkbox> for View {
    fn from(c: Checkbox) -> View {
        let t = c.theme;
        let mut builder = Builder::new(CheckboxProps {
            style: c.style,
            state: c.state,
            disabled: c.disabled,
            label: c.label.clone(),
            focus: c.focus,
            spring: c.spring,
            motion: c.motion,
            on_change: c.on_change,
        });

        // Label hanya digambar bila memang ada mesin teksnya; `checkbox_only`
        // tetap punya nama a11y tanpa satu glyph pun.
        if let (Some(fonts), Some(label)) = (c.fonts, c.label) {
            let warna = if c.disabled {
                t.color.disabled_label
            } else {
                t.color.label
            };
            builder = builder.child(
                text(&fonts, &label)
                    .size(t.typography.body_size)
                    .line_height(t.typography.body_line_height)
                    .weight(FontWeight::REGULAR)
                    .color(warna)
                    // Nama kontrol dibacakan sekali, dari node checkbox-nya —
                    // bukan dua kali (aturan yang sama dengan `button`).
                    .role(AccessRole::Container),
            );
        }
        if let Some(key) = c.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for Checkbox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Checkbox")
            .field("label", &self.label)
            .field("state", &self.state.name())
            .field("disabled", &self.disabled)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::input::{
        Event, InputRouter, KeyCode, KeyEvent, NamedKey, PointerEvent, PointerPhase,
    };
    use silka_core::tree::{BoxConstraints, RenderTree, TextDirection};
    use silka_core::view::{reconcile, View};
    use silka_paint::{Command, Scene};
    use silka_theme::{Appearance, Preset};
    use std::cell::Cell;
    use std::time::Duration;

    const RUANG: Size = Size::new(400.0, 200.0);

    fn tema() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn pohon(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(RUANG));
        tree
    }

    fn node(tree: &RenderTree) -> &CheckboxNode {
        let id = tree.children(tree.root())[0];
        tree.node_ref::<CheckboxNode>(id).expect("node checkbox")
    }

    fn detak(tree: &mut RenderTree, motion: Motion) -> bool {
        let tick = Tick::manual(Duration::from_millis(16), motion);
        let id = tree.children(tree.root())[0];
        let bergeser = tree
            .node_mut_ref::<CheckboxNode>(id)
            .map(|n| n.advance(&tick))
            .unwrap_or(false);
        tree.mark_needs_paint(id);
        bergeser
    }

    fn selesaikan(tree: &mut RenderTree) {
        let id = tree.children(tree.root())[0];
        if let Some(n) = tree.node_mut_ref::<CheckboxNode>(id) {
            n.settle();
        }
        tree.mark_needs_paint(id);
    }

    fn klik(tree: &mut RenderTree, router: &mut InputRouter, titik: Point) {
        for e in [
            PointerEvent::new(PointerPhase::Move, titik, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, titik, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, titik, Duration::from_millis(60))
                .button(PointerButton::Primary),
        ] {
            router.dispatch(tree, &Event::Pointer(e));
        }
    }

    // -- geometri goresan ---------------------------------------------------

    #[test]
    fn goresan_kosong_saat_belum_mulai_dan_penuh_saat_selesai() {
        let kotak = Rect::new(0.0, 0.0, 16.0, 16.0);
        assert!(check_dots(kotak, 2.0, 0.0).is_empty());
        assert!(check_dots(kotak, 2.0, -1.0).is_empty());

        let penuh = check_dots(kotak, 2.0, 1.0);
        assert!(penuh.len() > 4, "goresan terlalu jarang: {}", penuh.len());
        let awal = penuh[0];
        let akhir = penuh[penuh.len() - 1];
        assert!((awal.x - 16.0 * JALUR[0].0).abs() < 1e-3);
        assert!((akhir.x - 16.0 * JALUR[2].0).abs() < 1e-3);
        assert!((akhir.y - 16.0 * JALUR[2].1).abs() < 1e-3);
    }

    #[test]
    fn goresan_tumbuh_monoton_dan_jejak_awalnya_tidak_bergeser() {
        let kotak = Rect::new(0.0, 0.0, 16.0, 16.0);
        let mut sebelumnya = 0usize;
        let mut jejak_awal: Option<Vec<Point>> = None;
        for i in 0..=10 {
            let d = check_dots(kotak, 2.0, i as f32 / 10.0);
            assert!(d.len() >= sebelumnya, "goresan menyusut di {i}");
            sebelumnya = d.len();
            // Semua kecuali jejak ujung berada di kisi yang sama setiap saat.
            if let Some(awal) = &jejak_awal {
                let n = awal.len();
                assert_eq!(&d[..n], &awal[..], "jejak awal bergeser di {i}");
            }
            if d.len() > 1 {
                jejak_awal = Some(d[..d.len() - 1].to_vec());
            }
        }
    }

    #[test]
    fn goresan_tidak_pernah_keluar_dari_kotaknya() {
        let kotak = Rect::new(4.0, 6.0, 16.0, 16.0);
        let tebal = 2.0;
        for pusat in check_dots(kotak, tebal, 1.0) {
            assert!(pusat.x - tebal * 0.5 >= kotak.min_x() - 1e-3, "{pusat:?}");
            assert!(pusat.x + tebal * 0.5 <= kotak.max_x() + 1e-3, "{pusat:?}");
            assert!(pusat.y - tebal * 0.5 >= kotak.min_y() - 1e-3, "{pusat:?}");
            assert!(pusat.y + tebal * 0.5 <= kotak.max_y() + 1e-3, "{pusat:?}");
        }
    }

    #[test]
    fn kotak_raksasa_tidak_melahirkan_ribuan_perintah() {
        let kotak = Rect::new(0.0, 0.0, 4000.0, 4000.0);
        assert!(check_dots(kotak, 1.0, 1.0).len() <= MAX_JEJAK + 1);
    }

    #[test]
    fn garis_indeterminate_tumbuh_dari_tengah() {
        let kotak = Rect::new(0.0, 0.0, 16.0, 16.0);
        assert!(dash_rect(kotak, 2.0, 0.0).is_none());
        let separuh = dash_rect(kotak, 2.0, 0.5).expect("garis");
        let penuh = dash_rect(kotak, 2.0, 1.0).expect("garis");
        assert!(separuh.size.width < penuh.size.width);
        assert!((separuh.center().x - penuh.center().x).abs() < 1e-3);
        assert!((penuh.center().y - kotak.center().y).abs() < 1e-3);
        assert_eq!(penuh.size.height, 2.0);
    }

    // -- keadaan ------------------------------------------------------------

    #[test]
    fn mixed_tidak_pernah_jadi_pilihan_pengguna() {
        assert_eq!(CheckState::Off.toggled(), CheckState::On);
        assert_eq!(CheckState::On.toggled(), CheckState::Off);
        // Mengaktifkan checkbox "sebagian" berarti memutuskan: penuh.
        assert_eq!(CheckState::Mixed.toggled(), CheckState::On);
    }

    // -- layout & hit target ------------------------------------------------

    #[test]
    fn hit_target_minimal_44pt_walau_kotaknya_16pt() {
        let f = Fonts::bundled_only();
        let t = tema();
        for view in [
            View::from(checkbox(&f, &t, "Ok")),
            View::from(checkbox_only(&t).label("Ok")),
        ] {
            let tree = pohon(view);
            let id = tree.children(tree.root())[0];
            let ukuran = tree.size(id);
            assert!(
                ukuran.height >= MIN_HIT_TARGET,
                "hit target cuma {ukuran:?} (HIG minta {MIN_HIT_TARGET}pt)"
            );
            assert!(ukuran.width >= t.space(4.0));
            // Yang digambar tetap sekecil tokennya.
            assert_eq!(node(&tree).box_rect().size.width, t.space(4.0));
        }
    }

    #[test]
    fn label_diletakkan_di_sisi_awal_baca() {
        let f = Fonts::bundled_only();
        let t = tema();

        let mut ltr = RenderTree::new();
        reconcile(&mut ltr, checkbox(&f, &t, "Aktif"));
        ltr.layout(BoxConstraints::loose(RUANG));
        let kotak_ltr = node(&ltr).box_rect();
        assert_eq!(kotak_ltr.min_x(), 0.0, "LTR: kotak di kiri");

        let mut rtl = RenderTree::new();
        rtl.set_direction(TextDirection::Rtl);
        reconcile(&mut rtl, checkbox(&f, &t, "Aktif"));
        rtl.layout(BoxConstraints::loose(RUANG));
        let id = rtl.children(rtl.root())[0];
        let kotak_rtl = rtl.node_ref::<CheckboxNode>(id).expect("node").box_rect();
        assert!(
            kotak_rtl.max_x() >= rtl.size(id).width - 1e-3,
            "RTL: kotak harus di kanan, bukan {kotak_rtl:?}"
        );
    }

    // -- a11y ---------------------------------------------------------------

    #[test]
    fn dibacakan_sebagai_checkbox_dengan_keadaan_tiga_nilai() {
        let f = Fonts::bundled_only();
        let t = tema();
        for (state, harapan) in [
            (CheckState::Off, AccessToggled::Off),
            (CheckState::On, AccessToggled::On),
            (CheckState::Mixed, AccessToggled::Mixed),
        ] {
            let tree = pohon(checkbox(&f, &t, "Notifikasi").state(state));
            let a11y = tree.access_tree(None);
            let e = a11y
                .find_label("Notifikasi")
                .unwrap_or_else(|| panic!("{}", a11y.dump()));
            assert_eq!(e.node.role, AccessRole::CheckBox);
            assert_eq!(e.node.toggled, Some(harapan));
            assert!(e.node.actions.contains(AccessActions::CLICK));
            assert!(e.node.actions.contains(AccessActions::FOCUS));

            // Labelnya tidak ikut diumumkan sendiri: satu kontrol = satu nama.
            let jumlah = a11y
                .entries()
                .iter()
                .filter(|x| x.node.label.as_deref() == Some("Notifikasi"))
                .count();
            assert_eq!(jumlah, 1, "nama dibacakan dua kali:\n{}", a11y.dump());
        }
    }

    #[test]
    fn checkbox_mati_tetap_dibacakan_tapi_tanpa_aksi() {
        let f = Fonts::bundled_only();
        let t = tema();
        let tree = pohon(checkbox(&f, &t, "Terkunci").checked(true).disabled(true));
        let a11y = tree.access_tree(None);
        let e = a11y.find_label("Terkunci").expect("tetap ada");
        assert!(e.node.disabled);
        assert_eq!(e.node.toggled, Some(AccessToggled::On));
        assert!(!e.node.actions.contains(AccessActions::CLICK));
    }

    // -- interaksi ----------------------------------------------------------

    #[test]
    fn klik_menceritakan_keadaan_baru_bukan_mengubah_dirinya_sendiri() {
        let f = Fonts::bundled_only();
        let t = tema();
        let dilihat: Rc<Cell<Option<CheckState>>> = Rc::new(Cell::new(None));
        let catat = dilihat.clone();

        let mut tree = pohon(
            checkbox(&f, &t, "Aktif")
                .checked(false)
                .on_change(move |s| catat.set(Some(s))),
        );
        let id = tree.children(tree.root())[0];
        let tengah = tree.bounds(id).center();

        let mut router = InputRouter::new();
        klik(&mut tree, &mut router, tengah);

        assert_eq!(dilihat.get(), Some(CheckState::On));
        // Node tidak menebak duluan: keadaannya baru berubah lewat rebuild.
        assert_eq!(node(&tree).state(), CheckState::Off);
        assert_eq!(node(&tree).activations(), 1);

        // Rebuild dengan keadaan baru = spring diarahkan, bukan dilompati.
        reconcile(&mut tree, checkbox(&f, &t, "Aktif").checked(true));
        assert_eq!(node(&tree).state(), CheckState::On);
        assert!(node(&tree).is_animating());
    }

    #[test]
    fn klik_pada_label_ikut_mengaktifkan() {
        let f = Fonts::bundled_only();
        let t = tema();
        let n = Rc::new(Cell::new(0u32));
        let catat = n.clone();
        let mut tree = pohon(
            checkbox(&f, &t, "Label panjang sekali").on_change(move |_| catat.set(catat.get() + 1)),
        );
        let id = tree.children(tree.root())[0];
        let kotak = tree.bounds(id);
        // Jauh di kanan kotak centangnya — masih di dalam labelnya.
        let titik = Point::new(kotak.max_x() - 4.0, kotak.center().y);

        let mut router = InputRouter::new();
        klik(&mut tree, &mut router, titik);
        assert_eq!(n.get(), 1, "label harus ikut bisa diklik");
    }

    #[test]
    fn spasi_mengaktifkan_enter_tidak() {
        let f = Fonts::bundled_only();
        let t = tema();
        let n = Rc::new(Cell::new(0u32));
        let catat = n.clone();
        let mut tree =
            pohon(checkbox(&f, &t, "Aktif").on_change(move |_| catat.set(catat.get() + 1)));

        let mut router = InputRouter::new();
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Tab),
                Duration::ZERO,
            )),
        );
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Space),
                Duration::from_millis(20),
            )),
        );
        assert_eq!(n.get(), 1, "Space harus mengaktifkan");

        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Enter),
                Duration::from_millis(40),
            )),
        );
        assert_eq!(n.get(), 1, "Enter milik tombol default, bukan checkbox");
        assert!(node(&tree).is_focused(), "Tab harus memberi fokus");
    }

    #[test]
    fn checkbox_mati_tidak_bisa_diklik_maupun_difokuskan() {
        let f = Fonts::bundled_only();
        let t = tema();
        let n = Rc::new(Cell::new(0u32));
        let catat = n.clone();
        let mut tree = pohon(
            checkbox(&f, &t, "Terkunci")
                .disabled(true)
                .on_change(move |_| catat.set(catat.get() + 1)),
        );
        let id = tree.children(tree.root())[0];
        let tengah = tree.bounds(id).center();

        let mut router = InputRouter::new();
        klik(&mut tree, &mut router, tengah);
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Tab),
                Duration::ZERO,
            )),
        );
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Space),
                Duration::from_millis(20),
            )),
        );
        assert_eq!(n.get(), 0);
        assert!(!node(&tree).is_focused());
    }

    // -- spring -------------------------------------------------------------

    #[test]
    fn lahir_tercentang_langsung_tergambar_tercentang() {
        let f = Fonts::bundled_only();
        let t = tema();
        let tree = pohon(checkbox(&f, &t, "Aktif").checked(true));
        let n = node(&tree);
        assert!(!n.is_animating(), "kontrol tidak beranimasi masuk");
        assert_eq!(n.check_progress(), 1.0);
        assert_eq!(n.background(), t.color.accent);
    }

    #[test]
    fn perubahan_keadaan_menggores_centang_bertahap_lalu_berhenti() {
        let f = Fonts::bundled_only();
        let t = tema();
        let mut tree = pohon(checkbox(&f, &t, "Aktif").checked(false));
        assert_eq!(node(&tree).check_progress(), 0.0);

        reconcile(&mut tree, checkbox(&f, &t, "Aktif").checked(true));
        assert!(node(&tree).is_animating());

        let mut frame = 0;
        let mut pernah_di_tengah = false;
        while node(&tree).is_animating() && frame < 600 {
            detak(&mut tree, Motion::Full);
            let p = node(&tree).check_progress();
            if p > 0.05 && p < 0.95 {
                pernah_di_tengah = true;
            }
            frame += 1;
        }
        assert!(
            frame > 1,
            "centang selesai dalam satu frame = bukan animasi"
        );
        assert!(pernah_di_tengah, "goresan tidak pernah setengah jalan");
        assert_eq!(node(&tree).check_progress(), 1.0);
        assert!(!node(&tree).is_animating(), "spring harus benar-benar diam");
    }

    #[test]
    fn dibatalkan_di_tengah_goresan_berbalik_membawa_kecepatan() {
        let f = Fonts::bundled_only();
        let t = tema();
        let mut tree = pohon(checkbox(&f, &t, "Aktif").checked(false));
        reconcile(&mut tree, checkbox(&f, &t, "Aktif").checked(true));
        for _ in 0..4 {
            detak(&mut tree, Motion::Full);
        }
        let tengah = node(&tree).check_progress();
        assert!(tengah > 0.0 && tengah < 1.0, "belum di tengah: {tengah}");

        // Retarget, bukan animasi baru: posisinya tidak melompat.
        reconcile(&mut tree, checkbox(&f, &t, "Aktif").checked(false));
        assert_eq!(node(&tree).check_progress(), tengah);
        assert!(node(&tree).is_animating());

        let mut frame = 0;
        while node(&tree).is_animating() && frame < 600 {
            detak(&mut tree, Motion::Full);
            frame += 1;
        }
        assert_eq!(node(&tree).check_progress(), 0.0);
    }

    #[test]
    fn reduced_motion_membuang_hiasan_tapi_tidak_membuang_centangnya() {
        let f = Fonts::bundled_only();
        let t = tema();

        // Hiasan (cincin fokus) ditandai dekoratif: langsung selesai.
        let mut tree = pohon(checkbox(&f, &t, "Aktif"));
        let mut router = InputRouter::new();
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Tab),
                Duration::ZERO,
            )),
        );
        detak(&mut tree, Motion::Reduced);
        assert_eq!(
            node(&tree).focus_progress(),
            1.0,
            "cincin dekoratif harus langsung sampai di reduced-motion"
        );

        // Yang menjelaskan (goresan centang) tetap bergerak — hanya tanpa
        // pantulan.
        let mut tree = pohon(checkbox(&f, &t, "Aktif").checked(false));
        reconcile(&mut tree, checkbox(&f, &t, "Aktif").checked(true));
        detak(&mut tree, Motion::Reduced);
        let p = node(&tree).check_progress();
        assert!(p > 0.0 && p < 1.0, "goresan ikut dimatikan: {p}");
    }

    #[test]
    fn rebuild_yang_mengubah_peran_gerakan_benar_benar_berlaku() {
        let f = Fonts::bundled_only();
        let t = tema();

        // Dibangun sebagai gerakan penjelas, lalu aplikasi berubah pikiran.
        let mut tree = pohon(checkbox(&f, &t, "Aktif").checked(false));
        assert_eq!(node(&tree).motion_role(), MotionRole::Essential);

        reconcile(
            &mut tree,
            checkbox(&f, &t, "Aktif").checked(false).decorative(),
        );
        assert_eq!(
            node(&tree).motion_role(),
            MotionRole::Decorative,
            "peran lama dipertahankan diam-diam"
        );

        // Dan peran barunya harus terbaca di perilaku, bukan cuma di field:
        // dekoratif + reduced-motion = tidak ada goresan sama sekali.
        reconcile(
            &mut tree,
            checkbox(&f, &t, "Aktif").checked(true).decorative(),
        );
        detak(&mut tree, Motion::Reduced);
        assert_eq!(
            node(&tree).check_progress(),
            1.0,
            "goresan dekoratif harus langsung sampai di reduced-motion"
        );

        // Perjalanan baliknya juga: dekoratif -> penjelas.
        reconcile(&mut tree, checkbox(&f, &t, "Aktif").checked(true));
        assert_eq!(node(&tree).motion_role(), MotionRole::Essential);
    }

    #[test]
    fn hiasan_tetap_hiasan_walau_peran_dinaikkan_jadi_penjelas() {
        let f = Fonts::bundled_only();
        let t = tema();
        // `press_t`/`ring_t` tidak boleh ikut naik peran: keduanya tidak
        // membawa informasi apa pun, jadi reduced-motion selalu memakannya.
        let mut tree = pohon(checkbox(&f, &t, "Aktif").decorative());
        reconcile(&mut tree, checkbox(&f, &t, "Aktif"));
        let n = node(&tree);
        assert_eq!(n.motion_role(), MotionRole::Essential);
        assert_eq!(n.press_t.role(), MotionRole::Decorative);
        assert_eq!(n.ring_t.role(), MotionRole::Decorative);
    }

    #[test]
    fn tanpa_perubahan_tidak_ada_satu_frame_pun_yang_diminta() {
        let f = Fonts::bundled_only();
        let t = tema();
        let mut tree = pohon(checkbox(&f, &t, "Aktif").checked(true));
        assert!(
            !detak(&mut tree, Motion::Full),
            "checkbox diam harus gratis"
        );
        assert!(!node(&tree).is_animating());
    }

    // -- token --------------------------------------------------------------

    #[test]
    fn warna_dan_bentuk_selalu_datang_dari_token_di_kedua_preset() {
        let f = Fonts::bundled_only();
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut tree = pohon(checkbox(&f, &t, "Aktif").checked(true));
                selesaikan(&mut tree);
                let mut scene = Scene::new(t.color.background);
                tree.paint_into(&mut scene);

                let kotak: Vec<_> = scene
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        Command::Quad(q) => Some(q.clone()),
                        _ => None,
                    })
                    .collect();
                assert!(!kotak.is_empty());
                // Kotaknya digambar lebih dulu, lalu jejak-jejak goresan.
                assert_eq!(kotak[0].background, t.color.accent, "{preset:?}");
                assert_eq!(kotak[0].corners.style, t.radius.style, "{preset:?}");
                assert_eq!(kotak[0].border_color, t.color.accent);

                let jejak = &kotak[1..];
                assert!(jejak.len() > 4, "centang nyaris tidak tergambar");
                for q in jejak {
                    assert_eq!(q.background, t.color.on_accent, "{preset:?}");
                    // Ujung pena selalu busur — squircle preset milik kotaknya.
                    assert_eq!(q.corners.style, CornerStyle::Arc);
                }
            }
        }
    }

    #[test]
    fn keadaan_kosong_benar_benar_gratis() {
        let f = Fonts::bundled_only();
        let t = tema();
        let mut tree = pohon(checkbox(&f, &t, "Aktif").checked(false));
        selesaikan(&mut tree);
        let mut scene = Scene::new(t.color.background);
        tree.paint_into(&mut scene);

        let kotak = scene
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::Quad(_)))
            .count();
        assert_eq!(
            kotak, 1,
            "kotak kosong = satu quad, tanpa goresan sama sekali"
        );
    }

    #[test]
    fn indeterminate_menggambar_garis_bukan_centang() {
        let f = Fonts::bundled_only();
        let t = tema();
        let mut tree = pohon(checkbox(&f, &t, "Semua").state(CheckState::Mixed));
        selesaikan(&mut tree);
        let mut scene = Scene::new(t.color.background);
        tree.paint_into(&mut scene);

        let kotak: Vec<_> = scene
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::Quad(q) => Some(q.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(kotak.len(), 2, "kotak + satu garis, bukan rantai centang");
        let garis = &kotak[1];
        assert!(garis.rect.size.width > garis.rect.size.height);
        assert_eq!(garis.background, t.color.on_accent);
        // Latarnya tetap terisi seperti keadaan tercentang.
        assert_eq!(kotak[0].background, t.color.accent);
    }

    #[test]
    fn cincin_fokus_digambar_di_luar_kotak_agar_centang_tetap_terbaca() {
        let f = Fonts::bundled_only();
        let t = tema();
        let mut tree = pohon(checkbox(&f, &t, "Aktif").checked(true));
        let mut router = InputRouter::new();
        router.dispatch(
            &mut tree,
            &Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Tab),
                Duration::ZERO,
            )),
        );
        selesaikan(&mut tree);
        let kotak_node = node(&tree).box_rect();

        let mut scene = Scene::new(t.color.background);
        tree.paint_into(&mut scene);
        let cincin = scene
            .commands()
            .iter()
            .find_map(|c| match c {
                Command::Quad(q) if q.border_color == t.color.focus_ring => Some(q.clone()),
                _ => None,
            })
            .expect("cincin fokus");
        assert!(cincin.rect.size.width > kotak_node.size.width);
        assert!(cincin.border_width > 0.0);
    }
}
