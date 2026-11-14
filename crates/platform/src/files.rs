//! Dateien wählen: die Dialoge "Speichern unter" und "Öffnen" von Windows.

use std::path::PathBuf;

/// Welche Dateien der Dialog zeigt, z. B. ("Kalender (*.ics)", "*.ics")
pub struct Filter<'a> {
    pub name: &'a str,
    pub pattern: &'a str,
    /// Endung, die "Speichern unter" ergänzt, ohne Punkt
    pub extension: &'a str,
}

/// "Speichern unter" mit Vorschlag für den Dateinamen; `None` bei Abbruch.
/// `owner` ist das Fenster, über dem der Dialog steht (HWND).
pub fn save_dialog(
    owner: Option<isize>,
    title: &str,
    file_name: &str,
    filter: &Filter,
) -> Option<PathBuf> {
    imp::show(owner, title, Some(file_name), filter)
}

/// "Öffnen" für eine vorhandene Datei; `None` bei Abbruch
pub fn open_dialog(owner: Option<isize>, title: &str, filter: &Filter) -> Option<PathBuf> {
    imp::show(owner, title, None, filter)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree,
    };
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FOS_FILEMUSTEXIST, FOS_OVERWRITEPROMPT, FileOpenDialog, FileSaveDialog, IFileDialog,
        IFileOpenDialog, IFileSaveDialog, SIGDN_FILESYSPATH,
    };
    use windows::core::{HSTRING, Interface, PCWSTR};

    /// `file_name` gesetzt: Speichern, sonst Öffnen
    pub fn show(
        owner: Option<isize>,
        title: &str,
        file_name: Option<&str>,
        filter: &Filter,
    ) -> Option<PathBuf> {
        // SAFETY: mehrfaches Initialisieren im selben Thread ist erlaubt
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let name = HSTRING::from(filter.name);
        let pattern = HSTRING::from(filter.pattern);
        let specs = [COMDLG_FILTERSPEC {
            pszName: PCWSTR(name.as_ptr()),
            pszSpec: PCWSTR(pattern.as_ptr()),
        }];
        // SAFETY: COM-Aufrufe; die Zeichenketten der Filter leben bis zum Ende,
        // der Pfadpuffer wird mit CoTaskMemFree freigegeben
        unsafe {
            let dialog: IFileDialog = match file_name {
                Some(_) => CoCreateInstance::<_, IFileSaveDialog>(
                    &FileSaveDialog,
                    None,
                    CLSCTX_INPROC_SERVER,
                )
                .ok()?
                .cast()
                .ok()?,
                None => CoCreateInstance::<_, IFileOpenDialog>(
                    &FileOpenDialog,
                    None,
                    CLSCTX_INPROC_SERVER,
                )
                .ok()?
                .cast()
                .ok()?,
            };
            let options = dialog.GetOptions().ok()?;
            let extra = if file_name.is_some() {
                FOS_OVERWRITEPROMPT
            } else {
                FOS_FILEMUSTEXIST
            };
            dialog.SetOptions(options | extra).ok()?;
            let _ = dialog.SetTitle(&HSTRING::from(title));
            let _ = dialog.SetFileTypes(&specs);
            let _ = dialog.SetDefaultExtension(&HSTRING::from(filter.extension));
            if let Some(file_name) = file_name {
                let _ = dialog.SetFileName(&HSTRING::from(file_name));
            }
            dialog.Show(owner.map(|h| HWND(h as *mut _))).ok()?;
            let item = dialog.GetResult().ok()?;
            let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let text = path.to_string().ok();
            CoTaskMemFree(Some(path.0 as *const _));
            text.map(PathBuf::from)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn show(
        _owner: Option<isize>,
        _title: &str,
        _file_name: Option<&str>,
        _filter: &Filter,
    ) -> Option<PathBuf> {
        None
    }
}
