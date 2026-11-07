//! Windows-Anzeigesprache und Regionsformate (Datum, Uhrzeit) sowie die
//! Sprache, die der Installer gewählt hat.

/// Registry-Schlüssel (unter HKCU), in den der Installer die Sprache schreibt
pub const SETTINGS_KEY: &str = "Software\\Hora";

/// Anzeigesprache von Windows, z. B. "de-DE"
pub fn ui_language() -> Option<String> {
    imp::ui_language()
}

/// Kurzes Datums- und Zeitmuster der Region, z. B. ("dd.MM.yyyy", "HH:mm")
pub fn region_formats() -> Option<(String, String)> {
    imp::region_formats()
}

/// Vom Installer gewählte Sprache ("en", "de", … oder "system")
pub fn installer_language() -> Option<String> {
    crate::registry::get_string(SETTINGS_KEY, "Language").filter(|s| !s.trim().is_empty())
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Globalization::{
        GetLocaleInfoEx, GetUserDefaultUILanguage, LCIDToLocaleName, LOCALE_SSHORTDATE,
        LOCALE_SSHORTTIME,
    };

    fn from_wide(buf: &[u16], len: i32) -> Option<String> {
        // Länge enthält die abschließende Null
        let len = usize::try_from(len).ok()?.checked_sub(1)?;
        Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
    }

    pub fn ui_language() -> Option<String> {
        let mut buf = [0u16; 85];
        // SAFETY: Puffer mit angegebener Länge; LCIDToLocaleName schreibt höchstens so viel
        let len = unsafe {
            LCIDToLocaleName(
                u32::from(GetUserDefaultUILanguage()),
                buf.as_mut_ptr(),
                buf.len() as i32,
                0,
            )
        };
        from_wide(&buf, len)
    }

    fn info(kind: u32) -> Option<String> {
        let mut buf = [0u16; 80];
        // SAFETY: NULL steht für die Region des Benutzers; Puffer mit Länge
        let len =
            unsafe { GetLocaleInfoEx(std::ptr::null(), kind, buf.as_mut_ptr(), buf.len() as i32) };
        from_wide(&buf, len)
    }

    pub fn region_formats() -> Option<(String, String)> {
        Some((info(LOCALE_SSHORTDATE)?, info(LOCALE_SSHORTTIME)?))
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn ui_language() -> Option<String> {
        None
    }

    pub fn region_formats() -> Option<(String, String)> {
        None
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn windows_reports_language_and_formats() {
        let lang = super::ui_language().expect("Anzeigesprache");
        assert!(lang.len() >= 2, "{lang}");
        let (date, time) = super::region_formats().expect("Regionsformate");
        assert!(date.contains('d') && date.contains('M'), "{date}");
        assert!(time.contains('m'), "{time}");
    }
}
