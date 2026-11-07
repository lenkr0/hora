//! Journaleinträge (VJOURNAL, RFC 5545 Abschnitt 3.6.3): freie Notizen an
//! einem Kalendertag, wie Calinos Journal. Titel (SUMMARY) ist optional,
//! der Text (DESCRIPTION) ist Markdown, Tags stehen in CATEGORIES.
//!
//! VJOURNAL darf mehrere DESCRIPTION-Eigenschaften haben; gelesen werden sie
//! mit Leerzeile verbunden. Geschrieben wird nur, was sich geändert hat, so
//! bleiben Einträge anderer Programme unverändert, bis man sie bearbeitet.

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;

use crate::edit::{EditError, PRODID, ensure_timezones, set_text, time_property, touch, utc_stamp};
use crate::ical::{Component, Property, escape_text, split_values, unescape_text};
use crate::model::CalendarObject;
use crate::time::{DateTimeValue, local_to_utc};

/// Ein Journaleintrag, aufbereitet für Liste und Editor
#[derive(Debug, Clone, PartialEq)]
pub struct JournalInfo {
    pub uid: String,
    pub summary: String,
    pub description: String,
    /// DTSTART; fehlt er, gilt der Tag der Erstellung
    pub date: Option<DateTimeValue>,
    pub categories: Vec<String>,
    pub cancelled: bool,
    pub rrule: Option<String>,
    pub recurrence_id: Option<DateTimeValue>,
    pub created: Option<DateTime<Utc>>,
    pub last_modified: Option<DateTime<Utc>>,
}

fn value_of(component: &Component, name: &str, root: &Component) -> Option<DateTimeValue> {
    component
        .property(name)
        .and_then(|p| DateTimeValue::from_property(p, Some(root)).ok())
}

fn descriptions(component: &Component) -> String {
    component
        .properties_named("DESCRIPTION")
        .map(Property::text_value)
        .filter(|t| !t.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn categories(component: &Component) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for property in component.properties_named("CATEGORIES") {
        for raw in split_values(&property.value) {
            let tag = unescape_text(raw).trim().to_string();
            if !tag.is_empty() && !out.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
                out.push(tag);
            }
        }
    }
    out
}

impl JournalInfo {
    pub fn from_component(journal: &Component, root: &Component) -> Option<JournalInfo> {
        let uid = journal.property("UID")?.value.trim().to_string();
        let stamp = |name: &str| value_of(journal, name, root).map(|v| v.to_utc(Tz::UTC));
        Some(JournalInfo {
            summary: journal.text_of("SUMMARY"),
            description: descriptions(journal),
            date: value_of(journal, "DTSTART", root),
            categories: categories(journal),
            cancelled: journal
                .text_of("STATUS")
                .trim()
                .eq_ignore_ascii_case("CANCELLED"),
            rrule: journal
                .property("RRULE")
                .map(|p| p.value.trim().to_string()),
            recurrence_id: value_of(journal, "RECURRENCE-ID", root),
            created: stamp("CREATED"),
            last_modified: stamp("LAST-MODIFIED").or_else(|| stamp("DTSTAMP")),
            uid,
        })
    }

    /// Kalendertag des Eintrags in `home` (ohne DTSTART: Tag der Erstellung)
    pub fn day(&self, home: Tz) -> Option<NaiveDate> {
        match self.date {
            Some(DateTimeValue::Date(d)) => Some(d),
            Some(DateTimeValue::Floating(n)) => Some(n.date()),
            Some(v) => Some(v.to_utc(home).with_timezone(&home).date_naive()),
            None => self
                .created
                .or(self.last_modified)
                .map(|c| c.with_timezone(&home).date_naive()),
        }
    }
}

impl CalendarObject {
    /// Alle VJOURNALs dieses Objekts (Einträge und Ausnahmen einer Serie)
    pub fn journals(&self) -> Vec<JournalInfo> {
        self.root
            .children_named("VJOURNAL")
            .filter_map(|c| JournalInfo::from_component(c, &self.root))
            .collect()
    }
}

/// Felder des Editors
#[derive(Debug, Clone, PartialEq)]
pub struct JournalFields {
    pub summary: String,
    pub description: String,
    pub day: NaiveDate,
    pub categories: Vec<String>,
}

/// Tags bereinigen: getrimmt, ohne leere und ohne Dubletten (Groß-/Kleinschreibung egal)
pub fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let tag = tag.trim().trim_start_matches('#').trim();
        if !tag.is_empty() && !out.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
            out.push(tag.to_string());
        }
    }
    out
}

fn write_fields(journal: &mut Component, fields: &JournalFields, home: Tz, root: &Component) {
    set_text(journal, "SUMMARY", fields.summary.trim());
    let text = fields.description.trim_end();
    if descriptions(journal) != text {
        journal.remove_properties("DESCRIPTION");
        if !text.trim().is_empty() {
            journal
                .properties
                .push(Property::new("DESCRIPTION", escape_text(text)));
        }
    }
    let tags = clean_tags(&fields.categories);
    if categories(journal) != tags {
        let at = journal
            .properties
            .iter()
            .position(|p| p.is("CATEGORIES"))
            .unwrap_or(journal.properties.len());
        journal.remove_properties("CATEGORIES");
        if !tags.is_empty() {
            let value = tags
                .iter()
                .map(|t| escape_text(t))
                .collect::<Vec<_>>()
                .join(",");
            let at = at.min(journal.properties.len());
            journal
                .properties
                .insert(at, Property::new("CATEGORIES", value));
        }
    }
    // Datum: unverändert lassen, wenn der Tag gleich bleibt (Uhrzeit und Zone
    // anderer Programme bleiben); sonst gleiche Uhrzeit am neuen Tag
    let current = value_of(journal, "DTSTART", root);
    let info_day = current.map(|v| match v {
        DateTimeValue::Date(d) => d,
        DateTimeValue::Floating(n) => n.date(),
        other => other
            .to_utc(home)
            .with_timezone(&other.zone(home))
            .date_naive(),
    });
    if info_day != Some(fields.day) {
        let value = match current {
            None | Some(DateTimeValue::Date(_)) => DateTimeValue::Date(fields.day),
            Some(DateTimeValue::Floating(n)) => {
                DateTimeValue::Floating(fields.day.and_time(n.time()))
            }
            Some(v) => {
                let zone = v.zone(home);
                let local = v.to_utc(home).with_timezone(&zone).naive_local();
                v.moved_to(local_to_utc(fields.day.and_time(local.time()), zone), home)
            }
        };
        let property = time_property("DTSTART", &value, journal.property("DTSTART"));
        journal.set_property(property);
    }
}

/// Neues Kalenderobjekt mit einem Journaleintrag (ganztägig am Tag `fields.day`)
pub fn new_journal(
    uid: &str,
    fields: &JournalFields,
    home: Tz,
    now: DateTime<Utc>,
) -> CalendarObject {
    let mut root = Component::new("VCALENDAR");
    root.properties.push(Property::new("VERSION", "2.0"));
    root.properties.push(Property::new("PRODID", PRODID));
    root.properties.push(Property::new("CALSCALE", "GREGORIAN"));
    let stamp = utc_stamp(now);
    let mut journal = Component::new("VJOURNAL");
    for (name, value) in [
        ("UID", uid),
        ("DTSTAMP", &stamp),
        ("CREATED", &stamp),
        ("LAST-MODIFIED", &stamp),
    ] {
        journal.properties.push(Property::new(name, value));
    }
    journal.properties.push(Property::new("SEQUENCE", "0"));
    // Wie Calino: kein STATUS; RFC 5545 nimmt dann einen fertigen Eintrag an
    journal.properties.push(time_property(
        "DTSTART",
        &DateTimeValue::Date(fields.day),
        None,
    ));
    let empty = Component::new("VCALENDAR");
    write_fields(&mut journal, fields, home, &empty);
    root.components.push(journal);
    CalendarObject { root }
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
            c.is("VJOURNAL")
                && c.property("RECURRENCE-ID").is_none()
                && c.property("UID").is_some_and(|p| p.value.trim() == uid)
        })
        .ok_or_else(|| EditError::NotFound(uid.to_string()))
}

/// Felder eines bestehenden Eintrags setzen; fremde Eigenschaften bleiben.
/// Ohne Änderung bleibt das Objekt unverändert (kein neuer Zeitstempel).
pub fn update_journal(
    object: &mut CalendarObject,
    uid: &str,
    fields: &JournalFields,
    home: Tz,
    now: DateTime<Utc>,
) -> Result<bool, EditError> {
    let root = object.root.clone();
    let journal = master_mut(object, uid)?;
    let before = journal.clone();
    write_fields(journal, fields, home, &root);
    if *journal == before {
        return Ok(false);
    }
    touch(journal, now);
    ensure_timezones(object);
    Ok(true)
}

/// Wörter im Text (für die Fußzeile des Editors)
pub fn word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::Europe::Berlin;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 6, 18, 30, 0).unwrap()
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn fields(summary: &str, description: &str, tags: &[&str]) -> JournalFields {
        JournalFields {
            summary: summary.into(),
            description: description.into(),
            day: day(2026, 11, 6),
            categories: tags.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn new_entry_is_an_all_day_vjournal_with_escaped_text_and_tags() {
        let object = new_journal(
            "j1",
            &fields(
                "Erster Schnee",
                "Heute früh lag Schnee.\n\n**Kalt**, aber schön; sehr.",
                &["Winter", "draußen", "winter"],
            ),
            Berlin,
            now(),
        );
        let ics = object.to_ics();
        assert!(ics.contains("BEGIN:VJOURNAL\r\n"), "{ics}");
        assert!(ics.contains("DTSTART;VALUE=DATE:20261106\r\n"), "{ics}");
        assert!(ics.contains("SUMMARY:Erster Schnee\r\n"), "{ics}");
        assert!(
            ics.contains("DESCRIPTION:Heute früh lag Schnee.\\n\\n**Kalt**\\, aber schön\\; sehr."),
            "{ics}"
        );
        assert!(ics.contains("CATEGORIES:Winter,draußen\r\n"), "{ics}");
        assert!(!ics.contains("STATUS"), "{ics}");
        let parsed = CalendarObject::parse(&ics).unwrap();
        assert_eq!(parsed.kind(), Some(crate::model::Kind::Journal));
        let info = &parsed.journals()[0];
        assert_eq!(info.summary, "Erster Schnee");
        assert_eq!(
            info.description,
            "Heute früh lag Schnee.\n\n**Kalt**, aber schön; sehr."
        );
        assert_eq!(info.categories, ["Winter", "draußen"]);
        assert_eq!(info.day(Berlin), Some(day(2026, 11, 6)));
    }

    #[test]
    fn title_is_optional_and_empty_text_removes_the_description() {
        let mut object = new_journal("j2", &fields("", "Nur Text", &[]), Berlin, now());
        assert!(!object.to_ics().contains("SUMMARY"));
        let changed =
            update_journal(&mut object, "j2", &fields("Titel", "", &[]), Berlin, now()).unwrap();
        assert!(changed);
        let ics = object.to_ics();
        assert!(ics.contains("SUMMARY:Titel\r\n"));
        assert!(!ics.contains("DESCRIPTION"), "{ics}");
        assert!(ics.contains("SEQUENCE:1\r\n"));
    }

    const FOREIGN: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Fremd//DE\r\n\
        BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\nBEGIN:STANDARD\r\nDTSTART:19701025T030000\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n\
        BEGIN:VJOURNAL\r\nUID:fremd\r\nDTSTAMP:20261101T080000Z\r\nDTSTART;TZID=Europe/Berlin:20261103T213000\r\n\
        SUMMARY:Abend\r\nDESCRIPTION:Teil eins\r\nDESCRIPTION:Teil zwei\r\nCATEGORIES:a,b\r\nCATEGORIES:c\r\n\
        X-FREMD:bleibt\r\nATTACH:https://example.com/bild.jpg\r\nEND:VJOURNAL\r\nEND:VCALENDAR\r\n";

    #[test]
    fn foreign_entries_are_read_and_kept_until_something_changes() {
        let mut object = CalendarObject::parse(FOREIGN).unwrap();
        let info = object.journals().remove(0);
        assert_eq!(info.description, "Teil eins\n\nTeil zwei");
        assert_eq!(info.categories, ["a", "b", "c"]);
        assert_eq!(info.day(Berlin), Some(day(2026, 11, 3)));
        // Gleiche Felder: nichts ändert sich
        let same = JournalFields {
            summary: info.summary.clone(),
            description: info.description.clone(),
            day: day(2026, 11, 3),
            categories: info.categories.clone(),
        };
        let before = object.to_ics();
        assert!(!update_journal(&mut object, "fremd", &same, Berlin, now()).unwrap());
        assert_eq!(object.to_ics(), before);
        // Nur der Titel: Beschreibungen, Tags, Uhrzeit und Fremdes bleiben
        let renamed = JournalFields {
            summary: "Später Abend".into(),
            ..same.clone()
        };
        assert!(update_journal(&mut object, "fremd", &renamed, Berlin, now()).unwrap());
        let ics = object.to_ics();
        assert!(
            ics.contains("DESCRIPTION:Teil eins\r\nDESCRIPTION:Teil zwei\r\n"),
            "{ics}"
        );
        assert!(ics.contains("CATEGORIES:a,b\r\nCATEGORIES:c\r\n"), "{ics}");
        assert!(
            ics.contains("DTSTART;TZID=Europe/Berlin:20261103T213000\r\n"),
            "{ics}"
        );
        assert!(ics.contains("X-FREMD:bleibt\r\nATTACH:https://example.com/bild.jpg\r\n"));
        // Anderer Tag: gleiche Uhrzeit in derselben Zone; Text und Tags zusammengefasst
        let moved = JournalFields {
            day: day(2026, 11, 4),
            description: "Neu".into(),
            categories: vec!["a".into()],
            ..renamed
        };
        update_journal(&mut object, "fremd", &moved, Berlin, now()).unwrap();
        let ics = object.to_ics();
        assert!(
            ics.contains("DTSTART;TZID=Europe/Berlin:20261104T213000\r\n"),
            "{ics}"
        );
        assert_eq!(ics.matches("DESCRIPTION").count(), 1);
        assert!(ics.contains("CATEGORIES:a\r\n"), "{ics}");
        assert_eq!(ics.matches("CATEGORIES").count(), 1);
    }

    #[test]
    fn entry_without_start_belongs_to_its_creation_day() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VJOURNAL\r\nUID:x\r\nCREATED:20261105T233000Z\r\nSUMMARY:Spät\r\nEND:VJOURNAL\r\nEND:VCALENDAR\r\n";
        let info = CalendarObject::parse(ics).unwrap().journals().remove(0);
        assert_eq!(info.date, None);
        // 23:30 UTC ist in Berlin schon der 6.
        assert_eq!(info.day(Berlin), Some(day(2026, 11, 6)));
        let mut object = CalendarObject::parse(ics).unwrap();
        let f = JournalFields {
            summary: "Spät".into(),
            description: String::new(),
            day: day(2026, 11, 6),
            categories: Vec::new(),
        };
        update_journal(&mut object, "x", &f, Berlin, now()).unwrap();
        assert!(object.to_ics().contains("DTSTART;VALUE=DATE:20261106\r\n"));
    }

    #[test]
    fn tags_are_trimmed_without_hash_and_deduplicated() {
        let tags: Vec<String> = [" #Urlaub", "urlaub", "", "Reise ", "a,b"]
            .iter()
            .map(|t| t.to_string())
            .collect();
        assert_eq!(clean_tags(&tags), ["Urlaub", "Reise", "a,b"]);
        let object = new_journal(
            "t",
            &JournalFields {
                categories: tags,
                ..fields("", "x", &[])
            },
            Berlin,
            now(),
        );
        let ics = object.to_ics();
        assert!(ics.contains("CATEGORIES:Urlaub,Reise,a\\,b\r\n"), "{ics}");
        assert_eq!(object.journals()[0].categories, ["Urlaub", "Reise", "a,b"]);
    }

    #[test]
    fn words_are_counted_without_markdown_markers() {
        assert_eq!(word_count("# Titel\n\n- eins\n- **zwei** drei"), 4);
        assert_eq!(word_count("   "), 0);
    }
}
