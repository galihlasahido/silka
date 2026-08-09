//! Satu tab: sorotan hover/press bertransisi **spring**, plus node AccessKit
//! ber-peran [`AccessRole::Tab`].
//!
//! Kenapa bukan [`silka_core::tree::Interactive`] yang dipakai ulang, padahal
//! `button` memakainya? Karena tiga hal yang berbeda secara kontrak, bukan
//! secara selera:
//!
//! 1. **Tab bukan tujuan Tab.** Satu deretan tab adalah **satu** perhentian
//!    keyboard (`FocusPolicy` deretan, bukan tiap tabnya) — kebiasaan
//!    `NSSegmentedControl` sekaligus pola "roving tabindex" ARIA. `Interactive`
//!    selalu focusable.
//! 2. **Tab punya keadaan terpilih**, yang harus muncul di pohon a11y sebagai
//!    [`AccessToggled`] — `Interactive` tidak punya konsep itu.
//! 3. **Transisinya spring** (`KOMPONEN.md` DoD), bukan lompatan warna seperti
//!    `Interactive` hari ini.
//!
//! Yang **tidak** dilakukan node ini: menggambar keadaan terpilih. Latar tab
//! aktif adalah indikator milik deretan ([`super::list::TabListBox`]) yang
//! bergerak dengan satu spring — kalau tiap tab menggambar latarnya sendiri,
//! yang terlihat adalah dua kotak menyala bergantian, bukan satu thumb yang
//! meluncur.

use silka_core::access::{AccessNode, AccessRole, AccessToggled};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick, Tolerance};
use silka_core::input::{
    CursorIcon, Event, EventCtx, FocusPolicy, HitBehavior, HitShape, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::ViewNode;
use silka_core::Callback;
use silka_paint::{Color, Corners, Point, Quad, Size};

/// Node render satu tab.
pub struct TabBox {
    /// Nama yang dibacakan screen reader.
    pub label: String,
    /// Posisinya di dalam deretan — argumen yang diberikan ke `on_select`.
    pub index: usize,
    /// Sedang menjadi tab aktif.
    pub selected: bool,
    /// Tidak bisa dipilih (tetap dibacakan sebagai dimmed).
    pub disabled: bool,
    /// Bentuk sudut sorotan — **sama** dengan bentuk hit-test (§3.6).
    pub corners: Corners,
    /// Sorotan hover (token `surface_hover`).
    pub hover: Color,
    /// Sorotan tekan (token `surface_pressed`).
    pub pressed_color: Color,
    /// Apa yang dijalankan saat tab ini dipilih pengguna.
    pub on_press: Option<Callback>,

    hovered: bool,
    pressed: bool,
    /// Warna sorotan yang sedang berlaku — inilah yang di-spring.
    tint: SpringValue<Color>,
    /// Benar begitu ada yang pernah memanggil [`TabBox::advance`].
    ///
    /// Lihat [`super`]: tanpa penggerak frame, transisi dijalankan sebagai
    /// lompatan alih-alih membeku di tengah jalan.
    driven: bool,
}

impl TabBox {
    /// Warna sorotan yang seharusnya berlaku untuk keadaan sekarang.
    ///
    /// Keadaan diam bukan [`Color::TRANSPARENT`] melainkan warna hover dengan
    /// alpha nol: yang memudar hanya alpha-nya, sehingga sorotan tidak pernah
    /// terlihat "menghitam dulu" di tengah transisi.
    fn target_tint(&self) -> Color {
        if self.disabled {
            return self.hover.with_alpha(0.0);
        }
        // `pressed` bertahan saat penunjuk ditangkap keluar kotak; tampilan
        // "ditekan" hanya berlaku selama penunjuknya masih di dalam (AppKit).
        if self.pressed && self.hovered {
            self.pressed_color
        } else if self.hovered {
            self.hover
        } else {
            self.hover.with_alpha(0.0)
        }
    }

    /// Arahkan sorotan ke keadaan sekarang.
    fn arahkan(&mut self) {
        let target = self.target_tint();
        if self.driven {
            self.tint.set_target(target);
        } else {
            self.tint.jump_to(target);
        }
    }

    /// Warna sorotan yang digambar frame ini.
    pub fn tint(&self) -> Color {
        self.tint.position()
    }

    /// Benar bila sorotannya masih bergerak.
    pub fn is_animating(&self) -> bool {
        self.tint.is_animating()
    }

    /// Penunjuk sedang di atas tab ini.
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// Tab ini sedang ditekan.
    pub fn is_pressed(&self) -> bool {
        self.pressed
    }

    /// Majukan sorotan satu frame; benar bila warnanya berubah.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        // Ditandai walau tidak ada yang bergerak: yang penting diketahui adalah
        // "ada penggerak frame di aplikasi ini", bukan "sedang ada animasi".
        self.driven = true;
        if !self.tint.is_animating() {
            return false;
        }
        let sebelum = self.tint.position();
        tick.advance(&mut self.tint);
        self.tint.position() != sebelum
    }

    /// Selesaikan transisi seketika (uji dan snapshot).
    pub fn settle(&mut self) {
        self.tint.settle();
    }

    /// Jalankan `on_press` — dipisah agar callback disalin keluar dulu, persis
    /// [`silka_core::tree::Interactive`]: ia hampir selalu menulis signal, dan
    /// tulisan signal tidak boleh berjalan sambil node ini dipinjam `&mut`.
    fn pilih(&mut self) {
        if self.disabled {
            return;
        }
        if let Some(cb) = self.on_press.clone() {
            cb.call();
        }
    }
}

impl RenderNode for TabBox {
    fn type_name(&self) -> &'static str {
        "Tab"
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

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let sorot = self.tint.position();
        if sorot.a > 0.0 {
            ctx.quad(
                Quad::new(ctx.local_bounds())
                    .background(sorot)
                    .corners(self.corners),
            );
        }
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Tab;
        node.label = Some(self.label.clone());
        node.disabled = self.disabled;
        // Kosakata a11y kita mengenal on/off/mixed; untuk sebuah tab itulah
        // yang dibaca screen reader sebagai "terpilih".
        node.toggled = Some(AccessToggled::from(self.selected));
        if !self.disabled {
            node.actions |= silka_core::access::AccessActions::CLICK;
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        // Tab yang dimatikan tetap menyerap penunjuk: klik di atasnya tidak
        // boleh menembus ke deretan di belakangnya dan memilih yang lain.
        HitBehavior::Opaque
    }

    /// **Satu deretan = satu perhentian Tab.** Fokusnya dipegang
    /// [`super::list::TabListBox`]; panah kiri/kanan yang memindahkan pilihan.
    fn focus_policy(&self) -> FocusPolicy {
        FocusPolicy::NONE
    }

    fn cursor(&self) -> Option<CursorIcon> {
        (!self.disabled).then_some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else {
            return;
        };
        if self.disabled {
            if matches!(p.phase, PointerPhase::Down | PointerPhase::Up) {
                ctx.handled();
            }
            return;
        }
        match p.phase {
            PointerPhase::Enter => {
                if !self.hovered {
                    self.hovered = true;
                    self.arahkan();
                    ctx.request_paint();
                    ctx.request_animation();
                }
            }
            PointerPhase::Leave => {
                if self.hovered || self.pressed {
                    self.hovered = false;
                    self.arahkan();
                    ctx.request_paint();
                    ctx.request_animation();
                }
            }
            PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                self.pressed = true;
                self.arahkan();
                ctx.capture_pointer();
                ctx.request_paint();
                ctx.request_animation();
                // **Sengaja tidak ditandai handled**: fokus harus mendarat di
                // deretan, bukan di tab (lihat `focus_policy`), dan satu-satunya
                // cara deretan mendapatkannya adalah membiarkan Down
                // menggelembung ke leluhur. Penunjuknya tetap milik tab ini —
                // capture tidak ada hubungannya dengan `handled`.
            }
            PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                let di_dalam = self.corners.contains(ctx.size(), ctx.local());
                let jadi = self.pressed && di_dalam;
                self.pressed = false;
                self.arahkan();
                ctx.release_pointer();
                ctx.request_paint();
                ctx.request_animation();
                ctx.handled();
                if jadi {
                    self.pilih();
                }
            }
            // Dibatalkan OS ≠ dilepas: tidak ada pemilihan.
            PointerPhase::Cancel if self.pressed => {
                self.pressed = false;
                self.arahkan();
                ctx.request_paint();
                ctx.request_animation();
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for TabBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TabBox")
            .field("label", &self.label)
            .field("index", &self.index)
            .field("selected", &self.selected)
            .field("disabled", &self.disabled)
            .field("tint", &self.tint.position())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props satu tab — bentuk view dari [`TabBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct TabProps {
    pub(super) label: String,
    pub(super) index: usize,
    pub(super) selected: bool,
    pub(super) disabled: bool,
    pub(super) corners: Corners,
    pub(super) hover: Color,
    pub(super) pressed: Color,
    pub(super) on_press: Option<Callback>,
    pub(super) spring: Spring,
}

impl ViewNode for TabProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let diam = self.hover.with_alpha(0.0);
        Box::new(TabBox {
            label: self.label.clone(),
            index: self.index,
            selected: self.selected,
            disabled: self.disabled,
            corners: self.corners,
            hover: self.hover,
            pressed_color: self.pressed,
            on_press: self.on_press.clone(),
            hovered: false,
            pressed: false,
            tint: SpringValue::new(diam)
                .with_spring(self.spring)
                .with_tolerance(Tolerance::COLOR)
                // Sorotan hover tidak menjelaskan apa pun — di bawah
                // reduced-motion ia hilang sepenuhnya, bukan sekadar kehilangan
                // pantulannya ([`MotionRole`]).
                .decorative(),
            driven: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TabBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.index != self.index {
            n.index = self.index;
        }
        if n.selected != self.selected {
            n.selected = self.selected;
            dirty |= Dirty::PAINT;
        }
        if n.corners != self.corners {
            n.corners = self.corners;
            dirty |= Dirty::PAINT;
        }
        if n.hover != self.hover || n.pressed_color != self.pressed {
            n.hover = self.hover;
            n.pressed_color = self.pressed;
            n.arahkan();
            dirty |= Dirty::PAINT;
        }
        if n.disabled != self.disabled {
            n.disabled = self.disabled;
            if self.disabled {
                // Tab yang baru saja dimatikan tidak boleh membeku dalam
                // keadaan ditekan: penunjuknya tidak akan datang lagi.
                n.pressed = false;
                n.hovered = false;
            }
            n.arahkan();
            dirty |= Dirty::PAINT;
        }
        if n.tint.spring() != self.spring {
            n.tint.set_spring(self.spring);
        }
        // Callback selalu diganti tanpa dibandingkan: closure dibangun ulang
        // tiap rebuild dan menangkap nilai baru (lihat `InteractiveProps`).
        n.on_press.clone_from(&self.on_press);
        dirty
    }
}

/// Peran gerakan sorotan tab terhadap reduced-motion.
///
/// Konstanta agar uji bisa menyebutnya tanpa membongkar isi node.
pub const TAB_TINT_MOTION: MotionRole = MotionRole::Decorative;
