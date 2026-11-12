//! App-Zeiten zählen: Ein eigener Thread mit eigener Datenbankverbindung
//! bekommt Vordergrundwechsel und Sperre vom Beobachter aus `hora_platform`,
//! zählt nach der Regel aus `hora_core::activity` und speichert die
//! Abschnitte. Er läuft nur, solange die App-Zeiten eingeschaltet und nicht
//! pausiert sind und mindestens eine Aktivität Programme hat; sonst kostet
//! die Funktion nichts.
//!
//! Ist kein gezähltes Programm vorn, schläft der Thread bis zum nächsten
//! Wechsel. Sonst wacht er auf, wenn die Pausengrenze erreicht wäre, in einer
//! Pause alle paar Sekunden, und wenigstens einmal pro Minute, um den
//! laufenden Abschnitt zu speichern: Nach einem Absturz oder Stromausfall
//! fehlt so höchstens rund eine Minute.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use hora_core::activity::{Front, Segment, Tracker};
use hora_platform::activity::{self as platform, ForegroundApp, WatchEvent, Watcher};
use hora_storage::{Activity, Store};

/// Spätestens so oft nachsehen, solange ein gezähltes Programm vorn ist
const MAX_WAIT: TimeDelta = TimeDelta::seconds(60);
/// Laufenden Abschnitt speichern, wenn er seit dem letzten Speichern um so
/// viel gewachsen ist
const SAVE_EVERY: TimeDelta = TimeDelta::seconds(30);

/// Welches Programm zu welcher Aktivität gehört
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// Dateiname der exe in Kleinbuchstaben → Aktivität und Pausengrenze
    apps: HashMap<String, (i64, TimeDelta)>,
}

impl Config {
    pub fn new(activities: &[Activity]) -> Config {
        let mut apps = HashMap::new();
        for activity in activities {
            let limit = TimeDelta::seconds(activity.idle_limit.max(1));
            for exe in &activity.apps {
                apps.insert(exe.to_lowercase(), (activity.id, limit));
            }
        }
        Config { apps }
    }

    /// Nichts zu zählen
    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    fn front(&self, app: &ForegroundApp) -> Option<Front> {
        let (id, limit) = self.apps.get(&app.file_name().to_lowercase())?;
        Some(Front::new(id.to_string(), *limit))
    }
}

/// Ein gespeicherter, noch laufender Abschnitt
#[derive(Debug)]
struct Saved {
    row: i64,
    key: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

impl Saved {
    fn is(&self, segment: &Segment) -> bool {
        self.key == segment.key && self.start == segment.start
    }
}

/// Zählen und Speichern ohne Thread und Fenster, damit es sich testen lässt
struct Counting {
    store: Store,
    config: Config,
    tracker: Tracker,
    /// Was zuletzt vorn war; bleibt während der Sperre gemerkt
    app: Option<ForegroundApp>,
    locked: bool,
    saved: Option<Saved>,
}

impl Counting {
    fn new(store: Store, config: Config) -> Counting {
        Counting {
            store,
            config,
            tracker: Tracker::new(),
            app: None,
            locked: false,
            saved: None,
        }
    }

    fn event(&mut self, event: WatchEvent) {
        match event {
            WatchEvent::Foreground(app) => self.app = app,
            WatchEvent::Locked => self.locked = true,
            WatchEvent::Unlocked => {
                self.locked = false;
                self.app = platform::foreground_app();
            }
        }
    }

    /// Zum Zeitpunkt `at` liegt die letzte Eingabe `idle` zurück
    fn observe(&mut self, at: DateTime<Utc>, idle: TimeDelta) {
        let front = match (&self.app, self.locked) {
            (Some(app), false) => self.config.front(app),
            _ => None,
        };
        if let Some(segment) = self.tracker.observe(at, front, idle) {
            self.save_closed(&segment);
        }
        if let Some(segment) = self.tracker.running() {
            self.save_running(&segment);
        }
    }

    fn finish(&mut self, at: DateTime<Utc>) {
        if let Some(segment) = self.tracker.finish(at) {
            self.save_closed(&segment);
        }
    }

    /// Wie lange bis zur nächsten Beobachtung; `None`: bis zum nächsten Ereignis
    fn wait(&self) -> Option<Duration> {
        let wait = self.tracker.next_check()?.min(MAX_WAIT);
        Some(wait.to_std().unwrap_or_default())
    }

    fn save_closed(&mut self, segment: &Segment) {
        match self.saved.take() {
            Some(saved) if saved.is(segment) => {
                if saved.end != segment.end {
                    self.set_end(saved.row, segment.end);
                }
            }
            _ => {
                self.insert(segment);
            }
        }
    }

    fn save_running(&mut self, segment: &Segment) {
        match &mut self.saved {
            Some(saved) if saved.is(segment) => {
                if segment.end - saved.end >= SAVE_EVERY {
                    let row = saved.row;
                    saved.end = segment.end;
                    self.set_end(row, segment.end);
                }
            }
            _ => {
                self.saved = self.insert(segment).map(|row| Saved {
                    row,
                    key: segment.key.clone(),
                    start: segment.start,
                    end: segment.end,
                });
            }
        }
    }

    fn insert(&self, segment: &Segment) -> Option<i64> {
        let id = segment.key.parse().ok()?;
        // Scheitert, wenn die Aktivität gerade gelöscht wurde: dann soll
        // ihre Zeit ohnehin weg sein
        self.store
            .add_activity_time(id, segment.start, segment.end)
            .map_err(|e| crate::log::write(&format!("App-Zeiten: {e}")))
            .ok()
    }

    fn set_end(&self, row: i64, end: DateTime<Utc>) {
        if let Err(e) = self.store.set_activity_time_end(row, end) {
            crate::log::write(&format!("App-Zeiten: {e}"));
        }
    }
}

enum Msg {
    Watch(WatchEvent),
    Config(Config),
    Stop,
}

/// Läuft, solange es ihn gibt; beim Fallenlassen wird der letzte Abschnitt
/// gespeichert
pub struct Counter {
    sender: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
}

impl Counter {
    pub fn start(db: PathBuf, config: Config) -> Counter {
        let (sender, receiver) = mpsc::channel();
        let watch = sender.clone();
        let thread = std::thread::Builder::new()
            .name("app-zeiten".into())
            .spawn(move || run(&db, config, watch, receiver))
            .map_err(|e| crate::log::write(&format!("App-Zeiten: {e}")))
            .ok();
        Counter { sender, thread }
    }

    /// Geänderte Aktivitäten übernehmen; ein laufender Abschnitt endet, wenn
    /// sein Programm nicht mehr dazugehört oder die Pausengrenze sich ändert
    pub fn configure(&self, config: Config) {
        let _ = self.sender.send(Msg::Config(config));
    }
}

impl Drop for Counter {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn idle() -> TimeDelta {
    TimeDelta::from_std(platform::idle_time()).unwrap_or_default()
}

fn run(db: &std::path::Path, config: Config, watch: Sender<Msg>, receiver: Receiver<Msg>) {
    let store = match Store::open(db) {
        Ok(store) => store,
        Err(e) => {
            crate::log::write(&format!("App-Zeiten: {e}"));
            return;
        }
    };
    // Meldet gleich zu Beginn, was vorn ist
    let watcher = match Watcher::new(false, move |event| {
        let _ = watch.send(Msg::Watch(event));
    }) {
        Ok(watcher) => watcher,
        Err(e) => {
            crate::log::write(&format!("App-Zeiten: {e}"));
            return;
        }
    };
    let mut counting = Counting::new(store, config);
    let mut due: Option<Instant> = None;
    loop {
        let msg = match due {
            Some(due) => receiver.recv_timeout(due.saturating_duration_since(Instant::now())),
            None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match msg {
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Msg::Watch(event)) => counting.event(event),
            Ok(Msg::Config(config)) => counting.config = config,
            Err(RecvTimeoutError::Timeout) => {}
        }
        counting.observe(Utc::now(), idle());
        due = counting.wait().map(|wait| Instant::now() + wait);
    }
    counting.finish(Utc::now());
    drop(watcher);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use hora_storage::ActivityDraft;

    fn t(seconds: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 12, 18, 0, 0).unwrap() + TimeDelta::seconds(seconds)
    }

    fn s(seconds: i64) -> TimeDelta {
        TimeDelta::seconds(seconds)
    }

    fn app(name: &str) -> Option<ForegroundApp> {
        Some(ForegroundApp {
            exe: PathBuf::from(format!(r"C:\Programme\{name}")),
            pid: 4242,
        })
    }

    /// Speicher mit der Aktivität "Schreiben" (writer.exe, notes.exe)
    fn counting() -> (Counting, i64) {
        let mut store = Store::open_in_memory().unwrap();
        let id = store
            .add_activity(&ActivityDraft {
                name: "Schreiben",
                color: "#4285f4",
                idle_limit: 60,
                apps: &["Writer.exe".to_string(), "notes.exe".to_string()],
            })
            .unwrap();
        let config = Config::new(&store.activities().unwrap());
        (Counting::new(store, config), id)
    }

    fn total(counting: &Counting, id: i64) -> TimeDelta {
        counting
            .store
            .activity_totals(t(-3600), t(3600))
            .unwrap()
            .get(&id)
            .copied()
            .unwrap_or_default()
    }

    #[test]
    fn programs_are_matched_by_file_name_in_any_case() {
        let (counting, id) = counting();
        let front = counting.config.front(&app("WRITER.EXE").unwrap()).unwrap();
        assert_eq!(front.key, id.to_string());
        assert_eq!(front.idle_limit, s(60));
        assert_eq!(counting.config.front(&app("browser.exe").unwrap()), None);
    }

    #[test]
    fn work_is_saved_while_it_runs_and_when_it_ends() {
        let (mut c, id) = counting();
        c.event(WatchEvent::Foreground(app("writer.exe")));
        c.observe(t(0), s(0));
        assert_eq!(total(&c, id), s(0));
        // Nach einer Minute Arbeit ist der Abschnitt schon gespeichert
        c.observe(t(60), s(1));
        assert_eq!(total(&c, id), s(60));
        // Kurz danach nicht jedes Mal neu geschrieben, aber am Ende genau
        c.observe(t(70), s(1));
        assert_eq!(total(&c, id), s(60));
        c.event(WatchEvent::Foreground(app("browser.exe")));
        c.observe(t(80), s(0));
        assert_eq!(total(&c, id), s(80));
    }

    #[test]
    fn programs_of_one_activity_count_as_one_stretch() {
        let (mut c, id) = counting();
        c.event(WatchEvent::Foreground(app("writer.exe")));
        c.observe(t(0), s(0));
        c.event(WatchEvent::Foreground(app("notes.exe")));
        c.observe(t(30), s(0));
        c.event(WatchEvent::Foreground(app("writer.exe")));
        c.observe(t(50), s(0));
        c.finish(t(90));
        assert_eq!(total(&c, id), s(90));
    }

    #[test]
    fn lock_ends_the_stretch_and_unlock_needs_new_input() {
        let (mut c, id) = counting();
        c.event(WatchEvent::Foreground(app("writer.exe")));
        c.observe(t(0), s(0));
        c.event(WatchEvent::Locked);
        c.observe(t(40), s(0));
        assert_eq!(total(&c, id), s(40));
        // Gesperrt zählt nichts, auch nicht beim Beenden
        c.finish(t(100));
        assert_eq!(total(&c, id), s(40));
    }

    #[test]
    fn removing_a_program_ends_its_stretch() {
        let (mut c, id) = counting();
        c.event(WatchEvent::Foreground(app("writer.exe")));
        c.observe(t(0), s(0));
        c.config = Config::default();
        c.observe(t(20), s(0));
        assert_eq!(total(&c, id), s(20));
        assert_eq!(c.wait(), None);
    }

    #[test]
    fn waits_at_most_a_minute_while_a_program_is_in_front() {
        let (mut c, _) = counting();
        assert_eq!(c.wait(), None);
        c.event(WatchEvent::Foreground(app("writer.exe")));
        c.observe(t(0), s(0));
        assert_eq!(c.wait(), Some(Duration::from_secs(60)));
        // Pause: kurz nachsehen, ob wieder gearbeitet wird
        c.observe(t(70), s(70));
        assert!(c.wait().unwrap() <= Duration::from_secs(5));
    }
}
