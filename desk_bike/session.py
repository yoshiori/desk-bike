"""Turns a stream of crank readings into pedaling sessions.

A session starts at the first revolution and ends once the crank has been
still for `end_after`. BLE drops in between keep the session open; only the
revolutions made while disconnected are lost.

Power is not stored: an exercise bike's magnetic brake takes roughly
power = k * cadence^2 at a fixed dial, and k is a guess. Sessions keep the
integral of cadence^2 over time instead, so watts and kJ can be recomputed
whenever k is changed.
"""

from dataclasses import dataclass, field
from datetime import datetime, timedelta

from desk_bike.csc import COUNTER_WRAP, EVENT_TICKS_PER_SECOND, CrankReading

DEFAULT_END_AFTER = timedelta(minutes=2)
# Shorter sessions are bumps of the crank rather than pedaling.
DEFAULT_MIN_MOVING = timedelta(seconds=30)
# Revolutions further apart than this are separate efforts: the gap tells
# nothing about cadence, so it adds neither moving time nor work.
MAX_REVOLUTION_GAP_SECONDS = 5.0
SECONDS_PER_MINUTE = 60


@dataclass
class Session:
    start: datetime
    end: datetime
    revolutions: int = 0
    moving_seconds: float = 0.0
    cadence_sq_seconds: float = 0.0  # integral of cadence (rpm) squared over seconds

    @property
    def average_cadence(self) -> float:
        if self.moving_seconds == 0:
            return 0.0
        return self.revolutions * SECONDS_PER_MINUTE / self.moving_seconds


@dataclass
class SessionTracker:
    end_after: timedelta = DEFAULT_END_AFTER
    min_moving: timedelta = DEFAULT_MIN_MOVING
    _last: CrankReading | None = None
    _current: Session | None = None
    _last_revolution_at: datetime | None = field(default=None)

    def reset_baseline(self) -> None:
        """Call after a reconnect: the next reading only sets the counters."""
        self._last = None

    def feed(self, now: datetime, reading: CrankReading) -> None:
        previous, self._last = self._last, reading
        if previous is None:
            return
        revolutions = (reading.revolutions - previous.revolutions) % COUNTER_WRAP
        if revolutions == 0:
            return
        seconds = ((reading.event_time - previous.event_time) % COUNTER_WRAP) / EVENT_TICKS_PER_SECOND
        if self._current is None:
            self._current = Session(start=now - timedelta(seconds=min(seconds, MAX_REVOLUTION_GAP_SECONDS)), end=now)
        session = self._current
        session.revolutions += revolutions
        session.end = now
        self._last_revolution_at = now
        if 0 < seconds <= MAX_REVOLUTION_GAP_SECONDS:
            cadence = revolutions * SECONDS_PER_MINUTE / seconds
            session.moving_seconds += seconds
            session.cadence_sq_seconds += cadence * cadence * seconds

    def tick(self, now: datetime) -> list[Session]:
        """Returns the session that ended because the crank stopped, if any."""
        if self._current is None or now - self._last_revolution_at < self.end_after:
            return []
        return self.close()

    def close(self) -> list[Session]:
        """Ends the open session now (on shutdown)."""
        session, self._current = self._current, None
        if session is None or session.moving_seconds < self.min_moving.total_seconds():
            return []
        return [session]
