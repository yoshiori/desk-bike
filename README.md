# desk-bike

Records casual pedaling on an under-desk exercise bike from a Bluetooth LE
cadence sensor, and shows it live on a status bar.

It is made for light pedaling while working, not for training: there is no
power meter, no app to keep open and nothing to start or stop. Pedal and it is
recorded; stop and the session is saved.

Tested with an Alinco Aero Magnetic Bike Mini and a CYCPLUS C3 in cadence mode
on Linux (BlueZ). Any sensor that implements the standard Cycling Speed and
Cadence service (0x1816) with crank data should work.

## How it works

- A session starts with the first crank revolution and ends after 2 minutes
  without pedaling. Sessions shorter than 30 seconds of pedaling are dropped.
- Sessions are appended to `~/.local/share/health/desk-bike/sessions.jsonl`
  (the shared health data directory; override the root with `HEALTH_DATA_DIR`),
  one JSON
  object per line:

  ```json
  {"start": "2026-11-17T19:55:25+09:00", "end": "2026-11-17T19:56:03+09:00", "revolutions": 38, "moving_seconds": 38.7, "cadence_sq_seconds": 134512.4}
  ```

- The sensor only advertises once the crank turns, and connecting takes
  20-30 s. Revolutions made meanwhile are recovered from the sensor's
  cumulative counter (kept in `~/.local/state/desk-bike/sensor.json`; the C3
  restarts it at 0 after a long sleep), and their time is estimated from the
  session's cadence.
- While running, `$XDG_RUNTIME_DIR/desk-bike/status.json` is rewritten every
  second for a status bar, and deleted on exit:

  ```json
  {"show": true, "state": "pedaling", "cadence": 72, "session_minutes": 3, "today_minutes": 21}
  ```

  `state` is `"pedaling"` or `"stopped"`.
- Power is only an estimate, `k * cadence^2`, good for comparing days with
  each other. `k` is applied when reading, so changing it recomputes every
  past session.

## Build

Needs Rust (edition 2024) and the D-Bus headers for BlueZ
(`libdbus-1-dev` and `pkg-config` on Debian/Ubuntu).

```sh
cargo test
cargo install --path .        # installs ~/.cargo/bin/desk-bike
```

## Usage

```sh
desk-bike run                 # record; keeps running. Spin the crank to wake the sensor.
desk-bike today               # Today: 20 min · 2 sessions · 1200 revs · 60 rpm · ~54 W · ~65 kJ
desk-bike days -n 7           # one row per day
```

The sensor accepts one Bluetooth connection at a time: disconnect phones and
bike computers from it while recording.

### Run it as a systemd user service

`~/.config/systemd/user/desk-bike.service`:

```ini
[Unit]
Description=Record pedaling on the under-desk bike from its BLE cadence sensor

[Service]
Type=simple
ExecStart=%h/.cargo/bin/desk-bike run
Restart=always
RestartSec=10
Nice=5

[Install]
WantedBy=default.target
```

```sh
systemctl --user daemon-reload
systemctl --user enable --now desk-bike.service
journalctl --user -u desk-bike -f
```

### Show it on an eww bar

```lisp
;; "deskbike", not "desk-bike": a "-" in an eww expression reads as minus.
(defpoll deskbike :interval "2s"
                  :initial `{"show":false}`
  `cat "$XDG_RUNTIME_DIR/desk-bike/status.json" 2>/dev/null || echo '{"show":false}'`)

(defwidget desk-bike []
  (box :class "desk-bike desk-bike-${deskbike.state ?: 'stopped'}"
       :space-evenly false :spacing 6
       :visible {deskbike.show ?: false}
    (label :text "󰂣")
    (label :visible {(deskbike.state ?: "stopped") == "pedaling"}
           :text "${deskbike.cadence ?: 0} rpm · ${deskbike.session_minutes ?: 0}m")
    (label :visible {(deskbike.state ?: "stopped") != "pedaling"}
           :text "${deskbike.today_minutes ?: 0}m")))
```

Any bar that can read a JSON file works the same way.

## Config

Optional `~/.config/desk-bike/config.toml`:

```toml
power_coefficient = 0.015      # W per rpm^2 (60 rpm ~= 54 W)
sensor_name_prefix = "CYCPLUS" # or any sensor advertising the CSC service
```

## License

MIT
