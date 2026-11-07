//! Schnelleingabe (Calinos QuickAdd): Vorschau und Anlegen aus einem Satz.
//! Den Text liest [`hora_core::quick`]; hier kommen Kalender, Vorschau und
//! der Weg zum Termin bzw. zur Aufgabe dazu.

use chrono::{Duration, NaiveTime};
use hora_core::model::Kind;
use hora_core::quick::{self, Parsed, When};
use hora_core::time::DateTimeValue;
use slint::Color;

use crate::editing::{self, DialogInput, ReminderInput};
use crate::forms::{DraftTimes, describe, format_date, format_time, short_day};
use crate::repeat::RuleFields;
use crate::{AppWindow, QuickPreview, State, calendar_color, reminders, rgb, tasks};
use hora_i18n::{dates, tr};

/// Ergebnis der Auslegung
pub struct Plan {
    pub parsed: Parsed,
    pub when: When,
    /// Zielkalender; -1 = Beispieltermine, `None` = keiner passt
    pub calendar: Option<i64>,
    pub problem: String,
}

/// Standardkalender für neue Termine (wie im Termin-Dialog)
fn event_calendar(state: &State) -> Option<i64> {
    let ids: Vec<i64> = state
        .calendars
        .iter()
        .filter(|c| !c.read_only && c.supports(Kind::Event))
        .map(|c| c.id)
        .collect();
    let preferred = state
        .store
        .setting("default_calendar")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|id| ids.contains(id) || (*id == -1 && state.demo_visible));
    preferred
        .or_else(|| ids.first().copied())
        .or((state.demo_visible || ids.is_empty()).then_some(-1))
}

/// Text auslegen; `force_task` für die Eingabezeile der Aufgaben-Ansicht
pub fn plan(state: &State, text: &str, force_task: bool) -> Plan {
    let now = state.now();
    let mut parsed = quick::parse(text, now.date());
    parsed.task |= force_task;
    let when = parsed.when(now);
    let kind = if parsed.task { Kind::Todo } else { Kind::Event };
    let mut problem = String::new();
    let calendar = match &parsed.calendar {
        Some(name) => {
            let wanted = name.to_lowercase();
            let found = state
                .calendars
                .iter()
                .filter(|c| !c.read_only && c.supports(kind))
                .find(|c| c.name.to_lowercase().starts_with(&wanted))
                .map(|c| c.id);
            if found.is_none() {
                problem = if parsed.task {
                    tr!("No calendar “{}” for tasks.", name)
                } else {
                    tr!("No calendar “{}” for events.", name)
                };
            }
            found
        }
        None if parsed.task => {
            let found = tasks::default_calendar(state);
            if found.is_none() {
                problem = tr!("No calendar for tasks available.").into();
            }
            found
        }
        None => event_calendar(state),
    };
    if calendar == Some(-1) && parsed.rule.is_some() {
        problem = tr!("Sample events can't repeat; please connect an account first.").into();
    }
    Plan {
        parsed,
        when,
        calendar,
        problem,
    }
}

/// Beginn als Wert (für die Wiederholungsregel)
fn start_value(when: &When, home: chrono_tz::Tz) -> Option<DateTimeValue> {
    match when {
        When::Timed { start, .. } => Some(DateTimeValue::Zoned(*start, home)),
        When::AllDay { first, .. } => Some(DateTimeValue::Date(*first)),
        When::Due { day, .. } => Some(DateTimeValue::Date(*day)),
        When::Undated => None,
    }
}

fn rule_fields(plan: &Plan, home: chrono_tz::Tz) -> RuleFields {
    match (&plan.parsed.rule, start_value(&plan.when, home)) {
        (Some(rule), Some(start)) => {
            let rrule = rule.simple().to_rrule(&start, home);
            RuleFields::from_rule(Some(&rrule), &start, home)
        }
        _ => RuleFields::default(),
    }
}

/// Vorschau für das Fenster
pub fn preview(state: &State, text: &str) -> QuickPreview {
    let home = state.home;
    let plan = plan(state, text, false);
    let p = &plan.parsed;
    let when = match &plan.when {
        When::Timed { start, end } => describe(*start, *end, false),
        When::AllDay { first, last } => describe(
            first.and_time(NaiveTime::MIN),
            (*last + Duration::days(1)).and_time(NaiveTime::MIN),
            true,
        ),
        When::Due { day, time: Some(t) } => format!("{}, {}", short_day(*day), format_time(*t)),
        When::Due { day, time: None } => short_day(*day),
        When::Undated => tr!("no due date").into(),
    };
    let repeat = match (&p.rule, start_value(&plan.when, home)) {
        (Some(_), Some(start)) => {
            let fields = rule_fields(&plan, home);
            fields
                .read(state.now().date())
                .map(|r| r.describe(start.naive().date()))
                .unwrap_or_default()
        }
        _ => String::new(),
    };
    let (calendar, color) = match plan.calendar {
        Some(-1) => (
            tr!("Sample events (only on this device)").to_string(),
            rgb(crate::DEMO_COLOR),
        ),
        Some(id) => state
            .calendars
            .iter()
            .enumerate()
            .find(|(_, c)| c.id == id)
            .map(|(i, c)| {
                let account = state
                    .accounts
                    .iter()
                    .find(|a| a.id == c.account_id)
                    .map_or("", |a| a.name.as_str());
                (format!("{} · {account}", c.name), rgb(calendar_color(c, i)))
            })
            .unwrap_or_default(),
        None => (String::new(), Color::default()),
    };
    let all_day = matches!(plan.when, When::AllDay { .. });
    let reminder = match (p.task, p.reminder) {
        (false, Some(m)) => reminders::label(m, all_day),
        (false, None) if !all_day && plan.calendar != Some(-1) => state
            .settings
            .default_reminder
            .map(|m| tr!("{} (default)", reminders::label(m, false)))
            .unwrap_or_default(),
        _ => String::new(),
    };
    let title = if p.title.is_empty() {
        "…".to_string()
    } else {
        p.title.clone()
    };
    QuickPreview {
        valid: !p.title.is_empty() && plan.calendar.is_some() && plan.problem.is_empty(),
        kind: if p.task { tr!("Task") } else { tr!("Event") }.into(),
        task: p.task,
        title: title.into(),
        when: when.into(),
        location: p.location.clone().unwrap_or_default().into(),
        repeat: repeat.into(),
        calendar: calendar.into(),
        calendar_color: color,
        reminder: reminder.into(),
        problem: plan.problem.clone().into(),
    }
}

/// Anlegen oder im Dialog öffnen. Liefert den Tag, zu dem die Ansicht springen soll.
pub fn submit(
    app: &AppWindow,
    state: &mut State,
    text: &str,
    open: bool,
    force_task: bool,
) -> Result<Option<chrono::NaiveDate>, String> {
    let home = state.home;
    let plan = plan(state, text, force_task);
    let p = &plan.parsed;
    if p.title.is_empty() && !open {
        return Err(tr!("Please enter a title.").into());
    }
    if !plan.problem.is_empty() && !open {
        return Err(plan.problem.clone());
    }
    let calendar = plan.calendar.unwrap_or(-1);
    if p.task {
        let (due_mode, due_date, due_time) = match plan.when {
            When::Due { day, time: Some(t) } => (2, format_date(day), format_time(t)),
            When::Due { day, time: None } => (1, format_date(day), dates::clock(18, 0)),
            _ => (0, String::new(), dates::clock(18, 0)),
        };
        let input = tasks::TaskInput {
            title: p.title.clone(),
            due_mode,
            due_date: due_date.clone(),
            due_time: due_time.clone(),
            calendar,
            ..tasks::TaskInput::default()
        };
        if open {
            crate::open_task_dialog(app, state, None, None);
            let mut draft = app.get_task_draft();
            draft.title = p.title.as_str().into();
            draft.due_mode = due_mode;
            draft.due_date = due_date.into();
            draft.due_time = due_time.into();
            if let Some(index) = state
                .task_calendar_ids
                .iter()
                .position(|id| *id == calendar)
            {
                draft.calendar = index as i32;
            }
            app.set_task_draft(draft);
            return Ok(None);
        }
        let items = state.tasks.clone();
        tasks::save(state, &items, None, &input)?;
        return Ok(None);
    }

    let (times, day) = match plan.when {
        When::Timed { start, end } => (
            DraftTimes::from_values(
                &DateTimeValue::Floating(start),
                &DateTimeValue::Floating(end),
                chrono_tz::UTC,
            ),
            start.date(),
        ),
        When::AllDay { first, last } => (
            DraftTimes::from_values(
                &DateTimeValue::Date(first),
                &DateTimeValue::Date(last + Duration::days(1)),
                chrono_tz::UTC,
            ),
            first,
        ),
        // Kommt bei Terminen nicht vor; sicherheitshalber ganztägig heute
        When::Due { .. } | When::Undated => {
            let today = state.now().date();
            (
                DraftTimes::from_values(
                    &DateTimeValue::Date(today),
                    &DateTimeValue::Date(today + Duration::days(1)),
                    chrono_tz::UTC,
                ),
                today,
            )
        }
    };
    let rule = rule_fields(&plan, home);
    let all_day = times.all_day;
    let minutes: Vec<i64> = match p.reminder {
        Some(m) => vec![m],
        None if !all_day => state.settings.default_reminder.into_iter().collect(),
        None => Vec::new(),
    };
    if open {
        crate::open_dialog(app, state, None, times);
        let mut draft = app.get_event_draft();
        draft.title = p.title.as_str().into();
        draft.location = p.location.clone().unwrap_or_default().into();
        draft.custom_rule = false;
        draft.rule = crate::rule_draft(&rule);
        if let Some(index) = state.option_ids.iter().position(|id| *id == calendar) {
            draft.calendar = index as i32;
        }
        app.set_event_draft(draft);
        if let Some(m) = p.reminder {
            state.reminder_rows.set_vec(vec![crate::ReminderChip {
                label: reminders::label(m, all_day).into(),
                minutes: m as i32,
                keep: -1,
            }]);
        }
        return Ok(None);
    }
    let input = DialogInput {
        title: p.title.clone(),
        times,
        location: p.location.clone().unwrap_or_default(),
        description: String::new(),
        calendar,
        rule,
        reminders: Some(ReminderInput {
            minutes,
            keep: Vec::new(),
        }),
    };
    editing::save(state, None, &input)?;
    Ok(Some(day))
}
