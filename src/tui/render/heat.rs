//! What every heat surface draws alike: the tick axis over a row of cells and
//! the `Less … More` ramp over `theme::heat_color`.

use crate::tui::theme;
use ratatui::prelude::*;

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

    fn labels(texts: &[Option<&str>]) -> Vec<Option<String>> {
        texts.iter().map(|text| text.map(str::to_string)).collect()
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
