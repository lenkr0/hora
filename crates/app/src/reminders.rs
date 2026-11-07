//! Erinnerungen planen und als Windows-Benachrichtigung zeigen, auch wenn
//! das Fenster im Infobereich liegt.
//!
//! Der Planer hält die Erinnerungen der nächsten Stunden bereit und rechnet
//! sie neu, wenn sich Termine ändern. Verpasste Erinnerungen (Hora lief nicht)
//! kommen nach dem Start, solange der Termin noch nicht vorbei ist. Was schon
//! gezeigt wurde, merkt sich die Datenbank.

use std::hash::{DefaultHasher, Hash, Hasher};

use chrono::{DateTime, Duration, NaiveTime, Timelike, Utc};
use hora_core::Tz;
use hora_core::alarm::{Alarm, MAX_AFTER, MAX_BEFORE, Trigger, reminders_between};
use hora_core::task::task_reminders;
use hora_platform::notify::Toast;

use crate::State;
use crate::forms::{format_date, format_time, short_day};

/// So weit im Voraus wird geplant
const LOOKAHEAD: Duration = Duration::hours(6);
/// Verpasste Erinnerungen bis so weit zurück
const MISSED: Duration = Duration::hours(24);
/// Erinnerungen zu vergangenen Terminen nur, wenn sie so frisch sind
const GRACE: Duration = Duration::minutes(15);

/// Vorgaben im Dialog (Minuten vor Beginn), wie bei Calino
pub const TIMED: [i64; 10] = [0, 5, 10, 15, 30, 60, 120, 1440, 2880, 10080];
/// Ganztägig: am Tag um 9 Uhr, am Vortag, zwei Tage und eine Woche vorher
pub const ALL_DAY: [i64; 4] = [-540, 900, 2340, 9540];

/// Eine geplante Erinnerung
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    pub key: String,
    pub at: DateTime<Utc>,
    pub calendar_id: i64,
    pub title: String,
    pub location: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
    /// Erinnerung an eine Aufgabe (`start` = Fälligkeit)
    pub task: bool,
}

impl Planned {
    /// Kennung des Toasts (Windows erlaubt höchstens 64 Zeichen)
    pub fn tag(&self) -> String {
        let mut hasher = DefaultHasher::new();
        self.key.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// Noch zeigen? Vorbei sind nur frische Erinnerungen interessant.
    pub fn relevant(&self, now: DateTime<Utc>) -> bool {
        self.at <= now && (self.end > now || self.at > now - GRACE)
    }
}

#[derive(Debug, Default)]
pub struct Planner {
    upcoming: Vec<Planned>,
    planned_until: Option<DateTime<Utc>>,
    /// Termine haben sich geändert: neu planen
    pub dirty: bool,
}

impl Planner {
    /// Fällige Erinnerungen herausnehmen; `plan` liefert die Erinnerungen
    /// eines Zeitraums, wenn neu geplant werden muss.
    pub fn take_due(
        &mut self,
        now: DateTime<Utc>,
        plan: impl FnOnce(DateTime<Utc>, DateTime<Utc>) -> Vec<Planned>,
    ) -> Vec<Planned> {
        if self.dirty
            || self
                .planned_until
                .is_none_or(|until| until < now + Duration::minutes(10))
        {
            self.upcoming = plan(now - MISSED, now + LOOKAHEAD);
            self.planned_until = Some(now + LOOKAHEAD);
            self.dirty = false;
        }
        let (due, later): (Vec<Planned>, Vec<Planned>) = std::mem::take(&mut self.upcoming)
            .into_iter()
            .partition(|p| p.at <= now);
        self.upcoming = later;
        due.into_iter().filter(|p| p.relevant(now)).collect()
    }
}

/// Erinnerungen aller sichtbaren Kalender, die in `[from, to)` auslösen
pub fn plan(state: &mut State, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Planned> {
    let home = state.home;
    let resources = state
        .store
        .events_in_range(
            (from - MAX_AFTER).timestamp(),
            (to + MAX_BEFORE).timestamp(),
        )
        .unwrap_or_default();
    let mut out = Vec::new();
    for resource in resources {
        let events = crate::items::parsed_events(
            &mut state.parsed,
            resource.calendar_id,
            &resource.href,
            &resource.ics,
        );
        for due in reminders_between(events, home, from, to) {
            let o = due.occurrence;
            let instance = o
                .instance
                .map_or("-".to_string(), |i| i.timestamp().to_string());
            out.push(Planned {
                key: format!(
                    "{}|{}|{}|{instance}|{}|{}",
                    resource.calendar_id,
                    resource.href,
                    o.uid,
                    due.alarm,
                    due.at.timestamp()
                ),
                at: due.at,
                calendar_id: resource.calendar_id,
                title: if o.summary.trim().is_empty() {
                    "(ohne Titel)".into()
                } else {
                    o.summary.clone()
                },
                location: o.location.clone(),
                start: o.start,
                end: o.end,
                all_day: o.dates.is_some(),
                task: false,
            });
        }
    }
    // Aufgaben: eigene Erinnerungen oder zur Fälligkeit
    let at_due = state.settings.task_reminders;
    for resource in state.store.tasks().unwrap_or_default() {
        let Ok(object) = hora_core::model::CalendarObject::parse(&resource.ics) else {
            continue;
        };
        for r in task_reminders(&object.tasks(), home, from, to, at_due) {
            let instance = r
                .instance
                .map_or("-".to_string(), |i| i.timestamp().to_string());
            let alarm = r.alarm.map_or("f".to_string(), |a| a.to_string());
            out.push(Planned {
                key: format!(
                    "t|{}|{}|{}|{instance}|{alarm}|{}",
                    resource.calendar_id,
                    resource.href,
                    r.uid,
                    r.at.timestamp()
                ),
                at: r.at,
                calendar_id: resource.calendar_id,
                title: if r.summary.trim().is_empty() {
                    "(ohne Titel)".into()
                } else {
                    r.summary.clone()
                },
                location: String::new(),
                start: r.due,
                // Verpasst gilt sie noch einen Tag lang
                end: r.due + Duration::days(1),
                all_day: r.date_only,
                task: true,
            });
        }
    }
    out.sort_by_key(|p| p.at);
    out
}

/// "Fällig in 15 Minuten · 18:00", "Fällig heute", "Überfällig seit 5 Min. · 18:00"
fn task_text(p: &Planned, now: DateTime<Utc>, home: Tz) -> String {
    let local = p.start.with_timezone(&home).naive_local();
    let today = now.with_timezone(&home).date_naive();
    let day = match (local.date() - today).num_days() {
        -1 => "gestern".to_string(),
        0 => "heute".to_string(),
        1 => "morgen".to_string(),
        _ => format!("am {}", short_day(local.date())),
    };
    if p.all_day {
        return format!("Fällig {day}");
    }
    let time = format_time(local.time());
    let minutes = ((p.start - now).num_seconds() as f64 / 60.0).round() as i64;
    match minutes {
        m if m < 0 => format!("Überfällig seit {} Min. · {time}", -m),
        0 => format!("Jetzt fällig · {time}"),
        1 => format!("Fällig in 1 Minute · {time}"),
        m if m < 120 => format!("Fällig in {m} Minuten · {time}"),
        _ => format!("Fällig {day} um {time}"),
    }
}

/// "In 15 Minuten · 15:00–16:00", "Morgen · ganztägig", "Läuft seit 5 Min. · …"
pub fn when_text(p: &Planned, now: DateTime<Utc>, home: Tz) -> String {
    if p.task {
        return task_text(p, now, home);
    }
    let local = |t: DateTime<Utc>| t.with_timezone(&home).naive_local();
    let (start, end, today) = (local(p.start), local(p.end), local(now).date());
    let days = (start.date() - today).num_days();
    let day = match days {
        0 => "Heute".to_string(),
        1 => "Morgen".to_string(),
        2 => "Übermorgen".to_string(),
        _ => short_day(start.date()),
    };
    if p.all_day {
        let started = p.start <= now;
        return format!(
            "{} · ganztägig",
            if started { "Heute".to_string() } else { day }
        );
    }
    let range = if end.date() == start.date() {
        format!("{}–{}", format_time(start.time()), format_time(end.time()))
    } else {
        format!(
            "{} – {}, {}",
            format_time(start.time()),
            short_day(end.date()),
            format_time(end.time())
        )
    };
    let minutes = ((p.start - now).num_seconds() as f64 / 60.0).round() as i64;
    let when = match minutes {
        m if m < 0 => format!("Läuft seit {} Min.", -m),
        0 => "Jetzt".to_string(),
        1 => "In 1 Minute".to_string(),
        m if m < 120 => format!("In {m} Minuten"),
        _ => day,
    };
    format!("{when} · {range}")
}

/// Toast für eine Erinnerung
pub fn toast_for(p: &Planned, now: DateTime<Utc>, home: Tz) -> Toast {
    let mut lines = vec![when_text(p, now, home)];
    if !p.location.trim().is_empty() {
        lines.push(p.location.trim().to_string());
    }
    Toast {
        title: p.title.clone(),
        lines,
        tag: p.tag(),
        launch: format!(
            "{}|{}|{}",
            if p.task { "task" } else { "show" },
            p.calendar_id,
            p.start.timestamp()
        ),
        reminder: true,
    }
}

fn plural(n: i64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Beschriftung einer Vorgabe: "15 Minuten vorher", "Am Vortag um 09:00"
pub fn label(minutes: i64, all_day: bool) -> String {
    if all_day {
        let offset = -minutes;
        let (days, time) = (offset.div_euclid(1440), offset.rem_euclid(1440));
        let at = NaiveTime::from_hms_opt((time / 60) as u32, (time % 60) as u32, 0)
            .map_or_else(String::new, format_time);
        return match days {
            0 => format!("Am Tag um {at}"),
            -1 => format!("Am Vortag um {at}"),
            -7 => format!("1 Woche vorher um {at}"),
            d if d < 0 => format!("{} vorher um {at}", plural(-d, "Tag", "Tage")),
            d => format!("{} danach um {at}", plural(d, "Tag", "Tage")),
        };
    }
    let (m, suffix) = if minutes >= 0 {
        (minutes, "vorher")
    } else {
        (-minutes, "danach")
    };
    if m == 0 {
        return "Zum Zeitpunkt des Termins".into();
    }
    let amount = if m % 10080 == 0 {
        plural(m / 10080, "Woche", "Wochen")
    } else if m % 1440 == 0 {
        plural(m / 1440, "Tag", "Tage")
    } else if m % 60 == 0 {
        plural(m / 60, "Stunde", "Stunden")
    } else {
        plural(m, "Minute", "Minuten")
    };
    format!("{amount} {suffix}")
}

/// Beschriftung eines Alarms, den der Dialog nur anzeigt und behält
pub fn alarm_label(alarm: &Alarm, all_day: bool, home: Tz) -> String {
    match alarm.trigger {
        Trigger::Absolute(at) => {
            let local = at.with_timezone(&home).naive_local();
            format!(
                "Am {} um {}",
                format_date(local.date()),
                format_time(local.time().with_second(0).unwrap_or_default())
            )
        }
        Trigger::Relative {
            offset,
            from_end: true,
        } => match -offset.num_minutes() {
            0 => "Zum Ende des Termins".into(),
            m if m > 0 => format!("{} vor Ende", label(m, false).trim_end_matches(" vorher")),
            m => format!("{} nach Ende", label(-m, false).trim_end_matches(" vorher")),
        },
        Trigger::Relative { .. } => alarm
            .minutes_before()
            .map_or_else(|| "Erinnerung".into(), |m| label(m, all_day)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDateTime, TimeZone};
    use chrono_tz::Europe::Berlin;

    fn at(s: &str) -> DateTime<Utc> {
        Berlin
            .from_local_datetime(&NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap())
            .unwrap()
            .with_timezone(&Utc)
    }

    fn planned(key: &str, at_: &str, start: &str, end: &str) -> Planned {
        Planned {
            key: key.into(),
            at: at(at_),
            calendar_id: 1,
            title: "Zahnarzt".into(),
            location: "Praxis".into(),
            start: at(start),
            end: at(end),
            all_day: false,
            task: false,
        }
    }

    #[test]
    fn task_toasts_say_when_it_is_due() {
        let mut p = planned(
            "t",
            "2026-11-10 17:45",
            "2026-11-10 18:00",
            "2026-11-11 18:00",
        );
        p.task = true;
        assert_eq!(
            when_text(&p, at("2026-11-10 17:45"), Berlin),
            "Fällig in 15 Minuten · 18:00"
        );
        assert_eq!(
            when_text(&p, at("2026-11-10 18:10"), Berlin),
            "Überfällig seit 10 Min. · 18:00"
        );
        assert_eq!(
            when_text(&p, at("2026-11-09 18:00"), Berlin),
            "Fällig morgen um 18:00"
        );
        p.all_day = true;
        assert_eq!(
            when_text(&p, at("2026-11-10 09:00"), Berlin),
            "Fällig heute"
        );
        assert!(
            toast_for(&p, at("2026-11-10 09:00"), Berlin)
                .launch
                .starts_with("task|")
        );
    }

    #[test]
    fn labels_follow_calino() {
        assert_eq!(label(0, false), "Zum Zeitpunkt des Termins");
        assert_eq!(label(1, false), "1 Minute vorher");
        assert_eq!(label(15, false), "15 Minuten vorher");
        assert_eq!(label(60, false), "1 Stunde vorher");
        assert_eq!(label(90, false), "90 Minuten vorher");
        assert_eq!(label(2880, false), "2 Tage vorher");
        assert_eq!(label(10080, false), "1 Woche vorher");
        assert_eq!(label(-540, true), "Am Tag um 09:00");
        assert_eq!(label(900, true), "Am Vortag um 09:00");
        assert_eq!(label(2340, true), "2 Tage vorher um 09:00");
        assert_eq!(label(9540, true), "1 Woche vorher um 09:00");
    }

    #[test]
    fn toast_text_says_when() {
        let now = at("2026-11-10 14:45");
        let p = planned(
            "a",
            "2026-11-10 14:45",
            "2026-11-10 15:00",
            "2026-11-10 16:00",
        );
        assert_eq!(when_text(&p, now, Berlin), "In 15 Minuten · 15:00–16:00");
        assert_eq!(
            when_text(&p, at("2026-11-10 15:05"), Berlin),
            "Läuft seit 5 Min. · 15:00–16:00"
        );
        assert_eq!(
            when_text(&p, at("2026-11-10 15:00"), Berlin),
            "Jetzt · 15:00–16:00"
        );
        assert_eq!(
            when_text(&p, at("2026-11-09 15:00"), Berlin),
            "Morgen · 15:00–16:00"
        );
        let mut whole = planned(
            "b",
            "2026-11-09 09:00",
            "2026-11-10 00:00",
            "2026-11-11 00:00",
        );
        whole.all_day = true;
        assert_eq!(
            when_text(&whole, at("2026-11-09 09:00"), Berlin),
            "Morgen · ganztägig"
        );
        let toast = toast_for(&p, now, Berlin);
        assert_eq!(
            toast.lines,
            vec![
                "In 15 Minuten · 15:00–16:00".to_string(),
                "Praxis".to_string()
            ]
        );
        assert!(toast.reminder && toast.tag.len() == 16);
    }

    #[test]
    fn planner_hands_out_due_reminders_once_and_skips_old_ones() {
        let mut planner = Planner::default();
        let list = vec![
            // Termin längst vorbei, Erinnerung alt: nicht mehr zeigen
            planned(
                "alt",
                "2026-11-10 08:00",
                "2026-11-10 08:15",
                "2026-11-10 09:00",
            ),
            // Verpasst, Termin läuft noch: zeigen
            planned(
                "verpasst",
                "2026-11-10 13:45",
                "2026-11-10 14:00",
                "2026-11-10 15:00",
            ),
            planned(
                "gleich",
                "2026-11-10 14:45",
                "2026-11-10 15:00",
                "2026-11-10 16:00",
            ),
            planned(
                "später",
                "2026-11-10 16:45",
                "2026-11-10 17:00",
                "2026-11-10 18:00",
            ),
        ];
        let plans = std::cell::Cell::new(0);
        let take = |planner: &mut Planner, now: &str| {
            planner
                .take_due(at(now), |_, _| {
                    plans.set(plans.get() + 1);
                    list.clone()
                })
                .into_iter()
                .map(|p| p.key)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            take(&mut planner, "2026-11-10 14:45"),
            vec!["verpasst", "gleich"]
        );
        assert!(take(&mut planner, "2026-11-10 14:50").is_empty());
        assert_eq!(take(&mut planner, "2026-11-10 16:45"), vec!["später"]);
        assert_eq!(plans.get(), 1, "geplant wird nur einmal");
        planner.dirty = true;
        take(&mut planner, "2026-11-10 16:46");
        assert_eq!(plans.get(), 2);
    }
}
