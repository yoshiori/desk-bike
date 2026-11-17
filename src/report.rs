//! Text for `desk-bike today` and `desk-bike days`.

use crate::store::DaySummary;

pub fn format_today(day: Option<&DaySummary>) -> String {
    let Some(d) = day else { return "Today: no pedaling yet".into() };
    let noun = if d.sessions == 1 { "session" } else { "sessions" };
    format!(
        "Today: {} min · {} {noun} · {} revs · {} rpm · ~{} W · ~{} kJ",
        d.moving_minutes().round(),
        d.sessions,
        d.revolutions,
        d.average_cadence().round(),
        d.average_watts().round(),
        d.kilojoules().round()
    )
}

pub fn format_days(days: &[DaySummary]) -> String {
    let mut lines = vec![format!("{:<10}{:>5}{:>6}{:>7}{:>5}{:>6}{:>6}", "date", "min", "sess", "revs", "rpm", "~W", "~kJ")];
    for d in days {
        lines.push(format!(
            "{}{:>5}{:>6}{:>7}{:>5}{:>6}{:>6}",
            d.day,
            d.moving_minutes().round(),
            d.sessions,
            d.revolutions,
            d.average_cadence().round(),
            d.average_watts().round(),
            d.kilojoules().round()
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn day(d: u32, sessions: u32, revolutions: u32, moving_seconds: f64, joules: f64) -> DaySummary {
        DaySummary { day: NaiveDate::from_ymd_opt(2026, 11, d).unwrap(), sessions, revolutions, moving_seconds, joules }
    }

    #[test]
    fn today_line() {
        assert_eq!(
            format_today(Some(&day(17, 2, 1200, 1200.0, 64800.0))),
            "Today: 20 min · 2 sessions · 1200 revs · 60 rpm · ~54 W · ~65 kJ"
        );
    }

    #[test]
    fn today_without_pedaling() {
        assert_eq!(format_today(None), "Today: no pedaling yet");
    }

    #[test]
    fn days_table() {
        let days = [day(16, 1, 600, 600.0, 32400.0), day(17, 2, 1200, 1200.0, 64800.0)];
        assert_eq!(
            format_days(&days).lines().collect::<Vec<_>>(),
            [
                "date        min  sess   revs  rpm    ~W   ~kJ",
                "2026-11-16   10     1    600   60    54    32",
                "2026-11-17   20     2   1200   60    54    65",
            ]
        );
    }
}
