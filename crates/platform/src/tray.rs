//! Symbol im Infobereich der Taskleiste mit Kontextmenü. Läuft in einem
//! eigenen Thread mit Win32-Nachrichtenschleife und einem unsichtbaren
//! Fenster (Klasse [`WINDOW_CLASS`](crate::instance::WINDOW_CLASS), Titel =
//! Instanzname), damit es unabhängig vom UI-Backend funktioniert und die
//! zweite Instanz die erste findet.

#[cfg(not(windows))]
use crate::PlatformError;

/// Nachricht der zweiten Instanz: Fenster zeigen
pub const WM_SHOW_REQUEST: u32 = 0x8000 + 2;
/// Nachricht von `hora --quit` (Installer vor einem Update): beenden
pub const WM_QUIT_REQUEST: u32 = 0x8000 + 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// Klick auf das Symbol oder zweiter Start von Hora
    Open,
    /// Menüeintrag mit dieser ID
    Menu(u32),
    /// Ein anderer Prozess bittet Hora, sich zu beenden
    Quit,
}

/// Eintrag im Kontextmenü; `id == 0` ist ein Trennstrich
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub id: u32,
    pub label: String,
    /// Fett, wie der Eintrag für Doppelklick
    pub default: bool,
}

impl MenuItem {
    pub fn new(id: u32, label: &str) -> Self {
        MenuItem {
            id,
            label: label.into(),
            default: false,
        }
    }

    pub fn separator() -> Self {
        MenuItem {
            id: 0,
            label: String::new(),
            default: false,
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::mpsc;

    use super::{MenuItem, TrayEvent, WM_QUIT_REQUEST, WM_SHOW_REQUEST};
    use crate::PlatformError;
    use crate::instance::WINDOW_CLASS;
    use crate::registry::wide;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, HICON, IDI_APPLICATION,
        IMAGE_ICON, LR_DEFAULTCOLOR, LoadIconW, LoadImageW, MF_SEPARATOR, MF_STRING, MSG,
        PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SM_CXSMICON,
        SM_CYSMICON, SetForegroundWindow, SetMenuDefaultItem, TPM_BOTTOMALIGN, TPM_RETURNCMD,
        TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_CLOSE, WM_DESTROY, WM_LBUTTONDBLCLK,
        WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSEXW, WS_POPUP,
    };

    const WM_TRAY: u32 = 0x8000 + 1;

    /// Menüeinträge; die App kann sie ersetzen (z. B. nach einem Sprachwechsel)
    type SharedMenu = std::sync::Arc<std::sync::Mutex<Vec<MenuItem>>>;

    struct State {
        icon: HICON,
        tooltip: String,
        menu: SharedMenu,
        handler: Rc<dyn Fn(TrayEvent)>,
        taskbar_created: u32,
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    fn icon_data(hwnd: HWND, icon: HICON, tooltip: &str) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: icon,
            ..Default::default()
        };
        for (slot, c) in data.szTip.iter_mut().zip(tooltip.encode_utf16().take(127)) {
            *slot = c;
        }
        data
    }

    fn add_icon(hwnd: HWND) {
        STATE.with_borrow(|s| {
            if let Some(s) = s {
                let data = icon_data(hwnd, s.icon, &s.tooltip);
                // SAFETY: `data` ist vollständig initialisiert und lebt während des Aufrufs.
                unsafe { Shell_NotifyIconW(NIM_ADD, &data) };
            }
        });
    }

    fn handler() -> Option<Rc<dyn Fn(TrayEvent)>> {
        STATE.with_borrow(|s| s.as_ref().map(|s| s.handler.clone()))
    }

    fn show_menu(hwnd: HWND) {
        let Some(items) = STATE.with_borrow(|s| {
            s.as_ref()
                .map(|s| s.menu.lock().map(|m| m.clone()).unwrap_or_default())
        }) else {
            return;
        };
        // SAFETY: Menü wird hier erzeugt und wieder zerstört; Zeichenketten
        // leben bis nach AppendMenuW. SetForegroundWindow und WM_NULL sind
        // nötig, damit das Menü beim Klick daneben verschwindet.
        let chosen = unsafe {
            let menu = CreatePopupMenu();
            for item in &items {
                if item.id == 0 {
                    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                } else {
                    let label = wide(&item.label);
                    AppendMenuW(menu, MF_STRING, item.id as usize, label.as_ptr());
                    if item.default {
                        SetMenuDefaultItem(menu, item.id, 0);
                    }
                }
            }
            let mut point = POINT { x: 0, y: 0 };
            GetCursorPos(&mut point);
            SetForegroundWindow(hwnd);
            let chosen = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
                point.x,
                point.y,
                0,
                hwnd,
                std::ptr::null(),
            );
            PostMessageW(hwnd, WM_NULL, 0, 0);
            DestroyMenu(menu);
            chosen
        };
        if chosen > 0
            && let Some(h) = handler()
        {
            h(TrayEvent::Menu(chosen as u32));
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let taskbar_created = STATE.with_borrow(|s| s.as_ref().map_or(0, |s| s.taskbar_created));
        match message {
            WM_TRAY => {
                match lparam as u32 {
                    WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                        if let Some(h) = handler() {
                            h(TrayEvent::Open);
                        }
                    }
                    WM_RBUTTONUP => show_menu(hwnd),
                    _ => {}
                }
                0
            }
            WM_SHOW_REQUEST => {
                if let Some(h) = handler() {
                    h(TrayEvent::Open);
                }
                0
            }
            WM_QUIT_REQUEST => {
                if let Some(h) = handler() {
                    h(TrayEvent::Quit);
                }
                0
            }
            WM_CLOSE => {
                let data = icon_data(hwnd, std::ptr::null_mut(), "");
                // SAFETY: `data` lebt während der Aufrufe; das Fenster gehört diesem Thread.
                unsafe {
                    Shell_NotifyIconW(NIM_DELETE, &data);
                    DestroyWindow(hwnd);
                }
                0
            }
            WM_DESTROY => {
                // SAFETY: beendet nur die Nachrichtenschleife dieses Threads
                unsafe { PostQuitMessage(0) };
                0
            }
            // Explorer neu gestartet: Symbol wieder anlegen
            m if m == taskbar_created && m != 0 => {
                add_icon(hwnd);
                0
            }
            // SAFETY: Standardbehandlung mit den unveränderten Argumenten
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }

    fn load_icon() -> HICON {
        // SAFETY: Ressource 1 ist das Programmsymbol (build.rs der App); fehlt
        // sie, gibt LoadImageW null zurück und das Standardsymbol greift.
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            let icon = LoadImageW(
                module,
                // MAKEINTRESOURCE(1)
                std::ptr::without_provenance::<u16>(1),
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_DEFAULTCOLOR,
            );
            if icon.is_null() {
                LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
            } else {
                icon
            }
        }
    }

    pub struct Tray {
        hwnd: usize,
        thread: Option<std::thread::JoinHandle<()>>,
        menu: SharedMenu,
    }

    impl Tray {
        pub fn new(
            instance: &str,
            tooltip: &str,
            menu: Vec<MenuItem>,
            on_event: impl Fn(TrayEvent) + Send + 'static,
        ) -> Result<Tray, PlatformError> {
            let (sender, receiver) = mpsc::channel::<Result<usize, String>>();
            let (instance, tooltip) = (instance.to_string(), tooltip.to_string());
            let shared: SharedMenu = std::sync::Arc::new(std::sync::Mutex::new(menu));
            let menu = shared.clone();
            let thread = std::thread::Builder::new()
                .name("infobereich".into())
                .spawn(move || {
                    let class = wide(WINDOW_CLASS);
                    let title = wide(&instance);
                    // SAFETY: Klasse und Fenster werden in diesem Thread angelegt
                    // und benutzt; alle Zeiger zeigen auf lebende Puffer.
                    let hwnd = unsafe {
                        let module = GetModuleHandleW(std::ptr::null());
                        let wc = WNDCLASSEXW {
                            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                            lpfnWndProc: Some(window_proc),
                            hInstance: module,
                            lpszClassName: class.as_ptr(),
                            ..Default::default()
                        };
                        RegisterClassExW(&wc);
                        CreateWindowExW(
                            0,
                            class.as_ptr(),
                            title.as_ptr(),
                            WS_POPUP,
                            0,
                            0,
                            0,
                            0,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            module,
                            std::ptr::null(),
                        )
                    };
                    if hwnd.is_null() {
                        let _ =
                            sender.send(Err("Fenster für den Infobereich nicht anlegbar".into()));
                        return;
                    }
                    let taskbar = wide("TaskbarCreated");
                    STATE.set(Some(State {
                        icon: load_icon(),
                        tooltip,
                        menu,
                        handler: Rc::new(on_event),
                        // SAFETY: nullterminierter Puffer
                        taskbar_created: unsafe { RegisterWindowMessageW(taskbar.as_ptr()) },
                    }));
                    add_icon(hwnd);
                    let _ = sender.send(Ok(hwnd as usize));
                    let mut msg: MSG = Default::default();
                    // SAFETY: übliche Nachrichtenschleife dieses Threads
                    unsafe {
                        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                            TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                    STATE.set(None);
                })
                .map_err(|e| PlatformError(e.to_string()))?;
            let hwnd = receiver
                .recv()
                .map_err(|e| PlatformError(e.to_string()))?
                .map_err(PlatformError)?;
            Ok(Tray {
                hwnd,
                thread: Some(thread),
                menu: shared,
            })
        }

        /// Menüeinträge ersetzen; gilt ab dem nächsten Rechtsklick
        pub fn set_menu(&self, items: Vec<MenuItem>) {
            if let Ok(mut menu) = self.menu.lock() {
                *menu = items;
            }
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            // SAFETY: Fenster des Infobereich-Threads; WM_CLOSE entfernt das Symbol
            unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

#[cfg(windows)]
pub use imp::Tray;

#[cfg(not(windows))]
pub struct Tray;

#[cfg(not(windows))]
impl Tray {
    pub fn new(
        _instance: &str,
        _tooltip: &str,
        _menu: Vec<MenuItem>,
        _on_event: impl Fn(TrayEvent) + Send + 'static,
    ) -> Result<Tray, PlatformError> {
        Err(PlatformError("Infobereich nur unter Windows".into()))
    }

    pub fn set_menu(&self, _items: Vec<MenuItem>) {}
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn second_instance_reaches_the_tray_window() {
        let name = format!("Hora-Test-Tray-{}", std::process::id());
        let (sender, receiver) = mpsc::channel();
        let tray = Tray::new(
            &name,
            "Hora Test",
            vec![MenuItem::new(1, "Beenden")],
            move |e| {
                let _ = sender.send(e);
            },
        )
        .expect("Infobereich");
        assert!(crate::instance::activate_running(&name), "Fenster gefunden");
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5)),
            Ok(TrayEvent::Open)
        );
        drop(tray);
        assert!(
            !crate::instance::activate_running(&name),
            "nach dem Beenden weg"
        );
    }
}
