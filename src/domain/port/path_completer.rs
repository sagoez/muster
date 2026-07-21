/// Driven port: suggests directory paths for autocomplete.
pub trait PathCompleter {
    /// Directory paths that complete `partial`: entries of the directory it
    /// names (or its parent), keeping whatever prefix the user typed. Returns an
    /// empty list when nothing matches or the directory cannot be read.
    fn complete_dir(&self, partial: &str) -> Vec<String>;
}
