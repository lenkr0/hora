//! Plattform-Anbindung an Windows:
//!
//! - [`credentials`]: Passwörter im Windows-Anmeldeinformationsspeicher
//! - [`notify`]: Benachrichtigungen (Toasts) für Erinnerungen
//! - [`tray`]: Symbol im Infobereich mit Kontextmenü
//! - [`instance`]: nur eine Instanz je Datenordner, Fenster nach vorn holen
//! - [`autostart`]: Start mit Windows (HKCU, ohne Adminrechte)
//! - [`activity`]: Vordergrund-App und letzte Eingabe, für die App-Zeiten
//!
//! Unter anderen Systemen gibt es dieselben Funktionen als Platzhalter.

use std::time::Duration;

pub mod activity;
pub mod autostart;
pub mod credentials;
pub mod instance;
pub mod locale;
pub mod notify;
pub mod registry;
pub mod tray;

/// Fehler aus der Windows-Anbindung, als lesbarer Text
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct PlatformError(pub String);

/// Zeit seit Windows den Prozess erzeugt hat, also inklusive Laden der DLLs
/// und allem vor `main`. Ohne Windows `None`.
pub fn process_uptime() -> Option<Duration> {
    imp::process_uptime()
}

/// Nur Web- und Mail-Links öffnen, nie Programme oder Dateien
fn allowed_link(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["https://", "http://", "mailto:"]
        .iter()
        .any(|p| lower.starts_with(p))
        && !url.chars().any(char::is_control)
}

/// Fehlermeldung als Windows-Meldungsfenster (auch ohne Slint-Fenster)
pub fn error_box(title: &str, text: &str) {
    imp::error_box(title, text);
}

/// Rahmenloses Fenster wie ein normales aussehen lassen: runde Ecken unter
/// Windows 11 (ältere Versionen ignorieren das)
pub fn round_corners(hwnd: isize) {
    imp::round_corners(hwnd);
}

/// Link aus einem Journaleintrag im Standardbrowser öffnen
pub fn open_url(url: &str) -> Result<(), PlatformError> {
    if !allowed_link(url) {
        return Err(PlatformError(format!("Link wird nicht geöffnet: {url}")));
    }
    imp::open_url(url.trim())
}

#[cfg(windows)]
mod imp {
    use std::time::Duration;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW, SW_SHOWNORMAL,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn round_corners(hwnd: isize) {
        use windows_sys::Win32::Graphics::Dwm::{
            DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
        };
        let preference = DWMWCP_ROUND;
        // SAFETY: Zeiger auf einen lokalen Wert passender Größe
        unsafe {
            DwmSetWindowAttribute(
                hwnd as _,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                (&preference as *const i32).cast(),
                4,
            );
        }
    }

    pub fn error_box(title: &str, text: &str) {
        let (title, text) = (wide(title), wide(text));
        // SAFETY: Beide Zeichenketten sind nullterminiert und leben bis nach
        // dem Aufruf; ohne Besitzerfenster ist das Handle null.
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
            );
        }
    }

    pub fn open_url(url: &str) -> Result<(), super::PlatformError> {
        let (verb, target) = (wide("open"), wide(url));
        // SAFETY: Beide Zeichenketten sind nullterminiert und leben bis nach
        // dem Aufruf; die übrigen Zeiger dürfen laut Doku null sein.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        // Werte über 32 bedeuten Erfolg
        if result as isize > 32 {
            Ok(())
        } else {
            Err(super::PlatformError(format!(
                "Link konnte nicht geöffnet werden (Fehler {})",
                result as isize
            )))
        }
    }
    use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    /// FILETIME zählt in 100-ns-Schritten.
    fn ticks(ft: FILETIME) -> u64 {
        (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
    }

    pub fn process_uptime() -> Option<Duration> {
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user, mut now) =
            (zero, zero, zero, zero, zero);
        // SAFETY: Alle Zeiger verweisen auf gültige lokale FILETIME-Werte; das
        // Pseudo-Handle aus GetCurrentProcess muss nicht geschlossen werden.
        let ok = unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        };
        if ok == 0 {
            return None;
        }
        // SAFETY: `now` ist ein gültiger, beschreibbarer FILETIME-Wert.
        unsafe { GetSystemTimeAsFileTime(&mut now) };
        let diff = ticks(now).checked_sub(ticks(created))?;
        Some(Duration::from_nanos(diff * 100))
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn process_uptime() -> Option<std::time::Duration> {
        None
    }

    pub fn open_url(_url: &str) -> Result<(), super::PlatformError> {
        Err(super::PlatformError("nur unter Windows".into()))
    }

    pub fn error_box(title: &str, text: &str) {
        eprintln!("{title}: {text}");
    }

    pub fn round_corners(_hwnd: isize) {}
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_web_and_mail_links_are_opened() {
        assert!(super::allowed_link("https://example.com/a?b=c"));
        assert!(super::allowed_link("HTTP://x.de"));
        assert!(super::allowed_link("mailto:a@b.de"));
        assert!(!super::allowed_link("file:///C:/Windows/System32/cmd.exe"));
        assert!(!super::allowed_link("C:\\Windows\\notepad.exe"));
        assert!(!super::allowed_link("javascript:alert(1)"));
        assert!(!super::allowed_link("https://x.de\nfoo"));
    }

    #[test]
    #[cfg(windows)]
    fn uptime_is_positive_and_plausible() {
        let up = super::process_uptime().expect("GetProcessTimes");
        assert!(
            up.as_secs() < 3600,
            "Testprozess läuft nicht seit einer Stunde: {up:?}"
        );
    }
}
