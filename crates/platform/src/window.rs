//! Shell window: konstruktor gaya Dart + event loop winit + surface wgpu.
//!
//! Bentuk API mengikuti REKOMENDASI §2.5 — fungsi konstruktor lalu method
//! chaining, tanpa struct literal dan tanpa macro:
//!
//! ```no_run
//! use silka_platform::window;
//! use silka_theme::{Appearance, Theme};
//!
//! window("Gallery")
//!     .size(1024.0, 720.0)
//!     .theme(Theme::cupertino(Appearance::Dark))
//!     .run()
//!     .unwrap();
//! ```

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use silka_core::access::{AccessActionRequest, AccessTree};
use silka_core::animation::Tick;
use silka_core::app::{AppRuntime, BuildCtx, ScaleFactor};
use silka_core::input::{Event as InputEvent, ImeRequest, Response as InputResponse};
use silka_core::scheduler::{Dirty, FrameLogger, FrameScheduler, Vsync, Wake};
use silka_core::signals::Signal;
use silka_core::tree::RenderTree;
use silka_core::view::View;
use silka_paint::{Color, GlyphSource, Scene, Size};
use silka_renderer::{FrameOutcome, Gpu, SurfaceGeometry, WindowSurface};
use silka_theme::{Appearance, Preset, Theme};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use crate::access::{AccessAdapter, AccessEvent, AccessOutcome};
use crate::appearance::{appearance_from_winit, winit_theme_from_appearance, AppearanceSource};
use crate::error::PlatformError;
use crate::input::{cursor_to_winit, ime_area_to_winit, WinitInput};
use crate::vsync::VsyncSource;

/// Informasi satu frame yang diberikan ke pembangun scene.
///
/// Semua ukuran dalam **poin logis** — DPI sudah diselesaikan di lapisan
/// surface, jadi kode di atas sini tidak pernah berurusan dengan piksel fisik.
#[derive(Debug, Clone, Copy)]
pub struct FrameContext<'a> {
    theme: &'a Theme,
    size: Size,
    scale_factor: f64,
    frame: u64,
    elapsed: Duration,
    vsync: Vsync,
    animate: &'a Cell<bool>,
}

impl<'a> FrameContext<'a> {
    /// Theme aktif — sumber satu-satunya untuk warna, radius, dan spacing.
    pub fn theme(&self) -> &'a Theme {
        self.theme
    }

    /// Ukuran area gambar dalam poin logis.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Scale factor window (2.0 di layar Retina).
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// Nomor frame sejak window dibuka.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Waktu sejak window dibuka — dasar animasi sebelum sistem spring ada.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Detak layar yang sedang berlaku.
    ///
    /// Di macOS ini datang dari `CADisplayLink` dan **ikut ProMotion**; di OS
    /// lain ditaksir dari frame yang benar-benar terjadi. Bisa
    /// [`Vsync::UNKNOWN`] pada frame-frame pertama — jangan menggantinya
    /// dengan tebakan.
    pub fn vsync(&self) -> Vsync {
        self.vsync
    }

    /// Minta satu frame lagi setelah frame ini.
    ///
    /// Inilah satu-satunya cara animasi berjalan: selama ada yang memanggil
    /// ini, renderer tetap berdetak; begitu tidak ada lagi, window kembali
    /// benar-benar idle (REKOMENDASI §3.5). Spring yang belum selesai
    /// memanggilnya tiap frame, lalu berhenti sendiri saat mencapai target.
    pub fn request_animation_frame(&self) {
        self.animate.set(true);
    }
}

type SceneFn = Box<dyn FnMut(&FrameContext<'_>) -> Scene>;

/// Penanggap event input.
///
/// Bentuknya sengaja `Event -> Response`: aplikasi (atau lapisan widget di
/// atasnya) meneruskan event ke [`silka_core::input::InputRouter`] dan
/// mengembalikan apa adanya. Shell lalu menerjemahkan hasilnya menjadi
/// panggilan winit — `request_redraw`, `set_ime_cursor_area`, `set_cursor` —
/// sehingga tidak ada satu pun tipe winit yang perlu dilihat kode di atasnya.
type InputFn = Box<dyn FnMut(&InputEvent) -> InputResponse>;

/// Pembangun pohon aksesibilitas satu window.
///
/// Dipanggil **hanya** saat ada teknologi bantu yang mendengarkan — pengguna
/// yang tidak memakai screen reader tidak membayar pass-nya sama sekali.
type AccessFn = Box<dyn FnMut() -> AccessTree>;

/// Penanggap permintaan aksi dari teknologi bantu.
type AccessActionFn = Box<dyn FnMut(AccessActionRequest)>;

/// Sumber atlas glyph yang dipakai bersama pembangun scene.
///
/// Dibagikan lewat `Rc<RefCell<…>>` karena dua pihak memakainya bergantian di
/// thread yang sama: closure `on_frame` saat menyusun scene (merasterisasi
/// glyph baru ke atlas), lalu backend saat menggambar (mengunggah bagian
/// atlas yang berubah). Keduanya tidak pernah berjalan bersamaan, jadi tidak
/// ada biaya sinkronisasi — dan `silka-platform` tetap tidak tahu apa itu
/// font: yang dipegangnya hanyalah trait dari `silka-paint`.
type GlyphsRef = Rc<RefCell<dyn GlyphSource>>;

/// Konfigurasi window, dibangun dengan method chaining.
///
/// Dibuat lewat [`window`].
pub struct WindowConfig {
    title: String,
    size: Size,
    min_size: Option<Size>,
    resizable: bool,
    theme: Theme,
    appearance_source: AppearanceSource,
    scene_fn: Option<SceneFn>,
    glyphs: Option<GlyphsRef>,
    access_fn: Option<AccessFn>,
    access_action_fn: Option<AccessActionFn>,
    input_fn: Option<InputFn>,
    frame_log_every: u64,
}

/// Buat window baru dengan judul tertentu.
///
/// Nilai bawaan: 1024×720 poin, bisa di-resize, preset Cupertino, dan
/// appearance mengikuti OS.
pub fn window(title: impl Into<String>) -> WindowConfig {
    WindowConfig {
        title: title.into(),
        size: Size::new(1024.0, 720.0),
        min_size: Some(Size::new(480.0, 360.0)),
        resizable: true,
        theme: Theme::default(),
        appearance_source: AppearanceSource::System,
        scene_fn: None,
        glyphs: None,
        access_fn: None,
        access_action_fn: None,
        input_fn: None,
        frame_log_every: DEFAULT_FRAME_LOG_EVERY,
    }
}

/// Tiap berapa frame ringkasan frame time dicetak di debug build.
const DEFAULT_FRAME_LOG_EVERY: u64 = 120;

impl WindowConfig {
    /// Ukuran awal window dalam poin logis.
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = Size::new(width, height);
        self
    }

    /// Ukuran minimum window dalam poin logis.
    pub fn min_size(mut self, width: f32, height: f32) -> Self {
        self.min_size = Some(Size::new(width, height));
        self
    }

    /// Boleh di-resize atau tidak.
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Theme lengkap (preset + appearance).
    ///
    /// Menyetel theme dengan appearance eksplisit **mengunci** appearance:
    /// perubahan dark mode OS tidak lagi diikuti. Panggil
    /// [`WindowConfig::follow_system_appearance`] untuk mengembalikannya.
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self.appearance_source = AppearanceSource::Locked;
        self
    }

    /// Ganti preset saja, appearance tetap mengikuti sumber saat ini.
    pub fn preset(mut self, preset: Preset) -> Self {
        self.theme = self.theme.with_preset(preset);
        self
    }

    /// Kunci appearance ke nilai tertentu.
    pub fn appearance(mut self, appearance: Appearance) -> Self {
        self.theme = self.theme.with_appearance(appearance);
        self.appearance_source = AppearanceSource::Locked;
        self
    }

    /// Ikuti dark mode OS secara live (INTEGRASI-NATIVE §6).
    pub fn follow_system_appearance(mut self) -> Self {
        self.appearance_source = AppearanceSource::System;
        self
    }

    /// Pembangun scene per frame.
    ///
    /// Tanpa ini, window digambar dengan warna token `background` dari theme
    /// aktif — cukup untuk membuktikan jalur window → surface → token bekerja.
    pub fn on_frame(mut self, scene_fn: impl FnMut(&FrameContext<'_>) -> Scene + 'static) -> Self {
        self.scene_fn = Some(Box::new(scene_fn));
        self
    }

    /// Sumber atlas glyph untuk perintah teks.
    ///
    /// Tanpa ini, perintah `GlyphRun` di dalam scene **tidak menghasilkan
    /// piksel** — backend tidak punya bitmap untuk digambar. Biasanya yang
    /// diserahkan adalah `silka_text::TextEngine` yang sama dengan yang
    /// dipakai `on_frame`:
    ///
    /// ```no_run
    /// # use std::cell::RefCell;
    /// # use std::rc::Rc;
    /// # use silka_platform::window;
    /// # use silka_paint::{Color, Scene};
    /// # struct TextEngine;
    /// # impl silka_paint::GlyphSource for TextEngine {
    /// #   fn atlas_size(&self, _: silka_paint::GlyphFormat) -> u32 { 0 }
    /// #   fn atlas_pixels(&self, _: silka_paint::GlyphFormat) -> &[u8] { &[] }
    /// #   fn take_dirty(&mut self, _: silka_paint::GlyphFormat) -> Option<silka_paint::AtlasRegion> { None }
    /// #   fn placement(&self, _: silka_paint::GlyphImageId) -> Option<silka_paint::GlyphPlacement> { None }
    /// # }
    /// let mesin = Rc::new(RefCell::new(TextEngine));
    /// let untuk_scene = mesin.clone();
    /// window("Aplikasi")
    ///     .on_frame(move |ctx| {
    ///         let mut mesin = untuk_scene.borrow_mut();
    ///         // … mesin.draw(&mut scene, …) …
    ///         Scene::new(ctx.theme().color.background)
    ///     })
    ///     .glyphs(mesin);
    /// ```
    ///
    /// Kontraknya tetap dijaga: yang menyeberang hanyalah trait
    /// `silka_paint::GlyphSource` — shell tidak tahu apa itu font, dan
    /// backend tidak tahu apa itu winit.
    pub fn glyphs<G: GlyphSource + 'static>(mut self, glyphs: Rc<RefCell<G>>) -> Self {
        self.glyphs = Some(glyphs as GlyphsRef);
        self
    }

    /// Pembangun pohon aksesibilitas (§3.8).
    ///
    /// Biasanya `move || tree.access_tree(router.focus().focused())`. Tanpa
    /// ini, window tetap **terlihat** oleh screen reader — dengan judulnya
    /// sebagai nama — hanya isinya yang kosong; aplikasi tidak pernah buta
    /// total seperti GPUI/Floem/Makepad (§7.2).
    ///
    /// Closure hanya dipanggil saat ada teknologi bantu yang aktif.
    pub fn on_access(mut self, access_fn: impl FnMut() -> AccessTree + 'static) -> Self {
        self.access_fn = Some(Box::new(access_fn));
        self
    }

    /// Penanggap permintaan aksi dari teknologi bantu (klik VoiceOver, dst.).
    ///
    /// Permintaannya sudah divalidasi terhadap pohon yang benar-benar dikirim:
    /// node yang sudah mati atau aksi yang tidak diumumkan tidak pernah sampai
    /// ke sini.
    pub fn on_access_action(
        mut self,
        action_fn: impl FnMut(AccessActionRequest) + 'static,
    ) -> Self {
        self.access_action_fn = Some(Box::new(action_fn));
        self
    }

    /// Penanggap event input (pointer, keyboard, guliran, IME).
    ///
    /// Event datang dalam kosakata framework — tidak ada tipe winit yang
    /// menyeberang (§3.2 diterapkan ke input). Jalur normalnya satu baris:
    ///
    /// ```no_run
    /// # use silka_platform::window;
    /// # use silka_core::input::InputRouter;
    /// # use silka_core::tree::RenderTree;
    /// # use std::cell::RefCell;
    /// # use std::rc::Rc;
    /// let tree = Rc::new(RefCell::new(RenderTree::new()));
    /// let router = Rc::new(RefCell::new(InputRouter::new()));
    /// window("Aplikasi")
    ///     .on_input(move |event| router.borrow_mut().dispatch(&mut tree.borrow_mut(), event))
    ///     .run()
    ///     .unwrap();
    /// ```
    ///
    /// Yang dikembalikan menentukan apa yang dilakukan shell berikutnya:
    /// [`silka_core::input::Response::dirty`] membangunkan renderer (dan
    /// **hanya** itu yang membangunkannya — §3.5), `ime` diterjemahkan menjadi
    /// `set_ime_allowed`/`set_ime_cursor_area`, dan `cursor` menjadi
    /// `set_cursor`.
    pub fn on_input(
        mut self,
        input_fn: impl FnMut(&InputEvent) -> InputResponse + 'static,
    ) -> Self {
        self.input_fn = Some(Box::new(input_fn));
        self
    }

    /// Interval ringkasan frame time di debug build.
    ///
    /// `0` mematikan ringkasan berkala; frame yang melewati budget vsync tetap
    /// dicatat. Di release build pengukurannya tetap jalan (murah) tapi tidak
    /// ada yang dicetak.
    pub fn frame_log_every(mut self, frames: u64) -> Self {
        self.frame_log_every = frames;
        self
    }

    /// Buka window dan jalankan event loop sampai window ditutup.
    ///
    /// Event loop memakai [`ControlFlow::Wait`]: **tidak ada** loop yang
    /// berputar terus saat idle. Frame hanya digambar ketika OS meminta
    /// redraw atau ketika sesuatu menandai dirty (REKOMENDASI §3.5).
    pub fn run(self) -> Result<(), PlatformError> {
        // Event loop membawa *user event*: itulah jalur balik aksesibilitas
        // dari thread OS mana pun ke UI thread (§3.8).
        let event_loop = EventLoop::<AccessEvent>::with_user_event()
            .build()
            .map_err(|e| PlatformError::EventLoop(e.to_string()))?;
        let proxy = event_loop.create_proxy();
        let mut shell = Shell::new(self, proxy);
        event_loop
            .run_app(&mut shell)
            .map_err(|e| PlatformError::EventLoop(e.to_string()))?;
        match shell.error.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// Buka window dan jalankan aplikasi reaktif di dalamnya — **API menjalankan
/// aplikasi** (REKOMENDASI §2.5).
///
/// Inilah bentuk yang dilihat penulis aplikasi: sebuah window, sebuah closure
/// yang mengembalikan pohon view, dan tidak ada satu pun jahitan di antaranya
/// yang perlu dirakit sendiri.
///
/// ```no_run
/// use silka_platform::{run_app, window};
/// use silka_core::app::component;
/// use silka_core::signals::use_signal;
/// use silka_core::view::{column, fixed};
///
/// run_app(window("Hitung").size(480.0, 320.0), |_cx| {
///     let count = use_signal(|| 0i32);
///     column([component("angka", move |_| {
///         fixed(120.0, 20.0 + count.get() as f32).into()
///     })])
///     .into()
/// })
/// .unwrap();
/// ```
///
/// Yang dirakit fungsi ini, dan alasannya:
///
/// - **`on_frame` menghasilkan scene dari siklus hidup**, bukan dari scene yang
///   disusun tangan: `AppRuntime::frame()` menjalankan rebuild → diff → layout
///   → paint, dan yang menyeberang ke backend tetap hanya
///   [`silka_paint::Scene`] (§3.2).
/// - **`on_input` menyalurkan event ke pohon yang sama** dan mengembalikan
///   alasan dirty-nya — termasuk dirty yang lahir dari tulisan signal di dalam
///   handler.
/// - **`on_access` menyusun pohon a11y dari geometri frame yang sama**, dengan
///   fokus dari router (§3.8).
/// - **Theme dititipkan sebagai `Signal<Theme>`** di [`silka_core::app::Env`]:
///   dark mode OS yang berubah menulis signal itu, dan **hanya** komponen yang
///   benar-benar membaca theme yang ikut dibangun ulang (§2.7).
///
/// Janji "render hanya saat dirty" tetap utuh: setelah frame selesai, shell
/// hanya meminta frame berikutnya bila
/// [`silka_core::app::AppRuntime::is_idle`] bernilai salah.
///
/// [`WindowConfig::on_frame`], [`WindowConfig::on_input`], dan
/// [`WindowConfig::on_access`] yang sudah dipasang di `config` **digantikan**
/// oleh fungsi ini.
pub fn run_app(
    config: WindowConfig,
    build: impl Fn(&BuildCtx) -> View + 'static,
) -> Result<(), PlatformError> {
    sambungkan_app(config, build).run()
}

/// [`run_app`] **dengan penggerak animasi** — bentuk yang dipakai aplikasi yang
/// memakai widget beranimasi.
///
/// `animate` dipanggil sekali per frame **sebelum** siklus rebuild → layout →
/// paint, dengan render tree dan [`Tick`] frame itu; nilainya yang kembali
/// adalah alasan dirty, dan selama ia masih menyebut
/// [`Dirty::ANIMATION`](silka_core::scheduler::Dirty::ANIMATION) shell terus
/// meminta frame berikutnya. Begitu semua spring settle, event loop kembali
/// menunggu — janji "render hanya saat dirty" (§3.5) tidak dilanggar oleh
/// keberadaan animasi.
///
/// Bentuk `animate` sengaja persis milik `silka_widgets::advance`, sehingga
/// aplikasi biasa cukup menulis:
///
/// ```no_run
/// # use silka_platform::{run_app_with, window};
/// # use silka_core::view::fixed;
/// # fn advance(_: &mut silka_core::tree::RenderTree, _: &silka_core::animation::Tick)
/// #     -> silka_core::scheduler::Dirty { silka_core::scheduler::Dirty::NONE }
/// run_app_with(window("Demo"), |_cx| fixed(80.0, 24.0).into(), advance).unwrap();
/// ```
pub fn run_app_with(
    config: WindowConfig,
    build: impl Fn(&BuildCtx) -> View + 'static,
    animate: impl FnMut(&mut RenderTree, &Tick) -> Dirty + 'static,
) -> Result<(), PlatformError> {
    sambungkan_app_with(config, build, animate).run()
}

/// [`AppRuntime`] yang **dirakit persis seperti [`run_app`]**, tanpa window dan
/// tanpa GPU (REKOMENDASI §9.5).
///
/// Inilah pintu masuk uji integrasi headless: halaman yang sama yang tampil di
/// window dijalankan di sini, diberi event input lewat
/// [`AppRuntime::dispatch`], lalu [`AppRuntime::scene`]-nya bisa dirender ke
/// tekstur offscreen dan dihitung pikselnya. Karena `run_app` sendiri memakai
/// fungsi ini, titipan [`Env`] yang dilihat aplikasi tidak mungkin berbeda
/// antara "di layar" dan "di CI".
///
/// Yang dititipkan sama persis dengan `run_app`:
///
/// - `Signal<Theme>` — dark mode/preset yang berubah hanya membangun ulang
///   komponen yang benar-benar membacanya (§2.7).
/// - `Signal<ScaleFactor>` — resolusi layar untuk rasterisasi teks (§3.3).
///
/// ```
/// use silka_platform::headless_app;
/// use silka_core::view::fixed;
/// use silka_theme::{Appearance, Theme};
///
/// let theme = Theme::cupertino(Appearance::Dark);
/// let mut ui = headless_app(theme, |_cx| fixed(120.0, 24.0).into()).sized(320.0, 200.0);
/// ui.frame();
/// assert_eq!(ui.scene().clear_color(), theme.color.background);
/// ```
pub fn headless_app(theme: Theme, build: impl Fn(&BuildCtx) -> View + 'static) -> AppRuntime {
    AppRuntime::new(build)
        .clear_color(theme.color.background)
        .with_env(move |rt| rt.signal(theme))
        // Nilai awal jujur: sebelum window ada, scale factor-nya memang belum
        // diketahui. Shell menimpanya di frame pertama.
        .with_env(|rt| rt.signal(ScaleFactor::ONE))
}

/// Bagian [`run_app`] yang tidak menyentuh event loop.
///
/// Dipisah supaya jahitannya bisa diuji headless: test memanggil `scene_fn`,
/// `input_fn`, dan `access_fn` yang terpasang di sini dengan
/// [`FrameContext`] buatan, tanpa window dan tanpa GPU.
fn sambungkan_app(
    config: WindowConfig,
    build: impl Fn(&BuildCtx) -> View + 'static,
) -> WindowConfig {
    sambungkan_app_with(config, build, |_, _| Dirty::NONE)
}

/// [`sambungkan_app`] dengan penggerak animasi (lihat [`run_app_with`]).
fn sambungkan_app_with(
    config: WindowConfig,
    build: impl Fn(&BuildCtx) -> View + 'static,
    mut animate: impl FnMut(&mut RenderTree, &Tick) -> Dirty + 'static,
) -> WindowConfig {
    let app = Rc::new(RefCell::new(headless_app(config.theme, build)));

    let untuk_frame = app.clone();
    let untuk_input = app.clone();
    let untuk_access = app;

    config
        .on_frame(move |ctx| {
            let mut ui = untuk_frame.borrow_mut();
            // Perubahan dari shell masuk lebih dulu supaya rebuild frame ini
            // sudah melihatnya — bukan satu frame kemudian.
            ui.resize(ctx.size());
            ui.set_clear_color(ctx.theme().color.background);
            if let Some(theme) = ui.env::<Signal<Theme>>() {
                theme.set_if_changed(*ctx.theme());
            }
            // Teks harus dirasterisasi pada resolusi layar yang sebenarnya
            // (§3.3); window yang pindah ke monitor lain menulis signal ini,
            // dan hanya komponen yang membacanya yang ikut dibangun ulang.
            if let Some(scale) = ui.env::<Signal<ScaleFactor>>() {
                scale.set_if_changed(ScaleFactor(ctx.scale_factor() as f32));
            }
            ui.set_vsync(ctx.vsync());

            // Spring dimajukan **sebelum** frame: nilai yang bergerak menjadi
            // nilai frame ini, bukan frame berikutnya (§3.5). `dt`-nya dihitung
            // dari jam sungguhan oleh `AnimationDriver`, tidak pernah dari
            // konstanta 16,6 ms.
            ui.animate(&mut animate);

            ui.frame();

            // Satu-satunya cara frame berikutnya terjadi: masih ada yang kotor
            // (spring yang belum settle, signal yang ditulis saat build).
            if !ui.is_idle() {
                ctx.request_animation_frame();
            }
            ui.scene().clone()
        })
        .on_input(move |event| untuk_input.borrow_mut().dispatch(event))
        .on_access(move || untuk_access.borrow().access_tree())
}

fn latar_dari_token(ctx: &FrameContext<'_>) -> Scene {
    Scene::new(ctx.theme().color.background)
}

/// Pohon a11y bawaan: satu node window bernama judul aplikasi.
///
/// Aplikasi yang belum menyambungkan render tree-nya tetap **terlihat** oleh
/// screen reader — window-nya punya nama dan bisa difokuskan. Buta total
/// (GPUI, Floem, Makepad — §7.2) bukan keadaan bawaan yang bisa terjadi di
/// sini.
fn pohon_window_saja(title: String) -> AccessFn {
    let mut tree = RenderTree::new();
    tree.set_root_label(title);
    Box::new(move || tree.access_tree(None))
}

struct ShellState {
    window: Arc<Window>,
    gpu: Gpu,
    surface: WindowSurface,
    vsync: VsyncSource,
    access: AccessAdapter,
}

struct Shell {
    title: String,
    size: Size,
    min_size: Option<Size>,
    resizable: bool,
    theme: Theme,
    appearance_source: AppearanceSource,
    scene_fn: SceneFn,
    glyphs: Option<GlyphsRef>,
    access_fn: AccessFn,
    access_action_fn: Option<AccessActionFn>,
    input_fn: Option<InputFn>,
    input: WinitInput,
    ime_aktif: bool,
    proxy: EventLoopProxy<AccessEvent>,
    state: Option<ShellState>,
    started: Instant,
    scheduler: FrameScheduler,
    logger: FrameLogger,
    error: Option<PlatformError>,
}

impl Shell {
    fn new(config: WindowConfig, proxy: EventLoopProxy<AccessEvent>) -> Self {
        let access_fn = config
            .access_fn
            .unwrap_or_else(|| pohon_window_saja(config.title.clone()));
        Self {
            title: config.title,
            size: config.size,
            min_size: config.min_size,
            resizable: config.resizable,
            theme: config.theme,
            appearance_source: config.appearance_source,
            scene_fn: config
                .scene_fn
                .unwrap_or_else(|| Box::new(latar_dari_token)),
            glyphs: config.glyphs,
            access_fn,
            access_action_fn: config.access_action_fn,
            input_fn: config.input_fn,
            input: WinitInput::new(),
            ime_aktif: false,
            proxy,
            state: None,
            started: Instant::now(),
            scheduler: FrameScheduler::new(),
            logger: FrameLogger::every(config.frame_log_every),
            error: None,
        }
    }

    /// Kirim pohon a11y ke adapter.
    ///
    /// Dipisah dari [`Shell::gambar`] supaya bisa dipanggil juga saat teknologi
    /// bantu meminta pohon awal — momen yang tidak selalu bersamaan dengan
    /// frame.
    fn kirim_a11y(&mut self, penuh: bool) {
        let Shell {
            state, access_fn, ..
        } = self;
        let Some(state) = state.as_mut() else { return };
        let scale = state.surface.scale_factor();
        if penuh {
            let pohon = (access_fn)();
            state.access.update_full(scale, pohon);
        } else {
            state.access.update_with(scale, access_fn);
        }
    }

    fn gagal(&mut self, event_loop: &ActiveEventLoop, error: PlatformError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
        event_loop.exit();
    }

    fn buat_window(&mut self, event_loop: &ActiveEventLoop) -> Result<(), PlatformError> {
        // Sengaja tersembunyi dulu: adapter aksesibilitas **wajib** terpasang
        // sebelum window pertama kali terlihat (§3.8). Window ditampilkan
        // setelah adapter dan surface siap.
        let mut attrs = Window::default_attributes()
            .with_title(self.title.clone())
            .with_resizable(self.resizable)
            .with_visible(false)
            .with_inner_size(LogicalSize::new(
                self.size.width as f64,
                self.size.height as f64,
            ));
        if let Some(min) = self.min_size {
            attrs =
                attrs.with_min_inner_size(LogicalSize::new(min.width as f64, min.height as f64));
        }
        if self.appearance_source == AppearanceSource::Locked {
            attrs = attrs.with_theme(Some(winit_theme_from_appearance(self.theme.appearance)));
        }

        let window = event_loop
            .create_window(attrs)
            .map_err(|e| PlatformError::WindowCreation(e.to_string()))?;
        let window = Arc::new(window);

        // Accessibility dari hari pertama, bukan retrofit (§3.8, §5 poin 2).
        let access = AccessAdapter::new(event_loop, &window, self.proxy.clone());

        // Appearance awal dari OS, sebelum frame pertama digambar.
        if self.appearance_source == AppearanceSource::System {
            if let Some(t) = window.theme() {
                self.theme = self.theme.with_appearance(appearance_from_winit(t));
            }
        }

        let PhysicalSize { width, height } = window.inner_size();
        // Input berbicara poin logis; pembagi DPI-nya diketahui dari sini.
        self.input.set_scale_factor(window.scale_factor());
        let geometry = SurfaceGeometry::new(width, height, window.scale_factor());
        let (gpu, surface) = Gpu::with_surface(window.clone(), geometry)?;

        // Sumber detak frame: CADisplayLink di macOS, `request_redraw` di OS
        // lain. Dipasang dalam keadaan diam — belum ada yang dirty.
        let vsync = VsyncSource::attach(window.clone());
        self.scheduler.set_vsync(vsync.vsync());

        #[cfg(debug_assertions)]
        eprintln!(
            "silka: window \"{}\" — backend {} pada {} ({}×{} px @ {}x) · vsync {} ({})",
            self.title,
            gpu.backend_name(),
            gpu.adapter_name(),
            width,
            height,
            geometry.scale_factor(),
            self.scheduler.vsync(),
            vsync.kind().label(),
        );

        // Semua sudah siap — baru sekarang window boleh terlihat.
        window.set_visible(true);

        self.state = Some(ShellState {
            window,
            gpu,
            surface,
            vsync,
            access,
        });

        // Frame pertama: satu-satunya frame yang tidak dipicu perubahan.
        self.minta(Dirty::LAYOUT | Dirty::PAINT);
        Ok(())
    }

    /// Salurkan satu event input ke aplikasi lalu jalankan permintaannya.
    ///
    /// Ini satu-satunya tempat hasil routing bertemu winit: dirty membangunkan
    /// renderer, permintaan IME menjadi `set_ime_allowed`/`set_ime_cursor_area`
    /// (jendela kandidat CJK berlabuh di caret, §3.8), dan kursor menjadi
    /// `set_cursor`.
    fn masukan(&mut self, event: InputEvent) {
        let Some(input_fn) = self.input_fn.as_mut() else {
            return;
        };
        let hasil = input_fn(&event);

        if let Some(state) = self.state.as_ref() {
            match hasil.ime {
                Some(ImeRequest::Enable { area }) => {
                    if !self.ime_aktif {
                        state.window.set_ime_allowed(true);
                        self.ime_aktif = true;
                    }
                    let (posisi, ukuran) = ime_area_to_winit(area);
                    state.window.set_ime_cursor_area(posisi, ukuran);
                }
                Some(ImeRequest::Update { area }) => {
                    let (posisi, ukuran) = ime_area_to_winit(area);
                    state.window.set_ime_cursor_area(posisi, ukuran);
                }
                Some(ImeRequest::Disable) if self.ime_aktif => {
                    state.window.set_ime_allowed(false);
                    self.ime_aktif = false;
                }
                // IME yang memang sudah mati tidak perlu dimatikan lagi.
                Some(ImeRequest::Disable) | None => {}
            }
            if let Some(cursor) = hasil.cursor {
                state.window.set_cursor(cursor_to_winit(cursor));
            }
        }

        if !hasil.dirty.is_empty() {
            self.minta(hasil.dirty);
        }
    }

    /// Tandai dirty dan — hanya bila memang perlu — bangunkan sumber vsync.
    fn minta(&mut self, dirty: Dirty) {
        if self.scheduler.request(dirty) == Wake::Schedule {
            if let Some(state) = self.state.as_ref() {
                state.vsync.schedule();
            }
        }
    }

    fn gambar(&mut self) -> Result<(), PlatformError> {
        let Shell {
            state,
            scheduler,
            scene_fn,
            glyphs,
            theme,
            started,
            logger,
            ..
        } = self;
        let Some(state) = state.as_mut() else {
            return Ok(());
        };

        // Interval yang dilaporkan OS bisa berubah kapan saja (ProMotion naik
        // turun, window pindah monitor) — dibaca ulang tiap frame.
        scheduler.set_vsync(state.vsync.vsync());

        let mut start = scheduler.begin_frame(Instant::now());
        let animate = Cell::new(false);
        let ctx = FrameContext {
            theme,
            size: state.surface.logical_size(),
            scale_factor: state.surface.scale_factor(),
            frame: start.index(),
            elapsed: started.elapsed(),
            vsync: scheduler.vsync(),
            animate: &animate,
        };
        let scene = (scene_fn)(&ctx);

        // Batas antara kerja kita dan antrean swapchain. Tanpa penanda ini,
        // waktu menunggu vsync akan tercatat sebagai "frame lambat" padahal ia
        // justru tanda sistem sedang sehat.
        start.mark_built(Instant::now());

        // Wayland ingin tahu sebelum buffer di-attach; no-op di platform lain.
        state.window.pre_present_notify();
        // Atlas glyph dipinjam HANYA selama menggambar — closure scene sudah
        // selesai memakainya, jadi tidak pernah ada dua peminjam sekaligus.
        let hasil = match glyphs {
            Some(g) => {
                let mut sumber = g.borrow_mut();
                state
                    .surface
                    .render_with_glyphs(&state.gpu, &scene, &mut *sumber)
            }
            None => state.surface.render(&state.gpu, &scene),
        };

        // Frame ditutup lebih dulu, apa pun hasilnya: statistik frame yang
        // gagal justru yang paling menarik saat menyelidiki jank.
        let timing = scheduler.end_frame(
            start,
            Instant::now(),
            matches!(hasil, Ok(FrameOutcome::Presented)),
        );

        // Pengukuran selalu jalan; pencetakannya hanya di debug build.
        #[cfg(debug_assertions)]
        if let Some(line) = logger.line(scheduler.stats(), scheduler.vsync(), &timing) {
            eprintln!("{line}");
        }
        #[cfg(not(debug_assertions))]
        let _ = (logger, &timing);

        hasil?;

        // Pohon a11y disusun ulang setelah frame, dari geometri frame itu
        // juga — dan **hanya** kalau ada teknologi bantu yang mendengarkan.
        self.kirim_a11y(false);

        if animate.get() {
            self.minta(Dirty::ANIMATION);
        }

        // Tidak ada sisa pekerjaan → hentikan detak. Inilah yang membuat idle
        // benar-benar idle, bukan sekadar "menggambar frame kosong".
        if self.scheduler.is_idle() {
            if let Some(state) = self.state.as_ref() {
                state.vsync.idle();
            }
        }
        Ok(())
    }

    /// Ubah visibilitas window (occlusion/minimize) tanpa menggambar sia-sia.
    fn set_terlihat(&mut self, terlihat: bool) {
        if self.scheduler.set_visible(terlihat) == Wake::Schedule {
            if let Some(state) = self.state.as_ref() {
                state.vsync.schedule();
            }
        } else if !terlihat {
            if let Some(state) = self.state.as_ref() {
                state.vsync.idle();
            }
        }
    }
}

impl ApplicationHandler<AccessEvent> for Shell {
    /// Jalur balik dari teknologi bantu.
    ///
    /// `accesskit_winit` memanggil handler-nya di thread mana pun; event loop
    /// winit adalah kanal resmi untuk kembali ke UI thread.
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AccessEvent) {
        let cocok = self
            .state
            .as_ref()
            .is_some_and(|s| s.window.id() == event.window_id());
        if !cocok {
            return;
        }
        let hasil = match self.state.as_mut() {
            Some(state) => state.access.handle(&event),
            None => return,
        };
        match hasil {
            // Screen reader baru dinyalakan: ia tidak punya riwayat apa pun,
            // jadi yang dikirim harus pohon lengkap.
            AccessOutcome::NeedsFullTree => self.kirim_a11y(true),
            AccessOutcome::Action(request) => {
                if let Some(f) = self.access_action_fn.as_mut() {
                    f(request);
                    // Aksi dari teknologi bantu adalah input seperti klik
                    // mouse: apa pun yang berubah karenanya harus digambar.
                    self.minta(Dirty::PAINT);
                }
            }
            AccessOutcome::Idle => {}
        }
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, _cause: StartCause) {
        // Idle harus benar-benar idle: tidak ada polling, tidak ada timer.
        event_loop.set_control_flow(ControlFlow::Wait);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        if let Err(e) = self.buat_window(event_loop) {
            self.gagal(event_loop, e);
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        // Surface tidak valid selama suspend (aturan Android; tidak berbahaya
        // di desktop). Dibangun ulang di `resumed` berikutnya.
        self.state = None;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let cocok = self
            .state
            .as_ref()
            .is_some_and(|s| s.window.id() == window_id);
        if !cocok {
            return;
        }

        // Adapter a11y melihat event **sebelum** shell memprosesnya: fokus
        // window dan geometri ikut dari sini.
        if let Some(state) = self.state.as_mut() {
            let window = state.window.clone();
            state.access.process_event(&window, &event);
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(PhysicalSize { width, height }) => {
                if let Some(state) = self.state.as_mut() {
                    state.surface.resize(&state.gpu, width, height);
                }
                // Window diminimalkan datang sebagai ukuran 0×0. Tanpa ini,
                // animasi yang meminta frame berikutnya akan berputar tanpa
                // henti menggambar ke surface yang tidak bisa digambar.
                let bisa_digambar = self
                    .state
                    .as_ref()
                    .is_some_and(|s| s.surface.geometry().is_renderable());
                self.set_terlihat(bisa_digambar);
                if bisa_digambar {
                    self.minta(Dirty::SURFACE | Dirty::LAYOUT);
                }
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // winit menyusulkan `Resized` dengan ukuran fisik baru; di sini
                // cukup memperbarui pembagi poin-logis.
                if let Some(state) = self.state.as_mut() {
                    state.surface.set_scale_factor(scale_factor);
                }
                self.input.set_scale_factor(scale_factor);
                // Monitor baru bisa punya laju berbeda — taksiran lama batal.
                self.scheduler.reset_vsync_estimate();
                self.minta(Dirty::SURFACE | Dirty::LAYOUT);
            }

            WindowEvent::ThemeChanged(theme) => {
                if let Some(baru) = crate::appearance::apply_system_appearance(
                    self.theme,
                    self.appearance_source,
                    appearance_from_winit(theme),
                ) {
                    self.theme = baru;
                    self.minta(Dirty::THEME);
                }
            }

            // Window tertutup total: jangan bakar GPU untuk piksel yang tak
            // pernah dilihat siapa pun.
            WindowEvent::Occluded(occluded) => self.set_terlihat(!occluded),

            // -- input (INTEGRASI-NATIVE §3) ---------------------------------
            WindowEvent::ModifiersChanged(modifiers) => self.input.modifiers_changed(modifiers),

            WindowEvent::CursorMoved { position, .. } => {
                let e = self.input.cursor_moved(position);
                self.masukan(e);
            }

            // `CursorEntered` tidak membawa koordinat; `Enter` yang berguna
            // disintesis dari `CursorMoved` pertama.
            WindowEvent::CursorEntered { .. } => {}

            WindowEvent::CursorLeft { .. } => {
                if let Some(e) = self.input.cursor_left() {
                    self.masukan(e);
                }
            }

            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(e) = self.input.mouse_input(state, button) {
                    self.masukan(e);
                }
            }

            WindowEvent::MouseWheel { delta, phase, .. } => {
                if let Some(e) = self.input.mouse_wheel(delta, phase) {
                    self.masukan(e);
                }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                let e = self.input.keyboard_input(&event);
                self.masukan(e);
            }

            WindowEvent::Ime(ime) => {
                let e = self.input.ime(ime);
                self.masukan(e);
            }

            // Window kehilangan fokus: interaksi yang sedang berjalan
            // **dibatalkan**, bukan diselesaikan — tombol yang ditekan lalu
            // ditinggal tidak boleh menghasilkan klik.
            WindowEvent::Focused(false) => {
                if let Some(e) = self.input.cancel() {
                    self.masukan(e);
                }
            }

            WindowEvent::RedrawRequested => {
                if let Err(e) = self.gambar() {
                    self.gagal(event_loop, e);
                }
            }

            _ => {}
        }
    }
}

/// Warna latar bawaan untuk theme tertentu — jalur yang sama yang dipakai
/// shell bila aplikasi tidak menyediakan [`WindowConfig::on_frame`].
///
/// Diekspos supaya test dan tooling headless bisa memverifikasi bahwa clear
/// color memang datang dari token, bukan dari literal.
pub fn default_clear_color(theme: &Theme) -> Color {
    theme.color.background
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::AccessRole;

    /// Sumber atlas palsu — cukup untuk membuktikan jalurnya terpasang, tanpa
    /// menyeret stack text ke dalam uji shell.
    #[derive(Default)]
    struct AtlasPalsu {
        diminta: std::cell::Cell<u32>,
    }

    impl GlyphSource for AtlasPalsu {
        fn atlas_size(&self, _format: silka_paint::GlyphFormat) -> u32 {
            self.diminta.set(self.diminta.get() + 1);
            0
        }

        fn atlas_pixels(&self, _format: silka_paint::GlyphFormat) -> &[u8] {
            &[]
        }

        fn take_dirty(
            &mut self,
            _format: silka_paint::GlyphFormat,
        ) -> Option<silka_paint::AtlasRegion> {
            None
        }

        fn placement(
            &self,
            _image: silka_paint::GlyphImageId,
        ) -> Option<silka_paint::GlyphPlacement> {
            None
        }
    }

    #[test]
    fn tanpa_glyphs_window_tetap_bisa_dibangun() {
        // Aplikasi tanpa teks tidak membayar apa pun: tidak ada sumber atlas,
        // dan `render` biasa yang dipakai.
        assert!(window("Tanpa teks").glyphs.is_none());
    }

    #[test]
    fn sumber_atlas_terpasang_lewat_method_chaining() {
        let atlas = Rc::new(RefCell::new(AtlasPalsu::default()));
        let config = window("Dengan teks").glyphs(atlas.clone());
        let terpasang = config.glyphs.expect("sumber atlas tersimpan");
        // Objek yang sama, bukan salinan: atlas yang diisi closure scene
        // persis yang dibaca backend.
        assert!(Rc::ptr_eq(
            &(atlas as Rc<RefCell<dyn GlyphSource>>),
            &terpasang
        ));
    }

    #[test]
    fn pohon_a11y_bawaan_menyebut_judul_window() {
        let mut bangun = pohon_window_saja("Laporan".into());
        let a11y = bangun();
        assert_eq!(a11y.len(), 1, "hanya node window");
        let root = a11y.get(a11y.root()).expect("akar selalu ada");
        assert_eq!(root.node.role, AccessRole::Window);
        assert_eq!(
            root.node.label.as_deref(),
            Some("Laporan"),
            "screen reader harus bisa menyebut nama aplikasinya"
        );
        assert_eq!(a11y.focus(), a11y.root());
    }

    #[test]
    fn on_access_menggantikan_pohon_bawaan() {
        let c = window("Uji").on_access(|| RenderTree::new().access_tree(None));
        assert!(c.access_fn.is_some());
        assert!(window("Uji").access_fn.is_none());
    }

    #[test]
    fn nilai_bawaan_window_masuk_akal() {
        let c = window("Uji");
        assert_eq!(c.title, "Uji");
        assert_eq!(c.size, Size::new(1024.0, 720.0));
        assert!(c.resizable);
        assert_eq!(c.appearance_source, AppearanceSource::System);
        assert_eq!(c.theme.preset, Preset::Cupertino);
        assert!(c.scene_fn.is_none());
    }

    #[test]
    fn chaining_mengubah_hanya_yang_disebut() {
        let c = window("Uji").size(800.0, 600.0).min_size(320.0, 240.0);
        assert_eq!(c.size, Size::new(800.0, 600.0));
        assert_eq!(c.min_size, Some(Size::new(320.0, 240.0)));
        assert!(c.resizable);
    }

    #[test]
    fn menyetel_theme_mengunci_appearance() {
        let c = window("Uji").theme(Theme::tailwind(Appearance::Dark));
        assert_eq!(c.appearance_source, AppearanceSource::Locked);
        assert_eq!(c.theme.appearance, Appearance::Dark);
    }

    #[test]
    fn follow_system_membuka_kunci_lagi() {
        let c = window("Uji")
            .appearance(Appearance::Dark)
            .follow_system_appearance();
        assert_eq!(c.appearance_source, AppearanceSource::System);
    }

    #[test]
    fn ganti_preset_tidak_mengunci_appearance() {
        let c = window("Uji").preset(Preset::Tailwind);
        assert_eq!(c.theme.preset, Preset::Tailwind);
        assert_eq!(c.appearance_source, AppearanceSource::System);
    }

    #[test]
    fn clear_color_bawaan_adalah_token_background() {
        for theme in [
            Theme::cupertino(Appearance::Light),
            Theme::cupertino(Appearance::Dark),
            Theme::tailwind(Appearance::Light),
            Theme::tailwind(Appearance::Dark),
        ] {
            assert_eq!(default_clear_color(&theme), theme.color.background);
        }
    }

    #[test]
    fn scene_bawaan_memakai_token_background() {
        let theme = Theme::cupertino(Appearance::Dark);
        let animate = Cell::new(false);
        let ctx = FrameContext {
            theme: &theme,
            size: Size::new(1024.0, 720.0),
            scale_factor: 2.0,
            frame: 0,
            elapsed: Duration::ZERO,
            vsync: Vsync::UNKNOWN,
            animate: &animate,
        };
        let scene = latar_dari_token(&ctx);
        assert_eq!(scene.clear_color(), theme.color.background);
        assert!(scene.is_empty());
    }

    #[test]
    fn on_frame_menggantikan_scene_bawaan() {
        let mut c = window("Uji").on_frame(|ctx| Scene::new(ctx.theme().color.accent));
        let f = c.scene_fn.as_mut().expect("scene_fn terpasang");
        let theme = Theme::tailwind(Appearance::Light);
        let animate = Cell::new(false);
        let ctx = FrameContext {
            theme: &theme,
            size: Size::new(10.0, 10.0),
            scale_factor: 1.0,
            frame: 7,
            elapsed: Duration::from_millis(120),
            vsync: Vsync::UNKNOWN,
            animate: &animate,
        };
        assert_eq!(f(&ctx).clear_color(), theme.color.accent);
        assert_eq!(ctx.frame(), 7);
        assert_eq!(ctx.elapsed(), Duration::from_millis(120));
        assert_eq!(ctx.size(), Size::new(10.0, 10.0));
        assert_eq!(ctx.scale_factor(), 1.0);
    }

    #[test]
    fn tanpa_on_input_event_tidak_ke_mana_mana() {
        // Window yang tidak memasang penanggap input tetap sah — ia hanya
        // tidak pernah dibangunkan oleh input.
        assert!(window("Uji").input_fn.is_none());
    }

    #[test]
    fn on_input_menerima_event_dan_hasilnya_dipakai() {
        use silka_core::input::{Event, PointerEvent, PointerPhase};
        use silka_paint::Point;

        let mut c = window("Uji").on_input(|event| {
            let mut hasil = InputResponse::default();
            if matches!(event, Event::Pointer(_)) {
                hasil.dirty |= Dirty::PAINT;
                hasil.handled = true;
            }
            hasil
        });
        let f = c.input_fn.as_mut().expect("input_fn terpasang");
        let hasil = f(&Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            Point::new(4.0, 8.0),
            Duration::ZERO,
        )));
        assert!(hasil.handled);
        assert!(hasil.dirty.contains(Dirty::PAINT));
    }

    /// Kerangka [`FrameContext`] untuk test — satu-satunya bagian frame yang
    /// tidak bisa dibuat tanpa window.
    fn frame_ctx<'a>(theme: &'a Theme, animate: &'a Cell<bool>) -> FrameContext<'a> {
        FrameContext {
            theme,
            size: Size::new(320.0, 240.0),
            scale_factor: 2.0,
            frame: 0,
            elapsed: Duration::ZERO,
            vsync: Vsync::UNKNOWN,
            animate,
        }
    }

    #[test]
    fn run_app_menyusun_scene_dari_siklus_hidup_bukan_dari_tangan() {
        use silka_core::app::component;
        use silka_core::signals::{use_signal, Signal};
        use silka_core::view::{column, fixed};
        use silka_paint::Command;
        use std::rc::Rc;

        let pegangan: Rc<Cell<Option<Signal<i32>>>> = Rc::default();
        let simpan = pegangan.clone();

        let mut c = sambungkan_app(window("Uji"), move |_cx| {
            let count = use_signal(|| 0i32);
            simpan.set(Some(count));
            column([component("angka", move |_| {
                fixed(40.0, 20.0 + count.get() as f32 * 10.0)
                    .background(silka_paint::Color::WHITE)
                    .into()
            })])
            .into()
        });

        let theme = Theme::cupertino(Appearance::Dark);
        let animate = Cell::new(false);
        let ctx = frame_ctx(&theme, &animate);
        let f = c.scene_fn.as_mut().expect("run_app memasang scene_fn");

        // Frame pertama: scene datang dari pass paint render tree.
        let scene = f(&ctx);
        assert_eq!(scene.clear_color(), theme.color.background);
        let tinggi: Vec<f32> = scene
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::Quad(q) => Some(q.rect.size.height),
                _ => None,
            })
            .collect();
        assert_eq!(tinggi, vec![20.0]);
        assert!(
            !animate.get(),
            "tanpa perubahan signal, window kembali idle"
        );

        // Perubahan signal → frame berikutnya membawa scene yang berbeda.
        pegangan.get().unwrap().set(2);
        let scene = f(&ctx);
        let tinggi: Vec<f32> = scene
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::Quad(q) => Some(q.rect.size.height),
                _ => None,
            })
            .collect();
        assert_eq!(tinggi, vec![40.0]);
        assert!(!animate.get());
    }

    #[test]
    fn run_app_menyambungkan_input_dan_a11y_ke_pohon_yang_sama() {
        use silka_core::app::component;
        use silka_core::input::{Event, PointerButton, PointerEvent, PointerPhase};
        use silka_core::view::{fixed, interactive};
        use silka_paint::Point;

        let mut c = sambungkan_app(window("Uji"), |_cx| {
            component("tombol", |_| {
                interactive(fixed(120.0, 44.0)).label("Simpan").into()
            })
        });

        let theme = Theme::cupertino(Appearance::Light);
        let animate = Cell::new(false);
        let ctx = frame_ctx(&theme, &animate);
        let _ = c.scene_fn.as_mut().expect("scene_fn terpasang")(&ctx);

        // a11y membaca pohon yang baru saja di-layout, bukan pohon kosong.
        let pohon = c.access_fn.as_mut().expect("access_fn terpasang")();
        assert!(pohon.find_label("Simpan").is_some(), "{}", pohon.dump());

        // Input mengalir ke pohon yang sama dan menjadwalkan frame.
        let tekan = PointerEvent::new(PointerPhase::Down, Point::new(20.0, 20.0), Duration::ZERO)
            .button(PointerButton::Primary);
        let hasil = c.input_fn.as_mut().expect("input_fn terpasang")(&Event::Pointer(tekan));
        assert!(hasil.handled);
        assert!(!hasil.dirty.is_empty());
    }

    #[test]
    fn run_app_menitipkan_theme_sebagai_signal() {
        use silka_core::signals::Signal;
        use silka_core::view::fixed;
        use std::cell::RefCell;
        use std::rc::Rc;

        let terbaca: Rc<RefCell<Vec<Appearance>>> = Rc::default();
        let catat = terbaca.clone();

        let mut c = sambungkan_app(window("Uji").appearance(Appearance::Light), move |cx| {
            let theme: Signal<Theme> = cx.expect_env();
            catat.borrow_mut().push(theme.get().appearance);
            fixed(10.0, 10.0).into()
        });

        let animate = Cell::new(false);
        let terang = Theme::cupertino(Appearance::Light);
        let f = c.scene_fn.as_mut().expect("scene_fn terpasang");
        let _ = f(&frame_ctx(&terang, &animate));
        assert_eq!(*terbaca.borrow(), vec![Appearance::Light]);

        // Dark mode OS berubah → signal theme ditulis → komponen pembacanya
        // dibangun ulang, semuanya di dalam frame yang sama.
        let gelap = Theme::cupertino(Appearance::Dark);
        let scene = f(&frame_ctx(&gelap, &animate));
        assert_eq!(*terbaca.borrow(), vec![Appearance::Light, Appearance::Dark]);
        assert_eq!(scene.clear_color(), gelap.color.background);
        assert!(
            !animate.get(),
            "theme yang sudah diterapkan tidak menyisakan kerja"
        );
    }

    #[test]
    fn interval_log_frame_bisa_diatur() {
        let c = window("Uji");
        assert_eq!(c.frame_log_every, DEFAULT_FRAME_LOG_EVERY);
        assert_eq!(window("Uji").frame_log_every(0).frame_log_every, 0);
    }

    #[test]
    fn vsync_belum_diketahui_tidak_diganti_tebakan() {
        let theme = Theme::cupertino(Appearance::Light);
        let animate = Cell::new(false);
        let ctx = FrameContext {
            theme: &theme,
            size: Size::new(10.0, 10.0),
            scale_factor: 1.0,
            frame: 0,
            elapsed: Duration::ZERO,
            vsync: Vsync::UNKNOWN,
            animate: &animate,
        };
        assert!(!ctx.vsync().is_known());
        assert_eq!(ctx.vsync().budget(), None);
    }

    #[test]
    fn scene_fn_bisa_meminta_frame_berikutnya() {
        let theme = Theme::cupertino(Appearance::Light);
        let animate = Cell::new(false);
        let ctx = FrameContext {
            theme: &theme,
            size: Size::new(10.0, 10.0),
            scale_factor: 1.0,
            frame: 0,
            elapsed: Duration::ZERO,
            vsync: Vsync::UNKNOWN,
            animate: &animate,
        };
        assert!(!animate.get(), "frame tanpa animasi harus kembali idle");
        ctx.request_animation_frame();
        assert!(animate.get());
    }
}
