"""desk-bike run | today | days [-n N]"""

import argparse
import asyncio
import logging
from datetime import datetime, timedelta

from desk_bike.config import config_path, load_config, sessions_path
from desk_bike.store import DaySummary, daily_summaries, read_sessions

DEFAULT_DAYS_SHOWN = 14


def format_today(day: DaySummary | None) -> str:
    if day is None:
        return "Today: no pedaling yet"
    noun = "session" if day.sessions == 1 else "sessions"
    return (f"Today: {round(day.moving_minutes)} min · {day.sessions} {noun} · {day.revolutions} revs"
            f" · {round(day.average_cadence)} rpm · ~{round(day.average_watts)} W · ~{round(day.kilojoules)} kJ")


def format_days(days: list[DaySummary]) -> str:
    lines = [f"{'date':<10}{'min':>5}{'sess':>6}{'revs':>7}{'rpm':>5}{'~W':>6}{'~kJ':>6}"]
    for d in days:
        lines.append(f"{d.day.isoformat()}{round(d.moving_minutes):>5}{d.sessions:>6}{d.revolutions:>7}"
                     f"{round(d.average_cadence):>5}{round(d.average_watts):>6}{round(d.kilojoules):>6}")
    return "\n".join(lines)


def _summaries():
    config = load_config(config_path())
    now = datetime.now().astimezone()
    return now, daily_summaries(read_sessions(sessions_path()), config.power_coefficient, now.tzinfo)


def main(argv=None) -> None:
    parser = argparse.ArgumentParser(prog="desk-bike", description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("run", help="record sessions from the cadence sensor (runs until stopped)")
    sub.add_parser("today", help="today's totals")
    days_parser = sub.add_parser("days", help="totals per day")
    days_parser.add_argument("-n", type=int, default=DEFAULT_DAYS_SHOWN, help="how many recent days")
    args = parser.parse_args(argv)

    if args.command == "run":
        from desk_bike.ble import run  # bleak is only needed for recording
        logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
        asyncio.run(run(load_config(config_path()), sessions_path()))
    elif args.command == "today":
        now, days = _summaries()
        print(format_today(next((d for d in days if d.day == now.date()), None)))
    else:
        now, days = _summaries()
        since = now.date() - timedelta(days=args.n - 1)
        print(format_days([d for d in days if d.day >= since]))


if __name__ == "__main__":
    main()
