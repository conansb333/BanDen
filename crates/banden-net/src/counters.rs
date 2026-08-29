//! Per-interface traffic counters via `GetIfEntry2`.
//!
//! This is the always-available traffic source: it reports real totals for
//! the selected adapter without requiring Npcap.

use banden_core::traffic::Counters;
use windows::Win32::NetworkManagement::IpHelper::{GetIfEntry2, MIB_IF_ROW2};

/// Read cumulative counters for an interface index.
pub fn interface_counters(if_index: u32) -> Option<Counters> {
    unsafe {
        let mut row = MIB_IF_ROW2 {
            InterfaceIndex: if_index,
            ..Default::default()
        };
        if GetIfEntry2(&mut row).0 != 0 {
            return None;
        }
        Some(Counters {
            bytes_in: row.InOctets,
            bytes_out: row.OutOctets,
            packets_in: row.InUcastPkts.saturating_add(row.InNUcastPkts),
            packets_out: row.OutUcastPkts.saturating_add(row.OutNUcastPkts),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_for_invalid_index_is_none() {
        assert!(interface_counters(0xFFFF_FFFF).is_none());
    }
}
