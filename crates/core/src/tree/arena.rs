//! Arena render tree ber-ID + mesin layout box-constraints (REKOMENDASI §2, §3.4).
//!
//! Kenapa arena dan bukan ownership biasa: AccessKit dan Taffy sama-sama
//! berbasis ID/arena, jadi semuanya selaras — dan kita tidak perlu berperang
//! dengan borrow checker pada pohon yang saling menunjuk (induk ⇄ anak).
//! ID-nya **bergenerasi** persis seperti arena signals: slot yang sudah mati
//! tidak pernah tertukar dengan penghuninya yang baru.
//!
//! Modul ini adalah detail implementasi. Penulis aplikasi tidak pernah
//! menyentuhnya; penulis *widget* menyentuhnya lewat [`RenderNode`].

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use rustui_paint::{Color, Point, Rect, Scene, Size};

use crate::access::{AccessNode, AccessRole, AccessTree};
use crate::input::{CursorIcon, Event, EventCtx, FocusPolicy, HitBehavior, HitShape};
use crate::scheduler::Dirty;
use crate::signals::Key;

use super::constraints::BoxConstraints;
use super::paint::{paint_tree, PaintCache, PaintCtx};
use super::style::ItemStyle;

// ---------------------------------------------------------------------------
// ID
// ---------------------------------------------------------------------------

static NEXT_TREE: AtomicU32 = AtomicU32::new(0);

/// Identitas satu render tree (satu per window).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TreeId(u32);

/// Identitas satu node di arena render tree.
///
/// Bergenerasi: setelah node mati, ID lama tidak akan pernah cocok lagi dengan
/// node baru yang menempati slot yang sama. ID juga membawa [`TreeId`] supaya
/// node dari window lain tidak pernah tertukar diam-diam.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId {
    tree: TreeId,
    index: u32,
    generation: u32,
}

impl NodeId {
    /// Pohon pemilik node ini.
    pub fn tree(self) -> TreeId {
        self.tree
    }

    /// Nomor slot arena (stabil hanya selama node hidup).
    pub fn index(self) -> u32 {
        self.index
    }

    /// Generasi slot — pembeda antara penghuni lama dan baru.
    pub fn generation(self) -> u32 {
        self.generation
    }
}

impl core::fmt::Debug for NodeId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Node(#{}v{})", self.index, self.generation)
    }
}

// ---------------------------------------------------------------------------
// Trait node
// ---------------------------------------------------------------------------

/// Downcast otomatis untuk semua tipe `'static`.
///
/// Ada supaya penulis [`RenderNode`] tidak perlu menulis boilerplate `as_any`;
/// lapisan view memakainya untuk menerapkan props ke node yang sudah ada
/// ("trait object + downcast", REKOMENDASI §2).
pub trait AsAny: 'static {
    /// Referensi `Any` ke diri sendiri.
    fn as_any(&self) -> &dyn Any;
    /// Referensi `Any` mutable ke diri sendiri.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Perilaku satu node render: layout, batas relayout, dan emisi a11y.
///
/// Kontraknya persis tiga kalimat box constraints (lihat
/// [`BoxConstraints`]): terima constraints, kembalikan ukuran, dan **tetapkan
/// posisi anak-anak** lewat [`LayoutCtx::place_child`]. Node tidak pernah tahu
/// posisinya sendiri.
///
/// [`RenderNode::access`] **tidak punya implementasi bawaan** dan itu
/// disengaja: accessibility adalah keluaran pohon render, bukan tambahan
/// (§3.8). Widget baru yang lupa memikirkan screen reader tidak lolos compile
/// — inilah satu-satunya pertahanan yang terbukti terhadap failure mode
/// "accessibility di-retrofit" (§5 poin 2). Node yang memang hanya struktur
/// menyatakannya secara eksplisit dengan [`AccessRole::Container`].
pub trait RenderNode: AsAny {
    /// Nama tipe untuk debug/inspector.
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Hitung ukuran sendiri dari `constraints`, dan tempatkan anak-anak.
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size;

    /// Gambar node ini ke dalam scene (REKOMENDASI §3.2).
    ///
    /// Koordinatnya **lokal**: `(0, 0)` adalah sudut kiri-atas node, dan
    /// [`PaintCtx`] yang menaikkannya ke koordinat absolut — aturan yang sama
    /// dengan layout, di mana node juga tidak pernah tahu posisinya sendiri.
    ///
    /// Bawaannya **tidak menggambar apa pun** tapi tetap menurunkan isinya
    /// ([`PaintCtx::paint_children`]), sehingga node yang murni struktural
    /// (padding, align, wadah) tidak dipaksa menulis apa-apa dan pohonnya tidak
    /// menghilang. Node yang menimpanya wajib memanggil
    /// [`PaintCtx::paint_children`]/[`PaintCtx::paint_child`] sendiri — di
    /// situlah ia memutuskan apa yang berada di bawah dan di atas anaknya.
    ///
    /// Kosakatanya hanya `rustui-paint`; tipe wgpu tidak pernah sampai ke sini
    /// (§3.2).
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.paint_children();
    }

    /// Benar bila node ini **selalu** menjadi relayout boundary.
    ///
    /// Dipakai node yang ukurannya tidak pernah bergantung pada isinya —
    /// viewport scroll adalah contoh kanonis: isi boleh berubah setinggi apa
    /// pun, kotak viewport-nya tetap.
    fn is_relayout_boundary(&self) -> bool {
        false
    }

    /// Gaya node ini **sebagai item** di dalam wadah flex/grid.
    ///
    /// Padanan `ParentData` Flutter: datanya menempel di anak, tapi yang
    /// membacanya adalah induk ([`LayoutCtx::child_layout_style`]). Node biasa
    /// tidak perlu peduli — bawaannya [`ItemStyle::DEFAULT`], dan hanya
    /// [`super::LayoutItem`] (`expanded()`/`flexible()`) yang mengisinya.
    fn layout_style(&self) -> ItemStyle {
        ItemStyle::DEFAULT
    }

    /// Isi node aksesibilitas: role, name, value, actions, state.
    ///
    /// **Wajib diimplementasikan.** `bounds`, induk, dan daftar anak tidak ada
    /// di [`AccessNode`] sama sekali — semuanya dirakit mesin dari hasil layout
    /// ([`RenderTree::access_tree`]), jadi tidak mungkin basi dan tidak mungkin
    /// dipalsukan widget.
    ///
    /// Node yang murni struktural (padding, align) cukup menyatakan
    /// [`AccessRole::Container`]: teknologi bantu akan menyaringnya keluar dan
    /// anak-anaknya naik menggantikannya.
    fn access(&self, node: &mut AccessNode);

    // -- input ------------------------------------------------------------
    //
    // Empat kait berikut adalah kontrak input. Semuanya punya nilai bawaan
    // yang aman ("saya tidak ikut campur"), karena mayoritas node memang
    // struktural — tapi node interaktif yang lupa mengisinya akan langsung
    // terasa: tidak bisa diklik, tidak bisa di-Tab.

    /// Bentuk area sentuh node — **inilah tempat squircle merembet ke
    /// hit-testing** (REKOMENDASI §3.6).
    ///
    /// Bawaannya kotak penuh. Node yang menggambar dirinya dengan sudut
    /// melengkung wajib mengembalikan [`HitShape::Rounded`] dengan
    /// [`rustui_paint::Corners`] **yang sama persis** dengan yang dikirim ke
    /// shader — kalau tidak, ada pita beberapa poin di tiap pojok yang terlihat
    /// kosong tapi bisa diklik.
    fn hit_shape(&self) -> HitShape {
        HitShape::Rect
    }

    /// Perilaku node terhadap event penunjuk.
    ///
    /// Bawaannya [`HitBehavior::DeferToChild`]: wadah struktural tidak mencuri
    /// klik dari isinya.
    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::DeferToChild
    }

    /// Benar bila node memotong isinya di kotaknya sendiri.
    ///
    /// Viewport menjawab benar: baris yang sudah tergulir keluar layar tidak
    /// boleh bisa diklik hanya karena masih ada di pohon.
    fn clips_children(&self) -> bool {
        false
    }

    /// Peran node dalam navigasi fokus keyboard (Tab/Shift+Tab).
    fn focus_policy(&self) -> FocusPolicy {
        FocusPolicy::NONE
    }

    /// Bentuk kursor saat penunjuk berada di atas node ini.
    ///
    /// `None` = "terserah node di bawahku". Router menanyakan ini pada jalur
    /// hover, jadi tidak ada state kursor yang bisa basi.
    fn cursor(&self) -> Option<CursorIcon> {
        None
    }

    /// Tangani satu event input.
    ///
    /// Node hanya boleh mengubah **dirinya sendiri**; segala hal yang
    /// menyangkut dunia luar (fokus, capture, IME, permintaan gambar ulang)
    /// dititipkan lewat [`EventCtx`]. Struktur pohon tidak boleh berubah dari
    /// sini — itu wewenang view-diff.
    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let _ = (ctx, event);
    }
}

impl dyn RenderNode {
    /// Downcast ke tipe node konkret.
    pub fn downcast_ref<T: RenderNode>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }

    /// Downcast mutable ke tipe node konkret.
    pub fn downcast_mut<T: RenderNode>(&mut self) -> Option<&mut T> {
        self.as_any_mut().downcast_mut::<T>()
    }
}

impl core::fmt::Debug for dyn RenderNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.type_name())
    }
}

// ---------------------------------------------------------------------------
// Arah teks
// ---------------------------------------------------------------------------

/// Arah baca dokumen — dipahami sistem layout **sejak awal** (§9.8).
///
/// Mirroring RTL bukan fitur susulan: `row` membalik urutan sumbu utamanya dan
/// sumbu silang `column` ikut terbalik, keduanya di dalam mesin layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextDirection {
    /// Kiri ke kanan (Latin, CJK).
    #[default]
    Ltr,
    /// Kanan ke kiri (Arab, Ibrani).
    Rtl,
}

impl TextDirection {
    /// Benar bila arahnya kanan-ke-kiri.
    pub fn is_rtl(self) -> bool {
        matches!(self, TextDirection::Rtl)
    }
}

// ---------------------------------------------------------------------------
// Node & slot
// ---------------------------------------------------------------------------

struct Node {
    key: Option<Key>,
    type_id: TypeId,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    depth: u32,
    /// `None` **hanya** selama node itu sedang menjalankan layout-nya sendiri.
    render: Option<Box<dyn RenderNode>>,
    size: Size,
    /// Posisi relatif induk — selalu ditulis induk, tidak pernah diri sendiri.
    offset: Point,
    constraints: Option<BoxConstraints>,
    needs_layout: bool,
    needs_paint: bool,
    boundary: bool,
    parent_uses_size: bool,
    /// Bolehkah constraints tight menjadikan node ini relayout boundary?
    ///
    /// Biasanya ya: kalau induk sudah memaksa ukurannya, isi node tidak mungkin
    /// mengubah siapa pun di atas. Kecuali satu kasus — wadah flex/grid yang
    /// **menurunkan angka tight itu dari hasil mengukur anaknya sendiri**
    /// ([`super::TaffyBox`]). Di sana ketatnya semu: isi berubah → hasil ukur
    /// berubah → seluruh flex wajib dihitung ulang, jadi rambatan dirty tidak
    /// boleh berhenti di anak.
    tight_is_boundary: bool,
    /// Cermin persis dari keanggotaan di `RenderTree::dirty_boundaries`.
    ///
    /// Ada supaya antrean bisa dijaga bebas duplikat **tanpa** memakai
    /// early-out "sudah ditandai berarti sudah terdaftar" yang pernah membuat
    /// boundary hilang dari antrean selamanya.
    queued: bool,
    layout_count: u32,
    /// Perintah gambar subtree ini dari pass paint terakhir.
    ///
    /// Hanya diisi di relayout boundary (selain akar) — lihat
    /// [`super::paint`]. `None` berarti "belum pernah, atau bukan tempat
    /// menyimpan cache".
    paint_cache: Option<PaintCache>,
    paint_count: u32,
}

struct Slot {
    generation: u32,
    node: Option<Node>,
}

/// Node akar: meneruskan constraints window apa adanya ke satu anak.
///
/// Bagi teknologi bantu ia adalah **window**, dan namanya (judul window)
/// adalah hal pertama yang dibacakan screen reader saat aplikasi mendapat
/// fokus — karena itu labelnya ikut disimpan di sini.
#[derive(Default)]
struct Root {
    label: Option<String>,
}

impl RenderNode for Root {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Point::ZERO);
        size
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Window;
        node.label.clone_from(&self.label);
    }
}

// ---------------------------------------------------------------------------
// RenderTree
// ---------------------------------------------------------------------------

/// Render tree retained berbasis arena.
///
/// Strukturnya **hanya** diubah lapisan view-diff ([`crate::view`]); layout
/// tidak pernah menambah atau membuang node. Karena itu `depth` selalu benar
/// dan urutan flush layout bisa diandalkan.
///
/// ```
/// use rustui_core::tree::{BoxConstraints, RenderTree};
/// use rustui_core::view::{fixed, pad, reconcile};
/// use rustui_paint::{Insets, Point, Size};
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, pad(Insets::all(8.0), fixed(100.0, 20.0)));
/// tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
///
/// let luar = tree.children(tree.root())[0];
/// let dalam = tree.children(luar)[0];
/// // Ukuran naik: padding = anak + insets.
/// assert_eq!(tree.size(luar), Size::new(116.0, 36.0));
/// // Induk yang menentukan posisi anak.
/// assert_eq!(tree.offset(dalam), Point::new(8.0, 8.0));
/// ```
pub struct RenderTree {
    id: TreeId,
    slots: Vec<Slot>,
    free: Vec<u32>,
    root: NodeId,
    dirty_boundaries: Vec<NodeId>,
    dirty: Dirty,
    direction: TextDirection,
    clear_color: Color,
}

impl Default for RenderTree {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderTree {
    /// Pohon baru berisi satu node akar.
    pub fn new() -> Self {
        let id = TreeId(NEXT_TREE.fetch_add(1, Ordering::Relaxed));
        let mut tree = Self {
            id,
            slots: Vec::new(),
            free: Vec::new(),
            root: NodeId {
                tree: id,
                index: 0,
                generation: 0,
            },
            dirty_boundaries: Vec::new(),
            dirty: Dirty::NONE,
            direction: TextDirection::Ltr,
            clear_color: Color::TRANSPARENT,
        };
        let root = tree.alloc(None, None, TypeId::of::<Root>(), Box::<Root>::default());
        tree.root = root;
        if let Some(n) = tree.node_mut(root) {
            n.boundary = true;
        }
        tree
    }

    /// Identitas pohon ini.
    pub fn id(&self) -> TreeId {
        self.id
    }

    /// Node akar (selalu hidup).
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// Arah baca yang berlaku untuk seluruh pohon.
    pub fn direction(&self) -> TextDirection {
        self.direction
    }

    /// Ganti arah baca; **seluruh** pohon perlu di-layout ulang.
    ///
    /// Arah baca adalah masukan layout yang tidak ikut jadi kunci cache, jadi
    /// menandai akar saja tidak cukup — cache anak-anak yang constraints-nya
    /// tidak berubah akan menahan mirroring-nya (§9.8).
    pub fn set_direction(&mut self, direction: TextDirection) {
        if self.direction == direction {
            return;
        }
        self.direction = direction;
        self.invalidate_all();
    }

    /// Batalkan seluruh cache layout — dipakai saat masukan global berubah
    /// (arah baca, dan nanti scale factor/theme yang memengaruhi ukuran).
    pub fn invalidate_all(&mut self) {
        for slot in &mut self.slots {
            if let Some(n) = slot.node.as_mut() {
                n.needs_layout = true;
                n.needs_paint = true;
            }
        }
        self.dirty.insert(Dirty::LAYOUT | Dirty::PAINT);
        let root = self.root;
        self.enqueue_boundary(root);
    }

    // -- inspeksi ---------------------------------------------------------

    /// Jumlah node hidup (termasuk akar).
    pub fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    /// Benar bila hanya ada akar.
    pub fn is_empty(&self) -> bool {
        self.len() <= 1
    }

    /// Benar bila `id` masih menunjuk node hidup di pohon ini.
    pub fn contains(&self, id: NodeId) -> bool {
        self.node(id).is_some()
    }

    /// Induk sebuah node (`None` untuk akar atau id mati).
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id)?.parent
    }

    /// Anak-anak sebuah node, urut.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.node(id).map(|n| n.children.as_slice()).unwrap_or(&[])
    }

    /// Kunci identitas node (dari view yang membangunnya).
    pub fn key(&self, id: NodeId) -> Option<Key> {
        self.node(id)?.key.clone()
    }

    /// Tipe view yang membangun node ini — dipakai diffing untuk memutuskan
    /// "perbarui di tempat" vs "ganti".
    pub fn type_id_of(&self, id: NodeId) -> Option<TypeId> {
        Some(self.node(id)?.type_id)
    }

    /// Kedalaman dari akar (akar = 0).
    pub fn depth(&self, id: NodeId) -> Option<u32> {
        Some(self.node(id)?.depth)
    }

    /// Ukuran hasil layout terakhir.
    pub fn size(&self, id: NodeId) -> Size {
        self.node(id).map(|n| n.size).unwrap_or(Size::ZERO)
    }

    /// Posisi relatif terhadap induk (ditetapkan induk).
    pub fn offset(&self, id: NodeId) -> Point {
        self.node(id).map(|n| n.offset).unwrap_or(Point::ZERO)
    }

    /// Posisi absolut di dalam pohon.
    pub fn global_offset(&self, id: NodeId) -> Point {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(n) = self.node(c) else { break };
            x += n.offset.x;
            y += n.offset.y;
            cur = n.parent;
        }
        Point::new(x, y)
    }

    /// Kotak absolut node — inilah `bounds` yang dipakai a11y dan hit-testing.
    pub fn bounds(&self, id: NodeId) -> Rect {
        Rect::from_origin_size(self.global_offset(id), self.size(id))
    }

    /// Constraints yang dipakai pada layout terakhir.
    pub fn constraints(&self, id: NodeId) -> Option<BoxConstraints> {
        self.node(id)?.constraints
    }

    /// Berapa kali node ini benar-benar menjalankan layout-nya.
    ///
    /// Ada untuk membuktikan janji "kerja layout dibatasi relayout boundary" —
    /// dipakai unit test dan inspector, bukan logika framework.
    pub fn layout_count(&self, id: NodeId) -> u32 {
        self.node(id).map(|n| n.layout_count).unwrap_or(0)
    }

    /// Benar bila node menunggu layout.
    pub fn needs_layout(&self, id: NodeId) -> bool {
        self.node(id).map(|n| n.needs_layout).unwrap_or(false)
    }

    /// Benar bila node menunggu digambar ulang.
    pub fn needs_paint(&self, id: NodeId) -> bool {
        self.node(id).map(|n| n.needs_paint).unwrap_or(false)
    }

    /// Benar bila node adalah relayout boundary menurut layout terakhir.
    pub fn is_relayout_boundary(&self, id: NodeId) -> bool {
        self.node(id).map(|n| n.boundary).unwrap_or(false)
    }

    /// Jumlah boundary yang menunggu di antrean relayout.
    pub fn pending_boundaries(&self) -> usize {
        self.dirty_boundaries.len()
    }

    /// Perilaku node.
    pub fn render(&self, id: NodeId) -> Option<&dyn RenderNode> {
        self.node(id)?.render.as_deref()
    }

    /// Perilaku node, mutable.
    pub fn render_mut(&mut self, id: NodeId) -> Option<&mut dyn RenderNode> {
        self.node_mut(id)?.render.as_deref_mut()
    }

    /// Perilaku node yang sudah di-downcast ke tipe konkret.
    pub fn node_ref<T: RenderNode>(&self, id: NodeId) -> Option<&T> {
        self.render(id)?.downcast_ref::<T>()
    }

    /// Perilaku node yang sudah di-downcast ke tipe konkret, mutable.
    ///
    /// Perubahan lewat jalur ini **tidak** otomatis menandai dirty — panggil
    /// [`RenderTree::mark_needs_layout`]/[`RenderTree::mark_needs_paint`]
    /// sendiri. Jalur normalnya adalah view-diff, yang sudah melakukannya.
    pub fn node_mut_ref<T: RenderNode>(&mut self, id: NodeId) -> Option<&mut T> {
        self.render_mut(id)?.downcast_mut::<T>()
    }

    /// Keluarkan perilaku node dari arena sementara.
    ///
    /// Dipakai routing input dengan alasan yang sama seperti layout: selama
    /// sebuah node menangani event, ia tidak boleh bisa melihat (apalagi
    /// mengubah) dirinya sendiri lewat pohon. Wajib dikembalikan dengan
    /// [`RenderTree::put_render`].
    pub(crate) fn take_render(&mut self, id: NodeId) -> Option<Box<dyn RenderNode>> {
        self.node_mut(id)?.render.take()
    }

    /// Kembalikan perilaku node yang diambil [`RenderTree::take_render`].
    pub(crate) fn put_render(&mut self, id: NodeId, render: Box<dyn RenderNode>) {
        if let Some(node) = self.node_mut(id) {
            node.render = Some(render);
        }
    }

    // -- mutasi struktur --------------------------------------------------

    /// Sisipkan anak baru di `index` (dipotong ke jumlah anak yang ada).
    ///
    /// Hanya lapisan view-diff yang boleh memanggil ini.
    pub fn insert_child(
        &mut self,
        parent: NodeId,
        index: usize,
        key: Option<Key>,
        type_id: TypeId,
        render: Box<dyn RenderNode>,
    ) -> NodeId {
        assert!(self.contains(parent), "induk {parent:?} sudah mati");
        let child = self.alloc(Some(parent), key, type_id, render);
        let depth = self.node(parent).map(|n| n.depth).unwrap_or(0) + 1;
        if let Some(n) = self.node_mut(child) {
            n.depth = depth;
        }
        let n = self.node_mut(parent).expect("induk hidup");
        let at = index.min(n.children.len());
        n.children.insert(at, child);
        self.mark_needs_layout(parent);
        child
    }

    /// Buang node beserta seluruh keturunannya; kembalikan jumlah node terbuang.
    ///
    /// Akar tidak bisa dibuang (panik) — pohon selalu punya akar.
    pub fn remove_subtree(&mut self, id: NodeId) -> usize {
        assert!(id != self.root, "akar render tree tidak boleh dibuang");
        let Some(node) = self.node(id) else { return 0 };
        let parent = node.parent;
        if let Some(p) = parent {
            if let Some(pn) = self.node_mut(p) {
                pn.children.retain(|c| *c != id);
            }
            self.mark_needs_layout(p);
        }
        let mut stack = vec![id];
        let mut removed = 0;
        while let Some(cur) = stack.pop() {
            let Some(idx) = self.index_of(cur) else {
                continue;
            };
            let node = self.slots[idx].node.take().expect("slot hidup");
            stack.extend(node.children);
            self.slots[idx].generation = self.slots[idx].generation.wrapping_add(1);
            self.free.push(idx as u32);
            removed += 1;
        }
        self.dirty.insert(Dirty::LAYOUT | Dirty::PAINT);
        removed
    }

    /// Tata ulang urutan anak `parent` menjadi `order`.
    ///
    /// `order` wajib berisi persis anak-anak yang ada sekarang (jumlah dan
    /// himpunan sama) — pelanggaran = panik, bukan pohon yang rusak diam-diam.
    pub fn set_children(&mut self, parent: NodeId, order: &[NodeId]) {
        let current = self.children(parent);
        assert_eq!(
            current.len(),
            order.len(),
            "set_children harus memuat semua anak {parent:?}"
        );
        if current == order {
            return;
        }
        for id in order {
            assert_eq!(
                self.parent(*id),
                Some(parent),
                "{id:?} bukan anak {parent:?}"
            );
        }
        if let Some(n) = self.node_mut(parent) {
            n.children.clear();
            n.children.extend_from_slice(order);
        }
        self.mark_needs_layout(parent);
    }

    // -- dirty ------------------------------------------------------------

    /// Tandai node butuh layout ulang.
    ///
    /// Penandaan merambat ke atas **sampai relayout boundary terdekat**, lalu
    /// boundary itulah yang masuk antrean. Inilah yang membuat perubahan kecil
    /// di dalam scroll view tidak pernah membuat seluruh window di-layout ulang
    /// (§3.4).
    pub fn mark_needs_layout(&mut self, id: NodeId) {
        self.dirty.insert(Dirty::LAYOUT | Dirty::PAINT);
        // Rambatan paint punya aturannya sendiri (sampai akar, tanpa berhenti
        // di relayout boundary) — lihat [`RenderTree::mark_needs_paint`].
        self.mark_needs_paint(id);
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(node) = self.node_mut(c) else { return };
            // Sengaja **tidak** berhenti hanya karena `needs_layout` sudah true:
            // tanda itu tidak membuktikan boundary di atasnya masih mengantre
            // (antrean bisa saja sudah dikuras oleh pass sebelumnya). Berhenti
            // di situ = boundary yang tidak pernah dikerjakan lagi, dan
            // `needs_layout` yang tidak pernah bisa dibersihkan. Jalannya tetap
            // pendek: rambatan selalu berhenti di boundary terdekat.
            node.needs_layout = true;
            node.needs_paint = true;
            let boundary = node.boundary;
            let parent = node.parent;
            if boundary || parent.is_none() {
                self.enqueue_boundary(c);
                return;
            }
            cur = parent;
        }
    }

    /// Masukkan boundary ke antrean relayout, sekali saja.
    ///
    /// `Node::queued` adalah cermin keanggotaan antrean, jadi pemanggilan
    /// berulang tidak pernah menumpuk duplikat.
    fn enqueue_boundary(&mut self, id: NodeId) {
        let Some(node) = self.node_mut(id) else {
            return;
        };
        if node.queued {
            return;
        }
        node.queued = true;
        self.dirty_boundaries.push(id);
    }

    /// Tambahkan sebuah alasan dirty ke pohon tanpa menyentuh node mana pun.
    ///
    /// Untuk alasan yang **bukan tentang geometri**, dan hanya satu yang
    /// begitu: [`Dirty::ANIMATION`]. Sebuah spring yang baru diarahkan (props
    /// `open` sebuah dialog berubah lewat view-diff, tombol masuk keadaan
    /// loading) belum menggeser satu piksel pun frame ini — yang dibutuhkannya
    /// adalah **frame berikutnya**. Tanpa pintu ini alasan itu hilang di
    /// perjalanan dan animasinya membeku sampai ada event input berikutnya.
    pub fn mark_dirty(&mut self, dirty: Dirty) {
        self.dirty.insert(dirty);
    }

    /// Tandai node perlu digambar ulang (tanpa layout).
    ///
    /// Penandaan merambat **sampai akar**, dan itu bukan pemborosan: pass paint
    /// menyimpan perintah gambar satu subtree di relayout boundary
    /// ([`RenderTree::paint`]). Kalau tanda ini berhenti di tengah, sebuah
    /// boundary di atas node yang berubah akan mengira dirinya bersih dan
    /// menyalin kembali gambar lama — perubahannya hilang tanpa suara. "Bersih"
    /// harus benar-benar berarti "tidak ada apa pun di dalamku yang berubah".
    ///
    /// Repaint boundary (layer yang ukurannya tidak merambat) adalah pekerjaan
    /// milestone layer/offscreen, bukan di sini.
    pub fn mark_needs_paint(&mut self, id: NodeId) {
        self.dirty.insert(Dirty::PAINT);
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(node) = self.node_mut(c) else { return };
            // Sengaja **tidak** berhenti hanya karena `needs_paint` sudah true:
            // tanda itu bisa datang dari jalur lain (node baru dialokasikan,
            // ukurannya berubah saat layout) yang tidak ikut merambat ke atas.
            // Berhenti di situ = leluhur yang mengira dirinya bersih. Jalannya
            // pendek: satu jalur lurus ke akar.
            node.needs_paint = true;
            cur = node.parent;
        }
    }

    /// Ambil alasan dirty yang terkumpul dan kosongkan.
    ///
    /// Inilah yang disambungkan ke
    /// [`crate::scheduler::FrameScheduler::request`] — render tetap **hanya
    /// saat dirty** (§3.5).
    pub fn take_dirty(&mut self) -> Dirty {
        core::mem::replace(&mut self.dirty, Dirty::NONE)
    }

    /// Alasan dirty yang terkumpul, tanpa mengosongkan.
    pub fn dirty(&self) -> Dirty {
        self.dirty
    }

    /// Tandai seluruh pohon sudah digambar.
    pub fn clear_paint(&mut self) {
        for slot in &mut self.slots {
            if let Some(n) = slot.node.as_mut() {
                n.needs_paint = false;
            }
        }
    }

    // -- paint ------------------------------------------------------------

    /// Warna latar frame — **selalu token theme** (`theme.color.background`).
    pub fn clear_color(&self) -> Color {
        self.clear_color
    }

    /// Ganti warna latar frame (mis. setelah dark mode berubah).
    ///
    /// Mengubahnya menandai seluruh pohon perlu digambar ulang: warna latar
    /// berganti karena preset/appearance berganti, dan itu mengubah setiap
    /// warna token yang sudah terlanjur masuk cache gambar.
    pub fn set_clear_color(&mut self, color: Color) {
        if self.clear_color == color {
            return;
        }
        self.clear_color = color;
        self.dirty.insert(Dirty::PAINT);
        for slot in &mut self.slots {
            if let Some(n) = slot.node.as_mut() {
                n.needs_paint = true;
            }
        }
    }

    /// **Pass paint**: susun [`Scene`] frame ini dari render tree (§3.2).
    ///
    /// Sejajar dengan layout dan a11y. Subtree yang bersih **tidak** dijalankan
    /// ulang: perintah gambarnya disalin dari cache di relayout boundary, dan
    /// `needs_paint` seluruh pohon dibersihkan setelah selesai.
    ///
    /// Wajib dipanggil setelah layout: posisi absolut yang dipakai di sini
    /// datang dari hasil layout, sama persis seperti `bounds` a11y.
    ///
    /// ```
    /// use rustui_core::tree::{BoxConstraints, RenderTree};
    /// use rustui_core::view::{fixed, pad, reconcile};
    /// use rustui_paint::{Color, Insets, Size};
    ///
    /// let mut tree = RenderTree::new();
    /// reconcile(
    ///     &mut tree,
    ///     pad(Insets::all(8.0), fixed(100.0, 20.0).background(Color::WHITE)),
    /// );
    /// tree.layout(BoxConstraints::loose(Size::new(200.0, 200.0)));
    /// let scene = tree.paint();
    /// assert_eq!(scene.len(), 1);
    /// ```
    pub fn paint(&mut self) -> Scene {
        let mut scene = Scene::new(self.clear_color);
        self.paint_into(&mut scene);
        scene
    }

    /// Versi [`RenderTree::paint`] yang memakai ulang buffer scene.
    ///
    /// Inilah jalur per-frame: alokasi perintah gambar dipertahankan antar
    /// frame, jadi menggambar ulang tidak menyentuh allocator.
    pub fn paint_into(&mut self, scene: &mut Scene) {
        scene.reset(self.clear_color);
        paint_tree(self, scene);
        self.clear_paint();
    }

    /// Berapa kali node ini benar-benar menjalankan gambarnya.
    ///
    /// Kembarannya [`RenderTree::layout_count`], dan ada untuk alasan yang
    /// sama: membuktikan subtree bersih memang dilewati. Bukan logika framework.
    pub fn paint_count(&self, id: NodeId) -> u32 {
        self.node(id).map(|n| n.paint_count).unwrap_or(0)
    }

    /// Geometri yang dibutuhkan pass paint: offset relatif, ukuran, tanda
    /// kotor, dan status boundary.
    pub(super) fn paint_geometry(&self, id: NodeId) -> Option<(Point, Size, bool, bool)> {
        let n = self.node(id)?;
        Some((n.offset, n.size, n.needs_paint, n.boundary))
    }

    /// Perintah gambar subtree ini dari frame sebelumnya, bila ada.
    pub(super) fn paint_cache(&self, id: NodeId) -> Option<&PaintCache> {
        self.node(id)?.paint_cache.as_ref()
    }

    /// Simpan hasil pass paint sebuah node dan catat bahwa ia benar-benar
    /// menggambar.
    pub(super) fn finish_paint(&mut self, id: NodeId, cache: Option<PaintCache>) {
        if let Some(n) = self.node_mut(id) {
            n.paint_cache = cache;
            n.paint_count = n.paint_count.saturating_add(1);
        }
    }

    // -- layout -----------------------------------------------------------

    /// Layout penuh dari akar dengan constraints window.
    ///
    /// Dipanggil pada frame pertama dan setiap kali ukuran surface berubah.
    /// Constraints yang sama + pohon bersih = tidak ada kerja sama sekali.
    ///
    /// Setelah pass penuh, antrean relayout ikut dikuras
    /// ([`RenderTree::flush_layout`]): boundary yang mengantre bisa saja
    /// terlewat karena leluhurnya kena cache-hit, dan setelah ini
    /// [`RenderTree::pending_boundaries`] selalu nol untuk boundary yang sudah
    /// pernah di-layout.
    pub fn layout(&mut self, constraints: BoxConstraints) -> Size {
        let root = self.root;
        let size = self.layout_node(root, constraints, true, true);
        // Antrean **tidak boleh sekadar dibuang**. Pass penuh berhenti lebih
        // awal setiap kali menemui cache-hit, jadi boundary yang mengantre di
        // bawah leluhur yang bersih (mis. scroll view di dalam kotak
        // berukuran tight, sementara yang berubah adalah saudaranya) tidak
        // pernah tersentuh. Membuang entrinya membuat `needs_layout`-nya
        // menetap selamanya dan scroll view mati tanpa suara.
        self.flush_layout();
        size
    }

    /// Layout ulang **hanya** subtree yang kotor, memakai constraints yang
    /// tersimpan di tiap boundary. Kembalikan jumlah boundary yang dikerjakan.
    ///
    /// Boundary menjamin ukurannya sendiri tidak berubah, jadi tidak ada yang
    /// perlu merambat ke atas.
    pub fn flush_layout(&mut self) -> usize {
        let mut queue = core::mem::take(&mut self.dirty_boundaries);
        for id in &queue {
            if let Some(n) = self.node_mut(*id) {
                n.queued = false;
            }
        }
        // Leluhur lebih dulu: satu boundary bisa membersihkan boundary di
        // bawahnya, dan yang sudah bersih tinggal dilewati.
        queue.sort_by_key(|id| self.depth(*id).unwrap_or(0));
        let mut done = 0;
        for id in queue {
            let Some((needs_layout, constraints, parent_uses_size, tight_is_boundary)) =
                self.node(id).map(|n| {
                    (
                        n.needs_layout,
                        n.constraints,
                        n.parent_uses_size,
                        n.tight_is_boundary,
                    )
                })
            else {
                continue;
            };
            if !needs_layout {
                continue;
            }
            let Some(constraints) = constraints else {
                // Belum pernah di-layout: harus lewat layout penuh dari akar,
                // jadi entrinya dikembalikan ke antrean.
                self.enqueue_boundary(id);
                continue;
            };
            self.layout_node(id, constraints, parent_uses_size, tight_is_boundary);
            done += 1;
        }
        done
    }

    /// Jalur layout normal per frame: layout penuh bila constraints berubah
    /// atau akar kotor, selain itu cukup subtree yang kotor.
    pub fn perform_layout(&mut self, constraints: BoxConstraints) -> Size {
        let root = self.root;
        let perlu_penuh =
            self.needs_layout(root) || self.constraints(root) != Some(constraints.normalized());
        if perlu_penuh {
            self.layout(constraints)
        } else {
            self.flush_layout();
            self.size(root)
        }
    }

    fn layout_node(
        &mut self,
        id: NodeId,
        constraints: BoxConstraints,
        parent_uses_size: bool,
        tight_is_boundary: bool,
    ) -> Size {
        let constraints = constraints.normalized();
        let (is_root, intrinsic) = {
            let node = self
                .node(id)
                .unwrap_or_else(|| panic!("layout node mati: {id:?}"));
            let render = node.render.as_ref().unwrap_or_else(|| {
                panic!("{id:?} sedang melakukan layout — layout rekursif tidak diizinkan")
            });
            (node.parent.is_none(), render.is_relayout_boundary())
        };
        // Aturan Flutter: boundary bila ukuran sendiri tidak mungkin dipengaruhi
        // isinya, atau induk memang tidak memakai ukuran kita. `tight_is_boundary`
        // adalah pengecualian yang dipakai wadah flex/grid — lihat field
        // `Node::tight_is_boundary`.
        let boundary = is_root
            || intrinsic
            || (tight_is_boundary && constraints.is_tight())
            || !parent_uses_size;

        {
            let node = self.node_mut(id).expect("node hidup");
            if !node.needs_layout
                && node.constraints == Some(constraints)
                && node.boundary == boundary
            {
                return node.size;
            }
            node.boundary = boundary;
            node.parent_uses_size = parent_uses_size;
            node.tight_is_boundary = tight_is_boundary;
            node.constraints = Some(constraints);
        }

        let mut render = self
            .node_mut(id)
            .expect("node hidup")
            .render
            .take()
            .expect("render node tersedia");
        let size = {
            let mut ctx = LayoutCtx {
                tree: self,
                node: id,
            };
            render.layout(&mut ctx, constraints)
        };
        let size = constraints.constrain(size);
        debug_assert!(
            size.width.is_finite() && size.height.is_finite(),
            "{id:?} ({}) memilih ukuran tak hingga di bawah constraints tanpa batas",
            render.type_name()
        );

        let node = self
            .node_mut(id)
            .expect("struktur pohon tidak boleh berubah selama layout");
        node.render = Some(render);
        node.size = size;
        node.needs_layout = false;
        node.layout_count = node.layout_count.saturating_add(1);
        // Node yang benar-benar menjalankan layout **selalu** perlu digambar
        // ulang: kita hanya sampai di sini kalau ia kotor atau constraints-nya
        // berubah, dan keduanya bisa mengubah ukurannya. Menandainya dari sini
        // menutup jalur yang tidak lewat `mark_needs_layout` sama sekali —
        // mis. anak yang di-layout ulang hanya karena induknya berubah.
        self.mark_needs_paint(id);
        size
    }

    // -- a11y -------------------------------------------------------------

    /// Emisi seluruh pohon aksesibilitas — **pass sejajar layout dan paint**,
    /// bukan lapisan susulan (§3.8).
    ///
    /// `bounds` tiap node datang dari hasil layout, jadi apa yang dibacakan
    /// screen reader dan apa yang digambar tidak mungkin berbeda. Node
    /// [`AccessNode::hidden`] hilang beserta seluruh keturunannya.
    ///
    /// `focus` **sengaja dititipkan pemanggil**, bukan disimpan di pohon:
    /// pemegang fokus yang sah adalah [`crate::input::FocusManager`], dan dua
    /// tempat penyimpanan fokus berarti cepat atau lambat keduanya berbeda.
    /// Biasanya `router.focus().focused()`; `None` berarti window sendirilah
    /// yang memegang fokus (aturan AccessKit).
    pub fn access_tree(&self, focus: Option<NodeId>) -> AccessTree {
        AccessTree::emit(self, focus)
    }

    /// Nama window bagi teknologi bantu (biasanya judul window).
    ///
    /// Inilah hal pertama yang dibacakan screen reader saat aplikasi mendapat
    /// fokus, jadi ia bagian dari pohon a11y — bukan sekadar dekorasi titlebar.
    pub fn set_root_label(&mut self, label: impl Into<String>) {
        let root = self.root;
        let label = label.into();
        if let Some(r) = self.node_mut_ref::<Root>(root) {
            if r.label.as_deref() != Some(label.as_str()) {
                r.label = Some(label);
                self.dirty.insert(Dirty::PAINT);
            }
        }
    }

    // -- internal ---------------------------------------------------------

    fn alloc(
        &mut self,
        parent: Option<NodeId>,
        key: Option<Key>,
        type_id: TypeId,
        render: Box<dyn RenderNode>,
    ) -> NodeId {
        let node = Node {
            key,
            type_id,
            parent,
            children: Vec::new(),
            depth: 0,
            render: Some(render),
            size: Size::ZERO,
            offset: Point::ZERO,
            constraints: None,
            needs_layout: true,
            needs_paint: true,
            boundary: false,
            parent_uses_size: true,
            tight_is_boundary: true,
            queued: false,
            layout_count: 0,
            paint_cache: None,
            paint_count: 0,
        };
        match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                slot.node = Some(node);
                NodeId {
                    tree: self.id,
                    index,
                    generation: slot.generation,
                }
            }
            None => {
                let index = self.slots.len() as u32;
                self.slots.push(Slot {
                    generation: 0,
                    node: Some(node),
                });
                NodeId {
                    tree: self.id,
                    index,
                    generation: 0,
                }
            }
        }
    }

    fn index_of(&self, id: NodeId) -> Option<usize> {
        if id.tree != self.id {
            return None;
        }
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation || slot.node.is_none() {
            return None;
        }
        Some(id.index as usize)
    }

    fn node(&self, id: NodeId) -> Option<&Node> {
        let idx = self.index_of(id)?;
        self.slots[idx].node.as_ref()
    }

    fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let idx = self.index_of(id)?;
        self.slots[idx].node.as_mut()
    }
}

impl core::fmt::Debug for RenderTree {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RenderTree")
            .field("id", &self.id)
            .field("nodes", &self.len())
            .field("direction", &self.direction)
            .field("dirty", &self.dirty)
            .field("pending_boundaries", &self.dirty_boundaries.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// LayoutCtx
// ---------------------------------------------------------------------------

/// Akses terbatas ke pohon selama sebuah node menjalankan layout-nya.
///
/// Sengaja **tidak** menyediakan mutasi struktur: pohon hanya berubah lewat
/// view-diff. Yang boleh dilakukan node: melayout anak dan menempatkannya.
pub struct LayoutCtx<'a> {
    tree: &'a mut RenderTree,
    node: NodeId,
}

impl LayoutCtx<'_> {
    /// Node yang sedang di-layout.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// Arah baca yang berlaku (mirroring RTL, §9.8).
    pub fn direction(&self) -> TextDirection {
        self.tree.direction
    }

    /// Anak-anak node ini.
    pub fn children(&self) -> &[NodeId] {
        self.tree.children(self.node)
    }

    /// Jumlah anak.
    pub fn child_count(&self) -> usize {
        self.tree.children(self.node).len()
    }

    /// Anak ke-`index`.
    ///
    /// Panik bila di luar jangkauan — indeks anak selalu datang dari
    /// [`LayoutCtx::child_count`].
    pub fn child(&self, index: usize) -> NodeId {
        self.tree.children(self.node)[index]
    }

    /// Layout seorang anak; ukurannya boleh memengaruhi ukuran kita.
    pub fn layout_child(&mut self, child: NodeId, constraints: BoxConstraints) -> Size {
        debug_assert_eq!(
            self.tree.parent(child),
            Some(self.node),
            "hanya boleh melayout anak sendiri"
        );
        self.tree.layout_node(child, constraints, true, true)
    }

    /// Layout seorang anak dengan constraints yang **berasal dari hasil
    /// mengukur anak itu sendiri**.
    ///
    /// Bedanya dengan [`LayoutCtx::layout_child`] hanya satu, tapi penting:
    /// constraints tight di sini **tidak** menjadikan anak relayout boundary.
    /// Wadah flex/grid ([`super::TaffyBox`]) memberi anaknya ukuran pasti
    /// setelah menanyakan sendiri "kamu maunya sebesar apa?" — kalau isi anak
    /// berubah, jawabannya berubah dan seluruh flex harus dihitung ulang. Kalau
    /// anak dijadikan boundary di situ, perubahan itu tidak akan pernah sampai
    /// ke wadahnya dan layout membeku tanpa suara.
    pub fn layout_child_measured(&mut self, child: NodeId, constraints: BoxConstraints) -> Size {
        debug_assert_eq!(
            self.tree.parent(child),
            Some(self.node),
            "hanya boleh melayout anak sendiri"
        );
        self.tree.layout_node(child, constraints, true, false)
    }

    /// Gaya anak sebagai item flex/grid ([`RenderNode::layout_style`]).
    pub fn child_layout_style(&self, child: NodeId) -> ItemStyle {
        self.tree
            .render(child)
            .map(|n| n.layout_style())
            .unwrap_or(ItemStyle::DEFAULT)
    }

    /// Layout seorang anak yang ukurannya **tidak** memengaruhi ukuran kita.
    ///
    /// Anak otomatis menjadi relayout boundary: perubahan di dalamnya berhenti
    /// di situ. Ukurannya tetap dikembalikan agar bisa ditempatkan, tapi tidak
    /// boleh dipakai untuk menghitung ukuran sendiri.
    pub fn layout_child_boundary(&mut self, child: NodeId, constraints: BoxConstraints) -> Size {
        debug_assert_eq!(
            self.tree.parent(child),
            Some(self.node),
            "hanya boleh melayout anak sendiri"
        );
        self.tree.layout_node(child, constraints, false, true)
    }

    /// Ukuran anak dari layout terakhir.
    pub fn child_size(&self, child: NodeId) -> Size {
        self.tree.size(child)
    }

    /// **Induk yang menentukan posisi**: tempatkan anak relatif terhadap
    /// sudut kiri-atas node ini.
    pub fn place_child(&mut self, child: NodeId, offset: Point) {
        debug_assert_eq!(
            self.tree.parent(child),
            Some(self.node),
            "hanya boleh menempatkan anak sendiri"
        );
        let berubah = match self.tree.node_mut(child) {
            Some(n) if n.offset != offset => {
                n.offset = offset;
                true
            }
            _ => false,
        };
        if berubah {
            // Anak yang pindah menggeser seluruh keturunannya; yang perlu
            // ditandai adalah jalur ke atas, karena cache gambar leluhur
            // memuat gambar anak ini. Pergeseran keturunan tertangkap sendiri:
            // cache mereka menyimpan posisi absolut saat dibuat.
            self.tree.mark_needs_paint(child);
        }
    }
}

// ---------------------------------------------------------------------------
// Utilitas untuk lapisan view
// ---------------------------------------------------------------------------

/// Peta `key -> node` untuk anak-anak sebuah node; dipakai diffing berkunci.
///
/// Panik bila ada dua saudara berkunci sama: peta akan menelan salah satunya,
/// dan node yang tertelan tidak akan pernah dicocokkan maupun dibuang — yang
/// baru terlihat satu frame kemudian sebagai invariant arena yang meledak.
/// Lebih baik berisik di tempat kesalahannya (§9.7).
pub(crate) fn keyed_children(tree: &RenderTree, parent: NodeId) -> HashMap<Key, NodeId> {
    let mut map = HashMap::new();
    for id in tree.children(parent) {
        if let Some(key) = tree.key(*id) {
            if let Some(sebelumnya) = map.insert(key.clone(), *id) {
                panic!(
                    "kunci ganda di antara anak {parent:?}: {key:?} dipakai {sebelumnya:?} \
                     dan {id:?} — kunci wajib unik di antara saudara"
                );
            }
        }
    }
    map
}
