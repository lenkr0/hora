//! Übersetzungen für Hora.
//!
//! Quelltexte sind englisch. Je Sprache gibt es eine gettext-Datei
//! `translations/<code>/LC_MESSAGES/hora-app.po`; die Slint-Oberfläche bündelt
//! sie über `@tr(...)`, die Rust-Seite über die Makros [`tr!`], [`trn!`] und
//! [`trc!`] aus derselben Datei. Platzhalter wie in Slint: `{}` der Reihe nach,
//! `{0}`/`{1}` nach Position, `{n}` für die Anzahl bei Mehrzahlformen.

use std::cell::Cell;
use std::fmt::{Display, Write as _};
use std::sync::atomic::{AtomicU8, Ordering};

pub mod dates;

mod catalog {
    include!(concat!(env!("OUT_DIR"), "/catalog.rs"));
}

/// Unterstützte Sprachen; Englisch ist die Quellsprache und der Standard
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    En,
    De,
    Fr,
    Es,
    It,
    Nl,
    Pl,
}

impl Lang {
    pub const ALL: [Lang; 7] = [
        Lang::En,
        Lang::De,
        Lang::Fr,
        Lang::Es,
        Lang::It,
        Lang::Nl,
        Lang::Pl,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
            Lang::Fr => "fr",
            Lang::Es => "es",
            Lang::It => "it",
            Lang::Nl => "nl",
            Lang::Pl => "pl",
        }
    }

    /// Name der Sprache in ihr selbst, für die Auswahl
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::De => "Deutsch",
            Lang::Fr => "Français",
            Lang::Es => "Español",
            Lang::It => "Italiano",
            Lang::Nl => "Nederlands",
            Lang::Pl => "Polski",
        }
    }

    /// "de", "de-AT", "de_DE.UTF-8", "DE" … → Deutsch
    pub fn from_code(code: &str) -> Option<Lang> {
        let base = code
            .split(['-', '_', '.', '@'])
            .next()?
            .trim()
            .to_ascii_lowercase();
        Lang::ALL.into_iter().find(|l| l.code() == base)
    }

    /// Index der Mehrzahlform (wie `Plural-Forms` in der .po-Datei)
    pub fn plural_index(self, n: i64) -> usize {
        let n = n.unsigned_abs();
        match self {
            Lang::Fr => usize::from(n > 1),
            Lang::Pl => {
                if n == 1 {
                    0
                } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
                    1
                } else {
                    2
                }
            }
            _ => usize::from(n != 1),
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

thread_local! {
    /// Für Tests: Sprache nur in diesem Thread
    static OVERRIDE: Cell<Option<Lang>> = const { Cell::new(None) };
}

/// Sprache der App festlegen (gilt für alle Threads)
pub fn set_language(lang: Lang) {
    let index = Lang::ALL.iter().position(|l| *l == lang).unwrap_or(0);
    CURRENT.store(index as u8, Ordering::Relaxed);
}

pub fn language() -> Lang {
    OVERRIDE
        .with(Cell::get)
        .unwrap_or_else(|| Lang::ALL[usize::from(CURRENT.load(Ordering::Relaxed)).min(6)])
}

/// `f` mit einer anderen Sprache ausführen, nur in diesem Thread (Tests)
pub fn with_language<R>(lang: Lang, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Lang>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OVERRIDE.with(|o| o.set(self.0));
        }
    }
    let _restore = Restore(OVERRIDE.with(|o| o.replace(Some(lang))));
    f()
}

fn find(ctx: &str, msgid: &'static str) -> &'static str {
    let Some(cat) = catalog::get(language().code()) else {
        return msgid;
    };
    match cat
        .messages
        .binary_search_by(|(c, id, _)| (*c, *id).cmp(&(ctx, msgid)))
    {
        Ok(i) => cat.messages[i].2,
        Err(_) => msgid,
    }
}

/// Übersetzung von `msgid`, sonst `msgid` selbst
pub fn tr(msgid: &'static str) -> &'static str {
    find("", msgid)
}

/// Übersetzung mit Kontext (`msgctxt`), für gleiche englische Wörter mit
/// verschiedener Bedeutung
pub fn trc(ctx: &'static str, msgid: &'static str) -> &'static str {
    find(ctx, msgid)
}

/// Passende Mehrzahlform für `n`, noch mit Platzhaltern
pub fn trn(one: &'static str, many: &'static str, n: i64) -> &'static str {
    let lang = language();
    if let Some(cat) = catalog::get(lang.code())
        && let Ok(i) = cat
            .plurals
            .binary_search_by(|(c, id, _)| (*c, *id).cmp(&("", one)))
        && let Some(form) = cat.plurals[i].2.get(lang.plural_index(n))
    {
        return form;
    }
    if n == 1 { one } else { many }
}

/// Platzhalter füllen wie Slints `@tr`: `{}` der Reihe nach, `{0}` nach
/// Position, `{n}` = Anzahl, `{{`/`}}` für geschweifte Klammern
pub fn fill(template: &str, n: Option<i64>, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0;
    let mut rest = template;
    while let Some(p) = rest.find(['{', '}']) {
        out.push_str(&rest[..p]);
        let tail = &rest[p..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        if let Some(after) = tail.strip_prefix('}') {
            out.push('}');
            rest = after;
            continue;
        }
        let Some(end) = tail.find('}') else {
            out.push_str(tail);
            return out;
        };
        let name = tail[1..end].trim();
        let written = if name.is_empty() {
            next += 1;
            args.get(next - 1).map(|a| write!(out, "{a}"))
        } else if let Ok(i) = name.parse::<usize>() {
            args.get(i).map(|a| write!(out, "{a}"))
        } else if name == "n" {
            n.map(|n| write!(out, "{n}"))
        } else {
            None
        };
        if written.is_none() {
            out.push_str(&tail[..=end]);
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// `tr!("Text")` → `&'static str`; `tr!("Hallo {}", name)` → `String`
#[macro_export]
macro_rules! tr {
    ($msg:literal) => {
        $crate::tr($msg)
    };
    ($msg:literal, $($arg:expr),+ $(,)?) => {
        $crate::fill($crate::tr($msg), None, &[$(&$arg as &dyn ::std::fmt::Display),+])
    };
}

/// `trn!("{n} event", "{n} events", count)`, weitere Argumente für `{}`
#[macro_export]
macro_rules! trn {
    ($one:literal, $many:literal, $n:expr $(, $arg:expr)* $(,)?) => {{
        let n = ($n) as i64;
        $crate::fill($crate::trn($one, $many, n), Some(n), &[$(&$arg as &dyn ::std::fmt::Display),*])
    }};
}

/// `trc!("Kontext", "Text")` → `&'static str`
#[macro_export]
macro_rules! trc {
    ($ctx:literal, $msg:literal) => {
        $crate::trc($ctx, $msg)
    };
}

/// Markiert einen Text zur Übersetzung, ohne ihn schon zu übersetzen (für
/// Tabellen); übersetzt wird später mit [`tr()`]
#[macro_export]
macro_rules! tr_noop {
    ($msg:literal) => {
        $msg
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plural_rules() {
        assert_eq!(Lang::En.plural_index(1), 0);
        assert_eq!(Lang::En.plural_index(0), 1);
        assert_eq!(Lang::Fr.plural_index(0), 0);
        assert_eq!(Lang::Fr.plural_index(2), 1);
        let pl: Vec<usize> = [1, 2, 4, 5, 12, 14, 21, 22, 25, 112, 122]
            .into_iter()
            .map(|n| Lang::Pl.plural_index(n))
            .collect();
        assert_eq!(pl, [0, 1, 1, 2, 2, 2, 2, 1, 2, 2, 1]);
    }

    #[test]
    fn placeholders() {
        assert_eq!(fill("{} of {}", None, &[&1, &"x"]), "1 of x");
        assert_eq!(fill("{1} before {0}", None, &[&"a", &"b"]), "b before a");
        assert_eq!(
            fill("{n} events in {}", Some(3), &[&"May"]),
            "3 events in May"
        );
        assert_eq!(fill("{{literal}} {}", None, &[&2]), "{literal} 2");
        assert_eq!(fill("missing {}", None, &[]), "missing {}");
    }

    #[test]
    fn codes() {
        assert_eq!(Lang::from_code("de-AT"), Some(Lang::De));
        assert_eq!(Lang::from_code("pl_PL.UTF-8"), Some(Lang::Pl));
        assert_eq!(Lang::from_code("EN"), Some(Lang::En));
        assert_eq!(Lang::from_code("sv-SE"), None);
    }

    #[test]
    fn english_is_the_fallback() {
        with_language(Lang::En, || {
            assert_eq!(
                tr("No such text in any catalog"),
                "No such text in any catalog"
            );
            assert_eq!(trn("{n} thing", "{n} things", 2), "{n} things");
        });
    }

    #[test]
    fn override_is_per_thread_and_restored() {
        with_language(Lang::Pl, || {
            assert_eq!(language(), Lang::Pl);
            with_language(Lang::Fr, || assert_eq!(language(), Lang::Fr));
            assert_eq!(language(), Lang::Pl);
        });
    }
}
