//! Termine ändern und neu anlegen. Geändert werden nur die betroffenen
//! Eigenschaften; alles andere im Objekt (Alarme, Teilnehmer, Eigenschaften
//! anderer Programme, VTIMEZONE) bleibt, wie es war.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;

use crate::ical::{Component, Property, escape_text};
use crate::model::CalendarObject;
use crate::time::{DateTimeValue, resolve_tzid};
use crate::vtimezone::vtimezone;

pub const PRODID: &str = "-//Hora//Hora Kalender//DE";

#[derive(Debug, Clone, PartialEq)]
pub struct EventFields {
    pub summary: String,
    pub location: String,
    pub description: String,
    pub start: DateTimeValue,
    /// Ende; bei ganztägigen Terminen der Tag nach dem letzten
    pub end: DateTimeValue,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("{}", hora_i18n::tr!("Item {} not found", .0))]
    NotFound(String),
}

pub(crate) fn utc_stamp(now: DateTime<Utc>) -> String {
    now.format("%Y%m%dT%H%M%SZ").to_string()
}

/// DTSTART/DTEND/… für einen Wert. Fremde Parameter einer bestehenden
/// Eigenschaft bleiben erhalten, TZID und VALUE werden neu gesetzt.
pub fn time_property(name: &str, value: &DateTimeValue, existing: Option<&Property>) -> Property {
    let mut property = existing.cloned().unwrap_or_else(|| Property::new(name, ""));
    property.name = name.to_string();
    property.remove_param("TZID");
    property.remove_param("VALUE");
    property.value = match value {
        DateTimeValue::Date(d) => {
            property.set_param("VALUE", "DATE");
            d.format("%Y%m%d").to_string()
        }
        DateTimeValue::Floating(n) => n.format("%Y%m%dT%H%M%S").to_string(),
        DateTimeValue::Utc(n) => n.format("%Y%m%dT%H%M%SZ").to_string(),
        DateTimeValue::Zoned(n, tz) => {
            property.set_param("TZID", tz.name());
            n.format("%Y%m%dT%H%M%S").to_string()
        }
    };
    property
}

fn master_mut<'a>(
    object: &'a mut CalendarObject,
    uid: &str,
) -> Result<&'a mut Component, EditError> {
    object
        .root
        .components
        .iter_mut()
        .find(|c| {
            c.is("VEVENT")
                && c.property("RECURRENCE-ID").is_none()
                && c.property("UID").is_some_and(|p| p.value.trim() == uid)
        })
        .ok_or_else(|| EditError::NotFound(uid.to_string()))
}

/// Textfeld setzen; leer entfernt die Eigenschaft. Unveränderter Text lässt
/// die Eigenschaft samt Parametern (z. B. LANGUAGE) unangetastet.
pub(crate) fn set_text(event: &mut Component, name: &str, text: &str) {
    if text.trim().is_empty() {
        event.remove_properties(name);
        return;
    }
    if event.property(name).is_some_and(|p| p.text_value() == text) {
        return;
    }
    let mut property = event
        .property(name)
        .cloned()
        .unwrap_or_else(|| Property::new(name, ""));
    property.value = escape_text(text);
    event.set_property(property);
}

pub(crate) fn write_times(event: &mut Component, start: &DateTimeValue, end: &DateTimeValue) {
    let dtstart = time_property("DTSTART", start, event.property("DTSTART"));
    event.set_property(dtstart);
    event.remove_properties("DURATION");
    let dtend = time_property("DTEND", end, event.property("DTEND"));
    event.set_property(dtend);
}

/// DTSTAMP und LAST-MODIFIED auf jetzt, SEQUENCE eins höher.
pub(crate) fn touch(event: &mut Component, now: DateTime<Utc>) {
    let stamp = utc_stamp(now);
    event.set_property(Property::new("DTSTAMP", stamp.clone()));
    event.set_property(Property::new("LAST-MODIFIED", stamp));
    let sequence = event
        .property("SEQUENCE")
        .and_then(|p| p.value.trim().parse::<u32>().ok())
        .unwrap_or(0);
    event.set_property(Property::new("SEQUENCE", (sequence + 1).to_string()));
}

/// Ergänzt eine VTIMEZONE für jede TZID, die eine IANA-Zone nennt und noch
/// keine hat (RFC 5545, Abschnitt 3.6.5).
pub(crate) fn ensure_timezones(object: &mut CalendarObject) {
    let mut needed: BTreeMap<String, (Tz, i32, i32)> = BTreeMap::new();
    for component in object.root.components.iter().filter(|c| !c.is("VTIMEZONE")) {
        for property in &component.properties {
            let Some(tzid) = property.param("TZID") else {
                continue;
            };
            let Some(tz) = resolve_tzid(tzid).filter(|tz| tz.name() == tzid) else {
                continue;
            };
            let year = property
                .value
                .get(..4)
                .and_then(|y| y.parse::<i32>().ok())
                .unwrap_or(1970);
            let entry = needed.entry(tzid.to_string()).or_insert((tz, year, year));
            entry.1 = entry.1.min(year);
            entry.2 = entry.2.max(year);
        }
    }
    for (tzid, (tz, first, last)) in needed {
        let present = object
            .root
            .children_named("VTIMEZONE")
            .any(|v| v.property("TZID").is_some_and(|p| p.value.trim() == tzid));
        if !present {
            // Folgejahr dazu, damit Termine bis Silvester abgedeckt sind
            object
                .root
                .components
                .insert(0, vtimezone(tz, first, last + 1));
        }
    }
}

/// Beginn und Ende des Termins `uid` setzen (bei Serien: der ganzen Serie).
pub fn set_times(
    object: &mut CalendarObject,
    uid: &str,
    start: DateTimeValue,
    end: DateTimeValue,
    now: DateTime<Utc>,
) -> Result<(), EditError> {
    let event = master_mut(object, uid)?;
    write_times(event, &start, &end);
    touch(event, now);
    ensure_timezones(object);
    Ok(())
}

/// Titel, Ort und Beschreibung setzen, Zeiten unverändert lassen.
pub fn set_texts(
    object: &mut CalendarObject,
    uid: &str,
    summary: &str,
    location: &str,
    description: &str,
    now: DateTime<Utc>,
) -> Result<(), EditError> {
    let event = master_mut(object, uid)?;
    set_text(event, "SUMMARY", summary);
    set_text(event, "LOCATION", location);
    set_text(event, "DESCRIPTION", description);
    touch(event, now);
    Ok(())
}

/// Alle Felder eines bestehenden Termins setzen.
pub fn update_event(
    object: &mut CalendarObject,
    uid: &str,
    fields: &EventFields,
    now: DateTime<Utc>,
) -> Result<(), EditError> {
    let event = master_mut(object, uid)?;
    set_text(event, "SUMMARY", &fields.summary);
    set_text(event, "LOCATION", &fields.location);
    set_text(event, "DESCRIPTION", &fields.description);
    write_times(event, &fields.start, &fields.end);
    touch(event, now);
    ensure_timezones(object);
    Ok(())
}

/// Neues Kalenderobjekt mit einem Termin.
pub fn new_event(uid: &str, fields: &EventFields, now: DateTime<Utc>) -> CalendarObject {
    let mut root = Component::new("VCALENDAR");
    root.properties.push(Property::new("VERSION", "2.0"));
    root.properties.push(Property::new("PRODID", PRODID));
    root.properties.push(Property::new("CALSCALE", "GREGORIAN"));
    let stamp = utc_stamp(now);
    let mut event = Component::new("VEVENT");
    for (name, value) in [
        ("UID", uid),
        ("DTSTAMP", &stamp),
        ("CREATED", &stamp),
        ("LAST-MODIFIED", &stamp),
    ] {
        event.properties.push(Property::new(name, value));
    }
    event.properties.push(Property::new("SEQUENCE", "0"));
    set_text(&mut event, "SUMMARY", &fields.summary);
    set_text(&mut event, "LOCATION", &fields.location);
    set_text(&mut event, "DESCRIPTION", &fields.description);
    write_times(&mut event, &fields.start, &fields.end);
    root.components.push(event);
    let mut object = CalendarObject { root };
    ensure_timezones(&mut object);
    object
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveDateTime, TimeZone};
    use chrono_tz::Europe::Berlin;

    fn naive(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 4, 12, 0, 0).unwrap()
    }

    const FOREIGN: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Mozilla.org//DE\r\n\
BEGIN:VEVENT\r\nUID:t1\r\nDTSTAMP:20261101T080000Z\r\nDTSTART;TZID=America/New_York;X-FOO=bar:20261109T100000\r\n\
DURATION:PT1H\r\nSUMMARY;LANGUAGE=de:Call\r\nSEQUENCE:4\r\nATTENDEE;CN=\"Müller, Anna\":mailto:a@example.org\r\n\
X-MOZ-GENERATION:7\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn moving_keeps_everything_else_and_the_events_zone() {
        let mut object = CalendarObject::parse(FOREIGN).unwrap();
        let ny: Tz = "America/New_York".parse().unwrap();
        set_times(
            &mut object,
            "t1",
            DateTimeValue::Zoned(naive("2026-11-10 11:00"), ny),
            DateTimeValue::Zoned(naive("2026-11-10 12:30"), ny),
            now(),
        )
        .unwrap();
        let ics = object.to_ics();
        assert!(
            ics.contains("DTSTART;TZID=America/New_York;X-FOO=bar:20261110T110000\r\n")
                || ics.contains("DTSTART;X-FOO=bar;TZID=America/New_York:20261110T110000\r\n")
        );
        assert!(ics.contains("DTEND;TZID=America/New_York:20261110T123000\r\n"));
        assert!(!ics.contains("DURATION"));
        assert!(ics.contains("SEQUENCE:5\r\n"));
        assert!(ics.contains("DTSTAMP:20261104T120000Z\r\n"));
        for kept in [
            "X-MOZ-GENERATION:7",
            "ATTENDEE;CN=\"Müller, Anna\":mailto:a@example.org",
            "TRIGGER:-PT15M",
            "SUMMARY;LANGUAGE=de:Call",
        ] {
            assert!(ics.contains(kept), "{kept} fehlt in\n{ics}");
        }
        // VTIMEZONE für New York ergänzt
        assert!(ics.contains("BEGIN:VTIMEZONE\r\nTZID:America/New_York\r\n"));
        let (events, errors) = CalendarObject::parse(&ics).unwrap().events();
        assert!(errors.is_empty());
        assert_eq!(
            events[0].start,
            DateTimeValue::Zoned(naive("2026-11-10 11:00"), ny)
        );
    }

    #[test]
    fn texts_are_escaped_and_empty_ones_removed() {
        let mut object = CalendarObject::parse(FOREIGN).unwrap();
        set_texts(
            &mut object,
            "t1",
            "Call, verschoben; neu",
            "",
            "Zeile 1\nZeile 2",
            now(),
        )
        .unwrap();
        let ics = object.to_ics();
        assert!(ics.contains("SUMMARY;LANGUAGE=de:Call\\, verschoben\\; neu\r\n"));
        assert!(ics.contains("DESCRIPTION:Zeile 1\\nZeile 2\r\n"));
        assert!(!ics.contains("LOCATION"));
        let (events, _) = CalendarObject::parse(&ics).unwrap().events();
        assert_eq!(events[0].summary, "Call, verschoben; neu");
        assert_eq!(events[0].description, "Zeile 1\nZeile 2");
    }

    #[test]
    fn unknown_uid_is_an_error() {
        let mut object = CalendarObject::parse(FOREIGN).unwrap();
        assert_eq!(
            set_texts(&mut object, "gibt-es-nicht", "x", "", "", now()),
            Err(EditError::NotFound("gibt-es-nicht".into()))
        );
    }

    #[test]
    fn only_the_series_master_changes() {
        let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:s\r\nDTSTART:20261102T070000Z\r\nDTEND:20261102T080000Z\r\nRRULE:FREQ=DAILY\r\nSUMMARY:Alt\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:s\r\nRECURRENCE-ID:20261103T070000Z\r\nDTSTART:20261103T090000Z\r\nDTEND:20261103T100000Z\r\nSUMMARY:Einzeln\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let mut object = CalendarObject::parse(ics).unwrap();
        set_texts(&mut object, "s", "Neu", "", "", now()).unwrap();
        let (events, _) = object.events();
        assert_eq!(events[0].summary, "Neu");
        assert_eq!(events[1].summary, "Einzeln");
    }

    #[test]
    fn new_event_with_zone_gets_vtimezone_and_reads_back() {
        let fields = EventFields {
            summary: "Zahnarzt".into(),
            location: "Praxis Dr. Weber".into(),
            description: String::new(),
            start: DateTimeValue::Zoned(naive("2026-11-10 15:00"), Berlin),
            end: DateTimeValue::Zoned(naive("2026-11-10 16:00"), Berlin),
        };
        let object = new_event("neu-1", &fields, now());
        let ics = object.to_ics();
        assert!(ics.starts_with(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Hora//Hora Kalender//DE\r\n"
        ));
        assert!(ics.contains("BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\n"));
        assert!(ics.contains("DTSTART;TZID=Europe/Berlin:20261110T150000\r\n"));
        let back = CalendarObject::parse(&ics).unwrap();
        assert_eq!(back.uid().as_deref(), Some("neu-1"));
        let (events, errors) = back.events();
        assert!(errors.is_empty());
        assert_eq!(events[0].summary, "Zahnarzt");
        assert_eq!(events[0].start, fields.start);
        assert_eq!(events[0].end, fields.end);
    }

    #[test]
    fn all_day_event_uses_value_date() {
        let day = NaiveDate::from_ymd_opt(2026, 12, 24).unwrap();
        let fields = EventFields {
            summary: "Heiligabend".into(),
            location: String::new(),
            description: String::new(),
            start: DateTimeValue::Date(day),
            end: DateTimeValue::Date(day.succ_opt().unwrap()),
        };
        let ics = new_event("x", &fields, now()).to_ics();
        assert!(ics.contains("DTSTART;VALUE=DATE:20261224\r\n"));
        assert!(ics.contains("DTEND;VALUE=DATE:20261225\r\n"));
        assert!(!ics.contains("VTIMEZONE"));
    }
}
