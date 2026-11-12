//! A list that builds only the rows in view: fixed 30 px rows inside a `scrollable`, with
//! one spacer above and one below standing in for the rows that are not built. This is
//! what keeps a 100 000-entry folder smooth.

use std::ops::Range;

use iced::widget::scrollable::Viewport;
use iced::widget::{Id, column, responsive, scrollable, space};
use iced::{Element, Length};

use crate::ui::theme;

/// Every row is this tall (mockup: `td { height: 30px }`).
pub const ROW_HEIGHT: f32 = 30.0;
/// Rows built above and below the visible ones, so a fast scroll never shows a gap.
pub const OVERSCAN: usize = 8;

/// The rows to build: those intersecting `[offset_y, offset_y + viewport_h)`, widened by
/// `overscan` on each side and clamped to `0..total`. A negative or NaN offset counts as
/// the top; an offset past the end gives an empty range at `total`.
pub fn visible_range(
    total: usize,
    row_height: f32,
    offset_y: f32,
    viewport_h: f32,
    overscan: usize,
) -> Range<usize> {
    if total == 0 || row_height <= 0.0 {
        return 0..0;
    }
    // `max` also turns NaN into 0.
    let top = offset_y.max(0.0);
    let bottom = top + viewport_h.max(0.0);
    let first = (top / row_height).floor() as usize;
    if first >= total {
        return total..total;
    }
    let past_last = ((bottom / row_height).ceil() as usize).min(total);
    first.saturating_sub(overscan)..past_last.saturating_add(overscan).min(total)
}

/// The scrollable list of `total` rows, `row(i)` building row `i`. `offset_y` is the
/// scroll offset last reported by `on_scroll`; the visible height comes from the layout.
pub fn view<'a, Message, R, S>(
    id: Id,
    total: usize,
    offset_y: f32,
    on_scroll: S,
    row: R,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
    R: Fn(usize) -> Element<'a, Message> + 'a,
    S: Fn(Viewport) -> Message + Clone + 'a,
{
    responsive(move |size| {
        // A stale offset (the window grew, the list shrank) never points past the end.
        let max_offset = (total as f32 * ROW_HEIGHT - size.height).max(0.0);
        let offset = offset_y.clamp(0.0, max_offset);
        let range = visible_range(total, ROW_HEIGHT, offset, size.height, OVERSCAN);
        let above = range.start as f32 * ROW_HEIGHT;
        let below = (total - range.end) as f32 * ROW_HEIGHT;
        let rows = column(range.map(&row));
        scrollable(
            column![
                space().height(Length::Fixed(above)),
                rows,
                space().height(Length::Fixed(below)),
            ]
            .width(Length::Fill),
        )
        .id(id.clone())
        .on_scroll(on_scroll.clone())
        .style(theme::scrollbar)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_range_edges() {
        // Empty list: nothing, wherever the offset is.
        assert_eq!(visible_range(0, 30.0, 0.0, 600.0, 8), 0..0);
        assert_eq!(visible_range(0, 30.0, 900.0, 600.0, 8), 0..0);

        // Top, no overscan: exactly the rows that fit (600 / 30 = 20).
        assert_eq!(visible_range(100, 30.0, 0.0, 600.0, 0), 0..20);
        // A partly visible last row is built.
        assert_eq!(visible_range(100, 30.0, 0.0, 610.0, 0), 0..21);
        // Top with overscan: nothing above row 0.
        assert_eq!(visible_range(100, 30.0, 0.0, 600.0, 8), 0..28);

        // Middle, half a row scrolled: rows 10 (partly) to 30 (partly).
        assert_eq!(visible_range(100, 30.0, 315.0, 600.0, 0), 10..31);
        assert_eq!(visible_range(100, 30.0, 315.0, 600.0, 8), 2..39);

        // End: clamped to the total.
        assert_eq!(visible_range(100, 30.0, 2400.0, 600.0, 0), 80..100);
        assert_eq!(visible_range(100, 30.0, 2400.0, 600.0, 8), 72..100);
        // Past the end (stale offset): an empty range at the end, never out of bounds.
        assert_eq!(visible_range(100, 30.0, 9000.0, 600.0, 8), 100..100);

        // Fewer rows than fit: all of them.
        assert_eq!(visible_range(5, 30.0, 0.0, 600.0, 8), 0..5);

        // Negative (elastic scroll) or NaN offsets count as the top.
        assert_eq!(visible_range(100, 30.0, -50.0, 600.0, 0), 0..20);
        assert_eq!(visible_range(100, 30.0, f32::NAN, 600.0, 0), 0..20);
        // No viewport yet: only the overscan.
        assert_eq!(visible_range(100, 30.0, 0.0, 0.0, 8), 0..8);

        // 100 000 rows: a screenful plus overscan, anywhere in the list.
        let total = 100_000;
        let end = total as f32 * 30.0 - 900.0;
        assert_eq!(visible_range(total, 30.0, 0.0, 900.0, 8), 0..38);
        assert_eq!(
            visible_range(total, 30.0, 1_500_000.0, 900.0, 8),
            49_992..50_038
        );
        assert_eq!(visible_range(total, 30.0, end, 900.0, 8), 99_962..100_000);
        for offset in [0.0, 12_345.0, 1_500_000.0, end] {
            let range = visible_range(total, 30.0, offset, 900.0, 8);
            assert!(range.len() <= 30 + 1 + 2 * 8, "{range:?}");
            assert!(range.end <= total);
        }
    }
}
