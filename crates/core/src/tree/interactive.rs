//! [`Interactive`] — node yang **memakai seluruh kontrak input**.
//!
//! Ia bukan widget: `Button`, `Checkbox`, dan kawan-kawan tinggal membungkusnya
//! nanti dengan token theme dan spring. Yang ia lakukan adalah menutup satu
//! putaran penuh — hit-test squircle, hover, tekan, capture, fokus, aktivasi
//! keyboard, emisi a11y — sehingga ada satu tempat konkret yang membuktikan
//! kontraknya bisa dipenuhi, dan satu contoh yang bisa ditiru penulis widget.
//!
//! Aturan HIG yang sudah tertanam di sini:
//!
//! - **Space dan Enter mengaktifkan** apa pun yang bisa diklik, sehingga
//!   keyboard tidak pernah menjadi warga kelas dua (`KOMPONEN.md` DoD).
//! - **Tekan lalu tarik keluar = batal.** Selama tombol ditahan penunjuk
//!   ditangkap, dan pelepasan di luar bentuk node tidak menghasilkan klik —
//!   perilaku yang sama dengan AppKit dan UIKit.
//! - **Bentuk sentuh = bentuk gambar.** [`Interactive::corners`] mengalir ke
//!   [`RenderNode::hit_shape`] **dan** ke [`Decoration::corners`] saat
//!   menggambar, jadi squircle Cupertino diuji sebagai squircle dan tidak
//!   mungkin ada pojok yang terlihat kosong tapi bisa diklik.
//! - **Warna per state datang dari token, bukan dari sini.**
//!   [`Interactive::decoration`], [`Interactive::hover_background`], dan
//!   [`Interactive::press_background`] adalah nilai yang **sudah diresolusi**
//!   satu tingkat di atas (§2.6, §2.7) — mesin tidak punya pendapat tentang
//!   warna, jadi preset Cupertino/Tailwind berganti tanpa satu baris pun
//!   berubah di berkas ini.

use silka_paint::{Color, CornerRadii, Corners, Insets, Point, Quad, Size};

use crate::access::{AccessActions, AccessNode, AccessRole};
use crate::callback::Callback;
use crate::input::{
    CursorIcon, Event, EventCtx, FocusEvent, FocusPolicy, HitBehavior, HitShape, KeyCode, NamedKey,
    PointerButton, PointerPhase,
};

use super::arena::{LayoutCtx, RenderNode};
use super::constraints::BoxConstraints;
use super::paint::{Decoration, PaintCtx};

/// Cincin fokus keyboard: tebal dan warna, keduanya dari token theme.
///
/// Digambar **di luar** kotak node supaya tidak menutupi isinya — kebiasaan
/// AppKit, dan syarat agar tombol kecil tetap terbaca saat difokuskan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FocusRing {
    /// Tebal cincin, poin logis.
    pub width: f32,
    /// Warna cincin — token `focus_ring`.
    pub color: Color,
}

impl FocusRing {
    /// Cincin setebal `width` berwarna `color`.
    pub fn new(width: f32, color: Color) -> Self {
        Self {
            width: width.max(0.0),
            color,
        }
    }
}

/// Node interaktif serba guna: bisa di-hover, ditekan, difokuskan, dan
/// diaktifkan dari keyboard.
#[derive(Debug, Clone, PartialEq)]
pub struct Interactive {
    /// Bentuk sudut — **sama** dengan yang digambar (§3.6).
    pub corners: Corners,
    /// Peran fokus keyboard.
    pub focus: FocusPolicy,
    /// Peran a11y.
    pub role: AccessRole,
    /// Nama yang dibacakan screen reader.
    pub label: Option<String>,
    /// Bentuk kursor saat di-hover.
    pub cursor: Option<CursorIcon>,
    /// Tidak bisa dipakai: tidak menerima event, tetap dibacakan sebagai dimmed.
    pub disabled: bool,

    /// Latar keadaan diam — nilainya sudah diresolusi dari token theme.
    pub decoration: Decoration,
    /// Latar saat penunjuk berada di atasnya (`None` = tidak berubah).
    pub hover_background: Option<Color>,
    /// Latar saat sedang ditekan (`None` = pakai yang hover/diam).
    pub press_background: Option<Color>,
    /// Cincin fokus keyboard (`None` = tidak digambar).
    pub focus_ring: Option<FocusRing>,
    /// Apa yang dijalankan setiap kali node ini diaktifkan (klik atau
    /// Space/Enter) — inilah `on_press` gaya Dart (§2.5).
    pub on_press: Option<Callback>,

    /// Penunjuk sedang berada di atasnya.
    pub hovered: bool,
    /// Tombol sedang ditekan **dan** penunjuk masih di dalam bentuknya.
    pub pressed: bool,
    /// Sedang memegang fokus keyboard.
    pub focused: bool,
    /// Jumlah aktivasi (klik atau Space/Enter) sejak node dibuat.
    pub activations: u32,
}

impl Default for Interactive {
    fn default() -> Self {
        Self {
            corners: Corners::SHARP,
            focus: FocusPolicy::FOCUSABLE,
            role: AccessRole::Button,
            label: None,
            cursor: None,
            disabled: false,
            decoration: Decoration::NONE,
            hover_background: None,
            press_background: None,
            focus_ring: None,
            on_press: None,
            hovered: false,
            pressed: false,
            focused: false,
            activations: 0,
        }
    }
}

impl Interactive {
    /// Node interaktif dengan nilai bawaan (tombol, sudut tajam).
    pub fn new() -> Self {
        Self::default()
    }

    /// Benar bila node sedang menerima event sama sekali.
    fn aktif(&self) -> bool {
        !self.disabled
    }

    /// Catat satu aktivasi lalu jalankan `on_press`.
    ///
    /// Callback-nya **disalin keluar dulu**: ia hampir selalu menulis signal,
    /// dan tulisan signal boleh memicu apa saja di runtime — yang tidak boleh
    /// terjadi adalah ia berjalan sambil node ini masih dipinjam `&mut`.
    fn aktifkan(&mut self) {
        self.activations = self.activations.saturating_add(1);
        if let Some(cb) = self.on_press.clone() {
            cb.call();
        }
    }

    /// Latar yang berlaku untuk keadaan node saat ini.
    ///
    /// Bentuk sudutnya **selalu** [`Interactive::corners`] — sumber yang sama
    /// dengan hit-testing (§3.6), sehingga keduanya tidak mungkin berbeda.
    pub fn dekorasi_aktif(&self) -> Decoration {
        let mut d = self.decoration;
        d.corners = self.corners;
        if self.disabled {
            return d;
        }
        // `pressed` bertahan saat penunjuk ditangkap keluar kotak (lihat
        // `PointerPhase::Leave`), tapi tampilan "ditekan" hanya berlaku selama
        // penunjuknya masih di dalam — persis AppKit/UIKit.
        if self.pressed && self.hovered {
            if let Some(c) = self.press_background.or(self.hover_background) {
                d.background = c;
            }
        } else if self.hovered {
            if let Some(c) = self.hover_background {
                d.background = c;
            }
        }
        d
    }
}

impl RenderNode for Interactive {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(size)
    }

    /// Latar sesuai keadaan, lalu cincin fokus, lalu isinya.
    ///
    /// Urutannya menentukan hasil: cincin fokus digambar **di bawah** isi tapi
    /// **di luar** kotak node, sehingga label tetap terbaca penuh.
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.dekorasi_aktif());
        if self.focused && !self.disabled {
            if let Some(ring) = self.focus_ring.filter(|r| r.width > 0.0 && r.color.a > 0.0) {
                // `deflate` dengan inset negatif = mengembang; radiusnya ikut
                // tumbuh supaya cincin sejajar dengan tepi yang dibulatkan.
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
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.role;
        node.label.clone_from(&self.label);
        node.disabled = self.disabled;
        if self.aktif() {
            node.actions |= AccessActions::CLICK;
            if self.focus.focusable {
                node.actions |= AccessActions::FOCUS;
            }
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Node yang tidak bisa dipakai tetap **menyerap** penunjuk: klik pada
        // tombol disabled tidak boleh menembus ke konten di belakangnya.
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
        self.cursor.filter(|_| self.aktif())
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if !self.aktif() {
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
                        ctx.request_paint();
                    }
                }
                PointerPhase::Leave => {
                    if self.hovered || self.pressed {
                        self.hovered = false;
                        // Sengaja tidak membatalkan `pressed`: penunjuk yang
                        // ditangkap boleh keluar-masuk selama tombol ditahan.
                        ctx.request_paint();
                    }
                }
                PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                    self.pressed = true;
                    ctx.capture_pointer();
                    ctx.request_focus();
                    ctx.request_paint();
                    ctx.handled();
                }
                PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                    let di_dalam = self.corners.contains(ctx.size(), ctx.local());
                    if self.pressed && di_dalam {
                        self.aktifkan();
                    }
                    self.pressed = false;
                    ctx.release_pointer();
                    ctx.request_paint();
                    ctx.handled();
                }
                // Dibatalkan OS ≠ dilepas: tidak ada aktivasi.
                PointerPhase::Cancel if self.pressed => {
                    self.pressed = false;
                    ctx.request_paint();
                }
                _ => {}
            },

            Event::Key(k) if k.is_pressed() => {
                let aktivasi = matches!(
                    k.code,
                    KeyCode::Named(NamedKey::Space) | KeyCode::Named(NamedKey::Enter)
                );
                if aktivasi && k.modifiers.is_empty() {
                    self.aktifkan();
                    ctx.request_paint();
                    ctx.handled();
                }
            }

            Event::Focus(f) => {
                self.focused = *f == FocusEvent::Gained;
                if !self.focused {
                    self.pressed = false;
                }
                ctx.request_paint();
            }

            _ => {}
        }
    }
}
