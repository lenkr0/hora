//! Ganze Kalender als eine .ics-Datei: Einträge zusammenführen (Export) und
//! eine Datei wieder in einzelne Einträge aufteilen (Import). Ein Eintrag
//! ist, wie auf einem CalDAV-Server, ein VCALENDAR mit allen Komponenten
//! einer UID (Serie samt Ausnahmen) und den Zeitzonen, die sie brauchen.

use std::collections::{BTreeMap, BTreeSet};

use crate::edit::PRODID;
use crate::ical::{self, Component, ParseError, Property};

/// Komponenten mit eigenem Inhalt (keine Zeitzonen)
const ITEMS: [&str; 3] = ["VEVENT", "VTODO", "VJOURNAL"];

/// Ein Eintrag aus einer .ics-Datei
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub uid: String,
    pub ics: String,
}

fn calendar() -> Component {
    let mut root = Component::new("VCALENDAR");
    root.properties.push(Property::new("VERSION", "2.0"));
    root.properties.push(Property::new("PRODID", PRODID));
    root
}

fn tzid(component: &Component) -> Option<String> {
    component
        .property("TZID")
        .map(|p| p.value.trim().to_string())
}

/// Alle Einträge (je ein VCALENDAR) zu einer Datei mit dem Kalendernamen;
/// jede Zeitzone steht nur einmal darin. Unlesbare Einträge fallen weg und
/// werden gezählt.
pub fn merge(resources: &[&str], name: &str) -> (String, usize) {
    let mut root = calendar();
    root.properties.push(Property::text("X-WR-CALNAME", name));
    let mut zones: BTreeMap<String, Component> = BTreeMap::new();
    let mut items = Vec::new();
    let mut skipped = 0;
    for resource in resources {
        let Ok(parsed) = ical::parse_calendar(resource) else {
            skipped += 1;
            continue;
        };
        for child in parsed.components {
            if child.is("VTIMEZONE") {
                if let Some(id) = tzid(&child) {
                    zones.entry(id).or_insert(child);
                }
            } else {
                items.push(child);
            }
        }
    }
    root.components.extend(zones.into_values());
    root.components.extend(items);
    (ical::serialize(&root), skipped)
}

/// Zeitzonen, auf die Eigenschaften der Komponente (und ihrer Kinder) zeigen
fn referenced_zones(component: &Component, out: &mut BTreeSet<String>) {
    for property in &component.properties {
        if let Some(id) = property.param("TZID") {
            out.insert(id.trim().to_string());
        }
    }
    for child in &component.components {
        referenced_zones(child, out);
    }
}

/// Eine .ics-Datei (auch mit mehreren VCALENDAR) in Einträge je UID.
/// Komponenten ohne UID bekommen eine aus `fallback_uid`.
pub fn split(
    input: &str,
    mut fallback_uid: impl FnMut() -> String,
) -> Result<Vec<Entry>, ParseError> {
    let roots = ical::parse(input)?;
    if !roots.iter().any(|c| c.is("VCALENDAR")) {
        return Err(ParseError::NoCalendar);
    }
    let mut zones: BTreeMap<String, Component> = BTreeMap::new();
    // UID -> Komponenten, in der Reihenfolge des ersten Auftretens
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, Vec<Component>> = BTreeMap::new();
    for root in roots.into_iter().filter(|c| c.is("VCALENDAR")) {
        for child in root.components {
            if child.is("VTIMEZONE") {
                if let Some(id) = tzid(&child) {
                    zones.entry(id).or_insert(child);
                }
            } else if ITEMS.iter().any(|name| child.is(name)) {
                let mut child = child;
                let uid = match child.property("UID").map(|p| p.value.trim().to_string()) {
                    Some(uid) if !uid.is_empty() => uid,
                    _ => {
                        let uid = fallback_uid();
                        child.set_property(Property::new("UID", uid.clone()));
                        uid
                    }
                };
                if !groups.contains_key(&uid) {
                    order.push(uid.clone());
                }
                groups.entry(uid).or_default().push(child);
            }
        }
    }
    Ok(order
        .into_iter()
        .map(|uid| {
            let components = groups.remove(&uid).unwrap_or_default();
            let mut needed = BTreeSet::new();
            for component in &components {
                referenced_zones(component, &mut needed);
            }
            let mut root = calendar();
            root.components
                .extend(needed.iter().filter_map(|id| zones.get(id).cloned()));
            root.components.extend(components);
            Entry {
                uid,
                ics: ical::serialize(&root),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZONE: &str = "BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\nBEGIN:STANDARD\r\nDTSTART:19701025T030000\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n";

    fn event(uid: &str, extra: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\n{ZONE}BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART;TZID=Europe/Berlin:20261116T090000\r\nDTEND;TZID=Europe/Berlin:20261116T100000\r\nSUMMARY:Termin {uid}\r\n{extra}END:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    }

    #[test]
    fn merged_file_splits_back_into_the_same_entries() {
        let series = event("serie", "RRULE:FREQ=WEEKLY\r\n").replace(
            "END:VCALENDAR",
            "BEGIN:VEVENT\r\nUID:serie\r\nRECURRENCE-ID;TZID=Europe/Berlin:20261123T090000\r\nDTSTART;TZID=Europe/Berlin:20261123T110000\r\nDTEND;TZID=Europe/Berlin:20261123T120000\r\nSUMMARY:Verschoben\r\nEND:VEVENT\r\nEND:VCALENDAR",
        );
        let task = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:aufgabe\r\nSUMMARY:Einkaufen\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let one = event("eins", "");
        let (file, skipped) = merge(&[&one, &series, task, "kaputt"], "Privat");
        assert_eq!(skipped, 1);
        // Die Zeitzone steht nur einmal in der Datei, der Name ist dabei
        assert_eq!(file.matches("BEGIN:VTIMEZONE").count(), 1);
        assert!(file.contains("X-WR-CALNAME:Privat"));

        let entries = split(&file, || unreachable!()).unwrap();
        let uids: Vec<&str> = entries.iter().map(|e| e.uid.as_str()).collect();
        assert_eq!(uids, ["eins", "serie", "aufgabe"]);
        // Serie mit Ausnahme bleibt ein Eintrag, mit ihrer Zeitzone
        let serie = ical::parse_calendar(&entries[1].ics).unwrap();
        assert_eq!(serie.children_named("VEVENT").count(), 2);
        assert_eq!(serie.children_named("VTIMEZONE").count(), 1);
        // Die Aufgabe braucht keine Zeitzone
        let aufgabe = ical::parse_calendar(&entries[2].ics).unwrap();
        assert_eq!(aufgabe.children_named("VTIMEZONE").count(), 0);
        assert_eq!(aufgabe.children_named("VTODO").count(), 1);
    }

    #[test]
    fn several_calendars_and_missing_uids() {
        let input = format!(
            "{}BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VJOURNAL\r\nSUMMARY:Ohne UID\r\nEND:VJOURNAL\r\nEND:VCALENDAR\r\n",
            event("a", "")
        );
        let mut n = 0;
        let entries = split(&input, || {
            n += 1;
            format!("neu-{n}")
        })
        .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].uid, "neu-1");
        assert!(entries[1].ics.contains("UID:neu-1"));
        let no_calendar = "BEGIN:VEVENT
UID:x
END:VEVENT
";
        assert_eq!(split(no_calendar, String::new), Err(ParseError::NoCalendar));
    }
}
