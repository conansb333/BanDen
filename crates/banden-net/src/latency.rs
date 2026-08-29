//! Gateway latency probe via the IP Helper ICMP API (`IcmpSendEcho`).

use crate::arp::ip_to_wire;
use crate::error::NetError;
use std::net::Ipv4Addr;
use std::time::Duration;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::NetworkManagement::IpHelper::{IcmpCreateFile, IcmpSendEcho, ICMP_ECHO_REPLY};

/// Measure round-trip time to `target` with a single ICMP echo.
/// Returns None when the target does not answer within the timeout.
pub fn ping(target: Ipv4Addr, timeout: Duration) -> Result<Option<u64>, NetError> {
    unsafe {
        let handle = IcmpCreateFile().map_err(|e| NetError::Icmp(e.code().0 as u32))?;
        let request: [u8; 8] = *b"banden!!";
        let mut reply_buffer: [u8; 128] = [0; 128];
        let replies = IcmpSendEcho(
            handle,
            ip_to_wire(target),
            request.as_ptr() as *const core::ffi::c_void,
            request.len() as u16,
            None,
            reply_buffer.as_mut_ptr() as *mut core::ffi::c_void,
            reply_buffer.len() as u32,
            timeout.as_millis().min(u32::MAX as u128) as u32,
        );
        let _ = CloseHandle(handle);
        if replies == 0 {
            return Ok(None); // timeout or unreachable
        }
        let reply = &*(reply_buffer.as_ptr() as *const ICMP_ECHO_REPLY);
        if reply.Status != 0 {
            return Ok(None);
        }
        Ok(Some(reply.RoundTripTime as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_answers() {
        let rtt = ping(Ipv4Addr::LOCALHOST, Duration::from_millis(1000)).unwrap();
        assert!(rtt.is_some());
    }

    #[test]
    fn unroutable_times_out() {
        // TEST-NET address; should not answer.
        let rtt = ping("192.0.2.123".parse().unwrap(), Duration::from_millis(300)).unwrap();
        assert!(rtt.is_none());
    }
}
