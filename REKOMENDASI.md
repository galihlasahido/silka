# Rekomendasi: Framework GUI Desktop Rust ala Flutter dengan Kualitas Visual macOS

> Target: macOS, Windows, Linux · Fokus: Performance, Keamanan, Tampilan, Cross-platform, Kestabilan
> Filosofi desain: Apple Human Interface Guidelines — halus, interruptible, penuh perhatian pada detail.
> Status riset: Agustus 2026.

> **Update keputusan (9 Agu 2026):**
> 1. **BSD di-drop dulu** — target v1 adalah macOS/Windows/Linux. Renderer cukup satu tingkat (wgpu); abstraksi paint tetap tipis agar BSD bisa kembali nanti sebagai "tambah backend", bukan tulis ulang.
> 2. **"Model Flutter" = gaya kode ala Dart** (komposisi widget nested di kode), **bukan** meniru arsitektur internal Flutter. Lihat §2.5.
> 3. **Styling utility-first ala Tailwind sebagai method chain** (pola GPUI/Zed) — tanpa CSS, tanpa webview. Lihat §2.6.
> 4. **Dual theme preset**: "Cupertino" (HIG Apple, default) dan "Tailwind/shadcn" — widget ditulis sekali terhadap token semantik, tampilan mengikuti preset aktif. Lihat §2.7.
> 5. **Model state: Signals + rebuild per-komponen** (pola Dioxus 0.7, rasa `setState` Flutter) — perubahan signal menandai komponen dirty → rebuild subtree → diff ke render tree. Menutup §9.4.
> 6. **Flagship app: tool bisnis/finance internal** yang dipakai tim sehari-hari — forcing function untuk table tervirtualisasi, form, chart, dan cerita async (§9.6). Menutup §9.3.
> 7. **Mode kerja: serius dengan dukungan tim/perusahaan** — roadmap 4 fase (12 bulan) dan scope penuh berlaku. Menutup §9.2.
> 8. **Nama proyek: `silka`** (10 Agu 2026). Nama lama `rustui` ditinggalkan karena sudah dipakai (github.com/algoscienceacademy/RustUI).
>    Verifikasi via API crates.io: `silka` bebas di crates.io & docs.rs; satu-satunya tabrakan adalah tipografi Silka (Atipo Foundry, dunia desain — bukan software).
>    Kandidat lain yang juga bersih: `halus`, `anggun`, `selaras`. Yang WAJIB dihindari karena taken/berbahaya: `motif` (crate GUI framework Rust milik desainer Zed + toolkit X11 The Open Group),
>    `lumina` (ada `lumina-core` = framework GUI wgpu+Taffy), `facet` (2.5k★), `cadence` (59jt unduhan + TM Cadence Design Systems), `lumo` (TM Proton AG), `nimbus`, `lucent`, `marble`, `sable`.
>    Rencana crate: `silka-core`, `silka-paint`, `silka-renderer`, `silka-text`, `silka-theme`, `silka-widgets`, `silka-platform`, `silka-chart`.
>    **Penggantian nama dijadwalkan setelah Fase 2 selesai** (sapuan mekanis; tidak boleh saat agen aktif menulis `use rustui_*`).
>    Catatan: belum ada pencarian trademark formal (USPTO/EUIPO/DJKI) — perlu dilakukan bila dikomersialkan.
>
> Katalog lengkap komponen yang harus dibuat: lihat **`KOMPONEN.md`**.
> Katalog integrasi native & low-level (window/menu/clipboard/DnD/lifecycle/hardware/distribusi): lihat **`INTEGRASI-NATIVE.md`**.

---

## 1. Kesimpulan Utama (TL;DR)

1. **Jangan bangun semuanya dari nol.** Flutter dibangun puluhan engineer Google selama ~4 tahun ke 1.0. Zed (GPUI) butuh ~5 tahun dengan tim kecil untuk 3 platform. Bagian tersulit (text shaping, accessibility, GPU rendering) sudah ada crate-nya yang matang — pakai itu.
2. **Bangun framework widget ala Flutter di ATAS ekosistem yang ada** — bukan aplikasi di atas framework orang lain, dan bukan juga engine dari nol. Lapisan yang kita tulis sendiri: model widget (declarative view + retained render tree), sistem animasi spring, design system ala Apple, dan shader UI.
3. **Kualitas visual macOS itu bisa dicapai** — Zed membuktikannya (120 fps di ProMotion). Kuncinya bukan engine vector umum, tapi renderer SDF khusus UI: rounded rect, shadow, glyph atlas, blur.
4. **Estimasi realistis**: 6–12 bulan ke demo yang mengesankan; 2–4 engineer-years ke framework yang bisa dipakai orang lain shipping app.
5. **Musuh terbesar bukan rendering — tapi TEXT** (CJK, Arabic, IME, emoji, font fallback). Ini pembunuh #1 framework GUI baru. Solusi: pakai `cosmic-text` atau `parley`, jangan pernah tulis shaper sendiri.

---

## 2. Model Widget ala Flutter — Bagaimana Menerjemahkannya ke Rust

Flutter punya 3 pohon: `Widget` (immutable, declarative) → `Element` (mutable, identity) → `RenderObject` (layout & paint). Model ini bergantung pada **inheritance + garbage collector** — dua hal yang tidak ada di Rust.

Terjemahan yang terbukti bekerja di Rust:

| Lapisan Flutter | Padanan Rust yang direkomendasikan |
|---|---|
| `Widget` (declarative) | **View tree ala Xilem**: struct ringan, dibangun ulang tiap update, di-diff |
| `Element` (identity/state) | **Arena/slotmap berisi node ber-ID** (framework yang memiliki state, bukan user) |
| `RenderObject` (layout/paint) | **Retained render tree ala Masonry**: trait object + downcast, pass layout/paint/accessibility terpisah |
| Constraint layout | **Box constraints ala Flutter sebagai protokol native** ("constraints turun, ukuran naik"), + **Taffy** untuk widget Flex/Grid |

Kenapa arena + ID, bukan ownership biasa? Karena AccessKit (accessibility) dan Taffy (layout) sama-sama berbasis ID/arena — semuanya jadi selaras, dan kita menghindari perang borrow-checker pada pohon yang saling menunjuk.

### 2.5 Gaya kode ala Dart (API publik) — terpisah dari arsitektur internal

Keputusan: yang ditiru dari Flutter adalah **rasa menulis kodenya** (komposisi widget nested), bukan arsitektur internalnya. Rasa Dart datang dari named/optional parameters + nesting; padanan idiomatis di Rust adalah **fungsi konstruktor + method chaining**:

```dart
// Dart / Flutter
Column(
  spacing: 12,
  children: [
    Text('Hello', style: TextStyle(fontSize: 17)),
    ElevatedButton(onPressed: save, child: Text('Save')),
  ],
)
```

```rust
// API publik framework kita
column((
    text("Hello").size(17.0),
    button("Save").on_press(Msg::Save),
))
.spacing(12.0)
```

- Struktur nesting identik dengan Dart; properti opsional pindah ke method chain. Pola terbukti (Xilem, iced, SwiftUI).
- **Ditolak sebagai fondasi**: macro DSL (`rsx!`-style) — autocomplete/go-to-definition/pesan error jauh lebih buruk; boleh jadi gula sintaks opsional nanti. Struct literal + `..Default::default()` — verbose dan canggung.
- Konsekuensi: trio Widget/Element/RenderObject, `BuildContext`, `setState` dst. TIDAK ditiru. Arsitektur internal (view-diff → arena render tree, §2) tetap dipakai tapi berstatus detail implementasi yang tersembunyi di balik API ini.
- **Model state (DIPUTUSKAN, 9 Agu 2026): Signals + rebuild per-komponen** — pola Dioxus 0.7: `use_signal` untuk state lokal; perubahan signal menandai komponen yang membacanya sebagai dirty → rebuild subtree kecil itu → diff. Paling dekat mental model `setState` Flutter, state lokal komponen natural. Harga yang diterima sadar: scheduler dirty-marking + scope tracking di internal framework, dan disiplin key/identity di list dinamis. Alternatif yang ditolak: message ala Elm (state lokal canggung, rasa bukan Dart), referensi ala Xilem (ergonomi belum terbukti, lens asing bagi pendatang Flutter).

```rust
let count = use_signal(|| 0);

column((
    text(format!("Nilai: {}", count.get())),
    button("Tambah").on_press(move || count.set(count.get() + 1)),
))
.spacing(12.0)
```

### 2.6 Sistem styling: utility-first ala Tailwind, tanpa teknologi web

Keputusan: styling memakai **kosakata utility Tailwind sebagai method chain** (pola GPUI/Zed) — bukan CSS betulan, bukan webview:

```rust
div()
    .flex()
    .items_center()
    .gap_3()
    .px_4()
    .rounded_lg()
    .bg(theme.surface)
    .shadow_md()
    .hover(|s| s.bg(theme.surface_hover))
    .child(text("Simpan"))
```

Kenapa opsi ini:
- **Type-safe**: typo utility = error compile, bukan diam-diam tidak berefek seperti string CSS. Autocomplete IDE penuh.
- **Terbukti produksi**: pola persis yang dipakai Zed dan gpui-component (60+ widget) di 120fps, rendering 100% GPU native.
- **Koheren dengan §2.5**: sama-sama method chaining — satu bahasa API, bukan dua sistem (markup + style) yang harus dijembatani.

Alternatif yang ditolak:
- **String class via macro** (`class!("flex gap-3")`) — terasa lebih "Tailwind asli" tapi kehilangan autocomplete dan menambah kompleksitas macro; ergonomi nyaris sama dengan method chain.
- **Engine CSS sungguhan tanpa browser** (Stylo + Vello, cara Blitz/Dioxus Native) — membuktikan ini mungkin secara teknis, tapi mewarisi seluruh kompleksitas web (cascade, specificity, selector matching) yang justru ingin dihindari; Blitz sendiri masih alpha.

Dua disiplin agar tetap "berjiwa Apple":
1. **Nilai dikunci ke design token** — skala spacing 4pt, palet warna semantik, radius squircle; filosofi constraint-based Tailwind, tapi konstrainnya diisi HIG Apple, bukan default web.
2. **Utility interaktif terhubung ke spring system** — `hover(...)`/`pressed(...)`/`focused(...)` otomatis bertransisi lewat spring animation (§3.5), bukan lompat seperti CSS tanpa transition.

**Klarifikasi penting**: tidak ada CSS Tailwind yang benar-benar dipakai — tidak ada parser, tidak ada cascade. Yang diambil adalah (a) kosakata utility-nya sebagai method Rust, dan (b) **angka-angka design token-nya** (skala spacing 4px, radius, palet warna step 50–950, definisi shadow, skala font). "Tampilan Tailwind" yang orang kenal sebenarnya datang dari token-token itu + component library di atasnya (shadcn/ui, daisyUI) — dan keduanya bisa direplikasi persis secara native. Bukti: gpui-component merender 60+ komponen bergaya shadcn/ui sepenuhnya di GPU tanpa CSS, nyaris tak terbedakan dari versi web. Batasan: fitur CSS kompleks (selector rumit, `@keyframes`, `grid-template-areas`) tidak ikut; rendering text sedikit berbeda dari browser (stack text sendiri).

### 2.7 Dual theme preset: "Cupertino" dan "Tailwind/shadcn"

Keputusan: arsitektur token mendukung **theme preset yang bisa dipilih per aplikasi**, dengan dua preset first-party:

| | Preset **Cupertino** (default) | Preset **Tailwind/shadcn** |
|---|---|---|
| Kiblat | Apple HIG / macOS | shadcn/ui di web |
| Sudut | Squircle (continuous corner) | Rounded rect biasa (`rounded-lg` = 8px) |
| Warna | Palet semantik HIG (label/secondaryLabel, materials) | Palet Tailwind (slate/zinc/blue, step 50–950) |
| Shadow | Ganda ambient + key ala HIG | `shadow-sm/md/lg` ala Tailwind |
| Tipografi | Inter dengan optical size, tracking ala SF | Skala font Tailwind |
| Motion | Spring `smooth`/`snappy`/`bouncy` | Spring juga (bukan CSS ease) — rasa native dipertahankan |

Konsekuensi arsitektur:
- Utility (`bg`, `rounded_lg`, `shadow_md`) **tidak pernah hard-code angka** — selalu resolve lewat token theme aktif. `rounded_lg` di Cupertino = squircle; di Tailwind = arc 8px. Karena itu geometri sudut harus jadi parameter shader, bukan konstanta (konsisten dengan §3.6).
- Widget bawaan (Button, TextField, Sheet, dst.) ditulis sekali terhadap token semantik (`surface`, `accent`, `radius_md`) sehingga otomatis benar di kedua preset.
- Preset ketiga (brand kustom) = tinggal isi token — mengikuti pola theme system gpui-component.
- Spring animation berlaku di semua preset — "rasa native halus" adalah identitas framework, bukan milik satu theme.

Arsitektur alternatif yang dipelajari dan alasan tidak dipilih sebagai fondasi:
- **Elm-style (iced)** — sederhana, tapi rebuild penuh tiap message dan state lokal widget menyakitkan; iced sendiri masih menyebut dirinya "experimental" setelah 7 tahun.
- **Fine-grained signals (Floem/Leptos)** — performa update terbaik, tapi mental model signal-graph dan interior mutability di mana-mana; kurang "rasa Flutter".
- **GPUI (Zed)** — terbukti di skala produksi, tapi API-nya mengikuti kebutuhan Zed, dokumentasi tipis, dan bukan model widget declarative penuh.

---

## 3. Stack Teknologi yang Direkomendasikan

### 3.1 Windowing & Input
- **winit** (standar de-facto) + **raw-window-handle**.
- BSD: winit jalan di FreeBSD/OpenBSD lewat backend X11/Wayland (bukti nyata: Alacritty ada di ports FreeBSD/OpenBSD), tapi tidak di-CI — perlu kita test sendiri.
- **Escape hatch per-platform** untuk polish: `objc2`/`objc2-app-kit` (macOS: vibrancy, traffic lights, display link), `windows-rs` (Win32), langsung ke NSWindow saat winit tidak cukup.

### 3.2 Rendering — satu tingkat: wgpu (BSD di-drop, lihat Update Keputusan)
- **wgpu** (Metal di macOS, Vulkan/D3D12 di Linux/Windows) + **shader SDF khusus UI ala GPUI**.
- Linux tua tanpa Vulkan tertutup gratis oleh backend GL milik wgpu sendiri — tidak perlu tier terpisah.
- **Asuransi murah**: kode widget tidak boleh menyentuh tipe wgpu langsung — lewat abstraksi paint tipis (perintah gambar: rect/glyph/shadow/blur). Kalau BSD (atau CPU fallback) dibutuhkan lagi nanti, itu jadi backend baru di satu tempat, bukan tulis ulang framework. Kandidat saat itu: vello_hybrid (GL) / tiny-skia + softbuffer (CPU).

Pelajaran dari Impeller (Flutter): **jangan pernah generate shader saat runtime** — kompilasi semua varian shader di build time, itu yang membunuh jank. Pelajaran dari Zed: UI itu 95% rounded rect + shadow + glyph dari atlas + ikon monochrome — renderer SDF khusus mencapai 120 fps jauh lebih cepat daripada engine vector umum. Vello bisa di-swap masuk nanti di belakang abstraksi paint.

Alternatif "boring tapi benar": **skia-safe** — model imaging teruji Chrome, ada CPU fallback. Harga: build sangat berat, FFI C++, dan di BSD harus compile Skia sendiri. Simpan sebagai plan B.

### 3.3 Text (lapisan tersulit — jangan diremehkan)
- **Hari ini**: `cosmic-text` (dipakai COSMIC desktop, iced, Floem) — paket lengkap: fontdb + rustybuzz + swash + line breaking + editing.
- **Arah masa depan**: `parley` (Linebender) — model rich-text lebih baik, bergerak ke `harfrust` (penerus resmi HarfBuzz di Rust) + fontations. Masih pre-1.0.
- Yang wajib benar sejak awal: font fallback per platform, bidi (UAX #9), gerakan kursor per grapheme cluster (UAX #29), emoji ZWJ/warna, subpixel *positioning* (bukan subpixel AA — itu sudah mati, macOS pun sudah drop), preedit IME dirender inline.
- Realita: text saja = 1–2 engineer-years kalau ditulis sendiri. **Pakai crate yang ada.**

### 3.4 Layout
- Protokol native: **box constraints ala Flutter** (single pass, relayout boundaries).
- **Taffy** untuk Flexbox/Grid sebagai widget (dipakai Dioxus, Bevy, fork-nya dipakai Zed). Text measurement diintegrasikan lewat measure function di leaf node.

### 3.5 Animasi — jantung "rasa Apple"
Dari WWDC23 "Animate with springs" — filosofi Apple secara teknis:
- **Spring adalah kurva default**, bukan ease-in-out. Parameternya *perceptual duration + bounce*, bukan mass/stiffness/damping.
- **Semua animasi harus interruptible**: nilai animasi menyimpan `(posisi, velocity)` dan bisa di-retarget kapan saja sambil membawa velocity — gesture handoff (fling → spring) butuh velocity tracker di input layer.
- Solusi closed-form damped harmonic oscillator dievaluasi per frame (~200 baris kode, referensi terbaik: `SpringSimulation` Flutter).
- Frame scheduling: render hanya kalau dirty. Vsync per platform: `CADisplayLink` (macOS, cara benar dapat ProMotion 120Hz — jangan hardcode 16.6ms), `WaitForVBlank`/compositor clock (Windows), `wl_surface::frame` (Wayland — jalan juga di BSD), Present extension (X11).
- Preset ala SwiftUI: `smooth` / `snappy` / `bouncy`.
- Jangan lupa setting **reduced-motion** (accessibility).

### 3.6 Design System ala Apple — detail yang membuat "terasa macOS"
- **Squircle (continuous corners)**: sudut Apple bukan busur lingkaran — superellipse blend G2-continuous (~1.528× radius nominal). Implementasikan langsung di SDF shader. Putuskan SEJAK AWAL karena geometri sudut merembet ke hit-testing, border, shadow, clipping.
- **Tipografi**: SF Pro tidak boleh di-ship (lisensi Apple). Pengganti standar: **Inter v4** (open, punya axis `opsz` optical size). Butuh dukungan variable font + fitur OpenType (tabular figures, tracking ketat per ukuran).
- **Materials/translucency**: blur behind-window via `window-vibrancy` (NSVisualEffectView; juga acrylic/mica di Windows). Blur *dalam aplikasi* (sidebar di atas konten sendiri) harus dirender sendiri: dual-Kawase downsample + tint + saturation — artinya render graph butuh dukungan layer/offscreen texture, bukan cuma langsung ke swapchain.
- **Shadow ala HIG**: dua shadow bertumpuk (ambient + key), murah dengan SDF.
- **Micro-interactions**: scale-on-press, focus ring — murah begitu spring + SDF ada.
- **Dark mode**: observer appearance per platform (winit punya theme event).

### 3.7 Integrasi Platform
- Menu native: **muda** · Tray: **tray-icon** · Dialog file: **rfd** (XDG portal → jalan di BSD juga) · Clipboard: **arboard** · Notifikasi: **notify-rust**.
- Custom titlebar macOS: `titlebarAppearsTransparent` + `fullSizeContentView` + reposisi traffic lights (objc2).
- Wayland: siapkan waktu untuk **client-side decorations** (sudut + shadow harus digambar sendiri).
- Gap yang harus ditulis per-platform: **drag-and-drop sebagai source** (winit hanya kasih drop target).

### 3.8 Accessibility & IME — dari HARI PERTAMA
- **AccessKit** (UIA/NSAccessibility/AT-SPI) — sudah diadopsi egui, iced, Xilem, Floem, Slint. Setiap widget harus bisa emit node AccessKit (role, name, bounds, actions). **Retrofit accessibility adalah failure mode klasik** — bangun sebagai output first-class dari render tree.
- IME: winit menyediakan `Ime::Preedit`/`Commit` + `set_ime_cursor_area`. Widget text harus render preedit inline dengan underline dan menahan key event normal selama komposisi. Testing CJK di 4 OS = mingguan-bulanan, bukan harian.

---

## 4. Fokus Anda, Dijawab Satu per Satu

| Fokus | Jawaban |
|---|---|
| **Performance** | Rust + wgpu + SDF renderer + dirty-region tracking → target 120 fps idle-cheap. Pelajaran Impeller: shader precompiled = frame time prediktabel. |
| **Keamanan** | Rust memory-safe by default. Minimalkan `unsafe` (terkonsentrasi di FFI platform & GPU). Dependency audit dengan `cargo-audit`/`cargo-deny`. Tanpa runtime JS/webview = permukaan serangan jauh lebih kecil daripada Electron/Tauri. |
| **Tampilan** | Squircle + spring + materials + tipografi optical-size + shadow ganda = resep "rasa Apple". Semua achievable, sudah dibuktikan Zed. |
| **Cross-platform** | winit + wgpu menutup mac/Win/Linux penuh (X11 + Wayland). BSD ditunda; pintu tetap terbuka lewat abstraksi paint tipis (§3.2). |
| **Kestabilan** | Failure mode klasik Rust GUI adalah **API churn selamanya** (iced 7 tahun "experimental", Xilem masih alpha). Kunci: bekukan kontrak widget-author lebih awal, ubah internal sesukanya. |

---

## 5. Failure Modes yang Harus Dihindari (dari riset framework yang gagal/stagnan)

1. **Meremehkan text** — demo bahasa Inggris jalan, mati di CJK/Arabic/IME/emoji. Pembunuh #1.
2. **Accessibility di-retrofit** — hampir tidak pernah berhasil setelah model widget beku.
3. **Ekor 90/10 platform polish** — menu, dialog, DnD, CSD Wayland, multi-monitor DPI, dark mode: masing-masing kecil, totalnya bertahun-tahun.
4. **API churn tanpa akhir** — pilih arsitektur, bekukan kontrak publik.
5. **Perfeksionisme renderer** — engine vector umum menunda shipping bertahun-tahun; SDF primitives dulu, Vello belakangan.
6. **Tidak punya flagship app** — framework tanpa aplikasi first-party yang menuntut (Zed→GPUI, Lapce→Floem, COSMIC→iced) akan melayang tanpa arah. **Bangun framework + satu aplikasi nyata bersamaan.**
7. **Renderer hard-wired ke satu API grafis** — meski BSD ditunda, kode widget yang menyentuh tipe wgpu langsung membuat backend baru (GL/CPU/BSD) mustahil ditambah nanti tanpa tulis ulang. Jaga abstraksi paint tetap tipis sejak awal.

---

## 6. Roadmap yang Disarankan

**Fase 0 — Fondasi (bulan 1–2)**
winit + wgpu jalan di mac/Linux/Windows; shader SDF: rounded rect (termasuk squircle), border, shadow ganda; glyph atlas + cosmic-text; frame scheduling + display link per platform.

**Fase 1 — Core framework (bulan 3–6)**
Render tree arena + box constraints; view layer declarative + diffing; sistem spring animation (posisi+velocity, retargetable); input + hit-testing + velocity tracker; Taffy sebagai widget Flex; AccessKit node emission sejak sekarang.

**Fase 2 — Design system (bulan 6–9)**
Widget set ala Apple: Button, TextField (dengan IME!), List/ScrollView dengan rubber-band + momentum, Sidebar dengan material blur, Sheet/Popover dengan spring transition; dark mode; Inter + optical sizing; arsitektur token semantik + dua preset first-party (Cupertino & Tailwind/shadcn, §2.7).

**Fase 3 — Platform tail (bulan 8–11, lebih pendek karena BSD di-drop)**
muda/rfd/arboard/tray; custom titlebar macOS; CSD Wayland; pendalaman IME/CJK di 3 OS; **satu aplikasi flagship nyata** sebagai pembuktian. (Pekerjaan tier GL/CPU + CI BSD dihapus dari scope v1.)

---

## 7. Perbandingan Framework yang Sudah Ada (data terverifikasi, Agustus 2026)

### 7.1 Tabel ringkas

| Framework | Versi | Model | Rendering | Aksesibilitas | IME | BSD | Lisensi | Aplikasi produksi |
|---|---|---|---|---|---|---|---|---|
| **egui** | 0.36 (Agu 2026) | Immediate mode | glow (GL) / wgpu; text stack baru Skrifa+vello_cpu | ✅ AccessKit (default) | ✅ Baru dibenahi 0.35/0.36 | ✅ Terbaik di kelas native (jalur GL; banyak app di FreeBSD ports) | MIT/Apache | Rerun |
| **iced** | 0.14 (Des 2025) | Elm architecture | wgpu + fallback CPU tiny-skia | ❌ Tanpa AccessKit (issue terbuka sejak 2020) | ✅ Baru masuk 0.14 | ✅ Halloy & Sniffnet ada di FreeBSD ports | MIT | COSMIC DE, Halloy, Sniffnet, Kraken Desktop |
| **Slint** | 1.17 (Jul 2026) | DSL retained + property binding | FemtoVG / Skia / CPU renderer | ✅ Terbaik — "Narrator works perfectly" | ✅ | ⚠️ Compile jalan, tidak resmi; CPU renderer = escape hatch | ⚠️ GPLv3 / royalty-free / komersial | HMI embedded/industri |
| **Tauri** | 2.11 (Jul 2026) | Webview OS + Rust backend | WKWebView/WebView2/webkit2gtk | ✅ Warisan browser (terbaik) | ✅ | ✅ Satu-satunya dengan cfg resmi FreeBSD/OpenBSD/NetBSD/DragonFly | MIT/Apache | Spacedrive, Yaak, Clash Verge, Hoppscotch |
| **GPUI** | pre-1.0 di crates.io | Entity retained + element tree per frame | Custom: Metal / blade-Vulkan / DX11 (bukan wgpu) | ❌ Tidak ada — screen reader buta total | ✅ | ⚠️ Zed build di FreeBSD dgn patch (ada docs resmi + paket); OpenBSD tidak ada | Apache (ada isu dep GPL #55470) | Zed 1.0 (Apr 2026), Longbridge Pro |
| **Dioxus** | 0.7 (Okt 2025) | React-like (RSX + signals) | Webview (matang) / Blitz+Vello (alpha) | ✅ webview; ❌ Blitz belum | ✅ webview | ⚠️ Jalur webview ikut wry; Blitz untested | MIT/Apache | Tools internal; belum ada flagship native |
| **Xilem/Masonry** | 0.4, alpha eksplisit | View-diff → retained tree (paling mirip Flutter) | Vello (+ varian CPU/Hybrid) | ✅ AccessKit sejak awal (masih ada bug posisi) | ✅ Sebagian besar jalan | ⚠️ Untested | Apache | Belum ada |
| **Makepad** | 1.0 (Mei 2025) | DSL live-design, styling via shader | Custom GPU (Metal/DX11/GL), tanpa CPU fallback | ❌ Nol total | ⚠️ Parsial | ❌ Tidak ada cerita BSD | MIT/Apache | Robrix, Moly |
| **Floem** | 0.2 | Fine-grained signals | vger/Vello/wgpu + tiny-skia CPU | ❌ Tidak jalan | ❌ Tidak aktif (survey 2025) | ⚠️ Untested | MIT | Lapce |
| **Freya** | 0.4 (Jul 2026, pivot lepas dari Dioxus) | Component retained, builder API (paling "Flutter-ish") | Skia + layout engine sendiri (Torin) | ⚠️ AccessKit parsial | ⚠️ Jalan, UI komposisi tersembunyi | ❌ Unsupported | MIT | Belum ada |

### 7.2 Temuan lintas-framework yang penting untuk keputusan kita

**Soal polish setara macOS** — hanya dua jalur yang terbukti mencapainya hari ini:
1. **GPUI** — desain Metal-first 120fps, text lewat CoreText/DirectWrite dengan subpixel positioning; Zed adalah buktinya. Inilah validasi terkuat bahwa pendekatan "renderer custom khusus UI" (yang kita rekomendasikan di §3.2) benar.
2. **Webview (Tauri/Dioxus-desktop)** — dapat polish browser gratis (subpixel text, backdrop-filter), tapi mengorbankan konsistensi antar OS (webkit2gtk di Linux jauh lebih buruk) dan bukan "native Rust rendering" yang Anda inginkan.

Slint/Skia peringkat berikutnya untuk kualitas text. egui/iced/Xilem/Floem semuanya grayscale-AA via wgpu tanpa efek native.

**Soal aksesibilitas** — temuan paling mengejutkan: dari semua framework native, screen reader hanya benar-benar berfungsi di **Slint**. GPUI, iced upstream, Floem, dan Makepad semuanya buta total bagi screen reader. Ini memperkuat pelajaran §5 poin 2: bahkan framework terbaik (GPUI) gagal karena accessibility di-retrofit. Kita harus lebih baik dari ini.

**Soal BSD** — peringkat praktis: **Tauri/wry** (satu-satunya dengan target cfg BSD resmi) ≥ **iced/egui** (app nyata sudah ada di FreeBSD ports) > **GPUI** (FreeBSD jalan dengan patch, OpenBSD tidak) > **Slint** > sisanya untested. Pola yang terlihat: framework yang jalan di BSD adalah yang punya jalur GL atau CPU — memvalidasi strategi renderer 3 tingkat kita.

**Soal kestabilan** — hanya **Slint** (1.x disiplin rilis) dan **Tauri** (2.x, governance Commons Conservancy) yang benar-benar stabil. Semua framework native murni lainnya pre-1.0 dengan API churn. GPUI baru saja masuk crates.io dan punya isu lisensi (dep GPL transitif via `sum_tree`→`ztracing`) yang harus diawasi kalau mau dipakai untuk app proprietary.

**Soal arsitektur mirip Flutter** — yang paling dekat secara konsep: **Xilem/Masonry** (view-diff → retained tree, persis pola yang kita rekomendasikan) dan **Freya 0.4** (component model + builder API, di atas Skia). Keduanya belum matang — artinya ceruk "Flutter-quality widget framework di Rust" masih benar-benar kosong. Tidak ada satu pun framework yang sekaligus: model widget declarative + polish macOS + accessibility + BSD. Itulah celah yang proyek ini bisa isi.

### 7.3 Komponen yang bisa kita curi pelajarannya (atau pakai langsung)

- **gpui-component** (Longbridge, Apache-2.0, 12.5k ⭐) — 60+ widget di atas GPUI termasuk table tervirtualisasi, chart, code editor; bukti bahwa lapisan widget bisa dibangun komunitas di atas renderer bagus.
- **Blitz** (DioxusLabs) — engine HTML/CSS modular (Stylo + Taffy + Parley + Vello) yang bisa di-embed; infrastruktur baru paling penting untuk dipantau.
- **libcosmic** (System76) — contoh nyata membangun design system + DE lengkap di atas iced.
- **window-vibrancy** — blur/vibrancy native untuk window winit apa pun (macOS + Windows acrylic/mica).

---

## 8. Rekomendasi Akhir

> **Bangun "Flutter-nya Rust dengan jiwa Apple" sebagai LAPISAN, bukan sebagai DUNIA.**
>
> - Ambil dari ekosistem: winit, wgpu, cosmic-text/parley, Taffy, AccessKit, muda/rfd/arboard.
> - Tulis sendiri (di sinilah nilai uniknya): API publik bergaya Dart (§2.5) + styling utility ala Tailwind (§2.6) di atas view-diff → arena render tree, renderer SDF dengan squircle & materials, sistem spring animation interruptible ala SwiftUI, dan design system dengan disiplin HIG.
> - Bangun **bersama satu aplikasi flagship** sungguhan.
> - Accessibility bukan afterthought — masuk desain sejak hari pertama. (BSD ditunda dari scope v1; abstraksi paint tipis menjaga pintunya tetap terbuka.)

---

## 9. Lubang yang Harus Ditutup (analisis kegagalan, 9 Agu 2026)

Hasil sisir ulang rancangan dengan kacamata "apa yang membunuh proyek seperti ini". Diurutkan dari yang paling mematikan. **#1–4 harus dijawab sebelum baris kode pertama; #5–9 masuk rancangan dan roadmap.**

> **Status (9 Agu 2026)**: §9.2 ✅ (mode kerja: serius dengan tim/perusahaan), §9.3 ✅ (flagship: tool bisnis/finance internal), §9.4 ✅ (model state: signals — lihat §2.5). Tersisa terbuka: §9.1 (strategi hot reload/DX) dan §9.5–9.9 (masuk roadmap).

### 9.1 Developer experience / hot reload — bahaya terbesar
Dengan memilih "gaya Dart di Rust" kita kehilangan senjata utama Flutter: hot reload sub-detik. Iterasi UI di Rust = edit → compile → run (10–60 detik per perubahan padding). Ini membunuh produktivitas kita sendiri saat membangun design system — polish ala Apple lahir dari ribuan iterasi kecil.
**Mitigasi masuk rancangan**: hot-patching ala Dioxus 0.7 (subsecond, terbukti bisa di Rust), preview app dengan token live-editable, dynamic linking di dev build, disiplin compile time (waspadai ledakan generics view-type — pengalaman pahit iced/Xilem).

### 9.2 Keberlanjutan: siapa dan dengan uang apa
Estimasi 2–4 engineer-years. Semua framework yang bertahan punya patron: Zed=perusahaan, Slint=SixtyFPS GmbH, egui=Rerun, iced=System76, Floem=Lapce. Framework tanpa penyandang dana mati di tahun kedua saat maintainer burnout. **Keputusan non-teknis paling penting yang belum ada jawabannya.**

### 9.3 Flagship app belum dipilih
Sudah ditetapkan *harus ada* (failure mode #6), belum dipilih *apa*. Tanpa aplikasi konkret tidak ada forcing function yang menentukan komponen mana yang benar-benar penting. Kriteria: aplikasi yang kita sendiri butuhkan dan pakai setiap hari.

### 9.4 Model state — keputusan API besar terakhir yang menggantung
Bentuk API semua komponen di KOMPONEN.md tergantung ini: message ala Elm vs signals vs referensi state ala Xilem. Harus jadi keputusan pertama Fase 1 — mengubahnya setelah 20 komponen jadi = tulis ulang semuanya.

### 9.5 Strategi testing — saat ini nol
Klaim "kestabilan" butuh: golden/snapshot test visual (cara Flutter menjaga jutaan widget), rendering headless untuk CI, simulasi input, matrix CI 3 OS × X11/Wayland, dan **benchmark frame-time dengan regression gate** — janji 120fps tanpa gate perf akan terkikis diam-diam oleh setiap PR.

### 9.6 Cerita async/threading
Aplikasi nyata melakukan network/IO/database. Yang harus dijawab di arsitektur inti (GPUI dan iced punya jawaban eksplisit): bagaimana hasil async kembali ke UI thread, integrasi tokio, apakah widget boleh spawn task.

### 9.7 Strategi panic
Di Rust, satu `unwrap()` di satu widget = seluruh aplikasi mati; Flutter bisa "red screen" per-widget dan jalan terus. Perlu kebijakan: `catch_unwind` di boundary mana, error-boundary widget, crash recovery + simpan state sebelum mati.

### 9.8 i18n & RTL — arsitektural, bukan fitur susulan
Layout mirroring RTL (row terbalik, ikon panah flip, alignment) harus dipahami sistem layout sejak awal — retrofit RTL sama mustahilnya dengan retrofit accessibility. Plus sistem terjemahan (Fluent/gettext) untuk string bawaan widget ("Cancel", dialog).

### 9.9 Dokumentasi & gallery app sebagai produk
GPUI unggul teknis tapi adopsi tersendat karena docs tipis. Perlu: tutorial "app pertama", contoh per komponen, dan **gallery app interaktif** (ala Flutter Gallery) — sekaligus alat testing visual manual.

**Pola umum**: rancangan sudah kuat di *arsitektur dan scope*, lubangnya ada di *proses dan keberlanjutan* — cara kerja sehari-hari (DX, testing), cara bertahan hidup (funding, flagship), dan cara gagal dengan anggun (panic, error).

---

Referensi kunci: [winit](https://github.com/rust-windowing/winit) · [wgpu](https://docs.rs/wgpu) · [Vello](https://github.com/linebender/vello) · [cosmic-text](https://github.com/pop-os/cosmic-text) · [parley](https://github.com/linebender/parley) · [Taffy](https://github.com/DioxusLabs/taffy) · [Xilem/Masonry](https://github.com/linebender/xilem) · [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) · [Zed: 120fps](https://zed.dev/blog/videogame) · [AccessKit](https://accesskit.dev/) · [WWDC23 Springs](https://developer.apple.com/videos/play/wwdc2023/10158/) · [Impeller stencil-then-cover](https://github.com/flutter/engine/pull/50856)
