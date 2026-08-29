//! Application state: the composition root wiring the core runtime,
//! database, traffic monitor and watchdog together.

use banden_core::{AppSettings, NetworkSummary, SystemStatus, TrafficSample, TrafficSnapshot};
use banden_db::Db;
use banden_net::{TrafficHooks, TrafficMonitor, TrafficMonitorConfig};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;

pub struct AppState {
    pub app: AppHandle,
    pub runtime: Arc<banden_core::CoreRuntime>,
    pub db: Arc<Db>,
    pub data_dir: PathBuf,
    pub heartbeat_path: PathBuf,
    /// Sync mutex: never held across awaits; kill/wait are quick syscalls.
    pub watchdog: Mutex<Option<std::process::Child>>,
    pub selected_interface: RwLock<Option<banden_core::InterfaceInfo>>,
    pub monitor: RwLock<Option<Arc<TrafficMonitor>>>,
    /// Latest snapshot cache shared with the monitor's sync callbacks.
    pub latest_snapshot: Arc<Mutex<TrafficSnapshot>>,
    pub latency_ms: RwLock<Option<u64>>,
    pub settings: RwLock<AppSettings>,
    /// Consecutive discovery misses per MAC; a device is marked offline
    /// only after two misses in a row (phones in power-save miss probes).
    pub discovery_misses: Mutex<std::collections::HashMap<String, u32>>,
}

impl AppState {
    pub async fn settings(&self) -> AppSettings {
        self.settings.read().await.clone()
    }

    pub async fn network_summary(&self) -> NetworkSummary {
        let iface = self.selected_interface.read().await;
        match iface.as_ref() {
            Some(i) => NetworkSummary {
                interface_name: i.friendly_name.clone().or(Some(i.name.clone())),
                interface_id: Some(i.id.clone()),
                ipv4: i.ipv4.clone(),
                cidr: i.cidr.clone(),
                gateway: i.gateway.clone(),
                connection_status: if i.is_up {
                    banden_core::ConnectionStatus::Connected
                } else {
                    banden_core::ConnectionStatus::Disconnected
                },
            },
            None => NetworkSummary::default(),
        }
    }

    pub async fn system_status(&self) -> SystemStatus {
        let settings = self.settings().await;
        let (device_count, online_devices) = self.db.device_counts().unwrap_or((0, 0));
        let snapshot = self.latest_snapshot.lock().unwrap().clone();
        let watchdog_running = self.watchdog.lock().unwrap().is_some();
        let monitor_active = self.monitor.read().await.is_some();
        let inputs = banden_core::StatusInputs {
            network: self.network_summary().await,
            device_count,
            online_devices,
            capture_source: capture_source_label(monitor_active, &settings),
            watchdog_running,
            latency_ms: *self.latency_ms.read().await,
            download_rate_bps: snapshot.total.download_rate_bps,
            upload_rate_bps: snapshot.total.upload_rate_bps,
        };
        self.runtime.status(inputs).await
    }
}

fn capture_source_label(monitor_active: bool, _settings: &AppSettings) -> String {
    if !monitor_active {
        "inactive".to_string()
    } else {
        "interface counters".to_string()
    }
}

/// Bridges traffic monitor output to the UI (events), the snapshot cache
/// and periodic persistence.
pub struct AppTrafficHooks {
    pub app: AppHandle,
    pub db: Arc<Db>,
    /// Shared with `AppState.latest_snapshot`.
    pub snapshot_cache: Arc<Mutex<TrafficSnapshot>>,
}

impl TrafficHooks for AppTrafficHooks {
    fn on_snapshot(&self, snapshot: &TrafficSnapshot) {
        *self.snapshot_cache.lock().unwrap() = snapshot.clone();
        if let Err(e) = self.app.emit(banden_core::events::TRAFFIC_UPDATE, snapshot) {
            tracing::warn!(error = %e, "failed to emit traffic_update");
        }
    }

    fn on_persist(&self, snapshot: &TrafficSnapshot) {
        let latest = TrafficSample {
            timestamp: snapshot.timestamp,
            bytes_in: snapshot.total.bytes_in,
            bytes_out: snapshot.total.bytes_out,
            packets_in: snapshot.total.packets_in,
            packets_out: snapshot.total.packets_out,
            download_rate_bps: snapshot.total.download_rate_bps,
            upload_rate_bps: snapshot.total.upload_rate_bps,
        };
        let db = self.db.clone();
        let devices: Vec<(String, banden_core::DeviceTraffic)> = snapshot
            .top_devices
            .iter()
            .filter_map(|d| d.mac_address.clone().map(|m| (m, d.clone())))
            .collect();
        // Persist on the async runtime; must not block the monitor loop.
        tauri::async_runtime::spawn(async move {
            let ts = latest.timestamp;
            if let Err(e) = db.insert_traffic_sample(
                ts,
                None,
                latest.bytes_in,
                latest.bytes_out,
                latest.packets_in,
                latest.packets_out,
            ) {
                tracing::warn!(error = %e, "failed to persist traffic sample");
            }
            for (mac, dev) in devices {
                let _ = db.insert_traffic_sample(
                    ts,
                    Some(&mac),
                    dev.bytes_in,
                    dev.bytes_out,
                    dev.packets_in,
                    dev.packets_out,
                );
            }
        });
    }
}

/// Start (or restart) traffic monitoring for the selected interface.
pub async fn start_monitoring(state: &Arc<AppState>) {
    let _settings = state.settings().await;
    let Some(iface) = state.selected_interface.read().await.clone() else {
        tracing::warn!("no interface selected; monitoring not started");
        return;
    };
    stop_monitoring(state).await;

    let config = TrafficMonitorConfig {
        if_index: iface.if_index.unwrap_or(0),
        mode: banden_net::CaptureMode::CountersOnly,
        ..Default::default()
    };
    let hooks = Arc::new(AppTrafficHooks {
        app: state.app.clone(),
        db: state.db.clone(),
        snapshot_cache: state.latest_snapshot.clone(),
    });
    let monitor = TrafficMonitor::new(config, hooks);
    *state.monitor.write().await = Some(monitor);
    tracing::info!("traffic monitoring started");
}

pub async fn stop_monitoring(state: &Arc<AppState>) {
    if let Some(m) = state.monitor.write().await.take() {
        m.stop();
        tracing::info!("traffic monitoring stopped");
    }
}

/// Touch the heartbeat file so the watchdog knows we are alive.
pub fn touch_heartbeat(path: &std::path::Path) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::File::create(path) {
        let _ = writeln!(f, "{}", chrono::Utc::now().timestamp());
    }
}

/// Spawn the watchdog process next to the current executable, if present.
pub fn spawn_watchdog(state: &Arc<AppState>) {
    let mut guard = state.watchdog.lock().unwrap();
    if guard.is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(watchdog_exe) = exe
        .parent()
        .map(|d| d.join("banden-watchdog.exe"))
        .filter(|p| p.exists())
    else {
        tracing::warn!(
            "watchdog binary not found next to the app; independent recovery unavailable"
        );
        return;
    };
    let pid = std::process::id();
    let db = state.data_dir.join("banden.db");
    let hb = state.heartbeat_path.clone();
    match std::process::Command::new(watchdog_exe)
        .arg("--db")
        .arg(&db)
        .arg("--heartbeat")
        .arg(&hb)
        .arg("--parent-pid")
        .arg(pid.to_string())
        .spawn()
    {
        Ok(child) => {
            tracing::info!(pid = child.id(), "watchdog started");
            *guard = Some(child);
        }
        Err(e) => tracing::warn!(error = %e, "failed to spawn watchdog"),
    }
}

pub async fn kill_watchdog(state: &Arc<AppState>) {
    if let Some(mut child) = state.watchdog.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
