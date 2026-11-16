//! The divider at the top of the compare panel and of the transfer queue: a 7 px band
//! with the resize cursor holding the panel's 1 px strong rule. A press starts a resize
//! ([`DividerMsg::Press`], at the position [`crate::ui::pointer`] published just before),
//! a double-click resets the panel's height ([`DividerMsg::Reset`]). A 24 × 3 px grip in
//! `muted` shows on hover (and while the panel is being dragged).

use iced::widget::{column, container, hover, mouse_area, rule, space, stack};
use iced::{Element, Length, mouse};

use crate::app::{DividerMsg, Message, Panel};
use crate::ui::theme;

/// The divider's height: its hit area.
pub const HIT: f32 = 7.0;
/// The grip's size.
const GRIP_WIDTH: f32 = 24.0;
const GRIP_HEIGHT: f32 = 3.0;

/// The divider of `panel`, drawn `height` px tall now (where a drag starts from) and
/// `cap` px at most in the window now. `hovered_grip`: the grip shows even without the
/// pointer over it (a drag in progress).
pub fn view<'a>(panel: Panel, height: f32, cap: f32, hovered_grip: bool) -> Element<'a, Message> {
    let grip = container(
        container(space())
            .width(GRIP_WIDTH)
            .height(GRIP_HEIGHT)
            .style(theme::grip),
    )
    .center(Length::Fill);
    let band: Element<'a, Message> = if hovered_grip {
        stack![band(), grip].into()
    } else {
        hover(band(), grip)
    };
    mouse_area(container(band).width(Length::Fill).height(HIT))
        .interaction(mouse::Interaction::ResizingVertically)
        .on_press(Message::Divider(DividerMsg::Press { panel, height, cap }))
        .on_double_click(Message::Divider(DividerMsg::Reset(panel)))
        .into()
}

/// The same band for a panel that cannot be resized now (the hidden queue): the rule,
/// no cursor, no grip, so the layout does not move.
pub fn inert<'a>() -> Element<'a, Message> {
    band()
}

/// The 1 px strong rule, centred in the hit area.
fn band<'a>() -> Element<'a, Message> {
    let pad = (HIT - theme::HAIRLINE) / 2.0;
    column![
        space().height(pad),
        rule::horizontal(theme::HAIRLINE).style(theme::rule_strong),
        space().height(pad),
    ]
    .width(Length::Fill)
    .height(HIT)
    .into()
}
