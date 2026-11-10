//! Hora Setup: eigener Installer und Deinstaller mit der Oberfläche der App.
//!
//! Dieselbe Datei hat zwei Rollen: Mit angehängten Programmdateien
//! (Hora-Setup-<Version>.exe) installiert oder aktualisiert sie, ohne Anhang
//! (uninstall.exe im Installationsordner) entfernt sie Hora.
//!
//! Befehlszeile wie beim früheren NSIS-Installer:
//! - `/S` still, ohne Fenster; `/D=<Ordner>` Installationsordner (als Letztes)
//! - Deinstaller: `/S` still, `_?=<Ordner>` im Ordner laufen und warten
//! - `--lang=de`, `--no-autostart`, `--desktop-shortcut` für stille Installation
//! - `--relaunch`: nach stiller Installation Hora starten (Update aus der App)
//! - `--pack <Ordner> <Ziel.exe>` baut eine Setup-Datei (tools/build-installer.ps1)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod install;
mod payload;
mod win;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Datelike, Days};
use hora_i18n::{Lang, dates, tr};
use slint::{Color, ComponentHandle, ModelRc, SharedString, VecModel};

use crate::install::Options;
use crate::payload::Payload;

slint::include_modules!();

/// Farben der Beispielkalender in der App (für die Punkte im Mini-Kalender)
const DOT_COLORS: [u32; 5] = [0x4f7cac, 0x6a9a6b, 0x8a6fb0, 0xc49a3a, 0xb8645a];

#[derive(Debug, Default, PartialEq)]
struct Args {
    silent: bool,
    dir: Option<PathBuf>,
    /// Deinstaller läuft im Installationsordner und wartet (NSIS: `_?=`)
    in_place: Option<PathBuf>,
    uninstall: bool,
    pack: Option<(PathBuf, PathBuf)>,
    lang: Option<String>,
    no_autostart: bool,
    desktop: bool,
    /// Nach stiller Installation Hora wieder starten (Update aus der App)
    relaunch: bool,
    /// Entwicklung: Seite als Bild speichern (`--snapshot <png> <Seite> <Modus>`)
    snapshot: Option<(PathBuf, i32, i32)>,
    /// Entwicklung: die Schritte im Fenster selbst durchklicken (Test)
    autopilot: bool,
}

fn parse_args(args: &[String]) -> Args {
    let mut a = Args::default();
    let rest_of_line = |first: &str, more: &[String]| {
        let mut path = first.to_string();
        for part in more {
            path.push(' ');
            path.push_str(part);
        }
        PathBuf::from(path)
    };
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg.eq_ignore_ascii_case("/S") {
            a.silent = true;
        } else if let Some(first) = arg.strip_prefix("/D=") {
            // Wie bei NSIS: der Rest der Zeile ist der Pfad, auch mit Leerzeichen
            a.dir = Some(rest_of_line(first, &args[i + 1..]));
            break;
        } else if let Some(first) = arg.strip_prefix("_?=") {
            a.in_place = Some(rest_of_line(first, &args[i + 1..]));
            a.uninstall = true;
            break;
        } else if arg == "--uninstall" {
            a.uninstall = true;
        } else if arg == "--pack" && i + 2 < args.len() {
            a.pack = Some((args[i + 1].clone().into(), args[i + 2].clone().into()));
            i += 2;
        } else if arg == "--snapshot" && i + 3 < args.len() {
            a.snapshot = Some((
                args[i + 1].clone().into(),
                args[i + 2].parse().unwrap_or(0),
                args[i + 3].parse().unwrap_or(0),
            ));
            i += 3;
        } else if let Some(code) = arg.strip_prefix("--lang=") {
            a.lang = Some(code.to_string());
        } else if arg == "--no-autostart" {
            a.no_autostart = true;
        } else if arg == "--desktop-shortcut" {
            a.desktop = true;
        } else if arg == "--relaunch" {
            a.relaunch = true;
        } else if arg == "--autopilot" {
            a.autopilot = true;
        }
        i += 1;
    }
    a
}

/// Fehler und stille Läufe in %TEMP%\Hora-Setup.log
fn log(message: &str) {
    use std::io::Write;
    let path = std::env::temp_dir().join("Hora-Setup.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(file, "{stamp} {message}");
    }
}

fn main() -> ExitCode {
    let args = parse_args(&std::env::args().skip(1).collect::<Vec<_>>());
    if let Some((dir, target)) = &args.pack {
        return match payload::pack_command(dir, target) {
            Ok(summary) => {
                println!("{summary}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some((path, page, mode)) = &args.snapshot {
        return match preview(&args, path, *page, *mode) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }
    let Ok(exe) = std::env::current_exe() else {
        return ExitCode::FAILURE;
    };
    select_backend();
    let result = match payload::read(&exe) {
        Ok(Some(payload)) if !args.uninstall => installer(&args, &exe, payload),
        Ok(_) => uninstaller(&args, &exe),
        Err(e) => Err(e),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log(&format!("Fehler: {e}"));
            if !args.silent {
                hora_platform::error_box(tr!("Hora Setup"), &e);
            }
            ExitCode::FAILURE
        }
    }
}

/// Sprache des Setups: Befehlszeile, frühere Wahl im Installer, sonst Englisch
fn initial_language(args: &Args) -> Lang {
    args.lang
        .clone()
        .or_else(install::installer_language)
        .and_then(|c| Lang::from_code(&c))
        .unwrap_or(Lang::En)
}

fn installer(args: &Args, exe: &Path, payload: Payload) -> Result<(), String> {
    let existing = install::installed();
    let stub = payload::stub_bytes(exe, payload.stub_len)?;
    if args.silent {
        let dir = args
            .dir
            .clone()
            .or_else(|| existing.as_ref().map(|e| e.dir.clone()))
            .unwrap_or_else(install::default_dir);
        let opts = Options {
            dir,
            autostart: !args.no_autostart && (existing.is_none() || install::autostart_enabled()),
            desktop: args.desktop
                || (existing.is_some() && install::desktop_link().is_some_and(|l| l.exists())),
            language: args.lang.clone(),
        };
        log(&format!("Stille Installation nach {}", opts.dir.display()));
        install::install(&payload, &stub, &opts, &|_, _| {})?;
        if args.relaunch {
            win::launch(&opts.dir.join(install::EXE), &[])?;
        }
        return Ok(());
    }
    let lang = initial_language(args);
    hora_i18n::set_language(lang);
    let window = SetupWindow::new().map_err(|e| e.to_string())?;
    let _ = slint::select_bundled_translation(lang.code());
    window.set_mode(i32::from(existing.is_some()));
    window.set_version(env!("CARGO_PKG_VERSION").into());
    window.set_old_version(
        existing
            .as_ref()
            .map(|e| e.version.clone())
            .unwrap_or_default()
            .into(),
    );
    let dir = args
        .dir
        .clone()
        .or_else(|| existing.as_ref().map(|e| e.dir.clone()))
        .unwrap_or_else(install::default_dir);
    window.set_install_dir(dir.display().to_string().into());
    window.set_can_change_dir(existing.is_none());
    window.set_autostart(existing.is_none() || install::autostart_enabled());
    window.set_desktop(existing.is_some() && install::desktop_link().is_some_and(|l| l.exists()));
    window.set_languages(languages());
    window.set_language(lang.code().into());
    let size_mb = (payload.total_size() as f64 / 1e6).ceil() as u64;
    let relabel = move |w: &SetupWindow| {
        w.set_size_text(tr!("Hora needs about {} MB of disk space.", size_mb).into());
        calendar(w);
    };
    relabel(&window);

    window.on_choose_language({
        let weak = window.as_weak();
        move |code| {
            let Some(w) = weak.upgrade() else { return };
            let lang = Lang::from_code(&code).unwrap_or(Lang::En);
            hora_i18n::set_language(lang);
            let _ = slint::select_bundled_translation(lang.code());
            w.set_language(code);
            relabel(&w);
        }
    });
    window.on_browse({
        let weak = window.as_weak();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let current = PathBuf::from(w.get_install_dir().as_str());
            let start = current.parent().map(Path::to_path_buf).unwrap_or(current);
            if let Some(chosen) =
                win::pick_folder(hwnd(&w), &start, tr!("Choose a folder for Hora"))
            {
                w.set_install_dir(
                    install::with_app_folder(&chosen)
                        .display()
                        .to_string()
                        .into(),
                );
            }
        }
    });
    let payload = Arc::new(payload);
    let stub = Arc::new(stub);
    window.on_start({
        let weak = window.as_weak();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let opts = Options {
                dir: PathBuf::from(w.get_install_dir().as_str()),
                autostart: w.get_autostart(),
                desktop: w.get_desktop(),
                language: Some(w.get_language().to_string()),
            };
            w.set_progress(0.0);
            w.set_status(tr!("Preparing…").into());
            w.set_page(2);
            let (payload, stub, weak) = (payload.clone(), stub.clone(), weak.clone());
            std::thread::spawn(move || {
                let progress = weak.clone();
                let report = move |value: f32, text: String| {
                    let w = progress.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = w.upgrade() {
                            w.set_progress(value);
                            w.set_status(text.into());
                        }
                    });
                };
                let result = install::install(&payload, &stub, &opts, &report);
                if let Err(e) = &result {
                    log(&format!("Installation fehlgeschlagen: {e}"));
                }
                finished(weak, result);
            });
        }
    });
    window.on_finish({
        let weak = window.as_weak();
        move || {
            if let Some(w) = weak.upgrade()
                && w.get_launch()
            {
                let exe = PathBuf::from(w.get_install_dir().as_str()).join(install::EXE);
                let _ = win::launch(&exe, &[]);
            }
            let _ = slint::quit_event_loop();
        }
    });
    run(window, args.autopilot)
}

fn uninstaller(args: &Args, exe: &Path) -> Result<(), String> {
    let existing = install::installed();
    // Ordner: _?=, sonst der Ordner dieser Datei, sonst laut Registry
    let dir = args
        .in_place
        .clone()
        .or_else(|| {
            exe.parent()
                .filter(|d| d.join(install::EXE).is_file())
                .map(Path::to_path_buf)
        })
        .or_else(|| existing.as_ref().map(|e| e.dir.clone()))
        .ok_or_else(|| tr!("Hora is not installed.").to_string())?;
    // Läuft der Deinstaller aus dem Ordner, löscht er sich erst nach dem Beenden
    let inside = exe.parent().is_some_and(|p| same_path(p, &dir));
    let cleanup = inside && args.in_place.is_none();
    if args.silent {
        log(&format!("Stille Deinstallation aus {}", dir.display()));
        install::uninstall(&dir, false, inside, &|_, _| {})?;
        if cleanup {
            install::schedule_cleanup(&dir);
        }
        return Ok(());
    }
    let lang = initial_language(args);
    hora_i18n::set_language(lang);
    let window = SetupWindow::new().map_err(|e| e.to_string())?;
    let _ = slint::select_bundled_translation(lang.code());
    window.set_mode(2);
    window.set_version(
        existing
            .map(|e| e.version)
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
            .into(),
    );
    calendar(&window);
    window.on_start({
        let weak = window.as_weak();
        let dir = dir.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let delete_data = w.get_delete_data();
            w.set_progress(0.0);
            w.set_page(2);
            let (weak, dir) = (weak.clone(), dir.clone());
            std::thread::spawn(move || {
                let progress = weak.clone();
                let report = move |value: f32, text: String| {
                    let w = progress.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = w.upgrade() {
                            w.set_progress(value);
                            w.set_status(text.into());
                        }
                    });
                };
                let result = install::uninstall(&dir, delete_data, inside, &report);
                if let Err(e) = &result {
                    log(&format!("Deinstallation fehlgeschlagen: {e}"));
                }
                finished(weak, result);
            });
        }
    });
    window.on_finish(move || {
        if cleanup {
            install::schedule_cleanup(&dir);
        }
        let _ = slint::quit_event_loop();
    });
    run(window, args.autopilot)
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.canonicalize()
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .trim_end_matches('\\')
            .to_lowercase()
    };
    norm(a) == norm(b)
}

/// Ergebnis aus dem Arbeitsthread anzeigen
fn finished(weak: slint::Weak<SetupWindow>, result: Result<(), String>) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(w) = weak.upgrade() {
            match result {
                Ok(()) => {
                    w.set_progress(1.0);
                    // Kurz bei 100 % stehen lassen
                    let weak = w.as_weak();
                    slint::Timer::single_shot(Duration::from_millis(350), move || {
                        if let Some(w) = weak.upgrade() {
                            w.set_page(3);
                        }
                    });
                }
                Err(e) => {
                    w.set_error(e.into());
                    w.set_page(4);
                }
            }
        }
    });
}

/// Gemeinsame Rückrufe, Titelleiste, Ereignisschleife
fn run(window: SetupWindow, autopilot: bool) -> Result<(), String> {
    window.on_cancel(|| {
        let _ = slint::quit_event_loop();
    });
    window.on_retry({
        let weak = window.as_weak();
        move || {
            if let Some(w) = weak.upgrade() {
                w.set_page(if w.get_mode() == 2 { 0 } else { 1 });
            }
        }
    });
    window.window().on_close_requested({
        let weak = window.as_weak();
        move || match weak.upgrade() {
            // Während der Arbeit nicht schließen
            Some(w) if w.get_page() == 2 => slint::CloseRequestResponse::KeepWindowShown,
            _ => {
                let _ = slint::quit_event_loop();
                slint::CloseRequestResponse::HideWindow
            }
        }
    });
    {
        use slint::winit_030::WinitWindowAccessor;
        let weak = window.as_weak();
        window.on_window_drag(move || {
            if let Some(w) = weak.upgrade() {
                w.window().with_winit_window(|w| w.drag_window().ok());
            }
        });
        let weak = window.as_weak();
        window.on_window_minimize(move || {
            if let Some(w) = weak.upgrade() {
                w.window().set_minimized(true);
            }
        });
        let weak = window.as_weak();
        window.on_window_close(move || {
            if let Some(w) = weak.upgrade() {
                let _ = w
                    .window()
                    .dispatch_event_with_result(slint::platform::WindowEvent::CloseRequested);
            }
        });
    }
    window.show().map_err(|e| e.to_string())?;
    if let Some(h) = hwnd(&window) {
        hora_platform::round_corners(h);
    }
    if autopilot {
        start_autopilot(&window);
    }
    slint::run_event_loop().map_err(|e| e.to_string())?;
    let _ = window.hide();
    Ok(())
}

/// Klickt sich durch: Sprache Deutsch, weiter, installieren bzw. entfernen,
/// fertig (ohne Hora zu starten). Ausgabe und Exit-Code für Tests.
#[cfg(feature = "devtools")]
fn start_autopilot(window: &SetupWindow) {
    let weak = window.as_weak();
    let mut tick = 0;
    let timer = Box::leak(Box::new(slint::Timer::default()));
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(200),
        move || {
            let Some(w) = weak.upgrade() else { return };
            tick += 1;
            let removing = w.get_mode() == 2;
            match w.get_page() {
                0 if !removing && tick == 3 => w.invoke_choose_language("de".into()),
                0 if !removing && tick > 5 => {
                    println!("autopilot: Sprache {}", hora_i18n::language().code());
                    w.set_page(1);
                }
                0 if removing && tick > 3 => w.invoke_start(),
                1 => {
                    println!("autopilot: installiere nach {}", w.get_install_dir());
                    w.invoke_start();
                }
                3 => {
                    println!("autopilot: fertig");
                    w.set_launch(false);
                    w.invoke_finish();
                }
                4 => {
                    println!("autopilot: Fehler: {}", w.get_error());
                    std::process::exit(1);
                }
                _ if tick > 300 => {
                    println!("autopilot: Zeit abgelaufen auf Seite {}", w.get_page());
                    std::process::exit(1);
                }
                _ => {}
            }
        },
    );
}

#[cfg(not(feature = "devtools"))]
fn start_autopilot(_: &SetupWindow) {}

/// Software-Renderer wie die App; rahmenloses Fenster mit dem Schatten von
/// Windows
fn select_backend() {
    let selector = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into());
    #[cfg(windows)]
    let selector = selector.with_winit_window_attributes_hook(|attributes| {
        use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
        attributes.with_undecorated_shadow(true)
    });
    let _ = selector.select();
}

fn hwnd(window: &SetupWindow) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = window.window().window_handle();
    match handle.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

fn languages() -> ModelRc<Language> {
    ModelRc::new(VecModel::from(
        Lang::ALL
            .iter()
            .map(|l| Language {
                code: l.code().into(),
                name: l.native_name().into(),
                tag: l.code().to_uppercase().into(),
            })
            .collect::<Vec<_>>(),
    ))
}

/// Mini-Kalender des laufenden Monats mit ein paar Beispielpunkten
fn calendar(window: &SetupWindow) {
    let today = chrono::Local::now().date_naive();
    let first = today.with_day(1).unwrap_or(today);
    let start = first - Days::new(u64::from(first.weekday().num_days_from_monday()));
    let days: Vec<CalDay> = (0..42)
        .map(|i| {
            let day = start + Days::new(i);
            let n = day.day();
            let dot = (n * 7 + day.month() * 3) % 9;
            CalDay {
                label: SharedString::from(n.to_string()),
                today: day == today,
                outside: day.month() != today.month(),
                dot: if dot < 3 && day.month() == today.month() {
                    let c = DOT_COLORS[(n as usize) % DOT_COLORS.len()];
                    Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8)
                } else {
                    Color::default()
                },
            }
        })
        .collect();
    window.set_days(ModelRc::new(VecModel::from(days)));
    window.set_letters(ModelRc::new(VecModel::from(
        (0..7)
            .map(|i| SharedString::from(dates::weekday_letter(i)))
            .collect::<Vec<_>>(),
    )));
    window.set_month(
        dates::capitalize(&dates::month_year(today.year(), today.month0() as usize)).into(),
    );
}

/// Entwicklung: eine Seite mit Beispielwerten als Bild speichern, ohne etwas
/// zu installieren (`--snapshot <png> <Seite> <Modus>`, `HORA_THEME=dark`)
#[cfg(feature = "devtools")]
fn preview(args: &Args, path: &Path, page: i32, mode: i32) -> Result<(), String> {
    let lang = args
        .lang
        .as_deref()
        .and_then(Lang::from_code)
        .unwrap_or(Lang::En);
    hora_i18n::set_language(lang);
    select_backend();
    let window = SetupWindow::new().map_err(|e| e.to_string())?;
    let _ = slint::select_bundled_translation(lang.code());
    match std::env::var("HORA_THEME").as_deref() {
        Ok("light") => window.global::<Theme>().set_scheme_override(1),
        Ok("dark") => window.global::<Theme>().set_scheme_override(2),
        _ => {}
    }
    window.set_version(env!("CARGO_PKG_VERSION").into());
    window.set_languages(languages());
    window.set_language(lang.code().into());
    window.set_size_text(tr!("Hora needs about {} MB of disk space.", 32).into());
    calendar(&window);
    window.set_mode(mode);
    window.set_page(page);
    window.set_progress(0.62);
    window.set_status(tr!("Copying {}…", "hora.exe").into());
    // Beispielpfad statt des echten (Bilder sollen keinen Benutzernamen zeigen)
    window.set_install_dir(r"C:\Users\you\AppData\Local\Programs\Hora".into());
    window.set_old_version("1.0.0".into());
    window.set_error(
        tr!("Hora is still running. Please quit it (tray icon → Quit) and try again.").into(),
    );
    window.show().map_err(|e| e.to_string())?;
    let weak = window.as_weak();
    let path = path.to_path_buf();
    // Ein paar Bilder warten, bis Animationen durch sind
    slint::Timer::single_shot(Duration::from_millis(1600), move || {
        if let Some(w) = weak.upgrade() {
            let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                let buffer = w.window().take_snapshot()?;
                let file = std::io::BufWriter::new(std::fs::File::create(&path)?);
                let mut encoder = png::Encoder::new(file, buffer.width(), buffer.height());
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .write_header()?
                    .write_image_data(buffer.as_bytes())?;
                Ok(())
            })();
            if let Err(e) = result {
                eprintln!("snapshot fehlgeschlagen: {e}");
            }
        }
        let _ = slint::quit_event_loop();
    });
    slint::run_event_loop().map_err(|e| e.to_string())
}

#[cfg(not(feature = "devtools"))]
fn preview(_: &Args, _: &Path, _: i32, _: i32) -> Result<(), String> {
    Err("snapshot nur mit --features devtools".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Args {
        parse_args(&line.split(' ').map(String::from).collect::<Vec<_>>())
    }

    #[test]
    fn nsis_style_command_line() {
        let a = args(r"/S /D=C:\Users\Test User\Apps\Hora");
        assert!(a.silent);
        assert_eq!(a.dir, Some(PathBuf::from(r"C:\Users\Test User\Apps\Hora")));
        let u = args(r"/S _?=C:\Program Dir\Hora");
        assert!(u.silent && u.uninstall);
        assert_eq!(u.in_place, Some(PathBuf::from(r"C:\Program Dir\Hora")));
        let p = args("--pack stage out.exe --lang=de");
        assert_eq!(
            p.pack,
            Some((PathBuf::from("stage"), PathBuf::from("out.exe")))
        );
        assert_eq!(p.lang.as_deref(), Some("de"));
    }

    #[test]
    fn update_from_the_app() {
        // So startet Hora das geladene Setup
        let a = args("/S --relaunch");
        assert!(a.silent && a.relaunch && !a.uninstall);
        assert_eq!(a.dir, None);
        assert!(!args("/S").relaunch);
    }
}
