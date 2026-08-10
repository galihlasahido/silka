//! Lifecycle verification **without a GPU**: this whole file runs headless.
//!
//! What is proven here is exactly the five things that make this seam
//! trustworthy: the first frame produces a scene, a signal change schedules a
//! frame, only the relevant subtree is rebuilt, the scene changes with it, and
//! without a signal change there is no frame at all.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use silka_paint::{Color, Command, Point, Quad, Scene, Size};

use crate::input::{Event, PointerButton, PointerEvent, PointerPhase};
use crate::scheduler::{Dirty, Wake};
use crate::signals::{list, use_signal, Key, Signal};
use crate::tree::TextDirection;
use crate::view::{column, fixed, interactive, row};

use super::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Every quad in the scene, in draw order.
fn quads(scene: &Scene) -> Vec<Quad> {
    scene
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Quad(q) => Some(q.clone()),
            _ => None,
        })
        .collect()
}

/// The height of every quad — a scene fingerprint that is easy to read.
fn tinggi(scene: &Scene) -> Vec<f32> {
    quads(scene).iter().map(|q| q.rect.size.height).collect()
}

/// Sample application: one counter in the root scope, two child components.
/// Only the `"angka"` component **reads** the counter.
fn app_counter() -> (AppRuntime, Rc<Cell<Option<Signal<i32>>>>) {
    let pegangan: Rc<Cell<Option<Signal<i32>>>> = Rc::default();
    let simpan = pegangan.clone();
    let ui = app(move |_cx| {
        let count = use_signal(|| 0i32);
        simpan.set(Some(count));
        column([
            component("judul", |_| {
                fixed(80.0, 20.0).background(Color::WHITE).into()
            }),
            component("angka", move |_| {
                fixed(40.0, 20.0 + count.get() as f32 * 10.0)
                    .background(Color::WHITE)
                    .into()
            }),
        ])
        .into()
    })
    .sized(320.0, 240.0);
    (ui, pegangan)
}

/// The `i`-th component anchor under the root column.
fn jangkar(ui: &AppRuntime, i: usize) -> crate::tree::NodeId {
    let akar = ui.tree().root();
    let kolom = ui.tree().children(akar)[0];
    ui.tree().children(kolom)[i]
}

// ---------------------------------------------------------------------------
// (a)+(b) the first frame
// ---------------------------------------------------------------------------

#[test]
fn frame_pertama_membangun_pohon_dan_scene() {
    let (mut ui, pegangan) = app_counter();

    // Before the first frame: the tree is still empty, but a frame is already
    // scheduled.
    assert!(
        !ui.is_idle(),
        "frame pertama harus terjadwal sejak konstruksi"
    );
    assert_eq!(ui.tree().len(), 1, "baru node akar");

    let laporan = ui.frame();

    assert_eq!(laporan.rebuilt, 1, "hanya scope akar yang dibangun");
    // column + 2 component anchors + 2 leaves.
    assert_eq!(laporan.diff.created, 5);
    assert_eq!(laporan.diff.removed, 0);
    assert_eq!(laporan.size, Size::new(320.0, 240.0));

    assert_eq!(tinggi(ui.scene()), vec![20.0, 20.0]);
    assert!(
        pegangan.get().is_some(),
        "use_signal berjalan di scope akar"
    );
    assert_eq!(ui.runtime().live_scopes(), 3, "akar + dua komponen");
}

#[test]
fn scene_memakai_warna_latar_dari_token() {
    let (mut ui, _) = app_counter();
    ui.set_clear_color(Color::hex(0x1C1C1E));
    ui.frame();
    assert_eq!(ui.scene().clear_color(), Color::hex(0x1C1C1E));
}

// ---------------------------------------------------------------------------
// (c)+(d) a signal change → rebuild the subtree only
// ---------------------------------------------------------------------------

#[test]
fn perubahan_signal_hanya_membangun_ulang_subtree_pembacanya() {
    let (mut ui, pegangan) = app_counter();
    ui.frame();

    let count = pegangan.get().unwrap();
    let jangkar_judul = jangkar(&ui, 0);
    let jangkar_angka = jangkar(&ui, 1);
    let daun_judul = ui.tree().children(jangkar_judul)[0];

    count.set(3);

    let laporan = ui.frame();

    // (d) — one scope rebuilt, and not a single new node.
    assert_eq!(laporan.rebuilt, 1, "hanya komponen 'angka'");
    assert_eq!(laporan.diff.created, 0);
    assert_eq!(laporan.diff.replaced, 0);
    assert_eq!(laporan.diff.removed, 0);
    assert_eq!(laporan.diff.moved, 0);
    assert_eq!(laporan.diff.reused, 1, "hanya daun 'angka' yang di-diff");
    assert_eq!(laporan.diff.updated, 1);

    // The neighbour node keeps its identity: it never entered the diff path.
    assert_eq!(jangkar(&ui, 0), jangkar_judul);
    assert_eq!(jangkar(&ui, 1), jangkar_angka);
    assert_eq!(ui.tree().children(jangkar_judul)[0], daun_judul);

    // The scene changes exactly in line with the new value.
    assert_eq!(tinggi(ui.scene()), vec![20.0, 50.0]);
}

#[test]
fn scope_akar_tidak_ikut_kotor_saat_anaknya_yang_membaca() {
    let (mut ui, pegangan) = app_counter();
    ui.frame();
    let count = pegangan.get().unwrap();

    count.set(1);
    assert_eq!(
        ui.runtime().dirty_len(),
        1,
        "hanya satu scope yang berlangganan counter"
    );
    assert!(!ui.runtime().is_dirty(ui.root_scope()));
}

#[test]
fn komponen_yang_tidak_membaca_apa_pun_tidak_pernah_dibangun_ulang() {
    let bangun_judul = Rc::new(Cell::new(0u32));
    let hitung = bangun_judul.clone();
    let pegangan: Rc<Cell<Option<Signal<i32>>>> = Rc::default();
    let simpan = pegangan.clone();

    let mut ui = app(move |_cx| {
        let count = use_signal(|| 0i32);
        simpan.set(Some(count));
        let hitung = hitung.clone();
        column([
            component("judul", move |_| {
                hitung.set(hitung.get() + 1);
                fixed(80.0, 20.0).background(Color::WHITE).into()
            }),
            component("angka", move |_| {
                fixed(40.0, 20.0 + count.get() as f32 * 10.0)
                    .background(Color::WHITE)
                    .into()
            }),
        ])
        .into()
    })
    .sized(320.0, 240.0);

    ui.frame();
    assert_eq!(bangun_judul.get(), 1);

    for n in 1..=3 {
        pegangan.get().unwrap().set(n);
        ui.frame();
    }
    assert_eq!(
        bangun_judul.get(),
        1,
        "komponen tetangga tidak boleh dibangun ulang sama sekali"
    );
    assert_eq!(tinggi(ui.scene()), vec![20.0, 50.0]);
}

#[test]
fn rebuild_akar_memasuki_kembali_setiap_anak_yang_dipertahankan() {
    // The `drain_dirty` contract: pruning descendants is sound only if
    // rebuilding an ancestor really does re-enter each of its children.
    let masuk = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let catat = masuk.clone();
    let pegangan: Rc<Cell<Option<Signal<i32>>>> = Rc::default();
    let simpan = pegangan.clone();

    let mut ui = app(move |_cx| {
        let akar = use_signal(|| 0i32);
        simpan.set(Some(akar));
        let _ = akar.get(); // the root itself subscribes
        let catat = catat.clone();
        column([
            component("a", {
                let catat = catat.clone();
                move |_| {
                    catat.borrow_mut().push("a");
                    fixed(10.0, 10.0).into()
                }
            }),
            component("b", move |_| {
                catat.borrow_mut().push("b");
                fixed(10.0, 10.0).into()
            }),
        ])
        .into()
    })
    .sized(200.0, 200.0);

    ui.frame();
    assert_eq!(*masuk.borrow(), vec!["a", "b"]);

    pegangan.get().unwrap().set(1);
    ui.frame();
    assert_eq!(
        *masuk.borrow(),
        vec!["a", "b", "a", "b"],
        "anak yang dipertahankan wajib dimasuki lagi"
    );
    assert_eq!(ui.runtime().live_scopes(), 3, "tidak ada scope yang lahir");
}

// ---------------------------------------------------------------------------
// (e) idle = zero
// ---------------------------------------------------------------------------

#[test]
fn tanpa_perubahan_signal_tidak_ada_frame_terjadwal() {
    let (mut ui, pegangan) = app_counter();
    ui.frame();

    // (e) — after the first frame, genuinely idle.
    assert!(ui.is_idle());
    assert_eq!(ui.pending(), Dirty::NONE);

    // Reading a signal wakes nothing.
    let count = pegangan.get().unwrap();
    assert_eq!(count.peek(), 0);
    assert!(ui.is_idle());

    // Neither does writing the same value.
    assert!(!count.set_if_changed(0));
    assert!(
        ui.is_idle(),
        "nilai yang tidak berubah tidak menjadwalkan frame"
    );

    // Only a real change schedules anything.
    count.set(1);
    assert!(!ui.is_idle());
    assert_eq!(ui.pending(), Dirty::LAYOUT | Dirty::PAINT);

    ui.frame();
    assert!(ui.is_idle(), "frame yang sudah dilayani mengembalikan idle");
}

#[test]
fn signal_yang_tidak_dibaca_siapa_pun_tidak_membangunkan_renderer() {
    let (mut ui, _) = app_counter();
    ui.frame();

    let yatim = ui.runtime().signal(0i32);
    yatim.set(99);
    assert!(
        ui.is_idle(),
        "tidak ada komponen yang berlangganan → nol kerja"
    );
}

#[test]
fn frame_ulang_tanpa_perubahan_tidak_mengerjakan_apa_pun() {
    let (mut ui, _) = app_counter();
    ui.frame();
    let sebelum = tinggi(ui.scene());

    // A frame the OS asks for (expose) may happen even with nothing dirty.
    let laporan = ui.frame();
    assert!(laporan.is_noop());
    assert_eq!(laporan.reason, Dirty::NONE);
    assert_eq!(tinggi(ui.scene()), sebelum);
    assert!(ui.is_idle());
}

// ---------------------------------------------------------------------------
// The wiring into the scheduler
// ---------------------------------------------------------------------------

#[test]
fn on_wake_meneruskan_jadwal_ke_shell() {
    let (mut ui, pegangan) = app_counter();
    ui.frame();

    let jejak = Rc::new(RefCell::new(Vec::<Wake>::new()));
    let rekam = jejak.clone();
    ui.on_wake(move |w| rekam.borrow_mut().push(w));

    let count = pegangan.get().unwrap();
    count.set(1);
    assert_eq!(*jejak.borrow(), vec![Wake::Schedule]);

    // A second write to the same signal marks no new scope, so it never
    // reaches the scheduler at all — the platform is not poked twice for one
    // and the same frame.
    count.set(2);
    assert_eq!(*jejak.borrow(), vec![Wake::Schedule]);

    // Even a genuinely different source only finds the frame already
    // scheduled.
    assert_eq!(ui.request(Dirty::EXTERNAL), Wake::AlreadyScheduled);
    assert_eq!(
        *jejak.borrow(),
        vec![Wake::Schedule, Wake::AlreadyScheduled]
    );
}

#[test]
fn batch_menyatukan_banyak_tulisan_menjadi_satu_pembangunan() {
    let (mut ui, pegangan) = app_counter();
    ui.frame();

    let jejak = Rc::new(RefCell::new(Vec::<Wake>::new()));
    let rekam = jejak.clone();
    ui.on_wake(move |w| rekam.borrow_mut().push(w));

    let count = pegangan.get().unwrap();
    let rt = ui.runtime().clone();
    rt.batch(|| {
        count.set(1);
        count.set(2);
        count.set(3);
    });
    assert_eq!(*jejak.borrow(), vec![Wake::Schedule], "satu wake per batch");

    let laporan = ui.frame();
    assert_eq!(laporan.rebuilt, 1);
    assert_eq!(tinggi(ui.scene()), vec![20.0, 50.0]);
}

#[test]
fn resize_menjadwalkan_layout_tanpa_membangun_ulang_komponen() {
    let (mut ui, _) = app_counter();
    ui.frame();
    assert!(ui.is_idle());

    assert!(ui.resize(Size::new(400.0, 300.0)));
    assert_eq!(ui.pending(), Dirty::SURFACE | Dirty::LAYOUT);

    let laporan = ui.frame();
    assert_eq!(laporan.rebuilt, 0, "resize tidak membangun ulang komponen");
    assert_eq!(laporan.size, Size::new(400.0, 300.0));

    assert!(
        !ui.resize(Size::new(400.0, 300.0)),
        "ukuran sama = nol kerja"
    );
    assert!(ui.is_idle());
}

// ---------------------------------------------------------------------------
// Identity & dynamic lists
// ---------------------------------------------------------------------------

#[test]
fn daftar_berkunci_mempertahankan_state_saat_urutannya_berubah() {
    let mut ui = app(|cx| {
        let urutan: Signal<Vec<i64>> = cx.expect_env();
        row(list(
            urutan.get(),
            |id| Key::num(*id),
            |id| {
                let id = *id;
                component(Key::num(id), move |_| {
                    // Per-row local state: created once, tied to its key.
                    let tinggi = use_signal(|| 10.0 + id as f32);
                    fixed(20.0, tinggi.get()).background(Color::WHITE).into()
                })
            },
        ))
        .into()
    })
    .with_env(|rt| rt.signal(vec![1i64, 2, 3]))
    .sized(320.0, 240.0);

    ui.frame();
    assert_eq!(tinggi(ui.scene()), vec![11.0, 12.0, 13.0]);
    let hidup = ui.runtime().live_scopes();

    let urutan: Signal<Vec<i64>> = ui.env().unwrap();
    urutan.set(vec![3, 2, 1]);
    let laporan = ui.frame();

    assert_eq!(laporan.rebuilt, 1, "akar yang membaca daftar");
    assert_eq!(laporan.diff.created, 0, "tidak ada baris yang lahir");
    assert_eq!(laporan.diff.removed, 0, "tidak ada baris yang mati");
    assert_eq!(ui.runtime().live_scopes(), hidup);
    // State follows its key, not its position.
    assert_eq!(tinggi(ui.scene()), vec![13.0, 12.0, 11.0]);
}

#[test]
fn daftar_yang_menyusut_membuang_scope_dan_node() {
    // `component()` already creates its own scope, so `list()` is not used
    // here — using both together would give every row two scopes.
    let mut ui = app(|cx| {
        let urutan: Signal<Vec<i64>> = cx.expect_env();
        row(urutan
            .get()
            .into_iter()
            .map(|id| {
                component(Key::num(id), move |_| {
                    fixed(20.0, 10.0 + id as f32)
                        .background(Color::WHITE)
                        .into()
                })
            })
            .collect::<Vec<_>>())
        .into()
    })
    .with_env(|rt| rt.signal(vec![1i64, 2, 3]))
    .sized(320.0, 240.0);

    ui.frame();
    assert_eq!(ui.runtime().live_scopes(), 4);

    let urutan: Signal<Vec<i64>> = ui.env().unwrap();
    urutan.set(vec![1, 3]);
    let laporan = ui.frame();

    assert_eq!(laporan.rebuilt, 1);
    assert_eq!(laporan.diff.removed, 2, "jangkar + daun milik kunci 2");
    assert_eq!(tinggi(ui.scene()), vec![11.0, 13.0]);
    assert_eq!(ui.runtime().live_scopes(), 3, "scope kunci 2 ikut dibuang");
}

// ---------------------------------------------------------------------------
// Other contracts that travel through this cycle
// ---------------------------------------------------------------------------

#[test]
fn jangkar_komponen_transparan_bagi_layout() {
    let mut polos =
        app(|_| column([fixed(40.0, 20.0), fixed(60.0, 30.0)]).into()).sized(200.0, 200.0);
    let mut berkomponen = app(|_| {
        column([
            component("a", |_| fixed(40.0, 20.0).into()),
            component("b", |_| fixed(60.0, 30.0).into()),
        ])
        .into()
    })
    .sized(200.0, 200.0);

    let a = polos.frame();
    let b = berkomponen.frame();
    assert_eq!(a.size, b.size);

    // Leaf sizes & positions must be identical to the component-free version.
    let geometri = |ui: &AppRuntime| -> Vec<(Size, Point)> {
        let akar = ui.tree().root();
        let kolom = ui.tree().children(akar)[0];
        ui.tree()
            .children(kolom)
            .iter()
            .map(|c| {
                // Descend past the component anchor when there is one.
                let daun = match ui.tree().render(*c) {
                    Some(n) if n.downcast_ref::<ComponentBox>().is_some() => {
                        ui.tree().children(*c)[0]
                    }
                    _ => *c,
                };
                (ui.tree().size(daun), ui.tree().global_offset(daun))
            })
            .collect()
    };
    assert_eq!(geometri(&polos), geometri(&berkomponen));
}

#[test]
fn pohon_a11y_datang_dari_geometri_frame_yang_sama() {
    let mut ui = app(|_| {
        column([component("isi", |_| {
            fixed(80.0, 20.0).label("Simpan").into()
        })])
        .into()
    })
    .sized(200.0, 200.0);
    ui.frame();

    let pohon = ui.access_tree();
    let entri = pohon
        .find_label("Simpan")
        .expect("label ikut ke pohon a11y");
    assert_eq!(entri.bounds.size, Size::new(80.0, 20.0));
    assert_eq!(pohon.focus(), pohon.root(), "belum ada yang difokuskan");
}

#[test]
fn input_mengalir_ke_pohon_hasil_siklus_ini() {
    let mut ui = app(|_| {
        component("tombol", |_| {
            interactive(fixed(120.0, 44.0).background(Color::WHITE))
                .label("Simpan")
                .into()
        })
    })
    .sized(320.0, 240.0);
    ui.frame();
    assert!(ui.is_idle());

    let tekan = PointerEvent::new(PointerPhase::Down, Point::new(20.0, 20.0), Duration::ZERO)
        .button(PointerButton::Primary);
    let hasil = ui.dispatch(&Event::Pointer(tekan));

    assert!(hasil.handled);
    assert!(!hasil.dirty.is_empty());
    assert!(!ui.is_idle(), "input yang berdampak menjadwalkan frame");

    ui.frame();
    assert!(ui.is_idle());
    assert!(ui.router().focus().focused().is_some(), "fokus ikut pindah");
}

#[test]
fn arah_rtl_diteruskan_ke_pohon() {
    let mut ui = app(|_| column([fixed(10.0, 10.0)]).into()).sized(100.0, 100.0);
    ui.frame();
    assert!(ui.set_direction(TextDirection::Rtl));
    assert_eq!(ui.tree().direction(), TextDirection::Rtl);
    assert!(!ui.is_idle());
    ui.frame();
    assert!(ui.is_idle());
}

#[test]
fn dua_aplikasi_di_thread_yang_sama_tidak_saling_mengganggu() {
    let (mut satu, p1) = app_counter();
    let (mut dua, p2) = app_counter();
    satu.frame();
    dua.frame();

    p1.get().unwrap().set(5);
    assert!(!satu.is_idle());
    assert!(
        dua.is_idle(),
        "signal aplikasi lain tidak membangunkan yang ini"
    );

    satu.frame();
    assert_eq!(tinggi(satu.scene()), vec![20.0, 70.0]);
    assert_eq!(tinggi(dua.scene()), vec![20.0, 20.0]);
    assert!(p2.get().is_some());
}

#[test]
fn komponen_di_luar_build_adalah_kesalahan_yang_jelas() {
    let hasil = std::panic::catch_unwind(|| {
        let _ = component("liar", |_| fixed(1.0, 1.0).into());
    });
    assert!(hasil.is_err());
}

// ---------------------------------------------------------------------------
// Animation: who schedules the next frame
// ---------------------------------------------------------------------------

/// A node with one spring that can be retargeted through props — the same
/// shape as an overlay panel or a button entering its loading state.
#[derive(Debug)]
struct Bergerak {
    nilai: crate::animation::SpringValue<f32>,
}

impl crate::tree::RenderNode for Bergerak {
    fn layout(
        &mut self,
        _ctx: &mut crate::tree::LayoutCtx<'_>,
        constraints: crate::tree::BoxConstraints,
    ) -> Size {
        constraints.constrain(Size::new(10.0, 10.0 + self.nilai.position()))
    }

    fn access(&self, node: &mut crate::access::AccessNode) {
        node.role = crate::access::AccessRole::Container;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct BergerakProps {
    target: f32,
}

impl crate::view::ViewNode for BergerakProps {
    fn build(&self) -> Box<dyn crate::tree::RenderNode> {
        Box::new(Bergerak {
            nilai: crate::animation::SpringValue::new(0.0),
        })
    }

    fn update(&self, node: &mut dyn crate::tree::RenderNode) -> Dirty {
        let n = node.downcast_mut::<Bergerak>().expect("tipe sama");
        if n.nilai.target() == self.target {
            return Dirty::NONE;
        }
        n.nilai.set_target(self.target);
        // Exactly what `OverlayProps::update` reports when a dialog opens:
        // something is **going to move**, but has not moved this frame.
        Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION
    }
}

/// Advance every `Bergerak` in the tree — the `silka_widgets::advance` shape.
fn maju(tree: &mut crate::tree::RenderTree, tick: &crate::animation::Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    let mut tumpukan = vec![tree.root()];
    while let Some(id) = tumpukan.pop() {
        tumpukan.extend_from_slice(tree.children(id));
        let hasil = tree.node_mut_ref::<Bergerak>(id).map(|n| {
            let sebelum = n.nilai.position();
            tick.advance(&mut n.nilai);
            (n.nilai.position() != sebelum, n.nilai.is_animating())
        });
        if let Some((pindah, bergerak)) = hasil {
            if pindah {
                tree.mark_needs_layout(id);
                dirty |= Dirty::LAYOUT | Dirty::PAINT;
            }
            if bergerak {
                dirty |= Dirty::ANIMATION;
            }
        }
    }
    dirty
}

fn app_bergerak() -> (AppRuntime, Rc<Cell<Option<Signal<f32>>>>) {
    let pegangan: Rc<Cell<Option<Signal<f32>>>> = Rc::default();
    let simpan = pegangan.clone();
    let ui = app(move |_cx| {
        let target = use_signal(|| 0.0f32);
        simpan.set(Some(target));
        crate::view::Builder::new(BergerakProps {
            target: target.get(),
        })
        .into()
    })
    .sized(100.0, 100.0);
    (ui, pegangan)
}

#[test]
fn animasi_yang_dimulai_view_diff_menjadwalkan_frame_berikutnya() {
    // Regression: `Dirty::ANIMATION` from `ViewNode::update` used to be lost
    // twice — in `terapkan_dirty` (it never reached the tree) and at the end of
    // `frame` (dropped along with the other flags). A dialog opened through a
    // signal therefore froze on its first frame until the next input event.
    let (mut ui, pegangan) = app_bergerak();
    ui.frame();
    assert!(ui.is_idle());

    pegangan.get().unwrap().set(50.0);
    ui.frame();
    assert!(
        ui.pending().contains(Dirty::ANIMATION),
        "spring yang baru diarahkan harus meminta frame berikutnya"
    );
    assert!(!ui.is_idle());
}

#[test]
fn spring_yang_belum_settle_tetap_meminta_frame_sampai_selesai() {
    use std::time::Instant;

    let (mut ui, pegangan) = app_bergerak();
    ui.frame();
    pegangan.get().unwrap().set(50.0);
    ui.frame();

    // The shell cycle: animate → frame. `begin_frame` clears the reasons that
    // scheduled this frame, so what keeps the animation alive is `frame`
    // itself — not a request that has already been served.
    let mut jam = Instant::now();
    let mut n = 0;
    while !ui.is_idle() {
        jam += Duration::from_millis(16);
        ui.animate_at(jam, maju);
        ui.frame();
        n += 1;
        assert!(n < 600, "animasi tidak pernah selesai");
    }
    assert!(n > 1, "transisi harus memakan lebih dari satu frame");
    assert!(!ui.is_animating());
    // The value really does reach its target rather than stopping halfway.
    let node = ui.tree().children(ui.tree().root())[0];
    let posisi = ui
        .tree()
        .node_ref::<Bergerak>(node)
        .unwrap()
        .nilai
        .position();
    assert_eq!(posisi, 50.0);
}

// ---------------------------------------------------------------------------
// Ambient theme (§2.6): the utility vocabulary resolves against `Env`
// ---------------------------------------------------------------------------

/// The whole rebuild pass runs under the injected `Signal<Theme>`, so a page
/// written in the utility vocabulary never has to name a theme.
#[test]
fn kosakata_utility_memakai_theme_dari_env() {
    use crate::view::div;
    use silka_theme::{Appearance, ColorToken, Theme};

    let tema = Theme::tailwind(Appearance::Dark);
    let mut ui = app(|_cx| div().bg(ColorToken::Surface).into())
        .with_env(move |rt| rt.signal(tema))
        .sized(200.0, 120.0);
    ui.frame();
    assert_eq!(quads(ui.scene())[0].background, tema.color.surface);
}

/// A theme change repaints in the new palette **without** the page mentioning
/// it: the value is resolved during the rebuild the signal triggers.
#[test]
fn ganti_theme_mengubah_warna_yang_diresolusi_utility() {
    use crate::view::div;
    use silka_theme::{Appearance, ColorToken, Theme};

    let terang = Theme::cupertino(Appearance::Light);
    let gelap = Theme::cupertino(Appearance::Dark);
    let mut ui = app(|cx| {
        // Read: this is what marks the root dirty when the theme changes.
        let _t: Theme = cx.expect_env::<Signal<Theme>>().get();
        div().bg(ColorToken::Surface).into()
    })
    .with_env(move |rt| rt.signal(terang))
    .sized(200.0, 120.0);
    ui.frame();
    assert_eq!(quads(ui.scene())[0].background, terang.color.surface);

    ui.env::<Signal<Theme>>().unwrap().set(gelap);
    ui.frame();
    assert_eq!(quads(ui.scene())[0].background, gelap.color.surface);
    assert_ne!(terang.color.surface, gelap.color.surface);
}

/// A component deeper in the tree is rebuilt on its own, outside the root
/// closure — the ambient theme has to reach it too, or a hover would resolve
/// its colors against `Theme::default` the moment a signal fires.
#[test]
fn komponen_yang_dibangun_ulang_sendiri_tetap_dapat_theme() {
    use crate::view::div;
    use silka_theme::{Appearance, ColorToken, Theme};

    let tema = Theme::tailwind(Appearance::Light);
    let pegangan: Rc<RefCell<Option<Signal<bool>>>> = Rc::new(RefCell::new(None));
    let simpan = pegangan.clone();

    let mut ui = app(move |_cx| {
        let simpan = simpan.clone();
        column([component("kartu", move |_cx| {
            let tekan = use_signal(|| false);
            *simpan.borrow_mut() = Some(tekan);
            let warna = if tekan.get() {
                ColorToken::SurfacePressed
            } else {
                ColorToken::Surface
            };
            div().bg(warna).child(fixed(60.0, 24.0)).into()
        })])
        .into()
    })
    .with_env(move |rt| rt.signal(tema))
    .sized(200.0, 120.0);
    ui.frame();
    assert_eq!(quads(ui.scene())[0].background, tema.color.surface);

    // Only the component's own scope is dirty here: the root closure — and with
    // it any `with_theme` a shell might have wrapped around it — does not run.
    pegangan.borrow().unwrap().set(true);
    let laporan = ui.frame();
    assert_eq!(laporan.rebuilt, 1);
    assert_eq!(quads(ui.scene())[0].background, tema.color.surface_pressed);
}
