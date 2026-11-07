//! Windows-Benachrichtigungen (Toasts) für Erinnerungen.
//!
//! Hora ist ein Programm ohne Paket. Damit Windows seine Toasts annimmt,
//! meldet es eine eigene App-ID unter `HKCU\Software\Classes\AppUserModelId`
//! an (Anzeigename und Symbol), so wie es das Windows App SDK für Programme
//! ohne Paket macht. Ein Startmenü-Eintrag mit AUMID ist dafür nicht nötig.
//!
//! "Schlummern" und "Schließen" sind Systemaktionen: Die erledigt Windows
//! selbst. Ein Klick auf den Toast meldet sich bei Hora, solange es läuft.

#[cfg(not(windows))]
use std::path::Path;

#[cfg(not(windows))]
use crate::PlatformError;

/// App-ID der Benachrichtigungen
pub const AUMID: &str = "Hora.Kalender";

/// Ein Toast
#[derive(Debug, Clone, Default)]
pub struct Toast {
    pub title: String,
    /// Weitere Zeilen (Windows zeigt höchstens zwei)
    pub lines: Vec<String>,
    /// Kennung, um den Toast zu ersetzen oder zu entfernen (bis 64 Zeichen)
    pub tag: String,
    /// Kommt beim Klick auf den Toast zurück
    pub launch: String,
    /// Erinnerung: bleibt stehen, mit Schlummern und Schließen
    pub reminder: bool,
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() && c != '\n' => {}
            c => out.push(c),
        }
    }
    out
}

/// Toast-XML (Schema "ToastGeneric")
pub fn toast_xml(toast: &Toast) -> String {
    let mut xml = format!("<toast launch=\"{}\"", escape(&toast.launch));
    if toast.reminder {
        xml.push_str(" scenario=\"reminder\"");
    }
    xml.push_str("><visual><binding template=\"ToastGeneric\">");
    xml.push_str(&format!("<text>{}</text>", escape(&toast.title)));
    for line in toast.lines.iter().take(2) {
        xml.push_str(&format!("<text>{}</text>", escape(line)));
    }
    xml.push_str("</binding></visual>");
    if toast.reminder {
        xml.push_str(concat!(
            "<actions>",
            "<input id=\"snooze\" type=\"selection\" defaultInput=\"5\">",
            "<selection id=\"5\" content=\"5 Minuten\"/>",
            "<selection id=\"10\" content=\"10 Minuten\"/>",
            "<selection id=\"15\" content=\"15 Minuten\"/>",
            "<selection id=\"30\" content=\"30 Minuten\"/>",
            "<selection id=\"60\" content=\"1 Stunde\"/>",
            "</input>",
            "<action activationType=\"system\" arguments=\"snooze\" hint-inputId=\"snooze\" content=\"Schlummern\"/>",
            "<action activationType=\"system\" arguments=\"dismiss\" content=\"Schließen\"/>",
            "</actions>",
            "<audio src=\"ms-winsoundevent:Notification.Reminder\"/>",
        ));
    }
    xml.push_str("</toast>");
    xml
}

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::path::Path;

    use super::{Toast, toast_xml};
    use crate::{PlatformError, registry};
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{
        NotificationSetting, ToastActivatedEventArgs, ToastNotification, ToastNotificationManager,
        ToastNotifier,
    };
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::core::{HSTRING, IInspectable, Interface};

    impl From<windows::core::Error> for PlatformError {
        fn from(e: windows::core::Error) -> Self {
            PlatformError(format!("{} (0x{:08x})", e.message(), e.code().0))
        }
    }

    /// Schlüssel der App-ID-Anmeldung
    fn key(aumid: &str) -> String {
        format!(r"Software\Classes\AppUserModelId\{aumid}")
    }

    pub struct Notifier {
        aumid: HSTRING,
        notifier: ToastNotifier,
        /// Gezeigte Toasts, damit ihr Klick-Ereignis ankommt
        alive: RefCell<VecDeque<ToastNotification>>,
    }

    impl Notifier {
        pub fn new(
            aumid: &str,
            display_name: &str,
            icon: Option<&Path>,
        ) -> Result<Notifier, PlatformError> {
            let key = key(aumid);
            registry::set_string(&key, "DisplayName", display_name)?;
            if let Some(icon) = icon {
                registry::set_string(&key, "IconUri", &icon.display().to_string())?;
            }
            // SAFETY: Initialisiert COM für diesen Thread; ist es schon (auch
            // in anderem Modus) initialisiert, ändert der Aufruf nichts.
            let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            let aumid = HSTRING::from(aumid);
            let notifier = ToastNotificationManager::CreateToastNotifierWithId(&aumid)?;
            Ok(Notifier {
                aumid,
                notifier,
                alive: RefCell::new(VecDeque::new()),
            })
        }

        /// Hat der Nutzer Benachrichtigungen für Hora (oder ganz) ausgeschaltet?
        pub fn blocked(&self) -> Option<String> {
            match self.notifier.Setting().ok()? {
                NotificationSetting::Enabled => None,
                NotificationSetting::DisabledForApplication => Some(
                    "Benachrichtigungen für Hora sind in den Windows-Einstellungen ausgeschaltet."
                        .into(),
                ),
                NotificationSetting::DisabledForUser => Some(
                    "Benachrichtigungen sind in den Windows-Einstellungen ausgeschaltet.".into(),
                ),
                _ => Some("Benachrichtigungen sind durch eine Richtlinie gesperrt.".into()),
            }
        }

        pub fn show(
            &self,
            toast: &Toast,
            on_activate: impl Fn(String) + Send + 'static,
        ) -> Result<(), PlatformError> {
            let doc = XmlDocument::new()?;
            doc.LoadXml(&HSTRING::from(toast_xml(toast)))?;
            let notification = ToastNotification::CreateToastNotification(&doc)?;
            if !toast.tag.is_empty() {
                notification.SetTag(&HSTRING::from(toast.tag.as_str()))?;
            }
            notification.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
                move |_, args| {
                    if let Some(args) = args.as_ref()
                        && let Ok(args) = args.cast::<ToastActivatedEventArgs>()
                    {
                        on_activate(args.Arguments()?.to_string());
                    }
                    Ok(())
                },
            ))?;
            self.notifier.Show(&notification)?;
            let mut alive = self.alive.borrow_mut();
            alive.push_back(notification);
            while alive.len() > 32 {
                alive.pop_front();
            }
            Ok(())
        }

        /// Toast mit dieser Kennung aus dem Infocenter entfernen
        pub fn remove(&self, tag: &str) {
            if let Ok(history) = ToastNotificationManager::History() {
                let _ = history.RemoveGroupedTagWithId(
                    &HSTRING::from(tag),
                    &HSTRING::new(),
                    &self.aumid,
                );
            }
        }

        /// Kennungen der Toasts von Hora im Infocenter
        pub fn delivered(&self) -> Vec<String> {
            let Ok(history) = ToastNotificationManager::History() else {
                return Vec::new();
            };
            let Ok(list) = history.GetHistoryWithId(&self.aumid) else {
                return Vec::new();
            };
            list.into_iter()
                .filter_map(|t| t.Tag().ok())
                .map(|t| t.to_string())
                .collect()
        }
    }

    /// App-ID-Anmeldung wieder entfernen (Deinstallation, Tests)
    pub fn unregister(aumid: &str) -> Result<(), PlatformError> {
        registry::delete_key(&key(aumid))
    }
}

#[cfg(windows)]
pub use imp::{Notifier, unregister};

#[cfg(not(windows))]
pub struct Notifier;

#[cfg(not(windows))]
impl Notifier {
    pub fn new(
        _aumid: &str,
        _display_name: &str,
        _icon: Option<&Path>,
    ) -> Result<Notifier, PlatformError> {
        Err(PlatformError("Benachrichtigungen nur unter Windows".into()))
    }
    pub fn blocked(&self) -> Option<String> {
        None
    }
    pub fn show(
        &self,
        _toast: &Toast,
        _on_activate: impl Fn(String) + Send + 'static,
    ) -> Result<(), PlatformError> {
        Ok(())
    }
    pub fn remove(&self, _tag: &str) {}
    pub fn delivered(&self) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escapes_text_and_offers_snooze() {
        let xml = toast_xml(&Toast {
            title: "Zahnarzt <Dr. Weber> & Co".into(),
            lines: vec![
                "in 15 Minuten · 15:00–16:00".into(),
                "Praxis \"Mitte\"".into(),
                "zu viel".into(),
            ],
            tag: "abc".into(),
            launch: "open|1|/a.ics".into(),
            reminder: true,
        });
        assert!(xml.starts_with("<toast launch=\"open|1|/a.ics\" scenario=\"reminder\">"));
        assert!(xml.contains("<text>Zahnarzt &lt;Dr. Weber&gt; &amp; Co</text>"));
        assert!(xml.contains("<text>Praxis &quot;Mitte&quot;</text>"));
        assert!(!xml.contains("zu viel"));
        assert!(xml.contains("arguments=\"snooze\" hint-inputId=\"snooze\""));
        let plain = toast_xml(&Toast {
            title: "Test".into(),
            ..Toast::default()
        });
        assert!(!plain.contains("scenario") && !plain.contains("<actions>"));
    }

    #[test]
    #[cfg(windows)]
    #[ignore = "zeigt eine echte Windows-Benachrichtigung und meldet eine Test-App-ID in HKCU an"]
    fn toast_reaches_the_action_center() {
        let aumid = format!("Hora.Test{}", std::process::id());
        let notifier = Notifier::new(&aumid, "Hora Test", None).expect("Anmeldung");
        if let Some(blocked) = notifier.blocked() {
            panic!("{blocked}");
        }
        let toast = Toast {
            title: "Hora-Test".into(),
            lines: vec!["Kann weg".into()],
            tag: "t1".into(),
            ..Toast::default()
        };
        notifier.show(&toast, |_| {}).expect("Toast zeigen");
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert_eq!(notifier.delivered(), vec!["t1".to_string()]);
        notifier.remove("t1");
        unregister(&aumid).unwrap();
    }
}
