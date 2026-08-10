# AUDIT LINTAS-FASE

> Catatan internal (bahasa Indonesia, sejalan dengan `catatan/`).
> Tanggal: **10 Agustus 2026** · Basis: working tree `main`, 48 berkas berubah/baru sejak commit `d96c416`.
> Acuan yang mengikat: `catatan/REKOMENDASI.md`, `catatan/KOMPONEN.md`, `catatan/INTEGRASI-NATIVE.md`, `catatan/STATUS.md`.
> **Pembaruan status 11 Agustus 2026 (milestone `utility-adopt`):** P-1 dan P-2 ditandai
> **DITUTUP** di §2, beserta ringkasan bagaimana keduanya ditutup dan berkas buktinya. Temuannya
> sendiri tidak dihapus — uraian aslinya tetap di bawah setiap kotak status, karena riwayat audit
> yang dipotong tidak bisa dipakai memeriksa apakah keputusannya masih masuk akal. Baris yang ikut
> disesuaikan karena faktanya berubah: tabel kontrak §1, baris `div`/`container` di §4, jumlah
> halaman galeri di §6, dan urutan penutupan §7 poin 1 & 4.
>
> **Audit ini tidak mengubah satu baris kode pun.** Tidak ada `cargo check`/`cargo test` yang dijalankan
> (agen lain sedang memakai `target/` yang sama); seluruh temuan berbasis pembacaan sumber, dan setiap
> klaim membawa berkas + baris agar bisa diverifikasi ulang tanpa build.

---

## 1. Ringkasan eksekutif

Kabar baiknya lebih besar daripada kabar buruknya, dan kabar buruknya terpusat di satu tempat.

**Yang benar-benar aman.** Semua batas crate yang dirancang §3.2/§3.3/§3.4 **tidak dilanggar sama
sekali**. Penyebutan `wgpu`, `taffy`, `cosmic-text`, `muda`, `rfd`, `arboard`, `winit` di luar crate
pemiliknya berjumlah **nol dalam kode** — yang ada hanyalah penyebutan di doc-comment yang justru
menjelaskan aturannya. `silka-widgets` bahkan tidak punya `silka-renderer` di `Cargo.toml`-nya, jadi
pelanggaran itu tidak mungkin terjadi secara tidak sengaja.

Emisi node AccessKit lengkap **32 dari 32** `impl RenderNode` di jalur produksi — dan lengkapnya
bukan karena disiplin: `fn access(&self, node: &mut AccessNode);` di `crates/core/src/tree/arena.rs:170`
**tidak punya body default**, jadi widget tanpa node a11y tidak akan compile. Inilah bentuk paling
kuat dari janji §3.8 "AccessKit adalah bagian dari kontrak widget, bukan susulan", dan pantas ditiru
untuk kontrak lain yang hari ini hanya dijaga doc-comment (lihat P-1).

Squircle memang parameter shader (`sdf.wgsl` baris 5–10, 126–137), bukan konstanta.
`unsafe` hanya ada di `crates/platform` (24) dan `crates/renderer` (4) — nol di tujuh crate lain.

**Yang bocor.** *(Diperbarui 11 Agustus 2026: bagian §2.6 dari paragraf ini sudah tidak berlaku —
P-1 ditutup, kosakata utility ada dan sudah dipakai. Yang tersisa dari paragraf ini adalah §2.5,
yaitu P-3.)* Titik lemahnya bukan arsitektur, melainkan **permukaan API publik**: dua dari delapan
keputusan mengikat di REKOMENDASI (§2.5 gaya Dart, §2.6 utility ala Tailwind) baru terwujud sebagian,
dan sisanya justru yang paling mahal diperbaiki nanti karena menyentuh tanda tangan semua konstruktor
(lihat P-1, P-2, P-3). Padahal §4 "Kestabilan" secara eksplisit menyuruh **membekukan kontrak
widget-author lebih awal**. Setiap komponen baru yang ditulis dengan bentuk sekarang menambah biaya
koreksi.

**Yang tertinggal.** Katalog `KOMPONEN.md` Tier 0–4 baru terisi **17 penuh + 2 sebagian dari 46**, dan yang
kosong justru Tier 0–1 (primitif & layout) yang urutan pengerjaannya nomor 1. Empat kekurangan di
kosakata `silka-paint` (stroke, transform, tekstur, layer) memblokir sekitar delapan komponen
sekaligus — menutup keempatnya adalah satu-satunya pekerjaan dengan efek pengganda terbesar di
seluruh daftar ini.

| Kontrak | Status | Catatan |
|---|---|---|
| §3.2 kode widget tidak menyentuh wgpu | ✅ bersih | nol penyebutan di kode; `silka-widgets` tidak mendepend `silka-renderer` |
| §3.3 cosmic-text hanya di `crates/text` | ✅ bersih | |
| §3.4 Taffy hanya di `tree::taffy_box` | ✅ bersih | |
| §3.4 box constraints sebagai protokol native | ✅ | `BoxConstraints`, `MeasuredBox` sebagai satu-satunya pintu ukur teks |
| INTEGRASI §1–2 crate native hanya di `crates/platform` | ✅ bersih | |
| §3.8 setiap widget emit node AccessKit | ✅ 32/32 | dipaksa compiler: method trait tanpa body default |
| §3.6 squircle = parameter shader | ✅ | `CornerStyle::superellipse_exponent()` → uniform shader |
| §2.7 token semantik + dual preset | ✅ | Cupertino & Tailwind, 25 token warna |
| §2.5 API gaya Dart | ⚠️ menyimpang | `&Fonts`/`&Theme` eksplisit di setiap konstruktor (P-3) |
| §2.6 kosakata utility Tailwind | ✅ ditutup | `view::utility` (`div()`, `p_*`, `rounded_*`, `shadow_*`, `bg()`), nilai hanya lewat token, tema ambient dipasang `AppRuntime::frame` (P-1) |
| §2.6 utility interaktif bertransisi spring | ✅ ditutup | `StateStyle` + SpringValue per properti di `tree::Interactive`, dimajukan `RenderTree::advance`, dipakai kartu galeri (P-2) |
| §9.8 RTL arsitektural | ⚠️ separuh | layout termirror, geometri gambar-tangan belum (P-6) |
| §9.5 testing infra | ✅ | 1.679 `#[test]`, golden headless, gerbang frame-time, CI 3 OS |
| §9.6 async/threading | ❌ nol | tidak ada satu pun `spawn`/kanal/jembatan ke UI thread |
| §9.7 strategi panic | ❌ nol | `catch_unwind` hanya di test harness, tidak ada error boundary |
| §9.1 hot reload / DX | ❌ nol | |

---

## 2. Pelanggaran & penyimpangan kontrak

Diurutkan berdasarkan biaya memperbaikinya nanti dibanding sekarang.

### P-1 — Kosakata utility §2.6 belum ada (dampak: tinggi) — **DITUTUP**

> **Status: selesai (milestone `utility-vocab` + `utility-adopt`).** Kosakatanya ada di
> `crates/core/src/view/utility.rs`: `div()`/`container()`, `flex()`/`items_*`/`justify_*`,
> `p_*`/`px_*`/`py_*`/`m_*`, `gap_*`, `rounded_none/sm/md/lg/xl/full`, `border_0/1/2/4`,
> `shadow_none/sm/md/lg/xl`, `bg()`, `text_*`/`font_*`, plus bentuk closure
> `hover(|s| …)`/`pressed(|s| …)`/`focused(|s| …)`/`disabled_style(|s| …)`.
>
> Yang membuatnya lebih dari sekadar sinonim: **jalur normal hanya menerima token**
> (`ColorToken`, `RadiusToken`, `ShadowToken`, `SpaceToken`, `FontToken`), sehingga
> `.bg(Color::hex(0x1E90FF))` **tidak compile** — disiplin §2.6 poin 1 kini dijaga tipe, bukan
> doc-comment. Warna yang benar-benar milik aplikasi keluar lewat pintu darurat yang sengaja
> mencolok di diff (`bg_raw`, `rounded_raw`, `shadow_raw`, `p_raw`).
>
> Token bertemu angka lewat tema ambient (`view::with_theme`) yang dipasang
> `AppRuntime::frame` dari `Signal<Theme>` di `Env` — jadi komponen yang dibangun ulang sendiri
> di tengah pohon meresolusi terhadap tema yang sama dengan akar, bukan `Theme::default`
> (`crates/core/src/app/host.rs`, uji: `app::tests::komponen_yang_dibangun_ulang_sendiri_tetap_dapat_theme`).
>
> Adopsinya nyata, bukan hanya tersedia: `examples/gallery/src/reactive.rs`,
> `primitives.rs`, dan `counter.rs` ditulis ulang memakainya (aritmetika tata letak,
> `Insets::all(t.space(…))`, dan pencarian `t.color.*` hilang seluruhnya), halaman baru
> `examples/gallery/src/utility.rs` memamerkan spacing/radius/shadow/state sebagai rujukan
> hidup, dan `crates/core/src/styling.rs` adalah halaman rustdoc yang mengajarkannya sebagai
> **cara utama** menata tampilan (enam contoh ter-doctest, termasuk satu `compile_fail`).
> Kosakata tipografi menyusul di `silka_widgets::text` (`font(FontToken)`, `text_*`,
> `font_*`, `text_color(ColorToken)`), sehingga trait `view::TextStyled` punya penghuni.
>
> Yang **belum** ditutup dan tetap milik P-3: konstruktor widget masih meminta `&Fonts`/`&Theme`
> eksplisit. Tema ambient menghapus `theme` dari sisi styling, bukan dari tanda tangan
> konstruktor.
>
> Uraian di bawah dipertahankan sebagai catatan sejarah.

`REKOMENDASI §2.6` memberi contoh yang mengikat:

```rust
div().flex().items_center().gap_3().px_4().rounded_lg().bg(theme.surface).shadow_md()
     .hover(|s| s.bg(theme.surface_hover)).child(text("Simpan"))
```

Yang benar-benar ada di `crates/core/src/view/primitives.rs:44–72` hanyalah empat method generik
bernilai token: `background(Color)`, `corners(Corners)`, `border(f32, Color)`, `shadow(ShadowPair)`.
Dari keluarga utility, satu-satunya yang sudah berbentuk kosakata adalah `gap_0`…`gap_12` +
`gap_steps` di `LayoutProps`.

Belum ada: `div()`/`container()`, `p_*`/`px_*`/`py_*`, `rounded_sm/md/lg/full`, `shadow_sm/md/lg`,
`bg()`, `flex()`, `items_center()`, `justify_between()`, dan bentuk closure `hover(|s| …)`.

Konsekuensinya bukan kosmetik. Karena `background()` menerima `Color` mentah dan bukan token
bernama, **disiplin "nilai dikunci ke design token" (§2.6 poin 1) hanya dijaga oleh doc-comment**,
bukan oleh tipe. Tidak ada yang mencegah aplikasi menulis `.background(Color::hex(0x1E90FF))`; hari
ini `crates/widgets` dan `crates/chart` bersih dari itu (nol `Color::hex`/`Color::rgb` di luar
`#[cfg(test)]`), tapi kebersihannya adalah kebiasaan, bukan kontrak yang dipaksakan compiler.

### P-2 — Utility interaktif tidak bertransisi spring (dampak: tinggi) — **DITUTUP**

> **Status: selesai (milestone `utility-spring`).** `tree::Interactive` kini
> menyimpan `SpringValue` untuk tiap properti yang bisa dianimasikan (warna
> latar, warna border + lebarnya, cincin fokus, skala), keadaan ditulis lewat
> bentuk closure `hover(|s| …)` / `pressed(|s| …)` / `focused(|s| …)` /
> `disabled_style(|s| …)` di atas kosakata token, dan semuanya dimajukan oleh
> satu jalur `RenderTree::advance` yang sama untuk seluruh pohon (kontrak baru
> `RenderNode::advance`/`is_animating`/`settle_motion`). Retarget di tengah
> jalan membawa velocity; reduced-motion membuat warna mendarat seketika tanpa
> menjadwalkan frame lagi, cincin fokus tetap tumbuh tanpa pantulan, dan skala
> tidak terjadi sama sekali. Bukti: `crates/core/src/tree/interactive_tests.rs`.
>
> **Diperkuat di milestone `utility-adopt`:** kalimat "setiap `interactive(...)` yang ditulis
> aplikasi (mis. kartu di halaman galeri) melompat" kini punya uji regresi di tempat yang
> persis disebut. Kartu `examples/gallery/src/reactive.rs` memakai
> `hover`/`pressed`/`focused`, dan tiga ujinya menjaga perilakunya:
> `kartu_tidak_melompat_saat_hover_melainkan_bertransisi` (dua frame belum sampai tujuan,
> dan transisinya memang selesai), `pointer_pergi_di_tengah_jalan_berbalik_tanpa_sambungan`,
> serta `reduced_motion_mendarat_seketika_dan_tidak_mengecil`. Halaman
> `examples/gallery/src/utility.rs` menambah satu uji setara untuk tile `hover`.
> Efek sampingan yang menyenangkan: karena `update()` me-retarget spring yang sama,
> pergantian tema kini ikut cross-fade, bukan berkedip.
> Uraian di bawah dipertahankan sebagai catatan sejarah.

`REKOMENDASI §2.6` disiplin #2: *"`hover(...)`/`pressed(...)`/`focused(...)` otomatis bertransisi
lewat spring animation (§3.5), bukan lompat seperti CSS tanpa transition."*

`crates/core/src/tree/interactive.rs:152–171` (`Interactive::dekorasi_aktif`) memilih warna dengan
rantai `if` biasa:

```rust
if self.pressed && self.hovered {
    if let Some(c) = self.press_background.or(self.hover_background) { d.background = c; }
} else if self.hovered {
    if let Some(c) = self.hover_background { d.background = c; }
}
```

Tidak ada `SpringValue`, tidak ada interpolasi — **potongan keras**, persis yang dilarang. Widget
bawaan (button, checkbox, switch, slider, tabs, select) selamat karena masing-masing menyimpan
spring sendiri, tetapi itu justru inti persoalannya: **spring milik widget, bukan milik sistem
utility**. Setiap `interactive(...)` yang ditulis aplikasi (mis. kartu di halaman galeri) melompat.

Ini juga membuat P-1 lebih mahal: menambahkan `hover(|s| …)` belakangan berarti membangun spring per
properti di `Interactive`, yaitu perubahan pada node yang paling banyak dipakai di seluruh pohon.

### P-3 — Konstruktor tidak berbentuk seperti janji §2.5 (dampak: tinggi)

Yang dijanjikan: `text("Hello").size(17.0)`, `button("Save").on_press(Msg::Save)`.
Yang ada: `text(&fonts, "Hello")`, `button(&fonts, &theme, "Tambah")`,
`scroll_view(&theme, child)`, `table(&fonts, &theme, …)`.

Sudah diakui sebagai utang di `crates/widgets/src/lib.rs:126–130`, tapi bobotnya belum tergambar:
ini menyentuh **tanda tangan setiap konstruktor publik di katalog**, dan `REKOMENDASI §4` menyebut
"API churn tanpa akhir" sebagai failure mode kestabilan nomor satu.

Kabar baiknya, mekanismenya sudah ada dan sudah terbukti: `app::Env` (`crates/core/src/app/host.rs:49`)
adalah peta bertipe-kunci yang doc-comment-nya sendiri sudah menyebut `Signal<Theme>` sebagai kasus
pakai utamanya, dan `ScaleFactor` sudah disuntikkan lewat jalur itu. Yang kurang hanyalah keputusan
untuk memindahkan `Theme` + `Fonts` ke sana dan sebuah `BuildCtx` yang bisa membacanya di dalam
konstruktor. **Semakin banyak komponen ditulis, semakin mahal.** Ini kandidat terkuat untuk
dikerjakan sebelum komponen Tier 0–1 yang hilang ditulis, bukan sesudah.

### P-4 — Tier 0/1 dilewati, padahal urutannya nomor 1 (dampak: sedang-tinggi)

`KOMPONEN.md` "Urutan pengerjaan" #1: *"Tier 0 + 1 dulu, sampai benar-benar solid — semua tier di
atasnya adalah komposisi dari sini."* Kenyataannya Tier 2–5 dibangun lebih dulu dan Tier 0 masih
bolong 4 dari 6. Rinciannya di §4 dokumen ini.

Efek yang sudah terasa: karena tidak ada `divider`, `spacer`, dan `card`, halaman-halaman galeri
menyusun garis pemisah dan kartu dari `fixed()`/`pad()` bertoken langsung — pengulangan yang akan
harus dibongkar begitu komponen aslinya lahir.

### P-5 — Empat kekurangan `silka-paint` memblokir ~8 komponen (dampak: sedang-tinggi)

`Command` di `crates/paint/src/scene.rs:132–156` hanya punya lima varian: `Quad`, `Shadow`,
`GlyphRun`, `PushClip`, `PopClip`. Yang tidak ada, dan komponen yang tersandera karenanya:

| Perintah yang belum ada | Yang tersandera | Bukti utang di kode |
|---|---|---|
| **Stroke SDF** | garis chart dirasterisasi jadi kotak per kolom; centang checkbox dirakit dari belasan kotak bulat | `chart/src/stroke.rs:35`, `widgets/src/checkbox.rs:40` |
| **Transform** | "scale-on-press" hanya mengempiskan kotak latar, labelnya tidak ikut mengecil | `widgets/src/button.rs:42–44` |
| **Tekstur/gambar** | `image`, `icon`, `avatar` **tidak mungkin ditulis sama sekali** | — (belum tercatat di mana pun) |
| **Layer/offscreen** | blur material dalam aplikasi → `sidebar` 🪟, repaint boundary sejati | `paint/src/lib.rs:14–15`, `core/src/lib.rs:223` |

Tiga dari empat sudah diakui di doc-comment masing-masing, tapi **tidak ada satu tempat pun** yang
menyatakan bahwa keempatnya adalah simpul yang sama dan memblokir delapan komponen. Menutupnya
adalah pekerjaan dengan rasio manfaat tertinggi di seluruh audit ini.

Catatan positif: `crates/renderer/src/offscreen.rs` sudah punya `OffscreenTarget` yang merender ke
tekstur — jadi setengah dari infrastruktur layer sebenarnya sudah berdiri, hanya belum punya
perintah di kosakata paint yang bisa memanggilnya.

### P-6 — RTL baru separuh jalan (dampak: sedang)

Mirroring layout sudah benar dan diteruskan ke Taffy (`tree/taffy_box.rs:170, 357–360, 455–460`),
dan `RenderTree::direction()` bersifat global. Yang belum: geometri yang digambar tangan oleh widget.
Jumlah penyebutan `TextDirection` per berkas:

- Sudah sadar arah: `slider.rs` (25), `overlay/placement.rs` (25), `table/node.rs` (4),
  `checkbox.rs` (3), `select/*` (2), `tabs/list.rs` (1), `switch.rs` (1), `overlay/entry.rs` (1).
- **Nol**: `text_field.rs` (caret, seleksi, gulir horizontal), `scroll_view/*` (sisi scrollbar, arah
  gulir horizontal), `list/*`, `dialog.rs` (urutan tombol default/cancel), `text.rs`, seluruh
  `crates/chart` (arah sumbu, sisi label).

`REKOMENDASI §9.8` menyebut retrofit RTL "sama mustahilnya dengan retrofit accessibility". Yang
ditakutkan itu justru sedang terjadi di lapisan gambar.

### P-7 — Angka mentah menyelinap lewat `impl Default` (dampak: rendah, tapi ini pintunya)

Jalur normal semua widget benar: `ScrollbarStyle::from_theme(theme)`, `scroll_view(theme, …)`,
`table(fonts, theme, …)` semuanya menurunkan ukuran dari `theme.space(…)` / `theme.radius` /
`theme.typography`. Tetapi `impl Default` untuk struct style yang sama memuat angka mentah:

- `widgets/src/scroll_view/mod.rs:272–277` — `line_height: 40.0`, `thickness: 7.0`,
  `thickness_hover: 12.0`, `margin: 2.0`
  (bandingkan `from_theme`: `theme.space(1.75)`, `theme.space(3.0)`, `theme.space(0.5)`).
- `widgets/src/table/node.rs:150,152` — `indicator_size: 8.0`, `handle_width: 2.0`
  (bandingkan `table/view.rs:373–376`: `theme.space(2.0)`, `theme.space(0.5)`).

Hari ini tidak ada yang memakai jalur `Default` untuk merender, jadi ini bukan cacat visual. Tapi
`Default` yang melewati token adalah cara paling mudah bagi angka hard-code untuk masuk kembali tanpa
ketahuan review. Pilihannya: hapus `Default`, atau ubah agar semuanya `0.0`/`TRANSPARENT` (seperti
yang sudah dilakukan untuk field warna di struct yang sama — konsisten setengah jalan).

Konstanta lain yang sempat dicurigai ternyata **sah** dan tidak perlu jadi token: `MIN_HIT_TARGET
44.0` (HIG), `RUBBER_BAND 0.55` (konstanta fisika macOS), `MAX_FLING 12.0`, `MIN_COLUMN_WIDTH 48.0`,
`MIN_TICK_SPACING 48.0` — semuanya perilaku, bukan tampilan, dan semuanya berdokumen alasan.

### P-8 — Doc-comment basi di `crates/core/src/lib.rs:223–226`

Tertulis: *"What is still missing … wiring `animation::AnimationDriver` into `app::AppRuntime::frame`
(for now springs are still driven by the application through `request_animation_frame`)."*

Sudah tidak benar. `AppRuntime` memiliki `anim: AnimationDriver` (`app/host.rs:345, 402`) dan
`frame()` memanggil `begin_frame`/`end_frame` (`host.rs:607–609`) serta memakainya untuk keputusan
"masih perlu frame lagi?" (`host.rs:725`). Yang benar-benar masih hilang dari kalimat itu hanyalah
repaint boundary berbasis layer.

### P-9 — `crates/testing` satu-satunya crate pustaka tanpa `#![warn(missing_docs)]`

Delapan crate lain memasangnya. `crates/testing/src/lib.rs` tidak. Crate ini adalah alat yang akan
dipakai penulis widget pihak ketiga, jadi justru butuh disiplin dokumentasi yang sama.

### P-10 — Sisa bahasa Indonesia di permukaan publik

Sesuai `STATUS.md` §Bahasa hal ini memang belum diminta diubah, tapi perlu dicatat sebagai daftar
konkret sebelum publikasi crates.io:

- **8 dari 9** `description` di `Cargo.toml` berbahasa Indonesia (hanya `silka-testing` yang Inggris).
  Field ini tampil di crates.io dan docs.rs.
- Pesan `Display` tipe error publik (mis. `titlebar.rs:166–171`: "material tidak didukung",
  "versi OS terlalu lama", "handle window tidak terbaca").
- ~54 pesan `expect()` berbahasa Indonesia yang akan muncul di panic backtrace pengguna.

---

## 3. Utang teknis milestone sebelumnya — status verifikasi

`STATUS.md` poin 4 menyebut daftar utang tanpa rincian. Berikut hasil pengecekannya satu per satu.

| Utang (sumber) | Status hari ini | Bukti |
|---|---|---|
| Eviction LRU atlas glyph | ❌ **masih terbuka, dan lebih tajam dari yang tertulis** | `text/src/cache.rs:283–316` |
| Emoji berwarna belum diuji dengan COLR/CBDT sungguhan | ❌ masih terbuka | jalur `AtlasFormat::Color` ada, tanpa fixture font berwarna |
| Repaint boundary sejati (layer/offscreen untuk blur) | ❌ masih terbuka | lihat P-5 |
| `silka-paint` belum punya stroke (chart) | ❌ masih terbuka | `chart/src/stroke.rs:35` |
| `Fonts` diteruskan eksplisit ke setiap konstruktor | ❌ masih terbuka | lihat P-3 |
| "scale-on-press" tanpa perintah transform | ❌ masih terbuka | `widgets/src/button.rs:42` |
| Overlay tanpa auto-focus saat baru dibuka | ❌ masih terbuka | `widgets/src/lib.rs:131–133`, `select/trigger.rs:8` |
| Clipboard `text_field` (⌘C/⌘X/⌘V) belum tersambung | ❌ masih terbuka | `widgets/src/text_field.rs:55–59` |
| Caret `text_field` tidak berkedip (butuh jalur timer di scheduler) | ❌ masih terbuka | `widgets/src/text_field.rs:60–62` |
| `list` tidak melaporkan `set_size` ke a11y | ❌ masih terbuka | `widgets/src/list/mod.rs:79` |
| Chart memakai `AccessRole::Image` (belum ada peran "chart") | ❌ masih terbuka, disengaja | `chart/src/node.rs:892–908` |
| Sumbu `opsz` Inter belum otomatis per ukuran | ❌ masih terbuka | `text/src/lib.rs:92–94` |
| Rich text + ellipsis otomatis | ❌ masih terbuka | `text/src/lib.rs:95–97` |
| `AnimationDriver` belum tersambung ke `AppRuntime` | ✅ **sudah lunas** (dokumennya yang basi) | lihat P-8 |
| Rename `rustui-*` → `silka-*` | ✅ sudah lunas | nol sisa nama lama |
| Verifikasi menyeluruh pasca-Fase 2 (`STATUS.md` poin 2) | ⚠️ **belum pernah dilakukan** | tidak ada catatan tindak lanjut |

### Satu utang yang perlu dinaikkan kelasnya: atlas glyph

`STATUS.md` mencatatnya sebagai "belum ada eviction LRU". Membaca kodenya, mode kegagalannya lebih
buruk daripada sekadar boros memori:

`GlyphCache::insert` (`crates/text/src/cache.rs:287–316`) menangani atlas penuh dengan `grow()`, dan
`grow()` **membuang seluruh isi atlas** lalu menggandakan ukurannya (`reset_atlas`, baris 329–345).
Setelah menyentuh `UKURAN_MAKS`, `grow()` mengembalikan `None`, `insert` mengembalikan `None`, dan
glyph itu **dilewati diam-diam**.

Artinya: aplikasi berjalan lama yang mengetik CJK di beberapa ukuran font dan berpindah antar monitor
(setiap `ScaleFactor` menghasilkan kunci glyph berbeda) pada akhirnya akan **berhenti menggambar
sebagian huruf, tanpa error, tanpa log**. Keputusan "lebih baik dilewati daripada panic di tengah
frame" (§9.7) sudah benar; yang hilang adalah eviction supaya keadaan itu tidak pernah tercapai.
Ini persis `REKOMENDASI §5` failure mode #1 — "demo bahasa Inggris jalan, mati di CJK".

---

## 4. Katalog `KOMPONEN.md` — apa yang belum ada

Legenda: ✅ ada · 🟡 ada sebagian / tersamar · ❌ belum ada · 🚧 terblokir kekurangan `silka-paint` (P-5)

### Tier 0 — Primitif (1 ✅ + 1 🟡 dari 6)

| Komponen | Status | Catatan |
|---|---|---|
| `div` / `container` | ✅ | `view::div()`/`container()` + kosakata utility di atasnya (P-1 ditutup). `fixed()`/`pad()`/`constrained()` tetap ada sebagai lapisan di bawahnya. |
| `text` | ✅ | `widgets::text` — mengukur sendiri, isi jadi nama node a11y |
| `image` | ❌ 🚧 | butuh perintah tekstur; belum ada async decode |
| `icon` | ❌ 🚧 | butuh atlas SVG + perintah tekstur |
| `spacer` | ❌ | murah: `expanded()` sudah ada sebagai bahan |
| `divider` | ❌ | murah: `AccessRole::Separator` sudah ada di kosakata, tidak dipakai siapa pun |

### Tier 1 — Layout (6 dari 8)

| Komponen | Status | Catatan |
|---|---|---|
| `row` / `column` | ✅ | `core::view::row/column` + `.spacing()`/`.gap_*()` |
| `stack` (z-axis) | ❌ | tidak ada padanan ZStack; overlay tidak menggantikannya |
| `flex` | ✅ | `expanded()`/`flexible()`, wrap, justify/align |
| `grid` | ✅ | `core::view::grid` lewat Taffy CSS Grid |
| `scroll_view` | ✅ | rubber-band, momentum OS, scrollbar overlay |
| `list` (virtualized) | ✅ | sticky header, seleksi bergerak spring |
| `padding`, `constrained_box` | ✅ | `pad()`, `constrained()` |
| `aspect_ratio`, `align`, `center` | ❌ | `align_self`/`cross()` ada di dalam flex, tapi tiga primitif berdiri sendiri ini tidak ada |

### Tier 2 — Kontrol dasar (6 dari 12)

| Ada | Belum ada |
|---|---|
| `button`, `text_field`, `checkbox`, `switch`, `slider` (+`range_slider`), `select` | `icon_button` (🚧 butuh `icon`), `text_area` (fondasinya sudah ada: `TextEdit::multiline`), `radio`/`radio_group` (`AccessRole::RadioButton` sudah ada, tak terpakai), `stepper` (`AccessRole::Stepper` sudah ada, tak terpakai), `combo_box`, `label`+`form` |

### Tier 3 — Navigasi & struktur (3 ✅ + 1 🟡 dari 10)

| Ada | Belum ada |
|---|---|
| `window` (`silka-platform`, termasuk titlebar kustom + vibrancy), `tabs` (3 varian), `menu_bar` native (`platform::menu`, muda) | `sidebar` (🚧 butuh layer/blur), `toolbar` (`AccessRole::Toolbar` ada, tak terpakai), `segmented_control` berdiri sendiri (kini hanya varian `tabs`), `breadcrumb`, `command_palette`, `split_view`/`resizable` |
| | `context_menu` 🟡 — `platform::menu::PopupMenu` ada di jalur native, tapi belum ada widget in-window |

### Tier 4 — Overlay & feedback (1 dari 10, plus infrastruktur `overlay`)

| Ada | Belum ada |
|---|---|
| `overlay` (infrastruktur, sesuai aturan #3 KOMPONEN.md), `dialog`/`alert` | `sheet`, `popover`, `tooltip` umum (yang ada hanya `chart::tooltip` — sudah menumpang `overlay`, tinggal digeneralisasi), `toast`, `progress_bar`/`progress_circle` (`AccessRole::ProgressIndicator` ada, tak terpakai), `skeleton`, `badge`, `hover_card`, `drawer` |

Catatan positif: `overlay` betul-betul dibangun sekali dan `dialog` serta `chart::tooltip` sudah
membuktikan bahwa penumpangnya nol perhitungan posisi. Sembilan komponen sisa di tier ini seharusnya
murah — masing-masing "pilih `Placement` + `Barrier`" sesuai janji `widgets/src/lib.rs:118–124`.

### Tier 5 — Data display (di luar Tier 0–4, dicatat untuk kelengkapan)

Ada: `table` (virtualized, di atas infrastruktur `list` — aturan #4 dipatuhi), `chart` (`silka-chart`).
Belum: `tree`, `card`, `accordion`, `avatar`, `tag`/`chip`, `calendar`/`date_picker`, `color_picker`.

### Ringkasan angka

**Tier 0–4: 18 ✅ + 1 🟡 dari 46 baris katalog** (`div`/`container` naik dari 🟡 ke ✅ setelah P-1) (`overlay` tidak ikut dihitung — ia infrastruktur,
bukan baris katalog).

Yang paling murah sekaligus paling menaikkan kelengkapan: `spacer`, `divider`, `stack`, `center`,
`align`, `aspect_ratio`, `radio`, `stepper`, `badge`, `card`, `progress_bar`, `skeleton`, `tooltip`
umum, `popover`, `sheet`, `toast` — **enam belas komponen yang tidak satu pun butuh perintah paint
baru**, dan sebagian besar hanya perlu memilih `Placement` di atas `overlay` yang sudah ada.

---

## 5. `INTEGRASI-NATIVE.md` — status per bagian

| § | Bagian | Status |
|---|---|---|
| 1 | Window & shell | ✅ sebagian besar — `window.rs`, `titlebar.rs` (custom titlebar, traffic light inset, material/vibrancy), `lifecycle/restore.rs` (restorasi posisi + `MonitorArea`). Belum: window snapping hints |
| 2 | Menubar, tray, dialog | ✅ `menu.rs` (muda, termasuk `standard_edit_menu` yang wajib untuk Cmd+C macOS), `tray.rs`, `dialog.rs` (rfd). ❌ **notifikasi sistem** (`notify-rust` bahkan belum jadi dependensi), ❌ badge count, ❌ dock/taskbar |
| 3 | Input low-level | ✅ IME, velocity tracker, momentum OS. ❌ `global-hotkey`, ❌ media keys (`souvlaki`), ❌ Force Touch/haptic, ❌ pen/stylus |
| 4 | Clipboard & DnD | ✅ `clipboard.rs` (arboard, teks + gambar). ❌ **drop target belum tersambung sama sekali** (nol penyebutan `DroppedFile`/`HoveredFile` di `crates/platform`), ❌ drag source — satu-satunya item P0 tanpa crate siap pakai, dan §"Urutan pengerjaan" #3 menyuruh menjadwalkannya eksplisit |
| 5 | OS services & file system | ❌ **belum ada apa pun** — single instance, deep link, file association, recent files, `open`, `notify`, `trash`, `keyring` (P1, wajar tertunda) |
| 6 | Lifecycle & setelan OS | ✅ paling matang di antara semuanya — `lifecycle/` menangani dark mode live, accent color OS, reduced motion & reduce transparency, restore placement, `SystemSettings`. ❌ launch at login, prevent sleep, locale change |
| 7 | Hardware & media | ❌ nol (P2, memang direncanakan sebagai crate companion terpisah) |
| 8 | **Escape hatch** | ✅ **selesai dan rapi** — `platform::raw`, re-export `objc2`/`objc2-app-kit`/`objc2-foundation`/`objc2-quartz-core` (macOS), `windows-rs` (Windows), `zbus` (Linux), plus hook event native (`event.rs`, `forward_native_events`). §"Urutan pengerjaan" #2 menyuruh ini diputuskan sebelum 1.0 — sudah |
| 9 | Distribusi & operasional | ❌ nol — tidak ada `cargo-packager`/bundling, tidak ada signing/notarization di CI, tidak ada updater, tidak ada crash reporting. §"Urutan pengerjaan" #4 menyuruh signing masuk CI sejak flagship app pertama |

---

## 6. Lubang `REKOMENDASI §9` yang masih menganga

| § | Lubang | Status |
|---|---|---|
| 9.1 | Hot reload / DX | ❌ **nol** — tidak ada hot-patching, tidak ada preview app dengan token live-editable, tidak ada dynamic linking di dev build. Yang ada dan membantu: profil `dev` sudah dituning (`debug = "line-tables-only"`, dependensi tanpa debuginfo) dan galeri bisa dibuka per halaman lewat `--page`. §9.1 menyebut ini "bahaya terbesar" |
| 9.5 | Testing infra | ✅ **ditutup dengan baik** — `crates/testing` (golden headless, `Simulator`, `Bench`/`Budget`, `for_each_case`), 1.679 `#[test]`, CI 3 OS dengan lavapipe di Linux, `SILKA_REQUIRE_GPU=1` supaya runner tanpa driver tidak melaporkan suite visual hijau palsu, gerbang frame-time terpisah di mode release. Yang sengaja belum: sumbu X11 × Wayland (didokumentasikan alasannya di `ci.yml`) |
| 9.6 | Async / threading | ❌ **nol** — nol `spawn`, nol kanal, nol `tokio`, nol jembatan "hasil async kembali ke UI thread" di `crates/core` maupun `crates/platform`. Aplikasi flagship "tool bisnis/finance" (§9.3) melakukan network + database; ini akan jadi penghalang pertama begitu aplikasi itu mulai ditulis |
| 9.7 | Strategi panic | ❌ **nol di jalur produksi** — `catch_unwind` hanya muncul di `crates/testing` dan di unit test. Tidak ada error-boundary per komponen, tidak ada `panic::set_hook`, tidak ada crash recovery/simpan state. Satu-satunya kepatuhan §9.7 yang nyata adalah `GlyphCache::insert` yang memilih melewatkan glyph daripada panic |
| 9.8 | i18n & RTL | ⚠️ separuh — mirroring layout ✅ (P-6), tapi **nol sistem terjemahan**: tidak ada Fluent/gettext, dan string bawaan widget (mis. label tombol dialog) tidak melalui lapisan apa pun. `Locale` hanya ada di `silka-chart` untuk format angka/tanggal — bagus, tapi lokal untuk satu crate |
| 9.9 | Dokumentasi & gallery | ⚠️ galeri ✅ (17 halaman termasuk rujukan kosakata utility, katalog terpusat di `catalog.rs` sehingga sidebar/`--page`/test tidak bisa berbeda pendapat), dokumentasi Fase 4 ❌ belum mulai — tidak ada tutorial "app pertama", tidak ada 3 contoh app, tidak ada scaffold flagship |

---

## 7. Urutan penutupan yang disarankan

Diurutkan berdasarkan "biaya sekarang dibanding biaya nanti", bukan berdasarkan ukuran.

1. **P-3 — sisa terakhir dari bentuk API publik.** ~~P-1~~ dan ~~P-2~~ sudah ditutup (kosakata
   utility bertoken + spring milik sistem, keduanya sudah dipakai galeri dan didokumentasikan di
   `crates/core/src/styling.rs`); yang tersisa dari paket ini adalah memindahkan `Theme` + `Fonts`
   ke `Env` plus `BuildCtx` yang membacanya, supaya konstruktor widget berbentuk seperti janji
   §2.5. Tema ambient sudah membuktikan mekanismenya: nilainya dipasang sekali per frame di
   `AppRuntime::frame` dan tidak satu pun call site menyebut `theme` lagi di sisi styling.

   > Catatan asli, saat ketiganya masih terbuka: ketiganya menyentuh permukaan yang sama dan
   ketiganya makin mahal setiap kali satu komponen ditambahkan. Konkretnya: pindahkan `Theme` dan
   `Fonts` ke `Env` (mekanismenya sudah terbukti lewat `ScaleFactor`), tambahkan `div()` plus
   kosakata utility bernama yang resolve lewat token, dan pasang `SpringValue` di `Interactive`
   supaya `hover`/`pressed` bertransisi. `REKOMENDASI §4` menyuruh membekukan kontrak ini lebih awal
   — sekarang adalah "lebih awal" yang terakhir.
2. **P-5 — empat perintah `silka-paint`.** Satu pekerjaan di satu crate yang membuka `image`, `icon`,
   `avatar`, `sidebar`, blur material, repaint boundary, sekaligus melunasi tiga utang yang sudah
   tercatat (stroke chart, centang checkbox, scale-on-press). `OffscreenTarget` sudah setengah jadi.
3. **Eviction atlas glyph** (§3 dokumen ini). Kecil, terisolasi di satu berkas, dan menutup mode
   kegagalan senyap di jalur CJK — failure mode #1 REKOMENDASI.
4. **Tier 0–1 yang hilang.** `spacer`, `divider`, `stack`, `aspect_ratio`, `align`, `center`
   (`div()` sudah ada sejak P-1 ditutup) — enam komponen yang tidak butuh perintah paint baru sama sekali; `image` dan
   `icon` menyusul otomatis setelah poin 2. Kerjakan setelah bentuk API di poin 1 beku, supaya tidak
   ditulis dua kali. Urutan #1 `KOMPONEN.md` menyuruh tier ini solid sebelum tier di atasnya, dan
   hari ini urutan itu terbalik.
5. **P-6 — RTL di widget yang menggambar sendiri**, terutama `text_field` dan `scroll_view`.
6. **§9.6 async + §9.7 panic.** Keduanya arsitektural dan keduanya akan menghadang flagship app.
7. **INTEGRASI §4 drop target + drag source**, satu-satunya item P0 tanpa crate siap pakai.
8. **P-7 sampai P-10** — bersih-bersih: `Default` yang melewati token, doc-comment basi,
   `missing_docs` di `crates/testing`, dan sisa bahasa Indonesia di permukaan crates.io.
9. **`STATUS.md` poin 2** — verifikasi menyeluruh pasca-Fase 2 belum pernah dilakukan; tujuh
   komponen gagal ronde pertama dan dugaan "tabrakan antar-agen" masih berstatus dugaan.

---

## 8. Metode & batasan

- Basis: pembacaan seluruh `crates/*/src`, `examples/gallery/src`, `Cargo.toml` workspace + per-crate,
  `.github/workflows/ci.yml`, dan keempat dokumen di `catatan/`.
- Pemeriksaan batas crate dilakukan dengan menyisir setiap penyebutan nama crate terlarang di seluruh
  `*.rs` dan `*.toml`, lalu memilah mana yang berada di dalam kode dan mana yang di doc-comment.
- Cakupan AccessKit diperiksa dengan mencocokkan setiap `impl RenderNode for` terhadap keberadaan
  `fn access()` di berkas yang sama: 41 impl total, 9 di antaranya node uji di dalam `#[cfg(test)]`,
  32 di jalur produksi, dan 32 punya `access()`.
- **Tidak dijalankan**: `cargo check`, `cargo test`, `cargo clippy`. Agen lain berjalan paralel di
  `target/` yang sama dan build penuh akan menguncinya. Konsekuensinya: audit ini **tidak menyatakan
  apa pun tentang apakah working tree hari ini compile**. Angka 1.679 adalah jumlah atribut `#[test]`
  di sumber, bukan jumlah test yang lulus (bandingkan 1.503 lulus yang tercatat di `STATUS.md`
  sebelum pekerjaan Fase 3 dimulai).
- Audit ini tidak mengubah kode, tidak menjalankan git, dan tidak menyentuh berkas di luar repositori.
