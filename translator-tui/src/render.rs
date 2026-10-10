//! Draws one `AppView` frame with ratatui.

use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;
use translator_app::{AppView, LaneView, SettingsRowView};
use translator_core::controller::Phase;
use translator_core::keys::KeyboardMode;
use translator_core::lane::Lane;

const MIN_WIDTH: u16 = 40;
const MIN_HEIGHT: u16 = 12;
const BAR_GLYPHS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const SETTINGS_WIDTH: u16 = 36;
const SETTINGS_HINT: &str = "↑/↓ choose  ←/→ change  Esc close";

/// `is_truecolor`: the terminal takes 24-bit colour (`COLORTERM`).
pub fn draw(frame: &mut Frame, view: &AppView, is_truecolor: bool) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let hint = Paragraph::new(format!("Terminal too small (min {MIN_WIDTH}x{MIN_HEIGHT})"))
            .wrap(Wrap { trim: true });
        frame.render_widget(hint, area);
        return;
    }
    let accent = accent_color(view.theme_rgb, is_truecolor);
    let [lanes_area, levels_area, transcript_area, status_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .areas(area);
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .areas(lanes_area);
    for (area, lane) in [left, right].into_iter().zip(&view.lanes) {
        draw_lane(frame, area, lane, view.show_active_marker, accent);
    }
    frame.render_widget(
        Paragraph::new(level_bars(&view.levels, levels_area.width)).style(Style::new().fg(accent)),
        levels_area,
    );
    draw_transcript(frame, transcript_area, view, accent);
    frame.render_widget(status_line(view, accent, status_area.width), status_area);
    if view.is_settings_open {
        draw_settings(frame, area, &view.settings_rows, accent);
    }
}

/// The theme colour as RGB, or the closest named / 256-colour value for
/// terminals without 24-bit colour (where RGB would be dropped and the
/// recording lane's black text would sit on the default background).
fn accent_color((red, green, blue): (u8, u8, u8), is_truecolor: bool) -> Color {
    if is_truecolor {
        return Color::Rgb(red, green, blue);
    }
    match (red, green, blue) {
        (0xff, 0x44, 0x44) => Color::LightRed,
        (0xff, 0xeb, 0x3b) => Color::Yellow,
        (0x21, 0x96, 0xf3) => Color::LightBlue,
        (0x4c, 0xaf, 0x50) => Color::Green,
        (0xff, 0xa5, 0x00) => Color::Indexed(214),
        _ => Color::White,
    }
}

fn draw_lane(
    frame: &mut Frame,
    area: Rect,
    lane: &LaneView,
    show_active_marker: bool,
    accent: Color,
) {
    let marker = if show_active_marker && lane.is_active {
        "▶ "
    } else {
        "  "
    };
    let badge = if lane.is_recording {
        "● REC"
    } else {
        "◀ ▶"
    };
    let style = if lane.is_recording {
        Style::new()
            .fg(Color::Black)
            .bg(accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(accent)
    };
    let block = Block::bordered()
        .title(format!(" PERSON {} ", lane.label))
        .border_style(Style::new().fg(accent));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [name_area, badge_area] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(6)]).areas(inner);
    frame.render_widget(
        Paragraph::new(format!("{marker}{}", lane.language))
            .style(style.add_modifier(Modifier::BOLD)),
        name_area,
    );
    frame.render_widget(
        Paragraph::new(badge)
            .style(style)
            .alignment(Alignment::Right),
        badge_area,
    );
}

/// The newest `width` levels as bar glyphs.
fn level_bars(levels: &[f32], width: u16) -> String {
    let skip = levels.len().saturating_sub(usize::from(width));
    levels[skip..]
        .iter()
        .map(|level| {
            let index = (level.clamp(0.0, 1.0) * (BAR_GLYPHS.len() - 1) as f32).round() as usize;
            BAR_GLYPHS[index]
        })
        .collect()
}

fn draw_transcript(frame: &mut Frame, area: Rect, view: &AppView, accent: Color) {
    let [source_area, target_area] =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);
    let border = Style::new().fg(accent);
    let source_block = Block::bordered()
        .title(format!(" {} ", view.source_label))
        .border_style(border);
    frame.render_widget(
        scrolled_to_end(
            Paragraph::new(view.transcription.as_str()),
            source_block,
            source_area,
        ),
        source_area,
    );
    let alignment = if view.is_translation_right_to_left {
        Alignment::Right
    } else {
        Alignment::Left
    };
    let target_block = Block::bordered()
        .title(format!(" {} ", view.target_label))
        .border_style(border);
    frame.render_widget(
        scrolled_to_end(
            Paragraph::new(view.translation.as_str()).alignment(alignment),
            target_block,
            target_area,
        ),
        target_area,
    );
}

/// Wraps `paragraph` inside `block` and scrolls it so its last wrapped line
/// is visible: streamed text grows at the end.
fn scrolled_to_end<'a>(paragraph: Paragraph<'a>, block: Block<'a>, area: Rect) -> Paragraph<'a> {
    let inner = block.inner(area);
    let paragraph = paragraph.wrap(Wrap { trim: true });
    let hidden_lines = paragraph
        .line_count(inner.width)
        .saturating_sub(usize::from(inner.height));
    paragraph
        .scroll((u16::try_from(hidden_lines).unwrap_or(u16::MAX), 0))
        .block(block)
}

/// Phase or error, then any audio warning, then the key hints; the hints are
/// left out when all three do not fit in `width`, so a narrow terminal never
/// cuts the warning for them.
fn status_line(view: &AppView, accent: Color, width: u16) -> Paragraph<'static> {
    let mut line = Line::from(Span::styled(
        view.status.clone(),
        Style::new().fg(accent).add_modifier(Modifier::BOLD),
    ));
    if let Some(warning) = &view.audio_warning {
        line.push_span(Span::raw(" · "));
        line.push_span(Span::styled(
            warning.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        ));
    }
    let hint = Span::raw(format!(" · {}", status_hint(view)));
    if line.width() + hint.width() <= usize::from(width) {
        line.push_span(hint);
    }
    Paragraph::new(line)
}

/// Key hints for the current mode and phase; the idle landscape line fits 80
/// columns together with the status word.
fn status_hint(view: &AppView) -> &'static str {
    if view.is_settings_open {
        return SETTINGS_HINT;
    }
    match (view.phase, view.keyboard_mode) {
        (Phase::Recording(Lane::Two), KeyboardMode::Vertical) => "X again to stop",
        (Phase::Recording(_), _) => "Z again to stop",
        (_, KeyboardMode::Landscape) => {
            "Z tap to record  Space lane  ←/→ language  Esc settings  Ctrl-C quit"
        }
        (_, KeyboardMode::Vertical) => {
            "Z/X tap to record  ←/→ −/+ language  Esc settings  Ctrl-C quit"
        }
    }
}

fn draw_settings(frame: &mut Frame, area: Rect, rows: &[SettingsRowView], accent: Color) {
    let height = rows.len() as u16 + 3;
    let [popup] = Layout::horizontal([Constraint::Length(SETTINGS_WIDTH)])
        .flex(Flex::Center)
        .areas(area);
    let [popup] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(popup);
    let lines: Vec<Line> = rows
        .iter()
        .map(|row| {
            let text = format!(" {:<16}{:>16} ", row.label, row.value);
            let style = if row.is_selected {
                Style::new().fg(Color::Black).bg(accent)
            } else {
                Style::new().fg(accent)
            };
            Line::styled(text, style)
        })
        .chain([Line::raw(format!(" {SETTINGS_HINT}"))])
        .collect();
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(" SETTINGS ")
                .border_style(Style::new().fg(accent)),
        ),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use translator_core::config::ThemeColor;
    use translator_core::controller::Phase;
    use translator_core::keys::KeyboardMode;
    use translator_core::lane::Lane;

    use super::*;

    fn view() -> AppView {
        AppView {
            theme_rgb: ThemeColor::Orange.rgb(),
            lanes: [
                LaneView {
                    label: "1".to_string(),
                    language: "ARABIC".to_string(),
                    is_active: true,
                    is_recording: false,
                },
                LaneView {
                    label: "2".to_string(),
                    language: "CHINESE".to_string(),
                    is_active: false,
                    is_recording: false,
                },
            ],
            keyboard_mode: KeyboardMode::Landscape,
            phase: Phase::Idle,
            show_active_marker: true,
            source_label: "Arabic (Source)".to_string(),
            transcription: "مرحبا".to_string(),
            target_label: "Chinese (Translation)".to_string(),
            translation: "早上好，你好吗？".to_string(),
            is_translation_right_to_left: false,
            status: "READY".to_string(),
            audio_warning: None,
            levels: vec![0.0, 0.5, 1.0, 0.25],
            is_settings_open: false,
            settings_rows: vec![
                SettingsRowView {
                    label: "KEYBOARD".to_string(),
                    value: "LANDSCAPE".to_string(),
                    is_selected: true,
                },
                SettingsRowView {
                    label: "VOLUME".to_string(),
                    value: "75%".to_string(),
                    is_selected: false,
                },
            ],
        }
    }

    /// Draws at the given size and returns each row's text. A wide (CJK)
    /// character's continuation cell holds a space, so callers strip spaces
    /// before searching for CJK text; Arabic letters are one cell wide.
    fn rows(view: &AppView, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, view, true)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    fn screen(view: &AppView, width: u16, height: u16) -> String {
        rows(view, width, height).join("\n")
    }

    #[test]
    fn main_screen_shows_lanes_transcript_and_status() {
        let text = screen(&view(), 80, 24);
        for expected in [
            "PERSON 1",
            "▶ ARABIC",
            "PERSON 2",
            "CHINESE",
            "Arabic (Source)",
            "READY",
            "Ctrl-C quit",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
    }

    #[test]
    fn cjk_translation_is_drawn_in_full() {
        let compact: String = screen(&view(), 80, 24)
            .chars()
            .filter(|c| *c != ' ')
            .collect();
        assert!(compact.contains("早上好，你好吗？"), "{compact}");
    }

    #[test]
    fn recording_shows_rec_and_the_stop_hint() {
        let mut recording = view();
        recording.phase = Phase::Recording(Lane::One);
        recording.lanes[0].is_recording = true;
        recording.status = "RECORDING".to_string();
        let text = screen(&recording, 80, 24);
        assert!(text.contains("● REC"), "{text}");
        assert!(text.contains("Z again to stop"), "{text}");
    }

    #[test]
    fn vertical_mode_names_x_for_lane_two_and_marks_no_lane() {
        let mut recording = view();
        recording.keyboard_mode = KeyboardMode::Vertical;
        recording.show_active_marker = false;
        recording.phase = Phase::Recording(Lane::Two);
        recording.lanes[1].is_recording = true;
        let text = screen(&recording, 80, 24);
        assert!(text.contains("X again to stop"), "{text}");
        assert!(
            !text.contains("▶ ARABIC") && !text.contains("▶ CHINESE"),
            "{text}"
        );
    }

    #[test]
    fn audio_warning_is_shown_in_the_status_line() {
        let mut silent = view();
        silent.audio_warning = Some("no microphone: recording silence".to_string());
        let text = screen(&silent, 80, 24);
        assert!(text.contains("no microphone: recording silence"), "{text}");
    }

    /// A translation that wraps to far more lines than its box holds and
    /// ends with a word found nowhere else.
    fn long_translation(sentence_count: usize) -> AppView {
        let mut long = view();
        let sentence = "one two three four five six seven eight nine ten eleven twelve. ";
        long.translation = format!("{}newestword", sentence.repeat(sentence_count));
        long
    }

    #[test]
    fn long_translation_scrolls_to_its_newest_words_at_minimum_size() {
        let text = screen(&long_translation(5), MIN_WIDTH, MIN_HEIGHT);
        assert!(text.contains("newestword"), "{text}");
    }

    #[test]
    fn long_translation_scrolls_to_its_newest_words_at_80x24() {
        let text = screen(&long_translation(20), 80, 24);
        assert!(text.contains("newestword"), "{text}");
    }

    #[test]
    fn audio_warning_wins_over_the_hints_at_minimum_width() {
        let mut silent = view();
        silent.audio_warning = Some("no microphone: recording silence".to_string());
        let text = screen(&silent, MIN_WIDTH, MIN_HEIGHT);
        assert!(text.contains("no microphone: recording silence"), "{text}");
        silent.audio_warning = Some("no mic".to_string());
        let rows = rows(&silent, MIN_WIDTH, MIN_HEIGHT);
        let status = rows.last().expect("status row");
        assert!(status.contains("READY · no mic"), "{status:?}");
        assert!(!status.contains("Z tap"), "{status:?}");
        assert!(!status.trim_end().ends_with('·'), "{status:?}");
    }

    #[test]
    fn settings_popup_lists_rows() {
        let mut settings = view();
        settings.is_settings_open = true;
        let text = screen(&settings, 80, 24);
        assert!(text.contains("SETTINGS"), "{text}");
        assert!(text.contains("KEYBOARD"), "{text}");
        assert!(text.contains("75%"), "{text}");
        assert!(text.contains("Esc close"), "{text}");
    }

    #[test]
    fn minimum_size_still_draws_the_layout() {
        let text = screen(&view(), MIN_WIDTH, MIN_HEIGHT);
        assert!(text.contains("PERSON 1"), "{text}");
        assert!(text.contains("READY"), "{text}");
    }

    #[test]
    fn tiny_terminals_show_the_hint_without_panicking() {
        let text = screen(&view(), MIN_WIDTH - 1, MIN_HEIGHT);
        assert!(text.contains("too small"), "{text}");
        for (width, height) in [(1, 1), (5, 2), (39, 30), (200, 11)] {
            rows(&view(), width, height);
        }
    }

    #[test]
    fn right_to_left_translation_is_right_aligned() {
        let mut arabic = view();
        arabic.transcription = "hello".to_string();
        arabic.translation = "مرحبا".to_string();
        arabic.is_translation_right_to_left = true;
        let rows = rows(&arabic, 80, 24);
        let row = rows
            .iter()
            .find(|row| row.contains("مرحبا"))
            .expect("translation row");
        let inner = row.trim_end_matches('│').trim_end();
        assert!(inner.ends_with("مرحبا"), "{row:?}");
    }

    #[test]
    fn accent_falls_back_to_named_colours_without_truecolor() {
        assert_eq!(
            accent_color(ThemeColor::Orange.rgb(), true),
            Color::Rgb(0xff, 0xa5, 0x00)
        );
        assert_eq!(
            accent_color(ThemeColor::Orange.rgb(), false),
            Color::Indexed(214)
        );
        assert_eq!(
            accent_color(ThemeColor::Blue.rgb(), false),
            Color::LightBlue
        );
        assert_eq!(accent_color(ThemeColor::Red.rgb(), false), Color::LightRed);
        assert_eq!(accent_color(ThemeColor::Green.rgb(), false), Color::Green);
        assert_eq!(accent_color(ThemeColor::Yellow.rgb(), false), Color::Yellow);
        assert_eq!(accent_color(ThemeColor::White.rgb(), false), Color::White);
        // A theme colour missing from the fallback table would render white.
        for color in ThemeColor::ALL {
            if color != ThemeColor::White {
                assert_ne!(accent_color(color.rgb(), false), Color::White, "{color:?}");
            }
        }
    }
}
