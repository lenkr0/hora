//! Vorkommen von Terminen in einem Zeitraum: Einzeltermine, Serien mit RRULE
//! und RDATE, ausgelassene Termine (EXDATE) und geänderte Einzeltermine
//! (RECURRENCE-ID). Serien werden in der Zeitzone ihres Beginns berechnet,
//! damit "jeden Montag 8 Uhr" auch nach der Zeitumstellung 8 Uhr bleibt.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use rrule::{RRule, RRuleSet, Unvalidated};

use crate::model::EventInfo;
use crate::time::{DateTimeValue, local_to_utc};

/// Obergrenze je Serie und Abfrage, schützt vor endlosen Regeln.
const MAX_OCCURRENCES: u16 = 2000;

#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub uid: String,
    pub summary: String,
    pub location: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Bei ganztägigen Terminen: erster Tag und Tag nach dem letzten
    pub dates: Option<(NaiveDate, NaiveDate)>,
    /// Ursprünglicher Beginn innerhalb einer Serie (Wert für RECURRENCE-ID)
    pub instance: Option<DateTime<Utc>>,
}

impl Occurrence {
    pub fn all_day(&self) -> bool {
        self.dates.is_some()
    }
}

/// Alle Vorkommen der Termine eines Kalenderobjekts, die `[from, to)`
/// berühren, nach Beginn sortiert. Ganztägige und schwebende Termine gelten
/// in `home`.
pub fn expand(
    events: &[EventInfo],
    home: Tz,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<Occurrence> {
    let overrides: Vec<&EventInfo> = events
        .iter()
        .filter(|e| e.recurrence_id.is_some())
        .collect();
    let mut out = Vec::new();

    for master in events
        .iter()
        .filter(|e| e.recurrence_id.is_none() && !e.cancelled)
    {
        let start = master.start.to_utc(home);
        let length = (master.end.to_utc(home) - start).max(Duration::zero());
        if !master.is_recurring() {
            push_if_visible(&mut out, master, home, start, length, None, from, to);
            continue;
        }
        let replaced: Vec<DateTime<Utc>> = overrides
            .iter()
            .filter(|o| o.uid == master.uid)
            .filter_map(|o| o.recurrence_id.map(|r| r.to_utc(home)))
            .collect();
        // Nicht lesbare Regel: wenigstens den ersten Termin zeigen
        let starts = occurrence_starts(master, home, from - length - Duration::seconds(1), to)
            .unwrap_or_else(|_| vec![start]);
        for s in starts.into_iter().filter(|s| !replaced.contains(s)) {
            push_if_visible(&mut out, master, home, s, length, Some(s), from, to);
        }
    }

    for o in overrides.into_iter().filter(|o| !o.cancelled) {
        let start = o.start.to_utc(home);
        let length = (o.end.to_utc(home) - start).max(Duration::zero());
        let instance = o.recurrence_id.map(|r| r.to_utc(home));
        push_if_visible(&mut out, o, home, start, length, instance, from, to);
    }

    out.sort_by_key(|o| (o.start, o.end));
    out
}

#[allow(clippy::too_many_arguments)]
fn push_if_visible(
    out: &mut Vec<Occurrence>,
    event: &EventInfo,
    home: Tz,
    start: DateTime<Utc>,
    length: Duration,
    instance: Option<DateTime<Utc>>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) {
    let dates = match (event.start, event.end) {
        (DateTimeValue::Date(first), DateTimeValue::Date(end)) => {
            let day = start.with_timezone(&home).date_naive();
            Some((day, day + Duration::days((end - first).num_days().max(1))))
        }
        (DateTimeValue::Date(_), _) => {
            let day = start.with_timezone(&home).date_naive();
            Some((day, day + Duration::days(1)))
        }
        _ => None,
    };
    // Ganztägige Termine enden um Mitternacht des Folgetags in `home`
    let end = match dates {
        Some((_, last)) => local_to_utc(last.and_time(chrono::NaiveTime::MIN), home),
        None => start + length,
    };
    let visible = if end > start {
        start < to && end > from
    } else {
        start >= from && start < to
    };
    if visible {
        out.push(Occurrence {
            uid: event.uid.clone(),
            summary: event.summary.clone(),
            location: event.location.clone(),
            start,
            end,
            dates,
            instance,
        });
    }
}

fn to_rrule(value: &DateTimeValue, home: Tz, zone: rrule::Tz) -> DateTime<rrule::Tz> {
    zone.from_utc_datetime(&value.to_utc(home).naive_utc())
}

/// UNTIL muss laut RFC 5545 in UTC stehen, wenn DTSTART eine Zeitzone hat.
/// Viele Clients schreiben trotzdem ein Datum oder eine lokale Zeit; die
/// Bibliothek lehnt das ab, deshalb wird UNTIL hier nach UTC umgerechnet.
fn normalize_until(rule: &str, zone: Tz) -> String {
    rule.split(';')
        .map(|part| {
            let Some(value) = part
                .strip_prefix("UNTIL=")
                .or_else(|| part.strip_prefix("until="))
            else {
                return part.to_string();
            };
            if value.ends_with(['Z', 'z']) {
                return part.to_string();
            }
            let local = if value.len() == 8 {
                NaiveDate::parse_from_str(value, "%Y%m%d")
                    .ok()
                    .and_then(|d| d.and_hms_opt(23, 59, 59))
            } else {
                NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()
            };
            match local {
                Some(local) => format!(
                    "UNTIL={}",
                    local_to_utc(local, zone).format("%Y%m%dT%H%M%SZ")
                ),
                None => part.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn occurrence_starts(
    master: &EventInfo,
    home: Tz,
    after: DateTime<Utc>,
    before: DateTime<Utc>,
) -> Result<Vec<DateTime<Utc>>, rrule::RRuleError> {
    let tz = master.start.zone(home);
    let zone: rrule::Tz = tz.into();
    let dt_start = to_rrule(&master.start, home, zone);
    let mut set = match &master.rrule {
        Some(rule) => normalize_until(rule, tz)
            .parse::<RRule<Unvalidated>>()?
            .build(dt_start)?,
        None => RRuleSet::new(dt_start),
    };
    // DTSTART ist laut RFC 5545 immer das erste Vorkommen, auch wenn es nicht
    // zur Regel passt; die Bibliothek erzeugt es nur aus der Regel.
    set = set.rdate(dt_start);
    for date in &master.rdates {
        set = set.rdate(to_rrule(date, home, zone));
    }
    for date in &master.exdates {
        set = set.exdate(to_rrule(date, home, zone));
    }
    let result = set
        .after(after.with_timezone(&zone))
        .before(before.with_timezone(&zone))
        .all(MAX_OCCURRENCES);
    let mut starts: Vec<DateTime<Utc>> = result
        .dates
        .into_iter()
        .map(|d| d.with_timezone(&Utc))
        .collect();
    starts.sort();
    starts.dedup();
    Ok(starts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CalendarObject;
    use chrono_tz::Europe::Berlin;

    fn utc(s: &str) -> DateTime<Utc> {
        Utc.from_utc_datetime(&NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap())
    }

    fn expand_ics(body: &str, from: &str, to: &str) -> Vec<Occurrence> {
        let ics = format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}END:VCALENDAR\r\n");
        let (events, errors) = CalendarObject::parse(&ics).unwrap().events();
        assert!(errors.is_empty(), "{errors:?}");
        expand(&events, Berlin, utc(from), utc(to))
    }

    fn starts(occ: &[Occurrence]) -> Vec<DateTime<Utc>> {
        occ.iter().map(|o| o.start).collect()
    }

    #[test]
    fn weekly_series_keeps_local_time_across_dst() {
        // Jeden Montag 8:00 Berlin: vor dem 29.03. UTC+1, danach UTC+2
        let occ = expand_ics(
            "BEGIN:VEVENT\r\nUID:m\r\nDTSTART;TZID=Europe/Berlin:20260316T080000\r\nDTEND;TZID=Europe/Berlin:20260316T093000\r\nRRULE:FREQ=WEEKLY;BYDAY=MO\r\nEND:VEVENT\r\n",
            "2026-03-01 00:00",
            "2026-04-07 00:00",
        );
        assert_eq!(
            starts(&occ),
            vec![
                utc("2026-03-16 07:00"),
                utc("2026-03-23 07:00"),
                utc("2026-03-30 06:00"),
                utc("2026-04-06 06:00")
            ]
        );
        assert!(occ.iter().all(|o| o.end - o.start == Duration::minutes(90)));
    }

    #[test]
    fn weekly_series_across_end_of_dst() {
        let occ = expand_ics(
            "BEGIN:VEVENT\r\nUID:m\r\nDTSTART;TZID=Europe/Berlin:20261019T080000\r\nDTEND;TZID=Europe/Berlin:20261019T090000\r\nRRULE:FREQ=WEEKLY;COUNT=2\r\nEND:VEVENT\r\n",
            "2026-10-01 00:00",
            "2026-11-30 00:00",
        );
        assert_eq!(
            starts(&occ),
            vec![utc("2026-10-19 06:00"), utc("2026-10-26 07:00")]
        );
    }

    #[test]
    fn exdate_removes_an_occurrence() {
        let occ = expand_ics(
            "BEGIN:VEVENT\r\nUID:x\r\nDTSTART;TZID=Europe/Berlin:20261102T080000\r\nDTEND;TZID=Europe/Berlin:20261102T090000\r\nRRULE:FREQ=WEEKLY;COUNT=4\r\nEXDATE;TZID=Europe/Berlin:20261109T080000\r\nEND:VEVENT\r\n",
            "2026-11-01 00:00",
            "2026-12-31 00:00",
        );
        assert_eq!(
            starts(&occ),
            vec![
                utc("2026-11-02 07:00"),
                utc("2026-11-16 07:00"),
                utc("2026-11-23 07:00")
            ]
        );
    }

    #[test]
    fn recurrence_id_replaces_one_occurrence() {
        let occ = expand_ics(
            "BEGIN:VEVENT\r\nUID:r\r\nDTSTART;TZID=Europe/Berlin:20261102T080000\r\nDTEND;TZID=Europe/Berlin:20261102T090000\r\nSUMMARY:Mathe\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:r\r\nRECURRENCE-ID;TZID=Europe/Berlin:20261109T080000\r\nDTSTART;TZID=Europe/Berlin:20261110T100000\r\nDTEND;TZID=Europe/Berlin:20261110T110000\r\nSUMMARY:Mathe (verlegt)\r\nEND:VEVENT\r\n",
            "2026-11-01 00:00",
            "2026-12-01 00:00",
        );
        assert_eq!(
            starts(&occ),
            vec![
                utc("2026-11-02 07:00"),
                utc("2026-11-10 09:00"),
                utc("2026-11-16 07:00")
            ]
        );
        assert_eq!(occ[1].summary, "Mathe (verlegt)");
        assert_eq!(occ[1].instance, Some(utc("2026-11-09 07:00")));
    }

    #[test]
    fn cancelled_override_hides_occurrence() {
        let occ = expand_ics(
            "BEGIN:VEVENT\r\nUID:c\r\nDTSTART:20261102T070000Z\r\nDTEND:20261102T080000Z\r\nRRULE:FREQ=DAILY;COUNT=3\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:c\r\nRECURRENCE-ID:20261103T070000Z\r\nDTSTART:20261103T070000Z\r\nDTEND:20261103T080000Z\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n",
            "2026-11-01 00:00",
            "2026-11-30 00:00",
        );
        assert_eq!(
            starts(&occ),
            vec![utc("2026-11-02 07:00"), utc("2026-11-04 07:00")]
        );
    }

    #[test]
    fn yearly_all_day_birthday() {
        let occ = expand_ics(
            "BEGIN:VEVENT\r\nUID:b\r\nDTSTART;VALUE=DATE:19900315\r\nDTEND;VALUE=DATE:19900316\r\nRRULE:FREQ=YEARLY\r\nSUMMARY:Geburtstag\r\nEND:VEVENT\r\n",
            "2026-03-09 00:00",
            "2026-03-16 00:00",
        );
        assert_eq!(occ.len(), 1);
        let d = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        assert_eq!(occ[0].dates, Some((d, d.succ_opt().unwrap())));
    }

    #[test]
    fn multi_day_all_day_event_overlapping_range_start() {
        let occ = expand_ics(
            "BEGIN:VEVENT\r\nUID:u\r\nDTSTART;VALUE=DATE:20261030\r\nDTEND;VALUE=DATE:20261104\r\nSUMMARY:Urlaub\r\nEND:VEVENT\r\n",
            "2026-11-01 23:00",
            "2026-11-08 23:00",
        );
        assert_eq!(occ.len(), 1);
        assert_eq!(
            occ[0].dates,
            Some((
                NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(),
                NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
            ))
        );
    }

    #[test]
    fn until_as_local_date_is_accepted() {
        // UNTIL ohne Z bei DTSTART mit Zeitzone (häufig, aber nicht RFC-konform)
        let occ = expand_ics(
            "BEGIN:VEVENT\r\nUID:until\r\nDTSTART;TZID=Europe/Berlin:20261102T080000\r\nRRULE:FREQ=DAILY;UNTIL=20261104\r\nEND:VEVENT\r\n",
            "2026-11-01 00:00",
            "2026-11-30 00:00",
        );
        assert_eq!(occ.len(), 3);
    }

    #[test]
    fn floating_time_uses_home_zone() {
        let occ = expand_ics(
            "BEGIN:VEVENT\r\nUID:f\r\nDTSTART:20261109T080000\r\nDTEND:20261109T090000\r\nEND:VEVENT\r\n",
            "2026-11-09 00:00",
            "2026-11-10 00:00",
        );
        assert_eq!(starts(&occ), vec![utc("2026-11-09 07:00")]);
    }

    #[test]
    fn event_starting_before_range_but_ending_inside_is_included() {
        let occ = expand_ics(
            "BEGIN:VEVENT\r\nUID:n\r\nDTSTART:20261108T220000Z\r\nDTEND:20261109T020000Z\r\nEND:VEVENT\r\n",
            "2026-11-09 00:00",
            "2026-11-10 00:00",
        );
        assert_eq!(occ.len(), 1);
    }

    #[test]
    fn rdate_adds_extra_occurrence() {
        let occ = expand_ics(
            "BEGIN:VEVENT\r\nUID:rd\r\nDTSTART:20261102T070000Z\r\nDTEND:20261102T080000Z\r\nRDATE:20261105T070000Z\r\nEND:VEVENT\r\n",
            "2026-11-01 00:00",
            "2026-11-30 00:00",
        );
        assert_eq!(
            starts(&occ),
            vec![utc("2026-11-02 07:00"), utc("2026-11-05 07:00")]
        );
    }

    #[test]
    fn unbounded_daily_series_far_from_start_is_cheap_and_correct() {
        let occ = expand_ics(
            "BEGIN:VEVENT\r\nUID:d\r\nDTSTART;TZID=Europe/Berlin:20000101T120000\r\nDTEND;TZID=Europe/Berlin:20000101T121500\r\nRRULE:FREQ=DAILY\r\nEND:VEVENT\r\n",
            "2026-11-02 00:00",
            "2026-11-09 00:00",
        );
        assert_eq!(occ.len(), 7);
        assert_eq!(occ[0].start, utc("2026-11-02 11:00"));
    }
}
