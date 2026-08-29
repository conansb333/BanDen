//! wa_probe: standalone WhatsApp-block EXPERIMENT script.
//!
//! Poisons the target into the PC's MITM path (same primitives as the app's
//! engine) and applies configurable blocking rules while logging every flow.
//! Purpose: discover exactly which path WhatsApp uses when it "still works",
//! so the finding can be ported back into the engine/catalog.
//!
//! Rules (independent):
//!   dns    - kill DNS responses answering WhatsApp domains (default on)
//!   sni    - kill TLS ClientHellos with WhatsApp SNI (default on)
//!   cidr   - kill traffic to/from the catalog's WhatsApp IP ranges (default on)
//!   est    - kill flows that already existed when the block began
//!            (--kill-established)
//!   ports  - kill TCP flows on WhatsApp chat ports 5222/5228/5229/5230
//!            (--kill-chat-ports)
//!
//! Every flow is printed at exit (endpoints, bytes, DNS names, kill reason).
//!
//! Usage:
//!   wa_probe --ip 192.168.8.4 --mac 9C:2E:A1:2C:0A:99 \
//!            --gw-mac 22:99:FE:E7:89:B1 --gw-ip 192.168.8.1 \
//!            --seconds 240 [--kill-established] [--kill-chat-ports] \
//!            [--device-guid D4...]

use banden_net::control::CompiledBlocked;
use banden_net::dpi::{parse_dns, parse_sni, transport_of};
use banden_net::rawframe::{
    arp_reply_frame, parse_mac, set_ethernet_dst, set_ethernet_src, PcapLib, RawSender,
};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

const WHATSAPP_DOMAINS: &[&str] = &["whatsapp.com", "whatsapp.net", "whatsapp.org", "wa.me"];
const CHAT_PORTS: &[u16] = &[5222, 5228, 5229, 5230];

#[derive(Clone)]
struct FlowInfo {
    first_seen: Instant,
    last_seen: Instant,
    bytes_up: u64,
    bytes_down: u64,
    pkts: u64,
    killed_by: Option<&'static str>,
    names: Vec<String>,
}

fn flow_key(a: [u8; 4], ap: u16, b: [u8; 4], bp: u16) -> [u8; 12] {
    let (ip_a, port_a, ip_b, port_b) = if a <= b {
        (a, ap, b, bp)
    } else {
        (b, bp, a, ap)
    };
    let mut k = [0u8; 12];
    k[0..4].copy_from_slice(&ip_a);
    k[4..6].copy_from_slice(&port_a.to_be_bytes());
    k[6..10].copy_from_slice(&ip_b);
    k[10..12].copy_from_slice(&port_b.to_be_bytes());
    k
}

fn arg(name: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == name {
            return args.next();
        }
    }
    None
}

fn flag(name: &str) -> bool {
    std::env::args().skip(1).any(|a| a == name)
}

fn name_matches(domains: &[&str], name: &str) -> bool {
    let n = name.trim_end_matches('.').to_ascii_lowercase();
    domains
        .iter()
        .any(|d| n == *d || n.ends_with(&format!(".{d}")))
}

fn main() {
    let ip: Ipv4Addr = arg("--ip").expect("--ip").parse().expect("bad ip");
    let phone = parse_mac(&arg("--mac").expect("--mac")).unwrap();
    let gw_mac = parse_mac(&arg("--gw-mac").expect("--gw-mac")).unwrap();
    let gw_ip: Ipv4Addr = arg("--gw-ip").expect("--gw-ip").parse().expect("bad gw ip");
    let seconds: u64 = arg("--seconds").and_then(|v| v.parse().ok()).unwrap_or(240);
    let guid = arg("--device-guid");
    let kill_est = flag("--kill-established");
    let kill_ports = flag("--kill-chat-ports");

    let blocked = Arc::new(CompiledBlocked::from_ids(&["whatsapp".to_string()]));

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
            devices
                .iter()
                .find(|(_, desc)| desc.to_lowercase().contains("realtek"))
        })
        .cloned()
        .expect("no adapter");
    eprintln!("[wa_probe] capturing on {device} ({desc})");

    let sender = RawSender::open_ex(Arc::clone(&lib), &device, true, 200).expect("open capture");
    let our_mac: [u8; 6] = [0x34, 0x5a, 0x60, 0xc7, 0xd7, 0xb7];

    // Poison frames. ROUTER-ONLY, matching the engine: never send poisoned
    // replies to the phone itself - MIUI ignores them for data routing and
    // flips the phone to cellular (wire-verified on this LAN).
    let who_has_target = arp_reply_frame(our_mac, ip, gw_mac, gw_ip); // gw: target is at us

    // Background poison every 500 ms (3 fast repeats at startup).
    let stop_poison = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let lib2 = Arc::clone(&lib);
        let dev = device.clone();
        let who_target = who_has_target.clone();
        let stop2 = Arc::clone(&stop_poison);
        std::thread::spawn(move || {
            let poison = RawSender::open_ex(lib2, &dev, true, 200).expect("open poison sender");
            let mut n = 0u64;
            while !stop2.load(std::sync::atomic::Ordering::SeqCst) {
                let _ = poison.send(&who_target);
                n += 1;
                std::thread::sleep(Duration::from_millis(if n < 3 { 400 } else { 500 }));
            }
        });
    }

    let mut flows: HashMap<[u8; 12], FlowInfo> = HashMap::new();
    let mut ip_names: HashMap<Ipv4Addr, String> = HashMap::new();
    let mut dns_killed = 0u64;
    let mut established_cutoff: Option<Instant> = None;

    eprintln!(
        "[wa_probe] blocking WhatsApp for {seconds}s (est={kill_est}, ports={kill_ports}) - try WhatsApp now"
    );

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(seconds) {
        let frame = match sender.recv() {
            Ok(Some(f)) => f,
            Ok(None) => continue,
            Err(e) => {
                eprintln!("capture error: {e}");
                break;
            }
        };
        if frame.len() < 34 {
            continue;
        }
        let srcm: [u8; 6] = frame[6..12].try_into().unwrap();
        let dstm: [u8; 6] = frame[0..6].try_into().unwrap();
        let is_arp = frame[12] == 0x08 && frame[13] == 0x06;

        // Sticky responder: router asking for the target -> wired answers.
        if is_arp && frame.len() >= 42 {
            let op = u16::from_be_bytes([frame[20], frame[21]]);
            let spa = Ipv4Addr::new(frame[28], frame[29], frame[30], frame[31]);
            let tpa = Ipv4Addr::new(frame[38], frame[39], frame[40], frame[41]);
            if op == 1 && spa == gw_ip && tpa == ip {
                for _ in 0..4 {
                    let _ = sender.send(&who_has_target);
                }
                continue;
            }
        }
        if is_arp || frame[12] != 0x08 || frame[13] != 0x00 {
            continue;
        }
        let Some(transport) = transport_of(&frame) else {
            continue;
        };
        // The two directions that make up the MITM path:
        //  upstream   : phone -> PC  (phone was poisoned into sending here;
        //             MIUI may instead send straight to the router - those
        //             frames never reach us and cannot be filtered)
        //  downstream : gateway -> PC -> phone (the router was poisoned into
        //             handing the phone's inbound traffic to us)
        let upstream = srcm == phone && dstm == our_mac;
        let downstream = srcm == gw_mac && dstm == our_mac;
        if !upstream && !downstream {
            continue;
        }
        // Downstream must actually be headed to the target's IP.
        if downstream {
            let d = Ipv4Addr::new(frame[30], frame[31], frame[32], frame[33]);
            if d != ip {
                continue;
            }
        }

        let key = flow_key(
            transport.ip_src,
            transport.src_port,
            transport.ip_dst,
            transport.dst_port,
        );
        let entry = flows.entry(key).or_insert_with(|| FlowInfo {
            first_seen: Instant::now(),
            last_seen: Instant::now(),
            bytes_up: 0,
            bytes_down: 0,
            pkts: 0,
            killed_by: None,
            names: Vec::new(),
        });
        entry.last_seen = Instant::now();
        if upstream {
            entry.bytes_up += frame.len() as u64;
        } else {
            entry.bytes_down += frame.len() as u64;
        }
        entry.pkts += 1;

        let remote_ip = if upstream {
            Ipv4Addr::new(
                transport.ip_dst[0],
                transport.ip_dst[1],
                transport.ip_dst[2],
                transport.ip_dst[3],
            )
        } else {
            Ipv4Addr::new(
                transport.ip_src[0],
                transport.ip_src[1],
                transport.ip_src[2],
                transport.ip_src[3],
            )
        };
        let remote_port = if upstream {
            transport.dst_port
        } else {
            transport.src_port
        };

        // DNS observation + kill (downstream responses from any server).
        if downstream && transport.proto == 17 && transport.src_port == 53 {
            let off = transport.payload_off.unwrap_or(frame.len());
            if let Some(dns) = parse_dns(&frame[off..]) {
                let hit = dns
                    .answer_names
                    .iter()
                    .any(|n| name_matches(WHATSAPP_DOMAINS, n));
                for (ip4, n) in dns.answer_ips.iter().zip(&dns.answer_names) {
                    ip_names.entry(*ip4).or_insert_with(|| n.clone());
                }
                if hit {
                    dns_killed += 1;
                    continue; // drop WhatsApp's resolution
                }
            }
        }

        // Learn SNI on upstream TLS ClientHellos.
        if upstream && transport.proto == 6 {
            if let Some(off) = transport.payload_off {
                if off < frame.len() {
                    if let Some(sni) = parse_sni(&frame[off..]) {
                        let e = flows.get_mut(&key).unwrap();
                        if !e.names.iter().any(|n| n == &sni) {
                            e.names.push(sni.clone());
                        }
                    }
                }
            }
        }

        // Decide kill.
        let established_cutoff = *established_cutoff.get_or_insert(start + Duration::from_secs(2));
        let mut killed_by: Option<&'static str> = None;
        if blocked.ip_blocked(u32::from(remote_ip)) {
            killed_by = Some("cidr");
        }
        if kill_ports && CHAT_PORTS.contains(&remote_port) {
            killed_by = Some("chat-port");
        }
        if kill_est && established_cutoff <= Instant::now() {
            let first = flows.get(&key).unwrap().first_seen;
            if first < established_cutoff {
                killed_by = Some("established");
            }
        }
        if let Some(name) = ip_names.get(&remote_ip) {
            if name_matches(WHATSAPP_DOMAINS, name) {
                killed_by = Some("dns-learned");
            }
        }
        if let Some(f) = flows.get(&key) {
            if f.names.iter().any(|n| name_matches(WHATSAPP_DOMAINS, n)) {
                killed_by = Some("sni");
            }
            if f.killed_by.is_some() {
                killed_by = f.killed_by;
            }
        }

        let drop_frame = flows.get(&key).unwrap().killed_by.is_some() || killed_by.is_some();
        let e = flows.get_mut(&key).unwrap();
        if let Some(k) = killed_by {
            if e.killed_by.is_none() {
                e.killed_by = Some(k);
            }
        }

        if drop_frame {
            continue;
        }

        // Forward with MAC rewrite.
        let mut f = frame.clone();
        if upstream {
            set_ethernet_dst(&mut f, &gw_mac);
            set_ethernet_src(&mut f, &our_mac);
        } else {
            set_ethernet_dst(&mut f, &phone);
            set_ethernet_src(&mut f, &our_mac);
        }
        let _ = sender.send(&f);
    }
    let _ = established_cutoff;

    // Restoration: truthful announcements both directions.
    for _ in 0..3 {
        let _ = sender.send(&arp_reply_frame(gw_mac, gw_ip, phone, gw_ip));
        let _ = sender.send(&arp_reply_frame(phone, ip, gw_mac, gw_ip));
        std::thread::sleep(Duration::from_millis(150));
    }
    stop_poison.store(true, std::sync::atomic::Ordering::SeqCst);

    eprintln!(
        "\n[wa_probe] done (dns_killed={dns_killed}). FLOW TABLE ({} flows, top by bytes):",
        flows.len()
    );
    println!(
        "{:<24} {:>9} {:>9} {:>6} {:<11} dns/observed",
        "remote", "up", "down", "pkts", "killed_by",
    );
    let mut rows: Vec<_> = flows.into_iter().collect();
    rows.sort_by_key(|(_, f)| std::cmp::Reverse(f.bytes_up + f.bytes_down));
    for (key, f) in rows.iter().take(25) {
        let ip = Ipv4Addr::new(key[0], key[1], key[2], key[3]);
        let port = u16::from_be_bytes([key[4], key[5]]);
        let remote = remote_of(key, ip);
        let names = if f.names.is_empty() {
            ip_names.get(&remote).cloned().unwrap_or_default()
        } else {
            f.names.join(",")
        };
        println!(
            "{:<24} {:>9} {:>9} {:>6} {:<11} {}",
            format!("{remote}:{port}"),
            f.bytes_up,
            f.bytes_down,
            f.pkts,
            f.killed_by.unwrap_or("-"),
            names
        );
    }
}

fn remote_of(key: &[u8; 12], phone_ip: Ipv4Addr) -> Ipv4Addr {
    let a = Ipv4Addr::new(key[0], key[1], key[2], key[3]);
    let b = Ipv4Addr::new(key[6], key[7], key[8], key[9]);
    if a == phone_ip {
        b
    } else {
        a
    }
}
