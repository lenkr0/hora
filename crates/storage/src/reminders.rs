//! Schon gezeigte Erinnerungen, damit nach einem Neustart oder Sync keine
//! doppelt kommt. Der Schlüssel enthält Termin, Vorkommen, Alarm und
//! Auslösezeit: Wird ein Termin verschoben, kommt seine Erinnerung neu.

use rusqlite::{OptionalExtension, params};

use crate::{Result, Store};

impl Store {
    pub fn reminder_shown(&self, key: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM fired_reminders WHERE key = ?1",
                [key],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Als gezeigt merken (`at` = Auslösezeit in Sekunden seit 1970)
    pub fn mark_reminder_shown(&self, key: &str, at: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO fired_reminders (key, fired_at) VALUES (?1, ?2)",
            params![key, at],
        )?;
        Ok(())
    }

    /// Einträge mit Auslösezeit vor `before` vergessen
    pub fn prune_reminders(&self, before: i64) -> Result<usize> {
        Ok(self
            .conn
            .execute("DELETE FROM fired_reminders WHERE fired_at < ?1", [before])?)
    }
}

#[cfg(test)]
mod tests {
    use crate::Store;

    #[test]
    fn shown_reminders_are_remembered_and_pruned() {
        let store = Store::open_in_memory().unwrap();
        assert!(!store.reminder_shown("a").unwrap());
        store.mark_reminder_shown("a", 100).unwrap();
        store.mark_reminder_shown("b", 300).unwrap();
        assert!(store.reminder_shown("a").unwrap());
        assert_eq!(store.prune_reminders(200).unwrap(), 1);
        assert!(!store.reminder_shown("a").unwrap());
        assert!(store.reminder_shown("b").unwrap());
    }
}
