use ratatui::style::Color;

pub const ACCENT: Color = Color::Rgb(138, 180, 248);        // Light blue
pub const ACTIVE: Color = Color::Rgb(129, 199, 132);        // Green
pub const INACTIVE: Color = Color::Rgb(144, 144, 144);      // Gray
pub const HEADER_BG: Color = Color::Rgb(48, 48, 48);        // Dark gray
pub const SELECTED_BG: Color = Color::Rgb(66, 66, 66);      // Medium gray
pub const HIGHLIGHT: Color = Color::Rgb(255, 213, 79);      // Yellow/gold
pub const DURATION_HIGH: Color = Color::Rgb(239, 154, 154); // Light red
pub const DURATION_MED: Color = Color::Rgb(255, 224, 130);  // Light yellow
pub const DURATION_LOW: Color = Color::Rgb(165, 214, 167);  // Light green
pub const BORDER: Color = Color::Rgb(88, 88, 88);           // Border gray
pub const TITLE: Color = Color::Rgb(186, 186, 186);         // Light gray
pub const DAY_HEADER_BG: Color = Color::Rgb(38, 48, 68);    // Dark blue for day separators

/// Thresholds (in hours) for coloring a single time entry's duration.
pub const ENTRY_DURATION_HIGH_H: i64 = 4;
pub const ENTRY_DURATION_MED_H: i64 = 2;

/// Thresholds (in hours) for coloring a day's total tracked duration.
pub const DAY_DURATION_HIGH_H: i64 = 8;
pub const DAY_DURATION_MED_H: i64 = 4;

/// Maps a duration (in hours) to a color given high/medium thresholds.
pub fn duration_color(hours: i64, high_threshold: i64, med_threshold: i64) -> Color {
    if hours >= high_threshold {
        DURATION_HIGH
    } else if hours >= med_threshold {
        DURATION_MED
    } else {
        DURATION_LOW
    }
}
