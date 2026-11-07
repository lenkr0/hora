//! Datums- und Zeitwerte aus iCalendar mit Zeitzonen.
//!
//! TZID-Werte werden auf IANA-Zonen (chrono-tz) abgebildet. Neben echten
//! IANA-Namen werden Präfixe wie `/mozilla.org/20050126_1/Europe/Berlin`,
//! Windows-Namen wie `W. Europe Standard Time` (Outlook/Exchange) und
//! `X-LIC-LOCATION` aus der mitgelieferten VTIMEZONE erkannt. Unbekannte
//! Zonen werden wie schwebende Zeit in der eigenen Zeitzone behandelt.

use chrono::{
    DateTime, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, Offset as _, TimeZone,
    Utc,
};
use chrono_tz::Tz;

use crate::ical::{Component, Property};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimeValue {
    /// Ganztägig (`VALUE=DATE`)
    Date(NaiveDate),
    /// Ohne Zeitzone: gilt in der jeweils eigenen Zeitzone
    Floating(NaiveDateTime),
    Utc(NaiveDateTime),
    Zoned(NaiveDateTime, Tz),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", hora_i18n::tr!("invalid date/time value: {}", .0))]
pub struct TimeError(pub String);

impl DateTimeValue {
    /// Liest DTSTART, DTEND, DUE, RECURRENCE-ID usw. `calendar` liefert die
    /// VTIMEZONE-Definitionen für unbekannte TZID-Werte.
    pub fn from_property(
        property: &Property,
        calendar: Option<&Component>,
    ) -> Result<Self, TimeError> {
        let date_only = property
            .param("VALUE")
            .is_some_and(|v| v.eq_ignore_ascii_case("DATE"));
        let tz = property
            .param("TZID")
            .and_then(|id| resolve_tzid_in(id, calendar));
        parse_value(property.value.trim(), tz, date_only)
    }

    pub fn is_date(&self) -> bool {
        matches!(self, DateTimeValue::Date(_))
    }

    /// Wandzeit am Ort des Termins (bei ganztägigen Terminen Mitternacht).
    pub fn naive(&self) -> NaiveDateTime {
        match *self {
            DateTimeValue::Date(d) => d.and_time(NaiveTime::MIN),
            DateTimeValue::Floating(n) | DateTimeValue::Utc(n) | DateTimeValue::Zoned(n, _) => n,
        }
    }

    /// Derselbe Wert-Typ (ganztägig, schwebend, UTC, Zone) zu einem neuen
    /// Zeitpunkt. So behält ein verschobener Termin seine Zeitzone: ein Call
    /// in New York bleibt in New-York-Zeit gespeichert.
    pub fn moved_to(&self, instant: DateTime<Utc>, home: Tz) -> DateTimeValue {
        match *self {
            DateTimeValue::Date(_) => {
                DateTimeValue::Date(instant.with_timezone(&home).date_naive())
            }
            DateTimeValue::Floating(_) => {
                DateTimeValue::Floating(instant.with_timezone(&home).naive_local())
            }
            DateTimeValue::Utc(_) => DateTimeValue::Utc(instant.naive_utc()),
            DateTimeValue::Zoned(_, tz) => {
                DateTimeValue::Zoned(instant.with_timezone(&tz).naive_local(), tz)
            }
        }
    }

    /// Zeitpunkt in UTC. Ganztägige und schwebende Werte gelten in `home`.
    pub fn to_utc(&self, home: Tz) -> DateTime<Utc> {
        match *self {
            DateTimeValue::Utc(n) => Utc.from_utc_datetime(&n),
            DateTimeValue::Zoned(n, tz) => local_to_utc(n, tz),
            DateTimeValue::Date(_) | DateTimeValue::Floating(_) => local_to_utc(self.naive(), home),
        }
    }

    /// Zeitzone, in der der Wert gemeint ist (`home` für ganztägig/schwebend).
    pub fn zone(&self, home: Tz) -> Tz {
        match *self {
            DateTimeValue::Utc(_) => Tz::UTC,
            DateTimeValue::Zoned(_, tz) => tz,
            DateTimeValue::Date(_) | DateTimeValue::Floating(_) => home,
        }
    }
}

/// Liest einen DATE- oder DATE-TIME-Wert. Ohne `VALUE=DATE` werden auch
/// achtstellige Werte als Datum erkannt, weil manche Clients den Parameter
/// weglassen.
pub fn parse_value(
    value: &str,
    tz: Option<Tz>,
    date_only: bool,
) -> Result<DateTimeValue, TimeError> {
    let err = || TimeError(value.to_string());
    if date_only || value.len() == 8 {
        let date =
            NaiveDate::parse_from_str(&value[..value.len().min(8)], "%Y%m%d").map_err(|_| err())?;
        return Ok(DateTimeValue::Date(date));
    }
    let (body, utc) = match value.strip_suffix(['Z', 'z']) {
        Some(body) => (body, true),
        None => (value, false),
    };
    let naive = NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M%S").map_err(|_| err())?;
    Ok(match (utc, tz) {
        (true, _) => DateTimeValue::Utc(naive),
        (false, Some(tz)) => DateTimeValue::Zoned(naive, tz),
        (false, None) => DateTimeValue::Floating(naive),
    })
}

/// Lokale Wandzeit in UTC nach RFC 5545, Abschnitt 3.3.5: in der doppelten
/// Stunde (Ende der Sommerzeit) gilt das erste Vorkommen, in der Lücke
/// (Beginn der Sommerzeit) der UTC-Versatz vor der Lücke. 02:30 am Tag der
/// Umstellung auf Sommerzeit wird so zu 03:30 Sommerzeit.
pub fn local_to_utc(naive: NaiveDateTime, tz: Tz) -> DateTime<Utc> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt.with_timezone(&Utc),
        LocalResult::Ambiguous(first, _) => first.with_timezone(&Utc),
        LocalResult::None => {
            let before = tz
                .from_local_datetime(&(naive - Duration::hours(6)))
                .earliest()
                .map(|dt| dt.offset().fix().local_minus_utc())
                .unwrap_or(0);
            Utc.from_utc_datetime(&(naive - Duration::seconds(i64::from(before))))
        }
    }
}

/// TZID auf eine IANA-Zone abbilden, ohne VTIMEZONE.
pub fn resolve_tzid(tzid: &str) -> Option<Tz> {
    let tzid = tzid.trim().trim_matches('"');
    if let Ok(tz) = tzid.parse::<Tz>() {
        return Some(tz);
    }
    if let Some(tz) = windows_zone(tzid) {
        return Some(tz);
    }
    // Präfixe wie /mozilla.org/20050126_1/Europe/Berlin: hintere Teile probieren
    let parts: Vec<&str> = tzid.split('/').filter(|p| !p.is_empty()).collect();
    (1..=3.min(parts.len()))
        .rev()
        .find_map(|n| parts[parts.len() - n..].join("/").parse::<Tz>().ok())
}

/// TZID auflösen; hilft die Bezeichnung nicht, zählt `X-LIC-LOCATION` der
/// passenden VTIMEZONE.
pub fn resolve_tzid_in(tzid: &str, calendar: Option<&Component>) -> Option<Tz> {
    resolve_tzid(tzid).or_else(|| {
        calendar?
            .children_named("VTIMEZONE")
            .find(|vtz| {
                vtz.property("TZID")
                    .is_some_and(|p| p.value.trim() == tzid.trim())
            })
            .and_then(|vtz| vtz.property("X-LIC-LOCATION"))
            .and_then(|p| resolve_tzid(&p.value))
    })
}

/// Häufige Windows-Zeitzonennamen (CLDR windowsZones, Gebiet 001).
fn windows_zone(name: &str) -> Option<Tz> {
    let iana = match name {
        "W. Europe Standard Time" => "Europe/Berlin",
        "Central Europe Standard Time" => "Europe/Budapest",
        "Central European Standard Time" => "Europe/Warsaw",
        "Romance Standard Time" => "Europe/Paris",
        "GMT Standard Time" => "Europe/London",
        "Greenwich Standard Time" => "Atlantic/Reykjavik",
        "E. Europe Standard Time" => "Europe/Chisinau",
        "FLE Standard Time" => "Europe/Kiev",
        "GTB Standard Time" => "Europe/Bucharest",
        "Russian Standard Time" => "Europe/Moscow",
        "Turkey Standard Time" => "Europe/Istanbul",
        "Israel Standard Time" => "Asia/Jerusalem",
        "Egypt Standard Time" => "Africa/Cairo",
        "South Africa Standard Time" => "Africa/Johannesburg",
        "W. Central Africa Standard Time" => "Africa/Lagos",
        "Morocco Standard Time" => "Africa/Casablanca",
        "Arabian Standard Time" => "Asia/Dubai",
        "Iran Standard Time" => "Asia/Tehran",
        "Pakistan Standard Time" => "Asia/Karachi",
        "India Standard Time" => "Asia/Kolkata",
        "SE Asia Standard Time" => "Asia/Bangkok",
        "China Standard Time" => "Asia/Shanghai",
        "Singapore Standard Time" => "Asia/Singapore",
        "Taipei Standard Time" => "Asia/Taipei",
        "Tokyo Standard Time" => "Asia/Tokyo",
        "Korea Standard Time" => "Asia/Seoul",
        "W. Australia Standard Time" => "Australia/Perth",
        "Cen. Australia Standard Time" => "Australia/Adelaide",
        "E. Australia Standard Time" => "Australia/Brisbane",
        "AUS Eastern Standard Time" => "Australia/Sydney",
        "New Zealand Standard Time" => "Pacific/Auckland",
        "Hawaiian Standard Time" => "Pacific/Honolulu",
        "Alaskan Standard Time" => "America/Anchorage",
        "Pacific Standard Time" => "America/Los_Angeles",
        "US Mountain Standard Time" => "America/Phoenix",
        "Mountain Standard Time" => "America/Denver",
        "Central Standard Time" => "America/Chicago",
        "Canada Central Standard Time" => "America/Regina",
        "Central Standard Time (Mexico)" => "America/Mexico_City",
        "Eastern Standard Time" => "America/New_York",
        "SA Pacific Standard Time" => "America/Bogota",
        "Atlantic Standard Time" => "America/Halifax",
        "Newfoundland Standard Time" => "America/St_Johns",
        "E. South America Standard Time" => "America/Sao_Paulo",
        "Argentina Standard Time" => "America/Buenos_Aires",
        "UTC" | "Coordinated Universal Time" => "UTC",
        _ => return None,
    };
    iana.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ical::parse_calendar;
    use chrono_tz::Europe::Berlin;

    fn naive(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn utc(s: &str) -> DateTime<Utc> {
        Utc.from_utc_datetime(&naive(s))
    }

    #[test]
    fn parses_all_value_forms() {
        assert_eq!(
            parse_value("20261109", None, true).unwrap(),
            DateTimeValue::Date(NaiveDate::from_ymd_opt(2026, 11, 9).unwrap())
        );
        assert_eq!(
            parse_value("20261109T080000Z", Some(Berlin), false).unwrap(),
            DateTimeValue::Utc(naive("2026-11-09 08:00"))
        );
        assert_eq!(
            parse_value("20261109T080000", Some(Berlin), false).unwrap(),
            DateTimeValue::Zoned(naive("2026-11-09 08:00"), Berlin)
        );
        assert_eq!(
            parse_value("20261109T080000", None, false).unwrap(),
            DateTimeValue::Floating(naive("2026-11-09 08:00"))
        );
        assert!(parse_value("2026-11-09", None, false).is_err());
    }

    #[test]
    fn winter_and_summer_offsets() {
        // Winterzeit UTC+1, Sommerzeit UTC+2
        assert_eq!(
            local_to_utc(naive("2026-01-15 08:00"), Berlin),
            utc("2026-01-15 07:00")
        );
        assert_eq!(
            local_to_utc(naive("2026-07-15 08:00"), Berlin),
            utc("2026-07-15 06:00")
        );
    }

    #[test]
    fn spring_forward_gap_uses_offset_before_gap() {
        // 29.03.2026: 02:00 -> 03:00. 02:30 existiert nicht und wird 03:30 MESZ.
        assert_eq!(
            local_to_utc(naive("2026-03-29 02:30"), Berlin),
            utc("2026-03-29 01:30")
        );
        assert_eq!(
            local_to_utc(naive("2026-03-29 03:30"), Berlin),
            utc("2026-03-29 01:30")
        );
    }

    #[test]
    fn fall_back_overlap_uses_first_occurrence() {
        // 25.10.2026: 03:00 -> 02:00. 02:30 kommt zweimal vor, das erste Mal in MESZ.
        assert_eq!(
            local_to_utc(naive("2026-10-25 02:30"), Berlin),
            utc("2026-10-25 00:30")
        );
    }

    #[test]
    fn floating_and_dates_use_home_zone() {
        let tokyo: Tz = "Asia/Tokyo".parse().unwrap();
        let floating = DateTimeValue::Floating(naive("2026-11-09 08:00"));
        assert_eq!(floating.to_utc(Berlin), utc("2026-11-09 07:00"));
        assert_eq!(floating.to_utc(tokyo), utc("2026-11-08 23:00"));
        let date = DateTimeValue::Date(NaiveDate::from_ymd_opt(2026, 11, 9).unwrap());
        assert_eq!(date.to_utc(Berlin), utc("2026-11-08 23:00"));
    }

    #[test]
    fn moved_values_keep_their_kind() {
        let ny: Tz = "America/New_York".parse().unwrap();
        let instant = utc("2026-11-10 16:00");
        assert_eq!(
            DateTimeValue::Zoned(naive("2026-11-09 10:00"), ny).moved_to(instant, Berlin),
            DateTimeValue::Zoned(naive("2026-11-10 11:00"), ny)
        );
        assert_eq!(
            DateTimeValue::Floating(naive("2026-11-09 10:00")).moved_to(instant, Berlin),
            DateTimeValue::Floating(naive("2026-11-10 17:00"))
        );
        assert_eq!(
            DateTimeValue::Utc(naive("2026-11-09 10:00")).moved_to(instant, Berlin),
            DateTimeValue::Utc(naive("2026-11-10 16:00"))
        );
        assert_eq!(
            DateTimeValue::Date(NaiveDate::from_ymd_opt(2026, 11, 9).unwrap())
                .moved_to(instant, Berlin),
            DateTimeValue::Date(NaiveDate::from_ymd_opt(2026, 11, 10).unwrap())
        );
    }

    #[test]
    fn resolves_tzid_variants() {
        assert_eq!(resolve_tzid("Europe/Berlin"), Some(Berlin));
        assert_eq!(
            resolve_tzid("/mozilla.org/20050126_1/Europe/Berlin"),
            Some(Berlin)
        );
        assert_eq!(
            resolve_tzid("/freeassociation.sourceforge.net/Tzfile/Europe/Berlin"),
            Some(Berlin)
        );
        assert_eq!(resolve_tzid("W. Europe Standard Time"), Some(Berlin));
        assert_eq!(resolve_tzid("\"Europe/Berlin\""), Some(Berlin));
        assert_eq!(resolve_tzid("Mitteleuropa"), None);
    }

    #[test]
    fn resolves_custom_tzid_via_vtimezone() {
        let cal = parse_calendar(
            "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Mitteleuropa\r\nX-LIC-LOCATION:Europe/Berlin\r\nEND:VTIMEZONE\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        assert_eq!(resolve_tzid_in("Mitteleuropa", Some(&cal)), Some(Berlin));
        let mut prop = Property::new("DTSTART", "20261109T080000");
        prop.set_param("TZID", "Mitteleuropa");
        assert_eq!(
            DateTimeValue::from_property(&prop, Some(&cal)).unwrap(),
            DateTimeValue::Zoned(naive("2026-11-09 08:00"), Berlin)
        );
    }
}
