//! Traffic monitoring.
//!
//! A `TrafficMonitor` samples the selected interface's counters once per
//! second, feeds the core `TrafficAggregator`, optionally simulates
//! per-device flows when lab mode is on (no Npcap required), persists a
//! history sample periodically and emits snapshots to a callback.
//!
//! Raw packets are never forwarded to the UI: everything the frontend sees
//! comes from the bounded snapshot.

use banden_core::models::TrafficSnapshot;
use banden_core::traffic::{Counters, TrafficAggregator};
use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// Callback types the app layer registers.
pub trait TrafficHooks: Send + Sync {
    /// Called (roughly) once per second with the fresh snapshot.
    fn on_snapshot(&self, snapshot: &TrafficSnapshot);
    /// Called once per persistence interval with a total sample.
    fn on_persist(&self, snapshot: &TrafficSnapshot);
}

/// No-op hooks for tests.
pub struct NoopHooks;
impl TrafficHooks for NoopHooks {
    fn on_snapshot(&self, _: &TrafficSnapshot) {}
    fn on_persist(&self, _: &TrafficSnapshot) {}
}

pub struct TrafficMonitor {
    aggregator: Arc<Mutex<TrafficAggregator>>,
    stop: CancellationToken,
}

/// Description of how per-device visibility is produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// Real totals from interface counters; per-device flows simulated.
    CountersPlusSimulation,
    /// Real totals only (no per-device data).
    CountersOnly,
}

pub struct TrafficMonitorConfig {
    pub if_index: u32,
    pub mode: CaptureMode,
    /// Snapshot cadence.
    pub sample_interval: Duration,
    /// Persistence cadence.
    pub persist_interval: Duration,
    /// Devices used by the simulation source (lab mode).
    pub simulated_devices: Vec<(String, String)>, // (mac, label)
    /// Network base of the selected subnet; simulated device IPs are
    /// assigned inside it so the lab view matches the real network.
    pub simulated_ip_base: Option<std::net::Ipv4Addr>,
}

impl Default for TrafficMonitorConfig {
    fn default() -> Self {
        Self {
            if_index: 0,
            mode: CaptureMode::CountersPlusSimulation,
            sample_interval: Duration::from_secs(1),
            persist_interval: Duration::from_secs(10),
            simulated_devices: Vec::new(),
            simulated_ip_base: None,
        }
    }
}

impl TrafficMonitor {
    pub fn new(config: TrafficMonitorConfig, hooks: Arc<dyn TrafficHooks>) -> Arc<Self> {
        let monitor = Arc::new(Self {
            aggregator: Arc::new(Mutex::new(TrafficAggregator::new())),
            stop: CancellationToken::new(),
        });
        let agg = monitor.aggregator.clone();
        let stop = monitor.stop.clone();
        let mut persist_countdown =
            config.persist_interval.as_secs_f64() / config.sample_interval.as_secs_f64().max(0.001);
        let mut sim =
            SimulationFlowSource::new(&config.simulated_devices, config.simulated_ip_base);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(config.sample_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = stop.cancelled() => break,
                    _ = ticker.tick() => {}
                }
                let now = Utc::now();
                let mut agg = agg.lock().await;
                if let Some(counters) = crate::counters::interface_counters(config.if_index) {
                    agg.push_total(counters, now);
                }
                match config.mode {
                    CaptureMode::CountersPlusSimulation => {
                        for (mac, ip, label, delta, protocol) in sim.step() {
                            agg.push_device(Some(mac), Some(ip), label, delta, now);
                            agg.push_protocol(
                                &protocol,
                                delta.bytes_in + delta.bytes_out,
                                delta.packets_in + delta.packets_out,
                            );
                        }
                    }
                    CaptureMode::CountersOnly => {}
                }
                let snapshot = agg.snapshot();
                drop(agg);
                hooks.on_snapshot(&snapshot);
                persist_countdown -= 1.0;
                if persist_countdown <= 0.0 {
                    hooks.on_persist(&snapshot);
                    persist_countdown = config.persist_interval.as_secs_f64()
                        / config.sample_interval.as_secs_f64().max(0.001);
                }
            }
        });
        monitor
    }

    pub fn stop(&self) {
        self.stop.cancel();
    }

    /// Latest snapshot on demand.
    pub async fn snapshot(&self) -> TrafficSnapshot {
        self.aggregator.lock().await.snapshot()
    }
}

/// Deterministic-ish simulated per-device flows for lab mode. Produces
/// plausible, bounded traffic so the UI, persistence and analytics can be
/// exercised without packet capture hardware.
pub struct SimulationFlowSource {
    devices: Vec<SimDevice>,
    tick: u64,
}

struct SimDevice {
    mac: String,
    ip: String,
    label: String,
    state: u64,
}

impl SimulationFlowSource {
    pub fn new(devices: &[(String, String)], ip_base: Option<std::net::Ipv4Addr>) -> Self {
        let base = u32::from(ip_base.unwrap_or(std::net::Ipv4Addr::new(192, 168, 1, 0)));
        Self {
            devices: devices
                .iter()
                .enumerate()
                .map(|(i, (mac, label))| SimDevice {
                    mac: mac.clone(),
                    ip: std::net::Ipv4Addr::from(base + 20 + i as u32).to_string(),
                    label: label.clone(),
                    state: (i as u64).wrapping_mul(7919).wrapping_add(13),
                })
                .collect(),
            tick: 0,
        }
    }

    /// Produce one second of simulated per-device deltas.
    pub fn step(&mut self) -> Vec<(String, String, String, Counters, String)> {
        self.tick += 1;
        self.devices
            .iter_mut()
            .map(|d| {
                d.state = d
                    .state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let r = (d.state >> 33) % 100;
                let down_bytes = 5_000 + r * 40_000; // 40 KB .. ~4 MB per tick
                let up_bytes = 1_000 + (r % 20) * 8_000;
                let protocol = if r % 3 == 0 {
                    "UDP"
                } else if r % 3 == 1 {
                    "TCP"
                } else {
                    "QUIC"
                };
                (
                    d.mac.clone(),
                    d.ip.clone(),
                    d.label.clone(),
                    Counters {
                        bytes_in: down_bytes,
                        bytes_out: up_bytes,
                        packets_in: 1 + r / 10,
                        packets_out: 1 + r / 25,
                    },
                    protocol.to_string(),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulated_flows_are_bounded_and_labeled() {
        let mut sim = SimulationFlowSource::new(
            &[
                ("AA:AA:AA:AA:AA:01".into(), "Laptop".into()),
                ("AA:AA:AA:AA:AA:02".into(), "Phone".into()),
            ],
            None,
        );
        let mut max_down = 0;
        for _ in 0..100 {
            for (mac, _ip, label, delta, protocol) in sim.step() {
                assert!(mac.starts_with("AA:AA:AA"));
                assert!(!label.is_empty());
                assert!(delta.bytes_in <= 5_000_000);
                assert!(matches!(protocol.as_str(), "TCP" | "UDP" | "QUIC"));
                max_down = max_down.max(delta.bytes_in);
            }
        }
        assert!(max_down > 0);
    }

    #[test]
    fn simulated_flows_vary_over_time() {
        let mut sim =
            SimulationFlowSource::new(&[("AA:AA:AA:AA:AA:01".into(), "Laptop".into())], None);
        let first = sim.step()[0].3.bytes_in;
        let mut different = false;
        for _ in 0..10 {
            if sim.step()[0].3.bytes_in != first {
                different = true;
            }
        }
        assert!(different);
    }
}
