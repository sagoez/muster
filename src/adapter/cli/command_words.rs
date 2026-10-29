use super::error::CliError;
use crate::domain::value::CommandLine;

/// The command `add` and `run` store, from the words after `--`.
///
/// A single argument is already a shell expression the user quoted (e.g.
/// `'npm test && npm run build'`), so it is kept verbatim to preserve
/// pipelines, redirects, `&&` and variable expansion. Several arguments came
/// from the shell's own tokenization, so each is re-escaped to keep its
/// boundary (e.g. `printf '%s\n' 'hello world'`).
///
/// # Errors
/// Returns [`CliError::InvalidCommand`] when the words cannot be joined into
/// a shell command, or [`CliError::EmptyCommand`] for a blank one.
pub fn command_string(words: &[String]) -> Result<CommandLine, CliError> {
    let joined = match words {
        [expression] => expression.clone(),
        _ => shlex::try_join(words.iter().map(String::as_str))
            .map_err(|_| CliError::InvalidCommand)?,
    };
    CommandLine::try_new(joined).map_err(|_| CliError::EmptyCommand)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words, owned.
    fn words(raw: &[&str]) -> Vec<String> {
        raw.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn several_words_keep_their_boundaries() {
        let argv = words(&["printf", "%s\n", "hello world"]);
        let rebuilt = command_string(&argv).unwrap();

        assert_eq!(shlex::split(rebuilt.as_ref()).unwrap(), argv);
        assert_ne!(rebuilt.as_ref(), argv.join(" "));
    }

    #[test]
    fn a_single_argument_is_kept_as_a_shell_expression() {
        assert_eq!(
            command_string(&words(&["npm test && npm run build"]))
                .unwrap()
                .as_ref(),
            "npm test && npm run build"
        );
    }

    #[test]
    fn a_blank_or_unjoinable_command_is_refused() {
        assert!(matches!(
            command_string(&words(&["  "])),
            Err(CliError::EmptyCommand)
        ));
        assert!(matches!(
            command_string(&words(&["a", "b\0c"])),
            Err(CliError::InvalidCommand)
        ));
    }
}
