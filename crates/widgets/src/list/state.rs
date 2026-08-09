//! [`ListState`] — posisi guliran dan baris terpilih sebuah daftar.
//!
//! State ini **wajib hidup di luar view**, karena view dibangun ulang setiap
//! kali ada signal berubah sementara jari pengguna masih menggulir. Ia juga
//! yang menutup lingkaran virtualisasi:
//!
//! ```text
//! roda/trackpad → ScrollView::event → posisi guliran berubah
//! frame berikut → super::sync       → tulis ListState.scroll   (signal)
//!                                   → komponen daftar dirty
//!                 rebuild daftar    → baca ListState.scroll
//!                                   → bangun HANYA baris yang terlihat
//! ```
//!
//! Karena itu posisi guliran memang harus berupa [`Signal`]: tanpa
//! pemberitahuan, jendela baris yang dibangun tidak akan pernah menyusul
//! guliran, dan daftar akan tampak kosong begitu digulir.

use silka_core::signals::{use_signal, Runtime, Signal};

use super::geometry::ListMetrics;

/// Keadaan guliran sebuah daftar, sebagai satu nilai yang bisa dibaca saat
/// build.
///
/// Semua field-nya **hasil pengukuran**, bukan properti: [`super::ListBody`]
/// mengisi tinggi isi saat layout, dan [`super::sync`] mengisi posisi guliran
/// dari wadah gulir di atasnya. Aplikasi boleh membacanya (mis. untuk prefetch
/// data yang akan terlihat); yang boleh **diminta** hanyalah posisi baru, lewat
/// [`ListState::scroll_to`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ListScroll {
    /// Posisi guliran (poin logis, 0 = paling atas).
    pub offset: f32,
    /// Tinggi jendela pandang hasil layout terakhir; 0 = belum pernah.
    pub viewport: f32,
    /// Tinggi seluruh isi (header + semua baris).
    pub content: f32,
    /// Tinggi satu baris.
    pub extent: f32,
    /// Tinggi header; 0 = tanpa header.
    pub header: f32,
}

impl ListScroll {
    /// Guliran maksimum yang masih menyisakan isi di layar.
    pub fn max_scroll(&self) -> f32 {
        (self.content - self.viewport).max(0.0)
    }

    /// Benar bila daftar sedang menempel di ujung atas.
    pub fn is_at_top(&self) -> bool {
        self.offset <= 0.0
    }

    /// Benar bila daftar sedang menempel di ujung bawah.
    pub fn is_at_bottom(&self) -> bool {
        self.offset >= self.max_scroll()
    }

    /// Rentang baris yang sedang terlihat, untuk prefetch data.
    pub fn visible_range(&self, count: usize) -> super::ListRange {
        ListMetrics {
            count,
            extent: self.extent,
            header: self.header,
            sticky: false,
            viewport: self.viewport,
        }
        .visible_range(self.offset, 0)
    }
}

/// State sebuah daftar: posisi guliran + baris terpilih.
///
/// `Copy` dan seukuran dua ID — boleh masuk ke closure `move` sebanyak yang
/// diperlukan, persis seperti [`Signal`] (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListState {
    scroll: Signal<ListScroll>,
    selected: Signal<Option<usize>>,
    /// Permintaan `scroll_to` yang belum dilayani.
    ///
    /// Kanal terpisah dari [`ListScroll::offset`] dengan sengaja: `offset`
    /// adalah **hasil pengukuran** yang diterbitkan ulang setiap frame, jadi
    /// perintah yang dititipkan di sana akan tertimpa sebelum sempat dibaca
    /// siapa pun. Perintah dan hasil pengukuran tidak boleh berbagi satu
    /// tempat.
    request: Signal<Option<f32>>,
}

impl ListState {
    /// State baru di dalam sebuah runtime — bentuk yang dipakai uji dan
    /// aplikasi yang memegang state-nya sendiri di tingkat aplikasi.
    pub fn new(runtime: &Runtime) -> Self {
        Self {
            scroll: runtime.signal(ListScroll::default()),
            selected: runtime.signal(None),
            request: runtime.signal(None),
        }
    }

    /// Keadaan guliran saat ini — **melacak** bila dipanggil saat build.
    ///
    /// Inilah pembacaan yang membuat komponen daftar dibangun ulang saat
    /// digulir, dan karena itulah jendela barisnya selalu menyusul.
    pub fn scroll(&self) -> ListScroll {
        self.scroll.get()
    }

    /// Keadaan guliran **tanpa** berlangganan.
    pub fn peek_scroll(&self) -> ListScroll {
        self.scroll.peek()
    }

    /// Posisi guliran saat ini, tanpa berlangganan.
    pub fn offset(&self) -> f32 {
        self.scroll.peek().offset
    }

    /// Gulir ke posisi tertentu; daftar **beranimasi** ke sana lewat spring
    /// milik [`scroll_view`](mod@crate::scroll_view), bukan melompat.
    ///
    /// Permintaannya dilayani pada frame berikutnya oleh [`super::sync`] —
    /// satu-satunya pihak yang boleh menyentuh wadah gulir di atas daftar.
    pub fn scroll_to(&self, offset: f32) {
        self.request.set(Some(offset));
    }

    /// Gulir sampai baris `index` berada di tepi atas.
    pub fn scroll_to_item(&self, index: usize, count: usize) {
        let s = self.scroll.peek();
        let m = ListMetrics {
            count,
            extent: s.extent,
            header: s.header,
            sticky: false,
            viewport: s.viewport,
        };
        self.scroll_to(m.scroll_to_item(index));
    }

    /// Permintaan `scroll_to` yang tertunda — **melacak**, dan itu memang
    /// gunanya: komponen daftar harus berlangganan supaya `scroll_to` dari
    /// sebuah event handler benar-benar menjadwalkan frame.
    pub(crate) fn pending_scroll(&self) -> Option<f32> {
        self.request.get()
    }

    /// Ambil permintaan `scroll_to` yang tertunda (dipanggil [`super::sync`]).
    pub(crate) fn take_request(&self) -> Option<f32> {
        if !self.request.is_alive() {
            return None;
        }
        let permintaan = self.request.peek();
        if permintaan.is_some() {
            self.request.set(None);
        }
        permintaan
    }

    /// Baris yang sedang terpilih — **melacak** bila dipanggil saat build.
    pub fn selected(&self) -> Option<usize> {
        self.selected.get()
    }

    /// Pilih sebuah baris (atau `None` untuk melepas seleksi).
    pub fn select(&self, index: Option<usize>) {
        self.selected.set_if_changed(index);
    }

    /// Benar bila seluruh signal masih hidup (scope pemiliknya belum dibuang).
    ///
    /// Node render bisa hidup sesaat lebih lama daripada scope yang
    /// membangunnya saat sebuah daftar dilepas dari pohon; menulis ke signal
    /// mati adalah panik, jadi penulisan selalu lewat penjaga ini.
    pub fn is_alive(&self) -> bool {
        self.scroll.is_alive() && self.selected.is_alive() && self.request.is_alive()
    }

    /// Terbitkan hasil pengukuran layout; hanya menulis bila memang berubah.
    ///
    /// "Hanya bila berubah" bukan optimasi melainkan syarat: setiap tulisan
    /// signal menjadwalkan frame, dan menulis nilai yang sama setiap layout
    /// akan membuat aplikasi berputar selamanya pada 120 fps tanpa ada satu
    /// piksel pun yang berubah (§3.5 "render hanya saat dirty").
    pub(super) fn publish(&self, scroll: ListScroll) -> bool {
        if !self.scroll.is_alive() {
            return false;
        }
        self.scroll.set_if_changed(scroll)
    }

    /// Terbitkan apa yang **hanya diketahui isi daftar**: tinggi seluruh isi,
    /// tinggi baris, tinggi header.
    ///
    /// Dipanggil dari layout [`super::ListBody`]. Tulisan pertamanya juga yang
    /// membangunkan frame kedua sebuah daftar yang baru lahir — dan frame
    /// kedua itulah yang pertama kali bisa membaca tinggi jendela yang
    /// sebenarnya lewat [`ListState::publish_view`].
    pub(crate) fn publish_content(&self, content: f32, extent: f32, header: f32) -> bool {
        if !self.scroll.is_alive() {
            return false;
        }
        let lama = self.scroll.peek();
        if lama.content == content && lama.extent == extent && lama.header == header {
            return false;
        }
        self.publish(ListScroll {
            content,
            extent,
            header,
            ..lama
        })
    }

    /// Terbitkan apa yang **hanya diketahui wadah gulir**: posisi guliran dan
    /// tinggi jendela pandang.
    ///
    /// Dipanggil [`super::sync`] sekali per frame, sebelum rebuild — itulah
    /// yang membuat jendela baris menyusul guliran pada frame yang sama.
    pub(crate) fn publish_view(&self, offset: f32, viewport: f32) -> bool {
        if !self.scroll.is_alive() {
            return false;
        }
        let lama = self.scroll.peek();
        if lama.offset == offset && lama.viewport == viewport {
            return false;
        }
        self.publish(ListScroll {
            offset,
            viewport,
            ..lama
        })
    }

    /// Setel seleksi dari dalam node (mengabaikan signal yang sudah mati).
    pub(super) fn publish_selection(&self, index: Option<usize>) -> bool {
        if !self.selected.is_alive() {
            return false;
        }
        self.selected.set_if_changed(index)
    }

    /// Kunci identitas komponen daftar ini — diturunkan dari identitas
    /// state-nya, sehingga dua daftar bersaudara tidak pernah bertabrakan
    /// walau penulisnya lupa memberi kunci.
    pub(crate) fn component_key(&self) -> String {
        format!("list:{}", self.scroll.id().index())
    }
}

/// State daftar milik komponen yang sedang dibangun (§2.5).
///
/// Hook: dipanggil sekali per build, tidak boleh di dalam `if`/`loop`.
///
/// ```ignore
/// let daftar = use_list_state();
/// list(&t, daftar, baris.len(), move |i| baris_view(i)).item_extent(44.0)
/// ```
pub fn use_list_state() -> ListState {
    let scroll = use_signal(ListScroll::default);
    let selected = use_signal(|| None);
    let request = use_signal(|| None);
    ListState {
        scroll,
        selected,
        request,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_hanya_menulis_saat_berubah() {
        let rt = Runtime::new();
        let state = ListState::new(&rt);
        let s = ListScroll {
            offset: 10.0,
            viewport: 440.0,
            content: 4400.0,
            extent: 44.0,
            header: 0.0,
        };
        assert!(state.publish(s), "nilai pertama selalu berubah");
        assert!(
            !state.publish(s),
            "nilai sama tidak boleh membangunkan frame"
        );
        assert_eq!(state.offset(), 10.0);
    }

    #[test]
    fn scroll_to_adalah_permintaan_bukan_hasil_pengukuran() {
        let rt = Runtime::new();
        let state = ListState::new(&rt);
        state.publish(ListScroll {
            offset: 0.0,
            viewport: 440.0,
            content: 4400.0,
            extent: 44.0,
            header: 8.0,
        });
        state.scroll_to(120.0);

        // Hasil pengukuran **tidak** dipalsukan: `offset` tetap apa adanya
        // sampai wadah gulir benar-benar bergerak.
        let s = state.peek_scroll();
        assert_eq!(s.offset, 0.0);
        assert_eq!(s.viewport, 440.0);

        assert_eq!(state.take_request(), Some(120.0));
        assert_eq!(
            state.take_request(),
            None,
            "permintaan hanya dilayani sekali"
        );
    }

    #[test]
    fn scroll_to_item_memakai_ukuran_hasil_layout() {
        let rt = Runtime::new();
        let state = ListState::new(&rt);
        state.publish(ListScroll {
            offset: 0.0,
            viewport: 440.0,
            content: 4400.0,
            extent: 44.0,
            header: 0.0,
        });
        state.scroll_to_item(10, 100);
        assert_eq!(state.take_request(), Some(440.0));
        // Tidak pernah melewati ujung.
        state.scroll_to_item(99, 100);
        assert_eq!(state.take_request(), Some(4400.0 - 440.0));
    }

    #[test]
    fn rentang_terlihat_bisa_dibaca_aplikasi_untuk_prefetch() {
        let rt = Runtime::new();
        let state = ListState::new(&rt);
        state.publish(ListScroll {
            offset: 440.0,
            viewport: 440.0,
            content: 44.0 * 1000.0,
            extent: 44.0,
            header: 0.0,
        });
        let r = state.peek_scroll().visible_range(1000);
        assert_eq!(r.first, 10);
        assert_eq!(r.len, 10);
    }

    #[test]
    fn seleksi_hanya_menandai_dirty_saat_benar_benar_berubah() {
        let rt = Runtime::new();
        let state = ListState::new(&rt);
        assert!(state.publish_selection(Some(3)));
        assert!(!state.publish_selection(Some(3)));
        assert_eq!(state.selected.peek(), Some(3));
    }

    #[test]
    fn kunci_komponen_berbeda_untuk_dua_daftar() {
        let rt = Runtime::new();
        let a = ListState::new(&rt);
        let b = ListState::new(&rt);
        assert_ne!(a.component_key(), b.component_key());
    }
}
