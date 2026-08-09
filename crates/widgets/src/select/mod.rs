//! `select()` — komponen Tier 2 (`KOMPONEN.md`): pop-up button macOS / Select
//! shadcn.
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::column;
//! # use silka_theme::{Appearance, Theme};
//! # use silka_widgets::{overlay::overlay_layer, select, Fonts, SelectState};
//! # let rt = Runtime::new();
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! let state = rt.signal(SelectState::with_selected(0));
//!
//! let mata_uang = select(&fonts, &t, ["IDR", "USD", "EUR"])
//!     .label("Mata uang")
//!     .bind(state);
//!
//! // Pemicunya berdiri di dalam konten, popupnya di layer overlay.
//! let _ = overlay_layer(column([mata_uang.trigger()]))
//!     .overlay(mata_uang.popup());
//! ```
//!
//! ## Kenapa dua bagian, bukan satu view
//!
//! Popup **tidak boleh** hidup di tempat pemicunya berdiri: ia harus menimpa
//! konten lain dan boleh melampaui kotak induknya. Infrastruktur untuk itu
//! sudah ada dan dibangun sekali untuk sepuluh komponen ([`crate::overlay`],
//! `KOMPONEN.md` aturan #3), dan bentuknya adalah layer di akar halaman. Karena
//! belum ada mekanisme "portal" yang bisa menitipkan panel dari kedalaman pohon
//! ke layer itu, select menyerahkan dua potong yang dipasang di dua tempat:
//! [`Select::trigger`] di dalam konten dan [`Select::popup`] di layer. Begitu
//! portal ada, satu-satunya yang berubah adalah berkas ini — bukan aplikasi
//! yang memakainya, karena keduanya lahir dari builder yang sama.
//!
//! ## Siapa memegang keadaan
//!
//! Seluruh keadaan ada di [`SelectState`] milik aplikasi, dan node render hanya
//! **melapor niat** ([`SelectIntent`]). [`Select::bind`] menyambungkan keduanya
//! ke satu [`Signal`] sehingga pemakaian normal cukup satu baris; aplikasi yang
//! ingin mengendalikan sendiri (validasi, undo, sinkron ke server) memakai
//! [`Select::state`] + [`Select::on_intent`] dan tidak kehilangan apa pun.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Syarat | Di mana |
//! |---|---|
//! | Benar di kedua preset | Seluruh nilai lewat [`SelectTriggerStyle`]/[`SelectOptionStyle`] yang diisi token |
//! | State interaktif lewat spring | Latar, cincin fokus, dan segitiga penunjuk ([`trigger`]); latar baris ([`option`]) |
//! | Keyboard penuh + focus ring | Space/Enter/panah/Home/End/Esc + typeahead, semuanya di pemicu yang memegang fokus |
//! | Node AccessKit | Pemicu = `Button` + nilai + `Expand`/`Collapse`; baris = `MenuItem` + `toggled` |
//! | Dark mode | Token; tidak ada satu pun angka warna di berkas ini |
//! | Hit target ≥ 44pt | `min_height` pemicu **dan** setiap baris |
//! | Reduced-motion | Semua spring lewat [`Tick`](silka_core::animation::Tick) yang membawa [`Motion`](silka_core::animation::Motion) |
//!
//! ## Yang sengaja belum ada
//!
//! - **Kotak pencarian di dalam popup** (`KOMPONEN.md`: "search/filter
//!   opsional") menunggu `text_field`. Yang sudah ada dan menutup kebutuhan
//!   yang sama untuk daftar sedang: **typeahead** — mengetik huruf melompat ke
//!   pilihan yang cocok, persis menu native.
//! - **Pilihan bertingkat/grup** dan pilihan yang dimatikan satu-satu.

mod option;
mod state;
#[cfg(test)]
mod tests;
mod trigger;

use std::rc::Rc;

use silka_core::access::AccessRole;
use silka_core::animation::Spring;
use silka_core::input::FocusPolicy;
use silka_core::signals::{Key, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign};
use silka_core::view::{column, constrained, pad, viewport, Builder, View};
use silka_paint::Insets;
use silka_text::{FontWeight, TextConstraints, TextStyle};
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::overlay::{overlay, Align, Anchor, Barrier, Dismiss, OverlayBuilder, Placement, Side};
use crate::text::text;

pub use option::{SelectOption, SelectOptionProps, SelectOptionStyle};
pub use state::{SelectIntent, SelectState};
pub use trigger::{bar_width, cari_awalan, SelectTrigger, SelectTriggerProps, SelectTriggerStyle};

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// Ke mana sebuah [`SelectIntent`] dikirim.
///
/// Bentuknya sama dengan [`Callback`](silka_core::Callback) — `Clone` murah,
/// kesamaan berdasarkan identitas — hanya saja ia membawa satu argumen, yang
/// belum ada padanannya di inti.
#[derive(Clone)]
pub struct SelectHandler(Rc<dyn Fn(SelectIntent)>);

impl SelectHandler {
    /// Bungkus sebuah closure.
    pub fn new(f: impl Fn(SelectIntent) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Kirim satu niat.
    pub fn emit(&self, intent: SelectIntent) {
        (self.0)(intent)
    }
}

impl PartialEq for SelectHandler {
    /// Identitas, bukan isi: dua `Rc` yang sama = handler yang sama.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for SelectHandler {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SelectHandler")
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder select bergaya Dart (§2.5).
///
/// Menyimpan bahan mentahnya dan baru **meresolusi token** saat menjadi view,
/// sehingga method yang dipanggil belakangan tetap mengubah seluruh hasilnya.
/// `Clone` murah: [`Select::trigger`] dan [`Select::popup`] memakai builder yang
/// sama, jadi keduanya mustahil melenceng satu sama lain.
#[derive(Clone)]
pub struct Select {
    fonts: Fonts,
    theme: Theme,
    options: Rc<Vec<String>>,
    label: Option<String>,
    placeholder: String,
    state: SelectState,
    disabled: bool,
    width: Option<f32>,
    max_visible: usize,
    spring: Spring,
    focus: FocusPolicy,
    bound: Option<Signal<SelectState>>,
    on_intent: Option<SelectHandler>,
    on_select: Option<Rc<dyn Fn(usize)>>,
    key: Option<Key>,
}

/// Pilihan tunggal dari sebuah daftar — komponen `select` (`KOMPONEN.md`).
///
/// `fonts` adalah mesin teks aplikasi, `theme` sumber seluruh nilainya.
pub fn select<S: Into<String>>(
    fonts: &Fonts,
    theme: &Theme,
    options: impl IntoIterator<Item = S>,
) -> Select {
    Select {
        fonts: fonts.clone(),
        theme: *theme,
        options: Rc::new(options.into_iter().map(Into::into).collect()),
        label: None,
        placeholder: String::from("Pilih…"),
        state: SelectState::new(),
        disabled: false,
        width: None,
        max_visible: 8,
        // `snappy` adalah rasa kontrol macOS: cepat sampai, nyaris tanpa
        // pantulan (WWDC23).
        spring: Spring::snappy(),
        focus: FocusPolicy::FOCUSABLE,
        bound: None,
        on_intent: None,
        on_select: None,
        key: None,
    }
}

impl Select {
    /// Nama yang dibacakan screen reader (dan judul popup).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Teks saat belum ada pilihan.
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Keadaan yang berlaku, dikendalikan aplikasi sepenuhnya.
    pub fn state(mut self, state: SelectState) -> Self {
        self.state = state;
        self
    }

    /// Sambungkan ke satu signal: membacanya **dan** menulisinya.
    ///
    /// Inilah bentuk yang dipakai 95% aplikasi — satu titipan state, dan seluruh
    /// aturan (sorotan yang dijepit, gulir yang mengikuti, popup yang menutup
    /// setelah memilih) sudah benar karena semuanya lewat
    /// [`SelectState::apply`].
    pub fn bind(mut self, state: Signal<SelectState>) -> Self {
        // Dibaca **saat build**, jadi komponen yang memanggilnya berlangganan:
        // memilih sesuatu membangun ulang persis komponen itu (§2.5).
        self.state = state.get();
        self.bound = Some(state);
        self
    }

    /// Indeks yang terpilih.
    pub fn selected(mut self, selected: Option<usize>) -> Self {
        self.state.selected = selected;
        self
    }

    /// Popup terbuka atau tidak.
    pub fn open(mut self, open: bool) -> Self {
        self.state.open = open;
        self
    }

    /// Jangkar popup pada koordinat lokal layer overlay.
    ///
    /// Biasanya tidak perlu diisi tangan: [`SelectIntent::Open`] membawa kotak
    /// pemicunya dan [`SelectState::apply`] yang menyimpannya.
    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.state.anchor = anchor;
        self
    }

    /// Matikan kontrol (tetap dibacakan screen reader sebagai dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Lebar kotak, poin logis. Tanpa ini lebarnya diukur dari pilihan
    /// terpanjang — kebiasaan NSPopUpButton, dan yang mencegah kontrol berubah
    /// lebar setiap kali pilihannya berganti.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width.max(0.0));
        self
    }

    /// Berapa baris yang terlihat sebelum popup mulai bisa digulir.
    pub fn max_visible(mut self, rows: usize) -> Self {
        self.max_visible = rows.max(1);
        self
    }

    /// Spring yang menjalankan transisi state (`smooth`/`snappy`/`bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Bisa menerima fokus keyboard atau tidak.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focus.focusable = focusable;
        self
    }

    /// Urutan tab eksplisit (mendahului urutan pohon).
    pub fn tab_order(mut self, order: i32) -> Self {
        self.focus.focusable = true;
        self.focus.order = Some(order);
        self
    }

    /// Terima setiap niat pengguna mentah-mentah — jalur untuk aplikasi yang
    /// mengurus keadaannya sendiri.
    pub fn on_intent(mut self, f: impl Fn(SelectIntent) + 'static) -> Self {
        self.on_intent = Some(SelectHandler::new(f));
        self
    }

    /// Dipanggil setiap kali pengguna memilih sebuah baris.
    pub fn on_select(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    /// Kunci identitas di antara saudara-saudaranya (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    // -- pembacaan (dipakai gallery, uji, dan kode di bawah) -----------------

    /// Daftar pilihan.
    pub fn options(&self) -> &[String] {
        &self.options
    }

    /// Keadaan yang sedang berlaku.
    pub fn state_value(&self) -> SelectState {
        self.state
    }

    /// Teks pilihan sekarang, bila ada.
    pub fn selected_label(&self) -> Option<&str> {
        self.state
            .selected
            .and_then(|i| self.options.get(i))
            .map(String::as_str)
    }

    /// Teks yang tampil di pemicu: pilihan sekarang, atau placeholder.
    pub fn display_text(&self) -> &str {
        self.selected_label().unwrap_or(&self.placeholder)
    }

    /// Tinggi satu baris popup — sekaligus hit target minimum (HIG).
    pub fn row_height(&self) -> f32 {
        MIN_HIT_TARGET
    }

    /// Berapa baris yang benar-benar terlihat di popup.
    pub fn visible_rows(&self) -> usize {
        self.options.len().clamp(1, self.max_visible.max(1))
    }

    /// Benar bila daftarnya lebih panjang dari jendelanya.
    pub fn is_scrollable(&self) -> bool {
        self.options.len() > self.max_visible
    }

    /// Lebar kotak yang berlaku, poin logis.
    ///
    /// Sengaja **tidak** lewat [`Select::trigger_style`]: gaya itu sendiri
    /// memuat `min_width`, dan menanyakannya dari sini akan menjadi lingkaran.
    pub fn width_value(&self) -> f32 {
        self.width.unwrap_or_else(|| {
            self.content_width() + self.padding().horizontal() + self.gap() + self.indicator()
        })
    }

    /// Jarak isi ke tepi kotak.
    fn padding(&self) -> Insets {
        Insets::symmetric(self.theme.space(3.0), self.theme.space(1.5))
    }

    /// Jarak antara label dan segitiga penunjuk.
    fn gap(&self) -> f32 {
        self.theme.space(2.0)
    }

    /// Lebar segitiga penunjuk.
    fn indicator(&self) -> f32 {
        self.theme.space(2.0)
    }

    /// Lebar teks terpanjang (placeholder ikut dihitung), poin logis.
    ///
    /// Diukur lewat mesin teks yang sama yang nanti menggambarnya, jadi tidak
    /// ada tebakan lebar huruf di mana pun (§3.3, §3.4).
    pub fn content_width(&self) -> f32 {
        let gaya = self.text_style();
        self.fonts.with(|m| {
            let mut w = m
                .measure(&self.placeholder, &gaya, TextConstraints::UNBOUNDED)
                .content_size
                .width;
            for o in self.options.iter() {
                w = w.max(
                    m.measure(o, &gaya, TextConstraints::UNBOUNDED)
                        .content_size
                        .width,
                );
            }
            w.ceil()
        })
    }

    fn text_style(&self) -> TextStyle {
        TextStyle::new()
            .size(self.theme.typography.body_size)
            .weight(FontWeight::MEDIUM)
            .single_line()
    }

    /// Nilai gambar pemicu — dipakai gallery dan uji token.
    pub fn trigger_style(&self) -> SelectTriggerStyle {
        let t = &self.theme;
        SelectTriggerStyle {
            rest: t.color.surface,
            hover: t.color.surface_hover,
            pressed: t.color.surface_pressed,
            // Kontrol yang mati **meredup ke arah latar halaman** — aturan yang
            // sama yang dipakai macOS, dan nilainya tetap turunan token.
            disabled: t.color.surface.lerp(t.color.background, 0.6),
            corners: t.corners(t.radius.md),
            border_width: t.space(0.25),
            border: t.color.border,
            border_disabled: t.color.separator,
            shadows: t.shadow.sm,
            focus_ring_width: t.space(0.5),
            focus_ring: t.color.focus_ring,
            padding: Insets::symmetric(t.space(3.0), t.space(1.5)),
            gap: t.space(2.0),
            indicator: t.space(2.0),
            indicator_color: if self.disabled {
                t.color.disabled_label
            } else {
                t.color.secondary_label
            },
            min_width: self.width_value(),
            min_height: MIN_HIT_TARGET,
        }
    }

    /// Nilai gambar satu baris popup.
    pub fn option_style(&self) -> SelectOptionStyle {
        let t = &self.theme;
        SelectOptionStyle {
            // Baris yang diam tidak menggambar apa pun: yang terlihat adalah
            // permukaan panel di belakangnya.
            rest: t.color.surface_hover.with_alpha(0.0),
            highlight: t.color.surface_hover,
            selected: t.color.accent_muted,
            corners: t.corners(t.radius.sm),
            padding: Insets::symmetric(t.space(2.0), t.space(1.0)),
            marker: t.color.accent,
            marker_size: t.space(1.5),
            min_height: MIN_HIT_TARGET,
        }
    }

    /// Handler yang menerjemahkan niat menjadi keadaan baru.
    fn handler(&self) -> SelectHandler {
        let count = self.options.len();
        let visible = self.visible_rows();
        let bound = self.bound;
        let luar = self.on_intent.clone();
        let dipilih = self.on_select.clone();
        SelectHandler::new(move |intent| {
            if let Some(sig) = bound {
                // `peek`, bukan `get`: handler berjalan di luar build, dan
                // berlangganan dari dalam event handler tidak pernah benar.
                let mut baru = sig.peek();
                if baru.apply(intent, count, visible) {
                    sig.set(baru);
                }
            }
            if let Some(h) = &luar {
                h.emit(intent);
            }
            if let SelectIntent::Commit(i) = intent {
                if let Some(f) = &dipilih {
                    if count > 0 {
                        f(i.min(count - 1));
                    }
                }
            }
        })
    }

    // -- dua potong yang dipasang di dua tempat ------------------------------

    /// Kotak pemicu — dipasang di dalam konten halaman.
    pub fn trigger(&self) -> View {
        let t = &self.theme;
        let warna = if self.disabled {
            t.color.disabled_label
        } else if self.state.selected.is_some() {
            t.color.label
        } else {
            // Placeholder lebih redup dari isi sungguhan.
            t.color.tertiary_label
        };
        let isi = text(&self.fonts, self.display_text())
            .size(t.typography.body_size)
            .weight(FontWeight::MEDIUM)
            .color(warna)
            .single_line()
            // Nama kontrol dibacakan sekali, dari node select — bukan dua kali.
            .role(AccessRole::Container);

        let mut b = Builder::new(SelectTriggerProps {
            style: self.trigger_style(),
            label: self.label.clone(),
            value: self.selected_label().map(str::to_string),
            options: self.options.clone(),
            open: self.state.open,
            highlight: self.state.highlight,
            disabled: self.disabled,
            focus: self.focus,
            spring: self.spring,
            on_intent: Some(self.handler()),
        })
        .child(isi);
        if let Some(key) = &self.key {
            b = b.key(key.clone());
        }
        b.into()
    }

    /// Panel pilihan — dipasang di [`crate::overlay::overlay_layer`].
    ///
    /// Penempatannya diserahkan sepenuhnya ke sistem overlay: menempel di bawah
    /// pemicu, rata awal baris, dan **membalik ke atas sendiri** saat mepet tepi
    /// bawah layar. Tidak ada satu pun koordinat yang dihitung di berkas ini
    /// (`KOMPONEN.md` aturan #3).
    ///
    /// Satu batas yang disadari: pada daftar yang bisa digulir, posisi gulir
    /// **dikendalikan** oleh [`SelectState::first_visible`] supaya sorotan
    /// keyboard selalu terlihat. Guliran roda mouse tetap jalan, tapi rebuild
    /// berikutnya mengembalikannya ke jendela milik sorotan. Menyatukan
    /// keduanya butuh guliran yang bisa dibaca balik dari node — kait yang
    /// belum ada di [`silka_core::tree::Viewport`].
    pub fn popup(&self) -> OverlayBuilder {
        let t = &self.theme;
        let handler = self.handler();
        let gaya_baris = self.option_style();

        let baris: Vec<View> = self
            .options
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let terpilih = self.state.selected == Some(i);
                let disorot = self.state.open && self.state.highlight == i;
                let isi = text(&self.fonts, label)
                    .size(t.typography.body_size)
                    .weight(FontWeight::REGULAR)
                    .color(if terpilih {
                        t.color.accent
                    } else {
                        t.color.label
                    })
                    .single_line()
                    // Nama baris dibacakan dari node barisnya, bukan dua kali.
                    .role(AccessRole::Container);
                Builder::new(SelectOptionProps {
                    style: gaya_baris,
                    index: i,
                    label: Some(label.clone()),
                    selected: terpilih,
                    highlighted: disorot,
                    spring: self.spring,
                    on_intent: Some(handler.clone()),
                })
                // Disiplin kunci di daftar dinamis (§2.5).
                .key(i)
                .child(isi)
                .into()
            })
            .collect();

        let daftar = column(baris).cross(CrossAlign::Stretch);
        let tinggi_baris = self.row_height();
        let isi: View = if self.is_scrollable() {
            let tinggi = tinggi_baris * self.visible_rows() as f32;
            // Gulirnya **turunan sorotan** ([`SelectState::first_visible`]):
            // panah bawah yang melewati baris terakhir yang terlihat menggeser
            // jendelanya satu baris, bukan melompat ke tengah.
            constrained(
                BoxConstraints::new(0.0, f32::INFINITY, tinggi, tinggi),
                viewport(daftar)
                    .scroll(self.state.scroll_offset(tinggi_baris))
                    .line_height(tinggi_baris),
            )
            .into()
        } else {
            daftar.into()
        };

        let panel = pad(Insets::all(t.space(1.0)), isi)
            .background(t.color.surface_elevated)
            .corners(t.corners(t.radius.lg))
            .border(t.space(0.25), t.color.separator)
            .shadow(t.shadow.lg);
        // Lebar panel dikunci ke lebar pemicu: daftar yang "melompat lebar" saat
        // terbuka adalah hal pertama yang membuat sebuah select terasa murah.
        let lebar = self.width_value();
        let panel = constrained(BoxConstraints::new(lebar, lebar, 0.0, f32::INFINITY), panel);

        let tutup = handler.clone();
        let mut b = overlay(panel)
            .open(self.state.open)
            .anchor(self.state.anchor)
            .placement(
                Placement::anchored(Side::Bottom)
                    .align(Align::Start)
                    .gap(t.space(1.0)),
            )
            // Popup, bukan dialog: konten di belakang tetap hidup bagi keyboard
            // dan screen reader, tapi klik di luar menutupnya.
            .barrier(Barrier::Light)
            .dismiss(Dismiss::ALL)
            .no_backdrop()
            .role(AccessRole::Menu)
            .spring(self.spring)
            .on_dismiss(move || tutup.emit(SelectIntent::Close));
        if let Some(label) = &self.label {
            b = b.label(label.clone());
        }
        if let Some(key) = &self.key {
            b = b.key(key.clone());
        }
        b
    }
}

impl core::fmt::Debug for Select {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Select")
            .field("options", &self.options.len())
            .field("label", &self.label)
            .field("state", &self.state)
            .field("disabled", &self.disabled)
            .finish()
    }
}
