//! The Ctrl+K palette overlay: a box at the top center of the window, over the dimmed
//! window (the dialogs' backdrop), with the search field and up to 12 rows. A row reads
//! its name (Inter for an action, JetBrains Mono for data: a tab, site or workspace name),
//! its detail in mono (`user@host`, the tab Close tab closes), its group and its key. The
//! highlighted row has the accent wash. A click outside closes it.

use iced::widget::{
    Id, button, column, container, mouse_area, opaque, row, rule, space, stack, text, text_input,
};
use iced::{Alignment, Element, Length, Padding};

use crate::app::palette::{Entry, Group, Scope};
use crate::app::{App, Message, PaletteMsg};
use crate::ui::strings;
use crate::ui::theme::{self, palette};

/// The box's width.
pub const WIDTH: f32 = 560.0;
/// Its distance from the window's top (under the top bar).
pub const TOP: f32 = 56.0;

/// The id of the palette's search field ([`crate::app::Effect::FocusPalette`]).
pub fn input_id() -> Id {
    Id::new("palette-input")
}

fn msg(m: PaletteMsg) -> Message {
    Message::Palette(m)
}

/// `base` with the palette over it, when it is open.
pub fn overlay<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    let Some(state) = &app.palette else {
        return base;
    };
    let placeholder = match state.scope {
        Scope::All => strings::palette_placeholder(),
        Scope::Workspaces => strings::palette_workspaces_placeholder(),
    };
    let input = text_input(placeholder, &state.query)
        .id(input_id())
        .on_input(|query| msg(PaletteMsg::Query(query)))
        .on_submit(msg(PaletteMsg::Run))
        .padding(Padding::from([theme::SPACE_XS, theme::SPACE_SM]))
        .size(theme::TEXT_MD)
        .style(theme::search);

    let results = app.palette_results();
    let selected = state.selected.min(results.len().saturating_sub(1));
    let list: Element<'a, Message> = if results.is_empty() {
        container(
            text(strings::palette_no_results())
                .size(theme::TEXT_SM)
                .color(palette().muted),
        )
        .padding(Padding::from([theme::SPACE_XS, theme::SPACE_SM]))
        .into()
    } else {
        column(
            results
                .into_iter()
                .enumerate()
                .map(|(i, entry)| result_row(i, entry, i == selected)),
        )
        .spacing(theme::SPACE_3XS)
        .into()
    };

    let boxed = container(
        column![
            input,
            rule::horizontal(theme::HAIRLINE).style(theme::rule_1px),
            list
        ]
        .spacing(theme::SPACE_XS),
    )
    .width(WIDTH)
    .padding(theme::SPACE_XS)
    .style(theme::dialog_box);

    // The box takes its own clicks; one anywhere else closes the palette, and nothing
    // behind it gets the pointer.
    let layer = container(opaque(boxed))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .padding(Padding::ZERO.top(TOP))
        .style(theme::backdrop);
    let layer = opaque(mouse_area(layer).on_press(msg(PaletteMsg::Close)));
    stack![base, layer].into()
}

/// One result: its name, detail, group and key; a click runs it.
fn result_row<'a>(index: usize, entry: Entry, selected: bool) -> Element<'a, Message> {
    let p = palette();
    let label = if entry.mono {
        text(entry.label).font(theme::MONO).size(theme::TEXT_MONO)
    } else {
        text(entry.label).size(theme::TEXT_SM)
    };
    let detail = entry.detail.map(|detail| {
        text(detail)
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .color(p.muted)
            .wrapping(text::Wrapping::None)
    });
    let group = match entry.group {
        Group::Action => strings::palette_group_action(),
        Group::Tab => strings::palette_group_tab(),
        Group::Site => strings::palette_group_site(),
        Group::Workspace => strings::palette_group_workspace(),
    };
    let shortcut = entry.shortcut.map(|key| {
        container(text(key).size(theme::TEXT_XS))
            .padding(Padding::ZERO.left(theme::SPACE_2XS).right(theme::SPACE_2XS))
            .style(theme::kbd)
    });
    let line = row![
        label.wrapping(text::Wrapping::None),
        detail,
        space::horizontal(),
        text(group).size(theme::TEXT_TAG).color(p.muted),
        shortcut,
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    button(line)
        .width(Length::Fill)
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_SM]))
        .style(if selected {
            theme::row_selected
        } else {
            theme::row
        })
        .on_press(msg(PaletteMsg::Pick(index)))
        .into()
}
