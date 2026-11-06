//! The compact text an agent reads for results that carry output, so the
//! lines are not paid for twice as JSON.

use muster_client::LogSliceView;
use muster_os::sanitize::printable_lines;
use muster_protocol::service::WaitOutcome;
use muster_terminal::log_renderer::dropped_lines;

/// Most bytes of a screen returned, so a huge terminal cannot flood a call.
pub const SCREEN_MAX_BYTES: usize = 64 * 1024;
/// Marks a screen cut to [`SCREEN_MAX_BYTES`].
const SCREEN_CUT: &str = "[... screen cut]";

/// A log slice under a one-line header naming the range it covers, with the
/// count of earlier lines dropped to fit.
pub fn log_text(view: &LogSliceView) -> String {
    let mut text = format!(
        "{} {}..{} ({} lines)",
        view.process(),
        view.from(),
        view.cursor(),
        view.lines_returned()
    );
    if view.omitted_lines() > 0 {
        text.push_str(&format!(
            "\n[... {} earlier lines omitted]",
            view.omitted_lines()
        ));
    }
    if view.unrendered_lines() > 0 {
        text.push_str(&format!("\n[{}]", dropped_lines(view.unrendered_lines())));
    }
    if !view.text().is_empty() {
        text.push('\n');
        text.push_str(view.text());
    }
    text
}

/// `screen` made safe to read and cut to [`SCREEN_MAX_BYTES`], without its
/// trailing blank lines.
pub fn screen_text(screen: &str) -> String {
    let printable = printable_lines(screen.trim_end());
    if printable.len() <= SCREEN_MAX_BYTES {
        return printable;
    }
    let mut end = SCREEN_MAX_BYTES;
    while !printable.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n{SCREEN_CUT}", &printable[..end])
}

/// A wait that did not match, as one header line and the run's last lines.
pub fn tail_text(outcome: &WaitOutcome) -> String {
    let mut text = format!(
        "{} {} after {}ms (last {} lines)",
        outcome.process(),
        outcome.status(),
        outcome.elapsed().as_millis(),
        outcome.tail().len()
    );
    for line in outcome.tail() {
        text.push('\n');
        text.push_str(line);
    }
    text
}

#[cfg(test)]
mod tests {
    use muster_domain::value::{OutputCursor, ProcessName};
    use muster_protocol::holder::LogSlice;

    use super::*;

    /// A log reads as its header, the omission marker and its lines.
    #[test]
    fn a_log_reads_as_a_header_and_its_lines() {
        let slice = LogSlice::builder()
            .from(OutputCursor::new(16010))
            .cursor(OutputCursor::new(18234))
            .lines(vec![
                "Listening on 3000".to_string(),
                "GET / 200".to_string(),
            ])
            .omitted_lines(3)
            .rotated_away_bytes(0)
            .unrendered_lines(0)
            .build();
        let view = LogSliceView::for_slice(ProcessName::try_new("api").unwrap(), &slice, None);

        assert_eq!(
            log_text(&view),
            "api o:16010..o:18234 (2 lines)\n[... 3 earlier lines omitted]\nListening on 3000\nGET / 200"
        );
    }

    /// Lines rendering dropped are named under the header, so an agent knows
    /// the log it reads is missing some.
    #[test]
    fn dropped_lines_are_named() {
        let slice = LogSlice::builder()
            .from(OutputCursor::new(0))
            .cursor(OutputCursor::new(9))
            .lines(vec!["ok".to_string()])
            .omitted_lines(0)
            .rotated_away_bytes(0)
            .unrendered_lines(2)
            .build();
        let view = LogSliceView::for_slice(ProcessName::try_new("api").unwrap(), &slice, None);

        assert_eq!(view.unrendered_lines(), 2);
        assert_eq!(
            log_text(&view),
            "api o:0..o:9 (1 lines)\n[2 line(s) of output could not be rendered and were \
             dropped]\nok"
        );
        assert_eq!(serde_json::to_value(&view).unwrap()["unrendered_lines"], 2);
    }

    /// A screen loses its trailing blanks and its escape sequences, and a
    /// huge one is cut.
    #[test]
    fn a_screen_is_safe_and_bounded() {
        let screen = screen_text("\u{1b}]52;c;evil\u{7}hello\n\n\n");
        assert!(!screen.contains('\u{1b}'), "{screen:?}");
        assert!(screen.ends_with("hello"), "{screen:?}");
        let huge = screen_text(&"x".repeat(SCREEN_MAX_BYTES * 2));
        assert!(huge.len() < SCREEN_MAX_BYTES + SCREEN_CUT.len() + 2);
        assert!(huge.ends_with(SCREEN_CUT));
    }
}
