/// A cell color independent of any rendering frontend. Palette indices pass
/// through so the host terminal applies its own theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellColor {
    /// The terminal's default foreground or background.
    Default,
    /// An entry of the 256-color palette.
    Indexed(u8),
    /// A true-color value.
    Rgb(u8, u8, u8),
}
