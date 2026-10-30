use getset::Getters;
use typed_builder::TypedBuilder;

use super::{
    colouring::Colouring,
    escape::{CSI, SGR_RESET},
    line_cell::LineCell,
    pen::Pen,
};

/// Final byte of an SGR sequence.
const SGR_FINAL: char = 'm';

/// One line of a process's output as a log reader shows it: the characters
/// left on it once its carriage returns, erases and cursor moves were
/// applied, each with the pen it was drawn in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Getters, TypedBuilder)]
pub struct LogLine {
    /// The characters, left to right.
    #[getset(get = "pub")]
    cells: Vec<LineCell>,
}

impl LogLine {
    /// The line's text, without trailing whitespace.
    pub fn plain(&self) -> String {
        self.shown().iter().map(LineCell::character).collect()
    }

    /// The line's text as [`LogLine::plain`] has it, each run of characters
    /// preceded by the SGR sequence that draws its pen, and a reset at the
    /// end when a pen is still in effect, so no colour outlives the line.
    pub fn coloured(&self) -> String {
        let mut text = String::new();
        let mut current = Pen::default();
        for cell in self.shown() {
            if *cell.pen() != current {
                if !current.is_plain() {
                    text.push_str(SGR_RESET);
                }
                if !cell.pen().is_plain() {
                    text.push_str(CSI);
                    text.push_str(cell.pen().params());
                    text.push(SGR_FINAL);
                }
                current = cell.pen().clone();
            }
            text.push(cell.character());
        }
        if !current.is_plain() {
            text.push_str(SGR_RESET);
        }
        text
    }

    /// The line's text as `colouring` asks.
    pub fn text(&self, colouring: Colouring) -> String {
        match colouring {
            Colouring::Plain => self.plain(),
            Colouring::Coloured => self.coloured(),
        }
    }

    /// Whether nothing but whitespace is left on the line.
    pub fn is_blank(&self) -> bool {
        self.shown().is_empty()
    }

    /// The cells up to the last one that is not whitespace.
    fn shown(&self) -> &[LineCell] {
        let end = self
            .cells
            .iter()
            .rposition(|cell| !cell.character().is_whitespace())
            .map_or(0, |last| last + 1);
        &self.cells[..end]
    }
}
