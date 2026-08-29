//! ARP subsystem: neighbor-table inspection and `SendARP` probes.
//!
//! This module is intentionally isolated: packet/FFI details never leak
//! into session management. Parsing and normalization are pure functions
//! covered by tests independent of live hardware.

use crate::error::{NetError, NetResult};
use std::net::Ipv4Addr;
use windows::Win32::NetworkManagement::IpHelper::{
    DeleteIpNetEntry2, FreeMibTable, GetIpNetTable2, SendARP, MIB_IPNET_ROW2, MIB_IPNET_TABLE2,
};
use windows::Win32::Networking::WinSock::AF_INET;

/// One neighbor (ARP) entry.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ArpEntry {
    pub ip: Ipv4Addr,
    pub mac: String,
    pub interface_index: u32,
    pub state: NeighborState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeighborState {
    Reachable,
    Stale,
    Delay,
    Probe,
    Permanent,
    Incomplete,
    Unreachable,
    Other,
}

/// Snapshot of the system IPv4 neighbor table.
pub fn read_arp_table() -> NetResult<Vec<ArpEntry>> {
    unsafe {
        let mut table: *mut MIB_IPNET_TABLE2 = std::ptr::null_mut();
        let rc = GetIpNetTable2(AF_INET, &mut table);
        if rc.0 != 0 {
            return Err(NetError::ArpQuery(rc.0));
        }
        let mut out = Vec::new();
        if !table.is_null() {
            let header = &*table;
            for i in 0..header.NumEntries as usize {
                let row = &*(header.Table.as_ptr().add(i));
                if let Some(entry) = row_to_entry(row) {
                    out.push(entry);
                }
            }
            FreeMibTable(table as *const core::ffi::c_void);
        }
        Ok(out)
    }
}

unsafe fn row_to_entry(row: &MIB_IPNET_ROW2) -> Option<ArpEntry> {
    // SAFETY: union read; si_family decides the active member.
    if row.Address.si_family != AF_INET {
        return None;
    }
    let octets: [u8; 4] = std::mem::transmute(row.Address.Ipv4.sin_addr);
    if row.PhysicalAddressLength as usize != 6 {
        return None;
    }
    let p = &row.PhysicalAddress;
    Some(ArpEntry {
        ip: Ipv4Addr::from(octets),
        mac: format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            p[0], p[1], p[2], p[3], p[4], p[5]
        ),
        interface_index: row.InterfaceIndex,
        state: classify_state(row.State.0),
    })
}

fn classify_state(code: i32) -> NeighborState {
    match code {
        1 => NeighborState::Reachable,
        2 => NeighborState::Stale,
        3 => NeighborState::Delay,
        4 => NeighborState::Probe,
        5 => NeighborState::Permanent,
        14 => NeighborState::Incomplete,
        15 => NeighborState::Unreachable,
        _ => NeighborState::Other,
    }
}

/// Probe a single address with `SendARP`. Returns the MAC when the host
/// answers. Source address 0 lets the stack pick the interface.
pub fn probe_arp(ip: Ipv4Addr) -> Option<String> {
    unsafe {
        let dest = ip_to_wire(ip);
        let mut mac: [u8; 6] = [0; 6];
        let mut len: u32 = 6;
        let rc = SendARP(
            dest,
            0,
            mac.as_mut_ptr() as *mut core::ffi::c_void,
            &mut len,
        );
        if rc != 0 || len != 6 {
            return None;
        }
        // Some stacks answer with an all-zero MAC (proxy responders);
        // that is not a usable identity.
        if mac == [0, 0, 0, 0, 0, 0] {
            return None;
        }
        Some(format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        ))
    }
}

/// IP octets in network byte order packed into a u32 (what IP Helper APIs
/// expect on the wire).
pub fn ip_to_wire(ip: Ipv4Addr) -> u32 {
    u32::from_le_bytes(ip.octets())
}

pub fn wire_to_ip(w: u32) -> Ipv4Addr {
    Ipv4Addr::from(w.to_le_bytes())
}

/// Best-effort neighbor deletion; used by restoration actions.
pub fn delete_neighbor(ip: Ipv4Addr) -> NetResult<()> {
    let table = read_arp_table()?;
    let Some(entry) = table.iter().find(|e| e.ip == ip) else {
        return Ok(()); // already absent
    };
    unsafe {
        // SAFETY: IN_ADDR is 4 raw network-order bytes.
        let sin_addr = std::mem::transmute::<[u8; 4], windows::Win32::Networking::WinSock::IN_ADDR>(
            ip.octets(),
        );
        let mut row = MIB_IPNET_ROW2 {
            InterfaceIndex: entry.interface_index,
            ..Default::default()
        };
        row.Address.Ipv4.sin_family = AF_INET;
        row.Address.Ipv4.sin_addr = sin_addr;
        row.Address.si_family = AF_INET;
        let rc = DeleteIpNetEntry2(&row);
        if rc.0 != 0 {
            return Err(NetError::ArpQuery(rc.0));
        }
    }
    Ok(())
}

/// Filter an ARP-table snapshot down to usable IP -> MAC bindings. Pure;
/// unit-testable without hardware.
pub fn normalize_entries(entries: &[ArpEntry]) -> Vec<&ArpEntry> {
    entries
        .iter()
        .filter(|e| {
            !matches!(
                e.state,
                NeighborState::Incomplete | NeighborState::Unreachable
            )
        })
        .collect()
}

/// Compare two snapshots and return (new, changed, removed) MAC bindings.
pub fn diff_snapshots(
    before: &[ArpEntry],
    after: &[ArpEntry],
) -> (Vec<ArpEntry>, Vec<ArpEntry>, Vec<ArpEntry>) {
    let key = |e: &ArpEntry| (e.ip, e.mac.clone());
    let after_map: std::collections::HashMap<(Ipv4Addr, String), &ArpEntry> =
        after.iter().map(|e| (key(e), e)).collect();
    let before_map: std::collections::HashMap<(Ipv4Addr, String), &ArpEntry> =
        before.iter().map(|e| (key(e), e)).collect();

    let new: Vec<ArpEntry> = after
        .iter()
        .filter(|e| !before_map.contains_key(&key(e)))
        .cloned()
        .collect();
    let removed: Vec<ArpEntry> = before
        .iter()
        .filter(|e| !after_map.contains_key(&key(e)))
        .cloned()
        .collect();

    // A MAC change for the same IP appears as one removal + one addition.
    let changed: Vec<ArpEntry> = new
        .iter()
        .filter(|n| before.iter().any(|b| b.ip == n.ip && b.mac != n.mac))
        .cloned()
        .collect();
    (new, changed, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ip: &str, mac: &str, state: NeighborState) -> ArpEntry {
        ArpEntry {
            ip: ip.parse().unwrap(),
            mac: mac.into(),
            interface_index: 12,
            state,
        }
    }

    #[test]
    fn wire_roundtrip() {
        let ip: Ipv4Addr = "192.168.1.77".parse().unwrap();
        assert_eq!(wire_to_ip(ip_to_wire(ip)), ip);
    }

    #[test]
    fn classify_covers_known_states() {
        assert_eq!(classify_state(1), NeighborState::Reachable);
        assert_eq!(classify_state(2), NeighborState::Stale);
        assert_eq!(classify_state(15), NeighborState::Unreachable);
        assert_eq!(classify_state(99), NeighborState::Other);
    }

    #[test]
    fn normalize_skips_unusable() {
        let entries = vec![
            entry("10.0.0.1", "AA:AA:AA:AA:AA:01", NeighborState::Reachable),
            entry("10.0.0.2", "AA:AA:AA:AA:AA:02", NeighborState::Incomplete),
            entry("10.0.0.3", "AA:AA:AA:AA:AA:03", NeighborState::Unreachable),
            entry("10.0.0.4", "AA:AA:AA:AA:AA:04", NeighborState::Stale),
        ];
        let usable = normalize_entries(&entries);
        assert_eq!(usable.len(), 2);
    }

    #[test]
    fn diff_detects_new_changed_removed() {
        let before = vec![
            entry("10.0.0.1", "AA:AA:AA:AA:AA:01", NeighborState::Reachable),
            entry("10.0.0.2", "AA:AA:AA:AA:AA:02", NeighborState::Reachable),
        ];
        let after = vec![
            entry("10.0.0.1", "AA:AA:AA:AA:AA:01", NeighborState::Reachable),
            entry("10.0.0.2", "BB:BB:BB:BB:BB:02", NeighborState::Reachable), // changed MAC
            entry("10.0.0.9", "CC:CC:CC:CC:CC:09", NeighborState::Reachable), // new host
        ];
        let (new, changed, removed) = diff_snapshots(&before, &after);
        assert!(new.iter().any(|e| e.ip.to_string() == "10.0.0.9"));
        assert!(changed
            .iter()
            .any(|e| e.ip.to_string() == "10.0.0.2" && e.mac == "BB:BB:BB:BB:BB:02"));
        assert!(removed
            .iter()
            .any(|e| e.ip.to_string() == "10.0.0.2" && e.mac == "AA:AA:AA:AA:AA:02"));
    }

    #[test]
    fn arp_table_reads_on_real_machine() {
        // Smoke test: must not panic on a live system.
        let _ = read_arp_table();
    }
}
