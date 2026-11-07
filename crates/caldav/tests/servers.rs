//! Integrationstests gegen echte CalDAV-Server in Docker-Containern
//! (Radicale und Baïkal), nie gegen echte Daten.
//!
//! Laufen nur auf Anfrage und brauchen ein laufendes Docker:
//!
//! ```text
//! cargo test -p hora-caldav --test servers -- --ignored --test-threads=1
//! ```

use std::collections::HashMap;
use std::process::Command;
use std::time::{Duration, Instant};

use hora_caldav::{Client, Error, Precondition};
use hora_core::model::CalendarObject;
use url::Url;

const USER: &str = "test";
const PASSWORD: &str = "geheim-123";

struct Server {
    container: String,
    url: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = docker(&["rm", "-f", &self.container]);
    }
}

fn docker(args: &[&str]) -> Result<String, String> {
    let output = Command::new("docker")
        .args(args)
        .output()
        .map_err(|e| format!("docker nicht startbar: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!(
            "docker {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn unique_name(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("hora-test-{prefix}-{}-{nanos}", std::process::id())
}

fn host_port(container: &str, port: &str) -> String {
    let mapping = docker(&["port", container, port]).expect("Port-Zuordnung");
    // "127.0.0.1:49153" (ggf. mehrere Zeilen für IPv4/IPv6)
    mapping
        .lines()
        .next()
        .unwrap()
        .rsplit(':')
        .next()
        .unwrap()
        .to_string()
}

/// Wartet, bis der Server Anfragen beantwortet (egal mit welchem Status).
fn wait_ready(url: &str) {
    let client = Client::new(USER, PASSWORD);
    let started = Instant::now();
    loop {
        match client.discover(url) {
            Err(Error::Network(e)) if started.elapsed() < Duration::from_secs(90) => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(500));
            }
            _ => return,
        }
    }
}

fn radicale() -> Server {
    let name = unique_name("radicale");
    docker(&[
        "create",
        "--name",
        &name,
        "-p",
        "127.0.0.1::5232",
        "-e",
        "RADICALE_CONFIG_AUTH_TYPE=htpasswd",
        "-e",
        "RADICALE_CONFIG_AUTH_HTPASSWD_FILENAME=/tmp/users",
        "-e",
        "RADICALE_CONFIG_AUTH_HTPASSWD_ENCRYPTION=plain",
        "tomsquest/docker-radicale:3.9.0.0",
    ])
    .expect("Radicale-Container anlegen");
    // Ab hier räumt Drop den Container auch bei einem Fehlschlag weg
    let mut server = Server {
        container: name.clone(),
        url: String::new(),
    };
    let dir = tempfile::tempdir().unwrap();
    let users = dir.path().join("users");
    std::fs::write(&users, format!("{USER}:{PASSWORD}\n")).unwrap();
    docker(&["cp", users.to_str().unwrap(), &format!("{name}:/tmp/users")])
        .expect("Benutzerdatei kopieren");
    docker(&["start", &name]).expect("Radicale starten");
    server.url = format!("http://127.0.0.1:{}/", host_port(&name, "5232"));
    wait_ready(&server.url);
    server
}

/// Richtet Baïkal ohne Web-Assistent ein: Datenbank aus dem mitgelieferten
/// Schema, ein Benutzer und eine Konfiguration mit Basic Auth.
const BAIKAL_SETUP: &str = r#"
$root = '/var/www/baikal';
require $root . '/Core/Distrib.php';
$db = $root . '/Specific/db/db.sqlite';
@mkdir(dirname($db), 0777, true);
$pdo = new PDO('sqlite:' . $db);
$pdo->exec(file_get_contents($root . '/Core/Resources/Db/SQLite/db.sql'));
$pdo->prepare('INSERT INTO users (username, digesta1) VALUES (?, ?)')
    ->execute([getenv('HORA_USER'), md5(getenv('HORA_USER') . ':BaikalDAV:' . getenv('HORA_PASSWORD'))]);
$pdo->prepare('INSERT INTO principals (uri, email, displayname) VALUES (?, ?, ?)')
    ->execute(['principals/' . getenv('HORA_USER'), 'test@example.org', 'Test']);
$yaml = "system:\n"
    . "  configured_version: '" . BAIKAL_VERSION . "'\n"
    . "  timezone: Europe/Berlin\n"
    . "  card_enabled: true\n"
    . "  cal_enabled: true\n"
    . "  dav_auth_type: Basic\n"
    // Ohne Admin-Passwort gilt Baïkal als nicht eingerichtet; die Admin-Oberfläche brauchen die Tests nicht
    . "  admin_passwordhash: '" . hash('sha256', 'admin:BaikalDAV:' . bin2hex(random_bytes(16))) . "'\n"
    . "  failed_access_message: 'user %u authentication failure for Baikal'\n"
    . "  auth_realm: BaikalDAV\n"
    . "  base_uri: ''\n"
    . "database:\n"
    . "  sqlite_file: '" . $db . "'\n"
    . "  backend: sqlite\n"
    . "  mysql_host: ''\n  mysql_dbname: ''\n  mysql_username: ''\n  mysql_password: ''\n"
    . "  encryption_key: 'hora-test'\n"
    . "  pgsql_host: ''\n  pgsql_dbname: ''\n  pgsql_username: ''\n  pgsql_password: ''\n";
file_put_contents($root . '/config/baikal.yaml', $yaml);
echo 'Baikal ' . BAIKAL_VERSION . " eingerichtet\n";
"#;

fn baikal() -> Server {
    let name = unique_name("baikal");
    docker(&[
        "create",
        "--name",
        &name,
        "-p",
        "127.0.0.1::80",
        "ckulka/baikal:0.10.1-nginx",
    ])
    .expect("Baïkal-Container anlegen");
    let mut server = Server {
        container: name.clone(),
        url: String::new(),
    };
    docker(&["start", &name]).expect("Baïkal starten");
    let user = format!("HORA_USER={USER}");
    let password = format!("HORA_PASSWORD={PASSWORD}");
    docker(&[
        "exec",
        "-e",
        &user,
        "-e",
        &password,
        &name,
        "php",
        "-r",
        BAIKAL_SETUP,
    ])
    .expect("Baïkal einrichten");
    docker(&[
        "exec",
        &name,
        "chmod",
        "-R",
        "a+rwX",
        "/var/www/baikal/Specific",
        "/var/www/baikal/config",
    ])
    .expect("Rechte setzen");
    server.url = format!("http://127.0.0.1:{}/dav.php", host_port(&name, "80"));
    wait_ready(&server.url);
    server
}

fn event(uid: &str, start: &str, summary: &str, extra: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Hora//Test//DE\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261104T120000Z\r\n\
         DTSTART;TZID=Europe/Berlin:{start}\r\nDURATION:PT1H\r\nSUMMARY:{summary}\r\n{extra}END:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn file_name(href: &str) -> String {
    href.rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(href)
        .to_string()
}

/// Der gemeinsame Ablauf für alle Server.
fn exercise(server: &Server) {
    // Falsches Passwort
    match Client::new(USER, "falsch").discover(&server.url) {
        Err(Error::Unauthorized) => {}
        other => panic!("falsches Passwort: erwartet Unauthorized, erhalten {other:?}"),
    }

    // Discovery und Kalender anlegen
    let client = Client::new(USER, PASSWORD);
    let discovery = client.discover(&server.url).expect("Discovery");
    let calendar = client
        .make_calendar(
            &discovery.home,
            "hora-test",
            "Hora Test & Co",
            Some("#8a6fb0"),
            &["VEVENT", "VTODO"],
        )
        .expect("MKCALENDAR");
    let info = client
        .calendars(&discovery.home)
        .unwrap()
        .into_iter()
        .find(|c| c.url == calendar.as_str())
        .expect("neuer Kalender in der Liste");
    assert_eq!(info.name, "Hora Test & Co");
    assert_eq!(info.color.as_deref(), Some("#8a6fb0"));
    assert!(info.components.iter().any(|c| c == "VEVENT"));
    assert!(!info.read_only);
    assert!(info.sync_token.is_some(), "Server unterstützt sync-token");

    // Zwei Termine hochladen; der zweite mit Serie, Alarm und eigener Eigenschaft
    let a_url = calendar.join("a.ics").unwrap();
    let b_url = calendar.join("b.ics").unwrap();
    let a_etag = client
        .put(
            &a_url,
            &event("a", "20261109T080000", "Mathe", ""),
            Precondition::Create,
        )
        .unwrap();
    client
        .put(
            &b_url,
            &event(
                "b",
                "20261110T100000",
                "Chor",
                "RRULE:FREQ=WEEKLY;BYDAY=TU\r\nX-HORA-TEST;X-PARAM=\"a,b\":bleibt erhalten\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nDESCRIPTION:Chor\r\nEND:VALARM\r\n",
            ),
            Precondition::Create,
        )
        .unwrap();
    assert!(
        matches!(
            client.put(
                &a_url,
                &event("a", "20261109T080000", "Doppelt", ""),
                Precondition::Create
            ),
            Err(Error::Conflict)
        ),
        "If-None-Match: * verhindert Überschreiben"
    );

    // Erster Sync: alles neu
    let first = client
        .sync(calendar.as_str(), &HashMap::new(), None, None)
        .expect("erster Sync");
    assert!(first.full);
    assert_eq!(
        sorted(first.changed.iter().map(|o| file_name(&o.href)).collect()),
        vec!["a.ics", "b.ics"]
    );
    assert!(first.ctag.is_some() && first.sync_token.is_some());
    let b = first
        .changed
        .iter()
        .find(|o| o.href.ends_with("b.ics"))
        .unwrap();
    let object = CalendarObject::parse(&b.data).expect("Server liefert gültiges iCalendar");
    let vevent = object.root.children_named("VEVENT").next().unwrap();
    let custom = vevent
        .property("X-HORA-TEST")
        .expect("unbekannte Eigenschaft bleibt");
    assert_eq!(custom.value, "bleibt erhalten");
    assert_eq!(custom.param("X-PARAM"), Some("a,b"));
    assert!(
        vevent.children_named("VALARM").next().is_some(),
        "VALARM bleibt"
    );
    let (events, _) = object.events();
    assert_eq!(events[0].rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=TU"));

    let mut known: HashMap<String, String> = first
        .changed
        .iter()
        .map(|o| (o.href.clone(), o.etag.clone()))
        .collect();

    // Unverändert: ctag gleich -> nichts zu tun
    let idle = client
        .sync(
            calendar.as_str(),
            &known,
            first.ctag.as_deref(),
            first.sync_token.as_deref(),
        )
        .unwrap();
    assert!(idle.skipped, "ctag unverändert");

    // Ändern, löschen, neu anlegen; dann inkrementeller Sync über das Token
    let a_href = first
        .changed
        .iter()
        .find(|o| o.href.ends_with("a.ics"))
        .unwrap()
        .href
        .clone();
    let b_href = b.href.clone();
    let a_known_etag = known[&a_href].clone();
    client
        .put(
            &a_url,
            &event("a", "20261109T090000", "Mathe (verlegt)", ""),
            Precondition::Match(&a_known_etag),
        )
        .expect("Ändern mit passendem ETag");
    client
        .delete(&b_url, Some(&known[&b_href]))
        .expect("Löschen");
    client
        .put(
            &calendar.join("c.ics").unwrap(),
            &event("c", "20261111T080000", "Neu", ""),
            Precondition::Create,
        )
        .unwrap();

    let second = client
        .sync(
            calendar.as_str(),
            &known,
            first.ctag.as_deref(),
            first.sync_token.as_deref(),
        )
        .unwrap();
    assert!(
        !second.skipped && !second.full,
        "inkrementell über sync-token"
    );
    assert_eq!(
        sorted(second.changed.iter().map(|o| file_name(&o.href)).collect()),
        vec!["a.ics", "c.ics"]
    );
    assert_eq!(
        second
            .removed
            .iter()
            .map(|h| file_name(h))
            .collect::<Vec<_>>(),
        vec!["b.ics"]
    );
    assert!(
        second
            .changed
            .iter()
            .any(|o| o.data.contains("Mathe (verlegt)"))
    );
    assert_ne!(second.sync_token, first.sync_token);

    // Konflikt: Ändern mit veraltetem ETag
    let stale = a_etag.unwrap_or(a_known_etag);
    assert!(
        matches!(
            client.put(
                &a_url,
                &event("a", "20261109T100000", "Veraltet", ""),
                Precondition::Match(&stale)
            ),
            Err(Error::Conflict)
        ),
        "If-Match mit altem ETag muss 412 liefern"
    );

    for o in &second.changed {
        known.insert(o.href.clone(), o.etag.clone());
    }
    known.remove(&b_href);

    // Ungültiges Token: Vollabgleich statt Fehler
    let reset = client
        .sync(
            calendar.as_str(),
            &known,
            None,
            Some("http://invalid.example/sync/999"),
        )
        .unwrap();
    assert!(reset.full);
    assert!(
        reset.changed.is_empty(),
        "nichts geändert: {:?}",
        reset.changed
    );
    assert!(reset.removed.is_empty());
    assert_eq!(
        sorted(reset.unchanged.iter().map(|h| file_name(h)).collect()),
        vec!["a.ics", "c.ics"]
    );

    // Abgleich über ETags (für Server ohne sync-token)
    let mut outdated = known.clone();
    outdated.insert(a_href.clone(), "\"alt\"".into());
    outdated.insert("/gibt/es/nicht.ics".into(), "\"x\"".into());
    let by_etag = client
        .sync_with_etags(&Url::parse(calendar.as_str()).unwrap(), &outdated)
        .unwrap();
    assert_eq!(
        by_etag
            .changed
            .iter()
            .map(|o| file_name(&o.href))
            .collect::<Vec<_>>(),
        vec!["a.ics"]
    );
    assert_eq!(by_etag.removed, vec!["/gibt/es/nicht.ics".to_string()]);

    exercise_hora_objects(&client, &calendar);
    exercise_series(&client, &calendar);
    exercise_tasks(&client, &calendar);
    exercise_journals(&client, &discovery.home);
}

/// Journal: eigener Kalender nur für VJOURNAL (wie Horas Knopf
/// „Journal-Kalender anlegen“), Eintrag anlegen, ändern, löschen.
fn exercise_journals(client: &Client, home: &Url) {
    use chrono::{NaiveDate, TimeZone, Utc};
    use chrono_tz::Europe::Berlin;
    use hora_core::journal::{self, JournalFields};

    let calendar = client
        .make_calendar(home, "hora-journal", "Journal", None, &["VJOURNAL"])
        .expect("MKCALENDAR für Journale");
    let info = client
        .calendars(home)
        .unwrap()
        .into_iter()
        .find(|c| c.url == calendar.as_str())
        .expect("Journal-Kalender in der Liste");
    assert_eq!(info.components, vec!["VJOURNAL".to_string()]);

    let now = Utc.with_ymd_and_hms(2026, 11, 6, 18, 0, 0).unwrap();
    let day = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
    let fields = JournalFields {
        summary: "Erster Schnee".into(),
        description: "# Morgens\n\n**Kalt**, aber schön; ein *guter* Tag.\n\n- Tee\n- Schal".into(),
        day,
        categories: vec!["Winter".into(), "draußen".into()],
    };
    let url = calendar.join("schnee.ics").unwrap();
    client
        .put(
            &url,
            &journal::new_journal("schnee", &fields, Berlin, now).to_ics(),
            Precondition::Create,
        )
        .expect("Journaleintrag anlegen");
    let (data, etag) = client.get(&url).unwrap();
    let mut object = CalendarObject::parse(&data).unwrap();
    assert_eq!(object.kind(), Some(hora_core::model::Kind::Journal));
    let entry = &object.journals()[0];
    assert_eq!(entry.summary, "Erster Schnee");
    assert_eq!(entry.description, fields.description);
    assert_eq!(entry.categories, ["Winter", "draußen"]);
    assert_eq!(entry.day(Berlin), Some(day));

    // Ändern: anderer Tag, Text leer, ein Tag weniger
    let changed = JournalFields {
        description: String::new(),
        day: day.succ_opt().unwrap(),
        categories: vec!["Winter".into()],
        ..fields
    };
    assert!(journal::update_journal(&mut object, "schnee", &changed, Berlin, now).unwrap());
    client
        .put(&url, &object.to_ics(), Precondition::Match(&etag.unwrap()))
        .expect("Journaleintrag ändern");
    let (data, etag) = client.get(&url).unwrap();
    let back = CalendarObject::parse(&data).unwrap();
    let entry = &back.journals()[0];
    assert_eq!(entry.description, "", "leerer Text kommt leer zurück");
    assert_eq!(entry.categories, ["Winter"]);
    assert_eq!(entry.day(Berlin), day.succ_opt());

    client
        .delete(&url, etag.as_deref())
        .expect("Journaleintrag löschen");
    assert!(matches!(client.get(&url), Err(Error::NotFound(_))));
}

/// Von Hora erzeugte und bearbeitete Objekte: Server nimmt sie an und gibt
/// sie inhaltlich unverändert zurück.
fn exercise_hora_objects(client: &Client, calendar: &Url) {
    use chrono::{NaiveDateTime, TimeZone, Utc};
    use chrono_tz::Europe::Berlin;
    use hora_core::edit::{self, EventFields};
    use hora_core::time::DateTimeValue;

    let at = |s: &str| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 4, 12, 0, 0).unwrap();
    let fields = EventFields {
        summary: "Neu aus Hora, mit Komma; und Semikolon".into(),
        location: "Raum 1".into(),
        description: "Zeile 1\nZeile 2 mit Umlauten äöü".into(),
        start: DateTimeValue::Zoned(at("2026-11-12 09:00"), Berlin),
        end: DateTimeValue::Zoned(at("2026-11-12 10:00"), Berlin),
    };
    let url = calendar.join("hora-neu.ics").unwrap();
    client
        .put(
            &url,
            &edit::new_event("hora-neu", &fields, now).to_ics(),
            Precondition::Create,
        )
        .expect("Server nimmt von Hora erzeugten Termin mit VTIMEZONE an");
    let (data, etag) = client.get(&url).unwrap();
    let mut object = CalendarObject::parse(&data).unwrap();
    let (events, errors) = object.events();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(events[0].summary, fields.summary);
    assert_eq!(events[0].description, fields.description);
    assert_eq!(events[0].start, fields.start);
    assert_eq!(events[0].end, fields.end);

    // Verschieben wie beim Ziehen in der Woche, gegen das ETag des Servers
    edit::set_times(
        &mut object,
        "hora-neu",
        DateTimeValue::Zoned(at("2026-11-13 14:00"), Berlin),
        DateTimeValue::Zoned(at("2026-11-13 15:30"), Berlin),
        now,
    )
    .unwrap();
    let etag = etag.expect("Server liefert ETag beim GET");
    client
        .put(&url, &object.to_ics(), Precondition::Match(&etag))
        .expect("Ändern mit If-Match");
    let (data, _) = client.get(&url).unwrap();
    let (events, _) = CalendarObject::parse(&data).unwrap().events();
    assert_eq!(
        events[0].start,
        DateTimeValue::Zoned(at("2026-11-13 14:00"), Berlin)
    );
    assert_eq!(
        events[0].end,
        DateTimeValue::Zoned(at("2026-11-13 15:30"), Berlin)
    );
}

/// Serien: Ausnahme, gelöschtes Vorkommen und geteilte Serie. Der Server
/// muss Ausnahmen in derselben Ressource behalten und beide Hälften einer
/// geteilten Serie annehmen.
fn exercise_series(client: &Client, calendar: &Url) {
    use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
    use chrono_tz::Europe::Berlin;
    use hora_core::edit::{self, EventFields};
    use hora_core::expand::expand;
    use hora_core::series::{self, RuleChange, Scope, Target};
    use hora_core::time::DateTimeValue;

    let wall = |s: &str| {
        DateTimeValue::Zoned(
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap(),
            Berlin,
        )
    };
    let at = |s: &str| wall(s).to_utc(Berlin);
    let now = Utc.with_ymd_and_hms(2026, 11, 4, 12, 0, 0).unwrap();
    let fields = |summary: &str, start: &str, end: &str| EventFields {
        summary: summary.into(),
        location: "Raum 204".into(),
        description: String::new(),
        start: wall(start),
        end: wall(end),
    };
    let starts = |objects: &[&CalendarObject]| -> Vec<(DateTime<Utc>, String)> {
        let mut all: Vec<_> = objects
            .iter()
            .flat_map(|o| {
                expand(
                    &o.events().0,
                    Berlin,
                    at("2026-11-01 00:00"),
                    at("2026-12-15 00:00"),
                )
            })
            .map(|o| (o.start, o.summary))
            .collect();
        all.sort();
        all
    };
    let expect = |list: &[(&str, &str)]| -> Vec<(DateTime<Utc>, String)> {
        list.iter().map(|(t, s)| (at(t), s.to_string())).collect()
    };

    // Jeden Montag 8:00–9:30
    let url = calendar.join("serie.ics").unwrap();
    let mut object = edit::new_event(
        "serie",
        &fields("Mathe", "2026-11-02 08:00", "2026-11-02 09:30"),
        now,
    );
    series::set_rule(&mut object, "serie", Some("FREQ=WEEKLY;BYDAY=MO")).unwrap();
    client
        .put(&url, &object.to_ics(), Precondition::Create)
        .expect("Serie anlegen");

    // Nur dieser: 16.11. auf Dienstag 10:00
    let (data, etag) = client.get(&url).unwrap();
    let mut object = CalendarObject::parse(&data).unwrap();
    let target = |t: &str| Target {
        uid: "serie",
        instance: at(t),
    };
    let moved = fields("Mathe (verlegt)", "2026-11-17 10:00", "2026-11-17 11:30");
    series::edit(
        &mut object,
        target("2026-11-16 08:00"),
        Scope::This,
        &moved,
        &RuleChange::Keep,
        "",
        Berlin,
        now,
    )
    .unwrap();
    // Nur dieser löschen: 23.11.
    series::delete(
        &mut object,
        target("2026-11-23 08:00"),
        Scope::This,
        Berlin,
        now,
    )
    .unwrap();
    client
        .put(&url, &object.to_ics(), Precondition::Match(&etag.unwrap()))
        .expect("Ausnahme und EXDATE hochladen");
    let (data, etag) = client.get(&url).unwrap();
    let mut object = CalendarObject::parse(&data).unwrap();
    assert_eq!(
        object.root.children_named("VEVENT").count(),
        2,
        "Ausnahme bleibt in der Ressource"
    );
    assert_eq!(
        starts(&[&object])[..4].to_vec(),
        expect(&[
            ("2026-11-02 08:00", "Mathe"),
            ("2026-11-09 08:00", "Mathe"),
            ("2026-11-17 10:00", "Mathe (verlegt)"),
            ("2026-11-30 08:00", "Mathe"),
        ])
    );

    // Ab 30.11. alle folgenden um 9:00 und mit neuem Titel: zwei Ressourcen
    let later = fields("Mathe LK", "2026-11-30 09:00", "2026-11-30 10:30");
    let new = series::edit(
        &mut object,
        target("2026-11-30 08:00"),
        Scope::Following,
        &later,
        &RuleChange::Keep,
        "serie-2",
        Berlin,
        now,
    )
    .unwrap()
    .expect("neue Serie");
    client
        .put(&url, &object.to_ics(), Precondition::Match(&etag.unwrap()))
        .expect("alte Serie kürzen");
    let new_url = calendar.join("serie-2.ics").unwrap();
    client
        .put(&new_url, &new.to_ics(), Precondition::Create)
        .expect("neue Serie anlegen");
    let old = CalendarObject::parse(&client.get(&url).unwrap().0).unwrap();
    let new = CalendarObject::parse(&client.get(&new_url).unwrap().0).unwrap();
    assert_eq!(
        starts(&[&old, &new]),
        expect(&[
            ("2026-11-02 08:00", "Mathe"),
            ("2026-11-09 08:00", "Mathe"),
            ("2026-11-17 10:00", "Mathe (verlegt)"),
            ("2026-11-30 09:00", "Mathe LK"),
            ("2026-12-07 09:00", "Mathe LK"),
            ("2026-12-14 09:00", "Mathe LK"),
        ])
    );
}

/// Aufgaben: Fälligkeit, Teilaufgabe, erledigen, ein Vorkommen einer Serie
/// erledigen. Der Server muss VTODO annehmen und unverändert zurückgeben.
fn exercise_tasks(client: &Client, calendar: &Url) {
    use chrono::{NaiveDateTime, TimeZone, Utc};
    use chrono_tz::Europe::Berlin;
    use hora_core::task::{self, TaskFields};
    use hora_core::time::DateTimeValue;

    let now = Utc.with_ymd_and_hms(2026, 11, 5, 12, 0, 0).unwrap();
    let due = DateTimeValue::Zoned(
        NaiveDateTime::parse_from_str("2026-11-10 18:00", "%Y-%m-%d %H:%M").unwrap(),
        Berlin,
    );
    let fields = |summary: &str, parent: Option<&str>| TaskFields {
        summary: summary.into(),
        description: "Notiz".into(),
        due: Some(due),
        start: None,
        priority: 1,
        parent: parent.map(str::to_string),
    };

    let parent_url = calendar.join("aufgabe.ics").unwrap();
    let child_url = calendar.join("teilaufgabe.ics").unwrap();
    client
        .put(
            &parent_url,
            &task::new_task("aufgabe", &fields("Umzug planen", None), Berlin, now).to_ics(),
            Precondition::Create,
        )
        .expect("Aufgabe anlegen");
    client
        .put(
            &child_url,
            &task::new_task(
                "teil",
                &fields("Kartons kaufen", Some("aufgabe")),
                Berlin,
                now,
            )
            .to_ics(),
            Precondition::Create,
        )
        .expect("Teilaufgabe anlegen");

    let (data, etag) = client.get(&child_url).unwrap();
    let mut object = CalendarObject::parse(&data).unwrap();
    let child = &object.tasks()[0];
    assert_eq!(child.parent.as_deref(), Some("aufgabe"));
    assert_eq!(child.due, Some(due));
    assert_eq!(child.priority, 1);
    task::set_completed(&mut object, "teil", true, now).unwrap();
    client
        .put(
            &child_url,
            &object.to_ics(),
            Precondition::Match(&etag.unwrap()),
        )
        .expect("Erledigt hochladen");
    let back = CalendarObject::parse(&client.get(&child_url).unwrap().0).unwrap();
    assert!(back.tasks()[0].completed, "Server behält STATUS:COMPLETED");

    // Wiederkehrende Aufgabe: ein Vorkommen erledigen
    let series_url = calendar.join("sport.ics").unwrap();
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Hora//Test//DE\r\nBEGIN:VTODO\r\nUID:sport\r\nDTSTAMP:20261105T120000Z\r\n\
               SUMMARY:Sport\r\nDTSTART;VALUE=DATE:20261103\r\nDUE;VALUE=DATE:20261103\r\nRRULE:FREQ=WEEKLY;BYDAY=TU\r\n\
               STATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
    client
        .put(&series_url, ics, Precondition::Create)
        .expect("Serie anlegen");
    let (data, etag) = client.get(&series_url).unwrap();
    let mut object = CalendarObject::parse(&data).unwrap();
    let current = task::series_state(&object.tasks(), Berlin)
        .unwrap()
        .current
        .unwrap();
    task::complete_occurrence(&mut object, "sport", &current, true, Berlin, now).unwrap();
    client
        .put(
            &series_url,
            &object.to_ics(),
            Precondition::Match(&etag.unwrap()),
        )
        .expect("Vorkommen erledigen");
    let back = CalendarObject::parse(&client.get(&series_url).unwrap().0).unwrap();
    assert_eq!(
        back.tasks().len(),
        2,
        "Serie und erledigtes Vorkommen in einer Ressource"
    );
    let state = task::series_state(&back.tasks(), Berlin).unwrap();
    assert_eq!(state.completed, 1);
    assert_eq!(
        state.current.unwrap().due,
        Some(DateTimeValue::Date(
            chrono::NaiveDate::from_ymd_opt(2026, 11, 10).unwrap()
        ))
    );
}

#[test]
#[ignore = "braucht Docker"]
fn radicale_full_flow() {
    let server = radicale();
    exercise(&server);
}

#[test]
#[ignore = "braucht Docker"]
fn baikal_full_flow() {
    let server = baikal();
    exercise(&server);
}
