//! Pemilik siklus hidup: satu tempat yang memegang runtime signals, closure
//! pembangun view akar, render tree, dan frame scheduler.
//!
//! Modul ini adalah **jahitan** yang sebelumnya sengaja ditinggalkan kosong:
//! [`crate::signals::Runtime::drain_dirty`] akhirnya punya pemanggil, dan
//! kontraknya dipenuhi apa adanya — membangun ulang sebuah scope **memasuki
//! kembali** setiap anak yang dipertahankan, karena [`super::component`]
//! membangun anaknya secara eager di dalam [`crate::signals::scope`].

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Instant;

use rustui_paint::{Color, Scene, Size};

use crate::access::AccessTree;
use crate::animation::{AnimationDriver, Motion, Tick};
use crate::input::{Event, InputRouter, Response};
use crate::scheduler::{Dirty, FrameScheduler, FrameTiming, Wake};
use crate::signals::{current_scope, Runtime, ScopeId};
use crate::tree::{BoxConstraints, NodeId, RenderTree, TextDirection};
use crate::view::{reconcile_children, DiffStats, View};

use super::component::ComponentBox;

/// Closure pembangun satu komponen: dijalankan **di dalam** scope-nya sendiri.
pub(super) type ComponentBuilder = Rc<dyn Fn(&BuildCtx) -> View>;

/// Pemberitahu shell bahwa scheduler menerima permintaan frame.
type WakeFn = Rc<dyn Fn(Wake)>;

// ---------------------------------------------------------------------------
// Env
// ---------------------------------------------------------------------------

/// Titipan tingkat aplikasi yang bisa dibaca setiap komponen saat dibangun.
///
/// Isinya biasanya **signal**, bukan nilai: shell menaruh `Signal<Theme>` di
/// sini sekali, lalu memperbaruinya tiap kali dark mode OS berubah — dan hanya
/// komponen yang benar-benar membaca theme yang ikut dibangun ulang (§2.7,
/// §3.5). Menaruh nilai mentah juga sah, tapi ia tidak reaktif.
///
/// Dikunci per tipe: satu tipe = satu titipan. Itu cukup, dan menutup kelas bug
/// "ambil yang mana" yang muncul kalau kuncinya string.
#[derive(Default)]
pub struct Env {
    map: HashMap<TypeId, Box<dyn Any>>,
}

impl Env {
    /// Env kosong.
    pub fn new() -> Self {
        Self::default()
    }

    /// Titipkan sebuah nilai; yang bertipe sama sebelumnya digantikan.
    pub fn insert<T: 'static>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Ambil rujukan ke titipan bertipe `T`.
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map.get(&TypeId::of::<T>())?.downcast_ref::<T>()
    }

    /// Benar bila ada titipan bertipe `T`.
    pub fn contains<T: 'static>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    /// Jumlah titipan.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Benar bila tidak ada titipan sama sekali.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl core::fmt::Debug for Env {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Env").field("len", &self.map.len()).finish()
    }
}

/// Scale factor layar tempat aplikasi ini digambar (2.0 di Retina).
///
/// Titipan [`Env`] standar, dan bukan kemewahan: **teks harus dirasterisasi
/// pada resolusi layar yang sebenarnya** (§3.3), jadi komponen yang mengukur
/// atau menggambar teks perlu tahu angkanya. Shell menitipkannya sebagai
/// `Signal<ScaleFactor>` dan memperbaruinya tiap frame, sehingga memindahkan
/// window ke monitor lain hanya membangun ulang komponen yang benar-benar
/// membacanya (§2.7, §3.5).
///
/// Ukuran logis tidak pernah ikut berubah karenanya — yang berubah hanya
/// resolusi bitmap glyph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScaleFactor(pub f32);

impl ScaleFactor {
    /// Satu piksel fisik per poin logis — layar non-Retina, dan nilai bawaan
    /// sebelum shell melapor.
    pub const ONE: ScaleFactor = ScaleFactor(1.0);

    /// Nilainya, selalu terhingga dan positif.
    pub fn get(self) -> f32 {
        if self.0.is_finite() && self.0 > 0.0 {
            self.0
        } else {
            1.0
        }
    }
}

impl Default for ScaleFactor {
    fn default() -> Self {
        Self::ONE
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Peta scope → cara membangunnya kembali, dan di mana hasilnya menempel.
///
/// Inilah yang membuat rebuild **per-komponen** mungkin: `drain_dirty()`
/// memberi `ScopeId`, dan dua peta ini menerjemahkannya menjadi "closure mana
/// yang dijalankan" dan "di bawah node render mana hasilnya di-diff".
#[derive(Default)]
struct Registry {
    builders: HashMap<ScopeId, ComponentBuilder>,
    anchors: HashMap<ScopeId, NodeId>,
}

// ---------------------------------------------------------------------------
// HostShared
// ---------------------------------------------------------------------------

/// Bagian [`AppRuntime`] yang harus bisa disentuh dari dalam build.
///
/// [`super::component`] dipanggil di tengah-tengah closure pengguna, jauh dari
/// `&mut AppRuntime`; ia menemukan host lewat tumpukan thread-local di bawah.
pub(super) struct HostShared {
    runtime: Runtime,
    scheduler: RefCell<FrameScheduler>,
    wake: RefCell<Option<WakeFn>>,
    env: RefCell<Env>,
    reg: RefCell<Registry>,
}

impl HostShared {
    /// Catat closure pembangun sebuah scope komponen.
    pub(super) fn register(&self, scope: ScopeId, builder: ComponentBuilder) {
        self.reg.borrow_mut().builders.insert(scope, builder);
    }
}

thread_local! {
    /// Tumpukan host yang sedang membangun di thread ini.
    ///
    /// Tumpukan (bukan satu slot) karena dua window = dua [`AppRuntime`], dan
    /// keduanya hidup di UI thread yang sama.
    static HOSTS: RefCell<Vec<Rc<HostShared>>> = const { RefCell::new(Vec::new()) };
}

/// Host yang sedang membangun di thread ini, bila ada.
pub(super) fn current_host() -> Option<Rc<HostShared>> {
    HOSTS.with(|h| h.borrow().last().cloned())
}

/// Penjaga tumpukan host — tetap benar walau closure pengguna panik.
struct HostGuard;

impl HostGuard {
    fn push(host: Rc<HostShared>) -> Self {
        HOSTS.with(|h| h.borrow_mut().push(host));
        HostGuard
    }
}

impl Drop for HostGuard {
    fn drop(&mut self) {
        let _ = HOSTS.try_with(|h| {
            h.borrow_mut().pop();
        });
    }
}

// ---------------------------------------------------------------------------
// BuildCtx
// ---------------------------------------------------------------------------

/// Apa yang dilihat sebuah komponen saat dibangun.
///
/// Sengaja tipis: state lokal datang dari [`crate::signals::use_signal`], anak
/// datang dari [`super::component`], dan titipan tingkat aplikasi dari
/// [`BuildCtx::env`]. Tidak ada `setState`, tidak ada pohon widget yang bisa
/// diraba dari sini (§2.5).
pub struct BuildCtx {
    host: Rc<HostShared>,
}

impl BuildCtx {
    pub(super) fn new(host: Rc<HostShared>) -> Self {
        Self { host }
    }

    /// Runtime signals aplikasi ini.
    pub fn runtime(&self) -> &Runtime {
        &self.host.runtime
    }

    /// Scope komponen yang sedang dibangun.
    ///
    /// Panik bila dipanggil di luar build — sama seperti
    /// [`crate::signals::use_signal`].
    pub fn scope(&self) -> ScopeId {
        current_scope().expect("BuildCtx::scope() hanya berlaku saat komponen dibangun")
    }

    /// Salin titipan tingkat aplikasi bertipe `T` ([`Env`]).
    ///
    /// Dibuat mengembalikan salinan, bukan rujukan: yang dititipkan hampir
    /// selalu [`crate::signals::Signal`] yang `Copy`, dan mengembalikan
    /// rujukan akan menahan pinjaman `Env` selama build berlangsung.
    pub fn env<T: Clone + 'static>(&self) -> Option<T> {
        self.host.env.borrow().get::<T>().cloned()
    }

    /// Seperti [`BuildCtx::env`], tapi panik bila titipannya tidak ada.
    pub fn expect_env<T: Clone + 'static>(&self) -> T {
        self.env::<T>().unwrap_or_else(|| {
            panic!(
                "tidak ada titipan bertipe {} di Env aplikasi — pasang lewat AppRuntime::with_env",
                std::any::type_name::<T>()
            )
        })
    }
}

impl core::fmt::Debug for BuildCtx {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BuildCtx")
            .field("scope", &current_scope())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// FrameReport
// ---------------------------------------------------------------------------

/// Ringkasan satu putaran [`AppRuntime::frame`].
///
/// Bukan hiasan: inilah yang dipakai test untuk membuktikan bahwa **hanya**
/// subtree terkait yang dibangun ulang, dan yang dipakai inspector untuk
/// menjelaskan jank.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameReport {
    /// Nomor urut frame.
    pub index: u64,
    /// Alasan frame ini dijadwalkan (kosong = frame yang diminta OS).
    pub reason: Dirty,
    /// Berapa scope komponen yang benar-benar dibangun ulang.
    pub rebuilt: usize,
    /// Hasil diff seluruh rebuild pada frame ini, dijumlahkan.
    pub diff: DiffStats,
    /// Berapa relayout boundary yang mengantre saat layout dimulai.
    pub relayouts: usize,
    /// Ukuran akhir pohon setelah layout.
    pub size: Size,
    /// Pengukuran waktu frame.
    pub timing: FrameTiming,
}

impl FrameReport {
    /// Benar bila frame ini tidak mengubah struktur maupun props apa pun.
    pub fn is_noop(&self) -> bool {
        self.rebuilt == 0 && self.diff.is_noop()
    }
}

// ---------------------------------------------------------------------------
// AppRuntime
// ---------------------------------------------------------------------------

/// Pemilik satu siklus hidup UI: **signals → view → layout → paint →
/// scheduler**.
///
/// Satu instans per window. Ia memegang keempat bagian yang selama ini hidup
/// terpisah dan menjahitnya menjadi satu putaran [`AppRuntime::frame`]:
///
/// 1. [`crate::signals::Runtime::drain_dirty`] → daftar scope yang harus
///    dibangun ulang (sudah terurut akar→daun dan terpangkas).
/// 2. Untuk tiap scope: jalankan ulang closure-nya **di dalam scope itu**, lalu
///    diff hasilnya terhadap anak-anak node jangkar scope tersebut.
/// 3. [`crate::tree::RenderTree::perform_layout`] — penuh bila constraints
///    window berubah, selebihnya hanya boundary yang kotor.
/// 4. [`crate::tree::RenderTree::paint`] → [`Scene`].
///
/// Sambungan ke scheduler dipasang sekali di [`AppRuntime::new`]:
/// [`crate::signals::Runtime::on_wake`] langsung memanggil
/// [`FrameScheduler::request`], sehingga janji §3.5 tetap utuh — signal yang
/// tidak dibaca komponen mana pun tidak menjadwalkan apa pun, dan tanpa
/// perubahan signal [`AppRuntime::is_idle`] tetap benar.
///
/// ```
/// use rustui_core::app::{app, component};
/// use rustui_core::signals::use_signal;
/// use rustui_core::view::{column, fixed};
/// use rustui_paint::Color;
///
/// let mut ui = app(|_cx| {
///     let count = use_signal(|| 0i32);
///     column([component("angka", move |_| {
///         fixed(40.0, 20.0 + count.get() as f32).background(Color::WHITE).into()
///     })])
///     .into()
/// })
/// .sized(200.0, 200.0);
///
/// let laporan = ui.frame();
/// assert_eq!(laporan.rebuilt, 1);
/// assert_eq!(ui.scene().len(), 1);
/// // Tanpa perubahan signal, tidak ada frame berikutnya.
/// assert!(ui.is_idle());
/// ```
pub struct AppRuntime {
    host: Rc<HostShared>,
    tree: RenderTree,
    scene: Scene,
    router: InputRouter,
    root: ScopeId,
    constraints: BoxConstraints,
    mounted: bool,
    /// Jam animasi + preferensi reduced-motion (§3.5). Dipakai
    /// [`AppRuntime::animate`], satu-satunya pintu tempat spring dimajukan.
    anim: AnimationDriver,
}

/// Buat aplikasi dari closure pembangun view akar — konstruktor gaya Dart
/// (§2.5).
///
/// Closure-nya dijalankan di dalam scope akar runtime signals, jadi
/// [`crate::signals::use_signal`] boleh dipakai langsung di dalamnya.
pub fn app(build: impl Fn(&BuildCtx) -> View + 'static) -> AppRuntime {
    AppRuntime::new(build)
}

impl AppRuntime {
    /// Bentuk panjang dari [`app`].
    pub fn new(build: impl Fn(&BuildCtx) -> View + 'static) -> Self {
        let runtime = Runtime::new();
        let root = runtime.root();
        let host = Rc::new(HostShared {
            runtime: runtime.clone(),
            scheduler: RefCell::new(FrameScheduler::new()),
            wake: RefCell::new(None),
            env: RefCell::new(Env::new()),
            reg: RefCell::new(Registry::default()),
        });

        // **Sambungan signals → scheduler** (§3.5). `Weak` supaya runtime yang
        // memegang closure ini tidak menahan host-nya hidup selamanya.
        let lemah: Weak<HostShared> = Rc::downgrade(&host);
        runtime.on_wake(move |dirty| {
            let Some(host) = lemah.upgrade() else { return };
            let wake = host.scheduler.borrow_mut().request(dirty);
            // Pinjaman dilepas sebelum callback platform berjalan: ia boleh
            // memanggil balik ke sini (mis. menyalakan display link).
            let cb = host.wake.borrow().clone();
            if let Some(cb) = cb {
                cb(wake);
            }
        });

        let tree = RenderTree::new();
        {
            let mut reg = host.reg.borrow_mut();
            reg.builders.insert(root, Rc::new(build));
            // Jangkar scope akar adalah akar render tree — dengan begitu satu
            // jalur rebuild yang sama melayani akar dan komponen mana pun.
            reg.anchors.insert(root, tree.root());
        }

        let clear = tree.clear_color();
        let app = Self {
            host,
            tree,
            scene: Scene::new(clear),
            router: InputRouter::new(),
            root,
            constraints: BoxConstraints::tight(Size::ZERO),
            mounted: false,
            anim: AnimationDriver::new(),
        };
        // Frame pertama adalah satu-satunya frame yang tidak dipicu perubahan.
        app.request(Dirty::LAYOUT | Dirty::PAINT);
        app
    }

    // -- konfigurasi (method chaining, §2.5) --------------------------------

    /// Ukuran area gambar dalam poin logis.
    pub fn sized(mut self, width: f32, height: f32) -> Self {
        self.resize(Size::new(width, height));
        self
    }

    /// Warna latar frame — selalu token `background`, tidak pernah literal.
    pub fn clear_color(mut self, color: Color) -> Self {
        self.set_clear_color(color);
        self
    }

    /// Arah baca dokumen (§9.8).
    pub fn direction(mut self, direction: TextDirection) -> Self {
        self.set_direction(direction);
        self
    }

    /// Titipkan nilai tingkat aplikasi ke [`Env`].
    ///
    /// Closure-nya menerima runtime supaya titipan yang lazim — sebuah
    /// [`crate::signals::Signal`] — bisa dibuat di tempat:
    ///
    /// ```
    /// # use rustui_core::app::app;
    /// # use rustui_core::view::fixed;
    /// let ui = app(|cx| {
    ///     let judul: rustui_core::signals::Signal<&'static str> = cx.expect_env();
    ///     fixed(10.0, 10.0).label(judul.get()).into()
    /// })
    /// .with_env(|rt| rt.signal("Beranda"));
    /// ```
    pub fn with_env<T: 'static>(self, f: impl FnOnce(&Runtime) -> T) -> Self {
        let value = f(&self.host.runtime);
        self.host.env.borrow_mut().insert(value);
        self
    }

    /// Pasang pemberitahu "frame dijadwalkan" untuk shell.
    ///
    /// Dipanggil setiap kali scheduler menerima permintaan — [`Wake::Schedule`]
    /// berarti sumber vsync harus dibangunkan, sisanya berarti tidak ada yang
    /// perlu dilakukan.
    pub fn on_wake(&self, f: impl Fn(Wake) + 'static) {
        *self.host.wake.borrow_mut() = Some(Rc::new(f));
    }

    // -- akses ---------------------------------------------------------------

    /// Runtime signals aplikasi ini.
    pub fn runtime(&self) -> &Runtime {
        &self.host.runtime
    }

    /// Render tree hasil frame terakhir.
    pub fn tree(&self) -> &RenderTree {
        &self.tree
    }

    /// Router input aplikasi ini.
    pub fn router(&self) -> &InputRouter {
        &self.router
    }

    /// Scene hasil frame terakhir.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Scope akar (komponen terluar).
    pub fn root_scope(&self) -> ScopeId {
        self.root
    }

    /// Node render jangkar sebuah scope komponen, bila masih hidup.
    pub fn anchor(&self, scope: ScopeId) -> Option<NodeId> {
        self.host.reg.borrow().anchors.get(&scope).copied()
    }

    /// Constraints window yang berlaku.
    pub fn constraints(&self) -> BoxConstraints {
        self.constraints
    }

    /// Salin titipan [`Env`] bertipe `T` dari luar build.
    ///
    /// Inilah cara shell menyentuh kembali signal yang ia titipkan sendiri
    /// (mis. `Signal<Theme>` saat dark mode OS berubah).
    pub fn env<T: Clone + 'static>(&self) -> Option<T> {
        self.host.env.borrow().get::<T>().cloned()
    }

    /// Pohon aksesibilitas dari geometri frame terakhir (§3.8).
    ///
    /// Fokusnya diambil dari [`InputRouter`], bukan disimpan dua kali.
    pub fn access_tree(&self) -> AccessTree {
        self.tree.access_tree(self.router.focus().focused())
    }

    // -- scheduler ------------------------------------------------------------

    /// Minta satu frame karena `dirty`.
    pub fn request(&self, dirty: Dirty) -> Wake {
        let wake = self.host.scheduler.borrow_mut().request(dirty);
        let cb = self.host.wake.borrow().clone();
        if let Some(cb) = cb {
            cb(wake);
        }
        wake
    }

    /// Alasan-alasan yang belum dilayani.
    pub fn pending(&self) -> Dirty {
        self.host.scheduler.borrow().pending()
    }

    /// Benar bila tidak ada apa pun yang perlu digambar — **idle = nol kerja**.
    pub fn is_idle(&self) -> bool {
        self.host.scheduler.borrow().is_idle()
    }

    /// Nomor frame berikutnya.
    pub fn frame_index(&self) -> u64 {
        self.host.scheduler.borrow().frame_index()
    }

    /// Laporkan detak layar dari platform.
    pub fn set_vsync(&self, vsync: crate::scheduler::Vsync) {
        self.host.scheduler.borrow_mut().set_vsync(vsync);
    }

    /// Ringkasan waktu frame (salinan, karena scheduler-nya dibagi).
    pub fn frame_stats(&self) -> crate::scheduler::FrameStats {
        self.host.scheduler.borrow().stats().clone()
    }

    // -- animasi ---------------------------------------------------------------

    /// Preferensi gerakan yang berlaku (reduced-motion OS).
    pub fn motion(&self) -> Motion {
        self.anim.motion()
    }

    /// Laporkan setting reduced-motion dari OS.
    ///
    /// Shell yang membacanya (`INTEGRASI-NATIVE` §6); di sini ia hanya
    /// dititipkan ke [`AnimationDriver`] dan, bila berubah, meminta satu frame
    /// supaya gerakan dekoratif yang sedang berjalan bisa menyelesaikan dirinya
    /// alih-alih membeku di tengah jalan.
    pub fn set_motion(&mut self, motion: Motion) -> Dirty {
        let dirty = self.anim.set_motion(motion);
        if !dirty.is_empty() {
            self.request(dirty);
        }
        dirty
    }

    /// **Majukan animasi satu frame** — jahitan antara spring dan siklus frame.
    ///
    /// Ini pintu tunggal yang sebelumnya sengaja ditinggalkan kosong: sistem
    /// animasi (§3.5) sudah lengkap, tapi tidak ada yang memanggilnya per frame.
    /// `f` menerima render tree dan [`Tick`] frame ini, lalu mengembalikan
    /// alasan dirty-nya — bentuk yang persis dipenuhi
    /// `rustui_widgets::advance`. Dirty-nya digabung dengan permintaan
    /// scheduler, jadi selama masih ada spring yang bergerak frame berikutnya
    /// dijadwalkan sendiri, dan begitu semuanya settle renderer kembali tidur.
    ///
    /// Dipanggil **sebelum** [`AppRuntime::frame`] supaya nilai yang bergerak
    /// sudah menjadi nilai frame ini, bukan frame berikutnya.
    ///
    /// ```
    /// use rustui_core::app::app;
    /// use rustui_core::scheduler::Dirty;
    /// use rustui_core::view::fixed;
    ///
    /// let mut ui = app(|_cx| fixed(80.0, 24.0).into()).sized(200.0, 100.0);
    /// // Tanpa satu pun animasi, majunya frame tidak melahirkan pekerjaan.
    /// assert_eq!(ui.animate(|_tree, _tick| Dirty::NONE), Dirty::NONE);
    /// ui.frame();
    /// assert!(ui.is_idle());
    /// ```
    pub fn animate(&mut self, f: impl FnOnce(&mut RenderTree, &Tick) -> Dirty) -> Dirty {
        self.animate_at(Instant::now(), f)
    }

    /// [`AppRuntime::animate`] dengan waktu frame yang ditentukan pemanggil.
    ///
    /// Untuk uji yang harus deterministik (§9.5) dan untuk shell yang sudah
    /// memegang timestamp vsync-nya sendiri — jangan pernah mengarang 16,6 ms
    /// (§3.5).
    pub fn animate_at(
        &mut self,
        now: Instant,
        f: impl FnOnce(&mut RenderTree, &Tick) -> Dirty,
    ) -> Dirty {
        let tick = self.anim.begin_frame(now);
        let mut dirty = f(&mut self.tree, &tick);
        dirty |= self.anim.end_frame(tick);
        // Tanda dirty yang lahir dari node yang baru saja bergerak ikut terbawa,
        // sama seperti di `dispatch`.
        dirty |= self.tree.take_dirty();
        if !dirty.is_empty() {
            self.request(dirty);
        }
        dirty
    }

    /// Benar bila frame animasi sebelumnya masih menyisakan yang bergerak.
    pub fn is_animating(&self) -> bool {
        self.anim.is_animating()
    }

    // -- perubahan dari luar ---------------------------------------------------

    /// Ganti ukuran area gambar; benar bila memang berubah.
    pub fn resize(&mut self, size: Size) -> bool {
        let baru = BoxConstraints::tight(size);
        if self.constraints == baru {
            return false;
        }
        self.constraints = baru;
        self.request(Dirty::SURFACE | Dirty::LAYOUT);
        true
    }

    /// Ganti warna latar frame; benar bila memang berubah.
    pub fn set_clear_color(&mut self, color: Color) -> bool {
        if self.tree.clear_color() == color {
            return false;
        }
        self.tree.set_clear_color(color);
        self.request(Dirty::THEME | Dirty::PAINT);
        true
    }

    /// Ganti arah baca dokumen; benar bila memang berubah.
    pub fn set_direction(&mut self, direction: TextDirection) -> bool {
        if self.tree.direction() == direction {
            return false;
        }
        self.tree.set_direction(direction);
        self.request(Dirty::LAYOUT | Dirty::PAINT);
        true
    }

    /// Salurkan satu event input ke pohon.
    ///
    /// Yang dikembalikan sudah memperhitungkan **tulisan signal** yang terjadi
    /// di dalam handler: `dirty`-nya digabung dengan apa yang menunggu di
    /// scheduler, sehingga shell tidak perlu tahu bedanya.
    pub fn dispatch(&mut self, event: &Event) -> Response {
        let mut hasil = self.router.dispatch(&mut self.tree, event);
        hasil.dirty |= self.tree.take_dirty();
        if !hasil.dirty.is_empty() {
            self.request(hasil.dirty);
        }
        hasil.dirty |= self.pending();
        hasil
    }

    // -- satu frame -----------------------------------------------------------

    /// Jalankan satu putaran penuh dan kembalikan ringkasannya.
    ///
    /// Urutannya tetap dan tidak boleh ditukar: rebuild → diff → layout →
    /// paint. Scene-nya bisa dibaca lewat [`AppRuntime::scene`].
    pub fn frame(&mut self) -> FrameReport {
        let mut start = self.host.scheduler.borrow_mut().begin_frame(Instant::now());

        // 1. Siapa yang harus dibangun ulang.
        //
        // Frame pertama membangun akar; selebihnya daftar datang dari signals
        // — sudah terurut akar→daun dan **terpangkas** (keturunan dari scope
        // yang juga kotor dibuang), jadi tidak ada subtree yang dikerjakan dua
        // kali.
        let antrean: Vec<ScopeId> = if self.mounted {
            self.host.runtime.drain_dirty()
        } else {
            self.mounted = true;
            vec![self.root]
        };

        // 2. Rebuild + diff per scope.
        let mut diff = DiffStats::default();
        let mut rebuilt = 0usize;
        for scope in antrean {
            if let Some(stat) = self.rebuild_scope(scope) {
                diff.merge(stat);
                rebuilt += 1;
            }
        }
        if diff.removed > 0 {
            self.kumpulkan_sampah();
        }

        // 3. Layout: penuh bila constraints berubah atau akar kotor, selebihnya
        //    hanya boundary yang kotor.
        let relayouts = self.tree.pending_boundaries();
        let size = self.tree.perform_layout(self.constraints);

        // 4. Paint ke buffer yang dipakai ulang antar frame.
        self.tree.paint_into(&mut self.scene);

        // Tanda dirty pohon sudah dilayani frame ini juga — kalau ia dibiarkan
        // menumpuk, frame berikutnya akan dijadwalkan tanpa sebab dan "idle =
        // nol" berhenti berlaku.
        //
        // Satu-satunya yang **tidak** selesai di frame ini adalah
        // [`Dirty::ANIMATION`]: sebuah spring yang baru diarahkan oleh
        // view-diff (props `open` sebuah dialog berubah, tombol masuk keadaan
        // loading) belum bergerak sama sekali — ia baru akan bergerak di
        // `animate` frame berikutnya. Membuangnya di sini berarti animasinya
        // membeku sampai ada event input berikutnya, dan itu pernah terjadi.
        let sisa = self.tree.take_dirty();
        if sisa.contains(Dirty::ANIMATION) || self.anim.is_animating() {
            self.request(Dirty::ANIMATION);
        }

        start.mark_built(Instant::now());
        let timing = self
            .host
            .scheduler
            .borrow_mut()
            .end_frame(start, Instant::now(), true);

        FrameReport {
            index: timing.index,
            reason: timing.reason,
            rebuilt,
            diff,
            relayouts,
            size,
            timing,
        }
    }

    /// Bangun ulang satu scope dan diff hasilnya ke jangkarnya.
    ///
    /// `None` bila scope-nya sudah mati atau jangkarnya sudah tidak ada —
    /// keduanya normal terjadi saat sebuah daftar menyusut pada frame yang sama.
    fn rebuild_scope(&mut self, scope: ScopeId) -> Option<DiffStats> {
        let (builder, anchor) = {
            let reg = self.host.reg.borrow();
            (
                reg.builders.get(&scope).cloned()?,
                reg.anchors.get(&scope).copied()?,
            )
        };
        if !self.tree.contains(anchor) {
            return None;
        }

        let cx = BuildCtx::new(self.host.clone());
        let view = {
            // Host dipasang selama build supaya `component()` — yang dipanggil
            // di tengah closure pengguna — bisa menemukannya.
            let _g = HostGuard::push(self.host.clone());
            if scope == self.root {
                Some(self.host.runtime.build_root(|| builder(&cx)))
            } else {
                self.host.runtime.rebuild(scope, || builder(&cx))
            }
        }?;

        let stats = reconcile_children(&mut self.tree, anchor, std::slice::from_ref(&view));
        // Node jangkar komponen di dalam subtree ini bisa saja baru dibuat atau
        // diganti; petanya diperbarui dari pohon yang sebenarnya, bukan ditebak.
        self.segarkan_jangkar(anchor);
        Some(stats)
    }

    /// Catat ulang `scope → NodeId` untuk setiap komponen di dalam subtree.
    fn segarkan_jangkar(&self, from: NodeId) {
        let mut tumpukan = vec![from];
        let mut reg = self.host.reg.borrow_mut();
        while let Some(id) = tumpukan.pop() {
            if let Some(node) = self.tree.render(id) {
                if let Some(k) = node.downcast_ref::<ComponentBox>() {
                    reg.anchors.insert(k.scope, id);
                }
            }
            tumpukan.extend_from_slice(self.tree.children(id));
        }
    }

    /// Buang entri milik scope yang sudah mati.
    ///
    /// Hanya dijalankan pada frame yang benar-benar membuang node, jadi daftar
    /// yang stabil tidak membayar apa pun.
    fn kumpulkan_sampah(&self) {
        let rt = &self.host.runtime;
        let mut reg = self.host.reg.borrow_mut();
        reg.builders.retain(|s, _| rt.is_scope_alive(*s));
        reg.anchors
            .retain(|s, id| rt.is_scope_alive(*s) && self.tree.contains(*id));
    }
}

impl core::fmt::Debug for AppRuntime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AppRuntime")
            .field("runtime", &self.host.runtime)
            .field("nodes", &self.tree.len())
            .field("komponen", &self.host.reg.borrow().builders.len())
            .field("idle", &self.is_idle())
            .finish()
    }
}
