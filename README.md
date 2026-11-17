# desk-bike

Records casual pedaling on an under-desk exercise bike (Alinco Aero Magnetic
Bike Mini) from a Bluetooth cadence sensor (CYCPLUS C3 in cadence mode), and
shows it on the eww bar.

- A session starts with the first crank revolution and ends after 2 minutes
  without pedaling. Sessions shorter than 30 seconds of pedaling are dropped.
- Sessions are appended to `~/.local/share/desk-bike/sessions.jsonl`.
- The sensor only advertises once the crank turns, and connecting takes
  20-30 s. Revolutions made meanwhile are recovered from the sensor's counter
  (saved in `~/.local/state/desk-bike/sensor.json`; the C3 restarts it at 0
  after a long sleep), and their time is estimated from the session's cadence.
- While running, `$XDG_RUNTIME_DIR/desk-bike/status.json` is rewritten every
  second for the eww bar (`{"show", "state": "pedaling"|"stopped", "cadence",
  "session_minutes", "today_minutes"}`) and deleted on exit.
- Power is an estimate, `k * cadence^2`, for comparing days with each other
  only. `k` is applied when reading, so changing it recomputes every session.

## Build and run

Needs the D-Bus headers for BlueZ (`libdbus-1-dev`, `pkg-config`).

```sh
cargo test
cargo install --path .        # ~/.cargo/bin/desk-bike
desk-bike run                 # record (keeps running; spin the crank to wake the sensor)
desk-bike today               # Today: 20 min · 2 sessions · 1200 revs · 60 rpm · ~54 W · ~65 kJ
desk-bike days -n 7
```

`desk-bike run` runs as the systemd user service `desk-bike.service` (unit
in the dotfiles repo); logs: `journalctl --user -u desk-bike`.

The sensor accepts one Bluetooth connection at a time: disconnect phones and
bike computers from it while recording.

## Config

Optional `~/.config/desk-bike/config.toml`:

```toml
power_coefficient = 0.015      # W per rpm^2 (60 rpm ~= 54 W)
sensor_name_prefix = "CYCPLUS"
```
