//! `text()` — the first Tier 0 component (`KOMPONEN.md`): text that
//! **actually shows up** in the render tree.
//!
//! ```
//! # use silka_widgets::{text, Fonts};
//! # use silka_theme::{Appearance, Theme};
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! text(&fonts, "Nilai: 3")
//!     .size(t.typography.body_size * 2.0)
//!     .color(t.color.label);
//! ```
//!
//! Three things make it part of the engine rather than something bolted on
//! beside it:
//!
//! 1. **Its size comes from `measure`, not from a guess.** This node is the
//!    "measure function leaf" of §3.4: the width limit handed down by the box
//!    constraints (or by a flex/grid container) is used as-is to wrap lines,
//!    and the resulting size travels back up to the parent.
//! 2. **It draws in local coordinates.** Glyphs are rasterized from `(0, 0)`,
//!    the node's top-left corner; [`silka_core::tree::PaintCtx`] lifts them
//!    into absolute coordinates — so moving the text touches not a single line
//!    of drawing code (§3.2).
//! 3. **A screen reader can read it.** The text content is the a11y node's
//!    `name`, with `bounds` coming from the layout result (§3.8).
//!
//! What is **not** here: the name `cosmic-text`, the name `wgpu`, and hard-coded
//! color numbers — everything is a token (§2.6, §3.2, §3.3).

use silka_core::access::{AccessNode, AccessRole};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, GlyphRun, Point, Size};
use silka_text::{FontWeight, TextConstraints, TextStyle, TextWrap};

use crate::fonts::Fonts;

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// The text leaf: it keeps both the source **and** the shaping result.
///
/// The source is kept because that is what diffing compares; the shaping
/// result is kept because shaping is the most expensive work in the whole
/// framework and must not be redone every frame (§3.3).
pub struct TextBox {
    text: String,
    style: TextStyle,
    color: Color,
    max_width: Option<f32>,
    role: AccessRole,
    fonts: Fonts,

    // -- derived (always a product of the fields above) --
    run: GlyphRun,
    size: Size,
    /// Width limit used for the last shaping pass (`INFINITY` = unbounded).
    shaped_width: f32,
    /// Scale factor at the last rasterization — atlas glyphs are tied to it.
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

    /// The width limit before the first layout: only what the view asked for.
    fn batas_awal(&self) -> f32 {
        self.max_width.unwrap_or(f32::INFINITY)
    }

    /// Shape and rasterize against a given width limit.
    ///
    /// Rasterization uses origin `(0, 0)`: each glyph's destination rect is
    /// relative to the node's top-left corner, like every other draw command.
    fn bentuk(&mut self, batas_lebar: f32) {
        let scale = self.fonts.scale_factor();
        let teks = &self.text;
        let gaya = &self.style;
        let warna = self.color;
        let (run, size) = self.fonts.with(|mesin| {
            // `TextConstraints::width(INFINITY)` = unbounded, so a single path
            // serves both a one-line label and a column of paragraph text.
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

    /// Make sure the shaping result is still valid for this width limit and DPI.
    ///
    /// There are two legitimate reasons to reshape, and only two: the column
    /// width changed (lines must be re-wrapped) and the scale factor changed
    /// (atlas glyph bitmaps are tied to the display resolution, §3.3).
    fn pastikan_bentuk(&mut self, batas_lebar: f32) {
        let scale = self.fonts.scale_factor();
        let sama_lebar = self.shaped_width == batas_lebar
            || (self.shaped_width.is_infinite() && batas_lebar.is_infinite());
        if sama_lebar && self.shaped_scale == scale {
            return;
        }
        self.bentuk(batas_lebar);
    }

    /// The text currently being displayed.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The natural size from the last measure, in logical points.
    pub fn measured_size(&self) -> Size {
        self.size
    }

    /// How many glyphs will be drawn.
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
        // Width limit = the tighter of what the view asked for and the space
        // actually available. This is "constraints down, sizes up" for text
        // (§3.4).
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
        // Text deliberately declared structural (e.g. the label inside a
        // button, whose name the button already announces) has no name of its
        // own — otherwise a screen reader would read it twice.
        if !self.text.is_empty() && self.role != AccessRole::Container {
            node.label = Some(self.text.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props for the text leaf.
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
        // Reshaping is deferred to layout: that is where the effective column
        // width is known, so changed text is not shaped twice.
        n.shaped_width = f32::NAN;
        Dirty::LAYOUT | Dirty::PAINT
    }
}

/// A Dart-style text builder (§2.5).
///
/// Created through [`text`]; becomes a [`View`] as soon as it is placed into
/// any container.
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    props: TextProps,
    key: Option<Key>,
}

/// Single-style text — the `text` component (`KOMPONEN.md` Tier 0).
///
/// `fonts` is the application's text engine ([`Fonts`]); it is passed
/// explicitly for as long as there is no ambient context for
/// application-level dependencies.
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

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Font size in logical points — **always** derived from the
    /// `typography` tokens (§2.6).
    pub fn size(self, size: f32) -> Self {
        self.map(move |p| p.style.size = size.max(0.5))
    }

    /// Font weight (one variable font, not four files — §3.6).
    pub fn weight(self, weight: FontWeight) -> Self {
        self.map(move |p| p.style.weight = weight)
    }

    /// Text color — the `label`/`secondary_label`/`on_accent` tokens.
    pub fn color(self, color: Color) -> Self {
        self.map(move |p| p.color = color)
    }

    /// Line height as a multiple of the font size.
    pub fn line_height(self, factor: f32) -> Self {
        self.map(move |p| p.style.line_height = factor)
    }

    /// Tracking (letter-spacing) in em — negative for big SF-style headlines.
    pub fn tracking(self, em: f32) -> Self {
        self.map(move |p| p.style.tracking = em)
    }

    /// Force a single line (no wrapping) — the shape labels and buttons use.
    pub fn single_line(self) -> Self {
        self.map(move |p| {
            p.style.wrap = TextWrap::None;
            p.style.max_lines = Some(1);
        })
    }

    /// Cap on the number of lines; the rest is clipped and marked as overflow.
    pub fn max_lines(self, lines: usize) -> Self {
        self.map(move |p| p.style.max_lines = Some(lines.max(1)))
    }

    /// Column width limit, in logical points. Without it, the width comes
    /// from the parent's constraints.
    pub fn max_width(self, width: f32) -> Self {
        self.map(move |p| p.max_width = Some(width.max(0.0)))
    }

    /// A complete style at once (e.g. one already assembled from tokens).
    pub fn style(self, style: TextStyle) -> Self {
        self.map(move |p| p.style = style)
    }

    /// The a11y role — [`AccessRole::Label`] by default.
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
    use silka_core::tree::RenderTree;
    use silka_core::view::reconcile;
    use silka_paint::{Command, Scene};
    use silka_text::TextConstraints;

    fn pohon(view: impl Into<silka_core::view::View>, batas: BoxConstraints) -> RenderTree {
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
        // Still one line tall, so the caret has somewhere to sit.
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
        // Re-layout: the node sees the changed scale factor and reshapes.
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
        // The a11y bounds come from the layout result, not from the widget.
        assert_eq!(e.bounds.size, tree.size(e.id));
    }
}
