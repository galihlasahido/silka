//! # rustui-core
//!
//! Mesin framework: semua yang ada di bawah API publik bergaya Dart
//! (REKOMENDASI §2). Isi crate ini adalah **detail implementasi** — kontrak
//! yang dilihat penulis aplikasi hidup di `rustui-widgets`.
//!
//! Lapisan yang ditampung:
//!
//! - **Signals + rebuild per-komponen** (pola Dioxus 0.7, §2.5): perubahan
//!   signal menandai komponen yang membacanya sebagai dirty → rebuild subtree
//!   kecil itu → diff. Butuh scheduler dirty-marking + scope tracking.
//! - **View-diff → arena render tree** (§2): view tree ringan dibangun ulang
//!   tiap update dan di-diff ke retained tree berbasis arena/slotmap ber-ID.
//!   Arena dipilih karena AccessKit dan Taffy sama-sama berbasis ID.
//! - **Box constraints ala Flutter** sebagai protokol layout native
//!   ("constraints turun, ukuran naik"), single pass + relayout boundaries;
//!   Taffy dipakai untuk widget Flex/Grid, dengan measure function leaf
//!   menumpang `rustui-text` (§3.4). Layout harus paham **mirroring RTL**
//!   sejak awal — retrofit RTL semustahil retrofit a11y (§9.8).
//! - **Spring animation** (§3.5): nilai animasi menyimpan `(posisi, velocity)`
//!   dan **selalu interruptible/retargetable** — solusi closed-form damped
//!   harmonic oscillator, parameter perceptual (duration + bounce), preset
//!   `smooth`/`snappy`/`bouncy`. Wajib menghormati reduced-motion.
//! - **Input + hit-testing + velocity tracker** — velocity dibutuhkan untuk
//!   handoff gesture (fling → spring).
//! - **Scheduler**: render **hanya saat dirty**; vsync lewat display link per
//!   platform, jangan pernah hardcode 16.6 ms.
//!
//! **AccessKit adalah output first-class dari render tree** (§3.8), bukan
//! lapisan susulan: setiap node menyediakan role, name, bounds, dan actions.
//!
//! ## Yang sudah ada
//!
//! **Milestone `frame-scheduling`** — [`scheduler`]: mesin **render-on-dirty**
//! beserta pengukuran frame time. Murni logika: ia tidak tahu winit maupun
//! wgpu. Platform hanya menyuplai detak vsync dan interval terukurnya;
//! `rustui-platform` memakai `CADisplayLink` di macOS (ProMotion-aware) dan
//! `request_redraw` winit di OS lain. **Tidak ada 16,6 ms di mana pun** —
//! kalau interval belum diketahui, ia bernilai `None` dan tidak ada yang
//! berpura-pura tahu.
//!
//! **Milestone `signals`** — [`signals`]: runtime state pola Dioxus.
//! [`signals::use_signal`] untuk state lokal komponen, dependency tracking
//! per-scope, dirty marking + batching, dan identitas scope berbasis
//! [`signals::Key`] untuk list dinamis. Sambungannya ke [`scheduler`] hanya
//! satu baris ([`signals::Runtime::on_wake`]) sehingga janji "render hanya saat
//! dirty" tetap utuh: signal yang tidak dibaca komponen mana pun **tidak**
//! membangunkan GPU.
//!
//! **Milestone `arena-tree`** — [`tree`] dan [`view`]: retained render tree
//! berbasis arena ber-ID bergenerasi, protokol **box constraints ala Flutter**
//! ("constraints turun, ukuran naik, induk menentukan posisi"), cache layout
//! plus **relayout boundary**, dan di atasnya lapisan **view-diff**: view tree
//! ringan bergaya Dart dibangun ulang tiap rebuild lalu di-diff ke render tree
//! (§2). Identitas anak memakai [`signals::Key`] yang sama dengan scope
//! komponen, jadi hanya ada satu disiplin kunci di seluruh framework.
//! [`tree::RenderNode::access`] adalah bagian kontrak node sejak awal, dengan
//! `bounds` yang datang dari hasil layout (§3.8).
//!
//! Alur satu frame yang dirakit ketiganya:
//!
//! ```
//! use rustui_core::scheduler::FrameScheduler;
//! use rustui_core::tree::{BoxConstraints, RenderTree};
//! use rustui_core::view::{column, fixed, reconcile};
//! use rustui_paint::Size;
//!
//! let mut scheduler = FrameScheduler::new();
//! let mut tree = RenderTree::new();
//!
//! // 1. Komponen dibangun ulang → view baru → diff ke render tree.
//! reconcile(&mut tree, column([fixed(120.0, 24.0)]).spacing(8.0));
//! // 2. Apa yang berubah menentukan apakah renderer perlu dibangunkan.
//! scheduler.request(tree.take_dirty());
//! // 3. Layout: penuh saat ukuran window berubah, subtree saja selebihnya.
//! tree.perform_layout(BoxConstraints::tight(Size::new(320.0, 200.0)));
//! ```
//!
//! **Milestone `spring`** — [`animation`]: sistem animasi spring dengan solusi
//! **closed-form damped harmonic oscillator**. Nilai menyimpan
//! `(posisi, velocity)` ([`animation::SpringValue`]) sehingga **selalu
//! interruptible**: [`animation::SpringValue::set_target`] boleh dipanggil
//! kapan saja dan velocity ikut terbawa (WWDC23), yang sekaligus menjadi jalur
//! handoff gesture fling → spring. Parameternya perceptual (durasi + bounce)
//! dengan preset `smooth`/`snappy`/`bouncy`, dan **reduced-motion**
//! ([`animation::Motion`]) adalah bagian kontrak, bukan poles akhir.
//! Sambungannya ke [`scheduler`] mengikuti aturan yang sama seperti signal:
//! [`animation::AnimationDriver::end_frame`] hanya mengembalikan
//! [`Dirty::ANIMATION`] selama benar-benar ada yang bergerak — tidak ada timer
//! yang berdetak, dan begitu semua spring settle GPU kembali tidur.
//!
//! **Milestone `accesskit`** — [`access`]: emisi node aksesibilitas sebagai
//! **pass render tree**, sejajar layout dan paint (§3.8).
//! [`tree::RenderNode::access`] adalah method **wajib** — widget yang lupa
//! memikirkan screen reader tidak lolos compile — dan `bounds` tiap node
//! datang dari hasil layout, bukan dari widget, sehingga apa yang dibacakan
//! teknologi bantu tidak mungkin berbeda dari apa yang digambar.
//! [`access::AccessTree::dump`] memberi tree dump deterministik untuk golden
//! test, dan [`access::AccessTree::changes_since`] menjaga janji "hanya saat
//! dirty" tetap berlaku untuk screen reader juga. Konversi ke `accesskit`
//! terkurung di satu berkas; adapter winit-nya ada di `rustui-platform`.
//!
//! **Milestone `taffy-flex`** — [`tree::TaffyBox`]: Flexbox dan CSS Grid
//! dijalankan **Taffy sebagai widget di dalam protokol box constraints**
//! (§3.4). `row()`/`column()`/`grid()` bergaya Dart ([`view::row`],
//! [`view::column`], [`view::grid`]) dengan `.spacing()`/`.gap_*()` yang
//! terkunci ke skala 4pt ([`tree::SPACING_UNIT`], §2.6), `expanded()`/
//! `flexible()` sebagai padanan `Expanded`/`Flexible` Flutter, dan mirroring
//! RTL diteruskan apa adanya ke Taffy (§9.8). Nama `taffy::` tidak pernah
//! keluar dari satu modul: kosakata publiknya milik kita
//! ([`tree::ContainerStyle`], [`tree::ItemStyle`], [`tree::Track`]).
//! **Text measurement masuk lewat measure function leaf** —
//! [`tree::MeasuredBox`] (`view::measured`) adalah satu-satunya pintu, dipakai
//! sama persis oleh mesin box-constraints kita dan oleh Taffy.
//!
//! **Milestone `input-hittest`** — [`input`]: routing event pointer/keyboard,
//! hit-testing, fokus, velocity tracker, dan IME. Empat janji dokumen ditutup
//! di sini:
//!
//! 1. **Hit-testing sadar squircle** (§3.6) — [`input::HitShape::Rounded`]
//!    menguji superellipse yang **sama persis** dengan yang dikirim ke shader
//!    ([`rustui_paint::Corners::contains`]), jadi pojok yang terlihat kosong
//!    tidak bisa diklik dan sebaliknya. Viewport memotong isinya, sehingga
//!    baris yang sudah tergulir keluar tidak lagi bisa disentuh.
//! 2. **Fokus & tab-order** ([`input::FocusManager`]) dihitung dari render tree
//!    yang sama dengan layout dan a11y, lengkap dengan urutan eksplisit dan
//!    **focus scope** sebagai perangkap fokus dialog.
//! 3. **Velocity tracker** ([`input::VelocityTracker`]) — regresi kuadrat
//!    terkecil derajat dua ala Flutter; inilah pemasok `velocity` awal untuk
//!    [`animation::SpringValue::set_target`], yaitu handoff fling → spring yang
//!    dijanjikan §3.5.
//! 4. **IME** ([`input::ImeRequest`]) — preedit/commit hanya mengalir ke node
//!    terfokus, dan permintaan area caret mengalir balik ke shell sehingga
//!    jendela kandidat CJK berlabuh di tempat yang benar (§3.8).
//!
//! Kontraknya melekat di [`tree::RenderNode`] (`hit_shape`, `hit_behavior`,
//! `focus_policy`, `cursor`, `event`) sejajar dengan `access` — bukan lapisan
//! susulan. [`tree::Interactive`] (`view::interactive`) adalah node pertama yang
//! memakainya utuh, dan `rustui-platform` menerjemahkan winit ke kosakata ini
//! di satu berkas.
//!
//! **Milestone `paint-pass`** — [`tree::RenderTree::paint`]: penyusunan
//! [`rustui_paint::Scene`] dari render tree, pass ketiga yang sejajar dengan
//! layout dan a11y (§3.2). [`tree::RenderNode::paint`] adalah bagian kontrak
//! node, dan kosakatanya **hanya** `rustui-paint` — quad, shadow ganda, glyph
//! run: tidak ada satu tipe wgpu pun yang bisa sampai ke kode widget, jadi
//! backend baru (GL/CPU) nanti masuk di satu tempat. Empat sifatnya:
//!
//! 1. **Node menggambar dalam koordinat lokal** — cerminan aturan layout "node
//!    tidak pernah tahu posisinya sendiri"; [`tree::PaintCtx`] yang menaikkannya
//!    ke koordinat absolut, dan absolut itu sama persis dengan `bounds` a11y.
//! 2. **Induk sebelum anak**, sehingga urutan perintah = urutan tumpuk.
//! 3. **Clip** memakai jawaban [`tree::RenderNode::clips_children`] yang sudah
//!    dipakai hit-testing: satu jawaban untuk dua pass, jadi mustahil ada baris
//!    yang tergulir keluar layar tapi masih bisa diklik.
//! 4. **Render hanya saat dirty, sampai ke tingkat subtree** (§3.5): perintah
//!    gambar disimpan di relayout boundary, dan subtree bersih yang tidak
//!    bergeser tidak dijalankan ulang sama sekali.
//!
//! Warna tidak pernah datang dari mesin: [`tree::Decoration`] membawa nilai
//! yang **sudah diresolusi** dari token theme satu tingkat di atas, sehingga
//! preset Cupertino/Tailwind (§2.7) berganti tanpa satu baris pun berubah di
//! sini — termasuk geometri sudut, yang tetap parameter dan bukan konstanta.
//!
//! **Milestone `reactive-glue`** — [`mod@app`]: keenam lapisan di atas akhirnya
//! **tersambung menjadi satu siklus hidup**. [`app::AppRuntime`] memegang
//! runtime signals, closure pembangun view akar, render tree, dan
//! [`scheduler::FrameScheduler`]; [`app::AppRuntime::frame`] menjalankan satu
//! putaran penuh:
//!
//! ```text
//! signals::Runtime::drain_dirty()          ← scope yang harus dibangun ulang
//!   → jalankan ulang closure-nya DI DALAM scope itu
//!   → view::reconcile_children(tree, jangkar, [view baru])
//!   → tree::RenderTree::perform_layout(constraints window)
//!   → tree::RenderTree::paint_into(scene)
//! ```
//!
//! Dua sambungan yang sebelumnya menganga ditutup di sini.
//! [`signals::Runtime::drain_dirty`] akhirnya punya pemanggil, dan pemanggil
//! itu memenuhi kontraknya apa adanya: [`app::component`] membangun isinya
//! secara *eager* di dalam [`signals::scope`], sehingga membangun ulang sebuah
//! scope **memasuki kembali setiap anak yang dipertahankan** — syarat yang
//! membuat pemangkasan keturunan pada daftar dirty sah. Dan
//! [`signals::Runtime::on_wake`] disambungkan langsung ke
//! [`scheduler::FrameScheduler::request`], jadi janji §3.5 berlaku
//! ujung-ke-ujung: tulisan signal menjadwalkan tepat satu frame, dan begitu
//! frame itu selesai [`app::AppRuntime::is_idle`] kembali benar tanpa satu pun
//! timer yang berdetak.
//!
//! Setiap komponen punya **node jangkar** ([`app::ComponentBox`]) — transparan
//! bagi layout dan disaring keluar dari pohon a11y — karena tanpa itu satu
//! satunya cara menerapkan hasil rebuild adalah mendiff dari akar, dan
//! "rebuild per-komponen" tinggal nama.
//!
//! **Milestone `demo-end-to-end`** — tiga potong terakhir yang membuat rantai
//! itu bisa **dilihat dan disentuh**, bukan sekadar diuji:
//!
//! 1. [`Callback`] + [`tree::Interactive::on_press`] — aksi yang dititipkan
//!    aplikasi ke sebuah node. Inilah `on_press` gaya Dart yang dijanjikan
//!    §2.5, dan penutup jalur `klik → signal → rebuild`: sebelum ini node
//!    interaktif hanya bisa **menghitung** aktivasi, tidak bisa menceritakannya
//!    kepada siapa pun.
//! 2. **Tampilan per state** ([`tree::Interactive::decoration`],
//!    `hover_background`, `press_background`, [`tree::FocusRing`]) — nilainya
//!    sudah diresolusi dari token satu tingkat di atas (§2.6), dan bentuk
//!    sudutnya dijamin sama dengan bentuk yang diuji hit-test karena keduanya
//!    membaca [`tree::Interactive::corners`] yang sama (§3.6).
//! 3. [`app::ScaleFactor`] sebagai titipan [`app::Env`] standar — teks harus
//!    dirasterisasi pada resolusi layar yang sebenarnya (§3.3), dan window yang
//!    pindah monitor hanya membangun ulang komponen yang membacanya.
//!
//! Buktinya hidup di `examples/gallery` halaman `counter`: satu klik yang
//! disimulasikan lewat lapisan input berakhir sebagai piksel berbeda pada
//! tekstur yang dirender GPU.
//!
//! [`rustui_paint::Command::PushClip`] sudah dieksekusi backend sebagai scissor
//! rect per rentang instance, jadi kontrak clip pass ini berlaku sampai ke
//! piksel.
//!
//! Yang belum ada dan menjadi sambungan berikutnya:
//! repaint boundary berbasis layer/offscreen, dan sambungan
//! [`animation::AnimationDriver`] ke [`app::AppRuntime::frame`] (sekarang
//! spring masih dikemudikan aplikasi lewat `request_animation_frame`).

#![warn(missing_docs)]

pub mod access;
pub mod animation;
pub mod app;
mod callback;
pub mod input;
pub mod scheduler;
pub mod signals;
pub mod tree;
pub mod view;

pub use access::{
    AccessAction, AccessActionRequest, AccessActions, AccessEntry, AccessNode, AccessRole,
    AccessToggled, AccessTree, AccessUpdate,
};
pub use animation::{
    Animatable, AnimationDriver, Motion, MotionRole, Propagator, Spring, SpringValue, Tick,
    Tolerance,
};
pub use app::{app, component, AppRuntime, BuildCtx, Env, FrameReport, ScaleFactor};
pub use callback::Callback;
pub use input::{
    hit_test, CursorIcon, Event, EventCtx, FocusDirection, FocusManager, FocusPolicy, HitBehavior,
    HitShape, ImeEvent, ImeRequest, InputResponse, InputRouter, KeyCode, KeyEvent, Modifiers,
    NamedKey, PointerButton, PointerEvent, PointerPhase, ScrollEvent, Velocity, VelocityTracker,
};
pub use scheduler::{
    ClockSource, Dirty, FrameLogger, FrameScheduler, FrameStart, FrameStats, FrameTiming,
    RefreshEstimator, Vsync, Wake,
};
pub use signals::{
    current_scope, list, scope, untracked, use_signal, Key, Runtime, ScopeId, Signal, SignalId,
};
pub use tree::{
    BoxConstraints, ContainerStyle, CrossAlign, Decoration, ItemStyle, LayoutCtx, MainAlign,
    NodeId, PaintCtx, RenderNode, RenderTree, TextDirection, Track,
};
pub use view::{reconcile, DiffStats, View, ViewNode};
