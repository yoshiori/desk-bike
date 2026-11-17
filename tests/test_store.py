from datetime import date, datetime, timedelta, timezone

import pytest

from desk_bike.csc import CrankReading
from desk_bike.session import Session
from desk_bike.store import (append_session, daily_summaries, load_last_reading, read_sessions,
                              save_last_reading)

TOKYO = timezone(timedelta(hours=9))


def session(day, hour, minutes, rpm):
    start = datetime(2026, 11, day, hour, 0, tzinfo=TOKYO)
    seconds = minutes * 60
    return Session(start=start, end=start + timedelta(seconds=seconds),
                   revolutions=round(rpm * minutes), moving_seconds=seconds,
                   cadence_sq_seconds=rpm * rpm * seconds)


def test_round_trips_sessions_through_jsonl(tmp_path):
    path = tmp_path / "sub" / "sessions.jsonl"
    first, second = session(17, 10, 5, 60), session(17, 11, 3, 70)
    append_session(path, first)
    append_session(path, second)
    assert read_sessions(path) == [first, second]


def test_missing_file_reads_as_no_sessions(tmp_path):
    assert read_sessions(tmp_path / "nope.jsonl") == []


def test_daily_summary_adds_sessions_per_local_day():
    sessions = [session(16, 22, 10, 60), session(17, 10, 5, 60), session(17, 11, 15, 60)]
    days = daily_summaries(sessions, power_coefficient=0.015, tz=TOKYO)
    assert [d.day for d in days] == [date(2026, 11, 16), date(2026, 11, 17)]
    today = days[1]
    assert today.sessions == 2
    assert today.moving_minutes == pytest.approx(20)
    assert today.revolutions == 1200
    assert today.average_cadence == pytest.approx(60)
    # 0.015 W/rpm^2 * 60^2 = 54 W for 20 minutes
    assert today.average_watts == pytest.approx(54)
    assert today.kilojoules == pytest.approx(54 * 1200 / 1000)


def test_summary_uses_the_local_date_of_the_start():
    late = session(17, 23, 30, 60)  # 23:00-23:30 JST is still the 17th locally
    days = daily_summaries([late], power_coefficient=0.015, tz=TOKYO)
    assert days[0].day == date(2026, 11, 17)


def test_last_reading_round_trips(tmp_path):
    path = tmp_path / "state" / "sensor.json"
    save_last_reading(path, CrankReading(1234, 5678))
    assert load_last_reading(path) == CrankReading(1234, 5678)


def test_missing_or_broken_last_reading_is_none(tmp_path):
    assert load_last_reading(tmp_path / "nope.json") is None
    broken = tmp_path / "broken.json"
    broken.write_text("{")
    assert load_last_reading(broken) is None
