//! # silka-widgets
//!
//! Katalog komponen (lihat `KOMPONEN.md`) sekaligus **permukaan API publik**
//! framework. Inilah kontrak yang harus dibekukan lebih awal; internal boleh
//! berubah sesuka hati (REKOMENDASI §4 "Kestabilan").
//!
//! Dua aturan bentuk API yang MENGIKAT:
//!
//! 1. **Gaya Dart** (§2.5) — fungsi konstruktor + method chaining, nesting
//!    identik dengan Flutter; properti opsional pindah ke method chain.
//!    Macro DSL ala `rsx!` ditolak sebagai fondasi.
//! 2. **Styling utility ala Tailwind sebagai method chain** (§2.6) — tanpa
//!    CSS, tanpa parser, tanpa cascade. Nilai selalu resolve lewat token
//!    `silka-theme`, dan utility interaktif (`hover`/`pressed`/`focused`)
//!    bertransisi lewat spring, bukan lompat.
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::{column, View};
//! # use silka_theme::{Appearance, Theme};
//! use silka_widgets::{button, text, Fonts};
//!
//! # let rt = Runtime::new();
//! # let count = rt.signal(0i32);
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! column([
//!     View::from(text(&fonts, format!("Nilai: {}", count.get())).color(t.color.label)),
//!     View::from(button(&fonts, &t, "Tambah").on_press(move || count.set(count.get() + 1))),
//! ])
//! .spacing(t.space(3.0));
//! ```
//!
//! ## Yang sudah ada
//!
//! - [`text`] (Tier 0) — daun teks yang **mengukur dirinya sendiri** lewat
//!   `silka-text` dan menggambar glyph dari atlas; wrap mengikuti lebar yang
//!   turun dari box constraints, dan isinya adalah nama node a11y.
//! - [`button`] (Tier 2) — kontrol lengkap di atas token: varian
//!   primary/secondary/ghost/destructive/link, state hover/press/focus/
//!   disabled/loading yang **seluruhnya bertransisi lewat spring**, focus ring
//!   yang tumbuh, Space/Enter, node AccessKit, dan hit target ≥ 44pt.
//! - [`checkbox`] (Tier 2) — kotak centang **tiga-nilai** (termasuk
//!   indeterminate): goresan centangnya benar-benar *ditarik* lewat spring
//!   ([`check_dots`]), labelnya ikut bisa diklik dan sekaligus menjadi nama
//!   a11y, Space mengaktifkan, dan hit target ≥ 44pt walau kotaknya 16pt.
//! - [`switch`] / [`toggle`] (Tier 2) — sakelar on/off yang **bisa diseret**,
//!   bukan cuma diklik: thumb mengikuti jari 1:1, kecepatan jari diserahkan ke
//!   spring saat dilepas (handoff §3.5), warna lintasan menyeberang tepat di
//!   tengah, Space + panah kiri/kanan, node AccessKit dengan keadaan
//!   on/off, dan hit target ≥ 44pt walau lintasannya 32pt/24pt.
//! - [`slider`] / [`range_slider`] (Tier 2) — penggeser nilai: drag yang
//!   melekat pada jari, klik di track, **snap ke step**, keyboard penuh
//!   (panah/Home/End/PageUp), varian range dua thumb, node AccessKit dengan
//!   nilai + aksi increment/decrement, dan pita sentuh ≥ 44pt di sekeliling
//!   track yang setipis 4pt.
//! - [`tabs`](mod@tabs) (Tier 3) — deretan tab dengan tiga varian
//!   (segmented/underline/enclosed) di atas **satu** mesin: indikator yang
//!   meluncur lewat spring yang bisa di-retarget, satu perhentian Tab untuk
//!   seluruh deretan (panah/Home/End di dalamnya, melewati tab yang mati,
//!   dicerminkan di RTL), cincin fokus yang ikut meluncur, dan node AccessKit
//!   `TabList`/`Tab` lengkap dengan keadaan terpilih.
//! - [`select`] (Tier 2) — pop-up button macOS / Select shadcn: popup yang
//!   **menumpang sistem overlay** (berjangkar ke pemicu, auto-flip di tepi
//!   layar), keyboard penuh di pemicu yang tetap memegang fokus
//!   (Space/Enter/panah/Home/End/Esc) plus **typeahead** ala menu native,
//!   daftar panjang dengan jendela yang mengikuti sorotan, node AccessKit
//!   `Button` bernilai + `Menu`/`MenuItem` bertanda, dan hit target ≥ 44pt di
//!   kotaknya maupun setiap barisnya.
//! - [`scroll_view`](mod@scroll_view) (Tier 1) — wadah bergulir dengan
//!   **rubber band ala macOS**, pantulan yang mewarisi kecepatan ekor inersia
//!   OS (momentum tetap milik OS, INTEGRASI-NATIVE §3), scrollbar overlay yang
//!   melebar saat di-hover dan memudar sendiri saat diam, seret thumb, navigasi
//!   keyboard penuh + focus ring, `scroll_to`/`scroll_into_view`, dan aksi
//!   AccessKit `SCROLL` yang benar-benar bekerja.
//! - [`list`](mod@list) (Tier 1) — daftar **tervirtualisasi**: `item` hanya
//!   dipanggil untuk baris yang benar-benar terlihat, jadi seratus ribu baris
//!   tetap menjadi belasan node. Ia tinggal **di dalam**
//!   [`scroll_view`](mod@scroll_view) — momentum, rubber band, dan scrollbar
//!   tidak ditulis dua kali — dan menambahkan yang memang milik daftar: sticky
//!   header, seleksi yang sorotannya *meluncur* lewat spring, ↑/↓/Page/Home/End
//!   yang menggerakkan seleksi sambil menggulirkan barisnya ke layar, dan node
//!   AccessKit `List`/`ListItem` beserta keadaan terpilihnya.
//! - [`table`](mod@table) (Tier 5) — tabel **tervirtualisasi** yang menumpang
//!   infrastruktur `list` alih-alih menumbuhkan yang kedua
//!   (`KOMPONEN.md` aturan urutan #4): jendela barisnya dihitung
//!   [`ListMetrics`] yang sama, guliran dan rubber band-nya milik
//!   [`scroll_view`](mod@scroll_view), dan jahitan di antara keduanya adalah
//!   [`list::sync_virtual`] yang sama. Yang ditambahkannya adalah yang memang
//!   tidak ada di daftar: sort per kolom, resize dan reorder kolom lewat seret
//!   di header, seleksi jamak berjangkar (⇧ merentang, ⌘ memungut, ⌘A
//!   semuanya) yang disimpan sebagai **rentang** sehingga seratus ribu baris
//!   terpilih tetap satu entri, navigasi keyboard antar **sel** dengan cincin
//!   fokus yang mengelilingi sel aktif, sel kustom (widget apa pun di dalam
//!   sel), sticky header, empty state, dan node AccessKit `Table`/`Row`/`Cell`.
//! - [`text_field`] (Tier 2, **komponen tersulit di seluruh katalog**) — kolom
//!   teks satu baris: caret dan seleksi **per grapheme cluster** (UAX #29),
//!   klik ganda per kata, klik tripel seluruh isi, drag-select, undo/redo yang
//!   menggabungkan ketikan beruntun, guliran horizontal yang menjaga caret
//!   terlihat, dan **preedit IME dirender inline bergaris bawah** — dengan
//!   jalur tombol normal ditahan selama komposisi, sehingga aplikasi tidak
//!   pernah menerima huruf setengah jadi (§3.3, §3.8). Model editingnya hidup
//!   di [`silka_text::edit`], geometrinya di [`silka_text::TextLayout`].
//! - [`advance`] (infrastruktur) — satu detak untuk seluruh pohon: di sinilah
//!   spring setiap widget dimajukan, sekali per frame, dan dari sinilah
//!   jawaban "masih adakah yang bergerak" datang.
//! - [`Fonts`] — pegangan bersama ke mesin teks aplikasi, satu atlas untuk
//!   seluruh aplikasi.
//! - [`dialog`](mod@dialog) / [`alert`] (Tier 4) — modal berbackdrop di atas
//!   [`overlay`](mod@overlay): judul, pesan, dan barisan tombol yang
//!   **urutannya mengikuti konvensi OS** ([`ButtonOrder`]), dengan Return
//!   menjalankan tombol default dan Esc menjalankan aksi batal.
//! - [`overlay`](mod@overlay) (Tier 4, **infrastruktur**) — layer di atas
//!   konten, penempatan berjangkar dengan auto-flip di tepi, backdrop, dismiss
//!   (klik luar/Esc), dan transisi spring yang bisa di-retarget. Dibangun
//!   sekali persis seperti yang diperintahkan `KOMPONEN.md` aturan #3: dialog,
//!   sheet, popover, tooltip, menu, dan toast nanti **menumpang** modul ini —
//!   masing-masing tinggal memilih [`Placement`] dan [`Barrier`], tidak satu
//!   pun boleh menghitung posisinya sendiri.
//!
//! Utang teknis yang disadari dan sengaja tidak disembunyikan: `Fonts` masih
//! diserahkan eksplisit ke tiap konstruktor karena belum ada context ambient
//! untuk titipan tingkat aplikasi, dan "scale-on-press" digambar sebagai
//! kempisnya kotak latar (lapisan paint belum punya perintah transform, §3.2)
//! sehingga label di dalamnya tidak ikut mengecil. Untuk overlay,
//! yang belum ada adalah **fokus otomatis** ke panel yang baru terbuka:
//! [`overlay::topmost`] menyediakan nodenya, tapi belum ada kait "baru saja
//! terbuka" di siklus frame yang memanggilnya.
//!
//! Urutan pengerjaan mengikuti tier di `KOMPONEN.md`: Tier 0 (primitif) dan
//! Tier 1 (layout) sampai benar-benar solid dulu, `text_field` dimulai paling
//! awal di Tier 2 karena memaksa stack text/IME/a11y matang, dan overlay
//! system dibangun sekali untuk dialog/popover/tooltip/menu/toast.
//!
//! **Definition of Done setiap komponen** (KOMPONEN.md): benar di kedua
//! preset, semua state interaktif bertransisi spring, navigasi keyboard penuh
//! plus focus ring, **node AccessKit** (role/name/actions), dark mode, hit
//! target minimal 44pt, dan menghormati reduced-motion.
//!
//! Kode di crate ini **tidak boleh menyentuh tipe wgpu** — hanya perintah
//! gambar `silka-paint` (§3.2, §5 failure mode #7).

#![warn(missing_docs)]

pub mod button;
pub mod checkbox;
pub mod dialog;
pub mod fonts;
pub mod list;
pub mod motion;
pub mod overlay;
pub mod scroll_view;
pub mod select;
pub mod slider;
pub mod switch;
pub mod table;
pub mod tabs;
pub mod text;
pub mod text_field;

pub use button::{
    button, button_variant, Button, ButtonBox, ButtonProps, ButtonState, ButtonStyle,
    ButtonVariant, MIN_HIT_TARGET,
};
pub use checkbox::{
    check_dots, checkbox, checkbox_only, dash_rect, ChangeCallback, CheckState, Checkbox,
    CheckboxNode, CheckboxProps, CheckboxStyle,
};
pub use dialog::{
    action, activate_default, alert, dialog, ActionKind, ButtonOrder, DialogAction, DialogBuilder,
    DialogPanel, DialogPanelProps, DIALOG_WIDTH_STEPS,
};
pub use fonts::Fonts;
pub use list::{
    list, use_list_state, ListBody, ListBuilder, ListMetrics, ListRange, ListRowBox, ListScroll,
    ListState, ListStyle, RowAction, Virtualized,
};
pub use motion::{advance, is_animating, settle};
pub use overlay::{overlay, overlay_layer, Anchor, Barrier, Dismiss, Placement, Side};
pub use scroll_view::{
    scroll_view, ScrollBar, ScrollBuilder, ScrollProps, ScrollView, Scrollbar, ScrollbarStyle,
    Thumb,
};
pub use select::{
    select, Select, SelectHandler, SelectIntent, SelectOption, SelectOptionProps,
    SelectOptionStyle, SelectState, SelectTrigger, SelectTriggerProps, SelectTriggerStyle,
};
pub use slider::{
    range_slider, slider, Slider, SliderBuilder, SliderGeometry, SliderProps, SliderStyle,
};
pub use switch::{
    switch, switch_only, toggle, StateColors, Switch, SwitchCallback, SwitchNode, SwitchProps,
    SwitchStyle,
};
pub use table::{
    col, table, use_table_state, CellAlign, Column, ColumnLayout, ColumnWidth, HeaderStyle,
    Selection, SelectionMode, SortBy, SortDirection, TableBody, TableBuilder, TableCellBox,
    TableHeaderBox, TableRowBox, TableState, TableStyle,
};
pub use tabs::{tab, tabs, Tab, Tabs, TabsStyle, TabsVariant};
pub use text::{text, Text, TextBox, TextProps};
pub use text_field::{text_field, TextCallback, TextField, TextFieldBox, TextFieldProps};
