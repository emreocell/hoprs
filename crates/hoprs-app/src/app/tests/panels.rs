//! Plan 13 Task 11: the compare panel and the queue resize by dragging their top edge
//! (a double-click resets them); the heights are saved.

use iced::Point;

use super::*;
use crate::ui::virtual_list::ROW_HEIGHT;
use crate::ui::{compare, queue};

/// The settings a lone save effect carries.
fn saved(effects: &[Effect]) -> Settings {
    match effects {
        [Effect::SaveSettings(settings)] => settings.clone(),
        other => panic!("expected one SaveSettings, got {other:?}"),
    }
}

/// Presses the `panel`'s divider at `y`, drawn `height` px tall.
fn press(app: &mut App, panel: Panel, y: f32, height: f32) {
    assert!(
        app.update(Message::PointerPressed(Point::new(500.0, y)))
            .is_empty()
    );
    assert!(
        app.update(Message::Divider(DividerMsg::Press { panel, height }))
            .is_empty()
    );
}

fn move_to(app: &mut App, y: f32) -> Vec<Effect> {
    app.update(Message::PointerMoved(Point::new(500.0, y)))
}

#[test]
fn divider_drag_changes_and_saves_height() {
    let mut app = App::new();
    press(&mut app, Panel::Compare, 600.0, 270.0);
    assert_eq!(
        app.resize,
        Some(Resize {
            panel: Panel::Compare,
            start_y: 600.0,
            start_height: 270.0,
        })
    );
    // Up 100 px: 100 px taller, not saved while the button is down.
    assert!(move_to(&mut app, 500.0).is_empty());
    assert_eq!(app.settings.compare_height, Some(370.0));
    // A fraction of a pixel is rounded: saved heights stay whole pixels.
    assert!(move_to(&mut app, 499.6).is_empty());
    assert_eq!(app.settings.compare_height, Some(370.0));
    let effects = app.update(Message::DragEnd);
    assert_eq!(saved(&effects).compare_height, Some(370.0));
    assert_eq!(app.resize, None);

    // The queue, the same way; the pointer leaving the window ends it and keeps the value.
    let _ = app.update(Message::SettingsSaved(Ok(())));
    press(&mut app, Panel::Queue, 700.0, 150.0);
    assert!(move_to(&mut app, 650.0).is_empty());
    assert_eq!(app.settings.queue_height, 200.0);
    let effects = app.update(Message::DragCancel);
    assert_eq!(saved(&effects).queue_height, 200.0);
    assert_eq!(app.resize, None);
}

#[test]
fn divider_drag_clamps_to_minimum() {
    let mut app = App::new();
    press(&mut app, Panel::Compare, 600.0, 270.0);
    assert!(move_to(&mut app, 1600.0).is_empty());
    assert_eq!(app.settings.compare_height, Some(compare::MIN_HEIGHT));
    let _ = app.update(Message::DragEnd);
    let _ = app.update(Message::SettingsSaved(Ok(())));
    press(&mut app, Panel::Queue, 600.0, 150.0);
    assert!(move_to(&mut app, 1600.0).is_empty());
    assert_eq!(app.settings.queue_height, queue::MIN_HEIGHT);
}

#[test]
fn double_click_resets() {
    let mut app = App::new();
    app.settings.compare_height = Some(400.0);
    app.settings.queue_height = 300.0;
    let effects = app.update(Message::Divider(DividerMsg::Reset(Panel::Compare)));
    assert_eq!(app.settings.compare_height, None);
    assert_eq!(saved(&effects).compare_height, None);
    let _ = app.update(Message::SettingsSaved(Ok(())));
    let effects = app.update(Message::Divider(DividerMsg::Reset(Panel::Queue)));
    assert_eq!(
        app.settings.queue_height,
        crate::ui::layout::QUEUE_MAX_HEIGHT
    );
    assert_eq!(
        saved(&effects).queue_height,
        crate::ui::layout::QUEUE_MAX_HEIGHT
    );
    // A double-click is also a press: the reset ends the resize it started.
    let _ = app.update(Message::SettingsSaved(Ok(())));
    press(&mut app, Panel::Queue, 600.0, 150.0);
    let _ = app.update(Message::Divider(DividerMsg::Reset(Panel::Queue)));
    assert_eq!(app.resize, None);
    assert!(!app.tracks_pointer());
}

#[test]
fn heights_survive_restart() {
    let mut app = App::new();
    press(&mut app, Panel::Compare, 600.0, 270.0);
    let _ = move_to(&mut app, 559.0);
    let _ = app.update(Message::DragEnd);
    let _ = app.update(Message::SettingsSaved(Ok(())));
    press(&mut app, Panel::Queue, 700.0, 150.0);
    let _ = move_to(&mut app, 600.0);
    let settings = saved(&app.update(Message::DragEnd));
    assert_eq!(settings.compare_height, Some(311.0));
    assert_eq!(settings.queue_height, 250.0);
    let back = crate::settings::parse(&crate::settings::render(&settings)).unwrap();
    assert_eq!(back.compare_height, Some(311.0));
    assert_eq!(back.queue_height, 250.0);
}

/// The queue's saved height is its maximum: a taller one does not grow a queue of two
/// rows, a shorter one cuts it.
#[test]
fn queue_divider_sets_the_maximum() {
    use super::transfers::{item, synced};
    use hoprs_transfer::TransferState;
    let mut app = App::new();
    let _ = app.update(Message::WindowResized(iced::Size::new(1280.0, 1200.0)));
    let site = SiteId::new();
    synced(
        &mut app,
        1,
        vec![
            item(1, &site, TransferState::Active),
            item(2, &site, TransferState::Active),
        ],
    );
    let two_rows = queue::MIN_HEIGHT + ROW_HEIGHT;
    assert_eq!(queue::drawn_height(&app), two_rows);
    press(&mut app, Panel::Queue, 700.0, two_rows);
    let _ = move_to(&mut app, 400.0);
    let _ = app.update(Message::DragEnd);
    assert_eq!(app.settings.queue_height, two_rows + 300.0);
    assert_eq!(queue::max_height(&app), two_rows + 300.0);
    assert_eq!(queue::drawn_height(&app), two_rows);
    // Dragged below its rows: the queue is cut to the saved height.
    let _ = app.update(Message::SettingsSaved(Ok(())));
    press(&mut app, Panel::Queue, 700.0, two_rows);
    let _ = move_to(&mut app, 715.0);
    assert_eq!(app.settings.queue_height, two_rows - 15.0);
    assert_eq!(queue::drawn_height(&app), two_rows - 15.0);
}

#[test]
fn pointer_tracked_only_while_resizing() {
    let mut app = App::new();
    assert!(!app.tracks_pointer());
    press(&mut app, Panel::Compare, 600.0, 270.0);
    assert!(app.tracks_pointer());
    let _ = app.update(Message::DragEnd);
    assert!(!app.tracks_pointer());
}

#[test]
fn pointer_moved_without_resize_is_a_no_op() {
    let mut app = App::new();
    let before = app.settings.clone();
    assert!(move_to(&mut app, 100.0).is_empty());
    assert_eq!(app.settings, before);
    // A drag between panes is not a resize: its end saves nothing.
    assert!(app.update(Message::DragEnd).is_empty());
}
