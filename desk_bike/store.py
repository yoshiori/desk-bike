"""Session log (one JSON object per line) and per-day totals."""

import json
from dataclasses import dataclass
from datetime import date, datetime, tzinfo
from pathlib import Path

from desk_bike.csc import CrankReading
from desk_bike.session import SECONDS_PER_MINUTE, Session

JOULES_PER_KILOJOULE = 1000


def append_session(path: Path, session: Session) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    record = {
        "start": session.start.isoformat(),
        "end": session.end.isoformat(),
        "revolutions": session.revolutions,
        "moving_seconds": round(session.moving_seconds, 3),
        "cadence_sq_seconds": round(session.cadence_sq_seconds, 3),
    }
    with path.open("a") as f:
        f.write(json.dumps(record) + "\n")


def read_sessions(path: Path) -> list[Session]:
    if not path.exists():
        return []
    sessions = []
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        record = json.loads(line)
        sessions.append(Session(
            start=datetime.fromisoformat(record["start"]),
            end=datetime.fromisoformat(record["end"]),
            revolutions=record["revolutions"],
            moving_seconds=record["moving_seconds"],
            cadence_sq_seconds=record["cadence_sq_seconds"],
        ))
    return sessions


@dataclass
class DaySummary:
    day: date
    sessions: int
    revolutions: int
    moving_seconds: float
    joules: float

    @property
    def moving_minutes(self) -> float:
        return self.moving_seconds / SECONDS_PER_MINUTE

    @property
    def average_cadence(self) -> float:
        return self.revolutions * SECONDS_PER_MINUTE / self.moving_seconds if self.moving_seconds else 0.0

    @property
    def average_watts(self) -> float:
        return self.joules / self.moving_seconds if self.moving_seconds else 0.0

    @property
    def kilojoules(self) -> float:
        return self.joules / JOULES_PER_KILOJOULE


def daily_summaries(sessions: list[Session], power_coefficient: float, tz: tzinfo) -> list[DaySummary]:
    """Totals per local calendar day of each session's start, oldest first.
    Watts are estimated as power_coefficient * cadence^2."""
    days: dict[date, DaySummary] = {}
    for s in sessions:
        day = s.start.astimezone(tz).date()
        summary = days.setdefault(day, DaySummary(day, 0, 0, 0.0, 0.0))
        summary.sessions += 1
        summary.revolutions += s.revolutions
        summary.moving_seconds += s.moving_seconds
        summary.joules += power_coefficient * s.cadence_sq_seconds
    return [days[d] for d in sorted(days)]


def save_last_reading(path: Path, reading: CrankReading) -> None:
    """Keeps the sensor's counters across restarts, so revolutions made
    before the next connection can be counted."""
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps({"revolutions": reading.revolutions, "event_time": reading.event_time}))
    tmp.replace(path)


def load_last_reading(path: Path) -> CrankReading | None:
    try:
        record = json.loads(path.read_text())
        return CrankReading(int(record["revolutions"]), int(record["event_time"]))
    except (OSError, ValueError, KeyError, TypeError):
        return None
