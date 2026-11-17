"""Live status for the eww bar: a small JSON file rewritten every second.

The file is the interface to the bar, so a rewrite of the recorder (e.g. in
Rust) only has to keep writing the same fields.
"""

import json
from datetime import datetime
from pathlib import Path

from desk_bike.session import SECONDS_PER_MINUTE, SessionTracker


def _minutes(seconds: float) -> int:
    return int(round(seconds)) // SECONDS_PER_MINUTE


def live_status(tracker: SessionTracker, now: datetime, saved_today_seconds: float) -> dict:
    """state is "pedaling" while the crank turns, otherwise "stopped"."""
    pedaling = tracker.is_pedaling(now)
    session_seconds = tracker.open_session_seconds()
    return {
        "show": True,
        "state": "pedaling" if pedaling else "stopped",
        "cadence": round(tracker.cadence(now)) if pedaling else 0,
        "session_minutes": _minutes(session_seconds),
        "today_minutes": _minutes(saved_today_seconds + session_seconds),
    }


def write_status(path: Path, status: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(status))
    tmp.replace(path)  # atomic, so the bar never reads half a file
