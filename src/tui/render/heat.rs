//! What every heat surface draws alike: the tick axis over a row of cells and
//! the `Less … More` ramp over `theme::heat_color`.

use crate::tui::summary::{BucketGrid, Grain};
use crate::tui::theme;
use chrono::{Datelike, Timelike};
use ratatui::prelude::*;

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

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::{Duration, NaiveDate};

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
