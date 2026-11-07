//! Aufgaben: Liste für die Aufgaben-Ansicht (Gruppen wie Calinos
//! taskGroups.ts, Teilaufgaben als Baum) und Bearbeiten (erledigen,
//! anlegen, ändern, löschen). Alles geht wie bei Terminen sofort in den
//! Cache und über die Warteschlange zum Server.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use hora_core::Tz;
use hora_core::model::{CalendarObject, Kind};
use hora_core::task::{self, Priority, TaskFields, TaskInfo, TaskOccurrence, due_instant};
use hora_core::time::{DateTimeValue, local_to_utc};

use crate::forms::{format_date, format_time, parse_date, parse_time, short_day};
use crate::{State, calendar_color, editing};

/// Eine Aufgabe in der Liste (bei Serien: ein Vorkommen)
#[derive(Debug, Clone, PartialEq)]
pub struct TaskItem {
    pub calendar_id: i64,
    pub href: String,
    pub uid: String,
    /// Die Aufgabe bzw. die Serie
    pub info: TaskInfo,
    /// Vorkommen einer Serie (offen oder erledigt)
    pub occurrence: Option<TaskOccurrence>,
    pub due: Option<DateTimeValue>,
    pub start: Option<DateTimeValue>,
    pub done: bool,
    pub completed_at: Option<DateTime<Utc>>,
    pub recurring: bool,
    /// Serie: schon erledigte Vorkommen
    pub completed_count: usize,
    pub color: u32,
    pub calendar_name: String,
    pub read_only: bool,
}

/// Filter der Ansicht
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Active,
    Done,
}

impl Filter {
    pub fn from_index(i: i32) -> Filter {
        match i {
            1 => Filter::Active,
            2 => Filter::Done,
            _ => Filter::All,
        }
    }

    pub fn index(self) -> i32 {
        match self {
            Filter::All => 0,
            Filter::Active => 1,
            Filter::Done => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Overdue,
    Today,
    Week,
    Later,
    NoDate,
    Done,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Overdue => "ÜBERFÄLLIG",
            Group::Today => "HEUTE",
            Group::Week => "DIESE WOCHE",
            Group::Later => "SPÄTER",
            Group::NoDate => "KEIN FÄLLIGKEITSDATUM",
            Group::Done => "ERLEDIGT",
        }
    }
}

fn hash_of(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Aufgaben aller sichtbaren Kalender aus dem Cache
pub fn collect(state: &mut State) -> Vec<TaskItem> {
    let home = state.home;
    let resources = state.store.tasks().unwrap_or_default();
    let calendars: HashMap<i64, (u32, String, bool)> = state
        .calendars
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id, (calendar_color(c, i), c.name.clone(), c.read_only)))
        .collect();
    let mut items = Vec::new();
    for resource in resources {
        let (color, calendar_name, read_only) = calendars
            .get(&resource.calendar_id)
            .cloned()
            .unwrap_or((crate::CALENDAR_COLORS[0], String::new(), false));
        let hash = hash_of(&resource.ics);
        let entry = state
            .parsed_tasks
            .entry((resource.calendar_id, resource.href.clone()))
            .or_insert((0, Vec::new()));
        if entry.0 != hash {
            *entry = (
                hash,
                CalendarObject::parse(&resource.ics)
                    .map(|o| o.tasks())
                    .unwrap_or_default(),
            );
        }
        let tasks = entry.1.clone();
        let base = |info: &TaskInfo| TaskItem {
            calendar_id: resource.calendar_id,
            href: resource.href.clone(),
            uid: info.uid.clone(),
            info: info.clone(),
            occurrence: None,
            due: info.due,
            start: info.start,
            done: info.completed,
            completed_at: info.completed_at,
            recurring: false,
            completed_count: 0,
            color,
            calendar_name: calendar_name.clone(),
            read_only,
        };
        let has_master = |uid: &str| {
            tasks
                .iter()
                .any(|t| t.uid == uid && t.recurrence_id.is_none())
        };
        for info in &tasks {
            match (info.recurrence_id, info.is_recurring()) {
                // Erledigtes Vorkommen einer Serie (auch ohne Serie in dieser Ressource)
                (Some(rid), _) => {
                    if info.completed || !has_master(&info.uid) {
                        items.push(TaskItem {
                            occurrence: Some(TaskOccurrence {
                                recurrence_id: rid,
                                start: info.start,
                                due: info.due,
                            }),
                            recurring: has_master(&info.uid),
                            ..base(info)
                        });
                    }
                }
                (None, true) => {
                    let family: Vec<TaskInfo> = tasks
                        .iter()
                        .filter(|t| t.uid == info.uid)
                        .cloned()
                        .collect();
                    let state = task::series_state(&family, home);
                    match state {
                        Some(s) if s.current.is_some() => {
                            let current = s.current.unwrap();
                            items.push(TaskItem {
                                due: current.due,
                                start: current.start,
                                occurrence: Some(current),
                                done: false,
                                completed_at: None,
                                recurring: true,
                                completed_count: s.completed,
                                ..base(info)
                            });
                        }
                        // Keine Vorkommen mehr: die ganze Serie ist erledigt
                        Some(s) => items.push(TaskItem {
                            done: true,
                            recurring: true,
                            completed_count: s.completed,
                            ..base(info)
                        }),
                        None => items.push(base(info)),
                    }
                }
                (None, false) => items.push(base(info)),
            }
        }
    }
    items.retain(|i| !i.info.cancelled);
    items
}

/// Gruppe nach Fälligkeit; heute fällig mit vergangener Uhrzeit ist überfällig
pub fn group_of(due: Option<&DateTimeValue>, now: DateTime<Utc>, home: Tz) -> Group {
    let Some(due) = due else {
        return Group::NoDate;
    };
    let today = now.with_timezone(&home).date_naive();
    let day = due_day(due, home);
    let days = (day - today).num_days();
    if days < 0 || (days == 0 && !due.is_date() && due.to_utc(home) < now) {
        Group::Overdue
    } else if days == 0 {
        Group::Today
    } else if days <= 6 {
        Group::Week
    } else {
        Group::Later
    }
}

fn due_day(due: &DateTimeValue, home: Tz) -> NaiveDate {
    match due {
        DateTimeValue::Date(d) => *d,
        other => other.to_utc(home).with_timezone(&home).date_naive(),
    }
}

/// "Heute, 18:00", "Morgen", "Fr., 13. Nov.", "04.01.2027"
pub fn due_label(due: &DateTimeValue, now: DateTime<Utc>, home: Tz) -> String {
    let today = now.with_timezone(&home).date_naive();
    let day = due_day(due, home);
    let name = match (day - today).num_days() {
        -1 => "Gestern".to_string(),
        0 => "Heute".to_string(),
        1 => "Morgen".to_string(),
        _ if day.year() == today.year() => short_day(day),
        _ => format_date(day),
    };
    if due.is_date() {
        name
    } else {
        let local = due.to_utc(home).with_timezone(&home).naive_local();
        format!("{name}, {}", format_time(local.time()))
    }
}

/// Überfällig (und noch offen)?
pub fn overdue(item: &TaskItem, now: DateTime<Utc>, home: Tz) -> bool {
    !item.done && group_of(item.due.as_ref(), now, home) == Group::Overdue
}

/// Eine Zeile der Liste
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub header: Option<(Group, usize)>,
    /// Index in der Aufgabenliste
    pub item: usize,
    pub depth: usize,
    pub has_children: bool,
    pub collapsed: bool,
    /// "1/3": erledigte von allen direkten Teilaufgaben
    pub children: String,
    /// Übergeordnete Aufgabe, wenn sie nicht in der Liste steht
    pub context: String,
}

fn priority_rank(p: u8) -> u8 {
    match Priority::from_value(p) {
        Priority::High => 0,
        Priority::Medium => 1,
        Priority::Low => 2,
        Priority::None => 3,
    }
}

/// Zeilen der Ansicht: Gruppen, darin Aufgaben mit ihren Teilaufgaben
pub fn rows(
    items: &[TaskItem],
    filter: Filter,
    collapsed: &HashSet<String>,
    now: DateTime<Utc>,
    home: Tz,
) -> Vec<Row> {
    let visible: Vec<usize> = (0..items.len())
        .filter(|i| match filter {
            Filter::All => true,
            Filter::Active => !items[*i].done,
            Filter::Done => items[*i].done,
        })
        .collect();
    let shown: HashSet<usize> = visible.iter().copied().collect();
    // Aufgabe je UID (bei Serien das offene Vorkommen bzw. die Serie)
    let mut by_uid: HashMap<&str, usize> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        let candidate = !(item.recurring && item.done && item.occurrence.is_some());
        if candidate {
            by_uid.entry(item.uid.as_str()).or_insert(i);
        }
    }
    let parent_of = |i: usize| -> Option<usize> {
        items[i]
            .info
            .parent
            .as_deref()
            .and_then(|p| by_uid.get(p).copied())
            .filter(|p| *p != i)
    };
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut roots = Vec::new();
    for &i in &visible {
        match parent_of(i).filter(|p| shown.contains(p)) {
            Some(p) => children.entry(p).or_default().push(i),
            None => roots.push(i),
        }
    }
    let sort_key = |i: &usize| {
        let item = &items[*i];
        (
            item.due
                .map_or(i64::MAX, |d| due_instant(&d, home).timestamp()),
            priority_rank(item.info.priority),
            item.info.sort_order.unwrap_or(i64::MAX),
            item.info.summary.to_lowercase(),
        )
    };
    for list in children.values_mut() {
        list.sort_by_key(|i| {
            (
                items[*i].done,
                items[*i].info.sort_order.unwrap_or(i64::MAX),
                sort_key(i),
            )
        });
    }
    let group = |i: usize| {
        if items[i].done || filter == Filter::Done {
            Group::Done
        } else {
            group_of(items[i].due.as_ref(), now, home)
        }
    };
    let mut groups: Vec<(Group, Vec<usize>)> = Vec::new();
    for &root in &roots {
        let g = group(root);
        match groups.iter_mut().find(|(k, _)| *k == g) {
            Some((_, list)) => list.push(root),
            None => groups.push((g, vec![root])),
        }
    }
    groups.sort_by_key(|(g, _)| *g);
    let mut out = Vec::new();
    for (g, mut list) in groups {
        if g == Group::Done {
            list.sort_by_key(|i| std::cmp::Reverse(items[*i].completed_at));
        } else {
            list.sort_by_key(sort_key);
        }
        let count = list.len();
        out.push(Row {
            header: Some((g, count)),
            item: usize::MAX,
            depth: 0,
            has_children: false,
            collapsed: false,
            children: String::new(),
            context: String::new(),
        });
        let mut seen = HashSet::new();
        for root in list {
            emit(
                root, 0, items, &children, collapsed, &parent_of, &mut seen, &mut out,
            );
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn emit(
    i: usize,
    depth: usize,
    items: &[TaskItem],
    children: &HashMap<usize, Vec<usize>>,
    collapsed: &HashSet<String>,
    parent_of: &dyn Fn(usize) -> Option<usize>,
    seen: &mut HashSet<usize>,
    out: &mut Vec<Row>,
) {
    if !seen.insert(i) {
        return;
    }
    let kids = children.get(&i).cloned().unwrap_or_default();
    let all_kids: Vec<usize> = (0..items.len())
        .filter(|k| parent_of(*k) == Some(i) && !items[*k].recurring)
        .collect();
    let is_collapsed = collapsed.contains(&items[i].uid);
    let context = match (depth, parent_of(i)) {
        (0, Some(p)) => format!("Unteraufgabe von „{}“", items[p].info.summary),
        _ => String::new(),
    };
    out.push(Row {
        header: None,
        item: i,
        depth,
        has_children: !kids.is_empty(),
        collapsed: is_collapsed,
        children: if all_kids.is_empty() {
            String::new()
        } else {
            format!(
                "{}/{}",
                all_kids.iter().filter(|k| items[**k].done).count(),
                all_kids.len()
            )
        },
        context,
    });
    if !is_collapsed {
        for kid in kids {
            emit(
                kid,
                depth + 1,
                items,
                children,
                collapsed,
                parent_of,
                seen,
                out,
            );
        }
    }
}

/// Alle Teilaufgaben (rekursiv) der Aufgabe `i`
pub fn descendants(items: &[TaskItem], i: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut queue = vec![items[i].uid.clone()];
    let mut seen: HashSet<String> = HashSet::from([items[i].uid.clone()]);
    while let Some(uid) = queue.pop() {
        for (k, item) in items.iter().enumerate() {
            if item.info.parent.as_deref() == Some(uid.as_str()) && seen.insert(item.uid.clone()) {
                out.push(k);
                queue.push(item.uid.clone());
            }
        }
    }
    out
}

fn load(state: &State, calendar_id: i64, href: &str) -> Result<CalendarObject, String> {
    let resource = state
        .store
        .resource(calendar_id, href)
        .map_err(|e| e.to_string())?
        .ok_or("Die Aufgabe ist nicht mehr vorhanden.")?;
    CalendarObject::parse(&resource.ics).map_err(|e| e.to_string())
}

/// Erledigt umschalten. Eine erledigte übergeordnete Aufgabe nimmt ihre
/// offenen Teilaufgaben mit (wie bei Calino).
pub fn toggle(state: &mut State, items: &[TaskItem], index: usize) -> Result<(), String> {
    let item = &items[index];
    if item.read_only {
        return Err("Dieser Kalender ist schreibgeschützt.".into());
    }
    let done = !item.done;
    let now = Utc::now();
    let home = state.home;
    let mut object = load(state, item.calendar_id, &item.href)?;
    match (&item.occurrence, item.recurring) {
        (Some(occurrence), true) => {
            task::complete_occurrence(&mut object, &item.uid, occurrence, done, home, now)
        }
        (Some(occurrence), false) => {
            // Ausnahme ohne Serie in dieser Ressource: nur ihren Stand setzen
            task::complete_occurrence(&mut object, &item.uid, occurrence, done, home, now)
        }
        (None, _) => task::set_completed(&mut object, &item.uid, done, now),
    }
    .map_err(|e| e.to_string())?;
    editing::store_object(state, item.calendar_id, &item.href, &object)?;
    if done {
        for k in descendants(items, index) {
            let kid = &items[k];
            if kid.done || kid.recurring || kid.read_only {
                continue;
            }
            let mut object = load(state, kid.calendar_id, &kid.href)?;
            task::set_completed(&mut object, &kid.uid, true, now).map_err(|e| e.to_string())?;
            editing::store_object(state, kid.calendar_id, &kid.href, &object)?;
        }
    }
    Ok(())
}

/// Aufgabe samt Teilaufgaben löschen; liefert die Anzahl
pub fn delete(state: &mut State, items: &[TaskItem], index: usize) -> Result<usize, String> {
    let item = &items[index];
    if item.read_only {
        return Err("Dieser Kalender ist schreibgeschützt.".into());
    }
    let mut targets = vec![index];
    targets.extend(descendants(items, index));
    let mut done = HashSet::new();
    for k in targets {
        let t = &items[k];
        if t.read_only || !done.insert((t.calendar_id, t.href.clone())) {
            continue;
        }
        state
            .store
            .local_delete(t.calendar_id, &t.href)
            .map_err(|e| e.to_string())?;
    }
    state.send(crate::sync::Command::Push);
    Ok(done.len())
}

/// Kalender, in denen Aufgaben angelegt werden können
pub fn task_calendars(state: &State) -> Vec<i64> {
    state
        .calendars
        .iter()
        .filter(|c| !c.read_only && c.supports(Kind::Todo))
        .map(|c| c.id)
        .collect()
}

/// Kalender für neue Aufgaben: zuletzt benutzter, sonst der erste passende
pub fn default_calendar(state: &State) -> Option<i64> {
    let ids = task_calendars(state);
    state
        .store
        .setting("default_task_calendar")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|id| ids.contains(id))
        .or_else(|| ids.first().copied())
}

/// Eingaben des Aufgaben-Dialogs
#[derive(Debug, Clone, Default)]
pub struct TaskInput {
    pub title: String,
    /// 0 kein Datum, 1 nur Datum, 2 Datum und Uhrzeit
    pub due_mode: i32,
    pub due_date: String,
    pub due_time: String,
    pub start_date: String,
    /// 0 keine, 1 hoch, 2 mittel, 3 niedrig
    pub priority: i32,
    pub calendar: i64,
    pub parent: Option<String>,
    pub description: String,
}

fn priority_value(ui: i32) -> u8 {
    match ui {
        1 => Priority::High.value(),
        2 => Priority::Medium.value(),
        3 => Priority::Low.value(),
        _ => 0,
    }
}

pub fn priority_index(value: u8) -> i32 {
    match Priority::from_value(value) {
        Priority::None => 0,
        Priority::High => 1,
        Priority::Medium => 2,
        Priority::Low => 3,
    }
}

impl TaskInput {
    /// Felder prüfen; `existing` ist die bisherige Fälligkeit (behält ihre Zone)
    fn fields(
        &self,
        today: NaiveDate,
        home: Tz,
        existing: Option<&DateTimeValue>,
    ) -> Result<TaskFields, String> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err("Bitte einen Titel eingeben.".into());
        }
        let due = match self.due_mode {
            0 => None,
            mode => {
                let day = parse_date(&self.due_date, today)
                    .ok_or("Fällig: Datum nicht erkannt (z. B. 13.11.2026)")?;
                if mode == 1 {
                    Some(DateTimeValue::Date(day))
                } else {
                    let time = parse_time(&self.due_time)
                        .ok_or("Fällig: Uhrzeit nicht erkannt (z. B. 18:00)")?;
                    let local = day.and_time(time);
                    Some(match existing.filter(|v| !v.is_date()) {
                        Some(v) => v.moved_to(local_to_utc(local, v.zone(home)), home),
                        None => DateTimeValue::Zoned(local, home),
                    })
                }
            }
        };
        let start = match (self.start_date.trim(), &due) {
            ("", _) | (_, None) => None,
            (text, Some(due)) => {
                let day = parse_date(text, today)
                    .ok_or("Beginn: Datum nicht erkannt (z. B. 09.11.2026)")?;
                if day > due_day(due, home) {
                    return Err("Der Beginn liegt nach der Fälligkeit.".into());
                }
                Some(DateTimeValue::Date(day))
            }
        };
        Ok(TaskFields {
            summary: title.to_string(),
            description: self.description.trim_end().to_string(),
            due,
            start,
            priority: priority_value(self.priority),
            parent: self.parent.clone(),
        })
    }
}

/// Dialog speichern: neue Aufgabe oder `editing` ändern
pub fn save(
    state: &mut State,
    items: &[TaskItem],
    editing: Option<usize>,
    input: &TaskInput,
) -> Result<(), String> {
    let today = state.now().date();
    let home = state.home;
    let now = Utc::now();
    if let Some(parent) = &input.parent {
        let parent_item = items
            .iter()
            .find(|i| &i.uid == parent && !(i.recurring && i.occurrence.is_some() && i.done));
        match parent_item {
            Some(p) if p.calendar_id != input.calendar => {
                return Err("Die übergeordnete Aufgabe liegt in einem anderen Kalender.".into());
            }
            Some(p) if p.recurring => {
                return Err("Wiederkehrende Aufgaben können keine Unteraufgaben haben.".into());
            }
            _ => {}
        }
        if let Some(e) = editing
            && (parent == &items[e].uid
                || descendants(items, e)
                    .iter()
                    .any(|d| &items[*d].uid == parent))
        {
            return Err("Eine Aufgabe kann nicht in ihrer eigenen Unteraufgabe liegen.".into());
        }
    }
    match editing {
        None => {
            let fields = input.fields(today, home, None)?;
            let uid = editing::new_uid();
            let href = editing::new_href(state, input.calendar, &uid)?;
            let object = task::new_task(&uid, &fields, home, now);
            let _ = state
                .store
                .set_setting("default_task_calendar", &input.calendar.to_string());
            editing::store_object(state, input.calendar, &href, &object)
        }
        Some(e) => {
            let item = &items[e];
            if item.read_only {
                return Err("Dieser Kalender ist schreibgeschützt.".into());
            }
            let mut fields = input.fields(today, home, item.info.due.as_ref())?;
            if item.recurring {
                // Fälligkeit gehört der Serie und bleibt
                fields.due = item.info.due;
                fields.start = item.info.start;
            }
            let mut object = load(state, item.calendar_id, &item.href)?;
            task::update_task(&mut object, &item.uid, &fields, home, now)
                .map_err(|e| e.to_string())?;
            editing::store_object(state, item.calendar_id, &item.href, &object)
        }
    }
}

/// Felder des Dialogs für eine bestehende Aufgabe: (Art, Datum, Uhrzeit, Beginn)
pub fn draft_due(item: &TaskItem, home: Tz) -> (i32, String, String, String) {
    let start = item
        .start
        .map(|s| format_date(due_day(&s, home)))
        .unwrap_or_default();
    match item.due {
        None => (0, String::new(), "18:00".into(), String::new()),
        Some(DateTimeValue::Date(d)) => (1, format_date(d), "18:00".into(), start),
        Some(due) => {
            let local = due.to_utc(home).with_timezone(&home).naive_local();
            (
                2,
                format_date(local.date()),
                format_time(local.time()),
                start,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDateTime, TimeZone};
    use chrono_tz::Europe::Berlin;

    fn at(s: &str) -> DateTime<Utc> {
        Berlin
            .from_local_datetime(&NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap())
            .unwrap()
            .with_timezone(&Utc)
    }

    fn zoned(s: &str) -> DateTimeValue {
        DateTimeValue::Zoned(
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap(),
            Berlin,
        )
    }

    fn day(y: i32, m: u32, d: u32) -> DateTimeValue {
        DateTimeValue::Date(NaiveDate::from_ymd_opt(y, m, d).unwrap())
    }

    fn item(uid: &str, due: Option<DateTimeValue>, parent: Option<&str>, done: bool) -> TaskItem {
        let info = TaskInfo {
            uid: uid.into(),
            summary: uid.into(),
            description: String::new(),
            location: String::new(),
            start: None,
            due,
            completed: done,
            completed_at: None,
            cancelled: false,
            priority: 0,
            parent: parent.map(str::to_string),
            rrule: None,
            exdates: Vec::new(),
            recurrence_id: None,
            sort_order: None,
            alarms: Vec::new(),
        };
        TaskItem {
            calendar_id: 1,
            href: format!("/{uid}.ics"),
            uid: uid.into(),
            due,
            start: None,
            done,
            completed_at: None,
            recurring: false,
            completed_count: 0,
            color: 0,
            calendar_name: "Privat".into(),
            read_only: false,
            occurrence: None,
            info,
        }
    }

    #[test]
    fn groups_follow_calino() {
        let now = at("2026-11-05 12:00");
        assert_eq!(
            group_of(Some(&day(2026, 11, 4)), now, Berlin),
            Group::Overdue
        );
        assert_eq!(
            group_of(Some(&zoned("2026-11-05 09:00")), now, Berlin),
            Group::Overdue,
            "heute, Uhrzeit vorbei"
        );
        assert_eq!(
            group_of(Some(&zoned("2026-11-05 18:00")), now, Berlin),
            Group::Today
        );
        assert_eq!(group_of(Some(&day(2026, 11, 5)), now, Berlin), Group::Today);
        assert_eq!(group_of(Some(&day(2026, 11, 11)), now, Berlin), Group::Week);
        assert_eq!(
            group_of(Some(&day(2026, 11, 12)), now, Berlin),
            Group::Later
        );
        assert_eq!(group_of(None, now, Berlin), Group::NoDate);
        assert_eq!(
            due_label(&zoned("2026-11-05 18:00"), now, Berlin),
            "Heute, 18:00"
        );
        assert_eq!(due_label(&day(2026, 11, 6), now, Berlin), "Morgen");
        assert_eq!(due_label(&day(2026, 11, 13), now, Berlin), "Fr. 13. Nov.");
        assert_eq!(due_label(&day(2027, 1, 4), now, Berlin), "04.01.2027");
    }

    #[test]
    fn rows_nest_subtasks_under_their_parent() {
        let now = at("2026-11-05 12:00");
        let items = vec![
            item("umzug", Some(day(2026, 11, 20)), None, false),
            item("kartons", Some(day(2026, 11, 4)), Some("umzug"), false),
            item("strom", None, Some("umzug"), true),
            item("muell", Some(day(2026, 11, 5)), None, false),
            item("alt", None, None, true),
        ];
        let rows = rows(&items, Filter::All, &HashSet::new(), now, Berlin);
        let shape: Vec<String> = rows
            .iter()
            .map(|r| match r.header {
                Some((g, n)) => format!("# {} {n}", g.title()),
                None => format!("{}{}", "  ".repeat(r.depth), items[r.item].uid),
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                "# HEUTE 1",
                "muell",
                "# SPÄTER 1",
                "umzug",
                "  kartons",
                "  strom",
                "# ERLEDIGT 1",
                "alt"
            ]
        );
        assert_eq!(rows[3].children, "1/2");
        // Aktiv: die erledigte Teilaufgabe fehlt, die überfällige bleibt beim Elternteil
        let active = super::rows(&items, Filter::Active, &HashSet::new(), now, Berlin);
        assert_eq!(active.iter().filter(|r| r.header.is_none()).count(), 3);
        // Eingeklappt: keine Teilaufgaben
        let folded = super::rows(
            &items,
            Filter::All,
            &HashSet::from(["umzug".to_string()]),
            now,
            Berlin,
        );
        assert!(
            folded
                .iter()
                .all(|r| r.header.is_some() || items[r.item].uid != "kartons")
        );
        // Erledigt-Filter: Teilaufgabe ohne sichtbares Elternteil nennt es
        let done = super::rows(&items, Filter::Done, &HashSet::new(), now, Berlin);
        let strom = done
            .iter()
            .find(|r| r.header.is_none() && items[r.item].uid == "strom")
            .unwrap();
        assert_eq!(strom.context, "Unteraufgabe von „umzug“");
        assert_eq!(descendants(&items, 0), vec![1, 2]);
    }

    #[test]
    fn dialog_fields_are_checked() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let input = TaskInput {
            title: "Bericht".into(),
            due_mode: 2,
            due_date: "13.11.".into(),
            due_time: "18".into(),
            start_date: "9.11.".into(),
            priority: 1,
            calendar: 1,
            ..TaskInput::default()
        };
        let fields = input.fields(today, Berlin, None).unwrap();
        assert_eq!(fields.due, Some(zoned("2026-11-13 18:00")));
        assert_eq!(fields.start, Some(day(2026, 11, 9)));
        assert_eq!(fields.priority, 1);
        let late_start = TaskInput {
            start_date: "20.11.".into(),
            ..input.clone()
        };
        assert!(late_start.fields(today, Berlin, None).is_err());
        let untitled = TaskInput {
            title: " ".into(),
            ..input
        };
        assert!(untitled.fields(today, Berlin, None).is_err());
    }
}
