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
//! The same holding applies while the transport is reconnecting by itself (`Reconnecting`
//! on a site that connected before: its own round after a drop, or the session manager's
//! quiet extra round), without a reconnect of the app's own: the actions run when it is
//! back. If that round gives up instead, the held actions get the app's reconnection.
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

    /// An action on the site waits for a reconnection (running, by the app or the
    /// transport, or to be started).
    pub(crate) fn awaits_reconnection(&self) -> bool {
        matches!(&self.state, TabState::Connected(_))
            && (self.reconnecting
                || self.lost_to_network()
                || matches!(self.conn, Some(ConnectionState::Reconnecting { .. })))
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
            if let Effect::Enqueue(specs) = &effect {
                // Queued at once (the engine keeps an item until its site is back); each
                // lost site it names reconnects.
                let mut sites: Vec<SiteId> = Vec::new();
                for spec in specs {
                    if !sites.contains(&spec.site) {
                        sites.push(spec.site.clone());
                    }
                }
                for site in sites {
                    if self.conn(&site).awaits_reconnection() {
                        out.extend(self.reconnect_on_demand(&site));
                    }
                }
                out.push(effect);
                continue;
            }
            let site = match &effect {
                Effect::List {
                    session: Some(session),
                    ..
                }
                | Effect::FileOp {
                    session: Some(session),
                    ..
                } => self.site_of_session(*session),
                _ => None,
            };
            let Some(site) = site.filter(|site| self.conn(site).awaits_reconnection()) else {
                out.push(effect);
                continue;
            };
            out.extend(self.reconnect_on_demand(&site));
            self.hold(site, effect);
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

    /// The site is connected again: its held actions run, once. A listing or a file
    /// operation its pane no longer waits for (the tab closed, another folder) is dropped.
    pub(super) fn release_held(&mut self, site: &SiteId) -> Vec<Effect> {
        let held = self.held.remove(site).unwrap_or_default();
        let mut effects = Vec::new();
        for effect in held {
            let pane_of = |tab: &super::TabId, side: &crate::backend::Side| {
                self.tab_index(*tab)
                    .and_then(|index| self.tabs[index].pane(*side))
            };
            match &effect {
                Effect::List {
                    tab,
                    side,
                    request_id,
                    ..
                } => {
                    if !pane_of(tab, side).is_some_and(|pane| pane.request == Some(*request_id)) {
                        continue;
                    }
                    self.replayed.insert(*request_id);
                }
                Effect::FileOp { tab, side, op, .. } => {
                    let waiting = pane_of(tab, side)
                        .and_then(|pane| pane.busy.as_ref())
                        .is_some_and(|busy| busy.op == *op);
                    if !waiting {
                        continue;
                    }
                }
                _ => {}
            }
            effects.push(effect);
        }
        self.forget_stale_replays();
        effects
    }

    /// Keeps only the replayed listings a pane still waits for (the others are done or
    /// superseded), so the set never grows.
    pub(super) fn forget_stale_replays(&mut self) {
        let waiting: Vec<super::files::RequestId> = self
            .tabs
            .iter()
            .flat_map(|tab| {
                [
                    tab.pane(crate::backend::Side::Local),
                    tab.pane(crate::backend::Side::Remote),
                ]
            })
            .flatten()
            .filter_map(|pane| pane.request)
            .collect();
        self.replayed.retain(|request| waiting.contains(request));
    }

    /// The transport gave up while actions waited for its own round and the app has not
    /// reconnected yet: they get the app's reconnection instead (see the module docs).
    pub(super) fn reconnect_for_held(&mut self, site: &SiteId) -> Option<Effect> {
        if self.held.get(site).is_none_or(Vec::is_empty) {
            return None;
        }
        self.reconnect_on_demand(site)
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
