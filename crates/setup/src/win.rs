//! Windows-Teile des Setups: Verknüpfungen (COM), Ordnerauswahl, DWORD in
//! der Registry, Programme ohne Konsole starten.

use std::path::{Path, PathBuf};

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::process::CommandExt;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree, IPersistFile,
    };
    use windows::Win32::UI::Shell::{
        FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem, IShellLinkW,
        SHCreateItemFromParsingName, SIGDN_FILESYSPATH, ShellLink,
    };
    use windows::core::{HSTRING, Interface};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    fn com() {
        // SAFETY: mehrfaches Initialisieren im selben Thread ist erlaubt
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    }

    pub fn create_shortcut(link: &Path, target: &Path, description: &str) -> Result<(), String> {
        com();
        if let Some(parent) = link.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let dir = target.parent().unwrap_or(target);
        // SAFETY: COM-Aufrufe mit gültigen Zeichenketten; Schnittstellen
        // werden von windows-rs freigegeben.
        unsafe {
            let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
            shell
                .SetPath(&HSTRING::from(target.as_os_str()))
                .and_then(|_| shell.SetWorkingDirectory(&HSTRING::from(dir.as_os_str())))
                .and_then(|_| shell.SetDescription(&HSTRING::from(description)))
                .and_then(|_| shell.SetIconLocation(&HSTRING::from(target.as_os_str()), 0))
                .map_err(|e| e.to_string())?;
            let file: IPersistFile = shell.cast().map_err(|e| e.to_string())?;
            file.Save(&HSTRING::from(link.as_os_str()), true)
                .map_err(|e| e.to_string())
        }
    }

    /// Ordner wählen; `None` bei Abbruch
    pub fn pick_folder(owner: Option<isize>, start: &Path, title: &str) -> Option<PathBuf> {
        com();
        // SAFETY: COM-Aufrufe; der Pfadpuffer wird mit CoTaskMemFree freigegeben
        unsafe {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            let options = dialog.GetOptions().ok()?;
            dialog.SetOptions(options | FOS_PICKFOLDERS).ok()?;
            let _ = dialog.SetTitle(&HSTRING::from(title));
            let mut folder = Some(start);
            while let Some(f) = folder {
                if f.is_dir() {
                    if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(
                        &HSTRING::from(f.as_os_str()),
                        None,
                    ) {
                        let _ = dialog.SetFolder(&item);
                    }
                    break;
                }
                folder = f.parent();
            }
            let hwnd = owner.map(|h| windows::Win32::Foundation::HWND(h as *mut _));
            dialog.Show(hwnd).ok()?;
            let item = dialog.GetResult().ok()?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let path = name.to_string().ok();
            CoTaskMemFree(Some(name.0 as *const _));
            path.map(PathBuf::from)
        }
    }

    pub fn set_dword(key: &str, name: &str, value: u32) -> Result<(), String> {
        use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_DWORD, RegSetKeyValueW};
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        // SAFETY: nullterminierte Puffer und ein u32, die während des Aufrufs leben
        let code = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                wide(key).as_ptr(),
                wide(name).as_ptr(),
                REG_DWORD,
                (&value as *const u32).cast(),
                4,
            )
        };
        if code == 0 {
            Ok(())
        } else {
            Err(format!("Registry: Fehler {code}"))
        }
    }

    /// Programm unabhängig vom Setup starten
    pub fn launch(exe: &Path, args: &[&str]) -> Result<(), String> {
        std::process::Command::new(exe)
            .args(args)
            .current_dir(exe.parent().unwrap_or(exe))
            .creation_flags(DETACHED_PROCESS)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// cmd.exe-Zeile ohne Fenster (für das Aufräumen nach dem Deinstallieren).
    /// Unverändert übergeben: Rusts Maskierung (\") versteht cmd.exe nicht.
    pub fn run_cmd_hidden(line: &str) -> Result<(), String> {
        std::process::Command::new("cmd.exe")
            .raw_arg(format!("/d /c {line}"))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn create_shortcut(_: &Path, _: &Path, _: &str) -> Result<(), String> {
        Err("nur unter Windows".into())
    }
    pub fn pick_folder(_: Option<isize>, _: &Path, _: &str) -> Option<PathBuf> {
        None
    }
    pub fn set_dword(_: &str, _: &str, _: u32) -> Result<(), String> {
        Err("nur unter Windows".into())
    }
    pub fn launch(exe: &Path, args: &[&str]) -> Result<(), String> {
        std::process::Command::new(exe)
            .args(args)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    pub fn run_cmd_hidden(line: &str) -> Result<(), String> {
        std::process::Command::new("sh")
            .args(["-c", line])
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

pub use imp::*;
