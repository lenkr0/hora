//! Signierwerkzeug für Releases. Läuft im Release-Workflow und einmal von
//! Hand zum Erzeugen des Schlüssels; gehört nicht zum Installer.
//!
//! - `hora-sign keygen <Datei>`: neues Schlüsselpaar. Der geheime Schlüssel
//!   (Base64, PKCS#8) landet in der Datei und gehört als Secret
//!   `HORA_UPDATE_KEY` zu GitHub; der öffentliche wird ausgegeben (für
//!   `PUBLIC_KEY` in crates/update/src/lib.rs).
//! - `hora-sign manifest <Hora-Setup-x.y.z.exe> <Ordner>`: schreibt
//!   update.txt und update.sig in den Ordner, Schlüssel aus
//!   `HORA_UPDATE_KEY`. Bricht ab, wenn er nicht zu `PUBLIC_KEY` passt –
//!   Hora würde das Update sonst ablehnen.

use std::path::Path;
use std::process::ExitCode;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use hora_update::{MANIFEST, Manifest, PUBLIC_KEY, SIGNATURE};
use ring::rand::SystemRandom;
use ring::signature::Ed25519KeyPair;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["keygen", file] => keygen(Path::new(file)),
        ["manifest", setup, dir] => manifest(Path::new(setup), Path::new(dir)),
        _ => Err(
            "Aufruf: hora-sign keygen <Datei> | hora-sign manifest <Hora-Setup-x.y.z.exe> <Ordner>"
                .to_string(),
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn keygen(file: &Path) -> Result<(), String> {
    if file.exists() {
        return Err(format!(
            "{} gibt es schon. Ein neuer Schlüssel sperrt bestehende Installationen aus.",
            file.display()
        ));
    }
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
        .map_err(|_| "Keine Zufallszahlen vom System".to_string())?;
    let key = hora_update::public_key(pkcs8.as_ref())?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(file, BASE64.encode(pkcs8.as_ref()) + "\n").map_err(|e| e.to_string())?;
    let bytes: Vec<String> = key.iter().map(|b| format!("0x{b:02x}")).collect();
    println!("pub const PUBLIC_KEY: [u8; 32] = [{}];", bytes.join(", "));
    Ok(())
}

fn manifest(setup: &Path, dir: &Path) -> Result<(), String> {
    let secret = std::env::var("HORA_UPDATE_KEY")
        .map_err(|_| "HORA_UPDATE_KEY fehlt (geheimer Schlüssel, Base64)".to_string())?;
    let pkcs8 = BASE64
        .decode(secret.trim())
        .map_err(|_| "HORA_UPDATE_KEY ist kein Base64".to_string())?;
    if hora_update::public_key(&pkcs8)? != PUBLIC_KEY {
        return Err(
            "HORA_UPDATE_KEY passt nicht zu PUBLIC_KEY; Hora würde dieses Update ablehnen"
                .to_string(),
        );
    }
    let manifest = Manifest::for_file(setup).map_err(|e| e.to_string())?;
    let text = manifest.render();
    let signature = hora_update::sign(text.as_bytes(), &pkcs8)?;
    // Gegenprobe wie in der App
    hora_update::verify(text.as_bytes(), &signature, &PUBLIC_KEY).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(MANIFEST), &text).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(SIGNATURE), signature + "\n").map_err(|e| e.to_string())?;
    println!(
        "{MANIFEST} und {SIGNATURE}: Hora {} ({} Bytes)",
        manifest.version, manifest.size
    );
    Ok(())
}
