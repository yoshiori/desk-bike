//! desk-bike: records casual pedaling on an under-desk exercise bike from a
//! BLE cadence sensor. See README.md.

mod ble;
mod config;
mod csc;
mod live;
mod report;
mod session;
mod store;

use anyhow::Result;
use chrono::{Local, TimeDelta};
use clap::{Parser, Subcommand};

use crate::config::{config_path, load_config, sensor_state_path, sessions_path, status_path};
use crate::store::{DaySummary, daily_summaries, read_sessions};

const DEFAULT_DAYS_SHOWN: i64 = 14;

#[derive(Parser)]
#[command(version, about = "Record casual pedaling on an under-desk bike from a BLE cadence sensor")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Record sessions from the cadence sensor (runs until stopped)
    Run,
    /// Today's totals
    Today,
    /// Totals per day
    Days {
        /// How many recent days
        #[arg(short, default_value_t = DEFAULT_DAYS_SHOWN)]
        n: i64,
    },
}

fn summaries() -> Result<(chrono::NaiveDate, Vec<DaySummary>)> {
    let config = load_config(&config_path())?;
    let now = Local::now().fixed_offset();
    let sessions = read_sessions(&sessions_path())?;
    Ok((now.date_naive(), daily_summaries(&sessions, config.power_coefficient, now.offset())))
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Run => {
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                .format_timestamp(None) // journald adds its own
                .init();
            let config = load_config(&config_path())?;
            let paths = ble::Paths { sessions: sessions_path(), sensor_state: sensor_state_path(), status: status_path() };
            ble::run(config, paths).await
        }
        Command::Today => {
            let (today, days) = summaries()?;
            println!("{}", report::format_today(days.iter().find(|d| d.day == today)));
            Ok(())
        }
        Command::Days { n } => {
            let (today, days) = summaries()?;
            let since = today - TimeDelta::days(n - 1);
            let recent: Vec<_> = days.into_iter().filter(|d| d.day >= since).collect();
            println!("{}", report::format_days(&recent));
            Ok(())
        }
    }
}
