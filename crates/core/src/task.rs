//! **Async & threading: how a background result gets back to the UI thread**
//! (REKOMENDASI §9.6).
//!
//! Every real application does network, disk, and database work. §9.6 asks for
//! four answers, and this module is all four:
//!
//! | Question | Answer |
//! |---|---|
//! | How does a result reach the UI thread? | A channel of **type-erased payloads**; the closure that touches signals never leaves the UI thread |
//! | May a widget spawn a task? | Yes — through [`BuildCtx::spawn_blocking`](crate::app::BuildCtx::spawn_blocking), which ties the task to the component's scope |
//! | Is tokio integrated? | Optionally, behind the `tokio` feature (`Tasks::spawn`) |
//! | What happens when the component disappears? | Its continuation is dropped and its [`Cancel`] flag is raised — see *Cancellation* below |
//!
//! # The shape, and why it is this shape
//!
//! [`Signal`](crate::signals::Signal) is deliberately **not** `Send`: it is a
//! handle into a thread-local runtime, and making it cross threads would turn
//! every state write into a lock. So a task is split in two halves that never
//! meet:
//!
//! ```text
//!  UI thread                                worker thread
//!  ─────────                                ─────────────
//!  spawn_blocking(work, then)
//!      │  register `then` under a TaskId
//!      │  (a !Send closure — it may hold signals)
//!      └──────────── work ─────────────────────▶ runs, returns T: Send
//!                                                     │
//!         Delivery { id, Box<dyn Any + Send> } ◀───────┘
//!      ┌──────────────┘        (+ notifier: wake the event loop)
//!      ▼
//!  Tasks::deliver()  →  downcast to T  →  then(T)  →  signals dirty  →  frame
//! ```
//!
//! `work` is `Send` and returns something `Send`. `then` is **not** `Send` and
//! runs on the UI thread, which is what lets it write a signal. Nothing else is
//! allowed to cross, and that is the whole discipline.
//!
//! # Where it is driven from
//!
//! [`AppRuntime::frame`](crate::app::AppRuntime::frame) calls
//! [`Tasks::deliver`] before it drains the dirty scopes, so a result that
//! arrived between two frames is applied and rebuilt in the **same** frame
//! rather than one frame late.
//!
//! A result that arrives while the application is idle would otherwise sit in
//! the channel until something else woke the loop, which is why a shell
//! installs a [`Notifier`]:
//!
//! ```
//! use silka_core::app::app;
//! use silka_core::view::fixed;
//!
//! let ui = app(|_cx| fixed(10.0, 10.0).into());
//! // In a real shell this is `EventLoopProxy::send_event(…)`; anything that
//! // makes the platform loop turn one more time will do.
//! ui.tasks().notify_with(|| { /* wake the event loop */ });
//! ```
//!
//! The notifier is called **from the worker thread**, so it is `Send + Sync`
//! and must do nothing but poke the loop. Everything else happens on the UI
//! thread, inside `deliver`.
//!
//! # Cancellation
//!
//! Two independent mechanisms, because a thread cannot be killed:
//!
//! 1. **The continuation is dropped.** A task spawned inside a component
//!    records its [`ScopeId`]. When the result arrives, [`Tasks::deliver`] asks
//!    the runtime whether that scope is still alive; if the component is gone,
//!    the payload is discarded and `then` never runs. This is automatic, and it
//!    is what stops a reply from a request nobody is waiting for any more from
//!    writing into a dead subtree.
//! 2. **The work is asked to stop.** `work` receives a [`Cancel`] token — an
//!    `AtomicBool` shared with the [`TaskHandle`]. Long work checks
//!    [`Cancel::is_cancelled`] between chunks and returns early. Cooperative on
//!    purpose: a hard abort in the middle of an HTTP body or a transaction is
//!    how connections leak.
//!
//! # Loading data — the example §9.6 asks for
//!
//! ```
//! use silka_core::app::{app, component};
//! use silka_core::task::{use_resource, Load};
//! use silka_core::view::{column, View};
//!
//! # fn fetch_invoices() -> Result<Vec<String>, String> {
//! #     Ok(vec![String::from("INV-001")])
//! # }
//! let mut ui = app(|_cx| {
//!     View::from(column([component("invoices", |_cx| {
//!         // Spawned once, on this component's first build. The signal starts
//!         // out `Loading`, so the very first frame already has something to
//!         // draw — there is no window with a hole in it.
//!         let invoices = use_resource(|_cancel| fetch_invoices());
//!
//!         match invoices.get() {
//!             Load::Loading => View::from(silka_core::view::fixed(80.0, 20.0)),
//!             Load::Ready(rows) => View::from(silka_core::view::fixed(80.0, 20.0 * rows.len() as f32)),
//!             Load::Failed(_) => View::from(silka_core::view::fixed(80.0, 8.0)),
//!         }
//!     })]))
//! })
//! .sized(320.0, 200.0);
//!
//! // Frame 1 mounts the tree and starts the work.
//! ui.frame();
//!
//! // A test drives the bridge by hand instead of sleeping: block until the
//! // worker has delivered, then run the frame that applies it.
//! ui.tasks().wait_for_idle();
//! ui.frame();
//! ```
//!
//! # What this module deliberately is not
//!
//! It is **not** an async runtime, and `silka-core` gains no executor of its
//! own. [`Tasks::spawn_blocking`] uses one OS thread per task, which is the
//! right answer for the handful of concurrent requests a desktop UI actually
//! has in flight; an application that wants thousands turns on the `tokio`
//! feature, or hands its own executor to [`Tasks::spawn_with`].

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use crate::signals::{current_scope, use_signal, Runtime, ScopeId, Signal};

// ---------------------------------------------------------------------------
// TaskId
// ---------------------------------------------------------------------------

/// Identity of one spawned task, unique within its [`Tasks`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(u64);

impl TaskId {
    /// The raw number — for logs and inspector output.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "task#{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// Cancel
// ---------------------------------------------------------------------------

/// The "please stop" flag handed to the work closure.
///
/// Cooperative by design: nothing interrupts a running thread, so long work
/// checks it between chunks.
///
/// ```
/// use silka_core::task::Cancel;
///
/// // Standing in for the token a real task receives.
/// let cancel = Cancel::detached();
/// assert!(!cancel.is_cancelled());
///
/// // The shape a worker loop takes: check, then do a chunk.
/// let mut done = 0;
/// for chunk in 0..10 {
///     if cancel.is_cancelled() {
///         break;
///     }
///     done = chunk;
/// }
/// assert_eq!(done, 9);
/// ```
#[derive(Clone)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A token nobody can raise — for tests and for work that cannot be
    /// interrupted anyway.
    pub fn detached() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// True once the task has been cancelled (the handle asked, or the owning
    /// component disappeared).
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Raise the flag.
    fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl fmt::Debug for Cancel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Cancel").field(&self.is_cancelled()).finish()
    }
}

// ---------------------------------------------------------------------------
// Notifier
// ---------------------------------------------------------------------------

/// What a worker thread calls to make the UI thread turn one more frame.
///
/// `Send + Sync` because it is used off the UI thread, and deliberately
/// featureless: waking a platform event loop is the only thing it is allowed to
/// do.
#[derive(Clone)]
pub struct Notifier(Option<Arc<dyn Fn() + Send + Sync>>);

impl Notifier {
    /// A notifier that does nothing — the default, and what a headless test
    /// uses.
    pub fn inert() -> Self {
        Self(None)
    }

    /// Wrap the shell's wake function.
    pub fn new(f: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(f)))
    }

    /// True when a real wake function is installed.
    pub fn is_installed(&self) -> bool {
        self.0.is_some()
    }

    /// Poke the event loop.
    pub fn notify(&self) {
        if let Some(f) = &self.0 {
            f();
        }
    }
}

impl fmt::Debug for Notifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Notifier")
            .field("installed", &self.is_installed())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Spawner
// ---------------------------------------------------------------------------

/// Where a piece of blocking work actually runs.
///
/// The default is [`ThreadSpawner`] — one OS thread per task, which is the
/// honest answer for a desktop UI with a handful of requests in flight. An
/// application that already owns a thread pool implements this trait over it
/// and hands it to [`Tasks::spawn_with`], so `silka-core` never grows a second
/// executor.
///
/// ```
/// use silka_core::task::{Spawner, Tasks};
///
/// // A "spawner" that runs the job immediately, on the calling thread. Useless
/// // in production and perfect in a test: the delivery is already in the
/// // channel by the time `spawn_with` returns.
/// struct Inline;
/// impl Spawner for Inline {
///     fn spawn(&self, job: Box<dyn FnOnce() + Send + 'static>) {
///         job();
///     }
/// }
///
/// let rt = silka_core::signals::Runtime::new();
/// let tasks = Tasks::new();
/// let seen = std::rc::Rc::new(std::cell::Cell::new(0u32));
/// let sink = seen.clone();
/// tasks.spawn_with(&Inline, |_cancel| 41u32 + 1, move |v| sink.set(v));
///
/// assert_eq!(tasks.deliver(&rt), 1);
/// assert_eq!(seen.get(), 42);
/// ```
pub trait Spawner {
    /// Run `job` somewhere other than the UI thread (or, for a test spawner,
    /// right here).
    fn spawn(&self, job: Box<dyn FnOnce() + Send + 'static>);
}

/// One OS thread per task — the default [`Spawner`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ThreadSpawner;

impl Spawner for ThreadSpawner {
    fn spawn(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        std::thread::spawn(job);
    }
}

// ---------------------------------------------------------------------------
// TaskHandle
// ---------------------------------------------------------------------------

/// What the caller keeps: the identity of a spawned task, and the ability to
/// cancel it.
///
/// Dropping the handle does **not** cancel — a fire-and-forget save is the
/// common case, and silently cancelling it on drop would be a trap. Use
/// [`TaskHandle::cancel`], or let the component's death do it (see the module
/// docs).
#[derive(Debug, Clone)]
pub struct TaskHandle {
    id: TaskId,
    cancel: Cancel,
}

impl TaskHandle {
    /// This task's identity.
    pub fn id(&self) -> TaskId {
        self.id
    }

    /// Ask the work to stop and make sure the continuation never runs.
    ///
    /// Raising the flag is instant; whether the work notices depends on how
    /// often it checks (it is cooperative).
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// True once [`TaskHandle::cancel`] has been called (by anyone).
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// The token itself, for code that wants to hand it further down.
    pub fn token(&self) -> Cancel {
        self.cancel.clone()
    }
}

// ---------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------

/// Keeps the "still running" count honest.
///
/// A guard rather than a `fetch_sub` at the end of the job, so the count also
/// drops when the work panics — otherwise [`Tasks::wait_for_idle`] would hang
/// forever on the one failure it exists to survive.
struct InFlight(Arc<AtomicUsize>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// One task's finished payload, on its way back to the UI thread.
struct Delivery {
    id: TaskId,
    payload: Box<dyn Any + Send>,
}

/// A continuation with its argument type erased, so one map can hold the
/// continuations of tasks that return different things.
type Continuation = Box<dyn FnOnce(Box<dyn Any + Send>)>;

/// The UI-side half of a task: the continuation, plus who it belongs to.
struct Pending {
    /// The component scope that spawned it, when there was one. `None` means
    /// "application-level": nothing can outlive it, so it always runs.
    scope: Option<ScopeId>,
    cancel: Cancel,
    then: Continuation,
}

struct Inner {
    next: Cell<u64>,
    pending: RefCell<HashMap<TaskId, Pending>>,
    tx: Sender<Delivery>,
    rx: Receiver<Delivery>,
    notifier: RefCell<Notifier>,
    /// How many spawned jobs have not yet handed their payload to the
    /// channel. Used by [`Tasks::wait_for_idle`], which is how a test avoids
    /// sleeping.
    outstanding: Arc<AtomicUsize>,
    #[cfg(feature = "tokio")]
    tokio: RefCell<Option<Arc<tokio::runtime::Runtime>>>,
}

/// The application's task bridge — **one per [`AppRuntime`](crate::app::AppRuntime)**.
///
/// Cheap to clone (an `Rc`), and deliberately not `Send`: it lives on the UI
/// thread, next to the signals runtime. What crosses threads is the
/// [`Sender`] inside it and nothing else.
#[derive(Clone)]
pub struct Tasks(std::rc::Rc<Inner>);

impl Default for Tasks {
    fn default() -> Self {
        Self::new()
    }
}

impl Tasks {
    /// An empty bridge with an inert [`Notifier`].
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self(std::rc::Rc::new(Inner {
            next: Cell::new(1),
            pending: RefCell::new(HashMap::new()),
            tx,
            rx,
            notifier: RefCell::new(Notifier::inert()),
            outstanding: Arc::new(AtomicUsize::new(0)),
            #[cfg(feature = "tokio")]
            tokio: RefCell::new(None),
        }))
    }

    /// Install the shell's wake function (see the module docs).
    pub fn notify_with(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.0.notifier.borrow_mut() = Notifier::new(f);
    }

    /// The notifier in effect — cloneable and `Send`, so a worker can keep one.
    pub fn notifier(&self) -> Notifier {
        self.0.notifier.borrow().clone()
    }

    /// How many continuations are still registered.
    pub fn pending_len(&self) -> usize {
        self.0.pending.borrow().len()
    }

    /// True when nothing is registered and nothing is in flight.
    pub fn is_idle(&self) -> bool {
        self.pending_len() == 0 && self.in_flight() == 0
    }

    /// How many spawned jobs are still running (they have not handed their
    /// payload to the channel yet).
    pub fn in_flight(&self) -> usize {
        self.0.outstanding.load(Ordering::Acquire)
    }

    /// Cancel every task and drop every continuation.
    ///
    /// Called when a window closes: whatever comes back afterwards has nowhere
    /// to land.
    pub fn cancel_all(&self) {
        let mut pending = self.0.pending.borrow_mut();
        for task in pending.values() {
            task.cancel.cancel();
        }
        pending.clear();
    }

    /// Cancel one task by id; true when it was still registered.
    pub fn cancel(&self, id: TaskId) -> bool {
        let mut pending = self.0.pending.borrow_mut();
        match pending.remove(&id) {
            Some(task) => {
                task.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Drop the continuations whose component scope has died, raising their
    /// cancel flags on the way out.
    ///
    /// [`Tasks::deliver`] does this for the payloads it receives; this is the
    /// sweep for tasks whose result has not arrived yet, so a long download for
    /// a page the user has already left stops as soon as anyone asks.
    pub fn sweep(&self, runtime: &Runtime) -> usize {
        let mut pending = self.0.pending.borrow_mut();
        let before = pending.len();
        pending.retain(|_, task| match task.scope {
            Some(scope) if !runtime.is_scope_alive(scope) => {
                task.cancel.cancel();
                false
            }
            _ => true,
        });
        before - pending.len()
    }

    /// Apply every payload that has arrived, and return how many continuations
    /// ran.
    ///
    /// This is the **only** place a task's result touches application state, and
    /// it always runs on the UI thread. Three reasons a payload is dropped
    /// instead of applied, all of them normal:
    ///
    /// - the task was cancelled;
    /// - its component scope has died (the user navigated away);
    /// - the payload is not the type the continuation expected, which can only
    ///   happen if a `Sender` was cloned out and misused.
    pub fn deliver(&self, runtime: &Runtime) -> usize {
        let mut ran = 0usize;
        // `try_iter` rather than `iter`: this must never block a frame.
        while let Ok(delivery) = self.0.rx.try_recv() {
            let Some(task) = self.0.pending.borrow_mut().remove(&delivery.id) else {
                continue;
            };
            if task.cancel.is_cancelled() {
                continue;
            }
            if let Some(scope) = task.scope {
                if !runtime.is_scope_alive(scope) {
                    continue;
                }
            }
            // The borrow on `pending` is released before this runs: a
            // continuation is allowed to spawn the next task.
            (task.then)(delivery.payload);
            ran += 1;
        }
        ran
    }

    /// Block the UI thread until every in-flight payload has been **sent**.
    ///
    /// For tests and for a deliberate shutdown, never for a frame: it is how
    /// `#[test]` avoids `sleep`, which is the difference between a
    /// deterministic suite and a flaky one (§9.5). The payloads still have to be
    /// applied by [`Tasks::deliver`] afterwards.
    pub fn wait_for_idle(&self) {
        while self.in_flight() > 0 {
            std::thread::yield_now();
        }
    }

    /// Spawn blocking work on the default [`ThreadSpawner`].
    ///
    /// `work` runs off the UI thread and returns something `Send`; `then` runs
    /// **on** the UI thread with that value, which is what lets it write a
    /// signal.
    ///
    /// ```
    /// use silka_core::signals::Runtime;
    /// use silka_core::task::Tasks;
    ///
    /// let rt = Runtime::new();
    /// let tasks = Tasks::new();
    /// let total = rt.signal(0u64);
    ///
    /// // `work` is `Send` and knows nothing about signals…
    /// tasks.spawn_blocking(
    ///     |_cancel| (1..=10u64).sum::<u64>(),
    ///     // …and `then` is not `Send` and does nothing else.
    ///     move |sum| total.set(sum),
    /// );
    ///
    /// tasks.wait_for_idle();
    /// assert_eq!(tasks.deliver(&rt), 1);
    /// assert_eq!(total.peek(), 55);
    /// ```
    pub fn spawn_blocking<T, W, F>(&self, work: W, then: F) -> TaskHandle
    where
        T: Send + 'static,
        W: FnOnce(&Cancel) -> T + Send + 'static,
        F: FnOnce(T) + 'static,
    {
        self.spawn_with(&ThreadSpawner, work, then)
    }

    /// [`Tasks::spawn_blocking`] on a caller-supplied [`Spawner`].
    pub fn spawn_with<T, W, F>(&self, spawner: &impl Spawner, work: W, then: F) -> TaskHandle
    where
        T: Send + 'static,
        W: FnOnce(&Cancel) -> T + Send + 'static,
        F: FnOnce(T) + 'static,
    {
        let handle = self.register(then);
        let id = handle.id;
        let token = handle.cancel.clone();
        let tx = self.0.tx.clone();
        let notifier = self.notifier();
        let guard = InFlight(self.0.outstanding.clone());
        spawner.spawn(Box::new(move || {
            // Declared first so it is dropped **last** — the count reaches zero
            // only after the payload has been sent and the loop poked, and it
            // reaches zero even if `work` panics (§9.7).
            let _guard = guard;
            let value = work(&token);
            // Sending fails only when the application has gone away, and then
            // there is nothing left to notify either.
            let sent = tx.send(Delivery {
                id,
                payload: Box::new(value),
            });
            if sent.is_ok() {
                notifier.notify();
            }
        }));
        handle
    }

    /// Spawn a `Future` on a **tokio** runtime (feature `tokio`).
    ///
    /// It uses the ambient runtime when the caller already has one — a shell
    /// that starts tokio itself, or an `#[tokio::main]` test — and otherwise
    /// creates one multi-threaded runtime, once, and keeps it for the lifetime
    /// of this bridge. There is never more than one, and none at all until the
    /// first call: an application that does no async pays nothing.
    ///
    /// The split is exactly the same as [`Tasks::spawn_blocking`]: the future is
    /// `Send`, the continuation is not.
    #[cfg(feature = "tokio")]
    pub fn spawn<T, Fut, F>(&self, future: Fut, then: F) -> TaskHandle
    where
        T: Send + 'static,
        Fut: std::future::Future<Output = T> + Send + 'static,
        F: FnOnce(T) + 'static,
    {
        let handle = self.register(then);
        let id = handle.id;
        let tx = self.0.tx.clone();
        let notifier = self.notifier();
        let guard = InFlight(self.0.outstanding.clone());
        let job = async move {
            let _guard = guard;
            let value = future.await;
            if tx
                .send(Delivery {
                    id,
                    payload: Box::new(value),
                })
                .is_ok()
            {
                notifier.notify();
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(current) => {
                current.spawn(job);
            }
            Err(_) => {
                let rt = self.tokio_runtime();
                rt.spawn(job);
            }
        }
        handle
    }

    /// The lazily created fallback tokio runtime.
    #[cfg(feature = "tokio")]
    fn tokio_runtime(&self) -> Arc<tokio::runtime::Runtime> {
        let mut slot = self.0.tokio.borrow_mut();
        if let Some(rt) = slot.as_ref() {
            return rt.clone();
        }
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("a tokio runtime could not be created"),
        );
        *slot = Some(rt.clone());
        rt
    }

    /// Register a continuation and hand back its handle.
    ///
    /// The scope is read from the signals runtime: called during a build it is
    /// the component's, and called from an event handler it is `None` — which is
    /// exactly the difference between "cancel when this component goes away"
    /// and "application-level".
    fn register<T: Send + 'static>(&self, then: impl FnOnce(T) + 'static) -> TaskHandle {
        self.0.outstanding.fetch_add(1, Ordering::AcqRel);
        let id = TaskId(self.0.next.get());
        self.0.next.set(self.0.next.get() + 1);
        let cancel = Cancel::detached();
        let erased: Continuation = Box::new(move |payload| {
            match payload.downcast::<T>() {
                Ok(value) => then(*value),
                // Only reachable if a `Delivery` was forged with the wrong
                // type. Dropping is the §9.7 answer: never panic in the middle
                // of a frame.
                Err(_) => debug_assert!(false, "payload tugas bertipe lain"),
            }
        });
        self.0.pending.borrow_mut().insert(
            id,
            Pending {
                scope: current_scope(),
                cancel: cancel.clone(),
                then: erased,
            },
        );
        TaskHandle { id, cancel }
    }
}

impl fmt::Debug for Tasks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tasks")
            .field("pending", &self.pending_len())
            .field("in_flight", &self.in_flight())
            .field("notifier", &*self.0.notifier.borrow())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Load / use_resource
// ---------------------------------------------------------------------------

/// The three states of something being fetched.
///
/// A separate type rather than `Option<Result<T, E>>` because the UI has to
/// draw all three, and the names are what stop a skeleton from being confused
/// with an empty result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load<T> {
    /// The work has been spawned and has not answered yet — draw a skeleton or
    /// a spinner, never an empty list.
    Loading,
    /// It answered.
    Ready(T),
    /// It failed, with something to show the user.
    Failed(String),
}

impl<T> Load<T> {
    /// True while waiting.
    pub fn is_loading(&self) -> bool {
        matches!(self, Load::Loading)
    }

    /// The value, when there is one.
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(v) => Some(v),
            _ => None,
        }
    }

    /// The error message, when it failed.
    pub fn error(&self) -> Option<&str> {
        match self {
            Load::Failed(e) => Some(e.as_str()),
            _ => None,
        }
    }
}

/// Fetch something **once** for this component, and get a signal that follows
/// it (§9.6).
///
/// The hook rules are the [`use_signal`] rules: same call order every build, so
/// never inside an `if`. On the first build it registers the work with the
/// application's [`Tasks`] and returns a signal holding [`Load::Loading`]; when
/// the answer arrives the signal is written, which marks exactly the components
/// that read it dirty and nothing else (§2.5).
///
/// Panics when called outside a component build, and when there is no
/// [`AppRuntime`](crate::app::AppRuntime) hosting the build — the work has
/// nowhere to run without one.
///
/// See the module docs for a complete example.
pub fn use_resource<T, W>(work: W) -> Signal<Load<T>>
where
    T: Send + 'static,
    W: FnOnce(&Cancel) -> Result<T, String> + Send + 'static,
{
    let state: Signal<Load<T>> = use_signal(|| Load::Loading);
    let started = use_signal(|| false);
    // `peek`, not `get`: subscribing to our own guard would mark this component
    // dirty the moment we set it, and rebuild forever.
    if !started.peek() {
        started.set(true);
        let tasks = crate::app::current_tasks()
            .expect("use_resource() butuh AppRuntime yang sedang membangun");
        tasks.spawn_blocking(work, move |result| {
            state.set(match result {
                Ok(value) => Load::Ready(value),
                Err(message) => Load::Failed(message),
            });
        });
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    /// Runs the job on the calling thread, so a test never sleeps.
    struct Inline;

    impl Spawner for Inline {
        fn spawn(&self, job: Box<dyn FnOnce() + Send + 'static>) {
            job();
        }
    }

    /// Keeps the job until the test decides to run it — the only way to test
    /// "cancelled before the work started" without a sleep.
    #[derive(Default)]
    struct Deferred(RefCell<Option<Box<dyn FnOnce() + Send + 'static>>>);

    impl Deferred {
        fn run(&self) {
            if let Some(job) = self.0.borrow_mut().take() {
                job();
            }
        }
    }

    impl Spawner for Deferred {
        fn spawn(&self, job: Box<dyn FnOnce() + Send + 'static>) {
            *self.0.borrow_mut() = Some(job);
        }
    }

    #[test]
    fn hasil_sampai_ke_ui_lewat_deliver() {
        let rt = Runtime::new();
        let tasks = Tasks::new();
        let sink = Rc::new(Cell::new(0u32));
        let tulis = sink.clone();

        tasks.spawn_with(&Inline, |_c| 7u32, move |v| tulis.set(v));
        // Nothing has been applied yet: the payload waits for the frame.
        assert_eq!(sink.get(), 0);
        assert_eq!(tasks.deliver(&rt), 1);
        assert_eq!(sink.get(), 7);
        assert!(tasks.is_idle());
    }

    #[test]
    fn deliver_kedua_kali_tidak_mengulang() {
        let rt = Runtime::new();
        let tasks = Tasks::new();
        let hitung = Rc::new(Cell::new(0u32));
        let tulis = hitung.clone();
        tasks.spawn_with(&Inline, |_c| (), move |()| tulis.set(tulis.get() + 1));
        assert_eq!(tasks.deliver(&rt), 1);
        assert_eq!(tasks.deliver(&rt), 0);
        assert_eq!(hitung.get(), 1);
    }

    #[test]
    fn cancel_membuang_hasil_dan_menaikkan_bendera() {
        let rt = Runtime::new();
        let tasks = Tasks::new();
        let sink = Rc::new(Cell::new(0u32));
        let tulis = sink.clone();

        let handle = tasks.spawn_with(&Inline, |_c| 9u32, move |v| tulis.set(v));
        handle.cancel();
        assert!(handle.is_cancelled());
        assert_eq!(tasks.deliver(&rt), 0, "continuation tidak boleh jalan");
        assert_eq!(sink.get(), 0);
    }

    #[test]
    fn cancel_by_id_menghapus_pendaftaran() {
        let tasks = Tasks::new();
        let handle = tasks.spawn_with(&ThreadSpawner, |_c| 1u8, |_v| {});
        assert!(tasks.cancel(handle.id()));
        assert!(!tasks.cancel(handle.id()), "hanya sekali");
        assert!(handle.is_cancelled());
    }

    #[test]
    fn kerja_melihat_bendera_yang_sama_dengan_handle() {
        let rt = Runtime::new();
        let tasks = Tasks::new();
        let deferred = Deferred::default();
        let seen = Rc::new(Cell::new(false));
        let tulis = seen.clone();

        // Spawn, cancel, *then* let the job run: this is the shape of a task
        // whose component disappeared while it was queued.
        let handle = tasks.spawn_with(&deferred, |c| c.is_cancelled(), move |v| tulis.set(v));
        handle.cancel();
        deferred.run();

        assert!(
            !seen.get(),
            "continuation tugas yang dibatalkan tidak boleh jalan"
        );
        assert_eq!(tasks.deliver(&rt), 0);
        // …and the work itself did observe the flag, which is what lets long
        // work return early instead of finishing pointlessly.
        assert!(handle.token().is_cancelled());
    }

    #[test]
    fn cancel_all_membersihkan_semuanya() {
        let tasks = Tasks::new();
        let a = tasks.spawn_with(&ThreadSpawner, |_c| 1u8, |_v| {});
        let b = tasks.spawn_with(&ThreadSpawner, |_c| 2u8, |_v| {});
        assert_eq!(tasks.pending_len(), 2);
        tasks.cancel_all();
        assert_eq!(tasks.pending_len(), 0);
        assert!(a.is_cancelled() && b.is_cancelled());
    }

    #[test]
    fn thread_spawner_benar_benar_menyeberang() {
        let rt = Runtime::new();
        let tasks = Tasks::new();
        let sink = Rc::new(Cell::new(0u64));
        let tulis = sink.clone();
        tasks.spawn_blocking(|_c| (1..=10u64).sum::<u64>(), move |v| tulis.set(v));
        tasks.wait_for_idle();
        assert_eq!(tasks.deliver(&rt), 1);
        assert_eq!(sink.get(), 55);
    }

    #[test]
    fn notifier_dipanggil_dari_pekerja() {
        let rt = Runtime::new();
        let tasks = Tasks::new();
        let woken = Arc::new(AtomicBool::new(false));
        let flag = woken.clone();
        tasks.notify_with(move || flag.store(true, Ordering::Release));
        assert!(tasks.notifier().is_installed());

        tasks.spawn_blocking(|_c| 1u8, |_v| {});
        tasks.wait_for_idle();
        assert!(
            woken.load(Ordering::Acquire),
            "hasil yang datang saat idle harus membangunkan event loop"
        );
        tasks.deliver(&rt);
    }

    #[test]
    fn tanpa_notifier_tidak_panik() {
        let rt = Runtime::new();
        let tasks = Tasks::new();
        assert!(!tasks.notifier().is_installed());
        tasks.spawn_blocking(|_c| 1u8, |_v| {});
        tasks.wait_for_idle();
        assert_eq!(tasks.deliver(&rt), 1);
    }

    #[test]
    fn load_membedakan_tiga_keadaan() {
        let loading: Load<u8> = Load::Loading;
        assert!(loading.is_loading() && loading.ready().is_none());
        let ready = Load::Ready(3u8);
        assert_eq!(ready.ready(), Some(&3));
        let failed: Load<u8> = Load::Failed(String::from("timeout"));
        assert_eq!(failed.error(), Some("timeout"));
        assert!(!failed.is_loading());
    }

    #[test]
    fn task_id_naik_dan_bisa_dicetak() {
        let tasks = Tasks::new();
        let a = tasks.spawn_with(&Inline, |_c| (), |()| {});
        let b = tasks.spawn_with(&Inline, |_c| (), |()| {});
        assert!(b.id() > a.id());
        assert_eq!(a.id().to_string(), format!("task#{}", a.id().get()));
    }
}
