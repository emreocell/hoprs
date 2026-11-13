//! Plain-text search over the whole grid, scrollback included.

use std::ops::RangeInclusive;

use alacritty_terminal::index::{Direction, Point, Side};
use alacritty_terminal::term::search::RegexSearch;

use super::TerminalModel;

/// A match: its first and last cell, inclusive (grid points; negative
/// lines are scrollback).
pub type Range = RangeInclusive<Point>;

/// Finds `query` as literal text, ignoring case, across the scrollback and
/// the screen; matches may span wrapped rows.
///
/// Forward, it returns the first match starting at or after `from`;
/// backwards, the last match starting at or before `from`. Both wrap around
/// the grid. To step through matches, pass the cell after (or before) the
/// current match's start. `None` for an empty query or no match.
// used by the terminal tab (Plan 6 Task 4)
#[cfg_attr(not(test), allow(dead_code))]
pub fn find(model: &TerminalModel, query: &str, from: Point, backwards: bool) -> Option<Range> {
    if query.is_empty() {
        return None;
    }
    let pattern = format!("(?i){}", escape(query));
    let mut regex = RegexSearch::new(&pattern).ok()?;
    let direction = if backwards {
        Direction::Left
    } else {
        Direction::Right
    };
    let origin = model.clamp(from);
    model
        .term
        .search_next(&mut regex, origin, direction, Side::Left, None)
}

/// Escapes regex metacharacters (regex-syntax's set), so `query` is literal.
fn escape(query: &str) -> String {
    const META: &str = "\\.+*?()|[]{}^$#&-~";
    let mut out = String::with_capacity(query.len() * 2);
    for c in query.chars() {
        if META.contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::index::{Column, Line};

    fn at(line: i32, column: usize) -> Point {
        Point::new(Line(line), Column(column))
    }

    #[test]
    fn search_finds_in_scrollback_both_directions() {
        let mut model = TerminalModel::new(20, 5, 1_000);
        model.feed(b"needle one\r\n");
        for i in 0..30 {
            model.feed(format!("filler {i}\r\n").as_bytes());
        }
        model.feed(b"the Needle two\r\n");
        for i in 0..10 {
            model.feed(format!("filler {i}\r\n").as_bytes());
        }
        // 42 lines written + the empty prompt line, 5 on screen: 38 in history.
        assert_eq!(model.history_len(), 38);
        let first = at(-38, 0)..=at(-38, 5);
        let second = at(-7, 4)..=at(-7, 9);
        let top = at(-38, 0);
        assert_eq!(model.grid_bounds().0, top);
        let bottom = at(4, 19);

        // Forward from the top, case-insensitive.
        assert_eq!(find(&model, "NEEDLE", top, false), Some(first.clone()));
        // The origin is inclusive; one cell later finds the next match.
        assert_eq!(
            find(&model, "needle", at(-38, 1), false),
            Some(second.clone())
        );
        // Backwards from the bottom.
        assert_eq!(find(&model, "needle", bottom, true), Some(second.clone()));
        assert_eq!(find(&model, "Needle", at(-7, 3), true), Some(first.clone()));
        // Both directions wrap around.
        assert_eq!(
            find(&model, "needle", at(-6, 0), false),
            Some(first.clone())
        );
        assert_eq!(
            find(&model, "needle", at(-38, 0), true),
            Some(first.clone())
        );
        assert_eq!(
            find(&model, "needle", at(-37, 0), true),
            Some(first.clone())
        );
        assert_eq!(find(&model, "needle", at(-38, 4), false), Some(second));
        // An origin outside the grid is clamped to it.
        assert_eq!(find(&model, "needle", at(-500, 0), false), Some(first));
        // Regex metacharacters are literal.
        model.feed(b"a.b (c) [d]*");
        assert_eq!(
            find(&model, "(C) [d]*", top, false),
            Some(at(4, 4)..=at(4, 11))
        );
        assert_eq!(find(&model, "a?b", top, false), None);
        assert_eq!(find(&model, "absent", top, false), None);
        assert_eq!(find(&model, "", top, false), None);
    }
}
