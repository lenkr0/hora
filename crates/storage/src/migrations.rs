//! Schema-Migrationen über `PRAGMA user_version`. Neue Schritte nur anhängen,
//! nie bestehende ändern.

use rusqlite::Connection;

const STEPS: &[&str] = &[
    // 1: Konten, Kalender, Ressourcen, Einstellungen
    "CREATE TABLE accounts (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        url TEXT NOT NULL,
        username TEXT NOT NULL,
        credential_key TEXT NOT NULL UNIQUE,
        principal_url TEXT,
        home_url TEXT,
        created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
    );
    CREATE TABLE calendars (
        id INTEGER PRIMARY KEY,
        account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        href TEXT NOT NULL,
        name TEXT NOT NULL,
        server_color TEXT,
        local_color TEXT,
        visible INTEGER NOT NULL DEFAULT 1,
        read_only INTEGER NOT NULL DEFAULT 0,
        components TEXT NOT NULL DEFAULT '',
        sort_order INTEGER NOT NULL DEFAULT 0,
        ctag TEXT,
        sync_token TEXT,
        last_sync INTEGER,
        UNIQUE (account_id, href)
    );
    CREATE TABLE resources (
        id INTEGER PRIMARY KEY,
        calendar_id INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
        href TEXT NOT NULL,
        etag TEXT NOT NULL,
        ics TEXT NOT NULL,
        kind TEXT,
        uid TEXT,
        start_utc INTEGER,
        end_utc INTEGER,
        UNIQUE (calendar_id, href)
    );
    CREATE INDEX resources_range ON resources (kind, start_utc, end_utc);
    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );",
    // 2: Änderungs-Warteschlange für Offline-Bearbeitung
    "CREATE TABLE pending (
        id INTEGER PRIMARY KEY,
        calendar_id INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
        href TEXT NOT NULL,
        op TEXT NOT NULL,
        ics TEXT,
        if_match TEXT,
        state TEXT NOT NULL DEFAULT 'pending',
        attempts INTEGER NOT NULL DEFAULT 0,
        next_attempt INTEGER NOT NULL DEFAULT 0,
        last_error TEXT,
        created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
        UNIQUE (calendar_id, href)
    );",
    // 3: schon gezeigte Erinnerungen
    "CREATE TABLE fired_reminders (
        key TEXT PRIMARY KEY,
        fired_at INTEGER NOT NULL
    );",
    // 4: App-Zeiten (Aktivitäten, ihre Programme, gezählte Zeit)
    "CREATE TABLE activities (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        color TEXT NOT NULL,
        idle_limit INTEGER NOT NULL DEFAULT 60,
        sort_order INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
    );
    CREATE TABLE activity_apps (
        exe TEXT PRIMARY KEY COLLATE NOCASE,
        activity_id INTEGER NOT NULL REFERENCES activities(id) ON DELETE CASCADE
    );
    CREATE INDEX activity_apps_activity ON activity_apps (activity_id);
    CREATE TABLE activity_time (
        id INTEGER PRIMARY KEY,
        activity_id INTEGER NOT NULL REFERENCES activities(id) ON DELETE CASCADE,
        start_ms INTEGER NOT NULL,
        end_ms INTEGER NOT NULL
    );
    CREATE INDEX activity_time_activity ON activity_time (activity_id, start_ms);",
    // App-Zeiten: Wochenziel in Minuten (0 = keins) und Termine, die zu einer
    // Aktivität gehören (nur auf diesem PC, nach UID, gilt für ganze Serien)
    "ALTER TABLE activities ADD COLUMN weekly_goal INTEGER NOT NULL DEFAULT 0;
    CREATE TABLE activity_events (
        uid TEXT PRIMARY KEY,
        activity_id INTEGER NOT NULL REFERENCES activities(id) ON DELETE CASCADE
    );",
];

pub fn run(conn: &Connection) -> rusqlite::Result<()> {
    let current: usize =
        conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? as usize;
    for (index, step) in STEPS.iter().enumerate().skip(current) {
        conn.execute_batch(&format!(
            "BEGIN; {step} PRAGMA user_version = {}; COMMIT;",
            index + 1
        ))?;
    }
    Ok(())
}
