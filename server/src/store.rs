//! SQLite persistence for SMS, including reassembly of concatenated messages.

use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use chrono::{DateTime, Duration, FixedOffset, Local};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sms {
    pub id: i64,
    /// `in` | `out`
    pub direction: String,
    pub peer: String,
    pub text: String,
    pub ts: String,
    /// in: `unread` | `read`; out: `sending` | `sent` | `failed`
    pub status: String,
    pub parts: i64,
    pub error: Option<String>,
    /// Modem memory the message was read from: `SM` (SIM card) or `ME` (module). None for sent messages.
    pub storage: Option<String>,
    /// At least one part is still stored on the modem.
    pub on_modem: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub peer: String,
    pub last_text: String,
    pub last_ts: String,
    pub last_direction: String,
    pub unread: i64,
    pub total: i64,
}

pub struct Store {
    conn: Mutex<Connection>,
}

const SMS_COLS: &str = "id, direction, peer, text, ts, status, parts, error, storage,
    EXISTS(SELECT 1 FROM sms_part p WHERE p.sms_id = sms.id AND p.present = 1)";

fn row_sms(r: &Row) -> rusqlite::Result<Sms> {
    Ok(Sms {
        id: r.get(0)?,
        direction: r.get(1)?,
        peer: r.get(2)?,
        text: r.get(3)?,
        ts: r.get(4)?,
        status: r.get(5)?,
        parts: r.get(6)?,
        error: r.get(7)?,
        storage: r.get(8)?,
        on_modem: r.get(9)?,
    })
}

fn now() -> String {
    Local::now().fixed_offset().to_rfc3339()
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS sms (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                direction TEXT NOT NULL,
                peer TEXT NOT NULL,
                text TEXT NOT NULL,
                ts TEXT NOT NULL,
                status TEXT NOT NULL,
                parts INTEGER NOT NULL DEFAULT 1,
                error TEXT,
                created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS sms_peer ON sms(peer, id);
             CREATE TABLE IF NOT EXISTS sms_part (
                pdu TEXT PRIMARY KEY,
                peer TEXT NOT NULL,
                ref INTEGER NOT NULL,
                total INTEGER NOT NULL,
                seq INTEGER NOT NULL,
                text TEXT NOT NULL,
                ts TEXT NOT NULL,
                received_at TEXT NOT NULL,
                sms_id INTEGER
             );
             CREATE INDEX IF NOT EXISTS sms_part_group ON sms_part(peer, ref, total) WHERE sms_id IS NULL;
             CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        for (table, column, ty) in [
            ("sms", "storage", "TEXT"),
            ("sms_part", "storage", "TEXT"),
            ("sms_part", "present", "INTEGER NOT NULL DEFAULT 0"),
            ("sms_part", "idx", "INTEGER"),
        ] {
            add_column(&conn, table, column, ty)?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Store one received PDU. Returns the complete message once all parts are present.
    /// Already-seen PDUs are ignored.
    #[allow(clippy::too_many_arguments)]
    pub fn ingest(
        &self,
        pdu: &str,
        storage: &str,
        peer: &str,
        reference: u16,
        total: u8,
        seq: u8,
        text: &str,
        ts: DateTime<FixedOffset>,
    ) -> Result<Option<Sms>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO sms_part (pdu, storage, peer, ref, total, seq, text, ts, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                pdu,
                storage,
                peer,
                reference,
                total,
                seq,
                text,
                ts.to_rfc3339(),
                now()
            ],
        )?;
        if inserted == 0 {
            return Ok(None);
        }
        let have: u8 = tx.query_row(
            "SELECT COUNT(DISTINCT seq) FROM sms_part WHERE peer=?1 AND ref=?2 AND total=?3 AND sms_id IS NULL",
            params![peer, reference, total],
            |r| r.get(0),
        )?;
        let sms = if have >= total {
            Some(assemble(&tx, peer, reference, total, false)?)
        } else {
            None
        };
        tx.commit()?;
        Ok(sms)
    }

    /// Assemble partial long messages whose missing parts never arrived.
    pub fn flush_stale(&self, older_than: Duration) -> Result<Vec<Sms>> {
        let cutoff = (Local::now() - older_than).fixed_offset().to_rfc3339();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let groups: Vec<(String, u16, u8)> = {
            let mut st = tx.prepare(
                "SELECT peer, ref, total FROM sms_part WHERE sms_id IS NULL
                 GROUP BY peer, ref, total HAVING MAX(received_at) < ?1",
            )?;
            st.query_map([cutoff], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut out = vec![];
        for (peer, reference, total) in groups {
            out.push(assemble(&tx, &peer, reference, total, true)?);
        }
        tx.commit()?;
        Ok(out)
    }

    /// Record which PDUs are currently stored in modem memory `storage`, and at which index.
    pub fn mark_present(&self, storage: &str, present: &[(String, u32)]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE sms_part SET present=0, idx=NULL WHERE storage=?1",
            [storage],
        )?;
        for (pdu, idx) in present {
            tx.execute(
                "UPDATE sms_part SET present=1, idx=?3 WHERE pdu=?1 AND storage=?2",
                params![pdu, storage, idx],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Modem locations `(storage, index)` of the parts of the given messages.
    pub fn modem_locations(&self, id: Option<i64>, peer: Option<&str>) -> Result<Vec<(String, u32)>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT p.storage, p.idx FROM sms_part p JOIN sms s ON s.id = p.sms_id
             WHERE p.present=1 AND p.idx IS NOT NULL AND (?1 IS NULL OR s.id=?1) AND (?2 IS NULL OR s.peer=?2)",
        )?;
        let rows = st
            .query_map(params![id, peer], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=?2",
            [key, value],
        )?;
        Ok(())
    }

    pub fn insert_outgoing(&self, peer: &str, text: &str, parts: usize) -> Result<Sms> {
        let conn = self.conn();
        let ts = now();
        conn.execute(
            "INSERT INTO sms (direction, peer, text, ts, status, parts, created_at)
             VALUES ('out', ?1, ?2, ?3, 'sending', ?4, ?3)",
            params![peer, text, ts, parts as i64],
        )?;
        let id = conn.last_insert_rowid();
        get(&conn, id)?.ok_or_else(|| anyhow::anyhow!("insert failed"))
    }

    pub fn set_status(&self, id: i64, status: &str, error: Option<&str>) -> Result<Option<Sms>> {
        let conn = self.conn();
        conn.execute(
            "UPDATE sms SET status=?2, error=?3 WHERE id=?1",
            params![id, status, error],
        )?;
        get(&conn, id)
    }

    pub fn list(&self, peer: Option<&str>, before: Option<i64>, limit: i64) -> Result<Vec<Sms>> {
        let conn = self.conn();
        let mut st = conn.prepare(&format!(
            "SELECT {SMS_COLS} FROM sms
             WHERE (?1 IS NULL OR peer = ?1) AND (?2 IS NULL OR id < ?2)
             ORDER BY id DESC LIMIT ?3"
        ))?;
        let rows = st
            .query_map(params![peer, before, limit], row_sms)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn conversations(&self) -> Result<Vec<Conversation>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT s.peer, s.text, s.ts, s.direction, g.unread, g.total
             FROM (SELECT peer, MAX(id) AS last_id,
                          SUM(status = 'unread') AS unread, COUNT(*) AS total
                   FROM sms GROUP BY peer) g
             JOIN sms s ON s.id = g.last_id
             ORDER BY s.id DESC",
        )?;
        let rows = st
            .query_map([], |r| {
                Ok(Conversation {
                    peer: r.get(0)?,
                    last_text: r.get(1)?,
                    last_ts: r.get(2)?,
                    last_direction: r.get(3)?,
                    unread: r.get(4)?,
                    total: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn mark_read(&self, peer: &str) -> Result<usize> {
        Ok(self.conn().execute(
            "UPDATE sms SET status='read' WHERE peer=?1 AND status='unread'",
            [peer],
        )?)
    }

    pub fn delete(&self, id: i64) -> Result<bool> {
        let conn = self.conn();
        conn.execute("DELETE FROM sms_part WHERE sms_id=?1", [id])?;
        Ok(conn.execute("DELETE FROM sms WHERE id=?1", [id])? > 0)
    }

    pub fn delete_peer(&self, peer: &str) -> Result<usize> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM sms_part WHERE sms_id IN (SELECT id FROM sms WHERE peer=?1)",
            [peer],
        )?;
        Ok(conn.execute("DELETE FROM sms WHERE peer=?1", [peer])?)
    }

    pub fn unread_count(&self) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM sms WHERE status='unread'", [], |r| r.get(0))?)
    }
}

fn add_column(conn: &Connection, table: &str, column: &str, ty: &str) -> Result<()> {
    let exists: bool = conn.query_row(
        &format!("SELECT COUNT(*) > 0 FROM pragma_table_info('{table}') WHERE name=?1"),
        [column],
        |r| r.get(0),
    )?;
    if !exists {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {ty}"))?;
    }
    Ok(())
}

fn get(conn: &Connection, id: i64) -> Result<Option<Sms>> {
    Ok(conn
        .query_row(&format!("SELECT {SMS_COLS} FROM sms WHERE id=?1"), [id], row_sms)
        .optional()?)
}

fn assemble(tx: &rusqlite::Transaction, peer: &str, reference: u16, total: u8, partial: bool) -> Result<Sms> {
    let parts: Vec<(u8, String, String, Option<String>)> = {
        let mut st = tx.prepare(
            "SELECT seq, text, ts, storage FROM sms_part
             WHERE peer=?1 AND ref=?2 AND total=?3 AND sms_id IS NULL ORDER BY seq",
        )?;
        st.query_map(params![peer, reference, total], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?
    };
    let mut text = String::new();
    let mut expected = 1u8;
    let mut last_seq = 0u8;
    for (seq, t, _, _) in &parts {
        if *seq == last_seq {
            continue; // duplicate part
        }
        if partial && *seq != expected {
            text.push_str("[…]");
        }
        text.push_str(t);
        expected = seq + 1;
        last_seq = *seq;
    }
    if partial && expected <= total {
        text.push_str("[…]");
    }
    let ts = parts.first().map(|p| p.2.clone()).unwrap_or_else(now);
    let storage = parts.first().and_then(|p| p.3.clone());
    tx.execute(
        "INSERT INTO sms (direction, peer, text, ts, status, parts, created_at, storage)
         VALUES ('in', ?1, ?2, ?3, 'unread', ?4, ?5, ?6)",
        params![peer, text, ts, total, now(), storage],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "UPDATE sms_part SET sms_id=?4 WHERE peer=?1 AND ref=?2 AND total=?3 AND sms_id IS NULL",
        params![peer, reference, total, id],
    )?;
    Ok(Sms {
        id,
        direction: "in".into(),
        peer: peer.into(),
        text,
        ts,
        status: "unread".into(),
        parts: total as i64,
        error: None,
        storage,
        on_modem: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concat_reassembly() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let ts = Local::now().fixed_offset();
        assert!(
            store
                .ingest("p2", "SM", "10086", 5, 2, 2, "world", ts)
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .ingest("p2", "SM", "10086", 5, 2, 2, "world", ts)
                .unwrap()
                .is_none()
        );
        let sms = store
            .ingest("p1", "SM", "10086", 5, 2, 1, "hello ", ts)
            .unwrap()
            .unwrap();
        assert_eq!(sms.text, "hello world");
        let single = store
            .ingest("p3", "ME", "10086", 0, 1, 1, "hi", ts)
            .unwrap()
            .unwrap();
        assert_eq!(single.text, "hi");
        assert_eq!(single.storage.as_deref(), Some("ME"));
        store.mark_present("ME", &[("p3".into(), 4)]).unwrap();
        assert_eq!(
            store.modem_locations(Some(single.id), None).unwrap(),
            vec![("ME".to_string(), 4)]
        );
        let listed = store.list(Some("10086"), None, 10).unwrap();
        assert!(listed.iter().any(|m| m.id == single.id && m.on_modem));
        let convs = store.conversations().unwrap();
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].unread, 2);
    }
}
