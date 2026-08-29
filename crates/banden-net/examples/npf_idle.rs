//! NPF-impact isolation: opens capture + send handles on the adapter and
//! does NOTHING else. If gateway pings degrade while these handles are
//! open, the loss is caused by the NPF driver's presence in the stack,
//! not by any BanDen traffic.

use banden_net::rawframe::PcapLib;
use std::sync::Arc;

fn main() {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let lib = Arc::new(PcapLib::load().expect("wpcap"));
    let interfaces = banden_net::list_interfaces().unwrap();
    let sel = interfaces
        .iter()
        .find(|i| i.is_up && !i.is_loopback && i.gateway.is_some())
        .expect("no usable interface");
    let device =
        banden_net::pcap_device_for_target(&lib, sel.ipv4.as_deref().unwrap().parse().unwrap())
            .expect("device");

    println!("[npf_idle] opening handles on {device} for {secs}s (doing nothing)...");
    let _cap = banden_net::rawframe::RawSender::open_ex(Arc::clone(&lib), &device, true, 400)
        .expect("capture");
    let _send = banden_net::rawframe::RawSender::open(Arc::clone(&lib), &device).expect("send");
    std::thread::sleep(std::time::Duration::from_secs(secs));
    println!("[npf_idle] done - handles closed");
}
