//! Sync-Thread: Konten anlegen, lokale Änderungen hochladen und Kalender
//! abgleichen, ohne die Oberfläche zu blockieren. Der Thread hat eine eigene
//! Schreibverbindung zur Datenbank; Ergebnisse meldet er über `notify`.

use hora_i18n::tr;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{SystemTime, UNIX_EPOCH};

use hora_caldav::{CalendarInfo, Client, Error, Precondition};
use hora_core::model::Kind;
use hora_platform::credentials;
use hora_storage::{NewAccount, OpState, Operation, PendingOp, RemoteCalendar, Store, SyncBatch};
use url::Url;

pub enum Command {
    AddAccount {
        name: String,
        url: String,
        username: String,
        password: String,
    },
    SyncAll,
    /// Wartende lokale Änderungen hochladen
    Push,
    /// Konflikt auflösen: eigene Version behalten oder Server-Version übernehmen
    ResolveConflict {
        op_id: i64,
        keep_mine: bool,
    },
    /// Neuen Kalender im Konto anlegen (z. B. für Journaleinträge) und abgleichen
    CreateCalendar {
        account_id: i64,
        name: String,
        components: Vec<&'static str>,
    },
}

pub enum Update {
    AccountAdded,
    AddAccountFailed(String),
    SyncStarted,
    /// Kalender, Termine oder Warteschlange in der Datenbank haben sich geändert
    DataChanged,
    SyncFinished {
        errors: Vec<String>,
    },
    /// Ergebnis von [`Command::CreateCalendar`]; `None` = angelegt
    CalendarCreated(Option<String>),
}

/// Wartezeit vor dem n-ten erneuten Versuch: 30 s, 1 min, 2 min … höchstens 1 h.
pub fn retry_delay(attempts: i64) -> i64 {
    (30_i64 << attempts.clamp(0, 7)).min(3600)
}

pub struct Worker {
    tx: Sender<Command>,
}

impl Worker {
    pub fn spawn(db: PathBuf, notify: impl Fn(Update) + Send + 'static) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("hora-sync".into())
            .spawn(move || run(db, rx, notify))?;
        Ok(Worker { tx })
    }

    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

fn run(db: PathBuf, rx: Receiver<Command>, notify: impl Fn(Update)) {
    let mut store = match Store::open(&db) {
        Ok(store) => store,
        Err(e) => {
            notify(Update::SyncFinished {
                errors: vec![e.to_string()],
            });
            return;
        }
    };
    let mut queue: VecDeque<Command> = VecDeque::new();
    loop {
        let command = match queue.pop_front() {
            Some(command) => command,
            None => match rx.recv() {
                Ok(command) => command,
                Err(_) => return,
            },
        };
        // Weitere wartende Befehle einsammeln; nichts geht verloren
        queue.extend(rx.try_iter());
        match command {
            Command::AddAccount {
                name,
                url,
                username,
                password,
            } => match add_account(&mut store, &name, &url, &username, &password) {
                Ok(()) => {
                    notify(Update::AccountAdded);
                    notify(Update::DataChanged);
                    sync_all(&mut store, &notify);
                }
                Err(message) => notify(Update::AddAccountFailed(message)),
            },
            Command::SyncAll => {
                // Ein Sync lädt ohnehin zuerst hoch: doppelte Wünsche entfallen
                queue.retain(|c| !matches!(c, Command::SyncAll | Command::Push));
                sync_all(&mut store, &notify);
            }
            Command::Push => {
                queue.retain(|c| !matches!(c, Command::Push));
                let errors = push_pending(&mut store, &notify);
                if !errors.is_empty() {
                    notify(Update::SyncFinished { errors });
                }
            }
            Command::CreateCalendar {
                account_id,
                name,
                components,
            } => match create_calendar(&store, account_id, &name, &components) {
                Ok(()) => {
                    sync_all(&mut store, &notify);
                    notify(Update::CalendarCreated(None));
                }
                Err(message) => notify(Update::CalendarCreated(Some(message))),
            },
            Command::ResolveConflict { op_id, keep_mine } => {
                let errors = match resolve_conflict(&mut store, op_id, keep_mine) {
                    Ok(()) if keep_mine => push_pending(&mut store, &notify),
                    Ok(()) => {
                        sync_all(&mut store, &notify);
                        Vec::new()
                    }
                    Err(message) => vec![message],
                };
                notify(Update::DataChanged);
                if !errors.is_empty() {
                    notify(Update::SyncFinished { errors });
                }
            }
        }
    }
}

fn save_calendar_list(
    store: &mut Store,
    account_id: i64,
    calendars: &[CalendarInfo],
) -> hora_storage::Result<()> {
    let components: Vec<String> = calendars.iter().map(|c| c.components.join(",")).collect();
    let list: Vec<RemoteCalendar> = calendars
        .iter()
        .zip(&components)
        .map(|(c, components)| RemoteCalendar {
            href: &c.url,
            name: &c.name,
            color: c.color.as_deref(),
            components,
            read_only: c.read_only,
        })
        .collect();
    store.sync_calendar_list(account_id, &list)
}

/// Kalender mit eindeutigem Pfad unter dem Kalender-Home des Kontos anlegen
fn create_calendar(
    store: &Store,
    account_id: i64,
    name: &str,
    components: &[&str],
) -> Result<(), String> {
    let account = store
        .accounts()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|a| a.id == account_id)
        .ok_or(tr!("Account no longer exists"))?;
    let home = account
        .home_url
        .as_deref()
        .and_then(|h| Url::parse(h).ok())
        .ok_or(tr!(
            "The account doesn't report a location for new calendars."
        ))?;
    let client = client_for(store, account_id)?;
    let slug = format!("hora-{}-{:x}", name.to_lowercase(), unix_now());
    client
        .make_calendar(&home, &slug, name, None, components)
        .map(|_| ())
        .map_err(|e| tr!("Create calendar: {}", e))
}

fn credential_key() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("konto-{nanos:x}")
}

fn add_account(
    store: &mut Store,
    name: &str,
    url: &str,
    username: &str,
    password: &str,
) -> Result<(), String> {
    let address = hora_caldav::parse_address(url).map_err(|e| e.to_string())?;
    let existing = store.accounts().map_err(|e| e.to_string())?;
    if existing
        .iter()
        .any(|a| a.url == address.as_str() && a.username == username)
    {
        return Err(tr!("This account is already set up.").into());
    }
    let client = Client::new(username, password);
    let discovery = client
        .discover(address.as_str())
        .map_err(|e| e.to_string())?;
    if discovery.calendars.is_empty() {
        return Err(tr!("Sign-in worked, but the server reports no calendars.").into());
    }
    let key = credential_key();
    credentials::save(&key, password).map_err(|e| e.to_string())?;
    let name = match name.trim() {
        "" => address.host_str().unwrap_or("CalDAV").to_string(),
        name => name.to_string(),
    };
    let result = (|| {
        let id = store.add_account(&NewAccount {
            name: &name,
            url: address.as_str(),
            username,
            credential_key: &key,
            principal_url: Some(discovery.principal.as_str()),
            home_url: Some(discovery.home.as_str()),
        })?;
        save_calendar_list(store, id, &discovery.calendars)
    })();
    result.map_err(|e| {
        let _ = credentials::delete(&key);
        e.to_string()
    })
}

/// Client mit gespeichertem Passwort für ein Konto.
fn client_for(store: &Store, account_id: i64) -> Result<Client, String> {
    let accounts = store.accounts().map_err(|e| e.to_string())?;
    let account = accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or(tr!("Account no longer exists"))?;
    match credentials::load(&account.credential_key) {
        Ok(Some(password)) => Ok(Client::new(&account.username, &password)),
        Ok(None) => Err(tr!(
            "{}: password missing from Credential Manager",
            account.name
        )),
        Err(e) => Err(format!("{}: {e}", account.name)),
    }
}

fn resource_url(op: &PendingOp) -> Result<Url, Error> {
    Url::parse(&op.calendar_url)
        .and_then(|c| c.join(&op.href))
        .map_err(|e| Error::InvalidUrl(e.to_string()))
}

/// Eine Änderung hochladen. Liefert ETag und, falls der Server die Daten beim
/// Speichern verändert hat, seine Fassung.
fn push_one(client: &Client, op: &PendingOp) -> Result<(Option<String>, Option<String>), Error> {
    let url = resource_url(op)?;
    match op.operation {
        Operation::Put => {
            let condition = match &op.if_match {
                Some(etag) => Precondition::Match(etag),
                None => Precondition::Create,
            };
            match client.put(&url, op.ics.as_deref().unwrap_or_default(), condition)? {
                Some(etag) => Ok((Some(etag), None)),
                // Kein ETag heißt laut RFC 4791: der Server hat verändert gespeichert
                None => {
                    let (data, etag) = client.get(&url)?;
                    Ok((etag, Some(data)))
                }
            }
        }
        Operation::Delete => {
            client.delete(&url, op.if_match.as_deref())?;
            Ok((None, None))
        }
    }
}

/// Fällige Änderungen hochladen; liefert Fehlermeldungen für die Anzeige.
fn push_pending(store: &mut Store, notify: &impl Fn(Update)) -> Vec<String> {
    let now = unix_now();
    let ops = match store.pending_due(now) {
        Ok(ops) => ops,
        Err(e) => return vec![e.to_string()],
    };
    if ops.is_empty() {
        return Vec::new();
    }
    let mut clients: HashMap<i64, Result<Client, String>> = HashMap::new();
    let mut errors = Vec::new();
    for op in ops {
        let client = clients
            .entry(op.account_id)
            .or_insert_with(|| client_for(store, op.account_id));
        let client = match client {
            Ok(client) => client,
            Err(message) => {
                errors.push(message.clone());
                continue;
            }
        };
        let result = match push_one(client, &op) {
            Ok((etag, data)) => store.complete_pending(&op, etag.as_deref(), data.as_deref()),
            Err(Error::Conflict) => {
                store.mark_pending(op.id, OpState::Conflict, &Error::Conflict.to_string())
            }
            Err(
                e @ (Error::Network(_)
                | Error::Status {
                    status: 500..=599, ..
                }),
            ) => {
                errors.push(tr!("Change waiting: {}", e));
                store.retry_pending_later(op.id, now + retry_delay(op.attempts), &e.to_string())
            }
            Err(e) => {
                errors.push(tr!("Change rejected: {}", e));
                store.mark_pending(op.id, OpState::Failed, &e.to_string())
            }
        };
        if let Err(e) = result {
            errors.push(e.to_string());
        }
    }
    notify(Update::DataChanged);
    errors
}

fn resolve_conflict(store: &mut Store, op_id: i64, keep_mine: bool) -> Result<(), String> {
    let op = store
        .pending_ops()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|o| o.id == op_id)
        .ok_or(tr!("Conflict is already resolved"))?;
    if !keep_mine {
        return store.discard_pending(op.id).map_err(|e| e.to_string());
    }
    // Gegen den aktuellen Server-Stand erneut senden
    let client = client_for(store, op.account_id)?;
    let url = resource_url(&op).map_err(|e| e.to_string())?;
    let server_etag = match client.get(&url) {
        Ok((_, etag)) => etag,
        Err(Error::NotFound(_)) => None,
        Err(e) => return Err(e.to_string()),
    };
    let result = if op.operation == Operation::Delete && server_etag.is_none() {
        // Schon gelöscht: nichts mehr zu tun
        store.complete_pending(&op, None, None)
    } else {
        store.retry_pending_against(op.id, server_etag.as_deref())
    };
    result.map_err(|e| e.to_string())
}

fn sync_all(store: &mut Store, notify: &impl Fn(Update)) {
    // Kalender nur auf diesem PC haben keinen Server
    let accounts = match store.accounts() {
        Ok(accounts) if accounts.iter().any(|a| !a.local) => accounts
            .into_iter()
            .filter(|a| !a.local)
            .collect::<Vec<_>>(),
        Ok(_) => return,
        Err(e) => {
            notify(Update::SyncFinished {
                errors: vec![e.to_string()],
            });
            return;
        }
    };
    notify(Update::SyncStarted);
    // Erst hochladen, damit der Abgleich die eigenen Änderungen schon kennt
    let mut errors = push_pending(store, notify);
    for account in accounts {
        let client = match client_for(store, account.id) {
            Ok(client) => client,
            Err(message) => {
                errors.push(message);
                continue;
            }
        };

        // Kalenderliste auffrischen (neue, umbenannte, gelöschte Kalender)
        if let Some(home) = account.home_url.as_deref().and_then(|h| Url::parse(h).ok()) {
            match client.calendars(&home) {
                Ok(list) => {
                    if let Err(e) = save_calendar_list(store, account.id, &list) {
                        errors.push(e.to_string());
                    }
                    notify(Update::DataChanged);
                }
                Err(e) => {
                    errors.push(format!("{}: {e}", account.name));
                    continue;
                }
            }
        }

        let calendars = store.calendars().unwrap_or_default();
        // Auch reine Aufgabenlisten und Journal-Kalender
        for calendar in calendars.iter().filter(|c| {
            c.account_id == account.id
                && [Kind::Event, Kind::Todo, Kind::Journal]
                    .into_iter()
                    .any(|k| c.supports(k))
        }) {
            let known = store.etags(calendar.id).unwrap_or_default();
            let outcome = match client.sync(
                &calendar.href,
                &known,
                calendar.ctag.as_deref(),
                calendar.sync_token.as_deref(),
            ) {
                Ok(outcome) => outcome,
                Err(e) => {
                    errors.push(format!("{}: {e}", calendar.name));
                    continue;
                }
            };
            if outcome.skipped {
                continue;
            }
            let batch = SyncBatch {
                upserts: outcome
                    .changed
                    .iter()
                    .map(|o| (o.href.as_str(), o.etag.as_str(), o.data.as_str()))
                    .collect(),
                deletes: outcome.removed.iter().map(String::as_str).collect(),
                full: outcome.full,
                keep: outcome.unchanged.iter().map(String::as_str).collect(),
                ctag: outcome.ctag.as_deref(),
                sync_token: outcome.sync_token.as_deref(),
            };
            match store.apply_sync(calendar.id, &batch) {
                Ok(()) => notify(Update::DataChanged),
                Err(e) => errors.push(format!("{}: {e}", calendar.name)),
            }
        }
    }
    notify(Update::SyncFinished { errors });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kalender nur auf diesem PC: kein Abgleich, nichts hochzuladen
    #[test]
    fn local_calendars_are_not_synced() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.add_local_calendar("Zuhause", "#4f7cac").unwrap();
        let ics = "BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:x
DTSTART:20261116T090000Z
DTEND:20261116T100000Z
END:VEVENT
END:VCALENDAR
";
        store.local_put(id, "x.ics", ics).unwrap();
        let updates = std::cell::Cell::new(0);
        sync_all(&mut store, &|_| updates.set(updates.get() + 1));
        assert_eq!(updates.get(), 0);
        assert!(push_pending(&mut store, &|_| {}).is_empty());
        assert!(store.resource(id, "x.ics").unwrap().is_some());
    }

    /// Ohne Netz bleibt die Änderung erhalten und wird später erneut versucht.
    #[test]
    fn unreachable_server_keeps_change_for_later() {
        // SAFETY: Kein anderer Test liest Umgebungsvariablen; der Speicher für
        // Passwörter wird hier zum ersten Mal benutzt.
        unsafe { std::env::set_var("HORA_CREDENTIALS", "memory") };
        let mut store = Store::open_in_memory().unwrap();
        let account = store
            .add_account(&NewAccount {
                name: "Offline",
                // Port 9 auf localhost ist praktisch nie belegt
                url: "http://127.0.0.1:9/",
                username: "u",
                credential_key: "k-offline",
                principal_url: None,
                home_url: None,
            })
            .unwrap();
        credentials::save("k-offline", "pw").unwrap();
        store
            .sync_calendar_list(
                account,
                &[RemoteCalendar {
                    href: "http://127.0.0.1:9/cal/",
                    name: "Kalender",
                    color: None,
                    components: "",
                    read_only: false,
                }],
            )
            .unwrap();
        let calendar = store.calendars().unwrap()[0].id;
        let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:20261109T070000Z\r\nDTEND:20261109T080000Z\r\nSUMMARY:Offline angelegt\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        store.local_put(calendar, "/cal/x.ics", ics).unwrap();

        let errors = push_pending(&mut store, &|_| {});
        assert!(
            errors.iter().any(|e| e.contains("not reachable")),
            "{errors:?}"
        );
        let ops = store.pending_ops().unwrap();
        assert_eq!(ops.len(), 1, "Änderung bleibt erhalten");
        assert_eq!(ops[0].state, OpState::Pending);
        assert_eq!(ops[0].attempts, 1);
        assert!(
            store.pending_due(unix_now()).unwrap().is_empty(),
            "erst nach der Wartezeit erneut"
        );
        assert_eq!(
            store
                .pending_due(unix_now() + retry_delay(0))
                .unwrap()
                .len(),
            1
        );
        // Der Termin ist lokal weiter sichtbar
        assert_eq!(store.events_in_range(0, i64::MAX / 2).unwrap().len(), 1);
    }

    #[test]
    fn retry_delay_doubles_up_to_an_hour() {
        assert_eq!(retry_delay(0), 30);
        assert_eq!(retry_delay(1), 60);
        assert_eq!(retry_delay(3), 240);
        assert_eq!(retry_delay(7), 3600);
        assert_eq!(retry_delay(50), 3600);
    }
}
