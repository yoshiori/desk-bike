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
    first_event = event
    elapsed = 0.0
    for second in range(1, seconds + 1):
        while elapsed + seconds_per_rev <= second:
            elapsed += seconds_per_rev
            revs += 1
            event = first_event + round(elapsed * TICKS_PER_SECOND)
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


def test_reconnect_keeps_the_session_open_and_recovers_missed_revolutions():
    tracker = SessionTracker(end_after=timedelta(minutes=2))
    tracker.feed(T0, CrankReading(0, 0))
    revs, event = pedal(tracker, T0, seconds=30, rpm=60)
    tracker.reset_baseline()  # BLE dropped and came back
    resumed = T0 + timedelta(seconds=40)
    revs, event = revs + 5, event + 5 * TICKS_PER_SECOND
    tracker.feed(resumed, CrankReading(revs, event))
    pedal(tracker, resumed, seconds=30, rpm=60, revs=revs, event=event)
    session = finish(tracker, resumed + timedelta(minutes=10))
    assert session.revolutions == 65
    assert session.moving_seconds == pytest.approx(65, abs=2)  # 5 revs at 60 rpm add 5 s
    assert session.start == T0


def test_revolutions_before_connecting_are_recovered():
    """The sensor wakes when the crank turns, but connecting takes ~25 s:
    the cumulative counter tells how many revolutions happened meanwhile."""
    tracker = SessionTracker()
    tracker.feed(T0, CrankReading(100, 0))  # last reading of an earlier connection
    tracker.reset_baseline()
    connected = T0 + timedelta(hours=1)
    revs, event = 130, 30 * TICKS_PER_SECOND  # 30 revolutions while connecting
    tracker.feed(connected, CrankReading(revs, event))
    pedal(tracker, connected, seconds=35, rpm=70, revs=revs, event=event)
    session = finish(tracker, connected + timedelta(minutes=10))
    assert session.revolutions == pytest.approx(30 + 41, abs=1)
    unseen_seconds = 30 * 60 / 70
    assert session.moving_seconds == pytest.approx(35 + unseen_seconds, abs=2)
    assert session.average_cadence == pytest.approx(70, abs=2)
    assert abs((session.start - (connected - timedelta(seconds=unseen_seconds))).total_seconds()) < 2


def test_known_counter_from_an_earlier_run_is_used():
    tracker = SessionTracker(last_reading=CrankReading(100, 0))
    revs, event = 130, 30 * TICKS_PER_SECOND
    tracker.feed(T0, CrankReading(revs, event))
    pedal(tracker, T0, seconds=35, rpm=70, revs=revs, event=event)
    session = finish(tracker, T0 + timedelta(minutes=10))
    assert session.revolutions == pytest.approx(71, abs=1)
    assert tracker.last_reading is not None


def test_counter_reset_by_deep_sleep_counts_from_zero():
    """After a long sleep the C3 restarts its counter at 0 (seen on
    2026-11-17: 95 before, 3 after a few turns), so the first reading after
    connecting is the number of revolutions since it woke up."""
    tracker = SessionTracker(last_reading=CrankReading(95, 40000))
    revs, event = 30, 30 * TICKS_PER_SECOND
    tracker.feed(T0, CrankReading(revs, event))
    pedal(tracker, T0, seconds=35, rpm=70, revs=revs, event=event)
    session = finish(tracker, T0 + timedelta(minutes=10))
    assert session.revolutions == pytest.approx(30 + 41, abs=1)


def test_implausible_counter_jump_is_not_counted():
    """A battery change resets the counter, and a long gap may hold rides
    nobody recorded; neither should land in one session."""
    tracker = SessionTracker(last_reading=CrankReading(100, 0))
    revs, event = 5000, 0
    tracker.feed(T0, CrankReading(revs, event))
    pedal(tracker, T0, seconds=60, rpm=60, revs=revs, event=event)
    session = finish(tracker, T0 + timedelta(minutes=10))
    assert session.revolutions == 60


def test_missed_revolutions_alone_use_a_typical_cadence():
    tracker = SessionTracker(last_reading=CrankReading(100, 0))
    tracker.feed(T0, CrankReading(160, 0))  # connected just as the pedaling stopped
    session = finish(tracker, T0 + timedelta(minutes=10))
    assert session.revolutions == 60
    assert session.moving_seconds == pytest.approx(60)  # at the 60 rpm fallback


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
