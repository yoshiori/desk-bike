"""User settings in ~/.config/desk-bike/config.toml (optional)."""

import os
import tomllib
from dataclasses import dataclass
from pathlib import Path

# Guess for the Alinco Aero Magnetic Bike Mini at its usual dial position
# (between 2 and 3): power ~= k * cadence^2, so 60 rpm ~= 54 W. Nobody has
# measured it; it only makes days comparable with each other. Change it in
# config.toml and every past session is recomputed.
DEFAULT_POWER_COEFFICIENT = 0.015
DEFAULT_SENSOR_NAME_PREFIX = "CYCPLUS"


@dataclass(frozen=True)
class Config:
    power_coefficient: float = DEFAULT_POWER_COEFFICIENT
    sensor_name_prefix: str = DEFAULT_SENSOR_NAME_PREFIX


def _xdg(variable: str, fallback: str) -> Path:
    return Path(os.environ.get(variable) or Path.home() / fallback)


def config_path() -> Path:
    return _xdg("XDG_CONFIG_HOME", ".config") / "desk-bike" / "config.toml"


def sessions_path() -> Path:
    return _xdg("XDG_DATA_HOME", ".local/share") / "desk-bike" / "sessions.jsonl"


def load_config(path: Path) -> Config:
    if not path.exists():
        return Config()
    values = tomllib.loads(path.read_text())
    return Config(
        power_coefficient=float(values.get("power_coefficient", DEFAULT_POWER_COEFFICIENT)),
        sensor_name_prefix=str(values.get("sensor_name_prefix", DEFAULT_SENSOR_NAME_PREFIX)),
    )
