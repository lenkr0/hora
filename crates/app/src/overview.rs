//! Agenda und Jahr wie Calinos AgendaView und YearView: die Agenda listet die
//! Tage eines Monats mit ihren Terminen und fasst leere Tage zusammen, das
//! Jahr zeigt zwölf Monate mit einem Punkt an belegten Tagen.

use std::collections::HashMap;

use chrono::{Datelike, Duration, NaiveDate, Timelike};
use hora_core::Tz;

use crate::forms::{MONTHS, WEEKDAYS, format_time, short_day};
use crate::items::Item;

/// Eine Zeile der Agenda
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgendaRow {
    Day {
        date: NaiveDate,
        today: bool,
        empty: bool,
    },
    Item {
        item: usize,
        date: NaiveDate,
    },
    /// Mehrere leere Tage hintereinander
    Skip {
        first: NaiveDate,
        last: NaiveDate,
    },
}

/// Zeilen für die Tage `first` bis `last` (einschließlich)
pub fn agenda_rows(
    items: &[Item],
    first: NaiveDate,
    last: NaiveDate,
    today: NaiveDate,
    home: Tz,
) -> Vec<AgendaRow> {
    let mut per_day: HashMap<NaiveDate, Vec<usize>> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        let (from, to) = item.span.days(home);
        let mut day = from.max(first);
        while day <= to.min(last) {
            per_day.entry(day).or_default().push(i);
            day += Duration::days(1);
        }
    }
    let mut rows = Vec::new();
    let mut empty_run: Vec<NaiveDate> = Vec::new();
    let flush = |run: &mut Vec<NaiveDate>, rows: &mut Vec<AgendaRow>| {
        if run.len() >= 2 {
            rows.push(AgendaRow::Skip {
                first: run[0],
                last: *run.last().unwrap(),
            });
        } else {
            for d in run.iter() {
                rows.push(AgendaRow::Day {
                    date: *d,
                    today: false,
                    empty: true,
                });
            }
        }
        run.clear();
    };
    let mut day = first;
    while day <= last {
        let mut ids = per_day.remove(&day).unwrap_or_default();
        if ids.is_empty() && day != today {
            empty_run.push(day);
        } else {
            flush(&mut empty_run, &mut rows);
            ids.sort_by_key(|i| {
                let it = &items[*i];
                (
                    !it.span.is_all_day_like(),
                    !it.is_task(),
                    it.span.start,
                    it.title.clone(),
                )
            });
            rows.push(AgendaRow::Day {
                date: day,
                today: day == today,
                empty: ids.is_empty(),
            });
            rows.extend(
                ids.into_iter()
                    .map(|item| AgendaRow::Item { item, date: day }),
            );
        }
        day += Duration::days(1);
    }
    flush(&mut empty_run, &mut rows);
    rows
}

/// "3 Tage ohne Termine · 10.–12. Nov."
pub fn skip_label(first: NaiveDate, last: NaiveDate) -> String {
    let days = (last - first).num_days() + 1;
    let range = if first.month() == last.month() {
        format!("{}.–{}. {}", first.day(), last.day(), short_month(last))
    } else {
        format!("{} – {}", short_day(first), short_day(last))
    };
    format!("{days} Tage ohne Termine · {range}")
}

fn short_month(d: NaiveDate) -> &'static str {
    const SHORT: [&str; 12] = [
        "Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sept.", "Okt.", "Nov.",
        "Dez.",
    ];
    SHORT[d.month0() as usize]
}

/// Wochentag und Datum für den Tageskopf: ("Montag", "9. November")
pub fn day_label(date: NaiveDate) -> (String, String) {
    (
        WEEKDAYS[date.weekday().num_days_from_monday() as usize].to_string(),
        format!("{}. {}", date.day(), MONTHS[date.month0() as usize]),
    )
}

/// Zeit-Spalte und Zusatzzeile eines Termins an einem Tag
pub fn item_texts(item: &Item, date: NaiveDate, home: Tz) -> (String, String) {
    let local = |t: chrono::DateTime<chrono::Utc>| t.with_timezone(&home).naive_local();
    let (start, end) = (local(item.span.start), local(item.span.end));
    let mut sub: Vec<String> = Vec::new();
    let time = if item.is_task() {
        "Fällig".to_string()
    } else if item.span.is_all_day_like() {
        let (from, to) = item.span.days(home);
        if from != to {
            sub.push(format!("{} – {}", short_day(from), short_day(to)));
        }
        "ganztägig".to_string()
    } else if start.date() == date {
        if end.date() == date
            || (end.time().num_seconds_from_midnight() == 0
                && end.date() == date + Duration::days(1))
        {
            sub.push(format!("bis {}", format_time(end.time())));
        } else {
            sub.push(format!(
                "bis {}, {}",
                short_day(end.date()),
                format_time(end.time())
            ));
        }
        format_time(start.time())
    } else {
        sub.push(format!(
            "seit {}, {}",
            short_day(start.date()),
            format_time(start.time())
        ));
        format!("bis {}", format_time(end.time()))
    };
    if !item.location.trim().is_empty() {
        sub.push(item.location.trim().to_string());
    }
    (time, sub.join(" · "))
}

/// Ein Tag in der Jahresansicht
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YearDay {
    /// `None` für Felder vor dem Ersten bzw. nach dem Letzten
    pub date: Option<NaiveDate>,
    pub count: usize,
}

/// Zwölf Monate zu je sechs Wochen (Montag zuerst)
pub fn year_grid(year: i32, counts: &HashMap<NaiveDate, usize>) -> Vec<Vec<YearDay>> {
    (1..=12)
        .map(|month| {
            let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or_default();
            let offset = first.weekday().num_days_from_monday() as i64;
            (0..42)
                .map(|cell| {
                    let date = first + Duration::days(cell - offset);
                    let inside = date.month() == month && date.year() == year;
                    YearDay {
                        date: inside.then_some(date),
                        count: if inside {
                            counts.get(&date).copied().unwrap_or(0)
                        } else {
                            0
                        },
                    }
                })
                .collect()
        })
        .collect()
}

/// Wie viele Einträge je Tag (mehrtägige zählen an jedem Tag)
pub fn day_counts(items: &[Item], home: Tz) -> HashMap<NaiveDate, usize> {
    let mut counts = HashMap::new();
    for item in items {
        let (from, to) = item.span.days(home);
        let mut day = from;
        while day <= to && (day - from).num_days() < 400 {
            *counts.entry(day).or_insert(0) += 1;
            day += Duration::days(1);
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::Source;
    use crate::view::Span;
    use chrono::{NaiveDateTime, TimeZone, Utc};
    use chrono_tz::Europe::Berlin;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    fn timed(title: &str, start: &str, end: &str) -> Item {
        let at = |s: &str| {
            Berlin
                .from_local_datetime(&NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap())
                .unwrap()
                .with_timezone(&Utc)
        };
        Item {
            source: Source::DemoEvent(0),
            title: title.into(),
            location: "Raum 204".into(),
            span: Span {
                start: at(start),
                end: at(end),
                dates: None,
            },
            color: 0,
        }
    }

    #[test]
    fn agenda_groups_days_and_skips_empty_runs() {
        let items = vec![
            timed("Mathe", "2026-11-02 08:00", "2026-11-02 09:30"),
            timed("Physik", "2026-11-02 07:00", "2026-11-02 07:45"),
            timed("Kino", "2026-11-06 20:00", "2026-11-06 22:00"),
            timed("Nachtschicht", "2026-11-08 22:00", "2026-11-09 06:00"),
        ];
        let rows = agenda_rows(&items, d(11, 1), d(11, 9), d(11, 5), Berlin);
        let shape: Vec<String> = rows
            .iter()
            .map(|r| match r {
                AgendaRow::Day { date, today, empty } => format!(
                    "{}{}{}",
                    date.day(),
                    if *today { " heute" } else { "" },
                    if *empty { " leer" } else { "" }
                ),
                AgendaRow::Item { item, .. } => format!("  {}", items[*item].title),
                AgendaRow::Skip { first, last } => format!("({}-{})", first.day(), last.day()),
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                "1 leer",
                "2",
                "  Physik",
                "  Mathe",
                "(3-4)",
                "5 heute leer",
                "6",
                "  Kino",
                "7 leer",
                "8",
                "  Nachtschicht",
                "9",
                "  Nachtschicht",
            ]
        );
        assert_eq!(
            skip_label(d(11, 10), d(11, 12)),
            "3 Tage ohne Termine · 10.–12. Nov."
        );
        assert_eq!(day_label(d(11, 9)), ("Montag".into(), "9. November".into()));
        assert_eq!(
            item_texts(&items[0], d(11, 2), Berlin),
            ("08:00".into(), "bis 09:30 · Raum 204".into())
        );
        assert_eq!(
            item_texts(&items[3], d(11, 8), Berlin).1,
            "bis Mo. 9. Nov., 06:00 · Raum 204"
        );
        assert_eq!(item_texts(&items[3], d(11, 9), Berlin).0, "bis 06:00");
    }

    #[test]
    fn year_grid_starts_on_monday() {
        let counts = HashMap::from([(d(11, 5), 3)]);
        let grid = year_grid(2026, &counts);
        assert_eq!(grid.len(), 12);
        // 1. November 2026 ist ein Sonntag: sechs leere Felder davor
        let nov = &grid[10];
        assert!(nov[..6].iter().all(|c| c.date.is_none()));
        assert_eq!(nov[6].date, Some(d(11, 1)));
        assert_eq!(
            nov.iter().find(|c| c.date == Some(d(11, 5))).unwrap().count,
            3
        );
        assert_eq!(nov.iter().filter(|c| c.date.is_some()).count(), 30);
    }
}
