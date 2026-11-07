//! Protokoll `hora.log` im Datenordner: Abstürze und Fehler, die sonst
//! nirgends sichtbar wären (der Release-Build hat kein Konsolenfenster).
//! Die Datei bleibt klein: Ab 256 KB wird sie zu `hora.log.1`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static FILE: OnceLock<PathBuf> = OnceLock::new();

/// Größe, ab der die Datei beiseitegelegt wird
const LIMIT: u64 = 256 * 1024;

/// Protokoll im Ordner `dir` anlegen und Abstürze dorthin schreiben
pub fn init(dir: &Path) {
    let _ = FILE.set(dir.join("hora.log"));
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let place = info
            .location()
            .map(|l| format!(" ({}:{})", l.file(), l.line()))
            .unwrap_or_default();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unbekannter Fehler".into());
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("?");
        write(&format!("Absturz im Thread {name}: {message}{place}"));
        // Im Hauptthread endet das Programm; dann sichtbar melden
        if name == "main" {
            hora_platform::error_box(
                "Hora",
                &format!(
                    "Hora ist auf einen Fehler gestoßen und muss beendet werden.\n\n{message}\n\nEinzelheiten stehen in {}.",
                    path().map(|p| p.display().to_string()).unwrap_or_default()
                ),
            );
        }
        previous(info);
    }));
}

pub fn path() -> Option<&'static Path> {
    FILE.get().map(PathBuf::as_path)
}

/// Eine Zeile mit Zeitstempel anhängen
pub fn write(message: &str) {
    let Some(path) = path() else {
        return;
    };
    if std::fs::metadata(path).is_ok_and(|m| m.len() > LIMIT) {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{stamp} {message}");
    }
    #[cfg(debug_assertions)]
    eprintln!("{message}");
}

/// Schwerer Fehler beim Start: protokollieren, anzeigen
pub fn fatal(message: &str) {
    write(&format!("Start abgebrochen: {message}"));
    hora_platform::error_box("Hora", message);
}

#[cfg(test)]
mod tests {
    #[test]
    fn lines_are_appended_and_the_file_rotates() {
        let dir = tempfile::tempdir().unwrap();
        super::init(dir.path());
        // Ein anderer Test im selben Prozess hat vielleicht schon einen Ordner gesetzt
        let path = super::path().unwrap().to_path_buf();
        super::write("erste Zeile");
        super::write("zweite Zeile");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("erste Zeile\n") || text.contains("erste Zeile\r\n"));
        assert!(text.lines().last().unwrap().ends_with("zweite Zeile"));
        std::fs::write(&path, vec![b'x'; super::LIMIT as usize + 1]).unwrap();
        super::write("nach dem Drehen");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().lines().count(),
            1,
            "neue Datei"
        );
        assert!(path.with_extension("log.1").exists());
    }
}
