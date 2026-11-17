"""Turns a stream of crank readings into pedaling sessions.

A session starts at the first revolution and ends once the crank has been
still for `end_after`. BLE drops in between keep the session open.

Revolutions made while not connected are recovered from the sensor's
cumulative counter (which restarts at 0 after a long sleep): the sensor only starts advertising once the crank turns
and connecting takes ~25 s, which lost 25 of 60 seconds in a test on
2026-11-17. Their time is not known, so it is estimated from the session's
measured cadence.

Power is not stored: an exercise bike's magnetic brake takes roughly
power = k * cadence^2 at a fixed dial, and k is a guess. Sessions keep the
integral of cadence^2 over time instead, so watts and kJ can be recomputed
whenever k is changed.
"""

from collections import deque
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
# Larger counter jumps mean a battery change (counter reset) or rides nobody
# recorded, not the seconds it took to connect. 300 is 5 minutes at 60 rpm.
MAX_UNSEEN_REVOLUTIONS = 300
# Cadence for unseen revolutions when the session has no measured pedaling.
FALLBACK_CADENCE_RPM = 60.0
# Live cadence averages the last few seconds; with no revolution for
# STOPPED_AFTER (slower than 15 rpm) the crank counts as still.
CADENCE_WINDOW = timedelta(seconds=5)
STOPPED_AFTER = timedelta(seconds=4)


@dataclass
class Session:
    start: datetime
    end: datetime
    revolutions: int = 0
    moving_seconds: float = 0.0
    cadence_sq_seconds: float = 0.0  # integral of cadence (rpm) squared over seconds
    unseen_revolutions: int = 0  # counted from the sensor's counter, time not yet estimated
    starts_unseen: bool = False  # the session began before the connection

    @property
    def average_cadence(self) -> float:
        if self.moving_seconds == 0:
            return 0.0
        return self.revolutions * SECONDS_PER_MINUTE / self.moving_seconds


@dataclass
class SessionTracker:
    end_after: timedelta = DEFAULT_END_AFTER
    min_moving: timedelta = DEFAULT_MIN_MOVING
    # The sensor's counters as last seen, possibly from an earlier run.
    last_reading: CrankReading | None = None
    _connecting: bool = True
    _current: Session | None = None
    _last_revolution_at: datetime | None = field(default=None)
    # (received at, revolutions, seconds) of recent measured revolutions
    _recent: deque = field(default_factory=deque)

    def reset_baseline(self) -> None:
        """Call after a disconnect: the next reading is compared with the last
        one only to count revolutions made while disconnected."""
        self._connecting = True

    def feed(self, now: datetime, reading: CrankReading) -> None:
        previous, self.last_reading = self.last_reading, reading
        if self._connecting:
            self._connecting = False
            self._add_unseen(now, _unseen_revolutions(previous, reading))
            return
        revolutions = (reading.revolutions - previous.revolutions) % COUNTER_WRAP
        if revolutions == 0:
            return
        seconds = ((reading.event_time - previous.event_time) % COUNTER_WRAP) / EVENT_TICKS_PER_SECOND
        session = self._session(now, start=now - timedelta(seconds=min(seconds, MAX_REVOLUTION_GAP_SECONDS)))
        session.revolutions += revolutions
        if 0 < seconds <= MAX_REVOLUTION_GAP_SECONDS:
            cadence = revolutions * SECONDS_PER_MINUTE / seconds
            session.moving_seconds += seconds
            session.cadence_sq_seconds += cadence * cadence * seconds
            self._recent.append((now, revolutions, seconds))
            while now - self._recent[0][0] > CADENCE_WINDOW:
                self._recent.popleft()

    def _add_unseen(self, now: datetime, revolutions: int) -> None:
        if not 0 < revolutions <= MAX_UNSEEN_REVOLUTIONS:
            return
        starting = self._current is None
        session = self._session(now, start=now)
        session.starts_unseen = session.starts_unseen or starting
        session.revolutions += revolutions
        session.unseen_revolutions += revolutions

    def _session(self, now: datetime, start: datetime) -> Session:
        if self._current is None:
            self._current = Session(start=start, end=now)
        self._current.end = now
        self._last_revolution_at = now
        return self._current

    def is_pedaling(self, now: datetime) -> bool:
        return self._last_revolution_at is not None and now - self._last_revolution_at <= STOPPED_AFTER

    def cadence(self, now: datetime) -> float:
        """Cadence over the last CADENCE_WINDOW, 0 when the crank is still."""
        if not self.is_pedaling(now):
            return 0.0
        recent = [(revs, secs) for at, revs, secs in self._recent if now - at <= CADENCE_WINDOW]
        seconds = sum(secs for _, secs in recent)
        return sum(revs for revs, _ in recent) * SECONDS_PER_MINUTE / seconds if seconds else 0.0

    def open_session_seconds(self) -> float:
        """Pedaling time of the session in progress, as it would be saved."""
        if self._current is None:
            return 0.0
        return self._current.moving_seconds + _unseen_time(self._current)[0]

    def tick(self, now: datetime) -> list[Session]:
        """Returns the session that ended because the crank stopped, if any."""
        if self._current is None or now - self._last_revolution_at < self.end_after:
            return []
        return self.close()

    def close(self) -> list[Session]:
        """Ends the open session now (on shutdown)."""
        session, self._current = self._current, None
        if session is None:
            return []
        _estimate_unseen_time(session)
        if session.moving_seconds < self.min_moving.total_seconds():
            return []
        return [session]


def _unseen_revolutions(previous: CrankReading | None, reading: CrankReading) -> int:
    """Revolutions between two connections. The C3 restarts its counter at 0
    after a long sleep, so a smaller counter means it counted from 0 since
    waking. (A real 16-bit wrap looks the same and is off by a few turns.)"""
    if previous is None:
        return 0
    if reading.revolutions < previous.revolutions:
        return reading.revolutions
    return reading.revolutions - previous.revolutions


def _unseen_time(session: Session) -> tuple[float, float]:
    """(seconds, cadence) estimated for the session's unseen revolutions."""
    if session.unseen_revolutions == 0:
        return 0.0, 0.0
    measured_revolutions = session.revolutions - session.unseen_revolutions
    if session.moving_seconds > 0 and measured_revolutions > 0:
        cadence = measured_revolutions * SECONDS_PER_MINUTE / session.moving_seconds
    else:
        cadence = FALLBACK_CADENCE_RPM
    return session.unseen_revolutions * SECONDS_PER_MINUTE / cadence, cadence


def _estimate_unseen_time(session: Session) -> None:
    seconds, cadence = _unseen_time(session)
    if seconds == 0:
        return
    session.moving_seconds += seconds
    session.cadence_sq_seconds += cadence * cadence * seconds
    if session.starts_unseen:
        session.start -= timedelta(seconds=seconds)
    session.unseen_revolutions = 0
