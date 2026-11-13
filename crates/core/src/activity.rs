//! App-Zeiten: wie lange eine Anwendung im Vordergrund aktiv benutzt wird.
//!
//! Die Plattform meldet Beobachtungen: was gerade vorn ist (schon einer
//! Aktivität zugeordnet, sonst `None`) und wie lange die letzte Eingabe mit
//! Maus, Tastatur oder Stift her ist. [`Tracker`] macht daraus Zeitabschnitte:
//!
//! - Gezählt wird, solange dieselbe Aktivität vorn ist und die letzte Eingabe
//!   weniger als ihre Pausengrenze zurückliegt.
//! - Ein Abschnitt beginnt beim Wechsel in die App, nach einer Pause mit der
//!   letzten bemerkten Eingabe.
//! - Wechselt der Vordergrund, endet er im Moment des Wechsels. War vorher
//!   schon die Pausengrenze erreicht, endet er bei der letzten Eingabe.
//! - Bleibt die Eingabe bis zur Pausengrenze aus, endet er bei der letzten
//!   Eingabe: Die Wartezeit zählt nicht.
//!
//! Vordergrundwechsel meldet die Plattform sofort; dazwischen meldet sie sich
//! spätestens nach [`Tracker::next_check`] wieder.

use chrono::{DateTime, TimeDelta, Utc};

/// Ist die App vorn, aber die Pausengrenze erreicht: so oft nachsehen, ob
/// wieder gearbeitet wird. Windows meldet nur die jüngste Eingabe; der
/// Abschnitt beginnt mit ihr, nach einer Pause fehlen also höchstens so viele
/// Sekunden (gezählt wird eher zu wenig als zu viel).
pub const RESUME_POLL: TimeDelta = TimeDelta::seconds(2);

/// Kürzester Abstand zwischen zwei Nachfragen
const MIN_CHECK: TimeDelta = TimeDelta::seconds(1);

/// Was gerade im Vordergrund ist: die Aktivität und ihre Pausengrenze
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Front {
    pub key: String,
    pub idle_limit: TimeDelta,
}

impl Front {
    pub fn new(key: impl Into<String>, idle_limit: TimeDelta) -> Self {
        Front {
            key: key.into(),
            idle_limit,
        }
    }
}

/// Gezählte Zeit einer Aktivität am Stück
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub key: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl Segment {
    pub fn duration(&self) -> TimeDelta {
        self.end - self.start
    }
}

#[derive(Debug)]
struct Current {
    front: Front,
    /// Seit wann diese Aktivität vorn ist
    since: DateTime<Utc>,
    /// Beginn des laufenden Abschnitts; `None` während einer Pause
    open: Option<DateTime<Utc>>,
}

impl Current {
    /// Ende des laufenden Abschnitts, wenn jetzt `at` ist und die letzte
    /// Eingabe `last_input` war
    fn end(&self, at: DateTime<Utc>, last_input: DateTime<Utc>) -> DateTime<Utc> {
        if at - last_input < self.front.idle_limit {
            at
        } else {
            last_input
        }
    }

    fn close(&mut self, at: DateTime<Utc>, last_input: DateTime<Utc>) -> Option<Segment> {
        let start = self.open.take()?;
        let end = self.end(at, last_input);
        // Uhr zurückgestellt oder keine Eingabe seit Beginn: nichts zu zählen
        (end > start).then(|| Segment {
            key: self.front.key.clone(),
            start,
            end,
        })
    }
}

/// Zählt die aktive Zeit im Vordergrund (Regeln im Modultext)
#[derive(Debug, Default)]
pub struct Tracker {
    current: Option<Current>,
    /// Letzte Beobachtung: Zeitpunkt und letzte Eingabe
    seen: Option<(DateTime<Utc>, DateTime<Utc>)>,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Beobachtung verarbeiten: Zum Zeitpunkt `at` ist `front` vorn (oder
    /// nichts Gezähltes, auch bei gesperrtem Bildschirm), die letzte Eingabe
    /// ist `idle` her. Liefert den Abschnitt, der dabei endet.
    pub fn observe(
        &mut self,
        at: DateTime<Utc>,
        front: Option<Front>,
        idle: TimeDelta,
    ) -> Option<Segment> {
        let last_input = at - idle.max(TimeDelta::zero());
        self.seen = Some((at, last_input));
        let mut closed = None;
        if self.current.as_ref().map(|c| &c.front) != front.as_ref() {
            closed = self
                .current
                .take()
                .and_then(|mut c| c.close(at, last_input));
            self.current = front.map(|front| Current {
                front,
                since: at,
                open: None,
            });
        }
        if let Some(current) = &mut self.current {
            if at - last_input < current.front.idle_limit {
                current.open.get_or_insert(current.since.max(last_input));
            } else if let Some(segment) = current.close(at, last_input) {
                closed = Some(segment);
            }
        }
        closed
    }

    /// Zählung beenden (Hora wird beendet); liefert den letzten Abschnitt
    pub fn finish(&mut self, at: DateTime<Utc>) -> Option<Segment> {
        let (_, last_input) = self.seen?;
        let segment = self.current.take()?.close(at, last_input);
        self.seen = None;
        segment
    }

    /// Der laufende Abschnitt bis zur letzten Beobachtung, zum regelmäßigen
    /// Speichern (damit nach einem Absturz wenig fehlt)
    pub fn running(&self) -> Option<Segment> {
        let current = self.current.as_ref()?;
        let start = current.open?;
        let (at, last_input) = self.seen?;
        let end = current.end(at, last_input);
        (end > start).then(|| Segment {
            key: current.front.key.clone(),
            start,
            end,
        })
    }

    /// Wann spätestens die nächste Beobachtung nötig ist. `None`: erst beim
    /// nächsten Vordergrundwechsel.
    pub fn next_check(&self) -> Option<TimeDelta> {
        let current = self.current.as_ref()?;
        let (at, last_input) = self.seen?;
        Some(if current.open.is_some() {
            // Genau dann, wenn die Pausengrenze ohne neue Eingabe erreicht wäre
            (last_input + current.front.idle_limit - at).max(MIN_CHECK)
        } else {
            RESUME_POLL
        })
    }
}

/// Liegt zwischen zwei Abschnitten weniger als das, gehören sie zu einer
/// Sitzung (kurz etwas nachsehen, Kaffee holen).
pub const SESSION_GAP: TimeDelta = TimeDelta::minutes(15);

/// Gezählter Zeitraum einer Aktivität: Beginn und Ende
pub type Span = (DateTime<Utc>, DateTime<Utc>);

/// Abschnitte mit kurzen Unterbrechungen, zusammengefasst
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Session {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Gezählte Zeit, ohne die Unterbrechungen
    pub counted: TimeDelta,
}

/// Sitzungen aus den Abschnitten einer Aktivität, in zeitlicher Reihenfolge
pub fn sessions(spans: &[Span], gap: TimeDelta) -> Vec<Session> {
    let mut spans = spans.to_vec();
    spans.sort();
    let mut out: Vec<Session> = Vec::new();
    for (start, end) in spans {
        if end <= start {
            continue;
        }
        match out.last_mut() {
            Some(last) if start - last.end < gap => {
                last.end = last.end.max(end);
                last.counted += end - start;
            }
            _ => out.push(Session {
                start,
                end,
                counted: end - start,
            }),
        }
    }
    out
}

/// Gezählte Zeit zwischen je zwei aufeinanderfolgenden Grenzen (aufsteigend;
/// n Grenzen ergeben n − 1 Summen). Ein Abschnitt über eine Grenze zählt zu
/// beiden Seiten mit seinem Teil.
pub fn per_range(spans: &[Span], bounds: &[DateTime<Utc>]) -> Vec<TimeDelta> {
    bounds
        .windows(2)
        .map(|range| {
            spans
                .iter()
                .map(|&(start, end)| {
                    (end.min(range[1]) - start.max(range[0])).max(TimeDelta::zero())
                })
                .fold(TimeDelta::zero(), |sum, part| sum + part)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const LIMIT: TimeDelta = TimeDelta::seconds(60);

    fn t(seconds: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 11, 18, 0, 0).unwrap() + TimeDelta::seconds(seconds)
    }

    fn s(seconds: i64) -> TimeDelta {
        TimeDelta::seconds(seconds)
    }

    fn app(key: &str) -> Option<Front> {
        Some(Front::new(key, LIMIT))
    }

    fn seg(key: &str, start: i64, end: i64) -> Segment {
        Segment {
            key: key.into(),
            start: t(start),
            end: t(end),
        }
    }

    #[test]
    fn counts_while_working_until_the_switch() {
        let mut tracker = Tracker::new();
        assert_eq!(tracker.observe(t(0), app("editor"), s(0)), None);
        // Zwischendurch nachgefragt: Eingabe immer frisch
        for at in [50, 100, 150, 200, 250] {
            assert_eq!(tracker.observe(t(at), app("editor"), s(10)), None);
        }
        assert_eq!(
            tracker.observe(t(300), app("browser"), s(0)),
            Some(seg("editor", 0, 300))
        );
        assert_eq!(
            tracker.observe(t(330), None, s(0)),
            Some(seg("browser", 300, 330))
        );
    }

    #[test]
    fn a_pause_ends_at_the_last_input_and_work_resumes_with_input() {
        let mut tracker = Tracker::new();
        tracker.observe(t(0), app("editor"), s(0));
        // Letzte Eingabe bei 20 s, Grenze bei 80 s erreicht
        assert_eq!(
            tracker.observe(t(80), app("editor"), s(60)),
            Some(seg("editor", 0, 20))
        );
        // Pause: nichts läuft, nichts endet
        assert_eq!(tracker.running(), None);
        assert_eq!(tracker.observe(t(150), app("editor"), s(130)), None);
        // Bei 200 s geht es weiter, bemerkt bei 203 s
        assert_eq!(tracker.observe(t(203), app("editor"), s(3)), None);
        assert_eq!(tracker.running(), Some(seg("editor", 200, 203)));
        assert_eq!(
            tracker.observe(t(400), app("browser"), s(0)),
            Some(seg("editor", 200, 400))
        );
    }

    #[test]
    fn time_without_input_before_a_late_check_does_not_count() {
        let mut tracker = Tracker::new();
        tracker.observe(t(0), app("editor"), s(0));
        // Nachfrage kam zu spät (z. B. Standby): Eingabe vor 10 Minuten
        assert_eq!(
            tracker.observe(t(900), app("editor"), s(600)),
            Some(seg("editor", 0, 300))
        );
    }

    #[test]
    fn a_switch_without_input_ends_at_the_last_input() {
        let mut tracker = Tracker::new();
        tracker.observe(t(0), app("editor"), s(0));
        // Ein Fenster drängt sich nach vorn, lange nach der letzten Eingabe
        assert_eq!(
            tracker.observe(t(400), app("updater"), s(300)),
            Some(seg("editor", 0, 100))
        );
        // Das neue Fenster zählt erst mit einer Eingabe
        assert_eq!(tracker.running(), None);
        assert_eq!(tracker.observe(t(500), app("updater"), s(400)), None);
        assert_eq!(tracker.observe(t(505), None, s(405)), None);
    }

    #[test]
    fn short_breaks_before_a_switch_count() {
        let mut tracker = Tracker::new();
        tracker.observe(t(0), app("editor"), s(0));
        // 50 s ohne Eingabe, dann Wechsel: zählt bis zum Wechsel
        assert_eq!(
            tracker.observe(t(110), app("browser"), s(50)),
            Some(seg("editor", 0, 110))
        );
    }

    #[test]
    fn lock_and_untracked_apps_count_nothing() {
        let mut tracker = Tracker::new();
        assert_eq!(tracker.observe(t(0), None, s(0)), None);
        assert_eq!(tracker.next_check(), None);
        tracker.observe(t(10), app("editor"), s(0));
        // Bildschirm gesperrt (Win+L ist eine Eingabe)
        assert_eq!(
            tracker.observe(t(70), None, s(0)),
            Some(seg("editor", 10, 70))
        );
        assert_eq!(tracker.running(), None);
        assert_eq!(tracker.next_check(), None);
    }

    #[test]
    fn each_activity_has_its_own_limit() {
        let mut tracker = Tracker::new();
        let slow = || Some(Front::new("reader", TimeDelta::minutes(5)));
        tracker.observe(t(0), slow(), s(0));
        // Letzte Eingabe bei 100 s; 80 s ohne Eingabe liegen unter der Grenze
        assert_eq!(tracker.observe(t(180), slow(), s(80)), None);
        assert_eq!(tracker.running(), Some(seg("reader", 0, 180)));
        // Grenze bei 400 s erreicht: zählt bis zur letzten Eingabe
        assert_eq!(
            tracker.observe(t(400), slow(), s(300)),
            Some(seg("reader", 0, 100))
        );
    }

    #[test]
    fn changing_the_limit_starts_a_new_segment() {
        let mut tracker = Tracker::new();
        tracker.observe(t(0), app("editor"), s(0));
        let longer = Some(Front::new("editor", TimeDelta::minutes(2)));
        assert_eq!(
            tracker.observe(t(30), longer.clone(), s(0)),
            Some(seg("editor", 0, 30))
        );
        tracker.observe(t(90), longer, s(10));
        assert_eq!(tracker.running(), Some(seg("editor", 30, 90)));
    }

    #[test]
    fn a_clock_set_back_never_gives_negative_time() {
        let mut tracker = Tracker::new();
        tracker.observe(t(100), app("editor"), s(0));
        assert_eq!(tracker.observe(t(40), app("browser"), s(0)), None);
        assert_eq!(
            tracker.observe(t(90), None, s(0)),
            Some(seg("browser", 40, 90))
        );
    }

    fn span(start: i64, end: i64) -> Span {
        (t(start), t(end))
    }

    #[test]
    fn short_breaks_stay_in_one_session() {
        let gap = TimeDelta::minutes(15);
        // Unsortiert: 0–10 min, nach 5 min Pause 15–30 min, nach 20 min Pause 50–60 min
        let spans = [span(3000, 3600), span(0, 600), span(900, 1800)];
        let found = sessions(&spans, gap);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].start, found[0].end), (t(0), t(1800)));
        assert_eq!(found[0].counted, TimeDelta::minutes(25));
        assert_eq!(found[1].counted, TimeDelta::minutes(10));
        // Leere Abschnitte zählen nicht
        assert!(sessions(&[span(10, 10)], gap).is_empty());
    }

    #[test]
    fn spans_are_split_at_the_bounds() {
        let spans = [span(0, 100), span(150, 400)];
        let bounds = [t(50), t(200), t(300), t(1000)];
        assert_eq!(per_range(&spans, &bounds), vec![s(100), s(100), s(100)]);
        assert!(per_range(&spans, &[t(0)]).is_empty());
    }

    #[test]
    fn finish_closes_the_running_segment() {
        let mut tracker = Tracker::new();
        assert_eq!(tracker.finish(t(0)), None);
        tracker.observe(t(0), app("editor"), s(0));
        tracker.observe(t(30), app("editor"), s(5));
        assert_eq!(tracker.finish(t(40)), Some(seg("editor", 0, 40)));
        assert_eq!(tracker.finish(t(50)), None);
        // Beenden lange nach der letzten Eingabe: zählt bis zur Eingabe
        tracker.observe(t(100), app("editor"), s(0));
        tracker.observe(t(130), app("editor"), s(0));
        assert_eq!(tracker.finish(t(400)), Some(seg("editor", 100, 130)));
    }

    #[test]
    fn next_check_follows_the_limit_and_polls_during_a_pause() {
        let mut tracker = Tracker::new();
        tracker.observe(t(0), app("editor"), s(0));
        assert_eq!(tracker.next_check(), Some(s(60)));
        tracker.observe(t(60), app("editor"), s(15));
        assert_eq!(tracker.next_check(), Some(s(45)));
        // Kurz vor der Grenze geweckt: mindestens eine Sekunde warten
        tracker.observe(t(105), app("editor"), s(60) - TimeDelta::milliseconds(1));
        assert_eq!(tracker.next_check(), Some(MIN_CHECK));
        tracker.observe(t(200), app("editor"), s(155));
        assert_eq!(tracker.next_check(), Some(RESUME_POLL));
    }
}
