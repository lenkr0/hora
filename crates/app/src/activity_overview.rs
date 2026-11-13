//! App-Zeiten: Zeit dieser Woche in der Seitenleiste und die Übersicht einer
//! Aktivität (Summen, Verlauf, Sitzungen). Gerechnet wird beim Öffnen und,
//! solange das Fenster sichtbar ist, mit der Uhr der Jetzt-Linie; bei
//! ausgeschalteten App-Zeiten gar nicht.

use chrono::{DateTime, Datelike, Months, NaiveDate, NaiveTime, TimeDelta, Utc};
use hora_core::Tz;
use hora_core::activity::{SESSION_GAP, Span, per_range, sessions};
use hora_i18n::{dates, tr, trn};
use hora_storage::Activity;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::activity_ui::{color_of, short};
use crate::{
    ActivityBar, ActivityRow, ActivitySession, ActivityStats, AppWindow, SelectOption, State,
};

/// So viele Wochen oder Monate zeigt der Verlauf
const BARS: u32 = 12;
/// So viele Sitzungen zeigt die Liste, die neuesten zuerst
const RECENT: usize = 30;

fn utc(date: NaiveDate, home: Tz) -> DateTime<Utc> {
    hora_core::time::local_to_utc(date.and_time(NaiveTime::MIN), home)
}

/// Seitenleiste: Aktivitäten mit der Zeit dieser Woche; leer, wenn die
/// App-Zeiten aus sind
pub fn sidebar(app: &AppWindow, state: &State) {
    let rows = if state.settings.activities {
        sidebar_rows(state)
    } else {
        Vec::new()
    };
    app.set_activity_sidebar(ModelRc::new(VecModel::from(rows)));
}

fn sidebar_rows(state: &State) -> Vec<ActivityRow> {
    let activities = state.store.activities().unwrap_or_default();
    if activities.is_empty() {
        return Vec::new();
    }
    let week = state
        .store
        .activity_totals(
            crate::activity_ui::week_start(state),
            Utc::now() + TimeDelta::days(1),
        )
        .unwrap_or_default();
    activities
        .iter()
        .map(|activity| ActivityRow {
            id: activity.id as i32,
            name: activity.name.clone().into(),
            color: color_of(activity),
            detail: short(week.get(&activity.id).copied().unwrap_or_default()).into(),
        })
        .collect()
}

/// Mit der Uhr: Seitenleiste nachziehen, solange das Fenster sichtbar ist
pub fn tick(app: &AppWindow, state: &State) {
    if state.settings.activities && app.window().is_visible() {
        sidebar(app, state);
    }
}

/// Übersicht einer Aktivität öffnen (aus der Seitenleiste)
pub fn open(app: &AppWindow, state: &mut State, id: i32) {
    state.activity_shown = Some(i64::from(id));
    fill(app, state);
    app.set_activity_overview_open(true);
}

/// Andere Aktivität in der Auswahl
pub fn select(app: &AppWindow, state: &mut State, index: i32) {
    let activities = state.store.activities().unwrap_or_default();
    let Some(activity) = activities.get(index.max(0) as usize) else {
        return;
    };
    if state.activity_shown != Some(activity.id) {
        state.activity_shown = Some(activity.id);
        fill(app, state);
    }
}

/// Verlauf zwischen Wochen und Monaten umgeschaltet
pub fn range(app: &AppWindow, state: &mut State) {
    fill(app, state);
}

fn fill(app: &AppWindow, state: &mut State) {
    let activities = state.store.activities().unwrap_or_default();
    let index = activities
        .iter()
        .position(|a| Some(a.id) == state.activity_shown)
        .unwrap_or(0);
    let Some(activity) = activities.get(index) else {
        app.set_activity_overview_open(false);
        return;
    };
    state.activity_shown = Some(activity.id);
    let options: Vec<SelectOption> = activities
        .iter()
        .map(|a| SelectOption {
            id: a.id as i32,
            name: a.name.clone().into(),
            color: color_of(a),
        })
        .collect();
    app.set_activity_overview_options(ModelRc::new(VecModel::from(options)));
    app.set_activity_overview_index(index as i32);
    let spans = state.store.activity_spans(activity.id).unwrap_or_default();
    let months = app.get_activity_overview_months();
    app.set_activity_stats(stats(activity, &spans, months, state.home, Utc::now()));
}

/// Grenzen des Verlaufs: die letzten [`BARS`] Wochen (ab Montag) oder Monate,
/// die laufende zuletzt; eine Grenze mehr als Balken
fn bounds(today: NaiveDate, months: bool) -> Vec<NaiveDate> {
    if months {
        let first = today.with_day(1).unwrap_or(today);
        let start = first
            .checked_sub_months(Months::new(BARS - 1))
            .unwrap_or(first);
        (0..=BARS)
            .filter_map(|i| start.checked_add_months(Months::new(i)))
            .collect()
    } else {
        let start = crate::monday_of(today) - TimeDelta::weeks(i64::from(BARS) - 1);
        (0..=i64::from(BARS))
            .map(|i| start + TimeDelta::weeks(i))
            .collect()
    }
}

/// Oberste Linie des Verlaufs: der kleinste runde Wert, der den größten
/// Balken fasst. Die mittlere Linie liegt bei der Hälfte, auch sie rund.
fn scale(max: TimeDelta) -> TimeDelta {
    const MINUTES: [i64; 20] = [
        30, 60, 120, 240, 360, 480, 600, 720, 960, 1200, 1440, 1800, 2400, 3000, 3600, 4800, 6000,
        7200, 9600, 12000,
    ];
    let minutes = max.num_minutes() + i64::from(max.num_seconds() % 60 > 0);
    let top = MINUTES
        .iter()
        .copied()
        .find(|&m| m >= minutes)
        // Darüber in Schritten von 100 Stunden
        .unwrap_or_else(|| (minutes + 5999) / 6000 * 6000);
    TimeDelta::minutes(top)
}

/// Beschriftung einer Linie: "15 min", "3 h"
fn scale_label(d: TimeDelta) -> String {
    let minutes = d.num_minutes();
    if minutes < 60 {
        tr!("{} min", minutes)
    } else {
        tr!("{} h", minutes / 60)
    }
}

fn stats(
    activity: &Activity,
    spans: &[Span],
    months: bool,
    home: Tz,
    now: DateTime<Utc>,
) -> ActivityStats {
    let local = |at: DateTime<Utc>| at.with_timezone(&home).naive_local();
    let today = local(now).date();
    let total = spans
        .iter()
        .fold(TimeDelta::zero(), |sum, (start, end)| sum + (*end - *start));

    let monday = crate::monday_of(today);
    let first_of_month = today.with_day(1).unwrap_or(today);
    let next_month = first_of_month
        .checked_add_months(Months::new(1))
        .unwrap_or(today);
    let week_and_month = per_range(
        spans,
        &[utc(monday, home), utc(monday + TimeDelta::weeks(1), home)],
    )
    .into_iter()
    .chain(per_range(
        spans,
        &[utc(first_of_month, home), utc(next_month, home)],
    ))
    .collect::<Vec<_>>();

    let edges = bounds(today, months);
    let utc_edges: Vec<DateTime<Utc>> = edges.iter().map(|&d| utc(d, home)).collect();
    let values = per_range(spans, &utc_edges);
    let top = scale(values.iter().copied().max().unwrap_or_default());
    let bars: Vec<ActivityBar> = edges
        .windows(2)
        .zip(&values)
        .enumerate()
        .map(|(i, (range, &value))| {
            let (label, title) = if months {
                let m0 = range[0].month0() as usize;
                (
                    dates::month_short(m0).to_string(),
                    dates::month_year(range[0].year(), m0),
                )
            } else {
                (
                    dates::day_month_short(range[0]),
                    dates::short_day_range(range[0], range[1] - TimeDelta::days(1)),
                )
            };
            ActivityBar {
                label: label.into(),
                title: title.into(),
                text: short(value).into(),
                value: (value.num_seconds() as f64 / top.num_seconds() as f64) as f32,
                current: i + 1 == values.len(),
            }
        })
        .collect();

    let found = sessions(spans, SESSION_GAP);
    let summary = match found.iter().map(|s| s.counted).max() {
        Some(longest) => {
            let count = found.len() as i64;
            let average = TimeDelta::seconds(
                found.iter().map(|s| s.counted.num_seconds()).sum::<i64>() / count,
            );
            tr!(
                "{0} · average {1} · longest {2}",
                trn!("{n} session", "{n} sessions", count),
                short(average),
                short(longest)
            )
        }
        None => String::new(),
    };
    let recent: Vec<ActivitySession> = found
        .iter()
        .rev()
        .take(RECENT)
        .map(|s| {
            let (start, end) = (local(s.start), local(s.end));
            let day = start.date();
            ActivitySession {
                day: if day.year() == today.year() {
                    dates::short_weekday_date(day)
                } else {
                    dates::day_month_year(day)
                }
                .into(),
                span: format!("{}–{}", dates::time(start.time()), dates::time(end.time())).into(),
                length: short(s.counted).into(),
            }
        })
        .collect();

    ActivityStats {
        color: color_of(activity),
        total: short(total).into(),
        since: spans
            .iter()
            .map(|(start, _)| *start)
            .min()
            .map(|first| tr!("since {}", dates::day_month_year(local(first).date())))
            .unwrap_or_default()
            .into(),
        week: short(week_and_month[0]).into(),
        week_range: dates::short_day_range(monday, monday + TimeDelta::days(6)).into(),
        month: short(week_and_month[1]).into(),
        month_name: dates::month_year(today.year(), today.month0() as usize).into(),
        scale_top: scale_label(top).into(),
        scale_mid: scale_label(top / 2).into(),
        bars: ModelRc::new(VecModel::from(bars)),
        summary: summary.into(),
        sessions: ModelRc::new(VecModel::from(recent)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn history_ends_with_the_running_week_or_month() {
        // Donnerstag, 12. November 2026
        let weeks = bounds(day(2026, 11, 12), false);
        assert_eq!(weeks.len(), BARS as usize + 1);
        assert_eq!(weeks[0], day(2026, 8, 24));
        assert_eq!(weeks[BARS as usize - 1], day(2026, 11, 9));
        assert_eq!(weeks[BARS as usize], day(2026, 11, 16));
        let months = bounds(day(2026, 11, 12), true);
        assert_eq!(months.len(), BARS as usize + 1);
        assert_eq!(months[0], day(2025, 12, 1));
        assert_eq!(months[BARS as usize], day(2026, 12, 1));
    }

    #[test]
    fn the_scale_is_round() {
        assert_eq!(scale(TimeDelta::zero()), TimeDelta::minutes(30));
        assert_eq!(scale(TimeDelta::minutes(30)), TimeDelta::minutes(30));
        assert_eq!(scale(TimeDelta::seconds(30 * 60 + 1)), TimeDelta::hours(1));
        assert_eq!(scale(TimeDelta::minutes(250)), TimeDelta::hours(6));
        assert_eq!(scale(TimeDelta::hours(150)), TimeDelta::hours(160));
        assert_eq!(scale(TimeDelta::hours(250)), TimeDelta::hours(300));
        // Die Hälfte ist ebenfalls rund
        assert_eq!(scale_label(TimeDelta::minutes(30) / 2), "15 min");
        assert_eq!(scale_label(TimeDelta::hours(6) / 2), "3 h");
    }

    #[test]
    fn stats_count_week_month_and_sessions() {
        let home: Tz = "Europe/Berlin".parse().unwrap();
        let at = |d: NaiveDate, h: u32, m: u32| {
            hora_core::time::local_to_utc(d.and_hms_opt(h, m, 0).unwrap(), home)
        };
        let activity = Activity {
            id: 1,
            name: "Writing".into(),
            color: "#34a853".into(),
            idle_limit: 60,
            apps: vec!["writer.exe".into()],
        };
        let spans = [
            // Oktober: eine Stunde
            (at(day(2026, 10, 20), 18, 0), at(day(2026, 10, 20), 19, 0)),
            // Montag dieser Woche: 30 + 20 min mit 5 min Pause, eine Sitzung
            (at(day(2026, 11, 9), 19, 0), at(day(2026, 11, 9), 19, 30)),
            (at(day(2026, 11, 9), 19, 35), at(day(2026, 11, 9), 19, 55)),
            // Sonntag davor bis nach Mitternacht: 30 min, davon 10 in dieser Woche
            (at(day(2026, 11, 8), 23, 40), at(day(2026, 11, 9), 0, 10)),
        ];
        let now = at(day(2026, 11, 12), 12, 0);
        let s = hora_i18n::with_language(hora_i18n::Lang::En, || {
            stats(&activity, &spans, false, home, now)
        });
        assert_eq!(s.total, "2:20 h");
        assert_eq!(s.week, "1:00 h");
        assert_eq!(s.month, "1:20 h");
        assert_eq!(s.scale_top, "1 h");
        let bars: Vec<ActivityBar> = s.bars.iter().collect();
        assert_eq!(bars.len(), BARS as usize);
        assert!(bars[BARS as usize - 1].current);
        assert_eq!(bars[BARS as usize - 1].text, "1:00 h");
        assert_eq!(bars[BARS as usize - 2].text, "20 min");
        assert_eq!(s.summary, "3 sessions · average 46 min · longest 1:00 h");
        let recent: Vec<ActivitySession> = s.sessions.iter().collect();
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].length, "50 min");
        assert_eq!(recent[0].span, "19:00–19:55");
    }
}
