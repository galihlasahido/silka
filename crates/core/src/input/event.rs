//! Kosakata event input — **milik kita sendiri**, bukan tipe winit.
//!
//! Alasannya sama dengan alasan `rustui-paint` tidak memuat tipe wgpu
//! (REKOMENDASI §3.2): kode widget berbicara dalam kosakata ini, dan
//! `rustui-platform` adalah satu-satunya tempat yang tahu winit. Backend shell
//! lain (uji headless, replay rekaman input, nanti mungkin platform baru) cukup
//! menghasilkan tipe-tipe di modul ini.
//!
//! Semua koordinat dalam **poin logis** dan **global terhadap window** — DPI
//! sudah diselesaikan di lapisan platform, dan konversi ke koordinat lokal node
//! dilakukan hit-testing ([`crate::input::hit`]).
//!
//! Semua stempel waktu adalah [`Duration`] sejak window dibuka, bukan
//! `Instant`. Dengan begitu velocity tracker bisa diuji secara deterministik
//! tanpa menyentuh jam sistem.

use core::fmt;
use std::time::Duration;

use rustui_paint::Point;

// ---------------------------------------------------------------------------
// Modifiers
// ---------------------------------------------------------------------------

/// Tombol pengubah yang sedang ditahan, sebagai bitset.
///
/// [`Modifiers::COMMAND`] adalah alias yang menunjuk tombol "aksi utama" OS:
/// ⌘ di macOS, Ctrl di Windows/Linux. Widget menulis pintasan sekali memakai
/// itu dan otomatis benar di ketiga platform.
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Modifiers(u8);

impl Modifiers {
    /// Tidak ada modifier.
    pub const NONE: Self = Self(0);
    /// Shift.
    pub const SHIFT: Self = Self(1 << 0);
    /// Control.
    pub const CONTROL: Self = Self(1 << 1);
    /// Alt / Option.
    pub const ALT: Self = Self(1 << 2);
    /// Meta: ⌘ di macOS, tombol Windows di PC.
    pub const META: Self = Self(1 << 3);

    /// Tombol "aksi utama" per platform: ⌘ di macOS, Ctrl selain itu.
    #[cfg(target_os = "macos")]
    pub const COMMAND: Self = Self::META;
    /// Tombol "aksi utama" per platform: ⌘ di macOS, Ctrl selain itu.
    #[cfg(not(target_os = "macos"))]
    pub const COMMAND: Self = Self::CONTROL;

    const NAMES: [(Self, &'static str); 4] = [
        (Self::SHIFT, "shift"),
        (Self::CONTROL, "control"),
        (Self::ALT, "alt"),
        (Self::META, "meta"),
    ];

    /// Bit mentah.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Benar bila tidak ada modifier sama sekali.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Benar bila seluruh bit `other` ada di sini.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Gabungan dua himpunan.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Tambahkan modifier.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Buang modifier.
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    /// Benar bila **persis** modifier ini yang ditahan (tidak lebih).
    ///
    /// Dipakai pintasan: `Tab` polos tidak boleh cocok dengan `Ctrl+Tab`.
    pub const fn is_exactly(self, other: Self) -> bool {
        self.0 == other.0
    }
}

impl core::ops::BitOr for Modifiers {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Self) {
        self.insert(rhs);
    }
}

impl fmt::Debug for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("Modifiers(none)");
        }
        f.write_str("Modifiers(")?;
        let mut first = true;
        for (bit, name) in Self::NAMES {
            if self.contains(bit) {
                if !first {
                    f.write_str("|")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        f.write_str(")")
    }
}

// ---------------------------------------------------------------------------
// Pointer
// ---------------------------------------------------------------------------

/// Identitas satu penunjuk. Mouse selalu [`PointerId::MOUSE`]; sentuhan dan
/// pena mendapat id per jari/alat sehingga multi-touch bisa dilacak terpisah.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PointerId(pub u64);

impl PointerId {
    /// Penunjuk mouse — satu-satunya yang selalu ada di desktop.
    pub const MOUSE: Self = Self(0);
}

/// Jenis alat penunjuk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PointerKind {
    /// Mouse atau trackpad.
    #[default]
    Mouse,
    /// Jari di layar sentuh.
    Touch,
    /// Pena/stylus (bisa membawa tekanan).
    Pen,
}

/// Tombol penunjuk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PointerButton {
    /// Tombol utama (kiri pada mouse tangan-kanan, sentuhan jari).
    Primary,
    /// Tombol sekunder (kanan) — menu konteks.
    Secondary,
    /// Tombol tengah.
    Middle,
    /// Navigasi mundur.
    Back,
    /// Navigasi maju.
    Forward,
    /// Tombol lain menurut nomor OS.
    Other(u16),
}

impl PointerButton {
    /// Nomor bit untuk [`Buttons`]; tombol eksotis dipetakan ke bit terakhir.
    const fn bit(self) -> u8 {
        match self {
            PointerButton::Primary => 1 << 0,
            PointerButton::Secondary => 1 << 1,
            PointerButton::Middle => 1 << 2,
            PointerButton::Back => 1 << 3,
            PointerButton::Forward => 1 << 4,
            PointerButton::Other(_) => 1 << 5,
        }
    }
}

/// Himpunan tombol yang sedang ditahan.
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Buttons(u8);

impl Buttons {
    /// Tidak ada tombol ditahan.
    pub const NONE: Self = Self(0);

    /// Bit mentah.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Benar bila tidak ada tombol ditahan.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Benar bila `button` sedang ditahan.
    pub const fn contains(self, button: PointerButton) -> bool {
        self.0 & button.bit() != 0
    }

    /// Tandai tombol sedang ditahan.
    pub fn insert(&mut self, button: PointerButton) {
        self.0 |= button.bit();
    }

    /// Tandai tombol sudah dilepas.
    pub fn remove(&mut self, button: PointerButton) {
        self.0 &= !button.bit();
    }

    /// Lepas semua tombol (dipakai saat pointer dibatalkan).
    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

impl fmt::Debug for Buttons {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Buttons({:#06b})", self.0)
    }
}

/// Tahap hidup sebuah event penunjuk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PointerPhase {
    /// Penunjuk masuk ke area window.
    Enter,
    /// Penunjuk bergerak.
    Move,
    /// Tombol ditekan.
    Down,
    /// Tombol dilepas.
    Up,
    /// Interaksi dibatalkan OS (window kehilangan fokus, gesture diambil alih).
    ///
    /// Widget **wajib** memperlakukan ini seperti "batal", bukan seperti
    /// [`PointerPhase::Up`]: tombol yang dibatalkan tidak menghasilkan klik.
    Cancel,
    /// Penunjuk meninggalkan area window.
    Leave,
}

/// Satu event penunjuk.
#[derive(Debug, Clone, PartialEq)]
pub struct PointerEvent {
    /// Penunjuk mana.
    pub id: PointerId,
    /// Alat apa.
    pub kind: PointerKind,
    /// Tahap.
    pub phase: PointerPhase,
    /// Posisi global dalam poin logis.
    pub position: Point,
    /// Tombol yang memicu event ini (hanya pada [`PointerPhase::Down`]/`Up`).
    pub button: Option<PointerButton>,
    /// Tombol yang sedang ditahan setelah event ini.
    pub buttons: Buttons,
    /// Modifier keyboard saat event terjadi.
    pub modifiers: Modifiers,
    /// Waktu sejak window dibuka.
    pub time: Duration,
    /// Nomor klik beruntun: 1 = klik tunggal, 2 = ganda, 3 = tripel.
    ///
    /// Diisi router, bukan platform — ambang waktu dan jaraknya milik
    /// framework agar seragam di tiga OS.
    pub click_count: u32,
}

impl PointerEvent {
    /// Event penunjuk mouse sederhana; dipakai konstruktor platform dan test.
    pub fn new(phase: PointerPhase, position: Point, time: Duration) -> Self {
        Self {
            id: PointerId::MOUSE,
            kind: PointerKind::Mouse,
            phase,
            position,
            button: None,
            buttons: Buttons::NONE,
            modifiers: Modifiers::NONE,
            time,
            click_count: 0,
        }
    }

    /// Setel tombol pemicu.
    pub fn button(mut self, button: PointerButton) -> Self {
        self.button = Some(button);
        self
    }

    /// Setel modifier.
    pub fn modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// Benar bila event ini menekan/melepas tombol utama.
    pub fn is_primary(&self) -> bool {
        self.button == Some(PointerButton::Primary)
    }
}

// ---------------------------------------------------------------------------
// Scroll
// ---------------------------------------------------------------------------

/// Besaran guliran.
///
/// Roda mouse melapor dalam **baris**, trackpad dalam **poin logis**. Keduanya
/// dibiarkan apa adanya sampai ke widget: hanya widget yang tahu berapa tinggi
/// satu barisnya.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollDelta {
    /// Kelipatan baris (roda mouse).
    Lines {
        /// Horizontal.
        x: f32,
        /// Vertikal.
        y: f32,
    },
    /// Poin logis (trackpad, layar sentuh).
    Points {
        /// Horizontal.
        x: f32,
        /// Vertikal.
        y: f32,
    },
}

impl ScrollDelta {
    /// Konversi ke poin logis dengan tinggi baris tertentu.
    pub fn to_points(self, line_height: f32) -> Point {
        match self {
            ScrollDelta::Lines { x, y } => Point::new(x * line_height, y * line_height),
            ScrollDelta::Points { x, y } => Point::new(x, y),
        }
    }

    /// Benar bila tidak ada pergerakan sama sekali.
    pub fn is_zero(self) -> bool {
        match self {
            ScrollDelta::Lines { x, y } | ScrollDelta::Points { x, y } => x == 0.0 && y == 0.0,
        }
    }
}

/// Tahap gesture guliran.
///
/// **momentum datang dari OS, bukan dari kita** (INTEGRASI-NATIVE §3): di macOS
/// sistem mengirim sendiri ekor inersia setelah jari diangkat, dan menirunya di
/// framework menghasilkan guliran ganda. Karena itu tahapnya dibawa sampai ke
/// widget: scroll physics kita hanya boleh menyalakan simulasi inersia sendiri
/// bila platform **tidak** menyediakannya.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScrollPhase {
    /// Roda mouse — diskret, tanpa awal/akhir gesture.
    Wheel,
    /// Jari menyentuh trackpad.
    Began,
    /// Jari bergerak.
    Changed,
    /// Jari diangkat; belum tentu diikuti momentum.
    Ended,
    /// Ekor inersia **dari OS**.
    Momentum,
    /// Ekor inersia OS selesai.
    MomentumEnded,
}

impl ScrollPhase {
    /// Benar bila guliran ini adalah inersia yang dihasilkan OS.
    pub fn is_momentum(self) -> bool {
        matches!(self, ScrollPhase::Momentum | ScrollPhase::MomentumEnded)
    }
}

/// Satu event guliran.
#[derive(Debug, Clone, PartialEq)]
pub struct ScrollEvent {
    /// Penunjuk yang menggulir (untuk trackpad = mouse).
    pub id: PointerId,
    /// Posisi kursor saat menggulir — menentukan wadah mana yang menerima.
    pub position: Point,
    /// Besaran.
    pub delta: ScrollDelta,
    /// Tahap gesture.
    pub phase: ScrollPhase,
    /// Modifier (⌘+scroll = zoom di banyak aplikasi).
    pub modifiers: Modifiers,
    /// Waktu sejak window dibuka.
    pub time: Duration,
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

/// Tombol bernama (non-teks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NamedKey {
    /// Tab — navigasi fokus.
    Tab,
    /// Enter/Return.
    Enter,
    /// Escape.
    Escape,
    /// Spasi (tetap bernama walau menghasilkan teks: ia mengaktifkan tombol).
    Space,
    /// Backspace.
    Backspace,
    /// Delete (forward delete).
    Delete,
    /// Insert.
    Insert,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Panah kiri.
    ArrowLeft,
    /// Panah kanan.
    ArrowRight,
    /// Panah atas.
    ArrowUp,
    /// Panah bawah.
    ArrowDown,
    /// Tombol fungsi F1–F24.
    Function(u8),
}

/// Tombol yang ditekan, dalam kosakata **logis** (sudah melewati layout
/// keyboard OS).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyCode {
    /// Tombol yang menghasilkan karakter (sudah sesuai layout & dead key).
    Character(char),
    /// Tombol bernama.
    Named(NamedKey),
    /// Tombol yang tidak bisa diterjemahkan; nomornya milik OS.
    Unidentified,
}

impl KeyCode {
    /// Benar bila ini tombol bernama tertentu.
    pub fn is(&self, named: NamedKey) -> bool {
        matches!(self, KeyCode::Named(n) if *n == named)
    }
}

/// Ditekan atau dilepas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyState {
    /// Ditekan.
    Pressed,
    /// Dilepas.
    Released,
}

/// Satu event keyboard.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyEvent {
    /// Tombol logis.
    pub code: KeyCode,
    /// Ditekan atau dilepas.
    pub state: KeyState,
    /// Benar bila ini pengulangan otomatis karena tombol ditahan.
    pub repeat: bool,
    /// Teks yang dihasilkan tombol ini, bila ada.
    ///
    /// **Selama komposisi IME nilainya diabaikan**: widget teks menahan jalur
    /// key normal dan hanya mendengarkan [`ImeEvent`] (REKOMENDASI §3.8).
    pub text: Option<String>,
    /// Modifier yang ditahan.
    pub modifiers: Modifiers,
    /// Waktu sejak window dibuka.
    pub time: Duration,
}

impl KeyEvent {
    /// Event tombol ditekan tanpa modifier — dipakai test dan pintasan sintetis.
    pub fn pressed(code: KeyCode, time: Duration) -> Self {
        Self {
            code,
            state: KeyState::Pressed,
            repeat: false,
            text: None,
            modifiers: Modifiers::NONE,
            time,
        }
    }

    /// Versi dilepas.
    pub fn released(code: KeyCode, time: Duration) -> Self {
        Self {
            state: KeyState::Released,
            ..Self::pressed(code, time)
        }
    }

    /// Setel modifier.
    pub fn modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// Benar bila tombol sedang ditekan (bukan dilepas).
    pub fn is_pressed(&self) -> bool {
        self.state == KeyState::Pressed
    }
}

// ---------------------------------------------------------------------------
// IME
// ---------------------------------------------------------------------------

/// Event komposisi IME (CJK, dead key, emoji picker).
///
/// Pemetaannya 1:1 dengan `winit::event::Ime` — sengaja, karena bentuk itu
/// adalah bentuk yang sama di ketiga OS. Yang **tidak** boleh bocor ke sini
/// adalah tipe winit-nya sendiri.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeEvent {
    /// IME dinyalakan untuk window ini.
    Enabled,
    /// Teks komposisi berjalan; harus dirender **inline dengan garis bawah**.
    Preedit {
        /// Teks komposisi saat ini (kosong = komposisi dibersihkan).
        text: String,
        /// Rentang kursor di dalam `text`, dalam **indeks byte**.
        cursor: Option<(usize, usize)>,
    },
    /// Teks final yang harus disisipkan.
    Commit(String),
    /// IME dimatikan; preedit yang tersisa harus dibuang.
    Disabled,
}

impl ImeEvent {
    /// Benar bila event ini bagian dari komposisi yang sedang berjalan.
    pub fn is_composing(&self) -> bool {
        matches!(self, ImeEvent::Preedit { text, .. } if !text.is_empty())
    }
}

// ---------------------------------------------------------------------------
// Fokus
// ---------------------------------------------------------------------------

/// Perubahan fokus yang dikirim ke node yang bersangkutan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusEvent {
    /// Node ini menjadi tujuan keyboard.
    Gained,
    /// Node ini berhenti menjadi tujuan keyboard.
    Lost,
}

// ---------------------------------------------------------------------------
// Event
// ---------------------------------------------------------------------------

/// Satu event input apa pun, sebagaimana dilihat render tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Penunjuk (mouse/sentuh/pena).
    Pointer(PointerEvent),
    /// Guliran.
    Scroll(ScrollEvent),
    /// Keyboard.
    Key(KeyEvent),
    /// Komposisi IME.
    Ime(ImeEvent),
    /// Fokus datang/pergi (dikirim langsung ke node, tidak menggelembung).
    Focus(FocusEvent),
}

impl Event {
    /// Posisi global event ini, bila ia punya posisi.
    pub fn position(&self) -> Option<Point> {
        match self {
            Event::Pointer(e) => Some(e.position),
            Event::Scroll(e) => Some(e.position),
            _ => None,
        }
    }

    /// Waktu event, bila ada.
    pub fn time(&self) -> Option<Duration> {
        match self {
            Event::Pointer(e) => Some(e.time),
            Event::Scroll(e) => Some(e.time),
            Event::Key(e) => Some(e.time),
            _ => None,
        }
    }
}

impl From<PointerEvent> for Event {
    fn from(e: PointerEvent) -> Self {
        Event::Pointer(e)
    }
}

impl From<ScrollEvent> for Event {
    fn from(e: ScrollEvent) -> Self {
        Event::Scroll(e)
    }
}

impl From<KeyEvent> for Event {
    fn from(e: KeyEvent) -> Self {
        Event::Key(e)
    }
}

impl From<ImeEvent> for Event {
    fn from(e: ImeEvent) -> Self {
        Event::Ime(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_adalah_bitset() {
        let mut m = Modifiers::SHIFT;
        m |= Modifiers::ALT;
        assert!(m.contains(Modifiers::SHIFT | Modifiers::ALT));
        assert!(!m.contains(Modifiers::META));
        m.remove(Modifiers::SHIFT);
        assert!(m.is_exactly(Modifiers::ALT));
    }

    #[test]
    fn is_exactly_menolak_modifier_tambahan() {
        let m = Modifiers::SHIFT | Modifiers::CONTROL;
        assert!(m.contains(Modifiers::SHIFT));
        assert!(!m.is_exactly(Modifiers::SHIFT));
        assert!(Modifiers::NONE.is_exactly(Modifiers::NONE));
    }

    #[test]
    fn command_mengikuti_platform() {
        // Bukan konstanta yang sama di semua OS — itulah gunanya.
        #[cfg(target_os = "macos")]
        assert_eq!(Modifiers::COMMAND, Modifiers::META);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(Modifiers::COMMAND, Modifiers::CONTROL);
    }

    #[test]
    fn debug_modifiers_menyebut_namanya() {
        let m = Modifiers::SHIFT | Modifiers::META;
        assert_eq!(format!("{m:?}"), "Modifiers(shift|meta)");
        assert_eq!(format!("{:?}", Modifiers::NONE), "Modifiers(none)");
    }

    #[test]
    fn buttons_melacak_tekanan() {
        let mut b = Buttons::NONE;
        assert!(b.is_empty());
        b.insert(PointerButton::Primary);
        b.insert(PointerButton::Secondary);
        assert!(b.contains(PointerButton::Primary));
        b.remove(PointerButton::Primary);
        assert!(!b.contains(PointerButton::Primary));
        assert!(b.contains(PointerButton::Secondary));
        b.clear();
        assert!(b.is_empty());
    }

    #[test]
    fn scroll_baris_dikonversi_dengan_tinggi_baris() {
        let d = ScrollDelta::Lines { x: 0.0, y: -3.0 };
        assert_eq!(d.to_points(20.0), Point::new(0.0, -60.0));
        let p = ScrollDelta::Points { x: 4.0, y: 8.0 };
        assert_eq!(p.to_points(20.0), Point::new(4.0, 8.0));
    }

    #[test]
    fn momentum_dikenali_sebagai_milik_os() {
        assert!(ScrollPhase::Momentum.is_momentum());
        assert!(ScrollPhase::MomentumEnded.is_momentum());
        assert!(!ScrollPhase::Wheel.is_momentum());
        assert!(!ScrollPhase::Changed.is_momentum());
    }

    #[test]
    fn preedit_kosong_bukan_komposisi() {
        let habis = ImeEvent::Preedit {
            text: String::new(),
            cursor: None,
        };
        assert!(!habis.is_composing());
        let jalan = ImeEvent::Preedit {
            text: "に".into(),
            cursor: Some((0, 3)),
        };
        assert!(jalan.is_composing());
    }

    #[test]
    fn event_membawa_posisi_hanya_bila_punya() {
        let p = Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            Point::new(3.0, 4.0),
            Duration::ZERO,
        ));
        assert_eq!(p.position(), Some(Point::new(3.0, 4.0)));
        assert_eq!(Event::Ime(ImeEvent::Enabled).position(), None);
        assert_eq!(Event::Focus(FocusEvent::Gained).time(), None);
    }
}
