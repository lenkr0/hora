//! Updates in der App: einmal am Tag im Hintergrund nachsehen, ein neueres
//! Setup laden und prüfen (crates/update). Installiert wird erst auf Klick
//! (Titelleiste oder Einstellungen): Hora beendet sich, das Setup
//! aktualisiert still und startet Hora wieder (`/S --relaunch`).
//!
//! Für Tests: `HORA_UPDATE_URL` statt der Releases auf GitHub (dann auch
//! ohne Installation), `HORA_UPDATE_DELAY` Sekunden bis zur ersten Prüfung.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::Timelike;
use hora_i18n::{dates, tr};
use hora_update::{Manifest, Source, Version};

use crate::{AppWindow, State, with_ui};

/// So alt darf die letzte erfolgreiche Prüfung werden
const INTERVAL: Duration = Duration::from_secs(20 * 60 * 60);
/// Erste Prüfung nach dem Start, wenn Hora sich eingerichtet hat
const FIRST_DELAY: Duration = Duration::from_secs(30);
/// So oft wird nachgesehen, ob eine Prüfung fällig ist
const TICK: Duration = Duration::from_secs(60 * 60);
/// Eintrag des Installers (crates/setup/src/install.rs)
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Hora";

pub enum Status {
    /// In dieser Sitzung noch nicht geprüft
    Idle,
    /// Hora läuft nicht aus seiner Installation (Entwicklung, kopierte exe)
    NotInstalled,
    Checking,
    Downloading {
        version: Version,
        percent: u32,
    },
    UpToDate,
    Ready {
        manifest: Manifest,
        path: PathBuf,
    },
    Failed(String),
}

pub struct Updates {
    pub status: Status,
    /// Download-Ordner im Datenordner
    dir: PathBuf,
    base: String,
    timer: slint::Timer,
    /// Setup, das nach dem Ende der Ereignisschleife startet
    install_on_exit: Option<PathBuf>,
}

enum Event {
    Downloading(Version, u32),
    Done(Result<Option<(Manifest, PathBuf)>, hora_update::Error>),
}

pub fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version(0, 0, 0))
}

impl Updates {
    pub fn new(data_dir: &Path) -> Updates {
        let base = std::env::var("HORA_UPDATE_URL").ok();
        let possible = base.is_some() || installed_here();
        Updates {
            status: if possible {
                Status::Idle
            } else {
                Status::NotInstalled
            },
            dir: data_dir.join("updates"),
            base: base.unwrap_or_else(|| hora_update::RELEASES.to_string()),
            timer: slint::Timer::default(),
            install_on_exit: None,
        }
    }

    fn busy(&self) -> bool {
        matches!(self.status, Status::Checking | Status::Downloading { .. })
    }

    fn ready(&self) -> Option<Version> {
        match &self.status {
            Status::Ready { manifest, .. } => Some(manifest.version),
            _ => None,
        }
    }
}

/// Läuft diese exe aus dem Ordner, den Hora Setup eingetragen hat?
fn installed_here() -> bool {
    let Some(dir) = hora_platform::registry::get_string(UNINSTALL_KEY, "InstallLocation") else {
        return false;
    };
    // Vergleich nach canonicalize: Kurznamen (RUNNER~1) und Schreibweise egal
    let here = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .and_then(|p| std::fs::canonicalize(p).ok());
    here.is_some() && here == std::fs::canonicalize(dir).ok()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Ist eine Prüfung fällig? (Auch wenn die Uhr zurückgestellt wurde)
fn due(last: Option<u64>, now: u64) -> bool {
    last.is_none_or(|t| now < t || now - t >= INTERVAL.as_secs())
}

fn last_check(state: &State) -> Option<u64> {
    state
        .store
        .setting("update_checked")
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
}

/// Beim Start: alte Setups löschen, Anzeige setzen, Prüfung planen
pub fn start(app: &AppWindow, state: &mut State) {
    hora_update::clean(&state.updates.dir, current());
    show(app, state);
    if matches!(state.updates.status, Status::NotInstalled) {
        return;
    }
    let delay = std::env::var("HORA_UPDATE_DELAY")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(FIRST_DELAY, Duration::from_secs);
    slint::Timer::single_shot(delay, || {
        with_ui(|app, state| tick(app, &mut state.borrow_mut()));
    });
    state
        .updates
        .timer
        .start(slint::TimerMode::Repeated, TICK, || {
            with_ui(|app, state| tick(app, &mut state.borrow_mut()));
        });
}

/// Automatisch prüfen, wenn eingeschaltet und fällig
pub fn tick(app: &AppWindow, state: &mut State) {
    let updates = &state.updates;
    if state.settings.update_check
        && !matches!(updates.status, Status::NotInstalled)
        && !updates.busy()
        && updates.ready().is_none()
        && due(last_check(state), now())
    {
        check(app, state);
    }
}

/// Jetzt prüfen und ein neueres Setup gleich laden
pub fn check(app: &AppWindow, state: &mut State) {
    if matches!(state.updates.status, Status::NotInstalled) || state.updates.busy() {
        return;
    }
    state.updates.status = Status::Checking;
    show(app, state);
    let source = Source::new(&state.updates.base, hora_update::PUBLIC_KEY);
    let dir = state.updates.dir.clone();
    std::thread::spawn(move || {
        let result = (|| {
            let manifest = source.latest()?;
            if manifest.version <= current() {
                return Ok(None);
            }
            let (version, size) = (manifest.version, manifest.size.max(1));
            let mut shown = u32::MAX;
            let path = source.download(&manifest, &dir, &mut |done| {
                let percent = u32::try_from(done * 100 / size).unwrap_or(100);
                if percent != shown {
                    shown = percent;
                    post(Event::Downloading(version, percent));
                }
            })?;
            Ok(Some((manifest, path)))
        })();
        post(Event::Done(result));
    });
}

fn post(event: Event) {
    let _ = slint::invoke_from_event_loop(move || {
        with_ui(|app, state| apply(app, &mut state.borrow_mut(), event));
    });
}

fn apply(app: &AppWindow, state: &mut State, event: Event) {
    let checked = match event {
        Event::Downloading(version, percent) => {
            state.updates.status = Status::Downloading { version, percent };
            false
        }
        Event::Done(Ok(None)) => {
            state.updates.status = Status::UpToDate;
            true
        }
        Event::Done(Ok(Some((manifest, path)))) => {
            state.updates.status = Status::Ready { manifest, path };
            true
        }
        // Nicht gespeichert: die nächste Stunde versucht es noch einmal
        Event::Done(Err(e)) => {
            state.updates.status = Status::Failed(e.to_string());
            false
        }
    };
    if checked {
        let _ = state
            .store
            .set_setting("update_checked", &now().to_string());
    }
    show(app, state);
}

/// Hinweis in der Titelleiste und Zeile in den Einstellungen
pub fn show(app: &AppWindow, state: &State) {
    let updates = &state.updates;
    app.set_update_version(
        updates
            .ready()
            .map(|v| v.to_string())
            .unwrap_or_default()
            .into(),
    );
    app.set_update_busy(updates.busy());
    app.set_update_possible(!matches!(updates.status, Status::NotInstalled));
    app.set_update_status(status_text(state).into());
}

fn status_text(state: &State) -> String {
    match &state.updates.status {
        Status::NotInstalled => {
            tr!("This copy of Hora wasn't installed with Hora Setup, so it can't update itself.")
                .to_string()
        }
        Status::Idle => {
            match last_check(state).and_then(|t| chrono::DateTime::from_timestamp(t as i64, 0)) {
                Some(at) => {
                    let local = at.with_timezone(&state.home).naive_local();
                    tr!(
                        "Last checked: {}, {}",
                        dates::numeric(local.date()),
                        dates::clock(local.hour(), local.minute())
                    )
                }
                None => tr!("Hora looks for new versions on GitHub.").to_string(),
            }
        }
        Status::Checking => tr!("Looking for updates…").to_string(),
        Status::Downloading { version, percent } => {
            tr!("Downloading Hora {}… {}%", version, percent)
        }
        Status::UpToDate => tr!("Hora is up to date.").to_string(),
        Status::Ready { manifest, .. } => {
            tr!(
                "Hora {} is ready. It installs when Hora restarts.",
                manifest.version
            )
        }
        Status::Failed(e) => tr!("Couldn't check for updates: {}", e),
    }
}

/// Klick auf „Neu starten“: Datei noch einmal prüfen, dann Hora beenden;
/// das Setup startet erst, wenn alles gesichert ist ([`run_pending`])
pub fn install(app: &AppWindow, state: &mut State) {
    let Status::Ready { manifest, path } = &state.updates.status else {
        return;
    };
    if let Err(e) = hora_update::check_file(path, manifest) {
        let _ = std::fs::remove_file(path);
        state.updates.status = Status::Failed(e.to_string());
        show(app, state);
        return;
    }
    state.updates.install_on_exit = Some(path.clone());
    let _ = slint::quit_event_loop();
}

/// Nach dem Ende der Ereignisschleife: Setup still starten. Es wartet, bis
/// diese Instanz beendet ist, aktualisiert und startet Hora neu.
pub fn run_pending(updates: &Updates) {
    let Some(path) = &updates.install_on_exit else {
        return;
    };
    let mut command = std::process::Command::new(path);
    command.args(["/S", "--relaunch"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        command.creation_flags(DETACHED_PROCESS);
    }
    if let Err(e) = command.spawn() {
        crate::log::write(&format!(
            "Update: Setup {} startet nicht: {e}",
            path.display()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_once_a_day_and_after_clock_changes() {
        let hour = 60 * 60;
        let now = 1_800_000_000;
        assert!(due(None, now));
        assert!(!due(Some(now - hour), now));
        assert!(!due(Some(now - 19 * hour), now));
        assert!(due(Some(now - 20 * hour), now));
        // Uhr zurückgestellt: lieber prüfen als tagelang warten
        assert!(due(Some(now + hour), now));
    }

    #[test]
    fn current_version_is_the_crate_version() {
        assert_eq!(current().to_string(), env!("CARGO_PKG_VERSION"));
    }
}
