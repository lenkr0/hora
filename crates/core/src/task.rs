//! Aufgaben (VTODO, RFC 5545 Abschnitt 3.6.2): lesen, anlegen, ändern,
//! erledigen. Teilaufgaben verweisen mit `RELATED-TO` (RELTYPE=PARENT) auf
//! ihre übergeordnete Aufgabe.
//!
//! Wiederkehrende Aufgaben wie Thunderbird und Calino: Die Serie (mit RRULE)
//! bleibt unverändert; ein erledigtes Vorkommen ist eine Ausnahme mit
//! `RECURRENCE-ID` und `STATUS:COMPLETED` in derselben Ressource. Angezeigt
//! wird das erste offene Vorkommen nach dem zuletzt erledigten.

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;

use crate::alarm::{Alarm, alarms_of};
use crate::edit::{EditError, PRODID, ensure_timezones, set_text, time_property, touch, utc_stamp};
use crate::expand::expand;
use crate::ical::{Component, Property};
use crate::model::{CalendarObject, EventInfo};
use crate::time::DateTimeValue;

/// Eine Aufgabe, aufbereitet für Liste und Dialog
#[derive(Debug, Clone, PartialEq)]
pub struct TaskInfo {
    pub uid: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub start: Option<DateTimeValue>,
    pub due: Option<DateTimeValue>,
    pub completed: bool,
    pub completed_at: Option<DateTime<Utc>>,
    pub cancelled: bool,
    /// 0 = keine, 1 = höchste … 9 = niedrigste (RFC 5545)
    pub priority: u8,
    /// UID der übergeordneten Aufgabe
    pub parent: Option<String>,
    pub rrule: Option<String>,
    pub exdates: Vec<DateTimeValue>,
    pub recurrence_id: Option<DateTimeValue>,
    /// Manuelle Reihenfolge (X-APPLE-SORT-ORDER, wie Apple, Nextcloud, Calino)
    pub sort_order: Option<i64>,
    pub alarms: Vec<Alarm>,
}

fn value_of(component: &Component, name: &str, root: &Component) -> Option<DateTimeValue> {
    component
        .property(name)
        .and_then(|p| DateTimeValue::from_property(p, Some(root)).ok())
}

fn is_parent_relation(p: &Property) -> bool {
    p.is("RELATED-TO")
        && p.param("RELTYPE")
            .is_none_or(|t| t.eq_ignore_ascii_case("PARENT"))
}

impl TaskInfo {
    pub fn from_component(todo: &Component, root: &Component) -> Option<TaskInfo> {
        let uid = todo.property("UID")?.value.trim().to_string();
        let status = todo.text_of("STATUS").trim().to_ascii_uppercase();
        let percent = todo
            .text_of("PERCENT-COMPLETE")
            .trim()
            .parse::<u8>()
            .unwrap_or(0);
        let completed_at = value_of(todo, "COMPLETED", root).map(|v| v.to_utc(Tz::UTC));
        let mut exdates = Vec::new();
        for property in todo.properties_named("EXDATE") {
            for part in crate::ical::split_values(&property.value) {
                let mut single = property.clone();
                single.value = part.trim().to_string();
                if let Ok(v) = DateTimeValue::from_property(&single, Some(root)) {
                    exdates.push(v);
                }
            }
        }
        Some(TaskInfo {
            summary: todo.text_of("SUMMARY"),
            description: todo.text_of("DESCRIPTION"),
            location: todo.text_of("LOCATION"),
            start: value_of(todo, "DTSTART", root),
            due: value_of(todo, "DUE", root),
            completed: status == "COMPLETED" || percent >= 100 || completed_at.is_some(),
            completed_at,
            cancelled: status == "CANCELLED",
            priority: todo
                .text_of("PRIORITY")
                .trim()
                .parse::<u8>()
                .unwrap_or(0)
                .min(9),
            parent: todo
                .properties
                .iter()
                .find(|p| is_parent_relation(p))
                .map(|p| p.value.trim().to_string())
                .filter(|p| !p.is_empty() && *p != uid),
            rrule: todo.property("RRULE").map(|p| p.value.trim().to_string()),
            exdates,
            recurrence_id: value_of(todo, "RECURRENCE-ID", root),
            sort_order: todo.text_of("X-APPLE-SORT-ORDER").trim().parse().ok(),
            alarms: alarms_of(todo),
            uid,
        })
    }

    pub fn is_recurring(&self) -> bool {
        self.rrule.is_some() && self.recurrence_id.is_none()
    }

    /// Verankert eine Serie: DTSTART, ersatzweise DUE
    fn anchor(&self) -> Option<DateTimeValue> {
        self.start.or(self.due)
    }
}

impl CalendarObject {
    /// Alle VTODOs (Serie und Ausnahmen) dieses Objekts
    pub fn tasks(&self) -> Vec<TaskInfo> {
        self.root
            .children_named("VTODO")
            .filter_map(|c| TaskInfo::from_component(c, &self.root))
            .collect()
    }
}

/// Priorität im Dialog (wie Calino): keine, hoch, mittel, niedrig
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    None,
    High,
    Medium,
    Low,
}

impl Priority {
    pub fn from_value(value: u8) -> Priority {
        match value {
            0 => Priority::None,
            1..=4 => Priority::High,
            5 => Priority::Medium,
            _ => Priority::Low,
        }
    }

    pub fn value(self) -> u8 {
        match self {
            Priority::None => 0,
            Priority::High => 1,
            Priority::Medium => 5,
            Priority::Low => 9,
        }
    }
}

/// Felder des Aufgaben-Dialogs
#[derive(Debug, Clone, PartialEq)]
pub struct TaskFields {
    pub summary: String,
    pub description: String,
    pub due: Option<DateTimeValue>,
    pub start: Option<DateTimeValue>,
    /// RFC-5545-Wert; beim Bearbeiten bleibt ein Wert der gleichen Stufe stehen
    pub priority: u8,
    pub parent: Option<String>,
}

const SERIES_PROPERTIES: [&str; 4] = ["RRULE", "RDATE", "EXDATE", "EXRULE"];

fn master_mut<'a>(
    object: &'a mut CalendarObject,
    uid: &str,
) -> Result<&'a mut Component, EditError> {
    object
        .root
        .components
        .iter_mut()
        .find(|c| {
            c.is("VTODO")
                && c.property("RECURRENCE-ID").is_none()
                && c.property("UID").is_some_and(|p| p.value.trim() == uid)
        })
        .ok_or_else(|| EditError::NotFound(uid.to_string()))
}

/// DTSTART und DUE müssen dieselbe Art haben (RFC 5545): ein Beginn mit
/// Uhrzeit bei ganztägiger Fälligkeit wird zum Datum.
fn same_kind(
    start: Option<DateTimeValue>,
    due: Option<DateTimeValue>,
    home: Tz,
) -> Option<DateTimeValue> {
    match (start, due) {
        (Some(s), Some(DateTimeValue::Date(_))) if !s.is_date() => Some(DateTimeValue::Date(
            s.to_utc(home).with_timezone(&s.zone(home)).date_naive(),
        )),
        (Some(DateTimeValue::Date(d)), Some(due)) if !due.is_date() => Some(due.moved_to(
            crate::time::local_to_utc(d.and_hms_opt(0, 0, 0)?, due.zone(home)),
            home,
        )),
        (s, _) => s,
    }
}

fn write_fields(todo: &mut Component, fields: &TaskFields, home: Tz) {
    set_text(todo, "SUMMARY", &fields.summary);
    set_text(todo, "DESCRIPTION", &fields.description);
    let start = same_kind(fields.start, fields.due, home);
    for (name, value) in [("DUE", fields.due), ("DTSTART", start)] {
        match value {
            Some(v) => {
                let property = time_property(name, &v, todo.property(name));
                todo.set_property(property);
            }
            None => todo.remove_properties(name),
        }
    }
    todo.remove_properties("DURATION");
    let old =
        TaskInfo::from_component(todo, &Component::new("VCALENDAR")).map_or(0, |t| t.priority);
    if fields.priority == 0 {
        todo.remove_properties("PRIORITY");
    } else if Priority::from_value(old) != Priority::from_value(fields.priority) {
        todo.set_property(Property::new("PRIORITY", fields.priority.to_string()));
    }
    let current = todo
        .properties
        .iter()
        .find(|p| is_parent_relation(p))
        .map(|p| p.value.trim().to_string());
    if current != fields.parent {
        todo.properties.retain(|p| !is_parent_relation(p));
        if let Some(parent) = &fields.parent {
            let mut related = Property::new("RELATED-TO", parent.clone());
            related.set_param("RELTYPE", "PARENT");
            todo.properties.push(related);
        }
    }
}

/// Neues Kalenderobjekt mit einer offenen Aufgabe
pub fn new_task(uid: &str, fields: &TaskFields, home: Tz, now: DateTime<Utc>) -> CalendarObject {
    let mut root = Component::new("VCALENDAR");
    root.properties.push(Property::new("VERSION", "2.0"));
    root.properties.push(Property::new("PRODID", PRODID));
    root.properties.push(Property::new("CALSCALE", "GREGORIAN"));
    let stamp = utc_stamp(now);
    let mut todo = Component::new("VTODO");
    for (name, value) in [
        ("UID", uid),
        ("DTSTAMP", &stamp),
        ("CREATED", &stamp),
        ("LAST-MODIFIED", &stamp),
    ] {
        todo.properties.push(Property::new(name, value));
    }
    todo.properties.push(Property::new("SEQUENCE", "0"));
    todo.properties
        .push(Property::new("STATUS", "NEEDS-ACTION"));
    write_fields(&mut todo, fields, home);
    root.components.push(todo);
    let mut object = CalendarObject { root };
    ensure_timezones(&mut object);
    object
}

/// Felder einer bestehenden Aufgabe (bei Serien: der Serie) setzen; fremde
/// Eigenschaften bleiben.
pub fn update_task(
    object: &mut CalendarObject,
    uid: &str,
    fields: &TaskFields,
    home: Tz,
    now: DateTime<Utc>,
) -> Result<(), EditError> {
    let todo = master_mut(object, uid)?;
    write_fields(todo, fields, home);
    touch(todo, now);
    ensure_timezones(object);
    Ok(())
}

fn mark(todo: &mut Component, done: bool, now: DateTime<Utc>) {
    if done {
        todo.set_property(Property::new("STATUS", "COMPLETED"));
        todo.set_property(Property::new("PERCENT-COMPLETE", "100"));
        todo.set_property(Property::new("COMPLETED", utc_stamp(now)));
    } else {
        todo.set_property(Property::new("STATUS", "NEEDS-ACTION"));
        todo.remove_properties("PERCENT-COMPLETE");
        todo.remove_properties("COMPLETED");
    }
    touch(todo, now);
}

/// Aufgabe (ohne Wiederholung) erledigen oder wieder öffnen
pub fn set_completed(
    object: &mut CalendarObject,
    uid: &str,
    done: bool,
    now: DateTime<Utc>,
) -> Result<(), EditError> {
    mark(master_mut(object, uid)?, done, now);
    Ok(())
}

/// Ein Vorkommen einer wiederkehrenden Aufgabe
#[derive(Debug, Clone, PartialEq)]
pub struct TaskOccurrence {
    /// Wert für RECURRENCE-ID (Art und Zone des Ankers)
    pub recurrence_id: DateTimeValue,
    pub start: Option<DateTimeValue>,
    pub due: Option<DateTimeValue>,
}

/// Stand einer wiederkehrenden Aufgabe
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesState {
    /// Das aktuelle offene Vorkommen; `None`, wenn keines mehr übrig ist
    pub current: Option<TaskOccurrence>,
    /// Das Vorkommen danach
    pub next: Option<DateTime<Utc>>,
    /// Wie viele Vorkommen schon erledigt sind
    pub completed: usize,
}

fn shift(value: DateTimeValue, delta: Duration) -> DateTimeValue {
    match value {
        DateTimeValue::Date(d) => DateTimeValue::Date(d + Duration::days(delta.num_days())),
        DateTimeValue::Floating(n) => DateTimeValue::Floating(n + delta),
        DateTimeValue::Utc(n) => DateTimeValue::Utc(n + delta),
        DateTimeValue::Zoned(n, tz) => DateTimeValue::Zoned(n + delta, tz),
    }
}

/// Erstes offenes Vorkommen nach dem zuletzt erledigten. `tasks` sind die
/// VTODOs mit der UID der Serie (Serie und Ausnahmen).
pub fn series_state(tasks: &[TaskInfo], home: Tz) -> Option<SeriesState> {
    let master = tasks.iter().find(|t| t.is_recurring())?;
    let anchor = master.anchor()?;
    let done: Vec<DateTime<Utc>> = tasks
        .iter()
        .filter(|t| t.recurrence_id.is_some() && (t.completed || t.cancelled))
        .filter_map(|t| t.recurrence_id.map(|r| r.to_utc(home)))
        .collect();
    let from = done
        .iter()
        .max()
        .copied()
        .map_or(anchor.to_utc(home), |last| last + Duration::seconds(1));
    let info = EventInfo {
        uid: master.uid.clone(),
        summary: String::new(),
        location: String::new(),
        description: String::new(),
        start: anchor,
        end: anchor,
        rrule: master.rrule.clone(),
        rdates: Vec::new(),
        exdates: master.exdates.clone(),
        recurrence_id: None,
        cancelled: false,
        alarms: Vec::new(),
        last_ack: None,
    };
    // Vorkommen ab dem Anker in Schritten suchen, damit auch lange Serien gehen
    let mut window_start = from - Duration::seconds(1);
    let mut open: Vec<DateTime<Utc>> = Vec::new();
    for _ in 0..8 {
        let window_end = window_start + Duration::days(3650);
        let found: Vec<DateTime<Utc>> =
            expand(std::slice::from_ref(&info), home, window_start, window_end)
                .into_iter()
                .map(|o| o.start)
                .filter(|s| *s >= from && !done.contains(s))
                .collect();
        open.extend(found);
        if open.len() >= 2 {
            break;
        }
        window_start = window_end;
    }
    open.sort();
    let current = open.first().map(|instance| {
        let recurrence_id = anchor.moved_to(*instance, home);
        let delta = recurrence_id.naive() - anchor.naive();
        TaskOccurrence {
            recurrence_id,
            start: master.start.map(|s| shift(s, delta)),
            due: master.due.map(|d| shift(d, delta)),
        }
    });
    Some(SeriesState {
        current,
        next: open.get(1).copied(),
        completed: done.len(),
    })
}

/// Ein Vorkommen einer wiederkehrenden Aufgabe erledigen (Ausnahme mit
/// `STATUS:COMPLETED`) oder das Erledigt wieder entfernen.
pub fn complete_occurrence(
    object: &mut CalendarObject,
    uid: &str,
    occurrence: &TaskOccurrence,
    done: bool,
    home: Tz,
    now: DateTime<Utc>,
) -> Result<(), EditError> {
    let root = object.root.clone();
    let instance = occurrence.recurrence_id.to_utc(home);
    let is_exception = |c: &Component| {
        c.is("VTODO")
            && c.property("UID").is_some_and(|p| p.value.trim() == uid)
            && value_of(c, "RECURRENCE-ID", &root).is_some_and(|r| r.to_utc(home) == instance)
    };
    if !done {
        object.root.components.retain(|c| !is_exception(c));
        return Ok(());
    }
    if let Some(existing) = object.root.components.iter_mut().find(|c| is_exception(c)) {
        mark(existing, true, now);
        return Ok(());
    }
    let master = master_mut(object, uid)?;
    let mut copy = master.clone();
    for name in SERIES_PROPERTIES {
        copy.remove_properties(name);
    }
    copy.properties.push(time_property(
        "RECURRENCE-ID",
        &occurrence.recurrence_id,
        None,
    ));
    for (name, value) in [("DTSTART", occurrence.start), ("DUE", occurrence.due)] {
        if let Some(v) = value {
            let property = time_property(name, &v, copy.property(name));
            copy.set_property(property);
        }
    }
    mark(&mut copy, true, now);
    let index = object
        .root
        .components
        .iter()
        .position(|c| c.is("VTODO") && c.property("UID").is_some_and(|p| p.value.trim() == uid))
        .map_or(object.root.components.len(), |i| i + 1);
    object.root.components.insert(index, copy);
    Ok(())
}

/// Ortszeit, zu der eine Aufgabe nur mit Datum fällig gemeldet wird (Calino)
pub const DATE_ONLY_HOUR: u32 = 9;

/// Zeitpunkt, zu dem eine Aufgabe fällig ist: ihre Uhrzeit, bei Aufgaben nur
/// mit Datum 9 Uhr am Tag
pub fn due_instant(due: &DateTimeValue, home: Tz) -> DateTime<Utc> {
    match due {
        DateTimeValue::Date(d) => crate::time::local_to_utc(
            d.and_hms_opt(DATE_ONLY_HOUR, 0, 0).unwrap_or_default(),
            home,
        ),
        other => other.to_utc(home),
    }
}

/// Eine fällige Erinnerung an eine Aufgabe
#[derive(Debug, Clone, PartialEq)]
pub struct TaskReminder {
    pub uid: String,
    pub at: DateTime<Utc>,
    /// Index des Alarms; `None` = Erinnerung zur Fälligkeit
    pub alarm: Option<usize>,
    pub summary: String,
    pub due: DateTime<Utc>,
    pub date_only: bool,
    /// Vorkommen einer wiederkehrenden Aufgabe
    pub instance: Option<DateTime<Utc>>,
}

/// Erinnerungen offener Aufgaben in `[from, to)`: eigene Alarme (relativ zur
/// Fälligkeit bzw. zum Beginn) oder, wenn eine Aufgabe keine hat und
/// `at_due` gilt, eine zur Fälligkeit. Bei Serien zählt nur das aktuelle
/// offene Vorkommen.
pub fn task_reminders(
    tasks: &[TaskInfo],
    home: Tz,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    at_due: bool,
) -> Vec<TaskReminder> {
    let mut out = Vec::new();
    for task in tasks
        .iter()
        .filter(|t| t.recurrence_id.is_none() && !t.cancelled)
    {
        let (due, start, instance) = if task.is_recurring() {
            let family: Vec<TaskInfo> = tasks
                .iter()
                .filter(|t| t.uid == task.uid)
                .cloned()
                .collect();
            match series_state(&family, home).and_then(|s| s.current) {
                Some(o) => (o.due, o.start, Some(o.recurrence_id.to_utc(home))),
                None => continue,
            }
        } else if task.completed {
            continue;
        } else {
            (task.due, task.start, None)
        };
        let Some(due) = due else {
            continue;
        };
        let due_at = due_instant(&due, home);
        let begin = start.map_or(due_at, |s| s.to_utc(home));
        let notifying: Vec<&Alarm> = task.alarms.iter().filter(|a| a.notifies()).collect();
        let mut push = |at: DateTime<Utc>, alarm: Option<usize>| {
            if at >= from && at < to {
                out.push(TaskReminder {
                    uid: task.uid.clone(),
                    at,
                    alarm,
                    summary: task.summary.clone(),
                    due: due_at,
                    date_only: due.is_date(),
                    instance,
                });
            }
        };
        if notifying.is_empty() {
            if at_due {
                push(due_at, None);
            }
        } else {
            for alarm in notifying {
                let acknowledged =
                    |at: DateTime<Utc>| alarm.acknowledged.is_some_and(|ack| ack >= at);
                // VTODO: relativ zum Beginn, mit RELATED=END zur Fälligkeit
                let at = alarm.fires_at(begin, due_at);
                if !acknowledged(at) {
                    push(at, Some(alarm.index));
                }
            }
        }
    }
    out.sort_by_key(|r| r.at);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveDateTime, TimeZone};
    use chrono_tz::Europe::Berlin;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 5, 12, 0, 0).unwrap()
    }

    fn berlin(s: &str) -> DateTimeValue {
        DateTimeValue::Zoned(
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap(),
            Berlin,
        )
    }

    fn date(y: i32, m: u32, d: u32) -> DateTimeValue {
        DateTimeValue::Date(NaiveDate::from_ymd_opt(y, m, d).unwrap())
    }

    fn fields(summary: &str) -> TaskFields {
        TaskFields {
            summary: summary.into(),
            description: String::new(),
            due: None,
            start: None,
            priority: 0,
            parent: None,
        }
    }

    #[test]
    fn new_tasks_read_back() {
        let mut f = fields("Steuererklärung");
        f.due = Some(berlin("2026-11-10 18:00"));
        f.priority = Priority::High.value();
        f.parent = Some("projekt".into());
        let object = new_task("t1", &f, Berlin, now());
        let ics = object.to_ics();
        assert!(ics.contains("BEGIN:VTODO") && ics.contains("STATUS:NEEDS-ACTION"));
        assert!(ics.contains("DUE;TZID=Europe/Berlin:20261110T180000"));
        assert!(ics.contains("RELATED-TO;RELTYPE=PARENT:projekt"));
        assert!(ics.contains("BEGIN:VTIMEZONE"));
        let task = &object.tasks()[0];
        assert_eq!(task.summary, "Steuererklärung");
        assert_eq!(task.due, Some(berlin("2026-11-10 18:00")));
        assert_eq!(task.priority, 1);
        assert_eq!(task.parent.as_deref(), Some("projekt"));
        assert!(!task.completed);
    }

    #[test]
    fn foreign_tasks_keep_their_extras() {
        let mut object = CalendarObject::parse(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:x\r\nSUMMARY:Alt\r\nPRIORITY:3\r\nX-APPLE-SORT-ORDER:2000\r\n\
             RELATED-TO;RELTYPE=SIBLING:y\r\nCATEGORIES:Haus\r\nDUE;VALUE=DATE:20261110\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        let task = object.tasks()[0].clone();
        assert_eq!(
            (task.priority, task.sort_order, task.parent.clone()),
            (3, Some(2000), None)
        );
        assert_eq!(task.due, Some(date(2026, 11, 10)));
        let mut f = fields("Neu");
        f.due = task.due;
        f.priority = 2; // gleiche Stufe "hoch": PRIORITY:3 bleibt
        update_task(&mut object, "x", &f, Berlin, now()).unwrap();
        let ics = object.to_ics();
        assert!(
            ics.contains("PRIORITY:3")
                && ics.contains("CATEGORIES:Haus")
                && ics.contains("RELTYPE=SIBLING")
        );
        assert!(ics.contains("SUMMARY:Neu"));
    }

    #[test]
    fn completing_and_reopening() {
        let mut object = new_task("t", &fields("Müll"), Berlin, now());
        set_completed(&mut object, "t", true, now()).unwrap();
        let ics = object.to_ics();
        assert!(
            ics.contains("STATUS:COMPLETED")
                && ics.contains("PERCENT-COMPLETE:100")
                && ics.contains("COMPLETED:20261105T120000Z")
        );
        assert!(object.tasks()[0].completed);
        set_completed(&mut object, "t", false, now()).unwrap();
        let ics = object.to_ics();
        assert!(
            ics.contains("STATUS:NEEDS-ACTION")
                && !ics.contains("COMPLETED:")
                && !ics.contains("PERCENT")
        );
    }

    #[test]
    fn start_follows_the_kind_of_due() {
        let mut f = fields("Bericht");
        f.due = Some(date(2026, 11, 13));
        f.start = Some(berlin("2026-11-09 08:00"));
        let ics = new_task("b", &f, Berlin, now()).to_ics();
        assert!(ics.contains("DTSTART;VALUE=DATE:20261109"), "{ics}");
    }

    #[test]
    fn recurring_tasks_complete_one_occurrence_at_a_time() {
        // Calinos Beispiel: Sport jeden Dienstag ab 3.3.2026
        let mut object = CalendarObject::parse(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:gym\r\nSUMMARY:Sport\r\nDTSTART;VALUE=DATE:20260303\r\n\
             DUE;VALUE=DATE:20260303\r\nRRULE:FREQ=WEEKLY;BYDAY=TU\r\nSTATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        let state = series_state(&object.tasks(), Berlin).unwrap();
        let current = state.current.clone().unwrap();
        assert_eq!(current.due, Some(date(2026, 3, 3)));
        assert_eq!(state.completed, 0);
        complete_occurrence(&mut object, "gym", &current, true, Berlin, now()).unwrap();
        let ics = object.to_ics();
        assert!(ics.contains("RECURRENCE-ID;VALUE=DATE:20260303"));
        assert_eq!(
            ics.matches("RRULE").count(),
            1,
            "die Serie bleibt unverändert"
        );
        let state = series_state(&object.tasks(), Berlin).unwrap();
        assert_eq!(state.current.as_ref().unwrap().due, Some(date(2026, 3, 10)));
        assert_eq!(state.completed, 1);
        // Wieder öffnen entfernt die Ausnahme
        complete_occurrence(&mut object, "gym", &current, false, Berlin, now()).unwrap();
        assert_eq!(object.tasks().len(), 1);
    }

    #[test]
    fn reminders_at_due_time_or_from_alarms() {
        let object = CalendarObject::parse(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n\
             BEGIN:VTODO\r\nUID:a\r\nSUMMARY:Nur Datum\r\nDUE;VALUE=DATE:20261110\r\nEND:VTODO\r\n\
             BEGIN:VTODO\r\nUID:b\r\nSUMMARY:Mit Uhrzeit\r\nDUE;TZID=Europe/Berlin:20261110T150000\r\nEND:VTODO\r\n\
             BEGIN:VTODO\r\nUID:c\r\nSUMMARY:Mit Alarm\r\nDUE;TZID=Europe/Berlin:20261110T150000\r\n\
             BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:x\r\nTRIGGER;RELATED=END:-PT30M\r\nEND:VALARM\r\nEND:VTODO\r\n\
             BEGIN:VTODO\r\nUID:d\r\nSUMMARY:Erledigt\r\nDUE;VALUE=DATE:20261110\r\nSTATUS:COMPLETED\r\nEND:VTODO\r\n\
             BEGIN:VTODO\r\nUID:e\r\nSUMMARY:Ohne Datum\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        let from = Utc.with_ymd_and_hms(2026, 11, 10, 0, 0, 0).unwrap();
        let got: Vec<(String, DateTime<Utc>)> = task_reminders(
            &object.tasks(),
            Berlin,
            from,
            from + Duration::days(1),
            true,
        )
        .into_iter()
        .map(|r| (r.uid, r.at))
        .collect();
        let utc = |h, m| Utc.with_ymd_and_hms(2026, 11, 10, h, m, 0).unwrap();
        assert_eq!(
            got,
            vec![
                ("a".into(), utc(8, 0)),
                ("c".into(), utc(13, 30)),
                ("b".into(), utc(14, 0))
            ]
        );
        // Ohne Fälligkeits-Erinnerung bleiben nur eigene Alarme
        assert_eq!(
            task_reminders(
                &object.tasks(),
                Berlin,
                from,
                from + Duration::days(1),
                false
            )
            .len(),
            1
        );
    }

    #[test]
    fn finished_series_has_no_current_occurrence() {
        let mut object = CalendarObject::parse(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:z\r\nSUMMARY:Zweimal\r\nDTSTART;TZID=Europe/Berlin:20261102T090000\r\n\
             DUE;TZID=Europe/Berlin:20261102T100000\r\nRRULE:FREQ=DAILY;COUNT=2\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        for _ in 0..2 {
            let occurrence = series_state(&object.tasks(), Berlin)
                .unwrap()
                .current
                .unwrap();
            complete_occurrence(&mut object, "z", &occurrence, true, Berlin, now()).unwrap();
        }
        let state = series_state(&object.tasks(), Berlin).unwrap();
        assert_eq!((state.current, state.completed), (None, 2));
        // Das zweite Vorkommen hatte seine eigenen Zeiten
        assert!(
            object
                .to_ics()
                .contains("DUE;TZID=Europe/Berlin:20261103T100000")
        );
    }
}
