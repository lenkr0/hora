//! Verteilt Termine auf die Tagesspalten einer Ansicht (1, 3 oder 7 Tage),
//! in der eigenen Zeitzone und mit Wandzeit.
//!
//! - Termine bis unter 24 Stunden erscheinen im Raster; über Mitternacht
//!   werden sie in je einen Teil pro Tag geteilt.
//! - Ganztägige Termine und Termine ab 24 Stunden erscheinen in der
//!   Ganztagszeile.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc};
use hora_core::Tz;
use hora_core::expand::Occurrence;

/// Zeitraum eines Termins: Zeitpunkte in UTC, bei ganztägigen Terminen dazu
/// erster Tag und Tag nach dem letzten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub dates: Option<(NaiveDate, NaiveDate)>,
}

impl From<&Occurrence> for Span {
    fn from(o: &Occurrence) -> Self {
        Span {
            start: o.start,
            end: o.end,
            dates: o.dates,
        }
    }
}

impl Span {
    /// Erster und letzter Tag (einschließlich) in `home`.
    pub fn days(&self, home: Tz) -> (NaiveDate, NaiveDate) {
        if let Some((first, end)) = self.dates {
            return (first, (end - Duration::days(1)).max(first));
        }
        let start = self.start.with_timezone(&home).naive_local();
        let end = self.end.with_timezone(&home).naive_local();
        let last = if end > start {
            (end - Duration::seconds(1)).date()
        } else {
            start.date()
        };
        (start.date(), last.max(start.date()))
    }

    /// Gehört in die Ganztagszeile (ganztägig oder mindestens 24 Stunden)?
    pub fn is_all_day_like(&self) -> bool {
        self.dates.is_some() || self.end - self.start >= Duration::hours(24)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// Je Tag: (Spalte, Beginn in Minuten, Dauer in Minuten)
    Timed(Vec<(i32, i32, i32)>),
    /// Erste Spalte und Spalte nach der letzten, auf die Ansicht begrenzt
    AllDay(i32, i32),
    Outside,
}

fn minutes(t: NaiveDateTime) -> i32 {
    (t.hour() * 60 + t.minute()) as i32
}

/// Lage eines Termins in einer Ansicht mit `days` Spalten ab `first_day`.
pub fn place(span: &Span, first_day: NaiveDate, days: i32, home: Tz) -> Placement {
    let day_index = |d: NaiveDate| (d - first_day).num_days() as i32;
    let clip = |first: i32, end: i32| {
        if end <= 0 || first >= days {
            Placement::Outside
        } else {
            Placement::AllDay(first.max(0), end.min(days))
        }
    };
    if span.is_all_day_like() {
        let (first, last) = span.days(home);
        return clip(day_index(first), day_index(last) + 1);
    }

    let start = span.start.with_timezone(&home).naive_local();
    let end = span.end.with_timezone(&home).naive_local().max(start);
    let mut parts = Vec::new();
    let mut day = start.date();
    loop {
        let index = day_index(day);
        let day_start = day.and_time(NaiveTime::MIN);
        let next = day_start + Duration::days(1);
        let (from, to) = (start.max(day_start), end.min(next));
        if (0..days).contains(&index) && (to > from || start == end) {
            parts.push((index, minutes(from), (to - from).num_minutes() as i32));
        }
        if end <= next {
            break;
        }
        day = next.date();
    }
    if parts.is_empty() {
        Placement::Outside
    } else {
        Placement::Timed(parts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::Europe::Berlin;

    /// Montag, 2. November 2026
    fn monday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 2).unwrap()
    }

    fn date(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn at(s: &str) -> DateTime<Utc> {
        Berlin
            .from_local_datetime(&NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap())
            .unwrap()
            .with_timezone(&Utc)
    }

    fn timed(start: &str, end: &str) -> Span {
        Span {
            start: at(start),
            end: at(end),
            dates: None,
        }
    }

    fn all_day(first: (u32, u32), end: (u32, u32)) -> Span {
        Span {
            dates: Some((date(first.0, first.1), date(end.0, end.1))),
            ..timed("2026-11-02 00:00", "2026-11-02 00:00")
        }
    }

    fn week(span: Span) -> Placement {
        place(&span, monday(), 7, Berlin)
    }

    #[test]
    fn same_day_event() {
        assert_eq!(
            week(timed("2026-11-02 09:00", "2026-11-02 10:30")),
            Placement::Timed(vec![(0, 540, 90)])
        );
    }

    #[test]
    fn event_across_midnight_is_split() {
        assert_eq!(
            week(timed("2026-11-03 22:00", "2026-11-04 02:00")),
            Placement::Timed(vec![(1, 1320, 120), (2, 0, 120)])
        );
        // Sonntagnacht: der Teil am Montag gehört zur nächsten Woche
        assert_eq!(
            week(timed("2026-11-08 22:00", "2026-11-09 02:00")),
            Placement::Timed(vec![(6, 1320, 120)])
        );
    }

    #[test]
    fn event_ending_at_midnight_stays_on_its_day() {
        assert_eq!(
            week(timed("2026-11-05 23:00", "2026-11-06 00:00")),
            Placement::Timed(vec![(3, 1380, 60)])
        );
    }

    #[test]
    fn long_timed_events_go_to_the_all_day_row() {
        assert_eq!(
            week(timed("2026-11-03 18:00", "2026-11-05 10:00")),
            Placement::AllDay(1, 4)
        );
    }

    #[test]
    fn all_day_events_are_clipped_to_the_view() {
        assert_eq!(week(all_day((10, 30), (11, 4))), Placement::AllDay(0, 2));
        assert_eq!(week(all_day((11, 6), (11, 12))), Placement::AllDay(4, 7));
        assert_eq!(week(all_day((11, 9), (11, 10))), Placement::Outside);
    }

    #[test]
    fn zero_length_event_is_shown_at_its_time() {
        assert_eq!(
            week(timed("2026-11-04 12:00", "2026-11-04 12:00")),
            Placement::Timed(vec![(2, 720, 0)])
        );
    }

    #[test]
    fn times_are_shown_in_the_home_zone() {
        // 15:00 UTC = 16:00 in Berlin (Winterzeit)
        let span = Span {
            start: Utc.with_ymd_and_hms(2026, 11, 2, 15, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 11, 2, 16, 0, 0).unwrap(),
            dates: None,
        };
        assert_eq!(week(span), Placement::Timed(vec![(0, 960, 60)]));
    }

    #[test]
    fn day_and_three_day_views_clip_to_their_columns() {
        let tuesday = timed("2026-11-03 09:00", "2026-11-03 10:00");
        assert_eq!(place(&tuesday, date(11, 4), 3, Berlin), Placement::Outside);
        assert_eq!(
            place(&tuesday, date(11, 3), 1, Berlin),
            Placement::Timed(vec![(0, 540, 60)])
        );
        // Mehrtägig in der Tagesansicht: genau eine Spalte
        assert_eq!(
            place(&all_day((11, 2), (11, 7)), date(11, 4), 1, Berlin),
            Placement::AllDay(0, 1)
        );
    }

    #[test]
    fn covered_days() {
        assert_eq!(
            all_day((11, 4), (11, 7)).days(Berlin),
            (date(11, 4), date(11, 6))
        );
        assert_eq!(
            timed("2026-11-04 22:00", "2026-11-05 06:00").days(Berlin).1,
            date(11, 5)
        );
        assert_eq!(
            timed("2026-11-04 22:00", "2026-11-05 00:00").days(Berlin).1,
            date(11, 4)
        );
    }
}
