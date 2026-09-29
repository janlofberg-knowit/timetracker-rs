//! What the Entries box and the heat box share: the view selector line and
//! the `<kind>: <view> | <period> | <tail>` title.

use crate::tracker::TimeData;
use crate::tui::types::ViewMode;
use crate::tui::{App, theme};
use chrono::{Datelike, Duration};
use ratatui::{prelude::*, widgets::Tabs};

pub(super) fn render_view_selector(f: &mut Frame, app: &App, area: Rect) {
    let selected = match app.view_mode {
        ViewMode::Day => 0,
        ViewMode::Week => 1,
        ViewMode::Month => 2,
        ViewMode::Year => 3,
        ViewMode::All => 4,
    };
    let tabs = Tabs::new(vec![
        "[1] Day",
        "[2] Week",
        "[3] Month",
        "[4] Year",
        "[5] All",
    ])
    .select(selected)
    .style(Style::default().fg(theme::inactive()))
    .highlight_style(Style::default().fg(theme::accent()).bold());
    f.render_widget(tabs, area);
}

pub(super) fn period_label(app: &App) -> String {
    match app.view_mode {
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
        ViewMode::Month => app.selected_date.format("%B %Y").to_string(),
        ViewMode::Year => format!("Year {}", app.selected_date.year()),
    }
}

/// The period segment is left out for `ViewMode::All`, whose title says it.
pub(super) fn content_title(app: &App, kind: &str, tail: &str) -> String {
    let view = app.view_mode.title();
    if app.view_mode == ViewMode::All {
        format!(" {kind}: {view} | {tail} ")
    } else {
        format!(" {kind}: {view} | {} | {tail} ", period_label(app))
    }
}
