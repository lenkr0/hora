//! Lokaler Offline-Cache in SQLite: Konten, Kalender und die `.ics`-Ressourcen
//! so, wie sie vom Server kamen, dazu die Änderungs-Warteschlange für
//! Offline-Bearbeitung ([`pending`]) und die App-Zeiten ([`activities`]).
//!
//! Die Datenbank läuft im WAL-Modus, damit die Oberfläche lesen kann,
//! während der Sync-Thread schreibt.

use std::collections::HashMap;
use std::path::Path;

use hora_core::Tz;
use hora_core::model::{CalendarObject, Kind};
use rusqlite::{Connection, OptionalExtension, params};

pub mod activities;
mod migrations;
pub mod pending;
mod reminders;

pub use activities::{Activity, ActivityDraft};
pub use pending::{OpState, Operation, PendingOp, PendingSummary};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}", hora_i18n::tr!("Database: {}", .0))]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub username: String,
    /// Schlüssel des Passworts im Anmeldeinformationsspeicher
    pub credential_key: String,
    pub principal_url: Option<String>,
    pub home_url: Option<String>,
    /// Konto für Kalender nur auf diesem PC (kein Server, kein Abgleich)
    pub local: bool,
}

#[derive(Debug, Clone)]
pub struct NewAccount<'a> {
    pub name: &'a str,
    pub url: &'a str,
    pub username: &'a str,
    pub credential_key: &'a str,
    pub principal_url: Option<&'a str>,
    pub home_url: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calendar {
    pub id: i64,
    pub account_id: i64,
    /// Vollständige URL der Sammlung
    pub href: String,
    pub name: String,
    pub server_color: Option<String>,
    /// Lokal gewählte Farbe, hat Vorrang vor der Serverfarbe
    pub local_color: Option<String>,
    pub visible: bool,
    pub read_only: bool,
    /// Unterstützte Komponenten, z. B. `VEVENT,VTODO`
    pub components: String,
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
    /// Nur auf diesem PC: Änderungen werden nicht hochgeladen
    pub local: bool,
}

impl Calendar {
    pub fn color(&self) -> Option<&str> {
        self.local_color.as_deref().or(self.server_color.as_deref())
    }

    pub fn supports(&self, kind: Kind) -> bool {
        self.components.is_empty()
            || self
                .components
                .split(',')
                .any(|c| c.trim().eq_ignore_ascii_case(kind.component_name()))
    }
}

/// Kalender, wie ihn die Discovery meldet.
#[derive(Debug, Clone)]
pub struct RemoteCalendar<'a> {
    pub href: &'a str,
    pub name: &'a str,
    pub color: Option<&'a str>,
    pub components: &'a str,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub calendar_id: i64,
    pub href: String,
    pub etag: String,
    pub ics: String,
}

/// Ergebnis eines Sync-Laufs für einen Kalender, atomar übernommen.
#[derive(Debug, Default)]
pub struct SyncBatch<'a> {
    /// (href, etag, ics)
    pub upserts: Vec<(&'a str, &'a str, &'a str)>,
    pub deletes: Vec<&'a str>,
    /// Vollständiger Abgleich: alles, was nicht in `upserts` oder `keep` steht, entfällt
    pub full: bool,
    pub keep: Vec<&'a str>,
    pub ctag: Option<&'a str>,
    pub sync_token: Option<&'a str>,
}

/// Indexwerte einer Ressource für Zeitraumabfragen (Sekunden seit 1970, UTC).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Index {
    kind: Option<&'static str>,
    uid: Option<String>,
    start: Option<i64>,
    /// `None` bei Serien ohne Ende
    end: Option<i64>,
}

/// Ganztägige und schwebende Termine werden in UTC indiziert; Abfragen
/// erweitern den Zeitraum um diesen Puffer, damit keine Zeitzone durchfällt.
const ZONE_MARGIN: i64 = 26 * 3600;

fn index_of(ics: &str) -> Index {
    let Ok(object) = CalendarObject::parse(ics) else {
        return Index {
            kind: None,
            uid: None,
            start: None,
            end: None,
        };
    };
    let (events, _) = object.events();
    let mut start: Option<i64> = None;
    let mut end: Option<i64> = Some(i64::MIN);
    for event in &events {
        let s = event.start.to_utc(Tz::UTC).timestamp();
        start = Some(start.map_or(s, |v| v.min(s)));
        if event.is_recurring() {
            end = None;
        } else if let Some(e) = end {
            end = Some(e.max(event.end.to_utc(Tz::UTC).timestamp().max(s)));
        }
    }
    Index {
        kind: object.kind().map(Kind::component_name),
        uid: object.uid(),
        start,
        end: if events.is_empty() { None } else { end },
    }
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        // Gibt den neuen Modus zurück ("memory" bei In-Memory-Datenbanken)
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // Mit WAL empfohlen: Commits warten nicht mehr auf die Platte, nur
        // Checkpoints. Bei einem Absturz von Hora geht nichts verloren, nur
        // bei Stromausfall eventuell die letzten Sekunden. Dadurch halten
        // Oberfläche und Sync-Thread die Schreibsperre viel kürzer.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // Großzügig warten, falls der andere Thread gerade viel schreibt
        // (z. B. erster Abgleich mit Tausenden Terminen)
        conn.busy_timeout(std::time::Duration::from_secs(15))?;
        migrations::run(&conn)?;
        Ok(Store { conn })
    }

    /// Schreib-Transaktion, die die Sperre gleich zu Beginn holt. Bei WAL
    /// scheitert eine Transaktion, die erst liest und dann schreibt, sonst
    /// sofort mit "database is locked", wenn der Sync-Thread (eigene
    /// Verbindung) dazwischen geschrieben hat; so greift die Wartezeit.
    fn write(&mut self) -> Result<rusqlite::Transaction<'_>> {
        Ok(self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?)
    }

    // ---- Konten ----

    pub fn add_account(&self, account: &NewAccount) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO accounts (name, url, username, credential_key, principal_url, home_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                account.name,
                account.url,
                account.username,
                account.credential_key,
                account.principal_url,
                account.home_url
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, url, username, credential_key, principal_url, home_url, kind = 'local'
             FROM accounts ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Account {
                id: r.get(0)?,
                name: r.get(1)?,
                url: r.get(2)?,
                username: r.get(3)?,
                credential_key: r.get(4)?,
                principal_url: r.get(5)?,
                home_url: r.get(6)?,
                local: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_account(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---- Kalender ----

    /// Legt Kalender an oder aktualisiert Name, Serverfarbe und Rechte.
    /// Sichtbarkeit, lokale Farbe und Sync-Stand bleiben erhalten. Kalender
    /// des Kontos, die der Server nicht mehr meldet, werden entfernt.
    pub fn sync_calendar_list(&mut self, account_id: i64, remote: &[RemoteCalendar]) -> Result<()> {
        let tx = self.write()?;
        for (order, cal) in remote.iter().enumerate() {
            tx.execute(
                "INSERT INTO calendars (account_id, href, name, server_color, components, read_only, sort_order)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (account_id, href) DO UPDATE SET
                    name = excluded.name, server_color = excluded.server_color,
                    components = excluded.components, read_only = excluded.read_only,
                    sort_order = excluded.sort_order",
                params![account_id, cal.href, cal.name, cal.color, cal.components, cal.read_only, order as i64],
            )?;
        }
        let known: Vec<&str> = remote.iter().map(|c| c.href).collect();
        let mut stmt = tx.prepare("SELECT id, href FROM calendars WHERE account_id = ?1")?;
        let stale: Vec<i64> = stmt
            .query_map([account_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?
            .filter_map(|row| row.ok())
            .filter(|(_, href)| !known.contains(&href.as_str()))
            .map(|(id, _)| id)
            .collect();
        drop(stmt);
        for id in stale {
            tx.execute("DELETE FROM calendars WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn calendars(&self) -> Result<Vec<Calendar>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.account_id, c.href, c.name, c.server_color, c.local_color, c.visible,
                    c.read_only, c.components, c.ctag, c.sync_token, a.kind = 'local'
             FROM calendars c JOIN accounts a ON a.id = c.account_id
             ORDER BY c.account_id, c.sort_order, c.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Calendar {
                id: r.get(0)?,
                account_id: r.get(1)?,
                href: r.get(2)?,
                name: r.get(3)?,
                server_color: r.get(4)?,
                local_color: r.get(5)?,
                visible: r.get(6)?,
                read_only: r.get(7)?,
                components: r.get(8)?,
                ctag: r.get(9)?,
                sync_token: r.get(10)?,
                local: r.get(11)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- Kalender nur auf diesem PC ----

    /// Konto für lokale Kalender; beim ersten Mal angelegt. Es hat weder
    /// Server noch Passwort, der Abgleich überspringt es.
    pub fn local_account(&self) -> Result<i64> {
        let found: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM accounts WHERE kind = 'local' ORDER BY id LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = found {
            return Ok(id);
        }
        self.conn.execute(
            "INSERT INTO accounts (name, url, username, credential_key, kind)
             VALUES ('', '', '', 'local', 'local')",
            [],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Neuer Kalender nur auf diesem PC, für Termine, Aufgaben und Journal
    pub fn add_local_calendar(&mut self, name: &str, color: &str) -> Result<i64> {
        let account = self.local_account()?;
        let tx = self.write()?;
        let order: i64 = tx.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM calendars WHERE account_id = ?1",
            [account],
            |r| r.get(0),
        )?;
        // Statt einer Server-Adresse eine eigene, an die sich Termine wie
        // "<uid>.ics" anhängen lassen
        let next: i64 =
            tx.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM calendars", [], |r| {
                r.get(0)
            })?;
        tx.execute(
            "INSERT INTO calendars (account_id, href, name, local_color, components, sort_order)
             VALUES (?1, ?2, ?3, ?4, 'VEVENT,VTODO,VJOURNAL', ?5)",
            params![
                account,
                format!("hora-local://pc/{next}/"),
                name,
                color,
                order
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// Lokalen Kalender umbenennen (Server-Kalender benennt der Server)
    pub fn rename_local_calendar(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE calendars SET name = ?2
             WHERE id = ?1 AND account_id IN (SELECT id FROM accounts WHERE kind = 'local')",
            params![id, name],
        )?;
        Ok(())
    }

    /// Lokalen Kalender mit allen Terminen, Aufgaben und Einträgen löschen
    pub fn delete_local_calendar(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM calendars
             WHERE id = ?1 AND account_id IN (SELECT id FROM accounts WHERE kind = 'local')",
            [id],
        )?;
        Ok(())
    }

    pub fn set_calendar_visible(&self, id: i64, visible: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE calendars SET visible = ?2 WHERE id = ?1",
            params![id, visible],
        )?;
        Ok(())
    }

    pub fn set_calendar_color(&self, id: i64, color: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE calendars SET local_color = ?2 WHERE id = ?1",
            params![id, color],
        )?;
        Ok(())
    }

    // ---- Ressourcen ----

    pub fn etags(&self, calendar_id: i64) -> Result<HashMap<String, String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT href, etag FROM resources WHERE calendar_id = ?1")?;
        let rows = stmt.query_map([calendar_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Übernimmt das Ergebnis eines Sync-Laufs in einer Transaktion.
    /// Ressourcen mit wartender lokaler Änderung bleiben unberührt; sie
    /// werden nach dem Hochladen bzw. Verwerfen wieder abgeglichen.
    pub fn apply_sync(&mut self, calendar_id: i64, batch: &SyncBatch) -> Result<()> {
        let tx = self.write()?;
        let protected = pending::pending_hrefs(&tx, calendar_id)?;
        for &(href, etag, ics) in batch
            .upserts
            .iter()
            .filter(|(h, _, _)| !protected.contains(*h))
        {
            let index = index_of(ics);
            tx.execute(
                "INSERT INTO resources (calendar_id, href, etag, ics, kind, uid, start_utc, end_utc)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (calendar_id, href) DO UPDATE SET
                    etag = excluded.etag, ics = excluded.ics, kind = excluded.kind, uid = excluded.uid,
                    start_utc = excluded.start_utc, end_utc = excluded.end_utc",
                params![calendar_id, href, etag, ics, index.kind, index.uid, index.start, index.end],
            )?;
        }
        for href in batch.deletes.iter().filter(|h| !protected.contains(**h)) {
            tx.execute(
                "DELETE FROM resources WHERE calendar_id = ?1 AND href = ?2",
                params![calendar_id, href],
            )?;
        }
        if batch.full {
            let mut stmt = tx.prepare("SELECT href FROM resources WHERE calendar_id = ?1")?;
            let existing: Vec<String> = stmt
                .query_map([calendar_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            drop(stmt);
            for href in existing.into_iter().filter(|h| !protected.contains(h)) {
                let listed = batch.upserts.iter().any(|(h, _, _)| *h == href)
                    || batch.keep.contains(&href.as_str());
                if !listed {
                    tx.execute(
                        "DELETE FROM resources WHERE calendar_id = ?1 AND href = ?2",
                        params![calendar_id, href],
                    )?;
                }
            }
        }
        tx.execute(
            "UPDATE calendars SET ctag = ?2, sync_token = ?3, last_sync = strftime('%s', 'now') WHERE id = ?1",
            params![calendar_id, batch.ctag, batch.sync_token],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Termine sichtbarer Kalender, die `[from, to)` (Sekunden, UTC)
    /// berühren können. Serien ohne Ende sind immer dabei.
    pub fn events_in_range(&self, from: i64, to: i64) -> Result<Vec<Resource>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.calendar_id, r.href, r.etag, r.ics FROM resources r
             JOIN calendars c ON c.id = r.calendar_id
             WHERE c.visible = 1 AND r.kind = 'VEVENT'
               AND r.start_utc < ?2 AND (r.end_utc IS NULL OR r.end_utc > ?1)",
        )?;
        let rows = stmt.query_map(params![from - ZONE_MARGIN, to + ZONE_MARGIN], |r| {
            Ok(Resource {
                calendar_id: r.get(0)?,
                href: r.get(1)?,
                etag: r.get(2)?,
                ics: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn resource_count(&self, calendar_id: i64) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM resources WHERE calendar_id = ?1",
            [calendar_id],
            |r| r.get(0),
        )?)
    }

    // ---- Einstellungen ----

    /// Alle Aufgaben (VTODO) sichtbarer Kalender
    pub fn tasks(&self) -> Result<Vec<Resource>> {
        self.resources_of_kind("VTODO")
    }

    /// Alle Termine (VEVENT) sichtbarer Kalender, z. B. für die Suche
    /// Alle Journaleinträge (VJOURNAL) sichtbarer Kalender
    pub fn journals(&self) -> Result<Vec<Resource>> {
        self.resources_of_kind("VJOURNAL")
    }

    pub fn all_events(&self) -> Result<Vec<Resource>> {
        self.resources_of_kind("VEVENT")
    }

    fn resources_of_kind(&self, kind: &str) -> Result<Vec<Resource>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.calendar_id, r.href, r.etag, r.ics FROM resources r
             JOIN calendars c ON c.id = r.calendar_id
             WHERE c.visible = 1 AND r.kind = ?1
             ORDER BY r.calendar_id, r.href",
        )?;
        let rows = stmt.query_map([kind], |r| {
            Ok(Resource {
                calendar_id: r.get(0)?,
                href: r.get(1)?,
                etag: r.get(2)?,
                ics: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(uid: &str, start: &str, end: &str, extra: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:{start}\r\nDTEND:{end}\r\n{extra}END:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    }

    fn ts(s: &str) -> i64 {
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M")
            .unwrap()
            .and_utc()
            .timestamp()
    }

    fn store_with_calendar() -> (Store, i64) {
        let mut store = Store::open_in_memory().unwrap();
        let account = store
            .add_account(&NewAccount {
                name: "Baïkal",
                url: "http://dav.test/dav.php",
                username: "testuser",
                credential_key: "k1",
                principal_url: None,
                home_url: None,
            })
            .unwrap();
        store
            .sync_calendar_list(
                account,
                &[RemoteCalendar {
                    href: "http://dav.test/dav.php/calendars/testuser/default/",
                    name: "Privat",
                    color: Some("#4f7cac"),
                    components: "VEVENT,VTODO",
                    read_only: false,
                }],
            )
            .unwrap();
        let id = store.calendars().unwrap()[0].id;
        (store, id)
    }

    #[test]
    fn local_calendars_stay_on_this_pc() {
        let (mut store, server) = store_with_calendar();
        let id = store.add_local_calendar("Zuhause", "#4f7cac").unwrap();
        let second = store.add_local_calendar("Sport", "#5b8a5a").unwrap();
        // Ein lokales Konto, unabhängig vom Server-Konto
        let accounts = store.accounts().unwrap();
        assert_eq!(accounts.iter().filter(|a| a.local).count(), 1);
        assert!(!accounts[0].local);
        let calendars = store.calendars().unwrap();
        let local = calendars.iter().find(|c| c.id == id).unwrap();
        assert!(local.local);
        assert!(local.supports(Kind::Event) && local.supports(Kind::Todo));
        assert_eq!(local.color(), Some("#4f7cac"));
        assert!(!calendars.iter().find(|c| c.id == server).unwrap().local);
        assert_ne!(
            local.href,
            calendars.iter().find(|c| c.id == second).unwrap().href
        );
        // Termine lassen sich an die Adresse anhängen (wie bei Server-Kalendern)
        let base = url::Url::parse(&local.href).unwrap();
        assert_eq!(base.join("a.ics").unwrap().path(), format!("/{id}/a.ics"));

        // Änderungen landen nicht in der Warteschlange, Server-Kalender schon
        let ics = event("a", "20261116T090000Z", "20261116T100000Z", "");
        store.local_put(id, "a.ics", &ics).unwrap();
        store.local_put(server, "b.ics", &ics).unwrap();
        let ops = store.pending_ops().unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].calendar_id, server);
        assert!(store.resource(id, "a.ics").unwrap().is_some());
        store.local_delete(id, "a.ics").unwrap();
        assert!(store.resource(id, "a.ics").unwrap().is_none());
        assert_eq!(store.pending_ops().unwrap().len(), 1);

        // Umbenennen und Löschen nur für lokale Kalender
        store.rename_local_calendar(id, "Daheim").unwrap();
        store.rename_local_calendar(server, "Nein").unwrap();
        store.local_put(id, "c.ics", &ics).unwrap();
        store.delete_local_calendar(id).unwrap();
        store.delete_local_calendar(server).unwrap();
        let calendars = store.calendars().unwrap();
        assert!(calendars.iter().all(|c| c.id != id));
        assert_eq!(
            calendars.iter().find(|c| c.id == server).unwrap().name,
            "Privat"
        );
        assert!(store.resource(id, "c.ics").unwrap().is_none());
        assert_eq!(
            calendars.iter().find(|c| c.id == second).unwrap().name,
            "Sport"
        );
    }

    #[test]
    fn calendar_list_keeps_local_choices() {
        let (mut store, id) = store_with_calendar();
        store.set_calendar_visible(id, false).unwrap();
        store.set_calendar_color(id, Some("#b8645a")).unwrap();
        let account = store.calendars().unwrap()[0].account_id;
        store
            .sync_calendar_list(
                account,
                &[RemoteCalendar {
                    href: "http://dav.test/dav.php/calendars/testuser/default/",
                    name: "Privat (umbenannt)",
                    color: Some("#000000"),
                    components: "VEVENT",
                    read_only: true,
                }],
            )
            .unwrap();
        let cal = &store.calendars().unwrap()[0];
        assert_eq!(cal.id, id);
        assert_eq!(cal.name, "Privat (umbenannt)");
        assert!(!cal.visible);
        assert_eq!(cal.color(), Some("#b8645a"));
        assert!(cal.read_only);
        assert!(cal.supports(Kind::Event) && !cal.supports(Kind::Todo));
    }

    #[test]
    fn removed_calendars_disappear_with_their_events() {
        let (mut store, id) = store_with_calendar();
        let ics = event("a", "20261109T070000Z", "20261109T080000Z", "");
        store
            .apply_sync(
                id,
                &SyncBatch {
                    upserts: vec![("/a.ics", "\"1\"", &ics)],
                    ..Default::default()
                },
            )
            .unwrap();
        let account = store.calendars().unwrap()[0].account_id;
        store.sync_calendar_list(account, &[]).unwrap();
        assert!(store.calendars().unwrap().is_empty());
        assert!(store.events_in_range(0, i64::MAX / 2).unwrap().is_empty());
    }

    #[test]
    fn sync_batches_upsert_delete_and_full_resync() {
        let (mut store, id) = store_with_calendar();
        let a = event("a", "20261109T070000Z", "20261109T080000Z", "");
        let b = event("b", "20261110T070000Z", "20261110T080000Z", "");
        store
            .apply_sync(
                id,
                &SyncBatch {
                    upserts: vec![("/a.ics", "\"1\"", &a), ("/b.ics", "\"1\"", &b)],
                    sync_token: Some("t1"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(store.resource_count(id).unwrap(), 2);
        assert_eq!(
            store.calendars().unwrap()[0].sync_token.as_deref(),
            Some("t1")
        );

        store
            .apply_sync(
                id,
                &SyncBatch {
                    deletes: vec!["/a.ics"],
                    sync_token: Some("t2"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            store.etags(id).unwrap().keys().collect::<Vec<_>>(),
            vec!["/b.ics"]
        );

        // Vollabgleich: b bleibt unverändert, c kommt neu, Rest entfällt
        let c = event("c", "20261111T070000Z", "20261111T080000Z", "");
        store
            .apply_sync(
                id,
                &SyncBatch {
                    upserts: vec![("/c.ics", "\"1\"", &c)],
                    keep: vec!["/b.ics"],
                    full: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let mut hrefs: Vec<String> = store.etags(id).unwrap().into_keys().collect();
        hrefs.sort();
        assert_eq!(hrefs, vec!["/b.ics", "/c.ics"]);
    }

    #[test]
    fn range_query_finds_overlapping_and_open_ended_series() {
        let (mut store, id) = store_with_calendar();
        let single = event("s", "20261109T070000Z", "20261109T080000Z", "");
        let past = event("p", "20250101T070000Z", "20250101T080000Z", "");
        let series = event(
            "r",
            "20200106T070000Z",
            "20200106T080000Z",
            "RRULE:FREQ=WEEKLY\r\n",
        );
        let todo = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t\r\nDUE:20261109T070000Z\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        store
            .apply_sync(
                id,
                &SyncBatch {
                    upserts: vec![
                        ("/s.ics", "1", &single),
                        ("/p.ics", "1", &past),
                        ("/r.ics", "1", &series),
                        ("/t.ics", "1", todo),
                        (
                            "/j.ics",
                            "1",
                            "BEGIN:VCALENDAR\r\nBEGIN:VJOURNAL\r\nUID:j\r\nDTSTART;VALUE=DATE:20261109\r\nEND:VJOURNAL\r\nEND:VCALENDAR\r\n",
                        ),
                    ],
                    ..Default::default()
                },
            )
            .unwrap();
        let mut found: Vec<String> = store
            .events_in_range(ts("2026-11-09 00:00"), ts("2026-11-16 00:00"))
            .unwrap()
            .into_iter()
            .map(|r| r.href)
            .collect();
        found.sort();
        assert_eq!(found, vec!["/r.ics", "/s.ics"]);

        // Für die Suche: alle Termine unabhängig vom Zeitraum, keine Aufgaben
        let all: Vec<String> = store
            .all_events()
            .unwrap()
            .into_iter()
            .map(|r| r.href)
            .collect();
        assert_eq!(all, vec!["/p.ics", "/r.ics", "/s.ics"]);
        let todos: Vec<String> = store.tasks().unwrap().into_iter().map(|r| r.href).collect();
        assert_eq!(todos, vec!["/t.ics"]);
        let notes: Vec<String> = store
            .journals()
            .unwrap()
            .into_iter()
            .map(|r| r.href)
            .collect();
        assert_eq!(notes, vec!["/j.ics"]);

        store.set_calendar_visible(id, false).unwrap();
        assert!(
            store
                .events_in_range(ts("2026-11-09 00:00"), ts("2026-11-16 00:00"))
                .unwrap()
                .is_empty()
        );
        assert!(store.all_events().unwrap().is_empty());
        assert!(store.journals().unwrap().is_empty());
    }

    #[test]
    fn settings_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.setting("demo_visible").unwrap(), None);
        store.set_setting("demo_visible", "0").unwrap();
        store.set_setting("demo_visible", "1").unwrap();
        assert_eq!(store.setting("demo_visible").unwrap().as_deref(), Some("1"));
    }

    #[test]
    fn reopening_a_file_keeps_data_and_schema() {
        let dir = std::env::temp_dir().join(format!("hora-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hora.db");
        {
            let store = Store::open(&path).unwrap();
            store.set_setting("x", "1").unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.setting("x").unwrap().as_deref(), Some("1"));
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
