//! Wiederholung im Termin-Dialog (wie Calinos RecurrenceFields): Felder aus
//! einer RRULE füllen, wieder lesen und auf Deutsch beschreiben.

use chrono::{Datelike, NaiveDate};
use hora_core::Tz;
use hora_core::recur::{End, Frequency, MonthlyBy, SimpleRule, WEEKDAYS, nth_weekday_of};
use hora_core::series::RuleChange;
use hora_core::time::DateTimeValue;

use crate::forms::{MONTHS, WEEKDAYS as DAY_NAMES, format_date, parse_date};

/// Auswahl "Wiederholen"; die Reihenfolge entspricht der Liste im Dialog.
pub const NEVER: i32 = 0;
pub const CUSTOM: i32 = 5;
const FREQUENCIES: [Frequency; 4] = [
    Frequency::Daily,
    Frequency::Weekly,
    Frequency::Monthly,
    Frequency::Yearly,
];

/// Monatsmuster im Dialog
const DAY_OF_MONTH: i32 = 0;
const NTH_WEEKDAY: i32 = 1;
const LAST_WEEKDAY: i32 = 2;

const ORDINALS: [&str; 4] = ["ersten", "zweiten", "dritten", "vierten"];

/// Rohwerte der Dialogfelder
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleFields {
    pub repeat: i32,
    pub interval: String,
    /// Wochentage als Bits (Mo = 1, Di = 2, Mi = 4 …); 0 = Wochentag des Beginns
    pub weekdays: i32,
    pub monthly: i32,
    /// 0 = nie, 1 = am Datum, 2 = nach Anzahl
    pub end: i32,
    pub until: String,
    pub count: String,
}

impl Default for RuleFields {
    fn default() -> Self {
        RuleFields {
            repeat: NEVER,
            interval: "1".into(),
            weekdays: 0,
            monthly: DAY_OF_MONTH,
            end: 0,
            until: String::new(),
            count: "10".into(),
        }
    }
}

/// Gelesene Felder
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repeat {
    Never,
    /// Eine Regel, die der Dialog nicht darstellen kann; bleibt unverändert
    Custom,
    Simple {
        frequency: Frequency,
        interval: u32,
        weekdays: i32,
        monthly: i32,
        end: End,
    },
}

fn bit(day: chrono::Weekday) -> i32 {
    1 << day.num_days_from_monday()
}

fn days_of(mask: i32, start_day: NaiveDate) -> Vec<chrono::Weekday> {
    if mask == 0 {
        return vec![start_day.weekday()];
    }
    WEEKDAYS
        .into_iter()
        .filter(|d| mask & bit(*d) != 0)
        .collect()
}

/// n-ter Wochentag von `day`; ab dem fünften gilt "letzter"
fn nth_of(day: NaiveDate) -> i32 {
    match nth_weekday_of(day, false) {
        5 => -1,
        n => n,
    }
}

impl RuleFields {
    /// Felder für die Regel eines Termins, der mit `start` beginnt.
    pub fn from_rule(rule: Option<&str>, start: &DateTimeValue, home: Tz) -> RuleFields {
        let Some(rule) = rule else {
            return RuleFields::default();
        };
        let start_day = start.naive().date();
        let custom = RuleFields {
            repeat: CUSTOM,
            ..RuleFields::default()
        };
        let Some(simple) = SimpleRule::parse(rule, start, home) else {
            return custom;
        };
        let monthly = match simple.monthly {
            MonthlyBy::DayOfMonth => DAY_OF_MONTH,
            MonthlyBy::NthWeekday(-1)
                if (start_day + chrono::Duration::days(7)).month() != start_day.month() =>
            {
                LAST_WEEKDAY
            }
            MonthlyBy::NthWeekday(n) if n == nth_weekday_of(start_day, false) => NTH_WEEKDAY,
            // Muster passt nicht zum Beginn: so lassen, wie es ist
            MonthlyBy::NthWeekday(_) => return custom,
        };
        let mask = simple.weekdays.iter().fold(0, |m, d| m | bit(*d));
        let (end, until, count) = match simple.end {
            End::Never => (0, String::new(), "10".to_string()),
            End::Until(day) => (1, format_date(day), "10".to_string()),
            End::Count(n) => (2, String::new(), n.to_string()),
        };
        RuleFields {
            repeat: FREQUENCIES
                .iter()
                .position(|f| *f == simple.frequency)
                .map_or(CUSTOM, |i| i as i32 + 1),
            interval: simple.interval.to_string(),
            weekdays: if simple.weekdays == [start_day.weekday()] {
                0
            } else {
                mask
            },
            monthly,
            end,
            until,
            count,
        }
    }

    /// Felder prüfen und lesen.
    pub fn read(&self, today: NaiveDate) -> Result<Repeat, String> {
        let frequency = match self.repeat {
            NEVER => return Ok(Repeat::Never),
            CUSTOM => return Ok(Repeat::Custom),
            n => *FREQUENCIES
                .get((n - 1) as usize)
                .ok_or("Unbekannte Wiederholung.")?,
        };
        let interval = self
            .interval
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=99).contains(n))
            .ok_or("Bitte bei „alle“ eine Zahl von 1 bis 99 angeben.")?;
        let end = match self.end {
            1 => End::Until(
                parse_date(&self.until, today)
                    .ok_or("Das Ende der Wiederholung ist kein gültiges Datum.")?,
            ),
            2 => End::Count(
                self.count
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|n| (1..=999).contains(n))
                    .ok_or("Bitte als Anzahl eine Zahl von 1 bis 999 angeben.")?,
            ),
            _ => End::Never,
        };
        Ok(Repeat::Simple {
            frequency,
            interval,
            weekdays: if frequency == Frequency::Weekly {
                self.weekdays & 0x7f
            } else {
                0
            },
            monthly: if frequency == Frequency::Monthly {
                self.monthly
            } else {
                DAY_OF_MONTH
            },
            end,
        })
    }
}

impl Repeat {
    fn simple(&self, start_day: NaiveDate) -> Option<SimpleRule> {
        let Repeat::Simple {
            frequency,
            interval,
            weekdays,
            monthly,
            end,
        } = self
        else {
            return None;
        };
        Some(SimpleRule {
            frequency: *frequency,
            interval: *interval,
            weekdays: if *frequency == Frequency::Weekly {
                days_of(*weekdays, start_day)
            } else {
                Vec::new()
            },
            monthly: match *monthly {
                NTH_WEEKDAY => MonthlyBy::NthWeekday(nth_of(start_day)),
                LAST_WEEKDAY => MonthlyBy::NthWeekday(-1),
                _ => MonthlyBy::DayOfMonth,
            },
            end: *end,
        })
    }

    /// RRULE für eine Serie, die mit `start` beginnt
    pub fn rrule(&self, start: &DateTimeValue, home: Tz) -> Option<String> {
        self.simple(start.naive().date())
            .map(|r| r.to_rrule(start, home))
    }

    /// Ende vor dem Beginn?
    pub fn ends_before(&self, start_day: NaiveDate) -> bool {
        matches!(self, Repeat::Simple { end: End::Until(day), .. } if *day < start_day)
    }

    /// "Wöchentlich am Montag und Mittwoch, bis 30.06.2027"
    pub fn describe(&self, start_day: NaiveDate) -> String {
        let Some(rule) = self.simple(start_day) else {
            return match self {
                Repeat::Custom => "Eigene Wiederholungsregel".into(),
                _ => String::new(),
            };
        };
        let n = rule.interval;
        let day_name = |d: chrono::Weekday| DAY_NAMES[d.num_days_from_monday() as usize];
        let base = match rule.frequency {
            Frequency::Daily if n == 1 => "Täglich".to_string(),
            Frequency::Daily => format!("Alle {n} Tage"),
            Frequency::Weekly => {
                let names: Vec<&str> = rule.weekdays.iter().map(|d| day_name(*d)).collect();
                let mask = rule.weekdays.iter().fold(0, |m, d| m | bit(*d));
                if mask == 0b11111 && n == 1 {
                    "Werktags (Montag bis Freitag)".to_string()
                } else if n == 1 {
                    format!("Wöchentlich am {}", join(&names))
                } else {
                    format!("Alle {n} Wochen am {}", join(&names))
                }
            }
            Frequency::Monthly => {
                let on = match rule.monthly {
                    MonthlyBy::DayOfMonth => format!("am {}.", start_day.day()),
                    MonthlyBy::NthWeekday(-1) => {
                        format!("am letzten {}", day_name(start_day.weekday()))
                    }
                    MonthlyBy::NthWeekday(k) => format!(
                        "am {} {}",
                        ORDINALS
                            .get((k - 1).max(0) as usize)
                            .copied()
                            .unwrap_or("letzten"),
                        day_name(start_day.weekday())
                    ),
                };
                if n == 1 {
                    format!("Monatlich {on}")
                } else {
                    format!("Alle {n} Monate {on}")
                }
            }
            Frequency::Yearly => {
                let on = format!(
                    "am {}. {}",
                    start_day.day(),
                    MONTHS[start_day.month0() as usize]
                );
                if n == 1 {
                    format!("Jährlich {on}")
                } else {
                    format!("Alle {n} Jahre {on}")
                }
            }
        };
        match rule.end {
            End::Never => base,
            End::Until(day) => format!("{base}, bis {}", format_date(day)),
            End::Count(1) => format!("{base}, einmal"),
            End::Count(c) => format!("{base}, {c}-mal"),
        }
    }
}

/// "Montag", "Montag und Mittwoch", "Montag, Mittwoch und Freitag"
fn join(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} und {last}", rest.join(", ")),
    }
}

/// Beschreibung der Regel einer Serie für die Vorschau
pub fn describe_rule(rule: &str, start: &DateTimeValue, home: Tz) -> String {
    let today = start.naive().date();
    RuleFields::from_rule(Some(rule), start, home)
        .read(today)
        .map(|r| r.describe(start.naive().date()))
        .unwrap_or_else(|_| "Eigene Wiederholungsregel".into())
}

/// Was beim Speichern mit der Regel geschieht. `before` sind die Felder, wie
/// der Dialog sie geöffnet hat; unverändert heißt: Regel behalten (eine
/// Verschiebung passt Wochentage dann selbst an).
pub fn rule_change(
    had_rule: bool,
    before: &Repeat,
    after: &Repeat,
    rule_start: &DateTimeValue,
    home: Tz,
) -> RuleChange {
    if before == after {
        return RuleChange::Keep;
    }
    match after.rrule(rule_start, home) {
        Some(rule) => RuleChange::Set(rule),
        None if *after == Repeat::Never && had_rule => RuleChange::Remove,
        None => RuleChange::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;
    use chrono_tz::Europe::Berlin;

    fn start(s: &str) -> DateTimeValue {
        DateTimeValue::Zoned(
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap(),
            Berlin,
        )
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn roundtrip(rule: &str, from: &str) -> (RuleFields, String) {
        let s = start(from);
        let fields = RuleFields::from_rule(Some(rule), &s, Berlin);
        let read = fields.read(day(2026, 11, 4)).unwrap();
        (fields, read.describe(s.naive().date()))
    }

    #[test]
    fn rules_fill_the_fields_and_read_back() {
        // Mittwoch, 4.11.2026
        let (fields, text) = roundtrip("FREQ=WEEKLY", "2026-11-04 08:00");
        assert_eq!((fields.repeat, fields.weekdays), (2, 0));
        assert_eq!(text, "Wöchentlich am Mittwoch");

        let (fields, text) = roundtrip(
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;UNTIL=20270630T215959Z",
            "2026-11-04 08:00",
        );
        assert_eq!(fields.weekdays, 0b101);
        assert_eq!(fields.until, "30.06.2027");
        assert_eq!(text, "Alle 2 Wochen am Montag und Mittwoch, bis 30.06.2027");

        assert_eq!(
            roundtrip("FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR", "2026-11-04 08:00").1,
            "Werktags (Montag bis Freitag)"
        );
        assert_eq!(
            roundtrip("FREQ=MONTHLY;BYDAY=1WE;COUNT=6", "2026-11-04 08:00").1,
            "Monatlich am ersten Mittwoch, 6-mal"
        );
        assert_eq!(
            roundtrip("FREQ=MONTHLY;BYDAY=-1MO", "2026-11-30 08:00").1,
            "Monatlich am letzten Montag"
        );
        assert_eq!(
            roundtrip("FREQ=MONTHLY;INTERVAL=3", "2026-11-04 08:00").1,
            "Alle 3 Monate am 4."
        );
        assert_eq!(
            roundtrip("FREQ=YEARLY", "2026-03-15 00:00").1,
            "Jährlich am 15. März"
        );
        assert_eq!(
            roundtrip("FREQ=DAILY;INTERVAL=2;COUNT=1", "2026-11-04 08:00").1,
            "Alle 2 Tage, einmal"
        );

        // Unbekannte Teile oder Muster, die nicht zum Beginn passen: eigene Regel
        assert_eq!(
            roundtrip("FREQ=MONTHLY;BYMONTHDAY=1,15", "2026-11-04 08:00")
                .0
                .repeat,
            CUSTOM
        );
        assert_eq!(
            roundtrip("FREQ=MONTHLY;BYDAY=3WE", "2026-11-04 08:00")
                .0
                .repeat,
            CUSTOM
        );
        assert_eq!(
            describe_rule("FREQ=HOURLY", &start("2026-11-04 08:00"), Berlin),
            "Eigene Wiederholungsregel"
        );
    }

    #[test]
    fn unchanged_fields_keep_the_rule() {
        let s = start("2026-11-02 08:00");
        let fields = RuleFields::from_rule(Some("FREQ=WEEKLY;BYDAY=MO;X-FOO=1"), &s, Berlin);
        assert_eq!(fields.repeat, CUSTOM);
        let before = fields.read(day(2026, 11, 4)).unwrap();
        // Auf Dienstag verlegt, Regel nicht angefasst: behalten (wird verschoben)
        let moved = start("2026-11-03 08:00");
        assert_eq!(
            rule_change(true, &before, &before, &moved, Berlin),
            RuleChange::Keep
        );

        let weekly = RuleFields::from_rule(Some("FREQ=WEEKLY;BYDAY=MO"), &s, Berlin);
        let before = weekly.read(day(2026, 11, 4)).unwrap();
        let mut changed = weekly.clone();
        changed.weekdays = 0b11;
        let after = changed.read(day(2026, 11, 4)).unwrap();
        assert_eq!(
            rule_change(true, &before, &after, &s, Berlin),
            RuleChange::Set("FREQ=WEEKLY;BYDAY=MO,TU".into())
        );
        assert_eq!(
            rule_change(true, &before, &Repeat::Never, &s, Berlin),
            RuleChange::Remove
        );
        assert_eq!(
            rule_change(false, &Repeat::Never, &Repeat::Never, &s, Berlin),
            RuleChange::Keep
        );
    }

    #[test]
    fn new_rules_follow_the_start() {
        let fields = RuleFields {
            repeat: 3,
            monthly: NTH_WEEKDAY,
            end: 2,
            count: "4".into(),
            ..RuleFields::default()
        };
        let repeat = fields.read(day(2026, 11, 4)).unwrap();
        assert_eq!(
            repeat.rrule(&start("2026-11-18 08:00"), Berlin).as_deref(),
            Some("FREQ=MONTHLY;BYDAY=3WE;COUNT=4")
        );
        // Fünfter Montag gibt es nicht jeden Monat: dann "letzter"
        assert_eq!(
            repeat.rrule(&start("2026-11-30 08:00"), Berlin).as_deref(),
            Some("FREQ=MONTHLY;BYDAY=-1MO;COUNT=4")
        );
        let weekly = RuleFields {
            repeat: 2,
            end: 1,
            until: "31.12.26".into(),
            ..RuleFields::default()
        };
        let repeat = weekly.read(day(2026, 11, 4)).unwrap();
        assert_eq!(
            repeat.rrule(&start("2026-11-04 08:00"), Berlin).as_deref(),
            Some("FREQ=WEEKLY;BYDAY=WE;UNTIL=20261231T225959Z")
        );
        assert!(repeat.ends_before(day(2027, 1, 1)));
    }

    #[test]
    fn invalid_fields_are_reported() {
        let today = day(2026, 11, 4);
        let bad_interval = RuleFields {
            repeat: 1,
            interval: "0".into(),
            ..RuleFields::default()
        };
        assert!(bad_interval.read(today).is_err());
        let bad_count = RuleFields {
            repeat: 1,
            end: 2,
            count: "x".into(),
            ..RuleFields::default()
        };
        assert!(bad_count.read(today).is_err());
        let bad_until = RuleFields {
            repeat: 1,
            end: 1,
            until: "irgendwann".into(),
            ..RuleFields::default()
        };
        assert!(bad_until.read(today).is_err());
    }
}
