// Release-Build ohne Konsolenfenster
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod activity;
mod activity_ui;
mod editing;
mod forms;
mod items;
mod journal;
mod journal_ui;
mod log;
mod overview;
mod palette;
mod quick_add;
mod reminders;
mod repeat;
mod sample;
mod shell;
mod sync;
mod tasks;
mod updates;
mod view;
mod week;

#[cfg(feature = "devtools")]
mod devtools;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use chrono::{Datelike, Days, Months, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc};
use hora_core::Tz;
use hora_core::model::{EventInfo, Kind};
use hora_core::series::Scope;
use hora_storage::{Account, Calendar, OpState, PendingSummary, Store};
use slint::{Color, ComponentHandle, Model, ModelRc, SharedString, VecModel};

use editing::{DialogInput, ReminderInput, SeriesAction};
use forms::DraftTimes;
use hora_i18n::{Lang, dates, tr, trn};
use items::{Item, Source};
use sync::{Command, Update, Worker};
use view::Placement;

slint::include_modules!();

/// Calino CALENDAR_COLORS, für Kalender ohne eigene Farbe
const CALENDAR_COLORS: [u32; 12] = [
    0x4285F4, 0xEA4335, 0xFBBC05, 0x34A853, 0xFF6D01, 0x46BDC6, 0x7B1FA2, 0xC2185B, 0x00796B,
    0xF57C00, 0x455A64, 0x5D4037,
];
const DEMO_COLOR: u32 = 0xb07d4f;
const SYNC_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Wie oft wartende Änderungen erneut versucht werden (der Thread hält die
/// Wartezeit je Änderung selbst ein)
const PUSH_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Day,
    ThreeDays,
    Week,
    Month,
    Year,
    Agenda,
    Tasks,
    Journal,
}

impl View {
    fn from_index(i: i32) -> Self {
        match i {
            0 => View::Day,
            1 => View::ThreeDays,
            3 => View::Month,
            4 => View::Year,
            5 => View::Agenda,
            6 => View::Tasks,
            7 => View::Journal,
            _ => View::Week,
        }
    }

    fn index(self) -> i32 {
        match self {
            View::Day => 0,
            View::ThreeDays => 1,
            View::Week => 2,
            View::Month => 3,
            View::Year => 4,
            View::Agenda => 5,
            View::Tasks => 6,
            View::Journal => 7,
        }
    }
}

/// Einstellungen aus der Datenbank (Autostart steht in der Registry)
struct Settings {
    reminders: bool,
    /// Minuten vor Beginn für neue Termine; `None` = keine
    default_reminder: Option<i64>,
    keep_running: bool,
    /// Offene Aufgaben am Tag ihrer Fälligkeit in Woche und Monat
    show_tasks: bool,
    /// Erinnerung zur Fälligkeit (Aufgaben ohne eigene Erinnerung)
    task_reminders: bool,
    /// Einmal am Tag nach Updates sehen und sie im Hintergrund laden
    update_check: bool,
    /// App-Zeiten zählen (aus, bis man es einschaltet)
    activities: bool,
}

impl Settings {
    fn load(store: &Store) -> Settings {
        let get = |key: &str| store.setting(key).ok().flatten();
        Settings {
            reminders: get("reminders").as_deref() != Some("0"),
            default_reminder: match get("default_reminder").as_deref() {
                Some("none") => None,
                Some(v) => v.parse().ok().or(Some(15)),
                None => Some(15),
            },
            keep_running: get("keep_running").as_deref() != Some("0"),
            show_tasks: get("show_tasks").as_deref() != Some("0"),
            task_reminders: get("task_reminders").as_deref() != Some("0"),
            update_check: get("update_check").as_deref() != Some("0"),
            activities: get("activities").as_deref() == Some("1"),
        }
    }

    fn save(&self, store: &Store) {
        let flag = |b: bool| if b { "1" } else { "0" };
        let _ = store.set_setting("reminders", flag(self.reminders));
        let _ = store.set_setting("keep_running", flag(self.keep_running));
        let _ = store.set_setting("show_tasks", flag(self.show_tasks));
        let _ = store.set_setting("task_reminders", flag(self.task_reminders));
        let _ = store.set_setting("update_check", flag(self.update_check));
        let _ = store.set_setting("activities", flag(self.activities));
        let reminder = self
            .default_reminder
            .map_or("none".to_string(), |m| m.to_string());
        let _ = store.set_setting("default_reminder", &reminder);
    }
}

#[derive(Default)]
struct SyncInfo {
    running: bool,
    last: Option<NaiveDateTime>,
    errors: Vec<String>,
}

struct State {
    home: Tz,
    view: View,
    /// Gewählter Tag; Woche und Monat ergeben sich daraus
    anchor: NaiveDate,
    /// Erster Tag des Monats im Mini-Kalender
    mini_month: NaiveDate,
    /// Leseverbindung; geschrieben wird vor allem im Sync-Thread
    store: Store,
    accounts: Vec<Account>,
    calendars: Vec<Calendar>,
    events: Vec<sample::Event>,
    all_day: Vec<sample::AllDay>,
    next_id: i32,
    demo_visible: bool,
    /// Gelesene Termine je (Kalender, href), mit Prüfsumme des Inhalts
    parsed: HashMap<(i64, String), (u64, Vec<EventInfo>)>,
    /// Termine der aktuellen Ansicht; ID im Modell = Index
    items: Vec<Item>,
    rows: Rc<VecModel<EventItem>>,
    /// Schlüssel der Zeilen in `rows`, damit Neuladen die Reihenfolge hält
    row_keys: Vec<String>,
    worker: Option<Worker>,
    sync: SyncInfo,
    pending: PendingSummary,
    /// Termin in Vorschau, Dialog bzw. Löschbestätigung
    preview: Option<Item>,
    editing: Option<Item>,
    deleting: Option<Item>,
    /// Änderung an einer Serie, die auf "nur dieser / folgende / alle" wartet
    series_action: Option<SeriesAction>,
    settings: Settings,
    planner: reminders::Planner,
    /// Infobereich und Benachrichtigungen
    shell: Option<shell::Shell>,
    /// Erinnerungen im Termin-Dialog und ihr Stand beim Öffnen
    reminder_rows: Rc<VecModel<ReminderChip>>,
    reminders_before: Vec<ReminderChip>,
    /// Kennungen der zuletzt gezeigten Toasts (für den Ende-zu-Ende-Test)
    #[cfg_attr(not(feature = "devtools"), allow(dead_code))]
    shown_toasts: Vec<(String, String)>,
    /// Aufgaben: gelesene VTODOs je Ressource, Liste der Ansicht, Zustand
    parsed_tasks: HashMap<(i64, String), (u64, Vec<hora_core::task::TaskInfo>)>,
    tasks: Vec<tasks::TaskItem>,
    task_filter: tasks::Filter,
    collapsed_tasks: HashSet<String>,
    editing_task: Option<tasks::TaskItem>,
    deleting_task: Option<tasks::TaskItem>,
    /// Auswahllisten des Aufgaben-Dialogs
    task_calendar_ids: Vec<i64>,
    task_parent_ids: Vec<Option<String>>,
    /// Kalender-IDs der Auswahlliste im Dialog (-1 = Beispieltermine)
    option_ids: Vec<i64>,
    conflict: Option<i64>,
    /// Mit "Später" zurückgestellte Konflikte
    dismissed_conflicts: HashSet<i64>,
    /// Einträge der offenen Befehlspalette
    palette: Vec<palette::Entry>,
    /// Suchindex über alle Termine; bei Änderungen verworfen
    search_index: Option<Vec<palette::SearchDoc>>,
    /// Journal: gelesene VJOURNALs je Ressource, Beispiel-Einträge im Speicher
    parsed_journals: HashMap<(i64, String), (u64, Vec<hora_core::journal::JournalInfo>)>,
    demo_journals: Vec<(String, hora_core::model::CalendarObject)>,
    /// Alle Einträge und die gezeigten (Monat oder alle)
    journal_entries: Vec<journal::Entry>,
    journals: Vec<journal::Entry>,
    journal_all: bool,
    journal_selected: Option<String>,
    journal_draft: Option<journal::Entry>,
    /// Eingaben im Editor, die noch nicht gespeichert sind
    journal_dirty: bool,
    journal_timer: slint::Timer,
    /// Titel und Text, wie der Editor sie zuletzt geladen bzw. gespeichert hat
    journal_loaded: (String, String),
    journal_error: Option<String>,
    journal_create_busy: bool,
    journal_create_error: Option<String>,
    deleting_journal: Option<journal::Entry>,
    updates: updates::Updates,
    /// Datenbankdatei, für Threads mit eigener Verbindung
    db: PathBuf,
    /// App-Zeiten: Zählung (nur wenn eingeschaltet und nicht pausiert),
    /// Pause über den Infobereich, Dialog einer Aktivität
    counter: Option<activity::Counter>,
    counting_paused: bool,
    activity_dialog: activity_ui::Dialog,
}

impl State {
    fn now(&self) -> NaiveDateTime {
        Utc::now().with_timezone(&self.home).naive_local()
    }

    fn reload_calendars(&mut self) {
        self.accounts = self.store.accounts().unwrap_or_default();
        self.calendars = self.store.calendars().unwrap_or_default();
    }

    fn send(&self, command: Command) {
        if let Some(worker) = &self.worker {
            worker.send(command);
        }
    }

    /// Erster Tag und Anzahl Tage der Ansicht
    fn range(&self) -> (NaiveDate, i64) {
        match self.view {
            View::Day => (self.anchor, 1),
            View::ThreeDays => (self.anchor, 3),
            View::Week | View::Tasks => (monday_of(self.anchor), 7),
            View::Month => (
                monday_of(self.anchor.with_day(1).unwrap_or(self.anchor)),
                42,
            ),
            View::Year => {
                let first =
                    NaiveDate::from_ymd_opt(self.anchor.year(), 1, 1).unwrap_or(self.anchor);
                (first, (first + Months::new(12) - first).num_days())
            }
            View::Agenda | View::Journal => {
                let first = self.anchor.with_day(1).unwrap_or(self.anchor);
                (first, (first + Months::new(1) - first).num_days())
            }
        }
    }
}

type UiHandle = (slint::Weak<AppWindow>, Rc<RefCell<State>>);

thread_local! {
    /// Für Rückmeldungen aus dem Sync-Thread, die über die Ereignisschleife kommen
    static UI: RefCell<Option<UiHandle>> = const { RefCell::new(None) };
}

fn monday_of(date: NaiveDate) -> NaiveDate {
    date - Days::new(u64::from(date.weekday().num_days_from_monday()))
}

fn minutes_of(t: NaiveDateTime) -> i32 {
    (t.hour() * 60 + t.minute()) as i32
}

fn rgb(hex: u32) -> Color {
    Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn parse_hex(color: &str) -> Option<u32> {
    u32::from_str_radix(color.trim().strip_prefix('#')?.get(..6)?, 16).ok()
}

fn calendar_color(calendar: &Calendar, index: usize) -> u32 {
    calendar
        .color()
        .and_then(parse_hex)
        .unwrap_or(CALENDAR_COLORS[index % CALENDAR_COLORS.len()])
}

/// "November 2026", über Monatsgrenzen "Oct – Nov 2026" bzw. "Dec 2026 – Jan 2027"
fn month_title(first: NaiveDate, last: NaiveDate) -> String {
    let short = |d: NaiveDate| dates::capitalize(dates::month_short(d.month0() as usize));
    if first.month() == last.month() && first.year() == last.year() {
        dates::capitalize(&dates::month_year(first.year(), first.month0() as usize))
    } else if first.year() == last.year() {
        format!("{} – {} {}", short(first), short(last), last.year())
    } else {
        format!(
            "{} {} – {} {}",
            short(first),
            first.year(),
            short(last),
            last.year()
        )
    }
}

fn view_title(view: View, anchor: NaiveDate, first: NaiveDate, days: i64) -> String {
    match view {
        View::Day => forms::long_day(anchor),
        View::Month | View::Agenda => {
            dates::capitalize(&dates::month_year(anchor.year(), anchor.month0() as usize))
        }
        View::Year => anchor.year().to_string(),
        _ => month_title(first, first + Days::new(days as u64 - 1)),
    }
}

/// Spaltenaufteilung je Tag neu berechnen.
fn apply_layout(items: &mut [EventItem]) {
    let days = items.iter().map(|it| it.day).max().map_or(0, |d| d + 1);
    for day in 0..days {
        let indices: Vec<usize> = (0..items.len()).filter(|&i| items[i].day == day).collect();
        let spans: Vec<(i32, i32)> = indices
            .iter()
            .map(|&i| (items[i].start_min, items[i].start_min + items[i].dur_min))
            .collect();
        for (slot, &i) in week::layout_day(&spans).into_iter().zip(&indices) {
            items[i].lane = slot.lane;
            items[i].lanes = slot.lanes;
            items[i].span = slot.span;
        }
    }
}

fn calendar_rows(state: &State) -> Vec<CalendarRow> {
    let mut rows = vec![CalendarRow {
        id: -1,
        account: tr!("On this device").into(),
        name: tr!("Sample events").into(),
        color: rgb(DEMO_COLOR),
        visible: state.demo_visible,
        read_only: false,
        first_of_account: true,
    }];
    for account in &state.accounts {
        let calendars = state
            .calendars
            .iter()
            .enumerate()
            // Auch reine Aufgabenlisten und Journal-Kalender (ein-/ausblenden, Farbe)
            .filter(|(_, c)| {
                c.account_id == account.id
                    && [Kind::Event, Kind::Todo, Kind::Journal]
                        .into_iter()
                        .any(|k| c.supports(k))
            });
        for (n, (index, calendar)) in calendars.enumerate() {
            rows.push(CalendarRow {
                id: calendar.id as i32,
                account: account.name.as_str().into(),
                name: calendar.name.as_str().into(),
                color: rgb(calendar_color(calendar, index)),
                visible: calendar.visible,
                read_only: calendar.read_only,
                first_of_account: n == 0,
            });
        }
    }
    rows
}

/// Kalender, in die neue Termine dürfen; Beispieltermine, solange sichtbar
/// oder kein beschreibbarer Kalender existiert.
fn calendar_options(state: &mut State) -> Vec<SelectOption> {
    let mut options = Vec::new();
    let mut ids = Vec::new();
    for (index, calendar) in state.calendars.iter().enumerate() {
        if calendar.read_only || !calendar.supports(Kind::Event) {
            continue;
        }
        let account = state
            .accounts
            .iter()
            .find(|a| a.id == calendar.account_id)
            .map_or("", |a| a.name.as_str());
        options.push(SelectOption {
            id: calendar.id as i32,
            name: format!("{} · {}", calendar.name, account).into(),
            color: rgb(calendar_color(calendar, index)),
        });
        ids.push(calendar.id);
    }
    if state.demo_visible || options.is_empty() {
        options.push(SelectOption {
            id: -1,
            name: tr!("Sample events (only on this device)").into(),
            color: rgb(DEMO_COLOR),
        });
        ids.push(-1);
    }
    state.option_ids = ids;
    options
}

fn load_week(app: &AppWindow, state: &mut State, first: NaiveDate, days: i64, keep_order: bool) {
    let today = state.now().date();
    let infos: Vec<DayInfo> = (0..days)
        .map(|i| {
            let date = first + Days::new(i as u64);
            DayInfo {
                name: dates::weekday_header(date.weekday().num_days_from_monday() as usize)
                    .to_uppercase()
                    .into(),
                number: date.day().to_string().into(),
                today: date == today,
            }
        })
        .collect();
    app.set_days(ModelRc::new(VecModel::from(infos)));
    app.set_week_label(tr!("W{}", first.iso_week().week()).into());

    let home = state.home;
    let mut rows: Vec<(String, EventItem)> = Vec::new();
    // (id, Titel, Farbe, erste Spalte, Spalte nach der letzten)
    let mut spans: Vec<(i32, String, u32, i32, i32)> = Vec::new();
    for (id, item) in state.items.iter().enumerate() {
        match view::place(&item.span, first, days as i32, home) {
            Placement::Timed(parts) => {
                let whole = parts.len() == 1;
                for (n, (day, start, dur)) in parts.into_iter().enumerate() {
                    rows.push((
                        format!("{}#{n}", item.key()),
                        EventItem {
                            id: id as i32,
                            title: item.title.as_str().into(),
                            location: item.location.as_str().into(),
                            color: rgb(item.color),
                            day,
                            start_min: start,
                            dur_min: dur,
                            lane: 0,
                            lanes: 1,
                            span: 1,
                            // Über Mitternacht geteilte Termine nur im Dialog ändern
                            editable: item.movable() && whole,
                        },
                    ));
                }
            }
            Placement::AllDay(f, e) => {
                spans.push((id as i32, item.title.clone(), item.color, f, e))
            }
            Placement::Outside => {}
        }
    }

    // Nach dem Neuladen derselben Ansicht bleibt die Reihenfolge der Karten,
    // damit Tastaturfokus und Karten-Instanzen erhalten bleiben; sonst zeitlich
    if keep_order {
        let position: HashMap<&str, usize> = state
            .row_keys
            .iter()
            .enumerate()
            .map(|(i, k)| (k.as_str(), i))
            .collect();
        rows.sort_by_key(|(key, it)| {
            (
                position.get(key.as_str()).copied().unwrap_or(usize::MAX),
                it.day,
                it.start_min,
            )
        });
    } else {
        rows.sort_by_key(|(_, it)| (it.day, it.start_min, -it.dur_min, it.id));
    }
    let keys: Vec<String> = rows.iter().map(|(k, _)| k.clone()).collect();
    let mut events: Vec<EventItem> = rows.into_iter().map(|(_, it)| it).collect();
    apply_layout(&mut events);
    if keys == state.row_keys && events.len() == state.rows.row_count() {
        for (row, item) in events.into_iter().enumerate() {
            if state.rows.row_data(row).as_ref() != Some(&item) {
                state.rows.set_row_data(row, item);
            }
        }
    } else {
        state.rows.set_vec(events);
    }
    state.row_keys = keys;

    let ranges: Vec<(i32, i32)> = spans.iter().map(|s| (s.3, s.4)).collect();
    let lanes = week::layout_all_day(&ranges);
    let all_day: Vec<AllDayItem> = spans
        .iter()
        .zip(&lanes)
        .map(|((id, title, color, first, end), &lane)| AllDayItem {
            id: *id,
            title: title.as_str().into(),
            color: rgb(*color),
            start_day: *first,
            span: end - first,
            lane,
            task: state.items.get(*id as usize).is_some_and(Item::is_task),
        })
        .collect();
    app.set_all_day(ModelRc::new(VecModel::from(all_day)));
    app.set_all_day_lanes(lanes.iter().copied().max().map_or(0, |m| m + 1));
}

fn load_month(app: &AppWindow, state: &mut State, first: NaiveDate) {
    let home = state.home;
    let today = state.now().date();
    // Je Tag die Termine: ganztägige und mehrtägige zuerst, dann nach Beginn
    let mut per_cell: Vec<Vec<usize>> = vec![Vec::new(); 42];
    for (id, item) in state.items.iter().enumerate() {
        let (from, to) = item.span.days(home);
        let mut day = from.max(first);
        while day <= to {
            let cell = (day - first).num_days();
            if !(0..42).contains(&cell) {
                break;
            }
            per_cell[cell as usize].push(id);
            day = day + Days::new(1);
        }
    }
    let mut chips = Vec::new();
    let mut cells = Vec::new();
    // Journaleinträge je Tag (Buch-Symbol)
    let mut journal_days: HashMap<NaiveDate, i32> = HashMap::new();
    for entry in journal::collect(state) {
        if !entry.draft {
            *journal_days.entry(entry.day).or_default() += 1;
        }
    }
    for (cell, ids) in per_cell.iter_mut().enumerate() {
        ids.sort_by_key(|&id| {
            let item = &state.items[id];
            (!item.span.is_all_day_like(), item.span.start)
        });
        let date = first + Days::new(cell as u64);
        for (slot, &id) in ids.iter().enumerate() {
            let item = &state.items[id];
            let starts_today = item.span.start.with_timezone(&home).date_naive() == date;
            let time = if item.span.is_all_day_like() || !starts_today {
                String::new()
            } else {
                forms::format_time(item.span.start.with_timezone(&home).time())
            };
            chips.push(MonthChip {
                id: id as i32,
                cell: cell as i32,
                slot: slot as i32,
                title: item.title.as_str().into(),
                time: time.into(),
                color: rgb(item.color),
                editable: item.movable(),
                task: item.is_task(),
            });
        }
        cells.push(MonthCell {
            number: date.day().to_string().into(),
            today: date == today,
            other_month: date.month() != state.anchor.month(),
            count: ids.len() as i32,
            journal: journal_days.get(&date).copied().unwrap_or(0),
        });
    }
    let weeks: Vec<SharedString> = (0..6)
        .map(|w| {
            (first + Days::new(w * 7))
                .iso_week()
                .week()
                .to_string()
                .into()
        })
        .collect();
    app.set_month_cells(ModelRc::new(VecModel::from(cells)));
    app.set_month_chips(ModelRc::new(VecModel::from(chips)));
    app.set_month_week_labels(ModelRc::new(VecModel::from(weeks)));
    let today_cell = (today - first).num_days();
    app.set_month_today_cell(if (0..42).contains(&today_cell) {
        today_cell as i32
    } else {
        -1
    });
}

/// Zeilenhöhen der Agenda (agenda-view.slint), um zu heute zu blättern
const AGENDA_DAY_H: f32 = 46.0;
const AGENDA_ITEM_H: f32 = 60.0;
const AGENDA_SKIP_H: f32 = 40.0;

fn load_agenda(app: &AppWindow, state: &State, first: NaiveDate, days: i64, keep_order: bool) {
    let home = state.home;
    let today = state.now().date();
    let last = first + Days::new(days.max(1) as u64 - 1);
    let mut y = 0.0;
    let mut target = None;
    let mut lines = Vec::new();
    for row in overview::agenda_rows(&state.items, first, last, today, home) {
        let line = match row {
            overview::AgendaRow::Day { date, today, empty } => {
                if target.is_none() && date >= state.now().date() {
                    target = Some(y);
                }
                y += AGENDA_DAY_H;
                let (weekday, label) = overview::day_label(date);
                AgendaLine {
                    kind: 0,
                    label: weekday.into(),
                    date_text: label.into(),
                    today,
                    empty,
                    day: (date - first).num_days() as i32,
                    ..AgendaLine::default()
                }
            }
            overview::AgendaRow::Item { item, date } => {
                y += AGENDA_ITEM_H;
                let it = &state.items[item];
                let (time, sub) = overview::item_texts(it, date, home);
                AgendaLine {
                    kind: 1,
                    day: (date - first).num_days() as i32,
                    id: item as i32,
                    time: time.into(),
                    title: it.title.as_str().into(),
                    sub: sub.into(),
                    color: rgb(it.color),
                    task: it.is_task(),
                    ..AgendaLine::default()
                }
            }
            overview::AgendaRow::Skip {
                first: from,
                last: to,
            } => {
                if target.is_none() && to >= state.now().date() {
                    target = Some(y);
                }
                y += AGENDA_SKIP_H;
                AgendaLine {
                    kind: 2,
                    label: overview::skip_label(from, to).into(),
                    day: (from - first).num_days() as i32,
                    ..AgendaLine::default()
                }
            }
        };
        lines.push(line);
    }
    app.set_agenda_rows(ModelRc::new(VecModel::from(lines)));
    // Beim Blättern oben bzw. bei heute beginnen; Neuladen hält die Stelle
    if !keep_order {
        let at = if (first..=last).contains(&today) {
            target.unwrap_or(0.0)
        } else {
            0.0
        };
        // Erst nach dem Layout: vorher kennt die Liste ihre Höhe noch nicht
        let weak = app.as_weak();
        slint::Timer::single_shot(Duration::from_millis(30), move || {
            if let Some(app) = weak.upgrade() {
                app.invoke_agenda_scroll(at);
            }
        });
    }
}

fn load_year(app: &AppWindow, state: &State, first: NaiveDate, keep_order: bool) {
    let home = state.home;
    let today = state.now().date();
    let year = first.year();
    let counts = overview::day_counts(&state.items, home);
    let months: Vec<YearMonth> = overview::year_grid(year, &counts)
        .into_iter()
        .enumerate()
        .map(|(m, cells)| {
            let start = NaiveDate::from_ymd_opt(year, m as u32 + 1, 1).unwrap_or(first);
            YearMonth {
                name: dates::capitalize(dates::month(m)).into(),
                start: start.ordinal0() as i32,
                days: ModelRc::new(VecModel::from(
                    cells
                        .into_iter()
                        .map(|c| YearCell {
                            number: c.date.map_or(0, |d| d.day() as i32),
                            count: c.count as i32,
                            today: c.date == Some(today),
                        })
                        .collect::<Vec<_>>(),
                )),
            }
        })
        .collect();
    app.set_year_months(ModelRc::new(VecModel::from(months)));
    if !keep_order {
        let cursor = if today.year() == year {
            today
        } else {
            state.anchor
        };
        app.set_year_cursor(cursor.ordinal0() as i32);
    }
}

fn load_mini(app: &AppWindow, state: &State) {
    let first = monday_of(state.mini_month);
    let today = state.now().date();
    let days: Vec<MiniDay> = (0..42)
        .map(|i| {
            let date = first + Days::new(i);
            MiniDay {
                number: date.day().to_string().into(),
                today: date == today,
                other_month: date.month() != state.mini_month.month(),
                selected: date == state.anchor,
            }
        })
        .collect();
    app.set_mini_month(dates::capitalize(dates::month(state.mini_month.month0() as usize)).into());
    app.set_mini_year(state.mini_month.year().to_string().into());
    app.set_mini_days(ModelRc::new(VecModel::from(days)));
}

/// Termine der Ansicht neu sammeln und anzeigen. `keep_order` beim Neuladen
/// derselben Ansicht (Sync, Bearbeiten), sonst zeitlich sortiert.
fn load_view(app: &AppWindow, state: &mut State, keep_order: bool) {
    let (first, days) = state.range();
    state.items = items::collect(state, first, days);
    app.set_view(state.view.index());
    if state.view == View::Tasks {
        app.set_month_title(tr!("Tasks").into());
    } else if state.view == View::Journal {
        app.set_month_title(journal_ui::title(state).into());
    } else {
        app.set_month_title(view_title(state.view, state.anchor, first, days).into());
    }
    match state.view {
        View::Tasks => load_tasks(app, state),
        View::Journal => journal_ui::load(app, state),
        View::Month => load_month(app, state, first),
        View::Year => load_year(app, state, first, keep_order),
        View::Agenda => load_agenda(app, state, first, days, keep_order),
        View::Day | View::ThreeDays | View::Week => load_week(app, state, first, days, keep_order),
    }
    load_mini(app, state);
    update_now(app, state);
}

fn update_now(app: &AppWindow, state: &State) {
    let now = state.now();
    let (first, days) = state.range();
    let day = (now.date() - first).num_days();
    let columns = matches!(state.view, View::Day | View::ThreeDays | View::Week);
    app.set_today_index(if (0..days).contains(&day) && columns {
        day as i32
    } else {
        -1
    });
    app.set_now_min(minutes_of(now));
}

fn update_sync_status(app: &AppWindow, state: &State) {
    let p = state.pending;
    let (code, text, detail) = if state.accounts.is_empty() {
        (0, String::new(), String::new())
    } else if state.sync.running {
        (2, tr!("Syncing…").to_string(), String::new())
    } else if p.conflicts > 0 {
        (
            3,
            trn!(
                "{n} conflict while saving",
                "{n} conflicts while saving",
                p.conflicts
            ),
            String::new(),
        )
    } else if p.failed > 0 {
        (
            3,
            trn!(
                "{n} change rejected by the server",
                "{n} changes rejected by the server",
                p.failed
            ),
            state.sync.errors.join("\n"),
        )
    } else if let Some(first) = state.sync.errors.first() {
        let more = state.sync.errors.len() - 1;
        let text = if more > 0 {
            tr!("{} (+{} more)", first, more)
        } else {
            first.clone()
        };
        (3, text, state.sync.errors.join("\n"))
    } else if p.waiting > 0 {
        (
            2,
            trn!(
                "{n} change waiting to upload",
                "{n} changes waiting to upload",
                p.waiting
            ),
            String::new(),
        )
    } else if let Some(last) = state.sync.last {
        (
            1,
            tr!("Synced {}", dates::clock(last.hour(), last.minute())),
            String::new(),
        )
    } else {
        (1, tr!("Not synced yet").to_string(), String::new())
    };
    app.set_sync_state(code);
    app.set_sync_text(text.into());
    app.set_sync_detail(detail.into());
}

/// Ersten offenen Konflikt anzeigen, sofern kein Dialog offen ist.
fn check_conflicts(app: &AppWindow, state: &mut State) {
    if app.get_conflict_open()
        || app.get_event_dialog_open()
        || app.get_confirm_open()
        || app.get_scope_open()
        || app.get_settings_open()
        || app.get_task_dialog_open()
        || app.get_quick_open()
        || app.get_palette_open()
    {
        return;
    }
    let ops = state.store.pending_ops().unwrap_or_default();
    let Some(op) = ops
        .iter()
        .find(|o| o.state == OpState::Conflict && !state.dismissed_conflicts.contains(&o.id))
    else {
        return;
    };
    let title = op
        .ics
        .as_deref()
        .and_then(|ics| hora_core::model::CalendarObject::parse(ics).ok())
        .and_then(|o| o.events().0.into_iter().next())
        .map(|e| e.summary)
        .unwrap_or_else(|| tr!("An event").into());
    state.conflict = Some(op.id);
    app.set_conflict_message(
        if op.operation == hora_storage::Operation::Delete {
            tr!(
                "“{}” was changed on the server before your deletion was uploaded. Which version should apply?",
                title
            )
        } else {
            tr!(
                "“{}” was changed on the server before your change was uploaded. Which version should apply?",
                title
            )
        }
        .into(),
    );
    app.set_conflict_open(true);
}

/// Seitenleiste, Ansicht, Sync-Anzeige und Konflikte neu aufbauen.
fn refresh(app: &AppWindow, state: &mut State, keep_order: bool) {
    state.planner.dirty = true;
    state.search_index = None;
    app.set_calendars(ModelRc::new(VecModel::from(calendar_rows(state))));
    app.set_has_accounts(!state.accounts.is_empty());
    state.pending = state.store.pending_summary().unwrap_or_default();
    load_view(app, state, keep_order);
    update_sync_status(app, state);
    check_conflicts(app, state);
}

fn apply_update(app: &AppWindow, state: &mut State, update: Update) {
    match update {
        Update::AccountAdded => {
            let first_account = state.accounts.is_empty();
            app.set_add_account_busy(false);
            app.set_add_account_error("".into());
            app.set_add_account_open(false);
            app.invoke_focus_main();
            state.reload_calendars();
            // Mit dem ersten echten Konto verschwinden die Beispieltermine
            if first_account && state.demo_visible {
                state.demo_visible = false;
                let _ = state.store.set_setting("demo_visible", "0");
            }
            refresh(app, state, false);
        }
        Update::AddAccountFailed(message) => {
            app.set_add_account_busy(false);
            app.set_add_account_error(message.into());
        }
        Update::SyncStarted => {
            state.sync.running = true;
            update_sync_status(app, state);
        }
        Update::DataChanged => {
            state.reload_calendars();
            refresh(app, state, true);
        }
        Update::CalendarCreated(error) => {
            state.journal_create_busy = false;
            state.journal_create_error = error;
            if state.view == View::Journal {
                journal_ui::load(app, state);
            }
        }
        Update::SyncFinished { errors } => {
            state.sync.running = false;
            state.sync.last = Some(state.now());
            state.sync.errors = errors;
            state.pending = state.store.pending_summary().unwrap_or_default();
            update_sync_status(app, state);
            check_conflicts(app, state);
        }
    }
}

fn dispatch(update: Update) {
    UI.with(|ui| {
        if let Some((weak, state)) = ui.borrow().as_ref()
            && let Some(app) = weak.upgrade()
        {
            apply_update(&app, &mut state.borrow_mut(), update);
        }
    });
}

/// Vorschau-Inhalt für einen Termin.
fn details(state: &mut State, item: &Item) -> EventDetails {
    let home = state.home;
    let local = |t: chrono::DateTime<Utc>| t.with_timezone(&home).naive_local();
    let when = match item.span.dates {
        Some((first, after)) => forms::describe(
            first.and_time(NaiveTime::MIN),
            after.and_time(NaiveTime::MIN),
            true,
        ),
        None => forms::describe(local(item.span.start), local(item.span.end), false),
    };
    let calendar = match &item.source {
        Source::Server { calendar_id, .. } => state
            .calendars
            .iter()
            .find(|c| c.id == *calendar_id)
            .map(|c| {
                let account = state
                    .accounts
                    .iter()
                    .find(|a| a.id == c.account_id)
                    .map_or("", |a| a.name.as_str());
                format!("{} · {}", c.name, account)
            })
            .unwrap_or_default(),
        _ => tr!("Sample events (only on this device)").to_string(),
    };
    let rule = editing::series_rule(state, item)
        .map(|(rule, start)| repeat::describe_rule(&rule, &start, state.home));
    let note = match (&item.source, rule) {
        (
            Source::Server {
                read_only: true, ..
            },
            _,
        ) => tr!("This calendar is read-only.").to_string(),
        (_, Some(rule)) => rule,
        _ => String::new(),
    };
    EventDetails {
        title: item.title.as_str().into(),
        when: when.into(),
        location: item.location.as_str().into(),
        calendar: calendar.into(),
        color: rgb(item.color),
        description: editing::description(state, item).into(),
        note: note.into(),
        can_edit: item.writable(),
        can_delete: item.writable(),
    }
}

/// Termin-Dialog öffnen: `item` zum Bearbeiten, sonst neu mit `times`.
fn open_dialog(app: &AppWindow, state: &mut State, item: Option<Item>, times: DraftTimes) {
    let options = calendar_options(state);
    let (title, location, description, calendar, rule) = match &item {
        Some(item) => {
            let calendar = match &item.source {
                Source::Server { calendar_id, .. } => *calendar_id,
                _ => -1,
            };
            let rule = editing::series_rule(state, item)
                .map(|(rule, start)| repeat::RuleFields::from_rule(Some(&rule), &start, state.home))
                .unwrap_or_default();
            (
                item.title.clone(),
                item.location.clone(),
                editing::description(state, item),
                calendar,
                rule,
            )
        }
        None => {
            let preferred = state
                .store
                .setting("default_calendar")
                .ok()
                .flatten()
                .and_then(|v| v.parse::<i64>().ok());
            let calendar = preferred
                .filter(|id| state.option_ids.contains(id))
                .or_else(|| state.option_ids.first().copied())
                .unwrap_or(-1);
            (
                String::new(),
                String::new(),
                String::new(),
                calendar,
                repeat::RuleFields::default(),
            )
        }
    };
    let index = state
        .option_ids
        .iter()
        .position(|id| *id == calendar)
        .unwrap_or(0);
    app.set_calendar_options(ModelRc::new(VecModel::from(options)));
    let chips = dialog_reminders(state, item.as_ref(), times.all_day);
    state.reminder_rows.set_vec(chips.clone());
    state.reminders_before = chips;
    app.set_event_draft(EventDraft {
        is_new: item.is_none(),
        title: title.into(),
        all_day: times.all_day,
        start_date: times.start_date.into(),
        start_time: times.start_time.into(),
        end_date: times.end_date.into(),
        end_time: times.end_time.into(),
        location: location.into(),
        description: description.into(),
        calendar: index as i32,
        custom_rule: rule.repeat == repeat::CUSTOM,
        rule: rule_draft(&rule),
        calendar_locked: item.is_some(),
        can_delete: item.as_ref().is_some_and(Item::writable),
        note: "".into(),
    });
    app.set_event_error("".into());
    state.editing = item;
    app.set_event_dialog_open(true);
}

/// Erinnerungen für den Dialog: die des Termins bzw. die Standarderinnerung
fn dialog_reminders(state: &mut State, item: Option<&Item>, all_day: bool) -> Vec<ReminderChip> {
    let home = state.home;
    match item {
        Some(item) => editing::alarms(state, item)
            .iter()
            .filter(|a| a.notifies())
            .map(|a| match a.minutes_before() {
                Some(m) => ReminderChip {
                    label: reminders::label(m, all_day).into(),
                    minutes: m as i32,
                    keep: -1,
                },
                None => ReminderChip {
                    label: reminders::alarm_label(a, all_day, home).into(),
                    minutes: 0,
                    keep: a.index as i32,
                },
            })
            .collect(),
        None if !all_day => state
            .settings
            .default_reminder
            .map(|m| ReminderChip {
                label: reminders::label(m, false).into(),
                minutes: m as i32,
                keep: -1,
            })
            .into_iter()
            .collect(),
        None => Vec::new(),
    }
}

/// Erinnerungen aus dem Dialog; `None`, wenn ein bestehender Termin sie behält
fn reminder_input(state: &State) -> Option<ReminderInput> {
    let rows: Vec<ReminderChip> = state.reminder_rows.iter().collect();
    if state.editing.is_some() && rows == state.reminders_before {
        return None;
    }
    Some(ReminderInput {
        minutes: rows
            .iter()
            .filter(|c| c.keep < 0)
            .map(|c| i64::from(c.minutes))
            .collect(),
        keep: rows
            .iter()
            .filter(|c| c.keep >= 0)
            .map(|c| c.keep as usize)
            .collect(),
    })
}

fn reminder_presets(minutes: &[i64], all_day: bool) -> ModelRc<SelectOption> {
    let options: Vec<SelectOption> = minutes
        .iter()
        .map(|m| SelectOption {
            id: *m as i32,
            name: reminders::label(*m, all_day).into(),
            color: Color::default(),
        })
        .collect();
    ModelRc::new(VecModel::from(options))
}

/// Auswahl der Standarderinnerung: "None" und die Vorgaben
fn default_reminder_options() -> ModelRc<SelectOption> {
    let mut options = vec![SelectOption {
        id: -1,
        name: tr!("No reminder").into(),
        color: Color::default(),
    }];
    options.extend(reminders::TIMED.iter().map(|m| SelectOption {
        id: *m as i32,
        name: reminders::label(*m, false).into(),
        color: Color::default(),
    }));
    ModelRc::new(VecModel::from(options))
}

fn default_reminder_index(minutes: Option<i64>) -> i32 {
    minutes
        .and_then(|m| reminders::TIMED.iter().position(|t| *t == m))
        .map_or(0, |i| i as i32 + 1)
}

/// Fällige Erinnerungen zeigen (oder bei ausgeschalteten Erinnerungen still
/// als erledigt merken, damit nach dem Einschalten keine Flut kommt).
fn check_reminders(state: &mut State) {
    let now = Utc::now();
    let mut planner = std::mem::take(&mut state.planner);
    let due = planner.take_due(now, |from, to| reminders::plan(state, from, to));
    state.planner = planner;
    for p in due {
        if state.store.reminder_shown(&p.key).unwrap_or(true) {
            continue;
        }
        if state.settings.reminders
            && let Some(shell) = &state.shell
        {
            let toast = reminders::toast_for(&p, now, state.home);
            if let Err(e) = shell.show(&toast, |launch| {
                let _ = slint::invoke_from_event_loop(move || on_toast(launch));
            }) {
                log::write(&format!("Erinnerung nicht gezeigt: {e}"));
                continue;
            }
            state.shown_toasts.push((p.tag(), p.title.clone()));
            if state.shown_toasts.len() > 50 {
                state.shown_toasts.remove(0);
            }
        }
        let _ = state.store.mark_reminder_shown(&p.key, p.at.timestamp());
    }
}

/// Fenster und Zustand, wenn ein Ereignis von außen kommt (Infobereich, Toast)
fn with_ui(f: impl FnOnce(&AppWindow, &Rc<RefCell<State>>)) {
    UI.with(|ui| {
        if let Some((weak, state)) = ui.borrow().as_ref()
            && let Some(app) = weak.upgrade()
        {
            f(&app, state);
        }
    });
}

fn on_tray(event: hora_platform::tray::TrayEvent) {
    use hora_platform::tray::TrayEvent;
    with_ui(|app, state| match event {
        TrayEvent::Open | TrayEvent::Menu(shell::MENU_OPEN) => shell::show_window(app),
        TrayEvent::Menu(shell::MENU_SYNC) => {
            let s = state.borrow();
            if !s.accounts.is_empty() {
                s.send(Command::SyncAll);
            }
        }
        TrayEvent::Menu(shell::MENU_SETTINGS) => {
            shell::show_window(app);
            open_settings(app, &state.borrow());
        }
        TrayEvent::Menu(shell::MENU_PAUSE) => {
            activity_ui::toggle_pause(app, &mut state.borrow_mut());
        }
        TrayEvent::Menu(shell::MENU_QUIT) | TrayEvent::Quit => {
            let _ = slint::quit_event_loop();
        }
        TrayEvent::Menu(_) => {}
    });
}

/// Klick auf eine Erinnerung: Fenster zeigen, zum Tag des Termins springen
fn on_toast(launch: String) {
    with_ui(|app, state| {
        shell::show_window(app);
        // Aufgabe: zur Aufgaben-Ansicht
        if launch.starts_with("task|") {
            let mut s = state.borrow_mut();
            s.view = View::Tasks;
            load_view(app, &mut s, false);
            return;
        }
        let start = launch
            .split('|')
            .nth(2)
            .and_then(|t| t.parse::<i64>().ok())
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0));
        if let Some(start) = start {
            let mut s = state.borrow_mut();
            let local = start.with_timezone(&s.home).naive_local();
            s.anchor = local.date();
            s.mini_month = s.anchor.with_day(1).unwrap_or(s.anchor);
            load_view(app, &mut s, false);
            app.invoke_scroll_to_minute((minutes_of(local) - 60).max(0));
        }
    });
}

fn open_settings(app: &AppWindow, state: &State) {
    app.set_settings_language(language_index(&language_choice(&state.store)));
    app.set_settings_formats(format_index(&format_choice(&state.store)));
    app.set_settings_reminders(state.settings.reminders);
    app.set_settings_default_reminder(default_reminder_index(state.settings.default_reminder));
    app.set_settings_keep_running(state.settings.keep_running);
    app.set_settings_show_tasks(state.settings.show_tasks);
    app.set_settings_task_reminders(state.settings.task_reminders);
    app.set_settings_autostart(shell::autostart_enabled());
    app.set_settings_update_check(state.settings.update_check);
    activity_ui::show(app, state);
    updates::show(app, state);
    app.set_settings_status(
        state
            .shell
            .as_ref()
            .map(shell::Shell::status)
            .unwrap_or_default()
            .into(),
    );
    app.set_settings_open(true);
}

/// Die Aufgabe hinter einem Eintrag in Woche oder Monat
fn task_of_item(state: &State, item: &Item) -> Option<tasks::TaskItem> {
    let Source::Task {
        calendar_id,
        href,
        uid,
    } = &item.source
    else {
        return None;
    };
    state
        .tasks
        .iter()
        .find(|t| t.calendar_id == *calendar_id && &t.href == href && &t.uid == uid && !t.done)
        .cloned()
}

/// Dieselbe Aufgabe (bzw. dasselbe Vorkommen), auch nach neuem Einlesen
fn same_task(a: &tasks::TaskItem, b: &tasks::TaskItem) -> bool {
    a.calendar_id == b.calendar_id
        && a.href == b.href
        && a.uid == b.uid
        && a.occurrence.as_ref().map(|o| o.recurrence_id)
            == b.occurrence.as_ref().map(|o| o.recurrence_id)
}

/// Aufgaben-Ansicht neu aufbauen
fn load_tasks(app: &AppWindow, state: &mut State) {
    state.tasks = tasks::collect(state);
    let now = Utc::now();
    let home = state.home;
    let rows = tasks::rows(
        &state.tasks,
        state.task_filter,
        &state.collapsed_tasks,
        now,
        home,
    );
    let model: Vec<TaskRow> = rows
        .into_iter()
        .map(|r| match r.header {
            Some((group, count)) => TaskRow {
                header: true,
                title: group.title().into(),
                count: count as i32,
                overdue: group == tasks::Group::Overdue,
                ..TaskRow::default()
            },
            None => {
                let t = &state.tasks[r.item];
                let title = if t.info.summary.trim().is_empty() {
                    tr!("(untitled)")
                } else {
                    t.info.summary.trim()
                };
                TaskRow {
                    header: false,
                    title: title.into(),
                    count: 0,
                    overdue: tasks::overdue(t, now, home),
                    id: r.item as i32,
                    note: t
                        .info
                        .description
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .into(),
                    due: t
                        .due
                        .map(|d| tasks::due_label(&d, now, home))
                        .unwrap_or_default()
                        .into(),
                    calendar: t.calendar_name.as_str().into(),
                    color: rgb(t.color),
                    done: t.done,
                    depth: r.depth as i32,
                    has_children: r.has_children,
                    collapsed: r.collapsed,
                    children: r.children.into(),
                    priority: tasks::priority_index(t.info.priority),
                    recurring: t.recurring,
                    context: r.context.into(),
                }
            }
        })
        .collect();
    app.set_task_rows(ModelRc::new(VecModel::from(model)));
    app.set_task_active_count(state.tasks.iter().filter(|t| !t.done).count() as i32);
    app.set_task_done_count(state.tasks.iter().filter(|t| t.done).count() as i32);
    app.set_task_filter(state.task_filter.index());
    app.set_task_can_add(tasks::default_calendar(state).is_some());
}

/// Aufgaben-Dialog: `item` bearbeiten, sonst neu (als Teilaufgabe von `parent`)
fn open_task_dialog(
    app: &AppWindow,
    state: &mut State,
    item: Option<tasks::TaskItem>,
    parent: Option<String>,
) {
    let home = state.home;
    let ids = tasks::task_calendars(state);
    let options: Vec<SelectOption> = ids
        .iter()
        .filter_map(|id| {
            state
                .calendars
                .iter()
                .enumerate()
                .find(|(_, c)| c.id == *id)
        })
        .map(|(i, c)| {
            let account = state
                .accounts
                .iter()
                .find(|a| a.id == c.account_id)
                .map_or("", |a| a.name.as_str());
            SelectOption {
                id: c.id as i32,
                name: format!("{} · {}", c.name, account).into(),
                color: rgb(calendar_color(c, i)),
            }
        })
        .collect();
    let parent_item = parent
        .as_ref()
        .and_then(|p| state.tasks.iter().find(|t| &t.uid == p))
        .cloned();
    let calendar = item
        .as_ref()
        .map(|t| t.calendar_id)
        .or(parent_item.as_ref().map(|p| p.calendar_id))
        .or_else(|| tasks::default_calendar(state));
    // Übergeordnete Aufgabe: offene, nicht wiederkehrende, nicht die eigene oder eine ihrer Teilaufgaben
    let own = item
        .as_ref()
        .and_then(|t| state.tasks.iter().position(|x| same_task(x, t)));
    let mut excluded: HashSet<String> = HashSet::new();
    if let Some(own) = own {
        excluded.insert(state.tasks[own].uid.clone());
        excluded.extend(
            tasks::descendants(&state.tasks, own)
                .into_iter()
                .map(|d| state.tasks[d].uid.clone()),
        );
    }
    let current_parent = item
        .as_ref()
        .and_then(|t| t.info.parent.clone())
        .or(parent.clone());
    let mut parent_ids: Vec<Option<String>> = vec![None];
    let mut parent_names: Vec<SelectOption> = vec![SelectOption {
        id: 0,
        name: tr!("No parent task").into(),
        color: Color::default(),
    }];
    for t in &state.tasks {
        let wanted = (!t.done && !t.recurring) || current_parent.as_deref() == Some(t.uid.as_str());
        if wanted && !excluded.contains(&t.uid) && !parent_ids.contains(&Some(t.uid.clone())) {
            parent_ids.push(Some(t.uid.clone()));
            parent_names.push(SelectOption {
                id: parent_names.len() as i32,
                name: format!("{} · {}", t.info.summary, t.calendar_name).into(),
                color: Color::default(),
            });
        }
    }
    let parent_index = parent_ids
        .iter()
        .position(|p| *p == current_parent)
        .unwrap_or(0);
    let (due_mode, due_date, due_time, start_date) = match &item {
        Some(t) => tasks::draft_due(t, home),
        None => (0, String::new(), "18:00".into(), String::new()),
    };
    let note = match &item {
        Some(t) if t.recurring => {
            let rule = t
                .info
                .rrule
                .as_deref()
                .zip(t.info.start.or(t.info.due))
                .map(|(r, start)| repeat::describe_rule(r, &start, home))
                .unwrap_or_default();
            tr!(
                "Recurring task: {}. Completed {} times so far. The due date applies to the series.",
                rule,
                t.completed_count
            )
        }
        Some(t) if t.read_only => tr!("This calendar is read-only.").into(),
        _ => String::new(),
    };
    let writable = item.as_ref().is_some_and(|t| !t.read_only);
    app.set_task_calendar_options(ModelRc::new(VecModel::from(options)));
    app.set_task_parent_options(ModelRc::new(VecModel::from(parent_names)));
    app.set_task_draft(TaskDraft {
        is_new: item.is_none(),
        title: item
            .as_ref()
            .map(|t| t.info.summary.clone())
            .unwrap_or_default()
            .into(),
        due_mode,
        due_date: due_date.into(),
        due_time: due_time.into(),
        start_date: start_date.into(),
        priority: item
            .as_ref()
            .map_or(0, |t| tasks::priority_index(t.info.priority)),
        calendar: calendar
            .and_then(|c| ids.iter().position(|id| *id == c))
            .unwrap_or(0) as i32,
        parent: parent_index as i32,
        description: item
            .as_ref()
            .map(|t| t.info.description.clone())
            .unwrap_or_default()
            .into(),
        calendar_locked: item.is_some() || parent.is_some(),
        can_delete: writable,
        can_add_subtask: writable && item.as_ref().is_some_and(|t| !t.recurring && !t.done),
        times_locked: item.as_ref().is_some_and(|t| t.recurring),
        note: note.into(),
    });
    app.set_task_error("".into());
    state.task_calendar_ids = ids;
    state.task_parent_ids = parent_ids;
    state.editing_task = item;
    app.set_task_dialog_open(true);
}

fn ask_delete_task(app: &AppWindow, state: &mut State, task: tasks::TaskItem) {
    let index = state.tasks.iter().position(|t| same_task(t, &task));
    let subtasks = index.map_or(0, |i| tasks::descendants(&state.tasks, i).len());
    let mut message = tr!("“{}” will be deleted.", task.info.summary);
    if subtasks > 0 {
        message.push(' ');
        message.push_str(&trn!(
            "So will its subtask.",
            "So will its {n} subtasks.",
            subtasks
        ));
    }
    if task.recurring {
        message.push(' ');
        message.push_str(tr!("The whole series will be deleted."));
    }
    app.set_confirm_title(tr!("Delete task?").into());
    app.set_confirm_message(message.into());
    state.deleting_task = Some(task);
    app.set_confirm_open(true);
}

fn ask_delete(app: &AppWindow, state: &mut State, item: Item) {
    if item.recurring() {
        // Der Serien-Dialog ist zugleich die Bestätigung
        ask_scope(app, state, SeriesAction::Delete { item }, true);
        return;
    }
    app.set_confirm_title(tr!("Delete event?").into());
    app.set_confirm_message(tr!("“{}” will be deleted.", item.title).into());
    state.deleting = Some(item);
    app.set_confirm_open(true);
}

/// Serien-Dialog öffnen; die Änderung wartet in `state.series_action`.
fn ask_scope(app: &AppWindow, state: &mut State, action: SeriesAction, allow_this: bool) {
    app.set_scope_deleting(matches!(action, SeriesAction::Delete { .. }));
    app.set_scope_allow_this(allow_this);
    state.series_action = Some(action);
    app.set_scope_open(true);
}

/// Felder der Wiederholung für den Dialog
fn rule_draft(rule: &repeat::RuleFields) -> RuleDraft {
    RuleDraft {
        repeat: rule.repeat,
        interval: rule.interval.as_str().into(),
        weekdays: rule.weekdays,
        monthly: rule.monthly,
        end: rule.end,
        until: rule.until.as_str().into(),
        count: rule.count.as_str().into(),
    }
}

fn rule_fields(draft: &RuleDraft) -> repeat::RuleFields {
    repeat::RuleFields {
        repeat: draft.repeat,
        interval: draft.interval.to_string(),
        weekdays: draft.weekdays,
        monthly: draft.monthly,
        end: draft.end,
        until: draft.until.to_string(),
        count: draft.count.to_string(),
    }
}

/// Schreibt Startzeiten (ab Prozessstart) in die Datei aus `HORA_MEASURE`:
/// Beginn von `main`, erster Durchlauf der Ereignisschleife und, sofern der
/// Renderer es meldet, das erste fertig gezeichnete Bild. Software-Renderer
/// und Skia melden Letzteres nicht. Die Datei entsteht drei Sekunden nach
/// dem Start.
fn install_startup_probe(app: &AppWindow, at_main: Option<Duration>) {
    let Ok(path) = std::env::var("HORA_MEASURE") else {
        return;
    };
    let uptime_ms = || hora_platform::process_uptime().map(|d| d.as_secs_f64() * 1000.0);
    let event_loop = Rc::new(Cell::new(None::<f64>));
    let first_frame = Rc::new(Cell::new(None::<f64>));

    let frame = first_frame.clone();
    let _ = app.window().set_rendering_notifier(move |state, _| {
        if matches!(state, slint::RenderingState::AfterRendering) && frame.get().is_none() {
            frame.set(uptime_ms());
        }
    });
    let looped = event_loop.clone();
    slint::Timer::single_shot(Duration::ZERO, move || looped.set(uptime_ms()));

    slint::Timer::single_shot(Duration::from_secs(3), move || {
        let ms = |v: Option<f64>| v.map_or("-".to_string(), |ms| format!("{ms:.1}"));
        let line = format!(
            "backend={} main_ms={} event_loop_ms={} first_frame_ms={}\n",
            std::env::var("SLINT_BACKEND").unwrap_or_else(|_| "auto".into()),
            ms(at_main.map(|d| d.as_secs_f64() * 1000.0)),
            ms(event_loop.get()),
            ms(first_frame.get()),
        );
        let _ = std::fs::write(&path, line);
    });
}

/// Renderer wählen, bevor das erste Fenster entsteht. Standard ist der
/// Software-Renderer: im Vergleich der sparsamste (rund 9 MB privater
/// Arbeitsspeicher, Skia gut 70 MB) und unabhängig von OpenGL-Treibern, die
/// z. B. per Remotedesktop fehlen; femtovg beendet sich dort ohne Meldung.
/// `SLINT_BACKEND` hat Vorrang, etwa `winit-femtovg`.
fn select_renderer() -> Result<(), slint::PlatformError> {
    if std::env::var_os("SLINT_BACKEND").is_some() {
        return Ok(());
    }
    let selector = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into());
    // Rahmenloses Fenster (eigene Titelleiste) mit dem Schatten von Windows
    #[cfg(windows)]
    let selector = selector.with_winit_window_attributes_hook(|attributes| {
        use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
        attributes.with_undecorated_shadow(true)
    });
    selector.select()
}

/// Eigene Titelleiste: Verschieben, Größe ziehen, Minimieren, Maximieren,
/// Schließen (wie das X von Windows, also ggf. in den Infobereich)
fn install_window_controls(app: &AppWindow) {
    use slint::winit_030::{WinitWindowAccessor, winit::window::ResizeDirection};
    let weak = app.as_weak();
    app.on_window_drag(move || {
        if let Some(app) = weak.upgrade() {
            app.window().with_winit_window(|w| w.drag_window().ok());
        }
    });
    let weak = app.as_weak();
    app.on_window_resize(move |direction| {
        let direction = match direction {
            0 => ResizeDirection::North,
            1 => ResizeDirection::South,
            2 => ResizeDirection::West,
            3 => ResizeDirection::East,
            4 => ResizeDirection::NorthWest,
            5 => ResizeDirection::NorthEast,
            6 => ResizeDirection::SouthWest,
            _ => ResizeDirection::SouthEast,
        };
        if let Some(app) = weak.upgrade() {
            app.window()
                .with_winit_window(|w| w.drag_resize_window(direction).ok());
        }
    });
    let weak = app.as_weak();
    app.on_window_minimize(move || {
        if let Some(app) = weak.upgrade() {
            app.window().set_minimized(true);
        }
    });
    let weak = app.as_weak();
    app.on_window_toggle_maximize(move || {
        if let Some(app) = weak.upgrade() {
            let maximized = !app.window().is_maximized();
            app.window().set_maximized(maximized);
            app.set_window_maximized(maximized);
        }
    });
    let weak = app.as_weak();
    app.on_window_close(move || {
        if let Some(app) = weak.upgrade() {
            let _ = app
                .window()
                .dispatch_event_with_result(slint::platform::WindowEvent::CloseRequested);
        }
    });
    let weak = app.as_weak();
    app.on_window_changed(move || {
        if let Some(app) = weak.upgrade() {
            app.set_window_maximized(app.window().is_maximized());
        }
    });
}

/// Zeitzone des Systems, sonst Europe/Berlin.
fn home_zone() -> Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|name| name.parse().ok())
        .unwrap_or(chrono_tz::Europe::Berlin)
}

/// `%LOCALAPPDATA%\Hora`, für Tests mit `HORA_DATA_DIR` umlenkbar.
fn data_dir() -> PathBuf {
    std::env::var_os("HORA_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("Hora")
        })
}

/// Callback mit Zugriff auf Fenster und Zustand
fn with<F>(app: &AppWindow, state: &Rc<RefCell<State>>, f: F) -> impl Fn() + 'static
where
    F: Fn(&AppWindow, &mut State) + 'static,
{
    let weak = app.as_weak();
    let state = state.clone();
    move || {
        if let Some(app) = weak.upgrade() {
            f(&app, &mut state.borrow_mut());
        }
    }
}

/// Wie [`with`], für Rückrufe mit einem Argument
fn with_arg<T, F>(app: &AppWindow, state: &Rc<RefCell<State>>, f: F) -> impl Fn(T) + 'static
where
    F: Fn(&AppWindow, &mut State, T) + 'static,
{
    let weak = app.as_weak();
    let state = state.clone();
    move |arg| {
        if let Some(app) = weak.upgrade() {
            f(&app, &mut state.borrow_mut(), arg);
        }
    }
}

/// Treffer der Befehlspalette anzeigen; Auswahl auf dem ersten Eintrag
fn palette_query(app: &AppWindow, state: &mut State, query: &str) {
    let entries = palette::entries(state, query);
    let mut rows = Vec::new();
    let mut y = 0.0f32;
    let mut group = "";
    for (id, entry) in entries.iter().enumerate() {
        if entry.group != group {
            group = entry.group;
            rows.push(PaletteItem {
                header: true,
                title: entry.group.into(),
                id: -1,
                y,
                ..PaletteItem::default()
            });
            y += 30.0;
        }
        let task = matches!(entry.action, palette::Action::OpenTask { .. })
            || (matches!(entry.action, palette::Action::Create(_))
                && entry.title.starts_with(tr!("Task: {}", "").as_str()));
        rows.push(PaletteItem {
            header: false,
            title: entry.title.as_str().into(),
            detail: entry.detail.as_str().into(),
            badge: entry.badge.as_str().into(),
            badge_color: entry.badge_color.map(rgb).unwrap_or_default(),
            dot: entry.badge_color.is_some(),
            ring: task,
            shortcut: entry.shortcut.as_str().into(),
            id: id as i32,
            y,
        });
        y += 44.0;
    }
    let first = rows.iter().position(|r| !r.header).map_or(-1, |i| i as i32);
    app.set_palette_items(ModelRc::new(VecModel::from(rows)));
    app.set_palette_selected(first);
    state.palette = entries;
}

/// Tag zeigen (Aufgaben wechseln dafür in die Tagesansicht)
fn go_to_day(app: &AppWindow, state: &mut State, day: NaiveDate) {
    state.anchor = day;
    state.mini_month = day.with_day(1).unwrap_or(day);
    if state.view == View::Tasks {
        state.view = View::Day;
    }
    load_view(app, state, false);
}

/// Gewählten Eintrag der Befehlspalette ausführen. Läuft ohne geliehenen
/// Zustand, weil viele Einträge die vorhandenen Rückrufe auslösen.
fn run_palette_action(app: &AppWindow, state: &Rc<RefCell<State>>, action: palette::Action) {
    use palette::Action;
    match action {
        Action::View(view) => app.invoke_set_view(view.index()),
        Action::Today => app.invoke_go_today(),
        Action::Tomorrow => {
            let mut s = state.borrow_mut();
            let day = s.now().date() + Days::new(1);
            go_to_day(app, &mut s, day);
        }
        Action::Prev => app.invoke_prev(),
        Action::Next => app.invoke_next(),
        Action::NewEvent => {
            {
                // Aus der Aufgaben-Ansicht gäbe "neu" eine Aufgabe
                let mut s = state.borrow_mut();
                if s.view == View::Tasks {
                    s.view = View::Week;
                    load_view(app, &mut s, false);
                }
            }
            app.invoke_new_event();
        }
        Action::NewTask => app.invoke_task_new(),
        Action::NewJournal => {
            let mut s = state.borrow_mut();
            if s.view != View::Journal {
                s.view = View::Journal;
                load_view(app, &mut s, false);
            }
            journal_ui::new_entry(app, &mut s);
        }
        Action::OpenJournal { key, day } => {
            let mut s = state.borrow_mut();
            journal_ui::show(app, &mut s, &key, day);
            app.invoke_journal_focus_list();
        }
        Action::Quick => app.invoke_show_quick(),
        Action::Sync => app.invoke_sync_now(),
        Action::Settings => app.invoke_show_settings(),
        Action::AddAccount => app.invoke_show_add_account(),
        Action::About => app.set_about_open(true),
        Action::Theme => {
            // System → hell → dunkel → System
            let theme = app.global::<Theme>();
            let next = (theme.get_scheme_override() + 1).rem_euclid(3);
            theme.set_scheme_override(next);
            let name = ["system", "light", "dark"][next as usize];
            let _ = state.borrow().store.set_setting("theme", name);
        }
        Action::ToggleCalendar(id) => app.invoke_toggle_calendar(id as i32),
        Action::GoTo(day) => go_to_day(app, &mut state.borrow_mut(), day),
        Action::Create(text) => {
            let mut s = state.borrow_mut();
            match quick_add::submit(app, &mut s, &text, false, false) {
                Ok(day) => {
                    if let Some(day) = day {
                        s.anchor = day;
                        s.mini_month = day.with_day(1).unwrap_or(day);
                        if s.view == View::Tasks {
                            s.view = View::Week;
                        }
                    }
                    refresh(app, &mut s, false);
                }
                // Was fehlt, zeigt die Schnelleingabe mit demselben Text
                Err(message) => {
                    app.set_quick_text(text.into());
                    app.set_quick_error(message.into());
                    app.set_quick_open(true);
                }
            }
        }
        Action::OpenEvent { key, start } => {
            let mut s = state.borrow_mut();
            let local = start.with_timezone(&s.home).naive_local();
            if matches!(s.view, View::Year | View::Tasks) {
                s.view = View::Week;
            }
            s.anchor = local.date();
            s.mini_month = local.date().with_day(1).unwrap_or(local.date());
            load_view(app, &mut s, false);
            if matches!(s.view, View::Day | View::ThreeDays | View::Week) {
                app.invoke_scroll_to_minute((minutes_of(local) - 60).max(0));
            }
            let found = s.items.iter().position(|it| it.key() == key);
            if let Some(id) = found {
                let item = s.items[id].clone();
                let details = details(&mut s, &item);
                s.preview = Some(item);
                app.set_preview_details(details);
                let size = app.window().size().to_logical(app.window().scale_factor());
                app.invoke_show_preview(id as i32, size.width / 2.0 - 158.0, 96.0, 0.0);
            }
        }
        Action::OpenTask {
            calendar_id,
            href,
            uid,
        } => {
            let mut s = state.borrow_mut();
            let items = tasks::collect(&mut s);
            let task = items
                .iter()
                .find(|t| t.calendar_id == calendar_id && t.href == href && t.uid == uid)
                .cloned();
            s.tasks = items;
            if let Some(task) = task {
                open_task_dialog(app, &mut s, Some(task), None);
            }
        }
    }
}

/// Gewählte Sprache: HORA_LANGUAGE, sonst die Einstellung, sonst die Wahl im
/// Installer, sonst Englisch. "system" steht für die Windows-Anzeigesprache.
/// Eine neue Wahl im Installer gilt, auch wenn in Hora schon eine Sprache
/// eingestellt war; danach zählt wieder die Einstellung in Hora.
fn adopt_installer_language(store: &Store) {
    let Some(chosen) = hora_platform::locale::installer_language() else {
        return;
    };
    if store
        .setting("installer_language")
        .ok()
        .flatten()
        .as_deref()
        != Some(chosen.as_str())
    {
        let _ = store.set_setting("installer_language", &chosen);
        let _ = store.set_setting("language", &chosen);
    }
}

fn language_choice(store: &Store) -> String {
    std::env::var("HORA_LANGUAGE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| store.setting("language").ok().flatten())
        .or_else(hora_platform::locale::installer_language)
        .unwrap_or_else(|| "en".into())
}

fn resolve_language(choice: &str) -> Lang {
    if choice.eq_ignore_ascii_case("system") {
        return hora_platform::locale::ui_language()
            .as_deref()
            .and_then(Lang::from_code)
            .unwrap_or(Lang::En);
    }
    Lang::from_code(choice).unwrap_or(Lang::En)
}

/// Feste Datums- und Zeitformate (Einstellung "formats"); "windows" folgt der
/// Region in den Windows-Einstellungen
const FORMATS: [(&str, &str, &str); 4] = [
    ("dmy-dot", "dd.MM.yyyy", "HH:mm"),
    ("dmy-slash", "dd/MM/yyyy", "HH:mm"),
    ("mdy", "MM/dd/yyyy", "h:mm tt"),
    ("ymd", "yyyy-MM-dd", "HH:mm"),
];

/// Gewähltes Format: HORA_FORMATS (Tests, Bilder), sonst die Einstellung
fn format_choice(store: &Store) -> String {
    std::env::var("HORA_FORMATS")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| store.setting("formats").ok().flatten())
        .unwrap_or_else(|| "windows".into())
}

fn region_for(choice: &str) -> dates::Region {
    if let Some((_, date, time)) = FORMATS.iter().find(|(code, ..)| *code == choice) {
        return dates::Region::from_patterns(date, time);
    }
    hora_platform::locale::region_formats()
        .map(|(date, time)| dates::Region::from_patterns(&date, &time))
        .unwrap_or(dates::Region::DEFAULT)
}

fn format_index(choice: &str) -> i32 {
    FORMATS
        .iter()
        .position(|(code, ..)| *code == choice)
        .map_or(0, |i| i as i32 + 1)
}

/// Auswahl mit Beispielen: "Windows (11/7/2026, 2:30 PM)", "07.11.2026, 14:30" …
fn format_options() -> ModelRc<SelectOption> {
    let sample = NaiveDate::from_ymd_opt(2026, 11, 7).unwrap_or_default();
    let example = |r: dates::Region| {
        format!(
            "{}, {}",
            dates::numeric_in(sample, r),
            dates::clock_in(14, 30, r)
        )
    };
    let mut options = vec![SelectOption {
        id: 0,
        name: tr!("Windows ({})", example(region_for("windows"))).into(),
        color: Color::default(),
    }];
    options.extend(
        FORMATS
            .iter()
            .enumerate()
            .map(|(i, (code, ..))| SelectOption {
                id: i as i32 + 1,
                name: example(region_for(code)).into(),
                color: Color::default(),
            }),
    );
    ModelRc::new(VecModel::from(options))
}

/// Eintrag in der Sprachauswahl der Einstellungen (0 = Windows-Sprache)
fn language_index(choice: &str) -> i32 {
    if choice.eq_ignore_ascii_case("system") {
        return 0;
    }
    let lang = Lang::from_code(choice).unwrap_or(Lang::En);
    Lang::ALL
        .iter()
        .position(|l| *l == lang)
        .map_or(1, |i| i as i32 + 1)
}

fn language_options() -> ModelRc<SelectOption> {
    let mut options = vec![SelectOption {
        id: 0,
        name: tr!("Windows language").into(),
        color: Color::default(),
    }];
    options.extend(Lang::ALL.iter().enumerate().map(|(i, l)| SelectOption {
        id: i as i32 + 1,
        name: l.native_name().into(),
        color: Color::default(),
    }));
    ModelRc::new(VecModel::from(options))
}

/// Sprache anwenden: Rust-Texte, Slint-Übersetzungen, Namen und Formate
fn apply_language(app: &AppWindow, lang: Lang) {
    hora_i18n::set_language(lang);
    let _ = slint::select_bundled_translation(lang.code());
    let names = |name: &dyn Fn(usize) -> String| -> ModelRc<SharedString> {
        ModelRc::new(VecModel::from(
            (0..7)
                .map(|i| SharedString::from(name(i)))
                .collect::<Vec<_>>(),
        ))
    };
    let locale = app.global::<Locale>();
    locale.set_weekday_long(names(&|i| dates::capitalize(dates::weekday(i))));
    locale.set_weekday_header(names(&|i| dates::capitalize(dates::weekday_header(i))));
    locale.set_weekday_caps(names(&|i| dates::weekday_header(i).to_uppercase()));
    locale.set_weekday_letter(names(&|i| dates::weekday_letter(i).to_string()));
    locale.set_date_placeholder(dates::numeric_placeholder().into());
    locale.set_hour12(dates::region().hour12);
    app.set_language_options(language_options());
    app.set_format_options(format_options());
    app.set_default_reminder_options(default_reminder_options());
    app.set_reminder_presets(reminder_presets(&reminders::TIMED, false));
    app.set_reminder_presets_all_day(reminder_presets(&reminders::ALL_DAY, true));
}

/// Nach einem Sprachwechsel: Beispieldaten in der neuen Sprache, Menü im
/// Infobereich und alle Ansichten neu
fn relabel(app: &AppWindow, state: &mut State) {
    let today = state.now().date();
    let (events, all_day, next_id) = sample::generate(monday_of(today));
    state.events = events;
    state.all_day = all_day;
    state.next_id = next_id;
    state.demo_journals = journal::sample(today, Utc::now());
    state.parsed_journals.clear();
    if let Some(shell) = &state.shell {
        shell.relabel();
    }
    activity_ui::show(app, state);
    updates::show(app, state);
    refresh(app, state, true);
}

fn main() -> std::process::ExitCode {
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    log::init(&dir);
    match run(dir) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            log::fatal(&tr!("Hora could not start: {}", e));
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(dir: PathBuf) -> Result<(), slint::PlatformError> {
    let at_main = hora_platform::process_uptime();
    // Läuft Hora schon, dort das Fenster zeigen und hier aufhören
    let instance_name = hora_platform::instance::name_for(&dir);
    // `--quit` (Installer vor einem Update): laufende Instanz beenden
    let quit = std::env::args().any(|a| a == "--quit");
    let Some(_instance) = hora_platform::instance::acquire(&instance_name) else {
        if quit {
            hora_platform::instance::request_quit(&instance_name);
        } else {
            hora_platform::instance::activate_running(&instance_name);
        }
        return Ok(());
    };
    if quit {
        return Ok(());
    }
    select_renderer()?;
    let app = AppWindow::new()?;
    install_window_controls(&app);

    let theme_choice = |name: &str| match name {
        "light" => Some(1),
        "dark" => Some(2),
        "system" => Some(0),
        _ => None,
    };
    let env_theme = std::env::var("HORA_THEME")
        .ok()
        .and_then(|v| theme_choice(&v));
    app.set_version(env!("CARGO_PKG_VERSION").into());
    app.global::<DialogLogic>()
        .on_is_insecure(|url| url.trim().to_ascii_lowercase().starts_with("http://"));
    let logic = app.global::<WeekLogic>();
    logic.on_snap_move(|start, delta, dur| week::snap_move(start, f64::from(delta), dur));
    logic.on_snap_resize(|start, dur, delta| week::snap_resize(start, dur, f64::from(delta)));

    let home = home_zone();
    let db = dir.join("hora.db");
    // Lässt sich die Datenbank nicht öffnen, lieber sichtbar abbrechen als mit
    // einer leeren im Speicher weiterzumachen, deren Einträge dann verloren gingen
    let store = Store::open(&db).map_err(|e| {
        slint::PlatformError::from(tr!(
            "The database {} can't be opened ({}). To avoid losing anything, Hora will quit.",
            db.display(),
            e
        ))
    })?;
    // Zahlenformate (Windows-Region oder eigene Wahl), Sprache der App
    dates::set_region(region_for(&format_choice(&store)));
    adopt_installer_language(&store);
    apply_language(&app, resolve_language(&language_choice(&store)));
    let accounts = store.accounts().unwrap_or_default();
    let calendars = store.calendars().unwrap_or_default();
    let demo_visible = match store.setting("demo_visible").ok().flatten() {
        Some(value) => value == "1",
        None => accounts.is_empty(),
    };
    let worker = Worker::spawn(db.clone(), |update| {
        let _ = slint::invoke_from_event_loop(move || dispatch(update));
    })
    .ok();

    let today = Utc::now().with_timezone(&home).date_naive();
    let (events, all_day, next_id) = sample::generate(monday_of(today));
    let rows = Rc::new(VecModel::<EventItem>::default());
    app.set_events(ModelRc::from(rows.clone()));
    let reminder_rows = Rc::new(VecModel::<ReminderChip>::default());
    app.set_event_reminders(ModelRc::from(reminder_rows.clone()));
    let settings = Settings::load(&store);
    // Theme: HORA_THEME vor der Wahl aus der Befehlspalette
    let theme = env_theme.or_else(|| {
        store
            .setting("theme")
            .ok()
            .flatten()
            .and_then(|v| theme_choice(&v))
    });
    if let Some(scheme) = theme {
        app.global::<Theme>().set_scheme_override(scheme);
    }
    let _ = store.prune_reminders((Utc::now() - chrono::Duration::days(60)).timestamp());
    let state = Rc::new(RefCell::new(State {
        home,
        view: View::Week,
        anchor: today,
        mini_month: today.with_day(1).unwrap_or(today),
        store,
        accounts,
        calendars,
        events,
        all_day,
        next_id,
        demo_visible,
        parsed: HashMap::new(),
        items: Vec::new(),
        rows,
        row_keys: Vec::new(),
        worker,
        sync: SyncInfo::default(),
        pending: PendingSummary::default(),
        preview: None,
        editing: None,
        deleting: None,
        series_action: None,
        settings,
        planner: reminders::Planner::default(),
        shell: None,
        reminder_rows: reminder_rows.clone(),
        reminders_before: Vec::new(),
        shown_toasts: Vec::new(),
        parsed_tasks: HashMap::new(),
        tasks: Vec::new(),
        task_filter: tasks::Filter::default(),
        collapsed_tasks: HashSet::new(),
        editing_task: None,
        deleting_task: None,
        task_calendar_ids: Vec::new(),
        task_parent_ids: Vec::new(),
        option_ids: Vec::new(),
        conflict: None,
        dismissed_conflicts: HashSet::new(),
        palette: Vec::new(),
        search_index: None,
        parsed_journals: HashMap::new(),
        demo_journals: journal::sample(today, Utc::now()),
        journal_entries: Vec::new(),
        journals: Vec::new(),
        journal_all: false,
        journal_selected: None,
        journal_draft: None,
        journal_dirty: false,
        journal_timer: slint::Timer::default(),
        journal_loaded: (String::new(), String::new()),
        journal_error: None,
        journal_create_busy: false,
        journal_create_error: None,
        deleting_journal: None,
        updates: updates::Updates::new(&dir),
        db,
        counter: None,
        counting_paused: false,
        activity_dialog: activity_ui::Dialog::default(),
    }));
    UI.with(|ui| *ui.borrow_mut() = Some((app.as_weak(), state.clone())));
    updates::start(&app, &mut state.borrow_mut());
    // Infobereich und Benachrichtigungen erst nach dem ersten Bild: Das
    // Fenster erscheint so schneller (WinRT braucht gut 20 ms)
    slint::Timer::single_shot(Duration::from_millis(100), {
        let state = state.clone();
        move || {
            let shell = shell::Shell::start(&instance_name, &dir, |event| {
                let _ = slint::invoke_from_event_loop(move || on_tray(event));
            });
            let mut s = state.borrow_mut();
            s.shell = Some(shell);
            // App-Zeiten (falls eingeschaltet) ebenfalls erst jetzt
            activity_ui::apply(&mut s);
            drop(s);
            shell::refresh_autostart();
        }
    });

    // Navigation
    let navigate = |state: &mut State, forward: bool| {
        let step = |d: NaiveDate, days: u64| {
            if forward {
                d + Days::new(days)
            } else {
                d - Days::new(days)
            }
        };
        state.anchor = match state.view {
            View::Day => step(state.anchor, 1),
            View::ThreeDays => step(state.anchor, 3),
            View::Week | View::Tasks => step(state.anchor, 7),
            View::Month | View::Agenda | View::Journal => {
                let first = state.anchor.with_day(1).unwrap_or(state.anchor);
                if forward {
                    first + Months::new(1)
                } else {
                    first - Months::new(1)
                }
            }
            View::Year => {
                if forward {
                    state.anchor + Months::new(12)
                } else {
                    state.anchor - Months::new(12)
                }
            }
        };
        state.mini_month = state.anchor.with_day(1).unwrap_or(state.anchor);
    };
    app.on_prev(with(&app, &state, move |app, s| {
        navigate(s, false);
        load_view(app, s, false);
    }));
    app.on_next(with(&app, &state, move |app, s| {
        navigate(s, true);
        load_view(app, s, false);
    }));
    app.on_go_today(with(&app, &state, |app, s| {
        s.anchor = s.now().date();
        s.mini_month = s.anchor.with_day(1).unwrap_or(s.anchor);
        load_view(app, s, false);
        app.invoke_scroll_to_minute((minutes_of(s.now()) - 120).max(0));
    }));
    app.on_set_view({
        let (weak, state) = (app.as_weak(), state.clone());
        move |v| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            journal_ui::flush(&app, &mut s);
            s.view = View::from_index(v);
            load_view(&app, &mut s, false);
            // Lag der Fokus in der alten Ansicht (z. B. im Jahresraster), wäre er
            // sonst mit ihr verschwunden und die Tastenkürzel blieben stumm
            app.invoke_focus_main();
        }
    });
    app.on_open_day({
        let (weak, state) = (app.as_weak(), state.clone());
        move |cell| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            let (first, _) = s.range();
            s.anchor = first + Days::new(cell.max(0) as u64);
            s.view = View::Day;
            load_view(&app, &mut s, false);
            app.invoke_focus_main();
        }
    });
    app.on_open_month({
        let (weak, state) = (app.as_weak(), state.clone());
        move |m| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            let year = s.range().0.year();
            if let Some(first) = NaiveDate::from_ymd_opt(year, m.clamp(0, 11) as u32 + 1, 1) {
                s.anchor = first;
                s.mini_month = first;
                s.view = View::Month;
                load_view(&app, &mut s, false);
                app.invoke_focus_main();
            }
        }
    });
    app.on_mini_prev(with(&app, &state, |app, s| {
        s.mini_month = s.mini_month - Months::new(1);
        load_mini(app, s);
    }));
    app.on_mini_next(with(&app, &state, |app, s| {
        s.mini_month = s.mini_month + Months::new(1);
        load_mini(app, s);
    }));
    app.on_mini_pick({
        let (weak, state) = (app.as_weak(), state.clone());
        move |i| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            s.anchor = monday_of(s.mini_month) + Days::new(i.max(0) as u64);
            s.mini_month = s.anchor.with_day(1).unwrap_or(s.anchor);
            load_view(&app, &mut s, false);
        }
    });

    // Termine: Vorschau, Bearbeiten, Verschieben, Löschen
    app.on_open_event({
        let (weak, state) = (app.as_weak(), state.clone());
        move |id, x, y, w| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            let Some(item) = s.items.get(id as usize).cloned() else {
                return;
            };
            // Aufgaben öffnen gleich ihren Dialog
            if let Some(task) = task_of_item(&s, &item) {
                open_task_dialog(&app, &mut s, Some(task), None);
                return;
            }
            let details = details(&mut s, &item);
            s.preview = Some(item);
            app.set_preview_details(details);
            app.invoke_show_preview(id, x, y, w);
        }
    });
    app.on_edit_event({
        let (weak, state) = (app.as_weak(), state.clone());
        move |id| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            // Aus der Vorschau gilt der dort gezeigte Termin, auch wenn die Liste inzwischen neu geladen wurde
            let item = s
                .preview
                .take()
                .filter(|_| id < 0)
                .or_else(|| s.items.get(id as usize).cloned());
            if let Some(task) = item.as_ref().and_then(|i| task_of_item(&s, i)) {
                open_task_dialog(&app, &mut s, Some(task), None);
                return;
            }
            if let Some(item) = item.filter(Item::writable) {
                let times = editing::draft_times(&s, &item);
                open_dialog(&app, &mut s, Some(item), times);
            }
        }
    });
    app.on_delete_event({
        let (weak, state) = (app.as_weak(), state.clone());
        move |id| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            let item = s
                .preview
                .take()
                .filter(|_| id < 0)
                .or_else(|| s.items.get(id as usize).cloned());
            if let Some(task) = item.as_ref().and_then(|i| task_of_item(&s, i)) {
                if !task.read_only {
                    ask_delete_task(&app, &mut s, task);
                }
                return;
            }
            if let Some(item) = item.filter(Item::writable) {
                ask_delete(&app, &mut s, item);
            }
        }
    });
    app.on_dialog_delete(with(&app, &state, |app, s| {
        if let Some(item) = s.editing.clone() {
            app.set_event_dialog_open(false);
            ask_delete(app, s, item);
        }
    }));
    app.on_confirm_accepted(with(&app, &state, |app, s| {
        app.set_confirm_open(false);
        if let Some(entry) = s.deleting_journal.take() {
            journal_ui::delete(app, s, &entry);
            return;
        }
        if let Some(task) = s.deleting_task.take() {
            let items = s.tasks.clone();
            if let Some(index) = items.iter().position(|t| same_task(t, &task))
                && let Err(message) = tasks::delete(s, &items, index)
            {
                s.sync.errors = vec![message];
            }
            refresh(app, s, true);
            return;
        }
        if let Some(item) = s.deleting.take() {
            if let Err(message) = editing::delete(s, &item) {
                s.sync.errors = vec![message];
            }
            refresh(app, s, true);
        }
    }));
    app.on_change_event({
        let (weak, state) = (app.as_weak(), state.clone());
        move |id, day, start, dur| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            let Some(item) = s.items.get(id as usize).cloned() else {
                return;
            };
            let (first, _) = s.range();
            let begin = (first + Days::new(day.max(0) as u64)).and_time(NaiveTime::MIN)
                + chrono::Duration::minutes(i64::from(start));
            let end = begin + chrono::Duration::minutes(i64::from(dur));
            if item.recurring() && item.writable() {
                ask_scope(
                    &app,
                    &mut s,
                    SeriesAction::Move {
                        item,
                        start: begin,
                        end,
                    },
                    true,
                );
                return;
            }
            if let Err(message) = editing::move_to(&mut s, &item, begin, end) {
                s.sync.errors = vec![message];
            }
            refresh(&app, &mut s, true);
        }
    });
    app.on_month_move({
        let (weak, state) = (app.as_weak(), state.clone());
        move |id, delta| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            let Some(item) = s.items.get(id as usize).cloned() else {
                return;
            };
            if item.recurring() && item.writable() {
                let days = i64::from(delta);
                ask_scope(&app, &mut s, SeriesAction::ShiftDays { item, days }, true);
                return;
            }
            if let Err(message) = editing::shift_days(&mut s, &item, i64::from(delta)) {
                s.sync.errors = vec![message];
            }
            refresh(&app, &mut s, true);
        }
    });
    app.on_new_event(with(&app, &state, |app, s| {
        if s.view == View::Tasks {
            open_task_dialog(app, s, None, None);
            return;
        }
        if s.view == View::Journal {
            journal_ui::new_entry(app, s);
            return;
        }
        let now = s.now();
        let (first, days) = s.range();
        let shown = (0..days).contains(&(now.date() - first).num_days());
        let day = if s.view == View::Month || !shown {
            s.anchor
        } else {
            now.date()
        };
        let start = editing::next_full_hour(day.and_time(now.time()));
        open_dialog(app, s, None, editing::new_times(start, 60));
    }));
    app.on_new_event_at({
        let (weak, state) = (app.as_weak(), state.clone());
        move |day, minute| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            let (first, _) = s.range();
            let start = (first + Days::new(day.max(0) as u64)).and_time(NaiveTime::MIN)
                + chrono::Duration::minutes(i64::from(minute));
            open_dialog(&app, &mut s, None, editing::new_times(start, 60));
        }
    });
    app.on_new_event_on({
        let (weak, state) = (app.as_weak(), state.clone());
        move |cell| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            let (first, _) = s.range();
            let start = (first + Days::new(cell.max(0) as u64))
                .and_hms_opt(9, 0, 0)
                .unwrap();
            open_dialog(&app, &mut s, None, editing::new_times(start, 60));
        }
    });
    app.on_save_event({
        let (weak, state) = (app.as_weak(), state.clone());
        move |title,
              all_day,
              start_date,
              start_time,
              end_date,
              end_time,
              location,
              description,
              calendar,
              rule| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            let input = DialogInput {
                title: title.to_string(),
                times: DraftTimes {
                    all_day,
                    start_date: start_date.to_string(),
                    start_time: start_time.to_string(),
                    end_date: end_date.to_string(),
                    end_time: end_time.to_string(),
                },
                location: location.to_string(),
                description: description.to_string(),
                calendar: s
                    .option_ids
                    .get(calendar.max(0) as usize)
                    .copied()
                    .unwrap_or(-1),
                rule: rule_fields(&rule),
                reminders: reminder_input(&s),
            };
            let editing = s.editing.clone();
            if let Some(item) = editing.clone().filter(Item::recurring) {
                match editing::check_series_save(&s, &item, &input) {
                    Ok(rule_changed) => {
                        app.set_event_error("".into());
                        ask_scope(
                            &app,
                            &mut s,
                            SeriesAction::Save {
                                item,
                                input: Box::new(input),
                            },
                            !rule_changed,
                        );
                    }
                    Err(message) => app.set_event_error(message.into()),
                }
                return;
            }
            match editing::save(&mut s, editing.as_ref(), &input) {
                Ok(()) => {
                    app.set_event_dialog_open(false);
                    s.editing = None;
                    app.invoke_focus_main();
                    refresh(&app, &mut s, true);
                }
                Err(message) => app.set_event_error(message.into()),
            }
        }
    });

    // Journal
    app.on_journal_set_mode(with_arg(&app, &state, |app, s, mode: i32| {
        journal_ui::flush(app, s);
        s.journal_all = mode == 1;
        load_view(app, s, true);
    }));
    app.on_journal_new(with(&app, &state, journal_ui::new_entry));
    app.on_journal_select(with_arg(&app, &state, |app, s, id: i32| {
        let Some(entry) = s.journals.get(id as usize).cloned() else {
            return;
        };
        // Noch einmal auf den gewählten Eintrag: Editor schließen (Calino)
        if s.journal_selected.as_deref() == Some(entry.key().as_str()) {
            journal_ui::select(app, s, None, false);
        } else {
            journal_ui::select(app, s, Some(entry.key()), false);
        }
    }));
    app.on_journal_move(with_arg(&app, &state, |app, s, delta: i32| {
        journal_ui::step(app, s, delta);
    }));
    app.on_journal_close(with(&app, &state, |app, s| {
        journal_ui::select(app, s, None, false);
    }));
    app.on_journal_edited(with(&app, &state, journal_ui::edited));
    app.on_journal_add_tag(with_arg(&app, &state, |app, s, text: SharedString| {
        journal_ui::add_tag(app, s, &text);
    }));
    app.on_journal_remove_tag(with_arg(&app, &state, |app, s, i: i32| {
        journal_ui::remove_tag(app, s, i.max(0) as usize);
    }));
    app.on_journal_set_date(with_arg(&app, &state, |app, s, text: SharedString| {
        journal_ui::set_date(app, s, &text);
    }));
    app.on_journal_set_calendar(with_arg(&app, &state, |app, s, id: i32| {
        journal_ui::set_calendar(app, s, i64::from(id));
    }));
    app.on_journal_delete(with(&app, &state, journal_ui::ask_delete));
    app.on_journal_set_write(with_arg(&app, &state, |app, s, write: bool| {
        journal_ui::set_write(app, s, write);
    }));
    app.on_journal_create_calendar(with(&app, &state, |app, s| {
        let Some(account) = s.accounts.iter().find(|a| a.home_url.is_some()) else {
            return;
        };
        s.send(Command::CreateCalendar {
            account_id: account.id,
            name: tr!("Journal").into(),
            components: vec!["VJOURNAL"],
        });
        s.journal_create_busy = true;
        s.journal_create_error = None;
        journal_ui::load(app, s);
    }));
    app.on_journal_open_link(|url| {
        if let Err(e) = hora_platform::open_url(&url) {
            log::write(&format!("Link öffnen: {e}"));
        }
    });
    app.on_journal_format(|text, anchor, cursor, marker| {
        journal_ui::format(&text, anchor, cursor, &marker)
    });
    app.on_month_open_journal(with_arg(&app, &state, |app, s, cell: i32| {
        let (first, _) = s.range();
        let day = first + Days::new(cell.max(0) as u64);
        s.journal_all = false;
        let key = journal::collect(s)
            .into_iter()
            .find(|e| e.day == day && !e.draft)
            .map(|e| e.key());
        match key {
            Some(key) => journal_ui::show(app, s, &key, day),
            None => {
                s.view = View::Journal;
                s.anchor = day;
                load_view(app, s, false);
            }
        }
        app.invoke_focus_main();
    }));

    // Aufgaben
    app.on_task_set_filter({
        let (weak, state) = (app.as_weak(), state.clone());
        move |i| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            s.task_filter = tasks::Filter::from_index(i);
            load_tasks(&app, &mut s);
        }
    });
    app.on_task_quick_add({
        let (weak, state) = (app.as_weak(), state.clone());
        move |title| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            // Die Eingabezeile versteht Fälligkeiten wie die Schnelleingabe ("Milch kaufen morgen")
            if let Err(message) = quick_add::submit(&app, &mut s, &title, false, true) {
                s.sync.errors = vec![message];
            }
            refresh(&app, &mut s, true);
        }
    });
    app.on_task_new(with(&app, &state, |app, s| {
        open_task_dialog(app, s, None, None)
    }));
    app.on_task_toggle({
        let (weak, state) = (app.as_weak(), state.clone());
        move |id| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            let items = s.tasks.clone();
            if (id as usize) < items.len()
                && let Err(message) = tasks::toggle(&mut s, &items, id as usize)
            {
                s.sync.errors = vec![message];
            }
            refresh(&app, &mut s, true);
        }
    });
    app.on_task_open({
        let (weak, state) = (app.as_weak(), state.clone());
        move |id| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            if let Some(item) = s.tasks.get(id as usize).cloned() {
                open_task_dialog(&app, &mut s, Some(item), None);
            }
        }
    });
    app.on_task_remove({
        let (weak, state) = (app.as_weak(), state.clone());
        move |id| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            if let Some(item) = s.tasks.get(id as usize).cloned().filter(|t| !t.read_only) {
                ask_delete_task(&app, &mut s, item);
            }
        }
    });
    app.on_task_collapse({
        let (weak, state) = (app.as_weak(), state.clone());
        move |id| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            if let Some(uid) = s.tasks.get(id as usize).map(|t| t.uid.clone())
                && !s.collapsed_tasks.remove(&uid)
            {
                s.collapsed_tasks.insert(uid);
            }
            load_tasks(&app, &mut s);
        }
    });
    app.on_task_save({
        let (weak, state) = (app.as_weak(), state.clone());
        move |title,
              due_mode,
              due_date,
              due_time,
              start_date,
              priority,
              calendar,
              parent,
              description| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            let input = tasks::TaskInput {
                title: title.to_string(),
                due_mode,
                due_date: due_date.to_string(),
                due_time: due_time.to_string(),
                start_date: start_date.to_string(),
                priority,
                calendar: s
                    .task_calendar_ids
                    .get(calendar.max(0) as usize)
                    .copied()
                    .unwrap_or(-1),
                parent: s
                    .task_parent_ids
                    .get(parent.max(0) as usize)
                    .cloned()
                    .flatten(),
                description: description.to_string(),
            };
            if input.calendar < 0 {
                app.set_task_error(tr!("Please choose a calendar that supports tasks.").into());
                return;
            }
            let items = s.tasks.clone();
            let editing = s.editing_task.clone();
            let index = editing
                .as_ref()
                .and_then(|t| items.iter().position(|x| same_task(x, t)));
            if editing.is_some() && index.is_none() {
                app.set_task_error(tr!("The task no longer exists.").into());
                return;
            }
            match tasks::save(&mut s, &items, index, &input) {
                Ok(()) => {
                    app.set_task_dialog_open(false);
                    s.editing_task = None;
                    app.invoke_focus_main();
                    refresh(&app, &mut s, true);
                }
                Err(message) => app.set_task_error(message.into()),
            }
        }
    });
    app.on_task_delete(with(&app, &state, |app, s| {
        if let Some(item) = s.editing_task.take() {
            app.set_task_dialog_open(false);
            ask_delete_task(app, s, item);
        }
    }));
    app.on_task_add_subtask(with(&app, &state, |app, s| {
        if let Some(parent) = s.editing_task.take() {
            open_task_dialog(app, s, None, Some(parent.uid));
        }
    }));

    // Schnelleingabe
    app.on_show_quick(with(&app, &state, |app, _| {
        app.set_quick_error("".into());
        app.set_quick_text("".into());
        app.set_quick_open(true);
    }));
    app.on_quick_preview({
        let state = state.clone();
        // Wird beim Zeichnen ausgewertet; ist der Zustand gerade belegt, leer
        move |text| {
            state
                .try_borrow()
                .map(|s| quick_add::preview(&s, &text))
                .unwrap_or_default()
        }
    });
    app.on_quick_submit({
        let (weak, state) = (app.as_weak(), state.clone());
        move |text, open| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            match quick_add::submit(&app, &mut s, &text, open, false) {
                Ok(day) => {
                    app.set_quick_open(false);
                    if let Some(day) = day {
                        s.anchor = day;
                        s.mini_month = day.with_day(1).unwrap_or(day);
                    }
                    if !open {
                        app.invoke_focus_main();
                    }
                    refresh(&app, &mut s, false);
                }
                Err(message) => app.set_quick_error(message.into()),
            }
        }
    });

    // Befehlspalette
    app.on_show_palette({
        let (weak, state) = (app.as_weak(), state.clone());
        move || {
            let Some(app) = weak.upgrade() else { return };
            if app.get_palette_open() {
                app.set_palette_open(false);
                app.invoke_focus_main();
                return;
            }
            app.set_palette_text("".into());
            palette_query(&app, &mut state.borrow_mut(), "");
            app.set_palette_open(true);
        }
    });
    app.on_palette_query({
        let (weak, state) = (app.as_weak(), state.clone());
        move |text| {
            let Some(app) = weak.upgrade() else { return };
            palette_query(&app, &mut state.borrow_mut(), &text);
        }
    });
    app.on_palette_run({
        let (weak, state) = (app.as_weak(), state.clone());
        move |row| {
            let Some(app) = weak.upgrade() else { return };
            let id = app
                .get_palette_items()
                .row_data(row.max(0) as usize)
                .map_or(-1, |r| r.id);
            let action = usize::try_from(id)
                .ok()
                .and_then(|id| state.borrow().palette.get(id).map(|e| e.action.clone()));
            let Some(action) = action else { return };
            app.set_palette_open(false);
            app.invoke_focus_main();
            run_palette_action(&app, &state, action);
        }
    });

    // Serien
    app.on_scope_chosen({
        let (weak, state) = (app.as_weak(), state.clone());
        move |choice| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            let Some(action) = s.series_action.take() else {
                return;
            };
            let scope = match choice {
                0 => Scope::This,
                1 => Scope::Following,
                _ => Scope::All,
            };
            let saving = matches!(action, SeriesAction::Save { .. });
            match editing::apply_series(&mut s, &action, scope) {
                Ok(()) if saving => {
                    app.set_event_dialog_open(false);
                    s.editing = None;
                }
                Ok(()) => {}
                Err(message) if saving => app.set_event_error(message.into()),
                Err(message) => s.sync.errors = vec![message],
            }
            if !app.get_event_dialog_open() {
                app.invoke_focus_main();
            }
            refresh(&app, &mut s, true);
        }
    });
    app.on_scope_dismissed(with(&app, &state, |app, s| {
        // Abgebrochenes Ziehen springt zurück; ein offener Dialog bleibt offen
        s.series_action = None;
        if !app.get_event_dialog_open() {
            app.invoke_focus_main();
        }
        refresh(app, s, true);
    }));
    app.on_weekday_of({
        let state = state.clone();
        move |date| {
            let today = state.borrow().now().date();
            forms::parse_date(&date, today)
                .map_or(-1, |d| d.weekday().num_days_from_monday() as i32)
        }
    });
    app.on_rule_summary({
        let state = state.clone();
        move |rule, start| {
            let today = state.borrow().now().date();
            let start = forms::parse_date(&start, today).unwrap_or(today);
            match rule_fields(&rule).read(today) {
                Ok(repeat) if repeat.ends_before(start) => {
                    tr!("The repetition ends before the start.").into()
                }
                Ok(repeat) => repeat.describe(start).into(),
                Err(message) => message.into(),
            }
        }
    });

    // Erinnerungen im Termin-Dialog
    app.on_add_reminder({
        let state = state.clone();
        move |minutes, label| {
            let rows = state.borrow().reminder_rows.clone();
            if !rows.iter().any(|c| c.keep < 0 && c.minutes == minutes) {
                rows.push(ReminderChip {
                    label,
                    minutes,
                    keep: -1,
                });
            }
        }
    });
    app.on_remove_reminder({
        let state = state.clone();
        move |index| {
            let rows = state.borrow().reminder_rows.clone();
            if index >= 0 && (index as usize) < rows.row_count() {
                rows.remove(index as usize);
            }
        }
    });

    // Einstellungen
    app.on_show_settings(with(&app, &state, |app, s| open_settings(app, s)));
    app.on_language_changed(with(&app, &state, |app, s| {
        let index = app.get_settings_language();
        let choice = match usize::try_from(index - 1)
            .ok()
            .and_then(|i| Lang::ALL.get(i))
        {
            Some(lang) => lang.code(),
            None => "system",
        };
        let _ = s.store.set_setting("language", choice);
        apply_language(app, resolve_language(choice));
        // Die Auswahl selbst bleibt stehen, die Liste hat neue Namen
        app.set_settings_language(index);
        relabel(app, s);
    }));
    app.on_formats_changed(with(&app, &state, |app, s| {
        let index = app.get_settings_formats();
        let choice = usize::try_from(index - 1)
            .ok()
            .and_then(|i| FORMATS.get(i))
            .map_or("windows", |(code, ..)| *code);
        let _ = s.store.set_setting("formats", choice);
        dates::set_region(region_for(choice));
        apply_language(app, hora_i18n::language());
        app.set_settings_formats(index);
        relabel(app, s);
    }));
    app.on_settings_changed(with(&app, &state, |app, s| {
        let before = (s.settings.reminders, shell::autostart_enabled());
        let checked_updates = s.settings.update_check;
        s.settings.update_check = app.get_settings_update_check();
        let shown_tasks = s.settings.show_tasks;
        s.settings.reminders = app.get_settings_reminders();
        s.settings.keep_running = app.get_settings_keep_running();
        s.settings.show_tasks = app.get_settings_show_tasks();
        let counted = s.settings.activities;
        s.settings.activities = app.get_settings_activities();
        if s.settings.task_reminders != app.get_settings_task_reminders() {
            s.settings.task_reminders = app.get_settings_task_reminders();
            s.planner.dirty = true;
        }
        if shown_tasks != s.settings.show_tasks {
            load_view(app, s, true);
        }
        let index = app.get_settings_default_reminder();
        s.settings.default_reminder = (index > 0)
            .then(|| reminders::TIMED.get(index as usize - 1).copied())
            .flatten();
        s.settings.save(&s.store);
        if counted != s.settings.activities {
            activity_ui::apply(s);
            activity_ui::show(app, s);
        }
        if s.settings.update_check && !checked_updates {
            updates::tick(app, s);
        }
        if before.0 != s.settings.reminders {
            s.planner.dirty = true;
        }
        let autostart = app.get_settings_autostart();
        if autostart != before.1
            && let Err(e) = shell::set_autostart(autostart)
        {
            app.set_settings_status(tr!("Autostart not changed: {}", e).into());
            app.set_settings_autostart(before.1);
        }
    }));
    app.on_check_updates(with(&app, &state, updates::check));
    app.on_activity_new(with(&app, &state, activity_ui::open_new));
    app.on_activity_edit(with_arg(&app, &state, activity_ui::open_edit));
    app.on_activity_add_app({
        let (weak, state) = (app.as_weak(), state.clone());
        move |text| {
            weak.upgrade()
                .is_some_and(|app| activity_ui::add_app(&app, &mut state.borrow_mut(), &text))
        }
    });
    app.on_activity_toggle_app({
        let state = state.clone();
        move |index, checked| activity_ui::toggle_app(&mut state.borrow_mut(), index, checked)
    });
    app.on_activity_save(with(&app, &state, activity_ui::save));
    app.on_activity_delete(with(&app, &state, activity_ui::delete));
    app.on_install_update(with(&app, &state, updates::install));
    app.on_test_notification(with(&app, &state, |app, s| {
        let Some(shell) = &s.shell else { return };
        let now = Utc::now();
        let sample = reminders::Planned {
            key: "test".into(),
            at: now,
            calendar_id: -1,
            title: tr!("Test notification").into(),
            location: tr!("This is what Hora's reminders look like").into(),
            start: now + chrono::Duration::minutes(15),
            end: now + chrono::Duration::minutes(75),
            all_day: false,
            task: false,
        };
        let mut toast = reminders::toast_for(&sample, now, s.home);
        toast.tag = "test".into();
        toast.launch = String::new();
        let status = match shell.show(&toast, |_| {}) {
            Ok(()) => shell.status(),
            Err(e) => e,
        };
        app.set_settings_status(
            if status.is_empty() {
                tr!("Sent. If nothing appears: Windows Settings → System → Notifications.")
                    .to_string()
            } else {
                status
            }
            .into(),
        );
    }));

    // Konflikte
    // Beantwortete Konflikte nicht noch einmal zeigen, bis der Sync-Thread sie erledigt hat
    app.on_conflict_keep_mine(with(&app, &state, |_, s| {
        if let Some(op_id) = s.conflict.take() {
            s.dismissed_conflicts.insert(op_id);
            s.send(Command::ResolveConflict {
                op_id,
                keep_mine: true,
            });
        }
    }));
    app.on_conflict_take_server(with(&app, &state, |_, s| {
        if let Some(op_id) = s.conflict.take() {
            s.dismissed_conflicts.insert(op_id);
            s.send(Command::ResolveConflict {
                op_id,
                keep_mine: false,
            });
        }
    }));
    app.on_conflict_later(with(&app, &state, |_, s| {
        if let Some(op_id) = s.conflict.take() {
            s.dismissed_conflicts.insert(op_id);
        }
    }));

    // Kalender und Konten
    app.on_toggle_calendar({
        let (weak, state) = (app.as_weak(), state.clone());
        move |id| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            if id < 0 {
                s.demo_visible = !s.demo_visible;
                let value = if s.demo_visible { "1" } else { "0" };
                let _ = s.store.set_setting("demo_visible", value);
            } else if let Some(calendar) = s.calendars.iter().find(|c| c.id == i64::from(id)) {
                let _ = s.store.set_calendar_visible(calendar.id, !calendar.visible);
            }
            s.reload_calendars();
            refresh(&app, &mut s, false);
        }
    });
    app.on_set_calendar_color({
        let (weak, state) = (app.as_weak(), state.clone());
        move |id, color| {
            let Some(app) = weak.upgrade() else { return };
            if id < 0 {
                return;
            }
            let mut s = state.borrow_mut();
            let hex = format!(
                "#{:02x}{:02x}{:02x}",
                color.red(),
                color.green(),
                color.blue()
            );
            let _ = s.store.set_calendar_color(i64::from(id), Some(&hex));
            s.reload_calendars();
            refresh(&app, &mut s, true);
        }
    });
    app.on_show_add_account({
        let weak = app.as_weak();
        move || {
            if let Some(app) = weak.upgrade() {
                app.set_add_account_error("".into());
                app.set_add_account_busy(false);
                app.set_add_account_open(true);
            }
        }
    });
    app.on_connect({
        let (weak, state) = (app.as_weak(), state.clone());
        move |name, url, username, password| {
            let Some(app) = weak.upgrade() else { return };
            let missing = if url.trim().is_empty() {
                Some(tr!("Please enter the server address."))
            } else if username.trim().is_empty() {
                Some(tr!("Please enter the username."))
            } else if password.is_empty() {
                Some(tr!("Please enter the password."))
            } else {
                None
            };
            if let Some(message) = missing {
                app.set_add_account_error(message.into());
                return;
            }
            app.set_add_account_error("".into());
            app.set_add_account_busy(true);
            state.borrow().send(Command::AddAccount {
                name: name.to_string(),
                url: url.trim().to_string(),
                username: username.trim().to_string(),
                password: password.to_string(),
            });
        }
    });
    app.on_sync_now(with(&app, &state, |app, s| {
        // Zurückgestellte Konflikte erneut zeigen
        s.dismissed_conflicts.clear();
        check_conflicts(app, s);
        s.send(Command::SyncAll);
    }));

    refresh(&app, &mut state.borrow_mut(), false);
    app.invoke_scroll_to_minute(7 * 60);
    if !state.borrow().accounts.is_empty() {
        state.borrow().send(Command::SyncAll);
    }

    // Jetzt-Linie mitlaufen lassen
    let clock = slint::Timer::default();
    clock.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(20),
        with(&app, &state, |app, s| update_now(app, s)),
    );
    // Regelmäßig abgleichen; wartende Änderungen öfter versuchen
    let sync_timer = slint::Timer::default();
    sync_timer.start(
        slint::TimerMode::Repeated,
        SYNC_INTERVAL,
        with(&app, &state, |_, s| {
            if !s.accounts.is_empty() {
                s.send(Command::SyncAll);
            }
        }),
    );
    let push_timer = slint::Timer::default();
    push_timer.start(
        slint::TimerMode::Repeated,
        PUSH_INTERVAL,
        with(&app, &state, |_, s| {
            if s.pending.waiting > 0 {
                s.send(Command::Push);
            }
        }),
    );

    install_startup_probe(&app, at_main);

    // Erinnerungen prüfen: gleich nach dem Start, dann alle 15 Sekunden
    let reminder_timer = slint::Timer::default();
    reminder_timer.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(15),
        with(&app, &state, |_, s| check_reminders(s)),
    );
    slint::Timer::single_shot(Duration::from_secs(2), {
        let state = state.clone();
        move || check_reminders(&mut state.borrow_mut())
    });

    // Schließen legt Hora in den Infobereich, sofern eingestellt
    app.window().on_close_requested({
        let state = state.clone();
        let weak = app.as_weak();
        move || {
            if let Some(app) = weak.upgrade() {
                journal_ui::flush(&app, &mut state.borrow_mut());
            }
            let s = state.borrow();
            let tray = s.shell.as_ref().is_some_and(|sh| sh.tray.is_some());
            if !(s.settings.keep_running && tray) {
                let _ = slint::quit_event_loop();
            } else if s.store.setting("tray_hint_shown").ok().flatten().is_none() {
                let _ = s.store.set_setting("tray_hint_shown", "1");
                if let Some(shell) = &s.shell {
                    let hint = hora_platform::notify::Toast {
                        title: tr!("Hora keeps running in the notification area").into(),
                        lines: vec![tr!("Reminders keep coming. Quit via the icon in the notification area (right-click).").into()],
                        tag: "infobereich".into(),
                        ..Default::default()
                    };
                    let _ = shell.show(&hint, |_| {});
                }
            }
            slint::CloseRequestResponse::HideWindow
        }
    });

    #[cfg(feature = "devtools")]
    devtools::install(&app, state.clone());

    if !std::env::args().any(|a| a == shell::TRAY_ARG) {
        app.show()?;
        if let Some(hwnd) = shell::hwnd(&app) {
            hora_platform::round_corners(hwnd);
        }
    }
    slint::run_event_loop_until_quit()?;
    // Beim Beenden über den Infobereich: offene Journal-Eingaben sichern
    journal_ui::flush(&app, &mut state.borrow_mut());
    // App-Zeiten: laufenden Abschnitt abschließen
    state.borrow_mut().counter = None;
    // Symbol im Infobereich entfernen
    state.borrow_mut().shell = None;
    // Update angefordert: jetzt erst das Setup starten
    updates::run_pending(&state.borrow().updates);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn week_starts_on_monday() {
        // Mittwoch, 4. November 2026
        assert_eq!(monday_of(date(2026, 11, 4)), date(2026, 11, 2));
        // Sonntag gehört zur Woche davor
        assert_eq!(monday_of(date(2026, 11, 8)), date(2026, 11, 2));
        assert_eq!(monday_of(date(2026, 11, 2)), date(2026, 11, 2));
    }

    #[test]
    fn titles_in_english() {
        assert_eq!(
            month_title(date(2026, 10, 26), date(2026, 11, 1)),
            "Oct – Nov 2026"
        );
        assert_eq!(
            view_title(View::Day, date(2026, 11, 4), date(2026, 11, 4), 1),
            "Wednesday, 4 November 2026"
        );
        hora_i18n::with_language(Lang::Fr, || {
            assert_eq!(
                view_title(View::Month, date(2026, 11, 4), date(2026, 10, 26), 42),
                "Novembre 2026"
            );
        });
    }

    #[test]
    fn titles_in_german() {
        hora_i18n::with_language(Lang::De, titles_in_german_inner);
    }

    fn titles_in_german_inner() {
        assert_eq!(
            month_title(date(2026, 11, 2), date(2026, 11, 8)),
            "November 2026"
        );
        assert_eq!(
            month_title(date(2026, 10, 26), date(2026, 11, 1)),
            "Okt. – Nov. 2026"
        );
        assert_eq!(
            month_title(date(2026, 12, 28), date(2027, 1, 3)),
            "Dez. 2026 – Jan. 2027"
        );
        assert_eq!(
            view_title(View::Day, date(2026, 11, 4), date(2026, 11, 4), 1),
            "Mittwoch, 4. November 2026"
        );
        assert_eq!(
            view_title(View::Month, date(2026, 11, 4), date(2026, 10, 26), 42),
            "November 2026"
        );
    }

    #[test]
    fn iso_week_number() {
        assert_eq!(date(2026, 11, 2).iso_week().week(), 45);
        // 28.12.2026 liegt noch in KW 53 des Jahres 2026
        assert_eq!(date(2026, 12, 28).iso_week().week(), 53);
    }

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex("#4f7cac"), Some(0x4f7cac));
        assert_eq!(parse_hex("#4F7CACFF"), Some(0x4f7cac));
        assert_eq!(parse_hex("rot"), None);
    }

    #[test]
    fn view_indices_roundtrip() {
        for view in [
            View::Day,
            View::ThreeDays,
            View::Week,
            View::Month,
            View::Year,
            View::Agenda,
            View::Tasks,
        ] {
            assert_eq!(View::from_index(view.index()), view);
        }
    }
}
