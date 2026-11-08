//! Installieren, Aktualisieren und Entfernen – nur für den angemeldeten
//! Benutzer (HKCU, %LOCALAPPDATA%), ohne Administratorrechte. Verhalten wie
//! der frühere NSIS-Installer.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hora_i18n::tr;
use hora_platform::{autostart, registry};

use crate::payload::Payload;
use crate::win;

pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Hora";
/// Anmeldung für Benachrichtigungen (siehe hora_platform::notify)
const AUMID_KEY: &str = r"Software\Classes\AppUserModelId\Hora.Kalender";
/// Einstellungen des Installers (Sprache), siehe hora_platform::locale
const SETTINGS_KEY: &str = hora_platform::locale::SETTINGS_KEY;
const AUTOSTART_NAME: &str = "Hora";
pub const EXE: &str = "hora.exe";
pub const UNINSTALLER: &str = "uninstall.exe";

/// Vorhandene Installation
#[derive(Debug, Clone)]
pub struct Installed {
    pub dir: PathBuf,
    pub version: String,
}

pub fn installed() -> Option<Installed> {
    let dir = PathBuf::from(registry::get_string(UNINSTALL_KEY, "InstallLocation")?);
    dir.join(EXE).is_file().then(|| Installed {
        version: registry::get_string(UNINSTALL_KEY, "DisplayVersion").unwrap_or_default(),
        dir,
    })
}

/// %LOCALAPPDATA%\Programs\Hora
pub fn default_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Programs")
        .join("Hora")
}

/// Gewählter Ordner; ein allgemeiner Ordner (z. B. D:\Apps) bekommt
/// "Hora" angehängt, wie bei anderen Installern
pub fn with_app_folder(chosen: &Path) -> PathBuf {
    let named = chosen
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("hora"));
    if named || chosen.join(EXE).is_file() {
        chosen.to_path_buf()
    } else {
        chosen.join("Hora")
    }
}

pub fn start_menu_link() -> Option<PathBuf> {
    Some(
        dirs::data_dir()?
            .join(r"Microsoft\Windows\Start Menu\Programs")
            .join("Hora.lnk"),
    )
}

pub fn desktop_link() -> Option<PathBuf> {
    Some(dirs::desktop_dir()?.join("Hora.lnk"))
}

/// Lokale Daten von Hora (Cache, Einstellungen, Protokoll)
pub fn data_dir() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("Hora"))
}

pub fn autostart_enabled() -> bool {
    autostart::current(AUTOSTART_NAME).is_some()
}

pub fn installer_language() -> Option<String> {
    hora_platform::locale::installer_language()
}

#[derive(Debug, Clone)]
pub struct Options {
    pub dir: PathBuf,
    pub autostart: bool,
    pub desktop: bool,
    /// Sprache für Hora; `None` lässt die bisherige Wahl
    pub language: Option<String>,
}

/// Fortschritt (0…1) und was gerade passiert
pub type Report<'a> = &'a dyn Fn(f32, String);

/// Läuft Hora aus `dir`, beenden und warten, bis die Datei frei ist
fn quit_running(dir: &Path) -> Result<(), String> {
    let exe = dir.join(EXE);
    if !exe.is_file() {
        return Ok(());
    }
    if let Ok(mut child) = std::process::Command::new(&exe).arg("--quit").spawn() {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            if child.try_wait().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    wait_writable(&exe, Duration::from_secs(8))
}

/// Eine laufende exe lässt sich unter Windows nicht zum Schreiben öffnen
fn wait_writable(file: &Path, timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match std::fs::OpenOptions::new().write(true).open(file) {
            Ok(_) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) if started.elapsed() < timeout => std::thread::sleep(Duration::from_millis(150)),
            Err(_) => {
                return Err(tr!(
                    "Hora is still running. Please quit it (tray icon → Quit) and try again."
                )
                .into());
            }
        }
    }
}

fn write_retrying(path: &Path, data: &[u8]) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match std::fs::write(path, data) {
            Ok(()) => return Ok(()),
            Err(_) if started.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(200))
            }
            Err(e) => return Err(tr!("Can't write {}: {}", path.display(), e)),
        }
    }
}

fn folder_size_kb(dir: &Path) -> u32 {
    fn walk(dir: &Path) -> u64 {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| {
                let p = e.path();
                if p.is_dir() {
                    walk(&p)
                } else {
                    e.metadata().map(|m| m.len()).unwrap_or(0)
                }
            })
            .sum()
    }
    u32::try_from(walk(dir) / 1024).unwrap_or(u32::MAX)
}

pub fn install(
    payload: &Payload,
    stub: &[u8],
    opts: &Options,
    report: Report,
) -> Result<(), String> {
    let dir = &opts.dir;
    report(0.02, tr!("Closing Hora…").into());
    quit_running(dir)?;

    std::fs::create_dir_all(dir)
        .map_err(|e| tr!("Can't create the folder {}: {}", dir.display(), e))?;
    let total = payload.total_size().max(1) as f32;
    let mut done = 0u64;
    for file in &payload.files {
        report(
            0.05 + 0.8 * done as f32 / total,
            tr!("Copying {}…", file.path),
        );
        let target = dir.join(file.path.replace('/', "\\"));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| tr!("Can't create the folder {}: {}", parent.display(), e))?;
        }
        write_retrying(&target, &file.data)?;
        done += file.data.len() as u64;
    }

    report(0.87, tr!("Creating the uninstaller…").into());
    write_retrying(&dir.join(UNINSTALLER), stub)?;

    report(0.91, tr!("Adding Hora to the Start menu…").into());
    let exe = dir.join(EXE);
    let description = tr!("Desktop calendar for Windows");
    if let Some(link) = start_menu_link() {
        win::create_shortcut(&link, &exe, description)
            .map_err(|e| tr!("Can't create the Start menu entry: {}", e))?;
    }
    if let Some(link) = desktop_link() {
        if opts.desktop {
            win::create_shortcut(&link, &exe, description)
                .map_err(|e| tr!("Can't create the desktop shortcut: {}", e))?;
        } else {
            let _ = std::fs::remove_file(&link);
        }
    }

    report(0.95, tr!("Registering Hora…").into());
    let uninstaller = dir.join(UNINSTALLER);
    let strings = [
        ("DisplayName", "Hora".to_string()),
        ("DisplayVersion", env!("CARGO_PKG_VERSION").to_string()),
        ("Publisher", "lenkr0".to_string()),
        ("DisplayIcon", exe.display().to_string()),
        ("InstallLocation", dir.display().to_string()),
        ("UninstallString", format!("\"{}\"", uninstaller.display())),
        (
            "QuietUninstallString",
            format!("\"{}\" /S", uninstaller.display()),
        ),
    ];
    for (name, value) in strings {
        registry::set_string(UNINSTALL_KEY, name, &value).map_err(|e| e.to_string())?;
    }
    win::set_dword(UNINSTALL_KEY, "NoModify", 1)?;
    win::set_dword(UNINSTALL_KEY, "NoRepair", 1)?;
    win::set_dword(UNINSTALL_KEY, "EstimatedSize", folder_size_kb(dir))?;
    if opts.autostart {
        autostart::enable(AUTOSTART_NAME, &exe, "--tray").map_err(|e| e.to_string())?;
    } else {
        autostart::disable(AUTOSTART_NAME).map_err(|e| e.to_string())?;
    }
    if let Some(code) = &opts.language {
        registry::set_string(SETTINGS_KEY, "Language", code).map_err(|e| e.to_string())?;
    }
    report(1.0, tr!("Done").into());
    Ok(())
}

/// Hora aus `dir` entfernen. `keep_self`: der laufende Deinstaller liegt in
/// `dir` und kann sich nicht selbst löschen (das erledigt danach
/// [`schedule_cleanup`]).
pub fn uninstall(
    dir: &Path,
    delete_data: bool,
    keep_self: bool,
    report: Report,
) -> Result<(), String> {
    report(0.05, tr!("Closing Hora…").into());
    quit_running(dir)?;

    report(0.3, tr!("Removing files…").into());
    let exe = dir.join(EXE);
    if exe.exists() {
        std::fs::remove_file(&exe).map_err(|e| tr!("Can't delete {}: {}", exe.display(), e))?;
    }
    for name in ["LICENSE", "NOTICE.md"] {
        let _ = std::fs::remove_file(dir.join(name));
    }
    let _ = std::fs::remove_dir_all(dir.join("THIRD_PARTY_LICENSES"));
    if !keep_self {
        let _ = std::fs::remove_file(dir.join(UNINSTALLER));
    }

    report(0.6, tr!("Removing shortcuts and registry entries…").into());
    for link in [start_menu_link(), desktop_link()].into_iter().flatten() {
        let _ = std::fs::remove_file(link);
    }
    let _ = autostart::disable(AUTOSTART_NAME);
    let _ = registry::delete_key(AUMID_KEY);
    let _ = registry::delete_key(SETTINGS_KEY);
    let _ = registry::delete_key(UNINSTALL_KEY);

    if delete_data && let Some(data) = data_dir() {
        report(0.8, tr!("Deleting local data…").into());
        let _ = std::fs::remove_dir_all(data);
    }
    // Leeren Ordner entfernen; ein fremder Inhalt bleibt
    let _ = std::fs::remove_dir(dir);
    report(1.0, tr!("Done").into());
    Ok(())
}

/// Nach dem Beenden den Deinstaller selbst und den dann leeren Ordner löschen
pub fn schedule_cleanup(dir: &Path) {
    let me = dir.join(UNINSTALLER);
    let script = format!(
        "ping 127.0.0.1 -n 3 > nul & del /f /q \"{}\" & rmdir \"{}\"",
        me.display(),
        dir.display()
    );
    let _ = win::run_cmd_hidden(&script);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chosen_folder_gets_hora_appended() {
        assert_eq!(
            with_app_folder(Path::new(r"D:\Apps")),
            PathBuf::from(r"D:\Apps\Hora")
        );
        assert_eq!(
            with_app_folder(Path::new(r"D:\Apps\Hora")),
            PathBuf::from(r"D:\Apps\Hora")
        );
        assert_eq!(
            with_app_folder(Path::new(r"D:\Apps\hora")),
            PathBuf::from(r"D:\Apps\hora")
        );
    }

    #[test]
    fn default_dir_is_per_user() {
        let dir = default_dir();
        assert!(dir.ends_with(r"Programs\Hora"), "{}", dir.display());
    }
}
