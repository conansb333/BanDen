//! Measures a host's real IP throughput by promiscuously capturing frames
//! to/from its MAC. Used to verify the shaper's enforced rate.
//!
//! Usage: measure_host --mac 08:00:27:00:00:AA --seconds 30

use banden_net::rawframe::{parse_mac, PcapLib, RawSender};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mac = parse_mac(
        args.iter()
            .position(|a| a == "--mac")
            .and_then(|i| args.get(i + 1))
            .expect("--mac required"),
    )
    .unwrap();
    let secs: u64 = args
        .iter()
        .position(|a| a == "--seconds")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);

    let lib = Arc::new(PcapLib::load().expect("wpcap"));
    let dev = device_name_with(&lib);
    let cap = RawSender::open_ex(Arc::clone(&lib), &dev, true, 300).expect("capture");
    println!("capturing on {dev}");

    println!("{:>4} {:>12} {:>12}", "sec", "in_Mbps", "out_Mbps");
    let mut prev_in: u64 = 0;
    let mut prev_out: u64 = 0;
    for sec in 1..=secs {
        let mut in_bytes: u64 = 0;
        let mut out_bytes: u64 = 0;
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while std::time::Instant::now() < until {
            match cap.recv() {
                Ok(Some(frame)) => {
                    if frame.len() < 34 || frame[12] != 0x08 || frame[13] != 0x00 {
                        continue;
                    }
                    if &frame[0..6] == mac.as_ref() {
                        in_bytes += frame.len() as u64;
                    } else if &frame[6..12] == mac.as_ref() {
                        out_bytes += frame.len() as u64;
                    }
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
        // delta vs previous second's cumulative counters
        println!(
            "{:>4} {:>12.2} {:>12.2}",
            sec,
            (in_bytes.saturating_sub(prev_in)) as f64 * 8.0 / 1e6,
            (out_bytes.saturating_sub(prev_out)) as f64 * 8.0 / 1e6
        );
        prev_in = in_bytes;
        prev_out = out_bytes;
    }
}

fn device_name_with(lib: &Arc<PcapLib>) -> String {
    let lib = Arc::clone(lib);
    let interfaces = banden_net::list_interfaces().unwrap();
    let sel = interfaces
        .iter()
        .find(|i| i.is_up && !i.is_loopback && i.gateway.is_some())
        .expect("no usable interface");
    banden_net::pcap_device_for_target(&lib, sel.ipv4.as_deref().unwrap().parse().unwrap())
        .expect("device")
}
