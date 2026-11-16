//! Where a widget is: iced tells a list's position only through `scrollable::on_scroll`,
//! and only while its content overflows (a folder that fits never reports it). [`report`]
//! wraps any content and publishes its layout bounds, in window coordinates, whenever they
//! differ from what the app last stored (`known`), overflow or not: at the first frame, after
//! a resize, when a bar above it comes or goes. A menu opened from the keyboard goes there
//! (the file panes; the compare list can use it the same way). Otherwise it is transparent:
//! same tree, layout, events and drawing as its content.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget, overlay};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};

/// `content`, publishing `on_change(bounds)` when its bounds are not `known`.
pub fn report<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    known: Rectangle,
    on_change: impl Fn(Rectangle) -> Message + 'a,
) -> Element<'a, Message> {
    Element::new(Report {
        content: content.into(),
        known,
        on_change: Box::new(on_change),
    })
}

struct Report<'a, Message> {
    content: Element<'a, Message>,
    known: Rectangle,
    on_change: Box<dyn Fn(Rectangle) -> Message + 'a>,
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Report<'_, Message> {
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
        // Any event will do (a redraw comes with every frame); once the app stored the
        // bounds, `known` matches and nothing more is sent.
        let bounds = layout.bounds();
        if bounds != self.known {
            shell.publish((self.on_change)(bounds));
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
    use iced::widget::{column, space};
    use iced::{Point, window};
    use iced_runtime::user_interface::{Cache, UserInterface};

    use super::*;
    use crate::app::Message;
    use crate::ui;

    fn renderer() -> iced::Renderer {
        iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
            ui::theme::INTER,
            ui::theme::TEXT_SM.into(),
        ))
    }

    /// What one frame publishes for content 40 px below the top of a `window`, given the
    /// bounds the app knows.
    fn frame(window: Size, known: Rectangle) -> Vec<String> {
        let mut renderer = renderer();
        let content = column![
            space().height(40),
            report(
                space().width(Length::Fill).height(Length::Fill),
                known,
                |b: Rectangle| Message::WindowResized(Size::new(b.x + b.width, b.y + b.height)),
            ),
        ];
        let mut ui = UserInterface::build(content, window, Cache::new(), &mut renderer);
        let mut messages = Vec::new();
        ui.update(
            &[Event::Window(window::Event::RedrawRequested(
                std::time::Instant::now(),
            ))],
            mouse::Cursor::Unavailable,
            &mut renderer,
            &mut clipboard::Null,
            &mut messages,
        );
        messages.iter().map(|m| format!("{m:?}")).collect()
    }

    #[test]
    fn publishes_bounds_until_known() {
        let window = Size::new(200.0, 300.0);
        let bounds = Rectangle::new(Point::new(0.0, 40.0), Size::new(200.0, 260.0));
        // Nothing known yet (content that never scrolls): its bounds are sent.
        assert_eq!(
            frame(window, Rectangle::default()),
            [format!(
                "{:?}",
                Message::WindowResized(Size::new(200.0, 300.0))
            )]
        );
        // Known: nothing.
        assert!(frame(window, bounds).is_empty());
        // The window grew: the new bounds are sent.
        assert_eq!(
            frame(Size::new(250.0, 400.0), bounds),
            [format!(
                "{:?}",
                Message::WindowResized(Size::new(250.0, 400.0))
            )]
        );
    }
}
