# Katalog Komponen Framework

> Pendamping `REKOMENDASI.md`. Daftar lengkap komponen yang harus dibuat, diurutkan berdasarkan prioritas dan ketergantungan.
> Referensi silang: widget catalog Flutter, SwiftUI/AppKit (HIG), shadcn/ui, gpui-component (60+ komponen).
> Semua komponen ditulis terhadap **token semantik** (§2.7 REKOMENDASI.md) sehingga otomatis benar di preset Cupertino maupun Tailwind/shadcn. Semua state interaktif bertransisi lewat **spring system**.

Legenda status ketergantungan:
- 🔤 butuh stack text matang (cosmic-text/parley)
- ⌨️ butuh IME preedit
- 🌀 butuh scroll physics (momentum + rubber-band)
- 🪟 butuh layer/offscreen texture (blur, §3.6)
- 🧭 butuh sistem overlay/popup (window anak atau layer atas)

---

## Tier 0 — Primitif (Fase 0–1, fondasi semua komponen lain)

| Komponen | Padanan | Catatan |
|---|---|---|
| `div` / `container` | Container (Flutter), View (SwiftUI) | Kanvas utility styling: bg, border, radius (squircle/arc via token), shadow ganda, padding |
| `text` | Text | 🔤 Rich text span, optical size, tabular figures, truncation + ellipsis |
| `image` | Image | Async decode + upload concurrent (pelajaran iced 0.14), fit modes, rounded clip |
| `icon` | SF Symbols-like | Set ikon monochrome dari atlas SVG; ukuran & warna via token; varian weight |
| `spacer` | Spacer | Fleksibel di flex layout |
| `divider` | Divider/Separator | Horizontal/vertikal, warna token `separator` |

## Tier 1 — Layout (Fase 1)

| Komponen | Padanan | Catatan |
|---|---|---|
| `row` / `column` | Row/Column (Flutter), HStack/VStack | Gaya Dart §2.5: `column((a, b)).spacing(12.0)` |
| `stack` (z-axis) | Stack, ZStack | Alignment + positioned children |
| `flex` | Flex | Utility penuh: grow/shrink/basis, wrap, justify/align |
| `grid` | Grid, LazyVGrid | Via Taffy CSS Grid |
| `scroll_view` | ScrollView | 🌀 Momentum + rubber-band ala macOS, scrollbar overlay auto-hide, scroll-to |
| `list` (virtualized) | ListView.builder | 🌀 Virtualisasi wajib sejak awal (pelajaran gpui-component); sticky header |
| `aspect_ratio`, `constrained_box`, `padding`, `align`, `center` | idem Flutter | Primitif constraint ala Flutter |

## Tier 2 — Kontrol dasar (Fase 2 awal — dipakai semua aplikasi)

| Komponen | Padanan | Catatan |
|---|---|---|
| `button` | Button | Varian: primary/secondary/ghost/destructive/link; scale-on-press micro-interaction; loading state |
| `icon_button` | — | Hit area ≥ 44pt walau visual kecil (HIG) |
| `text_field` | TextField | 🔤⌨️ **Komponen tersulit di seluruh katalog**: caret + selection per grapheme, IME preedit inline, undo/redo, placeholder, clear button, prefix/suffix, drag-select, klik-ganda per kata |
| `text_area` | TextEditor | 🔤⌨️ Multiline + soft-wrap; fondasi untuk editor |
| `checkbox` | Checkbox | Termasuk state indeterminate; animasi centang |
| `radio` / `radio_group` | Radio | |
| `switch` / `toggle` | Toggle | Spring drag — bisa di-drag, bukan cuma klik (rasa iOS/macOS) |
| `slider` | Slider | Drag + keyboard + snap ke step; varian range (dua thumb) |
| `stepper` | Stepper | Angka +/- ala macOS |
| `select` / `dropdown` | Picker, shadcn Select | 🧭 Popup dengan search/filter opsional |
| `combo_box` | NSComboBox | 🧭🔤 Text field + dropdown saran |
| `label` + `form` layout | Form (SwiftUI) | Grid label-kanan/kontrol-kiri ala macOS Settings; validasi + pesan error |

## Tier 3 — Navigasi & struktur aplikasi (Fase 2)

| Komponen | Padanan | Catatan |
|---|---|---|
| `window` | NSWindow wrapper | Custom titlebar, traffic lights, vibrancy opsional 🪟, multi-window |
| `sidebar` | NavigationSplitView | 🪟 Material blur di macOS; collapsible; source-list style |
| `toolbar` | NSToolbar-like | Item overflow otomatis; inline dengan titlebar (macOS) |
| `tabs` | TabView, shadcn Tabs | Varian: segmented (macOS), underline, enclosed; animasi indikator spring |
| `segmented_control` | NSSegmentedControl | Thumb geser dengan spring (rasa iOS) |
| `breadcrumb` | shadcn Breadcrumb | |
| `menu_bar` (native) | NSMenu via muda | Menu global macOS + fallback in-window di Win/Linux |
| `context_menu` | 🧭 | Native (muda) atau custom-rendered — putuskan per platform |
| `command_palette` | Cmd+K (Zed/Raycast style) | 🧭🔤 Fuzzy search; identitas aplikasi modern |
| `split_view` / `resizable` | NSSplitView, shadcn Resizable | Drag handle, collapse, simpan proporsi |

## Tier 4 — Overlay & feedback (Fase 2–3)

| Komponen | Padanan | Catatan |
|---|---|---|
| `dialog` / `alert` | NSAlert, shadcn Dialog | 🧭 Modal dengan backdrop dim; tombol default/cancel mengikuti konvensi per-OS |
| `sheet` | Sheet (macOS slide-down) | 🧭 Spring transition dari titlebar |
| `popover` | NSPopover | 🧭 Panah penunjuk anchor, auto-flip saat mepet tepi layar |
| `tooltip` | 🧭 | Delay muncul ala macOS, mengikuti kursor keluar = hilang |
| `toast` / `notification` (in-app) | shadcn Sonner | 🧭 Stack + auto-dismiss + swipe-to-dismiss |
| `progress_bar` / `progress_circle` | ProgressView | Determinate + indeterminate |
| `skeleton` | shadcn Skeleton | Shimmer animation |
| `badge` | shadcn Badge | |
| `hover_card` | shadcn HoverCard | 🧭 |
| `drawer` | shadcn Drawer | 🧭 Slide dari tepi, drag-to-close dengan velocity handoff |

## Tier 5 — Data display (Fase 3, dibutuhkan aplikasi "serius")

| Komponen | Padanan | Catatan |
|---|---|---|
| `table` (virtualized) | NSTableView, gpui-component Table | 🌀🔤 Sort, resize/reorder kolom, seleksi baris, sticky header — komponen terberat kedua setelah text_field |
| `tree` | NSOutlineView, shadcn Tree | 🌀 Expand/collapse dengan animasi, virtualized |
| `card` | shadcn Card | |
| `accordion` / `collapsible` | shadcn Accordion | Animasi tinggi via spring |
| `avatar` + `avatar_group` | shadcn Avatar | |
| `tag` / `chip` | NSTokenField-ish | Removable, warna token |
| `calendar` / `date_picker` | shadcn Calendar, NSDatePicker | 🧭 Lokalisasi kalender = jebakan i18n, jangan remehkan |
| `color_picker` | NSColorPanel-like | 🧭 |
| `chart` (dasar) | Swift Charts, gpui-component Chart | Line/bar/area minimal; bisa ditunda ke crate terpisah |

## Tier 6 — Lanjutan (pasca-v1 / didorong kebutuhan flagship app)

| Komponen | Padanan | Catatan |
|---|---|---|
| `code_editor` | gpui-component editor (200k baris) | 🔤⌨️🌀 Syntax highlight, gutter, multi-cursor — hanya jika flagship app butuh |
| `markdown_view` | — | 🔤 Render markdown ke widget tree |
| `virtual_canvas` | node editor / whiteboard | Pan + zoom dengan gesture |
| `video_view` | AVPlayerView-like | Integrasi per-platform, berat |
| `web_view` (opsional) | wry | Escape hatch untuk konten web — bukan bagian rendering inti |
| `dock` / `panel system` | gpui-component Dock | Layout IDE-style drag-dock |

---

## Urutan pengerjaan yang disarankan

1. **Tier 0 + 1 dulu, sampai benar-benar solid** — semua tier di atasnya adalah komposisi dari sini. `scroll_view` dengan physics yang enak adalah pembeda "rasa native" paling awal yang terasa pengguna.
2. **`text_field` dimulai paling awal di Tier 2** walau selesainya paling lama — dia memaksa stack text, IME, dan accessibility matang lebih cepat (failure mode #1 dan #2 di REKOMENDASI §5).
3. **Overlay system (🧭) dibangun sekali, dipakai 10+ komponen** — dialog/popover/tooltip/menu/toast semuanya menumpang infrastruktur yang sama. Desain dulu, baru komponennya.
4. **`table` dan `tree` menunggu virtualisasi `list` terbukti** — jangan bangun tiga sistem virtualisasi.
5. Setiap komponen selesai = **emit node AccessKit + navigasi keyboard lengkap** sebagai definition of done, bukan menyusul.

## Definition of Done per komponen

- [ ] Benar di kedua preset (Cupertino & Tailwind/shadcn) via token semantik
- [ ] Semua state interaktif (hover/pressed/focused/disabled) dengan transisi spring
- [ ] Navigasi keyboard penuh + focus ring
- [ ] Node AccessKit (role, name, actions) — screen reader bisa membacanya
- [ ] Dark mode
- [ ] Hit target ≥ 44pt untuk kontrol interaktif (HIG)
- [ ] Reduced-motion menghormati setting OS
