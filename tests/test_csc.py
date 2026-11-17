import struct

from desk_bike.csc import CrankReading, parse_measurement

WHEEL = 0x01
CRANK = 0x02


def test_crank_only_measurement():
    data = struct.pack("<BHH", CRANK, 257, 2048)
    assert parse_measurement(data) == CrankReading(revolutions=257, event_time=2048)


def test_skips_wheel_data_before_crank_data():
    data = struct.pack("<BIHHH", WHEEL | CRANK, 1000, 500, 42, 1024)
    assert parse_measurement(data) == CrankReading(revolutions=42, event_time=1024)


def test_wheel_only_measurement_has_no_crank_reading():
    data = struct.pack("<BIH", WHEEL, 1000, 500)
    assert parse_measurement(data) is None


def test_truncated_measurement_has_no_crank_reading():
    assert parse_measurement(bytes([CRANK, 1, 0])) is None
    assert parse_measurement(b"") is None
