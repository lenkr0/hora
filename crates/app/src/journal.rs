//! Journal wie Calinos JournalView: Einträge (VJOURNAL) aller sichtbaren
//! Kalender, ohne Konto Beispiel-Einträge im Speicher. Die Liste steht nach
//! Tag absteigend, der Editor speichert automatisch; die Leseansicht zeigt
//! den Text als Markdown.
//!
//! Calino: Copyright (c) 2025 Ivan Malinovski, MIT-Lizenz.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use chrono::{Datelike, Days, NaiveDate, Utc};
use hora_core::journal::{self, JournalFields, JournalInfo};
use hora_core::model::{CalendarObject, Kind};

use crate::forms::{MONTHS_SHORT, WEEKDAYS_SHORT};
use crate::{DEMO_COLOR, State, calendar_color, editing};

/// Kalender-ID der Beispiel-Einträge (wie bei den Beispielterminen)
pub const DEMO: i64 = -1;

/// Ein Eintrag der Liste
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub calendar_id: i64,
    pub href: String,
    pub uid: String,
    pub info: JournalInfo,
    pub day: NaiveDate,
    pub color: u32,
    pub calendar_name: String,
    pub read_only: bool,
    /// Neu und noch nie gespeichert (leere Entwürfe werden verworfen)
    pub draft: bool,
}

impl Entry {
    pub fn key(&self) -> String {
        key_of(self.calendar_id, &self.href)
    }
}

pub fn key_of(calendar_id: i64, href: &str) -> String {
    format!("j|{calendar_id}|{href}")
}

fn hash_of(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn shown(info: &JournalInfo) -> bool {
    info.recurrence_id.is_none() && !info.cancelled
}

/// Alle Einträge, neueste zuerst (Tag, dann Erstellung)
pub fn collect(state: &mut State) -> Vec<Entry> {
    let home = state.home;
    let today = state.now().date();
    let calendars: HashMap<i64, (u32, String, bool)> = state
        .calendars
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id, (calendar_color(c, i), c.name.clone(), c.read_only)))
        .collect();
    let mut out = Vec::new();
    for resource in state.store.journals().unwrap_or_default() {
        let (color, calendar_name, read_only) = calendars
            .get(&resource.calendar_id)
            .cloned()
            .unwrap_or((crate::CALENDAR_COLORS[0], String::new(), false));
        let hash = hash_of(&resource.ics);
        let cached = state
            .parsed_journals
            .entry((resource.calendar_id, resource.href.clone()))
            .or_insert((0, Vec::new()));
        if cached.0 != hash {
            *cached = (
                hash,
                CalendarObject::parse(&resource.ics)
                    .map(|o| o.journals())
                    .unwrap_or_default(),
            );
        }
        // Eine Ressource = ein Eintrag (Ausnahmen einer Serie bleiben unsichtbar)
        if let Some(info) = cached.1.iter().find(|i| shown(i)) {
            out.push(Entry {
                calendar_id: resource.calendar_id,
                href: resource.href.clone(),
                uid: info.uid.clone(),
                day: info.day(home).unwrap_or(today),
                info: info.clone(),
                color,
                calendar_name: calendar_name.clone(),
                read_only,
                draft: false,
            });
        }
    }
    if state.demo_visible {
        for (href, object) in &state.demo_journals {
            if let Some(info) = object.journals().into_iter().find(shown) {
                out.push(Entry {
                    calendar_id: DEMO,
                    href: href.clone(),
                    uid: info.uid.clone(),
                    day: info.day(home).unwrap_or(today),
                    info,
                    color: DEMO_COLOR,
                    calendar_name: "Beispieltermine".into(),
                    read_only: false,
                    draft: false,
                });
            }
        }
    }
    if let Some(draft) = &state.journal_draft
        && !out.iter().any(|e| e.key() == draft.key())
    {
        out.push(draft.clone());
    }
    out.sort_by(|a, b| {
        b.day
            .cmp(&a.day)
            .then(b.draft.cmp(&a.draft))
            .then(b.info.created.cmp(&a.info.created))
            .then(a.key().cmp(&b.key()))
    });
    out
}

/// Einträge der Ansicht: ein Monat oder alle
pub fn filter(entries: Vec<Entry>, month: Option<NaiveDate>) -> Vec<Entry> {
    match month {
        None => entries,
        Some(m) => entries
            .into_iter()
            .filter(|e| e.day.year() == m.year() && e.day.month() == m.month())
            .collect(),
    }
}

/// Kalender, in die Einträge gehen können: (ID, Name, Farbe)
pub fn calendars(state: &State) -> Vec<(i64, String, u32)> {
    let mut out: Vec<(i64, String, u32)> = state
        .calendars
        .iter()
        .enumerate()
        .filter(|(_, c)| c.visible && !c.read_only && c.supports(Kind::Journal))
        .map(|(i, c)| (c.id, c.name.clone(), calendar_color(c, i)))
        .collect();
    if state.demo_visible {
        out.push((DEMO, "Beispieltermine".into(), DEMO_COLOR));
    }
    out
}

/// Kalender für neue Einträge: zuletzt benutzter, sonst der erste
pub fn default_calendar(state: &State) -> Option<i64> {
    let ids: Vec<i64> = calendars(state).into_iter().map(|c| c.0).collect();
    state
        .store
        .setting("default_journal_calendar")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|id| ids.contains(id))
        .or_else(|| ids.first().copied())
}

fn href_in(state: &State, calendar_id: i64, uid: &str) -> Result<String, String> {
    if calendar_id == DEMO {
        Ok(format!("demo/{uid}.ics"))
    } else {
        editing::new_href(state, calendar_id, uid)
    }
}

/// Neuer, leerer Entwurf am Tag `day`
pub fn new_draft(state: &State, day: NaiveDate) -> Result<Entry, String> {
    let calendar_id = default_calendar(state).ok_or("Kein Kalender nimmt Journaleinträge an.")?;
    let uid = editing::new_uid();
    let href = href_in(state, calendar_id, &uid)?;
    let (name, color) = calendars(state)
        .into_iter()
        .find(|c| c.0 == calendar_id)
        .map(|c| (c.1, c.2))
        .unwrap_or_default();
    Ok(Entry {
        calendar_id,
        href,
        info: JournalInfo {
            uid: uid.clone(),
            summary: String::new(),
            description: String::new(),
            date: None,
            categories: Vec::new(),
            cancelled: false,
            rrule: None,
            recurrence_id: None,
            created: Some(Utc::now()),
            last_modified: None,
        },
        uid,
        day,
        color,
        calendar_name: name,
        read_only: false,
        draft: true,
    })
}

fn load(state: &State, entry: &Entry) -> Result<CalendarObject, String> {
    if entry.calendar_id == DEMO {
        return state
            .demo_journals
            .iter()
            .find(|(h, _)| *h == entry.href)
            .map(|(_, o)| o.clone())
            .ok_or_else(|| "Der Eintrag ist nicht mehr vorhanden.".to_string());
    }
    let resource = state
        .store
        .resource(entry.calendar_id, &entry.href)
        .map_err(|e| e.to_string())?
        .ok_or("Der Eintrag ist nicht mehr vorhanden.")?;
    CalendarObject::parse(&resource.ics).map_err(|e| e.to_string())
}

fn put(
    state: &mut State,
    calendar_id: i64,
    href: &str,
    object: CalendarObject,
) -> Result<(), String> {
    if calendar_id == DEMO {
        match state.demo_journals.iter_mut().find(|(h, _)| h == href) {
            Some(slot) => slot.1 = object,
            None => state.demo_journals.push((href.to_string(), object)),
        }
        Ok(())
    } else {
        editing::store_object(state, calendar_id, href, &object)
    }
}

/// Ergebnis von [`save`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saved {
    /// Nichts geändert (oder leerer Entwurf)
    Unchanged,
    Created,
    Updated,
}

/// Eintrag mit den Feldern des Editors speichern
pub fn save(state: &mut State, entry: &Entry, fields: &JournalFields) -> Result<Saved, String> {
    if entry.read_only {
        return Err("Dieser Kalender ist schreibgeschützt.".into());
    }
    let home = state.home;
    let now = Utc::now();
    if entry.draft {
        if fields.summary.trim().is_empty()
            && fields.description.trim().is_empty()
            && fields.categories.is_empty()
        {
            return Ok(Saved::Unchanged);
        }
        let object = journal::new_journal(&entry.uid, fields, home, now);
        put(state, entry.calendar_id, &entry.href, object)?;
        state.journal_draft = None;
        let _ = state
            .store
            .set_setting("default_journal_calendar", &entry.calendar_id.to_string());
        return Ok(Saved::Created);
    }
    let mut object = load(state, entry)?;
    let changed = journal::update_journal(&mut object, &entry.uid, fields, home, now)
        .map_err(|e| e.to_string())?;
    if !changed {
        return Ok(Saved::Unchanged);
    }
    put(state, entry.calendar_id, &entry.href, object)?;
    Ok(Saved::Updated)
}

/// Eintrag löschen
pub fn delete(state: &mut State, entry: &Entry) -> Result<(), String> {
    if entry.draft {
        state.journal_draft = None;
        return Ok(());
    }
    if entry.read_only {
        return Err("Dieser Kalender ist schreibgeschützt.".into());
    }
    if entry.calendar_id == DEMO {
        state.demo_journals.retain(|(h, _)| *h != entry.href);
        return Ok(());
    }
    state
        .store
        .local_delete(entry.calendar_id, &entry.href)
        .map_err(|e| e.to_string())?;
    state.send(crate::sync::Command::Push);
    Ok(())
}

/// Eintrag in einen anderen Kalender verschieben (gleiche UID, neue
/// Ressource dort, alte gelöscht). Liefert den neuen Schlüssel.
pub fn move_to(state: &mut State, entry: &Entry, target: i64) -> Result<String, String> {
    if target == entry.calendar_id {
        return Ok(entry.key());
    }
    let (name, color) = calendars(state)
        .into_iter()
        .find(|c| c.0 == target)
        .map(|c| (c.1, c.2))
        .ok_or("Dieser Kalender nimmt keine Journaleinträge an.")?;
    let href = href_in(state, target, &entry.uid)?;
    if entry.draft {
        let mut draft = entry.clone();
        draft.calendar_id = target;
        draft.href = href;
        draft.calendar_name = name;
        draft.color = color;
        let key = draft.key();
        state.journal_draft = Some(draft);
        return Ok(key);
    }
    if entry.read_only {
        return Err("Dieser Kalender ist schreibgeschützt.".into());
    }
    let object = load(state, entry)?;
    put(state, target, &href, object)?;
    delete(state, entry)?;
    let _ = state
        .store
        .set_setting("default_journal_calendar", &target.to_string());
    Ok(key_of(target, &href))
}

/// Tag des Eintrags für Liste und Editor: ("6", "FR.", "NOV. 2026")
pub fn date_labels(day: NaiveDate) -> (String, String, String) {
    (
        day.day().to_string(),
        WEEKDAYS_SHORT[day.weekday().num_days_from_monday() as usize].to_uppercase(),
        format!("{} {}", MONTHS_SHORT[day.month0() as usize], day.year()).to_uppercase(),
    )
}

/// Vorschau in der Liste: Text ohne Markdown-Zeichen, eine Zeile
pub fn preview(text: &str) -> String {
    let mut out = String::new();
    let mut fence = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if fence || line.is_empty() || is_rule(line) {
            continue;
        }
        let mut line = line.trim_start_matches('#').trim_start();
        line = line.trim_start_matches('>').trim_start();
        for bullet in ["- ", "* ", "+ "] {
            if let Some(rest) = line.strip_prefix(bullet) {
                line = rest;
            }
        }
        if let Some((number, rest)) = line.split_once(". ")
            && !number.is_empty()
            && number.chars().all(|c| c.is_ascii_digit())
        {
            line = rest;
        }
        let plain = strip_inline(line);
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(plain.trim());
        if out.chars().count() > 240 {
            break;
        }
    }
    out.chars().take(240).collect()
}

/// Inline-Markdown entfernen: **fett**, *kursiv*, `Code`, ~~durch~~, [Link](url)
fn strip_inline(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '[' {
            // [Text](Ziel) → Text
            if let Some(close) = chars[i..].iter().position(|&x| x == ']').map(|p| p + i)
                && chars.get(close + 1) == Some(&'(')
                && let Some(end) = chars[close..]
                    .iter()
                    .position(|&x| x == ')')
                    .map(|p| p + close)
            {
                out.extend(&chars[i + 1..close]);
                i = end + 1;
                continue;
            }
        }
        if matches!(c, '*' | '`' | '~') || (c == '_' && (i == 0 || !chars[i - 1].is_alphanumeric()))
        {
            i += 1;
            continue;
        }
        if c == '_' && chars.get(i + 1).is_none_or(|n| !n.is_alphanumeric()) {
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn is_rule(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && ['-', '*', '_']
            .iter()
            .any(|m| compact.chars().all(|c| c == *m))
}

/// Block der Leseansicht
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// Absatz oder Liste (Markdown für Slints StyledText)
    Paragraph,
    Heading(u8),
    Quote,
    Rule,
    /// Code-Block, als Klartext
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub text: String,
}

/// Text in Blöcke zerlegen. Slints StyledText kann Absätze, Listen, fett,
/// kursiv, durchgestrichen, Code und Links; Überschriften, Zitate, Linien
/// und Code-Blöcke legt Hora selbst an.
pub fn blocks(text: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut quote: Vec<&str> = Vec::new();
    let mut code: Option<Vec<&str>> = None;
    fn flush(out: &mut Vec<Block>, lines: &mut Vec<&str>, kind: BlockKind) {
        if !lines.is_empty() {
            out.push(Block {
                kind,
                text: lines.join("\n"),
            });
            lines.clear();
        }
    }
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(lines) = code.as_mut() {
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                out.push(Block {
                    kind: BlockKind::Code,
                    text: lines.join("\n"),
                });
                code = None;
            } else {
                lines.push(line);
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            flush(&mut out, &mut paragraph, BlockKind::Paragraph);
            flush(&mut out, &mut quote, BlockKind::Quote);
            code = Some(Vec::new());
            continue;
        }
        if trimmed.is_empty() {
            flush(&mut out, &mut paragraph, BlockKind::Paragraph);
            flush(&mut out, &mut quote, BlockKind::Quote);
            continue;
        }
        let hashes = trimmed.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            flush(&mut out, &mut paragraph, BlockKind::Paragraph);
            flush(&mut out, &mut quote, BlockKind::Quote);
            out.push(Block {
                kind: BlockKind::Heading(hashes.min(3) as u8),
                text: trimmed[hashes..]
                    .trim()
                    .trim_end_matches('#')
                    .trim()
                    .to_string(),
            });
            continue;
        }
        if is_rule(trimmed) {
            flush(&mut out, &mut paragraph, BlockKind::Paragraph);
            flush(&mut out, &mut quote, BlockKind::Quote);
            out.push(Block {
                kind: BlockKind::Rule,
                text: String::new(),
            });
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('>') {
            flush(&mut out, &mut paragraph, BlockKind::Paragraph);
            quote.push(rest.strip_prefix(' ').unwrap_or(rest));
            continue;
        }
        flush(&mut out, &mut quote, BlockKind::Quote);
        paragraph.push(line);
    }
    if let Some(lines) = code {
        out.push(Block {
            kind: BlockKind::Code,
            text: lines.join("\n"),
        });
    }
    flush(&mut out, &mut paragraph, BlockKind::Paragraph);
    flush(&mut out, &mut quote, BlockKind::Quote);
    out
}

/// Strg+B / Strg+I: Auswahl (Byte-Offsets) mit `marker` umschließen; ohne
/// Auswahl ein leeres Paar einfügen und den Cursor dazwischen setzen. Ist die
/// Auswahl schon umschlossen, wird die Markierung entfernt.
/// Liefert (Text, Anker, Cursor).
pub fn format(text: &str, anchor: usize, cursor: usize, marker: &str) -> (String, usize, usize) {
    let clamp = |i: usize| {
        let mut i = i.min(text.len());
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let (start, end) = {
        let (a, c) = (clamp(anchor), clamp(cursor));
        (a.min(c), a.max(c))
    };
    let m = marker.len();
    let before = &text[..start];
    let after = &text[end..];
    // Schon umschlossen (und nicht Teil eines längeren Markers wie ** bei *)
    let wrapped = before.ends_with(marker)
        && after.starts_with(marker)
        && !(m == 1
            && before.ends_with("**")
            && after.starts_with("**")
            && !text[start..end].starts_with('*'));
    if wrapped && start != end {
        let mut out = String::with_capacity(text.len());
        out.push_str(&before[..before.len() - m]);
        out.push_str(&text[start..end]);
        out.push_str(&after[m..]);
        return (out, start - m, end - m);
    }
    let mut out = String::with_capacity(text.len() + 2 * m);
    out.push_str(before);
    out.push_str(marker);
    out.push_str(&text[start..end]);
    out.push_str(marker);
    out.push_str(after);
    (out, start + m, end + m)
}

/// Beispiel-Einträge für den Start ohne Konto
pub fn sample(today: NaiveDate, now: chrono::DateTime<Utc>) -> Vec<(String, CalendarObject)> {
    let home = chrono_tz::Europe::Berlin;
    let entries: [(u64, &str, &str, &[&str]); 4] = [
        (
            0,
            "Lauftreff im Nebel",
            "Heute früh mit der Gruppe **10 km** gelaufen, der Park lag komplett im Nebel.\n\n- Tempo locker, *5:40* pro Kilometer\n- Nächstes Mal Stirnlampe mitnehmen\n\n> Die ersten drei Kilometer sind immer die schwersten.",
            &["Sport", "draußen"],
        ),
        (
            2,
            "Elternabend vorbereiten",
            "## Themen\n\n1. Klassenfahrt im Mai\n2. Neue Hausaufgaben-Regeln\n3. Weihnachtsfeier\n\nUnterlagen liegen im Ordner *Schule*.",
            &["Schule"],
        ),
        (
            5,
            "",
            "Abends noch lange mit Oma telefoniert. Sie erzählt wieder vom Garten und will im Frühjahr Tomaten ziehen.",
            &["Familie"],
        ),
        (
            9,
            "Rückblick Oktober",
            "Viel geschafft: Umzug geplant, Steuer erledigt, endlich wieder regelmäßig gelaufen.\n\n---\n\nFür November: **mehr lesen**, weniger Bildschirm.",
            &[],
        ),
    ];
    entries
        .iter()
        .enumerate()
        .map(|(i, (ago, title, text, tags))| {
            let uid = format!("beispiel-journal-{i}");
            let fields = JournalFields {
                summary: title.to_string(),
                description: text.to_string(),
                day: today - Days::new(*ago),
                categories: tags.iter().map(|t| t.to_string()).collect(),
            };
            let created = now - chrono::Duration::days(*ago as i64);
            (
                format!("demo/{uid}.ics"),
                journal::new_journal(&uid, &fields, home, created),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn blocks_split_headings_quotes_rules_and_code() {
        let text = "# Titel #\n\nErster Absatz\nmit Umbruch\n- eins\n- **zwei**\n\n> Zitat\n> weiter\n***\n```\nlet x = 1;\n```\n### Drei\n#### Vier\n#kein Titel";
        let got = blocks(text);
        let kinds: Vec<BlockKind> = got.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            vec![
                BlockKind::Heading(1),
                BlockKind::Paragraph,
                BlockKind::Quote,
                BlockKind::Rule,
                BlockKind::Code,
                BlockKind::Heading(3),
                BlockKind::Heading(3),
                BlockKind::Paragraph,
            ]
        );
        assert_eq!(got[0].text, "Titel");
        assert_eq!(
            got[1].text,
            "Erster Absatz\nmit Umbruch\n- eins\n- **zwei**"
        );
        assert_eq!(got[2].text, "Zitat\nweiter");
        assert_eq!(got[4].text, "let x = 1;");
        assert_eq!(got[7].text, "#kein Titel");
        // Jeder Absatz geht durch Slints Markdown-Parser
        for b in got.iter().filter(|b| b.kind == BlockKind::Paragraph) {
            assert!(
                slint::StyledText::from_markdown(&b.text).is_ok(),
                "{}",
                b.text
            );
        }
    }

    #[test]
    fn preview_drops_markdown_markers() {
        assert_eq!(
            preview(
                "## Themen\n\n1. Klassen_fahrt **im** Mai\n- [Link](https://x.de) und `code`\n> Zitat\n---\n```\nnicht\n```"
            ),
            "Themen Klassen_fahrt im Mai Link und code Zitat"
        );
        assert_eq!(preview("_kursiv_ und ~~weg~~"), "kursiv und weg");
    }

    #[test]
    fn format_wraps_and_unwraps_the_selection() {
        let text = "ein Wort hier";
        // "Wort" fett
        let (t, a, c) = format(text, 4, 8, "**");
        assert_eq!(t, "ein **Wort** hier");
        assert_eq!(&t[a..c], "Wort");
        // noch einmal: wieder normal
        let (t2, a2, c2) = format(&t, a, c, "**");
        assert_eq!(t2, text);
        assert_eq!((a2, c2), (4, 8));
        // fett und kursiv ergibt drei Sternchen (wie in Calinos Test)
        let (t3, a3, c3) = format(&t, a, c, "*");
        assert_eq!(t3, "ein ***Wort*** hier");
        assert_eq!(&t3[a3..c3], "Wort");
        // ohne Auswahl: leeres Paar, Cursor in der Mitte; Umlaute sind sicher
        let (t4, a4, c4) = format("schön", 99, 99, "*");
        assert_eq!(t4, "schön**");
        assert_eq!((a4, c4), ("schön*".len(), "schön*".len()));
    }

    #[test]
    fn date_labels_are_german_capitals() {
        assert_eq!(
            date_labels(day(2026, 11, 6)),
            ("6".into(), "FR.".into(), "NOV. 2026".into())
        );
    }

    #[test]
    fn filter_keeps_one_month() {
        let now = Utc::now();
        let today = day(2026, 11, 3);
        let samples = sample(today, now);
        assert_eq!(samples.len(), 4);
        let days: Vec<NaiveDate> = samples
            .iter()
            .map(|(_, o)| o.journals()[0].day(chrono_tz::Europe::Berlin).unwrap())
            .collect();
        assert_eq!(
            days,
            vec![
                day(2026, 11, 3),
                day(2026, 11, 1),
                day(2026, 10, 29),
                day(2026, 10, 25)
            ]
        );
        let entries: Vec<Entry> = samples
            .iter()
            .zip(&days)
            .map(|((href, o), d)| Entry {
                calendar_id: DEMO,
                href: href.clone(),
                uid: o.uid().unwrap(),
                info: o.journals().remove(0),
                day: *d,
                color: 0,
                calendar_name: String::new(),
                read_only: false,
                draft: false,
            })
            .collect();
        assert_eq!(filter(entries.clone(), Some(day(2026, 11, 20))).len(), 2);
        assert_eq!(filter(entries, None).len(), 4);
    }
}
