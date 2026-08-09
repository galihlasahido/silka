//! `scroll_view()` — Tier 1 `KOMPONEN.md`: **momentum + rubber-band ala macOS,
//! scrollbar overlay auto-hide, scroll-to**.
//!
//! `KOMPONEN.md` menyebutnya pembeda paling awal yang benar-benar terasa
//! pengguna ("scroll_view dengan physics yang enak adalah pembeda rasa native
//! paling awal"). Karena itu ia bukan `viewport` dengan cat baru: yang
//! ditambahkan justru semua yang membuat guliran terasa hidup, dan tiap
//! bagiannya menempel pada satu keputusan dokumen.
//!
//! | Bagian | Keputusan yang ditaati |
//! |---|---|
//! | Rubber band + pantulan | Spring `(posisi, velocity)` yang bisa di-retarget (§3.5) — bukan kurva ease |
//! | Momentum | **Milik OS**, bukan simulasi kita (INTEGRASI-NATIVE §3, [`ScrollPhase`]) |
//! | Scrollbar | Warna, tebal, dan sudut dari token; squircle/arc ikut preset (§2.7) |
//! | Auto-hide | Spring peredup + hitung mundur di [`advance`]; tidak ada timer yang berdetak (§3.5) |
//! | Keyboard | Panah/Page/Home/End + focus ring — DoD `KOMPONEN.md`, bukan susulan |
//! | AccessKit | Peran [`AccessRole::ScrollView`] + aksi [`AccessActions::SCROLL`] yang **benar-benar jalan** ([`handle_access_action`]) |
//!
//! ```
//! # use rustui_theme::{Appearance, Theme};
//! # use rustui_core::view::{column, fixed};
//! use rustui_widgets::scroll_view;
//!
//! # let t = Theme::cupertino(Appearance::Dark);
//! let _ = scroll_view(&t, column((0..50).map(|_| fixed(320.0, 44.0))))
//!     .label("Daftar transaksi");
//! ```
//!
//! ## Momentum tidak ditiru — itu keputusan, bukan kekurangan
//!
//! macOS mengirim ekor inersianya sendiri setelah jari diangkat
//! ([`ScrollPhase::Momentum`]). Menyimulasikan fling sendiri di atasnya
//! menghasilkan guliran ganda yang terasa "licin" dan salah. Jadi yang kita
//! kerjakan hanyalah bagian yang **tidak** dikirim OS: rubber band saat isi
//! melewati tepi, dan pantulan kembali dengan kecepatan yang diwarisi dari ekor
//! inersia itu ([`physics::velocity_from`] → [`SpringValue::set_velocity`]).
//! Roda mouse — yang diskret dan tanpa inersia — digulir lewat spring supaya
//! satu detik tidak berubah jadi lompatan.
//!
//! ## Menjalankan animasinya
//!
//! Sama seperti [`crate::overlay`]: seluruh spring dimajukan di **satu** tempat,
//! [`advance`], yang dipanggil siklus frame aplikasi sebelum layout. Yang
//! dikembalikannya adalah alasan dirty — dan begitu tidak ada lagi yang
//! bergerak, ia kosong dan GPU benar-benar tidur (§3.5).
//!
//! ```
//! # use rustui_core::animation::{Motion, Tick};
//! # use rustui_core::scheduler::Dirty;
//! # use rustui_core::tree::{BoxConstraints, RenderTree};
//! # use rustui_core::view::{fixed, reconcile};
//! # use rustui_paint::Size;
//! # use rustui_theme::{Appearance, Theme};
//! # use std::time::Duration;
//! use rustui_widgets::scroll_view;
//! use rustui_widgets::scroll_view::{advance, nodes, scroll_to};
//!
//! # let t = Theme::cupertino(Appearance::Light);
//! let mut tree = RenderTree::new();
//! reconcile(&mut tree, scroll_view(&t, fixed(200.0, 2000.0)));
//! tree.layout(BoxConstraints::tight(Size::new(200.0, 400.0)));
//!
//! let sv = nodes(&tree)[0];
//! scroll_to(&mut tree, sv, 800.0);
//! let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
//! assert!(advance(&mut tree, &tick).contains(Dirty::ANIMATION));
//! ```
//!
//! ## Batas yang diketahui
//!
//! Hit-test menelusuri anak lebih dulu (Flutter, [`rustui_core::input::hit`]),
//! jadi tombol yang kebetulan berada **persis di bawah** scrollbar overlay
//! menerima klik lebih dulu daripada thumb-nya. Menukar prioritas itu adalah
//! perubahan di lapisan hit-test, bukan di widget ini; sampai saat itu jalur
//! aman yang sudah tersedia adalah memberi isi padding sebesar
//! [`ScrollbarStyle::hit_width`] pada sisi scrollbar.

pub mod physics;
#[cfg(test)]
mod tests;

use std::time::Duration;

use rustui_core::access::{
    AccessAction, AccessActionRequest, AccessActions, AccessNode, AccessRole,
};
use rustui_core::animation::{MotionRole, Spring, SpringValue, Tick};
use rustui_core::input::{
    Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode, Modifiers, NamedKey,
    PointerButton, PointerPhase, ScrollPhase,
};
use rustui_core::scheduler::Dirty;
use rustui_core::tree::{
    Axis, BoxConstraints, Decoration, FocusRing, LayoutCtx, NodeId, PaintCtx, RenderNode,
    RenderTree,
};
use rustui_core::view::{Builder, Decorated, View, ViewNode};
use rustui_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, Size};
use rustui_theme::Theme;

use crate::button::MIN_HIT_TARGET;

pub use physics::{Thumb, RUBBER_BAND};

/// Lama diam sebelum scrollbar overlay memudar (kebiasaan macOS).
pub const AUTO_HIDE: Duration = Duration::from_millis(900);

// ---------------------------------------------------------------------------
// Kebijakan scrollbar
// ---------------------------------------------------------------------------

/// Kapan scrollbar terlihat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Scrollbar {
    /// Overlay yang muncul saat digulir/di-hover lalu memudar sendiri — bawaan
    /// macOS sejak Lion, dan bawaan kita.
    #[default]
    Auto,
    /// Selalu terlihat (daftar padat, tabel, preferensi "always" di macOS).
    Always,
    /// Tidak pernah digambar. Guliran tetap jalan — ini soal tampilan, bukan
    /// soal kemampuan.
    Hidden,
}

impl Scrollbar {
    /// Benar bila kebijakan ini pernah menggambar scrollbar sama sekali.
    pub fn is_visible(self) -> bool {
        !matches!(self, Scrollbar::Hidden)
    }
}

/// Rupa scrollbar — seluruh nilainya **sudah diresolusi dari token** satu
/// tingkat di atas (§2.6, §2.7), jadi node tidak punya pendapat soal warna.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarStyle {
    /// Tebal thumb saat diam, poin logis.
    pub thickness: f32,
    /// Tebal thumb saat penunjuk berada di jalurnya (macOS melebarkannya).
    pub thickness_hover: f32,
    /// Jarak thumb dari tepi wadah.
    pub margin: f32,
    /// Warna thumb saat diam.
    pub thumb: Color,
    /// Warna thumb saat di-hover/diseret.
    pub thumb_active: Color,
    /// Latar jalur, hanya terlihat saat scrollbar melebar.
    pub track: Color,
    /// Bentuk sudut thumb — squircle di Cupertino, arc di Tailwind.
    pub corners: Corners,
}

impl ScrollbarStyle {
    /// Rupa bawaan dari token theme.
    pub fn from_theme(theme: &Theme) -> Self {
        let thickness = theme.space(1.75);
        Self {
            thickness,
            thickness_hover: theme.space(3.0),
            margin: theme.space(0.5),
            thumb: theme.color.tertiary_label,
            thumb_active: theme.color.secondary_label,
            track: theme.color.surface_sunken,
            corners: theme.corners(thickness / 2.0),
        }
    }

    /// Lebar area **sentuh** scrollbar — ≥ 44pt walau visualnya beberapa poin
    /// saja (HIG; aturan yang sama dengan `icon_button`).
    pub fn hit_width(&self) -> f32 {
        MIN_HIT_TARGET.max(self.thickness_hover + self.margin * 2.0)
    }

    /// Tebal thumb pada kemajuan pelebaran `t` (0 = diam, 1 = melebar penuh).
    fn thickness_at(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        self.thickness + (self.thickness_hover - self.thickness) * t
    }
}

/// Nama lain [`ScrollbarStyle`], dipertahankan supaya kedua ejaan yang wajar
/// sama-sama benar di kode pemakai.
pub type ScrollBar = ScrollbarStyle;

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// Apa yang berubah pada satu detak [`ScrollView::advance`].
///
/// Dua bendera terpisah karena akibatnya berbeda: isi yang **pindah** memaksa
/// layout subtree diulang, sedangkan scrollbar yang memudar atau melebar hanya
/// piksel. Menyamakan keduanya berarti setiap scrollbar yang memudar akan
/// menghitung ulang seluruh isi daftar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Advanced {
    /// Posisi guliran berubah frame ini.
    pub moved: bool,
    /// Ada piksel scrollbar yang berubah frame ini.
    pub repainted: bool,
}

impl Advanced {
    /// Benar bila tidak ada apa pun yang berubah.
    pub fn is_none(self) -> bool {
        !self.moved && !self.repainted
    }
}

/// Node render `scroll_view`.
///
/// Ia **relayout boundary permanen** dan memotong isinya, dua sifat yang
/// diwarisi dari [`rustui_core::tree::Viewport`] dan sama pentingnya: isi
/// setinggi apa pun tidak pernah membuat window di-layout ulang, dan baris yang
/// sudah tergulir keluar tidak bisa diklik.
pub struct ScrollView {
    /// Sumbu guliran.
    pub axis: Axis,
    /// Tinggi satu baris roda mouse, poin logis (token tipografi).
    pub line_height: f32,
    /// Kapan scrollbar terlihat.
    pub scrollbar: Scrollbar,
    /// Rupa scrollbar.
    pub bar: ScrollbarStyle,
    /// Isi boleh melar melewati tepi (rubber band).
    pub rubber_band: bool,
    /// Ikut navigasi keyboard (Tab) selama isinya memang bisa digulir.
    pub focusable: bool,
    /// Latar area gulir — token `surface_sunken` bila diisi.
    pub decoration: Decoration,
    /// Bentuk sudut area, dipakai hit-test **dan** cincin fokus (§3.6).
    pub corners: Corners,
    /// Cincin fokus keyboard.
    pub focus_ring: Option<FocusRing>,
    /// Nama yang dibacakan screen reader.
    pub label: Option<String>,

    /// Posisi guliran: satu-satunya nilai yang benar-benar dianimasikan.
    offset: SpringValue<f32>,
    /// Peredup scrollbar overlay (0 = tersembunyi).
    fade: SpringValue<f32>,
    /// Pelebaran scrollbar saat di-hover/diseret.
    expand: SpringValue<f32>,
    /// Ukuran wadah dari layout terakhir.
    viewport: Size,
    /// Ukuran isi pada sumbu guliran dari layout terakhir.
    content: f32,
    /// Lama tidak ada interaksi guliran (untuk auto-hide).
    idle: Duration,
    /// Sedang memegang fokus keyboard.
    focused: bool,
    /// Penunjuk berada di jalur scrollbar.
    over_bar: bool,
    /// Sedang menyeret thumb; nilainya jarak genggam dari awal thumb.
    drag: Option<f32>,
    /// Gesture trackpad sedang berlangsung (jari masih menempel).
    gesture: bool,
    /// Waktu event guliran terakhir — dasar perkiraan kecepatan momentum.
    last_scroll: Option<Duration>,
    /// Posisi terakhir yang **diminta aplikasi** lewat props (controlled).
    controlled: Option<f32>,
}

impl Default for ScrollView {
    fn default() -> Self {
        Self {
            axis: Axis::Vertical,
            line_height: 40.0,
            scrollbar: Scrollbar::default(),
            bar: ScrollbarStyle {
                thickness: 7.0,
                thickness_hover: 12.0,
                margin: 2.0,
                thumb: Color::TRANSPARENT,
                thumb_active: Color::TRANSPARENT,
                track: Color::TRANSPARENT,
                corners: Corners::SHARP,
            },
            rubber_band: true,
            focusable: true,
            decoration: Decoration::NONE,
            corners: Corners::SHARP,
            focus_ring: None,
            label: None,
            offset: default_offset_spring(Spring::smooth()),
            fade: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            expand: SpringValue::new(0.0)
                .with_spring(Spring::snappy())
                .decorative(),
            viewport: Size::ZERO,
            content: 0.0,
            idle: Duration::ZERO,
            focused: false,
            over_bar: false,
            drag: None,
            gesture: false,
            last_scroll: None,
            controlled: None,
        }
    }
}

/// Spring posisi guliran.
///
/// **Dekoratif** dengan sengaja ([`MotionRole::Decorative`]): yang membawa
/// informasi adalah *di mana isi berhenti*, bukan perjalanannya. Karena itu di
/// bawah reduced-motion guliran tetap sampai ke tempat yang benar — hanya
/// meluncurnya yang hilang (§3.5, DoD `KOMPONEN.md`).
fn default_offset_spring(spring: Spring) -> SpringValue<f32> {
    SpringValue::new(0.0).with_spring(spring).decorative()
}

impl ScrollView {
    /// Posisi guliran saat ini, poin logis. Bisa di luar `0..=max` selama isi
    /// sedang melar (rubber band).
    pub fn offset(&self) -> f32 {
        self.offset.position()
    }

    /// Posisi yang sedang dituju.
    pub fn target(&self) -> f32 {
        self.offset.target()
    }

    /// Ukuran isi pada sumbu guliran (hasil layout terakhir).
    pub fn content(&self) -> f32 {
        self.content
    }

    /// Ukuran wadah pada sumbu guliran (hasil layout terakhir).
    pub fn extent(&self) -> f32 {
        self.axis.main_of(self.viewport)
    }

    /// Guliran maksimum; nol berarti isi muat seluruhnya.
    pub fn max_scroll(&self) -> f32 {
        physics::max_scroll(self.extent(), self.content)
    }

    /// Benar bila ada yang bisa digulir sama sekali.
    pub fn can_scroll(&self) -> bool {
        self.max_scroll() > 0.0
    }

    /// Kemajuan guliran 0..1 (0 bila tidak bisa digulir).
    pub fn progress(&self) -> f32 {
        let max = self.max_scroll();
        if max <= 0.0 {
            0.0
        } else {
            (self.offset() / max).clamp(0.0, 1.0)
        }
    }

    /// Peredup scrollbar saat ini (0 = tak terlihat).
    pub fn bar_opacity(&self) -> f32 {
        match self.scrollbar {
            Scrollbar::Hidden => 0.0,
            Scrollbar::Always => 1.0,
            Scrollbar::Auto => self.fade.position().clamp(0.0, 1.0),
        }
    }

    /// Geometri thumb saat ini, bila memang ada yang bisa digulir.
    pub fn thumb(&self) -> Option<Thumb> {
        physics::thumb(self.extent(), self.content, self.offset(), MIN_HIT_TARGET)
    }

    /// Benar bila spring guliran/scrollbar masih bergerak.
    pub fn is_animating(&self) -> bool {
        self.offset.is_animating() || self.fade.is_animating() || self.expand.is_animating()
    }

    /// Benar bila node ini masih membutuhkan frame berikutnya.
    ///
    /// Lebih luas dari [`ScrollView::is_animating`] karena **hitung mundur
    /// auto-hide** juga butuh frame walau tidak ada satu piksel pun yang
    /// bergerak. Itu tetap bukan timer: begitu scrollbar memudar, nilainya
    /// kembali salah dan tidak ada lagi yang diminta (§3.5).
    pub fn wants_frame(&self) -> bool {
        self.is_animating() || (self.scrollbar == Scrollbar::Auto && self.fade.target() > 0.0)
    }

    /// Benar bila isi sedang melar melewati tepi.
    pub fn is_overscrolled(&self) -> bool {
        physics::overshoot(self.offset(), self.max_scroll()) != 0.0
    }

    /// Spring yang menjalankan guliran.
    pub fn spring(&self) -> Spring {
        self.offset.spring()
    }

    /// Ganti spring tanpa mengganggu gerakan yang sedang berjalan.
    pub fn set_spring(&mut self, spring: Spring) {
        self.offset.set_spring(spring);
    }

    /// **Scroll-to**: arahkan guliran ke `offset` dengan spring.
    ///
    /// Retarget, bukan animasi baru: memanggilnya di tengah guliran membelokkan
    /// gerakan sambil membawa kecepatannya (§3.5). Benar bila tujuannya berubah.
    pub fn scroll_to(&mut self, offset: f32) -> bool {
        let tujuan = physics::clamp_scroll(offset, self.max_scroll());
        if self.offset.target() == tujuan && !self.is_overscrolled() {
            return false;
        }
        self.offset.set_target(tujuan);
        self.show_bar();
        true
    }

    /// Lompat ke `offset` seketika (memuat state, pindah halaman).
    pub fn jump_to(&mut self, offset: f32) -> bool {
        let tujuan = physics::clamp_scroll(offset, self.max_scroll());
        if self.offset.position() == tujuan && !self.offset.is_animating() {
            return false;
        }
        self.offset.jump_to(tujuan);
        true
    }

    /// Geser guliran sejauh `delta` (positif = isi naik) dengan spring.
    pub fn scroll_by(&mut self, delta: f32) -> bool {
        self.scroll_to(self.offset.target() + delta)
    }

    /// Gulirkan sampai rentang `[start, start + extent]` **pada koordinat isi**
    /// terlihat penuh.
    pub fn reveal(&mut self, start: f32, extent: f32, padding: f32) -> bool {
        let tujuan =
            physics::scroll_to_reveal(self.offset.target(), self.extent(), start, extent, padding);
        self.scroll_to(tujuan)
    }

    /// Tampilkan scrollbar dan setel ulang hitung mundur auto-hide.
    fn show_bar(&mut self) {
        self.idle = Duration::ZERO;
        if self.scrollbar == Scrollbar::Auto && self.can_scroll() {
            self.fade.set_target(1.0);
        }
    }

    /// Benar bila pengguna sedang menyentuh guliran ini (jari, thumb, hover).
    fn interacting(&self) -> bool {
        self.gesture || self.drag.is_some() || self.over_bar
    }

    /// Majukan seluruh spring satu frame; yang kembali adalah **apa** yang
    /// berubah.
    ///
    /// Di sinilah auto-hide hidup: selama hitung mundur berjalan node meminta
    /// frame berikutnya lewat [`Tick::keep_awake`], dan begitu scrollbar
    /// memudar habis tidak ada lagi yang meminta apa pun — tidak ada timer yang
    /// berdetak di latar (§3.5).
    pub fn advance(&mut self, tick: &Tick) -> Advanced {
        let sebelum = (
            self.offset.position(),
            self.fade.position(),
            self.expand.position(),
        );
        tick.advance(&mut self.offset);
        tick.advance(&mut self.fade);
        tick.advance(&mut self.expand);

        if self.scrollbar == Scrollbar::Auto
            && self.fade.target() > 0.0
            && !self.interacting()
            && !self.offset.is_animating()
        {
            self.idle = self.idle.saturating_add(tick.dt());
            if self.idle >= AUTO_HIDE {
                self.fade.set_target(0.0);
            } else {
                // Hitung mundur belum selesai: satu frame lagi, bukan timer.
                tick.keep_awake();
            }
        } else if self.interacting() || self.offset.is_animating() {
            self.idle = Duration::ZERO;
        }

        Advanced {
            moved: self.offset.position() != sebelum.0,
            repainted: self.fade.position() != sebelum.1 || self.expand.position() != sebelum.2,
        }
    }

    /// Selesaikan seluruh gerakan seketika (uji, snapshot, `jump_to` internal).
    pub fn settle(&mut self) {
        self.offset.settle();
        self.fade.settle();
        self.expand.settle();
    }

    // -- geometri scrollbar ------------------------------------------------

    /// Kotak **sentuh** scrollbar dalam koordinat lokal.
    fn bar_region(&self) -> Rect {
        let s = self.viewport;
        let tebal = self.bar.hit_width();
        match self.axis {
            Axis::Vertical => {
                let w = tebal.min(s.width);
                Rect::new(s.width - w, 0.0, w, s.height)
            }
            Axis::Horizontal => {
                let h = tebal.min(s.height);
                Rect::new(0.0, s.height - h, s.width, h)
            }
        }
    }

    /// Kotak jalur scrollbar, yang hanya digambar saat scrollbar melebar.
    fn bar_track_rect(&self) -> Rect {
        let s = self.viewport;
        let tebal = self.bar.thickness_at(self.expand.position()) + self.bar.margin * 2.0;
        match self.axis {
            Axis::Vertical => Rect::new((s.width - tebal).max(0.0), 0.0, tebal, s.height),
            Axis::Horizontal => Rect::new(0.0, (s.height - tebal).max(0.0), s.width, tebal),
        }
    }

    /// Kotak **gambar** thumb dalam koordinat lokal.
    fn thumb_rect(&self, t: Thumb) -> Rect {
        let s = self.viewport;
        let tebal = self.bar.thickness_at(self.expand.position());
        match self.axis {
            Axis::Vertical => Rect::new(
                (s.width - self.bar.margin - tebal).max(0.0),
                t.offset,
                tebal,
                t.length,
            ),
            Axis::Horizontal => Rect::new(
                t.offset,
                (s.height - self.bar.margin - tebal).max(0.0),
                t.length,
                tebal,
            ),
        }
    }

    /// Komponen sumbu guliran dari sebuah titik lokal.
    fn main_of_point(&self, p: Point) -> f32 {
        match self.axis {
            Axis::Vertical => p.y,
            Axis::Horizontal => p.x,
        }
    }

    // -- guliran -----------------------------------------------------------

    /// Selisih guliran pada sumbu wadah ini, poin logis.
    ///
    /// Positif = isi bergerak naik/kiri (posisi guliran bertambah). Wadah
    /// mendatar juga menerima roda vertikal: itu satu-satunya cara menggulir
    /// daftar mendatar dengan mouse biasa.
    fn main_delta(&self, delta: Point) -> f32 {
        match self.axis {
            Axis::Vertical => -delta.y,
            Axis::Horizontal => {
                if delta.x != 0.0 {
                    -delta.x
                } else {
                    -delta.y
                }
            }
        }
    }

    fn handle_scroll(&mut self, ctx: &mut EventCtx<'_>, e: &rustui_core::input::ScrollEvent) {
        let gerak = self.main_delta(e.delta.to_points(self.line_height));
        let max = self.max_scroll();
        let dt = e.time.saturating_sub(self.last_scroll.unwrap_or(e.time));
        self.last_scroll = Some(e.time);

        // Tidak ada yang bisa digulir: **jangan** ditelan — wadah di atasnya
        // yang mengambil alih (scroll chaining).
        if !self.can_scroll() {
            return;
        }

        match e.phase {
            ScrollPhase::Began | ScrollPhase::Changed => {
                self.gesture = true;
                let posisi = self.offset.position();
                let baru = if self.rubber_band {
                    physics::apply_delta(posisi, gerak, max, self.extent(), RUBBER_BAND)
                } else {
                    physics::clamp_scroll(posisi + gerak, max)
                };
                if baru == posisi {
                    return;
                }
                // Jari yang menempel = manipulasi langsung, bukan animasi:
                // isinya harus persis di bawah jari.
                self.offset.jump_to(baru);
                self.show_bar();
                ctx.request_layout();
                ctx.handled();
            }
            ScrollPhase::Momentum => {
                let posisi = self.offset.position();
                let simpangan = physics::overshoot(posisi, max);
                if simpangan != 0.0 {
                    // Ekor inersia OS sudah membentur tepi: mulai pantulan
                    // dengan kecepatan yang diwarisi darinya (§3.5 handoff).
                    self.offset.set_target(physics::nearest_bound(posisi, max));
                    self.offset.set_velocity(physics::velocity_from(gerak, dt));
                    self.show_bar();
                    ctx.request_animation();
                    ctx.request_layout();
                    ctx.handled();
                    return;
                }
                let baru = if self.rubber_band {
                    physics::apply_delta(posisi, gerak, max, self.extent(), RUBBER_BAND)
                } else {
                    physics::clamp_scroll(posisi + gerak, max)
                };
                if baru == posisi {
                    return;
                }
                self.offset.jump_to(baru);
                self.show_bar();
                ctx.request_layout();
                ctx.handled();
            }
            ScrollPhase::Ended | ScrollPhase::MomentumEnded => {
                self.gesture = false;
                self.last_scroll = None;
                if self.is_overscrolled() {
                    self.offset
                        .set_target(physics::nearest_bound(self.offset.position(), max));
                    ctx.request_animation();
                    ctx.request_layout();
                }
                self.show_bar();
                ctx.handled();
            }
            // `ScrollPhase` non-exhaustive: tahap baru dari platform
            // diperlakukan seperti roda — diskret, dijalankan lewat spring.
            _ => {
                // Roda mouse itu diskret dan tanpa inersia: yang membuatnya
                // terasa halus adalah spring, bukan lompatan per klik.
                let tujuan = physics::clamp_scroll(self.offset.target() + gerak, max);
                if tujuan == self.offset.target() && !self.offset.is_animating() {
                    return;
                }
                self.offset.set_target(tujuan);
                self.show_bar();
                ctx.request_animation();
                ctx.request_layout();
                ctx.handled();
            }
        }
    }

    fn handle_pointer(&mut self, ctx: &mut EventCtx<'_>, e: &rustui_core::input::PointerEvent) {
        let lokal = ctx.local();
        let utama = self.main_of_point(lokal);
        let di_jalur =
            self.scrollbar.is_visible() && self.can_scroll() && self.bar_region().contains(lokal);

        match e.phase {
            PointerPhase::Enter | PointerPhase::Move => {
                if let Some(genggam) = self.drag {
                    let tujuan = physics::scroll_for_thumb(
                        self.extent(),
                        self.content,
                        utama - genggam,
                        MIN_HIT_TARGET,
                    );
                    if self.offset.position() != tujuan {
                        self.offset.jump_to(tujuan);
                        ctx.request_layout();
                    }
                    ctx.handled();
                    return;
                }
                if di_jalur != self.over_bar {
                    self.over_bar = di_jalur;
                    self.expand.set_target(if di_jalur { 1.0 } else { 0.0 });
                    if di_jalur {
                        self.show_bar();
                    } else {
                        self.idle = Duration::ZERO;
                    }
                    ctx.request_animation();
                    ctx.request_paint();
                }
            }
            PointerPhase::Leave => {
                if self.over_bar {
                    self.over_bar = false;
                    self.expand.set_target(0.0);
                    self.idle = Duration::ZERO;
                    ctx.request_animation();
                    ctx.request_paint();
                }
            }
            PointerPhase::Down if e.button == Some(PointerButton::Primary) => {
                if di_jalur {
                    if let Some(t) = self.thumb() {
                        if t.contains(utama) {
                            self.drag = Some(utama - t.offset);
                            self.expand.set_target(1.0);
                            self.show_bar();
                            ctx.capture_pointer();
                            ctx.request_animation();
                            ctx.request_paint();
                        } else {
                            // Klik di jalur = satu halaman ke arah klik, aturan
                            // AppKit dengan "jump to spot" dimatikan.
                            let arah = if utama < t.offset { -1.0 } else { 1.0 };
                            self.scroll_by(
                                arah * physics::page_step(self.extent(), self.line_height),
                            );
                            ctx.request_animation();
                            ctx.request_layout();
                        }
                        ctx.handled();
                    }
                }
                // Klik di dalam area gulir memindahkan fokus keyboard ke sini —
                // itulah yang membuat panah langsung bekerja tanpa Tab dulu.
                if self.focusable && self.can_scroll() && !ctx.is_handled() {
                    ctx.request_focus();
                }
            }
            PointerPhase::Up if e.button == Some(PointerButton::Primary) => {
                if self.drag.take().is_some() {
                    self.expand
                        .set_target(if self.over_bar { 1.0 } else { 0.0 });
                    ctx.release_pointer();
                    ctx.request_animation();
                    ctx.request_paint();
                    ctx.handled();
                }
            }
            PointerPhase::Cancel => {
                if self.drag.take().is_some() {
                    self.expand.set_target(0.0);
                    ctx.request_animation();
                    ctx.request_paint();
                }
            }
            _ => {}
        }
    }

    fn handle_key(&mut self, ctx: &mut EventCtx<'_>, e: &rustui_core::input::KeyEvent) {
        if !self.can_scroll() {
            return;
        }
        let baris = self.line_height;
        let halaman = physics::page_step(self.extent(), baris);
        let max = self.max_scroll();
        let mendatar = self.axis == Axis::Horizontal;

        let sekarang = self.offset.target();
        let polos = e.modifiers.is_empty();
        let tujuan = match &e.code {
            KeyCode::Named(NamedKey::ArrowDown) if !mendatar && polos => Some(sekarang + baris),
            KeyCode::Named(NamedKey::ArrowUp) if !mendatar && polos => Some(sekarang - baris),
            KeyCode::Named(NamedKey::ArrowRight) if mendatar && polos => Some(sekarang + baris),
            KeyCode::Named(NamedKey::ArrowLeft) if mendatar && polos => Some(sekarang - baris),
            KeyCode::Named(NamedKey::PageDown) if polos => Some(sekarang + halaman),
            KeyCode::Named(NamedKey::PageUp) if polos => Some(sekarang - halaman),
            // Spasi menggulir satu halaman (AppKit, dan setiap browser);
            // Shift+Spasi kembali ke atas.
            KeyCode::Named(NamedKey::Space) if polos => Some(sekarang + halaman),
            KeyCode::Named(NamedKey::Space) if e.modifiers.is_exactly(Modifiers::SHIFT) => {
                Some(sekarang - halaman)
            }
            KeyCode::Named(NamedKey::Home) if polos => Some(0.0),
            KeyCode::Named(NamedKey::End) if polos => Some(max),
            _ => None,
        };
        let Some(tujuan) = tujuan else { return };
        self.scroll_to(tujuan);
        ctx.request_animation();
        ctx.request_layout();
        ctx.handled();
    }
}

impl RenderNode for ScrollView {
    fn is_relayout_boundary(&self) -> bool {
        true
    }

    fn clips_children(&self) -> bool {
        true
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // Aturan Flutter yang sama: sumbu guliran WAJIB terbatas. Bug layout
        // harus berisik, bukan diam-diam setinggi nol.
        debug_assert!(
            match self.axis {
                Axis::Vertical => constraints.has_bounded_height(),
                Axis::Horizontal => constraints.has_bounded_width(),
            },
            "scroll_view {:?} menerima sumbu guliran tanpa batas — beri pembatas ukuran di atasnya",
            self.axis
        );
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
        self.viewport = ukuran;

        if ctx.child_count() == 0 {
            self.content = 0.0;
            self.offset.jump_to(0.0);
            return ukuran;
        }

        let child = ctx.child(0);
        let batas_anak = match self.axis {
            Axis::Vertical => BoxConstraints::new(ukuran.width, ukuran.width, 0.0, f32::INFINITY),
            Axis::Horizontal => {
                BoxConstraints::new(0.0, f32::INFINITY, ukuran.height, ukuran.height)
            }
        };
        // **`layout_child`, bukan `layout_child_boundary`** — dan itu bukan
        // kelalaian. Ukuran kita memang tidak bergantung pada isi (wadah ini
        // sendiri sudah `is_relayout_boundary`, jadi window di atasnya tetap
        // aman), tapi **guliran maksimum bergantung penuh** padanya. Kalau isi
        // dijadikan boundary sendiri, daftar yang menyusut tidak akan pernah
        // memberi tahu kita, dan pengguna tertinggal menatap ruang kosong yang
        // tidak bisa digulir pulang.
        let ukuran_anak = ctx.layout_child(child, batas_anak);
        self.content = self.axis.main_of(ukuran_anak);

        // Isi yang menyusut (atau window yang membesar) tidak boleh menyisakan
        // ruang kosong di bawah. Yang dijepit adalah **tujuan**, bukan posisi,
        // supaya guliran yang sedang berjalan tetap mulus.
        let max = self.max_scroll();
        if !self.gesture && self.drag.is_none() {
            let tujuan = self.offset.target();
            let jepit = physics::clamp_scroll(tujuan, max);
            if jepit != tujuan {
                // **Tujuan** yang di luar rentang berarti isinya yang berubah
                // (atau window yang membesar), bukan rubber band — dan ruang
                // kosong di bawah daftar bukan sesuatu yang perlu dianimasikan.
                self.offset.jump_to(jepit);
            } else if !self.offset.is_animating()
                && physics::overshoot(self.offset.position(), max) != 0.0
            {
                // Simpangan yang tertinggal tanpa spring yang menariknya
                // pulang: jaring pengaman, bukan jalur normal.
                self.offset.jump_to(jepit);
            }
        }

        let geser = -self.offset.position();
        let offset = match self.axis {
            Axis::Vertical => Point::new(0.0, geser),
            Axis::Horizontal => Point::new(geser, 0.0),
        };
        ctx.place_child(child, offset);
        ukuran
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.decoration);
        ctx.paint_children();

        // Scrollbar digambar **di atas** isi dan **di luar** clip anak: ia
        // melayang, bukan ikut tergulir.
        if let Some(t) = self.thumb() {
            let alpha = self.bar_opacity();
            if alpha > 0.0 {
                let lebar = self.expand.position().clamp(0.0, 1.0);
                if self.bar.track.a > 0.0 && lebar > 0.0 {
                    let jalur = self.bar_track_rect();
                    ctx.quad(
                        Quad::new(jalur)
                            .background(self.bar.track.with_alpha(self.bar.track.a * alpha * lebar))
                            .corners(self.bar.corners),
                    );
                }
                let warna = self
                    .bar
                    .thumb
                    .lerp(self.bar.thumb_active, lebar)
                    .with_alpha(self.bar.thumb.a * alpha);
                ctx.quad(
                    Quad::new(self.thumb_rect(t))
                        .background(warna)
                        .corners(self.bar.corners),
                );
            }
        }

        if self.focused {
            if let Some(ring) = self.focus_ring.filter(|r| r.width > 0.0 && r.color.a > 0.0) {
                let kotak = ctx.local_bounds().deflate(Insets::all(-ring.width));
                let corners = Corners::new(
                    CornerRadii::all(self.corners.radii.max() + ring.width),
                    self.corners.style,
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
        node.role = AccessRole::ScrollView;
        node.label.clone_from(&self.label);
        if self.can_scroll() {
            node.actions |= AccessActions::SCROLL;
            if self.focusable {
                node.actions |= AccessActions::FOCUS;
            }
            // Posisi dibacakan sebagai persen: satu-satunya bentuk yang berarti
            // bagi pengguna screen reader, dan datang dari hasil layout yang
            // sama dengan yang digambar (§3.8).
            node.value = Some(format!("{}%", (self.progress() * 100.0).round() as i32));
        }
    }

    fn hit_shape(&self) -> HitShape {
        if self.corners.radii.is_sharp() {
            HitShape::Rect
        } else {
            HitShape::Rounded(self.corners)
        }
    }

    /// Permukaan yang bisa digulir itu padat: guliran di atas area kosongnya
    /// tetap miliknya, dan klik tidak menembus ke apa pun di belakangnya.
    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        // Wadah yang isinya muat bukan tempat berhenti Tab: tidak ada yang bisa
        // dilakukan keyboard di sana.
        if self.focusable && self.can_scroll() {
            FocusPolicy::FOCUSABLE
        } else {
            FocusPolicy::NONE
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        match event {
            Event::Scroll(e) => self.handle_scroll(ctx, e),
            Event::Pointer(e) => self.handle_pointer(ctx, e),
            Event::Key(e) if e.is_pressed() => self.handle_key(ctx, e),
            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if self.focused {
                    self.show_bar();
                    ctx.request_animation();
                }
                ctx.request_paint();
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for ScrollView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ScrollView")
            .field("axis", &self.axis)
            .field("offset", &self.offset.position())
            .field("target", &self.offset.target())
            .field("content", &self.content)
            .field("viewport", &self.viewport)
            .field("bar", &self.bar_opacity())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props `scroll_view` — bentuk view dari [`ScrollView`].
#[derive(Debug, Clone, PartialEq)]
pub struct ScrollProps {
    axis: Axis,
    scroll: Option<f32>,
    line_height: f32,
    scrollbar: Scrollbar,
    bar: ScrollbarStyle,
    rubber_band: bool,
    focusable: bool,
    decoration: Decoration,
    corners: Corners,
    focus_ring: Option<FocusRing>,
    label: Option<String>,
    spring: Spring,
    motion: MotionRole,
}

impl Decorated for ScrollProps {
    fn decoration_mut(&mut self) -> &mut Decoration {
        &mut self.decoration
    }
}

impl ScrollProps {
    fn spring_value(&self) -> SpringValue<f32> {
        let mut v = SpringValue::new(self.scroll.unwrap_or(0.0)).with_spring(self.spring);
        if self.motion == MotionRole::Decorative {
            v = v.decorative();
        }
        v
    }
}

impl ViewNode for ScrollProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ScrollView {
            axis: self.axis,
            line_height: self.line_height,
            scrollbar: self.scrollbar,
            bar: self.bar,
            rubber_band: self.rubber_band,
            focusable: self.focusable,
            decoration: self.decoration,
            corners: self.corners,
            focus_ring: self.focus_ring,
            label: self.label.clone(),
            offset: self.spring_value(),
            controlled: self.scroll,
            ..ScrollView::default()
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ScrollView>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.axis != self.axis {
            n.axis = self.axis;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.line_height != self.line_height {
            n.line_height = self.line_height;
        }
        if n.scrollbar != self.scrollbar {
            n.scrollbar = self.scrollbar;
            dirty |= Dirty::PAINT;
        }
        if n.bar != self.bar {
            n.bar = self.bar;
            dirty |= Dirty::PAINT;
        }
        if n.rubber_band != self.rubber_band {
            n.rubber_band = self.rubber_band;
        }
        if n.focusable != self.focusable {
            n.focusable = self.focusable;
            dirty |= Dirty::PAINT;
        }
        if n.decoration != self.decoration {
            n.decoration = self.decoration;
            dirty |= Dirty::PAINT;
        }
        if n.corners != self.corners {
            n.corners = self.corners;
            dirty |= Dirty::PAINT;
        }
        if n.focus_ring != self.focus_ring {
            n.focus_ring = self.focus_ring;
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.offset.spring() != self.spring {
            n.offset.set_spring(self.spring);
        }

        // **Controlled hanya saat aplikasi benar-benar mengubah angkanya.**
        // Membandingkannya dengan posisi node akan melempar pengguna kembali ke
        // atas setiap kali ada signal lain berubah — bug klasik "controlled
        // component" yang justru muncul karena roda mouse memiliki posisinya.
        if self.scroll != n.controlled {
            n.controlled = self.scroll;
            if let Some(v) = self.scroll {
                if n.scroll_to(v) {
                    dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
                }
            }
        }
        dirty
    }
}

/// Wadah bergulir berisi `child` — konstruktor gaya Dart (§2.5).
///
/// Seluruh nilainya datang dari `theme`: warna scrollbar, tebal, sudut (squircle
/// di Cupertino, arc di Tailwind), tinggi baris roda, dan cincin fokus.
pub fn scroll_view(theme: &Theme, child: impl Into<View>) -> ScrollBuilder {
    ScrollBuilder {
        key: None,
        props: ScrollProps {
            axis: Axis::Vertical,
            scroll: None,
            // Satu "baris" roda mouse = satu baris teks badan, bukan konstanta
            // desktop yang ditebak (INTEGRASI-NATIVE §3).
            line_height: theme.typography.body_size * theme.typography.body_line_height,
            scrollbar: Scrollbar::default(),
            bar: ScrollbarStyle::from_theme(theme),
            rubber_band: true,
            focusable: true,
            decoration: Decoration::NONE,
            corners: Corners::SHARP,
            focus_ring: Some(FocusRing::new(theme.space(0.5), theme.color.focus_ring)),
            label: None,
            spring: Spring::smooth(),
            motion: MotionRole::Decorative,
        },
        child: child.into(),
    }
}

/// Builder `scroll_view` bergaya Dart (§2.5).
///
/// Tipe sendiri, bukan [`rustui_core::view::Builder`], karena aturan orphan
/// Rust: method chain sebuah widget hanya boleh hidup di crate yang memiliki
/// tipenya. Bentuk penulisannya tetap sama persis dengan primitif inti —
/// itulah yang penting bagi pemakai (`KOMPONEN.md`).
#[derive(Debug)]
pub struct ScrollBuilder {
    key: Option<rustui_core::signals::Key>,
    props: ScrollProps,
    child: View,
}

impl From<ScrollBuilder> for View {
    fn from(b: ScrollBuilder) -> View {
        let mut builder = Builder::new(b.props).child(b.child);
        if let Some(key) = b.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl ScrollBuilder {
    fn map(mut self, f: impl FnOnce(&mut ScrollProps)) -> Self {
        f(&mut self.props);
        self
    }

    /// Kunci identitas di antara saudara-saudaranya (§2.5).
    pub fn key(mut self, key: impl Into<rustui_core::signals::Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    // -- utility styling (§2.6): nilainya selalu token, tidak pernah literal --

    /// Warna latar area gulir — token `surface_sunken` biasanya.
    pub fn background(self, color: Color) -> Self {
        self.map(move |p| p.decoration.background = color)
    }

    /// Geometri sudut area: squircle di Cupertino, arc di Tailwind — dan
    /// bentuk yang sama dipakai hit-testing (§3.6).
    pub fn corners(self, corners: Corners) -> Self {
        self.map(move |p| {
            p.corners = corners;
            p.decoration.corners = corners;
        })
    }

    /// Border setebal `width` berwarna `color` (token `separator`/`border`).
    pub fn border(self, width: f32, color: Color) -> Self {
        self.map(move |p| {
            p.decoration.border_width = width.max(0.0);
            p.decoration.border_color = color;
        })
    }

    /// Bayangan ganda ala HIG untuk satu tingkat elevasi.
    pub fn shadow(self, shadows: rustui_paint::ShadowPair) -> Self {
        self.map(move |p| p.decoration.shadows = shadows)
    }

    /// Sumbu guliran.
    pub fn axis(self, axis: Axis) -> Self {
        self.map(move |p| p.axis = axis)
    }

    /// Gulir mendatar.
    pub fn horizontal(self) -> Self {
        self.axis(Axis::Horizontal)
    }

    /// Gulir menegak (bawaan).
    pub fn vertical(self) -> Self {
        self.axis(Axis::Vertical)
    }

    /// Kendalikan posisi guliran dari aplikasi (mis. tombol "ke atas").
    ///
    /// Diterapkan **hanya saat angkanya berubah**, dan diterapkan sebagai
    /// animasi spring — bukan lompatan.
    pub fn scroll(self, offset: f32) -> Self {
        self.map(move |p| p.scroll = Some(offset))
    }

    /// Tinggi satu baris roda mouse, poin logis.
    pub fn line_height(self, line_height: f32) -> Self {
        self.map(move |p| p.line_height = line_height.max(1.0))
    }

    /// Kapan scrollbar terlihat.
    pub fn scrollbar(self, scrollbar: Scrollbar) -> Self {
        self.map(move |p| p.scrollbar = scrollbar)
    }

    /// Tanpa scrollbar (guliran tetap jalan).
    pub fn no_scrollbar(self) -> Self {
        self.scrollbar(Scrollbar::Hidden)
    }

    /// Rupa scrollbar — tetap harus diisi dari token.
    pub fn bar_style(self, bar: ScrollbarStyle) -> Self {
        self.map(move |p| p.bar = bar)
    }

    /// Nama lain [`ScrollBuilder::bar_style`].
    pub fn bar(self, bar: ScrollbarStyle) -> Self {
        self.bar_style(bar)
    }

    /// Matikan rubber band (daftar yang harus terasa "kaku", mis. tabel data).
    pub fn no_rubber_band(self) -> Self {
        self.map(|p| p.rubber_band = false)
    }

    /// Ikut/tidak ikut navigasi Tab.
    pub fn focusable(self, focusable: bool) -> Self {
        self.map(move |p| p.focusable = focusable)
    }

    /// Nama yang dibacakan screen reader.
    pub fn label(self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.map(move |p| p.label = Some(label))
    }

    /// Cincin fokus keyboard (token `focus_ring`).
    pub fn focus_ring(self, width: f32, color: Color) -> Self {
        self.map(move |p| p.focus_ring = Some(FocusRing::new(width, color)))
    }

    /// Tanpa cincin fokus.
    pub fn no_focus_ring(self) -> Self {
        self.map(|p| p.focus_ring = None)
    }

    /// Spring yang menjalankan guliran (`smooth`/`snappy`/`bouncy`).
    pub fn spring(self, spring: Spring) -> Self {
        self.map(move |p| p.spring = spring)
    }

    /// Perlakukan gerakan guliran sebagai **esensial**: reduced-motion hanya
    /// membuang pantulannya, bukan meluncurnya.
    ///
    /// Bawaannya dekoratif, dan itu yang benar untuk hampir semua daftar.
    pub fn essential_motion(self) -> Self {
        self.map(|p| p.motion = MotionRole::Essential)
    }
}

// ---------------------------------------------------------------------------
// Operasi tingkat pohon
// ---------------------------------------------------------------------------

/// Semua [`ScrollView`] di `tree`, urut sesuai pohon (terluar dulu).
pub fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    if tree.node_ref::<ScrollView>(id).is_some() {
        out.push(id);
    }
    for anak in tree.children(id) {
        kumpulkan(tree, *anak, out);
    }
}

/// Majukan seluruh guliran satu frame — satu tempat untuk semuanya.
///
/// Artinya tepat sama dengan [`crate::overlay::advance`]:
///
/// - [`Dirty::LAYOUT`] `|` [`Dirty::PAINT`] — ada isi yang **berpindah** frame
///   ini.
/// - [`Dirty::ANIMATION`] — masih ada yang bergerak (atau hitung mundur
///   auto-hide masih berjalan), jadi frame berikutnya harus dijadwalkan.
/// - [`Dirty::NONE`] — tidak ada pekerjaan yang lahir dari modul ini, dan GPU
///   boleh tidur.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in nodes(tree) {
        let (hasil, lagi) = match tree.node_mut_ref::<ScrollView>(id) {
            Some(s) => (s.advance(tick), s.wants_frame()),
            None => continue,
        };
        if hasil.moved {
            // Guliran memindahkan anak; scroll_view adalah relayout boundary,
            // jadi pekerjaannya berhenti di subtree ini.
            tree.mark_needs_layout(id);
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if hasil.repainted {
            // Scrollbar yang memudar/melebar tidak memindahkan apa pun.
            tree.mark_needs_paint(id);
            dirty |= Dirty::PAINT;
        }
        if lagi {
            dirty |= Dirty::ANIMATION;
        }
    }
    dirty
}

/// Benar bila masih ada guliran yang bergerak.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<ScrollView>(id)
            .is_some_and(ScrollView::is_animating)
    })
}

/// Selesaikan seluruh gerakan guliran seketika (uji dan snapshot).
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(s) = tree.node_mut_ref::<ScrollView>(id) {
            s.settle();
        }
        tree.mark_needs_layout(id);
    }
}

/// **Scroll-to** sebuah wadah: animasikan guliran `id` ke `offset`.
pub fn scroll_to(tree: &mut RenderTree, id: NodeId, offset: f32) -> bool {
    let berubah = tree
        .node_mut_ref::<ScrollView>(id)
        .is_some_and(|s| s.scroll_to(offset));
    if berubah {
        tree.mark_needs_layout(id);
    }
    berubah
}

/// Wadah bergulir terdekat yang membungkus `node`.
pub fn enclosing(tree: &RenderTree, node: NodeId) -> Option<NodeId> {
    let mut cur = tree.parent(node);
    while let Some(id) = cur {
        if tree.node_ref::<ScrollView>(id).is_some() {
            return Some(id);
        }
        cur = tree.parent(id);
    }
    None
}

/// Gulirkan wadah terdekat agar `target` terlihat penuh.
///
/// Inilah bentuk `ScrollIntoView` yang dipakai dua jalur sekaligus: fokus
/// keyboard yang berpindah ke baris di luar layar, dan permintaan
/// [`AccessAction::ScrollIntoView`] dari teknologi bantu (§3.8). Keduanya harus
/// memakai perhitungan yang sama, jadi perhitungannya cuma ada satu.
pub fn scroll_into_view(tree: &mut RenderTree, target: NodeId, padding: f32) -> bool {
    let Some(sv) = enclosing(tree, target) else {
        return false;
    };
    let asal = tree.global_offset(sv);
    let anak = tree.global_offset(target);
    let ukuran = tree.size(target);
    let Some(s) = tree.node_ref::<ScrollView>(sv) else {
        return false;
    };
    // Koordinat isi = posisi yang terlihat + guliran yang sudah dilakukan.
    let (relatif, panjang) = match s.axis {
        Axis::Vertical => (anak.y - asal.y, ukuran.height),
        Axis::Horizontal => (anak.x - asal.x, ukuran.width),
    };
    let mulai = relatif + s.offset();
    let berubah = tree
        .node_mut_ref::<ScrollView>(sv)
        .is_some_and(|s| s.reveal(mulai, panjang, padding));
    if berubah {
        tree.mark_needs_layout(sv);
    }
    berubah
}

/// Layani permintaan guliran dari teknologi bantu.
///
/// Tanpa fungsi ini, [`AccessActions::SCROLL`] yang diumumkan node hanyalah
/// janji kosong: VoiceOver akan menawarkan "scroll down" dan tidak terjadi
/// apa-apa. Shell memanggilnya dari
/// `WindowConfig::on_access_action`; benar bila permintaannya benar-benar
/// dilayani.
pub fn handle_access_action(tree: &mut RenderTree, request: &AccessActionRequest) -> bool {
    let target = request.target;
    match request.action {
        AccessAction::ScrollIntoView => scroll_into_view(tree, target, 0.0),
        AccessAction::ScrollUp
        | AccessAction::ScrollDown
        | AccessAction::ScrollLeft
        | AccessAction::ScrollRight => {
            let Some(s) = tree.node_ref::<ScrollView>(target) else {
                return false;
            };
            let langkah = physics::page_step(s.extent(), s.line_height);
            let arah = match (request.action, s.axis) {
                (AccessAction::ScrollUp, Axis::Vertical)
                | (AccessAction::ScrollLeft, Axis::Horizontal) => -1.0,
                (AccessAction::ScrollDown, Axis::Vertical)
                | (AccessAction::ScrollRight, Axis::Horizontal) => 1.0,
                // Arah yang tidak sesuai sumbu ditolak, bukan ditebak.
                _ => return false,
            };
            let berubah = tree
                .node_mut_ref::<ScrollView>(target)
                .is_some_and(|s| s.scroll_by(arah * langkah));
            if berubah {
                tree.mark_needs_layout(target);
            }
            berubah
        }
        _ => false,
    }
}
