//! Raw Ethernet frame sending via WinPcap/Npcap (`wpcap.dll`).
//!
//! The library is loaded at runtime with `LoadLibrary` (searching
//! `System32\Npcap\wpcap.dll` first, then plain `wpcap.dll`) so BanDen
//! builds and runs without the Npcap SDK. This is the same strategy
//! Wireshark uses.
//!
//! Frame construction (ARP replies / gratuitous ARP) lives here too so
//! the packet logic stays isolated from session management.

use crate::error::{NetError, NetResult};
use std::net::Ipv4Addr;
use std::sync::Arc;
use windows::core::{PCSTR, PCWSTR};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

// ---------------------------------------------------------------------------
// wpcap FFI types (subset)
// ---------------------------------------------------------------------------

#[repr(C)]
struct PcapIf {
    next: *mut PcapIf,
    name: *mut std::os::raw::c_char,
    description: *mut std::os::raw::c_char,
    addresses: *mut std::os::raw::c_void,
    flags: u32,
}

type PcapT = std::ffi::c_void;

type PcapFindAllDevs =
    unsafe extern "system" fn(alldevsp: *mut *mut PcapIf, errbuf: *mut u8) -> i32;
type PcapFreeAllDevs = unsafe extern "system" fn(alldevsp: *mut PcapIf);
type PcapOpenLive = unsafe extern "system" fn(
    device: *const std::os::raw::c_char,
    snaplen: i32,
    promisc: i32,
    to_ms: i32,
    errbuf: *mut u8,
) -> *mut PcapT;
type PcapSendPacket = unsafe extern "system" fn(p: *mut PcapT, buf: *const u8, size: i32) -> i32;
type PcapClose = unsafe extern "system" fn(p: *mut PcapT);
type PcapGetErr = unsafe extern "system" fn(p: *mut PcapT) -> *mut std::os::raw::c_char;
type PcapNextEx = unsafe extern "system" fn(
    p: *mut PcapT,
    pkt_header: *mut *mut PcapPkthdr,
    pkt_data: *mut *const u8,
) -> i32;

/// `pcap_pkthdr` as laid out by WinPcap/Npcap on Windows
/// (MSVC `long` is 32-bit even on x86_64).
#[repr(C)]
pub struct PcapPkthdr {
    pub ts_sec: i32,
    pub ts_usec: i32,
    pub caplen: u32,
    pub len: u32,
}

/// Loaded wpcap function table.
pub struct PcapLib {
    findalldevs: PcapFindAllDevs,
    freealldevs: PcapFreeAllDevs,
    openlive: PcapOpenLive,
    sendpacket: PcapSendPacket,
    close: PcapClose,
    geterr: PcapGetErr,
    nextex: PcapNextEx,
}

// The function pointers are process-wide immutable after load.
unsafe impl Send for PcapLib {}
unsafe impl Sync for PcapLib {}

const PCAP_ERRBUF_SIZE: usize = 256;

impl PcapLib {
    /// Load wpcap.dll (Npcap first, then WinPcap), or explain why not.
    pub fn load() -> NetResult<Arc<Self>> {
        unsafe {
            let candidates: [&str; 2] = ["C:\\Windows\\System32\\Npcap\\wpcap.dll", "wpcap.dll"];
            let mut module = windows::Win32::Foundation::HMODULE::default();
            for cand in candidates {
                let wide: Vec<u16> = cand.encode_utf16().chain(std::iter::once(0)).collect();
                if let Ok(m) = LoadLibraryW(PCWSTR(wide.as_ptr())) {
                    module = m;
                    break;
                }
            }
            if module.is_invalid() {
                return Err(NetError::PcapUnavailable(
                    "wpcap.dll not found; install Npcap (https://npcap.com) or WinPcap".into(),
                ));
            }
            let proc_addr = |name: &[u8]| -> *const std::os::raw::c_void {
                GetProcAddress(module, PCSTR(name.as_ptr()))
                    .map(|f| f as *const std::os::raw::c_void)
                    .unwrap_or(std::ptr::null())
            };
            let findalldevs = proc_addr(b"pcap_findalldevs\0");
            let freealldevs = proc_addr(b"pcap_freealldevs\0");
            let openlive = proc_addr(b"pcap_open_live\0");
            let sendpacket = proc_addr(b"pcap_sendpacket\0");
            let close = proc_addr(b"pcap_close\0");
            let geterr = proc_addr(b"pcap_geterr\0");
            let nextex = proc_addr(b"pcap_next_ex\0");
            let missing: Vec<&str> = [
                ("pcap_findalldevs", findalldevs),
                ("pcap_freealldevs", freealldevs),
                ("pcap_open_live", openlive),
                ("pcap_sendpacket", sendpacket),
                ("pcap_close", close),
                ("pcap_geterr", geterr),
                ("pcap_next_ex", nextex),
            ]
            .iter()
            .filter(|(_, p)| p.is_null())
            .map(|(n, _)| *n)
            .collect();
            if !missing.is_empty() {
                return Err(NetError::PcapUnavailable(format!(
                    "wpcap.dll is missing exports: {}",
                    missing.join(", ")
                )));
            }
            Ok(Arc::new(Self {
                findalldevs: std::mem::transmute::<*const std::os::raw::c_void, PcapFindAllDevs>(
                    findalldevs,
                ),
                freealldevs: std::mem::transmute::<*const std::os::raw::c_void, PcapFreeAllDevs>(
                    freealldevs,
                ),
                openlive: std::mem::transmute::<*const std::os::raw::c_void, PcapOpenLive>(
                    openlive,
                ),
                sendpacket: std::mem::transmute::<*const std::os::raw::c_void, PcapSendPacket>(
                    sendpacket,
                ),
                close: std::mem::transmute::<*const std::os::raw::c_void, PcapClose>(close),
                geterr: std::mem::transmute::<*const std::os::raw::c_void, PcapGetErr>(geterr),
                nextex: std::mem::transmute::<*const std::os::raw::c_void, PcapNextEx>(nextex),
            }))
        }
    }

    /// List adapter names known to wpcap (names look like
    /// `\Device\NPF_{GUID}` on WinPcap or `\Device\Npcap_{GUID}` on Npcap).
    pub fn list_devices(&self) -> NetResult<Vec<(String, String)>> {
        unsafe {
            let mut head: *mut PcapIf = std::ptr::null_mut();
            let mut errbuf = [0u8; PCAP_ERRBUF_SIZE];
            if (self.findalldevs)(&mut head, errbuf.as_mut_ptr()) != 0 {
                let msg = errbuf_cstr(&errbuf);
                return Err(NetError::PcapList(msg));
            }
            let mut out = Vec::new();
            let mut cur = head;
            while !cur.is_null() {
                let dev = &*cur;
                let name = if dev.name.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(dev.name)
                        .to_string_lossy()
                        .into_owned()
                };
                let desc = if dev.description.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(dev.description)
                        .to_string_lossy()
                        .into_owned()
                };
                out.push((name, desc));
                cur = dev.next;
            }
            if !head.is_null() {
                (self.freealldevs)(head);
            }
            Ok(out)
        }
    }

    /// Open an adapter and run `f` with the live handle (sends happen on
    /// the calling thread).
    pub fn with_sender(
        self: &Arc<Self>,
        device: &str,
        f: impl FnOnce(&RawSender) -> NetResult<()>,
    ) -> NetResult<()> {
        let sender = RawSender::open(Arc::clone(self), device)?;
        let result = f(&sender);
        drop(sender);
        result
    }
}

fn errbuf_cstr(buf: &[u8]) -> String {
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// An open adapter handle for sending. One per thread.
pub struct RawSender {
    lib: Arc<PcapLib>,
    handle: *mut PcapT,
}

unsafe impl Send for RawSender {}

impl RawSender {
    pub fn open(lib: Arc<PcapLib>, device: &str) -> NetResult<Self> {
        Self::open_ex(lib, device, false, 500)
    }

    /// `promisc` is required for the shaper's capture loop; `to_ms` is the
    /// read timeout (the loop checks its stop flag between timeouts).
    pub fn open_ex(lib: Arc<PcapLib>, device: &str, promisc: bool, to_ms: i32) -> NetResult<Self> {
        unsafe {
            let mut errbuf = [0u8; PCAP_ERRBUF_SIZE];
            let cdev = std::ffi::CString::new(device)
                .map_err(|_| NetError::PcapOpen("device name contains NUL".into()))?;
            let handle = (lib.openlive)(
                cdev.as_ptr(),
                65535,
                promisc as i32,
                to_ms,
                errbuf.as_mut_ptr(),
            );
            if handle.is_null() {
                return Err(NetError::PcapOpen(errbuf_cstr(&errbuf)));
            }
            Ok(Self { lib, handle })
        }
    }

    /// Receive one packet. Returns `Ok(None)` on read timeout.
    pub fn recv(&self) -> NetResult<Option<Vec<u8>>> {
        unsafe {
            let mut hdr: *mut PcapPkthdr = std::ptr::null_mut();
            let mut data: *const u8 = std::ptr::null();
            let rc = (self.lib.nextex)(self.handle, &mut hdr, &mut data);
            if rc == 0 {
                return Ok(None); // timeout
            }
            if rc < 0 {
                let err = (self.lib.geterr)(self.handle);
                let msg = if err.is_null() {
                    format!("next_ex rc={rc}")
                } else {
                    std::ffi::CStr::from_ptr(err).to_string_lossy().into_owned()
                };
                return Err(NetError::PcapRecv(msg));
            }
            let caplen = (*hdr).caplen as usize;
            let mut out = Vec::with_capacity(caplen);
            std::ptr::copy_nonoverlapping(data, out.as_mut_ptr(), caplen);
            out.set_len(caplen);
            Ok(Some(out))
        }
    }

    pub fn send(&self, frame: &[u8]) -> NetResult<()> {
        unsafe {
            let rc = (self.lib.sendpacket)(self.handle, frame.as_ptr(), frame.len() as i32);
            if rc != 0 {
                let err = (self.lib.geterr)(self.handle);
                let msg = if err.is_null() {
                    format!("sendpacket rc={rc}")
                } else {
                    std::ffi::CStr::from_ptr(err).to_string_lossy().into_owned()
                };
                return Err(NetError::PcapSend(msg));
            }
        }
        Ok(())
    }
}

impl Drop for RawSender {
    fn drop(&mut self) {
        unsafe { (self.lib.close)(self.handle) }
    }
}

// ---------------------------------------------------------------------------
// ARP frame construction
// ---------------------------------------------------------------------------

pub const BROADCAST: [u8; 6] = [0xFF; 6];

fn push_mac(buf: &mut Vec<u8>, mac: &[u8; 6]) {
    buf.extend_from_slice(mac);
}

fn push_ip(buf: &mut Vec<u8>, ip: Ipv4Addr) {
    buf.extend_from_slice(&ip.octets());
}

/// Build a full Ethernet + ARP reply frame.
///
/// `sender_*` is the identity being announced (this is what the receiver's
/// OS caches); `target_*` addresses the frame.
pub fn arp_reply_frame(
    sender_mac: [u8; 6],
    sender_ip: Ipv4Addr,
    target_mac: [u8; 6],
    target_ip: Ipv4Addr,
) -> Vec<u8> {
    let mut f = Vec::with_capacity(42);
    push_mac(&mut f, &target_mac); // ethernet dst
    push_mac(&mut f, &sender_mac); // ethernet src
    f.extend_from_slice(&[0x08, 0x06]); // ethertype ARP
    f.extend_from_slice(&[0x00, 0x01]); // hardware type: ethernet
    f.extend_from_slice(&[0x08, 0x00]); // protocol type: IPv4
    f.push(6); // hlen
    f.push(4); // plen
    f.extend_from_slice(&[0x00, 0x02]); // operation: reply
    push_mac(&mut f, &sender_mac); // sender hardware address
    push_ip(&mut f, sender_ip); // sender protocol address
    push_mac(&mut f, &target_mac); // target hardware address
    push_ip(&mut f, target_ip); // target protocol address
    f
}

/// Unsolicited (gratuitous) ARP reply to broadcast announcing
/// `sender_ip` is-at `sender_mac`. Receivers update their neighbor cache,
/// which is how restoration reaches devices we poisoned without a direct
/// frame to each of them.
pub fn arp_gratuitous_frame(sender_mac: [u8; 6], sender_ip: Ipv4Addr) -> Vec<u8> {
    arp_reply_frame(sender_mac, sender_ip, BROADCAST, sender_ip)
}

/// Overwrite a frame's ethernet destination MAC in place.
pub fn set_ethernet_dst(frame: &mut [u8], new_dst: &[u8; 6]) {
    frame[0..6].copy_from_slice(new_dst);
}

/// Overwrite a frame's ethernet source MAC in place.
///
/// Forwarded frames MUST carry the forwarding host's MAC as source:
/// keeping the original source would make the switch re-learn the
/// router/target MACs on this host's port (MAC flapping), cutting
/// connectivity for the whole network.
pub fn set_ethernet_src(frame: &mut [u8], new_src: &[u8; 6]) {
    frame[6..12].copy_from_slice(new_src);
}

/// IPv4 destination address of a frame, when it is an Ethernet/IPv4 frame.
pub fn ipv4_dst(frame: &[u8]) -> Option<Ipv4Addr> {
    if frame.len() >= 34 && frame[12] == 0x08 && frame[13] == 0x00 {
        Some(Ipv4Addr::new(frame[30], frame[31], frame[32], frame[33]))
    } else {
        None
    }
}

/// Parse a `AA:BB:CC:DD:EE:FF` (any separator) MAC into bytes.
pub fn parse_mac(s: &str) -> NetResult<[u8; 6]> {
    let hex: Vec<u8> = s
        .split(|c: char| !c.is_ascii_hexdigit())
        .filter(|p| !p.is_empty())
        .map(|p| u8::from_str_radix(p, 16).map_err(|_| NetError::MacParse(s.to_string())))
        .collect::<Result<_, _>>()?;
    if hex.len() == 6 && s.chars().filter(|c| c.is_ascii_hexdigit()).count() == 12 {
        Ok([hex[0], hex[1], hex[2], hex[3], hex[4], hex[5]])
    } else {
        Err(NetError::MacParse(s.to_string()))
    }
}

/// Best-effort probe: are we running elevated? Raw sending may be
/// restricted on Npcap's admin-only installs.
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_QUERY};
    use windows::Win32::System::Threading::GetCurrentProcess;
    unsafe {
        let mut token = HANDLE::default();
        if windows::Win32::System::Threading::OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY,
            &mut token,
        )
        .is_err()
        {
            return false;
        }
        let mut elevation: u32 = 0;
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut u32 as *mut std::os::raw::c_void),
            std::mem::size_of::<u32>() as u32,
            &mut ret_len,
        )
        .is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(token);
        ok && elevation != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arp_reply_frame_layout() {
        let f = arp_reply_frame(
            [0x01, 0x02, 0x03, 0x04, 0x05, 0x06],
            "192.168.8.1".parse().unwrap(),
            [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
            "192.168.8.4".parse().unwrap(),
        );
        assert_eq!(f.len(), 42);
        assert_eq!(&f[0..6], &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]); // dst
        assert_eq!(&f[6..12], &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]); // src
        assert_eq!(&f[12..14], &[0x08, 0x06]); // ARP
        assert_eq!(&f[14..16], &[0x00, 0x01]); // htype ethernet
        assert_eq!(&f[16..18], &[0x08, 0x00]); // IPv4
        assert_eq!(&f[20..22], &[0x00, 0x02]); // operation: reply
        assert_eq!(&f[22..28], &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]); // sha
        assert_eq!(&f[28..32], &[192, 168, 8, 1]); // spa
        assert_eq!(&f[32..38], &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]); // tha
        assert_eq!(&f[38..42], &[192, 168, 8, 4]); // tpa
    }

    #[test]
    fn gratuitous_frame_is_broadcast() {
        let f = arp_gratuitous_frame([0x11; 6], "10.0.0.1".parse().unwrap());
        assert_eq!(&f[0..6], &[0xFF; 6]);
        assert_eq!(&f[38..42], &[10, 0, 0, 1]); // tpa == spa
    }

    #[test]
    fn mac_parsing() {
        assert_eq!(
            parse_mac("9C:2E:A1:2C:0A:99").unwrap(),
            [0x9C, 0x2E, 0xA1, 0x2C, 0x0A, 0x99]
        );
        assert_eq!(
            parse_mac("9c-2e-a1-2c-0a-99").unwrap(),
            [0x9C, 0x2E, 0xA1, 0x2C, 0x0A, 0x99]
        );
        assert!(parse_mac("00:00:00:00:00").is_err());
        assert!(parse_mac("zz:zz").is_err());
    }

    #[test]
    fn wpcap_loads_when_installed() {
        // Machine-dependent smoke test. Machines WITH Npcap/WinPcap must get
        // a live library and visible adapters; machines without one (like
        // CI runners) skip the assertion instead of failing - wpcap is an
        // optional runtime dependency, not a build dependency.
        match PcapLib::load() {
            Ok(lib) => {
                let devs = lib.list_devices().unwrap();
                assert!(!devs.is_empty(), "no adapters visible to wpcap");
                assert!(devs
                    .iter()
                    .any(|(n, _)| n.contains("NPF_") || n.contains("Npcap_")));
            }
            Err(e) => eprintln!("skipping: wpcap not available on this machine ({e})"),
        }
    }
}
