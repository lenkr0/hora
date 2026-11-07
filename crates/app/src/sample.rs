//! Beispieltermine für den Start ohne Konto, in der App-Sprache.

use chrono::{Days, NaiveDate, NaiveDateTime, NaiveTime};
use hora_i18n::tr;

#[derive(Debug, Clone)]
pub struct Event {
    pub id: i32,
    pub title: String,
    pub location: String,
    pub color: u32,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
}

#[derive(Debug, Clone)]
pub struct AllDay {
    pub id: i32,
    pub title: String,
    pub color: u32,
    pub first: NaiveDate,
    /// Tag nach dem letzten Tag, wie DTEND bei ganztägigen Terminen
    pub end: NaiveDate,
}

const ARBEIT: u32 = 0x4f7cac;
const PRIVAT: u32 = 0x6a9a6b;
const SCHULE: u32 = 0x8a6fb0;
const SPORT: u32 = 0xc49a3a;
const FAMILIE: u32 = 0xb8645a;

struct Generator {
    next_id: i32,
    events: Vec<Event>,
    all_day: Vec<AllDay>,
}

impl Generator {
    #[allow(clippy::too_many_arguments)]
    fn timed(
        &mut self,
        monday: NaiveDate,
        day: u64,
        from: (u32, u32),
        to: (u32, u32),
        title: &str,
        location: &str,
        color: u32,
    ) {
        let date = monday + Days::new(day);
        let at = |(h, m): (u32, u32)| {
            date.and_time(NaiveTime::from_hms_opt(h, m, 0).expect("gültige Uhrzeit"))
        };
        self.events.push(Event {
            id: self.next_id,
            title: title.into(),
            location: location.into(),
            color,
            start: at(from),
            end: at(to),
        });
        self.next_id += 1;
    }

    fn all_day(&mut self, monday: NaiveDate, first: u64, days: u64, title: &str, color: u32) {
        self.all_day.push(AllDay {
            id: self.next_id,
            title: title.into(),
            color,
            first: monday + Days::new(first),
            end: monday + Days::new(first + days),
        });
        self.next_id += 1;
    }
}

/// Termine für die Woche ab `monday` und je zwei Wochen davor und danach.
pub fn generate(monday: NaiveDate) -> (Vec<Event>, Vec<AllDay>, i32) {
    let mut g = Generator {
        next_id: 1,
        events: Vec::new(),
        all_day: Vec::new(),
    };

    for offset in -2i64..=2 {
        let week = if offset < 0 {
            monday - Days::new(offset.unsigned_abs() * 7)
        } else {
            monday + Days::new(offset as u64 * 7)
        };
        // Jede Woche
        g.timed(
            week,
            0,
            (8, 0),
            (9, 30),
            tr!("Math"),
            tr!("Room 204"),
            SCHULE,
        );
        g.timed(
            week,
            2,
            (8, 0),
            (9, 30),
            tr!("Physics"),
            tr!("Room 112"),
            SCHULE,
        );
        g.timed(
            week,
            3,
            (7, 30),
            (8, 15),
            tr!("Running"),
            tr!("City park"),
            SPORT,
        );
        g.timed(week, 4, (13, 0), (14, 0), tr!("Weekly review"), "", ARBEIT);
        if offset != 0 {
            continue;
        }
        // Nur in der aktuellen Woche: dichter, mit Überschneidungen
        g.timed(
            week,
            0,
            (10, 0),
            (11, 0),
            tr!("Team standup"),
            tr!("Meeting room 2"),
            ARBEIT,
        );
        g.timed(
            week,
            0,
            (12, 30),
            (13, 15),
            tr!("Lunch with Anna"),
            tr!("Cafeteria"),
            PRIVAT,
        );
        g.timed(
            week,
            1,
            (9, 0),
            (10, 0),
            tr!("Dentist"),
            tr!("Dr. Weber's office"),
            PRIVAT,
        );
        g.timed(
            week,
            1,
            (9, 30),
            (11, 0),
            tr!("Project meeting"),
            tr!("Online"),
            ARBEIT,
        );
        g.timed(
            week,
            1,
            (16, 0),
            (16, 30),
            tr!("Pick up parcel"),
            "",
            PRIVAT,
        );
        g.timed(week, 2, (14, 0), (16, 0), tr!("Focus time"), "", ARBEIT);
        g.timed(
            week,
            2,
            (14, 30),
            (15, 30),
            tr!("Draft review"),
            tr!("Online"),
            ARBEIT,
        );
        g.timed(
            week,
            2,
            (15, 0),
            (15, 45),
            tr!("Call back insurance"),
            "",
            PRIVAT,
        );
        g.timed(week, 3, (15, 0), (15, 30), tr!("Call landlord"), "", PRIVAT);
        g.timed(
            week,
            3,
            (19, 0),
            (20, 30),
            tr!("Choir rehearsal"),
            tr!("Community hall"),
            PRIVAT,
        );
        g.timed(
            week,
            4,
            (18, 0),
            (21, 0),
            tr!("Movies"),
            tr!("Film Palace"),
            FAMILIE,
        );
        g.timed(
            week,
            5,
            (10, 0),
            (12, 0),
            tr!("Farmers' market"),
            tr!("Market square"),
            PRIVAT,
        );
        g.timed(week, 6, (16, 0), (17, 0), tr!("Call family"), "", FAMILIE);
        g.all_day(week, 0, 3, tr!("Project week"), ARBEIT);
        g.all_day(week, 1, 1, tr!("Trash collection"), PRIVAT);
        g.all_day(week, 4, 1, tr!("Grandma's birthday"), FAMILIE);
    }

    let next_id = g.next_id;
    (g.events, g.all_day, next_id)
}
