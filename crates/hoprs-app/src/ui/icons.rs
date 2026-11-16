//! Phosphor icons (MIT) as a font: `Icon` names a glyph of the subset in
//! `assets/fonts/Phosphor-hoprs.ttf`, built by `scripts/icons-subset.sh` from
//! `assets/fonts/phosphor-glyphs.txt`.

use hoprs_core::EntryKind;
use iced::Font;
use iced::widget::{Text, text};

/// The family name inside the subset font.
pub const ICON_FONT: Font = Font::with_name("Phosphor hoprs");
/// The subset font file.
pub const ICON_BYTES: &[u8] = include_bytes!("../../assets/fonts/Phosphor-hoprs.ttf");
/// Icon size beside a file or list row label.
pub const ROW: f32 = 16.0;
/// Icon size inside small controls.
pub const SMALL: f32 = 15.0;

/// One glyph of the icon font.
// Variants for menus (Plan 13 tasks 13-14) are named by the font check only until then.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Folder,
    FolderPlus,
    FolderSimplePlus,
    File,
    FileText,
    FileCode,
    FileHtml,
    FileImage,
    FilePdf,
    FileZip,
    FileArchive,
    ArrowBendLeftUp,
    UploadSimple,
    DownloadSimple,
    PencilSimple,
    PencilSimpleLine,
    Textbox,
    Trash,
    LockKey,
    LockSimple,
    GitDiff,
    Copy,
    ArrowsClockwise,
    Desktop,
    HardDrives,
    TerminalWindow,
    Stack,
    PlugsConnected,
    Plug,
    Warning,
    MagnifyingGlass,
    Plus,
    X,
    CaretDown,
    CaretRight,
    CaretUp,
    DotsThree,
    GearSix,
    SlidersHorizontal,
    ArrowsDownUp,
    ShieldCheck,
    Scroll,
    Broom,
    Pause,
    Play,
    ArrowUp,
    ArrowDown,
    ArrowsLeftRight,
    Files,
    WarningCircle,
    Check,
}

impl Icon {
    #[cfg(test)] // The font check lists every icon.
    pub const ALL: [Icon; 51] = [
        Icon::Folder,
        Icon::FolderPlus,
        Icon::FolderSimplePlus,
        Icon::File,
        Icon::FileText,
        Icon::FileCode,
        Icon::FileHtml,
        Icon::FileImage,
        Icon::FilePdf,
        Icon::FileZip,
        Icon::FileArchive,
        Icon::ArrowBendLeftUp,
        Icon::UploadSimple,
        Icon::DownloadSimple,
        Icon::PencilSimple,
        Icon::PencilSimpleLine,
        Icon::Textbox,
        Icon::Trash,
        Icon::LockKey,
        Icon::LockSimple,
        Icon::GitDiff,
        Icon::Copy,
        Icon::ArrowsClockwise,
        Icon::Desktop,
        Icon::HardDrives,
        Icon::TerminalWindow,
        Icon::Stack,
        Icon::PlugsConnected,
        Icon::Plug,
        Icon::Warning,
        Icon::MagnifyingGlass,
        Icon::Plus,
        Icon::X,
        Icon::CaretDown,
        Icon::CaretRight,
        Icon::CaretUp,
        Icon::DotsThree,
        Icon::GearSix,
        Icon::SlidersHorizontal,
        Icon::ArrowsDownUp,
        Icon::ShieldCheck,
        Icon::Scroll,
        Icon::Broom,
        Icon::Pause,
        Icon::Play,
        Icon::ArrowUp,
        Icon::ArrowDown,
        Icon::ArrowsLeftRight,
        Icon::Files,
        Icon::WarningCircle,
        Icon::Check,
    ];

    /// The Unicode private-use codepoint of the glyph.
    pub const fn codepoint(self) -> char {
        match self {
            Icon::Folder => '\u{e24a}',
            Icon::FolderPlus => '\u{e258}',
            Icon::FolderSimplePlus => '\u{e25e}',
            Icon::File => '\u{e230}',
            Icon::FileText => '\u{e23a}',
            Icon::FileCode => '\u{e914}',
            Icon::FileHtml => '\u{eb38}',
            Icon::FileImage => '\u{ea24}',
            Icon::FilePdf => '\u{e702}',
            Icon::FileZip => '\u{e958}',
            Icon::FileArchive => '\u{eb2a}',
            Icon::ArrowBendLeftUp => '\u{e01e}',
            Icon::UploadSimple => '\u{e4c0}',
            Icon::DownloadSimple => '\u{e20c}',
            Icon::PencilSimple => '\u{e3b4}',
            Icon::PencilSimpleLine => '\u{ebc6}',
            Icon::Textbox => '\u{eb0a}',
            Icon::Trash => '\u{e4a6}',
            Icon::LockKey => '\u{e2fe}',
            Icon::LockSimple => '\u{e308}',
            Icon::GitDiff => '\u{e27c}',
            Icon::Copy => '\u{e1ca}',
            Icon::ArrowsClockwise => '\u{e094}',
            Icon::Desktop => '\u{e560}',
            Icon::HardDrives => '\u{e2a0}',
            Icon::TerminalWindow => '\u{eae8}',
            Icon::Stack => '\u{e466}',
            Icon::PlugsConnected => '\u{eb5a}',
            Icon::Plug => '\u{e946}',
            Icon::Warning => '\u{e4e0}',
            Icon::MagnifyingGlass => '\u{e30c}',
            Icon::Plus => '\u{e3d4}',
            Icon::X => '\u{e4f6}',
            Icon::CaretDown => '\u{e136}',
            Icon::CaretRight => '\u{e13a}',
            Icon::CaretUp => '\u{e13c}',
            Icon::DotsThree => '\u{e1fe}',
            Icon::GearSix => '\u{e272}',
            Icon::SlidersHorizontal => '\u{e434}',
            Icon::ArrowsDownUp => '\u{e098}',
            Icon::ShieldCheck => '\u{e40c}',
            Icon::Scroll => '\u{eb7a}',
            Icon::Broom => '\u{ec54}',
            Icon::Pause => '\u{e39e}',
            Icon::Play => '\u{e3d0}',
            Icon::ArrowUp => '\u{e08e}',
            Icon::ArrowDown => '\u{e03e}',
            Icon::ArrowsLeftRight => '\u{e0a0}',
            Icon::Files => '\u{e710}',
            Icon::WarningCircle => '\u{e4e2}',
            Icon::Check => '\u{e182}',
        }
    }

    /// The Phosphor kebab-case name.
    #[cfg(test)]
    pub const fn name(self) -> &'static str {
        match self {
            Icon::Folder => "folder",
            Icon::FolderPlus => "folder-plus",
            Icon::FolderSimplePlus => "folder-simple-plus",
            Icon::File => "file",
            Icon::FileText => "file-text",
            Icon::FileCode => "file-code",
            Icon::FileHtml => "file-html",
            Icon::FileImage => "file-image",
            Icon::FilePdf => "file-pdf",
            Icon::FileZip => "file-zip",
            Icon::FileArchive => "file-archive",
            Icon::ArrowBendLeftUp => "arrow-bend-left-up",
            Icon::UploadSimple => "upload-simple",
            Icon::DownloadSimple => "download-simple",
            Icon::PencilSimple => "pencil-simple",
            Icon::PencilSimpleLine => "pencil-simple-line",
            Icon::Textbox => "textbox",
            Icon::Trash => "trash",
            Icon::LockKey => "lock-key",
            Icon::LockSimple => "lock-simple",
            Icon::GitDiff => "git-diff",
            Icon::Copy => "copy",
            Icon::ArrowsClockwise => "arrows-clockwise",
            Icon::Desktop => "desktop",
            Icon::HardDrives => "hard-drives",
            Icon::TerminalWindow => "terminal-window",
            Icon::Stack => "stack",
            Icon::PlugsConnected => "plugs-connected",
            Icon::Plug => "plug",
            Icon::Warning => "warning",
            Icon::MagnifyingGlass => "magnifying-glass",
            Icon::Plus => "plus",
            Icon::X => "x",
            Icon::CaretDown => "caret-down",
            Icon::CaretRight => "caret-right",
            Icon::CaretUp => "caret-up",
            Icon::DotsThree => "dots-three",
            Icon::GearSix => "gear-six",
            Icon::SlidersHorizontal => "sliders-horizontal",
            Icon::ArrowsDownUp => "arrows-down-up",
            Icon::ShieldCheck => "shield-check",
            Icon::Scroll => "scroll",
            Icon::Broom => "broom",
            Icon::Pause => "pause",
            Icon::Play => "play",
            Icon::ArrowUp => "arrow-up",
            Icon::ArrowDown => "arrow-down",
            Icon::ArrowsLeftRight => "arrows-left-right",
            Icon::Files => "files",
            Icon::WarningCircle => "warning-circle",
            Icon::Check => "check",
        }
    }
}

/// A glyph as text in the icon font; the colour is left to the caller.
pub fn icon<'a>(icon: Icon, size: f32) -> Text<'a> {
    text(icon.codepoint().to_string())
        .font(ICON_FONT)
        .size(size)
        .line_height(1.0)
}

/// The icon for a file-list row: folders, then files by extension (case-insensitive).
/// Symlinks are shown as files.
pub fn for_entry(name: &str, kind: EntryKind) -> Icon {
    if kind == EntryKind::Dir {
        return Icon::Folder;
    }
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return Icon::File;
    };
    if stem.is_empty() {
        return Icon::File;
    }
    match ext.to_ascii_lowercase().as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "avif" | "bmp" | "ico" | "tif"
        | "tiff" => Icon::FileImage,
        "pdf" => Icon::FilePdf,
        "zip" => Icon::FileZip,
        "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "wpress" => Icon::FileArchive,
        "html" | "htm" => Icon::FileHtml,
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" | "go" | "java" | "kt" | "c"
        | "h" | "cc" | "cpp" | "hpp" | "cs" | "php" | "rb" | "sh" | "bash" | "zsh" | "fish"
        | "ps1" | "css" | "scss" | "less" | "json" | "toml" | "yaml" | "yml" | "xml" | "sql"
        | "lua" | "swift" | "vue" => Icon::FileCode,
        "txt" | "md" | "log" | "csv" | "ini" | "conf" | "cfg" | "env" => Icon::FileText,
        _ => Icon::File,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_icon_is_in_the_font() {
        let face = ttf_parser::Face::parse(ICON_BYTES, 0).expect("font parses");
        for icon in Icon::ALL {
            assert!(
                face.glyph_index(icon.codepoint()).is_some(),
                "{} (U+{:04X}) missing from Phosphor-hoprs.ttf",
                icon.name(),
                icon.codepoint() as u32
            );
        }
    }

    #[test]
    fn font_family_is_phosphor_hoprs() {
        let face = ttf_parser::Face::parse(ICON_BYTES, 0).expect("font parses");
        let family = face
            .names()
            .into_iter()
            .filter(|n| n.name_id == ttf_parser::name_id::FAMILY)
            .find_map(|n| n.to_string());
        assert_eq!(family.as_deref(), Some("Phosphor hoprs"));
    }

    #[test]
    fn glyph_list_matches_the_enum() {
        let listed: BTreeSet<(String, u32)> =
            include_str!("../../assets/fonts/phosphor-glyphs.txt")
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| {
                    let (name, cp) = l.split_once(' ').expect("name and codepoint");
                    let cp = cp.trim().strip_prefix("U+").expect("U+ prefix");
                    (name.to_string(), u32::from_str_radix(cp, 16).unwrap())
                })
                .collect();
        let coded: BTreeSet<(String, u32)> = Icon::ALL
            .iter()
            .map(|i| (i.name().to_string(), i.codepoint() as u32))
            .collect();
        assert_eq!(listed, coded);
        assert_eq!(Icon::ALL.len(), coded.len(), "no duplicate icons");
    }

    #[test]
    fn file_kinds_by_extension() {
        let file = |n: &str| for_entry(n, EntryKind::File);
        assert_eq!(file("photo.JPG"), Icon::FileImage);
        assert_eq!(file("a.pdf"), Icon::FilePdf);
        assert_eq!(file("a.zip"), Icon::FileZip);
        for n in ["site.wpress", "b.tar.gz", "c.7z"] {
            assert_eq!(file(n), Icon::FileArchive, "{n}");
        }
        assert_eq!(file("index.html"), Icon::FileHtml);
        for n in ["main.rs", "app.tsx", "deploy.sh", "x.json"] {
            assert_eq!(file(n), Icon::FileCode, "{n}");
        }
        for n in ["README.md", "access.log", "notes.txt"] {
            assert_eq!(file(n), Icon::FileText, "{n}");
        }
        for n in ["Makefile", "blob.bin", ".env.local"] {
            assert_eq!(file(n), Icon::File, "{n}");
        }
        assert_eq!(for_entry("photo.jpg", EntryKind::Dir), Icon::Folder);
        assert_eq!(for_entry("link.rs", EntryKind::Symlink), Icon::FileCode);
    }
}
