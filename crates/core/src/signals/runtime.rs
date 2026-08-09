//! Mesin runtime signals: arena scope, arena signal, dependency tracking,
//! dirty marking, dan batching.
//!
//! Modul ini adalah **detail implementasi**. Yang dilihat penulis widget hanya
//! [`super::use_signal`], [`super::scope`], dan [`super::Signal`].
//!
//! Dua arena, keduanya ber-ID bergenerasi (`index` + `generation`) persis
//! seperti render tree dan AccessKit (REKOMENDASI §2): ID yang sudah mati tidak
//! pernah tertukar dengan penghuni baru di slot yang sama.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::{Rc, Weak};

use super::Key;
use crate::scheduler::Dirty;

/// Alasan dirty yang dikirim ke scheduler saat sebuah signal berubah.
///
/// Perubahan signal berarti komponen dibangun ulang → view-diff → ukuran bisa
/// berubah, tampilan pasti berubah. Karena itu keduanya, bukan `PAINT` saja.
pub const SIGNAL_DIRTY: Dirty = Dirty::LAYOUT.union(Dirty::PAINT);

// ---------------------------------------------------------------------------
// Thread-local: daftar runtime + tumpukan build
// ---------------------------------------------------------------------------

thread_local! {
    static TLS: RefCell<Tls> = RefCell::new(Tls::default());
}

#[derive(Default)]
struct Tls {
    /// Runtime hidup di thread ini. Disimpan sebagai `Weak` supaya runtime
    /// tetap mati saat handle terakhirnya di-drop (tidak ada kebocoran).
    runtimes: Vec<(RuntimeId, Weak<RuntimeInner>)>,
    /// Tumpukan scope yang sedang dibangun. `None` = pembatas
    /// [`super::untracked`]: pembacaan di atasnya tidak berlangganan apa pun.
    building: Vec<Option<(RuntimeId, ScopeId)>>,
    next_runtime: u32,
}

fn alloc_runtime_id() -> RuntimeId {
    TLS.with(|t| {
        let mut t = t.borrow_mut();
        let id = RuntimeId(t.next_runtime);
        t.next_runtime += 1;
        id
    })
}

/// Scope yang sedang dibangun di thread ini, bila ada.
pub(crate) fn current_build() -> Option<(RuntimeId, ScopeId)> {
    TLS.with(|t| t.borrow().building.last().copied().flatten())
}

fn runtime_by_id(id: RuntimeId) -> Option<Runtime> {
    let inner = TLS.with(|t| {
        t.borrow()
            .runtimes
            .iter()
            .find(|(i, _)| *i == id)
            .and_then(|(_, w)| w.upgrade())
    })?;
    Some(Runtime { inner })
}

/// Runtime pemilik `id`; panik bila runtime-nya sudah mati.
pub(crate) fn runtime_of(id: SignalId) -> Runtime {
    runtime_by_id(id.rt).unwrap_or_else(|| {
        panic!("signal {id:?} dipakai setelah runtime-nya mati (atau di thread lain)")
    })
}

/// Penjaga tumpukan build — memastikan tumpukan tetap benar walau body panik.
struct BuildGuard;

impl BuildGuard {
    fn push(entry: Option<(RuntimeId, ScopeId)>) -> Self {
        TLS.with(|t| t.borrow_mut().building.push(entry));
        BuildGuard
    }
}

impl Drop for BuildGuard {
    fn drop(&mut self) {
        let _ = TLS.try_with(|t| {
            t.borrow_mut().building.pop();
        });
    }
}

/// Jalankan `f` tanpa berlangganan apa pun (pembatas untracked).
pub(crate) fn run_untracked<R>(f: impl FnOnce() -> R) -> R {
    let _g = BuildGuard::push(None);
    f()
}

// ---------------------------------------------------------------------------
// ID
// ---------------------------------------------------------------------------

/// Identitas satu runtime di dalam thread-nya.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RuntimeId(u32);

/// Identitas satu scope komponen di arena runtime.
///
/// Bergenerasi: setelah scope-nya mati, ID lama tidak akan pernah cocok lagi
/// dengan scope baru yang menempati slot yang sama.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopeId {
    rt: RuntimeId,
    index: u32,
    generation: u32,
}

impl ScopeId {
    /// Nomor slot arena (stabil hanya selama scope hidup).
    pub fn index(self) -> u32 {
        self.index
    }

    /// Generasi slot — pembeda antara penghuni lama dan baru.
    pub fn generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Debug for ScopeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Scope(#{}v{})", self.index, self.generation)
    }
}

/// Identitas satu signal di arena runtime (bergenerasi, sama seperti [`ScopeId`]).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignalId {
    rt: RuntimeId,
    index: u32,
    generation: u32,
}

impl SignalId {
    /// Nomor slot arena.
    pub fn index(self) -> u32 {
        self.index
    }

    /// Generasi slot.
    pub fn generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Debug for SignalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signal(#{}v{})", self.index, self.generation)
    }
}

// ---------------------------------------------------------------------------
// Slot arena
// ---------------------------------------------------------------------------

struct Hook {
    signal: SignalId,
    type_id: TypeId,
}

struct ScopeSlot {
    generation: u32,
    alive: bool,
    parent: Option<ScopeId>,
    depth: u32,
    key: Key,
    /// Anak-anak dalam urutan kunjungan build terakhir.
    children: Vec<ScopeId>,
    /// State milik scope ini (`use_signal`), diakses per urutan pemanggilan.
    hooks: Vec<Hook>,
    hook_cursor: usize,
    /// Signal yang dibaca pada build terakhir — inilah dependency tracking.
    deps: Vec<SignalId>,
    dirty: bool,
    building: bool,
    /// Anak dari build sebelumnya yang belum dicocokkan (rekonsiliasi kunci).
    old_children: HashMap<Key, ScopeId>,
    /// Anak yang sudah dikunjungi pada build yang sedang berjalan.
    new_children: Vec<ScopeId>,
    /// Kunci yang sudah dipakai pada build ini — pendeteksi kunci ganda.
    seen_keys: HashSet<Key>,
}

impl ScopeSlot {
    fn new(parent: Option<ScopeId>, depth: u32, key: Key) -> Self {
        Self {
            generation: 0,
            alive: true,
            parent,
            depth,
            key,
            children: Vec::new(),
            hooks: Vec::new(),
            hook_cursor: 0,
            deps: Vec::new(),
            dirty: false,
            building: false,
            old_children: HashMap::new(),
            new_children: Vec::new(),
            seen_keys: HashSet::new(),
        }
    }
}

struct SignalSlot {
    generation: u32,
    alive: bool,
    /// `None` saat nilainya sedang dipinjam keluar (lihat [`ValueGuard`]).
    value: Option<Box<dyn Any>>,
    /// Scope yang membaca signal ini pada build terakhirnya.
    subscribers: Vec<ScopeId>,
    /// Scope pemilik bila lahir dari `use_signal`; `None` bila milik runtime.
    owner: Option<ScopeId>,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

struct State {
    scopes: Vec<ScopeSlot>,
    free_scopes: Vec<u32>,
    live_scopes: usize,
    signals: Vec<SignalSlot>,
    free_signals: Vec<u32>,
    live_signals: usize,
    dirty: Vec<ScopeId>,
    batch_depth: u32,
    wake_pending: bool,
}

impl State {
    fn scope(&self, id: ScopeId) -> Option<&ScopeSlot> {
        self.scopes
            .get(id.index as usize)
            .filter(|s| s.alive && s.generation == id.generation)
    }

    fn scope_mut(&mut self, id: ScopeId) -> Option<&mut ScopeSlot> {
        self.scopes
            .get_mut(id.index as usize)
            .filter(|s| s.alive && s.generation == id.generation)
    }

    fn signal(&self, id: SignalId) -> Option<&SignalSlot> {
        self.signals
            .get(id.index as usize)
            .filter(|s| s.alive && s.generation == id.generation)
    }

    fn signal_mut(&mut self, id: SignalId) -> Option<&mut SignalSlot> {
        self.signals
            .get_mut(id.index as usize)
            .filter(|s| s.alive && s.generation == id.generation)
    }

    fn alloc_scope(
        &mut self,
        rt: RuntimeId,
        parent: Option<ScopeId>,
        depth: u32,
        key: Key,
    ) -> ScopeId {
        self.live_scopes += 1;
        if let Some(index) = self.free_scopes.pop() {
            let slot = &mut self.scopes[index as usize];
            slot.alive = true;
            slot.parent = parent;
            slot.depth = depth;
            slot.key = key;
            slot.dirty = false;
            slot.building = false;
            slot.hook_cursor = 0;
            return ScopeId {
                rt,
                index,
                generation: slot.generation,
            };
        }
        let index = self.scopes.len() as u32;
        self.scopes.push(ScopeSlot::new(parent, depth, key));
        ScopeId {
            rt,
            index,
            generation: 0,
        }
    }

    fn alloc_signal(
        &mut self,
        rt: RuntimeId,
        value: Box<dyn Any>,
        owner: Option<ScopeId>,
    ) -> SignalId {
        self.live_signals += 1;
        if let Some(index) = self.free_signals.pop() {
            let slot = &mut self.signals[index as usize];
            slot.alive = true;
            slot.value = Some(value);
            slot.owner = owner;
            slot.subscribers.clear();
            return SignalId {
                rt,
                index,
                generation: slot.generation,
            };
        }
        let index = self.signals.len() as u32;
        self.signals.push(SignalSlot {
            generation: 0,
            alive: true,
            value: Some(value),
            subscribers: Vec::new(),
            owner,
        });
        SignalId {
            rt,
            index,
            generation: 0,
        }
    }

    /// Bebaskan satu signal dan lepaskan seluruh langganannya.
    fn free_signal(&mut self, id: SignalId) {
        let Some(slot) = self.signal_mut(id) else {
            return;
        };
        let subs = std::mem::take(&mut slot.subscribers);
        slot.value = None;
        slot.alive = false;
        slot.owner = None;
        slot.generation = slot.generation.wrapping_add(1);
        self.live_signals -= 1;
        self.free_signals.push(id.index);
        for s in subs {
            if let Some(scope) = self.scope_mut(s) {
                scope.deps.retain(|d| *d != id);
            }
        }
    }

    /// Bebaskan satu scope beserta seluruh subtree, hook, dan langganannya.
    fn free_scope(&mut self, id: ScopeId) {
        let Some(slot) = self.scope_mut(id) else {
            return;
        };
        let children = std::mem::take(&mut slot.children);
        let leftovers: Vec<ScopeId> = slot.old_children.drain().map(|(_, v)| v).collect();
        let pending = std::mem::take(&mut slot.new_children);
        let hooks = std::mem::take(&mut slot.hooks);
        let deps = std::mem::take(&mut slot.deps);
        slot.seen_keys.clear();
        slot.alive = false;
        slot.dirty = false;
        slot.building = false;
        slot.hook_cursor = 0;
        slot.parent = None;
        slot.generation = slot.generation.wrapping_add(1);
        self.live_scopes -= 1;
        self.free_scopes.push(id.index);

        for d in deps {
            if let Some(sig) = self.signal_mut(d) {
                sig.subscribers.retain(|s| *s != id);
            }
        }
        for h in hooks {
            self.free_signal(h.signal);
        }
        for c in children.into_iter().chain(leftovers).chain(pending) {
            self.free_scope(c);
        }
    }
}

// ---------------------------------------------------------------------------
// RuntimeInner
// ---------------------------------------------------------------------------

/// Pemberitahu "ada yang dirty" milik platform ([`Runtime::on_wake`]).
type Waker = Rc<dyn Fn(Dirty)>;

struct RuntimeInner {
    id: RuntimeId,
    root: ScopeId,
    state: RefCell<State>,
    wake: RefCell<Option<Waker>>,
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        let id = self.id;
        let _ = TLS.try_with(|t| {
            t.borrow_mut().runtimes.retain(|(i, _)| *i != id);
        });
    }
}

/// Nilai signal yang dipinjam keluar dari arena; dikembalikan saat di-drop.
///
/// Trik ini melepaskan `RefCell` arena sebelum closure pengguna berjalan,
/// sehingga membaca signal lain di dalam `with(...)` tidak memanikkan borrow
/// checker runtime. Akses **rekursif ke signal yang sama** tetap terlarang dan
/// dilaporkan dengan pesan yang jelas, bukan `already borrowed`.
struct ValueGuard<'a> {
    inner: &'a RuntimeInner,
    id: SignalId,
    value: Option<Box<dyn Any>>,
}

impl Drop for ValueGuard<'_> {
    fn drop(&mut self) {
        if let Some(v) = self.value.take() {
            let mut st = self.inner.state.borrow_mut();
            if let Some(slot) = st.signal_mut(self.id) {
                slot.value = Some(v);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

/// Runtime signals: pemilik seluruh state komponen.
///
/// Satu runtime per window/aplikasi. Handle-nya murah di-clone (`Rc` di
/// dalamnya) dan **tidak** `Send`: signals adalah barang UI thread.
///
/// Runtime mendaftarkan dirinya ke thread saat dibuat dan mencabut pendaftaran
/// saat handle terakhirnya di-drop. Itulah yang membuat [`super::Signal`] bisa
/// `Copy` tanpa membawa pointer apa pun — persis pola Dioxus.
#[derive(Clone)]
pub struct Runtime {
    inner: Rc<RuntimeInner>,
}

impl Runtime {
    /// Runtime baru beserta scope akarnya.
    pub fn new() -> Self {
        let id = alloc_runtime_id();
        let mut state = State {
            scopes: Vec::new(),
            free_scopes: Vec::new(),
            live_scopes: 0,
            signals: Vec::new(),
            free_signals: Vec::new(),
            live_signals: 0,
            dirty: Vec::new(),
            batch_depth: 0,
            wake_pending: false,
        };
        let root = state.alloc_scope(id, None, 0, Key::Root);
        let inner = Rc::new(RuntimeInner {
            id,
            root,
            state: RefCell::new(state),
            wake: RefCell::new(None),
        });
        TLS.with(|t| t.borrow_mut().runtimes.push((id, Rc::downgrade(&inner))));
        Self { inner }
    }

    /// Runtime yang scope-nya sedang dibangun di thread ini, bila ada.
    pub fn current() -> Option<Runtime> {
        current_build().and_then(|(rt, _)| runtime_by_id(rt))
    }

    /// Identitas runtime ini.
    pub fn id(&self) -> RuntimeId {
        self.inner.id
    }

    /// Scope akar.
    pub fn root(&self) -> ScopeId {
        self.inner.root
    }

    /// Pasang pemberitahu "ada yang dirty".
    ///
    /// Dipanggil **sekali per flush** (satu kali per batch, bukan sekali per
    /// tulisan) dengan alasan [`SIGNAL_DIRTY`]. Sambungkan langsung ke
    /// [`crate::scheduler::FrameScheduler::request`].
    pub fn on_wake(&self, f: impl Fn(Dirty) + 'static) {
        *self.inner.wake.borrow_mut() = Some(Rc::new(f));
    }

    // -- membangun ---------------------------------------------------------

    /// Bangun (atau bangun ulang) scope akar.
    pub fn build_root<R>(&self, body: impl FnOnce() -> R) -> R {
        self.run_scope(self.inner.root, body)
            .expect("scope akar selalu hidup")
    }

    /// Bangun ulang satu scope saja — inilah "rebuild per-komponen".
    ///
    /// `None` bila scope sudah mati (mis. terhapus dari list sebelum
    /// sempat dilayani).
    pub fn rebuild<R>(&self, id: ScopeId, body: impl FnOnce() -> R) -> Option<R> {
        self.run_scope(id, body)
    }

    pub(crate) fn run_scope<R>(&self, id: ScopeId, body: impl FnOnce() -> R) -> Option<R> {
        self.begin_build(id)?;
        let result = {
            let _g = BuildGuard::push(Some((self.inner.id, id)));
            body()
        };
        self.end_build(id);
        Some(result)
    }

    fn begin_build(&self, id: ScopeId) -> Option<()> {
        let mut st = self.inner.state.borrow_mut();
        let slot = st.scope_mut(id)?;
        assert!(
            !slot.building,
            "{id:?} sudah sedang dibangun — build rekursif tidak diizinkan"
        );
        slot.building = true;
        slot.dirty = false;
        slot.hook_cursor = 0;
        let deps = std::mem::take(&mut slot.deps);
        let children = std::mem::take(&mut slot.children);
        slot.new_children.clear();
        slot.seen_keys.clear();

        // Langganan lama dilepas: komponen yang berhenti membaca sebuah signal
        // harus benar-benar berhenti dibangunkan olehnya.
        for d in deps {
            if let Some(sig) = st.signal_mut(d) {
                sig.subscribers.retain(|s| *s != id);
            }
        }

        let mut old = HashMap::with_capacity(children.len());
        for c in children {
            if let Some(k) = st.scope(c).map(|s| s.key.clone()) {
                old.insert(k, c);
            }
        }
        if let Some(slot) = st.scope_mut(id) {
            slot.old_children = old;
        }
        Some(())
    }

    fn end_build(&self, id: ScopeId) {
        let mut st = self.inner.state.borrow_mut();
        let Some(slot) = st.scope_mut(id) else {
            return;
        };
        let cursor = slot.hook_cursor;
        let hooks = slot.hooks.len();
        slot.building = false;
        let leftovers: Vec<ScopeId> = slot.old_children.drain().map(|(_, v)| v).collect();
        slot.children = std::mem::take(&mut slot.new_children);
        slot.seen_keys.clear();
        for c in leftovers {
            st.free_scope(c);
        }
        drop(st);
        assert_eq!(
            cursor, hooks,
            "jumlah use_signal berubah antar-build di {id:?} — hook tidak boleh dipanggil di dalam if/loop"
        );
    }

    /// Cocokkan (atau buat) anak ber-`key` di bawah scope yang sedang dibangun.
    pub(crate) fn reconcile_child(&self, parent: ScopeId, key: Key) -> ScopeId {
        let mut st = self.inner.state.borrow_mut();
        let slot = st
            .scope_mut(parent)
            .unwrap_or_else(|| panic!("{parent:?} sudah mati"));
        assert!(
            slot.building,
            "scope(...) hanya boleh dipanggil saat komponen dibangun"
        );
        assert!(
            slot.seen_keys.insert(key.clone()),
            "kunci ganda {key:?} di bawah {parent:?} — identitas anak harus unik (REKOMENDASI §2.5)"
        );
        if let Some(existing) = slot.old_children.remove(&key) {
            slot.new_children.push(existing);
            return existing;
        }
        let depth = slot.depth + 1;
        let child = st.alloc_scope(self.inner.id, Some(parent), depth, key);
        if let Some(slot) = st.scope_mut(parent) {
            slot.new_children.push(child);
        }
        child
    }

    // -- hooks -------------------------------------------------------------

    /// Implementasi [`super::use_signal`].
    pub(crate) fn use_signal_hook<T: 'static>(
        &self,
        scope: ScopeId,
        init: impl FnOnce() -> T,
    ) -> SignalId {
        let cursor = {
            let mut st = self.inner.state.borrow_mut();
            let slot = st
                .scope_mut(scope)
                .unwrap_or_else(|| panic!("{scope:?} sudah mati"));
            assert!(
                slot.building,
                "use_signal hanya boleh dipanggil saat komponen dibangun"
            );
            let cursor = slot.hook_cursor;
            slot.hook_cursor += 1;
            if let Some(hook) = slot.hooks.get(cursor) {
                assert!(
                    hook.type_id == TypeId::of::<T>(),
                    "urutan use_signal berubah antar-build di {scope:?} (hook #{cursor})"
                );
                return hook.signal;
            }
            cursor
        };
        // `init` boleh membaca signal lain / memanggil runtime, jadi arena
        // tidak boleh dalam keadaan terpinjam saat ia berjalan.
        let value = init();
        let mut st = self.inner.state.borrow_mut();
        let id = st.alloc_signal(self.inner.id, Box::new(value), Some(scope));
        let slot = st
            .scope_mut(scope)
            .unwrap_or_else(|| panic!("{scope:?} sudah mati"));
        assert_eq!(
            slot.hooks.len(),
            cursor,
            "urutan hook rusak di {scope:?} (hook #{cursor})"
        );
        slot.hooks.push(Hook {
            signal: id,
            type_id: TypeId::of::<T>(),
        });
        id
    }

    /// Signal milik runtime (bukan milik scope) — hidup selama runtime hidup.
    ///
    /// Dipakai untuk state tingkat aplikasi dan untuk pengujian; state lokal
    /// komponen memakai [`super::use_signal`].
    pub fn signal<T: 'static>(&self, value: T) -> super::Signal<T> {
        let mut st = self.inner.state.borrow_mut();
        let id = st.alloc_signal(self.inner.id, Box::new(value), None);
        super::Signal::from_id(id)
    }

    // -- nilai signal ------------------------------------------------------

    fn take_value(&self, id: SignalId) -> ValueGuard<'_> {
        let mut st = self.inner.state.borrow_mut();
        let slot = st
            .signal_mut(id)
            .unwrap_or_else(|| panic!("{id:?} sudah mati — scope pemiliknya sudah dibuang"));
        let value = slot
            .value
            .take()
            .unwrap_or_else(|| panic!("akses rekursif ke {id:?}: nilainya sedang dipinjam"));
        drop(st);
        ValueGuard {
            inner: &self.inner,
            id,
            value: Some(value),
        }
    }

    pub(crate) fn with_value<T: 'static, R>(&self, id: SignalId, f: impl FnOnce(&T) -> R) -> R {
        let guard = self.take_value(id);
        let value = guard
            .value
            .as_ref()
            .expect("nilai ada di guard")
            .downcast_ref::<T>()
            .unwrap_or_else(|| panic!("tipe {id:?} tidak cocok"));
        f(value)
    }

    pub(crate) fn update_value<T: 'static, R>(
        &self,
        id: SignalId,
        f: impl FnOnce(&mut T) -> R,
    ) -> R {
        let result = {
            let mut guard = self.take_value(id);
            let value = guard
                .value
                .as_mut()
                .expect("nilai ada di guard")
                .downcast_mut::<T>()
                .unwrap_or_else(|| panic!("tipe {id:?} tidak cocok"));
            f(value)
        };
        self.notify(id);
        result
    }

    pub(crate) fn replace_value<T: 'static>(&self, id: SignalId, value: T) -> T {
        let old = {
            let mut guard = self.take_value(id);
            let slot = guard
                .value
                .as_mut()
                .expect("nilai ada di guard")
                .downcast_mut::<T>()
                .unwrap_or_else(|| panic!("tipe {id:?} tidak cocok"));
            std::mem::replace(slot, value)
        };
        self.notify(id);
        old
    }

    /// Catat bahwa scope yang sedang dibangun membaca `id`.
    pub(crate) fn track(&self, id: SignalId) {
        let Some((rt, scope)) = current_build() else {
            return;
        };
        if rt != self.inner.id {
            return;
        }
        let mut st = self.inner.state.borrow_mut();
        if st.signal(id).is_none() {
            return;
        }
        let baru = match st.scope_mut(scope) {
            Some(slot) => {
                if slot.deps.contains(&id) {
                    false
                } else {
                    slot.deps.push(id);
                    true
                }
            }
            None => false,
        };
        if baru {
            if let Some(sig) = st.signal_mut(id) {
                if !sig.subscribers.contains(&scope) {
                    sig.subscribers.push(scope);
                }
            }
        }
    }

    // -- dirty & batching ---------------------------------------------------

    /// Tandai semua pembaca `id` sebagai dirty, lalu bangunkan (sekali).
    pub(crate) fn notify(&self, id: SignalId) {
        let flush = {
            let mut st = self.inner.state.borrow_mut();
            let Some(sig) = st.signal(id) else {
                return;
            };
            let subs = sig.subscribers.clone();
            let mut marked = false;
            for s in subs {
                let perlu = match st.scope_mut(s) {
                    Some(slot) if !slot.dirty => {
                        slot.dirty = true;
                        true
                    }
                    _ => false,
                };
                if perlu {
                    st.dirty.push(s);
                    marked = true;
                }
            }
            if marked {
                st.wake_pending = true;
            }
            st.batch_depth == 0 && st.wake_pending
        };
        if flush {
            self.flush_wake();
        }
    }

    fn flush_wake(&self) {
        let fire = {
            let mut st = self.inner.state.borrow_mut();
            std::mem::replace(&mut st.wake_pending, false)
        };
        if !fire {
            return;
        }
        let wake = self.inner.wake.borrow().clone();
        if let Some(wake) = wake {
            wake(SIGNAL_DIRTY);
        }
    }

    /// Kelompokkan banyak tulisan menjadi **satu** pembangunan renderer.
    ///
    /// Nilai signal berubah seketika (tidak ada transaksi); yang ditunda hanya
    /// pemberitahuan ke scheduler. Batch boleh bersarang — flush terjadi saat
    /// batch terluar selesai.
    pub fn batch<R>(&self, f: impl FnOnce() -> R) -> R {
        struct Guard<'a>(&'a Runtime);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                let depth = {
                    let mut st = self.0.inner.state.borrow_mut();
                    st.batch_depth -= 1;
                    st.batch_depth
                };
                if depth == 0 {
                    self.0.flush_wake();
                }
            }
        }
        self.inner.state.borrow_mut().batch_depth += 1;
        let _g = Guard(self);
        f()
    }

    /// Benar bila sedang berada di dalam [`Runtime::batch`].
    pub fn is_batching(&self) -> bool {
        self.inner.state.borrow().batch_depth > 0
    }

    /// Ambil daftar scope yang harus dibangun ulang.
    ///
    /// Hasilnya sudah **diurutkan dari akar ke daun** dan **dipangkas**:
    /// keturunan dari scope yang juga dirty dibuang, karena membangun ulang
    /// leluhurnya sudah membangun ulang subtree-nya (§2.5). Semua tanda dirty
    /// dibersihkan — pemanggil wajib benar-benar membangun ulang hasilnya.
    ///
    /// Konsekuensi kontrak pemangkasan: **membangun ulang sebuah scope harus
    /// memasuki kembali setiap anak yang dipertahankannya** (lewat [`super::scope`]).
    /// Selama itu dipenuhi, memoisasi anak boleh ditambahkan nanti — tapi anak
    /// yang di-memo tidak boleh dilewati saat leluhurnya dirty.
    ///
    /// Jebakan yang harus diketahui: menulis signal yang dibaca oleh scope yang
    /// sedang dibangun akan menandai scope itu dirty lagi, dan frame berikutnya
    /// akan terus dijadwalkan. Itu terlihat jelas di log frame sebagai animasi
    /// yang tidak pernah selesai — bukan hang diam-diam.
    pub fn drain_dirty(&self) -> Vec<ScopeId> {
        let mut st = self.inner.state.borrow_mut();
        let mut kandidat = std::mem::take(&mut st.dirty);
        kandidat.retain(|id| match st.scope_mut(*id) {
            Some(slot) => {
                slot.dirty = false;
                true
            }
            None => false,
        });
        let set: HashSet<ScopeId> = kandidat.iter().copied().collect();
        kandidat.sort_by_key(|id| (st.scope(*id).map(|s| s.depth).unwrap_or(0), id.index));

        let mut keluar = Vec::with_capacity(kandidat.len());
        'kandidat: for id in kandidat {
            let mut naik = st.scope(id).and_then(|s| s.parent);
            while let Some(a) = naik {
                if set.contains(&a) {
                    continue 'kandidat;
                }
                naik = st.scope(a).and_then(|s| s.parent);
            }
            keluar.push(id);
        }
        keluar
    }

    // -- introspeksi (dipakai view layer, devtools, dan test) --------------

    /// Benar bila scope masih hidup.
    pub fn is_scope_alive(&self, id: ScopeId) -> bool {
        self.inner.state.borrow().scope(id).is_some()
    }

    /// Benar bila signal masih hidup.
    pub fn is_signal_alive(&self, id: SignalId) -> bool {
        self.inner.state.borrow().signal(id).is_some()
    }

    /// Benar bila scope menunggu dibangun ulang.
    pub fn is_dirty(&self, id: ScopeId) -> bool {
        self.inner.state.borrow().scope(id).is_some_and(|s| s.dirty)
    }

    /// Berapa scope yang menunggu dibangun ulang (sebelum pemangkasan).
    pub fn dirty_len(&self) -> usize {
        self.inner.state.borrow().dirty.len()
    }

    /// Anak-anak scope dalam urutan build terakhir.
    pub fn children(&self, id: ScopeId) -> Vec<ScopeId> {
        self.inner
            .state
            .borrow()
            .scope(id)
            .map(|s| s.children.clone())
            .unwrap_or_default()
    }

    /// Induk scope (`None` untuk akar atau scope mati).
    pub fn parent(&self, id: ScopeId) -> Option<ScopeId> {
        self.inner.state.borrow().scope(id).and_then(|s| s.parent)
    }

    /// Kedalaman scope; akar = 0.
    pub fn depth(&self, id: ScopeId) -> Option<u32> {
        self.inner.state.borrow().scope(id).map(|s| s.depth)
    }

    /// Kunci identitas scope.
    pub fn key(&self, id: ScopeId) -> Option<Key> {
        self.inner.state.borrow().scope(id).map(|s| s.key.clone())
    }

    /// Jumlah scope hidup (termasuk akar).
    pub fn live_scopes(&self) -> usize {
        self.inner.state.borrow().live_scopes
    }

    /// Jumlah signal hidup.
    pub fn live_signals(&self) -> usize {
        self.inner.state.borrow().live_signals
    }

    /// Berapa scope yang berlangganan sebuah signal.
    pub fn subscriber_count(&self, id: SignalId) -> usize {
        self.inner
            .state
            .borrow()
            .signal(id)
            .map(|s| s.subscribers.len())
            .unwrap_or(0)
    }

    /// Scope pemilik sebuah signal; `None` bila signal milik runtime
    /// ([`Runtime::signal`]) atau signal-nya sudah mati.
    pub fn signal_owner(&self, id: SignalId) -> Option<ScopeId> {
        self.inner.state.borrow().signal(id).and_then(|s| s.owner)
    }

    /// Berapa signal yang dibaca sebuah scope pada build terakhirnya.
    pub fn dependency_count(&self, id: ScopeId) -> usize {
        self.inner
            .state
            .borrow()
            .scope(id)
            .map(|s| s.deps.len())
            .unwrap_or(0)
    }

    /// Jumlah `use_signal` yang dimiliki sebuah scope.
    pub fn hook_count(&self, id: ScopeId) -> usize {
        self.inner
            .state
            .borrow()
            .scope(id)
            .map(|s| s.hooks.len())
            .unwrap_or(0)
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let st = self.inner.state.borrow();
        f.debug_struct("Runtime")
            .field("id", &self.inner.id)
            .field("scopes", &st.live_scopes)
            .field("signals", &st.live_signals)
            .field("dirty", &st.dirty.len())
            .finish()
    }
}
