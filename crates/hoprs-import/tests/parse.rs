use std::path::{Path, PathBuf};

use hoprs_core::{ErrorKind, FtpMode, Protocol};
use hoprs_import::{
    ParsedEntry, ParsedPassword, ParsedSite, ParsedSiteManager, parse_sitemanager, read_sitemanager,
};
use secrecy::ExposeSecret;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn parse(name: &str) -> ParsedSiteManager {
    parse_sitemanager(&fixture(name)).unwrap()
}

fn find<'a>(entries: &'a [ParsedEntry], name: &str) -> Option<&'a ParsedSite> {
    for entry in entries {
        match entry {
            ParsedEntry::Site(s) if s.name == name => return Some(s),
            ParsedEntry::Site(_) => {}
            ParsedEntry::Folder { entries, .. } => {
                if let Some(s) = find(entries, name) {
                    return Some(s);
                }
            }
        }
    }
    None
}

fn site(name: &str) -> ParsedSite {
    let mut parsed = parse("sitemanager-3.66.xml");
    let idx = parsed
        .entries
        .iter()
        .position(|e| matches!(e, ParsedEntry::Site(s) if s.name == name))
        .unwrap_or_else(|| panic!("no site {name}"));
    match parsed.entries.remove(idx) {
        ParsedEntry::Site(s) => s,
        _ => unreachable!(),
    }
}

#[test]
fn parses_all_protocols() {
    let legacy = site("Legacy FTP");
    assert_eq!(legacy.protocol, Ok(Protocol::FtpsExplicit));
    assert_eq!(
        legacy.notes,
        vec![
            "FileZilla used TLS only if available; if the server has no TLS, edit the site to ftp"
        ]
    );
    assert_eq!(legacy.host, "ftp.legacy.example.test");
    assert_eq!(legacy.port, Some(21));
    assert_eq!(legacy.user, "alice");

    assert_eq!(site("Shell").protocol, Ok(Protocol::Ssh));
    assert_eq!(site("Implicit TLS").protocol, Ok(Protocol::FtpsImplicit));
    let explicit = site("Explicit TLS");
    assert_eq!(explicit.protocol, Ok(Protocol::FtpsExplicit));
    assert!(explicit.notes.is_empty());
    assert_eq!(site("Plain FTP").protocol, Ok(Protocol::Ftp));
}

#[test]
fn unknown_protocol_is_skipped_with_reason() {
    let s = site("Object storage");
    assert_eq!(s.protocol, Err("unsupported protocol 7".to_string()));
    assert_eq!(s.host, "bucket.example.test");
}

#[test]
fn base64_password_decoded() {
    match site("Legacy FTP").password {
        ParsedPassword::Plain(p) => assert_eq!(p.expose_secret(), "hunter2-fictional"),
        _ => panic!("expected plain password"),
    }
    match site("Servidor Café & Açaí").password {
        ParsedPassword::Plain(p) => assert_eq!(p.expose_secret(), "senha-ç-ã"),
        _ => panic!("expected plain password"),
    }
    // Logon type 4 (account) also carries the password.
    assert!(matches!(
        site("Implicit TLS").password,
        ParsedPassword::Plain(_)
    ));
}

#[test]
fn invalid_base64_password_is_noted() {
    let s = site("Bad password");
    assert!(matches!(s.password, ParsedPassword::None));
    assert_eq!(s.notes.len(), 2, "{:?}", s.notes); // password + remote dir type
    assert!(s.notes.iter().any(|n| n.contains("password")));
    assert!(s.notes.iter().all(|n| !n.contains("not base64")));
}

#[test]
fn crypt_password_is_encrypted_with_note() {
    let parsed = parse("sitemanager-crypt.xml");
    let s = find(&parsed.entries, "Locked").unwrap();
    assert!(matches!(s.password, ParsedPassword::Encrypted));
    assert_eq!(
        s.notes,
        vec!["password is protected by a FileZilla master password; enter it in hoprs"]
    );
}

#[test]
fn ask_and_interactive_logons() {
    assert!(matches!(
        site("Explicit TLS").password,
        ParsedPassword::AskEachTime
    ));
    assert!(matches!(
        site("Interactive").password,
        ParsedPassword::AskEachTime
    ));
}

#[test]
fn anonymous_logon() {
    let s = site("Plain FTP");
    assert_eq!(s.user, "anonymous");
    assert!(matches!(s.password, ParsedPassword::None));
}

#[test]
fn key_file_logon() {
    let s = site("Shell");
    assert_eq!(
        s.key_file,
        Some(PathBuf::from("/home/alice/.ssh/id_ed25519"))
    );
    assert!(matches!(s.password, ParsedPassword::None));
    assert_eq!(s.port, Some(2222));
    assert_eq!(s.local_dir, Some(PathBuf::from("/home/alice/work")));
}

#[test]
fn missing_port_is_none() {
    assert_eq!(site("Explicit TLS").port, None);
}

#[test]
fn pasv_modes() {
    assert_eq!(site("Legacy FTP").mode, Some(FtpMode::Passive));
    assert_eq!(site("Implicit TLS").mode, Some(FtpMode::Active));
    assert_eq!(site("Shell").mode, None); // MODE_DEFAULT
    assert_eq!(site("Explicit TLS").mode, None); // absent
}

#[test]
fn remote_dir_decoding() {
    assert_eq!(site("Shell").remote_dir.as_deref(), Some("/home/alice"));
    assert_eq!(
        site("Servidor Café & Açaí").remote_dir.as_deref(),
        Some("/relatório/x")
    );
    assert_eq!(site("Legacy FTP").remote_dir, None); // empty element

    // Lengths are accepted as UTF-8 bytes or as characters.
    let xml = |dir: &str| {
        format!(
            "<FileZilla3><Servers><Server><Host>h</Host><Protocol>1</Protocol>\
             <RemoteDir>{dir}</RemoteDir><Name>n</Name></Server></Servers></FileZilla3>"
        )
    };
    for dir in ["1 0 10 relatório 1 x", "1 0 9 relatório 1 x"] {
        let parsed = parse_sitemanager(&xml(dir)).unwrap();
        let ParsedEntry::Site(s) = &parsed.entries[0] else {
            panic!()
        };
        assert_eq!(s.remote_dir.as_deref(), Some("/relatório/x"), "{dir}");
    }
    let parsed = parse_sitemanager(&xml("1 0")).unwrap();
    let ParsedEntry::Site(s) = &parsed.entries[0] else {
        panic!()
    };
    assert_eq!(s.remote_dir.as_deref(), Some("/"));

    // Invalid formats and non-unix types become a note.
    for name in ["Bad dir", "Bad password"] {
        let s = site(name);
        assert_eq!(s.remote_dir, None, "{name}");
        assert!(
            s.notes.iter().any(|n| n.contains("remote directory")),
            "{name}"
        );
    }
}

#[test]
fn nested_folders_keep_structure() {
    let parsed = parse("sitemanager-3.40-folders.xml");
    assert_eq!(parsed.entries.len(), 2);
    let ParsedEntry::Folder { name, entries } = &parsed.entries[0] else {
        panic!("expected folder first");
    };
    assert_eq!(name, "Work");
    assert_eq!(entries.len(), 2);
    assert!(matches!(&entries[0], ParsedEntry::Site(s) if s.name == "Alpha"));
    let ParsedEntry::Folder { name, entries } = &entries[1] else {
        panic!("expected nested folder");
    };
    assert_eq!(name, "Clientes de Produção");
    assert_eq!(entries.len(), 2);
    assert!(matches!(&entries[0], ParsedEntry::Site(s) if s.name == "Beta"));
    let ParsedEntry::Folder { name, entries } = &entries[1] else {
        panic!("expected deep folder");
    };
    assert_eq!(name, "Deep");
    assert!(matches!(&entries[0], ParsedEntry::Site(s) if s.name == "Gamma"));
    assert!(matches!(&parsed.entries[1], ParsedEntry::Site(s) if s.name == "Top level"));
    // Older format: no Port, passwords still decoded.
    assert_eq!(find(&parsed.entries, "Beta").unwrap().port, None);
    assert!(matches!(
        find(&parsed.entries, "Alpha").unwrap().password,
        ParsedPassword::Plain(_)
    ));
}

#[test]
fn accented_names_survive() {
    let s = site("Servidor Café & Açaí");
    assert_eq!(s.host, "café.example.test");
    assert_eq!(s.user, "zoë");
    let parsed = parse("sitemanager-3.40-folders.xml");
    assert!(
        matches!(&parsed.entries[0], ParsedEntry::Folder { entries, .. }
        if matches!(&entries[1], ParsedEntry::Folder { name, .. } if name == "Clientes de Produção"))
    );
}

#[test]
fn broken_xml_is_protocol_and_all_or_nothing() {
    let err = parse_sitemanager(&fixture("sitemanager-broken.xml")).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Protocol);
    assert_eq!(err.message, "Cannot read sitemanager.xml");
    let detail = err.detail.unwrap();
    assert!(detail.contains("line 6"), "{detail}");
    assert!(detail.contains("column"), "{detail}");

    let truncated = "<FileZilla3><Servers><Server><Host>h</Host>";
    let err = parse_sitemanager(truncated).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Protocol);
    assert!(parse_sitemanager("not xml at all <<").is_err());
}

#[test]
fn doctype_and_oversized_input_are_rejected() {
    let xml = "<!DOCTYPE x [<!ENTITY e SYSTEM \"file:///etc/passwd\">]><FileZilla3><Servers/></FileZilla3>";
    let err = parse_sitemanager(xml).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Protocol);

    let big = " ".repeat(16 * 1024 * 1024 + 1);
    let err = parse_sitemanager(&big).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Protocol);
    assert_eq!(err.message, "sitemanager.xml is too large");
}

#[test]
fn read_sitemanager_reads_and_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sitemanager.xml");
    std::fs::write(&path, fixture("sitemanager-3.66.xml")).unwrap();
    assert!(!read_sitemanager(&path).unwrap().entries.is_empty());

    let big = dir.path().join("big.xml");
    let file = std::fs::File::create(&big).unwrap();
    file.set_len(16 * 1024 * 1024 + 1).unwrap();
    let err = read_sitemanager(&big).unwrap_err();
    assert_eq!(err.message, "sitemanager.xml is too large");

    let err = read_sitemanager(&dir.path().join("missing.xml")).unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

#[test]
fn debug_has_no_password() {
    for name in ["sitemanager-3.66.xml", "sitemanager-3.40-folders.xml"] {
        let dbg = format!("{:?}", parse(name));
        assert!(!dbg.contains("hunter2"), "{dbg}");
        assert!(!dbg.contains("senha-"), "{dbg}");
        assert!(!dbg.contains("aHVudGVy"), "{dbg}");
    }
    let s = site("Legacy FTP");
    assert!(!format!("{:?}", s.password).contains("hunter2"));
    assert!(!format!("{s:?}").contains("hunter2"));
}
