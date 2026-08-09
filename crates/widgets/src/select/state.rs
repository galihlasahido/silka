//! Keadaan sebuah select dan satu-satunya tempat aturannya hidup.
//!
//! [`SelectState`] sengaja **data murni**: tidak ada node, tidak ada callback,
//! tidak ada theme. Semua yang bisa salah pada sebuah dropdown — sorotan yang
//! keluar batas, gulir yang tidak mengikuti sorotan, popup yang lupa menutup
//! setelah memilih — diselesaikan di [`SelectState::apply`] sebagai fungsi
//! `(keadaan, niat) → keadaan`. Karena itu seluruhnya bisa diuji tanpa GPU,
//! tanpa font, dan tanpa satu pun frame (§9.5).

use rustui_paint::Rect;

use crate::overlay::Anchor;

/// Apa yang **diminta** pengguna terhadap sebuah select.
///
/// Node render tidak pernah mengubah pilihan sendiri: ia hanya melapor niat,
/// dan aplikasi (atau [`SelectState::apply`]) yang memutuskan. Itulah yang
/// membuat select bisa dikendalikan penuh dari signal — pola "controlled
/// component" yang sama dengan `Viewport::scroll` (§2.5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SelectIntent {
    /// Buka popup; kotaknya adalah **kotak global pemicu**, calon jangkar.
    Open(Rect),
    /// Tutup popup tanpa mengubah pilihan.
    Close,
    /// Pindahkan sorotan ke indeks ini (arah panah, hover, typeahead).
    Highlight(usize),
    /// Pilih indeks ini lalu tutup.
    Commit(usize),
}

/// Keadaan satu select yang **dimiliki aplikasi**.
///
/// Ringkas dan `Copy` supaya muat di satu
/// [`Signal`](rustui_core::signals::Signal): satu titipan state, bukan empat.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SelectState {
    /// Popup sedang terbuka.
    pub open: bool,
    /// Indeks yang terpilih; `None` = belum ada (placeholder tampil).
    pub selected: Option<usize>,
    /// Indeks yang sedang disorot di dalam popup (keyboard/hover).
    pub highlight: usize,
    /// Baris pertama yang terlihat saat daftar lebih panjang dari jendelanya.
    ///
    /// Inilah yang membuat sorotan keyboard **selalu ikut terlihat** tanpa
    /// state kedua di dalam node: posisi gulir adalah turunan darinya
    /// ([`SelectState::scroll_offset`]).
    pub first_visible: usize,
    /// Kotak pemicu pada koordinat lokal layer overlay — jangkar popup.
    pub anchor: Anchor,
}

impl SelectState {
    /// Keadaan awal: tertutup, belum memilih apa pun.
    pub fn new() -> Self {
        Self::default()
    }

    /// Keadaan awal dengan satu pilihan sudah aktif.
    pub fn with_selected(index: usize) -> Self {
        Self {
            selected: Some(index),
            highlight: index,
            ..Self::default()
        }
    }

    /// Benar bila popup sedang terbuka.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Posisi gulir daftar, poin logis — turunan [`SelectState::first_visible`].
    pub fn scroll_offset(&self, row_height: f32) -> f32 {
        self.first_visible as f32 * row_height.max(0.0)
    }

    /// Terapkan sebuah niat; benar bila keadaannya benar-benar berubah.
    ///
    /// `count` adalah jumlah pilihan dan `visible` jumlah baris yang muat di
    /// jendela popup. Keduanya diserahkan pemanggil karena keduanya milik
    /// tampilan, bukan milik keadaan — select yang sama bisa ditampilkan dua
    /// kali dengan tinggi berbeda.
    pub fn apply(&mut self, intent: SelectIntent, count: usize, visible: usize) -> bool {
        let sebelum = *self;
        match intent {
            SelectIntent::Open(kotak) => {
                self.open = true;
                self.anchor = Anchor::Rect(kotak);
                // Popup selalu terbuka dengan sorotan pada yang terpilih —
                // kebiasaan NSPopUpButton, dan yang membuat panah pertama
                // bergerak dari tempat yang benar.
                let mulai = self.selected.unwrap_or(0);
                self.set_highlight(mulai, count, visible);
            }
            SelectIntent::Close => self.open = false,
            SelectIntent::Highlight(i) => self.set_highlight(i, count, visible),
            SelectIntent::Commit(i) => {
                if count > 0 {
                    let i = i.min(count - 1);
                    self.selected = Some(i);
                    self.set_highlight(i, count, visible);
                }
                self.open = false;
            }
        }
        *self != sebelum
    }

    /// Pindahkan sorotan, jepit ke rentang yang sah, lalu pastikan terlihat.
    fn set_highlight(&mut self, index: usize, count: usize, visible: usize) {
        if count == 0 {
            self.highlight = 0;
            self.first_visible = 0;
            return;
        }
        self.highlight = index.min(count - 1);
        self.reveal(count, visible);
    }

    /// Geser jendela seminimal mungkin agar sorotan berada di dalamnya.
    ///
    /// "Seminimal mungkin" itu penting: menggulir ke tengah setiap kali sorotan
    /// pindah membuat daftar terasa melompat-lompat, dan itulah bedanya listbox
    /// yang enak dengan yang membingungkan.
    fn reveal(&mut self, count: usize, visible: usize) {
        let jendela = visible.max(1).min(count);
        if self.highlight < self.first_visible {
            self.first_visible = self.highlight;
        } else if self.highlight >= self.first_visible + jendela {
            self.first_visible = self.highlight + 1 - jendela;
        }
        self.first_visible = self.first_visible.min(count - jendela);
    }
}
