//! Unit tests for the §2.6 utility vocabulary.
//!
//! What is being proven here is not "the setter sets": it is the two contracts
//! the vocabulary exists for.
//!
//! 1. **One call, two geometries** — `rounded_lg()` is a 14pt squircle under
//!    Cupertino and an 8pt arc under Tailwind, with the call site unchanged
//!    (§2.7).
//! 2. **The 4pt scale** — `p_4()` is four *steps*, i.e. 16pt, and follows the
//!    scale when a brand preset changes the unit.
//!
//! Everything is read back through a real diff into the render tree, not from
//! the props: what the node ends up holding is what the paint pass and
//! hit-testing will use.

use silka_paint::{Color, CornerStyle, Insets, Size};
use silka_text::{FontWeight, TextStyle};
use silka_theme::{Appearance, ColorToken, Preset, RadiusToken, SpaceToken, SpacingTokens, Theme};

use crate::scheduler::Dirty;
use crate::tree::{
    Axis, BoxConstraints, ContainerStyle, CrossAlign, Decoration, FlexWrap, Interactive,
    LayoutItem, LayoutMode, MainAlign, PaddingBox, RenderNode, RenderTree, TaffyBox,
};

use super::{
    active_theme, container, div, expanded, fixed, interactive, item, pad, reconcile, with_theme,
    Builder, TextStyled, View, ViewNode,
};

// ---------------------------------------------------------------------------
// Helpers: build a view, diff it, read the node back
// ---------------------------------------------------------------------------

fn dengan_node<N: RenderNode, R>(view: impl Into<View>, f: impl FnOnce(&N) -> R) -> R {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, view);
    let root = tree.root();
    let id = tree.children(root)[0];
    let node = tree
        .render(id)
        .expect("node akar ada setelah diff")
        .downcast_ref::<N>()
        .expect("tipe render node sesuai dengan view-nya");
    f(node)
}

fn dekorasi(view: impl Into<View>) -> Decoration {
    dengan_node::<TaffyBox, _>(view, |n| n.decoration)
}

fn padding(view: impl Into<View>) -> Insets {
    dengan_node::<TaffyBox, _>(view, |n| n.style.padding)
}

fn gaya(view: impl Into<View>) -> ContainerStyle {
    dengan_node::<TaffyBox, _>(view, |n| n.style.clone())
}

fn kedua_preset<T: PartialEq + core::fmt::Debug>(f: impl Fn() -> T) -> (T, T) {
    let a = with_theme(Theme::new(Preset::Cupertino, Appearance::Light), &f);
    let b = with_theme(Theme::new(Preset::Tailwind, Appearance::Light), &f);
    (a, b)
}

// ---------------------------------------------------------------------------
// The ambient theme
// ---------------------------------------------------------------------------

#[test]
fn tanpa_with_theme_tetap_ada_tema_bawaan() {
    // A utility must never panic just because nobody installed a theme: the
    // default is Cupertino/light, exactly what `Theme::default()` says.
    assert_eq!(active_theme().preset, Preset::Cupertino);
    assert_eq!(active_theme().appearance, Appearance::Light);
}

#[test]
fn with_theme_bersarang_dan_memulihkan() {
    let luar = Theme::tailwind(Appearance::Dark);
    with_theme(luar, || {
        assert_eq!(active_theme().preset, Preset::Tailwind);
        with_theme(Theme::cupertino(Appearance::Light), || {
            assert_eq!(active_theme().preset, Preset::Cupertino);
        });
        // The inner block gave the theme back.
        assert_eq!(active_theme().appearance, Appearance::Dark);
    });
    assert_eq!(active_theme().preset, Theme::default().preset);
}

#[test]
fn with_theme_memulihkan_meski_panic() {
    let hasil = std::panic::catch_unwind(|| {
        with_theme(Theme::tailwind(Appearance::Dark), || panic!("sengaja"));
    });
    assert!(hasil.is_err());
    assert_eq!(active_theme().preset, Theme::default().preset);
}

// ---------------------------------------------------------------------------
// Radius: one call, two geometries (§2.7)
// ---------------------------------------------------------------------------

#[test]
fn rounded_lg_menghasilkan_geometri_berbeda_di_dua_preset() {
    let (cupertino, tailwind) = kedua_preset(|| dekorasi(div().rounded_lg()).corners);

    // Different radius…
    assert_eq!(cupertino.radii.top_left, 14.0);
    assert_eq!(tailwind.radii.top_left, 8.0);
    // …and, more importantly, a different *shape*: the squircle is a shader
    // parameter, not a constant (§3.6).
    assert_eq!(cupertino.style, CornerStyle::squircle());
    assert_eq!(tailwind.style, CornerStyle::Arc);
    assert_ne!(cupertino, tailwind);
}

#[test]
fn setiap_token_radius_berbeda_antar_preset_kecuali_none_dan_full() {
    for token in [
        RadiusToken::Sm,
        RadiusToken::Md,
        RadiusToken::Lg,
        RadiusToken::Xl,
    ] {
        let (a, b) = kedua_preset(|| dekorasi(div().rounded(token)).corners);
        assert_ne!(a, b, "{}", token.name());
    }
    // `none` and `full` are the two ends of the scale: both presets agree on
    // the number, and only the shape differs.
    let (a, b) = kedua_preset(|| dekorasi(div().rounded_none()).corners);
    assert_eq!(a.radii.max(), 0.0);
    assert_eq!(b.radii.max(), 0.0);
    let (a, b) = kedua_preset(|| dekorasi(div().rounded_full()).corners);
    assert_eq!(a.radii.max(), b.radii.max());
}

#[test]
fn rounded_sama_dengan_resolve_theme() {
    let t = Theme::tailwind(Appearance::Dark);
    let dari_utility = with_theme(t, || dekorasi(div().rounded_md()).corners);
    assert_eq!(dari_utility, t.corners_of(RadiusToken::Md));
}

// ---------------------------------------------------------------------------
// Spacing: the 4pt scale
// ---------------------------------------------------------------------------

#[test]
fn p_4_adalah_16pt_di_kedua_preset() {
    let (cupertino, tailwind) = kedua_preset(|| padding(div().p_4()));
    assert_eq!(cupertino, Insets::all(16.0));
    assert_eq!(tailwind, Insets::all(16.0));
}

#[test]
fn skala_spasi_adalah_kelipatan_4pt() {
    for (dapat, harap) in [
        (padding(div().p_0()), 0.0),
        (padding(div().p_1()), 4.0),
        (padding(div().p_2()), 8.0),
        (padding(div().p_3()), 12.0),
        (padding(div().p_4()), 16.0),
        (padding(div().p_6()), 24.0),
        (padding(div().p_12()), 48.0),
        (padding(div().p_24()), 96.0),
    ] {
        assert_eq!(dapat, Insets::all(harap));
    }
    // The hairline is the one value deliberately off the scale.
    assert_eq!(padding(div().p_px()), Insets::all(1.0));
}

#[test]
fn px_py_dan_sisi_tunggal_hanya_menyentuh_tepinya_sendiri() {
    assert_eq!(
        padding(div().px_4()),
        Insets {
            top: 0.0,
            right: 16.0,
            bottom: 0.0,
            left: 16.0
        }
    );
    assert_eq!(
        padding(div().py_2()),
        Insets {
            top: 8.0,
            right: 0.0,
            bottom: 8.0,
            left: 0.0
        }
    );
    assert_eq!(
        padding(div().pt_1().pr_2().pb_3().pl_4()),
        Insets {
            top: 4.0,
            right: 8.0,
            bottom: 12.0,
            left: 16.0
        }
    );
}

#[test]
fn urutan_rantai_menang_seperti_tailwind() {
    // `.p_4().px_2()` = 16pt vertically, 8pt horizontally — the later utility
    // wins on the axis it names, exactly as a Tailwind class list behaves.
    assert_eq!(
        padding(div().p_4().px_2()),
        Insets {
            top: 16.0,
            right: 8.0,
            bottom: 16.0,
            left: 8.0
        }
    );
}

#[test]
fn skala_ikut_unit_preset_brand() {
    // A custom brand preset that doubles the unit moves every spacing utility
    // at once — that is the whole point of resolving through a token.
    let brand = Theme::cupertino(Appearance::Light).with_spacing(SpacingTokens { unit: 8.0 });
    let dapat = with_theme(brand, || padding(div().p_4()));
    assert_eq!(dapat, Insets::all(32.0));
    // …and the hairline stays a hairline.
    assert_eq!(
        with_theme(brand, || padding(div().p_px())),
        Insets::all(1.0)
    );
}

#[test]
fn gap_ikut_skala_yang_sama_dengan_padding() {
    let brand = Theme::tailwind(Appearance::Dark).with_spacing(SpacingTokens { unit: 8.0 });
    let s = with_theme(brand, || gaya(div().gap_3()));
    assert_eq!(s.gap_x, 24.0);
    assert_eq!(s.gap_y, 24.0);
    // The token form agrees with the shorthand.
    let lain = with_theme(brand, || gaya(div().gap_token(SpaceToken::S3)));
    assert_eq!(s.gap_x, lain.gap_x);
}

#[test]
fn p_juga_berlaku_untuk_pad() {
    // `Padded` is a trait, so the vocabulary is written once and works on every
    // view that has insets — here the dedicated `pad()` node.
    let insets =
        dengan_node::<PaddingBox, _>(pad(Insets::ZERO, fixed(10.0, 10.0)).p_4(), |n| n.insets);
    assert_eq!(insets, Insets::all(16.0));
}

#[test]
fn margin_hanya_untuk_item_flex() {
    let margin =
        dengan_node::<LayoutItem, _>(item(fixed(10.0, 10.0)).mx_2().mt_4(), |n| n.style.margin);
    assert_eq!(
        margin,
        Insets {
            top: 16.0,
            right: 8.0,
            bottom: 0.0,
            left: 8.0
        }
    );
}

// ---------------------------------------------------------------------------
// Color, border, shadow
// ---------------------------------------------------------------------------

#[test]
fn bg_memakai_warna_token_bukan_angka() {
    for preset in Preset::ALL {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            let dapat = with_theme(t, || dekorasi(div().bg(ColorToken::Surface)).background);
            assert_eq!(dapat, t.color.surface, "{preset:?}/{appearance:?}");
        }
    }
}

#[test]
fn token_yang_sama_berubah_saat_dark_mode() {
    let terang = Theme::cupertino(Appearance::Light);
    let a = with_theme(terang, || {
        dekorasi(div().bg(ColorToken::Background)).background
    });
    let b = with_theme(terang.with_appearance(Appearance::Dark), || {
        dekorasi(div().bg(ColorToken::Background)).background
    });
    assert_ne!(a, b);
}

#[test]
fn escape_hatch_melewatkan_warna_apa_adanya() {
    let ungu = Color::hex(0x7C3AED);
    assert_eq!(dekorasi(div().bg_raw(ungu)).background, ungu);
    assert_eq!(dekorasi(div().border_color_raw(ungu)).border_color, ungu);
}

#[test]
fn lebar_border_terikat_skala() {
    assert_eq!(dekorasi(div().border_1()).border_width, 1.0);
    assert_eq!(dekorasi(div().border_2()).border_width, 2.0);
    assert_eq!(dekorasi(div().border_4()).border_width, 4.0);
    assert_eq!(dekorasi(div().border_2().border_0()).border_width, 0.0);
    // Width and color are set independently, GPUI-style.
    let d = dekorasi(div().border_1().border_color(ColorToken::Separator));
    assert_eq!(d.border_width, 1.0);
    assert_eq!(d.border_color, Theme::default().color.separator);
}

#[test]
fn shadow_md_adalah_pasangan_ambient_plus_key_dari_preset() {
    for preset in Preset::ALL {
        let t = Theme::new(preset, Appearance::Light);
        let dapat = with_theme(t, || dekorasi(div().shadow_md()).shadows);
        assert_eq!(dapat, t.shadow.md, "{preset:?}");
        assert!(dapat.is_visible());
        assert!(dapat.ambient.blur > dapat.key.blur, "{preset:?}");
    }
    assert!(!dekorasi(div().shadow_md().shadow_none())
        .shadows
        .is_visible());
}

#[test]
fn elevasi_naik_berarti_bayangan_melebar() {
    let sm = dekorasi(div().shadow_sm()).shadows;
    let md = dekorasi(div().shadow_md()).shadows;
    let lg = dekorasi(div().shadow_lg()).shadows;
    assert!(sm.ambient.blur < md.ambient.blur);
    assert!(md.ambient.blur < lg.ambient.blur);
}

// ---------------------------------------------------------------------------
// Layout vocabulary
// ---------------------------------------------------------------------------

#[test]
fn div_bawaannya_menumpuk_ke_bawah_dan_melebar() {
    let s = gaya(div());
    assert_eq!(s.mode, LayoutMode::Flex);
    assert_eq!(s.axis, Axis::Vertical);
    assert_eq!(s.cross, CrossAlign::Stretch);
    // `container()` is the same node under a name that is not from the web.
    assert_eq!(gaya(container()), s);
}

#[test]
fn flex_dan_penjajaran_terpetakan_ke_kosakata_container() {
    let s = gaya(div().flex().items_center().justify_between());
    assert_eq!(s.axis, Axis::Horizontal);
    assert_eq!(s.cross, CrossAlign::Center);
    assert_eq!(s.main, MainAlign::SpaceBetween);

    let s = gaya(div().flex_col().items_end().justify_evenly());
    assert_eq!(s.axis, Axis::Vertical);
    assert_eq!(s.cross, CrossAlign::End);
    assert_eq!(s.main, MainAlign::SpaceEvenly);

    assert_eq!(gaya(div().flex().wrap()).wrap, FlexWrap::Wrap);
    assert_eq!(gaya(div().flex().wrap().nowrap()).wrap, FlexWrap::NoWrap);
}

#[test]
fn flex_1_sama_dengan_expanded() {
    let a = dengan_node::<LayoutItem, _>(item(fixed(10.0, 10.0)).flex_1(), |n| n.style);
    let b = dengan_node::<LayoutItem, _>(expanded(fixed(10.0, 10.0)), |n| n.style);
    assert_eq!(a, b);
}

#[test]
fn rantai_utility_lengkap_ikut_berlayout() {
    // The §2.6 example, end to end: build → diff → layout, with the numbers
    // coming out where the tokens said they would.
    let mut tree = RenderTree::new();
    with_theme(Theme::cupertino(Appearance::Light), || {
        reconcile(
            &mut tree,
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .py_2()
                .rounded_lg()
                .bg(ColorToken::Surface)
                .shadow_md()
                .child(fixed(60.0, 20.0).label("Save"))
                .child(fixed(60.0, 20.0).label("Cancel")),
        );
    });
    tree.perform_layout(BoxConstraints::loose(Size::new(400.0, 200.0)));
    let root = tree.root();
    let id = tree.children(root)[0];
    let size = tree.size(id);
    // 60 + 12 (gap_3) + 60 + 2 × 16 (px_4) = 164 wide, 20 + 2 × 8 (py_2) = 36 tall.
    assert_eq!(size.width, 164.0);
    assert_eq!(size.height, 36.0);
}

#[test]
fn membungkus_satu_frame_sudah_cukup_untuk_seluruh_pohon() {
    // The integration the shell has to perform, in full: wrap the frame. Every
    // component rebuild happens synchronously inside it, so no constructor and
    // no `BuildCtx` has to carry a theme argument (§2.5).
    let mut ui = crate::app::app(|_cx| {
        View::from(
            div()
                .bg(ColorToken::Accent)
                .child(crate::app::component("isi", |_| {
                    View::from(div().bg(ColorToken::Surface).rounded_lg())
                })),
        )
    })
    .sized(200.0, 100.0);

    let t = Theme::tailwind(Appearance::Dark);
    with_theme(t, || ui.frame());

    fn telusuri(tree: &RenderTree, id: crate::tree::NodeId, t: &Theme, n: &mut usize) {
        if let Some(node) = tree.render(id).and_then(|x| x.downcast_ref::<TaffyBox>()) {
            if node.decoration.background == t.color.surface {
                assert_eq!(node.decoration.corners, t.corners_of(RadiusToken::Lg));
                *n += 1;
            }
        }
        for anak in tree.children(id) {
            telusuri(tree, *anak, t, n);
        }
    }

    let mut ditemukan = 0;
    let akar = ui.tree().root();
    telusuri(ui.tree(), akar, &t, &mut ditemukan);
    assert_eq!(ditemukan, 1, "komponen anak ikut memakai tema aktif");
}

// ---------------------------------------------------------------------------
// Interactive
// ---------------------------------------------------------------------------

#[test]
fn interactive_menerima_token_untuk_setiap_keadaan() {
    let t = Theme::tailwind(Appearance::Light);
    let mut tree = RenderTree::new();
    with_theme(t, || {
        reconcile(
            &mut tree,
            interactive(fixed(80.0, 32.0))
                .label("Save")
                .bg(ColorToken::Accent)
                .hover_bg(ColorToken::AccentHover)
                .press_bg(ColorToken::AccentPressed)
                .rounded_md(),
        );
    });
    let root = tree.root();
    let id = tree.children(root)[0];
    let node = tree
        .render(id)
        .expect("node akar ada setelah diff")
        .downcast_ref::<Interactive>()
        .expect("interactive() adalah Interactive");
    assert_eq!(node.decoration.background, t.color.accent);
    assert_eq!(node.hover.background, Some(t.color.accent_hover));
    assert_eq!(node.press.background, Some(t.color.accent_pressed));
    // Corners feed hit-testing as well as the shader, so the touch area is a
    // squircle exactly when the box is (§3.6).
    assert_eq!(node.corners, t.corners_of(RadiusToken::Md));
}

// ---------------------------------------------------------------------------
// Typography — proven through a stand-in implementor of `TextStyled`
// ---------------------------------------------------------------------------

/// A minimal props type that carries a text style, standing in for the text
/// leaf that lives in `silka-widgets` (which needs the font stack, and so
/// cannot live here).
#[derive(Debug, Clone, PartialEq)]
struct PropsTeks {
    style: TextStyle,
    color: Color,
}

impl Default for PropsTeks {
    fn default() -> Self {
        Self {
            style: TextStyle::new(),
            color: Color::TRANSPARENT,
        }
    }
}

impl TextStyled for PropsTeks {
    fn text_style_mut(&mut self) -> &mut TextStyle {
        &mut self.style
    }

    fn text_color_mut(&mut self) -> &mut Color {
        &mut self.color
    }
}

impl ViewNode for PropsTeks {
    fn build(&self) -> Box<dyn RenderNode> {
        unreachable!("props uji tidak pernah didiff")
    }

    fn update(&self, _node: &mut dyn RenderNode) -> Dirty {
        unreachable!("props uji tidak pernah didiff")
    }
}

fn gaya_teks(f: impl FnOnce(Builder<PropsTeks>) -> Builder<PropsTeks>) -> PropsTeks {
    let mut hasil = PropsTeks::default();
    f(Builder::new(PropsTeks::default())).map(|p| hasil = p.clone());
    hasil
}

#[test]
fn skala_tipografi_datang_dari_preset() {
    for preset in Preset::ALL {
        let t = Theme::new(preset, Appearance::Light);
        let p = with_theme(t, || gaya_teks(|b| b.text_base()));
        assert_eq!(p.style.size, t.typography.body.size, "{preset:?}");
        assert_eq!(
            p.style.line_height, t.typography.body.line_height,
            "{preset:?}"
        );
    }
    // Larger role = larger text, in both presets.
    let (a, b) = kedua_preset(|| {
        let kecil = gaya_teks(|x| x.text_sm()).style.size;
        let besar = gaya_teks(|x| x.text_2xl()).style.size;
        besar > kecil
    });
    assert!(a && b);
}

#[test]
fn bobot_dan_warna_teks_memakai_token() {
    let t = Theme::cupertino(Appearance::Dark);
    let p = with_theme(t, || {
        gaya_teks(|b| {
            b.text_base()
                .font_semibold()
                .text_color(ColorToken::SecondaryLabel)
        })
    });
    assert_eq!(p.style.weight, FontWeight(600));
    assert_eq!(p.color, t.color.secondary_label);

    // The weight utility comes after the role, so it wins — chain order again.
    let p = with_theme(t, || gaya_teks(|b| b.font_bold().text_base()));
    assert_eq!(p.style.weight, FontWeight(t.typography.body.weight));
}
