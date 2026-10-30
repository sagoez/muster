/// Whether a rendered log line keeps the colours it was drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colouring {
    /// Text only.
    Plain,
    /// Text with its colours as SGR sequences, reset before the line ends.
    Coloured,
}
