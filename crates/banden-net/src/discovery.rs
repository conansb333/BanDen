//! LAN device discovery.
//!
//! Combines three sources:
//! 1. the system ARP/neighbor table (instant, passive),
//! 2. a `SendARP` sweep of the local subnet (active, bounded),
//! 3. reverse DNS + OUI enrichment (best-effort, per host).
//!
//! Hardware-touching work runs on blocking threads; the service API is
//! async and safe to call from the UI runtime.

use crate::arp::{probe_arp, read_arp_table, ArpEntry};
use crate::error::{NetError, NetResult};
use crate::hostname::resolve_hostname;
use crate::interfaces::list_interfaces;
use crate::oui::{guess_device_type, lookup_vendor, resolve_vendor};
use banden_core::InterfaceInfo;
use std::collections::HashMap;
use std::net::Ipv4Addr;

/// Maximum addresses probed in an active sweep. Larger subnets fall back
/// to ARP-table-only discovery to keep scans bounded.
pub const MAX_SWEEP_HOSTS: usize = 1024;

/// Parallelism of the SendARP sweep.
const SWEEP_WORKERS: usize = 32;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DiscoveredDevice {
    pub ip: Ipv4Addr,
    pub mac: String,
    pub hostname: Option<String>,
    pub vendor: Option<String>,
    pub device_type: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DiscoveryReport {
    pub interface: String,
    pub devices: Vec<DiscoveredDevice>,
    /// True when the active sweep was skipped (subnet too large).
    pub sweep_skipped: bool,
    pub duration_ms: u128,
}

/// Enumerate subnet hosts from an interface CIDR. Pure; tested without
/// hardware. Returns None when the interface has no usable IPv4 subnet.
pub fn subnet_hosts(interface: &InterfaceInfo) -> Option<Vec<Ipv4Addr>> {
    let ip: Ipv4Addr = interface.ipv4.as_deref()?.parse().ok()?;
    let prefix = interface.prefix_len?;
    if !(8..=30).contains(&prefix) {
        return None; // degenerate or point-to-point; not sweepable
    }
    let base = u32::from(ip) & mask_of(prefix);
    let count: u32 = 1u32 << (32 - prefix);
    let mut hosts = Vec::with_capacity(count as usize);
    let network = base;
    let broadcast = base + count - 1;
    for raw in network..=broadcast {
        // Skip network and broadcast addresses for small subnets.
        if prefix < 31 && (raw == network || raw == broadcast) {
            continue;
        }
        hosts.push(Ipv4Addr::from(raw));
    }
    Some(hosts)
}

fn mask_of(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix as u32)
    }
}

/// The discovery service. Construct once; call `discover` per cycle.
pub struct DiscoveryService;

impl DiscoveryService {
    /// Run one discovery cycle for the given interface. Blocking FFI is
    /// wrapped so callers can `.await` from an async context via
    /// `tokio::task::spawn_blocking`.
    pub async fn discover(interface: InterfaceInfo) -> NetResult<DiscoveryReport> {
        let if_clone = interface.clone();
        tokio::task::spawn_blocking(move || discover_blocking(if_clone))
            .await
            .map_err(|e| NetError::Io(std::io::Error::other(e)))?
    }
}

/// Synchronous discovery; also the seam used by tests with fake tables.
pub fn discover_blocking(interface: InterfaceInfo) -> NetResult<DiscoveryReport> {
    let started = std::time::Instant::now();
    let mut by_mac: HashMap<String, DiscoveredDevice> = HashMap::new();

    // 1. Passive: ARP table entries on this interface's subnet.
    let table: Vec<ArpEntry> = read_arp_table().unwrap_or_default();
    let local = subnet_hosts(&interface);
    for entry in table {
        if let Some(hosts) = &local {
            if !hosts.contains(&entry.ip) {
                // Entry belongs to another interface; keep it anyway if the
                // interface could not be resolved, drop otherwise.
                continue;
            }
        }
        by_mac.entry(entry.mac.clone()).or_insert(DiscoveredDevice {
            ip: entry.ip,
            mac: entry.mac.clone(),
            hostname: None,
            vendor: lookup_vendor(&entry.mac).map(|s| s.to_string()),
            device_type: None,
        });
    }

    // 2. Active sweep, bounded.
    let mut sweep_skipped = false;
    if let Some(hosts) = subnet_hosts(&interface) {
        if hosts.len() > MAX_SWEEP_HOSTS {
            sweep_skipped = true;
        } else {
            for (mac, ip) in sweep_subnet(&hosts) {
                by_mac.entry(mac.clone()).or_insert(DiscoveredDevice {
                    ip,
                    mac,
                    hostname: None,
                    vendor: None,
                    device_type: None,
                });
            }
        }
    }

    // 3. Enrichment: multi-tier vendor + device-type pipeline.
    let mut devices: Vec<DiscoveredDevice> = by_mac.into_values().collect();
    for d in &mut devices {
        d.hostname = resolve_hostname(d.ip);
        let res = resolve_vendor(&d.mac, d.hostname.as_deref());
        d.vendor = res.vendor;
        d.device_type = guess_device_type(&d.mac, d.hostname.as_deref()).map(|s| s.to_string());
    }
    devices.sort_by_key(|d| u32::from(d.ip));

    Ok(DiscoveryReport {
        interface: interface
            .friendly_name
            .clone()
            .unwrap_or_else(|| interface.name.clone()),
        devices,
        sweep_skipped,
        duration_ms: started.elapsed().as_millis(),
    })
}

/// Probe all hosts in parallel using scoped threads.
fn sweep_subnet(hosts: &[Ipv4Addr]) -> Vec<(String, Ipv4Addr)> {
    let queue = std::sync::Mutex::new(hosts.to_vec());
    let found = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..SWEEP_WORKERS.min(hosts.len().max(1)) {
            scope.spawn(|| loop {
                let next = {
                    let mut q = queue.lock().unwrap();
                    q.pop()
                };
                let Some(ip) = next else { break };
                if let Some(mac) = probe_arp(ip) {
                    found.lock().unwrap().push((mac, ip));
                }
            });
        }
    });
    found.into_inner().unwrap()
}

/// Convenience: discover against the automatically selected interface.
pub async fn discover_auto(preferred: Option<&str>) -> NetResult<DiscoveryReport> {
    let interfaces = list_interfaces()?;
    let Some(selected) = crate::interfaces::select_interface(&interfaces, preferred) else {
        return Err(NetError::InterfaceNotFound(
            preferred.unwrap_or("<auto>").to_string(),
        ));
    };
    DiscoveryService::discover(selected).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(ip: &str, prefix: u8) -> InterfaceInfo {
        InterfaceInfo {
            id: "test-if".into(),
            if_index: Some(7),
            name: "test".into(),
            friendly_name: Some("Test NIC".into()),
            mac_address: Some("AA:BB:CC:00:00:01".into()),
            ipv4: Some(ip.into()),
            prefix_len: Some(prefix),
            cidr: Some(format!("192.168.1.0/{prefix}")),
            gateway: Some("192.168.1.1".into()),
            is_up: true,
            is_loopback: false,
            is_physical: true,
        }
    }

    #[test]
    fn subnet_hosts_24() {
        let hosts = subnet_hosts(&iface("192.168.1.130", 24)).unwrap();
        assert_eq!(hosts.len(), 254);
        assert_eq!(hosts.first().unwrap().to_string(), "192.168.1.1");
        assert_eq!(hosts.last().unwrap().to_string(), "192.168.1.254");
    }

    #[test]
    fn subnet_hosts_30() {
        // 10.0.0.5/30 covers 10.0.0.4 (network) .. 10.0.0.7 (broadcast);
        // usable hosts are .5 and .6.
        let hosts = subnet_hosts(&iface("10.0.0.5", 30)).unwrap();
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].to_string(), "10.0.0.5");
        assert_eq!(hosts[1].to_string(), "10.0.0.6");
    }

    #[test]
    fn subnet_hosts_rejects_degenerate() {
        assert!(subnet_hosts(&iface("0.0.0.0", 0)).is_none());
        assert!(subnet_hosts(&iface("1.2.3.4", 31)).is_none());
        let big = iface("10.0.0.1", 8);
        assert_eq!(subnet_hosts(&big).unwrap().len(), 16_777_214);
    }

    #[test]
    fn sweep_bounds_respected() {
        // /22 = 1022 hosts <= MAX_SWEEP_HOSTS
        assert!(subnet_hosts(&iface("10.1.4.1", 22)).unwrap().len() <= MAX_SWEEP_HOSTS);
        // /21 = 2046 hosts > MAX_SWEEP_HOSTS
        assert!(subnet_hosts(&iface("10.1.8.1", 21)).unwrap().len() > MAX_SWEEP_HOSTS);
    }

    #[tokio::test]
    async fn discovery_runs_on_loopback_machine() {
        // Machine-dependent smoke test; must not panic or error.
        let interfaces = list_interfaces().unwrap();
        let Some(sel) = crate::interfaces::select_interface(&interfaces, None) else {
            return; // no network in CI container; fine
        };
        let report = DiscoveryService::discover(sel).await.unwrap();
        assert!(!report.interface.is_empty());
    }
}
