//! Tauri command handlers. Thin: they delegate to the core runtime and
//! never contain business or networking logic themselves.

use crate::error::CommandResult;
use crate::state::{self, AppState};
use banden_core::{
    AppSettings, EventCategory, SessionConfig, SessionRecordView, StopReason, SystemStatus,
    TrafficSnapshot,
};
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, State};

// ---------------------------------------------------------------------------
// Network / interfaces
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_network_interfaces() -> CommandResult<Vec<banden_core::InterfaceInfo>> {
    let list = tauri::async_runtime::spawn_blocking(banden_net::list_interfaces)
        .await
        .map_err(|e| crate::error::CommandError::new("join_error", e.to_string()))?
        .map_err(crate::error::CommandError::from)?;
    Ok(list)
}

#[tauri::command]
pub async fn get_network_info(
    state: State<'_, Arc<AppState>>,
) -> CommandResult<banden_core::NetworkSummary> {
    Ok(state.network_summary().await)
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn discover_devices(
    state: State<'_, Arc<AppState>>,
) -> CommandResult<DeviceDiscoveryResult> {
    let discovered = crate::discovery::run_discovery(&state)
        .await
        .map_err(crate::error::CommandError::from)?;
    Ok(DeviceDiscoveryResult {
        discovered,
        devices: state
            .db
            .list_devices()
            .map_err(crate::error::CommandError::from)?,
    })
}

#[derive(Serialize)]
pub struct DeviceDiscoveryResult {
    pub discovered: usize,
    pub devices: Vec<banden_core::NetworkDevice>,
}

/// Device row enriched with the effective kind: a user override when set,
/// otherwise the automatic hostname/OUI classification.
#[derive(Serialize)]
pub struct DeviceView {
    #[serde(flatten)]
    pub device: banden_core::NetworkDevice,
    pub kind: Option<String>,
    pub kind_source: String,
}

fn enrich(state: &Arc<AppState>, devices: Vec<banden_core::NetworkDevice>) -> Vec<DeviceView> {
    let overrides = state.db.device_kind_overrides().unwrap_or_default();
    devices
        .into_iter()
        .map(|d| {
            let manual = overrides.get(&d.mac_address.to_uppercase()).cloned();
            let (kind, kind_source) = match manual {
                Some(k) => (Some(k), "manual".to_string()),
                None => (
                    d.device_type.clone(),
                    if d.device_type.is_some() {
                        "auto"
                    } else {
                        "unknown"
                    }
                    .to_string(),
                ),
            };
            DeviceView {
                device: d,
                kind,
                kind_source,
            }
        })
        .collect()
}

#[tauri::command]
pub async fn get_devices(state: State<'_, Arc<AppState>>) -> CommandResult<Vec<DeviceView>> {
    let devices = state
        .db
        .list_devices()
        .map_err(crate::error::CommandError::from)?;
    Ok(enrich(&state, devices))
}

#[tauri::command]
pub async fn get_device(
    state: State<'_, Arc<AppState>>,
    device_id: i64,
) -> CommandResult<Option<DeviceView>> {
    let device = state
        .db
        .get_device(device_id)
        .map_err(crate::error::CommandError::from)?;
    Ok(device.map(|d| enrich(&state, vec![d]).remove(0)))
}

// ---------------------------------------------------------------------------
// Connectivity (per-device ping + samples + history)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct PingResult {
    pub latency_ms: Option<u64>,
}

#[derive(Serialize)]
pub struct LatencyPoint {
    /// Unix epoch seconds.
    pub ts: i64,
    pub latency_ms: Option<i64>,
}

#[derive(Serialize)]
pub struct ConnectivityReport {
    pub samples: Vec<LatencyPoint>,
    pub sample_count: u64,
    pub ok_count: u64,
    pub availability_pct: f64,
    pub current_latency_ms: Option<i64>,
}

#[tauri::command]
pub async fn ping_device(
    state: State<'_, Arc<AppState>>,
    mac: String,
    ip: String,
) -> CommandResult<PingResult> {
    let target: Ipv4Addr = ip
        .parse()
        .map_err(|e| crate::error::CommandError::new("bad_ip", format!("{e}")))?;
    let rtt = tokio::task::spawn_blocking(move || {
        banden_net::ping(target, Duration::from_millis(1000))
            .ok()
            .flatten()
    })
    .await
    .map_err(|e| crate::error::CommandError::new("join_error", e.to_string()))?;
    state
        .db
        .insert_latency_sample(&mac, &ip, rtt.map(|ms| ms as i64))
        .map_err(crate::error::CommandError::from)?;
    Ok(PingResult { latency_ms: rtt })
}

#[tauri::command]
pub async fn get_device_connectivity(
    state: State<'_, Arc<AppState>>,
    mac: String,
) -> CommandResult<ConnectivityReport> {
    let samples = state.db.latency_samples(&mac, 100).unwrap_or_default();
    let sample_count = samples.len() as u64;
    let ok_count = samples.iter().filter(|(_, ms)| ms.is_some()).count() as u64;
    let availability_pct = if sample_count > 0 {
        (ok_count as f64 / sample_count as f64) * 100.0
    } else {
        0.0
    };
    let current_latency_ms = samples.first().and_then(|(_, ms)| *ms);
    Ok(ConnectivityReport {
        samples: samples
            .into_iter()
            .map(|(ts, latency_ms)| LatencyPoint { ts, latency_ms })
            .collect(),
        sample_count,
        ok_count,
        availability_pct,
        current_latency_ms,
    })
}

#[tauri::command]
pub async fn get_device_events(
    state: State<'_, Arc<AppState>>,
    mac: String,
    ip: String,
    limit: Option<i64>,
) -> CommandResult<Vec<banden_core::ActivityEvent>> {
    state
        .db
        .device_events(&mac, &ip, limit.unwrap_or(100))
        .map_err(crate::error::CommandError::from)
}

#[tauri::command]
pub async fn set_device_kind(
    state: State<'_, Arc<AppState>>,
    mac: String,
    ip: String,
    kind: Option<String>,
) -> CommandResult<()> {
    state
        .db
        .set_device_kind_override(&mac, kind.as_deref())
        .map_err(crate::error::CommandError::from)?;
    let msg = match &kind {
        Some(k) => format!("{ip} device type set to {k}"),
        None => format!("{ip} device type reset to automatic"),
    };
    state
        .runtime
        .record_activity(EventCategory::Info, msg, None)
        .await;
    state
        .app
        .emit(
            banden_core::events::DEVICE_UPDATED,
            serde_json::json!({ "v": 1, "mac": mac }),
        )
        .ok();
    Ok(())
}

// ---------------------------------------------------------------------------
// Traffic
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_traffic_stats(state: State<'_, Arc<AppState>>) -> CommandResult<TrafficSnapshot> {
    Ok(state.latest_snapshot.lock().unwrap().clone())
}

#[derive(Serialize)]
pub struct TrafficHistoryPoint {
    pub ts: chrono::DateTime<chrono::Utc>,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

#[tauri::command]
pub async fn get_traffic_history(
    state: State<'_, Arc<AppState>>,
    window_secs: Option<u64>,
    bucket_secs: Option<i64>,
) -> CommandResult<Vec<TrafficHistoryPoint>> {
    let window = window_secs.unwrap_or(3600).min(24 * 3600);
    let bucket = bucket_secs.unwrap_or(60).clamp(10, 3600);
    let since = chrono::Utc::now() - chrono::Duration::seconds(window as i64);
    let rows = state
        .db
        .traffic_history(since, bucket, None)
        .map_err(crate::error::CommandError::from)?;
    Ok(rows
        .into_iter()
        .map(|(ts, bytes_in, bytes_out)| TrafficHistoryPoint {
            ts,
            bytes_in,
            bytes_out,
        })
        .collect())
}

#[tauri::command]
pub async fn get_device_traffic(
    state: State<'_, Arc<AppState>>,
    mac: String,
) -> CommandResult<DeviceTrafficReport> {
    // Live view first, history second.
    let snapshot = state.latest_snapshot.lock().unwrap().clone();
    let live = snapshot
        .top_devices
        .iter()
        .find(|d| d.mac_address.as_deref() == Some(mac.as_str()))
        .cloned();
    let history = state
        .db
        .device_traffic_history(chrono::Utc::now() - chrono::Duration::hours(24))
        .map_err(crate::error::CommandError::from)?
        .into_iter()
        .find(|(m, _, _)| *m == mac)
        .map(|(_, bytes_in, bytes_out)| HistoricalBytes {
            bytes_in,
            bytes_out,
        });
    Ok(DeviceTrafficReport {
        live,
        bytes_last_24h: history.map(|h| h.bytes_in + h.bytes_out).unwrap_or(0),
    })
}

#[derive(Serialize)]
pub struct DeviceTrafficReport {
    pub live: Option<banden_core::DeviceTraffic>,
    pub bytes_last_24h: u64,
}

struct HistoricalBytes {
    bytes_in: u64,
    bytes_out: u64,
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_sessions(
    state: State<'_, Arc<AppState>>,
) -> CommandResult<Vec<SessionRecordView>> {
    Ok(state.runtime.sessions.list().await)
}

#[tauri::command]
pub async fn get_session(
    state: State<'_, Arc<AppState>>,
    session_id: String,
) -> CommandResult<SessionRecordView> {
    let id = uuid::Uuid::parse_str(&session_id)
        .map_err(|_| crate::error::CommandError::new("bad_request", "Invalid session id"))?;
    state.runtime.sessions.get(id).await.map_err(Into::into)
}

#[tauri::command]
pub async fn start_session(
    state: State<'_, Arc<AppState>>,
    config: SessionConfig,
) -> CommandResult<SessionRecordView> {
    let view = state
        .runtime
        .start_session(config)
        .await
        .map_err(crate::error::CommandError::from)?;
    state
        .db
        .save_session(&view)
        .map_err(crate::error::CommandError::from)?;
    Ok(view)
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StopReasonDto {
    #[default]
    UserRequested,
    DurationElapsed,
}

#[tauri::command]
pub async fn stop_session(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    reason: Option<StopReasonDto>,
) -> CommandResult<SessionRecordView> {
    let id = uuid::Uuid::parse_str(&session_id)
        .map_err(|_| crate::error::CommandError::new("bad_request", "Invalid session id"))?;
    let why = match reason.unwrap_or_default() {
        StopReasonDto::UserRequested => StopReason::UserRequested,
        StopReasonDto::DurationElapsed => StopReason::DurationElapsed,
    };
    let view = state
        .runtime
        .stop_session(id, why)
        .await
        .map_err(crate::error::CommandError::from)?;
    state
        .db
        .save_session(&view)
        .map_err(crate::error::CommandError::from)?;
    Ok(view)
}

// ---------------------------------------------------------------------------
// Emergency stop
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn emergency_stop(
    state: State<'_, Arc<AppState>>,
) -> CommandResult<banden_core::EmergencyStopOutcome> {
    let outcome = state
        .runtime
        .emergency_stop()
        .await
        .map_err(crate::error::CommandError::from)?;
    for view in state.runtime.sessions.list().await {
        let _ = state.db.save_session(&view);
    }
    Ok(outcome)
}

// ---------------------------------------------------------------------------
// Activity & system
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_activity(
    state: State<'_, Arc<AppState>>,
    limit: Option<u64>,
    category: Option<String>,
    search: Option<String>,
) -> CommandResult<Vec<banden_core::ActivityEvent>> {
    let cat = category.and_then(|c| match c.to_uppercase().as_str() {
        "INFO" => Some(EventCategory::Info),
        "WARNING" => Some(EventCategory::Warning),
        "ERROR" => Some(EventCategory::Error),
        "RECOVERY" => Some(EventCategory::Recovery),
        "NETWORK" => Some(EventCategory::Network),
        "SESSION" => Some(EventCategory::Session),
        _ => None,
    });
    Ok(state
        .runtime
        .recent_activity(limit.unwrap_or(200).min(1000), cat, search)
        .await)
}

#[tauri::command]
pub async fn get_system_status(state: State<'_, Arc<AppState>>) -> CommandResult<SystemStatus> {
    Ok(state.system_status().await)
}

#[tauri::command]
pub async fn start_monitoring(state: State<'_, Arc<AppState>>) -> CommandResult<()> {
    state::start_monitoring(&state).await;
    Ok(())
}

#[tauri::command]
pub async fn stop_monitoring(state: State<'_, Arc<AppState>>) -> CommandResult<()> {
    state::stop_monitoring(&state).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_settings(state: State<'_, Arc<AppState>>) -> CommandResult<AppSettings> {
    Ok(state.settings().await)
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, Arc<AppState>>,
    settings: AppSettings,
) -> CommandResult<AppSettings> {
    // Persist first, then apply side effects.
    state
        .db
        .set_settings(
            &serde_json::to_value(&settings).map_err(|e| {
                crate::error::CommandError::new("serialization_error", e.to_string())
            })?,
        )
        .map_err(crate::error::CommandError::from)?;
    *state.settings.write().await = settings.clone();
    state.runtime.set_settings(settings.clone()).await;

    // Watchdog side effect.
    if settings.safety.recovery_watchdog {
        state::spawn_watchdog(&state);
    } else {
        state::kill_watchdog(&state).await;
    }

    // Autostart side effect.
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        if settings.general.start_with_windows {
            let _ = state.app.autolaunch().enable();
        } else {
            let _ = state.app.autolaunch().disable();
        }
    }

    Ok(settings)
}

// ---------------------------------------------------------------------------
// Data maintenance
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn clear_history(state: State<'_, Arc<AppState>>) -> CommandResult<()> {
    state
        .db
        .clear_history()
        .map_err(crate::error::CommandError::from)?;
    state
        .runtime
        .record_activity(EventCategory::Info, "History cleared by user".into(), None)
        .await;
    Ok(())
}

#[tauri::command]
pub async fn purge_old_data(state: State<'_, Arc<AppState>>) -> CommandResult<(usize, usize)> {
    let retention = state.settings().await.data.retention_days;
    state
        .db
        .purge_old(retention)
        .map_err(crate::error::CommandError::from)
}

/// Helper for tests / debugging: is the database reachable?
#[tauri::command]
pub async fn get_data_dir(state: State<'_, Arc<AppState>>) -> CommandResult<String> {
    Ok(state.data_dir.display().to_string())
}
