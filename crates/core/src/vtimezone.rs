//! VTIMEZONE-Komponenten aus der Zeitzonendatenbank (chrono-tz).
//!
//! RFC 5545 verlangt zu jeder TZID eine VTIMEZONE im selben Objekt. Hora
//! schreibt dafür die tatsächlichen Umstellungen der betroffenen Jahre als
//! einzelne STANDARD-/DAYLIGHT-Abschnitte; Clients mit eigener
//! Zeitzonendatenbank richten sich ohnehin nach der TZID.

use chrono::{DateTime, Duration, NaiveDate, Offset as _, TimeZone, Utc};
use chrono_tz::{OffsetComponents as _, OffsetName as _, Tz};

use crate::ical::{Component, Property};

fn offset_seconds(tz: Tz, at: DateTime<Utc>) -> i32 {
    tz.offset_from_utc_datetime(&at.naive_utc())
        .fix()
        .local_minus_utc()
}

/// `+0100`, `-0430`
fn format_offset(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let minutes = seconds.abs() / 60;
    format!("{sign}{:02}{:02}", minutes / 60, minutes % 60)
}

/// Alle Umstellungen in `[from, to)`: (Zeitpunkt, Versatz vorher, Versatz nachher).
fn transitions(tz: Tz, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<(DateTime<Utc>, i32, i32)> {
    let step = Duration::hours(6);
    let mut found = Vec::new();
    let mut at = from;
    let mut before = offset_seconds(tz, at);
    while at < to {
        let next = at + step;
        let after = offset_seconds(tz, next);
        if after != before {
            // Auf ganze Minuten eingrenzen: low hat noch den alten Versatz, high den neuen
            let minute = |m: i64| DateTime::from_timestamp(m * 60, 0).unwrap();
            let (mut low, mut high) = (at.timestamp() / 60, next.timestamp() / 60);
            while high - low > 1 {
                let mid = low + (high - low) / 2;
                if offset_seconds(tz, minute(mid)) == before {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            found.push((minute(high), before, after));
            before = after;
        }
        at = next;
    }
    found
}

fn observance(kind: &str, local_start: &str, from: i32, to: i32, name: Option<&str>) -> Component {
    let mut c = Component::new(kind);
    c.properties
        .push(Property::new("TZOFFSETFROM", format_offset(from)));
    c.properties
        .push(Property::new("TZOFFSETTO", format_offset(to)));
    if let Some(name) = name {
        c.properties.push(Property::new("TZNAME", name));
    }
    c.properties.push(Property::new("DTSTART", local_start));
    c
}

/// VTIMEZONE für `tz` mit allen Umstellungen der Jahre `from_year..=to_year`.
pub fn vtimezone(tz: Tz, from_year: i32, to_year: i32) -> Component {
    let mut component = Component::new("VTIMEZONE");
    component.properties.push(Property::new("TZID", tz.name()));
    component
        .properties
        .push(Property::new("X-LIC-LOCATION", tz.name()));

    let year_start = |y: i32| {
        Utc.from_utc_datetime(
            &NaiveDate::from_ymd_opt(y, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        )
    };
    let changes = transitions(tz, year_start(from_year), year_start(to_year + 1));
    if changes.is_empty() {
        let at = year_start(from_year);
        let offset = offset_seconds(tz, at);
        let name = tz.offset_from_utc_datetime(&at.naive_utc());
        component.components.push(observance(
            "STANDARD",
            "19700101T000000",
            offset,
            offset,
            name.abbreviation(),
        ));
        return component;
    }
    for (at, before, after) in changes {
        let info = tz.offset_from_utc_datetime(&at.naive_utc());
        let kind = if info.dst_offset().is_zero() {
            "STANDARD"
        } else {
            "DAYLIGHT"
        };
        // DTSTART ist die Ortszeit vor der Umstellung
        let local = (at + Duration::seconds(i64::from(before)))
            .naive_utc()
            .format("%Y%m%dT%H%M%S")
            .to_string();
        component
            .components
            .push(observance(kind, &local, before, after, info.abbreviation()));
    }
    component
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ical::serialize;
    use chrono_tz::Europe::Berlin;

    #[test]
    fn berlin_has_daylight_and_standard_transitions() {
        let vtz = vtimezone(Berlin, 2026, 2026);
        assert_eq!(vtz.text_of("TZID"), "Europe/Berlin");
        assert_eq!(vtz.components.len(), 2);
        let spring = &vtz.components[0];
        assert!(spring.is("DAYLIGHT"));
        assert_eq!(spring.text_of("DTSTART"), "20260329T020000");
        assert_eq!(spring.text_of("TZOFFSETFROM"), "+0100");
        assert_eq!(spring.text_of("TZOFFSETTO"), "+0200");
        assert_eq!(spring.text_of("TZNAME"), "CEST");
        let autumn = &vtz.components[1];
        assert!(autumn.is("STANDARD"));
        assert_eq!(autumn.text_of("DTSTART"), "20261025T030000");
        assert_eq!(autumn.text_of("TZOFFSETTO"), "+0100");
        assert_eq!(autumn.text_of("TZNAME"), "CET");
    }

    #[test]
    fn zone_without_dst_gets_one_standard_block() {
        let tokyo: Tz = "Asia/Tokyo".parse().unwrap();
        let vtz = vtimezone(tokyo, 2026, 2027);
        assert_eq!(vtz.components.len(), 1);
        assert_eq!(vtz.components[0].text_of("TZOFFSETTO"), "+0900");
    }

    #[test]
    fn negative_and_half_hour_offsets() {
        assert_eq!(format_offset(-4 * 3600 - 1800), "-0430");
        assert_eq!(format_offset(5 * 3600 + 1800), "+0530");
        let ny: Tz = "America/New_York".parse().unwrap();
        let out = serialize(&vtimezone(ny, 2026, 2026));
        assert!(out.contains("TZOFFSETFROM:-0500\r\nTZOFFSETTO:-0400"));
        assert!(out.contains("DTSTART:20260308T020000"));
    }
}
