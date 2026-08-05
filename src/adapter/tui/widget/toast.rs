use std::time::{Duration, Instant};

use getset::Getters;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use typed_builder::TypedBuilder;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// How long a toast stays on screen before it auto-dismisses.
pub const TOAST_DURATION: Duration = Duration::from_secs(2);
/// Confirmation shown when a selection is copied to the clipboard.
pub const TOAST_COPIED: &str = "Copied to clipboard";

/// Border and glyph color for a success toast.
const TOAST_SUCCESS_COLOR: Color = Color::Green;
/// Border and glyph color for an error toast.
const TOAST_ERROR_COLOR: Color = Color::Red;
/// Glyph preceding a success toast's message.
const SUCCESS_GLYPH: &str = "\u{2713} ";
/// Close affordance drawn on the box's top-right border.
const CLOSE_GLYPH: &str = "\u{2715}";
/// Cells of horizontal breathing room inside the toast border.
const HORIZONTAL_PADDING: u16 = 2;
/// Border rows and columns consumed on each axis.
const BORDERS: u16 = 2;
/// One content row inside the top and bottom borders.
const CONTENT_HEIGHT: u16 = 1;
/// Full height of one stacked toast box.
const BOX_HEIGHT: u16 = CONTENT_HEIGHT + BORDERS;
/// Widest a toast box may grow; longer messages wrap onto more lines.
const MAX_WIDTH: u16 = 44;
/// Inset from the bottom-right corner of the pane area.
const MARGIN: u16 = 1;

/// Visual priority for a toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastTone {
    /// A completed action worth confirming (e.g. a clipboard copy).
    Success,
    /// A failure or unavailable action that needs attention.
    Error,
}

impl ToastTone {
    /// Border and glyph color for this tone.
    fn color(self) -> Color {
        match self {
            Self::Success => TOAST_SUCCESS_COLOR,
            Self::Error => TOAST_ERROR_COLOR,
        }
    }

    /// Glyph shown before the message for this tone, when it carries one. Errors
    /// carry none: the red border and text already read as a failure.
    fn glyph(self) -> Option<&'static str> {
        match self {
            Self::Success => Some(SUCCESS_GLYPH),
            Self::Error => None,
        }
    }
}

/// A transient, auto-dismissing feedback box.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Toast {
    message: String,
    expires_at: Instant,
    tone: ToastTone,
}

/// The box `message` occupies, pinned to the bottom-right of `area` and lifted
/// `bottom_offset` rows, or `None` when there is no room. Lets a caller hit-test
/// clicks against the same box `render` draws.
pub fn region(area: Rect, message: &str, tone: ToastTone, bottom_offset: u16) -> Option<Rect> {
    geometry(area, message, tone, bottom_offset).map(|(rect, _)| rect)
}

/// The box rectangle and its wrapped lines, or `None` when `area` has no room at
/// `bottom_offset`.
fn geometry(
    area: Rect,
    message: &str,
    tone: ToastTone,
    bottom_offset: u16,
) -> Option<(Rect, Vec<String>)> {
    let available = area
        .height
        .saturating_sub(MARGIN)
        .saturating_sub(bottom_offset);
    if area.width <= MARGIN || available < BOX_HEIGHT {
        return None;
    }
    let label = match tone.glyph() {
        Some(glyph) => format!("{glyph}{message}"),
        None => message.to_string(),
    };
    let width = (UnicodeWidthStr::width(label.as_str()) as u16)
        .saturating_add(HORIZONTAL_PADDING)
        .saturating_add(BORDERS)
        .min(MAX_WIDTH)
        .min(area.width.saturating_sub(MARGIN));
    // Without room for both borders and at least one content column the box would
    // show no message; report no fit so the caller can fall back to the status bar.
    if width <= BORDERS {
        return None;
    }
    let inner_width = width.saturating_sub(BORDERS).max(1);
    let lines = wrap(&label, inner_width as usize);
    let height = (lines.len() as u16).saturating_add(BORDERS);
    // Clamping instead would let `Paragraph` silently clip wrapped lines while
    // still reporting a region, suppressing the status-bar fallback; report no fit
    // so the full message survives there.
    if height > available {
        return None;
    }
    let x = area.x + area.width.saturating_sub(width).saturating_sub(MARGIN);
    let bottom = area.y + area.height - MARGIN - bottom_offset;
    let y = bottom.saturating_sub(height);
    Some((
        Rect {
            x,
            y,
            width,
            height,
        },
        lines,
    ))
}

/// Draws a message as a rounded box pinned to the bottom-right of `area`, its
/// bottom edge lifted `bottom_offset` rows so boxes stack upward without
/// overlapping. The box grows to its message up to `MAX_WIDTH`, then wraps
/// downward-growing text onto more lines rather than truncating, carries a close
/// affordance on its top-right border, and never exceeds `area`. Returns the
/// height it drew, so a caller can stack the next box directly above it.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    message: &str,
    tone: ToastTone,
    bottom_offset: u16,
) -> u16 {
    let Some((rect, lines)) = geometry(area, message, tone, bottom_offset) else {
        return 0;
    };
    let color = tone.color();
    // Wipe the box interior first: over a nonblank pane the paragraph and border
    // only overwrite their own cells, leaving underlying output showing through.
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color));
    let body: Vec<Line> = lines.into_iter().map(Line::from).collect();
    frame.render_widget(
        Paragraph::new(body)
            .style(Style::default().fg(color).add_modifier(Modifier::BOLD))
            .block(block),
        rect,
    );
    let close_x = rect.x + rect.width.saturating_sub(BORDERS);
    frame.buffer_mut().set_string(
        close_x,
        rect.y,
        CLOSE_GLYPH,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    );
    rect.height
}

/// Greedily wraps `text` into lines at most `width` display columns wide,
/// measuring in terminal columns so full-width glyphs count as two, and
/// hard-breaking any single word wider than `width`. Always returns one line.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        while UnicodeWidthStr::width(word.as_str()) > width {
            if current_width > 0 {
                lines.push(std::mem::take(&mut current));
                current_width = 0;
            }
            let (head, tail) = split_at_width(&word, width);
            lines.push(head);
            word = tail;
        }
        let word_width = UnicodeWidthStr::width(word.as_str());
        let separator = usize::from(current_width > 0);
        if current_width + separator + word_width > width && current_width > 0 {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
        }
        if current_width > 0 {
            current.push(' ');
            current_width += 1;
        }
        current.push_str(&word);
        current_width += word_width;
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// Splits `word` at the last character boundary that keeps the head within
/// `width` display columns, always taking at least one character so a caller
/// looping on the remainder makes progress. Returns `(head, remainder)`.
fn split_at_width(word: &str, width: usize) -> (String, String) {
    let mut head = String::new();
    let mut head_width = 0;
    for (index, ch) in word.char_indices() {
        let column_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if index > 0 && head_width + column_width > width {
            return (head, word[index..].to_string());
        }
        head.push(ch);
        head_width += column_width;
    }
    (head, String::new())
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    /// Wrapping counts full-width glyphs as two columns, so five CJK characters
    /// fill an eight-column line as four-then-one rather than five on one line.
    #[test]
    fn wrapping_measures_wide_glyphs_as_two_columns() {
        let lines = wrap("\u{4e00}\u{4e8c}\u{4e09}\u{56db}\u{4e94}", 8);

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].chars().count(), 4);
        assert_eq!(lines[1].chars().count(), 1);
    }

    /// A success toast shows its glyph and message, pinned one cell from the
    /// bottom-right corner of the area.
    #[test]
    fn a_success_toast_is_pinned_to_the_bottom_right() {
        let backend_width = 40;
        let backend_height = 12;
        let mut terminal =
            Terminal::new(TestBackend::new(backend_width, backend_height)).expect("test terminal");

        terminal
            .draw(|frame| {
                render(frame, frame.area(), TOAST_COPIED, ToastTone::Success, 0);
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();
        let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(rendered.contains(&format!("{SUCCESS_GLYPH}{TOAST_COPIED}")));
        let bottom_right = buffer
            .cell((backend_width - 1 - MARGIN, backend_height - 1 - MARGIN))
            .expect("bottom-right corner cell");
        assert_eq!(bottom_right.symbol(), "\u{256f}");
    }

    /// A box offset by one box-height stacks directly above the bottom one.
    #[test]
    fn a_stacked_toast_sits_above_the_first() {
        let backend_width = 40;
        let backend_height = 12;
        let mut terminal =
            Terminal::new(TestBackend::new(backend_width, backend_height)).expect("test terminal");

        let mut height = 0;
        terminal
            .draw(|frame| {
                height = render(frame, frame.area(), "second", ToastTone::Error, BOX_HEIGHT)
            })
            .expect("draw");

        assert_eq!(height, BOX_HEIGHT);
        let buffer = terminal.backend().buffer();
        let bottom_right = buffer
            .cell((
                backend_width - 1 - MARGIN,
                backend_height - 1 - MARGIN - BOX_HEIGHT,
            ))
            .expect("stacked corner cell");
        assert_eq!(bottom_right.symbol(), "\u{256f}");
    }

    /// A region too narrow for borders plus a content column reports no fit, so
    /// the caller falls back to the status bar instead of drawing an empty box.
    #[test]
    fn a_too_narrow_area_has_no_region() {
        let narrow = Rect::new(0, 0, BORDERS + 1, 12);
        assert!(region(narrow, "boom", ToastTone::Error, 0).is_none());
        let wide_enough = Rect::new(0, 0, BORDERS + 2, 12);
        assert!(region(wide_enough, "boom", ToastTone::Error, 0).is_some());
    }

    /// A long message wraps onto more lines instead of being truncated: every
    /// character survives, and the box never grows past the maximum width.
    #[test]
    fn a_long_message_wraps_instead_of_being_truncated() {
        let backend_width = 120;
        let backend_height = 12;
        let mut terminal =
            Terminal::new(TestBackend::new(backend_width, backend_height)).expect("test terminal");
        let long = "x".repeat(120);

        terminal
            .draw(|frame| {
                render(frame, frame.area(), &long, ToastTone::Error, 0);
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();
        let preserved = buffer
            .content
            .iter()
            .filter(|cell| cell.symbol() == "x")
            .count();
        assert_eq!(preserved, 120);
        let right_border = backend_width - 1 - MARGIN;
        let left_border = right_border - (MAX_WIDTH - 1);
        assert_eq!(
            buffer
                .cell((left_border, backend_height - 1 - MARGIN))
                .unwrap()
                .symbol(),
            "\u{2570}"
        );
    }
}
