//! Verifikasi siklus hidup **tanpa GPU**: seluruh berkas ini berjalan headless.
//!
//! Yang dibuktikan di sini persis lima hal yang membuat jahitan ini bisa
//! dipercaya: frame pertama menghasilkan scene, perubahan signal menjadwalkan
//! frame, hanya subtree terkait yang dibangun ulang, scene ikut berubah, dan
//! tanpa perubahan signal tidak ada frame sama sekali.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use rustui_paint::{Color, Command, Point, Quad, Scene, Size};

use crate::input::{Event, PointerButton, PointerEvent, PointerPhase};
use crate::scheduler::{Dirty, Wake};
use crate::signals::{list, use_signal, Key, Signal};
use crate::tree::TextDirection;
use crate::view::{column, fixed, interactive, row};

use super::*;

// ---------------------------------------------------------------------------
// Bantuan
// ---------------------------------------------------------------------------

/// Semua quad di dalam scene, dalam urutan gambar.
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

/// Tinggi setiap quad — sidik jari scene yang mudah dibaca.
fn tinggi(scene: &Scene) -> Vec<f32> {
    quads(scene).iter().map(|q| q.rect.size.height).collect()
}

/// Aplikasi contoh: satu counter di scope akar, dua komponen anak. Hanya
/// komponen `"angka"` yang **membaca** counter-nya.
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

/// Jangkar komponen ke-`i` di bawah kolom akar.
fn jangkar(ui: &AppRuntime, i: usize) -> crate::tree::NodeId {
    let akar = ui.tree().root();
    let kolom = ui.tree().children(akar)[0];
    ui.tree().children(kolom)[i]
}

// ---------------------------------------------------------------------------
// (a)+(b) frame pertama
// ---------------------------------------------------------------------------

#[test]
fn frame_pertama_membangun_pohon_dan_scene() {
    let (mut ui, pegangan) = app_counter();

    // Sebelum frame pertama: pohon masih kosong, tapi frame sudah dijadwalkan.
    assert!(
        !ui.is_idle(),
        "frame pertama harus terjadwal sejak konstruksi"
    );
    assert_eq!(ui.tree().len(), 1, "baru node akar");

    let laporan = ui.frame();

    assert_eq!(laporan.rebuilt, 1, "hanya scope akar yang dibangun");
    // kolom + 2 jangkar komponen + 2 daun.
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
// (c)+(d) perubahan signal → rebuild subtree saja
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

    // (d) — satu scope dibangun ulang, dan tidak ada node baru sama sekali.
    assert_eq!(laporan.rebuilt, 1, "hanya komponen 'angka'");
    assert_eq!(laporan.diff.created, 0);
    assert_eq!(laporan.diff.replaced, 0);
    assert_eq!(laporan.diff.removed, 0);
    assert_eq!(laporan.diff.moved, 0);
    assert_eq!(laporan.diff.reused, 1, "hanya daun 'angka' yang di-diff");
    assert_eq!(laporan.diff.updated, 1);

    // Identitas node tetangga bertahan: ia tidak pernah masuk jalur diff.
    assert_eq!(jangkar(&ui, 0), jangkar_judul);
    assert_eq!(jangkar(&ui, 1), jangkar_angka);
    assert_eq!(ui.tree().children(jangkar_judul)[0], daun_judul);

    // Scene berubah persis sesuai nilai baru.
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
    // Kontrak `drain_dirty`: pemangkasan keturunan hanya sah bila membangun
    // ulang leluhur benar-benar memasuki lagi setiap anaknya.
    let masuk = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let catat = masuk.clone();
    let pegangan: Rc<Cell<Option<Signal<i32>>>> = Rc::default();
    let simpan = pegangan.clone();

    let mut ui = app(move |_cx| {
        let akar = use_signal(|| 0i32);
        simpan.set(Some(akar));
        let _ = akar.get(); // akar sendiri yang berlangganan
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
// (e) idle = nol
// ---------------------------------------------------------------------------

#[test]
fn tanpa_perubahan_signal_tidak_ada_frame_terjadwal() {
    let (mut ui, pegangan) = app_counter();
    ui.frame();

    // (e) — sesudah frame pertama, benar-benar idle.
    assert!(ui.is_idle());
    assert_eq!(ui.pending(), Dirty::NONE);

    // Membaca signal tidak membangunkan apa pun.
    let count = pegangan.get().unwrap();
    assert_eq!(count.peek(), 0);
    assert!(ui.is_idle());

    // Menulis nilai yang sama juga tidak.
    assert!(!count.set_if_changed(0));
    assert!(
        ui.is_idle(),
        "nilai yang tidak berubah tidak menjadwalkan frame"
    );

    // Baru perubahan sungguhan yang menjadwalkan.
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

    // Frame yang diminta OS (expose) boleh terjadi walau tidak ada yang dirty.
    let laporan = ui.frame();
    assert!(laporan.is_noop());
    assert_eq!(laporan.reason, Dirty::NONE);
    assert_eq!(tinggi(ui.scene()), sebelum);
    assert!(ui.is_idle());
}

// ---------------------------------------------------------------------------
// Sambungan ke scheduler
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

    // Tulisan kedua ke signal yang sama tidak menandai scope baru mana pun,
    // jadi ia tidak sampai ke scheduler sama sekali — platform tidak di-poke
    // dua kali untuk satu frame yang sama.
    count.set(2);
    assert_eq!(*jejak.borrow(), vec![Wake::Schedule]);

    // Sumber lain yang benar-benar baru pun hanya menemukan frame yang sudah
    // terjadwal.
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
// Identitas & daftar dinamis
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
                    // State lokal per baris: dibuat sekali, ikut kuncinya.
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
    // State ikut kuncinya, bukan posisinya.
    assert_eq!(tinggi(ui.scene()), vec![13.0, 12.0, 11.0]);
}

#[test]
fn daftar_yang_menyusut_membuang_scope_dan_node() {
    // `component()` sudah membuat scope-nya sendiri, jadi `list()` tidak dipakai
    // di sini — kalau keduanya dipakai bersama, tiap baris punya dua scope.
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
// Kontrak lain yang ikut lewat siklus ini
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

    // Ukuran & posisi daun harus identik dengan versi tanpa komponen.
    let geometri = |ui: &AppRuntime| -> Vec<(Size, Point)> {
        let akar = ui.tree().root();
        let kolom = ui.tree().children(akar)[0];
        ui.tree()
            .children(kolom)
            .iter()
            .map(|c| {
                // Turun melewati jangkar komponen bila ada.
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
// Animasi: siapa yang menjadwalkan frame berikutnya
// ---------------------------------------------------------------------------

/// Node yang punya satu spring dan bisa diarahkan lewat props — bentuk yang
/// sama dengan panel overlay dan tombol yang masuk keadaan loading.
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
        // Persis yang dilaporkan `OverlayProps::update` saat dialog dibuka:
        // ada yang akan **bergerak**, tapi belum bergerak frame ini.
        Dirty::LAYOUT | Dirty::PAINT | Dirty::ANIMATION
    }
}

/// Majukan seluruh `Bergerak` di pohon — bentuk `rustui_widgets::advance`.
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
    // Regresi: `Dirty::ANIMATION` dari `ViewNode::update` sempat hilang dua
    // kali — di `terapkan_dirty` (tidak pernah sampai ke pohon) dan di akhir
    // `frame` (dibuang bersama tanda lain). Akibatnya dialog yang dibuka lewat
    // signal membeku di frame pertama sampai ada event input berikutnya.
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

    // Siklus shell: animate → frame. `begin_frame` mengosongkan alasan yang
    // menjadwalkan frame ini, jadi yang menjaga animasi tetap hidup adalah
    // `frame` sendiri — bukan permintaan yang sudah dilayani.
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
    // Nilainya benar-benar sampai di tujuan, bukan berhenti di tengah jalan.
    let node = ui.tree().children(ui.tree().root())[0];
    let posisi = ui
        .tree()
        .node_ref::<Bergerak>(node)
        .unwrap()
        .nilai
        .position();
    assert_eq!(posisi, 50.0);
}
