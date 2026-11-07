//! Serien bearbeiten: nur dieses Vorkommen, dieses und alle folgenden, alle.
//!
//! - **Nur dieser:** eine Ausnahme (VEVENT mit RECURRENCE-ID) anlegen oder
//!   ändern; Löschen setzt ein EXDATE.
//! - **Dieser und alle folgenden:** die Serie wird geteilt. Die alte endet
//!   vor dem Vorkommen (UNTIL bzw. kleineres COUNT), eine neue Serie mit
//!   eigener UID beginnt dort. Ausnahmen und EXDATEs ab dem Vorkommen ziehen
//!   mit um. So machen es auch Thunderbird und Google.
//! - **Alle:** die Serie wird um dieselbe Wandzeit-Differenz verschoben wie
//!   das bearbeitete Vorkommen; RECURRENCE-ID, EXDATE, RDATE und Wochentage
//!   der Regel wandern mit, damit Ausnahmen ihre Termine behalten.
//!
//! Maßstab ist immer das Vorkommen, wie es angezeigt wurde: Wer an einer
//! bereits verlegten Ausnahme nur den Ort ändert und "alle" wählt, verschiebt
//! die Serie nicht und benennt sie auch nicht um. Texte gehen nur an
//! Ausnahmen, die noch den Text der Serie hatten.
//!
//! Alle Wandzeiten werden in der Zone der Serie gerechnet, damit "8 Uhr" über
//! die Zeitumstellung hinweg 8 Uhr bleibt.

use std::collections::HashSet;

use chrono::{DateTime, Duration, NaiveDateTime, NaiveTime, Utc};
use chrono_tz::Tz;

use crate::edit::{
    EditError, EventFields, PRODID, ensure_timezones, set_text, time_property, touch, write_times,
};
use crate::expand::expand;
use crate::ical::{Component, Property, split_values};
use crate::model::{CalendarObject, EventInfo};
use crate::recur::{RuleParts, parse_until, until_value};
use crate::time::{DateTimeValue, local_to_utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    This,
    Following,
    All,
}

/// Was mit der Wiederholungsregel geschehen soll
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleChange {
    Keep,
    Set(String),
    Remove,
}

/// Ein Vorkommen einer Serie
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    pub uid: &'a str,
    /// Ursprünglicher Beginn (Wert der RECURRENCE-ID) als Zeitpunkt
    pub instance: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    /// Das Objekt wurde geändert und muss gespeichert werden
    Changed,
    /// Die ganze Ressource soll gelöscht werden
    RemoveResource,
}

const SERIES_PROPERTIES: [&str; 4] = ["RRULE", "RDATE", "EXDATE", "EXRULE"];

fn is_event_of(component: &Component, uid: &str) -> bool {
    component.is("VEVENT")
        && component
            .property("UID")
            .is_some_and(|p| p.value.trim() == uid)
}

fn master_index(object: &CalendarObject, uid: &str) -> Result<usize, EditError> {
    object
        .root
        .components
        .iter()
        .position(|c| is_event_of(c, uid) && c.property("RECURRENCE-ID").is_none())
        .ok_or_else(|| EditError::NotFound(uid.to_string()))
}

fn master_info(object: &CalendarObject, index: usize) -> Result<EventInfo, EditError> {
    let component = &object.root.components[index];
    EventInfo::from_component(component, &object.root)
        .map_err(|_| EditError::NotFound(component.text_of("UID")))
}

/// Ursprünglicher Beginn einer Ausnahme als Zeitpunkt
fn recurrence_instant(component: &Component, root: &Component, home: Tz) -> Option<DateTime<Utc>> {
    let property = component.property("RECURRENCE-ID")?;
    Some(
        DateTimeValue::from_property(property, Some(root))
            .ok()?
            .to_utc(home),
    )
}

/// Indizes der Ausnahmen einer Serie mit ihrem ursprünglichen Beginn
fn overrides(object: &CalendarObject, uid: &str, home: Tz) -> Vec<(usize, DateTime<Utc>)> {
    object
        .root
        .components
        .iter()
        .enumerate()
        .filter(|(_, c)| is_event_of(c, uid))
        .filter_map(|(i, c)| recurrence_instant(c, &object.root, home).map(|t| (i, t)))
        .collect()
}

/// Wandzeit eines Werts in der Zone `tz`; ganztägige Werte um Mitternacht.
fn wall(value: &DateTimeValue, tz: Tz, home: Tz) -> NaiveDateTime {
    match value {
        DateTimeValue::Date(d) => d.and_time(NaiveTime::MIN),
        other => other.to_utc(home).with_timezone(&tz).naive_local(),
    }
}

/// Wert vom Typ (und in der Zone) von `like` zur Wandzeit `local` in `tz`.
fn value_in(like: &DateTimeValue, local: NaiveDateTime, tz: Tz, home: Tz) -> DateTimeValue {
    match like {
        DateTimeValue::Date(_) => DateTimeValue::Date(local.date()),
        _ => like.moved_to(local_to_utc(local, tz), home),
    }
}

/// Wie viele Vorkommen die Regel vor `instance` erzeugt (EXDATE zählt mit,
/// wie bei COUNT laut RFC 5545).
fn count_before(info: &EventInfo, instance: DateTime<Utc>, home: Tz) -> u32 {
    let mut plain = info.clone();
    plain.exdates.clear();
    plain.rdates.clear();
    let from = info.start.to_utc(home) - Duration::seconds(1);
    expand(&[plain], home, from, instance)
        .iter()
        .filter(|o| o.start < instance)
        .count() as u32
}

/// EXDATE- bzw. RDATE-Werte einer Komponente einzeln lesen.
fn date_values(
    component: &Component,
    name: &str,
    root: &Component,
) -> Vec<(Property, DateTimeValue)> {
    let mut values = Vec::new();
    for property in component.properties_named(name) {
        if property
            .param("VALUE")
            .is_some_and(|v| v.eq_ignore_ascii_case("PERIOD"))
        {
            continue;
        }
        for part in split_values(&property.value) {
            let mut single = property.clone();
            single.value = part.trim().to_string();
            if let Ok(value) = DateTimeValue::from_property(&single, Some(root)) {
                values.push((single, value));
            }
        }
    }
    values
}

/// Wie sich das bearbeitete Vorkommen geändert hat, gemessen an dem, was
/// angezeigt wurde (die Ausnahme, falls es eine gibt).
struct Change {
    /// Zone der Serie
    tz: Tz,
    /// Verschiebung des Beginns in Wandzeit
    delta: Duration,
    /// Verschiebung in Kalendertagen (für BYDAY)
    days: i64,
    /// Neue Dauer der Serie
    length: Duration,
    /// Index der Ausnahme dieses Vorkommens
    exception: Option<usize>,
    /// Das Vorkommen vor der Änderung
    shown: EventInfo,
}

impl Change {
    fn of(
        object: &CalendarObject,
        master: &EventInfo,
        target: Target,
        fields: &EventFields,
        home: Tz,
    ) -> Change {
        let tz = master.start.zone(home);
        let exception = overrides(object, target.uid, home)
            .into_iter()
            .find(|(_, t)| *t == target.instance)
            .map(|(i, _)| i);
        let master_length = wall(&master.end, tz, home) - wall(&master.start, tz, home);
        let shown = exception
            .and_then(|i| EventInfo::from_component(&object.root.components[i], &object.root).ok())
            .unwrap_or_else(|| {
                let mut info = master.clone();
                info.start = master.start.moved_to(target.instance, home);
                info.end = value_in(
                    &master.end,
                    wall(&info.start, tz, home) + master_length,
                    tz,
                    home,
                );
                info
            });
        let old_start = wall(&shown.start, tz, home);
        let old_length = wall(&shown.end, tz, home) - old_start;
        let new_start = wall(&fields.start, tz, home);
        let new_length = wall(&fields.end, tz, home) - new_start;
        Change {
            tz,
            delta: new_start - old_start,
            days: (new_start.date() - old_start.date()).num_days(),
            length: master_length + new_length - old_length,
            exception,
            shown,
        }
    }
}

/// Geänderte Texte übernehmen: in die Serie immer, in Ausnahmen nur, wenn sie
/// noch den alten Text der Serie hatten.
fn apply_texts(
    component: &mut Component,
    fields: &EventFields,
    shown: &EventInfo,
    master: &EventInfo,
    is_master: bool,
) {
    for (name, new, seen, of_master) in [
        ("SUMMARY", &fields.summary, &shown.summary, &master.summary),
        (
            "LOCATION",
            &fields.location,
            &shown.location,
            &master.location,
        ),
        (
            "DESCRIPTION",
            &fields.description,
            &shown.description,
            &master.description,
        ),
    ] {
        if new != seen && (is_master || component.text_of(name) == *of_master) {
            set_text(component, name, new);
        }
    }
}

fn set_all_texts(component: &mut Component, fields: &EventFields) {
    set_text(component, "SUMMARY", &fields.summary);
    set_text(component, "LOCATION", &fields.location);
    set_text(component, "DESCRIPTION", &fields.description);
}

/// Regel nach einer Verschiebung um `delta` (Wandzeit) bzw. `days` Tage.
fn shifted_rule(rule: &str, days: i64, delta: Duration, start: &DateTimeValue, home: Tz) -> String {
    let mut parts = RuleParts::parse(rule);
    parts.shift_days(days);
    if let Some(until) = parts.get("UNTIL").and_then(|u| parse_until(u, start, home)) {
        parts.set("UNTIL", until_value(start, until + delta, home));
    }
    parts.to_string()
}

/// RECURRENCE-ID, EXDATE und RDATE mit `shift` umrechnen.
fn shift_dates(
    object: &mut CalendarObject,
    uid: &str,
    shift: impl Fn(&DateTimeValue) -> DateTimeValue,
) {
    let root = object.root.clone();
    for component in object
        .root
        .components
        .iter_mut()
        .filter(|c| is_event_of(c, uid))
    {
        if let Some(rid) = component
            .property("RECURRENCE-ID")
            .and_then(|p| DateTimeValue::from_property(p, Some(&root)).ok())
        {
            let property = time_property(
                "RECURRENCE-ID",
                &shift(&rid),
                component.property("RECURRENCE-ID"),
            );
            component.set_property(property);
            continue;
        }
        for name in ["EXDATE", "RDATE"] {
            let values = date_values(component, name, &root);
            if values.is_empty() {
                continue;
            }
            component.remove_properties(name);
            for (property, value) in values {
                component
                    .properties
                    .push(time_property(name, &shift(&value), Some(&property)));
            }
        }
    }
}

/// Nach einer neuen Regel: Ausnahmen und EXDATEs entfernen, die zu keinem
/// Vorkommen mehr gehören (sonst stünden sie als verwaiste Termine da).
fn drop_unmatched(object: &mut CalendarObject, uid: &str, home: Tz) -> Result<(), EditError> {
    let master = master_index(object, uid)?;
    let info = master_info(object, master)?;
    let exceptions = overrides(object, uid, home);
    let exdates: Vec<DateTime<Utc>> = info.exdates.iter().map(|d| d.to_utc(home)).collect();
    let all: Vec<DateTime<Utc>> = exceptions
        .iter()
        .map(|(_, t)| *t)
        .chain(exdates.iter().copied())
        .collect();
    let (Some(first), Some(last)) = (all.iter().min(), all.iter().max()) else {
        return Ok(());
    };
    let mut plain = info.clone();
    plain.exdates.clear();
    let valid: HashSet<DateTime<Utc>> = expand(
        &[plain],
        home,
        *first - Duration::seconds(1),
        *last + Duration::seconds(1),
    )
    .into_iter()
    .map(|o| o.start)
    .collect();
    for (index, _) in exceptions.iter().rev().filter(|(_, t)| !valid.contains(t)) {
        object.root.components.remove(*index);
    }
    let master = master_index(object, uid)?;
    let root = object.root.clone();
    let component = &mut object.root.components[master];
    let values = date_values(component, "EXDATE", &root);
    component.remove_properties("EXDATE");
    for (property, value) in values {
        if valid.contains(&value.to_utc(home)) {
            component.properties.push(property);
        }
    }
    Ok(())
}

fn edit_this(
    object: &mut CalendarObject,
    target: Target,
    fields: &EventFields,
    home: Tz,
    now: DateTime<Utc>,
) -> Result<(), EditError> {
    let master = master_index(object, target.uid)?;
    let info = master_info(object, master)?;
    let index = match overrides(object, target.uid, home)
        .into_iter()
        .find(|(_, t)| *t == target.instance)
    {
        Some((i, _)) => i,
        None => {
            // Ausnahme als Kopie der Serie: Alarme, Teilnehmer usw. bleiben
            let mut copy = object.root.components[master].clone();
            for name in SERIES_PROPERTIES {
                copy.remove_properties(name);
            }
            let rid = info.start.moved_to(target.instance, home);
            copy.properties
                .push(time_property("RECURRENCE-ID", &rid, None));
            object.root.components.insert(master + 1, copy);
            master + 1
        }
    };
    let component = &mut object.root.components[index];
    set_all_texts(component, fields);
    write_times(component, &fields.start, &fields.end);
    touch(component, now);
    ensure_timezones(object);
    Ok(())
}

/// Aus der Serie einen einzelnen Termin machen
fn make_single(
    object: &mut CalendarObject,
    uid: &str,
    fields: &EventFields,
    now: DateTime<Utc>,
) -> Result<(), EditError> {
    object
        .root
        .components
        .retain(|c| !is_event_of(c, uid) || c.property("RECURRENCE-ID").is_none());
    let master = master_index(object, uid)?;
    let component = &mut object.root.components[master];
    for name in SERIES_PROPERTIES {
        component.remove_properties(name);
    }
    set_all_texts(component, fields);
    write_times(component, &fields.start, &fields.end);
    touch(component, now);
    ensure_timezones(object);
    Ok(())
}

fn edit_all(
    object: &mut CalendarObject,
    target: Target,
    fields: &EventFields,
    rule: &RuleChange,
    home: Tz,
    now: DateTime<Utc>,
) -> Result<(), EditError> {
    if *rule == RuleChange::Remove {
        return make_single(object, target.uid, fields, now);
    }
    let master = master_index(object, target.uid)?;
    let info = master_info(object, master)?;
    let change = Change::of(object, &info, target, fields, home);
    let tz = change.tz;
    let master_start = wall(&info.start, tz, home) + change.delta;
    let start = value_in(&fields.start, master_start, tz, home);
    let end = value_in(&fields.end, master_start + change.length, tz, home);

    shift_dates(object, target.uid, |v| {
        value_in(&start, wall(v, tz, home) + change.delta, tz, home)
    });
    for (index, _) in overrides(object, target.uid, home) {
        let component = &mut object.root.components[index];
        if Some(index) == change.exception {
            // Das bearbeitete Vorkommen selbst zeigt genau das Eingegebene
            set_all_texts(component, fields);
            write_times(component, &fields.start, &fields.end);
        } else {
            apply_texts(component, fields, &change.shown, &info, false);
        }
        touch(component, now);
    }

    let component = &mut object.root.components[master];
    match rule {
        RuleChange::Set(new_rule) => {
            component.set_property(Property::new("RRULE", new_rule.clone()))
        }
        _ => {
            if let Some(existing) = component.property("RRULE").map(|p| p.value.clone()) {
                component.set_property(Property::new(
                    "RRULE",
                    shifted_rule(&existing, change.days, change.delta, &start, home),
                ));
            }
        }
    }
    apply_texts(component, fields, &change.shown, &info, true);
    write_times(component, &start, &end);
    touch(component, now);
    if matches!(rule, RuleChange::Set(_)) {
        drop_unmatched(object, target.uid, home)?;
    }
    ensure_timezones(object);
    Ok(())
}

/// Serie vor `instance` enden lassen. Ausnahmen und EXDATE/RDATE ab dort
/// werden entfernt und zurückgegeben.
fn truncate(
    object: &mut CalendarObject,
    master: usize,
    info: &EventInfo,
    instance: DateTime<Utc>,
    home: Tz,
) -> (Vec<Component>, Vec<(String, Property, DateTimeValue)>) {
    let root = object.root.clone();
    let component = &mut object.root.components[master];
    if let Some(rule) = component.property("RRULE").map(|p| p.value.clone()) {
        let mut parts = RuleParts::parse(&rule);
        if parts.get("COUNT").is_some() {
            parts.set(
                "COUNT",
                count_before(info, instance, home).max(1).to_string(),
            );
        } else {
            parts.set(
                "UNTIL",
                until_value(&info.start, instance - Duration::seconds(1), home),
            );
        }
        component.set_property(Property::new("RRULE", parts.to_string()));
    }
    let mut moved_dates = Vec::new();
    for name in ["EXDATE", "RDATE"] {
        let values = date_values(component, name, &root);
        if values.is_empty() {
            continue;
        }
        component.remove_properties(name);
        for (property, value) in values {
            if value.to_utc(home) >= instance {
                moved_dates.push((name.to_string(), property, value));
            } else {
                component.properties.push(property);
            }
        }
    }
    let later: Vec<usize> = overrides(object, &info.uid, home)
        .into_iter()
        .filter(|(_, t)| *t >= instance)
        .map(|(i, _)| i)
        .collect();
    let mut moved = Vec::new();
    for index in later.into_iter().rev() {
        moved.insert(0, object.root.components.remove(index));
    }
    (moved, moved_dates)
}

fn split(
    object: &mut CalendarObject,
    target: Target,
    fields: &EventFields,
    rule: &RuleChange,
    new_uid: &str,
    home: Tz,
    now: DateTime<Utc>,
) -> Result<CalendarObject, EditError> {
    let master = master_index(object, target.uid)?;
    let info = master_info(object, master)?;
    let change = Change::of(object, &info, target, fields, home);
    let tz = change.tz;
    let single = *rule == RuleChange::Remove;
    // Die neue Serie beginnt dort, wo das Vorkommen nach der Regel läge
    let first = wall(&info.start.moved_to(target.instance, home), tz, home) + change.delta;
    let (start, end) = if single {
        (fields.start, fields.end)
    } else {
        (
            value_in(&fields.start, first, tz, home),
            value_in(&fields.end, first + change.length, tz, home),
        )
    };
    let shift = |v: &DateTimeValue| value_in(&start, wall(v, tz, home) + change.delta, tz, home);

    let mut series = object.root.components[master].clone();
    series.set_property(Property::new("UID", new_uid));
    for name in ["EXDATE", "RDATE"] {
        series.remove_properties(name);
    }
    let old_rule = series.property("RRULE").map(|p| p.value.clone());
    match (rule, old_rule) {
        (RuleChange::Set(new_rule), _) => {
            series.set_property(Property::new("RRULE", new_rule.clone()))
        }
        (RuleChange::Remove, _) | (RuleChange::Keep, None) => series.remove_properties("RRULE"),
        (RuleChange::Keep, Some(existing)) => {
            let mut parts = RuleParts::parse(&shifted_rule(
                &existing,
                change.days,
                change.delta,
                &start,
                home,
            ));
            if let Some(total) = parts.get("COUNT").and_then(|c| c.parse::<u32>().ok()) {
                let before = count_before(&info, target.instance, home);
                parts.set("COUNT", total.saturating_sub(before).max(1).to_string());
            }
            series.set_property(Property::new("RRULE", parts.to_string()));
        }
    }
    if single {
        set_all_texts(&mut series, fields);
    } else {
        apply_texts(&mut series, fields, &change.shown, &info, true);
    }
    write_times(&mut series, &start, &end);
    touch(&mut series, now);
    series.set_property(Property::new("SEQUENCE", "0"));

    // Alte Serie kürzen; spätere Ausnahmen und Datumslisten ziehen um
    let original = object.root.clone();
    let (moved, moved_dates) = truncate(object, master, &info, target.instance, home);
    touch(&mut object.root.components[master], now);

    let mut root = Component::new("VCALENDAR");
    root.properties.push(Property::new("VERSION", "2.0"));
    root.properties.push(Property::new("PRODID", PRODID));
    root.properties.push(Property::new("CALSCALE", "GREGORIAN"));
    // Zeitzonen des Originals übernehmen (auch solche ohne IANA-Namen)
    root.components
        .extend(original.children_named("VTIMEZONE").cloned());
    if !single {
        for (name, property, value) in moved_dates {
            series
                .properties
                .push(time_property(&name, &shift(&value), Some(&property)));
        }
    }
    root.components.push(series);
    if !single {
        for mut component in moved {
            let Some(rid) = component
                .property("RECURRENCE-ID")
                .and_then(|p| DateTimeValue::from_property(p, Some(&original)).ok())
            else {
                continue;
            };
            component.set_property(Property::new("UID", new_uid));
            let property = time_property(
                "RECURRENCE-ID",
                &shift(&rid),
                component.property("RECURRENCE-ID"),
            );
            component.set_property(property);
            if rid.to_utc(home) == target.instance {
                set_all_texts(&mut component, fields);
                write_times(&mut component, &fields.start, &fields.end);
            } else {
                apply_texts(&mut component, fields, &change.shown, &info, false);
            }
            touch(&mut component, now);
            root.components.push(component);
        }
    }
    let mut new_object = CalendarObject { root };
    if matches!(rule, RuleChange::Set(_)) {
        drop_unmatched(&mut new_object, new_uid, home)?;
    }
    ensure_timezones(&mut new_object);
    ensure_timezones(object);
    Ok(new_object)
}

/// Vorkommen `target` ändern. Bei [`Scope::Following`] entsteht ab dem
/// zweiten Vorkommen eine neue Serie, die zurückgegeben wird (neue Ressource
/// mit UID `new_uid`); sonst `None`.
#[allow(clippy::too_many_arguments)]
pub fn edit(
    object: &mut CalendarObject,
    target: Target,
    scope: Scope,
    fields: &EventFields,
    rule: &RuleChange,
    new_uid: &str,
    home: Tz,
    now: DateTime<Utc>,
) -> Result<Option<CalendarObject>, EditError> {
    match scope {
        Scope::This => edit_this(object, target, fields, home, now).map(|_| None),
        Scope::All => edit_all(object, target, fields, rule, home, now).map(|_| None),
        Scope::Following => {
            let info = master_info(object, master_index(object, target.uid)?)?;
            if target.instance <= info.start.to_utc(home) {
                // Ab dem ersten Vorkommen ist "alle folgenden" dasselbe wie "alle"
                return edit_all(object, target, fields, rule, home, now).map(|_| None);
            }
            split(object, target, fields, rule, new_uid, home, now).map(Some)
        }
    }
}

/// Vorkommen `target` löschen.
pub fn delete(
    object: &mut CalendarObject,
    target: Target,
    scope: Scope,
    home: Tz,
    now: DateTime<Utc>,
) -> Result<DeleteOutcome, EditError> {
    let master = master_index(object, target.uid)?;
    let info = master_info(object, master)?;
    match scope {
        Scope::All => Ok(DeleteOutcome::RemoveResource),
        Scope::Following if target.instance <= info.start.to_utc(home) => {
            Ok(DeleteOutcome::RemoveResource)
        }
        Scope::Following => {
            truncate(object, master, &info, target.instance, home);
            touch(&mut object.root.components[master], now);
            Ok(DeleteOutcome::Changed)
        }
        Scope::This => {
            if let Some((index, _)) = overrides(object, target.uid, home)
                .into_iter()
                .find(|(_, t)| *t == target.instance)
            {
                object.root.components.remove(index);
            }
            let master = master_index(object, target.uid)?;
            let rid = info.start.moved_to(target.instance, home);
            let component = &mut object.root.components[master];
            component
                .properties
                .push(time_property("EXDATE", &rid, None));
            touch(component, now);
            ensure_timezones(object);
            Ok(DeleteOutcome::Changed)
        }
    }
}

/// Regel eines (bisher einzelnen) Termins setzen oder entfernen.
pub fn set_rule(
    object: &mut CalendarObject,
    uid: &str,
    rule: Option<&str>,
) -> Result<(), EditError> {
    let master = master_index(object, uid)?;
    let component = &mut object.root.components[master];
    match rule {
        Some(rule) => component.set_property(Property::new("RRULE", rule)),
        None => {
            for name in SERIES_PROPERTIES {
                component.remove_properties(name);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expand::Occurrence;
    use chrono::{NaiveDate, TimeZone};
    use chrono_tz::Europe::Berlin;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 4, 12, 0, 0).unwrap()
    }

    fn naive(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn berlin(s: &str) -> DateTimeValue {
        DateTimeValue::Zoned(naive(s), Berlin)
    }

    /// Zeitpunkt einer Berliner Wandzeit
    fn at(s: &str) -> DateTime<Utc> {
        berlin(s).to_utc(Berlin)
    }

    /// Jeden Montag 8:00–9:30 ab 2.11.2026, mit Alarm und fremder Eigenschaft
    fn weekly(rule: &str, extra: &str) -> CalendarObject {
        CalendarObject::parse(&format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//DE\r\nBEGIN:VEVENT\r\nUID:mathe\r\nDTSTAMP:20261101T000000Z\r\n\
             DTSTART;TZID=Europe/Berlin:20261102T080000\r\nDTEND;TZID=Europe/Berlin:20261102T093000\r\nRRULE:{rule}\r\n\
             SUMMARY:Mathe\r\nLOCATION:Raum 204\r\nX-FREMD:bleibt\r\n{extra}BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\n\
             END:VEVENT\r\nEND:VCALENDAR\r\n"
        ))
        .unwrap()
    }

    fn occurrences(objects: &[&CalendarObject], from: &str, to: &str) -> Vec<Occurrence> {
        let mut all: Vec<Occurrence> = objects
            .iter()
            .flat_map(|o| expand(&o.events().0, Berlin, at(from), at(to)))
            .collect();
        all.sort_by_key(|o| o.start);
        all
    }

    fn starts(objects: &[&CalendarObject], from: &str, to: &str) -> Vec<DateTime<Utc>> {
        occurrences(objects, from, to)
            .iter()
            .map(|o| o.start)
            .collect()
    }

    fn summaries(
        objects: &[&CalendarObject],
        from: &str,
        to: &str,
    ) -> Vec<(DateTime<Utc>, String)> {
        occurrences(objects, from, to)
            .into_iter()
            .map(|o| (o.start, o.summary))
            .collect()
    }

    fn fields(summary: &str, start: &str, end: &str) -> EventFields {
        EventFields {
            summary: summary.into(),
            location: "Raum 204".into(),
            description: String::new(),
            start: berlin(start),
            end: berlin(end),
        }
    }

    fn target(instance: &str) -> Target<'static> {
        Target {
            uid: "mathe",
            instance: at(instance),
        }
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(DateTime<Utc>, String)> {
        list.iter().map(|(t, s)| (at(t), s.to_string())).collect()
    }

    /// Serie mit einer Ausnahme: 16.11. auf 12:00–13:00 verlegt und umbenannt
    fn with_exception() -> CalendarObject {
        let mut object = weekly("FREQ=WEEKLY;BYDAY=MO", "");
        let ex = fields("Sondertermin", "2026-11-16 12:00", "2026-11-16 13:00");
        edit(
            &mut object,
            target("2026-11-16 08:00"),
            Scope::This,
            &ex,
            &RuleChange::Keep,
            "",
            Berlin,
            now(),
        )
        .unwrap();
        object
    }

    #[test]
    fn this_occurrence_becomes_an_exception() {
        let mut object = weekly("FREQ=WEEKLY;BYDAY=MO", "");
        let f = fields("Mathe (verlegt)", "2026-11-17 10:00", "2026-11-17 11:30");
        edit(
            &mut object,
            target("2026-11-16 08:00"),
            Scope::This,
            &f,
            &RuleChange::Keep,
            "",
            Berlin,
            now(),
        )
        .unwrap();
        let ics = object.to_ics();
        assert!(ics.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261116T080000\r\n"));
        assert_eq!(
            ics.matches("X-FREMD:bleibt").count(),
            2,
            "Ausnahme übernimmt fremde Eigenschaften"
        );
        assert_eq!(ics.matches("TRIGGER:-PT10M").count(), 2, "und Alarme");
        assert_eq!(
            summaries(&[&object], "2026-11-01 00:00", "2026-11-30 00:00"),
            pairs(&[
                ("2026-11-02 08:00", "Mathe"),
                ("2026-11-09 08:00", "Mathe"),
                ("2026-11-17 10:00", "Mathe (verlegt)"),
                ("2026-11-23 08:00", "Mathe"),
            ])
        );

        // Zweites Bearbeiten ändert dieselbe Ausnahme
        let again = fields("Mathe (verlegt)", "2026-11-17 12:00", "2026-11-17 13:30");
        edit(
            &mut object,
            target("2026-11-16 08:00"),
            Scope::This,
            &again,
            &RuleChange::Keep,
            "",
            Berlin,
            now(),
        )
        .unwrap();
        assert_eq!(object.root.children_named("VEVENT").count(), 2);
        assert!(
            starts(&[&object], "2026-11-15 00:00", "2026-11-20 00:00")
                .contains(&at("2026-11-17 12:00"))
        );
    }

    #[test]
    fn deleting_this_occurrence_adds_exdate_and_drops_its_exception() {
        let mut object = with_exception();
        assert_eq!(
            delete(
                &mut object,
                target("2026-11-16 08:00"),
                Scope::This,
                Berlin,
                now()
            )
            .unwrap(),
            DeleteOutcome::Changed
        );
        assert!(
            object
                .to_ics()
                .contains("EXDATE;TZID=Europe/Berlin:20261116T080000\r\n")
        );
        assert_eq!(object.root.children_named("VEVENT").count(), 1);
        assert_eq!(
            starts(&[&object], "2026-11-01 00:00", "2026-11-24 00:00"),
            vec![
                at("2026-11-02 08:00"),
                at("2026-11-09 08:00"),
                at("2026-11-23 08:00")
            ]
        );
    }

    #[test]
    fn all_occurrences_move_with_exceptions_and_exdates() {
        let mut object = weekly(
            "FREQ=WEEKLY;BYDAY=MO",
            "EXDATE;TZID=Europe/Berlin:20261123T080000\r\n",
        );
        let ex = fields("Sondertermin", "2026-11-16 12:00", "2026-11-16 13:00");
        edit(
            &mut object,
            target("2026-11-16 08:00"),
            Scope::This,
            &ex,
            &RuleChange::Keep,
            "",
            Berlin,
            now(),
        )
        .unwrap();
        // Alle: vom Montag 9.11. 8:00 auf Dienstag 10.11. 9:00, neuer Titel
        let f = fields("Analysis", "2026-11-10 09:00", "2026-11-10 10:30");
        edit(
            &mut object,
            target("2026-11-09 08:00"),
            Scope::All,
            &f,
            &RuleChange::Keep,
            "",
            Berlin,
            now(),
        )
        .unwrap();
        let ics = object.to_ics();
        assert!(
            ics.contains("RRULE:FREQ=WEEKLY;BYDAY=TU\r\n"),
            "Wochentag der Regel wandert mit\n{ics}"
        );
        assert!(ics.contains("EXDATE;TZID=Europe/Berlin:20261124T090000\r\n"));
        assert!(ics.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261117T090000\r\n"));
        assert_eq!(
            summaries(&[&object], "2026-11-01 00:00", "2026-12-02 00:00"),
            pairs(&[
                ("2026-11-03 09:00", "Analysis"),
                ("2026-11-10 09:00", "Analysis"),
                ("2026-11-16 12:00", "Sondertermin"),
                ("2026-12-01 09:00", "Analysis"),
            ])
        );
    }

    #[test]
    fn all_from_a_moved_exception_uses_what_was_shown() {
        // Nur den Ort der verlegten Ausnahme ändern: Serie bleibt, wo sie ist
        let mut object = with_exception();
        let mut f = fields("Sondertermin", "2026-11-16 12:00", "2026-11-16 13:00");
        f.location = "Aula".into();
        edit(
            &mut object,
            target("2026-11-16 08:00"),
            Scope::All,
            &f,
            &RuleChange::Keep,
            "",
            Berlin,
            now(),
        )
        .unwrap();
        let occ = occurrences(&[&object], "2026-11-01 00:00", "2026-11-24 00:00");
        let got: Vec<(DateTime<Utc>, &str, &str)> = occ
            .iter()
            .map(|o| (o.start, o.summary.as_str(), o.location.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (at("2026-11-02 08:00"), "Mathe", "Aula"),
                (at("2026-11-09 08:00"), "Mathe", "Aula"),
                (at("2026-11-16 12:00"), "Sondertermin", "Aula"),
                (at("2026-11-23 08:00"), "Mathe", "Aula"),
            ]
        );

        // Die Ausnahme eine Stunde später ziehen, "alle": alle eine Stunde später
        let later = fields("Sondertermin", "2026-11-16 13:00", "2026-11-16 14:00");
        edit(
            &mut object,
            target("2026-11-16 08:00"),
            Scope::All,
            &later,
            &RuleChange::Keep,
            "",
            Berlin,
            now(),
        )
        .unwrap();
        assert_eq!(
            starts(&[&object], "2026-11-01 00:00", "2026-11-24 00:00"),
            vec![
                at("2026-11-02 09:00"),
                at("2026-11-09 09:00"),
                at("2026-11-16 13:00"),
                at("2026-11-23 09:00")
            ]
        );
        assert!(
            object
                .to_ics()
                .contains("DTEND;TZID=Europe/Berlin:20261102T103000"),
            "Dauer der Serie bleibt"
        );
    }

    #[test]
    fn following_splits_the_series_with_until_in_utc() {
        let mut object = weekly("FREQ=WEEKLY;BYDAY=MO", "");
        // Ausnahme nach dem Teilungspunkt zieht mit um
        let ex = fields(
            "Mathe (Raumwechsel)",
            "2026-11-30 08:00",
            "2026-11-30 09:30",
        );
        edit(
            &mut object,
            target("2026-11-30 08:00"),
            Scope::This,
            &ex,
            &RuleChange::Keep,
            "",
            Berlin,
            now(),
        )
        .unwrap();
        let f = fields("Mathe neu", "2026-11-16 10:00", "2026-11-16 11:30");
        let new = edit(
            &mut object,
            target("2026-11-16 08:00"),
            Scope::Following,
            &f,
            &RuleChange::Keep,
            "mathe-2",
            Berlin,
            now(),
        )
        .unwrap()
        .expect("neue Serie");
        // 16.11. 08:00 Berlin = 07:00 UTC, also UNTIL 06:59:59 UTC
        assert!(
            object
                .to_ics()
                .contains("RRULE:FREQ=WEEKLY;BYDAY=MO;UNTIL=20261116T065959Z\r\n")
        );
        assert_eq!(new.uid().as_deref(), Some("mathe-2"));
        assert!(
            new.to_ics()
                .contains("BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin")
        );
        assert_eq!(
            summaries(&[&object, &new], "2026-11-01 00:00", "2026-12-08 00:00"),
            pairs(&[
                ("2026-11-02 08:00", "Mathe"),
                ("2026-11-09 08:00", "Mathe"),
                ("2026-11-16 10:00", "Mathe neu"),
                ("2026-11-23 10:00", "Mathe neu"),
                ("2026-11-30 08:00", "Mathe (Raumwechsel)"),
                ("2026-12-07 10:00", "Mathe neu"),
            ])
        );
        assert_eq!(
            object.root.children_named("VEVENT").count(),
            1,
            "Ausnahme ist umgezogen"
        );
    }

    #[test]
    fn following_from_a_moved_exception_keeps_the_pattern() {
        let mut object = with_exception();
        let mut f = fields("Sondertermin", "2026-11-16 12:00", "2026-11-16 13:00");
        f.location = "Aula".into();
        let new = edit(
            &mut object,
            target("2026-11-16 08:00"),
            Scope::Following,
            &f,
            &RuleChange::Keep,
            "m2",
            Berlin,
            now(),
        )
        .unwrap()
        .unwrap();
        let occ = occurrences(&[&object, &new], "2026-11-01 00:00", "2026-12-01 00:00");
        let got: Vec<(DateTime<Utc>, &str, &str)> = occ
            .iter()
            .map(|o| (o.start, o.summary.as_str(), o.location.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (at("2026-11-02 08:00"), "Mathe", "Raum 204"),
                (at("2026-11-09 08:00"), "Mathe", "Raum 204"),
                (at("2026-11-16 12:00"), "Sondertermin", "Aula"),
                (at("2026-11-23 08:00"), "Mathe", "Aula"),
                (at("2026-11-30 08:00"), "Mathe", "Aula"),
            ]
        );
    }

    #[test]
    fn following_splits_count_between_both_series() {
        let mut object = weekly("FREQ=WEEKLY;BYDAY=MO;COUNT=5", "");
        let f = fields("Mathe", "2026-11-16 08:00", "2026-11-16 09:30");
        let new = edit(
            &mut object,
            target("2026-11-16 08:00"),
            Scope::Following,
            &f,
            &RuleChange::Keep,
            "m2",
            Berlin,
            now(),
        )
        .unwrap()
        .unwrap();
        assert!(object.to_ics().contains("COUNT=2"));
        assert!(new.to_ics().contains("COUNT=3"));
        assert_eq!(
            starts(&[&object, &new], "2026-11-01 00:00", "2027-01-01 00:00").len(),
            5
        );
    }

    #[test]
    fn following_from_the_first_occurrence_is_all() {
        let mut object = weekly("FREQ=WEEKLY;BYDAY=MO", "");
        let f = fields("Mathe", "2026-11-02 09:00", "2026-11-02 10:30");
        let new = edit(
            &mut object,
            target("2026-11-02 08:00"),
            Scope::Following,
            &f,
            &RuleChange::Keep,
            "x",
            Berlin,
            now(),
        )
        .unwrap();
        assert!(new.is_none());
        assert!(
            object
                .to_ics()
                .contains("DTSTART;TZID=Europe/Berlin:20261102T090000")
        );
    }

    #[test]
    fn split_across_dst_keeps_local_times() {
        // Serie ab 16.3. (Winterzeit), geteilt am 6.4. (Sommerzeit)
        let mut object = CalendarObject::parse(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:mathe\r\nDTSTART;TZID=Europe/Berlin:20260316T080000\r\n\
             DTEND;TZID=Europe/Berlin:20260316T090000\r\nRRULE:FREQ=WEEKLY\r\nSUMMARY:Mathe\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        let f = fields("Mathe", "2026-04-06 09:00", "2026-04-06 10:00");
        let new = edit(
            &mut object,
            target("2026-04-06 08:00"),
            Scope::Following,
            &f,
            &RuleChange::Keep,
            "m2",
            Berlin,
            now(),
        )
        .unwrap()
        .unwrap();
        // 6.4. 08:00 MESZ = 06:00 UTC
        assert!(object.to_ics().contains("UNTIL=20260406T055959Z"));
        assert_eq!(
            starts(&[&object, &new], "2026-03-15 00:00", "2026-04-14 00:00"),
            vec![
                at("2026-03-16 08:00"),
                at("2026-03-23 08:00"),
                at("2026-03-30 08:00"),
                at("2026-04-06 09:00"),
                at("2026-04-13 09:00")
            ]
        );
    }

    #[test]
    fn moving_all_across_dst_keeps_the_wall_clock_time() {
        // Serie aus der Winterzeit, im Sommer um eine Stunde verschoben
        let mut object = CalendarObject::parse(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:mathe\r\nDTSTART;TZID=Europe/Berlin:20260316T080000\r\n\
             DTEND;TZID=Europe/Berlin:20260316T090000\r\nRRULE:FREQ=WEEKLY;UNTIL=20260420T070000Z\r\nSUMMARY:Mathe\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        let f = fields("Mathe", "2026-04-06 09:00", "2026-04-06 10:00");
        edit(
            &mut object,
            target("2026-04-06 08:00"),
            Scope::All,
            &f,
            &RuleChange::Keep,
            "",
            Berlin,
            now(),
        )
        .unwrap();
        assert!(
            object
                .to_ics()
                .contains("DTSTART;TZID=Europe/Berlin:20260316T090000")
        );
        assert_eq!(
            starts(&[&object], "2026-03-15 00:00", "2026-05-01 00:00"),
            vec![
                at("2026-03-16 09:00"),
                at("2026-03-23 09:00"),
                at("2026-03-30 09:00"),
                at("2026-04-06 09:00"),
                at("2026-04-13 09:00"),
                at("2026-04-20 09:00")
            ]
        );
    }

    #[test]
    fn deleting_following_truncates_or_removes() {
        let mut object = with_exception();
        assert_eq!(
            delete(
                &mut object,
                target("2026-11-09 08:00"),
                Scope::Following,
                Berlin,
                now()
            )
            .unwrap(),
            DeleteOutcome::Changed
        );
        assert_eq!(
            starts(&[&object], "2026-11-01 00:00", "2027-01-01 00:00"),
            vec![at("2026-11-02 08:00")]
        );
        assert_eq!(
            object.root.children_named("VEVENT").count(),
            1,
            "spätere Ausnahme ist weg"
        );
        let mut first = weekly("FREQ=WEEKLY;BYDAY=MO", "");
        assert_eq!(
            delete(
                &mut first,
                target("2026-11-02 08:00"),
                Scope::Following,
                Berlin,
                now()
            )
            .unwrap(),
            DeleteOutcome::RemoveResource
        );
        assert_eq!(
            delete(
                &mut first,
                target("2026-11-09 08:00"),
                Scope::All,
                Berlin,
                now()
            )
            .unwrap(),
            DeleteOutcome::RemoveResource
        );
    }

    #[test]
    fn all_day_birthday_skips_one_year() {
        let mut object = CalendarObject::parse(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:geb\r\nDTSTART;VALUE=DATE:19900315\r\nDTEND;VALUE=DATE:19900316\r\n\
             RRULE:FREQ=YEARLY\r\nSUMMARY:Geburtstag\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        let instance =
            DateTimeValue::Date(NaiveDate::from_ymd_opt(2026, 3, 15).unwrap()).to_utc(Berlin);
        delete(
            &mut object,
            Target {
                uid: "geb",
                instance,
            },
            Scope::This,
            Berlin,
            now(),
        )
        .unwrap();
        assert!(object.to_ics().contains("EXDATE;VALUE=DATE:20260315\r\n"));
        assert!(starts(&[&object], "2026-01-01 00:00", "2027-01-01 00:00").is_empty());
        assert_eq!(
            starts(&[&object], "2027-01-01 00:00", "2028-01-01 00:00").len(),
            1
        );
    }

    #[test]
    fn floating_series_gets_floating_until() {
        let mut object = CalendarObject::parse(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:f\r\nDTSTART:20261102T080000\r\nDTEND:20261102T090000\r\n\
             RRULE:FREQ=DAILY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        delete(
            &mut object,
            Target {
                uid: "f",
                instance: at("2026-11-05 08:00"),
            },
            Scope::Following,
            Berlin,
            now(),
        )
        .unwrap();
        assert!(
            object
                .to_ics()
                .contains("RRULE:FREQ=DAILY;UNTIL=20261105T075959\r\n")
        );
        assert_eq!(
            starts(&[&object], "2026-11-01 00:00", "2026-12-01 00:00").len(),
            3
        );
    }

    #[test]
    fn removing_the_rule_leaves_a_single_event() {
        let mut object = with_exception();
        let f = fields("Mathe einmalig", "2026-11-09 08:00", "2026-11-09 09:30");
        edit(
            &mut object,
            target("2026-11-09 08:00"),
            Scope::All,
            &f,
            &RuleChange::Remove,
            "",
            Berlin,
            now(),
        )
        .unwrap();
        let ics = object.to_ics();
        assert!(!ics.contains("RRULE") && !ics.contains("RECURRENCE-ID"));
        assert_eq!(
            starts(&[&object], "2026-11-01 00:00", "2027-01-01 00:00"),
            vec![at("2026-11-09 08:00")]
        );
    }

    #[test]
    fn new_rule_keeps_only_matching_exceptions() {
        // Ausnahme am 16.11., EXDATE am 23.11.
        let mut object = with_exception();
        delete(
            &mut object,
            target("2026-11-23 08:00"),
            Scope::This,
            Berlin,
            now(),
        )
        .unwrap();
        // Täglich statt wöchentlich: beide Montage gibt es weiterhin
        let f = fields("Mathe", "2026-11-02 08:00", "2026-11-02 09:30");
        let daily = RuleChange::Set("FREQ=DAILY;COUNT=30".into());
        edit(
            &mut object,
            target("2026-11-02 08:00"),
            Scope::All,
            &f,
            &daily,
            "",
            Berlin,
            now(),
        )
        .unwrap();
        assert_eq!(object.root.children_named("VEVENT").count(), 2);
        assert!(object.to_ics().contains("EXDATE"));
        // Alle zwei Wochen: der 16. bleibt (Ausnahme), den 23. gibt es nicht mehr
        let biweekly = RuleChange::Set("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO".into());
        edit(
            &mut object,
            target("2026-11-02 08:00"),
            Scope::All,
            &f,
            &biweekly,
            "",
            Berlin,
            now(),
        )
        .unwrap();
        let ics = object.to_ics();
        assert_eq!(object.root.children_named("VEVENT").count(), 2, "{ics}");
        assert!(!ics.contains("EXDATE"));
        // Alle drei Wochen: auch die Ausnahme vom 16. gehört zu keinem Termin mehr
        let triweekly = RuleChange::Set("FREQ=WEEKLY;INTERVAL=3;BYDAY=MO".into());
        edit(
            &mut object,
            target("2026-11-02 08:00"),
            Scope::All,
            &f,
            &triweekly,
            "",
            Berlin,
            now(),
        )
        .unwrap();
        assert_eq!(object.root.children_named("VEVENT").count(), 1);
        assert_eq!(
            starts(&[&object], "2026-11-01 00:00", "2026-12-01 00:00"),
            vec![at("2026-11-02 08:00"), at("2026-11-23 08:00")]
        );
    }

    #[test]
    fn rule_can_be_set_on_a_single_event() {
        let mut single = CalendarObject::parse(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:s\r\nDTSTART:20261102T070000Z\r\nDTEND:20261102T080000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        set_rule(&mut single, "s", Some("FREQ=DAILY;COUNT=2")).unwrap();
        assert_eq!(
            starts(&[&single], "2026-11-01 00:00", "2026-12-01 00:00").len(),
            2
        );
        set_rule(&mut single, "s", None).unwrap();
        assert_eq!(
            starts(&[&single], "2026-11-01 00:00", "2026-12-01 00:00").len(),
            1
        );
    }
}
