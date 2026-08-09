# Katalog Integrasi Native & Komponen Low-Level

> Pendamping `REKOMENDASI.md` dan `KOMPONEN.md`. Lapisan yang membuat aplikasi terasa "warga asli" di tiap OS — inilah "ekor 90/10 platform polish" (failure mode #3): masing-masing kecil, totalnya menentukan kualitas.
> Target: macOS, Windows, Linux (X11/Wayland).
> Prinsip: framework menyediakan API lintas-platform untuk yang umum, plus **escape hatch resmi** (raw handle + objc2/windows-rs/zbus) untuk yang spesifik-platform.

Legenda prioritas: **P0** = wajib v1 · **P1** = segera setelah v1 · **P2** = didorong kebutuhan aplikasi

---

## 1. Window & Shell (P0)

| Fitur | macOS | Windows | Linux | Crate/API |
|---|---|---|---|---|
| Custom titlebar | `titlebarAppearsTransparent` + reposisi traffic lights | DWM extend frame | CSD Wayland (gambar sendiri) | objc2-app-kit, windows-rs |
| Vibrancy/blur behind-window | NSVisualEffectView | Acrylic/Mica | KWin blur protocol (opsional) | window-vibrancy |
| Multi-window + modal antar window | ✅ | ✅ | ✅ | winit |
| Always-on-top, fullscreen, minimize-to-tray | ✅ | ✅ | ✅ | winit |
| Multi-monitor + per-monitor DPI | ✅ | per-monitor v2 | `wp_fractional_scale_v1` (Wayland) | winit |
| Window snapping hints | Stage Manager-aware | Snap Layouts (Win11) | tiling WM friendly | per-platform |
| Restorasi posisi/ukuran window | state sendiri | state sendiri | state sendiri | framework kita |

## 2. Menu, Tray, Dialog, Notifikasi (P0)

| Fitur | Catatan | Crate |
|---|---|---|
| Menubar native | Global di macOS (wajib — Cmd+C/V lewat responder chain butuh Edit menu standar); in-window di Win/Linux | **muda** |
| Context menu native | Alternatif: custom-rendered agar konsisten — putuskan per komponen | muda |
| Tray/status icon | + menu tray | **tray-icon** |
| Dock (macOS) | Badge count, bounce, dock menu | objc2-app-kit |
| Taskbar (Windows) | Jump list, progress di taskbar, overlay badge | windows-rs |
| Dialog file & pesan native | Open/save/folder; di Linux lewat XDG portal (ramah Flatpak) | **rfd** |
| Notifikasi sistem | macOS: UNUserNotificationCenter (butuh signing!); Win: toast; Linux: D-Bus | **notify-rust** + per-platform |
| Badge count aplikasi | Dock/taskbar/launcher unity API | per-platform |

## 3. Input Low-Level (P0 untuk gesture & IME, sisanya P1)

| Fitur | Catatan | Crate/API |
|---|---|---|
| IME (CJK) | Preedit inline + posisi candidate window di caret | winit `Ime::*` |
| Gesture trackpad | Pinch, rotate, smart-zoom, momentum scroll **native** (macOS kirim event momentum sendiri — jangan simulasikan sendiri di sana, pakai event OS) | winit + objc2 |
| Force Touch / haptic feedback | NSHapticFeedbackManager | objc2-app-kit |
| Global hotkey | Di luar fokus aplikasi | **global-hotkey** |
| Velocity tracker | Untuk fling → spring handoff (§3.5 REKOMENDASI) | framework kita |
| Pen/stylus + pressure | Tablet events | winit (parsial) + per-platform |
| Keyboard layout, dead keys | Sudah ditangani winit; uji layout non-US | winit |
| Media keys + Now Playing | Kontrol media OS + metadata di control center | **souvlaki** |

## 4. Clipboard & Drag-and-Drop (P0)

| Fitur | Catatan | Crate |
|---|---|---|
| Clipboard text + image | | **arboard** |
| Clipboard format kaya | RTF, HTML, file-list, format kustom | per-platform (arboard terbatas) |
| Drop target (menerima file/data) | winit menyediakan | winit |
| **Drag source** (memulai drag keluar app) | **Gap ekosistem — tulis sendiri**: NSDraggingSession / DoDragDrop / wl_data_device | per-platform |
| Drag preview/thumbnail | Gambar yang ikut kursor saat drag | per-platform |

## 5. OS Services & File System (P1)

| Fitur | Catatan | Crate/API |
|---|---|---|
| File association | Buka file .xyz dengan app kita; Info.plist / registry / .desktop | packaging |
| Deep link / URL scheme | `myapp://…` + single-instance forwarding | per-platform |
| Single instance | Instance kedua meneruskan argumen ke yang pertama | named pipe / D-Bus / NSRunningApplication |
| Recent files | NSDocumentController / jump list / recent-manager | per-platform |
| Share sheet | NSSharingServicePicker (macOS), Share UI (Win) | objc2 / windows-rs |
| Quick Look (macOS) | Preview file tanpa buka app lain | objc2 |
| Buka URL/file dengan app default | | **open** / **opener** |
| Watch file system | Untuk auto-reload dokumen | **notify** |
| Trash (bukan hapus permanen) | | **trash** |
| Penyimpanan kredensial | Keychain / Credential Manager / Secret Service | **keyring** |
| Biometrik | Touch ID via LocalAuthentication; Windows Hello | objc2 / windows-rs |
| Status jaringan, proxy sistem | Reachability + proxy settings OS | sysinfo / per-platform |

## 6. Lifecycle & Setting OS (P0 — murah tapi sering dilupakan)

| Fitur | Catatan |
|---|---|
| Dark mode change (live) | winit theme event — semua token warna harus reaktif |
| Accent color OS | macOS & Windows punya; petakan ke token `accent` |
| Reduced motion / reduce transparency | Matikan spring bounce & blur — sudah jadi DoD komponen |
| Locale/region change | Format tanggal/angka mengikuti OS |
| Launch at login | SMAppService / registry Run / autostart .desktop |
| Prevent sleep | Caffeinate saat render/export panjang: IOPMAssertion / SetThreadExecutionState / D-Bus inhibit |
| Peristiwa quit/logout OS | Simpan state sebelum ditutup; `NSApplicationDelegate` / WM_QUERYENDSESSION |
| Session restore | Buka kembali window & dokumen seperti sebelum quit |

## 7. Hardware & Media (P2 — sediakan sebagai crate companion opsional, bukan inti)

| Fitur | Crate |
|---|---|
| Audio in/out | **cpal** (+ **rodio** untuk playback) |
| Kamera | **nokhwa** |
| Screen capture | ScreenCaptureKit (macOS) / **scap**; ingat izin privacy per-OS |
| Bluetooth LE | **btleplug** |
| Serial / USB | **serialport** / **rusb** |
| Info sistem (CPU/RAM/baterai) | **sysinfo** / **battery** |
| GPU info & pemilihan adapter | wgpu sudah expose |

## 8. Escape Hatch — kontrak resmi framework (P0, keputusan arsitektur)

Aplikasi harus bisa turun ke level platform **tanpa menunggu framework**:
- `window.raw_handle()` → `RawWindowHandle` (NSWindow*/HWND/wl_surface) untuk FFI langsung.
- Re-export resmi: **objc2** (macOS), **windows-rs** (Windows), **zbus** (D-Bus Linux) — versi dikunci framework agar tidak konflik.
- Hook event loop: aplikasi bisa menyisipkan handler event native mentah sebelum framework memprosesnya.
- Konvensi `#[cfg(target_os)]` + modul `platform::` di API publik — kode spesifik platform adalah hal normal, bukan aib.

## 9. Distribusi & Operasional (P0 untuk signing + updater, sisanya P1)

| Fitur | Catatan | Tool |
|---|---|---|
| Code signing + notarization macOS | Tanpa ini: Gatekeeper blokir + notifikasi tidak jalan | codesign/notarytool di CI |
| Signing Windows | Authenticode; tanpa ini SmartScreen menakuti user | signtool |
| Bundling | .app/.dmg, MSI/NSIS, AppImage/Flatpak/deb/rpm | **cargo-packager** / **cargo-bundle** |
| Auto-update | Pola Sparkle: cek feed, unduh delta, verifikasi tanda tangan, ganti saat restart | **velopack** / cargo-packager updater |
| Crash reporting | Minidump + simbolisasi | **sentry** / minidump-writer |
| Sandbox & entitlements | Wajib untuk Mac App Store; batasi API yang dipakai | per-target |

---

## Urutan pengerjaan yang disarankan

1. **P0 §1+§2+§6 dulu** — window/menu/dialog/dark-mode adalah wajah "warga asli" yang paling cepat terasa.
2. **§8 (escape hatch) diputuskan sebelum API 1.0** — ini kontrak publik; menambahkannya belakangan memecah ekosistem.
3. **Drag source (§4) dijadwalkan eksplisit** — satu-satunya item P0 tanpa crate siap pakai; murni kerja per-platform.
4. **§9 signing + updater masuk CI sejak flagship app pertama** — jangan tunggu rilis; notarization yang gagal di hari rilis adalah tradisi buruk yang bisa dihindari.
5. **§7 hardware = crate companion terpisah** (`framework-media`, `framework-device`) — jaga inti tetap ramping.
