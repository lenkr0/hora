//! Passwörter im Windows-Anmeldeinformationsspeicher (Systemsteuerung →
//! Anmeldeinformationsverwaltung → Windows-Anmeldeinformationen, Einträge
//! `<schlüssel>.Hora`). Die Datenbank kennt nur den Schlüssel, nie das
//! Passwort.
//!
//! Mit der Umgebungsvariable `HORA_CREDENTIALS=memory` bleiben Passwörter
//! nur im Arbeitsspeicher; das nutzen Tests und der Selbsttest, damit sie
//! den echten Speicher nicht berühren.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const SERVICE: &str = "Hora";

#[derive(Debug, thiserror::Error)]
#[error("Anmeldeinformationsspeicher: {0}")]
pub struct CredentialError(String);

enum Backend {
    Native,
    Memory(Mutex<HashMap<String, String>>),
}

static BACKEND: OnceLock<Result<Backend, String>> = OnceLock::new();

fn backend() -> Result<&'static Backend, CredentialError> {
    BACKEND
        .get_or_init(|| {
            if std::env::var("HORA_CREDENTIALS").is_ok_and(|v| v == "memory") {
                return Ok(Backend::Memory(Mutex::new(HashMap::new())));
            }
            native_store()?;
            Ok(Backend::Native)
        })
        .as_ref()
        .map_err(|e| CredentialError(e.clone()))
}

#[cfg(windows)]
fn native_store() -> Result<(), String> {
    let store = windows_native_keyring_store::Store::new().map_err(|e| e.to_string())?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(not(windows))]
fn native_store() -> Result<(), String> {
    Err("auf diesem System noch nicht unterstützt".into())
}

fn entry(key: &str) -> Result<keyring_core::Entry, CredentialError> {
    keyring_core::Entry::new(SERVICE, key).map_err(|e| CredentialError(e.to_string()))
}

pub fn save(key: &str, password: &str) -> Result<(), CredentialError> {
    match backend()? {
        Backend::Memory(map) => {
            map.lock().unwrap().insert(key.into(), password.into());
            Ok(())
        }
        Backend::Native => entry(key)?
            .set_password(password)
            .map_err(|e| CredentialError(e.to_string())),
    }
}

/// `None`, wenn es zum Schlüssel kein Passwort gibt.
pub fn load(key: &str) -> Result<Option<String>, CredentialError> {
    match backend()? {
        Backend::Memory(map) => Ok(map.lock().unwrap().get(key).cloned()),
        Backend::Native => match entry(key)?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(CredentialError(e.to_string())),
        },
    }
}

pub fn delete(key: &str) -> Result<(), CredentialError> {
    match backend()? {
        Backend::Memory(map) => {
            map.lock().unwrap().remove(key);
            Ok(())
        }
        Backend::Native => match entry(key)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(CredentialError(e.to_string())),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Schreibt kurz einen Eintrag `hora-test-<pid>.Hora` in den echten
    /// Speicher und löscht ihn wieder.
    #[test]
    #[ignore = "schreibt in den echten Windows-Anmeldeinformationsspeicher"]
    fn native_roundtrip() {
        let key = format!("hora-test-{}", std::process::id());
        save(&key, "geheim äöü").unwrap();
        assert_eq!(load(&key).unwrap().as_deref(), Some("geheim äöü"));
        delete(&key).unwrap();
        assert_eq!(load(&key).unwrap(), None);
        delete(&key).unwrap();
    }
}
