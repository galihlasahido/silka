# rustui

Framework GUI desktop Rust ala Flutter dengan kualitas visual macOS.
Target v1: macOS, Windows, Linux (X11/Wayland).

Semua keputusan arsitektur ada di dokumen rancangan berikut dan **mengikat**:

- [`REKOMENDASI.md`](REKOMENDASI.md) — arsitektur inti: API gaya Dart (§2.5), styling utility ala Tailwind (§2.6), token semantik + dual preset (§2.7), stack teknologi (§3), roadmap (§6).
- [`KOMPONEN.md`](KOMPONEN.md) — katalog komponen per tier + Definition of Done.
- [`INTEGRASI-NATIVE.md`](INTEGRASI-NATIVE.md) — katalog integrasi native & low-level per platform.

## Struktur workspace

| Crate | Isi |
|---|---|
| `crates/paint` | Abstraksi perintah gambar (rect/glyph/shadow/blur) — tanpa tipe wgpu di API publik |
| `crates/renderer` | Backend wgpu yang mengimplementasikan `paint` |
| `crates/text` | Wrapper cosmic-text: shaping, glyph atlas, measure |
| `crates/core` | Signals, view-diff, arena render tree, box constraints, spring, input, scheduler |
| `crates/theme` | Token semantik + preset Cupertino & Tailwind/shadcn |
| `crates/widgets` | Komponen sesuai `KOMPONEN.md` |
| `crates/platform` | Shell winit, escape hatch `raw_handle`, integrasi native |
| `examples/gallery` | Binary demo/gallery (REKOMENDASI §9.9) |

## Build

```sh
cargo check   # seluruh workspace
cargo test    # unit test logika non-visual
```

## Menjalankan gallery

```sh
cargo run -p rustui-gallery
cargo run -p rustui-gallery -- --preset tailwind --appearance dark
```

Tanpa argumen, gallery memakai preset Cupertino dan **mengikuti dark mode OS
secara live**. `--appearance light|dark` mengunci appearance (berguna untuk QA
visual), `--preset cupertino|tailwind` memilih preset.

## Status

Fase 0 baru dimulai. Yang sudah jalan: window winit 0.30 dengan surface wgpu
(Metal di macOS, Vulkan/D3D12/GL di tempat lain), resize + DPI yang benar, dark
mode OS live, dan warna latar yang selalu diresolusi dari token theme.

**Frame scheduling** juga sudah jalan: renderer hanya bekerja saat ada yang
dirty — saat aplikasi diam, tidak ada satu pun frame yang digambar. Detak vsync
datang dari `CADisplayLink` di macOS (ikut ProMotion 120 Hz, dan ikut berubah
saat window pindah monitor) dan dari `request_redraw` winit di OS lain, dengan
interval ditaksir dari frame nyata. **Tidak ada 16,6 ms yang dikonstanta di mana
pun.** Debug build mencetak frame time (cpu, Δ, p50/p95/max, budget) dan
memberi tanda `LAMBAT` pada frame yang melewati budget vsync.

**Runtime signals** (Fase 1) sudah jalan di `crates/core`: `use_signal` untuk
state lokal komponen, pelacakan dependensi per-komponen (membaca saat build =
berlangganan), dirty marking + batching, dan identitas scope berbasis kunci
untuk list dinamis — menukar urutan baris memindahkan scope-nya, bukan
state-nya. Sambungannya ke frame scheduler cuma satu baris, dan janji "render
hanya saat dirty" tetap utuh: signal yang tidak dibaca komponen mana pun sama
sekali tidak membangunkan GPU.

**View-diff → arena render tree** (Fase 1) juga sudah jalan di `crates/core`:
render tree retained berbasis arena ber-ID bergenerasi, protokol **box
constraints ala Flutter** (constraints turun, ukuran naik, induk yang
menentukan posisi), cache layout, **relayout boundary**, dan mirroring RTL di
dalam mesin layout. Di atasnya, view tree ringan bergaya Dart di-diff ke render
tree: tipe + kunci menentukan identitas, sehingga menukar urutan baris
memindahkan node-nya, bukan state layout-nya. Perubahan di dalam scroll view
berhenti di viewport — bukan membuat seluruh window di-layout ulang. Setiap
node render wajib bisa memancarkan node aksesibilitas (role/name/actions)
dengan `bounds` yang datang langsung dari hasil layout.

**Flexbox & Grid lewat Taffy** (Fase 1) sudah jalan di `crates/core`, dan
**di dalam** protokol box constraints — bukan di sampingnya. `row()`,
`column()`, dan `grid()` ditulis gaya Dart dengan `.spacing()`/`.gap_*()` yang
terkunci ke skala 4pt, plus `expanded()`/`flexible()` sebagai padanan
`Expanded`/`Flexible` Flutter. Ukuran daun masuk lewat **measure function**:
satu fungsi `constraints -> ukuran` yang dipakai sama persis oleh mesin layout
kita dan oleh Taffy — itulah pintu masuk pengukuran teks dari `crates/text`.
Nama `taffy::` tidak pernah keluar dari satu modul; kosakata gayanya milik
kita sendiri, sehingga mesin layout bisa diganti tanpa menyentuh satu widget
pun. Mirroring RTL diteruskan apa adanya ke Taffy.

**Emisi node AccessKit** (Fase 1) sudah jalan di `crates/core` sebagai **pass
render tree**, sejajar layout — bukan lapisan susulan. `RenderNode::access`
adalah method **wajib**: widget yang lupa memikirkan screen reader tidak lolos
compile. `bounds` tiap node datang dari hasil layout, bukan dari widget, jadi
apa yang dibacakan teknologi bantu tidak mungkin berbeda dari apa yang
digambar; `AccessTree::dump()` mencetak pohonnya sebagai teks deterministik
untuk golden test. Yang dikirim ke platform hanyalah selisih antar-frame —
janji "hanya saat dirty" berlaku juga untuk screen reader, dan pass-nya sama
sekali tidak dijalankan kalau tidak ada teknologi bantu yang aktif. Nama
`accesskit::` terkurung di satu berkas; adapter winit-nya
(UIA / NSAccessibility / AT-SPI) ada di `crates/platform`, dan window yang
belum menyambungkan render tree-nya pun tetap punya nama yang bisa dibacakan.

Shader SDF dan glyph atlas sudah tersambung ke GPU: perintah `GlyphRun`
menjadi quad bertekstur yang men-sample atlas dari `crates/text` (unggah
inkremental, kotak tujuan disetel ke grid piksel fisik agar tajam di layar 2×)
dan digambar dalam draw call yang sama dengan kotak dan bayangan, sehingga
urutannya terjaga. Jembatannya adalah trait `rustui_paint::GlyphSource` —
renderer tidak pernah menyebut `rustui-text`, dan `rustui-text` tidak pernah
menyebut wgpu. Uji rasterisasi headless (`crates/renderer/tests/teks.rs`)
menghitung piksel teks yang benar-benar keluar dari GPU, lengkap dengan
kontrol negatif "scene tanpa teks = nol piksel".

**Demo ujung-ke-ujung** (`cargo run -p rustui-gallery -- --page counter`) kini
menutup rantainya sampai bisa dilihat mata: halaman counter ditulis **hanya**
dengan API publik — `use_signal`, `column`/`row` gaya Dart, `text()`, dan
`button().on_press(...)` — dan dijalankan lewat jalur resmi `run_app`. Satu klik
menempuh hit-test squircle → `on_press` → tulisan signal → penjadwalan frame →
rebuild **satu komponen saja** → view-diff → layout → paint → glyph atlas →
GPU, dan angka di layar benar-benar berganti.

Dua komponen pertama lahir di `crates/widgets`: `text()` (Tier 0) yang mengukur
dirinya sendiri lewat measure function dan menggambar glyph dari atlas, serta
`button()` (Tier 2) sebagai komposisi `interactive` + `text` di atas token —
lengkap dengan varian, hover/press, focus ring, aktivasi Space/Enter, dan hit
target 44pt. Node interaktif sekarang bisa membawa `on_press` (`Callback`) dan
latar per state, dengan bentuk sudut yang dijamin sama antara yang digambar dan
yang diuji hit-test.

Uji integrasinya permanen dan tidak bisa lulus secara kebetulan: klik
disimulasikan lewat lapisan input pada koordinat yang diambil dari **pohon
aksesibilitas**, lalu scene-nya dirender ke tekstur offscreen dengan pipeline
yang sama dengan window; piksel di pita angka dihitung dan di-hash sebelum dan
sesudah klik. Menonaktifkan tombol atau memutus atlas glyph membuat test ini
merah.

**`text_field`** (`cargo run -p rustui-gallery -- --page kolom-teks`) — komponen
tersulit di seluruh katalog — sudah berdiri di atas stack yang sama. Caret dan
seleksi bergerak **per grapheme cluster** (UAX #29): emoji ZWJ dan huruf
beraksen dilewati sekali tekan, tidak pernah terbelah. Klik ganda menyeleksi
kata, klik tripel seluruh isi, seretan memperluas seleksi walau keluar kolom,
dan undo/redo menggabungkan ketikan beruntun jadi satu langkah ala macOS.
**Preedit IME dirender inline dengan garis bawah**, jalur tombol normal ditahan
selama komposisi, dan isinya baru sampai ke aplikasi setelah IME commit —
`set_ime_cursor_area` mengikuti caret lewat `EventCtx::request_ime`. Model
editingnya (`rustui_text::edit`) murni dan diuji tanpa satu piksel pun;
geometrinya (`TextLayout::hit`/`caret`/`selection_rects`) datang dari hasil
shaping yang sama dengan glyph yang digambar, jadi caret tidak mungkin meleset
dari hurufnya.

`PushClip`/`PopClip` sudah dieksekusi backend sebagai scissor rect per rentang
instance, jadi konten yang terpotong sebagian oleh viewport benar-benar hilang
di GPU — bukan sekadar dibuang saat seluruhnya di luar clip. Uji pikselnya ada
di `crates/renderer/tests/klip.rs`.

Yang belum tersambung dan disadari: `AnimationDriver` ke siklus frame (jadi
transisi state masih lompat, belum spring) dan repaint boundary berbasis layer.
