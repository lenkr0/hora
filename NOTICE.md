# Third-party notices

## Calino

The design (colors, typography, spacing, corner radii, animations) and
behavior of this app are modeled on the open-source app **Calino**; parts of
the logic (e.g. snapping while dragging, column layout of overlapping events,
quick add) were ported from its TypeScript source to Rust.

- Project: https://github.com/Ivan-Malinovski/calino
- Copyright (c) 2025 Ivan Malinovski
- License: MIT, full text in [`THIRD_PARTY_LICENSES/Calino-LICENSE.txt`](THIRD_PARTY_LICENSES/Calino-LICENSE.txt)

## Newsreader (font)

Embedded in `crates/app/ui/fonts/`, static instances from Google Fonts.

- Copyright 2020 The Newsreader Project Authors (https://github.com/productiontype/Newsreader)
- License: SIL Open Font License 1.1, text in [`THIRD_PARTY_LICENSES/Newsreader-OFL.txt`](THIRD_PARTY_LICENSES/Newsreader-OFL.txt)

## Slint (UI framework)

Used under the **Slint Royalty-free License 2.0** (desktop applications, free
of charge). It requires a visible attribution to Slint; the app's About dialog
(sidebar → "About Hora") shows the `AboutSlint` widget for this.

- https://slint.dev
- License text: https://github.com/slint-ui/slint/blob/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md

## Rust libraries

All crates that go into `hora.exe`, with version, license and their license
texts, are listed in
[`THIRD_PARTY_LICENSES/Rust-Crates.txt`](THIRD_PARTY_LICENSES/Rust-Crates.txt)
(generated with `python tools/third-party-licenses.py`). Almost all are under
MIT and/or Apache-2.0; also Unicode-3.0, BSD, ISC, Zlib, BSL-1.0 and
CDLA-Permissive-2.0 (`webpki-roots`). `option-ext` is under the MPL-2.0; its
unmodified source is available at https://crates.io/crates/option-ext.
