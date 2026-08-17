use std::collections::HashMap;
use std::rc::Rc;
use chrono::{Duration, Local, NaiveDate};
use ratatui::{
    prelude::*,
    widgets::{
        Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState,
        Tabs,
    },
};
use crate::tracker::TimeData;
use super::{theme, App};
use super::types::{InputField, InputMode, Pane, ViewMode};

/// What marks the row a cursor is on, in every list that has a cursor: the entries
/// table and both P/T panes. One string so the lists stay recognisably the same
/// mechanism, and so the width a list has to reserve for it is never out of step
/// with what gets drawn.
const CURSOR_MARKER: &str = ">> ";

/// A named vertical row of the main layout.
///
/// Some rows are conditional — the search bar only while searching, the P/T pane
/// surface only while a pane is open — so the row a given index refers to depends
/// on which conditions hold. Numbering them by hand meant one shifted-index pair
/// per combination (`if show_search { (3, 4) } else { (2, 3) }`), which multiplies
/// with every new conditional row. `LayoutRows` keeps the names and the
/// constraints in lockstep instead and resolves a row to its `Rect` by name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LayoutRow {
    Status,
    Marks,
    Panes,
    Tabs,
    Search,
    Content,
    Footer,
}

/// The rows actually laid out this frame, paired with their areas.
struct LayoutRows {
    names: Vec<LayoutRow>,
    areas: Rc<[Rect]>,
}

impl LayoutRows {
    /// `plan` is the rows to lay out, top to bottom; a conditional row is simply
    /// left out rather than given a zero-height constraint.
    fn split(area: Rect, plan: Vec<(LayoutRow, Constraint)>) -> Self {
        let (names, constraints): (Vec<LayoutRow>, Vec<Constraint>) = plan.into_iter().unzip();
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);
        Self { names, areas }
    }

    /// The row's area, or `None` when this frame does not include the row.
    fn get(&self, row: LayoutRow) -> Option<Rect> {
        self.names
            .iter()
            .position(|name| *name == row)
            .map(|idx| self.areas[idx])
    }

    /// For the rows that are always part of the layout.
    fn area(&self, row: LayoutRow) -> Rect {
        self.get(row)
            .unwrap_or_else(|| panic!("layout row {row:?} is always present"))
    }
}

pub fn ui(f: &mut Frame, app: &mut App) {
    let mut plan = vec![(LayoutRow::Status, Constraint::Length(3))];
    // Marks sit between Status and the panes: it answers the same question Status
    // does — what is running — and Status alone cannot see a `tt-safe` phase.
    let marks_height = app.marks_surface_height();
    if marks_height > 0 {
        plan.push((LayoutRow::Marks, Constraint::Length(marks_height)));
    }
    let pane_height = app.pane_surface_height();
    if pane_height > 0 {
        plan.push((LayoutRow::Panes, Constraint::Length(pane_height)));
    }
    plan.push((LayoutRow::Tabs, Constraint::Length(3))); // Tabs + date info
    if app.is_searching() {
        plan.push((LayoutRow::Search, Constraint::Length(3)));
    }
    plan.push((LayoutRow::Content, Constraint::Min(10)));
    plan.push((LayoutRow::Footer, Constraint::Length(3)));
    let rows = LayoutRows::split(f.area(), plan);

    // Status header
    let (status_text, status_style) = match app.data.active_entry() {
        Some(entry) => (
            format!(
                "{}  {} - {} ",
                crate::icons::ACTIVE,
                entry.description,
                entry.format_duration()
            ),
            Style::default().fg(theme::ACTIVE).bold(),
        ),
        None => (
            "No active task".to_string(),
            Style::default().fg(theme::INACTIVE).italic(),
        ),
    };
    let header = Paragraph::new(status_text)
        .style(status_style)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::BORDER))
                .title(Span::styled(" Status ", Style::default().fg(theme::TITLE))),
        );
    f.render_widget(header, rows.area(LayoutRow::Status));

    if let Some(area) = rows.get(LayoutRow::Marks) {
        render_marks_surface(f, app, area);
    }

    if let Some(area) = rows.get(LayoutRow::Panes) {
        render_pane_surface(f, app, area);
    }

    // View tabs
    let tab_titles = vec!["[1] Day", "[2] Week", "[3] All"];
    let selected_tab = match app.view_mode {
        ViewMode::Day => 0,
        ViewMode::Week => 1,
        ViewMode::All => 2,
    };
    let date_info = match app.view_mode {
        ViewMode::All => "All entries".to_string(),
        ViewMode::Day => app.selected_date.format("%A, %B %d, %Y").to_string(),
        ViewMode::Week => {
            let week_start = TimeData::week_start(app.selected_date);
            let week_end = week_start + Duration::days(6);
            format!(
                "{} - {}",
                week_start.format("%b %d"),
                week_end.format("%b %d, %Y")
            )
        }
    };
    let tabs = Tabs::new(tab_titles)
        .select(selected_tab)
        .style(Style::default().fg(theme::INACTIVE))
        .highlight_style(Style::default().fg(theme::ACCENT).bold())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::BORDER))
                .title(Span::styled(
                    format!(" {} | {} | {} ", app.view_mode.title(), date_info, app.sort_order.label()),
                    Style::default().fg(theme::HIGHLIGHT),
                )),
        );
    f.render_widget(tabs, rows.area(LayoutRow::Tabs));

    if let Some(area) = rows.get(LayoutRow::Search) {
        render_search_bar(f, app, area);
    }

    let content = rows.area(LayoutRow::Content);
    if app.input_mode == InputMode::AddingEntry || app.input_mode == InputMode::EditingEntry {
        render_entry_form(f, app, content);
    } else if app.view_mode == ViewMode::Week {
        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(25), Constraint::Min(40)])
            .split(content);
        render_weekly_breakdown(f, app, content_chunks[0]);
        render_entries_table(f, app, content_chunks[1]);
    } else {
        render_entries_table(f, app, content);
    }

    // Footer: left = hints (clips), right = "? : help" (always visible)
    let searching = app.is_searching() && !app.search_term.is_empty();
    let (total, total_label) = if searching || app.is_filtering() {
        (app.filtered_total(), "Filtered: ")
    } else {
        let t = match app.view_mode {
            ViewMode::All => app.data.today_total(),
            ViewMode::Day => app.data.total_for_date(app.selected_date),
            ViewMode::Week => {
                let week_start = TimeData::week_start(app.selected_date);
                app.data.total_for_week(week_start)
            }
        };
        (t, "Total: ")
    };

    let total_str = crate::duration::format(total);
    let footer = rows.area(LayoutRow::Footer);
    let footer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER));
    let footer_inner = footer_block.inner(footer);
    f.render_widget(footer_block, footer);

    // The right-hand zone is the part of the footer that never clips. `?: help`
    // has always lived there; the pane keys join it because with both panes
    // collapsed this legend is the surface's only trace on screen, so it has to
    // survive a narrow terminal too.
    // Hand-counted against the spans below; update it whenever they change, or the
    // legend loses its last characters to the clipping zone on its left.
    const KEYS_WIDTH: u16 = 24; // " | P/T/M | Tab | ?: help"
    let hints_width = footer_inner.width.saturating_sub(KEYS_WIDTH);
    let footer_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(hints_width), Constraint::Length(KEYS_WIDTH)])
        .split(footer_inner);

    let hint_spans = vec![
        Span::styled(format!(" {}", total_label), Style::default().fg(theme::TITLE)),
        Span::styled(total_str, Style::default().fg(theme::HIGHLIGHT).bold()),
        Span::styled(" | ", Style::default().fg(theme::BORDER)),
        // Detail goes first among the hints: this zone clips from the right at 80
        // columns, and `Enter` is the newest and least guessable binding here.
        Span::styled("Enter", Style::default().fg(theme::ACCENT)),
        Span::styled(": detail | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("t", Style::default().fg(theme::ACCENT)),
        Span::styled(": today | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("/", Style::default().fg(theme::ACCENT)),
        Span::styled(": search | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("a", Style::default().fg(theme::ACCENT)),
        Span::styled(": add | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("e", Style::default().fg(theme::ACCENT)),
        Span::styled(": edit | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("d", Style::default().fg(theme::ACCENT)),
        Span::styled(": del | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("s", Style::default().fg(theme::ACCENT)),
        Span::styled(": stop", Style::default().fg(theme::INACTIVE)),
    ];
    let hints = Paragraph::new(Line::from(hint_spans));
    f.render_widget(hints, footer_chunks[0]);

    // A surface's key is accented while that surface is open and dim while it is
    // hidden, so this legend says both that the surfaces exist and which are
    // showing. `M` joins `P`/`T` as the third `Shift`-letter toggle; `Tab` is
    // separated off after them because it cycles the panes only — the marks
    // surface is display-only and deliberately not on the focus ring.
    let key_style =
        |on: bool| Style::default().fg(if on { theme::ACCENT } else { theme::INACTIVE });
    let panes_open = !app.visible_panes().is_empty();
    let keys_hint = Paragraph::new(Line::from(vec![
        Span::styled(" | ", Style::default().fg(theme::BORDER)),
        Span::styled("P", key_style(app.show_projects)),
        Span::styled("/", Style::default().fg(theme::BORDER)),
        Span::styled("T", key_style(app.show_tags)),
        Span::styled("/", Style::default().fg(theme::BORDER)),
        Span::styled("M", key_style(app.show_marks)),
        Span::styled(" | ", Style::default().fg(theme::BORDER)),
        Span::styled("Tab", key_style(panes_open)),
        Span::styled(" | ", Style::default().fg(theme::BORDER)),
        Span::styled("?", Style::default().fg(theme::ACCENT)),
        Span::styled(": help", Style::default().fg(theme::INACTIVE)),
    ]));
    f.render_widget(keys_hint, footer_chunks[1]);

    match app.input_mode {
        InputMode::Help => render_help_popup(f),
        InputMode::Detail => render_detail_popup(f, app),
        _ => {}
    }
}

/// Geometry and chrome shared by every modal overlay, and the only place either
/// popover is positioned or framed.
///
/// The two popups are the same object with different content: a centred `Rect`
/// cleared out of the frame beneath it, a bordered block filled with the overlay
/// background, the subject on the left of the top border with a marker
/// right-aligned opposite it, and a line of key hints on the last row inside the
/// bottom border. Returns the content area between the two.
fn render_overlay(
    f: &mut Frame,
    width: u16,
    height: u16,
    title: Span<'_>,
    marker: Span<'_>,
    hints: Line<'_>,
) -> Rect {
    let area = f.area();
    let width = width.min(area.width.saturating_sub(4));
    let height = height.min(area.height.saturating_sub(2));
    let popup_area = Rect {
        x: (area.width.saturating_sub(width)) / 2,
        y: (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };

    // `Clear` first: without it the widgets underneath show through the gaps in
    // whatever the popup draws.
    f.render_widget(Clear, popup_area);
    let overlay_style = Style::default().bg(theme::OVERLAY_BG);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT))
        .title(title)
        .title_top(Line::from(marker).right_aligned())
        .style(overlay_style);
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);
    f.render_widget(Paragraph::new(hints).style(overlay_style), rows[1]);
    rows[0]
}

/// ` esc close `-style hints, spelled out rather than glyphed to match the pane
/// keys and the footer legend.
fn overlay_hints(pairs: &[(&'static str, &'static str)]) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (i, (k, what)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default().fg(theme::BORDER)));
        }
        spans.push(Span::styled(*k, Style::default().fg(theme::ACCENT).bold()));
        spans.push(Span::styled(
            format!(" {}", what),
            Style::default().fg(theme::INACTIVE),
        ));
    }
    Line::from(spans)
}

pub fn render_help_popup(f: &mut Frame) {
    fn key(k: &'static str) -> Span<'static> {
        Span::styled(k, Style::default().fg(theme::ACCENT).bold())
    }
    fn sep(s: &'static str) -> Span<'static> {
        Span::styled(s, Style::default().fg(theme::INACTIVE))
    }
    fn heading(s: &'static str) -> Line<'static> {
        Line::from(Span::styled(s, Style::default().fg(theme::HIGHLIGHT).bold()))
    }

    let lines: Vec<Line> = vec![
        heading("  Navigation"),
        Line::from(vec![key("  h / ←"), sep("  previous period")]),
        Line::from(vec![key("  l / →"), sep("  next period")]),
        Line::from(vec![key("  j / ↓"), sep("  select next entry")]),
        Line::from(vec![key("  k / ↑"), sep("  select previous entry")]),
        Line::from(vec![key("  t"), sep("        go to today")]),
        Line::from(vec![key("  1 / 2 / 3"), sep("  day / week / all view")]),
        Line::from(Span::raw("")),
        heading("  Entries"),
        Line::from(vec![key("  a"), sep("        add entry (uses browsed date)")]),
        Line::from(vec![key("  e"), sep("        edit selected entry")]),
        Line::from(vec![key("  d"), sep("        delete selected entry")]),
        Line::from(vec![key("  s"), sep("        stop active entry")]),
        Line::from(vec![key("  Enter"), sep("    entry detail")]),
        Line::from(Span::raw("")),
        heading("  Search & Filter"),
        // This section's key column is two wider than the rest so `Shift-Tab`,
        // the longest binding in the popup, still gets a gap before its text.
        Line::from(vec![key("  /"), sep("          search any field")]),
        Line::from(vec![key("  Shift-P"), sep("    Projects pane on / off")]),
        Line::from(vec![key("  Shift-T"), sep("    Tags pane on / off")]),
        Line::from(vec![key("  Tab"), sep("        focus table / panes")]),
        Line::from(vec![key("  Shift-Tab"), sep("  focus panes in reverse")]),
        Line::from(vec![key("  Enter"), sep("      filter on the pane value")]),
        Line::from(Span::raw("")),
        heading("  Other"),
        // Not under Search & Filter with the P/T panes: this surface filters
        // nothing and takes no focus, it only shows what `tt-safe` has open.
        Line::from(vec![key("  Shift-M"), sep("  open marks on / off")]),
        Line::from(vec![key("  o"), sep("        toggle sort order")]),
        Line::from(vec![key("  r"), sep("        reload data from disk")]),
        Line::from(vec![key("  ?"), sep("        toggle this help")]),
        Line::from(vec![key("  q / Esc"), sep("  quit")]),
    ];

    // Two borders, one hints row and one row per line — sized from the content so
    // adding a binding cannot silently push the last one out of the box.
    let content = render_overlay(
        f,
        52,
        lines.len() as u16 + 3,
        Span::styled(
            " Keybindings ",
            Style::default().fg(theme::HIGHLIGHT).bold(),
        ),
        Span::styled(" ? ", Style::default().fg(theme::INACTIVE)),
        overlay_hints(&[("esc", "close")]),
    );
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::OVERLAY_BG)),
        content,
    );
}

/// The selected entry, every field of it, wrapped rather than truncated — the
/// answer to a table cell that had to clip.
fn render_detail_popup(f: &mut Frame, app: &App) {
    let Some(entry) = app.selected_entry() else {
        return;
    };

    // Label column wide enough for the longest label, so the values line up and a
    // wrapped value hangs under its own first line rather than under the label.
    const LABEL: usize = 12;
    let width = 80u16.min(f.area().width.saturating_sub(4)).max(24) as usize;
    // 2 border columns, the leading two-space indent every line carries, the label
    // column, and a two-column right margin so a wrapped line never runs flush into
    // the border.
    let value_width = width.saturating_sub(2 + 2 + LABEL + 2).max(8);

    fn label(text: &str) -> Span<'_> {
        Span::styled(
            format!("  {:<width$}", text, width = LABEL),
            Style::default().fg(theme::TITLE),
        )
    }

    /// Greedy word wrap. Values are joined tag lists and prose, so the break
    /// points are spaces; a single word longer than the line is left long rather
    /// than cut, since cutting is the thing this popover exists to avoid.
    fn wrap(text: &str, width: usize) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        for word in text.split_whitespace() {
            match lines.last_mut() {
                Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                    line.push(' ');
                    line.push_str(word);
                }
                _ => lines.push(word.to_string()),
            }
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }

    /// One labelled field: the first line beside the label, the rest indented to
    /// the value column.
    fn field(name: &str, value: &str, style: Style, value_width: usize) -> Vec<Line<'static>> {
        wrap(value, value_width)
            .into_iter()
            .enumerate()
            .map(|(i, text)| {
                let head = if i == 0 {
                    format!("  {:<width$}", name, width = LABEL)
                } else {
                    " ".repeat(2 + LABEL)
                };
                Line::from(vec![
                    Span::styled(head, Style::default().fg(theme::TITLE)),
                    Span::styled(text, style),
                ])
            })
            .collect()
    }

    let value = Style::default().fg(Color::White);
    let tags = if entry.tags.is_empty() {
        "—".to_string()
    } else {
        entry.format_tags()
    };
    let mut lines: Vec<Line> = vec![Line::from(Span::raw(""))];
    lines.extend(field(
        "Project",
        entry.project.as_deref().unwrap_or("—"),
        Style::default().fg(theme::ACCENT),
        value_width,
    ));
    lines.extend(field(
        "Tags",
        &tags,
        Style::default().fg(theme::HIGHLIGHT),
        value_width,
    ));
    lines.extend(field("Date", &entry.format_date(), value, value_width));
    lines.extend(field(
        "Start",
        &entry.format_start_time(),
        value,
        value_width,
    ));
    // `format_end_time` is already the em dash while an entry runs, so an active
    // entry needs no special case here.
    lines.extend(field("End", &entry.format_end_time(), value, value_width));
    lines.extend(field(
        "Duration",
        &entry.format_duration(),
        Style::default().fg(theme::ACCENT).bold(),
        value_width,
    ));
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(label("Description")));
    // The description gets the full inner width rather than the value column: it
    // is prose, and it is the field most likely to need the room.
    for text in wrap(&entry.description, width.saturating_sub(2 + 2 + 2).max(8)) {
        lines.push(Line::from(vec![Span::raw("  "), Span::styled(text, value)]));
    }
    lines.push(Line::from(Span::raw("")));

    let (marker, marker_style) = if entry.is_active() {
        (" active ", Style::default().fg(theme::ACTIVE).bold())
    } else {
        (" logged ", Style::default().fg(theme::INACTIVE))
    };
    let content = render_overlay(
        f,
        width as u16,
        lines.len() as u16 + 3,
        Span::styled(
            format!(" Entry #{} ", entry.id),
            Style::default().fg(theme::HIGHLIGHT).bold(),
        ),
        Span::styled(marker, marker_style),
        // Every hint here is bound in the `InputMode::Detail` arm — a hint that does
        // nothing would be worse than none. Traversal comes first because it is what
        // keeps the list interactive from inside the overlay.
        overlay_hints(&[
            ("j/k", "move"),
            ("e", "edit"),
            ("d", "delete"),
            ("esc", "close"),
        ]),
    );
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::OVERLAY_BG)),
        content,
    );
}

/// The `Marks` surface: `tt-safe`'s open phase marks, newest first.
///
/// Rows are `tt-safe marks`' own format — `project/issue phase`, start time,
/// elapsed — assembled from `crate::marks`, which owns the row format as well as
/// the file format so the CLI and the TUI cannot disagree. Elapsed is *asked for*
/// per frame rather than stored, which is what makes the numbers count up between
/// directory reads.
fn render_marks_surface(f: &mut Frame, app: &App, area: Rect) {
    /// Narrowest the label column gets, so a box of short labels still lines its
    /// times up where the owner's approved shape puts them.
    const LABEL_WIDTH: usize = 18;

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .title(Span::styled(
            " Marks (M) ",
            Style::default().fg(theme::TITLE),
        ));
    let inner = block.inner(area);

    // The panes' own border count, on the panes' own mechanism, driven off the
    // rows this frame really has rather than a constant — so a box squeezed by a
    // short terminal still tells the truth about what is off screen.
    if let Some(count) = app.marks_count(inner.height as usize) {
        block = block.title_top(
            Line::from(Span::styled(
                format!(" {} ", count),
                Style::default().fg(theme::INACTIVE),
            ))
            .right_aligned(),
        );
    }

    let marks = app.visible_marks();
    // One label column for the whole box, so the start times and elapsed times
    // read as columns instead of trailing each label at its own indent.
    let label_width = marks
        .iter()
        .map(|mark| mark.label().chars().count())
        .max()
        .unwrap_or(0)
        .max(LABEL_WIDTH);

    let lines: Vec<Line> = if marks.is_empty() {
        vec![Line::from(Span::styled(
            " no open marks",
            Style::default().fg(theme::INACTIVE).italic(),
        ))]
    } else {
        marks
            .iter()
            .map(|mark| {
                let label = mark.label();
                let pad = " ".repeat(label_width.saturating_sub(label.chars().count()));
                Line::from(vec![
                    Span::styled(
                        format!(" {}{}", label, pad),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!(" {}", mark.started_at()),
                        Style::default().fg(theme::INACTIVE),
                    ),
                    Span::styled(
                        format!("   ({})", mark.elapsed()),
                        Style::default().fg(theme::HIGHLIGHT),
                    ),
                ])
            })
            .collect()
    };

    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// The pane surface: both panes side by side, or the single open one full width.
fn render_pane_surface(f: &mut Frame, app: &App, area: Rect) {
    let panes = app.visible_panes();
    let share = panes.len() as u32;
    let constraints: Vec<Constraint> = panes.iter().map(|_| Constraint::Ratio(1, share)).collect();
    let areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);
    for (pane, pane_area) in panes.iter().zip(areas.iter()) {
        render_pane(f, app, *pane, *pane_area);
    }
}

fn render_pane(f: &mut Frame, app: &App, pane: Pane, area: Rect) {
    let focused = app.focused_pane() == Some(pane);
    let values = app.pane_values(pane);

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused {
            theme::ACCENT
        } else {
            theme::BORDER
        }))
        .title(Span::styled(
            pane.title(),
            Style::default().fg(if focused {
                theme::HIGHLIGHT
            } else {
                theme::TITLE
            }),
        ));

    let inner = block.inner(area);

    // Scrolled lists say so on the top border, opposite the title: ` 2/8 `. The
    // top border is dead space, so this costs no row and no inner column — a
    // Scrollbar would have to steal one of the two, and a thumb only hints that
    // more exists where the count states how much.
    if let Some(indicator) = app.pane_scroll_indicator(pane, inner.height as usize) {
        block = block.title_top(
            Line::from(Span::styled(
                format!(" {} ", indicator),
                Style::default().fg(theme::INACTIVE),
            ))
            .right_aligned(),
        );
    }

    // The cursor marker is the entries table's, so the two lists read as one
    // system. It is a gutter the List reserves on *every* row, so it comes off the
    // width the rows lay themselves out in — otherwise the right-flushed counts
    // would be pushed off the end.
    let width = (inner.width as usize).saturating_sub(CURSOR_MARKER.len());
    let items: Vec<ListItem> = if values.is_empty() {
        vec![ListItem::new(Span::styled(
            " nothing in view",
            Style::default().fg(theme::INACTIVE).italic(),
        ))]
    } else {
        values
            .iter()
            .map(|(value, count)| {
                // Value left, match count flushed right within the block. The lead
                // column the value already sat in carries the selection mark, so
                // marking one costs no width and shifts nothing.
                let selected = app.pane_value_is_selected(pane, value);
                let mark = if selected { "•" } else { " " };
                let count = count.to_string();
                let used = 1 + value.chars().count() + count.chars().count() + 1;
                let gap = width.saturating_sub(used).max(1);
                let value_style = if selected {
                    Style::default().fg(theme::ACCENT).bold()
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{}{}", mark, value), value_style),
                    Span::raw(" ".repeat(gap)),
                    Span::styled(count, Style::default().fg(theme::HIGHLIGHT)),
                ]))
            })
            .collect()
    };

    // Shown whether or not the pane has focus: the border accent is what says
    // "keys land here", while the marker says where this pane's cursor will resume,
    // which is worth knowing from the other side of the surface. The entries table
    // shows its own marker unconditionally for the same reason.
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().bg(theme::SELECTED_BG))
        .highlight_symbol(CURSOR_MARKER);
    let mut state = ListState::default();
    if !values.is_empty() {
        state.select(Some(app.pane_cursor(pane)));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn render_search_bar(f: &mut Frame, app: &App, area: Rect) {
    let is_active = app.input_mode == InputMode::Searching;
    let border_style = if is_active {
        Style::default().fg(theme::ACCENT)
    } else {
        Style::default().fg(theme::BORDER)
    };

    let match_count = app.filtered_entries().len();
    let match_info = if app.search_term.is_empty() {
        String::new()
    } else {
        format!(" ({} matches)", match_count)
    };

    let search_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(
            format!(" Search{} ", match_info),
            if is_active {
                Style::default().fg(theme::HIGHLIGHT)
            } else {
                Style::default().fg(theme::TITLE)
            },
        ));

    let search_text = if is_active && app.search_term.is_empty() {
        "Type to search... (Enter to confirm, Esc to clear)"
    } else {
        &app.search_term
    };

    let search_input = Paragraph::new(search_text)
        .style(if app.search_term.is_empty() && is_active {
            Style::default().fg(theme::INACTIVE).italic()
        } else {
            Style::default().fg(Color::White)
        })
        .block(search_block);
    f.render_widget(search_input, area);

    if is_active {
        let byte_idx = app.search_term.char_indices().nth(app.cursor_pos).map(|(i, _)| i).unwrap_or(app.search_term.len());
        f.set_cursor_position((
            area.x + Line::from(&app.search_term[..byte_idx]).width() as u16 + 1,
            area.y + 1,
        ));
    }
}

fn render_entry_form(f: &mut Frame, app: &App, area: Rect) {
    let is_editing = app.input_mode == InputMode::EditingEntry;
    let form_title = if is_editing {
        " Edit Entry ".to_string()
    } else {
        let today = Local::now().date_naive();
        if app.selected_date == today {
            " Add Log Entry ".to_string()
        } else {
            format!(" Add Log Entry — {} ", app.selected_date.format("%a, %d %b %Y"))
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3), // Description
            Constraint::Length(3), // Project
            Constraint::Length(3), // Tags
            Constraint::Length(3), // Duration
            Constraint::Length(3), // Start Time
            Constraint::Length(3), // End Time
            Constraint::Length(3), // Help
            Constraint::Min(0),
        ])
        .split(area);

    fn field_block(label: &'static str, active: bool) -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .border_style(if active {
                Style::default().fg(theme::ACCENT)
            } else {
                Style::default().fg(theme::INACTIVE)
            })
            .title(Span::styled(
                label,
                if active {
                    Style::default().fg(theme::HIGHLIGHT)
                } else {
                    Style::default().fg(theme::TITLE)
                },
            ))
    }

    let active = app.input_field;

    f.render_widget(
        Paragraph::new(app.input_description.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(" Description ", active == InputField::Description)),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(app.input_project.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(
                " Project (optional: single name, e.g. acme) ",
                active == InputField::Project,
            )),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(app.input_tags.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(" Tags (space-separated, e.g., work meeting) ", active == InputField::Tags)),
        chunks[2],
    );
    f.render_widget(
        Paragraph::new(app.input_duration.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(" Duration (optional: 1h30m, 45m, 2h) ", active == InputField::Duration)),
        chunks[3],
    );
    f.render_widget(
        Paragraph::new(app.input_start_time.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(" Start Time (e.g. 9am, 14:30, 25/03 9.30am) ", active == InputField::StartTime)),
        chunks[4],
    );
    f.render_widget(
        Paragraph::new(app.input_end_time.as_str())
            .style(Style::default().fg(Color::White))
            .block(field_block(" End Time (optional: e.g. 9am, 14:30, 25/03 9.30am) ", active == InputField::EndTime)),
        chunks[5],
    );

    let help = Paragraph::new(Line::from(vec![
        Span::styled("Tab", Style::default().fg(theme::ACCENT)),
        Span::styled(": switch field | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("Enter", Style::default().fg(theme::ACCENT)),
        Span::styled(": save | ", Style::default().fg(theme::INACTIVE)),
        Span::styled("Esc", Style::default().fg(theme::ACCENT)),
        Span::styled(": cancel  ", Style::default().fg(theme::INACTIVE)),
        Span::styled("Need ≥2 of: Start, End, Duration", Style::default().fg(theme::BORDER)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER))
            .title(Span::styled(form_title, Style::default().fg(theme::HIGHLIGHT))),
    );
    f.render_widget(help, chunks[6]);

    // Cursor: use cursor_pos for correct mid-text cursor placement
    let cursor_text_width = |text: &str, pos: usize| -> u16 {
        let byte_idx = text.char_indices().nth(pos).map(|(i, _)| i).unwrap_or(text.len());
        Line::from(&text[..byte_idx]).width() as u16
    };
    let (cursor_x, cursor_y) = match app.input_field {
        InputField::Description => (
            chunks[0].x + cursor_text_width(&app.input_description, app.cursor_pos) + 1,
            chunks[0].y + 1,
        ),
        InputField::Project => (
            chunks[1].x + cursor_text_width(&app.input_project, app.cursor_pos) + 1,
            chunks[1].y + 1,
        ),
        InputField::Tags => (
            chunks[2].x + cursor_text_width(&app.input_tags, app.cursor_pos) + 1,
            chunks[2].y + 1,
        ),
        InputField::Duration => (
            chunks[3].x + cursor_text_width(&app.input_duration, app.cursor_pos) + 1,
            chunks[3].y + 1,
        ),
        InputField::StartTime => (
            chunks[4].x + cursor_text_width(&app.input_start_time, app.cursor_pos) + 1,
            chunks[4].y + 1,
        ),
        InputField::EndTime => (
            chunks[5].x + cursor_text_width(&app.input_end_time, app.cursor_pos) + 1,
            chunks[5].y + 1,
        ),
    };
    f.set_cursor_position((cursor_x, cursor_y));
}

fn render_weekly_breakdown(f: &mut Frame, app: &App, area: Rect) {
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

            let dur_color = if hours >= 8 {
                theme::DURATION_HIGH
            } else if hours >= 4 {
                theme::DURATION_MED
            } else {
                theme::DURATION_LOW
            };

            let (day_style, date_style) = if is_today {
                (
                    Style::default().fg(theme::HIGHLIGHT).bold(),
                    Style::default().fg(theme::HIGHLIGHT),
                )
            } else {
                (
                    Style::default().fg(theme::ACCENT),
                    Style::default().fg(theme::TITLE),
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
            .border_style(Style::default().fg(theme::BORDER))
            .title(Span::styled(" Daily Totals ", Style::default().fg(theme::TITLE))),
    );
    f.render_widget(table, area);
}

fn entry_row(entry: &crate::tracker::TimeEntry, stripe: bool) -> Row<'_> {
    let hours = entry.duration().num_hours();
    let dur_color = if hours >= 4 {
        theme::DURATION_HIGH
    } else if hours >= 2 {
        theme::DURATION_MED
    } else {
        theme::DURATION_LOW
    };

    let status_style = if entry.is_active() {
        Style::default().fg(theme::ACTIVE)
    } else {
        Style::default().fg(theme::INACTIVE)
    };

    let row_style = if stripe {
        Style::default().bg(Color::Rgb(35, 35, 35))
    } else {
        Style::default()
    };

    Row::new(vec![
        Cell::from(entry.format_date()).style(Style::default().fg(theme::TITLE)),
        Cell::from(entry.format_start_time()).style(Style::default().fg(theme::ACCENT)),
        Cell::from(entry.format_end_time()).style(Style::default().fg(theme::INACTIVE)),
        Cell::from(entry.description.clone()),
        Cell::from(entry.format_tags()).style(Style::default().fg(theme::HIGHLIGHT)),
        Cell::from(entry.format_duration()).style(Style::default().fg(dur_color)),
        Cell::from(entry.status_icon()).style(status_style),
    ])
    .style(row_style)
}

fn day_header_row(date: NaiveDate, total: Duration) -> Row<'static> {
    let weekday = format!("\n{}", date.format("%A"));
    let date_str = format!("\n{}", date.format("%B %d, %Y"));
    let total_str = format!("\n{}", crate::duration::format(total));

    Row::new(vec![
        Cell::from(weekday).style(
            Style::default()
                .fg(theme::HIGHLIGHT)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(""),
        Cell::from(""),
        Cell::from(date_str).style(Style::default().fg(theme::TITLE)),
        Cell::from(""),
        Cell::from(total_str).style(
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(""),
    ])
    .height(2)
    .style(Style::default().bg(theme::DAY_HEADER_BG))
}

fn render_entries_table(f: &mut Frame, app: &mut App, area: Rect) {
    let header_cells = ["Date", "Start", "End", "Description", "Tags", "Duration", ""]
        .into_iter()
        .map(|h| {
            Cell::from(h).style(
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            )
        });
    let header_row = Row::new(header_cells)
        .height(1)
        .style(Style::default().bg(theme::HEADER_BG));

    let entries = app.filtered_entries();

    let (rows, visual_selected): (Vec<Row>, Option<usize>) =
        if app.view_mode == ViewMode::Week {
            let mut day_totals: HashMap<NaiveDate, Duration> = HashMap::new();
            for entry in &entries {
                let date = entry.start_time.date_naive();
                *day_totals.entry(date).or_insert_with(Duration::zero) += entry.duration();
            }

            let mut rows: Vec<Row> = Vec::new();
            let mut visual_idx_map: Vec<usize> = Vec::with_capacity(entries.len());
            let mut current_date: Option<NaiveDate> = None;
            let mut stripe = false;

            for entry in entries.iter() {
                let entry_date = entry.start_time.date_naive();
                if current_date != Some(entry_date) {
                    current_date = Some(entry_date);
                    stripe = false;
                    let total = day_totals
                        .get(&entry_date)
                        .copied()
                        .unwrap_or_else(Duration::zero);
                    rows.push(day_header_row(entry_date, total));
                }
                visual_idx_map.push(rows.len());
                rows.push(entry_row(entry, stripe));
                stripe = !stripe;
            }

            let visual_sel = app
                .table_state
                .selected()
                .and_then(|idx| visual_idx_map.get(idx).copied());
            (rows, visual_sel)
        } else {
            let rows = entries
                .iter()
                .enumerate()
                .map(|(i, entry)| entry_row(entry, i % 2 != 0))
                .collect();
            (rows, app.table_state.selected())
        };

    // Projects read as `(tt)` and tags as `#impl`, the same sigils the CLI listing
    // uses, so the title says which pane a value came from without a legend.
    let title = if app.is_filtering() {
        let values: Vec<String> = app
            .selected_projects
            .iter()
            .map(|p| format!("({})", p))
            .chain(app.selected_tags.iter().map(|t| format!("#{}", t)))
            .collect();
        format!(" Entries [filtered: {}] ", values.join(" "))
    } else {
        " Entries ".to_string()
    };

    // Tags used to be `Length(14)`, so the cell clipped at the same 14 columns on a
    // 30-inch screen as on a half-open laptop. `Fill(1)` beside the description's
    // `Min(12)` splits whatever is left between the only two columns whose content
    // has no fixed width, so both grow with the terminal — the full tag list of the
    // widest row shows from ~146 columns.
    //
    // The fixed columns are cut to exactly what they render (`2026-08-17`, `14:30`,
    // and the `Duration` header, which is wider than any duration) instead of
    // carrying two or three columns of padding each: `column_spacing` already puts
    // a gap between cells, and those 7 reclaimed columns are what makes the
    // 80-column case gain on *both* variable columns (tags 14 -> 19, description
    // 17 -> 19) rather than trading one for the other.
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
            .border_style(Style::default().fg(theme::BORDER))
            .title(Span::styled(title, Style::default().fg(theme::TITLE))),
    )
    .row_highlight_style(Style::default().bg(theme::SELECTED_BG))
    .highlight_symbol(CURSOR_MARKER);

    let mut render_state = TableState::default().with_selected(visual_selected);
    f.render_stateful_widget(table, area, &mut render_state);
}
