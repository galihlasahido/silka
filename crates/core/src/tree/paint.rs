//! **Pass paint: render tree → [`Scene`]** (REKOMENDASI §3.2).
//!
//! Pass ketiga, sejajar dengan layout dan a11y — bukan lapisan susulan. Yang
//! keluar adalah satu daftar perintah gambar `silka-paint`; **tidak ada tipe
//! wgpu di mana pun di jalur ini**, dan tidak boleh pernah ada. Node render
//! berbicara dalam quad/shadow/glyph, backend menerjemahkannya.
//!
//! Tiga aturan yang mengatur pass ini, cerminan tiga aturan layout:
//!
//! 1. **Node menggambar dalam koordinat lokal.** Sama seperti layout, node
//!    tidak pernah tahu posisinya sendiri: `(0, 0)` adalah sudut kiri-atasnya
//!    dan [`PaintCtx`] yang menaikkannya ke koordinat absolut window. Konsekuensi
//!    langsungnya: memindahkan sebuah node tidak menyentuh kode gambar satu
//!    barisnya pun.
//! 2. **Induk digambar sebelum anak.** Urutan perintah di [`Scene`] adalah
//!    urutan gambar dari belakang ke depan, jadi anak selalu menumpuk di atas
//!    induknya. Node yang menimpa [`RenderNode::paint`] wajib memanggil
//!    [`PaintCtx::paint_children`] (atau [`PaintCtx::paint_child`]) sendiri —
//!    di situlah ia menentukan apa yang berada di bawah dan di atas anaknya.
//! 3. **Clip datang dari [`RenderNode::clips_children`]**, kontrak yang sama
//!    yang sudah dipakai hit-testing. Satu jawaban, dua pass: mustahil ada
//!    baris yang tergulir keluar layar tapi masih bisa diklik, atau sebaliknya.
//!
//! ## Melewati subtree yang bersih
//!
//! Perintah gambar satu subtree disimpan di **relayout boundary** — node yang
//! menjamin ukurannya tidak bergantung pada isinya, mis. viewport scroll
//! ([`RenderNode::is_relayout_boundary`]). Selama boundary itu tidak kotor
//! **dan** posisi absolut serta clip-nya tidak berubah, perintahnya
//! disalin kembali apa adanya — logika gambarnya tidak dijalankan ulang. Karena
//! itu `needs_paint` merambat **ke atas sampai akar** (lihat
//! [`RenderTree::mark_needs_paint`]): boundary yang bersih harus benar-benar
//! berarti "tidak ada apa pun di dalamku yang berubah".
//!
//! Akar sengaja **tidak** ikut menyimpan cache: pass paint hanya dipanggil saat
//! ada yang kotor (§3.5), jadi cache di akar akan selalu meleset dan hanya
//! menyalin seluruh frame dua kali.

use silka_paint::{Color, Command, Corners, GlyphRun, Point, Quad, Rect, Scene, ShadowPair, Size};

use super::arena::{NodeId, RenderTree};
// Hanya untuk tautan dokumentasi: kontrak pass ini hidup di `RenderNode`.
#[allow(unused_imports)]
use super::arena::RenderNode;

// ---------------------------------------------------------------------------
// Dekorasi
// ---------------------------------------------------------------------------

/// Latar sebuah node: isi, sudut, border, dan bayangan ganda.
///
/// **Nilainya selalu hasil resolusi token theme** (`surface`, `separator`,
/// `radius_md`, `shadow.md`) satu tingkat di atas — persis seperti `Insets`
/// pada [`super::PaddingBox`] yang sudah menerima sisi fisik, bukan `start`/
/// `end`. `silka-core` sengaja tidak mengenal `silka-theme`: mesin tidak
/// boleh punya pendapat tentang warna, dan preset Cupertino/Tailwind (§2.7)
/// berganti tanpa satu baris pun berubah di sini.
///
/// Bentuk sudut ikut sebagai **parameter**, bukan konstanta: squircle
/// Cupertino dan arc Tailwind adalah dua nilai [`Corners`] yang sama sahnya
/// (§2.7, §3.6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decoration {
    /// Warna isi.
    pub background: Color,
    /// Geometri sudut — mengalir apa adanya ke shader **dan** ke hit-testing.
    pub corners: Corners,
    /// Tebal border (0 = tanpa border).
    pub border_width: f32,
    /// Warna border.
    pub border_color: Color,
    /// Bayangan ganda ala HIG (ambient + key).
    pub shadows: ShadowPair,
}

impl Default for Decoration {
    /// Tidak menggambar apa pun: bawaan sebuah node adalah **tidak terlihat**,
    /// sehingga warna hanya muncul kalau memang ada token yang memintanya.
    fn default() -> Self {
        Self::NONE
    }
}

impl Decoration {
    /// Tanpa gambar apa pun — node yang murni struktural.
    pub const NONE: Decoration = Decoration {
        background: Color::TRANSPARENT,
        corners: Corners::SHARP,
        border_width: 0.0,
        border_color: Color::TRANSPARENT,
        shadows: ShadowPair::NONE,
    };

    /// Latar polos berwarna `background`.
    pub fn fill(background: Color) -> Self {
        Self {
            background,
            ..Self::NONE
        }
    }

    /// Setel geometri sudut.
    pub fn corners(mut self, corners: Corners) -> Self {
        self.corners = corners;
        self
    }

    /// Setel border.
    pub fn border(mut self, width: f32, color: Color) -> Self {
        self.border_width = width.max(0.0);
        self.border_color = color;
        self
    }

    /// Setel bayangan ganda.
    pub fn shadows(mut self, shadows: ShadowPair) -> Self {
        self.shadows = shadows;
        self
    }

    /// Benar bila dekorasi ini menyumbang piksel sama sekali.
    ///
    /// Elevasi 0 dan latar transparan **gratis**: tidak ada perintah yang
    /// dibuat, jadi node struktural tidak membebani scene.
    pub fn is_visible(&self) -> bool {
        self.background.a > 0.0
            || (self.border_width > 0.0 && self.border_color.a > 0.0)
            || self.shadows.is_visible()
    }
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

/// Perintah gambar satu subtree, siap dipakai ulang.
///
/// Disimpan bersama **syarat berlakunya**: posisi absolut dan clip saat ia
/// dibuat. Keduanya dicek sebelum dipakai, sehingga node yang bergeser (atau
/// yang clip-nya berubah karena guliran) tidak pernah menampilkan geometri
/// basi meski `needs_paint`-nya kebetulan bersih.
pub(super) struct PaintCache {
    pub(super) origin: Point,
    pub(super) clip: Option<Rect>,
    pub(super) commands: Vec<Command>,
}

// ---------------------------------------------------------------------------
// PaintCtx
// ---------------------------------------------------------------------------

/// Akses terbatas ke scene selama sebuah node menggambar dirinya.
///
/// Kosakatanya **hanya** `silka-paint` — quad, shadow, glyph run. Tidak ada
/// jalan dari sini ke tipe grafis backend, dan itu disengaja: kalau nanti ada
/// backend GL/CPU, ia masuk di satu tempat tanpa menyentuh satu widget pun
/// (§3.2).
///
/// Semua koordinat yang diterima method di sini adalah **lokal**: `(0, 0)`
/// adalah sudut kiri-atas node yang sedang menggambar.
pub struct PaintCtx<'a> {
    tree: &'a mut RenderTree,
    scene: &'a mut Scene,
    node: NodeId,
    origin: Point,
    size: Size,
    /// Clip yang berlaku untuk gambar node ini sendiri (absolut).
    clip: Option<Rect>,
    /// Clip yang berlaku untuk anak-anaknya (absolut) — sudah termasuk kotak
    /// node ini bila ia memotong isinya.
    child_clip: Option<Rect>,
    /// Benar bila node ini memotong isinya, sehingga anak perlu dibungkus
    /// [`Command::PushClip`].
    clips: bool,
    /// Benar selama sebuah pembungkus clip sedang terbuka — penjaga agar
    /// `paint_child` di dalam `paint_children` tidak membuka pembungkus kedua.
    clip_open: bool,
}

impl PaintCtx<'_> {
    /// Node yang sedang menggambar.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// Ukuran node ini dari layout terakhir.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Kotak node ini dalam koordinat **lokal**: selalu berpangkal di `(0, 0)`.
    pub fn local_bounds(&self) -> Rect {
        Rect::from_origin_size(Point::ZERO, self.size)
    }

    /// Kotak potong yang berlaku, dalam koordinat **lokal**.
    ///
    /// `None` berarti tidak ada yang memotong. Berguna bagi node yang bisa
    /// menggambar lebih hemat kalau tahu batasnya (mis. daftar tervirtualisasi).
    pub fn clip(&self) -> Option<Rect> {
        self.clip.map(|c| {
            Rect::from_origin_size(
                Point::new(c.origin.x - self.origin.x, c.origin.y - self.origin.y),
                c.size,
            )
        })
    }

    /// Benar bila kotak lokal ini menyumbang piksel di dalam clip yang berlaku.
    pub fn is_visible(&self, local: Rect) -> bool {
        terlihat(local.translated(self.origin), self.clip)
    }

    /// Gambar sebuah kotak (koordinat lokal).
    ///
    /// Radius sudut otomatis dibatasi terhadap ukuran kotak, sehingga bentuk
    /// yang dikirim ke shader tidak pernah mustahil.
    pub fn quad(&mut self, quad: Quad) -> &mut Self {
        let quad = self.absolutkan(quad);
        if terlihat(quad.rect, self.clip) {
            self.scene.push(quad);
        }
        self
    }

    /// Gambar sebuah kotak beserta bayangan gandanya (ambient + key).
    ///
    /// Urutannya ditentukan `silka-paint`: ambient, key, baru kotaknya.
    pub fn shadowed(&mut self, quad: Quad, shadows: ShadowPair) -> &mut Self {
        let quad = self.absolutkan(quad);
        for lapis in shadows.layers() {
            if !lapis.is_visible() {
                continue;
            }
            let bayangan = silka_paint::ShadowQuad::for_quad(&quad, lapis);
            // Ekor gaussian ikut diperhitungkan: bayangan yang kotaknya di luar
            // clip masih bisa menyumbang piksel di dalamnya.
            if bayangan.is_visible() && terlihat(bayangan.bounds(), self.clip) {
                self.scene.push(bayangan);
            }
        }
        if terlihat(quad.rect, self.clip) {
            self.scene.push(quad);
        }
        self
    }

    /// Gambar latar, border, dan bayangan sebuah [`Decoration`] pada seluruh
    /// kotak node ini.
    ///
    /// Inilah jalur yang dipakai semua primitif: warna datang dari token, dan
    /// dekorasi yang tak terlihat tidak menghasilkan perintah sama sekali.
    pub fn decorate(&mut self, decoration: &Decoration) -> &mut Self {
        if !decoration.is_visible() || self.size.is_empty() {
            return self;
        }
        let quad = Quad::new(self.local_bounds())
            .background(decoration.background)
            .corners(decoration.corners)
            .border(decoration.border_width, decoration.border_color);
        self.shadowed(quad, decoration.shadows)
    }

    /// Gambar sekumpulan glyph sewarna (koordinat lokal).
    ///
    /// Glyph yang seluruhnya di luar clip dibuang di sini, di CPU: satu run
    /// panjang di dalam scroll view tidak dikirim utuh ke GPU hanya karena
    /// sebagian kecilnya terlihat.
    pub fn glyph_run(&mut self, run: GlyphRun) -> &mut Self {
        let mut absolut = GlyphRun::with_capacity(run.color, run.glyphs.len());
        absolut.clip = run.clip.map(|c| c.translated(self.origin));
        for glyph in &run.glyphs {
            let bounds = glyph.bounds.translated(self.origin);
            if !terlihat(bounds, self.clip) {
                continue;
            }
            absolut.push(silka_paint::Glyph::new(glyph.image, bounds));
        }
        if !absolut.is_empty() {
            self.scene.push(absolut);
        }
        self
    }

    // -- anak --------------------------------------------------------------

    /// Anak-anak node ini, dalam urutan gambar.
    pub fn children(&self) -> &[NodeId] {
        self.tree.children(self.node)
    }

    /// Jumlah anak.
    pub fn child_count(&self) -> usize {
        self.tree.children(self.node).len()
    }

    /// Anak ke-`index`. Panik bila di luar jangkauan.
    pub fn child(&self, index: usize) -> NodeId {
        self.tree.children(self.node)[index]
    }

    /// Gambar seorang anak **di atas** apa yang sudah digambar sejauh ini.
    pub fn paint_child(&mut self, child: NodeId) {
        debug_assert_eq!(
            self.tree.parent(child),
            Some(self.node),
            "hanya boleh menggambar anak sendiri"
        );
        if self.clip_open {
            self.gambar_anak(child);
        } else {
            self.dengan_clip(|ctx| ctx.gambar_anak(child));
        }
    }

    /// Gambar semua anak, urut — yang terakhir berada paling atas.
    ///
    /// Inilah perilaku bawaan [`RenderNode::paint`]: node yang tidak menggambar
    /// apa pun sendiri tetap menurunkan isinya.
    pub fn paint_children(&mut self) {
        if self.child_count() == 0 {
            return;
        }
        if self.clip_open {
            self.semua_anak();
        } else {
            self.dengan_clip(|ctx| ctx.semua_anak());
        }
    }

    fn semua_anak(&mut self) {
        let kids: Vec<NodeId> = self.tree.children(self.node).to_vec();
        for child in kids {
            self.gambar_anak(child);
        }
    }

    fn gambar_anak(&mut self, child: NodeId) {
        paint_node(self.tree, self.scene, child, self.origin, self.child_clip);
    }

    /// Bungkus gambar anak dengan perintah clip bila node ini memotong isinya.
    ///
    /// Pembungkus yang ternyata tidak berisi apa pun dibatalkan: scene tidak
    /// boleh memuat pasangan clip kosong yang memaksa backend menyetel scissor
    /// tanpa alasan.
    fn dengan_clip(&mut self, f: impl FnOnce(&mut Self)) {
        let Some(clip) = self.child_clip.filter(|_| self.clips) else {
            f(self);
            return;
        };
        if clip.size.is_empty() {
            // Viewport yang menyusut jadi nol: isinya tidak bisa terlihat sama
            // sekali, jadi tidak ada gunanya menelusurinya.
            return;
        }
        let sebelum = self.scene.len();
        self.scene.push(Command::PushClip(clip));
        let dibuka = self.clip_open;
        self.clip_open = true;
        f(self);
        self.clip_open = dibuka;
        if self.scene.len() == sebelum + 1 {
            self.scene.truncate(sebelum);
        } else {
            self.scene.push(Command::PopClip);
        }
    }

    fn absolutkan(&self, quad: Quad) -> Quad {
        Quad {
            rect: quad.rect.translated(self.origin),
            ..quad
        }
        .normalized()
    }
}

fn terlihat(rect: Rect, clip: Option<Rect>) -> bool {
    if rect.size.is_empty() {
        return false;
    }
    match clip {
        Some(c) => rect.intersects(c),
        None => true,
    }
}

// ---------------------------------------------------------------------------
// Traversal
// ---------------------------------------------------------------------------

/// Jalankan pass paint atas seluruh pohon ke dalam `scene`.
pub(super) fn paint_tree(tree: &mut RenderTree, scene: &mut Scene) {
    let root = tree.root();
    paint_node(tree, scene, root, Point::ZERO, None);
}

fn paint_node(
    tree: &mut RenderTree,
    scene: &mut Scene,
    id: NodeId,
    parent_origin: Point,
    clip: Option<Rect>,
) {
    let Some((offset, size, needs_paint, boundary)) = tree.paint_geometry(id) else {
        return;
    };
    let origin = Point::new(parent_origin.x + offset.x, parent_origin.y + offset.y);

    // Akar tidak ikut menyimpan cache: pass ini hanya berjalan saat ada yang
    // kotor, jadi cache di akar dijamin meleset dan hanya menyalin frame dua kali.
    let cacheable = boundary && tree.parent(id).is_some();
    if cacheable && !needs_paint {
        if let Some(cache) = tree.paint_cache(id) {
            if cache.origin == origin && cache.clip == clip {
                scene.push_all(&cache.commands);
                return;
            }
        }
    }

    let awal = scene.len();
    let Some(render) = tree.take_render(id) else {
        debug_assert!(
            false,
            "{id:?} sedang menggambar — paint rekursif tidak diizinkan"
        );
        return;
    };
    let clips = render.clips_children();
    // `None` di jalur hilir berarti "tanpa batas", jadi irisan kosong TIDAK boleh
    // dipetakan ke `None`: itu akan menukar "tidak ada yang terlihat" dengan
    // "semuanya terlihat" dan meloloskan isi node ke scene. Rect degenerate
    // (ukuran nol) dipakai sebagai sentinel — `dengan_clip` memotong penelusuran
    // dan `terlihat` menolak semua rect terhadapnya.
    let child_clip = if clips {
        let sendiri = Rect::from_origin_size(origin, size);
        Some(match clip {
            Some(c) => sendiri
                .intersect(c)
                .unwrap_or(Rect::from_origin_size(origin, Size::ZERO)),
            None => sendiri,
        })
    } else {
        clip
    };
    {
        let mut ctx = PaintCtx {
            tree,
            scene,
            node: id,
            origin,
            size,
            clip,
            child_clip,
            clips,
            clip_open: false,
        };
        render.paint(&mut ctx);
    }
    tree.put_render(id, render);

    let cache = cacheable.then(|| PaintCache {
        origin,
        clip,
        commands: scene.commands()[awal..].to_vec(),
    });
    tree.finish_paint(id, cache);
}
