from datetime import date, datetime, timedelta, timezone

from desk_bike.cli import format_days, format_today
from desk_bike.config import DEFAULT_POWER_COEFFICIENT, Config, load_config
from desk_bike.store import DaySummary

TOKYO = timezone(timedelta(hours=9))


def test_defaults_when_config_is_missing(tmp_path):
    config = load_config(tmp_path / "config.toml")
    assert config == Config()
    assert config.power_coefficient == DEFAULT_POWER_COEFFICIENT
    assert config.sensor_name_prefix == "CYCPLUS"


def test_reads_overrides(tmp_path):
    path = tmp_path / "config.toml"
    path.write_text('power_coefficient = 0.02\nsensor_name_prefix = "Wahoo"\n')
    assert load_config(path) == Config(power_coefficient=0.02, sensor_name_prefix="Wahoo")


def test_today_line():
    day = DaySummary(date(2026, 11, 17), sessions=2, revolutions=1200, moving_seconds=1200, joules=64800)
    assert format_today(day) == "Today: 20 min · 2 sessions · 1200 revs · 60 rpm · ~54 W · ~65 kJ"


def test_today_without_pedaling():
    assert format_today(None) == "Today: no pedaling yet"


def test_days_table():
    days = [
        DaySummary(date(2026, 11, 16), 1, 600, 600, 32400),
        DaySummary(date(2026, 11, 17), 2, 1200, 1200, 64800),
    ]
    assert format_days(days).splitlines() == [
        "date        min  sess   revs  rpm    ~W   ~kJ",
        "2026-11-16   10     1    600   60    54    32",
        "2026-11-17   20     2   1200   60    54    65",
    ]
