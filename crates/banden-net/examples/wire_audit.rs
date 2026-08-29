//! Wire audit for per-app block sessions. Classifies how the target's
//! downstream actually flows while a block is live:
//!
//! - via-us   : router -> this PC -> target (the interception path; we
//!   split it by blocked-CIDR match so blocked vs forwarded traffic is
//!   visible)
//! - direct-v4: router -> target directly (ARP poison lost = bypass)
//! - via-v6   : any IPv6 delivered to the target's MAC (we never poison
//!   NDP, so IPv6 always flows direct = bypass lane) with the top
//!   destination addresses
//! - upstream : target -> router frames (context)
//!
//! Usage: wire_audit --ip 192.168.8.4 --mac 9C:2E:A1:2C:0A:99 \
//!                   --gw-mac 22:99:FE:E7:89:B1 --block whatsapp \
//!                   --seconds 240 [--device-guid D4A801E9]

use banden_net::appcatalog;
use banden_net::control::CompiledBlocked;
use banden_net::{rawframe, PcapLib};
use std::collections::HashMap;
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

fn mac6(s: &str) -> [u8; 6] {
    rawframe::parse_mac(s).expect("bad mac")
}

fn main() {
    let ip: std::net::Ipv4Addr = arg("--ip").expect("--ip").parse().expect("bad ip");
    let phone = mac6(&arg("--mac").expect("--mac"));
    let gw = mac6(&arg("--gw-mac").expect("--gw-mac"));
    let blocked_list: Vec<String> = arg("--block")
        .map(|s| s.split(',').map(|x| x.trim().to_string()).collect())
        .unwrap_or_default();
    let seconds: u64 = arg("--seconds").and_then(|v| v.parse().ok()).unwrap_or(240);
    let guid = arg("--device-guid");

    let blocked = Arc::new(CompiledBlocked::from_ids(&blocked_list));

    let lib = PcapLib::load().expect("npcap");
    let devices = lib.list_devices().expect("devices");
    let (device, desc) = devices
        .iter()
        .find(|(d, _)| {
            guid.as_deref()
                .map(|g| d.to_lowercase().contains(&g.to_lowercase()))
                .unwrap_or(false)
        })
        .or_else(|| {
            devices.iter().find(|(_, desc)| {
                let d = desc.to_lowercase();
                d.contains("realtek") || d.contains("pcie")
            })
        })
        .or_else(|| devices.first())
        .cloned()
        .expect("no adapter");
    eprintln!("capturing on {device} ({desc})");

    let sender =
        rawframe::RawSender::open_ex(Arc::clone(&lib), &device, true, 400).expect("open capture");
    let our_mac = mac6("34:5A:60:C7:D7:B7"); // this PC's Ethernet MAC

    let start = Instant::now();
    let mut via_us_total: u64 = 0;
    let mut via_us_blocked: u64 = 0;
    let mut via_us_blocked_bytes: u64 = 0;
    let mut via_us_other: u64 = 0;
    let mut direct_v4: u64 = 0;
    let mut v6_to_phone: u64 = 0;
    let mut v6_from_gw_to_phone: u64 = 0;
    let mut upstream: u64 = 0;
    let mut other_remote: HashMap<String, u64> = HashMap::new();
    let mut v6_dsts: HashMap<String, u64> = HashMap::new();
    // Frames WE forwarded to the phone (the app let them through).
    let mut delivered_total: u64 = 0;
    let mut delivered_bytes: u64 = 0;
    let mut delivered_blocked: u64 = 0;
    let mut delivered_blocked_bytes: u64 = 0;
    let mut delivered_remote: HashMap<String, u64> = HashMap::new();
    // DNS responses to the phone whose answers name a blocked app
    // (suffixes come from the shared app catalog for --block).
    let blocked_apps = appcatalog::resolve_ids(&blocked_list);
    let mut dns_blocked_hits: u64 = 0;
    let mut dns_other_hits: u64 = 0;
    // Timestamped events for anything that would otherwise be a bare
    // counter, so bypass/blocked frames can be attributed to the session
    // window (elapsed < duration) vs the capture's baseline margins.
    let mut events: Vec<String> = Vec::new();

    while start.elapsed() < Duration::from_secs(seconds) {
        match sender.recv() {
            Ok(Some(frame)) => {
                if frame.len() < 34 {
                    continue;
                }
                let dstm: [u8; 6] = frame[0..6].try_into().unwrap();
                let srcm: [u8; 6] = frame[6..12].try_into().unwrap();
                let etype = u16::from_be_bytes([frame[12], frame[13]]);

                if srcm == phone && dstm == gw {
                    upstream += 1;
                    continue;
                }
                // Direct bypass: router handing frames straight to the phone.
                if srcm == gw && dstm == phone {
                    if etype == 0x86DD {
                        v6_from_gw_to_phone += 1;
                        if frame.len() >= 78 {
                            let d = format!(
                                "{:x}",
                                u128::from_be_bytes(frame[38..54].try_into().unwrap())
                            );
                            *v6_dsts
                                .entry(format!("{}...{}", &d[..8], &d[24..]))
                                .or_insert(0) += 1;
                        }
                    } else if etype == 0x0800 {
                        direct_v4 += 1;
                        let src =
                            std::net::Ipv4Addr::new(frame[26], frame[27], frame[28], frame[29]);
                        events.push(format!(
                            "+{:>4}s direct_v4 src={src} len={}",
                            start.elapsed().as_secs(),
                            frame.len()
                        ));
                    }
                    continue;
                }
                // General IPv6 delivered to the phone (any source).
                if etype == 0x86DD && dstm == phone {
                    v6_to_phone += 1;
                    continue;
                }
                // Frames delivered to the phone by our own forwarder.
                if srcm == our_mac && dstm == phone && etype == 0x0800 && frame.len() >= 38 {
                    delivered_total += 1;
                    delivered_bytes += frame.len() as u64;
                    let src = std::net::Ipv4Addr::new(frame[26], frame[27], frame[28], frame[29]);
                    if blocked.ip_blocked(u32::from(src)) {
                        delivered_blocked += 1;
                        delivered_blocked_bytes += frame.len() as u64;
                        events.push(format!(
                            "+{:>4}s delivered_blocked src={src} proto={} len={}",
                            start.elapsed().as_secs(),
                            frame[23],
                            frame.len()
                        ));
                    }
                    *delivered_remote.entry(src.to_string()).or_insert(0) += 1;
                    continue;
                }
                if etype != 0x0800 {
                    continue;
                }
                // IPv4 downstream captured at this PC (the interception path).
                if dstm == our_mac && frame.len() >= 38 {
                    let dst = std::net::Ipv4Addr::new(frame[30], frame[31], frame[32], frame[33]);
                    if dst != ip {
                        continue;
                    }
                    via_us_total += 1;
                    let src = std::net::Ipv4Addr::new(frame[26], frame[27], frame[28], frame[29]);
                    // DNS response? then check whether its answers name a
                    // blocked app (the forwarder kills these to stop
                    // resolution even when IPs are unknown yet).
                    let proto = frame[23];
                    let src_port = u16::from_be_bytes([frame[34], frame[35]]);
                    if proto == 17 && src_port == 53 {
                        if let Some(off) =
                            banden_net::dpi::transport_of(&frame).and_then(|t| t.payload_off)
                        {
                            if off < frame.len() {
                                if let Some(dns) = banden_net::dpi::parse_dns(&frame[off..]) {
                                    let hit = dns.answer_names.iter().any(|n| {
                                        blocked_apps.iter().any(|a| appcatalog::name_matches(a, n))
                                    });
                                    if hit {
                                        dns_blocked_hits += 1;
                                    } else {
                                        dns_other_hits += 1;
                                    }
                                }
                            }
                        }
                    }
                    if blocked.ip_blocked(u32::from(src)) {
                        via_us_blocked += 1;
                        via_us_blocked_bytes += frame.len() as u64;
                        events.push(format!(
                            "+{:>4}s via_us_blocked src={src} proto={} len={}",
                            start.elapsed().as_secs(),
                            frame[23],
                            frame.len()
                        ));
                    } else {
                        via_us_other += 1;
                        let o = src.octets();
                        // crude global check: not private/reserved/link-local
                        let is_global = !(o[0] == 10
                            || o[0] == 127
                            || (o[0] == 172 && o[1] >= 16 && o[1] <= 31)
                            || (o[0] == 192 && o[1] == 168)
                            || (o[0] == 169 && o[1] == 254)
                            || o[0] >= 224);
                        if is_global {
                            *other_remote.entry(src.to_string()).or_insert(0) += 1;
                        }
                    }
                }
            }
            Ok(None) => continue,
            Err(e) => {
                eprintln!("capture error: {e}");
                break;
            }
        }
    }

    let mut others: Vec<(String, u64)> = other_remote.into_iter().collect();
    others.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    let mut v6s: Vec<(String, u64)> = v6_dsts.into_iter().collect();
    v6s.sort_by_key(|(_, c)| std::cmp::Reverse(*c));

    println!("WIRE AUDIT RESULT ({}s)", start.elapsed().as_secs());
    println!("  via_us_total={via_us_total} (interception path active if >0)");
    println!("  via_us_blocked={via_us_blocked} pkts / {via_us_blocked_bytes} B (dropped)");
    println!("  via_us_other_forwarded={via_us_other}");
    println!("  direct_v4_bypass={direct_v4} (must be 0 while poison holds)");
    println!("  v6_to_phone_total={v6_to_phone} v6_from_router={v6_from_gw_to_phone}");
    println!("  upstream_direct={upstream}");
    println!("  delivered_to_phone={delivered_total} pkts / {delivered_bytes} B (what the app let through)");
    println!(
        "  delivered_blocked={delivered_blocked} pkts / {delivered_blocked_bytes} B (must be 0)"
    );
    println!("  dns_responses_blocked_named={dns_blocked_hits} other={dns_other_hits}");
    if !events.is_empty() {
        println!("  timestamped blocked/bypass events (elapsed since capture start):");
        for e in events.iter().take(50) {
            println!("    {e}");
        }
    }
    println!("  top forwarded remote IPv4s:");
    for (ip, c) in others.iter().take(10) {
        println!("    {ip} x{c}");
    }
    println!("  top IPv6 destinations to phone:");
    for (a, c) in v6s.iter().take(8) {
        println!("    {a} x{c}");
    }
    let mut delivered: Vec<(String, u64)> = delivered_remote.into_iter().collect();
    delivered.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    println!("  top delivered remote IPv4s:");
    for (ip, c) in delivered.iter().take(10) {
        println!("    {ip} x{c}");
    }
    let _ = AtomicBool::new(false);
    let _ = Ordering::SeqCst;
}
