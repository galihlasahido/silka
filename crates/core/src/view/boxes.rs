//! Views for `crate::tree` — `align()`, `center()`, `stack()` and
//! `aspect_ratio()`, the rest of `KOMPONEN.md` Tier 1 written in the Dart style
//! (§2.5).
//!
//! ```
//! use silka_core::view::{align, aspect_ratio, center, div, fixed, stack, View, ASPECT_16_9};
//! use silka_core::tree::Alignment;
//! use silka_theme::{ColorToken, RadiusToken};
//!
//! // A badge pinned to the top-end corner of a tile: the base child fills the
//! // stack, so the badge's own `align` is what picks the corner.
//! let tile = stack([
//!     View::from(div().bg(ColorToken::Surface).rounded_lg()),
//!     View::from(align(fixed(20.0, 16.0)).alignment(Alignment::TOP_END)),
//! ])
//! .expand();
//!
//! // A 16:9 frame that keeps its shape at any column width.
//! let hero = aspect_ratio(ASPECT_16_9, div().rounded(RadiusToken::Lg));
//!
//! // A message in the middle of whatever space it is given.
//! let empty = center(fixed(120.0, 20.0)).bg(ColorToken::SurfaceSunken);
//! # let _ = (tile, hero, empty);
//! ```
//!
//! All three speak the same styling vocabulary as every other container
//! ([`Decorated`]), so `bg`/`rounded`/`border`/`shadow` work on them without
//! being written a fourth time.

use crate::scheduler::Dirty;
use crate::tree::{
    AccessRole, AlignBox, Alignment, AspectRatioBox, Decoration, RenderNode, StackBox, StackFit,
};

use super::primitives::Decorated;
use super::{Builder, View, ViewNode};

/// Widescreen video and most hero images.
pub const ASPECT_16_9: f32 = 16.0 / 9.0;
/// The classic photograph.
pub const ASPECT_4_3: f32 = 4.0 / 3.0;
/// A square — avatars, tiles, icons in a grid.
pub const ASPECT_SQUARE: f32 = 1.0;
/// The card shape most dashboard tiles take.
pub const ASPECT_3_2: f32 = 3.0 / 2.0;

/// Clamp a factor a caller computed: never negative, never `NaN`.
fn sane_factor(v: f32) -> f32 {
    if v.is_finite() {
        v.max(0.0)
    } else {
        1.0
    }
}

// ---------------------------------------------------------------------------
// align / center
// ---------------------------------------------------------------------------

/// Props for [`align`] and [`center`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AlignProps {
    alignment: Alignment,
    width_factor: Option<f32>,
    height_factor: Option<f32>,
    decoration: Decoration,
}

impl Decorated for AlignProps {
    fn decoration_mut(&mut self) -> &mut Decoration {
        &mut self.decoration
    }
}

impl ViewNode for AlignProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(AlignBox {
            alignment: self.alignment,
            width_factor: self.width_factor,
            height_factor: self.height_factor,
            decoration: self.decoration,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<AlignBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;
        if n.alignment != self.alignment
            || n.width_factor != self.width_factor
            || n.height_factor != self.height_factor
        {
            n.alignment = self.alignment;
            n.width_factor = self.width_factor;
            n.height_factor = self.height_factor;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        // A decoration never changes size, so it repaints and does not relayout.
        if n.decoration != self.decoration {
            n.decoration = self.decoration;
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

/// Place `child` somewhere inside the space this box is given — Flutter's
/// `Align`.
///
/// ```
/// use silka_core::tree::Alignment;
/// use silka_core::view::{align, fixed};
///
/// // A timestamp pinned to the bottom-end corner of whatever contains it.
/// let stamp = align(fixed(48.0, 14.0)).alignment(Alignment::BOTTOM_END);
/// # let _ = stamp;
/// ```
pub fn align(child: impl Into<View>) -> Builder<AlignProps> {
    Builder::new(AlignProps::default()).child(child)
}

/// Place `child` in the middle — [`align`] at [`Alignment::CENTER`], which is
/// the default anyway, under the name that says what it does.
///
/// ```
/// use silka_core::view::{center, fixed};
///
/// let _ = center(fixed(120.0, 20.0));
/// ```
pub fn center(child: impl Into<View>) -> Builder<AlignProps> {
    align(child)
}

impl Builder<AlignProps> {
    /// Where the child sits.
    pub fn alignment(self, alignment: Alignment) -> Self {
        self.map(move |p| p.alignment = alignment)
    }

    /// Top, at the reading start.
    pub fn top_start(self) -> Self {
        self.alignment(Alignment::TOP_START)
    }

    /// Top, horizontally centred.
    pub fn top_center(self) -> Self {
        self.alignment(Alignment::TOP_CENTER)
    }

    /// Top, at the reading end.
    pub fn top_end(self) -> Self {
        self.alignment(Alignment::TOP_END)
    }

    /// Vertically centred, at the reading start.
    pub fn center_start(self) -> Self {
        self.alignment(Alignment::CENTER_START)
    }

    /// Dead centre.
    pub fn center_center(self) -> Self {
        self.alignment(Alignment::CENTER)
    }

    /// Vertically centred, at the reading end.
    pub fn center_end(self) -> Self {
        self.alignment(Alignment::CENTER_END)
    }

    /// Bottom, at the reading start.
    pub fn bottom_start(self) -> Self {
        self.alignment(Alignment::BOTTOM_START)
    }

    /// Bottom, horizontally centred.
    pub fn bottom_center(self) -> Self {
        self.alignment(Alignment::BOTTOM_CENTER)
    }

    /// Bottom, at the reading end.
    pub fn bottom_end(self) -> Self {
        self.alignment(Alignment::BOTTOM_END)
    }

    /// Size this box as a multiple of its child's width rather than filling the
    /// space offered — `1.0` shrink-wraps the child.
    pub fn width_factor(self, factor: f32) -> Self {
        let factor = sane_factor(factor);
        self.map(move |p| p.width_factor = Some(factor))
    }

    /// Size this box as a multiple of its child's height rather than filling
    /// the space offered — `1.0` shrink-wraps the child.
    pub fn height_factor(self, factor: f32) -> Self {
        let factor = sane_factor(factor);
        self.map(move |p| p.height_factor = Some(factor))
    }

    /// Shrink-wrap the child on both axes: the box becomes exactly its child's
    /// size, and the alignment then only matters to whoever contains it.
    pub fn shrink_wrap(self) -> Self {
        self.width_factor(1.0).height_factor(1.0)
    }
}

// ---------------------------------------------------------------------------
// stack
// ---------------------------------------------------------------------------

/// Props for [`stack`].
#[derive(Debug, Clone, PartialEq)]
pub struct StackProps {
    alignment: Alignment,
    fit: StackFit,
    decoration: Decoration,
    clip: bool,
    label: Option<String>,
    role: AccessRole,
}

impl Default for StackProps {
    fn default() -> Self {
        Self {
            alignment: Alignment::CENTER,
            fit: StackFit::Loose,
            decoration: Decoration::NONE,
            clip: false,
            label: None,
            role: AccessRole::Container,
        }
    }
}

impl Decorated for StackProps {
    fn decoration_mut(&mut self) -> &mut Decoration {
        &mut self.decoration
    }
}

impl ViewNode for StackProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(StackBox {
            alignment: self.alignment,
            fit: self.fit,
            decoration: self.decoration,
            clip: self.clip,
            label: self.label.clone(),
            role: self.role,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<StackBox>()
            .expect("same view type means same render node type");
        let mut dirty = Dirty::NONE;
        if n.alignment != self.alignment || n.fit != self.fit || n.clip != self.clip {
            n.alignment = self.alignment;
            n.fit = self.fit;
            n.clip = self.clip;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.decoration != self.decoration {
            n.decoration = self.decoration;
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label || n.role != self.role {
            n.label.clone_from(&self.label);
            n.role = self.role;
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

/// Pile `children` along the **z-axis** — the first one at the back, the last
/// one on top.
///
/// ```
/// use silka_core::tree::Alignment;
/// use silka_core::view::{div, fixed, stack, View};
///
/// let badge_on_tile = stack([View::from(div()), View::from(fixed(20.0, 16.0))])
///     .alignment(Alignment::TOP_END);
/// # let _ = badge_on_tile;
/// ```
pub fn stack<C: Into<View>>(children: impl IntoIterator<Item = C>) -> Builder<StackProps> {
    Builder::new(StackProps::default()).children(children)
}

impl Builder<StackProps> {
    /// Where every child sits inside the stack's box.
    pub fn alignment(self, alignment: Alignment) -> Self {
        self.map(move |p| p.alignment = alignment)
    }

    /// Take every point on offer and hand the whole box to each child — the
    /// shape a full-bleed background or a scrim needs, and the mode that lets a
    /// child do its own alignment.
    pub fn expand(self) -> Self {
        self.fit(StackFit::Expand)
    }

    /// How much room the children are offered.
    pub fn fit(self, fit: StackFit) -> Self {
        self.map(move |p| p.fit = fit)
    }

    /// Clip the children to this box — corners included, so a photograph really
    /// does end at a rounded edge instead of poking out of it.
    pub fn clip(self) -> Self {
        self.map(move |p| p.clip = true)
    }

    /// The name a screen reader announces for the whole pile.
    ///
    /// Naming a stack promotes it from structure to a [`AccessRole::Group`]:
    /// use it when the children are one thing ("Profile photo, online"), and
    /// leave it off when they merely happen to overlap.
    pub fn label(self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.map(move |p| {
            if p.role == AccessRole::Container {
                p.role = AccessRole::Group;
            }
            p.label = Some(label);
        })
    }

    /// The a11y role, for the rare pile that is something more specific than a
    /// group.
    pub fn role(self, role: AccessRole) -> Self {
        self.map(move |p| p.role = role)
    }
}

// ---------------------------------------------------------------------------
// aspect_ratio
// ---------------------------------------------------------------------------

/// Props for [`aspect_ratio`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AspectRatioProps {
    ratio: f32,
}

impl ViewNode for AspectRatioProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(AspectRatioBox { ratio: self.ratio })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<AspectRatioBox>()
            .expect("same view type means same render node type");
        if n.ratio == self.ratio {
            return Dirty::NONE;
        }
        n.ratio = self.ratio;
        Dirty::LAYOUT | Dirty::PAINT
    }
}

/// Keep `child` at `ratio` (width ÷ height) whatever space it is given.
///
/// ```
/// use silka_core::view::{aspect_ratio, div, ASPECT_SQUARE};
///
/// let tile = aspect_ratio(ASPECT_SQUARE, div());
/// # let _ = tile;
/// ```
pub fn aspect_ratio(ratio: f32, child: impl Into<View>) -> Builder<AspectRatioProps> {
    Builder::new(AspectRatioProps { ratio }).child(child)
}

impl Builder<AspectRatioProps> {
    /// Change the ratio.
    pub fn ratio(self, ratio: f32) -> Self {
        self.map(move |p| p.ratio = ratio)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{BoxConstraints, RenderTree, TextDirection};
    use crate::view::{fixed, reconcile};
    use silka_paint::{Color, Command, Point, Scene, Size};
    use silka_theme::ColorToken;

    const BOX: Size = Size::new(200.0, 100.0);
    const CHILD: Size = Size::new(40.0, 20.0);

    fn child() -> Builder<super::super::FixedProps> {
        fixed(CHILD.width, CHILD.height)
    }

    fn placed(view: impl Into<View>, direction: TextDirection) -> (Size, Point) {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.set_direction(direction);
        tree.layout(BoxConstraints::tight(BOX));
        let outer = tree.children(tree.root())[0];
        let inner = tree.children(outer)[0];
        (tree.size(outer), tree.offset(inner))
    }

    // -- align -------------------------------------------------------------

    #[test]
    fn center_mengisi_kotaknya_dan_menaruh_anak_di_tengah() {
        let (size, offset) = placed(center(child()), TextDirection::Ltr);
        assert_eq!(size, BOX);
        assert_eq!(offset, Point::new(80.0, 40.0));
    }

    #[test]
    fn setiap_sudut_bernama_mendarat_sesuai_namanya() {
        for (alignment, expected) in [
            (Alignment::TOP_START, Point::new(0.0, 0.0)),
            (Alignment::TOP_END, Point::new(160.0, 0.0)),
            (Alignment::BOTTOM_START, Point::new(0.0, 80.0)),
            (Alignment::BOTTOM_END, Point::new(160.0, 80.0)),
            (Alignment::CENTER, Point::new(80.0, 40.0)),
        ] {
            let (_, offset) = placed(align(child()).alignment(alignment), TextDirection::Ltr);
            assert_eq!(offset, expected, "{alignment:?}");
        }
    }

    #[test]
    fn start_dan_end_bertukar_di_dokumen_rtl() {
        let (_, ltr) = placed(align(child()).top_start(), TextDirection::Ltr);
        let (_, rtl) = placed(align(child()).top_start(), TextDirection::Rtl);
        assert_eq!(ltr.x, 0.0);
        assert_eq!(rtl.x, BOX.width - CHILD.width);
        // The vertical axis has no reading direction and must not move.
        assert_eq!(ltr.y, rtl.y);
    }

    #[test]
    fn shrink_wrap_melepas_ruang_yang_ditawarkan() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, align(child()).shrink_wrap());
        tree.layout(BoxConstraints::loose(BOX));
        assert_eq!(tree.size(tree.children(tree.root())[0]), CHILD);
    }

    #[test]
    fn sumbu_tanpa_batas_menyusut_ke_anak_bukan_ke_tak_hingga() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, center(child()));
        tree.layout(BoxConstraints::width(BOX.width));
        let size = tree.size(tree.children(tree.root())[0]);
        assert_eq!(size.width, BOX.width);
        assert_eq!(size.height, CHILD.height, "ukuran tak hingga bukan ukuran");
    }

    #[test]
    fn align_tidak_muncul_di_pohon_aksesibilitas() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, center(child().label("Isi")));
        tree.layout(BoxConstraints::tight(BOX));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Isi")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Label);
    }

    #[test]
    fn alignment_ngawur_dijepit_bukan_dilempar_keluar_layar() {
        let (_, offset) = placed(
            align(child()).alignment(Alignment::new(9.0, f32::NAN)),
            TextDirection::Ltr,
        );
        assert_eq!(offset.x, BOX.width - CHILD.width);
        assert_eq!(offset.y, (BOX.height - CHILD.height) * 0.5);
    }

    #[test]
    fn align_bisa_menggambar_latar_dari_token() {
        use silka_theme::{Appearance, Theme};

        let t = Theme::cupertino(Appearance::Dark);
        super::super::with_theme(t, || {
            let mut tree = RenderTree::new();
            reconcile(&mut tree, center(child()).bg(ColorToken::SurfaceSunken));
            tree.layout(BoxConstraints::tight(BOX));
            let mut scene = Scene::new(Color::BLACK);
            tree.paint_into(&mut scene);
            assert!(
                scene.commands().iter().any(|c| match c {
                    Command::Quad(q) => q.background == t.color_of(ColorToken::SurfaceSunken),
                    _ => false,
                }),
                "empty state = align yang menggambar"
            );
        });
    }

    // -- stack -------------------------------------------------------------

    #[test]
    fn stack_sebesar_anak_terbesarnya() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, stack([fixed(120.0, 60.0), fixed(40.0, 90.0)]));
        tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
        assert_eq!(
            tree.size(tree.children(tree.root())[0]),
            Size::new(120.0, 90.0)
        );
    }

    #[test]
    fn anak_stack_berbagi_satu_kotak_bukan_mengantre() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, stack([fixed(120.0, 60.0), fixed(40.0, 20.0)]));
        tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
        let id = tree.children(tree.root())[0];
        let anak = tree.children(id).to_vec();
        // Both start inside the same 120x60 box; a row would have put the
        // second one after the first.
        assert_eq!(tree.offset(anak[0]), Point::ZERO);
        assert_eq!(tree.offset(anak[1]), Point::new(40.0, 20.0));
    }

    #[test]
    fn alignment_stack_menentukan_letak_anak_kecil() {
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            stack([fixed(120.0, 60.0), fixed(20.0, 20.0)]).alignment(Alignment::TOP_END),
        );
        tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
        let id = tree.children(tree.root())[0];
        let anak = tree.children(id).to_vec();
        assert_eq!(tree.offset(anak[1]), Point::new(100.0, 0.0));
    }

    #[test]
    fn alignment_stack_ikut_bercermin_di_rtl() {
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            stack([fixed(120.0, 60.0), fixed(20.0, 20.0)]).alignment(Alignment::TOP_END),
        );
        tree.set_direction(TextDirection::Rtl);
        tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
        let id = tree.children(tree.root())[0];
        let anak = tree.children(id).to_vec();
        assert_eq!(tree.offset(anak[1]), Point::ZERO);
    }

    #[test]
    fn expand_mengambil_seluruh_tawaran_dan_meneruskannya() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, stack([fixed(20.0, 20.0)]).expand());
        tree.layout(BoxConstraints::loose(BOX));
        let id = tree.children(tree.root())[0];
        assert_eq!(tree.size(id), BOX);
        assert_eq!(tree.size(tree.children(id)[0]), BOX, "tight, bukan loose");
    }

    #[test]
    fn anak_terakhir_digambar_paling_atas() {
        use silka_theme::{Appearance, Theme};

        let t = Theme::cupertino(Appearance::Dark);
        super::super::with_theme(t, || {
            let mut tree = RenderTree::new();
            reconcile(
                &mut tree,
                stack([
                    fixed(120.0, 60.0).bg(ColorToken::Surface),
                    fixed(20.0, 20.0).bg(ColorToken::Accent),
                ]),
            );
            tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
            let mut scene = Scene::new(Color::BLACK);
            tree.paint_into(&mut scene);
            let latar: Vec<Color> = scene
                .commands()
                .iter()
                .filter_map(|c| match c {
                    Command::Quad(q) => Some(q.background),
                    _ => None,
                })
                .collect();
            assert_eq!(
                latar,
                vec![
                    t.color_of(ColorToken::Surface),
                    t.color_of(ColorToken::Accent)
                ],
                "urutan gambar = urutan anak, belakang ke depan"
            );
        });
    }

    #[test]
    fn stack_tanpa_nama_diam_dan_yang_bernama_jadi_group() {
        let mut quiet = RenderTree::new();
        reconcile(&mut quiet, stack([fixed(40.0, 40.0)]));
        quiet.layout(BoxConstraints::loose(BOX));
        assert!(quiet.access_tree(None).find_label("Profil").is_none());

        let mut named = RenderTree::new();
        reconcile(&mut named, stack([fixed(40.0, 40.0)]).label("Profil"));
        named.layout(BoxConstraints::loose(BOX));
        let a11y = named.access_tree(None);
        let e = a11y
            .find_label("Profil")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Group);
    }

    // -- aspect_ratio ------------------------------------------------------

    fn frame_size(ratio: f32, constraints: BoxConstraints) -> Size {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, aspect_ratio(ratio, fixed(10.0, 10.0)));
        tree.layout(constraints);
        tree.size(tree.children(tree.root())[0])
    }

    #[test]
    fn lebar_terbatas_menentukan_tinggi() {
        let size = frame_size(
            ASPECT_16_9,
            BoxConstraints::new(0.0, 320.0, 0.0, f32::INFINITY),
        );
        assert_eq!(size.width, 320.0);
        assert!((size.height - 180.0).abs() < 0.01, "{size:?}");
    }

    #[test]
    fn kotak_pendek_jatuh_kembali_ke_tinggi() {
        let size = frame_size(ASPECT_16_9, BoxConstraints::loose(Size::new(320.0, 90.0)));
        assert!((size.height - 90.0).abs() < 0.01, "{size:?}");
        assert!((size.width - 160.0).abs() < 0.01, "{size:?}");
    }

    #[test]
    fn anak_menerima_persis_bingkainya() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, aspect_ratio(ASPECT_SQUARE, fixed(10.0, 10.0)));
        tree.layout(BoxConstraints::new(0.0, 120.0, 0.0, f32::INFINITY));
        let frame = tree.children(tree.root())[0];
        let anak = tree.children(frame)[0];
        assert_eq!(tree.size(anak), tree.size(frame));
        assert_eq!(tree.size(frame), Size::new(120.0, 120.0));
    }

    #[test]
    fn dua_sumbu_tanpa_batas_bertanya_ke_anak() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, aspect_ratio(2.0, fixed(80.0, 999.0)));
        tree.layout(BoxConstraints::UNBOUNDED);
        let size = tree.size(tree.children(tree.root())[0]);
        assert_eq!(size, Size::new(80.0, 40.0));
        assert!(size.width.is_finite() && size.height.is_finite());
    }

    #[test]
    fn rasio_ngawur_menjadi_bujur_sangkar_bukan_pembagian_nol() {
        for ratio in [0.0, -3.0, f32::NAN, f32::INFINITY] {
            let size = frame_size(ratio, BoxConstraints::new(0.0, 100.0, 0.0, f32::INFINITY));
            assert_eq!(size, Size::new(100.0, 100.0), "rasio {ratio}");
        }
    }

    #[test]
    fn tawaran_tight_tetap_dihormati_walau_merusak_rasio() {
        // The parent always wins: a child may never grow beyond what it was
        // given, ratio or no ratio.
        let size = frame_size(ASPECT_16_9, BoxConstraints::tight(Size::new(100.0, 100.0)));
        assert_eq!(size, Size::new(100.0, 100.0));
    }

    // -- diffing -----------------------------------------------------------

    #[test]
    fn membangun_ulang_yang_identik_tidak_melakukan_apa_pun() {
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            stack([fixed(40.0, 40.0)]).alignment(Alignment::TOP_START),
        );
        tree.layout(BoxConstraints::loose(BOX));
        assert!(reconcile(
            &mut tree,
            stack([fixed(40.0, 40.0)]).alignment(Alignment::TOP_START)
        )
        .is_noop());

        let mut frame = RenderTree::new();
        reconcile(&mut frame, aspect_ratio(ASPECT_4_3, fixed(10.0, 10.0)));
        frame.layout(BoxConstraints::loose(BOX));
        assert!(reconcile(&mut frame, aspect_ratio(ASPECT_4_3, fixed(10.0, 10.0))).is_noop());

        let mut middle = RenderTree::new();
        reconcile(&mut middle, center(child()));
        middle.layout(BoxConstraints::tight(BOX));
        assert!(reconcile(&mut middle, center(child())).is_noop());
        let _ = ASPECT_3_2;
    }
}
