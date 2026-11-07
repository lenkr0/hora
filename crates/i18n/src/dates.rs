//! Datums- und Zeitangaben je Sprache.
//!
//! Namen (Wochentage, Monate) und Satzbau ("9. November", "9 de noviembre")
//! kommen aus der App-Sprache; Zahlenformate (Reihenfolge Tag/Monat/Jahr,
//! Trennzeichen, 12- oder 24-Stunden-Uhr) aus den Windows-Regionseinstellungen,
//! siehe [`Region`].

use crate::{Lang, language};
use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};
use std::sync::RwLock;

/// Reihenfolge in Zahlendaten
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateOrder {
    /// 09.11.2026, 09/11/2026
    Dmy,
    /// 11/09/2026
    Mdy,
    /// 2026-11-09
    Ymd,
}

/// Zahlenformate aus den Windows-Regionseinstellungen
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub order: DateOrder,
    pub sep: char,
    pub pad_day: bool,
    pub pad_month: bool,
    pub hour12: bool,
    pub pad_hour: bool,
}

impl Region {
    /// Hora bis 1.0: 09.11.2026, 09:00
    pub const DEFAULT: Region = Region {
        order: DateOrder::Dmy,
        sep: '.',
        pad_day: true,
        pad_month: true,
        hour12: false,
        pad_hour: true,
    };

    /// Aus Windows-Mustern wie "dd.MM.yyyy" / "M/d/yyyy" und "HH:mm" / "h:mm tt"
    pub fn from_patterns(short_date: &str, short_time: &str) -> Region {
        let mut region = Region::DEFAULT;
        let date: String = short_date
            .chars()
            .filter(|c| matches!(c, 'd' | 'M' | 'y'))
            .collect();
        let first = |c: char| date.find(c).unwrap_or(usize::MAX);
        let (d, m, y) = (first('d'), first('M'), first('y'));
        if d != usize::MAX && m != usize::MAX {
            region.order = if y < m && y < d {
                DateOrder::Ymd
            } else if m < d {
                DateOrder::Mdy
            } else {
                DateOrder::Dmy
            };
        }
        if let Some(sep) = short_date
            .chars()
            .find(|c| !c.is_alphanumeric() && !c.is_whitespace() && *c != '\'')
        {
            region.sep = sep;
        }
        region.pad_day = short_date.contains("dd");
        region.pad_month = short_date.contains("MM");
        let time = short_time.trim();
        region.hour12 = time.contains('h') && !time.contains('H') || time.contains('t');
        region.pad_hour = time.contains("HH") || time.contains("hh");
        region
    }
}

static REGION: RwLock<Region> = RwLock::new(Region::DEFAULT);

pub fn set_region(region: Region) {
    if let Ok(mut r) = REGION.write() {
        *r = region;
    }
}

pub fn region() -> Region {
    REGION.read().map(|r| *r).unwrap_or(Region::DEFAULT)
}

const WEEKDAYS: [[&str; 7]; 7] = [
    [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ],
    [
        "Montag",
        "Dienstag",
        "Mittwoch",
        "Donnerstag",
        "Freitag",
        "Samstag",
        "Sonntag",
    ],
    [
        "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche",
    ],
    [
        "lunes",
        "martes",
        "miércoles",
        "jueves",
        "viernes",
        "sábado",
        "domingo",
    ],
    [
        "lunedì",
        "martedì",
        "mercoledì",
        "giovedì",
        "venerdì",
        "sabato",
        "domenica",
    ],
    [
        "maandag",
        "dinsdag",
        "woensdag",
        "donderdag",
        "vrijdag",
        "zaterdag",
        "zondag",
    ],
    [
        "poniedziałek",
        "wtorek",
        "środa",
        "czwartek",
        "piątek",
        "sobota",
        "niedziela",
    ],
];

/// Abkürzung im Fließtext ("Mon", "Mo.", "lun.")
const WEEKDAYS_SHORT: [[&str; 7]; 7] = [
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    ["Mo.", "Di.", "Mi.", "Do.", "Fr.", "Sa.", "So."],
    ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."],
    ["lun.", "mar.", "mié.", "jue.", "vie.", "sáb.", "dom."],
    ["lun", "mar", "mer", "gio", "ven", "sab", "dom"],
    ["ma", "di", "wo", "do", "vr", "za", "zo"],
    ["pon.", "wt.", "śr.", "czw.", "pt.", "sob.", "niedz."],
];

/// Spaltenkopf ohne Punkt ("Mon", "Mo", "lun")
const WEEKDAYS_HEADER: [[&str; 7]; 7] = [
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"],
    ["lun", "mar", "mer", "jeu", "ven", "sam", "dim"],
    ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"],
    ["lun", "mar", "mer", "gio", "ven", "sab", "dom"],
    ["ma", "di", "wo", "do", "vr", "za", "zo"],
    ["Pn", "Wt", "Śr", "Cz", "Pt", "So", "Nd"],
];

/// Ein Buchstabe für den Mini-Kalender
const WEEKDAY_LETTERS: [[&str; 7]; 7] = [
    ["M", "T", "W", "T", "F", "S", "S"],
    ["M", "D", "M", "D", "F", "S", "S"],
    ["L", "M", "M", "J", "V", "S", "D"],
    ["L", "M", "X", "J", "V", "S", "D"],
    ["L", "M", "M", "G", "V", "S", "D"],
    ["M", "D", "W", "D", "V", "Z", "Z"],
    ["P", "W", "Ś", "C", "P", "S", "N"],
];

const MONTHS: [[&str; 12]; 7] = [
    [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    [
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
    ],
    [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ],
    [
        "enero",
        "febrero",
        "marzo",
        "abril",
        "mayo",
        "junio",
        "julio",
        "agosto",
        "septiembre",
        "octubre",
        "noviembre",
        "diciembre",
    ],
    [
        "gennaio",
        "febbraio",
        "marzo",
        "aprile",
        "maggio",
        "giugno",
        "luglio",
        "agosto",
        "settembre",
        "ottobre",
        "novembre",
        "dicembre",
    ],
    [
        "januari",
        "februari",
        "maart",
        "april",
        "mei",
        "juni",
        "juli",
        "augustus",
        "september",
        "oktober",
        "november",
        "december",
    ],
    [
        "styczeń",
        "luty",
        "marzec",
        "kwiecień",
        "maj",
        "czerwiec",
        "lipiec",
        "sierpień",
        "wrzesień",
        "październik",
        "listopad",
        "grudzień",
    ],
];

/// Polnisch braucht nach dem Tag den Genitiv ("9 listopada")
const MONTHS_PL_GENITIVE: [&str; 12] = [
    "stycznia",
    "lutego",
    "marca",
    "kwietnia",
    "maja",
    "czerwca",
    "lipca",
    "sierpnia",
    "września",
    "października",
    "listopada",
    "grudnia",
];

const MONTHS_SHORT: [[&str; 12]; 7] = [
    [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    [
        "Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sep.", "Okt.", "Nov.",
        "Dez.",
    ],
    [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
        "déc.",
    ],
    [
        "ene.", "feb.", "mar.", "abr.", "may.", "jun.", "jul.", "ago.", "sept.", "oct.", "nov.",
        "dic.",
    ],
    [
        "gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic",
    ],
    [
        "jan.", "feb.", "mrt.", "apr.", "mei", "jun.", "jul.", "aug.", "sep.", "okt.", "nov.",
        "dec.",
    ],
    [
        "sty", "lut", "mar", "kwi", "maj", "cze", "lip", "sie", "wrz", "paź", "lis", "gru",
    ],
];

fn li() -> usize {
    Lang::ALL.iter().position(|l| *l == language()).unwrap_or(0)
}

fn wd(d: NaiveDate) -> usize {
    d.weekday().num_days_from_monday() as usize
}

/// Wochentag, 0 = Montag ("Monday", "Montag", "lundi")
pub fn weekday(i: usize) -> &'static str {
    WEEKDAYS[li()][i % 7]
}

pub fn weekday_short(i: usize) -> &'static str {
    WEEKDAYS_SHORT[li()][i % 7]
}

pub fn weekday_header(i: usize) -> &'static str {
    WEEKDAYS_HEADER[li()][i % 7]
}

pub fn weekday_letter(i: usize) -> &'static str {
    WEEKDAY_LETTERS[li()][i % 7]
}

/// Monatsname, 0 = Januar (Nominativ: "listopad")
pub fn month(m0: usize) -> &'static str {
    MONTHS[li()][m0 % 12]
}

pub fn month_short(m0: usize) -> &'static str {
    MONTHS_SHORT[li()][m0 % 12]
}

/// Erster Buchstabe groß ("lundi" → "Lundi"), für Überschriften
pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Tageszahl wie im Satz: "9", "9." (de), "1er" (fr, nur der Erste)
fn day_num(d: NaiveDate) -> String {
    match language() {
        Lang::De => format!("{}.", d.day()),
        Lang::Fr if d.day() == 1 => "1er".into(),
        _ => d.day().to_string(),
    }
}

/// "9 November", "9. November", "9 novembre", "9 de noviembre", "9 listopada"
pub fn day_month(d: NaiveDate) -> String {
    let m = d.month0() as usize;
    match language() {
        Lang::Es => format!("{} de {}", d.day(), month(m)),
        Lang::Pl => format!("{} {}", d.day(), MONTHS_PL_GENITIVE[m]),
        _ => format!("{} {}", day_num(d), month(m)),
    }
}

/// "9 Nov", "9. Nov.", "9 nov."
pub fn day_month_short(d: NaiveDate) -> String {
    format!("{} {}", day_num(d), month_short(d.month0() as usize))
}

/// "9 November 2026", "9 de noviembre de 2026"
pub fn day_month_year(d: NaiveDate) -> String {
    match language() {
        Lang::Es => format!("{} de {}", day_month(d), d.year()),
        _ => format!("{} {}", day_month(d), d.year()),
    }
}

fn joined(weekday: &str, rest: &str) -> String {
    match language() {
        // Ohne Komma: "lundi 9 novembre", "lunedì 9 novembre", "maandag 9 november"
        Lang::Fr | Lang::It | Lang::Nl => format!("{weekday} {rest}"),
        _ => format!("{weekday}, {rest}"),
    }
}

/// "Monday, 9 November", "Montag, 9. November", "lundi 9 novembre"
pub fn weekday_day_month(d: NaiveDate) -> String {
    joined(weekday(wd(d)), &day_month(d))
}

/// "Monday, 9 November 2026"
pub fn full_date(d: NaiveDate) -> String {
    joined(weekday(wd(d)), &day_month_year(d))
}

/// "Mon, 9 Nov", "Mo., 9. Nov.", "lun. 9 nov."
pub fn short_weekday_date(d: NaiveDate) -> String {
    joined(weekday_short(wd(d)), &day_month_short(d))
}

/// "November 2026", "noviembre de 2026", "listopad 2026"
pub fn month_year(year: i32, m0: usize) -> String {
    match language() {
        Lang::Es => format!("{} de {year}", month(m0)),
        _ => format!("{} {year}", month(m0)),
    }
}

/// Zeitraum zweier Tage: "2–8 November 2026", "30 Nov – 6 Dec 2026",
/// "28 Dec 2026 – 3 Jan 2027" (Deutsch: "2.–8. November 2026")
pub fn day_range(first: NaiveDate, last: NaiveDate) -> String {
    if first == last {
        return day_month_year(first);
    }
    if first.year() == last.year() && first.month() == last.month() {
        return format!("{}–{}", day_num(first), day_month_year(last));
    }
    if first.year() == last.year() {
        return format!(
            "{} – {} {}",
            day_month_short(first),
            day_month_short(last),
            last.year()
        );
    }
    format!(
        "{} {} – {} {}",
        day_month_short(first),
        first.year(),
        day_month_short(last),
        last.year()
    )
}

/// Polnische Wochentage im Akkusativ ("w środę")
const WEEKDAYS_PL_ACCUSATIVE: [&str; 7] = [
    "poniedziałek",
    "wtorek",
    "środę",
    "czwartek",
    "piątek",
    "sobotę",
    "niedzielę",
];

/// Wochentage für "wöchentlich am …": "Monday and Wednesday",
/// "Montag, Mittwoch und Freitag", "poniedziałek i środę" (0 = Montag)
pub fn weekday_list(days: &[usize]) -> String {
    let lang = language();
    let names: Vec<&str> = days
        .iter()
        .map(|d| match lang {
            Lang::Pl => WEEKDAYS_PL_ACCUSATIVE[d % 7],
            _ => weekday(*d),
        })
        .collect();
    let and = match lang {
        Lang::En => "and",
        Lang::De => "und",
        Lang::Fr => "et",
        Lang::Es => "y",
        Lang::It => "e",
        Lang::Nl => "en",
        Lang::Pl => "i",
    };
    match names.as_slice() {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} {and} {last}", rest.join(", ")),
    }
}

/// n-ter Wochentag im Monat für "monatlich am …" (n = 1…4, -1 = letzter):
/// "first Wednesday", "ersten Mittwoch", "premier mercredi", "la prima
/// domenica", "pierwszą środę". Artikel und Präposition stehen in der
/// Übersetzung von "Monthly on the {}", nur Italienisch braucht den Artikel
/// hier, weil er vom Geschlecht des Wochentags abhängt.
pub fn nth_weekday(n: i32, weekday_index: usize) -> String {
    let i = match n {
        1..=4 => (n - 1) as usize,
        _ => 4,
    };
    let wd = weekday_index % 7;
    match language() {
        Lang::En => {
            let ord = ["first", "second", "third", "fourth", "last"][i];
            format!("{ord} {}", weekday(wd))
        }
        Lang::De => {
            let ord = ["ersten", "zweiten", "dritten", "vierten", "letzten"][i];
            format!("{ord} {}", weekday(wd))
        }
        Lang::Fr => {
            let ord = ["premier", "deuxième", "troisième", "quatrième", "dernier"][i];
            format!("{ord} {}", weekday(wd))
        }
        Lang::Es => {
            let ord = ["primer", "segundo", "tercer", "cuarto", "último"][i];
            format!("{ord} {}", weekday(wd))
        }
        Lang::It => {
            // Nur "domenica" ist weiblich
            let ord = if wd == 6 {
                [
                    "la prima",
                    "la seconda",
                    "la terza",
                    "la quarta",
                    "l'ultima",
                ][i]
            } else {
                [
                    "il primo",
                    "il secondo",
                    "il terzo",
                    "il quarto",
                    "l'ultimo",
                ][i]
            };
            format!("{ord} {}", weekday(wd))
        }
        Lang::Nl => {
            let ord = ["eerste", "tweede", "derde", "vierde", "laatste"][i];
            format!("{ord} {}", weekday(wd))
        }
        Lang::Pl => {
            // środa, sobota, niedziela sind weiblich
            let ord = if matches!(wd, 2 | 5 | 6) {
                ["pierwszą", "drugą", "trzecią", "czwartą", "ostatnią"][i]
            } else {
                ["pierwszy", "drugi", "trzeci", "czwarty", "ostatni"][i]
            };
            format!("{ord} {}", WEEKDAYS_PL_ACCUSATIVE[wd])
        }
    }
}

/// Kurzer Zeitraum ohne Jahr: "10–12 Nov", "10.–12. Nov.", "30 Nov – 2 Dec"
pub fn short_day_range(first: NaiveDate, last: NaiveDate) -> String {
    if first == last {
        return day_month_short(first);
    }
    if first.year() == last.year() && first.month() == last.month() {
        return format!("{}–{}", day_num(first), day_month_short(last));
    }
    format!("{} – {}", day_month_short(first), day_month_short(last))
}

/// Datum in Zahlen nach Region: "09.11.2026", "11/9/2026", "2026-11-09"
pub fn numeric(d: NaiveDate) -> String {
    numeric_in(d, region())
}

/// Wie [`numeric`], aber mit einer bestimmten Region
pub fn numeric_in(d: NaiveDate, r: Region) -> String {
    let day = if r.pad_day {
        format!("{:02}", d.day())
    } else {
        d.day().to_string()
    };
    let month = if r.pad_month {
        format!("{:02}", d.month())
    } else {
        d.month().to_string()
    };
    let s = r.sep;
    match r.order {
        DateOrder::Dmy => format!("{day}{s}{month}{s}{}", d.year()),
        DateOrder::Mdy => format!("{month}{s}{day}{s}{}", d.year()),
        DateOrder::Ymd => format!("{}{s}{month}{s}{day}", d.year()),
    }
}

/// Platzhalter für Datumsfelder: "DD.MM.YYYY", "TT.MM.JJJJ", "MM/DD/YYYY"
pub fn numeric_placeholder() -> String {
    let (d, m, y) = match language() {
        Lang::En => ("DD", "MM", "YYYY"),
        Lang::De => ("TT", "MM", "JJJJ"),
        Lang::Fr => ("JJ", "MM", "AAAA"),
        Lang::Es => ("DD", "MM", "AAAA"),
        Lang::It => ("GG", "MM", "AAAA"),
        Lang::Nl => ("DD", "MM", "JJJJ"),
        Lang::Pl => ("DD", "MM", "RRRR"),
    };
    let r = region();
    let s = r.sep;
    match r.order {
        DateOrder::Dmy => format!("{d}{s}{m}{s}{y}"),
        DateOrder::Mdy => format!("{m}{s}{d}{s}{y}"),
        DateOrder::Ymd => format!("{y}{s}{m}{s}{d}"),
    }
}

/// Zahlendatum nach Region lesen (auch ohne Jahr, zweistelliges Jahr, ISO)
pub fn parse_numeric(input: &str, default_year: i32) -> Option<NaiveDate> {
    let s = input.trim();
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    let parts: Vec<&str> = s
        .trim_end_matches(['.', '/', '-'])
        .split(['.', '/', '-'])
        .map(str::trim)
        .collect();
    if parts.len() < 2 || parts.len() > 3 || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let num = |i: usize| parts.get(i).and_then(|p| p.parse::<i32>().ok());
    let year = |p: Option<&&str>| -> Option<i32> {
        match p {
            None => Some(default_year),
            Some(y) if y.len() <= 2 => Some(2000 + y.parse::<i32>().ok()?),
            Some(y) => y.parse().ok(),
        }
    };
    let (y, m, d) = match region().order {
        DateOrder::Ymd if parts.len() == 3 => (year(parts.first())?, num(1)?, num(2)?),
        DateOrder::Ymd => (default_year, num(0)?, num(1)?),
        DateOrder::Mdy => (year(parts.get(2))?, num(0)?, num(1)?),
        DateOrder::Dmy => (year(parts.get(2))?, num(1)?, num(0)?),
    };
    NaiveDate::from_ymd_opt(y, u32::try_from(m).ok()?, u32::try_from(d).ok()?)
}

/// Uhrzeit nach Region: "09:30", "9:30 AM"
pub fn time(t: NaiveTime) -> String {
    clock(t.hour(), t.minute())
}

/// Wie [`time`], aber auch 24:00 für das Tagesende
pub fn clock(hour: u32, minute: u32) -> String {
    clock_in(hour, minute, region())
}

/// Wie [`clock`], aber mit einer bestimmten Region
pub fn clock_in(hour: u32, minute: u32, r: Region) -> String {
    if r.hour12 {
        // 24:00 als Tagesende ist in 12-Stunden-Schreibweise 12:00 AM
        let hour = hour % 24;
        let h = match hour % 12 {
            0 => 12,
            h => h,
        };
        let suffix = if hour < 12 { "AM" } else { "PM" };
        return format!("{h}:{minute:02} {suffix}");
    }
    if r.pad_hour {
        format!("{hour:02}:{minute:02}")
    } else {
        format!("{hour}:{minute:02}")
    }
}

/// Stundenbeschriftung der Zeitachse: "08:00" oder "8 AM"
pub fn hour_label(hour: u32) -> String {
    if region().hour12 {
        let h = match hour % 12 {
            0 => 12,
            h => h,
        };
        return format!("{h} {}", if hour < 12 { "AM" } else { "PM" });
    }
    format!("{hour:02}:00")
}

/// Uhrzeit lesen: "9", "930", "9:30", "9.30", "21:05", "9 Uhr", "9h30",
/// "9pm", "9:30 pm", "12 am"
pub fn parse_time(input: &str) -> Option<NaiveTime> {
    let mut s = input.trim().to_lowercase().replace(' ', "");
    let mut pm = None;
    for (suffix, is_pm) in [("a.m.", false), ("p.m.", true), ("am", false), ("pm", true)] {
        if let Some(rest) = s.strip_suffix(suffix) {
            pm = Some(is_pm);
            s = rest.to_string();
            break;
        }
    }
    let s = s.strip_suffix("uhr").unwrap_or(&s);
    let s = s.strip_suffix('h').unwrap_or(s);
    let (hour, minute) = if let Some((h, m)) = s.split_once([':', '.', 'h']) {
        let m = if m.is_empty() { "0" } else { m };
        (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?)
    } else if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
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
    let hour = match pm {
        Some(_) if hour == 0 || hour > 12 => return None,
        Some(true) if hour < 12 => hour + 12,
        Some(false) if hour == 12 => 0,
        _ => hour,
    };
    NaiveTime::from_hms_opt(hour, minute, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::with_language;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn long_dates_in_every_language() {
        let date = d(2026, 11, 9);
        let expect = [
            (Lang::En, "Monday, 9 November 2026"),
            (Lang::De, "Montag, 9. November 2026"),
            (Lang::Fr, "lundi 9 novembre 2026"),
            (Lang::Es, "lunes, 9 de noviembre de 2026"),
            (Lang::It, "lunedì 9 novembre 2026"),
            (Lang::Nl, "maandag 9 november 2026"),
            (Lang::Pl, "poniedziałek, 9 listopada 2026"),
        ];
        for (lang, text) in expect {
            with_language(lang, || assert_eq!(full_date(date), text, "{lang:?}"));
        }
        with_language(Lang::Fr, || assert_eq!(day_month(d(2026, 5, 1)), "1er mai"));
        with_language(Lang::Pl, || {
            assert_eq!(month_year(2026, 10), "listopad 2026")
        });
    }

    #[test]
    fn ranges() {
        with_language(Lang::En, || {
            assert_eq!(
                day_range(d(2026, 11, 2), d(2026, 11, 8)),
                "2–8 November 2026"
            );
            assert_eq!(
                day_range(d(2026, 11, 30), d(2026, 12, 6)),
                "30 Nov – 6 Dec 2026"
            );
            assert_eq!(
                day_range(d(2026, 12, 28), d(2027, 1, 3)),
                "28 Dec 2026 – 3 Jan 2027"
            );
        });
        with_language(Lang::De, || {
            assert_eq!(
                day_range(d(2026, 11, 2), d(2026, 11, 8)),
                "2.–8. November 2026"
            );
            assert_eq!(short_weekday_date(d(2026, 11, 4)), "Mi., 4. Nov.");
        });
    }

    #[test]
    fn windows_patterns() {
        let us = Region::from_patterns("M/d/yyyy", "h:mm tt");
        assert_eq!(
            (us.order, us.sep, us.hour12, us.pad_day),
            (DateOrder::Mdy, '/', true, false)
        );
        let de = Region::from_patterns("dd.MM.yyyy", "HH:mm");
        assert_eq!(de, Region::DEFAULT);
        let iso = Region::from_patterns("yyyy-MM-dd", "HH:mm");
        assert_eq!((iso.order, iso.sep), (DateOrder::Ymd, '-'));
        let uk = Region::from_patterns("dd/MM/yyyy", "HH:mm");
        assert_eq!((uk.order, uk.sep, uk.hour12), (DateOrder::Dmy, '/', false));
    }

    #[test]
    fn times() {
        assert_eq!(parse_time("9pm"), NaiveTime::from_hms_opt(21, 0, 0));
        assert_eq!(parse_time("12 am"), NaiveTime::from_hms_opt(0, 0, 0));
        assert_eq!(parse_time("12:30 PM"), NaiveTime::from_hms_opt(12, 30, 0));
        assert_eq!(parse_time("9h30"), NaiveTime::from_hms_opt(9, 30, 0));
        assert_eq!(parse_time("9 Uhr"), NaiveTime::from_hms_opt(9, 0, 0));
        assert_eq!(parse_time("0930"), NaiveTime::from_hms_opt(9, 30, 0));
        assert_eq!(parse_time("13pm"), None);
        assert_eq!(clock(9, 5), "09:05");
    }
}
