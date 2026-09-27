//! The `souvlaki` boundary (INTEGRASI-NATIVE §3).
//!
//! Every `souvlaki::` type in the framework lives and dies inside this file.
//! Above it, [`crate::media`] speaks only its own vocabulary — the same rule
//! `notification` keeps for `notify-rust` and `credential` for `keyring`.
//!
//! What the crate gives each platform:
//!
//! | Platform | API underneath | What this module must hand it |
//! |---|---|---|
//! | macOS | `MPNowPlayingInfoCenter` + `MPRemoteCommandCenter` | nothing extra — but the process must already be running an `NSApplication` event loop |
//! | Windows | `SystemMediaTransportControls` (WinRT) | the window's `HWND`, which the controls bind to |
//! | Linux | MPRIS 2 over D-Bus, via souvlaki's `use_zbus` | a D-Bus-legal `dbus_name`; a running session bus |
//!
//! One backend covers all three, so [`MediaError::Unsupported`] has shrunk to
//! its honest meaning here: "called before there was an event loop to deliver
//! key presses into", not "no backend on this build".

use std::time::Duration;

use souvlaki::{
    MediaControlEvent, MediaControls as BackendControls, MediaMetadata, MediaPlayback,
    MediaPosition, PlatformConfig, SeekDirection,
};

use super::{MediaControls, MediaError, MediaKey, NowPlaying, PlaybackState};

// ---------------------------------------------------------------------------
// Event translation
// ---------------------------------------------------------------------------

/// A media key from the OS, or `None` for the events the framework has no
/// vocabulary for (`SetVolume`, `OpenUri`, `Raise`, `Quit`, `SetPosition`,
/// `SeekBy`).
///
/// The mapping is deliberately total over the two enums: every event that has
/// a [`MediaKey`] maps to exactly the one a user would name, and everything
/// else reports absent rather than being squeezed into the nearest key.
/// `SeekBy` (skip ±N seconds) is the tempting one to fold into
/// [`MediaKey::SeekForward`] / [`MediaKey::SeekBackward`] — and it is *not*
/// folded, because an application that answers a seek key by jumping to a
/// preset would then skip an arbitrary amount when the OS sends `SeekBy`.
pub(super) fn media_key_of(event: &MediaControlEvent) -> Option<MediaKey> {
    let key = match event {
        MediaControlEvent::Play => MediaKey::Play,
        MediaControlEvent::Pause => MediaKey::Pause,
        MediaControlEvent::Toggle => MediaKey::PlayPause,
        MediaControlEvent::Stop => MediaKey::Stop,
        MediaControlEvent::Next => MediaKey::Next,
        MediaControlEvent::Previous => MediaKey::Previous,
        MediaControlEvent::Seek(direction) => seek_key(*direction),
        _ => return None,
    };
    Some(key)
}

/// The one seek direction souvlaki names, as our key.
fn seek_key(direction: SeekDirection) -> MediaKey {
    match direction {
        SeekDirection::Forward => MediaKey::SeekForward,
        SeekDirection::Backward => MediaKey::SeekBackward,
    }
}

// ---------------------------------------------------------------------------
// Publishing
// ---------------------------------------------------------------------------

/// What the OS should display, from one of ours.
///
/// Split from the publish call below so the no-title rule is testable without
/// a backend: an unnamed entry in the Control Centre is worse than none, so an
/// empty title is refused before the OS is asked.
fn metadata_of(track: &NowPlaying) -> Result<MediaMetadata<'_>, MediaError> {
    if track.title().trim().is_empty() {
        return Err(MediaError::NoTitle);
    }
    Ok(MediaMetadata {
        title: Some(track.title()),
        artist: track.artist_name(),
        album: track.album_name(),
        cover_url: track.artwork_url(),
        duration: track.total(),
    })
}

/// The playback status, from one of ours.
fn playback_of(track: &NowPlaying) -> MediaPlayback {
    let progress = MediaPosition(Duration::from_secs(track.elapsed().as_secs()));
    match track.playback_state() {
        PlaybackState::Playing => MediaPlayback::Playing {
            progress: Some(progress),
        },
        PlaybackState::Paused => MediaPlayback::Paused {
            progress: Some(progress),
        },
        PlaybackState::Stopped => MediaPlayback::Stopped,
    }
}

// ---------------------------------------------------------------------------
// The live session
// ---------------------------------------------------------------------------

/// A live media session — what [`MediaControls::install`] returns.
///
/// Owns the OS registration. Dropping it detaches the remote-command handlers
/// and removes the entry from the OS's Now Playing surfaces.
pub(super) struct Session {
    controls: BackendControls,
}

/// Claim the media keys and start publishing.
///
/// Must run **after** the event loop exists: the key presses arrive on an OS
/// thread of the OS's choosing and are forwarded through the
/// [`EventLoopProxy`](winit::event_loop::EventLoopProxy) that
/// [`crate::forward_native_events`] registered — the same path menu clicks and
/// global hotkeys take. Without one there is nowhere to deliver the keys, so
/// this refuses rather than installing controls whose buttons would do
/// nothing.
pub(super) fn install(
    description: &MediaControls,
    hwnd: Option<*mut core::ffi::c_void>,
) -> Result<Session, MediaError> {
    let proxy = crate::event::shell_proxy().ok_or(MediaError::NoEventLoop)?;

    // The dbus_name must be D-Bus-legal on Linux (alphanumeric + `_`), and it
    // doubles as the identity everywhere. An application that passed an empty
    // identity gets a name, not a half-alive registration.
    let dbus_name = description.identity.trim();
    let config = PlatformConfig {
        dbus_name: if dbus_name.is_empty() {
            "silka_app"
        } else {
            dbus_name
        },
        display_name: description.name(),
        hwnd,
    };

    let mut controls = BackendControls::new(config).map_err(|e| MediaError::Os(e.to_string()))?;

    // The handler fires wherever the OS feels like firing it. It must not
    // touch application state; it translates, filters against the capability
    // set (a Bluetooth remote has every button whatever was advertised), and
    // forwards. Sending into a gone loop is the normal shutdown race — the
    // same silence the menu and hotkey callbacks keep.
    let capabilities = description.capabilities_set();
    controls
        .attach(move |event| {
            let Some(key) = media_key_of(&event) else {
                return;
            };
            if capabilities.allows(key) {
                let _ = proxy.send_event(crate::ShellEvent::Media(key));
            }
        })
        .map_err(|e| MediaError::Os(e.to_string()))?;

    Ok(Session { controls })
}

impl Session {
    /// Publish what is playing.
    ///
    /// `&mut self` because every souvlaki platform backend mutates its OS
    /// object; a session is owned by the application, so exclusive access is
    /// the natural shape.
    pub(super) fn publish(&mut self, track: &NowPlaying) -> Result<(), MediaError> {
        let metadata = metadata_of(track)?;
        let playback = playback_of(track);
        self.controls
            .set_metadata(metadata)
            .map_err(|e| MediaError::Os(e.to_string()))?;
        self.controls
            .set_playback(playback)
            .map_err(|e| MediaError::Os(e.to_string()))?;
        Ok(())
    }

    /// Withdraw from Now Playing: playback stopped, so the OS surface has
    /// nothing left to show. The keys stay claimed until the session drops.
    pub(super) fn stop(&mut self) -> Result<(), MediaError> {
        self.controls
            .set_playback(MediaPlayback::Stopped)
            .map_err(|e| MediaError::Os(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn setiap_tombol_dipetakan_ke_kunci_yang_dinamai_pengguna() {
        assert_eq!(media_key_of(&MediaControlEvent::Play), Some(MediaKey::Play));
        assert_eq!(
            media_key_of(&MediaControlEvent::Pause),
            Some(MediaKey::Pause)
        );
        assert_eq!(
            media_key_of(&MediaControlEvent::Toggle),
            Some(MediaKey::PlayPause)
        );
        assert_eq!(media_key_of(&MediaControlEvent::Stop), Some(MediaKey::Stop));
        assert_eq!(media_key_of(&MediaControlEvent::Next), Some(MediaKey::Next));
        assert_eq!(
            media_key_of(&MediaControlEvent::Previous),
            Some(MediaKey::Previous)
        );
        assert_eq!(
            media_key_of(&MediaControlEvent::Seek(SeekDirection::Forward)),
            Some(MediaKey::SeekForward)
        );
        assert_eq!(
            media_key_of(&MediaControlEvent::Seek(SeekDirection::Backward)),
            Some(MediaKey::SeekBackward)
        );
    }

    #[test]
    fn peristiwa_tanpa_kosakata_dilaporkan_kosong() {
        // Squeezing `SetVolume` into "next" would fire the wrong handler in
        // every application; `None` is the honest answer.
        assert_eq!(
            media_key_of(&MediaControlEvent::SeekBy(
                SeekDirection::Forward,
                Duration::from_secs(10)
            )),
            None
        );
        assert_eq!(media_key_of(&MediaControlEvent::SetVolume(0.5)), None);
        assert_eq!(
            media_key_of(&MediaControlEvent::SetPosition(MediaPosition(
                Duration::ZERO
            ))),
            None
        );
        assert_eq!(media_key_of(&MediaControlEvent::OpenUri("x".into())), None);
        assert_eq!(media_key_of(&MediaControlEvent::Raise), None);
        assert_eq!(media_key_of(&MediaControlEvent::Quit), None);
    }

    #[test]
    fn judul_kosong_ditolak_sebelum_os_ditanya() {
        assert_eq!(
            metadata_of(&super::super::now_playing("  ")),
            Err(MediaError::NoTitle)
        );
        assert_eq!(
            metadata_of(&super::super::now_playing("")),
            Err(MediaError::NoTitle)
        );
    }

    #[test]
    fn metadata_membawa_satu_lapangan_per_penjawab_kosakata() {
        let track = super::super::now_playing("When The Sun Hits")
            .artist("Slowdive")
            .album("Souvlaki")
            .artwork("https://example.com/cover.jpg")
            .duration(Duration::from_secs(290))
            .position(Duration::from_secs(30))
            .state(PlaybackState::Playing);
        let m = metadata_of(&track).expect("metadata");
        assert_eq!(m.title, Some("When The Sun Hits"));
        assert_eq!(m.artist, Some("Slowdive"));
        assert_eq!(m.album, Some("Souvlaki"));
        assert_eq!(m.cover_url, Some("https://example.com/cover.jpg"));
        assert_eq!(m.duration, Some(Duration::from_secs(290)));
    }

    #[test]
    fn posisi_ikut_ditayangkan_hentikan_tidak() {
        let playing = super::super::now_playing("x")
            .state(PlaybackState::Playing)
            .position(Duration::from_secs(5));
        let paused = super::super::now_playing("x")
            .state(PlaybackState::Paused)
            .position(Duration::from_secs(6));
        let stopped = super::super::now_playing("x").state(PlaybackState::Stopped);
        assert!(matches!(
            playback_of(&playing),
            MediaPlayback::Playing { progress: Some(_) }
        ));
        assert!(matches!(
            playback_of(&paused),
            MediaPlayback::Paused { progress: Some(_) }
        ));
        assert_eq!(playback_of(&stopped), MediaPlayback::Stopped);
    }

    #[test]
    fn arah_mencari_dipetakan_dua_arah() {
        assert_eq!(seek_key(SeekDirection::Forward), MediaKey::SeekForward);
        assert_eq!(seek_key(SeekDirection::Backward), MediaKey::SeekBackward);
    }
}
