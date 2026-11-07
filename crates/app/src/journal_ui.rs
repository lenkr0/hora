//! Journal-Ansicht an der Oberfläche: Liste und Editor füllen, Eingaben
//! nach einer halben Sekunde Pause speichern (wie Calino), beim Wechseln
//! des Eintrags, der Ansicht und beim Schließen sofort.

use std::time::Duration;

use chrono::{Datelike, NaiveDate};
use hora_core::journal::{JournalFields, clean_tags, word_count};
use hora_storage::OpState;
use slint::{ModelRc, SharedString, StyledText, VecModel};

use crate::journal::{self, BlockKind, DEMO, Entry, Saved};
use crate::{
    AppWindow, JournalBlock, JournalCalendar, JournalEdit, JournalRow, MONTHS, State, View, forms,
    rgb, with_ui,
};

/// Pause nach der letzten Eingabe, bis gespeichert wird (Calino: 500 ms)
const SAVE_DELAY: Duration = Duration::from_millis(500);
/// Zeilenhöhen der Liste (journal-view.slint)
const ROW_H: f32 = 90.0;
const TAG_ROW_H: f32 = 23.0;

/// Gewählter Eintrag (auch ein neuer Entwurf)
pub fn selected(state: &State) -> Option<Entry> {
    let key = state.journal_selected.as_ref()?;
    state
        .journal_entries
        .iter()
        .find(|e| &e.key() == key)
        .cloned()
        .or_else(|| state.journal_draft.clone().filter(|d| &d.key() == key))
}

fn calendar_can_be_created(state: &State) -> bool {
    !state.accounts.is_empty() && state.accounts.iter().any(|a| a.home_url.is_some())
}

/// Titel der Kopfzeile
pub fn title(state: &State) -> String {
    if state.journal_all {
        "Journal".into()
    } else {
        format!(
            "{} {}",
            MONTHS[state.anchor.month0() as usize],
            state.anchor.year()
        )
    }
}

/// Liste neu aufbauen; der Editor behält Titel und Text, solange der
/// Eintrag gewählt bleibt (nur fremde Änderungen werden übernommen).
pub fn load(app: &AppWindow, state: &mut State) {
    let all = journal::collect(state);
    let month = (!state.journal_all).then_some(state.anchor);
    state.journals = journal::filter(all.clone(), month);
    state.journal_entries = all;
    if let Some(key) = state.journal_selected.clone()
        && !state.journal_entries.iter().any(|e| e.key() == key)
    {
        state.journal_selected = None;
        state.journal_dirty = false;
    }
    let selected_key = state.journal_selected.clone();
    let mut y = 0.0f32;
    let rows: Vec<JournalRow> = state
        .journals
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let (day, weekday, month) = journal::date_labels(e.day);
            let height = ROW_H
                + if e.info.categories.is_empty() {
                    0.0
                } else {
                    TAG_ROW_H
                };
            let row = JournalRow {
                id: i as i32,
                day: day.into(),
                weekday: weekday.into(),
                month: month.into(),
                title: e.info.summary.trim().into(),
                preview: journal::preview(&e.info.description).into(),
                tags: ModelRc::new(VecModel::from(
                    e.info
                        .categories
                        .iter()
                        .map(|t| SharedString::from(t.to_uppercase()))
                        .collect::<Vec<_>>(),
                )),
                selected: selected_key.as_ref() == Some(&e.key()),
                y,
                height,
            };
            y += height;
            row
        })
        .collect();
    app.set_journal_rows(ModelRc::new(VecModel::from(rows)));
    app.set_journal_list_height(y);
    app.set_journal_count(state.journals.len() as i32);
    app.set_journal_mode(i32::from(state.journal_all));
    app.set_journal_list_title(
        if state.journal_all {
            "ALLE EINTRÄGE".to_string()
        } else {
            title(state).to_uppercase()
        }
        .into(),
    );
    let can_add = journal::default_calendar(state).is_some();
    app.set_journal_can_add(can_add);
    app.set_journal_can_create_calendar(!can_add && calendar_can_be_created(state));
    app.set_journal_create_busy(state.journal_create_busy);
    app.set_journal_create_error(
        state
            .journal_create_error
            .clone()
            .unwrap_or_default()
            .into(),
    );
    match selected(state) {
        Some(entry) => {
            // Fremde Änderung (anderes Programm) übernehmen, wenn hier nichts offen ist
            let stored = (
                entry.info.summary.trim().to_string(),
                entry.info.description.trim_end().to_string(),
            );
            if !state.journal_dirty && !entry.draft && stored != state.journal_loaded {
                app.set_journal_title(entry.info.summary.as_str().into());
                app.set_journal_body(entry.info.description.as_str().into());
                render(app, &entry.info.description);
                state.journal_loaded = stored;
            }
            meta(app, state, &entry);
            app.set_journal_open(true);
        }
        None => app.set_journal_open(false),
    }
}

/// Text der Leseansicht in Blöcke für StyledText umsetzen
fn render(app: &AppWindow, text: &str) {
    let styled = |md: &str| {
        StyledText::from_markdown(md).unwrap_or_else(|_| StyledText::from_plain_text(md))
    };
    let blocks: Vec<JournalBlock> = journal::blocks(text)
        .into_iter()
        .map(|b| {
            let (kind, md) = match b.kind {
                BlockKind::Paragraph => (0, b.text.clone()),
                BlockKind::Heading(level) => (
                    i32::from(level),
                    // Kleinste Überschrift fett in der Grundschrift
                    if level == 3 {
                        format!("**{}**", b.text)
                    } else {
                        b.text.clone()
                    },
                ),
                BlockKind::Quote => (4, b.text.clone()),
                BlockKind::Rule => (5, String::new()),
                BlockKind::Code => (6, String::new()),
            };
            JournalBlock {
                kind,
                text: styled(&md),
                plain: b.text.into(),
            }
        })
        .collect();
    app.set_journal_blocks(ModelRc::new(VecModel::from(blocks)));
}

/// Datum, Tags, Kalender, Wörter und Speicherstand des Editors
fn meta(app: &AppWindow, state: &State, entry: &Entry) {
    let (day, _, _) = journal::date_labels(entry.day);
    app.set_journal_day_num(day.into());
    app.set_journal_weekday_long(
        forms::WEEKDAYS[entry.day.weekday().num_days_from_monday() as usize].into(),
    );
    app.set_journal_month_year(
        format!(
            "{} {}",
            MONTHS[entry.day.month0() as usize],
            entry.day.year()
        )
        .into(),
    );
    app.set_journal_date_text(forms::format_date(entry.day).into());
    app.set_journal_tags(ModelRc::new(VecModel::from(
        entry
            .info
            .categories
            .iter()
            .map(|t| SharedString::from(t.as_str()))
            .collect::<Vec<_>>(),
    )));
    let calendars: Vec<JournalCalendar> = journal::calendars(state)
        .into_iter()
        .map(|(id, name, color)| JournalCalendar {
            id: id as i32,
            name: name.into(),
            color: rgb(color),
            selected: id == entry.calendar_id,
        })
        .collect();
    app.set_journal_calendars(ModelRc::new(VecModel::from(calendars)));
    app.set_journal_read_only(entry.read_only);
    app.set_journal_words(word_count(&app.get_journal_body()) as i32);
    let (status, text) = status(state, entry);
    app.set_journal_status(status);
    app.set_journal_status_text(text.into());
}

/// (Stufe, Text): 0 gespeichert, 1 offen, 2 wird übertragen, 3 nur lokal, 4 Fehler
fn status(state: &State, entry: &Entry) -> (i32, String) {
    if state.journal_dirty {
        return (1, "Nicht gespeicherte Änderungen".into());
    }
    if let Some(error) = &state.journal_error {
        return (4, error.clone());
    }
    if entry.draft {
        return (0, "Neuer Eintrag".into());
    }
    if entry.calendar_id == DEMO {
        // Wie die Beispieltermine nur im Speicher
        return (3, "Beispiel, wird nicht dauerhaft gespeichert".into());
    }
    match state.store.pending_state(entry.calendar_id, &entry.href) {
        Ok(None) => (0, "Gespeichert".into()),
        Ok(Some((OpState::Pending, 0))) => (2, "Speichere …".into()),
        Ok(Some((OpState::Pending, _))) => (3, "Lokal gespeichert, wird übertragen".into()),
        Ok(Some((OpState::Conflict, _))) => (4, "Konflikt mit dem Server".into()),
        Ok(Some((OpState::Failed, _))) => (4, "Synchronisierung fehlgeschlagen".into()),
        Err(e) => (4, e.to_string()),
    }
}

fn open_editor(app: &AppWindow, state: &mut State, entry: &Entry, write: bool) {
    app.set_journal_title(entry.info.summary.as_str().into());
    app.set_journal_body(entry.info.description.as_str().into());
    state.journal_loaded = (
        entry.info.summary.trim().to_string(),
        entry.info.description.trim_end().to_string(),
    );
    state.journal_error = None;
    let empty = entry.info.summary.trim().is_empty() && entry.info.description.trim().is_empty();
    app.set_journal_write(!entry.read_only && (write || empty));
    app.set_journal_date_error("".into());
    render(app, &entry.info.description);
}

/// Eintrag wählen (`None` schließt den Editor). Offene Eingaben werden
/// vorher gespeichert, ein leerer Entwurf verworfen.
pub fn select(app: &AppWindow, state: &mut State, key: Option<String>, write: bool) {
    flush(app, state);
    if state
        .journal_draft
        .as_ref()
        .is_some_and(|d| Some(d.key()) != key)
    {
        state.journal_draft = None;
    }
    state.journal_selected = key;
    if let Some(entry) = selected(state) {
        open_editor(app, state, &entry, write);
    }
    load(app, state);
    reveal(app, state);
}

/// Gewählte Zeile sichtbar machen
fn reveal(app: &AppWindow, state: &State) {
    let Some(key) = &state.journal_selected else {
        return;
    };
    let mut y = 0.0f32;
    for e in &state.journals {
        let h = ROW_H
            + if e.info.categories.is_empty() {
                0.0
            } else {
                TAG_ROW_H
            };
        if &e.key() == key {
            app.invoke_journal_reveal(y, h);
            return;
        }
        y += h;
    }
}

/// Pfeiltasten in der Liste: vorheriger bzw. nächster Eintrag
pub fn step(app: &AppWindow, state: &mut State, delta: i32) {
    let count = state.journals.len() as i32;
    if count == 0 {
        return;
    }
    let current = state
        .journal_selected
        .as_ref()
        .and_then(|k| state.journals.iter().position(|e| &e.key() == k))
        .map(|i| i as i32);
    let next = match current {
        Some(i) => (i + delta).clamp(0, count - 1),
        None if delta < 0 => count - 1,
        None => 0,
    };
    if current == Some(next) {
        return;
    }
    let key = state.journals[next as usize].key();
    select(app, state, Some(key), false);
}

/// Neuer Eintrag: heute, oder der Erste des gezeigten Monats (wie Calino)
pub fn new_entry(app: &AppWindow, state: &mut State) {
    flush(app, state);
    let today = state.now().date();
    let same_month = state.anchor.year() == today.year() && state.anchor.month() == today.month();
    let day = if state.journal_all || same_month {
        today
    } else {
        state.anchor.with_day(1).unwrap_or(state.anchor)
    };
    match journal::new_draft(state, day) {
        Ok(draft) => {
            let key = draft.key();
            state.journal_draft = Some(draft);
            select(app, state, Some(key), true);
            // Der Editor entsteht erst mit dem nächsten Bild
            slint::Timer::single_shot(Duration::from_millis(30), || {
                with_ui(|app, _| app.invoke_journal_focus_title());
            });
        }
        Err(message) => {
            state.journal_create_error = Some(message);
            load(app, state);
        }
    }
}

/// Eingabe im Titel oder Text: nach einer Pause speichern
pub fn edited(app: &AppWindow, state: &mut State) {
    state.journal_dirty = true;
    state.journal_error = None;
    app.set_journal_status(1);
    app.set_journal_status_text("Nicht gespeicherte Änderungen".into());
    app.set_journal_words(word_count(&app.get_journal_body()) as i32);
    state
        .journal_timer
        .start(slint::TimerMode::SingleShot, SAVE_DELAY, || {
            with_ui(|app, state| save_now(app, &mut state.borrow_mut()));
        });
}

/// Offene Eingaben sofort speichern
pub fn flush(app: &AppWindow, state: &mut State) {
    state.journal_timer.stop();
    if state.journal_dirty {
        save_now(app, state);
    }
}

fn save_now(app: &AppWindow, state: &mut State) {
    save_with(app, state, |_| {});
}

/// Felder des Editors (mit Änderung `change`) speichern
fn save_with(app: &AppWindow, state: &mut State, change: impl FnOnce(&mut JournalFields)) {
    state.journal_timer.stop();
    let Some(entry) = selected(state) else {
        state.journal_dirty = false;
        return;
    };
    if entry.read_only {
        state.journal_dirty = false;
        return;
    }
    let mut fields = JournalFields {
        summary: app.get_journal_title().to_string(),
        description: app.get_journal_body().to_string(),
        day: entry.day,
        categories: entry.info.categories.clone(),
    };
    change(&mut fields);
    fields.categories = clean_tags(&fields.categories);
    // Ein leerer Entwurf merkt sich Datum und Tags, bis etwas darin steht
    if let Some(draft) = state
        .journal_draft
        .as_mut()
        .filter(|d| d.key() == entry.key())
    {
        draft.day = fields.day;
        draft.info.categories = fields.categories.clone();
    }
    match journal::save(state, &entry, &fields) {
        Ok(saved) => {
            state.journal_dirty = false;
            state.journal_error = None;
            state.journal_loaded = (
                fields.summary.trim().to_string(),
                fields.description.trim_end().to_string(),
            );
            if saved != Saved::Unchanged {
                state.search_index = None;
            }
            load(app, state);
            reveal(app, state);
        }
        Err(message) => {
            state.journal_error = Some(message);
            if let Some(entry) = selected(state) {
                meta(app, state, &entry);
            }
        }
    }
}

/// Schreiben (true) oder Lesen; Lesen speichert und zeigt Markdown
pub fn set_write(app: &AppWindow, state: &mut State, write: bool) {
    if !write {
        flush(app, state);
        render(app, &app.get_journal_body());
    }
    let read_only = selected(state).is_none_or(|e| e.read_only);
    app.set_journal_write(write && !read_only);
}

pub fn add_tag(app: &AppWindow, state: &mut State, text: &str) {
    let new: Vec<String> = text.split(',').map(str::to_string).collect();
    save_with(app, state, |f| f.categories.extend(new));
}

pub fn remove_tag(app: &AppWindow, state: &mut State, index: usize) {
    save_with(app, state, |f| {
        if index < f.categories.len() {
            f.categories.remove(index);
        }
    });
}

/// Datum aus "TT.MM.JJJJ" (auch "heute", "morgen") setzen
pub fn set_date(app: &AppWindow, state: &mut State, text: &str) {
    let today = state.now().date();
    let Some(day) = forms::parse_date(text, today) else {
        app.set_journal_date_error("Datum nicht erkannt (z. B. 06.11.2026)".into());
        return;
    };
    app.set_journal_date_error("".into());
    save_with(app, state, |f| f.day = day);
    // Den Monat des Eintrags zeigen, damit er in der Liste bleibt
    if !state.journal_all
        && (state.anchor.year(), state.anchor.month()) != (day.year(), day.month())
    {
        state.anchor = day;
        state.mini_month = day.with_day(1).unwrap_or(day);
        crate::load_view(app, state, true);
    }
}

pub fn set_calendar(app: &AppWindow, state: &mut State, target: i64) {
    flush(app, state);
    let Some(entry) = selected(state) else {
        return;
    };
    match journal::move_to(state, &entry, target) {
        Ok(key) => {
            state.journal_selected = Some(key);
            state.journal_error = None;
            state.search_index = None;
        }
        Err(message) => state.journal_error = Some(message),
    }
    load(app, state);
}

/// Löschen: leere Entwürfe sofort, sonst mit Rückfrage
pub fn ask_delete(app: &AppWindow, state: &mut State) {
    flush(app, state);
    let Some(entry) = selected(state) else {
        return;
    };
    if entry.read_only {
        return;
    }
    if entry.draft {
        delete(app, state, &entry);
        return;
    }
    let name = entry.info.summary.trim();
    app.set_confirm_title("Journaleintrag löschen?".into());
    app.set_confirm_message(
        if name.is_empty() {
            format!(
                "Der Eintrag vom {} wird gelöscht.",
                forms::format_date(entry.day)
            )
        } else {
            format!("„{name}“ wird gelöscht.")
        }
        .into(),
    );
    state.deleting_journal = Some(entry);
    app.set_confirm_open(true);
}

/// Löschen und den nächsten Eintrag wählen (oder den vorherigen, wenn es
/// der letzte war)
pub fn delete(app: &AppWindow, state: &mut State, entry: &Entry) {
    let index = state.journals.iter().position(|e| e.key() == entry.key());
    if let Err(message) = journal::delete(state, entry) {
        state.journal_error = Some(message);
        load(app, state);
        return;
    }
    state.journal_dirty = false;
    state.search_index = None;
    state.journal_selected = None;
    load(app, state);
    let next = index.and_then(|i| {
        state
            .journals
            .get(i)
            .or_else(|| i.checked_sub(1).and_then(|p| state.journals.get(p)))
            .map(Entry::key)
    });
    if next.is_some() {
        select(app, state, next, false);
    }
    app.invoke_journal_focus_list();
}

/// Eintrag öffnen, z. B. aus der Befehlspalette oder dem Monat
pub fn show(app: &AppWindow, state: &mut State, key: &str, day: NaiveDate) {
    flush(app, state);
    state.view = View::Journal;
    if !state.journal_all {
        state.anchor = day;
        state.mini_month = day.with_day(1).unwrap_or(day);
    }
    crate::load_view(app, state, false);
    select(app, state, Some(key.to_string()), false);
}

/// Strg+B / Strg+I im Text
pub fn format(text: &str, anchor: i32, cursor: i32, marker: &str) -> JournalEdit {
    let (text, anchor, cursor) =
        journal::format(text, anchor.max(0) as usize, cursor.max(0) as usize, marker);
    JournalEdit {
        text: text.into(),
        anchor: anchor as i32,
        cursor: cursor as i32,
    }
}
