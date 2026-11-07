//! Datum und Uhrzeit auf Deutsch: Eingaben im Termin-Dialog lesen und
//! Zeiträume für Vorschau und Titel beschreiben.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use hora_core::Tz;
use hora_core::time::{DateTimeValue, local_to_utc};

pub const WEEKDAYS: [&str; 7] = [
    "Montag",
    "Dienstag",
    "Mittwoch",
    "Donnerstag",
    "Freitag",
    "Samstag",
    "Sonntag",
];
pub const WEEKDAYS_SHORT: [&str; 7] = ["Mo.", "Di.", "Mi.", "Do.", "Fr.", "Sa.", "So."];
pub const MONTHS: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];
pub const MONTHS_SHORT: [&str; 12] = [
    "Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sep.", "Okt.", "Nov.", "Dez.",
];

/// "4.11.2026", "04.11.26", "4.11." / "4.11" (Jahr von `today`), "2026-11-04",
/// "heute", "morgen", "übermorgen".
pub fn parse_date(input: &str, today: NaiveDate) -> Option<NaiveDate> {
    let s = input.trim().to_lowercase();
    match s.as_str() {
        "heute" => return Some(today),
        "morgen" => return today.succ_opt(),
        "übermorgen" | "uebermorgen" => return today.checked_add_signed(Duration::days(2)),
        _ => {}
    }
    if let Ok(d) = NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
        return Some(d);
    }
    let parts: Vec<&str> = s.trim_end_matches('.').split('.').map(str::trim).collect();
    let day: u32 = parts.first()?.parse().ok()?;
    let month: u32 = parts.get(1)?.parse().ok()?;
    let year: i32 = match parts.get(2) {
        None | Some(&"") => today.year(),
        Some(y) if y.len() <= 2 => 2000 + y.parse::<i32>().ok()?,
        Some(y) => y.parse().ok()?,
    };
    if parts.len() > 3 {
        return None;
    }
    NaiveDate::from_ymd_opt(year, month, day)
}

/// "9", "09", "930", "0930", "9:30", "9.30", "21:05", "9 Uhr", "9:30 Uhr".
pub fn parse_time(input: &str) -> Option<NaiveTime> {
    let s = input.trim().to_lowercase();
    let s = s.strip_suffix("uhr").unwrap_or(&s).trim();
    let (hour, minute) = if let Some((h, m)) = s.split_once([':', '.']) {
        (h.trim().parse::<u32>().ok()?, m.trim().parse::<u32>().ok()?)
    } else if s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() {
        match s.len() {
            1 | 2 => (s.parse().ok()?, 0),
            3 | 4 => {
                let (h, m) = s.split_at(s.len() - 2);
                (h.parse().ok()?, m.parse().ok()?)
            }
            _ => return None,
        }
    } else {
        return None;
    };
    NaiveTime::from_hms_opt(hour, minute, 0)
}

pub fn format_date(d: NaiveDate) -> String {
    d.format("%d.%m.%Y").to_string()
}

pub fn format_time(t: NaiveTime) -> String {
    format!("{:02}:{:02}", t.hour(), t.minute())
}

/// "Mi., 4. Nov."
pub fn short_day(d: NaiveDate) -> String {
    format!(
        "{} {}. {}",
        WEEKDAYS_SHORT[d.weekday().num_days_from_monday() as usize],
        d.day(),
        MONTHS_SHORT[d.month0() as usize]
    )
}

/// "Mittwoch, 4. November 2026"
pub fn long_day(d: NaiveDate) -> String {
    format!(
        "{}, {}. {} {}",
        WEEKDAYS[d.weekday().num_days_from_monday() as usize],
        d.day(),
        MONTHS[d.month0() as usize],
        d.year()
    )
}

/// Zeitraum für die Vorschau. `end` ist bei ganztägigen Terminen der Tag nach
/// dem letzten.
pub fn describe(start: NaiveDateTime, end: NaiveDateTime, all_day: bool) -> String {
    if all_day {
        let last = (end.date() - Duration::days(1)).max(start.date());
        return if last == start.date() {
            format!("{} · ganztägig", short_day(start.date()))
        } else {
            format!(
                "{} – {} · ganztägig",
                short_day(start.date()),
                short_day(last)
            )
        };
    }
    if start.date() == end.date()
        || (end.time() == NaiveTime::MIN
            && end.date() == start.date().succ_opt().unwrap_or(end.date()))
    {
        let end_text = if end.time() == NaiveTime::MIN && end.date() != start.date() {
            "24:00".to_string()
        } else {
            format_time(end.time())
        };
        format!(
            "{} · {}–{}",
            short_day(start.date()),
            format_time(start.time()),
            end_text
        )
    } else {
        format!(
            "{}, {} – {}, {}",
            short_day(start.date()),
            format_time(start.time()),
            short_day(end.date()),
            format_time(end.time())
        )
    }
}

/// Felder des Termin-Dialogs als Text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftTimes {
    pub all_day: bool,
    pub start_date: String,
    pub start_time: String,
    pub end_date: String,
    pub end_time: String,
}

impl DraftTimes {
    /// Für die Anzeige: Wandzeiten in `home`; bei ganztägig der letzte Tag
    /// (einschließlich), nicht der Tag danach.
    pub fn from_values(start: &DateTimeValue, end: &DateTimeValue, home: Tz) -> Self {
        if let (DateTimeValue::Date(first), DateTimeValue::Date(after)) = (start, end) {
            let last = (*after - Duration::days(1)).max(*first);
            return DraftTimes {
                all_day: true,
                start_date: format_date(*first),
                start_time: "09:00".into(),
                end_date: format_date(last),
                end_time: "10:00".into(),
            };
        }
        let s = start.to_utc(home).with_timezone(&home).naive_local();
        let e = end.to_utc(home).with_timezone(&home).naive_local();
        DraftTimes {
            all_day: false,
            start_date: format_date(s.date()),
            start_time: format_time(s.time()),
            end_date: format_date(e.date()),
            end_time: format_time(e.time()),
        }
    }

    /// Eingaben prüfen und in Werte umrechnen. Ein bestehender Termin
    /// (`original`) behält seine Art: ein Termin in New-York-Zeit bleibt in
    /// New-York-Zeit gespeichert. Neue Termine gelten in `home`.
    pub fn to_values(
        &self,
        today: NaiveDate,
        home: Tz,
        original: Option<&DateTimeValue>,
    ) -> Result<(DateTimeValue, DateTimeValue), String> {
        let start_date = parse_date(&self.start_date, today)
            .ok_or("Beginn: Datum nicht erkannt (z. B. 04.11.2026)")?;
        let end_date = if self.end_date.trim().is_empty() {
            start_date
        } else {
            parse_date(&self.end_date, today)
                .ok_or("Ende: Datum nicht erkannt (z. B. 04.11.2026)")?
        };
        if self.all_day {
            if end_date < start_date {
                return Err("Das Ende liegt vor dem Beginn.".into());
            }
            return Ok((
                DateTimeValue::Date(start_date),
                DateTimeValue::Date(end_date + Duration::days(1)),
            ));
        }
        let start_time =
            parse_time(&self.start_time).ok_or("Beginn: Uhrzeit nicht erkannt (z. B. 9:30)")?;
        let end_time =
            parse_time(&self.end_time).ok_or("Ende: Uhrzeit nicht erkannt (z. B. 10:30)")?;
        let start = start_date.and_time(start_time);
        let end = end_date.and_time(end_time);
        if end <= start {
            return Err("Das Ende muss nach dem Beginn liegen.".into());
        }
        let keep = original.filter(|v| !v.is_date());
        let value = |naive: NaiveDateTime| match keep {
            Some(original) => original.moved_to(local_to_utc(naive, home), home),
            None => DateTimeValue::Zoned(naive, home),
        };
        Ok((value(start), value(end)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Europe::Berlin;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn time(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn today() -> NaiveDate {
        date(2026, 11, 4)
    }

    #[test]
    fn dates_in_german_notation() {
        assert_eq!(parse_date("4.11.2026", today()), Some(date(2026, 11, 4)));
        assert_eq!(parse_date("04.11.26", today()), Some(date(2026, 11, 4)));
        assert_eq!(parse_date("24.12.", today()), Some(date(2026, 12, 24)));
        assert_eq!(parse_date("24.12", today()), Some(date(2026, 12, 24)));
        assert_eq!(parse_date(" 2026-11-04 ", today()), Some(date(2026, 11, 4)));
        assert_eq!(parse_date("morgen", today()), Some(date(2026, 11, 5)));
        assert_eq!(parse_date("Übermorgen", today()), Some(date(2026, 11, 6)));
        assert_eq!(parse_date("31.2.2026", today()), None);
        assert_eq!(parse_date("Dienstag", today()), None);
        assert_eq!(parse_date("1.2.3.4", today()), None);
    }

    #[test]
    fn times_in_many_spellings() {
        assert_eq!(parse_time("9"), Some(time(9, 0)));
        assert_eq!(parse_time("09"), Some(time(9, 0)));
        assert_eq!(parse_time("930"), Some(time(9, 30)));
        assert_eq!(parse_time("0930"), Some(time(9, 30)));
        assert_eq!(parse_time("9:30"), Some(time(9, 30)));
        assert_eq!(parse_time("9.30"), Some(time(9, 30)));
        assert_eq!(parse_time("21:05"), Some(time(21, 5)));
        assert_eq!(parse_time("9 Uhr"), Some(time(9, 0)));
        assert_eq!(parse_time("9:30 uhr"), Some(time(9, 30)));
        assert_eq!(parse_time("25:00"), None);
        assert_eq!(parse_time("9:61"), None);
        assert_eq!(parse_time("abends"), None);
        assert_eq!(parse_time(""), None);
    }

    #[test]
    fn descriptions() {
        let at = |d: NaiveDate, h, m| d.and_time(time(h, m));
        assert_eq!(
            describe(at(today(), 9, 0), at(today(), 10, 30), false),
            "Mi. 4. Nov. · 09:00–10:30"
        );
        assert_eq!(
            describe(at(today(), 23, 0), at(date(2026, 11, 5), 0, 0), false),
            "Mi. 4. Nov. · 23:00–24:00"
        );
        assert_eq!(
            describe(at(today(), 22, 0), at(date(2026, 11, 5), 6, 0), false),
            "Mi. 4. Nov., 22:00 – Do. 5. Nov., 06:00"
        );
        assert_eq!(
            describe(at(today(), 0, 0), at(date(2026, 11, 5), 0, 0), true),
            "Mi. 4. Nov. · ganztägig"
        );
        assert_eq!(
            describe(at(today(), 0, 0), at(date(2026, 11, 7), 0, 0), true),
            "Mi. 4. Nov. – Fr. 6. Nov. · ganztägig"
        );
        assert_eq!(long_day(today()), "Mittwoch, 4. November 2026");
    }

    #[test]
    fn draft_roundtrip_for_new_timed_event() {
        let draft = DraftTimes {
            all_day: false,
            start_date: "5.11.".into(),
            start_time: "930".into(),
            end_date: "".into(),
            end_time: "11".into(),
        };
        let (start, end) = draft.to_values(today(), Berlin, None).unwrap();
        let at = |h, m| date(2026, 11, 5).and_time(time(h, m));
        assert_eq!(start, DateTimeValue::Zoned(at(9, 30), Berlin));
        assert_eq!(end, DateTimeValue::Zoned(at(11, 0), Berlin));
        let shown = DraftTimes::from_values(&start, &end, Berlin);
        assert_eq!(
            (shown.start_time.as_str(), shown.end_date.as_str()),
            ("09:30", "05.11.2026")
        );
    }

    #[test]
    fn all_day_end_is_inclusive_in_the_form() {
        let draft = DraftTimes {
            all_day: true,
            start_date: "24.12.2026".into(),
            end_date: "26.12.2026".into(),
            ..Default::default()
        };
        let (start, end) = draft.to_values(today(), Berlin, None).unwrap();
        assert_eq!(start, DateTimeValue::Date(date(2026, 12, 24)));
        assert_eq!(end, DateTimeValue::Date(date(2026, 12, 27)));
        assert_eq!(
            DraftTimes::from_values(&start, &end, Berlin).end_date,
            "26.12.2026"
        );
    }

    #[test]
    fn edited_event_keeps_its_zone() {
        let ny: Tz = "America/New_York".parse().unwrap();
        let original = DateTimeValue::Zoned(date(2026, 11, 9).and_time(time(10, 0)), ny);
        // In Berlin auf 17:00 gesetzt = 11:00 in New York
        let draft = DraftTimes {
            all_day: false,
            start_date: "09.11.2026".into(),
            start_time: "17:00".into(),
            end_date: "09.11.2026".into(),
            end_time: "18:00".into(),
        };
        let (start, _) = draft.to_values(today(), Berlin, Some(&original)).unwrap();
        assert_eq!(
            start,
            DateTimeValue::Zoned(date(2026, 11, 9).and_time(time(11, 0)), ny)
        );
    }

    #[test]
    fn invalid_drafts_explain_the_problem() {
        let base = DraftTimes {
            all_day: false,
            start_date: "5.11.".into(),
            start_time: "10".into(),
            end_date: "5.11.".into(),
            end_time: "9".into(),
        };
        assert_eq!(
            base.to_values(today(), Berlin, None),
            Err("Das Ende muss nach dem Beginn liegen.".into())
        );
        let bad_time = DraftTimes {
            start_time: "abends".into(),
            ..base.clone()
        };
        assert!(
            bad_time
                .to_values(today(), Berlin, None)
                .unwrap_err()
                .starts_with("Beginn: Uhrzeit")
        );
        let bad_date = DraftTimes {
            end_date: "32.1.".into(),
            ..base
        };
        assert!(
            bad_date
                .to_values(today(), Berlin, None)
                .unwrap_err()
                .starts_with("Ende: Datum")
        );
    }
}
