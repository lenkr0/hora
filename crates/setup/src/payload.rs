//! Die Programmdateien im Setup: als .xz (LZMA2 mit x86-Filter, wie NSIS und
//! 7-Zip für Programme) hinter dem eigentlichen Programm angehängt. Windows lädt nur den PE-Teil; der
//! Anhang bleibt beim Start unberührt.
//!
//! Aufbau: `<Setup-Programm> <komprimiertes Archiv> <Schluss, 32 Bytes>`.
//! Der Schluss enthält `HORASETP`, die Länge des Programmteils, die Länge
//! des Archivs und eine FNV-1a-Prüfsumme darüber. Das Archiv ist eine Folge
//! von Einträgen `u16 Pfadlänge, Pfad (UTF-8, mit /), u64 Länge, Daten`.
//!
//! Ohne Anhang ist dieselbe Datei der Deinstaller (`uninstall.exe`).

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use hora_i18n::tr;
use xz2::stream::{Check, Filters, LzmaOptions, Stream};

const MAGIC: &[u8; 8] = b"HORASETP";
const TRAILER: u64 = 32;
/// xz-Stufe 9 "extrem" mit 64-MB-Wörterbuch; `fast` für Tests
const PRESET_EXTREME: u32 = 1 << 31;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// Relativer Pfad mit `/`
    pub path: String,
    pub data: Vec<u8>,
}

/// Inhalt einer Setup-Datei
#[derive(Debug)]
pub struct Payload {
    pub files: Vec<File>,
    /// Länge des Programmteils (für die Kopie als Deinstaller)
    pub stub_len: u64,
}

impl Payload {
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.data.len() as u64).sum()
    }
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn archive(files: &[File]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for f in files {
        let path = f.path.as_bytes();
        let len = u16::try_from(path.len()).map_err(|_| format!("Pfad zu lang: {}", f.path))?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(path);
        out.extend_from_slice(&(f.data.len() as u64).to_le_bytes());
        out.extend_from_slice(&f.data);
    }
    Ok(out)
}

fn unarchive(mut data: &[u8]) -> Result<Vec<File>, String> {
    let mut files = Vec::new();
    let take = |data: &mut &[u8], n: usize| -> Result<Vec<u8>, String> {
        if data.len() < n {
            return Err("Archiv ist abgeschnitten".into());
        }
        let (head, rest) = data.split_at(n);
        *data = rest;
        Ok(head.to_vec())
    };
    while !data.is_empty() {
        let len = u16::from_le_bytes(take(&mut data, 2)?.try_into().unwrap_or_default());
        let path = String::from_utf8(take(&mut data, usize::from(len))?)
            .map_err(|_| "Pfad im Archiv ist kein UTF-8".to_string())?;
        if path.is_empty() || path.contains("..") || path.starts_with('/') || path.contains(':') {
            return Err(format!("Ungültiger Pfad im Archiv: {path}"));
        }
        let size = u64::from_le_bytes(take(&mut data, 8)?.try_into().unwrap_or_default());
        let size = usize::try_from(size).map_err(|_| "Datei zu groß".to_string())?;
        files.push(File {
            path,
            data: take(&mut data, size)?,
        });
    }
    Ok(files)
}

/// Alle Dateien unter `dir` (relativ, sortiert)
pub fn collect(dir: &Path) -> Result<Vec<File>, String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<File>) -> Result<(), String> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(base, &path, out)?;
            } else {
                let rel = path
                    .strip_prefix(base)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                out.push(File { path: rel, data });
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    Ok(files)
}

/// .xz mit x86-Filter (Sprungziele relativ → absolut, dadurch packt LZMA
/// Programmcode deutlich besser) und LZMA2
fn compress(raw: &[u8], fast: bool) -> Result<Vec<u8>, String> {
    let preset = if fast { 1 } else { 9 | PRESET_EXTREME };
    let mut options = LzmaOptions::new_preset(preset).map_err(|e| e.to_string())?;
    if !fast {
        options.dict_size(64 << 20);
    }
    let mut filters = Filters::new();
    filters.x86();
    filters.lzma2(&options);
    let stream = Stream::new_stream_encoder(&filters, Check::Crc64).map_err(|e| e.to_string())?;
    let mut encoder = xz2::write::XzEncoder::new_stream(Vec::new(), stream);
    encoder.write_all(raw).map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}

fn decompress(packed: &[u8]) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    xz2::read::XzDecoder::new(packed)
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    Ok(raw)
}

/// Setup-Datei bauen: Programmteil `stub` plus komprimierte `files`
pub fn pack(stub: &[u8], files: &[File], fast: bool) -> Result<Vec<u8>, String> {
    let raw = archive(files)?;
    let compressed = compress(&raw, fast)?;
    let mut out = Vec::with_capacity(stub.len() + compressed.len() + TRAILER as usize);
    out.extend_from_slice(stub);
    out.extend_from_slice(&compressed);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(stub.len() as u64).to_le_bytes());
    out.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
    out.extend_from_slice(&fnv1a(&compressed).to_le_bytes());
    Ok(out)
}

/// `hora-setup --pack <ordner> <ziel.exe>`: dieses Programm plus die Dateien
/// im Ordner als neue Setup-Datei
pub fn pack_command(dir: &Path, target: &Path) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (stub_len, _) =
        trailer(&exe)?.unwrap_or((std::fs::metadata(&exe).map_err(|e| e.to_string())?.len(), 0));
    let mut stub = std::fs::read(&exe).map_err(|e| e.to_string())?;
    stub.truncate(stub_len as usize);
    let files = collect(dir)?;
    let raw: u64 = files.iter().map(|f| f.data.len() as u64).sum();
    let packed = pack(&stub, &files, false)?;
    std::fs::write(target, &packed).map_err(|e| format!("{}: {e}", target.display()))?;
    Ok(format!(
        "{} Dateien, {:.1} MB → {:.1} MB (Setup-Programm {:.1} MB)",
        files.len(),
        raw as f64 / 1e6,
        packed.len() as f64 / 1e6,
        stub_len as f64 / 1e6
    ))
}

/// (Länge des Programmteils, Länge des Archivs), falls ein Anhang da ist
fn trailer(exe: &Path) -> Result<Option<(u64, u64)>, String> {
    let mut file = std::fs::File::open(exe).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    if len < TRAILER {
        return Ok(None);
    }
    let mut tail = [0u8; TRAILER as usize];
    file.seek(SeekFrom::Start(len - TRAILER))
        .and_then(|_| file.read_exact(&mut tail))
        .map_err(|e| e.to_string())?;
    if &tail[..8] != MAGIC {
        return Ok(None);
    }
    let num = |i: usize| u64::from_le_bytes(tail[i..i + 8].try_into().unwrap_or_default());
    let (stub_len, packed_len) = (num(8), num(16));
    if stub_len
        .checked_add(packed_len)
        .and_then(|n| n.checked_add(TRAILER))
        != Some(len)
    {
        return Err(tr!("The setup file is damaged. Please download it again.").into());
    }
    Ok(Some((stub_len, packed_len)))
}

/// Anhang dieser Datei lesen; `None` ohne Anhang (Deinstaller)
pub fn read(exe: &Path) -> Result<Option<Payload>, String> {
    let Some((stub_len, packed_len)) = trailer(exe)? else {
        return Ok(None);
    };
    let mut file = std::fs::File::open(exe).map_err(|e| e.to_string())?;
    let mut packed = vec![0u8; packed_len as usize];
    file.seek(SeekFrom::Start(stub_len))
        .and_then(|_| file.read_exact(&mut packed))
        .map_err(|e| e.to_string())?;
    let mut sum = [0u8; 8];
    file.seek(SeekFrom::End(-8))
        .and_then(|_| file.read_exact(&mut sum))
        .map_err(|e| e.to_string())?;
    if fnv1a(&packed) != u64::from_le_bytes(sum) {
        return Err(tr!("The setup file is damaged. Please download it again.").into());
    }
    let raw = decompress(&packed)?;
    Ok(Some(Payload {
        files: unarchive(&raw)?,
        stub_len,
    }))
}

/// Programmteil der laufenden Datei (wird zu `uninstall.exe`)
pub fn stub_bytes(exe: &Path, stub_len: u64) -> Result<Vec<u8>, String> {
    let mut data = Vec::with_capacity(stub_len as usize);
    std::fs::File::open(exe)
        .map_err(|e| e.to_string())?
        .take(stub_len)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_through_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![
            File {
                path: "hora.exe".into(),
                data: (0..200_000u32)
                    .flat_map(|n| (n % 251).to_le_bytes())
                    .collect(),
            },
            File {
                path: "THIRD_PARTY_LICENSES/Rust-Crates.txt".into(),
                data: b"MIT\n".repeat(1000),
            },
        ];
        let stub = b"MZ-stub-bytes".to_vec();
        let packed = pack(&stub, &files, true).unwrap();
        let exe = dir.path().join("setup.exe");
        std::fs::write(&exe, &packed).unwrap();
        let payload = read(&exe).unwrap().unwrap();
        assert_eq!(payload.files, files);
        assert_eq!(payload.stub_len, stub.len() as u64);
        assert_eq!(stub_bytes(&exe, payload.stub_len).unwrap(), stub);

        // Ohne Anhang: kein Payload (Deinstaller)
        std::fs::write(&exe, &stub).unwrap();
        assert!(read(&exe).unwrap().is_none());

        // Ein umgekipptes Byte fällt auf
        let mut broken = packed.clone();
        broken[stub.len() + 10] ^= 0xff;
        std::fs::write(&exe, &broken).unwrap();
        assert!(read(&exe).unwrap_err().contains("damaged"));
    }

    #[test]
    fn archive_rejects_paths_outside_the_folder() {
        let bad = archive(&[File {
            path: "../evil.exe".into(),
            data: vec![1],
        }])
        .unwrap();
        assert!(unarchive(&bad).is_err());
    }

    #[test]
    fn collect_walks_subfolders() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::write(dir.path().join("sub").join("b.txt"), "b").unwrap();
        let paths: Vec<String> = collect(dir.path())
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect();
        assert_eq!(paths, ["a.txt", "sub/b.txt"]);
    }
}
