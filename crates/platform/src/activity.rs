//! Vordergrund und Eingaben beobachten, für die App-Zeiten (gezählt wird in
//! `hora_core::activity`).
//!
//! - [`foreground_app`]: welche Anwendung gerade vorn ist (Pfad der exe).
//!   Store-Apps laufen in einem gemeinsamen Rahmenprozess
//!   (ApplicationFrameHost.exe); gemeldet wird die App darin.
//! - [`idle_time`]: wie lange die letzte Eingabe her ist, so wie Windows sie
//!   zählt (Maus, Tastatur und was der Treiber sonst als Eingabe meldet).
//! - [`open_apps`]: Programme mit sichtbarem Fenster, zur Auswahl in den
//!   Einstellungen.
//! - [`Watcher`]: meldet Vordergrundwechsel sowie Sperren und Entsperren
//!   sofort, ohne ständig nachzufragen. Hora lädt dabei keinen Code in andere
//!   Programme (WinEvent außerhalb des Prozesses, wie Barrierefreiheits-Tools).
//!   Auf Wunsch zählt er zusätzlich Stifteingaben (Raw Input), um zu prüfen,
//!   ob Windows sie als Eingabe meldet.

use std::path::PathBuf;
use std::time::Duration;

use crate::PlatformError;

/// Anwendung im Vordergrund
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundApp {
    pub exe: PathBuf,
    pub pid: u32,
}

impl ForegroundApp {
    /// Dateiname der exe, z. B. `notepad.exe`
    pub fn file_name(&self) -> String {
        self.exe
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// Was der [`Watcher`] meldet
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// Ein anderes Fenster ist vorn (gleich nach dem Start einmal der
    /// Ausgangszustand); `None`, wenn Windows nichts Zuordenbares meldet
    Foreground(Option<ForegroundApp>),
    Locked,
    Unlocked,
}

/// Welche Anwendung gerade vorn ist
pub fn foreground_app() -> Option<ForegroundApp> {
    imp::foreground_app()
}

/// Zeit seit der letzten Eingabe in dieser Windows-Sitzung
pub fn idle_time() -> Duration {
    imp::idle_time()
}

/// Programme mit sichtbarem Fenster in der Taskleiste (ohne Hora selbst),
/// je Dateiname einmal, nach Namen sortiert
pub fn open_apps() -> Vec<ForegroundApp> {
    let own = std::process::id();
    let mut apps: Vec<ForegroundApp> = imp::window_apps()
        .into_iter()
        .filter(|app| app.pid != own)
        .collect();
    apps.sort_by_key(|app| app.file_name().to_lowercase());
    apps.dedup_by_key(|app| app.file_name().to_lowercase());
    apps
}

/// Beobachtet Vordergrund und Sperre in einem eigenen Thread, bis er
/// fallen gelassen wird. `on_event` läuft in diesem Thread.
pub struct Watcher(imp::Watcher);

impl Watcher {
    pub fn new(
        count_pen: bool,
        on_event: impl Fn(WatchEvent) + Send + 'static,
    ) -> Result<Watcher, PlatformError> {
        imp::Watcher::new(count_pen, on_event).map(Watcher)
    }

    /// Anzahl der Stifteingaben (Raw-Input-Pakete) seit dem Start; 0, wenn
    /// nicht gezählt wird
    pub fn pen_packets(&self) -> u64 {
        self.0.pen_packets()
    }
}

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleHandleW, GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
    };
    use windows_sys::Win32::System::RemoteDesktop::NOTIFY_FOR_THIS_SESSION;
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows_sys::Win32::UI::Input::{RAWINPUTDEVICE, RIDEV_INPUTSINK, RegisterRawInputDevices};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EVENT_SYSTEM_FOREGROUND,
        EnumChildWindows, EnumWindows, GW_OWNER, GWL_EXSTYLE, GetForegroundWindow, GetMessageW,
        GetWindow, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowThreadProcessId, HWND_MESSAGE,
        IsWindowVisible, MSG, OBJID_WINDOW, PostMessageW, PostQuitMessage, RegisterClassExW,
        TranslateMessage, WINEVENT_OUTOFCONTEXT, WM_CLOSE, WM_DESTROY, WM_INPUT,
        WM_WTSSESSION_CHANGE, WNDCLASSEXW, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW, WTS_SESSION_LOCK,
        WTS_SESSION_UNLOCK,
    };

    use super::{ForegroundApp, WatchEvent};
    use crate::PlatformError;

    const WINDOW_CLASS: &str = "HoraActivityWatcher";
    /// Rahmenprozess der Store-Apps
    const FRAME_HOST: &str = "ApplicationFrameHost.exe";
    /// HID-Seite „Digitizer“: Stift (0x02) und allgemeine Digitizer (0x01)
    const DIGITIZER_PAGE: u16 = 0x0D;
    const DIGITIZER_USAGES: [u16; 2] = [0x01, 0x02];

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn idle_time() -> Duration {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        // SAFETY: `info` ist eine gültige Struktur mit gesetzter Größe
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return Duration::ZERO;
        }
        // Beide Werte zählen Millisekunden seit dem Systemstart und laufen
        // nach 49,7 Tagen über; die Differenz stimmt trotzdem.
        // SAFETY: ohne Argumente
        let now = unsafe { GetTickCount() };
        Duration::from_millis(u64::from(now.wrapping_sub(info.dwTime)))
    }

    fn pid_of(hwnd: HWND) -> u32 {
        let mut pid = 0;
        // SAFETY: Ein ungültiges Fenster liefert 0; `pid` ist beschreibbar.
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        pid
    }

    /// Pfad der exe eines Prozesses. Mit eingeschränktem Abfragerecht klappt
    /// das auch bei Programmen, die als Administrator laufen; geschützte
    /// Systemprozesse liefern `None`.
    pub(super) fn exe_of(pid: u32) -> Option<PathBuf> {
        // SAFETY: Das Handle wird unten wieder geschlossen.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return None;
        }
        let mut buffer = vec![0u16; 1024];
        let mut len = buffer.len() as u32;
        // SAFETY: Puffer und Länge passen zusammen; das Handle ist offen.
        let mut ok = unsafe {
            QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut len)
        };
        if ok == 0 {
            // Sehr lange Pfade
            buffer = vec![0u16; 32768];
            len = buffer.len() as u32;
            // SAFETY: wie oben
            ok = unsafe {
                QueryFullProcessImageNameW(
                    process,
                    PROCESS_NAME_WIN32,
                    buffer.as_mut_ptr(),
                    &mut len,
                )
            };
        }
        // SAFETY: von OpenProcess geöffnet
        unsafe { CloseHandle(process) };
        (ok != 0).then(|| PathBuf::from(OsString::from_wide(&buffer[..len as usize])))
    }

    /// Bei Store-Apps gehört das Fenster dem Rahmenprozess; die eigentliche
    /// App besitzt ein Kindfenster darin.
    fn hosted_app(frame: HWND, frame_pid: u32) -> Option<u32> {
        unsafe extern "system" fn other_process(child: HWND, lparam: LPARAM) -> i32 {
            // SAFETY: `lparam` zeigt auf `search` unten, das während
            // EnumChildWindows lebt.
            let search = unsafe { &mut *(lparam as *mut (u32, u32)) };
            let pid = pid_of(child);
            if pid != 0 && pid != search.0 {
                search.1 = pid;
                return 0;
            }
            1
        }
        let mut search = (frame_pid, 0u32);
        // SAFETY: Rückruf und Zeiger bleiben gültig, bis der Aufruf endet.
        unsafe {
            EnumChildWindows(
                frame,
                Some(other_process),
                &mut search as *mut (u32, u32) as LPARAM,
            )
        };
        (search.1 != 0).then_some(search.1)
    }

    fn app_of(hwnd: HWND) -> Option<ForegroundApp> {
        if hwnd.is_null() {
            return None;
        }
        let pid = pid_of(hwnd);
        if pid == 0 {
            return None;
        }
        let exe = exe_of(pid)?;
        let framed = exe
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(FRAME_HOST));
        if framed
            && let Some(inner) = hosted_app(hwnd, pid)
            && let Some(inner_exe) = exe_of(inner)
        {
            return Some(ForegroundApp {
                exe: inner_exe,
                pid: inner,
            });
        }
        Some(ForegroundApp { exe, pid })
    }

    pub fn foreground_app() -> Option<ForegroundApp> {
        // SAFETY: ohne Argumente
        app_of(unsafe { GetForegroundWindow() })
    }

    /// Steht das Fenster in der Taskleiste? Sichtbar, mit Titel, ohne
    /// Besitzer (keine Dialoge) und kein Werkzeugfenster; ausgeblendete
    /// Store-Apps sind „verhüllt“.
    fn in_taskbar(hwnd: HWND) -> bool {
        // SAFETY: Alle Aufrufe vertragen beliebige Fensterhandles.
        unsafe {
            if IsWindowVisible(hwnd) == 0 || GetWindowTextLengthW(hwnd) == 0 {
                return false;
            }
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            if style & WS_EX_TOOLWINDOW != 0 {
                return false;
            }
            if !GetWindow(hwnd, GW_OWNER).is_null() && style & WS_EX_APPWINDOW == 0 {
                return false;
            }
            let mut cloaked = 0u32;
            let hr = DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED as u32,
                &mut cloaked as *mut u32 as *mut _,
                std::mem::size_of::<u32>() as u32,
            );
            hr != 0 || cloaked == 0
        }
    }

    pub fn window_apps() -> Vec<ForegroundApp> {
        unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
            // SAFETY: `lparam` zeigt auf `windows` unten, das während
            // EnumWindows lebt.
            let windows = unsafe { &mut *(lparam as *mut Vec<HWND>) };
            if in_taskbar(hwnd) {
                windows.push(hwnd);
            }
            1
        }
        let mut windows: Vec<HWND> = Vec::new();
        // SAFETY: Rückruf und Zeiger bleiben gültig, bis der Aufruf endet.
        unsafe { EnumWindows(Some(collect), &mut windows as *mut Vec<HWND> as LPARAM) };
        windows.into_iter().filter_map(app_of).collect()
    }

    pub(super) type RegisterSession = unsafe extern "system" fn(HWND, u32) -> i32;
    pub(super) type UnregisterSession = unsafe extern "system" fn(HWND) -> i32;

    /// Sitzungsmeldungen (Sperren, Entsperren) aus wtsapi32.dll. Die DLL wird
    /// erst hier geladen und nicht fest verknüpft: Sonst lüde Windows sie bei
    /// jedem Start von Hora mit, auch wenn die App-Zeiten aus sind.
    pub(super) fn session_api() -> Option<(RegisterSession, UnregisterSession)> {
        let name = wide("wtsapi32.dll");
        // SAFETY: nullterminierter Name; nur aus dem Systemordner laden
        let module = unsafe {
            LoadLibraryExW(
                name.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if module.is_null() {
            return None;
        }
        // SAFETY: Das Modul bleibt geladen (nie freigegeben); die Namen sind
        // nullterminiert und die Signaturen die aus der Windows-Doku.
        unsafe {
            let register =
                GetProcAddress(module, c"WTSRegisterSessionNotification".as_ptr().cast())?;
            let unregister =
                GetProcAddress(module, c"WTSUnRegisterSessionNotification".as_ptr().cast())?;
            Some((
                std::mem::transmute::<unsafe extern "system" fn() -> isize, RegisterSession>(
                    register,
                ),
                std::mem::transmute::<unsafe extern "system" fn() -> isize, UnregisterSession>(
                    unregister,
                ),
            ))
        }
    }

    struct State {
        handler: Rc<dyn Fn(WatchEvent)>,
        pen: Option<Arc<AtomicU64>>,
        hook: HWINEVENTHOOK,
        /// Abmelden der Sitzungsmeldungen, wenn die Anmeldung geklappt hat
        session: Option<UnregisterSession>,
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    fn emit(event: WatchEvent) {
        // Handler außerhalb der Ausleihe aufrufen
        let handler = STATE.with(|s| s.borrow().as_ref().map(|s| s.handler.clone()));
        if let Some(handler) = handler {
            handler(event);
        }
    }

    unsafe extern "system" fn on_win_event(
        _hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        object: i32,
        _child: i32,
        _thread: u32,
        _time: u32,
    ) {
        if event == EVENT_SYSTEM_FOREGROUND && object == OBJID_WINDOW {
            emit(WatchEvent::Foreground(app_of(hwnd)));
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_WTSSESSION_CHANGE => {
                match wparam as u32 {
                    WTS_SESSION_LOCK => emit(WatchEvent::Locked),
                    WTS_SESSION_UNLOCK => emit(WatchEvent::Unlocked),
                    _ => {}
                }
                0
            }
            WM_INPUT => {
                STATE.with(|s| {
                    if let Some(pen) = s.borrow().as_ref().and_then(|s| s.pen.as_ref()) {
                        pen.fetch_add(1, Ordering::Relaxed);
                    }
                });
                // Windows räumt die Eingabedaten dabei auf
                // SAFETY: unveränderte Nachricht an die Standardbehandlung
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_CLOSE => {
                // SAFETY: eigenes Fenster dieses Threads
                unsafe { DestroyWindow(hwnd) };
                0
            }
            WM_DESTROY => {
                if let Some(state) = STATE.with(|s| s.borrow_mut().take()) {
                    // SAFETY: Haken und Anmeldung stammen aus diesem Thread
                    unsafe {
                        UnhookWinEvent(state.hook);
                        if let Some(unregister) = state.session {
                            unregister(hwnd);
                        }
                    }
                }
                // SAFETY: beendet die Nachrichtenschleife dieses Threads
                unsafe { PostQuitMessage(0) };
                0
            }
            // SAFETY: unveränderte Nachricht an die Standardbehandlung
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    /// Unsichtbares Nachrichtenfenster für Sitzungs- und Stiftmeldungen
    fn create_window() -> HWND {
        let class = wide(WINDOW_CLASS);
        // SAFETY: Klasse und Fenster gehören diesem Thread; alle Zeiger zeigen
        // auf lebende Puffer. Eine schon registrierte Klasse ist kein Fehler.
        unsafe {
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
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                module,
                std::ptr::null(),
            )
        }
    }

    fn register_pen(hwnd: HWND) -> bool {
        let devices = DIGITIZER_USAGES.map(|usage| RAWINPUTDEVICE {
            usUsagePage: DIGITIZER_PAGE,
            usUsage: usage,
            // Auch melden, wenn ein anderes Programm vorn ist
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        });
        // SAFETY: Feld und Größenangaben passen zusammen
        unsafe {
            RegisterRawInputDevices(
                devices.as_ptr(),
                devices.len() as u32,
                std::mem::size_of::<RAWINPUTDEVICE>() as u32,
            ) != 0
        }
    }

    pub struct Watcher {
        hwnd: usize,
        thread: Option<std::thread::JoinHandle<()>>,
        pen: Arc<AtomicU64>,
    }

    impl Watcher {
        pub fn new(
            count_pen: bool,
            on_event: impl Fn(WatchEvent) + Send + 'static,
        ) -> Result<Watcher, PlatformError> {
            let (sender, receiver) = mpsc::channel::<Result<usize, String>>();
            let pen = Arc::new(AtomicU64::new(0));
            let counter = count_pen.then(|| pen.clone());
            let thread = std::thread::Builder::new()
                .name("app-zeiten".into())
                .spawn(move || {
                    let hwnd = create_window();
                    if hwnd.is_null() {
                        let _ =
                            sender.send(Err("Fenster für die App-Zeiten nicht anlegbar".into()));
                        return;
                    }
                    // SAFETY: Rückruf mit passender Signatur; die Meldungen
                    // kommen über die Nachrichtenschleife dieses Threads.
                    let hook = unsafe {
                        SetWinEventHook(
                            EVENT_SYSTEM_FOREGROUND,
                            EVENT_SYSTEM_FOREGROUND,
                            std::ptr::null_mut(),
                            Some(on_win_event),
                            0,
                            0,
                            WINEVENT_OUTOFCONTEXT,
                        )
                    };
                    if hook.is_null() {
                        // SAFETY: eigenes Fenster dieses Threads
                        unsafe { DestroyWindow(hwnd) };
                        let _ = sender.send(Err("Vordergrund nicht beobachtbar".into()));
                        return;
                    }
                    // Ohne Sitzungsmeldungen endet die Zählung beim Sperren
                    // trotzdem, nur erst mit der Pausengrenze
                    let session = session_api().and_then(|(register, unregister)| {
                        // SAFETY: eigenes Fenster dieses Threads
                        let ok = unsafe { register(hwnd, NOTIFY_FOR_THIS_SESSION) } != 0;
                        ok.then_some(unregister)
                    });
                    let counter = counter.filter(|_| register_pen(hwnd));
                    STATE.set(Some(State {
                        handler: Rc::new(on_event),
                        pen: counter,
                        hook,
                        session,
                    }));
                    let _ = sender.send(Ok(hwnd as usize));
                    emit(WatchEvent::Foreground(foreground_app()));
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
            Ok(Watcher {
                hwnd,
                thread: Some(thread),
                pen,
            })
        }

        pub fn pen_packets(&self) -> u64 {
            self.pen.load(Ordering::Relaxed)
        }
    }

    impl Drop for Watcher {
        fn drop(&mut self) {
            // SAFETY: Fenster des Beobachter-Threads; WM_CLOSE räumt dort auf
            unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{ForegroundApp, WatchEvent};
    use crate::PlatformError;
    use std::time::Duration;

    pub fn foreground_app() -> Option<ForegroundApp> {
        None
    }

    pub fn idle_time() -> Duration {
        Duration::ZERO
    }

    pub fn window_apps() -> Vec<ForegroundApp> {
        Vec::new()
    }

    pub struct Watcher;

    impl Watcher {
        pub fn new(
            _count_pen: bool,
            _on_event: impl Fn(WatchEvent) + Send + 'static,
        ) -> Result<Watcher, PlatformError> {
            Err(PlatformError("App-Zeiten nur unter Windows".into()))
        }

        pub fn pen_packets(&self) -> u64 {
            0
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn finds_the_exe_of_a_process() {
        let exe = imp::exe_of(std::process::id()).expect("eigener Prozess");
        assert_eq!(
            exe.canonicalize().unwrap(),
            std::env::current_exe().unwrap().canonicalize().unwrap()
        );
        assert_eq!(imp::exe_of(0), None);
    }

    #[test]
    fn open_apps_are_unique_and_without_hora() {
        let apps = open_apps();
        let own = std::process::id();
        assert!(apps.iter().all(|app| app.pid != own));
        let mut names: Vec<String> = apps.iter().map(|a| a.file_name().to_lowercase()).collect();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn session_messages_are_available() {
        assert!(imp::session_api().is_some());
    }

    #[test]
    fn idle_time_is_plausible() {
        // 49,7 Tage sind der größte Wert, den Windows liefern kann
        assert!(idle_time() < Duration::from_secs(50 * 24 * 3600));
    }

    #[test]
    fn watcher_reports_the_start_state_and_stops() {
        let (sender, receiver) = mpsc::channel();
        let watcher = Watcher::new(true, move |event| {
            let _ = sender.send(event);
        })
        .expect("Beobachter");
        let first = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("Ausgangszustand");
        assert!(matches!(first, WatchEvent::Foreground(_)), "{first:?}");
        drop(watcher);
        // Thread beendet: Der Sender ist mit dem Zustand weg
        loop {
            match receiver.recv_timeout(Duration::from_secs(5)) {
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => panic!("Beobachter läuft weiter"),
            }
        }
    }
}
