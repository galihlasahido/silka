# STATUS PROYEK — titik lanjut untuk sesi berikutnya

> Baca file ini PERTAMA saat melanjutkan pekerjaan. Diperbarui tiap akhir fase.
> Terakhir diperbarui: **10 Agustus 2026, 05:40**
> Dokumen rancangan: `REKOMENDASI.md` (arsitektur + 8 keputusan) · `KOMPONEN.md` (katalog widget) · `INTEGRASI-NATIVE.md` (lapisan platform)
> Nama proyek: **silka** (crate masih `rustui-*`, rename dijadwalkan setelah Fase 2 — lihat REKOMENDASI keputusan #8)

---

## Cara melanjutkan

Semua kemajuan tersimpan sebagai **kode di disk**, bukan di dalam workflow. Berhenti kapan pun aman.

```bash
# Menjalankan satu fase penuh
Workflow({scriptPath: ".../workflow-rustui.js", args: {"phase":"fase2"}})

# Melanjutkan sebagian saja (carry-over lintas sesi)
Workflow({..., args: {"phase":"fase2", "only":["chart-lib"]}})          # HANYA milestone ini
Workflow({..., args: {"phase":"fase2", "skip":["widget:button"]}})      # lewati yang sudah selesai
```

**Penting:** `resumeFromRunId` hanya berlaku dalam sesi yang sama. Untuk melanjutkan di sesi baru, pakai `args.only` / `args.skip` berdasarkan tabel di bawah.

Urutan fase wajib: `fase0` → `fase1` → `fase0b` → `fase1b` → `fase2` → `fase3` → `fase4`

---

## Ringkasan kemajuan

| Fase | Status | Bukti |
|---|---|---|
| Fase 0 — Fondasi | ✅ SELESAI | window winit+wgpu jalan di Metal, shader SDF squircle+shadow ganda, glyph atlas, frame scheduling CADisplayLink |
| Fase 0b — Jembatan Glyph | ✅ SELESAI | `Command::GlyphRun` dieksekusi; 10 test piksel termasuk 2 uji negatif |
| Fase 1 — Core | ✅ SELESAI | signals, arena render tree, view-diff, Taffy, spring, input/hit-test, AccessKit (566→698 test) |
| Fase 1b — Jahitan | ✅ SELESAI | `RenderNode::paint()`, `RenderTree::paint()→Scene`, `AppRuntime`, `run_app()`, demo counter end-to-end |
| **Fase 2 — Design System** | 🔄 **BERJALAN** | lihat tabel milestone di bawah |
| Fase 3 — Platform | ⬜ BELUM | native P0, lifecycle, escape hatch, testing infra, gallery app |
| Fase 4 — Docs & Contoh | ⬜ BELUM | API docs, tutorial, 3 contoh app, scaffold flagship |

---

## Fase 2 — status per milestone (per 05:40, 10 Agu 2026)

| Milestone | Status |
|---|---|
| clip-gpu | ✅ terverifikasi |
| tokens-preset | ✅ terverifikasi |
| overlay-system | ✅ terverifikasi |
| widget:button | ✅ terverifikasi |
| widget:checkbox | ✅ terverifikasi (setelah 1 ronde perbaikan) |
| widget:switch | ✅ terverifikasi |
| widget:slider | ✅ terverifikasi (setelah 1 ronde perbaikan) |
| widget:select | ✅ terverifikasi (setelah 1 ronde perbaikan) |
| widget:text_field | ✅ terverifikasi (setelah 1 ronde perbaikan) |
| widget:scroll_view | ✅ terverifikasi (setelah 1 ronde perbaikan) |
| widget:list | ✅ terverifikasi |
| widget:tabs | ✅ terverifikasi (setelah 1 ronde perbaikan) |
| widget:dialog | ✅ terverifikasi (setelah 1 ronde perbaikan) |
| widget:table | 🔄 berjalan |
| chart-lib | ⬜ belum tersentuh |

**Kalau sesi terputus sekarang**, lanjutkan dengan:
```
args: {"phase":"fase2", "only":["widget:table","chart-lib"]}
```

---

## Git

- Repositori lokal aktif di branch `main`. Remote: `https://github.com/galihlasahido/silka.git` (**PUBLIK**).
- Riwayat sudah digabung dengan commit awal remote (LICENSE MIT — Advance Dynamic Software). `.gitignore` menyatukan template Rust GitHub + entri editor/OS. `target/` (12 GB) terabaikan.
- **BELUM DI-PUSH** — sesuai keputusan 10 Agu 2026: push dilakukan setelah (1) Fase 2 selesai dan (2) rename `rustui-*` → `silka-*`, supaya repo publik tampil konsisten sejak commit pertama yang terlihat publik.
- **ATURAN COMMIT/PR: dilarang mencantumkan Anthropic, Claude, atau Claude Code** di pesan commit, PR, komentar kode, atau dokumen. Tanpa trailer `Co-Authored-By`, tanpa baris "Generated with".

## Pekerjaan tertunda yang harus diingat

1. **Rename `rustui-*` → `silka-*`** — sapuan mekanis (semua `Cargo.toml` + setiap `use rustui_*`). Jadwal: setelah Fase 2, saat tidak ada agen aktif.
2. **Verifikasi menyeluruh pasca-Fase 2** — tujuh komponen sempat gagal verifikasi ronde pertama, diduga karena 10 agen menulis `crates/widgets` bersamaan. Perlu dipastikan itu memang tabrakan antar-agen, bukan cacat nyata.
3. **Lubang §9 yang masih terbuka** (REKOMENDASI.md): §9.1 strategi hot reload/DX, §9.5 testing infra (dijadwalkan Fase 3), §9.6 async/threading, §9.7 strategi panic, §9.8 i18n & RTL, §9.9 dokumentasi (Fase 4).
4. **Utang teknis terdokumentasi di kode** — antara lain: belum ada eviction LRU atlas glyph, emoji berwarna belum diuji dengan glyph COLR/CBDT sungguhan, repaint boundary sejati (layer/offscreen untuk blur material) belum ada.

---

## Pelajaran proses (jangan diulang)

- **Verifikasi `cargo test` saja tidak cukup.** Tiga lubang integrasi lolos karena semua test hijau: teks tidak tergambar (Fase 0), render tree tidak bisa paint + signal tidak memicu apa pun (Fase 1). Sejak Fase 0b setiap milestone visual wajib dibuktikan berbasis piksel, termasuk **uji negatif**.
- **Jangan jalankan dua fase paralel** di workspace yang sama — gerbang `cargo test --workspace` milik satu fase akan melihat kode setengah jadi milik fase lain.
- **`args` bisa datang sebagai string JSON**, bukan objek. Script sudah menormalkannya; jangan hapus normalisasi itu atau semua run jatuh ke default `fase0`.
