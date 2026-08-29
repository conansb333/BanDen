//! Network interface discovery via `GetAdaptersAddresses`.
//!
//! All Win32 calls are blocking FFI and must run inside `spawn_blocking`
//! (the discovery service does this); the functions here are synchronous.

use crate::error::{NetError, NetResult};
use banden_core::InterfaceInfo;
use std::net::Ipv4Addr;
use windows::Win32::Foundation::ERROR_BUFFER_OVERFLOW;
use windows::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST,
    GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GET_ADAPTERS_ADDRESSES_FLAGS,
    IP_ADAPTER_ADDRESSES_LH,
};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR, SOCKADDR_IN};

const GAA_FLAGS: GET_ADAPTERS_ADDRESSES_FLAGS = GET_ADAPTERS_ADDRESSES_FLAGS(
    GAA_FLAG_INCLUDE_GATEWAYS.0
        | GAA_FLAG_SKIP_ANYCAST.0
        | GAA_FLAG_SKIP_MULTICAST.0
        | GAA_FLAG_SKIP_DNS_SERVER.0,
);

const IF_TYPE_ETHERNET_CSMACD: u32 = 6;
const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
const IF_TYPE_IEEE80211: u32 = 71;

/// Enumerate all adapters, best-effort per adapter.
pub fn list_interfaces() -> NetResult<Vec<InterfaceInfo>> {
    unsafe {
        let mut size: u32 = 16 * 1024;
        let mut buffer: Vec<u8>;
        loop {
            buffer = vec![0u8; size as usize];
            let rc = GetAdaptersAddresses(
                AF_INET.0 as u32,
                GAA_FLAGS,
                None,
                Some(buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
                &mut size,
            );
            if rc == 0 {
                break;
            }
            if rc == ERROR_BUFFER_OVERFLOW.0 {
                continue; // size was updated; retry with the larger buffer
            }
            return Err(NetError::AdapterQuery(rc));
        }

        let mut out = Vec::new();
        let mut current = buffer.as_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
        while !current.is_null() {
            let adapter = &*current;
            match adapter_to_info(adapter) {
                Ok(Some(info)) => out.push(info),
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e, "skipping malformed adapter entry"),
            }
            current = adapter.Next;
        }
        Ok(out)
    }
}

unsafe fn adapter_to_info(adapter: &IP_ADAPTER_ADDRESSES_LH) -> NetResult<Option<InterfaceInfo>> {
    let adapter_name = adapter
        .AdapterName
        .to_string()
        .unwrap_or_else(|_| "unknown".into());
    let friendly_name = adapter.FriendlyName.to_string().ok();
    let mac = if adapter.PhysicalAddressLength == 6 {
        Some(format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            adapter.PhysicalAddress[0],
            adapter.PhysicalAddress[1],
            adapter.PhysicalAddress[2],
            adapter.PhysicalAddress[3],
            adapter.PhysicalAddress[4],
            adapter.PhysicalAddress[5]
        ))
    } else {
        None
    };

    let is_loopback = adapter.IfType == IF_TYPE_SOFTWARE_LOOPBACK;
    let is_up = adapter.OperStatus == IfOperStatusUp;
    let is_physical =
        adapter.IfType == IF_TYPE_ETHERNET_CSMACD || adapter.IfType == IF_TYPE_IEEE80211;
    // SAFETY: union read of the length/index prefix.
    let if_index = adapter.Anonymous1.Anonymous.IfIndex;

    // First IPv4 unicast address.
    let mut ipv4: Option<(Ipv4Addr, u8)> = None;
    let mut ua = adapter.FirstUnicastAddress;
    while !ua.is_null() && ipv4.is_none() {
        let entry = &*ua;
        if let Some((ip, _)) = sockaddr_ipv4(entry.Address.lpSockaddr) {
            ipv4 = Some((ip, entry.OnLinkPrefixLength));
        }
        ua = entry.Next;
    }

    // First gateway.
    let mut gateway: Option<Ipv4Addr> = None;
    let mut ga = adapter.FirstGatewayAddress;
    while !ga.is_null() && gateway.is_none() {
        let entry = &*ga;
        if let Some((ip, _)) = sockaddr_ipv4(entry.Address.lpSockaddr) {
            gateway = Some(ip);
        }
        ga = entry.Next;
    }

    let Some((ip, prefix_len)) = ipv4 else {
        return Ok(None); // no IPv4: not usable for BanDen
    };

    let cidr = format!("{}/{}", apply_mask(&ip, prefix_len), prefix_len);

    Ok(Some(InterfaceInfo {
        id: adapter_name,
        if_index: Some(if_index),
        name: friendly_name.clone().unwrap_or_else(|| "adapter".into()),
        friendly_name,
        mac_address: mac,
        ipv4: Some(ip.to_string()),
        prefix_len: Some(prefix_len),
        cidr: Some(cidr),
        gateway: gateway.map(|g| g.to_string()),
        is_up,
        is_loopback,
        is_physical,
    }))
}

/// Extract the IPv4 address from a sockaddr; None for non-IPv4.
unsafe fn sockaddr_ipv4(sa: *mut SOCKADDR) -> Option<(Ipv4Addr, u8)> {
    if sa.is_null() || (*sa).sa_family != AF_INET {
        return None;
    }
    let sin = &*(sa as *const SOCKADDR as *const SOCKADDR_IN);
    // SAFETY: IN_ADDR is 4 raw bytes in network order.
    let octets: [u8; 4] = std::mem::transmute(sin.sin_addr);
    Some((Ipv4Addr::from(octets), 0))
}

fn apply_mask(ip: &Ipv4Addr, prefix: u8) -> Ipv4Addr {
    if prefix == 0 {
        return Ipv4Addr::UNSPECIFIED;
    }
    let mask: u32 = if prefix >= 32 {
        u32::MAX
    } else {
        u32::MAX << (32 - prefix as u32)
    };
    Ipv4Addr::from(u32::from(*ip) & mask)
}

/// Pick the default working interface: preferred id, else the first
/// up, physical, non-loopback adapter with IPv4 + gateway.
pub fn select_interface(
    interfaces: &[InterfaceInfo],
    preferred: Option<&str>,
) -> Option<InterfaceInfo> {
    if let Some(id) = preferred {
        if let Some(found) = interfaces.iter().find(|i| i.id == id && i.is_up) {
            return Some(found.clone());
        }
    }
    interfaces
        .iter()
        .filter(|i| i.is_up && !i.is_loopback && i.ipv4.is_some())
        .max_by_key(|i| (i.is_physical, i.gateway.is_some()))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_applies_correctly() {
        let ip: Ipv4Addr = "192.168.1.130".parse().unwrap();
        assert_eq!(apply_mask(&ip, 24).to_string(), "192.168.1.0");
        assert_eq!(apply_mask(&ip, 16).to_string(), "192.168.0.0");
        assert_eq!(apply_mask(&ip, 32).to_string(), "192.168.1.130");
        assert_eq!(apply_mask(&ip, 0).to_string(), "0.0.0.0");
        let ip2: Ipv4Addr = "10.1.2.3".parse().unwrap();
        assert_eq!(apply_mask(&ip2, 8).to_string(), "10.0.0.0");
    }

    #[test]
    fn select_prefers_physical_with_gateway() {
        let mk = |id: &str, physical: bool, gw: bool, up: bool| InterfaceInfo {
            id: id.into(),
            if_index: Some(1),
            name: id.into(),
            friendly_name: Some(id.into()),
            mac_address: None,
            ipv4: Some("192.168.1.2".into()),
            prefix_len: Some(24),
            cidr: Some("192.168.1.0/24".into()),
            gateway: gw.then(|| "192.168.1.1".into()),
            is_up: up,
            is_loopback: false,
            is_physical: physical,
        };
        let list = vec![
            mk("vpn", false, true, true),
            mk("eth", true, true, true),
            mk("wifi", true, false, true),
            mk("down", true, true, false),
        ];
        assert_eq!(select_interface(&list, None).unwrap().id, "eth");
        assert_eq!(select_interface(&list, Some("vpn")).unwrap().id, "vpn");
        assert_eq!(select_interface(&list, Some("down")).unwrap().id, "eth");
    }

    #[test]
    fn lists_real_interfaces() {
        // Machine-dependent smoke test; must never panic.
        let list = list_interfaces().unwrap();
        assert!(list.iter().any(|i| i.is_loopback));
    }
}
