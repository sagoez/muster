use std::collections::BTreeMap;

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{
    coloring::Coloring,
    escape::{CSI, SGR_FINAL, SGR_RESET},
    line_cell::LineCell,
    pen::Pen,
};

/// One line of a process's output as a log reader shows it: the glyphs left
/// on it once its carriage returns, erases and cursor moves were applied,
/// each with the pen it was drawn in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct LogLine {
    /// The columns, left to right: no erased column is left in, and a glyph's
    /// later columns follow it whole.
    #[getset(get = "pub")]
    cells: Vec<LineCell>,
    /// The combining marks drawn onto the glyph starting at each column.
    #[getset(get = "pub")]
    #[builder(default)]
    marks: BTreeMap<usize, String>,
    /// Whether this goes on from where the same line, shown in part before,
    /// left off, rather than being a line of its own.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    continues: bool,
}

impl LogLine {
    /// The line's text, without trailing whitespace.
    pub fn plain(&self) -> String {
        let mut text = String::new();
        for (index, cell) in self.glyphs() {
            self.push_glyph(&mut text, index, cell);
        }
        text
    }

    /// The line's text as [`LogLine::plain`] has it, each run of glyphs
    /// preceded by the SGR sequence that draws its pen, and a reset at the
    /// end when a pen is still in effect, so no color outlives the line.
    pub fn colored(&self) -> String {
        let mut text = String::new();
        let mut current = Pen::default();
        for (index, cell) in self.glyphs() {
            if cell.pen() != current {
                if !current.is_plain() {
                    text.push_str(SGR_RESET);
                }
                if !cell.pen().is_plain() {
                    text.push_str(CSI);
                    text.push_str(&cell.pen().params());
                    text.push(SGR_FINAL);
                }
                current = cell.pen();
            }
            self.push_glyph(&mut text, index, cell);
        }
        if !current.is_plain() {
            text.push_str(SGR_RESET);
        }
        text
    }

    /// The line's text as `coloring` asks.
    pub fn text(&self, coloring: Coloring) -> String {
        match coloring {
            Coloring::Plain => self.plain(),
            Coloring::Colored => self.colored(),
        }
    }

    /// Whether nothing but whitespace is left on the line.
    pub fn is_blank(&self) -> bool {
        self.shown_end() == 0
    }

    /// Pushes the glyph starting at column `index` onto `text`, with any
    /// combining marks drawn onto it.
    fn push_glyph(&self, text: &mut String, index: usize, cell: LineCell) {
        text.push(cell.character());
        if let Some(marks) = self.marks.get(&index) {
            text.push_str(marks);
        }
    }

    /// The glyphs up to the last one that is not whitespace, each with the
    /// column it starts at.
    fn glyphs(&self) -> impl Iterator<Item = (usize, LineCell)> + '_ {
        self.cells[..self.shown_end()]
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, cell)| !cell.continues())
    }

    /// The column just past the last glyph that shows anything.
    pub fn shown_end(&self) -> usize {
        self.cells
            .iter()
            .enumerate()
            .rposition(|(index, cell)| {
                !cell.continues()
                    && (!cell.character().is_whitespace() || self.marks.contains_key(&index))
            })
            .map_or(0, |last| {
                (last + usize::from(self.cells[last].width())).min(self.cells.len())
            })
    }
}
