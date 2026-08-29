//! Real end-to-end verification of the shaper.
//!
//! Starts the ARP shaper against a device, then floods its downstream
//! path with crafted ICMP echo requests that appear to come from an
//! external internet host (6.6.6.6). The device's real TCP/IP stack
//! answers them, and every reply must travel back through the shaper's
//! upload bucket. The downstream requests themselves are throttled by
//! the download bucket.
//!
//! The printed rates come from the shaper's live byte counters - real
//! frames on the wire, not simulated numbers.
//!
//! Usage: shaper_test --ip 192.168.8.4 --mac 9C:2E:A1:2C:0A:99 \
//!                    --down-mbps 1 --up-mbps 1 --seconds 20

use banden_core::{ControlBackend, Session, SessionConfig, SessionStateMachine};
use banden_net::rawframe::{parse_mac, PcapLib, RawSender};
use banden_net::ArpCutBackend;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn internet_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// Build a full Ethernet + IPv4 + ICMP echo-request frame.
/// `gatewat_mac`/`our_mac`: the frame must look like it arrives FROM the
/// gateway TO this PC (the poisoned downstream path).
#[allow(clippy::too_many_arguments)]
fn icmp_frame(
    eth_src: [u8; 6],
    eth_dst: [u8; 6],
    ip_src: Ipv4Addr,
    ip_dst: Ipv4Addr,
    ident: u16,
    seq: u16,
    payload_len: usize,
) -> Vec<u8> {
    let ip_total = 20 + 8 + payload_len;
    let mut f = Vec::with_capacity(14 + ip_total);
    // Ethernet
    f.extend_from_slice(&eth_dst);
    f.extend_from_slice(&eth_src);
    f.extend_from_slice(&[0x08, 0x00]);
    // IPv4 header
    f.push(0x45); // v4, IHL 5
    f.push(0x00); // DSCP
    f.extend_from_slice(&(ip_total as u16).to_be_bytes());
    f.extend_from_slice(&(ident).to_be_bytes()); // identification
    f.extend_from_slice(&[0x00, 0x00]); // flags/frag
    f.push(64); // TTL
    f.push(1); // proto ICMP
    f.extend_from_slice(&[0x00, 0x00]); // checksum placeholder
    f.extend_from_slice(&ip_src.octets());
    f.extend_from_slice(&ip_dst.octets());
    let ip_h = f.len() - 20;
    let csum = internet_checksum(&f[ip_h..]);
    f[ip_h + 10..ip_h + 12].copy_from_slice(&csum.to_be_bytes());
    // ICMP echo request
    f.push(8); // type
    f.push(0); // code
    f.extend_from_slice(&[0x00, 0x00]); // checksum placeholder
    f.extend_from_slice(&ident.to_be_bytes());
    f.extend_from_slice(&seq.to_be_bytes());
    let payload_byte = (seq as u8).wrapping_mul(31).wrapping_add(7);
    f.extend(std::iter::repeat(payload_byte).take(payload_len));
    let icmp_start = 14 + 20;
    let csum = internet_checksum(&f[icmp_start..]);
    f[icmp_start + 2..icmp_start + 4].copy_from_slice(&csum.to_be_bytes());
    f
}

fn main() {
    let mut ip = None;
    let mut mac = None;
    let mut down_mbps: u64 = 1;
    let mut up_mbps: u64 = 1;
    let mut seconds: u64 = 20;
    let mut payload_len: usize = 1200;
    let mut blocked: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--ip" => ip = args.next(),
            "--mac" => mac = args.next(),
            "--down-mbps" => down_mbps = args.next().and_then(|v| v.parse().ok()).unwrap_or(1),
            "--up-mbps" => up_mbps = args.next().and_then(|v| v.parse().ok()).unwrap_or(1),
            "--seconds" => seconds = args.next().and_then(|v| v.parse().ok()).unwrap_or(20),
            "--payload" => payload_len = args.next().and_then(|v| v.parse().ok()).unwrap_or(1200),
            "--block" => {
                if let Some(list) = args.next() {
                    blocked = list.split(',').map(|s| s.trim().to_string()).collect();
                }
            }
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(1);
            }
        }
    }
    let ip = ip.expect("--ip required");
    let mac = mac.expect("--mac required");

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("warn"))
        .init();

    let backend = ArpCutBackend::new().expect("wpcap must be available");
    let lib = PcapLib::load().expect("wpcap");
    let session = Session {
        id: uuid::Uuid::new_v4(),
        config: SessionConfig {
            target_mac: mac.clone(),
            target_ip: ip.clone(),
            target_label: Some("shaper-test".into()),
            download_limit_bps: Some(down_mbps * 1_000_000),
            upload_limit_bps: Some(up_mbps * 1_000_000),
            blocked_apps: blocked.clone(),
            allowed_apps: Vec::new(),
            duration_secs: None,
            priority: None,
        },
        machine: SessionStateMachine::new(),
        created_at: chrono::Utc::now(),
        started_at: None,
        ended_at: None,
        error: None,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async move {
        let captured = backend.prepare(&session).await.expect("prepare");
        backend.apply(&session).await.expect("apply");
        println!(
            "[shaper_test] shaping {} at {down_mbps}/{up_mbps} Mbps - letting ARP poison settle...",
            ip
        );
        tokio::time::sleep(Duration::from_secs(4)).await;

        // Resolve the frames the injector needs: this PC's identity on the
        // target's subnet and the real gateway MAC.
        let interfaces = banden_net::list_interfaces().expect("interfaces");
        let target: Ipv4Addr = ip.parse().unwrap();
        let sel = banden_net::interface_for_target(&interfaces, target)
            .expect("no interface routes the target");
        let our_mac = parse_mac(sel.mac_address.as_deref().unwrap()).unwrap();
        let ext_src: Ipv4Addr = "6.6.6.6".parse().unwrap();

        // Injector: real ICMP echo requests delivered DIRECTLY to the
        // target, with a spoofed external source (6.6.6.6). The target's
        // genuine kernel answers them - and because its gateway entry is
        // poisoned to this PC, every reply arrives through the shaper's
        // capture handle and is capped by the upload bucket. (Requires a
        // target stack that honors unsolicited ARP updates, e.g. Linux;
        // some Android builds ignore them.)
        let device = banden_net::pcap_device_for_target(&lib, target).expect("pcap device");
        let inj = Arc::new(Mutex::new(
            RawSender::open_ex(Arc::clone(&lib), &device, false, 100).expect("injector"),
        ));

        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let inj2 = Arc::clone(&inj);
        let phone_ip: Ipv4Addr = ip.parse().unwrap();
        let target_mac = parse_mac(&mac).unwrap();
        let flood = std::thread::spawn(move || {
            let inj = inj2.lock().unwrap();
            let mut seq: u16 = 1;
            while !stop2.load(Ordering::SeqCst) {
                let f = icmp_frame(
                    our_mac,
                    target_mac,
                    ext_src,
                    phone_ip,
                    0xBAD1,
                    seq,
                    payload_len,
                );
                if inj.send(&f).is_err() {
                    break;
                }
                seq = seq.wrapping_add(1);
            }
        });

        // Sampler: report the shaper's measured rates every second.
        println!(
            "{:>6} {:>14} {:>14} {:>14} {:>14}",
            "sec", "down_fwd_bps", "down_drop_bps", "up_fwd_bps", "up_drop_bps"
        );
        // stats() = (up_forwarded, up_dropped, down_forwarded, down_dropped)
        let mut prev = backend.stats().unwrap_or_default();
        let _started = Instant::now();
        for sec in 1..=seconds {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let cur = backend.stats().unwrap_or_default();
            let uf = cur.0.wrapping_sub(prev.0);
            let ud = cur.1.wrapping_sub(prev.1);
            let df = cur.2.wrapping_sub(prev.2);
            let dd = cur.3.wrapping_sub(prev.3);
            prev = cur;
            println!(
                "{:>6} {:>14} {:>14} {:>14} {:>14}",
                sec, df * 8, dd * 8, uf * 8, ud * 8
            );
        }
        stop.store(true, Ordering::SeqCst);
        let _ = flood.join();

        let (uf, ud, df, dd, ab) = backend.stats().unwrap();
        println!("[shaper_test] totals: down forwarded {:.2} MB, dropped {:.2} MB | up forwarded {:.2} MB, dropped {:.2} MB | app-blocked {:.2} MB",
            df as f64 / 1e6, dd as f64 / 1e6, uf as f64 / 1e6, ud as f64 / 1e6, ab as f64 / 1e6);

        println!("[shaper_test] restoring...");
        if let Err(e) = backend.teardown(&session).await {
            tracing::error!(error = %e, "teardown failed");
        }
        match backend.verify_restoration(session.id, &captured).await {
            Ok(()) => println!("[shaper_test] restoration verified"),
            Err(e) => println!("[shaper_test] RESTORATION FAILED: {e}"),
        }
    });
}
