use crate::{
    config::{UpsConfig, UpsMapping, UpsOids},
    snmp::{SnmpClient, SnmpError},
};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UpsError {
    #[error(transparent)]
    Snmp(#[from] SnmpError),
    #[error("unsupported UPS adapter: {0}")]
    UnsupportedAdapter(String),
    #[error("missing SNMP value for OID {0}")]
    MissingValue(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsSample {
    pub power_source: PowerSource,
    pub battery_charge_percent: Option<u8>,
    pub battery_health: BatteryHealth,
    pub seconds_on_battery: Option<u32>,
    pub runtime_remaining_minutes: Option<u32>,
    pub raw: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerSource {
    Line,
    Battery,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryHealth {
    Normal,
    Low,
    Depleted,
    Unknown,
}

pub trait UpsAdapter {
    fn read_sample(&self, client: &dyn SnmpClient) -> Result<UpsSample, UpsError>;
}

pub fn build_adapter(config: UpsConfig) -> Result<Box<dyn UpsAdapter>, UpsError> {
    match config.adapter.as_str() {
        "ups_mib" | "santak_ups_mib" => {
            Ok(Box::new(UpsMibAdapter::new(config.oids, config.mapping)))
        }
        other => Err(UpsError::UnsupportedAdapter(other.into())),
    }
}

#[derive(Debug, Clone)]
pub struct UpsMibAdapter {
    oids: UpsOids,
    mapping: UpsMapping,
}

impl UpsMibAdapter {
    pub fn new(oids: UpsOids, mapping: UpsMapping) -> Self {
        Self { oids, mapping }
    }

    fn oid_list(&self) -> Vec<String> {
        let mut oids = vec![
            self.oids.output_source.clone(),
            self.oids.battery_charge_percent.clone(),
            self.oids.battery_status.clone(),
        ];
        if let Some(oid) = &self.oids.seconds_on_battery {
            oids.push(oid.clone());
        }
        if let Some(oid) = &self.oids.runtime_remaining_minutes {
            oids.push(oid.clone());
        }
        oids
    }
}

impl UpsAdapter for UpsMibAdapter {
    fn read_sample(&self, client: &dyn SnmpClient) -> Result<UpsSample, UpsError> {
        let oids = self.oid_list();
        let values = client.get_many(&oids)?;
        let raw: BTreeMap<String, String> = oids.into_iter().zip(values).collect();

        let output_source = raw
            .get(&self.oids.output_source)
            .ok_or_else(|| UpsError::MissingValue(self.oids.output_source.clone()))?;
        let battery_status = raw
            .get(&self.oids.battery_status)
            .ok_or_else(|| UpsError::MissingValue(self.oids.battery_status.clone()))?;
        let charge = raw
            .get(&self.oids.battery_charge_percent)
            .and_then(|value| parse_u8(value));
        let runtime = self
            .oids
            .runtime_remaining_minutes
            .as_ref()
            .and_then(|oid| raw.get(oid))
            .and_then(|value| parse_u32(value));
        let seconds_on_battery = self
            .oids
            .seconds_on_battery
            .as_ref()
            .and_then(|oid| raw.get(oid))
            .and_then(|value| parse_u32(value));

        Ok(UpsSample {
            power_source: parse_power_source(output_source, &self.mapping),
            battery_charge_percent: charge,
            battery_health: parse_battery_health(battery_status, &self.mapping),
            seconds_on_battery,
            runtime_remaining_minutes: runtime,
            raw,
        })
    }
}

fn parse_power_source(value: &str, mapping: &UpsMapping) -> PowerSource {
    if matches_any(value, &mapping.on_battery_values) {
        PowerSource::Battery
    } else if matches_any(value, &mapping.line_values) {
        PowerSource::Line
    } else {
        PowerSource::Unknown
    }
}

fn parse_battery_health(value: &str, mapping: &UpsMapping) -> BatteryHealth {
    if matches_any(value, &mapping.depleted_battery_values) {
        BatteryHealth::Depleted
    } else if matches_any(value, &mapping.low_battery_values) {
        BatteryHealth::Low
    } else if parse_u32(value) == Some(2) || normalize(value) == "batterynormal" {
        BatteryHealth::Normal
    } else {
        BatteryHealth::Unknown
    }
}

fn matches_any(value: &str, candidates: &[String]) -> bool {
    let normalized = normalize(value);
    let numeric = parse_u32(value);

    candidates.iter().any(|candidate| {
        normalize(candidate) == normalized
            || numeric
                .zip(parse_u32(candidate))
                .map(|(left, right)| left == right)
                .unwrap_or(false)
    })
}

fn parse_u8(value: &str) -> Option<u8> {
    parse_u32(value).and_then(|number| u8::try_from(number).ok())
}

fn parse_u32(value: &str) -> Option<u32> {
    let value = value.trim();
    if let Ok(number) = value.parse::<u32>() {
        return Some(number);
    }

    let start = value.rfind('(')?;
    let end = value[start + 1..].find(')')? + start + 1;
    value[start + 1..end].trim().parse::<u32>().ok()
}

fn normalize(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snmp::SnmpClient;

    struct FakeSnmp {
        values: Vec<String>,
    }

    impl SnmpClient for FakeSnmp {
        fn get_many(&self, _oids: &[String]) -> Result<Vec<String>, SnmpError> {
            Ok(self.values.clone())
        }
    }

    #[test]
    fn parses_standard_ups_mib_battery_sample() {
        let adapter = UpsMibAdapter::new(UpsOids::default(), UpsMapping::default());
        let sample = adapter
            .read_sample(&FakeSnmp {
                values: vec![
                    "battery(5)".into(),
                    "25".into(),
                    "batteryLow(3)".into(),
                    "70".into(),
                    "7".into(),
                ],
            })
            .unwrap();

        assert_eq!(sample.power_source, PowerSource::Battery);
        assert_eq!(sample.battery_charge_percent, Some(25));
        assert_eq!(sample.battery_health, BatteryHealth::Low);
        assert_eq!(sample.seconds_on_battery, Some(70));
        assert_eq!(sample.runtime_remaining_minutes, Some(7));
    }

    #[test]
    fn parses_standard_ups_mib_line_sample() {
        let adapter = UpsMibAdapter::new(UpsOids::default(), UpsMapping::default());
        let sample = adapter
            .read_sample(&FakeSnmp {
                values: vec![
                    "3".into(),
                    "98".into(),
                    "2".into(),
                    "0".into(),
                    "120".into(),
                ],
            })
            .unwrap();

        assert_eq!(sample.power_source, PowerSource::Line);
        assert_eq!(sample.battery_charge_percent, Some(98));
        assert_eq!(sample.battery_health, BatteryHealth::Normal);
    }
}
