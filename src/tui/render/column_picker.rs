use super::overlay::{CURSOR_MARKER, overlay_hints, render_overlay, wrap};
use crate::tui::render::columns::EntryColumn;
use crate::tui::{App, theme};
use ratatui::{prelude::*, widgets::Paragraph};

/// The `c` popup: one row per column, its number or a dash, and the order
/// the current ranks derive. Never reordered while the user types. A number
/// two rows share draws red until the tie is gone.
pub(super) fn render_column_picker_popup(f: &mut Frame, app: &App) {
    let mut lines: Vec<Line> = Vec::new();
    for (index, column) in EntryColumn::ALL_WITH_PROJECT.into_iter().enumerate() {
        let cursor = if index == app.column_cursor {
            CURSOR_MARKER
        } else {
            "   "
        };
        let rank = match app.column_ranks[index] {
            Some(rank) => rank.to_string(),
            None => "-".to_string(),
        };
        let selected = index == app.column_cursor;
        let colour = if app.rank_is_shared(index) {
            theme::error()
        } else if selected {
            theme::highlight()
        } else if app.column_ranks[index].is_some() {
            theme::active()
        } else {
            theme::inactive()
        };
        // Bold marks the cursor row only; colour carries the state.
        let row_style = if selected {
            Style::default().fg(colour).bold()
        } else {
            Style::default().fg(colour)
        };
        lines.push(Line::from(vec![
            Span::styled(cursor, Style::default().fg(theme::accent()).bold()),
            Span::styled(format!("{rank:>2} "), row_style),
            Span::styled(column.label(), row_style),
        ]));
    }

    let order = app.column_picker_order();
    let foot = if order.is_empty() {
        "Number at least one column".to_string()
    } else {
        let names: Vec<&str> = order.iter().map(|c| c.label()).collect();
        format!("Order: {}", names.join("  "))
    };

    let widest = lines
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(0)
        .max(foot.chars().count());
    let width = (widest as u16 + 4).clamp(40, 72);
    let inner_width = width.saturating_sub(2) as usize;

    lines.push(Line::from(""));
    for line in wrap(&foot, inner_width) {
        lines.push(Line::from(Span::styled(
            line,
            Style::default().fg(theme::title()),
        )));
    }

    let content = render_overlay(
        f,
        width,
        lines.len() as u16 + 3,
        Span::styled(" Columns ", Style::default().fg(theme::highlight()).bold()),
        Span::styled(" c ", Style::default().fg(theme::inactive())),
        overlay_hints(&[
            ("j/k", "move"),
            ("1-9", "number"),
            ("0/space", "clear"),
            ("enter", "apply"),
            ("esc", "cancel"),
        ]),
    );
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::overlay_bg())),
        content,
    );
}
