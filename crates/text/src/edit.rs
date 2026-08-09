//! **Model editing teks**: caret per grapheme, seleksi, undo/redo, preedit IME.
//!
//! Ini separuh non-visual dari `text_field` (`KOMPONEN.md` Tier 2, "komponen
//! tersulit di seluruh katalog"). Ia sengaja hidup di `silka-text`, bukan di
//! widget, karena tiga alasan:
//!
//! 1. **Aturannya aturan Unicode, bukan aturan tampilan.** Gerakan caret per
//!    grapheme cluster dan batas kata adalah UAX #29 (§3.3) — sama persis untuk
//!    `text_field`, `text_area`, `combo_box`, dan nanti `code_editor`.
//! 2. **Bisa diuji tanpa satu piksel pun.** Seluruh berkas ini tidak menyentuh
//!    font, GPU, maupun render tree; testnya jalan di CI tanpa layar (§9.5).
//! 3. **Preedit IME adalah keadaan model, bukan hiasan.** Selama komposisi
//!    berjalan, teks yang *terlihat* bukan teks yang *tersimpan* — bedanya
//!    dijaga di sini ([`TextEdit::display_text`]), supaya widget tidak pernah
//!    salah menyimpan huruf setengah jadi ke aplikasi.
//!
//! ```
//! use silka_text::edit::{Movement, TextEdit};
//!
//! let mut e = TextEdit::new("halo");
//! e.move_caret(Movement::LineEnd, false);
//! e.insert(" dunia");
//! assert_eq!(e.text(), "halo dunia");
//!
//! // Satu kata yang diketik beruntun = satu langkah undo.
//! e.undo();
//! assert_eq!(e.text(), "halo");
//! ```
//!
//! Yang **tidak** ada di sini: koordinat, piksel, dan warna. Geometri caret dan
//! seleksi datang dari [`crate::TextLayout`], yang tahu hasil shaping-nya.

use std::borrow::Cow;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

// ---------------------------------------------------------------------------
// Grapheme & kata
// ---------------------------------------------------------------------------

/// Jepit `index` ke batas grapheme terdekat **ke kiri**.
///
/// Indeks yang datang dari luar (klik, aplikasi, dikte suara) tidak pernah
/// dipercaya: caret yang berhenti di tengah karakter 4 byte atau di tengah
/// emoji ZWJ adalah bug yang berakhir dengan `String` yang panik saat dipotong.
pub fn snap_grapheme(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut batas = 0;
    for (i, _) in text.grapheme_indices(true) {
        if i > index {
            break;
        }
        batas = i;
    }
    batas
}

/// Batas grapheme berikutnya setelah `index` (UAX #29).
///
/// Satu langkah = satu **grapheme cluster**, bukan satu `char`: "é" yang
/// tersusun dari e + combining acute, bendera, dan emoji keluarga ZWJ
/// masing-masing dilewati sekali tekan.
pub fn next_grapheme(text: &str, index: usize) -> usize {
    let index = snap_grapheme(text, index);
    text.grapheme_indices(true)
        .map(|(i, g)| i + g.len())
        .find(|&akhir| akhir > index)
        .unwrap_or(text.len())
}

/// Batas grapheme sebelum `index`.
pub fn prev_grapheme(text: &str, index: usize) -> usize {
    let index = snap_grapheme(text, index);
    text.grapheme_indices(true)
        .map(|(i, _)| i)
        .filter(|&awal| awal < index)
        .next_back()
        .unwrap_or(0)
}

/// Benar bila potongan ini dianggap "kata" untuk keperluan lompat/seleksi.
fn kata(potong: &str) -> bool {
    potong.chars().any(char::is_alphanumeric)
}

/// Akhir kata berikutnya di kanan `index` — padanan ⌥→ di macOS.
pub fn next_word(text: &str, index: usize) -> usize {
    let index = snap_grapheme(text, index);
    for (awal, potong) in text.split_word_bound_indices() {
        let akhir = awal + potong.len();
        if akhir > index && kata(potong) {
            return akhir;
        }
    }
    text.len()
}

/// Awal kata sebelum `index` — padanan ⌥←.
pub fn prev_word(text: &str, index: usize) -> usize {
    let index = snap_grapheme(text, index);
    let mut hasil = 0;
    for (awal, potong) in text.split_word_bound_indices() {
        if awal >= index {
            break;
        }
        if kata(potong) {
            hasil = awal;
        }
    }
    hasil
}

/// Rentang kata yang memuat `index` — inilah yang diseleksi **klik ganda**.
///
/// Klik ganda pada spasi menyeleksi rentang spasi itu, sama seperti AppKit:
/// yang dikembalikan adalah potongan batas-kata tempat `index` berada, apa pun
/// isinya.
pub fn word_range(text: &str, index: usize) -> Range<usize> {
    if text.is_empty() {
        return 0..0;
    }
    let index = snap_grapheme(text, index);
    let mut terakhir = 0..0;
    for (awal, potong) in text.split_word_bound_indices() {
        let akhir = awal + potong.len();
        terakhir = awal..akhir;
        if index < akhir {
            return terakhir;
        }
    }
    terakhir
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

/// Seleksi teks sebagai pasangan **indeks byte**: tempat seret dimulai
/// (`anchor`) dan tempat caret sekarang (`focus`).
///
/// Keduanya dibedakan dengan sengaja: Shift+← memindahkan `focus` dan
/// membiarkan `anchor`, dan itulah satu-satunya cara seleksi terasa benar saat
/// arah seretnya berbalik.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    /// Titik tambat (tidak bergerak saat seleksi diperluas).
    pub anchor: usize,
    /// Posisi caret (yang bergerak).
    pub focus: usize,
}

impl Selection {
    /// Caret tanpa seleksi pada `at`.
    pub const fn caret(at: usize) -> Self {
        Self {
            anchor: at,
            focus: at,
        }
    }

    /// Seleksi dari `anchor` ke `focus`.
    pub const fn new(anchor: usize, focus: usize) -> Self {
        Self { anchor, focus }
    }

    /// Batas kiri.
    pub fn start(self) -> usize {
        self.anchor.min(self.focus)
    }

    /// Batas kanan.
    pub fn end(self) -> usize {
        self.anchor.max(self.focus)
    }

    /// Rentang byte yang terseleksi.
    pub fn range(self) -> Range<usize> {
        self.start()..self.end()
    }

    /// Benar bila tidak ada teks yang terseleksi (hanya caret).
    pub fn is_collapsed(self) -> bool {
        self.anchor == self.focus
    }

    /// Jepit kedua ujungnya ke batas grapheme `text`.
    pub fn snapped(self, text: &str) -> Self {
        Self {
            anchor: snap_grapheme(text, self.anchor),
            focus: snap_grapheme(text, self.focus),
        }
    }
}

// ---------------------------------------------------------------------------
// Preedit
// ---------------------------------------------------------------------------

/// Komposisi IME yang sedang berjalan (CJK, dead key, emoji picker).
///
/// Teksnya **belum** masuk ke nilai yang dipegang aplikasi: ia hidup di sini
/// sampai IME mengirim commit. Itulah yang membuat `on_change` tidak pernah
/// melaporkan huruf setengah jadi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preedit {
    /// Teks komposisi.
    pub text: String,
    /// Rentang kursor **di dalam** `text` (indeks byte), bila IME memberinya.
    pub cursor: Option<(usize, usize)>,
    /// Posisi sisipan komposisi di dalam teks tersimpan.
    pub at: usize,
}

// ---------------------------------------------------------------------------
// Gerakan
// ---------------------------------------------------------------------------

/// Satu langkah gerakan caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Movement {
    /// Satu grapheme ke kiri (←).
    Prev,
    /// Satu grapheme ke kanan (→).
    Next,
    /// Satu kata ke kiri (⌥←).
    PrevWord,
    /// Satu kata ke kanan (⌥→).
    NextWord,
    /// Awal baris (⌘← / Home).
    LineStart,
    /// Akhir baris (⌘→ / End).
    LineEnd,
}

/// Jenis suntingan terakhir — dasar penggabungan langkah undo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Jenis {
    Sisip,
    Hapus,
    Lain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rekaman {
    text: String,
    selection: Selection,
}

// ---------------------------------------------------------------------------
// TextEdit
// ---------------------------------------------------------------------------

/// Berapa banyak langkah undo yang disimpan sebelum yang tertua dibuang.
const KAPASITAS_UNDO: usize = 128;

/// Keadaan sebuah kolom teks yang bisa disunting.
///
/// Semua operasi bekerja dalam **indeks byte yang selalu berada di batas
/// grapheme**; tidak ada satu pun jalan masuk yang bisa meninggalkan caret di
/// tengah karakter.
#[derive(Debug, Clone)]
pub struct TextEdit {
    text: String,
    selection: Selection,
    preedit: Option<Preedit>,
    multiline: bool,
    undo: Vec<Rekaman>,
    redo: Vec<Rekaman>,
    terakhir: Jenis,
}

impl Default for TextEdit {
    fn default() -> Self {
        Self::new("")
    }
}

impl TextEdit {
    /// Kolom berisi `text`, caret di akhir.
    pub fn new(text: impl Into<String>) -> Self {
        let text: String = text.into();
        let akhir = text.len();
        Self {
            text,
            selection: Selection::caret(akhir),
            preedit: None,
            multiline: false,
            undo: Vec::new(),
            redo: Vec::new(),
            terakhir: Jenis::Lain,
        }
    }

    /// Izinkan baris baru (fondasi `text_area`). Bawaannya satu baris: newline
    /// yang tertempel dari clipboard dibuang, bukan diam-diam merusak layout.
    pub fn multiline(mut self, multiline: bool) -> Self {
        self.multiline = multiline;
        self
    }

    /// Benar bila baris baru diizinkan.
    pub fn is_multiline(&self) -> bool {
        self.multiline
    }

    /// Teks **tersimpan** — tanpa preedit yang sedang dikomposisi.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Seleksi saat ini terhadap [`TextEdit::text`].
    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// Komposisi IME yang sedang berjalan.
    pub fn preedit(&self) -> Option<&Preedit> {
        self.preedit.as_ref()
    }

    /// Benar bila IME sedang mengomposisi.
    pub fn is_composing(&self) -> bool {
        self.preedit.is_some()
    }

    /// Teks yang **terlihat**: tersimpan + preedit yang disisipkan di caret.
    ///
    /// Inilah yang dishape dan digambar; [`TextEdit::text`] yang dilaporkan ke
    /// aplikasi (REKOMENDASI §3.8: preedit dirender inline, tapi ia belum jadi
    /// isi kolom).
    pub fn display_text(&self) -> Cow<'_, str> {
        match &self.preedit {
            None => Cow::Borrowed(&self.text),
            Some(p) => {
                let mut s = String::with_capacity(self.text.len() + p.text.len());
                s.push_str(&self.text[..p.at]);
                s.push_str(&p.text);
                s.push_str(&self.text[p.at..]);
                Cow::Owned(s)
            }
        }
    }

    /// Rentang preedit di dalam [`TextEdit::display_text`] — yang digarisbawahi.
    pub fn preedit_range(&self) -> Option<Range<usize>> {
        self.preedit.as_ref().map(|p| p.at..p.at + p.text.len())
    }

    /// Seleksi dalam koordinat [`TextEdit::display_text`].
    ///
    /// Selama komposisi, caret mengikuti kursor yang **ditentukan IME** di
    /// dalam preedit — bukan ujung preedit — karena itulah yang dilihat
    /// pengguna saat memilih kandidat.
    pub fn display_selection(&self) -> Selection {
        let Some(p) = &self.preedit else {
            return self.selection;
        };
        match p.cursor {
            Some((mulai, akhir)) => Selection::new(
                p.at + mulai.min(p.text.len()),
                p.at + akhir.min(p.text.len()),
            ),
            None => Selection::caret(p.at + p.text.len()),
        }
    }

    /// Ganti seluruh isi (aplikasi yang menyetel nilai, dikte suara).
    ///
    /// Bukan langkah undo pengguna: seleksi dijepit ke isi baru dan komposisi
    /// yang sedang berjalan dibuang.
    pub fn set_text(&mut self, text: impl Into<String>) -> bool {
        let text: String = text.into();
        if text == self.text {
            return false;
        }
        self.preedit = None;
        self.text = text;
        self.selection = Selection::caret(self.text.len());
        self.terakhir = Jenis::Lain;
        true
    }

    /// Setel seleksi (dijepit ke batas grapheme).
    pub fn set_selection(&mut self, selection: Selection) -> bool {
        let baru = Selection {
            anchor: snap_grapheme(&self.text, selection.anchor.min(self.text.len())),
            focus: snap_grapheme(&self.text, selection.focus.min(self.text.len())),
        };
        self.terakhir = Jenis::Lain;
        if baru == self.selection {
            return false;
        }
        self.selection = baru;
        true
    }

    /// Taruh caret di `at`, atau perluas seleksi ke sana bila `extend`.
    pub fn place_caret(&mut self, at: usize, extend: bool) -> bool {
        let at = snap_grapheme(&self.text, at.min(self.text.len()));
        let baru = if extend {
            Selection::new(self.selection.anchor, at)
        } else {
            Selection::caret(at)
        };
        self.set_selection(baru)
    }

    /// Seleksi seluruh isi (⌘A).
    pub fn select_all(&mut self) -> bool {
        self.set_selection(Selection::new(0, self.text.len()))
    }

    /// Seleksi kata yang memuat `at` — **klik ganda**.
    pub fn select_word_at(&mut self, at: usize) -> bool {
        let r = word_range(&self.text, at.min(self.text.len()));
        self.set_selection(Selection::new(r.start, r.end))
    }

    /// Pindahkan caret; `extend` = Shift ditahan.
    pub fn move_caret(&mut self, movement: Movement, extend: bool) -> bool {
        let t = &self.text;
        let fokus = self.selection.focus;
        // Tanpa Shift, seleksi yang ada **runtuh ke ujungnya** dulu — kebiasaan
        // AppKit: ← setelah menyeleksi kata menaruh caret di awal kata, bukan
        // satu huruf sebelum caret.
        if !extend && !self.selection.is_collapsed() {
            match movement {
                Movement::Prev => {
                    return self.set_selection(Selection::caret(self.selection.start()))
                }
                Movement::Next => {
                    return self.set_selection(Selection::caret(self.selection.end()))
                }
                _ => {}
            }
        }
        let tujuan = match movement {
            Movement::Prev => prev_grapheme(t, fokus),
            Movement::Next => next_grapheme(t, fokus),
            Movement::PrevWord => prev_word(t, fokus),
            Movement::NextWord => next_word(t, fokus),
            Movement::LineStart => baris_awal(t, fokus),
            Movement::LineEnd => baris_akhir(t, fokus),
        };
        self.place_caret(tujuan, extend)
    }

    /// Sisipkan teks, mengganti seleksi bila ada.
    ///
    /// Karakter kendali dibuang (dan newline juga, kecuali
    /// [`TextEdit::multiline`]): teks yang ditempel dari mana pun tidak boleh
    /// bisa merusak layout satu baris.
    pub fn insert(&mut self, teks: &str) -> bool {
        let bersih = self.saring(teks);
        if bersih.is_empty() && self.selection.is_collapsed() {
            return false;
        }
        self.preedit = None;
        self.rekam(Jenis::Sisip);
        let r = self.selection.range();
        self.text.replace_range(r.clone(), &bersih);
        self.selection = Selection::caret(r.start + bersih.len());
        true
    }

    /// Hapus ke belakang (Backspace) — satu grapheme, atau seleksi bila ada.
    pub fn delete_backward(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return self.hapus_seleksi();
        }
        let fokus = self.selection.focus;
        if fokus == 0 {
            return false;
        }
        let awal = prev_grapheme(&self.text, fokus);
        self.hapus_rentang(awal..fokus)
    }

    /// Hapus ke depan (Delete/fn+Backspace).
    pub fn delete_forward(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return self.hapus_seleksi();
        }
        let fokus = self.selection.focus;
        if fokus >= self.text.len() {
            return false;
        }
        let akhir = next_grapheme(&self.text, fokus);
        self.hapus_rentang(fokus..akhir)
    }

    /// Hapus satu kata ke belakang (⌥Backspace).
    pub fn delete_word_backward(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return self.hapus_seleksi();
        }
        let fokus = self.selection.focus;
        if fokus == 0 {
            return false;
        }
        let awal = prev_word(&self.text, fokus);
        self.hapus_rentang(awal..fokus)
    }

    /// Hapus satu kata ke depan (⌥Delete).
    pub fn delete_word_forward(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return self.hapus_seleksi();
        }
        let fokus = self.selection.focus;
        if fokus >= self.text.len() {
            return false;
        }
        let akhir = next_word(&self.text, fokus);
        self.hapus_rentang(fokus..akhir)
    }

    // -- IME ----------------------------------------------------------------

    /// Mulai/perbarui komposisi IME.
    ///
    /// Preedit kosong berarti komposisi dibersihkan (winit mengirimnya begitu).
    /// Komposisi pertama **mengganti seleksi** yang ada, persis seperti
    /// mengetik.
    pub fn set_preedit(&mut self, teks: &str, cursor: Option<(usize, usize)>) -> bool {
        if teks.is_empty() {
            return self.clear_preedit();
        }
        if self.preedit.is_none() && !self.selection.is_collapsed() {
            self.hapus_seleksi();
        }
        let at = self.selection.start();
        let cursor = cursor.map(|(a, b)| (a.min(teks.len()), b.min(teks.len())));
        let baru = Preedit {
            text: teks.to_string(),
            cursor,
            at,
        };
        if self.preedit.as_ref() == Some(&baru) {
            return false;
        }
        self.preedit = Some(baru);
        self.selection = Selection::caret(at);
        true
    }

    /// Buang komposisi yang sedang berjalan tanpa menyisipkan apa pun.
    pub fn clear_preedit(&mut self) -> bool {
        self.preedit.take().is_some()
    }

    /// Commit teks final dari IME.
    pub fn commit(&mut self, teks: &str) -> bool {
        let ada = self.preedit.take().is_some();
        // Commit selalu jadi langkah undo sendiri: satu kandidat CJK yang
        // dipilih adalah satu keputusan pengguna.
        self.terakhir = Jenis::Lain;
        self.insert(teks) || ada
    }

    // -- undo/redo ----------------------------------------------------------

    /// Benar bila ada yang bisa di-undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Benar bila ada yang bisa di-redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Kembalikan satu langkah (⌘Z).
    pub fn undo(&mut self) -> bool {
        let Some(r) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.pulihkan(r);
        true
    }

    /// Ulangi langkah yang di-undo (⇧⌘Z).
    pub fn redo(&mut self) -> bool {
        let Some(r) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.pulihkan(r);
        true
    }

    // -- internal -----------------------------------------------------------

    fn snapshot(&self) -> Rekaman {
        Rekaman {
            text: self.text.clone(),
            selection: self.selection,
        }
    }

    fn pulihkan(&mut self, r: Rekaman) {
        self.preedit = None;
        self.text = r.text;
        self.selection = r.selection.snapped(&self.text);
        // Langkah berikutnya selalu memulai kelompok baru: mengetik setelah
        // undo tidak boleh menempel ke kelompok yang barusan dipulihkan.
        self.terakhir = Jenis::Lain;
    }

    /// Catat keadaan sebelum sebuah suntingan.
    ///
    /// Suntingan sejenis yang beruntun **digabung**: mengetik satu kata lalu
    /// menekan ⌘Z mengembalikan seluruh kata, bukan satu huruf — perilaku yang
    /// diharapkan di macOS.
    fn rekam(&mut self, jenis: Jenis) {
        self.redo.clear();
        if self.terakhir == jenis && jenis != Jenis::Lain && !self.undo.is_empty() {
            return;
        }
        self.undo.push(self.snapshot());
        if self.undo.len() > KAPASITAS_UNDO {
            self.undo.remove(0);
        }
        self.terakhir = jenis;
    }

    fn hapus_seleksi(&mut self) -> bool {
        let r = self.selection.range();
        self.hapus_rentang(r)
    }

    fn hapus_rentang(&mut self, r: Range<usize>) -> bool {
        if r.is_empty() {
            return false;
        }
        self.preedit = None;
        self.rekam(Jenis::Hapus);
        self.text.replace_range(r.clone(), "");
        self.selection = Selection::caret(r.start);
        true
    }

    /// Buang karakter yang tidak boleh masuk kolom ini.
    fn saring(&self, teks: &str) -> String {
        teks.chars()
            .filter_map(|c| match c {
                '\r' | '\n' if self.multiline => Some('\n'),
                c if c.is_control() => None,
                c => Some(c),
            })
            .collect()
    }
}

/// Awal baris yang memuat `index`.
fn baris_awal(text: &str, index: usize) -> usize {
    text[..index.min(text.len())]
        .rfind('\n')
        .map_or(0, |i| i + 1)
}

/// Akhir baris yang memuat `index`.
fn baris_akhir(text: &str, index: usize) -> usize {
    let mulai = index.min(text.len());
    text[mulai..].find('\n').map_or(text.len(), |i| mulai + i)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "é" sebagai e + combining acute: dua char, **satu** grapheme.
    const AKSEN: &str = "cafe\u{301}";
    /// Keluarga emoji ZWJ: satu grapheme, 25 byte.
    const KELUARGA: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";

    #[test]
    fn gerakan_caret_per_grapheme_bukan_per_char() {
        // Dari akhir "café", satu langkah ke kiri melewati e+acute sekaligus.
        assert_eq!(prev_grapheme(AKSEN, AKSEN.len()), 3);
        assert_eq!(next_grapheme(AKSEN, 3), AKSEN.len());

        // Emoji ZWJ tidak boleh terbelah jadi tiga.
        assert_eq!(next_grapheme(KELUARGA, 0), KELUARGA.len());
        assert_eq!(prev_grapheme(KELUARGA, KELUARGA.len()), 0);
    }

    #[test]
    fn indeks_di_tengah_karakter_dijepit_ke_batas() {
        // 1 byte di tengah emoji: bukan batas yang sah.
        assert_eq!(snap_grapheme(KELUARGA, 2), 0);
        assert_eq!(snap_grapheme(AKSEN, 5), 3);
        assert_eq!(snap_grapheme("abc", 99), 3);
    }

    #[test]
    fn batas_kata_mengikuti_uax29() {
        let t = "satu dua tiga";
        assert_eq!(next_word(t, 0), 4);
        assert_eq!(next_word(t, 4), 8);
        assert_eq!(next_word(t, 13), 13);
        assert_eq!(prev_word(t, 13), 9);
        assert_eq!(prev_word(t, 0), 0);
        assert_eq!(word_range(t, 5), 5..8);
        // Klik ganda pada spasi menyeleksi spasinya.
        assert_eq!(word_range(t, 4), 4..5);
    }

    #[test]
    fn mengetik_menyisipkan_di_caret_dan_mengganti_seleksi() {
        let mut e = TextEdit::new("halo");
        e.move_caret(Movement::LineEnd, false);
        assert!(e.insert(" dunia"));
        assert_eq!(e.text(), "halo dunia");
        assert_eq!(e.selection(), Selection::caret(10));

        e.set_selection(Selection::new(0, 4));
        e.insert("hai");
        assert_eq!(e.text(), "hai dunia");
        assert_eq!(e.selection(), Selection::caret(3));
    }

    #[test]
    fn newline_dibuang_di_kolom_satu_baris() {
        let mut e = TextEdit::new("");
        e.insert("dua\nbaris\ttab");
        assert_eq!(e.text(), "duabaristab");

        let mut m = TextEdit::new("").multiline(true);
        m.insert("dua\r\nbaris");
        assert_eq!(m.text(), "dua\n\nbaris");
    }

    #[test]
    fn backspace_menghapus_satu_grapheme_utuh() {
        let mut e = TextEdit::new(KELUARGA);
        assert!(e.delete_backward());
        assert_eq!(e.text(), "");

        let mut a = TextEdit::new(AKSEN);
        a.delete_backward();
        assert_eq!(a.text(), "caf");
    }

    #[test]
    fn hapus_kata_dan_hapus_maju() {
        let mut e = TextEdit::new("satu dua tiga");
        e.delete_word_backward();
        assert_eq!(e.text(), "satu dua ");

        e.set_selection(Selection::caret(0));
        e.delete_forward();
        assert_eq!(e.text(), "atu dua ");
        e.delete_word_forward();
        assert_eq!(e.text(), " dua ");
    }

    #[test]
    fn panah_tanpa_shift_meruntuhkan_seleksi_ke_ujungnya() {
        let mut e = TextEdit::new("satu dua");
        e.set_selection(Selection::new(0, 4));
        e.move_caret(Movement::Prev, false);
        assert_eq!(e.selection(), Selection::caret(0));

        e.set_selection(Selection::new(0, 4));
        e.move_caret(Movement::Next, false);
        assert_eq!(e.selection(), Selection::caret(4));
    }

    #[test]
    fn shift_memperluas_dari_anchor_yang_diam() {
        let mut e = TextEdit::new("satu dua");
        e.set_selection(Selection::caret(4));
        e.move_caret(Movement::PrevWord, true);
        assert_eq!(e.selection(), Selection::new(4, 0));
        // Berbalik arah: anchor tetap, focus menyeberang.
        e.move_caret(Movement::LineEnd, true);
        assert_eq!(e.selection(), Selection::new(4, 8));
        assert!(!e.selection().is_collapsed());
    }

    #[test]
    fn undo_menggabungkan_ketikan_beruntun_jadi_satu_langkah() {
        let mut e = TextEdit::new("");
        for c in ["a", "b", "c"] {
            e.insert(c);
        }
        assert_eq!(e.text(), "abc");
        assert!(e.undo());
        assert_eq!(e.text(), "", "satu kata yang diketik = satu langkah undo");
        assert!(e.redo());
        assert_eq!(e.text(), "abc");
        assert!(!e.redo());
    }

    #[test]
    fn memindahkan_caret_memulai_kelompok_undo_baru() {
        let mut e = TextEdit::new("");
        e.insert("satu");
        e.move_caret(Movement::LineStart, false);
        e.insert("X");
        assert_eq!(e.text(), "Xsatu");
        e.undo();
        assert_eq!(
            e.text(),
            "satu",
            "sisipan setelah pindah caret = langkah lain"
        );
        e.undo();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn hapus_dan_sisip_tidak_pernah_digabung() {
        let mut e = TextEdit::new("abc");
        e.delete_backward();
        e.insert("z");
        assert_eq!(e.text(), "abz");
        e.undo();
        assert_eq!(e.text(), "ab");
        e.undo();
        assert_eq!(e.text(), "abc");
    }

    #[test]
    fn suntingan_baru_membuang_tumpukan_redo() {
        let mut e = TextEdit::new("");
        e.insert("a");
        e.undo();
        assert!(e.can_redo());
        e.insert("b");
        assert!(!e.can_redo());
    }

    #[test]
    fn preedit_terlihat_tapi_belum_tersimpan() {
        let mut e = TextEdit::new("ha");
        e.move_caret(Movement::LineEnd, false);
        e.set_preedit("に", Some((3, 3)));
        assert!(e.is_composing());
        assert_eq!(e.text(), "ha", "isi kolom belum berubah");
        assert_eq!(e.display_text(), "haに");
        assert_eq!(e.preedit_range(), Some(2..5));
        assert_eq!(e.display_selection(), Selection::caret(5));

        // Commit memindahkannya menjadi isi sungguhan.
        e.commit("日");
        assert!(!e.is_composing());
        assert_eq!(e.text(), "ha日");
        assert_eq!(e.display_text(), "ha日");
    }

    #[test]
    fn preedit_pertama_mengganti_seleksi() {
        let mut e = TextEdit::new("halo");
        e.select_all();
        e.set_preedit("か", None);
        assert_eq!(e.text(), "");
        assert_eq!(e.display_text(), "か");
    }

    #[test]
    fn preedit_kosong_membatalkan_komposisi() {
        let mut e = TextEdit::new("x");
        e.set_preedit("か", None);
        assert!(e.set_preedit("", None));
        assert!(!e.is_composing());
        assert_eq!(e.display_text(), "x");
    }

    #[test]
    fn kursor_ime_di_tengah_preedit_dihormati() {
        let mut e = TextEdit::new("");
        e.set_preedit("にほん", Some((3, 6)));
        assert_eq!(e.display_selection(), Selection::new(3, 6));
        assert!(!e.display_selection().is_collapsed());
    }

    #[test]
    fn seleksi_kata_lewat_klik_ganda() {
        let mut e = TextEdit::new("satu dua tiga");
        e.select_word_at(6);
        assert_eq!(e.selection().range(), 5..8);
        e.select_all();
        assert_eq!(e.selection().range(), 0..13);
    }

    #[test]
    fn seleksi_selalu_jatuh_di_batas_grapheme() {
        let mut e = TextEdit::new(KELUARGA);
        e.set_selection(Selection::new(2, 7));
        assert_eq!(e.selection(), Selection::caret(0));
        // …dan menghapusnya tidak pernah panik.
        assert!(!e.delete_backward());
    }

    #[test]
    fn set_text_menjepit_seleksi_dan_membuang_komposisi() {
        let mut e = TextEdit::new("panjang sekali");
        e.select_all();
        e.set_preedit("か", None);
        assert!(e.set_text("x"));
        assert!(!e.is_composing());
        assert_eq!(e.selection(), Selection::caret(1));
    }
}
