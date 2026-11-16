//! What a drag between panes looks like: a chip ("ghost") that follows the pointer with
//! the first dragged item's kind icon and name, and a count badge when several items go;
//! over the pane a drop would land in, a dashed outline, a faint wash and a label saying
//! what the drop does (the pane puts [`drop_layer`] over itself, `ui::files`).

use iced::widget::{container, pin, row, stack, text};
use iced::{Alignment, Element, Length, Padding, Point, Size, Vector};

use crate::app::{App, Message};
use crate::ui::dashed::dashed;
use crate::ui::icons::{self, Icon};
use crate::ui::theme::{self, palette};
use crate::ui::{fit, strings};

/// Where the chip sits from the pointer: right of and below it, clear of the cursor.
pub const GHOST_OFFSET: Vector = Vector::new(14.0, 12.0);

/// The widest a dragged name is drawn in the chip before it ends in "…".
pub const GHOST_NAME_WIDTH: f32 = 200.0;

/// The most room the chip takes (border, padding, icon, a 200 px name, "+N" and the
/// badge, with their gaps): what is kept inside the window.
pub const GHOST_SIZE: Size = Size::new(324.0, 36.0);

/// The least room left between the chip and a window edge.
const EDGE: f32 = 4.0;

/// The chip's padding (mockup `.ghost`: 6 px by 10 px) and the drop label's (`.msg`: 7 px
/// by 12 px).
const GHOST_PADDING: Padding = Padding {
    top: 6.0,
    bottom: 6.0,
    left: 10.0,
    right: 10.0,
};
const LABEL_PADDING: Padding = Padding {
    top: 7.0,
    bottom: 7.0,
    left: 12.0,
    right: 12.0,
};
/// How far the drop label sits above the target pane's bottom edge.
const LABEL_LIFT: f32 = 22.0;

/// What the chip shows: the first dragged entry's kind icon and name, and how many more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ghost {
    pub icon: Icon,
    pub name: String,
    pub more: usize,
}

/// The chip of the drag under way, once it moved past the threshold and carries something:
/// the first selected entry (in row order) of the pane it started in.
pub fn ghost(app: &App) -> Option<Ghost> {
    let drag = app.drag.filter(|d| d.active)?;
    let index = app.tab_index(drag.tab)?;
    let entries = app.tabs[index].pane(drag.from)?.selected_entries();
    let first = entries.first()?;
    Some(Ghost {
        icon: icons::for_entry(&first.name, first.kind),
        name: first.name.clone(),
        more: entries.len() - 1,
    })
}

/// Where the chip goes for the pointer at `at` in a `window`: [`GHOST_OFFSET`] from it,
/// moved in so that a chip of [`GHOST_SIZE`] stays 4 px inside every edge (the top left
/// wins when the window is smaller than the chip).
pub fn ghost_origin(at: Point, window: Size) -> Point {
    let axis = |at: f32, len: f32, room: f32| at.min(room - EDGE - len).max(EDGE);
    let at = at + GHOST_OFFSET;
    Point::new(
        axis(at.x, GHOST_SIZE.width, window.width),
        axis(at.y, GHOST_SIZE.height, window.height),
    )
}

/// `base` with the chip over it while a drag is under way.
///
/// Always a `stack` with `base` first, as `menu::overlay`, so nothing behind loses its
/// state when a drag starts or ends. The chip's layer takes no events: it holds only text
/// (no widget in it captures an event or asks for a cursor, so the panes under the pointer
/// keep getting both).
pub fn overlay<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    let (Some(ghost), Some(drag)) = (ghost(app), app.drag) else {
        return stack![base].into();
    };
    let window = Size::new(app.window_width, app.window_height);
    stack![
        base,
        pin(chip(ghost)).position(ghost_origin(drag.at, window))
    ]
    .into()
}

/// The chip: kind icon, the name in mono (cut to [`GHOST_NAME_WIDTH`]), then "+N" and a
/// badge with the whole count when several items go.
fn chip<'a>(ghost: Ghost) -> Element<'a, Message> {
    let p = palette();
    let name = fit::ellipsize(
        &ghost.name,
        GHOST_NAME_WIDTH,
        theme::TEXT_MONO,
        fit::MONO_ADVANCE,
    )
    .into_owned();
    let mut line = row![
        icons::icon(ghost.icon, icons::ROW).color(p.ink),
        text(name)
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .color(p.ink)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    if ghost.more > 0 {
        line = line
            .push(
                text(strings::drag_more(ghost.more))
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .color(p.muted)
                    .wrapping(text::Wrapping::None),
            )
            .push(
                container(
                    text(strings::drag_count(ghost.more + 1))
                        .size(theme::TEXT_TAG)
                        .wrapping(text::Wrapping::None),
                )
                .padding(Padding::ZERO.left(6.0).right(6.0))
                .style(theme::drag_badge),
            );
    }
    container(line)
        .padding(GHOST_PADDING)
        .style(theme::drag_chip)
        .into()
}

/// `label` split around the last place `folder` appears in it: the words before, the
/// folder, the words after (trimmed at the joins). `None` for the folder when the label
/// does not hold it (a translation that rewrote it): all of it is drawn as words.
pub fn split_label<'s>(label: &'s str, folder: &str) -> (&'s str, Option<&'s str>, &'s str) {
    match label.rfind(folder).filter(|_| !folder.is_empty()) {
        Some(at) => (
            label[..at].trim_end(),
            Some(&label[at..at + folder.len()]),
            label[at + folder.len()..].trim_start(),
        ),
        None => (label, None, ""),
    }
}

/// What goes over the target pane of a drop: a 1 px dashed `ink_2` outline, a faint wash,
/// and near the bottom the label (`upload`: the upload icon, else the download one) with
/// the folder in mono, cut with "…" to the pane's `width`. It takes no events.
pub fn drop_layer<'a>(label: &str, folder: &str, upload: bool, width: f32) -> Element<'a, Message> {
    let p = palette();
    let (before, folder, after) = split_label(label, folder);
    let words = |s: &str| {
        text(s.to_owned())
            .size(theme::TEXT_SM)
            .color(p.ink)
            .wrapping(text::Wrapping::None)
    };
    let estimate = |s: &str| s.chars().count() as f32 * theme::TEXT_SM * fit::INTER_ADVANCE;
    let icon = if upload {
        Icon::UploadSimple
    } else {
        Icon::DownloadSimple
    };
    let mut line = row![icons::icon(icon, icons::ROW).color(p.ink), words(before)]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center);
    if let Some(folder) = folder {
        // What the pane leaves the folder: its width less the label's margins, border,
        // padding, icon, words and gaps.
        let room = width
            - 2.0 * theme::SPACE_MD
            - 2.0 * theme::HAIRLINE
            - LABEL_PADDING.left
            - LABEL_PADDING.right
            - icons::ROW
            - 3.0 * theme::SPACE_XS
            - estimate(before)
            - estimate(after);
        let shown = fit::ellipsize(folder, room.max(0.0), theme::TEXT_XS, fit::MONO_ADVANCE);
        line = line.push(
            text(shown.into_owned())
                .font(theme::MONO)
                .size(theme::TEXT_XS)
                .color(p.ink)
                .wrapping(text::Wrapping::None),
        );
    }
    if !after.is_empty() {
        line = line.push(words(after));
    }
    let label = container(line)
        .padding(LABEL_PADDING)
        .style(theme::drag_chip)
        .clip(true);
    let wash = container(label)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(
            Padding::ZERO
                .bottom(LABEL_LIFT)
                .left(theme::SPACE_MD)
                .right(theme::SPACE_MD),
        )
        .align_x(Alignment::Center)
        .align_y(Alignment::End)
        .style(theme::drop_wash);
    dashed(wash, Some(p.ink_2))
}

#[cfg(test)]
mod tests {
    use iced::{Point, Size};

    use super::*;
    use crate::app::tests::{pane_msg, row_of, transfer_tab};
    use crate::app::{Message, PaneMsg};
    use crate::backend::Side;
    use crate::ui::icons;
    use hoprs_core::EntryKind;

    const IN_LOCAL: Point = Point::new(100.0, 200.0);

    fn press(app: &mut App, tab: crate::app::TabId, name: &str) {
        let row = row_of(app, Side::Local, name);
        app.update(Message::PointerPressed(IN_LOCAL));
        app.update(pane_msg(tab, Side::Local, PaneMsg::Click(row)));
    }

    #[tokio::test]
    async fn ghost_for_one_and_several() {
        let (_fake, mut app, _a, tab) = transfer_tab().await;
        press(&mut app, tab, "b.txt");
        app.update(Message::PointerMoved(Point::new(300.0, 200.0)));
        assert_eq!(
            ghost(&app),
            Some(Ghost {
                icon: icons::for_entry("b.txt", EntryKind::File),
                name: "b.txt".into(),
                more: 0,
            })
        );
        app.update(Message::DragCancel);

        // Three selected (Ctrl+click adds a.txt and dir): the first one leads, two more.
        app.mods.ctrl = true;
        press(&mut app, tab, "a.txt");
        app.update(Message::DragCancel);
        press(&mut app, tab, "dir");
        app.mods.ctrl = false;
        app.update(Message::PointerMoved(Point::new(300.0, 200.0)));
        assert_eq!(app.drag_count(), 3);
        let g = ghost(&app).expect("a ghost");
        assert_eq!(g.more, 2);
        let first = app.tabs[0].pane(Side::Local).unwrap().selected_entries()[0];
        assert_eq!(g.name, first.name);
        assert_eq!(g.icon, icons::for_entry(&first.name, first.kind));
        // What the chip writes after the name: "+2" and the badge "3".
        assert_eq!(
            (strings::drag_more(g.more), strings::drag_count(g.more + 1)),
            ("+2".to_owned(), "3".to_owned())
        );
        app.update(Message::DragCancel);
        assert_eq!(ghost(&app), None);
    }

    #[tokio::test]
    async fn no_ghost_before_threshold() {
        let (_fake, mut app, _a, tab) = transfer_tab().await;
        press(&mut app, tab, "b.txt");
        assert_eq!(ghost(&app), None, "a press is not a drag");
        app.update(Message::PointerMoved(Point::new(
            IN_LOCAL.x + 3.0,
            IN_LOCAL.y,
        )));
        assert_eq!(ghost(&app), None, "3 px is still a click");
        app.update(Message::PointerMoved(Point::new(
            IN_LOCAL.x + 10.0,
            IN_LOCAL.y,
        )));
        assert!(ghost(&app).is_some());
    }

    #[test]
    fn label_splits_around_the_folder() {
        assert_eq!(
            split_label("Download 2 items to ~/Downloads", "~/Downloads"),
            ("Download 2 items to", Some("~/Downloads"), "")
        );
        // The folder in the middle (another language's word order) and the last match.
        assert_eq!(
            split_label("Send /a to /a now", "/a"),
            ("Send /a to", Some("/a"), "now")
        );
        assert_eq!(split_label("Upload", "/srv"), ("Upload", None, ""));
        assert_eq!(split_label("Upload", ""), ("Upload", None, ""));
    }

    #[test]
    fn ghost_stays_inside_window() {
        let window = Size::new(1280.0, 800.0);
        // Room around the pointer: right of and below it by the offset.
        assert_eq!(
            ghost_origin(Point::new(100.0, 100.0), window),
            Point::new(100.0, 100.0) + GHOST_OFFSET
        );
        // Near the bottom right corner: the whole chip stays inside.
        let o = ghost_origin(Point::new(1275.0, 795.0), window);
        assert!(
            o.x >= 0.0 && o.x + GHOST_SIZE.width <= window.width,
            "{o:?}"
        );
        assert!(
            o.y >= 0.0 && o.y + GHOST_SIZE.height <= window.height,
            "{o:?}"
        );
        // Past the top left (a pointer just leaving): clamped in.
        let o = ghost_origin(Point::new(-50.0, -50.0), window);
        assert!(o.x >= 0.0 && o.y >= 0.0, "{o:?}");
        // A window smaller than the chip: pinned to the top left.
        let o = ghost_origin(Point::new(60.0, 30.0), Size::new(100.0, 20.0));
        assert!(o.x >= 0.0 && o.y >= 0.0, "{o:?}");
    }
}
