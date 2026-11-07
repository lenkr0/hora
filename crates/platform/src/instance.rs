//! Nur eine Hora-Instanz je Datenordner. Startet man Hora ein zweites Mal,
//! holt die neue Instanz das Fenster der laufenden nach vorn und beendet
//! sich. Der Name enthält den Datenordner, damit Testläufe mit eigenem
//! `HORA_DATA_DIR` neben dem richtigen Hora laufen können.

/// Fensterklasse des (unsichtbaren) Infobereich-Fensters; dessen Titel ist
/// der Instanzname, darüber findet die zweite Instanz die erste.
pub const WINDOW_CLASS: &str = "HoraTrayWindow";

/// Instanzname aus dem Datenordner (Groß-/Kleinschreibung egal)
pub fn name_for(data_dir: &std::path::Path) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    data_dir.to_string_lossy().to_lowercase().hash(&mut hasher);
    format!("Hora-{:016x}", hasher.finish())
}

#[cfg(windows)]
mod imp {
    use super::WINDOW_CLASS;
    use crate::registry::wide;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND,
    };
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, IsIconic, PostMessageW,
        SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
    };

    /// Hält die Instanz, solange es lebt
    pub struct Guard(HANDLE);

    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: Das Handle stammt aus CreateMutexW und wird nur hier geschlossen.
            unsafe { CloseHandle(self.0) };
        }
    }

    pub fn acquire(name: &str) -> Option<Guard> {
        let mutex = wide(&format!("Local\\{name}"));
        // SAFETY: Name ist ein nullterminierter Puffer; ohne Sicherheitsattribute.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, mutex.as_ptr()) };
        // SAFETY: direkt nach CreateMutexW gelesen
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if handle.is_null() {
            // Ohne Mutex lieber starten als gar nicht
            return Some(Guard(handle));
        }
        if exists {
            // SAFETY: gültiges Handle aus CreateMutexW
            unsafe { CloseHandle(handle) };
            return None;
        }
        Some(Guard(handle))
    }

    pub fn find(name: &str) -> Option<HWND> {
        let (class, title) = (wide(WINDOW_CLASS), wide(name));
        // SAFETY: nullterminierte Puffer
        let hwnd = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
        (!hwnd.is_null()).then_some(hwnd)
    }

    pub fn activate(name: &str, message: u32) -> bool {
        let Some(hwnd) = find(name) else {
            return false;
        };
        let mut pid = 0u32;
        // SAFETY: `hwnd` ist ein gefundenes Fenster, `pid` beschreibbar. Die
        // laufende Instanz darf danach ihr Fenster in den Vordergrund holen.
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
            AllowSetForegroundWindow(pid);
            PostMessageW(hwnd, message, 0, 0) != 0
        }
    }

    pub fn focus_window(hwnd: isize) {
        let hwnd = hwnd as HWND;
        // SAFETY: Das Fenster gehört diesem Prozess; ungültige Handles lassen
        // die Aufrufe nur fehlschlagen.
        unsafe {
            ShowWindow(
                hwnd,
                if IsIconic(hwnd) != 0 {
                    SW_RESTORE
                } else {
                    SW_SHOW
                },
            );
            SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(windows)]
pub use imp::Guard;

#[cfg(not(windows))]
pub struct Guard;

/// Diese Instanz ist die erste: `Some` (festhalten, solange Hora läuft).
/// Läuft schon eine, `None`.
pub fn acquire(name: &str) -> Option<Guard> {
    #[cfg(windows)]
    return imp::acquire(name);
    #[cfg(not(windows))]
    {
        let _ = name;
        Some(Guard)
    }
}

/// Die laufende Instanz bitten, ihr Fenster zu zeigen
pub fn activate_running(name: &str) -> bool {
    #[cfg(windows)]
    return imp::activate(name, crate::tray::WM_SHOW_REQUEST);
    #[cfg(not(windows))]
    {
        let _ = name;
        false
    }
}

/// Die laufende Instanz bitten, sich zu beenden (`hora --quit`)
pub fn request_quit(name: &str) -> bool {
    #[cfg(windows)]
    return imp::activate(name, crate::tray::WM_QUIT_REQUEST);
    #[cfg(not(windows))]
    {
        let _ = name;
        false
    }
}

/// Eigenes Fenster wiederherstellen und nach vorn holen (HWND als Zahl)
pub fn focus_window(hwnd: isize) {
    #[cfg(windows)]
    imp::focus_window(hwnd);
    #[cfg(not(windows))]
    let _ = hwnd;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_depend_on_the_data_dir() {
        let a = name_for(std::path::Path::new(r"C:\Users\A\AppData\Local\Hora"));
        assert_eq!(
            a,
            name_for(std::path::Path::new(r"c:\users\a\appdata\local\hora"))
        );
        assert_ne!(a, name_for(std::path::Path::new(r"C:\Temp\hora-test")));
        assert!(a.starts_with("Hora-"));
    }

    #[test]
    #[cfg(windows)]
    fn second_acquire_fails_while_the_first_lives() {
        let name = format!("Hora-Test-{}", std::process::id());
        let first = acquire(&name).expect("erste Instanz");
        assert!(acquire(&name).is_none());
        drop(first);
        assert!(acquire(&name).is_some());
    }
}
