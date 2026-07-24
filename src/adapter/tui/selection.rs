use getset::CopyGetters;
use ratatui::layout::{Position, Rect};
use typed_builder::TypedBuilder;
use unicode_width::UnicodeWidthChar;

use crate::domain::value::PaneId;

/// Punctuation that ends a double-click word selection (herdr's set).
const WORD_SEPARATORS: [char; 10] = ['|', '(', ')', '[', ']', '{', '}', ',', ';', '!'];
/// URL prefixes a double-click prefers to select whole.
const URL_PREFIXES: [&str; 2] = ["http://", "https://"];
/// Multiplier turning the drag distance past a pane edge into scrolled lines.
const EDGE_SCROLL_FACTOR: usize = 3;
/// Fewest lines an edge drag scrolls at once.
const EDGE_SCROLL_MIN: usize = 3;
/// Most lines an edge drag scrolls at once.
const EDGE_SCROLL_MAX: usize = 15;

/// One cell of a pane's visible grid, in zero-based viewport coordinates.
/// Ordering is row-major (field order matters for the derive).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct GridCell {
    row: u16,
    column: u16,
}

/// One cell addressed in absolute buffer coordinates: a row counted from the
/// top of the scrollback, so the position survives viewport scrolling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct BufferCell {
    row: usize,
    column: u16,
}

/// Where a pane's viewport sits within its scrollback: `offset` rows up from
/// the live screen, out of `len` retained scrollback rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct ScrollMetrics {
    offset: usize,
    len: usize,
}

impl ScrollMetrics {
    /// Absolute buffer row of the viewport's top line.
    pub fn viewport_top(&self) -> usize {
        self.len.saturating_sub(self.offset)
    }

    /// Lifts a viewport cell into absolute buffer coordinates.
    pub fn buffer_cell(&self, cell: GridCell) -> BufferCell {
        BufferCell::builder()
            .row(self.viewport_top() + usize::from(cell.row()))
            .column(cell.column())
            .build()
    }
}

/// Lifecycle of a drag selection, mirroring a native terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Button down, not yet moved: just a click unless a drag follows.
    Anchored,
    /// Actively dragging; cells highlight as the head moves.
    Dragging,
    /// Finalized word copy; the highlight lingers briefly as feedback.
    Done,
}

/// A text selection over one pane's scrollback buffer.
#[derive(Clone, Copy, Debug, CopyGetters)]
pub struct Selection {
    #[getset(get_copy = "pub")]
    pane: PaneId,
    anchor: BufferCell,
    head: BufferCell,
    phase: Phase,
}

impl Selection {
    /// Records a pressed anchor; nothing is highlighted until a drag follows.
    pub fn anchored(pane: PaneId, cell: BufferCell) -> Self {
        Self {
            pane,
            anchor: cell,
            head: cell,
            phase: Phase::Anchored,
        }
    }

    /// A finalized word selection spanning `start..=end`, highlighted as
    /// short-lived copy feedback.
    pub fn word(pane: PaneId, start: BufferCell, end: BufferCell) -> Self {
        Self {
            pane,
            anchor: start,
            head: end,
            phase: Phase::Done,
        }
    }

    /// Moves the head to `cell`; leaving the anchor cell turns the gesture
    /// into a real drag.
    pub fn extend_to(&mut self, cell: BufferCell) {
        self.head = cell;
        if self.phase == Phase::Anchored && self.head != self.anchor {
            self.phase = Phase::Dragging;
        }
    }

    /// Whether the gesture is still a bare click.
    pub fn is_click(&self) -> bool {
        self.phase == Phase::Anchored
    }

    /// Whether the user is actively dragging.
    pub fn is_dragging(&self) -> bool {
        self.phase == Phase::Dragging
    }

    /// Whether the pointer is still down and the selection can keep extending.
    pub fn is_in_progress(&self) -> bool {
        matches!(self.phase, Phase::Anchored | Phase::Dragging)
    }

    /// Whether the highlight should be drawn.
    pub fn is_visible(&self) -> bool {
        matches!(self.phase, Phase::Dragging | Phase::Done)
    }

    /// The selection endpoints in reading order (top-left to bottom-right).
    pub fn span(&self) -> (BufferCell, BufferCell) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// The visible part of the span inside a viewport of `rows` x `columns`
    /// whose top line is absolute row `viewport_top`; `None` when the span is
    /// entirely off-screen or nothing should be highlighted.
    pub fn viewport_span(
        &self,
        viewport_top: usize,
        rows: u16,
        columns: u16,
    ) -> Option<(GridCell, GridCell)> {
        if !self.is_visible() || rows == 0 || columns == 0 {
            return None;
        }
        let (start, end) = self.span();
        let bottom = viewport_top + usize::from(rows);
        if end.row() < viewport_top || start.row() >= bottom {
            return None;
        }
        let max_column = columns - 1;
        let first = if start.row() < viewport_top {
            GridCell::builder().row(0).column(0).build()
        } else {
            GridCell::builder()
                .row((start.row() - viewport_top) as u16)
                .column(start.column().min(max_column))
                .build()
        };
        let last = if end.row() >= bottom {
            GridCell::builder().row(rows - 1).column(max_column).build()
        } else {
            GridCell::builder()
                .row((end.row() - viewport_top) as u16)
                .column(end.column().min(max_column))
                .build()
        };
        Some((first, last))
    }
}

/// Direction an edge drag keeps scrolling while the button is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoscrollDirection {
    /// Scrolling back into the scrollback.
    Up,
    /// Scrolling forward toward the live screen.
    Down,
}

/// An active edge-drag autoscroll: the direction plus the last pointer
/// position, used to keep extending the selection between ticks.
#[derive(Clone, Copy, Debug, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct Autoscroll {
    direction: AutoscrollDirection,
    column: u16,
    row: u16,
}

/// Lines an edge drag scrolls immediately, scaled by how far past the edge
/// the pointer travelled (herdr's ramp).
pub fn edge_scroll_lines(distance: u16) -> usize {
    usize::from(distance)
        .saturating_mul(EDGE_SCROLL_FACTOR)
        .clamp(EDGE_SCROLL_MIN, EDGE_SCROLL_MAX)
}

/// Maps an absolute terminal coordinate to a cell of the grid drawn at `grid`,
/// or `None` when the coordinate falls outside it.
pub fn cell_at(grid: Rect, column: u16, row: u16) -> Option<GridCell> {
    grid.contains(Position::new(column, row)).then(|| {
        GridCell::builder()
            .row(row - grid.y)
            .column(column - grid.x)
            .build()
    })
}

/// Maps an absolute terminal coordinate to the nearest cell of the grid drawn
/// at `grid`, clamping coordinates that fall outside it. `None` when the grid
/// is empty.
pub fn nearest_cell(grid: Rect, column: u16, row: u16) -> Option<GridCell> {
    if grid.width == 0 || grid.height == 0 {
        return None;
    }
    let column = column.clamp(grid.x, grid.x + grid.width - 1);
    let row = row.clamp(grid.y, grid.y + grid.height - 1);
    cell_at(grid, column, row)
}

/// One display cell of a row's text: the character and the terminal columns it
/// occupies, so word bounds hold under wide glyphs. Ported from herdr.
#[derive(Clone, Copy)]
struct TextCell {
    ch: char,
    start_column: u16,
    end_column: u16,
}

/// An inclusive range of cell indices within a row.
#[derive(Clone, Copy)]
struct CellSpan {
    start: usize,
    end: usize,
}

impl CellSpan {
    /// Whether the span covers the given cell index.
    fn contains(self, index: usize) -> bool {
        index >= self.start && index <= self.end
    }

    /// The inclusive terminal-column bounds the span occupies.
    fn columns(self, cells: &[TextCell]) -> (u16, u16) {
        (cells[self.start].start_column, cells[self.end].end_column)
    }
}

/// The inclusive column bounds of the token under `column` in a row's text,
/// following herdr's pipeline: prefer spans users expect to copy whole (URLs,
/// then quoted paths), falling back to a separator-delimited token. `None`
/// when the click lands on whitespace or a separator.
pub fn word_bounds(row: &str, column: u16) -> Option<(u16, u16)> {
    let cells = text_cells(row);
    let clicked = cells
        .iter()
        .position(|cell| cell.start_column <= column && column <= cell.end_column)?;
    let span = url_span_at(&cells, clicked)
        .or_else(|| quoted_path_span_at(&cells, clicked))
        .or_else(|| token_span_at(&cells, clicked))?;
    Some(span.columns(&cells))
}

/// Maps a row's characters onto the display columns they occupy.
fn text_cells(row: &str) -> Vec<TextCell> {
    let mut next_column: u16 = 0;
    row.chars()
        .map(|ch| {
            let width = ch.width().unwrap_or(0) as u16;
            let start_column = if width == 0 {
                next_column.saturating_sub(1)
            } else {
                next_column
            };
            let end_column = start_column + width.saturating_sub(1);
            if width > 0 {
                next_column = next_column.saturating_add(width);
            }
            TextCell {
                ch,
                start_column,
                end_column,
            }
        })
        .collect()
}

/// The span of a URL containing `clicked`: a scheme prefix run to the next
/// whitespace, with unbalanced closers and trailing punctuation trimmed.
fn url_span_at(cells: &[TextCell], clicked: usize) -> Option<CellSpan> {
    let mut start = 0;
    while start < cells.len() {
        if !URL_PREFIXES
            .iter()
            .any(|prefix| starts_with_chars(&cells[start..], prefix))
        {
            start += 1;
            continue;
        }
        let mut end = start;
        while end + 1 < cells.len() && !cells[end + 1].ch.is_whitespace() {
            end += 1;
        }
        if let Some(span) = trim_url_edges(cells, CellSpan { start, end })
            && span.contains(clicked)
        {
            return Some(span);
        }
        start = end + 1;
    }
    None
}

/// Drops trailing punctuation and unbalanced closing brackets from a URL span.
fn trim_url_edges(cells: &[TextCell], span: CellSpan) -> Option<CellSpan> {
    let start = span.start;
    let mut end = span.end;
    while start <= end && should_trim_trailing_url_cell(cells, start, end) {
        if end == 0 {
            return None;
        }
        end -= 1;
    }
    (start <= end).then_some(CellSpan { start, end })
}

/// Whether the URL span's final cell is punctuation that reads as prose, not
/// as part of the URL.
fn should_trim_trailing_url_cell(cells: &[TextCell], start: usize, end: usize) -> bool {
    match cells[end].ch {
        '"' | '\'' | '`' | '.' | ',' | ';' | ':' | '!' | '?' => true,
        ')' => !trailing_url_closer_is_balanced(cells, start, end, '(', ')'),
        ']' => !trailing_url_closer_is_balanced(cells, start, end, '[', ']'),
        '}' => !trailing_url_closer_is_balanced(cells, start, end, '{', '}'),
        _ => false,
    }
}

/// Whether a trailing closer has a matching opener earlier in the URL.
fn trailing_url_closer_is_balanced(
    cells: &[TextCell],
    start: usize,
    end: usize,
    open: char,
    close: char,
) -> bool {
    let mut balance = 0i32;
    for cell in &cells[start..end] {
        if cell.ch == open {
            balance += 1;
        } else if cell.ch == close {
            balance -= 1;
        }
    }
    balance > 0
}

/// The span between matching unescaped quotes around `clicked`, selected only
/// when the quoted text looks like a path (contains a slash).
fn quoted_path_span_at(cells: &[TextCell], clicked: usize) -> Option<CellSpan> {
    let clicked_char = cells.get(clicked)?.ch;
    if matches!(clicked_char, '"' | '\'' | '`') {
        return None;
    }
    for quote in ['"', '\'', '`'] {
        let mut start = None;
        for (index, cell) in cells.iter().copied().enumerate() {
            if cell.ch != quote || is_escaped(cells, index) {
                continue;
            }
            if let Some(open) = start {
                if clicked > open
                    && clicked < index
                    && cells[open + 1..index].iter().any(|cell| cell.ch == '/')
                {
                    return Some(CellSpan {
                        start: open + 1,
                        end: index - 1,
                    });
                }
                start = None;
            } else {
                start = Some(index);
            }
        }
    }
    None
}

/// Whether the cell at `index` is preceded by an odd run of backslashes.
fn is_escaped(cells: &[TextCell], index: usize) -> bool {
    let mut slashes = 0;
    let mut cursor = index;
    while cursor > 0 && cells[cursor - 1].ch == '\\' {
        slashes += 1;
        cursor -= 1;
    }
    slashes % 2 == 1
}

/// Whether the cells begin with the given ASCII prefix.
fn starts_with_chars(cells: &[TextCell], prefix: &str) -> bool {
    prefix
        .chars()
        .enumerate()
        .all(|(index, expected)| cells.get(index).is_some_and(|cell| cell.ch == expected))
}

/// The separator-delimited token containing `clicked`, with surrounding
/// wrapper punctuation (quotes, brackets, trailing prose marks) trimmed.
fn token_span_at(cells: &[TextCell], clicked: usize) -> Option<CellSpan> {
    if is_word_separator(cells[clicked].ch) {
        return None;
    }
    let mut start = clicked;
    while start > 0 && !is_word_separator(cells[start - 1].ch) {
        start -= 1;
    }
    let mut end = clicked;
    while end + 1 < cells.len() && !is_word_separator(cells[end + 1].ch) {
        end += 1;
    }
    trim_token_edges(cells, CellSpan { start, end }).filter(|span| span.contains(clicked))
}

/// Strips wrapper punctuation from a token's edges, keeping shell `$`-suffixed
/// wrappers intact the way herdr does.
fn trim_token_edges(cells: &[TextCell], span: CellSpan) -> Option<CellSpan> {
    let mut start = span.start;
    let mut end = span.end;
    while start <= end && is_leading_token_wrapper(cells[start].ch) {
        start += 1;
    }
    if start < end && cells[end].ch == '$' && is_trailing_token_wrapper(cells[end - 1].ch) {
        end -= 1;
    }
    while start <= end && is_trailing_token_wrapper(cells[end].ch) {
        if end == 0 {
            return None;
        }
        end -= 1;
    }
    (start <= end).then_some(CellSpan { start, end })
}

/// Opening punctuation trimmed from a token's left edge.
fn is_leading_token_wrapper(ch: char) -> bool {
    matches!(ch, '(' | '[' | '{' | '<' | '"' | '\'' | '`')
}

/// Closing punctuation trimmed from a token's right edge.
fn is_trailing_token_wrapper(ch: char) -> bool {
    matches!(
        ch,
        ')' | ']' | '}' | '>' | '"' | '\'' | '`' | '.' | ',' | ';' | ':' | '!' | '?'
    )
}

/// Whether a character ends a double-click word.
fn is_word_separator(ch: char) -> bool {
    ch.is_whitespace() || WORD_SEPARATORS.contains(&ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grid used by every mapping test: x 10, y 5, 20 columns, 10 rows.
    fn grid() -> Rect {
        Rect::new(10, 5, 20, 10)
    }

    fn cell(row: u16, column: u16) -> GridCell {
        GridCell::builder().row(row).column(column).build()
    }

    fn buffer_cell(row: usize, column: u16) -> BufferCell {
        BufferCell::builder().row(row).column(column).build()
    }

    fn metrics(offset: usize, len: usize) -> ScrollMetrics {
        ScrollMetrics::builder().offset(offset).len(len).build()
    }

    /// Coordinates inside the grid map to zero-based cells.
    #[test]
    fn cell_at_maps_inside_coordinates() {
        assert_eq!(cell_at(grid(), 10, 5), Some(cell(0, 0)));
        assert_eq!(cell_at(grid(), 29, 14), Some(cell(9, 19)));
    }

    /// Coordinates outside the grid do not begin a selection.
    #[test]
    fn cell_at_rejects_outside_coordinates() {
        assert_eq!(cell_at(grid(), 9, 5), None);
        assert_eq!(cell_at(grid(), 30, 5), None);
        assert_eq!(cell_at(grid(), 10, 4), None);
        assert_eq!(cell_at(grid(), 10, 15), None);
    }

    /// Dragging past any edge clamps to the nearest grid cell.
    #[test]
    fn nearest_cell_clamps_outside_coordinates() {
        assert_eq!(nearest_cell(grid(), 0, 0), Some(cell(0, 0)));
        assert_eq!(nearest_cell(grid(), 100, 100), Some(cell(9, 19)));
        assert_eq!(nearest_cell(grid(), 15, 100), Some(cell(9, 5)));
    }

    /// Scroll metrics lift viewport cells into stable buffer coordinates.
    #[test]
    fn metrics_lift_cells_into_buffer_rows() {
        let scrolled = metrics(4, 10);
        assert_eq!(scrolled.viewport_top(), 6);
        assert_eq!(scrolled.buffer_cell(cell(2, 7)), buffer_cell(8, 7));
    }

    /// A backwards drag normalizes into a forward span.
    #[test]
    fn span_orders_backwards_drags() {
        let mut selection = Selection::anchored(PaneId::new(1), buffer_cell(5, 8));
        selection.extend_to(buffer_cell(2, 12));
        assert_eq!(selection.span(), (buffer_cell(2, 12), buffer_cell(5, 8)));
        assert!(selection.is_dragging());
    }

    /// A selection that never left its anchor stays a bare click.
    #[test]
    fn undragged_selection_is_a_bare_click() {
        let mut selection = Selection::anchored(PaneId::new(1), buffer_cell(1, 1));
        selection.extend_to(buffer_cell(1, 1));
        assert!(selection.is_click());
        assert!(!selection.is_visible());
    }

    /// The viewport span clips a partially visible selection to the screen.
    #[test]
    fn viewport_span_clips_offscreen_rows() {
        let mut selection = Selection::anchored(PaneId::new(1), buffer_cell(2, 4));
        selection.extend_to(buffer_cell(30, 3));
        // Viewport shows rows 10..20: the span enters from above and exits below.
        assert_eq!(
            selection.viewport_span(10, 10, 40),
            Some((cell(0, 0), cell(9, 39)))
        );
        // Fully above the viewport.
        assert_eq!(selection.viewport_span(40, 10, 40), None);
    }

    /// An anchored click never renders a viewport span.
    #[test]
    fn viewport_span_hides_bare_clicks() {
        let selection = Selection::anchored(PaneId::new(1), buffer_cell(2, 4));
        assert_eq!(selection.viewport_span(0, 10, 40), None);
    }

    /// The edge-drag ramp scales with distance within herdr's bounds.
    #[test]
    fn edge_scroll_ramp_is_clamped() {
        assert_eq!(edge_scroll_lines(0), EDGE_SCROLL_MIN);
        assert_eq!(edge_scroll_lines(1), 3);
        assert_eq!(edge_scroll_lines(4), 12);
        assert_eq!(edge_scroll_lines(50), EDGE_SCROLL_MAX);
    }

    /// A double-click on a word selects between separators.
    #[test]
    fn word_bounds_select_the_clicked_token() {
        assert_eq!(word_bounds("hello world", 1), Some((0, 4)));
        assert_eq!(word_bounds("hello world", 8), Some((6, 10)));
        assert_eq!(word_bounds("a (b/c.txt)", 4), Some((3, 9)));
    }

    /// Clicking whitespace or a separator selects nothing.
    #[test]
    fn word_bounds_reject_separators() {
        assert_eq!(word_bounds("hello world", 5), None);
        assert_eq!(word_bounds("a (b)", 2), None);
    }

    /// Clicking inside a URL selects the whole URL, separators included.
    #[test]
    fn word_bounds_prefer_whole_urls() {
        let row = "see https://example.com/a(1),b now";
        assert_eq!(word_bounds(row, 10), Some((4, 29)));
    }

    /// Trailing prose punctuation is trimmed off a URL.
    #[test]
    fn word_bounds_trim_url_punctuation() {
        let row = "read https://example.com/doc.";
        assert_eq!(word_bounds(row, 8), Some((5, 27)));
    }

    /// A quoted path is selected whole, without its quotes.
    #[test]
    fn word_bounds_select_quoted_paths() {
        assert_eq!(word_bounds("x '/a/b c' y", 4), Some((3, 8)));
    }

    /// Wrapper punctuation is trimmed from a token's edges.
    #[test]
    fn word_bounds_trim_token_wrappers() {
        assert_eq!(word_bounds("'foo'.", 2), Some((1, 3)));
    }

    /// Wide glyphs occupy two columns without shifting word bounds.
    #[test]
    fn word_bounds_follow_display_columns() {
        // "你好 ok": the CJK pair covers columns 0..=3, "ok" starts at 5.
        assert_eq!(word_bounds("你好 ok", 2), Some((0, 3)));
        assert_eq!(word_bounds("你好 ok", 5), Some((5, 6)));
    }
}
