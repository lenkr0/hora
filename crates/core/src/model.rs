//! Typisierte Sicht auf ein Kalenderobjekt (eine `.ics`-Ressource auf dem
//! Server). Das Objekt selbst bleibt als [`Component`] erhalten; die Sicht
//! liest nur die Eigenschaften, die Hora braucht.

use chrono::{DateTime, Duration, Utc};

use crate::alarm::{Alarm, alarms_of};

use crate::duration::parse_duration;
use crate::ical::{self, Component, ParseError, split_values};
use crate::time::{DateTimeValue, TimeError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Event,
    Todo,
    Journal,
}

impl Kind {
    pub fn component_name(self) -> &'static str {
        match self {
            Kind::Event => "VEVENT",
            Kind::Todo => "VTODO",
            Kind::Journal => "VJOURNAL",
        }
    }
}

/// Eine `.ics`-Ressource: ein VCALENDAR mit einem Termin (bzw. Aufgabe,
/// Journal) samt geänderten Einzelterminen und Zeitzonen.
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarObject {
    pub root: Component,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("{0}")]
    Parse(#[from] ParseError),
    #[error("{0}")]
    Time(#[from] TimeError),
    #[error("{}", hora_i18n::tr!("{} without {}", component, property))]
    Missing {
        component: &'static str,
        property: &'static str,
    },
}

/// Ein VEVENT, aufbereitet für Anzeige und Expansion.
#[derive(Debug, Clone, PartialEq)]
pub struct EventInfo {
    pub uid: String,
    pub summary: String,
    pub location: String,
    pub description: String,
    pub start: DateTimeValue,
    /// Ende; fehlt DTEND, aus DURATION bzw. nach RFC 5545 berechnet
    pub end: DateTimeValue,
    pub rrule: Option<String>,
    pub rdates: Vec<DateTimeValue>,
    pub exdates: Vec<DateTimeValue>,
    pub recurrence_id: Option<DateTimeValue>,
    pub cancelled: bool,
    pub alarms: Vec<Alarm>,
    /// Erinnerungen bis hierhin weggeklickt (Thunderbird: X-MOZ-LASTACK)
    pub last_ack: Option<DateTime<Utc>>,
}

impl CalendarObject {
    pub fn parse(ics: &str) -> Result<Self, ModelError> {
        Ok(CalendarObject {
            root: ical::parse_calendar(ics)?,
        })
    }

    pub fn to_ics(&self) -> String {
        ical::serialize(&self.root)
    }

    /// Art des Objekts nach der ersten Komponente, die keine Zeitzone ist.
    pub fn kind(&self) -> Option<Kind> {
        self.root.components.iter().find_map(|c| {
            [Kind::Event, Kind::Todo, Kind::Journal]
                .into_iter()
                .find(|k| c.is(k.component_name()))
        })
    }

    pub fn uid(&self) -> Option<String> {
        self.root
            .components
            .iter()
            .find(|c| !c.is("VTIMEZONE"))
            .and_then(|c| c.property("UID"))
            .map(|p| p.value.trim().to_string())
    }

    /// Alle VEVENTs; fehlerhafte werden übersprungen und als Fehler gemeldet.
    pub fn events(&self) -> (Vec<EventInfo>, Vec<ModelError>) {
        let mut events = Vec::new();
        let mut errors = Vec::new();
        for component in self.root.children_named("VEVENT") {
            match EventInfo::from_component(component, &self.root) {
                Ok(event) => events.push(event),
                Err(e) => errors.push(e),
            }
        }
        (events, errors)
    }
}

fn date_list(
    component: &Component,
    name: &str,
    calendar: &Component,
) -> Result<Vec<DateTimeValue>, TimeError> {
    let mut values = Vec::new();
    for property in component.properties_named(name) {
        // PERIOD-Werte (RDATE;VALUE=PERIOD) werden noch nicht unterstützt
        if property
            .param("VALUE")
            .is_some_and(|v| v.eq_ignore_ascii_case("PERIOD"))
        {
            continue;
        }
        for part in split_values(&property.value) {
            let mut single = property.clone();
            single.value = part.trim().to_string();
            values.push(DateTimeValue::from_property(&single, Some(calendar))?);
        }
    }
    Ok(values)
}

impl EventInfo {
    pub fn from_component(event: &Component, calendar: &Component) -> Result<Self, ModelError> {
        let missing = |property| ModelError::Missing {
            component: "VEVENT",
            property,
        };
        let start_prop = event.property("DTSTART").ok_or(missing("DTSTART"))?;
        let start = DateTimeValue::from_property(start_prop, Some(calendar))?;
        let end = match (event.property("DTEND"), event.property("DURATION")) {
            (Some(end), _) => DateTimeValue::from_property(end, Some(calendar))?,
            (None, Some(duration)) => add(
                start,
                parse_duration(&duration.value).unwrap_or_else(Duration::zero),
            ),
            // RFC 5545: ganztägig ohne Ende = ein Tag, sonst ohne Dauer
            (None, None) if start.is_date() => add(start, Duration::days(1)),
            (None, None) => start,
        };
        Ok(EventInfo {
            uid: event
                .property("UID")
                .map(|p| p.value.trim().to_string())
                .ok_or(missing("UID"))?,
            summary: event.text_of("SUMMARY"),
            location: event.text_of("LOCATION"),
            description: event.text_of("DESCRIPTION"),
            start,
            end,
            rrule: event.property("RRULE").map(|p| p.value.trim().to_string()),
            rdates: date_list(event, "RDATE", calendar)?,
            exdates: date_list(event, "EXDATE", calendar)?,
            recurrence_id: event
                .property("RECURRENCE-ID")
                .map(|p| DateTimeValue::from_property(p, Some(calendar)))
                .transpose()?,
            cancelled: event
                .property("STATUS")
                .is_some_and(|p| p.value.trim().eq_ignore_ascii_case("CANCELLED")),
            alarms: alarms_of(event),
            last_ack: event
                .property("X-MOZ-LASTACK")
                .and_then(|p| DateTimeValue::from_property(p, None).ok())
                .map(|v| v.to_utc(chrono_tz::UTC)),
        })
    }

    pub fn all_day(&self) -> bool {
        self.start.is_date()
    }

    pub fn is_recurring(&self) -> bool {
        self.rrule.is_some() || !self.rdates.is_empty()
    }
}

/// Dauer zu einem Wert addieren, Art (ganztägig, Zone …) bleibt gleich.
fn add(value: DateTimeValue, duration: Duration) -> DateTimeValue {
    match value {
        DateTimeValue::Date(d) => DateTimeValue::Date(d + Duration::days(duration.num_days())),
        DateTimeValue::Floating(n) => DateTimeValue::Floating(n + duration),
        DateTimeValue::Utc(n) => DateTimeValue::Utc(n + duration),
        DateTimeValue::Zoned(n, tz) => DateTimeValue::Zoned(n + duration, tz),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveDateTime};
    use chrono_tz::Europe::Berlin;

    fn obj(body: &str) -> CalendarObject {
        CalendarObject::parse(&format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}END:VCALENDAR\r\n"
        ))
        .unwrap()
    }

    fn naive(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn reads_event_fields() {
        let o = obj(
            "BEGIN:VEVENT\r\nUID:a1\r\nDTSTART;TZID=Europe/Berlin:20261109T080000\r\nDTEND;TZID=Europe/Berlin:20261109T093000\r\nSUMMARY:Mathe\r\nLOCATION:Raum 204\r\nRRULE:FREQ=WEEKLY;BYDAY=MO\r\nEXDATE;TZID=Europe/Berlin:20261116T080000,20261123T080000\r\nEND:VEVENT\r\n",
        );
        assert_eq!(o.kind(), Some(Kind::Event));
        assert_eq!(o.uid().as_deref(), Some("a1"));
        let (events, errors) = o.events();
        assert!(errors.is_empty());
        let e = &events[0];
        assert_eq!(e.summary, "Mathe");
        assert_eq!(
            e.start,
            DateTimeValue::Zoned(naive("2026-11-09 08:00"), Berlin)
        );
        assert_eq!(
            e.end,
            DateTimeValue::Zoned(naive("2026-11-09 09:30"), Berlin)
        );
        assert_eq!(e.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO"));
        assert_eq!(e.exdates.len(), 2);
        assert!(e.is_recurring());
    }

    #[test]
    fn end_from_duration_or_defaults() {
        let o = obj(
            "BEGIN:VEVENT\r\nUID:d\r\nDTSTART:20261109T080000Z\r\nDURATION:PT45M\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:e\r\nDTSTART;VALUE=DATE:20261224\r\nEND:VEVENT\r\n",
        );
        let (events, _) = o.events();
        assert_eq!(events[0].end, DateTimeValue::Utc(naive("2026-11-09 08:45")));
        assert_eq!(
            events[1].end,
            DateTimeValue::Date(NaiveDate::from_ymd_opt(2026, 12, 25).unwrap())
        );
        assert!(events[1].all_day());
    }

    #[test]
    fn skips_broken_events_but_keeps_others() {
        let o = obj(
            "BEGIN:VEVENT\r\nUID:ok\r\nDTSTART:20261109T080000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:kaputt\r\nSUMMARY:ohne Beginn\r\nEND:VEVENT\r\n",
        );
        let (events, errors) = o.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            errors,
            vec![ModelError::Missing {
                component: "VEVENT",
                property: "DTSTART"
            }]
        );
    }

    #[test]
    fn recognises_todos_and_journals() {
        assert_eq!(
            obj("BEGIN:VTODO\r\nUID:t\r\nEND:VTODO\r\n").kind(),
            Some(Kind::Todo)
        );
        assert_eq!(
            obj("BEGIN:VJOURNAL\r\nUID:j\r\nEND:VJOURNAL\r\n").kind(),
            Some(Kind::Journal)
        );
    }
}
