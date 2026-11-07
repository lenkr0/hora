//! Erinnerungen (VALARM, RFC 5545 Abschnitt 3.6.6): lesen, Auslösezeiten
//! für die Vorkommen eines Zeitraums berechnen und für den Termin-Dialog als
//! "Minuten vor Beginn" schreiben (wie Calino).
//!
//! Alarme, die der Dialog nicht darstellt (E-Mail, absolute Zeit, relativ
//! zum Ende), bleiben beim Bearbeiten unverändert. Was ein anderes Gerät
//! schon weggeklickt hat (`ACKNOWLEDGED` nach RFC 9074, Thunderbirds
//! `X-MOZ-LASTACK`), meldet Hora nicht noch einmal.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;

use crate::duration::parse_duration;
use crate::expand::{Occurrence, expand};
use crate::ical::{Component, Property, escape_text};
use crate::model::{CalendarObject, EventInfo};
use crate::time::DateTimeValue;

/// So weit vor einem Termin darf eine Erinnerung liegen (Calino: 4 Wochen)
pub const MAX_BEFORE: Duration = Duration::days(29);
/// So weit nach dem Beginn (z. B. "am Tag um 9 Uhr" bei ganztägigen Terminen)
pub const MAX_AFTER: Duration = Duration::days(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// Abstand zum Beginn bzw. bei `from_end` zum Ende; negativ = vorher
    Relative {
        offset: Duration,
        from_end: bool,
    },
    Absolute(DateTime<Utc>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alarm {
    /// Position unter den VALARM-Komponenten des Termins
    pub index: usize,
    pub action: String,
    pub trigger: Trigger,
    /// Bereits weggeklickt bis zu diesem Zeitpunkt (ACKNOWLEDGED)
    pub acknowledged: Option<DateTime<Utc>>,
}

fn utc_of(property: &Property) -> Option<DateTime<Utc>> {
    Some(
        DateTimeValue::from_property(property, None)
            .ok()?
            .to_utc(Tz::UTC),
    )
}

impl Alarm {
    pub fn from_component(index: usize, alarm: &Component) -> Option<Alarm> {
        let property = alarm.property("TRIGGER")?;
        let absolute = property
            .param("VALUE")
            .is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME"));
        let trigger = if absolute {
            Trigger::Absolute(utc_of(property)?)
        } else {
            Trigger::Relative {
                offset: parse_duration(&property.value)?,
                from_end: property
                    .param("RELATED")
                    .is_some_and(|v| v.eq_ignore_ascii_case("END")),
            }
        };
        Some(Alarm {
            index,
            action: alarm.text_of("ACTION").trim().to_ascii_uppercase(),
            trigger,
            acknowledged: alarm.property("ACKNOWLEDGED").and_then(utc_of),
        })
    }

    /// Zeigt eine Benachrichtigung an (DISPLAY, AUDIO). E-Mail-Alarme
    /// verschickt, wenn überhaupt, der Server.
    pub fn notifies(&self) -> bool {
        matches!(self.action.as_str(), "DISPLAY" | "AUDIO")
    }

    /// Zeitpunkt für ein Vorkommen von `start` bis `end`
    pub fn fires_at(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> DateTime<Utc> {
        match self.trigger {
            Trigger::Relative { offset, from_end } => (if from_end { end } else { start }) + offset,
            Trigger::Absolute(at) => at,
        }
    }

    /// Für den Dialog: ganze Minuten vor dem Beginn (negativ = danach, etwa
    /// 9 Uhr am Tag eines ganztägigen Termins). `None` für andere Formen.
    pub fn minutes_before(&self) -> Option<i64> {
        match self.trigger {
            Trigger::Relative {
                offset,
                from_end: false,
            } if offset.num_seconds() % 60 == 0 => Some(-offset.num_minutes()),
            _ => None,
        }
    }
}

/// Alle lesbaren VALARMs eines Termins
pub fn alarms_of(event: &Component) -> Vec<Alarm> {
    event
        .children_named("VALARM")
        .enumerate()
        .filter_map(|(i, c)| Alarm::from_component(i, c))
        .collect()
}

/// TRIGGER-Wert für "`minutes` vor Beginn": `-PT15M`, `-P1D`, `PT9H`, `PT0S`
pub fn trigger_value(minutes: i64) -> String {
    if minutes == 0 {
        return "PT0S".into();
    }
    let sign = if minutes > 0 { "-" } else { "" };
    let m = minutes.abs();
    if m % 10080 == 0 {
        format!("{sign}P{}W", m / 10080)
    } else if m % 1440 == 0 {
        format!("{sign}P{}D", m / 1440)
    } else if m % 60 == 0 {
        format!("{sign}PT{}H", m / 60)
    } else {
        format!("{sign}PT{m}M")
    }
}

fn new_alarm(minutes: i64, summary: &str) -> Component {
    let mut alarm = Component::new("VALARM");
    alarm.properties.push(Property::new("ACTION", "DISPLAY"));
    let text = if summary.trim().is_empty() {
        "Erinnerung"
    } else {
        summary
    };
    alarm
        .properties
        .push(Property::new("DESCRIPTION", escape_text(text)));
    alarm
        .properties
        .push(Property::new("TRIGGER", trigger_value(minutes)));
    alarm
}

/// Erinnerungen eines Termins setzen: je eine für `minutes` vor Beginn, dazu
/// die unveränderten Alarme mit den Indizes `keep`. Ein vorhandener Alarm mit
/// gleichem Abstand bleibt samt eigener Felder stehen; E-Mail-Alarme und
/// unlesbare Alarme bleiben immer.
pub fn set_reminders(event: &mut Component, minutes: &[i64], keep: &[usize]) {
    let summary = event.text_of("SUMMARY");
    let mut wanted: Vec<i64> = Vec::new();
    for m in minutes {
        if !wanted.contains(m) {
            wanted.push(*m);
        }
    }
    let mut index = 0;
    let mut children = Vec::new();
    for child in std::mem::take(&mut event.components) {
        if !child.is("VALARM") {
            children.push(child);
            continue;
        }
        let alarm = Alarm::from_component(index, &child);
        index += 1;
        let stays = match &alarm {
            None => true,
            Some(a) if !a.notifies() || keep.contains(&a.index) => true,
            Some(a) => match a.minutes_before() {
                Some(m) if wanted.contains(&m) => {
                    wanted.retain(|w| *w != m);
                    true
                }
                _ => false,
            },
        };
        if stays {
            children.push(child);
        }
    }
    children.extend(wanted.into_iter().map(|m| new_alarm(m, &summary)));
    event.components = children;
}

/// Alarme eines Termins zum Vergleichen (Art und Auslöser in Reihenfolge)
fn signature(event: &Component) -> Vec<(String, Trigger)> {
    alarms_of(event)
        .into_iter()
        .map(|a| (a.action, a.trigger))
        .collect()
}

/// Erinnerungen in einem Termin des Objekts setzen (siehe [`set_reminders`]).
/// `instance` wählt die Ausnahme einer Serie, sonst gilt die Hauptkomponente;
/// mit `like_master` auch alle Ausnahmen, die bisher dieselben Erinnerungen
/// hatten wie die Hauptkomponente.
pub fn set_reminders_in(
    object: &mut CalendarObject,
    uid: &str,
    instance: Option<DateTime<Utc>>,
    home: Tz,
    minutes: &[i64],
    keep: &[usize],
    like_master: bool,
) {
    let root = object.root.clone();
    let of_uid =
        |c: &Component| c.is("VEVENT") && c.property("UID").is_some_and(|p| p.value.trim() == uid);
    let rid = |c: &Component| {
        c.property("RECURRENCE-ID")
            .and_then(|p| DateTimeValue::from_property(p, Some(&root)).ok())
            .map(|v| v.to_utc(home))
    };
    let master = root
        .components
        .iter()
        .find(|c| of_uid(c) && rid(c).is_none())
        .map(signature);
    for component in object.root.components.iter_mut().filter(|c| of_uid(c)) {
        let own = rid(component);
        let target = match instance {
            Some(i) => own == Some(i),
            None => {
                own.is_none() || (like_master && master.as_ref() == Some(&signature(component)))
            }
        };
        if target {
            set_reminders(component, minutes, keep);
        }
    }
}

/// Eine fällige Erinnerung
#[derive(Debug, Clone, PartialEq)]
pub struct Due {
    pub at: DateTime<Utc>,
    /// Index des Alarms im Termin (bzw. in der Ausnahme)
    pub alarm: usize,
    pub occurrence: Occurrence,
}

/// Erinnerungen, die in `[from, to)` auslösen. Ausnahmen einer Serie bringen
/// ihre eigenen Alarme mit; Weggeklicktes fällt weg.
pub fn reminders_between(
    events: &[EventInfo],
    home: Tz,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<Due> {
    let mut out = Vec::new();
    let mut absolute_seen = HashSet::new();
    for occurrence in expand(events, home, from - MAX_AFTER, to + MAX_BEFORE) {
        let master = events
            .iter()
            .find(|e| e.uid == occurrence.uid && e.recurrence_id.is_none());
        let source = occurrence
            .instance
            .and_then(|i| {
                events.iter().find(|e| {
                    e.uid == occurrence.uid && e.recurrence_id.is_some_and(|r| r.to_utc(home) == i)
                })
            })
            .or(master);
        let Some(source) = source else {
            continue;
        };
        let last_ack = source.last_ack.max(master.and_then(|m| m.last_ack));
        for alarm in source.alarms.iter().filter(|a| a.notifies()) {
            let at = alarm.fires_at(occurrence.start, occurrence.end);
            let acknowledged = alarm
                .acknowledged
                .max(last_ack)
                .is_some_and(|ack| ack >= at);
            if acknowledged || at < from || at >= to {
                continue;
            }
            // Absolute Zeit in einer Serie: nur einmal
            if matches!(alarm.trigger, Trigger::Absolute(_))
                && !absolute_seen.insert((occurrence.uid.clone(), alarm.index))
            {
                continue;
            }
            out.push(Due {
                at,
                alarm: alarm.index,
                occurrence: occurrence.clone(),
            });
        }
    }
    out.sort_by_key(|d| d.at);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CalendarObject;
    use chrono::{NaiveDateTime, TimeZone};
    use chrono_tz::Europe::Berlin;

    fn at(s: &str) -> DateTime<Utc> {
        Berlin
            .from_local_datetime(&NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap())
            .unwrap()
            .with_timezone(&Utc)
    }

    fn object(body: &str) -> CalendarObject {
        CalendarObject::parse(&format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}END:VCALENDAR\r\n"
        ))
        .unwrap()
    }

    const ZAHNARZT: &str = "BEGIN:VEVENT\r\nUID:z\r\nDTSTART;TZID=Europe/Berlin:20261110T150000\r\nDTEND;TZID=Europe/Berlin:20261110T160000\r\n\
        SUMMARY:Zahnarzt\r\n\
        BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Zahnarzt\r\nTRIGGER:-PT15M\r\nX-FREMD:bleibt\r\nEND:VALARM\r\n\
        BEGIN:VALARM\r\nACTION:EMAIL\r\nDESCRIPTION:Mail\r\nSUMMARY:Mail\r\nATTENDEE:mailto:a@example.org\r\nTRIGGER:-P1D\r\nEND:VALARM\r\n\
        BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Ende\r\nTRIGGER;RELATED=END:-PT5M\r\nEND:VALARM\r\n\
        BEGIN:VALARM\r\nACTION:AUDIO\r\nTRIGGER;VALUE=DATE-TIME:20261110T080000Z\r\nEND:VALARM\r\n\
        END:VEVENT\r\n";

    #[test]
    fn reads_all_trigger_forms() {
        let o = object(ZAHNARZT);
        let alarms = alarms_of(o.root.children_named("VEVENT").next().unwrap());
        assert_eq!(alarms.len(), 4);
        assert_eq!(alarms[0].minutes_before(), Some(15));
        assert!(!alarms[1].notifies());
        assert_eq!(alarms[2].minutes_before(), None);
        assert_eq!(
            alarms[2].fires_at(at("2026-11-10 15:00"), at("2026-11-10 16:00")),
            at("2026-11-10 15:55")
        );
        assert_eq!(
            alarms[3].trigger,
            Trigger::Absolute(Utc.with_ymd_and_hms(2026, 11, 10, 8, 0, 0).unwrap())
        );
        let (events, _) = o.events();
        assert_eq!(events[0].alarms, alarms);
    }

    #[test]
    fn trigger_values_use_the_largest_unit() {
        assert_eq!(trigger_value(0), "PT0S");
        assert_eq!(trigger_value(15), "-PT15M");
        assert_eq!(trigger_value(90), "-PT90M");
        assert_eq!(trigger_value(120), "-PT2H");
        assert_eq!(trigger_value(2880), "-P2D");
        assert_eq!(trigger_value(10080), "-P1W");
        // Ganztägig: am Tag um 9 Uhr bzw. am Vortag um 9 Uhr
        assert_eq!(trigger_value(-540), "PT9H");
        assert_eq!(trigger_value(900), "-PT15H");
        for m in [0, 5, 15, 90, 120, 1440, 10080, -540, 900] {
            let alarm = Alarm::from_component(0, &new_alarm(m, "x")).unwrap();
            assert_eq!(alarm.minutes_before(), Some(m));
        }
    }

    #[test]
    fn setting_reminders_keeps_what_the_dialog_does_not_show() {
        let mut o = object(ZAHNARZT);
        let event = o
            .root
            .components
            .iter_mut()
            .find(|c| c.is("VEVENT"))
            .unwrap();
        // 15 Minuten bleibt (mit fremdem Feld), 1 Stunde kommt dazu, "vor Ende" fällt weg, die absolute bleibt
        set_reminders(event, &[15, 60], &[3]);
        let ics = o.to_ics();
        assert!(ics.contains("X-FREMD:bleibt"));
        assert!(ics.contains("ACTION:EMAIL"), "E-Mail-Alarm bleibt");
        assert!(!ics.contains("RELATED=END"));
        assert!(ics.contains("TRIGGER;VALUE=DATE-TIME:20261110T080000Z"));
        assert!(ics.contains(
            "BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Zahnarzt\r\nTRIGGER:-PT1H\r\nEND:VALARM"
        ));
        let event = o
            .root
            .components
            .iter_mut()
            .find(|c| c.is("VEVENT"))
            .unwrap();
        set_reminders(event, &[], &[]);
        assert_eq!(
            o.root
                .children_named("VEVENT")
                .next()
                .unwrap()
                .children_named("VALARM")
                .count(),
            1
        );
    }

    #[test]
    fn due_reminders_for_series_exceptions_and_all_day_events() {
        let o = object(
            "BEGIN:VEVENT\r\nUID:mathe\r\nDTSTART;TZID=Europe/Berlin:20261102T080000\r\nDTEND;TZID=Europe/Berlin:20261102T093000\r\n\
             RRULE:FREQ=WEEKLY\r\nSUMMARY:Mathe\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Mathe\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:mathe\r\nRECURRENCE-ID;TZID=Europe/Berlin:20261109T080000\r\nDTSTART;TZID=Europe/Berlin:20261109T100000\r\n\
             DTEND;TZID=Europe/Berlin:20261109T113000\r\nSUMMARY:Mathe (verlegt)\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:x\r\nTRIGGER:-PT1H\r\nEND:VALARM\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:geb\r\nDTSTART;VALUE=DATE:20261112\r\nDTEND;VALUE=DATE:20261113\r\nSUMMARY:Geburtstag\r\n\
             BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:x\r\nTRIGGER:-PT15H\r\nEND:VALARM\r\nEND:VEVENT\r\n",
        );
        let (events, _) = o.events();
        let due = reminders_between(
            &events,
            Berlin,
            at("2026-11-01 00:00"),
            at("2026-11-17 00:00"),
        );
        let got: Vec<(DateTime<Utc>, &str)> = due
            .iter()
            .map(|d| (d.at, d.occurrence.summary.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (at("2026-11-02 07:50"), "Mathe"),
                (at("2026-11-09 09:00"), "Mathe (verlegt)"),
                // Ganztägig am 12.: -15 Stunden = Vortag 9 Uhr
                (at("2026-11-11 09:00"), "Geburtstag"),
                (at("2026-11-16 07:50"), "Mathe"),
            ]
        );
        // Die Erinnerung kann vor dem Zeitraum der Termine liegen
        let early = reminders_between(
            &events,
            Berlin,
            at("2026-11-16 07:00"),
            at("2026-11-16 08:00"),
        );
        assert_eq!(early.len(), 1);
    }

    #[test]
    fn series_reminders_follow_the_scope() {
        let series = "BEGIN:VEVENT\r\nUID:m\r\nDTSTART;TZID=Europe/Berlin:20261102T080000\r\nDTEND;TZID=Europe/Berlin:20261102T090000\r\n\
             RRULE:FREQ=WEEKLY\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:x\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:m\r\nRECURRENCE-ID;TZID=Europe/Berlin:20261109T080000\r\nDTSTART;TZID=Europe/Berlin:20261109T100000\r\n\
             DTEND;TZID=Europe/Berlin:20261109T110000\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:x\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:m\r\nRECURRENCE-ID;TZID=Europe/Berlin:20261116T080000\r\nDTSTART;TZID=Europe/Berlin:20261116T100000\r\n\
             DTEND;TZID=Europe/Berlin:20261116T110000\r\nEND:VEVENT\r\n";
        let minutes = |o: &CalendarObject| -> Vec<Vec<Option<i64>>> {
            o.root
                .children_named("VEVENT")
                .map(|e| alarms_of(e).iter().map(|a| a.minutes_before()).collect())
                .collect()
        };
        // Nur dieser (9.11.)
        let mut o = object(series);
        set_reminders_in(
            &mut o,
            "m",
            Some(at("2026-11-09 08:00")),
            Berlin,
            &[60],
            &[],
            false,
        );
        assert_eq!(minutes(&o), vec![vec![Some(10)], vec![Some(60)], vec![]]);
        // Alle: Serie und die Ausnahme mit gleicher Erinnerung; die ohne bleibt ohne
        let mut o = object(series);
        set_reminders_in(&mut o, "m", None, Berlin, &[30], &[], true);
        assert_eq!(minutes(&o), vec![vec![Some(30)], vec![Some(30)], vec![]]);
    }

    #[test]
    fn acknowledged_reminders_are_skipped() {
        let o = object(
            "BEGIN:VEVENT\r\nUID:a\r\nDTSTART:20261110T140000Z\r\nDTEND:20261110T150000Z\r\nRRULE:FREQ=DAILY;COUNT=3\r\n\
             X-MOZ-LASTACK:20261110T135000Z\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:a\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:b\r\nDTSTART:20261110T140000Z\r\nDTEND:20261110T150000Z\r\n\
             BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:b\r\nTRIGGER:-PT15M\r\nACKNOWLEDGED:20261110T134600Z\r\nEND:VALARM\r\nEND:VEVENT\r\n",
        );
        let (events, _) = o.events();
        let from = Utc.with_ymd_and_hms(2026, 11, 10, 0, 0, 0).unwrap();
        let due = reminders_between(&events, Berlin, from, from + Duration::days(5));
        // a: erster Termin schon bestätigt, die beiden weiteren kommen; b: bestätigt
        assert_eq!(due.len(), 2);
        assert!(due.iter().all(|d| d.occurrence.uid == "a"));
    }
}
