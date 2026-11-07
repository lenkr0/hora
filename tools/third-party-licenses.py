"""Lizenztexte aller Rust-Crates, die in hora.exe eingehen (normale
Abhängigkeiten von hora-app für Windows, ohne Build- und Test-Abhängigkeiten),
nach THIRD_PARTY_LICENSES/Rust-Crates.txt schreiben.

    python tools/third-party-licenses.py

Braucht nur `cargo metadata` (offline, die Quellen liegen nach einem Build im
Cargo-Register). Mit --check bricht es ab, wenn die Datei veraltet ist (CI).
"""
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "THIRD_PARTY_LICENSES", "Rust-Crates.txt")
TARGET = "x86_64-pc-windows-msvc"
FILES = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT", "UNLICENSE")


def metadata():
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", TARGET],
        cwd=ROOT, check=True, capture_output=True,
    ).stdout
    return json.loads(out)


def shipped(meta):
    """IDs der Pakete, die mit den Standard-Features in hora-app landen.

    `cargo metadata` nimmt auch optionale Abhängigkeiten nicht gewählter
    Features mit (z. B. Skia); `cargo tree` zeigt genau den Build."""
    out = subprocess.run(
        ["cargo", "tree", "-p", "hora-app", "-e", "normal", "--target", TARGET,
         "--prefix", "none", "--locked", "-f", "{p}"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout
    wanted = set()
    for line in out.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[1].startswith("v"):
            wanted.add((parts[0], parts[1][1:]))
    return {p["id"] for p in meta["packages"] if (p["name"], p["version"]) in wanted}


def license_files(package):
    folder = os.path.dirname(package["manifest_path"])
    found = []
    for name in sorted(os.listdir(folder)):
        upper = name.upper()
        if upper.startswith(FILES) and os.path.isfile(os.path.join(folder, name)):
            found.append(os.path.join(folder, name))
    if package.get("license_file"):
        extra = os.path.join(folder, package["license_file"])
        if os.path.isfile(extra) and extra not in found:
            found.append(extra)
    return found


def main():
    meta = metadata()
    ids = shipped(meta)
    packages = sorted(
        (p for p in meta["packages"] if p["id"] in ids and p["source"] is not None),
        key=lambda p: (p["name"].lower(), p["version"]),
    )
    lines = [
        "Lizenzen der Rust-Bibliotheken in hora.exe",
        "==========================================",
        "",
        "Erzeugt mit tools/third-party-licenses.py aus Cargo.lock. Je Crate: Name,",
        "Version, Lizenzangabe und die Lizenzdateien des Pakets. Pakete ohne",
        "eigene Datei stehen am Ende; für sie gilt der Standardtext der genannten",
        "Lizenz (MIT, Apache-2.0 usw., siehe die Texte anderer Pakete oben).",
        "",
    ]
    without = []
    texts = {}
    for p in packages:
        files = license_files(p)
        if not files:
            without.append(p)
            continue
        lines.append("-" * 78)
        lines.append(f"{p['name']} {p['version']} ({p.get('license') or 'ohne Angabe'})")
        if p.get("repository"):
            lines.append(p["repository"])
        lines.append("")
        for f in files:
            with open(f, encoding="utf-8", errors="replace") as fh:
                text = fh.read().strip()
            # Gleiche Texte (z. B. identische Apache-2.0) nur einmal ausschreiben
            key = text
            if key in texts:
                lines.append(f"[{os.path.basename(f)}: gleicher Text wie bei {texts[key]}]")
            else:
                texts[key] = f"{p['name']} {p['version']}"
                lines.append(f"[{os.path.basename(f)}]")
                lines.append(text)
            lines.append("")
    if without:
        lines.append("-" * 78)
        lines.append("Pakete ohne eigene Lizenzdatei:")
        lines.append("")
        for p in without:
            lines.append(f"  {p['name']} {p['version']} ({p.get('license') or 'ohne Angabe'})")
        lines.append("")
    content = "\n".join(lines).replace("\r\n", "\n")
    if "--check" in sys.argv:
        current = open(OUT, encoding="utf-8").read() if os.path.exists(OUT) else ""
        if current != content:
            sys.exit("THIRD_PARTY_LICENSES/Rust-Crates.txt ist veraltet: python tools/third-party-licenses.py")
        print(f"{len(packages)} Crates, Datei aktuell")
        return
    with open(OUT, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(content)
    print(f"{len(packages)} Crates ({len(without)} ohne eigene Datei) -> {OUT}")


if __name__ == "__main__":
    main()
