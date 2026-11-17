"""Bluetooth Cycling Speed and Cadence (CSC) measurement parsing."""

import struct
from dataclasses import dataclass

CSC_SERVICE_UUID = "00001816-0000-1000-8000-00805f9b34fb"
CSC_MEASUREMENT_UUID = "00002a5b-0000-1000-8000-00805f9b34fb"

WHEEL_DATA_PRESENT = 0x01
CRANK_DATA_PRESENT = 0x02
WHEEL_DATA_BYTES = 6  # uint32 cumulative wheel revolutions + uint16 event time
CRANK_FORMAT = "<HH"  # uint16 cumulative crank revolutions + uint16 event time

COUNTER_WRAP = 0x10000
EVENT_TICKS_PER_SECOND = 1024


@dataclass(frozen=True)
class CrankReading:
    """Raw sensor counters: both wrap at 65536. event_time is in 1/1024 s and
    is the time of the last revolution, not of the notification."""

    revolutions: int
    event_time: int


def parse_measurement(data: bytes) -> CrankReading | None:
    if not data:
        return None
    flags = data[0]
    if not flags & CRANK_DATA_PRESENT:
        return None
    offset = 1 + (WHEEL_DATA_BYTES if flags & WHEEL_DATA_PRESENT else 0)
    if len(data) < offset + struct.calcsize(CRANK_FORMAT):
        return None
    revolutions, event_time = struct.unpack_from(CRANK_FORMAT, data, offset)
    return CrankReading(revolutions, event_time)
