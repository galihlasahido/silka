//! End-to-end tests for tracking, dirty marking, batching, and scope identity.
//! All pure logic — no GPU, no window.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::*;
use crate::scheduler::{Dirty, FrameScheduler};

/// Records how many times `on_wake` fired, along with the last reason.
#[derive(Clone, Default)]
struct Bangun {
    jumlah: Rc<Cell<u32>>,
    terakhir: Rc<Cell<Dirty>>,
}

impl Bangun {
    fn pasang(rt: &Runtime) -> Self {
        let b = Bangun::default();
        let c = b.clone();
        rt.on_wake(move |d| {
            c.jumlah.set(c.jumlah.get() + 1);
            c.terakhir.set(d);
        });
        b
    }

    fn jumlah(&self) -> u32 {
        self.jumlah.get()
    }
}

fn hitung() -> Rc<Cell<u32>> {
    Rc::new(Cell::new(0))
}

// ---------------------------------------------------------------------------
// Hooks / component-local state
// ---------------------------------------------------------------------------

#[test]
fn use_signal_membuat_state_sekali_dan_bertahan_lintas_rebuild() {
    let rt = Runtime::new();
    let init = hitung();
    let terlihat: Rc<RefCell<Vec<i32>>> = Rc::new(RefCell::new(Vec::new()));

    let i = init.clone();
    let t = terlihat.clone();
    let body = move || {
        let s = use_signal(|| {
            i.set(i.get() + 1);
            10i32
        });
        t.borrow_mut().push(s.peek());
        s
    };

    let a = rt.build_root(&body);
    a.set(42);
    let b = rt.build_root(body);

    assert_eq!(a, b, "hook yang sama harus mengembalikan signal yang sama");
    assert_eq!(init.get(), 1, "init hanya berjalan pada build pertama");
    assert_eq!(*terlihat.borrow(), vec![10, 42], "state bertahan");
    assert_eq!(rt.hook_count(rt.root()), 1);
}

#[test]
#[should_panic(expected = "jumlah use_signal berubah")]
fn hook_kondisional_ditolak() {
    let rt = Runtime::new();
    let dua = Rc::new(Cell::new(true));
    let d = dua.clone();
    let body = move || {
        use_signal(|| 1i32);
        if d.get() {
            use_signal(|| 2i32);
        }
    };
    rt.build_root(&body);
    dua.set(false);
    rt.build_root(body);
}

#[test]
#[should_panic(expected = "urutan use_signal berubah")]
fn hook_yang_berganti_tipe_ditolak() {
    let rt = Runtime::new();
    let pertama = Rc::new(Cell::new(true));
    let p = pertama.clone();
    let body = move || {
        if p.get() {
            use_signal(|| 1i32);
        } else {
            use_signal(String::new);
        }
    };
    rt.build_root(&body);
    pertama.set(false);
    rt.build_root(body);
}

#[test]
#[should_panic(expected = "use_signal hanya boleh dipanggil saat komponen dibangun")]
fn use_signal_di_luar_build_ditolak() {
    let _rt = Runtime::new();
    use_signal(|| 0i32);
}

#[test]
fn current_scope_hanya_terisi_saat_membangun() {
    let rt = Runtime::new();
    assert_eq!(current_scope(), None);
    rt.build_root(|| {
        assert_eq!(current_scope(), Some(rt.root()));
        scope("anak", || {
            assert_ne!(current_scope(), Some(rt.root()));
        });
        assert_eq!(current_scope(), Some(rt.root()));
    });
    assert_eq!(current_scope(), None);
}

// ---------------------------------------------------------------------------
// Dependency tracking
// ---------------------------------------------------------------------------

#[test]
fn membaca_saat_build_berlangganan() {
    let rt = Runtime::new();
    let s = rt.signal(0i32);
    assert_eq!(rt.subscriber_count(s.id()), 0);

    rt.build_root(|| {
        let _ = s.get();
    });
    assert_eq!(rt.subscriber_count(s.id()), 1);
    assert_eq!(rt.dependency_count(rt.root()), 1);
}

#[test]
fn membaca_di_luar_build_tidak_berlangganan() {
    let rt = Runtime::new();
    let s = rt.signal(0i32);
    assert_eq!(s.get(), 0);
    s.with(|v| assert_eq!(*v, 0));
    assert_eq!(
        rt.subscriber_count(s.id()),
        0,
        "event handler tidak boleh jadi pembaca"
    );
}

#[test]
fn peek_tidak_pernah_berlangganan() {
    let rt = Runtime::new();
    let s = rt.signal(1i32);
    rt.build_root(|| {
        assert_eq!(s.peek(), 1);
        s.peek_with(|v| assert_eq!(*v, 1));
    });
    assert_eq!(rt.subscriber_count(s.id()), 0);
}

#[test]
fn untracked_membatalkan_pelacakan() {
    let rt = Runtime::new();
    let dilacak = rt.signal(1i32);
    let diam = rt.signal(2i32);
    rt.build_root(|| {
        let _ = dilacak.get();
        untracked(|| {
            let _ = diam.get();
        });
    });
    assert_eq!(rt.subscriber_count(dilacak.id()), 1);
    assert_eq!(rt.subscriber_count(diam.id()), 0);
    assert_eq!(rt.dependency_count(rt.root()), 1);
}

#[test]
fn langganan_tidak_ganda_walau_dibaca_berkali_kali() {
    let rt = Runtime::new();
    let s = rt.signal(0i32);
    rt.build_root(|| {
        let _ = s.get();
        let _ = s.get();
        s.with(|_| {});
    });
    assert_eq!(rt.subscriber_count(s.id()), 1);
    assert_eq!(rt.dependency_count(rt.root()), 1);
}

#[test]
fn langganan_basi_dilepas_saat_rebuild() {
    let rt = Runtime::new();
    let a = rt.signal(1i32);
    let b = rt.signal(2i32);
    let baca_a = Rc::new(Cell::new(true));
    let f = baca_a.clone();
    let body = move || {
        if f.get() {
            let _ = a.get();
        } else {
            let _ = b.get();
        }
    };

    rt.build_root(&body);
    assert_eq!(rt.subscriber_count(a.id()), 1);
    assert_eq!(rt.subscriber_count(b.id()), 0);

    baca_a.set(false);
    rt.build_root(body);
    assert_eq!(
        rt.subscriber_count(a.id()),
        0,
        "komponen yang berhenti membaca harus berhenti dibangunkan"
    );
    assert_eq!(rt.subscriber_count(b.id()), 1);

    a.set(99);
    assert!(!rt.is_dirty(rt.root()));
    b.set(99);
    assert!(rt.is_dirty(rt.root()));
}

#[test]
fn dua_runtime_di_satu_thread_tidak_saling_mengotori() {
    let a = Runtime::new();
    let b = Runtime::new();
    let sa = a.signal(1i32);
    let sb = b.signal(2i32);

    b.build_root(|| {
        let _ = sa.get();
        let _ = sb.get();
    });

    assert_eq!(
        a.subscriber_count(sa.id()),
        0,
        "langganan lintas-runtime tidak dicatat"
    );
    assert_eq!(b.subscriber_count(sb.id()), 1);
    assert_eq!(sa.peek(), 1);
    assert_eq!(sb.peek(), 2);
}

// ---------------------------------------------------------------------------
// Dirty marking
// ---------------------------------------------------------------------------

#[test]
fn signal_tanpa_pembaca_tidak_membangunkan_renderer() {
    let rt = Runtime::new();
    let bangun = Bangun::pasang(&rt);
    let s = rt.signal(0i32);
    s.set(1);
    s.set(2);
    assert_eq!(bangun.jumlah(), 0, "render hanya saat benar-benar dirty");
    assert_eq!(rt.dirty_len(), 0);
    assert_eq!(s.peek(), 2);
}

#[test]
fn menulis_menandai_pembacanya_dirty_dan_membangunkan_sekali() {
    let rt = Runtime::new();
    let bangun = Bangun::pasang(&rt);
    let s = rt.signal(0i32);
    rt.build_root(|| {
        let _ = s.get();
    });

    s.set(1);
    assert!(rt.is_dirty(rt.root()));
    assert_eq!(bangun.jumlah(), 1);
    assert_eq!(bangun.terakhir.get(), SIGNAL_DIRTY);
    assert_eq!(rt.dirty_len(), 1, "tidak ada entri dirty ganda");
}

#[test]
fn update_menandai_dirty() {
    let rt = Runtime::new();
    let s = rt.signal(vec![1i32, 2]);
    rt.build_root(|| {
        s.with(|v| assert_eq!(v.len(), 2));
    });
    s.update(|v| v.push(3));
    assert!(rt.is_dirty(rt.root()));
    assert_eq!(s.peek().len(), 3);
}

#[test]
fn replace_mengembalikan_nilai_lama() {
    let rt = Runtime::new();
    let s = rt.signal(String::from("lama"));
    assert_eq!(s.replace("baru".into()), "lama");
    assert_eq!(s.peek(), "baru");
}

#[test]
fn set_if_changed_diam_saat_nilainya_sama() {
    let rt = Runtime::new();
    let bangun = Bangun::pasang(&rt);
    let s = rt.signal(7i32);
    rt.build_root(|| {
        let _ = s.get();
    });

    assert!(!s.set_if_changed(7));
    assert!(!rt.is_dirty(rt.root()));
    assert_eq!(bangun.jumlah(), 0);

    assert!(s.set_if_changed(8));
    assert!(rt.is_dirty(rt.root()));
    assert_eq!(bangun.jumlah(), 1);
}

// ---------------------------------------------------------------------------
// Batching
// ---------------------------------------------------------------------------

#[test]
fn batch_membangunkan_renderer_sekali_untuk_banyak_tulisan() {
    let p = pohon();
    let bangun = Bangun::pasang(&p.rt);

    p.rt.batch(|| {
        p.s_kiri.set(1);
        p.s_kanan.set(2);
        p.s_bawah.set(3);
        assert_eq!(p.s_kiri.peek(), 1, "nilai berubah seketika, bukan di akhir");
        assert!(p.rt.is_dirty(p.kiri), "dirty juga ditandai seketika");
        assert_eq!(bangun.jumlah(), 0, "yang ditunda hanya pemberitahuannya");
    });

    assert_eq!(bangun.jumlah(), 1, "tiga komponen dirty, satu kali bangun");
    assert_eq!(p.rt.drain_dirty(), vec![p.kiri, p.kanan]);
}

#[test]
fn tanpa_batch_setiap_komponen_baru_membangunkan() {
    let p = pohon();
    let bangun = Bangun::pasang(&p.rt);
    p.s_kiri.set(1);
    p.s_kanan.set(2);
    p.s_bawah.set(3);
    assert_eq!(bangun.jumlah(), 3);
}

#[test]
fn menandai_scope_yang_sudah_dirty_tidak_memoke_platform_lagi() {
    let p = pohon();
    let bangun = Bangun::pasang(&p.rt);
    p.s_kanan.set(1);
    p.s_kanan.set(2);
    assert_eq!(
        bangun.jumlah(),
        1,
        "frame sudah dijadwalkan — tidak perlu dipoke dua kali"
    );
    assert_eq!(p.rt.dirty_len(), 1);

    p.rt.drain_dirty();
    p.s_kanan.set(3);
    assert_eq!(
        bangun.jumlah(),
        2,
        "setelah antrean dilayani, tulisan baru menjadwalkan lagi"
    );
}

#[test]
fn batch_bersarang_flush_hanya_di_batch_terluar() {
    let rt = Runtime::new();
    let bangun = Bangun::pasang(&rt);
    let s = rt.signal(0i32);
    rt.build_root(|| {
        let _ = s.get();
    });

    rt.batch(|| {
        assert!(rt.is_batching());
        s.set(1);
        rt.batch(|| {
            s.set(2);
        });
        assert_eq!(bangun.jumlah(), 0, "batch dalam tidak boleh flush");
    });
    assert!(!rt.is_batching());
    assert_eq!(bangun.jumlah(), 1);
}

#[test]
fn batch_tanpa_perubahan_tidak_membangunkan() {
    let rt = Runtime::new();
    let bangun = Bangun::pasang(&rt);
    rt.batch(|| {});
    assert_eq!(bangun.jumlah(), 0);
}

#[test]
fn batch_mengembalikan_nilai_dan_tetap_flush_saat_panik() {
    let rt = Runtime::new();
    let bangun = Bangun::pasang(&rt);
    let s = rt.signal(0i32);
    rt.build_root(|| {
        let _ = s.get();
    });

    assert_eq!(rt.batch(|| 5), 5);

    let hasil = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rt.batch(|| {
            s.set(1);
            panic!("boom");
        })
    }));
    assert!(hasil.is_err());
    assert!(!rt.is_batching(), "kedalaman batch harus pulih");
    assert_eq!(
        bangun.jumlah(),
        1,
        "dirty yang sudah tercatat tetap dibayar"
    );
}

// ---------------------------------------------------------------------------
// Dirty queue & per-component rebuild
// ---------------------------------------------------------------------------

/// Test tree: root → (`kiri` → `kiri.bawah`), `kanan`.
struct Pohon {
    rt: Runtime,
    kiri: ScopeId,
    bawah: ScopeId,
    kanan: ScopeId,
    s_root: Signal<i32>,
    s_kiri: Signal<i32>,
    s_bawah: Signal<i32>,
    s_kanan: Signal<i32>,
}

fn pohon() -> Pohon {
    let rt = Runtime::new();
    let s_root = rt.signal(0i32);
    let s_kiri = rt.signal(0i32);
    let s_bawah = rt.signal(0i32);
    let s_kanan = rt.signal(0i32);
    rt.build_root(|| {
        let _ = s_root.get();
        scope("kiri", || {
            let _ = s_kiri.get();
            scope("bawah", || {
                let _ = s_bawah.get();
            });
        });
        scope("kanan", || {
            let _ = s_kanan.get();
        });
    });
    let anak = rt.children(rt.root());
    let kiri = anak[0];
    let kanan = anak[1];
    let bawah = rt.children(kiri)[0];
    Pohon {
        rt,
        kiri,
        bawah,
        kanan,
        s_root,
        s_kiri,
        s_bawah,
        s_kanan,
    }
}

#[test]
fn kedalaman_dan_induk_mengikuti_bentuk_pohon() {
    let p = pohon();
    assert_eq!(p.rt.depth(p.rt.root()), Some(0));
    assert_eq!(p.rt.depth(p.kiri), Some(1));
    assert_eq!(p.rt.depth(p.bawah), Some(2));
    assert_eq!(p.rt.parent(p.bawah), Some(p.kiri));
    assert_eq!(p.rt.parent(p.rt.root()), None);
    assert_eq!(p.rt.key(p.kanan), Some(Key::text("kanan")));
    assert_eq!(p.rt.key(p.rt.root()), Some(Key::Root));
    assert_eq!(p.rt.live_scopes(), 4);
}

#[test]
fn drain_dirty_mengurutkan_dari_akar_ke_daun() {
    let p = pohon();
    p.rt.batch(|| {
        p.s_bawah.set(1);
        p.s_kanan.set(1);
    });
    assert_eq!(p.rt.drain_dirty(), vec![p.kanan, p.bawah]);
}

#[test]
fn drain_dirty_memangkas_keturunan_dari_scope_yang_juga_dirty() {
    let p = pohon();
    p.rt.batch(|| {
        p.s_bawah.set(1);
        p.s_kiri.set(1);
        p.s_kanan.set(1);
    });
    let antre = p.rt.drain_dirty();
    assert_eq!(
        antre,
        vec![p.kiri, p.kanan],
        "membangun ulang `kiri` sudah membangun ulang `bawah`"
    );
    assert!(!p.rt.is_dirty(p.bawah), "tanda yang dipangkas ikut bersih");
}

#[test]
fn dirty_di_akar_menelan_seluruh_pohon() {
    let p = pohon();
    p.rt.batch(|| {
        p.s_root.set(1);
        p.s_kiri.set(1);
        p.s_bawah.set(1);
        p.s_kanan.set(1);
    });
    assert_eq!(p.rt.drain_dirty(), vec![p.rt.root()]);
}

#[test]
fn drain_dirty_membersihkan_antrean() {
    let p = pohon();
    p.s_kanan.set(1);
    assert_eq!(p.rt.drain_dirty(), vec![p.kanan]);
    assert!(p.rt.drain_dirty().is_empty());
    assert!(!p.rt.is_dirty(p.kanan));

    // Once drained, the next write is still recorded.
    p.s_kanan.set(2);
    assert_eq!(p.rt.drain_dirty(), vec![p.kanan]);
}

#[test]
fn drain_dirty_melewatkan_scope_yang_sudah_mati() {
    let p = pohon();
    p.s_kanan.set(1);
    // `kanan` disappears from the tree before it can be serviced.
    let s_root = p.s_root;
    let s_kiri = p.s_kiri;
    let s_bawah = p.s_bawah;
    p.rt.build_root(|| {
        let _ = s_root.get();
        scope("kiri", || {
            let _ = s_kiri.get();
            scope("bawah", || {
                let _ = s_bawah.get();
            });
        });
    });
    assert!(!p.rt.is_scope_alive(p.kanan));
    assert!(p.rt.drain_dirty().is_empty());
}

#[test]
fn rebuild_per_komponen_tidak_menyentuh_saudaranya() {
    let rt = Runtime::new();
    let kiri_signal = rt.signal(0i32);
    let n_kiri = hitung();
    let n_kanan = hitung();

    let nk = n_kiri.clone();
    let nn = n_kanan.clone();
    let bangun_kiri = move || {
        nk.set(nk.get() + 1);
        let _ = kiri_signal.get();
    };
    let bangun_kanan = move || {
        nn.set(nn.get() + 1);
    };

    rt.build_root(|| {
        scope("kiri", &bangun_kiri);
        scope("kanan", bangun_kanan);
    });
    assert_eq!((n_kiri.get(), n_kanan.get()), (1, 1));

    kiri_signal.set(1);
    let antre = rt.drain_dirty();
    assert_eq!(antre.len(), 1);
    for id in antre {
        rt.rebuild(id, &bangun_kiri).expect("scope hidup");
    }
    assert_eq!(
        (n_kiri.get(), n_kanan.get()),
        (2, 1),
        "hanya komponen pembaca yang dibangun ulang"
    );
}

#[test]
fn rebuild_scope_mati_mengembalikan_none() {
    let p = pohon();
    let s_root = p.s_root;
    p.rt.build_root(|| {
        let _ = s_root.get();
    });
    assert!(!p.rt.is_scope_alive(p.kiri));
    assert_eq!(p.rt.rebuild(p.kiri, || 1), None);
}

// ---------------------------------------------------------------------------
// Scope identity & dynamic lists
// ---------------------------------------------------------------------------

fn bangun_list(rt: &Runtime, item: &[i64]) -> Vec<Signal<i64>> {
    rt.build_root(|| {
        list(
            item.iter().copied(),
            |id| Key::num(*id),
            |id| use_signal(|| *id * 10),
        )
    })
}

#[test]
fn anak_dikenali_dari_kunci_bukan_posisi() {
    let rt = Runtime::new();
    let a = bangun_list(&rt, &[1, 2, 3]);
    let awal = rt.children(rt.root());
    a[1].set(999);

    // Insert a new item at the front: existing identities must not shift.
    let b = bangun_list(&rt, &[0, 1, 2, 3]);
    let sesudah = rt.children(rt.root());
    assert_eq!(&sesudah[1..], &awal[..], "scope lama dipakai ulang");
    assert_eq!(b[2].peek(), 999, "state ikut kuncinya, bukan posisinya");
    assert_eq!(b[0].peek(), 0, "item baru dapat state segar");
    assert_eq!(rt.live_scopes(), 5);
}

#[test]
fn menukar_urutan_memindahkan_scope_bukan_state() {
    let rt = Runtime::new();
    let awal_signal = bangun_list(&rt, &[1, 2, 3]);
    awal_signal[0].set(111);
    let awal = rt.children(rt.root());

    let balik = bangun_list(&rt, &[3, 2, 1]);
    assert_eq!(rt.children(rt.root()), vec![awal[2], awal[1], awal[0]]);
    assert_eq!(balik[2].peek(), 111);
    assert_eq!(awal_signal[0], balik[2], "signal-nya benar-benar sama");
    assert_eq!(rt.live_scopes(), 4);
}

#[test]
fn anak_yang_hilang_dibuang_beserta_state_dan_langganannya() {
    let rt = Runtime::new();
    let bersama = rt.signal(0i32);
    let simpan: Rc<RefCell<Vec<Signal<i64>>>> = Rc::new(RefCell::new(Vec::new()));

    let s = simpan.clone();
    let bangun = |item: &[i64]| {
        let s = s.clone();
        rt.build_root(|| {
            s.borrow_mut().clear();
            let v = list(
                item.iter().copied(),
                |id| Key::num(*id),
                |id| {
                    let sig = use_signal(|| *id * 10);
                    let _ = bersama.get();
                    sig
                },
            );
            *s.borrow_mut() = v;
        });
    };

    bangun(&[1, 2, 3]);
    let anak = rt.children(rt.root());
    let sinyal_hilang = simpan.borrow()[1];
    assert_eq!(rt.live_scopes(), 4);
    assert_eq!(rt.live_signals(), 4, "1 signal runtime + 3 milik anak");
    assert_eq!(rt.subscriber_count(bersama.id()), 3);

    bangun(&[1, 3]);
    assert!(!rt.is_scope_alive(anak[1]));
    assert!(!rt.is_signal_alive(sinyal_hilang.id()));
    assert_eq!(rt.live_scopes(), 3);
    assert_eq!(rt.live_signals(), 3);
    assert_eq!(
        rt.subscriber_count(bersama.id()),
        2,
        "langganan anak yang mati harus ikut dilepas"
    );
}

#[test]
fn membuang_subtree_membuang_seluruh_keturunannya() {
    let rt = Runtime::new();
    let tampil = Rc::new(Cell::new(true));
    let t = tampil.clone();
    let body = move || {
        if t.get() {
            scope("cabang", || {
                use_signal(|| 1i32);
                scope("ranting", || {
                    use_signal(|| 2i32);
                });
            });
        }
    };

    rt.build_root(&body);
    assert_eq!(rt.live_scopes(), 3);
    assert_eq!(rt.live_signals(), 2);

    tampil.set(false);
    rt.build_root(body);
    assert_eq!(rt.live_scopes(), 1, "hanya akar yang tersisa");
    assert_eq!(rt.live_signals(), 0, "state keturunan ikut dibuang");
}

#[test]
fn slot_arena_dipakai_ulang_tanpa_menghidupkan_id_lama() {
    let rt = Runtime::new();
    let kunci = Rc::new(Cell::new("a"));
    let k = kunci.clone();
    let body = move || {
        scope(k.get(), || {});
    };

    rt.build_root(&body);
    let lama = rt.children(rt.root())[0];

    // The old child is only freed at the end of the build, so "b" gets a fresh
    // slot ...
    kunci.set("b");
    rt.build_root(&body);
    assert!(!rt.is_scope_alive(lama));

    // ... and the now-free slot of "a" is reused by "c".
    kunci.set("c");
    rt.build_root(body);
    let baru = rt.children(rt.root())[0];

    assert_eq!(lama.index(), baru.index(), "slot arena dipakai ulang");
    assert_ne!(
        lama.generation(),
        baru.generation(),
        "generasi naik supaya ID lama tidak pernah cocok lagi"
    );
    assert!(!rt.is_scope_alive(lama));
    assert!(rt.is_scope_alive(baru));
    assert_eq!(rt.live_scopes(), 2);
}

#[test]
#[should_panic(expected = "kunci ganda")]
fn kunci_ganda_di_antara_saudara_ditolak() {
    let rt = Runtime::new();
    rt.build_root(|| {
        scope("a", || {});
        scope("a", || {});
    });
}

#[test]
fn kunci_yang_sama_di_induk_berbeda_tidak_bentrok() {
    let rt = Runtime::new();
    rt.build_root(|| {
        scope("kiri", || {
            scope("isi", || {});
        });
        scope("kanan", || {
            scope("isi", || {});
        });
    });
    assert_eq!(rt.live_scopes(), 5);
}

#[test]
fn kunci_teks_dan_angka_tidak_pernah_tertukar() {
    assert_ne!(Key::from(7), Key::from("7"));
    assert_eq!(Key::from(7u32), Key::Num(7));
    assert_eq!(Key::from(String::from("x")), Key::text("x"));
    assert_eq!(Key::Num(3).to_string(), "3");
    assert_eq!(format!("{:?}", Key::text("baris")), "Key(\"baris\")");
}

// ---------------------------------------------------------------------------
// Signal lifetimes & errors that must read clearly
// ---------------------------------------------------------------------------

#[test]
fn kepemilikan_signal_terbaca_dari_runtime() {
    let rt = Runtime::new();
    let global = rt.signal(0i32);
    let lokal: Rc<Cell<Option<Signal<i32>>>> = Rc::new(Cell::new(None));
    let l = lokal.clone();
    rt.build_root(move || {
        scope("a", || {
            l.set(Some(use_signal(|| 1i32)));
        });
    });
    let anak = rt.children(rt.root())[0];
    assert_eq!(rt.signal_owner(global.id()), None, "signal milik runtime");
    assert_eq!(rt.signal_owner(lokal.get().unwrap().id()), Some(anak));
}

#[test]
fn signal_milik_runtime_hidup_selama_runtime() {
    let rt = Runtime::new();
    let global = rt.signal(1i32);
    rt.build_root(|| {
        scope("a", || {
            let _ = global.get();
        });
    });
    rt.build_root(|| {});
    assert!(global.is_alive());
    assert_eq!(rt.subscriber_count(global.id()), 0);
    global.set(2);
    assert_eq!(global.peek(), 2);
}

#[test]
#[should_panic(expected = "sudah mati")]
fn signal_yang_scope_pemiliknya_dibuang_dilaporkan_jelas() {
    let rt = Runtime::new();
    let simpan: Rc<Cell<Option<Signal<i32>>>> = Rc::new(Cell::new(None));
    let s = simpan.clone();
    rt.build_root(|| {
        scope("a", || {
            s.set(Some(use_signal(|| 5i32)));
        });
    });
    rt.build_root(|| {});
    let mati = simpan.get().unwrap();
    assert!(!mati.is_alive());
    let _ = mati.get();
}

#[test]
#[should_panic(expected = "akses rekursif")]
fn akses_rekursif_ke_signal_yang_sama_dilaporkan_jelas() {
    let rt = Runtime::new();
    let s = rt.signal(1i32);
    s.with(|_| {
        let _ = s.peek();
    });
}

#[test]
fn membaca_signal_lain_di_dalam_with_tetap_boleh() {
    let rt = Runtime::new();
    let a = rt.signal(1i32);
    let b = rt.signal(2i32);
    let jumlah = a.with(|x| *x + b.peek());
    assert_eq!(jumlah, 3);
}

#[test]
fn signal_copy_masuk_ke_closure_event_handler() {
    let rt = Runtime::new();
    let count = rt.signal(0i32);
    rt.build_root(|| {
        // Read during the build (so it subscribes) ...
        let _teks = format!("Nilai: {}", count.get());
    });
    // ... then used again from a 'static closure, without cloning.
    let on_press: Box<dyn Fn()> = Box::new(move || count.set(count.peek() + 1));
    on_press();
    on_press();
    assert_eq!(count.peek(), 2);
    assert!(rt.is_dirty(rt.root()));
}

// ---------------------------------------------------------------------------
// Wiring to the scheduler
// ---------------------------------------------------------------------------

#[test]
fn signal_membangunkan_frame_scheduler_dan_diam_saat_tidak_ada_pembaca() {
    let rt = Runtime::new();
    let sched = Rc::new(RefCell::new(FrameScheduler::new()));
    let s = sched.clone();
    rt.on_wake(move |alasan| {
        s.borrow_mut().request(alasan);
    });

    let sunyi = rt.signal(0i32);
    let dibaca = rt.signal(0i32);
    rt.build_root(|| {
        let _ = dibaca.get();
    });

    sunyi.set(1);
    assert!(
        sched.borrow().is_idle(),
        "signal tanpa pembaca tidak boleh membangunkan GPU"
    );

    dibaca.set(1);
    assert!(!sched.borrow().is_idle());
    assert!(sched
        .borrow()
        .pending()
        .contains(Dirty::LAYOUT | Dirty::PAINT));

    // The frame begins: the scheduler's dirty flags clear, the rebuild queue is
    // serviced.
    let start = sched.borrow_mut().begin_frame(std::time::Instant::now());
    assert!(sched.borrow().is_idle());
    assert_eq!(rt.drain_dirty(), vec![rt.root()]);
    sched
        .borrow_mut()
        .end_frame(start, std::time::Instant::now(), true);
}

#[test]
fn debug_runtime_menyebut_isi_arena() {
    let rt = Runtime::new();
    rt.signal(0i32);
    let teks = format!("{rt:?}");
    assert!(teks.contains("scopes: 1"), "{teks}");
    assert!(teks.contains("signals: 1"), "{teks}");
}
