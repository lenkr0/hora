//! Messtest für die App-Zeiten: schreibt mit, welche Anwendung vorn ist und
//! wann Eingaben kommen, und zählt die aktive Zeit je Anwendung nach der
//! Regel aus `hora_core::activity` (vorn und Eingabe innerhalb der
//! Pausengrenze). Läuft in der Konsole, bis Enter gedrückt wird.
//!
//! ```text
//! cargo run -p hora-platform --example activity-probe --release
//! cargo run -p hora-platform --example activity-probe --release -- --limit 90
//! cargo run -p hora-platform --example activity-probe --release -- --summary <protokoll.csv>
//! cargo run -p hora-platform --example activity-probe --release -- --apps
//! ```
//!
//! Das Protokoll enthält nur Programmnamen und Zeiten, keine Fenstertitel,
//! und bleibt auf diesem PC (Standard: im TEMP-Ordner). Zusätzlich zählt es
//! Stifteingaben, um zu prüfen, ob Windows sie als Eingabe meldet.
//! `--summary` wertet ein Protokoll erneut aus, auch mit anderer `--limit`.
//! `--for <Sekunden>` beendet den Test nach dieser Zeit statt mit Enter.
//! `--apps` zeigt nur die offenen Programme, wie Hora sie zur Auswahl anbietet.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, SecondsFormat, TimeDelta, Utc};
use hora_core::activity::{Front, Segment, Tracker};
use hora_platform::activity::{self, WatchEvent, Watcher};

const HEADER: &str = "zeit,art,programm,leerlauf_ms,stift";

enum Msg {
    Watch(WatchEvent),
    Quit,
}

#[derive(Default)]
struct Stats {
    /// Gezählte Zeit und Anzahl Abschnitte je Programm
    totals: BTreeMap<String, (TimeDelta, usize)>,
    first: Option<DateTime<Utc>>,
    last: Option<DateTime<Utc>>,
    /// Sekunden mit Stifteingaben, davon ohne Eingabe-Meldung von Windows
    pen_seconds: u64,
    pen_unseen: u64,
}

impl Stats {
    fn seen(&mut self, at: DateTime<Utc>) {
        self.first.get_or_insert(at);
        self.last = Some(at);
    }

    fn add(&mut self, segment: Option<Segment>) -> Option<Segment> {
        let segment = segment?;
        let entry = self
            .totals
            .entry(segment.key.clone())
            .or_insert((TimeDelta::zero(), 0));
        entry.0 += segment.duration();
        entry.1 += 1;
        Some(segment)
    }

    fn pen(&mut self, idle_ms: u64) {
        self.pen_seconds += 1;
        if idle_ms >= 1000 {
            self.pen_unseen += 1;
        }
    }

    fn print(&self, limit: TimeDelta) {
        let (Some(first), Some(last)) = (self.first, self.last) else {
            println!("Nichts beobachtet.");
            return;
        };
        println!();
        println!(
            "Auswertung (Pausengrenze {} s), {} bis {}, {} beobachtet:",
            limit.num_seconds(),
            first.with_timezone(&Local).format("%d.%m. %H:%M"),
            last.with_timezone(&Local).format("%d.%m. %H:%M"),
            hms(last - first)
        );
        let mut rows: Vec<_> = self.totals.iter().collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.1.0));
        for (name, (time, count)) in rows {
            println!("  {name:<36} {:>9}   {count} Abschnitt(e)", hms(*time));
        }
        println!();
        if self.pen_seconds == 0 {
            println!(
                "Stift: keine Stifteingaben gesehen (kein Stift angeschlossen, oder er meldet sich \
                 als Maus und zählt dann ohnehin als Eingabe)."
            );
        } else {
            println!(
                "Stift: in {} Sekunden kamen Stifteingaben, davon {} ohne Eingabe-Meldung von Windows.",
                self.pen_seconds, self.pen_unseen
            );
            if self.pen_unseen * 20 <= self.pen_seconds {
                println!("  Windows zählt den Stift als Eingabe.");
            } else {
                println!(
                    "  Ein Teil kam bei Windows nicht als Eingabe an (z. B. ein Stift, der \
                     nur schwebt). Ist die gezählte Zeit zu niedrig, muss Hora den Stift selbst beobachten."
                );
            }
        }
    }
}

fn hms(d: TimeDelta) -> String {
    let s = d.num_seconds().max(0);
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn fail(text: impl std::fmt::Display) -> ! {
    eprintln!("{text}");
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--apps") {
        for app in activity::open_apps() {
            println!("{:<36} {}", app.file_name(), app.exe.display());
        }
        return;
    }
    let limit = arg_value(&args, "--limit").map_or(60, |v| {
        v.parse::<i64>()
            .unwrap_or_else(|_| fail("--limit erwartet Sekunden"))
    });
    let limit = TimeDelta::seconds(limit);
    if let Some(path) = arg_value(&args, "--summary") {
        replay(Path::new(&path), limit);
        return;
    }
    let path = arg_value(&args, "--log").map_or_else(
        || {
            std::env::temp_dir().join(format!(
                "hora-activity-probe-{}.csv",
                Local::now().format("%Y%m%d-%H%M%S")
            ))
        },
        PathBuf::from,
    );
    let duration = arg_value(&args, "--for").map(|v| {
        Duration::from_secs(
            v.parse()
                .unwrap_or_else(|_| fail("--for erwartet Sekunden")),
        )
    });
    record(&path, limit, duration);
}

fn record(path: &Path, limit: TimeDelta, duration: Option<Duration>) {
    let file = File::create(path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
    let mut log = BufWriter::new(file);
    let _ = writeln!(log, "{HEADER}");

    let (sender, receiver) = mpsc::channel();
    let watch = sender.clone();
    let watcher = Watcher::new(true, move |event| {
        let _ = watch.send(Msg::Watch(event));
    })
    .unwrap_or_else(|e| fail(e));
    std::thread::spawn(move || {
        match duration {
            Some(duration) => std::thread::sleep(duration),
            None => {
                let mut line = String::new();
                let _ = std::io::stdin().read_line(&mut line);
            }
        }
        let _ = sender.send(Msg::Quit);
    });
    println!("Messtest läuft, Pausengrenze {} s.", limit.num_seconds());
    println!("Protokoll: {}", path.display());
    match duration {
        Some(d) => println!("Endet nach {} s und zeigt die Auswertung.", d.as_secs()),
        None => println!("Enter beendet ihn und zeigt die Auswertung."),
    }
    println!();

    let mut tracker = Tracker::new();
    let mut stats = Stats::default();
    let mut front: Option<String> = None;
    let mut locked = false;
    let mut due: Option<Instant> = None;
    let mut next_sample = Instant::now() + Duration::from_secs(1);
    let mut pen_seen = 0;
    loop {
        let wake = due.map_or(next_sample, |d| d.min(next_sample));
        let msg = receiver.recv_timeout(wake.saturating_duration_since(Instant::now()));
        let mut kind = match msg {
            Ok(Msg::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Msg::Watch(WatchEvent::Foreground(app))) => {
                // Komma trennt die Spalten im Protokoll
                front = app.map(|a| a.file_name().replace(',', ";"));
                Some("vorn")
            }
            Ok(Msg::Watch(WatchEvent::Locked)) => {
                locked = true;
                Some("gesperrt")
            }
            Ok(Msg::Watch(WatchEvent::Unlocked)) => {
                locked = false;
                front = activity::foreground_app().map(|a| a.file_name().replace(',', ";"));
                Some("entsperrt")
            }
            Err(RecvTimeoutError::Timeout) => None,
        };
        if kind.is_none() && due.is_some_and(|d| Instant::now() >= d) {
            kind = Some("check");
        }
        let at = Utc::now();
        let idle = activity::idle_time();
        let idle_ms = idle.as_millis() as u64;
        if let Some(kind) = kind {
            let key = if locked { None } else { front.clone() };
            let _ = writeln!(
                log,
                "{},{kind},{},{idle_ms},",
                stamp(at),
                key.as_deref().unwrap_or("")
            );
            let _ = log.flush();
            stats.seen(at);
            let observed = key.map(|k| Front::new(k, limit));
            let idle = TimeDelta::from_std(idle).unwrap_or_default();
            if let Some(segment) = stats.add(tracker.observe(at, observed, idle)) {
                print_segment(&segment, &stats);
            }
            due = tracker
                .next_check()
                .map(|d| Instant::now() + d.to_std().unwrap_or_default());
        }
        if Instant::now() >= next_sample {
            next_sample = Instant::now() + Duration::from_secs(1);
            let packets = watcher.pen_packets();
            if packets > pen_seen {
                stats.pen(idle_ms);
                let _ = writeln!(log, "{},stift,,{idle_ms},{}", stamp(at), packets - pen_seen);
                let _ = log.flush();
            }
            pen_seen = packets;
        }
    }
    drop(watcher);
    let at = Utc::now();
    stats.seen(at);
    if let Some(segment) = stats.add(tracker.finish(at)) {
        print_segment(&segment, &stats);
    }
    let _ = writeln!(log, "{},ende,,,", stamp(at));
    let _ = log.flush();
    stats.print(limit);
}

fn print_segment(segment: &Segment, stats: &Stats) {
    let total = stats
        .totals
        .get(&segment.key)
        .map_or(TimeDelta::zero(), |t| t.0);
    println!(
        "{}  {:<36} +{}  (zusammen {})",
        segment.end.with_timezone(&Local).format("%H:%M:%S"),
        segment.key,
        hms(segment.duration()),
        hms(total)
    );
}

/// Protokoll erneut auswerten
fn replay(path: &Path, limit: TimeDelta) {
    let file = File::open(path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
    let mut tracker = Tracker::new();
    let mut stats = Stats::default();
    for line in BufReader::new(file).lines().map_while(Result::ok).skip(1) {
        let fields: Vec<&str> = line.split(',').collect();
        let [time, kind, program, idle, _pen] = fields[..] else {
            continue;
        };
        let Ok(at) = DateTime::parse_from_rfc3339(time).map(|t| t.with_timezone(&Utc)) else {
            continue;
        };
        stats.seen(at);
        let idle_ms: u64 = idle.parse().unwrap_or(0);
        match kind {
            "stift" => stats.pen(idle_ms),
            "ende" => {
                stats.add(tracker.finish(at));
            }
            _ => {
                let front = (!program.is_empty()).then(|| Front::new(program, limit));
                let idle = TimeDelta::milliseconds(idle_ms as i64);
                stats.add(tracker.observe(at, front, idle));
            }
        }
    }
    if let Some(last) = stats.last {
        stats.add(tracker.finish(last));
    }
    stats.print(limit);
}
