//! # silka-widgets
//!
//! The component catalogue (see `KOMPONEN.md`) and at the same time the
//! framework's **public API surface**. This is the contract that has to be
//! frozen early; internals may change at will (REKOMENDASI §4 "Kestabilan").
//!
//! Two BINDING rules for the shape of the API:
//!
//! 1. **Dart style** (§2.5) — constructor functions plus method chaining,
//!    nesting identical to Flutter; optional properties move into the method
//!    chain. An `rsx!`-style DSL macro is rejected as the foundation.
//! 2. **Tailwind-style utility styling as a method chain** (§2.6) — no CSS,
//!    no parser, no cascade. Values always resolve through `silka-theme`
//!    tokens, and interactive utilities (`hover`/`pressed`/`focused`)
//!    transition on a spring instead of jumping.
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::{column, View};
//! use silka_theme::ColorToken;
//! use silka_widgets::{button, text};
//!
//! # let rt = Runtime::new();
//! # let count = rt.signal(0i32);
//! column([
//!     View::from(text(format!("Nilai: {}", count.get())).text_color(ColorToken::Label)),
//!     View::from(button("Tambah").on_press(move || count.set(count.get() + 1))),
//! ])
//! .gap_3();
//! ```
//!
//! Neither the text engine nor the theme appears at the call site: both are
//! **ambient** for the duration of a build pass ([`mod@ambient`]). Every
//! constructor keeps an explicit sibling — [`text_in`], [`button_in`], … — for
//! views built outside one.
//!
//! ## What already exists
//!
//! - [`mod@text`] (Tier 0) — a text leaf that **measures itself** through
//!   `silka-text` and draws glyphs from the atlas; wrapping follows the width
//!   handed down by the box constraints, and its content is the a11y node name.
//! - **The Tier 0/1 primitives** — the smallest components in the catalogue and,
//!   ironically, the last ones written. Their absence was visible: both example
//!   applications had grown their own separator out of a constrained empty box,
//!   and every page had its own `expanded(fixed(0.0, 0.0))` doing duty as a gap.
//!   - [`mod@spacer`] — the flexible gap, plus a fixed one on the spacing scale.
//!   - [`mod@divider`] — a hairline in the `separator` token that finally uses
//!     [`silka_core::access::AccessRole::Separator`], with an inset that
//!     mirrors in an RTL document.
//!   - [`stack`] — the **z-axis** container: children sharing one box.
//!     [`mod@overlay`] is not a substitute — that is a layer above the window,
//!     this is a purely local pile that clips and lays out with everything
//!     around it.
//!   - [`align`] / [`center`] — one box positioned inside a bigger one, with a
//!     reading-relative [`Alignment`] the whole catalogue shares.
//!   - [`aspect_ratio`] — "as wide as you like, but keep me 16:9", the one
//!     constraint neither a flex child nor a fixed box can express.
//!
//!     Those four are **re-exports** from [`silka_core::view`], where they sit
//!     beside `div`/`column`/`row`/`expanded`: they are pure constraint
//!     primitives with no font, image or theme dependency beyond tokens, so
//!     duplicating them here would have been a second implementation of the
//!     same three rules.
//!   - [`mod@image`] — a bitmap with fit modes and a rounded clip, both of which
//!     cost nothing extra: cropping is a source rectangle and the clip is a
//!     shader mask, so a photograph is still one draw call.
//!   - [`mod@icon`] — monochrome symbols rasterised from SVG paths into the same
//!     atlas as everything else, sized and coloured by tokens. Coverage, not
//!     colour: one chevron bitmap serves every text colour, exactly as one
//!     glyph bitmap does.
//!   - [`Images`] — the shared bitmap atlas the last two ride on, the exact
//!     counterpart of [`Fonts`].
//! - [`mod@button`] (Tier 2) — a complete control built on tokens:
//!   primary/secondary/ghost/destructive/link variants, hover/press/focus/
//!   disabled/loading states that **all transition on springs**, a focus ring
//!   that grows, Space/Enter, an AccessKit node, and a hit target ≥ 44pt.
//! - [`mod@checkbox`] (Tier 2) — a **tri-state** checkbox (indeterminate
//!   included): the check mark is genuinely *drawn* by a spring
//!   (a real [`silka_paint::Stroke`] along [`check_path`]), the label is
//!   clickable too and doubles as the a11y
//!   name, Space activates, and the hit target is ≥ 44pt even though the box
//!   is 16pt.
//! - [`mod@radio`] / [`radio_group`] (Tier 2) — one answer out of several, and
//!   the first component to use [`silka_core::access::AccessRole::RadioButton`].
//!   The group is a real node rather than a `column` of circles, because "one
//!   Tab stop, arrows inside it" belongs to the **group**: arrows move *and
//!   change* the selection (WAI-ARIA), skip disabled answers, stop at the ends
//!   instead of wrapping, and the focus ring is owned by the container so it
//!   **glides** from option to option. The dot grows out of the middle rather
//!   than fading in, so the state survives a low-contrast screen.
//! - [`mod@stepper`] (Tier 2) — `[−] value [+]`, and one control rather than two
//!   buttons: a screen reader hears
//!   [`silka_core::access::AccessRole::Stepper`] carrying the **value** with
//!   `increment`/`decrement` as actions, the whole thing is a single Tab stop,
//!   and ↑/↓/←/→/Home/End/Page belong to it as a whole. Both halves are a full
//!   44pt wide, the range and the step are the control's job rather than the
//!   caller's arithmetic, and the glyphs are strokes — no font, no atlas.
//! - [`mod@icon_button`] (Tier 2) — a button whose whole content is a symbol.
//!   It takes the accessible name as a **required argument**, because that is
//!   the one thing an icon-only button cannot borrow from what it draws; the
//!   symbol stays 20pt while the target is 44pt, and the interaction contract
//!   is [`ButtonBox`] itself, not a copy of it.
//! - [`label`] / [`field`] / [`form`] (Tier 2) — the macOS Settings layout:
//!   one label column measured **once for the whole form** through the same
//!   text engine that will draw it, a required marker, and a help line that an
//!   error replaces. Each field is a [`silka_core::access::AccessRole::Group`]
//!   carrying its question, so a screen reader hears the question before the
//!   answer.
//! - [`mod@combo_box`] (Tier 2) — a text field with suggestions under it, and
//!   the one place in the catalogue where two finished components have to
//!   cooperate: the typing, IME and undo stay `text_field`'s, the panel stays
//!   [`menu`](mod@menu)'s (state included — a suggestion list's rules are
//!   [`MenuState::apply`](menu::MenuState::apply)'s rules), and what is new is a
//!   single node that takes the four keys the field lets through. Filtering is
//!   deliberately the application's job.
//! - [`mod@switch`] / [`toggle`] (Tier 2) — an on/off switch you can **drag**, not
//!   merely click: the thumb tracks the finger 1:1, the finger's velocity is
//!   handed to the spring on release (handoff §3.5), the track color crosses
//!   over exactly at the midpoint, Space plus left/right arrows, an AccessKit
//!   node carrying the on/off state, and a hit target ≥ 44pt even though the
//!   track is 32pt/24pt.
//! - [`mod@slider`] / [`range_slider`] (Tier 2) — value sliders: dragging that
//!   sticks to the finger, click-on-track, **snapping to steps**, full
//!   keyboard support (arrows/Home/End/PageUp), a two-thumb range variant, an
//!   AccessKit node with the value plus increment/decrement actions, and a
//!   ≥ 44pt touch band around a track only 4pt thick.
//! - [`tabs`](mod@tabs) (Tier 3) — a row of tabs with three variants
//!   (segmented/underline/enclosed) over **one** engine: an indicator that
//!   glides on a retargetable spring, a single Tab stop for the whole row
//!   (arrows/Home/End inside it, skipping disabled tabs, mirrored in RTL), a
//!   focus ring that glides along, and AccessKit `TabList`/`Tab` nodes
//!   complete with the selected state.
//! - [`segmented_control`](mod@segmented_control) (Tier 3) — the standalone
//!   `NSSegmentedControl`, no longer a `tabs` variant. The split is a contract
//!   rather than a preference: this control picks a **value**, so it is a
//!   [`silka_core::access::AccessRole::Group`] of
//!   [`silka_core::access::AccessRole::RadioButton`]s announced as "2 of 3",
//!   never a tab list that promises navigation it does not perform. It also
//!   does the one thing tabs cannot: the thumb **follows the finger** across
//!   segments on a drag, which is what iOS feels like.
//! - [`toolbar`](mod@toolbar) (Tier 3) — the first user of
//!   [`silka_core::access::AccessRole::Toolbar`], and the component whose
//!   contents genuinely do not fit. Overflow is decided in layout by a **pure**
//!   function over natural widths and priorities ([`toolbar::fit_plan`]), so the
//!   answer never depends on the previous frame's answer — the classic toolbar
//!   flip-flop cannot happen. An item that overflows is not merely undrawn: its
//!   wrapper reports `hidden`, so it leaves the a11y tree, stops being a Tab
//!   stop and stops being clickable, all together.
//! - [`breadcrumb`](mod@breadcrumb) (Tier 3) — where you are and how to get
//!   back. The last crumb is deliberately **not** a link but a `Label` carrying
//!   `selected`, and nothing in the API lets an application get that wrong: the
//!   roles come from position. Two kinds of "too narrow" get two answers —
//!   too many levels collapse the middle into a `…`, too little room takes
//!   width from the oldest ancestors first and from the current page last.
//! - [`split_view`](mod@split_view) (Tier 3) — two panes and a divider that is
//!   a real control: an [`silka_core::access::AccessRole::Separator`] carrying
//!   its percentage, a Tab stop with arrows and Home/End, a grab band ≥ 44pt
//!   around a hairline, and a collapse on a retargetable spring. The proportion
//!   is the application's, which is what makes "remember the pane size" a
//!   one-liner rather than a feature.
//! - [`sidebar`](mod@sidebar) (Tier 3) — the source list, and the first
//!   component built on the **layer** command: the whole panel composites as
//!   one group, which is what makes a translucent sidebar read as a single
//!   sheet of glass. Its blur is offered with its limitation stated out loud
//!   rather than faked. Collapsing animates the width while the content stays
//!   laid out at full width and slides under a clip — laying it out at the
//!   animated width instead is what makes a collapsing sidebar re-wrap its
//!   labels on every frame.
//! - [`command_palette`](mod@command_palette) (Tier 3) — ⌘K. Almost nothing is
//!   drawn from scratch: the typing is [`mod@text_field`]'s and the panel is
//!   [`mod@overlay`]'s. What is new is [`command_palette::fuzzy_match`] — a
//!   scored subsequence matcher, pure and therefore arguable in a unit test
//!   rather than by squinting at a running app — and a single node that takes
//!   the four keys the field lets through (↑/↓/Return/Esc), with focus never
//!   leaving the field, because a palette that stops you typing is not a
//!   palette.
//! - [`mod@select`] (Tier 2) — a macOS pop-up button / shadcn Select: a popup that
//!   **rides the overlay system** (anchored to the trigger, auto-flipping at
//!   the screen edge), full keyboard support on the trigger, which keeps focus
//!   (Space/Enter/arrows/Home/End/Esc), plus native-menu-style **typeahead**,
//!   long lists with a window that follows the highlight, an AccessKit
//!   `Button` carrying a value plus marked `Menu`/`MenuItem` nodes, and a hit
//!   target ≥ 44pt on both the box and every row.
//! - [`menu`](mod@menu) (Tier 3 `context_menu`) — the **in-app** menu, drawn by
//!   us inside the window: a dropdown behind a button or chip, and the very
//!   same menu behind a right-click on a region. Rows carry an optional icon, a
//!   displayed shortcut, a check mark or radio dot, and a disabled state;
//!   separators are announced as separators; submenus nest and open to the side.
//!   Every panel **rides the overlay system** — flipping above the trigger at
//!   the bottom of the screen and to the other side at the right edge, without
//!   this module computing a single coordinate. The keyboard is complete
//!   (↑/↓, Home/End, →/← through submenus mirrored in RTL, Return, typeahead,
//!   and **Esc that closes one level**, not the whole menu) and lives on the
//!   trigger, which keeps focus. Not to be confused with
//!   `silka_platform::menu`, which is the OS's own menubar and tray — the
//!   module docs open with the table that says which one to reach for.
//! - [`scroll_view`](mod@scroll_view) (Tier 1) — a scrolling container with
//!   **macOS-style rubber banding**, a bounce that inherits the velocity of
//!   the OS inertia tail (momentum stays the OS's job, INTEGRASI-NATIVE §3),
//!   overlay scrollbars that widen on hover and fade out on their own when
//!   idle, thumb dragging, full keyboard navigation plus a focus ring,
//!   `scroll_to`/`scroll_into_view`, and an AccessKit `SCROLL` action that
//!   genuinely works.
//! - [`list`](mod@list) (Tier 1) — a **virtualized** list: `item` is called
//!   only for rows that are actually visible, so a hundred thousand rows still
//!   come out as a dozen-odd nodes. It lives **inside**
//!   [`scroll_view`](mod@scroll_view) — momentum, rubber banding, and
//!   scrollbars are not written twice — and adds what genuinely belongs to a
//!   list: sticky headers, a selection whose highlight *glides* on a spring,
//!   ↑/↓/Page/Home/End that move the selection while scrolling its row into
//!   view, and AccessKit `List`/`ListItem` nodes along with their selected
//!   state.
//! - [`table`](mod@table) (Tier 5) — a **virtualized** table that rides the
//!   `list` infrastructure instead of growing a second one
//!   (`KOMPONEN.md` ordering rule #4): its row window is computed by the same
//!   [`ListMetrics`], its scrolling and rubber banding belong to
//!   [`scroll_view`](mod@scroll_view), and the seam between the two is the
//!   same [`list::sync_virtual`]. What it adds is precisely what a list does
//!   not have: per-column sorting, column resize and reorder by dragging in
//!   the header, anchored multiple selection (⇧ extends, ⌘ picks, ⌘A takes
//!   everything) stored as **ranges** so a hundred thousand selected rows are
//!   still a single entry, keyboard navigation between **cells** with a focus
//!   ring around the active cell, custom cells (any widget inside a cell),
//!   sticky headers, an empty state, and AccessKit `Table`/`Row`/`Cell` nodes.
//! - [`tree`](mod@tree) (Tier 5) — a **virtualized** outline view
//!   (`NSOutlineView`) that rides the *same* infrastructure again, so there are
//!   still only two systems and not three: the hierarchy is flattened into rows
//!   once per expansion change, and from there `list`'s [`ListMetrics`] answers
//!   which rows are visible. What it adds is what a flat list cannot have:
//!   opening and closing as a genuine **height animation** (the subtree grows
//!   inside a clipping window while the rows below slide, on a spring), a
//!   chevron that *rotates* rather than swapping glyphs, indentation with
//!   connector guides, single and multiple selection, keyboard navigation where
//!   → opens or steps in and ← closes or steps out, type-to-jump, **children
//!   loaded on open** for trees too large to hold in memory, an empty state, and
//!   AccessKit `Tree`/`TreeItem` nodes carrying level, position in set, size of
//!   set, and expanded state.
//! - [`mod@text_field`] (Tier 2, **the hardest component in the whole catalogue**)
//!   — a single-line text field: caret and selection **per grapheme cluster**
//!   (UAX #29), double-click by word, triple-click for the whole content,
//!   drag-select, undo/redo that coalesces consecutive typing, horizontal
//!   scrolling that keeps the caret visible, and **IME preedit rendered inline
//!   with an underline** — with the normal key path held back during
//!   composition, so the application never receives half-finished letters
//!   (§3.3, §3.8). Its editing model lives in [`silka_text::edit`], its
//!   geometry in [`silka_text::TextLayout`].
//! - [`mod@text_area`] (Tier 2) — the **multi-line** editor. It does not own a
//!   second editing engine, and that is the whole point: the document, the
//!   graphemes, the undo, and the IME are the very same
//!   [`silka_text::TextEdit`] `text_field` uses (in `multiline` mode), the
//!   keymap is the very same [`editing::handle_key`], and the scrolling is
//!   [`scroll_view`](mod@scroll_view) — momentum, rubber banding, and the
//!   auto-hiding scrollbar are not written twice. What it adds is what
//!   genuinely belongs to multiple lines: soft wrapping against the width,
//!   ↑/↓ across **visual** lines with a real **goal column**, Home/End per
//!   visual line with ⌘/Ctrl+Home/End across the document, PageUp/PageDown by
//!   a viewport, selection across lines, Enter as a new line (⌘Enter submits),
//!   a **configurable Tab that moves focus by default** so a text box can
//!   never become a keyboard trap, optional auto-grow up to a maximum height,
//!   an optional line-number gutter, and an AccessKit node with the
//!   **multiline** role that reports its caret and selection.
//! - [`wysiwyg`](mod@wysiwyg) (Tier 6, **the heaviest component in the
//!   catalogue**) — the rich text editor. It is built **on** `text_area`'s
//!   machinery rather than beside it: the frame, the focus ring, auto-grow, the
//!   scroll view and its `SCROLL` action, and the `AreaLink` seam between them
//!   are literally the same nodes. What it adds is the thing that makes rich
//!   text hard — the contents are no longer a string but a **tree of blocks**
//!   (paragraph, three heading levels, bulleted and numbered lists, quotation,
//!   code block) holding **styled inline runs** (bold, italic, underline,
//!   strike, inline code, links). Its selection crosses both block and style
//!   boundaries; its undo works on **document operations** (insert, delete,
//!   restyle, retype) rather than string snapshots, so undoing a deleted bullet
//!   brings the bullet back and not merely its letters, while a run of typing
//!   still collapses into a single ⌘Z. The toolbar reflects what is under the
//!   caret and rides the components that already exist — the block menu is
//!   [`select`](mod@select), the link sheet is [`dialog`](mod@dialog) — and
//!   copy keeps its styling inside the application while degrading to plain
//!   text on the way out.
//! - [`editing`](mod@editing) (infrastructure) — the half of the text keymap
//!   that means the same thing in a one-line field and a multi-line editor,
//!   written **once** and run by both.
//! - [`advance`] (infrastructure) — one tick for the whole tree: this is where
//!   every widget's springs are advanced, once per frame, and where the answer
//!   to "is anything still moving?" comes from.
//! - [`Fonts`] — the shared handle to the application's text engine, one atlas
//!   for the whole application.
//! - [`dialog`](mod@dialog) / [`alert`] (Tier 4) — a backdropped modal on top
//!   of [`overlay`](mod@overlay): a title, a message, and a button row whose
//!   **order follows OS convention** ([`ButtonOrder`]), with Return running
//!   the default button and Esc running the cancel action.
//! - [`overlay`](mod@overlay) (Tier 4, **infrastructure**) — a layer above the
//!   content, anchored placement with auto-flip at the edges, a backdrop,
//!   dismissal (outside click/Esc), and retargetable spring transitions. Built
//!   exactly once, precisely as `KOMPONEN.md` rule #3 demands: dialog, sheet,
//!   popover, tooltip, menu, and toast all **ride** this module — each one
//!   merely picks a [`Placement`] and a [`Barrier`], and not one of them
//!   computes its own position.
//! - **The rest of Tier 4** — the overlay system's promise, collected. Every
//!   one of these is a *preset* on [`overlay`](mod@overlay) plus whatever is
//!   genuinely its own, and the second half of each line is the only reason it
//!   is a component rather than a call site:
//!   - [`tooltip`](mod@tooltip) — the short label under the pointer. It rides
//!     [`Barrier::None`], because a tooltip that catches the mouse swallows the
//!     very motion keeping it alive. What is its own is
//!     [`TooltipTimer`] — hover intent as a **pure** state machine, delays and
//!     grace period and warm window, so "does it wait 500 ms?" is a unit test
//!     rather than a stopwatch. `silka-chart`'s tooltip is now this one with
//!     different contents.
//!   - [`popover`](mod@popover) — the anchored panel, and the component that
//!     finally draws the **arrow** the overlay module deliberately left out.
//!     The arrow reads the side the overlay *ended up* using, so a popover
//!     that flipped at the screen edge points the other way without a line of
//!     placement code here; the fact travels through a sync seam
//!     ([`popover::sync`]), never through layout.
//!   - [`hover_card`](mod@hover_card) — a popover opened by hovering, with
//!     [`Barrier::Panel`] so the pointer may **enter** it. Three defaults and
//!     nothing else: the panel is `popover`'s, the timing is `tooltip`'s.
//!   - [`toast`](mod@toast) — the stack in the corner. One overlay entry
//!     holding a column, because separate entries would sit on top of one
//!     another. Each card counts itself down, **pauses while hovered**, can be
//!     swiped away with the finger's velocity handed to the spring, and
//!     animates out *before* the application is told to drop it.
//!   - [`sheet`](mod@sheet) — the macOS sheet: `Placement::edge`, so its
//!     entrance genuinely comes from off-screen, with the two corners meeting
//!     the window edge left square. Its keyboard is
//!     [`dialog`](mod@dialog)'s — the very same node — because a sheet is
//!     behaviourally a dialog and letting the two drift is how one of them
//!     loses its default button.
//!   - [`drawer`](mod@drawer) — the panel that spans a whole edge. Modal or
//!     not, and honest about it: a non-modal drawer announces itself as a
//!     group rather than a dialog, so no screen reader promises a focus trap
//!     that is not there.
//!   - [`progress_bar`] / [`progress_circle`] — the first users of
//!     [`silka_core::access::AccessRole::ProgressIndicator`]. Determinate and
//!     indeterminate are **one** node, so a download that learns its length
//!     mid-transfer keeps the value it had animated to; the endless sweep is
//!     decorative and therefore stops under reduced motion.
//!   - [`skeleton`](mod@skeleton) — content-shaped placeholders. Hidden from
//!     assistive technology by default, because a screen reader meeting eight
//!     empty boxes learns nothing; the shimmer is quads rather than a gradient
//!     the paint layer does not have.
//!   - [`badge`](mod@badge) — the status pill both example applications had
//!     already written by hand. What the hand-rolled version lacked: a floor on
//!     the width (so a one-character badge is a circle), a tone vocabulary
//!     instead of a colour pair, an optional dot as a second channel for a
//!     reader who cannot separate the hues, and a name for assistive
//!     technology.
//! - **Tier 5 — data display.** The tier the two example applications had
//!   already written by hand, which is why every entry below opens with what it
//!   replaced rather than with what it draws:
//!   - [`card`](mod@card) — the surface panel. The ERP dashboard had grown two
//!     of these plus a header and a shortcut tile, each repeating the same four
//!     lines and each inventing its own idea of how much padding a card has.
//!     What the recipe never had: a vocabulary for *where the panel sits*
//!     ([`CardVariant`], so a card inside a card stops doubling its shadow), a
//!     name a screen reader can jump **between** rather than through, and a
//!     pressable card that is a real control without being a second component.
//!   - [`accordion`](mod@accordion) / [`collapsible`] — content that folds. The
//!     hard part is not the chevron, it is the **height**: the content has to
//!     stay laid out at its natural size (or a paragraph re-wraps on every
//!     frame and the text boils), the box around it has to be only as tall as
//!     the spring has got to (or nothing below it moves), and it has to clip
//!     (or the part with no room yet paints over its neighbours).
//!     [`DisclosureBox`] is those three lines. A closed panel is **gone** —
//!     out of the a11y tree and out of the Tab order — because a button
//!     inside a folded section that can still be tabbed to is where a focus
//!     ring goes to disappear.
//!   - [`tag`](mod@tag) — the removable, selectable pill. It shares
//!     [`badge`](mod@badge)'s tone vocabulary and nothing else: a badge *says*
//!     something and a tag *does* something, so this one is a `Button`
//!     carrying `toggled`, a Tab stop, and a 44pt hit area with the pill drawn
//!     smaller inside it. Its cross is a **second** focusable node with its
//!     own name ("Remove Urgent", never "×").
//!   - [`calendar`](mod@calendar) — a month as a grid, and the densest piece of
//!     localisation in the catalogue. Which day the week starts on, what the
//!     days and months are called, and how a date is *spoken* all come out of
//!     [`silka_core::locale::Locale`] — the headings through
//!     [`Locale::weekday_columns`](silka_core::locale::Locale::weekday_columns)
//!     and the cells through
//!     [`Date::column_from`](silka_core::date::Date::column_from), so the two
//!     cannot end up one column apart. One Tab stop, arrows inside it, and a
//!     focus ring owned by the grid so it glides.
//!   - [`date_picker`](mod@date_picker) — that grid under a field. Almost
//!     nothing here is new: what it adds is a field that **reads back what it
//!     shows** ([`Locale::parse_numeric`](silka_core::locale::Locale::parse_numeric)
//!     refuses `3/8` rather than guessing which of two real dates it is), a
//!     control whose a11y **value** is the spoken date rather than its name,
//!     and a way to empty it again.
//!   - [`color_picker`](mod@color_picker) — a palette grid rather than a colour
//!     wheel, for two reasons: the paint layer has no gradient command, and an
//!     application built on a design system wants one of *its* colours rather
//!     than one of sixteen million. [`spectrum`] covers the free-form case by
//!     generating swatches, and a translucent one gets a checkerboard so
//!     "see-through" is visible rather than inferred.
//!   - [`avatar`](mod@avatar) / [`avatar_group`] — a person as a disc. The
//!     fallback is the main case ([`initials`] is pure and has an answer for
//!     one word, three words, an empty string, and a script with no capital
//!     letters), the name is required rather than optional, and a group draws
//!     back to front so the **leading** avatar is on top and every ring is
//!     visible.
//!
//!
//! Technical debt we are aware of and deliberately do not hide:
//!
//! - For overlays, what is missing is **automatic focus** on a freshly opened
//!   panel: [`overlay::topmost`] provides the node, but there is no "just
//!   opened" hook in the frame cycle that calls it.
//! - [`AccessNode`](silka_core::access::AccessNode) has no **live region**
//!   concept, so a [`toast`](mod@toast) is announced when a screen reader
//!   reaches it rather than the moment it appears. Fixing it means an
//!   `AccessLive` field in `silka-core`, and inventing one from this crate
//!   would put it in the wrong place.
//! - A [`tooltip`](mod@tooltip)'s [`TooltipTimer`] is fed by the application,
//!   the same way every other component's `open` is. There is no "the pointer
//!   entered widget X" hook in the frame cycle either — the same missing hook
//!   as automatic focus, seen from the other side.
//!
//! The order of work follows the tiers in `KOMPONEN.md`: Tier 0 (primitives)
//! and Tier 1 (layout) until they are genuinely solid first, `text_field`
//! started earliest in Tier 2 because it forces the text/IME/a11y stack to
//! mature, and the overlay system built once for
//! dialog/popover/tooltip/menu/toast.
//!
//! **Definition of Done for every component** (KOMPONEN.md): correct in both
//! presets, every interactive state transitions on a spring, full keyboard
//! navigation plus a focus ring, an **AccessKit node** (role/name/actions),
//! dark mode, a minimum 44pt hit target, and respect for reduced-motion.
//!
//! Code in this crate **must not touch wgpu types** — only `silka-paint`
//! drawing commands (§3.2, §5 failure mode #7).

#![warn(missing_docs)]
// Documentation is part of the public contract, so the checks rustdoc offers
// are turned on here rather than left to a reviewer's eye. A broken intra-doc
// link is an error: it means a rename silently orphaned a reference.
#![deny(rustdoc::broken_intra_doc_links)]
#![warn(
    rustdoc::private_intra_doc_links,
    rustdoc::invalid_codeblock_attributes,
    rustdoc::invalid_html_tags,
    rustdoc::bare_urls,
    rustdoc::unescaped_backticks
)]

pub mod accordion;
pub mod ambient;
pub mod avatar;
pub mod badge;
pub mod breadcrumb;
pub mod button;
pub mod calendar;
pub mod card;
pub mod checkbox;
pub mod color_picker;
pub mod combo_box;
pub mod command_palette;
pub mod date_picker;
pub mod dialog;
pub mod divider;
pub mod drawer;
pub mod editing;
pub mod fonts;
pub mod form;
pub mod hover_card;
pub mod icon;
pub mod icon_button;
pub mod image;
pub mod images;
pub mod list;
pub mod menu;
pub mod motion;
pub mod overlay;
pub mod popover;
pub mod progress;
pub mod radio;
pub mod scroll_view;
pub mod segmented_control;
pub mod select;
pub mod sheet;
pub mod sidebar;
pub mod skeleton;
pub mod slider;
pub mod spacer;
pub mod split_view;
pub mod stepper;
pub mod switch;
pub mod table;
pub mod tabs;
pub mod tag;
pub mod text;
pub mod text_area;
pub mod text_field;
pub mod toast;
pub mod toolbar;
pub mod tooltip;
pub mod tree;
pub mod wysiwyg;

pub use ambient::{
    active_fonts, active_theme, fonts_installed, install_fonts, uninstall_fonts, with_ambient,
    with_fonts,
};
pub use divider::{divider, divider_in, Divider, DividerBox, DividerProps};
pub use icon::{
    chevron_back, chevron_back_in, chevron_forward, chevron_forward_in, icon, icon_in, icon_path,
    icon_path_in, Icon, IconBox, IconName, IconProps, ICON_VIEWPORT,
};
pub use image::{image, image_in, Image, ImageBox, ImageFit, ImageProps};
pub use images::{
    active_images, images_installed, install_images, uninstall_images, with_images, Images,
};
pub use spacer::{spacer, spacer_flex, spacer_of, spacer_of_in};

pub use badge::{
    badge, badge_count, badge_count_in, badge_in, format_count, Badge, BadgeBox, BadgeColors,
    BadgeProps, BadgeStyle, BadgeTone, BadgeVariant,
};
pub use breadcrumb::{
    breadcrumb, breadcrumb_in, crumb, Breadcrumb, BreadcrumbBox, BreadcrumbProps, BreadcrumbStyle,
    Crumb, CrumbBox, CrumbCallback, CrumbKind, CrumbProps, CrumbSeparator,
};
pub use button::{
    button, button_in, button_variant, button_variant_in, Button, ButtonBox, ButtonProps,
    ButtonState, ButtonStyle, ButtonVariant, MIN_HIT_TARGET,
};
pub use checkbox::{
    check_path, checkbox, checkbox_in, checkbox_only, checkbox_only_in, dash_rect, ChangeCallback,
    CheckState, Checkbox, CheckboxNode, CheckboxProps, CheckboxStyle,
};
pub use combo_box::{
    combo_box, combo_box_in, ComboBox, ComboFieldBox, ComboFieldProps, PickCallback,
};
pub use command_palette::{
    command, command_palette, command_palette_in, use_palette_state, Command, CommandPalette,
    FuzzyMatch, Hit, HitCallback, PaletteBox, PaletteProps, PaletteRowBox, PaletteRowProps,
    PaletteState, PaletteStyle,
};
pub use dialog::{
    action, activate_default, alert, alert_in, dialog, dialog_in, ActionKind, ButtonOrder,
    DialogAction, DialogBuilder, DialogPanel, DialogPanelProps, DIALOG_WIDTH_STEPS,
};
pub use drawer::{
    drawer, drawer_in, edge_corners, inner_edge, Drawer, DrawerPanel, DrawerPanelProps, DrawerStyle,
};
pub use editing::{EditCaps, TextCallback};
pub use fonts::Fonts;
pub use form::{field, form, form_in, label, label_in, Form, FormField, FormLabel, FormStyle};
pub use hover_card::{hover_card, hover_card_in, hover_card_timer, HoverCard, HOVER_CARD_DELAY};
pub use icon_button::{
    icon_button, icon_button_in, icon_button_with, icon_button_with_in, IconButton,
    ICON_BUTTON_SIDE,
};
pub use list::{
    list, list_in, use_list_state, ListBody, ListBuilder, ListMetrics, ListRange, ListRowBox,
    ListScroll, ListState, ListStyle, RowAction, Virtualized,
};
pub use menu::{
    menu, menu_in, Menu, MenuEntry, MenuHandler, MenuIntent, MenuItem, MenuMark, MenuState,
};
pub use motion::{advance, is_animating, settle};
pub use overlay::{
    overlay, overlay_layer, Align, Anchor, Barrier, Dismiss, PhysicalSide, Placement, Side,
};
pub use popover::{popover, popover_in, Popover, PopoverPanel, PopoverPanelProps, PopoverStyle};
pub use progress::{
    progress_bar, progress_bar_in, progress_circle, progress_circle_in, ProgressBar,
    ProgressBarBox, ProgressBarProps, ProgressCircle, ProgressCircleBox, ProgressCircleProps,
    ProgressStyle,
};
pub use radio::{
    radio, radio_group, radio_group_in, radio_in, radio_item, radio_only, radio_only_in, Radio,
    RadioGroup, RadioGroupBox, RadioGroupProps, RadioItem, RadioNode, RadioProps, RadioStyle,
};
pub use scroll_view::{
    scroll_view, scroll_view_in, ScrollBar, ScrollBuilder, ScrollProps, ScrollView, Scrollbar,
    ScrollbarStyle, Thumb,
};
pub use segmented_control::{
    segment, segmented_control, segmented_control_in, OnPick, Segment, SegmentBox, SegmentProps,
    SegmentedBox, SegmentedControl, SegmentedProps, SegmentedStyle,
};
pub use select::{
    select, select_in, Select, SelectHandler, SelectIntent, SelectOption, SelectOptionProps,
    SelectOptionStyle, SelectState, SelectTrigger, SelectTriggerProps, SelectTriggerStyle,
};
pub use sheet::{sheet, sheet_in, Sheet, SHEET_WIDTH_STEPS};
pub use sidebar::{
    sidebar, sidebar_in, sidebar_item, sidebar_item_in, sidebar_section, sidebar_section_in,
    Sidebar, SidebarBox, SidebarItem, SidebarMaterial, SidebarProps, SidebarRowBox,
    SidebarRowProps, SidebarStyle, SIDEBAR_WIDTH,
};
/// The pure-layout Tier 1 primitives, re-exported so the catalogue is complete
/// in one import.
///
/// They live in [`silka_core::view`] beside `div`/`column`/`row`/`expanded`,
/// because that is what they are: constraint primitives that know nothing about
/// fonts, images or a theme beyond its tokens. Nothing here is a second
/// implementation — these are the very same items.
pub use silka_core::tree::{AlignBox, Alignment, AspectRatioBox, StackBox, StackFit};
pub use silka_core::view::{
    align, aspect_ratio, center, stack, AlignProps, AspectRatioProps, StackProps, ASPECT_16_9,
    ASPECT_3_2, ASPECT_4_3, ASPECT_SQUARE,
};
pub use skeleton::{
    skeleton, skeleton_circle, skeleton_circle_in, skeleton_in, skeleton_text, skeleton_text_in,
    Skeleton, SkeletonBox, SkeletonProps, SkeletonStyle,
};
pub use slider::{
    range_slider, range_slider_in, slider, slider_in, Slider, SliderBuilder, SliderGeometry,
    SliderProps, SliderStyle,
};
pub use split_view::{
    split_view, split_view_in, ResizeCallback, SplitHandleBox, SplitHandleProps, SplitSide,
    SplitStyle, SplitView, SplitViewBox, SplitViewProps,
};
pub use stepper::{
    stepper, stepper_in, StepCallback, Stepper, StepperHalf, StepperNode, StepperProps,
    StepperStyle,
};
pub use switch::{
    switch, switch_in, switch_only, switch_only_in, toggle, toggle_in, StateColors, Switch,
    SwitchCallback, SwitchNode, SwitchProps, SwitchStyle,
};
pub use table::{
    col, table, table_in, use_table_state, CellAlign, Column, ColumnLayout, ColumnWidth,
    HeaderStyle, Selection, SelectionMode, SortBy, SortDirection, TableBody, TableBuilder,
    TableCellBox, TableHeaderBox, TableRowBox, TableState, TableStyle,
};
pub use tabs::{tab, tabs, tabs_in, Tab, Tabs, TabsStyle, TabsVariant};
pub use text::{text, text_in, Text, TextBox, TextProps};
pub use text_area::{
    text_area, text_area_in, AreaLink, BodyColors, FrameStyle, TabBehavior, TextArea, TextAreaBody,
    TextAreaBodyProps, TextAreaFrame, TextAreaFrameProps,
};
pub use text_field::{
    text_field, text_field_in, ArrowKeys, TextField, TextFieldBox, TextFieldProps,
};
pub use toast::{
    toast, toasts, toasts_in, use_toast_state, Toast, ToastAction, ToastBox, ToastCallback,
    ToastProps, ToastState, ToastStyle, ToastTone, Toaster,
};
pub use toolbar::{
    tool, tool_space, toolbar, toolbar_in, use_toolbar_state, Toolbar, ToolbarBox, ToolbarFit,
    ToolbarItem, ToolbarItemBox, ToolbarMeta, ToolbarOverflowBox, ToolbarState, ToolbarStyle,
};
pub use tooltip::{tooltip, tooltip_in, Tooltip, TooltipDelay, TooltipPhase, TooltipTimer};
pub use tree::{
    tree, tree_in, use_tree_state, Expansion, TreeAction, TreeBody, TreeBuilder, TreeFlat, TreeGap,
    TreeGapBox, TreeKey, TreeMetrics, TreeNode, TreeRow, TreeRowBox, TreeSource, TreeState,
    TreeStyle, TreeWindow,
};
pub use wysiwyg::{
    wysiwyg, wysiwyg_in, Block, BlockKind, DocPos, DocRange, DocSelection, Document, EditorCommand,
    EditorHandle, EditorSnapshot, EditorStyle, InlineStyle, Marks, RichEdit, Wysiwyg, WysiwygBody,
    WysiwygBodyProps,
};

pub use accordion::{
    accordion, accordion_in, collapsible, collapsible_in, toggled_set, Accordion, AccordionMode,
    Collapsible, CollapsibleHeaderBox, CollapsibleHeaderProps, CollapsibleStyle, DisclosureBox,
    DisclosureProps, ToggleCallback, HEADER_BAND_STEPS, HEADER_INSET_STEPS,
};
pub use avatar::{
    avatar, avatar_group, avatar_group_in, avatar_in, avatar_slot, group_plan, initials, Avatar,
    AvatarBox, AvatarGroup, AvatarGroupBox, AvatarGroupProps, AvatarProps, AvatarStyle,
    AVATAR_STEPS, GROUP_OVERLAP, INITIALS_RATIO,
};
pub use calendar::{
    calendar, calendar_in, clamp_date, month_grid, weeks_in_month, Calendar, CalendarBox,
    CalendarDayBox, CalendarDayProps, CalendarProps, CalendarStyle, DateCallback, CELL_STEPS,
    FIXED_WEEKS,
};
pub use card::{
    card, card_body, card_body_in, card_footer, card_footer_in, card_header, card_header_in,
    card_in, card_padded, card_padded_in, Card, CardBox, CardHeader, CardProps, CardStyle,
    CardSurface, CardVariant, CARD_BAND_STEPS, CARD_INSET_STEPS,
};
pub use color_picker::{
    color_picker, color_picker_in, grid_shape, hex_string, hsv, parse_hex, spectrum, ColorCallback,
    ColorGridBox, ColorGridProps, ColorPicker, ColorPickerStyle, ColorSwatchBox, ColorSwatchProps,
    DEFAULT_COLUMNS, SWATCH_STEPS,
};
pub use date_picker::{
    date_picker, date_picker_in, DateFieldBox, DateFieldProps, DateFieldStyle, DateHandler,
    DateIntent, DatePicker, DatePickerState,
};
pub use tag::{
    cross_path, tag, tag_in, Tag, TagBox, TagProps, TagRemoveBox, TagRemoveProps, TagStyle,
    TAG_HEIGHT_STEPS,
};

/// Compiles and runs every Rust example in this crate's `README.md`.
///
/// The item only exists while rustdoc is collecting doctests, so it never
/// shows up in the rendered documentation. Its whole purpose is to stop the
/// README from drifting away from the API it advertises.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
