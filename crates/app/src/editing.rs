//! Termine verschieben, speichern und löschen. Beispieltermine ändern sich
//! nur im Speicher; Server-Termine werden im Cache geändert und in die
//! Warteschlange gelegt, die der Sync-Thread hochlädt.
//!
//! Vorkommen einer Serie ändert [`apply_series`] mit dem im Serien-Dialog
//! gewählten Umfang (nur dieser, dieser und alle folgenden, alle).

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration, NaiveDateTime, NaiveTime, Timelike, Utc};
use hora_core::alarm::{self, Alarm};
use hora_core::edit::{self, EventFields};
use hora_core::model::{CalendarObject, EventInfo};
use hora_core::series::{self, DeleteOutcome, RuleChange, Scope, Target};
use hora_core::time::{DateTimeValue, local_to_utc};
use url::Url;

use crate::forms::DraftTimes;
use crate::items::{Item, Source};
use crate::repeat::{Repeat, RuleFields, rule_change};
use crate::sync::Command;
use crate::{State, sample};

/// Felder des Termin-Dialogs
#[derive(Debug, Clone)]
pub struct DialogInput {
    pub title: String,
    pub times: DraftTimes,
    pub location: String,
    pub description: String,
    /// Kalender-ID aus der Auswahlliste; -1 = Beispieltermine
    pub calendar: i64,
    pub rule: RuleFields,
    /// Erinnerungen; `None` = unverändert
    pub reminders: Option<ReminderInput>,
}

/// Erinnerungen aus dem Dialog
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderInput {
    /// Minuten vor Beginn
    pub minutes: Vec<i64>,
    /// Indizes der Alarme anderer Programme, die bleiben
    pub keep: Vec<usize>,
}

impl ReminderInput {
    /// Erinnerungen im Termin `uid` setzen; `instance` wählt eine Ausnahme,
    /// `like_master` nimmt gleichartige Ausnahmen mit.
    fn apply(
        &self,
        object: &mut CalendarObject,
        uid: &str,
        instance: Option<DateTime<Utc>>,
        home: chrono_tz::Tz,
        like_master: bool,
    ) {
        alarm::set_reminders_in(
            object,
            uid,
            instance,
            home,
            &self.minutes,
            &self.keep,
            like_master,
        );
    }
}

/// Änderung an einem Vorkommen einer Serie, die auf die Wahl im
/// Serien-Dialog wartet
#[derive(Debug, Clone)]
pub enum SeriesAction {
    /// Gezogen bzw. per Tastatur verschoben (Wandzeiten in `home`)
    Move {
        item: Item,
        start: NaiveDateTime,
        end: NaiveDateTime,
    },
    /// In der Monatsansicht auf einen anderen Tag gezogen
    ShiftDays {
        item: Item,
        days: i64,
    },
    Save {
        item: Item,
        input: Box<DialogInput>,
    },
    Delete {
        item: Item,
    },
}

impl SeriesAction {
    pub fn item(&self) -> &Item {
        match self {
            SeriesAction::Move { item, .. }
            | SeriesAction::ShiftDays { item, .. }
            | SeriesAction::Save { item, .. }
            | SeriesAction::Delete { item } => item,
        }
    }
}

/// Zufällige UID für neue Termine (32 Hex-Zeichen)
pub(crate) fn new_uid() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or_default();
    let random = RandomState::new().hash_one(nanos);
    format!("{nanos:016x}{random:016x}")
}

/// Pfad für eine neue Ressource im Kalender
pub(crate) fn new_href(state: &State, calendar_id: i64, uid: &str) -> Result<String, String> {
    let calendar = state
        .calendars
        .iter()
        .find(|c| c.id == calendar_id)
        .ok_or("Bitte einen Kalender wählen.")?;
    Ok(Url::parse(&calendar.href)
        .and_then(|u| u.join(&format!("{uid}.ics")))
        .map_err(|e| e.to_string())?
        .path()
        .to_string())
}

/// Ein Server-Termin aus dem Cache: Objekt, Serie bzw. Einzeltermin und das
/// angezeigte Vorkommen (bei Serien die Ausnahme oder das berechnete Vorkommen).
struct Loaded {
    object: CalendarObject,
    master: EventInfo,
    shown: EventInfo,
}

fn load(state: &State, source: &Source) -> Result<Loaded, String> {
    let Source::Server {
        calendar_id,
        href,
        uid,
        instance,
        ..
    } = source
    else {
        return Err("Kein Server-Termin.".into());
    };
    let gone = "Der Termin ist nicht mehr vorhanden.";
    let resource = state
        .store
        .resource(*calendar_id, href)
        .map_err(|e| e.to_string())?
        .ok_or(gone)?;
    let object = CalendarObject::parse(&resource.ics).map_err(|e| e.to_string())?;
    let events: Vec<EventInfo> = object
        .events()
        .0
        .into_iter()
        .filter(|e| e.uid == *uid)
        .collect();
    let master = events
        .iter()
        .find(|e| e.recurrence_id.is_none())
        .cloned()
        .ok_or(gone)?;
    let home = state.home;
    let shown = match instance {
        None => master.clone(),
        Some(instance) => events
            .iter()
            .find(|e| e.recurrence_id.is_some_and(|r| r.to_utc(home) == *instance))
            .cloned()
            .unwrap_or_else(|| {
                let length = master.end.to_utc(home) - master.start.to_utc(home);
                EventInfo {
                    start: master.start.moved_to(*instance, home),
                    end: master.end.moved_to(*instance + length, home),
                    ..master.clone()
                }
            }),
    };
    Ok(Loaded {
        object,
        master,
        shown,
    })
}

pub(crate) fn store_object(
    state: &mut State,
    calendar_id: i64,
    href: &str,
    object: &CalendarObject,
) -> Result<(), String> {
    state
        .store
        .local_put(calendar_id, href, &object.to_ics())
        .map_err(|e| e.to_string())?;
    state.send(Command::Push);
    Ok(())
}

fn server_ids(source: &Source) -> Option<(i64, &str, &str, Option<DateTime<Utc>>)> {
    match source {
        Source::Server {
            calendar_id,
            href,
            uid,
            instance,
            ..
        } => Some((*calendar_id, href, uid, *instance)),
        _ => None,
    }
}

/// Texte des angezeigten Vorkommens mit neuen Zeiten
fn fields_with_times(shown: &EventInfo, start: DateTimeValue, end: DateTimeValue) -> EventFields {
    EventFields {
        summary: shown.summary.clone(),
        location: shown.location.clone(),
        description: shown.description.clone(),
        start,
        end,
    }
}

/// Wert um ganze Tage verschieben; Uhrzeit (in `home`) bleibt.
fn shift_value(value: &DateTimeValue, delta: Duration, home: chrono_tz::Tz) -> DateTimeValue {
    match value {
        DateTimeValue::Date(d) => DateTimeValue::Date(*d + delta),
        other => {
            let local = other.to_utc(home).with_timezone(&home).naive_local() + delta;
            other.moved_to(local_to_utc(local, home), home)
        }
    }
}

/// Neue Zeiten für einen einzelnen Server-Termin
fn set_server_times(
    state: &mut State,
    source: &Source,
    f: impl FnOnce(&EventInfo) -> (DateTimeValue, DateTimeValue),
) -> Result<(), String> {
    let Some((calendar_id, href, uid, _)) = server_ids(source) else {
        return Ok(());
    };
    let Loaded {
        mut object, master, ..
    } = load(state, source)?;
    let (start, end) = f(&master);
    edit::set_times(&mut object, uid, start, end, Utc::now()).map_err(|e| e.to_string())?;
    let href = href.to_string();
    store_object(state, calendar_id, &href, &object)
}

/// Termin auf neue Wandzeiten (in `home`) legen, z. B. nach dem Ziehen in
/// der Woche. Serien gehen über [`SeriesAction::Move`].
pub fn move_to(
    state: &mut State,
    item: &Item,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<(), String> {
    if !item.movable() {
        return Err("Dieser Termin kann hier nicht verschoben werden.".into());
    }
    let home = state.home;
    match &item.source {
        Source::DemoEvent(id) => {
            if let Some(e) = state.events.iter_mut().find(|e| e.id == *id) {
                e.start = start;
                e.end = end;
            }
            Ok(())
        }
        Source::DemoAllDay(_) => Ok(()),
        source => {
            let (start, end) = (local_to_utc(start, home), local_to_utc(end, home));
            set_server_times(state, source, |m| {
                (m.start.moved_to(start, home), m.end.moved_to(end, home))
            })
        }
    }
}

/// Termin um ganze Tage verschieben (Monatsansicht); Uhrzeiten bleiben.
pub fn shift_days(state: &mut State, item: &Item, days: i64) -> Result<(), String> {
    if !item.movable() {
        return Err("Dieser Termin kann hier nicht verschoben werden.".into());
    }
    let delta = Duration::days(days);
    let home = state.home;
    match &item.source {
        Source::DemoEvent(id) => {
            if let Some(e) = state.events.iter_mut().find(|e| e.id == *id) {
                e.start += delta;
                e.end += delta;
            }
            Ok(())
        }
        Source::DemoAllDay(id) => {
            if let Some(a) = state.all_day.iter_mut().find(|a| a.id == *id) {
                a.first += delta;
                a.end += delta;
            }
            Ok(())
        }
        source => set_server_times(state, source, |m| {
            (
                shift_value(&m.start, delta, home),
                shift_value(&m.end, delta, home),
            )
        }),
    }
}

pub fn delete(state: &mut State, item: &Item) -> Result<(), String> {
    match &item.source {
        Source::DemoEvent(id) => state.events.retain(|e| e.id != *id),
        Source::DemoAllDay(id) => state.all_day.retain(|a| a.id != *id),
        Source::Server {
            read_only: true, ..
        } => return Err("Dieser Kalender ist schreibgeschützt.".into()),
        Source::Server {
            calendar_id, href, ..
        } => {
            state
                .store
                .local_delete(*calendar_id, href)
                .map_err(|e| e.to_string())?;
            state.send(Command::Push);
        }
        // Aufgaben löscht die Aufgaben-Ansicht (mit Teilaufgaben)
        Source::Task { .. } => return Err("Aufgaben bitte in der Aufgaben-Ansicht löschen.".into()),
    }
    Ok(())
}

fn title_or_default(title: &str) -> String {
    if title.trim().is_empty() {
        "Neuer Termin".to_string()
    } else {
        title.trim().to_string()
    }
}

/// Beispieltermin anlegen oder ersetzen (ganztägig oder mit Uhrzeit).
fn put_demo(
    state: &mut State,
    id: Option<i32>,
    input: &DialogInput,
    start: &DateTimeValue,
    end: &DateTimeValue,
) {
    if let Some(id) = id {
        state.events.retain(|e| e.id != id);
        state.all_day.retain(|a| a.id != id);
    }
    let id = id.unwrap_or_else(|| {
        state.next_id += 1;
        state.next_id - 1
    });
    let color = 0x4f7cac;
    match (start, end) {
        (DateTimeValue::Date(first), DateTimeValue::Date(after)) => {
            state.all_day.push(sample::AllDay {
                id,
                title: title_or_default(&input.title),
                color,
                first: *first,
                end: *after,
            })
        }
        _ => state.events.push(sample::Event {
            id,
            title: title_or_default(&input.title),
            location: input.location.trim().to_string(),
            color,
            start: start.naive(),
            end: end.naive(),
        }),
    }
}

fn dialog_fields(input: &DialogInput, start: DateTimeValue, end: DateTimeValue) -> EventFields {
    EventFields {
        summary: title_or_default(&input.title),
        location: input.location.trim().to_string(),
        description: input.description.trim_end().to_string(),
        start,
        end,
    }
}

/// Wiederholung aus dem Dialog lesen; das Ende darf nicht vor dem Beginn liegen.
fn read_repeat(
    input: &DialogInput,
    start: &DateTimeValue,
    today: chrono::NaiveDate,
) -> Result<Repeat, String> {
    let repeat = input.rule.read(today)?;
    if repeat.ends_before(start.naive().date()) {
        return Err("Die Wiederholung endet vor dem Beginn.".into());
    }
    Ok(repeat)
}

/// Prüft den Dialog vor dem Serien-Dialog. Liefert, ob die Regel geändert
/// wurde (dann ergibt "nur dieser Termin" keinen Sinn).
pub fn check_series_save(state: &State, item: &Item, input: &DialogInput) -> Result<bool, String> {
    let today = state.now().date();
    let home = state.home;
    let Loaded { master, shown, .. } = load(state, &item.source)?;
    let (start, _) = input.times.to_values(today, home, Some(&shown.start))?;
    let before = RuleFields::from_rule(master.rrule.as_deref(), &master.start, home).read(today)?;
    let after = read_repeat(input, &start, today)?;
    Ok(before != after)
}

/// Dialog speichern: `editing` ist der bearbeitete Termin, `None` bei neuen.
/// Vorkommen von Serien gehen über [`SeriesAction::Save`].
pub fn save(state: &mut State, editing: Option<&Item>, input: &DialogInput) -> Result<(), String> {
    let today = state.now().date();
    let home = state.home;
    match editing.map(|i| &i.source) {
        None => {
            let (start, end) = input.times.to_values(today, home, None)?;
            let repeat = read_repeat(input, &start, today)?;
            if input.calendar < 0 {
                if repeat != Repeat::Never {
                    return Err("Beispieltermine können sich nicht wiederholen.".into());
                }
                put_demo(state, None, input, &start, &end);
                return Ok(());
            }
            let uid = new_uid();
            let href = new_href(state, input.calendar, &uid)?;
            let rule = repeat.rrule(&start, home);
            let mut object = edit::new_event(&uid, &dialog_fields(input, start, end), Utc::now());
            if rule.is_some() {
                series::set_rule(&mut object, &uid, rule.as_deref()).map_err(|e| e.to_string())?;
            }
            if let Some(reminders) = &input.reminders {
                reminders.apply(&mut object, &uid, None, home, false);
            }
            let _ = state
                .store
                .set_setting("default_calendar", &input.calendar.to_string());
            store_object(state, input.calendar, &href, &object)
        }
        Some(Source::DemoEvent(id) | Source::DemoAllDay(id)) => {
            let (start, end) = input.times.to_values(today, home, None)?;
            if read_repeat(input, &start, today)? != Repeat::Never {
                return Err("Beispieltermine können sich nicht wiederholen.".into());
            }
            put_demo(state, Some(*id), input, &start, &end);
            Ok(())
        }
        Some(Source::Server {
            read_only: true, ..
        }) => Err("Dieser Kalender ist schreibgeschützt.".into()),
        Some(source) => {
            let (calendar_id, href, uid, _) = server_ids(source).expect("Server-Termin");
            let Loaded {
                mut object, master, ..
            } = load(state, source)?;
            let (start, end) = input.times.to_values(today, home, Some(&master.start))?;
            let after = read_repeat(input, &start, today)?;
            let before =
                RuleFields::from_rule(master.rrule.as_deref(), &master.start, home).read(today)?;
            let change = rule_change(master.rrule.is_some(), &before, &after, &start, home);
            let fields = dialog_fields(input, start, end);
            edit::update_event(&mut object, uid, &fields, Utc::now()).map_err(|e| e.to_string())?;
            match change {
                RuleChange::Keep => {}
                RuleChange::Set(rule) => {
                    series::set_rule(&mut object, uid, Some(&rule)).map_err(|e| e.to_string())?
                }
                RuleChange::Remove => {
                    series::set_rule(&mut object, uid, None).map_err(|e| e.to_string())?
                }
            }
            if let Some(reminders) = &input.reminders {
                reminders.apply(&mut object, uid, None, home, false);
            }
            let href = href.to_string();
            store_object(state, calendar_id, &href, &object)
        }
    }
}

/// Eine Änderung an einem Vorkommen einer Serie mit dem gewählten Umfang ausführen.
pub fn apply_series(state: &mut State, action: &SeriesAction, scope: Scope) -> Result<(), String> {
    let item = action.item();
    if !item.writable() {
        return Err("Dieser Kalender ist schreibgeschützt.".into());
    }
    let Some((calendar_id, href, uid, Some(instance))) = server_ids(&item.source) else {
        return Err("Kein Termin einer Serie.".into());
    };
    let (href, uid) = (href.to_string(), uid.to_string());
    let home = state.home;
    let today = state.now().date();
    let now = Utc::now();
    let Loaded {
        mut object,
        master,
        shown,
    } = load(state, &item.source)?;
    let target = Target {
        uid: &uid,
        instance,
    };

    let (fields, rule) = match action {
        SeriesAction::Delete { .. } => {
            return match series::delete(&mut object, target, scope, home, now)
                .map_err(|e| e.to_string())?
            {
                DeleteOutcome::Changed => store_object(state, calendar_id, &href, &object),
                DeleteOutcome::RemoveResource => delete(state, item),
            };
        }
        SeriesAction::Move { start, end, .. } => {
            let (start, end) = (local_to_utc(*start, home), local_to_utc(*end, home));
            let fields = fields_with_times(
                &shown,
                shown.start.moved_to(start, home),
                shown.end.moved_to(end, home),
            );
            (fields, RuleChange::Keep)
        }
        SeriesAction::ShiftDays { days, .. } => {
            let delta = Duration::days(*days);
            let fields = fields_with_times(
                &shown,
                shift_value(&shown.start, delta, home),
                shift_value(&shown.end, delta, home),
            );
            (fields, RuleChange::Keep)
        }
        SeriesAction::Save { input, .. } => {
            let (start, end) = input.times.to_values(today, home, Some(&shown.start))?;
            let after = read_repeat(input, &start, today)?;
            let before =
                RuleFields::from_rule(master.rrule.as_deref(), &master.start, home).read(today)?;
            // Eine neue Regel gilt ab dem Beginn der (ggf. neuen) Serie
            let rule_start = match scope {
                Scope::All => shift_value(&master.start, start.naive() - shown.start.naive(), home),
                _ => start,
            };
            let rule = rule_change(true, &before, &after, &rule_start, home);
            (dialog_fields(input, start, end), rule)
        }
    };
    let new_uid = new_uid();
    let new = series::edit(
        &mut object,
        target,
        scope,
        &fields,
        &rule,
        &new_uid,
        home,
        now,
    )
    .map_err(|e| e.to_string())?;
    let mut new = new;
    if let SeriesAction::Save { input, .. } = action
        && let Some(reminders) = &input.reminders
    {
        match (&mut new, scope) {
            (Some(new), _) => reminders.apply(new, &new_uid, None, home, true),
            (None, Scope::This) => reminders.apply(&mut object, &uid, Some(instance), home, false),
            (None, _) => reminders.apply(&mut object, &uid, None, home, true),
        }
    }
    // Neue Serie zuerst anlegen: Schlägt das Kürzen der alten fehl, ist nichts verloren
    if let Some(new) = new {
        let new_href = new_href(state, calendar_id, &new_uid)?;
        store_object(state, calendar_id, &new_href, &new)?;
    }
    store_object(state, calendar_id, &href, &object)
}

/// Alarme des angezeigten Vorkommens (für die Erinnerungen im Dialog)
pub fn alarms(state: &mut State, item: &Item) -> Vec<Alarm> {
    shown_event(state, item)
        .map(|e| e.alarms)
        .unwrap_or_default()
}

/// Das angezeigte Vorkommen eines Server-Termins (Ausnahme oder Serie)
fn shown_event(state: &mut State, item: &Item) -> Option<EventInfo> {
    let (calendar_id, href, uid, instance) = server_ids(&item.source)?;
    let resource = state.store.resource(calendar_id, href).ok().flatten()?;
    let home = state.home;
    let events = crate::items::parsed_events(&mut state.parsed, calendar_id, href, &resource.ics);
    events
        .iter()
        .find(|e| {
            e.uid == uid
                && e.recurrence_id
                    .zip(instance)
                    .is_some_and(|(r, i)| r.to_utc(home) == i)
        })
        .or_else(|| {
            events
                .iter()
                .find(|e| e.uid == uid && e.recurrence_id.is_none())
        })
        .cloned()
}

/// Beschreibung eines Server-Termins aus dem Cache (für Vorschau und Dialog).
pub fn description(state: &mut State, item: &Item) -> String {
    shown_event(state, item)
        .map(|e| e.description)
        .unwrap_or_default()
}

/// Regel der Serie eines Termins (für Vorschau und Dialog): Wert und Beginn
pub fn series_rule(state: &mut State, item: &Item) -> Option<(String, DateTimeValue)> {
    let (calendar_id, href, uid, _) = server_ids(&item.source)?;
    let resource = state.store.resource(calendar_id, href).ok().flatten()?;
    crate::items::parsed_events(&mut state.parsed, calendar_id, href, &resource.ics)
        .iter()
        .find(|e| e.uid == uid && e.recurrence_id.is_none())
        .and_then(|e| e.rrule.clone().map(|r| (r, e.start)))
}

/// Felder des Dialogs für einen bestehenden Termin (Wandzeiten in `home`).
pub fn draft_times(state: &State, item: &Item) -> DraftTimes {
    let home = state.home;
    match item.span.dates {
        Some((first, after)) => DraftTimes::from_values(
            &DateTimeValue::Date(first),
            &DateTimeValue::Date(after),
            home,
        ),
        None => {
            let local = |t: chrono::DateTime<Utc>| t.with_timezone(&home).naive_local();
            DraftTimes::from_values(
                &DateTimeValue::Floating(local(item.span.start)),
                &DateTimeValue::Floating(local(item.span.end)),
                home,
            )
        }
    }
}

/// Vorschlag für einen neuen Termin: `minutes` Minuten ab `start`.
pub fn new_times(start: NaiveDateTime, minutes: i64) -> DraftTimes {
    DraftTimes::from_values(
        &DateTimeValue::Floating(start),
        &DateTimeValue::Floating(start + Duration::minutes(minutes)),
        chrono_tz::UTC,
    )
}

/// Nächste volle Stunde nach `now`, nicht über 23:00 hinaus
pub fn next_full_hour(now: NaiveDateTime) -> NaiveDateTime {
    let hour = (now.hour() + 1).min(23);
    now.date()
        .and_time(NaiveTime::from_hms_opt(hour, 0, 0).unwrap())
}
