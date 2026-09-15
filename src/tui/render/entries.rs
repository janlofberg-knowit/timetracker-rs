use super::heat::{TODAY_MARKER, axis_line, day_heat_legend, heat_block_title};
use super::overlay::CURSOR_MARKER;
use crate::tracker::TimeData;
use crate::tui::panes::Polarity;
use crate::tui::rows::{GroupHeader, Member, VisibleRow};
use crate::tui::summary::strip_cells;
use crate::tui::{App, theme};
use chrono::{Datelike, Duration, Local, NaiveDate};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};
use std::collections::HashMap;

/// A GitHub-style yearly contribution heatmap: one column band per week, one
/// row band per weekday, each cell shaded by `theme::heat_color` for that
/// day's total. The grid fills the block in both axes; when the width runs out
/// the oldest weeks shed, since a window onto the year still reads as the year.
pub(super) fn render_year_heatmap(f: &mut Frame, app: &App, area: Rect) {
    const GUTTER: usize = 4; // weekday label width, e.g. "Mon "
    const WEEKDAY_LABELS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

    let year = app.selected_date.year();
    let breakdown = app.data.year_breakdown(year);
    let today = Local::now().date_naive();

    let grid_start = TimeData::week_start(NaiveDate::from_ymd_opt(year, 1, 1).unwrap());
    let grid_end =
        TimeData::week_start(NaiveDate::from_ymd_opt(year, 12, 31).unwrap()) + Duration::days(6);
    let total_weeks = ((grid_end - grid_start).num_days() / 7 + 1) as usize;

    // Cells are asked for in pairs of columns, so a week keeps its two-column
    // minimum and the oldest weeks still shed once the room runs out.
    let available = (area.width as usize)
        .saturating_sub(2)
        .saturating_sub(GUTTER);
    let (pairs, visible_weeks) = strip_cells(available / 2, total_weeks);
    let cell_width = pairs * 2;
    let first_week = total_weeks - visible_weeks;

    // Seven bands over the rows the month header leaves.
    let cell_height = ((area.height as usize).saturating_sub(3) / 7).max(1);
    let middle = cell_height / 2;

    let monday_of = |week: usize| grid_start + Duration::days(((first_week + week) * 7) as i64);
    let header = axis_line(
        &(0..visible_weeks)
            .map(|week| {
                let monday = monday_of(week);
                (monday.year() == year && monday.day() <= 7)
                    .then(|| monday.format("%b").to_string())
            })
            .collect::<Vec<_>>(),
        cell_width,
    );
    let mut lines = vec![Line::from(Span::styled(
        format!("{}{}", " ".repeat(GUTTER), header),
        Style::default().fg(theme::inactive()),
    ))];

    for (weekday, label) in WEEKDAY_LABELS.iter().enumerate() {
        for row in 0..cell_height {
            // The label names the band once, so the gutter does not read as
            // seven repeated words.
            let named = row == middle;
            let mut spans = vec![Span::styled(
                format!(
                    "{:<width$}",
                    if named { *label } else { "" },
                    width = GUTTER
                ),
                Style::default().fg(theme::inactive()),
            )];
            for week in 0..visible_weeks {
                let date = monday_of(week) + Duration::days(weekday as i64);
                if date.year() != year {
                    spans.push(Span::raw(" ".repeat(cell_width)));
                    continue;
                }
                let hours = breakdown.get(&date).map(|d| d.num_hours()).unwrap_or(0);
                let cell_style = Style::default().bg(theme::heat_color(hours));
                if date == today && named {
                    let pad = " ".repeat(cell_width.saturating_sub(1));
                    spans.push(Span::styled(
                        format!("{TODAY_MARKER}{pad}"),
                        cell_style.fg(theme::highlight()).bold(),
                    ));
                } else {
                    spans.push(Span::styled(" ".repeat(cell_width), cell_style));
                }
            }
            lines.push(Line::from(spans));
        }
    }

    let total = breakdown.values().fold(Duration::zero(), |acc, d| acc + *d);
    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::border()))
            .title(Span::styled(
                heat_block_title(total, breakdown.len(), "day"),
                Style::default().fg(theme::title()),
            ))
            .title_bottom(day_heat_legend().right_aligned()),
    );
    f.render_widget(paragraph, area);
}

pub(super) fn render_weekly_breakdown(f: &mut Frame, app: &App, area: Rect) {
    let week_start = TimeData::week_start(app.selected_date);
    let breakdown = app.data.daily_breakdown(week_start);

    let rows: Vec<Row> = breakdown
        .iter()
        .map(|(date, dur)| {
            let day_name = date.format("%a").to_string();
            let date_str = date.format("%m/%d").to_string();
            let dur_str = crate::duration::format(*dur);
            let is_today = *date == Local::now().date_naive();
            let hours = dur.num_hours();

            let dur_color = theme::duration_color(
                hours,
                theme::theme().day_duration_high_h,
                theme::theme().day_duration_med_h,
            );

            let (day_style, date_style) = if is_today {
                (
                    Style::default().fg(theme::highlight()).bold(),
                    Style::default().fg(theme::highlight()),
                )
            } else {
                (
                    Style::default().fg(theme::accent()),
                    Style::default().fg(theme::title()),
                )
            };

            Row::new(vec![
                Cell::from(day_name).style(day_style),
                Cell::from(date_str).style(date_style),
                Cell::from(dur_str).style(Style::default().fg(dur_color)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Min(8),
        ],
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::border()))
            .title(Span::styled(
                " Daily Totals ",
                Style::default().fg(theme::title()),
            )),
    );
    f.render_widget(table, area);
}

/// One entry's row. A member row hangs off its group header's tree connector
/// and carries the tint rather than the stripe.
fn entry_row<'e>(
    entry: &'e crate::tracker::TimeEntry,
    stripe: bool,
    member: Option<&Member>,
) -> Row<'e> {
    let hours = entry.duration().num_hours();
    let dur_color = theme::duration_color(
        hours,
        theme::theme().entry_duration_high_h,
        theme::theme().entry_duration_med_h,
    );

    let status_style = if entry.is_active() {
        Style::default().fg(theme::active())
    } else {
        Style::default().fg(theme::inactive())
    };

    let row_style = match (member, stripe) {
        (Some(_), _) => Style::default().bg(theme::member_bg()),
        (None, true) => Style::default().bg(Color::Rgb(35, 35, 35)),
        (None, false) => Style::default(),
    };
    // The connector opens in the column the group header's chevron sits in, so
    // its stroke runs unbroken from the header down to the last member.
    let description = match member {
        Some(member) if member.last => format!("\u{2514}\u{2500}\u{2500} {}", entry.description),
        Some(_) => format!("\u{251c}\u{2500}\u{2500} {}", entry.description),
        None => entry.description.clone(),
    };

    Row::new(vec![
        Cell::from(entry.format_date()).style(Style::default().fg(theme::title())),
        Cell::from(entry.format_start_time()).style(Style::default().fg(theme::accent())),
        Cell::from(entry.format_end_time()).style(Style::default().fg(theme::inactive())),
        Cell::from(description),
        Cell::from(entry.format_tags()).style(Style::default().fg(theme::highlight())),
        Cell::from(entry.format_duration()).style(Style::default().fg(dur_color)),
        Cell::from(entry.status_icon()).style(status_style),
    ])
    .style(row_style)
}

/// The live sum of the entries at `members`. Summed per frame and never
/// cached: a running member's duration moves with the clock.
fn members_total(entries: &[crate::tracker::TimeEntry], members: &[usize]) -> Duration {
    members
        .iter()
        .filter_map(|index| entries.get(*index))
        .fold(Duration::zero(), |acc, entry| acc + entry.duration())
}

fn day_header_row(date: NaiveDate, total: Duration) -> Row<'static> {
    let weekday = format!("\n{}", date.format("%A"));
    let date_str = format!("\n{}", date.format("%B %d, %Y"));
    let total_str = format!("\n{}", crate::duration::format(total));

    Row::new(vec![
        Cell::from(weekday).style(
            Style::default()
                .fg(theme::highlight())
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(""),
        Cell::from(""),
        Cell::from(date_str).style(Style::default().fg(theme::title())),
        Cell::from(""),
        Cell::from(total_str).style(
            Style::default()
                .fg(theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(""),
    ])
    .height(2)
    .style(Style::default().bg(theme::day_header_bg()))
}

/// One issue's collapsed row: its span, its member count and id behind a
/// chevron, and the members' summed duration.
fn group_header_row(header: &GroupHeader, entries: &[crate::tracker::TimeEntry]) -> Row<'static> {
    let total = members_total(entries, &header.members);
    let dur_color = theme::duration_color(
        total.num_hours(),
        theme::theme().entry_duration_high_h,
        theme::theme().entry_duration_med_h,
    );
    let chevron = if header.expanded {
        "\u{25be}"
    } else {
        "\u{25b8}"
    };
    let end = header
        .end
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default();
    let icon = if header.end.is_none() {
        crate::icons::active()
    } else {
        ""
    };

    Row::new(vec![
        Cell::from(header.start.format("%Y-%m-%d").to_string())
            .style(Style::default().fg(theme::title())),
        Cell::from(header.start.format("%H:%M").to_string())
            .style(Style::default().fg(theme::accent())),
        Cell::from(end).style(Style::default().fg(theme::inactive())),
        // The count leads: this column is 19 cells at 80 columns, so whatever
        // comes second is what the clip takes.
        Cell::from(format!(
            "{chevron} {} entries - {}",
            header.members.len(),
            header.tag
        ))
        .style(Style::default().add_modifier(Modifier::BOLD)),
        // Stored tags carry no `#`; the display prefix comes from `format_tags`.
        Cell::from(crate::tracker::format_tags(std::slice::from_ref(
            &header.tag,
        )))
        .style(Style::default().fg(theme::highlight())),
        Cell::from(crate::duration::format(total)).style(Style::default().fg(dur_color)),
        Cell::from(icon).style(Style::default().fg(theme::active())),
    ])
    .style(Style::default().bg(theme::group_header_bg()))
}

pub(super) fn render_entries_table(f: &mut Frame, app: &mut App, area: Rect) {
    let header_cells = [
        "Date",
        "Start",
        "End",
        "Description",
        "Tags",
        "Duration",
        "",
    ]
    .into_iter()
    .map(|h| {
        Cell::from(h).style(
            Style::default()
                .fg(theme::accent())
                .add_modifier(Modifier::BOLD),
        )
    });
    let header_row = Row::new(header_cells)
        .height(1)
        .style(Style::default().bg(theme::header_bg()));

    // Day totals are summed here rather than carried on the row, so a running
    // entry's minutes keep moving under a cached row model.
    let mut day_totals: HashMap<NaiveDate, Duration> = HashMap::new();
    for entry in app.filtered_entries() {
        *day_totals
            .entry(entry.start_time.date_naive())
            .or_insert_with(Duration::zero) += entry.duration();
    }

    // One walk over the row model builds the visual rows and, beside them, the
    // map from the cursor's selectable index to the visual index it draws at.
    let model = app.rows();
    let mut rows: Vec<Row> = Vec::with_capacity(model.len());
    let mut visual_of_selectable: Vec<usize> = Vec::with_capacity(model.len());
    let mut stripe = false;
    for row in &model {
        match row {
            VisibleRow::DayHeader { date } => {
                // The stripe alternation restarts inside each day partition.
                stripe = false;
                let total = day_totals.get(date).copied().unwrap_or_else(Duration::zero);
                rows.push(day_header_row(*date, total));
            }
            VisibleRow::GroupHeader(header) => {
                visual_of_selectable.push(rows.len());
                rows.push(group_header_row(header, &app.data.entries));
            }
            VisibleRow::Entry { index, member } => {
                if let Some(entry) = app.data.entries.get(*index) {
                    visual_of_selectable.push(rows.len());
                    rows.push(entry_row(entry, stripe, member.as_ref()));
                    // A member carries no stripe, so an expansion leaves the
                    // top-level alternation around it as it was.
                    if member.is_none() {
                        stripe = !stripe;
                    }
                }
            }
        }
    }
    let visual_selected = app
        .table_state
        .selected()
        .and_then(|idx| visual_of_selectable.get(idx).copied());

    // `(tt)` and `#impl`, the CLI's own sigils, so the title needs no legend;
    // an excluded value carries a `-` prefix.
    let title = if app.is_filtering() {
        let negate = |p: Polarity| if p == Polarity::Exclude { "-" } else { "" };
        let values: Vec<String> = app
            .project_filter
            .values()
            .map(|(v, p)| format!("{}({})", negate(p), v))
            .chain(
                app.tag_filter
                    .values()
                    .map(|(v, p)| format!("{}#{}", negate(p), v)),
            )
            .collect();
        format!(" Entries [filtered: {}] ", values.join(" "))
    } else {
        " Entries ".to_string()
    };

    // `Fill(1)` and `Min(12)` share what the fixed columns leave, so both grow with
    // the terminal. The fixed widths are exactly what they render, with no padding.
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(12),
            Constraint::Fill(1),
            Constraint::Length(8),
            Constraint::Length(3),
        ],
    )
    .header(header_row)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::border()))
            .title(Span::styled(title, Style::default().fg(theme::title()))),
    )
    .row_highlight_style(Style::default().bg(theme::selected_bg()))
    .highlight_symbol(CURSOR_MARKER);

    let mut render_state = TableState::default().with_selected(visual_selected);
    f.render_stateful_widget(table, area, &mut render_state);
}
