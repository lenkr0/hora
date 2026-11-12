//! App-Zeiten in der Oberfläche: Abschnitt in den Einstellungen, Dialog
//! einer Aktivität, Pause über den Infobereich. Gezählt wird in
//! [`crate::activity`].

use std::rc::Rc;

use chrono::{DateTime, NaiveTime, TimeDelta, Utc};
use hora_i18n::tr;
use hora_storage::{Activity, ActivityDraft};
use slint::{Model, ModelRc, VecModel};

use crate::activity::{Config, Counter};
use crate::{ActivityRow, AppChoice, AppWindow, SelectOption, State};

/// Wahlmöglichkeiten für die Pausengrenze in Sekunden
const LIMITS: [i64; 6] = [30, 60, 120, 300, 600, 900];
/// Pausengrenze neuer Aktivitäten
const DEFAULT_LIMIT: i64 = 60;

/// Dialog einer Aktivität: welche bearbeitet wird und die Programmliste
#[derive(Default)]
pub struct Dialog {
    pub editing: Option<i64>,
    pub apps: Rc<VecModel<AppChoice>>,
}

/// Dauer kurz: "45 min", "3:20 h"
pub fn short(d: TimeDelta) -> String {
    let minutes = d.num_minutes().max(0);
    if minutes < 60 {
        tr!("{} min", minutes)
    } else {
        tr!("{} h", format!("{}:{:02}", minutes / 60, minutes % 60))
    }
}

fn color_hex(index: i32) -> String {
    let colors = crate::CALENDAR_COLORS;
    let color = colors[index.clamp(0, colors.len() as i32 - 1) as usize];
    format!("#{color:06x}")
}

fn color_index(hex: &str) -> i32 {
    crate::parse_hex(hex)
        .and_then(|c| crate::CALENDAR_COLORS.iter().position(|&x| x == c))
        .unwrap_or(0) as i32
}

/// Beginn der laufenden Woche (Montag, 0 Uhr) in UTC
fn week_start(state: &State) -> DateTime<Utc> {
    let monday = crate::monday_of(state.now().date());
    hora_core::time::local_to_utc(monday.and_time(NaiveTime::MIN), state.home)
}

/// Zählung starten, anpassen oder anhalten, je nach Einstellung, Pause und
/// Aktivitäten; dazu den Eintrag im Infobereich
pub fn apply(state: &mut State) {
    let activities = if state.settings.activities {
        state.store.activities().unwrap_or_default()
    } else {
        Vec::new()
    };
    let config = Config::new(&activities);
    // Pausieren lässt sich nur, wenn es etwas zu zählen gibt
    let menu = (!config.is_empty()).then_some(state.counting_paused);
    if config.is_empty() || state.counting_paused {
        state.counter = None;
    } else if let Some(counter) = &state.counter {
        counter.configure(config);
    } else {
        state.counter = Some(Counter::start(state.db.clone(), config));
    }
    if let Some(shell) = &state.shell {
        shell.set_counting(menu);
    }
}

/// Pause über den Infobereich (bis zum Fortsetzen oder Neustart von Hora)
pub fn toggle_pause(app: &AppWindow, state: &mut State) {
    state.counting_paused = !state.counting_paused;
    apply(state);
    app.set_activities_paused(state.counting_paused);
}

fn rows(state: &State, activities: &[Activity]) -> Vec<ActivityRow> {
    let now = Utc::now();
    let week = state
        .store
        .activity_totals(week_start(state), now + TimeDelta::days(1))
        .unwrap_or_default();
    let total = state
        .store
        .activity_totals(DateTime::UNIX_EPOCH, now + TimeDelta::days(1))
        .unwrap_or_default();
    activities
        .iter()
        .map(|activity| {
            let time = |totals: &std::collections::HashMap<i64, TimeDelta>| {
                short(totals.get(&activity.id).copied().unwrap_or_default())
            };
            let apps = if activity.apps.is_empty() {
                tr!("No programs").to_string()
            } else {
                activity.apps.join(", ")
            };
            ActivityRow {
                id: activity.id as i32,
                name: activity.name.clone().into(),
                color: crate::parse_hex(&activity.color)
                    .map(crate::rgb)
                    .unwrap_or_else(|| crate::rgb(crate::CALENDAR_COLORS[0])),
                detail: tr!(
                    "{0} · this week {1} · total {2}",
                    apps,
                    time(&week),
                    time(&total)
                )
                .into(),
            }
        })
        .collect()
}

/// Abschnitt in den Einstellungen füllen
pub fn show(app: &AppWindow, state: &State) {
    app.set_settings_activities(state.settings.activities);
    app.set_activities_paused(state.counting_paused);
    let activities = state.store.activities().unwrap_or_default();
    app.set_activity_rows(ModelRc::new(VecModel::from(rows(state, &activities))));
}

fn limit_options() -> ModelRc<SelectOption> {
    let options: Vec<SelectOption> = LIMITS
        .iter()
        .map(|&seconds| SelectOption {
            id: seconds as i32,
            name: if seconds < 60 {
                tr!("{} seconds", seconds)
            } else {
                hora_i18n::trn!("{n} minute", "{n} minutes", seconds / 60)
            }
            .into(),
            color: Default::default(),
        })
        .collect();
    ModelRc::new(VecModel::from(options))
}

fn limit_index(seconds: i64) -> i32 {
    LIMITS
        .iter()
        .position(|&l| l >= seconds)
        .unwrap_or(LIMITS.len() - 1) as i32
}

/// Programmliste: gewählte zuerst, dann die gerade offenen
fn app_choices(chosen: &[String], activities: &[Activity], editing: Option<i64>) -> Vec<AppChoice> {
    let owner = |exe: &str| {
        activities
            .iter()
            .find(|a| Some(a.id) != editing && a.apps.iter().any(|x| x.eq_ignore_ascii_case(exe)))
    };
    let mut choices: Vec<AppChoice> = chosen
        .iter()
        .map(|exe| AppChoice {
            name: exe.into(),
            checked: true,
            note: Default::default(),
        })
        .collect();
    for app in hora_platform::activity::open_apps() {
        let exe = app.file_name();
        if exe.is_empty() || chosen.iter().any(|c| c.eq_ignore_ascii_case(&exe)) {
            continue;
        }
        choices.push(AppChoice {
            note: owner(&exe)
                .map(|a| tr!("counts for {}", a.name))
                .unwrap_or_default()
                .into(),
            name: exe.into(),
            checked: false,
        });
    }
    choices
}

fn open(app: &AppWindow, state: &mut State, activity: Option<&Activity>) {
    let activities = state.store.activities().unwrap_or_default();
    let editing = activity.map(|a| a.id);
    let chosen = activity.map(|a| a.apps.clone()).unwrap_or_default();
    state.activity_dialog.editing = editing;
    state
        .activity_dialog
        .apps
        .set_vec(app_choices(&chosen, &activities, editing));
    app.set_activity_apps(ModelRc::from(state.activity_dialog.apps.clone()));
    app.set_activity_editing(editing.is_some());
    app.set_activity_name(activity.map(|a| a.name.clone()).unwrap_or_default().into());
    let color = match activity {
        Some(a) => color_index(&a.color),
        // Nächste Farbe, die noch keine Aktivität hat
        None => (0..crate::CALENDAR_COLORS.len() as i32)
            .find(|&i| !activities.iter().any(|a| color_index(&a.color) == i))
            .unwrap_or(0),
    };
    app.set_activity_color(color);
    app.set_activity_limit_options(limit_options());
    app.set_activity_limit(limit_index(
        activity.map_or(DEFAULT_LIMIT, |a| a.idle_limit),
    ));
    app.set_activity_error(Default::default());
    if let Some(a) = activity {
        let time = state
            .store
            .activity_totals(DateTime::UNIX_EPOCH, Utc::now() + TimeDelta::days(1))
            .ok()
            .and_then(|t| t.get(&a.id).copied())
            .unwrap_or_default();
        app.set_activity_delete_message(
            tr!(
                "Delete “{0}” with its counted time ({1})? This can't be undone.",
                a.name,
                short(time)
            )
            .into(),
        );
    }
    app.set_activity_dialog_open(true);
}

pub fn open_new(app: &AppWindow, state: &mut State) {
    open(app, state, None);
}

pub fn open_edit(app: &AppWindow, state: &mut State, id: i32) {
    let activity = state
        .store
        .activities()
        .unwrap_or_default()
        .into_iter()
        .find(|a| a.id == i64::from(id));
    if let Some(activity) = activity {
        open(app, state, Some(&activity));
    }
}

/// Eingegebenen Dateinamen prüfen: "Programm", "programm.exe" oder ein
/// ganzer Pfad werden zu "programm.exe"
fn file_name(text: &str) -> Option<String> {
    let text = text.trim().trim_matches('"').trim();
    let name = text.rsplit(['\\', '/']).next().unwrap_or(text).trim();
    if name.is_empty() || name.contains(['<', '>', ':', '"', '|', '?', '*']) {
        return None;
    }
    if name.to_lowercase().ends_with(".exe") {
        Some(name.to_string())
    } else if name.contains('.') {
        None
    } else {
        Some(format!("{name}.exe"))
    }
}

pub fn add_app(app: &AppWindow, state: &mut State, text: &str) -> bool {
    let Some(name) = file_name(text) else {
        app.set_activity_error(
            tr!("Please enter the program's file name, e.g. program.exe.").into(),
        );
        return false;
    };
    app.set_activity_error(Default::default());
    let model = &state.activity_dialog.apps;
    if let Some(i) = model
        .iter()
        .position(|c| c.name.eq_ignore_ascii_case(&name))
    {
        let mut row = model.row_data(i).unwrap_or_default();
        row.checked = true;
        model.set_row_data(i, row);
        return true;
    }
    // Nach den gewählten einreihen
    let at = model.iter().take_while(|c| c.checked).count();
    model.insert(
        at,
        AppChoice {
            name: name.into(),
            checked: true,
            note: Default::default(),
        },
    );
    true
}

pub fn toggle_app(state: &mut State, index: i32, checked: bool) {
    let model = &state.activity_dialog.apps;
    if let Some(mut row) = model.row_data(index.max(0) as usize) {
        row.checked = checked;
        model.set_row_data(index as usize, row);
    }
}

pub fn save(app: &AppWindow, state: &mut State) {
    let name = app.get_activity_name().trim().to_string();
    if name.is_empty() {
        app.set_activity_error(tr!("Please enter a name.").into());
        return;
    }
    let apps: Vec<String> = state
        .activity_dialog
        .apps
        .iter()
        .filter(|c| c.checked)
        .map(|c| c.name.to_string())
        .collect();
    if apps.is_empty() {
        app.set_activity_error(tr!("Please choose at least one program.").into());
        return;
    }
    let limit = LIMITS
        .get(app.get_activity_limit().max(0) as usize)
        .copied()
        .unwrap_or(DEFAULT_LIMIT);
    let color = color_hex(app.get_activity_color());
    let draft = ActivityDraft {
        name: &name,
        color: &color,
        idle_limit: limit,
        apps: &apps,
    };
    let result = match state.activity_dialog.editing {
        Some(id) => state.store.update_activity(id, &draft),
        None => state.store.add_activity(&draft).map(|_| ()),
    };
    if let Err(e) = result {
        app.set_activity_error(e.to_string().into());
        return;
    }
    app.set_activity_dialog_open(false);
    apply(state);
    show(app, state);
}

pub fn delete(app: &AppWindow, state: &mut State) {
    let Some(id) = state.activity_dialog.editing else {
        return;
    };
    if let Err(e) = state.store.delete_activity(id) {
        app.set_activity_error(e.to_string().into());
        return;
    }
    app.set_activity_dialog_open(false);
    apply(state);
    show(app, state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_cleaned_up() {
        assert_eq!(file_name("notepad"), Some("notepad.exe".into()));
        assert_eq!(file_name(" Notepad.EXE "), Some("Notepad.EXE".into()));
        assert_eq!(
            file_name(r#""C:\Program Files\Editor\editor.exe""#),
            Some("editor.exe".into())
        );
        assert_eq!(file_name("C:/tools/run.exe"), Some("run.exe".into()));
        assert_eq!(file_name("document.txt"), None);
        assert_eq!(file_name("a?b"), None);
        assert_eq!(file_name("   "), None);
    }

    #[test]
    fn durations_are_short() {
        assert_eq!(short(TimeDelta::seconds(59)), "0 min");
        assert_eq!(short(TimeDelta::minutes(45)), "45 min");
        assert_eq!(short(TimeDelta::minutes(200)), "3:20 h");
        assert_eq!(short(TimeDelta::hours(120)), "120:00 h");
    }

    #[test]
    fn limits_snap_to_a_choice() {
        assert_eq!(limit_index(60), 1);
        assert_eq!(limit_index(45), 1);
        assert_eq!(limit_index(10), 0);
        assert_eq!(limit_index(100_000), LIMITS.len() as i32 - 1);
    }
}
