//! Hora im Infobereich: Symbol mit Menü, Fenster ausblenden und zurückholen,
//! Autostart und Windows-Benachrichtigungen.

use hora_i18n::tr;
use std::path::{Path, PathBuf};

use hora_platform::notify::{AUMID, Notifier, Toast};
use hora_platform::tray::{MenuItem, Tray, TrayEvent};
use hora_platform::{autostart, instance};
use slint::ComponentHandle;

use crate::AppWindow;

/// Name des Autostart-Eintrags
pub const AUTOSTART_NAME: &str = "Hora";
/// Startet ohne Fenster im Infobereich (Autostart)
pub const TRAY_ARG: &str = "--tray";

pub const MENU_OPEN: u32 = 1;
pub const MENU_SYNC: u32 = 2;
pub const MENU_SETTINGS: u32 = 3;
pub const MENU_QUIT: u32 = 4;

/// Symbol für Benachrichtigungen und Infocenter
const ICON_PNG: &[u8] = include_bytes!("../assets/hora.png");

/// Menü im Infobereich, in der aktuellen Sprache
fn tray_menu() -> Vec<MenuItem> {
    vec![
        MenuItem {
            default: true,
            ..MenuItem::new(MENU_OPEN, tr!("Open Hora"))
        },
        MenuItem::new(MENU_SYNC, tr!("Sync now")),
        MenuItem::new(MENU_SETTINGS, tr!("Settings…")),
        MenuItem::separator(),
        MenuItem::new(MENU_QUIT, tr!("Quit")),
    ]
}

/// Benachrichtigungen und Infobereich; fehlt etwas, läuft Hora ohne weiter
pub struct Shell {
    pub notifier: Option<Notifier>,
    /// Warum es keine Benachrichtigungen gibt
    pub notifier_error: Option<String>,
    pub tray: Option<Tray>,
}

impl Shell {
    pub fn start(
        instance_name: &str,
        data_dir: &Path,
        on_tray: impl Fn(TrayEvent) + Send + 'static,
    ) -> Shell {
        let tray = Tray::new(instance_name, "Hora", tray_menu(), on_tray)
            .map_err(|e| crate::log::write(&format!("Infobereich: {e}")))
            .ok();
        let (notifier, notifier_error) =
            match Notifier::new(AUMID, "Hora", icon_file(data_dir).as_deref()) {
                Ok(n) => (Some(n), None),
                Err(e) => (None, Some(tr!("Notifications not available: {}", e))),
            };
        Shell {
            notifier,
            notifier_error,
            tray,
        }
    }

    /// Hinweis für die Einstellungen, wenn Benachrichtigungen nicht ankommen
    /// Texte nach einem Sprachwechsel erneuern
    pub fn relabel(&self) {
        if let Some(tray) = &self.tray {
            tray.set_menu(tray_menu());
        }
    }

    pub fn status(&self) -> String {
        match &self.notifier {
            Some(n) => n.blocked().unwrap_or_default(),
            None => self.notifier_error.clone().unwrap_or_default(),
        }
    }

    pub fn show(
        &self,
        toast: &Toast,
        on_click: impl Fn(String) + Send + 'static,
    ) -> Result<(), String> {
        match &self.notifier {
            Some(n) => n.show(toast, on_click).map_err(|e| e.to_string()),
            None => Err(self.notifier_error.clone().unwrap_or_default()),
        }
    }
}

/// Symbol als Datei im Datenordner, damit Windows es anzeigen kann
fn icon_file(dir: &Path) -> Option<PathBuf> {
    let path = dir.join("hora.png");
    if std::fs::read(&path).ok().as_deref() != Some(ICON_PNG) {
        std::fs::write(&path, ICON_PNG).ok()?;
    }
    Some(path)
}

/// Fenster zeigen, wiederherstellen und nach vorn holen
pub fn show_window(app: &AppWindow) {
    let _ = app.show();
    if let Some(hwnd) = hwnd(app) {
        hora_platform::round_corners(hwnd);
        instance::focus_window(hwnd);
    }
}

pub fn hwnd(app: &AppWindow) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = app.window().window_handle();
    match handle.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

/// Autostart an (für das laufende Programm) oder aus
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    if !enabled {
        return autostart::disable(AUTOSTART_NAME).map_err(|e| e.to_string());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    autostart::enable(AUTOSTART_NAME, &exe, TRAY_ARG).map_err(|e| e.to_string())
}

pub fn autostart_enabled() -> bool {
    autostart::current(AUTOSTART_NAME).is_some()
}

/// Zeigt der Autostart auf ein Programm, das es nicht mehr gibt (Hora wurde
/// verschoben), auf das laufende umstellen. Ein Testlauf aus einem anderen
/// Ordner lässt einen gültigen Eintrag in Ruhe.
pub fn refresh_autostart() {
    let (Some(current), Ok(exe)) = (autostart::current(AUTOSTART_NAME), std::env::current_exe())
    else {
        return;
    };
    let target = current
        .trim_start_matches('"')
        .split('"')
        .next()
        .unwrap_or_default();
    if !Path::new(target).exists() {
        let _ = autostart::enable(AUTOSTART_NAME, &exe, TRAY_ARG);
    }
}
