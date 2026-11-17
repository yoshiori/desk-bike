//! Turns a stream of crank readings into pedaling sessions.
//!
//! A session starts at the first revolution and ends once the crank has been
//! still for `end_after`. BLE drops in between keep the session open. A
//! session never spans two days: at midnight it is split, so pedaling counts
//! for the day it was done on.
//!
//! Revolutions made while not connected are recovered from the sensor's
//! cumulative counter (which the C3 restarts at 0 after a long sleep): the
//! sensor only starts advertising once the crank turns and connecting takes
//! ~25 s, which lost 25 of 60 seconds in a test on 2026-11-17. Their time is
//! not known, so it is estimated from the session's measured cadence.
//!
//! Power is not stored: an exercise bike's magnetic brake takes roughly
//! power = k * cadence^2 at a fixed dial, and k is a guess. Sessions keep the
//! integral of cadence^2 over time instead, so watts and kJ can be recomputed
//! whenever k is changed.

use std::collections::VecDeque;

use chrono::{DateTime, FixedOffset, NaiveTime, TimeDelta, TimeZone};
use serde::{Deserialize, Serialize};

use crate::csc::{CrankReading, EVENT_TICKS_PER_SECOND};

pub type Time = DateTime<FixedOffset>;

pub const SECONDS_PER_MINUTE: f64 = 60.0;
const DEFAULT_END_AFTER_SECONDS: i64 = 120;
/// Shorter sessions are bumps of the crank rather than pedaling.
const DEFAULT_MIN_MOVING_SECONDS: f64 = 30.0;
/// Revolutions further apart than this are separate efforts: the gap tells
/// nothing about cadence, so it adds neither moving time nor work.
const MAX_REVOLUTION_GAP_SECONDS: f64 = 5.0;
/// Larger counter jumps mean a battery change or rides nobody recorded, not
/// the seconds it took to connect. 300 is 5 minutes at 60 rpm.
const MAX_UNSEEN_REVOLUTIONS: u32 = 300;
/// Cadence for unseen revolutions when the session has no measured pedaling.
const FALLBACK_CADENCE_RPM: f64 = 60.0;
/// Live cadence averages the last few seconds; with no revolution for
/// STOPPED_AFTER (slower than 15 rpm) the crank counts as still.
const CADENCE_WINDOW_SECONDS: f64 = 5.0;
const STOPPED_AFTER_SECONDS: f64 = 4.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub start: Time,
    pub end: Time,
    pub revolutions: u32,
    pub moving_seconds: f64,
    /// Integral of cadence (rpm) squared over seconds.
    pub cadence_sq_seconds: f64,
    /// Counted from the sensor's counter; their time is not estimated yet.
    #[serde(skip)]
    unseen_revolutions: u32,
    /// The session began before the connection.
    #[serde(skip)]
    starts_unseen: bool,
}

impl Session {
    pub fn new(start: Time, end: Time, revolutions: u32, moving_seconds: f64, cadence_sq_seconds: f64) -> Self {
        Session { start, end, revolutions, moving_seconds, cadence_sq_seconds, unseen_revolutions: 0, starts_unseen: false }
    }

    pub fn average_cadence(&self) -> f64 {
        if self.moving_seconds == 0.0 { 0.0 } else { self.revolutions as f64 * SECONDS_PER_MINUTE / self.moving_seconds }
    }
}

fn seconds(delta: TimeDelta) -> f64 {
    delta.num_milliseconds() as f64 / 1000.0
}

fn delta(seconds: f64) -> TimeDelta {
    TimeDelta::milliseconds((seconds * 1000.0).round() as i64)
}

#[derive(Debug)]
pub struct SessionTracker {
    end_after: TimeDelta,
    min_moving_seconds: f64,
    /// The sensor's counters as last seen, possibly from an earlier run.
    pub last_reading: Option<CrankReading>,
    connecting: bool,
    current: Option<Session>,
    last_revolution_at: Option<Time>,
    /// (received at, revolutions, seconds) of recent measured revolutions
    recent: VecDeque<(Time, u32, f64)>,
}

impl Default for SessionTracker {
    fn default() -> Self {
        Self::new(None)
    }
}

impl SessionTracker {
    pub fn new(last_reading: Option<CrankReading>) -> Self {
        SessionTracker {
            end_after: TimeDelta::seconds(DEFAULT_END_AFTER_SECONDS),
            min_moving_seconds: DEFAULT_MIN_MOVING_SECONDS,
            last_reading,
            connecting: true,
            current: None,
            last_revolution_at: None,
            recent: VecDeque::new(),
        }
    }

    #[cfg(test)]
    pub fn with_end_after(mut self, end_after: TimeDelta) -> Self {
        self.end_after = end_after;
        self
    }

    #[cfg(test)]
    pub fn with_min_moving(mut self, seconds: f64) -> Self {
        self.min_moving_seconds = seconds;
        self
    }

    /// Call after a disconnect: the next reading is compared with the last one
    /// only to count revolutions made while disconnected.
    pub fn reset_baseline(&mut self) {
        self.connecting = true;
    }

    pub fn feed(&mut self, now: Time, reading: CrankReading) {
        let previous = self.last_reading.replace(reading);
        if self.connecting {
            self.connecting = false;
            self.add_unseen(now, unseen_revolutions(previous, reading));
            return;
        }
        let Some(previous) = previous else { return };
        let revolutions = reading.revolutions.wrapping_sub(previous.revolutions) as u32;
        if revolutions == 0 {
            return;
        }
        let secs = reading.event_time.wrapping_sub(previous.event_time) as f64 / EVENT_TICKS_PER_SECOND;
        let session = self.session(now, now - delta(secs.min(MAX_REVOLUTION_GAP_SECONDS)));
        session.revolutions += revolutions;
        if secs > 0.0 && secs <= MAX_REVOLUTION_GAP_SECONDS {
            let cadence = revolutions as f64 * SECONDS_PER_MINUTE / secs;
            session.moving_seconds += secs;
            session.cadence_sq_seconds += cadence * cadence * secs;
            self.recent.push_back((now, revolutions, secs));
            while self.recent.front().is_some_and(|(at, _, _)| seconds(now - *at) > CADENCE_WINDOW_SECONDS) {
                self.recent.pop_front();
            }
        }
    }

    fn add_unseen(&mut self, now: Time, revolutions: u32) {
        if revolutions == 0 || revolutions > MAX_UNSEEN_REVOLUTIONS {
            return;
        }
        let starting = self.current.is_none();
        let session = self.session(now, now);
        session.starts_unseen |= starting;
        session.revolutions += revolutions;
        session.unseen_revolutions += revolutions;
    }

    fn session(&mut self, now: Time, start: Time) -> &mut Session {
        self.last_revolution_at = Some(now);
        let session = self.current.get_or_insert_with(|| Session::new(start, now, 0, 0.0, 0.0));
        session.end = now;
        session
    }

    pub fn is_pedaling(&self, now: Time) -> bool {
        self.last_revolution_at.is_some_and(|at| seconds(now - at) <= STOPPED_AFTER_SECONDS)
    }

    /// Cadence over the last few seconds, 0 when the crank is still.
    pub fn cadence(&self, now: Time) -> f64 {
        if !self.is_pedaling(now) {
            return 0.0;
        }
        let (revs, secs) = self
            .recent
            .iter()
            .filter(|(at, _, _)| seconds(now - *at) <= CADENCE_WINDOW_SECONDS)
            .fold((0u32, 0.0), |(r, s), (_, revs, secs)| (r + revs, s + secs));
        if secs > 0.0 { revs as f64 * SECONDS_PER_MINUTE / secs } else { 0.0 }
    }

    /// Pedaling time of the session in progress, as it would be saved.
    pub fn open_session_seconds(&self) -> f64 {
        self.current.as_ref().map_or(0.0, |s| s.moving_seconds + unseen_time(s).0)
    }

    /// Returns the session that ended because the crank stopped, if any.
    pub fn tick(&mut self, now: Time) -> Option<Session> {
        let still = self.last_revolution_at.is_some_and(|at| now - at >= self.end_after);
        if self.current.is_some() && still { self.close() } else { None }
    }

    /// Splits the open session at local midnight, so each day keeps the
    /// pedaling done on it. Returns the part before midnight; pedaling goes
    /// on in a new session that starts at midnight. Call every tick: the
    /// split happens on the first tick of the new day, so at most one tick of
    /// pedaling lands on the wrong day.
    pub fn split_day(&mut self, now: Time) -> Option<Session> {
        let session = self.current.as_ref()?;
        if session.start.with_timezone(now.offset()).date_naive() == now.date_naive() {
            return None;
        }
        let midnight = now
            .offset()
            .from_local_datetime(&now.date_naive().and_time(NaiveTime::MIN))
            .single()
            .expect("a fixed offset has exactly one midnight");
        let mut before = self.current.replace(Session::new(midnight, midnight, 0, 0.0, 0.0))?;
        estimate_unseen_time(&mut before);
        before.end = before.end.min(midnight);
        // Kept even when shorter than min_moving_seconds: it is the start of
        // a real session, not a bump of the crank.
        (before.moving_seconds > 0.0).then_some(before)
    }

    /// Ends the open session now (on shutdown).
    pub fn close(&mut self) -> Option<Session> {
        let mut session = self.current.take()?;
        estimate_unseen_time(&mut session);
        (session.moving_seconds >= self.min_moving_seconds).then_some(session)
    }
}

/// Revolutions between two connections. The C3 restarts its counter at 0
/// after a long sleep, so a smaller counter means it counted from 0 since
/// waking. (A real 16-bit wrap looks the same and is off by a few turns.)
fn unseen_revolutions(previous: Option<CrankReading>, reading: CrankReading) -> u32 {
    match previous {
        None => 0,
        Some(p) if reading.revolutions < p.revolutions => reading.revolutions as u32,
        Some(p) => (reading.revolutions - p.revolutions) as u32,
    }
}

/// (seconds, cadence) estimated for the session's unseen revolutions.
fn unseen_time(session: &Session) -> (f64, f64) {
    if session.unseen_revolutions == 0 {
        return (0.0, 0.0);
    }
    let measured = session.revolutions - session.unseen_revolutions;
    let cadence = if session.moving_seconds > 0.0 && measured > 0 {
        measured as f64 * SECONDS_PER_MINUTE / session.moving_seconds
    } else {
        FALLBACK_CADENCE_RPM
    };
    (session.unseen_revolutions as f64 * SECONDS_PER_MINUTE / cadence, cadence)
}

fn estimate_unseen_time(session: &mut Session) {
    let (secs, cadence) = unseen_time(session);
    if secs == 0.0 {
        return;
    }
    session.moving_seconds += secs;
    session.cadence_sq_seconds += cadence * cadence * secs;
    if session.starts_unseen {
        session.start -= delta(secs);
    }
    session.unseen_revolutions = 0;
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const TICKS_PER_SECOND: f64 = 1024.0;

    pub fn t0() -> Time {
        FixedOffset::east_opt(9 * 3600).unwrap().with_ymd_and_hms(2026, 11, 17, 10, 0, 0).unwrap()
    }

    pub fn at(start: Time, secs: f64) -> Time {
        start + delta(secs)
    }

    fn reading(revs: u32, event: u32) -> CrankReading {
        CrankReading { revolutions: (revs % 0x10000) as u16, event_time: (event % 0x10000) as u16 }
    }

    /// Feeds one notification per second at a steady cadence. Returns the
    /// sensor counters after the last one, to continue from.
    pub fn pedal(tracker: &mut SessionTracker, start: Time, secs: u32, rpm: f64, revs: u32, event: u32) -> (u32, u32) {
        let seconds_per_rev = 60.0 / rpm;
        let (mut revs, mut event, first_event) = (revs, event, event);
        let mut elapsed = 0.0;
        for second in 1..=secs {
            while elapsed + seconds_per_rev <= second as f64 {
                elapsed += seconds_per_rev;
                revs += 1;
                event = first_event + (elapsed * TICKS_PER_SECOND).round() as u32;
            }
            tracker.feed(at(start, second as f64), reading(revs, event));
        }
        (revs, event)
    }

    pub fn started(secs: u32, rpm: f64) -> SessionTracker {
        let mut tracker = SessionTracker::default();
        tracker.feed(t0(), reading(0, 0));
        pedal(&mut tracker, t0(), secs, rpm, 0, 0);
        tracker
    }

    fn finish(tracker: &mut SessionTracker, now: Time) -> Session {
        tracker.tick(now).expect("a finished session")
    }

    fn approx(actual: f64, expected: f64, tolerance: f64) {
        assert!((actual - expected).abs() <= tolerance, "{actual} is not within {tolerance} of {expected}");
    }

    #[test]
    fn steady_pedaling_becomes_one_session() {
        let mut tracker = started(120, 60.0);
        let session = finish(&mut tracker, at(t0(), 600.0));
        assert_eq!(session.revolutions, 120);
        approx(session.moving_seconds, 120.0, 1.0);
        approx(session.average_cadence(), 60.0, 1.0);
        approx(session.cadence_sq_seconds, 3600.0 * 120.0, 3600.0 * 120.0 * 0.02);
        assert_eq!(session.start, t0());
        assert!(session.end <= at(t0(), 120.0));
    }

    #[test]
    fn session_stays_open_until_pedaling_stops_long_enough() {
        let mut tracker = started(60, 60.0);
        assert!(tracker.tick(at(t0(), 60.0 + 119.0)).is_none());
        assert!(tracker.tick(at(t0(), 60.0 + 121.0)).is_some());
        assert!(tracker.tick(at(t0(), 60.0 + 300.0)).is_none());
    }

    #[test]
    fn first_reading_after_connect_is_only_a_baseline() {
        let mut tracker = SessionTracker::default();
        tracker.feed(t0(), reading(5000, 30000));
        assert!(tracker.tick(at(t0(), 3600.0)).is_none());
    }

    #[test]
    fn counters_wrap_around() {
        let mut tracker = SessionTracker::default();
        let (revs, event) = (0xFFF0, 0xFC00);
        tracker.feed(t0(), reading(revs, event));
        pedal(&mut tracker, t0(), 60, 60.0, revs, event);
        let session = finish(&mut tracker, at(t0(), 600.0));
        assert_eq!(session.revolutions, 60);
        approx(session.average_cadence(), 60.0, 1.0);
    }

    #[test]
    fn long_gap_counts_revolutions_but_not_time() {
        let mut tracker = started(0, 60.0);
        let (revs, event) = pedal(&mut tracker, t0(), 30, 60.0, 0, 0);
        let later = at(t0(), 90.0);
        tracker.feed(later, reading(revs + 1, event + 60 * 1024));
        let session = finish(&mut tracker, at(later, 600.0));
        assert_eq!(session.revolutions, revs + 1);
        approx(session.moving_seconds, 30.0, 1.0);
    }

    #[test]
    fn reconnect_keeps_the_session_open_and_recovers_missed_revolutions() {
        let mut tracker = started(0, 60.0);
        let (revs, event) = pedal(&mut tracker, t0(), 30, 60.0, 0, 0);
        tracker.reset_baseline(); // BLE dropped and came back
        let resumed = at(t0(), 40.0);
        let (revs, event) = (revs + 5, event + 5 * 1024);
        tracker.feed(resumed, reading(revs, event));
        pedal(&mut tracker, resumed, 30, 60.0, revs, event);
        let session = finish(&mut tracker, at(resumed, 600.0));
        assert_eq!(session.revolutions, 65);
        approx(session.moving_seconds, 65.0, 2.0); // 5 revs at 60 rpm add 5 s
        assert_eq!(session.start, t0());
    }

    #[test]
    fn revolutions_before_connecting_are_recovered() {
        let mut tracker = SessionTracker::default();
        tracker.feed(t0(), reading(100, 0)); // last reading of an earlier connection
        tracker.reset_baseline();
        let connected = at(t0(), 3600.0);
        let (revs, event) = (130, 30 * 1024); // 30 revolutions while connecting
        tracker.feed(connected, reading(revs, event));
        pedal(&mut tracker, connected, 35, 70.0, revs, event);
        let session = finish(&mut tracker, at(connected, 600.0));
        approx(session.revolutions as f64, 71.0, 1.0);
        let unseen_seconds = 30.0 * 60.0 / 70.0;
        approx(session.moving_seconds, 35.0 + unseen_seconds, 2.0);
        approx(session.average_cadence(), 70.0, 2.0);
        approx(seconds(session.start - at(connected, -unseen_seconds)), 0.0, 2.0);
    }

    #[test]
    fn known_counter_from_an_earlier_run_is_used() {
        let mut tracker = SessionTracker::new(Some(reading(100, 0)));
        let (revs, event) = (130, 30 * 1024);
        tracker.feed(t0(), reading(revs, event));
        pedal(&mut tracker, t0(), 35, 70.0, revs, event);
        let session = finish(&mut tracker, at(t0(), 600.0));
        approx(session.revolutions as f64, 71.0, 1.0);
        assert!(tracker.last_reading.is_some());
    }

    #[test]
    fn counter_reset_by_deep_sleep_counts_from_zero() {
        // Seen on 2026-11-17: 95 before, 3 after a few turns.
        let mut tracker = SessionTracker::new(Some(reading(95, 40000)));
        let (revs, event) = (30, 30 * 1024);
        tracker.feed(t0(), reading(revs, event));
        pedal(&mut tracker, t0(), 35, 70.0, revs, event);
        let session = finish(&mut tracker, at(t0(), 600.0));
        approx(session.revolutions as f64, 71.0, 1.0);
    }

    #[test]
    fn implausible_counter_jump_is_not_counted() {
        let mut tracker = SessionTracker::new(Some(reading(100, 0)));
        tracker.feed(t0(), reading(5000, 0));
        pedal(&mut tracker, t0(), 60, 60.0, 5000, 0);
        let session = finish(&mut tracker, at(t0(), 600.0));
        assert_eq!(session.revolutions, 60);
    }

    #[test]
    fn missed_revolutions_alone_use_a_typical_cadence() {
        let mut tracker = SessionTracker::new(Some(reading(100, 0)));
        tracker.feed(t0(), reading(160, 0)); // connected just as the pedaling stopped
        let session = finish(&mut tracker, at(t0(), 600.0));
        assert_eq!(session.revolutions, 60);
        approx(session.moving_seconds, 60.0, 0.01); // at the 60 rpm fallback
    }

    #[test]
    fn too_short_sessions_are_dropped() {
        let mut tracker = SessionTracker::default().with_min_moving(30.0);
        tracker.feed(t0(), reading(0, 0));
        pedal(&mut tracker, t0(), 10, 60.0, 0, 0);
        assert!(tracker.tick(at(t0(), 600.0)).is_none());
    }

    #[test]
    fn close_returns_the_open_session() {
        let mut tracker = started(60, 60.0);
        assert!(tracker.close().is_some());
        assert!(tracker.close().is_none());
    }

    fn before_midnight() -> Time {
        FixedOffset::east_opt(9 * 3600).unwrap().with_ymd_and_hms(2026, 11, 17, 23, 59, 0).unwrap()
    }

    fn midnight() -> Time {
        FixedOffset::east_opt(9 * 3600).unwrap().with_ymd_and_hms(2026, 11, 18, 0, 0, 0).unwrap()
    }

    #[test]
    fn a_session_across_midnight_is_split_into_the_two_days() {
        let mut tracker = SessionTracker::default();
        tracker.feed(before_midnight(), reading(0, 0));
        let (revs, event) = pedal(&mut tracker, before_midnight(), 60, 60.0, 0, 0);
        assert!(tracker.split_day(at(before_midnight(), 59.0)).is_none()); // still the same day

        let yesterday = tracker.split_day(midnight()).expect("the part before midnight");
        assert_eq!(yesterday.start, before_midnight());
        assert!(yesterday.end <= midnight());
        assert_eq!(yesterday.revolutions, 60);
        approx(yesterday.moving_seconds, 60.0, 1.0);
        assert_eq!(tracker.open_session_seconds(), 0.0);
        assert!(tracker.is_pedaling(midnight())); // the bar keeps showing the cadence

        pedal(&mut tracker, midnight(), 90, 60.0, revs, event);
        let today = finish(&mut tracker, at(midnight(), 300.0));
        assert_eq!(today.start, midnight());
        assert_eq!(today.revolutions, 90);
        approx(today.moving_seconds, 90.0, 1.0);
    }

    #[test]
    fn splitting_is_done_once_per_day() {
        let mut tracker = SessionTracker::default();
        tracker.feed(before_midnight(), reading(0, 0));
        pedal(&mut tracker, before_midnight(), 60, 60.0, 0, 0);
        assert!(tracker.split_day(midnight()).is_some());
        assert!(tracker.split_day(at(midnight(), 1.0)).is_none());
    }

    #[test]
    fn a_short_part_before_midnight_is_kept() {
        // The 30 s minimum is for bumps of the crank; this is the start of a
        // real session and its time must not be lost.
        let mut tracker = SessionTracker::default();
        let late = at(midnight(), -10.0);
        tracker.feed(late, reading(0, 0));
        pedal(&mut tracker, late, 10, 60.0, 0, 0);
        let yesterday = tracker.split_day(midnight()).expect("the part before midnight");
        approx(yesterday.moving_seconds, 10.0, 1.0);
    }

    #[test]
    fn nothing_to_split_without_an_open_session() {
        let mut tracker = SessionTracker::default();
        assert!(tracker.split_day(midnight()).is_none());
    }

    #[test]
    fn a_stopped_session_left_open_over_midnight_is_dropped_after_midnight() {
        // Pedaling ended at 23:59:30; the session closes 2 minutes later. The
        // empty part after midnight must not become a session of its own.
        let mut tracker = SessionTracker::default();
        let late = at(midnight(), -90.0);
        tracker.feed(late, reading(0, 0));
        pedal(&mut tracker, late, 60, 60.0, 0, 0);
        assert!(tracker.split_day(midnight()).is_some());
        assert!(tracker.tick(at(midnight(), 120.0)).is_none());
    }

    #[test]
    fn end_after_is_configurable() {
        let mut tracker = started(60, 60.0);
        tracker = tracker.with_end_after(TimeDelta::seconds(30));
        assert!(tracker.tick(at(t0(), 60.0 + 31.0)).is_some());
    }
}
