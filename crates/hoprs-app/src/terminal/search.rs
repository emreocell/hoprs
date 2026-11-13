//! Plain-text search over the whole grid, scrollback included.

use std::ops::RangeInclusive;

use alacritty_terminal::index::{Direction, Point, Side};
use alacritty_terminal::term::search::RegexSearch;

use super::TerminalModel;

/// A match: its first and last cell, inclusive (grid points; negative
/// lines are scrollback).
pub type Range = RangeInclusive<Point>;

/// Finds a query as literal text, ignoring case, across the scrollback and
/// the screen; matches may span wrapped rows. The compiled matcher is kept
/// for the query (search-as-you-type compiles once per query, not once per
/// step).
///
/// Forward, [`find`](Self::find) returns the first match starting at or
/// after `from`; backwards, the last match starting at or before `from`.
/// Both wrap around the grid. To step through matches, pass the cell after
/// (or before) the current match's start. `None` for an empty query or no
/// match.
#[derive(Default)]
pub struct Searcher {
    query: String,
    /// `None` for an empty query (or one that cannot compile).
    regex: Option<RegexSearch>,
    /// How many times a matcher was compiled.
    #[cfg(test)]
    compiles: usize,
}

impl std::fmt::Debug for Searcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Searcher")
            .field("query", &self.query)
            .finish_non_exhaustive()
    }
}

impl Searcher {
    /// The query searched for.
    #[cfg(test)]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Searches for `query` from now on; compiles only when it changed.
    pub fn set_query(&mut self, query: &str) {
        // Same query: same matcher, even one that failed to compile (it would fail again).
        if query == self.query {
            return;
        }
        query.clone_into(&mut self.query);
        self.regex = if query.is_empty() {
            None
        } else {
            #[cfg(test)]
            {
                self.compiles += 1;
            }
            RegexSearch::new(&format!("(?i){}", escape(query))).ok()
        };
    }

    /// The next match of the query from `from` (see the type docs).
    pub fn find(&mut self, model: &TerminalModel, from: Point, backwards: bool) -> Option<Range> {
        let regex = self.regex.as_mut()?;
        let direction = if backwards {
            Direction::Left
        } else {
            Direction::Right
        };
        let origin = model.clamp(from);
        model
            .term
            .search_next(regex, origin, direction, Side::Left, None)
    }
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

    /// One search with a fresh matcher.
    fn find(model: &TerminalModel, query: &str, from: Point, backwards: bool) -> Option<Range> {
        let mut searcher = Searcher::default();
        searcher.set_query(query);
        searcher.find(model, from, backwards)
    }

    #[test]
    fn searcher_compiles_once_per_query() {
        let mut model = TerminalModel::new(20, 5, 100);
        model.feed(b"one two one\r\n");
        let (top, _) = model.grid_bounds();
        let mut searcher = Searcher::default();
        searcher.set_query("one");
        let first = searcher.find(&model, top, false).unwrap();
        let next = searcher.find(&model, at(0, 1), false).unwrap();
        assert_eq!((*first.start(), *next.start()), (at(0, 0), at(0, 8)));
        searcher.set_query("one");
        assert_eq!(searcher.compiles, 1, "the same query reuses its matcher");
        searcher.set_query("two");
        assert_eq!(searcher.find(&model, top, false), Some(at(0, 4)..=at(0, 6)));
        assert_eq!(searcher.compiles, 2);
        searcher.set_query("");
        assert_eq!(searcher.find(&model, top, false), None);
        assert_eq!(searcher.compiles, 2, "an empty query compiles nothing");
        // A query too big to compile fails once, not on every step.
        let huge = "ab".repeat(100_000);
        searcher.set_query(&huge);
        assert!(searcher.regex.is_none(), "too big to compile");
        assert_eq!(searcher.find(&model, top, false), None);
        searcher.set_query(&huge);
        assert_eq!(searcher.compiles, 3, "a failed compile is cached too");
        assert!(searcher.query() == huge);
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

    #[test]
    fn search_spans_wide_chars_and_wrapped_rows() {
        let mut model = TerminalModel::new(10, 4, 100);
        // `中文` takes columns 2-5; the match ends on the wide cell.
        model.feed("ab中文cd\r\n".as_bytes());
        let (top, _) = model.grid_bounds();
        assert_eq!(
            find(&model, "b中文c", top, false),
            Some(at(0, 1)..=at(0, 6))
        );
        assert_eq!(find(&model, "中", top, false), Some(at(0, 2)..=at(0, 3)));
        // `wrapped` starts at column 7 and continues on the next row.
        model.feed(b"xxxxxxxwrapped\r\n");
        assert_eq!(
            find(&model, "WRAPPED", top, false),
            Some(at(1, 7)..=at(2, 3))
        );
        assert_eq!(
            find(&model, "wrapped", at(3, 0), true),
            Some(at(1, 7)..=at(2, 3))
        );
    }
}
