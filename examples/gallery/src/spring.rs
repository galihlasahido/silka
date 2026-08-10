//! Demo page: the **interactive spring playground** (REKOMENDASI §3.5).
//!
//! Every transition in this design system is a spring, and a spring is the one
//! thing a screenshot cannot show. This page is where it can be *felt*: four
//! lanes race the same distance at the same instant, three of them on the
//! framework's presets ([`Spring::smooth`], [`Spring::snappy`],
//! [`Spring::bouncy`]) and the fourth on parameters you move with sliders.
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | Springs are **retargetable mid-flight** | Click far to the right, then immediately click on the left: the pucks reverse **carrying their velocity**; they never jump |
//! | Position **and** velocity are state, not a timeline | Drag across a lane: the puck chases the finger continuously instead of restarting an animation on every move |
//! | Duration/bounce is the API, stiffness/damping is the consequence | Move the sliders: the readout under them is derived from the spring, not typed in twice (WWDC23: perceptual duration, not stiffness) |
//! | Reduced motion is honoured | Flip "Kurangi gerak" in the top bar: the bouncy lane stops overshooting, and nothing else about the page changes |
//! | Idle really is zero | Once the pucks settle, the window stops asking for frames — the GPU sleeps until the next click (§3.5) |
//!
//! The lane is the gallery's **own render node**. That is deliberate: it shows
//! that a component outside `silka-widgets` obeys exactly the same contract —
//! `layout`/`paint`/`access`/`event`, its springs stepped by an `advance`
//! function the application composes into a single tick, and not one wgpu type
//! in sight (§3.2, §3.5, §3.8).

use std::rc::Rc;

use silka_core::access::{AccessNode, AccessRole};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::input::{Event, EventCtx, HitBehavior, PointerButton, PointerPhase};
use silka_core::scheduler::Dirty;
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{
    BoxConstraints, CrossAlign, LayoutCtx, MainAlign, NodeId, PaintCtx, RenderNode, RenderTree,
};
use silka_core::view::{column, constrained, row, Builder, View, ViewNode};
use silka_paint::{Color, Corners, Insets, Point, Quad, Rect, ShadowPair, Size};
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{button, button_variant, slider, text, ButtonVariant, Fonts};

/// The page title.
pub const JUDUL: &str = "Spring";

/// The width of the playground, in spacing steps.
const LEBAR_LANGKAH: f32 = 120.0;

/// The a11y name (and slider label) of the duration control.
pub const DURASI: &str = "Durasi";
/// The a11y name of the bounce control.
pub const BOUNCE: &str = "Bounce";
/// The button that sends every puck to the far end.
pub const KIRIM: &str = "Lempar ke ujung";
/// The button that sends every puck home.
pub const PULANG: &str = "Kembali";
/// The button that sends every puck to the middle.
pub const TENGAH: &str = "Ke tengah";

/// The smallest and largest duration the slider offers, in seconds.
const DURASI_MIN: f32 = 0.1;
const DURASI_MAKS: f32 = 1.5;
/// The bounce range: negative is over-damped (creeps in), positive overshoots.
const BOUNCE_MIN: f32 = -0.4;
const BOUNCE_MAKS: f32 = 0.7;

/// Default position of the pucks: near the start, but not glued to the edge.
const AWAL: f32 = 0.04;

/// One lane of the playground: a name and the spring that drives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lajur {
    /// The label shown above the lane — also its a11y name.
    pub nama: &'static str,
    /// Which of the four springs it runs.
    pub jenis: Jenis,
}

/// Which spring a lane runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jenis {
    /// [`Spring::smooth`] — the framework default: no overshoot at all.
    Smooth,
    /// [`Spring::snappy`] — short, with a hint of overshoot.
    Snappy,
    /// [`Spring::bouncy`] — the playful one.
    Bouncy,
    /// The one the sliders control.
    Sendiri,
}

/// The four lanes, top to bottom.
pub const LAJUR: [Lajur; 4] = [
    Lajur {
        nama: "smooth",
        jenis: Jenis::Smooth,
    },
    Lajur {
        nama: "snappy",
        jenis: Jenis::Snappy,
    },
    Lajur {
        nama: "bouncy",
        jenis: Jenis::Bouncy,
    },
    Lajur {
        nama: "sendiri",
        jenis: Jenis::Sendiri,
    },
];

/// The a11y name of one lane.
///
/// Deliberately *not* the same string as the caption printed above it: two
/// nodes sharing a name are two nodes a screen reader cannot tell apart — and
/// two nodes a test cannot tell apart either.
pub fn nama_lajur(l: Lajur) -> String {
    format!("lajur {}", l.nama)
}

impl Jenis {
    /// The spring for this lane; `durasi`/`bounce` only matter to
    /// [`Jenis::Sendiri`].
    pub fn spring(self, durasi: f32, bounce: f32) -> Spring {
        match self {
            Jenis::Smooth => Spring::smooth(),
            Jenis::Snappy => Spring::snappy(),
            Jenis::Bouncy => Spring::bouncy(),
            Jenis::Sendiri => Spring::new(durasi, bounce),
        }
    }

    /// The lane's colour — a token, never a literal (§2.6).
    pub fn warna(self, t: &Theme) -> Color {
        match self {
            Jenis::Smooth => t.color.accent,
            Jenis::Snappy => t.color.success,
            Jenis::Bouncy => t.color.warning,
            Jenis::Sendiri => t.color.destructive,
        }
    }
}

/// A one-line summary of a spring, derived from it rather than restated.
///
/// The whole point of the duration/bounce API is that stiffness and damping
/// are **consequences**; printing them next to the sliders is what makes that
/// visible instead of merely claimed.
pub fn ringkasan(spring: Spring) -> String {
    format!(
        "durasi {:.2} s · bounce {:+.2} · kekakuan {:.0} · redaman {:.0} · rasio {:.2}",
        spring.duration(),
        spring.bounce(),
        spring.stiffness(),
        spring.damping(),
        spring.damping_ratio(),
    )
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

/// The view tree for the whole page.
///
/// The title and the prose are read in the root scope; the target and the
/// spring parameters are read one level down, so dragging a slider rebuilds
/// the playground and the readout — not the page (§2.5).
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    // The target is owned by the **application**, exactly like a checkbox's
    // value: the node owns the motion, never the destination.
    let target = use_signal(|| AWAL);
    let durasi = use_signal(|| 0.5f32);
    let bounce = use_signal(|| 0.2f32);

    column([
        View::from(
            text(fonts, JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                fonts,
                "Empat lajur, satu tujuan yang sama. Klik di mana saja pada \
                 sebuah lajur — lalu klik lagi di seberangnya sebelum pucknya \
                 sampai: mereka berbalik sambil membawa kecepatannya, bukan \
                 mengulang dari nol. Seret untuk membuat targetnya bergerak \
                 terus.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(LEBAR_LANGKAH)),
        ),
        arena(fonts, target, durasi, bounce),
        kendali(fonts, target, durasi, bounce),
    ])
    .spacing(t.space(5.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// The four lanes as **one component**: the only place the target is read.
fn arena(fonts: &Fonts, target: Signal<f32>, durasi: Signal<f32>, bounce: Signal<f32>) -> View {
    let fonts = fonts.clone();
    component("arena-spring", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let tujuan = target.get();
        let d = durasi.get();
        let b = bounce.get();

        let mut anak: Vec<View> = Vec::with_capacity(LAJUR.len() * 2);
        for l in LAJUR {
            anak.push(
                text(&fonts, l.nama)
                    .size(t.typography.caption1.size)
                    .color(t.color.tertiary_label)
                    .single_line()
                    .into(),
            );
            anak.push(
                lintasan(&t, l.jenis.spring(d, b), l.jenis.warna(&t))
                    .key(l.nama)
                    .label(nama_lajur(l))
                    .target(tujuan)
                    .on_target(move |v| target.set(v))
                    .into(),
            );
        }

        constrained(
            BoxConstraints::new(
                t.space(LEBAR_LANGKAH),
                t.space(LEBAR_LANGKAH),
                0.0,
                f32::INFINITY,
            ),
            column(anak)
                .spacing(t.space(1.0))
                .cross(CrossAlign::Stretch)
                .padding(Insets::all(t.space(3.0)))
                .background(t.color.surface)
                .corners(t.corners(t.radius.lg))
                .border(t.space(0.25), t.color.separator),
        )
        .into()
    })
}

/// The sliders, the buttons, and the readout derived from them.
fn kendali(fonts: &Fonts, target: Signal<f32>, durasi: Signal<f32>, bounce: Signal<f32>) -> View {
    let fonts = fonts.clone();
    component("kendali-spring", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let d = durasi.get();
        let b = bounce.get();

        let baris_durasi = row([
            View::from(
                text(&fonts, DURASI)
                    .size(t.typography.body_size)
                    .color(t.color.secondary_label)
                    .single_line(),
            ),
            View::from(constrained(
                BoxConstraints::new(t.space(60.0), t.space(60.0), 0.0, f32::INFINITY),
                slider(&t, d)
                    .range(DURASI_MIN..=DURASI_MAKS)
                    .step(0.05)
                    .label(DURASI)
                    .on_change(move |v| durasi.set(v)),
            )),
        ])
        .spacing(t.space(3.0))
        .cross(CrossAlign::Center);

        let baris_bounce = row([
            View::from(
                text(&fonts, BOUNCE)
                    .size(t.typography.body_size)
                    .color(t.color.secondary_label)
                    .single_line(),
            ),
            View::from(constrained(
                BoxConstraints::new(t.space(60.0), t.space(60.0), 0.0, f32::INFINITY),
                slider(&t, b)
                    .range(BOUNCE_MIN..=BOUNCE_MAKS)
                    .step(0.05)
                    .label(BOUNCE)
                    .on_change(move |v| bounce.set(v)),
            )),
        ])
        .spacing(t.space(3.0))
        .cross(CrossAlign::Center);

        let tombol = row([
            View::from(button(&fonts, &t, KIRIM).on_press(move || target.set(1.0))),
            View::from(
                button_variant(&fonts, &t, PULANG, ButtonVariant::Secondary)
                    .on_press(move || target.set(0.0)),
            ),
            View::from(
                button_variant(&fonts, &t, TENGAH, ButtonVariant::Secondary)
                    .on_press(move || target.set(0.5)),
            ),
        ])
        .spacing(t.space(3.0))
        .cross(CrossAlign::Center);

        column([
            View::from(baris_durasi),
            View::from(baris_bounce),
            View::from(
                text(&fonts, ringkasan(Spring::new(d, b)))
                    .size(t.typography.footnote.size)
                    .color(t.color.tertiary_label)
                    .single_line(),
            ),
            View::from(tombol),
        ])
        .spacing(t.space(3.0))
        .cross(CrossAlign::Center)
        .into()
    })
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every measurement and colour a lane draws with — all of them tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GayaLintasan {
    /// Height of the whole lane.
    pub tinggi: f32,
    /// Thickness of the rail the puck slides along.
    pub tebal: f32,
    /// Diameter of the puck.
    pub puck: f32,
    /// Free space at both ends, so an overshoot stays inside the box instead
    /// of being clipped at the edge — the overshoot is the thing worth seeing.
    pub tepi: f32,
    /// The rail's colour.
    pub rel: Color,
    /// The rail's hairline.
    pub batas: Color,
    /// Hairline width.
    pub lebar_batas: f32,
    /// The colour of the target marker.
    pub penanda: Color,
    /// Corner geometry of the rail (a pill).
    pub corners_rel: Corners,
    /// Corner geometry of the puck (a circle).
    pub corners_puck: Corners,
    /// The puck's layered shadow.
    pub bayangan: ShadowPair,
}

impl GayaLintasan {
    /// Resolve the style from the active theme.
    pub fn from_theme(t: &Theme) -> Self {
        let puck = t.space(6.0);
        Self {
            tinggi: puck + t.space(2.0),
            tebal: t.space(1.5),
            puck,
            tepi: t.space(2.0),
            rel: t.color.surface_sunken,
            batas: t.color.separator,
            lebar_batas: t.space(0.25),
            penanda: t.color.tertiary_label,
            corners_rel: t.corners(t.radius.full),
            corners_puck: t.corners(t.radius.full),
            bayangan: t.shadow.sm,
        }
    }
}

// ---------------------------------------------------------------------------
// Render node
// ---------------------------------------------------------------------------

/// The action a lane reports when the pointer picks a new target.
#[derive(Clone)]
pub struct TargetCallback(Rc<dyn Fn(f32)>);

impl TargetCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(f32) + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run it with the fraction the pointer landed on.
    pub fn call(&self, fraksi: f32) {
        (self.0)(fraksi)
    }
}

impl PartialEq for TargetCallback {
    /// Identity, not contents: closures are rebuilt on every rebuild.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for TargetCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TargetCallback")
    }
}

/// One lane: a rail, a target marker, and a puck driven by a spring.
#[derive(Debug)]
pub struct LintasanSpring {
    nilai: SpringValue<f32>,
    warna: Color,
    gaya: GayaLintasan,
    label: Option<String>,
    ukuran: Size,
    on_target: Option<TargetCallback>,
    /// True between `Down` and `Up`: a `Move` only counts as scrubbing while
    /// the finger is genuinely down (the same rule the slider follows).
    menyeret: bool,
}

impl LintasanSpring {
    /// The puck's current position, `0.0` at the start and `1.0` at the end.
    ///
    /// It may leave that range while a bouncy spring overshoots — which is
    /// exactly why the lane reserves [`GayaLintasan::tepi`] at both ends.
    pub fn posisi(&self) -> f32 {
        self.nilai.position()
    }

    /// Where the puck is heading.
    pub fn target(&self) -> f32 {
        self.nilai.target()
    }

    /// The spring currently driving it.
    pub fn spring(&self) -> Spring {
        self.nilai.spring()
    }

    /// True while the puck is still moving.
    pub fn is_animating(&self) -> bool {
        self.nilai.is_animating()
    }

    /// Advance by one frame; true when the drawn position changed.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let sebelum = self.nilai.position();
        tick.advance(&mut self.nilai);
        self.nilai.position() != sebelum
    }

    /// The travel band: where the puck's left edge may sit.
    fn rentang(&self) -> (f32, f32) {
        let kiri = self.gaya.tepi;
        let kanan = (self.ukuran.width - self.gaya.puck - self.gaya.tepi).max(kiri);
        (kiri, kanan)
    }

    /// The puck's rectangle for a given fraction.
    fn puck_rect(&self, fraksi: f32) -> Rect {
        let (kiri, kanan) = self.rentang();
        let x = kiri + (kanan - kiri) * fraksi;
        let y = ((self.ukuran.height - self.gaya.puck) * 0.5).max(0.0);
        Rect::new(x, y, self.gaya.puck, self.gaya.puck)
    }

    /// The fraction a pointer at `local` is asking for.
    ///
    /// The puck's **centre** follows the finger, so the point under the cursor
    /// is the point the puck lands on — the same rule as a slider thumb.
    pub fn fraksi_dari(&self, local: Point) -> f32 {
        let (kiri, kanan) = self.rentang();
        let lebar = kanan - kiri;
        if lebar <= 0.0 {
            return 0.0;
        }
        ((local.x - self.gaya.puck * 0.5 - kiri) / lebar).clamp(0.0, 1.0)
    }

    /// Report the fraction under the pointer to the application.
    ///
    /// The lane never writes its own target: it asks, the application decides,
    /// and the new value comes back as props. That is what keeps every lane in
    /// step with a single signal (§2.5).
    fn pilih_target(&mut self, ctx: &mut EventCtx<'_>) {
        let fraksi = self.fraksi_dari(ctx.local());
        // The callback is copied out before it runs: it writes a signal, and
        // that write schedules a frame through this very node.
        if let Some(cb) = self.on_target.clone() {
            cb.call(fraksi);
        }
        ctx.request_animation();
        ctx.request_paint();
        ctx.handled();
    }
}

impl RenderNode for LintasanSpring {
    fn type_name(&self) -> &'static str {
        "LintasanSpring"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        // As wide as it is allowed to be, as tall as the style says: the width
        // is the playground, so it must not shrink to its content.
        let lebar = if constraints.max_width.is_finite() {
            constraints.max_width
        } else {
            self.gaya.puck * 8.0
        };
        self.ukuran = constraints.constrain(Size::new(lebar, self.gaya.tinggi));
        self.ukuran
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let g = self.gaya;
        let size = ctx.size();
        if size.is_empty() {
            return;
        }

        // The rail.
        let y = ((size.height - g.tebal) * 0.5).max(0.0);
        let rel = Rect::new(0.0, y, size.width, g.tebal);
        ctx.quad(
            Quad::new(rel)
                .background(g.rel)
                .corners(g.corners_rel.clamp_to(rel.size))
                .border(g.lebar_batas, g.batas),
        );

        // Where the puck is heading — a hairline, so the distance still to
        // cover is visible while it is being covered.
        let tanda = self.puck_rect(self.target().clamp(0.0, 1.0));
        let lebar_tanda = g.lebar_batas * 2.0;
        ctx.quad(
            Quad::new(Rect::new(
                tanda.center().x - lebar_tanda * 0.5,
                0.0,
                lebar_tanda,
                size.height,
            ))
            .background(g.penanda),
        );

        // The puck itself.
        let puck = self.puck_rect(self.posisi());
        ctx.shadowed(
            Quad::new(puck)
                .background(self.warna)
                .corners(g.corners_puck.clamp_to(puck.size)),
            g.bayangan,
        );
    }

    fn access(&self, node: &mut AccessNode) {
        // There is no "animated demo" role in the vocabulary, and inventing
        // one would touch the platform adapter; `Image` plus a spoken summary
        // is the honest choice — the same one `silka-chart` makes.
        node.role = AccessRole::Image;
        node.label.clone_from(&self.label);
        node.value = Some(ringkasan(self.spring()));
    }

    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else { return };
        match p.phase {
            PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                self.menyeret = true;
                ctx.capture_pointer();
                self.pilih_target(ctx);
            }
            // Dragging keeps writing the target: the spring is asked to chase
            // a moving destination, which is the clearest demonstration that
            // retargeting is not a special case.
            PointerPhase::Move if self.menyeret => self.pilih_target(ctx),
            PointerPhase::Up | PointerPhase::Cancel if self.menyeret => {
                self.menyeret = false;
                ctx.release_pointer();
                ctx.handled();
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// A lane's props — the view form of [`LintasanSpring`].
#[derive(Debug, Clone, PartialEq)]
pub struct LintasanProps {
    target: f32,
    spring: Spring,
    warna: Color,
    gaya: GayaLintasan,
    label: Option<String>,
    on_target: Option<TargetCallback>,
}

impl ViewNode for LintasanProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(LintasanSpring {
            // A fresh lane starts **at** its target: a page that has just been
            // opened must not animate in from nowhere.
            nilai: SpringValue::new(self.target).with_spring(self.spring),
            warna: self.warna,
            gaya: self.gaya,
            label: self.label.clone(),
            ukuran: Size::ZERO,
            on_target: self.on_target.clone(),
            menyeret: false,
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<LintasanSpring>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.nilai.spring() != self.spring {
            n.nilai.set_spring(self.spring);
        }
        // The heart of the page: a new target **retargets** the spring, it
        // does not restart it. Position and velocity survive.
        if n.nilai.target() != self.target {
            n.nilai.set_target(self.target);
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.warna != self.warna {
            n.warna = self.warna;
            dirty |= Dirty::PAINT;
        }
        if n.gaya != self.gaya {
            n.gaya = self.gaya;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
        }
        n.on_target.clone_from(&self.on_target);
        dirty
    }
}

/// A lane builder — a Dart-style constructor (§2.5).
#[derive(Debug)]
pub struct LintasanBuilder {
    props: LintasanProps,
    key: Option<silka_core::signals::Key>,
}

/// One lane of the playground, driven by `spring` and painted in `warna`.
pub fn lintasan(theme: &Theme, spring: Spring, warna: Color) -> LintasanBuilder {
    LintasanBuilder {
        props: LintasanProps {
            target: AWAL,
            spring,
            warna,
            gaya: GayaLintasan::from_theme(theme),
            label: None,
            on_target: None,
        },
        key: None,
    }
}

impl LintasanBuilder {
    /// This lane's identity among its siblings.
    pub fn key(mut self, key: impl Into<silka_core::signals::Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Where the puck should head, `0.0..=1.0`.
    pub fn target(mut self, target: f32) -> Self {
        self.props.target = target;
        self
    }

    /// The a11y name.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.props.label = Some(label.into());
        self
    }

    /// Called when the pointer picks a new target.
    pub fn on_target(mut self, f: impl Fn(f32) + 'static) -> Self {
        self.props.on_target = Some(TargetCallback::new(f));
        self
    }

    /// Override the resolved style.
    pub fn style(mut self, gaya: GayaLintasan) -> Self {
        self.props.gaya = gaya;
        self
    }
}

impl From<LintasanBuilder> for View {
    fn from(b: LintasanBuilder) -> View {
        let mut builder = Builder::new(b.props);
        if let Some(key) = b.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

// ---------------------------------------------------------------------------
// Animation pump
// ---------------------------------------------------------------------------

fn semua(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
    out.push(id);
    for anak in tree.children(id) {
        semua(tree, *anak, out);
    }
}

/// Advance every lane in `tree` by one frame.
///
/// The same shape as [`silka_widgets::advance`] and `silka_chart::advance`,
/// for the same reason: animation belongs to whoever owns the node, and the
/// application still calls **one** function per frame (§3.5). Only pixels
/// change — a puck moves inside its own lane, so nothing is laid out again.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut daftar = Vec::new();
    semua(tree, tree.root(), &mut daftar);

    let mut dirty = Dirty::NONE;
    for id in daftar {
        let hasil = tree
            .node_mut_ref::<LintasanSpring>(id)
            .map(|l| (l.advance(tick), l.is_animating()));
        if let Some((bergeser, bergerak)) = hasil {
            if bergeser {
                tree.mark_needs_paint(id);
                dirty |= Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
        }
    }
    dirty
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::animation::Motion;
    use silka_core::app::AppRuntime;
    use silka_core::input::PointerEvent;
    use silka_paint::Command;
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use std::time::Duration;

    const VIEWPORT: Size = Size::new(900.0, 760.0);
    const FRAME: Duration = Duration::from_micros(8_333);

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        let mut ui = headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height);
        ui.frame();
        ui
    }

    fn tema() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    fn ketuk(ui: &mut AppRuntime, p: Point) {
        for e in [
            PointerEvent::new(PointerPhase::Move, p, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, p, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, p, Duration::from_millis(60))
                .button(PointerButton::Primary),
        ] {
            ui.dispatch(&Event::Pointer(e));
        }
        ui.frame();
    }

    /// The first lane's node, straight out of the render tree.
    fn lajur_pertama(ui: &AppRuntime) -> (NodeId, f32, f32) {
        let mut daftar = Vec::new();
        semua(ui.tree(), ui.tree().root(), &mut daftar);
        for id in daftar {
            if let Some(l) = ui.tree().node_ref::<LintasanSpring>(id) {
                return (id, l.posisi(), l.target());
            }
        }
        panic!("tidak ada lajur spring di pohon");
    }

    #[test]
    fn spring_preset_berbeda_per_lajur() {
        assert_eq!(Jenis::Smooth.spring(0.5, 0.2), Spring::smooth());
        assert_eq!(Jenis::Snappy.spring(0.5, 0.2), Spring::snappy());
        assert_eq!(Jenis::Bouncy.spring(0.5, 0.2), Spring::bouncy());
        // Only the fourth lane listens to the sliders.
        assert_eq!(Jenis::Sendiri.spring(0.8, 0.4), Spring::new(0.8, 0.4));
    }

    #[test]
    fn ringkasan_diturunkan_dari_spring_bukan_diketik_ulang() {
        let s = Spring::new(0.5, 0.0);
        let teks = ringkasan(s);
        assert!(teks.contains("durasi 0.50"), "{teks}");
        assert!(
            teks.contains(&format!("{:.0}", s.stiffness())),
            "kekakuan tidak ikut nilai spring: {teks}"
        );
        // Bounce 0 means critically damped: ratio 1, no overshoot.
        assert!(!s.overshoots());
    }

    #[test]
    fn halaman_menggambar_keempat_lajur() {
        let f = fonts();
        let ui = ui(tema(), &f);
        for l in LAJUR {
            assert!(
                ui.access_tree().find_label(&nama_lajur(l)).is_some(),
                "lajur '{}' tidak ada di pohon a11y",
                l.nama
            );
        }
        assert!(ui
            .scene()
            .commands()
            .iter()
            .any(|c| matches!(c, Command::Quad(_) | Command::Shadow(_))));
    }

    #[test]
    fn klik_di_lajur_memindahkan_target_semua_lajur() {
        let f = fonts();
        let mut ui = ui(tema(), &f);
        let lajur = kotak(&ui, &nama_lajur(LAJUR[0]));
        // Far right — the target must follow the finger, not a fixed step.
        ketuk(
            &mut ui,
            Point::new(lajur.origin.x + lajur.size.width - 4.0, lajur.center().y),
        );

        let (_, _, target) = lajur_pertama(&ui);
        assert!(
            target > 0.9,
            "target seharusnya di ujung kanan, bukan {target}"
        );
    }

    #[test]
    fn puck_bergerak_lalu_berhenti_sendiri() {
        let f = fonts();
        let mut ui = ui(tema(), &f);
        let lajur = kotak(&ui, &nama_lajur(LAJUR[0]));
        ketuk(&mut ui, Point::new(lajur.center().x, lajur.center().y));

        // A simulated clock, never `Instant::now()`: back-to-back frames in a
        // test are microseconds apart, and a spring asked to advance by zero
        // seconds does exactly what it is told (§9.5). The very first frame of
        // any burst is the one that *starts* the clock, so it carries `dt = 0`
        // by design — the movement begins on the second one.
        let mut jam = std::time::Instant::now();
        jam += FRAME;
        let _ = ui.animate_at(jam, advance);
        ui.frame();
        let (_, awal, _) = lajur_pertama(&ui);

        jam += FRAME;
        let _ = ui.animate_at(jam, advance);
        ui.frame();
        let (_, setelah_satu_frame, _) = lajur_pertama(&ui);
        assert!(
            setelah_satu_frame > awal,
            "puck tidak bergerak sama sekali ({awal} → {setelah_satu_frame})"
        );

        // …and the rest of the frames prove it stops on its own, which is what
        // lets the GPU sleep (§3.5).
        for _ in 0..600 {
            jam += FRAME;
            let dirty = ui.animate_at(jam, advance);
            ui.frame();
            if dirty.is_empty() {
                let (_, posisi, target) = lajur_pertama(&ui);
                assert!(
                    (posisi - target).abs() < 0.01,
                    "berhenti jauh dari target: {posisi} vs {target}"
                );
                return;
            }
        }
        panic!("spring tidak pernah settle");
    }

    #[test]
    fn target_baru_tidak_mengulang_dari_nol() {
        // Retargeting mid-flight is the page's whole claim: after a first
        // frame the puck has left the start, and a new target must continue
        // from where it is rather than teleport back.
        let mut n = SpringValue::new(0.0f32).with_spring(Spring::smooth());
        n.set_target(1.0);
        n.advance(Duration::from_millis(60), Motion::Full);
        let tengah = n.position();
        assert!(tengah > 0.0 && tengah < 1.0, "posisi antara: {tengah}");

        n.set_target(0.0);
        assert_eq!(n.position(), tengah, "retarget tidak boleh memindah posisi");
        assert!(n.velocity() > 0.0, "kecepatan harus terbawa, bukan dibuang");
    }

    #[test]
    fn gerak_dikurangi_menghapus_pantulan() {
        let bouncy = Spring::bouncy();
        assert!(bouncy.overshoots(), "spring bouncy harus melewati target");
        assert!(
            !Motion::Reduced.spring(bouncy).overshoots(),
            "dengan reduce motion, spring tidak boleh memantul lagi"
        );
    }

    #[test]
    fn fraksi_mengikuti_titik_di_bawah_kursor() {
        let t = tema();
        let gaya = GayaLintasan::from_theme(&t);
        let n = LintasanSpring {
            nilai: SpringValue::new(0.0),
            warna: t.color.accent,
            gaya,
            label: None,
            ukuran: Size::new(400.0, gaya.tinggi),
            on_target: None,
            menyeret: false,
        };
        let (kiri, kanan) = n.rentang();
        assert_eq!(n.fraksi_dari(Point::new(kiri + gaya.puck * 0.5, 0.0)), 0.0);
        assert_eq!(n.fraksi_dari(Point::new(kanan + gaya.puck * 0.5, 0.0)), 1.0);
        // Outside the band it clamps instead of running off the rail.
        assert_eq!(n.fraksi_dari(Point::new(-999.0, 0.0)), 0.0);
        assert_eq!(n.fraksi_dari(Point::new(9_999.0, 0.0)), 1.0);
    }

    #[test]
    fn benar_di_kedua_preset() {
        let f = fonts();
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let ui = ui(t, &f);
                assert_eq!(ui.scene().clear_color(), t.color.background);
                // Every colour on the page is a token, so the lane colours
                // must differ between the two presets exactly as the tokens do.
                assert_eq!(Jenis::Smooth.warna(&t), t.color.accent);
            }
        }
    }
}
