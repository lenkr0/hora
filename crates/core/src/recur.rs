//! Wiederholungsregeln (RRULE) lesen, ändern und schreiben.
//!
//! [`RuleParts`] bearbeitet eine Regel Teil für Teil und lässt unbekannte
//! Teile stehen. [`SimpleRule`] ist die Form, die der Termin-Dialog anbietet
//! (wie bei Calino: Häufigkeit, Intervall, Wochentage, Monatsmuster, Ende);
//! Regeln, die nicht hineinpassen, bleiben unverändert erhalten.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc, Weekday};
use chrono_tz::Tz;

use crate::time::{DateTimeValue, local_to_utc};

const WEEKDAY_CODES: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];
pub const WEEKDAYS: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

pub fn weekday_code(w: Weekday) -> &'static str {
    WEEKDAY_CODES[w.num_days_from_monday() as usize]
}

fn weekday_from_code(code: &str) -> Option<Weekday> {
    WEEKDAY_CODES
        .iter()
        .position(|c| c.eq_ignore_ascii_case(code))
        .map(|i| WEEKDAYS[i])
}

/// BYDAY-Eintrag in Ordnungszahl und Wochentag zerlegen: "-1FR" -> (Some(-1), Fr)
fn split_byday(token: &str) -> Option<(Option<i32>, Weekday)> {
    let token = token.trim();
    let (number, code) = token.split_at(token.len().checked_sub(2)?);
    let weekday = weekday_from_code(code)?;
    let ordinal = if number.is_empty() {
        None
    } else {
        Some(number.trim_start_matches('+').parse().ok()?)
    };
    Some((ordinal, weekday))
}

/// Eine RRULE als geordnete Liste von Teilen; Schlüssel in Großbuchstaben.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleParts(pub Vec<(String, String)>);

impl RuleParts {
    pub fn parse(rule: &str) -> Self {
        let rule = rule.trim().strip_prefix("RRULE:").unwrap_or(rule.trim());
        RuleParts(
            rule.split(';')
                .filter_map(|part| part.split_once('='))
                .map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim().to_string()))
                .collect(),
        )
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some(part) => part.1 = value,
            None => self.0.push((key.to_string(), value)),
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.0.retain(|(k, _)| k != key);
    }

    /// Wochentage (BYDAY) und Monatstage (BYMONTHDAY) um `days` Tage
    /// verschieben, wenn eine Serie auf einen anderen Tag gelegt wird.
    pub fn shift_days(&mut self, days: i64) {
        if days == 0 {
            return;
        }
        if let Some(byday) = self.get("BYDAY").map(str::to_string) {
            let shifted: Option<Vec<String>> = byday
                .split(',')
                .map(|token| {
                    let (ordinal, weekday) = split_byday(token)?;
                    let index =
                        (weekday.num_days_from_monday() as i64 + days).rem_euclid(7) as usize;
                    Some(format!(
                        "{}{}",
                        ordinal.map(|n| n.to_string()).unwrap_or_default(),
                        WEEKDAY_CODES[index]
                    ))
                })
                .collect();
            if let Some(shifted) = shifted {
                self.set("BYDAY", shifted.join(","));
            }
        }
        if let Some(bymonthday) = self.get("BYMONTHDAY").map(str::to_string) {
            let shifted: Vec<String> = bymonthday
                .split(',')
                .map(|d| match d.trim().parse::<i64>() {
                    // Positive Tage verschieben, im Bereich 1–31 bleiben
                    Ok(day) if day > 0 => ((day - 1 + days).rem_euclid(31) + 1).to_string(),
                    _ => d.trim().to_string(),
                })
                .collect();
            self.set("BYMONTHDAY", shifted.join(","));
        }
    }
}

impl std::fmt::Display for RuleParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts: Vec<String> = self.0.iter().map(|(k, v)| format!("{k}={v}")).collect();
        f.write_str(&parts.join(";"))
    }
}

/// UNTIL-Wert für "bis einschließlich `last`" passend zum DTSTART-Typ:
/// Datum bei ganztägig, Ortszeit bei schwebend, sonst UTC (RFC 5545, 3.3.10).
pub fn until_value(start: &DateTimeValue, last: DateTime<Utc>, home: Tz) -> String {
    match start {
        DateTimeValue::Date(_) => last
            .with_timezone(&home)
            .date_naive()
            .format("%Y%m%d")
            .to_string(),
        DateTimeValue::Floating(_) => last
            .with_timezone(&home)
            .naive_local()
            .format("%Y%m%dT%H%M%S")
            .to_string(),
        DateTimeValue::Utc(_) | DateTimeValue::Zoned(..) => {
            last.format("%Y%m%dT%H%M%SZ").to_string()
        }
    }
}

/// UNTIL-Wert als Zeitpunkt (Datum = Tagesende in der Zone des Beginns).
pub fn parse_until(value: &str, start: &DateTimeValue, home: Tz) -> Option<DateTime<Utc>> {
    let zone = start.zone(home);
    if value.len() == 8 {
        let day = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some(local_to_utc(day.and_hms_opt(23, 59, 59)?, zone));
    }
    let (body, utc) = match value.strip_suffix('Z') {
        Some(body) => (body, true),
        None => (value, false),
    };
    let naive = NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M%S").ok()?;
    Some(if utc {
        naive.and_utc()
    } else {
        local_to_utc(naive, zone)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Frequency {
    fn code(self) -> &'static str {
        match self {
            Frequency::Daily => "DAILY",
            Frequency::Weekly => "WEEKLY",
            Frequency::Monthly => "MONTHLY",
            Frequency::Yearly => "YEARLY",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonthlyBy {
    /// Am selben Tag wie der Beginn, z. B. jeden 4.
    DayOfMonth,
    /// Am n-ten Wochentag des Beginns (1–4, -1 = letzter), z. B. 1. Mittwoch
    NthWeekday(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Never,
    /// Letzter möglicher Tag (einschließlich)
    Until(NaiveDate),
    Count(u32),
}

/// Regel in der Form des Termin-Dialogs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleRule {
    pub frequency: Frequency,
    pub interval: u32,
    /// Bei wöchentlich: Wochentage (leer = Wochentag des Beginns)
    pub weekdays: Vec<Weekday>,
    pub monthly: MonthlyBy,
    pub end: End,
}

/// n-ter Wochentag des Monats für `day` (1–4) bzw. -1, wenn es der letzte ist.
pub fn nth_weekday_of(day: NaiveDate, prefer_last: bool) -> i32 {
    let is_last = (day + Duration::days(7)).month() != day.month();
    if prefer_last && is_last {
        -1
    } else {
        ((day.day() - 1) / 7 + 1) as i32
    }
}

impl SimpleRule {
    /// Liest eine RRULE, sofern sie in die Dialog-Form passt; sonst `None`.
    pub fn parse(rule: &str, start: &DateTimeValue, home: Tz) -> Option<SimpleRule> {
        let parts = RuleParts::parse(rule);
        let known = [
            "FREQ",
            "INTERVAL",
            "BYDAY",
            "BYMONTHDAY",
            "COUNT",
            "UNTIL",
            "WKST",
        ];
        if parts.0.iter().any(|(k, _)| !known.contains(&k.as_str())) {
            return None;
        }
        let interval = match parts.get("INTERVAL") {
            Some(v) => v.parse().ok().filter(|n| *n >= 1)?,
            None => 1,
        };
        let end = match (parts.get("COUNT"), parts.get("UNTIL")) {
            (Some(count), None) => End::Count(count.parse().ok()?),
            // Letzter Tag, an dem noch ein Termin beginnen kann: Endet die Regel
            // vor der Uhrzeit des Beginns (etwa nach dem Teilen einer Serie),
            // ist das der Vortag.
            (None, Some(until)) => {
                let local = parse_until(until, start, home)?
                    .with_timezone(&start.zone(home))
                    .naive_local();
                let day = local.date();
                End::Until(if local.time() < start.naive().time() {
                    day.pred_opt()?
                } else {
                    day
                })
            }
            (None, None) => End::Never,
            (Some(_), Some(_)) => return None,
        };
        let byday: Option<Vec<(Option<i32>, Weekday)>> = match parts.get("BYDAY") {
            Some(v) => Some(v.split(',').map(split_byday).collect::<Option<Vec<_>>>()?),
            None => None,
        };
        let plain_days = |list: &Vec<(Option<i32>, Weekday)>| -> Option<Vec<Weekday>> {
            list.iter()
                .map(|(n, w)| if n.is_none() { Some(*w) } else { None })
                .collect()
        };
        let start_day = start.naive().date();
        let (frequency, weekdays, monthly) = match parts.get("FREQ")? {
            "DAILY" | "WEEKLY" if parts.get("BYMONTHDAY").is_some() => return None,
            // Täglich an bestimmten Wochentagen ist dasselbe wie wöchentlich an diesen Tagen
            "DAILY" => match &byday {
                Some(list) if interval == 1 => {
                    (Frequency::Weekly, plain_days(list)?, MonthlyBy::DayOfMonth)
                }
                Some(_) => return None,
                None => (Frequency::Daily, Vec::new(), MonthlyBy::DayOfMonth),
            },
            "WEEKLY" => {
                let days = match &byday {
                    Some(list) => plain_days(list)?,
                    None => vec![start_day.weekday()],
                };
                (Frequency::Weekly, days, MonthlyBy::DayOfMonth)
            }
            "MONTHLY" => {
                let monthly = match (&byday, parts.get("BYMONTHDAY")) {
                    (None, None) => MonthlyBy::DayOfMonth,
                    (None, Some(d)) if d.parse::<u32>().ok() == Some(start_day.day()) => {
                        MonthlyBy::DayOfMonth
                    }
                    (Some(list), None) if list.len() == 1 && list[0].1 == start_day.weekday() => {
                        MonthlyBy::NthWeekday(
                            list[0].0.filter(|n| (1..=4).contains(n) || *n == -1)?,
                        )
                    }
                    _ => return None,
                };
                (Frequency::Monthly, Vec::new(), monthly)
            }
            "YEARLY" if byday.is_none() && parts.get("BYMONTHDAY").is_none() => {
                (Frequency::Yearly, Vec::new(), MonthlyBy::DayOfMonth)
            }
            _ => return None,
        };
        Some(SimpleRule {
            frequency,
            interval,
            weekdays,
            monthly,
            end,
        })
    }

    /// RRULE-Wert für eine Serie, die mit `start` beginnt.
    pub fn to_rrule(&self, start: &DateTimeValue, home: Tz) -> String {
        let mut parts = RuleParts(vec![("FREQ".into(), self.frequency.code().into())]);
        if self.interval > 1 {
            parts.set("INTERVAL", self.interval.to_string());
        }
        let start_day = start.naive().date();
        match self.frequency {
            Frequency::Weekly => {
                let mut days = if self.weekdays.is_empty() {
                    vec![start_day.weekday()]
                } else {
                    self.weekdays.clone()
                };
                days.sort_by_key(|w| w.num_days_from_monday());
                days.dedup();
                parts.set(
                    "BYDAY",
                    days.iter()
                        .map(|w| weekday_code(*w))
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
            Frequency::Monthly => {
                if let MonthlyBy::NthWeekday(n) = self.monthly {
                    parts.set("BYDAY", format!("{n}{}", weekday_code(start_day.weekday())));
                }
            }
            Frequency::Daily | Frequency::Yearly => {}
        }
        match self.end {
            End::Never => {}
            End::Count(n) => parts.set("COUNT", n.max(1).to_string()),
            End::Until(day) => {
                let last = local_to_utc(
                    day.and_time(NaiveTime::from_hms_opt(23, 59, 59).unwrap()),
                    start.zone(home),
                );
                parts.set("UNTIL", until_value(start, last, home));
            }
        }
        parts.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::Europe::Berlin;

    fn zoned(s: &str) -> DateTimeValue {
        DateTimeValue::Zoned(
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap(),
            Berlin,
        )
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn parts_keep_unknown_entries_and_order() {
        let mut parts = RuleParts::parse("FREQ=WEEKLY;X-NAME=foo;BYDAY=MO");
        parts.set("COUNT", "3");
        parts.remove("BYDAY");
        assert_eq!(parts.to_string(), "FREQ=WEEKLY;X-NAME=foo;COUNT=3");
    }

    #[test]
    fn shifting_moves_weekdays_and_month_days() {
        let mut parts = RuleParts::parse("FREQ=WEEKLY;BYDAY=MO,WE,SU");
        parts.shift_days(1);
        assert_eq!(parts.get("BYDAY"), Some("TU,TH,MO"));
        let mut monthly = RuleParts::parse("FREQ=MONTHLY;BYDAY=-1FR");
        monthly.shift_days(-1);
        assert_eq!(monthly.get("BYDAY"), Some("-1TH"));
        let mut days = RuleParts::parse("FREQ=MONTHLY;BYMONTHDAY=31,-1");
        days.shift_days(1);
        assert_eq!(days.get("BYMONTHDAY"), Some("1,-1"));
    }

    #[test]
    fn until_matches_the_start_type() {
        let last = Utc.with_ymd_and_hms(2026, 11, 30, 22, 59, 59).unwrap();
        assert_eq!(
            until_value(&zoned("2026-11-02 08:00"), last, Berlin),
            "20261130T225959Z"
        );
        assert_eq!(
            until_value(&DateTimeValue::Date(date(2026, 11, 2)), last, Berlin),
            "20261130"
        );
        let floating = DateTimeValue::Floating(date(2026, 11, 2).and_hms_opt(8, 0, 0).unwrap());
        assert_eq!(until_value(&floating, last, Berlin), "20261130T235959");
        assert_eq!(
            parse_until("20261130T225959Z", &zoned("2026-11-02 08:00"), Berlin),
            Some(last)
        );
    }

    #[test]
    fn simple_rules_roundtrip() {
        let start = zoned("2026-11-04 08:00"); // Mittwoch
        for rule in [
            "FREQ=DAILY",
            "FREQ=DAILY;INTERVAL=3;COUNT=10",
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE",
            "FREQ=MONTHLY",
            "FREQ=MONTHLY;BYDAY=1WE",
            "FREQ=YEARLY;UNTIL=20301104T225959Z",
        ] {
            let parsed =
                SimpleRule::parse(rule, &start, Berlin).unwrap_or_else(|| panic!("{rule}"));
            assert_eq!(parsed.to_rrule(&start, Berlin), rule);
        }
    }

    #[test]
    fn weekdays_only_daily_becomes_weekly() {
        let start = zoned("2026-11-02 09:30");
        let rule = SimpleRule::parse("FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR", &start, Berlin).unwrap();
        assert_eq!(rule.frequency, Frequency::Weekly);
        assert_eq!(rule.weekdays.len(), 5);
    }

    #[test]
    fn unusual_rules_are_not_simplified() {
        let start = zoned("2026-11-04 08:00");
        assert_eq!(
            SimpleRule::parse("FREQ=MONTHLY;BYSETPOS=-1;BYDAY=MO,TU", &start, Berlin),
            None
        );
        assert_eq!(SimpleRule::parse("FREQ=HOURLY", &start, Berlin), None);
        assert_eq!(
            SimpleRule::parse("FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU", &start, Berlin),
            None
        );
        assert_eq!(
            SimpleRule::parse("FREQ=MONTHLY;BYDAY=2TU", &start, Berlin),
            None,
            "Wochentag passt nicht zum Beginn"
        );
    }

    #[test]
    fn until_as_inclusive_day_in_the_events_zone() {
        let start = zoned("2026-11-04 08:00");
        let rule = SimpleRule {
            frequency: Frequency::Weekly,
            interval: 1,
            weekdays: vec![],
            monthly: MonthlyBy::DayOfMonth,
            end: End::Until(date(2026, 12, 31)),
        };
        // 31.12. 23:59:59 in Berlin = 22:59:59 UTC
        assert_eq!(
            rule.to_rrule(&start, Berlin),
            "FREQ=WEEKLY;BYDAY=WE;UNTIL=20261231T225959Z"
        );
        let back = SimpleRule::parse(&rule.to_rrule(&start, Berlin), &start, Berlin).unwrap();
        assert_eq!(back.end, End::Until(date(2026, 12, 31)));

        // Nach dem Teilen endet die Regel kurz vor dem Termin am 6.11. (9:30):
        // letzter Tag ist der 5.11., sonst käme der 6.11. beim Speichern zurück
        let standup = zoned("2026-11-02 09:30");
        let split = SimpleRule::parse(
            "FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR;UNTIL=20261106T082959Z",
            &standup,
            Berlin,
        )
        .unwrap();
        assert_eq!(split.end, End::Until(date(2026, 11, 5)));
    }

    #[test]
    fn nth_weekday() {
        assert_eq!(nth_weekday_of(date(2026, 11, 4), false), 1);
        assert_eq!(nth_weekday_of(date(2026, 11, 25), false), 4);
        assert_eq!(nth_weekday_of(date(2026, 11, 25), true), -1);
        assert_eq!(nth_weekday_of(date(2026, 11, 18), true), 3);
    }
}
