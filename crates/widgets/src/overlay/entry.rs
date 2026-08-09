//! Satu overlay: panel + backdrop + dismiss + transisi spring.
//!
//! [`OverlayEntry`] adalah node render yang **memenuhi seluruh layer**, dengan
//! panelnya sebagai satu-satunya anak yang ditempatkan lewat [`super::place`].
//! Bentuk itu dipilih dengan sengaja, karena ia menyelesaikan tiga hal
//! sekaligus dengan satu node:
//!
//! 1. **Backdrop** tinggal sebuah quad seukuran node ini (token `scrim`).
//! 2. **Klik di luar** = klik yang mendarat di node ini tapi di luar kotak
//!    panel — tidak perlu menebak-nebak koordinat global.
//! 3. **Penghalang penunjuk** ([`Barrier`]) cukup soal
//!    [`RenderNode::hit_behavior`]: `Opaque` menyerap, `Ignore` meneruskan.

use silka_core::access::{AccessNode, AccessRole};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick};
use silka_core::input::{
    Event, EventCtx, FocusPolicy, HitBehavior, NamedKey, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_core::Callback;
use silka_paint::{Color, Point, Quad, Rect, Size};

use super::placement::{Anchor, Placed, Placement, PlacementMode};

// ---------------------------------------------------------------------------
// Barrier
// ---------------------------------------------------------------------------

/// Bagaimana area **di luar panel** memperlakukan penunjuk, keyboard, dan
/// teknologi bantu.
///
/// Inilah satu-satunya sumbu yang membedakan dialog dari tooltip; sisanya
/// (penempatan, transisi, dismiss) sama persis untuk keduanya.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Barrier {
    /// **Modal**: penunjuk terhalang, konten di belakang menjadi inert —
    /// tidak bisa di-Tab dan tidak dibacakan screen reader. Dialog, alert,
    /// sheet.
    #[default]
    Modal,
    /// **Light dismiss**: klik di luar panel ditangkap untuk menutup, tapi
    /// konten di belakang tetap hidup bagi keyboard dan screen reader.
    /// Popover, menu, combo box.
    Light,
    /// Hanya panelnya yang menerima penunjuk; sisanya tembus ke konten.
    /// Toast, drawer non-modal.
    Panel,
    /// Tidak menerima penunjuk sama sekali — tooltip tidak boleh "menangkap"
    /// mouse yang lewat di bawahnya.
    None,
}

impl Barrier {
    /// Benar bila konten di belakang harus dimatikan (fokus + a11y).
    pub fn is_modal(self) -> bool {
        matches!(self, Barrier::Modal)
    }

    /// Benar bila area di luar panel menyerap penunjuk.
    pub fn blocks_pointer(self) -> bool {
        matches!(self, Barrier::Modal | Barrier::Light)
    }

    /// Peran node ini dalam navigasi fokus saat overlay terlihat.
    pub fn focus_policy(self) -> FocusPolicy {
        match self {
            // Perangkap fokus **dan** bisa dituju sendiri: dialog yang baru
            // terbuka harus punya tempat mendarat walau isinya belum ada satu
            // pun kontrol yang focusable.
            Barrier::Modal => FocusPolicy {
                focusable: true,
                scope: true,
                ..FocusPolicy::NONE
            },
            Barrier::Light => FocusPolicy::SCOPE,
            Barrier::Panel | Barrier::None => FocusPolicy::NONE,
        }
    }
}

// ---------------------------------------------------------------------------
// Dismiss
// ---------------------------------------------------------------------------

/// Cara-cara sebuah overlay boleh ditutup pengguna, sebagai bitset.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Dismiss(u8);

impl Dismiss {
    /// Tidak bisa ditutup pengguna (harus lewat tombol di dalam panel).
    pub const NONE: Self = Self(0);
    /// Klik/ketuk di luar panel.
    pub const OUTSIDE: Self = Self(1 << 0);
    /// Tombol Esc.
    pub const ESCAPE: Self = Self(1 << 1);
    /// Keduanya — bawaan HIG untuk popover dan dialog non-destruktif.
    pub const ALL: Self = Self(0b11);

    /// Benar bila tidak satu pun cara diizinkan.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Benar bila seluruh cara `other` termasuk di sini.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Gabungan dua himpunan.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl core::ops::BitOr for Dismiss {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl core::fmt::Debug for Dismiss {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut nama = Vec::new();
        if self.contains(Dismiss::OUTSIDE) {
            nama.push("outside");
        }
        if self.contains(Dismiss::ESCAPE) {
            nama.push("escape");
        }
        if nama.is_empty() {
            nama.push("none");
        }
        write!(f, "Dismiss({})", nama.join("|"))
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// Node render satu overlay.
pub struct OverlayEntry {
    /// Terbuka atau sedang menutup.
    pub open: bool,
    /// Titik tambat (koordinat lokal layer).
    pub anchor: Anchor,
    /// Resep penempatan.
    pub placement: Placement,
    /// Warna peredup di belakang panel — token `scrim`, `None` = tanpa
    /// backdrop.
    pub backdrop: Option<Color>,
    /// Perlakuan terhadap area di luar panel.
    pub barrier: Barrier,
    /// Cara-cara yang diizinkan untuk menutup.
    pub dismiss: Dismiss,
    /// Apa yang dijalankan saat pengguna menutup overlay ini.
    pub on_dismiss: Option<Callback>,
    /// Peran a11y panel (Dialog/Menu/Tooltip).
    pub role: AccessRole,
    /// Nama yang dibacakan screen reader.
    pub label: Option<String>,
    /// Jarak tempuh transisi masuk; `None` = bawaan mode penempatan.
    pub travel: Option<f32>,

    /// Kemajuan transisi: 0 = tertutup, 1 = terbuka.
    progress: SpringValue<f32>,
    /// Hasil penempatan terakhir — dipakai transisi dan uji.
    placed: Placed,
    /// Kotak panel pada koordinat lokal node ini, hasil layout terakhir.
    panel: Rect,
    /// Penunjuk ditekan di luar panel; pelepasannya baru menutup overlay.
    press_outside: bool,
}

impl Default for OverlayEntry {
    fn default() -> Self {
        Self {
            open: false,
            anchor: Anchor::None,
            placement: Placement::center(),
            backdrop: None,
            barrier: Barrier::default(),
            dismiss: Dismiss::ALL,
            on_dismiss: None,
            role: AccessRole::Dialog,
            label: None,
            travel: None,
            progress: SpringValue::new(0.0).with_spring(Spring::snappy()),
            placed: Placed {
                origin: Point::ZERO,
                side: super::placement::PhysicalSide::Top,
                mode: PlacementMode::Center,
                flipped: false,
                shifted: 0.0,
            },
            panel: Rect::default(),
            press_outside: false,
        }
    }
}

impl OverlayEntry {
    /// Kemajuan transisi saat ini (0..1).
    pub fn progress(&self) -> f32 {
        self.progress.position()
    }

    /// Spring yang menjalankan transisinya.
    pub fn spring(&self) -> Spring {
        self.progress.spring()
    }

    /// Ganti spring tanpa mengganggu gerakan yang sedang berjalan.
    pub fn set_spring(&mut self, spring: Spring) {
        self.progress.set_spring(spring);
    }

    /// Benar bila transisinya masih bergerak dan frame berikutnya dibutuhkan.
    pub fn is_animating(&self) -> bool {
        self.progress.is_animating()
    }

    /// Benar bila overlay masih menyumbang piksel — terbuka, **atau** sedang
    /// menutup.
    ///
    /// Selama transisi keluar berlangsung node tetap ada di pohon: itulah yang
    /// membuat "hilangnya" sebuah dialog bisa dianimasikan sama halusnya dengan
    /// kemunculannya, tanpa aplikasi harus menahan-nahan struktur view-nya.
    pub fn is_visible(&self) -> bool {
        self.open || self.progress.position() > 0.0
    }

    /// Hasil penempatan terakhir.
    pub fn placed(&self) -> Placed {
        self.placed
    }

    /// Kotak panel pada koordinat lokal node ini (hasil layout terakhir).
    pub fn panel_rect(&self) -> Rect {
        self.panel
    }

    /// Arahkan transisi ke keadaan `open`.
    ///
    /// Retarget, bukan animasi baru: dialog yang ditutup di tengah animasi
    /// buka berbalik arah membawa kecepatannya (§3.5).
    pub fn set_open(&mut self, open: bool) {
        if self.open == open {
            return;
        }
        self.open = open;
        self.progress.set_target(if open { 1.0 } else { 0.0 });
    }

    /// Majukan transisi satu frame; benar bila posisinya berubah.
    ///
    /// Dipanggil [`super::advance`], yang menjadi satu-satunya tempat seluruh
    /// overlay sebuah pohon dimajukan bersama-sama.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        if !self.progress.is_animating() {
            return false;
        }
        let sebelum = self.progress.position();
        tick.advance(&mut self.progress);
        self.progress.position() != sebelum
    }

    /// Selesaikan transisi seketika (tanpa animasi).
    pub fn settle(&mut self) {
        self.progress.settle();
    }

    /// Jalankan `on_dismiss` bila `cara` memang diizinkan; benar bila jadi
    /// ditutup.
    ///
    /// Callback disalin keluar dulu — ia hampir selalu menulis signal, dan
    /// tulisan signal boleh memicu apa saja; yang tidak boleh adalah ia
    /// berjalan sambil node ini masih dipinjam `&mut` (pola yang sama dengan
    /// [`silka_core::tree::Interactive`]).
    pub fn request_dismiss(&mut self, cara: Dismiss) -> bool {
        if !self.dismiss.contains(cara) {
            return false;
        }
        let Some(cb) = self.on_dismiss.clone() else {
            return false;
        };
        cb.call();
        true
    }
}

impl RenderNode for OverlayEntry {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // Overlay selalu **memenuhi layer**: backdrop, penghalang penunjuk,
        // dan "di luar panel" semuanya butuh kotak yang sama besarnya.
        //
        // [`super::overlay_layer`] selalu memberi constraints tight, jadi
        // "memenuhi" tidak ambigu di jalur normal. Tapi overlay yang dipasang
        // langsung di tempat lain bisa menerima sumbu tak terbatas, dan
        // "memenuhi tak hingga" tidak berarti apa-apa — sumbu itu jatuh ke
        // ukuran panelnya sendiri, bukan ke `f32::INFINITY`.
        let terbesar = constraints.biggest();
        if ctx.child_count() == 0 {
            self.panel = Rect::default();
            return constraints.constrain(Size::new(
                if terbesar.width.is_finite() {
                    terbesar.width
                } else {
                    0.0
                },
                if terbesar.height.is_finite() {
                    terbesar.height
                } else {
                    0.0
                },
            ));
        }
        let panel = ctx.child(0);
        // Panel mengukur dirinya sendiri, dibatasi ukuran layer.
        let ukuran = ctx.layout_child(panel, constraints.loosen());
        let size = constraints.constrain(Size::new(
            if terbesar.width.is_finite() {
                terbesar.width
            } else {
                ukuran.width
            },
            if terbesar.height.is_finite() {
                terbesar.height
            } else {
                ukuran.height
            },
        ));
        let bounds = Rect::from_origin_size(Point::ZERO, size);
        self.placed = super::place(
            ukuran,
            self.anchor.rect(bounds),
            bounds,
            self.placement,
            ctx.direction(),
        );
        let jarak = self
            .travel
            .unwrap_or_else(|| self.placement.default_travel(ukuran));
        let geser = self.placed.enter_offset(jarak, self.progress.position());
        let origin = Point::new(
            self.placed.origin.x + geser.x,
            self.placed.origin.y + geser.y,
        );
        ctx.place_child(panel, origin);
        self.panel = Rect::from_origin_size(origin, ukuran);
        size
    }

    /// Ukurannya ditentukan layer sepenuhnya, jadi isi panel setinggi apa pun
    /// tidak pernah membuat window di-layout ulang.
    fn is_relayout_boundary(&self) -> bool {
        true
    }

    /// Panel yang sedang menyembul dari tepi dipotong di tepi itu — tanpa ini
    /// sheet "masuk dari luar layar" justru terlihat menggantung di luar
    /// window.
    fn clips_children(&self) -> bool {
        true
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        if !self.is_visible() {
            return;
        }
        let p = self.progress.position().clamp(0.0, 1.0);
        if let Some(scrim) = self.backdrop {
            // Peredup ikut memudar bersama transisi — satu-satunya "fade" yang
            // bisa dijanjikan tanpa layer offscreen (§3.6).
            let warna = scrim.with_alpha(scrim.a * p);
            if warna.a > 0.0 {
                ctx.quad(Quad::new(ctx.local_bounds()).background(warna));
            }
        }
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.role;
        node.label.clone_from(&self.label);
        // Overlay tertutup tidak ada bagi screen reader — beserta seluruh
        // isinya, walau node-nya masih di pohon menunggu transisi keluar.
        node.hidden = !self.is_visible();
    }

    fn hit_behavior(&self) -> HitBehavior {
        if !self.is_visible() {
            return HitBehavior::Ignore;
        }
        match self.barrier {
            Barrier::None => HitBehavior::Ignore,
            Barrier::Panel => HitBehavior::DeferToChild,
            Barrier::Modal | Barrier::Light => HitBehavior::Opaque,
        }
    }

    fn focus_policy(&self) -> FocusPolicy {
        if !self.is_visible() {
            // Isi overlay tertutup tidak boleh bisa di-Tab.
            return FocusPolicy::NONE.skip_subtree();
        }
        self.barrier.focus_policy()
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        if !self.is_visible() {
            return;
        }
        match event {
            Event::Pointer(p) if self.barrier.blocks_pointer() => {
                let di_luar = !self.panel.contains(ctx.local());
                match p.phase {
                    PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                        self.press_outside = di_luar;
                        ctx.handled();
                    }
                    PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                        // Tekan **dan** lepas sama-sama di luar panel: aturan
                        // yang sama dengan tombol AppKit, dan yang mencegah
                        // drag dari dalam panel ke luar menutup overlay.
                        let tutup = self.press_outside && di_luar;
                        self.press_outside = false;
                        if tutup {
                            self.request_dismiss(Dismiss::OUTSIDE);
                        }
                        ctx.handled();
                    }
                    PointerPhase::Cancel => self.press_outside = false,
                    _ => {}
                }
            }
            // Esc hanya ditandai handled kalau overlay ini memang punya
            // penerimanya: alert tanpa `on_dismiss` harus **membiarkan** Esc
            // menggelembung, bukan menelannya diam-diam.
            Event::Key(k)
                if k.is_pressed()
                    && k.code.is(NamedKey::Escape)
                    && self.dismiss.contains(Dismiss::ESCAPE)
                    && self.on_dismiss.is_some() =>
            {
                let ditutup = self.request_dismiss(Dismiss::ESCAPE);
                debug_assert!(ditutup, "guard sudah memastikan Esc punya penerima");
                ctx.handled();
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for OverlayEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OverlayEntry")
            .field("open", &self.open)
            .field("progress", &self.progress.position())
            .field("barrier", &self.barrier)
            .field("dismiss", &self.dismiss)
            .field("panel", &self.panel)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props satu overlay — bentuk view dari [`OverlayEntry`].
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayProps {
    pub(super) open: bool,
    pub(super) anchor: Anchor,
    pub(super) placement: Placement,
    pub(super) backdrop: Option<Color>,
    pub(super) barrier: Barrier,
    pub(super) dismiss: Dismiss,
    pub(super) on_dismiss: Option<Callback>,
    pub(super) role: AccessRole,
    pub(super) label: Option<String>,
    pub(super) travel: Option<f32>,
    pub(super) spring: Spring,
    pub(super) motion: MotionRole,
}

impl Default for OverlayProps {
    fn default() -> Self {
        Self {
            open: false,
            anchor: Anchor::None,
            placement: Placement::center(),
            backdrop: None,
            barrier: Barrier::default(),
            dismiss: Dismiss::ALL,
            on_dismiss: None,
            role: AccessRole::Dialog,
            label: None,
            travel: None,
            spring: Spring::snappy(),
            motion: MotionRole::Essential,
        }
    }
}

impl OverlayProps {
    fn spring_value(&self) -> SpringValue<f32> {
        let mut v = SpringValue::new(0.0).with_spring(self.spring);
        if self.motion == MotionRole::Decorative {
            v = v.decorative();
        }
        // Overlay yang lahir dalam keadaan terbuka tetap **beranimasi masuk**:
        // itu perbedaan antara dialog yang muncul dan dialog yang mengagetkan.
        if self.open {
            v.set_target(1.0);
        }
        v
    }
}

impl ViewNode for OverlayProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(OverlayEntry {
            open: self.open,
            anchor: self.anchor,
            placement: self.placement,
            backdrop: self.backdrop,
            barrier: self.barrier,
            dismiss: self.dismiss,
            on_dismiss: self.on_dismiss.clone(),
            role: self.role,
            label: self.label.clone(),
            travel: self.travel,
            progress: self.spring_value(),
            ..OverlayEntry::default()
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<OverlayEntry>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.open != self.open {
            n.set_open(self.open);
            // Transisi butuh layout (panel bergeser) **dan** frame berikutnya.
            dirty |= Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.anchor != self.anchor || n.placement != self.placement || n.travel != self.travel {
            n.anchor = self.anchor;
            n.placement = self.placement;
            n.travel = self.travel;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.backdrop != self.backdrop {
            n.backdrop = self.backdrop;
            dirty |= Dirty::PAINT;
        }
        if n.barrier != self.barrier {
            n.barrier = self.barrier;
            n.press_outside = false;
            dirty |= Dirty::PAINT;
        }
        if n.dismiss != self.dismiss {
            n.dismiss = self.dismiss;
        }
        if n.role != self.role {
            n.role = self.role;
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.progress.spring() != self.spring {
            n.progress.set_spring(self.spring);
        }
        // Callback selalu diganti tanpa dibandingkan: closure dibangun ulang
        // tiap rebuild dan menangkap nilai baru (lihat `InteractiveProps`).
        n.on_dismiss.clone_from(&self.on_dismiss);
        dirty
    }
}

/// Satu overlay berisi `panel` — dialog, popover, tooltip, menu, atau toast.
///
/// Konstruktor gaya Dart (§2.5); seluruh sifatnya pindah ke method chain.
///
/// ```
/// # use silka_core::signals::Runtime;
/// # use silka_core::view::fixed;
/// # use silka_theme::{Appearance, Theme};
/// use silka_widgets::overlay::{overlay, Barrier, Dismiss, Placement, Side};
///
/// # let rt = Runtime::new();
/// # let terbuka = rt.signal(true);
/// # let t = Theme::cupertino(Appearance::Dark);
/// let _ = overlay(fixed(320.0, 180.0).background(t.color.surface_elevated))
///     .open(terbuka.get())
///     .placement(Placement::center())
///     .backdrop(t.color.scrim)
///     .barrier(Barrier::Modal)
///     .dismiss(Dismiss::ALL)
///     .label("Simpan perubahan?")
///     .on_dismiss(move || terbuka.set(false));
/// # let _ = Side::Bottom;
/// ```
pub fn overlay(panel: impl Into<View>) -> OverlayBuilder {
    OverlayBuilder {
        key: None,
        props: OverlayProps::default(),
        panel: panel.into(),
    }
}

/// Builder satu overlay.
///
/// Tipe sendiri, bukan [`silka_core::view::Builder`], karena lapisan layer
/// perlu **membaca** `open`/`barrier` sebelum pohon dirakit: hanya dengan
/// begitu ia tahu apakah konten di belakang harus dimatikan (lihat
/// [`super::overlay_layer`]).
pub struct OverlayBuilder {
    pub(super) key: Option<silka_core::signals::Key>,
    pub(super) props: OverlayProps,
    pub(super) panel: View,
}

impl OverlayBuilder {
    /// Kunci identitas — wajib untuk overlay yang datang dari daftar dinamis
    /// (tumpukan toast).
    pub fn key(mut self, key: impl Into<silka_core::signals::Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Terbuka atau tertutup. Perubahannya **memicu transisi**, bukan lompatan.
    pub fn open(mut self, open: bool) -> Self {
        self.props.open = open;
        self
    }

    /// Titik tambat (koordinat lokal layer) — lihat [`super::anchor_rect`].
    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.props.anchor = anchor;
        self
    }

    /// Resep penempatan.
    pub fn placement(mut self, placement: Placement) -> Self {
        self.props.placement = placement;
        self
    }

    /// Warna peredup di belakang panel — **selalu** token `scrim`.
    pub fn backdrop(mut self, color: Color) -> Self {
        self.props.backdrop = Some(color);
        self
    }

    /// Tanpa peredup.
    pub fn no_backdrop(mut self) -> Self {
        self.props.backdrop = None;
        self
    }

    /// Perlakuan area di luar panel.
    pub fn barrier(mut self, barrier: Barrier) -> Self {
        self.props.barrier = barrier;
        self
    }

    /// Cara-cara yang diizinkan untuk menutup.
    pub fn dismiss(mut self, dismiss: Dismiss) -> Self {
        self.props.dismiss = dismiss;
        self
    }

    /// Apa yang dijalankan saat pengguna menutup overlay ini.
    pub fn on_dismiss(mut self, f: impl Fn() + 'static) -> Self {
        self.props.on_dismiss = Some(Callback::new(f));
        self
    }

    /// Peran a11y panel (bawaan [`AccessRole::Dialog`]).
    pub fn role(mut self, role: AccessRole) -> Self {
        self.props.role = role;
        self
    }

    /// Nama yang dibacakan screen reader saat overlay terbuka.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.props.label = Some(label.into());
        self
    }

    /// Jarak tempuh transisi masuk, poin logis (token spacing).
    pub fn travel(mut self, travel: f32) -> Self {
        self.props.travel = Some(travel.max(0.0));
        self
    }

    /// Spring yang menjalankan transisinya (`smooth`/`snappy`/`bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.props.spring = spring;
        self
    }

    /// Tandai gerakannya **dekoratif**: reduced-motion mematikannya sepenuhnya
    /// alih-alih sekadar membuang pantulannya
    /// ([`silka_core::animation::Motion`]).
    pub fn decorative(mut self) -> Self {
        self.props.motion = MotionRole::Decorative;
        self
    }

    /// Benar bila overlay ini modal dan sedang terbuka.
    pub(super) fn blocks_content(&self) -> bool {
        self.props.open && self.props.barrier.is_modal()
    }
}

impl From<OverlayBuilder> for View {
    fn from(b: OverlayBuilder) -> View {
        let mut builder = Builder::new(b.props).child(b.panel);
        if let Some(key) = b.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for OverlayBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OverlayBuilder")
            .field("key", &self.key)
            .field("props", &self.props)
            .finish()
    }
}
