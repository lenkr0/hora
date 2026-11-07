//! WebDAV-Multistatus-Antworten (RFC 4918) lesen und Anfragen bauen.

use roxmltree::{Document, Node};

pub const DAV: &str = "DAV:";
pub const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
pub const CALSERVER: &str = "http://calendarserver.org/ns/";
pub const APPLE: &str = "http://apple.com/ns/ical/";

/// Eine Eigenschaft aus `<d:prop>` in vereinfachter Form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prop {
    pub ns: String,
    pub name: String,
    /// Gesamter Textinhalt, Leerraum am Rand entfernt
    pub text: String,
    /// Alle `<d:href>` darunter
    pub hrefs: Vec<String>,
    /// Namen aller Elemente darunter (Namensraum, lokaler Name)
    pub descendants: Vec<(String, String)>,
    /// `name`-Attribute von `<c:comp>` (supported-calendar-component-set)
    pub comps: Vec<String>,
}

impl Prop {
    pub fn has_descendant(&self, ns: &str, name: &str) -> bool {
        self.descendants.iter().any(|(n, l)| n == ns && l == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropStat {
    pub status: u16,
    pub props: Vec<Prop>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub href: String,
    /// Status der ganzen Antwort, z. B. 404 für gelöschte Einträge im Sync
    pub status: Option<u16>,
    pub propstats: Vec<PropStat>,
}

impl Entry {
    /// Eigenschaft aus einem erfolgreichen propstat.
    pub fn prop(&self, ns: &str, name: &str) -> Option<&Prop> {
        self.propstats
            .iter()
            .filter(|ps| (200..300).contains(&ps.status))
            .flat_map(|ps| &ps.props)
            .find(|p| p.ns == ns && p.name == name)
    }

    pub fn text(&self, ns: &str, name: &str) -> Option<String> {
        self.prop(ns, name)
            .map(|p| p.text.clone())
            .filter(|t| !t.is_empty())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Multistatus {
    pub entries: Vec<Entry>,
    /// Neues Sync-Token aus einem sync-collection-REPORT
    pub sync_token: Option<String>,
}

fn is(node: &Node, ns: &str, name: &str) -> bool {
    node.is_element() && node.tag_name().namespace() == Some(ns) && node.tag_name().name() == name
}

fn child<'a, 'i>(node: &Node<'a, 'i>, ns: &str, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|c| is(c, ns, name))
}

fn text_of(node: &Node) -> String {
    node.descendants()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect::<String>()
        .trim()
        .to_string()
}

/// "HTTP/1.1 404 Not Found" -> 404
fn parse_status(line: &str) -> Option<u16> {
    line.split_whitespace().nth(1)?.parse().ok()
}

pub fn parse_multistatus(xml: &str) -> Result<Multistatus, String> {
    let doc = Document::parse(xml).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if !is(&root, DAV, "multistatus") {
        return Err(format!(
            "Wurzelelement {} statt multistatus",
            root.tag_name().name()
        ));
    }
    let mut ms = Multistatus {
        sync_token: child(&root, DAV, "sync-token").map(|n| text_of(&n)),
        ..Default::default()
    };
    for response in root.children().filter(|n| is(n, DAV, "response")) {
        let Some(href) = child(&response, DAV, "href").map(|n| text_of(&n)) else {
            continue;
        };
        let status = child(&response, DAV, "status").and_then(|n| parse_status(&text_of(&n)));
        let propstats = response
            .children()
            .filter(|n| is(n, DAV, "propstat"))
            .map(|ps| PropStat {
                status: child(&ps, DAV, "status")
                    .and_then(|n| parse_status(&text_of(&n)))
                    .unwrap_or(200),
                props: child(&ps, DAV, "prop")
                    .map(|prop| {
                        prop.children()
                            .filter(Node::is_element)
                            .map(|p| read_prop(&p))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect();
        ms.entries.push(Entry {
            href,
            status,
            propstats,
        });
    }
    Ok(ms)
}

fn read_prop(node: &Node) -> Prop {
    let inner = node.descendants().skip(1).filter(Node::is_element);
    Prop {
        ns: node.tag_name().namespace().unwrap_or_default().to_string(),
        name: node.tag_name().name().to_string(),
        text: text_of(node),
        hrefs: node
            .descendants()
            .filter(|n| is(n, DAV, "href"))
            .map(|n| text_of(&n))
            .collect(),
        descendants: inner
            .clone()
            .map(|n| {
                (
                    n.tag_name().namespace().unwrap_or_default().to_string(),
                    n.tag_name().name().to_string(),
                )
            })
            .collect(),
        comps: inner
            .filter(|n| is(n, CALDAV, "comp"))
            .filter_map(|n| n.attribute("name"))
            .map(str::to_string)
            .collect(),
    }
}

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub const PROPFIND_PRINCIPAL: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;

pub const PROPFIND_HOME_SET: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><c:calendar-home-set/></d:prop></d:propfind>"#;

pub const PROPFIND_CALENDARS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/" xmlns:a="http://apple.com/ns/ical/">
  <d:prop>
    <d:resourcetype/>
    <d:displayname/>
    <a:calendar-color/>
    <c:supported-calendar-component-set/>
    <cs:getctag/>
    <d:sync-token/>
    <d:current-user-privilege-set/>
  </d:prop>
</d:propfind>"#;

pub const PROPFIND_STATE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/"><d:prop><cs:getctag/><d:sync-token/></d:prop></d:propfind>"#;

pub const PROPFIND_ETAGS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getetag/></d:prop></d:propfind>"#;

pub fn sync_collection(token: Option<&str>) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<d:sync-collection xmlns:d="DAV:"><d:sync-token>{}</d:sync-token><d:sync-level>1</d:sync-level><d:prop><d:getetag/></d:prop></d:sync-collection>"#,
        escape(token.unwrap_or_default())
    )
}

pub fn calendar_multiget(hrefs: &[&str]) -> String {
    let mut body = String::from(
        r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-multiget xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/><c:calendar-data/></d:prop>"#,
    );
    for href in hrefs {
        body.push_str(&format!("<d:href>{}</d:href>", escape(href)));
    }
    body.push_str("</c:calendar-multiget>");
    body
}

pub fn mkcalendar(name: &str, color: Option<&str>, components: &[&str]) -> String {
    let color = color
        .map(|c| format!("<a:calendar-color>{}</a:calendar-color>", escape(c)))
        .unwrap_or_default();
    let comps: String = components
        .iter()
        .map(|c| format!(r#"<c:comp name="{}"/>"#, escape(c)))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<c:mkcalendar xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/"><d:set><d:prop><d:displayname>{}</d:displayname>{color}<c:supported-calendar-component-set>{comps}</c:supported-calendar-component-set></d:prop></d:set></c:mkcalendar>"#,
        escape(name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_and_home_set() {
        let ms = parse_multistatus(
            r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns" xmlns:cal="urn:ietf:params:xml:ns:caldav">
 <d:response>
  <d:href>/dav.php/</d:href>
  <d:propstat>
   <d:prop><d:current-user-principal><d:href>/dav.php/principals/testuser/</d:href></d:current-user-principal></d:prop>
   <d:status>HTTP/1.1 200 OK</d:status>
  </d:propstat>
 </d:response>
</d:multistatus>"#,
        )
        .unwrap();
        let entry = &ms.entries[0];
        assert_eq!(entry.href, "/dav.php/");
        assert_eq!(
            entry.prop(DAV, "current-user-principal").unwrap().hrefs,
            vec!["/dav.php/principals/testuser/"]
        );
    }

    #[test]
    fn calendar_list_like_baikal() {
        let ms = parse_multistatus(
            r##"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/" xmlns:x1="http://apple.com/ns/ical/">
 <d:response>
  <d:href>/dav.php/calendars/testuser/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  <d:propstat><d:prop><d:displayname/><x1:calendar-color/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat>
 </d:response>
 <d:response>
  <d:href>/dav.php/calendars/testuser/default/</d:href>
  <d:propstat>
   <d:prop>
    <d:resourcetype><d:collection/><cal:calendar/></d:resourcetype>
    <d:displayname>Privat &amp; Familie</d:displayname>
    <x1:calendar-color>#4F7CACFF</x1:calendar-color>
    <cal:supported-calendar-component-set><cal:comp name="VEVENT"/><cal:comp name="VTODO"/></cal:supported-calendar-component-set>
    <cs:getctag>http://sabre.io/ns/sync/12</cs:getctag>
    <d:sync-token>http://sabre.io/ns/sync/12</d:sync-token>
    <d:current-user-privilege-set><d:privilege><d:read/></d:privilege><d:privilege><d:write/></d:privilege></d:current-user-privilege-set>
   </d:prop>
   <d:status>HTTP/1.1 200 OK</d:status>
  </d:propstat>
 </d:response>
</d:multistatus>"##,
        )
        .unwrap();
        assert_eq!(ms.entries.len(), 2);
        // displayname der Home-Sammlung kam mit 404 und zählt nicht
        assert_eq!(ms.entries[0].text(DAV, "displayname"), None);
        let cal = &ms.entries[1];
        assert!(
            cal.prop(DAV, "resourcetype")
                .unwrap()
                .has_descendant(CALDAV, "calendar")
        );
        assert_eq!(
            cal.text(DAV, "displayname").as_deref(),
            Some("Privat & Familie")
        );
        assert_eq!(
            cal.text(APPLE, "calendar-color").as_deref(),
            Some("#4F7CACFF")
        );
        assert_eq!(
            cal.prop(CALDAV, "supported-calendar-component-set")
                .unwrap()
                .comps,
            vec!["VEVENT", "VTODO"]
        );
        assert_eq!(
            cal.text(CALSERVER, "getctag").as_deref(),
            Some("http://sabre.io/ns/sync/12")
        );
        assert!(
            cal.prop(DAV, "current-user-privilege-set")
                .unwrap()
                .has_descendant(DAV, "write")
        );
    }

    #[test]
    fn sync_collection_with_deleted_entry_and_new_token() {
        let ms = parse_multistatus(
            r#"<d:multistatus xmlns:d="DAV:">
 <d:response><d:href>/cal/a.ics</d:href><d:propstat><d:prop><d:getetag>"2"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
 <d:response><d:href>/cal/b.ics</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>
 <d:sync-token>http://radicale.org/ns/sync/abc</d:sync-token>
</d:multistatus>"#,
        )
        .unwrap();
        assert_eq!(
            ms.sync_token.as_deref(),
            Some("http://radicale.org/ns/sync/abc")
        );
        assert_eq!(ms.entries[0].text(DAV, "getetag").as_deref(), Some("\"2\""));
        assert_eq!(ms.entries[1].status, Some(404));
    }

    #[test]
    fn multiget_with_escaped_and_cdata_calendar_data() {
        let ms = parse_multistatus(
            "<d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\">\
 <d:response><d:href>/a.ics</d:href><d:propstat><d:prop><d:getetag>\"1\"</d:getetag>\
 <c:calendar-data>BEGIN:VCALENDAR&#13;\nSUMMARY:A &amp; B&#13;\nEND:VCALENDAR&#13;\n</c:calendar-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>\
 <d:response><d:href>/b.ics</d:href><d:propstat><d:prop><c:calendar-data><![CDATA[BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n]]></c:calendar-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>\
</d:multistatus>",
        )
        .unwrap();
        assert_eq!(
            ms.entries[0].text(CALDAV, "calendar-data").unwrap(),
            "BEGIN:VCALENDAR\r\nSUMMARY:A & B\r\nEND:VCALENDAR"
        );
        // XML normalisiert Zeilenenden in Text und CDATA zu LF; der iCalendar-Parser nimmt beides
        assert_eq!(
            ms.entries[1].text(CALDAV, "calendar-data").unwrap(),
            "BEGIN:VCALENDAR\nEND:VCALENDAR"
        );
    }

    #[test]
    fn rejects_non_multistatus() {
        assert!(parse_multistatus("<html><body>Login</body></html>").is_err());
        assert!(parse_multistatus("kein xml").is_err());
    }

    #[test]
    fn request_bodies_escape_values() {
        assert!(sync_collection(Some("a&b")).contains("<d:sync-token>a&amp;b</d:sync-token>"));
        assert!(calendar_multiget(&["/x<y>.ics"]).contains("<d:href>/x&lt;y&gt;.ics</d:href>"));
        let mk = mkcalendar("Schule & Uni", Some("#8a6fb0"), &["VEVENT"]);
        assert!(mk.contains("<d:displayname>Schule &amp; Uni</d:displayname>"));
        assert!(mk.contains(r#"<c:comp name="VEVENT"/>"#));
        assert!(Document::parse(&mk).is_ok());
    }
}
