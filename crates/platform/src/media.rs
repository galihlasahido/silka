//! Media keys and Now Playing (INTEGRASI-NATIVE §3).
//!
//! Two halves of one feature. The **keys** are the play/pause, next and
//! previous buttons on a keyboard, a headset, or a Bluetooth car stereo — they
//! reach the OS, not the window, and an application that wants them has to ask
//! for them. **Now Playing** is the other direction: what the macOS Control
//! Centre, the Windows volume flyout and the GNOME lock screen show while
//! something is playing.
//!
//! | Platform | The API underneath |
//! |---|---|
//! | macOS | `MPNowPlayingInfoCenter` + `MPRemoteCommandCenter` (MediaPlayer.framework) |
//! | Windows | `SystemMediaTransportControls` (WinRT) |
//! | Linux | MPRIS 2 over D-Bus (`org.mpris.MediaPlayer2.Player`) |
//!
//! ## What is here
//!
//! The vocabulary and the arithmetic — [`NowPlaying`], [`MediaKey`],
//! [`MediaCapabilities`], and the pure functions that turn a position into the
//! string every one of those OS surfaces wants. What is **not** here is the
//! backend: `souvlaki` is the crate that covers all three, and it is not pinned
//! by this workspace, while MediaPlayer.framework and WinRT are outside the
//! binding set that is. [`MediaControls::install`] therefore reports
//! [`MediaError::Unsupported`] with that reason rather than doing nothing
//! quietly.
//!
//! ```
//! use std::time::Duration;
//! use silka_platform::media::{now_playing, MediaCapabilities, PlaybackState};
//!
//! let track = now_playing("Rhythm Is a Dancer")
//!     .artist("Snap!")
//!     .duration(Duration::from_secs(330))
//!     .position(Duration::from_secs(187))
//!     .state(PlaybackState::Playing);
//!
//! assert_eq!(track.position_text(), "3:07");
//! assert_eq!(track.duration_text().as_deref(), Some("5:30"));
//! // A player that cannot skip must not show a skip button.
//! assert!(MediaCapabilities::PLAY_PAUSE.contains(MediaCapabilities::PLAY_PAUSE));
//! ```

use core::fmt;
use std::time::Duration;

/// What the player is doing.
///
/// ```
/// use silka_platform::media::PlaybackState;
///
/// // Stopped is the default: an application that claims to be playing before
/// // it has anything loaded takes over the user's media keys for nothing.
/// assert_eq!(PlaybackState::default(), PlaybackState::Stopped);
/// assert!(PlaybackState::Playing.is_active());
/// assert!(PlaybackState::Paused.is_active());
/// assert!(!PlaybackState::Stopped.is_active());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackState {
    /// Nothing is loaded.
    #[default]
    Stopped,
    /// Playing.
    Playing,
    /// Loaded and paused — still the owner of the media keys, which is what
    /// makes the play button resume rather than starting somebody else's music.
    Paused,
}

impl PlaybackState {
    /// Whether something is loaded, playing or not.
    pub const fn is_active(self) -> bool {
        !matches!(self, PlaybackState::Stopped)
    }
}

/// A media key, as it arrives from the OS.
///
/// ```
/// use silka_platform::media::{MediaKey, PlaybackState};
///
/// // The one key that means different things depending on state, resolved
/// // once here rather than in every application.
/// assert_eq!(MediaKey::PlayPause.resolve(PlaybackState::Playing), MediaKey::Pause);
/// assert_eq!(MediaKey::PlayPause.resolve(PlaybackState::Paused), MediaKey::Play);
/// assert_eq!(MediaKey::Next.resolve(PlaybackState::Playing), MediaKey::Next);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MediaKey {
    /// Start playing.
    Play,
    /// Pause.
    Pause,
    /// The single key most keyboards actually have.
    PlayPause,
    /// Stop and unload.
    Stop,
    /// Next track.
    Next,
    /// Previous track.
    Previous,
    /// Jump forward.
    SeekForward,
    /// Jump backward.
    SeekBackward,
}

impl MediaKey {
    /// [`MediaKey::PlayPause`] resolved against the current state.
    ///
    /// A toggle key is the only one whose meaning depends on what the player is
    /// doing, and resolving it once here is what keeps every application from
    /// getting it subtly wrong while a track is loading.
    pub const fn resolve(self, state: PlaybackState) -> MediaKey {
        match (self, state) {
            (MediaKey::PlayPause, PlaybackState::Playing) => MediaKey::Pause,
            (MediaKey::PlayPause, _) => MediaKey::Play,
            (other, _) => other,
        }
    }
}

/// Which buttons the OS should offer.
///
/// A player with no playlist must not show a "next" button: the OS draws
/// exactly what it is told it can do, and a dead button is worse than a missing
/// one.
///
/// ```
/// use silka_platform::media::MediaCapabilities;
///
/// let podcast = MediaCapabilities::PLAY_PAUSE.union(MediaCapabilities::SEEK);
/// assert!(podcast.contains(MediaCapabilities::SEEK));
/// assert!(!podcast.contains(MediaCapabilities::NEXT));
/// assert!(MediaCapabilities::NONE.is_empty());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MediaCapabilities(u8);

impl MediaCapabilities {
    /// Nothing.
    pub const NONE: Self = Self(0);
    /// Play and pause.
    pub const PLAY_PAUSE: Self = Self(1 << 0);
    /// Stop.
    pub const STOP: Self = Self(1 << 1);
    /// Next track.
    pub const NEXT: Self = Self(1 << 2);
    /// Previous track.
    pub const PREVIOUS: Self = Self(1 << 3);
    /// Seeking within the current item.
    pub const SEEK: Self = Self(1 << 4);
    /// Everything.
    pub const ALL: Self = Self(0b1_1111);

    /// The raw bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// True when nothing is offered.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True when every bit of `other` is present.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The union of two sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether a key press should be delivered at all.
    ///
    /// The OS sometimes sends a key for a button it was never told to draw —
    /// a Bluetooth remote has all six buttons whatever the application said.
    /// Filtering here means an application never has to guard every handler.
    ///
    /// ```
    /// use silka_platform::media::{MediaCapabilities, MediaKey};
    ///
    /// let podcast = MediaCapabilities::PLAY_PAUSE;
    /// assert!(podcast.allows(MediaKey::PlayPause));
    /// // A car stereo's "next" button on a player with no playlist.
    /// assert!(!podcast.allows(MediaKey::Next));
    /// ```
    pub const fn allows(self, key: MediaKey) -> bool {
        let needed = match key {
            MediaKey::Play | MediaKey::Pause | MediaKey::PlayPause => Self::PLAY_PAUSE,
            MediaKey::Stop => Self::STOP,
            MediaKey::Next => Self::NEXT,
            MediaKey::Previous => Self::PREVIOUS,
            MediaKey::SeekForward | MediaKey::SeekBackward => Self::SEEK,
        };
        self.contains(needed)
    }
}

impl core::ops::BitOr for MediaCapabilities {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

/// A duration as `m:ss`, or `h:mm:ss` once it passes an hour.
///
/// The format every one of the three OS surfaces uses, and the two rules that
/// are easy to get wrong: seconds are always two digits (`3:7` is not a time),
/// and the hour field only appears when there is one (`0:03:07` reads as a
/// bug).
///
/// ```
/// use std::time::Duration;
/// use silka_platform::media::format_time;
///
/// assert_eq!(format_time(Duration::from_secs(187)), "3:07");
/// assert_eq!(format_time(Duration::from_secs(7)), "0:07");
/// assert_eq!(format_time(Duration::from_secs(3_600)), "1:00:00");
/// assert_eq!(format_time(Duration::from_secs(3_723)), "1:02:03");
/// assert_eq!(format_time(Duration::ZERO), "0:00");
/// ```
pub fn format_time(duration: Duration) -> String {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// A position pulled back inside the track it belongs to.
///
/// A position past the end makes a progress bar draw outside itself, and every
/// OS surface believes whatever it is told.
///
/// ```
/// use std::time::Duration;
/// use silka_platform::media::clamp_position;
///
/// let track = Some(Duration::from_secs(330));
/// assert_eq!(clamp_position(Duration::from_secs(400), track), Duration::from_secs(330));
/// assert_eq!(clamp_position(Duration::from_secs(10), track), Duration::from_secs(10));
/// // A live stream has no length, so nothing to clamp against.
/// assert_eq!(clamp_position(Duration::from_secs(9_999), None), Duration::from_secs(9_999));
/// ```
pub fn clamp_position(position: Duration, duration: Option<Duration>) -> Duration {
    match duration {
        Some(total) if position > total => total,
        _ => position,
    }
}

/// What the OS should show while something is playing.
///
/// A plain value: it can be built and asserted with no OS involved.
///
/// ```
/// use std::time::Duration;
/// use silka_platform::media::{now_playing, PlaybackState};
///
/// let track = now_playing("Rhythm Is a Dancer")
///     .artist("Snap!")
///     .album("The Madman's Return")
///     .duration(Duration::from_secs(330))
///     .position(Duration::from_secs(187))
///     .state(PlaybackState::Playing);
///
/// assert_eq!(track.title(), "Rhythm Is a Dancer");
/// assert_eq!(track.position_text(), "3:07");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NowPlaying {
    title: String,
    artist: Option<String>,
    album: Option<String>,
    artwork_url: Option<String>,
    duration: Option<Duration>,
    position: Duration,
    state: PlaybackState,
}

/// Describe what is playing.
pub fn now_playing(title: impl Into<String>) -> NowPlaying {
    NowPlaying {
        title: title.into(),
        ..NowPlaying::default()
    }
}

impl NowPlaying {
    /// The performer.
    pub fn artist(mut self, artist: impl Into<String>) -> Self {
        self.artist = Some(artist.into());
        self
    }

    /// The album or show.
    pub fn album(mut self, album: impl Into<String>) -> Self {
        self.album = Some(album.into());
        self
    }

    /// Cover art, as a URL the OS can fetch.
    pub fn artwork(mut self, url: impl Into<String>) -> Self {
        self.artwork_url = Some(url.into());
        self
    }

    /// How long the item is. `None` for a live stream.
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    /// How far in the player is; clamped to the duration.
    pub fn position(mut self, position: Duration) -> Self {
        self.position = clamp_position(position, self.duration);
        self
    }

    /// What the player is doing.
    pub fn state(mut self, state: PlaybackState) -> Self {
        self.state = state;
        self
    }

    /// The title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The performer, when known.
    pub fn artist_name(&self) -> Option<&str> {
        self.artist.as_deref()
    }

    /// The album, when known.
    pub fn album_name(&self) -> Option<&str> {
        self.album.as_deref()
    }

    /// The artwork URL, when known.
    pub fn artwork_url(&self) -> Option<&str> {
        self.artwork_url.as_deref()
    }

    /// The length, when known.
    pub fn total(&self) -> Option<Duration> {
        self.duration
    }

    /// How far in the player is.
    pub fn elapsed(&self) -> Duration {
        clamp_position(self.position, self.duration)
    }

    /// What the player is doing.
    pub fn playback_state(&self) -> PlaybackState {
        self.state
    }

    /// The position, formatted.
    pub fn position_text(&self) -> String {
        format_time(self.elapsed())
    }

    /// The length, formatted; `None` for a live stream.
    pub fn duration_text(&self) -> Option<String> {
        self.duration.map(format_time)
    }

    /// How far through the item the player is, 0…1.
    ///
    /// `None` for a live stream **and** for a zero-length item — dividing by a
    /// duration nobody set is how a progress bar ends up at NaN.
    ///
    /// ```
    /// use std::time::Duration;
    /// use silka_platform::media::now_playing;
    ///
    /// let half = now_playing("x")
    ///     .duration(Duration::from_secs(100))
    ///     .position(Duration::from_secs(50));
    /// assert_eq!(half.fraction(), Some(0.5));
    ///
    /// assert_eq!(now_playing("live").fraction(), None);
    /// assert_eq!(now_playing("x").duration(Duration::ZERO).fraction(), None);
    /// ```
    pub fn fraction(&self) -> Option<f32> {
        let total = self.duration?.as_secs_f32();
        if total <= 0.0 {
            return None;
        }
        Some((self.elapsed().as_secs_f32() / total).clamp(0.0, 1.0))
    }
}

/// Why the media integration is not live.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MediaError {
    /// Nothing has a title, so there is nothing the OS could show.
    NoTitle,
    /// No backend on this build. The message says what each platform needs.
    Unsupported(String),
    /// The OS refused.
    Os(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::NoTitle => write!(f, "nothing playing has a title"),
            MediaError::Unsupported(m) => write!(f, "no media integration: {m}"),
            MediaError::Os(m) => write!(f, "the OS refused the media controls: {m}"),
        }
    }
}

impl std::error::Error for MediaError {}

/// The application's end of the media integration.
///
/// A plain value: the identity and the capability set can be assembled and
/// asserted with no OS involved.
///
/// ```
/// use silka_platform::media::{media_controls, MediaCapabilities, MediaError};
///
/// let controls = media_controls("com.example.player")
///     .display_name("Player")
///     .capabilities(MediaCapabilities::PLAY_PAUSE.union(MediaCapabilities::SEEK));
///
/// assert_eq!(controls.identity(), "com.example.player");
/// assert!(controls.capabilities_set().contains(MediaCapabilities::SEEK));
///
/// // Honest about not being wired up yet.
/// assert!(matches!(controls.install(), Err(MediaError::Unsupported(_))));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaControls {
    identity: String,
    display_name: Option<String>,
    capabilities: MediaCapabilities,
}

/// Describe the media integration.
///
/// `identity` is the reverse-DNS name the OS files the player under — the MPRIS
/// bus name on Linux, the bundle identifier on macOS.
pub fn media_controls(identity: impl Into<String>) -> MediaControls {
    MediaControls {
        identity: identity.into(),
        display_name: None,
        // Play/pause is the one button every player has; the rest are opt-in,
        // because a dead button is worse than a missing one.
        capabilities: MediaCapabilities::PLAY_PAUSE,
    }
}

impl MediaControls {
    /// The name a user reads.
    pub fn display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    /// Which buttons the OS should offer.
    pub fn capabilities(mut self, capabilities: MediaCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// The reverse-DNS identity.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// The display name — the identity when none was set.
    pub fn name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.identity)
    }

    /// The capability set.
    pub fn capabilities_set(&self) -> MediaCapabilities {
        self.capabilities
    }

    /// Claim the media keys and start publishing Now Playing.
    ///
    /// # Errors
    ///
    /// Always [`MediaError::Unsupported`] today — see the module documentation.
    pub fn install(&self) -> Result<(), MediaError> {
        Err(MediaError::Unsupported(
            "souvlaki covers all three platforms and is not pinned by this workspace; \
             MediaPlayer.framework and the WinRT SystemMediaTransportControls are outside the \
             binding set that is"
                .into(),
        ))
    }

    /// Publish what is playing.
    ///
    /// # Errors
    ///
    /// [`MediaError::NoTitle`] for an item with nothing to show, and otherwise
    /// [`MediaError::Unsupported`] — see [`MediaControls::install`].
    pub fn publish(&self, track: &NowPlaying) -> Result<(), MediaError> {
        if track.title().trim().is_empty() {
            return Err(MediaError::NoTitle);
        }
        self.install()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detik_selalu_dua_angka() {
        // `3:7` is not a time.
        assert_eq!(format_time(Duration::from_secs(187)), "3:07");
        assert_eq!(format_time(Duration::from_secs(7)), "0:07");
        assert_eq!(format_time(Duration::ZERO), "0:00");
    }

    #[test]
    fn jam_hanya_muncul_kalau_memang_ada() {
        // `0:03:07` reads as a bug.
        assert_eq!(format_time(Duration::from_secs(3_599)), "59:59");
        assert_eq!(format_time(Duration::from_secs(3_600)), "1:00:00");
        assert_eq!(format_time(Duration::from_secs(3_723)), "1:02:03");
    }

    #[test]
    fn posisi_tidak_pernah_melewati_akhir() {
        let track = now_playing("x")
            .duration(Duration::from_secs(100))
            .position(Duration::from_secs(400));
        assert_eq!(track.elapsed(), Duration::from_secs(100));
        assert_eq!(track.fraction(), Some(1.0));
    }

    #[test]
    fn siaran_langsung_tidak_punya_pecahan() {
        // Dividing by a duration nobody set is how a progress bar reaches NaN.
        assert_eq!(now_playing("live").fraction(), None);
        assert_eq!(now_playing("live").duration_text(), None);
        assert_eq!(now_playing("x").duration(Duration::ZERO).fraction(), None);
    }

    #[test]
    fn urutan_pemanggilan_posisi_dan_durasi_tidak_mengubah_hasil() {
        // `position` clamps against whatever duration is known at the time, so
        // `elapsed` clamps again — otherwise setting the duration last would
        // leave a position past the end.
        let a = now_playing("x")
            .position(Duration::from_secs(400))
            .duration(Duration::from_secs(100));
        assert_eq!(a.elapsed(), Duration::from_secs(100));
    }

    #[test]
    fn tombol_toggle_diselesaikan_sekali_di_sini() {
        assert_eq!(
            MediaKey::PlayPause.resolve(PlaybackState::Playing),
            MediaKey::Pause
        );
        assert_eq!(
            MediaKey::PlayPause.resolve(PlaybackState::Paused),
            MediaKey::Play
        );
        assert_eq!(
            MediaKey::PlayPause.resolve(PlaybackState::Stopped),
            MediaKey::Play
        );
        // Everything else passes through untouched.
        assert_eq!(
            MediaKey::Next.resolve(PlaybackState::Paused),
            MediaKey::Next
        );
    }

    #[test]
    fn tombol_yang_tidak_ditawarkan_tidak_diteruskan() {
        // A Bluetooth remote has all six buttons whatever the application said.
        let podcast = MediaCapabilities::PLAY_PAUSE.union(MediaCapabilities::SEEK);
        assert!(podcast.allows(MediaKey::PlayPause));
        assert!(podcast.allows(MediaKey::SeekForward));
        assert!(!podcast.allows(MediaKey::Next));
        assert!(!podcast.allows(MediaKey::Stop));
        assert!(MediaCapabilities::ALL.allows(MediaKey::Previous));
        assert!(!MediaCapabilities::NONE.allows(MediaKey::Play));
    }

    #[test]
    fn kemampuan_bawaan_hanya_play_pause() {
        // A dead button is worse than a missing one.
        assert_eq!(
            media_controls("com.example.player").capabilities_set(),
            MediaCapabilities::PLAY_PAUSE
        );
    }

    #[test]
    fn nama_tampilan_jatuh_kembali_ke_identitas() {
        let c = media_controls("com.example.player");
        assert_eq!(c.name(), "com.example.player");
        assert_eq!(c.display_name("Player").name(), "Player");
    }

    #[test]
    fn paused_tetap_pemilik_tombol_media() {
        // What makes the play button resume rather than starting somebody
        // else's music.
        assert!(PlaybackState::Paused.is_active());
        assert!(!PlaybackState::Stopped.is_active());
        assert_eq!(PlaybackState::default(), PlaybackState::Stopped);
    }

    #[test]
    fn tanpa_judul_ditolak_sebelum_backend_disalahkan() {
        let controls = media_controls("com.example.player");
        assert_eq!(
            controls.publish(&now_playing("  ")),
            Err(MediaError::NoTitle)
        );
        assert!(matches!(
            controls.publish(&now_playing("x")),
            Err(MediaError::Unsupported(_))
        ));
    }
}
