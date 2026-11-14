//! Befehlspalette (Strg+K) wie Calinos CommandPalette: Befehle, Sprung zu
//! einem Datum, Kalender ein-/ausblenden, Suche über Termine, Aufgaben und
//! Journaleinträge und "Erstellen: …" aus der Schnelleingabe.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use hora_core::expand::expand;
use hora_core::model::EventInfo;
use hora_core::quick;

use crate::forms::{describe, short_day};
use crate::items::Source;
use crate::{State, View, calendar_color, journal, tasks};
use hora_i18n::tr;

/// Was ein Eintrag der Palette tut
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    View(View),
    Today,
    Tomorrow,
    Prev,
    Next,
    NewEvent,
    NewTask,
    Quick,
    Sync,
    Settings,
    AddAccount,
    Theme,
    About,
    ToggleCalendar(i64),
    GoTo(chrono::NaiveDate),
    Create(String),
    /// Termin zeigen: Schlüssel wie [`crate::items::Item::key`] und Beginn
    OpenEvent {
        key: String,
        start: DateTime<Utc>,
    },
    OpenTask {
        calendar_id: i64,
        href: String,
        uid: String,
    },
    NewJournal,
    /// Journaleintrag zeigen: Schlüssel wie [`journal::Entry::key`] und Tag
    OpenJournal {
        key: String,
        day: chrono::NaiveDate,
    },
}

/// Ein Eintrag (ohne Gruppenüberschriften)
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub group: &'static str,
    pub title: String,
    pub detail: String,
    pub badge: String,
    pub badge_color: Option<u32>,
    pub shortcut: String,
    pub action: Action,
}

struct Command {
    group: &'static str,
    title: &'static str,
    keywords: &'static str,
    shortcut: &'static str,
    action: Action,
}

fn commands() -> Vec<Command> {
    let c = |group, title, keywords, shortcut, action| Command {
        group,
        title,
        keywords,
        shortcut,
        action,
    };
    vec![
        c(
            tr!("NAVIGATION"),
            tr!("Go to today"),
            "heute today",
            "T",
            Action::Today,
        ),
        c(
            tr!("NAVIGATION"),
            tr!("Go to tomorrow"),
            "morgen tomorrow",
            "",
            Action::Tomorrow,
        ),
        c(
            tr!("NAVIGATION"),
            tr!("Day view"),
            "tag day ansicht",
            "D",
            Action::View(View::Day),
        ),
        c(
            tr!("NAVIGATION"),
            tr!("3-day view"),
            "drei tage three days ansicht",
            "X",
            Action::View(View::ThreeDays),
        ),
        c(
            tr!("NAVIGATION"),
            tr!("Week view"),
            "woche week ansicht",
            "W",
            Action::View(View::Week),
        ),
        c(
            tr!("NAVIGATION"),
            tr!("Month view"),
            "monat month ansicht",
            "M",
            Action::View(View::Month),
        ),
        c(
            tr!("NAVIGATION"),
            tr!("Year view"),
            "jahr year ansicht",
            "J",
            Action::View(View::Year),
        ),
        c(
            tr!("NAVIGATION"),
            tr!("Agenda view"),
            "agenda liste list ansicht",
            "L",
            Action::View(View::Agenda),
        ),
        c(
            tr!("NAVIGATION"),
            tr!("Tasks view"),
            "aufgaben tasks todo ansicht",
            "A",
            Action::View(View::Tasks),
        ),
        c(
            tr!("NAVIGATION"),
            tr!("Journal"),
            "journal tagebuch notizen einträge ansicht",
            "B",
            Action::View(View::Journal),
        ),
        c(
            tr!("NAVIGATION"),
            tr!("App time"),
            "app zeiten zeit aktivitäten stunden ziel app time activities",
            "Z",
            Action::View(View::Activities),
        ),
        c(
            tr!("NAVIGATION"),
            tr!("Previous"),
            "zurück vorherige vorheriger previous back woche monat",
            ",",
            Action::Prev,
        ),
        c(
            tr!("NAVIGATION"),
            tr!("Next"),
            "weiter nächste nächster next woche monat",
            ".",
            Action::Next,
        ),
        c(
            tr!("ACTIONS"),
            tr!("Create event"),
            "neu neuer termin new event erstellen",
            "N",
            Action::NewEvent,
        ),
        c(
            tr!("ACTIONS"),
            tr!("New task"),
            "neu aufgabe task todo erstellen",
            "",
            Action::NewTask,
        ),
        c(
            tr!("ACTIONS"),
            tr!("New journal entry"),
            "neu neuer journal eintrag tagebuch notiz schreiben",
            "",
            Action::NewJournal,
        ),
        c(
            tr!("ACTIONS"),
            tr!("Quick add"),
            "schnell quick eingabe eintragen",
            "Q",
            Action::Quick,
        ),
        c(
            tr!("ACTIONS"),
            tr!("Sync calendars"),
            "sync abgleichen synchronisieren aktualisieren",
            "F5",
            Action::Sync,
        ),
        c(
            tr!("ACTIONS"),
            tr!("Add account"),
            "konto account caldav server verbinden",
            "",
            Action::AddAccount,
        ),
        c(
            tr!("SETTINGS"),
            tr!("Open settings"),
            "einstellungen settings optionen erinnerungen autostart",
            tr!("Ctrl+,"),
            Action::Settings,
        ),
        c(
            tr!("SETTINGS"),
            tr!("Switch theme (Light → Dark → System)"),
            "theme hell dunkel dark light modus design",
            "",
            Action::Theme,
        ),
        c(
            tr!("SETTINGS"),
            tr!("About Hora"),
            "über about version lizenz",
            "",
            Action::About,
        ),
    ]
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Alle Wörter der Suche kommen im Text vor
fn matches(haystack: &str, query: &[String]) -> bool {
    !query.is_empty() && query.iter().all(|w| haystack.contains(w.as_str()))
}

/// Suchindex über Termine; wird bei Datenänderungen verworfen
pub struct SearchDoc {
    calendar_id: i64,
    href: String,
    uid: String,
    summary: String,
    location: String,
    haystack: String,
    events: Vec<EventInfo>,
}

pub fn build_index(state: &mut State) -> Vec<SearchDoc> {
    let mut docs = Vec::new();
    for resource in state.store.all_events().unwrap_or_default() {
        let events = crate::items::parsed_events(
            &mut state.parsed,
            resource.calendar_id,
            &resource.href,
            &resource.ics,
        )
        .to_vec();
        let Some(master) = events
            .iter()
            .find(|e| e.recurrence_id.is_none())
            .or(events.first())
            .cloned()
        else {
            continue;
        };
        let haystack = events
            .iter()
            .map(|e| format!("{} {} {}", e.summary, e.location, e.description))
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        docs.push(SearchDoc {
            calendar_id: resource.calendar_id,
            href: resource.href,
            uid: master.uid.clone(),
            summary: master.summary.clone(),
            location: master.location.clone(),
            haystack,
            events,
        });
    }
    docs
}

/// Einträge für die Eingabe `query`
pub fn entries(state: &mut State, query: &str) -> Vec<Entry> {
    let home = state.home;
    let q = words(query);
    let mut out: Vec<Entry> = Vec::new();
    let calendar_info: HashMap<i64, (String, u32)> = state
        .calendars
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id, (c.name.clone(), calendar_color(c, i))))
        .collect();

    // App-Zeiten nur, solange sie eingeschaltet sind
    let shown =
        |c: &Command| state.settings.activities || c.action != Action::View(View::Activities);

    // Ohne Eingabe: die wichtigsten Befehle
    if q.is_empty() {
        for c in commands()
            .into_iter()
            .filter(|c| !matches!(c.action, Action::Theme | Action::About))
            .filter(|c| shown(c))
        {
            out.push(Entry {
                group: c.group,
                title: c.title.into(),
                detail: String::new(),
                badge: String::new(),
                badge_color: None,
                shortcut: c.shortcut.into(),
                action: c.action,
            });
        }
        return out;
    }

    // Schnelleingabe auslegen: Datum allein = Sprung, sonst "Erstellen: …"
    let now = state.now();
    let parsed = quick::parse(query, now.date());
    let recognized = parsed.date.is_some()
        || parsed.start.is_some()
        || parsed.rule.is_some()
        || parsed.task
        || parsed.location.is_some();
    let mut create = None;
    if parsed.title.is_empty() {
        if let Some(date) = parsed.date {
            out.push(Entry {
                group: tr!("NAVIGATION"),
                title: tr!("Go to {}", short_day(date)),
                detail: String::new(),
                badge: String::new(),
                badge_color: None,
                shortcut: String::new(),
                action: Action::GoTo(date),
            });
        }
    } else {
        let plan = crate::quick_add::plan(state, query, false);
        let when = match plan.when {
            quick::When::Timed { start, end } => describe(start, end, false),
            quick::When::AllDay { first, last } => describe(
                first.and_time(chrono::NaiveTime::MIN),
                (last + Duration::days(1)).and_time(chrono::NaiveTime::MIN),
                true,
            ),
            quick::When::Due { day, .. } => tr!("due {}", short_day(day)),
            quick::When::Undated => String::new(),
        };
        let mut detail = when;
        if let Some(loc) = &parsed.location {
            detail.push_str(&format!(" · {loc}"));
        }
        let calendar = match plan.calendar {
            Some(-1) => Some((tr!("Sample events").to_string(), crate::DEMO_COLOR)),
            Some(id) => calendar_info.get(&id).cloned(),
            None => None,
        };
        create = Some(Entry {
            group: tr!("QUICK ADD"),
            title: if parsed.task {
                tr!("Task: {}", parsed.title)
            } else {
                tr!("Create: {}", parsed.title)
            },
            detail: if plan.problem.is_empty() {
                detail
            } else {
                plan.problem.clone()
            },
            badge: calendar.as_ref().map(|c| c.0.clone()).unwrap_or_default(),
            badge_color: calendar.map(|c| c.1),
            shortcut: String::new(),
            action: Action::Create(query.to_string()),
        });
    }
    // Mit erkannten Angaben zuerst anlegen, sonst zuerst Befehle und Treffer
    if recognized && let Some(entry) = create.take() {
        out.push(entry);
    }

    for c in commands().into_iter().filter(|c| shown(c)) {
        let haystack = format!("{} {}", c.title, c.keywords).to_lowercase();
        if matches(&haystack, &q) {
            out.push(Entry {
                group: c.group,
                title: c.title.into(),
                detail: String::new(),
                badge: String::new(),
                badge_color: None,
                shortcut: c.shortcut.into(),
                action: c.action,
            });
        }
    }
    let calendar_words = format!("kalender calendar {}", tr!("Calendar")).to_lowercase();
    let demo_words = format!(
        "{calendar_words} beispieltermine sample {}",
        tr!("Sample events")
    )
    .to_lowercase();
    if matches(&demo_words, &q) {
        out.push(Entry {
            group: tr!("CALENDARS"),
            title: if state.demo_visible {
                tr!("Hide {}", tr!("Sample events"))
            } else {
                tr!("Show {}", tr!("Sample events"))
            },
            detail: String::new(),
            badge: String::new(),
            badge_color: Some(crate::DEMO_COLOR),
            shortcut: String::new(),
            action: Action::ToggleCalendar(-1),
        });
    }
    for calendar in &state.calendars {
        if matches(
            &format!("{calendar_words} {}", calendar.name).to_lowercase(),
            &q,
        ) {
            out.push(Entry {
                group: tr!("CALENDARS"),
                title: if calendar.visible {
                    tr!("Hide {}", calendar.name)
                } else {
                    tr!("Show {}", calendar.name)
                },
                detail: String::new(),
                badge: String::new(),
                badge_color: calendar_info.get(&calendar.id).map(|c| c.1),
                shortcut: String::new(),
                action: Action::ToggleCalendar(calendar.id),
            });
        }
    }

    // Termine: Treffer mit dem nächsten (sonst letzten) Vorkommen
    if state.search_index.is_none() {
        state.search_index = Some(build_index(state));
    }
    let utc_now = Utc::now();
    let mut found = 0;
    for doc in state.search_index.as_ref().into_iter().flatten() {
        if found >= 8 || !matches(&doc.haystack, &q) {
            continue;
        }
        let next = expand(&doc.events, home, utc_now, utc_now + Duration::days(730))
            .into_iter()
            .next();
        let occurrence = next.or_else(|| {
            expand(&doc.events, home, utc_now - Duration::days(730), utc_now)
                .into_iter()
                .last()
        });
        let Some(o) = occurrence else {
            continue;
        };
        let local = |t: DateTime<Utc>| t.with_timezone(&home).naive_local();
        let mut detail = describe(local(o.start), local(o.end), o.all_day());
        if !doc.location.trim().is_empty() {
            detail.push_str(&format!(" · {}", doc.location.trim()));
        }
        let calendar = calendar_info.get(&doc.calendar_id).cloned();
        out.push(Entry {
            group: tr!("EVENTS"),
            title: if o.summary.trim().is_empty() {
                doc.summary.clone()
            } else {
                o.summary.clone()
            },
            detail,
            badge: calendar.as_ref().map(|c| c.0.clone()).unwrap_or_default(),
            badge_color: calendar.map(|c| c.1),
            shortcut: String::new(),
            action: Action::OpenEvent {
                key: Source::Server {
                    calendar_id: doc.calendar_id,
                    href: doc.href.clone(),
                    uid: doc.uid.clone(),
                    instance: o.instance,
                    read_only: false,
                }
                .key(),
                start: o.start,
            },
        });
        found += 1;
    }
    // Beispieltermine (nur auf diesem Gerät)
    if state.demo_visible {
        let utc = |t: chrono::NaiveDateTime| hora_core::time::local_to_utc(t, home);
        let demo = state
            .events
            .iter()
            .map(|e| {
                (
                    Source::DemoEvent(e.id),
                    e.title.clone(),
                    e.location.clone(),
                    describe(e.start, e.end, false),
                    utc(e.start),
                )
            })
            .chain(state.all_day.iter().map(|a| {
                let at = |d: chrono::NaiveDate| d.and_time(chrono::NaiveTime::MIN);
                (
                    Source::DemoAllDay(a.id),
                    a.title.clone(),
                    String::new(),
                    describe(at(a.first), at(a.end), true),
                    utc(at(a.first)),
                )
            }));
        // Je Titel nur das nächste (sonst letzte) Vorkommen, wie bei Serien
        let mut best: Vec<(Source, String, String, String, DateTime<Utc>)> = Vec::new();
        for hit in demo.filter(|(_, title, location, ..)| {
            matches(&format!("{title} {location}").to_lowercase(), &q)
        }) {
            match best.iter_mut().find(|b| b.1 == hit.1) {
                Some(b) => {
                    let better = if b.4 >= utc_now {
                        hit.4 >= utc_now && hit.4 < b.4
                    } else {
                        hit.4 > b.4
                    };
                    if better {
                        *b = hit;
                    }
                }
                None => best.push(hit),
            }
        }
        best.sort_by_key(|b| (b.4 < utc_now, b.4));
        for (source, title, location, when, start) in best {
            if found >= 8 {
                break;
            }
            let place = location.trim();
            out.push(Entry {
                group: tr!("EVENTS"),
                title,
                detail: if place.is_empty() {
                    when
                } else {
                    format!("{when} · {place}")
                },
                badge: tr!("Sample events").into(),
                badge_color: Some(crate::DEMO_COLOR),
                shortcut: String::new(),
                action: Action::OpenEvent {
                    key: source.key(),
                    start,
                },
            });
            found += 1;
        }
    }

    // Aufgaben
    let items = tasks::collect(state);
    let mut found = 0;
    for t in items
        .iter()
        .filter(|t| !(t.recurring && t.done && t.occurrence.is_some()))
    {
        let haystack = format!("{} {}", t.info.summary, t.info.description).to_lowercase();
        if found >= 6 || !matches(&haystack, &q) {
            continue;
        }
        let mut detail = match t.due {
            Some(due) => tr!("due {}", tasks::due_label(&due, utc_now, home)),
            None => tr!("no due date").into(),
        };
        if t.done {
            detail.push_str(&format!(" · {}", tr!("done")));
        }
        out.push(Entry {
            group: tr!("TASKS"),
            title: t.info.summary.clone(),
            detail,
            badge: t.calendar_name.clone(),
            badge_color: Some(t.color),
            shortcut: String::new(),
            action: Action::OpenTask {
                calendar_id: t.calendar_id,
                href: t.href.clone(),
                uid: t.uid.clone(),
            },
        });
        found += 1;
    }
    state.tasks = items;

    // Journal: Titel, Text und Tags
    let mut found = 0;
    for e in journal::collect(state).into_iter().filter(|e| !e.draft) {
        let haystack = format!(
            "{} {} {}",
            e.info.summary,
            e.info.description,
            e.info.categories.join(" ")
        )
        .to_lowercase();
        if found >= 6 || !matches(&haystack, &q) {
            continue;
        }
        let mut detail = short_day(e.day);
        let preview = journal::preview(&e.info.description);
        if !preview.is_empty() {
            let short: String = preview.chars().take(60).collect();
            detail.push_str(" · ");
            detail.push_str(&short);
            if preview.chars().count() > 60 {
                detail.push('…');
            }
        }
        out.push(Entry {
            group: tr!("JOURNAL"),
            title: if e.info.summary.trim().is_empty() {
                tr!("Untitled entry").into()
            } else {
                e.info.summary.trim().to_string()
            },
            detail,
            badge: e.calendar_name.clone(),
            badge_color: Some(e.color),
            shortcut: String::new(),
            action: Action::OpenJournal {
                key: e.key(),
                day: e.day,
            },
        });
        found += 1;
    }

    if let Some(entry) = create {
        out.push(entry);
    }
    // Gleiche Gruppen zusammenhalten, Reihenfolge des ersten Auftretens
    let mut order: Vec<&'static str> = Vec::new();
    for e in &out {
        if !order.contains(&e.group) {
            order.push(e.group);
        }
    }
    let mut sorted = Vec::with_capacity(out.len());
    for g in order {
        sorted.extend(out.iter().filter(|e| e.group == g).cloned());
    }
    sorted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_match_all_words() {
        let q = words("woche ans");
        let hits: Vec<&str> = commands()
            .into_iter()
            .filter(|c| matches(&format!("{} {}", c.title, c.keywords).to_lowercase(), &q))
            .map(|c| c.title)
            .collect();
        assert_eq!(hits, vec!["Week view"]);
        assert!(!matches("irgendwas", &[]));
        assert!(matches("einstellungen settings", &words("SETT")));
    }
}
