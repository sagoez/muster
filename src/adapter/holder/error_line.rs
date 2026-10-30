//! Recognising the lines an errors-only log read keeps: a line that reports
//! an error, and the continuation lines (a stack trace, a code excerpt) that
//! follow it.

/// Words that mark a line as an error, matched whole and ignoring case.
const ERROR_WORDS: [&str; 11] = [
    "error",
    "fatal",
    "panic",
    "panicked",
    "exception",
    "traceback",
    "failed",
    "uncaught",
    "unhandled",
    "eaddrinuse",
    "econnrefused",
];
/// Suffixes of an exception class name, such as `TypeError` or
/// `NullPointerException`, matched case-sensitively.
const ERROR_CLASS_SUFFIXES: [&str; 2] = ["Error", "Exception"];
/// A one-letter error level that opens a line, as in `E/Tag:` or `[E]`.
const ERROR_LEVEL_LETTER: &str = "E";
/// Digits after the level letter in a glog-style header such as `E1029`.
const GLOG_DATE_DIGITS: usize = 4;
/// Starts of a line that continues the error above it.
const CONTINUATION_PREFIXES: [&str; 5] = ["at ", "File \"", "Caused by", "-->", "|"];

/// Whether `character` belongs to a word.
fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Whether `word` names an exception class, such as `ValueError`.
fn is_error_class(word: &str) -> bool {
    word.starts_with(char::is_uppercase)
        && ERROR_CLASS_SUFFIXES
            .iter()
            .any(|suffix| word.len() > suffix.len() && word.ends_with(suffix))
}

/// Whether `word`, the first on its line, is an error level: `E`, or a
/// glog header `E` followed by the date.
fn is_error_level(word: &str) -> bool {
    word.strip_prefix(ERROR_LEVEL_LETTER).is_some_and(|rest| {
        rest.is_empty()
            || (rest.len() >= GLOG_DATE_DIGITS && rest.bytes().all(|byte| byte.is_ascii_digit()))
    })
}

/// Whether `line` reports an error: it holds an error word or an exception
/// class name, or opens with a one-letter error level.
pub fn is_error_line(line: &str) -> bool {
    let mut words = line
        .split(|character| !is_word_character(character))
        .filter(|word| !word.is_empty());
    if words.clone().next().is_some_and(is_error_level) {
        return true;
    }
    words.any(|word| {
        is_error_class(word)
            || ERROR_WORDS
                .iter()
                .any(|error| word.eq_ignore_ascii_case(error))
    })
}

/// Whether `line` continues the error above it: indented, or opening as a
/// stack frame, a cause or a compiler's code excerpt does.
pub fn is_continuation(line: &str) -> bool {
    let indented = line.starts_with(char::is_whitespace) && !line.trim().is_empty();
    indented
        || CONTINUATION_PREFIXES
            .iter()
            .any(|prefix| line.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_lines_are_recognised() {
        for line in [
            "Error: listen EADDRINUSE: address already in use :::3000",
            "thread 'main' panicked at src/main.rs:3:5:",
            "Traceback (most recent call last):",
            "ValueError: invalid literal",
            "Uncaught TypeError: x is undefined",
            "[ERROR] connection refused",
            "2026-10-29 FATAL out of memory",
            "E/ActivityManager: ANR in com.example",
            "E1029 10:41:02.123 server.cc:42] boom",
            "[E] boom",
            "1 test FAILED",
            "java.lang.NullPointerException",
        ] {
            assert!(is_error_line(line), "{line}");
        }
        for line in [
            "GET /health 200 3ms",
            "Listening on http://localhost:3000",
            "Eve signed in",
            "compiled with 0 errors",
            "Error_rate is 0",
            "E2E suite starting",
        ] {
            assert!(!is_error_line(line), "{line}");
        }
    }

    #[test]
    fn continuation_lines_are_recognised() {
        for line in [
            "    at Object.<anonymous> (/app/index.js:3:9)",
            "\tat com.example.Main.main(Main.java:5)",
            "  File \"app.py\", line 3, in <module>",
            "Caused by: java.io.IOException",
            "--> src/main.rs:3:5",
            "| let x = 1;",
            "at foo (bar.js:1:1)",
        ] {
            assert!(is_continuation(line), "{line}");
        }
        for line in ["next request", "", "   ", "GET /"] {
            assert!(!is_continuation(line), "{line}");
        }
    }
}
