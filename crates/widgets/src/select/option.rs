//! Satu baris pilihan di dalam popup select.
//!
//! Baris **tidak** ikut navigasi fokus: fokus tetap di pemicu (lihat
//! [`super::trigger`]), persis seperti menu native. Yang dilakukan baris hanya
//! tiga hal — melapor sorotan saat penunjuk lewat, melapor pilihan saat diklik,
//! dan mengumumkan dirinya ke teknologi bantu sebagai item menu yang
//! bertanda/tidak.

use silka_core::access::{AccessActions, AccessNode, AccessRole, AccessToggled};
use silka_core::animation::{Spring, SpringValue, Tick};
use silka_core::input::{
    CursorIcon, Event, EventCtx, HitBehavior, HitShape, PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, LayoutCtx, PaintCtx, RenderNode};
use silka_core::view::ViewNode;
use silka_paint::{Color, Corners, Insets, Point, Quad, Rect, Size};

use super::{SelectHandler, SelectIntent};

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Nilai gambar satu baris pilihan, **sudah diresolusi** dari token theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectOptionStyle {
    /// Latar keadaan diam (biasanya transparan).
    pub rest: Color,
    /// Latar saat disorot (keyboard atau penunjuk).
    pub highlight: Color,
    /// Latar baris yang sedang terpilih.
    pub selected: Color,
    /// Geometri sudut baris.
    pub corners: Corners,
    /// Jarak isi ke tepi baris.
    pub padding: Insets,
    /// Warna penanda "terpilih".
    pub marker: Color,
    /// Ukuran penanda "terpilih".
    pub marker_size: f32,
    /// Tinggi minimum baris — hit target HIG.
    pub min_height: f32,
}

impl SelectOptionStyle {
    /// Latar yang seharusnya berlaku — **target** spring, bukan yang digambar.
    pub fn background_for(&self, highlighted: bool, selected: bool) -> Color {
        if highlighted {
            self.highlight
        } else if selected {
            self.selected
        } else {
            self.rest
        }
    }

    /// Jarak isi ke tepi, sudah menyediakan ruang penanda di akhir baris (§9.8).
    pub fn insets(&self, rtl: bool) -> Insets {
        let ruang = self.marker_size * 2.0;
        let mut i = self.padding;
        if rtl {
            i.left += ruang;
        } else {
            i.right += ruang;
        }
        i
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// Node render satu baris pilihan.
pub struct SelectOption {
    style: SelectOptionStyle,
    index: usize,
    label: Option<String>,
    selected: bool,
    highlighted: bool,
    on_intent: Option<SelectHandler>,

    bg: SpringValue<Color>,
    hovered: bool,
    pressed: bool,
    rtl: bool,
}

impl SelectOption {
    fn new(props: &SelectOptionProps) -> Self {
        Self {
            bg: SpringValue::new(
                props
                    .style
                    .background_for(props.highlighted, props.selected),
            )
            .with_spring(props.spring),
            style: props.style,
            index: props.index,
            label: props.label.clone(),
            selected: props.selected,
            highlighted: props.highlighted,
            on_intent: props.on_intent.clone(),
            hovered: false,
            pressed: false,
            rtl: false,
        }
    }

    /// Indeks baris ini di dalam daftar.
    pub fn index(&self) -> usize {
        self.index
    }

    /// Latar yang digambar frame ini — posisi spring, bukan targetnya.
    pub fn background(&self) -> Color {
        self.bg.position()
    }

    /// Target latar yang sedang dituju spring.
    pub fn background_target(&self) -> Color {
        self.bg.target()
    }

    /// Sedang terpilih.
    pub fn is_selected(&self) -> bool {
        self.selected
    }

    /// Sedang disorot.
    pub fn is_highlighted(&self) -> bool {
        self.highlighted
    }

    /// Benar bila spring latarnya masih bergerak.
    pub fn is_animating(&self) -> bool {
        self.bg.is_animating()
    }

    /// Majukan spring satu frame; benar bila warnanya bergeser.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        let sebelum = self.bg.position();
        tick.advance(&mut self.bg);
        self.bg.position() != sebelum
    }

    /// Selesaikan gerakan seketika.
    pub fn settle(&mut self) {
        self.bg.settle();
    }

    fn retarget(&mut self) {
        self.bg
            .set_target(self.style.background_for(self.highlighted, self.selected));
    }

    fn kirim(&mut self, intent: SelectIntent) {
        if let Some(h) = self.on_intent.clone() {
            h.emit(intent);
        }
    }

    /// Kotak penanda "terpilih" dalam koordinat lokal.
    pub fn marker_rect(&self, bounds: Rect) -> Rect {
        let d = self.style.marker_size.max(0.0);
        let x = if self.rtl {
            self.style.padding.left
        } else {
            bounds.size.width - self.style.padding.right - d
        };
        Rect::new(x, bounds.center().y - d / 2.0, d, d)
    }
}

impl RenderNode for SelectOption {
    fn type_name(&self) -> &'static str {
        "SelectOption"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.rtl = ctx.direction().is_rtl();
        let insets = self.style.insets(self.rtl);
        if ctx.child_count() == 0 {
            return constraints.constrain(Size::new(constraints.min_width, self.style.min_height));
        }
        let child = ctx.child(0);
        let isi = ctx.layout_child(child, constraints.deflate(insets).loosen());
        // Baris mengisi lebar panel (daftar memberi lebar tight); kalau tidak
        // ada batas, ia jatuh ke lebar isinya sendiri.
        let lebar = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            isi.width + insets.horizontal()
        };
        let size = constraints.constrain(Size::new(
            lebar,
            (isi.height + insets.vertical()).max(self.style.min_height),
        ));
        let x = if self.rtl {
            (size.width - insets.right - isi.width).max(insets.left)
        } else {
            insets.left
        };
        let y = ((size.height - isi.height) / 2.0).max(0.0);
        ctx.place_child(child, Point::new(x, y));
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.local_bounds();
        let bg = self.bg.position();
        if bg.a > 0.0 {
            ctx.quad(Quad::new(bounds).background(bg).corners(self.style.corners));
        }
        ctx.paint_children();

        // Penanda "terpilih": titik berbentuk sudut preset yang sama, jadi
        // squircle Cupertino dan arc Tailwind tetap sejalan (§2.7).
        if self.selected && self.style.marker.a > 0.0 {
            let kotak = self.marker_rect(bounds);
            ctx.quad(
                Quad::new(kotak)
                    .background(self.style.marker)
                    .corners(Corners::uniform(
                        kotak.size.width / 2.0,
                        self.style.corners.style,
                    )),
            );
        }
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::MenuItem;
        node.label.clone_from(&self.label);
        node.toggled = Some(AccessToggled::from(self.selected));
        node.actions |= AccessActions::CLICK;
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.style.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    fn cursor(&self) -> Option<CursorIcon> {
        Some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else { return };
        let sebelum = (self.hovered, self.pressed, self.highlighted);
        match p.phase {
            PointerPhase::Enter | PointerPhase::Move => {
                self.hovered = true;
                if !self.highlighted {
                    // Sorotan ditulis di tempat **dan** dilaporkan: tanpa itu,
                    // gerakan penunjuk berikutnya sebelum frame berikutnya akan
                    // melaporkan hal yang sama sekali lagi.
                    self.highlighted = true;
                    self.retarget();
                    self.kirim(SelectIntent::Highlight(self.index));
                }
            }
            PointerPhase::Leave => self.hovered = false,
            PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                self.pressed = true;
                ctx.capture_pointer();
                ctx.handled();
            }
            PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                let di_dalam = self.style.corners.contains(ctx.size(), ctx.local());
                let aktif = self.pressed && di_dalam;
                self.pressed = false;
                ctx.release_pointer();
                ctx.handled();
                if aktif {
                    self.kirim(SelectIntent::Commit(self.index));
                }
            }
            PointerPhase::Cancel if self.pressed => self.pressed = false,
            _ => {}
        }
        if (self.hovered, self.pressed, self.highlighted) != sebelum {
            ctx.request_paint();
            ctx.request_animation();
        }
    }
}

impl core::fmt::Debug for SelectOption {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SelectOption")
            .field("index", &self.index)
            .field("label", &self.label)
            .field("selected", &self.selected)
            .field("highlighted", &self.highlighted)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props satu baris pilihan — bentuk view dari [`SelectOption`].
#[derive(Debug, Clone, PartialEq)]
pub struct SelectOptionProps {
    /// Nilai gambar, sudah diresolusi dari token.
    pub style: SelectOptionStyle,
    /// Indeks baris ini.
    pub index: usize,
    /// Nama yang dibacakan screen reader.
    pub label: Option<String>,
    /// Sedang terpilih.
    pub selected: bool,
    /// Sedang disorot.
    pub highlighted: bool,
    /// Spring yang menjalankan transisi latar.
    pub spring: Spring,
    /// Ke mana niat pengguna dikirim.
    pub on_intent: Option<SelectHandler>,
}

impl ViewNode for SelectOptionProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(SelectOption::new(self))
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<SelectOption>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        let berubah = n.style != self.style
            || n.selected != self.selected
            || n.highlighted != self.highlighted;
        n.style = self.style;
        n.selected = self.selected;
        n.highlighted = self.highlighted;
        if berubah {
            n.retarget();
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.index != self.index {
            n.index = self.index;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        if n.bg.spring() != self.spring {
            n.bg.set_spring(self.spring);
        }
        n.on_intent.clone_from(&self.on_intent);
        dirty
    }
}
