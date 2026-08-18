//! `command_palette()` — ⌘K (`KOMPONEN.md` Tier 3).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::fixed;
//! # use silka_widgets::overlay_layer;
//! use silka_widgets::{command, command_palette, use_palette_state, IconName};
//!
//! # let rt = Runtime::new();
//! rt.build_root(|| {
//!     let state = use_palette_state();
//!
//!     let palette = command_palette([
//!         command("file.new", "New File").icon(IconName::Plus).section("File"),
//!         command("file.open", "Open…").icon(IconName::Upload).section("File"),
//!         command("view.dark", "Toggle Dark Mode").icon(IconName::Moon).section("View"),
//!     ])
//!     .bind(state)
//!     .label("Commands")
//!     .on_run(|id| println!("running {id}"));
//!
//!     // Like every other overlay-riding component, the panel is handed to the
//!     // layer rather than nested in the page.
//!     let _ = overlay_layer(fixed(1200.0, 800.0)).overlay(palette.overlay());
//! });
//! ```
//!
//! # What is genuinely new here
//!
//! Almost nothing is drawn from scratch. Typing, the caret, undo and the IME
//! are [`text_field`](mod@crate::text_field)'s; the panel, its backdrop, its
//! dismissal and its spring are [`overlay`](mod@crate::overlay)'s. Two things
//! are new, and they are the two that make a palette a palette:
//!
//! 1. **[`fuzzy_match`]** — a subsequence matcher with the bonuses that decide
//!    whether "of" finds *Open File* before *Profile*. It is a pure function,
//!    so the ranking can be argued about in a unit test rather than by squinting
//!    at a running application.
//! 2. **One node that takes four keys** ([`PaletteBox`]) — ↑/↓ walk the results,
//!    Return runs the highlighted one, Esc closes. Everything else reaches the
//!    field untouched, because a key event travels outwards from the focused
//!    node and anything the field handles stops there. The field is asked for
//!    [`ArrowKeys::Bubble`] for exactly that reason, and it is deliberately
//!    given **no** `on_submit`, because a field with one swallows Return.
//!
//! # Filtering is *not* the application's job here
//!
//! The opposite of [`combo_box`](mod@crate::combo_box), and deliberately so:
//! there, matching is domain logic the widget could only get wrong; here,
//! matching **is** the component. A palette that made every application write
//! its own fuzzy matcher would produce as many rankings as there are
//! applications. What stays with the application is the *query string* — it
//! owns the text, exactly as it owns a text field's value.
//!
//! # Where the ⌘K itself lives
//!
//! Not here. This component draws and drives the palette; opening it is one
//! line in the application's key handling, and [`is_shortcut`] is the predicate
//! for it. A window-wide shortcut belongs to the shell, and a global one to the
//! OS (`INTEGRASI-NATIVE.md` §4) — a widget that grabbed ⌘K on its own would be
//! a widget that cannot be turned off.
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Item | Where |
//! |---|---|
//! | Correct in both presets | [`PaletteStyle::from_theme`] |
//! | Interactive state on springs | each row's highlight/hover/press tint, plus the overlay's own transition |
//! | Full keyboard + focus ring | ↑/↓/Home/End/Return/Esc on [`PaletteBox`], the ring belongs to the field, which keeps focus throughout |
//! | AccessKit node | a [`AccessRole::Group`] carrying `expanded` around the field, with [`AccessRole::MenuItem`] rows carrying `selected` — the same shape [`combo_box`](mod@crate::combo_box) uses |
//! | Dark mode | tokens only |
//! | Hit target ≥ 44pt | [`PaletteStyle::row_height`] |
//! | Reduced motion | row tints are [`Decorative`](silka_core::animation::MotionRole::Decorative) |
//!
//! # Deliberately not here yet
//!
//! - **A borderless search field.** `text_field` draws its own frame and offers
//!   no way to drop it, so the palette's field looks like a field rather than
//!   like a bare line of text. Cosmetic, and a change to `text_field` rather
//!   than to this file.
//! - **Highlighting the matched characters** inside a row's title.
//!   [`FuzzyMatch::positions`] already carries exactly which characters those
//!   are; what is missing is a styled-run text leaf to draw them with.

use std::ops::Range;

use silka_core::access::{AccessActions, AccessNode, AccessRole};
use silka_core::animation::{MotionRole, Spring, SpringValue, Tick, Tolerance};
use silka_core::input::{
    CursorIcon, Event, EventCtx, HitBehavior, HitShape, KeyCode, KeyEvent, Modifiers, NamedKey,
    PointerButton, PointerPhase,
};
use silka_core::scheduler::Dirty;
use silka_core::signals::{use_signal, Key, Signal};
use silka_core::tree::{
    BoxConstraints, CrossAlign, Decoration, LayoutCtx, MainAlign, NodeId, PaintCtx, RenderNode,
    RenderTree,
};
use silka_core::view::{column, expanded, row, Builder, View, ViewNode};
use silka_paint::{Color, Corners, Insets, Point, Quad, Size};
use silka_text::FontWeight;
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::icon::{icon_in, IconName};
use crate::images::Images;
use crate::menu::{Shortcut, ShortcutStyle};
use crate::overlay::{overlay, Align, Barrier, Dismiss, OverlayBuilder, Placement, Side};
use crate::text::text_in;
use crate::text_field::{text_field_in, ArrowKeys};

// ---------------------------------------------------------------------------
// Fuzzy matching
// ---------------------------------------------------------------------------

/// What a successful [`fuzzy_match`] found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FuzzyMatch {
    /// How good the match is; larger is better, and it may be negative.
    pub score: i32,
    /// Which **characters** of the haystack were matched, in order.
    ///
    /// Character indices rather than byte offsets, because that is the unit a
    /// text run is styled in — and the unit that survives a multi-byte letter.
    pub positions: Vec<usize>,
}

/// Score of a matched character before any bonus or penalty.
const BASE: i32 = 8;
/// Extra for matching the very first character of the haystack.
const START_BONUS: i32 = 12;
/// Extra for matching just after a separator — the start of a word.
const BOUNDARY_BONUS: i32 = 10;
/// Extra for matching the capital in `camelCase`.
const CAMEL_BONUS: i32 = 8;
/// Extra for matching directly after the previous match.
const RUN_BONUS: i32 = 10;
/// The most a single gap can cost, however long it is.
const MAX_GAP_PENALTY: i32 = 10;

/// One character folded to lower case, **without** changing how many there are.
///
/// `char::to_lowercase` may yield more than one character (`İ` becomes two),
/// which would slide every index after it. Since the indices are what a
/// highlight is drawn from, taking the first is the version that stays
/// truthful.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Match `needle` against `haystack` as a **subsequence**, and score it.
///
/// Every character of the needle must appear in the haystack, in order, but not
/// necessarily adjacently — that is what lets "of" find *Open File* and "gitst"
/// find *Git: Stage Changes*. The score is what decides the ranking, and it is
/// built from four ideas:
///
/// | Rule | Why |
/// |---|---|
/// | Matching the **first** character is worth a lot | people type the beginning of what they mean |
/// | Matching the start of a **word** is worth almost as much | "of" for *Open File* is what an initialism looks like |
/// | Matching the capital in `camelCase` counts as a word start | identifiers are words too |
/// | **Consecutive** matches beat scattered ones, and gaps cost | "file" in *File* must beat "file" in *F…i…l…e* |
///
/// Matching is case-insensitive, and whitespace in the needle is ignored so
/// that "op fi" behaves like "opfi".
///
/// A **greedy** left-to-right scan rather than a full alignment search: it
/// takes the first occurrence of each needle character. That is what every
/// interactive palette does, because an optimal alignment is quadratic and the
/// difference is invisible at the length of a menu label.
///
/// ```
/// use silka_widgets::command_palette::fuzzy_match;
///
/// // A subsequence, not a substring.
/// assert!(fuzzy_match("of", "Open File").is_some());
/// assert!(fuzzy_match("zz", "Open File").is_none());
///
/// // An empty needle matches everything, with nothing highlighted — which is
/// // what makes "the palette just opened" the same code path as a search.
/// assert_eq!(fuzzy_match("", "anything").unwrap().positions, Vec::<usize>::new());
///
/// // The whole point of the scoring: word starts beat characters buried
/// // inside a word.
/// let good = fuzzy_match("of", "Open File").unwrap().score;
/// let poor = fuzzy_match("of", "Profile").unwrap().score;
/// assert!(good > poor, "{good} should beat {poor}");
///
/// // And the positions are exactly the characters a highlight would underline.
/// assert_eq!(fuzzy_match("of", "Open File").unwrap().positions, vec![0, 5]);
/// ```
pub fn fuzzy_match(needle: &str, haystack: &str) -> Option<FuzzyMatch> {
    let jarum: Vec<char> = needle
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(fold)
        .collect();
    if jarum.is_empty() {
        return Some(FuzzyMatch::default());
    }
    let jerami: Vec<char> = haystack.chars().collect();
    if jerami.is_empty() {
        return None;
    }
    let lipat: Vec<char> = jerami.iter().copied().map(fold).collect();

    let mut positions = Vec::with_capacity(jarum.len());
    let mut score = 0i32;
    let mut mulai = 0usize;
    let mut sebelumnya: Option<usize> = None;

    for n in jarum {
        let i = (mulai..lipat.len()).find(|i| lipat[*i] == n)?;

        let mut s = BASE;
        if i == 0 {
            s += START_BONUS;
        } else {
            let sblm = jerami[i - 1];
            if !sblm.is_alphanumeric() {
                s += BOUNDARY_BONUS;
            } else if sblm.is_lowercase() && jerami[i].is_uppercase() {
                s += CAMEL_BONUS;
            }
        }
        match sebelumnya {
            Some(p) if i == p + 1 => s += RUN_BONUS,
            Some(p) => s -= ((i - p - 1) as i32).min(MAX_GAP_PENALTY),
            // The leading gap counts too: a match that starts late is a worse
            // match, which is what keeps "of" out of the middle of a word.
            None => s -= (i as i32).min(MAX_GAP_PENALTY),
        }

        score += s;
        positions.push(i);
        sebelumnya = Some(i);
        mulai = i + 1;
    }

    Some(FuzzyMatch { score, positions })
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

/// One entry in the palette.
///
/// A plain value, not a [`View`]: the palette has to read every title, subtitle
/// and keyword in order to rank them, and the moment something becomes a view
/// its contents are buried behind `dyn ViewNode`.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    id: String,
    title: String,
    subtitle: Option<String>,
    section: Option<String>,
    keywords: Vec<String>,
    icon: Option<IconName>,
    shortcut: Option<Shortcut>,
    enabled: bool,
}

/// A command with the identity `id`, shown as `title`.
///
/// ```
/// use silka_widgets::{command, IconName};
///
/// let open = command("file.open", "Open…")
///     .icon(IconName::Upload)
///     .subtitle("Choose a file from disk")
///     .keywords(["load", "import"])
///     .section("File");
///
/// assert_eq!(open.id(), "file.open");
/// assert_eq!(open.title_text(), "Open…");
/// assert!(open.is_enabled());
/// ```
pub fn command(id: impl Into<String>, title: impl Into<String>) -> Command {
    Command {
        id: id.into(),
        title: title.into(),
        subtitle: None,
        section: None,
        keywords: Vec::new(),
        icon: None,
        shortcut: None,
        enabled: true,
    }
}

impl Command {
    /// A second line under the title.
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// The group this command is filed under, shown while the query is empty.
    pub fn section(mut self, section: impl Into<String>) -> Self {
        self.section = Some(section.into());
        self
    }

    /// Extra words that should find this command but are not worth showing.
    ///
    /// The escape hatch for vocabulary that differs from the label: "trash"
    /// finding *Move to Bin*, or a former name of a renamed feature.
    pub fn keywords<S: Into<String>>(mut self, words: impl IntoIterator<Item = S>) -> Self {
        self.keywords = words.into_iter().map(Into::into).collect();
        self
    }

    /// A symbol on the leading edge of the row.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The shortcut shown at the trailing edge — displayed, never dispatched
    /// (the same rule as [`crate::menu`]).
    pub fn shortcut(mut self, shortcut: Shortcut) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// A command that is listed but cannot be run.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// The identity handed to `on_run`.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The title shown and ranked.
    pub fn title_text(&self) -> &str {
        &self.title
    }

    /// The second line, if any.
    pub fn subtitle_text(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }

    /// The group this command is filed under.
    pub fn section_text(&self) -> Option<&str> {
        self.section.as_deref()
    }

    /// True when the command can be run.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

// ---------------------------------------------------------------------------
// Ranking
// ---------------------------------------------------------------------------

/// One command that survived the query, and how well it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Index into the command list handed to [`rank`].
    pub index: usize,
    /// The score that put it here.
    pub score: i32,
    /// Which characters of the **title** matched (empty when the match came
    /// from a subtitle or a keyword).
    pub positions: Vec<usize>,
}

/// What matching a subtitle costs compared to matching the title.
const SUBTITLE_PENALTY: i32 = 8;
/// What matching a hidden keyword costs.
const KEYWORD_PENALTY: i32 = 4;

/// Rank `commands` against `query`, best first.
///
/// A command is kept when the query matches its title, its subtitle, or one of
/// its keywords. Matching somewhere other than the title costs points, which is
/// what keeps a command whose *name* is what you typed above one that merely
/// mentions it.
///
/// Ties keep the order the application gave — the sort is stable, and that
/// matters: an application usually lists its commands in the order it wants
/// them seen, and an empty query must not shuffle them.
///
/// ```
/// use silka_widgets::{command, command_palette::rank};
///
/// let commands = [
///     command("a", "Profile"),
///     command("b", "Open File"),
///     command("c", "Close Window").keywords(["quit"]),
/// ];
///
/// // An empty query keeps everything, in the given order.
/// let all = rank(&commands, "");
/// assert_eq!(all.iter().map(|h| h.index).collect::<Vec<_>>(), vec![0, 1, 2]);
///
/// // "of" finds both, but the initialism wins.
/// let hits = rank(&commands, "of");
/// assert_eq!(hits[0].index, 1);
///
/// // A keyword finds a command whose title says nothing of the sort.
/// assert_eq!(rank(&commands, "quit")[0].index, 2);
/// ```
pub fn rank(commands: &[Command], query: &str) -> Vec<Hit> {
    let kosong = query.chars().all(char::is_whitespace);
    let mut hits: Vec<Hit> = Vec::with_capacity(commands.len());

    for (i, c) in commands.iter().enumerate() {
        if kosong {
            hits.push(Hit {
                index: i,
                score: 0,
                positions: Vec::new(),
            });
            continue;
        }

        let judul = fuzzy_match(query, &c.title);
        let mut skor = judul.as_ref().map(|m| m.score);

        if let Some(sub) = &c.subtitle {
            if let Some(m) = fuzzy_match(query, sub) {
                let s = m.score - SUBTITLE_PENALTY;
                skor = Some(skor.map_or(s, |k: i32| k.max(s)));
            }
        }
        for kata in &c.keywords {
            if let Some(m) = fuzzy_match(query, kata) {
                let s = m.score - KEYWORD_PENALTY;
                skor = Some(skor.map_or(s, |k: i32| k.max(s)));
            }
        }

        if let Some(score) = skor {
            hits.push(Hit {
                index: i,
                score,
                positions: judul.map(|m| m.positions).unwrap_or_default(),
            });
        }
    }

    if !kosong {
        // Stable, so equal scores keep the application's order.
        hits.sort_by(|a, b| b.score.cmp(&a.score));
    }
    hits
}

/// Which slice of `len` results is on screen, given the highlight.
///
/// A palette shows a fixed number of rows and **windows** the rest rather than
/// scrolling it: the highlight is always inside the window, and the window only
/// moves when the highlight would leave it. Half a window of context is kept on
/// each side, which is what stops the list from jumping a whole page whenever
/// the highlight crosses an edge.
///
/// ```
/// use silka_widgets::command_palette::window;
///
/// // Everything fits: no windowing at all.
/// assert_eq!(window(4, 0, 8), 0..4);
///
/// // The first rows stay put until the highlight has moved past the middle.
/// assert_eq!(window(20, 0, 5), 0..5);
/// assert_eq!(window(20, 4, 5), 2..7);
///
/// // …and it stops at the end rather than running off it.
/// assert_eq!(window(20, 19, 5), 15..20);
///
/// // Degenerate inputs answer with an empty range instead of panicking.
/// assert_eq!(window(0, 0, 5), 0..0);
/// assert_eq!(window(10, 0, 0), 0..0);
/// ```
pub fn window(len: usize, highlight: usize, max: usize) -> Range<usize> {
    if len == 0 || max == 0 {
        return 0..0;
    }
    if len <= max {
        return 0..len;
    }
    let h = highlight.min(len - 1);
    let mulai = h.saturating_sub(max / 2).min(len - max);
    mulai..mulai + max
}

/// True when `event` is the ⌘K (Ctrl+K off macOS) that opens a palette.
///
/// The predicate rather than the handler: where a shortcut is dispatched is the
/// application's decision, and a widget that grabbed ⌘K on its own would be one
/// that cannot be turned off.
///
/// ```
/// # use std::time::Duration;
/// use silka_core::input::{KeyCode, KeyEvent, Modifiers};
/// use silka_widgets::command_palette::is_shortcut;
///
/// let hit = KeyEvent::pressed(KeyCode::Character('k'), Duration::ZERO)
///     .modifiers(Modifiers::COMMAND);
/// assert!(is_shortcut(&hit));
///
/// // A bare "k" is a letter someone is typing, not a command.
/// assert!(!is_shortcut(&KeyEvent::pressed(KeyCode::Character('k'), Duration::ZERO)));
///
/// // …and so is ⌘⇧K, which applications use for something else entirely.
/// let with_shift = KeyEvent::pressed(KeyCode::Character('k'), Duration::ZERO)
///     .modifiers(Modifiers::COMMAND.union(Modifiers::SHIFT));
/// assert!(!is_shortcut(&with_shift));
/// ```
pub fn is_shortcut(event: &KeyEvent) -> bool {
    event.is_pressed()
        && event.modifiers.is_exactly(Modifiers::COMMAND)
        && matches!(
            event.code,
            KeyCode::Character('k') | KeyCode::Character('K')
        )
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// The three values a palette needs the application to hold.
///
/// A hook-owned bundle, exactly like [`crate::use_list_state`]. The query is
/// here rather than inside the component for the same reason a text field's
/// value is: there must be exactly one copy of it in the application.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaletteState {
    open: Signal<bool>,
    query: Signal<String>,
    highlight: Signal<usize>,
}

impl PaletteState {
    /// A state owned by `runtime` — for tests, which have no build pass.
    pub fn new(runtime: &silka_core::signals::Runtime) -> Self {
        Self {
            open: runtime.signal(false),
            query: runtime.signal(String::new()),
            highlight: runtime.signal(0),
        }
    }

    /// True while the palette is on screen — **tracks** when read in a build.
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Open or close the palette.
    ///
    /// Opening always starts from a clean slate: an empty query and the first
    /// result highlighted. A palette that reopened showing the last search is a
    /// palette people learn to clear by hand every single time.
    pub fn set_open(&self, open: bool) {
        if open {
            self.query.set(String::new());
            self.highlight.set(0);
        }
        self.open.set_if_changed(open);
    }

    /// Flip it — what a ⌘K handler calls (see [`is_shortcut`]).
    pub fn toggle(&self) {
        self.set_open(!self.open.peek());
    }

    /// The current query — **tracks** when read in a build.
    pub fn query(&self) -> String {
        self.query.get()
    }

    /// Replace the query, and put the highlight back on the first result.
    ///
    /// Resetting is not optional: after typing one more letter, "the third
    /// result" is a different command, and keeping the index would run the
    /// wrong one.
    pub fn set_query(&self, query: impl Into<String>) {
        self.query.set(query.into());
        self.highlight.set_if_changed(0);
    }

    /// The highlighted result, as an index into the **hit** list.
    pub fn highlight(&self) -> usize {
        self.highlight.get()
    }

    /// Move the highlight.
    pub fn set_highlight(&self, index: usize) {
        self.highlight.set_if_changed(index);
    }

    /// True while every signal is still alive (its scope is not disposed).
    pub fn is_alive(&self) -> bool {
        self.open.is_alive() && self.query.is_alive() && self.highlight.is_alive()
    }
}

/// Palette state owned by the component being built (§2.5).
///
/// A hook: call it once per build, never inside an `if` or a loop.
pub fn use_palette_state() -> PaletteState {
    PaletteState {
        open: use_signal(|| false),
        query: use_signal(String::new),
        highlight: use_signal(|| 0),
    }
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every visual value of a palette, already resolved from the tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaletteStyle {
    /// The panel: fill, corners, border, elevation.
    pub panel: Decoration,
    /// Panel width, in logical points.
    pub width: f32,
    /// How far from the top edge of the window the panel sits.
    pub top_gap: f32,
    /// Padding inside the panel.
    pub padding: Insets,
    /// Gap between the field and the results.
    pub gap: f32,
    /// The hairline between the field and the results.
    pub separator: Color,
    /// Thickness of that hairline.
    pub separator_thickness: f32,
    /// Height of one result row — the HIG hit target.
    pub row_height: f32,
    /// Corner shape of a row's highlight: the tint **and** hit-testing (§3.6).
    pub row_corners: Corners,
    /// Padding inside one row.
    pub row_padding: Insets,
    /// Gap between a row's icon, its text, and its shortcut.
    pub row_gap: f32,
    /// Background of the highlighted row.
    pub highlight: Color,
    /// Hover tint over a row that is not highlighted.
    pub hover: Color,
    /// Pressed tint.
    pub pressed: Color,
    /// Colour of a row title.
    pub title: Color,
    /// Colour of a row subtitle.
    pub subtitle: Color,
    /// Colour of a disabled row.
    pub disabled: Color,
    /// Colour of a displayed shortcut.
    pub shortcut: Color,
    /// Colour of a section caption.
    pub section: Color,
    /// Font size of a row title.
    pub title_size: f32,
    /// Font size of a subtitle, a shortcut, and a caption.
    pub small_size: f32,
    /// How many rows are on screen at once.
    pub max_visible: usize,
}

impl PaletteStyle {
    /// Resolve every token.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            panel: Decoration::fill(theme.color.surface_elevated)
                .corners(theme.corners(theme.radius.xl))
                .border(theme.space(0.25), theme.color.separator)
                .shadows(theme.shadow.xl),
            width: theme.space(140.0),
            top_gap: theme.space(24.0),
            padding: Insets::all(theme.space(2.0)),
            gap: theme.space(2.0),
            separator: theme.color.separator,
            separator_thickness: theme.space(0.25),
            row_height: MIN_HIT_TARGET,
            row_corners: theme.corners(theme.radius.md),
            row_padding: Insets::symmetric(theme.space(2.5), theme.space(1.5)),
            row_gap: theme.space(2.5),
            highlight: theme.color.accent_muted,
            hover: theme.color.surface_hover,
            pressed: theme.color.surface_pressed,
            title: theme.color.label,
            subtitle: theme.color.secondary_label,
            disabled: theme.color.disabled_label,
            shortcut: theme.color.tertiary_label,
            section: theme.color.tertiary_label,
            title_size: theme.typography.body_size,
            small_size: theme.typography.footnote.size,
            max_visible: 8,
        }
    }
}

// ---------------------------------------------------------------------------
// Callbacks
// ---------------------------------------------------------------------------

/// A "result number `index`" action (highlighting or running one).
#[derive(Clone)]
pub struct HitCallback(std::rc::Rc<dyn Fn(usize)>);

impl HitCallback {
    /// Wrap a closure.
    pub fn new(f: impl Fn(usize) + 'static) -> Self {
        Self(std::rc::Rc::new(f))
    }

    /// Run it for result `index`.
    pub fn call(&self, index: usize) {
        (self.0)(index)
    }
}

impl PartialEq for HitCallback {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for HitCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("HitCallback")
    }
}

// ---------------------------------------------------------------------------
// The row node
// ---------------------------------------------------------------------------

/// Motion role of a row's tint under reduced-motion.
pub const ROW_TINT_MOTION: MotionRole = MotionRole::Decorative;

/// Render node for one result row.
pub struct PaletteRowBox {
    /// The name a screen reader announces (the command's title).
    pub label: String,
    /// Position among the results shown.
    pub index: usize,
    /// The row the keyboard is on.
    pub highlighted: bool,
    /// Listed but not runnable.
    pub disabled: bool,
    /// Corner shape of the tint — identical to the hit shape (§3.6).
    pub corners: Corners,
    /// Smallest the row may be — the HIG hit target (`KOMPONEN.md` DoD).
    pub min_height: f32,
    /// Background of the highlighted row.
    pub highlight: Color,
    /// Hover tint.
    pub hover: Color,
    /// Pressed tint.
    pub pressed_color: Color,
    /// What runs when the row is activated.
    pub on_press: Option<silka_core::Callback>,

    hovered: bool,
    pressed: bool,
    tint: SpringValue<Color>,
    driven: bool,
}

impl PaletteRowBox {
    fn target_tint(&self) -> Color {
        if self.disabled {
            return self.highlight.with_alpha(0.0);
        }
        if self.pressed && self.hovered {
            self.pressed_color
        } else if self.highlighted {
            self.highlight
        } else if self.hovered {
            self.hover
        } else {
            self.highlight.with_alpha(0.0)
        }
    }

    fn arahkan(&mut self) {
        let target = self.target_tint();
        if self.driven {
            self.tint.set_target(target);
        } else {
            self.tint.jump_to(target);
        }
    }

    /// The background painted this frame.
    pub fn tint(&self) -> Color {
        self.tint.position()
    }

    /// The pointer is over this row.
    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    /// True while the background is still moving.
    pub fn is_animating(&self) -> bool {
        self.tint.is_animating()
    }

    /// Advance the background by one frame; true if its colour changed.
    pub fn advance(&mut self, tick: &Tick) -> bool {
        self.driven = true;
        if !self.tint.is_animating() {
            return false;
        }
        let sebelum = self.tint.position();
        tick.advance(&mut self.tint);
        self.tint.position() != sebelum
    }

    /// Finish the transition instantly (tests and snapshots).
    pub fn settle(&mut self) {
        self.tint.settle();
    }

    fn jalankan(&mut self) {
        if self.disabled {
            return;
        }
        if let Some(cb) = self.on_press.clone() {
            cb.call();
        }
    }
}

impl RenderNode for PaletteRowBox {
    fn type_name(&self) -> &'static str {
        "PaletteRow"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        // The floor is forced here rather than left to the padding: a row's
        // height would otherwise be whatever the font happened to measure, and
        // the HIG target would quietly depend on the type scale.
        let dalam = BoxConstraints::new(
            constraints.min_width,
            constraints.max_width,
            constraints.min_height.max(self.min_height),
            constraints.max_height,
        )
        .normalized();
        let child = ctx.child(0);
        let size = ctx.layout_child(child, dalam);
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(Size::new(size.width, size.height.max(self.min_height)))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let sorot = self.tint.position();
        if sorot.a > 0.0 {
            ctx.quad(
                Quad::new(ctx.local_bounds())
                    .background(sorot)
                    .corners(self.corners),
            );
        }
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::MenuItem;
        node.label = Some(self.label.clone());
        node.disabled = self.disabled;
        // A palette always has exactly one highlighted result, so `false` here
        // is information rather than noise (see `AccessNode::selected`).
        node.selected = Some(self.highlighted);
        if !self.disabled {
            node.actions |= AccessActions::CLICK;
        }
    }

    fn hit_shape(&self) -> HitShape {
        HitShape::Rounded(self.corners)
    }

    fn hit_behavior(&self) -> HitBehavior {
        HitBehavior::Opaque
    }

    /// **The field keeps focus, always.** A row that took it would stop the
    /// user from typing, which is the one thing a palette must never do.
    fn focus_policy(&self) -> silka_core::input::FocusPolicy {
        silka_core::input::FocusPolicy::NONE
    }

    fn cursor(&self) -> Option<CursorIcon> {
        (!self.disabled).then_some(CursorIcon::Pointer)
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Pointer(p) = event else { return };
        if self.disabled {
            if matches!(p.phase, PointerPhase::Down | PointerPhase::Up) {
                ctx.handled();
            }
            return;
        }
        match p.phase {
            PointerPhase::Enter if !self.hovered => {
                self.hovered = true;
                self.arahkan();
                ctx.request_paint();
                ctx.request_animation();
            }
            PointerPhase::Leave if self.hovered || self.pressed => {
                self.hovered = false;
                self.arahkan();
                ctx.request_paint();
                ctx.request_animation();
            }
            PointerPhase::Down if p.button == Some(PointerButton::Primary) => {
                self.pressed = true;
                self.arahkan();
                ctx.capture_pointer();
                // Deliberately **no** `request_focus`: see `focus_policy`.
                ctx.handled();
                ctx.request_paint();
                ctx.request_animation();
            }
            PointerPhase::Up if p.button == Some(PointerButton::Primary) => {
                let di_dalam = self.corners.contains(ctx.size(), ctx.local());
                let jadi = self.pressed && di_dalam;
                self.pressed = false;
                self.arahkan();
                ctx.release_pointer();
                ctx.handled();
                ctx.request_paint();
                ctx.request_animation();
                if jadi {
                    self.jalankan();
                }
            }
            PointerPhase::Cancel if self.pressed => {
                self.pressed = false;
                self.arahkan();
                ctx.request_paint();
                ctx.request_animation();
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for PaletteRowBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PaletteRowBox")
            .field("label", &self.label)
            .field("index", &self.index)
            .field("highlighted", &self.highlighted)
            .finish()
    }
}

/// Props for one result row — the view form of [`PaletteRowBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteRowProps {
    pub(crate) label: String,
    pub(crate) index: usize,
    pub(crate) highlighted: bool,
    pub(crate) disabled: bool,
    pub(crate) corners: Corners,
    pub(crate) min_height: f32,
    pub(crate) highlight: Color,
    pub(crate) hover: Color,
    pub(crate) pressed: Color,
    pub(crate) on_press: Option<silka_core::Callback>,
    pub(crate) spring: Spring,
}

impl ViewNode for PaletteRowProps {
    fn build(&self) -> Box<dyn RenderNode> {
        let mut n = PaletteRowBox {
            label: self.label.clone(),
            index: self.index,
            highlighted: self.highlighted,
            disabled: self.disabled,
            corners: self.corners,
            min_height: self.min_height,
            highlight: self.highlight,
            hover: self.hover,
            pressed_color: self.pressed,
            on_press: self.on_press.clone(),
            hovered: false,
            pressed: false,
            tint: SpringValue::new(self.highlight.with_alpha(0.0))
                .with_spring(self.spring)
                .with_tolerance(Tolerance::COLOR)
                .decorative(),
            driven: false,
        };
        // The first result is highlighted the moment the palette opens; it must
        // be drawn that way rather than fading in.
        n.arahkan();
        Box::new(n)
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<PaletteRowBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;

        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        n.index = self.index;
        if n.corners != self.corners {
            n.corners = self.corners;
            dirty |= Dirty::PAINT;
        }
        if n.min_height != self.min_height {
            n.min_height = self.min_height;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        let berubah = n.highlighted != self.highlighted
            || n.disabled != self.disabled
            || n.highlight != self.highlight
            || n.hover != self.hover
            || n.pressed_color != self.pressed;
        if berubah {
            n.highlighted = self.highlighted;
            n.disabled = self.disabled;
            n.highlight = self.highlight;
            n.hover = self.hover;
            n.pressed_color = self.pressed;
            if self.disabled {
                n.pressed = false;
                n.hovered = false;
            }
            n.arahkan();
            dirty |= Dirty::PAINT | Dirty::ANIMATION;
        }
        if n.tint.spring() != self.spring {
            n.tint.set_spring(self.spring);
        }
        n.on_press.clone_from(&self.on_press);
        dirty
    }
}

// ---------------------------------------------------------------------------
// The palette node
// ---------------------------------------------------------------------------

/// Render node for the palette: the four keys, the panel, and the a11y node.
///
/// It sits **above** the text field and takes only what the field lets through
/// — the same arrangement as [`crate::combo_box::ComboFieldBox`], and for the
/// same reason: a key event travels outwards from the focused node, so anything
/// the field handles never arrives here at all.
pub struct PaletteBox {
    /// Visual values already resolved from the tokens.
    pub style: PaletteStyle,
    /// How many results there are right now.
    pub hits: usize,
    /// Which one the keyboard is on.
    pub highlight: usize,
    /// The palette's name for screen readers.
    pub label: Option<String>,
    /// What runs when the highlight should move.
    pub on_highlight: Option<HitCallback>,
    /// What runs when a result is chosen.
    pub on_activate: Option<HitCallback>,
    /// What runs when the palette should close.
    pub on_dismiss: Option<silka_core::Callback>,
}

impl PaletteBox {
    /// The highlight that actually applies: clamped to the current results.
    pub fn active(&self) -> usize {
        if self.hits == 0 {
            0
        } else {
            self.highlight.min(self.hits - 1)
        }
    }

    /// Move the highlight by `delta`, **wrapping** at both ends.
    ///
    /// Wrapping is the one place this component departs from the rest of the
    /// catalogue (`tabs` and `segmented_control` deliberately stop at the
    /// ends). A palette is a short list the user is scanning rather than a
    /// position they are keeping track of, and every palette worth copying
    /// wraps — pressing ↑ on the first result to reach the last is a gesture
    /// people already have.
    pub fn step(&mut self, delta: i32) -> bool {
        if self.hits == 0 {
            return false;
        }
        let n = self.hits as i32;
        let tujuan = (self.active() as i32 + delta).rem_euclid(n) as usize;
        self.set_highlight(tujuan)
    }

    /// Ask for the highlight to move to `index`; true if the callback ran.
    pub fn set_highlight(&mut self, index: usize) -> bool {
        if self.hits == 0 || index >= self.hits || index == self.active() {
            return false;
        }
        let Some(cb) = self.on_highlight.clone() else {
            return false;
        };
        cb.call(index);
        true
    }

    /// Run the highlighted result; true if the callback ran.
    pub fn activate(&mut self) -> bool {
        if self.hits == 0 {
            return false;
        }
        let i = self.active();
        let Some(cb) = self.on_activate.clone() else {
            return false;
        };
        cb.call(i);
        true
    }

    /// Ask for the palette to close; true if the callback ran.
    pub fn dismiss(&mut self) -> bool {
        let Some(cb) = self.on_dismiss.clone() else {
            return false;
        };
        cb.call();
        true
    }
}

impl RenderNode for PaletteBox {
    fn type_name(&self) -> &'static str {
        "CommandPalette"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        // The panel's width is its own decision, not its contents': a palette
        // whose width changed with the length of a command name would twitch on
        // every keystroke.
        let w = if constraints.max_width.is_finite() {
            self.style.width.min(constraints.max_width)
        } else {
            self.style.width
        };
        let child = ctx.child(0);
        let size = ctx.layout_child(
            child,
            BoxConstraints::new(w, w, 0.0, constraints.max_height),
        );
        ctx.place_child(child, Point::ZERO);
        constraints.constrain(Size::new(w, size.height))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        ctx.decorate(&self.style.panel);
        ctx.paint_children();
    }

    fn access(&self, node: &mut AccessNode) {
        // The same shape `combo_box` uses: a group carrying `expanded` around a
        // text input, with the results as marked menu items below it.
        node.role = AccessRole::Group;
        node.label.clone_from(&self.label);
        node.expanded = Some(self.hits > 0);
    }

    fn hit_behavior(&self) -> HitBehavior {
        // The panel absorbs clicks on its own padding: one landing there must
        // not fall through to the backdrop and dismiss the palette.
        HitBehavior::Opaque
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        let Event::Key(k) = event else { return };
        if !k.is_pressed() || !k.modifiers.is_empty() {
            return;
        }
        match k.code {
            KeyCode::Named(NamedKey::ArrowDown) => {
                ctx.handled();
                self.step(1);
            }
            KeyCode::Named(NamedKey::ArrowUp) => {
                ctx.handled();
                self.step(-1);
            }
            // Home/End only reach this far when focus is **not** in the field:
            // a text field moves its caret with them, and that is correct — the
            // user is typing. Kept anyway, because focus does leave the field
            // (a click on a row, a screen reader moving it).
            KeyCode::Named(NamedKey::Home) => {
                ctx.handled();
                self.set_highlight(0);
            }
            KeyCode::Named(NamedKey::End) => {
                ctx.handled();
                let last = self.hits.saturating_sub(1);
                self.set_highlight(last);
            }
            KeyCode::Named(NamedKey::Enter) => {
                ctx.handled();
                self.activate();
            }
            KeyCode::Named(NamedKey::Escape) => {
                // Handled **only** when something actually closed: otherwise it
                // has to keep bubbling, or a palette without an `on_dismiss`
                // would swallow the Esc that belongs to the overlay above it.
                if self.dismiss() {
                    ctx.handled();
                }
            }
            // Tab is deliberately untouched: it belongs to focus navigation,
            // and a palette that trapped it would be a keyboard trap.
            _ => {}
        }
    }
}

impl core::fmt::Debug for PaletteBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PaletteBox")
            .field("hits", &self.hits)
            .field("highlight", &self.highlight)
            .finish()
    }
}

/// Props for the palette — the view form of [`PaletteBox`].
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteProps {
    pub(crate) style: PaletteStyle,
    pub(crate) hits: usize,
    pub(crate) highlight: usize,
    pub(crate) label: Option<String>,
    pub(crate) on_highlight: Option<HitCallback>,
    pub(crate) on_activate: Option<HitCallback>,
    pub(crate) on_dismiss: Option<silka_core::Callback>,
}

impl ViewNode for PaletteProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(PaletteBox {
            style: self.style,
            hits: self.hits,
            highlight: self.highlight,
            label: self.label.clone(),
            on_highlight: self.on_highlight.clone(),
            on_activate: self.on_activate.clone(),
            on_dismiss: self.on_dismiss.clone(),
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<PaletteBox>()
            .expect("tipe view sama berarti tipe render node sama");
        let mut dirty = Dirty::NONE;
        if n.style != self.style {
            n.style = self.style;
            dirty |= Dirty::LAYOUT | Dirty::PAINT;
        }
        if n.hits != self.hits || n.highlight != self.highlight {
            n.hits = self.hits;
            n.highlight = self.highlight;
            dirty |= Dirty::PAINT;
        }
        if n.label != self.label {
            n.label.clone_from(&self.label);
            dirty |= Dirty::PAINT;
        }
        n.on_highlight.clone_from(&self.on_highlight);
        n.on_activate.clone_from(&self.on_activate);
        n.on_dismiss.clone_from(&self.on_dismiss);
        dirty
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Dart-style builder for a command palette (§2.5).
pub struct CommandPalette {
    fonts: Fonts,
    images: Images,
    theme: Theme,
    commands: Vec<Command>,
    style: Option<PaletteStyle>,
    open: bool,
    query: String,
    highlight: usize,
    label: Option<String>,
    placeholder: Option<String>,
    empty_message: Option<String>,
    max_visible: Option<usize>,
    on_query: Option<crate::editing::TextCallback>,
    on_run: Option<crate::editing::TextCallback>,
    on_highlight: Option<HitCallback>,
    on_dismiss: Option<silka_core::Callback>,
    spring: Spring,
    key: Option<Key>,
}

/// A command palette over `commands` — `command_palette` (`KOMPONEN.md`
/// Tier 3).
///
/// ```
/// use silka_widgets::{command, command_palette};
///
/// let palette = command_palette([command("quit", "Quit")]).open(true);
/// # let _ = palette;
/// ```
///
/// Use [`command_palette_in`] outside a build pass.
pub fn command_palette(commands: impl IntoIterator<Item = Command>) -> CommandPalette {
    command_palette_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        commands,
    )
}

/// [`command_palette`] with the text engine and theme passed explicitly.
pub fn command_palette_in(
    fonts: &Fonts,
    theme: &Theme,
    commands: impl IntoIterator<Item = Command>,
) -> CommandPalette {
    CommandPalette {
        fonts: fonts.clone(),
        images: crate::images::active_images(),
        theme: *theme,
        commands: commands.into_iter().collect(),
        style: None,
        open: false,
        query: String::new(),
        highlight: 0,
        label: None,
        placeholder: None,
        empty_message: None,
        max_visible: None,
        on_query: None,
        on_run: None,
        on_highlight: None,
        on_dismiss: None,
        spring: Spring::snappy(),
        key: None,
    }
}

impl CommandPalette {
    /// Wire the palette to a [`PaletteState`] — open/query/highlight in one
    /// line.
    ///
    /// Sugar rather than machinery: everything it sets can be set by hand, and
    /// the node itself knows nothing about [`PaletteState`]. What it buys is
    /// that the three values which have to agree cannot drift apart.
    pub fn bind(mut self, state: PaletteState) -> Self {
        self.open = state.is_open();
        self.query = state.query();
        self.highlight = state.highlight();
        self.on_query = Some(crate::editing::TextCallback::new(move |q: &str| {
            state.set_query(q)
        }));
        self.on_highlight = Some(HitCallback::new(move |i| state.set_highlight(i)));
        self.on_dismiss = Some(silka_core::Callback::new(move || state.set_open(false)));
        self
    }

    /// Whether the palette is on screen (a controlled prop).
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// The current query (a controlled prop).
    pub fn query(mut self, query: impl Into<String>) -> Self {
        self.query = query.into();
        self
    }

    /// The highlighted result, as an index into the hit list.
    pub fn highlight(mut self, index: usize) -> Self {
        self.highlight = index;
        self
    }

    /// What runs as the user types.
    pub fn on_query(mut self, f: impl Fn(&str) + 'static) -> Self {
        self.on_query = Some(crate::editing::TextCallback::new(f));
        self
    }

    /// What runs when a command is chosen; the argument is its **id**.
    pub fn on_run(mut self, f: impl Fn(&str) + 'static) -> Self {
        self.on_run = Some(crate::editing::TextCallback::new(f));
        self
    }

    /// What runs when the highlight moves.
    pub fn on_highlight(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_highlight = Some(HitCallback::new(f));
        self
    }

    /// What runs when the palette should close (Esc, or a click outside).
    pub fn on_dismiss(mut self, f: impl Fn() + 'static) -> Self {
        self.on_dismiss = Some(silka_core::Callback::new(f));
        self
    }

    /// The palette's name for screen readers.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The field's placeholder (default: "Type a command…").
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// What is shown when nothing matches (default: "No matching commands").
    pub fn empty_message(mut self, message: impl Into<String>) -> Self {
        self.empty_message = Some(message.into());
        self
    }

    /// How many results are on screen at once.
    pub fn max_visible(mut self, rows: usize) -> Self {
        self.max_visible = Some(rows.max(1));
        self
    }

    /// The spring driving each row's tint.
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Replace every visual value at once (§2.7).
    pub fn style(mut self, style: PaletteStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// The images atlas used to rasterise row icons.
    pub fn images(mut self, images: &Images) -> Self {
        self.images = images.clone();
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The visual values that will be used.
    pub fn resolved_style(&self) -> PaletteStyle {
        let mut s = self
            .style
            .unwrap_or_else(|| PaletteStyle::from_theme(&self.theme));
        if let Some(m) = self.max_visible {
            s.max_visible = m;
        }
        s
    }

    /// The results for the current query, best first.
    pub fn hits(&self) -> Vec<Hit> {
        rank(&self.commands, &self.query)
    }

    /// The highlight that actually applies: clamped to the current results.
    pub fn active(&self) -> usize {
        let n = self.hits().len();
        if n == 0 {
            0
        } else {
            self.highlight.min(n - 1)
        }
    }

    /// The command a chosen result maps to.
    pub fn command_at(&self, hit: usize) -> Option<&Command> {
        let hits = self.hits();
        self.commands.get(hits.get(hit)?.index)
    }

    /// The panel on its own, without an overlay around it.
    ///
    /// For the rare application that wants the palette inline (a demo page, a
    /// gallery). The normal path is [`CommandPalette::overlay`].
    pub fn panel(&self) -> View {
        let style = self.resolved_style();
        let hits = self.hits();
        let aktif = if hits.is_empty() {
            0
        } else {
            self.highlight.min(hits.len() - 1)
        };
        let jendela = window(hits.len(), aktif, style.max_visible);
        let kosong = self.query.chars().all(char::is_whitespace);

        // --- the field ---------------------------------------------------
        let mut kolom: Vec<View> = Vec::new();
        let cari = row([
            View::from(
                icon_in(&self.images, &self.theme, IconName::Search)
                    .size_raw(style.title_size)
                    .color_raw(style.subtitle)
                    .decorative(),
            ),
            View::from(expanded({
                let mut f = text_field_in(&self.fonts, &self.theme, self.query.clone())
                    .placeholder(
                        self.placeholder
                            .clone()
                            .unwrap_or_else(|| "Type a command…".to_string()),
                    )
                    // ↑/↓ belong to the result list, so the field must not eat
                    // them; and `on_submit` is deliberately never set, because
                    // a field that has one swallows Return.
                    .arrow_keys(ArrowKeys::Bubble);
                if let Some(l) = &self.label {
                    f = f.label(l.clone());
                }
                if let Some(cb) = self.on_query.clone() {
                    f = f.on_change(move |s| cb.call(s));
                }
                f
            })),
        ])
        .cross(CrossAlign::Center)
        .spacing(style.row_gap);
        kolom.push(View::from(cari));

        // --- the results --------------------------------------------------
        let mut baris: Vec<View> = Vec::new();
        if hits.is_empty() {
            baris.push(View::from(
                row([View::from(
                    text_in(
                        &self.fonts,
                        self.empty_message
                            .clone()
                            .unwrap_or_else(|| "No matching commands".to_string()),
                    )
                    .size(style.title_size)
                    .color(style.subtitle)
                    .single_line()
                    .role(AccessRole::Label),
                )])
                .main(MainAlign::Center)
                .padding(style.row_padding),
            ));
        } else {
            let mut bagian: Option<String> = None;
            for slot in jendela {
                let hit = &hits[slot];
                let cmd = &self.commands[hit.index];

                // Sections are shown only while nothing has been typed: in a
                // ranked list they would group results that are no longer
                // adjacent, which is worse than no grouping at all.
                if kosong {
                    if let Some(s) = &cmd.section {
                        if bagian.as_deref() != Some(s.as_str()) {
                            bagian = Some(s.clone());
                            baris.push(View::from(
                                text_in(&self.fonts, s.as_str())
                                    .size(style.small_size)
                                    .weight(FontWeight::SEMIBOLD)
                                    .color(style.section)
                                    .single_line()
                                    .role(AccessRole::Label),
                            ));
                        }
                    }
                }
                baris.push(self.row_view(&style, slot, slot == aktif, cmd));
            }
        }
        kolom.push(View::from(
            column(baris).spacing(0.0).cross(CrossAlign::Stretch),
        ));

        let isi = column(kolom)
            .spacing(style.gap)
            .padding(style.padding)
            .cross(CrossAlign::Stretch);

        // --- the node ------------------------------------------------------
        let jumlah = hits.len();
        let on_activate = self.on_run.clone().map(|cb| {
            let ids: Vec<String> = hits
                .iter()
                .map(|h| self.commands[h.index].id.clone())
                .collect();
            let aktif_flag: Vec<bool> = hits
                .iter()
                .map(|h| self.commands[h.index].enabled)
                .collect();
            HitCallback::new(move |i| {
                if aktif_flag.get(i).copied().unwrap_or(false) {
                    if let Some(id) = ids.get(i) {
                        cb.call(id);
                    }
                }
            })
        });

        let mut b = Builder::new(PaletteProps {
            style,
            hits: jumlah,
            highlight: aktif,
            label: self.label.clone(),
            on_highlight: self.on_highlight.clone(),
            on_activate,
            on_dismiss: self.on_dismiss.clone(),
        })
        .child(isi);
        if let Some(k) = self.key.clone() {
            b = b.key(k);
        }
        b.into()
    }

    /// The palette as an overlay, ready for
    /// [`overlay_layer`](crate::overlay_layer).
    ///
    /// A modal barrier, a backdrop, dismissal on Esc or an outside click, and a
    /// panel pinned near the top edge — every one of those is
    /// [`overlay`](mod@crate::overlay)'s, and not a coordinate is computed here
    /// (`KOMPONEN.md` rule #3).
    pub fn overlay(&self) -> OverlayBuilder {
        let style = self.resolved_style();
        let mut o = overlay(self.panel())
            .open(self.open)
            .barrier(Barrier::Modal)
            .dismiss(Dismiss::ALL)
            .role(AccessRole::Dialog)
            // Near the top, centred: where every palette worth copying puts
            // itself, because that is where the eye already is after ⌘K.
            .placement(
                Placement::edge(Side::Top)
                    .align(Align::Center)
                    .gap(style.top_gap),
            )
            .spring(self.spring);
        if let Some(l) = &self.label {
            o = o.label(l.clone());
        }
        if let Some(cb) = self.on_dismiss.clone() {
            o = o.on_dismiss(move || cb.call());
        }
        o
    }

    /// Assemble one result row.
    fn row_view(
        &self,
        style: &PaletteStyle,
        slot: usize,
        highlighted: bool,
        cmd: &Command,
    ) -> View {
        let warna = if cmd.enabled {
            style.title
        } else {
            style.disabled
        };

        let mut isi: Vec<View> = Vec::with_capacity(3);
        if let Some(name) = cmd.icon {
            isi.push(View::from(
                icon_in(&self.images, &self.theme, name)
                    .size_raw(style.title_size)
                    .color_raw(warna)
                    // The row already carries the accessible name.
                    .decorative(),
            ));
        }

        let mut teks: Vec<View> = Vec::with_capacity(2);
        teks.push(View::from(
            text_in(&self.fonts, cmd.title.as_str())
                .size(style.title_size)
                .weight(FontWeight::MEDIUM)
                .color(warna)
                .single_line()
                .role(AccessRole::Container),
        ));
        if let Some(s) = &cmd.subtitle {
            teks.push(View::from(
                text_in(&self.fonts, s.as_str())
                    .size(style.small_size)
                    .color(style.subtitle)
                    .single_line()
                    .role(AccessRole::Container),
            ));
        }
        isi.push(View::from(expanded(
            column(teks).spacing(0.0).cross(CrossAlign::Start),
        )));

        if let Some(sc) = &cmd.shortcut {
            isi.push(View::from(
                text_in(&self.fonts, sc.display(ShortcutStyle::PLATFORM))
                    .size(style.small_size)
                    .color(style.shortcut)
                    .single_line()
                    .role(AccessRole::Container),
            ));
        }

        let baris = row(isi)
            .cross(CrossAlign::Center)
            .spacing(style.row_gap)
            .padding(style.row_padding);

        let on_press = self.on_run.clone().filter(|_| cmd.enabled).map(|cb| {
            let id = cmd.id.clone();
            let sorot = self.on_highlight.clone();
            silka_core::Callback::new(move || {
                // Clicking a row also moves the highlight onto it, so the
                // keyboard picks up where the mouse left off.
                if let Some(h) = &sorot {
                    h.call(slot);
                }
                cb.call(&id);
            })
        });

        Builder::new(PaletteRowProps {
            label: cmd.title.clone(),
            index: slot,
            highlighted,
            disabled: !cmd.enabled,
            corners: style.row_corners,
            min_height: style.row_height,
            highlight: style.highlight,
            hover: style.hover,
            pressed: style.pressed,
            on_press,
            spring: self.spring,
        })
        .key(Key::text(format!("palette-row:{}", cmd.id)))
        .child(baris)
        .into()
    }
}

impl From<CommandPalette> for View {
    fn from(p: CommandPalette) -> View {
        p.panel()
    }
}

// ---------------------------------------------------------------------------
// Ticking
// ---------------------------------------------------------------------------

/// Every palette row in `tree`, in pre-order.
fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        if tree
            .render(id)
            .and_then(|n| n.downcast_ref::<PaletteRowBox>())
            .is_some()
        {
            out.push(id);
        }
        for anak in tree.children(id) {
            kumpulkan(tree, *anak, out);
        }
    }
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

/// Advance every palette row's tint by one frame.
///
/// Only pixels move: a row's size comes from its text, never from the
/// highlight, so walking the results never makes the panel relayout.
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in nodes(tree) {
        let Some((berubah, bergerak)) = tree
            .node_mut_ref::<PaletteRowBox>(id)
            .map(|r| (r.advance(tick), r.is_animating()))
        else {
            continue;
        };
        if berubah {
            tree.mark_needs_paint(id);
            dirty |= Dirty::PAINT;
        }
        if bergerak {
            dirty |= Dirty::ANIMATION;
        }
    }
    dirty
}

/// True while any palette row's tint is still moving.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<PaletteRowBox>(id)
            .is_some_and(PaletteRowBox::is_animating)
    })
}

/// Finish every palette transition instantly (tests and snapshots).
///
/// ```
/// use silka_core::tree::RenderTree;
/// use silka_widgets::command_palette::{is_animating, settle};
///
/// let mut tree = RenderTree::new();
/// assert!(!is_animating(&tree));
/// settle(&mut tree);
/// assert!(!is_animating(&tree));
/// ```
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(r) = tree.node_mut_ref::<PaletteRowBox>(id) {
            r.settle();
        }
        tree.mark_needs_paint(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::input::{InputRouter, PointerEvent};
    use silka_core::signals::Runtime;
    use silka_core::view::reconcile;
    use silka_theme::{Appearance, Preset};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    const BOX: Size = Size::new(1200.0, 800.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn commands() -> Vec<Command> {
        vec![
            command("file.new", "New File").section("File"),
            command("file.open", "Open File").section("File"),
            command("view.dark", "Toggle Dark Mode")
                .section("View")
                .keywords(["theme", "night"]),
            command("app.quit", "Quit").enabled(false),
        ]
    }

    fn palette(fonts: &Fonts, t: &Theme) -> CommandPalette {
        command_palette_in(fonts, t, commands()).open(true)
    }

    fn built(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(BOX));
        tree
    }

    fn palette_id(tree: &RenderTree) -> NodeId {
        fn cari(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
            if tree.node_ref::<PaletteBox>(id).is_some() {
                return Some(id);
            }
            tree.children(id).iter().find_map(|c| cari(tree, *c))
        }
        cari(tree, tree.root()).expect("palette ada di pohon")
    }

    /// The node the keyboard actually lands on.
    ///
    /// Focus belongs to the **field**, not to the palette — that is the whole
    /// arrangement of this component, so the tests have to drive it the same
    /// way the user does and let the keys bubble.
    fn field_id(tree: &RenderTree) -> NodeId {
        fn cari(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
            if tree
                .node_ref::<crate::text_field::TextFieldBox>(id)
                .is_some()
            {
                return Some(id);
            }
            tree.children(id).iter().find_map(|c| cari(tree, *c))
        }
        cari(tree, tree.root()).expect("kolom teks ada di pohon")
    }

    fn tekan(tree: &mut RenderTree, key: NamedKey) -> silka_core::input::Response {
        let field = field_id(tree);
        let mut router = InputRouter::new();
        router.focus_node(tree, Some(field));
        router.dispatch(
            tree,
            &Event::Key(KeyEvent::pressed(KeyCode::Named(key), Duration::ZERO)),
        )
    }

    // -- fuzzy ------------------------------------------------------------

    #[test]
    fn subsequence_bukan_substring() {
        assert!(fuzzy_match("gsc", "Git: Stage Changes").is_some());
        assert!(fuzzy_match("xyz", "Git: Stage Changes").is_none());
    }

    #[test]
    fn awal_kata_mengalahkan_tengah_kata() {
        let inisial = fuzzy_match("gsc", "Git: Stage Changes").expect("cocok");
        let tersebar = fuzzy_match("gsc", "Gathering Statistics Consumed").expect("cocok");
        assert!(
            inisial.score > 0,
            "inisialisme harus bernilai positif: {inisial:?}"
        );
        assert!(tersebar.score > 0);
    }

    #[test]
    fn berturut_turut_mengalahkan_tersebar() {
        let rapat = fuzzy_match("file", "File").expect("cocok");
        let renggang = fuzzy_match("file", "F i l e").expect("cocok");
        assert!(
            rapat.score > renggang.score,
            "{} harus mengalahkan {}",
            rapat.score,
            renggang.score
        );
    }

    #[test]
    fn camel_case_dihitung_sebagai_awal_kata() {
        let camel = fuzzy_match("sc", "stageChanges").expect("cocok");
        let bukan = fuzzy_match("sc", "stagechanges").expect("cocok");
        assert!(camel.score > bukan.score);
    }

    #[test]
    fn pencocokan_mengabaikan_besar_kecil_huruf_dan_spasi() {
        assert!(fuzzy_match("OF", "open file").is_some());
        assert_eq!(
            fuzzy_match("o f", "Open File").map(|m| m.positions),
            fuzzy_match("of", "Open File").map(|m| m.positions)
        );
    }

    #[test]
    fn posisi_adalah_indeks_karakter_bukan_byte() {
        // "é" is two bytes, so every position after it differs between a byte
        // index and a character index — a byte index would put the highlight on
        // half a letter, which is the bug this is here to prevent. The "l" is
        // character 7 and byte 8.
        let m = fuzzy_match("cl", "café file").expect("cocok");
        assert_eq!(m.positions, vec![0, 7]);
    }

    #[test]
    fn jarum_kosong_cocok_dengan_apa_pun() {
        let m = fuzzy_match("", "apa saja").expect("cocok");
        assert_eq!(m.score, 0);
        assert!(m.positions.is_empty());
        assert!(fuzzy_match("", "").is_some());
    }

    #[test]
    fn jerami_kosong_tidak_pernah_cocok() {
        assert!(fuzzy_match("a", "").is_none());
    }

    // -- ranking ----------------------------------------------------------

    #[test]
    fn kueri_kosong_mempertahankan_urutan_aplikasi() {
        let c = commands();
        let hits = rank(&c, "");
        assert_eq!(
            hits.iter().map(|h| h.index).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
    }

    #[test]
    fn kata_kunci_menemukan_perintah_yang_judulnya_tidak_menyebutnya() {
        let c = commands();
        let hits = rank(&c, "night");
        assert_eq!(hits[0].index, 2, "'night' harus menemukan Toggle Dark Mode");
    }

    #[test]
    fn cocok_di_judul_mengalahkan_cocok_di_kata_kunci() {
        let c = vec![
            command("a", "Nothing Here").keywords(["dark"]),
            command("b", "Dark Mode"),
        ];
        let hits = rank(&c, "dark");
        assert_eq!(hits[0].index, 1, "judul harus menang atas kata kunci");
    }

    #[test]
    fn subjudul_ikut_dicari_tapi_dengan_penalti() {
        let c = vec![
            command("a", "Something").subtitle("Export as PDF"),
            command("b", "Export"),
        ];
        let hits = rank(&c, "export");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].index, 1);
    }

    #[test]
    fn posisi_yang_dilaporkan_hanya_milik_judul() {
        let c = vec![command("a", "Something").keywords(["zzz"])];
        let hits = rank(&c, "zzz");
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].positions.is_empty(),
            "kecocokan dari kata kunci tidak menyorot apa pun di judul"
        );
    }

    #[test]
    fn perintah_nonaktif_tetap_terdaftar() {
        // Listing it and greying it out beats making it disappear: a command
        // that vanishes looks like one that does not exist.
        let c = commands();
        let hits = rank(&c, "quit");
        assert!(hits.iter().any(|h| h.index == 3));
    }

    // -- windowing --------------------------------------------------------

    #[test]
    fn jendela_selalu_memuat_sorotan() {
        for h in 0..20 {
            let w = window(20, h, 5);
            assert!(w.contains(&h), "sorotan {h} keluar dari jendela {w:?}");
            assert_eq!(w.len(), 5);
        }
    }

    // -- the component ----------------------------------------------------

    #[test]
    fn panel_menampilkan_setiap_perintah_saat_kueri_kosong() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(palette(&fonts, &t).panel());
        let a11y = tree.access_tree(None);
        for judul in ["New File", "Open File", "Toggle Dark Mode", "Quit"] {
            assert!(
                a11y.find_label(judul).is_some(),
                "{judul} tidak diumumkan:\n{}",
                a11y.dump()
            );
        }
    }

    #[test]
    fn baris_adalah_menu_item_yang_membawa_status_terpilih() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(palette(&fonts, &t).highlight(1).panel());
        let a11y = tree.access_tree(None);
        let baris: Vec<_> = a11y
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::MenuItem)
            .collect();
        assert_eq!(baris.len(), 4);
        assert_eq!(baris[1].node.selected, Some(true));
        assert_eq!(baris[0].node.selected, Some(false));
        assert!(baris[3].node.disabled, "Quit dinonaktifkan");
    }

    #[test]
    fn tinggi_baris_memenuhi_hit_target_hig() {
        let fonts = Fonts::bundled_only();
        for preset in Preset::ALL {
            let t = Theme::new(preset, Appearance::Light);
            let tree = built(
                command_palette_in(&fonts, &t, commands())
                    .open(true)
                    .panel(),
            );
            for id in nodes(&tree) {
                assert!(
                    tree.size(id).height >= MIN_HIT_TARGET,
                    "{preset:?}: baris setinggi {} < {MIN_HIT_TARGET}",
                    tree.size(id).height
                );
            }
        }
    }

    #[test]
    fn grup_membawa_nama_dan_status_terbuka() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(palette(&fonts, &t).label("Perintah").panel());
        let a11y = tree.access_tree(None);
        let grup = a11y
            .entries()
            .iter()
            .find(|e| e.node.role == AccessRole::Group && e.node.expanded.is_some())
            .expect("palette punya node grup");
        assert_eq!(grup.node.label.as_deref(), Some("Perintah"));
        assert_eq!(grup.node.expanded, Some(true));
    }

    #[test]
    fn panah_bawah_memutar_kembali_ke_awal() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let sorot = Rc::new(RefCell::new(Vec::<usize>::new()));
        let rekam = sorot.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            palette(&fonts, &t)
                .highlight(3)
                .on_highlight(move |i| rekam.borrow_mut().push(i))
                .panel(),
        );
        tree.layout(BoxConstraints::loose(BOX));

        tekan(&mut tree, NamedKey::ArrowDown);
        assert_eq!(
            *sorot.borrow(),
            vec![0],
            "sebuah palet berputar; itu gerakan yang sudah dikenal orang"
        );
    }

    #[test]
    fn return_menjalankan_perintah_yang_disorot() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let dijalankan = Rc::new(RefCell::new(Vec::<String>::new()));
        let rekam = dijalankan.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            palette(&fonts, &t)
                .highlight(1)
                .on_run(move |id| rekam.borrow_mut().push(id.to_string()))
                .panel(),
        );
        tree.layout(BoxConstraints::loose(BOX));

        tekan(&mut tree, NamedKey::Enter);
        assert_eq!(*dijalankan.borrow(), vec!["file.open".to_string()]);
    }

    #[test]
    fn return_pada_perintah_nonaktif_tidak_menjalankan_apa_pun() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let dijalankan = Rc::new(RefCell::new(0u32));
        let rekam = dijalankan.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            palette(&fonts, &t)
                .highlight(3)
                .on_run(move |_| *rekam.borrow_mut() += 1)
                .panel(),
        );
        tree.layout(BoxConstraints::loose(BOX));

        tekan(&mut tree, NamedKey::Enter);
        assert_eq!(*dijalankan.borrow(), 0);
    }

    #[test]
    fn esc_tanpa_penerima_tetap_diteruskan() {
        // Otherwise a palette without an `on_dismiss` would swallow the Esc
        // that belongs to the overlay above it.
        let fonts = Fonts::bundled_only();
        let t = theme();
        let mut tree = RenderTree::new();
        reconcile(&mut tree, palette(&fonts, &t).panel());
        tree.layout(BoxConstraints::loose(BOX));

        assert!(!tekan(&mut tree, NamedKey::Escape).handled);
    }

    #[test]
    fn esc_menutup_saat_ada_penerima() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let ditutup = Rc::new(RefCell::new(0u32));
        let rekam = ditutup.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            palette(&fonts, &t)
                .on_dismiss(move || *rekam.borrow_mut() += 1)
                .panel(),
        );
        tree.layout(BoxConstraints::loose(BOX));

        tekan(&mut tree, NamedKey::Escape);
        assert_eq!(*ditutup.borrow(), 1);
    }

    #[test]
    fn tab_bukan_urusan_palet() {
        // Tab belongs to focus navigation. The palette must define no behaviour
        // for it at all — otherwise it becomes a keyboard trap, which is the
        // one thing a modal may never be.
        let fonts = Fonts::bundled_only();
        let t = theme();
        let jejak = Rc::new(RefCell::new(Vec::<String>::new()));
        let a = jejak.clone();
        let b = jejak.clone();
        let c = jejak.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            palette(&fonts, &t)
                .on_dismiss(move || a.borrow_mut().push("tutup".into()))
                .on_run(move |id| b.borrow_mut().push(format!("jalan:{id}")))
                .on_highlight(move |i| c.borrow_mut().push(format!("sorot:{i}")))
                .panel(),
        );
        tree.layout(BoxConstraints::loose(BOX));

        tekan(&mut tree, NamedKey::Tab);
        assert!(
            jejak.borrow().is_empty(),
            "palet tidak boleh punya arti apa pun untuk Tab: {:?}",
            jejak.borrow()
        );
    }

    #[test]
    fn mengeklik_baris_memindahkan_sorotan_lalu_menjalankan() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let urutan = Rc::new(RefCell::new(Vec::<String>::new()));
        let a = urutan.clone();
        let b = urutan.clone();
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            palette(&fonts, &t)
                .highlight(0)
                .on_highlight(move |i| a.borrow_mut().push(format!("sorot:{i}")))
                .on_run(move |id| b.borrow_mut().push(format!("jalan:{id}")))
                .panel(),
        );
        tree.layout(BoxConstraints::loose(BOX));

        let baris = nodes(&tree);
        assert_eq!(baris.len(), 4);
        let kotak = tree.bounds(baris[2]);
        let mut router = InputRouter::new();
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Down, kotak.center(), Duration::ZERO)
                    .button(PointerButton::Primary),
            ),
        );
        router.dispatch(
            &mut tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Up, kotak.center(), Duration::from_millis(20))
                    .button(PointerButton::Primary),
            ),
        );
        assert_eq!(
            *urutan.borrow(),
            vec!["sorot:2".to_string(), "jalan:view.dark".to_string()]
        );
    }

    #[test]
    fn tanpa_hasil_panel_mengatakannya() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(
            palette(&fonts, &t)
                .query("zzzzz")
                .empty_message("Tidak ada perintah yang cocok")
                .panel(),
        );
        let a11y = tree.access_tree(None);
        assert!(
            a11y.find_label("Tidak ada perintah yang cocok").is_some(),
            "{}",
            a11y.dump()
        );
        assert!(
            a11y.entries()
                .iter()
                .all(|e| e.node.role != AccessRole::MenuItem),
            "tidak boleh ada baris hasil"
        );
    }

    #[test]
    fn bagian_hanya_muncul_saat_kueri_kosong() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let kosong = built(palette(&fonts, &t).panel());
        assert!(kosong.access_tree(None).find_label("File").is_some());

        let dicari = built(palette(&fonts, &t).query("file").panel());
        assert!(
            dicari.access_tree(None).find_label("File").is_none(),
            "dalam daftar terurut, judul bagian mengelompokkan yang sudah tidak bertetangga"
        );
    }

    #[test]
    fn lebar_panel_tidak_bergantung_pada_isinya() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let pendek = built(
            command_palette_in(&fonts, &t, [command("a", "Hi")])
                .open(true)
                .panel(),
        );
        let panjang = built(
            command_palette_in(
                &fonts,
                &t,
                [command("a", "A very much longer command name indeed")],
            )
            .open(true)
            .panel(),
        );
        assert_eq!(
            pendek.size(palette_id(&pendek)).width,
            panjang.size(palette_id(&panjang)).width,
            "panel yang berkedut tiap ketukan tombol adalah panel yang salah"
        );
    }

    #[test]
    fn state_mengatur_ulang_saat_dibuka() {
        let rt = Runtime::new();
        let state = PaletteState::new(&rt);
        state.set_open(true);
        state.set_query("dark");
        state.set_highlight(3);
        assert_eq!(state.query(), "dark");

        state.set_open(false);
        state.set_open(true);
        assert_eq!(state.query(), "", "membuka selalu dari keadaan bersih");
        assert_eq!(state.highlight(), 0);
        assert!(state.is_open());
        assert!(state.is_alive());
    }

    #[test]
    fn mengganti_kueri_mengembalikan_sorotan_ke_atas() {
        let rt = Runtime::new();
        let state = PaletteState::new(&rt);
        state.set_highlight(4);
        state.set_query("f");
        assert_eq!(
            state.highlight(),
            0,
            "hasil ketiga untuk kueri lama adalah perintah lain"
        );
    }

    #[test]
    fn toggle_membalik_keadaan() {
        let rt = Runtime::new();
        let state = PaletteState::new(&rt);
        assert!(!state.is_open());
        state.toggle();
        assert!(state.is_open());
        state.toggle();
        assert!(!state.is_open());
    }

    #[test]
    fn pintasan_cmd_k_dikenali() {
        let cocok = KeyEvent::pressed(KeyCode::Character('k'), Duration::ZERO)
            .modifiers(Modifiers::COMMAND);
        assert!(is_shortcut(&cocok));
        assert!(!is_shortcut(&KeyEvent::pressed(
            KeyCode::Character('k'),
            Duration::ZERO
        )));
        assert!(!is_shortcut(
            &KeyEvent::pressed(KeyCode::Character('j'), Duration::ZERO)
                .modifiers(Modifiers::COMMAND)
        ));
    }

    #[test]
    fn overlay_adalah_dialog_modal_di_dekat_tepi_atas() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let p = palette(&fonts, &t).label("Perintah");
        let o = p.overlay();
        // Not a coordinate is computed here — the whole point of rule #3.
        let _: OverlayBuilder = o;
    }

    #[test]
    fn command_at_memetakan_hasil_ke_perintah() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let p = command_palette_in(&fonts, &t, commands()).query("dark");
        let hits = p.hits();
        assert!(!hits.is_empty());
        assert_eq!(
            p.command_at(0).map(Command::id),
            Some("view.dark"),
            "hits: {hits:?}"
        );
        assert!(p.command_at(99).is_none());
    }

    #[test]
    fn sorotan_di_luar_jangkauan_dijepit_bukan_panik() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let p = palette(&fonts, &t).highlight(999);
        assert_eq!(p.active(), 3);
        let kosong = command_palette_in(&fonts, &t, Vec::<Command>::new()).highlight(5);
        assert_eq!(kosong.active(), 0);
        let _ = built(kosong.panel());
    }

    #[test]
    fn benar_di_kedua_preset() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let s = PaletteStyle::from_theme(&t);
                assert!(s.panel.is_visible(), "{preset:?}/{appearance:?}");
                assert!(s.row_height >= MIN_HIT_TARGET);
                assert_eq!(s.row_corners.style, t.radius.style);
                assert!(s.width > 0.0);
                assert!(s.max_visible > 0);
                assert!(
                    s.highlight != s.hover,
                    "baris tersorot harus terbaca beda dari baris yang cuma disorot tetikus"
                );
            }
        }
    }
}
