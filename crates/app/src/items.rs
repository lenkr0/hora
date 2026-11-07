//! Termine eines Zeitraums aus Beispielterminen und Cache, einheitlich als
//! [`Item`] mit ihrer Herkunft für Vorschau und Bearbeiten.

use hora_i18n::tr;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use hora_core::expand::expand;
use hora_core::model::{CalendarObject, EventInfo};
use hora_core::time::{DateTimeValue, local_to_utc};

use crate::view::Span;
use crate::{State, calendar_color};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    DemoEvent(i32),
    DemoAllDay(i32),
    Server {
        calendar_id: i64,
        href: String,
        uid: String,
        /// Bei Serien: ursprünglicher Beginn des Vorkommens (RECURRENCE-ID)
        instance: Option<DateTime<Utc>>,
        read_only: bool,
    },
    /// Offene Aufgabe am Tag ihrer Fälligkeit
    Task {
        calendar_id: i64,
        href: String,
        uid: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub source: Source,
    pub title: String,
    pub location: String,
    pub span: Span,
    pub color: u32,
}

impl Item {
    /// Termin darf geändert werden (Kalender nicht schreibgeschützt)
    pub fn writable(&self) -> bool {
        !matches!(
            self.source,
            Source::Server {
                read_only: true,
                ..
            }
        )
    }

    /// Zeiten dürfen per Ziehen oder Tastatur geändert werden (Aufgaben nicht)
    pub fn movable(&self) -> bool {
        self.writable() && !self.is_task()
    }

    pub fn is_task(&self) -> bool {
        matches!(self.source, Source::Task { .. })
    }

    /// Vorkommen einer Serie: Ändern fragt nach "nur dieser / folgende / alle"
    pub fn recurring(&self) -> bool {
        matches!(
            self.source,
            Source::Server {
                instance: Some(_),
                ..
            }
        )
    }

    /// Schlüssel, der beim Neuladen gleich bleibt, auch wenn der Termin
    /// verschoben wurde. Vorkommen einer Serie unterscheidet ihr
    /// ursprünglicher Beginn.
    pub fn key(&self) -> String {
        self.source.key()
    }
}

impl Source {
    /// Siehe [`Item::key`]
    pub fn key(&self) -> String {
        match self {
            Source::DemoEvent(id) => format!("e{id}"),
            Source::DemoAllDay(id) => format!("a{id}"),
            Source::Task {
                calendar_id,
                href,
                uid,
            } => format!("t|{calendar_id}|{href}|{uid}"),
            Source::Server {
                calendar_id,
                href,
                instance: None,
                ..
            } => format!("{calendar_id}|{href}"),
            Source::Server {
                calendar_id,
                href,
                uid,
                instance: Some(instance),
                ..
            } => format!("{calendar_id}|{href}|{uid}|{}", instance.timestamp()),
        }
    }
}

fn hash_of(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Gelesene Termine einer Ressource; neu gelesen, wenn sich der Inhalt ändert
/// (auch bei lokalen Änderungen, deren ETag noch fehlt).
pub fn parsed_events<'a>(
    parsed: &'a mut HashMap<(i64, String), (u64, Vec<EventInfo>)>,
    calendar_id: i64,
    href: &str,
    ics: &str,
) -> &'a [EventInfo] {
    let hash = hash_of(ics);
    let entry = parsed
        .entry((calendar_id, href.to_string()))
        .or_insert((0, Vec::new()));
    if entry.0 != hash || entry.1.is_empty() {
        *entry = (
            hash,
            CalendarObject::parse(ics)
                .map(|o| o.events().0)
                .unwrap_or_default(),
        );
    }
    &entry.1
}

/// Alle Termine, die `days` Tage ab `first_day` berühren.
pub fn collect(state: &mut State, first_day: NaiveDate, days: i64) -> Vec<Item> {
    let home = state.home;
    let last = first_day + Duration::days(days);
    let from = local_to_utc(first_day.and_time(NaiveTime::MIN), home);
    let to = local_to_utc(last.and_time(NaiveTime::MIN), home);
    let mut items = Vec::new();

    if state.demo_visible {
        for e in &state.events {
            let span = Span {
                start: local_to_utc(e.start, home),
                end: local_to_utc(e.end, home),
                dates: None,
            };
            if span.end > from && span.start < to {
                items.push(Item {
                    source: Source::DemoEvent(e.id),
                    title: e.title.clone(),
                    location: e.location.clone(),
                    span,
                    color: e.color,
                });
            }
        }
        for a in state
            .all_day
            .iter()
            .filter(|a| a.first < last && a.end > first_day)
        {
            items.push(Item {
                source: Source::DemoAllDay(a.id),
                title: a.title.clone(),
                location: String::new(),
                span: Span {
                    start: local_to_utc(a.first.and_time(NaiveTime::MIN), home),
                    end: local_to_utc(a.end.and_time(NaiveTime::MIN), home),
                    dates: Some((a.first, a.end)),
                },
                color: a.color,
            });
        }
    }

    let calendars: HashMap<i64, (u32, bool)> = state
        .calendars
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id, (calendar_color(c, i), c.read_only)))
        .collect();
    let resources = state
        .store
        .events_in_range(from.timestamp(), to.timestamp())
        .unwrap_or_default();
    for resource in resources {
        let (color, read_only) = calendars
            .get(&resource.calendar_id)
            .copied()
            .unwrap_or((crate::CALENDAR_COLORS[0], false));
        let events = parsed_events(
            &mut state.parsed,
            resource.calendar_id,
            &resource.href,
            &resource.ics,
        );
        for occurrence in expand(events, home, from, to) {
            items.push(Item {
                source: Source::Server {
                    calendar_id: resource.calendar_id,
                    href: resource.href.clone(),
                    uid: occurrence.uid.clone(),
                    instance: occurrence.instance,
                    read_only,
                },
                title: if occurrence.summary.trim().is_empty() {
                    tr!("(untitled)").to_string()
                } else {
                    occurrence.summary.clone()
                },
                location: occurrence.location.clone(),
                span: Span::from(&occurrence),
                color,
            });
        }
    }

    // Offene Aufgaben am Tag ihrer Fälligkeit (wie Calino, abschaltbar)
    if state.settings.show_tasks {
        let tasks = crate::tasks::collect(state);
        for task in tasks.iter().filter(|t| !t.done) {
            let Some(due) = task.due else {
                continue;
            };
            let day = match due {
                DateTimeValue::Date(d) => d,
                other => other.to_utc(home).with_timezone(&home).date_naive(),
            };
            if day < first_day || day >= last {
                continue;
            }
            let title = if task.info.summary.trim().is_empty() {
                tr!("(untitled)")
            } else {
                task.info.summary.trim()
            };
            let title = match due {
                DateTimeValue::Date(_) => title.to_string(),
                other => format!(
                    "{} {title}",
                    crate::forms::format_time(other.to_utc(home).with_timezone(&home).time())
                ),
            };
            let next = day + Duration::days(1);
            items.push(Item {
                source: Source::Task {
                    calendar_id: task.calendar_id,
                    href: task.href.clone(),
                    uid: task.uid.clone(),
                },
                title,
                location: String::new(),
                span: Span {
                    start: local_to_utc(day.and_time(NaiveTime::MIN), home),
                    end: local_to_utc(next.and_time(NaiveTime::MIN), home),
                    dates: Some((day, next)),
                },
                color: task.color,
            });
        }
        state.tasks = tasks;
    }
    items
}
