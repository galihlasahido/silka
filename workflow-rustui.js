export const meta = {
  name: 'rustui-fases',
  description: 'Orkestrasi pembangunan framework GUI Rust (Fase 0-3) sesuai REKOMENDASI.md, KOMPONEN.md, INTEGRASI-NATIVE.md',
  whenToUse: 'Jalankan per fase dengan args {phase:"fase0"|"fase0b"|"fase1"|"fase2"|"fase3"|"fase4"|"all"}; opsional args.components untuk membatasi komponen Fase 2. Urutan wajib: fase0 -> fase1 -> fase0b -> fase1b -> fase2 -> fase3 -> fase4',
  phases: [
    { title: 'Persiapan', detail: 'baca dokumen rancangan + scaffold workspace Cargo', model: 'opus' },
    { title: 'Fase 0 - Fondasi', detail: 'winit+wgpu, shader SDF squircle, glyph atlas, frame scheduling', model: 'opus' },
    { title: 'Fase 0b - Jembatan Glyph', detail: 'tambalan: GlyphRun -> GPU (teks belum tergambar)', model: 'opus' },
    { title: 'Fase 1 - Core', detail: 'signals, view-diff, arena render tree, layout, spring, input, AccessKit', model: 'opus' },
    { title: 'Fase 1b - Jahitan', detail: 'paint pass, glue signals->view->paint, demo end-to-end', model: 'opus' },
    { title: 'Fase 2 - Design System', detail: 'token + dual preset + overlay system + komponen + table + chart', model: 'opus' },
    { title: 'Fase 3 - Platform', detail: 'integrasi native P0, testing infra, gallery app', model: 'opus' },
    { title: 'Fase 4 - Docs & Contoh', detail: 'API docs rustdoc, tutorial, contoh aplikasi, scaffold flagship', model: 'opus' },
  ],
}

// Semua agen di seluruh fase memakai Opus 5 (permintaan eksplisit pemilik proyek).
const MODEL = 'opus'

// ============================================================
// Konfigurasi
// ============================================================
const ROOT = '/Users/galihlasahido/Documents/04Development/rustui'
const DOCS = `${ROOT}/REKOMENDASI.md, ${ROOT}/KOMPONEN.md, ${ROOT}/INTEGRASI-NATIVE.md`

// Konteks standar yang dibawa setiap agent implementasi.
const CTX = `
Kamu bekerja di workspace Rust di ${ROOT}.
WAJIB: baca dulu dokumen rancangan (${DOCS}) sebelum menulis kode — semua keputusan arsitektur ada di sana dan MENGIKAT:
- API publik gaya Dart: fungsi konstruktor + method chaining (REKOMENDASI §2.5)
- Styling utility ala Tailwind sebagai method chain, tanpa CSS (§2.6)
- Token semantik + dual preset Cupertino/Tailwind; radius squircle = parameter shader, bukan konstanta (§2.7)
- Model state: signals + rebuild per-komponen pola Dioxus (§2.5)
- Arsitektur: view-diff -> arena render tree; box constraints ala Flutter + Taffy untuk flex/grid (§2, §3.4)
- Renderer: wgpu satu tingkat di balik abstraksi paint tipis — kode widget TIDAK boleh menyentuh tipe wgpu (§3.2)
- Animasi: spring (posisi+velocity, retargetable); render hanya saat dirty (§3.5)
- AccessKit node emission adalah bagian dari kontrak widget, bukan susulan (§3.8)
Tulis kode yang compile (cargo check lulus), dengan unit test untuk logika non-visual. Jangan menyentuh file di luar ${ROOT}.
`

const IMPL_SCHEMA = {
  type: 'object',
  properties: {
    ok: { type: 'boolean' },
    summary: { type: 'string', description: 'ringkasan apa yang dibuat' },
    crates: { type: 'array', items: { type: 'string' }, description: 'crate/path yang disentuh' },
    notes: { type: 'string', description: 'keputusan penting / utang teknis yang ditinggalkan' },
  },
  required: ['ok', 'summary'],
}

const VERIFY_SCHEMA = {
  type: 'object',
  properties: {
    ok: { type: 'boolean' },
    errors: { type: 'string', description: 'output error cargo/test kalau gagal, ringkas' },
  },
  required: ['ok'],
}

// ============================================================
// Helper
// ============================================================
function budgetOk() {
  if (budget.total && budget.remaining() < 40_000) {
    log(`Budget hampir habis (sisa ${Math.round(budget.remaining() / 1000)}k) — berhenti rapi.`)
    return false
  }
  return true
}

// Filter milestone untuk carry-over: jalankan hanya sebagian, atau lewati yang sudah selesai.
//   args.only = ["widget:table","chart-lib"]  -> HANYA ini yang dijalankan
//   args.skip = ["widget:button", ...]        -> ini dilewati
// Dipakai saat melanjutkan pekerjaan di sesi baru (resumeFromRunId tidak lintas-sesi).
function shouldRun(key) {
  if (Array.isArray(ARGS.only) && ARGS.only.length) return ARGS.only.includes(key)
  if (Array.isArray(ARGS.skip) && ARGS.skip.includes(key)) return false
  return true
}

// implement -> verify (cargo check + test) -> fix (maks 2 ronde)
async function milestone(phaseTitle, key, spec) {
  if (!shouldRun(key)) {
    log(`[${key}] dilewati (filter args.only/args.skip)`)
    return { key, skipped: true }
  }
  if (!budgetOk()) return { key, skipped: true }
  const impl = await agent(`${CTX}\n\nTUGAS [${key}]:\n${spec}`, {
    label: `impl:${key}`, phase: phaseTitle, schema: IMPL_SCHEMA, model: MODEL,
  })
  if (!impl) return { key, ok: false, summary: 'agent implementasi gagal/di-skip' }

  let verdict = null
  for (let round = 0; round < 3; round++) {
    verdict = await agent(
      `Di ${ROOT}: jalankan \`cargo check --workspace\` lalu \`cargo test --workspace\`. ` +
      `Fokus verifikasi milestone [${key}] (${impl.summary}). ` +
      `Nilai juga: apakah hasil kerja mematuhi kontrak di ${DOCS} (abstraksi paint, gaya API Dart, token semantik). ` +
      `ok=true hanya jika build + test lulus DAN kontrak dipatuhi.`,
      { label: `verify:${key}`, phase: phaseTitle, schema: VERIFY_SCHEMA, model: MODEL },
    )
    if (!verdict || verdict.ok) break
    if (round === 2 || !budgetOk()) break
    log(`[${key}] verifikasi gagal (ronde ${round + 1}) — memperbaiki.`)
    await agent(
      `${CTX}\n\nPERBAIKAN [${key}]: verifikasi gagal dengan error berikut, perbaiki sampai cargo check + test lulus:\n${verdict.errors ?? '(tidak ada detail)'}`,
      { label: `fix:${key}`, phase: phaseTitle, schema: IMPL_SCHEMA, model: MODEL },
    )
  }
  return { key, ok: verdict ? verdict.ok : impl.ok, summary: impl.summary, notes: impl.notes ?? '' }
}

// args bisa datang sebagai objek ATAU sebagai string JSON — normalkan dulu.
// (Tanpa ini, args.phase = undefined dan SEMUA run jatuh ke default 'fase0'.)
const ARGS = (() => {
  if (typeof args === 'string') {
    try { return JSON.parse(args) } catch { return {} }
  }
  return args || {}
})()

const SELECTED_PHASE = ARGS.phase || 'fase0'
const wantPhase = p => SELECTED_PHASE === 'all' || SELECTED_PHASE === p
log(`Fase yang dipilih: ${SELECTED_PHASE}`)

const report = { persiapan: [], fase0: [], fase1: [], fase2: [], fase3: [], fase4: [] }

// ============================================================
// Persiapan — HANYA saat fase0/all (atau dipaksa via args.scaffold).
// Fase 1+ mengasumsikan workspace sudah ada; menjalankan scaffold lagi
// hanya membuang agen dan membingungkan pembaca progres.
// ============================================================
if (wantPhase('fase0') || ARGS.scaffold === true) {
phase('Persiapan')
const prep = await agent(
  `${CTX}\n\nTUGAS [scaffold]: Periksa apakah ${ROOT} sudah berisi workspace Cargo. ` +
  `Jika belum, buat workspace dengan struktur crate berikut (semua lib kosong tapi compile, dengan doc-comment tujuan masing-masing):\n` +
  `- crates/paint      (abstraksi perintah gambar: rect/glyph/shadow/blur — TANPA tipe wgpu di API publik)\n` +
  `- crates/renderer   (backend wgpu yang mengimplementasikan paint)\n` +
  `- crates/text       (wrapper cosmic-text: shaping, glyph atlas, measure)\n` +
  `- crates/core       (signals, view-diff, arena render tree, box constraints, spring, input, scheduler)\n` +
  `- crates/theme      (token semantik + preset cupertino & tailwind)\n` +
  `- crates/widgets    (komponen sesuai KOMPONEN.md)\n` +
  `- crates/platform   (winit shell, escape hatch raw_handle, integrasi native)\n` +
  `- examples/gallery  (binary demo)\n` +
  `Tambahkan rust-toolchain.toml, .gitignore, README.md singkat yang merujuk dokumen rancangan. ` +
  `Jika workspace sudah ada, laporkan strukturnya saja tanpa mengubah apa pun.`,
  { label: 'scaffold', phase: 'Persiapan', schema: IMPL_SCHEMA, model: MODEL },
)
report.persiapan.push(prep ? prep.summary : 'scaffold gagal')
}

// ============================================================
// Fase 0 — Fondasi rendering
// ============================================================
if (wantPhase('fase0')) {
  phase('Fase 0 - Fondasi')
  // Berurutan: window dulu (semua milestone lain menumpang padanya), sisanya paralel via pipeline.
  report.fase0.push(await milestone('Fase 0 - Fondasi', 'window-wgpu',
    `Di crates/platform + crates/renderer: window winit 0.30+ dengan surface wgpu (Metal di macOS), ` +
    `resize + DPI benar, clear color dari token theme. Binary examples/gallery menampilkannya.`))

  const fase0Jobs = [
    ['sdf-shader',
      `Di crates/renderer: shader WGSL SDF untuk rounded rect dengan DUA mode geometri sudut via parameter: ` +
      `arc biasa dan squircle (superellipse, REKOMENDASI §3.6). Plus shadow ganda ambient+key, dan border. ` +
      `Semua shader dikompilasi build-time (pelajaran Impeller — tanpa shader runtime). Demo di gallery: grid kartu squircle vs arc.`],
    ['glyph-atlas',
      `Di crates/text: integrasi cosmic-text — load font (bundel Inter), shaping, glyph atlas texture dengan cache varian subpixel-offset, ` +
      `API measure(text, constraints) untuk layout. Render teks lewat abstraksi paint. Demo teks di gallery.`],
    ['frame-scheduling',
      `Di crates/core + platform: scheduler render-on-dirty (tanpa loop terus-menerus saat idle), ` +
      `vsync via CADisplayLink di macOS (ProMotion-aware, jangan hardcode 16.6ms), fallback ke request_redraw winit di OS lain. ` +
      `Ukur dan log frame time di debug build.`],
  ]
  const r0 = await pipeline(fase0Jobs, j => milestone('Fase 0 - Fondasi', j[0], j[1]))
  report.fase0.push(...r0.filter(Boolean))
}

// ============================================================
// Fase 0b — Tambalan Fase 0: jembatan glyph atlas -> GPU
// Ditemukan 10 Agu 2026 saat audit manual: crates/renderer/src/instance.rs
// membuang Command::GlyphRun (`_ => continue`), sehingga TEKS TIDAK PERNAH
// TERGAMBAR walau crates/text sudah shaping + rasterisasi ke atlas.
// Lolos verifikasi otomatis karena cargo check/test hijau — tidak ada uji visual.
// WAJIB selesai sebelum Fase 2 (semua komponen menampilkan teks).
// ============================================================
if (wantPhase('fase0b')) {
  phase('Fase 0b - Jembatan Glyph')
  report.fase0.push(await milestone('Fase 0b - Jembatan Glyph', 'glyph-gpu-bridge',
    `Di crates/renderer (boleh menyesuaikan API crates/text bila perlu): lengkapi jalur render glyph yang HILANG. ` +
    `Kondisi saat ini yang harus diperbaiki: fill_instances() di instance.rs hanya menangani Command::Quad dan Command::Shadow, ` +
    `lalu \`_ => continue\` membuang Command::GlyphRun — akibatnya teks tidak pernah sampai ke GPU. ` +
    `Renderer juga belum punya tekstur atlas, sampler, maupun pipeline bertekstur (bind group yang ada hanya uniform Globals).\n\n` +
    `Yang harus dibangun:\n` +
    `1. Upload atlas glyph dari crates/text ke wgpu::Texture (R8Unorm untuk mask alpha; siapkan jalur untuk emoji berwarna nanti), ` +
    `dengan update inkremental saat glyph baru masuk atlas — JANGAN re-upload seluruh atlas tiap frame.\n` +
    `2. Bind group + sampler untuk atlas, dan pipeline/shader yang men-sample atlas lalu mewarnai dengan warna GlyphRun ` +
    `(alpha blending benar, warna dari token — jangan hard-code).\n` +
    `3. Konversi Command::GlyphRun menjadi instance quad bertekstur (posisi + UV dari GlyphImageId), sadar subpixel positioning ` +
    `dan scale factor DPI — hasil harus tajam di layar 2x.\n` +
    `4. Batching: glyph sewarna dari satu atlas digambar dalam satu draw call; urutan gambar tetap terjaga terhadap Quad/Shadow ` +
    `(teks di atas latar, bukan tertimpa).\n` +
    `5. Hapus/persempit lengan \`_ => continue\` agar command yang benar-benar belum didukung terdokumentasi eksplisit.\n\n` +
    `VERIFIKASI YANG DIWAJIBKAN (ini inti tambalan — jangan cuma cargo test): ` +
    `render headless ke offscreen texture (crates/renderer/src/offscreen.rs sudah ada), lalu BUKTIKAN secara terprogram ` +
    `bahwa piksel teks benar-benar ada — misal hitung jumlah piksel non-latar di dalam kotak teks dan bandingkan dengan ambang, ` +
    `plus uji negatif: scene tanpa GlyphRun harus menghasilkan nol piksel teks. Jadikan ini unit test permanen. ` +
    `Jalankan juga examples/gallery halaman teks untuk memastikan teks tampil di window sungguhan.`))
}

// ============================================================
// Fase 1 — Core framework
// ============================================================
if (wantPhase('fase1')) {
  phase('Fase 1 - Core')
  // Signals + arena tree adalah fondasi; harus selesai sebelum sisanya.
  report.fase1.push(await milestone('Fase 1 - Core', 'signals',
    `Di crates/core: runtime signals pola Dioxus (REKOMENDASI §2.5): use_signal, dependency tracking per-komponen, ` +
    `dirty marking + batching, scope/identity untuk list dinamis (key). Unit test menyeluruh untuk tracking & batching.`))
  report.fase1.push(await milestone('Fase 1 - Core', 'arena-tree',
    `Di crates/core: arena/slotmap render tree ber-ID + box constraints ala Flutter (constraints turun, size naik, parent set posisi), ` +
    `relayout boundaries, dan lapisan view-diff: view tree ringan gaya Dart di-diff ke render tree. Unit test diffing & layout.`))

  const fase1Jobs = [
    ['taffy-flex',
      `Di crates/core: integrasi Taffy sebagai widget flex/grid di dalam protokol box-constraints; ` +
      `text measurement masuk lewat measure function leaf. API publik: row()/column()/grid() gaya Dart dengan .spacing()/.gap_*().`],
    ['spring',
      `Di crates/core: sistem animasi spring — solusi closed-form damped harmonic oscillator, state (posisi, velocity), ` +
      `retarget kapan pun dengan velocity dibawa (WWDC23), preset smooth/snappy/bouncy, hormati reduced-motion. ` +
      `Terhubung ke scheduler dirty. Unit test konvergensi & retarget.`],
    ['input-hittest',
      `Di crates/core + platform: routing event pointer/keyboard, hit-testing di render tree (sadar geometri squircle), ` +
      `focus & tab-order, velocity tracker untuk gesture handoff, dan wiring IME winit (preedit/commit) ke arsitektur.`],
    ['accesskit',
      `Di crates/core: emisi node AccessKit sebagai output pass render tree (role, name, bounds, actions) + adapter winit. ` +
      `Kontrak widget: setiap widget wajib bisa mengisi node-nya. Verifikasi dengan tree dump di test.`],
  ]
  const r1 = await pipeline(fase1Jobs, j => milestone('Fase 1 - Core', j[0], j[1]))
  report.fase1.push(...r1.filter(Boolean))
}

// ============================================================
// Fase 1b — Menyambung jahitan antar-lapisan (ditemukan 10 Agu 2026)
// Audit manual setelah Fase 1: tiap lapisan matang SENDIRI-SENDIRI dan lulus
// cargo test, tapi SEAM antar-lapisan tidak ada, sehingga tidak ada apa pun
// yang benar-benar tampil dari sebuah view tree:
//   A. RenderNode tidak punya paint() — tidak ada yang menyusun rustui_paint::Scene
//      dari render tree (arena hanya punya flag needs_paint; core/lib.rs baris 143
//      menyebutnya sebagai rencana, bukan kode).
//   B. Runtime::drain_dirty() tidak pernah dipanggil dari luar modul signals —
//      perubahan signal tidak memicu rekonsiliasi view sama sekali.
// WAJIB selesai sebelum Fase 2: tanpa ini komponen dibangun di atas framework
// yang tidak bisa menampilkan apa pun.
// ============================================================
if (wantPhase('fase1b')) {
  phase('Fase 1b - Jahitan')

  report.fase1.push(await milestone('Fase 1b - Jahitan', 'paint-pass',
    `Di crates/core: bangun PAINT PASS yang menyusun rustui_paint::Scene dari render tree — saat ini TIDAK ADA. ` +
    `Fakta terkini: trait RenderNode hanya punya layout/is_relayout_boundary/access/type_name; arena punya flag ` +
    `needs_paint/mark_needs_paint/clear_paint tapi tidak ada yang menghasilkan perintah gambar.\n\n` +
    `Yang harus dibangun:\n` +
    `1. Tambah method paint(&self, ctx: &mut PaintCtx) ke trait RenderNode (default kosong agar node non-visual tidak terpaksa mengisi).\n` +
    `2. PaintCtx: menerima posisi absolut hasil layout, memberi API push quad/shadow/glyph_run lewat rustui-paint SAJA ` +
    `(dilarang menyentuh wgpu), plus paint_child() untuk merambati anak sesuai urutan gambar (anak di atas induk).\n` +
    `3. Clipping: Viewport harus meng-clip anaknya; sediakan mekanisme clip rect di PaintCtx (dan bawa ke Scene bila paint perlu perintah baru — ` +
    `Command sudah #[non_exhaustive] sehingga boleh ditambah, tapi renderer harus tetap compile).\n` +
    `4. Primitif yang sudah ada (FixedBox, PaddingBox, ConstrainedBox, Flex, Viewport) diberi implementasi paint yang masuk akal ` +
    `(latar dari token bila punya, teruskan ke anak).\n` +
    `5. RenderTree::paint(&mut self) -> Scene yang memakai flag needs_paint untuk melewati subtree bersih, dan clear_paint setelahnya.\n\n` +
    `VERIFIKASI WAJIB: unit test yang membangun pohon kecil, memanggil layout lalu paint, dan MEMERIKSA ISI Scene ` +
    `(jumlah command, urutan gambar induk-sebelum-anak, posisi absolut benar setelah padding/flex, clip Viewport bekerja).`))

  report.fase1.push(await milestone('Fase 1b - Jahitan', 'reactive-glue',
    `Di crates/core: sambungkan SIGNALS -> VIEW -> LAYOUT -> PAINT -> SCHEDULER menjadi satu siklus hidup. ` +
    `Fakta terkini: Runtime::drain_dirty() tidak pernah dipanggil di luar modul signals; catatan milestone arena-tree ` +
    `menyebut seam ini sengaja ditinggalkan ("yang memanggil reconcile_children untuk subtree itu belum ada").\n\n` +
    `Yang harus dibangun:\n` +
    `1. Struktur pemilik siklus (misal AppRuntime/UiHost di crates/core) yang memegang: Runtime signals, ViewNode akar + closure pembangunnya, ` +
    `RenderTree, dan FrameScheduler.\n` +
    `2. Alur satu frame: drain_dirty() -> untuk tiap scope kotor rebuild view-nya -> diff terhadap view lama -> terapkan ke render tree ` +
    `(buat/pakai-ulang/hapus node) -> flush_layout() -> paint() -> Scene. Hormati kontrak drain_dirty dari milestone signals: ` +
    `rebuild sebuah scope WAJIB memasuki kembali setiap anak yang dipertahankan.\n` +
    `3. Sambungkan on_wake(Dirty) signals ke FrameScheduler sehingga perubahan signal menjadwalkan frame, dan idle benar-benar nol kerja.\n` +
    `4. API publik gaya Dart untuk menjalankan aplikasi, misal: run_app(window_config, |cx| view_tree) — dipakai crates/platform.\n` +
    `5. Sambungkan ke crates/platform: on_frame menghasilkan Scene dari siklus ini, bukan Scene yang disusun manual.\n\n` +
    `VERIFIKASI WAJIB (headless, tanpa GPU): test yang (a) membuat app dengan use_signal counter, (b) merender frame pertama dan memeriksa Scene, ` +
    `(c) mengubah signal, (d) memastikan HANYA subtree terkait yang di-rebuild (pakai DiffStats) dan Scene berubah sesuai, ` +
    `(e) memastikan tanpa perubahan signal tidak ada frame terjadwal (idle = nol).`))

  report.fase1.push(await milestone('Fase 1b - Jahitan', 'demo-end-to-end',
    `Bukti hidup bahwa seluruh rantai bekerja: di examples/gallery buat halaman "counter" yang benar-benar memakai API publik framework — ` +
    `use_signal, view tree gaya Dart (column/row), teks yang tampil, dan tombol sederhana (boleh quad + hit-test dari modul input Fase 1) ` +
    `yang menaikkan counter sehingga ANGKA DI LAYAR BERUBAH. ` +
    `Wajib memakai jalur resmi (run_app / siklus dari milestone reactive-glue), bukan menyusun Scene manual. ` +
    `VERIFIKASI: (1) jalankan binary-nya sungguhan dan pastikan window tampil dengan teks terbaca; ` +
    `(2) test headless yang mensimulasikan klik lewat lapisan input, lalu membuktikan lewat piksel offscreen bahwa tampilan berubah ` +
    `(bandingkan hitungan piksel teks sebelum vs sesudah, atau hash region angka). Ini uji integrasi permanen paling berharga di repo.`))
}

// ============================================================
// Fase 2 — Design system & komponen
// ============================================================
if (wantPhase('fase2')) {
  phase('Fase 2 - Design System')

  // Prasyarat visual: tanpa ini scroll_view/list/table akan BOCOR keluar viewport.
  // Utang dari milestone paint-pass (Fase 1b): core sudah menerbitkan
  // Command::PushClip/PopClip, tapi renderer melewatinya diam-diam
  // (`Command::PushClip(_) | Command::PopClip => {}` di instance.rs).
  report.fase2.push(await milestone('Fase 2 - Design System', 'clip-gpu',
    `Di crates/renderer: EKSEKUSI Command::PushClip(Rect)/PopClip yang saat ini dilewati diam-diam di fill_instances ` +
    `(baris \`Command::PushClip(_) | Command::PopClip => {}\`). Akibat sekarang: konten yang terpotong SEBAGIAN oleh viewport ` +
    `tetap tergambar utuh di GPU — scroll view, list, dan table di fase ini akan terlihat bocor.\n\n` +
    `Yang harus dibangun:\n` +
    `1. Terjemahkan pasangan PushClip/PopClip menjadi scissor rect GPU. Karena seluruh scene saat ini satu draw call, ` +
    `pecah menjadi daftar batch (scissor_rect, rentang instance) dan set_scissor_rect per batch — pertahankan urutan gambar, ` +
    `jangan mengorbankan batching lebih dari yang perlu (batch baru hanya saat clip berubah).\n` +
    `2. Clip bersarang sudah diiriskan di CPU oleh core, jadi renderer TIDAK perlu tumpukan sendiri — cukup rect efektif per batch. ` +
    `Verifikasi asumsi ini terhadap kode core sebelum menyederhanakan.\n` +
    `3. Konversi koordinat: clip datang dalam poin logis, scissor butuh piksel fisik — pakai jalur konversi SurfaceGeometry yang sudah ada, ` +
    `bulatkan ke luar (jangan sampai memotong satu piksel tepi konten), dan jepit ke ukuran surface (scissor di luar batas = validation error wgpu).\n` +
    `4. Rect kosong/negatif = lewati batch-nya sama sekali.\n` +
    `5. Berlaku untuk WindowSurface maupun OffscreenTarget (uji piksel butuh yang kedua).\n\n` +
    `VERIFIKASI PIKSEL WAJIB (tambahkan ke crates/renderer/tests/): (a) kotak besar di dalam PushClip kecil — piksel HANYA muncul di dalam ` +
    `rect clip, dan NOL di luar rect padahal geometri quad-nya melampaui; (b) teks yang terpotong separuh oleh clip — baris atas terlihat, ` +
    `baris bawah tidak; (c) setelah PopClip, gambar berikutnya kembali tidak terpotong; (d) clip bersarang menghasilkan irisan yang benar; ` +
    `(e) clip di luar surface tidak menyebabkan panic/validation error.`))

  report.fase2.push(await milestone('Fase 2 - Design System', 'tokens-preset',
    `Di crates/theme: arsitektur token semantik (surface, accent, radius_md, shadow_md, skala spacing 4pt, skala font) ` +
    `+ dua preset lengkap sesuai tabel REKOMENDASI §2.7 (Cupertino: squircle, palet HIG, Inter optical-size; ` +
    `Tailwind: arc 8px, palet slate/blue 50-950). Dark mode untuk keduanya. Utility resolve lewat theme aktif.`))
  report.fase2.push(await milestone('Fase 2 - Design System', 'overlay-system',
    `Di crates/widgets: infrastruktur overlay SEKALI untuk semua (KOMPONEN.md aturan #3): layer di atas konten, ` +
    `anchor + auto-flip di tepi, backdrop, dismiss (klik luar/Esc), transisi spring. Dialog/popover/tooltip/menu/toast akan menumpang ini.`))

  // Daftar komponen bisa dibatasi via args.components (array nama).
  const DEFAULT_COMPONENTS = [
    'button', 'checkbox', 'switch', 'slider', 'select',
    'text_field', 'scroll_view', 'list', 'tabs', 'dialog',
  ]
  const components = Array.isArray(ARGS.components) ? ARGS.components : DEFAULT_COMPONENTS
  log(`Fase 2: ${components.length} komponen dasar (${components.join(', ')}), lalu table + chart untuk flagship finance.`)

  const r2 = await pipeline(components, name => milestone('Fase 2 - Design System', `widget:${name}`,
    `Di crates/widgets: implementasikan komponen "${name}" sesuai spesifikasinya di KOMPONEN.md ` +
    `(termasuk catatan khususnya) dan Definition of Done di bagian bawah file itu: ` +
    `benar di kedua preset, semua state interaktif dengan spring, keyboard + focus ring, node AccessKit, dark mode, hit target 44pt, reduced-motion. ` +
    `Khusus text_field: caret/selection per grapheme + IME preedit inline adalah bagian scope, jangan dipangkas. ` +
    `Tambahkan halaman demo komponen ini di examples/gallery.`))
  report.fase2.push(...r2.filter(Boolean))

  // --- Komponen berat untuk flagship finance (keputusan #6) ---
  // Dijalankan SETELAH pipeline di atas, bukan paralel: table menumpang virtualisasi `list`
  // (KOMPONEN.md aturan urutan #4) dan chart menumpang overlay system untuk tooltip.
  report.fase2.push(await milestone('Fase 2 - Design System', 'widget:table',
    `Di crates/widgets: komponen "table" TERVIRTUALISASI sesuai KOMPONEN.md Tier 5 — WAJIB memakai ulang ` +
    `infrastruktur virtualisasi dari komponen list yang sudah ada, JANGAN membuat sistem virtualisasi kedua. ` +
    `Scope: sort per kolom, resize + reorder kolom (drag), seleksi baris (single/multi + shift/cmd), sticky header, ` +
    `lebar kolom auto vs fixed, sel kustom (render widget di dalam sel), empty state. ` +
    `Definition of Done penuh dari KOMPONEN.md berlaku (dua preset, spring, keyboard nav antar sel/baris, node AccessKit ` +
    `dengan role table/row/cell, dark mode, reduced-motion). Uji performa dengan 100k baris dummy — scroll harus tetap mulus. ` +
    `Tambahkan halaman demo di examples/gallery.`))

  report.fase2.push(await milestone('Fase 2 - Design System', 'chart-lib',
    `Buat crate BARU crates/chart (tambahkan ke members workspace) sebagai library chart untuk framework ini. ` +
    `Alasan crate terpisah: menjaga crates/widgets tetap ramping, tapi chart tetap TUNDUK pada kontrak yang sama — ` +
    `token semantik + dual preset (JANGAN hard-code warna/ukuran), abstraksi paint (JANGAN sentuh wgpu langsung), ` +
    `animasi via spring, dan node AccessKit. ` +
    `Scope v1 (minimal tapi lengkap untuk flagship finance): jenis line, bar (vertikal/horizontal, stacked/grouped), area, dan sparkline; ` +
    `elemen bersama: sumbu X/Y dengan tick + label, gridline, legend, tooltip saat hover (WAJIB menumpang overlay system Fase 2, ` +
    `jangan bikin popup sendiri), format angka/tanggal sadar locale, empty state, dan animasi transisi data (spring) saat dataset berubah. ` +
    `Sediakan juga palet kategorikal yang aman untuk color-blind dan konsisten di light/dark kedua preset. ` +
    `API mengikuti gaya Dart + method chaining seperti komponen lain, contoh bentuk: ` +
    `line_chart(data).x(|d| d.tanggal).y(|d| d.nilai).legend(true).animated(true). ` +
    `Unit test untuk logika skala/tick/format (bukan visual). Tambahkan halaman demo semua jenis chart di examples/gallery.`))
}

// ============================================================
// Fase 3 — Platform tail & infrastruktur kualitas
// ============================================================
if (wantPhase('fase3')) {
  phase('Fase 3 - Platform')
  const fase3Jobs = [
    ['native-p0',
      `Di crates/platform: integrasi native P0 sesuai INTEGRASI-NATIVE.md §1-§2: menubar via muda (dengan Edit menu standar macOS), ` +
      `dialog via rfd, clipboard via arboard, tray via tray-icon, custom titlebar macOS + vibrancy via window-vibrancy.`],
    ['lifecycle',
      `Di crates/platform: INTEGRASI-NATIVE.md §6 — dark mode live (token reaktif), accent color OS, reduced-motion/transparency, ` +
      `restorasi posisi window, handler quit yang menyimpan state.`],
    ['escape-hatch',
      `Di crates/platform: kontrak escape hatch §8 — window.raw_handle() -> RawWindowHandle, re-export objc2/windows-rs dengan versi terkunci, ` +
      `modul platform:: di API publik, hook event native mentah. Sertakan contoh pemakaian di docs.`],
    ['testing-infra',
      `REKOMENDASI §9.5: golden/snapshot test visual (render headless ke texture, bandingkan per preset), ` +
      `simulasi input di test, benchmark frame-time dengan ambang gagal, GitHub Actions matrix macOS/Windows/Linux.`],
    ['gallery-app',
      `Jadikan examples/gallery aplikasi gallery sungguhan (§9.9): daftar semua komponen dengan variasinya, ` +
      `switcher preset Cupertino/Tailwind + dark mode live, halaman animasi spring interaktif. Ini alat QA visual utama.`],
  ]
  const r3 = await pipeline(fase3Jobs, j => milestone('Fase 3 - Platform', j[0], j[1]))
  report.fase3.push(...r3.filter(Boolean))

  report.fase3.push(await milestone('Fase 3 - Platform', 'audit-akhir',
    `Audit lintas-fase: baca ulang ${DOCS} lalu periksa workspace — kontrak mana yang dilanggar (widget menyentuh wgpu? ` +
    `komponen tanpa node AccessKit? angka hard-code yang seharusnya token?), utang teknis dari notes milestone sebelumnya, ` +
    `dan komponen KOMPONEN.md Tier 0-4 yang belum ada. Tulis hasilnya ke ${ROOT}/AUDIT.md tanpa mengubah kode.`))
}

// ============================================================
// Fase 4 — Dokumentasi & contoh (REKOMENDASI §9.9)
// ============================================================
if (wantPhase('fase4')) {
  phase('Fase 4 - Docs & Contoh')
  const fase4Jobs = [
    ['api-docs',
      `API docs via rustdoc: doc-comment lengkap untuk seluruh API publik semua crate (dengan contoh kode yang di-doctest), ` +
      `#![warn(missing_docs)] di tiap crate, README per crate, dan pastikan \`cargo doc --workspace --no-deps\` bersih tanpa warning.`],
    ['tutorial',
      `Tulis ${ROOT}/docs/TUTORIAL.md — "aplikasi pertamamu": langkah demi langkah membuat app todo kecil dari nol ` +
      `(window -> layout -> signals -> styling -> preset -> build release). Setiap potongan kode harus benar-benar compile ` +
      `(uji sebagai example di examples/todo). Bahasa Indonesia, nada ramah pemula.`],
    ['contoh-apps',
      `Buat 3 contoh aplikasi kecil di examples/: (1) todo (dipakai tutorial), (2) settings — form + sidebar ala macOS System Settings, ` +
      `(3) dashboard-mini — chart dasar + table kecil + async fetch data dummy. Masing-masing < 300 baris, idiomatis, ` +
      `berfungsi sebagai dokumentasi hidup pola pemakaian.`],
    ['flagship-scaffold',
      `Scaffold aplikasi flagship (keputusan #6: tool bisnis/finance internal) sebagai repo/folder terpisah ${ROOT}/../rustui-flagship ` +
      `ATAU ${ROOT}/apps/flagship (pilih yang lebih rapi untuk workspace): struktur modul, navigasi sidebar, ` +
      `halaman placeholder (dashboard, tabel transaksi, form entri), koneksi async runtime. Belum fitur bisnis nyata — ` +
      `tujuannya jadi wahana dogfooding framework sejak dini.`],
  ]
  const r4 = await pipeline(fase4Jobs, j => milestone('Fase 4 - Docs & Contoh', j[0], j[1]))
  report.fase4.push(...r4.filter(Boolean))
}

// ============================================================
// Laporan akhir
// ============================================================
const flat = Object.entries(report).flatMap(([f, items]) => items.map(i => ({ fase: f, ...((typeof i === 'string') ? { summary: i } : i) })))
const gagal = flat.filter(i => i.ok === false)
log(`Selesai: ${flat.length} milestone, ${gagal.length} gagal.`)
return { report, gagal: gagal.map(g => g.key) }
