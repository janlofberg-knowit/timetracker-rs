//! The key legend a surface draws on its bottom border.

use crate::tui::theme;
use ratatui::prelude::*;

/// Between two entries.
const SEPARATOR: &str = " \u{b7} ";

/// The keys a surface owns, as `(key, label, accented)`, most useful first.
/// The tail sheds while the line is wider than `width`; never a clipped line.
pub(super) fn legend(entries: &[(&str, &str, bool)], width: u16) -> Option<Line<'static>> {
    (1..=entries.len())
        .rev()
        .map(|kept| line(&entries[..kept]))
        .find(|line| line.width() <= width as usize)
}

fn line(entries: &[(&str, &str, bool)]) -> Line<'static> {
    let dim = Style::default().fg(theme::inactive());
    let mut spans = vec![Span::raw(" ")];
    for (index, (key, label, accented)) in entries.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(
                SEPARATOR,
                Style::default().fg(theme::border()),
            ));
        }
        let key_style = if *accented {
            Style::default().fg(theme::accent())
        } else {
            dim
        };
        spans.push(Span::styled((*key).to_string(), key_style));
        spans.push(Span::styled(format!(": {label}"), dim));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn pane_entries() -> [(&'static str, &'static str, bool); 3] {
        [
            ("Enter", "filter", false),
            ("-", "back", false),
            ("j/k", "move", false),
        ]
    }

    #[test]
    fn the_whole_legend_is_drawn_while_it_fits() {
        let line = legend(&pane_entries(), 37).expect("the full legend fits 37 cells");
        assert_eq!(
            text(&line),
            " Enter: filter \u{b7} -: back \u{b7} j/k: move "
        );
    }

    #[test]
    fn a_narrow_box_sheds_the_trailing_entries_one_at_a_time() {
        let line = legend(&pane_entries(), 36).expect("two entries fit 36 cells");
        assert_eq!(text(&line), " Enter: filter \u{b7} -: back ");

        let line = legend(&pane_entries(), 24).expect("one entry fits 24 cells");
        assert_eq!(text(&line), " Enter: filter ");
    }

    #[test]
    fn a_box_too_narrow_for_the_first_entry_draws_nothing() {
        assert!(legend(&pane_entries(), 14).is_none());
    }
}
