//! BanDen network layer (Windows).
//!
//! All hardware access lives here: adapter enumeration, the ARP subsystem,
//! LAN discovery, gateway latency, interface counters and the real
//! ARP-isolation control backend. Nothing in this crate touches UI state;
//! results are plain data structures.
//!
//! Win32 FFI is synchronous by nature — async entry points wrap the
//! blocking work in `spawn_blocking` so callers never stall the UI
//! runtime.

pub mod appcatalog;
pub mod arp;
pub mod control;
pub mod counters;
pub mod discovery;
pub mod dpi;
pub mod hostname;
pub mod interfaces;
pub mod latency;
pub mod oui;
pub mod rawframe;
pub mod traffic;

pub use arp::{read_arp_table, ArpEntry, NeighborState};
pub use control::ArpCutBackend;
pub use discovery::{DiscoveredDevice, DiscoveryReport, DiscoveryService};
pub use error::{NetError, NetResult};
pub use interfaces::{list_interfaces, select_interface};
pub use latency::ping;
pub use oui::{guess_device_type, is_randomized_mac, resolve_vendor, vendor_from_hostname};
pub use rawframe::PcapLib;
pub use traffic::{CaptureMode, TrafficHooks, TrafficMonitor, TrafficMonitorConfig};

mod error;

/// The interface that routes `target` (public helper for tooling/examples).
pub fn interface_for_target(
    interfaces: &[banden_core::InterfaceInfo],
    target: std::net::Ipv4Addr,
) -> Option<banden_core::InterfaceInfo> {
    control::interface_for_target(interfaces, target)
}

/// The wpcap device name for the interface that routes `target`.
pub fn pcap_device_for_target(
    lib: &std::sync::Arc<PcapLib>,
    target: std::net::Ipv4Addr,
) -> NetResult<String> {
    control::resolve_device(lib, target).map(|(name, _)| name)
}

/// Restoration executor backed by real network operations. Used by the
/// recovery manager and the watchdog.
pub struct NetRestorationExecutor;

#[async_trait::async_trait]
impl banden_core::RestorationExecutor for NetRestorationExecutor {
    async fn execute(&self, action: &banden_core::RestorationAction) -> Result<(), String> {
        use banden_core::RestorationAction::*;
        match action {
            NoOp { .. } => Ok(()),
            ClearNeighborEntry { ip } => {
                let ip: std::net::Ipv4Addr = ip.parse().map_err(|e| format!("bad ip: {e}"))?;
                tokio::task::spawn_blocking(move || arp::delete_neighbor(ip))
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())
            }
            RestoreNeighborEntry { ip, mac } => {
                // Broadcast gratuitous ARP announcing `ip -> mac` so any
                // host whose cache we poisoned re-learns the true mapping.
                let lib = PcapLib::load().map_err(|e| e.to_string())?;
                let ip: std::net::Ipv4Addr = ip.parse().map_err(|e| format!("bad ip: {e}"))?;
                let mac = rawframe::parse_mac(mac).map_err(|e| e.to_string())?;
                tokio::task::spawn_blocking(move || ArpCutBackend::announce(&lib, ip, mac))
                    .await
                    .map_err(|e| format!("join error: {e}"))?
                    .map_err(|e| e.to_string())
            }
            StopProcess { name } => Err(format!("stop process {name}: not implemented")),
        }
    }
}
