//! Passive verification sniffer. Captures the wired NIC promiscuously and
//! counts frames routed by the gateway to the target device, split by
//! whether the remote endpoint matches a blocked app's network ranges.
//!
//! While an app-block session is live, WhatsApp (Meta AS32934) traffic
//! destined to the phone arrives here for dropping - this tool makes that
//! interception visible without touching the session.
//!
//! Usage: sniff_app --ip 192.168.8.4 --block whatsapp --seconds 120

use banden_net::control::CompiledBlocked;
use banden_net::{rawframe, PcapLib};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn arg(name: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == name {
            return args.next();
        }
    }
    None
}

fn main() {
    let ip = arg("--ip").expect("--ip required");
    let blocked_list: Vec<String> = arg("--block")
        .map(|s| s.split(',').map(|x| x.trim().to_string()).collect())
        .unwrap_or_default();
    let seconds: u64 = arg("--seconds").and_then(|v| v.parse().ok()).unwrap_or(60);
    let guid = arg("--device-guid");

    let target_ip: std::net::Ipv4Addr = ip.parse().expect("bad ip");
    let blocked = Arc::new(CompiledBlocked::from_ids(&blocked_list));
    let stop = Arc::new(AtomicBool::new(false));

    let lib = PcapLib::load().expect("npcap");
    let devices = lib.list_devices().expect("devices");
    // NPF device names are GUIDs; "ethernet" only appears in descriptions
    // (and also in VirtualBox host-only names), so prefer an explicit
    // --device-guid match against the name, then a Realtek-style
    // description, and only then fall back.
    let (device, _desc) = devices
        .iter()
        .filter(|(d, _)| {
            guid.as_deref()
                .map(|g| d.to_lowercase().contains(g))
                .unwrap_or(false)
        })
        .chain(devices.iter().filter(|(_, desc)| {
            desc.to_lowercase().contains("realtek") || desc.to_lowercase().contains("pcie")
        }))
        .next()
        .or_else(|| devices.first())
        .cloned()
        .expect("no adapter");

    let sender =
        rawframe::RawSender::open_ex(Arc::clone(&lib), &device, true, 400).expect("open capture");
    let sender = sender;

    let start = Instant::now();
    let mut to_target_total: u64 = 0;
    let mut to_target_blocked_bytes: u64 = 0;
    let mut to_target_blocked_pkts: u64 = 0;
    let mut other_bytes: u64 = 0;

    eprintln!(
        "sniffing {} for {}s; blocked apps: {:?} ({} cidrs)",
        target_ip,
        seconds,
        blocked_list,
        blocked.cidrs.len()
    );

    while !stop.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(seconds) {
        match sender.recv() {
            Ok(Some(frame)) => {
                if frame.len() < 34 {
                    continue;
                }
                // IPv4 unicast to the target only (ethertype 0x0800).
                if !(frame[12] == 0x08 && frame[13] == 0x00) {
                    continue;
                }
                let dst = std::net::Ipv4Addr::new(frame[30], frame[31], frame[32], frame[33]);
                if dst != target_ip {
                    continue;
                }
                to_target_total += 1;
                // Remote endpoint: IP header src at byte 26.
                let src = std::net::Ipv4Addr::new(frame[26], frame[27], frame[28], frame[29]);
                if blocked.ip_blocked(u32::from(src)) {
                    to_target_blocked_pkts += 1;
                    to_target_blocked_bytes += frame.len() as u64;
                } else {
                    other_bytes += frame.len() as u64;
                }
            }
            Ok(None) => continue,
            Err(e) => {
                eprintln!("capture error: {e}");
                break;
            }
        }
    }

    println!(
        "SNIFF RESULT: to_target_pkts={to_target_total} whatsapp_pkts={to_target_blocked_pkts} whatsapp_bytes={to_target_blocked_bytes} other_downstream_bytes={other_bytes}"
    );
    let _ = stop;
}
