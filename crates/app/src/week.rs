//! Layout-Logik der Wochenansicht: Einrasten beim Ziehen und Aufteilung
//! überlappender Termine auf Spalten.
//!
//! Übertragen aus Calino (`src/features/calendar/lib/dragSnap.ts`,
//! `eventLayout.ts`), Copyright (c) 2025 Ivan Malinovski, MIT-Lizenz.

/// Raster beim Ziehen, wie in Calino eine Viertelstunde.
pub const SNAP_MINUTES: i32 = 15;
pub const MINUTES_PER_DAY: i32 = 24 * 60;

/// Kürzeste Dauer, mit der ein Termin für die Spaltenaufteilung zählt. Die Karte
/// ist nie niedriger als etwa eine Viertelstunde, sonst würden sich sehr kurze
/// Termine optisch überdecken, ohne als Überschneidung zu gelten.
const MIN_LAYOUT_MINUTES: i32 = SNAP_MINUTES;

fn round_to_snap(minutes: f64) -> i32 {
    (minutes / f64::from(SNAP_MINUTES)).round() as i32 * SNAP_MINUTES
}

/// Neuer Beginn, wenn ein Termin um `delta_min` Minuten verschoben wird.
///
/// Wie in Calino wird absolut auf die Viertelstunde gerundet: ein importierter
/// Termin um 9:07 landet nach dem Verschieben auf einer glatten Viertelstunde.
/// Abweichend von Calino bleibt der Termin vollständig im Tag, weil die
/// Wochenansicht im Prototyp noch keine Termine über Mitternacht darstellt.
pub fn snap_move(start: i32, delta_min: f64, dur: i32) -> i32 {
    let latest = ((MINUTES_PER_DAY - dur.clamp(0, MINUTES_PER_DAY)) / SNAP_MINUTES) * SNAP_MINUTES;
    round_to_snap(f64::from(start) + delta_min).clamp(0, latest)
}

/// Neue Dauer, wenn das Ende eines Termins um `delta_min` Minuten gezogen wird.
/// Das Ende rastet auf die Viertelstunde ein; mindestens eine Viertelstunde,
/// höchstens bis Mitternacht.
pub fn snap_resize(start: i32, dur: i32, delta_min: f64) -> i32 {
    let end = round_to_snap(f64::from(start + dur) + delta_min);
    end.clamp(
        start + SNAP_MINUTES,
        MINUTES_PER_DAY.max(start + SNAP_MINUTES),
    ) - start
}

/// Position eines Termins innerhalb seiner Tagesspalte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    /// Spalte, beginnend bei 0
    pub lane: i32,
    /// Anzahl Spalten der Gruppe überlappender Termine
    pub lanes: i32,
    /// Wie viele Spalten der Termin nach rechts belegen darf
    pub span: i32,
}

/// Verteilt die Termine eines Tages auf Spalten. `spans` enthält Beginn und
/// Ende in Minuten seit Mitternacht; das Ergebnis hat dieselbe Reihenfolge.
///
/// Termine, die sich direkt oder über andere überschneiden, bilden eine
/// Gruppe mit gemeinsamer Spaltenzahl. Jeder Termin nimmt die erste frei
/// gewordene Spalte und dehnt sich nach rechts über Spalten aus, die in
/// seinem Zeitraum leer sind.
pub fn layout_day(spans: &[(i32, i32)]) -> Vec<Slot> {
    let spans: Vec<(i32, i32)> = spans
        .iter()
        .map(|&(s, e)| (s, e.max(s + MIN_LAYOUT_MINUTES)))
        .collect();
    let mut order: Vec<usize> = (0..spans.len()).collect();
    // Früher zuerst, bei gleichem Beginn der längere zuerst
    order.sort_by_key(|&i| (spans[i].0, -spans[i].1, i));

    let mut slots = vec![
        Slot {
            lane: 0,
            lanes: 1,
            span: 1
        };
        spans.len()
    ];
    let mut group: Vec<usize> = Vec::new();
    let mut lane_ends: Vec<i32> = Vec::new();
    let mut group_end = i32::MIN;

    for i in order {
        let (start, end) = spans[i];
        if !group.is_empty() && start >= group_end {
            finish_group(&group, lane_ends.len(), &spans, &mut slots);
            group.clear();
            lane_ends.clear();
        }
        let lane = match lane_ends.iter().position(|&lane_end| lane_end <= start) {
            Some(free) => {
                lane_ends[free] = end;
                free
            }
            None => {
                lane_ends.push(end);
                lane_ends.len() - 1
            }
        };
        slots[i].lane = lane as i32;
        group_end = if group.is_empty() {
            end
        } else {
            group_end.max(end)
        };
        group.push(i);
    }
    if !group.is_empty() {
        finish_group(&group, lane_ends.len(), &spans, &mut slots);
    }
    slots
}

/// Hinweis "+n" für Termine, die nicht mehr nebeneinander passen: Beginn der
/// ersten und Anzahl einer zusammenhängenden Gruppe solcher Termine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overflow {
    pub start: i32,
    pub count: i32,
}

/// Ergebnis von [`limit_lanes`]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limited {
    /// Termin fällt weg (kein Platz mehr nebeneinander)
    pub hidden: Vec<bool>,
    /// Termin teilt sich die Zeit mit weggefallenen: Er lässt rechts Platz
    /// für den Hinweis
    pub crowded: Vec<bool>,
    pub overflows: Vec<Overflow>,
}

/// Höchstens `max` Spalten nebeneinander (Ergebnis von [`layout_day`]):
/// Termine weiter rechts fallen weg, die übrigen teilen sich die Breite.
pub fn limit_lanes(slots: &mut [Slot], spans: &[(i32, i32)], max: i32) -> Limited {
    let max = max.max(1);
    let hidden: Vec<bool> = slots.iter().map(|s| s.lane >= max).collect();
    let crowded: Vec<bool> = slots
        .iter()
        .map(|s| s.lane < max && s.lanes > max)
        .collect();
    for slot in slots.iter_mut().filter(|s| s.lane < max) {
        slot.lanes = slot.lanes.min(max);
        slot.span = slot.span.min(max - slot.lane);
    }
    // Weggefallene, die sich (auch über andere weggefallene) überschneiden,
    // teilen sich einen Hinweis
    let mut gone: Vec<(i32, i32)> = spans
        .iter()
        .zip(&hidden)
        .filter(|(_, h)| **h)
        .map(|(&(s, e), _)| (s, e.max(s + MIN_LAYOUT_MINUTES)))
        .collect();
    gone.sort();
    let mut overflows: Vec<Overflow> = Vec::new();
    let mut end = i32::MIN;
    for (start, stop) in gone {
        match overflows.last_mut() {
            Some(last) if start < end => {
                last.count += 1;
                end = end.max(stop);
            }
            _ => {
                overflows.push(Overflow { start, count: 1 });
                end = stop;
            }
        }
    }
    Limited {
        hidden,
        crowded,
        overflows,
    }
}

fn overlaps(a: (i32, i32), b: (i32, i32)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

fn finish_group(group: &[usize], lanes: usize, spans: &[(i32, i32)], slots: &mut [Slot]) {
    let lanes = lanes as i32;
    for &i in group {
        let mut span = 1;
        while slots[i].lane + span < lanes {
            let lane = slots[i].lane + span;
            let blocked = group
                .iter()
                .any(|&j| slots[j].lane == lane && overlaps(spans[i], spans[j]));
            if blocked {
                break;
            }
            span += 1;
        }
        slots[i].lanes = lanes;
        slots[i].span = span;
    }
}

/// Zeilen für ganztägige Termine im Kopf der Woche. `spans` enthält ersten
/// Tag und Tag nach dem Ende (0 = Montag); Ergebnis ist die Zeile je Termin.
pub fn layout_all_day(spans: &[(i32, i32)]) -> Vec<i32> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&i| (spans[i].0, -spans[i].1, i));
    let mut row_ends: Vec<i32> = Vec::new();
    let mut rows = vec![0; spans.len()];
    for i in order {
        let (first, end) = spans[i];
        let row = match row_ends.iter().position(|&row_end| row_end <= first) {
            Some(free) => {
                row_ends[free] = end;
                free
            }
            None => {
                row_ends.push(end);
                row_ends.len() - 1
            }
        };
        rows[i] = row as i32;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i32 = 60;

    fn slot(lane: i32, lanes: i32, span: i32) -> Slot {
        Slot { lane, lanes, span }
    }

    #[test]
    fn move_snaps_to_quarter_hours() {
        // 9:00 um 20 Minuten nach unten -> 9:15 (20 liegt näher an 15 als an 30)
        assert_eq!(snap_move(9 * H, 20.0, 60), 9 * H + 15);
        // 23 Minuten -> 9:30
        assert_eq!(snap_move(9 * H, 23.0, 60), 9 * H + 30);
        // nach oben
        assert_eq!(snap_move(9 * H, -50.0, 60), 8 * H + 15);
    }

    #[test]
    fn move_snaps_off_grid_start_to_grid() {
        // Importierter Termin um 9:07 landet ohne Verschiebung auf 9:00
        assert_eq!(snap_move(9 * H + 7, 0.0, 30), 9 * H);
        assert_eq!(snap_move(9 * H + 8, 0.0, 30), 9 * H + 15);
    }

    #[test]
    fn move_stays_inside_the_day() {
        assert_eq!(snap_move(30, -120.0, 60), 0);
        // 90-Minuten-Termin endet spätestens um 24:00 -> Beginn höchstens 22:30
        assert_eq!(snap_move(22 * H, 300.0, 90), 22 * H + 30);
        // krumme Dauer: spätester Beginn auf dem Raster, damit das Ende passt
        assert_eq!(snap_move(22 * H, 300.0, 50), 23 * H);
    }

    #[test]
    fn resize_snaps_end_and_keeps_minimum() {
        // 9:00–10:00, Ende um 40 Minuten nach unten -> 10:45
        assert_eq!(snap_resize(9 * H, 60, 40.0), 105);
        // nach oben über den Beginn hinaus -> eine Viertelstunde bleibt
        assert_eq!(snap_resize(9 * H, 60, -200.0), 15);
        // nicht über Mitternacht
        assert_eq!(snap_resize(23 * H, 30, 120.0), 60);
    }

    #[test]
    fn at_most_two_side_by_side() {
        // Fünf Termine: 9–10:30, 9–10, 9:30–11, 9:45–10:15, 10–11:30
        let spans = [(540, 630), (540, 600), (570, 660), (585, 615), (600, 690)];
        let mut slots = layout_day(&spans);
        assert_eq!(slots[0].lanes, 4);
        let Limited {
            hidden,
            crowded,
            overflows,
        } = limit_lanes(&mut slots, &spans, 2);
        // Sichtbar: 9–10:30 links, 9–10 und danach 10–11:30 rechts
        assert_eq!(hidden, [false, false, true, true, false]);
        assert_eq!(crowded, [true, true, false, false, true]);
        assert_eq!(slots[0], slot(0, 2, 1));
        assert_eq!(slots[1], slot(1, 2, 1));
        assert_eq!(slots[4], slot(1, 2, 1));
        // Die zwei weggefallenen überschneiden sich: ein Hinweis ab 9:30
        assert_eq!(
            overflows,
            [Overflow {
                start: 570,
                count: 2
            }]
        );

        // Ohne Überlauf ändert sich nichts
        let spans = [(540, 600), (570, 630)];
        let mut slots = layout_day(&spans);
        let before = slots.clone();
        let limited = limit_lanes(&mut slots, &spans, 2);
        assert_eq!(slots, before);
        assert!(limited.hidden.iter().chain(&limited.crowded).all(|h| !h));
        assert!(limited.overflows.is_empty());

        // Getrennte Gruppen weggefallener Termine bekommen eigene Hinweise
        let spans = [
            (540, 600),
            (540, 600),
            (540, 600),
            (840, 900),
            (840, 900),
            (840, 900),
        ];
        let mut slots = layout_day(&spans);
        assert_eq!(
            limit_lanes(&mut slots, &spans, 2).overflows,
            [
                Overflow {
                    start: 540,
                    count: 1
                },
                Overflow {
                    start: 840,
                    count: 1
                }
            ]
        );
        // Ein ausgedehnter Termin bleibt in den erlaubten Spalten
        let spans = [(540, 660), (540, 570), (540, 570), (600, 630)];
        let mut slots = layout_day(&spans);
        let hidden = limit_lanes(&mut slots, &spans, 2).hidden;
        assert!(
            slots
                .iter()
                .zip(&hidden)
                .filter(|(_, h)| !**h)
                .all(|(s, _)| s.lane + s.span <= 2)
        );
    }

    #[test]
    fn separate_events_use_full_width() {
        let slots = layout_day(&[(9 * H, 10 * H), (10 * H, 11 * H), (13 * H, 14 * H)]);
        assert_eq!(slots, vec![slot(0, 1, 1); 3]);
    }

    #[test]
    fn two_overlapping_events_share_two_lanes() {
        let slots = layout_day(&[(9 * H, 10 * H), (9 * H + 30, 11 * H)]);
        assert_eq!(slots, vec![slot(0, 2, 1), slot(1, 2, 1)]);
    }

    #[test]
    fn chain_reuses_freed_lane() {
        // A 9–10, B 9:30–11, C 10–11:30: C übernimmt A's Spalte
        let slots = layout_day(&[(9 * H, 10 * H), (9 * H + 30, 11 * H), (10 * H, 11 * H + 30)]);
        assert_eq!(slots, vec![slot(0, 2, 1), slot(1, 2, 1), slot(0, 2, 1)]);
    }

    #[test]
    fn event_expands_into_free_lanes() {
        // A 9–11, B 9–10, C 9–10, D 10–11: D nimmt B's Spalte und dehnt sich
        // in C's Spalte aus, die ab 10 Uhr frei ist.
        let slots = layout_day(&[
            (9 * H, 11 * H),
            (9 * H, 10 * H),
            (9 * H, 10 * H),
            (10 * H, 11 * H),
        ]);
        assert_eq!(
            slots,
            vec![slot(0, 3, 1), slot(1, 3, 1), slot(2, 3, 1), slot(1, 3, 2)]
        );
    }

    #[test]
    fn order_of_input_does_not_matter() {
        let a = layout_day(&[(10 * H, 11 * H), (9 * H, 10 * H + 30)]);
        assert_eq!(a, vec![slot(1, 2, 1), slot(0, 2, 1)]);
    }

    #[test]
    fn very_short_events_still_count_as_overlapping() {
        // Zwei 5-Minuten-Termine direkt hintereinander würden sich als Karten
        // überdecken, deshalb gelten sie als überlappend.
        let slots = layout_day(&[(9 * H, 9 * H + 5), (9 * H + 5, 9 * H + 10)]);
        assert_eq!(slots, vec![slot(0, 2, 1), slot(1, 2, 1)]);
    }

    #[test]
    fn all_day_rows() {
        // Mo–Mi, Di, Do–Fr, Mi
        let rows = layout_all_day(&[(0, 3), (1, 2), (3, 5), (2, 3)]);
        assert_eq!(rows, vec![0, 1, 0, 1]);
    }
}
