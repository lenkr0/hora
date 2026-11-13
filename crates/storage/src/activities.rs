//! App-Zeiten: Aktivitäten mit ihren Programmen und die gezählte Zeit.
//!
//! Ein Programm gehört höchstens zu einer Aktivität (Dateiname der exe, ohne
//! Rücksicht auf Groß- und Kleinschreibung). Gezählte Zeit liegt als
//! Abschnitte in Millisekunden seit 1970 (UTC) vor; der laufende Abschnitt
//! wird beim Zählen regelmäßig verlängert, damit nach einem Absturz wenig
//! fehlt. Wird eine Aktivität gelöscht, verschwindet ihre Zeit mit.

use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};
use rusqlite::params;

use crate::{Result, Store};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub id: i64,
    pub name: String,
    /// Farbe als `#rrggbb`
    pub color: String,
    /// Pausengrenze in Sekunden: so lange ohne Eingabe, dann ruht die Zählung
    pub idle_limit: i64,
    /// Dateinamen der Programme, z. B. `notepad.exe`, sortiert
    pub apps: Vec<String>,
}

/// Neue oder geänderte Aktivität
#[derive(Debug, Clone)]
pub struct ActivityDraft<'a> {
    pub name: &'a str,
    pub color: &'a str,
    pub idle_limit: i64,
    /// Ist ein Programm schon einer anderen Aktivität zugeordnet, wechselt es
    pub apps: &'a [String],
}

fn ms(at: DateTime<Utc>) -> i64 {
    at.timestamp_millis()
}

fn from_ms(millis: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(millis).unwrap_or_default()
}

impl Store {
    pub fn activities(&self) -> Result<Vec<Activity>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, color, idle_limit FROM activities ORDER BY sort_order, id",
        )?;
        let mut activities = stmt
            .query_map([], |r| {
                Ok(Activity {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    color: r.get(2)?,
                    idle_limit: r.get(3)?,
                    apps: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = self
            .conn
            .prepare("SELECT activity_id, exe FROM activity_apps ORDER BY exe COLLATE NOCASE")?;
        let apps = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        for app in apps {
            let (id, exe) = app?;
            if let Some(activity) = activities.iter_mut().find(|a| a.id == id) {
                activity.apps.push(exe);
            }
        }
        Ok(activities)
    }

    pub fn add_activity(&mut self, draft: &ActivityDraft) -> Result<i64> {
        let tx = self.write()?;
        tx.execute(
            "INSERT INTO activities (name, color, idle_limit, sort_order)
             VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM activities))",
            params![draft.name, draft.color, draft.idle_limit],
        )?;
        let id = tx.last_insert_rowid();
        set_apps(&tx, id, draft.apps)?;
        tx.commit()?;
        Ok(id)
    }

    pub fn update_activity(&mut self, id: i64, draft: &ActivityDraft) -> Result<()> {
        let tx = self.write()?;
        tx.execute(
            "UPDATE activities SET name = ?2, color = ?3, idle_limit = ?4 WHERE id = ?1",
            params![id, draft.name, draft.color, draft.idle_limit],
        )?;
        tx.execute("DELETE FROM activity_apps WHERE activity_id = ?1", [id])?;
        set_apps(&tx, id, draft.apps)?;
        tx.commit()?;
        Ok(())
    }

    /// Löscht die Aktivität samt ihrer gezählten Zeit
    pub fn delete_activity(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM activities WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Gezählten Abschnitt anlegen; liefert seine Kennung zum Verlängern
    pub fn add_activity_time(
        &self,
        activity_id: i64,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO activity_time (activity_id, start_ms, end_ms) VALUES (?1, ?2, ?3)",
            params![activity_id, ms(start), ms(end)],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Ende eines gezählten Abschnitts neu setzen
    pub fn set_activity_time_end(&self, id: i64, end: DateTime<Utc>) -> Result<()> {
        self.conn.execute(
            "UPDATE activity_time SET end_ms = ?2 WHERE id = ?1",
            params![id, ms(end)],
        )?;
        Ok(())
    }

    /// Gezählte Zeit je Aktivität zwischen `from` und `to`; Abschnitte am
    /// Rand zählen nur mit ihrem Teil darin
    pub fn activity_totals(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<HashMap<i64, TimeDelta>> {
        let mut stmt = self.conn.prepare(
            "SELECT activity_id, SUM(MIN(end_ms, ?2) - MAX(start_ms, ?1)) FROM activity_time
             WHERE end_ms > ?1 AND start_ms < ?2 GROUP BY activity_id",
        )?;
        let rows = stmt.query_map(params![ms(from), ms(to)], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut totals = HashMap::new();
        for row in rows {
            let (id, millis) = row?;
            totals.insert(id, TimeDelta::milliseconds(millis));
        }
        Ok(totals)
    }

    /// Gezählte Abschnitte einer Aktivität, nach Beginn sortiert
    pub fn activity_spans(&self, activity_id: i64) -> Result<Vec<(DateTime<Utc>, DateTime<Utc>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT start_ms, end_ms FROM activity_time WHERE activity_id = ?1 ORDER BY start_ms",
        )?;
        let rows = stmt.query_map(params![activity_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut spans = Vec::new();
        for row in rows {
            let (start, end) = row?;
            spans.push((from_ms(start), from_ms(end)));
        }
        Ok(spans)
    }
}

fn set_apps(tx: &rusqlite::Transaction, id: i64, apps: &[String]) -> Result<()> {
    let mut stmt = tx.prepare(
        "INSERT INTO activity_apps (exe, activity_id) VALUES (?1, ?2)
         ON CONFLICT (exe) DO UPDATE SET exe = excluded.exe, activity_id = excluded.activity_id",
    )?;
    for app in apps {
        stmt.execute(params![app, id])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn t(minutes: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 12, 18, 0, 0).unwrap() + TimeDelta::minutes(minutes)
    }

    fn draft<'a>(name: &'a str, apps: &'a [String]) -> ActivityDraft<'a> {
        ActivityDraft {
            name,
            color: "#4285f4",
            idle_limit: 60,
            apps,
        }
    }

    fn apps(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn activities_keep_their_programs() {
        let mut store = Store::open_in_memory().unwrap();
        let writing = store
            .add_activity(&draft("Writing", &apps(&["writer.exe", "Notes.exe"])))
            .unwrap();
        let code = store
            .add_activity(&draft("Code", &apps(&["editor.exe"])))
            .unwrap();
        let all = store.activities().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, writing);
        assert_eq!(all[0].apps, apps(&["Notes.exe", "writer.exe"]));
        assert_eq!(all[1].id, code);
        assert_eq!(all[1].idle_limit, 60);

        store
            .update_activity(
                code,
                &ActivityDraft {
                    idle_limit: 120,
                    ..draft("Coding", &apps(&["editor.exe", "terminal.exe"]))
                },
            )
            .unwrap();
        let all = store.activities().unwrap();
        assert_eq!(all[1].name, "Coding");
        assert_eq!(all[1].idle_limit, 120);
        assert_eq!(all[1].apps, apps(&["editor.exe", "terminal.exe"]));
    }

    #[test]
    fn a_program_belongs_to_one_activity() {
        let mut store = Store::open_in_memory().unwrap();
        let first = store
            .add_activity(&draft("First", &apps(&["shared.exe", "one.exe"])))
            .unwrap();
        // Andere Schreibweise, gleiches Programm: wechselt zur neuen Aktivität
        let second = store
            .add_activity(&draft("Second", &apps(&["SHARED.EXE"])))
            .unwrap();
        let all = store.activities().unwrap();
        assert_eq!(all[0].id, first);
        assert_eq!(all[0].apps, apps(&["one.exe"]));
        assert_eq!(all[1].id, second);
        assert_eq!(all[1].apps, apps(&["SHARED.EXE"]));
    }

    #[test]
    fn totals_count_only_the_part_in_the_range() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_activity(&draft("A", &apps(&["a.exe"]))).unwrap();
        let b = store.add_activity(&draft("B", &apps(&["b.exe"]))).unwrap();
        // A: 10 Minuten, davon 5 vor dem Zeitraum; dann 20 Minuten darin
        store.add_activity_time(a, t(-5), t(5)).unwrap();
        let running = store.add_activity_time(a, t(30), t(35)).unwrap();
        store.set_activity_time_end(running, t(50)).unwrap();
        // B: ganz außerhalb
        store.add_activity_time(b, t(-30), t(-10)).unwrap();
        let totals = store.activity_totals(t(0), t(60)).unwrap();
        assert_eq!(totals.get(&a), Some(&TimeDelta::minutes(25)));
        assert_eq!(totals.get(&b), None);
        let all = store.activity_totals(t(-60), t(60)).unwrap();
        assert_eq!(all.get(&b), Some(&TimeDelta::minutes(20)));
        // Die Abschnitte selbst, nach Beginn sortiert
        assert_eq!(
            store.activity_spans(a).unwrap(),
            vec![(t(-5), t(5)), (t(30), t(50))]
        );
        assert_eq!(store.activity_spans(b).unwrap(), vec![(t(-30), t(-10))]);
    }

    #[test]
    fn deleting_an_activity_removes_its_time() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_activity(&draft("A", &apps(&["a.exe"]))).unwrap();
        store.add_activity_time(a, t(0), t(10)).unwrap();
        store.delete_activity(a).unwrap();
        assert!(store.activities().unwrap().is_empty());
        assert!(store.activity_totals(t(-60), t(60)).unwrap().is_empty());
        // Das Programm ist wieder frei
        let b = store.add_activity(&draft("B", &apps(&["a.exe"]))).unwrap();
        assert_eq!(store.activities().unwrap()[0].id, b);
    }
}
