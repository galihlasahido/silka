//! `button()` — komponen Tier 2 pertama (`KOMPONEN.md`).
//!
//! ```
//! # use silka_widgets::{button, Fonts};
//! # use silka_theme::{Appearance, Theme};
//! # use silka_core::signals::Runtime;
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! # let rt = Runtime::new();
//! # let count = rt.signal(0i32);
//! button(&fonts, &t, "Tambah").on_press(move || count.set(count.get() + 1));
//! ```
//!
//! Tombol adalah **komposisi**, bukan primitif baru: yang dirakit di sini
//! adalah wadah flex berisi [`crate::text`] di dalam sebuah node
//! ([`ButtonBox`]) yang memegang seluruh kontrak interaksi — hit-test squircle,
//! hover/press/focus, Space/Enter, emisi a11y — **plus** yang tidak dimiliki
//! [`silka_core::tree::Interactive`]: setiap perpindahan state berjalan lewat
//! **spring** (§3.5), bukan lompat.
//!
//! Empat gerakan yang dijalankan node ini, dan perannya terhadap
//! reduced-motion ([`MotionRole`]):
//!
//! | Gerakan | Spring | Peran | Alasan |
//! |---|---|---|---|
//! | Warna latar hover/press/disabled | `snappy` | Essential | Menjelaskan keadaan kontrol |
//! | Kempis saat ditekan (scale-on-press) | `snappy` | Decorative | Hiasan; reduced-motion mematikannya |
//! | Cincin fokus tumbuh | `smooth` | Essential | Menjelaskan di mana fokus keyboard |
//! | Titik "memuat" | `smooth` | Decorative | Indikator tak tentu; diam saat reduced-motion |
//!
//! Definition of Done `KOMPONEN.md` yang dipenuhi berkas ini: benar di kedua
//! preset lewat token semantik, seluruh state interaktif bertransisi spring,
//! navigasi keyboard penuh + focus ring, node AccessKit dengan peran `Button`
//! (atau `Link`) beserta aksinya, dark mode, **hit target minimal 44pt**, dan
//! reduced-motion yang dihormati.
//!
//! Siapa yang memajukan spring-nya: [`crate::advance`], satu kali per frame
//! untuk seluruh pohon — persis pola [`crate::overlay::advance`], karena "render
//! hanya saat dirty" (§3.5) baru bisa dijanjikan kalau ada **satu** pihak yang
//! tahu masih adakah yang bergerak.
//!
//! Utang teknis yang disadari dan tidak disembunyikan: "scale-on-press" digambar
//! sebagai **kempisnya kotak latar**, bukan transform sejati, karena lapisan
//! paint (§3.2) belum punya perintah transform — label di dalamnya karena itu
//! tidak ikut mengecil. Begitu perintah transform ada, spring yang sama persis
//! yang menggerakkannya; bentuk API di berkas ini tidak berubah.

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, CrossAlign, LayoutCtx, MainAlign, PaintCtx, RenderNode};
use silka_core::view::{constrained, row, Builder, View, ViewNode};
use silka_core::Callback;
use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Rect, ShadowPair, Size};
use silka_text::FontWeight;
use silka_theme::{Appearance, Theme};

use crate::fonts::Fonts;
use crate::text::text;

/// Ukuran minimum area sentuh sebuah kontrol, poin logis (Apple HIG).
pub const MIN_HIT_TARGET: f32 = 44.0;

/// Jumlah titik indikator "memuat".
const JUMLAH_TITIK: usize = 3;

// ---------------------------------------------------------------------------
// Varian
// ---------------------------------------------------------------------------

/// Varian visual tombol (`KOMPONEN.md`: primary/secondary/ghost/destructive/
/// link).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ButtonVariant {
    /// Aksi utama: latar `accent`, teks `on_accent`.
    #[default]
    Primary,
    /// Aksi pendamping: latar `surface`, teks `label`, border `border`.
    Secondary,
    /// Tanpa latar sampai di-hover — toolbar, baris daftar.
    Ghost,
    /// Aksi merusak: latar `destructive`.
    Destructive,
    /// Terlihat seperti tautan: teks `accent`, tanpa latar.
    Link,
}

impl ButtonVariant {
    /// Semua varian, urut — dipakai gallery dan uji lintas-varian.
    pub const ALL: [ButtonVariant; 5] = [
        ButtonVariant::Primary,
        ButtonVariant::Secondary,
        ButtonVariant::Ghost,
        ButtonVariant::Destructive,
        ButtonVariant::Link,
    ];

    /// Nama pendek untuk gallery, log, dan dump uji.
    pub const fn name(self) -> &'static str {
        match self {
            ButtonVariant::Primary => "primary",
            ButtonVariant::Secondary => "secondary",
            ButtonVariant::Ghost => "ghost",
            ButtonVariant::Destructive => "destructive",
            ButtonVariant::Link => "link",
        }
    }

    /// Peran a11y varian ini — `Link` dibacakan sebagai tautan, sisanya tombol.
    pub const fn role(self) -> AccessRole {
        match self {
            ButtonVariant::Link => AccessRole::Link,
            _ => AccessRole::Button,
        }
    }

    /// Warna teks varian ini pada keadaan tertentu.
    ///
    /// Warna teks **tidak** dianimasikan: ia milik node teks di dalam tombol,
    /// dan node itu hanya berubah lewat diff. Yang bergerak adalah latarnya —
    /// dan justru itulah yang dilakukan macOS/iOS.
    pub fn foreground(self, theme: &Theme, state: ButtonState) -> Color {
        if state.disabled {
            return theme.color.disabled_label;
        }
        if state.loading {
            // Label disembunyikan tapi **tetap diukur**: tombol tidak boleh
            // berubah lebar saat mulai memuat.
            return Color::TRANSPARENT;
        }
        self.content_color(theme)
    }

    /// Warna isi (teks/titik) varian ini saat aktif.
    fn content_color(self, theme: &Theme) -> Color {
        match self {
            ButtonVariant::Primary => theme.color.on_accent,
            ButtonVariant::Secondary | ButtonVariant::Ghost => theme.color.label,
            ButtonVariant::Destructive => theme.color.on_destructive,
            ButtonVariant::Link => theme.color.accent,
        }
    }

    /// Seluruh nilai gambar varian ini, sudah diresolusi dari token.
    pub fn style(self, theme: &Theme, state: ButtonState) -> ButtonStyle {
        let (rest, hover, pressed) = match self {
            ButtonVariant::Primary => (
                theme.color.accent,
                theme.color.accent_hover,
                theme.color.accent_pressed,
            ),
            ButtonVariant::Secondary => (
                theme.color.surface,
                theme.color.surface_hover,
                theme.color.surface_pressed,
            ),
            ButtonVariant::Ghost => (
                // Ghost tidak menggambar apa pun sampai disentuh.
                theme.color.surface_hover.with_alpha(0.0),
                theme.color.surface_hover,
                theme.color.surface_pressed,
            ),
            ButtonVariant::Destructive => (
                theme.color.destructive,
                theme.color.destructive_hover,
                dorong(theme.color.destructive_hover, theme, 0.08),
            ),
            ButtonVariant::Link => (
                theme.color.accent_muted.with_alpha(0.0),
                theme.color.accent_muted,
                dorong(theme.color.accent_muted, theme, 0.08),
            ),
        };

        let border_width = match self {
            ButtonVariant::Secondary => theme.space(0.25),
            _ => 0.0,
        };
        let shadows = match self {
            ButtonVariant::Primary | ButtonVariant::Secondary | ButtonVariant::Destructive => {
                theme.shadow.sm
            }
            ButtonVariant::Ghost | ButtonVariant::Link => ShadowPair::NONE,
        };

        ButtonStyle {
            rest,
            hover,
            pressed,
            // Kontrol yang mati **meredup ke arah latar halaman** — aturan yang
            // sama yang dipakai macOS, dan nilainya tetap turunan token.
            disabled: rest.lerp(theme.color.background, 0.6),
            corners: theme.corners(theme.radius.md),
            border_width,
            border: theme.color.border,
            border_disabled: theme.color.separator,
            shadows,
            focus_ring_width: theme.space(0.5),
            focus_ring: theme.color.focus_ring,
            press_travel: theme.space(0.25),
            dot: self.content_color(theme),
            dot_size: theme.space(1.5),
            dot_gap: theme.space(1.0),
            state,
        }
    }
}

/// Geser sebuah warna ke arah "lebih ditekan" sebanyak `t`.
///
/// Di appearance terang artinya lebih gelap, di gelap lebih terang — aturan
/// yang sama yang dipakai macOS. Dipakai hanya di tempat token tidak
/// menyediakan langkah berikutnya (mis. `destructive_pressed` yang memang tidak
/// ada), jadi nilainya tetap **turunan** token, bukan angka warna baru.
fn dorong(color: Color, theme: &Theme, jumlah: f32) -> Color {
    let arah = if theme.appearance == Appearance::Dark {
        Color::WHITE
    } else {
        Color::BLACK
    };
    color.lerp(arah, jumlah.clamp(0.0, 1.0))
}

// ---------------------------------------------------------------------------
// State & style
// ---------------------------------------------------------------------------

/// Keadaan tombol yang **datang dari aplikasi** (bukan dari penunjuk).
///
/// Dipisah dari state runtime (hover/press/focus) karena keduanya hidup di
/// tempat berbeda: yang ini milik props dan berubah lewat diff, yang itu milik
/// node dan tidak boleh tersapu rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ButtonState {
    /// Tidak bisa dipakai — tetap dibacakan screen reader sebagai dimmed.
    pub disabled: bool,
    /// Sedang memproses: label disembunyikan, titik indikator berdenyut.
    pub loading: bool,
}

impl ButtonState {
    /// Benar bila tombol menerima aktivasi sama sekali.
    pub fn is_enabled(self) -> bool {
        !self.disabled && !self.loading
    }
}

/// Seluruh nilai gambar sebuah tombol, **sudah diresolusi** dari token theme.
///
/// Mesin tidak pernah punya pendapat tentang warna (§2.6, §2.7): preset
/// Cupertino dan Tailwind berganti dengan mengisi struct ini, tanpa satu baris
/// pun berubah di [`ButtonBox`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonStyle {
    /// Latar keadaan diam.
    pub rest: Color,
    /// Latar saat penunjuk di atasnya.
    pub hover: Color,
    /// Latar saat ditekan.
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
    /// Seberapa jauh latar mengempis saat ditekan, poin logis.
    pub press_travel: f32,
    /// Warna titik indikator "memuat".
    pub dot: Color,
    /// Diameter satu titik.
    pub dot_size: f32,
    /// Jarak antar titik.
    pub dot_gap: f32,
    /// Keadaan yang datang dari aplikasi.
    pub state: ButtonState,
}

impl ButtonStyle {
    /// Latar yang seharusnya berlaku untuk kombinasi state ini.
    ///
    /// Inilah **target** spring; yang digambar adalah posisi spring-nya, bukan
    /// nilai ini.
    pub fn background_for(&self, hovered: bool, pressed: bool) -> Color {
        if !self.state.is_enabled() {
            return self.disabled;
        }
        // `pressed` bertahan saat penunjuk ditangkap keluar kotak, tapi tampilan
        // "ditekan" hanya berlaku selama penunjuknya masih di dalam — persis
        // AppKit/UIKit.
        if pressed && hovered {
            self.pressed
        } else if hovered {
            self.hover
        } else {
            self.rest
        }
    }

    /// Warna border yang berlaku.
    pub fn border_for(&self) -> Color {
        if self.state.disabled {
            self.border_disabled
        } else {
            self.border
        }
    }
}

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// Node render sebuah tombol: kontrak input penuh + empat spring.
#[derive(Debug)]
pub struct ButtonBox {
    style: ButtonStyle,
    label: Option<String>,
    role: AccessRole,
    focus: FocusPolicy,
    on_press: Option<Callback>,

    /// Latar yang benar-benar digambar frame ini.
    bg: SpringValue<Color>,
    /// 0 = lepas, 1 = kempis penuh (scale-on-press).
    press_t: SpringValue<f32>,
    /// 0 = tanpa cincin fokus, 1 = cincin penuh.
    ring_t: SpringValue<f32>,
    /// Fase denyut titik "memuat" (ping-pong 0↔1).
    pulse: SpringValue<f32>,

    hovered: bool,
    pressed: bool,
    focused: bool,
    /// Jumlah aktivasi (klik atau Space/Enter) sejak node dibuat.
    activations: u32,
}

impl ButtonBox {
    /// Node baru yang **sudah berada** di keadaan diamnya — tombol tidak
    /// beranimasi masuk saat halaman pertama kali tampil.
    fn new(style: ButtonStyle, label: Option<String>, role: AccessRole, spring: Spring) -> Self {
        Self {
            bg: SpringValue::new(style.background_for(false, false)).with_spring(spring),
            press_t: SpringValue::new(0.0).with_spring(spring).decorative(),
            ring_t: SpringValue::new(0.0).with_spring(Spring::smooth()),
            pulse: SpringValue::new(0.0)
                .with_spring(Spring::smooth())
                .decorative(),
            style,
            label,
            role,
            focus: FocusPolicy::FOCUSABLE,
            on_press: None,
            hovered: false,
            pressed: false,
            focused: false,
            activations: 0,
        }
    }

    /// Keadaan yang datang dari aplikasi.
    pub fn state(&self) -> ButtonState {
        self.style.state
    }

    /// Nilai gambar yang sedang berlaku.
    pub fn style(&self) -> ButtonStyle {
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
            || self.press_t.is_animating()
            || self.ring_t.is_animating()
            || (self.style.state.loading && self.pulse.is_animating())
    }

    /// Arahkan seluruh spring ke keadaan sekarang.
    ///
    /// **Retarget, bukan animasi baru** (§3.5): tombol yang dilepas di tengah
    /// animasi tekan berbalik arah membawa kecepatannya.
    fn retarget(&mut self) {
        let enabled = self.style.state.is_enabled();
        self.bg
            .set_target(self.style.background_for(self.hovered, self.pressed));
        self.press_t
            .set_target(if self.pressed && self.hovered && enabled {
                1.0
            } else {
                0.0
            });
        self.ring_t
            .set_target(if self.focused && !self.style.state.disabled {
                1.0
            } else {
                0.0
            });
        if !self.style.state.loading {
            self.pulse.jump_to(0.0);
        }
    }

    /// Majukan seluruh spring satu frame; benar bila ada yang bergeser.
    ///
    /// Dipanggil [`crate::advance`], satu tempat untuk seluruh pohon.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let mut bergeser = false;

        // -- gerakan yang **menjelaskan**: tetap berjalan di reduced-motion,
        //    hanya kehilangan pantulannya (`Motion::spring`).
        let bg0 = self.bg.position();
        tick.advance(&mut self.bg);
        bergeser |= self.bg.position() != bg0;

        let r0 = self.ring_t.position();
        tick.advance(&mut self.ring_t);
        bergeser |= self.ring_t.position() != r0;

        // -- gerakan **hiasan**: hilang sepenuhnya di reduced-motion.
        //
        // "Hilang" di sini berarti benar-benar tidak terjadi, bukan terjadi
        // seketika: tombol yang berkedip mengempis dalam satu frame justru
        // lebih mengganggu daripada tombol yang diam.
        if tick.motion().suppresses(MotionRole::Decorative) {
            bergeser |= self.press_t.position() != 0.0 || self.pulse.position() != 0.0;
            self.press_t.jump_to(0.0);
            self.pulse.jump_to(0.0);
            return bergeser;
        }

        // Target dihitung ulang tiap frame supaya keadaan tetap benar sekalipun
        // pengguna baru saja mematikan reduced-motion di tengah tekanan.
        self.press_t.set_target(
            if self.pressed && self.hovered && self.style.state.is_enabled() {
                1.0
            } else {
                0.0
            },
        );
        let p0 = self.press_t.position();
        tick.advance(&mut self.press_t);
        bergeser |= self.press_t.position() != p0;

        // Indikator tak tentu: denyutnya membalik arah setiap kali sampai, dan
        // ia **satu-satunya** sumber gerakan yang tidak berhenti sendiri — jadi
        // ia juga satu-satunya yang harus menahan frame tetap datang
        // ([`Tick::keep_awake`]).
        if self.style.state.loading {
            if !self.pulse.is_animating() {
                let balik = if self.pulse.target() >= 0.5 { 0.0 } else { 1.0 };
                self.pulse.set_target(balik);
            }
            let d0 = self.pulse.position();
            tick.advance(&mut self.pulse);
            bergeser |= self.pulse.position() != d0;
            tick.keep_awake();
        }

        bergeser
    }

    /// Selesaikan seluruh gerakan seketika (uji, snapshot, reduced-motion).
    pub fn settle(&mut self) {
        self.bg.settle();
        self.press_t.settle();
        self.ring_t.settle();
        self.pulse.settle();
    }

    /// Catat satu aktivasi lalu jalankan `on_press`.
    ///
    /// Callback-nya **disalin keluar dulu**: ia hampir selalu menulis signal,
    /// dan tulisan signal boleh memicu apa saja di runtime — yang tidak boleh
    /// terjadi adalah ia berjalan sambil node ini masih dipinjam `&mut`.
    fn aktifkan(&mut self) {
        if !self.style.state.is_enabled() {
            return;
        }
        self.activations = self.activations.saturating_add(1);
        if let Some(cb) = self.on_press.clone() {
            cb.call();
        }
    }

    /// Kotak latar frame ini: mengempis mengikuti spring tekanan.
    fn kotak_latar(&self, bounds: Rect) -> (Rect, Corners) {
        let kempis = (self.press_t.position() * self.style.press_travel)
            .clamp(0.0, bounds.size.min_side() * 0.25);
        let kotak = bounds.deflate(Insets::all(kempis));
        let radii = (self.style.corners.radii.max() - kempis).max(0.0);
        (
            kotak,
            Corners::new(CornerRadii::all(radii), self.style.corners.style),
        )
    }

    /// Kotak ketiga titik indikator "memuat", koordinat lokal.
    fn titik(&self, bounds: Rect) -> [Rect; JUMLAH_TITIK] {
        let d = self.style.dot_size.max(1.0);
        let gap = self.style.dot_gap.max(0.0);
        let total = d * JUMLAH_TITIK as f32 + gap * (JUMLAH_TITIK as f32 - 1.0);
        let tengah = bounds.center();
        let x0 = tengah.x - total / 2.0;
        let y = tengah.y - d / 2.0;
        core::array::from_fn(|i| Rect::new(x0 + i as f32 * (d + gap), y, d, d))
    }
}

/// Opasitas satu titik indikator pada fase tertentu.
///
/// Fungsi murni dan karena itu bisa diuji tanpa GPU: gelombang segitiga dengan
/// beda fase antar titik, dijepit supaya tidak pernah benar-benar hilang (titik
/// yang berkedip sampai nol terbaca sebagai kedip, bukan sebagai denyut).
pub fn dot_opacity(phase: f32, index: usize) -> f32 {
    let t = (phase + index as f32 * 0.25).rem_euclid(1.0);
    let segitiga = 1.0 - (2.0 * t - 1.0).abs();
    0.35 + 0.65 * segitiga
}

impl RenderNode for ButtonBox {
    fn type_name(&self) -> &'static str {
        "Button"
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

    /// Latar (kempis mengikuti spring), lalu cincin fokus, lalu isinya, lalu
    /// indikator memuat di paling atas.
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let (kotak, corners) = self.kotak_latar(bounds);
        let bg = self.bg.position();
        let border = self.style.border_for();
        let ada_border = self.style.border_width > 0.0 && border.a > 0.0;
        if bg.a > 0.0 || ada_border || self.style.shadows.is_visible() {
            let quad = Quad::new(kotak)
                .background(bg)
                .corners(corners)
                .border(self.style.border_width, border);
            // Bayangan ikut mengempis bersama tombolnya karena ia dihitung dari
            // kotak yang sama — tidak ada geometri kedua yang bisa melenceng.
            ctx.shadowed(quad, self.style.shadows);
        }

        // Cincin fokus digambar **di luar** kotak node supaya tidak menutupi
        // label (kebiasaan AppKit), dan tumbuh lewat spring.
        let ring = self.ring_t.position().clamp(0.0, 1.0);
        if ring > 0.0 && self.style.focus_ring_width > 0.0 && self.style.focus_ring.a > 0.0 {
            let tebal = self.style.focus_ring_width * ring;
            if tebal > 0.0 {
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
        }

        ctx.paint_children();

        if self.style.state.loading {
            let fase = self.pulse.position();
            let bentuk = Corners::uniform(self.style.dot_size / 2.0, self.style.corners.style);
            for (i, kotak) in self.titik(bounds).into_iter().enumerate() {
                let alpha = self.style.dot.a * dot_opacity(fase, i);
                ctx.quad(
                    Quad::new(kotak)
                        .background(self.style.dot.with_alpha(alpha))
                        .corners(bentuk),
                );
            }
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.role;
        node.label.clone_from(&self.label);
        // Tombol yang sedang memuat **tidak bisa** ditekan; bagi teknologi bantu
        // itu berarti dimmed. (Kosakata `busy` belum ada di `AccessNode` —
        // utang yang disadari, bukan yang tersembunyi.)
        node.disabled = !self.style.state.is_enabled();
        if self.style.state.is_enabled() {
            node.actions |= AccessActions::CLICK;
            if self.focus.focusable {
                node.actions |= AccessActions::FOCUS;
            }
        }
    }

    fn hit_shape(&self) -> HitShape {
        // Bentuk sentuh = bentuk gambar **saat diam**: tombol yang mengempis
        // tidak boleh kehilangan area sentuhnya di tengah tekanan jari.
        HitShape::Rounded(self.style.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Tombol mati tetap **menyerap** penunjuk: kliknya tidak boleh menembus
        // ke konten di belakangnya.
        HitBehavior::Opaque
    }

    fn focus_policy(&self) -> FocusPolicy {
        if self.style.state.disabled {
            FocusPolicy::NONE
        } else {
            // Tombol yang sedang memuat tetap boleh dituju keyboard — fokus
            // tidak boleh melompat pergi hanya karena aplikasi sedang sibuk.
            self.focus
        }
    }

    fn cursor(&self) -> Option<CursorIcon> {
        if self.style.state.is_enabled() {
            Some(CursorIcon::Pointer)
        } else {
            None
        }
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if self.style.state.disabled {
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
                PointerPhase::Leave => {
                    // Sengaja tidak membatalkan `pressed`: penunjuk yang
                    // ditangkap boleh keluar-masuk selama tombol ditahan.
                    self.hovered = false;
                }
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
                        // Retarget dulu, baru callback: `on_press` boleh menulis
                        // signal yang membangun ulang tombol ini.
                        self.retarget();
                        self.aktifkan();
                    }
                }
                // Dibatalkan OS ≠ dilepas: tidak ada aktivasi.
                PointerPhase::Cancel if self.pressed => self.pressed = false,
                _ => {}
            },

            Event::Key(k) if k.is_pressed() => {
                let aktivasi = matches!(
                    k.code,
                    KeyCode::Named(NamedKey::Space) | KeyCode::Named(NamedKey::Enter)
                );
                if aktivasi && k.modifiers.is_empty() {
                    ctx.handled();
                    self.aktifkan();
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

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props tombol — bentuk view dari [`ButtonBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct ButtonProps {
    style: ButtonStyle,
    label: Option<String>,
    role: AccessRole,
    focus: FocusPolicy,
    spring: Spring,
    on_press: Option<Callback>,
}

impl ViewNode for ButtonProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut node = ButtonBox::new(self.style, self.label.clone(), self.role, self.spring);
        node.focus = self.focus;
        node.on_press.clone_from(&self.on_press);
        Box::new(node)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ButtonBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.style != self.style {
            let disabled_baru = self.style.state.disabled && !n.style.state.disabled;
            n.style = self.style;
            if disabled_baru {
                // Kontrol yang baru saja dimatikan tidak boleh membeku dalam
                // keadaan ditekan/hover — penunjuknya tidak akan datang lagi.
                n.pressed = false;
                n.hovered = false;
            }
            // Warna baru **dituju**, bukan dilompati: mengganti theme atau
            // menyalakan `loading` pun berjalan lewat spring.
            n.retarget();
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.role != self.role {
            n.role = self.role;
            dirty |= Dirty::PAINT;
        }
        if n.focus != self.focus {
            n.focus = self.focus;
            dirty |= Dirty::PAINT;
        }
        if n.bg.spring() != self.spring {
            // Ganti preset spring tanpa mengganggu gerakan yang sedang berjalan.
            n.bg.set_spring(self.spring);
            n.press_t.set_spring(self.spring);
        }
        // Callback selalu diganti tanpa dibandingkan: closure dibangun ulang
        // tiap rebuild dan **menangkap nilai baru**. Membiarkan yang lama
        // berarti tombol yang bekerja dari angka basi.
        n.on_press.clone_from(&self.on_press);
        dirty
    }
}

/// Builder tombol bergaya Dart (§2.5).
///
/// Menyimpan bahan mentahnya (theme, label, varian, state) dan baru
/// **meresolusi token** saat menjadi [`View`] — dengan begitu `.variant(…)` yang
/// dipanggil belakangan tetap mengubah seluruh paletnya.
#[derive(Debug, Clone)]
pub struct Button {
    fonts: Fonts,
    theme: Theme,
    label: String,
    variant: ButtonVariant,
    state: ButtonState,
    spring: Spring,
    focus: FocusPolicy,
    on_press: Option<Callback>,
    key: Option<Key>,
}

/// Tombol berlabel teks — komponen `button` (`KOMPONEN.md` Tier 2).
///
/// `fonts` adalah mesin teks aplikasi, `theme` sumber seluruh nilainya.
pub fn button(fonts: &Fonts, theme: &Theme, label: impl Into<String>) -> Button {
    button_variant(fonts, theme, label, ButtonVariant::default())
}

/// [`button`] dengan varian eksplisit.
pub fn button_variant(
    fonts: &Fonts,
    theme: &Theme,
    label: impl Into<String>,
    variant: ButtonVariant,
) -> Button {
    Button {
        fonts: fonts.clone(),
        theme: *theme,
        label: label.into(),
        variant,
        state: ButtonState::default(),
        // `snappy` adalah rasa kontrol macOS: cepat sampai, nyaris tanpa
        // pantulan (WWDC23).
        spring: Spring::snappy(),
        focus: FocusPolicy::FOCUSABLE,
        on_press: None,
        key: None,
    }
}

impl Button {
    /// Varian visual.
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Apa yang dijalankan saat tombol diaktifkan — klik **atau** Space/Enter.
    pub fn on_press(mut self, f: impl Fn() + 'static) -> Self {
        self.on_press = Some(Callback::new(f));
        self
    }

    /// Matikan tombol (tetap dibacakan screen reader sebagai dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.state.disabled = disabled;
        self
    }

    /// Tandai sedang memproses: label disembunyikan tanpa mengubah lebar, titik
    /// indikator berdenyut, dan aktivasi ditolak.
    pub fn loading(mut self, loading: bool) -> Self {
        self.state.loading = loading;
        self
    }

    /// Spring yang menjalankan transisi state (`smooth`/`snappy`/`bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
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

    /// Kunci identitas di antara saudara-saudaranya (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Nilai gambar yang akan dipakai — dipakai gallery dan uji token.
    pub fn style(&self) -> ButtonStyle {
        self.variant.style(&self.theme, self.state)
    }
}

impl From<Button> for View {
    fn from(b: Button) -> View {
        let t = b.theme;
        let style = b.variant.style(&t, b.state);
        let warna_teks = b.variant.foreground(&t, b.state);

        let isi = row([text(&b.fonts, &b.label)
            .size(t.typography.body_size)
            .weight(FontWeight::MEDIUM)
            .color(warna_teks)
            .single_line()
            // Nama tombol dibacakan sekali, dari node tombolnya — bukan dua kali.
            .role(AccessRole::Container)])
        .main(MainAlign::Center)
        .cross(CrossAlign::Center)
        .padding(Insets::symmetric(t.space(4.0), t.space(2.0)));

        // Hit target ≥ 44pt di kedua sumbu walau visualnya lebih kecil (HIG);
        // teksnya tetap di tengah karena wadah flex di dalamnya yang meratakan,
        // bukan aritmetika.
        let kotak = constrained(
            BoxConstraints::new(MIN_HIT_TARGET, f32::INFINITY, MIN_HIT_TARGET, f32::INFINITY),
            isi,
        );

        let mut builder = Builder::new(ButtonProps {
            style,
            label: Some(b.label),
            role: b.variant.role(),
            focus: b.focus,
            spring: b.spring,
            on_press: b.on_press,
        })
        .child(kotak);
        if let Some(key) = b.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

#[cfg(test)]
mod tests;
