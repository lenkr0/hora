# Changelog

## [Unreleased]

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

### Other changes

- The view tabs size themselves to their labels, so longer translations fit.
- The tray menu updates when the language changes.
- Version info of `hora.exe` is now in English ("Hora Calendar").

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
