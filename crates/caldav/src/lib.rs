//! CalDAV-Client (RFC 4791) mit Discovery (RFC 6764: `.well-known`,
//! current-user-principal, calendar-home-set), Sync über sync-token
//! (RFC 6578) bzw. ctag/etag, Konflikterkennung per `If-Match` und
//! MKCALENDAR.
//!
//! Der Client arbeitet synchron; die App ruft ihn aus einem eigenen Thread.
//! Passwörter werden als HTTP Basic Auth gesendet, nur an den Server, den
//! der Nutzer angegeben hat: Weiterleitungen auf einen anderen Rechner
//! werden nicht verfolgt.

use std::collections::HashMap;
use std::time::Duration;

use base64::Engine as _;
use ureq::http;
use url::Url;

pub mod xml;

use xml::{APPLE, CALDAV, CALSERVER, DAV, Entry, Multistatus};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Anmeldung abgelehnt – Benutzername oder Passwort stimmt nicht")]
    Unauthorized,
    #[error("Keine Berechtigung für {0}")]
    Forbidden(String),
    #[error("Nicht gefunden: {0}")]
    NotFound(String),
    #[error("Der Termin wurde inzwischen auf dem Server geändert")]
    Conflict,
    #[error("Server antwortet mit Status {status} für {url}")]
    Status { status: u16, url: String },
    #[error("Server nicht erreichbar: {0}")]
    Network(String),
    #[error("Ungültige Server-Adresse: {0}")]
    InvalidUrl(String),
    #[error("Unerwartete Antwort vom Server: {0}")]
    Protocol(String),
    #[error("Unter {0} wurde kein CalDAV-Server gefunden")]
    NoCalDav(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Wie lange auf einen Server gewartet wird.
const TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Höchstens so viele Ressourcen je calendar-multiget.
const MULTIGET_BATCH: usize = 100;
const MAX_REDIRECTS: usize = 5;
/// Größte Antwort, z. B. bei großen Kalendern im Vollabgleich.
const BODY_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarInfo {
    pub url: String,
    pub name: String,
    /// `#rrggbb`
    pub color: Option<String>,
    /// z. B. `["VEVENT", "VTODO"]`; leer heißt: alle
    pub components: Vec<String>,
    pub read_only: bool,
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Discovery {
    pub principal: Url,
    pub home: Url,
    pub calendars: Vec<CalendarInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedObject {
    /// href wie vom Server geliefert (Pfad)
    pub href: String,
    pub etag: String,
    pub data: String,
}

/// Ergebnis eines Sync-Laufs.
#[derive(Debug, Clone, Default)]
pub struct SyncOutcome {
    pub changed: Vec<FetchedObject>,
    pub removed: Vec<String>,
    /// Vollabgleich: `unchanged` enthält alle Ressourcen, die bleiben
    pub full: bool,
    pub unchanged: Vec<String>,
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
    /// Nichts zu tun: ctag unverändert
    pub skipped: bool,
}

/// Bedingung für PUT/DELETE.
#[derive(Debug, Clone, Copy)]
pub enum Precondition<'a> {
    /// Nur anlegen, wenn es die Ressource noch nicht gibt (`If-None-Match: *`)
    Create,
    /// Nur ändern, wenn der Stand noch `etag` ist (`If-Match`)
    Match(&'a str),
}

struct Response {
    status: u16,
    location: Option<String>,
    etag: Option<String>,
    body: String,
}

pub struct Client {
    agent: ureq::Agent,
    authorization: String,
}

impl Client {
    pub fn new(username: &str, password: &str) -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .allow_non_standard_methods(true)
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .timeout_global(Some(TIMEOUT))
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .user_agent(concat!("Hora/", env!("CARGO_PKG_VERSION")))
            .tls_config(tls)
            .build()
            .into();
        let token =
            base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
        Client {
            agent,
            authorization: format!("Basic {token}"),
        }
    }

    fn send(
        &self,
        method: &str,
        url: &Url,
        headers: &[(&str, &str)],
        body: Option<(&str, &str)>,
    ) -> Result<Response> {
        let method = http::Method::from_bytes(method.as_bytes())
            .map_err(|e| Error::Protocol(e.to_string()))?;
        let mut builder = http::Request::builder()
            .method(method)
            .uri(url.as_str())
            .header("Authorization", &self.authorization);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let result = match body {
            Some((content_type, body)) => {
                let request = builder
                    .header("Content-Type", content_type)
                    .body(body.to_string())
                    .map_err(|e| Error::Protocol(e.to_string()))?;
                self.agent.run(request)
            }
            None => {
                let request = builder
                    .body(())
                    .map_err(|e| Error::Protocol(e.to_string()))?;
                self.agent.run(request)
            }
        };
        let mut response = result.map_err(|e| Error::Network(e.to_string()))?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let location = header("Location");
        let etag = header("ETag");
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(BODY_LIMIT)
            .read_to_string()
            .map_err(|e| Error::Network(e.to_string()))?;
        Ok(Response {
            status,
            location,
            etag,
            body,
        })
    }

    /// PROPFIND bzw. REPORT mit Multistatus-Antwort. Folgt Weiterleitungen
    /// auf demselben Rechner und liefert die URL, die geantwortet hat.
    fn dav(&self, method: &str, url: &Url, depth: &str, body: &str) -> Result<(Url, Multistatus)> {
        let mut current = url.clone();
        for _ in 0..=MAX_REDIRECTS {
            let response = self.send(
                method,
                &current,
                &[("Depth", depth)],
                Some(("application/xml; charset=utf-8", body)),
            )?;
            match response.status {
                207 => {
                    let ms = xml::parse_multistatus(&response.body).map_err(Error::Protocol)?;
                    return Ok((current, ms));
                }
                301 | 302 | 303 | 307 | 308 => {
                    let location = response
                        .location
                        .ok_or_else(|| Error::Protocol("Weiterleitung ohne Ziel".into()))?;
                    let next = current
                        .join(&location)
                        .map_err(|e| Error::InvalidUrl(e.to_string()))?;
                    if next.host_str() != url.host_str() {
                        return Err(Error::Protocol(format!(
                            "Weiterleitung auf einen anderen Server ({}) wird aus Sicherheitsgründen nicht verfolgt",
                            next.host_str().unwrap_or("?")
                        )));
                    }
                    current = next;
                }
                status => return Err(status_error(status, &current, &response.body)),
            }
        }
        Err(Error::Protocol("zu viele Weiterleitungen".into()))
    }

    /// Findet Principal, calendar-home-set und Kalender. `address` darf die
    /// DAV-Adresse (z. B. `http://server/dav.php`) oder nur den Server nennen.
    pub fn discover(&self, address: &str) -> Result<Discovery> {
        let base = parse_address(address)?;
        let mut candidates = vec![base.clone()];
        if let Ok(well_known) = base.join("/.well-known/caldav") {
            candidates.push(well_known);
        }

        let mut principal = None;
        let mut last_error = None;
        for candidate in &candidates {
            match self.dav("PROPFIND", candidate, "0", xml::PROPFIND_PRINCIPAL) {
                Ok((answered, ms)) => {
                    let href = ms
                        .entries
                        .iter()
                        .find_map(|e| e.prop(DAV, "current-user-principal"))
                        .and_then(|p| p.hrefs.first().cloned());
                    if let Some(href) = href {
                        principal = Some(
                            answered
                                .join(&href)
                                .map_err(|e| Error::InvalidUrl(e.to_string()))?,
                        );
                        break;
                    }
                }
                Err(Error::Unauthorized) => return Err(Error::Unauthorized),
                Err(e) => last_error = Some(e),
            }
        }
        let Some(principal) = principal else {
            return Err(match last_error {
                Some(Error::Network(e)) => Error::Network(e),
                _ => Error::NoCalDav(base.to_string()),
            });
        };

        let (answered, ms) = self.dav("PROPFIND", &principal, "0", xml::PROPFIND_HOME_SET)?;
        let home_href = ms
            .entries
            .iter()
            .find_map(|e| e.prop(CALDAV, "calendar-home-set"))
            .and_then(|p| p.hrefs.first().cloned())
            .ok_or_else(|| Error::Protocol("Der Server nennt keinen calendar-home-set".into()))?;
        let home = answered
            .join(&home_href)
            .map_err(|e| Error::InvalidUrl(e.to_string()))?;
        let calendars = self.calendars(&home)?;
        Ok(Discovery {
            principal,
            home,
            calendars,
        })
    }

    /// Kalender unter dem calendar-home-set.
    pub fn calendars(&self, home: &Url) -> Result<Vec<CalendarInfo>> {
        let (answered, ms) = self.dav("PROPFIND", home, "1", xml::PROPFIND_CALENDARS)?;
        let mut calendars = Vec::new();
        for entry in &ms.entries {
            let is_calendar = entry
                .prop(DAV, "resourcetype")
                .is_some_and(|p| p.has_descendant(CALDAV, "calendar"));
            if !is_calendar {
                continue;
            }
            let url = answered
                .join(&entry.href)
                .map_err(|e| Error::InvalidUrl(e.to_string()))?;
            let read_only = entry
                .prop(DAV, "current-user-privilege-set")
                .is_some_and(|p| {
                    !["write", "write-content", "all"]
                        .iter()
                        .any(|w| p.has_descendant(DAV, w))
                });
            calendars.push(CalendarInfo {
                name: entry
                    .text(DAV, "displayname")
                    .unwrap_or_else(|| last_segment(&url)),
                color: entry
                    .text(APPLE, "calendar-color")
                    .and_then(|c| normalize_color(&c)),
                components: entry
                    .prop(CALDAV, "supported-calendar-component-set")
                    .map(|p| p.comps.clone())
                    .unwrap_or_default(),
                read_only,
                ctag: entry.text(CALSERVER, "getctag"),
                sync_token: entry.text(DAV, "sync-token"),
                url: url.to_string(),
            });
        }
        Ok(calendars)
    }

    /// Gleicht einen Kalender ab. `known` enthält href → etag des lokalen
    /// Stands, `ctag`/`sync_token` den Stand des letzten Laufs.
    pub fn sync(
        &self,
        calendar_url: &str,
        known: &HashMap<String, String>,
        ctag: Option<&str>,
        sync_token: Option<&str>,
    ) -> Result<SyncOutcome> {
        let url = Url::parse(calendar_url).map_err(|e| Error::InvalidUrl(e.to_string()))?;
        let (_, state) = self.dav("PROPFIND", &url, "0", xml::PROPFIND_STATE)?;
        let new_ctag = state
            .entries
            .first()
            .and_then(|e| e.text(CALSERVER, "getctag"));
        let new_token = state
            .entries
            .first()
            .and_then(|e| e.text(DAV, "sync-token"));
        if new_ctag.is_some() && new_ctag.as_deref() == ctag && !known.is_empty() {
            return Ok(SyncOutcome {
                skipped: true,
                ctag: new_ctag,
                sync_token: sync_token.map(str::to_string),
                ..Default::default()
            });
        }

        let mut outcome = match new_token {
            Some(_) => self.sync_with_token(&url, known, sync_token)?,
            None => self.sync_with_etags(&url, known)?,
        };
        outcome.ctag = new_ctag;
        Ok(outcome)
    }

    fn sync_with_token(
        &self,
        url: &Url,
        known: &HashMap<String, String>,
        token: Option<&str>,
    ) -> Result<SyncOutcome> {
        let (ms, full) = match self.dav("REPORT", url, "0", &xml::sync_collection(token)) {
            Ok((_, ms)) => (ms, token.is_none()),
            // Token abgelaufen oder vom Server verworfen: von vorn
            Err(
                Error::Forbidden(_)
                | Error::Status {
                    status: 409 | 412 | 400,
                    ..
                },
            ) if token.is_some() => (
                self.dav("REPORT", url, "0", &xml::sync_collection(None))?.1,
                true,
            ),
            Err(e) => return Err(e),
        };
        let mut outcome = SyncOutcome {
            full,
            sync_token: ms.sync_token.clone(),
            ..Default::default()
        };
        let mut to_fetch = Vec::new();
        for entry in members(url, &ms) {
            if entry.status == Some(404) {
                // Manche Server (Radicale) melden auch längst gelöschte Einträge
                if known.contains_key(&entry.href) {
                    outcome.removed.push(entry.href.clone());
                }
                continue;
            }
            let etag = entry.text(DAV, "getetag");
            if etag.is_some() && known.get(&entry.href) == etag.as_ref() {
                outcome.unchanged.push(entry.href.clone());
            } else {
                to_fetch.push(entry.href.clone());
            }
        }
        if full {
            let listed: Vec<&String> = outcome.unchanged.iter().chain(&to_fetch).collect();
            outcome
                .removed
                .extend(known.keys().filter(|h| !listed.contains(h)).cloned());
        }
        outcome.changed = self.fetch(url, &to_fetch)?;
        Ok(outcome)
    }

    /// Abgleich über ETags (PROPFIND Depth 1) für Server ohne sync-token.
    pub fn sync_with_etags(
        &self,
        url: &Url,
        known: &HashMap<String, String>,
    ) -> Result<SyncOutcome> {
        let (_, ms) = self.dav("PROPFIND", url, "1", xml::PROPFIND_ETAGS)?;
        let mut outcome = SyncOutcome {
            full: true,
            ..Default::default()
        };
        let mut to_fetch = Vec::new();
        for entry in members(url, &ms) {
            let is_collection = entry
                .prop(DAV, "resourcetype")
                .is_some_and(|p| p.has_descendant(DAV, "collection"));
            if is_collection {
                continue;
            }
            let etag = entry.text(DAV, "getetag");
            if etag.is_some() && known.get(&entry.href) == etag.as_ref() {
                outcome.unchanged.push(entry.href.clone());
            } else {
                to_fetch.push(entry.href.clone());
            }
        }
        let listed: Vec<&String> = outcome.unchanged.iter().chain(&to_fetch).collect();
        outcome.removed = known
            .keys()
            .filter(|h| !listed.contains(h))
            .cloned()
            .collect();
        outcome.changed = self.fetch(url, &to_fetch)?;
        Ok(outcome)
    }

    /// Lädt Ressourcen per calendar-multiget.
    pub fn fetch(&self, calendar_url: &Url, hrefs: &[String]) -> Result<Vec<FetchedObject>> {
        let mut objects = Vec::new();
        for batch in hrefs.chunks(MULTIGET_BATCH) {
            let refs: Vec<&str> = batch.iter().map(String::as_str).collect();
            let (_, ms) = self.dav("REPORT", calendar_url, "1", &xml::calendar_multiget(&refs))?;
            for entry in ms.entries {
                if let (Some(data), Some(etag)) = (
                    entry.text(CALDAV, "calendar-data"),
                    entry.text(DAV, "getetag"),
                ) {
                    objects.push(FetchedObject {
                        href: entry.href,
                        etag,
                        data,
                    });
                }
            }
        }
        Ok(objects)
    }

    /// Legt eine Ressource an oder ändert sie. Liefert das neue ETag, falls
    /// der Server es mitschickt.
    pub fn put(&self, url: &Url, ics: &str, condition: Precondition) -> Result<Option<String>> {
        let header = match condition {
            Precondition::Create => ("If-None-Match", "*"),
            Precondition::Match(etag) => ("If-Match", etag),
        };
        let response = self.send(
            "PUT",
            url,
            &[header],
            Some(("text/calendar; charset=utf-8", ics)),
        )?;
        match response.status {
            200..=299 => Ok(response.etag),
            412 => Err(Error::Conflict),
            status => Err(status_error(status, url, &response.body)),
        }
    }

    pub fn delete(&self, url: &Url, etag: Option<&str>) -> Result<()> {
        let headers: Vec<(&str, &str)> = etag.map(|e| ("If-Match", e)).into_iter().collect();
        let response = self.send("DELETE", url, &headers, None)?;
        match response.status {
            200..=299 | 404 => Ok(()),
            412 => Err(Error::Conflict),
            status => Err(status_error(status, url, &response.body)),
        }
    }

    /// Legt unter `home` einen Kalender `slug` für die Komponenten
    /// `components` an (z. B. `["VEVENT", "VTODO"]` oder `["VJOURNAL"]`).
    pub fn make_calendar(
        &self,
        home: &Url,
        slug: &str,
        name: &str,
        color: Option<&str>,
        components: &[&str],
    ) -> Result<Url> {
        let url = home
            .join(&format!("{slug}/"))
            .map_err(|e| Error::InvalidUrl(e.to_string()))?;
        let body = xml::mkcalendar(name, color, components);
        let response = self.send(
            "MKCALENDAR",
            &url,
            &[],
            Some(("application/xml; charset=utf-8", &body)),
        )?;
        match response.status {
            200..=299 => Ok(url),
            status => Err(status_error(status, &url, &response.body)),
        }
    }

    /// Rohe Ressource lesen (für Tests und Konfliktauflösung).
    pub fn get(&self, url: &Url) -> Result<(String, Option<String>)> {
        let response = self.send("GET", url, &[], None)?;
        match response.status {
            200 => Ok((response.body, response.etag)),
            status => Err(status_error(status, url, &response.body)),
        }
    }
}

fn status_error(status: u16, url: &Url, body: &str) -> Error {
    match status {
        401 => Error::Unauthorized,
        403 if body.contains("valid-sync-token") => Error::Status {
            status: 412,
            url: url.to_string(),
        },
        403 => Error::Forbidden(url.to_string()),
        404 => Error::NotFound(url.to_string()),
        412 => Error::Conflict,
        status => Error::Status {
            status,
            url: url.to_string(),
        },
    }
}

/// Einträge ohne die Sammlung selbst.
fn members<'a>(collection: &Url, ms: &'a Multistatus) -> impl Iterator<Item = &'a Entry> {
    let own = collection.path().trim_end_matches('/').to_string();
    ms.entries.iter().filter(move |e| {
        let path = Url::parse(&e.href)
            .map(|u| u.path().to_string())
            .unwrap_or_else(|_| e.href.clone());
        path.trim_end_matches('/') != own
    })
}

/// Server-Adresse aus der Eingabe; ohne Schema wird HTTPS angenommen.
pub fn parse_address(address: &str) -> Result<Url> {
    let address = address.trim();
    let with_scheme = if address.contains("://") {
        address.to_string()
    } else {
        format!("https://{address}")
    };
    let mut url =
        Url::parse(&with_scheme).map_err(|e| Error::InvalidUrl(format!("{address}: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::InvalidUrl(format!("{address}: nur http und https")));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

fn last_segment(url: &Url) -> String {
    url.path_segments()
        .and_then(|mut s| s.rfind(|p| !p.is_empty()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| url.to_string())
}

/// `#RRGGBBAA` oder `#RRGGBB` -> `#rrggbb`
pub fn normalize_color(color: &str) -> Option<String> {
    let hex = color.trim().strip_prefix('#')?;
    (hex.len() >= 6 && hex[..6].chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("#{}", hex[..6].to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_get_scheme_and_trailing_slash() {
        assert_eq!(
            parse_address("http://192.0.2.50:8089/dav.php")
                .unwrap()
                .as_str(),
            "http://192.0.2.50:8089/dav.php/"
        );
        assert_eq!(
            parse_address("dav.example.org").unwrap().as_str(),
            "https://dav.example.org/"
        );
        assert!(parse_address("ftp://x").is_err());
        assert!(parse_address("http://").is_err());
    }

    #[test]
    fn colors_are_normalized() {
        assert_eq!(normalize_color("#4F7CACFF").as_deref(), Some("#4f7cac"));
        assert_eq!(normalize_color("#4f7cac").as_deref(), Some("#4f7cac"));
        assert_eq!(normalize_color("blau"), None);
        assert_eq!(normalize_color("#12"), None);
    }

    #[test]
    fn members_skip_the_collection_itself() {
        let ms = xml::parse_multistatus(
            r#"<d:multistatus xmlns:d="DAV:">
<d:response><d:href>/dav.php/calendars/l/default/</d:href></d:response>
<d:response><d:href>/dav.php/calendars/l/default/a.ics</d:href></d:response>
</d:multistatus>"#,
        )
        .unwrap();
        let url = Url::parse("http://h/dav.php/calendars/l/default/").unwrap();
        let hrefs: Vec<&str> = members(&url, &ms).map(|e| e.href.as_str()).collect();
        assert_eq!(hrefs, vec!["/dav.php/calendars/l/default/a.ics"]);
    }

    #[test]
    fn calendar_name_falls_back_to_last_path_segment() {
        assert_eq!(
            last_segment(&Url::parse("http://h/dav.php/calendars/l/schule/").unwrap()),
            "schule"
        );
    }

    #[test]
    fn unreachable_server_reports_network_error() {
        // Port 9 auf localhost ist praktisch nie belegt
        let client = Client::new("u", "p");
        match client.discover("http://127.0.0.1:9/") {
            Err(Error::Network(_)) => {}
            other => panic!("erwartet Netzwerkfehler, erhalten: {other:?}"),
        }
    }
}
