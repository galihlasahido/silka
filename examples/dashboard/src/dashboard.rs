//! The flagship screen: the digital-lending dashboard.
//!
//! Reading down the page: heading, a wrapping KPI grid, two list cards side by
//! side, the disbursement trend chart, and the quick links. The structure is
//! the reference screenshot's; none of the pixels are, because everything here
//! is a token and therefore comes out right in Cupertino and Tailwind, light
//! and dark.
//!
//! What is **absent** from this file is the point of it: no hand-assembled
//! `Scene`, no layout arithmetic, no hex colour, and no manual number or date
//! formatting.

use silka_chart::format::NumberFormat;
use silka_chart::tooltip::ChartHover;
use silka_chart::{area_chart, ChartPalette};
use silka_core::app::BuildCtx;
use silka_core::signals::Signal;
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, expanded, flexible, row, View};
use silka_theme::Theme;
use silka_widgets::TreeState;

use crate::data::{self, Tint, TrendPoint};
use crate::kit;
use crate::nav::{self, Page};

/// The a11y name of the trend chart — and the anchor the tests look for.
pub const CHART_NAME: &str = "Daily disbursement";
/// The heading of the left card.
pub const AKAD_CARD: &str = "Akad Scheduled";
/// The heading of the right card.
pub const DISBURSEMENT_CARD: &str = "Recent Disbursements";
/// The heading of the shortcuts card.
pub const QUICK_CARD: &str = "Quick Links";
/// The link in every card header.
pub const VIEW_ALL: &str = "View all →";

/// The narrowest a KPI tile is allowed to get before the grid wraps, in
/// spacing steps.
const KPI_MIN_STEPS: f32 = 42.0;
/// The chart's height, in spacing steps.
const CHART_STEPS: f32 = 60.0;

/// The whole page.
pub fn page(cx: &BuildCtx, page_signal: Signal<Page>) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    let palette = ChartPalette::for_theme(&t);

    // **One** hover signal for the whole application, kept in `Env`: the
    // tooltip is a single entry in the shell's overlay layer, and a panel that
    // has to be a sibling of the content cannot be owned by the page that
    // triggers it.
    let hover: Signal<Option<ChartHover>> = cx.expect_env();
    // Navigating from a card link has to move the sidebar's selection too, or
    // the next build would read the old selection back and undo it.
    let nav_state: TreeState = cx.expect_env();

    column([
        heading(&t),
        kpi_grid(&t, &palette),
        list_cards(&t, nav_state, page_signal),
        trend_chart(&t, hover),
        quick_links(&t, &palette, nav_state, page_signal),
    ])
    .spacing(t.space(6.0))
    .cross(CrossAlign::Stretch)
    .p_8()
    .into()
}

fn heading(t: &Theme) -> View {
    column([
        kit::page_title(t, Page::Dashboard.title()),
        kit::subtitle(t, Page::Dashboard.subtitle()),
    ])
    .spacing(t.space(1.5))
    .cross(CrossAlign::Start)
    .into()
}

/// The ten statistics.
///
/// `wrap()` is what makes the grid behave when the window narrows: the tiles
/// keep their minimum width and fall onto a third row instead of being squeezed
/// into columns too narrow to read.
fn kpi_grid(t: &Theme, palette: &ChartPalette) -> View {
    let tiles: Vec<View> = data::KPIS
        .iter()
        .map(|k| {
            let slot = match k.tint {
                Tint::Plain => None,
                Tint::Slot(i) => Some(i),
            };
            View::from(
                flexible(kit::kpi_tile(t, palette, k.label, &k.value.text(), slot))
                    .grow(1.0)
                    .basis(t.space(KPI_MIN_STEPS)),
            )
        })
        .collect();

    row(tiles)
        .wrap()
        .gap(t.space(4.0), t.space(4.0))
        .cross(CrossAlign::Stretch)
        .into()
}

/// The two list cards, side by side and equally wide.
fn list_cards(t: &Theme, nav_state: TreeState, page_signal: Signal<Page>) -> View {
    row([
        View::from(expanded(akad_card(t, nav_state, page_signal))),
        View::from(expanded(disbursement_card(t, nav_state, page_signal))),
    ])
    .spacing(t.space(5.0))
    .cross(CrossAlign::Stretch)
    .into()
}

fn akad_card(t: &Theme, nav_state: TreeState, page_signal: Signal<Page>) -> View {
    // No `divider` after the header any more: the hairline belongs to
    // `card_header` itself, which is what stops a header and the rows under it
    // from ending up two points apart.
    let mut children = vec![kit::card_header(t, AKAD_CARD, VIEW_ALL, move || {
        nav::go_to(nav_state, page_signal, Page::Contracts)
    })];
    for a in data::AKAD {
        children.push(kit::list_row(
            t,
            a.name,
            &format!("NIK {}", a.nik),
            kit::trailing_text(t, &data::date(data::day(a.day_offset))),
        ));
    }
    kit::card(t, Some(AKAD_CARD), children)
}

fn disbursement_card(t: &Theme, nav_state: TreeState, page_signal: Signal<Page>) -> View {
    let mut children = vec![kit::card_header(
        t,
        DISBURSEMENT_CARD,
        VIEW_ALL,
        move || nav::go_to(nav_state, page_signal, Page::Transactions),
    )];
    for d in data::DISBURSEMENTS {
        children.push(kit::list_row(
            t,
            d.name,
            &data::rupiah(d.amount),
            kit::badge(t, d.status),
        ));
    }
    kit::card(t, Some(DISBURSEMENT_CARD), children)
}

/// The disbursement trend — the chart that proves `silka-chart` survives being
/// used by an application rather than by its own demo page.
fn trend_chart(t: &Theme, hover: Signal<Option<ChartHover>>) -> View {
    let data = data::trend();
    // No landmark name: the chart inside is already an `AccessRole::Image`
    // carrying `CHART_NAME`, and a group of the same name around it would be
    // announced twice.
    kit::padded_card(
        t,
        None,
        [constrained(
            BoxConstraints::new(
                0.0,
                f32::INFINITY,
                t.space(CHART_STEPS),
                t.space(CHART_STEPS),
            ),
            area_chart(data)
                .key("trend")
                .x(|d: &TrendPoint| d.date)
                .y_named("Disbursed", |d: &TrendPoint| d.disbursed)
                .time()
                .title(CHART_NAME)
                .animated(true)
                .locale(data::LOCALE)
                // The axis speaks the same locale as the cards: `120 jt`, not
                // `120M`. One product, one way of writing a number.
                .value_format(NumberFormat::Compact)
                .empty("No disbursement in this period")
                .on_hover(move |h| hover.set(h)),
        )
        .into()],
    )
}

/// The coloured shortcut tiles.
fn quick_links(
    t: &Theme,
    palette: &ChartPalette,
    nav_state: TreeState,
    page_signal: Signal<Page>,
) -> View {
    let tiles: Vec<View> = data::QUICK_LINKS
        .iter()
        .map(|q| {
            let target = match q.label {
                "Daily recap" => Page::DailyRecap,
                "Release funds" => Page::Disbursement,
                "Schedule akad" => Page::Contracts,
                _ => Page::Contracts,
            };
            View::from(
                flexible(kit::action_tile(
                    t,
                    palette,
                    q.label,
                    q.detail,
                    q.slot,
                    move || nav::go_to(nav_state, page_signal, target),
                ))
                .grow(1.0)
                .basis(t.space(KPI_MIN_STEPS)),
            )
        })
        .collect();

    kit::padded_card(
        t,
        Some(QUICK_CARD),
        [
            View::from(
                row([View::from(
                    silka_widgets::text(QUICK_CARD)
                        .size(t.typography.headline.size)
                        .weight(silka_text::FontWeight::SEMIBOLD)
                        .tracking(t.typography.headline.tracking)
                        .color(t.color.label)
                        .single_line(),
                )])
                .cross(CrossAlign::Center),
            ),
            row(tiles)
                .wrap()
                .gap(t.space(3.0), t.space(3.0))
                .main(MainAlign::Start)
                .cross(CrossAlign::Stretch)
                .into(),
        ],
    )
}
