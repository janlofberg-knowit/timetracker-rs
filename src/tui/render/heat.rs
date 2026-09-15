//! What every heat surface draws alike: the tick axis over a row of cells, the
//! `Less … More` ramps, and the content pane's own one-row heatmap.

use crate::tui::summary::{BucketGrid, Grain, strip_cells};
use crate::tui::types::ViewMode;
use crate::tui::{App, theme};
use chrono::{Datelike, Duration, Local, Timelike};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

/// Today's cell marker, as the year grid draws it too.
pub(super) const TODAY_MARKER: &str = "\u{25cf}";

/// What a heat block calls itself: its total and how many `unit`s made it.
pub(super) fn heat_block_title(total: Duration, active: usize, unit: &str) -> String {
    format!(
        " {} tracked over {} active {}{} ",
        crate::duration::format(total),
        active,
        unit,
        if active == 1 { "" } else { "s" }
    )
}

/// The open view's period as one row of heat cells filling the content area:
/// the buckets stretch to the width `strip_cells` gives them and to every inner
/// row the tick axis leaves. A bounded period never hides a bucket of its own
/// — too narrow to fit, it wraps onto further bands — while `All`, which has no
/// bounded period, sheds its oldest buckets as every other strip does.
pub(super) fn render_heat_row(f: &mut Frame, app: &App, area: Rect) {
    let grid = app.view_heat_buckets();
    let row = &grid.rows[0];
    let buckets = grid.len();

    let (cell_width, cells) = strip_cells(area.width.saturating_sub(2) as usize, buckets);
    let unbounded = app.view_mode == ViewMode::All;
    let first = if unbounded { buckets - cells } else { 0 };
    let drawn = &row[first..];
    let total = drawn.iter().fold(Duration::zero(), |acc, cell| acc + *cell);
    let active = drawn.iter().filter(|cell| cell.num_seconds() > 0).count();
    // One maximum over every cell drawn, so the shades stay comparable.
    let peak = drawn
        .iter()
        .map(|cell| cell.num_seconds())
        .max()
        .unwrap_or(0);

    let ramp = match grid.grain {
        Grain::Day => Some(day_heat_legend()),
        _ => relative_heat_legend(0, area.width.saturating_sub(2)),
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::border()))
        .title(Span::styled(
            heat_block_title(total, active, grid.grain.unit()),
            Style::default().fg(theme::title()),
        ));
    if let Some(ramp) = ramp {
        block = block.title_bottom(ramp.right_aligned());
    }
    let inner = block.inner(area);

    if cells == 0 {
        f.render_widget(Paragraph::new(Vec::<Line>::new()).block(block), area);
        return;
    }

    let bands = if unbounded {
        1
    } else {
        buckets.div_ceil(cells)
    };
    // Each band is its own axis over a band of cell rows; two lines is the
    // least a band can be drawn as.
    let band_rows = (inner.height as usize / bands).max(2);
    let cell_rows = band_rows - 1;
    let today = today_bucket(&grid, buckets);

    let mut lines: Vec<Line> = Vec::with_capacity(bands * band_rows);
    for band in 0..bands {
        let start = first + band * cells;
        let count = cells.min(buckets - start);
        lines.push(Line::from(Span::styled(
            heat_axis(&grid, start, count, cell_width),
            Style::default().fg(theme::inactive()),
        )));
        for line in 0..cell_rows {
            // The marker names its cell once, so a tall band does not stack it.
            let named = line == cell_rows / 2;
            lines.push(Line::from(
                (start..start + count)
                    .map(|bucket| {
                        let style = Style::default().bg(cell_color(grid.grain, row[bucket], peak));
                        if today == Some(bucket) && named {
                            let pad = " ".repeat(cell_width.saturating_sub(1));
                            return Span::styled(
                                format!("{TODAY_MARKER}{pad}"),
                                style.fg(theme::highlight()).bold(),
                            );
                        }
                        Span::styled(" ".repeat(cell_width), style)
                    })
                    .collect::<Vec<Span>>(),
            ));
        }
    }
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// A cell's shade. `theme::heat_color`'s thresholds are day totals, so only a
/// day bucket may take them; every other grain reads against the row's own
/// `peak`, which a day threshold would paint uniformly empty.
fn cell_color(grain: Grain, cell: Duration, peak: i64) -> Color {
    match grain {
        Grain::Day => theme::heat_color(cell.num_hours()),
        _ => theme::heat_shade(cell.num_seconds(), peak),
    }
}

/// Which bucket holds this moment, or `None` when the period is not now.
/// `grid.start(buckets)` is the end the last bucket is closed at, so a period
/// wholly in the past marks nothing.
fn today_bucket(grid: &BucketGrid, buckets: usize) -> Option<usize> {
    let now = Local::now().naive_local();
    (0..buckets).find(|index| grid.start(*index) <= now && grid.start(index + 1) > now)
}

/// The axis that heads a strip of `cells` buckets, the first of them `first`:
/// one tick over the cell it belongs to, as the yearly overview heads its grid
/// with month names.
pub(super) fn heat_axis(
    grid: &BucketGrid,
    first: usize,
    cells: usize,
    cell_width: usize,
) -> String {
    let labels: Vec<Option<String>> = (0..cells)
        .map(|cell| axis_tick(grid, first + cell))
        .collect();
    axis_line(&labels, cell_width)
}

/// What bucket `index` is called on the axis, or `None` where no tick belongs.
/// The period's own length decides, never the grain alone: `Grain::Day` covers
/// both a seven-day week and a month, and a month of weekday names orients
/// nobody.
fn axis_tick(grid: &BucketGrid, index: usize) -> Option<String> {
    /// Hours between ticks, so a day reads `00 06 12 18`.
    const TICK_HOURS: u32 = 6;
    /// Day buckets a week holds; more than this is a month, not a week.
    const WEEK_DAYS: usize = 7;

    let start = grid.start(index);
    match grid.grain {
        Grain::Hour => start
            .hour()
            .is_multiple_of(TICK_HOURS)
            .then(|| format!("{:02}", start.hour())),
        // A week names its weekdays; a longer day axis takes the date of every
        // seventh bucket and nothing between, so a month reads `1 8 15 22 29`.
        Grain::Day if grid.len() <= WEEK_DAYS => Some(start.format("%a").to_string()),
        Grain::Day => index
            .is_multiple_of(WEEK_DAYS)
            .then(|| (index + 1).to_string()),
        // The week opening a month carries its name, as the yearly overview does.
        Grain::Week => (start.day() <= 7).then(|| start.format("%b").to_string()),
        Grain::Month => Some(start.format("%b").to_string()),
    }
}

/// One tick per labelled cell, each written over the cell it belongs to. A
/// tick is cut one column short of the next one, so a dense axis abbreviates
/// instead of running its labels together; the last may run to the end.
pub(super) fn axis_line(labels: &[Option<String>], cell_width: usize) -> String {
    let ticks: Vec<(usize, &String)> = labels
        .iter()
        .enumerate()
        .filter_map(|(cell, label)| label.as_ref().map(|text| (cell * cell_width, text)))
        .collect();
    let mut axis = vec![' '; labels.len() * cell_width];
    for (index, (column, text)) in ticks.iter().enumerate() {
        let room = ticks
            .get(index + 1)
            .map(|(next, _)| (next - column).saturating_sub(1).max(1))
            .unwrap_or_else(|| axis.len().saturating_sub(*column));
        for (offset, symbol) in text.chars().take(room).enumerate() {
            axis[column + offset] = symbol;
        }
    }
    axis.into_iter().collect()
}

/// The day-total ramp, `Less` to `More`, over `theme::heat_color`. Carries no
/// leading padding, so a caller can put it straight on a block border.
pub(super) fn day_heat_legend() -> Line<'static> {
    let t = theme::theme();
    let mut spans = vec![Span::styled(
        " Less ",
        Style::default().fg(theme::inactive()),
    )];
    for hours in [
        0,
        1,
        t.day_duration_med_h / 2,
        t.day_duration_med_h,
        t.day_duration_high_h,
    ] {
        spans.push(Span::styled(
            "  ",
            Style::default().bg(theme::heat_color(hours)),
        ));
    }
    spans.push(Span::styled(
        " More ",
        Style::default().fg(theme::inactive()),
    ));
    Line::from(spans)
}

/// The relative ramp, `Less` to `More`, over `theme::heat_shade` — the one the
/// grains too small for a day threshold read against. `None` when it would run
/// into the key legend sharing its border, whichever side each sits on.
pub(super) fn relative_heat_legend(keys_width: u16, inner_width: u16) -> Option<Line<'static>> {
    // Part and maximum per swatch, one per quarter of the ramp. An empty
    // bucket is never drawn, so the ramp does not offer a swatch for one.
    const SWATCHES: [(i64, i64); 4] = [(1, 4), (1, 2), (3, 4), (1, 1)];

    let mut spans = vec![Span::styled(
        " Less ",
        Style::default().fg(theme::inactive()),
    )];
    spans.extend(SWATCHES.iter().map(|(part, max)| {
        Span::styled("  ", Style::default().bg(theme::heat_shade(*part, *max)))
    }));
    spans.push(Span::styled(
        " More ",
        Style::default().fg(theme::inactive()),
    ));
    let line = Line::from(spans);
    (line.width() as u16 + keys_width <= inner_width).then_some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::storage::{env_guard, env_sandbox as sandbox};
    use crate::tracker::{TimeData, TimeEntry};
    use crate::tui::types::ViewMode;
    use chrono::NaiveDate;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn labels(texts: &[Option<&str>]) -> Vec<Option<String>> {
        texts.iter().map(|text| text.map(str::to_string)).collect()
    }

    /// A grid of `buckets` empty cells at `grain`, opening on 2026-01-01.
    fn grid(grain: Grain, buckets: usize) -> BucketGrid {
        BucketGrid {
            grain,
            anchor: NaiveDate::from_ymd_opt(2026, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            rows: vec![vec![Duration::zero(); buckets]],
        }
    }

    fn ticks(grid: &BucketGrid) -> Vec<Option<String>> {
        (0..grid.len())
            .map(|index| axis_tick(grid, index))
            .collect()
    }

    /// A month of weekday names orients nobody, so the long day axis takes the
    /// date of every seventh bucket instead.
    #[test]
    fn a_day_axis_longer_than_a_week_is_labelled_every_seventh_date() {
        let month = grid(Grain::Day, 31);
        let labelled: Vec<(usize, String)> = ticks(&month)
            .into_iter()
            .enumerate()
            .filter_map(|(index, tick)| tick.map(|text| (index, text)))
            .collect();
        assert_eq!(
            labelled,
            vec![
                (0, "1".to_string()),
                (7, "8".to_string()),
                (14, "15".to_string()),
                (21, "22".to_string()),
                (28, "29".to_string()),
            ]
        );
    }

    /// Seven day buckets are a week, and a week still names its weekdays.
    #[test]
    fn a_week_long_day_axis_keeps_its_weekday_names() {
        assert_eq!(
            ticks(&grid(Grain::Day, 7))
                .into_iter()
                .collect::<Option<Vec<String>>>()
                .expect("every weekday is named"),
            vec!["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"],
        );
    }

    /// An entry of `minutes` opening at `at`.
    fn entry(id: u64, at: chrono::NaiveDateTime, minutes: i64) -> TimeEntry {
        let start = at.and_local_timezone(Local).unwrap();
        TimeEntry {
            id,
            description: "seed".to_string(),
            project: Some("tt".to_string()),
            tags: Vec::new(),
            start_time: start,
            end_time: Some(start + Duration::minutes(minutes)),
            idle: Vec::new(),
            data: None,
        }
    }

    fn app_for(view: ViewMode, on: NaiveDate, entries: Vec<TimeEntry>) -> App {
        let next_id = entries.len() as u64;
        crate::storage::save_data(&TimeData {
            entries,
            next_id,
            schema_version: 1,
        })
        .unwrap();
        let mut app = App::new().unwrap();
        app.view_mode = view;
        app.selected_date = on;
        app
    }

    /// The drawn heat block as `(tick lines, one column list per painted row)`.
    /// A cell is a blank symbol, so only its background says it is there.
    fn drawn(app: &App, width: u16, height: u16) -> (Vec<String>, Vec<Vec<u16>>) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| render_heat_row(f, app, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut ticks = Vec::new();
        let mut rows = Vec::new();
        for y in 1..height.saturating_sub(1) {
            let painted: Vec<u16> = (1..width - 1)
                .filter(|x| buffer[(*x, y)].bg != Color::Reset)
                .collect();
            if painted.is_empty() {
                let text: String = (1..width - 1)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string();
                if !text.is_empty() {
                    ticks.push(text);
                }
            } else {
                rows.push(painted);
            }
        }
        (ticks, rows)
    }

    /// The colour of each cell of the first painted row, one per cell.
    fn shades(app: &App, width: u16, height: u16) -> Vec<Color> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| render_heat_row(f, app, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let y = (1..height)
            .find(|y| (1..width - 1).any(|x| buffer[(x, *y)].bg != Color::Reset))
            .expect("nothing painted");
        (1..width - 1)
            .map(|x| buffer[(x, y)].bg)
            .filter(|bg| *bg != Color::Reset)
            .collect()
    }

    /// The day's 24 hours stretch to the width, and the band takes every inner
    /// row the tick line leaves.
    #[test]
    fn a_day_heat_row_draws_every_hour_and_fills_the_box() {
        let _guard = env_guard();
        sandbox("heat-row-day");
        let today = Local::now().date_naive();
        let app = app_for(
            ViewMode::Day,
            today,
            vec![entry(0, today.and_hms_opt(9, 0, 0).unwrap(), 60)],
        );

        let (ticks, rows) = drawn(&app, 80, 10);
        assert_eq!(ticks.len(), 1, "one axis heads one band: {ticks:?}");
        // 78 inner columns over 24 hours: three each, and no bucket dropped.
        assert_eq!(rows[0].len(), 24 * 3);
        assert_eq!(rows.len(), 7, "the band left inner rows unpainted");

        let (_, taller) = drawn(&app, 80, 20);
        assert_eq!(taller[0].len(), 24 * 3, "a taller box changed the width");
        assert!(
            taller.len() > rows.len(),
            "the band did not grow with the box"
        );
    }

    /// One tick per cell would be a wall of numbers; the axis names a few.
    #[test]
    fn a_day_heat_row_heads_itself_with_sparse_ticks() {
        let _guard = env_guard();
        sandbox("heat-row-ticks");
        let today = Local::now().date_naive();
        let app = app_for(
            ViewMode::Day,
            today,
            vec![entry(0, today.and_hms_opt(9, 0, 0).unwrap(), 60)],
        );

        let (ticks, _) = drawn(&app, 80, 10);
        let axis = &ticks[0];
        assert!(axis.starts_with("00"), "the axis opens on the day: {axis}");
        for hour in ["06", "12", "18"] {
            assert!(axis.contains(hour), "no `{hour}` on the axis: {axis}");
        }
        assert_eq!(
            axis.split_whitespace().count(),
            4,
            "the axis named more than every sixth hour: {axis}"
        );
    }

    /// An hour is never a day's worth, so a day threshold would paint the whole
    /// row empty. The hour grain reads against the row's own peak instead.
    #[test]
    fn an_hour_row_shades_against_its_own_peak() {
        let _guard = env_guard();
        sandbox("heat-row-relative");
        let today = Local::now().date_naive();
        let app = app_for(
            ViewMode::Day,
            today,
            vec![
                entry(0, today.and_hms_opt(9, 0, 0).unwrap(), 60),
                entry(1, today.and_hms_opt(11, 0, 0).unwrap(), 15),
            ],
        );

        let cells = shades(&app, 80, 10);
        let busiest = cells[9 * 3];
        let quieter = cells[11 * 3];
        assert_eq!(
            busiest,
            theme::heat_shade(1, 1),
            "the peak hour is not full"
        );
        assert_eq!(quieter, theme::heat_shade(1, 4), "a quarter of the peak");
        assert_ne!(busiest, quieter, "the row shaded flat");
    }

    /// A month that will not fit wraps: it never hides a day of its own.
    #[test]
    fn a_bounded_period_wraps_instead_of_shedding_buckets() {
        let _guard = env_guard();
        sandbox("heat-row-wrap");
        let january = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        let app = app_for(
            ViewMode::Month,
            january,
            vec![entry(0, january.and_hms_opt(9, 0, 0).unwrap(), 60)],
        );

        let (ticks, rows) = drawn(&app, 20, 12);
        assert_eq!(ticks.len(), 2, "a wrapped row heads each band: {ticks:?}");
        let per_band: Vec<usize> = rows
            .iter()
            .map(Vec::len)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        assert_eq!(
            per_band,
            vec![13, 18],
            "18 columns, then January's other 13"
        );
    }

    /// `All` has no bounded period, so it sheds its oldest buckets as every
    /// other strip does rather than wrapping a window with no end.
    #[test]
    fn the_all_view_sheds_its_oldest_buckets() {
        let _guard = env_guard();
        sandbox("heat-row-shed");
        let today = Local::now().date_naive();
        let entries: Vec<TimeEntry> = (0..40)
            .map(|week| {
                entry(
                    week,
                    (today - Duration::days(week as i64 * 7))
                        .and_hms_opt(9, 0, 0)
                        .unwrap(),
                    60,
                )
            })
            .collect();
        let app = app_for(ViewMode::All, today, entries);

        let (ticks, rows) = drawn(&app, 30, 12);
        assert_eq!(
            ticks.len(),
            1,
            "`All` wrapped instead of shedding: {ticks:?}"
        );
        assert_eq!(rows[0].len(), 28, "the shed row did not fill the box");
    }

    /// The other grains read the bucket alone, whatever the period's length.
    #[test]
    fn the_hour_week_and_month_axes_are_unchanged() {
        let day = ticks(&grid(Grain::Hour, 24));
        assert_eq!(day[0].as_deref(), Some("00"));
        assert_eq!(day[6].as_deref(), Some("06"));
        assert_eq!(day[7], None);

        let all = ticks(&grid(Grain::Week, 10));
        assert_eq!(all[0].as_deref(), Some("Jan"), "the week opening a month");
        assert_eq!(all[1], None);

        let year = ticks(&grid(Grain::Month, 12));
        assert_eq!(year[0].as_deref(), Some("Jan"));
        assert_eq!(year[11].as_deref(), Some("Dec"));
    }

    /// A crowded tick gives up its tail; the last one keeps the whole row.
    #[test]
    fn a_tick_is_cut_one_column_short_of_its_neighbour() {
        let dense = axis_line(&labels(&[Some("Jan"), Some("Feb"), None, None]), 2);
        assert_eq!(dense, "J Feb   ", "`Jan` kept a column its neighbour needs");

        let sparse = axis_line(&labels(&[Some("Jan"), None, None, Some("Feb"), None]), 2);
        assert_eq!(
            sparse, "Jan   Feb ",
            "a tick with room keeps its whole name"
        );
    }

    /// The row's width binds the last tick as a neighbour would.
    #[test]
    fn the_last_tick_is_cut_at_the_end_of_the_row() {
        assert_eq!(axis_line(&labels(&[None, Some("Feb")]), 2), "  Fe");
    }

    /// Nothing labelled is a blank axis of the row's own width.
    #[test]
    fn an_unlabelled_axis_is_blank_and_as_wide_as_the_cells() {
        assert_eq!(axis_line(&labels(&[None, None, None]), 3), " ".repeat(9));
        assert_eq!(axis_line(&[], 3), "");
    }
}
