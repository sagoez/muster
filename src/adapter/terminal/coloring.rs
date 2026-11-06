/// Whether a rendered log line keeps the colors it was drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coloring {
    /// Text only.
    Plain,
    /// Text with its colors as SGR sequences, reset before the line ends.
    Colored,
}
