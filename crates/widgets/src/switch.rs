//! `switch()` / `toggle()` — sakelar on/off Tier 2 (`KOMPONEN.md`), **dengan
//! seretan spring** seperti yang diminta catatan khususnya: *"Spring drag —
//! bisa di-drag, bukan cuma klik (rasa iOS/macOS)"*.
//!
//! ```
//! # use rustui_widgets::{switch, Fonts};
//! # use rustui_theme::{Appearance, Theme};
//! # use rustui_core::signals::Runtime;
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! # let rt = Runtime::new();
//! let wifi = rt.signal(true);
//!
//! switch(&fonts, &t, "Wi-Fi")
//!     .on(wifi.get())
//!     .on_change(move |nyala| wifi.set(nyala));
//! ```
//!
//! ## Kenapa ini node sendiri, bukan pembungkus `Interactive`
//!
//! Karena sakelar adalah satu-satunya kontrol Tier 2 yang **mengikuti jari**.
//! Pembungkus interaktif serba guna hanya mengenal tekan-lepas; sakelar harus
//! menyeret thumb 1:1 sepanjang lintasan, lalu **menyerahkan kecepatan jari ke
//! spring** saat dilepas (REKOMENDASI §3.5: handoff fling → spring, lewat
//! [`VelocityTracker`] milik lapisan input — bukan taksiran sendiri). Dua hal
//! lain ikut menuntut node sendiri: keadaan on/off yang sampai ke screen reader
//! sebagai [`AccessToggled`], dan lintasan kecil di dalam area sentuh ≥ 44pt.
//!
//! ## Siapa yang memiliki nilainya
//!
//! Aplikasi. Node **tidak pernah** mengubah `on`-nya sendiri: ia menceritakan
//! keinginan pengguna lewat [`Builder::on_change`], aplikasi menulis signal, dan
//! nilai barunya kembali lewat rebuild ([`SwitchProps::update`]) — aturan yang
//! sama dengan [`crate::checkbox`] dan [`crate::button`]. Kalau node menebak
//! duluan, sakelar yang perubahannya ditolak aplikasi akan terlihat berpindah
//! selama satu frame.
//!
//! Yang **milik node** hanyalah presentasi: posisi thumb, warna lintasan,
//! tekanan, dan cincin fokus — empat [`SpringValue`] yang dimajukan
//! [`crate::advance`] sekali per frame bersama seluruh pohon.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! - **Kedua preset** — setiap angka lewat [`SwitchStyle::from_theme`];
//!   lintasan 52×32 di Cupertino (HIG 51×31) dan 44×24 di Tailwind/shadcn
//!   (`w-11 h-6`), sudutnya `radius.full` yang di Cupertino squircle dan di
//!   Tailwind arc — parameter shader, bukan konstanta (§2.7, §3.6).
//! - **Semua state interaktif dengan spring** — posisi thumb, warna lintasan
//!   (diam/hover/tekan), pelebaran thumb saat ditekan, dan cincin fokus,
//!   semuanya di-retarget di tengah jalan sambil membawa kecepatan (§3.5).
//! - **Keyboard + focus ring** — Space mengaktifkan; panah kiri/kanan dan
//!   Home/End menyetel nilai **eksplisit** (kebiasaan AppKit dan ARIA: panah
//!   kiri selalu mematikan, tidak pernah sekadar membalik).
//! - **Node AccessKit** — peran [`AccessRole::Switch`], nama dari labelnya,
//!   keadaan [`AccessToggled`], aksi klik + fokus.
//! - **Dark mode** — seluruh warna token, tanpa satu literal pun.
//! - **Hit target ≥ 44pt** — dijamin [`SwitchNode::layout`], bukan pemanggil.
//! - **Reduced-motion** — gerakan yang *menjelaskan* (thumb, warna lintasan)
//!   tetap berjalan tanpa pantulan; yang cuma menghias (pelebaran tekan, cincin
//!   fokus) ditandai [`MotionRole::Decorative`] dan hilang sepenuhnya.

use std::rc::Rc;

use rustui_core::access::{AccessActions, AccessNode, AccessRole, AccessToggled};
use rustui_core::animation::{MotionRole, Spring, SpringValue, Tick};
use rustui_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, KeyCode, NamedKey,
    PointerButton, PointerPhase, VelocityTracker,
};
use rustui_core::scheduler::Dirty;
use rustui_core::signals::Key;
use rustui_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use rustui_core::view::{Builder, View, ViewNode};
use rustui_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, ShadowPair, Size};
use rustui_text::FontWeight;
use rustui_theme::{Preset, RadiusToken, Theme};

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::text::text;

/// Kecepatan (bagian lintasan per detik) yang sudah dianggap lemparan.
///
/// Di atasnya **arah lemparan mengalahkan posisi**: jari yang melempar ke kanan
/// menyalakan sakelar walau baru lewat sepertiga lintasan — perilaku yang sama
/// dengan `UISwitch`.
pub const FLING: f32 = 1.5;

/// Batas atas kecepatan yang boleh diserahkan ke spring, bagian lintasan per
/// detik.
///
/// Satu sampel gila dari driver trackpad tidak boleh melempar thumb entah ke
/// mana (§3.5).
pub const MAX_FLING: f32 = 12.0;

/// Seberapa jauh warna dipudarkan ke arah latar saat sakelar dimatikan.
const REDUP: f32 = 0.5;

// ---------------------------------------------------------------------------
// Warna per keadaan
// ---------------------------------------------------------------------------

/// Tiga warna satu keadaan kontrol: diam, di-hover, ditekan.
///
/// Ketiganya token; komponen tidak pernah menghitung warna sendiri (§2.6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StateColors {
    /// Keadaan diam.
    pub idle: Color,
    /// Penunjuk di atasnya.
    pub hover: Color,
    /// Sedang ditekan.
    pub press: Color,
}

impl StateColors {
    /// Warna yang berlaku untuk keadaan penunjuk saat ini.
    pub fn pick(self, hovered: bool, pressed: bool) -> Color {
        match (pressed, hovered) {
            (true, _) => self.press,
            (false, true) => self.hover,
            _ => self.idle,
        }
    }
}

// ---------------------------------------------------------------------------
// Gaya
// ---------------------------------------------------------------------------

/// Seluruh ukuran, warna, dan bentuk sebuah sakelar — **sudah diresolusi dari
/// token** theme aktif.
///
/// Ukuran lintasan adalah satu-satunya tempat di komponen ini yang perlu tahu
/// preset mana yang aktif: sakelar iOS dan sakelar shadcn memang berbeda
/// ukuran, dan keduanya tetap ditulis sebagai **kelipatan skala spacing**, tidak
/// pernah sebagai angka lepas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwitchStyle {
    /// Ukuran lintasan (track).
    pub track: Size,
    /// Jarak thumb ke tepi lintasan.
    pub inset: f32,
    /// Jarak lintasan ke label.
    pub gap: f32,
    /// Sisi terpendek area sentuh (HIG).
    pub min_target: f32,
    /// Warna lintasan saat mati.
    pub off: StateColors,
    /// Warna lintasan saat nyala.
    pub on: StateColors,
    /// Warna garis tepi lintasan.
    pub border: Color,
    /// Tebal garis tepi lintasan.
    pub border_width: f32,
    /// Warna thumb.
    pub thumb: Color,
    /// Bayangan ganda thumb (ambient + key ala HIG).
    pub thumb_shadow: ShadowPair,
    /// Warna cincin fokus.
    pub focus_ring: Color,
    /// Tebal cincin fokus.
    pub focus_width: f32,
    /// Ke mana warna dipudarkan saat sakelar dimatikan.
    pub dim: Color,
    /// Bentuk sudut "pil": radius penuh dengan geometri preset (§3.6).
    pub pill: Corners,
    /// Pelebaran thumb saat ditekan (rasa iOS).
    pub press_stretch: f32,
}

impl SwitchStyle {
    /// Gaya baku untuk theme aktif.
    pub fn from_theme(theme: &Theme) -> Self {
        let (lebar, tinggi) = match theme.preset {
            // 13 × 8 langkah = 52 × 32pt (HIG: 51 × 31).
            Preset::Cupertino => (13.0, 8.0),
            // 11 × 6 langkah = 44 × 24pt (shadcn `w-11 h-6`).
            Preset::Tailwind => (11.0, 6.0),
        };
        Self {
            track: Size::new(theme.space(lebar), theme.space(tinggi)),
            inset: theme.space(0.5),
            gap: theme.space(2.0),
            min_target: MIN_HIT_TARGET,
            // Lintasan mati = token `separator`: abu-abu tembus pandang di
            // Cupertino (systemFill) dan slate-200/800 di Tailwind — persis
            // warna yang dipakai sakelar aslinya di kedua kiblat.
            off: StateColors {
                idle: theme.color.separator,
                hover: theme.color.surface_hover,
                press: theme.color.surface_pressed,
            },
            on: StateColors {
                idle: theme.color.accent,
                hover: theme.color.accent_hover,
                press: theme.color.accent_pressed,
            },
            border: theme.color.separator,
            border_width: 0.0,
            // Warna yang memang "terbaca di atas accent": putih di kedua
            // preset, dan tetap terbaca di atas lintasan mati.
            thumb: theme.color.on_accent,
            thumb_shadow: theme.shadow.sm,
            focus_ring: theme.color.focus_ring,
            focus_width: theme.space(0.5),
            dim: theme.color.background,
            pill: theme.corners_of(RadiusToken::Full),
            press_stretch: theme.space(1.0),
        }
    }

    /// Garis tengah thumb.
    pub fn thumb_size(self) -> f32 {
        (self.track.height - self.inset * 2.0).max(0.0)
    }

    /// Jarak tempuh thumb dari mati ke nyala.
    ///
    /// Sama dengan `lebar - tinggi`: inset dan garis tengah thumb saling
    /// meniadakan, jadi lintasan yang lebih tebal tidak pernah memendekkan
    /// perjalanannya.
    pub fn travel(self) -> f32 {
        (self.track.width - self.track.height).max(0.0)
    }

    /// Warna lintasan untuk keadaan tertentu.
    pub fn track_for(self, on: bool, disabled: bool, hovered: bool, pressed: bool) -> Color {
        let aktif = !disabled;
        let c = if on {
            self.on.pick(hovered && aktif, pressed && aktif)
        } else {
            self.off.pick(hovered && aktif, pressed && aktif)
        };
        if disabled {
            c.lerp(self.dim, REDUP)
        } else {
            c
        }
    }

    /// Warna thumb untuk keadaan tertentu.
    pub fn thumb_for(self, disabled: bool) -> Color {
        if disabled {
            self.thumb.lerp(self.dim, REDUP)
        } else {
            self.thumb
        }
    }

    /// Kotak thumb di dalam `track` untuk posisi `fraction` (0..1) dan
    /// pelebaran `stretch` poin.
    ///
    /// Pelebarannya tumbuh **menjauhi sisi yang sedang ditempati**: thumb yang
    /// menempel di kanan melar ke kiri, jadi ia tidak pernah keluar lintasan.
    pub fn thumb_rect(self, track: Rect, fraction: f32, stretch: f32) -> Rect {
        let f = fraction.clamp(0.0, 1.0);
        let d = self.thumb_size();
        let s = stretch.max(0.0);
        Rect::new(
            track.origin.x + self.inset + self.travel() * f - s * f,
            track.origin.y + self.inset,
            d + s,
            d,
        )
    }
}

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------

/// Aksi yang menerima nilai baru sebuah sakelar.
#[derive(Clone)]
pub struct SwitchCallback(Rc<dyn Fn(bool)>);

impl SwitchCallback {
    /// Bungkus sebuah closure.
    pub fn new(f: impl Fn(bool) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Jalankan aksinya.
    pub fn call(&self, on: bool) {
        (self.0)(on)
    }
}

impl PartialEq for SwitchCallback {
    /// Identitas, bukan isi — aturan yang sama dengan [`rustui_core::Callback`].
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for SwitchCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SwitchCallback")
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// Seretan yang sedang berlangsung.
#[derive(Debug, Clone)]
struct Seretan {
    /// Koordinat lokal x saat jari menyentuh.
    awal_x: f32,
    /// Posisi thumb saat jari menyentuh.
    awal_fraksi: f32,
    /// Benar begitu jari melewati ambang seret — sebelum itu ini masih ketukan.
    bergeser: bool,
    /// Pelacak kecepatan milik lapisan input, untuk handoff ke spring (§3.5).
    velocity: VelocityTracker,
}

/// Node render sebuah sakelar: lintasan + thumb, dengan label opsional sebagai
/// satu-satunya anak.
pub struct SwitchNode {
    /// Ukuran, warna, dan bentuk — semuanya token.
    pub style: SwitchStyle,
    /// Nilai sakelar. **Milik aplikasi**; node tidak pernah mengubahnya sendiri.
    pub on: bool,
    /// Tidak bisa dipakai (tetap dibacakan sebagai dimmed).
    pub disabled: bool,
    /// Nama yang dibacakan screen reader.
    pub label: Option<String>,
    /// Peran fokus keyboard.
    pub focus: FocusPolicy,
    /// Apa yang dijalankan saat pengguna meminta nilai baru.
    pub on_change: Option<SwitchCallback>,

    /// Posisi thumb (0 = mati, 1 = nyala).
    progress: SpringValue<f32>,
    /// Warna lintasan.
    bg: SpringValue<Color>,
    /// Pelebaran thumb saat ditekan (dekoratif).
    press_t: SpringValue<f32>,
    /// Kemunculan cincin fokus (dekoratif).
    ring_t: SpringValue<f32>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    activations: u32,
    track_rect: Rect,
    seret: Option<Seretan>,
}

impl SwitchNode {
    /// Node baru yang **sudah berada** di keadaan `on` — tanpa animasi masuk.
    pub fn new(style: SwitchStyle, on: bool, disabled: bool, spring: Spring) -> Self {
        Self {
            style,
            on,
            disabled,
            label: None,
            focus: FocusPolicy::FOCUSABLE,
            on_change: None,
            progress: SpringValue::new(if on { 1.0 } else { 0.0 })
                .with_spring(spring)
                // Satuan posisinya **bagian lintasan**, bukan poin: toleransi
                // kecepatan yang lebih longgar mencegah ekor gerakan yang tak
                // terlihat tapi terus meminta frame (§3.5).
                .with_tolerance(rustui_core::animation::Tolerance::new(
                    1.0 / 512.0,
                    1.0 / 64.0,
                )),
            bg: SpringValue::new(style.track_for(on, disabled, false, false)).with_spring(spring),
            press_t: SpringValue::new(0.0).with_spring(spring).decorative(),
            ring_t: SpringValue::new(0.0).with_spring(spring).decorative(),
            hovered: false,
            pressed: false,
            focused: false,
            activations: 0,
            track_rect: Rect::new(0.0, 0.0, style.track.width, style.track.height),
            seret: None,
        }
    }

    /// Nilai sakelar.
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Gaya yang sedang dipakai.
    pub fn style(&self) -> SwitchStyle {
        self.style
    }

    /// Tidak bisa dipakai.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Kotak lintasan hasil layout terakhir (koordinat lokal node).
    pub fn track_rect(&self) -> Rect {
        self.track_rect
    }

    /// Posisi thumb yang digambar frame ini (0..1).
    pub fn fraction(&self) -> f32 {
        self.progress.position().clamp(0.0, 1.0)
    }

    /// Warna lintasan yang digambar frame ini.
    pub fn track_color(&self) -> Color {
        self.bg.position()
    }

    /// Warna lintasan yang sedang dituju.
    pub fn track_target(&self) -> Color {
        self.bg.target()
    }

    /// Kemajuan pelebaran thumb (0..1).
    pub fn press_progress(&self) -> f32 {
        self.press_t.position()
    }

    /// Kemajuan cincin fokus (0..1).
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

    /// Benar bila jari sedang benar-benar menyeret thumb.
    pub fn is_dragging(&self) -> bool {
        self.seret.as_ref().is_some_and(|s| s.bergeser)
    }

    /// Berapa kali pengguna mengaktifkannya sejak node dibuat.
    pub fn activations(&self) -> u32 {
        self.activations
    }

    /// Nilai yang sedang **terlihat**: saat diseret, sisi lintasan tempat thumb
    /// berada — bukan nilai yang masih dipegang aplikasi.
    ///
    /// Inilah yang membuat warna lintasan berganti tepat saat thumb melewati
    /// tengah, bukan sesaat setelah jari diangkat.
    pub fn visual_on(&self) -> bool {
        match &self.seret {
            Some(s) if s.bergeser => self.progress.position() >= 0.5,
            _ => self.on,
        }
    }

    /// Benar bila masih ada spring yang bergerak.
    pub fn is_animating(&self) -> bool {
        self.progress.is_animating()
            || self.bg.is_animating()
            || self.press_t.is_animating()
            || self.ring_t.is_animating()
    }

    /// Arahkan seluruh spring ke keadaan sekarang.
    ///
    /// **Retarget, bukan animasi baru** (§3.5): sakelar yang dibalik dua kali
    /// dengan cepat berbalik arah membawa kecepatannya. Satu fungsi untuk empat
    /// nilai, dipanggil setiap kali apa pun berubah — dengan begitu tidak
    /// mungkin ada satu spring yang lupa di-retarget dan tertinggal menampilkan
    /// keadaan kemarin.
    fn retarget(&mut self) {
        let aktif = !self.disabled;
        // Selama jari menempel, posisi thumb **milik jari**: spring tidak boleh
        // menariknya ke mana-mana.
        if !self.is_dragging() {
            self.progress.set_target(if self.on { 1.0 } else { 0.0 });
        }
        let tampak = self.visual_on();
        self.bg.set_target(
            self.style
                .track_for(tampak, self.disabled, self.hovered, self.pressed),
        );
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
        bergeser |= maju(&mut self.progress, tick);
        bergeser |= maju_warna(&mut self.bg, tick);
        bergeser |= maju(&mut self.press_t, tick);
        bergeser |= maju(&mut self.ring_t, tick);
        bergeser
    }

    /// Selesaikan seluruh gerakan seketika (uji, snapshot, golden test).
    pub fn settle(&mut self) {
        self.progress.settle();
        self.bg.settle();
        self.press_t.settle();
        self.ring_t.settle();
    }

    /// Minta nilai `baru` ke aplikasi.
    ///
    /// Node **tidak** mengubah `on`-nya sendiri (lihat dokumentasi modul).
    /// Callback disalin keluar dulu: ia hampir selalu menulis signal, dan itu
    /// tidak boleh terjadi sambil node ini dipinjam `&mut`.
    fn minta(&mut self, baru: bool) {
        if self.disabled || baru == self.on {
            return;
        }
        self.activations = self.activations.saturating_add(1);
        if let Some(cb) = self.on_change.clone() {
            cb.call(baru);
        }
    }

    /// Ambang jari berubah dari ketukan menjadi seretan, poin logis.
    fn ambang_seret(&self) -> f32 {
        (self.style.inset * 2.0).max(1.0)
    }

    /// Kotak thumb yang benar-benar digambar frame ini.
    fn thumb_tergambar(&self) -> Rect {
        let stretch = self.press_t.position() * self.style.press_stretch;
        self.style
            .thumb_rect(self.track_rect, self.fraction(), stretch)
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

impl RenderNode for SwitchNode {
    fn type_name(&self) -> &'static str {
        "Switch"
    }

    /// Lintasan di sisi awal baca, label mengikuti, dan **area sentuh ≥ 44pt**.
    ///
    /// RTL ditangani di sini dan hanya di sini: lintasannya pindah ke kanan
    /// bersama isinya, karena arah baca adalah urusan layout — bukan urusan
    /// tiap widget menghitungnya sendiri (§9.8).
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let t = self.style.track;

        if ctx.child_count() == 0 {
            let size = constraints.constrain(Size::new(
                t.width.max(self.style.min_target),
                t.height.max(self.style.min_target),
            ));
            self.track_rect = Rect::new(
                ((size.width - t.width) * 0.5).max(0.0),
                ((size.height - t.height) * 0.5).max(0.0),
                t.width,
                t.height,
            );
            return size;
        }

        let depan = t.width + self.style.gap;
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
            ukuran_anak.height.max(t.height).max(self.style.min_target),
        ));
        let y_track = ((size.height - t.height) * 0.5).max(0.0);
        let y_anak = ((size.height - ukuran_anak.height) * 0.5).max(0.0);

        if ctx.direction().is_rtl() {
            self.track_rect = Rect::new(size.width - t.width, y_track, t.width, t.height);
            ctx.place_child(
                anak,
                Point::new((size.width - depan - ukuran_anak.width).max(0.0), y_anak),
            );
        } else {
            self.track_rect = Rect::new(0.0, y_track, t.width, t.height);
            ctx.place_child(anak, Point::new(depan, y_anak));
        }
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let s = self.style;
        let track = self.track_rect;
        if track.size.is_empty() {
            return;
        }
        let pill = s.pill.clamp_to(track.size);

        // Cincin fokus digambar **di luar** lintasan supaya tidak menutupi
        // isinya, dan tumbuh dengan spring — bukan berkedip muncul.
        let ring = self.ring_t.position();
        if ring > 0.0 && s.focus_width > 0.0 && !self.disabled {
            let w = s.focus_width * ring;
            ctx.quad(
                Quad::new(track.deflate(Insets::all(-w)))
                    .corners(Corners::new(
                        CornerRadii::all(pill.radii.max() + w),
                        pill.style,
                    ))
                    .border(w, s.focus_ring),
            );
        }

        let mut lintasan = Quad::new(track)
            .background(self.bg.position())
            .corners(pill);
        if s.border_width > 0.0 {
            lintasan = lintasan.border(s.border_width, s.border);
        }
        ctx.quad(lintasan);

        let thumb = self.thumb_tergambar();
        if !thumb.size.is_empty() {
            ctx.shadowed(
                Quad::new(thumb)
                    .background(s.thumb_for(self.disabled))
                    .corners(s.pill.clamp_to(thumb.size)),
                if self.disabled {
                    ShadowPair::NONE
                } else {
                    s.thumb_shadow
                },
            );
        }
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Switch;
        node.label.clone_from(&self.label);
        node.toggled = Some(AccessToggled::from(self.on));
        node.disabled = self.disabled;
        if !self.disabled {
            node.actions |= AccessActions::CLICK;
            if self.focus.focusable {
                node.actions |= AccessActions::FOCUS;
            }
        }
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Sakelar yang dimatikan tetap **menyerap** penunjuk: kliknya tidak
        // boleh tembus ke baris di belakangnya.
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
                        ctx.request_paint();
                    }
                }
                PointerPhase::Leave => {
                    if self.hovered {
                        self.hovered = false;
                        self.retarget();
                        ctx.request_animation();
                        ctx.request_paint();
                    }
                }
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    let mut velocity = VelocityTracker::new();
                    velocity.add(p.time, ctx.local());
                    self.seret = Some(Seretan {
                        awal_x: ctx.local().x,
                        awal_fraksi: self.progress.position(),
                        bergeser: false,
                        velocity,
                    });
                    self.pressed = true;
                    self.retarget();
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.request_animation();
                    ctx.request_paint();
                    ctx.handled();
                }
                PointerPhase::Move => {
                    let ambang = self.ambang_seret();
                    let travel = self.style.travel();
                    let lokal = ctx.local();
                    let Some(s) = self.seret.as_mut() else {
                        return;
                    };
                    s.velocity.add(p.time, lokal);
                    let dx = lokal.x - s.awal_x;
                    if !s.bergeser && dx.abs() >= ambang {
                        s.bergeser = true;
                    }
                    if s.bergeser && travel > 0.0 {
                        // Thumb mengikuti jari **1:1**, tanpa spring: kontrol
                        // yang "tertinggal" dari jari terasa rusak.
                        let f = (s.awal_fraksi + dx / travel).clamp(0.0, 1.0);
                        self.progress.jump_to(f);
                        self.retarget();
                        ctx.request_animation();
                        ctx.request_paint();
                    }
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let travel = self.style.travel();
                    let di_dalam = {
                        let size = ctx.size();
                        let l = ctx.local();
                        l.x >= 0.0 && l.y >= 0.0 && l.x < size.width && l.y < size.height
                    };
                    let selesai = self.seret.take();
                    self.pressed = false;
                    ctx.release_pointer();
                    ctx.request_animation();
                    ctx.request_paint();
                    ctx.handled();

                    match selesai {
                        // Seretan: posisi **dan** kecepatan jari yang
                        // memutuskan, lalu kecepatan itu diserahkan ke spring
                        // apa adanya (§3.5).
                        Some(s) if s.bergeser => {
                            let f = self.progress.position().clamp(0.0, 1.0);
                            let v = if travel > 0.0 {
                                (s.velocity.velocity().x / travel).clamp(-MAX_FLING, MAX_FLING)
                            } else {
                                0.0
                            };
                            let baru = if v.abs() >= FLING { v > 0.0 } else { f >= 0.5 };
                            self.progress.set_velocity(v);
                            // Retarget lebih dulu supaya thumb tetap bergerak
                            // walau aplikasi menolak perubahannya.
                            self.retarget();
                            self.minta(baru);
                        }
                        // Ketukan biasa — dan seperti tombol AppKit, jari yang
                        // ditarik keluar sebelum dilepas berarti batal.
                        Some(_) if di_dalam => {
                            self.retarget();
                            self.minta(!self.on);
                        }
                        _ => self.retarget(),
                    }
                }
                // Dibatalkan OS ≠ dilepas: tidak ada perubahan nilai, dan thumb
                // kembali ke tempatnya.
                PointerPhase::Cancel => {
                    if self.seret.take().is_some() || self.pressed {
                        self.pressed = false;
                        self.retarget();
                        ctx.request_animation();
                        ctx.request_paint();
                    }
                }
                _ => {}
            },

            Event::Key(k) if k.is_pressed() && k.modifiers.is_empty() => match k.code {
                // Space mengaktifkan kontrol on/off — di HIG maupun di web.
                // Enter sengaja tidak: ia milik tombol default sebuah form.
                KeyCode::Named(NamedKey::Space) => {
                    self.minta(!self.on);
                    ctx.request_animation();
                    ctx.request_paint();
                    ctx.handled();
                }
                KeyCode::Named(NamedKey::ArrowLeft) | KeyCode::Named(NamedKey::Home) => {
                    self.minta(false);
                    ctx.request_animation();
                    ctx.handled();
                }
                KeyCode::Named(NamedKey::ArrowRight) | KeyCode::Named(NamedKey::End) => {
                    self.minta(true);
                    ctx.request_animation();
                    ctx.handled();
                }
                _ => {}
            },

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.pressed = false;
                    self.seret = None;
                }
                self.retarget();
                ctx.request_animation();
                ctx.request_paint();
            }

            _ => {}
        }
    }
}

impl core::fmt::Debug for SwitchNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SwitchNode")
            .field("on", &self.on)
            .field("fraction", &self.fraction())
            .field("disabled", &self.disabled)
            .field("hovered", &self.hovered)
            .field("pressed", &self.pressed)
            .field("focused", &self.focused)
            .field("dragging", &self.is_dragging())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props [`SwitchNode`] — bentuk view sebuah sakelar.
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchProps {
    style: SwitchStyle,
    on: bool,
    disabled: bool,
    label: Option<String>,
    focus: FocusPolicy,
    spring: Spring,
    motion: MotionRole,
    on_change: Option<SwitchCallback>,
}

impl SwitchProps {
    /// Props bawaan untuk theme aktif.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            style: SwitchStyle::from_theme(theme),
            on: false,
            disabled: false,
            label: None,
            focus: FocusPolicy::FOCUSABLE,
            spring: Spring::snappy(),
            motion: MotionRole::Essential,
            on_change: None,
        }
    }
}

impl ViewNode for SwitchProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut node = SwitchNode::new(self.style, self.on, self.disabled, self.spring);
        node.label.clone_from(&self.label);
        node.focus = self.focus;
        node.on_change.clone_from(&self.on_change);
        if self.motion == MotionRole::Decorative {
            // Aplikasi yang menyatakan gerakan ini sekadar hiasan: reduced-
            // motion mematikannya sepenuhnya, bukan cuma membuang pantulannya.
            node.progress = node.progress.decorative();
            node.bg = node.bg.decorative();
        }
        Box::new(node)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<SwitchNode>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.style != self.style {
            // Ukuran lintasan dan jarak ke label ikut di sini, jadi preset yang
            // berganti memang harus di-layout ulang — bukan cuma digambar ulang.
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
        if n.progress.spring() != self.spring {
            n.progress.set_spring(self.spring);
            n.bg.set_spring(self.spring);
            n.press_t.set_spring(self.spring);
            n.ring_t.set_spring(self.spring);
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                // Kontrol yang baru dimatikan tidak boleh membeku dalam keadaan
                // ditekan/hover: penunjuknya tidak akan pernah datang lagi.
                n.pressed = false;
                n.hovered = false;
                n.seret = None;
            }
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.on != self.on {
            n.on = self.on;
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

/// Builder sebuah sakelar.
///
/// Tipe sendiri, bukan [`Builder`], karena label dan sakelarnya baru dirakit
/// saat pohon dibentuk: [`switch_only`] tetap punya nama a11y tanpa satu glyph
/// pun, dan warna labelnya ikut keadaan `disabled` yang bisa saja disetel
/// belakangan di rantai method.
pub struct Switch {
    fonts: Option<Fonts>,
    theme: Theme,
    label: Option<String>,
    style: SwitchStyle,
    on: bool,
    disabled: bool,
    spring: Spring,
    motion: MotionRole,
    focus: FocusPolicy,
    on_change: Option<SwitchCallback>,
    key: Option<Key>,
}

/// Sakelar berlabel.
///
/// Labelnya ikut bisa diklik **dan sekaligus** menjadi nama yang dibacakan
/// screen reader — satu sumber, jadi tidak mungkin yang terlihat dan yang
/// terdengar berbeda.
///
/// ```
/// # use rustui_widgets::{switch, Fonts};
/// # use rustui_theme::{Appearance, Theme};
/// # let fonts = Fonts::bundled_only();
/// # let t = Theme::tailwind(Appearance::Light);
/// switch(&fonts, &t, "Mode pesawat")
///     .on(true)
///     .on_change(|nyala| println!("sekarang {nyala}"));
/// ```
pub fn switch(fonts: &Fonts, theme: &Theme, label: impl Into<String>) -> Switch {
    Switch {
        fonts: Some(fonts.clone()),
        label: Some(label.into()),
        ..switch_only(theme)
    }
}

/// Nama lain [`switch`] — `KOMPONEN.md` menyebut komponen ini
/// "`switch` / `toggle`".
pub fn toggle(fonts: &Fonts, theme: &Theme, label: impl Into<String>) -> Switch {
    switch(fonts, theme, label)
}

/// Sakelar tanpa label terlihat — di dalam sel tabel atau di ujung baris
/// daftar yang sudah punya judulnya sendiri.
///
/// Tetap **wajib** punya nama lewat [`Switch::label`]: kontrol tanpa nama
/// adalah kontrol yang tidak ada bagi screen reader (§3.8), dan itu bug, bukan
/// pilihan desain.
///
/// ```
/// # use rustui_widgets::switch_only;
/// # use rustui_theme::{Appearance, Theme};
/// # let t = Theme::cupertino(Appearance::Light);
/// switch_only(&t).label("Wi-Fi").on(true);
/// ```
pub fn switch_only(theme: &Theme) -> Switch {
    Switch {
        fonts: None,
        theme: *theme,
        label: None,
        style: SwitchStyle::from_theme(theme),
        on: false,
        disabled: false,
        spring: Spring::snappy(),
        motion: MotionRole::Essential,
        focus: FocusPolicy::FOCUSABLE,
        on_change: None,
        key: None,
    }
}

impl Switch {
    /// Kunci identitas — wajib untuk anggota daftar dinamis (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Nilai sakelar.
    pub fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    /// Nama lain [`Switch::on`], senada dengan `checkbox`.
    pub fn checked(self, checked: bool) -> Self {
        self.on(checked)
    }

    /// Nama yang dibacakan screen reader.
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

    /// Apa yang dijalankan saat **pengguna** meminta nilai baru.
    ///
    /// Tidak dipanggil saat aplikasi sendiri yang menulis nilainya — sama
    /// seperti `onChanged` di Flutter.
    pub fn on_change(mut self, f: impl Fn(bool) + 'static) -> Self {
        self.on_change = Some(SwitchCallback::new(f));
        self
    }

    /// Nama lain [`Switch::on_change`], senada dengan `checkbox`.
    pub fn on_toggle(self, f: impl Fn(bool) + 'static) -> Self {
        self.on_change(f)
    }

    /// Ganti spring-nya (`smooth`/`snappy`/`bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Tandai gerakan thumb **dekoratif**: reduced-motion mematikannya
    /// sepenuhnya alih-alih sekadar membuang pantulannya.
    ///
    /// Bawaannya [`MotionRole::Essential`] — thumb yang bergeser *menjelaskan*
    /// perubahan nilai, jadi menghapusnya berarti menghapus informasi.
    pub fn decorative(mut self) -> Self {
        self.motion = MotionRole::Decorative;
        self
    }

    /// Gaya kustom — jarang dipakai; bawaannya sudah token.
    pub fn style(mut self, style: SwitchStyle) -> Self {
        self.style = style;
        self
    }
}

impl From<Switch> for View {
    fn from(s: Switch) -> View {
        let t = s.theme;
        let mut builder = Builder::new(SwitchProps {
            style: s.style,
            on: s.on,
            disabled: s.disabled,
            label: s.label.clone(),
            focus: s.focus,
            spring: s.spring,
            motion: s.motion,
            on_change: s.on_change,
        });

        // Label hanya digambar bila memang ada mesin teksnya; `switch_only`
        // tetap punya nama a11y tanpa satu glyph pun.
        if let (Some(fonts), Some(label)) = (s.fonts, s.label) {
            let warna = if s.disabled {
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
                    // Nama kontrol dibacakan sekali, dari node sakelarnya —
                    // bukan dua kali (aturan yang sama dengan `button`).
                    .role(AccessRole::Container),
            );
        }
        if let Some(key) = s.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for Switch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Switch")
            .field("label", &self.label)
            .field("on", &self.on)
            .field("disabled", &self.disabled)
            .field("key", &self.key)
            .finish()
    }
}

#[cfg(test)]
mod tests;
