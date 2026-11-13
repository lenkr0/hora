//! App-Zeiten im Kalender: Ein Termin kann zu einer Aktivität gehören und
//! zeigt dann, wie viel davon gezählt wurde ("1:12 / 1:30 h"). Gezählt wird
//! jede Sitzung der Aktivität, die in den Termin hineinreicht: Wer etwas
//! früher anfängt oder länger macht, dem zählt die ganze Sitzung.
//!
//! Die Zuordnung gilt für die UID, also für alle Vorkommen einer Serie, und
//! bleibt auf diesem PC; am Termin auf dem Server ändert sich nichts.
//! Beispieltermine haben keine feste Kennung und lassen sich nicht zuordnen.

use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};
use hora_core::activity::{SESSION_GAP, Session, sessions};
use hora_i18n::tr;
use slint::{Model, ModelRc, VecModel};

use crate::activity_ui::color_of;
use crate::items::{Item, Source};
use crate::{AppWindow, SelectOption, State};

/// Fortschritt eines Termins in der Wochenansicht
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    /// "1:12 / 1:30 h"
    pub text: String,
    /// Gezählt geteilt durch geplant; ab 1 erreicht
    pub fraction: f32,
}

/// Dauer als Stunden und Minuten: "1:05"
pub fn hm(d: TimeDelta) -> String {
    let minutes = d.num_minutes().max(0);
    format!("{}:{:02}", minutes / 60, minutes % 60)
}

/// UID eines Termins vom Server
pub fn uid_of(item: &Item) -> Option<&str> {
    match &item.source {
        Source::Server { uid, .. } => Some(uid),
        _ => None,
    }
}

/// Gezählte Zeit für einen Termin: alle Sitzungen, die hineinreichen
fn counted(found: &[Session], start: DateTime<Utc>, end: DateTime<Utc>) -> TimeDelta {
    found
        .iter()
        .filter(|s| s.start < end && s.end > start)
        .fold(TimeDelta::zero(), |sum, s| sum + s.counted)
}

fn progress_of(counted: TimeDelta, planned: TimeDelta) -> Progress {
    Progress {
        text: tr!("{0} / {1} h", hm(counted), hm(planned)),
        fraction: if planned > TimeDelta::zero() {
            (counted.num_seconds() as f64 / planned.num_seconds() as f64) as f32
        } else {
            1.0
        },
    }
}

/// Fortschritt der begonnenen Termine mit Aktivität, nach Index in `items`.
/// Kostet nichts, solange die App-Zeiten aus sind oder kein Termin zugeordnet ist.
pub fn progress(state: &State, items: &[Item]) -> HashMap<usize, Progress> {
    let mut out = HashMap::new();
    if !state.settings.activities {
        return out;
    }
    let links = state.store.event_activities().unwrap_or_default();
    if links.is_empty() {
        return out;
    }
    let now = Utc::now();
    let mut by_activity: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        // Ganztägige und noch nicht begonnene Termine zeigen nichts
        if item.span.dates.is_some() || item.span.start > now {
            continue;
        }
        if let Some(&id) = uid_of(item).and_then(|uid| links.get(uid)) {
            by_activity.entry(id).or_default().push(i);
        }
    }
    for (activity, indices) in by_activity {
        let from = indices.iter().map(|&i| items[i].span.start).min();
        let to = indices.iter().map(|&i| items[i].span.end).max();
        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };
        // Sitzungen dürfen über den Termin hinausreichen
        let spans = state
            .store
            .activity_spans_between(activity, from - TimeDelta::days(1), to + TimeDelta::days(1))
            .unwrap_or_default();
        let found = sessions(&spans, SESSION_GAP);
        for i in indices {
            let span = &items[i].span;
            out.insert(
                i,
                progress_of(counted(&found, span.start, span.end), span.end - span.start),
            );
        }
    }
    out
}

/// Auswahl im Termin-Dialog: "Keine" und die Aktivitäten; leer, solange die
/// App-Zeiten aus sind oder es keine Aktivität gibt
pub fn dialog(app: &AppWindow, state: &State, item: Option<&Item>) {
    let activities = if state.settings.activities {
        state.store.activities().unwrap_or_default()
    } else {
        Vec::new()
    };
    if activities.is_empty() {
        app.set_event_activity_options(ModelRc::default());
        app.set_event_activity(0);
        return;
    }
    let mut options = vec![SelectOption {
        id: -1,
        name: tr!("None").into(),
        color: Default::default(),
    }];
    options.extend(activities.iter().map(|a| SelectOption {
        id: a.id as i32,
        name: a.name.clone().into(),
        color: color_of(a),
    }));
    let linked = item
        .and_then(uid_of)
        .and_then(|uid| state.store.event_activities().ok()?.get(uid).copied());
    let index = linked
        .and_then(|id| activities.iter().position(|a| a.id == id))
        .map_or(0, |i| i + 1);
    app.set_event_activity_options(ModelRc::new(VecModel::from(options)));
    app.set_event_activity(index as i32);
}

/// Wahl aus dem Termin-Dialog für den Termin mit dieser UID übernehmen.
/// Ohne Auswahl (App-Zeiten aus) bleibt eine bestehende Zuordnung erhalten.
pub fn save(app: &AppWindow, state: &State, uid: &str) {
    let options = app.get_event_activity_options();
    if options.row_count() == 0 {
        return;
    }
    let chosen = options
        .row_data(app.get_event_activity().max(0) as usize)
        .map(|o| i64::from(o.id))
        .filter(|&id| id >= 0);
    if let Err(e) = state.store.set_event_activity(uid, chosen) {
        crate::log::write(&format!(
            "App-Zeiten: Zuordnung des Termins nicht gespeichert: {e}"
        ));
    }
}

/// Eine Serie wurde geteilt ("dieses und folgende"): Die neue Serie gehört
/// zur selben Aktivität wie die alte
pub fn carry_over(state: &State, from: &str, to: &str) {
    let link = state
        .store
        .event_activities()
        .ok()
        .and_then(|links| links.get(from).copied());
    if let Some(id) = link {
        let _ = state.store.set_event_activity(to, Some(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(minutes: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 9, 18, 0, 0).unwrap() + TimeDelta::minutes(minutes)
    }

    #[test]
    fn sessions_reaching_into_the_event_count_whole() {
        // Termin 0–90 min; Sitzung beginnt 10 min früher und endet 20 min später
        let found = sessions(&[(t(-10), t(50)), (t(55), t(110))], SESSION_GAP);
        assert_eq!(counted(&found, t(0), t(90)), TimeDelta::minutes(115));
        // Eine Sitzung lange vorher zählt nicht
        let found = sessions(&[(t(-200), t(-100)), (t(30), t(60))], SESSION_GAP);
        assert_eq!(counted(&found, t(0), t(90)), TimeDelta::minutes(30));
        assert_eq!(counted(&[], t(0), t(90)), TimeDelta::zero());
    }

    #[test]
    fn progress_shows_hours_and_minutes() {
        let p = hora_i18n::with_language(hora_i18n::Lang::En, || {
            progress_of(TimeDelta::minutes(72), TimeDelta::minutes(90))
        });
        assert_eq!(p.text, "1:12 / 1:30 h");
        assert!((p.fraction - 0.8).abs() < 1e-6);
        assert_eq!(hm(TimeDelta::minutes(5)), "0:05");
        assert_eq!(
            progress_of(TimeDelta::zero(), TimeDelta::zero()).fraction,
            1.0
        );
    }
}
