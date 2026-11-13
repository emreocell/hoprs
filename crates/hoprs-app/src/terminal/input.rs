//! Keyboard and paste encoding, following xterm's conventions (what
//! `TERM=xterm-256color` promises the remote side).

use alacritty_terminal::term::TermMode;

/// A key press, as the terminal tab sees it. The tab maps iced's keys onto
/// this (Plan 6 Task 4).
// used by the terminal tab (Plan 6 Task 4)
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// A character key, already shifted by the layout (`A`, `é`, `{`).
    Char(char),
    /// Committed text, e.g. from an input method.
    Text(String),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1–F12; other numbers encode to nothing.
    F(u8),
}

/// Modifier keys held during a key press.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Modifiers {
    /// xterm's modifier parameter: 1 + shift + 2·alt + 4·ctrl.
    fn param(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }
}

const ESC: u8 = 0x1b;

/// The bytes a key press sends, or `None` when it sends nothing.
///
/// Copy and paste shortcuts (Ctrl+Shift+C / V) are the tab's to intercept
/// first: here Ctrl+Shift+C is plain Ctrl+C (`0x03`).
// used by the terminal tab (Plan 6 Task 4)
#[cfg_attr(not(test), allow(dead_code))]
pub fn encode_key(key: &Key, mods: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let alt_prefixed = |bytes: &[u8]| {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        if mods.alt {
            out.push(ESC);
        }
        out.extend_from_slice(bytes);
        out
    };
    let bytes = match key {
        Key::Text(text) if text.is_empty() => return None,
        Key::Text(text) => alt_prefixed(text.as_bytes()),
        Key::Char(c) => match ctrl_code(*c).filter(|_| mods.ctrl) {
            Some(code) => alt_prefixed(&[code]),
            None => alt_prefixed(c.encode_utf8(&mut [0; 4]).as_bytes()),
        },
        Key::Enter if mode.contains(TermMode::LINE_FEED_NEW_LINE) => alt_prefixed(b"\r\n"),
        Key::Enter => alt_prefixed(b"\r"),
        Key::Backspace if mods.ctrl => alt_prefixed(&[0x08]),
        Key::Backspace => alt_prefixed(&[0x7f]),
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => alt_prefixed(b"\t"),
        Key::Escape => alt_prefixed(&[ESC]),
        Key::Up => cursor_key(b'A', mods, mode),
        Key::Down => cursor_key(b'B', mods, mode),
        Key::Right => cursor_key(b'C', mods, mode),
        Key::Left => cursor_key(b'D', mods, mode),
        Key::Home => cursor_key(b'H', mods, mode),
        Key::End => cursor_key(b'F', mods, mode),
        Key::Insert => tilde_key(2, mods),
        Key::Delete => tilde_key(3, mods),
        Key::PageUp => tilde_key(5, mods),
        Key::PageDown => tilde_key(6, mods),
        Key::F(n @ 1..=4) => {
            let last = b'P' + (n - 1);
            if mods.param() > 1 {
                format!("\x1b[1;{}{}", mods.param(), char::from(last)).into_bytes()
            } else {
                vec![ESC, b'O', last]
            }
        }
        Key::F(n @ 5..=12) => {
            const CODES: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
            tilde_key(CODES[usize::from(n - 5)], mods)
        }
        Key::F(_) => return None,
    };
    Some(bytes)
}

/// Ctrl+`c`: letters to 0x01–0x1a, and xterm's symbols and digits.
fn ctrl_code(c: char) -> Option<u8> {
    let code = match c {
        'a'..='z' | 'A'..='Z' => (c.to_ascii_lowercase() as u8) - b'a' + 1,
        '@' | ' ' | '2' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '/' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    };
    Some(code)
}

/// Arrows, Home and End: SS3 in application cursor mode, CSI otherwise,
/// `CSI 1;m` with modifiers.
fn cursor_key(last: u8, mods: Modifiers, mode: TermMode) -> Vec<u8> {
    if mods.param() > 1 {
        format!("\x1b[1;{}{}", mods.param(), char::from(last)).into_bytes()
    } else if mode.contains(TermMode::APP_CURSOR) {
        vec![ESC, b'O', last]
    } else {
        vec![ESC, b'[', last]
    }
}

/// `CSI n ~`, or `CSI n;m ~` with modifiers.
fn tilde_key(n: u8, mods: Modifiers) -> Vec<u8> {
    if mods.param() > 1 {
        format!("\x1b[{n};{}~", mods.param()).into_bytes()
    } else {
        format!("\x1b[{n}~").into_bytes()
    }
}

const PASTE_START: &str = "\x1b[200~";
const PASTE_END: &str = "\x1b[201~";

/// The bytes a paste sends. With bracketed paste on, the text is wrapped in
/// `ESC[200~ … ESC[201~`, and every end marker inside it is removed (until
/// none is left, so removing one cannot form another): pasted text cannot
/// end the paste early and run as typed commands. Without it, line breaks
/// are sent as Enter (`\r`), as typing them would.
// used by the terminal tab (Plan 6 Task 4)
#[cfg_attr(not(test), allow(dead_code))]
pub fn encode_paste(text: &str, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let mut body = text.to_owned();
        while body.contains(PASTE_END) {
            body = body.replace(PASTE_END, "");
        }
        if body.is_empty() {
            return Vec::new();
        }
        [PASTE_START, &body, PASTE_END].concat().into_bytes()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
    };
    const SHIFT: Modifiers = Modifiers {
        shift: true,
        ..NONE
    };
    const CTRL: Modifiers = Modifiers { ctrl: true, ..NONE };
    const ALT: Modifiers = Modifiers { alt: true, ..NONE };

    fn normal(key: Key, mods: Modifiers) -> Option<Vec<u8>> {
        encode_key(&key, mods, TermMode::default())
    }

    fn app(key: Key, mods: Modifiers) -> Option<Vec<u8>> {
        encode_key(&key, mods, TermMode::default() | TermMode::APP_CURSOR)
    }

    fn bytes(s: &str) -> Option<Vec<u8>> {
        Some(s.as_bytes().to_vec())
    }

    #[test]
    fn encode_keys_normal_and_app_cursor_mode() {
        assert_eq!(normal(Key::Char('a'), NONE), bytes("a"));
        assert_eq!(normal(Key::Char('é'), NONE), bytes("é"));
        assert_eq!(normal(Key::Text("ção".into()), NONE), bytes("ção"));
        assert_eq!(normal(Key::Text(String::new()), NONE), None);
        assert_eq!(normal(Key::Enter, NONE), bytes("\r"));
        assert_eq!(
            encode_key(&Key::Enter, NONE, TermMode::LINE_FEED_NEW_LINE),
            bytes("\r\n")
        );
        assert_eq!(normal(Key::Backspace, NONE), Some(vec![0x7f]));
        assert_eq!(normal(Key::Tab, NONE), bytes("\t"));
        assert_eq!(normal(Key::Tab, SHIFT), bytes("\x1b[Z"));
        assert_eq!(normal(Key::Escape, NONE), bytes("\x1b"));

        // Arrows: CSI normally, SS3 in application cursor mode.
        for (key, last) in [
            (Key::Up, 'A'),
            (Key::Down, 'B'),
            (Key::Right, 'C'),
            (Key::Left, 'D'),
            (Key::Home, 'H'),
            (Key::End, 'F'),
        ] {
            assert_eq!(normal(key.clone(), NONE), bytes(&format!("\x1b[{last}")));
            assert_eq!(app(key.clone(), NONE), bytes(&format!("\x1bO{last}")));
            // With a modifier, always CSI 1;m.
            assert_eq!(app(key, CTRL), bytes(&format!("\x1b[1;5{last}")));
        }
        assert_eq!(normal(Key::Up, SHIFT), bytes("\x1b[1;2A"));
        assert_eq!(normal(Key::Left, ALT), bytes("\x1b[1;3D"));

        assert_eq!(normal(Key::Insert, NONE), bytes("\x1b[2~"));
        assert_eq!(normal(Key::Delete, NONE), bytes("\x1b[3~"));
        assert_eq!(normal(Key::PageUp, NONE), bytes("\x1b[5~"));
        assert_eq!(normal(Key::PageDown, NONE), bytes("\x1b[6~"));
        assert_eq!(normal(Key::PageUp, SHIFT), bytes("\x1b[5;2~"));
        assert_eq!(app(Key::Delete, NONE), bytes("\x1b[3~"));

        let f_keys = [
            "\x1bOP", "\x1bOQ", "\x1bOR", "\x1bOS", "\x1b[15~", "\x1b[17~", "\x1b[18~", "\x1b[19~",
            "\x1b[20~", "\x1b[21~", "\x1b[23~", "\x1b[24~",
        ];
        for (n, want) in (1..=12).zip(f_keys) {
            assert_eq!(normal(Key::F(n), NONE), bytes(want), "F{n}");
        }
        assert_eq!(normal(Key::F(1), SHIFT), bytes("\x1b[1;2P"));
        assert_eq!(normal(Key::F(5), CTRL), bytes("\x1b[15;5~"));
        assert_eq!(normal(Key::F(0), NONE), None);
        assert_eq!(normal(Key::F(13), NONE), None);
    }

    #[test]
    fn ctrl_and_alt_combinations() {
        for (i, c) in ('a'..='z').enumerate() {
            let code = u8::try_from(i).unwrap() + 1;
            assert_eq!(normal(Key::Char(c), CTRL), Some(vec![code]), "ctrl+{c}");
            let upper = c.to_ascii_uppercase();
            let ctrl_shift = Modifiers {
                shift: true,
                ..CTRL
            };
            assert_eq!(normal(Key::Char(upper), ctrl_shift), Some(vec![code]));
        }
        assert_eq!(normal(Key::Char('['), CTRL), Some(vec![0x1b]));
        assert_eq!(normal(Key::Char('\\'), CTRL), Some(vec![0x1c]));
        assert_eq!(normal(Key::Char(']'), CTRL), Some(vec![0x1d]));
        assert_eq!(normal(Key::Char('^'), CTRL), Some(vec![0x1e]));
        assert_eq!(normal(Key::Char('_'), CTRL), Some(vec![0x1f]));
        assert_eq!(normal(Key::Char(' '), CTRL), Some(vec![0x00]));
        assert_eq!(normal(Key::Char('@'), CTRL), Some(vec![0x00]));
        assert_eq!(normal(Key::Backspace, CTRL), Some(vec![0x08]));

        // Alt prefixes ESC.
        assert_eq!(normal(Key::Char('x'), ALT), bytes("\x1bx"));
        assert_eq!(normal(Key::Char('é'), ALT), bytes("\x1bé"));
        assert_eq!(normal(Key::Text("ab".into()), ALT), bytes("\x1bab"));
        assert_eq!(normal(Key::Backspace, ALT), Some(vec![0x1b, 0x7f]));
        assert_eq!(normal(Key::Enter, ALT), bytes("\x1b\r"));
        let ctrl_alt = Modifiers { alt: true, ..CTRL };
        assert_eq!(normal(Key::Char('c'), ctrl_alt), Some(vec![0x1b, 0x03]));
        assert_eq!(normal(Key::Up, ctrl_alt), bytes("\x1b[1;7A"));
        let all = Modifiers {
            shift: true,
            ctrl: true,
            alt: true,
        };
        assert_eq!(normal(Key::Delete, all), bytes("\x1b[3;8~"));
        // Ctrl on a character with no control code sends the character.
        assert_eq!(normal(Key::Char('1'), CTRL), bytes("1"));
    }

    #[test]
    fn bracketed_paste_wraps_and_strips_end_marker() {
        let bracketed = TermMode::default() | TermMode::BRACKETED_PASTE;
        assert_eq!(
            encode_paste("ls -la\n", bracketed),
            b"\x1b[200~ls -la\n\x1b[201~".to_vec()
        );
        // An embedded end marker cannot break out of the paste...
        assert_eq!(
            encode_paste("a\x1b[201~rm -rf ~\n", bracketed),
            b"\x1b[200~arm -rf ~\n\x1b[201~".to_vec()
        );
        // ...not even one that removing another would form.
        let pasted = encode_paste("x\x1b[20\x1b[201~1~y", bracketed);
        assert_eq!(pasted, b"\x1b[200~xy\x1b[201~".to_vec());
        // Without bracketed paste: raw text, newlines sent as Enter.
        assert_eq!(
            encode_paste("one\r\ntwo\nthree", TermMode::default()),
            b"one\rtwo\rthree".to_vec()
        );
        // Nothing to paste, nothing sent.
        assert_eq!(encode_paste("", bracketed), Vec::<u8>::new());
        assert_eq!(encode_paste("\x1b[201~", bracketed), Vec::<u8>::new());
    }
}
