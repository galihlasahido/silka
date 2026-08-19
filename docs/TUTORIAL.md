# Aplikasi pertamamu

> Membuat aplikasi todo kecil dengan **silka**, dari jendela kosong sampai build release.
> Perkiraan waktu: 30–45 menit. Tidak perlu tahu GPU, tidak perlu tahu Flutter.

Kalau kamu pernah menulis Rust sederhana (`struct`, `Vec`, closure), kamu sudah cukup siap.
Yang akan kita bangun:

```
┌────────────────────────────────┐
│  Tugas hari ini                │
│  ┌──────────────────┐ ┌──────┐ │
│  │ Apa yang mau …   │ │Tambah│ │
│  └──────────────────┘ └──────┘ │
│  ☑ Baca dokumen tutorial  Hapus│
│  ☐ Jalankan cargo run     Hapus│
│  ☐ Ganti preset           Hapus│
│                                │
│  1 dari 3 selesai  [Bersihkan] │
└────────────────────────────────┘
```

Seluruh kode di halaman ini adalah kode sungguhan yang ikut dikompilasi CI. Kamu bisa
membacanya utuh di:

- [`examples/todo/src/bin/first_window.rs`](../examples/todo/src/bin/first_window.rs) — langkah 2 (jendela pertama)
- [`examples/todo/src/main.rs`](../examples/todo/src/main.rs) — aplikasi todo lengkap

Kalau ada potongan di sini yang tidak bisa di-`cargo run`, itu **bug dokumen**, bukan salahmu.

---

## Peta perjalanan

| Langkah | Isi | Yang kamu pelajari |
|---|---|---|
| 1 | Menyiapkan proyek | `Cargo.toml`, crate mana yang dipakai |
| 2 | Jendela pertama | `window()`, `run_app()`, `Fonts` |
| 3 | Layout | `div()`, flex, `gap`, `p_*`, `constrained` |
| 4 | State | `use_signal`, `component()`, rebuild per-komponen |
| 5 | Styling | kosakata utility + token semantik |
| 6 | Preset & dark mode | Cupertino ⇄ Tailwind, ikut OS |
| 7 | Uji tanpa jendela | `headless_app`, klik simulasi, pohon aksesibilitas |
| 8 | Build release | ukuran binary, profil rilis |

---

## Langkah 0 — Yang perlu ada

- **Rust 1.80 atau lebih baru** (`rustup update`).
- Kartu grafis apa pun yang jalan di dekade ini. silka menggambar lewat GPU (Metal di macOS,
  Vulkan/D3D12 di Linux/Windows), tapi kamu tidak akan pernah menulis kode GPU di tutorial ini.
- Salinan repositori ini. silka belum terbit di crates.io, jadi aplikasi kita hidup sebagai
  anggota workspace dan menunjuk framework lewat `path`.

Cek dulu bahwa yang sudah ada bisa jalan:

```bash
cargo run -p silka-gallery
```

Kalau galeri terbuka, semuanya beres. Kalau tidak, perbaiki itu dulu sebelum lanjut — masalahnya
ada di driver/toolchain, bukan di kode yang akan kita tulis.

---

## Langkah 1 — Menyiapkan proyek

Buat paket baru di dalam workspace:

```bash
cargo new --bin examples/todo
```

Lalu daftarkan di `Cargo.toml` di akar repo, di daftar `members`:

```toml
[workspace]
members = [
    # …
    "examples/todo",
]
```

Isi `examples/todo/Cargo.toml` seperti ini:

```toml
[package]
name = "silka-todo"
description = "The little todo app built step by step in docs/TUTORIAL.md"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
publish = false

[dependencies]
silka-core.workspace = true
silka-theme.workspace = true
silka-widgets.workspace = true
silka-platform.workspace = true
```

Empat crate, dan masing-masing punya satu tugas:

| Crate | Untuk apa | Contoh isi |
|---|---|---|
| `silka-platform` | jendela, event loop, GPU | `window()`, `run_app()` |
| `silka-core` | pohon view, layout, signals | `div()`, `use_signal`, `component()` |
| `silka-widgets` | komponen siap pakai | `button`, `checkbox`, `text_field`, `text` |
| `silka-theme` | token warna/jarak/radius | `ColorToken`, `Theme`, `Preset` |

> **Kenapa tidak ada `wgpu` di daftar itu?**
> Karena kode aplikasi tidak pernah boleh menyentuh tipe GPU. Itu aturan arsitektur, bukan
> selera: renderer boleh diganti tanpa satu baris pun berubah di aplikasimu.

---

## Langkah 2 — Jendela pertama

Kita mulai dari program terkecil yang masih jujur: satu jendela sungguhan, satu frame GPU
sungguhan, teks sungguhan dari atlas glyph. Supaya nanti tidak tertimpa aplikasi todo, taruh
langkah ini sebagai binary terpisah — buat berkas baru `examples/todo/src/bin/first_window.rs`
(Cargo otomatis menjadikan setiap berkas di `src/bin/` sebuah binary bernama sama):

```rust
use silka_core::app::{BuildCtx, ScaleFactor};
use silka_core::signals::Signal;
use silka_core::view::{div, View};
use silka_platform::{run_app, window, PlatformError};
use silka_theme::{ColorToken, FontToken};
use silka_widgets::{active_fonts, install_fonts, text, Fonts};

fn main() -> Result<(), PlatformError> {
    // Satu mesin teks untuk seluruh aplikasi: atlas glyph-nya dipakai bersama,
    // jadi satu huruf tidak pernah dirasterisasi dua kali.
    let fonts = Fonts::new();
    // Satu baris, sekali saja: sejak titik ini setiap konstruktor menemukan
    // mesin teksnya sendiri, jadi `text("…")` tidak perlu dititipi apa pun.
    install_fonts(&fonts);

    let config = window("Halo silka")
        .size(420.0, 260.0)
        // Tanpa baris ini jendelamu terkunci di satu tampilan; dengan baris ini
        // ia ikut dark mode OS secara langsung.
        .follow_system_appearance()
        // Baris yang menyerahkan atlas glyph ke backend. Lupa menulisnya =
        // semua teks kosong.
        .glyphs(fonts.shared());

    run_app(config, halaman)
}
```

Lalu isinya:

```rust
/// Seluruh "aplikasi": satu sapaan di tengah.
fn halaman(cx: &BuildCtx) -> View {
    // Teks dirasterisasi pada resolusi layar sebenarnya; ukuran logis di bawah
    // tidak ikut berubah karenanya.
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());

    div()
        .justify_center()
        .items_center()
        .gap_2()
        .p_8()
        .child(
            text("Halo, silka")
                .font(FontToken::Title1)
                .text_color(ColorToken::Label)
                .single_line(),
        )
        .child(
            text("Jendela pertamamu sudah jalan.")
                .text_base()
                .text_color(ColorToken::SecondaryLabel)
                .single_line(),
        )
        .into()
}
```

```bash
cargo run -p silka-todo --bin first_window
```

Jendela terbuka. Tiga hal yang baru saja terjadi, dan ketiganya layak dilihat sebentar:

1. **`run_app` memanggil `halaman` untuk membangun pohon view.** Fungsimu tidak menggambar apa
   pun — ia hanya *mendeskripsikan* apa yang seharusnya ada. Framework yang mengubah deskripsi
   itu jadi layout, lalu jadi perintah gambar.
2. **Gaya penulisannya "fungsi konstruktor + method chaining".** `text(...)` membuat node,
   `.font(...)` dan `.text_color(...)` menyetelnya. Kalau kamu pernah melihat Flutter, nesting-nya
   sama persis; yang di Dart jadi named parameter, di sini jadi method.
3. **Tidak ada satu pun angka warna.** `ColorToken::Label` artinya "warna teks utama", dan theme
   aktif yang memutuskan itu hitam atau putih. Inilah alasan dark mode di langkah 6 nanti
   gratis.

> **Kalau jendelanya kosong melompong:** hampir selalu karena `.glyphs(fonts.shared())` lupa
> ditulis. Teks butuh atlas glyph menyeberang ke GPU.

---

## Langkah 3 — Layout: kotak di dalam kotak

Mulai sekarang kita pindah ke `examples/todo/src/main.rs` dan membangun aplikasi todo betulan.

Satu-satunya alat layout yang kamu butuhkan untuk 90% UI adalah `div()` — kotak fleksibel yang
menumpuk anaknya ke bawah, persis seperti `div` di web sebelum ada yang menyebut `display: flex`.
Panggil `.flex()` untuk menyusun ke samping.

```rust
div()
    .flex()            // susun mendatar
    .items_center()    // rata tengah pada sumbu silang
    .justify_between() // dorong anak pertama & terakhir ke tepi
    .gap_3()           // jarak 3 langkah skala (12pt)
    .p_6()             // padding 6 langkah (24pt)
```

Angka-angka itu bukan piksel sembarangan: `gap_3()` berarti **3 langkah pada skala 4pt**, dan
skalanya milik theme. Ganti preset, semua jarak ikut bergeser bersama-sama.

Untuk membatasi lebar kartu, bungkus dengan `constrained`:

```rust
div()
    .items_center()
    .p_8()
    .child(constrained(
        BoxConstraints::new(0.0, t.space(LEBAR_KARTU), 0.0, f32::INFINITY),
        kartu(daftar, masukan),
    ))
    .into()
```

`BoxConstraints::new(min_lebar, maks_lebar, min_tinggi, maks_tinggi)` — jadi kartu boleh selebar
apa pun sampai `t.space(110.0)` (110 langkah = 440pt), lalu berhenti. `t.space(n)` inilah cara
menulis "n langkah skala" saat angkanya dihitung, bukan ditulis literal.

> **Tidak ada aritmetika layout di kode aplikasi.** Kamu tidak akan pernah menulis
> `(lebar - padding * 2 - gap) / 2`. Kalau kamu mulai menulis itu, berarti ada utility yang
> belum kamu temukan.

Dan `expanded(...)` adalah "ambil sisa ruang":

```rust
div()
    .flex()
    .items_center()
    .gap_3()
    .child(expanded(kolom_masukan))   // memanjang
    .child(tombol_tambah)             // seukuran isinya
```

---

## Langkah 4 — State: signals

Sekarang bagian yang membuat aplikasi hidup.

### 4a. Tulis modelnya dulu, tanpa framework

Kebiasaan yang menyelamatkan banyak waktu: logika aplikasi ditulis sebagai Rust biasa, sehingga
bisa diuji tanpa jendela sama sekali.

```rust
/// Satu tugas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tugas {
    /// Identitas stabil — inilah yang dipakai view sebagai key.
    pub id: u64,
    /// Apa yang diketik pengguna.
    pub judul: String,
    /// Sudah dicentang atau belum.
    pub selesai: bool,
}

/// Menambah tugas, mengabaikan masukan kosong.
///
/// Mengembalikan `true` kalau benar-benar ada yang ditambahkan — itulah tanda
/// bagi pemanggil untuk mengosongkan kolom isian.
pub fn tambah(daftar: &mut Vec<Tugas>, judul: &str) -> bool {
    let judul = judul.trim();
    if judul.is_empty() {
        return false;
    }
    let id = id_berikutnya(daftar);
    daftar.push(Tugas {
        id,
        judul: judul.to_string(),
        selesai: false,
    });
    true
}
```

Selebihnya (`hapus`, `setel_selesai`, `bersihkan`, `ringkasan`) sama polosnya — lihat modul
`model` di `examples/todo/src/main.rs`.

### 4b. Simpan state di signal

```rust
pub fn aplikasi(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());

    // Dua potong state, dan itulah seluruh state aplikasi.
    let daftar = use_signal(model::contoh);
    let masukan = use_signal(String::new);
    // …
}
```

`use_signal(init)` membuat state milik komponen ini. Ia hanya memanggil `init` sekali; pada
rebuild berikutnya kamu mendapat signal yang sama.

Aturannya cuma satu, dan sama seperti hooks di React: **panggil `use_signal` dalam urutan yang
sama setiap kali build** — jangan di dalam `if` atau loop.

Membacanya:

```rust
let tugas = daftar.get();      // salinan, dan MENCATAT langganan
daftar.with(|d| d.len());      // pinjam, juga mencatat langganan
let judul = masukan.peek();    // baca TANPA berlangganan
```

Menulisnya:

```rust
daftar.update(|d| model::hapus(d, id));  // ubah di tempat
masukan.set(String::new());              // ganti isi
```

### 4c. "Membaca" = "berlangganan"

Ini inti model state silka: siapa yang **membaca** signal saat build, dialah yang dibangun ulang
saat signal itu berubah. Bukan seluruh aplikasi — hanya bagian itu.

Karena itu kita membungkus tiap bagian yang membaca signal ke dalam `component()`:

```rust
/// Baris isian, sebagai komponennya sendiri.
///
/// Ini satu-satunya tempat `masukan` dibaca, jadi satu ketukan tombol membangun
/// ulang baris ini dan tidak ada yang lain.
fn formulir(daftar: Signal<Vec<Tugas>>, masukan: Signal<String>) -> View {
    let tema = *t;
    component("formulir", move |cx| {
        let t: Theme = cx.env::<Signal<Theme>>().map(|s| s.get()).unwrap_or(tema);
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(expanded(
                text_field(masukan.get())
                    .key("baru")
                    .label(KOLOM_BARU)
                    .placeholder("Apa yang mau dikerjakan?")
                    .on_change(move |s| masukan.set(s.to_string()))
                    .on_submit(move |_| kirim(daftar, masukan)),
            ))
            .child(button(TOMBOL_TAMBAH).on_press(move || kirim(daftar, masukan)))
            .into()
    })
}
```

Tiga hal kecil yang penting di sini:

- **`.key("baru")` pada `text_field`.** Kolom ini dibangun ulang tiap ketukan tombol (karena ia
  membaca `masukan`), tapi karena key-nya tetap, node-nya bertahan — kursor, seleksi, dan
  komposisi IME tidak hilang.
- **`.label(KOLOM_BARU)`.** Itulah nama yang diumumkan screen reader. Bukan tambahan; ia bagian
  dari kontrak komponen.
- **`on_submit`.** Enter menambah tugas. Keyboard bukan warga kelas dua.

Handler-nya sendiri:

```rust
/// Menambahkan isi kolom, lalu mengosongkannya.
///
/// Sengaja `peek()` bukan `get()`: ini berjalan di dalam event handler, bukan di
/// dalam build, jadi tidak ada scope untuk berlangganan.
fn kirim(daftar: Signal<Vec<Tugas>>, masukan: Signal<String>) {
    let judul = masukan.peek();
    if daftar.update(|d| model::tambah(d, &judul)) {
        masukan.set(String::new());
    }
}
```

### 4d. Daftar tugas dan key

```rust
fn baris(daftar: Signal<Vec<Tugas>>, tugas: &Tugas) -> View {
    let id = tugas.id;
    let kunci = Key::num(id as i64);
    div()
        .key(kunci.clone())
        .flex()
        .items_center()
        .gap_2()
        .child(expanded(
            checkbox(tugas.judul.clone())
                .key(kunci)
                .checked(tugas.selesai)
                .on_toggle(move |on| daftar.update(|d| model::setel_selesai(d, id, on))),
        ))
        .child(
            button_variant(TOMBOL_HAPUS, ButtonVariant::Ghost)
                .key(Key::num(-(id as i64) - 1))
                .on_press(move || daftar.update(|d| model::hapus(d, id))),
        )
        .into()
}
```

> **Kenapa key-nya `id` dan bukan posisi?**
> Hapus baris pertama, dan semua baris di bawahnya bergeser satu posisi. Kalau key-nya posisi,
> framework menyimpulkan "baris ke-2 isinya berubah" dan state widget (animasi yang sedang
> berjalan, focus ring) menempel di tempat yang salah. Dengan key `id`, framework tahu satu baris
> **hilang** dan sisanya cuma pindah. Ini disiplin yang harus kamu jaga sendiri di setiap list
> dinamis.

Label tugas menempel pada checkbox, bukan sebagai teks terpisah: klik kata-katanya ikut
mencentang, dan screen reader menyebut tugas itu sekali, bukan dua kali.

---

## Langkah 5 — Styling: kosakata utility, tanpa CSS

Kartunya:

```rust
div()
    .gap_5()
    .p_6()
    .bg(ColorToken::Surface)
    .rounded_xl()
    .border_1()
    .border_color(ColorToken::Separator)
    .elevation(ShadowToken::Md)
    .child(/* … */)
```

Kalau kamu pernah memakai Tailwind, ini terasa akrab — dan memang disengaja. Bedanya:

- **Tidak ada CSS di belakangnya.** Tidak ada parser, tidak ada cascade, tidak ada specificity.
  `bg` adalah method Rust; salah ketik = error compile, bukan diam-diam tidak berefek.
- **Nilainya selalu token, bukan angka.** `bg(ColorToken::Surface)` menyebut *peran*
  ("permukaan kartu"), bukan warna. Karena itu satu baris yang sama benar di light dan dark, di
  Cupertino dan Tailwind.
- **`rounded_xl()` bukan sekadar "20px".** Di preset Cupertino sudutnya *squircle* (superellipse
  ala Apple), di preset Tailwind busur lingkaran biasa. Bentuk sudut adalah parameter shader,
  jadi keduanya benar-benar berbeda geometri, bukan tiruan.

Kosakata yang paling sering dipakai:

| Keluarga | Contoh | Arti |
|---|---|---|
| Latar & garis | `bg(ColorToken::Surface)`, `border_1()`, `border_color(...)` | permukaan dan tepi |
| Sudut | `rounded_sm/md/lg/xl/full()` | radius dari token |
| Bayangan | `elevation(ShadowToken::Md)`, `shadow_sm/md/lg()` | ketinggian |
| Jarak dalam | `p_4()`, `px_6()`, `py_2()`, `pt_3()` | padding, satuan 4pt |
| Jarak antar anak | `gap_1()` … `gap_12()` | spacing skala |
| Susunan | `flex()`, `flex_col()`, `items_center()`, `justify_between()` | flexbox |
| Teks | `text_sm()`, `text_base()`, `font(FontToken::Title2)`, `font_semibold()` | skala tipografi |
| Warna teks | `text_color(ColorToken::SecondaryLabel)` | peran teks |

Untuk elemen yang bereaksi terhadap kursor, ada `interactive(...)` dengan `hover`/`pressed`/
`focused` — dan transisinya **spring**, bukan potongan mendadak:

```rust
interactive(fixed(0.0, 0.0))
    .bg(ColorToken::Surface)
    .hover(|s| s.bg(ColorToken::SurfaceHover))
    .pressed(|s| s.bg(ColorToken::SurfacePressed).scale(0.98))
    .focused(|s| s.ring(ColorToken::FocusRing))
```

Kamu tidak menyebut durasi, tidak menyebut kurva, tidak memasang timer. Untuk komponen bawaan
seperti `button` dan `checkbox`, spring-nya sudah ada di dalam — asal aplikasimu memakai
`run_app_with(..., advance)` seperti di langkah berikutnya.

---

## Langkah 6 — Preset, dark mode, dan `main` yang lengkap

```rust
fn main() -> Result<(), PlatformError> {
    let fonts = Fonts::new();
    // Satu baris, sekali saja: sejak titik ini setiap konstruktor menemukan
    // mesin teksnya sendiri, jadi `text("…")` tidak perlu dititipi apa pun.
    install_fonts(&fonts);

    let config = window(NAMA_APLIKASI)
        .size(520.0, 640.0)
        .min_size(380.0, 420.0)
        .preset(preset_dari_argumen(std::env::args().skip(1)))
        .follow_system_appearance()
        .glyphs(fonts.shared());

    // `advance` menggerakkan semua spring milik widget sekali per frame. Event
    // loop tetap tidur begitu semuanya diam — adanya animasi tidak melanggar
    // janji "render hanya saat dirty".
    run_app_with(config, aplikasi, advance)
}
```

Dua entry point yang perlu kamu bedakan:

| Fungsi | Kapan dipakai |
|---|---|
| `run_app(config, build)` | tidak ada yang bergerak (halaman statis, form sederhana) |
| `run_app_with(config, build, advance)` | ada komponen bawaan yang beranimasi — hampir selalu ini |

Pemilihan preset:

```rust
/// `--preset tailwind` memilih preset first-party yang satunya.
pub fn preset_dari_argumen(args: impl Iterator<Item = String>) -> Preset {
    let args: Vec<String> = args.collect();
    let mut i = 0;
    let mut preset = Preset::Cupertino;
    while i < args.len() {
        if args[i] == "--preset" {
            if let Some(v) = args.get(i + 1) {
                preset = match v.as_str() {
                    "tailwind" | "shadcn" => Preset::Tailwind,
                    _ => Preset::Cupertino,
                };
                i += 1;
            }
        }
        i += 1;
    }
    preset
}
```

Coba keduanya:

```bash
cargo run -p silka-todo
cargo run -p silka-todo -- --preset tailwind
```

Lalu ubah dark mode di setelan OS **tanpa menutup jendela**. Warna berganti seketika, dan kamu
tidak menulis satu baris pun untuk itu: `.follow_system_appearance()` menulis theme baru ke
`Signal<Theme>`, dan hanya komponen yang membaca theme yang dibangun ulang.

Kalau kamu perlu theme di dalam kode:

```rust
let t: Theme = cx.expect_env::<Signal<Theme>>().get();
let lebar = t.space(110.0);          // 110 langkah skala
let warna_judul = t.color.label;     // kalau benar-benar butuh nilainya
```

Tapi tahan dulu: sembilan dari sepuluh kali, `text_color(ColorToken::Label)` lebih baik daripada
mengambil `t.color.label` sendiri.

---

## Langkah 7 — Menguji tanpa membuka jendela

Ini bagian yang biasanya dilewatkan tutorial GUI, dan justru bagian yang membuatmu berani
mengubah kode besok.

Model polos diuji seperti Rust biasa:

```rust
#[test]
fn tambah_memangkas_spasi_dan_memberi_id_baru() {
    let mut d = Vec::new();
    assert!(model::tambah(&mut d, "  beli kopi  "));
    assert!(model::tambah(&mut d, "tulis tutorial"));
    assert_eq!(d[0].judul, "beli kopi");
    assert_eq!(d[0].id, 1);
    assert_eq!(d[1].id, 2);
    assert!(!d[0].selesai);
}
```

Dan aplikasinya — pohon view yang sama persis dengan yang tampil di jendela — dijalankan lewat
`headless_app`, tanpa GPU, tanpa window server:

```rust
/// Aplikasi yang dirakit **persis seperti `run_app_with` merakitnya**.
fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
    install_fonts(fonts);
    headless_app(theme, aplikasi)
        .sized(VIEWPORT.width, VIEWPORT.height)
}

/// Kotak sebuah node **menurut pohon aksesibilitas** — supaya tes mengeklik
/// tepat di tempat yang diumumkan screen reader.
fn kotak(ui: &AppRuntime, label: &str) -> Rect {
    let pohon = ui.access_tree();
    pohon
        .find_label(label)
        .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
        .bounds
}
```

Dengan dua fungsi itu, sisanya tinggal pembantu satu baris — `ada(&ui, label)` (ada tidaknya node
dengan label itu) dan `klik_label(&mut ui, label)` (klik di tengah kotaknya, lalu satu frame).
Keduanya ada di bagian `mod tests` `examples/todo/src/main.rs`.

Setelah itu tes berbunyi seperti kalimat manusia:

```rust
#[test]
fn mencentang_dan_menghapus_lewat_klik() {
    let f = fonts();
    let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
    ui.frame();

    let awal = model::contoh();
    let kedua = awal[1].judul.clone();

    assert!(ada(&ui, "1 dari 3 selesai"));

    klik_label(&mut ui, &kedua);
    assert!(ada(&ui, "2 dari 3 selesai"), "centang terbaca di ringkasan");

    klik_label(&mut ui, TOMBOL_BERSIHKAN);
    assert!(!ada(&ui, &kedua), "tugas selesai ikut dibersihkan");
    assert!(ada(&ui, "0 dari 1 selesai"));

    klik_label(&mut ui, TOMBOL_HAPUS);
    assert!(ada(&ui, KOSONG), "kartu kosong punya kalimatnya sendiri");
}
```

Perhatikan: tes mencari elemen **lewat pohon aksesibilitas**, dengan label yang sama yang
diumumkan screen reader. Efek sampingnya bagus sekali — aplikasi yang tidak bisa dites dengan cara
ini adalah aplikasi yang juga tidak bisa dipakai screen reader.

```bash
cargo test -p silka-todo
```

Satu lagi yang murah dan sangat berguna, menjaga agar tidak ada warna liar menyelinap masuk:

```rust
#[test]
fn warna_selalu_datang_dari_token_di_kedua_preset() {
    for preset in [Preset::Cupertino, Preset::Tailwind] {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            // … bangun UI, lalu periksa setiap warna glyph adalah salah satu token.
        }
    }
}
```

---

## Langkah 8 — Build release

```bash
cargo build --release -p silka-todo
./target/release/silka-todo
```

Profil `release` bawaan Rust sudah jauh lebih cepat daripada `dev`. Untuk aplikasi yang benar-benar
kamu bagikan, tambahkan ini di `Cargo.toml` akar workspace:

```toml
[profile.release]
opt-level = 3
lto = "thin"        # optimasi lintas-crate; build lebih lama, binary lebih kecil & cepat
codegen-units = 1   # satu unit = optimasi terbaik
strip = "symbols"   # buang simbol debug dari binary yang didistribusikan
```

Beberapa catatan jujur:

- **Jangan pasang `panic = "abort"` tanpa berpikir.** Ia memang mengecilkan binary, tapi menutup
  pintu untuk pemulihan panic per-bagian UI. Kalau ragu, biarkan default.
- **Frame pertama selalu paling lambat**: pipeline shader dan atlas font baru dibangun di sana.
  Ukur performa setelah beberapa detik, bukan pada milidetik pertama.
- **Membungkus jadi `.app`/`.msi`/`.deb` bukan urusan `cargo build`.** Itu langkah paket
  tersendiri, di luar cakupan tutorial ini.

Cek ukurannya:

```bash
ls -lh target/release/silka-todo
```

---

## Kesalahan umum, dan artinya

| Yang kamu lihat | Penyebab tersering |
|---|---|
| Jendela terbuka, semua teks hilang | `.glyphs(fonts.shared())` belum dipasang di `WindowConfig` |
| Tombol tidak beranimasi saat di-hover | memakai `run_app` padahal butuh `run_app_with(..., advance)` |
| Panic "use_signal hanya boleh dipanggil saat komponen dibangun" | `use_signal` dipanggil di luar fungsi build (misal di dalam `on_press`) |
| Ketikan hilang tiap huruf | `text_field` tanpa `.key(...)`, jadi node-nya lahir baru terus |
| Baris list saling tertukar setelah hapus | key memakai indeks, bukan id |
| Klik tidak mengubah apa pun di layar | handler menulis signal, tapi tidak ada komponen yang **membaca** signal itu |
| Warna tidak ikut berubah saat dark mode | nilai warna ditulis literal, bukan lewat `ColorToken` |

---

## Selanjutnya

- **Jalankan galerinya**: `cargo run -p silka-gallery`. Setiap komponen punya halamannya sendiri,
  lengkap dengan sakelar preset dan reduce-motion. Ini juga cara tercepat melihat komponen apa
  saja yang tersedia.
- **Buka satu halaman langsung**: `cargo run -p silka-gallery -- --page table --solo`.
- **Baca `catatan/KOMPONEN.md`** untuk katalog komponen beserta Definition of Done masing-masing.
- **Baca `catatan/REKOMENDASI.md` §2.5–§2.7** kalau kamu ingin tahu *kenapa* API-nya berbentuk
  seperti ini — semua keputusan besar ada alasan tertulisnya.
- **Coba tambahkan sendiri**: filter "semua / aktif / selesai" dengan `tabs`, penyimpanan ke disk
  saat aplikasi ditutup (`on_quit`), atau menu klik-kanan per baris dengan `menu`.

Selamat — kamu baru saja menulis aplikasi desktop native, tanpa satu baris HTML, tanpa satu baris
CSS, dan tanpa pernah menyentuh GPU.
