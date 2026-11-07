//! Schnelleingabe auf Deutsch und Englisch, wie Calinos Smart Input
//! (NLParser.ts): "Mathe jeden Montag 8 Uhr bis 9:30 Raum 204",
//! "Zahnarzt morgen 15 Uhr bei Dr. Weber", "todo Milch kaufen bis Freitag".
//!
//! Der Text wird nacheinander nach Aufgaben-Präfix, Kalender (`#Arbeit`),
//! Erinnerung, Dauer, Wiederholung, Uhrzeit, Datum und Ort durchsucht. Jeder
//! erkannte Teil wird ausgeblendet; was übrig bleibt, ist der Titel. Wie bei
//! Calino gilt ohne Uhrzeit "ganztägig", ohne Ende eine Stunde.

use std::sync::LazyLock;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use regex::{Captures, Regex};

use crate::recur::{End, Frequency, MonthlyBy, SimpleRule};

/// Ohne Ende und Dauer dauert ein Termin so lange (Calino: 60 Minuten)
pub const DEFAULT_MINUTES: i64 = 60;

/// Erkannte Wiederholung
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickRule {
    pub frequency: Frequency,
    pub interval: u32,
    pub weekdays: Vec<Weekday>,
    pub until: Option<NaiveDate>,
    pub count: Option<u32>,
}

impl QuickRule {
    /// Als Regel für den Termin-Dialog
    pub fn simple(&self) -> SimpleRule {
        SimpleRule {
            frequency: self.frequency,
            interval: self.interval,
            weekdays: self.weekdays.clone(),
            monthly: MonthlyBy::DayOfMonth,
            end: match (self.until, self.count) {
                (Some(d), _) => End::Until(d),
                (None, Some(n)) => End::Count(n),
                _ => End::Never,
            },
        }
    }
}

/// Was im Text steht (noch ohne Bezug auf heute)
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parsed {
    pub title: String,
    pub task: bool,
    pub date: Option<NaiveDate>,
    /// Letzter Tag bei mehrtägigen ganztägigen Terminen
    pub end_date: Option<NaiveDate>,
    pub start: Option<NaiveTime>,
    pub end: Option<NaiveTime>,
    pub duration: Option<Duration>,
    pub all_day: bool,
    pub location: Option<String>,
    pub rule: Option<QuickRule>,
    /// Minuten vor Beginn
    pub reminder: Option<i64>,
    /// Name nach `#`
    pub calendar: Option<String>,
}

/// Fertig ausgelegt: konkrete Zeiten
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum When {
    /// Ganztägig von `first` bis einschließlich `last`
    AllDay { first: NaiveDate, last: NaiveDate },
    Timed {
        start: NaiveDateTime,
        end: NaiveDateTime,
    },
    /// Aufgabe ohne Fälligkeit
    Undated,
    /// Aufgabe fällig an einem Tag bzw. zu einer Uhrzeit
    Due {
        day: NaiveDate,
        time: Option<NaiveTime>,
    },
}

const WEEKDAY_WORDS: &[(&str, Weekday)] = &[
    ("montags", Weekday::Mon),
    ("montag", Weekday::Mon),
    ("mondays", Weekday::Mon),
    ("monday", Weekday::Mon),
    ("mon", Weekday::Mon),
    ("mo", Weekday::Mon),
    ("dienstags", Weekday::Tue),
    ("dienstag", Weekday::Tue),
    ("tuesdays", Weekday::Tue),
    ("tuesday", Weekday::Tue),
    ("tues", Weekday::Tue),
    ("tue", Weekday::Tue),
    ("di", Weekday::Tue),
    ("mittwochs", Weekday::Wed),
    ("mittwoch", Weekday::Wed),
    ("wednesdays", Weekday::Wed),
    ("wednesday", Weekday::Wed),
    ("wed", Weekday::Wed),
    ("mi", Weekday::Wed),
    ("donnerstags", Weekday::Thu),
    ("donnerstag", Weekday::Thu),
    ("thursdays", Weekday::Thu),
    ("thursday", Weekday::Thu),
    ("thurs", Weekday::Thu),
    ("thur", Weekday::Thu),
    ("thu", Weekday::Thu),
    ("do", Weekday::Thu),
    ("freitags", Weekday::Fri),
    ("freitag", Weekday::Fri),
    ("fridays", Weekday::Fri),
    ("friday", Weekday::Fri),
    ("fri", Weekday::Fri),
    ("fr", Weekday::Fri),
    ("samstags", Weekday::Sat),
    ("samstag", Weekday::Sat),
    ("sonnabend", Weekday::Sat),
    ("saturdays", Weekday::Sat),
    ("saturday", Weekday::Sat),
    ("sat", Weekday::Sat),
    ("sa", Weekday::Sat),
    ("sonntags", Weekday::Sun),
    ("sonntag", Weekday::Sun),
    ("sundays", Weekday::Sun),
    ("sunday", Weekday::Sun),
    ("sun", Weekday::Sun),
    ("so", Weekday::Sun),
];

/// Ausgeschriebene Wochentage (kurze Formen wie "Mo" nur mit Punkt oder nach "am/jeden")
const WEEKDAY_FULL: &str = "montags?|dienstags?|mittwochs?|donnerstags?|freitags?|samstags?|sonnabends?|sonntags?|mondays?|tuesdays?|wednesdays?|thursdays?|fridays?|saturdays?|sundays?";
const WEEKDAY_SHORT: &str = "mo|di|mi|do|fr|sa|so|mon|tues?|wed|thu(?:rs?)?|fri|sat|sun";

const MONTHS: &[(&str, u32)] = &[
    ("januar", 1),
    ("jänner", 1),
    ("january", 1),
    ("jan", 1),
    ("februar", 2),
    ("february", 2),
    ("feb", 2),
    ("märz", 3),
    ("maerz", 3),
    ("march", 3),
    ("mär", 3),
    ("mrz", 3),
    ("mar", 3),
    ("april", 4),
    ("apr", 4),
    ("mai", 5),
    ("may", 5),
    ("juni", 6),
    ("june", 6),
    ("jun", 6),
    ("juli", 7),
    ("july", 7),
    ("jul", 7),
    ("august", 8),
    ("aug", 8),
    ("september", 9),
    ("sept", 9),
    ("sep", 9),
    ("oktober", 10),
    ("october", 10),
    ("okt", 10),
    ("oct", 10),
    ("november", 11),
    ("nov", 11),
    ("dezember", 12),
    ("december", 12),
    ("dez", 12),
    ("dec", 12),
];
const MONTH_RE: &str = "januar|jänner|january|jan|februar|february|feb|märz|maerz|march|mär|mrz|mar|april|apr|mai|may|juni|june|jun|juli|july|jul|august|aug|september|sept|sep|oktober|october|okt|oct|november|nov|dezember|december|dez|dec";

const NUMBER_WORDS: &[(&str, u32)] = &[
    ("ein", 1),
    ("eine", 1),
    ("einen", 1),
    ("einem", 1),
    ("einer", 1),
    ("a", 1),
    ("an", 1),
    ("one", 1),
    ("zwei", 2),
    ("two", 2),
    ("zweiten", 2),
    ("second", 2),
    ("2nd", 2),
    ("other", 2),
    ("drei", 3),
    ("three", 3),
    ("dritten", 3),
    ("third", 3),
    ("3rd", 3),
    ("vier", 4),
    ("four", 4),
    ("vierten", 4),
    ("fourth", 4),
    ("fünf", 5),
    ("five", 5),
    ("sechs", 6),
    ("six", 6),
    ("sieben", 7),
    ("seven", 7),
    ("acht", 8),
    ("eight", 8),
    ("neun", 9),
    ("nine", 9),
    ("zehn", 10),
    ("ten", 10),
];
const NUMBER_RE: &str = r"\d{1,3}|einen|einem|einer|eine|ein|an|a|one|zwei|two|drei|three|vier|four|fünf|five|sechs|six|sieben|seven|acht|eight|neun|nine|zehn|ten";

fn re(pattern: &str) -> Regex {
    Regex::new(&format!("(?i){pattern}")).expect("gültiges Muster")
}

fn weekday_of(word: &str) -> Option<Weekday> {
    let w = word.trim_end_matches('.').to_lowercase();
    WEEKDAY_WORDS.iter().find(|(k, _)| *k == w).map(|(_, d)| *d)
}

fn month_of(word: &str) -> Option<u32> {
    let w = word.trim_end_matches('.').to_lowercase();
    MONTHS.iter().find(|(k, _)| *k == w).map(|(_, m)| *m)
}

fn number_of(word: &str) -> Option<u32> {
    let w = word.to_lowercase();
    w.parse()
        .ok()
        .or_else(|| NUMBER_WORDS.iter().find(|(k, _)| *k == w).map(|(_, n)| *n))
}

/// Text, aus dem erkannte Teile ausgeblendet werden (gleiche Byte-Positionen)
struct Scanner {
    text: String,
}

impl Scanner {
    fn caps<'a>(&'a self, re: &Regex) -> Option<Captures<'a>> {
        re.captures(&self.text)
    }

    fn all<'a>(&'a self, re: &Regex) -> Vec<Captures<'a>> {
        re.captures_iter(&self.text).collect()
    }

    fn blank(&mut self, range: std::ops::Range<usize>) {
        let mut bytes = std::mem::take(&mut self.text).into_bytes();
        for b in &mut bytes[range] {
            *b = b' ';
        }
        // Nur ganze Zeichen werden ersetzt, das Ergebnis bleibt gültiges UTF-8
        self.text = String::from_utf8(bytes).unwrap_or_default();
    }
}

/// Uhrzeit aus Stunde, Minute und am/pm
fn clock(h: u32, m: u32, ampm: Option<&str>) -> Option<NaiveTime> {
    let h = match ampm.map(|s| s.to_lowercase().replace('.', "")) {
        Some(a) if a == "pm" && h < 12 => h + 12,
        Some(a) if a == "am" && h == 12 => 0,
        _ => h,
    };
    if h == 24 && m == 0 {
        return NaiveTime::from_hms_opt(23, 59, 0);
    }
    NaiveTime::from_hms_opt(h, m, 0)
}

const TIME: &str = r"(\d{1,2})(?:[:.](\d{2}))?\s*(am|pm|a\.m\.|p\.m\.)?\s*(uhr|h)?";

struct Time {
    time: NaiveTime,
    marked: bool,
}

fn time_from(c: &Captures, first: usize) -> Option<Time> {
    let h: u32 = c.get(first)?.as_str().parse().ok()?;
    let m: u32 = c
        .get(first + 1)
        .map_or(Some(0), |m| m.as_str().parse().ok())?;
    let ampm = c.get(first + 2).map(|m| m.as_str());
    let marked = c.get(first + 1).is_some() || ampm.is_some() || c.get(first + 3).is_some();
    Some(Time {
        time: clock(h, m, ampm)?,
        marked,
    })
}

static TASK_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^\s*(?:aufgabe|todo|to-do|task|erinnere mich (?:an|daran)|remind me to|remind me|reminder|erledigen)\b\s*:?\s*",
    )
});
static CALENDAR: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^|\s)#([\p{L}\p{N}_-]+)"));
static REMINDER: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\b(?:mit\s+)?(?:erinnerung|erinnern|reminder|remind me|remind)\s+(\d{1,3})\s*(minuten|minute|min|mins|minutes|stunden|stunde|std|hours?|h|tage|tag|days?|d)\.?\s*(?:vorher|davor|früher|before)?",
    )
});
static ALL_DAY: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(?:ganztägig|ganztags|den ganzen tag|ganzer tag|all[- ]day)\b"));
static DURATION: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\b(?:(?:für|for|dauer)\s+)?(\d{1,3}(?:[.,]\d)?)\s*(stunden|stunde|std|hours|hour|hrs|hr|h|minuten|minute|minutes|mins|min)\b\.?",
    )
});
static DURATION_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\b(?:(?:für|for)\s+)?(eine stunde|einer stunde|an hour|one hour|eine halbe stunde|halbe stunde|half an hour|half hour|anderthalb stunden|eineinhalb stunden)\b",
    )
});
static EVERY_UNIT: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"\b(?:jeden|jede|jedes|every|each)\s+(?:({NUMBER_RE}|zweiten|dritten|vierten|other|second|third|2nd|3rd)\s+)?(tag|tage|woche|wochen|monat|monate|jahr|jahre|day|days|week|weeks|month|months|year|years|werktag|weekday|wochenende|weekend)\b"
    ))
});
static EVERY_N: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"\b(?:alle|every)\s+({NUMBER_RE})\s+(tage|wochen|monate|jahre|days|weeks|months|years)\b"
    ))
});
static EVERY_WEEKDAY: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"\b(?:jeden|jede|every|each|on)\s+(?:(zweiten|dritten|other|second|third)\s+)?((?:{WEEKDAY_FULL}|(?:{WEEKDAY_SHORT})\.?)(?:\s*(?:,|und|and|&|/)\s*(?:{WEEKDAY_FULL}|(?:{WEEKDAY_SHORT})\.?))*)\b"
    ))
});
static WEEKDAY_ADVERBS: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\b((?:montags|dienstags|mittwochs|donnerstags|freitags|samstags|sonntags|mondays|tuesdays|wednesdays|thursdays|fridays|saturdays|sundays)(?:\s*(?:,|und|and|&)\s*(?:montags|dienstags|mittwochs|donnerstags|freitags|samstags|sonntags|mondays|tuesdays|wednesdays|thursdays|fridays|saturdays|sundays))*)\b",
    )
});
static FREQUENCY_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\b(täglich|daily|wöchentlich|weekly|monatlich|monthly|jährlich|yearly|annually|werktags|weekdays|wochenends|weekends)\b",
    )
});
static COUNT: LazyLock<Regex> = LazyLock::new(|| re(r"\b(\d{1,3})\s*(?:mal|times)\b"));
static CASUAL_DAILY: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\b(?:jeden|every)\s+(morgen|vormittag|mittag|nachmittag|abend|morning|noon|afternoon|evening|night|nacht)\b",
    )
});
static TIME_BETWEEN: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"\b(?:zwischen|between)\s+{TIME}\s+(?:und|and)\s+{TIME}"
    ))
});
static TIME_RANGE: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"(?:\b(von|from|um|at|ab)\s+)?\b{TIME}\s*(?:bis|-|–|—|to|till|until)\s*{TIME}(?:\b|$)"
    ))
});
static HALF: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(?:um\s+)?(halb|viertel nach|viertel vor|dreiviertel)\s+(\d{1,2})\b"));
static TIME_SINGLE: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"(?:\b(um|at|gegen|ab|from|@)\s*)?\b{TIME}(?:\b|$)"
    ))
});
static CASUAL_TIME: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\b(?:heute\s+)?(?:am\s+|in the\s+|this\s+)?(morgens|vormittags?|mittags?|nachmittags?|abends?|nachts|mitternacht|noon|midday|midnight|tonight|morning|afternoon|evening)\b",
    )
});
static UNTIL_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\b(?:bis|until|till|by|spätestens|fällig)\s+(?:zum\s+|zur\s+|einschließlich\s+|am\s+|on\s+)?$",
    )
});
static FROM_PREFIX: LazyLock<Regex> = LazyLock::new(|| re(r"\b(?:vom|von|from)\s+$"));
static RELATIVE_DAY: LazyLock<Regex> = LazyLock::new(|| {
    re(r"\b(heute|today|übermorgen|uebermorgen|day after tomorrow|morgen|tomorrow)\b")
});
static IN_UNITS: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"\bin\s+({NUMBER_RE})\s+(tagen|tag|wochen|woche|monaten|monat|jahren|jahr|days|day|weeks|week|months|month|years|year)\b"
    ))
});
static NEXT_WEEK: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\b(?:nächste|nächsten|kommende|kommenden|next)\s+(woche|monat|week|month)\b|\b(?:am\s+|this\s+|on the\s+)?(wochenende|weekend)\b",
    )
});
static ISO_DATE: LazyLock<Regex> = LazyLock::new(|| re(r"\b(\d{4})-(\d{2})-(\d{2})\b"));
static NUMERIC_DATE: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(\d{1,2})\.(\d{1,2})\.(\d{4}|\d{2})?"));
static DAY_MONTH_NAME: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"\b(\d{{1,2}})(?:\.|st|nd|rd|th)?\s*(?:of\s+)?({MONTH_RE})\b\.?(?:\s+(\d{{4}}))?"
    ))
});
static MONTH_NAME_DAY: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"\b({MONTH_RE})\.?\s+(\d{{1,2}})(?:st|nd|rd|th)?\b(?:,?\s+(\d{{4}}))?"
    ))
});
static DAY_ONLY: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(?:am|on the|the)\s+(\d{1,2})(?:\.|st|nd|rd|th)(?:\s|$|,)"));
static WEEKDAY_DATE: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"\b(?:(am|on|this|diesen|diesem|kommenden|kommender|nächsten|nächster|next|übernächsten)\s+)?({WEEKDAY_FULL}|(?:{WEEKDAY_SHORT})\.)(?:\s|$|,)"
    ))
});
static WEEKDAY_SHORT_AFTER_AM: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"\b(am|on|bis|ab)\s+({WEEKDAY_SHORT})\b")));
static LOCATION_KEYWORD: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\b(raum|zimmer|saal|hörsaal|gebäude|halle|room|hall|building)\s+([\p{L}\p{N}][\p{L}\p{N}.\-/]*)",
    )
});
static LOCATION_AT: LazyLock<Regex> = LazyLock::new(|| re(r"@\s*([^\s@][^@]*?)\s*$"));
static LOCATION_PREP: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\s(?:in der|in dem|in den|im|in|bei der|bei dem|beim|bei|at the|at|in the|ort:)\s+(\S.*?)\s*$",
    )
});

/// Leerzeichen, Füllwörter und Satzzeichen an den Rändern entfernen
fn tidy(text: &str) -> String {
    const FILLERS: &[&str] = &[
        "am",
        "um",
        "ab",
        "von",
        "vom",
        "bis",
        "on",
        "at",
        "the",
        "den",
        "der",
        "zum",
        "zur",
        "jeden",
        "jede",
        "every",
        "from",
        "to",
        "und",
        "and",
        "für",
        "for",
        "in",
        "im",
        "mit",
        "with",
        "gegen",
        "ganztägig",
    ];
    let mut words: Vec<&str> = text.split_whitespace().collect();
    loop {
        let before = words.len();
        while let Some(w) = words.last() {
            let bare = w.trim_matches(|c: char| ",.;:-–".contains(c));
            if bare.is_empty() || FILLERS.contains(&bare.to_lowercase().as_str()) {
                words.pop();
            } else {
                break;
            }
        }
        while let Some(w) = words.first() {
            let bare = w.trim_matches(|c: char| ",.;:-–".contains(c));
            if bare.is_empty() || FILLERS.contains(&bare.to_lowercase().as_str()) {
                words.remove(0);
            } else {
                break;
            }
        }
        if words.len() == before {
            break;
        }
    }
    let joined = words.join(" ");
    let joined = joined
        .trim_end_matches([',', ';', ':', '-', '–'])
        .trim()
        .to_string();
    // Leere Satzzeichen in der Mitte ("Mathe , Raum") zusammenziehen
    let joined = joined.replace(" ,", ",").replace(",,", ",");
    let mut chars = joined.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Text lesen. `today` legt fest, welches Jahr bei "12.11." gemeint ist.
pub fn parse(input: &str, today: NaiveDate) -> Parsed {
    let mut p = Parsed::default();
    let mut s = Scanner {
        text: input.replace(['\n', '\t'], " "),
    };

    if let Some(c) = s.caps(&TASK_PREFIX) {
        p.task = true;
        let r = c.get(0).unwrap().range();
        s.blank(r);
    }
    if let Some(c) = s.caps(&CALENDAR) {
        p.calendar = Some(c[1].to_string());
        let r = c.get(0).unwrap().range();
        s.blank(r);
    }
    if let Some(c) = s.caps(&REMINDER) {
        let n: i64 = c[1].parse().unwrap_or(0);
        let unit = c[2].to_lowercase();
        let factor = if unit.starts_with('s') || unit.starts_with('h') {
            60
        } else if unit.starts_with('t') || unit.starts_with('d') {
            1440
        } else {
            1
        };
        p.reminder = Some(n * factor);
        let r = c.get(0).unwrap().range();
        s.blank(r);
    }
    if let Some(c) = s.caps(&ALL_DAY) {
        p.all_day = true;
        let r = c.get(0).unwrap().range();
        s.blank(r);
    }
    if let Some(c) = s.caps(&DURATION_WORDS) {
        let w = c[1].to_lowercase();
        p.duration = Some(Duration::minutes(
            if w.contains("halb") && !w.contains("einhalb") || w.starts_with("half") {
                30
            } else if w.contains("anderthalb") || w.contains("einhalb") {
                90
            } else {
                60
            },
        ));
        let r = c.get(0).unwrap().range();
        s.blank(r);
    } else if let Some(c) = s.caps(&DURATION) {
        // "8h" allein ist eher eine Uhrzeit, wenn kein "für" davorsteht und keine Uhrzeit folgt
        let amount: f64 = c[1].replace(',', ".").parse().unwrap_or(0.0);
        let unit = c[2].to_lowercase();
        let has_prefix = c
            .get(0)
            .unwrap()
            .as_str()
            .to_lowercase()
            .starts_with(['f', 'd']);
        if has_prefix || unit != "h" {
            let minutes = if unit.starts_with('m') {
                amount
            } else {
                amount * 60.0
            };
            p.duration = Some(Duration::minutes(minutes.round() as i64));
            let r = c.get(0).unwrap().range();
            s.blank(r);
        }
    }

    parse_rule(&mut p, &mut s, today);
    parse_times(&mut p, &mut s);
    parse_dates(&mut p, &mut s, today);

    // Ort: "Raum 204", "@ Café", "bei Dr. Weber", "im Büro"
    if let Some(c) = s.caps(&LOCATION_KEYWORD) {
        p.location = Some(c.get(0).unwrap().as_str().trim().to_string());
        let r = c.get(0).unwrap().range();
        s.blank(r);
    } else if let Some(c) = s.caps(&LOCATION_AT) {
        p.location = Some(tidy_location(&c[1]));
        let r = c.get(0).unwrap().range();
        s.blank(r);
    } else if let Some(c) = s.caps(&LOCATION_PREP) {
        // Nur den letzten zusammenhängenden Rest (bis zu einer Lücke aus Erkanntem)
        let place = c[1].split("   ").next().unwrap_or("").trim().to_string();
        let place = tidy_location(&place);
        if place.chars().count() >= 2 && !place.chars().all(|c| c.is_ascii_digit()) {
            let start = c.get(0).unwrap().start();
            let end =
                c.get(1).unwrap().start() + c[1].find(place.as_str()).unwrap_or(0) + place.len();
            p.location = Some(place);
            s.blank(start..end.min(s.text.len()));
        }
    }

    p.title = tidy(&s.text);
    p
}

fn tidy_location(text: &str) -> String {
    text.trim().trim_matches([',', ';', '.']).trim().to_string()
}

fn parse_rule(p: &mut Parsed, s: &mut Scanner, today: NaiveDate) {
    let mut rule: Option<QuickRule> = None;
    let make = |frequency, interval| QuickRule {
        frequency,
        interval,
        weekdays: Vec::new(),
        until: None,
        count: None,
    };

    if let Some(c) = s.caps(&CASUAL_DAILY) {
        rule = Some(make(Frequency::Daily, 1));
        let word = c[1].to_lowercase();
        p.start = Some(casual_hour(&word));
        let r = c.get(0).unwrap().range();
        s.blank(r);
    }
    if rule.is_none()
        && let Some(c) = s.caps(&EVERY_WEEKDAY)
    {
        let days: Vec<Weekday> = c[2]
            .split([',', '&', '/', ' '])
            .filter(|w| !w.is_empty() && !["und", "and"].contains(&w.to_lowercase().as_str()))
            .filter_map(weekday_of)
            .collect();
        if !days.is_empty() {
            let interval = c
                .get(1)
                .and_then(|m| number_of(m.as_str()))
                .unwrap_or(1)
                .max(1);
            let mut r = make(Frequency::Weekly, interval);
            r.weekdays = days;
            rule = Some(r);
            let range = c.get(0).unwrap().range();
            s.blank(range);
        }
    }
    if rule.is_none()
        && let Some(c) = s.caps(&EVERY_UNIT)
    {
        let interval = c
            .get(1)
            .and_then(|m| number_of(m.as_str()))
            .unwrap_or(1)
            .max(1);
        let unit = c[2].to_lowercase();
        let r = match unit.as_str() {
            "werktag" | "weekday" => {
                let mut r = make(Frequency::Weekly, interval);
                r.weekdays = vec![
                    Weekday::Mon,
                    Weekday::Tue,
                    Weekday::Wed,
                    Weekday::Thu,
                    Weekday::Fri,
                ];
                r
            }
            "wochenende" | "weekend" => {
                let mut r = make(Frequency::Weekly, interval);
                r.weekdays = vec![Weekday::Sat, Weekday::Sun];
                r
            }
            u if u.starts_with("tag") || u.starts_with("day") => make(Frequency::Daily, interval),
            u if u.starts_with("woche") || u.starts_with("week") => {
                make(Frequency::Weekly, interval)
            }
            u if u.starts_with("monat") || u.starts_with("month") => {
                make(Frequency::Monthly, interval)
            }
            _ => make(Frequency::Yearly, interval),
        };
        rule = Some(r);
        let range = c.get(0).unwrap().range();
        s.blank(range);
    }
    if rule.is_none()
        && let Some(c) = s.caps(&EVERY_N)
    {
        let interval = number_of(&c[1]).unwrap_or(1).max(1);
        let unit = c[2].to_lowercase();
        let frequency = if unit.starts_with('t') || unit.starts_with('d') {
            Frequency::Daily
        } else if unit.starts_with('w') {
            Frequency::Weekly
        } else if unit.starts_with('m') {
            Frequency::Monthly
        } else {
            Frequency::Yearly
        };
        rule = Some(make(frequency, interval));
        let range = c.get(0).unwrap().range();
        s.blank(range);
    }
    if rule.is_none()
        && let Some(c) = s.caps(&FREQUENCY_WORDS)
    {
        let w = c[1].to_lowercase();
        let mut r = match w.as_str() {
            "täglich" | "daily" => make(Frequency::Daily, 1),
            "monatlich" | "monthly" => make(Frequency::Monthly, 1),
            "jährlich" | "yearly" | "annually" => make(Frequency::Yearly, 1),
            _ => make(Frequency::Weekly, 1),
        };
        if ["werktags", "weekdays"].contains(&w.as_str()) {
            r.weekdays = vec![
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
            ];
        } else if ["wochenends", "weekends"].contains(&w.as_str()) {
            r.weekdays = vec![Weekday::Sat, Weekday::Sun];
        }
        rule = Some(r);
        let range = c.get(0).unwrap().range();
        s.blank(range);
    }
    // "montags und mittwochs": allein oder zusätzlich zu "alle zwei Wochen"
    if let Some(c) = s.caps(&WEEKDAY_ADVERBS) {
        let days: Vec<Weekday> = c[1]
            .split([',', '&', ' '])
            .filter(|w| !w.is_empty() && !["und", "and"].contains(&w.to_lowercase().as_str()))
            .filter_map(weekday_of)
            .collect();
        match &mut rule {
            Some(r) if r.frequency == Frequency::Weekly && r.weekdays.is_empty() => {
                r.weekdays = days
            }
            Some(_) => {}
            None => {
                let mut r = make(Frequency::Weekly, 1);
                r.weekdays = days;
                rule = Some(r);
            }
        }
        let range = c.get(0).unwrap().range();
        s.blank(range);
    }
    if let Some(r) = &mut rule
        && let Some(c) = s.caps(&COUNT)
    {
        r.count = c[1].parse().ok().filter(|n| *n > 0);
        let range = c.get(0).unwrap().range();
        s.blank(range);
    }
    let _ = today;
    p.rule = rule;
}

fn casual_hour(word: &str) -> NaiveTime {
    let h = match word {
        w if w.starts_with("morgen") || w.starts_with("vormittag") || w == "morning" => 9,
        w if w.starts_with("nachmittag") || w == "afternoon" => 15,
        w if w.starts_with("mittag") || w == "noon" || w == "midday" => 12,
        w if w.starts_with("abend") || w == "evening" => 19,
        "tonight" => 20,
        w if w.starts_with("nacht") || w == "night" => 22,
        _ => 0,
    };
    NaiveTime::from_hms_opt(h, 0, 0).unwrap_or_default()
}

fn parse_times(p: &mut Parsed, s: &mut Scanner) {
    if let Some(c) = s.caps(&TIME_BETWEEN)
        && let (Some(a), Some(b)) = (time_from(&c, 1), time_from(&c, 5))
    {
        p.start = Some(a.time);
        p.end = Some(b.time);
        let r = c.get(0).unwrap().range();
        s.blank(r);
        return;
    }
    // Bereiche: "8 Uhr bis 9:30", "15-16 Uhr", "von 8 bis 10", "3pm to 5pm"
    for c in s.all(&TIME_RANGE) {
        let (Some(mut a), Some(b)) = (time_from(&c, 2), time_from(&c, 6)) else {
            continue;
        };
        if !(a.marked || b.marked || c.get(1).is_some()) {
            continue;
        }
        // "3 to 5pm": am/pm des Endes gilt auch für den Beginn
        if c.get(4).is_none()
            && let Some(ampm) = c.get(8)
            && let Some(t) = clock(
                c[2].parse().unwrap_or(0),
                c.get(3).map_or(0, |m| m.as_str().parse().unwrap_or(0)),
                Some(ampm.as_str()),
            )
            && t < b.time
        {
            a.time = t;
        }
        p.start = Some(a.time);
        p.end = Some(b.time);
        let r = c.get(0).unwrap().range();
        s.blank(r);
        return;
    }
    if let Some(c) = s.caps(&HALF) {
        let h: u32 = c[2].parse().unwrap_or(0);
        let word = c[1].to_lowercase();
        let (h, m) = match word.as_str() {
            "halb" => (h + 23, 30),
            "viertel nach" => (h, 15),
            "viertel vor" | "dreiviertel" => (h + 23, 45),
            _ => (h, 0),
        };
        if let Some(t) = NaiveTime::from_hms_opt(h % 24, m, 0) {
            p.start = Some(t);
            let r = c.get(0).unwrap().range();
            s.blank(r);
            return;
        }
    }
    for c in s.all(&TIME_SINGLE) {
        let Some(t) = time_from(&c, 2) else {
            continue;
        };
        if !(t.marked || c.get(1).is_some()) {
            continue;
        }
        // "am 12." oder "12.11." sind Datumsangaben, keine Uhrzeit
        let whole = c.get(0).unwrap();
        let after = &s.text[whole.end()..];
        if after.starts_with('.')
            || (c.get(3).is_some() && c.get(1).is_none() && after.starts_with(char::is_numeric))
        {
            continue;
        }
        if c.get(3).is_some()
            && whole.as_str().contains('.')
            && after
                .trim_start()
                .starts_with(|ch: char| ch.is_ascii_digit())
        {
            continue;
        }
        p.start = Some(t.time);
        let r = whole.range();
        s.blank(r);
        return;
    }
    if p.start.is_none()
        && let Some(c) = s.caps(&CASUAL_TIME)
    {
        let word = c[1].to_lowercase();
        p.start = Some(casual_hour(&word));
        let range = c.get(1).unwrap().range();
        // "heute abend": "heute" bleibt für das Datum stehen
        s.blank(range);
    }
}

fn add_months(day: NaiveDate, months: u32) -> NaiveDate {
    day.checked_add_months(chrono::Months::new(months))
        .unwrap_or(day)
}

/// Nächster solcher Wochentag ab heute (heute zählt mit, außer bei "nächsten")
fn next_weekday(today: NaiveDate, day: Weekday, strictly_after: bool) -> NaiveDate {
    let mut d = if strictly_after {
        today + Duration::days(1)
    } else {
        today
    };
    while d.weekday() != day {
        d += Duration::days(1);
    }
    d
}

/// Jahr ergänzen: ein Datum ohne Jahr, das schon vorbei ist, meint das nächste
fn with_year(day: u32, month: u32, year: Option<i32>, today: NaiveDate) -> Option<NaiveDate> {
    match year {
        Some(y) => NaiveDate::from_ymd_opt(if y < 100 { 2000 + y } else { y }, month, day),
        None => {
            let this = NaiveDate::from_ymd_opt(today.year(), month, day)?;
            if this < today {
                NaiveDate::from_ymd_opt(today.year() + 1, month, day)
            } else {
                Some(this)
            }
        }
    }
}

/// Ein Datum an der ersten passenden Stelle; liefert Tag und Bereich
fn find_date(s: &Scanner, today: NaiveDate) -> Option<(NaiveDate, std::ops::Range<usize>)> {
    let mut found: Vec<(NaiveDate, std::ops::Range<usize>)> = Vec::new();
    if let Some(c) = s.caps(&ISO_DATE)
        && let Some(d) =
            NaiveDate::from_ymd_opt(c[1].parse().ok()?, c[2].parse().ok()?, c[3].parse().ok()?)
    {
        found.push((d, c.get(0).unwrap().range()));
    }
    for c in s.all(&NUMERIC_DATE) {
        let whole = c.get(0).unwrap();
        // "9.30 Uhr" ist eine Uhrzeit: ohne Punkt am Ende und ohne Jahr kein Datum
        if c.get(3).is_none() && !whole.as_str().ends_with('.') {
            continue;
        }
        let year = c.get(3).and_then(|y| y.as_str().parse().ok());
        if let (Ok(day), Ok(month)) = (c[1].parse(), c[2].parse())
            && let Some(d) = with_year(day, month, year, today)
        {
            found.push((d, whole.range()));
        }
    }
    if let Some(c) = s.caps(&DAY_MONTH_NAME)
        && let Some(month) = month_of(&c[2])
        && let Some(d) = with_year(
            c[1].parse().ok()?,
            month,
            c.get(3).and_then(|y| y.as_str().parse().ok()),
            today,
        )
    {
        found.push((d, c.get(0).unwrap().range()));
    }
    if let Some(c) = s.caps(&MONTH_NAME_DAY)
        && let Some(month) = month_of(&c[1])
        && let Some(d) = with_year(
            c[2].parse().ok()?,
            month,
            c.get(3).and_then(|y| y.as_str().parse().ok()),
            today,
        )
    {
        found.push((d, c.get(0).unwrap().range()));
    }
    if let Some(c) = s.caps(&RELATIVE_DAY) {
        let w = c[1].to_lowercase();
        let d = match w.as_str() {
            "heute" | "today" => today,
            "morgen" | "tomorrow" => today + Duration::days(1),
            _ => today + Duration::days(2),
        };
        found.push((d, c.get(0).unwrap().range()));
    }
    if let Some(c) = s.caps(&IN_UNITS)
        && let Some(n) = number_of(&c[1])
    {
        let unit = c[2].to_lowercase();
        let d = if unit.starts_with("tag") || unit.starts_with("day") {
            today + Duration::days(i64::from(n))
        } else if unit.starts_with("woche") || unit.starts_with("week") {
            today + Duration::days(7 * i64::from(n))
        } else if unit.starts_with("monat") || unit.starts_with("month") {
            add_months(today, n)
        } else {
            add_months(today, 12 * n)
        };
        found.push((d, c.get(0).unwrap().range()));
    }
    if let Some(c) = s.caps(&NEXT_WEEK) {
        let d = match c.get(1).map(|m| m.as_str().to_lowercase()) {
            Some(u) if u.starts_with("woche") || u == "week" => today + Duration::days(7),
            Some(_) => add_months(today, 1),
            None => next_weekday(today, Weekday::Sat, false),
        };
        found.push((d, c.get(0).unwrap().range()));
    }
    for c in s.all(&WEEKDAY_DATE) {
        let Some(day) = weekday_of(&c[2]) else {
            continue;
        };
        let modifier = c.get(1).map(|m| m.as_str().to_lowercase());
        let d = match modifier.as_deref() {
            Some("nächsten") | Some("nächster") | Some("next") => next_weekday(today, day, true),
            Some("übernächsten") => next_weekday(today, day, true) + Duration::days(7),
            _ => next_weekday(today, day, false),
        };
        let m = c.get(0).unwrap();
        let end = m.end() - m.as_str().len() + m.as_str().trim_end_matches([' ', ',']).len();
        found.push((d, m.start()..end));
        break;
    }
    if let Some(c) = s.caps(&WEEKDAY_SHORT_AFTER_AM)
        && let Some(day) = weekday_of(&c[2])
    {
        found.push((next_weekday(today, day, false), c.get(2).unwrap().range()));
    }
    if let Some(c) = s.caps(&DAY_ONLY)
        && let Ok(day) = c[1].parse::<u32>()
    {
        let this = NaiveDate::from_ymd_opt(today.year(), today.month(), day);
        let d = match this {
            Some(d) if d >= today => Some(d),
            _ => {
                let next = add_months(today.with_day(1).unwrap_or(today), 1);
                NaiveDate::from_ymd_opt(next.year(), next.month(), day)
            }
        };
        if let Some(d) = d {
            let m = c.get(0).unwrap();
            let end = m.end() - m.as_str().len() + m.as_str().trim_end_matches([' ', ',']).len();
            found.push((d, m.start()..end));
        }
    }
    // Die früheste Stelle im Text gewinnt
    found.into_iter().min_by_key(|(_, r)| r.start)
}

fn parse_dates(p: &mut Parsed, s: &mut Scanner, today: NaiveDate) {
    // Bis zu drei Daten: z. B. "vom 3.5. bis 5.5." oder "ab Montag bis 30.6."
    let mut dates: Vec<(NaiveDate, std::ops::Range<usize>, bool, bool)> = Vec::new();
    for _ in 0..3 {
        let Some((d, range)) = find_date(s, today) else {
            break;
        };
        let before = &s.text[..range.start];
        let until = UNTIL_PREFIX.find(before).map(|m| m.start());
        let from = FROM_PREFIX.find(before).map(|m| m.start());
        let start = until.or(from).unwrap_or(range.start);
        dates.push((d, range.clone(), until.is_some(), from.is_some()));
        s.blank(start..range.end);
    }
    for (d, _, is_until, _) in dates {
        if is_until {
            match (&mut p.rule, p.task) {
                (Some(r), _) => r.until = Some(d),
                (None, true) => p.date = Some(d),
                (None, false) if p.date.is_some() => p.end_date = Some(d),
                (None, false) => p.date = Some(d),
            }
        } else if p.date.is_none() {
            p.date = Some(d);
        } else if p.end_date.is_none() && p.rule.is_none() {
            p.end_date = Some(d);
        }
    }
}

impl Parsed {
    /// Zeiten festlegen. Ohne Datum: heute, ist die Uhrzeit schon vorbei,
    /// morgen; bei Wiederholungen der erste passende Tag.
    pub fn when(&self, now: NaiveDateTime) -> When {
        let today = now.date();
        let first_rule_day = self.rule.as_ref().and_then(|r| {
            let days = &r.weekdays;
            (!days.is_empty()).then(|| {
                days.iter()
                    .map(|d| next_weekday(today, *d, false))
                    .min()
                    .unwrap_or(today)
            })
        });
        let day = self.date.or(first_rule_day);
        if self.task {
            return match (day, self.start) {
                (None, None) => When::Undated,
                (None, Some(t)) => When::Due {
                    day: if t <= now.time() {
                        today + Duration::days(1)
                    } else {
                        today
                    },
                    time: Some(t),
                },
                (Some(d), t) => When::Due { day: d, time: t },
            };
        }
        match (self.start, self.all_day) {
            (Some(t), false) => {
                let day = day.unwrap_or(if t <= now.time() && self.rule.is_none() {
                    today + Duration::days(1)
                } else {
                    today
                });
                let start = day.and_time(t);
                let end = match (self.end, self.duration) {
                    (Some(e), _) => {
                        let end = day.and_time(e);
                        if end <= start {
                            end + Duration::days(1)
                        } else {
                            end
                        }
                    }
                    (None, Some(d)) => start + d,
                    (None, None) => start + Duration::minutes(DEFAULT_MINUTES),
                };
                When::Timed { start, end }
            }
            _ => {
                let first = day.unwrap_or(today);
                let last = self.end_date.filter(|e| *e >= first).unwrap_or(first);
                When::AllDay { first, last }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Donnerstag, 5. November 2026, 10:00
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 5).unwrap()
    }

    fn now() -> NaiveDateTime {
        today().and_hms_opt(10, 0, 0).unwrap()
    }

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn timed(text: &str) -> (String, NaiveDateTime, NaiveDateTime) {
        let p = parse(text, today());
        match p.when(now()) {
            When::Timed { start, end } => (p.title, start, end),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn at(m: u32, day: u32, h: u32, min: u32) -> NaiveDateTime {
        d(m, day).and_time(t(h, min))
    }

    #[test]
    fn the_examples_from_the_request() {
        let p = parse("Mathe jeden Montag 8 Uhr bis 9:30 Raum 204", today());
        assert_eq!(p.title, "Mathe");
        assert_eq!(p.location.as_deref(), Some("Raum 204"));
        assert_eq!((p.start, p.end), (Some(t(8, 0)), Some(t(9, 30))));
        let rule = p.rule.clone().unwrap();
        assert_eq!(
            (rule.frequency, rule.interval, rule.weekdays.clone()),
            (Frequency::Weekly, 1, vec![Weekday::Mon])
        );
        assert_eq!(
            p.when(now()),
            When::Timed {
                start: at(11, 9, 8, 0),
                end: at(11, 9, 9, 30)
            }
        );

        assert_eq!(
            timed("Zahnarzt morgen 15 Uhr"),
            ("Zahnarzt".into(), at(11, 6, 15, 0), at(11, 6, 16, 0))
        );
    }

    #[test]
    fn german_dates_and_times() {
        assert_eq!(
            timed("Friseur am Freitag um 14:30"),
            ("Friseur".into(), at(11, 6, 14, 30), at(11, 6, 15, 30))
        );
        assert_eq!(
            timed("Elternabend 12.11. 19-21 Uhr"),
            ("Elternabend".into(), at(11, 12, 19, 0), at(11, 12, 21, 0))
        );
        assert_eq!(
            timed("Besprechung übermorgen von 10 bis 11:15"),
            ("Besprechung".into(), at(11, 7, 10, 0), at(11, 7, 11, 15))
        );
        assert_eq!(
            timed("Kino nächsten Donnerstag 20 Uhr"),
            ("Kino".into(), at(11, 12, 20, 0), at(11, 12, 21, 0))
        );
        assert_eq!(
            timed("Laufen um halb 8"),
            ("Laufen".into(), at(11, 6, 7, 30), at(11, 6, 8, 30)),
            "8 Uhr ist vorbei: morgen"
        );
        assert_eq!(
            timed("Telefonat 9.30 Uhr für 30 Minuten").1,
            at(11, 6, 9, 30)
        );
        assert_eq!(
            timed("Telefonat 9.30 Uhr für 30 Minuten").2,
            at(11, 6, 10, 0)
        );
        assert_eq!(
            timed("Workshop 3. Dezember 9 Uhr 2 Stunden"),
            ("Workshop".into(), at(12, 3, 9, 0), at(12, 3, 11, 0))
        );
        assert_eq!(timed("Konzert heute abend").1, at(11, 5, 19, 0));
        assert_eq!(
            timed("Nachtschicht morgen 22 bis 6 Uhr").2,
            at(11, 7, 6, 0),
            "Ende am nächsten Tag"
        );
        assert_eq!(timed("Arzt in 3 Tagen um 8").1, at(11, 8, 8, 0));
    }

    #[test]
    fn english_dates_and_times() {
        assert_eq!(
            timed("Lunch with Anna tomorrow at 1pm"),
            ("Lunch with Anna".into(), at(11, 6, 13, 0), at(11, 6, 14, 0))
        );
        assert_eq!(
            timed("Call next monday 3 to 5pm"),
            ("Call".into(), at(11, 9, 15, 0), at(11, 9, 17, 0))
        );
        assert_eq!(timed("Dentist November 20th 9:15am").1, at(11, 20, 9, 15));
        assert_eq!(timed("Review in 2 weeks at 10").1, at(11, 19, 10, 0));
        assert_eq!(timed("Meeting between 2pm and 3:30pm").2, at(11, 5, 15, 30));
    }

    #[test]
    fn all_day_and_multi_day() {
        let p = parse("Kurzurlaub vom 20.11. bis 23.11.", today());
        assert_eq!(p.title, "Kurzurlaub");
        assert_eq!(
            p.when(now()),
            When::AllDay {
                first: d(11, 20),
                last: d(11, 23)
            }
        );
        let p = parse("Geburtstag Oma 12. Januar", today());
        assert_eq!(p.title, "Geburtstag Oma");
        assert_eq!(
            p.when(now()),
            When::AllDay {
                first: NaiveDate::from_ymd_opt(2027, 1, 12).unwrap(),
                last: NaiveDate::from_ymd_opt(2027, 1, 12).unwrap()
            }
        );
        let p = parse("Messe morgen ganztägig", today());
        assert_eq!(
            p.when(now()),
            When::AllDay {
                first: d(11, 6),
                last: d(11, 6)
            }
        );
        let p = parse("Brainstorming", today());
        assert_eq!(
            (p.title.as_str(), p.when(now())),
            (
                "Brainstorming",
                When::AllDay {
                    first: d(11, 5),
                    last: d(11, 5)
                }
            )
        );
    }

    #[test]
    fn recurrence_forms() {
        let rule = |text: &str| parse(text, today()).rule.unwrap();
        let r = rule("Chor dienstags und donnerstags 19 Uhr");
        assert_eq!(r.weekdays, vec![Weekday::Tue, Weekday::Thu]);
        let r = rule("Team-Call alle zwei Wochen montags 9 Uhr");
        assert_eq!(
            (r.frequency, r.interval, r.weekdays.clone()),
            (Frequency::Weekly, 2, vec![Weekday::Mon])
        );
        assert_eq!(rule("Vitamine täglich 8 Uhr").frequency, Frequency::Daily);
        assert_eq!(rule("Miete jeden Monat").frequency, Frequency::Monthly);
        assert_eq!(rule("Standup werktags 9:30").weekdays.len(), 5);
        let r = rule("Kurs jeden Mittwoch 18 Uhr bis 30.6.2027");
        assert_eq!(r.until, NaiveDate::from_ymd_opt(2027, 6, 30));
        assert_eq!(rule("Physio wöchentlich 10 mal").count, Some(10));
        assert_eq!(parse("Physio wöchentlich 10 mal", today()).title, "Physio");
        let r = rule("Gym every other monday 7am");
        assert_eq!((r.interval, r.weekdays.clone()), (2, vec![Weekday::Mon]));
        assert_eq!(rule("Stretching every day").frequency, Frequency::Daily);
        let p = parse("Joggen jeden Morgen", today());
        assert_eq!(
            (p.title.as_str(), p.start, p.rule.unwrap().frequency),
            ("Joggen", Some(t(9, 0)), Frequency::Daily)
        );
        // Erster Termin einer Serie mit Wochentag: der nächste solche Tag
        assert_eq!(
            parse("Chor dienstags 19 Uhr", today()).when(now()),
            When::Timed {
                start: at(11, 10, 19, 0),
                end: at(11, 10, 20, 0)
            }
        );
        // Die Regel als RRULE für den Dialog
        assert_eq!(rule("Chor dienstags").simple().weekdays, vec![Weekday::Tue]);
    }

    #[test]
    fn locations() {
        let p = parse("Zahnarzt bei Dr. Weber morgen 15 Uhr", today());
        assert_eq!(
            (p.title.as_str(), p.location.as_deref()),
            ("Zahnarzt", Some("Dr. Weber"))
        );
        let p = parse("Essen mit Anna Freitag 19 Uhr im Café Rot", today());
        assert_eq!(
            (p.title.as_str(), p.location.as_deref()),
            ("Essen mit Anna", Some("Café Rot"))
        );
        let p = parse("Lunch at Joe's tomorrow 12:30", today());
        assert_eq!(
            (p.title.as_str(), p.location.as_deref()),
            ("Lunch", Some("Joe's"))
        );
        let p = parse("Lesung @ Stadtbibliothek", today());
        assert_eq!(p.location.as_deref(), Some("Stadtbibliothek"));
        // Zahlen im Titel sind keine Uhrzeit
        let p = parse("3 Kartons kaufen", today());
        assert_eq!((p.title.as_str(), p.start), ("3 Kartons kaufen", None));
    }

    #[test]
    fn odd_input_does_not_break() {
        for text in [
            "",
            "   ",
            "31.02. Termin",
            "99 Uhr",
            "25:61",
            "Treffen 24 Uhr",
            "🎉 Party morgen 20 Uhr 🎂",
            "STRAẞENFEST am Samstag",
            "bis",
            "am",
            "#",
            "@",
            "jeden",
            "alle 0 Wochen",
            "in 999 Jahren",
            "1.1.1",
            "Raum",
            "um halb 25",
            "vom bis",
            "Montag Dienstag Mittwoch",
            "x".repeat(2000).as_str(),
        ] {
            let p = parse(text, today());
            let _ = p.when(now());
            if let Some(rule) = &p.rule {
                let _ = rule.simple().to_rrule(
                    &crate::time::DateTimeValue::Date(today()),
                    chrono_tz::Europe::Berlin,
                );
            }
        }
        assert_eq!(
            parse("STRAẞENFEST am Samstag", today()).title,
            "STRAẞENFEST"
        );
        assert_eq!(
            parse("🎉 Party morgen 20 Uhr 🎂", today()).title,
            "🎉 Party 🎂"
        );
    }

    #[test]
    fn tasks_calendars_and_reminders() {
        let p = parse("todo Milch kaufen bis Freitag", today());
        assert!(p.task);
        assert_eq!(p.title, "Milch kaufen");
        assert_eq!(
            p.when(now()),
            When::Due {
                day: d(11, 6),
                time: None
            }
        );
        let p = parse("Aufgabe: Steuererklärung bis 31.12. 18 Uhr", today());
        assert_eq!(
            (p.title.as_str(), p.when(now())),
            (
                "Steuererklärung",
                When::Due {
                    day: d(12, 31),
                    time: Some(t(18, 0))
                }
            )
        );
        let p = parse("remind me to call mom", today());
        assert_eq!(
            (p.task, p.title.as_str(), p.when(now())),
            (true, "Call mom", When::Undated)
        );
        let p = parse(
            "Planung #Arbeit morgen 10 Uhr Erinnerung 30 min vorher",
            today(),
        );
        assert_eq!(
            (p.title.as_str(), p.calendar.as_deref(), p.reminder),
            ("Planung", Some("Arbeit"), Some(30))
        );
        let p = parse("Flug Freitag 6:40 remind 1 hour before", today());
        assert_eq!((p.title.as_str(), p.reminder), ("Flug", Some(60)));
    }
}
