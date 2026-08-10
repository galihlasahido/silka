//! The window shell: Dart-style constructor + winit event loop + wgpu surface.
//!
//! The API shape follows REKOMENDASI §2.5 — a constructor function followed by
//! method chaining, with no struct literals and no macros:
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

/// Everything about one frame that the scene builder is given.
///
/// All sizes are in **logical points** — DPI is already resolved in the surface
/// layer, so code above here never deals with physical pixels.
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
    /// The active theme — the single source of colors, radii, and spacing.
    pub fn theme(&self) -> &'a Theme {
        self.theme
    }

    /// Size of the drawing area, in logical points.
    pub fn size(&self) -> Size {
        self.size
    }

    /// The window's scale factor (2.0 on a Retina display).
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// Frame number since the window opened.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Time since the window opened — the basis for animation until the spring
    /// system exists.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// The display clock currently in effect.
    ///
    /// On macOS this comes from `CADisplayLink` and **follows ProMotion**; on
    /// other systems it is estimated from the frames that actually happened. It
    /// may be [`Vsync::UNKNOWN`] during the first few frames — do not replace
    /// that with a guess.
    pub fn vsync(&self) -> Vsync {
        self.vsync
    }

    /// Ask for one more frame after this one.
    ///
    /// This is the only way animation keeps running: as long as someone calls
    /// it, the renderer keeps ticking; the moment nobody does, the window goes
    /// truly idle again (REKOMENDASI §3.5). An unsettled spring calls it every
    /// frame and stops on its own once it reaches its target.
    pub fn request_animation_frame(&self) {
        self.animate.set(true);
    }
}

type SceneFn = Box<dyn FnMut(&FrameContext<'_>) -> Scene>;

/// Input event handler.
///
/// Its shape is deliberately `Event -> Response`: the application (or the
/// widget layer above it) forwards the event to
/// [`silka_core::input::InputRouter`] and returns the result verbatim. The
/// shell then translates that into winit calls — `request_redraw`,
/// `set_ime_cursor_area`, `set_cursor` — so no winit type is ever visible to
/// the code above.
type InputFn = Box<dyn FnMut(&InputEvent) -> InputResponse>;

/// Builder of one window's accessibility tree.
///
/// Called **only** while some assistive technology is listening — users without
/// a screen reader do not pay for the pass at all.
type AccessFn = Box<dyn FnMut() -> AccessTree>;

/// Handler for action requests coming from assistive technology.
type AccessActionFn = Box<dyn FnMut(AccessActionRequest)>;

/// The glyph atlas source shared with the scene builder.
///
/// Shared through `Rc<RefCell<…>>` because two parties use it in turn on the
/// same thread: the `on_frame` closure while assembling the scene (rasterising
/// new glyphs into the atlas), then the backend while drawing (uploading the
/// changed part of the atlas). The two never run at the same time, so there is
/// no synchronisation cost — and `silka-platform` still has no idea what a font
/// is: all it holds is a trait from `silka-paint`.
type GlyphsRef = Rc<RefCell<dyn GlyphSource>>;

/// Window configuration, built up by method chaining.
///
/// Created through [`window`].
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

/// Create a new window with the given title.
///
/// Defaults: 1024×720 points, resizable, the Cupertino preset, and an
/// appearance that follows the OS.
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

/// How many frames apart the frame-time summary is printed in debug builds.
const DEFAULT_FRAME_LOG_EVERY: u64 = 120;

impl WindowConfig {
    /// Initial window size, in logical points.
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = Size::new(width, height);
        self
    }

    /// Minimum window size, in logical points.
    pub fn min_size(mut self, width: f32, height: f32) -> Self {
        self.min_size = Some(Size::new(width, height));
        self
    }

    /// Whether the window may be resized.
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// The complete theme (preset + appearance).
    ///
    /// Setting a theme with an explicit appearance **pins** the appearance: OS
    /// dark-mode changes are no longer followed. Call
    /// [`WindowConfig::follow_system_appearance`] to undo that.
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self.appearance_source = AppearanceSource::Locked;
        self
    }

    /// Change only the preset; the appearance keeps following its current
    /// source.
    pub fn preset(mut self, preset: Preset) -> Self {
        self.theme = self.theme.with_preset(preset);
        self
    }

    /// Pin the appearance to a specific value.
    pub fn appearance(mut self, appearance: Appearance) -> Self {
        self.theme = self.theme.with_appearance(appearance);
        self.appearance_source = AppearanceSource::Locked;
        self
    }

    /// Follow the OS dark mode live (INTEGRASI-NATIVE §6).
    pub fn follow_system_appearance(mut self) -> Self {
        self.appearance_source = AppearanceSource::System;
        self
    }

    /// The per-frame scene builder.
    ///
    /// Without it, the window is painted with the active theme's `background`
    /// token — enough to prove the window → surface → token path works.
    pub fn on_frame(mut self, scene_fn: impl FnMut(&FrameContext<'_>) -> Scene + 'static) -> Self {
        self.scene_fn = Some(Box::new(scene_fn));
        self
    }

    /// The glyph atlas source for text commands.
    ///
    /// Without it, a `GlyphRun` command inside the scene **produces no pixels**
    /// — the backend has no bitmap to draw. What is normally handed over is the
    /// same `silka_text::TextEngine` that `on_frame` uses:
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
    /// The contract still holds: all that crosses over is the
    /// `silka_paint::GlyphSource` trait — the shell has no idea what a font is,
    /// and the backend has no idea what winit is.
    pub fn glyphs<G: GlyphSource + 'static>(mut self, glyphs: Rc<RefCell<G>>) -> Self {
        self.glyphs = Some(glyphs as GlyphsRef);
        self
    }

    /// The accessibility tree builder (§3.8).
    ///
    /// Usually `move || tree.access_tree(router.focus().focused())`. Without
    /// it, the window is still **visible** to a screen reader — with its title
    /// as the name — only its contents are empty; the application is never
    /// totally blind the way GPUI/Floem/Makepad are (§7.2).
    ///
    /// The closure is called only while assistive technology is active.
    pub fn on_access(mut self, access_fn: impl FnMut() -> AccessTree + 'static) -> Self {
        self.access_fn = Some(Box::new(access_fn));
        self
    }

    /// Handler for action requests from assistive technology (a VoiceOver
    /// click, and so on).
    ///
    /// The request has already been validated against the tree that was
    /// actually sent: dead nodes and actions that were never advertised never
    /// reach this point.
    pub fn on_access_action(
        mut self,
        action_fn: impl FnMut(AccessActionRequest) + 'static,
    ) -> Self {
        self.access_action_fn = Some(Box::new(action_fn));
        self
    }

    /// Handler for input events (pointer, keyboard, scroll, IME).
    ///
    /// Events arrive in the framework's vocabulary — no winit type crosses over
    /// (§3.2 applied to input). The normal path is a single line:
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
    /// What comes back decides what the shell does next:
    /// [`silka_core::input::Response::dirty`] wakes the renderer (and it is the
    /// **only** thing that wakes it — §3.5), `ime` is translated into
    /// `set_ime_allowed`/`set_ime_cursor_area`, and `cursor` into
    /// `set_cursor`.
    pub fn on_input(
        mut self,
        input_fn: impl FnMut(&InputEvent) -> InputResponse + 'static,
    ) -> Self {
        self.input_fn = Some(Box::new(input_fn));
        self
    }

    /// Interval between frame-time summaries in debug builds.
    ///
    /// `0` disables the periodic summary; frames that blow the vsync budget are
    /// still reported. In release builds the measurement still runs (it is
    /// cheap) but nothing is printed.
    pub fn frame_log_every(mut self, frames: u64) -> Self {
        self.frame_log_every = frames;
        self
    }

    /// Open the window and run the event loop until the window is closed.
    ///
    /// The event loop uses [`ControlFlow::Wait`]: **nothing** spins while idle.
    /// A frame is drawn only when the OS asks for a redraw or something marks
    /// the window dirty (REKOMENDASI §3.5).
    pub fn run(self) -> Result<(), PlatformError> {
        // The event loop carries a *user event*: that is the accessibility
        // return path from any OS thread back to the UI thread (§3.8).
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

/// Open a window and run a reactive application inside it — **the
/// run-an-application API** (REKOMENDASI §2.5).
///
/// This is the shape application authors see: a window, a closure that returns
/// a view tree, and not one seam in between that has to be assembled by hand.
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
/// What this function wires up, and why:
///
/// - **`on_frame` produces the scene from the lifecycle**, not from a
///   hand-assembled scene: `AppRuntime::frame()` runs rebuild → diff → layout →
///   paint, and all that crosses over to the backend is still a
///   [`silka_paint::Scene`] (§3.2).
/// - **`on_input` routes events into that same tree** and returns the reason it
///   is dirty — including dirtiness born from signal writes inside the handler.
/// - **`on_access` builds the a11y tree from that same frame's geometry**, with
///   focus taken from the router (§3.8).
/// - **The theme is provided as a `Signal<Theme>`** in
///   [`silka_core::app::Env`]: an OS dark-mode change writes that signal, and
///   **only** the components that actually read the theme are rebuilt (§2.7).
///
/// The "render only when dirty" promise stays intact: once a frame is done, the
/// shell asks for another one only if
/// [`silka_core::app::AppRuntime::is_idle`] is false.
///
/// Any [`WindowConfig::on_frame`], [`WindowConfig::on_input`], and
/// [`WindowConfig::on_access`] already set on `config` are **replaced** by this
/// function.
pub fn run_app(
    config: WindowConfig,
    build: impl Fn(&BuildCtx) -> View + 'static,
) -> Result<(), PlatformError> {
    sambungkan_app(config, build).run()
}

/// [`run_app`] **with an animation driver** — the shape used by applications
/// that use animated widgets.
///
/// `animate` is called once per frame **before** the rebuild → layout → paint
/// cycle, with that frame's render tree and [`Tick`]; what it returns is the
/// dirty reason, and as long as it keeps naming
/// [`Dirty::ANIMATION`](silka_core::scheduler::Dirty::ANIMATION) the shell
/// keeps asking for another frame. Once every spring settles, the event loop
/// goes back to waiting — the presence of animation does not break the "render
/// only when dirty" promise (§3.5).
///
/// The signature of `animate` deliberately matches `silka_widgets::advance`, so
/// an ordinary application only has to write:
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

/// An [`AppRuntime`] **assembled exactly like [`run_app`] does**, without a
/// window and without a GPU (REKOMENDASI §9.5).
///
/// This is the entry point for headless integration tests: the same page that
/// shows up in a window runs here, is fed input events through
/// [`AppRuntime::dispatch`], and its [`AppRuntime::scene`] can then be rendered
/// into an offscreen texture and have its pixels counted. Because `run_app`
/// itself uses this function, the [`Env`] values the application sees cannot
/// differ between "on screen" and "in CI".
///
/// What is provided is identical to `run_app`:
///
/// - `Signal<Theme>` — a dark-mode/preset change rebuilds only the components
///   that actually read it (§2.7).
/// - `Signal<ScaleFactor>` — the screen resolution used for text rasterisation
///   (§3.3).
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
        // An honest starting value: before a window exists, the scale factor
        // genuinely is unknown. The shell overwrites it on the first frame.
        .with_env(|rt| rt.signal(ScaleFactor::ONE))
}

/// The part of [`run_app`] that never touches the event loop.
///
/// Split out so the seams can be tested headlessly: tests call the `scene_fn`,
/// `input_fn`, and `access_fn` installed here with a hand-made
/// [`FrameContext`], without a window and without a GPU.
fn sambungkan_app(
    config: WindowConfig,
    build: impl Fn(&BuildCtx) -> View + 'static,
) -> WindowConfig {
    sambungkan_app_with(config, build, |_, _| Dirty::NONE)
}

/// [`sambungkan_app`] with an animation driver (see [`run_app_with`]).
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
            // Changes from the shell land first so this frame's rebuild
            // already sees them — not one frame later.
            ui.resize(ctx.size());
            ui.set_clear_color(ctx.theme().color.background);
            if let Some(theme) = ui.env::<Signal<Theme>>() {
                theme.set_if_changed(*ctx.theme());
            }
            // Text must be rasterised at the real screen resolution (§3.3); a
            // window moved to another monitor writes this signal, and only the
            // components that read it are rebuilt.
            if let Some(scale) = ui.env::<Signal<ScaleFactor>>() {
                scale.set_if_changed(ScaleFactor(ctx.scale_factor() as f32));
            }
            ui.set_vsync(ctx.vsync());

            // Springs are advanced **before** the frame: the value that moves
            // becomes this frame's value, not the next frame's (§3.5). Its `dt`
            // is computed from a real clock by `AnimationDriver`, never from a
            // 16.6 ms constant.
            ui.animate(&mut animate);

            ui.frame();

            // The only way a next frame happens: something is still dirty (an
            // unsettled spring, a signal written during build).
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

/// The default a11y tree: a single window node named after the application.
///
/// An application that has not yet wired up its render tree is still
/// **visible** to a screen reader — its window has a name and can be focused.
/// Total blindness (GPUI, Floem, Makepad — §7.2) is not a default state that
/// can happen here.
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

    /// Send the a11y tree to the adapter.
    ///
    /// Split out from [`Shell::gambar`] so it can also be called when assistive
    /// technology asks for the initial tree — a moment that does not always
    /// coincide with a frame.
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
        // Deliberately hidden at first: the accessibility adapter **must** be
        // attached before the window is ever visible (§3.8). The window is
        // shown once the adapter and the surface are ready.
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

        // Accessibility from day one, not a retrofit (§3.8, §5 point 2).
        let access = AccessAdapter::new(event_loop, &window, self.proxy.clone());

        // Initial appearance from the OS, before the first frame is drawn.
        if self.appearance_source == AppearanceSource::System {
            if let Some(t) = window.theme() {
                self.theme = self.theme.with_appearance(appearance_from_winit(t));
            }
        }

        let PhysicalSize { width, height } = window.inner_size();
        // Input speaks logical points; its DPI divisor is learned here.
        self.input.set_scale_factor(window.scale_factor());
        let geometry = SurfaceGeometry::new(width, height, window.scale_factor());
        let (gpu, surface) = Gpu::with_surface(window.clone(), geometry)?;

        // Frame clock source: CADisplayLink on macOS, `request_redraw`
        // elsewhere. Attached idle — nothing is dirty yet.
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

        // Everything is ready — only now may the window become visible.
        window.set_visible(true);

        self.state = Some(ShellState {
            window,
            gpu,
            surface,
            vsync,
            access,
        });

        // The first frame: the only frame not triggered by a change.
        self.minta(Dirty::LAYOUT | Dirty::PAINT);
        Ok(())
    }

    /// Route one input event into the application, then carry out what it asks
    /// for.
    ///
    /// This is the only place routing results meet winit: dirty wakes the
    /// renderer, an IME request becomes
    /// `set_ime_allowed`/`set_ime_cursor_area` (the CJK candidate window
    /// anchors to the caret, §3.8), and a cursor becomes `set_cursor`.
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
                // An IME that is already off does not need turning off again.
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

    /// Mark dirty and — only when genuinely needed — wake the vsync source.
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

        // The interval the OS reports can change at any moment (ProMotion
        // stepping up and down, the window moving monitor) — reread each frame.
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

        // The boundary between our work and the swapchain queue. Without this
        // marker, time spent waiting for vsync would be recorded as a "slow
        // frame" when it is in fact a sign of a healthy system.
        start.mark_built(Instant::now());

        // Wayland wants to know before the buffer is attached; a no-op
        // elsewhere.
        state.window.pre_present_notify();
        // The glyph atlas is borrowed ONLY while drawing — the scene closure is
        // already done with it, so there are never two borrowers at once.
        let hasil = match glyphs {
            Some(g) => {
                let mut sumber = g.borrow_mut();
                state
                    .surface
                    .render_with_glyphs(&state.gpu, &scene, &mut *sumber)
            }
            None => state.surface.render(&state.gpu, &scene),
        };

        // The frame is closed first whatever the outcome: the statistics of a
        // failed frame are exactly the interesting ones when hunting jank.
        let timing = scheduler.end_frame(
            start,
            Instant::now(),
            matches!(hasil, Ok(FrameOutcome::Presented)),
        );

        // Measurement always runs; printing happens only in debug builds.
        #[cfg(debug_assertions)]
        if let Some(line) = logger.line(scheduler.stats(), scheduler.vsync(), &timing) {
            eprintln!("{line}");
        }
        #[cfg(not(debug_assertions))]
        let _ = (logger, &timing);

        hasil?;

        // The a11y tree is rebuilt after the frame, from that same frame's
        // geometry — and **only** if assistive technology is listening.
        self.kirim_a11y(false);

        if animate.get() {
            self.minta(Dirty::ANIMATION);
        }

        // No work left → stop the clock. This is what makes idle truly idle,
        // rather than merely "drawing an empty frame".
        if self.scheduler.is_idle() {
            if let Some(state) = self.state.as_ref() {
                state.vsync.idle();
            }
        }
        Ok(())
    }

    /// Change window visibility (occlusion/minimize) without wasted drawing.
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
    /// The return path from assistive technology.
    ///
    /// `accesskit_winit` calls its handler on any thread; the winit event loop
    /// is the official channel back to the UI thread.
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
            // A screen reader was just switched on: it has no history at all,
            // so what we send must be the complete tree.
            AccessOutcome::NeedsFullTree => self.kirim_a11y(true),
            AccessOutcome::Action(request) => {
                if let Some(f) = self.access_action_fn.as_mut() {
                    f(request);
                    // An action from assistive technology is input just like a
                    // mouse click: whatever it changes must be drawn.
                    self.minta(Dirty::PAINT);
                }
            }
            AccessOutcome::Idle => {}
        }
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, _cause: StartCause) {
        // Idle must really be idle: no polling, no timers.
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
        // The surface is invalid while suspended (an Android rule; harmless on
        // desktop). It is rebuilt on the next `resumed`.
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

        // The a11y adapter sees the event **before** the shell handles it:
        // window focus and geometry are tracked from here.
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
                // A minimized window arrives as a 0×0 size. Without this, an
                // animation that keeps asking for the next frame would spin
                // forever drawing into an undrawable surface.
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
                // winit follows up with a `Resized` carrying the new physical
                // size; here it is enough to update the logical-point divisor.
                if let Some(state) = self.state.as_mut() {
                    state.surface.set_scale_factor(scale_factor);
                }
                self.input.set_scale_factor(scale_factor);
                // A new monitor may refresh at a different rate — the old
                // estimate is void.
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

            // The window is fully covered: do not burn GPU on pixels nobody
            // will ever see.
            WindowEvent::Occluded(occluded) => self.set_terlihat(!occluded),

            // -- input (INTEGRASI-NATIVE §3) ---------------------------------
            WindowEvent::ModifiersChanged(modifiers) => self.input.modifiers_changed(modifiers),

            WindowEvent::CursorMoved { position, .. } => {
                let e = self.input.cursor_moved(position);
                self.masukan(e);
            }

            // `CursorEntered` carries no coordinates; the useful `Enter` is
            // synthesised from the first `CursorMoved`.
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

            // The window lost focus: an in-flight interaction is **cancelled**,
            // not completed — a button that is pressed and then abandoned must
            // not produce a click.
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

/// The default background color for a theme — the very path the shell uses
/// when an application supplies no [`WindowConfig::on_frame`].
///
/// Exposed so tests and headless tooling can verify that the clear color really
/// does come from a token rather than from a literal.
pub fn default_clear_color(theme: &Theme) -> Color {
    theme.color.background
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::AccessRole;

    /// A fake atlas source — enough to prove the path is wired up, without
    /// dragging the text stack into the shell tests.
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
        // An application without text pays nothing: no atlas source, and the
        // plain `render` path is used.
        assert!(window("Tanpa teks").glyphs.is_none());
    }

    #[test]
    fn sumber_atlas_terpasang_lewat_method_chaining() {
        let atlas = Rc::new(RefCell::new(AtlasPalsu::default()));
        let config = window("Dengan teks").glyphs(atlas.clone());
        let terpasang = config.glyphs.expect("sumber atlas tersimpan");
        // The same object, not a copy: the atlas the scene closure fills is
        // exactly the one the backend reads.
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
        // A window with no input handler is still valid — it simply never gets
        // woken by input.
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

    /// A skeleton [`FrameContext`] for tests — the only part of a frame that
    /// cannot be built without a window.
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

        // First frame: the scene comes from the render tree's paint pass.
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

        // A signal change → the next frame carries a different scene.
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

        // a11y reads the tree that was just laid out, not an empty one.
        let pohon = c.access_fn.as_mut().expect("access_fn terpasang")();
        assert!(pohon.find_label("Simpan").is_some(), "{}", pohon.dump());

        // Input flows into the same tree and schedules a frame.
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

        // OS dark mode changes → the theme signal is written → the components
        // that read it are rebuilt, all within the same frame.
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
