//! Tray / status-bar icon (INTEGRASI-NATIVE §2).
//!
//! A tray icon is what makes "minimize to tray" and background applications
//! possible at all, and it is one of the few native features with genuinely
//! different rules per OS. The two that bite hardest are encoded in this API
//! rather than left in a README:
//!
//! - **macOS wants a template image.** A status-bar icon must be a monochrome
//!   silhouette flagged as a template, or the OS cannot recolour it and it
//!   turns invisible the moment the menubar switches to dark. [`TrayConfig`]
//!   therefore defaults [`TrayConfig::template`] to `true` — the correct macOS
//!   behaviour — and it is a no-op elsewhere.
//! - **Linux has no click events.** libappindicator only ever shows the menu;
//!   there is no click callback to hang behaviour off. So a tray icon without a
//!   menu is refused up front ([`TrayError::NoMenu`]) instead of shipping an
//!   icon that does nothing on one of the three targets.
//!
//! ```no_run
//! use silka_platform::menu::{item, menu};
//! use silka_platform::tray::tray;
//! use silka_platform::image::RgbaImage;
//!
//! let ikon = RgbaImage::solid(16, 16, [255, 255, 255, 255])?;
//! let _tray = tray("utama")
//!     .tooltip("Silka")
//!     .icon(ikon)
//!     .menu(menu("Silka").item(item("tray.quit", "Keluar")))
//!     .install()?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use core::fmt;

use crate::image::RgbaImage;
use crate::menu::{Menu, MenuError};

/// Which mouse button produced a tray event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayButton {
    /// Left / primary.
    Primary,
    /// Right / secondary — the one that conventionally opens the menu.
    Secondary,
    /// Middle.
    Middle,
}

/// Something the user did to the tray icon.
///
/// Positions are in **physical** pixels because a tray icon does not belong to
/// any window and therefore has no scale factor of its own to divide by; the
/// screen it sits on is the only frame of reference there is.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum TrayActivation {
    /// The icon was clicked.
    Click {
        /// Which tray icon.
        id: String,
        /// Which button.
        button: TrayButton,
        /// Where the click happened, in physical screen pixels.
        position: (f64, f64),
    },
    /// The pointer entered the icon.
    Enter {
        /// Which tray icon.
        id: String,
    },
    /// The pointer left the icon.
    Leave {
        /// Which tray icon.
        id: String,
    },
}

impl TrayActivation {
    /// Which tray icon this refers to.
    pub fn id(&self) -> &str {
        match self {
            TrayActivation::Click { id, .. }
            | TrayActivation::Enter { id }
            | TrayActivation::Leave { id } => id,
        }
    }
}

/// Translate a tray-icon event into ours.
///
/// `Move` is dropped on the floor on purpose: it fires continuously while the
/// pointer crosses the icon and there is nothing an application can usefully do
/// with it that `Enter`/`Leave` does not already cover. Waking the UI thread
/// for it would break "render only when dirty" (§3.5) for a hover.
pub(crate) fn activation_from_tray_icon(event: tray_icon::TrayIconEvent) -> Option<TrayActivation> {
    use tray_icon::TrayIconEvent as E;
    match event {
        E::Click {
            id,
            button,
            button_state,
            position,
            ..
        } => {
            // Only the release is an activation; reporting both edges would
            // make every click count twice.
            if button_state != tray_icon::MouseButtonState::Up {
                return None;
            }
            Some(TrayActivation::Click {
                id: id.0,
                button: match button {
                    tray_icon::MouseButton::Left => TrayButton::Primary,
                    tray_icon::MouseButton::Right => TrayButton::Secondary,
                    tray_icon::MouseButton::Middle => TrayButton::Middle,
                },
                position: (position.x, position.y),
            })
        }
        // Windows-only, and it arrives *in addition to* the two clicks that
        // made it — forwarding it would report the same gesture three times.
        E::DoubleClick { .. } => None,
        E::Enter { id, .. } => Some(TrayActivation::Enter { id: id.0 }),
        E::Leave { id, .. } => Some(TrayActivation::Leave { id: id.0 }),
        E::Move { .. } => None,
        _ => None,
    }
}

/// Why a tray icon could not be created.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrayError {
    /// No menu was given. A menuless tray icon is inert on Linux.
    NoMenu,
    /// No icon was given. Every platform needs an image to draw.
    NoIcon,
    /// The menu description is wrong (see [`MenuError`]).
    Menu(MenuError),
    /// The OS refused the icon.
    Os(String),
}

impl fmt::Display for TrayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrayError::NoMenu => write!(f, "tray tanpa menu tidak berguna di Linux"),
            TrayError::NoIcon => write!(f, "tray tanpa ikon tidak bisa digambar"),
            TrayError::Menu(e) => write!(f, "menu tray tidak sah: {e}"),
            TrayError::Os(m) => write!(f, "tray ditolak OS: {m}"),
        }
    }
}

impl std::error::Error for TrayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TrayError::Menu(e) => Some(e),
            _ => None,
        }
    }
}

impl From<MenuError> for TrayError {
    fn from(e: MenuError) -> Self {
        TrayError::Menu(e)
    }
}

/// A tray icon description, built by method chaining.
#[derive(Debug, Clone, PartialEq)]
pub struct TrayConfig {
    id: String,
    tooltip: Option<String>,
    title: Option<String>,
    icon: Option<RgbaImage>,
    menu: Option<Menu>,
    template: bool,
    menu_on_left_click: bool,
}

/// Describe a tray icon.
///
/// `id` is what comes back in a [`TrayActivation`], so an application with
/// several icons can tell them apart.
pub fn tray(id: impl Into<String>) -> TrayConfig {
    TrayConfig {
        id: id.into(),
        tooltip: None,
        title: None,
        icon: None,
        menu: None,
        // The macOS-correct default; ignored on Windows and Linux.
        template: true,
        // macOS convention: the left button opens the menu too. Windows users
        // expect left-click to be the application's own action, but that
        // difference belongs to the application, not to this default.
        menu_on_left_click: true,
    }
}

impl TrayConfig {
    /// The hover tooltip.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Text shown beside the icon (macOS status bar, some Linux shells).
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// The icon image.
    pub fn icon(mut self, icon: RgbaImage) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The menu shown when the icon is clicked.
    pub fn menu(mut self, menu: Menu) -> Self {
        self.menu = Some(menu);
        self
    }

    /// Whether the icon is a macOS **template** image — a silhouette the OS
    /// recolours for light and dark menubars. Defaults to `true`.
    ///
    /// Turn it off only for an icon that genuinely must keep its own colours,
    /// and accept that it will not adapt.
    pub fn template(mut self, template: bool) -> Self {
        self.template = template;
        self
    }

    /// Whether a left click opens the menu as well as a right click.
    pub fn menu_on_left_click(mut self, enable: bool) -> Self {
        self.menu_on_left_click = enable;
        self
    }

    /// The id this icon reports in its activations.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Whether the description is complete enough to install.
    ///
    /// Checked separately from [`TrayConfig::install`] so an application can
    /// assert it in a test that never touches the OS.
    pub fn check(&self) -> Result<(), TrayError> {
        if self.icon.is_none() {
            return Err(TrayError::NoIcon);
        }
        let Some(menu) = &self.menu else {
            return Err(TrayError::NoMenu);
        };
        if let Some(id) = menu.duplicate_ids().into_iter().next() {
            return Err(TrayError::Menu(MenuError::DuplicateId(id)));
        }
        Ok(())
    }

    /// Create the tray icon.
    ///
    /// # Panics
    ///
    /// Main thread only, like every other OS menu API.
    pub fn install(self) -> Result<Tray, TrayError> {
        self.check()?;
        let icon = self.icon.expect("check() memastikan ada ikon");
        let menu = self.menu.expect("check() memastikan ada menu");

        let (w, h) = (icon.width(), icon.height());
        let ikon = tray_icon::Icon::from_rgba(icon.into_rgba(), w, h)
            .map_err(|e| TrayError::Os(e.to_string()))?;

        let mut builder = tray_icon::TrayIconBuilder::new()
            .with_id(self.id.clone())
            .with_icon(ikon)
            .with_icon_as_template(self.template)
            .with_menu_on_left_click(self.menu_on_left_click)
            // The tray icon owns its menu: `tray-icon` keeps the boxed menu
            // alive for as long as the icon exists, so there is nothing here
            // for the caller to keep a handle on.
            .with_menu(Box::new(menu.popup()?.into_root()));
        if let Some(t) = &self.tooltip {
            builder = builder.with_tooltip(t);
        }
        if let Some(t) = &self.title {
            builder = builder.with_title(t);
        }

        builder
            .build()
            .map(|inner| Tray { inner })
            .map_err(|e| TrayError::Os(e.to_string()))
    }
}

/// A live tray icon.
///
/// **Keep it alive.** Dropping it removes the icon from the tray.
pub struct Tray {
    inner: tray_icon::TrayIcon,
}

impl fmt::Debug for Tray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tray")
            .field("id", &self.inner.id().0)
            .finish()
    }
}

impl Tray {
    /// The id this icon reports in its activations.
    pub fn id(&self) -> String {
        self.inner.id().0.clone()
    }

    /// Replace the tooltip.
    pub fn set_tooltip(&self, tooltip: Option<&str>) -> Result<(), TrayError> {
        self.inner
            .set_tooltip(tooltip)
            .map_err(|e| TrayError::Os(e.to_string()))
    }

    /// Show or hide the icon.
    pub fn set_visible(&self, visible: bool) -> Result<(), TrayError> {
        self.inner
            .set_visible(visible)
            .map_err(|e| TrayError::Os(e.to_string()))
    }

    /// Replace the icon image, keeping the template flag it was created with.
    pub fn set_icon(&self, icon: &RgbaImage) -> Result<(), TrayError> {
        let ikon = tray_icon::Icon::from_rgba(icon.rgba().to_vec(), icon.width(), icon.height())
            .map_err(|e| TrayError::Os(e.to_string()))?;
        self.inner
            .set_icon(Some(ikon))
            .map_err(|e| TrayError::Os(e.to_string()))
    }
}

/// Drain one pending tray activation, if the OS has queued any.
///
/// The same either/or rule as [`crate::menu::poll_menu_activation`]:
/// `tray-icon` delivers to a callback *or* to this queue, so this returns
/// `None` forever once [`crate::forward_native_events`] has claimed the
/// callback.
pub fn poll_tray_activation() -> Option<TrayActivation> {
    loop {
        let event = tray_icon::TrayIconEvent::receiver().try_recv().ok()?;
        if let Some(a) = activation_from_tray_icon(event) {
            return Some(a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::{item, menu};

    fn ikon() -> RgbaImage {
        RgbaImage::solid(2, 2, [255, 255, 255, 255]).expect("ukuran sah")
    }

    #[test]
    fn ikon_macos_bawaan_adalah_template() {
        // A non-template status-bar icon disappears against a dark menubar.
        assert!(tray("utama").template);
    }

    #[test]
    fn tray_tanpa_ikon_ditolak() {
        let e = tray("utama").menu(menu("M")).check();
        assert_eq!(e, Err(TrayError::NoIcon));
    }

    #[test]
    fn tray_tanpa_menu_ditolak_karena_mati_di_linux() {
        let e = tray("utama").icon(ikon()).check();
        assert_eq!(e, Err(TrayError::NoMenu));
    }

    #[test]
    fn tray_lengkap_lolos_pemeriksaan() {
        let c = tray("utama")
            .icon(ikon())
            .menu(menu("Silka").item(item("tray.quit", "Keluar")))
            .tooltip("Silka");
        assert_eq!(c.check(), Ok(()));
        assert_eq!(c.id(), "utama");
        assert_eq!(c.tooltip.as_deref(), Some("Silka"));
    }

    #[test]
    fn id_menu_ganda_di_tray_juga_ditolak() {
        let c = tray("utama")
            .icon(ikon())
            .menu(menu("Silka").item(item("x", "A")).item(item("x", "B")));
        assert!(matches!(
            c.check(),
            Err(TrayError::Menu(MenuError::DuplicateId(_)))
        ));
    }

    #[test]
    fn hanya_pelepasan_tombol_yang_jadi_aktivasi() {
        // Down and Up both arrive; counting both would double every click.
        let turun = tray_icon::TrayIconEvent::Click {
            id: tray_icon::TrayIconId("utama".into()),
            position: tray_icon::dpi::PhysicalPosition::new(4.0, 8.0),
            rect: tray_icon::Rect::default(),
            button: tray_icon::MouseButton::Left,
            button_state: tray_icon::MouseButtonState::Down,
        };
        assert_eq!(activation_from_tray_icon(turun), None);

        let naik = tray_icon::TrayIconEvent::Click {
            id: tray_icon::TrayIconId("utama".into()),
            position: tray_icon::dpi::PhysicalPosition::new(4.0, 8.0),
            rect: tray_icon::Rect::default(),
            button: tray_icon::MouseButton::Right,
            button_state: tray_icon::MouseButtonState::Up,
        };
        assert_eq!(
            activation_from_tray_icon(naik),
            Some(TrayActivation::Click {
                id: "utama".into(),
                button: TrayButton::Secondary,
                position: (4.0, 8.0),
            })
        );
    }

    #[test]
    fn gerakan_pointer_tidak_membangunkan_apa_pun() {
        // Hover movement must not wake the UI thread (§3.5).
        let gerak = tray_icon::TrayIconEvent::Move {
            id: tray_icon::TrayIconId("utama".into()),
            position: tray_icon::dpi::PhysicalPosition::new(1.0, 1.0),
            rect: tray_icon::Rect::default(),
        };
        assert_eq!(activation_from_tray_icon(gerak), None);
    }

    #[test]
    fn masuk_dan_keluar_membawa_id() {
        let masuk = tray_icon::TrayIconEvent::Enter {
            id: tray_icon::TrayIconId("a".into()),
            position: tray_icon::dpi::PhysicalPosition::new(0.0, 0.0),
            rect: tray_icon::Rect::default(),
        };
        assert_eq!(
            activation_from_tray_icon(masuk)
                .as_ref()
                .map(TrayActivation::id),
            Some("a")
        );
    }
}
