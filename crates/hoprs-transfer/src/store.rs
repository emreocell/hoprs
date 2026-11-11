use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hoprs_core::{Error, ErrorKind, RemotePath, Result, SiteId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::model::{
    ConflictInfo, Direction, TransferId, TransferItem, TransferSpec, TransferState,
};

const MIGRATION_V1: &str = "
CREATE TABLE transfers (
  id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES transfers(id) ON DELETE CASCADE,
  site TEXT NOT NULL, direction TEXT NOT NULL, local_path TEXT NOT NULL, remote_path TEXT NOT NULL,
  is_dir INTEGER, size INTEGER, bytes_done INTEGER NOT NULL DEFAULT 0,
  state TEXT NOT NULL, state_detail TEXT, error_kind TEXT, error_message TEXT, error_detail TEXT,
  conflict_json TEXT, created_at INTEGER NOT NULL);
CREATE INDEX transfers_parent ON transfers(parent_id);
";

const SELECT: &str = "SELECT id, parent_id, site, direction, local_path, remote_path, is_dir, size, \
    bytes_done, state, state_detail, error_kind, error_message, error_detail, conflict_json \
    FROM transfers";

/// The state a `Paused` item was paused from, so `resume` can treat it correctly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PausedFrom {
    Queued,
    Active,
    AwaitingDecision,
}

impl PausedFrom {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(PausedFrom::Queued),
            "active" => Some(PausedFrom::Active),
            "awaiting_decision" => Some(PausedFrom::AwaitingDecision),
            _ => None,
        }
    }
}

/// SQLite-backed persistent transfer queue. Synchronous: operations are short.
pub struct QueueStore {
    conn: Connection,
}

fn db_err(e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Io, "Queue database error").with_detail(e.to_string())
}

fn corrupt(what: &str) -> Error {
    Error::new(ErrorKind::Io, "Queue database error").with_detail(format!("corrupt row: {what}"))
}

trait Db<T> {
    fn db(self) -> Result<T>;
}

impl<T> Db<T> for rusqlite::Result<T> {
    fn db(self) -> Result<T> {
        self.map_err(db_err)
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn time_to_secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        Err(e) => i64::try_from(e.duration().as_secs()).map_or(i64::MIN, |v| -v),
    }
}

fn secs_to_time(s: i64) -> Option<SystemTime> {
    if s >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(s as u64))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(s.unsigned_abs()))
    }
}

fn opt_secs_json(t: Option<SystemTime>) -> String {
    t.map_or("null".to_owned(), |t| time_to_secs(t).to_string())
}

fn conflict_to_json(c: &ConflictInfo) -> String {
    format!(
        "{{\"src_size\":{},\"dst_size\":{},\"src_modified\":{},\"dst_modified\":{}}}",
        c.src_size,
        c.dst_size,
        opt_secs_json(c.src_modified),
        opt_secs_json(c.dst_modified)
    )
}

/// Extracts the raw value of `key` from the flat JSON object written by `conflict_to_json`.
fn json_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\":");
    let start = json.find(&pat)? + pat.len();
    let rest = &json[start..];
    let end = rest.find([',', '}'])?;
    Some(rest[..end].trim())
}

fn conflict_from_json(json: &str) -> Result<ConflictInfo> {
    let num = |k: &str| -> Result<u64> {
        json_field(json, k)
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| corrupt("conflict json"))
    };
    let time = |k: &str| -> Result<Option<SystemTime>> {
        match json_field(json, k) {
            Some("null") => Ok(None),
            Some(v) => v
                .parse::<i64>()
                .ok()
                .and_then(secs_to_time)
                .map(Some)
                .ok_or_else(|| corrupt("conflict json")),
            None => Err(corrupt("conflict json")),
        }
    };
    Ok(ConflictInfo {
        src_size: num("src_size")?,
        dst_size: num("dst_size")?,
        src_modified: time("src_modified")?,
        dst_modified: time("dst_modified")?,
    })
}

fn item_from_row(row: &Row<'_>) -> Result<TransferItem> {
    let site: String = row.get(2).db()?;
    let direction = match row.get::<_, String>(3).db()?.as_str() {
        "upload" => Direction::Upload,
        "download" => Direction::Download,
        _ => return Err(corrupt("direction")),
    };
    let local =
        RemotePath::new(&row.get::<_, String>(4).db()?).map_err(|_| corrupt("local_path"))?;
    let remote =
        RemotePath::new(&row.get::<_, String>(5).db()?).map_err(|_| corrupt("remote_path"))?;
    let state_name: String = row.get(9).db()?;
    let detail: Option<String> = row.get(10).db()?;
    let state = match state_name.as_str() {
        "queued" => TransferState::Queued,
        "active" => TransferState::Active,
        "paused" => TransferState::Paused,
        "awaiting_decision" => {
            let json: Option<String> = row.get(14).db()?;
            TransferState::AwaitingDecision(conflict_from_json(
                &json.ok_or_else(|| corrupt("missing conflict"))?,
            )?)
        }
        "done" => TransferState::Done,
        "skipped" => TransferState::Skipped(detail.unwrap_or_default()),
        "failed" => {
            let kind: Option<String> = row.get(11).db()?;
            let kind = kind
                .as_deref()
                .and_then(ErrorKind::parse)
                .ok_or_else(|| corrupt("error_kind"))?;
            let message: Option<String> = row.get(12).db()?;
            let mut err = Error::new(kind, message.unwrap_or_default());
            err.detail = row.get(13).db()?;
            TransferState::Failed(err)
        }
        "cancelled" => TransferState::Cancelled,
        _ => return Err(corrupt("state")),
    };
    Ok(TransferItem {
        id: TransferId(row.get(0).db()?),
        parent: row.get::<_, Option<i64>>(1).db()?.map(TransferId),
        spec: TransferSpec {
            site: SiteId(site),
            direction,
            local,
            remote,
        },
        is_dir: row.get::<_, Option<bool>>(6).db()?,
        size: row.get::<_, Option<i64>>(7).db()?.map(|s| s as u64),
        bytes_done: row.get::<_, i64>(8).db()? as u64,
        state,
    })
}

impl QueueStore {
    /// Opens (creating if needed) the queue database at `path`, in WAL mode.
    pub fn open(path: &Path) -> Result<QueueStore> {
        let conn = Connection::open(path).db()?;
        conn.pragma_update(None, "journal_mode", "WAL").db()?;
        Self::init(conn)
    }

    /// Opens a private in-memory queue database.
    pub fn open_in_memory() -> Result<QueueStore> {
        Self::init(Connection::open_in_memory().db()?)
    }

    fn init(conn: Connection) -> Result<QueueStore> {
        conn.pragma_update(None, "foreign_keys", "ON").db()?;
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .db()?;
        if version < 1 {
            let tx = conn.unchecked_transaction().db()?;
            tx.execute_batch(MIGRATION_V1).db()?;
            tx.pragma_update(None, "user_version", 1).db()?;
            tx.commit().db()?;
        }
        Ok(QueueStore { conn })
    }

    /// Inserts a new `Queued` item and returns its id.
    pub fn insert(
        &mut self,
        parent: Option<TransferId>,
        spec: &TransferSpec,
    ) -> Result<TransferId> {
        let direction = match spec.direction {
            Direction::Upload => "upload",
            Direction::Download => "download",
        };
        self.conn.execute(
            "INSERT INTO transfers (parent_id, site, direction, local_path, remote_path, state, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6)",
            params![
                parent.map(|p| p.0),
                spec.site.0,
                direction,
                spec.local.as_str(),
                spec.remote.as_str(),
                now_secs()
            ],
        ).db()?;
        Ok(TransferId(self.conn.last_insert_rowid()))
    }

    /// Persists a new state. Moving to `Paused` records the state it was paused from.
    pub fn set_state(&mut self, id: TransferId, state: &TransferState) -> Result<()> {
        let mut detail: Option<String> = None;
        let (mut kind, mut message, mut err_detail, mut conflict) = (None, None, None, None);
        let name = match state {
            TransferState::Queued => "queued",
            TransferState::Active => "active",
            TransferState::Paused => {
                let current: Option<(String, Option<String>)> = self
                    .conn
                    .query_row(
                        "SELECT state, state_detail FROM transfers WHERE id = ?1",
                        [id.0],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .db()?;
                detail = match current {
                    Some((s, _)) if PausedFrom::parse(&s).is_some() => Some(s),
                    Some((s, d)) if s == "paused" => d,
                    _ => None,
                };
                "paused"
            }
            TransferState::AwaitingDecision(c) => {
                conflict = Some(conflict_to_json(c));
                "awaiting_decision"
            }
            TransferState::Done => "done",
            TransferState::Skipped(reason) => {
                detail = Some(reason.clone());
                "skipped"
            }
            TransferState::Failed(e) => {
                kind = Some(e.kind.as_str());
                message = Some(e.message.clone());
                err_detail = e.detail.clone();
                "failed"
            }
            TransferState::Cancelled => "cancelled",
        };
        self.conn.execute(
            "UPDATE transfers SET state = ?2, state_detail = ?3, error_kind = ?4, error_message = ?5, \
             error_detail = ?6, conflict_json = ?7 WHERE id = ?1",
            params![id.0, name, detail, kind, message, err_detail, conflict],
        ).db()?;
        Ok(())
    }

    /// Returns what a `Paused` item was paused from, or `None` if it is not paused.
    pub fn paused_from(&self, id: TransferId) -> Result<Option<PausedFrom>> {
        let row: Option<(String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT state, state_detail FROM transfers WHERE id = ?1",
                [id.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .db()?;
        Ok(match row {
            Some((s, Some(d))) if s == "paused" => PausedFrom::parse(&d),
            _ => None,
        })
    }

    pub fn set_meta(&mut self, id: TransferId, is_dir: bool, size: Option<u64>) -> Result<()> {
        self.conn
            .execute(
                "UPDATE transfers SET is_dir = ?2, size = ?3 WHERE id = ?1",
                params![id.0, is_dir, size.map(|s| s as i64)],
            )
            .db()?;
        Ok(())
    }

    pub fn set_bytes_done(&mut self, id: TransferId, bytes: u64) -> Result<()> {
        self.conn
            .execute(
                "UPDATE transfers SET bytes_done = ?2 WHERE id = ?1",
                params![id.0, bytes as i64],
            )
            .db()?;
        Ok(())
    }

    /// Persists several byte counts in one transaction.
    pub fn set_bytes_done_many(&mut self, updates: &[(TransferId, u64)]) -> Result<()> {
        let tx = self.conn.transaction().db()?;
        {
            let mut stmt = tx
                .prepare_cached("UPDATE transfers SET bytes_done = ?2 WHERE id = ?1")
                .db()?;
            for (id, bytes) in updates {
                stmt.execute(params![id.0, *bytes as i64]).db()?;
            }
        }
        tx.commit().db()
    }

    pub fn get(&self, id: TransferId) -> Result<Option<TransferItem>> {
        self.query(&format!("{SELECT} WHERE id = ?1"), params![id.0])
            .map(|mut v| v.pop())
    }

    /// All items ordered by id.
    pub fn list_all(&self) -> Result<Vec<TransferItem>> {
        self.query(&format!("{SELECT} ORDER BY id"), [])
    }

    /// Direct children of `parent`, ordered by id.
    pub fn children(&self, parent: TransferId) -> Result<Vec<TransferItem>> {
        self.query(
            &format!("{SELECT} WHERE parent_id = ?1 ORDER BY id"),
            params![parent.0],
        )
    }

    fn query(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<TransferItem>> {
        let mut stmt = self.conn.prepare(sql).db()?;
        let mut rows = stmt.query(params).db()?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().db()? {
            out.push(item_from_row(row)?);
        }
        Ok(out)
    }

    /// Pauses `Active` and `AwaitingDecision` items (remembering the origin) and returns
    /// every non-terminal item.
    pub fn recover(&mut self) -> Result<Vec<TransferItem>> {
        self.conn
            .execute(
                "UPDATE transfers SET state = 'paused', state_detail = state, conflict_json = NULL \
             WHERE state IN ('active', 'awaiting_decision')",
                [],
            )
            .db()?;
        let all = self.query(&format!("{SELECT} ORDER BY id"), [])?;
        Ok(all.into_iter().filter(|i| !i.state.is_terminal()).collect())
    }

    /// Removes terminal root items and all their descendants (via `ON DELETE CASCADE`).
    /// Returns the number of root items removed (descendants are not counted).
    pub fn clear_finished(&mut self) -> Result<usize> {
        self.conn
            .execute(
                "DELETE FROM transfers WHERE parent_id IS NULL \
             AND state IN ('done', 'skipped', 'failed', 'cancelled')",
                [],
            )
            .db()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(local: &str, remote: &str) -> TransferSpec {
        TransferSpec {
            site: SiteId("site".to_string()),
            direction: Direction::Upload,
            local: RemotePath::new(local).unwrap(),
            remote: RemotePath::new(remote).unwrap(),
        }
    }

    fn new_item(s: &mut QueueStore) -> TransferId {
        s.insert(None, &spec("/a", "/b")).unwrap()
    }

    #[test]
    fn every_state_round_trips() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let conflict = ConflictInfo {
            src_size: 10,
            dst_size: 20,
            src_modified: Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            dst_modified: None,
        };
        let states = vec![
            TransferState::Queued,
            TransferState::Active,
            TransferState::Paused,
            TransferState::AwaitingDecision(conflict),
            TransferState::Done,
            TransferState::Skipped("exists".to_string()),
            TransferState::Failed(
                Error::new(ErrorKind::Permission, "denied").with_detail("EACCES /x"),
            ),
            TransferState::Cancelled,
        ];
        for st in states {
            let id = new_item(&mut s);
            s.set_state(id, &st).unwrap();
            let item = s.get(id).unwrap().unwrap();
            assert_eq!(item.state, st);
            assert_eq!(item.spec, spec("/a", "/b"));
        }
        assert!(s.get(TransferId(999)).unwrap().is_none());
    }

    #[test]
    fn leaving_a_state_clears_its_columns() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let conflict = ConflictInfo {
            src_size: 1,
            dst_size: 2,
            src_modified: None,
            dst_modified: None,
        };
        let leaving = [
            TransferState::Failed(Error::new(ErrorKind::Io, "x").with_detail("d")),
            TransferState::AwaitingDecision(conflict),
            TransferState::Skipped("why".to_string()),
        ];
        for st in leaving {
            let id = new_item(&mut s);
            s.set_state(id, &st).unwrap();
            s.set_state(id, &TransferState::Queued).unwrap();
            let nulls: i64 = s
                .conn
                .query_row(
                    "SELECT error_kind IS NULL AND error_message IS NULL AND error_detail IS NULL \
                     AND conflict_json IS NULL AND state_detail IS NULL FROM transfers WHERE id = ?1",
                    [id.0],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(nulls, 1, "columns not cleared after leaving {st:?}");
        }
    }

    #[test]
    fn out_of_range_stored_timestamps_are_errors_not_panics() {
        for bad in [
            "-9223372036854775808",
            "9223372036854775807",
            "99999999999999999",
        ] {
            let mut s = QueueStore::open_in_memory().unwrap();
            let id = new_item(&mut s);
            s.set_state(
                id,
                &TransferState::AwaitingDecision(ConflictInfo {
                    src_size: 1,
                    dst_size: 2,
                    src_modified: None,
                    dst_modified: None,
                }),
            )
            .unwrap();
            let json = format!(
                "{{\"src_size\":1,\"dst_size\":2,\"src_modified\":{bad},\"dst_modified\":null}}"
            );
            s.conn
                .execute("UPDATE transfers SET conflict_json = ?1", [json])
                .unwrap();
            // Representable on some platforms (Linux i64 seconds), not on others (Windows):
            // either way it must be a value or an Io error, never a panic. Unparseable
            // numbers are an error everywhere.
            let got = s.get(id);
            let listed = s.list_all();
            let recovered = s.recover();
            if bad.parse::<i64>().is_err() {
                assert_eq!(got.unwrap_err().kind, ErrorKind::Io);
                assert_eq!(listed.unwrap_err().kind, ErrorKind::Io);
                assert_eq!(recovered.unwrap_err().kind, ErrorKind::Io);
            } else if let Err(e) = got {
                assert_eq!(e.kind, ErrorKind::Io);
            }
        }
    }

    #[test]
    fn meta_and_bytes_are_persisted() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let id = new_item(&mut s);
        s.set_meta(id, false, Some(42)).unwrap();
        s.set_bytes_done(id, 7).unwrap();
        let item = s.get(id).unwrap().unwrap();
        assert_eq!(
            (item.is_dir, item.size, item.bytes_done),
            (Some(false), Some(42), 7)
        );
    }

    #[test]
    fn set_bytes_done_many_updates_each_listed_item() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let (a, b, c) = (new_item(&mut s), new_item(&mut s), new_item(&mut s));
        s.set_bytes_done(c, 5).unwrap();
        s.set_bytes_done_many(&[(a, 100), (b, 200)]).unwrap();
        let bytes: Vec<u64> = s.list_all().unwrap().iter().map(|i| i.bytes_done).collect();
        assert_eq!(bytes, vec![100, 200, 5]);
        s.set_bytes_done_many(&[]).unwrap();
    }

    #[test]
    fn children_returns_only_children_of_the_parent() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let p1 = new_item(&mut s);
        let p2 = new_item(&mut s);
        let c1 = s.insert(Some(p1), &spec("/a/x", "/b/x")).unwrap();
        s.insert(Some(p2), &spec("/a/y", "/b/y")).unwrap();
        let kids = s.children(p1).unwrap();
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].id, c1);
        assert_eq!(kids[0].parent, Some(p1));
        assert_eq!(s.list_all().unwrap().len(), 4);
    }

    #[test]
    fn recover_pauses_active_and_awaiting_and_skips_terminal() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let active = new_item(&mut s);
        let awaiting = new_item(&mut s);
        let done = new_item(&mut s);
        let queued = new_item(&mut s);
        s.set_state(active, &TransferState::Active).unwrap();
        s.set_state(
            awaiting,
            &TransferState::AwaitingDecision(ConflictInfo {
                src_size: 1,
                dst_size: 2,
                src_modified: None,
                dst_modified: None,
            }),
        )
        .unwrap();
        s.set_state(done, &TransferState::Done).unwrap();

        let recovered = s.recover().unwrap();
        let ids: Vec<_> = recovered.iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![active, awaiting, queued]);
        assert_eq!(recovered[0].state, TransferState::Paused);
        assert_eq!(recovered[1].state, TransferState::Paused);
        assert_eq!(recovered[2].state, TransferState::Queued);
        assert_eq!(s.get(done).unwrap().unwrap().state, TransferState::Done);
        assert_eq!(s.paused_from(active).unwrap(), Some(PausedFrom::Active));
        assert_eq!(
            s.paused_from(awaiting).unwrap(),
            Some(PausedFrom::AwaitingDecision)
        );
    }

    #[test]
    fn paused_from_tracks_the_origin_state() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let id = new_item(&mut s);
        assert_eq!(s.paused_from(id).unwrap(), None);
        s.set_state(id, &TransferState::Active).unwrap();
        s.set_state(id, &TransferState::Paused).unwrap();
        assert_eq!(s.paused_from(id).unwrap(), Some(PausedFrom::Active));
        // Pausing again keeps the original origin.
        s.set_state(id, &TransferState::Paused).unwrap();
        assert_eq!(s.paused_from(id).unwrap(), Some(PausedFrom::Active));
        s.set_state(id, &TransferState::Queued).unwrap();
        assert_eq!(s.paused_from(id).unwrap(), None);
        s.set_state(id, &TransferState::Paused).unwrap();
        assert_eq!(s.paused_from(id).unwrap(), Some(PausedFrom::Queued));
    }

    #[test]
    fn items_persist_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.db");
        let id = {
            let mut s = QueueStore::open(&path).unwrap();
            new_item(&mut s)
        };
        let s = QueueStore::open(&path).unwrap();
        let items = s.list_all().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, id);
    }

    #[test]
    fn clear_finished_removes_terminal_roots_with_descendants() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let done_root = new_item(&mut s);
        let child = s.insert(Some(done_root), &spec("/a/x", "/b/x")).unwrap();
        let grandchild = s.insert(Some(child), &spec("/a/x/y", "/b/x/y")).unwrap();
        let queued_root = new_item(&mut s);
        s.set_state(done_root, &TransferState::Done).unwrap();

        assert_eq!(s.clear_finished().unwrap(), 1);
        assert!(s.get(done_root).unwrap().is_none());
        assert!(s.get(child).unwrap().is_none());
        assert!(s.get(grandchild).unwrap().is_none());
        assert!(s.get(queued_root).unwrap().is_some());
    }

    #[test]
    fn corrupt_state_is_an_error_not_a_panic() {
        let mut s = QueueStore::open_in_memory().unwrap();
        let id = new_item(&mut s);
        s.conn
            .execute("UPDATE transfers SET state = 'bogus'", [])
            .unwrap();
        let err = s.get(id).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Io);
    }
}
