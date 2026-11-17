"""The recording loop: find the sensor, stay subscribed, write sessions.

The sensor sleeps (stops advertising) when the crank is still, so this keeps
scanning until it shows up again. All decisions about sessions live in
SessionTracker; this module only moves bytes and handles BLE failures.
"""

import asyncio
import logging
import signal
from datetime import datetime
from pathlib import Path

from bleak import BleakClient, BleakScanner
from bleak.exc import BleakError

from desk_bike.config import Config
from desk_bike.csc import CSC_MEASUREMENT_UUID, CSC_SERVICE_UUID, parse_measurement
from desk_bike.session import Session, SessionTracker
from desk_bike.live import live_status, write_status
from desk_bike.store import append_session, load_last_reading, read_sessions, save_last_reading

log = logging.getLogger(__name__)

SCAN_TIMEOUT_SECONDS = 30
CONNECT_TIMEOUT_SECONDS = 20
TICK_SECONDS = 1  # also the bar's refresh rate
RETRY_DELAY_SECONDS = 5


def _now() -> datetime:
    return datetime.now().astimezone()


async def _wait_any(*events: asyncio.Event, timeout: float | None = None) -> None:
    waiters = [asyncio.create_task(e.wait()) for e in events]
    try:
        await asyncio.wait(waiters, timeout=timeout, return_when=asyncio.FIRST_COMPLETED)
    finally:
        for w in waiters:
            w.cancel()


async def run(config: Config, sessions_path: Path, sensor_state_path: Path, status_path: Path) -> None:
    # The sensor's counters from the last run let the first connection count
    # revolutions made while the sensor was waking up and connecting.
    tracker = SessionTracker(last_reading=load_last_reading(sensor_state_path))
    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, stop.set)

    today = {"day": None, "seconds": 0.0}

    def saved_today_seconds(now: datetime) -> float:
        if today["day"] != now.date():
            today["day"] = now.date()
            today["seconds"] = sum(s.moving_seconds for s in read_sessions(sessions_path)
                                   if s.start.astimezone(now.tzinfo).date() == now.date())
        return today["seconds"]

    def save(sessions: list[Session]) -> None:
        for s in sessions:
            append_session(sessions_path, s)
            if s.start.astimezone().date() == today["day"]:
                today["seconds"] += s.moving_seconds
            log.info("saved session %s: %.0f s, %d revs, %.0f rpm",
                     s.start.isoformat(timespec="seconds"), s.moving_seconds, s.revolutions, s.average_cadence)

    def on_measurement(_sender, data: bytearray) -> None:
        reading = parse_measurement(bytes(data))
        if reading is not None:
            tracker.feed(_now(), reading)

    def is_sensor(device, adv) -> bool:
        name = adv.local_name or device.name or ""
        return name.startswith(config.sensor_name_prefix) or CSC_SERVICE_UUID in adv.service_uuids

    async def tick_forever() -> None:
        # Saving the counters while connected, not only on disconnect, keeps
        # them when the process is killed or the PC loses power.
        saved_reading = tracker.last_reading
        while not stop.is_set():
            now = _now()
            save(tracker.tick(now))
            _publish(status_path, live_status(tracker, now, saved_today_seconds(now)))
            if tracker.last_reading != saved_reading:
                saved_reading = tracker.last_reading
                _remember_counters(tracker, sensor_state_path)
            await _wait_any(stop, timeout=TICK_SECONDS)

    ticker = asyncio.create_task(tick_forever())
    log.info("waiting for a sensor named %s* (spin the crank to wake it)", config.sensor_name_prefix)
    try:
        while not stop.is_set():
            device = await BleakScanner.find_device_by_filter(is_sensor, timeout=SCAN_TIMEOUT_SECONDS)
            if device is None:
                continue
            disconnected = asyncio.Event()
            try:
                async with BleakClient(device, disconnected_callback=lambda _c: disconnected.set(),
                                       timeout=CONNECT_TIMEOUT_SECONDS) as client:
                    await client.start_notify(CSC_MEASUREMENT_UUID, on_measurement)
                    log.info("connected to %s (%s)", device.name, device.address)
                    await _wait_any(disconnected, stop)
                    log.info("disconnected from %s", device.address)
            except (BleakError, TimeoutError, OSError) as e:
                log.warning("sensor %s failed: %s: %s. Retrying in %d s; if it keeps failing, check that "
                            "no phone or bike computer is connected to it.",
                            device.address, type(e).__name__, e, RETRY_DELAY_SECONDS)
                await _wait_any(stop, timeout=RETRY_DELAY_SECONDS)
            finally:
                tracker.reset_baseline()
                _remember_counters(tracker, sensor_state_path)
    finally:
        ticker.cancel()
        save(tracker.close())
        _remember_counters(tracker, sensor_state_path)
        status_path.unlink(missing_ok=True)  # hides the bar segment
        log.info("stopped")


def _remember_counters(tracker: SessionTracker, path: Path) -> None:
    if tracker.last_reading is None:
        return
    try:
        save_last_reading(path, tracker.last_reading)
    except OSError as e:
        log.warning("could not save the sensor counters to %s: %s. Revolutions made before the next "
                    "connection will not be counted.", path, e)


def _publish(path: Path, status: dict) -> None:
    try:
        write_status(path, status)
    except OSError as e:
        log.warning("could not write the live status to %s: %s. The eww bar will not update.", path, e)
