//! The application's state, and the three rules about background work that the
//! whole example exists to demonstrate.
//!
//! ## Rule 1 — a request is a task, and the UI thread never waits for one
//!
//! [`send`] hands [`crate::http::send`] to
//! [`Tasks::spawn_blocking`](silka_core::task::Tasks::spawn_blocking) and
//! returns immediately, having written exactly one thing: the tab's outcome is
//! now [`Outcome::Sending`]. Everything the user sees while waiting — the
//! progress bar, the elapsed count, the Cancel button — is a rebuild of that
//! one signal. The worker knows nothing about signals; the continuation knows
//! nothing about sockets. That is the entire contract of §9.6, and this file is
//! the smallest honest use of it.
//!
//! ## Rule 2 — leaving a tab cancels its request
//!
//! [`activate`] cancels the request of the tab being *left*. This is a
//! deliberate product decision rather than an accident of the framework, and it
//! is worth being explicit that Postman does the opposite (a request keeps
//! running when you switch away). The brief asks for cancellation that provably
//! stops the work, and a tab switch is the honest place to put it: it is a user
//! action with a clear intent, and the effect is measurable — the worker thread
//! returns within one [`crate::http::POLL`], and the continuation never runs at
//! all.
//!
//! It is the *canceller* that writes [`Outcome::Cancelled`], never the
//! continuation, because a cancelled task's continuation is dropped by
//! [`Tasks::deliver`](silka_core::task::Tasks::deliver) before it can run. If
//! the pane ever showed a response for a tab the user had left, that would mean
//! the cancellation had not happened.
//!
//! ## Rule 3 — a failure is a value
//!
//! There is no `unwrap` on the network path. A refused connection, an unparsable
//! URL, a server that hangs up mid-header: each arrives as
//! [`Outcome::Failed`] carrying a sentence. The panic boundary in
//! [`crate::app`] exists for *programming* errors, and if it ever fires because
//! of a network condition, that is a bug in this file.

use std::collections::HashMap;

use silka_core::signals::{Runtime, Signal};
use silka_core::task::{TaskHandle, Tasks};
use silka_widgets::{SelectState, TreeState};

use crate::http::{self, Method, RequestSpec, Response};

/// Identity of one open request tab.
pub type TabId = u64;

/// How many completed requests the history keeps.
pub const HISTORY_LIMIT: usize = 50;

// ---------------------------------------------------------------------------
// Outcome
// ---------------------------------------------------------------------------

/// What the response pane has to draw for one tab.
///
/// Five states rather than `Option<Result<…>>`, for the reason
/// [`Load`](silka_core::task::Load) gives: the names are what stop "nothing has
/// been sent" from being drawn the same way as "the server sent nothing", and
/// what stop a cancellation from being reported as a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing has been sent from this tab yet.
    Blank,
    /// A request is in flight.
    Sending,
    /// The server answered — with any status, 500 included.
    Done(Response),
    /// The request never got an answer, and this is why, in a sentence.
    Failed(String),
    /// The user (or leaving the tab) stopped it, and this says which.
    Cancelled(String),
}

impl Outcome {
    /// True while a request is in flight — what the progress bar reads.
    pub fn is_sending(&self) -> bool {
        matches!(self, Outcome::Sending)
    }

    /// The response, when there is one.
    pub fn response(&self) -> Option<&Response> {
        match self {
            Outcome::Done(r) => Some(r),
            _ => None,
        }
    }

    /// The short line under the tab bar.
    pub fn summary(&self) -> String {
        match self {
            Outcome::Blank => "Ready".to_string(),
            Outcome::Sending => "Sending…".to_string(),
            Outcome::Done(r) => format!("{} · {} ms", r.status_line(), r.elapsed.as_millis()),
            Outcome::Failed(_) => "Could not send".to_string(),
            Outcome::Cancelled(_) => "Cancelled".to_string(),
        }
    }
}

/// Why a request stopped — the sentence [`Outcome::Cancelled`] carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelCause {
    /// The user switched to another tab.
    LeftTab,
    /// The user pressed Cancel.
    Asked,
    /// The tab was closed.
    Closed,
    /// A second Send was pressed while the first was still running.
    Superseded,
}

impl CancelCause {
    /// What the response pane says.
    pub fn note(self) -> &'static str {
        match self {
            CancelCause::LeftTab => "Stopped when you switched tabs.",
            CancelCause::Asked => "Stopped at your request.",
            CancelCause::Closed => "Stopped because the tab was closed.",
            CancelCause::Superseded => "Stopped by a newer request from this tab.",
        }
    }
}

// ---------------------------------------------------------------------------
// A tab
// ---------------------------------------------------------------------------

/// One open request: what it says, and what came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestTab {
    /// Stable identity — the key the tab row and the in-flight map use.
    pub id: TabId,
    /// The name on the tab.
    pub title: String,
    /// The request itself.
    pub spec: RequestSpec,
    /// What the response pane shows.
    pub outcome: Outcome,
    /// How many times this tab has been sent — a counter the tests read to
    /// tell "it did nothing" from "it did it twice".
    pub sends: u32,
}

impl RequestTab {
    /// A tab holding `spec`, named after it.
    pub fn new(id: TabId, spec: RequestSpec) -> Self {
        Self {
            id,
            title: title_for(&spec),
            spec,
            outcome: Outcome::Blank,
            sends: 0,
        }
    }

    /// The label the `tabs` row shows.
    pub fn label(&self) -> String {
        format!("{} {}", self.spec.method.as_str(), self.title)
    }
}

/// A short name for a request: the last path segment, or the host.
///
/// ```
/// # use silka_api_client::state::title_for;
/// # use silka_api_client::http::RequestSpec;
/// assert_eq!(title_for(&RequestSpec::get("http://h:9/orders/42")), "42");
/// assert_eq!(title_for(&RequestSpec::get("http://h:9/")), "h");
/// assert_eq!(title_for(&RequestSpec::get("")), "Untitled");
/// ```
pub fn title_for(spec: &RequestSpec) -> String {
    let Ok(url) = http::Url::parse(&spec.url) else {
        return "Untitled".to_string();
    };
    let path = url.target.split('?').next().unwrap_or_default();
    match path.rsplit('/').find(|s| !s.is_empty()) {
        Some(last) => last.to_string(),
        None => url.host,
    }
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

/// One finished request, for the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// Everything needed to send it again.
    pub spec: RequestSpec,
    /// The status, when there was a response.
    pub status: Option<u16>,
    /// How long it took, in milliseconds.
    pub millis: u64,
}

impl HistoryEntry {
    /// The second line of the row: `200 · 4 ms`, or the failure.
    pub fn detail(&self) -> String {
        match self.status {
            Some(status) => format!("{status} · {} ms", self.millis),
            None => "no answer".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// In-flight
// ---------------------------------------------------------------------------

/// The handles of every request currently running, keyed by tab.
///
/// Its own signal, and never read during a build: a handle changing must not
/// rebuild a pane. What the panes read is [`Outcome`], which is written in the
/// same breath.
#[derive(Debug, Default)]
pub struct Inflight {
    handles: HashMap<TabId, TaskHandle>,
    /// How many requests have been spawned since the application started.
    pub started: usize,
    /// How many have been cancelled.
    pub cancelled: usize,
}

impl Inflight {
    /// How many requests are running.
    pub fn len(&self) -> usize {
        self.handles.len()
    }

    /// True when nothing is running.
    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    /// True when this tab has a request in flight.
    pub fn holds(&self, tab: TabId) -> bool {
        self.handles.contains_key(&tab)
    }
}

// ---------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------

/// Everything about the requests. UI-only state lives in
/// [`Chrome`](crate::app::Chrome) instead, so that a keystroke in the body
/// editor does not rebuild the sidebar.
#[derive(Debug, Clone, Copy)]
pub struct Store {
    /// The open tabs, in order.
    pub tabs: Signal<Vec<RequestTab>>,
    /// Which tab is showing.
    pub active: Signal<usize>,
    /// Finished requests, newest first.
    pub history: Signal<Vec<HistoryEntry>>,
    /// The running requests — see [`Inflight`].
    pub inflight: Signal<Inflight>,
    /// Where the sample requests point.
    pub base: Signal<String>,
    /// The next tab id.
    next_id: Signal<TabId>,
}

impl Store {
    /// Install the store on `runtime`, with one tab open on `base`.
    pub fn install(runtime: &Runtime, base: impl Into<String>) -> Store {
        let base = base.into();
        let first = RequestTab::new(1, samples(&base).remove(0));
        Store {
            tabs: runtime.signal(vec![first]),
            active: runtime.signal(0),
            history: runtime.signal(Vec::new()),
            inflight: runtime.signal(Inflight::default()),
            base: runtime.signal(base),
            next_id: runtime.signal(2),
        }
    }

    /// The tab currently showing, if there is one.
    pub fn current(&self) -> Option<RequestTab> {
        let index = self.active.get();
        self.tabs.with(|tabs| tabs.get(index).cloned())
    }

    /// The id of the tab currently showing.
    pub fn current_id(&self) -> Option<TabId> {
        let index = self.active.peek();
        self.tabs.peek_with(|tabs| tabs.get(index).map(|t| t.id))
    }

    /// Read one tab without subscribing.
    pub fn tab(&self, id: TabId) -> Option<RequestTab> {
        self.tabs
            .peek_with(|tabs| tabs.iter().find(|t| t.id == id).cloned())
    }

    /// Change one tab in place; does nothing when it has been closed.
    pub fn edit(&self, id: TabId, f: impl FnOnce(&mut RequestTab)) {
        self.tabs.update(|tabs| {
            if let Some(tab) = tabs.iter_mut().find(|t| t.id == id) {
                f(tab);
            }
        });
    }

    /// Take the next tab id.
    fn take_id(&self) -> TabId {
        let id = self.next_id.peek();
        self.next_id.set(id + 1);
        id
    }
}

// ---------------------------------------------------------------------------
// Sending
// ---------------------------------------------------------------------------

/// Send the tab's request in the background.
///
/// Returns the handle so a caller that wants to watch it can; the store keeps
/// its own copy either way. Sending from a tab that is already sending cancels
/// the older request first — two answers arriving for one pane, in an order
/// nobody controls, is the classic async UI bug and it is cheaper to make it
/// impossible than to detect it.
pub fn send(store: &Store, tasks: &Tasks, tab: TabId) -> Option<TaskHandle> {
    let spec = store.tab(tab)?.spec;
    if store.inflight.peek_with(|f| f.holds(tab)) {
        cancel(store, tab, CancelCause::Superseded);
    }

    store.edit(tab, |t| {
        t.outcome = Outcome::Sending;
        t.sends += 1;
        t.title = title_for(&t.spec);
    });

    let store = *store;
    let handle = tasks.spawn_blocking(
        // The worker: `Send`, and it has never heard of a signal.
        move |cancel| http::send(&spec, cancel),
        // The continuation: not `Send`, runs on the UI thread, and is dropped
        // untouched if the task was cancelled or the tab has gone.
        move |result| finish(&store, tab, result),
    );

    store.inflight.update(|f| {
        f.handles.insert(tab, handle.clone());
        f.started += 1;
    });
    Some(handle)
}

/// Apply a finished request. Only ever called on the UI thread, by
/// [`Tasks::deliver`](silka_core::task::Tasks::deliver).
fn finish(store: &Store, tab: TabId, result: Result<Response, String>) {
    store.inflight.update(|f| {
        f.handles.remove(&tab);
    });

    let Some(spec) = store.tab(tab).map(|t| t.spec) else {
        // The tab was closed between the answer arriving and this running.
        // Nothing to write, and nothing to apologise for.
        return;
    };

    let entry = match &result {
        Ok(response) => HistoryEntry {
            spec: spec.clone(),
            status: Some(response.status),
            millis: response.elapsed.as_millis() as u64,
        },
        Err(_) => HistoryEntry {
            spec: spec.clone(),
            status: None,
            millis: 0,
        },
    };
    store.history.update(|h| {
        h.insert(0, entry);
        h.truncate(HISTORY_LIMIT);
    });

    store.edit(tab, |t| {
        t.outcome = match result {
            Ok(response) => Outcome::Done(response),
            // A worker that was cancelled never reaches here — its continuation
            // is dropped — so this arm is only for a race: the flag went up
            // after the payload was already on its way.
            Err(message) if message == http::CANCELLED => {
                Outcome::Cancelled(CancelCause::Asked.note().to_string())
            }
            Err(message) => Outcome::Failed(message),
        };
    });
}

/// Stop the tab's request, and say why in the pane.
///
/// The outcome is written **here** rather than in the continuation, because a
/// cancelled task has no continuation left to run.
pub fn cancel(store: &Store, tab: TabId, cause: CancelCause) -> bool {
    let handle = store.inflight.update(|f| f.handles.remove(&tab));
    let Some(handle) = handle else {
        return false;
    };
    handle.cancel();
    store.inflight.update(|f| f.cancelled += 1);
    store.edit(tab, |t| {
        t.outcome = Outcome::Cancelled(cause.note().to_string());
    });
    true
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

/// Show tab `index`, cancelling whatever the tab being left was doing.
pub fn activate(store: &Store, index: usize) {
    let count = store.tabs.peek_with(Vec::len);
    if count == 0 {
        return;
    }
    let index = index.min(count - 1);
    let current = store.active.peek();
    if current == index {
        return;
    }
    if let Some(leaving) = store.tabs.peek_with(|tabs| tabs.get(current).map(|t| t.id)) {
        cancel(store, leaving, CancelCause::LeftTab);
    }
    store.active.set(index);
}

/// Open `spec` in a new tab and show it. Returns the new tab's id.
pub fn open(store: &Store, spec: RequestSpec) -> TabId {
    let id = store.take_id();
    store
        .tabs
        .update(|tabs| tabs.push(RequestTab::new(id, spec)));
    let last = store.tabs.peek_with(Vec::len) - 1;
    activate(store, last);
    id
}

/// Close tab `index`, stopping its request first.
///
/// The last tab is never closed: a window with no request in it has nothing to
/// show and no way back.
pub fn close(store: &Store, index: usize) -> bool {
    let count = store.tabs.peek_with(Vec::len);
    if count <= 1 || index >= count {
        return false;
    }
    if let Some(id) = store.tabs.peek_with(|tabs| tabs.get(index).map(|t| t.id)) {
        cancel(store, id, CancelCause::Closed);
    }
    store.tabs.update(|tabs| {
        tabs.remove(index);
    });
    let active = store.active.peek();
    // Keep looking at the same request when one before it disappears.
    let next = if active > index {
        active - 1
    } else {
        active.min(count - 2)
    };
    store.active.set(next);
    true
}

// ---------------------------------------------------------------------------
// Samples
// ---------------------------------------------------------------------------

/// The saved requests the sidebar shows, all pointing at `base`.
///
/// Six of them, one per route of the bundled server, and between them they
/// produce every state the response pane can be in: a body, an error status, a
/// slow answer worth cancelling, and — the last one — a connection that is
/// refused, which is how the "a network error is tidy" claim is demonstrated
/// with one click rather than by unplugging anything.
pub fn samples(base: &str) -> Vec<RequestSpec> {
    vec![
        RequestSpec {
            method: Method::Get,
            url: format!("{base}/ok"),
            headers: "Accept: application/json\n".to_string(),
            body: String::new(),
        },
        RequestSpec {
            method: Method::Get,
            url: format!("{base}/slow?ms=2500"),
            headers: "Accept: application/json\n".to_string(),
            body: String::new(),
        },
        RequestSpec {
            method: Method::Post,
            url: format!("{base}/echo"),
            headers: "Content-Type: application/json\nAccept: application/json\n".to_string(),
            body: "{\n  \"customer\": \"Rahayu\",\n  \"amount\": 250000\n}".to_string(),
        },
        RequestSpec {
            method: Method::Get,
            url: format!("{base}/status/503"),
            headers: String::new(),
            body: String::new(),
        },
        RequestSpec {
            method: Method::Get,
            url: format!("{base}/headers"),
            headers: "X-Trace-Id: 7f3a\nAccept: application/json\n".to_string(),
            body: String::new(),
        },
        // Port 9 is the discard port, and nothing listens on it anywhere.
        RequestSpec::get("http://127.0.0.1:9/unreachable"),
    ]
}

/// The names the sidebar puts on [`samples`].
pub const SAMPLE_NAMES: [&str; 6] = [
    "Small JSON",
    "Slow (2.5 s)",
    "Echo a body",
    "Server error",
    "Header check",
    "Refused connection",
];

// ---------------------------------------------------------------------------
// UI-only state that other modules need to name
// ---------------------------------------------------------------------------

/// The two panes that are wrapped in a panic boundary.
///
/// Named rather than boolean because the hidden test switch has to say *which*
/// panel it is breaking, and a fallback that appeared in both would prove half
/// as much.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    /// The request editor.
    Request,
    /// The response viewer.
    Response,
}

impl Panel {
    /// The boundary label, which is what a [`PanicReport`](silka_core::recover::PanicReport)
    /// is filed under.
    pub fn boundary(self) -> &'static str {
        match self {
            Panel::Request => "api-client:request",
            Panel::Response => "api-client:response",
        }
    }
}

/// The `select` state for the method picker.
///
/// One for the whole window rather than one per tab: only one tab's picker is
/// on screen at a time, and the *selection* is not stored here anyway — it is
/// read from the tab's [`RequestSpec`] on every build, which is what keeps the
/// spec the single source of truth.
pub type MethodPicker = Signal<SelectState>;

/// The sidebar's tree state, re-exported so the modules that need it do not all
/// have to reach into `silka_widgets`.
pub type Sidebar = TreeState;

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::signals::Runtime;

    fn store() -> (Runtime, Store) {
        let rt = Runtime::new();
        let store = Store::install(&rt, "http://127.0.0.1:9100");
        (rt, store)
    }

    #[test]
    fn the_application_opens_on_one_tab_pointing_at_the_bundled_server() {
        let (_rt, store) = store();
        assert_eq!(store.tabs.peek_with(Vec::len), 1);
        let tab = store.current().expect("a tab");
        assert_eq!(tab.spec.method, Method::Get);
        assert!(tab.spec.url.ends_with("/ok"));
        assert_eq!(tab.outcome, Outcome::Blank);
        assert_eq!(tab.title, "ok");
    }

    #[test]
    fn opening_a_request_adds_a_tab_and_shows_it() {
        let (_rt, store) = store();
        let id = open(&store, RequestSpec::get("http://127.0.0.1:1/two"));
        assert_eq!(store.tabs.peek_with(Vec::len), 2);
        assert_eq!(store.active.peek(), 1);
        assert_eq!(store.current_id(), Some(id));
    }

    #[test]
    fn the_last_tab_cannot_be_closed_and_closing_keeps_the_eye_on_the_same_request() {
        let (_rt, store) = store();
        let a = store.current_id().expect("a tab");
        let b = open(&store, RequestSpec::get("http://127.0.0.1:1/b"));
        let c = open(&store, RequestSpec::get("http://127.0.0.1:1/c"));
        assert_eq!(store.current_id(), Some(c));

        // Closing a tab before the active one keeps the active one showing.
        assert!(close(&store, 1));
        assert_eq!(store.current_id(), Some(c));
        assert_eq!(store.tabs.peek_with(Vec::len), 2);
        let _ = b;

        assert!(close(&store, 1));
        assert_eq!(store.current_id(), Some(a));
        // And now there is one left, which stays.
        assert!(!close(&store, 0));
        assert_eq!(store.tabs.peek_with(Vec::len), 1);
    }

    #[test]
    fn switching_to_the_tab_already_showing_does_nothing_at_all() {
        let (_rt, store) = store();
        open(&store, RequestSpec::get("http://127.0.0.1:1/b"));
        store.edit(store.current_id().unwrap(), |t| {
            t.outcome = Outcome::Sending
        });
        activate(&store, 1);
        // Still `Sending`: a no-op switch must not cancel the request the user
        // is watching.
        assert!(store.current().unwrap().outcome.is_sending());
    }

    #[test]
    fn cancelling_a_tab_with_nothing_in_flight_is_a_no_op() {
        let (_rt, store) = store();
        let id = store.current_id().expect("a tab");
        assert!(!cancel(&store, id, CancelCause::Asked));
        assert_eq!(store.current().unwrap().outcome, Outcome::Blank);
    }

    #[test]
    fn every_sample_has_a_name_and_a_url_the_client_can_parse() {
        let samples = samples("http://127.0.0.1:9100");
        assert_eq!(samples.len(), SAMPLE_NAMES.len());
        for (spec, name) in samples.iter().zip(SAMPLE_NAMES) {
            assert!(!name.is_empty());
            http::Url::parse(&spec.url).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        // The last one is the deliberate failure, and it must not be pointed at
        // the bundled server or it would succeed.
        assert!(samples.last().unwrap().url.contains(":9/"));
    }

    #[test]
    fn an_outcome_says_what_it_is_without_the_pane_having_to_guess() {
        assert_eq!(Outcome::Blank.summary(), "Ready");
        assert!(Outcome::Sending.is_sending());
        assert_eq!(
            Outcome::Cancelled(CancelCause::LeftTab.note().to_string()).summary(),
            "Cancelled"
        );
        let done = Outcome::Done(Response {
            status: 201,
            reason: "Created".into(),
            elapsed: std::time::Duration::from_millis(7),
            ..Response::default()
        });
        assert_eq!(done.summary(), "201 Created · 7 ms");
        assert!(done.response().is_some());
    }
}
