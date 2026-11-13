//! Mouse selection over the grid, scrollback included. Points are grid
//! points (negative lines are scrollback); see
//! [`TerminalModel::viewport_to_point`].

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Boundary, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};

use super::TerminalModel;

/// What a drag selects.
// used by the terminal tab (Plan 6 Task 4)
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    /// Cell by cell (press and drag).
    Cells,
    /// Whole words (double click).
    Word,
    /// Whole lines (triple click).
    Line,
}

impl TerminalModel {
    /// Starts a selection at `point`, replacing any previous one.
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn start_selection(&mut self, kind: SelectionKind, point: Point, side: Side) {
        let ty = match kind {
            SelectionKind::Cells => SelectionType::Simple,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
        };
        let point = self.clamp(point);
        self.term.selection = Some(Selection::new(ty, point, side));
    }

    /// Moves the selection's end (the drag) to `point`.
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn update_selection(&mut self, point: Point, side: Side) {
        let point = self.clamp(point);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    /// The selected text; empty without a selection. Rows that wrapped
    /// because the line was wider than the terminal are joined without a
    /// `\n`; a line selection ends with one.
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn selected_text(&self) -> String {
        self.term.selection_to_string().unwrap_or_default()
    }

    /// `point` moved inside the grid (history to last screen line).
    pub(super) fn clamp(&self, point: Point) -> Point {
        let mut point = point.grid_clamp(&self.term, Boundary::Grid);
        point.column = point.column.min(self.term.last_column());
        point
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::index::{Column, Line};

    fn at(line: i32, column: usize) -> Point {
        Point::new(Line(line), Column(column))
    }

    #[test]
    fn selection_joins_wrapped_lines() {
        let mut model = TerminalModel::new(10, 4, 100);
        model.feed(b"0123456789abcdef\r\nnext word");
        // Cells: from the first cell to past the wrap, no newline inserted.
        model.start_selection(SelectionKind::Cells, at(0, 0), Side::Left);
        model.update_selection(at(1, 5), Side::Right);
        assert_eq!(model.selected_text(), "0123456789abcdef");
        // Cells across a hard line break keep the newline.
        model.start_selection(SelectionKind::Cells, at(1, 4), Side::Left);
        model.update_selection(at(2, 3), Side::Right);
        assert_eq!(model.selected_text(), "ef\nnext");
        // Line (triple click): the whole logical line, wrapped rows joined.
        model.start_selection(SelectionKind::Line, at(1, 2), Side::Left);
        assert_eq!(model.selected_text(), "0123456789abcdef\n");
        // Word (double click).
        model.start_selection(SelectionKind::Word, at(2, 6), Side::Left);
        assert_eq!(model.selected_text(), "word");
        // Selected cells are flagged for drawing.
        let selected: Vec<(usize, usize)> = model
            .cells()
            .filter(|c| c.selected)
            .map(|c| (c.line, c.column))
            .collect();
        assert_eq!(selected, vec![(2, 5), (2, 6), (2, 7), (2, 8)]);
        model.clear_selection();
        assert_eq!(model.selected_text(), "");
        assert!(model.cells().all(|c| !c.selected));
    }

    #[test]
    fn selection_reaches_into_scrollback() {
        let mut model = TerminalModel::new(10, 3, 100);
        for i in 0..10 {
            model.feed(format!("row {i}\r\n").as_bytes());
        }
        // Screen: row 8, row 9, empty; history: row 0..=7 at lines -8..=-1.
        model.start_selection(SelectionKind::Line, at(-8, 0), Side::Left);
        model.update_selection(at(0, 0), Side::Left);
        let want: String = (0..=8).map(|i| format!("row {i}\n")).collect();
        assert_eq!(model.selected_text(), want);
        // Points beyond the grid are clamped, not a panic.
        model.start_selection(SelectionKind::Cells, at(-500, 0), Side::Left);
        model.update_selection(at(500, 99), Side::Right);
        assert!(model.selected_text().starts_with("row 0\n"));
    }
}
