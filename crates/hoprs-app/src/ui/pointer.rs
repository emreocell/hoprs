//! Press positions: `mouse_area` tells a press happened, not where. [`tracker`] wraps the
//! whole window and publishes [`Message::PointerPressed`] with the cursor's position for
//! every left or right press, **before** the press reaches the widgets inside, so the
//! message a button or row sends for the same press finds [`crate::app::App::last_press`]
//! already set (a menu opens there; drags and resizes start there). It never captures:
//! the press goes on to the content as if the tracker were not there.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget, overlay};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};

use crate::app::Message;

/// `content`, with its left and right presses published as [`Message::PointerPressed`].
pub fn tracker<'a>(content: Element<'a, Message>) -> Element<'a, Message> {
    Element::new(PressTracker { content })
}

/// The widget behind [`tracker`]: transparent (same tree, layout and drawing as its
/// content) except for the message it publishes.
pub struct PressTracker<'a> {
    content: Element<'a, Message>,
}

impl Widget<Message, iced::Theme, iced::Renderer> for PressTracker<'_> {
    fn tag(&self) -> tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.content.as_widget().diff(tree);
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
        self.content.as_widget_mut().layout(tree, renderer, limits)
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
            .operate(tree, layout, renderer, operation);
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
        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
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
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
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
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

#[cfg(test)]
mod tests {
    use iced::advanced::clipboard;
    use iced::widget::{mouse_area, space};
    use iced::{Event, Point, Size, mouse};
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
}
