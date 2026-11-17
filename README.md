# desk-bike

Records casual pedaling on an under-desk exercise bike (Alinco Aero Magnetic
Bike Mini) from a Bluetooth cadence sensor (CYCPLUS C3 in cadence mode).

- A session starts with the first crank revolution and ends after 2 minutes
  without pedaling. Sessions shorter than 30 seconds of pedaling are dropped.
- Sessions are appended to `~/.local/share/desk-bike/sessions.jsonl`.
- Power is an estimate, `k * cadence^2`, for comparing days with each other
  only. `k` is applied when reading, so changing it recomputes every session.

## Usage

```sh
uv run desk-bike run      # record (keeps running; spin the crank to wake the sensor)
uv run desk-bike today    # Today: 20 min · 2 sessions · 1200 revs · 60 rpm · ~54 W · ~65 kJ
uv run desk-bike days -n 7
uv run pytest
```

The sensor accepts one Bluetooth connection at a time: disconnect phones and
bike computers from it while recording.

## Config

Optional `~/.config/desk-bike/config.toml`:

```toml
power_coefficient = 0.015      # W per rpm^2 (60 rpm ~= 54 W)
sensor_name_prefix = "CYCPLUS"
```
