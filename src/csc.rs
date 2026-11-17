//! Bluetooth Cycling Speed and Cadence (CSC) measurement parsing.

use serde::{Deserialize, Serialize};

const WHEEL_DATA_PRESENT: u8 = 0x01;
const CRANK_DATA_PRESENT: u8 = 0x02;
/// uint32 cumulative wheel revolutions + uint16 event time
const WHEEL_DATA_BYTES: usize = 6;
/// uint16 cumulative crank revolutions + uint16 event time
const CRANK_DATA_BYTES: usize = 4;

pub const EVENT_TICKS_PER_SECOND: f64 = 1024.0;

/// Raw sensor counters. Both wrap at 65536. `event_time` is in 1/1024 s and is
/// the time of the last revolution, not of the notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrankReading {
    pub revolutions: u16,
    pub event_time: u16,
}

pub fn parse_measurement(data: &[u8]) -> Option<CrankReading> {
    let flags = *data.first()?;
    if flags & CRANK_DATA_PRESENT == 0 {
        return None;
    }
    let offset = 1 + if flags & WHEEL_DATA_PRESENT != 0 { WHEEL_DATA_BYTES } else { 0 };
    let crank = data.get(offset..offset + CRANK_DATA_BYTES)?;
    Some(CrankReading {
        revolutions: u16::from_le_bytes([crank[0], crank[1]]),
        event_time: u16::from_le_bytes([crank[2], crank[3]]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crank_only(revs: u16, event: u16) -> Vec<u8> {
        let mut data = vec![CRANK_DATA_PRESENT];
        data.extend(revs.to_le_bytes());
        data.extend(event.to_le_bytes());
        data
    }

    #[test]
    fn crank_only_measurement() {
        assert_eq!(
            parse_measurement(&crank_only(257, 2048)),
            Some(CrankReading { revolutions: 257, event_time: 2048 })
        );
    }

    #[test]
    fn skips_wheel_data_before_crank_data() {
        let mut data = vec![WHEEL_DATA_PRESENT | CRANK_DATA_PRESENT];
        data.extend(1000u32.to_le_bytes());
        data.extend(500u16.to_le_bytes());
        data.extend(42u16.to_le_bytes());
        data.extend(1024u16.to_le_bytes());
        assert_eq!(parse_measurement(&data), Some(CrankReading { revolutions: 42, event_time: 1024 }));
    }

    #[test]
    fn wheel_only_measurement_has_no_crank_reading() {
        let mut data = vec![WHEEL_DATA_PRESENT];
        data.extend(1000u32.to_le_bytes());
        data.extend(500u16.to_le_bytes());
        assert_eq!(parse_measurement(&data), None);
    }

    #[test]
    fn truncated_measurement_has_no_crank_reading() {
        assert_eq!(parse_measurement(&[CRANK_DATA_PRESENT, 1, 0]), None);
        assert_eq!(parse_measurement(&[]), None);
    }
}
