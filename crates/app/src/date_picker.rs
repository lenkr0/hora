//! Datumsauswahl an Datumsfeldern: ein Monatsblatt wie der Mini-Kalender in
//! der Seitenleiste. Rust füllt es, Slint zeigt es; offen ist immer höchstens
//! eine Auswahl.

use chrono::{Datelike, Days, Duration, Months, NaiveDate};
use hora_i18n::dates;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::forms::{format_date, parse_date};
use crate::{AppWindow, DatePick, MiniDay, State, monday_of};

/// Zustand der offenen Auswahl
#[derive(Debug, Clone, Copy, Default)]
pub struct Picker {
    /// Erster des gezeigten Monats
    month: NaiveDate,
    /// Tag aus dem Feld, falls dort ein gültiges Datum steht
    chosen: Option<NaiveDate>,
    /// Tag unter dem Tastaturfokus
    cursor: NaiveDate,
}

fn first_of(day: NaiveDate) -> NaiveDate {
    day.with_day(1).unwrap_or(day)
}

/// Sechs Wochen ab dem Montag vor dem Monatsersten
pub fn sheet(month: NaiveDate, today: NaiveDate, selected: Option<NaiveDate>) -> Vec<MiniDay> {
    let first = monday_of(first_of(month));
    (0..42)
        .map(|i| {
            let date = first + Days::new(i);
            MiniDay {
                number: date.day().to_string().into(),
                today: date == today,
                other_month: date.month() != month.month(),
                selected: Some(date) == selected,
            }
        })
        .collect()
}

/// Auswahl öffnen: der Tag aus dem Feld, sonst heute
pub fn open(app: &AppWindow, state: &mut State, text: &str) {
    let today = state.now().date();
    let chosen = parse_date(text, today);
    let day = chosen.unwrap_or(today);
    state.picker = Picker {
        month: first_of(day),
        chosen,
        cursor: day,
    };
    fill(app, state);
}

/// Monate blättern; der Fokus bleibt am selben Tag, soweit der Monat ihn hat
pub fn shift(app: &AppWindow, state: &mut State, months: i32) {
    let p = &mut state.picker;
    let step = Months::new(months.unsigned_abs());
    let moved = if months >= 0 {
        p.month.checked_add_months(step)
    } else {
        p.month.checked_sub_months(step)
    };
    let Some(month) = moved else { return };
    p.month = month;
    p.cursor = (1..=p.cursor.day())
        .rev()
        .find_map(|d| month.with_day(d))
        .unwrap_or(month);
    fill(app, state);
}

/// Fokus um Tage verschieben; verlässt er den Monat, blättert das Blatt mit
pub fn move_cursor(app: &AppWindow, state: &mut State, days: i32) {
    let p = &mut state.picker;
    let Some(cursor) = p.cursor.checked_add_signed(Duration::days(i64::from(days))) else {
        return;
    };
    p.cursor = cursor;
    p.month = first_of(cursor);
    fill(app, state);
}

/// Tag an dieser Stelle des Blatts als Text fürs Feld
pub fn pick(state: &State, index: i32) -> String {
    let index = u64::try_from(index.clamp(0, 41)).unwrap_or(0);
    format_date(monday_of(state.picker.month) + Days::new(index))
}

fn fill(app: &AppWindow, state: &State) {
    let p = state.picker;
    let days = sheet(p.month, state.now().date(), p.chosen);
    let cursor = (p.cursor - monday_of(p.month)).num_days();
    let g = app.global::<DatePick>();
    g.set_month(dates::capitalize(dates::month(p.month.month0() as usize)).into());
    g.set_year(p.month.year().to_string().into());
    g.set_days(ModelRc::new(VecModel::from(days)));
    g.set_cursor(if (0..42).contains(&cursor) {
        cursor as i32
    } else {
        -1
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn the_sheet_starts_on_monday_and_marks_days() {
        // November 2026 beginnt an einem Sonntag
        let days = sheet(d(2026, 11, 1), d(2026, 11, 13), Some(d(2026, 11, 20)));
        assert_eq!(days.len(), 42);
        assert_eq!(days[0].number, "26");
        assert!(days[0].other_month);
        assert_eq!(days[6].number, "1");
        assert!(!days[6].other_month);
        assert!(days[18].today);
        assert!(days[25].selected);
        assert_eq!(days.iter().filter(|x| x.selected).count(), 1);
    }
}
