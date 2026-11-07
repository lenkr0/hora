//! Zeichenketten unter `HKEY_CURRENT_USER` lesen und schreiben (Autostart,
//! Anmeldung der App für Benachrichtigungen). Nie unter HKLM, Hora braucht
//! keine Administratorrechte.

use crate::PlatformError;

#[cfg(windows)]
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[cfg(windows)]
mod imp {
    use super::wide;
    use crate::PlatformError;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW,
        RegSetKeyValueW,
    };

    fn check(code: u32, what: &str) -> Result<(), PlatformError> {
        if code == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(PlatformError(format!("{what}: Fehler {code}")))
        }
    }

    pub fn set_string(key: &str, name: &str, value: &str) -> Result<(), PlatformError> {
        let data = wide(value);
        // SAFETY: Alle Zeiger zeigen auf nullterminierte UTF-16-Puffer, die
        // während des Aufrufs leben; `cbdata` ist ihre Länge in Bytes.
        let code = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                wide(key).as_ptr(),
                wide(name).as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        check(code, "Registry schreiben")
    }

    pub fn get_string(key: &str, name: &str) -> Option<String> {
        let (key, name) = (wide(key), wide(name));
        let mut size = 0u32;
        // SAFETY: Erster Aufruf nur mit Größenzeiger, um die Länge zu erfahren.
        let code = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if code != ERROR_SUCCESS {
            return None;
        }
        let mut buffer = vec![0u16; size as usize / 2 + 1];
        size = (buffer.len() * 2) as u32;
        // SAFETY: `buffer` ist `size` Bytes groß und beschreibbar.
        let code = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if code != ERROR_SUCCESS {
            return None;
        }
        let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..len]))
    }

    pub fn delete_value(key: &str, name: &str) -> Result<(), PlatformError> {
        // SAFETY: nullterminierte Puffer, die während des Aufrufs leben.
        let code = unsafe {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(name).as_ptr())
        };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(code, "Registry löschen")
    }

    pub fn delete_key(key: &str) -> Result<(), PlatformError> {
        // SAFETY: nullterminierter Puffer, der während des Aufrufs lebt.
        let code = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wide(key).as_ptr()) };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(code, "Registry-Schlüssel löschen")
    }
}

#[cfg(not(windows))]
mod imp {
    use crate::PlatformError;

    pub fn set_string(_: &str, _: &str, _: &str) -> Result<(), PlatformError> {
        Err(PlatformError("nur unter Windows".into()))
    }
    pub fn get_string(_: &str, _: &str) -> Option<String> {
        None
    }
    pub fn delete_value(_: &str, _: &str) -> Result<(), PlatformError> {
        Ok(())
    }
    pub fn delete_key(_: &str) -> Result<(), PlatformError> {
        Ok(())
    }
}

/// Wert `name` im Schlüssel `key` (relativ zu HKCU) setzen; der Schlüssel
/// entsteht bei Bedarf.
pub fn set_string(key: &str, name: &str, value: &str) -> Result<(), PlatformError> {
    imp::set_string(key, name, value)
}

pub fn get_string(key: &str, name: &str) -> Option<String> {
    imp::get_string(key, name)
}

/// Wert löschen; fehlt er schon, ist das kein Fehler.
pub fn delete_value(key: &str, name: &str) -> Result<(), PlatformError> {
    imp::delete_value(key, name)
}

/// Schlüssel samt Inhalt löschen; fehlt er schon, ist das kein Fehler.
pub fn delete_key(key: &str) -> Result<(), PlatformError> {
    imp::delete_key(key)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "schreibt in HKCU (eigener Testschlüssel, wird wieder gelöscht)"]
    fn strings_roundtrip_in_a_test_key() {
        let key = format!(r"Software\Hora-Test-{}", std::process::id());
        set_string(&key, "Wert", "C:\\Programme\\Hora\\hora.exe --tray äöü").unwrap();
        assert_eq!(
            get_string(&key, "Wert").as_deref(),
            Some("C:\\Programme\\Hora\\hora.exe --tray äöü")
        );
        delete_value(&key, "Wert").unwrap();
        assert_eq!(get_string(&key, "Wert"), None);
        delete_value(&key, "Wert").unwrap();
        delete_key(&key).unwrap();
    }
}
