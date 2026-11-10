//! Updates für Hora: neuestes Release finden, Setup laden und prüfen.
//!
//! Zu jedem Release gehören zwei kleine Dateien, die der Release-Workflow mit
//! `hora-sign` anlegt: `update.txt` nennt Version, Dateiname, Größe und
//! SHA-256 des Setups, `update.sig` ist die Ed25519-Signatur dieser Datei.
//! Den öffentlichen Schlüssel trägt Hora in sich ([`PUBLIC_KEY`]), den
//! geheimen kennt nur der Release-Workflow (Secret `HORA_UPDATE_KEY`).
//!
//! Hora nimmt ein Setup nur, wenn die Signatur passt und Größe und Prüfsumme
//! der geladenen Datei stimmen; ob die Version neuer ist, entscheidet die App.

use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use hora_i18n::tr;
use ring::digest::{Context, SHA256};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};

/// Öffentlicher Schlüssel der Releases (Ed25519). Der geheime liegt als
/// Secret `HORA_UPDATE_KEY` bei GitHub; ändert er sich, nehmen bestehende
/// Installationen keine Updates mehr an.
pub const PUBLIC_KEY: [u8; 32] = [
    0x0f, 0x58, 0x7d, 0x9e, 0xc4, 0x24, 0xce, 0xab, 0x63, 0xe9, 0xa4, 0xb9, 0x51, 0x12, 0xea, 0x94,
    0xeb, 0xd8, 0x3a, 0x86, 0x58, 0xce, 0xf8, 0x5a, 0x96, 0x1d, 0x89, 0x5c, 0xe8, 0x3e, 0x5c, 0x87,
];

/// Releases auf GitHub; `latest/download/<Datei>` führt zum neuesten
pub const RELEASES: &str = "https://github.com/lenkr0/hora/releases";
pub const MANIFEST: &str = "update.txt";
pub const SIGNATURE: &str = "update.sig";

const HEADER: &str = "hora-update 1";
/// Größer wird kein Setup; schützt vor endlosen Downloads
const MAX_SIZE: u64 = 256 * 1024 * 1024;
/// Für update.txt und update.sig
const SMALL_LIMIT: u64 = 64 * 1024;

/// Version wie in Cargo.toml (nur Zahlen, keine Vorabversionen)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// "1.2.3" oder "v1.2.3"
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.split('.').map(|part| {
            let digits = !part.is_empty() && part.len() <= 9;
            (digits && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse::<u32>().ok())
                .flatten()
        });
        let version = Version(parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// Dateiname des Setups einer Version
pub fn setup_name(version: Version) -> String {
    format!("Hora-Setup-{version}.exe")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Keine Verbindung, Zeitüberschreitung, Verbindung abgebrochen
    Network(String),
    /// Fehlercode des Servers; 404: kein Release (oder Repository privat)
    Status(u16),
    Manifest(String),
    Signature,
    /// Größe oder Prüfsumme der geladenen Datei stimmen nicht
    Damaged(String),
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Network(e) => f.write_str(&tr!("no connection to the update server ({})", e)),
            Error::Status(404) => f.write_str(tr!("no published version found")),
            Error::Status(code) => {
                f.write_str(&tr!("the update server answered with error {}", code))
            }
            Error::Manifest(e) => f.write_str(&tr!("the update information is invalid ({})", e)),
            Error::Signature => f.write_str(tr!("the update isn't signed with Hora's release key")),
            Error::Damaged(e) => f.write_str(&tr!("the download is damaged ({})", e)),
            Error::Io(e) => f.write_str(&tr!("the update can't be saved ({})", e)),
        }
    }
}

impl std::error::Error for Error {}

fn io_error(e: std::io::Error) -> Error {
    Error::Io(e.to_string())
}

fn net_error(e: ureq::Error) -> Error {
    match e {
        ureq::Error::StatusCode(code) => Error::Status(code),
        other => Error::Network(other.to_string()),
    }
}

/// Inhalt von update.txt
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub version: Version,
    /// Immer [`setup_name`] der Version
    pub file: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

impl Manifest {
    /// Für `hora-sign`: Version aus dem Dateinamen, Größe und Prüfsumme
    pub fn for_file(path: &Path) -> Result<Manifest, Error> {
        let file = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let version = file
            .strip_prefix("Hora-Setup-")
            .and_then(|rest| rest.strip_suffix(".exe"))
            .and_then(Version::parse)
            .ok_or_else(|| Error::Manifest(format!("{file}: expected Hora-Setup-<version>.exe")))?;
        let (size, sha256) = hash_file(path)?;
        Ok(Manifest {
            version,
            file: file.to_string(),
            size,
            sha256,
        })
    }

    pub fn render(&self) -> String {
        format!(
            "{HEADER}\nversion {}\nfile {}\nsize {}\nsha256 {}\n",
            self.version,
            self.file,
            self.size,
            hex(&self.sha256)
        )
    }

    pub fn parse(text: &str) -> Result<Manifest, Error> {
        let bad = |what: &str| Error::Manifest(what.to_string());
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        if lines.next() != Some(HEADER) {
            return Err(bad("unknown format"));
        }
        let (mut version, mut file, mut size, mut sha256) = (None, None, None, None);
        for line in lines {
            let (key, value) = line.split_once(' ').ok_or_else(|| bad(line))?;
            let value = value.trim();
            let twice = match key {
                "version" => version
                    .replace(Version::parse(value).ok_or_else(|| bad("version"))?)
                    .is_some(),
                "file" => file.replace(value.to_string()).is_some(),
                "size" => size
                    .replace(value.parse::<u64>().map_err(|_| bad("size"))?)
                    .is_some(),
                "sha256" => sha256
                    .replace(unhex(value).ok_or_else(|| bad("sha256"))?)
                    .is_some(),
                // Spätere Felder stören ältere Versionen nicht
                _ => false,
            };
            if twice {
                return Err(bad(&format!("{key} twice")));
            }
        }
        let version = version.ok_or_else(|| bad("version missing"))?;
        let file = file.ok_or_else(|| bad("file missing"))?;
        let size = size.ok_or_else(|| bad("size missing"))?;
        let sha256 = sha256.ok_or_else(|| bad("sha256 missing"))?;
        if file != setup_name(version) {
            return Err(bad("file name doesn't match the version"));
        }
        if size == 0 || size > MAX_SIZE {
            return Err(bad("size"));
        }
        Ok(Manifest {
            version,
            file,
            size,
            sha256,
        })
    }
}

/// Signatur prüfen, erst dann lesen
pub fn verify(manifest: &[u8], signature: &str, key: &[u8; 32]) -> Result<Manifest, Error> {
    let signature = BASE64
        .decode(signature.trim())
        .map_err(|_| Error::Signature)?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(manifest, &signature)
        .map_err(|_| Error::Signature)?;
    let text =
        std::str::from_utf8(manifest).map_err(|_| Error::Manifest("not UTF-8".to_string()))?;
    Manifest::parse(text)
}

/// Signatur (Base64) mit einem geheimen Schlüssel im PKCS#8-Format
pub fn sign(manifest: &[u8], pkcs8: &[u8]) -> Result<String, String> {
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|e| e.to_string())?;
    Ok(BASE64.encode(pair.sign(manifest).as_ref()))
}

/// Öffentlicher Schlüssel zu einem geheimen (PKCS#8)
pub fn public_key(pkcs8: &[u8]) -> Result<[u8; 32], String> {
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|e| e.to_string())?;
    pair.public_key()
        .as_ref()
        .try_into()
        .map_err(|_| "unexpected key length".to_string())
}

/// Größe und SHA-256 einer Datei
pub fn hash_file(path: &Path) -> Result<(u64, [u8; 32]), Error> {
    let mut file = File::open(path).map_err(io_error)?;
    let mut context = Context::new(&SHA256);
    let mut buffer = vec![0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buffer).map_err(io_error)?;
        if n == 0 {
            break;
        }
        context.update(&buffer[..n]);
        size += n as u64;
    }
    Ok((size, digest(context)))
}

/// Passt die Datei zu update.txt? (Vor dem Start des Setups noch einmal)
pub fn check_file(path: &Path, manifest: &Manifest) -> Result<(), Error> {
    let (size, sha256) = hash_file(path)?;
    if size != manifest.size {
        return Err(Error::Damaged(format!(
            "{size} instead of {} bytes",
            manifest.size
        )));
    }
    if sha256 != manifest.sha256 {
        return Err(Error::Damaged("SHA-256".to_string()));
    }
    Ok(())
}

/// Im Download-Ordner aufräumen: halbe Downloads und Setups, die nicht neuer
/// als `current` sind (nach einem Update ist das geladene Setup alt)
pub fn clean(dir: &Path, current: Version) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let stale = name.ends_with(".part")
            || name
                .strip_prefix("Hora-Setup-")
                .and_then(|rest| rest.strip_suffix(".exe"))
                .and_then(Version::parse)
                .is_some_and(|v| v <= current);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Wo die Updates liegen und wer sie signiert
pub struct Source {
    base: String,
    key: [u8; 32],
    agent: ureq::Agent,
}

impl Source {
    /// `base` wie [`RELEASES`], in Tests ein lokaler Server
    pub fn new(base: &str, key: [u8; 32]) -> Source {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_connect(Some(Duration::from_secs(15)))
            // Kein Gesamtlimit: 16 MB brauchen bei langsamer Leitung Minuten.
            // Hängt die Verbindung, bricht der Download nach 30 s ab.
            .timeout_per_read(Some(Duration::from_secs(30)))
            .user_agent(concat!("Hora/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Source {
            base: base.trim_end_matches('/').to_string(),
            key,
            agent,
        }
    }

    /// update.txt des neuesten Releases, Signatur geprüft
    pub fn latest(&self) -> Result<Manifest, Error> {
        let text = self.small(&format!("{}/latest/download/{MANIFEST}", self.base))?;
        // Signatur aus demselben Release, auch wenn gerade ein neues erscheint
        let version = Manifest::parse(&text)?.version;
        let signature = self.small(&format!("{}/download/v{version}/{SIGNATURE}", self.base))?;
        verify(text.as_bytes(), &signature, &self.key)
    }

    pub fn setup_url(&self, manifest: &Manifest) -> String {
        format!(
            "{}/download/v{}/{}",
            self.base, manifest.version, manifest.file
        )
    }

    /// Setup nach `dir` laden (oder das schon geladene nehmen) und prüfen.
    /// `progress` bekommt die bisher geladenen Bytes.
    pub fn download(
        &self,
        manifest: &Manifest,
        dir: &Path,
        progress: &mut dyn FnMut(u64),
    ) -> Result<PathBuf, Error> {
        let target = dir.join(&manifest.file);
        if check_file(&target, manifest).is_ok() {
            return Ok(target);
        }
        std::fs::create_dir_all(dir).map_err(io_error)?;
        let part = dir.join(format!("{}.part", manifest.file));
        if let Err(e) = self.fetch(&self.setup_url(manifest), &part, manifest, progress) {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
        let _ = std::fs::remove_file(&target);
        std::fs::rename(&part, &target).map_err(io_error)?;
        Ok(target)
    }

    fn fetch(
        &self,
        url: &str,
        part: &Path,
        manifest: &Manifest,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), Error> {
        let response = self.agent.get(url).call().map_err(net_error)?;
        // Ein Byte mehr als erwartet reicht, um zu große Dateien zu erkennen
        let mut reader = response.into_body().into_reader().take(manifest.size + 1);
        let mut file = File::create(part).map_err(io_error)?;
        let mut context = Context::new(&SHA256);
        let mut buffer = vec![0u8; 64 * 1024];
        let mut done = 0u64;
        loop {
            let n = reader
                .read(&mut buffer)
                .map_err(|e| Error::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            context.update(&buffer[..n]);
            file.write_all(&buffer[..n]).map_err(io_error)?;
            done += n as u64;
            progress(done);
        }
        file.sync_all().map_err(io_error)?;
        if done != manifest.size {
            return Err(Error::Damaged(format!(
                "{done} instead of {} bytes",
                manifest.size
            )));
        }
        if digest(context) != manifest.sha256 {
            return Err(Error::Damaged("SHA-256".to_string()));
        }
        Ok(())
    }

    fn small(&self, url: &str) -> Result<String, Error> {
        self.agent
            .get(url)
            .call()
            .map_err(net_error)?
            .into_body()
            .into_with_config()
            .limit(SMALL_LIMIT)
            .read_to_string()
            .map_err(net_error)
    }
}

fn digest(context: Context) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(context.finish().as_ref());
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests;
