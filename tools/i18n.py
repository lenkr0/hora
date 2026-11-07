"""Übersetzungen pflegen.

    python tools/i18n.py extract   # translations/hora-app.pot aus dem Quelltext
    python tools/i18n.py update    # neue Texte in alle .po-Dateien übernehmen
    python tools/i18n.py check     # CI: alles übersetzt, nichts veraltet?
    python tools/i18n.py stats     # Stand je Sprache

Texte stehen englisch im Quelltext: in Slint als @tr("..."), in Rust als
tr!("..."), trn!("one", "many", n), trc!("Kontext", "...") oder tr_noop!("...").
Je Sprache gibt es translations/<code>/LC_MESSAGES/hora-app.po; dieselbe Datei
nutzen die Slint-Oberfläche und der Rust-Code (crates/i18n).
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TRANSLATIONS = os.path.join(ROOT, "translations")
POT = os.path.join(TRANSLATIONS, "hora-app.pot")
SOURCES = [os.path.join(ROOT, "crates")]

PLURAL_FORMS = {
    "de": "nplurals=2; plural=(n != 1);",
    "fr": "nplurals=2; plural=(n > 1);",
    "es": "nplurals=2; plural=(n != 1);",
    "it": "nplurals=2; plural=(n != 1);",
    "nl": "nplurals=2; plural=(n != 1);",
    "pl": "nplurals=3; plural=(n == 1 ? 0 : n % 10 >= 2 && n % 10 <= 4 && (n % 100 < 10 || n % 100 >= 20) ? 1 : 2);",
}
LANGUAGE_NAMES = {
    "de": "German",
    "fr": "French",
    "es": "Spanish",
    "it": "Italian",
    "nl": "Dutch",
    "pl": "Polish",
}

STR = r'"((?:[^"\\]|\\.)*)"'
SLINT_TR = re.compile(r"@tr\(\s*(?:" + STR + r"\s*=>\s*)?" + STR + r"(?:\s*\|\s*" + STR + r"\s*%)?")
RUST_TR = re.compile(r"\btr!\(\s*" + STR)
RUST_TRN = re.compile(r"\btrn!\(\s*" + STR + r"\s*,\s*" + STR)
RUST_TRC = re.compile(r"\btrc!\(\s*" + STR + r"\s*,\s*" + STR)
RUST_NOOP = re.compile(r"\btr_noop!\(\s*" + STR)


def unescape(s):
    out = []
    i = 0
    while i < len(s):
        c = s[i]
        if c == "\\" and i + 1 < len(s):
            n = s[i + 1]
            if n == "n":
                out.append("\n")
            elif n == "t":
                out.append("\t")
            elif n == "u" and s[i + 2:i + 3] == "{":
                end = s.index("}", i)
                out.append(chr(int(s[i + 3:end], 16)))
                i = end + 1
                continue
            else:
                out.append(n)
            i += 2
            continue
        out.append(c)
        i += 1
    return "".join(out)


def po_quote(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t") + '"'


class Msg:
    def __init__(self, ctx, id, plural=None):
        self.ctx = ctx
        self.id = id
        self.plural = plural
        self.strs = []  # Übersetzungen (eine, bei Mehrzahl mehrere)
        self.refs = []
        self.fuzzy = False
        self.comments = []

    @property
    def key(self):
        return (self.ctx, self.id)


def extract():
    """Alle Texte mit Fundstellen, in der Reihenfolge des ersten Auftretens"""
    found = {}

    def add(ctx, msgid, plural, path, line):
        key = (ctx, msgid)
        if key not in found:
            found[key] = Msg(ctx, msgid, plural)
        rel = os.path.relpath(path, ROOT).replace("\\", "/")
        found[key].refs.append(f"{rel}:{line}")

    files = []
    for base in SOURCES:
        for folder, dirs, names in os.walk(base):
            dirs[:] = [d for d in dirs if d not in ("target", "tests")]
            for name in sorted(names):
                if name.endswith((".slint", ".rs")):
                    files.append(os.path.join(folder, name))
    for path in sorted(files):
        with open(path, encoding="utf-8-sig") as fh:
            text = fh.read()
        if path.endswith(".rs"):
            # Tests prüfen Übersetzungen, liefern aber keine eigenen Texte
            cut = text.find("#[cfg(test)]")
            if cut >= 0:
                text = text[:cut]
        # Kommentare (auch Doku-Beispiele) nicht mitzählen; Zeilen bleiben erhalten
        text = re.sub(r"(?m)^\s*//.*$", "", text)
        line_at = lambda pos: text.count("\n", 0, pos) + 1
        if path.endswith(".slint"):
            for m in SLINT_TR.finditer(text):
                ctx = unescape(m.group(1)) if m.group(1) is not None else ""
                plural = unescape(m.group(3)) if m.group(3) is not None else None
                add(ctx, unescape(m.group(2)), plural, path, line_at(m.start()))
        else:
            for m in RUST_TR.finditer(text):
                add("", unescape(m.group(1)), None, path, line_at(m.start()))
            for m in RUST_NOOP.finditer(text):
                add("", unescape(m.group(1)), None, path, line_at(m.start()))
            for m in RUST_TRN.finditer(text):
                add("", unescape(m.group(1)), unescape(m.group(2)), path, line_at(m.start()))
            for m in RUST_TRC.finditer(text):
                add(unescape(m.group(1)), unescape(m.group(2)), None, path, line_at(m.start()))
    return list(found.values())


def read_po(path):
    """Einträge einer .po-Datei (ohne Kopf); veraltete (#~) werden übergangen"""
    msgs = []
    cur = None
    field = None
    fuzzy = False
    comments = []
    with open(path, encoding="utf-8") as fh:
        lines = fh.read().splitlines()
    for raw in lines:
        line = raw.strip()
        if line.startswith("#~"):
            continue
        if line.startswith("#,"):
            fuzzy = fuzzy or "fuzzy" in line
            continue
        if line.startswith("# "):
            comments.append(line[2:])
            continue
        if not line or line.startswith("#"):
            continue
        begins = line.startswith("msgctxt ") or (line.startswith("msgid ") and field != "ctx")
        if begins:
            if cur is not None:
                msgs.append(cur)
            cur = Msg("", "")
            cur.fuzzy = fuzzy
            cur.comments = comments
            fuzzy = False
            comments = []
        if line.startswith("msgctxt "):
            cur.ctx = unescape(line[8:].strip()[1:-1])
            field = "ctx"
        elif line.startswith("msgid_plural "):
            cur.plural = unescape(line[13:].strip()[1:-1])
            field = "plural"
        elif line.startswith("msgid "):
            cur.id = unescape(line[6:].strip()[1:-1])
            field = "id"
        elif line.startswith("msgstr["):
            index, value = line[7:].split("]", 1)
            cur.strs.append(unescape(value.strip()[1:-1]))
            field = ("str", len(cur.strs) - 1)
        elif line.startswith("msgstr "):
            cur.strs.append(unescape(line[7:].strip()[1:-1]))
            field = ("str", len(cur.strs) - 1)
        elif line.startswith('"'):
            more = unescape(line[1:-1])
            if field == "ctx":
                cur.ctx += more
            elif field == "id":
                cur.id += more
            elif field == "plural":
                cur.plural += more
            elif isinstance(field, tuple):
                cur.strs[field[1]] += more
    if cur is not None:
        msgs.append(cur)
    return [m for m in msgs if m.id]


def header(lang):
    lines = [
        "Project-Id-Version: Hora",
        "Report-Msgid-Bugs-To: https://github.com/lenkr0/hora/issues",
        "MIME-Version: 1.0",
        "Content-Type: text/plain; charset=UTF-8",
        "Content-Transfer-Encoding: 8bit",
    ]
    if lang:
        lines.insert(1, f"Language: {lang}")
        lines.append(f"Plural-Forms: {PLURAL_FORMS[lang]}")
    out = ['msgid ""', 'msgstr ""']
    out += [po_quote(l + "\n") for l in lines]
    return out


def write_po(path, msgs, lang, obsolete=()):
    nplurals = int(PLURAL_FORMS[lang].split("nplurals=")[1].split(";")[0]) if lang else 2
    out = []
    if lang:
        out.append(f"# {LANGUAGE_NAMES[lang]} translation of Hora.")
    else:
        out.append("# Hora: source texts (English). Generated by tools/i18n.py extract.")
    out += header(lang)
    out.append("")
    for m in msgs:
        for c in m.comments:
            out.append(f"# {c}")
        for ref in m.refs[:3]:
            out.append(f"#: {ref}")
        if m.fuzzy:
            out.append("#, fuzzy")
        if m.ctx:
            out.append("msgctxt " + po_quote(m.ctx))
        out.append("msgid " + po_quote(m.id))
        if m.plural is not None:
            out.append("msgid_plural " + po_quote(m.plural))
            strs = list(m.strs) + [""] * (nplurals - len(m.strs))
            for i in range(nplurals):
                out.append(f"msgstr[{i}] " + po_quote(strs[i] if lang else ""))
        else:
            out.append("msgstr " + po_quote((m.strs[0] if m.strs else "") if lang else ""))
        out.append("")
    for m in obsolete:
        if m.ctx:
            out.append("#~ msgctxt " + po_quote(m.ctx))
        out.append("#~ msgid " + po_quote(m.id))
        if m.plural is not None:
            out.append("#~ msgid_plural " + po_quote(m.plural))
            for i, s in enumerate(m.strs):
                out.append(f"#~ msgstr[{i}] " + po_quote(s))
        else:
            out.append("#~ msgstr " + po_quote(m.strs[0] if m.strs else ""))
        out.append("")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("\n".join(out).rstrip("\n") + "\n")


def po_path(lang):
    return os.path.join(TRANSLATIONS, lang, "LC_MESSAGES", "hora-app.po")


def languages():
    if not os.path.isdir(TRANSLATIONS):
        return []
    return sorted(
        d for d in os.listdir(TRANSLATIONS) if os.path.isfile(po_path(d))
    )


def merge(source, existing):
    """Quelltexte mit vorhandenen Übersetzungen; dazu die nicht mehr benutzten"""
    old = {m.key: m for m in existing}
    merged = []
    for m in source:
        n = Msg(m.ctx, m.id, m.plural)
        n.refs = m.refs
        if m.key in old:
            o = old[m.key]
            n.strs = o.strs
            n.fuzzy = o.fuzzy
            n.comments = o.comments
            # Mehrzahl hinzugekommen oder weggefallen: neu übersetzen
            if (o.plural is None) != (m.plural is None):
                n.strs = []
        merged.append(n)
    keys = {m.key for m in source}
    obsolete = [m for m in existing if m.key not in keys and any(m.strs)]
    return merged, obsolete


def problems(lang, source):
    msgs = read_po(po_path(lang))
    by_key = {m.key: m for m in msgs}
    keys = {m.key for m in source}
    missing = [m for m in source if m.key not in by_key or not all(by_key[m.key].strs) or not by_key[m.key].strs or by_key[m.key].fuzzy]
    stale = [m for m in msgs if m.key not in keys]
    bad = []
    for m in source:
        t = by_key.get(m.key)
        if not t or not t.strs:
            continue
        want = sorted(re.findall(r"\{[^{}]*\}", m.id + (m.plural or "")))
        for s in t.strs:
            got = sorted(set(re.findall(r"\{[^{}]*\}", s)))
            if not set(got) <= set(want):
                bad.append((m.id, s))
    return missing, stale, bad


def main():
    # Konsolen mit Codepage 1252 können nicht jedes Zeichen der Texte zeigen
    sys.stdout.reconfigure(errors="replace")
    cmd = sys.argv[1] if len(sys.argv) > 1 else "check"
    source = extract()
    if cmd == "extract":
        write_po(POT, source, None)
        print(f"{len(source)} Texte -> {os.path.relpath(POT, ROOT)}")
    elif cmd == "update":
        write_po(POT, source, None)
        for lang in languages():
            merged, obsolete = merge(source, read_po(po_path(lang)))
            write_po(po_path(lang), merged, lang, obsolete)
            todo = sum(1 for m in merged if not m.strs or not all(m.strs))
            print(f"{lang}: {len(merged)} Texte, {todo} offen")
    elif cmd in ("check", "stats"):
        failed = False
        for lang in languages():
            missing, stale, bad = problems(lang, source)
            print(f"{lang}: {len(source) - len(missing)}/{len(source)} übersetzt, {len(stale)} veraltet, {len(bad)} mit falschen Platzhaltern")
            if cmd == "check":
                for m in missing[:20]:
                    print(f"  fehlt: {m.id!r}")
                for m in stale[:20]:
                    print(f"  veraltet: {m.id!r}")
                for id, s in bad[:20]:
                    print(f"  Platzhalter: {id!r} -> {s!r}")
            failed |= bool(missing or stale or bad)
        if cmd == "check" and not failed:
            print("Alle Sprachen vollständig")
        if cmd == "check" and failed:
            sys.exit("Übersetzungen unvollständig: python tools/i18n.py update, dann .po-Dateien ergänzen")
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
