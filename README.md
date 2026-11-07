# Hora

*Desktop-Kalender für Windows, nach dem Vorbild von Calino.*

Nativer CalDAV-Kalender für Windows, komplett in Rust mit [Slint](https://slint.dev),
ohne WebView. Design und Verhalten folgen der Open-Source-App
[Calino](https://github.com/Ivan-Malinovski/calino) (MIT), siehe [NOTICE.md](NOTICE.md).
Hora ist ein eigenständiges Projekt, kein offizielles Calino-Programm.

**Stand: Version 1.0** (Änderungen in [CHANGELOG.md](CHANGELOG.md)). CalDAV-Konten mit Kalenderfarben; Tag, 3 Tage,
Woche, Monat, Jahr und Agenda; Termine anlegen, bearbeiten, löschen, per Ziehen
verschieben und in der Dauer ändern. Wiederkehrende Termine lassen sich im
Dialog anlegen und ändern; wer einen Termin einer Serie ändert, zieht oder
löscht, wählt wie bei Calino „Nur dieser Termin“, „Dieser und alle
folgenden“ oder „Alle Termine der Serie“. Erinnerungen (VALARM) kommen als
Windows-Benachrichtigung, auch wenn das Fenster geschlossen ist und Hora im
Infobereich läuft. Aufgaben (VTODO) mit Fälligkeit, Priorität und
Teilaufgaben stehen in einer eigenen Ansicht wie bei Calino und am Tag ihrer
Fälligkeit im Kalender. Mit der Schnelleingabe (Taste `Q`) entsteht ein
Termin oder eine Aufgabe aus einem Satz auf Deutsch oder Englisch, z. B.
„Mathe jeden Montag 8 Uhr bis 9:30 Raum 204“. Die Befehlspalette (`Strg+K`)
sucht in Terminen, Aufgaben und Journaleinträgen, springt zu einem Datum,
führt Befehle aus und legt per Satz an. Im Journal (VJOURNAL) stehen freie
Notizen an Kalendertagen, mit Markdown und Tags. Änderungen gehen sofort in den lokalen Cache und werden
im Hintergrund hochgeladen (auch nach Offline-Phasen); hat ein anderes Gerät
denselben Termin geändert, fragt Hora nach.

## Installieren

`Hora-Setup-1.0.0.exe` installiert für das eigene Benutzerkonto nach
`%LOCALAPPDATA%\Programs\Hora`, ohne Administratorrechte: Startmenü-Eintrag,
Deinstallation unter Einstellungen → Apps, auf Wunsch Autostart (vorgewählt)
und Desktop-Verknüpfung. Ein Update beendet ein laufendes Hora vorher
(`hora --quit`). Die Deinstallation fragt, ob auch die lokalen Daten weg
sollen; die Termine auf dem Server bleiben immer unberührt. Still: `/S`,
anderer Ordner: `/D=C:\Pfad`. Releases auf GitHub haben daneben eine
`.sha256`-Datei (`Get-FileHash Hora-Setup-1.0.0.exe` muss denselben Wert
zeigen). Mitgeliefert werden `LICENSE`, `NOTICE.md` und die Lizenztexte aller
Bibliotheken (`THIRD_PARTY_LICENSES`).

## Bauen und starten

Voraussetzungen: Rust (stable, MSVC) und die Visual Studio Build Tools mit
„Desktopentwicklung mit C++“; für den Installer NSIS 3 (`winget install NSIS.NSIS`).

```
cargo run --release -p hora-app
powershell -ExecutionPolicy Bypass -File tools\build-installer.ps1   # target\installer\Hora-Setup-<Version>.exe
```

Das Programmsymbol erzeugt `python tools/make-icon.py` (Pillow) neu.

## Konto hinzufügen

Seitenleiste → „Konto hinzufügen“ (oder das + neben „KALENDER“):

- **Server-Adresse:** die DAV-Adresse, bei Baïkal z. B. `http://server:8089/dav.php`.
  Hora findet Principal, Kalender-Sammlung und Kalender selbst (auch über
  `/.well-known/caldav`).
- **Benutzername und Passwort:** HTTP Basic Auth. Das Passwort liegt im
  Windows-Anmeldeinformationsspeicher (Eintrag `konto-….Hora`), nie in der
  Datenbank. Bei `http://` warnt der Dialog, dass das Passwort unverschlüsselt
  übertragen wird.

Hora gleicht beim Start, alle 5 Minuten und auf Knopfdruck (Symbol oben rechts
oder F5) ab: per sync-token (RFC 6578), sonst per ctag/ETag. Ohne Netz bleiben
die zuletzt geladenen Termine sichtbar. Abgeglichen werden alle Kalender, auch
reine Aufgabenlisten und Kalender nur für Journaleinträge; die Seitenleiste
zeigt sie alle zum Ein- und Ausblenden.

## Bearbeiten und Offline

- Jede Änderung landet sofort im lokalen Cache und in einer Warteschlange.
  Der Sync-Thread lädt sie hoch: PUT mit `If-Match` (Änderung) bzw.
  `If-None-Match: *` (neu), DELETE mit `If-Match`.
- Ohne Netz bleibt die Änderung erhalten und wird nach 30 s, 1 min, 2 min …
  (höchstens stündlich) erneut versucht. Die Kopfzeile zeigt „N Änderungen
  warten auf Übertragung“.
- Hat ein anderes Gerät den Termin inzwischen geändert (HTTP 412), fragt Hora:
  „Meine Version behalten“ oder „Server-Version übernehmen“.
- Unbekannte Eigenschaften (Alarme, Teilnehmer, Felder anderer Programme)
  bleiben beim Bearbeiten erhalten; neue Termine bekommen eine VTIMEZONE.
- Wird eine Ressource erneut gespeichert, während der vorige Stand noch
  hochgeladen wird (das Journal speichert beim Tippen), geht der neuere Stand
  danach gegen das neue ETag hinterher; wird eine neue Ressource in dieser
  Zeit gelöscht oder verschoben, löscht Hora sie anschließend auf dem Server.

Daten: `%LOCALAPPDATA%\Hora\hora.db` (SQLite). Fehler, die sonst unsichtbar
blieben (Absturz, Benachrichtigung nicht zustellbar, Infobereich), stehen in
`%LOCALAPPDATA%\Hora\hora.log` (höchstens 256 KB, dann `hora.log.1`).
Lässt sich die Datenbank nicht öffnen (z. B. beschädigt oder gesperrt),
meldet Hora das und beendet sich, statt leer zu starten.

## Serien

Im Termin-Dialog unter „Wiederholen“: täglich, wöchentlich (an gewählten
Wochentagen), monatlich (am selben Tag, am n-ten oder letzten Wochentag),
jährlich, jeweils „alle n …“ und mit Ende nie, am Datum oder nach Anzahl.
Darunter steht die Regel in Worten, z. B. „Wöchentlich am Dienstag und
Donnerstag, 4-mal“.

- **Nur dieser Termin:** Hora legt eine Ausnahme an (VEVENT mit
  `RECURRENCE-ID` in derselben Ressource, mit Alarmen und Feldern der Serie);
  Löschen setzt ein `EXDATE`.
- **Dieser und alle folgenden:** Die Serie wird geteilt, so wie es
  Thunderbird und Google machen. Die alte endet vor dem Termin (`UNTIL`,
  bei fester Anzahl ein kleineres `COUNT`), die neue beginnt dort als eigene
  Ressource mit neuer UID. Spätere Ausnahmen und `EXDATE`s ziehen mit um.
- **Alle Termine der Serie:** Die Serie verschiebt sich um dieselbe Wandzeit
  wie der bearbeitete Termin, auch über die Zeitumstellung hinweg (8 Uhr
  bleibt 8 Uhr). Wochentage der Regel, Ausnahmen und `EXDATE`s wandern mit.
- Maßstab ist der Termin, wie er angezeigt wurde: Wer an einer verlegten
  Ausnahme nur den Ort ändert und „alle“ wählt, verschiebt die Serie nicht.
  Neue Titel gehen nur an Ausnahmen, die noch den Titel der Serie hatten.
- Wird die Regel selbst geändert, gibt es „nur dieser Termin“ nicht;
  Ausnahmen, die zu keinem Termin der neuen Regel mehr gehören, entfallen.
- Regeln aus anderen Programmen, die der Dialog nicht darstellen kann
  (z. B. `BYMONTHDAY=1,15` oder `BYSETPOS`), erscheinen als „Eigene Regel“
  und bleiben unverändert, solange man dort nichts anderes wählt.

## Erinnerungen und Infobereich

- Im Termin-Dialog unter „Erinnerungen“: Chips wie bei Calino, „+ Erinnerung“
  bietet „Zum Zeitpunkt des Termins“ bis „1 Woche vorher“, bei ganztägigen
  Terminen „Am Tag um 09:00“, „Am Vortag um 09:00“ usw. Neue Termine
  beginnen mit der Standarderinnerung (voreingestellt 15 Minuten, in den
  Einstellungen änderbar). Hora schreibt `VALARM` mit `ACTION:DISPLAY` und
  `TRIGGER:-PT15M`; Alarme anderer Programme (E-Mail, feste Uhrzeit, vor dem
  Ende) bleiben erhalten und erscheinen im Dialog nur als Chip.
- Bei Serien folgen Erinnerungen dem Umfang: „nur dieser“ ändert die
  Ausnahme, „alle“ die Serie und Ausnahmen, die bisher dieselben Erinnerungen
  hatten.
- Die Benachrichtigung zeigt Titel, „In 15 Minuten · 15:00–16:00“ und den Ort.
  „Schlummern“ (5 Minuten bis 1 Stunde) und „Schließen“ erledigt Windows
  selbst; ein Klick öffnet Hora am Tag des Termins.
- Verpasste Erinnerungen (Hora lief nicht) kommen nach dem Start, solange der
  Termin nicht vorbei ist, höchstens 24 Stunden zurück. Jede kommt nur
  einmal; wird ein Termin verschoben, kommt seine Erinnerung neu. Was ein
  anderes Gerät schon weggeklickt hat (`ACKNOWLEDGED`, Thunderbirds
  `X-MOZ-LASTACK`), meldet Hora nicht noch einmal.
- Erinnerungen gibt es für sichtbare Kalender.
- Schließen des Fensters legt Hora in den Infobereich (abschaltbar); das
  Symbol öffnet es wieder, Rechtsklick zeigt „Hora öffnen“, „Jetzt
  abgleichen“, „Einstellungen …“, „Beenden“. Ein zweiter Start holt das
  laufende Fenster nach vorn. `hora --tray` startet ohne Fenster (Autostart).
- Einstellungen (Seitenleiste unten oder `Strg+,`): Terminerinnerungen,
  Standarderinnerung, Testbenachrichtigung, Mit Windows starten, Im
  Infobereich weiterlaufen. Schaltet Windows die Benachrichtigungen für Hora
  aus, steht das dort.
- Technik: Hora meldet für Benachrichtigungen die App-ID `Hora.Kalender`
  unter `HKCU\Software\Classes\AppUserModelId` an (wie das Windows App SDK
  für Programme ohne Paket); Autostart steht unter `HKCU\…\Run`.

## Schnelleingabe

Taste `Q` (oder „Schnell eintragen“ in der Befehlspalette) öffnet ein
Eingabefeld wie Calinos Smart Input. Darunter zeigt eine Vorschau, was entsteht: Art,
Wann, Ort, Wiederholung, Erinnerung, Kalender. Eingabe legt an,
Umschalt+Eingabe (oder „Im Dialog öffnen“) öffnet den vorbefüllten Termin-
bzw. Aufgaben-Dialog. Die Eingabezeile der Aufgaben-Ansicht versteht
dieselben Angaben („Milch kaufen morgen“).

| Eingabe | Ergebnis |
|---|---|
| `Mathe jeden Montag 8 Uhr bis 9:30 Raum 204` | „Mathe“, wöchentlich montags 8:00–9:30, Ort „Raum 204“ |
| `Zahnarzt morgen 15 Uhr bei Dr. Weber` | morgen 15:00–16:00, Ort „Dr. Weber“ |
| `Chor dienstags und donnerstags 19 bis 21 Uhr bis 30.6.2027` | wöchentlich Di und Do bis 30.06.2027 |
| `Kurzurlaub vom 20.11. bis 23.11.` | ganztägig, vier Tage |
| `Lunch with Anna tomorrow at 1pm #Arbeit` | morgen 13:00 im Kalender „Arbeit“ |
| `Gym every other monday 7am` | alle zwei Wochen montags 7:00 |
| `todo Milch kaufen bis Freitag` | Aufgabe, fällig Freitag |
| `Planung morgen 10 Uhr Erinnerung 30 min vorher` | mit Erinnerung 30 Minuten vorher |

Erkannt werden:
- **Tage:** heute, morgen, übermorgen, Wochentage (auch „nächsten
  Donnerstag“), `12.11.`, `12.11.2026`, `12. November`, `November 20th`,
  „in 3 Tagen“, „nächste Woche“, „am Wochenende“.
- **Uhrzeiten:** `15 Uhr`, `um 15`, `14:30`, `9.30 Uhr`, `3pm`, „halb 8“,
  „viertel nach 9“, „heute Abend“.
- **Zeiträume und Dauer:** `8 bis 9:30`, `15-16 Uhr`, „von 10 bis 11:15“,
  „zwischen 2pm und 3pm“, „für 2 Stunden“, `90 min`.
- **Wiederholungen:** jeden Tag, jede Woche, täglich, werktags, montags,
  alle zwei Wochen, every other Monday, jeweils mit „bis …“ oder „10 mal“.
- **Ort, Art, Kalender, Erinnerung:** „bei …“, „im …“, `@ …` und Raum/Zimmer/Saal;
  Aufgaben mit `todo`, `Aufgabe:` oder `remind me to`; der Kalender per
  `#Name` (Anfang des Namens reicht); „Erinnerung 15 min vorher“ bzw.
  `remind 1 hour before`.

Ohne Uhrzeit wird der Termin ganztägig, ohne Ende dauert er eine Stunde (wie
bei Calino); liegt eine Uhrzeit ohne Datum heute schon zurück, gilt morgen.
Nackte Zahlen („3 Kartons“) bleiben im Titel.

## Agenda, Jahr und Befehlspalette

Wie bei Calino gibt es neben Tag, 3 Tagen, Woche und Monat zwei Übersichten:

- **Agenda** (`L` oder `G`): die Tage eines Monats untereinander, je Tag die
  Termine und fälligen Aufgaben als Karten mit Uhrzeit, Ende und Ort.
  Mehrere Tage ohne Termine stehen als eine Zeile da („3 Tage ohne Termine ·
  10.–12. Nov.“), heute steht immer da und ist markiert; die Liste beginnt
  bei heute. Klick auf den Wochentag öffnet den Tag, „+ Hinzufügen“ legt dort
  einen Termin an, Klick auf eine Karte zeigt die Vorschau.
- **Jahr** (`J` oder `Y`): zwölf Monate, ein Punkt an jedem Tag mit Terminen,
  heute hervorgehoben. Klick auf einen Tag öffnet ihn, Klick auf den
  Monatsnamen den Monat, Doppelklick legt einen Termin an. Mit der Tastatur:
  Pfeile wählen den Tag, Eingabe öffnet ihn, `N` legt an.

Die **Befehlspalette** öffnet `Strg+K` (oder `/`, oder die Lupe in der
Kopfzeile). Ohne Eingabe zeigt sie die wichtigsten Befehle; beim Tippen
sucht sie in Gruppen wie Calino:

| Gruppe | Was | Beispiel |
|---|---|---|
| Navigation | Ansichten, heute, morgen, zurück/weiter; ein Datum allein springt dorthin | `woche`, `24.12.` → „Zu Do. 24. Dez. wechseln“ |
| Aktionen | Termin erstellen, Neue Aufgabe, Neuer Journaleintrag, Schnell eintragen, Synchronisieren, Konto hinzufügen | `sync` |
| Einstellungen | Einstellungen öffnen, Theme wechseln (System → hell → dunkel), Über Hora | `dunkel` |
| Kalender | sichtbare und ausgeblendete Kalender ein-/ausblenden | `kalender arbeit` |
| Termine | Titel, Ort und Beschreibung aller Termine sichtbarer Kalender; bei Serien das nächste Vorkommen | `lauftreff park` |
| Aufgaben | Titel und Beschreibung | `bericht` |
| Journal | Titel, Text und Tags der Journaleinträge | `schnee` |
| Schnell hinzufügen | „Erstellen: …“ bzw. „Aufgabe: …“ aus der Schnelleingabe, mit Wann, Ort und Kalender | `Kino Freitag 20 Uhr #Privat` |

Alle Wörter müssen vorkommen (Groß-/Kleinschreibung egal). Pfeile wählen,
Eingabe führt aus, Esc schließt. Erkennt die Schnelleingabe Datum, Uhrzeit,
Wiederholung oder Ort, steht „Erstellen: …“ ganz oben, sonst unten nach den
Treffern. Ein Termin öffnet sich in seiner Woche mit der Vorschau, eine
Aufgabe in ihrem Dialog, ein Journaleintrag im Journal. Fehlt zum Anlegen etwas (z. B. ein unbekannter
`#Kalender`), öffnet sich die Schnelleingabe mit dem Text und dem Hinweis.

## Aufgaben

Die Ansicht „Aufgaben“ (oben rechts oder Taste `A`) folgt Calinos Aufgabenliste:

- Oben „N aktiv · M erledigt“, die Filter „Alle / Aktiv / Erledigt“ und
  „+ Aufgabe erstellen“; darunter die Eingabezeile „Was muss erledigt werden?“
  (Eingabe legt eine Aufgabe ohne Datum im zuletzt benutzten Kalender an).
- Gruppen nach Fälligkeit: Überfällig (auch heute fällig mit vergangener
  Uhrzeit), Heute, Diese Woche, Später, Kein Fälligkeitsdatum, Erledigt.
  Innerhalb einer Gruppe nach Fälligkeit, dann Priorität.
- Teilaufgaben (`RELATED-TO;RELTYPE=PARENT`) stehen eingerückt unter ihrer
  Aufgabe, mit „erledigt/alle“ und einklappbar. Wer eine Aufgabe abhakt, hakt
  ihre offenen Teilaufgaben mit ab; Löschen nimmt sie ebenfalls mit.
- Der Dialog: Titel, Fällig (kein Datum, nur Datum, Datum und Uhrzeit),
  Beginn, Priorität (hoch/mittel/niedrig, RFC-Werte 1/5/9; fremde Werte der
  gleichen Stufe bleiben), Kalender, übergeordnete Aufgabe, Beschreibung,
  „+ Teilaufgabe“.
- Wiederkehrende Aufgaben wie bei Thunderbird und Calino: Die Serie behält
  ihre `RRULE`; ein erledigtes Vorkommen wird eine Ausnahme mit
  `RECURRENCE-ID` und `STATUS:COMPLETED` in derselben Ressource. Gezeigt wird
  das erste offene Vorkommen nach dem zuletzt erledigten.
- Offene Aufgaben mit Fälligkeit erscheinen in Woche und Monat als
  ganztägiger Eintrag mit Kreis (abschaltbar in den Einstellungen); ein Klick
  öffnet den Dialog.
- Erinnerungen an Fälligkeitstermine (Einstellung, voreingestellt an): zur
  Fälligkeitszeit, bei Aufgaben nur mit Datum um 9:00 Uhr. Hat eine Aufgabe
  eigene `VALARM`s (z. B. aus Thunderbird), gelten diese stattdessen.

## Journal

Die Ansicht „Journal“ (oben rechts oder Taste `B`) folgt Calinos Journal:
freie Notizen an einem Kalendertag, gespeichert als `VJOURNAL` im Kalender.

- Oben die Anzahl, „Monat / Alle“ und „+ Neuer Eintrag“. Die Liste steht nach
  Tag absteigend; jede Zeile zeigt Tag, Wochentag, Monat, Titel, zwei Zeilen
  Text und die Tags. Im Monat blättern „Zurück“ und „Weiter“ (bzw. `,`/`.`).
- Klick auf eine Zeile öffnet den Eintrag rechts im Editor, ein zweiter Klick
  schließt ihn. Bei schmalen Fenstern liegt der Editor über der Liste
  („← Alle Einträge“).
- Der Editor: Datum (Klick ändert es, z. B. `13.11.`, `morgen`), Titel
  (optional), Tags („+ Tag“, Eingabe; mehrere mit Komma), Kalender (wenn
  mehrere Journale annehmen; wechselt den Kalender, auch auf dem Server),
  Löschen, darunter der Text. „Schreiben / Lesen“ unten: Lesen zeigt den Text
  als Markdown (Überschriften, Listen, fett, kursiv, durchgestrichen,
  `Code`, Zitate, Linien, Links); Doppelklick darauf schreibt weiter.
  `Strg+B` / `Strg+I` macht die Auswahl fett bzw. kursiv (noch einmal: wieder
  normal). Die Fußzeile zählt die Wörter.
- Gespeichert wird eine halbe Sekunde nach der letzten Eingabe (wie bei
  Calino), beim Wechsel des Eintrags oder der Ansicht und beim Schließen
  sofort. Rechts unten steht der Stand: „Nicht gespeicherte Änderungen“,
  „Speichere …“, „Gespeichert“, „Lokal gespeichert, wird übertragen“ (ohne
  Netz) oder ein Fehler. Ein neuer Eintrag ohne Titel, Text und Tags wird
  verworfen.
- Neue Einträge sind ganztägig (`DTSTART;VALUE=DATE`), heute bzw. am Ersten
  des gezeigten Monats. Titel ist `SUMMARY`, Text `DESCRIPTION`, Tags sind
  `CATEGORIES`. Einträge anderer Programme mit Uhrzeit behalten sie; mehrere
  `DESCRIPTION`-Zeilen (RFC 5545 erlaubt das) liest Hora mit Leerzeile
  verbunden und lässt sie stehen, bis man den Text ändert.
- Im Monat zeigt ein Buch-Symbol oben rechts im Tag, dass es Journaleinträge
  gibt; ein Klick öffnet sie.
- Journaleinträge gehen in Kalender, die `VJOURNAL` annehmen (bei Baïkal:
  „Notes“ am Kalender angehakt). Hat keiner das, bietet die Ansicht
  „Journal-Kalender anlegen“: Hora legt per MKCALENDAR einen Kalender
  „Journal“ nur für Journaleinträge im ersten Konto an.
- Ohne Konto stehen Beispiel-Einträge da; wie die Beispieltermine leben sie
  nur im Speicher.

## Bedienung

| Aktion | Maus | Tastatur |
|---|---|---|
| Ansicht | Tag / 3 Tage / Woche / Monat / Jahr / Agenda / Aufgaben / Journal oben rechts (schmal: Auswahlmenü) | `D`/`1`, `X`/`2`, `W`/`3`, `M`/`4`, `J`/`Y`/`5`, `L`/`G`/`6`, `A`/`7`, `B`/`8` |
| Befehlspalette, Suche | Lupe in der Kopfzeile | `Strg+K` oder `/`; ↑/↓, Eingabe, Esc |
| Zurück / Weiter / Heute | Pfeile, „Heute“ | `,` / `.` (wie Calino), Bild↑/↓, `T` |
| Zu einem Tag springen | Mini-Kalender; im Monat die Tageszahl; im Jahr der Tag; in der Agenda der Wochentag | im Monat und Jahr: Pfeile, Eingabe; Palette: Datum eintippen |
| Neuer Termin | Doppelklick ins Raster bzw. in einen Monatstag | `C` oder `N`; im Raster: Pfeile wählen die Zeit, Eingabe |
| Vorschau | Klick auf einen Termin | Tab bis zur Karte, Eingabe |
| Bearbeiten | Doppelklick oder „Bearbeiten“ in der Vorschau | in der Vorschau `E` |
| Löschen | „Löschen“ in Vorschau oder Dialog | Entf auf der Karte oder in der Vorschau |
| Verschieben | Karte ziehen (15-Minuten-Raster); im Monat auf einen anderen Tag | ↑/↓ = ±15 min, ←/→ = ±1 Tag |
| Dauer ändern | unteren Rand ziehen | Umschalt+↑/↓ |
| Serie: welche Termine? | Knopf im Serien-Dialog | Tab, Eingabe; Esc bricht ab (Ziehen springt zurück) |
| Wochentage einer Serie | Mo … So im Dialog anklicken | Tab, Leertaste |
| Kalender ein-/ausblenden, Farbe | Zeile bzw. Farbpunkt in der Seitenleiste | Tab, Leertaste |
| Jetzt abgleichen | Symbol oben rechts; Klick auf eine rote Meldung zeigt sie ganz | F5 |
| Erinnerung hinzufügen / entfernen | „+ Erinnerung“, × am Chip | Tab, ↑/↓ wählt die Vorgabe, Eingabe; am × Entf |
| Einstellungen | „Einstellungen“ unten in der Seitenleiste | `Strg+,` |
| Fenster zurückholen | Symbol im Infobereich | Hora erneut starten |
| Schnelleingabe | „Schnell eintragen“ in der Befehlspalette | `Q`; Eingabe legt an, Umschalt+Eingabe öffnet den Dialog |
| Aufgabe abhaken | Kreis vor dem Titel | Tab bis zur Zeile, Leertaste |
| Aufgabe öffnen / löschen | Klick auf die Zeile; „Löschen“ im Dialog | Eingabe / Entf |
| Teilaufgaben ein-/ausklappen | Pfeil vor dem Titel | ← / → |
| Neue Aufgabe | „+ Aufgabe erstellen“ oder Eingabezeile | in der Aufgaben-Ansicht `C` oder `N` |
| Neuer Journaleintrag | „+ Neuer Eintrag“ | im Journal `C` oder `N`; Eingabe springt vom Titel in den Text |
| Journaleintrag wählen | Klick auf die Zeile (noch einmal: schließen) | ↑/↓ (auch aus der Ansicht heraus), Esc schließt |
| Journal schreiben / lesen | „Schreiben / Lesen“, Doppelklick in den Text | Eingabe oder Leertaste auf der Zeile; Esc im Text liest |
| Journal formatieren | – | `Strg+B` fett, `Strg+I` kursiv |
| Journaleintrag löschen | Papierkorb im Editor | Entf auf der Zeile, mit Rückfrage |

Datum und Uhrzeit im Dialog verstehen z. B. `4.11.`, `04.11.26`, `morgen`,
`930`, `9.30`, `9 Uhr`.

## Renderer

Hora nutzt standardmäßig Slints Software-Renderer: rund 12 MB privater
Arbeitsspeicher im Leerlauf (Skia gut 70 MB), und er funktioniert auch per
Remotedesktop, wo femtovg mangels OpenGL abbricht. Anderer Renderer:

```
$env:SLINT_BACKEND = "winit-femtovg"; cargo run --release -p hora-app
```

## Tests

```
cargo test --workspace                                                # Unit-Tests
cargo test -p hora-caldav --test servers -- --ignored --test-threads=1 # Radicale + Baïkal in Docker
cargo test -p hora-platform -- --ignored                              # Passwortspeicher, Registry, echte Benachrichtigung
```

Ende-zu-Ende (Radicale-Container mit Testterminen, Hora verbindet sich und
macht einen Screenshot; berührt weder echte Daten noch den Passwortspeicher).
Mit `-EditTest` arbeitet Hora über dieselben Callbacks wie die Oberfläche:
Termin anlegen, einen verschieben (nach einer Änderung durch ein „anderes
Gerät“, also mit Konfliktdialog), einen löschen, eine Serie für alle Termine
umbenennen, einen Serientermin nur für sich löschen, eine Serie ab Freitag
verschieben (Teilung), einen Termin nur diese Woche verlegen und eine neue
Serie (Di/Do, 4-mal) anlegen. Außerdem liegt auf dem Server ein Termin in
10 Minuten mit „15 Minuten vorher“: Hora muss ihn als Windows-Benachrichtigung
zeigen, das Skript prüft das im Infocenter und entfernt den Test-Toast
wieder. Zu Aufgaben liegen auf dem Server eine überfällige, eine ohne Datum,
eine mit Teilaufgabe, eine wöchentliche und eine seit 5 Minuten fällige; Hora
legt per Schnelleingabe und Dialog Aufgaben und eine Teilaufgabe an, hakt ab
(auch ein Serienvorkommen und eine Aufgabe mit Teilaufgabe) und muss zur
fälligen eine Erinnerung zeigen. Über die Schnelleingabe legt es „Physik jeden
Montag 8 Uhr bis 9:30 Raum 204“, „Optiker morgen 15 Uhr bei Dr. Weber“,
„todo Geschenk kaufen #Arbeit“ und in der Aufgabenzeile „Unterlagen sortieren
bis Freitag“ an. In der Befehlspalette sucht es „lauftreff park“ (Vorschau
des Treffers), springt mit „24.12.“ in den Dezember, führt „Wochenansicht“
aus und legt „Elternabend übermorgen 19 Uhr in der Aula“ an. Im Journal liest
es aus einem Kalender nur für Journaleinträge einen Eintrag „eines anderen
Programms“ (mit Uhrzeit und fremder Eigenschaft) und benennt ihn um, legt
einen Eintrag mit Markdown und Tag an, speichert einen anderen sechsmal
schnell hintereinander (während hochgeladen wird), verschiebt einen in einen
anderen Kalender, löscht einen nach Rückfrage, findet einen über die Palette
und legt über den Knopf einen Journal-Kalender an. Danach prüft es den Stand
direkt auf dem Server (34 Prüfungen); `-Shots ordner` speichert
zusätzlich Bilder der Dialoge und Ansichten:

```
cargo build -p hora-app --features devtools
powershell -ExecutionPolicy Bypass -File tools\e2e-radicale.ps1 -Exe target\debug\hora.exe -EditTest -Shots screenshots
```

Infobereich und Installer (eigener Datenordner, der Installer-Test
installiert still in einen Testordner und deinstalliert danach wieder):

```
powershell -ExecutionPolicy Bypass -File tools\shell-test.ps1 -Exe target\release\hora.exe
powershell -ExecutionPolicy Bypass -File tools\installer-test.ps1 -Setup target\installer\Hora-Setup-1.0.0.exe
```

Weitere Entwicklerwerkzeuge (Feature `devtools`): `--selftest` (Ziehen,
Dauer, Tastatur; außerdem Strg+K, Tippen und Eingabe in der Palette, Pfeile
und Eingabe im Jahr, `L` für die Agenda, `/` und Esc; im Journal `B`, `N`,
Tippen, `Strg+B`, Speichern nach der Pause, Esc, Pfeile, Tags und Entf mit
Rückfrage), `--snapshot bild.png`, `--view 0–7` (Tag … Journal),
`--journal <titel>`, `--journal-write <titel>`, `--dialog`, `--about`,
`--new-event`, `--scope`, `--settings`, `--preview <titel>`, `--edit <titel>`,
`--edit-task <titel>`, `--quick <text>`, `--quick-dialog <text>`,
`--palette <text>`, `--sync-popup`, `--reminder-check`, `--fill-test-data`,
`--time-views`. Umgebung:
`HORA_THEME=light|dark`, `HORA_DATA_DIR=…` (andere Datenbank),
`HORA_CREDENTIALS=memory` (Passwörter nur im Arbeitsspeicher).

Startzeit und Speicher messen (immer mit eigenem Datenordner, nie dem
normalen), leer oder mit einem großen Testkalender:

```
powershell -ExecutionPolicy Bypass -File tools\measure.ps1 -Exe target\release\hora.exe -Backends auto
$env:HORA_DATA_DIR = "$env:TEMP\hora-gross"; target\debug\hora.exe --fill-test-data
powershell -ExecutionPolicy Bypass -File tools\measure.ps1 -Exe target\release\hora.exe -Backends auto -DataDir $env:TEMP\hora-gross
```

Messung zu Version 1.0 (Release-Build, Software-Renderer, 5 Läufe,
Windows 11 per Remotedesktop). „Start“ ist die Zeit vom Prozessstart bis zur
laufenden Ereignisschleife mit fertig aufgebauter Ansicht, „Speicher“ der
private Arbeitssatz (Spalte „Arbeitsspeicher“ im Task-Manager) nach 10 s
Leerlauf:

| Daten | Start | Speicher |
|---|---|---|
| leer (erster Start, Datenbank wird angelegt) | 69–79 ms | 12,3–12,6 MB |
| Testkalender: 3444 Termine, 60 Serien, 400 Aufgaben, 300 Journaleinträge | 61–69 ms | 18,7–19,0 MB |

Mit dem Testkalender (`--time-views`, Devtools-Release-Build) braucht der
Aufbau einer Ansicht 6–13 ms (Jahr am längsten), Weiterblättern im Monat
6,5 ms, die erste Suche in der Befehlspalette 44 ms (baut den Index), jede
weitere etwa 2,5 ms. Ziel waren unter 1 s und unter 50 MB.

Lizenztexte der Bibliotheken nach Änderungen an `Cargo.lock` neu erzeugen:

```
python tools/third-party-licenses.py
```

## CI und Releases

`.github/workflows/ci.yml` läuft bei jedem Push auf `main` und bei Pull
Requests: unter Windows Formatierung, Clippy (auch mit Devtools), alle
Unit-Tests, Prüfung der Lizenzliste und der Selbsttest der Oberfläche; unter
Linux die plattformunabhängigen Crates und die Integrationstests gegen
Radicale und Baïkal in Docker. Der Ende-zu-Ende-Test (`e2e-radicale.ps1`)
braucht Windows mit Linux-Containern und läuft deshalb nur lokal.

`.github/workflows/release.yml` baut bei einem Tag `v1.2.3` (muss zur Version
in `Cargo.toml` passen) den Installer, testet ihn mit `installer-test.ps1`
und legt ein GitHub-Release mit Installer, Prüfsumme und dem Abschnitt aus
`CHANGELOG.md` an:

```
git tag v1.0.0
git push origin v1.0.0
```

## Aufbau

| Crate | Inhalt |
|---|---|
| `crates/core` | verlustfreier iCalendar-Parser, Zeitzonen, Serien-Expansion, Regeln (`recur`), Serien bearbeiten (`series`), Erinnerungen (`alarm`), Aufgaben (`task`), Journaleinträge (`journal`), Schnelleingabe (`quick`), VTIMEZONE |
| `crates/caldav` | CalDAV-Client: Discovery, Sync, If-Match, MKCALENDAR |
| `crates/storage` | SQLite-Cache mit Migrationen, Änderungs-Warteschlange, gezeigte Erinnerungen, Abfragen für Zeitraum und Suche |
| `crates/platform` | Windows: Anmeldeinformationsspeicher, Benachrichtigungen, Infobereich, Einzelinstanz, Autostart, Links öffnen |
| `crates/app` | Slint-Oberfläche (`ui/*.slint`), Sync-Thread, Ansichten (`overview`: Agenda und Jahr), Bearbeiten, Aufgaben, Journal (`journal`, `journal_ui`), Erinnerungs-Planer, Befehlspalette (`palette`) |
| `installer` | NSIS-Skript für `Hora-Setup-<Version>.exe` |

## Bekannte Grenzen

- Journal: Calinos „+ Mehr“ (Link, Anhänge, Bezug auf Termine am selben Tag)
  fehlt; `URL`, `ATTACH` und `RELATED-TO` anderer Programme bleiben erhalten,
  Hora zeigt sie nur nicht. Kein Export als Markdown-Datei, kein „Rückgängig“
  nach dem Löschen (dafür die Rückfrage).
- Markdown in der Leseansicht: keine Tabellen, Bilder, Aufgabenlisten
  (`- [ ]`) und verschachtelten Zitate; Text, den Slints Markdown nicht
  versteht (z. B. HTML), erscheint als Klartext.
- Wiederkehrende Journaleinträge (`RRULE`, selten) stehen einmal am ersten
  Tag; einzelne Vorkommen zeigt Hora nicht.
- Radicale nimmt nur eine `DESCRIPTION` je `VJOURNAL` an (RFC 5545 erlaubt
  mehrere); Hora schreibt immer nur eine.
- Beispiel-Einträge ohne Konto gehen beim Beenden verloren (wie die
  Beispieltermine); die Fußzeile sagt das.
- Mit acht Reitern wird die Kopfzeile unter 1400 px zum Auswahlmenü für die
  Ansicht; die Tasten bleiben gleich.

- Die Suche der Befehlspalette findet Wörter, die vorkommen, nicht ähnliche
  Schreibweisen („Zahnartz“ findet „Zahnarzt“ nicht); Umlaute und ß müssen
  stimmen. Sie zeigt höchstens 8 Termine, 6 Aufgaben und 6 Journaleinträge,
  ausgeblendete Kalender durchsucht sie nicht.
- Bei Serien zeigt die Suche nur das nächste Vorkommen (innerhalb von zwei
  Jahren, sonst das letzte); einzelne andere Vorkommen findet man über die
  Ansichten.
- Die Agenda zeigt immer einen Monat; fortlaufendes Weiterscrollen in den
  nächsten Monat wie in Calinos Endlosliste gibt es nicht, dafür `.`/`,`.
- Das Jahr zeigt einen Punkt je belegtem Tag, nicht die Anzahl oder die
  Kalenderfarben; Details zeigt ein Klick auf den Tag.
- Bei schmalen Fenstern (unter etwa 1560 px) zeigt die Kopfzeile nur Lupe
  und Kürzel statt „Suchen oder eintragen“ und von der Sync-Anzeige nur den
  Punkt, solange alles in Ordnung ist; Fehler bleiben sichtbar und lassen
  sich anklicken.

- Die Schnelleingabe erkennt feste Muster, keine freie Sprache: „nach der
  Arbeit“, „übernächste Woche“ oder „letzter Freitag im Monat“ bleiben im
  Titel. Was sie nicht versteht, zeigt die Vorschau vor dem Anlegen.
- Datumsangaben wie `12/11` (amerikanisch) werden nicht gelesen, nur `12.11.`,
  `2026-11-12` und Monatsnamen.
- Der Ort nach „in“/„at“ reicht bis zum Ende des übrigen Textes; Uhrzeiten und
  Daten dahinter werden vorher erkannt und stören nicht.

- Wiederkehrende Aufgaben lassen sich in Hora abhaken und bearbeiten, aber
  noch nicht neu anlegen; ihre Fälligkeit gilt für die Serie und ist im
  Dialog gesperrt. Teilaufgaben können nicht wiederkehren (wie bei Calino).
- Eigene Erinnerungen an einzelne Aufgaben (`VALARM`) zeigt und behält Hora,
  bearbeiten lassen sie sich im Aufgaben-Dialog noch nicht; dafür gibt es die
  Erinnerung zur Fälligkeit.
- Keine manuelle Reihenfolge per Ziehen; `X-APPLE-SORT-ORDER` anderer
  Programme wird beim Sortieren beachtet.
- Aufgaben im Kalender lassen sich nicht per Ziehen verschieben.

- Ein Klick auf eine Benachrichtigung öffnet Hora nur, solange es läuft (im
  Infobereich reicht). Schlummern und Schließen gehen immer, die macht Windows.
- Weggeklickte Erinnerungen meldet Hora nicht an den Server zurück; ein
  Smartphone mit demselben Kalender erinnert also trotzdem.
- Erinnerungen nur für sichtbare Kalender; einzelne Kalender stummschalten
  (wie Calinos „Erinnerungen stummschalten“) gibt es noch nicht, ebenso
  keine frei eingetippten Abstände („Benutzerdefiniert …“).
- Der Installer ist nicht signiert (dafür braucht es ein
  Code-Signing-Zertifikat); Windows SmartScreen warnt beim ersten Start
  („Weitere Informationen“ → „Trotzdem ausführen“). Kein automatisches
  Update; ein neuer Installer aktualisiert an Ort und Stelle.
- Nach der Deinstallation bleiben Passwörter im Windows-Anmeldeinformations-
  speicher (Einträge `konto-….Hora`) stehen.

- „Dieser und alle folgenden“ lädt zwei Ressourcen hoch (neue Serie, dann
  gekürzte alte). Hat ein anderes Gerät die alte Serie inzwischen geändert
  und man übernimmt im Konfliktdialog die Server-Version, bleibt die neue
  Serie trotzdem bestehen; ab dem Teilungspunkt gibt es die Termine dann
  doppelt und man löscht eine Hälfte von Hand.
- Wird beim Teilen einer Serie mit fester Anzahl zugleich die Regel geändert,
  gilt die Anzahl im Dialog für den neuen Teil.
- Jährlich nur am Tag des Beginns; Calinos Auswahl „In Monaten“ fehlt noch.
- Neue VTIMEZONE-Blöcke decken Beginnjahr und Folgejahr ab. Hora schreibt
  IANA-Namen (`Europe/Berlin`), die übliche Clients selbst auflösen; ein
  Programm, das nur die VTIMEZONE liest, könnte spätere Jahre falsch lesen.
- Einen bestehenden Termin in einen anderen Kalender verschieben geht noch nicht.
- Termine, die über Mitternacht reichen, lassen sich nur im Dialog ändern,
  nicht per Ziehen; ganztägige nur im Monat per Ziehen.
- Nur HTTP Basic Auth (Baïkal: `dav_auth_type: Basic`), kein Digest.
- Zeitzonen: IANA-Namen, Windows-Namen und `X-LIC-LOCATION`; eigene
  VTIMEZONE-Regeln ohne bekannten Namen werden wie schwebende Zeit behandelt.
- Getestet gegen Radicale 3.9 und Baïkal 0.10.1 (neueres Docker-Image von
  Baïkal gibt es nicht).

## Lizenz

MIT, siehe [LICENSE](LICENSE). Drittanbieter-Hinweise in [NOTICE.md](NOTICE.md).
