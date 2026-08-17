//! `text()` — the first Tier 0 component (`KOMPONEN.md`): text that
//! **actually shows up** in the render tree.
//!
//! ```
//! # use silka_widgets::{text, Fonts};
//! # use silka_theme::{Appearance, Theme};
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! text_in(&fonts, "Value: 3")
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
use silka_core::view::{active_theme, Builder, TextStyled, View, ViewNode};
use silka_paint::{Color, GlyphRun, Point, Size};
use silka_text::{FontWeight, TextConstraints, TextStyle, TextWrap};
use silka_theme::{ColorToken, FontToken, Token};

use crate::fonts::Fonts;

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// The text leaf: it keeps both the source **and** the shaping result.
///
/// The source is kept because that is what diffing compares; the shaping
/// result is kept because shaping is the most expensive work in the whole
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_widgets::{text_in, Fonts, TextBox};
///
/// let fonts = Fonts::bundled_only();
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, text_in(&fonts, "the quick brown fox").size(15.0));
///
/// // Wrapping follows the width handed down by the constraints — the leaf
/// // never decides its own width.
/// tree.layout(BoxConstraints::tight(Size::new(60.0, 200.0)));
/// let id = tree.children(tree.root())[0];
/// let narrow = tree
///     .node_ref::<TextBox>(id)
///     .expect("a text node")
///     .measured_size();
///
/// tree.layout(BoxConstraints::loose(Size::new(600.0, 200.0)));
/// let wide = tree.node_ref::<TextBox>(id).unwrap().measured_size();
/// assert!(narrow.height > wide.height);
///
/// // The node keeps both the source and the shaping result, so a repaint
/// // never reshapes — the most expensive work in the framework happens once.
/// let node = tree.node_ref::<TextBox>(id).unwrap();
/// assert_eq!(node.text(), "the quick brown fox");
/// assert!(node.glyph_count() > 0);
/// ```
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
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_widgets::{text_in, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let build = |s: &str| text_in(&fonts, s.to_string()).size(15.0);
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, build("Count 0"));
/// tree.layout(BoxConstraints::loose(Size::new(320.0, 200.0)));
///
/// // Identical text is compared, found equal, and nothing is reshaped.
/// assert!(reconcile(&mut tree, build("Count 0")).is_noop());
///
/// // Different text updates the same node — which is what lets the shaping
/// // cache stay warm across a counter ticking upward.
/// let changed = reconcile(&mut tree, build("Count 1"));
/// assert_eq!(changed.replaced, 0);
/// assert!(changed.updated > 0);
/// ```
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
///
/// ```
/// use silka_core::signals::Key;
/// use silka_core::view::column;
/// use silka_theme::FontToken;
/// use silka_widgets::{text_in, Fonts};
///
/// let fonts = Fonts::bundled_only();
///
/// // A builder becomes a view the moment it is placed in a container, so it
/// // is never converted by hand.
/// let page = column([
///     text_in(&fonts, "Title").font(FontToken::Title3),
///     text_in(&fonts, "Body").text_sm().max_lines(3),
/// ]);
/// # let _ = page;
///
/// // A key gives a line a stable identity across a reorder.
/// let keyed = text_in(&fonts, "Row").key(Key::from("row-1"));
/// # let _ = keyed;
/// ```
/// any container.
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    props: TextProps,
    key: Option<Key>,
}

/// Single-style text — the `text` component (`KOMPONEN.md` Tier 0).
///
/// The Dart-style shape of §2.5: the content, then a method chain. The text
/// engine comes from [`crate::active_fonts`] and the tokens from the ambient
/// theme, so neither appears at the call site.
///
/// ```
/// use silka_theme::FontToken;
/// use silka_widgets::{text, Fonts};
///
/// // One line, no ceremony — this is the shape §2.5 promised.
/// let title = text("Inbox").font(FontToken::Title2);
///
/// // Tailwind-style utilities are the same vocabulary under shorter names.
/// let caption = text("3 unread").text_xs().font_medium();
/// # let _ = (title, caption, Fonts::bundled_only());
/// ```
///
/// Use [`text_in`] when the view is built outside a build pass and the engine
/// has to be spelled out.
pub fn text(text: impl Into<String>) -> Text {
    text_in(&crate::active_fonts(), text)
}

/// [`text`] with the text engine passed explicitly.
///
/// ```
/// use silka_theme::{Appearance, FontToken, Theme};
/// use silka_text::FontWeight;
/// use silka_widgets::{text_in, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// // A label: size and weight come from a *token*, so the same line is right
/// // under both presets.
/// let title = text_in(&fonts, "Inbox")
///     .font(FontToken::Title2)
///     .color(theme.color.label);
///
/// // Tailwind-style utilities are the same vocabulary under shorter names.
/// let caption = text_in(&fonts, "3 unread")
///     .text_xs()
///     .font_medium()
///     .color(theme.color.secondary_label);
///
/// // A single line that must not wrap — the shape a button label or a table
/// // cell needs.
/// let cell = text_in(&fonts, "a very long value that will not fit")
///     .single_line()
///     .max_width(120.0);
/// # let _ = (title, caption, cell, FontWeight::MEDIUM);
/// ```
pub fn text_in(fonts: &Fonts, text: impl Into<String>) -> Text {
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

    /// A typography token that has **already been resolved** against a theme.
    ///
    /// The same four properties [`Text::font`] applies — size, line height,
    /// weight and tracking — except that the theme is the caller's rather than
    /// the ambient one. That is the difference every `*_in` constructor needs:
    /// a view built outside a build pass has no ambient theme to resolve
    /// against, and picking the four apart by hand at each call site is how a
    /// type scale drifts.
    ///
    /// ```
    /// use silka_theme::{Appearance, Theme};
    /// use silka_widgets::{text_in, Fonts};
    ///
    /// let fonts = Fonts::bundled_only();
    /// let theme = Theme::cupertino(Appearance::Dark);
    /// let caption = text_in(&fonts, "3 unread").type_style(theme.typography.footnote);
    /// # let _ = caption;
    /// ```
    pub fn type_style(self, style: silka_theme::TypeStyle) -> Self {
        self.map(move |p| {
            p.style.size = style.size.max(0.5);
            p.style.line_height = style.line_height;
            p.style.weight = FontWeight(style.weight);
            p.style.tracking = style.tracking;
        })
    }

    /// The a11y role — [`AccessRole::Label`] by default.
    pub fn role(self, role: AccessRole) -> Self {
        self.map(move |p| p.role = role)
    }
}

// ---------------------------------------------------------------------------
// The typography vocabulary (§2.6)
// ---------------------------------------------------------------------------

/// The token half of the builder — the **normal** way to style text.
///
/// [`Text::size`]/[`Text::weight`]/[`Text::color`] above take resolved values
/// and stay as the layer underneath; what an application writes is a **role**:
///
/// ```
/// # use silka_widgets::{text, Fonts};
/// # use silka_theme::{Appearance, ColorToken, FontToken, Theme};
/// # use silka_core::view::with_theme;
/// # let fonts = Fonts::bundled_only();
/// with_theme(Theme::cupertino(Appearance::Dark), || {
///     text_in(&fonts, "Value: 3")
///         .font(FontToken::Title2)
///         .text_color(ColorToken::Label);
/// });
/// ```
///
/// Four properties travel together in one call — size, line height, weight and
/// tracking — because that is what a typographic role *is*; picking them apart
/// is how a type scale drifts. Resolution happens against the ambient theme
/// ([`silka_core::view::with_theme`]), which the frame installs, so the call
/// site never names a theme (§2.5).
impl Text {
    /// The full text style of one typography token: size, line height, weight
    /// and tracking at once.
    pub fn font(self, token: FontToken) -> Self {
        let gaya = token.resolve(&active_theme());
        self.map(move |p| {
            p.style.size = gaya.size;
            p.style.line_height = gaya.line_height;
            p.style.weight = FontWeight(gaya.weight);
            p.style.tracking = gaya.tracking;
        })
    }

    /// The smallest text — [`FontToken::Caption2`].
    pub fn text_xs(self) -> Self {
        self.font(FontToken::Caption2)
    }

    /// Small supporting text — [`FontToken::Footnote`].
    pub fn text_sm(self) -> Self {
        self.font(FontToken::Footnote)
    }

    /// The UI default — [`FontToken::Body`].
    pub fn text_base(self) -> Self {
        self.font(FontToken::Body)
    }

    /// A small title — [`FontToken::Title3`].
    pub fn text_lg(self) -> Self {
        self.font(FontToken::Title3)
    }

    /// A medium title — [`FontToken::Title2`].
    pub fn text_xl(self) -> Self {
        self.font(FontToken::Title2)
    }

    /// A large title — [`FontToken::Title1`].
    pub fn text_2xl(self) -> Self {
        self.font(FontToken::Title1)
    }

    /// A page title — [`FontToken::LargeTitle`].
    pub fn text_3xl(self) -> Self {
        self.font(FontToken::LargeTitle)
    }

    /// Weight 400 — body text.
    pub fn font_regular(self) -> Self {
        self.weight(FontWeight::REGULAR)
    }

    /// Weight 500 — control labels.
    pub fn font_medium(self) -> Self {
        self.weight(FontWeight::MEDIUM)
    }

    /// Weight 600 — HIG-style titles.
    pub fn font_semibold(self) -> Self {
        self.weight(FontWeight::SEMIBOLD)
    }

    /// Weight 700 — large titles.
    pub fn font_bold(self) -> Self {
        self.weight(FontWeight::BOLD)
    }

    /// The glyph color, named by its role (`Label`, `SecondaryLabel`,
    /// `OnAccent`, …).
    pub fn text_color(self, token: ColorToken) -> Self {
        let warna = token.resolve(&active_theme());
        self.map(move |p| p.color = warna)
    }

    /// **Escape hatch**: a glyph color that is not a token — syntax
    /// highlighting, a user-picked label color. Spelled `_raw` so it shows up
    /// in review (§2.6).
    pub fn text_color_raw(self, color: Color) -> Self {
        self.color(color)
    }
}

/// The trait behind the vocabulary, implemented so that a `Builder<TextProps>`
/// — the form these props take once they are inside a view tree — speaks it
/// too.
impl TextStyled for TextProps {
    fn text_style_mut(&mut self) -> &mut TextStyle {
        &mut self.style
    }

    fn text_color_mut(&mut self) -> &mut Color {
        &mut self.color
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
            text_in(&f, "Nilai: 3").style(gaya.clone()),
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
            text_in(&f, isi).size(13.0),
            BoxConstraints::loose(Size::new(600.0, 400.0)),
        );
        let sempit = pohon(
            text_in(&f, isi).size(13.0),
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
            text_in(&f, "Halo")
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
            text_in(&f, "").size(13.0),
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
        reconcile(&mut tree, text_in(&f, "0").size(40.0).single_line());
        tree.layout(BoxConstraints::loose(Size::new(400.0, 200.0)));
        let sebelum = glyph_run(&scene(&mut tree));

        let stat = reconcile(&mut tree, text_in(&f, "1234").size(40.0).single_line());
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
            text_in(&f, "Halo").size(17.0).single_line(),
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
            text_in(&f, "Nilai: 3").size(13.0),
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

    // -- the typography vocabulary (§2.6) ---------------------------------

    #[test]
    fn font_token_membawa_empat_properti_sekaligus() {
        use silka_core::view::with_theme;
        use silka_theme::{Appearance, Theme};

        let f = Fonts::bundled_only();
        let t = Theme::cupertino(Appearance::Light);
        with_theme(t, || {
            let v = text_in(&f, "Judul").font(FontToken::Title2);
            let gaya = t.typography.title2;
            assert_eq!(v.props.style.size, gaya.size);
            assert_eq!(v.props.style.line_height, gaya.line_height);
            assert_eq!(v.props.style.weight, FontWeight(gaya.weight));
            assert_eq!(v.props.style.tracking, gaya.tracking);
        });
    }

    #[test]
    fn peran_yang_sama_berukuran_beda_di_tiap_preset() {
        use silka_core::view::with_theme;
        use silka_theme::{Appearance, Theme};

        let f = Fonts::bundled_only();
        let ukuran = |t: Theme| with_theme(t, || text_in(&f, "x").text_xl().props.style.size);
        // One call site, two presets, two numbers — which is the whole point of
        // naming the role instead of the size.
        assert_ne!(
            ukuran(Theme::cupertino(Appearance::Light)),
            ukuran(Theme::tailwind(Appearance::Light))
        );
    }

    #[test]
    fn warna_teks_diresolusi_dari_token_bukan_dari_literal() {
        use silka_core::view::with_theme;
        use silka_theme::{Appearance, Theme};

        let f = Fonts::bundled_only();
        for t in [
            Theme::cupertino(Appearance::Light),
            Theme::cupertino(Appearance::Dark),
        ] {
            with_theme(t, || {
                let v = text_in(&f, "x").text_color(ColorToken::SecondaryLabel);
                assert_eq!(v.props.color, t.color.secondary_label);
            });
        }
    }
}
