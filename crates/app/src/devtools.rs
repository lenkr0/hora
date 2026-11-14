//! Entwicklerwerkzeuge (Feature `devtools`), nicht im normalen Build.
//!
//! - `--size 1280x840` setzt die Fenstergröße (logische Pixel).
//! - `--snapshot bild.png` speichert ein Bild des Fensters und beendet die App.
//! - `--selftest` spielt Ziehen, Dauer ändern, Pfeiltasten und Abbrechen mit
//!   synthetischen Eingaben durch, prüft das Ergebnis am Modell und beendet die
//!   App mit Exitcode 0 (alles bestanden) oder 1.
//! - `--connect <adresse> <benutzer> <passwort>` richtet ein Konto über
//!   denselben Weg ein wie der Dialog, wartet auf den Sync und macht dann
//!   den Screenshot (nur für Testserver gedacht). Mit `--edit-test` legt es
//!   danach über die Oberflächen-Callbacks Termine und Serien an, verschiebt
//!   und löscht welche (bei Serien mit "nur dieser / folgende / alle") und
//!   wartet, bis alles hochgeladen ist.
//! - `--view <0–7>` wählt Tag, 3 Tage, Woche, Monat, Jahr, Agenda, Aufgaben
//!   oder Journal; `--journal <titel>` bzw. `--journal-write <titel>` öffnet
//!   einen Journaleintrag zum Lesen bzw. Schreiben.
//! - `--palette <text>` öffnet die Befehlspalette mit diesem Text,
//!   `--sync-popup` die ganze Sync-Meldung.
//! - `--dialog`, `--about`, `--new-event`, `--scope` öffnen vor dem
//!   Screenshot den Kontodialog, den Über-Dialog, den Dialog für einen neuen
//!   Termin bzw. den Serien-Dialog; `--preview <titel>` zeigt die Vorschau,
//!   `--edit <titel>` den Bearbeiten-Dialog eines Termins, `--edit-task <titel>`
//!   den einer Aufgabe; `--settings` die Einstellungen; `--quick <text>` die
//!   Schnelleingabe mit diesem Text, `--quick-dialog <text>` gleich den
//!   daraus vorbefüllten Dialog (wie Umschalt+Eingabe).
//! - `--update-ready <version>` täuscht ein geladenes Update vor (Hinweis in
//!   der Titelleiste, Zeile in den Einstellungen), für Bildschirmfotos.
//! - `--reminder-check` (mit `--connect`): fällige Erinnerungen sofort zeigen,
//!   im Infocenter nachsehen und die Test-Toasts wieder entfernen.
//! - `--fill-test-data` legt im Datenordner (`HORA_DATA_DIR`) ein Testkonto
//!   ohne erreichbaren Server mit drei Kalendern an: drei Jahre Termine (gut
//!   4000, dazu 60 Serien), 400 Aufgaben und 300 Journaleinträge; für die
//!   Messung mit vielen Daten. Beendet sich danach.
//! - `--time-views` misst, wie lange jede Ansicht, Blättern und eine Suche in
//!   der Befehlspalette brauchen (Mittel aus 5 Durchläufen), und beendet sich.
//! - `--activities-demo` schaltet die App-Zeiten ein und legt, wenn es noch
//!   keine gibt, zwei Beispiel-Aktivitäten mit gut drei Monaten gezählter Zeit
//!   an (nur mit eigenem `HORA_DATA_DIR`); `--activity-dialog` öffnet den
//!   Dialog für eine neue Aktivität, `--activity-overview` den Bereich
//!   App-Zeiten mit der ersten Aktivität (mit `--months` den Verlauf nach
//!   Monaten). `--activity-plan` legt dazu ein Testkonto
//!   ohne erreichbaren Server mit einer Serie Mo/Mi/Fr 9–10:30 an, die zur
//!   ersten Aktivität gehört, samt gezählter Zeit in dieser Woche.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Model};

use crate::{AppWindow, EventItem, State};

pub fn install(app: &AppWindow, state: Rc<RefCell<State>>) {
    let args: Vec<String> = std::env::args().collect();
    let value_of = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--fill-test-data") {
        let result = fill_test_data(&mut state.borrow_mut().store);
        match result {
            Ok(counts) => println!("testdaten: {counts}"),
            Err(e) => println!("testdaten fehlgeschlagen: {e}"),
        }
        std::process::exit(0);
    }

    if let Some((w, h)) = value_of("--size").and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))
    }) {
        app.window().set_size(LogicalSize::new(w, h));
    }
    if let Some(view) = value_of("--view").and_then(|v| v.parse::<i32>().ok()) {
        app.invoke_set_view(view);
    }
    if let Some(version) = value_of("--update-ready").and_then(|v| hora_update::Version::parse(&v))
    {
        let mut s = state.borrow_mut();
        s.updates.status = crate::updates::Status::Ready {
            manifest: hora_update::Manifest {
                version,
                file: hora_update::setup_name(version),
                size: 1,
                sha256: [0; 32],
            },
            path: std::path::PathBuf::new(),
        };
        crate::updates::show(app, &s);
    }
    // Journaleintrag mit diesem Titel zeigen (lesen bzw. schreiben)
    if let Some(title) = value_of("--journal").or_else(|| value_of("--journal-write")) {
        let write = has("--journal-write");
        let mut s = state.borrow_mut();
        let found = crate::journal::collect(&mut s)
            .into_iter()
            .find(|e| e.info.summary.contains(&title));
        if let Some(entry) = found {
            crate::journal_ui::show(app, &mut s, &entry.key(), entry.day);
            crate::journal_ui::set_write(app, &mut s, write);
        }
    }

    if has("--time-views") {
        let weak = app.as_weak();
        slint::Timer::single_shot(Duration::from_millis(300), move || {
            if let Some(app) = weak.upgrade() {
                time_views(&app);
            }
            let _ = slint::quit_event_loop();
        });
    }

    let snapshot = value_of("--snapshot");
    if has("--dialog") {
        app.invoke_show_add_account();
    }
    if let Some(text) = value_of("--quick-dialog") {
        app.invoke_show_quick();
        app.invoke_quick_submit(text.into(), true);
    }
    if let Some(text) = value_of("--quick") {
        app.invoke_show_quick();
        app.set_quick_text(text.into());
    }
    if has("--activities-demo") {
        activities_demo(&mut state.borrow_mut());
        app.set_settings_activities(true);
    }
    if has("--activity-plan") {
        activity_plan(app, &mut state.borrow_mut());
    }
    if has("--settings") {
        app.invoke_show_settings();
    }
    if has("--activity-dialog") {
        crate::activity_ui::open_new(app, &mut state.borrow_mut());
    }
    if has("--activity-overview") {
        app.set_activity_overview_months(has("--months"));
        let first = state.borrow().store.activities().unwrap_or_default();
        if let Some(activity) = first.first() {
            crate::activity_overview::open(app, &mut state.borrow_mut(), activity.id as i32);
        }
    }
    if has("--sync-popup") {
        let weak = app.as_weak();
        slint::Timer::single_shot(Duration::from_millis(500), move || {
            if let Some(app) = weak.upgrade() {
                app.invoke_show_sync_popup();
            }
        });
    }
    if let Some(text) = value_of("--palette") {
        app.invoke_show_palette();
        app.set_palette_text(text.as_str().into());
        app.invoke_palette_query(text.into());
    }
    if has("--about") {
        app.set_about_open(true);
    }
    if has("--new-event") {
        app.invoke_new_event_at(2, 10 * 60);
    }
    if let Some(title) = value_of("--preview") {
        let weak = app.as_weak();
        let state = state.clone();
        slint::Timer::single_shot(Duration::from_millis(400), move || {
            let Some(app) = weak.upgrade() else { return };
            let row = state
                .borrow()
                .rows
                .iter()
                .find(|it| it.title == title.as_str());
            if let Some(it) = row {
                let col = app.get_column_width();
                let x = app.get_days_abs_x() + col * it.day as f32;
                let y = app.get_days_abs_y() + it.start_min as f32;
                app.invoke_open_event(it.id, x, y, col * 0.96);
            }
        });
    }
    if let Some(title) = value_of("--edit-task") {
        let weak = app.as_weak();
        slint::Timer::single_shot(Duration::from_millis(400), move || {
            let Some(app) = weak.upgrade() else { return };
            app.invoke_set_view(6);
            let row = app
                .get_task_rows()
                .iter()
                .find(|r| !r.header && r.title == title.as_str());
            if let Some(r) = row {
                app.invoke_task_open(r.id);
            }
        });
    }
    if let Some(title) = value_of("--edit") {
        let weak = app.as_weak();
        let state = state.clone();
        slint::Timer::single_shot(Duration::from_millis(400), move || {
            let Some(app) = weak.upgrade() else { return };
            let row = state
                .borrow()
                .rows
                .iter()
                .find(|it| it.title == title.as_str());
            if let Some(it) = row {
                app.invoke_edit_event(it.id);
            }
        });
    }
    if has("--scope") {
        app.set_scope_allow_this(true);
        app.set_scope_open(true);
    }
    if let Some(i) = args.iter().position(|a| a == "--connect") {
        let [url, user, password] = [1, 2, 3].map(|n| args.get(i + n).cloned().unwrap_or_default());
        connect_and_snapshot(
            app,
            state,
            url,
            user,
            password,
            snapshot,
            has("--edit-test"),
            has("--reminder-check"),
        );
    } else if has("--selftest") {
        Selftest::start(app, state, snapshot);
    } else if let Some(path) = snapshot {
        let weak = app.as_weak();
        slint::Timer::single_shot(Duration::from_millis(900), move || {
            if let Some(app) = weak.upgrade() {
                save_snapshot(&app, &path);
            }
            let _ = slint::quit_event_loop();
        });
    }
}

/// Ein "anderes Gerät" ändert den Termin `title` direkt auf dem Server, damit
/// Horas nächste Änderung auf einen Konflikt (412) läuft.
fn change_on_server(state: &Rc<RefCell<State>>, title: &str, user: &str, password: &str) {
    let s = state.borrow();
    let Some(crate::items::Source::Server {
        calendar_id, href, ..
    }) = s
        .items
        .iter()
        .find(|i| i.title == title)
        .map(|i| i.source.clone())
    else {
        println!("konflikt: {title} nicht gefunden");
        return;
    };
    let Some(calendar) = s.calendars.iter().find(|c| c.id == calendar_id) else {
        return;
    };
    let Ok(url) = url::Url::parse(&calendar.href).and_then(|c| c.join(&href)) else {
        return;
    };
    let client = hora_caldav::Client::new(user, password);
    let result = client.get(&url).and_then(|(ics, etag)| {
        let changed = ics.replace("SUMMARY:Zahnarzt", "SUMMARY:Zahnarzt (anderes Gerät)");
        client.put(
            &url,
            &changed,
            hora_caldav::Precondition::Match(etag.as_deref().unwrap_or("*")),
        )
    });
    println!(
        "konflikt: anderes Gerät ändert {title}: {}",
        if result.is_ok() {
            "ok"
        } else {
            "fehlgeschlagen"
        }
    );
}

/// Bearbeiten über dieselben Callbacks wie die Oberfläche.
fn run_edits(app: &AppWindow, state: &Rc<RefCell<State>>, user: &str, password: &str) {
    // Termin nach Titel und Wochentag (0 = Montag; -1 = beliebig)
    let find_on = |title: &str, day: i32| {
        state
            .borrow()
            .rows
            .iter()
            .find(|it| it.title.as_str() == title && (day < 0 || it.day == day))
    };
    let find = |title: &str| find_on(title, -1);
    change_on_server(state, "Zahnarzt", user, password);

    // 1. Neuer Termin am Mittwoch 14–15 Uhr im voreingestellten Kalender
    app.invoke_new_event_at(2, 14 * 60);
    let draft = app.get_event_draft();
    app.invoke_save_event(
        "Neu aus Hora".into(),
        false,
        draft.start_date.clone(),
        "14:00".into(),
        draft.end_date.clone(),
        "15:00".into(),
        "Raum 7".into(),
        "Angelegt im Ende-zu-Ende-Test".into(),
        draft.calendar,
        draft.rule.clone(),
    );
    println!(
        "bearbeiten: neu angelegt, Dialogfehler: '{}'",
        app.get_event_error()
    );

    // 2. Zahnarzt eine Stunde später
    match find("Zahnarzt") {
        Some(it) => {
            app.invoke_change_event(it.id, it.day, it.start_min + 60, it.dur_min);
            println!("bearbeiten: Zahnarzt verschoben");
        }
        None => println!("bearbeiten: Zahnarzt nicht gefunden"),
    }

    // 3. Projekt B löschen (mit Bestätigung)
    match find("Projekt B") {
        Some(it) => {
            app.invoke_delete_event(it.id);
            app.invoke_confirm_accepted();
            println!("bearbeiten: Projekt B gelöscht");
        }
        None => println!("bearbeiten: Projekt B nicht gefunden"),
    }

    // 4. Serie umbenennen: alle Termine (die verschobene Ausnahme behält ihren Titel)
    match find_on("Standup", 0) {
        Some(it) => {
            app.invoke_edit_event(it.id);
            let d = app.get_event_draft();
            app.invoke_save_event(
                "Team-Standup".into(),
                d.all_day,
                d.start_date.clone(),
                d.start_time.clone(),
                d.end_date.clone(),
                d.end_time.clone(),
                d.location.clone(),
                d.description.clone(),
                d.calendar,
                d.rule.clone(),
            );
            let asked = app.get_scope_open() && app.get_scope_allow_this();
            app.set_scope_open(false);
            app.invoke_scope_chosen(2);
            println!(
                "bearbeiten: Serie umbenannt (Serien-Dialog: {asked}), Dialogfehler: '{}'",
                app.get_event_error()
            );
        }
        None => println!("bearbeiten: Standup nicht gefunden"),
    }

    // 5. Dienstag: nur diesen Termin löschen
    match find_on("Team-Standup", 1) {
        Some(it) => {
            app.invoke_delete_event(it.id);
            let asked = app.get_scope_open() && app.get_scope_deleting();
            app.set_scope_open(false);
            app.invoke_scope_chosen(0);
            println!("bearbeiten: Standup am Dienstag gelöscht (Serien-Dialog: {asked})");
        }
        None => println!("bearbeiten: Team-Standup am Dienstag nicht gefunden"),
    }

    // 6. Freitag: dieser und alle folgenden eine halbe Stunde später
    match find_on("Team-Standup", 4) {
        Some(it) => {
            app.invoke_change_event(it.id, it.day, it.start_min + 30, it.dur_min);
            let asked = app.get_scope_open();
            app.set_scope_open(false);
            app.invoke_scope_chosen(1);
            println!("bearbeiten: Standup ab Freitag verschoben (Serien-Dialog: {asked})");
        }
        None => println!("bearbeiten: Team-Standup am Freitag nicht gefunden"),
    }

    // 7. Mathe diese Woche: nur dieser Termin eine Stunde später
    match find_on("Mathe", 0) {
        Some(it) => {
            app.invoke_change_event(it.id, it.day, it.start_min + 60, it.dur_min);
            let asked = app.get_scope_open();
            app.set_scope_open(false);
            app.invoke_scope_chosen(0);
            println!("bearbeiten: Mathe nur diese Woche verschoben (Serien-Dialog: {asked})");
        }
        None => println!("bearbeiten: Mathe nicht gefunden"),
    }

    // 8. Neue Serie über den Dialog: Lauftreff dienstags und donnerstags, 4-mal
    app.invoke_new_event_at(1, 18 * 60);
    let draft = app.get_event_draft();
    let mut rule = draft.rule.clone();
    rule.repeat = 2;
    rule.weekdays = 0b1010;
    rule.end = 2;
    rule.count = "4".into();
    app.invoke_save_event(
        "Lauftreff".into(),
        false,
        draft.start_date.clone(),
        "18:00".into(),
        draft.end_date.clone(),
        "19:00".into(),
        "Park".into(),
        "".into(),
        draft.calendar,
        rule,
    );
    println!(
        "bearbeiten: Serie angelegt, Dialogfehler: '{}'",
        app.get_event_error()
    );

    run_task_edits(app);
    run_journal_steps(app, state);
}

/// Aufgaben über die Callbacks der Aufgaben-Ansicht bearbeiten
fn run_task_edits(app: &AppWindow) {
    app.invoke_set_view(6);
    let find = |title: &str| {
        app.get_task_rows()
            .iter()
            .find(|r| !r.header && r.title.as_str() == title)
            .map(|r| r.id)
    };
    let toggle = |title: &str| match find(title) {
        Some(id) => {
            app.invoke_task_toggle(id);
            println!("aufgaben: {title} abgehakt");
        }
        None => println!("aufgaben: {title} nicht gefunden"),
    };
    println!(
        "aufgaben: {} aktiv, {} erledigt",
        app.get_task_active_count(),
        app.get_task_done_count()
    );
    app.invoke_task_quick_add("Milch kaufen".into());
    toggle("Steuererklärung");
    toggle("Sport");
    toggle("Umzug planen");

    // Neue Aufgabe mit Fälligkeit und Priorität, dann eine Teilaufgabe dazu
    app.invoke_task_new();
    let draft = app.get_task_draft();
    // Im Zahlenformat der Region, wie es ein Nutzer eintippen würde
    let due =
        crate::forms::format_date(chrono::Local::now().date_naive() + chrono::Duration::days(8));
    app.invoke_task_save(
        "Bericht schreiben".into(),
        2,
        due.into(),
        "18:00".into(),
        "".into(),
        1,
        draft.calendar,
        0,
        "Für die Abteilung".into(),
    );
    println!(
        "aufgaben: Bericht angelegt, Fehler: '{}'",
        app.get_task_error()
    );
    match find("Bericht schreiben") {
        Some(id) => {
            app.invoke_task_open(id);
            app.invoke_task_add_subtask();
            let draft = app.get_task_draft();
            app.invoke_task_save(
                "Gliederung".into(),
                0,
                "".into(),
                "18:00".into(),
                "".into(),
                0,
                draft.calendar,
                draft.parent,
                "".into(),
            );
            println!(
                "aufgaben: Teilaufgabe angelegt (übergeordnet: {}), Fehler: '{}'",
                draft.parent,
                app.get_task_error()
            );
        }
        None => println!("aufgaben: Bericht nicht gefunden"),
    }
    println!(
        "aufgaben: {} aktiv, {} erledigt",
        app.get_task_active_count(),
        app.get_task_done_count()
    );
    app.invoke_set_view(2);

    // Schnelleingabe: Termine und Aufgaben aus einem Satz
    for text in [
        "Physik jeden Montag 8 Uhr bis 9:30 Raum 204",
        "Optiker morgen 15 Uhr bei Dr. Weber",
        "todo Geschenk kaufen #Arbeit",
    ] {
        app.invoke_show_quick();
        app.invoke_quick_submit(text.into(), false);
        println!(
            "schnelleingabe: {text} (Fehler: '{}')",
            app.get_quick_error()
        );
        app.set_quick_open(false);
    }
    app.invoke_task_quick_add("Unterlagen sortieren bis Freitag".into());
    println!("schnelleingabe: Aufgabenzeile mit Fälligkeit");
    app.invoke_set_view(2);

    run_palette_steps(app);
}

/// Journal gegen den Server: fremden Eintrag lesen und ändern, neu anlegen,
/// schnell hintereinander speichern (während hochgeladen wird), in einen
/// anderen Kalender verschieben, löschen, suchen und einen Journal-Kalender
/// über den Knopf anlegen.
fn run_journal_steps(app: &AppWindow, state: &Rc<RefCell<State>>) {
    let calendar = |name: &str| {
        state
            .borrow()
            .calendars
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.id)
    };
    let select = |title: &str| {
        let row = app
            .get_journal_rows()
            .iter()
            .find(|r| r.title.as_str() == title)
            .map(|r| r.id);
        if let Some(id) = row {
            app.invoke_journal_select(id);
        }
        row.is_some()
    };
    // Tippen wie im Editor, dann speichern (Lesen speichert sofort)
    let write = |title: &str, body: &str| {
        app.invoke_journal_set_write(true);
        app.set_journal_title(title.into());
        app.set_journal_body(body.into());
        app.invoke_journal_edited();
        app.invoke_journal_set_write(false);
    };
    app.invoke_set_view(7);
    app.invoke_journal_set_mode(1);
    println!(
        "journal: {} einträge, kalender {}",
        app.get_journal_count(),
        state
            .borrow()
            .calendars
            .iter()
            .filter(|c| c.supports(hora_core::model::Kind::Journal))
            .map(|c| c.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // 1. Eintrag eines anderen Programms: lesen, Titel ändern
    if select("Notiz von woanders") {
        println!(
            "journal: fremd gelesen '{}' tags {:?}",
            app.get_journal_body().replace('\n', "|"),
            app.get_journal_tags()
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
        );
        let body = app.get_journal_body().to_string();
        write("Notiz von woanders (bearbeitet)", &body);
    } else {
        println!("journal: fremder eintrag fehlt");
    }

    // 2. Neuer Eintrag mit Tag im voreingestellten Journal-Kalender
    app.invoke_journal_new();
    write("Erster Schnee", "**Kalt**, aber schön");
    app.invoke_journal_add_tag("Winter".into());
    println!(
        "journal: neu '{}' ({})",
        app.get_journal_title(),
        app.get_journal_status_text()
    );

    // 3. Schnell hintereinander speichern, während der Sync-Thread hochlädt
    app.invoke_journal_new();
    let mut text = String::new();
    for i in 1..=6 {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&i.to_string());
        write("Tippen beim Hochladen", &text);
        std::thread::sleep(Duration::from_millis(35));
    }
    println!("journal: sechsmal gespeichert, zuletzt '{text}'");

    // 4. In den Kalender Arbeit verschieben
    if let Some(arbeit) = calendar("Arbeit")
        && select("Erster Schnee")
    {
        app.invoke_journal_set_calendar(arbeit as i32);
        let chosen = app
            .get_journal_calendars()
            .iter()
            .find(|c| c.selected)
            .map(|c| c.name.to_string());
        println!("journal: verschoben nach {chosen:?}");
    }

    // 5. Anlegen und wieder löschen (mit Rückfrage)
    app.invoke_journal_new();
    write("Wird gelöscht", "kurz da");
    app.invoke_journal_delete();
    let asked = app.get_confirm_open();
    app.invoke_confirm_accepted();
    println!("journal: gelöscht (rückfrage {asked})");

    // 6. Befehlspalette findet den Eintrag über ein Wort im Text
    app.invoke_show_palette();
    app.set_palette_text("kalt schön".into());
    app.invoke_palette_query("kalt schön".into());
    let row = app
        .get_palette_items()
        .iter()
        .position(|r| !r.header && r.title.as_str() == "Erster Schnee");
    match row {
        Some(row) => {
            app.invoke_palette_run(row as i32);
            println!(
                "palette: journal '{}' (ansicht {})",
                app.get_journal_title(),
                app.get_view()
            );
        }
        None => println!("palette: journal ohne treffer"),
    }

    // 7. Journal-Kalender über den Knopf anlegen (Sync-Thread)
    app.invoke_journal_create_calendar();
    println!("journal: kalender wird angelegt");
    app.invoke_set_view(2);
}

/// Befehlspalette: Suche, Sprung zu einem Datum, Anlegen per Satz
fn run_palette_steps(app: &AppWindow) {
    let query = |text: &str| {
        app.invoke_show_palette();
        app.set_palette_text(text.into());
        app.invoke_palette_query(text.into());
        app.get_palette_items()
    };
    let row_of = |items: &slint::ModelRc<crate::PaletteItem>,
                  pick: &dyn Fn(&crate::PaletteItem) -> bool| {
        items
            .iter()
            .position(|r| !r.header && pick(&r))
            .map(|i| i as i32)
    };

    let items = query("lauftreff park");
    match row_of(&items, &|r| r.title == "Lauftreff") {
        Some(row) => {
            app.invoke_palette_run(row);
            println!(
                "palette: suche gefunden, vorschau '{}' ({})",
                app.get_preview_details().title,
                app.get_month_title()
            );
        }
        None => println!("palette: suche ohne treffer"),
    }

    let items = query("24.12.");
    match row_of(&items, &|r| r.title.starts_with("Zu ")) {
        Some(row) => {
            app.invoke_set_view(3);
            app.invoke_palette_run(row);
            println!("palette: sprung nach '{}'", app.get_month_title());
        }
        None => println!("palette: kein sprung"),
    }

    let items = query("Wochenans");
    match row_of(&items, &|r| r.title == "Wochenansicht") {
        Some(row) => {
            app.invoke_palette_run(row);
            println!("palette: befehl ansicht {}", app.get_view());
        }
        None => println!("palette: befehl nicht gefunden"),
    }

    let text = "Elternabend übermorgen 19 Uhr in der Aula";
    let items = query(text);
    match row_of(&items, &|r| r.title.starts_with("Erstellen: ")) {
        Some(row) => {
            let title = items
                .row_data(row as usize)
                .map(|r| r.title)
                .unwrap_or_default();
            app.invoke_palette_run(row);
            println!(
                "palette: '{title}' angelegt (offen: {})",
                app.get_palette_open()
            );
        }
        None => println!("palette: kein Erstellen-Eintrag"),
    }
    app.invoke_go_today();
}

/// Konto einrichten wie über den Dialog, auf das Ende des Syncs warten
/// (oder auf eine Fehlermeldung), Ergebnis ausgeben und Screenshot machen.
#[allow(clippy::too_many_arguments)]
fn connect_and_snapshot(
    app: &AppWindow,
    state: Rc<RefCell<State>>,
    url: String,
    user: String,
    password: String,
    snapshot: Option<String>,
    edit_test: bool,
    reminder_check: bool,
) {
    let weak = app.as_weak();
    slint::Timer::single_shot(Duration::from_millis(300), move || {
        let Some(app) = weak.upgrade() else { return };
        // Mit Konto: Sync beim Start; ohne: über den Dialogweg verbinden
        if !app.get_has_accounts() {
            app.invoke_show_add_account();
            app.invoke_connect(
                "".into(),
                url.into(),
                user.as_str().into(),
                password.as_str().into(),
            );
        }
        let started = std::time::Instant::now();
        let edited = std::cell::Cell::new(false);
        let resolved = std::cell::Cell::new(false);
        let timer = Rc::new(slint::Timer::default());
        let keep = timer.clone();
        let weak = app.as_weak();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(200),
            move || {
                let _keep_alive = &keep;
                let Some(app) = weak.upgrade() else { return };
                let failed = !app.get_add_account_error().is_empty();
                // Erst fertig, wenn ein Sync gelaufen ist (nicht "Noch nicht synchronisiert")
                let synced = app.get_has_accounts()
                    && (app.get_sync_state() == 3
                        || app.get_sync_text().starts_with("Synchronisiert "));
                let timed_out = started.elapsed() > Duration::from_secs(60);
                if edit_test && synced && !edited.get() && !timed_out {
                    edited.set(true);
                    run_edits(&app, &state, &user, &password);
                    return;
                }
                // Konfliktdialog wie ein Nutzer beantworten: eigene Version behalten
                if edit_test && app.get_conflict_open() {
                    println!(
                        "konflikt: Dialog erschienen: {}",
                        app.get_conflict_message()
                    );
                    app.set_conflict_open(false);
                    app.invoke_conflict_keep_mine();
                    resolved.set(true);
                    return;
                }
                let uploaded = {
                    let s = state.borrow();
                    !edit_test
                        || (s.pending.waiting == 0
                            && s.pending.conflicts == 0
                            && !s.sync.running
                            && !s.journal_create_busy)
                };
                if !(failed || (synced && uploaded) || timed_out) {
                    return;
                }
                println!("konto-fehler: {}", app.get_add_account_error());
                println!(
                    "sync-status: {} {}",
                    app.get_sync_state(),
                    app.get_sync_text()
                );
                for row in app.get_calendars().iter() {
                    println!(
                        "kalender: {} / {} sichtbar={}",
                        row.account, row.name, row.visible
                    );
                }
                println!("termine in der woche: {}", app.get_events().row_count());
                if edit_test {
                    println!(
                        "konflikt aufgelöst: {}",
                        if resolved.get() {
                            "ja, eigene Version behalten"
                        } else {
                            "kein Konflikt aufgetreten"
                        }
                    );
                }
                let pending = state.borrow().pending;
                println!(
                    "warteschlange: {} wartend, {} konflikte, {} abgelehnt",
                    pending.waiting, pending.conflicts, pending.failed
                );
                if reminder_check {
                    // Fällige Erinnerungen jetzt zeigen, im Infocenter nachsehen, wieder entfernen
                    crate::check_reminders(&mut state.borrow_mut());
                    std::thread::sleep(Duration::from_millis(700));
                    let s = state.borrow();
                    let shown = &s.shown_toasts;
                    if let Some(notifier) = s.shell.as_ref().and_then(|sh| sh.notifier.as_ref()) {
                        let delivered = notifier.delivered();
                        let found = shown.iter().filter(|(t, _)| delivered.contains(t)).count();
                        println!(
                            "erinnerungen gezeigt: {}, im infocenter: {found}",
                            shown.len()
                        );
                        for (tag, title) in shown {
                            println!("erinnerung: {title}");
                            notifier.remove(tag);
                        }
                    } else {
                        println!("erinnerungen: keine Benachrichtigungen verfügbar");
                    }
                }
                if let Some(path) = &snapshot {
                    save_snapshot(&app, path);
                }
                let _ = slint::quit_event_loop();
            },
        );
    });
}

/// Dauer der Ansichten und der Suche (siehe `--time-views`)
fn time_views(app: &AppWindow) {
    let names = [
        "Tag", "3 Tage", "Woche", "Monat", "Jahr", "Agenda", "Aufgaben", "Journal",
    ];
    let time = |f: &dyn Fn()| {
        let started = std::time::Instant::now();
        for _ in 0..5 {
            f();
        }
        started.elapsed().as_secs_f64() * 1000.0 / 5.0
    };
    for (view, name) in names.iter().enumerate() {
        let ms = time(&|| {
            app.invoke_set_view(2);
            app.invoke_set_view(view as i32);
        });
        println!("ansicht {name}: {ms:.1} ms (mit Wechsel aus der Woche)");
    }
    app.invoke_set_view(3);
    let ms = time(&|| app.invoke_next());
    println!("monat weiter: {ms:.1} ms");
    app.invoke_go_today();
    app.invoke_set_view(2);
    for text in ["besprechung", "tagebuch kapitel", "aufgabe 12"] {
        // Erste Suche baut den Index auf, danach nur suchen
        let first = time(&|| {
            app.invoke_palette_query("".into());
        });
        let _ = first;
        let started = std::time::Instant::now();
        app.invoke_palette_query(text.into());
        let cold = started.elapsed().as_secs_f64() * 1000.0;
        let warm = time(&|| app.invoke_palette_query(text.into()));
        println!(
            "suche '{text}': {} treffer, {cold:.1} ms (erste), {warm:.1} ms (danach)",
            app.get_palette_items().iter().filter(|r| !r.header).count()
        );
    }
}

/// Große Testdaten für die Messung (siehe `--fill-test-data`)
/// Zwei Beispiel-Aktivitäten mit Zeit in dieser und der letzten Woche
fn activities_demo(state: &mut State) {
    use chrono::{TimeDelta, Utc};
    state.settings.activities = true;
    state.settings.save(&state.store);
    if !state.store.activities().unwrap_or_default().is_empty() {
        return;
    }
    // Name, Farbe, Programme, typische Minuten am Tag, Wochenziel in Minuten
    let examples: [(&str, &str, &[&str], i64, i64); 2] = [
        (
            "Schreiben",
            "#34a853",
            &["writer.exe", "notes.exe"],
            95,
            360,
        ),
        ("Programmieren", "#4285f4", &["editor.exe"], 40, 0),
    ];
    let now = Utc::now();
    for (name, color, apps, minutes, goal) in examples {
        let apps: Vec<String> = apps.iter().map(|a| a.to_string()).collect();
        let Ok(id) = state.store.add_activity(&hora_storage::ActivityDraft {
            name,
            color,
            idle_limit: 60,
            apps: &apps,
            weekly_goal: goal,
        }) else {
            continue;
        };
        // Gleiche Zahlen bei jedem Lauf: einfacher Zufallsgenerator
        let mut seed = 0x9e37_79b9_u64 + id as u64;
        let mut next = |n: i64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((seed >> 33) as i64).rem_euclid(n)
        };
        // Gut drei Monate, nicht jeden Tag, zuletzt etwas mehr
        for days in 0..100_i64 {
            if next(10) < 4 {
                continue;
            }
            let scale = 50 + (100 - days) / 2;
            let first = (minutes / 2 + next(minutes)) * scale / 100;
            let end = now - TimeDelta::days(days) - TimeDelta::minutes(5 + next(240));
            let start = end - TimeDelta::minutes(first.max(5));
            let _ = state.store.add_activity_time(id, start, end);
            // Manchmal nach kurzer Pause weiter: zählt als dieselbe Sitzung
            if next(3) == 0 {
                let end = start - TimeDelta::minutes(2 + next(8));
                let begin = end - TimeDelta::minutes(10 + next(minutes / 2));
                let _ = state.store.add_activity_time(id, begin, end);
            }
        }
    }
    crate::activity_ui::apply(state);
}

/// Testkonto mit einer Serie, die zur ersten Aktivität gehört (nur einmal)
fn activity_plan(app: &AppWindow, state: &mut State) {
    use chrono::{Days, NaiveTime};
    let Some(activity) = state
        .store
        .activities()
        .unwrap_or_default()
        .into_iter()
        .next()
    else {
        return;
    };
    if !state.store.accounts().unwrap_or_default().is_empty() {
        return;
    }
    let result = (|| -> Result<(), hora_storage::Error> {
        let account = state.store.add_account(&hora_storage::NewAccount {
            name: "Plan",
            url: "http://127.0.0.1:9/dav/",
            username: "plan",
            credential_key: "plan-ohne-passwort",
            principal_url: None,
            home_url: Some("http://127.0.0.1:9/dav/plan/"),
        })?;
        let href = "http://127.0.0.1:9/dav/plan/uebungen/";
        state.store.sync_calendar_list(
            account,
            &[hora_storage::RemoteCalendar {
                href,
                name: "Übungen",
                color: Some("#34A853"),
                components: "VEVENT",
                read_only: false,
            }],
        )?;
        let calendar = state
            .store
            .calendars()?
            .into_iter()
            .find(|c| c.account_id == account)
            .map(|c| c.id)
            .unwrap_or_default();
        let monday = crate::monday_of(state.now().date());
        let first = (monday - Days::new(21)).format("%Y%m%d");
        let ics = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Hora//Plan//DE\r\nBEGIN:VEVENT\r\nUID:plan-1\r\nDTSTAMP:20261101T080000Z\r\nDTSTART;TZID=Europe/Berlin:{first}T090000\r\nDTEND;TZID=Europe/Berlin:{first}T103000\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR\r\nSUMMARY:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            activity.name
        );
        state.store.apply_sync(
            calendar,
            &hora_storage::SyncBatch {
                upserts: vec![("plan-1.ics", "\"1\"", ics.as_str())],
                full: true,
                ..Default::default()
            },
        )?;
        state
            .store
            .set_event_activity("plan-1", Some(activity.id))?;
        // Gezählte Zeit in den Terminen dieser Woche: Montag kürzer, Mittwoch länger
        let at = |day: chrono::NaiveDate, h: u32, m: u32| {
            hora_core::time::local_to_utc(
                day.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap_or_default()),
                state.home,
            )
        };
        for (day, from, to) in [(0, (9, 5), (10, 15)), (2, (8, 50), (10, 55))] {
            let day = monday + Days::new(day);
            let (start, end) = (at(day, from.0, from.1), at(day, to.0, to.1));
            if end < chrono::Utc::now() {
                state.store.add_activity_time(activity.id, start, end)?;
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        println!("Plan fehlgeschlagen: {e}");
        return;
    }
    state.reload_calendars();
    state.demo_visible = false;
    crate::refresh(app, state, false);
}

fn fill_test_data(store: &mut hora_storage::Store) -> Result<String, hora_storage::Error> {
    use chrono::{Datelike, Days, Local, NaiveDate};
    let account = store.add_account(&hora_storage::NewAccount {
        name: "Messung",
        url: "http://127.0.0.1:9/dav/",
        username: "messung",
        credential_key: "messung-ohne-passwort",
        principal_url: None,
        home_url: Some("http://127.0.0.1:9/dav/messung/"),
    })?;
    let names = ["Arbeit", "Privat", "Schule"];
    let colors = ["#4285F4", "#34A853", "#7B1FA2"];
    let hrefs: Vec<String> = names
        .iter()
        .map(|n| format!("http://127.0.0.1:9/dav/messung/{}/", n.to_lowercase()))
        .collect();
    let remote: Vec<hora_storage::RemoteCalendar> = names
        .iter()
        .zip(&hrefs)
        .zip(colors)
        .map(|((name, href), color)| hora_storage::RemoteCalendar {
            href,
            name,
            color: Some(color),
            components: "VEVENT,VTODO,VJOURNAL",
            read_only: false,
        })
        .collect();
    store.sync_calendar_list(account, &remote)?;
    let calendars = store.calendars()?;
    let today = Local::now().date_naive();
    let first = today - Days::new(365 * 2);
    let wrap = |body: String| {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Hora//Messung//DE\r\n{body}END:VCALENDAR\r\n"
        )
    };
    let stamp = "DTSTAMP:20261101T080000Z\r\n";
    let titles = [
        "Besprechung",
        "Kundentermin",
        "Mittagessen",
        "Sport",
        "Arzt",
        "Elternabend",
        "Projekt-Review",
        "Telefonat",
        "Einkauf",
        "Kino",
        "Chorprobe",
        "Lernzeit",
    ];
    let places = ["Raum 204", "Online", "Büro", "Stadtpark", "Praxis", ""];
    let mut per_calendar: Vec<Vec<(String, String)>> = vec![Vec::new(); calendars.len()];
    let mut events = 0;
    let mut day = first;
    let mut n = 0usize;
    // Einzeltermine: an Werktagen vier, am Wochenende einer
    while day < today + Days::new(365) {
        let count = if day.weekday().num_days_from_monday() < 5 {
            4
        } else {
            1
        };
        for k in 0..count {
            n += 1;
            let hour = 8 + (k * 3 + n % 2) as u32;
            let d = day.format("%Y%m%d");
            let body = format!(
                "BEGIN:VEVENT\r\nUID:messung-{n}\r\n{stamp}DTSTART;TZID=Europe/Berlin:{d}T{hour:02}0000\r\nDTEND;TZID=Europe/Berlin:{d}T{:02}3000\r\nSUMMARY:{} {n}\r\nLOCATION:{}\r\nDESCRIPTION:Testtermin für die Messung mit etwas Text\\, damit die Suche etwas zu tun hat.\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Erinnerung\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\n",
                hour + 1,
                titles[n % titles.len()],
                places[n % places.len()],
            );
            per_calendar[n % calendars.len()].push((format!("e{n}.ics"), wrap(body)));
            events += 1;
        }
        day = day + Days::new(1);
    }
    // Serien: wöchentlich, täglich an Werktagen, monatlich
    for s in 0..60usize {
        let start = first + Days::new((s * 11) as u64);
        let rule = match s % 3 {
            0 => "FREQ=WEEKLY".to_string(),
            1 => "FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR;COUNT=400".to_string(),
            _ => "FREQ=MONTHLY;INTERVAL=1".to_string(),
        };
        let d = start.format("%Y%m%d");
        let body = format!(
            "BEGIN:VEVENT\r\nUID:serie-{s}\r\n{stamp}DTSTART;TZID=Europe/Berlin:{d}T{:02}1500\r\nDTEND;TZID=Europe/Berlin:{d}T{:02}4500\r\nRRULE:{rule}\r\nSUMMARY:Serie {s}\r\nEND:VEVENT\r\n",
            7 + s % 10,
            7 + s % 10,
        );
        per_calendar[s % calendars.len()].push((format!("s{s}.ics"), wrap(body)));
    }
    // Aufgaben: offen und erledigt, mit und ohne Fälligkeit, Teilaufgaben
    for t in 0..400usize {
        let due = today + Days::new((t % 90) as u64) - Days::new(30);
        let done = if t % 3 == 0 {
            "STATUS:COMPLETED\r\nCOMPLETED:20261101T080000Z\r\n"
        } else {
            "STATUS:NEEDS-ACTION\r\n"
        };
        let parent = if t % 7 == 1 {
            format!("RELATED-TO;RELTYPE=PARENT:aufgabe-{}\r\n", t - 1)
        } else {
            String::new()
        };
        let due = if t % 5 == 4 {
            String::new()
        } else {
            format!("DUE;VALUE=DATE:{}\r\n", due.format("%Y%m%d"))
        };
        let body = format!(
            "BEGIN:VTODO\r\nUID:aufgabe-{t}\r\n{stamp}SUMMARY:Aufgabe {t}\r\n{due}{done}{parent}END:VTODO\r\n"
        );
        per_calendar[t % calendars.len()].push((format!("t{t}.ics"), wrap(body)));
    }
    // Journal: ein Eintrag an jedem dritten Tag der letzten 900 Tage
    for j in 0..300usize {
        let d: NaiveDate = today - Days::new((j * 3) as u64);
        let body = format!(
            "BEGIN:VJOURNAL\r\nUID:journal-{j}\r\n{stamp}DTSTART;VALUE=DATE:{}\r\nSUMMARY:Tagebuch {j}\r\nDESCRIPTION:## Heute\\n\\n- **Erledigt:** eins\\, zwei\\n- *Gelesen:* ein Kapitel\\n\\nNoch ein Absatz mit etwas Text.\r\nCATEGORIES:tagebuch,test\r\nEND:VJOURNAL\r\n",
            d.format("%Y%m%d")
        );
        per_calendar[j % calendars.len()].push((format!("j{j}.ics"), wrap(body)));
    }
    let mut total = 0;
    for (calendar, items) in calendars.iter().zip(&per_calendar) {
        let upserts: Vec<(&str, &str, &str)> = items
            .iter()
            .map(|(href, ics)| (href.as_str(), "\"1\"", ics.as_str()))
            .collect();
        total += upserts.len();
        store.apply_sync(
            calendar.id,
            &hora_storage::SyncBatch {
                upserts,
                full: true,
                ..Default::default()
            },
        )?;
    }
    let _ = store.set_setting("demo_visible", "0");
    Ok(format!(
        "{total} Ressourcen ({events} Einzeltermine, 60 Serien, 400 Aufgaben, 300 Journaleinträge)"
    ))
}

fn save_snapshot(app: &AppWindow, path: &str) {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let buffer = app.window().take_snapshot()?;
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut encoder = png::Encoder::new(file, buffer.width(), buffer.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()?
            .write_image_data(buffer.as_bytes())?;
        Ok(())
    })();
    match result {
        Ok(()) => println!("snapshot: {path}"),
        Err(e) => eprintln!("snapshot fehlgeschlagen: {e}"),
    }
}

struct Selftest {
    app: slint::Weak<AppWindow>,
    state: Rc<RefCell<State>>,
    snapshot: Option<String>,
    step: usize,
    target: i32,
    failures: usize,
    log: Vec<String>,
}

impl Selftest {
    fn start(app: &AppWindow, state: Rc<RefCell<State>>, snapshot: Option<String>) {
        let test = Rc::new(RefCell::new(Selftest {
            app: app.as_weak(),
            state,
            snapshot,
            step: 0,
            target: -1,
            failures: 0,
            log: Vec::new(),
        }));
        let timer = Rc::new(slint::Timer::default());
        let keep = timer.clone();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(120),
            move || {
                let _keep_alive = &keep;
                test.borrow_mut().tick();
            },
        );
    }

    fn item(&self, id: i32) -> Option<EventItem> {
        self.state.borrow().rows.iter().find(|it| it.id == id)
    }

    fn check(&mut self, what: &str, ok: bool, detail: String) {
        self.log.push(format!(
            "{} {what}: {detail}",
            if ok { "OK  " } else { "FEHL" }
        ));
        if !ok {
            self.failures += 1;
        }
    }

    /// Punkt auf der Karte in Fensterkoordinaten; `fy` 0.0 = oben, 1.0 = unten.
    fn point(&self, app: &AppWindow, it: &EventItem, fy: f32, dy_px: f32) -> LogicalPosition {
        let col = app.get_column_width();
        let lane_x = col * (it.lane as f32 / it.lanes as f32) + col * 0.02;
        let x = app.get_days_abs_x() + col * it.day as f32 + lane_x + 12.0;
        let top = it.start_min as f32;
        let height = it.dur_min as f32;
        LogicalPosition::new(x, app.get_days_abs_y() + top + height * fy + dy_px)
    }

    fn pointer(app: &AppWindow, event: WindowEvent) {
        app.window().dispatch_event(event);
    }

    fn key(app: &AppWindow, key: impl Into<slint::SharedString>) {
        let text: slint::SharedString = key.into();
        app.window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        app.window()
            .dispatch_event(WindowEvent::KeyReleased { text });
    }

    fn type_text(app: &AppWindow, text: &str) {
        for c in text.chars() {
            Self::key(app, c.to_string());
        }
    }

    /// Taste mit gedrückter Zusatztaste (Umschalt, Strg)
    fn with_modifier(app: &AppWindow, modifier: Key, key: impl Into<slint::SharedString>) {
        app.window().dispatch_event(WindowEvent::KeyPressed {
            text: modifier.into(),
        });
        Self::key(app, key);
        app.window().dispatch_event(WindowEvent::KeyReleased {
            text: modifier.into(),
        });
    }

    /// Beispiel-Journaleintrag mit diesem Titel: (Text, Tags)
    fn demo_journal(&self, title: &str) -> Option<(String, Vec<String>)> {
        self.state
            .borrow()
            .demo_journals
            .iter()
            .filter_map(|(_, o)| o.journals().into_iter().next())
            .find(|j| j.summary == title)
            .map(|j| (j.description, j.categories))
    }

    fn tick(&mut self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let left = PointerEventButton::Left;
        self.step += 1;
        match self.step {
            // Ein paar Durchläufe warten, bis das Fenster gezeichnet ist
            1..=4 => {}
            5 => {
                // "Mathe", Montag 8:00–9:30, auf Mittwoch 9:00 ziehen
                let mathe = self
                    .state
                    .borrow()
                    .rows
                    .iter()
                    .find(|it| it.title == "Mathe" && it.day == 0);
                let Some(it) = mathe else {
                    self.check("Testtermin gefunden", false, "Mathe am Montag fehlt".into());
                    self.step = 99;
                    return;
                };
                self.target = it.id;
                let p = self.point(&app, &it, 0.25, 0.0);
                let col = app.get_column_width();
                Self::pointer(&app, WindowEvent::PointerMoved { position: p });
                Self::pointer(
                    &app,
                    WindowEvent::PointerPressed {
                        position: p,
                        button: left,
                    },
                );
                Self::pointer(
                    &app,
                    WindowEvent::PointerMoved {
                        position: LogicalPosition::new(p.x, p.y + 6.0),
                    },
                );
                Self::pointer(
                    &app,
                    WindowEvent::PointerMoved {
                        position: LogicalPosition::new(p.x + col, p.y + 30.0),
                    },
                );
                let end = LogicalPosition::new(p.x + col * 2.0, p.y + 62.0);
                Self::pointer(&app, WindowEvent::PointerMoved { position: end });
                Self::pointer(
                    &app,
                    WindowEvent::PointerReleased {
                        position: end,
                        button: left,
                    },
                );
            }
            7 => {
                let it = self.item(self.target).unwrap_or_default();
                self.check(
                    "Ziehen: Mo 8:00 -> Mi 9:00",
                    it.day == 2 && it.start_min == 9 * 60 && it.dur_min == 90,
                    format!(
                        "Tag {} Beginn {} Dauer {}",
                        it.day, it.start_min, it.dur_min
                    ),
                );
                self.check(
                    "Überschneidung mit Physik -> 2 Spalten",
                    it.lanes == 2,
                    format!("Spalte {} von {}", it.lane, it.lanes),
                );
            }
            8 => {
                // Unteren Rand 45 px (= 45 Minuten) nach unten ziehen
                let it = self.item(self.target).unwrap_or_default();
                let p = self.point(&app, &it, 1.0, -3.0);
                Self::pointer(&app, WindowEvent::PointerMoved { position: p });
                Self::pointer(
                    &app,
                    WindowEvent::PointerPressed {
                        position: p,
                        button: left,
                    },
                );
                Self::pointer(
                    &app,
                    WindowEvent::PointerMoved {
                        position: LogicalPosition::new(p.x, p.y + 20.0),
                    },
                );
                let end = LogicalPosition::new(p.x, p.y + 45.0);
                Self::pointer(&app, WindowEvent::PointerMoved { position: end });
                Self::pointer(
                    &app,
                    WindowEvent::PointerReleased {
                        position: end,
                        button: left,
                    },
                );
            }
            10 => {
                let it = self.item(self.target).unwrap_or_default();
                self.check(
                    "Dauer ändern: 90 -> 135 Minuten",
                    it.dur_min == 135 && it.start_min == 9 * 60,
                    format!("Beginn {} Dauer {}", it.start_min, it.dur_min),
                );
                // Karte hat seit dem Klick den Fokus: Pfeil runter = 15 Minuten später
                Self::key(&app, Key::DownArrow);
            }
            12 => {
                let it = self.item(self.target).unwrap_or_default();
                self.check(
                    "Pfeil runter: +15 Minuten",
                    it.start_min == 9 * 60 + 15,
                    format!("Beginn {}", it.start_min),
                );
                app.window().dispatch_event(WindowEvent::KeyPressed {
                    text: Key::Shift.into(),
                });
                Self::key(&app, Key::UpArrow);
                app.window().dispatch_event(WindowEvent::KeyReleased {
                    text: Key::Shift.into(),
                });
            }
            14 => {
                let it = self.item(self.target).unwrap_or_default();
                self.check(
                    "Umschalt+Pfeil hoch: Dauer -15",
                    it.dur_min == 120,
                    format!("Dauer {}", it.dur_min),
                );
                Self::key(&app, Key::LeftArrow);
            }
            16 => {
                let it = self.item(self.target).unwrap_or_default();
                self.check(
                    "Pfeil links: einen Tag früher",
                    it.day == 1,
                    format!("Tag {}", it.day),
                );
                // Ziehen mit Esc abbrechen
                let p = self.point(&app, &it, 0.25, 0.0);
                Self::pointer(
                    &app,
                    WindowEvent::PointerPressed {
                        position: p,
                        button: left,
                    },
                );
                Self::pointer(
                    &app,
                    WindowEvent::PointerMoved {
                        position: LogicalPosition::new(p.x, p.y + 8.0),
                    },
                );
                Self::pointer(
                    &app,
                    WindowEvent::PointerMoved {
                        position: LogicalPosition::new(p.x + 300.0, p.y + 200.0),
                    },
                );
                Self::key(&app, Key::Escape);
                let end = LogicalPosition::new(p.x + 300.0, p.y + 200.0);
                Self::pointer(
                    &app,
                    WindowEvent::PointerReleased {
                        position: end,
                        button: left,
                    },
                );
            }
            18 => {
                let it = self.item(self.target).unwrap_or_default();
                self.check(
                    "Esc bricht Ziehen ab",
                    it.day == 1 && it.start_min == 9 * 60 + 15,
                    format!("Tag {} Beginn {}", it.day, it.start_min),
                );
                // Strg+K öffnet die Befehlspalette
                app.invoke_focus_main();
                app.window().dispatch_event(WindowEvent::KeyPressed {
                    text: Key::Control.into(),
                });
                Self::key(&app, "k");
                app.window().dispatch_event(WindowEvent::KeyReleased {
                    text: Key::Control.into(),
                });
            }
            20 => {
                self.check(
                    "Strg+K öffnet die Befehlspalette",
                    app.get_palette_open(),
                    format!("offen: {}", app.get_palette_open()),
                );
                for c in ["j", "a", "h", "r"] {
                    Self::key(&app, c);
                }
            }
            22 => {
                let selected = app
                    .get_palette_items()
                    .row_data(app.get_palette_selected().max(0) as usize)
                    .map(|r| r.title.to_string())
                    .unwrap_or_default();
                self.check(
                    "Tippen sucht, erster Treffer gewählt",
                    selected == "Jahresansicht",
                    format!("„{}“ -> {selected}", app.get_palette_text()),
                );
                Self::key(&app, Key::Return);
            }
            24 => {
                self.check(
                    "Eingabe führt aus: Jahresansicht",
                    app.get_view() == 4 && !app.get_palette_open(),
                    format!(
                        "Ansicht {} Palette {}",
                        app.get_view(),
                        app.get_palette_open()
                    ),
                );
                // Erster Pfeil führt ins Raster, der zweite bewegt den Tag
                Self::key(&app, Key::RightArrow);
                Self::key(&app, Key::RightArrow);
            }
            26 => {
                use chrono::Datelike;
                let today = chrono::Local::now().date_naive();
                let last = if today.leap_year() { 365 } else { 364 };
                let expected = (today.ordinal0() as i32 + 1).min(last);
                self.check(
                    "Jahr: Pfeil rechts wählt den nächsten Tag",
                    app.get_year_cursor() == expected,
                    format!("Tag {} (erwartet {expected})", app.get_year_cursor()),
                );
                Self::key(&app, Key::Return);
            }
            28 => {
                let today = chrono::Local::now().date_naive();
                let day = (today + chrono::Duration::days(1)).min(
                    chrono::NaiveDate::from_ymd_opt(chrono::Datelike::year(&today), 12, 31)
                        .unwrap_or(today),
                );
                let title = crate::forms::long_day(day);
                self.check(
                    "Jahr: Eingabe öffnet den Tag",
                    app.get_view() == 0 && app.get_month_title() == title.as_str(),
                    format!("Ansicht {} „{}“", app.get_view(), app.get_month_title()),
                );
                app.invoke_go_today();
                Self::key(&app, "l");
            }
            30 => {
                let rows = app.get_agenda_rows();
                let today = rows.iter().any(|r| r.kind == 0 && r.today);
                self.check(
                    "L: Agenda mit heutigem Tag",
                    app.get_view() == 5 && today && rows.iter().any(|r| r.kind == 1),
                    format!(
                        "Ansicht {} Zeilen {} heute {today}",
                        app.get_view(),
                        rows.row_count()
                    ),
                );
                Self::key(&app, "/");
            }
            32 => {
                let open = app.get_palette_open();
                Self::key(&app, Key::Escape);
                self.check("/ öffnet die Palette", open, format!("offen: {open}"));
            }
            34 => {
                self.check(
                    "Esc schließt die Palette",
                    !app.get_palette_open(),
                    format!("offen: {}", app.get_palette_open()),
                );
                Self::key(&app, "b");
            }
            36 => {
                self.check(
                    "B: Journal mit Einträgen des Monats",
                    app.get_view() == 7 && app.get_journal_count() >= 1 && !app.get_journal_open(),
                    format!(
                        "Ansicht {} Einträge {} Editor {}",
                        app.get_view(),
                        app.get_journal_count(),
                        app.get_journal_open()
                    ),
                );
                Self::key(&app, "n");
            }
            38 => {
                self.check(
                    "N: neuer Eintrag im Schreibmodus",
                    app.get_journal_open()
                        && app.get_journal_write()
                        && app.get_journal_title().is_empty(),
                    format!(
                        "Editor {} schreiben {} Titel „{}“",
                        app.get_journal_open(),
                        app.get_journal_write(),
                        app.get_journal_title()
                    ),
                );
                Self::type_text(&app, "Selbsttest");
            }
            40 => {
                self.check(
                    "Tippen in den Titel, noch nicht gespeichert",
                    app.get_journal_title() == "Selbsttest" && app.get_journal_status() == 1,
                    format!(
                        "„{}“ Stand {} {}",
                        app.get_journal_title(),
                        app.get_journal_status(),
                        app.get_journal_status_text()
                    ),
                );
                // Eingabe springt in den Text; "Wort" markieren und Strg+B
                Self::key(&app, Key::Return);
                Self::type_text(&app, "Ein Wort");
                for _ in 0..4 {
                    Self::with_modifier(&app, Key::Shift, Key::LeftArrow);
                }
                Self::with_modifier(&app, Key::Control, "b");
            }
            42 => {
                self.check(
                    "Strg+B macht die Auswahl fett",
                    app.get_journal_body() == "Ein **Wort**",
                    format!("„{}“", app.get_journal_body()),
                );
            }
            // Nach einer halben Sekunde Pause gespeichert
            48 => {
                let saved = self.demo_journal("Selbsttest");
                self.check(
                    "Automatisch gespeichert",
                    saved
                        .as_ref()
                        .is_some_and(|(text, _)| text == "Ein **Wort**")
                        && app.get_journal_status() != 1,
                    format!("{saved:?} Stand „{}“", app.get_journal_status_text()),
                );
                Self::key(&app, Key::Escape);
            }
            50 => {
                self.check(
                    "Esc: Lesen mit Markdown",
                    !app.get_journal_write() && app.get_journal_blocks().row_count() == 1,
                    format!(
                        "schreiben {} Blöcke {}",
                        app.get_journal_write(),
                        app.get_journal_blocks().row_count()
                    ),
                );
                Self::key(&app, Key::DownArrow);
            }
            52 => {
                let title = app.get_journal_title();
                self.check(
                    "Pfeil runter: nächster Eintrag",
                    app.get_journal_open() && title != "Selbsttest",
                    format!("„{title}“"),
                );
                Self::key(&app, Key::UpArrow);
            }
            54 => {
                self.check(
                    "Pfeil hoch: zurück",
                    app.get_journal_title() == "Selbsttest",
                    format!("„{}“", app.get_journal_title()),
                );
                app.invoke_journal_add_tag("Probe, Test".into());
            }
            56 => {
                let tags: Vec<String> = app
                    .get_journal_tags()
                    .iter()
                    .map(|t| t.to_string())
                    .collect();
                let saved = self.demo_journal("Selbsttest").map(|(_, t)| t);
                self.check(
                    "Tags hinzufügen (mit Komma getrennt)",
                    tags == ["Probe", "Test"]
                        && saved.as_deref() == Some(&["Probe".to_string(), "Test".to_string()][..]),
                    format!("{tags:?} gespeichert {saved:?}"),
                );
                app.invoke_journal_focus_list();
                Self::key(&app, Key::Delete);
            }
            58 => {
                self.check(
                    "Entf fragt nach",
                    app.get_confirm_open(),
                    format!("Dialog {}", app.get_confirm_open()),
                );
                // Abbrechen, Löschen; Eingabe drückt den Knopf
                Self::key(&app, Key::Tab);
                Self::key(&app, Key::Tab);
                Self::key(&app, Key::Return);
            }
            60 => {
                let gone = self.demo_journal("Selbsttest").is_none();
                self.check(
                    "Gelöscht, nächster Eintrag gewählt",
                    gone && !app.get_confirm_open() && app.get_journal_open(),
                    format!(
                        "weg {gone} Dialog {} Editor {} „{}“",
                        app.get_confirm_open(),
                        app.get_journal_open(),
                        app.get_journal_title()
                    ),
                );
                app.invoke_set_view(2);
            }
            // Sprache im laufenden Betrieb wechseln (Index 0 = Windows, 1 = en, 2 = de, 3 = fr)
            61 => {
                app.set_settings_language(1);
                app.invoke_language_changed();
            }
            62 => {
                let rows = self
                    .state
                    .borrow()
                    .rows
                    .iter()
                    .map(|r| r.title.to_string())
                    .collect::<Vec<_>>();
                let week = app.get_week_label().to_string();
                self.check(
                    "Englisch: Beispieltermine und Kalenderwoche",
                    rows.iter().any(|t| t == "Math")
                        && week.starts_with('W')
                        && hora_i18n::language() == hora_i18n::Lang::En,
                    format!(
                        "Woche „{week}“, Termine {:?}",
                        rows.iter().take(3).collect::<Vec<_>>()
                    ),
                );
                app.set_settings_language(3);
                app.invoke_language_changed();
            }
            63 => {
                let rows = self
                    .state
                    .borrow()
                    .rows
                    .iter()
                    .map(|r| r.title.to_string())
                    .collect::<Vec<_>>();
                let mini = app.get_mini_month().to_string();
                self.check(
                    "Französisch: Termine und Monatsname",
                    rows.iter().any(|t| t == "Maths")
                        && mini.chars().next().is_some_and(char::is_uppercase)
                        && app
                            .global::<crate::Locale>()
                            .get_weekday_letter()
                            .row_data(3)
                            .as_deref()
                            == Some("J"),
                    format!(
                        "Monat „{mini}“, Termine {:?}",
                        rows.iter().take(3).collect::<Vec<_>>()
                    ),
                );
                // Feste Formate: 24 Stunden, Tag.Monat.Jahr
                app.set_settings_formats(1);
                app.invoke_formats_changed();
            }
            64 => {
                let locale = app.global::<crate::Locale>();
                let placeholder = locale.get_date_placeholder().to_string();
                self.check(
                    "Format 24 h mit Punkt",
                    !locale.get_hour12() && placeholder == "JJ.MM.AAAA",
                    format!("12 h {} Platzhalter „{placeholder}“", locale.get_hour12()),
                );
                app.set_settings_formats(3);
                app.invoke_formats_changed();
            }
            65 => {
                let locale = app.global::<crate::Locale>();
                self.check(
                    "Format 12 h, Monat/Tag/Jahr",
                    locale.get_hour12() && locale.get_date_placeholder() == "MM/JJ/AAAA",
                    format!(
                        "12 h {} Platzhalter „{}“",
                        locale.get_hour12(),
                        locale.get_date_placeholder()
                    ),
                );
                // Zurück: Deutsch, Windows-Format
                app.set_settings_formats(0);
                app.invoke_formats_changed();
                app.set_settings_language(2);
                app.invoke_language_changed();
            }
            66 => {
                let rows = self
                    .state
                    .borrow()
                    .rows
                    .iter()
                    .map(|r| r.title.to_string())
                    .collect::<Vec<_>>();
                self.check(
                    "Zurück auf Deutsch",
                    rows.iter().any(|t| t == "Mathe") && app.get_week_label().starts_with("KW"),
                    format!("Woche „{}“", app.get_week_label()),
                );
                self.step = 99;
            }
            100 => {
                if let Some(path) = self.snapshot.clone() {
                    save_snapshot(&app, &path);
                }
                for line in &self.log {
                    println!("{line}");
                }
                let summary = if self.failures == 0 {
                    "Selbsttest bestanden".to_string()
                } else {
                    format!("Selbsttest: {} Prüfung(en) fehlgeschlagen", self.failures)
                };
                println!("{summary}");
                let _ = std::fs::write(
                    "selftest.log",
                    format!("{}\n{summary}\n", self.log.join("\n")),
                );
                std::process::exit(if self.failures == 0 { 0 } else { 1 });
            }
            _ => {}
        }
    }
}
