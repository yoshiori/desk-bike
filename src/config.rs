//! User settings in ~/.config/desk-bike/config.toml (optional) and file paths.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Guess for the Alinco Aero Magnetic Bike Mini at its usual dial position
/// (between 2 and 3): power ~= k * cadence^2, so 60 rpm ~= 54 W. Nobody has
/// measured it; it only makes days comparable with each other. Change it in
/// config.toml and every past session is recomputed.
pub const DEFAULT_POWER_COEFFICIENT: f64 = 0.015;
pub const DEFAULT_SENSOR_NAME_PREFIX: &str = "CYCPLUS";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub power_coefficient: f64,
    pub sensor_name_prefix: String,
}

impl Default for Config {
    fn default() -> Self {
        Config { power_coefficient: DEFAULT_POWER_COEFFICIENT, sensor_name_prefix: DEFAULT_SENSOR_NAME_PREFIX.into() }
    }
}

pub fn load_config(path: &Path) -> Result<Config> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    toml::from_str(&text).with_context(|| {
        format!("{} is not valid; expected power_coefficient = <number> and sensor_name_prefix = \"<text>\"", path.display())
    })
}

fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn xdg(variable: &str, fallback: &str) -> PathBuf {
    env::var_os(variable).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(fallback))
}

pub fn config_path() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("desk-bike/config.toml")
}

/// Shared home for personal health records, read by other tools and agents:
/// $HEALTH_DATA_DIR, else $XDG_DATA_HOME/health, else ~/.local/share/health.
fn health_data_dir_from(health: Option<OsString>, xdg_data: Option<OsString>, home: PathBuf) -> PathBuf {
    let non_empty = |v: Option<OsString>| v.filter(|v| !v.is_empty()).map(PathBuf::from);
    non_empty(health)
        .or_else(|| non_empty(xdg_data).map(|d| d.join("health")))
        .unwrap_or_else(|| home.join(".local/share/health"))
}

pub fn sessions_path() -> PathBuf {
    health_data_dir_from(env::var_os("HEALTH_DATA_DIR"), env::var_os("XDG_DATA_HOME"), home())
        .join("desk-bike/sessions.jsonl")
}

pub fn sensor_state_path() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("desk-bike/sensor.json")
}

/// Live status for the eww bar; tmpfs, since it changes every second.
pub fn status_path() -> PathBuf {
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".cache"));
    runtime.join("desk-bike/status.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_config_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let config = load_config(&dir.path().join("config.toml")).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.power_coefficient, DEFAULT_POWER_COEFFICIENT);
        assert_eq!(config.sensor_name_prefix, "CYCPLUS");
    }

    #[test]
    fn reads_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "power_coefficient = 0.02\nsensor_name_prefix = \"Wahoo\"\n").unwrap();
        assert_eq!(load_config(&path).unwrap(), Config { power_coefficient: 0.02, sensor_name_prefix: "Wahoo".into() });
    }

    #[test]
    fn partial_config_keeps_other_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "power_coefficient = 0.02\n").unwrap();
        assert_eq!(load_config(&path).unwrap().sensor_name_prefix, DEFAULT_SENSOR_NAME_PREFIX);
    }

    #[test]
    fn health_dir_prefers_health_data_dir() {
        let dir = health_data_dir_from(Some("/h".into()), Some("/x".into()), PathBuf::from("/home/u"));
        assert_eq!(dir, PathBuf::from("/h"));
    }

    #[test]
    fn health_dir_falls_back_to_xdg_data_home() {
        let dir = health_data_dir_from(None, Some("/x".into()), PathBuf::from("/home/u"));
        assert_eq!(dir, PathBuf::from("/x/health"));
    }

    #[test]
    fn health_dir_defaults_under_home_and_ignores_empty_values() {
        let dir = health_data_dir_from(Some("".into()), Some("".into()), PathBuf::from("/home/u"));
        assert_eq!(dir, PathBuf::from("/home/u/.local/share/health"));
    }

    #[test]
    fn typo_is_an_error_that_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "power_coeficient = 0.02\n").unwrap();
        let err = format!("{:#}", load_config(&path).unwrap_err());
        assert!(err.contains("config.toml"), "{err}");
    }
}
