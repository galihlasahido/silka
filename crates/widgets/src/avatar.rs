//! `avatar()` / `avatar_group()` — a person, as a disc
//! (`KOMPONEN.md` Tier 5).
//!
//! ```
//! use silka_widgets::{avatar, avatar_group};
//!
//! // With no picture it falls back to initials — which is the case that
//! // actually happens, because most accounts have no photograph.
//! let me = avatar("Dian Permata").lg();
//!
//! let team = avatar_group([
//!     avatar("Dian Permata"),
//!     avatar("Bagas Nugroho"),
//!     avatar("Sari Wulandari"),
//!     avatar("Rizky Pratama"),
//! ])
//! .max(3)
//! .label("Project team");
//! # let _ = (me, team);
//! ```
//!
//! # Three things a hand-rolled avatar gets wrong
//!
//! Both applications in this repository had grown one, and between them they
//! made all three mistakes:
//!
//! 1. **The fallback is the main case.** An avatar is written as "a round
//!    photo", and then most accounts turn out to have no photo. Here the
//!    initials are not a degraded state: [`initials`] is a pure function with a
//!    right answer for one word, three words, an empty string and a name in a
//!    script with no capital letters at all.
//! 2. **A photo is announced as an image with no name.** A screen reader
//!    meeting a decorative disc learns nothing; meeting "Dian Permata" learns
//!    who is on the call. The name is a **required argument**, exactly as it is
//!    for [`icon_button`](crate::icon_button), because it is the one thing a
//!    picture cannot borrow from what it draws.
//! 3. **A stack of overlapping discs has no edges.** Four avatars overlapping
//!    by a third read as one blob unless each one carries a ring in the colour
//!    of the surface behind it. [`AvatarBox::ring_width`] is that ring, and
//!    [`avatar_group`] is what fills it in.
//!
//! # A colour per person, without inventing colours
//!
//! Every avatar component eventually wants "the same person is always the same
//! colour". The rule is [`avatar_slot`] — a pure, stable hash of the name onto
//! *n* slots — and the palette is deliberately **not** here: a design system
//! has one accent, not eight, and a widget that invented seven more would be
//! the one place in the framework where colours are born outside a token
//! (§2.6). An application hands the slot to a palette it already owns:
//!
//! ```
//! # use silka_widgets::{avatar, avatar_slot};
//! # let palette: [silka_paint::Color; 8] = [silka_paint::Color::WHITE; 8];
//! let name = "Dian Permata";
//! let a = avatar(name).tint_raw(palette[avatar_slot(name, palette.len())]);
//! # let _ = a;
//! ```
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Line | How it is met |
//! |---|---|
//! | Correct in both presets | the fill is a [`ColorToken`], the ring is `Surface`, the corner comes from [`Theme::corners`] so a squircle clamped to half its box is still a circle |
//! | Interactive states on a spring | none: an avatar is not a control. A clickable one is an avatar **inside** a [`button`](crate::button) or a pressable [`card`](crate::card) |
//! | Keyboard + focus ring | not a tab stop, by design |
//! | AccessKit node | [`AccessRole::Image`] carrying the person's name, or hidden when [`Avatar::decorative`] |
//! | Dark mode | token-driven |
//! | Hit target ≥ 44pt | not applicable: nothing here is clickable |
//! | Reduced motion | nothing moves |

use silka_core::access::{AccessNode, AccessRole};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::{Builder, View, ViewNode};
use silka_paint::{Color, Corners, ImageId, Insets, Point, Quad, Rect, Size};
use silka_text::FontWeight;
use silka_theme::{ColorToken, RadiusToken, SpaceToken, Theme};

use crate::fonts::Fonts;
use crate::image::{image_in, ImageFit};
use crate::images::{active_images, Images};
use crate::text::text_in;

/// The default diameter, in **spacing steps** (§2.6) — 8 × 4pt = 32pt.
pub const AVATAR_STEPS: f32 = 8.0;

/// How much of its own width each avatar in a group gives up to its neighbour.
///
/// A third: less and the row does not read as a stack, more and the initials
/// underneath disappear entirely.
pub const GROUP_OVERLAP: f32 = 1.0 / 3.0;

/// The initials' height as a fraction of the disc's diameter.
///
/// A **proportion** rather than a token, and deliberately so: the type scale
/// answers "how big is a footnote", not "how big are two letters inside a
/// 32pt circle". Below roughly this value the letters stop being legible at
/// small sizes; above it they start touching the ring.
pub const INITIALS_RATIO: f32 = 0.4;

// ---------------------------------------------------------------------------
// Pure rules
// ---------------------------------------------------------------------------

/// The initials of `name`, at most `max` letters.
///
/// A pure function, because "what does this name look like as two letters?"
/// has a right answer that must not depend on a running app — and because the
/// interesting cases are the ones nobody tests by clicking:
///
/// ```
/// use silka_widgets::initials;
///
/// assert_eq!(initials("Dian Permata", 2), "DP");
/// assert_eq!(initials("dian permata sari", 2), "DP");
/// // One word gives one letter. Two letters of a single word ("BA" for
/// // "Bagas") reads as somebody else's initials.
/// assert_eq!(initials("Bagas", 2), "B");
/// // Nothing in, nothing out — never a placeholder glyph the reader has to
/// // decode.
/// assert_eq!(initials("", 2), "");
/// assert_eq!(initials("   ", 2), "");
/// // Scripts with no capital letters keep their character rather than losing
/// // it to an uppercase mapping that does not exist.
/// assert_eq!(initials("山田 太郎", 2), "山太");
/// ```
pub fn initials(name: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    name.split_whitespace()
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
        .take(max)
        .flat_map(|c| c.to_uppercase())
        .collect()
}

/// A **stable** slot for `name`, in `0..slots`.
///
/// FNV-1a over the trimmed, lowercased name. Written out rather than borrowed
/// from `std`, because [`std::collections::hash_map::RandomState`] is seeded
/// per process: the same person would get a different colour every time the
/// application started, which is precisely the property an identity colour
/// exists to have.
///
/// ```
/// use silka_widgets::avatar_slot;
///
/// // Stable across runs, and insensitive to the things a name is written
/// // with rather than the name itself.
/// assert_eq!(avatar_slot("Dian Permata", 8), avatar_slot("  dian permata  ", 8));
/// assert!(avatar_slot("Dian Permata", 8) < 8);
/// // A palette of one is not an error, and neither is a palette of none.
/// assert_eq!(avatar_slot("anyone", 1), 0);
/// assert_eq!(avatar_slot("anyone", 0), 0);
/// ```
pub fn avatar_slot(name: &str, slots: usize) -> usize {
    if slots <= 1 {
        return 0;
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for c in name.trim().chars().flat_map(|c| c.to_lowercase()) {
        let mut buf = [0u8; 4];
        for b in c.encode_utf8(&mut buf).as_bytes() {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    (hash % slots as u64) as usize
}

/// How many avatars a group shows, and how many it has to admit to hiding.
///
/// A pure function, because the two off-by-ones live here: a group of exactly
/// `max` shows all of them (a `+0` counter is a lie about a group that fits),
/// and a group that overflows gives up one visible slot **to the counter**, so
/// `max` really is the number of discs on the row.
///
/// ```
/// use silka_widgets::avatar::group_plan;
///
/// // It fits: no counter at all.
/// assert_eq!(group_plan(3, 3), (3, 0));
/// // It does not: the counter takes the last slot, so the row is still 3 wide.
/// assert_eq!(group_plan(9, 3), (2, 7));
/// // `max` of zero means "never collapse".
/// assert_eq!(group_plan(9, 0), (9, 0));
/// ```
pub fn group_plan(total: usize, max: usize) -> (usize, usize) {
    if max == 0 || total <= max {
        return (total, 0);
    }
    let shown = max.saturating_sub(1);
    (shown, total - shown)
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every drawing and layout value of an avatar, already resolved from tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvatarStyle {
    /// The disc's diameter.
    pub diameter: f32,
    /// The corner geometry — half the diameter for a circle.
    pub corners: Corners,
    /// The fill behind the initials (transparent under a photograph).
    pub background: Color,
    /// The initials' colour.
    pub foreground: Color,
    /// The ring that separates overlapping discs; 0 = none.
    pub ring_width: f32,
    /// The ring's colour — the surface behind the group.
    pub ring_color: Color,
}

impl AvatarStyle {
    /// The default style at `diameter` in `theme`.
    pub fn from_theme(theme: &Theme, diameter: f32) -> Self {
        Self {
            diameter,
            // Half the diameter is a circle **whatever the preset's corner
            // shape is**: a squircle clamped to half its box is a circle too,
            // so this is one call with two geometries and no special case.
            corners: theme.corners(diameter * 0.5),
            background: theme.color_of(ColorToken::AccentMuted),
            foreground: theme.color_of(ColorToken::Accent),
            ring_width: 0.0,
            ring_color: theme.color_of(ColorToken::Surface),
        }
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// The disc: a fixed square, one centred child, and an optional ring.
///
/// It owns a node rather than being a decorated `center(…)` for the ring. A
/// ring drawn as a border would sit **inside** the disc and eat into the
/// picture; this one is drawn on top of the child's edge, which is what lets
/// two avatars overlap and still read as two.
pub struct AvatarBox {
    /// Every resolved drawing value.
    pub style: AvatarStyle,
    /// The name a screen reader announces.
    pub label: Option<String>,
    /// True when the avatar carries no information of its own.
    pub decorative: bool,
}

impl AvatarBox {
    /// The disc's rect in local coordinates.
    pub fn disc_rect(&self) -> Rect {
        Rect::new(0.0, 0.0, self.style.diameter, self.style.diameter)
    }

    /// The ring thickness actually drawn (clamped so it cannot swallow the
    /// disc it is meant to outline).
    pub fn ring_width(&self) -> f32 {
        self.style.ring_width.clamp(0.0, self.style.diameter * 0.25)
    }
}

impl RenderNode for AvatarBox {
    fn type_name(&self) -> &'static str {
        "Avatar"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let d = self.style.diameter.max(0.0);
        let size = constraints.constrain(Size::new(d, d));
        if ctx.child_count() == 0 {
            return size;
        }
        let child = ctx.child(0);
        // The content is inset by the ring, so a photograph is never clipped by
        // the very line meant to separate it from its neighbour.
        let ring = self.ring_width();
        let dalam = (d - ring * 2.0).max(0.0);
        let isi = ctx.layout_child(child, BoxConstraints::loose(Size::new(dalam, dalam)));
        ctx.place_child(
            child,
            Point::new(
                (size.width - isi.width) * 0.5,
                (size.height - isi.height) * 0.5,
            ),
        );
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let disc = self.disc_rect();
        let corners = self.style.corners.clamp_to(disc.size);
        if self.style.background.a > 0.0 {
            ctx.quad(
                Quad::new(disc)
                    .corners(corners)
                    .background(self.style.background),
            );
        }
        ctx.paint_children();

        // Drawn **after** the child and half over its edge, which is the whole
        // reason this is not a `border` on the fill: a border underneath a
        // photograph is a border nobody can see.
        let ring = self.ring_width();
        if ring > 0.0 && self.style.ring_color.a > 0.0 {
            ctx.quad(
                Quad::new(disc.deflate(Insets::all(ring * 0.5)))
                    .corners(corners.clamp_to(Size::new(
                        (disc.size.width - ring).max(0.0),
                        (disc.size.height - ring).max(0.0),
                    )))
                    .border(ring, self.style.ring_color),
            );
        }
    }

    /// A named image, or nothing at all.
    ///
    /// The middle ground — an image with no name — is the one thing this must
    /// not produce: a screen reader announcing "image" and stopping tells the
    /// reader something is there and refuses to say what.
    fn access(&self, node: &mut AccessNode) {
        if self.decorative {
            node.role = AccessRole::Container;
            node.hidden = true;
        } else if let Some(label) = &self.label {
            node.role = AccessRole::Image;
            node.label = Some(label.clone());
        } else {
            node.role = AccessRole::Container;
        }
    }

    /// The touch shape follows the drawn shape, so the corners of the square an
    /// avatar occupies are not clickable dead ground (§3.6).
    fn hit_shape(&self) -> silka_core::input::HitShape {
        silka_core::input::HitShape::Rounded(self.style.corners)
    }
}

impl core::fmt::Debug for AvatarBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AvatarBox")
            .field("label", &self.label)
            .field("diameter", &self.style.diameter)
            .field("ring", &self.style.ring_width)
            .finish()
    }
}

/// The props of [`AvatarBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct AvatarProps {
    style: AvatarStyle,
    label: Option<String>,
    decorative: bool,
}

impl ViewNode for AvatarProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(AvatarBox {
            style: self.style,
            label: self.label.clone(),
            decorative: self.decorative,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<AvatarBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.style.diameter != self.style.diameter || n.style.ring_width != self.style.ring_width {
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        } else if n.style != self.style {
            dirty |= Dirty::PAINT;
        }
        n.style = self.style;
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.decorative != self.decorative {
            n.decorative = self.decorative;
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

// ---------------------------------------------------------------------------
// Avatar builder
// ---------------------------------------------------------------------------

/// A person as a disc: their picture, or their initials when there is none.
///
/// Use [`avatar_in`] outside a build pass.
///
/// ```
/// use silka_widgets::avatar;
///
/// let a = avatar("Dian Permata").lg();
/// assert_eq!(a.initials(), "DP");
/// ```
pub fn avatar(name: impl Into<String>) -> Avatar {
    avatar_in(
        &crate::active_fonts(),
        &active_images(),
        &crate::ambient::active_theme(),
        name,
    )
}

/// [`avatar`] with the text engine, the atlas and the theme passed explicitly.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{avatar_in, Fonts, Images};
///
/// let fonts = Fonts::bundled_only();
/// let images = Images::new();
/// let theme = Theme::cupertino(Appearance::Dark);
///
/// let a = avatar_in(&fonts, &images, &theme, "Bagas Nugroho").sm();
/// assert_eq!(a.initials(), "BN");
/// assert_eq!(a.style().diameter, theme.space(6.0));
/// ```
pub fn avatar_in(fonts: &Fonts, images: &Images, theme: &Theme, name: impl Into<String>) -> Avatar {
    Avatar {
        fonts: fonts.clone(),
        images: images.clone(),
        theme: *theme,
        key: None,
        name: name.into(),
        image: None,
        diameter: None,
        radius: None,
        background: None,
        foreground: None,
        ring: None,
        label: None,
        max_initials: 2,
        decorative: false,
    }
}

/// The avatar builder — Dart-style (§2.5).
#[derive(Debug, Clone, PartialEq)]
pub struct Avatar {
    fonts: Fonts,
    images: Images,
    theme: Theme,
    key: Option<Key>,
    name: String,
    image: Option<ImageId>,
    diameter: Option<f32>,
    radius: Option<RadiusToken>,
    background: Option<Color>,
    foreground: Option<Color>,
    ring: Option<(f32, Color)>,
    label: Option<String>,
    max_initials: usize,
    decorative: bool,
}

impl Avatar {
    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The person's picture, already in the application's atlas.
    ///
    /// Decoding is deliberately not this component's job — the same line
    /// [`image`](crate::image) draws: what arrives here is an [`ImageId`], and
    /// how the bytes became one is the application's business (§9.6).
    pub fn image(mut self, image: ImageId) -> Self {
        self.image = Some(image);
        self
    }

    /// The picture, or the initials when there is none.
    ///
    /// The reason this exists next to [`Avatar::image`]: `None` is not an
    /// error, it is the ordinary case, and `avatar(name).image_or(photo)`
    /// reads better than an `if` at every call site.
    pub fn image_or(mut self, image: Option<ImageId>) -> Self {
        self.image = image;
        self
    }

    /// The diameter, from the spacing scale.
    pub fn size(mut self, token: SpaceToken) -> Self {
        self.diameter = Some(self.theme.space_of(token));
        self
    }

    /// The diameter in points, for a size that genuinely is not on the scale.
    pub fn size_raw(mut self, diameter: f32) -> Self {
        self.diameter = Some(diameter.max(0.0));
        self
    }

    /// 20pt — a table cell, a comment thread.
    pub fn xs(self) -> Self {
        self.size(SpaceToken::S5)
    }

    /// 24pt — a list row.
    pub fn sm(self) -> Self {
        self.size(SpaceToken::S6)
    }

    /// 32pt — the default: a toolbar, a card header.
    pub fn md(self) -> Self {
        self.size(SpaceToken::S8)
    }

    /// 40pt — a profile row.
    pub fn lg(self) -> Self {
        self.size(SpaceToken::S10)
    }

    /// 48pt — an account page.
    pub fn xl(self) -> Self {
        self.size(SpaceToken::S12)
    }

    /// A rounded square instead of a circle — the "app icon" shape.
    pub fn rounded(mut self, token: RadiusToken) -> Self {
        self.radius = Some(token);
        self
    }

    /// The fill behind the initials, from a token.
    pub fn tint(mut self, token: ColorToken) -> Self {
        self.background = Some(self.theme.color_of(token));
        self
    }

    /// The fill behind the initials, already resolved.
    ///
    /// The escape hatch for the one case the token set genuinely cannot
    /// answer: a colour **per person**, taken from a palette the application
    /// owns (see [`avatar_slot`]).
    pub fn tint_raw(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// The initials' colour, from a token.
    pub fn ink(mut self, token: ColorToken) -> Self {
        self.foreground = Some(self.theme.color_of(token));
        self
    }

    /// The initials' colour, already resolved.
    pub fn ink_raw(mut self, color: Color) -> Self {
        self.foreground = Some(color);
        self
    }

    /// A ring around the disc, in the colour of the surface behind it.
    ///
    /// [`avatar_group`] fills this in; on a lone avatar it is what makes a
    /// picture sitting on a busy background still read as a disc.
    pub fn ring(mut self, width: f32, color: Color) -> Self {
        self.ring = Some((width.max(0.0), color));
        self
    }

    /// The name a screen reader announces, when it is not the person's name.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// How many letters the initials may use (2 by default).
    pub fn max_initials(mut self, max: usize) -> Self {
        self.max_initials = max;
        self
    }

    /// Hide it from assistive technology entirely.
    ///
    /// The right answer when the name is already written beside the disc: a
    /// screen reader hearing "Dian Permata, image, Dian Permata" learns
    /// nothing from the second one.
    pub fn decorative(mut self) -> Self {
        self.decorative = true;
        self
    }

    /// The person's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The letters this avatar falls back to.
    pub fn initials(&self) -> String {
        initials(&self.name, self.max_initials)
    }

    /// True when this avatar draws a picture rather than letters.
    pub fn has_image(&self) -> bool {
        self.image.is_some()
    }

    /// Every resolved drawing value.
    pub fn style(&self) -> AvatarStyle {
        let d = self
            .diameter
            .unwrap_or_else(|| self.theme.space(AVATAR_STEPS));
        let mut style = AvatarStyle::from_theme(&self.theme, d);
        if let Some(token) = self.radius {
            style.corners = self.theme.corners_of(token);
        }
        if let Some(bg) = self.background {
            style.background = bg;
        }
        if let Some(fg) = self.foreground {
            style.foreground = fg;
        }
        if let Some((w, c)) = self.ring {
            style.ring_width = w;
            style.ring_color = c;
        }
        // Under a photograph the fill would only ever be seen through the
        // rounding, where it reads as a coloured halo rather than a background.
        if self.image.is_some() {
            style.background = Color::TRANSPARENT;
        }
        style
    }
}

impl From<Avatar> for View {
    fn from(a: Avatar) -> View {
        let style = a.style();
        let ring = style.ring_width.clamp(0.0, style.diameter * 0.25);
        let dalam = (style.diameter - ring * 2.0).max(0.0);

        let isi: Option<View> = match a.image {
            Some(id) => Some(View::from(
                image_in(&a.images, id)
                    .theme(&a.theme)
                    // `Cover` and not `Contain`: a portrait letterboxed inside a
                    // circle leaves two grey wedges, which is worse than a crop.
                    .fit(ImageFit::Cover)
                    .size(dalam, dalam)
                    .rounded_raw(style.corners.clamp_to(Size::new(dalam, dalam)))
                    // The disc carries the name; the bitmap inside it must not
                    // be announced a second time.
                    .decorative(),
            )),
            None => {
                let huruf = a.initials();
                (!huruf.is_empty()).then(|| {
                    View::from(
                        text_in(&a.fonts, huruf)
                            .size(style.diameter * INITIALS_RATIO)
                            .weight(FontWeight::SEMIBOLD)
                            .color(style.foreground)
                            .single_line()
                            .role(AccessRole::Container),
                    )
                })
            }
        };

        let mut builder = Builder::new(AvatarProps {
            style,
            label: Some(a.label.clone().unwrap_or_else(|| a.name.clone()))
                .filter(|l| !l.is_empty()),
            decorative: a.decorative,
        });
        if let Some(isi) = isi {
            builder = builder.child(isi);
        }
        if let Some(key) = a.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

// ---------------------------------------------------------------------------
// Group node
// ---------------------------------------------------------------------------

/// The overlapping row.
///
/// Two things make it a node rather than a `row` with a negative gap. First,
/// the layout engine has no negative gap, and adding one would be a general
/// feature invented for a single case. Second — and this is the one that
/// matters — the **paint order has to be the reverse of the layout order**: the
/// leading avatar is the one on top, so a stack reads as a deck of cards seen
/// from the left rather than from the right.
pub struct AvatarGroupBox {
    /// How far each avatar sits from the previous one.
    pub step: f32,
    /// The name a screen reader announces for the whole group.
    pub label: Option<String>,
}

impl RenderNode for AvatarGroupBox {
    fn type_name(&self) -> &'static str {
        "AvatarGroup"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let n = ctx.child_count();
        if n == 0 {
            return constraints.constrain(Size::ZERO);
        }
        let rtl = ctx.direction().is_rtl();
        let longgar = constraints.loosen();
        let mut ukuran: Vec<Size> = Vec::with_capacity(n);
        for i in 0..n {
            let id = ctx.child(i);
            ukuran.push(ctx.layout_child(id, longgar));
        }
        let tinggi = ukuran
            .iter()
            .map(|s| s.height)
            .fold(0.0f32, |a, b| a.max(b));
        // The last avatar contributes its whole width; every earlier one only
        // contributes the step it pushed the next one along by.
        let lebar = ukuran.last().map(|s| s.width).unwrap_or(0.0) + self.step * (n - 1) as f32;
        let size = constraints.constrain(Size::new(lebar.max(0.0), tinggi));

        for (i, s) in ukuran.iter().enumerate() {
            let x = self.step * i as f32;
            let id = ctx.child(i);
            ctx.place_child(
                id,
                Point::new(
                    if rtl {
                        (size.width - x - s.width).max(0.0)
                    } else {
                        x
                    },
                    (size.height - s.height) * 0.5,
                ),
            );
        }
        size
    }

    /// Painted back to front, so the **leading** avatar is the one on top.
    ///
    /// The default `paint_children` would draw them in layout order and put the
    /// last one on top, which makes the stack look like it is receding in the
    /// wrong direction — and hides the ring of every avatar but the last.
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        for i in (0..ctx.child_count()).rev() {
            let id = ctx.child(i);
            ctx.paint_child(id);
        }
    }

    /// A named group, or structural.
    ///
    /// Structural is the right default: without a name, "Dian Permata, Bagas
    /// Nugroho, +7" is exactly what a screen reader should read, and an
    /// anonymous group around it is one more level to walk past for no
    /// information (the trap [`AccessNode::selected`] documents).
    fn access(&self, node: &mut AccessNode) {
        match &self.label {
            Some(label) => {
                node.role = AccessRole::Group;
                node.label = Some(label.clone());
            }
            None => node.role = AccessRole::Container,
        }
    }
}

impl core::fmt::Debug for AvatarGroupBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AvatarGroupBox")
            .field("step", &self.step)
            .field("label", &self.label)
            .finish()
    }
}

/// The props of [`AvatarGroupBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct AvatarGroupProps {
    step: f32,
    label: Option<String>,
}

impl ViewNode for AvatarGroupProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(AvatarGroupBox {
            step: self.step,
            label: self.label.clone(),
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<AvatarGroupBox>()
            .expect("the same view type means the same render node type");
        let mut dirty = Dirty::NONE;
        if n.step != self.step {
            n.step = self.step;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

// ---------------------------------------------------------------------------
// Group builder
// ---------------------------------------------------------------------------

/// A stack of overlapping avatars, with a `+n` disc when they do not all fit.
///
/// Use [`avatar_group_in`] outside a build pass.
///
/// ```
/// use silka_widgets::{avatar, avatar_group};
///
/// let team = avatar_group([avatar("Dian Permata"), avatar("Bagas Nugroho")])
///     .label("Assignees");
/// # let _ = team;
/// ```
pub fn avatar_group(members: impl IntoIterator<Item = Avatar>) -> AvatarGroup {
    avatar_group_in(
        &crate::active_fonts(),
        &active_images(),
        &crate::ambient::active_theme(),
        members,
    )
}

/// [`avatar_group`] with the text engine, the atlas and the theme passed
/// explicitly.
pub fn avatar_group_in(
    fonts: &Fonts,
    images: &Images,
    theme: &Theme,
    members: impl IntoIterator<Item = Avatar>,
) -> AvatarGroup {
    AvatarGroup {
        fonts: fonts.clone(),
        images: images.clone(),
        theme: *theme,
        key: None,
        members: members.into_iter().collect(),
        diameter: None,
        max: 0,
        overlap: GROUP_OVERLAP,
        ring: None,
        label: None,
    }
}

/// The avatar-group builder — Dart-style (§2.5).
pub struct AvatarGroup {
    fonts: Fonts,
    images: Images,
    theme: Theme,
    key: Option<Key>,
    members: Vec<Avatar>,
    diameter: Option<f32>,
    max: usize,
    overlap: f32,
    ring: Option<(f32, Color)>,
    label: Option<String>,
}

impl AvatarGroup {
    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The diameter every member is forced to, from the spacing scale.
    ///
    /// One size for the whole row, because a stack of two different diameters
    /// reads as a mistake rather than as a hierarchy.
    pub fn size(mut self, token: SpaceToken) -> Self {
        self.diameter = Some(self.theme.space_of(token));
        self
    }

    /// The diameter in points.
    pub fn size_raw(mut self, diameter: f32) -> Self {
        self.diameter = Some(diameter.max(0.0));
        self
    }

    /// How many discs the row may hold before it collapses into a counter.
    ///
    /// The counter takes one of them, so this really is the width of the row
    /// (see [`group_plan`]). Zero means "never collapse".
    pub fn max(mut self, max: usize) -> Self {
        self.max = max;
        self
    }

    /// How much of its width each avatar gives up to its neighbour, 0…1.
    pub fn overlap(mut self, fraction: f32) -> Self {
        self.overlap = fraction.clamp(0.0, 0.9);
        self
    }

    /// The ring that separates the discs.
    ///
    /// Filled in automatically from [`ColorToken::Surface`]; override it when
    /// the group sits on something else.
    pub fn ring(mut self, width: f32, color: Color) -> Self {
        self.ring = Some((width.max(0.0), color));
        self
    }

    /// The name a screen reader announces for the whole group.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// How many members it holds, counting the ones that will be hidden.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// True when it holds none.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The diameter every member will use.
    pub fn diameter(&self) -> f32 {
        self.diameter
            .unwrap_or_else(|| self.theme.space(AVATAR_STEPS))
    }

    /// How many discs are shown, and how many the counter admits to.
    pub fn plan(&self) -> (usize, usize) {
        group_plan(self.members.len(), self.max)
    }
}

impl From<AvatarGroup> for View {
    fn from(g: AvatarGroup) -> View {
        let d = g.diameter();
        let (ring_w, ring_c) = g
            .ring
            .unwrap_or((g.theme.space(0.5), g.theme.color_of(ColorToken::Surface)));
        let (shown, overflow) = g.plan();

        let mut anak: Vec<View> = g
            .members
            .into_iter()
            .take(shown)
            .map(|a| View::from(a.size_raw(d).ring(ring_w, ring_c)))
            .collect();
        if overflow > 0 {
            // The counter is an avatar too, so it gets the same ring and the
            // same diameter for free — and it is named, because "+7" on its
            // own tells a screen reader nothing.
            anak.push(View::from(
                avatar_in(&g.fonts, &g.images, &g.theme, format!("+{overflow}"))
                    .size_raw(d)
                    .ring(ring_w, ring_c)
                    .tint(ColorToken::SurfaceSunken)
                    .ink(ColorToken::SecondaryLabel)
                    .max_initials(0)
                    .label(format!("{overflow} more")),
            ));
        }

        let mut builder = Builder::new(AvatarGroupProps {
            step: d * (1.0 - g.overlap.clamp(0.0, 0.9)),
            label: g.label,
        })
        .children(anak);
        if let Some(key) = g.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

impl core::fmt::Debug for AvatarGroup {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AvatarGroup")
            .field("members", &self.members.len())
            .field("max", &self.max)
            .field("label", &self.label)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::tree::{NodeId, RenderTree, TextDirection};
    use silka_core::view::reconcile;
    use silka_theme::{Appearance, Preset};

    const BOX: Size = Size::new(400.0, 200.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    // The ambient handle rather than a fresh engine per call: `Fonts`
    // compares by identity, so two engines would make every rebuild look like
    // a change and the no-op test below would be measuring nothing.
    fn fonts() -> Fonts {
        crate::active_fonts()
    }

    fn laid_out(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(BOX));
        tree
    }

    fn one(name: &str) -> Avatar {
        avatar_in(&fonts(), &Images::new(), &theme(), name)
    }

    fn find<T: RenderNode>(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
        if tree.node_ref::<T>(id).is_some() {
            return Some(id);
        }
        for c in tree.children(id) {
            if let Some(found) = find::<T>(tree, *c) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn initials_are_the_main_case_not_a_degraded_one() {
        assert_eq!(initials("Super Admin", 2), "SA");
        assert_eq!(initials("dian permata sari", 2), "DP");
        assert_eq!(initials("Bagas", 2), "B");
        assert_eq!(initials("", 2), "");
        assert_eq!(initials("   ", 2), "");
        assert_eq!(initials("Bagas", 0), "");
        assert_eq!(initials("a b c d", 3), "ABC");
    }

    #[test]
    fn initials_skip_what_is_not_a_letter() {
        // A name written with a leading bullet or a quotation mark must not
        // produce an avatar reading "•".
        assert_eq!(initials("(Dian) Permata", 2), "DP");
        assert_eq!(initials("- -", 2), "");
    }

    #[test]
    fn initials_survive_a_script_with_no_capital_letters() {
        assert_eq!(initials("山田 太郎", 2), "山太");
        assert_eq!(initials("дмитрий сергеев", 2), "ДС");
    }

    #[test]
    fn the_slot_of_a_name_is_stable_and_insensitive_to_its_spelling() {
        // A colour per person is only worth having if it is the *same* colour
        // next time the application starts — which is why this is FNV rather
        // than the standard library's randomly seeded hasher.
        assert_eq!(
            avatar_slot("Dian Permata", 8),
            avatar_slot("dian permata", 8)
        );
        assert_eq!(
            avatar_slot("Dian Permata", 8),
            avatar_slot("  Dian Permata ", 8)
        );
        assert_eq!(avatar_slot("Dian Permata", 8), 3);
        for name in ["a", "Dian Permata", "山田", ""] {
            assert!(avatar_slot(name, 8) < 8, "{name}");
        }
        assert_eq!(avatar_slot("x", 0), 0);
        assert_eq!(avatar_slot("x", 1), 0);
    }

    #[test]
    fn different_names_land_in_different_slots_often_enough_to_be_useful() {
        let nama = [
            "Dian Permata",
            "Bagas Nugroho",
            "Sari Wulandari",
            "Rizky Pratama",
            "Putri Ayu",
            "Andi Saputra",
        ];
        let mut slot: Vec<usize> = nama.iter().map(|n| avatar_slot(n, 8)).collect();
        slot.sort_unstable();
        slot.dedup();
        assert!(
            slot.len() >= 4,
            "a hash that put six names in {} colours is not an identity colour",
            slot.len()
        );
    }

    #[test]
    fn an_avatar_is_a_named_image_and_never_an_anonymous_one() {
        let tree = laid_out(one("Dian Permata"));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Dian Permata")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Image);
    }

    #[test]
    fn a_decorative_avatar_leaves_the_tree_entirely() {
        // The right answer when the name is written beside the disc: hearing
        // it twice teaches nothing.
        let tree = laid_out(one("Dian Permata").decorative());
        let a11y = tree.access_tree(None);
        assert!(a11y.find_label("Dian Permata").is_none(), "{}", a11y.dump());
    }

    #[test]
    fn the_initials_are_not_announced_on_top_of_the_name() {
        let tree = laid_out(one("Dian Permata"));
        let a11y = tree.access_tree(None);
        assert!(a11y.find_label("DP").is_none(), "{}", a11y.dump());
    }

    #[test]
    fn a_disc_is_square_and_round_at_every_size() {
        let t = theme();
        for token in [SpaceToken::S5, SpaceToken::S8, SpaceToken::S12] {
            let tree = laid_out(one("Dian Permata").size(token));
            let id = find::<AvatarBox>(&tree, tree.root()).expect("an avatar node");
            let size = tree.size(id);
            assert_eq!(size.width, size.height);
            assert_eq!(size.width, t.space_of(token));
            // Half the diameter is a circle whatever the preset's corner shape.
            let node = tree.node_ref::<AvatarBox>(id).unwrap();
            assert_eq!(node.style.corners.radii.max(), t.space_of(token) * 0.5);
        }
    }

    #[test]
    fn every_size_and_colour_moves_with_the_preset_and_the_appearance() {
        for preset in Preset::ALL {
            let light = AvatarStyle::from_theme(&Theme::new(preset, Appearance::Light), 32.0);
            let dark = AvatarStyle::from_theme(&Theme::new(preset, Appearance::Dark), 32.0);
            assert_ne!(light.background, dark.background, "{preset:?}");
            assert_ne!(light.ring_color, dark.ring_color, "{preset:?}");
        }
    }

    #[test]
    fn a_named_empty_string_still_draws_a_disc_and_says_nothing() {
        // The account with no name at all: a coloured circle is fine, an
        // avatar announcing "" is not.
        let tree = laid_out(one(""));
        let id = find::<AvatarBox>(&tree, tree.root()).expect("an avatar node");
        assert!(tree.size(id).width > 0.0);
        assert!(tree.node_ref::<AvatarBox>(id).unwrap().label.is_none());
    }

    #[test]
    fn the_group_overlaps_rather_than_lining_up() {
        let t = theme();
        let f = fonts();
        let im = Images::new();
        let d = t.space(AVATAR_STEPS);
        let tree = laid_out(avatar_group_in(
            &f,
            &im,
            &t,
            ["A One", "B Two", "C Three"].map(|n| avatar_in(&f, &im, &t, n)),
        ));
        let id = find::<AvatarGroupBox>(&tree, tree.root()).expect("a group node");
        // Three 32pt discs side by side would be 96pt wide; overlapping by a
        // third they are not.
        assert!(tree.size(id).width < d * 3.0);
        assert!(tree.size(id).width > d);
        assert_eq!(tree.size(id).height, d);
    }

    #[test]
    fn the_leading_avatar_is_the_one_on_top() {
        // Painted back to front. Drawing them in layout order would put the
        // last one on top and hide every ring but its own.
        let t = theme();
        let f = fonts();
        let im = Images::new();
        let mut tree = laid_out(avatar_group_in(
            &f,
            &im,
            &t,
            ["A One", "B Two"].map(|n| avatar_in(&f, &im, &t, n)),
        ));
        let mut scene = silka_paint::Scene::new(Color::BLACK);
        tree.paint_into(&mut scene);
        let x: Vec<f32> = scene
            .commands()
            .iter()
            .filter_map(|c| match c {
                silka_paint::Command::Quad(q) => Some(q.rect.origin.x),
                _ => None,
            })
            .collect();
        assert!(x.len() >= 2, "two discs, at least");
        assert!(
            x[0] > *x.last().unwrap(),
            "the trailing avatar has to be drawn first so the leading one \
             lands on top of it: {x:?}"
        );
    }

    #[test]
    fn a_group_that_fits_shows_no_counter_at_all() {
        assert_eq!(group_plan(3, 3), (3, 0));
        assert_eq!(group_plan(1, 3), (1, 0));
        assert_eq!(group_plan(0, 3), (0, 0));
        assert_eq!(group_plan(9, 0), (9, 0));
    }

    #[test]
    fn a_group_that_overflows_keeps_its_row_the_width_it_promised() {
        // `max` is the number of discs on the row, counter included — the
        // off-by-one that otherwise makes `max(3)` draw four things.
        assert_eq!(group_plan(9, 3), (2, 7));
        let t = theme();
        let f = fonts();
        let im = Images::new();
        let tree = laid_out(
            avatar_group_in(
                &f,
                &im,
                &t,
                (0..9).map(|i| avatar_in(&f, &im, &t, format!("N{i} X{i}"))),
            )
            .max(3),
        );
        let id = find::<AvatarGroupBox>(&tree, tree.root()).unwrap();
        assert_eq!(tree.children(id).len(), 3);
    }

    #[test]
    fn the_counter_says_how_many_rather_than_showing_a_symbol() {
        let t = theme();
        let f = fonts();
        let im = Images::new();
        let tree = laid_out(
            avatar_group_in(
                &f,
                &im,
                &t,
                (0..9).map(|i| avatar_in(&f, &im, &t, format!("N{i} X{i}"))),
            )
            .max(3),
        );
        let a11y = tree.access_tree(None);
        assert!(
            a11y.find_label("7 more").is_some(),
            "\"+7\" on its own tells a screen reader nothing: {}",
            a11y.dump()
        );
    }

    #[test]
    fn a_group_is_a_landmark_only_when_it_was_named() {
        let t = theme();
        let f = fonts();
        let im = Images::new();
        let build = |label: Option<&str>| {
            let g = avatar_group_in(&f, &im, &t, [avatar_in(&f, &im, &t, "A One")]);
            match label {
                Some(l) => g.label(l),
                None => g,
            }
        };
        let named = laid_out(build(Some("Assignees")));
        assert!(named.access_tree(None).find_label("Assignees").is_some());
        let anon = laid_out(build(None));
        assert!(
            anon.access_tree(None)
                .find_role(AccessRole::Group)
                .is_none(),
            "an anonymous group is one more level to walk past for no information"
        );
    }

    #[test]
    fn the_group_mirrors_in_an_rtl_document() {
        let t = theme();
        let f = fonts();
        let im = Images::new();
        let build = || {
            avatar_group_in(
                &f,
                &im,
                &t,
                ["A One", "B Two", "C Three"].map(|n| avatar_in(&f, &im, &t, n)),
            )
        };
        let mut ltr = RenderTree::new();
        reconcile(&mut ltr, build());
        ltr.layout(BoxConstraints::loose(BOX));
        let mut rtl = RenderTree::new();
        reconcile(&mut rtl, build());
        rtl.set_direction(TextDirection::Rtl);
        rtl.layout(BoxConstraints::loose(BOX));

        let ambil = |tree: &RenderTree| -> Vec<f32> {
            let g = find::<AvatarGroupBox>(tree, tree.root()).unwrap();
            tree.children(g).iter().map(|c| tree.offset(*c).x).collect()
        };
        let a = ambil(&ltr);
        let b = ambil(&rtl);
        assert_ne!(a, b);
        // The first avatar leads, so in a mirrored document it sits on the
        // right-hand end of the row.
        assert!(a[0] < a[2]);
        assert!(b[0] > b[2]);
    }

    #[test]
    fn every_member_of_a_group_gets_the_same_diameter_and_a_ring() {
        let t = theme();
        let f = fonts();
        let im = Images::new();
        let tree = laid_out(
            avatar_group_in(
                &f,
                &im,
                &t,
                [
                    avatar_in(&f, &im, &t, "A One").xl(),
                    avatar_in(&f, &im, &t, "B Two").xs(),
                ],
            )
            .size(SpaceToken::S8),
        );
        let g = find::<AvatarGroupBox>(&tree, tree.root()).unwrap();
        for c in tree.children(g) {
            let node = tree.node_ref::<AvatarBox>(*c).expect("an avatar node");
            assert_eq!(node.style.diameter, t.space_of(SpaceToken::S8));
            assert!(
                node.ring_width() > 0.0,
                "without a ring four overlapping discs read as one blob"
            );
        }
    }

    #[test]
    fn a_ring_can_never_swallow_the_disc_it_outlines() {
        let tree = laid_out(one("A One").size_raw(20.0).ring(40.0, Color::WHITE));
        let id = find::<AvatarBox>(&tree, tree.root()).unwrap();
        let node = tree.node_ref::<AvatarBox>(id).unwrap();
        assert!(node.ring_width() <= 5.0);
    }

    #[test]
    fn rebuilding_an_identical_avatar_does_nothing_at_all() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, one("Dian Permata"));
        tree.layout(BoxConstraints::loose(BOX));
        let again = reconcile(&mut tree, one("Dian Permata"));
        assert_eq!(again.created, 0);
        assert!(again.is_noop(), "identical props must be free");
    }
}
