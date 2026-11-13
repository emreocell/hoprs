//! The pure terminal core: an `alacritty_terminal` grid fed by the `vte`
//! parser, with colours resolved to RGB through a [`TermTheme`].
//!
//! No iced types, no I/O, no clock reads. The terminal tab (Plan 6 Task 4)
//! feeds PTY bytes in batches, writes [`TermEvent::PtyWrite`] back to the
//! channel and draws [`TerminalModel::cells`].

pub mod input;
pub mod search;
pub mod selection;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{self, Osc52, TermMode, point_to_viewport, viewport_to_point};
use alacritty_terminal::vte::ansi::{
    self, Color, CursorShape as AnsiCursorShape, NamedColor, Processor, Timeout,
};

/// An sRGB colour, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// The dim (SGR 2) variant: two thirds of the intensity.
    fn dim(self) -> Self {
        let scale = |v: u8| (u16::from(v) * 2 / 3) as u8;
        Self::new(scale(self.r), scale(self.g), scale(self.b))
    }
}

impl From<ansi::Rgb> for Rgb {
    fn from(c: ansi::Rgb) -> Self {
        Self::new(c.r, c.g, c.b)
    }
}

impl From<Rgb> for ansi::Rgb {
    fn from(c: Rgb) -> Self {
        Self {
            r: c.r,
            g: c.g,
            b: c.b,
        }
    }
}

/// Terminal colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermTheme {
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    /// The 16 named colours: black, red, green, yellow, blue, magenta, cyan,
    /// white, then their bright variants.
    pub ansi: [Rgb; 16],
}

impl TermTheme {
    /// "hoprs dark": the app palette (mockup v2) as a terminal theme. `bg`
    /// is `paper`, `fg` and `cursor` are `ink`, `selection` is
    /// `accent_wash`; `ui::theme` tests that these match the palette. ANSI
    /// 1–7 and 9–15 keep a contrast of at least 4.5:1 on `bg`.
    pub const fn hoprs_dark() -> Self {
        const fn hex(v: u32) -> Rgb {
            Rgb::new((v >> 16) as u8, (v >> 8) as u8, v as u8)
        }
        Self {
            fg: hex(0xf5f5f5),
            bg: hex(0x070707),
            cursor: hex(0xf5f5f5),
            selection: hex(0x1f1f1f),
            ansi: [
                hex(0x2e2e2e),
                hex(0xe5737a),
                hex(0x8fc77a),
                hex(0xdcbc6e),
                hex(0x6fa8e8),
                hex(0xc58be0),
                hex(0x5fc0c8),
                hex(0xcacaca),
                hex(0x7a7a7a),
                hex(0xff9aa0),
                hex(0xb0e39c),
                hex(0xf2d68e),
                hex(0x9cc6ff),
                hex(0xdcb0f2),
                hex(0x8ee0e6),
                hex(0xf5f5f5),
            ],
        }
    }
}

impl Default for TermTheme {
    fn default() -> Self {
        Self::hoprs_dark()
    }
}

/// Something the terminal asks of its host while parsing output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermEvent {
    /// The remote set the window title (OSC 0/2). Empty: back to the default.
    /// C0, DEL and C1 control characters are already removed, so it is safe
    /// to show as-is.
    Title(String),
    /// BEL.
    Bell,
    /// OSC 52 copy. Only emitted when the host opted in with
    /// [`TerminalModel::set_allow_clipboard_store`]; dropped otherwise.
    ClipboardStore(String),
    /// Bytes to write back to the PTY: replies to DA, DSR, colour queries.
    PtyWrite(Vec<u8>),
}

/// The title without control characters (C0, DEL, C1): a remote program
/// must not smuggle escape sequences or line breaks into the UI.
fn sanitize_title(title: &str) -> String {
    title.chars().filter(|c| !c.is_control()).collect()
}

/// One drawable cell of the viewport. Spacer cells after a wide character
/// are not yielded: the wide cell has `width == 2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellView {
    /// Viewport row, 0 at the top.
    pub line: usize,
    pub column: usize,
    pub ch: char,
    /// Final colours to draw: bold-as-bright, dim, hidden and inverse are
    /// already applied.
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// SGR 7 was set (already applied to `fg` / `bg`).
    pub inverse: bool,
    /// 1, or 2 for CJK and emoji.
    pub width: u8,
    /// Inside the selection: draw `TermTheme::selection` behind it.
    pub selected: bool,
}

/// How the cursor is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Underline,
    Beam,
    HollowBlock,
}

/// Where and how to draw the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorView {
    /// Viewport row and column.
    pub line: usize,
    pub column: usize,
    pub shape: CursorShape,
    /// False when the program hid it (DECTCEM) or it is scrolled out of view.
    pub visible: bool,
}

/// Collects alacritty's events during [`TerminalModel::feed`].
#[derive(Clone, Default)]
struct EventProxy(Arc<Mutex<Vec<Event>>>);

impl EventProxy {
    fn take(&self) -> Vec<Event> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

/// A synchronized-update (DEC 2026) timer that never runs: each `feed` is
/// already one batch, and the core reads no clock.
#[derive(Debug, Default)]
struct NoSyncTimeout;

impl Timeout for NoSyncTimeout {
    fn set_timeout(&mut self, _: Duration) {}
    fn clear_timeout(&mut self) {}
    fn pending_timeout(&self) -> bool {
        false
    }
}

/// Grid dimensions for alacritty.
struct Size {
    columns: usize,
    lines: usize,
}

impl Size {
    /// At least one row and two columns: alacritty needs a last column
    /// distinct from the first for wide characters.
    fn new(cols: u16, rows: u16) -> Self {
        Self {
            columns: usize::from(cols.max(2)),
            lines: usize::from(rows.max(1)),
        }
    }
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

/// A terminal emulator state machine: bytes in, grid and events out.
pub struct TerminalModel {
    term: Term<EventProxy>,
    parser: Processor<NoSyncTimeout>,
    events: EventProxy,
    theme: TermTheme,
    allow_clipboard_store: bool,
}

impl TerminalModel {
    /// A `cols` x `rows` terminal keeping `scrollback` lines of history.
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        let events = EventProxy::default();
        let config = term::Config {
            scrolling_history: scrollback,
            // Copy only: OSC 52 must never read the local clipboard. Copies
            // are still filtered by `allow_clipboard_store`.
            osc52: Osc52::OnlyCopy,
            ..term::Config::default()
        };
        Self {
            term: Term::new(config, &Size::new(cols, rows), events.clone()),
            parser: Processor::new(),
            events,
            theme: TermTheme::default(),
            allow_clipboard_store: false,
        }
    }

    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn theme(&self) -> &TermTheme {
        &self.theme
    }

    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn set_theme(&mut self, theme: TermTheme) {
        self.theme = theme;
    }

    /// Whether OSC 52 copies reach the host as [`TermEvent::ClipboardStore`].
    /// Off by default: a remote program must not fill the clipboard unasked.
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn set_allow_clipboard_store(&mut self, allow: bool) {
        self.allow_clipboard_store = allow;
    }

    /// Parses `bytes` (any bytes: invalid UTF-8 and broken sequences are
    /// absorbed) and returns what the host must act on, in order.
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<TermEvent> {
        self.parser.advance(&mut self.term, bytes);
        self.events
            .take()
            .into_iter()
            .filter_map(|event| self.translate(event))
            .collect()
    }

    fn translate(&self, event: Event) -> Option<TermEvent> {
        match event {
            Event::Title(title) => Some(TermEvent::Title(sanitize_title(&title))),
            Event::ResetTitle => Some(TermEvent::Title(String::new())),
            Event::Bell => Some(TermEvent::Bell),
            Event::PtyWrite(text) => Some(TermEvent::PtyWrite(text.into_bytes())),
            Event::ClipboardStore(_, text) if self.allow_clipboard_store => {
                Some(TermEvent::ClipboardStore(text))
            }
            Event::ColorRequest(index, format) => {
                let rgb = self.color_at(index);
                Some(TermEvent::PtyWrite(format(rgb.into()).into_bytes()))
            }
            // Clipboard loads are disabled (`Osc52::OnlyCopy`); the text
            // area size in pixels is the widget's business; the rest is
            // redraw bookkeeping the host does after every feed anyway.
            _ => None,
        }
    }

    /// Resizes the grid; the host also resizes the PTY. Zero is clamped.
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(Size::new(cols, rows));
    }

    pub fn columns(&self) -> usize {
        self.term.columns()
    }

    pub fn screen_lines(&self) -> usize {
        self.term.screen_lines()
    }

    /// Lines of scrollback currently held (at most the configured cap).
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn history_len(&self) -> usize {
        self.term.history_size()
    }

    /// How many lines the view is scrolled back into history.
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// The viewport's cells, row by row, colours resolved.
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn cells(&self) -> impl Iterator<Item = CellView> + '_ {
        let content = self.term.renderable_content();
        let offset = content.display_offset;
        let selection = content.selection;
        content
            .display_iter
            .filter(|indexed| !indexed.cell.flags.contains(Flags::WIDE_CHAR_SPACER))
            .map(move |indexed| {
                let cell = indexed.cell;
                let flags = cell.flags;
                let mut fg = self.resolve_fg(cell.fg, flags);
                let mut bg = self.resolve(cell.bg);
                let inverse = flags.contains(Flags::INVERSE);
                if inverse {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if flags.contains(Flags::HIDDEN) {
                    fg = bg;
                }
                let line = usize::try_from(indexed.point.line.0 + offset as i32).unwrap_or(0);
                CellView {
                    line,
                    column: indexed.point.column.0,
                    ch: cell.c,
                    fg,
                    bg,
                    bold: flags.contains(Flags::BOLD),
                    italic: flags.contains(Flags::ITALIC),
                    underline: flags.intersects(Flags::ALL_UNDERLINES),
                    inverse,
                    width: if flags.contains(Flags::WIDE_CHAR) {
                        2
                    } else {
                        1
                    },
                    selected: selection.is_some_and(|range| range.contains(indexed.point)),
                }
            })
    }

    /// The cursor, in viewport coordinates.
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn cursor(&self) -> CursorView {
        let content = self.term.renderable_content();
        let cursor = content.cursor;
        let viewport = point_to_viewport(content.display_offset, cursor.point)
            .filter(|p| p.line < self.screen_lines());
        let shape = match cursor.shape {
            AnsiCursorShape::Underline => Some(CursorShape::Underline),
            AnsiCursorShape::Beam => Some(CursorShape::Beam),
            AnsiCursorShape::HollowBlock => Some(CursorShape::HollowBlock),
            AnsiCursorShape::Block => Some(CursorShape::Block),
            AnsiCursorShape::Hidden => None,
        };
        let (line, column) = viewport.map_or((0, 0), |p| (p.line, p.column.0));
        CursorView {
            line,
            column,
            shape: shape.unwrap_or(CursorShape::Block),
            visible: shape.is_some() && viewport.is_some(),
        }
    }

    /// Scrolls the view: positive goes back into history. Clamped.
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn scroll(&mut self, delta_lines: i32) {
        self.term.scroll_display(Scroll::Delta(delta_lines));
    }

    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    /// The modes the remote program set: application cursor keys
    /// (`APP_CURSOR`), bracketed paste (`BRACKETED_PASTE`), mouse, etc.
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    /// The grid point under viewport cell (`row`, `col`), for selection and
    /// search origins. Negative lines are scrollback.
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn viewport_to_point(&self, row: usize, col: usize) -> Point {
        let row = row.min(self.screen_lines() - 1);
        let col = col.min(self.columns() - 1);
        viewport_to_point(self.display_offset(), Point::new(row, Column(col)))
    }

    /// The first and last cells of the whole grid, scrollback included.
    // used by the terminal tab (Plan 6 Task 4)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn grid_bounds(&self) -> (Point, Point) {
        (
            Point::new(self.term.topmost_line(), Column(0)),
            Point::new(self.term.bottommost_line(), self.term.last_column()),
        )
    }

    /// Foreground: bold brightens the 8 base colours, dim darkens.
    fn resolve_fg(&self, color: Color, flags: Flags) -> Rgb {
        let color = match color {
            Color::Named(named) if flags.contains(Flags::DIM) => Color::Named(named.to_dim()),
            Color::Named(named) if flags.contains(Flags::BOLD) => Color::Named(named.to_bright()),
            Color::Indexed(i)
                if i < 8 && flags.contains(Flags::BOLD) && !flags.contains(Flags::DIM) =>
            {
                Color::Indexed(i + 8)
            }
            other => other,
        };
        let rgb = self.resolve(color);
        match color {
            Color::Spec(_) | Color::Indexed(_) if flags.contains(Flags::DIM) => rgb.dim(),
            _ => rgb,
        }
    }

    fn resolve(&self, color: Color) -> Rgb {
        match color {
            Color::Spec(rgb) => rgb.into(),
            Color::Indexed(i) => self.color_at(usize::from(i)),
            Color::Named(named) => self.color_at(named as usize),
        }
    }

    /// Colour `index` in alacritty's table (see `term::color`): 0–15 named,
    /// 16–231 cube, 232–255 grey ramp, 256+ foreground, background, cursor
    /// and dim variants. OSC 4/10/11 overrides win over the theme.
    fn color_at(&self, index: usize) -> Rgb {
        if let Some(rgb) = self.term.colors()[index.min(term::color::COUNT - 1)] {
            return rgb.into();
        }
        let theme = &self.theme;
        match index {
            0..=15 => theme.ansi[index],
            16..=231 => {
                const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
                let i = index - 16;
                Rgb::new(LEVELS[i / 36], LEVELS[(i / 6) % 6], LEVELS[i % 6])
            }
            232..=255 => {
                let v = 8 + 10 * (index - 232) as u8;
                Rgb::new(v, v, v)
            }
            _ => match index {
                i if i == NamedColor::Background as usize => theme.bg,
                i if i == NamedColor::Cursor as usize => theme.cursor,
                i if i == NamedColor::DimForeground as usize => theme.fg.dim(),
                i if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize)
                    .contains(&i) =>
                {
                    self.color_at(i - NamedColor::DimBlack as usize).dim()
                }
                // Foreground, bright foreground.
                _ => theme.fg,
            },
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use alacritty_terminal::index::{Column, Line, Point};

    /// The text of grid line `line` (negative = scrollback), trailing blanks trimmed.
    pub(crate) fn line_text(model: &TerminalModel, line: i32) -> String {
        let last = Column(model.columns() - 1);
        model
            .term
            .bounds_to_string(
                Point::new(Line(line), Column(0)),
                Point::new(Line(line), last),
            )
            .trim_end()
            .to_owned()
    }

    fn cell_at(model: &TerminalModel, line: usize, column: usize) -> Option<CellView> {
        model.cells().find(|c| c.line == line && c.column == column)
    }

    #[test]
    fn truecolor_and_256_colors() {
        let mut model = TerminalModel::new(20, 4, 100);
        model.feed(b"\x1b[38;2;1;2;3mX\x1b[38;5;196mY\x1b[38;5;67mZ\x1b[48;5;232mW\x1b[0;31mR");
        let theme = TermTheme::default();
        let x = cell_at(&model, 0, 0).unwrap();
        assert_eq!((x.ch, x.fg), ('X', Rgb::new(1, 2, 3)));
        assert_eq!(x.bg, theme.bg);
        // 196 = cube (5, 0, 0).
        assert_eq!(cell_at(&model, 0, 1).unwrap().fg, Rgb::new(255, 0, 0));
        // 67 = cube (1, 2, 3) = 95, 135, 175.
        assert_eq!(cell_at(&model, 0, 2).unwrap().fg, Rgb::new(95, 135, 175));
        // 232 = first grey of the ramp.
        assert_eq!(cell_at(&model, 0, 3).unwrap().bg, Rgb::new(8, 8, 8));
        // Named colours come from the theme.
        assert_eq!(cell_at(&model, 0, 4).unwrap().fg, theme.ansi[1]);
        // Unstyled cells use the theme's foreground.
        assert_eq!(cell_at(&model, 1, 0).unwrap().fg, theme.fg);
    }

    #[test]
    fn wide_chars_take_two_cells() {
        let mut model = TerminalModel::new(20, 4, 100);
        model.feed("a中b😀c".as_bytes());
        let row: Vec<(usize, char, u8)> = model
            .cells()
            .filter(|c| c.line == 0 && c.column < 8)
            .map(|c| (c.column, c.ch, c.width))
            .collect();
        assert_eq!(
            row,
            vec![
                (0, 'a', 1),
                (1, '中', 2),
                (3, 'b', 1),
                (4, '😀', 2),
                (6, 'c', 1),
                (7, ' ', 1),
            ]
        );
        assert_eq!(model.cursor().column, 7);
    }

    /// A 64-bit LCG (Knuth's MMIX constants): deterministic, no dependency.
    struct Lcg(u64);

    impl Lcg {
        fn next_u8(&mut self) -> u8 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 56) as u8
        }
    }

    #[test]
    fn invalid_utf8_and_garbage_never_panic() {
        const COLS: u16 = 80;
        const ROWS: u16 = 24;
        const SCROLLBACK: usize = 1_000;
        // Escape sequences cut short, overlong numbers, bad UTF-8, mode flips.
        const TRUNCATED: &[&[u8]] = &[
            b"\x1b",
            b"\x1b[",
            b"\x1b[38;2;1",
            b"\x1b[38;5;",
            b"\x1b]0;half a title",
            b"\x1b]2;t\xc2\x9b\x7fx\x07",
            b"\x1b]52;c;aGVsbG8",
            b"\x1bP1$r",
            b"\x1b[99999999999;99999999999H",
            b"\x1b[?1049h",
            b"\x1b[?1049l",
            b"\x1b[?2026h",
            b"\x1b[999999999999999999999999999999m",
            b"\x1b[;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;r",
            b"\xe4\xb8",
            b"\xf0\x9f\x98",
            b"\xc0\xaf",
            b"\xed\xa0\x80",
            b"\xff\xfe\xfd",
            b"\x1b[6n\x1b[c\x1b[",
            b"\x1b[200~",
        ];
        let mut rng = Lcg(0x5eed_1234_abcd_0042);
        let mut model = TerminalModel::new(COLS, ROWS, SCROLLBACK);
        model.set_allow_clipboard_store(true);
        let mut fed = 0usize;
        let mut titles = 0usize;
        while fed < 1 << 20 {
            let len = 1 + usize::from(rng.next_u8()) * 16;
            let mut chunk: Vec<u8> = (0..len).map(|_| rng.next_u8()).collect();
            let pick = usize::from(rng.next_u8()) % TRUNCATED.len();
            chunk.extend_from_slice(TRUNCATED[pick]);
            fed += chunk.len();
            for event in model.feed(&chunk) {
                match event {
                    TermEvent::Title(title) => {
                        assert!(!title.chars().any(char::is_control), "{title:?}");
                        titles += 1;
                    }
                    TermEvent::PtyWrite(reply) => assert!(!reply.is_empty()),
                    TermEvent::Bell | TermEvent::ClipboardStore(_) => {}
                }
            }
            // alacritty's own cursor (not the clamped view) stays in the grid.
            let raw = model.term.grid().cursor.point;
            assert!(
                (0..i32::from(ROWS)).contains(&raw.line.0) && raw.column.0 < usize::from(COLS),
                "raw cursor {raw:?}"
            );
            assert!(model.history_len() <= SCROLLBACK);
        }
        // The garbage did reach the title path, so the check above ran.
        assert!(titles > 0);
        // The grid still has its shape.
        assert_eq!((model.columns(), model.screen_lines()), (80, 24));
        let mut wide = 0;
        let cells: Vec<CellView> = model.cells().collect();
        for cell in &cells {
            assert!(cell.line < 24 && cell.column + usize::from(cell.width) <= 80);
            assert!(cell.width == 1 || cell.width == 2);
            if cell.width == 2 {
                wide += 1;
            }
        }
        // Every column of every row is covered exactly once.
        assert_eq!(cells.len() + wide, 80 * 24);

        // Abort any string or sequence in progress, then hard-reset: the
        // terminal is usable again and draws plain text where expected.
        model.feed(b"\x18\x1b\\\x1bc");
        model.feed(b"hello");
        assert_eq!(line_text(&model, 0), "hello");
        let cursor = model.cursor();
        assert_eq!((cursor.line, cursor.column, cursor.visible), (0, 5, true));
        assert_eq!(cell_at(&model, 0, 0).unwrap().fg, TermTheme::default().fg);
        assert_eq!(model.mode(), TermMode::default());
    }

    #[test]
    fn da_and_dsr_produce_pty_write() {
        let mut model = TerminalModel::new(80, 24, 100);
        let events = model.feed(b"\x1b[c");
        assert_eq!(events, vec![TermEvent::PtyWrite(b"\x1b[?6c".to_vec())]);
        let events = model.feed(b"\x1b[3;5H\x1b[6n");
        assert_eq!(events, vec![TermEvent::PtyWrite(b"\x1b[3;5R".to_vec())]);
        let events = model.feed(b"\x1b[5n");
        assert_eq!(events, vec![TermEvent::PtyWrite(b"\x1b[0n".to_vec())]);
        // OSC 11 query: answered with the theme background.
        let bg = TermTheme::default().bg;
        let events = model.feed(b"\x1b]11;?\x07");
        let want = format!(
            "\x1b]11;rgb:{0:02x}{0:02x}/{1:02x}{1:02x}/{2:02x}{2:02x}\x07",
            bg.r, bg.g, bg.b
        );
        assert_eq!(events, vec![TermEvent::PtyWrite(want.into_bytes())]);
        // Control characters (C0, DEL, C1) never reach the host's title.
        let events = model.feed("\x1b]2;a\u{9b}b\x7fc\u{85}d\x1b\\".as_bytes());
        assert_eq!(events, vec![TermEvent::Title("abcd".into())]);
        // Title and bell come through too.
        let events = model.feed(b"\x1b]2;remote\x07\x07");
        assert_eq!(
            events,
            vec![TermEvent::Title("remote".into()), TermEvent::Bell]
        );
    }

    #[test]
    fn osc52_is_ignored_by_default() {
        let osc52 = b"\x1b]52;c;aGVsbG8=\x07";
        let mut model = TerminalModel::new(80, 24, 100);
        assert_eq!(model.feed(osc52), vec![]);
        // Nothing leaks onto the screen either.
        assert_eq!(line_text(&model, 0), "");
        model.set_allow_clipboard_store(true);
        assert_eq!(
            model.feed(osc52),
            vec![TermEvent::ClipboardStore("hello".into())]
        );
        // Reading the clipboard (OSC 52 "?") is never answered.
        assert_eq!(model.feed(b"\x1b]52;c;?\x07"), vec![]);
    }

    #[test]
    fn scrollback_is_capped() {
        let mut model = TerminalModel::new(20, 5, 50);
        for i in 0..500 {
            model.feed(format!("line {i}\r\n").as_bytes());
        }
        assert_eq!(model.history_len(), 50);
        // Screen: 496..=499 and the empty prompt line; history: 446..=495.
        assert_eq!(line_text(&model, -50), "line 446");
        assert_eq!(line_text(&model, -1), "line 495");
        assert_eq!(line_text(&model, 0), "line 496");
        // Scrolling stops at the cap.
        model.scroll(1_000);
        assert_eq!(model.display_offset(), 50);
        // Viewport cells map to scrollback points while scrolled.
        assert_eq!(
            model.viewport_to_point(0, 0),
            Point::new(Line(-50), Column(0))
        );
        assert_eq!(
            model.viewport_to_point(99, 99),
            Point::new(Line(-46), Column(19))
        );
        assert_eq!(
            model.grid_bounds(),
            (
                Point::new(Line(-50), Column(0)),
                Point::new(Line(4), Column(19))
            )
        );
        // The cursor is scrolled out of view.
        assert!(!model.cursor().visible);
        let top: String = model
            .cells()
            .filter(|c| c.line == 0)
            .map(|c| c.ch)
            .collect();
        assert_eq!(top.trim_end(), "line 446");
        model.scroll(-10);
        assert_eq!(model.display_offset(), 40);
        model.scroll_to_bottom();
        assert_eq!(model.display_offset(), 0);
    }

    #[test]
    fn attributes_theme_and_color_overrides() {
        let mut model = TerminalModel::new(20, 4, 100);
        let mut theme = TermTheme::default();
        theme.ansi[1] = Rgb::new(200, 10, 10);
        model.set_theme(theme);
        assert_eq!(model.theme(), &theme);
        // Bold brightens a base colour; inverse swaps; hidden hides.
        model.feed(b"\x1b[31mA\x1b[1mB\x1b[0;7mC\x1b[0;8mD\x1b[0;3;4mE\x1b[0;2;32mF");
        let a = cell_at(&model, 0, 0).unwrap();
        assert_eq!(a.fg, Rgb::new(200, 10, 10));
        let b = cell_at(&model, 0, 1).unwrap();
        assert!(b.bold);
        assert_eq!(b.fg, theme.ansi[9]);
        let c = cell_at(&model, 0, 2).unwrap();
        assert!(c.inverse);
        assert_eq!((c.fg, c.bg), (theme.bg, theme.fg));
        let d = cell_at(&model, 0, 3).unwrap();
        assert_eq!(d.fg, d.bg);
        let e = cell_at(&model, 0, 4).unwrap();
        assert!(e.italic && e.underline && !e.bold);
        let green = theme.ansi[2];
        let dim = |v: u8| (u16::from(v) * 2 / 3) as u8;
        assert_eq!(
            cell_at(&model, 0, 5).unwrap().fg,
            Rgb::new(dim(green.r), dim(green.g), dim(green.b))
        );
        // OSC 4 redefines a palette entry; OSC 104 resets it.
        model.feed(b"\x1b]4;1;rgb:10/20/30\x07\x1b[1;1H\x1b[0;31mG");
        assert_eq!(
            cell_at(&model, 0, 0).unwrap().fg,
            Rgb::new(0x10, 0x20, 0x30)
        );
        model.feed(b"\x1b]104;1\x07");
        assert_eq!(cell_at(&model, 0, 0).unwrap().fg, Rgb::new(200, 10, 10));
        // Cursor shape (DECSCUSR) and visibility (DECTCEM).
        model.feed(b"\x1b[6 q");
        assert_eq!(model.cursor().shape, CursorShape::Beam);
        model.feed(b"\x1b[?25l");
        assert!(!model.cursor().visible);
    }

    #[test]
    fn resize_keeps_content_and_mode_tracks_decset() {
        let mut model = TerminalModel::new(20, 5, 100);
        model.feed(b"abc\x1b[?1h\x1b[?2004h");
        assert!(model.mode().contains(TermMode::APP_CURSOR));
        assert!(model.mode().contains(TermMode::BRACKETED_PASTE));
        model.resize(40, 10);
        assert_eq!((model.columns(), model.screen_lines()), (40, 10));
        assert_eq!(line_text(&model, 0), "abc");
        // Shrinking reflows wrapped text and keeps the cursor in the grid.
        let mut model = TerminalModel::new(20, 5, 100);
        model.feed(b"0123456789abcdefghij\r\nsecond line here");
        model.resize(8, 3);
        assert_eq!((model.columns(), model.screen_lines()), (8, 3));
        let cursor = model.cursor();
        assert!(cursor.visible && cursor.line < 3 && cursor.column < 8);
        let raw = model.term.grid().cursor.point;
        assert!((0..3).contains(&raw.line.0) && raw.column.0 < 8);
        // The long first line now spans 3 rows of 8: all of it is kept.
        let mut all = String::new();
        let (top, bottom) = model.grid_bounds();
        for line in top.line.0..=bottom.line.0 {
            all.push_str(&line_text(&model, line));
            all.push('|');
        }
        assert_eq!(all, "01234567|89abcdef|ghij|second l|ine here||");
        // Growing back reflows the rows together again.
        model.resize(20, 5);
        assert_eq!(line_text(&model, 0), "0123456789abcdefghij");
        // Zero is clamped instead of panicking.
        model.resize(0, 0);
        assert!(model.columns() >= 1 && model.screen_lines() >= 1);
    }

    /// WCAG 2 relative luminance.
    fn luminance(c: Rgb) -> f64 {
        let lin = |v: u8| {
            let s = f64::from(v) / 255.0;
            if s <= 0.039_28 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn contrast(a: Rgb, b: Rgb) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn theme_contrast() {
        let theme = TermTheme::default();
        let hex: Vec<String> = theme
            .ansi
            .iter()
            .map(|c| format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b))
            .collect();
        // The recorded "hoprs dark" ANSI palette.
        assert_eq!(
            hex,
            [
                "#2e2e2e", "#e5737a", "#8fc77a", "#dcbc6e", "#6fa8e8", "#c58be0", "#5fc0c8",
                "#cacaca", "#7a7a7a", "#ff9aa0", "#b0e39c", "#f2d68e", "#9cc6ff", "#dcb0f2",
                "#8ee0e6", "#f5f5f5",
            ]
        );
        for i in (1..=7).chain(9..=15) {
            let ratio = contrast(theme.ansi[i], theme.bg);
            assert!(ratio >= 4.5, "ansi {i} {} has contrast {ratio:.2}", hex[i]);
        }
        assert!(contrast(theme.fg, theme.bg) >= 7.0);
        // Text stays readable on a selected cell.
        assert!(contrast(theme.fg, theme.selection) >= 4.5);
    }
}
