//! Live status for the eww bar: a small JSON file rewritten every second.
//!
//! The file is the interface to the bar (dotfiles: eww/widgets/desk-bike.yuck),
//! so its fields must stay as they are.

use serde::Serialize;

use crate::session::{SECONDS_PER_MINUTE, SessionTracker, Time};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiveStatus {
    pub show: bool,
    /// "pedaling" while the crank turns, otherwise "stopped".
    pub state: &'static str,
    pub cadence: u32,
    pub session_minutes: u32,
    pub today_minutes: u32,
}

fn minutes(seconds: f64) -> u32 {
    (seconds.round() / SECONDS_PER_MINUTE).floor() as u32
}

pub fn live_status(tracker: &SessionTracker, now: Time, saved_today_seconds: f64) -> LiveStatus {
    let pedaling = tracker.is_pedaling(now);
    let session_seconds = tracker.open_session_seconds();
    LiveStatus {
        show: true,
        state: if pedaling { "pedaling" } else { "stopped" },
        cadence: if pedaling { tracker.cadence(now).round() as u32 } else { 0 },
        session_minutes: minutes(session_seconds),
        today_minutes: minutes(saved_today_seconds + session_seconds),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csc::CrankReading;
    use crate::session::tests::{at, pedal, started, t0};

    #[test]
    fn pedaling_shows_cadence_and_session_time() {
        let tracker = started(181, 72.0);
        let status = live_status(&tracker, at(t0(), 181.0), 600.0);
        assert!(status.show);
        assert_eq!(status.state, "pedaling");
        assert!((70..=74).contains(&status.cadence), "{}", status.cadence);
        assert_eq!(status.session_minutes, 3);
        assert_eq!(status.today_minutes, 13); // 10 saved + 3 now
    }

    #[test]
    fn stopped_pedaling_while_the_session_is_still_open() {
        let tracker = started(181, 72.0);
        let status = live_status(&tracker, at(t0(), 191.0), 600.0);
        assert_eq!(status.state, "stopped");
        assert_eq!(status.cadence, 0);
        assert_eq!(status.today_minutes, 13);
    }

    #[test]
    fn idle_shows_only_todays_total() {
        let status = live_status(&SessionTracker::default(), t0(), 21.0 * 60.0);
        assert_eq!(
            serde_json::to_string(&status).unwrap(),
            r#"{"show":true,"state":"stopped","cadence":0,"session_minutes":0,"today_minutes":21}"#
        );
    }

    #[test]
    fn cadence_follows_recent_pedaling() {
        let mut tracker = started(60, 60.0);
        pedal(&mut tracker, at(t0(), 60.0), 10, 90.0, 60, 60 * 1024);
        let status = live_status(&tracker, at(t0(), 70.0), 0.0);
        assert!((85..=95).contains(&status.cadence), "{}", status.cadence);
    }

    #[test]
    fn unseen_revolutions_count_toward_the_session_time() {
        let mut tracker = SessionTracker::new(Some(CrankReading { revolutions: 100, event_time: 0 }));
        let (revs, event) = (130, 30 * 1024); // 30 revolutions before connecting
        tracker.feed(t0(), CrankReading { revolutions: revs as u16, event_time: event as u16 });
        pedal(&mut tracker, t0(), 35, 70.0, revs, event);
        assert_eq!(live_status(&tracker, at(t0(), 35.0), 0.0).session_minutes, 1); // 35 s + ~26 s
    }
}
