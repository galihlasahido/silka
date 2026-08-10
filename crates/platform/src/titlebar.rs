//! Custom titlebar and blur-behind-window (INTEGRASI-NATIVE §1, REKOMENDASI §3.6).
//!
//! Two things live here, because on macOS they are one decision: a window that
//! draws its own titlebar is also a window whose background the OS blurs.
//!
//! - [`TitlebarStyle`] — how much of the OS titlebar to keep. The macOS
//!   "custom titlebar" is not a hack: it is `titlebarAppearsTransparent` +
//!   `fullSizeContentView`, so the content view extends under the titlebar and
//!   the application draws the whole window itself while the traffic lights
//!   stay real, native, and keyboard-reachable.
//! - [`Material`] — translucency behind the window: `NSVisualEffectView` on
//!   macOS, Mica/Acrylic on Windows.
//!
//! ## Reduce transparency is not optional
//!
//! macOS and Windows both let a user ask for less transparency, and it is an
//! accessibility setting, not a preference. [`apply_material`] honours it by
//! default: when the OS says reduce, no material is applied and the window
//! keeps its opaque token background (§2.7 — the background always comes from
//! a token, so there is something correct to fall back to). An application that
//! genuinely must override this can call [`force_material`].
//!
//! ## Traffic lights
//!
//! With `fullSizeContentView` the close/minimise/zoom buttons keep their
//! default position, which is usually wrong for a custom titlebar: they end up
//! sitting on top of application content. [`set_traffic_light_inset`] moves
//! them, and [`traffic_light_area`] reports the rectangle the application must
//! leave clear. **The inset has to be re-applied after a resize or a fullscreen
//! transition** — AppKit puts the buttons back where it wants them — which is
//! why the shell calls it again on every resize rather than only at startup.

use silka_paint::{Point, Rect};

/// How much of the OS titlebar to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TitlebarStyle {
    /// The ordinary OS titlebar.
    #[default]
    Native,
    /// The titlebar is transparent and the content view extends under it: the
    /// application draws the whole window, and the traffic lights stay native.
    ///
    /// This is the "custom titlebar" of INTEGRASI-NATIVE §1.
    Transparent,
    /// No titlebar and no window buttons at all.
    ///
    /// Everything — dragging, closing, resizing — becomes the application's
    /// problem. Only sensible for a window that is not a document window, such
    /// as a HUD or a splash screen.
    Hidden,
}

impl TitlebarStyle {
    /// Whether the application is responsible for drawing the titlebar area.
    pub fn is_custom(self) -> bool {
        !matches!(self, TitlebarStyle::Native)
    }

    /// Whether the OS still draws the close/minimise/zoom buttons.
    pub fn has_window_buttons(self) -> bool {
        !matches!(self, TitlebarStyle::Hidden)
    }
}

/// Apply a titlebar style to the attributes a window is about to be created
/// with.
///
/// Titlebar shape is fixed at creation on macOS, so this has to happen while
/// the window is still a description — which is also why it takes and returns
/// the attributes rather than a live window.
pub fn apply_titlebar_style(
    attributes: winit::window::WindowAttributes,
    style: TitlebarStyle,
) -> winit::window::WindowAttributes {
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::WindowAttributesExtMacOS;
        match style {
            TitlebarStyle::Native => attributes,
            TitlebarStyle::Transparent => attributes
                .with_titlebar_transparent(true)
                .with_fullsize_content_view(true)
                .with_title_hidden(true),
            TitlebarStyle::Hidden => attributes
                .with_titlebar_hidden(true)
                .with_titlebar_buttons_hidden(true)
                .with_fullsize_content_view(true)
                .with_title_hidden(true),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Windows keeps its frame (DWM extension is a separate, later job) and
        // Wayland CSD is drawn by winit itself; only `Hidden` has a portable
        // meaning.
        match style {
            TitlebarStyle::Hidden => attributes.with_decorations(false),
            _ => attributes,
        }
    }
}

/// Translucency behind the window.
///
/// Named for what the surface *is*, not for the platform effect that
/// implements it — the same rule the theme tokens follow (§2.7). Each variant
/// maps to an `NSVisualEffectMaterial` on macOS and to Mica or Acrylic on
/// Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Material {
    /// No translucency: the window background is opaque.
    #[default]
    None,
    /// The window's own background — the whole-window material.
    Window,
    /// A sidebar sitting beside content.
    Sidebar,
    /// A toolbar or header strip at the top of a window.
    HeaderView,
    /// A popover attached to a control.
    Popover,
    /// A menu surface.
    Menu,
    /// A floating HUD panel.
    Hud,
    /// A tooltip.
    Tooltip,
    /// A modal sheet.
    Sheet,
    /// The area *behind* the window, showing the desktop through it.
    UnderWindow,
}

/// When the material stays lit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaterialState {
    /// Bright while the window is active, muted when it is not — the native
    /// behaviour, and the one users read as "this window is in front".
    #[default]
    FollowsWindow,
    /// Always active.
    Active,
    /// Always inactive.
    Inactive,
}

/// Why a material could not be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum VibrancyError {
    /// This platform has no blur-behind-window at all.
    Unsupported(String),
    /// The OS version is too old for this effect.
    TooOld(String),
    /// The call did not happen on the UI thread.
    NotMainThread(String),
    /// The window handle could not be read.
    NoWindowHandle(String),
}

impl core::fmt::Display for VibrancyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            VibrancyError::Unsupported(m) => write!(f, "material tidak didukung: {m}"),
            VibrancyError::TooOld(m) => write!(f, "versi OS terlalu lama: {m}"),
            VibrancyError::NotMainThread(m) => {
                write!(f, "material harus dipasang di UI thread: {m}")
            }
            VibrancyError::NoWindowHandle(m) => write!(f, "handle window tidak terbaca: {m}"),
        }
    }
}

impl std::error::Error for VibrancyError {}

fn dari_vibrancy(e: window_vibrancy::Error) -> VibrancyError {
    match e {
        window_vibrancy::Error::UnsupportedPlatform(m) => VibrancyError::Unsupported(m.to_string()),
        window_vibrancy::Error::UnsupportedPlatformVersion(m) => {
            VibrancyError::TooOld(m.to_string())
        }
        window_vibrancy::Error::NotMainThread(m) => VibrancyError::NotMainThread(m.to_string()),
        window_vibrancy::Error::NoWindowHandle(e) => VibrancyError::NoWindowHandle(e.to_string()),
    }
}

#[cfg(target_os = "macos")]
fn material_macos(material: Material) -> Option<window_vibrancy::NSVisualEffectMaterial> {
    use window_vibrancy::NSVisualEffectMaterial as M;
    Some(match material {
        Material::None => return None,
        Material::Window => M::WindowBackground,
        Material::Sidebar => M::Sidebar,
        Material::HeaderView => M::HeaderView,
        Material::Popover => M::Popover,
        Material::Menu => M::Menu,
        Material::Hud => M::HudWindow,
        Material::Tooltip => M::Tooltip,
        Material::Sheet => M::Sheet,
        Material::UnderWindow => M::UnderWindowBackground,
    })
}

#[cfg(target_os = "macos")]
fn state_macos(state: MaterialState) -> window_vibrancy::NSVisualEffectState {
    use window_vibrancy::NSVisualEffectState as S;
    match state {
        MaterialState::FollowsWindow => S::FollowsWindowActiveState,
        MaterialState::Active => S::Active,
        MaterialState::Inactive => S::Inactive,
    }
}

/// Whether the OS has been asked to reduce transparency (INTEGRASI-NATIVE §6).
///
/// `false` on platforms with no such setting — the honest answer, since there
/// is nothing to respect there.
pub fn system_reduces_transparency() -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        // The setting is read fresh on every call rather than cached: a user
        // can turn it on while the application is running, and a cached `false`
        // would keep the blur on for the rest of the session.
        NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency()
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// Apply a material behind the window, honouring "reduce transparency".
///
/// Returns `Ok(false)` when nothing was applied because the user asked for
/// reduced transparency, and `Ok(true)` when the material is live. Applying
/// [`Material::None`] clears whatever was there.
pub fn apply_material(
    window: &winit::window::Window,
    material: Material,
    state: MaterialState,
) -> Result<bool, VibrancyError> {
    if system_reduces_transparency() {
        clear_material(window)?;
        return Ok(false);
    }
    force_material(window, material, state)?;
    Ok(material != Material::None)
}

/// Apply a material **even when the user asked for reduced transparency**.
///
/// Separate function, and deliberately more awkward to reach than
/// [`apply_material`]: overriding an accessibility setting should be a decision
/// somebody made, not a default somebody inherited.
pub fn force_material(
    #[allow(unused_variables)] window: &winit::window::Window,
    material: Material,
    #[allow(unused_variables)] state: MaterialState,
) -> Result<(), VibrancyError> {
    if material == Material::None {
        return clear_material(window);
    }

    #[cfg(target_os = "macos")]
    {
        let m = material_macos(material).expect("Material::None sudah ditangani di atas");
        window_vibrancy::apply_vibrancy(window, m, Some(state_macos(state)), None)
            .map_err(dari_vibrancy)
    }

    #[cfg(target_os = "windows")]
    {
        // Mica is the whole-window material on Windows 11; everything smaller
        // than a window is Acrylic.
        match material {
            Material::Window => window_vibrancy::apply_mica(window, None),
            _ => window_vibrancy::apply_acrylic(window, None),
        }
        .map_err(dari_vibrancy)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Err(VibrancyError::Unsupported(
        "blur behind-window Linux butuh protokol khusus kompositor".into(),
    ))
}

/// Remove any material previously applied to the window.
pub fn clear_material(
    #[allow(unused_variables)] window: &winit::window::Window,
) -> Result<(), VibrancyError> {
    #[cfg(target_os = "macos")]
    {
        window_vibrancy::clear_vibrancy(window).map_err(dari_vibrancy)?;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        // Whichever of the two was applied; clearing the other is a no-op.
        let _ = window_vibrancy::clear_mica(window);
        let _ = window_vibrancy::clear_acrylic(window);
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Ok(())
}

/// Where the macOS traffic lights sit, in **logical points** relative to the
/// window's top-left corner.
///
/// `None` on platforms without them, and on a window whose buttons are hidden.
/// The application uses this to keep its own content out of their way — the
/// whole point of a custom titlebar is that the framework, not AppKit, decides
/// what fills that strip.
pub fn traffic_light_area(
    #[allow(unused_variables)] window: &winit::window::Window,
) -> Option<Rect> {
    #[cfg(target_os = "macos")]
    {
        macos::traffic_light_area(window)
    }
    #[cfg(not(target_os = "macos"))]
    None
}

/// Move the macOS traffic lights to sit `inset` points from the window's
/// top-left corner.
///
/// Returns whether anything moved. **Call it again after every resize**: AppKit
/// re-lays out the titlebar container and puts the buttons back.
pub fn set_traffic_light_inset(
    #[allow(unused_variables)] window: &winit::window::Window,
    #[allow(unused_variables)] inset: Point,
) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::set_traffic_light_inset(window, inset)
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// The default inset for a custom titlebar, in logical points.
///
/// Matches where AppKit puts the buttons in a standard document window, so a
/// window that only wants `fullSizeContentView` (and not a *moved* set of
/// buttons) can pass this and get the native look back.
pub const DEFAULT_TRAFFIC_LIGHT_INSET: Point = Point::new(20.0, 20.0);

#[cfg(target_os = "macos")]
mod macos {
    //! AppKit for the parts winit does not cover (INTEGRASI-NATIVE §8: platform
    //! code in a public API is normal here, not a disgrace).

    use objc2::rc::Retained;
    use objc2_app_kit::{NSView, NSWindow, NSWindowButton};
    use objc2_foundation::{NSPoint, NSRect};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use silka_paint::{Point, Rect};

    /// The three buttons, in the order AppKit lays them out.
    const TOMBOL: [NSWindowButton; 3] = [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ];

    fn ns_window(window: &winit::window::Window) -> Option<Retained<NSWindow>> {
        let handle = window.window_handle().ok()?;
        let RawWindowHandle::AppKit(h) = handle.as_raw() else {
            return None;
        };
        // SAFETY: winit hands out the `NSView` of a live window, and the
        // borrowed `window` keeps it alive for this call.
        let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
        view.window()
    }

    /// The union of the three buttons' frames, converted to top-left origin.
    pub(super) fn traffic_light_area(window: &winit::window::Window) -> Option<Rect> {
        let ns = ns_window(window)?;
        let mut kiri = f64::MAX;
        let mut kanan = f64::MIN;
        let mut atas = f64::MIN;
        let mut bawah = f64::MAX;
        let mut ada = false;

        for kind in TOMBOL {
            let Some(button) = ns.standardWindowButton(kind) else {
                continue;
            };
            // The frame is in the titlebar container's coordinates, which is
            // what `set_traffic_light_inset` writes back to; converting to
            // window coordinates keeps the two consistent.
            let NSRect { origin, size } = button.frame();
            kiri = kiri.min(origin.x);
            kanan = kanan.max(origin.x + size.width);
            bawah = bawah.min(origin.y);
            atas = atas.max(origin.y + size.height);
            ada = true;
        }
        if !ada {
            return None;
        }

        // AppKit's y grows upward from the bottom of the *container*; the
        // framework above speaks top-left down. The container's own height is
        // the conversion factor.
        let tinggi_kontainer = tinggi_kontainer(&ns).unwrap_or(atas);
        Some(Rect::new(
            kiri as f32,
            (tinggi_kontainer - atas) as f32,
            (kanan - kiri) as f32,
            (atas - bawah) as f32,
        ))
    }

    fn tinggi_kontainer(ns: &NSWindow) -> Option<f64> {
        let close = ns.standardWindowButton(NSWindowButton::CloseButton)?;
        // SAFETY: reading the superview of a live button; AppKit owns it and
        // the retained value keeps it alive for the read.
        let kontainer = unsafe { close.superview() }?;
        Some(kontainer.frame().size.height)
    }

    /// Move the three buttons so the leftmost one starts at `inset`.
    pub(super) fn set_traffic_light_inset(window: &winit::window::Window, inset: Point) -> bool {
        let Some(ns) = ns_window(window) else {
            return false;
        };
        let Some(tinggi) = tinggi_kontainer(&ns) else {
            return false;
        };

        // The horizontal gap between buttons is AppKit's, not ours: reproduce
        // whatever spacing the current system uses instead of hardcoding one
        // that will be wrong on the next macOS.
        let mut asal_x: Vec<f64> = Vec::with_capacity(TOMBOL.len());
        for kind in TOMBOL {
            match ns.standardWindowButton(kind) {
                Some(b) => asal_x.push(b.frame().origin.x),
                None => return false,
            }
        }
        let kiri_awal = asal_x.iter().copied().fold(f64::MAX, f64::min);
        let geser_x = inset.x as f64 - kiri_awal;

        let mut berubah = false;
        for kind in TOMBOL {
            let Some(button) = ns.standardWindowButton(kind) else {
                continue;
            };
            let frame = button.frame();
            // Back to AppKit's bottom-up y inside the container.
            let y = tinggi - inset.y as f64 - frame.size.height;
            button.setFrameOrigin(NSPoint::new(frame.origin.x + geser_x, y));
            berubah = true;
        }
        berubah
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaya_bawaan_adalah_titlebar_os() {
        assert_eq!(TitlebarStyle::default(), TitlebarStyle::Native);
        assert!(!TitlebarStyle::Native.is_custom());
        assert!(TitlebarStyle::Native.has_window_buttons());
    }

    #[test]
    fn transparent_tetap_menyisakan_traffic_lights() {
        // The whole reason `Transparent` and `Hidden` are different variants:
        // one keeps native, keyboard-reachable window buttons, the other does
        // not.
        assert!(TitlebarStyle::Transparent.is_custom());
        assert!(TitlebarStyle::Transparent.has_window_buttons());
        assert!(TitlebarStyle::Hidden.is_custom());
        assert!(!TitlebarStyle::Hidden.has_window_buttons());
    }

    #[test]
    fn material_bawaan_tidak_transparan() {
        assert_eq!(Material::default(), Material::None);
        assert_eq!(MaterialState::default(), MaterialState::FollowsWindow);
    }

    #[test]
    fn galat_vibrancy_diterjemahkan_dengan_pesannya() {
        let e = dari_vibrancy(window_vibrancy::Error::UnsupportedPlatform("hanya macOS"));
        assert_eq!(e, VibrancyError::Unsupported("hanya macOS".into()));
        assert!(e.to_string().contains("hanya macOS"));
        assert_eq!(
            dari_vibrancy(window_vibrancy::Error::NotMainThread("x")),
            VibrancyError::NotMainThread("x".into())
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn setiap_material_punya_padanan_appkit_kecuali_none() {
        for m in [
            Material::Window,
            Material::Sidebar,
            Material::HeaderView,
            Material::Popover,
            Material::Menu,
            Material::Hud,
            Material::Tooltip,
            Material::Sheet,
            Material::UnderWindow,
        ] {
            assert!(material_macos(m).is_some(), "{m:?} tidak punya padanan");
        }
        assert!(material_macos(Material::None).is_none());
    }

    #[test]
    fn inset_bawaan_sama_dengan_window_dokumen() {
        assert_eq!(DEFAULT_TRAFFIC_LIGHT_INSET, Point::new(20.0, 20.0));
    }

    #[test]
    fn atribut_window_bisa_diberi_gaya_titlebar() {
        // The description path has no OS in it, so it is safe to exercise here;
        // it is also where a wrong style would silently do nothing.
        let attrs = winit::window::Window::default_attributes();
        let _ = apply_titlebar_style(attrs.clone(), TitlebarStyle::Native);
        let hidden = apply_titlebar_style(attrs, TitlebarStyle::Hidden);
        #[cfg(not(target_os = "macos"))]
        assert!(!hidden.decorations);
        #[cfg(target_os = "macos")]
        let _ = hidden;
    }
}
