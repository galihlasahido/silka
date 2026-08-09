//! Unit test pass emisi a11y.
//!
//! Alat verifikasi utamanya adalah **tree dump**: satu string yang memuat
//! peran, nama, kotak hasil layout, aksi, dan fokus sekaligus. Kalau ada yang
//! bergeser — node hilang, kotak salah, aksi lupa diumumkan — dump-nya berubah
//! dan test gagal dengan diff yang bisa dibaca manusia.
//!
//! Wadah yang dipakai di sini adalah [`Stack`] milik test ini sendiri, bukan
//! `column`/`row` bawaan: yang sedang diuji adalah **pass a11y**, dan test-nya
//! tidak boleh ikut gagal setiap kali mesin flex di bawahnya disetel ulang.

use rustui_paint::{Insets, Point, Size};

use crate::scheduler::Dirty;
use crate::tree::{
    AccessActions, AccessNode, AccessRole, AccessToggled, Axis, BoxConstraints, LayoutCtx,
    RenderNode, RenderTree,
};
use crate::view::{fixed, pad, reconcile, viewport, Builder, View, ViewNode};

use super::{AccessAction, AccessEntry};

// ---------------------------------------------------------------------------
// Bahan uji
// ---------------------------------------------------------------------------

/// Wadah sederhana: menumpuk anak pada satu sumbu, tanpa jarak.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Stack {
    axis: Axis,
}

impl RenderNode for Stack {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let anak = ctx.child_count();
        let longgar = match self.axis {
            Axis::Vertical => BoxConstraints::new(0.0, constraints.max_width, 0.0, f32::INFINITY),
            Axis::Horizontal => {
                BoxConstraints::new(0.0, f32::INFINITY, 0.0, constraints.max_height)
            }
        };
        let mut utama = 0.0f32;
        let mut silang = 0.0f32;
        let mut ukuran = Vec::with_capacity(anak);
        for i in 0..anak {
            let s = ctx.layout_child(ctx.child(i), longgar);
            utama += self.axis.main_of(s);
            silang = silang.max(self.axis.cross_of(s));
            ukuran.push(s);
        }
        let mut posisi = 0.0f32;
        for (i, s) in ukuran.iter().copied().enumerate() {
            let offset = match self.axis {
                Axis::Vertical => Point::new(0.0, posisi),
                Axis::Horizontal => Point::new(posisi, 0.0),
            };
            ctx.place_child(ctx.child(i), offset);
            posisi += self.axis.main_of(s);
        }
        constraints.constrain(self.axis.size_of(utama, silang))
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Group;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct StackProps {
    axis: Axis,
}

impl ViewNode for StackProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(Stack { axis: self.axis })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node.downcast_mut::<Stack>().expect("tipe view sama");
        if n.axis != self.axis {
            n.axis = self.axis;
            return Dirty::LAYOUT | Dirty::PAINT;
        }
        Dirty::NONE
    }
}

fn tumpuk<C: Into<View>>(axis: Axis, children: impl IntoIterator<Item = C>) -> Builder<StackProps> {
    Builder::new(StackProps { axis }).children(children)
}

fn kolom<C: Into<View>>(children: impl IntoIterator<Item = C>) -> Builder<StackProps> {
    tumpuk(Axis::Vertical, children)
}

fn baris<C: Into<View>>(children: impl IntoIterator<Item = C>) -> Builder<StackProps> {
    tumpuk(Axis::Horizontal, children)
}

/// Daun yang berperilaku seperti kontrol sungguhan: punya nama, nilai, aksi,
/// dan keadaan. Mewakili apa yang harus diisi setiap widget di `KOMPONEN.md`.
#[derive(Debug, Clone, PartialEq)]
struct Control {
    size: Size,
    node: AccessNode,
}

impl RenderNode for Control {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        constraints.constrain(self.size)
    }

    fn access(&self, node: &mut AccessNode) {
        node.clone_from(&self.node);
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ControlProps {
    size: Size,
    node: AccessNode,
}

impl ViewNode for ControlProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(Control {
            size: self.size,
            node: self.node.clone(),
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node.downcast_mut::<Control>().expect("tipe view sama");
        let mut dirty = Dirty::NONE;
        if n.size != self.size {
            n.size = self.size;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.node != self.node {
            n.node.clone_from(&self.node);
            dirty |= Dirty::PAINT;
        }
        dirty
    }
}

fn control(w: f32, h: f32, node: AccessNode) -> View {
    View::new(ControlProps {
        size: Size::new(w, h),
        node,
    })
}

fn tombol(nama: &str) -> View {
    control(
        120.0,
        32.0,
        AccessNode::with_role(AccessRole::Button)
            .label(nama)
            .with_actions(AccessActions::CLICK | AccessActions::FOCUS),
    )
}

fn window(w: f32, h: f32) -> BoxConstraints {
    BoxConstraints::tight(Size::new(w, h))
}

fn anak(tree: &RenderTree, id: crate::tree::NodeId, i: usize) -> crate::tree::NodeId {
    tree.children(id)[i]
}

// ---------------------------------------------------------------------------
// Tree dump
// ---------------------------------------------------------------------------

#[test]
fn dump_memuat_peran_nama_kotak_dan_aksi() {
    let mut tree = RenderTree::new();
    tree.set_root_label("Laporan");
    reconcile(
        &mut tree,
        pad(
            Insets::all(10.0),
            kolom([
                View::from(fixed(120.0, 24.0).label("Judul")),
                tombol("Simpan"),
            ]),
        ),
    );
    tree.layout(window(300.0, 200.0));

    assert_eq!(
        tree.access_tree(None).dump(),
        "\
window \"Laporan\" [0,0 300x200] *focus
  container [0,0 300x200]
    group [10,10 280x180]
      label \"Judul\" [10,10 120x24]
      button \"Simpan\" [10,34 120x32] actions=click|focus
"
    );
}

#[test]
fn dump_menandai_nilai_keadaan_dan_fokus() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        baris([
            control(
                80.0,
                20.0,
                AccessNode::with_role(AccessRole::CheckBox)
                    .label("Arsipkan")
                    .toggled(AccessToggled::Mixed)
                    .with_actions(AccessActions::CLICK | AccessActions::FOCUS),
            ),
            control(
                80.0,
                20.0,
                AccessNode::with_role(AccessRole::TextInput)
                    .label("Nama")
                    .value("Budi")
                    .disabled(true),
            ),
        ]),
    );
    tree.layout(window(200.0, 100.0));

    let centang = anak(&tree, anak(&tree, tree.root(), 0), 0);

    assert_eq!(
        tree.access_tree(Some(centang)).dump(),
        "\
window [0,0 200x100]
  group [0,0 200x100]
    checkbox \"Arsipkan\" [0,0 80x20] actions=click|focus toggled=mixed *focus
    text_input \"Nama\" =\"Budi\" [80,0 80x20] disabled
"
    );
}

// ---------------------------------------------------------------------------
// Bounds datang dari layout
// ---------------------------------------------------------------------------

#[test]
fn bounds_datang_dari_hasil_layout_bukan_dari_widget() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, pad(Insets::all(10.0), kolom([tombol("Simpan")])));
    tree.layout(window(400.0, 400.0));

    let a11y = tree.access_tree(None);
    let entri = a11y.find_label("Simpan").expect("tombol ada di pohon a11y");
    assert_eq!(entri.bounds.origin, Point::new(10.0, 10.0));
    assert_eq!(entri.bounds.size, Size::new(120.0, 32.0));

    // Geser isinya: kotak a11y wajib ikut, tanpa widget mengubah apa pun.
    reconcile(&mut tree, pad(Insets::all(40.0), kolom([tombol("Simpan")])));
    tree.perform_layout(window(400.0, 400.0));
    let a11y = tree.access_tree(None);
    let entri = a11y.find_label("Simpan").expect("tombol masih ada");
    assert_eq!(entri.bounds.origin, Point::new(40.0, 40.0));
}

#[test]
fn kotak_a11y_mengikuti_guliran_viewport() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, viewport(kolom([tombol("Baris")])));
    tree.layout(window(200.0, 100.0));
    assert_eq!(
        tree.access_tree(None)
            .find_label("Baris")
            .expect("ada")
            .bounds
            .origin,
        Point::new(0.0, 0.0)
    );

    reconcile(&mut tree, viewport(kolom([tombol("Baris")])).scroll(30.0));
    tree.perform_layout(window(200.0, 100.0));
    assert_eq!(
        tree.access_tree(None)
            .find_label("Baris")
            .expect("ada")
            .bounds
            .origin,
        Point::new(0.0, -30.0),
        "screen reader harus melihat posisi yang sama dengan yang digambar"
    );
}

// ---------------------------------------------------------------------------
// Kontrak node
// ---------------------------------------------------------------------------

#[test]
fn viewport_mengumumkan_aksi_gulir() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, viewport(fixed(10.0, 10.0)));
    tree.layout(window(100.0, 100.0));

    let a11y = tree.access_tree(None);
    let vp = a11y
        .find_role(AccessRole::ScrollView)
        .expect("viewport muncul sebagai scroll view");
    assert!(vp.node.actions.contains(AccessActions::SCROLL));
}

#[test]
fn wadah_struktural_menyatakan_dirinya_struktural() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, pad(Insets::all(4.0), fixed(10.0, 10.0)));
    tree.layout(window(100.0, 100.0));

    let a11y = tree.access_tree(None);
    let padding = a11y
        .find_role(AccessRole::Container)
        .expect("padding adalah wadah struktural");
    assert!(padding.node.role.is_structural());
    assert!(padding.node.label.is_none());
}

#[test]
fn node_hidden_menghapus_seluruh_subtree() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        kolom([
            tombol("Terlihat"),
            View::from(pad(
                Insets::all(2.0),
                control(
                    10.0,
                    10.0,
                    AccessNode::with_role(AccessRole::Image)
                        .label("dekorasi")
                        .hidden(true),
                ),
            )),
        ]),
    );
    tree.layout(window(200.0, 200.0));

    let a11y = tree.access_tree(None);
    assert!(a11y.find_label("Terlihat").is_some());
    assert!(
        a11y.find_label("dekorasi").is_none(),
        "node hidden hilang dari pohon a11y"
    );
    assert!(
        a11y.find_role(AccessRole::Image).is_none(),
        "keturunannya ikut hilang"
    );
    // Pohon render tetap utuh — yang hilang hanya pandangan a11y.
    assert!(tree.len() > a11y.len());
}

#[test]
fn akar_tidak_pernah_hilang_meski_pohon_kosong() {
    let tree = RenderTree::new();
    let a11y = tree.access_tree(None);
    assert_eq!(a11y.len(), 1);
    assert_eq!(a11y.root(), tree.root());
    assert_eq!(a11y.focus(), tree.root(), "fokus jatuh ke window");
    assert!(a11y.is_empty());
}

// ---------------------------------------------------------------------------
// Fokus
// ---------------------------------------------------------------------------

#[test]
fn fokus_jatuh_ke_akar_bila_nodenya_mati() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, kolom([tombol("A"), tombol("B")]));
    tree.layout(window(200.0, 200.0));
    let b = anak(&tree, anak(&tree, tree.root(), 0), 1);
    assert_eq!(tree.access_tree(Some(b)).focus(), b);

    // Rebuild tanpa tombol B: fokus tidak boleh menunjuk hantu.
    reconcile(&mut tree, kolom([tombol("A")]));
    tree.perform_layout(window(200.0, 200.0));
    assert_eq!(tree.access_tree(Some(b)).focus(), tree.root());
}

#[test]
fn urutan_fokus_mengikuti_urutan_baca_bukan_koordinat() {
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        kolom([
            View::from(fixed(10.0, 10.0).label("bukan kontrol")),
            tombol("Satu"),
            control(
                10.0,
                10.0,
                AccessNode::with_role(AccessRole::Button)
                    .label("Mati")
                    .with_actions(AccessActions::FOCUS)
                    .disabled(true),
            ),
            tombol("Dua"),
        ]),
    );
    tree.layout(window(200.0, 200.0));

    let a11y = tree.access_tree(None);
    let nama: Vec<&str> = a11y
        .focus_order()
        .filter_map(|id| a11y.get(id)?.node.label.as_deref())
        .collect();
    assert_eq!(nama, ["Satu", "Dua"], "node disabled tidak ikut urutan Tab");
}

// ---------------------------------------------------------------------------
// Delta
// ---------------------------------------------------------------------------

#[test]
fn snapshot_pertama_selalu_pohon_penuh() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, kolom([tombol("Simpan")]));
    tree.layout(window(200.0, 200.0));

    let a11y = tree.access_tree(None);
    let update = a11y.changes_since(None);
    assert!(update.full);
    assert_eq!(update.changed.len(), a11y.len());
    assert!(update.removed.is_empty());
    assert!(!update.is_empty());
}

#[test]
fn frame_tanpa_perubahan_tidak_mengirim_apa_pun() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, kolom([tombol("Simpan")]));
    tree.layout(window(200.0, 200.0));

    let sebelum = tree.access_tree(None);
    let sesudah = tree.access_tree(None);
    let update = sesudah.changes_since(Some(&sebelum));
    assert!(
        update.is_empty(),
        "screen reader tidak boleh dibangunkan tanpa sebab"
    );
    assert!(!update.full);
}

#[test]
fn hanya_node_berubah_yang_dikirim() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, kolom([tombol("Simpan"), tombol("Batal")]));
    tree.layout(window(200.0, 200.0));
    let sebelum = tree.access_tree(None);

    reconcile(&mut tree, kolom([tombol("Simpan"), tombol("Tutup")]));
    tree.perform_layout(window(200.0, 200.0));
    let sesudah = tree.access_tree(None);

    let update = sesudah.changes_since(Some(&sebelum));
    let nama: Vec<Option<&str>> = update
        .changed
        .iter()
        .map(|e: &AccessEntry| e.node.label.as_deref())
        .collect();
    assert_eq!(nama, [Some("Tutup")]);
    assert!(update.removed.is_empty());
}

#[test]
fn node_yang_dibuang_ikut_membawa_induknya_yang_berubah() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, kolom([tombol("A"), tombol("B")]));
    tree.layout(window(200.0, 200.0));
    let sebelum = tree.access_tree(None);
    let wadah = anak(&tree, tree.root(), 0);
    let b = anak(&tree, wadah, 1);

    reconcile(&mut tree, kolom([tombol("A")]));
    tree.perform_layout(window(200.0, 200.0));
    let sesudah = tree.access_tree(None);

    let update = sesudah.changes_since(Some(&sebelum));
    assert_eq!(update.removed, [b]);
    assert!(
        update.changed.iter().any(|e| e.id == wadah),
        "induk wajib ikut supaya platform tahu anaknya hilang"
    );
}

#[test]
fn perpindahan_fokus_saja_tetap_terkirim() {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, kolom([tombol("A"), tombol("B")]));
    tree.layout(window(200.0, 200.0));
    let sebelum = tree.access_tree(None);

    let b = anak(&tree, anak(&tree, tree.root(), 0), 1);
    let sesudah = tree.access_tree(Some(b));

    let update = sesudah.changes_since(Some(&sebelum));
    assert!(update.changed.is_empty(), "tidak ada node yang berubah isi");
    assert!(update.focus_changed);
    assert!(!update.is_empty(), "perpindahan fokus wajib diumumkan");
}

#[test]
fn pohon_dari_window_lain_dianggap_pohon_baru() {
    let mut a = RenderTree::new();
    reconcile(&mut a, kolom([tombol("A")]));
    a.layout(window(200.0, 200.0));

    let mut b = RenderTree::new();
    reconcile(&mut b, kolom([tombol("A")]));
    b.layout(window(200.0, 200.0));

    let update = b
        .access_tree(None)
        .changes_since(Some(&a.access_tree(None)));
    assert!(
        update.full,
        "id node dari window lain tidak boleh dicocokkan"
    );
}

// ---------------------------------------------------------------------------
// Kosakata
// ---------------------------------------------------------------------------

#[test]
fn kemampuan_dan_permintaan_aksi_saling_menutup() {
    assert_eq!(AccessAction::Click.capability(), AccessActions::CLICK);
    assert_eq!(AccessAction::Blur.capability(), AccessActions::FOCUS);
    assert_eq!(
        AccessAction::ScrollDown.capability(),
        AccessActions::SCROLL,
        "semua arah gulir bersandar pada satu kemampuan"
    );
}

#[test]
fn actions_adalah_bitset_dengan_nama_stabil() {
    let mut a = AccessActions::CLICK;
    a |= AccessActions::FOCUS;
    assert!(a.contains(AccessActions::CLICK | AccessActions::FOCUS));
    assert!(!a.contains(AccessActions::SCROLL));
    assert_eq!(format!("{a:?}"), "AccessActions(click|focus)");
    assert_eq!(format!("{:?}", AccessActions::NONE), "AccessActions(none)");
    a.remove(AccessActions::CLICK);
    assert_eq!(a, AccessActions::FOCUS);
}

#[test]
fn node_default_adalah_wadah_tanpa_nama() {
    let n = AccessNode::new();
    assert_eq!(n.role, AccessRole::Container);
    assert!(n.label.is_none());
    assert!(n.actions.is_empty());
    assert!(!n.is_focusable());
}

// ---------------------------------------------------------------------------
// Jembatan AccessKit
// ---------------------------------------------------------------------------

#[cfg(feature = "accesskit")]
mod accesskit_bridge {
    use super::*;
    use crate::access::{accesskit, accesskit_id};

    #[test]
    fn tree_update_membawa_peran_nama_dan_kotak_piksel_fisik() {
        let mut tree = RenderTree::new();
        tree.set_root_label("Laporan");
        reconcile(&mut tree, pad(Insets::all(10.0), kolom([tombol("Simpan")])));
        tree.layout(window(300.0, 200.0));

        let a11y = tree.access_tree(None);
        let update = a11y.to_tree_update(2.0);

        assert_eq!(update.focus, accesskit_id(a11y.root()));
        assert_eq!(update.tree.as_ref().map(|t| t.root), Some(update.focus));
        assert_eq!(
            update.tree.as_ref().and_then(|t| t.toolkit_name.as_deref()),
            Some("rustui")
        );

        let tombol_id = a11y.find_label("Simpan").expect("ada").id;
        let (_, node) = update
            .nodes
            .iter()
            .find(|(id, _)| *id == accesskit_id(tombol_id))
            .expect("tombol ikut terkirim");
        assert_eq!(node.role(), accesskit::Role::Button);
        assert_eq!(node.label(), Some("Simpan"));
        assert!(node.supports_action(accesskit::Action::Click));
        assert!(node.supports_action(accesskit::Action::Focus));
        // Poin logis (10,10) 120×32 pada layar Retina.
        assert_eq!(
            node.bounds(),
            Some(accesskit::Rect::new(20.0, 20.0, 260.0, 84.0))
        );
    }

    #[test]
    fn wadah_struktural_dipetakan_ke_generic_container() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, pad(Insets::all(4.0), fixed(10.0, 10.0)));
        tree.layout(window(100.0, 100.0));

        let a11y = tree.access_tree(None);
        let padding = a11y.find_role(AccessRole::Container).expect("ada");
        let update = a11y.to_tree_update(1.0);
        let (_, node) = update
            .nodes
            .iter()
            .find(|(id, _)| *id == accesskit_id(padding.id))
            .expect("ada");
        assert_eq!(node.role(), accesskit::Role::GenericContainer);
    }

    #[test]
    fn id_tidak_pernah_diwarisi_slot_yang_dipakai_ulang() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, kolom([tombol("A")]));
        tree.layout(window(200.0, 200.0));
        let lama = anak(&tree, anak(&tree, tree.root(), 0), 0);

        tree.remove_subtree(lama);
        reconcile(&mut tree, kolom([tombol("A")]));
        tree.perform_layout(window(200.0, 200.0));
        let baru = anak(&tree, anak(&tree, tree.root(), 0), 0);

        assert_eq!(baru.index(), lama.index(), "slot arena dipakai ulang");
        assert_ne!(
            accesskit_id(baru),
            accesskit_id(lama),
            "id a11y wajib ikut generasi, kalau tidak screen reader salah orang"
        );
    }

    #[test]
    fn delta_tidak_membawa_data_pohon_ulang() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, kolom([tombol("A"), tombol("B")]));
        tree.layout(window(200.0, 200.0));
        let sebelum = tree.access_tree(None);

        reconcile(&mut tree, kolom([tombol("A"), tombol("C")]));
        tree.perform_layout(window(200.0, 200.0));
        let update = tree.access_tree(None).changes_since(Some(&sebelum));
        let ak = update.to_tree_update(1.0);
        assert!(ak.tree.is_none(), "hanya update penuh yang membawa Tree");
        assert_eq!(ak.nodes.len(), 1);
    }

    #[test]
    fn permintaan_aksi_diterjemahkan_dan_divalidasi() {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, kolom([tombol("Simpan")]));
        tree.layout(window(200.0, 200.0));
        let a11y = tree.access_tree(None);
        let tombol_id = a11y.find_label("Simpan").expect("ada").id;

        let sah = accesskit::ActionRequest {
            action: accesskit::Action::Click,
            target_tree: accesskit::TreeId::ROOT,
            target_node: accesskit_id(tombol_id),
            data: None,
        };
        let req = a11y.action_request(&sah).expect("permintaan sah diterima");
        assert_eq!(req.target, tombol_id);
        assert_eq!(req.action, AccessAction::Click);

        // Aksi yang tidak diumumkan node itu ditolak, bukan diteruskan.
        let tak_sah = accesskit::ActionRequest {
            action: accesskit::Action::Increment,
            target_tree: accesskit::TreeId::ROOT,
            target_node: accesskit_id(tombol_id),
            data: None,
        };
        assert!(a11y.action_request(&tak_sah).is_none());

        // Node yang sudah mati satu frame lalu juga ditolak.
        let hantu = accesskit::ActionRequest {
            action: accesskit::Action::Click,
            target_tree: accesskit::TreeId::ROOT,
            target_node: accesskit::NodeId(u64::MAX),
            data: None,
        };
        assert!(a11y.action_request(&hantu).is_none());
    }

    #[test]
    fn nilai_baru_ikut_di_permintaan_set_value() {
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            kolom([control(
                100.0,
                20.0,
                AccessNode::with_role(AccessRole::TextInput)
                    .label("Nama")
                    .with_actions(AccessActions::SET_VALUE | AccessActions::FOCUS),
            )]),
        );
        tree.layout(window(200.0, 200.0));
        let a11y = tree.access_tree(None);
        let field = a11y.find_label("Nama").expect("ada").id;

        let req = a11y
            .action_request(&accesskit::ActionRequest {
                action: accesskit::Action::SetValue,
                target_tree: accesskit::TreeId::ROOT,
                target_node: accesskit_id(field),
                data: Some(accesskit::ActionData::Value("Budi".into())),
            })
            .expect("permintaan sah");
        assert_eq!(req.action, AccessAction::SetValue);
        assert_eq!(req.value.as_deref(), Some("Budi"));
    }
}
