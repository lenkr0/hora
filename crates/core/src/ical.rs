//! Verlustfreies Lesen und Schreiben von iCalendar-Daten (RFC 5545).
//!
//! Jede Eigenschaft bleibt mit Namen, Parametern und unverändertem Rohwert
//! erhalten, auch wenn Hora sie nicht kennt (X-Eigenschaften, VALARM,
//! VTIMEZONE, Eigenschaften anderer Clients). Beim Bearbeiten werden nur
//! gezielt einzelne Eigenschaften ersetzt. Beim Schreiben ändern sich höchstens
//! Zeilenfaltung, Zeilenenden (immer CRLF) und unnötige Anführungszeichen
//! um Parameterwerte; der Inhalt bleibt gleich.

use std::fmt::Write as _;

/// Größte Zeilenlänge in Oktetten ohne CRLF (RFC 5545, Abschnitt 3.1).
const FOLD_AT: usize = 75;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub params: Vec<Param>,
    /// Rohwert, wie er in der Datei steht (Escapes nicht aufgelöst)
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    pub properties: Vec<Property>,
    pub components: Vec<Component>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("Zeile {line}: {message}")]
    Syntax { line: usize, message: String },
    #[error("BEGIN:{0} ohne passendes END")]
    Unclosed(String),
    #[error("Zeile {line}: END:{found} passt nicht zu BEGIN:{expected}")]
    Mismatch {
        line: usize,
        expected: String,
        found: String,
    },
    #[error("kein VCALENDAR gefunden")]
    NoCalendar,
}

impl Property {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Property {
            name: name.into(),
            params: Vec::new(),
            value: value.into(),
        }
    }

    /// Eigenschaft mit Textwert; Sonderzeichen werden escaped.
    pub fn text(name: impl Into<String>, text: &str) -> Self {
        Property::new(name, escape_text(text))
    }

    pub fn is(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }

    /// Erster Wert eines Parameters, Groß-/Kleinschreibung des Namens egal.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .and_then(|p| p.values.first())
            .map(String::as_str)
    }

    pub fn set_param(&mut self, name: &str, value: impl Into<String>) {
        let value = value.into();
        match self
            .params
            .iter_mut()
            .find(|p| p.name.eq_ignore_ascii_case(name))
        {
            Some(p) => p.values = vec![value],
            None => self.params.push(Param {
                name: name.into(),
                values: vec![value],
            }),
        }
    }

    pub fn remove_param(&mut self, name: &str) {
        self.params.retain(|p| !p.name.eq_ignore_ascii_case(name));
    }

    /// Wert als Text mit aufgelösten Escapes (`\n`, `\,`, `\;`, `\\`).
    pub fn text_value(&self) -> String {
        unescape_text(&self.value)
    }
}

impl Component {
    pub fn new(name: impl Into<String>) -> Self {
        Component {
            name: name.into(),
            properties: Vec::new(),
            components: Vec::new(),
        }
    }

    pub fn is(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }

    pub fn property(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.is(name))
    }

    pub fn property_mut(&mut self, name: &str) -> Option<&mut Property> {
        self.properties.iter_mut().find(|p| p.is(name))
    }

    pub fn properties_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Property> {
        self.properties.iter().filter(move |p| p.is(name))
    }

    /// Ersetzt die erste Eigenschaft gleichen Namens an ihrer Stelle oder hängt
    /// sie an. Weitere gleichnamige Eigenschaften werden entfernt.
    pub fn set_property(&mut self, property: Property) {
        match self.properties.iter().position(|p| p.is(&property.name)) {
            Some(first) => {
                let name = property.name.clone();
                self.properties[first] = property;
                let mut index = 0;
                self.properties.retain(|p| {
                    let keep = index <= first || !p.is(&name);
                    index += 1;
                    keep
                });
            }
            None => self.properties.push(property),
        }
    }

    pub fn remove_properties(&mut self, name: &str) {
        self.properties.retain(|p| !p.is(name));
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Component> {
        self.components.iter().filter(move |c| c.is(name))
    }

    /// Text einer Eigenschaft mit aufgelösten Escapes, leer wenn sie fehlt.
    pub fn text_of(&self, name: &str) -> String {
        self.property(name)
            .map(Property::text_value)
            .unwrap_or_default()
    }
}

pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

pub fn unescape_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Teilt einen mehrwertigen Rohwert an Kommas, die nicht escaped sind.
pub fn split_values(raw: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (i, c) in raw.char_indices() {
        match c {
            '\\' if !escaped => escaped = true,
            ',' if !escaped => {
                parts.push(&raw[start..i]);
                start = i + 1;
            }
            _ => escaped = false,
        }
    }
    parts.push(&raw[start..]);
    parts
}

/// Logische Zeilen nach dem Entfalten, mit Nummer der ersten physischen Zeile.
fn unfold(input: &str) -> Vec<(usize, String)> {
    let mut lines: Vec<(usize, String)> = Vec::new();
    for (index, raw) in input.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t'])
            && let Some((_, last)) = lines.last_mut()
        {
            last.push_str(rest);
            continue;
        }
        if !line.trim().is_empty() {
            lines.push((index + 1, line.to_string()));
        }
    }
    lines
}

fn parse_line(number: usize, line: &str) -> Result<Property, ParseError> {
    let syntax = |message: &str| ParseError::Syntax {
        line: number,
        message: message.into(),
    };
    let bytes = line.as_bytes();
    let mut pos = line
        .find([';', ':'])
        .ok_or_else(|| syntax("Doppelpunkt fehlt"))?;
    let name = &line[..pos];
    if name.is_empty() {
        return Err(syntax("Eigenschaft ohne Namen"));
    }
    let mut params = Vec::new();
    while bytes[pos] == b';' {
        let name_start = pos + 1;
        let eq = line[name_start..]
            .find('=')
            .map(|i| name_start + i)
            .ok_or_else(|| syntax("Parameter ohne ="))?;
        let param_name = line[name_start..eq].to_string();
        pos = eq + 1;
        let mut values = Vec::new();
        loop {
            if bytes.get(pos) == Some(&b'"') {
                let close = line[pos + 1..]
                    .find('"')
                    .map(|i| pos + 1 + i)
                    .ok_or_else(|| syntax("Anführungszeichen nicht geschlossen"))?;
                values.push(line[pos + 1..close].to_string());
                pos = close + 1;
            } else {
                let end = line[pos..]
                    .find([',', ';', ':'])
                    .map(|i| pos + i)
                    .ok_or_else(|| syntax("Doppelpunkt fehlt"))?;
                values.push(line[pos..end].to_string());
                pos = end;
            }
            match bytes.get(pos) {
                Some(b',') => pos += 1,
                Some(b';' | b':') => break,
                _ => return Err(syntax("unerwartetes Zeichen nach Parameterwert")),
            }
        }
        params.push(Param {
            name: param_name,
            values,
        });
    }
    Ok(Property {
        name: name.to_string(),
        params,
        value: line[pos + 1..].to_string(),
    })
}

/// Liest alle Komponenten der obersten Ebene (normalerweise ein VCALENDAR).
pub fn parse(input: &str) -> Result<Vec<Component>, ParseError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut roots = Vec::new();
    let mut stack: Vec<Component> = Vec::new();
    for (number, line) in unfold(input) {
        let property = parse_line(number, &line)?;
        if property.is("BEGIN") {
            stack.push(Component::new(property.value.trim()));
        } else if property.is("END") {
            let found = property.value.trim();
            let component = stack.pop().ok_or_else(|| ParseError::Syntax {
                line: number,
                message: format!("END:{found} ohne BEGIN"),
            })?;
            if !component.name.eq_ignore_ascii_case(found) {
                return Err(ParseError::Mismatch {
                    line: number,
                    expected: component.name,
                    found: found.into(),
                });
            }
            match stack.last_mut() {
                Some(parent) => parent.components.push(component),
                None => roots.push(component),
            }
        } else {
            match stack.last_mut() {
                Some(current) => current.properties.push(property),
                None => {
                    return Err(ParseError::Syntax {
                        line: number,
                        message: "Eigenschaft außerhalb einer Komponente".into(),
                    });
                }
            }
        }
    }
    if let Some(open) = stack.pop() {
        return Err(ParseError::Unclosed(open.name));
    }
    Ok(roots)
}

/// Liest das erste VCALENDAR aus `input`.
pub fn parse_calendar(input: &str) -> Result<Component, ParseError> {
    parse(input)?
        .into_iter()
        .find(|c| c.is("VCALENDAR"))
        .ok_or(ParseError::NoCalendar)
}

fn needs_quotes(value: &str) -> bool {
    value.contains([':', ';', ','])
}

fn write_folded(out: &mut String, line: &str) {
    let mut width = 0;
    for c in line.chars() {
        let len = c.len_utf8();
        if width + len > FOLD_AT {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += len;
    }
    out.push_str("\r\n");
}

fn write_component(out: &mut String, component: &Component) {
    write_folded(out, &format!("BEGIN:{}", component.name));
    for property in &component.properties {
        let mut line = property.name.clone();
        for param in &property.params {
            let _ = write!(line, ";{}=", param.name);
            for (i, value) in param.values.iter().enumerate() {
                if i > 0 {
                    line.push(',');
                }
                if needs_quotes(value) {
                    let _ = write!(line, "\"{value}\"");
                } else {
                    line.push_str(value);
                }
            }
        }
        line.push(':');
        line.push_str(&property.value);
        write_folded(out, &line);
    }
    for child in &component.components {
        write_component(out, child);
    }
    write_folded(out, &format!("END:{}", component.name));
}

/// Schreibt eine Komponente mit CRLF und Faltung nach 75 Oktetten.
pub fn serialize(component: &Component) -> String {
    let mut out = String::new();
    write_component(&mut out, component);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ein Objekt, wie es Thunderbird/DAVx5 schreiben, mit allem, was nicht
    /// verloren gehen darf.
    const SAMPLE: &str = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Mozilla.org/NONSGML Mozilla Calendar V1.1//EN\r\n\
X-WR-CALNAME:Privat\r\n\
BEGIN:VTIMEZONE\r\n\
TZID:Europe/Berlin\r\n\
X-LIC-LOCATION:Europe/Berlin\r\n\
BEGIN:DAYLIGHT\r\n\
TZOFFSETFROM:+0100\r\n\
TZOFFSETTO:+0200\r\n\
TZNAME:CEST\r\n\
DTSTART:19700329T020000\r\n\
RRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=3\r\n\
END:DAYLIGHT\r\n\
END:VTIMEZONE\r\n\
BEGIN:VEVENT\r\n\
UID:4f1c2a9e-1111-4c3b-9a51-0d8a1b2c3d4e\r\n\
DTSTAMP:20261104T120000Z\r\n\
DTSTART;TZID=Europe/Berlin:20261109T080000\r\n\
DTEND;TZID=Europe/Berlin:20261109T093000\r\n\
SUMMARY:Mathe\\, Analysis\r\n\
LOCATION:Raum 204\r\n\
DESCRIPTION:Erste Zeile\\nZweite Zeile mit Umlauten: äöüß und einem sehr lan\r\n\x20gen Text\\, der gefaltet werden muss\\; damit die Zeile nicht zu lang wird.\r\n\
ATTENDEE;CN=\"Müller, Anna\";ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED:mailto:anna@example.org\r\n\
CATEGORIES:Schule,Mathe\r\n\
X-MOZ-GENERATION:3\r\n\
X-UNBEKANNT;X-PARAM=a,b:wert\r\n\
BEGIN:VALARM\r\n\
ACTION:DISPLAY\r\n\
TRIGGER;RELATED=START:-PT15M\r\n\
DESCRIPTION:Erinnerung\r\n\
END:VALARM\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

    #[test]
    fn parses_structure() {
        let cal = parse_calendar(SAMPLE).unwrap();
        assert_eq!(cal.components.len(), 2);
        let event = &cal.components[1];
        assert!(event.is("vevent"));
        assert_eq!(event.text_of("SUMMARY"), "Mathe, Analysis");
        let alarm = event.children_named("VALARM").next().unwrap();
        assert_eq!(
            alarm.property("TRIGGER").unwrap().param("related"),
            Some("START")
        );
    }

    #[test]
    fn unfolds_and_unescapes_text() {
        let cal = parse_calendar(SAMPLE).unwrap();
        let description = cal.components[1].text_of("DESCRIPTION");
        assert_eq!(
            description,
            "Erste Zeile\nZweite Zeile mit Umlauten: äöüß und einem sehr langen Text, der gefaltet werden muss; damit die Zeile nicht zu lang wird."
        );
    }

    #[test]
    fn quoted_parameter_keeps_comma_and_colon() {
        let cal = parse_calendar(SAMPLE).unwrap();
        let attendee = cal.components[1].property("ATTENDEE").unwrap();
        assert_eq!(attendee.param("CN"), Some("Müller, Anna"));
        assert_eq!(attendee.value, "mailto:anna@example.org");
        let unknown = cal.components[1].property("X-UNBEKANNT").unwrap();
        assert_eq!(unknown.params[0].values, vec!["a", "b"]);
    }

    #[test]
    fn roundtrip_keeps_everything() {
        let cal = parse_calendar(SAMPLE).unwrap();
        let written = serialize(&cal);
        assert_eq!(parse_calendar(&written).unwrap(), cal);
        // Bis auf die Faltung identisch: entfaltet vergleichen
        let unfolded = |s: &str| s.replace("\r\n ", "");
        assert_eq!(unfolded(&written), unfolded(SAMPLE));
    }

    #[test]
    fn folds_long_lines_at_75_octets_without_splitting_utf8() {
        let mut event = Component::new("VEVENT");
        event
            .properties
            .push(Property::text("SUMMARY", &"ä".repeat(100)));
        let written = serialize(&event);
        for line in written.split("\r\n") {
            assert!(
                line.len() <= FOLD_AT,
                "Zeile zu lang: {} Oktette",
                line.len()
            );
        }
        assert_eq!(
            parse(&written).unwrap()[0].text_of("SUMMARY"),
            "ä".repeat(100)
        );
    }

    #[test]
    fn editing_one_property_leaves_unknown_ones_alone() {
        let mut cal = parse_calendar(SAMPLE).unwrap();
        let event = &mut cal.components[1];
        event.set_property(Property::text("SUMMARY", "Mathe (verlegt)"));
        let written = serialize(&cal);
        assert!(written.contains("SUMMARY:Mathe (verlegt)\r\n"));
        assert!(written.contains("X-MOZ-GENERATION:3\r\n"));
        assert!(written.contains("X-UNBEKANNT;X-PARAM=a,b:wert\r\n"));
        assert!(written.contains("TRIGGER;RELATED=START:-PT15M\r\n"));
        // Reihenfolge bleibt: SUMMARY steht weiter vor LOCATION
        assert!(written.find("\r\nSUMMARY:").unwrap() < written.find("\r\nLOCATION:").unwrap());
    }

    #[test]
    fn accepts_lf_line_endings_and_bom() {
        let input = "\u{feff}BEGIN:VCALENDAR\nVERSION:2.0\nEND:VCALENDAR\n";
        let cal = parse_calendar(input).unwrap();
        assert_eq!(cal.text_of("VERSION"), "2.0");
    }

    #[test]
    fn reports_broken_input() {
        assert!(matches!(
            parse("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nEND:VCALENDAR\r\n"),
            Err(ParseError::Mismatch { .. })
        ));
        assert!(matches!(
            parse("BEGIN:VCALENDAR\r\n"),
            Err(ParseError::Unclosed(_))
        ));
        assert!(matches!(
            parse("BEGIN:VCALENDAR\r\nKAPUTT\r\nEND:VCALENDAR\r\n"),
            Err(ParseError::Syntax { line: 2, .. })
        ));
    }

    #[test]
    fn splits_multi_values_on_unescaped_commas() {
        assert_eq!(split_values("a,b\\,c,d"), vec!["a", "b\\,c", "d"]);
        assert_eq!(
            split_values("20261109T080000,20261116T080000"),
            vec!["20261109T080000", "20261116T080000"]
        );
    }
}
