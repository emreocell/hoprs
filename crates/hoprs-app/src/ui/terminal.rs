//! The terminal tab's view: the terminal widget (a custom iced widget drawing a
//! [`TerminalModel`]), the bar over an ended shell, and the search bar.
//!
//! **Drawing.** Only the viewport's cells are drawn (the model yields just those): the
//! background as one quad per run of equal colour, the text as one run per row stretch of
//! plain ASCII in one style (wide and non-ASCII characters on their own, at their cell),
//! then the selection, the search match and the cursor (a block with the focus, an outline
//! without). JetBrains Mono, at the size of the settings.
//!
//! **Metrics.** The cell size comes from the font: one measure per font size, kept for the
//! whole run ([`cell_size`]), never per frame.
//!
//! **Events.** The widget reports its size in cells ([`Message::TerminalResized`]) when it
//! changes, keys while it has the focus (all of them: the tab decides what is a shortcut),
//! the wheel, and presses and drags as selections. The tab's state lives in the
//! [`App`](crate::app::App); the widget keeps only what a gesture needs.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use alacritty_terminal::index::{Point as GridPoint, Side as GridSide};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::text as adv_text;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::alignment;
use iced::keyboard::{self, key::Named};
use iced::mouse;
use iced::widget::{Id, button, column, container, row, space, text, text_input};
use iced::{
    Alignment, Background, Border, Color, Element, Event, Font, Length, Padding, Pixels, Point,
    Rectangle, Size,
};

use crate::app::terminal::{SearchBar, TerminalMsg, TerminalTab, WHEEL_LINES};
use crate::app::{App, Message, Tab, TabId, TabState};
use crate::terminal::input::{Key, Modifiers};
use crate::terminal::selection::SelectionKind;
use crate::terminal::{CellView, CursorShape, Rgb, TerminalModel};
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::widgets;

/// Space between the widget's edge and the grid.
pub const PADDING: f32 = 8.0;
/// Line height, relative to the font size.
const LINE_HEIGHT: f32 = 1.25;
/// Two presses this close make a double click.
const MULTI_CLICK: Duration = Duration::from_millis(400);

/// The id of a terminal tab's search field.
pub fn search_id(tab: TabId) -> Id {
    Id::from(format!("terminal-search-{}", tab.0))
}

/// The grid that fits `size` px with cells of `cell` px, inside [`PADDING`]: at least 2
/// columns and 1 row.
pub fn pixels_to_grid_size(size: Size, cell: Size) -> (u16, u16) {
    let fit = |space: f32, step: f32, min: u16| {
        if step <= 0.0 || !space.is_finite() {
            return min;
        }
        let n = ((space - 2.0 * PADDING) / step).floor();
        if n < f32::from(min) {
            min
        } else {
            n.min(f32::from(u16::MAX)) as u16
        }
    };
    (
        fit(size.width, cell.width, 2),
        fit(size.height, cell.height, 1),
    )
}

thread_local! {
    /// Cell sizes measured so far, by font size (the bits of the `f32`).
    static CELL_SIZES: RefCell<Vec<(u32, Size)>> = const { RefCell::new(Vec::new()) };
}

/// The cell of JetBrains Mono at `font_size`: the advance of one character, and the line
/// height rounded to whole pixels (rows then meet without seams). Measured once per size.
pub fn cell_size<P: adv_text::Paragraph<Font = Font>>(font_size: f32) -> Size {
    let key = font_size.to_bits();
    if let Some(size) = CELL_SIZES.with_borrow(|sizes| {
        sizes
            .iter()
            .find(|(bits, _)| *bits == key)
            .map(|(_, size)| *size)
    }) {
        return size;
    }
    const SAMPLE: &str = "MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM";
    let height = (font_size * LINE_HEIGHT).ceil();
    let paragraph = P::with_text(adv_text::Text {
        content: SAMPLE,
        bounds: Size::INFINITE,
        size: Pixels(font_size),
        line_height: adv_text::LineHeight::Absolute(Pixels(height)),
        font: theme::MONO,
        align_x: adv_text::Alignment::Left,
        align_y: alignment::Vertical::Top,
        shaping: adv_text::Shaping::Basic,
        wrapping: adv_text::Wrapping::None,
    });
    let width = paragraph.min_bounds().width / SAMPLE.len() as f32;
    // A renderer without fonts (tests) measures nothing: JetBrains Mono's advance is 0.6 em.
    let width = if width > 0.0 { width } else { font_size * 0.6 };
    let size = Size::new(width, height);
    CELL_SIZES.with_borrow_mut(|sizes| sizes.push((key, size)));
    size
}

/// An iced key press as the terminal's key, `None` for keys it does not send.
pub fn map_key(
    key: &keyboard::Key,
    modified_key: &keyboard::Key,
    text: Option<&str>,
    modifiers: keyboard::Modifiers,
) -> Option<(Key, Modifiers)> {
    let mods = Modifiers {
        shift: modifiers.shift(),
        ctrl: modifiers.control(),
        alt: modifiers.alt(),
    };
    let key = match key {
        keyboard::Key::Named(named) => match named {
            Named::Enter => Key::Enter,
            Named::Backspace => Key::Backspace,
            Named::Tab => Key::Tab,
            Named::Escape => Key::Escape,
            Named::ArrowUp => Key::Up,
            Named::ArrowDown => Key::Down,
            Named::ArrowLeft => Key::Left,
            Named::ArrowRight => Key::Right,
            Named::Home => Key::Home,
            Named::End => Key::End,
            Named::PageUp => Key::PageUp,
            Named::PageDown => Key::PageDown,
            Named::Insert => Key::Insert,
            Named::Delete => Key::Delete,
            Named::Space => Key::Char(' '),
            Named::F1 => Key::F(1),
            Named::F2 => Key::F(2),
            Named::F3 => Key::F(3),
            Named::F4 => Key::F(4),
            Named::F5 => Key::F(5),
            Named::F6 => Key::F(6),
            Named::F7 => Key::F(7),
            Named::F8 => Key::F(8),
            Named::F9 => Key::F(9),
            Named::F10 => Key::F(10),
            Named::F11 => Key::F(11),
            Named::F12 => Key::F(12),
            _ => return None,
        },
        keyboard::Key::Character(c) => {
            // With Ctrl the layout's letter (Ctrl+C is 0x03 whatever Shift says); with Alt
            // the shifted character (Alt+Shift+a is ESC A); else the text typed.
            let base = if mods.ctrl { key } else { modified_key };
            let typed = text.filter(|t| !t.is_empty() && !mods.ctrl && !mods.alt);
            match typed {
                Some(t) => {
                    let mut chars = t.chars();
                    match (chars.next(), chars.next()) {
                        (Some(c), None) => Key::Char(c),
                        _ => Key::Text(t.to_owned()),
                    }
                }
                None => match base {
                    keyboard::Key::Character(s) => Key::Char(s.chars().next()?),
                    _ => Key::Char(c.chars().next()?),
                },
            }
        }
        keyboard::Key::Unidentified => return None,
    };
    Some((key, mods))
}

/// The terminal tab: what the connection allows, else the terminal with its bars.
pub fn view<'a>(app: &'a App, tab: &'a Tab, terminal: &'a TerminalTab) -> Element<'a, Message> {
    let body: Element<'a, Message> = match app.conn(&tab.site).state {
        TabState::Failed(_) => crate::ui::files::not_connected(app, tab),
        TabState::Connecting => centered(text(strings::connecting()).color(palette().muted)),
        TabState::Connected(_) => {
            // An ended shell takes no keys (its cursor is an outline).
            let focused = app.terminal_has_keyboard(tab);
            let widget = TerminalView {
                tab: tab.id,
                model: &terminal.model,
                font_size: f32::from(app.settings.terminal_font_size),
                focused,
                found: terminal.search.as_ref().and_then(|bar| bar.current.clone()),
            };
            let mut col = column![];
            let index = app.tab_index(tab.id).unwrap_or_default();
            if let Some(banner) = end_banner(tab.id, terminal, app.can_restart_terminal(index)) {
                col = col.push(banner).push(hrule());
            }
            if terminal.input_dropped {
                col = col
                    .push(note(strings::terminal_input_dropped()))
                    .push(hrule());
            }
            col = col.push(
                container(Element::new(widget))
                    .width(Length::Fill)
                    .height(Length::Fill),
            );
            if let Some(bar) = &terminal.search {
                col = col.push(hrule()).push(search_bar(tab.id, bar));
            }
            col.height(Length::Fill).into()
        }
    };
    // The session's remote edits show here too (a terminal-only session can have them).
    let body = match crate::ui::edits::strip(app, tab) {
        Some(strip) => column![strip, hrule(), body].into(),
        None => body,
    };
    let body = match crate::ui::files::lost_banner(app, tab) {
        Some(banner) => column![banner, hrule(), body].into(),
        None => body,
    };
    match crate::ui::edits::close_ask(app, tab) {
        Some(ask) => column![ask, hrule(), body].into(),
        None => body,
    }
}

fn hrule<'a>() -> Element<'a, Message> {
    iced::widget::rule::horizontal(theme::HAIRLINE)
        .style(theme::rule_1px)
        .into()
}

fn centered<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content).center(Length::Fill).into()
}

fn msg(tab: TabId, msg: TerminalMsg) -> Message {
    Message::Terminal { tab, msg }
}

/// A thin muted note across the tab.
fn note<'a>(text_: &'a str) -> Element<'a, Message> {
    container(text(text_).size(theme::TEXT_XS).color(palette().muted))
        .width(Length::Fill)
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
        .style(theme::pane_bar)
        .into()
}

/// The bar over an ended shell: why, and the button that opens a new one. Without a live
/// session the button is dim, with a hint (the connection's own bar says why).
fn end_banner(
    tab: TabId,
    terminal: &TerminalTab,
    can_restart: bool,
) -> Option<Element<'_, Message>> {
    let banner = terminal.end_banner()?;
    let p = palette();
    let mut reason = row![text(banner.message).color(p.error)]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center);
    if let Some(detail) = banner.detail {
        reason = reason.push(
            text(detail)
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
                .color(p.muted),
        );
    }
    let action = widgets::outlined(
        button(text(banner.action))
            .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
            .on_press_maybe(can_restart.then(|| msg(tab, TerminalMsg::Restart))),
        can_restart,
    );
    let hint = (!can_restart).then(|| {
        text(strings::terminal_waiting_for_connection())
            .size(theme::TEXT_XS)
            .color(p.muted)
    });
    Some(
        container(
            row![reason, space::horizontal(), hint, action]
                .spacing(theme::SPACE_SM)
                .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
        .style(theme::pane_bar)
        .into(),
    )
}

/// The inline search bar: the query, "No match", previous, next and close.
fn search_bar(tab: TabId, bar: &SearchBar) -> Element<'_, Message> {
    let field = text_input(strings::terminal_search_placeholder(), &bar.query)
        .id(search_id(tab))
        .on_input(move |query| msg(tab, TerminalMsg::SearchInput(query)))
        .on_submit(msg(tab, TerminalMsg::SearchNext))
        .font(theme::MONO)
        .size(theme::TEXT_MONO)
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
        .width(280)
        .style(theme::search);
    let action = |label, m| {
        button(text(label).size(theme::TEXT_XS))
            .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_XS]))
            .style(theme::button_text)
            .on_press(msg(tab, m))
    };
    let no_match = bar.no_match.then(|| {
        text(strings::terminal_search_no_match())
            .size(theme::TEXT_XS)
            .color(palette().muted)
    });
    container(
        row![
            field,
            no_match,
            space::horizontal(),
            action(
                strings::terminal_search_previous(),
                TerminalMsg::SearchPrevious
            ),
            action(strings::terminal_search_next(), TerminalMsg::SearchNext),
            action(strings::terminal_search_close(), TerminalMsg::CloseSearch),
        ]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
    .style(theme::pane_bar)
    .into()
}

/// The terminal widget.
struct TerminalView<'a> {
    tab: TabId,
    model: &'a TerminalModel,
    font_size: f32,
    /// The keyboard goes to the shell (the tab has the focus and no dialog is open).
    focused: bool,
    /// The search match shown.
    found: Option<crate::terminal::search::Range>,
}

/// What a gesture needs between events.
#[derive(Default)]
struct State {
    /// A press started a selection; moves extend it until the release.
    dragging: bool,
    /// The last press: when, where (viewport row, column) and how many in a row.
    last_press: Option<(Instant, (usize, usize), u8)>,
    /// Wheel pixels not yet a whole line.
    wheel: f32,
    /// The size last reported, until the model takes it.
    reported: Option<(u16, u16)>,
}

fn color(rgb: Rgb) -> Color {
    Color::from_rgb8(rgb.r, rgb.g, rgb.b)
}

impl TerminalView<'_> {
    fn cell<R: adv_text::Renderer<Font = Font>>(&self) -> Size {
        cell_size::<R::Paragraph>(self.font_size)
    }

    /// The viewport cell under `position`, and which half of it.
    fn cell_at(&self, bounds: Rectangle, cell: Size, position: Point) -> (usize, usize, GridSide) {
        let x = (position.x - bounds.x - PADDING).max(0.0) / cell.width;
        let y = (position.y - bounds.y - PADDING).max(0.0) / cell.height;
        let col = (x.floor() as usize).min(self.model.columns().saturating_sub(1));
        let row = (y.floor() as usize).min(self.model.screen_lines().saturating_sub(1));
        let side = if x.fract() < 0.5 {
            GridSide::Left
        } else {
            GridSide::Right
        };
        (row, col, side)
    }

    /// Whether the viewport cell (`line`, `column`) is inside the search match.
    fn in_match(&self, line: usize, column: usize) -> bool {
        let Some(found) = &self.found else {
            return false;
        };
        let offset = i32::try_from(self.model.display_offset()).unwrap_or(0);
        let line = i32::try_from(line).unwrap_or(0) - offset;
        let point = GridPoint::new(
            alacritty_terminal::index::Line(line),
            alacritty_terminal::index::Column(column),
        );
        found.contains(&point)
    }
}

impl<Theme, R> Widget<Message, Theme, R> for TerminalView<'_>
where
    R: adv_text::Renderer<Font = Font>,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &R, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &R,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        let cell = self.cell::<R>();
        let tab = self.tab;

        // The size in cells: reported once per change, whatever the event.
        let grid = pixels_to_grid_size(bounds.size(), cell);
        let current = (
            u16::try_from(self.model.columns()).unwrap_or(u16::MAX),
            u16::try_from(self.model.screen_lines()).unwrap_or(u16::MAX),
        );
        if grid != current && state.reported != Some(grid) {
            state.reported = Some(grid);
            shell.publish(Message::TerminalResized {
                tab,
                cols: grid.0,
                rows: grid.1,
            });
        } else if grid == current {
            state.reported = None;
        }

        match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modified_key,
                modifiers,
                text,
                ..
            }) if self.focused => {
                if let Some((key, mods)) = map_key(key, modified_key, text.as_deref(), *modifiers) {
                    shell.publish(msg(tab, TerminalMsg::Key { key, mods }));
                }
                // Every key belongs to the terminal while it has the focus: the file panes'
                // keys (and Tab's focus change) never see it.
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(position) = cursor.position_over(bounds) else {
                    if self.focused {
                        shell.publish(msg(tab, TerminalMsg::Focus(false)));
                    }
                    return;
                };
                let (row, col, side) = self.cell_at(bounds, cell, position);
                let now = Instant::now();
                let count = match state.last_press {
                    Some((at, place, n)) if now - at < MULTI_CLICK && place == (row, col) => {
                        n % 3 + 1
                    }
                    _ => 1,
                };
                state.last_press = Some((now, (row, col), count));
                let kind = match count {
                    2 => SelectionKind::Word,
                    3 => SelectionKind::Line,
                    _ => SelectionKind::Cells,
                };
                state.dragging = true;
                shell.publish(msg(
                    tab,
                    TerminalMsg::SelectStart {
                        row,
                        col,
                        side,
                        kind,
                    },
                ));
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => {
                let (row, col, side) = self.cell_at(bounds, cell, *position);
                shell.publish(msg(tab, TerminalMsg::SelectMove { row, col, side }));
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging = false;
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => (y * WHEEL_LINES as f32).round(),
                    mouse::ScrollDelta::Pixels { y, .. } => {
                        state.wheel += y / cell.height;
                        let whole = state.wheel.trunc();
                        state.wheel -= whole;
                        whole
                    }
                };
                if lines != 0.0 {
                    shell.publish(msg(tab, TerminalMsg::Scroll(lines as i32)));
                }
                shell.capture_event();
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut R,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let Some(clip) = bounds.intersection(viewport) else {
            return;
        };
        let cell = self.cell::<R>();
        let colors = self.model.theme();
        let quad = |renderer: &mut R, rect: Rectangle, fill: Color| {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: rect,
                    ..renderer::Quad::default()
                },
                Background::Color(fill),
            );
        };
        quad(renderer, bounds, color(colors.bg));
        let origin = Point::new(bounds.x + PADDING, bounds.y + PADDING);
        let at = |line: usize, column: usize| {
            Point::new(
                origin.x + column as f32 * cell.width,
                origin.y + line as f32 * cell.height,
            )
        };
        let mark = color(colors.selection);
        let found = color(colors.ansi[8]);

        // One pass over the visible cells: backgrounds merged per run, text per run.
        let mut background: Option<(usize, usize, usize, Color)> = None; // line, from, to
        let mut run: Option<(usize, usize, String, Rgb, bool, bool)> = None; // line, column
        let flush_bg = |renderer: &mut R, bg: &mut Option<(usize, usize, usize, Color)>| {
            if let Some((line, from, to, fill)) = bg.take() {
                let start = at(line, from);
                let width = (to - from) as f32 * cell.width;
                quad(
                    renderer,
                    Rectangle::new(start, Size::new(width, cell.height)),
                    fill,
                );
            }
        };
        let text_of = |content: String, bold: bool, italic: bool, shaping| adv_text::Text {
            content,
            bounds: Size::new(f32::INFINITY, cell.height),
            size: Pixels(self.font_size),
            line_height: adv_text::LineHeight::Absolute(Pixels(cell.height)),
            font: Font {
                weight: if bold {
                    iced::font::Weight::Medium
                } else {
                    iced::font::Weight::Normal
                },
                style: if italic {
                    iced::font::Style::Italic
                } else {
                    iced::font::Style::Normal
                },
                ..theme::MONO
            },
            align_x: adv_text::Alignment::Left,
            align_y: alignment::Vertical::Top,
            shaping,
            wrapping: adv_text::Wrapping::None,
        };
        let mut texts: Vec<(Point, adv_text::Text, Color)> = Vec::new();
        let mut underlines: Vec<(Point, f32, Color)> = Vec::new();
        let flush_run = |run: &mut Option<(usize, usize, String, Rgb, bool, bool)>,
                         texts: &mut Vec<(Point, adv_text::Text, Color)>| {
            if let Some((line, column, content, fg, bold, italic)) = run.take()
                && !content.trim_end().is_empty()
            {
                texts.push((
                    at(line, column),
                    text_of(content, bold, italic, adv_text::Shaping::Basic),
                    color(fg),
                ));
            }
        };
        let cells: Vec<CellView> = self.model.cells().collect();
        for c in &cells {
            let fill = if c.selected {
                Some(mark)
            } else if self.in_match(c.line, c.column) {
                Some(found)
            } else if c.bg != colors.bg {
                Some(color(c.bg))
            } else {
                None
            };
            let end = c.column + usize::from(c.width);
            match (&mut background, fill) {
                (Some((line, _, to, f)), Some(fill))
                    if *line == c.line && *to == c.column && *f == fill =>
                {
                    *to = end;
                }
                (_, Some(fill)) => {
                    flush_bg(renderer, &mut background);
                    background = Some((c.line, c.column, end, fill));
                }
                (_, None) => flush_bg(renderer, &mut background),
            }
            if c.underline {
                underlines.push((
                    at(c.line, c.column),
                    f32::from(c.width) * cell.width,
                    color(c.fg),
                ));
            }
            let plain = c.width == 1 && (c.ch == ' ' || c.ch.is_ascii_graphic());
            if plain {
                match &mut run {
                    Some((line, column, content, fg, bold, italic))
                        if *line == c.line
                            && *column + content.len() == c.column
                            && (c.ch == ' '
                                || (*fg == c.fg && *bold == c.bold && *italic == c.italic)) =>
                    {
                        content.push(c.ch);
                    }
                    _ => {
                        flush_run(&mut run, &mut texts);
                        if c.ch != ' ' {
                            run =
                                Some((c.line, c.column, c.ch.to_string(), c.fg, c.bold, c.italic));
                        }
                    }
                }
            } else {
                flush_run(&mut run, &mut texts);
                if c.ch != '\0' && !c.ch.is_whitespace() {
                    texts.push((
                        at(c.line, c.column),
                        text_of(
                            c.ch.to_string(),
                            c.bold,
                            c.italic,
                            adv_text::Shaping::Advanced,
                        ),
                        color(c.fg),
                    ));
                }
            }
        }
        flush_bg(renderer, &mut background);
        flush_run(&mut run, &mut texts);
        for (position, text, fill) in texts {
            renderer.fill_text(text, position, fill, clip);
        }
        for (position, width, fill) in underlines {
            let y = position.y + cell.height - 2.0;
            quad(
                renderer,
                Rectangle::new(Point::new(position.x, y), Size::new(width, 1.0)),
                fill,
            );
        }

        // The cursor: a block with the focus, an outline without.
        let cursor = self.model.cursor();
        if !cursor.visible {
            return;
        }
        let under = cells
            .iter()
            .find(|c| c.line == cursor.line && c.column == cursor.column);
        let width = under.map_or(1.0, |c| f32::from(c.width)) * cell.width;
        let corner = at(cursor.line, cursor.column);
        let ink = color(colors.cursor);
        let rect = |w: f32, h: f32, y: f32| {
            Rectangle::new(Point::new(corner.x, corner.y + y), Size::new(w, h))
        };
        match (cursor.shape, self.focused) {
            (CursorShape::Block, true) => {
                quad(renderer, rect(width, cell.height, 0.0), ink);
                if let Some(c) = under.filter(|c| c.ch != ' ' && c.ch != '\0') {
                    let shaping = if c.ch.is_ascii() {
                        adv_text::Shaping::Basic
                    } else {
                        adv_text::Shaping::Advanced
                    };
                    renderer.fill_text(
                        text_of(c.ch.to_string(), c.bold, c.italic, shaping),
                        corner,
                        color(colors.bg),
                        clip,
                    );
                }
            }
            (CursorShape::Beam, _) => quad(renderer, rect(2.0, cell.height, 0.0), ink),
            (CursorShape::Underline, _) => {
                quad(renderer, rect(width, 2.0, cell.height - 2.0), ink);
            }
            (CursorShape::Block | CursorShape::HollowBlock, _) => renderer.fill_quad(
                renderer::Quad {
                    bounds: rect(width, cell.height, 0.0),
                    border: Border {
                        color: ink,
                        width: 1.0,
                        radius: 0.0.into(),
                    },
                    ..renderer::Quad::default()
                },
                Background::Color(Color::TRANSPARENT),
            ),
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &R,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::None
        }
    }
}

impl<'a, Theme, R> From<TerminalView<'a>> for Element<'a, Message, Theme, R>
where
    R: adv_text::Renderer<Font = Font> + 'a,
    Theme: 'a,
{
    fn from(view: TerminalView<'a>) -> Self {
        Element::new(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_to_grid_size() {
        let cell = Size::new(7.8, 16.0);
        // 800 - 16 px of padding = 784 / 7.8 = 100.5 -> 100; 600 - 16 = 584 / 16 = 36.5.
        assert_eq!(
            super::pixels_to_grid_size(Size::new(800.0, 600.0), cell),
            (100, 36)
        );
        // Exactly on a cell boundary.
        assert_eq!(
            super::pixels_to_grid_size(Size::new(16.0 + 78.0, 16.0 + 32.0), cell),
            (10, 2)
        );
        // Too small, zero, negative or not finite: the smallest grid, never a panic.
        for size in [
            Size::new(10.0, 10.0),
            Size::ZERO,
            Size::new(-5.0, -5.0),
            Size::new(f32::NAN, f32::INFINITY),
        ] {
            assert_eq!(super::pixels_to_grid_size(size, cell), (2, 1), "{size:?}");
        }
        // A zero cell (no font yet) does not divide by zero.
        assert_eq!(
            super::pixels_to_grid_size(Size::new(800.0, 600.0), Size::ZERO),
            (2, 1)
        );
        // Huge sizes saturate.
        assert_eq!(
            super::pixels_to_grid_size(Size::new(1e9, 1e9), Size::new(1.0, 1.0)),
            (u16::MAX, u16::MAX)
        );
    }

    #[test]
    fn iced_keys_map_to_terminal_keys() {
        use keyboard::{Key as IcedKey, Modifiers as M};
        let ch = |s: &str| IcedKey::Character(s.into());
        let named = IcedKey::Named;
        // Plain typing: the text produced.
        assert_eq!(
            map_key(&ch("a"), &ch("a"), Some("a"), M::empty()),
            Some((Key::Char('a'), Modifiers::default()))
        );
        assert_eq!(
            map_key(&ch("a"), &ch("A"), Some("A"), M::SHIFT).map(|(k, _)| k),
            Some(Key::Char('A'))
        );
        assert_eq!(
            map_key(&ch("é"), &ch("é"), Some("é"), M::empty()).map(|(k, _)| k),
            Some(Key::Char('é'))
        );
        // Ctrl: the letter, whatever text the platform made of it.
        let ctrl_shift_c = map_key(&ch("C"), &ch("C"), Some("\u{3}"), M::CTRL | M::SHIFT);
        assert_eq!(
            ctrl_shift_c,
            Some((
                Key::Char('C'),
                Modifiers {
                    shift: true,
                    ctrl: true,
                    alt: false
                }
            ))
        );
        // Alt: the shifted character.
        assert_eq!(
            map_key(&ch("a"), &ch("A"), None, M::ALT | M::SHIFT).map(|(k, _)| k),
            Some(Key::Char('A'))
        );
        // Named keys.
        assert_eq!(
            map_key(
                &named(Named::Enter),
                &named(Named::Enter),
                Some("\r"),
                M::empty()
            ),
            Some((Key::Enter, Modifiers::default()))
        );
        assert_eq!(
            map_key(&named(Named::F5), &named(Named::F5), None, M::empty()).map(|(k, _)| k),
            Some(Key::F(5))
        );
        assert_eq!(
            map_key(
                &named(Named::Space),
                &named(Named::Space),
                Some(" "),
                M::CTRL
            )
            .map(|(k, _)| k),
            Some(Key::Char(' '))
        );
        // Keys the terminal does not send.
        assert_eq!(
            map_key(&named(Named::Shift), &named(Named::Shift), None, M::SHIFT),
            None
        );
        assert_eq!(
            map_key(
                &IcedKey::Unidentified,
                &IcedKey::Unidentified,
                None,
                M::empty()
            ),
            None
        );
    }
}
