//! `component()` — satu-satunya tempat scope signals bertemu node render.
//!
//! Sebuah komponen adalah pasangan: **scope** di [`crate::signals`] (pemilik
//! `use_signal` dan langganannya) dan **satu node jangkar** di
//! [`crate::tree`] (tempat hasil build-nya di-diff). Node jangkarnya transparan
//! — ia hanya meneruskan constraints ke satu anak — sehingga menambahkan
//! komponen tidak mengubah hasil layout sama sekali.
//!
//! Kenapa jangkarnya perlu ada: `drain_dirty()` memberi `ScopeId`, dan
//! rebuild per-komponen harus tahu **di bawah node mana** view barunya
//! di-diff. Tanpa node itu, satu-satunya pilihan yang tersisa adalah mendiff
//! seluruh pohon dari akar setiap kali sebuah signal berubah — persis yang
//! ingin dihindari §2.5.

use rustui_paint::{Point, Size};

use crate::access::{AccessNode, AccessRole};
use crate::scheduler::Dirty;
use crate::signals::{current_scope, scope as masuk_scope, Key, ScopeId};
use crate::tree::{BoxConstraints, LayoutCtx, RenderNode};
use crate::view::{View, ViewNode};

use super::host::{current_host, BuildCtx, ComponentBuilder};

// ---------------------------------------------------------------------------
// Node render
// ---------------------------------------------------------------------------

/// Node jangkar sebuah komponen: transparan bagi layout, penanda bagi rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentBox {
    /// Scope signals yang membangun isi node ini.
    pub scope: ScopeId,
}

impl RenderNode for ComponentBox {
    fn type_name(&self) -> &'static str {
        "Component"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let child = ctx.child(0);
        // `layout_child_measured`, bukan `layout_child`: node ini transparan,
        // jadi ketatnya constraints yang lewat di sini **bukan** miliknya. Kalau
        // ia menjadikan anaknya relayout boundary, perubahan di dalam komponen
        // tidak akan pernah sampai ke wadah flex/grid di atasnya (lihat
        // `LayoutCtx::layout_child_measured`).
        let size = ctx.layout_child_measured(child, constraints);
        ctx.place_child(child, Point::ZERO);
        size
    }

    /// Murni struktural: teknologi bantu menyaringnya keluar dan anaknya naik
    /// menggantikannya (§3.8). Batas komponen adalah urusan framework, bukan
    /// sesuatu yang perlu dibacakan screen reader.
    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Props node jangkar komponen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentProps {
    scope: ScopeId,
}

impl ComponentProps {
    /// Scope yang dijangkarkan props ini.
    pub fn scope(&self) -> ScopeId {
        self.scope
    }
}

impl ViewNode for ComponentProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(ComponentBox { scope: self.scope })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<ComponentBox>()
            .expect("tipe view sama berarti tipe render node sama");
        if n.scope == self.scope {
            return Dirty::NONE;
        }
        // Kunci yang sama tapi scope berbeda berarti scope lama sudah mati dan
        // digantikan penghuni baru — isinya wajib dianggap baru seluruhnya.
        n.scope = self.scope;
        Dirty::LAYOUT | Dirty::PAINT
    }
}

/// Bangun satu komponen ber-`key` dan jadikan hasilnya sebuah [`View`].
///
/// Inilah bentuk gaya Dart untuk "bagian UI yang punya state sendiri dan
/// dibangun ulang sendiri" (§2.5):
///
/// ```
/// use rustui_core::app::{app, component};
/// use rustui_core::signals::use_signal;
/// use rustui_core::view::{column, fixed};
///
/// let mut ui = app(|_cx| {
///     column([
///         component("kiri", |_| fixed(40.0, 20.0).into()),
///         component("kanan", |_| {
///             let n = use_signal(|| 1i32);
///             fixed(40.0, 20.0 * n.get() as f32).into()
///         }),
///     ])
///     .into()
/// })
/// .sized(200.0, 200.0);
/// ui.frame();
/// ```
///
/// Tiga hal yang terjadi sekaligus, dan ketiganya wajib:
///
/// 1. **Masuk scope** ([`crate::signals::scope`]) — `key` yang sama pada build
///    berikutnya = scope yang sama = state yang sama, walau posisinya bergeser.
/// 2. **Bangun isinya sekarang juga.** Ini yang memenuhi kontrak
///    [`crate::signals::Runtime::drain_dirty`]: membangun ulang sebuah scope
///    **memasuki kembali setiap anak yang dipertahankan**, sehingga
///    pemangkasan keturunan pada daftar dirty tetap sah.
/// 3. **Simpan closure-nya** di registry host, supaya frame berikutnya bisa
///    membangun ulang komponen ini **saja**.
///
/// Panik bila dipanggil di luar build sebuah [`crate::app::AppRuntime`].
pub fn component<F>(key: impl Into<Key>, body: F) -> View
where
    F: Fn(&BuildCtx) -> View + 'static,
{
    let host = current_host().expect(
        "component() hanya boleh dipanggil saat komponen dibangun (di dalam AppRuntime::frame)",
    );
    let key: Key = key.into();
    let builder: ComponentBuilder = std::rc::Rc::new(body);

    let cx = BuildCtx::new(host.clone());
    let untuk_scope = builder.clone();
    let (scope, isi) = masuk_scope(key.clone(), move || {
        let id = current_scope().expect("scope() baru saja memasuki scope anak");
        (id, untuk_scope(&cx))
    });

    host.register(scope, builder);

    crate::view::Builder::new(ComponentProps { scope })
        .key(key)
        .child(isi)
        .into()
}
