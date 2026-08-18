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
use silka_paint::{Color, GlyphSource, ImageSource, Scene, Size};
use silka_renderer::{FrameOutcome, Gpu, SurfaceGeometry, WindowSurface};
use silka_theme::{Appearance, Preset, Theme};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use crate::access::{AccessAdapter, AccessOutcome};
use crate::appearance::{appearance_from_winit, winit_theme_from_appearance, AppearanceSource};
use crate::error::PlatformError;
use crate::event::{forward_native_events, ShellEvent};
use crate::hotkey::{HotkeyActivation, HotkeyManager, HotkeyRegistration};
use crate::input::{cursor_to_winit, ime_area_to_winit, WinitInput};
use crate::lifecycle::{
    restore_placement, AccentSource, MonitorArea, QuitContext, QuitReason, SessionState,
    StateStore, SystemSettings, WindowPlacement,
};
use crate::menu::{InstalledMenu, MenuActivation, MenuBar};
use crate::platform::{NativeEvent, NativeFlow, NativeWindow};
use crate::titlebar::{
    apply_material, apply_titlebar_style, set_traffic_light_inset, Material, MaterialState,
    TitlebarStyle,
};
use crate::tray::{Tray, TrayActivation, TrayConfig};
use crate::vsync::VsyncSource;

/// Everything about one frame that the scene builder is given.
///
/// All sizes are in **logical points** — DPI is already resolved in the surface
/// layer, so code above here never deals with physical pixels.
///
/// ```no_run
/// use silka_paint::{Color, Scene};
/// use silka_theme::ColorToken;
/// use silka_platform::window;
///
/// window("Demo")
///     .on_frame(|cx| {
///         // Everything a frame needs, and nothing in physical pixels.
///         let mut scene = Scene::new(cx.theme().color_of(ColorToken::Background));
///         let _size = cx.size();
///         let _scale = cx.scale_factor();
///         // Ask for another frame only while something is genuinely moving.
///         if cx.elapsed().as_secs() < 1 {
///             cx.request_animation_frame();
///         }
///         scene
///     })
///     .run()
///     .unwrap();
/// ```
#[derive(Debug, Clone, Copy)]
pub struct FrameContext<'a> {
    theme: &'a Theme,
    size: Size,
    scale_factor: f64,
    frame: u64,
    elapsed: Duration,
    vsync: Vsync,
    animate: &'a Cell<bool>,
    /// The platform window, when there is one — the escape hatch reached from
    /// inside a frame (INTEGRASI-NATIVE §8). `None` headlessly.
    native: Option<&'a NativeWindow>,
    /// The OS settings in effect this frame (INTEGRASI-NATIVE §6).
    settings: SystemSettings,
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

    /// The OS lifecycle settings in effect this frame (INTEGRASI-NATIVE §6).
    ///
    /// The accent color and reduce-transparency setting are **already applied
    /// to [`FrameContext::theme`]** — reading them here is for the rare case
    /// that wants to know *why* a token looks the way it does. Reduced motion
    /// is the one setting a scene builder still has to act on itself, and
    /// [`FrameContext::motion`] is the shorter way to ask.
    pub fn settings(&self) -> SystemSettings {
        self.settings
    }

    /// The user's reduced-motion preference (INTEGRASI-NATIVE §6).
    ///
    /// [`run_app`] already hands this to the animation driver, so every
    /// [`silka_core::animation::Tick`] carries it and ordinary widgets never
    /// have to ask. It is exposed for the code that animates outside a spring
    /// — a hand-rolled `on_frame` that moves something per elapsed time.
    pub fn motion(&self) -> silka_core::animation::Motion {
        self.settings.motion
    }

    /// The platform window behind this frame — the escape hatch
    /// (INTEGRASI-NATIVE §8).
    ///
    /// Use it for native work that has to stay in step with the drawing: an
    /// overlay layer, a native video surface, a `NSVisualEffectView` that must
    /// follow a panel's geometry.
    ///
    /// `None` when there is no window at all — in [`headless_app`] and in the
    /// integration tests that run the same page in CI (§9.5). That is exactly
    /// why it is an `Option`: a test must never be able to reach a window that
    /// does not exist.
    pub fn native(&self) -> Option<&'a NativeWindow> {
        self.native
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

/// Called once, after the window exists and before it is ever shown — the
/// moment platform polish belongs in (INTEGRASI-NATIVE §8).
type NativeReadyFn = Box<dyn FnMut(&NativeWindow)>;

/// Called for every window event, **before** the framework handles it (§8).
type NativeEventFn = Box<dyn FnMut(&NativeEvent<'_>) -> NativeFlow>;

/// Called once, when the application is closing — the last moment at which
/// state can still be written (INTEGRASI-NATIVE §6).
type QuitFn = Box<dyn FnMut(&mut QuitContext)>;

/// Handler for a chosen menu item (INTEGRASI-NATIVE §2).
///
/// It returns [`Dirty`] for the same reason [`WindowConfig::on_input`] does:
/// the shell has no way to know whether the handler changed anything, and
/// "render only when dirty" (§3.5) has to survive contact with menus too. A
/// handler that writes signals returns `Dirty::LAYOUT`; one that only opened a
/// file dialog and got a cancel returns `Dirty::NONE` and the window stays
/// asleep.
type MenuFn = Box<dyn FnMut(&MenuActivation) -> Dirty>;

/// Handler for a tray-icon gesture (INTEGRASI-NATIVE §2). Same contract as
/// [`MenuFn`].
type TrayFn = Box<dyn FnMut(&TrayActivation) -> Dirty>;

/// Handler for a global hotkey (INTEGRASI-NATIVE §3). Same contract as
/// [`MenuFn`] — and the one where `Dirty::NONE` is genuinely common, because a
/// hotkey often fires while the window is hidden and there is nothing to draw
/// until it is shown.
type HotkeyFn = Box<dyn FnMut(&HotkeyActivation) -> Dirty>;

/// The glyph atlas source shared with the scene builder.
///
/// Shared through `Rc<RefCell<…>>` because two parties use it in turn on the
/// same thread: the `on_frame` closure while assembling the scene (rasterising
/// new glyphs into the atlas), then the backend while drawing (uploading the
/// changed part of the atlas). The two never run at the same time, so there is
/// no synchronisation cost — and `silka-platform` still has no idea what a font
/// is: all it holds is a trait from `silka-paint`.
type GlyphsRef = Rc<RefCell<dyn GlyphSource>>;

/// The image atlas source shared with the scene builder.
///
/// Held exactly like [`GlyphsRef`], and for exactly the same reason: the scene
/// closure may insert a newly decoded bitmap while assembling the frame, and the
/// backend then uploads the part that changed. `silka-platform` still knows
/// nothing about image formats — all it holds is a trait from `silka-paint`.
type ImagesRef = Rc<RefCell<dyn ImageSource>>;

/// Window configuration, built up by method chaining.
///
/// Created through [`window`]. This is the framework's front door: every
/// platform capability — titlebar, material, menubar, tray, session state, the
/// escape hatch — hangs off this one chain rather than off a separate API.
///
/// ```no_run
/// use silka_platform::{window, FileStore, Material, TitlebarStyle};
/// use silka_theme::Preset;
///
/// window("Editor")
///     .size(960.0, 640.0)
///     .min_size(640.0, 480.0)
///     .preset(Preset::Cupertino)
///     .follow_system_appearance()      // live dark mode
///     .titlebar(TitlebarStyle::Transparent)
///     .material(Material::Sidebar)
///     .restore_state(FileStore::for_app("Editor"))
///     .run()
///     .unwrap();
/// ```
///
/// For an application built from views rather than hand-assembled scenes, pass
/// the same config to [`run_app`](crate::run_app).
pub struct WindowConfig {
    title: String,
    size: Size,
    min_size: Option<Size>,
    resizable: bool,
    theme: Theme,
    appearance_source: AppearanceSource,
    scene_fn: Option<SceneFn>,
    glyphs: Option<GlyphsRef>,
    images: Option<ImagesRef>,
    access_fn: Option<AccessFn>,
    access_action_fn: Option<AccessActionFn>,
    input_fn: Option<InputFn>,
    native_ready_fn: Option<NativeReadyFn>,
    native_event_fn: Option<NativeEventFn>,
    accent: AccentSource,
    settings: Option<SystemSettings>,
    store: Option<Box<dyn StateStore>>,
    quit_fn: Option<QuitFn>,
    restore_geometry: bool,
    menubar: Option<MenuBar>,
    menu_fn: Option<MenuFn>,
    tray: Option<TrayConfig>,
    tray_fn: Option<TrayFn>,
    hotkeys: Option<HotkeyManager>,
    hotkey_fn: Option<HotkeyFn>,
    titlebar: TitlebarStyle,
    material: Material,
    material_state: MaterialState,
    traffic_light_inset: Option<silka_paint::Point>,
    frame_log_every: u64,
}

/// Create a new window with the given title.
///
/// Defaults: 1024×720 points, resizable, the Cupertino preset, and an
/// appearance that follows the OS.
///
/// ```
/// use silka_paint::{Color, Scene};
/// use silka_theme::{Appearance, Preset, Theme};
/// use silka_platform::window;
///
/// // Everything optional is a method; nothing here opens a window yet.
/// let config = window("Silka")
///     .size(1280.0, 800.0)
///     .min_size(640.0, 480.0)
///     .resizable(true)
///     .preset(Preset::Cupertino)
///     .follow_system_appearance()
///     .on_frame(|cx| Scene::new(cx.theme().color.background));
/// # let _ = (config, Appearance::Dark, Theme::default(), Color::WHITE);
/// ```
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
        images: None,
        access_fn: None,
        access_action_fn: None,
        input_fn: None,
        native_ready_fn: None,
        native_event_fn: None,
        accent: AccentSource::System,
        settings: None,
        store: None,
        quit_fn: None,
        restore_geometry: true,
        menubar: None,
        menu_fn: None,
        tray: None,
        tray_fn: None,
        hotkeys: None,
        hotkey_fn: None,
        titlebar: TitlebarStyle::Native,
        material: Material::None,
        material_state: MaterialState::FollowsWindow,
        traffic_light_inset: None,
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

    /// The image atlas the backend uploads bitmaps from.
    ///
    /// Usually the application's [`silka_paint::ImageAtlas`]. Without it,
    /// `Command::Image` draws nothing at all — the same negative-control
    /// behaviour a missing glyph source has, and for the same reason: drawing
    /// nothing is honest, drawing garbage is not.
    pub fn images<I: ImageSource + 'static>(mut self, images: Rc<RefCell<I>>) -> Self {
        self.images = Some(images as ImagesRef);
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

    /// The escape hatch's opening move: called once, after the window exists
    /// and **before it is ever shown** (INTEGRASI-NATIVE §8).
    ///
    /// That moment is the point of this hook. A transparent titlebar, an
    /// extended DWM frame, a window level, a vibrancy layer — all of them are
    /// visible mistakes if applied one frame after the window appears, and
    /// invisible if applied here. The surface and the accessibility adapter
    /// already exist, so it is also the earliest safe place to attach a native
    /// layer of one's own.
    ///
    /// ```no_run
    /// # use silka_platform::window;
    /// window("Editor")
    ///     .on_native_ready(|native| {
    ///         #[cfg(target_os = "macos")]
    ///         if let Some(w) = native.ns_window() {
    ///             w.setTitlebarAppearsTransparent(true);
    ///         }
    ///         #[cfg(target_os = "windows")]
    ///         if let Some(hwnd) = native.hwnd() {
    ///             // … DwmExtendFrameIntoClientArea(HWND(hwnd as *mut _), …) …
    ///             let _ = hwnd;
    ///         }
    ///     })
    ///     .run()
    ///     .unwrap();
    /// ```
    ///
    /// The [`NativeWindow`] handed over is cheap to clone and keeps the window
    /// alive, so an application that needs the handle later stores **it**, not
    /// the raw pointer.
    pub fn on_native_ready(mut self, ready: impl FnMut(&NativeWindow) + 'static) -> Self {
        self.native_ready_fn = Some(Box::new(ready));
        self
    }

    /// Raw window events, **before** the framework processes them
    /// (INTEGRASI-NATIVE §8).
    ///
    /// The hook sees every event in winit's own vocabulary — the one place the
    /// framework hands out a winit type on purpose (§3.2 holds everywhere else)
    /// — and decides what happens next with [`NativeFlow`]:
    ///
    /// ```no_run
    /// # use silka_platform::{window, NativeFlow};
    /// # let ada_perubahan_belum_disimpan = || true;
    /// window("Editor")
    ///     .on_native_event(move |e| {
    ///         if e.is_close_requested() && ada_perubahan_belum_disimpan() {
    ///             // The window stays open; the application shows its own
    ///             // "save changes?" dialog.
    ///             return NativeFlow::Consume;
    ///         }
    ///         NativeFlow::Continue
    ///     })
    ///     .run()
    ///     .unwrap();
    /// ```
    ///
    /// [`NativeFlow::Consume`] skips the shell's **own** handling of that event
    /// — input routing, resize, redraw scheduling, closing the window. The
    /// accessibility adapter still sees it: it only observes window focus and
    /// geometry, and letting a hook silently corrupt the a11y tree would break
    /// §3.8 in a way nobody would trace back to their own code. Consuming
    /// [`NativeEvent::is_redraw_requested`] stops the frame from being drawn at
    /// all — almost never what is meant.
    ///
    /// What this hook does *not* see: OS messages below winit's own level (an
    /// `NSEvent` before AppKit dispatch, a `WM_` message before
    /// `DefWindowProc`). Those are reached the way any native application
    /// reaches them, starting from
    /// [`NativeWindow::raw_handle`](crate::NativeWindow::raw_handle).
    pub fn on_native_event(
        mut self,
        hook: impl FnMut(&NativeEvent<'_>) -> NativeFlow + 'static,
    ) -> Self {
        self.native_event_fn = Some(Box::new(hook));
        self
    }

    /// Follow the **OS accent color** (INTEGRASI-NATIVE §6) — the default.
    ///
    /// What the OS reports replaces the whole accent family, not just one
    /// token: hover, pressed, the soft badge fill, the focus ring, and the
    /// content color that has to stay readable on top of it
    /// ([`silka_theme::Theme::with_accent`]). When the OS has no accent — macOS
    /// left on "Multicolor", a desktop with no such concept — the preset's own
    /// accent applies, so there is never a hole.
    pub fn follow_system_accent(mut self) -> Self {
        self.accent = AccentSource::System;
        self
    }

    /// Pin the accent to the preset's own color; the OS setting is ignored.
    ///
    /// This is what a **branded** application wants: a purple product does not
    /// turn green because the user likes green.
    pub fn preset_accent(mut self) -> Self {
        self.accent = AccentSource::Preset;
        self
    }

    /// Pin the accent to a specific color.
    pub fn accent(mut self, accent: Color) -> Self {
        self.accent = AccentSource::Custom(accent);
        self
    }

    /// Pin the OS lifecycle settings by hand — nothing is read from the OS
    /// afterwards.
    ///
    /// Two uses, both real: a screenshot/CI run that must produce the same
    /// pixels on every machine (§9.5), and an application that reads a setting
    /// the framework does not yet know how to read — Windows' colorization
    /// color, `NSWorkspace`'s accessibility flags from inside a sandbox — and
    /// hands the answer over through the escape hatch (§8).
    ///
    /// ```no_run
    /// # use silka_platform::{window, SystemSettings};
    /// # use silka_core::animation::Motion;
    /// window("Cuplikan")
    ///     .system_settings(SystemSettings {
    ///         motion: Motion::Reduced,
    ///         ..SystemSettings::DEFAULT
    ///     })
    ///     .run()
    ///     .unwrap();
    /// ```
    pub fn system_settings(mut self, settings: SystemSettings) -> Self {
        self.settings = Some(settings);
        self
    }

    /// Remember this window's geometry and the application's own state between
    /// runs (INTEGRASI-NATIVE §6, "session restore").
    ///
    /// The store is read once, before the window is created, and written once,
    /// when the application quits. The window geometry is put in by the shell;
    /// everything else comes from [`WindowConfig::on_quit`].
    ///
    /// ```no_run
    /// # use silka_platform::{window, FileStore};
    /// window("Galeri")
    ///     .restore_state(FileStore::for_app("Galeri"))
    ///     .run()
    ///     .unwrap();
    /// ```
    ///
    /// A saved position is only reused if it is still **reachable**: a window
    /// last seen on a monitor that has since been unplugged comes back where
    /// the OS puts it, not at `x = 3000` where nobody would ever find it
    /// ([`restore_placement`]).
    pub fn restore_state<S: StateStore + 'static>(mut self, store: S) -> Self {
        self.store = Some(Box::new(store));
        self
    }

    /// Whether a stored geometry is applied to the window at startup.
    ///
    /// `false` keeps the store for the application's own values while letting
    /// the OS place the window — what a document window that always opens
    /// cascaded wants.
    pub fn restore_geometry(mut self, restore: bool) -> Self {
        self.restore_geometry = restore;
        self
    }

    /// The last moment before the application closes (INTEGRASI-NATIVE §6).
    ///
    /// The handler is given the [`SessionState`] that is about to be written —
    /// with the window geometry already filled in — and may add to it. It runs
    /// exactly once, whether the user closed the window, quit through the menu,
    /// or the OS is logging them out.
    ///
    /// ```no_run
    /// # use silka_platform::window;
    /// # let dokumen_terbuka = || "/tmp/a.txt".to_string();
    /// window("Editor")
    ///     .on_quit(move |quit| quit.remember("dokumen", dokumen_terbuka()))
    ///     .run()
    ///     .unwrap();
    /// ```
    ///
    /// A handler may also refuse the close with
    /// [`QuitContext::cancel`] — but only when the quit is still cancellable
    /// ([`QuitReason::can_cancel`]): vetoing an OS logout is not a right an
    /// application has.
    ///
    /// Without a [`WindowConfig::restore_state`] store the handler still runs;
    /// what it writes is simply thrown away, which keeps "does my quit path
    /// work?" answerable without a file on disk.
    pub fn on_quit(mut self, quit: impl FnMut(&mut QuitContext) + 'static) -> Self {
        self.quit_fn = Some(Box::new(quit));
        self
    }

    // -- native integration (INTEGRASI-NATIVE §1–§2) ------------------------

    /// The application menubar.
    ///
    /// On macOS it belongs to the whole application, not to this window; the
    /// window is simply the moment at which there is something to install it
    /// from. Use [`crate::menu::menubar`] to get one that already carries the
    /// standard Edit menu — without it ⌘C and ⌘V never reach a focused text
    /// field, whatever the widget layer does.
    ///
    /// ```no_run
    /// use silka_platform::menu::{item, menu, menubar};
    /// use silka_platform::window;
    ///
    /// window("Editor")
    ///     .menubar(menubar("Editor").menu(menu("File").item(item("file.new", "New"))))
    ///     .on_menu(|a| {
    ///         if a.is("file.new") { /* … */ }
    ///         silka_platform::Dirty::LAYOUT
    ///     })
    ///     .run()
    ///     .unwrap();
    /// ```
    pub fn menubar(mut self, menubar: MenuBar) -> Self {
        self.menubar = Some(menubar);
        self
    }

    /// Handler for menu items chosen by the user.
    ///
    /// What it returns decides whether a frame happens: [`Dirty::NONE`] leaves
    /// the window asleep, anything else schedules a frame.
    pub fn on_menu(mut self, handler: impl FnMut(&MenuActivation) -> Dirty + 'static) -> Self {
        self.menu_fn = Some(Box::new(handler));
        self
    }

    /// A tray / status-bar icon, created alongside the window.
    pub fn tray(mut self, tray: TrayConfig) -> Self {
        self.tray = Some(tray);
        self
    }

    /// Handler for tray-icon gestures.
    pub fn on_tray(mut self, handler: impl FnMut(&TrayActivation) -> Dirty + 'static) -> Self {
        self.tray_fn = Some(Box::new(handler));
        self
    }

    /// Global hotkeys, registered once the event loop is running
    /// (INTEGRASI-NATIVE §3).
    ///
    /// Registration is deliberately not done by the caller: both back-ends
    /// want the thread that pumps the event loop — macOS installs a Carbon
    /// handler on the application event target, Windows creates a message-only
    /// window — and this is the one place in the framework that knows when
    /// that thread has a loop on it.
    ///
    /// The registration lives exactly as long as the window does, so quitting
    /// gives every combination back to the desktop.
    ///
    /// A refusal is reported and stepped over rather than propagated, for the
    /// same reason a menubar's is: an application whose window will not open
    /// because another program owns ⌘⇧Space is worse than one whose global
    /// shortcut does not work.
    ///
    /// ```no_run
    /// use silka_core::input::{KeyCode, Modifiers};
    /// use silka_platform::hotkey::hotkeys;
    /// use silka_platform::menu::shortcut;
    /// use silka_platform::{window, Dirty};
    ///
    /// let mut keys = hotkeys();
    /// keys.add(
    ///     "app.quick_open",
    ///     shortcut(Modifiers::COMMAND | Modifiers::SHIFT, KeyCode::Character('k')),
    /// );
    ///
    /// window("Editor")
    ///     .hotkeys(keys)
    ///     .on_hotkey(|a| {
    ///         if a.is("app.quick_open") && a.is_pressed() { /* … */ }
    ///         Dirty::PAINT
    ///     })
    ///     .run()
    ///     .unwrap();
    /// ```
    pub fn hotkeys(mut self, hotkeys: HotkeyManager) -> Self {
        self.hotkeys = Some(hotkeys);
        self
    }

    /// Handler for global hotkeys.
    ///
    /// Fires for both the press and the release edge; `a.is_pressed()` picks
    /// the common one.
    pub fn on_hotkey(mut self, handler: impl FnMut(&HotkeyActivation) -> Dirty + 'static) -> Self {
        self.hotkey_fn = Some(Box::new(handler));
        self
    }

    /// How much of the OS titlebar to keep (INTEGRASI-NATIVE §1).
    ///
    /// Titlebar shape is decided when the window is created, so this is a
    /// window-configuration call and not something that can be changed later.
    pub fn titlebar(mut self, style: TitlebarStyle) -> Self {
        self.titlebar = style;
        self
    }

    /// Translucency behind the window (REKOMENDASI §3.6).
    ///
    /// Honoured only while the OS is not asking for reduced transparency; see
    /// [`crate::titlebar::apply_material`]. The window keeps its opaque token
    /// background in that case, so there is always something correct to fall
    /// back to.
    pub fn material(mut self, material: Material) -> Self {
        self.material = material;
        self
    }

    /// Whether the material dims when the window loses focus.
    pub fn material_state(mut self, state: MaterialState) -> Self {
        self.material_state = state;
        self
    }

    /// Move the macOS traffic lights, in logical points from the window's
    /// top-left corner.
    ///
    /// Only meaningful together with a custom [`titlebar`](Self::titlebar). The
    /// shell re-applies the inset after every resize, because AppKit puts the
    /// buttons back where it wants them.
    pub fn traffic_light_inset(mut self, x: f32, y: f32) -> Self {
        self.traffic_light_inset = Some(silka_paint::Point::new(x, y));
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
        let event_loop = EventLoop::<ShellEvent>::with_user_event()
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
/// itself uses this function, the [`Env`](silka_core::app::Env) values the
/// application sees cannot
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
    // The async bridge's return path (§9.6): a task that finishes while the
    // window is idle wakes the loop, and the next frame applies its result. The
    // proxy does not exist yet — `wake_notifier` looks it up per call, so this
    // is installed once, here, and starts working when the loop does.
    app.borrow()
        .tasks()
        .notify_with(crate::event::wake_notifier());

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
            // Reduced motion is part of the animation contract, not a widget's
            // own business (§6): from here every `Tick` carries it, and a
            // change asks for the one frame that lets decorative motion already
            // in flight finish itself off.
            ui.set_motion(ctx.motion());

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

/// The theme the user actually sees: the configured theme with the OS
/// settings folded in (INTEGRASI-NATIVE §6).
///
/// A free function rather than a method so the rule can be tested without a
/// window — the shell is unreachable from a unit test, and "which theme does a
/// frame get?" is exactly the kind of question that must not go untested.
fn tema_efektif(theme: Theme, settings: SystemSettings, accent: AccentSource) -> Theme {
    settings.apply(theme, accent)
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
    /// The installed menubar. Kept here because dropping it takes the menu
    /// down with it (see [`InstalledMenu`]).
    _menu: Option<InstalledMenu>,
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
    images: Option<ImagesRef>,
    access_fn: AccessFn,
    access_action_fn: Option<AccessActionFn>,
    input_fn: Option<InputFn>,
    native_ready_fn: Option<NativeReadyFn>,
    native_event_fn: Option<NativeEventFn>,
    /// Where the accent color comes from (§6).
    accent: AccentSource,
    /// The OS settings in effect, re-read on the events the OS already sends.
    settings: SystemSettings,
    /// `Some` when the application pinned the settings by hand; then nothing
    /// is ever read from the OS.
    settings_pinned: bool,
    store: Option<Box<dyn StateStore>>,
    quit_fn: Option<QuitFn>,
    restore_geometry: bool,
    /// Whether the stored geometry has already been consumed. A window that is
    /// rebuilt mid-session (`suspended` → `resumed`) must come back where the
    /// user last dragged it, not where the *previous run* left it.
    geometri_dipulihkan: bool,
    /// The geometry the window would return to if it were unmaximized right
    /// now — tracked live, because at quit time the window may be minimized
    /// and reporting nonsense.
    placement: WindowPlacement,
    /// State is written exactly once, whichever path the quit takes.
    state_saved: bool,
    /// The menubar description, installed once the window exists.
    menubar: Option<MenuBar>,
    menu_fn: Option<MenuFn>,
    /// The tray description, created once the event loop is running.
    tray_config: Option<TrayConfig>,
    tray_fn: Option<TrayFn>,
    /// The hotkey set, registered once the event loop is running.
    hotkey_config: Option<HotkeyManager>,
    hotkey_fn: Option<HotkeyFn>,
    titlebar: TitlebarStyle,
    material: Material,
    material_state: MaterialState,
    traffic_light_inset: Option<silka_paint::Point>,
    /// Live tray icon. Dropping it removes the icon, so it is owned here for
    /// the shell's whole life rather than by the window state, which is torn
    /// down and rebuilt on suspend/resume.
    tray: Option<Tray>,
    /// Live hotkey registration. Dropping it gives the combinations back to
    /// the rest of the desktop, so it is owned for the shell's whole life.
    hotkeys: Option<HotkeyRegistration>,
    input: WinitInput,
    ime_aktif: bool,
    proxy: EventLoopProxy<ShellEvent>,
    state: Option<ShellState>,
    started: Instant,
    scheduler: FrameScheduler,
    logger: FrameLogger,
    error: Option<PlatformError>,
}

impl Shell {
    fn new(config: WindowConfig, proxy: EventLoopProxy<ShellEvent>) -> Self {
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
            images: config.images,
            access_fn,
            access_action_fn: config.access_action_fn,
            input_fn: config.input_fn,
            native_ready_fn: config.native_ready_fn,
            native_event_fn: config.native_event_fn,
            accent: config.accent,
            settings: config.settings.unwrap_or(SystemSettings::DEFAULT),
            settings_pinned: config.settings.is_some(),
            store: config.store,
            quit_fn: config.quit_fn,
            restore_geometry: config.restore_geometry,
            geometri_dipulihkan: false,
            placement: WindowPlacement::sized(config.size),
            state_saved: false,
            menubar: config.menubar,
            menu_fn: config.menu_fn,
            tray_config: config.tray,
            tray_fn: config.tray_fn,
            hotkey_config: config.hotkeys,
            hotkey_fn: config.hotkey_fn,
            titlebar: config.titlebar,
            material: config.material,
            material_state: config.material_state,
            traffic_light_inset: config.traffic_light_inset,
            tray: None,
            hotkeys: None,
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

    /// The theme as the user actually sees it: preset + appearance, with the
    /// OS accent and the transparency preference applied on top (§6).
    ///
    /// Derived rather than stored, so there is exactly one copy of the truth —
    /// a cached effective theme is how "the accent changed but the focus ring
    /// did not" happens.
    fn tema_efektif(&self) -> Theme {
        tema_efektif(self.theme, self.settings, self.accent)
    }

    /// Read the OS settings and adopt them, without asking for a frame.
    ///
    /// Used at startup, where the first frame is coming anyway.
    fn baca_setelan(&mut self) {
        if self.settings_pinned {
            return;
        }
        self.settings = SystemSettings::read(self.theme.appearance);
        #[cfg(debug_assertions)]
        eprintln!("silka: setelan OS — {}", self.settings.label());
    }

    /// Re-read the OS settings and, if anything moved, schedule the frame that
    /// shows it.
    ///
    /// Called on events the OS already sends — a theme change, the window
    /// regaining focus after a trip to System Settings. **Never on a timer**:
    /// polling would keep the process awake for a setting that changes twice a
    /// year (§3.5).
    fn segarkan_setelan(&mut self) {
        if self.settings_pinned {
            return;
        }
        let baru = SystemSettings::read(self.theme.appearance);
        let dirty = self.settings.diff(&baru);
        if dirty.is_empty() {
            return;
        }
        self.settings = baru;
        self.minta(dirty);
    }

    /// Load the saved geometry and turn it into something safe to open now.
    fn pulihkan_geometri(&mut self, event_loop: &ActiveEventLoop) {
        if !self.restore_geometry || self.geometri_dipulihkan {
            return;
        }
        self.geometri_dipulihkan = true;
        let Some(tersimpan) = self.store.as_ref().and_then(|s| s.load().placement()) else {
            return;
        };
        let monitors: Vec<MonitorArea> = event_loop
            .available_monitors()
            .map(|m| {
                let pos = m.position();
                let size = m.size();
                MonitorArea::new(pos.x, pos.y, size.width, size.height, m.scale_factor())
            })
            .collect();
        let dipulihkan = restore_placement(tersimpan, &monitors);
        self.size = dipulihkan.size;
        self.placement = dipulihkan;
    }

    /// Write down where the window is *right now*.
    ///
    /// Two states are deliberately skipped: a minimized window (whose reported
    /// geometry is meaningless) and a maximized one (whose geometry is the
    /// screen, not the size the user chose). What gets remembered for a
    /// maximized window is therefore the size it will return to — which is what
    /// every well-behaved application does.
    fn rekam_geometri(&mut self) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        if state.window.is_minimized().unwrap_or(false) {
            return;
        }
        self.placement.maximized = state.window.is_maximized();
        self.placement.scale = state.window.scale_factor();
        if self.placement.maximized {
            return;
        }
        let ukuran = state.surface.logical_size();
        if ukuran.width > 0.0 && ukuran.height > 0.0 {
            self.placement.size = ukuran;
        }
        if let Ok(pos) = state.window.outer_position() {
            self.placement.position = Some((pos.x, pos.y));
        }
    }

    /// The quit path: collect the state, offer it to the application, save it.
    ///
    /// Returns whether the application may actually close. Runs its handler and
    /// its save **exactly once**, however many ways the same quit arrives — the
    /// user closing the window is immediately followed by the event loop
    /// exiting, and saving twice would mean the second, emptier save wins.
    fn tutup(&mut self, reason: QuitReason) -> bool {
        if self.state_saved {
            return true;
        }
        // Loaded rather than built from scratch: values the application wrote
        // in an earlier run, and has not touched this time, must survive.
        let mut state: SessionState = self.store.as_ref().map(|s| s.load()).unwrap_or_default();
        self.rekam_geometri();
        state.set_placement(self.placement);

        let mut ctx = QuitContext::new(reason, state);
        if let Some(f) = self.quit_fn.as_mut() {
            f(&mut ctx);
        }
        let (state, dibatalkan) = ctx.finish();
        if dibatalkan {
            // Nothing has been written and nothing has been marked done: a
            // later, real quit still gets its turn.
            return false;
        }
        self.state_saved = true;
        if let Some(store) = self.store.as_ref() {
            if let Err(_e) = store.save(&state) {
                // A state file that cannot be written must never take the
                // application down with it: what is lost is a window position,
                // not the user's work.
                #[cfg(debug_assertions)]
                eprintln!("silka: {_e}");
            }
        }
        true
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
        // Session restore happens **before** the window is created: geometry is
        // a window attribute, and applying it afterwards would show the window
        // jumping into place (INTEGRASI-NATIVE §6).
        self.pulihkan_geometri(event_loop);

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
        // The titlebar shape is fixed at creation on macOS — after this point
        // `fullSizeContentView` can no longer be turned on (§1).
        attrs = apply_titlebar_style(attrs, self.titlebar);
        if let Some((x, y)) = self.placement.position {
            attrs = attrs.with_position(PhysicalPosition::new(x, y));
        }
        if self.placement.maximized {
            attrs = attrs.with_maximized(true);
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

        // …and only now the rest of the §6 settings: the OS accent is a
        // light/dark **pair**, so reading it before the appearance is known
        // would pick the wrong half of it.
        self.baca_setelan();

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

        // The escape hatch, at the only moment that is right for it: the window
        // exists, its surface and a11y adapter exist, and nothing has been shown
        // yet — so a transparent titlebar or an extended frame is applied
        // *before* the first pixel instead of one frame late (§8).
        if let Some(ready) = self.native_ready_fn.as_mut() {
            ready(&NativeWindow::new(window.clone()));
        }

        // Native integration, still before the first pixel (INTEGRASI-NATIVE
        // §1–§2). All three are cosmetic-or-nothing: a menubar the OS refuses
        // or a material the OS has no support for must not stop an application
        // from opening its window, so each failure is reported and stepped
        // over rather than propagated.
        forward_native_events(self.proxy.clone());

        let menu = match self.menubar.as_ref().map(|bar| bar.install(&window)) {
            Some(Ok(m)) => Some(m),
            Some(Err(e)) => {
                eprintln!("silka: menubar tidak terpasang — {e}");
                None
            }
            None => None,
        };

        if self.material != Material::None {
            if let Err(e) = apply_material(&window, self.material, self.material_state) {
                eprintln!("silka: material tidak terpasang — {e}");
            }
        }
        self.pasang_traffic_light(&window);

        if self.tray.is_none() {
            if let Some(cfg) = self.tray_config.take() {
                match cfg.install() {
                    Ok(t) => self.tray = Some(t),
                    Err(e) => eprintln!("silka: tray tidak terpasang — {e}"),
                }
            }
        }

        // Global hotkeys, at the first moment the OS will accept them: the
        // loop exists and this is its thread (§3). Registered once and kept —
        // suspend/resume rebuilds the window, not the desktop-wide shortcuts.
        if self.hotkeys.is_none() {
            if let Some(set) = self.hotkey_config.take() {
                match set.register() {
                    Ok(r) => self.hotkeys = Some(r),
                    Err(e) => eprintln!("silka: hotkey global tidak terdaftar — {e}"),
                }
            }
        }

        // Everything is ready — only now may the window become visible.
        window.set_visible(true);

        self.state = Some(ShellState {
            window,
            _menu: menu,
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

    /// Offer one raw window event to the native hook (INTEGRASI-NATIVE §8).
    ///
    /// An application without a hook pays nothing at all: no `NativeWindow` is
    /// built, not even the reference-count bump.
    fn hook_native(&mut self, event: &WindowEvent) -> NativeFlow {
        let Shell {
            native_event_fn,
            state,
            ..
        } = self;
        let (Some(hook), Some(state)) = (native_event_fn.as_mut(), state.as_ref()) else {
            return NativeFlow::Continue;
        };
        let native = NativeWindow::new(state.window.clone());
        hook(&NativeEvent::new(&native, event))
    }

    /// Re-apply the traffic-light inset.
    ///
    /// Called at creation **and after every resize**: AppKit re-lays out the
    /// titlebar container whenever the window changes size or enters
    /// fullscreen, which puts the buttons back where it wants them. Doing this
    /// only once at startup is the classic way to get a custom titlebar that
    /// looks right until the user drags a corner.
    fn pasang_traffic_light(&self, window: &Window) {
        if let Some(inset) = self.traffic_light_inset {
            set_traffic_light_inset(window, inset);
        }
    }

    /// Route a menu activation into the application.
    fn menu(&mut self, activation: MenuActivation) {
        let Some(f) = self.menu_fn.as_mut() else {
            return;
        };
        let dirty = f(&activation);
        if !dirty.is_empty() {
            self.minta(dirty);
        }
    }

    /// Route a global hotkey into the application.
    fn hotkey_event(&mut self, activation: HotkeyActivation) {
        let Some(f) = self.hotkey_fn.as_mut() else {
            return;
        };
        let dirty = f(&activation);
        if !dirty.is_empty() {
            self.minta(dirty);
        }
    }

    /// Route a tray gesture into the application.
    fn tray_event(&mut self, activation: TrayActivation) {
        let Some(f) = self.tray_fn.as_mut() else {
            return;
        };
        let dirty = f(&activation);
        if !dirty.is_empty() {
            self.minta(dirty);
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
        // The theme the frame is drawn with is the *effective* one — preset and
        // appearance with the OS accent and transparency preference already
        // folded in (§6). Computed here, once, so no caller can accidentally
        // paint with the raw configured theme.
        let efektif = self.tema_efektif();
        let setelan = self.settings;
        let Shell {
            state,
            scheduler,
            scene_fn,
            glyphs,
            images,
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
        // One reference-count bump per frame buys the frame closure the escape
        // hatch (§8) — and, because it is an owned handle, the guarantee that
        // the window outlives every pointer read from it.
        let native = NativeWindow::new(state.window.clone());
        let ctx = FrameContext {
            theme: &efektif,
            size: state.surface.logical_size(),
            scale_factor: state.surface.scale_factor(),
            frame: start.index(),
            elapsed: started.elapsed(),
            vsync: scheduler.vsync(),
            animate: &animate,
            native: Some(&native),
            settings: setelan,
        };
        let scene = (scene_fn)(&ctx);

        // The boundary between our work and the swapchain queue. Without this
        // marker, time spent waiting for vsync would be recorded as a "slow
        // frame" when it is in fact a sign of a healthy system.
        start.mark_built(Instant::now());

        // Wayland wants to know before the buffer is attached; a no-op
        // elsewhere.
        state.window.pre_present_notify();
        // Both atlases are borrowed ONLY while drawing — the scene closure is
        // already done with them, so there are never two borrowers at once.
        let hasil = match (glyphs.as_ref(), images.as_ref()) {
            (Some(g), Some(i)) => {
                let mut teks = g.borrow_mut();
                let mut gambar = i.borrow_mut();
                state
                    .surface
                    .render_with_sources(&state.gpu, &scene, &mut *teks, &mut *gambar)
            }
            (Some(g), None) => {
                let mut teks = g.borrow_mut();
                state
                    .surface
                    .render_with_glyphs(&state.gpu, &scene, &mut *teks)
            }
            (None, Some(i)) => {
                let mut gambar = i.borrow_mut();
                state.surface.render_with_sources(
                    &state.gpu,
                    &scene,
                    &mut silka_paint::NoGlyphs,
                    &mut *gambar,
                )
            }
            (None, None) => state.surface.render(&state.gpu, &scene),
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

impl ApplicationHandler<ShellEvent> for Shell {
    /// The return path from assistive technology.
    ///
    /// `accesskit_winit` calls its handler on any thread; the winit event loop
    /// is the official channel back to the UI thread.
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: ShellEvent) {
        // Menus, the tray and global hotkeys are application-wide, not
        // window-owned: they are answered before the window-id check that
        // accessibility needs.
        let event = match event {
            ShellEvent::Access(e) => e,
            ShellEvent::Menu(a) => return self.menu(a),
            ShellEvent::Tray(a) => return self.tray_event(a),
            ShellEvent::Hotkey(a) => return self.hotkey_event(a),
            // A background task delivered something (§9.6). The payload is
            // already in the channel; one frame is all that is needed, and
            // `AppRuntime::frame` applies it before it drains the dirty scopes.
            ShellEvent::Wake => return self.minta(Dirty::LAYOUT | Dirty::PAINT),
        };
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

    /// The event loop is ending: Cmd+Q, a quit from the menu, or the OS
    /// logging the user out (INTEGRASI-NATIVE §6).
    ///
    /// This is the last line of defence for session state — and the only one
    /// on the paths that never produce a `CloseRequested`. Saving is idempotent
    /// ([`Shell::tutup`]), so arriving here after a normal window close costs
    /// nothing.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        let _ = self.tutup(QuitReason::Exiting);
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

        // The escape hatch gets first refusal (INTEGRASI-NATIVE §8): the
        // application sees the raw event before the framework acts on it, and
        // may keep it.
        let alur = self.hook_native(&event);

        // The a11y adapter sees the event next — even when the hook consumed
        // it. It only *observes* window focus and geometry; dropping it there
        // would leave a screen reader with a tree that quietly drifts out of
        // step with the window, a bug nobody would trace back to their own hook
        // (§3.8).
        if let Some(state) = self.state.as_mut() {
            let window = state.window.clone();
            state.access.process_event(&window, &event);
        }

        if alur.is_consumed() {
            return;
        }

        match event {
            // The one moment state can still be saved (§6). A handler may
            // refuse — an unsaved document — and then the window simply stays.
            WindowEvent::CloseRequested => {
                if self.tutup(QuitReason::CloseRequested) {
                    event_loop.exit();
                }
            }

            // Where the window is now, remembered while it is still true: at
            // quit time the window may be minimized and reporting nonsense.
            WindowEvent::Moved(_) => self.rekam_geometri(),

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
                    self.rekam_geometri();
                    // AppKit moved the traffic lights back during its own
                    // titlebar relayout; put them where the application asked.
                    if let Some(window) = self.state.as_ref().map(|s| s.window.clone()) {
                        self.pasang_traffic_light(&window);
                    }
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
                // The accent is a light/dark pair, so it moves *with* the
                // appearance even when the user changed nothing else (§6).
                self.segarkan_setelan();
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

            // Coming back from a trip to System Settings is exactly when the
            // accent or the reduce-motion switch has just changed. Re-reading
            // here is what makes those settings live without a single timer
            // (§6, §3.5).
            WindowEvent::Focused(true) => self.segarkan_setelan(),

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
///
/// ```
/// use silka_platform::default_clear_color;
/// use silka_theme::{Appearance, Theme};
///
/// // It really is a token, not a literal — which is why light and dark
/// // differ and why a preset swap changes it.
/// let light = Theme::cupertino(Appearance::Light);
/// let dark = Theme::cupertino(Appearance::Dark);
/// assert_eq!(default_clear_color(&light), light.color.background);
/// assert_ne!(default_clear_color(&light), default_clear_color(&dark));
/// ```
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
        // Nor does one without images.
        assert!(window("Tanpa gambar").images.is_none());
    }

    #[test]
    fn sumber_gambar_terpasang_lewat_method_chaining() {
        // The image atlas travels the same way the glyph atlas does: shared, not
        // copied, so the bitmap a scene closure inserts is the one the backend
        // uploads on the very same frame.
        let atlas = Rc::new(RefCell::new(silka_paint::ImageAtlas::new()));
        let config = window("Dengan gambar").images(atlas.clone());
        let terpasang = config.images.expect("sumber gambar tersimpan");
        assert!(Rc::ptr_eq(
            &(atlas as Rc<RefCell<dyn ImageSource>>),
            &terpasang
        ));
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
            // Headless: there is no window, and a test must not be able to
            // pretend there is one.
            native: None,
            settings: SystemSettings::DEFAULT,
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
            // Headless: there is no window, and a test must not be able to
            // pretend there is one.
            native: None,
            settings: SystemSettings::DEFAULT,
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
            native: None,
            settings: SystemSettings::DEFAULT,
        }
    }

    /// [`frame_ctx`] with the OS settings of the day (INTEGRASI-NATIVE §6).
    fn frame_ctx_dengan<'a>(
        theme: &'a Theme,
        animate: &'a Cell<bool>,
        settings: SystemSettings,
    ) -> FrameContext<'a> {
        FrameContext {
            settings,
            ..frame_ctx(theme, animate)
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
    fn tanpa_hook_native_aplikasi_tidak_membayar_apa_apa() {
        // The escape hatch is opt-in: an application that never asks for it
        // does not even get a `NativeWindow` built (INTEGRASI-NATIVE §8).
        let c = window("Uji");
        assert!(c.native_ready_fn.is_none());
        assert!(c.native_event_fn.is_none());
    }

    #[test]
    fn hook_native_terpasang_lewat_method_chaining() {
        let c = window("Uji")
            .on_native_ready(|_| {})
            .on_native_event(|_| NativeFlow::Continue);
        assert!(c.native_ready_fn.is_some());
        assert!(c.native_event_fn.is_some());
    }

    #[test]
    fn frame_headless_tidak_punya_window_native() {
        // A test must never be able to reach a window that does not exist —
        // hence `Option`, not a stub (§9.5).
        let theme = Theme::cupertino(Appearance::Light);
        let animate = Cell::new(false);
        let ctx = frame_ctx(&theme, &animate);
        assert!(ctx.native().is_none());
    }

    #[test]
    fn nilai_bawaan_lifecycle_masuk_akal() {
        let c = window("Uji");
        // The OS accent is followed by default — an app that wants its own
        // brand has to say so, which is the way round that makes the *silent*
        // choice the native-looking one (§6).
        assert_eq!(c.accent, AccentSource::System);
        // Nothing is pinned, nothing is persisted, but a store that is added
        // later restores geometry without further ceremony.
        assert!(c.settings.is_none());
        assert!(c.store.is_none());
        assert!(c.quit_fn.is_none());
        assert!(c.restore_geometry);
    }

    #[test]
    fn sumber_aksen_diatur_lewat_method_chaining() {
        assert_eq!(window("Uji").preset_accent().accent, AccentSource::Preset);
        assert_eq!(
            window("Uji").accent(Color::hex(0x7C3AED)).accent,
            AccentSource::Custom(Color::hex(0x7C3AED))
        );
        assert_eq!(
            window("Uji").preset_accent().follow_system_accent().accent,
            AccentSource::System
        );
    }

    #[test]
    fn setelan_yang_dipatok_menggantikan_pembacaan_os() {
        let dipatok = SystemSettings {
            motion: silka_core::animation::Motion::Reduced,
            ..SystemSettings::DEFAULT
        };
        let c = window("Cuplikan").system_settings(dipatok);
        assert_eq!(c.settings, Some(dipatok));
    }

    #[test]
    fn theme_frame_memakai_aksen_os() {
        // The whole §6 accent path in one line: what the OS reported must be
        // what the frame is painted with.
        let settings = SystemSettings {
            accent: Some(Color::hex(0xFF375F)),
            ..SystemSettings::DEFAULT
        };
        let t = tema_efektif(
            Theme::cupertino(Appearance::Dark),
            settings,
            AccentSource::System,
        );
        assert_eq!(t.color.accent, Color::hex(0xFF375F));
        assert_eq!(t.color.focus_ring.with_alpha(1.0), Color::hex(0xFF375F));
    }

    #[test]
    fn aplikasi_bermerek_tidak_ikut_berubah_dengan_aksen_os() {
        let settings = SystemSettings {
            accent: Some(Color::hex(0xFF375F)),
            ..SystemSettings::DEFAULT
        };
        let asal = Theme::tailwind(Appearance::Light);
        assert_eq!(tema_efektif(asal, settings, AccentSource::Preset), asal);
        assert_eq!(
            tema_efektif(asal, settings, AccentSource::Custom(Color::hex(0x7C3AED)))
                .color
                .accent,
            Color::hex(0x7C3AED)
        );
    }

    #[test]
    fn reduce_transparency_sampai_ke_theme_frame() {
        let settings = SystemSettings {
            transparency: silka_theme::Transparency::Reduced,
            ..SystemSettings::DEFAULT
        };
        let t = tema_efektif(
            Theme::cupertino(Appearance::Dark),
            settings,
            AccentSource::System,
        );
        assert_eq!(
            t.color.surface_hover.a, 1.0,
            "tidak boleh tersisa tembus pandang"
        );
    }

    #[test]
    fn setelan_bawaan_tidak_menyentuh_theme_sama_sekali() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                assert_eq!(
                    tema_efektif(t, SystemSettings::DEFAULT, AccentSource::System),
                    t
                );
            }
        }
    }

    #[test]
    fn reduced_motion_os_sampai_ke_tick_yang_dilihat_widget() {
        use silka_core::animation::Motion;
        use silka_core::view::fixed;
        use std::rc::Rc;

        // The whole chain: the OS setting → `FrameContext::motion` →
        // `AppRuntime::set_motion` → the `Tick` every animated widget reads.
        let terlihat: Rc<Cell<Option<Motion>>> = Rc::default();
        let catat = terlihat.clone();
        let mut c = sambungkan_app_with(
            window("Uji"),
            |_cx| fixed(40.0, 20.0).into(),
            move |_tree, tick| {
                catat.set(Some(tick.motion()));
                Dirty::NONE
            },
        );

        let theme = Theme::cupertino(Appearance::Light);
        let animate = Cell::new(false);
        let f = c.scene_fn.as_mut().expect("scene_fn terpasang");

        let _ = f(&frame_ctx_dengan(&theme, &animate, SystemSettings::DEFAULT));
        assert_eq!(terlihat.get(), Some(Motion::Full));

        let dikurangi = SystemSettings {
            motion: Motion::Reduced,
            ..SystemSettings::DEFAULT
        };
        let _ = f(&frame_ctx_dengan(&theme, &animate, dikurangi));
        assert_eq!(
            terlihat.get(),
            Some(Motion::Reduced),
            "widget harus melihat preferensi OS tanpa bertanya sendiri"
        );
    }

    #[test]
    fn perubahan_reduced_motion_diterapkan_dalam_frame_yang_sama() {
        use silka_core::animation::Motion;
        use silka_core::view::fixed;

        // The motion preference is handed over *before* rebuild → layout →
        // paint, so the frame that carries the change is already the frame that
        // shows it. The window must therefore go straight back to idle: a
        // setting the user changes twice a year may not leave the renderer
        // ticking (§3.5).
        //
        // The other half — waking an idle window when the setting changes — is
        // the shell's `segarkan_setelan`, and rests on
        // `SystemSettings::diff` naming `Dirty::ANIMATION`
        // (`lifecycle::tests::diff_hanya_menandai_yang_benar_benar_berubah`).
        let mut c = sambungkan_app(window("Uji"), |_cx| fixed(40.0, 20.0).into());
        let theme = Theme::cupertino(Appearance::Light);
        let animate = Cell::new(false);
        let f = c.scene_fn.as_mut().expect("scene_fn terpasang");

        let _ = f(&frame_ctx_dengan(&theme, &animate, SystemSettings::DEFAULT));
        assert!(!animate.get(), "tanpa perubahan, window kembali idle");

        let dikurangi = SystemSettings {
            motion: Motion::Reduced,
            ..SystemSettings::DEFAULT
        };
        let _ = f(&frame_ctx_dengan(&theme, &animate, dikurangi));
        assert!(
            !animate.get(),
            "perubahan setelan tidak boleh menyisakan frame yang berputar"
        );
    }

    #[test]
    fn frame_membawa_setelan_os_apa_adanya() {
        let theme = Theme::cupertino(Appearance::Light);
        let animate = Cell::new(false);
        let settings = SystemSettings {
            accent: Some(Color::hex(0x30D158)),
            motion: silka_core::animation::Motion::Reduced,
            ..SystemSettings::DEFAULT
        };
        let ctx = frame_ctx_dengan(&theme, &animate, settings);
        assert_eq!(ctx.settings(), settings);
        assert_eq!(ctx.motion(), silka_core::animation::Motion::Reduced);
    }

    #[test]
    fn store_dan_handler_quit_terpasang_lewat_method_chaining() {
        let c = window("Uji")
            .restore_state(crate::lifecycle::MemoryStore::new())
            .on_quit(|q| q.remember("halaman", "chart"))
            .restore_geometry(false);
        assert!(c.store.is_some());
        assert!(c.quit_fn.is_some());
        assert!(!c.restore_geometry);
    }

    #[test]
    fn handler_quit_menulis_ke_state_yang_akan_disimpan() {
        // The handler's shape, exercised without a window: what it writes is
        // what the store would receive.
        let mut c = window("Uji").on_quit(|q| {
            q.remember("dokumen", "/tmp/a.txt");
            if q.reason() == QuitReason::CloseRequested {
                q.remember("lewat", "tombol tutup");
            }
        });
        let f = c.quit_fn.as_mut().expect("on_quit terpasang");

        let mut ctx = QuitContext::new(QuitReason::CloseRequested, SessionState::new());
        f(&mut ctx);
        let (state, dibatalkan) = ctx.finish();
        assert!(!dibatalkan);
        assert_eq!(state.get("dokumen"), Some("/tmp/a.txt"));
        assert_eq!(state.get("lewat"), Some("tombol tutup"));
    }

    #[test]
    fn nilai_bawaan_native_tidak_mengubah_apa_pun() {
        // A window that asks for nothing native must look exactly like a plain
        // window: no menubar installed, no tray icon, OS titlebar, no material.
        let c = window("Uji");
        assert!(c.menubar.is_none());
        assert!(c.menu_fn.is_none());
        assert!(c.tray.is_none());
        assert!(c.tray_fn.is_none());
        assert_eq!(c.titlebar, TitlebarStyle::Native);
        assert_eq!(c.material, Material::None);
        assert!(c.traffic_light_inset.is_none());
    }

    #[test]
    fn menubar_terpasang_lewat_method_chaining_dengan_edit_menu() {
        use crate::menu::{item, menu, menubar};
        let c =
            window("Uji").menubar(menubar("Uji").menu(menu("File").item(item("file.new", "New"))));
        let bar = c.menubar.as_ref().expect("menubar tersimpan");
        assert!(
            bar.has_standard_edit_menu(),
            "⌘C/⌘V butuh Edit menu standar"
        );
        assert!(bar.ids().iter().any(|i| i.as_str() == "file.new"));
    }

    #[test]
    fn handler_menu_menentukan_apakah_ada_frame() {
        use crate::menu::MenuActivation;
        // The shell has no way to know whether a handler changed anything, so
        // the handler says — and "nothing changed" must leave the window
        // asleep (§3.5).
        let mut c = window("Uji").on_menu(|a| {
            if a.is("ubah") {
                Dirty::LAYOUT
            } else {
                Dirty::NONE
            }
        });
        let f = c.menu_fn.as_mut().expect("menu_fn terpasang");
        assert_eq!(f(&MenuActivation::new("ubah")), Dirty::LAYOUT);
        assert!(f(&MenuActivation::new("lain")).is_empty());
    }

    #[test]
    fn handler_tray_memakai_kontrak_yang_sama() {
        let mut c = window("Uji").on_tray(|_| Dirty::PAINT);
        let f = c.tray_fn.as_mut().expect("tray_fn terpasang");
        assert_eq!(
            f(&TrayActivation::Leave { id: "utama".into() }),
            Dirty::PAINT
        );
    }

    #[test]
    fn titlebar_dan_material_tersimpan_apa_adanya() {
        let c = window("Uji")
            .titlebar(TitlebarStyle::Transparent)
            .material(Material::Sidebar)
            .material_state(MaterialState::Active)
            .traffic_light_inset(20.0, 24.0);
        assert!(c.titlebar.is_custom());
        assert!(c.titlebar.has_window_buttons());
        assert_eq!(c.material, Material::Sidebar);
        assert_eq!(c.material_state, MaterialState::Active);
        assert_eq!(
            c.traffic_light_inset,
            Some(silka_paint::Point::new(20.0, 24.0))
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
            // Headless: there is no window, and a test must not be able to
            // pretend there is one.
            native: None,
            settings: SystemSettings::DEFAULT,
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
            // Headless: there is no window, and a test must not be able to
            // pretend there is one.
            native: None,
            settings: SystemSettings::DEFAULT,
        };
        assert!(!animate.get(), "frame tanpa animasi harus kembali idle");
        ctx.request_animation_frame();
        assert!(animate.get());
    }
}
