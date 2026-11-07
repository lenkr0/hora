//! Liest `translations/<sprache>/LC_MESSAGES/hora-app.po` und erzeugt daraus
//! sortierte Tabellen für die Rust-Seite. Die Slint-Oberfläche bündelt
//! dieselben Dateien selbst (siehe crates/app/build.rs), so gibt es für beide
//! Seiten genau eine Übersetzungsdatei je Sprache.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Entry {
    ctx: String,
    id: String,
    plural: Option<String>,
    strs: Vec<(usize, String)>,
    fuzzy: bool,
}

fn unquote(line: &str) -> String {
    let s = line.trim();
    let s = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s);
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

fn parse(text: &str) -> Vec<Entry> {
    // Welches Feld die Fortsetzungszeilen ("...") verlängern
    enum Field {
        None,
        Ctx,
        Id,
        Plural,
        Str(usize),
    }
    let mut entries = Vec::new();
    let mut cur: Option<Entry> = None;
    let mut field = Field::None;
    let mut fuzzy_next = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(flags) = line.strip_prefix("#,") {
            fuzzy_next |= flags.split(',').any(|f| f.trim() == "fuzzy");
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let begins = line.starts_with("msgctxt ")
            || (line.starts_with("msgid ") && !matches!(field, Field::Ctx));
        if begins {
            entries.extend(cur.take());
            cur = Some(Entry {
                fuzzy: std::mem::take(&mut fuzzy_next),
                ..Entry::default()
            });
        }
        let Some(e) = cur.as_mut() else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("msgctxt ") {
            e.ctx = unquote(rest);
            field = Field::Ctx;
        } else if let Some(rest) = line.strip_prefix("msgid_plural ") {
            e.plural = Some(unquote(rest));
            field = Field::Plural;
        } else if let Some(rest) = line.strip_prefix("msgid ") {
            e.id = unquote(rest);
            field = Field::Id;
        } else if let Some(rest) = line.strip_prefix("msgstr[") {
            let (index, value) = rest.split_once(']').expect("msgstr[n]");
            let index: usize = index.parse().expect("Pluralindex");
            e.strs.push((index, unquote(value)));
            field = Field::Str(e.strs.len() - 1);
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            e.strs.push((0, unquote(rest)));
            field = Field::Str(e.strs.len() - 1);
        } else if line.starts_with('"') {
            let more = unquote(line);
            match field {
                Field::Ctx => e.ctx.push_str(&more),
                Field::Id => e.id.push_str(&more),
                Field::Plural => {
                    if let Some(p) = e.plural.as_mut() {
                        p.push_str(&more)
                    }
                }
                Field::Str(i) => e.strs[i].1.push_str(&more),
                Field::None => {}
            }
        }
    }
    entries.extend(cur);
    entries
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("..").join("..").join("translations");
    println!("cargo:rerun-if-changed={}", root.display());

    let mut languages: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(dir) = std::fs::read_dir(&root) {
        for item in dir.flatten() {
            let po = item.path().join("LC_MESSAGES").join("hora-app.po");
            if po.is_file() {
                languages.push((item.file_name().to_string_lossy().into_owned(), po));
            }
        }
    }
    languages.sort();

    let mut out = String::from(
        "pub(crate) struct Catalog {\n    pub messages: &'static [(&'static str, &'static str, &'static str)],\n    pub plurals: &'static [(&'static str, &'static str, &'static [&'static str])],\n}\n\n",
    );
    let mut arms = String::new();
    for (code, path) in &languages {
        println!("cargo:rerun-if-changed={}", path.display());
        let text =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut messages = Vec::new();
        let mut plurals = Vec::new();
        for e in parse(&text) {
            if e.id.is_empty() || e.fuzzy {
                continue;
            }
            if e.plural.is_some() {
                let mut forms: Vec<(usize, String)> = e.strs;
                forms.sort();
                if forms.iter().all(|(_, s)| !s.is_empty()) && !forms.is_empty() {
                    plurals.push((
                        e.ctx,
                        e.id,
                        forms.into_iter().map(|(_, s)| s).collect::<Vec<_>>(),
                    ));
                }
            } else if let Some((_, s)) = e.strs.into_iter().next()
                && !s.is_empty()
            {
                messages.push((e.ctx, e.id, s));
            }
        }
        messages.sort();
        messages.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        plurals.sort();
        plurals.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        let name = code.to_uppercase().replace('-', "_");
        writeln!(out, "static {name}: Catalog = Catalog {{\n    messages: &[").unwrap();
        for (ctx, id, s) in &messages {
            writeln!(out, "        ({ctx:?}, {id:?}, {s:?}),").unwrap();
        }
        writeln!(out, "    ],\n    plurals: &[").unwrap();
        for (ctx, id, forms) in &plurals {
            writeln!(out, "        ({ctx:?}, {id:?}, &{forms:?}),").unwrap();
        }
        writeln!(out, "    ],\n}};\n").unwrap();
        writeln!(arms, "        {code:?} => Some(&{name}),").unwrap();
    }
    writeln!(
        out,
        "pub(crate) fn get(code: &str) -> Option<&'static Catalog> {{\n    match code {{\n{arms}        _ => None,\n    }}\n}}"
    )
    .unwrap();
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("catalog.rs");
    std::fs::write(dest, out).unwrap();
}
