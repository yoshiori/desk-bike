"""The live status the eww bar reads every couple of seconds."""

from datetime import datetime, timedelta, timezone

from desk_bike.csc import CrankReading
from desk_bike.live import live_status
from desk_bike.session import SessionTracker
from test_session import TICKS_PER_SECOND, pedal

TOKYO = timezone(timedelta(hours=9))
T0 = datetime(2026, 11, 17, 10, 0, tzinfo=TOKYO)


def started(seconds, rpm=72):
    tracker = SessionTracker()
    tracker.feed(T0, CrankReading(0, 0))
    pedal(tracker, T0, seconds=seconds, rpm=rpm)
    return tracker


def test_pedaling_shows_cadence_and_session_time():
    tracker = started(181)
    status = live_status(tracker, T0 + timedelta(seconds=181), saved_today_seconds=600)
    assert status["show"] is True
    assert status["state"] == "pedaling"
    assert 70 <= status["cadence"] <= 74
    assert status["session_minutes"] == 3
    assert status["today_minutes"] == 13  # 10 saved + 3 now


def test_stopped_pedaling_while_the_session_is_still_open():
    tracker = started(181)
    status = live_status(tracker, T0 + timedelta(seconds=191), saved_today_seconds=600)
    assert status["state"] == "stopped"
    assert status["cadence"] == 0
    assert status["today_minutes"] == 13


def test_idle_shows_only_todays_total():
    status = live_status(SessionTracker(), T0, saved_today_seconds=21 * 60)
    assert status == {"show": True, "state": "stopped", "cadence": 0,
                      "session_minutes": 0, "today_minutes": 21}


def test_cadence_follows_recent_pedaling():
    tracker = started(60, rpm=60)
    revs, event = 60, 60 * TICKS_PER_SECOND
    pedal(tracker, T0 + timedelta(seconds=60), seconds=10, rpm=90, revs=revs, event=event)
    status = live_status(tracker, T0 + timedelta(seconds=70), saved_today_seconds=0)
    assert 85 <= status["cadence"] <= 95


def test_unseen_revolutions_count_toward_the_session_time():
    tracker = SessionTracker(last_reading=CrankReading(100, 0))
    revs, event = 130, 30 * TICKS_PER_SECOND  # 30 revolutions before connecting
    tracker.feed(T0, CrankReading(revs, event))
    pedal(tracker, T0, seconds=35, rpm=70, revs=revs, event=event)
    status = live_status(tracker, T0 + timedelta(seconds=35), saved_today_seconds=0)
    assert status["session_minutes"] == 1  # 35 s + ~26 s estimated
