//! Reverse DNS via `getnameinfo` (ws2_32), with one-time WSAStartup.

use crate::error::{NetError, NetResult};
use std::net::Ipv4Addr;
use windows::Win32::Networking::WinSock::{
    getnameinfo, socklen_t, WSAStartup, AF_INET, NI_NAMEREQD, SOCKADDR, SOCKADDR_IN, WSADATA,
};

static WSA_INIT: std::sync::Once = std::sync::Once::new();
static WSA_RESULT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Resolve a hostname for an IPv4 address. Returns None when the name is
/// unknown — a missing PTR record is normal, not an error.
pub fn resolve_hostname(ip: Ipv4Addr) -> Option<String> {
    ensure_wsa().ok()?;
    unsafe {
        let sa = SOCKADDR_IN {
            sin_family: AF_INET,
            sin_port: 0,
            // SAFETY: IN_ADDR is 4 raw bytes.
            sin_addr: std::mem::transmute::<[u8; 4], windows::Win32::Networking::WinSock::IN_ADDR>(
                ip.octets(),
            ),
            sin_zero: Default::default(),
        };
        let mut node: [u8; 256] = [0; 256];
        let rc = getnameinfo(
            &sa as *const SOCKADDR_IN as *const SOCKADDR,
            socklen_t(std::mem::size_of::<SOCKADDR_IN>() as i32),
            Some(&mut node),
            None,
            NI_NAMEREQD as i32,
        );
        if rc != 0 {
            return None;
        }
        let end = node.iter().position(|b| *b == 0).unwrap_or(node.len());
        let name = String::from_utf8_lossy(&node[..end]).into_owned();
        // Some resolvers answer unknown names with "." or similar junk;
        // only accept names containing at least one letter or digit.
        if !name.chars().any(|c| c.is_ascii_alphanumeric()) {
            None
        } else {
            Some(name)
        }
    }
}

fn ensure_wsa() -> NetResult<()> {
    WSA_INIT.call_once(|| unsafe {
        let mut data = WSADATA::default();
        let rc = WSAStartup(0x0202, &mut data);
        WSA_RESULT.store(rc, std::sync::atomic::Ordering::SeqCst);
    });
    let stored = WSA_RESULT.load(std::sync::atomic::Ordering::SeqCst);
    if stored != 0 {
        return Err(NetError::WsaStartup(stored));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsa_startup_succeeds() {
        assert!(ensure_wsa().is_ok());
        assert!(ensure_wsa().is_ok()); // idempotent
    }

    #[test]
    fn resolve_never_panics() {
        let ip: Ipv4Addr = "127.0.0.1".parse().unwrap();
        let _ = resolve_hostname(ip);
    }

    #[test]
    fn junk_names_are_rejected() {
        // The filter lives inline in resolve_hostname; keep a sanity test
        // for the predicate it implements.
        let is_usable = |name: &str| name.chars().any(|c| c.is_ascii_alphanumeric());
        assert!(!is_usable("."));
        assert!(!is_usable(""));
        assert!(!is_usable("-"));
        assert!(is_usable("desktop-1"));
        assert!(is_usable("router.local"));
    }
}
