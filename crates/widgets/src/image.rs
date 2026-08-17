//! `image()` — the Tier 0 bitmap (`KOMPONEN.md`).
//!
//! Impossible to write until [`silka_paint::ImageQuad`] existed, and the reason
//! that command was built: everything else in the drawing vocabulary is a shape
//! the shader can generate, and a photograph is not.
//!
//! ```
//! use silka_widgets::{image, install_images, Images};
//!
//! let atlas = Images::new();
//! install_images(&atlas);
//! // Whatever decoded the bytes is the application's business; the widget
//! // layer only ever sees an opaque handle.
//! let poster = atlas.insert_rgba(2, 1, &[255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
//!
//! let hero = image(poster)
//!     .cover()          // fill the box, crop the overflow
//!     .rounded_lg()     // …and end at a rounded edge
//!     .label("Sunset over the harbour");
//! # let _ = hero;
//! ```
//!
//! # Cropping costs nothing
//!
//! [`ImageFit::Cover`] does **not** resample anything on the CPU: the crop is
//! expressed as a source rectangle on the quad
//! ([`silka_paint::ImageQuad::source_uv`]), so a square photograph in a wide box
//! is the same one draw call it always was. The rounded clip is the same trick —
//! [`silka_paint::ImageQuad::corners`] is a mask in the shader, not a second
//! texture.
//!
//! # Where the pixels come from
//!
//! From [`Images`], the application's one bitmap atlas, exactly as glyphs come
//! from [`crate::Fonts`]. Decoding is deliberately **not** this module's job:
//! the widget layer never learns what a PNG is, which is what keeps a decoder
//! (and eventually an async one, `SISA-PEKERJAAN` §C) an application concern
//! rather than a framework dependency.
//!
//! # Definition of done
//!
//! | Line | How it is met |
//! |---|---|
//! | Both presets, dark mode | the only colour an image carries is its optional tint, which is a token |
//! | Interactive states, keyboard, hit target | none: a picture is content. Wrap it in [`silka_core::view::interactive`] to make one clickable |
//! | AccessKit node | [`silka_core::access::AccessRole::Image`] with the alt text from `label()`; without one it declares itself **decorative** and is filtered out, which is the correct answer for a photograph beside a heading that already says the same thing |
//! | Reduced motion | nothing moves |
//! | Touch shape = drawn shape | the corners that round the bitmap are the same [`silka_paint::Corners`] value everywhere (§3.6) |

use silka_core::access::{AccessNode, AccessRole};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{Alignment, BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, Corners, ImageId, ImageQuad, Rect, Size};
use silka_theme::{ColorToken, RadiusToken, Theme};

use crate::ambient::active_theme;
use crate::images::{active_images, Images};

/// How a bitmap is fitted into the box it was given.
///
/// ```
/// use silka_paint::Size;
/// use silka_widgets::ImageFit;
///
/// let box_size = Size::new(200.0, 100.0);
/// let bitmap = Size::new(100.0, 100.0);
///
/// // `Contain` letterboxes: the whole picture is visible, the box is not full.
/// let (rect, uv) = ImageFit::Contain.place(box_size, bitmap);
/// assert_eq!(rect.size, Size::new(100.0, 100.0));
/// assert_eq!(uv, [0.0, 0.0, 1.0, 1.0]);
///
/// // `Cover` fills the box and crops instead — through the source rect, so no
/// // pixel is resampled on the CPU to do it.
/// let (rect, uv) = ImageFit::Cover.place(box_size, bitmap);
/// assert_eq!(rect.size, box_size);
/// assert_eq!((uv[1], uv[3]), (0.25, 0.75));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageFit {
    /// Stretch to fill the box, ignoring the bitmap's own proportions. Rarely
    /// what anyone wants for a photograph and exactly right for a texture.
    Fill,
    /// Scale until the **whole** bitmap fits; the leftover box is empty
    /// (letterboxing). The default, because it never lies about the content.
    #[default]
    Contain,
    /// Scale until the box is **full**; whatever hangs over the edge is
    /// cropped. What a hero image and an avatar want.
    Cover,
    /// Draw at the bitmap's own size, cropping anything that does not fit.
    None,
}

impl ImageFit {
    /// Where the bitmap is drawn inside a box, and which part of it is sampled.
    ///
    /// Returns the destination rect in **local** coordinates together with the
    /// normalized source rect `[u0, v0, u1, v1]`. Centred; see
    /// [`ImageFit::place_aligned`] to put it somewhere else.
    pub fn place(self, box_size: Size, bitmap: Size) -> (Rect, [f32; 4]) {
        self.place_aligned(box_size, bitmap, Alignment::CENTER, false)
    }

    /// [`ImageFit::place`] with an explicit alignment and reading direction.
    ///
    /// `rtl` only matters for a fit that leaves free space — a letterboxed
    /// picture pinned to the reading start moves with the document (§9.8).
    pub fn place_aligned(
        self,
        box_size: Size,
        bitmap: Size,
        alignment: Alignment,
        rtl: bool,
    ) -> (Rect, [f32; 4]) {
        const FULL: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
        let direction = if rtl {
            silka_core::tree::TextDirection::Rtl
        } else {
            silka_core::tree::TextDirection::Ltr
        };
        let whole = Rect::new(0.0, 0.0, box_size.width, box_size.height);
        if box_size.is_empty() || bitmap.is_empty() {
            return (whole, FULL);
        }

        // A drawn size, then the offset that alignment gives it: two steps, so
        // no fit has to know about the reading direction.
        let drawn = match self {
            ImageFit::Fill => box_size,
            ImageFit::Contain => {
                let s = (box_size.width / bitmap.width).min(box_size.height / bitmap.height);
                Size::new(bitmap.width * s, bitmap.height * s)
            }
            ImageFit::Cover => box_size,
            ImageFit::None => Size::new(
                bitmap.width.min(box_size.width),
                bitmap.height.min(box_size.height),
            ),
        };
        let offset = alignment.offset(box_size, drawn, direction);
        let rect = Rect::new(offset.x, offset.y, drawn.width, drawn.height);

        // Cropping happens on the source side, which is why it is free.
        let uv = match self {
            ImageFit::Fill | ImageFit::Contain => FULL,
            ImageFit::Cover => {
                let box_aspect = box_size.width / box_size.height;
                let bitmap_aspect = bitmap.width / bitmap.height;
                if bitmap_aspect > box_aspect {
                    crop(
                        box_aspect / bitmap_aspect,
                        alignment.resolve(direction).0,
                        true,
                    )
                } else {
                    crop(
                        bitmap_aspect / box_aspect,
                        alignment.resolve(direction).1,
                        false,
                    )
                }
            }
            ImageFit::None => {
                let fx = (box_size.width / bitmap.width).min(1.0);
                let fy = (box_size.height / bitmap.height).min(1.0);
                let (ax, ay) = alignment.resolve(direction);
                let [u0, _, u1, _] = crop(fx, ax, true);
                let [_, v0, _, v1] = crop(fy, ay, false);
                [u0, v0, u1, v1]
            }
        };
        (rect, uv)
    }
}

/// The visible fraction of one axis, positioned by an alignment factor.
fn crop(fraction: f32, factor: f32, horizontal: bool) -> [f32; 4] {
    let f = fraction.clamp(0.0, 1.0);
    let start = ((1.0 - f) * factor).clamp(0.0, 1.0 - f);
    if horizontal {
        [start, 0.0, start + f, 1.0]
    } else {
        [0.0, start, 1.0, start + f]
    }
}

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// The bitmap leaf.
///
/// ```
/// use silka_core::tree::{BoxConstraints, RenderTree};
/// use silka_core::view::reconcile;
/// use silka_paint::Size;
/// use silka_widgets::{image_in, Images};
///
/// let atlas = Images::new();
/// let id = atlas.insert_mask(40, 20, &[255; 800]).unwrap();
///
/// let mut tree = RenderTree::new();
/// reconcile(&mut tree, image_in(&atlas, id));
/// // Without an explicit size it takes the bitmap's own, fitted into the offer.
/// tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
/// assert_eq!(tree.size(tree.children(tree.root())[0]), Size::new(40.0, 20.0));
/// ```
pub struct ImageBox {
    image: ImageId,
    fit: ImageFit,
    alignment: Alignment,
    corners: Corners,
    tint: Color,
    width: Option<f32>,
    height: Option<f32>,
    expand: bool,
    label: Option<String>,
    images: Images,
}

impl ImageBox {
    /// The bitmap's own size in logical points, when the handle is still valid.
    fn natural(&self) -> Option<Size> {
        self.images
            .natural_points(self.image)
            .filter(|s| !s.is_empty())
    }
}

impl std::fmt::Debug for ImageBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageBox")
            .field("image", &self.image)
            .field("fit", &self.fit)
            .field("label", &self.label)
            .finish()
    }
}

impl RenderNode for ImageBox {
    fn type_name(&self) -> &'static str {
        "Image"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if self.expand {
            return constraints.constrain(Size::new(
                if constraints.has_bounded_width() {
                    constraints.max_width
                } else {
                    constraints.min_width
                },
                if constraints.has_bounded_height() {
                    constraints.max_height
                } else {
                    constraints.min_height
                },
            ));
        }

        let natural = self.natural();
        let aspect = natural.map(|n| n.width / n.height);
        let size = match (self.width, self.height) {
            (Some(w), Some(h)) => Size::new(w, h),
            (Some(w), None) => Size::new(w, aspect.map_or(w, |a| w / a)),
            (None, Some(h)) => Size::new(aspect.map_or(h, |a| h * a), h),
            (None, None) => match natural {
                // Scale down to fit the offer, keeping the proportions: a
                // constrain() alone would squash the picture instead.
                Some(n) => {
                    let mut size = n;
                    if size.width > constraints.max_width {
                        let s = constraints.max_width / size.width;
                        size = Size::new(constraints.max_width, size.height * s);
                    }
                    if size.height > constraints.max_height {
                        let s = constraints.max_height / size.height;
                        size = Size::new(size.width * s, constraints.max_height);
                    }
                    size
                }
                // A stale or missing handle draws nothing and takes no room —
                // skipping a picture beats reserving a hole for it.
                None => Size::ZERO,
            },
        };
        constraints.constrain(size)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let Some(natural) = self.natural() else {
            return;
        };
        let (rect, uv) = self
            .fit
            .place_aligned(ctx.size(), natural, self.alignment, ctx.is_rtl());
        ctx.image(
            ImageQuad::new(rect, self.image)
                .tint(self.tint)
                .corners(self.corners)
                .source_uv(uv[0], uv[1], uv[2], uv[3]),
        );
    }

    fn access(&self, node: &mut AccessNode) {
        match &self.label {
            // Alt text: the picture is content, and a screen reader announces
            // it as one.
            Some(label) => {
                node.role = AccessRole::Image;
                node.label = Some(label.clone());
            }
            // Decorative: filtered out entirely, which is the right answer for
            // a picture whose meaning is already in the text beside it.
            None => node.role = AccessRole::Container,
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props for the bitmap leaf.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageProps {
    image: ImageId,
    fit: ImageFit,
    alignment: Alignment,
    corners: Corners,
    tint: Color,
    width: Option<f32>,
    height: Option<f32>,
    expand: bool,
    label: Option<String>,
    images: Images,
}

impl ViewNode for ImageProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ImageBox {
            image: self.image,
            fit: self.fit,
            alignment: self.alignment,
            corners: self.corners,
            tint: self.tint,
            width: self.width,
            height: self.height,
            expand: self.expand,
            label: self.label.clone(),
            images: self.images.clone(),
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ImageBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.image != self.image
            || n.width != self.width
            || n.height != self.height
            || n.expand != self.expand
            || n.images != self.images
        {
            n.image = self.image;
            n.width = self.width;
            n.height = self.height;
            n.expand = self.expand;
            n.images = self.images.clone();
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.fit != self.fit
            || n.alignment != self.alignment
            || n.corners != self.corners
            || n.tint != self.tint
        {
            n.fit = self.fit;
            n.alignment = self.alignment;
            n.corners = self.corners;
            n.tint = self.tint;
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

/// A Dart-style image builder (§2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    props: ImageProps,
    theme: Theme,
    key: Option<Key>,
}

/// A bitmap from the application's atlas — the `image` component
/// (`KOMPONEN.md` Tier 0).
///
/// The atlas comes from [`active_images`] and the tokens from the ambient
/// theme, so neither appears at the call site.
///
/// ```
/// use silka_widgets::{image, Images};
/// # let atlas = Images::new();
/// # silka_widgets::install_images(&atlas);
/// # let photo = atlas.insert_mask(4, 4, &[255; 16]).unwrap();
/// let avatar = image(photo).cover().rounded_full().label("Dian Permata");
/// # let _ = avatar;
/// ```
pub fn image(id: ImageId) -> Image {
    image_in(&active_images(), id)
}

/// [`image()`] with the atlas passed explicitly — for views built outside a
/// build pass.
pub fn image_in(images: &Images, id: ImageId) -> Image {
    Image {
        props: ImageProps {
            image: id,
            fit: ImageFit::default(),
            alignment: Alignment::CENTER,
            corners: Corners::SHARP,
            tint: Color::WHITE,
            width: None,
            height: None,
            expand: false,
            label: None,
            images: images.clone(),
        },
        theme: active_theme(),
        key: None,
    }
}

impl Image {
    fn map(mut self, f: impl FnOnce(&mut ImageProps)) -> Self {
        f(&mut self.props);
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Resolve tokens against `theme` instead of the ambient one.
    pub fn theme(mut self, theme: &Theme) -> Self {
        self.theme = *theme;
        self
    }

    /// How the bitmap is fitted into its box.
    pub fn fit(self, fit: ImageFit) -> Self {
        self.map(move |p| p.fit = fit)
    }

    /// Fill the box and crop the overflow — [`ImageFit::Cover`].
    pub fn cover(self) -> Self {
        self.fit(ImageFit::Cover)
    }

    /// Show the whole bitmap, letterboxing the rest — [`ImageFit::Contain`].
    pub fn contain(self) -> Self {
        self.fit(ImageFit::Contain)
    }

    /// Stretch to the box, proportions be damned — [`ImageFit::Fill`].
    pub fn fill(self) -> Self {
        self.fit(ImageFit::Fill)
    }

    /// Where the bitmap sits when the fit leaves free space.
    pub fn alignment(self, alignment: Alignment) -> Self {
        self.map(move |p| p.alignment = alignment)
    }

    /// Take the whole box on offer rather than the bitmap's own size — the
    /// shape a full-bleed hero inside a `column().items_stretch()` needs.
    pub fn expand(self) -> Self {
        self.map(move |p| p.expand = true)
    }

    /// A fixed width in logical points; the height follows the bitmap's
    /// proportions unless it is given too.
    pub fn width(self, width: f32) -> Self {
        let width = sane(width);
        self.map(move |p| p.width = Some(width))
    }

    /// A fixed height in logical points; the width follows the bitmap's
    /// proportions unless it is given too.
    pub fn height(self, height: f32) -> Self {
        let height = sane(height);
        self.map(move |p| p.height = Some(height))
    }

    /// A fixed size on both axes.
    pub fn size(self, width: f32, height: f32) -> Self {
        self.width(width).height(height)
    }

    /// Round the bitmap's corners with one radius token — squircle under
    /// Cupertino, arc under Tailwind, exactly like a box (§2.7).
    pub fn rounded(self, token: RadiusToken) -> Self {
        let corners = self.theme.corners_of(token);
        self.map(move |p| p.corners = corners)
    }

    /// Square corners.
    pub fn rounded_none(self) -> Self {
        self.rounded(RadiusToken::None)
    }

    /// The `sm` radius token.
    pub fn rounded_sm(self) -> Self {
        self.rounded(RadiusToken::Sm)
    }

    /// The `md` radius token.
    pub fn rounded_md(self) -> Self {
        self.rounded(RadiusToken::Md)
    }

    /// The `lg` radius token.
    pub fn rounded_lg(self) -> Self {
        self.rounded(RadiusToken::Lg)
    }

    /// The `xl` radius token.
    pub fn rounded_xl(self) -> Self {
        self.rounded(RadiusToken::Xl)
    }

    /// Pill or circle — the shape an avatar wants, and the radius the shader
    /// clamps to half the box.
    pub fn rounded_full(self) -> Self {
        self.rounded(RadiusToken::Full)
    }

    /// **Escape hatch**: corner geometry computed rather than named.
    pub fn rounded_raw(self, corners: Corners) -> Self {
        self.map(move |p| p.corners = corners)
    }

    /// Multiply the bitmap by a token colour.
    ///
    /// For a photograph this is a scrim; for a coverage-only bitmap it is the
    /// colour itself, which is exactly how [`crate::icon()`] draws.
    pub fn tint(self, token: ColorToken) -> Self {
        let color = self.theme.color_of(token);
        self.map(move |p| p.tint = color)
    }

    /// **Escape hatch**: a tint that is not a token.
    pub fn tint_raw(self, color: Color) -> Self {
        self.map(move |p| p.tint = color)
    }

    /// Fade the whole bitmap.
    pub fn opacity(self, opacity: f32) -> Self {
        let opacity = opacity.clamp(0.0, 1.0);
        self.map(move |p| p.tint = p.tint.with_alpha(p.tint.a * opacity))
    }

    /// The alt text a screen reader announces.
    ///
    /// Without it the image declares itself **decorative** and disappears from
    /// the a11y tree — the right answer when the caption beside it already says
    /// the same thing, and the wrong one for a picture that carries meaning of
    /// its own.
    pub fn label(self, label: impl Into<String>) -> Self {
        let label = label.into();
        self.map(move |p| p.label = Some(label))
    }

    /// Mark the picture as decoration, dropping any name it had.
    pub fn decorative(self) -> Self {
        self.map(move |p| p.label = None)
    }
}

fn sane(v: f32) -> f32 {
    if v.is_finite() {
        v.max(0.0)
    } else {
        0.0
    }
}

impl From<Image> for View {
    fn from(i: Image) -> View {
        let mut b = Builder::new(i.props);
        if let Some(key) = i.key {
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
    use silka_theme::Appearance;

    /// A 40x20 bitmap: a shape wide enough that a fit mistake is visible.
    fn atlas() -> (Images, ImageId) {
        let images = Images::new();
        let id = images.insert_mask(40, 20, &[255; 800]).expect("fits");
        (images, id)
    }

    fn drawn(view: impl Into<View>, box_size: Size) -> Option<ImageQuad> {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::tight(box_size));
        let mut scene = Scene::new(Color::BLACK);
        tree.paint_into(&mut scene);
        scene.commands().iter().find_map(|c| match c {
            Command::Image(q) => Some(*q),
            _ => None,
        })
    }

    #[test]
    fn without_a_size_it_takes_the_bitmaps_own() {
        let (images, id) = atlas();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, image_in(&images, id));
        tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
        assert_eq!(
            tree.size(tree.children(tree.root())[0]),
            Size::new(40.0, 20.0)
        );
    }

    #[test]
    fn a_bitmap_bigger_than_the_offer_scales_down_instead_of_squashing() {
        let (images, id) = atlas();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, image_in(&images, id));
        tree.layout(BoxConstraints::loose(Size::new(20.0, 400.0)));
        let size = tree.size(tree.children(tree.root())[0]);
        assert_eq!(size, Size::new(20.0, 10.0), "the proportions have to hold");
    }

    #[test]
    fn one_axis_given_derives_the_other_from_the_bitmap() {
        let (images, id) = atlas();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, image_in(&images, id).width(80.0));
        tree.layout(BoxConstraints::loose(Size::new(400.0, 400.0)));
        assert_eq!(
            tree.size(tree.children(tree.root())[0]),
            Size::new(80.0, 40.0)
        );
    }

    #[test]
    fn cover_fills_the_box_and_crops_through_the_source_rect() {
        let (images, id) = atlas();
        let q = drawn(image_in(&images, id).cover(), Size::new(100.0, 100.0))
            .expect("an image command");
        assert_eq!(q.rect.size, Size::new(100.0, 100.0));
        // The 2:1 bitmap in a 1:1 box: half its width is sampled, centred.
        assert!((q.source_uv[0] - 0.25).abs() < 1e-5, "{:?}", q.source_uv);
        assert!((q.source_uv[2] - 0.75).abs() < 1e-5, "{:?}", q.source_uv);
        assert_eq!((q.source_uv[1], q.source_uv[3]), (0.0, 1.0));
    }

    #[test]
    fn contain_letterboxes_and_samples_the_whole_bitmap() {
        let (images, id) = atlas();
        let q = drawn(image_in(&images, id).contain(), Size::new(100.0, 100.0))
            .expect("an image command");
        assert_eq!(q.rect.size, Size::new(100.0, 50.0));
        assert_eq!(q.rect.min_y(), 25.0, "centred in the leftover space");
        assert_eq!(q.source_uv, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn fill_stretches_and_never_crops() {
        let (images, id) = atlas();
        let q =
            drawn(image_in(&images, id).fill(), Size::new(100.0, 100.0)).expect("an image command");
        assert_eq!(q.rect.size, Size::new(100.0, 100.0));
        assert_eq!(q.source_uv, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn the_rounded_clip_is_a_token_and_reaches_the_command() {
        let (images, id) = atlas();
        for theme in [
            Theme::cupertino(Appearance::Dark),
            Theme::tailwind(Appearance::Light),
        ] {
            let q = drawn(
                image_in(&images, id).theme(&theme).cover().rounded_lg(),
                Size::new(100.0, 100.0),
            )
            .expect("an image command");
            assert_eq!(q.corners.style, theme.radius.style);
            assert!(q.corners.radii.max() > 0.0);
        }
    }

    #[test]
    fn a_named_image_is_content_and_an_unnamed_one_is_decoration() {
        let (images, id) = atlas();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, image_in(&images, id).label("Harbour at dusk"));
        tree.layout(BoxConstraints::loose(Size::new(200.0, 200.0)));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Harbour at dusk")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Image);

        let mut quiet = RenderTree::new();
        reconcile(&mut quiet, image_in(&images, id));
        quiet.layout(BoxConstraints::loose(Size::new(200.0, 200.0)));
        let a11y = quiet.access_tree(None);
        assert!(
            a11y.entries()
                .iter()
                .all(|e| e.node.role != AccessRole::Image),
            "a decorative picture must not be announced:\n{}",
            a11y.dump()
        );
    }

    #[test]
    fn a_stale_handle_draws_nothing_rather_than_somebody_elses_pixels() {
        let (images, id) = atlas();
        images.with(|a| a.clear());
        assert!(drawn(image_in(&images, id), Size::new(100.0, 100.0)).is_none());
    }

    #[test]
    fn the_fit_arithmetic_survives_a_degenerate_box() {
        for (box_size, bitmap) in [
            (Size::ZERO, Size::new(10.0, 10.0)),
            (Size::new(10.0, 10.0), Size::ZERO),
        ] {
            for fit in [
                ImageFit::Fill,
                ImageFit::Contain,
                ImageFit::Cover,
                ImageFit::None,
            ] {
                let (rect, uv) = fit.place(box_size, bitmap);
                assert!(rect.size.width.is_finite() && rect.size.height.is_finite());
                assert!(uv.iter().all(|v| v.is_finite()), "{fit:?} {uv:?}");
            }
        }
    }

    #[test]
    fn rebuilding_an_identical_image_does_nothing() {
        let (images, id) = atlas();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, image_in(&images, id).cover());
        tree.layout(BoxConstraints::loose(Size::new(200.0, 200.0)));
        assert!(reconcile(&mut tree, image_in(&images, id).cover()).is_noop());
    }
}
