//! **Signals + per-component rebuild** — the state model of the framework
//! (REKOMENDASI §2.5).
//!
//! The binding decision: component-local state uses [`use_signal`]; every read
//! of a signal **during build** registers that component as a reader, and every
//! write marks its readers *dirty* → that small subtree is rebuilt → diffed.
//! This is the Dioxus 0.7 pattern, and its mental model is closest to Flutter's
//! `setState`.
//!
//! The price is paid knowingly (and provided here): a **dirty-marking scheduler
//! plus scope tracking** inside the framework, and **key/identity discipline in
//! dynamic lists**.
//!
//! ```
//! use silka_core::signals::{use_signal, Runtime};
//!
//! let rt = Runtime::new();
//! let count = rt.signal(0i32);
//!
//! // A component that reads a signal while building subscribes to it.
//! rt.build_root(|| {
//!     let _teks = format!("Nilai: {}", count.get());
//! });
//! assert!(!rt.is_dirty(rt.root()));
//!
//! // A write from an event handler marks its readers dirty.
//! count.set(1);
//! assert_eq!(rt.drain_dirty(), vec![rt.root()]);
//! ```
//!
//! ## Ground rules
//!
//! - **Reads track, writes mark.** [`Signal::get`]/[`Signal::with`] subscribe
//!   when called during a build; outside a build (event handlers, async
//!   results) they only read. [`Signal::peek`] never subscribes.
//! - **Subscriptions are rebuilt on every build.** A component that stops
//!   reading a signal stops being woken by it — no stale subscriptions.
//! - **Hooks must not be conditional.** `use_signal` is matched by call order;
//!   changing that order or count panics with a clear message instead of
//!   silently swapping state around.
//! - **Children must have keys.** [`scope`] and [`list`] use [`Key`] as
//!   identity; the same key means the same state even when the position moves.
//! - **Batching is about waking the renderer**, not about values: values change
//!   immediately, [`Runtime::batch`] only coalesces the notifications to the
//!   scheduler into one.
//!
//! ## Hooking up the scheduler
//!
//! [`Runtime::on_wake`] is called once per flush with [`SIGNAL_DIRTY`]. Wire it
//! to [`crate::scheduler::FrameScheduler::request`] and rendering stays
//! **dirty-driven only** (§3.5) — a signal nobody reads never wakes the GPU at
//! all.

mod runtime;
#[cfg(test)]
mod tests;

use std::fmt;
use std::marker::PhantomData;

use runtime::{current_build, run_untracked, runtime_of};
pub use runtime::{Runtime, RuntimeId, ScopeId, SignalId, SIGNAL_DIRTY};

// ---------------------------------------------------------------------------
// Key
// ---------------------------------------------------------------------------

/// The identity of a scope among its siblings.
///
/// This is the "key discipline" of §2.5: in a dynamic list it is the key — not
/// the position — that decides whose state is whose. Moving, inserting, or
/// swapping items does not move state around as long as the keys travel with
/// them.
///
/// ```
/// use silka_core::signals::Key;
///
/// // A database row keeps its state when the list is re-sorted, because the
/// // key travels with the row rather than with its position.
/// let row = Key::num(4_201);
/// assert_eq!(row, Key::num(4_201));
/// assert_ne!(row, Key::num(4_202));
///
/// // Slots and paths are text keys.
/// assert_eq!(Key::text("sidebar"), Key::from("sidebar"));
/// ```
///
/// The same key type identifies component scopes *and* child nodes in the
/// view-diff, so there is exactly one key discipline across the framework.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    /// The root scope's key; never constructed by users.
    Root,
    /// A numeric key (database row id, index, enum discriminant).
    Num(i64),
    /// A textual key (uuid, slot name, path).
    Text(Box<str>),
}

impl Key {
    /// A numeric key.
    pub fn num(n: impl Into<i64>) -> Self {
        Key::Num(n.into())
    }

    /// A textual key.
    pub fn text(s: impl AsRef<str>) -> Self {
        Key::Text(s.as_ref().into())
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Root => f.write_str("Key(root)"),
            Key::Num(n) => write!(f, "Key({n})"),
            Key::Text(s) => write!(f, "Key({s:?})"),
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Root => f.write_str("root"),
            Key::Num(n) => write!(f, "{n}"),
            Key::Text(s) => f.write_str(s),
        }
    }
}

macro_rules! key_from_num {
    ($($t:ty),*) => {$(
        impl From<$t> for Key {
            fn from(v: $t) -> Self {
                Key::Num(v as i64)
            }
        }
    )*};
}
key_from_num!(i8, i16, i32, i64, u8, u16, u32, usize);

impl From<&str> for Key {
    fn from(v: &str) -> Self {
        Key::Text(v.into())
    }
}

impl From<String> for Key {
    fn from(v: String) -> Self {
        Key::Text(v.into_boxed_str())
    }
}

impl From<&String> for Key {
    fn from(v: &String) -> Self {
        Key::Text(v.as_str().into())
    }
}

// ---------------------------------------------------------------------------
// Signal
// ---------------------------------------------------------------------------

/// A reactive value owned by the runtime.
///
/// A `Signal` is just an ID — `Copy`, the size of three `u32`s, and free to be
/// captured by as many `move` closures as needed. That is what makes the §2.5
/// writing style possible:
///
/// ```
/// use silka_core::signals::{use_signal, Runtime};
///
/// let rt = Runtime::new();
/// rt.build_root(|| {
///     let count = use_signal(|| 0i32);
///
///     // The signal is captured by value into as many closures as needed;
///     // this is the `on_press` an application writes.
///     let increment = move || count.set(count.get() + 1);
///     increment();
///     increment();
///     assert_eq!(count.get(), 2);
/// });
/// ```
///
/// A signal is bound to its runtime's thread (the UI thread) and is
/// deliberately **not** `Send`: async results come back through the scheduler,
/// not through cross-thread signals.
pub struct Signal<T: 'static> {
    id: SignalId,
    marker: PhantomData<*const T>,
}

impl<T: 'static> Signal<T> {
    pub(crate) fn from_id(id: SignalId) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }

    /// This signal's identity.
    pub fn id(&self) -> SignalId {
        self.id
    }

    /// True while the signal is alive (its owning scope has not been dropped).
    pub fn is_alive(&self) -> bool {
        runtime_of(self.id).is_signal_alive(self.id)
    }

    /// Read by reference — **tracks** when called during a build.
    ///
    /// The closure must not read or write the same signal (recursive access is
    /// reported as a clear panic).
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let rt = runtime_of(self.id);
        rt.track(self.id);
        rt.with_value(self.id, f)
    }

    /// Read a copy of the value — **tracks** when called during a build.
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(|v| v.clone())
    }

    /// Read by reference **without** tracking.
    pub fn peek_with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        runtime_of(self.id).with_value(self.id, f)
    }

    /// Read a copy of the value **without** tracking.
    pub fn peek(&self) -> T
    where
        T: Clone,
    {
        self.peek_with(|v| v.clone())
    }

    /// Write a new value and mark every reader dirty.
    pub fn set(&self, value: T) {
        let _ = self.replace(value);
    }

    /// Write a new value and return the old one.
    pub fn replace(&self, value: T) -> T {
        runtime_of(self.id).replace_value(self.id, value)
    }

    /// Mutate in place; **always** marks dirty (the runtime cannot tell whether
    /// the closure actually changed anything).
    pub fn update<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        runtime_of(self.id).update_value(self.id, f)
    }

    /// Write only if the value actually differs.
    ///
    /// Returns `true` when something changed (and the renderer was woken). This
    /// is the form to use when the source is noisy — e.g. a poll that keeps
    /// delivering the same value.
    pub fn set_if_changed(&self, value: T) -> bool
    where
        T: PartialEq,
    {
        if self.peek_with(|cur| *cur == value) {
            return false;
        }
        self.set(value);
        true
    }
}

impl<T: 'static> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Signal<T> {}

impl<T: 'static> PartialEq for Signal<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T: 'static> Eq for Signal<T> {}

impl<T: 'static> std::hash::Hash for Signal<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: 'static> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.id)
    }
}

// ---------------------------------------------------------------------------
// Hooks & scopes
// ---------------------------------------------------------------------------

/// Component-local state (§2.5) — created once, kept across rebuilds.
///
/// `init` runs only on this scope's first build. On later builds the same hook
/// is recognised by **call order**, so `use_signal` must not sit inside an
/// `if`/`loop` — a violation is reported as a panic rather than as state that
/// silently swaps places.
///
/// Panics when called outside a component build.
///
/// ```
/// use silka_core::signals::{use_signal, Runtime};
///
/// let rt = Runtime::new();
///
/// // `init` runs on the first build…
/// let first = rt.build_root(|| {
///     let count = use_signal(|| 0i32);
///     count.set(count.get() + 1);
///     count.get()
/// });
/// assert_eq!(first, 1);
///
/// // …and never again: a rebuild finds the same signal, holding the value the
/// // previous build left behind. This is what makes component-local state
/// // survive a rebuild without the parent storing it.
/// let second = rt.build_root(|| {
///     let count = use_signal(|| 0i32);
///     count.set(count.get() + 1);
///     count.get()
/// });
/// assert_eq!(second, 2);
/// ```
///
/// Hooks are recognised by **call order**, so a conditional hook is a bug the
/// runtime reports rather than one that silently swaps two pieces of state:
///
/// ```should_panic
/// use silka_core::signals::{use_signal, Runtime};
///
/// let rt = Runtime::new();
/// rt.build_root(|| {
///     let a = use_signal(|| 0i32);
///     let _ = a;
/// });
/// // A second build that reaches a *different* hook at the same position is
/// // caught here, instead of turning into a mysterious value later.
/// rt.build_root(|| {
///     let b = use_signal(|| "text");
///     let _ = b;
/// });
/// ```
pub fn use_signal<T: 'static>(init: impl FnOnce() -> T) -> Signal<T> {
    let (rt_id, scope) = current_build()
        .expect("use_signal hanya boleh dipanggil saat komponen dibangun (di dalam build_root/scope/rebuild)");
    let rt = Runtime::current().expect("runtime yang sedang membangun harus hidup");
    debug_assert_eq!(rt.id(), rt_id);
    Signal::from_id(rt.use_signal_hook::<T>(scope, init))
}

/// Build one child component with the identity `key`.
///
/// The same key on the next build means the same scope, hence the same state,
/// even if the order changed. A key that disappears means its scope is dropped
/// along with the whole subtree, its hooks, and its subscriptions.
///
/// Panics when called outside a build, or when `key` is already used by another
/// sibling in the same build.
///
/// ```
/// use silka_core::signals::{scope, use_signal, Key, Runtime};
///
/// let rt = Runtime::new();
///
/// // Two children, each with state of its own.
/// rt.build_root(|| {
///     scope(Key::from("left"), || use_signal(|| 1i32).set(10));
///     scope(Key::from("right"), || use_signal(|| 2i32).set(20));
/// });
///
/// // On the next build the *keys* decide which scope is which, so swapping
/// // the call order does not swap the state.
/// let (right, left) = rt.build_root(|| {
///     let r = scope(Key::from("right"), || use_signal(|| 2i32).get());
///     let l = scope(Key::from("left"), || use_signal(|| 1i32).get());
///     (r, l)
/// });
/// assert_eq!((left, right), (10, 20));
/// ```
pub fn scope<R>(key: impl Into<Key>, body: impl FnOnce() -> R) -> R {
    let (_, parent) =
        current_build().expect("scope() hanya boleh dipanggil saat komponen dibangun");
    let rt = Runtime::current().expect("runtime yang sedang membangun harus hidup");
    let child = rt.reconcile_child(parent, key.into());
    rt.run_scope(child, body)
        .expect("scope anak baru saja dibuat, tidak mungkin mati")
}

/// Build one child component per item, keyed by `key`.
///
/// A shorthand for [`scope`] over dynamic lists. Reordering items moves their
/// scopes, not their state.
///
/// ```
/// use silka_core::signals::{list, use_signal, Key, Runtime};
///
/// let rt = Runtime::new();
/// let mut baris: Vec<i64> = vec![1, 2, 3];
///
/// rt.build_root(|| {
///     list(baris.iter().copied(), |id| Key::num(*id), |id| use_signal(|| *id))
/// });
/// let awal = rt.children(rt.root());
///
/// // The items are reordered: their scopes move with them, identity is kept.
/// baris.swap(0, 2);
/// rt.build_root(|| {
///     list(baris.iter().copied(), |id| Key::num(*id), |id| use_signal(|| *id))
/// });
/// assert_eq!(rt.children(rt.root()), vec![awal[2], awal[1], awal[0]]);
/// ```
pub fn list<I, K, F, R>(items: I, key: K, mut body: F) -> Vec<R>
where
    I: IntoIterator,
    K: Fn(&I::Item) -> Key,
    F: FnMut(&I::Item) -> R,
{
    items
        .into_iter()
        .map(|item| {
            let k = key(&item);
            scope(k, || body(&item))
        })
        .collect()
}

/// Run `f` without tracking any signal reads.
///
/// Used when a component needs to *look at* a value without wanting to be
/// rebuilt when that value changes.
///
/// ```
/// use silka_core::signals::{untracked, Runtime};
///
/// let rt = Runtime::new();
/// let tracked = rt.signal(0i32);
/// let peeked = rt.signal(0i32);
///
/// rt.build_root(|| {
///     // A normal read subscribes this component to the signal…
///     let _ = tracked.get();
///     // …while an untracked read deliberately does not.
///     let _ = untracked(|| peeked.get());
/// });
///
/// // Only one of the two reads created a dependency.
/// assert_eq!(rt.dependency_count(rt.root()), 1);
///
/// // So writing the peeked signal schedules nothing…
/// peeked.set(99);
/// assert_eq!(rt.dirty_len(), 0);
///
/// // …while writing the tracked one marks the component for rebuild.
/// tracked.set(1);
/// assert_eq!(rt.drain_dirty(), vec![rt.root()]);
/// ```
pub fn untracked<R>(f: impl FnOnce() -> R) -> R {
    run_untracked(f)
}

/// The scope currently being built on this thread, if any.
///
/// `None` outside a build, which is what makes it usable as a "am I inside a
/// component right now?" check rather than as something that panics.
///
/// ```
/// use silka_core::signals::{current_scope, scope, Key, Runtime};
///
/// // Outside any build there is no current scope.
/// assert_eq!(current_scope(), None);
///
/// let rt = Runtime::new();
/// let (outer, inner) = rt.build_root(|| {
///     let outer = current_scope().expect("inside a build");
///     let inner = scope(Key::from("child"), || current_scope().unwrap());
///     (outer, inner)
/// });
///
/// // The root builds as itself; a keyed child builds as its own scope.
/// assert_eq!(outer, rt.root());
/// assert_ne!(inner, outer);
///
/// // And the build is over, so there is nothing current again.
/// assert_eq!(current_scope(), None);
/// ```
pub fn current_scope() -> Option<ScopeId> {
    current_build().map(|(_, scope)| scope)
}
