# Hinweise zu Drittanbieter-Werken

## Calino

Design (Farben, Typografie, Abstände, Rundungen, Animationen) und Verhalten
dieser App sind der Open-Source-App **Calino** nachempfunden; Teile der Logik
(z. B. Einrasten beim Ziehen, Spaltenaufteilung überlappender Termine,
Schnelleingabe) sind aus dem TypeScript-Quelltext nach Rust übertragen.

- Projekt: https://github.com/Ivan-Malinovski/calino
- Copyright (c) 2025 Ivan Malinovski
- Lizenz: MIT, vollständiger Text in [`THIRD_PARTY_LICENSES/Calino-LICENSE.txt`](THIRD_PARTY_LICENSES/Calino-LICENSE.txt)

## Newsreader (Schrift)

Eingebettet in `crates/app/ui/fonts/`, statische Instanzen aus Google Fonts.

- Copyright 2020 The Newsreader Project Authors (https://github.com/productiontype/Newsreader)
- Lizenz: SIL Open Font License 1.1, Text in [`THIRD_PARTY_LICENSES/Newsreader-OFL.txt`](THIRD_PARTY_LICENSES/Newsreader-OFL.txt)

## Slint (UI-Framework)

Genutzt unter der **Slint Royalty-free License 2.0** (Desktop-Anwendungen,
kostenlos). Bedingung ist ein sichtbarer Hinweis auf Slint; dafür zeigt der
Über-Dialog der App (Seitenleiste → „Über Hora“) das `AboutSlint`-Widget.

- https://slint.dev
- Lizenztext: https://github.com/slint-ui/slint/blob/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md

## Rust-Bibliotheken

Alle Crates, die in `hora.exe` eingehen, mit Version, Lizenzangabe und ihren
Lizenztexten stehen in
[`THIRD_PARTY_LICENSES/Rust-Crates.txt`](THIRD_PARTY_LICENSES/Rust-Crates.txt)
(erzeugt mit `python tools/third-party-licenses.py`). Fast alle stehen unter
MIT und/oder Apache-2.0; dazu Unicode-3.0, BSD, ISC, Zlib, BSL-1.0 und
CDLA-Permissive-2.0 (`webpki-roots`). `option-ext` steht unter der MPL-2.0;
sein Quelltext ist unverändert unter https://crates.io/crates/option-ext
erhältlich.
