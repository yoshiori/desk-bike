//! The recording loop: find the sensor, stay subscribed, write sessions and the
//! live status.
//!
//! The sensor sleeps (stops advertising) when the crank is still, so this
//! keeps scanning until it shows up again. All decisions about sessions live
//! in SessionTracker; this module only moves bytes and handles BLE failures.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use btleplug::api::bleuuid::uuid_from_u16;
use btleplug::api::{Central, CentralEvent, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use chrono::{Local, NaiveDate};
use futures::StreamExt;
use log::{info, warn};
use tokio::signal::unix::{SignalKind, signal};
use tokio::time::{interval, sleep, timeout};
use uuid::Uuid;

use crate::config::Config;
use crate::csc::{CrankReading, parse_measurement};
use crate::live::live_status;
use crate::session::{Session, SessionTracker, Time};
use crate::store::{append_session, load_last_reading, read_sessions, save_last_reading, write_atomically};

const CSC_SERVICE_UUID: Uuid = uuid_from_u16(0x1816);
const CSC_MEASUREMENT_UUID: Uuid = uuid_from_u16(0x2A5B);

const SCAN_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Also the bar's refresh rate.
const TICK: Duration = Duration::from_secs(1);
/// Notifications stop silently when the link drops, so check it this often.
const LINK_CHECK: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_secs(5);
/// Each attempt waits bluez-async's fixed 5 s, so up to 20 s in total.
const SERVICE_DISCOVERY_ATTEMPTS: u32 = 4;

pub struct Paths {
    pub sessions: PathBuf,
    pub sensor_state: PathBuf,
    pub status: PathBuf,
}

fn now() -> Time {
    Local::now().fixed_offset()
}

/// Everything the BLE task and the ticker share.
struct Recorder {
    tracker: SessionTracker,
    paths: Paths,
    today: Option<NaiveDate>,
    saved_today_seconds: f64,
    saved_reading: Option<CrankReading>,
}

impl Recorder {
    fn new(paths: Paths) -> Self {
        // The sensor's counters from the last run let the first connection
        // count revolutions made while the sensor was waking up and connecting.
        let last = load_last_reading(&paths.sensor_state);
        Recorder { tracker: SessionTracker::new(last), paths, today: None, saved_today_seconds: 0.0, saved_reading: last }
    }

    fn save(&mut self, session: Session) {
        match append_session(&self.paths.sessions, &session) {
            Ok(()) => info!(
                "saved session {}: {:.0} s, {} revs, {:.0} rpm",
                session.start.format("%Y-%m-%dT%H:%M:%S%:z"),
                session.moving_seconds,
                session.revolutions,
                session.average_cadence()
            ),
            Err(e) => warn!("could not append the session to {}: {e}. It is lost.", self.paths.sessions.display()),
        }
        if Some(session.start.with_timezone(&Local).date_naive()) == self.today {
            self.saved_today_seconds += session.moving_seconds;
        }
    }

    fn saved_today_seconds(&mut self, now: Time) -> f64 {
        let day = now.date_naive();
        if self.today != Some(day) {
            self.today = Some(day);
            self.saved_today_seconds = match read_sessions(&self.paths.sessions) {
                Ok(sessions) => sessions
                    .iter()
                    .filter(|s| s.start.with_timezone(now.offset()).date_naive() == day)
                    .map(|s| s.moving_seconds)
                    .sum(),
                Err(e) => {
                    warn!("could not read {}: {e}. Today's total on the bar starts from 0.", self.paths.sessions.display());
                    0.0
                }
            };
        }
        self.saved_today_seconds
    }

    fn tick(&mut self) {
        let now = now();
        if let Some(session) = self.tracker.tick(now) {
            self.save(session);
        }
        let saved_today = self.saved_today_seconds(now);
        let status = live_status(&self.tracker, now, saved_today);
        let json = serde_json::to_string(&status).expect("LiveStatus serializes");
        if let Err(e) = write_atomically(&self.paths.status, &json) {
            warn!("could not write the live status to {}: {e}. The eww bar will not update.", self.paths.status.display());
        }
        // Saving while connected, not only on disconnect, keeps the counters
        // when the process is killed or the PC loses power.
        if self.tracker.last_reading != self.saved_reading {
            self.remember_counters();
        }
    }

    fn remember_counters(&mut self) {
        let Some(reading) = self.tracker.last_reading else { return };
        match save_last_reading(&self.paths.sensor_state, &reading) {
            Ok(()) => self.saved_reading = Some(reading),
            Err(e) => warn!(
                "could not save the sensor counters to {}: {e}. Revolutions made before the next connection will not be counted.",
                self.paths.sensor_state.display()
            ),
        }
    }

    fn shutdown(&mut self) {
        if let Some(session) = self.tracker.close() {
            self.save(session);
        }
        self.remember_counters();
        // A missing file hides the bar segment.
        let _ = std::fs::remove_file(&self.paths.status);
    }
}

type Shared = Arc<Mutex<Recorder>>;

fn lock(shared: &Shared) -> std::sync::MutexGuard<'_, Recorder> {
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub async fn run(config: Config, paths: Paths) -> Result<()> {
    let shared: Shared = Arc::new(Mutex::new(Recorder::new(paths)));
    let manager = Manager::new().await.context("cannot reach BlueZ over D-Bus; is bluetoothd running?")?;
    let adapter = manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .context("no Bluetooth adapter found; check `bluetoothctl show`")?;

    let ticker = {
        let shared = shared.clone();
        tokio::spawn(async move {
            let mut every = interval(TICK);
            loop {
                every.tick().await;
                lock(&shared).tick();
            }
        })
    };

    info!("waiting for a sensor named {}* (spin the crank to wake it)", config.sensor_name_prefix);
    let mut terminate = signal(SignalKind::terminate()).context("cannot listen for SIGTERM")?;
    let result = tokio::select! {
        result = record_forever(&adapter, &config, &shared) => result,
        _ = tokio::signal::ctrl_c() => Ok(()),
        _ = terminate.recv() => Ok(()),
    };
    ticker.abort();
    lock(&shared).shutdown();
    info!("stopped");
    result
}

async fn record_forever(adapter: &Adapter, config: &Config, shared: &Shared) -> Result<()> {
    loop {
        let Some(sensor) = find_sensor(adapter, &config.sensor_name_prefix).await? else { continue };
        match stream_readings(&sensor, shared).await {
            Ok(()) => info!("disconnected from {}", sensor.address()),
            Err(e) => {
                warn!(
                    "sensor {} failed: {e:#}. Retrying in {} s; if it keeps failing, check that no phone or bike computer is connected to it.",
                    sensor.address(),
                    RETRY_DELAY.as_secs()
                );
                sleep(RETRY_DELAY).await;
            }
        }
        let _ = sensor.disconnect().await;
        let mut recorder = lock(shared);
        recorder.tracker.reset_baseline();
        recorder.remember_counters();
    }
}

/// The first advertising sensor, or None when nothing showed up in time.
async fn find_sensor(adapter: &Adapter, name_prefix: &str) -> Result<Option<Peripheral>> {
    let mut events = adapter.events().await.context("cannot watch Bluetooth events")?;
    adapter.start_scan(ScanFilter::default()).await.context("cannot start a Bluetooth scan")?;
    let found = timeout(SCAN_TIMEOUT, async {
        while let Some(event) = events.next().await {
            let (CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id)) = event else { continue };
            let Ok(peripheral) = adapter.peripheral(&id).await else { continue };
            let Ok(Some(props)) = peripheral.properties().await else { continue };
            // BlueZ remembers devices it saw before; RSSI is only set for ones
            // advertising right now.
            let advertising = props.rssi.is_some();
            let named = props.local_name.as_deref().is_some_and(|n| n.starts_with(name_prefix));
            if advertising && (named || props.services.contains(&CSC_SERVICE_UUID)) {
                return Some(peripheral);
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    let _ = adapter.stop_scan().await;
    Ok(found)
}

/// bluez-async gives up on service discovery after 5 s, while BlueZ keeps
/// resolving; over a weak link that is often not enough. Waiting again on the
/// same connection is much faster than disconnecting, rescanning and
/// reconnecting (the bar showed nothing for most of a minute on 2026-11-17).
async fn discover_services(sensor: &Peripheral) -> Result<()> {
    let mut attempt = 1;
    loop {
        match sensor.discover_services().await {
            Ok(()) => return Ok(()),
            Err(e) if attempt < SERVICE_DISCOVERY_ATTEMPTS && sensor.is_connected().await.unwrap_or(false) => {
                info!("service discovery attempt {attempt} failed ({e}); waiting again on the same connection");
                attempt += 1;
            }
            Err(e) => return Err(e).context("cannot read the sensor's services"),
        }
    }
}

/// Feeds notifications to the tracker until the link drops.
async fn stream_readings(sensor: &Peripheral, shared: &Shared) -> Result<()> {
    timeout(CONNECT_TIMEOUT, sensor.connect()).await.context("timed out connecting")??;
    discover_services(sensor).await?;
    let measurement = sensor
        .characteristics()
        .into_iter()
        .find(|c| c.uuid == CSC_MEASUREMENT_UUID)
        .context("the sensor has no CSC measurement; is it a cadence sensor?")?;
    let mut notifications = sensor.notifications().await?;
    sensor.subscribe(&measurement).await.context("cannot subscribe to cadence notifications")?;
    let name = sensor.properties().await.ok().flatten().and_then(|p| p.local_name).unwrap_or_default();
    info!("connected to {name} ({})", sensor.address());

    let mut link_check = interval(LINK_CHECK);
    loop {
        tokio::select! {
            notification = notifications.next() => match notification {
                Some(n) if n.uuid == CSC_MEASUREMENT_UUID => {
                    if let Some(reading) = parse_measurement(&n.value) {
                        lock(shared).tracker.feed(now(), reading);
                    }
                }
                Some(_) => {}
                None => return Ok(()),
            },
            _ = link_check.tick() => {
                if !sensor.is_connected().await.unwrap_or(false) {
                    return Ok(());
                }
            }
        }
    }
}
