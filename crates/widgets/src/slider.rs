//! `slider()` — komponen Tier 2 (`KOMPONEN.md`): **geser nilai** dengan drag,
//! klik di track, dan keyboard; plus varian **range** dengan dua thumb.
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_theme::{Appearance, Theme};
//! use silka_widgets::{range_slider, slider};
//!
//! # let rt = Runtime::new();
//! # let volume = rt.signal(40.0f32);
//! # let t = Theme::cupertino(Appearance::Dark);
//! slider(&t, volume.get())
//!     .range(0.0..=100.0)
//!     .step(5.0)
//!     .label("Volume")
//!     .on_change(move |v| volume.set(v));
//!
//! // Dua thumb: rentang harga, jam kerja, filter tabel.
//! range_slider(&t, 20.0, 80.0).range(0.0..=100.0).label("Harga");
//! ```
//!
//! Berbeda dengan [`crate::button`], slider **bukan** komposisi dari
//! `interactive`: nilainya kontinu, thumb-nya berpindah mengikuti jari, dan
//! geometri tracknya harus dipahami hit-testing maupun keyboard. Karena itu ia
//! sebuah [`RenderNode`] tersendiri — tapi kosakatanya tetap yang itu-itu juga:
//! perintah gambar `silka-paint`, node [`AccessNode`], dan spring
//! `silka-core`. Tidak ada satu pun angka warna atau tipe wgpu di berkas ini
//! (§2.6, §3.2).
//!
//! ## Definition of Done (`KOMPONEN.md`) — bagaimana masing-masing dipenuhi
//!
//! | Butir | Di mana |
//! |---|---|
//! | Benar di kedua preset | [`SliderStyle::from_theme`] — seluruh nilai token |
//! | State interaktif dengan spring | [`Slider::advance`]: posisi thumb + "lift" hover/press |
//! | Keyboard penuh + focus ring | panah/Home/End/PageUp/PageDown, cincin di thumb aktif |
//! | Node AccessKit | peran [`AccessRole::Slider`] + nilai + aksi increment/decrement/set |
//! | Dark mode | token yang sama, appearance yang berbeda |
//! | Hit target ≥ 44pt | tinggi node dikunci [`crate::MIN_HIT_TARGET`] walau tracknya 4pt |
//! | Reduced-motion | spring "lift" dekoratif (hilang), spring posisi tetap menjelaskan |
//!
//! ## Catatan tentang pompa animasi
//!
//! Seluruh gerakan dimajukan di satu tempat: [`crate::motion::advance`], yang
//! dipanggil aplikasi sekali per frame lewat
//! [`silka_core::app::AppRuntime::animate`] (atau `run_app_with`) — aturan
//! yang sama untuk setiap komponen beranimasi di crate ini, bukan siklus frame
//! kedua milik slider.
//!
//! Yang perlu diingat penulis uji dan snapshot: **nilai** slider tidak pernah
//! menunggu animasi. Ia berubah seketika (dan itulah yang dibacakan screen
//! reader); yang menyusul lewat spring hanyalah posisi thumb yang digambar.
//! Pohon yang sengaja tidak dipompa cukup memanggil [`crate::motion::settle`]
//! untuk mendapatkan gambar akhirnya.

use std::ops::RangeInclusive;
use std::rc::Rc;

use silka_core::access::{
    AccessAction, AccessActionRequest, AccessActions, AccessNode, AccessRole,
};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{
    BoxConstraints, LayoutCtx, NodeId, PaintCtx, RenderNode, RenderTree, TextDirection,
};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, CornerRadii, CornerStyle, Corners, Quad, Rect, ShadowPair, Size};
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;

/// Jumlah thumb terbanyak yang didukung satu slider (varian range).
pub const MAX_THUMBS: usize = 2;

/// Berapa langkah yang dilompati PageUp/PageDown.
pub const PAGE_STEPS: f32 = 10.0;

// ---------------------------------------------------------------------------
// Callback
// ---------------------------------------------------------------------------

/// Aksi yang dititipkan aplikasi untuk menerima nilai baru.
///
/// Selalu membawa **dua** nilai (awal, akhir) supaya slider biasa dan slider
/// range memakai satu jalur yang sama; slider satu thumb mengirim nilainya di
/// kedua posisi. Sifatnya persis [`silka_core::Callback`]: `Clone` murah,
/// kesamaan berdasarkan identitas, dan yang boleh dilakukannya hanyalah
/// menulis signal.
#[derive(Clone)]
pub struct ChangeCallback(Rc<dyn Fn(f32, f32)>);

impl ChangeCallback {
    /// Bungkus sebuah closure penerima nilai.
    pub fn new(f: impl Fn(f32, f32) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Jalankan dengan pasangan nilai terbaru.
    pub fn call(&self, start: f32, end: f32) {
        (self.0)(start, end)
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

/// Seluruh nilai tampilan slider, **sudah diresolusi dari token theme**.
///
/// Node render tidak pernah mengenal [`Theme`] (§2.7): yang menyeberang ke
/// bawah hanya angka dan warna jadi, sehingga preset Cupertino/Tailwind
/// berganti tanpa satu baris pun berubah di mesin. Geometri sudut ikut sebagai
/// **parameter** ([`SliderStyle::corner_style`]), bukan konstanta — squircle
/// dan arc sama sahnya (§3.6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderStyle {
    /// Warna track yang belum terisi (token `surface_sunken`).
    pub track: Color,
    /// Warna bagian terisi (token `accent`).
    pub fill: Color,
    /// Warna bagian terisi saat hover/tekan (token `accent_hover`).
    pub fill_hover: Color,
    /// Warna bagian terisi saat kontrol dimatikan (token `accent_muted`).
    pub fill_disabled: Color,
    /// Warna isi thumb (token `surface_elevated`).
    pub thumb: Color,
    /// Warna garis tepi thumb (token `separator`).
    pub thumb_border: Color,
    /// Tebal garis tepi thumb.
    pub thumb_border_width: f32,
    /// Warna cincin fokus keyboard (token `focus_ring`).
    pub focus_ring: Color,
    /// Tebal cincin fokus.
    pub focus_ring_width: f32,
    /// Bayangan ganda thumb (token `shadow.sm`).
    pub shadow: ShadowPair,
    /// Tebal track, poin logis.
    pub track_height: f32,
    /// Diameter thumb saat diam.
    pub thumb_size: f32,
    /// Pertambahan diameter thumb saat hover/ditekan (micro-interaction §3.6).
    pub thumb_grow: f32,
    /// Geometri sudut aktif — squircle di Cupertino, arc di Tailwind.
    pub corner_style: CornerStyle,
    /// Tinggi minimum kotak sentuh (HIG: 44pt).
    pub min_height: f32,
    /// Lebar yang dipakai bila constraints tidak membatasi lebar sama sekali.
    pub preferred_width: f32,
}

impl SliderStyle {
    /// Resolusi seluruh nilai dari theme aktif — **satu-satunya** pintu dari
    /// token ke slider.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            track: theme.color.surface_sunken,
            fill: theme.color.accent,
            fill_hover: theme.color.accent_hover,
            fill_disabled: theme.color.accent_muted,
            thumb: theme.color.surface_elevated,
            thumb_border: theme.color.separator,
            thumb_border_width: theme.space(0.25),
            focus_ring: theme.color.focus_ring,
            focus_ring_width: theme.space(0.5),
            shadow: theme.shadow.sm,
            track_height: theme.space(1.0),
            thumb_size: theme.space(5.0),
            thumb_grow: theme.space(0.5),
            corner_style: theme.radius.style,
            min_height: MIN_HIT_TARGET,
            preferred_width: theme.space(60.0),
        }
    }
}

// ---------------------------------------------------------------------------
// Geometri
// ---------------------------------------------------------------------------

/// Tata letak sebuah slider: track, rentang gerak thumb, dan garis tengahnya.
///
/// Fungsi murni dari (ukuran, style) — dipakai bersama oleh layout, paint,
/// hit-testing, dan uji. Karena hanya ada satu sumber, mustahil ada thumb yang
/// digambar di tempat berbeda dari tempat ia bisa ditangkap jari.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderGeometry {
    /// Kotak track penuh (koordinat lokal node).
    pub track: Rect,
    /// Posisi x pusat thumb pada ujung "awal" visual.
    pub start_x: f32,
    /// Posisi x pusat thumb pada ujung "akhir" visual.
    pub end_x: f32,
    /// Garis tengah vertikal node.
    pub center_y: f32,
}

impl SliderGeometry {
    /// Hitung geometri untuk sebuah ukuran node.
    pub fn new(size: Size, style: &SliderStyle) -> Self {
        let jari = (style.thumb_size.max(0.0) + style.thumb_grow.max(0.0)) * 0.5;
        let center_y = size.height * 0.5;
        let tebal = style.track_height.clamp(0.0, size.height.max(0.0));
        let track = Rect::new(0.0, center_y - tebal * 0.5, size.width.max(0.0), tebal);
        // Thumb tidak pernah keluar dari kotak node: pusatnya berhenti satu
        // jari-jari (termasuk pembesaran saat ditekan) dari tiap tepi.
        let start_x = jari.min(size.width.max(0.0) * 0.5);
        let end_x = (size.width.max(0.0) - jari).max(start_x);
        Self {
            track,
            start_x,
            end_x,
            center_y,
        }
    }

    /// Jarak tempuh pusat thumb, poin logis.
    pub fn travel(&self) -> f32 {
        self.end_x - self.start_x
    }

    /// Posisi x pusat thumb untuk nilai ternormalisasi `t` (0..1).
    ///
    /// **Mirroring RTL ada di sini**, bukan di pemanggil (§9.8): pada arah
    /// kanan-ke-kiri nilai terbesar berada di kiri, dan itu satu-satunya
    /// tempat yang perlu tahu.
    pub fn thumb_x(&self, t: f32, direction: TextDirection) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let u = if direction.is_rtl() { 1.0 - t } else { t };
        self.start_x + self.travel() * u
    }

    /// Kebalikan [`SliderGeometry::thumb_x`]: nilai ternormalisasi di titik x.
    pub fn t_at(&self, x: f32, direction: TextDirection) -> f32 {
        let travel = self.travel();
        let u = if travel <= 0.0 {
            0.0
        } else {
            ((x - self.start_x) / travel).clamp(0.0, 1.0)
        };
        if direction.is_rtl() {
            1.0 - u
        } else {
            u
        }
    }

    /// Ujung track tempat isian dimulai untuk slider satu thumb.
    fn anchor_x(&self, direction: TextDirection) -> f32 {
        if direction.is_rtl() {
            self.track.max_x()
        } else {
            self.track.min_x()
        }
    }
}

// ---------------------------------------------------------------------------
// Nilai
// ---------------------------------------------------------------------------

/// Bulatkan `value` ke kelipatan `step` terdekat dari `min`, lalu jepit ke
/// rentang.
///
/// `step` nol atau negatif berarti kontinu. Fungsi murni: inilah "snap ke step"
/// yang diminta `KOMPONEN.md`, dan ia diuji tanpa menyentuh pohon sama sekali.
pub fn snap(value: f32, min: f32, max: f32, step: Option<f32>) -> f32 {
    let (min, max) = if min <= max { (min, max) } else { (max, min) };
    if !value.is_finite() {
        return min;
    }
    let v = value.clamp(min, max);
    match step {
        Some(s) if s > 0.0 && s.is_finite() => {
            let n = ((v - min) / s).round();
            (min + n * s).clamp(min, max)
        }
        _ => v,
    }
}

/// Nilai → posisi 0..1 dalam rentang (`min == max` selalu 0).
pub fn normalize(value: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if span.abs() <= f32::EPSILON {
        0.0
    } else {
        ((value - min) / span).clamp(0.0, 1.0)
    }
}

/// Posisi 0..1 → nilai dalam rentang.
pub fn denormalize(t: f32, min: f32, max: f32) -> f32 {
    min + (max - min) * t.clamp(0.0, 1.0)
}

/// Teks nilai untuk screen reader: bilangan bulat tetap terbaca bulat.
fn teks_angka(v: f32) -> String {
    if (v - v.round()).abs() < 1e-4 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.2}")
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// Node render sebuah slider (satu atau dua thumb).
#[derive(Debug)]
pub struct Slider {
    /// Batas bawah rentang.
    pub min: f32,
    /// Batas atas rentang.
    pub max: f32,
    /// Kelipatan yang boleh ditempati nilai; `None` = kontinu.
    pub step: Option<f32>,
    /// Jumlah thumb: 1 (slider biasa) atau 2 (range).
    pub thumbs: usize,
    /// Tidak bisa dipakai — tetap dibacakan screen reader sebagai dimmed.
    pub disabled: bool,
    /// Nama yang dibacakan screen reader.
    pub label: Option<String>,
    /// Nilai tampilan, sudah diresolusi dari token.
    pub style: SliderStyle,
    /// Apa yang dijalankan setiap kali nilainya berubah karena pengguna.
    pub on_change: Option<ChangeCallback>,

    /// Nilai kedua thumb (indeks 1 diabaikan bila `thumbs == 1`).
    values: [f32; MAX_THUMBS],
    /// Posisi thumb ternormalisasi — **inilah yang digambar**, dan ia spring.
    pos: [SpringValue<f32>; MAX_THUMBS],
    /// Derajat "terangkat" tiap thumb (hover/tekan), 0..1.
    lift: [SpringValue<f32>; MAX_THUMBS],

    hovered: bool,
    hover_thumb: usize,
    dragging: Option<usize>,
    /// Selisih antara titik tekan dan pusat thumb saat drag dimulai.
    grab: f32,
    focused: bool,
    active: usize,
    direction: TextDirection,
}

impl Default for Slider {
    fn default() -> Self {
        let style = SliderStyle::from_theme(&Theme::default());
        Self::baru(0.0, 1.0, [0.0, 1.0], 1, style, Spring::snappy())
    }
}

impl Slider {
    fn baru(
        min: f32,
        max: f32,
        values: [f32; MAX_THUMBS],
        thumbs: usize,
        style: SliderStyle,
        spring: Spring,
    ) -> Self {
        let t0 = normalize(values[0], min, max);
        let t1 = normalize(values[1], min, max);
        Self {
            min,
            max,
            step: None,
            thumbs,
            disabled: false,
            label: None,
            style,
            on_change: None,
            values,
            pos: [
                SpringValue::new(t0).with_spring(spring),
                SpringValue::new(t1).with_spring(spring),
            ],
            // Pembesaran thumb tidak membawa informasi apa pun yang tidak
            // sudah diceritakan warnanya: di bawah reduced-motion ia hilang
            // sepenuhnya, bukan sekadar kehilangan pantulan.
            lift: [
                SpringValue::new(0.0).with_spring(spring).decorative(),
                SpringValue::new(0.0).with_spring(spring).decorative(),
            ],
            hovered: false,
            hover_thumb: 0,
            dragging: None,
            grab: 0.0,
            focused: false,
            active: 0,
            direction: TextDirection::Ltr,
        }
    }

    /// Nilai thumb pertama — nilai slider biasa.
    pub fn value(&self) -> f32 {
        self.values[0]
    }

    /// Pasangan nilai (awal, akhir); slider satu thumb mengulang nilainya.
    pub fn values(&self) -> (f32, f32) {
        if self.thumbs > 1 {
            (self.values[0], self.values[1])
        } else {
            (self.values[0], self.values[0])
        }
    }

    /// Posisi thumb yang **sedang digambar** (0..1), hasil spring.
    pub fn positions(&self) -> [f32; MAX_THUMBS] {
        [self.pos[0].position(), self.pos[1].position()]
    }

    /// Thumb yang menerima keyboard.
    pub fn active_thumb(&self) -> usize {
        self.active
    }

    /// Sedang diseret jari/penunjuk.
    pub fn is_dragging(&self) -> bool {
        self.dragging.is_some()
    }

    /// Sedang memegang fokus keyboard.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Masih ada spring yang bergerak, jadi frame berikutnya dibutuhkan.
    pub fn is_animating(&self) -> bool {
        self.pos
            .iter()
            .chain(self.lift.iter())
            .any(|s| s.is_animating())
    }

    /// Selesaikan seluruh gerakan seketika (dipakai uji dan snapshot).
    pub fn settle(&mut self) {
        for s in self.pos.iter_mut().chain(self.lift.iter_mut()) {
            s.settle();
        }
    }

    /// Majukan seluruh spring node ini satu frame; benar bila ada yang pindah.
    ///
    /// Inilah pompa yang dipanggil [`crate::motion::advance`] — satu tempat
    /// untuk seluruh pohon, dengan alasan yang sama seperti komponen lain:
    /// "render hanya saat dirty" (§3.5) baru bisa dijanjikan kalau ada satu
    /// pihak yang tahu apakah masih ada yang bergerak.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let mut pindah = false;
        for s in self.pos.iter_mut().chain(self.lift.iter_mut()) {
            let sebelum = s.position();
            tick.advance(s);
            pindah |= s.position() != sebelum;
        }
        pindah
    }

    /// Ganti spring seluruh gerakan tanpa mengganggu yang sedang berjalan.
    pub fn set_spring(&mut self, spring: Spring) {
        for s in self.pos.iter_mut().chain(self.lift.iter_mut()) {
            s.set_spring(spring);
        }
    }

    /// Langkah satu tekan panah.
    ///
    /// Slider berundak melangkah satu `step`; slider kontinu melangkah 1% dari
    /// rentang, kebiasaan yang sama dengan AppKit dan ARIA.
    pub fn key_step(&self) -> f32 {
        match self.step {
            Some(s) if s > 0.0 => s,
            _ => ((self.max - self.min) / 100.0).abs(),
        }
    }

    /// Teks nilai yang dibacakan screen reader.
    pub fn value_text(&self) -> String {
        if self.thumbs > 1 {
            format!(
                "{} – {}",
                teks_angka(self.values[0]),
                teks_angka(self.values[1])
            )
        } else {
            teks_angka(self.values[0])
        }
    }

    /// Setel nilai sebuah thumb; benar bila nilainya benar-benar berubah.
    ///
    /// Nilainya dijepit ke rentang, dibulatkan ke `step`, **dan** dijaga agar
    /// dua thumb tidak pernah saling melewati (thumb bawah berhenti di thumb
    /// atas, bukan bertukar tempat — pertukaran diam-diam adalah cara tercepat
    /// membuat pengguna kehilangan jejak jarinya).
    pub fn set_thumb(&mut self, index: usize, value: f32) -> bool {
        let i = index.min(self.thumbs.saturating_sub(1));
        let mut v = snap(value, self.min, self.max, self.step);
        if self.thumbs > 1 {
            if i == 0 {
                v = v.min(self.values[1]);
            } else {
                v = v.max(self.values[0]);
            }
        }
        if self.values[i] == v {
            return false;
        }
        self.values[i] = v;
        let t = normalize(v, self.min, self.max);
        self.retarget(i, t);
        true
    }

    /// Setel kedua nilai sekaligus (jalur props).
    fn set_values(&mut self, start: f32, end: f32) -> bool {
        let mut a = snap(start, self.min, self.max, self.step);
        let mut b = snap(end, self.min, self.max, self.step);
        if self.thumbs > 1 && a > b {
            core::mem::swap(&mut a, &mut b);
        }
        let mut berubah = false;
        for (i, v) in [a, b].into_iter().enumerate() {
            if self.values[i] != v {
                self.values[i] = v;
                self.retarget(i, normalize(v, self.min, self.max));
                berubah = true;
            }
        }
        berubah
    }

    /// Arahkan spring posisi thumb `i` ke `t`.
    ///
    /// Selama jari masih menempel, thumb **melekat pada jari**: tidak ada
    /// pegas yang tertinggal di belakang kursor (kebiasaan AppKit/UIKit).
    /// Spring baru mengambil alih untuk perubahan yang bukan datang dari
    /// gerakan langsung: keyboard, klik di track, dan snap ke step saat
    /// dilepas.
    fn retarget(&mut self, i: usize, t: f32) {
        if self.dragging == Some(i) && self.step.is_none() {
            self.pos[i].jump_to(t);
        } else {
            self.pos[i].set_target(t);
        }
    }

    /// Jalankan `on_change` dengan nilai sekarang.
    ///
    /// Callback disalin keluar dulu: ia hampir selalu menulis signal, dan
    /// tulisan signal boleh memicu apa saja — yang tidak boleh adalah ia
    /// berjalan sambil node ini masih dipinjam `&mut` (pola yang sama dengan
    /// [`silka_core::tree::Interactive`]).
    fn beritahu(&self) {
        if let Some(cb) = self.on_change.clone() {
            let (a, b) = self.values();
            cb.call(a, b);
        }
    }

    /// Naikkan nilai thumb aktif sebanyak `steps` langkah; benar bila berubah.
    pub fn increment(&mut self, steps: f32) -> bool {
        let i = self.active.min(self.thumbs - 1);
        let v = self.values[i] + self.key_step() * steps;
        self.set_thumb(i, v)
    }

    /// Turunkan nilai thumb aktif sebanyak `steps` langkah.
    pub fn decrement(&mut self, steps: f32) -> bool {
        self.increment(-steps)
    }

    /// Terapkan permintaan teknologi bantu; benar bila nilainya berubah.
    ///
    /// Screen reader tidak menekan tombol panah — ia meminta aksi
    /// ([`AccessAction::Increment`], `Decrement`, `SetValue`). Tanpa jalur ini
    /// slider akan "terlihat" oleh VoiceOver tapi tidak bisa digerakkan
    /// olehnya, yaitu setengah aksesibilitas.
    pub fn apply_access_action(&mut self, action: AccessAction, value: Option<&str>) -> bool {
        if self.disabled {
            return false;
        }
        let berubah = match action {
            AccessAction::Increment => self.increment(1.0),
            AccessAction::Decrement => self.decrement(1.0),
            AccessAction::SetValue => match value.and_then(|v| v.trim().parse::<f32>().ok()) {
                Some(v) => {
                    let i = self.active.min(self.thumbs - 1);
                    self.set_thumb(i, v)
                }
                None => false,
            },
            _ => false,
        };
        if berubah {
            self.beritahu();
        }
        berubah
    }

    /// Sasaran "lift" tiap thumb untuk keadaan sekarang.
    fn lift_target(&self, i: usize) -> f32 {
        if self.disabled {
            return 0.0;
        }
        if self.dragging == Some(i) {
            1.0
        } else if self.hovered && self.hover_thumb == i {
            0.5
        } else {
            0.0
        }
    }

    fn perbarui_lift(&mut self) {
        for i in 0..MAX_THUMBS {
            let target = self.lift_target(i);
            self.lift[i].set_target(target);
        }
    }

    /// Diameter thumb `i` frame ini (thumb yang aktif sedikit membesar).
    fn thumb_diameter(&self, i: usize) -> f32 {
        self.style.thumb_size + self.style.thumb_grow * self.lift[i].position().clamp(0.0, 1.0)
    }

    /// Kotak thumb `i` dalam koordinat lokal.
    fn thumb_rect(&self, g: &SliderGeometry, i: usize) -> Rect {
        let d = self.thumb_diameter(i);
        let x = g.thumb_x(self.pos[i].position(), self.direction);
        Rect::new(x - d * 0.5, g.center_y - d * 0.5, d, d)
    }

    /// Thumb terdekat dari titik `x` (selalu 0 untuk slider satu thumb).
    fn thumb_terdekat(&self, g: &SliderGeometry, x: f32) -> usize {
        if self.thumbs < 2 {
            return 0;
        }
        let a = (g.thumb_x(self.pos[0].position(), self.direction) - x).abs();
        let b = (g.thumb_x(self.pos[1].position(), self.direction) - x).abs();
        if b < a {
            1
        } else {
            0
        }
    }

    /// Nilai yang diminta oleh sebuah titik x (sudah memperhitungkan grab).
    fn nilai_di(&self, g: &SliderGeometry, x: f32) -> f32 {
        let t = g.t_at(x - self.grab, self.direction);
        denormalize(t, self.min, self.max)
    }
}

impl RenderNode for Slider {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // Arah baca disimpan di sini karena event handler tidak punya akses ke
        // `LayoutCtx` — dan mirroring RTL bukan fitur susulan (§9.8).
        self.direction = ctx.direction();

        let lebar = if constraints.max_width.is_finite() {
            constraints.max_width
        } else {
            self.style.preferred_width
        };
        // Hit target ≥ 44pt walau tracknya setipis 4pt (HIG).
        let tinggi = self
            .style
            .min_height
            .max(self.style.thumb_size + self.style.thumb_grow + self.style.focus_ring_width * 2.0);
        constraints.constrain(Size::new(lebar, tinggi))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let g = SliderGeometry::new(ctx.size(), &self.style);
        let bulat = |rect: Rect| {
            Corners::uniform(rect.size.min_side() * 0.5, self.style.corner_style)
                .clamp_to(rect.size)
        };

        // 1. Track.
        if !g.track.size.is_empty() {
            ctx.quad(
                Quad::new(g.track)
                    .background(self.style.track)
                    .corners(bulat(g.track)),
            );
        }

        // 2. Bagian terisi — warnanya ikut naik bersama "lift" tertinggi,
        //    sehingga hover/tekan terasa di seluruh kontrol, bukan cuma thumb.
        let lift = self.lift[0]
            .position()
            .max(self.lift[1].position())
            .clamp(0.0, 1.0);
        let isi = if self.disabled {
            self.style.fill_disabled
        } else {
            self.style.fill.lerp(self.style.fill_hover, lift)
        };
        let (a, b) = if self.thumbs > 1 {
            (
                g.thumb_x(self.pos[0].position(), self.direction),
                g.thumb_x(self.pos[1].position(), self.direction),
            )
        } else {
            (
                g.anchor_x(self.direction),
                g.thumb_x(self.pos[0].position(), self.direction),
            )
        };
        let (kiri, kanan) = if a <= b { (a, b) } else { (b, a) };
        let terisi = Rect::new(kiri, g.track.min_y(), kanan - kiri, g.track.size.height);
        if !terisi.size.is_empty() {
            ctx.quad(Quad::new(terisi).background(isi).corners(bulat(g.track)));
        }

        // 3. Thumb, lengkap dengan cincin fokus di yang sedang aktif.
        for i in 0..self.thumbs.min(MAX_THUMBS) {
            let rect = self.thumb_rect(&g, i);
            if rect.size.is_empty() {
                continue;
            }
            let corners = bulat(rect);
            if self.focused && !self.disabled && self.active == i {
                let ring = self.style.focus_ring_width;
                if ring > 0.0 && self.style.focus_ring.a > 0.0 {
                    let luar = rect.deflate(silka_paint::Insets::all(-ring));
                    ctx.quad(
                        Quad::new(luar)
                            .corners(Corners::new(
                                CornerRadii::all(corners.radii.max() + ring),
                                self.style.corner_style,
                            ))
                            .border(ring, self.style.focus_ring),
                    );
                }
            }
            let quad = Quad::new(rect)
                .background(self.style.thumb)
                .corners(corners)
                .border(self.style.thumb_border_width, self.style.thumb_border);
            if self.disabled {
                ctx.quad(quad);
            } else {
                ctx.shadowed(quad, self.style.shadow);
            }
        }

        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Slider;
        node.label.clone_from(&self.label);
        node.value = Some(self.value_text());
        node.disabled = self.disabled;
        if !self.disabled {
            node.actions |= AccessActions::FOCUS
                | AccessActions::INCREMENT
                | AccessActions::DECREMENT
                | AccessActions::SET_VALUE;
        }
    }

    /// Seluruh kotak node — termasuk pita 44pt di atas dan di bawah track.
    fn hit_shape(&self) -> HitShape {
        HitShape::Rect
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Slider yang dimatikan tetap menyerap: klik padanya tidak boleh
        // menembus ke konten di belakangnya.
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
        if self.disabled {
            return None;
        }
        Some(if self.dragging.is_some() {
            CursorIcon::Grabbing
        } else {
            CursorIcon::Grab
        })
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if self.disabled {
            if matches!(event, Event::Pointer(p) if matches!(p.phase, PointerPhase::Down | PointerPhase::Up))
            {
                ctx.handled();
            }
            return;
        }

        let g = SliderGeometry::new(ctx.size(), &self.style);
        match event {
            Event::Pointer(p) => match p.phase {
                PointerPhase::Enter | PointerPhase::Move => {
                    let x = ctx.local().x;
                    if p.phase == PointerPhase::Enter {
                        self.hovered = true;
                    }
                    match self.dragging {
                        Some(i) => {
                            let v = self.nilai_di(&g, x);
                            if self.set_thumb(i, v) {
                                self.beritahu();
                            }
                            ctx.handled();
                        }
                        None => {
                            self.hovered = true;
                            self.hover_thumb = self.thumb_terdekat(&g, x);
                        }
                    }
                    self.perbarui_lift();
                    ctx.request_paint();
                    if self.is_animating() {
                        ctx.request_animation();
                    }
                }
                PointerPhase::Leave => {
                    self.hovered = false;
                    self.perbarui_lift();
                    ctx.request_paint();
                    if self.is_animating() {
                        ctx.request_animation();
                    }
                }
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    let x = ctx.local().x;
                    let i = self.thumb_terdekat(&g, x);
                    self.active = i;
                    self.hover_thumb = i;
                    self.hovered = true;
                    // Tekan **di atas thumb** berarti menggenggamnya: nilainya
                    // tidak melompat, jari hanya menyeret dari titik itu. Tekan
                    // di track berarti "bawa thumb ke sini".
                    let thumb = self.thumb_rect(&g, i);
                    self.grab = if thumb.contains(ctx.local()) {
                        x - thumb.center().x
                    } else {
                        0.0
                    };
                    self.dragging = Some(i);
                    let v = self.nilai_di(&g, x);
                    if self.set_thumb(i, v) {
                        self.beritahu();
                    }
                    self.perbarui_lift();
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.request_paint();
                    ctx.request_animation();
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    if let Some(i) = self.dragging.take() {
                        let v = self.nilai_di(&g, ctx.local().x);
                        let berubah = self.set_thumb(i, v);
                        // Lepas = jari tidak lagi memegang: posisi thumb
                        // menyusul nilainya lewat spring (snap ke step).
                        self.retarget(i, normalize(self.values[i], self.min, self.max));
                        if berubah {
                            self.beritahu();
                        }
                    }
                    self.grab = 0.0;
                    self.perbarui_lift();
                    ctx.release_pointer();
                    ctx.request_paint();
                    ctx.request_animation();
                    ctx.handled();
                }
                PointerPhase::Cancel => {
                    if let Some(i) = self.dragging.take() {
                        self.retarget(i, normalize(self.values[i], self.min, self.max));
                    }
                    self.grab = 0.0;
                    self.perbarui_lift();
                    ctx.request_paint();
                }
                _ => {}
            },

            Event::Key(k) if k.is_pressed() && k.modifiers.is_empty() => {
                let langkah = match &k.code {
                    KeyCode::Named(NamedKey::ArrowUp) => Some(1.0),
                    KeyCode::Named(NamedKey::ArrowDown) => Some(-1.0),
                    // Panah mendatar ikut membalik pada arah kanan-ke-kiri:
                    // "kanan" selalu berarti "ke arah nilai yang lebih besar
                    // menurut mata pengguna" (§9.8).
                    KeyCode::Named(NamedKey::ArrowRight) => {
                        Some(if self.direction.is_rtl() { -1.0 } else { 1.0 })
                    }
                    KeyCode::Named(NamedKey::ArrowLeft) => {
                        Some(if self.direction.is_rtl() { 1.0 } else { -1.0 })
                    }
                    KeyCode::Named(NamedKey::PageUp) => Some(PAGE_STEPS),
                    KeyCode::Named(NamedKey::PageDown) => Some(-PAGE_STEPS),
                    _ => None,
                };
                let batas = match &k.code {
                    KeyCode::Named(NamedKey::Home) => Some(self.min),
                    KeyCode::Named(NamedKey::End) => Some(self.max),
                    _ => None,
                };

                let berubah = if let Some(n) = langkah {
                    self.increment(n)
                } else if let Some(v) = batas {
                    let i = self.active.min(self.thumbs - 1);
                    self.set_thumb(i, v)
                } else {
                    return;
                };
                if berubah {
                    self.beritahu();
                }
                ctx.request_paint();
                ctx.request_animation();
                ctx.handled();
            }

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.dragging = None;
                    self.perbarui_lift();
                }
                ctx.request_paint();
            }

            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props sebuah slider — bentuk view dari [`Slider`].
#[derive(Debug, Clone, PartialEq)]
pub struct SliderProps {
    min: f32,
    max: f32,
    values: [f32; MAX_THUMBS],
    thumbs: usize,
    step: Option<f32>,
    disabled: bool,
    label: Option<String>,
    style: SliderStyle,
    on_change: Option<ChangeCallback>,
    spring: Spring,
}

impl SliderProps {
    fn node(&self) -> Slider {
        let mut n = Slider::baru(
            self.min,
            self.max,
            self.values,
            self.thumbs,
            self.style,
            self.spring,
        );
        n.step = self.step;
        n.disabled = self.disabled;
        n.label.clone_from(&self.label);
        n.on_change.clone_from(&self.on_change);
        // Nilai awal ikut aturan snap yang sama dengan nilai dari pengguna.
        n.set_values(self.values[0], self.values[1]);
        n.settle();
        n
    }
}

impl ViewNode for SliderProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(self.node())
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<Slider>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.min != self.min || n.max != self.max || n.step != self.step || n.thumbs != self.thumbs
        {
            n.min = self.min;
            n.max = self.max;
            n.step = self.step;
            n.thumbs = self.thumbs.clamp(1, MAX_THUMBS);
            n.active = n.active.min(n.thumbs - 1);
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        // Nilai yang datang dari aplikasi adalah kebenaran terakhir — tapi
        // **bukan** selagi jari masih menempel: aplikasi yang lambat satu frame
        // tidak boleh menarik thumb kembali ke belakang kursor.
        if n.dragging.is_none() && n.set_values(self.values[0], self.values[1]) {
            // Ukuran slider tidak pernah bergantung pada nilainya: yang berubah
            // hanya piksel. Menggeser nilai karena itu **tidak** boleh membuat
            // halaman di-layout ulang.
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                n.dragging = None;
                n.hovered = false;
                n.perbarui_lift();
            }
            dirty |= Dirty::PAINT;
        }
        if n.pos[0].spring() != self.spring {
            n.set_spring(self.spring);
        }
        // Callback selalu diganti tanpa dibandingkan: closure dibangun ulang
        // tiap rebuild dan menangkap nilai baru (pola `InteractiveProps`).
        n.on_change.clone_from(&self.on_change);
        dirty
    }
}

fn props(theme: &Theme, values: [f32; MAX_THUMBS], thumbs: usize) -> SliderProps {
    SliderProps {
        min: 0.0,
        max: 1.0,
        values,
        thumbs,
        step: None,
        disabled: false,
        label: None,
        style: SliderStyle::from_theme(theme),
        on_change: None,
        // `smooth` adalah kurva bawaan framework; slider memakai `snappy`
        // karena gerakannya pendek dan harus terasa langsung mengikuti niat
        // pengguna (WWDC23: durasi perceptual, bukan mass/stiffness).
        spring: Spring::snappy(),
    }
}

/// Slider satu thumb — konstruktor gaya Dart (§2.5).
///
/// Rentang bawaannya `0.0..=1.0` seperti SwiftUI; ganti dengan
/// [`SliderBuilder::range`].
pub fn slider(theme: &Theme, value: f32) -> SliderBuilder {
    SliderBuilder {
        key: None,
        props: props(theme, [value, value], 1),
    }
}

/// Slider dua thumb (varian range, `KOMPONEN.md`).
pub fn range_slider(theme: &Theme, start: f32, end: f32) -> SliderBuilder {
    SliderBuilder {
        key: None,
        props: props(theme, [start, end], 2),
    }
}

/// Builder sebuah slider: seluruh sifat opsional pindah ke method chain (§2.5).
pub struct SliderBuilder {
    key: Option<Key>,
    props: SliderProps,
}

impl SliderBuilder {
    /// Kunci identitas — wajib untuk slider di dalam daftar dinamis (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Rentang nilai (`0.0..=100.0`).
    pub fn range(mut self, range: RangeInclusive<f32>) -> Self {
        let (a, b) = (*range.start(), *range.end());
        self.props.min = a.min(b);
        self.props.max = a.max(b);
        self
    }

    /// Kelipatan yang boleh ditempati nilai — "snap ke step" (`KOMPONEN.md`).
    pub fn step(mut self, step: f32) -> Self {
        self.props.step = (step > 0.0).then_some(step);
        self
    }

    /// Tanpa undakan: nilainya boleh berapa saja di dalam rentang.
    pub fn continuous(mut self) -> Self {
        self.props.step = None;
        self
    }

    /// Nama yang dibacakan screen reader (§3.8).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.props.label = Some(label.into());
        self
    }

    /// Matikan kontrol (tetap dibacakan sebagai dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.props.disabled = disabled;
        self
    }

    /// Apa yang dijalankan saat pengguna menggeser — nilai thumb pertama.
    pub fn on_change(mut self, f: impl Fn(f32) + 'static) -> Self {
        self.props.on_change = Some(ChangeCallback::new(move |a, _| f(a)));
        self
    }

    /// Versi range: menerima pasangan (awal, akhir).
    pub fn on_range_change(mut self, f: impl Fn(f32, f32) + 'static) -> Self {
        self.props.on_change = Some(ChangeCallback::new(f));
        self
    }

    /// Spring yang menjalankan gerakannya (`smooth`/`snappy`/`bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.props.spring = spring;
        self
    }

    /// Ganti seluruh nilai tampilan sekaligus — escape hatch untuk komponen
    /// turunan (misalnya slider di dalam toolbar yang lebih rapat).
    pub fn style(mut self, style: SliderStyle) -> Self {
        self.props.style = style;
        self
    }
}

impl From<SliderBuilder> for View {
    fn from(b: SliderBuilder) -> View {
        let mut builder = Builder::new(b.props);
        if let Some(key) = b.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for SliderBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SliderBuilder")
            .field("key", &self.key)
            .field("props", &self.props)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Pompa animasi
// ---------------------------------------------------------------------------

/// Semua node [`Slider`] di dalam `tree`, urut dari akar.
///
/// Dipakai [`crate::motion`] (pompa animasi crate ini) dan uji; aplikasi tidak
/// perlu memanggilnya sendiri.
pub fn sliders(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    if tree.node_ref::<Slider>(id).is_some() {
        out.push(id);
    }
    for anak in tree.children(id) {
        kumpulkan(tree, *anak, out);
    }
}

/// Salurkan permintaan teknologi bantu ke slider sasarannya.
///
/// Benar bila permintaannya benar-benar mengubah nilai. Shell memanggilnya dari
/// `on_access_action`; validasi "node masih ada dan aksinya memang diumumkan"
/// sudah dilakukan adapter platform sebelum sampai ke sini.
///
/// ```
/// # use silka_core::access::{AccessAction, AccessActionRequest};
/// # use silka_core::tree::{BoxConstraints, RenderTree};
/// # use silka_core::view::reconcile;
/// # use silka_paint::Size;
/// # use silka_theme::{Appearance, Theme};
/// use silka_widgets::slider::{apply_access_action, slider, sliders};
///
/// let t = Theme::cupertino(Appearance::Dark);
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, slider(&t, 50.0).range(0.0..=100.0).step(5.0));
/// tree.layout(BoxConstraints::tight(Size::new(320.0, 44.0)));
///
/// let target = sliders(&tree)[0];
/// assert!(apply_access_action(
///     &mut tree,
///     &AccessActionRequest { target, action: AccessAction::Increment, value: None },
/// ));
/// ```
pub fn apply_access_action(tree: &mut RenderTree, request: &AccessActionRequest) -> bool {
    let Some(s) = tree.node_mut_ref::<Slider>(request.target) else {
        return false;
    };
    let berubah = s.apply_access_action(request.action, request.value.as_deref());
    if berubah {
        // Nilai baru = piksel baru, bukan tata letak baru: ukuran slider tidak
        // pernah bergantung pada nilainya.
        tree.mark_needs_paint(request.target);
    }
    berubah
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{advance, is_animating, settle};
    use silka_core::animation::Motion;
    use silka_core::input::{Event, InputRouter, KeyEvent, Modifiers, PointerEvent, PointerPhase};
    use silka_core::view::{reconcile, View};
    use silka_paint::{Command, Point, Scene};
    use silka_theme::{Appearance, Preset};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    const RUANG: Size = Size::new(320.0, 60.0);

    fn tema() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    /// Pohon uji: constraints **longgar**, seperti slider di dalam kolom form
    /// sungguhan. Constraints tight menjadikan node relayout boundary, dan
    /// boundary menyimpan cache gambar — sesuatu yang tidak pernah terjadi pada
    /// slider di dalam layout nyata dan hanya akan mengaburkan apa yang diuji.
    fn pohon(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(RUANG));
        tree
    }

    fn node(tree: &RenderTree) -> &Slider {
        let id = sliders(tree)[0];
        tree.node_ref::<Slider>(id).expect("node slider")
    }

    fn geometri(tree: &RenderTree) -> SliderGeometry {
        let id = sliders(tree)[0];
        SliderGeometry::new(tree.size(id), &tree.node_ref::<Slider>(id).unwrap().style)
    }

    fn titik(tree: &RenderTree, x: f32) -> Point {
        let id = sliders(tree)[0];
        let asal = tree.global_offset(id);
        Point::new(asal.x + x, asal.y + tree.size(id).height * 0.5)
    }

    /// Satu drag penuh: tekan di `dari`, geser ke `ke`, lepas.
    fn seret(tree: &mut RenderTree, router: &mut InputRouter, dari: Point, ke: Point) {
        for (fase, p, ms) in [
            (PointerPhase::Move, dari, 0),
            (PointerPhase::Down, dari, 8),
            (PointerPhase::Move, ke, 24),
            (PointerPhase::Up, ke, 40),
        ] {
            let mut e = PointerEvent::new(fase, p, Duration::from_millis(ms));
            if matches!(fase, PointerPhase::Down | PointerPhase::Up) {
                e = e.button(PointerButton::Primary);
            }
            router.dispatch(tree, &Event::Pointer(e));
        }
    }

    fn tekan_tombol(tree: &mut RenderTree, router: &mut InputRouter, key: NamedKey) {
        router.dispatch(
            tree,
            &Event::Key(KeyEvent::pressed(KeyCode::Named(key), Duration::ZERO)),
        );
    }

    // -- logika murni --------------------------------------------------------

    #[test]
    fn snap_membulatkan_ke_kelipatan_step_dan_menjepit_rentang() {
        assert_eq!(snap(37.0, 0.0, 100.0, Some(5.0)), 35.0);
        assert_eq!(snap(38.0, 0.0, 100.0, Some(5.0)), 40.0);
        assert_eq!(snap(-10.0, 0.0, 100.0, Some(5.0)), 0.0);
        assert_eq!(snap(1000.0, 0.0, 100.0, Some(5.0)), 100.0);
        // Kelipatan dihitung dari `min`, bukan dari nol.
        assert_eq!(snap(13.0, 10.0, 20.0, Some(4.0)), 14.0);
        // Kontinu: nilainya lewat apa adanya (hanya dijepit).
        assert_eq!(snap(37.3, 0.0, 100.0, None), 37.3);
        // Nilai gila tidak pernah menjalar ke layout.
        assert_eq!(snap(f32::NAN, 0.0, 100.0, None), 0.0);
    }

    #[test]
    fn normalisasi_bolak_balik_konsisten() {
        for v in [0.0f32, 25.0, 50.0, 99.9, 100.0] {
            let t = normalize(v, 0.0, 100.0);
            assert!((denormalize(t, 0.0, 100.0) - v).abs() < 1e-3, "{v}");
        }
        // Rentang degenerate tidak boleh menghasilkan NaN.
        assert_eq!(normalize(5.0, 5.0, 5.0), 0.0);
    }

    #[test]
    fn geometri_menempatkan_thumb_di_dalam_kotak_node() {
        let style = SliderStyle::from_theme(&tema());
        let g = SliderGeometry::new(RUANG, &style);
        let kiri = g.thumb_x(0.0, TextDirection::Ltr);
        let kanan = g.thumb_x(1.0, TextDirection::Ltr);
        let jari = (style.thumb_size + style.thumb_grow) * 0.5;
        assert!(kiri - jari >= -1e-3, "thumb keluar kiri: {kiri}");
        assert!(kanan + jari <= RUANG.width + 1e-3, "thumb keluar kanan");
        // Track selalu di tengah secara vertikal.
        assert!((g.track.center().y - RUANG.height * 0.5).abs() < 1e-3);
        // Bolak-balik posisi ↔ nilai.
        let x = g.thumb_x(0.4, TextDirection::Ltr);
        assert!((g.t_at(x, TextDirection::Ltr) - 0.4).abs() < 1e-3);
    }

    #[test]
    fn geometri_membalik_arah_pada_rtl() {
        let style = SliderStyle::from_theme(&tema());
        let g = SliderGeometry::new(RUANG, &style);
        assert!(g.thumb_x(1.0, TextDirection::Rtl) < g.thumb_x(0.0, TextDirection::Rtl));
        let x = g.thumb_x(0.25, TextDirection::Rtl);
        assert!((g.t_at(x, TextDirection::Rtl) - 0.25).abs() < 1e-3);
    }

    // -- Definition of Done --------------------------------------------------

    #[test]
    fn hit_target_minimal_44pt_walau_tracknya_setipis_4pt() {
        let t = tema();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, slider(&t, 0.5));
        // Constraints longgar: node yang memilih tingginya sendiri.
        tree.layout(BoxConstraints::loose(Size::new(320.0, 400.0)));
        let id = sliders(&tree)[0];
        let ukuran = tree.size(id);
        assert!(
            ukuran.height >= MIN_HIT_TARGET,
            "hit target cuma {ukuran:?} (HIG minta {MIN_HIT_TARGET}pt)"
        );
        assert!(tree.node_ref::<Slider>(id).unwrap().style.track_height < 8.0);
    }

    #[test]
    fn node_a11y_slider_membawa_nilai_dan_aksi() {
        let t = tema();
        let tree = pohon(slider(&t, 42.0).range(0.0..=100.0).label("Volume"));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Volume")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Slider);
        assert_eq!(e.node.value.as_deref(), Some("42"));
        assert!(e.node.actions.contains(AccessActions::FOCUS));
        assert!(e.node.actions.contains(AccessActions::INCREMENT));
        assert!(e.node.actions.contains(AccessActions::DECREMENT));
        assert!(e.node.actions.contains(AccessActions::SET_VALUE));
        assert!(!e.node.disabled);
    }

    #[test]
    fn slider_dimatikan_dibacakan_dimmed_dan_tidak_bisa_difokuskan() {
        let t = tema();
        let tree = pohon(slider(&t, 0.5).label("Mati").disabled(true));
        let a11y = tree.access_tree(None);
        let e = a11y.find_label("Mati").unwrap();
        assert!(e.node.disabled);
        assert!(e.node.actions.is_empty());
        let id = sliders(&tree)[0];
        assert!(
            !tree
                .node_ref::<Slider>(id)
                .unwrap()
                .focus_policy()
                .focusable
        );
    }

    #[test]
    fn warna_dan_bentuk_selalu_datang_dari_token_di_kedua_preset() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut tree = pohon(slider(&t, 0.5));
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
                assert_eq!(kotak.len(), 3, "track + isian + thumb ({preset:?})");
                assert_eq!(kotak[0].background, t.color.surface_sunken);
                assert_eq!(kotak[1].background, t.color.accent);
                assert_eq!(kotak[2].background, t.color.surface_elevated);
                assert_eq!(kotak[2].border_color, t.color.separator);
                // Geometri sudut adalah parameter, bukan konstanta (§2.7).
                for q in &kotak {
                    assert_eq!(q.corners.style, t.radius.style);
                }
                // Thumb selalu berbayang ganda ala HIG.
                let bayangan = scene
                    .commands()
                    .iter()
                    .filter(|c| matches!(c, Command::Shadow(_)))
                    .count();
                assert_eq!(bayangan, 2, "ambient + key");
            }
        }
    }

    #[test]
    fn nilai_menentukan_lebar_isian() {
        let t = tema();
        let lebar = |v: f32| {
            let mut tree = pohon(slider(&t, v).range(0.0..=100.0));
            let mut scene = Scene::new(t.color.background);
            tree.paint_into(&mut scene);
            scene
                .commands()
                .iter()
                .filter_map(|c| match c {
                    Command::Quad(q) if q.background == t.color.accent => Some(q.rect.size.width),
                    _ => None,
                })
                .next()
                .unwrap_or(0.0)
        };
        let (a, b, c) = (lebar(0.0), lebar(50.0), lebar(100.0));
        assert!(a < b && b < c, "{a} {b} {c}");
        assert!(c > RUANG.width * 0.8, "isian penuh nyaris selebar track");
    }

    // -- interaksi -----------------------------------------------------------

    #[test]
    fn klik_di_track_memindahkan_thumb_ke_titik_itu_dan_memanggil_on_change() {
        let t = tema();
        let catat = Rc::new(RefCell::new(Vec::<f32>::new()));
        let tulis = catat.clone();
        let mut tree = pohon(
            slider(&t, 0.0)
                .range(0.0..=100.0)
                .on_change(move |v| tulis.borrow_mut().push(v)),
        );
        let mut router = InputRouter::new();

        let g = geometri(&tree);
        let tengah = titik(&tree, g.thumb_x(0.5, TextDirection::Ltr));
        seret(&mut tree, &mut router, tengah, tengah);

        let v = node(&tree).value();
        assert!((v - 50.0).abs() < 2.0, "klik di tengah → {v}");
        assert!(
            !catat.borrow().is_empty(),
            "on_change tidak pernah dipanggil"
        );
    }

    #[test]
    fn drag_mengikuti_jari_dan_berhenti_di_tepi() {
        let t = tema();
        let mut tree = pohon(slider(&t, 50.0).range(0.0..=100.0));
        let mut router = InputRouter::new();
        let g = geometri(&tree);

        let dari = titik(&tree, g.thumb_x(0.5, TextDirection::Ltr));
        let ke = titik(&tree, g.thumb_x(0.9, TextDirection::Ltr));
        seret(&mut tree, &mut router, dari, ke);
        let v = node(&tree).value();
        assert!((v - 90.0).abs() < 2.0, "drag ke 90% → {v}");

        // Jauh di luar kotak node: nilainya berhenti di batas, tidak meledak.
        let jauh = Point::new(titik(&tree, 0.0).x - 500.0, titik(&tree, 0.0).y);
        seret(&mut tree, &mut router, ke, jauh);
        assert_eq!(node(&tree).value(), 0.0);
        assert!(!node(&tree).is_dragging());
    }

    #[test]
    fn menggenggam_thumb_tidak_membuat_nilainya_melompat() {
        let t = tema();
        let mut tree = pohon(slider(&t, 50.0).range(0.0..=100.0));
        let mut router = InputRouter::new();
        let g = geometri(&tree);

        // Tekan sedikit di tepi thumb — bukan di pusatnya.
        let x = g.thumb_x(0.5, TextDirection::Ltr) + 6.0;
        let p = titik(&tree, x);
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Down, p, Duration::ZERO)
                    .button(PointerButton::Primary),
            ),
        );
        assert_eq!(node(&tree).value(), 50.0, "genggaman tidak boleh melompat");
    }

    #[test]
    fn keyboard_menggeser_nilai_dan_menghormati_step() {
        let t = tema();
        let mut tree = pohon(slider(&t, 50.0).range(0.0..=100.0).step(5.0));
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));

        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowRight);
        assert_eq!(node(&tree).value(), 55.0);
        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowLeft);
        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowDown);
        assert_eq!(node(&tree).value(), 45.0);
        tekan_tombol(&mut tree, &mut router, NamedKey::PageUp);
        assert_eq!(node(&tree).value(), 95.0);
        tekan_tombol(&mut tree, &mut router, NamedKey::Home);
        assert_eq!(node(&tree).value(), 0.0);
        tekan_tombol(&mut tree, &mut router, NamedKey::End);
        assert_eq!(node(&tree).value(), 100.0);
        // Sudah di batas: tidak melewatinya.
        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowUp);
        assert_eq!(node(&tree).value(), 100.0);
    }

    #[test]
    fn keyboard_kontinu_melangkah_satu_persen_rentang() {
        let t = tema();
        let mut tree = pohon(slider(&t, 0.0).range(0.0..=200.0));
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowRight);
        assert!((node(&tree).value() - 2.0).abs() < 1e-3);
    }

    #[test]
    fn panah_mendatar_terbalik_pada_arah_kanan_ke_kiri() {
        let t = tema();
        let mut tree = RenderTree::new();
        tree.set_direction(TextDirection::Rtl);
        reconcile(&mut tree, slider(&t, 50.0).range(0.0..=100.0).step(10.0));
        tree.layout(BoxConstraints::loose(RUANG));
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));

        // Di RTL, "kanan" secara visual berarti nilai lebih kecil.
        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowRight);
        assert_eq!(node(&tree).value(), 40.0);
        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowLeft);
        assert_eq!(node(&tree).value(), 50.0);
        // Atas/bawah tidak pernah ikut membalik.
        tekan_tombol(&mut tree, &mut router, NamedKey::ArrowUp);
        assert_eq!(node(&tree).value(), 60.0);
    }

    #[test]
    fn modifier_dibiarkan_lewat_agar_pintasan_aplikasi_tidak_ditelan() {
        let t = tema();
        let mut tree = pohon(slider(&t, 50.0).range(0.0..=100.0).step(5.0));
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        router.dispatch(
            &mut tree,
            &Event::Key(
                KeyEvent::pressed(KeyCode::Named(NamedKey::ArrowRight), Duration::ZERO)
                    .modifiers(Modifiers::COMMAND),
            ),
        );
        assert_eq!(node(&tree).value(), 50.0);
    }

    #[test]
    fn slider_mati_tidak_bergerak_oleh_apa_pun() {
        let t = tema();
        let mut tree = pohon(slider(&t, 50.0).range(0.0..=100.0).disabled(true));
        let mut router = InputRouter::new();
        let g = geometri(&tree);
        let p = titik(&tree, g.thumb_x(0.9, TextDirection::Ltr));
        seret(&mut tree, &mut router, p, p);
        assert_eq!(node(&tree).value(), 50.0);
    }

    #[test]
    fn fokus_menggambar_cincin_di_thumb_aktif() {
        let t = tema();
        let mut tree = pohon(slider(&t, 0.5).label("Fokus"));
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];

        let cincin = |tree: &mut RenderTree| {
            let mut scene = Scene::new(t.color.background);
            tree.paint_into(&mut scene);
            scene
                .commands()
                .iter()
                .filter(|c| matches!(c, Command::Quad(q) if q.border_color == t.color.focus_ring))
                .count()
        };
        assert_eq!(cincin(&mut tree), 0);
        router.focus_node(&mut tree, Some(id));
        assert!(node(&tree).is_focused());
        assert_eq!(cincin(&mut tree), 1, "cincin fokus tidak digambar");
        router.focus_node(&mut tree, None);
        assert_eq!(cincin(&mut tree), 0);
    }

    // -- range ---------------------------------------------------------------

    #[test]
    fn range_dua_thumb_tidak_pernah_saling_melewati() {
        let t = tema();
        let catat = Rc::new(RefCell::new((0.0f32, 0.0f32)));
        let tulis = catat.clone();
        let mut tree = pohon(
            range_slider(&t, 20.0, 80.0)
                .range(0.0..=100.0)
                .on_range_change(move |a, b| *tulis.borrow_mut() = (a, b)),
        );
        let mut router = InputRouter::new();
        let g = geometri(&tree);

        // Seret thumb bawah jauh melewati thumb atas.
        let dari = titik(&tree, g.thumb_x(0.2, TextDirection::Ltr));
        let ke = titik(&tree, g.thumb_x(0.95, TextDirection::Ltr));
        seret(&mut tree, &mut router, dari, ke);
        let (a, b) = node(&tree).values();
        assert!(a <= b, "thumb bertukar tempat: {a} > {b}");
        assert_eq!(b, 80.0, "thumb atas tidak boleh ikut terdorong");
        assert_eq!(*catat.borrow(), (a, b));
    }

    #[test]
    fn range_memilih_thumb_terdekat_dari_titik_tekan() {
        let t = tema();
        let mut tree = pohon(range_slider(&t, 20.0, 80.0).range(0.0..=100.0));
        let mut router = InputRouter::new();
        let g = geometri(&tree);

        let dekat_atas = titik(&tree, g.thumb_x(0.75, TextDirection::Ltr));
        seret(&mut tree, &mut router, dekat_atas, dekat_atas);
        let (a, b) = node(&tree).values();
        assert_eq!(a, 20.0, "thumb bawah tidak boleh ikut pindah");
        assert!((b - 75.0).abs() < 2.0, "thumb atas → {b}");
        assert_eq!(node(&tree).active_thumb(), 1);
    }

    #[test]
    fn nilai_range_dibacakan_sebagai_dua_angka() {
        let t = tema();
        let tree = pohon(
            range_slider(&t, 20.0, 80.0)
                .range(0.0..=100.0)
                .label("Harga"),
        );
        let a11y = tree.access_tree(None);
        let e = a11y.find_label("Harga").unwrap();
        assert_eq!(e.node.value.as_deref(), Some("20 – 80"));
    }

    // -- animasi -------------------------------------------------------------

    #[test]
    fn spring_bergerak_saat_dipompa_lalu_berhenti_sendiri() {
        let t = tema();
        let mut tree = pohon(slider(&t, 0.0).range(0.0..=100.0).step(10.0));
        let tick = |ms: u64| Tick::manual(Duration::from_millis(ms), Motion::Full);

        // Frame pertama: pompa terpasang, belum ada yang bergerak.
        assert_eq!(advance(&mut tree, &tick(16)), Dirty::NONE);

        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        tekan_tombol(&mut tree, &mut router, NamedKey::End);

        // Nilainya langsung benar, tapi thumb-nya masih di jalan.
        assert_eq!(node(&tree).value(), 100.0);
        assert!(
            node(&tree).positions()[0] < 1.0,
            "thumb melompat, bukan spring"
        );
        assert!(is_animating(&tree));

        let mut frame = 0;
        while is_animating(&tree) && frame < 600 {
            let dirty = advance(&mut tree, &tick(8));
            assert!(dirty.contains(Dirty::ANIMATION) || !is_animating(&tree));
            frame += 1;
        }
        assert!(frame > 1, "gerakan selesai dalam satu frame — itu lompatan");
        assert!(frame < 600, "spring tidak pernah settle");
        assert_eq!(node(&tree).positions()[0], 1.0);
        // Sudah diam: tidak ada frame berikutnya yang diminta (§3.5).
        assert_eq!(advance(&mut tree, &tick(8)), Dirty::NONE);
    }

    #[test]
    fn nilai_tidak_pernah_menunggu_animasi() {
        // Yang dibacakan screen reader dan yang dikirim ke aplikasi adalah
        // nilainya, bukan posisi thumb: keduanya tidak boleh satu frame
        // ketinggalan hanya karena ada spring yang masih berjalan.
        let t = tema();
        let mut tree = pohon(slider(&t, 0.0).range(0.0..=100.0));
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        tekan_tombol(&mut tree, &mut router, NamedKey::End);

        assert_eq!(node(&tree).value(), 100.0);
        let a11y = tree.access_tree(None);
        assert_eq!(
            a11y.entries()
                .iter()
                .find(|e| e.node.role == AccessRole::Slider)
                .and_then(|e| e.node.value.clone())
                .as_deref(),
            Some("100")
        );
        // Pohon yang sengaja tidak dipompa tinggal di-settle untuk snapshot.
        settle(&mut tree);
        assert_eq!(node(&tree).positions()[0], 1.0);
    }

    #[test]
    fn reduced_motion_membuang_pembesaran_thumb_tapi_tetap_menggerakkan_nilai() {
        let t = tema();
        let mut tree = pohon(slider(&t, 0.0).range(0.0..=100.0));
        let tick_penuh = Tick::manual(Duration::from_millis(16), Motion::Full);
        advance(&mut tree, &tick_penuh);

        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        let g = geometri(&tree);
        let p = titik(&tree, g.thumb_x(0.5, TextDirection::Ltr));
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Down, p, Duration::ZERO)
                    .button(PointerButton::Primary),
            ),
        );

        // Reduced-motion: "lift" dekoratif hilang seketika…
        let tick_kurang = Tick::manual(Duration::from_millis(16), Motion::Reduced);
        advance(&mut tree, &tick_kurang);
        let n = node(&tree);
        assert!(!n.lift[0].is_animating(), "gerakan dekoratif harus mati");
        assert_eq!(n.lift[0].position(), 1.0, "keadaan tetap terbaca");

        // …tapi gerakan yang menjelaskan nilai tetap berjalan (tanpa pantulan).
        let mut tree = pohon(slider(&t, 0.0).range(0.0..=100.0));
        advance(&mut tree, &tick_penuh);
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        tekan_tombol(&mut tree, &mut router, NamedKey::End);
        advance(&mut tree, &tick_kurang);
        let posisi = node(&tree).positions()[0];
        assert!(posisi > 0.0 && posisi < 1.0, "nilai ikut hilang: {posisi}");
    }

    #[test]
    fn spring_bisa_di_retarget_di_tengah_gerakan() {
        let t = tema();
        let mut tree = pohon(slider(&t, 0.0).range(0.0..=100.0));
        let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
        advance(&mut tree, &tick);
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));

        tekan_tombol(&mut tree, &mut router, NamedKey::End);
        advance(&mut tree, &tick);
        advance(&mut tree, &tick);
        let tengah = node(&tree).positions()[0];
        assert!(tengah > 0.0 && tengah < 1.0);

        // Berbalik arah di tengah jalan: tidak ada lompatan ke nol.
        tekan_tombol(&mut tree, &mut router, NamedKey::Home);
        assert_eq!(node(&tree).value(), 0.0);
        let sesudah = node(&tree).positions()[0];
        assert!(
            (sesudah - tengah).abs() < 1e-6,
            "retarget membuang posisi: {tengah} → {sesudah}"
        );
        assert!(is_animating(&tree));
    }

    #[test]
    fn settle_menyelesaikan_semuanya_seketika() {
        let t = tema();
        let mut tree = pohon(slider(&t, 0.0).range(0.0..=100.0));
        let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
        advance(&mut tree, &tick);
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        tekan_tombol(&mut tree, &mut router, NamedKey::End);
        assert!(is_animating(&tree));
        settle(&mut tree);
        assert!(!is_animating(&tree));
        assert_eq!(node(&tree).positions()[0], 1.0);
    }

    // -- props ---------------------------------------------------------------

    #[test]
    fn nilai_dari_aplikasi_menang_kecuali_saat_jari_menempel() {
        let t = tema();
        let mut tree = pohon(slider(&t, 10.0).range(0.0..=100.0));
        // Rebuild dengan nilai baru: node ikut.
        reconcile(&mut tree, slider(&t, 70.0).range(0.0..=100.0));
        tree.layout(BoxConstraints::loose(RUANG));
        assert_eq!(node(&tree).value(), 70.0);

        // Saat jari menempel, props yang basi tidak boleh menarik thumb balik.
        let mut router = InputRouter::new();
        let g = geometri(&tree);
        let p = titik(&tree, g.thumb_x(0.3, TextDirection::Ltr));
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Down, p, Duration::ZERO)
                    .button(PointerButton::Primary),
            ),
        );
        let sedang = node(&tree).value();
        reconcile(&mut tree, slider(&t, 70.0).range(0.0..=100.0));
        tree.layout(BoxConstraints::loose(RUANG));
        assert_eq!(node(&tree).value(), sedang);
    }

    #[test]
    fn rebuild_tidak_menghapus_keadaan_interaksi() {
        let t = tema();
        let mut tree = pohon(slider(&t, 50.0).range(0.0..=100.0));
        let mut router = InputRouter::new();
        let id = sliders(&tree)[0];
        router.focus_node(&mut tree, Some(id));
        assert!(node(&tree).is_focused());

        reconcile(&mut tree, slider(&t, 50.0).range(0.0..=100.0).label("Baru"));
        tree.layout(BoxConstraints::loose(RUANG));
        assert!(node(&tree).is_focused(), "fokus hilang saat rebuild");
        assert_eq!(sliders(&tree)[0], id, "node diganti, bukan diperbarui");
    }

    #[test]
    fn permintaan_teknologi_bantu_menggerakkan_nilai() {
        let t = tema();
        let catat = Rc::new(RefCell::new(Vec::<f32>::new()));
        let tulis = catat.clone();
        let mut tree = pohon(
            slider(&t, 50.0)
                .range(0.0..=100.0)
                .step(5.0)
                .on_change(move |v| tulis.borrow_mut().push(v)),
        );
        let id = sliders(&tree)[0];
        let minta = |action, value: Option<&str>| AccessActionRequest {
            target: id,
            action,
            value: value.map(str::to_string),
        };

        assert!(apply_access_action(
            &mut tree,
            &minta(AccessAction::Increment, None)
        ));
        assert_eq!(node(&tree).value(), 55.0);
        assert!(apply_access_action(
            &mut tree,
            &minta(AccessAction::Decrement, None)
        ));
        assert_eq!(node(&tree).value(), 50.0);
        assert!(apply_access_action(
            &mut tree,
            &minta(AccessAction::SetValue, Some("77"))
        ));
        assert_eq!(node(&tree).value(), 75.0, "nilai dikte ikut snap ke step");
        assert!(!apply_access_action(
            &mut tree,
            &minta(AccessAction::SetValue, Some("bukan angka"))
        ));
        assert_eq!(catat.borrow().len(), 3);
    }
}
