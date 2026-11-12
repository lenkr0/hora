//! Datenmodelle, iCalendar lesen/schreiben (VEVENT, VTODO, VJOURNAL),
//! Wiederholungen und Zeitzonen.
//!
//! - [`ical`]: verlustfreier Parser und Serialisierer (RFC 5545). Unbekannte
//!   Eigenschaften und Komponenten bleiben unverändert erhalten.
//! - [`time`]: Datums- und Zeitwerte mit TZID-Auflösung und den Regeln aus
//!   RFC 5545 für Sommerzeit-Lücken und doppelte Stunden.
//! - [`model`]: typisierte Sicht auf Termine eines Kalenderobjekts.
//! - [`expand`]: Vorkommen eines Termins in einem Zeitraum (RRULE, RDATE,
//!   EXDATE, RECURRENCE-ID).
//! - [`quick`]: Schnelleingabe auf Deutsch und Englisch ("Zahnarzt morgen 15 Uhr").
//! - [`recur`]: Wiederholungsregeln lesen, schreiben und anpassen.
//! - [`series`]: Serien bearbeiten (nur dieser, alle folgenden, alle).
//! - [`alarm`]: Erinnerungen (VALARM) lesen, schreiben und fällige berechnen.
//! - [`task`]: Aufgaben (VTODO) mit Teilaufgaben und wiederkehrenden Aufgaben.
//! - [`journal`]: Journaleinträge (VJOURNAL) mit Markdown-Text und Tags.
//! - [`edit`]: Termine ändern und anlegen, mit passender VTIMEZONE
//!   ([`vtimezone`]).
//! - [`activity`]: App-Zeiten, also aktive Zeit einer Anwendung im Vordergrund.

pub mod activity;
pub mod alarm;
pub mod duration;
pub mod edit;
pub mod expand;
pub mod ical;
pub mod journal;
pub mod model;
pub mod quick;
pub mod recur;
pub mod series;
pub mod task;
pub mod time;
pub mod vtimezone;

pub use chrono_tz::Tz;
