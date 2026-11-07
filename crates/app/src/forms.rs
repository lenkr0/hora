//! Datum und Uhrzeit: Eingaben im Termin-Dialog lesen und Zeiträume für
//! Vorschau und Titel beschreiben. Namen und Satzbau je Sprache, Zahlenformate
//! nach Windows-Region (siehe hora_i18n::dates).

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};
use hora_core::Tz;
use hora_core::time::{DateTimeValue, local_to_utc};
use hora_i18n::{dates, tr};

/// Wörter für heute, morgen, übermorgen in allen Sprachen von Hora
const RELATIVE_DAYS: [(&str, i64); 22] = [
    ("today", 0),
    ("tomorrow", 1),
    ("day after tomorrow", 2),
    ("heute", 0),
    ("morgen", 1),
    ("übermorgen", 2),
    ("uebermorgen", 2),
    ("aujourd'hui", 0),
    ("demain", 1),
    ("après-demain", 2),
    ("hoy", 0),
    ("mañana", 1),
    ("pasado mañana", 2),
    ("oggi", 0),
    ("domani", 1),
    ("dopodomani", 2),
    ("vandaag", 0),
    ("overmorgen", 2),
    ("dziś", 0),
    ("dzisiaj", 0),
    ("jutro", 1),
    ("pojutrze", 2),
];

/// Datum nach Windows-Region ("4.11.2026", "11/4/2026"), auch ohne Jahr oder
/// mit zweistelligem Jahr, ISO ("2026-11-04") und Wörter wie "today", "morgen".
pub fn parse_date(input: &str, today: NaiveDate) -> Option<NaiveDate> {
    let s = input.trim().to_lowercase();
    if let Some((_, days)) = RELATIVE_DAYS.iter().find(|(word, _)| *word == s) {
        return today.checked_add_signed(Duration::days(*days));
    }
    dates::parse_numeric(&s, today.year())
}

/// "9", "930", "9:30", "9.30", "21:05", "9 Uhr", "9pm", "9:30 am", "9h30"
pub fn parse_time(input: &str) -> Option<NaiveTime> {
    dates::parse_time(input)
}

pub fn format_date(d: NaiveDate) -> String {
    dates::numeric(d)
}

pub fn format_time(t: NaiveTime) -> String {
    dates::time(t)
}

/// "Wed, 4 Nov", "Mi., 4. Nov."
pub fn short_day(d: NaiveDate) -> String {
    dates::short_weekday_date(d)
}

/// "Wednesday, 4 November 2026", "Mittwoch, 4. November 2026"
pub fn long_day(d: NaiveDate) -> String {
    dates::full_date(d)
}

/// Zeitraum für die Vorschau. `end` ist bei ganztägigen Terminen der Tag nach
/// dem letzten.
pub fn describe(start: NaiveDateTime, end: NaiveDateTime, all_day: bool) -> String {
    if all_day {
        let last = (end.date() - Duration::days(1)).max(start.date());
        return if last == start.date() {
            tr!("{} · all day", short_day(start.date()))
        } else {
            tr!(
                "{} – {} · all day",
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
            dates::clock(24, 0)
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

fn at_hour(hour: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(hour, 0, 0).unwrap_or_default()
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
                start_time: format_time(at_hour(9)),
                end_date: format_date(last),
                end_time: format_time(at_hour(10)),
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
        let example = format_date(today);
        let start_date = parse_date(&self.start_date, today)
            .ok_or_else(|| tr!("Start: date not recognized (e.g. {})", example))?;
        let end_date = if self.end_date.trim().is_empty() {
            start_date
        } else {
            parse_date(&self.end_date, today)
                .ok_or_else(|| tr!("End: date not recognized (e.g. {})", example))?
        };
        if self.all_day {
            if end_date < start_date {
                return Err(tr!("The end is before the start.").into());
            }
            return Ok((
                DateTimeValue::Date(start_date),
                DateTimeValue::Date(end_date + Duration::days(1)),
            ));
        }
        let start_time = parse_time(&self.start_time)
            .ok_or_else(|| tr!("Start: time not recognized (e.g. {})", dates::clock(9, 30)))?;
        let end_time = parse_time(&self.end_time)
            .ok_or_else(|| tr!("End: time not recognized (e.g. {})", dates::clock(10, 30)))?;
        let start = start_date.and_time(start_time);
        let end = end_date.and_time(end_time);
        if end <= start {
            return Err(tr!("The end must be after the start.").into());
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
    use hora_i18n::{Lang, with_language};

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
        assert_eq!(parse_date("tomorrow", today()), Some(date(2026, 11, 5)));
        assert_eq!(parse_date("demain", today()), Some(date(2026, 11, 5)));
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
        assert_eq!(parse_time("9:30 pm"), Some(time(21, 30)));
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
            "Wed, 4 Nov · 09:00–10:30"
        );
        assert_eq!(
            describe(at(today(), 23, 0), at(date(2026, 11, 5), 0, 0), false),
            "Wed, 4 Nov · 23:00–24:00"
        );
        assert_eq!(
            describe(at(today(), 22, 0), at(date(2026, 11, 5), 6, 0), false),
            "Wed, 4 Nov, 22:00 – Thu, 5 Nov, 06:00"
        );
        assert_eq!(
            describe(at(today(), 0, 0), at(date(2026, 11, 5), 0, 0), true),
            "Wed, 4 Nov · all day"
        );
        assert_eq!(
            describe(at(today(), 0, 0), at(date(2026, 11, 7), 0, 0), true),
            "Wed, 4 Nov – Fri, 6 Nov · all day"
        );
        assert_eq!(long_day(today()), "Wednesday, 4 November 2026");
    }

    #[test]
    fn descriptions_in_german() {
        with_language(Lang::De, || {
            let at = |d: NaiveDate, h, m| d.and_time(time(h, m));
            assert_eq!(
                describe(at(today(), 0, 0), at(date(2026, 11, 7), 0, 0), true),
                "Mi., 4. Nov. – Fr., 6. Nov. · ganztägig"
            );
            assert_eq!(long_day(today()), "Mittwoch, 4. November 2026");
        });
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
            Err("The end must be after the start.".into())
        );
        let bad_time = DraftTimes {
            start_time: "abends".into(),
            ..base.clone()
        };
        assert!(
            bad_time
                .to_values(today(), Berlin, None)
                .unwrap_err()
                .starts_with("Start: time")
        );
        let bad_date = DraftTimes {
            end_date: "32.1.".into(),
            ..base
        };
        assert!(
            bad_date
                .to_values(today(), Berlin, None)
                .unwrap_err()
                .starts_with("End: date")
        );
    }
}
