use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use super::*;

enum Reply {
    Ok(Vec<u8>),
    Redirect(String),
}

/// Kleiner HTTP-Server wie GitHub Releases; merkt sich die abgerufenen Pfade
struct Server {
    base: String,
    hits: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn hits(&self, path: &str) -> usize {
        self.hits
            .lock()
            .unwrap()
            .iter()
            .filter(|p| *p == path)
            .count()
    }
}

fn serve(routes: Vec<(String, Reply)>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let routes: HashMap<String, Reply> = routes.into_iter().collect();
    let hits = Arc::new(Mutex::new(Vec::new()));
    let seen = hits.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            if reader.read_line(&mut request).is_err() {
                continue;
            }
            // Kopfzeilen bis zur Leerzeile überspringen
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) <= 2 {
                    break;
                }
            }
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
            seen.lock().unwrap().push(path.clone());
            let head = |status: &str, extra: String, len: usize| {
                format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {len}\r\nConnection: close\r\n{extra}\r\n"
                )
            };
            let _ = match routes.get(&path) {
                Some(Reply::Ok(body)) => stream
                    .write_all(head("200 OK", String::new(), body.len()).as_bytes())
                    .and_then(|()| stream.write_all(body)),
                Some(Reply::Redirect(to)) => {
                    stream.write_all(head("302 Found", format!("Location: {to}\r\n"), 0).as_bytes())
                }
                None => stream.write_all(head("404 Not Found", String::new(), 0).as_bytes()),
            };
        }
    });
    Server { base, hits }
}

fn key_pair() -> (Vec<u8>, [u8; 32]) {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let public = public_key(pkcs8.as_ref()).unwrap();
    (pkcs8.as_ref().to_vec(), public)
}

fn sha(data: &[u8]) -> [u8; 32] {
    let mut context = Context::new(&SHA256);
    context.update(data);
    digest(context)
}

struct Release {
    manifest: Manifest,
    text: String,
    signature: String,
    setup: Vec<u8>,
}

fn release(version: Version, pkcs8: &[u8], setup: &[u8]) -> Release {
    let manifest = Manifest {
        version,
        file: setup_name(version),
        size: setup.len() as u64,
        sha256: sha(setup),
    };
    let text = manifest.render();
    let signature = sign(text.as_bytes(), pkcs8).unwrap();
    Release {
        manifest,
        text,
        signature,
        setup: setup.to_vec(),
    }
}

/// Wie bei GitHub: `latest/download/…` leitet zum neuesten Release weiter
fn routes(r: &Release, setup: Vec<u8>) -> Vec<(String, Reply)> {
    let v = r.manifest.version;
    vec![
        (
            format!("/latest/download/{MANIFEST}"),
            Reply::Redirect(format!("/download/v{v}/{MANIFEST}")),
        ),
        (
            format!("/download/v{v}/{MANIFEST}"),
            Reply::Ok(r.text.clone().into_bytes()),
        ),
        (
            format!("/download/v{v}/{SIGNATURE}"),
            Reply::Ok(format!("{}\n", r.signature).into_bytes()),
        ),
        (
            format!("/download/v{v}/{}", r.manifest.file),
            Reply::Ok(setup),
        ),
    ]
}

fn setup_bytes() -> Vec<u8> {
    (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect()
}

#[test]
fn versions_parse_and_compare() {
    assert_eq!(Version::parse("1.2.3"), Some(Version(1, 2, 3)));
    assert_eq!(Version::parse("v10.0.1"), Some(Version(10, 0, 1)));
    for bad in [
        "",
        "1.2",
        "1.2.3.4",
        "1.2.x",
        "+1.2.3",
        "1..3",
        "1.2.3-beta",
        " 1.2.3",
    ] {
        assert_eq!(Version::parse(bad), None, "{bad:?}");
    }
    assert!(Version(1, 10, 0) > Version(1, 9, 9));
    assert!(Version(2, 0, 0) > Version(1, 99, 99));
    assert_eq!(Version(1, 2, 0).to_string(), "1.2.0");
    assert_eq!(setup_name(Version(1, 2, 0)), "Hora-Setup-1.2.0.exe");
}

#[test]
fn manifest_round_trip() {
    let (pkcs8, _) = key_pair();
    let r = release(Version(1, 2, 0), &pkcs8, b"setup");
    assert_eq!(Manifest::parse(&r.text).unwrap(), r.manifest);
    // Windows-Zeilenenden und spätere, unbekannte Felder stören nicht
    let crlf = r.text.replace('\n', "\r\n") + "notes Neue Funktionen\r\n";
    assert_eq!(Manifest::parse(&crlf).unwrap(), r.manifest);
}

#[test]
fn manifest_rejects_mistakes() {
    let (pkcs8, _) = key_pair();
    let good = release(Version(1, 2, 0), &pkcs8, b"setup").text;
    let cases = [
        good.replace("hora-update 1", "hora-update 2"),
        good.replace("Hora-Setup-1.2.0.exe", "Hora-Setup-1.3.0.exe"),
        good.replace("Hora-Setup-1.2.0.exe", "..\\evil.exe"),
        good.replace("size 5", "size 0"),
        good.replace("size 5", "size 999999999999"),
        good.replace("version 1.2.0\n", ""),
        good.clone() + "version 1.3.0\n",
        good[..good.len() - 2].to_string() + "zz\n",
    ];
    for text in cases {
        assert!(
            matches!(Manifest::parse(&text), Err(Error::Manifest(_))),
            "{text}"
        );
    }
}

#[test]
fn signature_must_match_key_and_content() {
    let (pkcs8, public) = key_pair();
    let (_, other) = key_pair();
    let r = release(Version(1, 2, 0), &pkcs8, b"setup");
    assert_eq!(
        verify(r.text.as_bytes(), &r.signature, &public).unwrap(),
        r.manifest
    );
    let tampered = r.text.replace("size 5", "size 6");
    assert_eq!(
        verify(tampered.as_bytes(), &r.signature, &public),
        Err(Error::Signature)
    );
    assert_eq!(
        verify(r.text.as_bytes(), &r.signature, &other),
        Err(Error::Signature)
    );
    assert_eq!(
        verify(r.text.as_bytes(), "kein base64!", &public),
        Err(Error::Signature)
    );
}

#[test]
fn latest_follows_redirect_and_checks_signature() {
    let (pkcs8, public) = key_pair();
    let r = release(Version(1, 2, 0), &pkcs8, &setup_bytes());
    let server = serve(routes(&r, r.setup.clone()));
    let manifest = Source::new(&server.base, public).latest().unwrap();
    assert_eq!(manifest, r.manifest);
}

#[test]
fn foreign_signature_is_rejected() {
    let (pkcs8, _) = key_pair();
    let (_, public) = key_pair();
    let r = release(Version(1, 2, 0), &pkcs8, &setup_bytes());
    let server = serve(routes(&r, r.setup.clone()));
    assert_eq!(
        Source::new(&server.base, public).latest(),
        Err(Error::Signature)
    );
}

#[test]
fn no_release_is_404() {
    let server = serve(Vec::new());
    let (_, public) = key_pair();
    assert_eq!(
        Source::new(&server.base, public).latest(),
        Err(Error::Status(404))
    );
}

#[test]
fn download_checks_and_reuses_the_file() {
    let (pkcs8, public) = key_pair();
    let r = release(Version(1, 2, 0), &pkcs8, &setup_bytes());
    let server = serve(routes(&r, r.setup.clone()));
    let source = Source::new(&server.base, public);
    let dir = tempfile::tempdir().unwrap();
    let mut last = 0;
    let path = source
        .download(&r.manifest, dir.path(), &mut |done| last = done)
        .unwrap();
    assert_eq!(path, dir.path().join("Hora-Setup-1.2.0.exe"));
    assert_eq!(std::fs::read(&path).unwrap(), r.setup);
    assert_eq!(last, r.setup.len() as u64);
    check_file(&path, &r.manifest).unwrap();
    // Schon geladen: kein zweiter Download
    source
        .download(&r.manifest, dir.path(), &mut |_| {})
        .unwrap();
    assert_eq!(server.hits("/download/v1.2.0/Hora-Setup-1.2.0.exe"), 1);
}

#[test]
fn damaged_or_oversized_downloads_leave_nothing_behind() {
    let (pkcs8, public) = key_pair();
    let r = release(Version(1, 2, 0), &pkcs8, &setup_bytes());
    let mut flipped = r.setup.clone();
    flipped[1000] ^= 1;
    let mut longer = r.setup.clone();
    longer.extend_from_slice(b"extra");
    for served in [flipped, longer, r.setup[..1000].to_vec()] {
        let server = serve(routes(&r, served));
        let dir = tempfile::tempdir().unwrap();
        let result =
            Source::new(&server.base, public).download(&r.manifest, dir.path(), &mut |_| {});
        assert!(matches!(result, Err(Error::Damaged(_))), "{result:?}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

#[test]
fn a_changed_file_on_disk_fails_the_check() {
    let (pkcs8, _) = key_pair();
    let r = release(Version(1, 2, 0), &pkcs8, b"setup");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(&r.manifest.file);
    std::fs::write(&path, b"setuq").unwrap();
    assert!(matches!(
        check_file(&path, &r.manifest),
        Err(Error::Damaged(_))
    ));
}

#[test]
fn manifest_for_file_takes_the_version_from_the_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Hora-Setup-1.2.0.exe");
    std::fs::write(&path, b"setup").unwrap();
    let manifest = Manifest::for_file(&path).unwrap();
    assert_eq!(manifest.version, Version(1, 2, 0));
    assert_eq!(manifest.size, 5);
    assert_eq!(manifest.sha256, sha(b"setup"));
    let other = dir.path().join("setup.exe");
    std::fs::write(&other, b"setup").unwrap();
    assert!(Manifest::for_file(&other).is_err());
}

#[test]
fn clean_removes_old_and_partial_setups() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "Hora-Setup-1.0.0.exe",
        "Hora-Setup-1.1.0.exe",
        "Hora-Setup-1.2.0.exe",
        "Hora-Setup-1.3.0.exe.part",
        "notes.txt",
    ] {
        std::fs::write(dir.path().join(name), b"x").unwrap();
    }
    clean(dir.path(), Version(1, 1, 0));
    let mut left: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, ["Hora-Setup-1.2.0.exe", "notes.txt"]);
}
