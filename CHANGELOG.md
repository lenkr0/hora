# Changelog

## [1.4.0] – 2026-11-14

### Calendars on this PC

- The **+** next to "Calendars" now asks: **On this PC** or **CalDAV
  server**. A calendar on this PC needs only a name and a color; events,
  series, tasks, reminders and journal entries work as usual, just without
  sync and without a backup on a server. With the first own calendar the
  sample events hide.
- Its color dot offers **Edit…** to rename it, change the color or delete it
  (with everything in it, after asking).

### App time

- App time has its own area: the clock in the bar on the left (and `Z` or
  `9`) shows your activities with this week's time and goal next to the
  selected one's total, history and sessions. New activities are created
  there, editing works from there too. Settings only keep the switch.
- A click on an activity in the sidebar opens the area with it.
- The sidebar sections "Calendars" and "App time" fold up with a click on
  their heading; Hora remembers it.

### Easier to use

- Date fields in the event and task dialogs have a calendar button: it opens
  a small month sheet to pick a day with one click. Arrow keys move through
  the days, Page Up/Down changes the month, Enter takes the day, Esc closes.
  Typing the date still works as before.
- A narrow bar on the left switches between calendar, agenda, tasks and
  journal. The switch at the top only holds the calendar's time spans (day
  to year); "Calendar" returns to the one you used last.
- The empty strip at the top of the window is gone: small window buttons sit
  in the top right corner with the time spans right below them, so the tabs
  fit in narrower windows too. A ready update shows as a round button at the
  bottom of the left bar; hovering it shows the version.
- In narrow windows the sync status is just its colored dot; a click shows
  the message.
- Week view: the top corners are cleanly rounded. Narrow events give their
  title the whole line, and those too narrow for any text show just their
  color instead of "…". The current time next to the red line is no longer
  cut off in 12-hour format ("12:54 PM").
- Settings → Appearance chooses light, dark or the same as Windows (until now
  only in the command palette).
- An error message in the event or task dialog disappears as soon as you
  change a field, instead of staying until the next save.

## [1.3.0] – 2026-11-13

### App time

- New in Settings → App time (off by default): Hora counts how long you
  actively use chosen programs. An activity has a name, a color, one or more
  programs (picked from the open ones or entered by file name) and a pause
  limit (default 1 minute).
- Time counts only while one of the activity's programs is in front and there
  is mouse, keyboard or pen input. Without input for the pause limit, counting
  pauses and the waiting time doesn't count; switching to another program or
  locking the screen stops it at once.
- Hora watches the foreground without loading anything into other programs
  and asks for input only while a counted program is in front. With app time
  off, nothing runs at all.
- The running stretch is saved at least once a minute. Settings show this
  week's and the total time per activity; the notification area menu can
  pause counting.
- The sidebar lists the activities with this week's time. A click opens the
  overview: total (and since when), this week, this month, a history of the
  last 12 weeks or months, and the sessions with count, average and longest.
  Stretches less than 15 minutes apart count as one session.
- Weekly goal per activity (optional, in hours). The sidebar shows progress
  and a check mark once it is reached; the overview shows the goal as a line
  in the history and how many weeks in a row you reached it.
- An event can belong to an activity (event dialog, only for calendars on a
  server). Once it has started, it shows the counted time against its length,
  e.g. "1:12 / 1:30 h", with a bar; every session that reaches into the event
  counts. The link applies to all occurrences of a series, also after
  "this and all following", and stays on this PC – the event on the server
  doesn't change.

### Fixes

- Select fields (calendar, reminder, language …), calendars in the sidebar,
  agenda entries and tasks now react to the first click. Before, the first
  click only moved the keyboard focus there and a second click was needed.

## [1.2.0] – 2026-11-10

### Updates

- Hora updates itself: once a day it looks for a new version on GitHub and
  downloads it in the background. When it's ready, the title bar shows
  "Hora 1.2.0 is ready · Restart to update"; a click quits Hora, installs the
  update silently and starts Hora again. Settings → Updates turns the daily
  check off, checks right away and shows the current state.
- Updates are signed: each release carries `update.txt` (version, size and
  SHA-256 of the installer) and `update.sig` (Ed25519 signature). Hora only
  installs a file whose signature matches the key built into it and whose
  size and checksum match; a damaged download is deleted.
- Only a copy installed with Hora Setup updates itself. Installers downloaded
  for an update are removed once Hora runs the new version.
- Hora 1.1.0 and older don't have this yet: install the next version once by
  hand.
- Hora Setup: `--relaunch` starts Hora after a silent installation.
- Releases: the workflow signs the update with the secret `HORA_UPDATE_KEY`
  (`hora-sign`); `tools\update-test.ps1` tests the whole update from the app.

## [1.1.0] – 2026-11-08

### Languages

- Hora now speaks English, German, French, Spanish, Italian, Dutch and Polish.
  English is the default; the language can be changed in Settings → General,
  including "Windows language". Changing it takes effect immediately: menus,
  dialogs, dates, reminders, notifications, the tray menu and the sample data.
- Dates and times follow the Windows region (day/month order, 12 or 24 hours)
  or a fixed format chosen in Settings ("07.11.2026, 14:30",
  "11/07/2026, 2:30 PM", "2026-11-07, 14:30" …).
- Dates are written the way each language writes them: "Monday, 9 November",
  "lundi 9 novembre", "lunes, 9 de noviembre", "9 listopada"; repeat rules too
  ("Monthly on the last Sunday", "Ogni mese la prima domenica",
  "Co miesiąc w ostatnią niedzielę").
- Date fields accept the region's format, ISO dates and words like "today",
  "tomorrow", "morgen" or "demain"; time fields also accept "2pm" or "9h30".
- Server and sign-in error messages are translated as well.
- Translations live in `translations/<language>/LC_MESSAGES/hora-app.po`
  (gettext). Slint and the Rust code share these files; `tools/i18n.py`
  extracts new texts and checks that every language is complete (also in CI).

### New installer

- Hora Setup replaces the NSIS installer: its own window in Hora's look
  (colors, Newsreader type, light and dark), with language selection on the
  first page, install location, "Start with Windows" and desktop shortcut, an
  animated progress bar and a finish page that can launch Hora.
- The language chosen in the installer becomes Hora's language at the next
  start, even if another language was set before.
- Updating detects the existing installation, closes a running Hora, keeps
  the location and the previous choices.
- Removing uses the same window (Settings → Apps → Hora → Uninstall) and can
  also delete the local data; the CalDAV server is never touched.
- Still per user and without administrator rights; silent installs and
  uninstalls work as before (`/S`, `/D=<folder>`, `_?=<folder>`), plus
  `--lang=<code>`, `--no-autostart` and `--desktop-shortcut`.
- Built with `tools\build-installer.ps1` (no NSIS needed): the setup program
  carries the files xz-compressed (LZMA2 with x86 filter).

### Own title bar

- Hora and Hora Setup draw their own title bar in the app's colors instead
  of the standard Windows one: minimize, maximize/restore and close in the
  Windows 11 style, drag anywhere in the top bar or on free header space,
  double-click to maximize, resize at the window edges. Windows still adds
  rounded corners, the shadow and snapping to screen edges.

### Smaller

- `hora.exe` shrank from 31.5 to 23.4 MB: Hora's own code (including the UI
  generated from the .slint files) is optimized for size, release builds use
  full LTO, and the unused OpenGL renderer femtovg is no longer built in (it
  remains available as the cargo feature `femtovg`). Startup and memory are
  unchanged (about 57 ms to a ready window, 12.4 MB).
- The installer packs with xz (LZMA2 with x86 filter) and is 15.5 MB.

### Other changes

- The database commits without waiting for the disk (`synchronous=NORMAL`,
  recommended with WAL) and waits up to 15 s for a lock. Saving while a
  large sync is running no longer risks "database is locked".
- The view tabs size themselves to their labels, so longer translations fit.
- The tray menu updates when the language changes.
- Version info of `hora.exe` is now in English ("Hora Calendar").
- The hour label under the red "now" marker in the week view is hidden, so
  the two no longer overlap.
- `HORA_FORMATS` (like `HORA_LANGUAGE`) fixes the date and time format for
  tests and screenshots.
- README in English with screenshots, German version in `README.de.md`.

## [1.0.0] – 2026-11-06

First complete version: all planned features, installer, CI.

- Calendars: CalDAV accounts (Baïkal, Radicale; Basic Auth, HTTP and HTTPS),
  sync via sync-token or ctag/ETag, offline queue with a conflict dialog,
  calendar colors, show and hide.
- Views: day, 3 days, week, month, year, agenda, tasks, journal; mini
  calendar, command palette (`Ctrl+K`) with search, jump to date, commands and
  creating from a sentence. Light and dark following Windows, German (24 h,
  Monday, Europe/Berlin), fully usable from the keyboard.
- Create, edit, delete, drag and resize events; series with "Only this / This
  and all following / All".
- Reminders (VALARM) as Windows notifications, also from the tray.
- Tasks (VTODO) with due dates, priority, subtasks and recurring tasks;
  journal (VJOURNAL) with Markdown and tags.
- Quick add in German and English.

New since 0.9.0:

- New name: Hora (working title was Pult). Program `hora.exe`, data in
  `%LOCALAPPDATA%\Hora`.
- Crashes and errors that would otherwise go unnoticed are written to
  `%LOCALAPPDATA%\Hora\hora.log`; severe errors at startup show a message box.
- If the database can't be opened, Hora quits with a message instead of
  carrying on with an empty in-memory database (anything entered there would
  have been lost on exit).
- License texts of all Rust libraries in `THIRD_PARTY_LICENSES/Rust-Crates.txt`
  (also in the installer).
- GitHub Actions: CI (format, Clippy, tests, UI self-test on Windows;
  Radicale and Baïkal on Linux) and release (build, test and publish the
  installer with a checksum).
- Measuring: `tools\measure.ps1` always uses its own data folder,
  `--fill-test-data` creates a large test calendar, `--time-views` times views
  and search.

## [0.9.0] – 2026-11-06

- Journal (VJOURNAL) like Calino's: list by day, editor with date, title,
  tags, calendar; write and read mode (Markdown), `Ctrl+B` / `Ctrl+I`,
  autosave; book icon in the month view; search in the palette; "Create
  journal calendar" when no calendar accepts journal entries.
- Sync also covers pure task lists and journal calendars; they appear in the
  sidebar.
- Queue: saving during an upload no longer loses a newer version; a resource
  deleted or moved during its first upload is then deleted on the server too.
- Narrow windows: view selection as a menu instead of eight tabs.

## [0.8.0] – 2026-11-06

- Agenda and year view, command palette with search across events and tasks,
  theme switching; compact header, sync message on click.

## [0.7.0]

- Quick add in German and English ("Math every Monday 8am to 9:30 room 204"),
  also for tasks.

## [0.6.0]

- Tasks (VTODO): own view, subtasks, priority, recurring tasks, due date
  reminders, tasks in the calendar.

## [0.5.0]

- Reminders as Windows notifications, tray icon, autostart, settings,
  installer.

## [0.4.0]

- Editing recurring events: only this, this and all following, all.

## [0.3.0]

- Dragging and resizing with mouse and keyboard.

## [0.2.0]

- Month, week and day; create, edit and delete events; offline queue and
  conflicts.

## [0.1.0]

- CalDAV accounts and calendars, sync, design based on Calino.
