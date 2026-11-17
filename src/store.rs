//! Session log (one JSON object per line), per-day totals and the sensor's
//! last counters.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use chrono::{FixedOffset, NaiveDate};

use crate::csc::CrankReading;
use crate::session::{SECONDS_PER_MINUTE, Session};

const JOULES_PER_KILOJOULE: f64 = 1000.0;

pub fn append_session(path: &Path, session: &Session) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(session)?)
}

pub fn read_sessions(path: &Path) -> io::Result<Vec<Session>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(i, line)| {
            serde_json::from_str(line).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("{}: line {}: {e}", path.display(), i + 1))
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct DaySummary {
    pub day: NaiveDate,
    pub sessions: u32,
    pub revolutions: u32,
    pub moving_seconds: f64,
    pub joules: f64,
}

impl DaySummary {
    pub fn moving_minutes(&self) -> f64 {
        self.moving_seconds / SECONDS_PER_MINUTE
    }

    pub fn average_cadence(&self) -> f64 {
        if self.moving_seconds > 0.0 { self.revolutions as f64 * SECONDS_PER_MINUTE / self.moving_seconds } else { 0.0 }
    }

    pub fn average_watts(&self) -> f64 {
        if self.moving_seconds > 0.0 { self.joules / self.moving_seconds } else { 0.0 }
    }

    pub fn kilojoules(&self) -> f64 {
        self.joules / JOULES_PER_KILOJOULE
    }
}

/// Totals per local calendar day of each session's start, oldest first.
/// Watts are estimated as power_coefficient * cadence^2.
pub fn daily_summaries(sessions: &[Session], power_coefficient: f64, tz: &FixedOffset) -> Vec<DaySummary> {
    let mut days: BTreeMap<NaiveDate, DaySummary> = BTreeMap::new();
    for s in sessions {
        let day = s.start.with_timezone(tz).date_naive();
        let summary = days.entry(day).or_insert(DaySummary { day, sessions: 0, revolutions: 0, moving_seconds: 0.0, joules: 0.0 });
        summary.sessions += 1;
        summary.revolutions += s.revolutions;
        summary.moving_seconds += s.moving_seconds;
        summary.joules += power_coefficient * s.cadence_sq_seconds;
    }
    days.into_values().collect()
}

/// Keeps the sensor's counters across restarts, so revolutions made before
/// the next connection can be counted.
pub fn save_last_reading(path: &Path, reading: &CrankReading) -> io::Result<()> {
    write_atomically(path, &serde_json::to_string(reading)?)
}

pub fn load_last_reading(path: &Path) -> Option<CrankReading> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// Writes through a temporary file so readers never see half a file.
pub fn write_atomically(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeDelta, TimeZone};

    fn tokyo() -> FixedOffset {
        FixedOffset::east_opt(9 * 3600).unwrap()
    }

    fn session(day: u32, hour: u32, minutes: u32, rpm: f64) -> Session {
        let start = tokyo().with_ymd_and_hms(2026, 11, day, hour, 0, 0).unwrap();
        let secs = minutes as f64 * 60.0;
        Session::new(start, start + TimeDelta::seconds(secs as i64), (rpm * minutes as f64).round() as u32, secs, rpm * rpm * secs)
    }

    fn approx(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }

    #[test]
    fn round_trips_sessions_through_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("sessions.jsonl");
        let (first, second) = (session(17, 10, 5, 60.0), session(17, 11, 3, 70.0));
        append_session(&path, &first).unwrap();
        append_session(&path, &second).unwrap();
        assert_eq!(read_sessions(&path).unwrap(), vec![first, second]);
    }

    #[test]
    fn reads_lines_written_by_the_python_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.jsonl");
        fs::write(&path, concat!(
            r#"{"start": "2026-11-17T20:15:23.381292+09:00", "end": "2026-11-17T20:16:20.549708+09:00", "#,
            r#""revolutions": 67, "moving_seconds": 55.953, "cadence_sq_seconds": 288838.16}"#, "\n"
        )).unwrap();
        let sessions = read_sessions(&path).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].revolutions, 67);
        approx(sessions[0].moving_seconds, 55.953);
    }

    #[test]
    fn missing_file_reads_as_no_sessions() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_sessions(&dir.path().join("nope.jsonl")).unwrap().is_empty());
    }

    #[test]
    fn broken_line_names_the_file_and_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.jsonl");
        fs::write(&path, "{\n").unwrap();
        let err = read_sessions(&path).unwrap_err().to_string();
        assert!(err.contains("line 1"), "{err}");
    }

    #[test]
    fn daily_summary_adds_sessions_per_local_day() {
        let sessions = [session(16, 22, 10, 60.0), session(17, 10, 5, 60.0), session(17, 11, 15, 60.0)];
        let days = daily_summaries(&sessions, 0.015, &tokyo());
        assert_eq!(days.iter().map(|d| d.day.to_string()).collect::<Vec<_>>(), ["2026-11-16", "2026-11-17"]);
        let today = &days[1];
        assert_eq!(today.sessions, 2);
        approx(today.moving_minutes(), 20.0);
        assert_eq!(today.revolutions, 1200);
        approx(today.average_cadence(), 60.0);
        approx(today.average_watts(), 54.0); // 0.015 W/rpm^2 * 60^2
        approx(today.kilojoules(), 54.0 * 1200.0 / 1000.0);
    }

    #[test]
    fn summary_uses_the_local_date_of_the_start() {
        let late = session(17, 23, 30, 60.0); // 23:00-23:30 JST is still the 17th locally
        assert_eq!(daily_summaries(&[late], 0.015, &tokyo())[0].day.to_string(), "2026-11-17");
    }

    #[test]
    fn last_reading_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("sensor.json");
        let reading = CrankReading { revolutions: 1234, event_time: 5678 };
        save_last_reading(&path, &reading).unwrap();
        assert_eq!(load_last_reading(&path), Some(reading));
    }

    #[test]
    fn reads_the_python_versions_last_reading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sensor.json");
        fs::write(&path, r#"{"revolutions": 70, "event_time": 62930}"#).unwrap();
        assert_eq!(load_last_reading(&path), Some(CrankReading { revolutions: 70, event_time: 62930 }));
    }

    #[test]
    fn missing_or_broken_last_reading_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_last_reading(&dir.path().join("nope.json")), None);
        let broken = dir.path().join("broken.json");
        fs::write(&broken, "{").unwrap();
        assert_eq!(load_last_reading(&broken), None);
    }
}
