//! Start mit Windows über `HKCU\…\CurrentVersion\Run` (ohne Adminrechte;
//! Windows zeigt den Eintrag unter Einstellungen → Apps → Autostart).

use std::path::Path;

use crate::{PlatformError, registry};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Befehlszeile für den Autostart: Programm in Anführungszeichen, dann `args`
pub fn command_line(exe: &Path, args: &str) -> String {
    let exe = exe.display();
    if args.is_empty() {
        format!("\"{exe}\"")
    } else {
        format!("\"{exe}\" {args}")
    }
}

/// Eingetragene Befehlszeile, falls Autostart an ist
pub fn current(name: &str) -> Option<String> {
    registry::get_string(RUN, name)
}

pub fn enable(name: &str, exe: &Path, args: &str) -> Result<(), PlatformError> {
    registry::set_string(RUN, name, &command_line(exe, args))
}

pub fn disable(name: &str) -> Result<(), PlatformError> {
    registry::delete_value(RUN, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_quotes_the_program() {
        let exe = Path::new(r"C:\Users\A B\AppData\Local\Programs\Hora\hora.exe");
        assert_eq!(
            command_line(exe, "--tray"),
            r#""C:\Users\A B\AppData\Local\Programs\Hora\hora.exe" --tray"#
        );
        assert_eq!(
            command_line(exe, ""),
            r#""C:\Users\A B\AppData\Local\Programs\Hora\hora.exe""#
        );
    }
}
