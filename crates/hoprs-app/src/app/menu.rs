//! Context menus: the model of the one menu that may be open ([`App::menu`]).
//!
//! - **Where:** a menu opens at [`App::last_press`], the window position of the press that
//!   opened it (published by the root [`crate::ui::pointer`] widget before any button sees
//!   the press); [`crate::ui::menu::origin`] keeps it inside the window.
//! - **What:** [`App::menu_items`] lists the items for the target as the app is now; an
//!   item maps to the message the same action sends elsewhere (a button, a key, the
//!   palette), so a menu never does anything the app cannot do without it.
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
}

/// What an item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Show or hide the workspaces section.
    ShowWorkspaces,
    /// Show or hide the sites section.
    ShowSites,
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
        }
    }

    /// The message `action` sends for `target`, if it does anything now.
    fn menu_message(&self, target: &MenuTarget, action: MenuAction) -> Option<Message> {
        match (target, action) {
            // Task 5 of Plan 13: `Message::Sidebar(SidebarMsg::SetHidden { .. })`.
            (MenuTarget::SidebarOptions, MenuAction::ShowWorkspaces | MenuAction::ShowSites) => {
                None
            }
        }
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
            MenuMsg::Choose(index) => {
                let Some(menu) = self.menu.take() else {
                    return Vec::new();
                };
                let message = self
                    .menu_items(&menu.target)
                    .get(index)
                    .filter(|item| item.enabled)
                    .and_then(|item| self.menu_message(&menu.target, item.action));
                match message {
                    Some(message) => self.apply(message),
                    None => Vec::new(),
                }
            }
            MenuMsg::Close => {
                self.menu = None;
                Vec::new()
            }
        }
    }
}
