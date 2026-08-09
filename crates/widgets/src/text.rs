//! `text()` — komponen Tier 0 pertama (`KOMPONEN.md`): teks yang **benar-benar
//! tampil** di dalam render tree.
//!
//! ```
//! # use rustui_widgets::{text, Fonts};
//! # use rustui_theme::{Appearance, Theme};
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! text(&fonts, "Nilai: 3")
//!     .size(t.typography.body_size * 2.0)
//!     .color(t.color.label);
//! ```
//!
//! Tiga hal yang membuatnya menyatu dengan mesin, bukan menempel di sampingnya:
//!
//! 1. **Ukurannya datang dari `measure`, bukan dari tebakan.** Node ini adalah
//!    "measure function leaf" §3.4: lebar batas yang turun dari box constraints
//!    (atau dari wadah flex/grid) dipakai apa adanya untuk membungkus baris,
//!    dan ukuran hasilnya naik ke induk.
//! 2. **Menggambar dalam koordinat lokal.** Glyph dirasterisasi dari `(0, 0)`
//!    sudut kiri-atas node; [`rustui_core::tree::PaintCtx`] yang menaikkannya
//!    ke koordinat absolut — jadi memindahkan teks tidak menyentuh satu baris
//!    pun kode gambar (§3.2).
//! 3. **Bisa dibacakan screen reader.** Isi teksnya adalah `name` node a11y,
//!    dengan `bounds` yang datang dari hasil layout (§3.8).
//!
//! Yang **tidak** ada di sini: nama `cosmic-text`, nama `wgpu`, dan angka warna
//! — semuanya token (§2.6, §3.2, §3.3).

use rustui_core::access::{AccessNode, AccessRole};
use rustui_core::scheduler::Dirty;
use rustui_core::signals::Key;
use rustui_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use rustui_core::view::{Builder, View, ViewNode};
use rustui_paint::{Color, GlyphRun, Point, Size};
use rustui_text::{FontWeight, TextConstraints, TextStyle, TextWrap};

use crate::fonts::Fonts;

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// Daun teks: menyimpan sumbernya **dan** hasil shaping-nya.
///
/// Sumbernya disimpan karena itulah yang dibandingkan saat diff; hasil
/// shaping-nya disimpan karena shaping adalah pekerjaan termahal di seluruh
/// framework dan tidak boleh diulang tiap frame (§3.3).
pub struct TextBox {
    text: String,
    style: TextStyle,
    color: Color,
    max_width: Option<f32>,
    role: AccessRole,
    fonts: Fonts,

    // -- turunan (selalu hasil dari yang di atas) --
    run: GlyphRun,
    size: Size,
    /// Lebar batas yang dipakai saat shaping terakhir (`INFINITY` = tanpa batas).
    shaped_width: f32,
    /// Scale factor saat rasterisasi terakhir — glyph di atlas terikat padanya.
    shaped_scale: f32,
}

impl TextBox {
    fn new(props: &TextProps) -> Self {
        let mut node = Self {
            text: props.text.clone(),
            style: props.style.clone(),
            color: props.color,
            max_width: props.max_width,
            role: props.role,
            fonts: props.fonts.clone(),
            run: GlyphRun::new(props.color),
            size: Size::ZERO,
            shaped_width: f32::NAN,
            shaped_scale: f32::NAN,
        };
        node.bentuk(node.batas_awal());
        node
    }

    /// Lebar batas sebelum layout pertama: hanya yang diminta view.
    fn batas_awal(&self) -> f32 {
        self.max_width.unwrap_or(f32::INFINITY)
    }

    /// Shape + rasterisasi terhadap lebar batas tertentu.
    ///
    /// Rasterisasi memakai origin `(0, 0)`: kotak tujuan tiap glyph relatif
    /// terhadap sudut kiri-atas node, sama seperti perintah gambar lain.
    fn bentuk(&mut self, batas_lebar: f32) {
        let scale = self.fonts.scale_factor();
        let teks = &self.text;
        let gaya = &self.style;
        let warna = self.color;
        let (run, size) = self.fonts.with(|mesin| {
            // `TextConstraints::width(INFINITY)` = tanpa batas, jadi satu jalur
            // saja melayani label satu baris maupun paragraf berkolom.
            let layout = mesin.layout(teks, gaya, TextConstraints::width(batas_lebar));
            let size = layout.measure().content_size;
            let run = mesin.rasterize(&layout, Point::ZERO, warna);
            (run, size)
        });
        self.run = run;
        self.size = size;
        self.shaped_width = batas_lebar;
        self.shaped_scale = scale;
    }

    /// Pastikan hasil shaping masih berlaku untuk lebar batas dan DPI ini.
    ///
    /// Dua alasan sah untuk shaping ulang, dan hanya dua: lebar kolom berubah
    /// (baris harus dibungkus ulang) dan scale factor berubah (bitmap glyph
    /// di atlas terikat pada resolusi layar, §3.3).
    fn pastikan_bentuk(&mut self, batas_lebar: f32) {
        let scale = self.fonts.scale_factor();
        let sama_lebar = self.shaped_width == batas_lebar
            || (self.shaped_width.is_infinite() && batas_lebar.is_infinite());
        if sama_lebar && self.shaped_scale == scale {
            return;
        }
        self.bentuk(batas_lebar);
    }

    /// Teks yang sedang ditampilkan.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Ukuran alami hasil ukur terakhir, poin logis.
    pub fn measured_size(&self) -> Size {
        self.size
    }

    /// Jumlah glyph yang akan digambar.
    pub fn glyph_count(&self) -> usize {
        self.run.len()
    }
}

impl std::fmt::Debug for TextBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextBox")
            .field("text", &self.text)
            .field("size", &self.size)
            .field("glyphs", &self.run.len())
            .finish()
    }
}

impl RenderNode for TextBox {
    fn type_name(&self) -> &'static str {
        "Text"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // Batas lebar = yang paling ketat antara permintaan view dan ruang yang
        // benar-benar tersedia. Inilah "constraints turun, ukuran naik" untuk
        // teks (§3.4).
        let batas = match self.max_width {
            Some(w) => w.min(constraints.max_width),
            None => constraints.max_width,
        };
        self.pastikan_bentuk(batas);
        constraints.constrain(self.size)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        if self.run.is_empty() {
            return;
        }
        ctx.glyph_run(self.run.clone());
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = self.role;
        // Teks yang sengaja dinyatakan struktural (mis. label di dalam tombol,
        // yang namanya sudah diumumkan tombolnya) tidak punya nama sendiri —
        // kalau tidak, screen reader membacakannya dua kali.
        if !self.text.is_empty() && self.role != AccessRole::Container {
            node.label = Some(self.text.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props daun teks.
#[derive(Debug, Clone, PartialEq)]
pub struct TextProps {
    text: String,
    style: TextStyle,
    color: Color,
    max_width: Option<f32>,
    role: AccessRole,
    fonts: Fonts,
}

impl ViewNode for TextProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(TextBox::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<TextBox>()
            .expect("tipe view sama berarti tipe render node sama");

        let mut dirty = Dirty::NONE;
        if n.role != self.role {
            n.role = self.role;
            dirty |= Dirty::PAINT;
        }
        let isi_berubah = n.text != self.text
            || n.style != self.style
            || n.color != self.color
            || n.max_width != self.max_width
            || n.fonts != self.fonts;
        if !isi_berubah {
            return dirty;
        }
        n.text.clone_from(&self.text);
        n.style.clone_from(&self.style);
        n.color = self.color;
        n.max_width = self.max_width;
        n.fonts = self.fonts.clone();
        // Shaping ulang ditunda ke layout: di sanalah lebar kolom yang berlaku
        // diketahui, jadi teks yang berubah tidak dibentuk dua kali.
        n.shaped_width = f32::NAN;
        Dirty::LAYOUT | Dirty::PAINT
    }
}

/// Builder teks bergaya Dart (§2.5).
///
/// Dibuat lewat [`text`]; menjadi [`View`] saat dimasukkan ke wadah mana pun.
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    props: TextProps,
    key: Option<Key>,
}

/// Teks satu gaya — komponen `text` (`KOMPONEN.md` Tier 0).
///
/// `fonts` adalah mesin teks aplikasi ([`Fonts`]); ia diserahkan eksplisit
/// selama belum ada context ambient untuk titipan tingkat aplikasi.
pub fn text(fonts: &Fonts, text: impl Into<String>) -> Text {
    Text {
        props: TextProps {
            text: text.into(),
            style: TextStyle::new(),
            color: Color::WHITE,
            max_width: None,
            role: AccessRole::Label,
            fonts: fonts.clone(),
        },
        key: None,
    }
}

impl Text {
    fn map(mut self, f: impl FnOnce(&mut TextProps)) -> Self {
        f(&mut self.props);
        self
    }

    /// Kunci identitas di antara saudara-saudaranya (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Ukuran font, poin logis — **selalu** diturunkan dari token
    /// `typography` (§2.6).
    pub fn size(self, size: f32) -> Self {
        self.map(move |p| p.style.size = size.max(0.5))
    }

    /// Berat font (satu variable font, bukan empat berkas — §3.6).
    pub fn weight(self, weight: FontWeight) -> Self {
        self.map(move |p| p.style.weight = weight)
    }

    /// Warna teks — token `label`/`secondary_label`/`on_accent`.
    pub fn color(self, color: Color) -> Self {
        self.map(move |p| p.color = color)
    }

    /// Tinggi baris sebagai kelipatan ukuran font.
    pub fn line_height(self, factor: f32) -> Self {
        self.map(move |p| p.style.line_height = factor)
    }

    /// Tracking (letter-spacing) dalam em — negatif untuk judul besar ala SF.
    pub fn tracking(self, em: f32) -> Self {
        self.map(move |p| p.style.tracking = em)
    }

    /// Paksa satu baris (tanpa wrap) — bentuk yang dipakai label dan tombol.
    pub fn single_line(self) -> Self {
        self.map(move |p| {
            p.style.wrap = TextWrap::None;
            p.style.max_lines = Some(1);
        })
    }

    /// Batas jumlah baris; sisanya dipotong dan ditandai overflow.
    pub fn max_lines(self, lines: usize) -> Self {
        self.map(move |p| p.style.max_lines = Some(lines.max(1)))
    }

    /// Batas lebar kolom, poin logis. Tanpa ini, lebarnya datang dari
    /// constraints induk.
    pub fn max_width(self, width: f32) -> Self {
        self.map(move |p| p.max_width = Some(width.max(0.0)))
    }

    /// Gaya lengkap sekaligus (mis. gaya yang sudah dirakit dari token).
    pub fn style(self, style: TextStyle) -> Self {
        self.map(move |p| p.style = style)
    }

    /// Peran a11y — bawaannya [`AccessRole::Label`].
    pub fn role(self, role: AccessRole) -> Self {
        self.map(move |p| p.role = role)
    }
}

impl From<Text> for View {
    fn from(t: Text) -> View {
        let mut b = Builder::new(t.props);
        if let Some(key) = t.key {
            b = b.key(key);
        }
        b.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustui_core::tree::RenderTree;
    use rustui_core::view::reconcile;
    use rustui_paint::{Command, Scene};
    use rustui_text::TextConstraints;

    fn pohon(view: impl Into<rustui_core::view::View>, batas: BoxConstraints) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(batas);
        tree
    }

    fn scene(tree: &mut RenderTree) -> Scene {
        let mut s = Scene::new(Color::BLACK);
        tree.paint_into(&mut s);
        s
    }

    fn glyph_run(scene: &Scene) -> GlyphRun {
        scene
            .commands()
            .iter()
            .find_map(|c| match c {
                Command::GlyphRun(r) => Some(r.clone()),
                _ => None,
            })
            .expect("teks harus menghasilkan perintah glyph")
    }

    #[test]
    fn ukurannya_sama_dengan_hasil_measure_mesin_teks() {
        let f = Fonts::bundled_only();
        let gaya = TextStyle::new().size(17.0).single_line();
        let harapan = f.with(|m| m.measure("Nilai: 3", &gaya, TextConstraints::UNBOUNDED));

        let tree = pohon(
            text(&f, "Nilai: 3").style(gaya.clone()),
            BoxConstraints::loose(Size::new(400.0, 200.0)),
        );
        let node = tree.children(tree.root())[0];
        let ukuran = tree.size(node);
        assert_eq!(ukuran.width, harapan.content_size.width);
        assert_eq!(ukuran.height, harapan.content_size.height);
        assert!(ukuran.width > 0.0 && ukuran.height > 0.0);
    }

    #[test]
    fn kolom_lebih_sempit_membungkus_lebih_banyak_baris() {
        let f = Fonts::bundled_only();
        let isi = "Musuh terbesar framework GUI baru bukan rendering, melainkan teks.";
        let lebar = pohon(
            text(&f, isi).size(13.0),
            BoxConstraints::loose(Size::new(600.0, 400.0)),
        );
        let sempit = pohon(
            text(&f, isi).size(13.0),
            BoxConstraints::loose(Size::new(180.0, 400.0)),
        );
        let tinggi = |t: &RenderTree| t.size(t.children(t.root())[0]).height;
        assert!(
            tinggi(&sempit) > tinggi(&lebar),
            "wrap harus mengikuti lebar yang turun dari constraints"
        );
    }

    #[test]
    fn glyph_digambar_di_dalam_kotak_nodenya() {
        let f = Fonts::bundled_only();
        let mut tree = pohon(
            text(&f, "Halo")
                .size(20.0)
                .color(Color::WHITE)
                .single_line(),
            BoxConstraints::loose(Size::new(400.0, 200.0)),
        );
        let ukuran = tree.size(tree.children(tree.root())[0]);
        let s = scene(&mut tree);
        let run = glyph_run(&s);
        assert!(run.len() >= 4, "empat huruf, minimal empat glyph");
        assert_eq!(run.color, Color::WHITE);
        let b = run.bounds().expect("run tidak kosong");
        assert!(b.min_x() >= -1.0 && b.min_y() >= -1.0, "{b:?}");
        assert!(b.max_x() <= ukuran.width + 1.0, "{b:?} vs {ukuran:?}");
        assert!(b.max_y() <= ukuran.height + 1.0, "{b:?} vs {ukuran:?}");
    }

    #[test]
    fn teks_kosong_tidak_menghasilkan_perintah_sama_sekali() {
        let f = Fonts::bundled_only();
        let mut tree = pohon(
            text(&f, "").size(13.0),
            BoxConstraints::loose(Size::new(200.0, 100.0)),
        );
        assert!(scene(&mut tree).is_empty());
        // Tetap setinggi satu baris supaya caret punya tempat.
        assert!(tree.size(tree.children(tree.root())[0]).height > 0.0);
    }

    #[test]
    fn mengganti_teks_membentuk_ulang_dan_menandai_layout() {
        let f = Fonts::bundled_only();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, text(&f, "0").size(40.0).single_line());
        tree.layout(BoxConstraints::loose(Size::new(400.0, 200.0)));
        let sebelum = glyph_run(&scene(&mut tree));

        let stat = reconcile(&mut tree, text(&f, "1234").size(40.0).single_line());
        assert_eq!(stat.created, 0, "node yang sama, hanya isinya berganti");
        assert!(tree.take_dirty().contains(Dirty::LAYOUT));
        tree.layout(BoxConstraints::loose(Size::new(400.0, 200.0)));
        let sesudah = glyph_run(&scene(&mut tree));
        assert!(sesudah.len() > sebelum.len());
        assert_ne!(sesudah.glyphs[0].image, sebelum.glyphs[0].image);
    }

    #[test]
    fn scale_factor_baru_merasterisasi_ulang_ke_atlas_beresolusi_lain() {
        let f = Fonts::bundled_only();
        let mut tree = pohon(
            text(&f, "Halo").size(17.0).single_line(),
            BoxConstraints::loose(Size::new(400.0, 200.0)),
        );
        let satu_x = glyph_run(&scene(&mut tree));

        f.set_scale_factor(2.0);
        // Layout ulang: node melihat scale factor berubah dan membentuk ulang.
        tree.layout(BoxConstraints::loose(Size::new(400.0, 199.0)));
        let dua_x = glyph_run(&scene(&mut tree));
        assert_ne!(
            satu_x.glyphs[0].image, dua_x.glyphs[0].image,
            "bitmap glyph terikat pada resolusi layar (§3.3)"
        );
    }

    #[test]
    fn teks_bisa_dibacakan_screen_reader() {
        let f = Fonts::bundled_only();
        let tree = pohon(
            text(&f, "Nilai: 3").size(13.0),
            BoxConstraints::loose(Size::new(400.0, 200.0)),
        );
        let pohon_a11y = tree.access_tree(None);
        let e = pohon_a11y
            .find_label("Nilai: 3")
            .unwrap_or_else(|| panic!("{}", pohon_a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Label);
        // Kotak a11y datang dari hasil layout, bukan dari widget.
        assert_eq!(e.bounds.size, tree.size(e.id));
    }
}
