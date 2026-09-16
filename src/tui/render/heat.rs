//! What every heat surface draws alike: the tick axis over its cells, the
//! `Less … More` ramps, and the content pane's own two-dimensional grid.

use super::legend::content_legend;
use crate::tui::summary::{BucketGrid, Grain, HeatBand, strip_cells};
use crate::tui::types::ViewMode;
use crate::tui::{App, theme};
use chrono::{Datelike, Duration, Timelike};
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

/// The open view's period as a grid of heat cells filling the content area: a
/// gutter of row labels, a tick axis per band, and cells stretched to the room
/// the box has in both axes. A cell is shaded by how full its own span is, so
/// the shading does not re-scale as the user pages through periods.
pub(super) fn render_heat_grid(f: &mut Frame, app: &App, area: Rect) {
    let inner_width = area.width.saturating_sub(2);
    let inner_height = area.height.saturating_sub(2);
    let grid = app.view_heat_grid(inner_width, inner_height);
    let day_sized = grid
        .bands
        .first()
        .is_some_and(|band| band.cell_span >= Duration::days(1));

    let keys = content_legend(app, inner_width);
    let keys_width = keys.as_ref().map(|line| line.width()).unwrap_or(0) as u16;
    let ramp = if day_sized {
        day_heat_legend(keys_width, inner_width)
    } else {
        relative_heat_legend(keys_width, inner_width)
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::border()))
        .title(Span::styled(
            heat_block_title(grid.total, grid.active(), grid.unit),
            Style::default().fg(theme::title()),
        ));
    if let Some(keys) = keys {
        block = block.title_bottom(keys.left_aligned());
    }
    if let Some(ramp) = ramp {
        block = block.title_bottom(ramp.right_aligned());
    }

    let available = (inner_width as usize).saturating_sub(grid.gutter);
    let (drawn, first_row) = scrolled(app, &grid, inner_height);
    let rows_total: usize = drawn
        .iter()
        .map(|band| band.rows())
        .sum::<usize>()
        .saturating_sub(first_row);
    if rows_total == 0 || available == 0 {
        f.render_widget(Paragraph::new(Vec::<Line>::new()).block(block), area);
        return;
    }
    // Every band keeps its axis line; the rows share what is left.
    let cell_height = ((inner_height as usize).saturating_sub(drawn.len()) / rows_total).max(1);
    let dim = Style::default().fg(theme::inactive());

    let mut lines: Vec<Line> = Vec::new();
    for band in drawn {
        let (cell_width, columns) = strip_cells(available, band.columns());
        lines.push(Line::from(Span::styled(
            format!(
                "{}{}",
                " ".repeat(grid.gutter),
                axis_line(&band.column_ticks[..columns], cell_width)
            ),
            dim,
        )));
        for (row, cells) in band.cells.iter().enumerate().skip(first_row) {
            for line in 0..cell_height {
                // The label names its band once, so a tall row does not stack it.
                let named = line == cell_height / 2;
                let opens = row == first_row && line == 0;
                let label = match (&band.title, opens, named) {
                    (Some(title), true, _) => title.clone(),
                    (_, _, true) => gutter_label(&band.row_labels[row], grid.gutter),
                    _ => " ".repeat(grid.gutter),
                };
                let mut spans = vec![Span::styled(label, dim)];
                for (column, cell) in cells.iter().take(columns).enumerate() {
                    let Some(held) = cell else {
                        spans.push(Span::raw(" ".repeat(cell_width)));
                        continue;
                    };
                    let style = Style::default().bg(if day_sized {
                        theme::heat_color(held.num_hours())
                    } else {
                        theme::heat_shade(held.num_seconds(), band.cell_span.num_seconds())
                    });
                    if band.now_column == Some(column) && marks_now(band, row, named) {
                        let pad = " ".repeat(cell_width.saturating_sub(1));
                        spans.push(Span::styled(
                            format!("{TODAY_MARKER}{pad}"),
                            style.fg(theme::highlight()).bold(),
                        ));
                    } else {
                        spans.push(Span::styled(" ".repeat(cell_width), style));
                    }
                }
                lines.push(Line::from(spans));
            }
        }
    }
    lines.truncate(inner_height as usize);
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// The bands to draw and the first row of each, once `heat_scroll` is taken
/// against the room the box has. `All` scrolls whole year bands; the Day view
/// scrolls its project rows. Both clamp so the last one rests at the bottom.
fn scrolled<'a>(
    app: &App,
    grid: &'a crate::tui::summary::HeatGrid,
    inner_height: u16,
) -> (&'a [HeatBand], usize) {
    let room = inner_height as usize;
    if app.view_mode == ViewMode::All {
        let per_band = grid.bands.first().map_or(1, |band| 1 + band.rows());
        let fits = (room / per_band).max(1);
        let first = app.heat_scroll.min(grid.bands.len().saturating_sub(fits));
        return (&grid.bands[first..], 0);
    }
    let rows = grid.bands.first().map_or(0, HeatBand::rows);
    let fits = room.saturating_sub(1);
    (&grid.bands, app.heat_scroll.min(rows.saturating_sub(fits)))
}

/// Whether the marker belongs on this line of `row`. A band whose rows are
/// time names its own row; one whose rows are projects marks its middle.
fn marks_now(band: &HeatBand, row: usize, named: bool) -> bool {
    named
        && match band.now_row {
            Some(now) => now == row,
            None => row == band.rows() / 2,
        }
}

/// A gutter label padded to `width`, cut to leave a column before the cells.
/// A band title takes the whole width, so a year is not cut to three digits.
fn gutter_label(text: &str, width: usize) -> String {
    let label: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{label:<width$}")
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

/// The day-total ramp, `Less` to `More`, over `theme::heat_color`. `None` when
/// it would run into the key legend sharing its border, as its relative
/// sibling is.
pub(super) fn day_heat_legend(keys_width: u16, inner_width: u16) -> Option<Line<'static>> {
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
    let line = Line::from(spans);
    (line.width() as u16 + keys_width <= inner_width).then_some(line)
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
    use chrono::{Local, NaiveDate};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn at_noon(year: i32, month: u32, day: u32) -> chrono::NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
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
        app.heat_view = true;
        app
    }

    /// Every inner line of the drawn block, as `(text, painted columns)`.
    fn drawn(app: &App, width: u16, height: u16) -> Vec<(String, Vec<u16>)> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| render_heat_grid(f, app, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (1..height.saturating_sub(1))
            .map(|y| {
                let text: String = (1..width - 1).map(|x| buffer[(x, y)].symbol()).collect();
                let painted: Vec<u16> = (1..width - 1)
                    .filter(|x| buffer[(*x, y)].bg != Color::Reset)
                    .collect();
                (text.trim_end().to_string(), painted)
            })
            .collect()
    }

    /// `All` stacks a band per year; the scroll offset drops the ones above.
    #[test]
    fn the_all_grid_scrolls_past_its_first_year_band() {
        let _guard = env_guard();
        sandbox("heat-grid-all-scroll");
        let mut app = app_for(
            ViewMode::All,
            NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
            vec![
                entry(0, at_noon(2026, 3, 4), 60),
                entry(1, at_noon(2024, 3, 4), 60),
            ],
        );

        let named = |app: &App| {
            drawn(app, 80, 12)
                .into_iter()
                .filter(|(text, _)| text.trim() == "2026" || text.trim() == "2024")
                .map(|(text, _)| text.trim().to_string())
                .collect::<Vec<String>>()
        };
        assert_eq!(named(&app)[0], "2026", "the newest year is not on top");

        app.heat_scroll = 1;
        assert_eq!(
            named(&app),
            vec!["2024".to_string()],
            "the offset did not move the bands"
        );

        app.heat_scroll = 9;
        assert_eq!(
            named(&app),
            vec!["2024".to_string()],
            "the last band scrolled off the box"
        );
    }

    /// The gutter names each project once and the cells fill what is left.
    #[test]
    fn a_day_grid_draws_a_project_row_over_the_hours_of_the_day() {
        let _guard = env_guard();
        sandbox("heat-grid-day-render");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let lines = drawn(
            &app_for(
                ViewMode::Day,
                day,
                vec![entry(0, day.and_hms_opt(9, 0, 0).unwrap(), 60)],
            ),
            80,
            12,
        );

        let axis = &lines[0].0;
        assert!(axis.starts_with(&" ".repeat(12)), "no gutter: {axis}");
        for hour in ["00", "06", "12", "18"] {
            assert!(axis.contains(hour), "no `{hour}` on the axis: {axis}");
        }
        assert!(
            lines.iter().any(|(text, _)| text.trim() == "tt"),
            "the gutter did not name the project"
        );
        let painted: Vec<usize> = lines
            .iter()
            .map(|(_, cells)| cells.len())
            .filter(|count| *count > 0)
            .collect();
        // 78 inner columns less the 12-column gutter: 66 over 24 hours.
        assert!(
            painted.iter().all(|count| *count == 24 * 2),
            "a cell was clipped or dropped: {painted:?}"
        );
        // Ten inner lines less the axis: the one row takes the other nine.
        assert_eq!(painted.len(), 9, "the row did not fill the box");
    }

    /// A cell shades by how full its own span is, not by the busiest cell, so
    /// two half-full hours read alike whatever else the day holds.
    #[test]
    fn a_cell_shades_by_the_fill_of_its_own_span() {
        let _guard = env_guard();
        sandbox("heat-grid-fill");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let app = app_for(
            ViewMode::Day,
            day,
            vec![
                entry(0, day.and_hms_opt(9, 0, 0).unwrap(), 60),
                entry(1, day.and_hms_opt(11, 0, 0).unwrap(), 30),
            ],
        );

        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal
            .draw(|f| render_heat_grid(f, &app, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let cells: Vec<Color> = (1..79).map(|x| buffer[(x, 2)].bg).collect();
        let gutter = 12;
        assert_eq!(
            cells[gutter + 9 * 2],
            theme::heat_shade(1, 1),
            "a full hour is not full"
        );
        assert_eq!(
            cells[gutter + 11 * 2],
            theme::heat_shade(1, 2),
            "half an hour is not half"
        );
    }

    /// A short box coarsens the rows; nothing of the week is hidden.
    #[test]
    fn a_week_grid_is_blocks_of_the_day_over_the_weekdays() {
        let _guard = env_guard();
        sandbox("heat-grid-week-render");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let app = app_for(
            ViewMode::Week,
            day,
            vec![entry(0, day.and_hms_opt(9, 0, 0).unwrap(), 60)],
        );

        let lines = drawn(&app, 80, 16);
        assert!(
            lines[0].0.contains("Mon"),
            "no weekday axis: {}",
            lines[0].0
        );
        assert!(lines[0].0.contains("Sun"));
        let labels: Vec<&str> = lines
            .iter()
            .map(|(text, _)| text.trim())
            .filter(|text| !text.is_empty())
            .collect();
        assert!(labels.contains(&"00"), "midnight is unlabelled: {labels:?}");
        assert!(labels.contains(&"22"), "the last block is unlabelled");
        let painted = lines.iter().filter(|(_, cells)| !cells.is_empty()).count();
        assert_eq!(painted, 12, "the twelve blocks were not all drawn");
    }

    /// 80 columns is the narrow case every view must still fit.
    #[test]
    fn a_month_grid_keeps_every_day_at_eighty_columns() {
        let _guard = env_guard();
        sandbox("heat-grid-month-render");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let app = app_for(
            ViewMode::Month,
            day,
            vec![entry(0, day.and_hms_opt(9, 0, 0).unwrap(), 60)],
        );

        let lines = drawn(&app, 80, 16);
        let widest = lines
            .iter()
            .map(|(_, cells)| cells.len())
            .max()
            .unwrap_or(0);
        // 74 columns over January's 31 days: two each, none shed.
        assert_eq!(widest, 62, "a day of the month was dropped");
        assert!(
            lines.iter().all(|(text, _)| text.chars().count() <= 78),
            "a line ran past the inner width"
        );
    }

    /// The block title counts what the entry list under it counts.
    #[test]
    fn the_block_title_totals_the_filtered_entries() {
        let _guard = env_guard();
        sandbox("heat-grid-title");
        let day = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let app = app_for(
            ViewMode::Day,
            day,
            vec![
                entry(0, day.and_hms_opt(9, 0, 0).unwrap(), 60),
                entry(1, day.and_hms_opt(11, 0, 0).unwrap(), 30),
            ],
        );

        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal
            .draw(|f| render_heat_grid(f, &app, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let title: String = (0..80).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(
            title.contains(&crate::duration::format(app.filtered_total())),
            "the title disagrees with the list: {title}"
        );
        assert!(title.contains("2 active hours"), "{title}");
    }

    fn labels(texts: &[Option<&str>]) -> Vec<Option<String>> {
        texts.iter().map(|text| text.map(str::to_string)).collect()
    }

    /// A grid of `buckets` empty cells at `grain`, opening on 2026-01-01.
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
