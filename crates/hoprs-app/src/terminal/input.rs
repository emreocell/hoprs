//! Keyboard and paste encoding, following xterm's conventions (what
//! `TERM=xterm-256color` promises the remote side).
//!
//! Known limitations: the application keypad (DECKPAM, `ESC =`) is not
//! supported; keypad keys arrive as their plain characters or as the
//! navigation keys they map to, as in numeric keypad mode. The kitty
//! keyboard protocol is off (alacritty's `kitty_keyboard: false`), so
//! programs fall back to these legacy encodings.

use alacritty_terminal::term::TermMode;

/// A key press, as the terminal tab sees it. The tab maps iced's keys onto
/// this (Plan 6 Task 4).
#[derive(Clone, PartialEq, Eq)]
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

/// Never the character or text typed (it may be a password at a prompt).
impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Char(_) => f.write_str("Char(..)"),
            Self::Text(text) => write!(f, "Text(<{} bytes>)", text.len()),
            Self::Enter => f.write_str("Enter"),
            Self::Backspace => f.write_str("Backspace"),
            Self::Tab => f.write_str("Tab"),
            Self::Escape => f.write_str("Escape"),
            Self::Up => f.write_str("Up"),
            Self::Down => f.write_str("Down"),
            Self::Left => f.write_str("Left"),
            Self::Right => f.write_str("Right"),
            Self::Home => f.write_str("Home"),
            Self::End => f.write_str("End"),
            Self::PageUp => f.write_str("PageUp"),
            Self::PageDown => f.write_str("PageDown"),
            Self::Insert => f.write_str("Insert"),
            Self::Delete => f.write_str("Delete"),
            Self::F(n) => write!(f, "F({n})"),
        }
    }
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
        // Ctrl+, is the app's (Settings): never the shell's.
        Key::Char(',') if mods.ctrl && !mods.alt => return None,
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

/// Characters a bracketed paste never carries: ESC and the 8-bit CSI
/// (any escape sequence, the end marker `ESC[201~` included), and ^C, on
/// which some shells end the paste.
const PASTE_FORBIDDEN: [char; 3] = ['\x1b', '\x03', '\u{9b}'];

/// The bytes a paste sends. With bracketed paste on, the text is wrapped in
/// `ESC[200~ … ESC[201~` after removing every [`PASTE_FORBIDDEN`]
/// character, in one linear pass: pasted text cannot end the paste early
/// and run as typed commands. An empty result sends nothing. Without
/// bracketed paste, line breaks are sent as Enter (`\r`), as typing them
/// would.
pub fn encode_paste(text: &str, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let body: String = text
            .chars()
            .filter(|c| !PASTE_FORBIDDEN.contains(c))
            .collect();
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

    /// Ctrl+, opens Settings (even from a focused terminal): it sends nothing to the
    /// shell, as it has no control code.
    #[test]
    fn ctrl_comma_sends_nothing() {
        assert_eq!(normal(Key::Char(','), CTRL), None);
        let ctrl_shift = Modifiers {
            shift: true,
            ..CTRL
        };
        assert_eq!(normal(Key::Char(','), ctrl_shift), None);
        // A plain comma, or Alt+comma, still types.
        assert_eq!(normal(Key::Char(','), NONE), bytes(","));
        assert_eq!(normal(Key::Char(','), ALT), bytes("\x1b,"));
    }

    #[test]
    fn bracketed_paste_wraps_and_strips_end_marker() {
        let bracketed = TermMode::default() | TermMode::BRACKETED_PASTE;
        assert_eq!(
            encode_paste("ls -la\n", bracketed),
            b"\x1b[200~ls -la\n\x1b[201~".to_vec()
        );
        // An embedded end marker cannot break out of the paste: every ESC
        // is dropped, so no escape sequence survives inside it.
        assert_eq!(
            encode_paste("a\x1b[201~rm -rf ~\n", bracketed),
            b"\x1b[200~a[201~rm -rf ~\n\x1b[201~".to_vec()
        );
        // Nested markers, whatever the removal order would form.
        for nested in [
            "x\x1b[20\x1b[201~1~y",
            "x\x1b\x1b[201~[201~y",
            "x\x1b[201\x1b[201~~y",
        ] {
            let pasted = encode_paste(nested, bracketed);
            let body = &pasted[6..pasted.len() - 6];
            assert!(pasted.starts_with(b"\x1b[200~") && pasted.ends_with(b"\x1b[201~"));
            assert!(!body.contains(&0x1b), "{nested:?} -> {pasted:?}");
        }
        // ^C (some shells end the paste on it) and the 8-bit CSI are dropped
        // too; everything else, newlines and UTF-8 included, is kept.
        assert_eq!(
            encode_paste("a\x03b\u{9b}201~c\x1bd\té\n", bracketed),
            "\x1b[200~ab201~cd\té\n\x1b[201~".as_bytes().to_vec()
        );
        // A huge adversarial paste stays linear and clean.
        let hostile = "\x1b[20\x1b[201~1~".repeat(100_000);
        let pasted = encode_paste(&hostile, bracketed);
        assert_eq!(pasted.iter().filter(|&&b| b == 0x1b).count(), 2);
        // Without bracketed paste: raw text, newlines sent as Enter.
        assert_eq!(
            encode_paste("one\r\ntwo\nthree", TermMode::default()),
            b"one\rtwo\rthree".to_vec()
        );
        // Nothing to paste, nothing sent.
        assert_eq!(encode_paste("", bracketed), Vec::<u8>::new());
        assert_eq!(encode_paste("\x1b\x03\u{9b}", bracketed), Vec::<u8>::new());
    }
}
