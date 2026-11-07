# Änderungen

## [1.0.0] – 2026-11-06

Erste vollständige Fassung: alle geplanten Funktionen, Installer, CI.

- Kalender: CalDAV-Konten (Baïkal, Radicale; Basic Auth, HTTP und HTTPS),
  Abgleich per sync-token bzw. ctag/ETag, Offline-Warteschlange mit
  Konfliktdialog, Kalenderfarben, ein- und ausblenden.
- Ansichten: Tag, 3 Tage, Woche, Monat, Jahr, Agenda, Aufgaben, Journal;
  Mini-Kalender, Befehlspalette (`Strg+K`) mit Suche, Datumssprung, Befehlen
  und Anlegen per Satz. Hell und dunkel nach Windows, deutsch (24 h, Montag,
  Europe/Berlin), komplett per Tastatur bedienbar.
- Termine anlegen, bearbeiten, löschen, ziehen und in der Dauer ändern;
  Serien mit „Nur dieser / Dieser und alle folgenden / Alle“.
- Erinnerungen (VALARM) als Windows-Benachrichtigung, auch im Infobereich.
- Aufgaben (VTODO) mit Fälligkeit, Priorität, Teilaufgaben und
  wiederkehrenden Aufgaben; Journal (VJOURNAL) mit Markdown und Tags.
- Schnelleingabe auf Deutsch und Englisch.

Neu gegenüber 0.9.0:

- Neuer Name: Hora (Arbeitstitel war Pult). Programm `hora.exe`, Daten in
  `%LOCALAPPDATA%\Hora`.
- Abstürze und Fehler, die sonst unsichtbar blieben, stehen in
  `%LOCALAPPDATA%\Hora\hora.log`; schwere Fehler beim Start zeigt ein
  Meldungsfenster.
- Lässt sich die Datenbank nicht öffnen, beendet sich Hora mit einer Meldung,
  statt mit einer leeren Datenbank im Speicher weiterzulaufen (dort
  Eingetragenes wäre beim Beenden verloren gewesen).
- Lizenztexte aller Rust-Bibliotheken in `THIRD_PARTY_LICENSES/Rust-Crates.txt`
  (auch im Installer).
- GitHub Actions: CI (Format, Clippy, Tests, Selbsttest der Oberfläche unter
  Windows; Radicale und Baïkal unter Linux) und Release (Installer bauen,
  testen, mit Prüfsumme veröffentlichen).
- Messung: `tools\measure.ps1` nutzt immer einen eigenen Datenordner,
  `--fill-test-data` legt einen großen Testkalender an, `--time-views` misst
  Ansichten und Suche.

## [0.9.0] – 2026-11-06

- Journal (VJOURNAL) wie bei Calino: Liste nach Tag, Editor mit Datum, Titel,
  Tags, Kalender; Schreiben und Lesen (Markdown), `Strg+B` / `Strg+I`,
  automatisches Speichern; Buch-Symbol im Monat; Suche in der Palette;
  „Journal-Kalender anlegen“, wenn kein Kalender Journale annimmt.
- Abgleich auch für reine Aufgabenlisten und Journal-Kalender; sie stehen in
  der Seitenleiste.
- Warteschlange: Speichern während des Hochladens verliert keinen neueren
  Stand mehr; eine während des ersten Hochladens gelöschte oder verschobene
  Ressource wird danach auch auf dem Server gelöscht.
- Schmale Fenster: Ansichtswahl als Menü statt acht Reitern.

## [0.8.0] – 2026-11-06

- Agenda und Jahresansicht, Befehlspalette mit Suche über Termine und
  Aufgaben, Theme wechseln; kompakte Kopfzeile, Sync-Meldung per Klick.

## [0.7.0]

- Schnelleingabe auf Deutsch und Englisch („Mathe jeden Montag 8 Uhr bis 9:30
  Raum 204“), auch für Aufgaben.

## [0.6.0]

- Aufgaben (VTODO): eigene Ansicht, Teilaufgaben, Priorität, wiederkehrende
  Aufgaben, Erinnerung zur Fälligkeit, Aufgaben im Kalender.

## [0.5.0]

- Erinnerungen als Windows-Benachrichtigung, Infobereich, Autostart,
  Einstellungen, Installer.

## [0.4.0]

- Wiederkehrende Termine bearbeiten: nur dieser, dieser und alle folgenden,
  alle.

## [0.3.0]

- Ziehen und Dauer ändern mit Maus und Tastatur.

## [0.2.0]

- Monat, Woche und Tag; Termine anlegen, bearbeiten und löschen; Offline-
  Warteschlange und Konflikte.

## [0.1.0]

- CalDAV-Konten und Kalender, Abgleich, Design nach Calino.
