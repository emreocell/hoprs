//! Context menus: the model of the one menu that may be open ([`App::menu`]).
//!
//! - **Where:** a menu opens at [`App::last_press`], the window position of the press that
//!   opened it (published by the root [`crate::ui::pointer`] widget before any button sees
//!   the press); [`crate::ui::menu::origin`] keeps it inside the window.
//! - **What:** [`App::menu_items`] lists the items for the target as the app is now; an
//!   item maps to the message the same action sends elsewhere (a button, a key, the
//!   palette), so a menu never does anything the app cannot do without it.
//! - **Choosing:** [`App::menu_command`] turns a choice into the item's own message at the
//!   top of `update`, as the palette does with a run row, so it is handled exactly as if
//!   its button or key had sent it (the reconnection of a lost session included).
//! - **Closing:** Escape, a press outside, a scroll, a window resize, choosing an item; a
//!   security dialog or the palette closes it too. Like other window input it is blocked
//!   behind a dialog or the palette, and ignored while the window closes.

use iced::Point;

use super::{App, Effect, Message};

/// The open menu.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextMenu {
    /// Where it was asked for, in window logical pixels.
    pub at: Point,
    pub target: MenuTarget,
    /// The item the keyboard highlights, if any.
    pub cursor: Option<usize>,
}

/// What a menu is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuTarget {
    /// The sidebar's ⋯ button: which sections show.
    SidebarOptions,
    /// Tests only: one item that goes to `to` in the tab's remote pane (no real menu
    /// sends a tab action before Plan 13 Part 3).
    #[cfg(test)]
    TestNavigate {
        tab: super::TabId,
        to: hoprs_core::RemotePath,
    },
}

/// What an item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Show or hide the workspaces section.
    ShowWorkspaces,
    /// Show or hide the sites section.
    ShowSites,
    /// Tests only: the item of [`MenuTarget::TestNavigate`].
    #[cfg(test)]
    TestNavigate,
}

/// One item, as the menu shows it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub action: MenuAction,
    /// Shown dimmed and not clickable when `false`.
    pub enabled: bool,
    /// A check item's state; `None` for a plain item.
    pub checked: Option<bool>,
    /// Drawn in the error colour (and last, after a rule).
    pub destructive: bool,
    /// A rule separates it from the item above.
    pub rule_before: bool,
}

impl MenuItem {
    fn new(action: MenuAction) -> Self {
        Self {
            action,
            enabled: true,
            checked: None,
            destructive: false,
            rule_before: false,
        }
    }
}

/// Something in the menu.
#[derive(Debug, Clone)]
pub enum MenuMsg {
    /// Opens the menu of `target` at the last press (replacing an open one).
    Open(MenuTarget),
    /// Chooses item `n` of the open menu: it closes, then the item runs if it is enabled.
    Choose(usize),
    /// A press outside, a scroll: closes it.
    Close,
}

impl App {
    /// The items of `target`'s menu, as the app is now.
    pub fn menu_items(&self, target: &MenuTarget) -> Vec<MenuItem> {
        match target {
            // Task 5 of Plan 13 wires these to the sidebar's hidden sections; until then
            // both sections always show and the items cannot change that.
            MenuTarget::SidebarOptions => [MenuAction::ShowWorkspaces, MenuAction::ShowSites]
                .into_iter()
                .map(|action| MenuItem {
                    enabled: false,
                    checked: Some(true),
                    ..MenuItem::new(action)
                })
                .collect(),
            #[cfg(test)]
            MenuTarget::TestNavigate { .. } => vec![MenuItem::new(MenuAction::TestNavigate)],
        }
    }

    /// The message `action` sends for `target`, if it does anything now.
    fn menu_message(&self, target: &MenuTarget, action: MenuAction) -> Option<Message> {
        match (target, action) {
            // Task 5 of Plan 13: `Message::Sidebar(SidebarMsg::SetHidden { .. })`.
            (MenuTarget::SidebarOptions, MenuAction::ShowWorkspaces | MenuAction::ShowSites) => {
                None
            }
            #[cfg(test)]
            (MenuTarget::TestNavigate { tab, to }, MenuAction::TestNavigate) => {
                Some(Message::Pane {
                    tab: *tab,
                    side: crate::backend::Side::Remote,
                    msg: super::PaneMsg::Navigate(to.clone()),
                })
            }
            #[cfg(test)]
            _ => None,
        }
    }

    /// A choice (`MenuMsg::Choose`) closes the menu and becomes the item's own message,
    /// which `update` then handles as if its button or key had sent it; a missing or
    /// disabled item, or no open menu, leaves the choice as it is (it does nothing).
    /// Anything else passes unchanged.
    pub(super) fn menu_command(&mut self, msg: Message) -> Message {
        let Message::Menu(MenuMsg::Choose(index)) = msg else {
            return msg;
        };
        let Some(menu) = self.menu.take() else {
            return msg;
        };
        self.menu_items(&menu.target)
            .get(index)
            .filter(|item| item.enabled)
            .and_then(|item| self.menu_message(&menu.target, item.action))
            .unwrap_or(msg)
    }

    pub(super) fn menu_msg(&mut self, msg: MenuMsg) -> Vec<Effect> {
        match msg {
            MenuMsg::Open(target) => {
                self.menu = Some(ContextMenu {
                    at: self.last_press,
                    target,
                    cursor: None,
                });
                Vec::new()
            }
            // `update` resolved it already ([`App::menu_command`]): what reaches here
            // chose nothing, and the menu is closed.
            MenuMsg::Choose(_) | MenuMsg::Close => {
                self.menu = None;
                Vec::new()
            }
        }
    }
}
