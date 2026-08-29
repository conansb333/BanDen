//! Discovery orchestration: merges network-layer discovery reports into
//! the persistent device registry and notifies the UI.

use banden_core::{EventCategory, NetworkDevice};
use std::sync::Arc;
use tauri::Emitter;

use crate::state::AppState;

/// One full discovery cycle: enumerate adapters, select, sweep, merge.
pub async fn run_discovery(state: &Arc<AppState>) -> Result<usize, banden_net::NetError> {
    let settings = state.settings().await;
    let interfaces = banden_net::list_interfaces()?;
    let selected =
        banden_net::select_interface(&interfaces, settings.network.default_interface.as_deref());

    if let Some(sel) = &selected {
        let changed = {
            let mut guard = state.selected_interface.write().await;
            let prev = guard.clone();
            *guard = Some(sel.clone());
            prev.as_ref() != Some(sel)
        };
        if changed {
            tracing::info!(iface = %sel.name, "active interface changed");
            state
                .runtime
                .record_activity(
                    EventCategory::Network,
                    format!(
                        "Active network interface is now {}",
                        sel.friendly_name.as_deref().unwrap_or(&sel.name)
                    ),
                    None,
                )
                .await;
            state
                .app
                .emit(
                    "network_state_changed",
                    serde_json::json!({
                        "v": 1,
                        "reason": "interface_changed",
                        "interface": sel.name,
                    }),
                )
                .ok();
        }
    }

    let Some(sel) = selected else {
        state
            .runtime
            .push_warning(
                "no_interface",
                "No usable network interface was detected.".into(),
            )
            .await;
        return Ok(0);
    };
    state.runtime.clear_warning("no_interface").await;

    // Never register our own adapters or junk zero-MACs as devices, and
    // heal the local PC's registry row (ARP conflicts during control
    // sessions can transiently map a local MAC to a foreign IP, which
    // would otherwise hijack this row).
    let local_macs: Vec<String> = interfaces
        .iter()
        .filter_map(|i| i.mac_address.as_deref().map(str::to_uppercase))
        .collect();
    let _ = state.db.delete_zero_mac_devices();
    let now = chrono::Utc::now();
    for (i, mac) in interfaces
        .iter()
        .filter_map(|i| i.mac_address.as_deref().map(|m| (i, m.to_uppercase())))
    {
        // Adapters without a gateway (VirtualBox host-only, etc.) are not
        // on the LAN - skip and purge any stale row for them.
        if i.gateway.is_none() {
            let _ = state.db.delete_device(&mac);
            continue;
        }
        if let Some(ip) = &i.ipv4 {
            let mut dev = NetworkDevice {
                id: 0,
                mac_address: mac.clone(),
                ip_address: ip.clone(),
                hostname: Some("This PC".into()),
                vendor: None,
                device_type: Some("this PC".into()),
                status: banden_core::DeviceStatus::Online,
                first_seen: now,
                last_seen: now,
            };
            if let Ok(id) = state.db.upsert_device(&dev) {
                dev.id = id;
            }
            let _ = dev;
        }
    }

    let report = banden_net::DiscoveryService::discover(sel).await?;
    if report.sweep_skipped {
        state
            .runtime
            .push_warning(
                "sweep_skipped",
                "Subnet is larger than the sweep limit; using the ARP table only.".into(),
            )
            .await;
    } else {
        state.runtime.clear_warning("sweep_skipped").await;
    }

    let now = chrono::Utc::now();
    let existing: Vec<NetworkDevice> = state.db.list_devices().unwrap_or_default();
    let mut discovered_events = 0usize;

    for d in &report.devices {
        let mac_upper = d.mac.to_uppercase();
        if mac_upper == "00:00:00:00:00:00" || local_macs.contains(&mac_upper) {
            let _ = state.db.delete_device(&mac_upper);
            continue;
        }
        let prev = existing.iter().find(|e| e.mac_address == mac_upper);
        // vendor + type are DERIVED, not observed: recompute every cycle
        // against the best-known hostname so stale labels (like the old
        // universal "desktop") self-correct.
        let best_host = d
            .hostname
            .as_deref()
            .or_else(|| prev.and_then(|p| p.hostname.as_deref()));
        let resolved = banden_net::resolve_vendor(&mac_upper, best_host);
        let resolved_type =
            banden_net::guess_device_type(&mac_upper, best_host).map(|s| s.to_string());
        let device = NetworkDevice {
            id: prev.map(|p| p.id).unwrap_or(0),
            mac_address: mac_upper,
            ip_address: d.ip.to_string(),
            hostname: d
                .hostname
                .clone()
                .or_else(|| prev.and_then(|p| p.hostname.clone())),
            vendor: resolved.vendor,
            device_type: resolved_type,
            status: banden_core::DeviceStatus::Online,
            first_seen: prev.map(|p| p.first_seen).unwrap_or(now),
            last_seen: now,
        };
        match state.db.upsert_device(&device) {
            Ok(id) => {
                let mut stored = device.clone();
                stored.id = id;
                if prev.is_none() {
                    discovered_events += 1;
                    state
                        .app
                        .emit(
                            banden_core::events::DEVICE_DISCOVERED,
                            serde_json::json!({ "v": 1, "device": stored }),
                        )
                        .ok();
                    state
                        .runtime
                        .record_activity(
                            EventCategory::Info,
                            format!(
                                "Device discovered: {} ({})",
                                stored.label(),
                                stored.ip_address
                            ),
                            None,
                        )
                        .await;
                } else {
                    // A known device that just reappeared after being marked
                    // offline is a liveness transition worth surfacing (and
                    // feeds the per-device History tab).
                    if prev.map(|p| p.status) == Some(banden_core::DeviceStatus::Offline) {
                        state
                            .runtime
                            .record_activity(
                                EventCategory::Info,
                                format!("{} is back online", device.ip_address),
                                Some(serde_json::json!({
                                    "mac": device.mac_address,
                                    "ip": device.ip_address,
                                })),
                            )
                            .await;
                    }
                    state
                        .app
                        .emit(
                            banden_core::events::DEVICE_UPDATED,
                            serde_json::json!({ "v": 1, "device": stored }),
                        )
                        .ok();
                }
            }
            Err(e) => tracing::warn!(error = %e, mac = %device.mac_address, "upsert failed"),
        }
    }

    // Mark previously-online devices as offline only after three
    // consecutive missed discovery cycles AND a stale last-seen timestamp:
    // phones in power-save routinely miss several ARP probes in a row
    // while still being connected to the network, and users see those
    // false OFFLINE rows. Devices that are the target of an active
    // control session are never marked offline - isolation deliberately
    // disturbs their reachability probes.
    let seen_macs: Vec<String> = report
        .devices
        .iter()
        .map(|d| d.mac.to_uppercase())
        .collect();
    let active_targets: Vec<String> = state
        .runtime
        .sessions
        .non_terminal_sessions()
        .await
        .iter()
        .map(|s| s.config.target_mac.to_uppercase())
        .collect();
    // Decide which devices crossed the offline threshold while holding the
    // miss counter, then perform the (async) transition work after the
    // guard is gone - std MutexGuards are not Send.
    let mut to_offline: Vec<banden_core::NetworkDevice> = Vec::new();
    {
        let mut misses = state.discovery_misses.lock().unwrap();
        for prev in &existing {
            if prev.status == banden_core::DeviceStatus::Online
                && !seen_macs.contains(&prev.mac_address)
            {
                if active_targets.contains(&prev.mac_address) {
                    misses.remove(&prev.mac_address);
                    continue;
                }
                let count = misses.entry(prev.mac_address.clone()).or_insert(0);
                *count += 1;
                let stale_secs = (now - prev.last_seen).num_seconds();
                if *count >= 3 && stale_secs >= 180 {
                    misses.remove(&prev.mac_address);
                    to_offline.push(prev.clone());
                }
            } else {
                misses.remove(&prev.mac_address);
            }
        }
    }
    for mut fixed in to_offline {
        let _ = state.db.mark_offline(&fixed.mac_address, now);
        // Recompute derived fields for the now-offline device so
        // stale labels (old "desktop" defaults) self-correct.
        let resolved = banden_net::resolve_vendor(&fixed.mac_address, fixed.hostname.as_deref());
        let resolved_type =
            banden_net::guess_device_type(&fixed.mac_address, fixed.hostname.as_deref())
                .map(|s| s.to_string());
        fixed.vendor = resolved.vendor;
        fixed.device_type = resolved_type;
        fixed.status = banden_core::DeviceStatus::Offline;
        let _ = state.db.upsert_device(&fixed);
        state
            .runtime
            .record_activity(
                EventCategory::Warning,
                format!("{} stopped responding", fixed.ip_address),
                Some(serde_json::json!({
                    "mac": fixed.mac_address,
                    "ip": fixed.ip_address,
                })),
            )
            .await;
        state
            .app
            .emit(
                banden_core::events::DEVICE_UPDATED,
                serde_json::json!({ "v": 1, "mac": fixed.mac_address, "status": "offline" }),
            )
            .ok();
    }

    Ok(discovered_events)
}

/// Periodic discovery + latency loop; runs until the runtime shuts down.
pub fn spawn_periodic_tasks(state: Arc<AppState>) {
    // Discovery cycle.
    let s = state.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let interval = s.settings().await.network.discovery_interval_secs.max(5);
            if tokio::time::timeout(
                std::time::Duration::from_secs(interval),
                s.runtime.shutdown_token().cancelled(),
            )
            .await
            .is_ok()
            {
                break;
            }
            if !s.settings().await.network.monitoring_enabled {
                continue;
            }
            let _ = run_discovery(&s).await;
        }
    });

    // Latency probe against the gateway.
    let s = state.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            if tokio::time::timeout(
                std::time::Duration::from_secs(10),
                s.runtime.shutdown_token().cancelled(),
            )
            .await
            .is_ok()
            {
                break;
            }
            let gateway = s
                .selected_interface
                .read()
                .await
                .as_ref()
                .and_then(|i| i.gateway.clone())
                .and_then(|g| g.parse().ok());
            if let Some(gw) = gateway {
                let rtt = tokio::task::spawn_blocking(move || {
                    banden_net::ping(gw, std::time::Duration::from_millis(1500))
                        .ok()
                        .flatten()
                })
                .await
                .ok()
                .flatten();
                *s.latency_ms.write().await = rtt;
            } else {
                *s.latency_ms.write().await = None;
            }
        }
    });

    // Heartbeat for the watchdog.
    let s = state.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            if tokio::time::timeout(
                std::time::Duration::from_secs(5),
                s.runtime.shutdown_token().cancelled(),
            )
            .await
            .is_ok()
            {
                break;
            }
            crate::state::touch_heartbeat(&s.heartbeat_path);
        }
    });

    // Per-device reachability sampling: one ICMP echo per known device per
    // minute feeds the Connectivity tab (samples, availability %, current
    // latency). Runs sequentially with a short timeout so a dead address
    // costs 700 ms, not a flood.
    let s = state.clone();
    tauri::async_runtime::spawn(async move {
        let mut last_prune = std::time::Instant::now() - std::time::Duration::from_secs(3600);
        loop {
            if tokio::time::timeout(
                std::time::Duration::from_secs(60),
                s.runtime.shutdown_token().cancelled(),
            )
            .await
            .is_ok()
            {
                break;
            }
            let devices = s.db.list_devices().unwrap_or_default();
            for d in devices {
                if s.runtime.shutdown_token().is_cancelled() {
                    break;
                }
                let ip = match d.ip_address.parse() {
                    Ok(ip) => ip,
                    Err(_) => continue,
                };
                let rtt = tokio::task::spawn_blocking(move || {
                    banden_net::ping(ip, std::time::Duration::from_millis(700))
                        .ok()
                        .flatten()
                })
                .await
                .ok()
                .flatten();
                let _ = s.db.insert_latency_sample(
                    &d.mac_address,
                    &d.ip_address,
                    rtt.map(|ms| ms as i64),
                );
            }
            if last_prune.elapsed() >= std::time::Duration::from_secs(3600) {
                let _ = s.db.prune_latency_samples(7);
                last_prune = std::time::Instant::now();
            }
        }
    });
}
