//! A 1 px dashed outline drawn over a widget's bounds (iced 0.14 borders cannot dash).
//! It serves the disabled outline button and the drop target outline.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Renderer as _, Shell, Widget, overlay};
use iced::{Border, Color, Element, Event, Length, Rectangle, Size, Vector, mouse};

use crate::app::Message;

/// Length of a dash.
pub const DASH_ON: f32 = 4.0;
/// Length of the gap between two dashes.
pub const DASH_OFF: f32 = 3.0;

/// The dashes along a line of `length`: `(start, len)`, the last one clipped to the end.
pub fn dash_runs(length: f32, on: f32, off: f32) -> Vec<(f32, f32)> {
    let mut runs = Vec::new();
    if length <= 0.0 || on <= 0.0 {
        return runs;
    }
    let mut start = 0.0;
    while start < length {
        runs.push((start, on.min(length - start)));
        start += on + off;
    }
    runs
}

/// `content` with a dashed 1 px outline in `color`; nothing is drawn when it is `None`.
pub fn dashed<'a>(
    content: impl Into<Element<'a, Message>>,
    color: Option<Color>,
) -> Element<'a, Message> {
    Element::new(Dashed {
        content: content.into(),
        color,
    })
}

struct Dashed<'a> {
    content: Element<'a, Message>,
    color: Option<Color>,
}

impl Widget<Message, iced::Theme, iced::Renderer> for Dashed<'_> {
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
        let Some(color) = self.color else { return };
        let Rectangle {
            x,
            y,
            width,
            height,
        } = layout.bounds();
        let mut dash = |bounds: Rectangle| {
            renderer.fill_quad(
                Quad {
                    bounds,
                    border: Border::default(),
                    ..Quad::default()
                },
                color,
            );
        };
        for (start, len) in dash_runs(width, DASH_ON, DASH_OFF) {
            dash(Rectangle::new((x + start, y).into(), Size::new(len, 1.0)));
            dash(Rectangle::new(
                (x + start, y + height - 1.0).into(),
                Size::new(len, 1.0),
            ));
        }
        for (start, len) in dash_runs(height, DASH_ON, DASH_OFF) {
            dash(Rectangle::new((x, y + start).into(), Size::new(1.0, len)));
            dash(Rectangle::new(
                (x + width - 1.0, y + start).into(),
                Size::new(1.0, len),
            ));
        }
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
    use super::*;

    #[test]
    fn dash_runs_clip_the_last_run() {
        assert_eq!(dash_runs(10.0, 4.0, 3.0), vec![(0.0, 4.0), (7.0, 3.0)]);
        assert_eq!(dash_runs(4.0, 4.0, 3.0), vec![(0.0, 4.0)]);
    }

    #[test]
    fn dash_runs_of_nothing_is_empty() {
        assert!(dash_runs(0.0, DASH_ON, DASH_OFF).is_empty());
    }
}
