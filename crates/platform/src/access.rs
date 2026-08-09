//! Adapter aksesibilitas winit — jembatan antara pass a11y `rustui-core` dan
//! API aksesibilitas OS (REKOMENDASI §3.8).
//!
//! Yang menyeberang dari framework ke sini hanyalah
//! [`AccessTree`]/[`AccessUpdate`]: satu snapshot pohon beserta selisihnya.
//! Yang menyeberang balik hanyalah [`AccessActionRequest`] yang sudah
//! divalidasi. `accesskit_winit` mengurus sisanya per platform: UIA di
//! Windows, NSAccessibility di macOS, AT-SPI di Linux.
//!
//! ## Tiga aturan yang mudah dilanggar dan sudah dikunci di sini
//!
//! 1. **Adapter dibuat sebelum window terlihat.** `accesskit_winit` panik
//!    kalau tidak — jadi shell membuat window dalam keadaan tersembunyi,
//!    memasang adapter, baru menampilkannya.
//! 2. **Nol biaya saat tidak ada teknologi bantu.** Pass a11y hanya dijalankan
//!    kalau adapter sedang aktif ([`AccessAdapter::update_with`]); pengguna
//!    yang tidak memakai screen reader tidak membayar apa pun. Ini perpanjangan
//!    langsung dari "render hanya saat dirty" (§3.5).
//! 3. **Aktivasi ulang selalu mengirim pohon penuh.** Screen reader yang baru
//!    dinyalakan tidak punya riwayat; mengirim delta ke sana berarti pohon yang
//!    tidak pernah lengkap.

use accesskit_winit::Adapter;
use rustui_core::access::{AccessActionRequest, AccessTree};
use winit::event::WindowEvent as WinitWindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

/// Event aksesibilitas yang datang dari OS lewat event loop winit.
///
/// Dipakai sebagai *user event* event loop shell. Newtype, bukan alias, supaya
/// event loop aplikasi bisa membawa event lain di kemudian hari tanpa memaksa
/// perubahan API.
#[derive(Debug)]
pub struct AccessEvent(pub accesskit_winit::Event);

impl From<accesskit_winit::Event> for AccessEvent {
    fn from(event: accesskit_winit::Event) -> Self {
        Self(event)
    }
}

impl AccessEvent {
    /// Window yang dimaksud event ini.
    pub fn window_id(&self) -> WindowId {
        self.0.window_id
    }
}

/// Apa yang harus dilakukan shell setelah sebuah [`AccessEvent`] diproses.
#[derive(Debug, Clone, PartialEq)]
pub enum AccessOutcome {
    /// Teknologi bantu meminta pohon lengkap — kirim satu update penuh.
    NeedsFullTree,
    /// Teknologi bantu meminta sebuah aksi pada sebuah node.
    Action(AccessActionRequest),
    /// Tidak ada yang perlu dilakukan (mis. aksesibilitas dimatikan).
    Idle,
}

/// Adapter aksesibilitas untuk satu window.
pub struct AccessAdapter {
    inner: Adapter,
    /// Snapshot terakhir yang benar-benar dikirim — dasar delta **dan** dasar
    /// penerjemahan permintaan aksi. Sengaja bukan pohon terbaru: teknologi
    /// bantu selalu berbicara tentang pohon yang pernah ia lihat.
    terkirim: Option<AccessTree>,
    /// Benar setelah aktivasi, sampai teknologi bantu dimatikan lagi.
    aktif: bool,
}

impl AccessAdapter {
    /// Pasang adapter pada sebuah window.
    ///
    /// **Wajib dipanggil sebelum window ditampilkan** — buat window dengan
    /// `with_visible(false)`, panggil ini, lalu `set_visible(true)`.
    pub fn new(
        event_loop: &ActiveEventLoop,
        window: &Window,
        proxy: EventLoopProxy<AccessEvent>,
    ) -> Self {
        Self {
            inner: Adapter::with_event_loop_proxy(event_loop, window, proxy),
            terkirim: None,
            aktif: false,
        }
    }

    /// Benar bila ada teknologi bantu yang sedang mendengarkan.
    pub fn is_active(&self) -> bool {
        self.aktif
    }

    /// Teruskan event window ke adapter.
    ///
    /// Harus dipanggil untuk **setiap** event window, sebelum shell
    /// memprosesnya sendiri: fokus dan geometri window ikut dari sini.
    pub fn process_event(&mut self, window: &Window, event: &WinitWindowEvent) {
        self.inner.process_event(window, event);
    }

    /// Tangani event aksesibilitas dari event loop.
    pub fn handle(&mut self, event: &AccessEvent) -> AccessOutcome {
        match &event.0.window_event {
            accesskit_winit::WindowEvent::InitialTreeRequested => {
                self.aktif = true;
                // Riwayat dibuang: penerima yang baru datang harus mendapat
                // pohon penuh, bukan potongan perubahan yang tidak ia punya
                // dasarnya.
                self.terkirim = None;
                AccessOutcome::NeedsFullTree
            }
            accesskit_winit::WindowEvent::ActionRequested(request) => self
                .terkirim
                .as_ref()
                .and_then(|pohon| pohon.action_request(request))
                .map(AccessOutcome::Action)
                .unwrap_or(AccessOutcome::Idle),
            accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                self.aktif = false;
                self.terkirim = None;
                AccessOutcome::Idle
            }
        }
    }

    /// Bangun pohon a11y **hanya bila ada yang mendengarkan**, lalu kirim
    /// selisihnya.
    ///
    /// `scale_factor` adalah scale factor window: AccessKit menuntut koordinat
    /// piksel fisik, sedangkan seluruh framework di atasnya berbicara poin
    /// logis.
    pub fn update_with(&mut self, scale_factor: f64, build: impl FnOnce() -> AccessTree) {
        if !self.aktif {
            return;
        }
        let pohon = build();
        let update = pohon.changes_since(self.terkirim.as_ref());
        if update.is_empty() {
            return;
        }
        self.inner
            .update_if_active(|| update.to_tree_update(scale_factor));
        self.terkirim = Some(pohon);
    }

    /// Kirim satu pohon penuh, apa pun riwayatnya.
    ///
    /// Dipakai saat menjawab [`AccessOutcome::NeedsFullTree`].
    pub fn update_full(&mut self, scale_factor: f64, pohon: AccessTree) {
        self.aktif = true;
        self.inner
            .update_if_active(|| pohon.to_tree_update(scale_factor));
        self.terkirim = Some(pohon);
    }

    /// Snapshot terakhir yang dikirim — sudut pandang teknologi bantu saat ini.
    pub fn last_sent(&self) -> Option<&AccessTree> {
        self.terkirim.as_ref()
    }
}

impl core::fmt::Debug for AccessAdapter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AccessAdapter")
            .field("aktif", &self.aktif)
            .field("node_terkirim", &self.terkirim.as_ref().map(|t| t.len()))
            .finish()
    }
}
