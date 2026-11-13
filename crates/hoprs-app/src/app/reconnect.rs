//! Reconnect on demand (Plan 6, Task 5; Review Focus 3).
//!
//! A connected site whose session ended `Disconnected` for a network reason (its transport
//! spent its attempts, say while nobody used it) comes back by itself on the user's next
//! action on one of its tabs: listing or going to a folder, a file operation, a transfer,
//! opening a terminal or restarting its shell. Instead of failing, the action reconnects
//! the session ([`Effect::Reconnect`], once for the site, the banner showing
//! "Reconnecting…" meanwhile) and waits:
//!
//! - a listing or a file operation is held, then sent once the session is `Connected`
//!   (whichever the app hears first: the session's feed or the reconnect's result), once;
//!   a newer listing of the same pane replaces the held one;
//! - a transfer is queued at once: the engine holds it until the site is back;
//! - a terminal waits without a shell and opens one when the session is back.
//!
//! A reconnection that fails fails the held actions with its reason; they are never
//! replayed later. A session refused for good (host key, authentication, TLS) never
//! reconnects this way: its actions fail as before, and its banner keeps the reason and
//! the fix. Only the user's own messages count as actions (see [`is_user_action`]): what
//! the app does by itself (a listing after a file operation, say) never starts a
//! reconnection.

use hoprs_core::{ConnectionState, Error, ErrorKind, SiteId};

use super::{App, Effect, Message, SiteConn, TabState};

/// Whether `msg` is the user acting on a tab (what may reconnect a lost session).
pub(super) fn is_user_action(msg: &Message) -> bool {
    matches!(
        msg,
        Message::Pane { .. }
            | Message::KeyPressed { .. }
            | Message::Key { .. }
            | Message::NewTab { .. }
            | Message::Terminal { .. }
            | Message::FileDropped(_)
            | Message::DragEnd
    )
}

impl SiteConn {
    /// The session was lost for a network reason: an action reconnects it.
    pub fn lost_to_network(&self) -> bool {
        matches!(&self.state, TabState::Connected(_))
            && matches!(
                &self.conn,
                Some(ConnectionState::Disconnected(error)) if error.kind == ErrorKind::Network
            )
    }

    /// An action on the site waits for a reconnection (running, or to be started).
    pub(crate) fn awaits_reconnection(&self) -> bool {
        matches!(&self.state, TabState::Connected(_))
            && (self.reconnecting || self.lost_to_network())
    }
}

impl App {
    /// Starts the reconnection of the site's lost session, unless one runs: the effect,
    /// or nothing.
    pub(super) fn reconnect_on_demand(&mut self, site: &SiteId) -> Option<Effect> {
        let conn = self.conn_mut(site);
        if conn.reconnecting || !conn.lost_to_network() {
            return None;
        }
        let session = conn.session()?;
        conn.reconnecting = true;
        Some(Effect::Reconnect { session })
    }

    /// The effects of a user action, with those that need a lost session's connection
    /// held until it is back (see the module docs).
    pub(super) fn hold_for_reconnection(&mut self, effects: Vec<Effect>) -> Vec<Effect> {
        if !self.conns.values().any(SiteConn::awaits_reconnection) {
            return effects;
        }
        let mut out = Vec::with_capacity(effects.len());
        for effect in effects {
            let site = match &effect {
                Effect::List {
                    session: Some(session),
                    ..
                }
                | Effect::FileOp {
                    session: Some(session),
                    ..
                } => self.site_of_session(*session),
                Effect::Enqueue(specs) => specs.first().map(|spec| spec.site.clone()),
                _ => None,
            };
            let Some(site) = site.filter(|site| self.conn(site).awaits_reconnection()) else {
                out.push(effect);
                continue;
            };
            out.extend(self.reconnect_on_demand(&site));
            match effect {
                // The engine keeps it queued until the site is back.
                Effect::Enqueue(_) => out.push(effect),
                held => self.hold(site, held),
            }
        }
        out
    }

    fn hold(&mut self, site: SiteId, effect: Effect) {
        let held = self.held.entry(site).or_default();
        // A newer listing of the same pane supersedes the held one.
        if let Effect::List { tab, side, .. } = &effect {
            held.retain(
                |e| !matches!(e, Effect::List { tab: t, side: s, .. } if t == tab && s == side),
            );
        }
        held.push(effect);
    }

    /// The site is connected again: its held actions run, once. A listing the pane no
    /// longer waits for is dropped.
    pub(super) fn release_held(&mut self, site: &SiteId) -> Vec<Effect> {
        let held = self.held.remove(site).unwrap_or_default();
        let mut effects = Vec::new();
        for effect in held {
            if let Effect::List {
                tab,
                side,
                request_id,
                ..
            } = &effect
            {
                let current = self
                    .tab_index(*tab)
                    .and_then(|index| self.tabs[index].pane(*side))
                    .is_some_and(|pane| pane.request == Some(*request_id));
                if !current {
                    continue;
                }
                self.replayed.insert(*request_id);
            }
            effects.push(effect);
        }
        effects
    }

    /// The reconnection failed (or the session was refused): the held actions fail with
    /// `error`.
    pub(super) fn fail_held(&mut self, site: &SiteId, error: &Error) -> Vec<Effect> {
        let held = self.held.remove(site).unwrap_or_default();
        let mut effects = Vec::new();
        for effect in held {
            effects.extend(match effect {
                Effect::List {
                    tab,
                    side,
                    request_id,
                    ..
                } => self.listed(tab, side, request_id, Err(error.clone())),
                Effect::FileOp { tab, side, op, .. } => {
                    self.file_op_done(tab, side, op, Err(error.clone()))
                }
                _ => Vec::new(),
            });
        }
        effects
    }
}
