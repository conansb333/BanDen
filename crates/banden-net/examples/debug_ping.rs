//! Wire-level debug probe: verify crafted frames reach the phone and that
//! the phone's answers are visible to a promiscuous capture handle.

use banden_net::rawframe::{parse_mac, PcapLib, RawSender};
use std::net::Ipv4Addr;
use std::sync::Arc;

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let phone_ip: Ipv4Addr = args
        .iter()
        .position(|a| a == "--ip")
        .and_then(|i| args.get(i + 1))
        .expect("--ip required")
        .parse()
        .expect("bad ip");
    let phone_mac = parse_mac(
        args.iter()
            .position(|a| a == "--mac")
            .and_then(|i| args.get(i + 1))
            .expect("--mac required"),
    )
    .unwrap();
    let src_ip_arg = args
        .iter()
        .position(|a| a == "--src-ip")
        .and_then(|i| args.get(i + 1))
        .cloned();

    let lib = Arc::new(PcapLib::load().expect("wpcap"));
    let interfaces = banden_net::list_interfaces().unwrap();
    let sel = banden_net::interface_for_target(&interfaces, phone_ip).expect("no iface");
    let our_mac = parse_mac(sel.mac_address.as_deref().unwrap()).unwrap();
    let our_ip: Ipv4Addr = sel.ipv4.as_deref().unwrap().parse().unwrap();
    let src_ip: Ipv4Addr = src_ip_arg.map(|v| v.parse().unwrap()).unwrap_or(our_ip);
    let device = banden_net::pcap_device_for_target(&lib, phone_ip).expect("device");
    println!("iface: {} | device: {device}", sel.name);

    // Capture handle: watches everything arriving (promiscuous).
    let cap = RawSender::open_ex(Arc::clone(&lib), &device, true, 200).expect("capture");

    println!("injecting pings with ip src={src_ip} dst={phone_ip}");
    // Send handle: crafted pings.
    let send = RawSender::open(Arc::clone(&lib), &device).expect("sender");
    let build = |seq: u16| -> Vec<u8> {
        let mut f: Vec<u8> = Vec::new();
        f.extend_from_slice(&phone_mac); // eth dst
        f.extend_from_slice(&our_mac); // eth src
        f.extend_from_slice(&[0x08, 0x00]);
        f.push(0x45);
        f.push(0x00);
        f.extend_from_slice(&(20u16 + 8 + 32).to_be_bytes());
        f.extend_from_slice(&[0xAB, 0xCD]); // id
        f.extend_from_slice(&[0x00, 0x00]);
        f.push(64);
        f.push(1);
        f.extend_from_slice(&[0x00, 0x00]);
        f.extend_from_slice(&src_ip.octets());
        f.extend_from_slice(&phone_ip.octets());
        let c = internet_checksum(&f[14..34]);
        f[24..26].copy_from_slice(&c.to_be_bytes());
        f.push(8); // echo request
        f.push(0);
        f.extend_from_slice(&[0x00, 0x00]);
        f.extend_from_slice(&[0x12, 0x34]); // ident
        f.extend_from_slice(&seq.to_be_bytes());
        f.extend_from_slice(b"banden-debug-payload-----------"); // 32 bytes
        let c = internet_checksum(&f[34..]);
        f[36..38].copy_from_slice(&c.to_be_bytes());
        f
    };

    // Background: capture for 4 seconds while pinging.
    let cap_thread = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        let mut icmp_from_phone = 0u32;
        let mut any_from_phone = 0u32;
        let mut total = 0u32;
        while std::time::Instant::now() < deadline {
            match cap.recv() {
                Ok(Some(frame)) => {
                    total += 1;
                    // Hexdump any frame involving the phone (ours out, theirs in).
                    if frame.len() >= 34
                        && (&frame[0..6] == phone_mac.as_ref()
                            || &frame[6..12] == phone_mac.as_ref())
                        && frame[12] == 0x08
                        && frame[13] == 0x00
                        && frame[23] == 1
                    {
                        let dir = if &frame[0..6] == phone_mac.as_ref() {
                            "IN "
                        } else {
                            "OUT"
                        };
                        let hex: Vec<String> = frame[..(42.min(frame.len()))]
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect();
                        println!("{dir} {} bytes: {}", frame.len(), hex.join(" "));
                    }
                    if frame.len() >= 14 && &frame[6..12] == phone_mac.as_ref() {
                        any_from_phone += 1;
                        if frame.len() >= 34 && frame[23] == 1 {
                            icmp_from_phone += 1;
                            println!(
                                "phone ICMP frame: eth_dst={:02x?} ip={}.{} -> {} bytes",
                                &frame[0..6],
                                frame[26],
                                frame[27],
                                frame.len()
                            );
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    println!("capture error: {e}");
                    break;
                }
            }
        }
        println!(
            "captured total={total} from_phone={any_from_phone} icmp_from_phone={icmp_from_phone}"
        );
    });

    std::thread::sleep(std::time::Duration::from_millis(300));
    for seq in 1..=5u16 {
        let f = build(seq);
        println!(
            "sending crafted ping seq={seq} ({} bytes) -> {}",
            f.len(),
            phone_ip
        );
        if let Err(e) = send.send(&f) {
            println!("send error: {e}");
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }

    cap_thread.join().unwrap();
}
