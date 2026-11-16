//! What fits at a given window width, down to the 960 px minimum: which file table
//! columns show, which form the pane toolbars use, and whether the queue head is compact.
//! Pure functions of the window width the app last saw ([`crate::app::App::window_width`]).

use std::borrow::Cow;

use crate::backend::Side;
use crate::ui::sidebar;

/// Fixed file table columns; the name takes the rest.
pub const SIZE_WIDTH: f32 = 88.0;
pub const MODIFIED_WIDTH: f32 = 116.0;
pub const PERMISSIONS_WIDTH: f32 = 110.0;
/// The name column never gets less than this while another column can still be dropped
/// (Permissions first, then Modified).
pub const NAME_MIN: f32 = 120.0;

/// From these pane widths the toolbar is [`Toolbar::Full`], labels and key hints (the
/// remote toolbar, with Edit, needs ~490 px; the local one ~345 px): both panes of the
/// default 1280 px window, the remote one down to a ~1240 px window. Narrower panes get
/// [`Toolbar::Compact`] (no hints) down to [`TOOLBAR_NARROW`], then [`Toolbar::Narrow`].
const TOOLBAR_FULL_REMOTE: f32 = 492.0;
/// The same in Portuguese, whose labels are wider ("Renomear", "Nova pasta",
/// "Permissões…"): measured at ~510 px, so the default 1280 px window (512 px panes) just fits.
const TOOLBAR_FULL_REMOTE_PT: f32 = 512.0;
const TOOLBAR_FULL_LOCAL: f32 = 360.0;
/// Below this pane width (either side) the toolbar also uses short labels ("Folder",
/// "Perms…"): without them the six remote actions need ~385 px, more than the 352 px
/// panes of the 960 px minimum window.
const TOOLBAR_NARROW: f32 = 400.0;
/// Below this center width the queue head uses short labels.
const QUEUE_HEAD_FULL: f32 = 900.0;

/// Advance of JetBrains Mono, in em: exact, every glyph has it.
pub const MONO_ADVANCE: f32 = 0.6;
/// Advance of Inter, in em: an average (the glyphs vary), so the container still clips.
pub const INTER_ADVANCE: f32 = 0.56;

/// `text` cut to fit `width` px at `size` px with a per-character `advance` (em), ending in
/// `…` when it had to be cut; the text itself, borrowed, when it fits. Counts characters.
pub fn ellipsize(text: &str, width: f32, size: f32, advance: f32) -> Cow<'_, str> {
    // The epsilon absorbs f32 rounding (75 / 7.5 must be 10, not 9.999).
    let budget = (width / (size * advance) + 1e-3).floor().max(0.0) as usize;
    let total = text.chars().count();
    if total <= budget {
        return Cow::Borrowed(text);
    }
    let kept: String = text.chars().take(budget.saturating_sub(1)).collect();
    Cow::Owned(format!("{kept}\u{2026}"))
}

/// The columns a file table shows besides Name and Size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Columns {
    pub modified: bool,
    /// Remote side only.
    pub permissions: bool,
}

/// The width right of the sidebar.
pub fn center_width(window: f32) -> f32 {
    (window - sidebar::WIDTH).max(0.0)
}

/// One of the two equal file panes.
pub fn pane_width(window: f32) -> f32 {
    center_width(window) / 2.0
}

/// The columns that fit in a pane `pane` px wide, keeping at least [`NAME_MIN`] for the
/// name: Permissions goes first, then Modified; Size always stays.
pub fn columns(pane: f32, side: Side) -> Columns {
    let room = pane - NAME_MIN - SIZE_WIDTH;
    let modified = room >= MODIFIED_WIDTH;
    let permissions = side == Side::Remote && room >= MODIFIED_WIDTH + PERMISSIONS_WIDTH;
    Columns {
        modified,
        permissions,
    }
}

/// How a pane's toolbar reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toolbar {
    /// Labels and key hints.
    Full,
    /// Labels only.
    Compact,
    /// Short labels only (the narrowest panes).
    Narrow,
}

impl Toolbar {
    /// Whether the key hints show.
    pub fn hints(self) -> bool {
        self == Self::Full
    }

    /// Whether the short labels replace the long ones.
    pub fn short_labels(self) -> bool {
        self == Self::Narrow
    }
}

/// The toolbar form for a pane `pane` px wide.
pub fn toolbar(pane: f32, side: Side) -> Toolbar {
    let full = match side {
        Side::Local => TOOLBAR_FULL_LOCAL,
        Side::Remote if crate::i18n::language() == crate::i18n::Language::PtBr => {
            TOOLBAR_FULL_REMOTE_PT
        }
        Side::Remote => TOOLBAR_FULL_REMOTE,
    };
    if pane >= full {
        Toolbar::Full
    } else if pane >= TOOLBAR_NARROW {
        Toolbar::Compact
    } else {
        Toolbar::Narrow
    }
}

/// Whether the queue head uses its short labels at this window width.
pub fn compact_queue(window: f32) -> bool {
    center_width(window) < QUEUE_HEAD_FULL
}

/// How the queue head shows the session's conflict policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyNote {
    /// "Conflicts: overwrite (this session)".
    Full,
    /// The short mono tag "overwrite: session".
    Tag,
}

/// What the queue head shows at a window width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueHead {
    /// Short labels, no worker count, tighter gaps.
    pub compact: bool,
    /// The "Transfers" title.
    pub title: bool,
    /// The session's conflict policy, if one is set.
    pub policy: Option<PolicyNote>,
}

/// The queue head at this window width, with or without a session policy to show. The
/// policy is never hidden: a narrow head shows its tag instead of the title.
pub fn queue_head(window: f32, has_policy: bool) -> QueueHead {
    let compact = compact_queue(window);
    let policy = has_policy.then_some(if compact {
        PolicyNote::Tag
    } else {
        PolicyNote::Full
    });
    QueueHead {
        compact,
        title: policy != Some(PolicyNote::Tag),
        policy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Columns = Columns {
        modified: true,
        permissions: true,
    };

    #[test]
    fn ellipsize_keeps_text_that_fits() {
        let out = ellipsize("short.txt", 200.0, 12.5, MONO_ADVANCE);
        assert!(matches!(out, Cow::Borrowed("short.txt")));
    }

    #[test]
    fn ellipsize_cuts_with_an_ellipsis() {
        // 75 / (12.5 * 0.6) = 10 characters.
        let out = ellipsize("public_html_backup_2026", 75.0, 12.5, MONO_ADVANCE);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('\u{2026}'));
        assert!(out.starts_with("public_ht"));
    }

    #[test]
    fn ellipsize_counts_chars_not_bytes() {
        // 4 characters, 6 bytes; a budget of 4 keeps all of them.
        let width = 4.0 * 10.0 * MONO_ADVANCE;
        assert!(matches!(
            ellipsize("a\u{e7}\u{e3}o", width, 10.0, MONO_ADVANCE),
            Cow::Borrowed(_)
        ));
        let out = ellipsize("a\u{e7}\u{e3}o!!", width, 10.0, MONO_ADVANCE);
        assert_eq!(out, "a\u{e7}\u{e3}\u{2026}");
    }

    #[test]
    fn ellipsize_zero_width_is_just_the_ellipsis() {
        assert_eq!(ellipsize("anything", 0.0, 12.5, MONO_ADVANCE), "\u{2026}");
        assert_eq!(ellipsize("anything", 5.0, 12.5, INTER_ADVANCE), "\u{2026}");
    }

    #[test]
    fn columns_drop_permissions_then_modified() {
        // 1280 px window: both panes ~512 px, everything fits.
        assert_eq!(columns(pane_width(1280.0), Side::Remote), ALL);
        assert_eq!(
            columns(pane_width(1280.0), Side::Local),
            Columns {
                modified: true,
                permissions: false
            }
        );
        // 960 px (the minimum): 352 px panes; the remote side drops Permissions only.
        let narrow = pane_width(960.0);
        assert_eq!(narrow, 352.0);
        assert_eq!(
            columns(narrow, Side::Remote),
            Columns {
                modified: true,
                permissions: false
            }
        );
        assert!(columns(narrow, Side::Local).modified);
        // The exact thresholds.
        let full = NAME_MIN + SIZE_WIDTH + MODIFIED_WIDTH + PERMISSIONS_WIDTH;
        assert_eq!(columns(full, Side::Remote), ALL);
        assert!(!columns(full - 1.0, Side::Remote).permissions);
        let with_modified = NAME_MIN + SIZE_WIDTH + MODIFIED_WIDTH;
        assert!(columns(with_modified, Side::Local).modified);
        assert_eq!(
            columns(with_modified - 1.0, Side::Remote),
            Columns {
                modified: false,
                permissions: false
            }
        );
    }

    #[test]
    fn name_keeps_its_minimum_while_columns_can_drop() {
        for pane in (200..1200).map(|w| w as f32) {
            let c = columns(pane, Side::Remote);
            let fixed = SIZE_WIDTH
                + if c.modified { MODIFIED_WIDTH } else { 0.0 }
                + if c.permissions {
                    PERMISSIONS_WIDTH
                } else {
                    0.0
                };
            if c.modified {
                assert!(pane - fixed >= NAME_MIN, "{pane}: {c:?}");
            }
        }
    }

    /// The Portuguese labels are wider: the remote toolbar keeps its hints at the default
    /// window (512 px panes, as measured) but drops them from a ~1240 px window, where
    /// "Permissões…" would be cut.
    #[test]
    fn remote_toolbar_needs_more_room_in_portuguese() {
        crate::i18n::set_language(crate::i18n::Language::PtBr);
        assert_eq!(toolbar(pane_width(1280.0), Side::Remote), Toolbar::Full);
        assert_eq!(toolbar(pane_width(1240.0), Side::Remote), Toolbar::Compact);
        assert_eq!(toolbar(pane_width(1280.0), Side::Local), Toolbar::Full);
        crate::i18n::set_language(crate::i18n::Language::En);
    }

    #[test]
    fn toolbars_and_queue_head_go_compact_when_narrow() {
        // The remote toolbar has Edit too (Plan 6 Task 8); both keep their key hints at
        // the default 1280 px window (Plan 6 Task 11), the remote one down to ~1240 px.
        assert_eq!(toolbar(pane_width(1280.0), Side::Remote), Toolbar::Full);
        assert_eq!(toolbar(pane_width(1240.0), Side::Remote), Toolbar::Full);
        assert_eq!(toolbar(pane_width(1200.0), Side::Remote), Toolbar::Compact);
        assert_eq!(toolbar(pane_width(1280.0), Side::Local), Toolbar::Full);
        assert_eq!(toolbar(pane_width(1120.0), Side::Remote), Toolbar::Compact);
        // At the 960 px minimum (352 px panes) both toolbars use the short labels, so
        // all six remote actions fit unclipped ("Permissions…" was cut there).
        assert_eq!(toolbar(pane_width(960.0), Side::Remote), Toolbar::Narrow);
        assert_eq!(toolbar(pane_width(960.0), Side::Local), Toolbar::Narrow);
        assert!(!Toolbar::Narrow.hints() && !Toolbar::Compact.hints() && Toolbar::Full.hints());
        assert!(Toolbar::Narrow.short_labels() && !Toolbar::Compact.short_labels());
        assert!(!compact_queue(1280.0));
        assert!(compact_queue(960.0));
        assert_eq!(center_width(100.0), 0.0);
    }

    /// The session's conflict policy never disappears from the queue head: a narrow
    /// window shows it as a short mono tag, making room by dropping the title.
    #[test]
    fn queue_head_keeps_the_session_policy_when_narrow() {
        let wide = queue_head(1280.0, true);
        assert_eq!(
            wide,
            QueueHead {
                compact: false,
                title: true,
                policy: Some(PolicyNote::Full)
            }
        );
        let narrow = queue_head(960.0, true);
        assert_eq!(
            narrow,
            QueueHead {
                compact: true,
                title: false,
                policy: Some(PolicyNote::Tag)
            }
        );
        // The threshold: the full note while the head is not compact.
        let edge = sidebar::WIDTH + QUEUE_HEAD_FULL;
        assert_eq!(queue_head(edge, true).policy, Some(PolicyNote::Full));
        assert_eq!(queue_head(edge - 1.0, true).policy, Some(PolicyNote::Tag));
        // No policy: nothing to show, and the title stays.
        for width in [960.0, 1280.0] {
            let head = queue_head(width, false);
            assert_eq!((head.title, head.policy), (true, None));
        }
        assert_eq!(
            crate::ui::strings::session_policy_tag(hoprs_transfer::ConflictPolicy::Overwrite),
            "overwrite: session"
        );
        assert_eq!(
            crate::ui::strings::session_policy_tag(
                hoprs_transfer::ConflictPolicy::OverwriteIfNewer
            ),
            "if newer: session"
        );
    }
}
