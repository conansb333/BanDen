//! Standalone ARP-isolation runner (client-side-only poisoning).
//!
//! Uses the same `ArpCutBackend` as the app: forges ARP replies to the
//! TARGET only, claiming the gateway's IP is at this PC's MAC, so the
//! target's internet-bound frames arrive here and are dropped. The
//! gateway's neighbor table is never touched, so nothing else on the
//! network is affected. On Ctrl+C (or after `--seconds`) corrective
//! replies are sent and restoration is verified.
//!
//! Cut:    arp_cut --ip 192.168.8.4 --mac 9C:2E:A1:2C:0A:99
//! Cut 60s: arp_cut --ip ... --mac ... --seconds 60
//! Heal:   arp_cut --restore --ip 192.168.8.1 --mac <router-mac>

use banden_core::{ControlBackend, Session, SessionConfig, SessionStateMachine};
use banden_net::{rawframe, ArpCutBackend, PcapLib};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn main() {
    let mut ip = None;
    let mut mac = None;
    let mut seconds: Option<u64> = None;
    let mut down_mbps: Option<u64> = None;
    let mut up_mbps: Option<u64> = None;
    let mut blocked: Vec<String> = Vec::new();
    let mut allowed: Vec<String> = Vec::new();
    let mut restore = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--ip" => ip = args.next(),
            "--mac" => mac = args.next(),
            "--seconds" => {
                seconds = args.next().and_then(|v| v.parse().ok());
            }
            "--down-mbps" => down_mbps = args.next().and_then(|v| v.parse().ok()),
            "--up-mbps" => up_mbps = args.next().and_then(|v| v.parse().ok()),
            "--block" => {
                if let Some(list) = args.next() {
                    blocked = list.split(',').map(|s| s.trim().to_string()).collect();
                }
            }
            "--allow-apps" => {
                if let Some(list) = args.next() {
                    allowed = list.split(',').map(|s| s.trim().to_string()).collect();
                }
            }
            "--restore" => restore = true,
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(1);
            }
        }
    }
    let ip = ip.expect("--ip <address> required");
    let mac = mac.expect("--mac <address> required");

    if restore {
        // Emergency heal: broadcast truthful gratuitous ARP announcing
        // ip -> mac so poisoned caches re-learn the real mapping.
        let lib = PcapLib::load().expect("wpcap");
        let parsed_ip: std::net::Ipv4Addr = ip.parse().expect("bad ip");
        let parsed_mac = rawframe::parse_mac(&mac).expect("bad mac");
        ArpCutBackend::announce(&lib, parsed_ip, parsed_mac).expect("announce");
        println!("[arp_cut] restoration announcements sent for {ip}");
        return;
    }

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
        .init();

    let backend = ArpCutBackend::new().expect("wpcap must be available");
    let session = Session {
        id: uuid::Uuid::new_v4(),
        config: SessionConfig {
            target_mac: mac,
            target_ip: ip.clone(),
            target_label: Some("manual-cut".into()),
            download_limit_bps: down_mbps.map(|m| m * 1_000_000),
            upload_limit_bps: up_mbps.map(|m| m * 1_000_000),
            blocked_apps: blocked.clone(),
            allowed_apps: allowed.clone(),
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

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_t = Arc::clone(&stop_flag);
    ctrlc_handler(stop_flag_t);

    rt.block_on(async move {
        let captured = backend.prepare(&session).await.expect("prepare");
        tracing::info!(
            actions = captured.actions.len(),
            "restoration journaled (in-memory)"
        );
        backend.apply(&session).await.expect("apply");
        tracing::warn!(target = %ip, "CUT ACTIVE - Ctrl+C or wait for --seconds to restore");

        let deadline =
            seconds.map(|s| std::time::Instant::now() + std::time::Duration::from_secs(s));
        loop {
            if stop_flag.load(Ordering::SeqCst) {
                break;
            }
            if let Some(d) = deadline {
                if std::time::Instant::now() >= d {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        if let Some((uf, ud, df, dd, ab)) = backend.stats() {
            tracing::info!(
                "session counters: down fwd {} B / drop {} B | up fwd {} B / drop {} B | app-blocked {} B",
                df, dd, uf, ud, ab
            );
        }
        tracing::info!("restoring...");
        if let Err(e) = backend.teardown(&session).await {
            tracing::error!(error = %e, "teardown failed");
        }
        match backend.verify_restoration(session.id, &captured).await {
            Ok(()) => tracing::info!("restoration verified - target should be back online"),
            Err(e) => tracing::error!(error = %e, "restoration verification FAILED"),
        }
    });
}

fn ctrlc_handler(flag: Arc<std::sync::atomic::AtomicBool>) {
    std::thread::spawn(move || {
        let sig = ctrlc_support::listener().expect("cannot install Ctrl+C handler");
        sig.recv();
        flag.store(true, Ordering::SeqCst);
    });
}

mod ctrlc_support {
    /// Minimal Ctrl+C listener via the console control handler API.
    pub struct Listener {
        pub rx: std::sync::mpsc::Receiver<()>,
    }
    impl Listener {
        pub fn recv(&self) {
            let _ = self.rx.recv();
        }
    }
    pub fn listener() -> Result<Listener, String> {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel::<()>();
        static TX: std::sync::OnceLock<mpsc::Sender<()>> = std::sync::OnceLock::new();
        TX.set(tx).map_err(|_| "already installed".to_string())?;
        unsafe {
            use windows::Win32::Foundation::BOOL;
            use windows::Win32::System::Console::SetConsoleCtrlHandler;
            unsafe extern "system" fn handler(_event: u32) -> BOOL {
                if let Some(tx) = TX.get() {
                    let _ = tx.send(());
                }
                BOOL(1) // handled; process keeps running until we exit
            }
            SetConsoleCtrlHandler(Some(handler), true).map_err(|e| e.to_string())?;
        }
        Ok(Listener { rx })
    }
}
