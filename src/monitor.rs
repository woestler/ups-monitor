use crate::{
    config::MonitorConfig,
    shutdown::{ShutdownError, ShutdownExecutor},
    snmp::SnmpClient,
    ups::{BatteryHealth, PowerSource, UpsAdapter, UpsError, UpsSample},
};
use log::{debug, error, info, warn};
use std::{
    thread,
    time::{Duration, Instant},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MonitorError {
    #[error(transparent)]
    Ups(#[from] UpsError),
    #[error(transparent)]
    Shutdown(#[from] ShutdownError),
}

#[derive(Debug, Clone)]
pub struct MonitorState {
    on_battery_since: Option<Duration>,
    last_power_source: Option<PowerSource>,
    last_battery_health: Option<BatteryHealth>,
    shutdown_requested: bool,
}

impl MonitorState {
    pub fn new() -> Self {
        Self {
            on_battery_since: None,
            last_power_source: None,
            last_battery_health: None,
            shutdown_requested: false,
        }
    }

    pub fn evaluate(
        &mut self,
        sample: &UpsSample,
        now: Duration,
        config: &MonitorConfig,
    ) -> MonitorDecision {
        let mut events = Vec::new();

        if self.last_power_source != Some(sample.power_source) {
            events.push(MonitorEvent::PowerSourceChanged {
                from: self.last_power_source,
                to: sample.power_source,
            });
            self.last_power_source = Some(sample.power_source);
        }

        if self.last_battery_health != Some(sample.battery_health) {
            events.push(MonitorEvent::BatteryHealthChanged {
                from: self.last_battery_health,
                to: sample.battery_health,
            });
            self.last_battery_health = Some(sample.battery_health);
        }

        match sample.power_source {
            PowerSource::Battery => {
                let started_at = *self.on_battery_since.get_or_insert_with(|| {
                    events.push(MonitorEvent::OnBatteryStarted);
                    now
                });

                if self.shutdown_requested {
                    return MonitorDecision {
                        events,
                        shutdown: None,
                    };
                }

                if config.trigger.checks_capacity() {
                    if let Some(percent) = sample.battery_charge_percent {
                        if percent <= config.low_battery_percent {
                            self.shutdown_requested = true;
                            return MonitorDecision {
                                events,
                                shutdown: Some(ShutdownRequest {
                                    reason: ShutdownReason::LowBattery {
                                        percent,
                                        threshold: config.low_battery_percent,
                                    },
                                }),
                            };
                        }
                    }

                    if matches!(
                        sample.battery_health,
                        BatteryHealth::Low | BatteryHealth::Depleted
                    ) {
                        self.shutdown_requested = true;
                        return MonitorDecision {
                            events,
                            shutdown: Some(ShutdownRequest {
                                reason: ShutdownReason::UpsReportedLowBattery {
                                    health: sample.battery_health,
                                },
                            }),
                        };
                    }
                }

                if config.trigger.checks_duration() {
                    let elapsed = now.saturating_sub(started_at);
                    if elapsed >= config.max_on_battery {
                        self.shutdown_requested = true;
                        return MonitorDecision {
                            events,
                            shutdown: Some(ShutdownRequest {
                                reason: ShutdownReason::OnBatteryTooLong {
                                    elapsed,
                                    limit: config.max_on_battery,
                                },
                            }),
                        };
                    }
                }
            }
            PowerSource::Line => {
                if self.on_battery_since.take().is_some() {
                    events.push(MonitorEvent::OnBatteryCanceled);
                }
                self.shutdown_requested = false;
            }
            PowerSource::Unknown => {}
        }

        MonitorDecision {
            events,
            shutdown: None,
        }
    }
}

impl Default for MonitorState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorDecision {
    pub events: Vec<MonitorEvent>,
    pub shutdown: Option<ShutdownRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitorEvent {
    PowerSourceChanged {
        from: Option<PowerSource>,
        to: PowerSource,
    },
    BatteryHealthChanged {
        from: Option<BatteryHealth>,
        to: BatteryHealth,
    },
    OnBatteryStarted,
    OnBatteryCanceled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownRequest {
    pub reason: ShutdownReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShutdownReason {
    LowBattery { percent: u8, threshold: u8 },
    UpsReportedLowBattery { health: BatteryHealth },
    OnBatteryTooLong { elapsed: Duration, limit: Duration },
}

impl ShutdownReason {
    pub fn message(&self) -> String {
        match self {
            ShutdownReason::LowBattery { percent, threshold } => {
                format!("battery charge {percent}% <= configured threshold {threshold}%")
            }
            ShutdownReason::UpsReportedLowBattery { health } => {
                format!("UPS reported battery health {health:?}")
            }
            ShutdownReason::OnBatteryTooLong { elapsed, limit } => {
                format!(
                    "on battery for {}s >= configured limit {}s",
                    elapsed.as_secs(),
                    limit.as_secs()
                )
            }
        }
    }
}

pub fn run_monitor_loop(
    config: MonitorConfig,
    adapter: &dyn UpsAdapter,
    snmp: &dyn SnmpClient,
    shutdown: &dyn ShutdownExecutor,
    once: bool,
) -> Result<(), MonitorError> {
    let mut state = MonitorState::new();
    let started_at = Instant::now();

    loop {
        let sample = match adapter.read_sample(snmp) {
            Ok(sample) => sample,
            Err(error) if once => return Err(error.into()),
            Err(error) => {
                let error = MonitorError::Ups(error);
                log_poll_error(&error);
                thread::sleep(config.poll_interval);
                continue;
            }
        };
        log_sample(&sample);
        let decision = state.evaluate(&sample, started_at.elapsed(), &config);
        log_events(&decision.events);

        if let Some(request) = decision.shutdown {
            warn!("shutdown condition met: {}", request.reason.message());
            shutdown.shutdown(&request.reason)?;
            if once {
                return Ok(());
            }
        }

        if once {
            return Ok(());
        }

        thread::sleep(config.poll_interval);
    }
}

fn log_sample(sample: &UpsSample) {
    debug!(
        "UPS sample: source={:?}, charge={:?}, health={:?}, runtime_remaining_minutes={:?}",
        sample.power_source,
        sample.battery_charge_percent,
        sample.battery_health,
        sample.runtime_remaining_minutes
    );
}

fn log_events(events: &[MonitorEvent]) {
    for event in events {
        match event {
            MonitorEvent::PowerSourceChanged { from, to } => {
                info!("UPS power source changed: {:?} -> {:?}", from, to);
            }
            MonitorEvent::BatteryHealthChanged { from, to } => {
                info!("UPS battery health changed: {:?} -> {:?}", from, to);
            }
            MonitorEvent::OnBatteryStarted => {
                warn!("UPS is on battery; shutdown countdown is active if duration trigger is enabled");
            }
            MonitorEvent::OnBatteryCanceled => {
                info!("utility power restored; shutdown countdown canceled");
            }
        }
    }
}

pub fn log_poll_error(error: &MonitorError) {
    error!("monitoring cycle failed: {error}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ShutdownTrigger;
    use std::collections::BTreeMap;

    fn sample(source: PowerSource, charge: Option<u8>, health: BatteryHealth) -> UpsSample {
        UpsSample {
            power_source: source,
            battery_charge_percent: charge,
            battery_health: health,
            runtime_remaining_minutes: None,
            raw: BTreeMap::new(),
        }
    }

    fn config(trigger: ShutdownTrigger) -> MonitorConfig {
        MonitorConfig {
            poll_interval: Duration::from_secs(10),
            trigger,
            low_battery_percent: 30,
            max_on_battery: Duration::from_secs(60),
        }
    }

    #[test]
    fn duration_trigger_waits_then_requests_shutdown() {
        let mut state = MonitorState::new();
        let config = config(ShutdownTrigger::OnBatteryDuration);

        let first = state.evaluate(
            &sample(PowerSource::Battery, Some(80), BatteryHealth::Normal),
            Duration::from_secs(0),
            &config,
        );
        assert!(first.shutdown.is_none());

        let second = state.evaluate(
            &sample(PowerSource::Battery, Some(80), BatteryHealth::Normal),
            Duration::from_secs(61),
            &config,
        );

        assert_eq!(
            second.shutdown.unwrap().reason,
            ShutdownReason::OnBatteryTooLong {
                elapsed: Duration::from_secs(61),
                limit: Duration::from_secs(60)
            }
        );
    }

    #[test]
    fn utility_restore_cancels_duration_countdown() {
        let mut state = MonitorState::new();
        let config = config(ShutdownTrigger::OnBatteryDuration);

        state.evaluate(
            &sample(PowerSource::Battery, Some(80), BatteryHealth::Normal),
            Duration::from_secs(0),
            &config,
        );
        let restored = state.evaluate(
            &sample(PowerSource::Line, Some(80), BatteryHealth::Normal),
            Duration::from_secs(30),
            &config,
        );
        let later = state.evaluate(
            &sample(PowerSource::Battery, Some(80), BatteryHealth::Normal),
            Duration::from_secs(45),
            &config,
        );

        assert!(restored.events.contains(&MonitorEvent::OnBatteryCanceled));
        assert!(later.shutdown.is_none());
    }

    #[test]
    fn capacity_trigger_requests_shutdown_immediately() {
        let mut state = MonitorState::new();
        let config = config(ShutdownTrigger::BatteryCapacity);

        let decision = state.evaluate(
            &sample(PowerSource::Battery, Some(29), BatteryHealth::Normal),
            Duration::from_secs(1),
            &config,
        );

        assert_eq!(
            decision.shutdown.unwrap().reason,
            ShutdownReason::LowBattery {
                percent: 29,
                threshold: 30
            }
        );
    }

    #[test]
    fn line_power_never_triggers_shutdown_even_with_low_charge() {
        let mut state = MonitorState::new();
        let config = config(ShutdownTrigger::Either);

        let decision = state.evaluate(
            &sample(PowerSource::Line, Some(5), BatteryHealth::Low),
            Duration::from_secs(1),
            &config,
        );

        assert!(decision.shutdown.is_none());
    }
}
