use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse yaml config {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_yaml::Error,
    },
    #[error("invalid config: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub snmp: SnmpConfig,
    #[serde(default)]
    pub ups: UpsConfig,
    #[serde(default)]
    pub monitor: MonitorConfig,
    #[serde(default)]
    pub shutdown: ShutdownConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

impl AppConfig {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let config: AppConfig =
            serde_yaml::from_str(&text).map_err(|source| ConfigError::Parse {
                path: path.to_path_buf(),
                source,
            })?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.snmp.address.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "snmp.address must not be empty".into(),
            ));
        }
        if self.snmp.username.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "snmp.username must not be empty".into(),
            ));
        }
        if self.monitor.poll_interval.is_zero() {
            return Err(ConfigError::Invalid(
                "monitor.poll_interval must be greater than zero".into(),
            ));
        }
        if self.monitor.low_battery_percent > 100 {
            return Err(ConfigError::Invalid(
                "monitor.low_battery_percent must be between 0 and 100".into(),
            ));
        }
        if matches!(
            self.monitor.trigger,
            ShutdownTrigger::OnBatteryDuration | ShutdownTrigger::Either
        ) && self.monitor.max_on_battery.is_zero()
        {
            return Err(ConfigError::Invalid(
                "monitor.max_on_battery must be greater than zero when duration trigger is enabled"
                    .into(),
            ));
        }
        if !self.shutdown.dry_run && self.shutdown.command.is_empty() {
            return Err(ConfigError::Invalid(
                "shutdown.command must not be empty when shutdown.dry_run is false".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SnmpConfig {
    pub address: String,
    #[serde(default = "default_snmp_port")]
    pub port: u16,
    #[serde(default = "default_snmp_version")]
    pub version: String,
    #[serde(default = "default_security_level")]
    pub security_level: String,
    pub username: String,
    #[serde(default)]
    pub auth_protocol: Option<String>,
    #[serde(default)]
    pub auth_password: Option<String>,
    #[serde(default)]
    pub privacy_protocol: Option<String>,
    #[serde(default)]
    pub privacy_password: Option<String>,
    #[serde(default = "default_snmp_timeout")]
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,
    #[serde(default = "default_snmp_retries")]
    pub retries: u8,
}

impl Default for SnmpConfig {
    fn default() -> Self {
        Self {
            address: "192.168.0.255".into(),
            port: default_snmp_port(),
            version: default_snmp_version(),
            security_level: default_security_level(),
            username: "local3".into(),
            auth_protocol: None,
            auth_password: None,
            privacy_protocol: None,
            privacy_password: None,
            timeout: default_snmp_timeout(),
            retries: default_snmp_retries(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpsConfig {
    #[serde(default = "default_adapter")]
    pub adapter: String,
    #[serde(default)]
    pub oids: UpsOids,
    #[serde(default)]
    pub mapping: UpsMapping,
}

impl Default for UpsConfig {
    fn default() -> Self {
        Self {
            adapter: default_adapter(),
            oids: UpsOids::default(),
            mapping: UpsMapping::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpsOids {
    pub output_source: String,
    pub battery_charge_percent: String,
    pub battery_status: String,
    #[serde(default)]
    pub runtime_remaining_minutes: Option<String>,
}

impl Default for UpsOids {
    fn default() -> Self {
        Self {
            output_source: "1.3.6.1.2.1.33.1.4.1.0".into(),
            battery_charge_percent: "1.3.6.1.2.1.33.1.2.4.0".into(),
            battery_status: "1.3.6.1.2.1.33.1.2.1.0".into(),
            runtime_remaining_minutes: Some("1.3.6.1.2.1.33.1.2.3.0".into()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpsMapping {
    #[serde(default = "default_on_battery_values")]
    pub on_battery_values: Vec<String>,
    #[serde(default = "default_line_values")]
    pub line_values: Vec<String>,
    #[serde(default = "default_low_battery_values")]
    pub low_battery_values: Vec<String>,
    #[serde(default = "default_depleted_battery_values")]
    pub depleted_battery_values: Vec<String>,
}

impl Default for UpsMapping {
    fn default() -> Self {
        Self {
            on_battery_values: default_on_battery_values(),
            line_values: default_line_values(),
            low_battery_values: default_low_battery_values(),
            depleted_battery_values: default_depleted_battery_values(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MonitorConfig {
    #[serde(default = "default_poll_interval")]
    #[serde(with = "humantime_serde")]
    pub poll_interval: Duration,
    #[serde(default)]
    pub trigger: ShutdownTrigger,
    #[serde(default = "default_low_battery_percent")]
    pub low_battery_percent: u8,
    #[serde(default = "default_max_on_battery")]
    #[serde(with = "humantime_serde")]
    pub max_on_battery: Duration,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            poll_interval: default_poll_interval(),
            trigger: ShutdownTrigger::Either,
            low_battery_percent: default_low_battery_percent(),
            max_on_battery: default_max_on_battery(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownTrigger {
    BatteryCapacity,
    OnBatteryDuration,
    #[default]
    Either,
}

impl ShutdownTrigger {
    pub fn checks_capacity(self) -> bool {
        matches!(self, Self::BatteryCapacity | Self::Either)
    }

    pub fn checks_duration(self) -> bool {
        matches!(self, Self::OnBatteryDuration | Self::Either)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShutdownConfig {
    #[serde(default = "default_dry_run")]
    pub dry_run: bool,
    #[serde(default = "default_shutdown_command")]
    pub command: Vec<String>,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            dry_run: default_dry_run(),
            command: default_shutdown_command(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default)]
    pub file: Option<PathBuf>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            file: None,
        }
    }
}

fn default_snmp_port() -> u16 {
    161
}

fn default_snmp_version() -> String {
    "3".into()
}

fn default_security_level() -> String {
    "noAuthNoPriv".into()
}

fn default_snmp_timeout() -> Duration {
    Duration::from_secs(3)
}

fn default_snmp_retries() -> u8 {
    1
}

fn default_adapter() -> String {
    "ups_mib".into()
}

fn default_on_battery_values() -> Vec<String> {
    vec!["5".into(), "battery".into(), "battery(5)".into()]
}

fn default_line_values() -> Vec<String> {
    vec!["3".into(), "normal".into(), "normal(3)".into()]
}

fn default_low_battery_values() -> Vec<String> {
    vec![
        "3".into(),
        "batteryLow".into(),
        "battery_low".into(),
        "batteryLow(3)".into(),
    ]
}

fn default_depleted_battery_values() -> Vec<String> {
    vec![
        "4".into(),
        "batteryDepleted".into(),
        "battery_depleted".into(),
        "batteryDepleted(4)".into(),
    ]
}

fn default_poll_interval() -> Duration {
    Duration::from_secs(10)
}

fn default_low_battery_percent() -> u8 {
    30
}

fn default_max_on_battery() -> Duration {
    Duration::from_secs(60)
}

fn default_dry_run() -> bool {
    true
}

fn default_shutdown_command() -> Vec<String> {
    vec!["/sbin/shutdown".into(), "-h".into(), "+0".into()]
}

fn default_log_level() -> String {
    "info".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_humane_durations_and_trigger_mode() {
        let yaml = r#"
snmp:
  address: 192.168.0.255
  username: local3
monitor:
  poll_interval: 5s
  trigger: on_battery_duration
  max_on_battery: 90s
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.monitor.poll_interval, Duration::from_secs(5));
        assert_eq!(config.monitor.trigger, ShutdownTrigger::OnBatteryDuration);
        assert_eq!(config.monitor.max_on_battery, Duration::from_secs(90));
        config.validate().unwrap();
    }

    #[test]
    fn rejects_impossible_low_battery_percent() {
        let mut config = AppConfig::default();
        config.monitor.low_battery_percent = 101;

        assert!(config.validate().is_err());
    }
}
