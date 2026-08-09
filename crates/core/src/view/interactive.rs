//! View untuk [`Interactive`] — pembungkus interaktif bergaya Dart (§2.5).
//!
//! ```
//! use silka_core::view::{fixed, interactive};
//! use silka_paint::{CornerStyle, Corners};
//!
//! let _ = interactive(fixed(120.0, 44.0))
//!     .label("Simpan")
//!     // Bentuk sudut datang dari token theme; hit-test memakai yang sama.
//!     .corners(Corners::uniform(10.0, CornerStyle::squircle()))
//!     .tab_order(1);
//! ```
//!
//! State runtime node (hover, pressed, focused, jumlah aktivasi) **tidak**
//! disentuh diffing: props hanya menulis yang memang properti. Kalau tidak,
//! setiap rebuild akan menghapus keadaan tombol yang sedang ditekan jari
//! pengguna.

use silka_paint::{Color, Corners, ShadowPair};

use crate::access::AccessRole;
use crate::callback::Callback;
use crate::input::{CursorIcon, FocusPolicy};
use crate::scheduler::Dirty;
use crate::tree::{Decoration, FocusRing, Interactive, RenderNode};

use super::{Builder, View, ViewNode};

/// Props node interaktif.
#[derive(Debug, Clone, PartialEq)]
pub struct InteractiveProps {
    corners: Corners,
    focus: FocusPolicy,
    role: AccessRole,
    label: Option<String>,
    cursor: Option<CursorIcon>,
    disabled: bool,
    decoration: Decoration,
    hover_background: Option<Color>,
    press_background: Option<Color>,
    focus_ring: Option<FocusRing>,
    on_press: Option<Callback>,
}

impl Default for InteractiveProps {
    fn default() -> Self {
        let bawaan = Interactive::default();
        Self {
            corners: bawaan.corners,
            focus: bawaan.focus,
            role: bawaan.role,
            label: None,
            cursor: None,
            disabled: false,
            decoration: Decoration::NONE,
            hover_background: None,
            press_background: None,
            focus_ring: None,
            on_press: None,
        }
    }
}

impl ViewNode for InteractiveProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(Interactive {
            corners: self.corners,
            focus: self.focus,
            role: self.role,
            label: self.label.clone(),
            cursor: self.cursor,
            disabled: self.disabled,
            decoration: self.decoration,
            hover_background: self.hover_background,
            press_background: self.press_background,
            focus_ring: self.focus_ring,
            on_press: self.on_press.clone(),
            ..Interactive::default()
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<Interactive>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.corners != self.corners {
            n.corners = self.corners;
            dirty |= Dirty::PAINT;
        }
        if n.focus != self.focus {
            n.focus = self.focus;
            dirty |= Dirty::PAINT;
        }
        if n.role != self.role {
            n.role = self.role;
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.cursor != self.cursor {
            n.cursor = self.cursor;
        }
        if n.decoration != self.decoration {
            n.decoration = self.decoration;
            dirty |= Dirty::PAINT;
        }
        if n.hover_background != self.hover_background
            || n.press_background != self.press_background
        {
            n.hover_background = self.hover_background;
            n.press_background = self.press_background;
            dirty |= Dirty::PAINT;
        }
        if n.focus_ring != self.focus_ring {
            n.focus_ring = self.focus_ring;
            dirty |= Dirty::PAINT;
        }
        // Callback selalu diganti tanpa membandingkan: closure dibangun ulang
        // tiap rebuild dan **menangkap nilai baru**. Membiarkan yang lama
        // berarti tombol yang menaikkan pencacah dari angka yang sudah basi.
        // Menggantinya tidak mengubah satu piksel pun, jadi tidak ada dirty.
        n.on_press.clone_from(&self.on_press);
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            // Node yang baru saja dimatikan tidak boleh membeku dalam keadaan
            // ditekan/hover — penunjuknya tidak akan pernah datang lagi.
            if self.disabled {
                n.pressed = false;
                n.hovered = false;
            }
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

/// Bungkus `child` menjadi area yang bisa di-hover, ditekan, dan difokuskan.
pub fn interactive(child: impl Into<View>) -> Builder<InteractiveProps> {
    Builder::new(InteractiveProps::default()).child(child)
}

impl Builder<InteractiveProps> {
    /// Nama yang dibacakan screen reader (§3.8).
    pub fn label(self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.map(move |p| p.label = Some(label))
    }

    /// Peran a11y (bawaan [`AccessRole::Button`]).
    pub fn role(self, role: AccessRole) -> Self {
        self.map(move |p| p.role = role)
    }

    /// Bentuk sudut — sekaligus bentuk area sentuh (§3.6).
    pub fn corners(self, corners: Corners) -> Self {
        self.map(move |p| p.corners = corners)
    }

    /// Bisa menerima fokus keyboard atau tidak.
    pub fn focusable(self, focusable: bool) -> Self {
        self.map(move |p| p.focus.focusable = focusable)
    }

    /// Urutan tab eksplisit (mendahului urutan pohon).
    pub fn tab_order(self, order: i32) -> Self {
        self.map(move |p| {
            p.focus.focusable = true;
            p.focus.order = Some(order);
        })
    }

    /// Jadikan node ini perangkap fokus (dialog/sheet/popover).
    pub fn focus_scope(self) -> Self {
        self.map(move |p| p.focus.scope = true)
    }

    /// Bentuk kursor saat di-hover.
    pub fn cursor(self, cursor: CursorIcon) -> Self {
        self.map(move |p| p.cursor = Some(cursor))
    }

    /// Matikan interaksi (tetap dibacakan sebagai dimmed).
    pub fn disabled(self, disabled: bool) -> Self {
        self.map(move |p| p.disabled = disabled)
    }

    // -- styling utility (§2.6) ----------------------------------------------
    //
    // Nilainya **selalu** token theme yang sudah diresolusi satu tingkat di
    // atas; tidak ada satu pun angka warna yang boleh lahir di sini.

    /// Warna latar keadaan diam.
    pub fn background(self, color: Color) -> Self {
        self.map(move |p| p.decoration.background = color)
    }

    /// Warna latar saat penunjuk di atasnya (token `surface_hover`/
    /// `accent_hover`).
    pub fn hover_background(self, color: Color) -> Self {
        self.map(move |p| p.hover_background = Some(color))
    }

    /// Warna latar saat ditekan.
    pub fn press_background(self, color: Color) -> Self {
        self.map(move |p| p.press_background = Some(color))
    }

    /// Border setebal `width` berwarna `color` (token `separator`).
    pub fn border(self, width: f32, color: Color) -> Self {
        self.map(move |p| {
            p.decoration.border_width = width.max(0.0);
            p.decoration.border_color = color;
        })
    }

    /// Bayangan ganda ala HIG untuk satu tingkat elevasi.
    pub fn shadow(self, shadows: ShadowPair) -> Self {
        self.map(move |p| p.decoration.shadows = shadows)
    }

    /// Cincin fokus keyboard (token `focus_ring`) — bagian Definition of Done
    /// setiap kontrol (`KOMPONEN.md`).
    pub fn focus_ring(self, width: f32, color: Color) -> Self {
        self.map(move |p| p.focus_ring = Some(FocusRing::new(width, color)))
    }

    /// Apa yang dijalankan saat node ini diaktifkan — klik **atau** Space/Enter
    /// (§2.5).
    ///
    /// ```
    /// # use silka_core::signals::Runtime;
    /// # let rt = Runtime::new();
    /// # let count = rt.signal(0i32);
    /// use silka_core::view::{fixed, interactive};
    ///
    /// let _ = interactive(fixed(120.0, 44.0))
    ///     .label("Tambah")
    ///     .on_press(move || count.set(count.get() + 1));
    /// ```
    pub fn on_press(self, f: impl Fn() + 'static) -> Self {
        let cb = Callback::new(f);
        self.map(move |p| p.on_press = Some(cb))
    }
}
