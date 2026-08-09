//! Routing event: dari satu event mentah menjadi node-node yang menanganinya.
//!
//! Empat aturan yang menentukan seluruh isi modul ini:
//!
//! 1. **Penunjuk mengikuti geometri** — rutenya adalah jalur hit-test
//!    ([`super::hit_test`]), dari node terdalam ke akar, dan berhenti di node
//!    pertama yang menyatakan event sudah ditangani.
//! 2. **Keyboard mengikuti fokus** — rutenya jalur fokus ke akar, sehingga
//!    pintasan di tingkat window tetap kebagian setelah widget menolak.
//! 3. **Tekanan menangkap penunjuk** — begitu sebuah node menekan tombol dan
//!    meminta capture, seluruh gerakan sampai tombol dilepas pergi ke node itu
//!    walau kursor sudah keluar dari kotaknya. Tanpa ini, slider yang di-drag
//!    cepat akan lepas di tengah jalan.
//! 4. **IME milik yang fokus** — preedit/commit hanya dikirim ke node terfokus,
//!    dan permintaan `set_ime_cursor_area` mengalir balik ke platform lewat
//!    [`Response`] (REKOMENDASI §3.8).
//!
//! Node **tidak** boleh mengubah struktur pohon dari dalam handler event: yang
//! tersedia hanyalah mengubah dirinya sendiri dan menitipkan permintaan lewat
//! [`EventCtx`]. Struktur hanya berubah lewat view-diff (§2) — itulah yang
//! menjaga arena tetap konsisten walau event datang di tengah frame.

use std::collections::HashMap;
use std::time::Duration;

use rustui_paint::{Point, Rect, Size};

use crate::scheduler::Dirty;
use crate::tree::{NodeId, RenderTree};

use super::event::{
    Event, FocusEvent, ImeEvent, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent,
    PointerId, PointerPhase, ScrollEvent,
};
use super::focus::{FocusChange, FocusDirection, FocusManager};
use super::hit::{hit_test, HitEntry, HitTestResult};
use super::velocity::{Velocity, VelocityTracker};

// ---------------------------------------------------------------------------
// Kursor
// ---------------------------------------------------------------------------

/// Bentuk kursor yang diminta sebuah node.
///
/// Kosakata sendiri, dipetakan ke `winit::window::CursorIcon` di
/// `rustui-platform` — alasan yang sama dengan seluruh modul input: kode widget
/// tidak menyentuh tipe pustaka luar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum CursorIcon {
    /// Panah biasa.
    #[default]
    Default,
    /// Tangan menunjuk (tautan, tombol di web-style).
    Pointer,
    /// Caret teks.
    Text,
    /// Sedang menunggu.
    Wait,
    /// Bisa digenggam (scroll pan, drag handle).
    Grab,
    /// Sedang digenggam.
    Grabbing,
    /// Ubah ukuran horizontal (split view vertikal).
    ResizeHorizontal,
    /// Ubah ukuran vertikal.
    ResizeVertical,
    /// Aksi tidak diizinkan.
    NotAllowed,
}

// ---------------------------------------------------------------------------
// Permintaan IME
// ---------------------------------------------------------------------------

/// Permintaan ke shell platform terkait IME.
///
/// Diterjemahkan `rustui-platform` menjadi `set_ime_allowed` +
/// `set_ime_cursor_area` — dua panggilan winit yang menentukan apakah jendela
/// kandidat CJK muncul di tempat yang benar (REKOMENDASI §3.8).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImeRequest {
    /// Nyalakan IME dan taruh area kandidat di `area` (poin logis, global).
    Enable {
        /// Kotak caret/preedit yang menjadi jangkar jendela kandidat.
        area: Rect,
    },
    /// IME sudah menyala; hanya areanya yang berpindah (caret bergerak).
    Update {
        /// Kotak caret yang baru.
        area: Rect,
    },
    /// Matikan IME — tidak ada lagi yang bisa menerima teks.
    Disable,
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// Hasil satu dispatch: apa yang harus dilakukan shell setelahnya.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Response {
    /// Alasan frame berikutnya dibutuhkan — langsung disambungkan ke
    /// [`crate::scheduler::FrameScheduler::request`]. Kosong = tidak ada yang
    /// perlu digambar, dan window tetap benar-benar idle (§3.5).
    pub dirty: Dirty,
    /// Benar bila ada node yang menyatakan event ini miliknya.
    pub handled: bool,
    /// Perpindahan fokus yang terjadi.
    pub focus: FocusChange,
    /// Permintaan IME untuk shell.
    pub ime: Option<ImeRequest>,
    /// Bentuk kursor baru (hanya diisi saat berubah).
    pub cursor: Option<CursorIcon>,
}

impl Response {
    /// Benar bila dispatch ini tidak berdampak apa pun.
    pub fn is_noop(&self) -> bool {
        self.dirty.is_empty()
            && !self.handled
            && !self.focus.changed()
            && self.ime.is_none()
            && self.cursor.is_none()
    }
}

// ---------------------------------------------------------------------------
// EventCtx
// ---------------------------------------------------------------------------

/// Apa yang dititipkan node lewat [`EventCtx`], dikumpulkan sepanjang satu
/// dispatch lalu diterapkan sekali di akhir.
#[derive(Debug, Default)]
struct Sink {
    dirty: Dirty,
    /// `Some(Some(n))` = minta fokus ke n, `Some(None)` = lepas fokus.
    focus: Option<Option<NodeId>>,
    /// `Some(Some(n))` = tangkap penunjuk untuk n, `Some(None)` = lepaskan.
    capture: Option<Option<NodeId>>,
    ime: Option<(NodeId, Option<Rect>)>,
}

/// Akses terbatas ke dunia luar selama sebuah node menangani event.
///
/// Sengaja **tidak** memuat `&mut RenderTree`: node hanya boleh mengubah
/// dirinya sendiri (lewat `&mut self`) dan menitipkan permintaan di sini.
/// Konsekuensinya struktur pohon tidak mungkin berubah di tengah dispatch, dan
/// tidak ada re-entrancy yang perlu dijaga.
pub struct EventCtx<'a> {
    node: NodeId,
    local: Point,
    size: Size,
    bounds: Rect,
    focused: bool,
    handled: &'a mut bool,
    sink: &'a mut Sink,
}

impl EventCtx<'_> {
    /// Node yang sedang menangani event.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// Posisi event dalam koordinat lokal node (poin logis).
    ///
    /// Untuk event tanpa posisi (keyboard, IME, fokus) nilainya
    /// [`Point::ZERO`].
    pub fn local(&self) -> Point {
        self.local
    }

    /// Ukuran node hasil layout terakhir.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Kotak global node — dipakai menghitung area caret untuk IME.
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Benar bila node ini sedang memegang fokus keyboard.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Nyatakan event sudah ditangani: penyampaian ke leluhur berhenti.
    pub fn handled(&mut self) {
        *self.handled = true;
    }

    /// Benar bila event sudah ditangani node yang lebih dalam.
    pub fn is_handled(&self) -> bool {
        *self.handled
    }

    /// Minta node digambar ulang (hover, pressed, focus ring).
    pub fn request_paint(&mut self) {
        self.sink.dirty |= Dirty::PAINT;
    }

    /// Minta layout ulang (mis. posisi guliran berubah).
    pub fn request_layout(&mut self) {
        self.sink.dirty |= Dirty::LAYOUT | Dirty::PAINT;
    }

    /// Minta frame berikutnya karena ada animasi berjalan (spring).
    pub fn request_animation(&mut self) {
        self.sink.dirty |= Dirty::ANIMATION;
    }

    /// Minta fokus keyboard pindah ke node ini.
    pub fn request_focus(&mut self) {
        self.sink.focus = Some(Some(self.node));
    }

    /// Lepaskan fokus dari siapa pun yang memegangnya.
    pub fn release_focus(&mut self) {
        self.sink.focus = Some(None);
    }

    /// Tangkap penunjuk: seluruh gerakan sampai tombol dilepas datang ke sini.
    pub fn capture_pointer(&mut self) {
        self.sink.capture = Some(Some(self.node));
    }

    /// Lepaskan tangkapan penunjuk.
    pub fn release_pointer(&mut self) {
        self.sink.capture = Some(None);
    }

    /// Minta IME menyala dengan area kandidat `area` (koordinat global).
    ///
    /// Dipanggil widget teks saat mendapat fokus dan setiap kali caret pindah.
    pub fn request_ime(&mut self, area: Rect) {
        self.sink.ime = Some((self.node, Some(area)));
    }

    /// Matikan IME (widget teks kehilangan fokus).
    pub fn disable_ime(&mut self) {
        self.sink.ime = Some((self.node, None));
    }
}

// ---------------------------------------------------------------------------
// Konfigurasi klik beruntun
// ---------------------------------------------------------------------------

/// Ambang klik beruntun (ganda/tripel).
///
/// Angkanya milik framework, bukan platform: tiga OS melaporkannya dengan cara
/// berbeda (dan Wayland tidak sama sekali), sementara pengguna mengharapkan
/// perilaku yang sama di semuanya.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClickConfig {
    /// Jeda maksimum antar klik.
    pub interval: Duration,
    /// Pergeseran maksimum antar klik, poin logis.
    pub distance: f32,
}

impl Default for ClickConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(500),
            distance: 4.0,
        }
    }
}

// ---------------------------------------------------------------------------
// State per penunjuk
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct PointerState {
    position: Point,
    /// Jalur node yang sedang di-hover, terdalam lebih dulu.
    hover: Vec<NodeId>,
    capture: Option<NodeId>,
    velocity: VelocityTracker,
    last_click: Option<(PointerButton, Point, Duration)>,
    click_count: u32,
}

// ---------------------------------------------------------------------------
// InputRouter
// ---------------------------------------------------------------------------

/// Penyalur event untuk satu render tree (satu window).
///
/// Menyimpan yang memang harus diingat antar-event: modifier terakhir, tombol
/// yang ditahan, jalur hover, capture, velocity per penunjuk, fokus, dan state
/// IME. Segala hal yang bisa dibaca ulang dari pohon **tidak** disimpan.
#[derive(Debug, Default)]
pub struct InputRouter {
    modifiers: Modifiers,
    pointers: HashMap<PointerId, PointerState>,
    focus: FocusManager,
    click: ClickConfig,
    cursor: CursorIcon,
    /// Node yang sedang memiliki sesi IME, beserta area caret terakhir.
    ime: Option<(NodeId, Rect)>,
}

impl InputRouter {
    /// Router baru tanpa fokus, tanpa hover, tanpa capture.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ambang klik beruntun.
    pub fn click_config(&self) -> ClickConfig {
        self.click
    }

    /// Ganti ambang klik beruntun (mis. mengikuti setting OS).
    pub fn set_click_config(&mut self, config: ClickConfig) {
        self.click = config;
    }

    /// Modifier keyboard terakhir yang diketahui.
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Perbarui modifier tanpa mengirim event (winit melaporkannya terpisah).
    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    /// Pemegang fokus keyboard.
    pub fn focus(&self) -> &FocusManager {
        &self.focus
    }

    /// Kursor yang berlaku sekarang.
    pub fn cursor(&self) -> CursorIcon {
        self.cursor
    }

    /// Node yang sedang menangkap penunjuk `id`.
    pub fn capture_of(&self, id: PointerId) -> Option<NodeId> {
        self.pointers.get(&id).and_then(|p| p.capture)
    }

    /// Jalur hover penunjuk `id`, terdalam lebih dulu.
    pub fn hover_of(&self, id: PointerId) -> &[NodeId] {
        self.pointers
            .get(&id)
            .map(|p| p.hover.as_slice())
            .unwrap_or(&[])
    }

    /// Kecepatan penunjuk `id` saat ini — inilah nilai yang diserahkan ke
    /// spring saat gesture dilepas (fling → spring, §3.5).
    pub fn velocity(&self, id: PointerId) -> Velocity {
        self.pointers
            .get(&id)
            .map(|p| p.velocity.velocity())
            .unwrap_or(Velocity::ZERO)
    }

    /// Fokuskan node tertentu dari luar (mis. setelah dialog terbuka).
    pub fn focus_node(&mut self, tree: &mut RenderTree, node: Option<NodeId>) -> Response {
        let mut out = Response::default();
        let change = self.focus.focus(tree, node);
        self.terapkan_fokus(tree, change, &mut out);
        out
    }

    /// Pindahkan fokus satu langkah (Tab / Shift+Tab yang dipicu program).
    pub fn move_focus(&mut self, tree: &mut RenderTree, direction: FocusDirection) -> Response {
        let mut out = Response::default();
        let change = self.focus.move_focus(tree, direction);
        self.terapkan_fokus(tree, change, &mut out);
        out
    }

    /// Selaraskan state input dengan pohon setelah view-diff.
    ///
    /// Node bisa lenyap kapan saja; fokus, capture, hover, dan sesi IME yang
    /// menunjuk kuburan harus dibersihkan **sebelum** event berikutnya datang —
    /// kalau tidak, keyboard diam total dan jendela kandidat IME menggantung di
    /// tempat yang salah.
    pub fn sync(&mut self, tree: &mut RenderTree) -> Response {
        let mut out = Response::default();
        // Node yang **masih hidup** tapi berhenti focusable (mis. tombol baru
        // saja di-disable) tetap diberi tahu lewat `Focus::Lost`; yang sudah
        // lenyap tidak bisa, dan `kirim_satu` melewatinya dengan tenang.
        let change = self.focus.prune(tree);
        self.terapkan_fokus(tree, change, &mut out);
        for state in self.pointers.values_mut() {
            if let Some(cap) = state.capture {
                if !tree.contains(cap) {
                    state.capture = None;
                }
            }
            state.hover.retain(|n| tree.contains(*n));
        }
        if let Some((owner, _)) = self.ime {
            if !tree.contains(owner) || !self.focus.is_focused(owner) {
                self.ime = None;
                out.ime = Some(ImeRequest::Disable);
            }
        }
        out
    }

    /// Salurkan satu event ke pohon.
    pub fn dispatch(&mut self, tree: &mut RenderTree, event: &Event) -> Response {
        match event {
            Event::Pointer(e) => self.pointer(tree, e),
            Event::Scroll(e) => self.scroll(tree, e),
            Event::Key(e) => self.key(tree, e),
            Event::Ime(e) => self.ime_event(tree, e),
            // Event fokus lahir di router, tidak pernah disuntikkan dari luar.
            Event::Focus(_) => Response::default(),
        }
    }

    // -- penunjuk ---------------------------------------------------------

    fn pointer(&mut self, tree: &mut RenderTree, event: &PointerEvent) -> Response {
        self.modifiers = event.modifiers;
        let mut out = Response::default();
        let click = self.click;

        // Riwayat gerak: dasar velocity tracker untuk handoff ke spring.
        {
            let state = self.pointers.entry(event.id).or_default();
            state.position = event.position;
            match event.phase {
                PointerPhase::Down => {
                    state.velocity.reset();
                    state.velocity.add(event.time, event.position);
                    state.click_count = hitung_klik(state, event, click);
                    state.last_click = event.button.map(|b| (b, event.position, event.time));
                }
                PointerPhase::Move | PointerPhase::Enter => {
                    state.velocity.add(event.time, event.position)
                }
                PointerPhase::Up => state.velocity.add(event.time, event.position),
                PointerPhase::Cancel | PointerPhase::Leave => state.velocity.reset(),
            }
        }

        // Hover: dihitung dari geometri, bukan dari capture — tombol yang
        // ditekan lalu ditarik keluar memang harus berhenti terlihat hover.
        let hit = if event.phase == PointerPhase::Leave {
            HitTestResult::new()
        } else {
            hit_test(tree, event.position)
        };
        self.perbarui_hover(tree, event, &hit, &mut out);

        if event.phase == PointerPhase::Leave {
            return out;
        }

        let mut event = event.clone();
        event.click_count = self.pointers.get(&event.id).map_or(0, |s| s.click_count);

        let rute = match self.capture_of(event.id) {
            Some(node) if tree.contains(node) => rute_dari_node(tree, node, event.position),
            _ => hit.path().to_vec(),
        };

        let mut sink = Sink::default();
        let handled = self.kirim(tree, &rute, &Event::Pointer(event.clone()), &mut sink);
        out.handled = handled;

        // Tombol dilepas/dibatalkan selalu mengakhiri capture, apa pun kata
        // node — kalau tidak, penunjuk bisa tersangkut selamanya di node yang
        // lupa melepaskannya.
        if matches!(event.phase, PointerPhase::Up | PointerPhase::Cancel)
            && sink.capture.is_none()
            && event.buttons.is_empty()
        {
            sink.capture = Some(None);
        }
        self.terapkan(tree, sink, Some(event.id), &mut out);

        // Kursor ditanya **setelah** event sampai ke node, bukan sebelumnya:
        // node yang bentuk kursornya bergantung pada posisi penunjuk di dalam
        // dirinya sendiri (pegangan resize kolom `table`, nanti `split_view`)
        // baru tahu jawabannya setelah menerima gerakan itu. Menanyakannya
        // lebih dulu berarti kursor panah tetap panah tepat di atas pegangan
        // yang bisa diseret — dan pengguna tidak pernah menemukan bahwa ia ada.
        self.perbarui_kursor(tree, event.id, &mut out);
        out
    }

    fn perbarui_hover(
        &mut self,
        tree: &mut RenderTree,
        event: &PointerEvent,
        hit: &HitTestResult,
        out: &mut Response,
    ) {
        let baru: Vec<NodeId> = hit.nodes().collect();
        let lama = std::mem::take(&mut self.pointers.entry(event.id).or_default().hover);
        if lama == baru {
            self.pointers.entry(event.id).or_default().hover = baru;
            return;
        }

        let mut sink = Sink::default();
        for node in lama.iter().filter(|n| !baru.contains(n)) {
            let mut e = event.clone();
            e.phase = PointerPhase::Leave;
            // Koordinat lokal tetap bermakna walau titiknya sudah di luar node
            // — widget yang menghitung "keluar lewat sisi mana" butuh itu.
            let origin = tree.global_offset(*node);
            let local = Point::new(e.position.x - origin.x, e.position.y - origin.y);
            self.kirim_satu(tree, *node, local, &Event::Pointer(e), &mut sink);
        }
        for entry in hit.path().iter().filter(|e| !lama.contains(&e.node)) {
            let mut e = event.clone();
            e.phase = PointerPhase::Enter;
            self.kirim_satu(tree, entry.node, entry.local, &Event::Pointer(e), &mut sink);
        }
        self.pointers.entry(event.id).or_default().hover = baru;
        self.terapkan(tree, sink, Some(event.id), out);
        self.perbarui_kursor(tree, event.id, out);
    }

    /// Tanyakan ulang bentuk kursor ke rantai hover, dan laporkan bila berubah.
    ///
    /// Kursor **ditanya** ke node, tidak pernah disimpan router — jadi node
    /// yang bentuk kursornya bergantung pada keadaannya sendiri (atau pada
    /// posisi penunjuk di dalam dirinya) cukup memperbarui keadaan itu di
    /// `event`, dan jawabannya sudah benar di sini pada event yang sama.
    fn perbarui_kursor(&mut self, tree: &RenderTree, id: PointerId, out: &mut Response) {
        let kursor = self
            .hover_of(id)
            .iter()
            .find_map(|n| tree.render(*n).and_then(|r| r.cursor()))
            .unwrap_or_default();
        if kursor != self.cursor {
            self.cursor = kursor;
            out.cursor = Some(kursor);
        }
    }

    // -- guliran ----------------------------------------------------------

    fn scroll(&mut self, tree: &mut RenderTree, event: &ScrollEvent) -> Response {
        self.modifiers = event.modifiers;
        let mut out = Response::default();
        let rute = hit_test(tree, event.position).path().to_vec();
        let mut sink = Sink::default();
        out.handled = self.kirim(tree, &rute, &Event::Scroll(event.clone()), &mut sink);
        self.terapkan(tree, sink, None, &mut out);
        out
    }

    // -- keyboard ---------------------------------------------------------

    fn key(&mut self, tree: &mut RenderTree, event: &KeyEvent) -> Response {
        self.modifiers = event.modifiers;
        let mut out = Response::default();

        let rute: Vec<HitEntry> = self
            .focus
            .path(tree)
            .into_iter()
            .map(|node| HitEntry {
                node,
                local: Point::ZERO,
            })
            .collect();
        let rute = if rute.is_empty() {
            vec![HitEntry {
                node: tree.root(),
                local: Point::ZERO,
            }]
        } else {
            rute
        };

        let mut sink = Sink::default();
        out.handled = self.kirim(tree, &rute, &Event::Key(event.clone()), &mut sink);
        self.terapkan(tree, sink, None, &mut out);

        // Tab adalah navigasi fokus **hanya** bila tidak ada yang mengambilnya
        // (text area memakai Tab untuk indentasi) dan hanya polos/Shift —
        // ⌘Tab dan Ctrl+Tab milik OS/aplikasi, bukan traversal widget.
        if !out.handled && event.is_pressed() && event.code.is(NamedKey::Tab) {
            let arah = if event.modifiers.is_exactly(Modifiers::SHIFT) {
                Some(FocusDirection::Previous)
            } else if event.modifiers.is_exactly(Modifiers::NONE) {
                Some(FocusDirection::Next)
            } else {
                None
            };
            if let Some(arah) = arah {
                let change = self.focus.move_focus(tree, arah);
                self.terapkan_fokus(tree, change, &mut out);
                out.handled = true;
            }
        }
        out
    }

    // -- IME --------------------------------------------------------------

    fn ime_event(&mut self, tree: &mut RenderTree, event: &ImeEvent) -> Response {
        let mut out = Response::default();
        let Some(fokus) = self.focus.focused() else {
            // Tidak ada tujuan komposisi: jangan biarkan IME menyala sendirian.
            if self.ime.take().is_some() {
                out.ime = Some(ImeRequest::Disable);
            }
            return out;
        };
        let rute = vec![HitEntry {
            node: fokus,
            local: Point::ZERO,
        }];
        let mut sink = Sink::default();
        out.handled = self.kirim(tree, &rute, &Event::Ime(event.clone()), &mut sink);
        self.terapkan(tree, sink, None, &mut out);
        out
    }

    // -- mesin penyampaian -------------------------------------------------

    /// Kirim event menyusuri rute (terdalam dulu) sampai ada yang menanganinya.
    fn kirim(
        &mut self,
        tree: &mut RenderTree,
        rute: &[HitEntry],
        event: &Event,
        sink: &mut Sink,
    ) -> bool {
        let mut handled = false;
        for entry in rute {
            self.sampaikan(tree, entry.node, entry.local, event, sink, &mut handled);
            if handled {
                break;
            }
        }
        handled
    }

    /// Kirim ke satu node saja (enter/leave, fokus, IME).
    fn kirim_satu(
        &mut self,
        tree: &mut RenderTree,
        node: NodeId,
        local: Point,
        event: &Event,
        sink: &mut Sink,
    ) {
        let mut handled = false;
        self.sampaikan(tree, node, local, event, sink, &mut handled);
    }

    fn sampaikan(
        &mut self,
        tree: &mut RenderTree,
        node: NodeId,
        local: Point,
        event: &Event,
        sink: &mut Sink,
        handled: &mut bool,
    ) {
        // Node dikeluarkan sementara dari arena — pola yang sama dengan layout,
        // dan alasan yang sama: handler tidak boleh melihat dirinya di pohon.
        let Some(mut render) = tree.take_render(node) else {
            return;
        };
        let mut ctx = EventCtx {
            node,
            local,
            size: tree.size(node),
            bounds: tree.bounds(node),
            focused: self.focus.is_focused(node),
            handled,
            sink,
        };
        render.event(&mut ctx, event);
        tree.put_render(node, render);
    }

    /// Terapkan titipan node: fokus, capture, IME, dan alasan dirty.
    ///
    /// `pointer` adalah penunjuk yang sedang diproses; capture hanya berlaku
    /// untuknya — jari kedua di layar sentuh tidak boleh ikut tertangkap.
    fn terapkan(
        &mut self,
        tree: &mut RenderTree,
        sink: Sink,
        pointer: Option<PointerId>,
        out: &mut Response,
    ) {
        out.dirty |= sink.dirty;

        if let (Some(capture), Some(id)) = (sink.capture, pointer) {
            self.pointers.entry(id).or_default().capture = capture;
        }

        if let Some(target) = sink.focus {
            let change = self.focus.focus(tree, target);
            self.terapkan_fokus(tree, change, out);
        }

        if let Some((node, area)) = sink.ime {
            self.terapkan_ime(node, area, out);
        }
    }

    fn terapkan_fokus(&mut self, tree: &mut RenderTree, change: FocusChange, out: &mut Response) {
        if !change.changed() {
            return;
        }
        out.focus = change;
        out.dirty |= Dirty::PAINT;
        let mut sink = Sink::default();
        if let Some(lost) = change.lost {
            self.kirim_satu(
                tree,
                lost,
                Point::ZERO,
                &Event::Focus(FocusEvent::Lost),
                &mut sink,
            );
        }
        if let Some(gained) = change.gained {
            self.kirim_satu(
                tree,
                gained,
                Point::ZERO,
                &Event::Focus(FocusEvent::Gained),
                &mut sink,
            );
        }
        out.dirty |= sink.dirty;
        // Node yang kehilangan fokus biasanya mematikan IME, dan yang mendapat
        // fokus menyalakannya — keduanya lewat titipan yang sama.
        if let Some((node, area)) = sink.ime {
            self.terapkan_ime(node, area, out);
        }
        // Sesi IME milik node yang sudah tidak fokus tidak boleh menggantung.
        if let Some((owner, _)) = self.ime {
            if !self.focus.is_focused(owner) {
                self.ime = None;
                out.ime = Some(ImeRequest::Disable);
            }
        }
    }

    fn terapkan_ime(&mut self, node: NodeId, area: Option<Rect>, out: &mut Response) {
        match area {
            Some(area) => {
                let permintaan = match self.ime {
                    Some((owner, sebelumnya)) if owner == node => {
                        if sebelumnya == area {
                            None
                        } else {
                            Some(ImeRequest::Update { area })
                        }
                    }
                    _ => Some(ImeRequest::Enable { area }),
                };
                self.ime = Some((node, area));
                if permintaan.is_some() {
                    out.ime = permintaan;
                }
            }
            None => {
                if matches!(self.ime, Some((owner, _)) if owner == node) {
                    self.ime = None;
                    out.ime = Some(ImeRequest::Disable);
                }
            }
        }
    }
}

/// Rute dari sebuah node ke akar, dengan koordinat lokal dihitung dari offset
/// global — dipakai saat penunjuk sedang ditangkap.
fn rute_dari_node(tree: &RenderTree, node: NodeId, position: Point) -> Vec<HitEntry> {
    let mut rute = Vec::new();
    let mut cur = Some(node);
    while let Some(id) = cur {
        if !tree.contains(id) {
            break;
        }
        let origin = tree.global_offset(id);
        rute.push(HitEntry {
            node: id,
            local: Point::new(position.x - origin.x, position.y - origin.y),
        });
        cur = tree.parent(id);
    }
    rute
}

fn hitung_klik(state: &PointerState, event: &PointerEvent, config: ClickConfig) -> u32 {
    let Some(button) = event.button else { return 0 };
    match state.last_click {
        Some((sebelumnya, posisi, waktu))
            if sebelumnya == button
                && event.time.saturating_sub(waktu) <= config.interval
                && jarak(posisi, event.position) <= config.distance =>
        {
            state.click_count.saturating_add(1).max(2)
        }
        _ => 1,
    }
}

fn jarak(a: Point, b: Point) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}
