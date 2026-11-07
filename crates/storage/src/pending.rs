//! Änderungs-Warteschlange für Offline-Bearbeitung.
//!
//! Eine Bearbeitung landet sofort im lokalen Cache (die Oberfläche zeigt sie
//! direkt) und zusätzlich hier, bis der Sync-Thread sie zum Server geschickt
//! hat. Je Ressource gibt es höchstens einen Eintrag: spätere Änderungen
//! fassen frühere zusammen, das erwartete ETag bleibt das des Server-Stands,
//! auf dem die erste Änderung beruhte. Solange ein Eintrag existiert, lässt
//! der Sync die Ressource in Ruhe.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};

use crate::{Resource, Result, Store, index_of};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Put,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpState {
    /// Wartet auf (erneutes) Hochladen
    Pending,
    /// Server-Stand hat sich geändert; der Nutzer entscheidet
    Conflict,
    /// Server lehnt die Änderung dauerhaft ab
    Failed,
}

impl Operation {
    fn as_str(self) -> &'static str {
        match self {
            Operation::Put => "put",
            Operation::Delete => "delete",
        }
    }
}

impl OpState {
    fn as_str(self) -> &'static str {
        match self {
            OpState::Pending => "pending",
            OpState::Conflict => "conflict",
            OpState::Failed => "failed",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "conflict" => OpState::Conflict,
            "failed" => OpState::Failed,
            _ => OpState::Pending,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOp {
    pub id: i64,
    pub calendar_id: i64,
    pub account_id: i64,
    /// URL des Kalenders (Sammlung)
    pub calendar_url: String,
    pub href: String,
    pub operation: Operation,
    pub ics: Option<String>,
    /// ETag des Server-Stands, auf dem die Änderung beruht; `None` = neu anlegen
    pub if_match: Option<String>,
    pub state: OpState,
    pub attempts: i64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PendingSummary {
    pub waiting: i64,
    pub conflicts: i64,
    pub failed: i64,
}

/// hrefs eines Kalenders mit wartender Änderung.
pub(crate) fn pending_hrefs(
    conn: &Connection,
    calendar_id: i64,
) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT href FROM pending WHERE calendar_id = ?1")?;
    let rows = stmt.query_map([calendar_id], |r| r.get(0))?;
    rows.collect()
}

const SELECT_OPS: &str = "SELECT p.id, p.calendar_id, c.account_id, c.href, p.href, p.op, p.ics, p.if_match, p.state, p.attempts, p.last_error
     FROM pending p JOIN calendars c ON c.id = p.calendar_id";

fn read_op(r: &rusqlite::Row) -> rusqlite::Result<PendingOp> {
    let op: String = r.get(5)?;
    let state: String = r.get(8)?;
    Ok(PendingOp {
        id: r.get(0)?,
        calendar_id: r.get(1)?,
        account_id: r.get(2)?,
        calendar_url: r.get(3)?,
        href: r.get(4)?,
        operation: if op == "delete" {
            Operation::Delete
        } else {
            Operation::Put
        },
        ics: r.get(6)?,
        if_match: r.get(7)?,
        state: OpState::parse(&state),
        attempts: r.get(9)?,
        last_error: r.get(10)?,
    })
}

impl Store {
    pub fn resource(&self, calendar_id: i64, href: &str) -> Result<Option<Resource>> {
        Ok(self
            .conn
            .query_row(
                "SELECT calendar_id, href, etag, ics FROM resources WHERE calendar_id = ?1 AND href = ?2",
                params![calendar_id, href],
                |r| Ok(Resource { calendar_id: r.get(0)?, href: r.get(1)?, etag: r.get(2)?, ics: r.get(3)? }),
            )
            .optional()?)
    }

    /// Ressource lokal anlegen oder ändern und zum Hochladen vormerken.
    pub fn local_put(&mut self, calendar_id: i64, href: &str, ics: &str) -> Result<()> {
        let tx = self.write()?;
        let base: Option<String> = tx
            .query_row(
                "SELECT etag FROM resources WHERE calendar_id = ?1 AND href = ?2",
                params![calendar_id, href],
                |r| r.get(0),
            )
            .optional()?;
        let index = index_of(ics);
        tx.execute(
            "INSERT INTO resources (calendar_id, href, etag, ics, kind, uid, start_utc, end_utc)
             VALUES (?1, ?2, '', ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (calendar_id, href) DO UPDATE SET
                ics = excluded.ics, kind = excluded.kind, uid = excluded.uid,
                start_utc = excluded.start_utc, end_utc = excluded.end_utc",
            params![
                calendar_id,
                href,
                ics,
                index.kind,
                index.uid,
                index.start,
                index.end
            ],
        )?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM pending WHERE calendar_id = ?1 AND href = ?2",
                params![calendar_id, href],
                |r| r.get(0),
            )
            .optional()?;
        match existing {
            Some(id) => tx.execute(
                "UPDATE pending SET op = 'put', ics = ?2, state = 'pending', attempts = 0, next_attempt = 0, last_error = NULL
                 WHERE id = ?1",
                params![id, ics],
            )?,
            None => tx.execute(
                "INSERT INTO pending (calendar_id, href, op, ics, if_match) VALUES (?1, ?2, 'put', ?3, ?4)",
                params![calendar_id, href, ics, base.filter(|e| !e.is_empty())],
            )?,
        };
        tx.commit()?;
        Ok(())
    }

    /// Ressource lokal löschen und das Löschen vormerken.
    pub fn local_delete(&mut self, calendar_id: i64, href: &str) -> Result<()> {
        let tx = self.write()?;
        let base: Option<String> = tx
            .query_row(
                "SELECT etag FROM resources WHERE calendar_id = ?1 AND href = ?2",
                params![calendar_id, href],
                |r| r.get(0),
            )
            .optional()?;
        tx.execute(
            "DELETE FROM resources WHERE calendar_id = ?1 AND href = ?2",
            params![calendar_id, href],
        )?;
        let existing: Option<(i64, Option<String>)> = tx
            .query_row(
                "SELECT id, if_match FROM pending WHERE calendar_id = ?1 AND href = ?2",
                params![calendar_id, href],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match existing {
            // Nie auf dem Server angekommen: einfach vergessen
            Some((id, None)) => {
                tx.execute("DELETE FROM pending WHERE id = ?1", [id])?;
            }
            Some((id, Some(_))) => {
                tx.execute(
                    "UPDATE pending SET op = 'delete', ics = NULL, state = 'pending', attempts = 0, next_attempt = 0, last_error = NULL
                     WHERE id = ?1",
                    [id],
                )?;
            }
            None => {
                if let Some(etag) = base.filter(|e| !e.is_empty()) {
                    tx.execute(
                        "INSERT INTO pending (calendar_id, href, op, if_match) VALUES (?1, ?2, 'delete', ?3)",
                        params![calendar_id, href, etag],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Einträge, die jetzt hochgeladen werden sollen (`now` in Sekunden seit 1970).
    pub fn pending_due(&self, now: i64) -> Result<Vec<PendingOp>> {
        let mut stmt = self.conn.prepare(&format!(
            "{SELECT_OPS} WHERE p.state = 'pending' AND p.next_attempt <= ?1 ORDER BY p.id"
        ))?;
        let rows = stmt.query_map([now], read_op)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn pending_ops(&self) -> Result<Vec<PendingOp>> {
        let mut stmt = self.conn.prepare(&format!("{SELECT_OPS} ORDER BY p.id"))?;
        let rows = stmt.query_map([], read_op)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Stand der wartenden Änderung an einer Ressource und bisherige
    /// Versuche; `None` = so auf dem Server
    pub fn pending_state(&self, calendar_id: i64, href: &str) -> Result<Option<(OpState, i64)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT state, attempts FROM pending WHERE calendar_id = ?1 AND href = ?2",
                params![calendar_id, href],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
            .map(|(s, attempts)| (OpState::parse(&s), attempts)))
    }

    pub fn pending_summary(&self) -> Result<PendingSummary> {
        Ok(self.conn.query_row(
            "SELECT
                coalesce(sum(state = 'pending'), 0),
                coalesce(sum(state = 'conflict'), 0),
                coalesce(sum(state = 'failed'), 0)
             FROM pending",
            [],
            |r| {
                Ok(PendingSummary {
                    waiting: r.get(0)?,
                    conflicts: r.get(1)?,
                    failed: r.get(2)?,
                })
            },
        )?)
    }

    /// Hochladen von `sent` hat geklappt. Mit `server_ics` wird die vom
    /// Server gespeicherte Fassung übernommen (manche Server ändern beim
    /// Speichern).
    ///
    /// Wurde die Ressource während des Hochladens erneut geändert (der
    /// Journal-Editor speichert laufend), bleibt der Eintrag mit der neueren
    /// Fassung stehen und wird gegen den neuen Server-Stand gesendet; die
    /// lokale Fassung wird dann nicht überschrieben. Wurde eine neue
    /// Ressource währenddessen lokal gelöscht (oder verschoben), wird das
    /// Löschen auf dem Server nachgeholt.
    pub fn complete_pending(
        &mut self,
        sent: &PendingOp,
        etag: Option<&str>,
        server_ics: Option<&str>,
    ) -> Result<()> {
        let tx = self.write()?;
        let Some((calendar_id, href, op, ics)) = tx
            .query_row(
                "SELECT calendar_id, href, op, ics FROM pending WHERE id = ?1",
                [sent.id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?
        else {
            let still_here: bool = tx.query_row(
                "SELECT count(*) FROM resources WHERE calendar_id = ?1 AND href = ?2",
                params![sent.calendar_id, sent.href],
                |r| r.get::<_, i64>(0),
            )? > 0;
            if sent.operation == Operation::Put && !still_here {
                tx.execute(
                    "INSERT INTO pending (calendar_id, href, op, if_match) VALUES (?1, ?2, 'delete', ?3)",
                    params![sent.calendar_id, sent.href, etag],
                )?;
                tx.commit()?;
            }
            return Ok(());
        };
        if op != sent.operation.as_str() || ics != sent.ics {
            // Nach einem Löschen gibt es die Ressource auf dem Server nicht mehr
            let base = match sent.operation {
                Operation::Put => etag,
                Operation::Delete => None,
            };
            tx.execute(
                "UPDATE pending SET if_match = ?2 WHERE id = ?1",
                params![sent.id, base],
            )?;
            if sent.operation == Operation::Put {
                tx.execute(
                    "UPDATE resources SET etag = ?3 WHERE calendar_id = ?1 AND href = ?2",
                    params![calendar_id, href, etag.unwrap_or_default()],
                )?;
            }
            tx.commit()?;
            return Ok(());
        }
        tx.execute("DELETE FROM pending WHERE id = ?1", [sent.id])?;
        if op == "put" {
            if let Some(ics) = server_ics {
                let index = index_of(ics);
                tx.execute(
                    "UPDATE resources SET ics = ?3, kind = ?4, uid = ?5, start_utc = ?6, end_utc = ?7
                     WHERE calendar_id = ?1 AND href = ?2",
                    params![calendar_id, href, ics, index.kind, index.uid, index.start, index.end],
                )?;
            }
            // Ohne ETag holt der nächste Sync die Ressource neu
            tx.execute(
                "UPDATE resources SET etag = ?3 WHERE calendar_id = ?1 AND href = ?2",
                params![calendar_id, href, etag.unwrap_or_default()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Später erneut versuchen (Netz weg, Server überlastet).
    pub fn retry_pending_later(&self, op_id: i64, next_attempt: i64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE pending SET attempts = attempts + 1, next_attempt = ?2, last_error = ?3 WHERE id = ?1",
            params![op_id, next_attempt, error],
        )?;
        Ok(())
    }

    pub fn mark_pending(&self, op_id: i64, state: OpState, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE pending SET state = ?2, last_error = ?3 WHERE id = ?1",
            params![op_id, state.as_str(), error],
        )?;
        Ok(())
    }

    /// Eigene Version trotz Konflikt behalten: gegen den aktuellen
    /// Server-Stand erneut senden (`None` = auf dem Server inzwischen gelöscht).
    pub fn retry_pending_against(&self, op_id: i64, server_etag: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE pending SET if_match = ?2, state = 'pending', attempts = 0, next_attempt = 0, last_error = NULL
             WHERE id = ?1",
            params![op_id, server_etag],
        )?;
        Ok(())
    }

    /// Lokale Änderung verwerfen. Der Kalender wird beim nächsten Sync
    /// vollständig abgeglichen und erhält so den Server-Stand zurück.
    pub fn discard_pending(&mut self, op_id: i64) -> Result<()> {
        let tx = self.write()?;
        let calendar_id: Option<i64> = tx
            .query_row(
                "SELECT calendar_id FROM pending WHERE id = ?1",
                [op_id],
                |r| r.get(0),
            )
            .optional()?;
        tx.execute("DELETE FROM pending WHERE id = ?1", [op_id])?;
        if let Some(calendar_id) = calendar_id {
            tx.execute(
                "UPDATE calendars SET ctag = NULL, sync_token = NULL WHERE id = ?1",
                [calendar_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    fn event(uid: &str, summary: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:20261109T070000Z\r\nDTEND:20261109T080000Z\r\nSUMMARY:{summary}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    }

    #[test]
    fn two_connections_write_at_the_same_time() {
        // Oberfläche und Sync-Thread haben je eine Verbindung auf dieselbe Datei
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hora.db");
        let calendar = setup_in(Store::open(&path).unwrap()).1;
        let writers: Vec<_> = (0..2)
            .map(|w| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let mut store = Store::open(&path).unwrap();
                    for i in 0..60 {
                        let href = format!("/cal/{w}-{}.ics", i % 7);
                        store
                            .local_put(calendar, &href, &event(&format!("{w}-{i}"), "x"))
                            .unwrap();
                        let due = store.pending_due(i64::MAX).unwrap();
                        if let Some(op) = due.first() {
                            store.complete_pending(op, Some("\"e\""), None).unwrap();
                        }
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().expect("kein \"database is locked\"");
        }
    }

    fn setup() -> (Store, i64) {
        setup_in(Store::open_in_memory().unwrap())
    }

    fn setup_in(store: Store) -> (Store, i64) {
        let mut store = store;
        let account = store
            .add_account(&NewAccount {
                name: "Test",
                url: "http://dav.test/",
                username: "u",
                credential_key: "k",
                principal_url: None,
                home_url: None,
            })
            .unwrap();
        store
            .sync_calendar_list(
                account,
                &[RemoteCalendar {
                    href: "http://dav.test/cal/",
                    name: "Privat",
                    color: None,
                    components: "",
                    read_only: false,
                }],
            )
            .unwrap();
        let id = store.calendars().unwrap()[0].id;
        (store, id)
    }

    fn summary_of(store: &Store, id: i64, href: &str) -> String {
        let ics = store.resource(id, href).unwrap().unwrap().ics;
        let (events, _) = hora_core::model::CalendarObject::parse(&ics)
            .unwrap()
            .events();
        events[0].summary.clone()
    }

    #[test]
    fn new_resource_is_visible_at_once_and_queued_as_create() {
        let (mut store, cal) = setup();
        store
            .local_put(cal, "/cal/neu.ics", &event("neu", "Erst"))
            .unwrap();
        store
            .local_put(cal, "/cal/neu.ics", &event("neu", "Dann"))
            .unwrap();
        let ops = store.pending_ops().unwrap();
        assert_eq!(ops.len(), 1, "zusammengefasst");
        assert_eq!(ops[0].operation, Operation::Put);
        assert_eq!(ops[0].if_match, None);
        assert_eq!(ops[0].calendar_url, "http://dav.test/cal/");
        assert!(ops[0].ics.as_deref().unwrap().contains("SUMMARY:Dann"));
        assert_eq!(summary_of(&store, cal, "/cal/neu.ics"), "Dann");
        assert_eq!(store.events_in_range(0, i64::MAX / 2).unwrap().len(), 1);
    }

    #[test]
    fn edit_of_synced_resource_expects_its_server_etag() {
        let (mut store, cal) = setup();
        let ics = event("a", "Server");
        store
            .apply_sync(
                cal,
                &SyncBatch {
                    upserts: vec![("/cal/a.ics", "\"7\"", &ics)],
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .local_put(cal, "/cal/a.ics", &event("a", "Lokal"))
            .unwrap();
        store
            .local_put(cal, "/cal/a.ics", &event("a", "Lokal 2"))
            .unwrap();
        let ops = store.pending_ops().unwrap();
        assert_eq!(ops[0].if_match.as_deref(), Some("\"7\""));
    }

    #[test]
    fn deleting_never_uploaded_resource_leaves_nothing() {
        let (mut store, cal) = setup();
        store
            .local_put(cal, "/cal/neu.ics", &event("neu", "x"))
            .unwrap();
        store.local_delete(cal, "/cal/neu.ics").unwrap();
        assert!(store.pending_ops().unwrap().is_empty());
        assert!(store.resource(cal, "/cal/neu.ics").unwrap().is_none());
    }

    #[test]
    fn deleting_synced_resource_queues_delete_with_etag() {
        let (mut store, cal) = setup();
        let ics = event("a", "Server");
        store
            .apply_sync(
                cal,
                &SyncBatch {
                    upserts: vec![("/cal/a.ics", "\"7\"", &ics)],
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .local_put(cal, "/cal/a.ics", &event("a", "Lokal"))
            .unwrap();
        store.local_delete(cal, "/cal/a.ics").unwrap();
        let ops = store.pending_ops().unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].operation, Operation::Delete);
        assert_eq!(ops[0].if_match.as_deref(), Some("\"7\""));
        assert!(store.events_in_range(0, i64::MAX / 2).unwrap().is_empty());
    }

    #[test]
    fn sync_leaves_resources_with_pending_changes_alone() {
        let (mut store, cal) = setup();
        let server = event("a", "Server");
        store
            .apply_sync(
                cal,
                &SyncBatch {
                    upserts: vec![("/cal/a.ics", "\"1\"", &server)],
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .local_put(cal, "/cal/a.ics", &event("a", "Lokal"))
            .unwrap();
        store
            .local_put(cal, "/cal/neu.ics", &event("neu", "Neu"))
            .unwrap();
        // Server meldet eine andere Fassung von a; Vollabgleich kennt neu.ics nicht
        let newer = event("a", "Server neuer");
        store
            .apply_sync(
                cal,
                &SyncBatch {
                    upserts: vec![("/cal/a.ics", "\"2\"", &newer)],
                    full: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(summary_of(&store, cal, "/cal/a.ics"), "Lokal");
        assert!(store.resource(cal, "/cal/neu.ics").unwrap().is_some());
    }

    #[test]
    fn completed_upload_updates_etag_and_server_version() {
        let (mut store, cal) = setup();
        store
            .local_put(cal, "/cal/neu.ics", &event("neu", "Lokal"))
            .unwrap();
        let op = store.pending_due(0).unwrap().remove(0);
        assert_eq!(
            store.pending_state(cal, "/cal/neu.ics").unwrap(),
            Some((OpState::Pending, 0))
        );
        store
            .complete_pending(&op, Some("\"9\""), Some(&event("neu", "Vom Server")))
            .unwrap();
        assert!(store.pending_ops().unwrap().is_empty());
        assert_eq!(store.pending_state(cal, "/cal/neu.ics").unwrap(), None);
        let res = store.resource(cal, "/cal/neu.ics").unwrap().unwrap();
        assert_eq!(res.etag, "\"9\"");
        assert_eq!(summary_of(&store, cal, "/cal/neu.ics"), "Vom Server");
    }

    #[test]
    fn delete_during_first_upload_is_sent_afterwards() {
        let (mut store, cal) = setup();
        store
            .local_put(cal, "/cal/neu.ics", &event("neu", "Gleich weg"))
            .unwrap();
        let sent = store.pending_due(0).unwrap().remove(0);
        // Noch nie hochgeladen: lokal einfach vergessen …
        store.local_delete(cal, "/cal/neu.ics").unwrap();
        assert!(store.pending_ops().unwrap().is_empty());
        // … aber das Hochladen lief schon: danach auf dem Server löschen
        store.complete_pending(&sent, Some("\"1\""), None).unwrap();
        let ops = store.pending_ops().unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].operation, Operation::Delete);
        assert_eq!(ops[0].if_match.as_deref(), Some("\"1\""));
        assert!(store.resource(cal, "/cal/neu.ics").unwrap().is_none());
        // Das Löschen selbst hinterlässt nichts
        store.complete_pending(&ops[0], None, None).unwrap();
        assert!(store.pending_ops().unwrap().is_empty());
    }

    #[test]
    fn edit_during_upload_is_kept_and_sent_against_the_new_etag() {
        let (mut store, cal) = setup();
        store
            .local_put(cal, "/cal/neu.ics", &event("neu", "Erster Stand"))
            .unwrap();
        // Der Sync-Thread lädt hoch, während der Editor weiter speichert
        let sent = store.pending_due(0).unwrap().remove(0);
        store
            .local_put(cal, "/cal/neu.ics", &event("neu", "Weiter getippt"))
            .unwrap();
        store
            .complete_pending(&sent, Some("\"1\""), Some(&event("neu", "Erster Stand")))
            .unwrap();
        let ops = store.pending_ops().unwrap();
        assert_eq!(ops.len(), 1, "neuere Fassung wartet noch");
        assert_eq!(ops[0].if_match.as_deref(), Some("\"1\""));
        assert!(ops[0].ics.as_deref().unwrap().contains("Weiter getippt"));
        assert_eq!(summary_of(&store, cal, "/cal/neu.ics"), "Weiter getippt");
        assert_eq!(
            store.resource(cal, "/cal/neu.ics").unwrap().unwrap().etag,
            "\"1\""
        );
        // Danach geht die neuere Fassung durch
        let again = store.pending_due(0).unwrap().remove(0);
        store.complete_pending(&again, Some("\"2\""), None).unwrap();
        assert!(store.pending_ops().unwrap().is_empty());
        assert_eq!(summary_of(&store, cal, "/cal/neu.ics"), "Weiter getippt");
    }

    #[test]
    fn retries_wait_until_due_and_count_attempts() {
        let (mut store, cal) = setup();
        store
            .local_put(cal, "/cal/neu.ics", &event("neu", "x"))
            .unwrap();
        let op = store.pending_due(100).unwrap().remove(0);
        store
            .retry_pending_later(op.id, 200, "Server nicht erreichbar")
            .unwrap();
        assert!(store.pending_due(150).unwrap().is_empty());
        let due = store.pending_due(200).unwrap();
        assert_eq!(due[0].attempts, 1);
        assert_eq!(
            due[0].last_error.as_deref(),
            Some("Server nicht erreichbar")
        );
        assert_eq!(
            store.pending_summary().unwrap(),
            PendingSummary {
                waiting: 1,
                conflicts: 0,
                failed: 0
            }
        );
    }

    #[test]
    fn conflicts_can_be_retried_or_discarded() {
        let (mut store, cal) = setup();
        let ics = event("a", "Server");
        store
            .apply_sync(
                cal,
                &SyncBatch {
                    upserts: vec![("/cal/a.ics", "\"1\"", &ics)],
                    sync_token: Some("t"),
                    ctag: Some("c"),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .local_put(cal, "/cal/a.ics", &event("a", "Lokal"))
            .unwrap();
        let op = store.pending_ops().unwrap().remove(0);
        store
            .mark_pending(op.id, OpState::Conflict, "geändert")
            .unwrap();
        assert_eq!(store.pending_summary().unwrap().conflicts, 1);
        assert!(
            store.pending_due(i64::MAX).unwrap().is_empty(),
            "Konflikte warten auf den Nutzer"
        );

        store.retry_pending_against(op.id, Some("\"2\"")).unwrap();
        let again = store.pending_due(0).unwrap().remove(0);
        assert_eq!(again.if_match.as_deref(), Some("\"2\""));

        store.discard_pending(again.id).unwrap();
        assert!(store.pending_ops().unwrap().is_empty());
        let cal_row = &store.calendars().unwrap()[0];
        assert_eq!(
            (cal_row.ctag.clone(), cal_row.sync_token.clone()),
            (None, None),
            "Vollabgleich beim nächsten Sync"
        );
    }
}
