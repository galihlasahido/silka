# silka

Framework GUI desktop untuk Rust dengan model widget deklaratif dan kualitas
visual bergaya Apple. Target v1: **macOS, Windows, dan Linux** (X11/Wayland).

> Status: dalam pengembangan aktif. API masih berubah.

## Filosofi

Silka dibangun di atas tiga keyakinan:

1. **Menulis UI harus terasa enak.** API-nya berupa komposisi bersarang dengan
   method chaining — mendekati rasa menulis widget di Flutter, tetapi tetap
   idiomatis Rust dan aman di waktu kompilasi.
2. **Gerakan adalah bagian dari desain.** Semua animasi memakai pegas (spring)
   yang menyimpan posisi dan kecepatan, sehingga bisa diarahkan ulang di
   tengah jalan tanpa patah — bukan kurva easing yang kaku.
3. **Detail kecil menentukan rasa.** Sudut squircle, bayangan berlapis, dan
   tipografi dengan optical sizing bukan hiasan, melainkan alasan sebuah
   antarmuka terasa halus.

## Contoh

```rust
let jumlah = use_signal(|| 0);

column((
    text(format!("Nilai: {}", jumlah.get())),
    button("Tambah").on_press(move || jumlah.set(jumlah.get() + 1)),
))
.spacing(12.0)
.padding(16.0)
```

Gaya penataan mengikuti pola utility seperti Tailwind, tetapi berupa method
yang diperiksa kompilator — salah tulis menjadi galat kompilasi, bukan diam
tanpa efek.

## Tema

Setiap komponen ditulis satu kali terhadap token semantik, lalu tampil sesuai
tema yang aktif:

| Preset | Karakter |
| --- | --- |
| **Cupertino** (bawaan) | Sudut squircle, palet Apple HIG, bayangan berlapis ambient + key |
| **Tailwind** | Sudut busur biasa, palet slate/blue, bayangan gaya Tailwind |

Keduanya mendukung mode terang dan gelap, dan keduanya memakai animasi pegas
yang sama — kehalusan gerak adalah identitas framework, bukan milik satu tema.

## Arsitektur

| Crate | Tanggung jawab |
| --- | --- |
| `paint` | Kosakata perintah gambar (kotak, bayangan, glyph, clip), bebas dari tipe GPU |
| `renderer` | Backend wgpu dengan shader SDF; seluruh scene digambar dalam satu draw call |
| `text` | Shaping teks, atlas glyph, dan pengukuran untuk layout |
| `core` | Signals, pohon render berbasis arena, constraint layout, animasi, input, aksesibilitas |
| `theme` | Token semantik dan preset tema |
| `widgets` | Kumpulan komponen siap pakai |
| `platform` | Shell jendela, siklus hidup aplikasi, dan integrasi sistem operasi |

Kode widget tidak pernah menyentuh tipe GPU secara langsung. Semuanya melewati
lapisan `paint`, sehingga backend penggambaran dapat diganti tanpa menyentuh
satu pun komponen.

## Aksesibilitas

Setiap komponen menerbitkan simpul aksesibilitas sebagai bagian dari kontraknya,
bukan sebagai tambahan menyusul. Navigasi papan ketik, cincin fokus, dan
penghormatan terhadap pengaturan *reduce motion* adalah syarat kelulusan sebuah
komponen, bukan fitur opsional.

## Menjalankan contoh

```bash
cargo run -p silka-gallery
```

Galeri menampilkan komponen yang tersedia beserta variasinya, lengkap dengan
pengalih tema dan mode gelap.

## Lisensi

MIT
