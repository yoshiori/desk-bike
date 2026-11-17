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
from desk_bike.store import append_session

log = logging.getLogger(__name__)

SCAN_TIMEOUT_SECONDS = 30
CONNECT_TIMEOUT_SECONDS = 20
TICK_SECONDS = 5
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


async def run(config: Config, sessions_path: Path) -> None:
    tracker = SessionTracker()
    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, stop.set)

    def save(sessions: list[Session]) -> None:
        for s in sessions:
            append_session(sessions_path, s)
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
        while not stop.is_set():
            save(tracker.tick(_now()))
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
                log.warning("sensor %s failed: %s. Retrying in %d s; if it keeps failing, check that "
                            "no phone or bike computer is connected to it.", device.address, e, RETRY_DELAY_SECONDS)
                await _wait_any(stop, timeout=RETRY_DELAY_SECONDS)
            finally:
                tracker.reset_baseline()
    finally:
        ticker.cancel()
        save(tracker.close())
        log.info("stopped")
