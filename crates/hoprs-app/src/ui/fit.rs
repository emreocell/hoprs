//! What fits at a given window width, down to the 960 px minimum: which file table
//! columns show, and whether the pane toolbars and the queue head use their compact form.
//! Pure functions of the window width the app last saw ([`crate::app::App::window_width`]).

use crate::backend::Side;
use crate::ui::sidebar;

/// Fixed file table columns; the name takes the rest.
pub const SIZE_WIDTH: f32 = 88.0;
pub const MODIFIED_WIDTH: f32 = 116.0;
pub const PERMISSIONS_WIDTH: f32 = 110.0;
/// The name column never gets less than this while another column can still be dropped
/// (Permissions first, then Modified).
pub const NAME_MIN: f32 = 120.0;

/// Below these pane widths the toolbar drops its key hints and tightens its padding
/// (the full remote toolbar needs ~500 px, the local one ~385 px).
const TOOLBAR_FULL_REMOTE: f32 = 500.0;
const TOOLBAR_FULL_LOCAL: f32 = 390.0;
/// Below this center width the queue head uses short labels.
const QUEUE_HEAD_FULL: f32 = 900.0;

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

/// Whether a pane `pane` px wide shows the toolbar without key hints.
pub fn compact_toolbar(pane: f32, side: Side) -> bool {
    pane < match side {
        Side::Local => TOOLBAR_FULL_LOCAL,
        Side::Remote => TOOLBAR_FULL_REMOTE,
    }
}

/// Whether the queue head uses its short labels at this window width.
pub fn compact_queue(window: f32) -> bool {
    center_width(window) < QUEUE_HEAD_FULL
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Columns = Columns {
        modified: true,
        permissions: true,
    };

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

    #[test]
    fn toolbars_and_queue_head_go_compact_when_narrow() {
        assert!(!compact_toolbar(pane_width(1280.0), Side::Remote));
        assert!(!compact_toolbar(pane_width(1280.0), Side::Local));
        assert!(compact_toolbar(pane_width(960.0), Side::Remote));
        assert!(compact_toolbar(pane_width(960.0), Side::Local));
        assert!(!compact_queue(1280.0));
        assert!(compact_queue(960.0));
        assert_eq!(center_width(100.0), 0.0);
    }
}
