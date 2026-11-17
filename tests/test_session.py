from datetime import datetime, timedelta, timezone

import pytest

from desk_bike.csc import CrankReading
from desk_bike.session import SessionTracker

TOKYO = timezone(timedelta(hours=9))
T0 = datetime(2026, 11, 17, 10, 0, tzinfo=TOKYO)
TICKS_PER_SECOND = 1024


def pedal(tracker, start, seconds, rpm, revs=0, event=0):
    """Feeds one notification per second at a steady cadence. Returns the
    sensor counters after the last one, to continue from."""
    seconds_per_rev = 60 / rpm
    elapsed = 0.0
    for second in range(1, seconds + 1):
        while elapsed + seconds_per_rev <= second:
            elapsed += seconds_per_rev
            revs += 1
            event = round(elapsed * TICKS_PER_SECOND)
        tracker.feed(start + timedelta(seconds=second),
                     CrankReading(revs % 0x10000, event % 0x10000))
    return revs, event


def finish(tracker, at):
    sessions = tracker.tick(at)
    assert len(sessions) == 1
    return sessions[0]


def test_steady_pedaling_becomes_one_session():
    tracker = SessionTracker()
    tracker.feed(T0, CrankReading(0, 0))
    pedal(tracker, T0, seconds=120, rpm=60)
    session = finish(tracker, T0 + timedelta(minutes=10))
    assert session.revolutions == 120
    assert session.moving_seconds == pytest.approx(120, abs=1)
    assert session.average_cadence == pytest.approx(60, abs=1)
    # cadence^2 integrated over time: 60^2 rpm^2 * 120 s
    assert session.cadence_sq_seconds == pytest.approx(3600 * 120, rel=0.02)
    assert session.start == T0
    assert session.end <= T0 + timedelta(seconds=120)


def test_session_stays_open_until_pedaling_stops_long_enough():
    tracker = SessionTracker(end_after=timedelta(minutes=2))
    tracker.feed(T0, CrankReading(0, 0))
    pedal(tracker, T0, seconds=60, rpm=60)
    assert tracker.tick(T0 + timedelta(seconds=60 + 119)) == []
    assert len(tracker.tick(T0 + timedelta(seconds=60 + 121))) == 1
    assert tracker.tick(T0 + timedelta(seconds=60 + 300)) == []


def test_first_reading_after_connect_is_only_a_baseline():
    tracker = SessionTracker()
    # The sensor's counters carry over from earlier rides.
    tracker.feed(T0, CrankReading(5000, 30000))
    assert tracker.tick(T0 + timedelta(hours=1)) == []


def test_counters_wrap_around():
    tracker = SessionTracker()
    revs, event = 0xFFF0, 0xFC00
    tracker.feed(T0, CrankReading(revs, event))
    pedal(tracker, T0, seconds=60, rpm=60, revs=revs, event=event)
    session = finish(tracker, T0 + timedelta(minutes=10))
    assert session.revolutions == 60
    assert session.average_cadence == pytest.approx(60, abs=1)


def test_long_gap_counts_revolutions_but_not_time():
    """After a pause the sensor's last event is old, so the first new
    revolution says nothing about cadence."""
    tracker = SessionTracker(end_after=timedelta(minutes=2))
    tracker.feed(T0, CrankReading(0, 0))
    revs, event = pedal(tracker, T0, seconds=30, rpm=60)
    later = T0 + timedelta(seconds=90)
    tracker.feed(later, CrankReading(revs + 1, (event + 60 * TICKS_PER_SECOND) % 0x10000))
    session = finish(tracker, later + timedelta(minutes=10))
    assert session.revolutions == revs + 1
    assert session.moving_seconds == pytest.approx(30, abs=1)


def test_reconnect_keeps_the_session_open():
    tracker = SessionTracker(end_after=timedelta(minutes=2))
    tracker.feed(T0, CrankReading(0, 0))
    revs, event = pedal(tracker, T0, seconds=30, rpm=60)
    tracker.reset_baseline()  # BLE dropped and came back
    resumed = T0 + timedelta(seconds=40)
    tracker.feed(resumed, CrankReading(revs + 5, event + 5 * TICKS_PER_SECOND))
    pedal(tracker, resumed, seconds=30, rpm=60, revs=revs + 5, event=event + 5 * TICKS_PER_SECOND)
    session = finish(tracker, resumed + timedelta(minutes=10))
    assert session.revolutions == 60  # the 5 revolutions during the drop are lost
    assert session.start == T0


def test_too_short_sessions_are_dropped():
    tracker = SessionTracker(min_moving=timedelta(seconds=30))
    tracker.feed(T0, CrankReading(0, 0))
    pedal(tracker, T0, seconds=10, rpm=60)
    assert tracker.tick(T0 + timedelta(minutes=10)) == []


def test_close_returns_the_open_session():
    tracker = SessionTracker()
    tracker.feed(T0, CrankReading(0, 0))
    pedal(tracker, T0, seconds=60, rpm=60)
    sessions = tracker.close()
    assert len(sessions) == 1
    assert tracker.close() == []
