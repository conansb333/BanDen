//! Traffic aggregation engine.
//!
//! Real-time traffic is aggregated in the backend and never streamed raw to
//! the UI. All buffers here are bounded: memory usage is a function of
//! window size, not of traffic volume.

use crate::models::{DeviceTraffic, ProtocolStat, TrafficSample, TrafficSnapshot};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, VecDeque};

/// Default number of realtime samples retained for the live chart
/// (at a 1s cadence this is ~5 minutes).
pub const DEFAULT_WINDOW_CAPACITY: usize = 300;

/// Default number of devices included in each snapshot.
pub const DEFAULT_TOP_DEVICES: usize = 8;

/// Rate computation from monotonic counter deltas. Counter resets
/// (adapter re-enumeration, reboot) yield zero instead of garbage.
pub fn compute_rates(delta_bytes_in: u64, delta_bytes_out: u64, elapsed_secs: f64) -> (f64, f64) {
    if elapsed_secs <= 0.0 {
        return (0.0, 0.0);
    }
    (
        delta_bytes_in as f64 * 8.0 / elapsed_secs,
        delta_bytes_out as f64 * 8.0 / elapsed_secs,
    )
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Counters {
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub packets_in: u64,
    pub packets_out: u64,
}

impl Counters {
    fn delta(&self, earlier: &Counters) -> Counters {
        Counters {
            bytes_in: self.bytes_in.saturating_sub(earlier.bytes_in),
            bytes_out: self.bytes_out.saturating_sub(earlier.bytes_out),
            packets_in: self.packets_in.saturating_sub(earlier.packets_in),
            packets_out: self.packets_out.saturating_sub(earlier.packets_out),
        }
    }
}

#[derive(Debug)]
struct DeviceAccumulator {
    mac: Option<String>,
    ip: Option<String>,
    label: String,
    current: Counters,
    previous: Option<Counters>,
    lifetime: Counters,
    last_seen: DateTime<Utc>,
}

impl DeviceAccumulator {
    fn traffic(&self, elapsed_secs: f64) -> DeviceTraffic {
        let (down, up) = match self.previous {
            Some(p) => {
                let d = self.current.delta(&p);
                compute_rates(d.bytes_in, d.bytes_out, elapsed_secs)
            }
            None => (0.0, 0.0),
        };
        DeviceTraffic {
            mac_address: self.mac.clone(),
            ip_address: self.ip.clone(),
            label: self.label.clone(),
            bytes_in: self.lifetime.bytes_in,
            bytes_out: self.lifetime.bytes_out,
            packets_in: self.lifetime.packets_in,
            packets_out: self.lifetime.packets_out,
            download_rate_bps: down,
            upload_rate_bps: up,
        }
    }
}

/// Aggregates flow/counter observations into snapshots for the UI and
/// the persistence layer.
pub struct TrafficAggregator {
    window: VecDeque<TrafficSample>,
    window_capacity: usize,
    top_devices: usize,
    total: DeviceAccumulator,
    devices: HashMap<String, DeviceAccumulator>,
    protocols: HashMap<String, (u64, u64)>, // protocol -> (bytes, packets)
    last_tick: Option<DateTime<Utc>>,
}

impl TrafficAggregator {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_WINDOW_CAPACITY, DEFAULT_TOP_DEVICES)
    }

    pub fn with_capacity(window_capacity: usize, top_devices: usize) -> Self {
        Self {
            window: VecDeque::with_capacity(window_capacity.min(1024)),
            window_capacity: window_capacity.max(2),
            top_devices: top_devices.max(1),
            total: DeviceAccumulator {
                mac: None,
                ip: None,
                label: "Total".into(),
                current: Counters::default(),
                previous: None,
                lifetime: Counters::default(),
                last_seen: Utc::now(),
            },
            devices: HashMap::new(),
            protocols: HashMap::new(),
            last_tick: None,
        }
    }

    /// Push a new total-counter observation (typically once per second).
    pub fn push_total(&mut self, counters: Counters, at: DateTime<Utc>) {
        let elapsed = self
            .last_tick
            .map(|t| (at - t).num_milliseconds() as f64 / 1000.0)
            .unwrap_or(1.0);
        self.last_tick = Some(at);

        self.total.previous = Some(self.total.current);
        self.total.current = counters;
        self.total.lifetime = counters;
        self.total.last_seen = at;

        let delta = self.total.current.delta(&self.total.previous.unwrap());
        let (down, up) = compute_rates(delta.bytes_in, delta.bytes_out, elapsed);

        let sample = TrafficSample {
            timestamp: at,
            bytes_in: delta.bytes_in,
            bytes_out: delta.bytes_out,
            packets_in: delta.packets_in,
            packets_out: delta.packets_out,
            download_rate_bps: down,
            upload_rate_bps: up,
        };
        self.window.push_back(sample);
        if self.window.len() > self.window_capacity {
            self.window.pop_front();
        }
    }

    /// Push per-device flow deltas (from a capture source).
    pub fn push_device(
        &mut self,
        mac: Option<String>,
        ip: Option<String>,
        label: String,
        delta: Counters,
        at: DateTime<Utc>,
    ) {
        let key = mac
            .clone()
            .or_else(|| ip.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let acc = self
            .devices
            .entry(key)
            .or_insert_with(|| DeviceAccumulator {
                mac: mac.clone(),
                ip: ip.clone(),
                label: label.clone(),
                current: Counters::default(),
                previous: None,
                lifetime: Counters::default(),
                last_seen: at,
            });
        acc.previous = Some(acc.current);
        acc.current = Counters {
            bytes_in: acc.current.bytes_in + delta.bytes_in,
            bytes_out: acc.current.bytes_out + delta.bytes_out,
            packets_in: acc.current.packets_in + delta.packets_in,
            packets_out: acc.current.packets_out + delta.packets_out,
        };
        acc.lifetime = acc.current;
        acc.last_seen = at;
        if let (Some(m), Some(i)) = (&mac, &ip) {
            acc.mac = Some(m.clone());
            acc.ip = Some(i.clone());
        }
        if !label.is_empty() {
            acc.label = label;
        }
    }

    /// Record protocol attribution for observed bytes.
    pub fn push_protocol(&mut self, protocol: &str, bytes: u64, packets: u64) {
        let e = self.protocols.entry(protocol.to_string()).or_insert((0, 0));
        e.0 = e.0.saturating_add(bytes);
        e.1 = e.1.saturating_add(packets);
    }

    /// Build the bounded snapshot for the UI.
    pub fn snapshot(&self) -> TrafficSnapshot {
        let elapsed = 1.0; // rates are pre-computed per tick
        let mut top: Vec<DeviceTraffic> =
            self.devices.values().map(|a| a.traffic(elapsed)).collect();
        top.sort_by(|a, b| {
            let a_total = a.download_rate_bps + a.upload_rate_bps;
            let b_total = b.download_rate_bps + b.upload_rate_bps;
            b_total
                .partial_cmp(&a_total)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        top.truncate(self.top_devices);

        let mut protocols: Vec<ProtocolStat> = self
            .protocols
            .iter()
            .map(|(p, (b, pk))| ProtocolStat {
                protocol: p.clone(),
                bytes: *b,
                packets: *pk,
            })
            .collect();
        protocols.sort_by_key(|p| std::cmp::Reverse(p.bytes));

        TrafficSnapshot {
            timestamp: self.total.last_seen,
            total: self.total.traffic(elapsed),
            history: self.window.iter().copied().collect(),
            top_devices: top,
            protocols,
        }
    }

    /// Drop device accumulators not seen within the timeout so stale
    /// entries do not linger forever.
    pub fn prune_devices(&mut self, older_than: chrono::Duration) {
        let cutoff = Utc::now() - older_than;
        self.devices.retain(|_, a| a.last_seen >= cutoff);
    }

    pub fn window_len(&self) -> usize {
        self.window.len()
    }
}

impl Default for TrafficAggregator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn rates_from_counter_deltas() {
        let (down, up) = compute_rates(1_000_000, 250_000, 1.0);
        assert!((down - 8_000_000.0).abs() < 0.01);
        assert!((up - 2_000_000.0).abs() < 0.01);
        // Zero elapsed time must never divide by zero.
        assert_eq!(compute_rates(1, 1, 0.0), (0.0, 0.0));
    }

    #[test]
    fn counter_reset_yields_zero_rate_not_garbage() {
        let mut agg = TrafficAggregator::with_capacity(10, 5);
        agg.push_total(
            Counters {
                bytes_in: 5_000_000,
                bytes_out: 1_000_000,
                packets_in: 5,
                packets_out: 5,
            },
            t(0),
        );
        agg.push_total(
            Counters {
                bytes_in: 100,
                bytes_out: 50,
                packets_in: 1,
                packets_out: 1,
            },
            t(1),
        ); // reset
        let snap = agg.snapshot();
        assert_eq!(snap.total.download_rate_bps, 0.0);
        assert_eq!(snap.total.upload_rate_bps, 0.0);
    }

    #[test]
    fn window_is_bounded() {
        let mut agg = TrafficAggregator::with_capacity(4, 5);
        for i in 0..10u64 {
            agg.push_total(
                Counters {
                    bytes_in: i * 100,
                    bytes_out: i,
                    packets_in: i,
                    packets_out: i,
                },
                t(i as i64),
            );
        }
        assert_eq!(agg.window_len(), 4);
        let snap = agg.snapshot();
        assert_eq!(snap.history.len(), 4);
        // Newest sample is last.
        assert_eq!(snap.history.last().unwrap().bytes_in, 900 - 800);
    }

    #[test]
    fn device_aggregation_and_top_ordering() {
        let mut agg = TrafficAggregator::with_capacity(10, 2);
        agg.push_device(
            Some("AA:AA:AA:AA:AA:01".into()),
            Some("192.168.1.10".into()),
            "Laptop".into(),
            Counters {
                bytes_in: 10_000_000,
                bytes_out: 0,
                packets_in: 10,
                packets_out: 0,
            },
            t(0),
        );
        agg.push_device(
            Some("AA:AA:AA:AA:AA:02".into()),
            Some("192.168.1.11".into()),
            "Phone".into(),
            Counters {
                bytes_in: 1_000_000,
                bytes_out: 0,
                packets_in: 1,
                packets_out: 0,
            },
            t(0),
        );
        agg.push_device(
            Some("AA:AA:AA:AA:AA:01".into()),
            Some("192.168.1.10".into()),
            "Laptop".into(),
            Counters {
                bytes_in: 5_000_000,
                bytes_out: 0,
                packets_in: 5,
                packets_out: 0,
            },
            t(1),
        );
        let snap = agg.snapshot();
        assert_eq!(snap.top_devices.len(), 2);
        assert_eq!(snap.top_devices[0].label, "Laptop");
        assert_eq!(snap.top_devices[0].bytes_in, 15_000_000);
        assert_eq!(snap.top_devices[1].label, "Phone");
    }

    #[test]
    fn protocol_stats_sorted_by_bytes() {
        let mut agg = TrafficAggregator::new();
        agg.push_protocol("TCP", 900, 9);
        agg.push_protocol("UDP", 100, 1);
        agg.push_protocol("QUIC", 5_000, 50);
        let snap = agg.snapshot();
        assert_eq!(snap.protocols[0].protocol, "QUIC");
        assert_eq!(snap.protocols[2].protocol, "UDP");
    }

    #[test]
    fn snapshot_total_rates_follow_deltas() {
        let mut agg = TrafficAggregator::with_capacity(10, 5);
        agg.push_total(
            Counters {
                bytes_in: 0,
                bytes_out: 0,
                packets_in: 0,
                packets_out: 0,
            },
            t(0),
        );
        agg.push_total(
            Counters {
                bytes_in: 125_000,
                bytes_out: 25_000,
                packets_in: 100,
                packets_out: 40,
            },
            t(1),
        );
        let snap = agg.snapshot();
        assert!((snap.total.download_rate_bps - 1_000_000.0).abs() < 0.01);
        assert!((snap.total.upload_rate_bps - 200_000.0).abs() < 0.01);
    }
}
