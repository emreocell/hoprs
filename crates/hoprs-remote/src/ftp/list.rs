//! Directory listing parsers: the unix `ls -l` format most servers send for `LIST`, and the
//! machine-readable `MLSD` format (RFC 3659). Also the `YYYYMMDDHHMMSS` timestamps of `MDTM`,
//! `MFMT` and the MLSD `modify` fact.
//!
//! Every time is UTC, as RFC 3659 requires for `MDTM`/`MLSD` and as vsftpd sends `LIST` dates
//! unless `use_localtime=YES`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hoprs_core::EntryKind;

/// One parsed listing line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListedEntry {
    pub(crate) name: String,
    /// `None` for FIFOs, sockets, devices and other special types, which callers hide.
    pub(crate) kind: Option<EntryKind>,
    /// Bytes for files; whatever the server said for anything else.
    pub(crate) size: u64,
    pub(crate) modified: Option<SystemTime>,
    /// Permission bits (`0o7777`).
    pub(crate) mode: Option<u32>,
}

/// Parses one line of a unix-format `LIST` reply; `now` resolves dates without a year.
///
/// `None` for `total N`, `.`, `..` and anything that does not look like an `ls -l` line.
///
/// The columns between the permissions and the date (links, owner, group, size) vary between
/// servers, so the line is anchored on the date: a month name, a day and a time or year,
/// preceded by the size. The name is everything after the single space that follows the
/// date, so leading and inner spaces in names survive.
pub(crate) fn parse_list_line(line: &str, now: SystemTime) -> Option<ListedEntry> {
    let (name, kind, size, modified, mode) = parse_list_fields(line, now)?;
    if name == "." || name == ".." {
        return None;
    }
    Some(ListedEntry {
        name,
        kind,
        size,
        modified,
        mode,
    })
}

/// Whether `line` is the `.` entry of a directory listing (sent by `LIST -a`).
pub(crate) fn is_dot_line(line: &str, now: SystemTime) -> bool {
    parse_list_fields(line, now).is_some_and(|(name, ..)| name == ".")
}

type ListFields = (
    String,
    Option<EntryKind>,
    u64,
    Option<SystemTime>,
    Option<u32>,
);

fn parse_list_fields(line: &str, now: SystemTime) -> Option<ListFields> {
    let tokens = tokens(line);
    let perms = tokens.first()?.1;
    let (kind, mode) = parse_perms(perms)?;
    // tokens[i] is the month; the size comes right before it, the day and the time or year
    // right after it.
    let (i, modified) = (2..tokens.len().saturating_sub(2)).find_map(|i| {
        let month = month_number(tokens[i].1)?;
        tokens[i - 1].1.parse::<u64>().ok()?;
        let day: u32 = tokens[i + 1].1.parse().ok()?;
        if !(1..=31).contains(&day) {
            return None;
        }
        let t = parse_list_time(month, day, tokens[i + 2].1, now)?;
        Some((i, t))
    })?;
    let size = tokens[i - 1].1.parse::<u64>().ok()?;
    let (end, last) = tokens[i + 2];
    // One separating space, then the name.
    let rest = line.get(end + last.len() + 1..)?;
    let rest = rest.trim_end_matches(['\r', '\n']);
    let name = match kind {
        Some(EntryKind::Symlink) => rest.split_once(" -> ").map_or(rest, |(n, _)| n),
        _ => rest,
    };
    if name.is_empty() {
        return None;
    }
    Some((name.to_owned(), kind, size, Some(modified), Some(mode)))
}

/// Whitespace-separated tokens with their byte offsets.
fn tokens(line: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in line.char_indices() {
        match (c.is_whitespace(), start) {
            (true, Some(s)) => {
                out.push((s, &line[s..i]));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, &line[s..]));
    }
    out
}

/// `drwxr-xr-x` (optionally followed by an ACL/xattr marker such as `+`, `@` or `.`) into the
/// entry kind and permission bits. Kind `None` for anything but `-`, `d` and `l`.
fn parse_perms(perms: &str) -> Option<(Option<EntryKind>, u32)> {
    let bytes = perms.as_bytes();
    if !(10..=11).contains(&bytes.len()) {
        return None;
    }
    let kind = match bytes[0] {
        b'-' => Some(EntryKind::File),
        b'd' => Some(EntryKind::Dir),
        b'l' => Some(EntryKind::Symlink),
        b'p' | b's' | b'c' | b'b' | b'D' => None,
        _ => return None,
    };
    let mut mode = 0;
    // (read, write, exec) bit of each class, and its set-id/sticky bit.
    for (class, special) in [(6, 0o4000), (3, 0o2000), (0, 0o1000)] {
        let at = 1 + (2 - class / 3) * 3;
        let [r, w, x] = [bytes[at], bytes[at + 1], bytes[at + 2]];
        match r {
            b'r' => mode |= 0o4 << class,
            b'-' => {}
            _ => return None,
        }
        match w {
            b'w' => mode |= 0o2 << class,
            b'-' => {}
            _ => return None,
        }
        match x {
            b'x' => mode |= 0o1 << class,
            b's' | b't' => mode |= (0o1 << class) | special,
            b'S' | b'T' => mode |= special,
            b'-' => {}
            _ => return None,
        }
    }
    Some((kind, mode))
}

fn month_number(s: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let lower = s.to_ascii_lowercase();
    MONTHS
        .iter()
        .position(|m| *m == lower)
        .and_then(|i| u32::try_from(i + 1).ok())
}

/// Clock skew allowed before a date without a year counts as "in the future".
const FUTURE_SLACK: Duration = Duration::from_secs(86_400);

/// `HH:MM` (this year, or last year if that would be more than a day in the future) or a year.
fn parse_list_time(month: u32, day: u32, s: &str, now: SystemTime) -> Option<SystemTime> {
    if let Some((h, m)) = s.split_once(':') {
        let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
        if h > 23 || m > 59 {
            return None;
        }
        let year = civil_from_days(
            i64::try_from(now.duration_since(UNIX_EPOCH).ok()?.as_secs() / 86_400).ok()?,
        )
        .0;
        let this_year = to_system_time(year, month, day, h, m, 0)?;
        if this_year > now + FUTURE_SLACK {
            to_system_time(year - 1, month, day, h, m, 0)
        } else {
            Some(this_year)
        }
    } else if s.len() == 4 {
        to_system_time(s.parse().ok()?, month, day, 0, 0, 0)
    } else {
        None
    }
}

/// Parses one `MLSD` line (`fact=value;...; name`). `None` for `cdir`/`pdir`, `.`, `..` and
/// malformed lines. A line without a `type` fact is a file; unknown types are special.
pub(crate) fn parse_mlsd_line(line: &str) -> Option<ListedEntry> {
    let line = line.trim_end_matches(['\r', '\n']);
    let (facts, name) = line.split_once(' ')?;
    if name.is_empty() || name == "." || name == ".." || !facts.contains('=') {
        return None;
    }
    let mut entry = ListedEntry {
        name: name.to_owned(),
        kind: Some(EntryKind::File),
        size: 0,
        modified: None,
        mode: None,
    };
    for fact in facts.split(';').filter(|f| !f.is_empty()) {
        let (key, value) = fact.split_once('=')?;
        match key.to_ascii_lowercase().as_str() {
            "type" => {
                let value = value.to_ascii_lowercase();
                entry.kind = match value.as_str() {
                    "file" => Some(EntryKind::File),
                    "dir" => Some(EntryKind::Dir),
                    "cdir" | "pdir" => return None,
                    v if v.starts_with("os.unix=slink") || v == "os.unix=symlink" => {
                        Some(EntryKind::Symlink)
                    }
                    _ => None,
                };
            }
            "size" => entry.size = value.parse().ok()?,
            "modify" => entry.modified = parse_timeval(value),
            "unix.mode" => entry.mode = u32::from_str_radix(value, 8).ok().map(|m| m & 0o7777),
            _ => {}
        }
    }
    Some(entry)
}

/// Parses `YYYYMMDDHHMMSS` with optional `.fff` fractions (UTC).
pub(crate) fn parse_timeval(s: &str) -> Option<SystemTime> {
    let (whole, fraction) = s.split_once('.').unwrap_or((s, ""));
    if whole.len() != 14 || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let num = |r: std::ops::Range<usize>| whole[r].parse::<u32>().ok();
    let year = i64::from(num(0..4)?);
    let t = to_system_time(
        year,
        num(4..6)?,
        num(6..8)?,
        num(8..10)?,
        num(10..12)?,
        num(12..14)?,
    )?;
    if fraction.is_empty() {
        return Some(t);
    }
    if !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Up to nanoseconds; extra digits are ignored.
    let digits = &fraction[..fraction.len().min(9)];
    let nanos: u32 = format!("{digits:0<9}").parse().ok()?;
    Some(t + Duration::from_nanos(u64::from(nanos)))
}

/// Formats `t` as `YYYYMMDDHHMMSS` (UTC), dropping fractions. `None` before 1970 or after
/// year 9999.
pub(crate) fn format_timeval(t: SystemTime) -> Option<String> {
    let secs = t.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let days = i64::try_from(secs / 86_400).ok()?;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    if y > 9999 {
        return None;
    }
    Some(format!(
        "{y:04}{m:02}{d:02}{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    ))
}

/// A UTC date and time, validated. `None` before 1970.
fn to_system_time(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Option<SystemTime> {
    if !(1..=12).contains(&mo) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let days = days_from_civil(y, mo, d);
    // Rejects days past the end of the month (Feb 30 would be Mar 1).
    if civil_from_days(days) != (y, mo, d) {
        return None;
    }
    let secs = days * 86_400 + i64::from(h * 3600 + mi * 60 + s);
    Some(UNIX_EPOCH + Duration::from_secs(u64::try_from(secs).ok()?))
}

/// Proleptic Gregorian date of a day count since 1970-01-01 (H. Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-12 13:56:00 UTC.
    fn now() -> SystemTime {
        at(2026, 11, 12, 13, 56, 0)
    }

    fn at(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> SystemTime {
        let days = days_from_civil(y, mo, d);
        let secs = days * 86_400 + i64::from(h * 3600 + mi * 60 + s);
        UNIX_EPOCH + Duration::from_secs(u64::try_from(secs).unwrap())
    }

    fn parse(line: &str) -> ListedEntry {
        parse_list_line(line, now()).unwrap_or_else(|| panic!("no entry for {line:?}"))
    }

    #[test]
    fn file_with_spaces_and_unicode() {
        let e =
            parse("-rw-r--r--    1 1101     1101       123456 Nov 12 13:56 relatório final.pdf");
        assert_eq!(e.name, "relatório final.pdf");
        assert_eq!(e.kind, Some(EntryKind::File));
        assert_eq!(e.size, 123_456);
        assert_eq!(e.mode, Some(0o644));
        assert_eq!(e.modified, Some(at(2026, 11, 12, 13, 56, 0)));
        let e = parse("-rw-r--r--    1 1101     1101            0 Nov 12 13:56  two  spaces ");
        assert_eq!(e.name, " two  spaces ");
        let e = parse("-rw-r--r--    1 u g 3 Jan 02 03:04 日本語.txt");
        assert_eq!(e.name, "日本語.txt");
    }

    #[test]
    fn dot_files_are_kept_but_dot_entries_are_not() {
        let e = parse("-rw-------    1 1101     1101           42 Nov 12 13:56 .env");
        assert_eq!(e.name, ".env");
        assert_eq!(e.mode, Some(0o600));
        for name in [".", ".."] {
            let line = format!("drwxr-xr-x    4 1101     1101         4096 Nov 12 13:56 {name}");
            assert_eq!(parse_list_line(&line, now()), None, "{name}");
        }
    }

    #[test]
    fn directory() {
        let e = parse("drwxr-x---    3 1101     1101         4096 Nov 12 13:56 d");
        assert_eq!(e.kind, Some(EntryKind::Dir));
        assert_eq!(e.mode, Some(0o750));
    }

    #[test]
    fn symlink_name_stops_before_the_arrow() {
        let e =
            parse("lrwxrwxrwx    1 1101     1101           11 Nov 12 13:56 link -> /etc/passwd");
        assert_eq!(e.name, "link");
        assert_eq!(e.kind, Some(EntryKind::Symlink));
        let e = parse("lrwxrwxrwx    1 u g 1 Nov 12 13:56 a b -> c d");
        assert_eq!(e.name, "a b");
        // Only symlinks are split: " -> " is a legal part of a file name.
        let e = parse("-rw-r--r--    1 u g 1 Nov 12 13:56 x -> y");
        assert_eq!(e.name, "x -> y");
    }

    #[test]
    fn time_without_year_is_this_year_or_the_last() {
        // Earlier this year.
        let e = parse("-rw-r--r--    1 u g 1 Mar  5 08:09 f");
        assert_eq!(e.modified, Some(at(2026, 3, 5, 8, 9, 0)));
        // Today, a little later than `now` (clock skew): still this year.
        let e = parse("-rw-r--r--    1 u g 1 Nov 12 15:00 f");
        assert_eq!(e.modified, Some(at(2026, 11, 12, 15, 0, 0)));
        // December would be in the future: last year.
        let e = parse("-rw-r--r--    1 u g 1 Dec 24 18:30 f");
        assert_eq!(e.modified, Some(at(2025, 12, 24, 18, 30, 0)));
    }

    #[test]
    fn date_with_year() {
        let e = parse("-rw-r--r--    1 u g 1 Jan 12  1970 f");
        assert_eq!(e.modified, Some(at(1970, 1, 12, 0, 0, 0)));
        let e = parse("-rw-r--r--    1 u g 1 Feb 29  2024 leap");
        assert_eq!(e.modified, Some(at(2024, 2, 29, 0, 0, 0)));
    }

    #[test]
    fn listing_without_group_column() {
        let e = parse("-rw-r--r--    1 owner        7 Nov 12 13:56 f");
        assert_eq!((e.name.as_str(), e.size), ("f", 7));
    }

    #[test]
    fn special_bits_and_acl_marker() {
        let e = parse("-rwsr-sr-t+   1 u g 1 Nov 12 13:56 f");
        assert_eq!(e.mode, Some(0o7755));
        let e = parse("drwxr-S--T    1 u g 1 Nov 12 13:56 d");
        assert_eq!(e.mode, Some(0o3740));
    }

    #[test]
    fn special_files_have_no_kind() {
        for line in [
            "prw-r--r--    1 1101     1101            0 Nov 12 13:56 fifo",
            "srwxr-xr-x    1 1101     1101            0 Nov 12 13:56 sock",
            "crw-rw-rw-    1 0        0          1,   3 Nov 12 13:56 null",
            "brw-rw----    1 0        6          8,   0 Nov 12 13:56 sda",
        ] {
            let e = parse(line);
            assert_eq!(e.kind, None, "{line}");
        }
        assert_eq!(
            parse("crw-rw-rw-    1 0        0          1,   3 Nov 12 13:56 null").name,
            "null"
        );
    }

    #[test]
    fn garbage_is_none() {
        for line in [
            "total 8",
            "",
            "   ",
            "hello world",
            "-rw-r--r-- 1 u g notanumber Nov 12 13:56 f",
            "-rw-r--r-- 1 u g 1 Foo 12 13:56 f",
            "-rw-r--r-- 1 u g 1 Nov 32 13:56 f",
            "-rw-r--r-- 1 u g 1 Nov 12 25:61 f",
            "-rw-r--r-- 1 u g 1 Nov 12 13:56",
            "-rw-r--r-- 1 u g 1 Nov 12 13:56 ",
            "xrw-r--r-- 1 u g 1 Nov 12 13:56 f",
            "-rw-r--r-",
            "226 Directory send OK.",
        ] {
            assert_eq!(parse_list_line(line, now()), None, "{line:?}");
        }
    }

    #[test]
    fn mlsd_lines() {
        let e = parse_mlsd_line(
            "type=file;size=5;modify=20261112135648;UNIX.mode=0644;perm=adfrw; relatório final.pdf",
        )
        .unwrap();
        assert_eq!(e.name, "relatório final.pdf");
        assert_eq!(e.kind, Some(EntryKind::File));
        assert_eq!(e.size, 5);
        assert_eq!(e.modified, Some(at(2026, 11, 12, 13, 56, 48)));
        assert_eq!(e.mode, Some(0o644));

        let e = parse_mlsd_line("Type=dir;Modify=20200101000000.123; d").unwrap();
        assert_eq!(e.kind, Some(EntryKind::Dir));
        assert_eq!(
            e.modified,
            Some(at(2020, 1, 1, 0, 0, 0) + Duration::from_millis(123))
        );
        assert_eq!(e.mode, None);

        let e = parse_mlsd_line("type=OS.unix=slink:/etc/passwd; link").unwrap();
        assert_eq!(e.kind, Some(EntryKind::Symlink));
        let e = parse_mlsd_line("type=OS.unix=symlink; link").unwrap();
        assert_eq!(e.kind, Some(EntryKind::Symlink));
        let e = parse_mlsd_line("type=OS.unix=fifo; fifo").unwrap();
        assert_eq!(e.kind, None);
        // No type fact: a file.
        let e = parse_mlsd_line("size=3; .env").unwrap();
        assert_eq!((e.name.as_str(), e.kind), (".env", Some(EntryKind::File)));
        let e = parse_mlsd_line("type=file;size=1;  lead").unwrap();
        assert_eq!(e.name, " lead");

        for line in [
            "type=cdir; /home/frank",
            "type=pdir; /home",
            "type=dir; .",
            "type=dir; ..",
            "no facts here",
            "type=file;",
            "type=file; ",
            "",
        ] {
            assert_eq!(parse_mlsd_line(line), None, "{line:?}");
        }
    }

    #[test]
    fn timevals() {
        assert_eq!(
            parse_timeval("20261112135648"),
            Some(at(2026, 11, 12, 13, 56, 48))
        );
        assert_eq!(
            parse_timeval("19700101000001.5"),
            Some(UNIX_EPOCH + Duration::from_millis(1500))
        );
        for bad in [
            "",
            "2026111213564",
            "2026131213564x",
            "20261332000000",
            "abc",
        ] {
            assert_eq!(parse_timeval(bad), None, "{bad}");
        }
        assert_eq!(
            format_timeval(at(2026, 11, 12, 13, 56, 48)).as_deref(),
            Some("20261112135648")
        );
        assert_eq!(
            format_timeval(UNIX_EPOCH + Duration::from_secs(1_000_000)).as_deref(),
            Some("19700112134640")
        );
        assert_eq!(format_timeval(UNIX_EPOCH - Duration::from_secs(1)), None);
    }
}
