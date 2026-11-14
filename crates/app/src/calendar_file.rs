//! Lokale Kalender als .ics-Datei sichern und wieder einlesen. Die Datei
//! lesen auch andere Kalender-Programme; eingelesene Einträge mit gleicher
//! UID ersetzen die vorhandenen, so entstehen beim erneuten Einlesen einer
//! Sicherung keine doppelten Termine.

use hora_core::bundle;
use hora_i18n::{tr, trn};
use hora_platform::files::{self, Filter};

use crate::{AppWindow, State, editing, shell};

/// Erlaubte Zeichen im Dateinamen eines Eintrags; sonst ein Hash der UID
fn file_stem(uid: &str) -> String {
    let safe = !uid.is_empty()
        && uid.len() <= 200
        && uid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'));
    if safe {
        return uid.to_string();
    }
    // FNV-1a, damit dieselbe UID immer denselben Namen bekommt
    let hash = uid.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    format!("import-{hash:016x}")
}

/// Kalendername als Dateiname: ohne Zeichen, die Windows nicht erlaubt
fn file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.');
    format!("{}.ics", if cleaned.is_empty() { "Hora" } else { cleaned })
}

fn filter() -> String {
    tr!("Calendar file").to_string()
}

fn local_name(state: &State, id: i64) -> Option<String> {
    state
        .calendars
        .iter()
        .find(|c| c.id == id && c.local)
        .map(|c| c.name.clone())
}

/// Alle Einträge des Kalenders in eine .ics-Datei
pub fn export(app: &AppWindow, state: &State, id: i64) {
    let Some(name) = local_name(state, id) else {
        return;
    };
    let label = filter();
    let filter = Filter {
        name: &format!("{label} (*.ics)"),
        pattern: "*.ics",
        extension: "ics",
    };
    let title = tr!("Export “{}”", name);
    let Some(path) = files::save_dialog(shell::hwnd(app), &title, &file_name(&name), &filter)
    else {
        return;
    };
    let resources = match state.store.calendar_resources(id) {
        Ok(resources) => resources,
        Err(e) => {
            app.set_add_account_error(e.to_string().into());
            return;
        }
    };
    let refs: Vec<&str> = resources.iter().map(String::as_str).collect();
    let (text, skipped) = bundle::merge(&refs, &name);
    if let Err(e) = std::fs::write(&path, text) {
        app.set_add_account_error(tr!("Could not save the file: {}", e).into());
        return;
    }
    let file = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let saved = resources.len() - skipped;
    let mut status = trn!(
        "{n} entry saved in {}",
        "{n} entries saved in {}",
        saved,
        file
    );
    if skipped > 0 {
        status.push_str(" · ");
        status.push_str(&trn!(
            "{n} could not be read",
            "{n} could not be read",
            skipped
        ));
    }
    app.set_add_account_error("".into());
    app.set_calendar_status(status.into());
}

/// Einträge aus einer .ics-Datei in den Kalender übernehmen
pub fn import(app: &AppWindow, state: &mut State, id: i64) -> bool {
    let Some(name) = local_name(state, id) else {
        return false;
    };
    let label = filter();
    let filter = Filter {
        name: &format!("{label} (*.ics)"),
        pattern: "*.ics",
        extension: "ics",
    };
    let title = tr!("Import into “{}”", name);
    let Some(path) = files::open_dialog(shell::hwnd(app), &title, &filter) else {
        return false;
    };
    let text = match std::fs::read(&path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) => {
            app.set_add_account_error(tr!("Could not read the file: {}", e).into());
            return false;
        }
    };
    let entries = match bundle::split(&text, editing::new_uid) {
        Ok(entries) => entries,
        Err(e) => {
            app.set_add_account_error(tr!("Could not read the file: {}", e).into());
            return false;
        }
    };
    let mut imported = 0;
    for entry in &entries {
        let stored = editing::new_href(state, id, &file_stem(&entry.uid)).and_then(|href| {
            state
                .store
                .local_put(id, &href, &entry.ics)
                .map_err(|e| e.to_string())
        });
        match stored {
            Ok(()) => imported += 1,
            Err(e) => crate::log::write(&format!("Import: {} nicht übernommen: {e}", entry.uid)),
        }
    }
    app.set_add_account_error("".into());
    app.set_calendar_status(trn!("{n} entry imported", "{n} entries imported", imported).into());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe_for_files_and_addresses() {
        assert_eq!(file_name("Privat"), "Privat.ics");
        assert_eq!(file_name("Arbeit: 2026/27?"), "Arbeit_ 2026_27_.ics");
        assert_eq!(file_name("  "), "Hora.ics");
        assert_eq!(file_stem("abc-123@example.org"), "abc-123@example.org");
        // Unsichere UID: immer derselbe Ersatzname
        let odd = "uid mit/Leerzeichen";
        assert_eq!(file_stem(odd), file_stem(odd));
        assert!(file_stem(odd).starts_with("import-"));
        assert_ne!(file_stem(odd), file_stem("uid mit/anderem"));
    }
}
