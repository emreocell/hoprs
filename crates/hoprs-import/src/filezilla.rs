use std::io::Read;
use std::path::{Path, PathBuf};

use base64::Engine;
use hoprs_core::{Error, ErrorKind, FtpMode, Protocol, Result};
use quick_xml::XmlVersion;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use secrecy::SecretString;
use zeroize::Zeroizing;

/// Largest `sitemanager.xml` accepted (real files are a few hundred KiB).
const MAX_SITEMANAGER_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
pub struct ParsedSiteManager {
    pub entries: Vec<ParsedEntry>,
}

#[derive(Debug)]
pub enum ParsedEntry {
    Site(ParsedSite),
    Folder {
        name: String,
        entries: Vec<ParsedEntry>,
    },
}

#[derive(Debug)]
pub struct ParsedSite {
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub protocol: std::result::Result<Protocol, String>,
    pub user: String,
    pub password: ParsedPassword,
    pub key_file: Option<PathBuf>,
    pub remote_dir: Option<String>,
    pub local_dir: Option<PathBuf>,
    pub mode: Option<FtpMode>,
    pub notes: Vec<String>,
}

pub enum ParsedPassword {
    None,
    Plain(SecretString),
    Encrypted,
    AskEachTime,
}

impl std::fmt::Debug for ParsedPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::Plain(_) => f.write_str("Plain([redacted])"),
            Self::Encrypted => f.write_str("Encrypted"),
            Self::AskEachTime => f.write_str("AskEachTime"),
        }
    }
}

/// Deepest folder nesting accepted; real files rarely pass a handful.
const MAX_DEPTH: usize = 128;
/// Longest encoded RemoteDir decoded (the parser backtracks on it).
const MAX_REMOTE_DIR_BYTES: usize = 4096;

const NOTE_TLS_IF_AVAILABLE: &str =
    "FileZilla used TLS only if available; if the server has no TLS, edit the site to ftp";
const NOTE_MASTER_PASSWORD: &str =
    "password is protected by a FileZilla master password; enter it in hoprs";

/// `~/.config/filezilla/sitemanager.xml` (platform config dir).
pub fn default_sitemanager_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("filezilla").join("sitemanager.xml"))
}

/// Reads and parses a `sitemanager.xml`, refusing files over 16 MiB.
pub fn read_sitemanager(path: &Path) -> Result<ParsedSiteManager> {
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len > MAX_SITEMANAGER_BYTES {
        return Err(too_large(len));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    // Bounded even if the file grows meanwhile.
    file.take(MAX_SITEMANAGER_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SITEMANAGER_BYTES {
        return Err(too_large(bytes.len() as u64));
    }
    let xml = String::from_utf8(bytes).map_err(|_| {
        Error::new(ErrorKind::Protocol, "Cannot read sitemanager.xml")
            .with_detail("file is not valid UTF-8")
    })?;
    parse_sitemanager(&xml)
}

fn too_large(len: u64) -> Error {
    Error::new(ErrorKind::Protocol, "sitemanager.xml is too large")
        .with_detail(format!("{len} bytes (limit {MAX_SITEMANAGER_BYTES})"))
}

/// Parses FileZilla 3.x `sitemanager.xml` content. All or nothing: any XML
/// error fails the whole parse. DTDs (and so entity definitions) are refused.
pub fn parse_sitemanager(xml: &str) -> Result<ParsedSiteManager> {
    if xml.len() as u64 > MAX_SITEMANAGER_BYTES {
        return Err(too_large(xml.len() as u64));
    }
    let xml = xml.strip_prefix('\u{feff}').unwrap_or(xml);
    let root = build_tree(xml)?;
    let top = root
        .children
        .iter()
        .find(|e| e.name == "FileZilla3")
        .ok_or_else(|| {
            Error::new(ErrorKind::Protocol, "Cannot read sitemanager.xml")
                .with_detail("root element is not FileZilla3")
        })?;
    let entries = top
        .child("Servers")
        .map(|servers| convert_entries(&servers.children))
        .unwrap_or_default();
    Ok(ParsedSiteManager { entries })
}

/// Minimal element tree: just what the converter needs.
#[derive(Default)]
struct Elem {
    name: String,
    attrs: Vec<(String, String)>,
    /// Text runs, split wherever a child element or the end tag interrupts.
    segments: Vec<String>,
    current: String,
    children: Vec<Elem>,
}

impl Elem {
    fn child(&self, name: &str) -> Option<&Elem> {
        self.children.iter().find(|c| c.name == name)
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// All text, untrimmed.
    fn text(&self) -> String {
        self.segments.concat()
    }

    /// The last non-blank text run, trimmed: how FileZilla names entries.
    fn trailing_text(&self) -> Option<&str> {
        self.segments
            .iter()
            .rev()
            .map(|s| s.trim())
            .find(|s| !s.is_empty())
    }

    fn flush(&mut self) {
        if !self.current.is_empty() {
            self.segments.push(std::mem::take(&mut self.current));
        }
    }
}

fn syntax_error(xml: &str, offset: u64, what: impl std::fmt::Display) -> Error {
    let offset = usize::try_from(offset).unwrap_or(usize::MAX).min(xml.len());
    let before = &xml.as_bytes()[..offset];
    let line = before.iter().filter(|&&b| b == b'\n').count() + 1;
    let line_start = before
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i + 1);
    let column = String::from_utf8_lossy(&before[line_start..])
        .chars()
        .count()
        + 1;
    Error::new(ErrorKind::Protocol, "Cannot read sitemanager.xml")
        .with_detail(format!("line {line}, column {column}: {what}"))
}

fn build_tree(xml: &str) -> Result<Elem> {
    let mut reader = Reader::from_str(xml);
    let mut stack = vec![Elem::default()]; // sentinel root
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(err) => {
                let pos = match reader.error_position() {
                    0 => reader.buffer_position(),
                    pos => pos,
                };
                return Err(syntax_error(xml, pos, err));
            }
        };
        let fail = |what: &str| syntax_error(xml, reader.buffer_position(), what);
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                let is_empty = matches!(event, Event::Empty(_));
                let mut elem = Elem {
                    name: start.name().as_ref().to_owned(),
                    ..Elem::default()
                };
                for attr in start.attributes() {
                    let attr = attr.map_err(|e| fail(&e.to_string()))?;
                    let value = attr
                        .normalized_value(XmlVersion::Implicit1_0)
                        .map_err(|e| fail(&e.to_string()))?;
                    elem.attrs
                        .push((attr.key.as_ref().to_owned(), value.into_owned()));
                }
                let parent = stack.last_mut().expect("sentinel is never popped");
                parent.flush();
                if is_empty {
                    parent.children.push(elem);
                } else {
                    if stack.len() > MAX_DEPTH {
                        return Err(fail("elements are nested too deeply"));
                    }
                    stack.push(elem);
                }
            }
            Event::End(_) => {
                if stack.len() < 2 {
                    return Err(fail("unexpected closing tag"));
                }
                let mut elem = stack.pop().expect("checked above");
                elem.flush();
                stack
                    .last_mut()
                    .expect("sentinel is never popped")
                    .children
                    .push(elem);
            }
            Event::Text(text) => {
                let text = text.xml_content(XmlVersion::Implicit1_0);
                stack.last_mut().expect("sentinel").current.push_str(&text);
            }
            Event::CData(data) => {
                stack.last_mut().expect("sentinel").current.push_str(&data);
            }
            Event::GeneralRef(reference) => {
                let ch = match reference.resolve_char_ref() {
                    Ok(Some(ch)) => ch,
                    Ok(None) => match &*reference {
                        "lt" => '<',
                        "gt" => '>',
                        "amp" => '&',
                        "apos" => '\'',
                        "quot" => '"',
                        other => return Err(fail(&format!("unknown entity &{other};"))),
                    },
                    Err(err) => return Err(fail(&err.to_string())),
                };
                stack.last_mut().expect("sentinel").current.push(ch);
            }
            Event::DocType(_) => return Err(fail("DTDs are not supported")),
            Event::Eof => break,
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
        }
    }
    if stack.len() > 1 {
        return Err(syntax_error(
            xml,
            reader.buffer_position(),
            "unexpected end of file",
        ));
    }
    let mut root = stack.pop().expect("sentinel");
    root.flush();
    Ok(root)
}

fn convert_entries(elems: &[Elem]) -> Vec<ParsedEntry> {
    elems
        .iter()
        .filter_map(|elem| match elem.name.as_str() {
            "Server" => Some(ParsedEntry::Site(convert_site(elem))),
            "Folder" => {
                let name = elem
                    .child("Name")
                    .map(|n| n.text().trim().to_owned())
                    .filter(|n| !n.is_empty())
                    .or_else(|| elem.trailing_text().map(str::to_owned))
                    .unwrap_or_default();
                Some(ParsedEntry::Folder {
                    name,
                    entries: convert_entries(&elem.children),
                })
            }
            _ => None,
        })
        .collect()
}

fn field(server: &Elem, name: &str) -> Option<String> {
    server
        .child(name)
        .map(|e| e.text().trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn convert_site(server: &Elem) -> ParsedSite {
    let mut notes = Vec::new();
    let host = field(server, "Host").unwrap_or_default();
    let name = field(server, "Name")
        .or_else(|| server.trailing_text().map(str::to_owned))
        .unwrap_or_else(|| host.clone());

    let port = field(server, "Port").and_then(|p| match p.parse::<u16>() {
        Ok(port) => Some(port),
        Err(_) => {
            notes.push(format!("invalid port {p:?}, using the default"));
            None
        }
    });

    // A missing Protocol is FileZilla's default: FTP.
    let protocol = match field(server, "Protocol").as_deref() {
        None | Some("0") => {
            notes.push(NOTE_TLS_IF_AVAILABLE.to_owned());
            Ok(Protocol::FtpsExplicit)
        }
        Some("1") => Ok(Protocol::Ssh),
        Some("3") => Ok(Protocol::FtpsImplicit),
        Some("4") => Ok(Protocol::FtpsExplicit),
        Some("6") => Ok(Protocol::Ftp),
        Some(other) => Err(format!("unsupported protocol {other}")),
    };

    let mut user = server.child("User").map(|e| e.text()).unwrap_or_default();
    let mut key_file = None;
    let password = match field(server, "Logontype").as_deref() {
        // A missing Logontype is anonymous, as in FileZilla.
        None | Some("0") => {
            user = "anonymous".to_owned();
            ParsedPassword::None
        }
        Some("1") => convert_password(server.child("Pass"), &mut notes),
        Some("4") => {
            notes.push("FileZilla account field not imported".to_owned());
            convert_password(server.child("Pass"), &mut notes)
        }
        Some("2") | Some("3") => {
            notes.push("password is asked at each connection".to_owned());
            ParsedPassword::AskEachTime
        }
        Some("5") => {
            key_file = field(server, "Keyfile").map(PathBuf::from);
            ParsedPassword::None
        }
        Some(other) => {
            notes.push(format!("unknown logon type {other}; no password imported"));
            ParsedPassword::None
        }
    };

    let mode = match field(server, "PasvMode").as_deref() {
        Some("MODE_ACTIVE") => Some(FtpMode::Active),
        Some("MODE_PASSIVE") => Some(FtpMode::Passive),
        _ => None,
    };

    let remote_dir = field(server, "RemoteDir").and_then(|raw| match decode_remote_dir(&raw) {
        Ok(dir) => Some(dir),
        Err(reason) => {
            notes.push(format!("remote directory not imported: {reason}"));
            None
        }
    });
    let local_dir = field(server, "LocalDir").map(PathBuf::from);

    ParsedSite {
        name,
        host,
        port,
        protocol,
        user,
        password,
        key_file,
        remote_dir,
        local_dir,
        mode,
        notes,
    }
}

fn convert_password(pass: Option<&Elem>, notes: &mut Vec<String>) -> ParsedPassword {
    let Some(pass) = pass else {
        return ParsedPassword::None;
    };
    let text = Zeroizing::new(pass.text());
    let text = text.trim();
    if text.is_empty() {
        return ParsedPassword::None;
    }
    match pass.attr("encoding") {
        Some("base64") => {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(text)
                .map(Zeroizing::new);
            let decoded = match decoded {
                Ok(bytes) => bytes,
                Err(_) => {
                    notes.push("password is not valid base64; not imported".to_owned());
                    return ParsedPassword::None;
                }
            };
            match std::str::from_utf8(&decoded) {
                Ok("") => ParsedPassword::None,
                Ok(plain) => ParsedPassword::Plain(SecretString::from(plain.to_owned())),
                Err(_) => {
                    notes.push("password is not valid UTF-8; not imported".to_owned());
                    ParsedPassword::None
                }
            }
        }
        Some("crypt") => {
            notes.push(NOTE_MASTER_PASSWORD.to_owned());
            ParsedPassword::Encrypted
        }
        // Very old versions stored the password as plain text.
        None => ParsedPassword::Plain(SecretString::from(text.to_owned())),
        Some(_) => {
            notes.push("password uses an unknown encoding; not imported".to_owned());
            ParsedPassword::None
        }
    }
}

/// Decodes FileZilla's safe path: `<type> <prefix-len> [<prefix>]
/// (<len> <segment>)*`, only for type 1 (Unix, no prefix). FileZilla counts
/// segment lengths in characters; lengths in UTF-8 bytes are accepted too,
/// so a path with non-ASCII segments decodes either way.
fn decode_remote_dir(raw: &str) -> std::result::Result<String, String> {
    const INVALID: &str = "unrecognized format";
    if raw.len() > MAX_REMOTE_DIR_BYTES {
        return Err("path is too long".to_owned());
    }
    let (kind, rest) = take_number(raw).ok_or(INVALID)?;
    if kind != 1 {
        return Err(format!("unsupported path type {kind}"));
    }
    let rest = rest.strip_prefix(' ').ok_or(INVALID)?;
    let (prefix_len, rest) = take_number(rest).ok_or(INVALID)?;
    if prefix_len != 0 {
        return Err(INVALID.to_owned());
    }
    let segments = decode_segments(rest).ok_or(INVALID)?;
    Ok(if segments.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", segments.join("/"))
    })
}

fn take_number(s: &str) -> Option<(usize, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 || end > 9 {
        return None;
    }
    Some((s[..end].parse().ok()?, &s[end..]))
}

/// ` <len> <segment>` repeated; backtracks between the character and byte
/// interpretation of `len`.
fn decode_segments(rest: &str) -> Option<Vec<&str>> {
    if rest.is_empty() {
        return Some(Vec::new());
    }
    let rest = rest.strip_prefix(' ')?;
    let (len, rest) = take_number(rest)?;
    let rest = rest.strip_prefix(' ')?;
    if len == 0 {
        return None;
    }
    let by_chars = rest
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(rest.len()))
        .nth(len);
    let by_bytes = (len <= rest.len() && rest.is_char_boundary(len)).then_some(len);
    let mut candidates = [by_chars, by_bytes];
    if candidates[0] == candidates[1] {
        candidates[1] = None;
    }
    for end in candidates.into_iter().flatten() {
        let (segment, tail) = rest.split_at(end);
        if !tail.is_empty() && !tail.starts_with(' ') {
            continue;
        }
        if let Some(mut more) = decode_segments(tail) {
            more.insert(0, segment);
            return Some(more);
        }
    }
    None
}
