//! Press positions: `mouse_area` tells a press happened, not where. [`tracker`] wraps the
//! whole window (its own tree node, the content its only child) and publishes [`Message::PointerPressed`] with the cursor's position for
//! every left or right press on the base widget tree, **before** the press reaches the
//! widgets inside. iced's own overlays (tooltips, pick_list dropdowns) are updated first
//! and may capture a press, which then is not published. On the base tree, the
//! message a button or row sends for the same press finds [`crate::app::App::last_press`]
//! already set (a menu opens there; drags and resizes start there). It never captures:
//! the press goes on to the content as if the tracker were not there. Moves come from
//! [`moved_message`], subscribed to only while something follows the pointer.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget, overlay};
use iced::event::Status;
use iced::{Element, Event, Length, Rectangle, Size, Vector, keyboard, mouse, window};

use crate::app::Message;

/// The pointer's moves, as [`Message::PointerMoved`]: listened to only while
/// [`crate::app::App::tracks_pointer`] (a divider is dragged), so they cost nothing else.
pub fn moved_message(event: Event, _: Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            Some(Message::PointerMoved(position))
        }
        _ => None,
    }
}

/// `content`, with its left and right presses published as [`Message::PointerPressed`].
/// On macOS a Control+click is a right click (winit reports a left press with Control
/// held), so the content sees it as one.
pub fn tracker<'a>(content: Element<'a, Message>) -> Element<'a, Message> {
    Element::new(PressTracker::new(content, cfg!(target_os = "macos")))
}

/// The widget behind [`tracker`]: transparent (same layout and drawing as its content)
/// except for the message it publishes and, with `control_click_is_right`, the
/// Control+left presses it turns into right ones.
pub struct PressTracker<'a> {
    content: Element<'a, Message>,
    control_click_is_right: bool,
}

impl<'a> PressTracker<'a> {
    /// `control_click_is_right`: a left press with Control held (and its release) reach
    /// the content as a right one (the macOS convention; set by [`tracker`] there only).
    pub fn new(content: Element<'a, Message>, control_click_is_right: bool) -> Self {
        Self {
            content,
            control_click_is_right,
        }
    }
}

/// What [`PressTracker`] remembers between events.
#[derive(Debug, Default)]
struct State {
    /// Control is held (from `ModifiersChanged`).
    control: bool,
    /// The left button went down with Control: its release is a right release too.
    translating: bool,
}

impl Widget<Message, iced::Theme, iced::Renderer> for PressTracker<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left | mouse::Button::Right)) =
            event
            && let Some(position) = cursor.position()
        {
            shell.publish(Message::PointerPressed(position));
        }
        let state = tree.state.downcast_mut::<State>();
        let translated;
        let event = if self.control_click_is_right {
            match event {
                Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                    state.control = modifiers.control();
                    event
                }
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if state.control => {
                    state.translating = true;
                    translated = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right));
                    &translated
                }
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                    if state.translating =>
                {
                    state.translating = false;
                    translated = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right));
                    &translated
                }
                _ => event,
            }
        } else {
            event
        };
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use iced::advanced::clipboard;
    use iced::widget::{mouse_area, space};
    use iced::{Event, Point, Size, keyboard, mouse};
    use iced_runtime::user_interface::{Cache, UserInterface};

    use super::*;
    use crate::ui;

    #[test]
    fn tracker_publishes_position_before_children() {
        let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
            ui::theme::INTER,
            ui::theme::TEXT_SM.into(),
        ));
        let content = mouse_area(space().width(100).height(100)).on_press(Message::NewSite);
        let mut ui = UserInterface::build(
            tracker(content.into()),
            Size::new(200.0, 200.0),
            Cache::new(),
            &mut renderer,
        );
        let at = Point::new(20.0, 30.0);
        let mut messages = Vec::new();
        ui.update(
            &[
                Event::Mouse(mouse::Event::CursorMoved { position: at }),
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            ],
            mouse::Cursor::Available(at),
            &mut renderer,
            &mut clipboard::Null,
            &mut messages,
        );
        let got: Vec<String> = messages.iter().map(|m| format!("{m:?}")).collect();
        assert_eq!(
            got,
            [
                format!("{:?}", Message::PointerPressed(at)),
                format!("{:?}", Message::NewSite),
            ]
        );
        // A right press is published too; a middle press is not.
        let mut messages = Vec::new();
        ui.update(
            &[
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle)),
            ],
            mouse::Cursor::Available(Point::new(150.0, 150.0)),
            &mut renderer,
            &mut clipboard::Null,
            &mut messages,
        );
        let got: Vec<String> = messages.iter().map(|m| format!("{m:?}")).collect();
        assert_eq!(
            got,
            [format!(
                "{:?}",
                Message::PointerPressed(Point::new(150.0, 150.0))
            )]
        );
    }

    /// The messages `events` make on a tracker (Control+click translation on or off)
    /// over a mouse area that tells left (NewSite / NewWorkspace on press / release)
    /// from right (ToggleQueue / ReloadProfiles).
    fn control_click(control_click_is_right: bool, events: &[Event]) -> Vec<String> {
        let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
            ui::theme::INTER,
            ui::theme::TEXT_SM.into(),
        ));
        let content = mouse_area(space().width(100).height(100))
            .on_press(Message::NewSite)
            .on_release(Message::NewWorkspace)
            .on_right_press(Message::ToggleQueue)
            .on_right_release(Message::ReloadProfiles);
        let mut ui = UserInterface::build(
            Element::new(PressTracker::new(content.into(), control_click_is_right)),
            Size::new(200.0, 200.0),
            Cache::new(),
            &mut renderer,
        );
        let at = Point::new(20.0, 30.0);
        let mut messages = Vec::new();
        let mut all = vec![Event::Mouse(mouse::Event::CursorMoved { position: at })];
        all.extend_from_slice(events);
        for event in all {
            ui.update(
                &[event],
                mouse::Cursor::Available(at),
                &mut renderer,
                &mut clipboard::Null,
                &mut messages,
            );
        }
        messages.iter().map(|m| format!("{m:?}")).collect()
    }

    fn modifiers(modifiers: keyboard::Modifiers) -> Event {
        Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers))
    }

    fn left(pressed: bool) -> Event {
        Event::Mouse(if pressed {
            mouse::Event::ButtonPressed(mouse::Button::Left)
        } else {
            mouse::Event::ButtonReleased(mouse::Button::Left)
        })
    }

    #[test]
    fn control_click_is_a_right_click_when_asked() {
        let pressed = format!("{:?}", Message::PointerPressed(Point::new(20.0, 30.0)));
        let dbg = |m: Message| format!("{m:?}");
        // Translation on (macOS): Control+left press and its release reach the content
        // as right ones, and the press is still published.
        let got = control_click(
            true,
            &[
                modifiers(keyboard::Modifiers::CTRL),
                left(true),
                // Control let go before the button: the release still matches the press.
                modifiers(keyboard::Modifiers::empty()),
                left(false),
                // A plain click afterwards is left again.
                left(true),
                left(false),
            ],
        );
        assert_eq!(
            got,
            [
                pressed.clone(),
                dbg(Message::ToggleQueue),
                dbg(Message::ReloadProfiles),
                pressed.clone(),
                dbg(Message::NewSite),
                dbg(Message::NewWorkspace),
            ]
        );
        // Translation off (Linux, Windows): Control+click stays a left click.
        let got = control_click(
            false,
            &[
                modifiers(keyboard::Modifiers::CTRL),
                left(true),
                left(false),
            ],
        );
        assert_eq!(
            got,
            [pressed, dbg(Message::NewSite), dbg(Message::NewWorkspace)]
        );
    }

    #[test]
    fn moves_become_pointer_moved() {
        let at = Point::new(12.0, 34.0);
        let id = iced::window::Id::unique();
        let moved = moved_message(
            Event::Mouse(mouse::Event::CursorMoved { position: at }),
            Status::Ignored,
            id,
        );
        assert_eq!(
            format!("{moved:?}"),
            format!("{:?}", Some(Message::PointerMoved(at)))
        );
        let pressed = moved_message(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            Status::Ignored,
            id,
        );
        assert!(pressed.is_none());
    }
}
