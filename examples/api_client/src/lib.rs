//! # silka-api-client — a Postman-shaped HTTP client
//!
//! The application that finally *uses* [`silka_core::task`] and
//! [`silka_core::recover`]. Both were born in Phase 5 with unit tests and no
//! customers: the ERP dashboard, the flagship example, holds static data, and
//! the notes editor's disk writes never fail, never take long enough to see,
//! and are never abandoned half-way. Everything the async story and the panic
//! policy claim was, until this crate existed, claimed about code nobody had
//! pointed at a network.
//!
//! ```text
//! cargo run -p silka-api-client
//! cargo run -p silka-api-client -- --port 9100
//! cargo run -p silka-api-client -- --url http://localhost:3000 --no-server
//! cargo run -p silka-api-client -- --preset tailwind --appearance light
//! ```
//!
//! ## The four claims, and where each is proved
//!
//! | Claim | In the application | In a test |
//! |---|---|---|
//! | A loading state is **visible**, and the window is not frozen behind it | [`response`] draws an indeterminate bar, a Cancel button and a live status line; the frame loop asks for another frame while anything is in flight ([`app::run`]) | `a_slow_request_shows_a_loading_state_the_whole_window_stays_live_behind` |
//! | A network error is **tidy** | [`state::Outcome::Failed`] carries a sentence; the pane draws it in a card | `a_refused_connection_lands_as_a_readable_card_not_a_crash` |
//! | **Cancellation** actually stops the work | [`state::activate`] cancels the tab being left; [`http::send`] checks the flag between reads | `switching_tabs_stops_the_request_it_left_behind` |
//! | A **panic** in one panel does not take the window with it | [`silka_core::recover::catch`] around the request pane, `guard_view_or` around the response pane | `a_panicking_panel_is_replaced_by_a_card_and_everything_else_keeps_working` |
//!
//! ## What it talks to
//!
//! A loopback server that ships inside the binary ([`serve`]), because the
//! three conditions above are conditions no public endpoint will perform on
//! request. `--url` points it at anything else. The reasoning — including why
//! there is no HTTP crate and no TLS — is written down in the [`http`] module,
//! where the decision lives.
//!
//! ## What this application found out — the point of writing it
//!
//! Two things, and neither of them was visible from a unit test.
//!
//! **1. The async bridge and the panic boundary hold up.** `spawn_blocking`,
//! `Cancel`, `TaskHandle::cancel`, `Tasks::deliver`'s scope check, `catch` and
//! `guard_view_or` all behaved exactly as their documentation says, against a
//! real socket, with real latency, and with a request being abandoned
//! mid-flight. Nothing in `silka-core` had to change for this crate to exist.
//!
//! **2. Layout cost multiplies by roughly five per nested flex container.**
//! This one is a genuine defect and it is recorded here because this
//! application is where it surfaced. Measured in a debug build, on the very
//! `form` this window uses:
//!
//! ```text
//! form([field("Headers", text_area(…))])                 44 ms
//!   wrapped in one more column                          220 ms
//!   wrapped in two                                      1.09 s
//!   wrapped in three                                    5.44 s
//! ```
//!
//! A bare `text_area` at the same depths costs 0.38 ms → 0.78 ms, so the
//! multiplier is not the leaf: it is a flex container that measures its
//! children and then lays them out, with a single-slot layout cache
//! (`layout_node` memoises exactly one `(constraints, boundary)` pair) that
//! every alternating pass invalidates. The first assembly of this window
//! rebuilt in **2.65 s**.
//!
//! Two things brought that to **216 ms**, and both are written down where they
//! are done rather than only here:
//!
//! - a `scroll_view` around each pane's content — it hands its child a constant
//!   constraint, so the ancestors' re-measure passes stop there. It made the
//!   cost of a subtree **flat** in depth (44 ms at every depth from 0 to 4);
//! - one fewer flex container between the scroll view and the form
//!   ([`silka_core::view::pad`] instead of a column with padding), which alone
//!   took the request pane from 255 ms to 65 ms.
//!
//! Both are good design on their own — a request editor should scroll — but
//! neither is a fix. The fix is a layout cache that remembers more than one
//! entry per node, and it belongs in `silka-core`, not in an example.
//!
//! ## Known limits, stated rather than hidden
//!
//! - **No `https`.** [`http::Url::parse`] says so in a sentence. Adding a TLS
//!   transport behind [`http::send`] would change nothing above that function.
//! - **No chunked transfer-encoding.** The client asks for `Connection: close`
//!   and reads to EOF, which covers every server that answers with a
//!   `Content-Length` and every one that closes; a chunked response arrives with
//!   its framing still in the body, visibly.
//! - **The response body is shaped in full.** `text_area` shapes the whole
//!   document rather than the visible part (its own recorded debt), so a
//!   ten-megabyte response is slow to *open* — the client caps a body at
//!   [`http::MAX_BODY`] rather than pretending otherwise.
//! - **Editing a request rebuilds the shell**, because the tab row genuinely
//!   shows the method and the URL. The panes below it are still separate
//!   boundaries; see [`request`] for why the popup forces the split.
//! - **Requests are not saved.** The sidebar's saved list is
//!   [`state::samples`], compiled in. Persistence is a story this example does
//!   not need to tell twice — the notes example already tells it.

pub mod app;
pub mod http;
pub mod request;
pub mod response;
pub mod serve;
pub mod sidebar;
pub mod state;

#[cfg(test)]
mod tests;
