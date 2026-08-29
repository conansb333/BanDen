//! BanDen desktop application (Tauri shell).
//!
//! Responsibilities are deliberately narrow:
//! - compose the core runtime with real ports (SQLite, Win32 networking),
//! - expose typed Tauri commands and events,
//! - own the application lifecycle: startup recovery, tray, global
//!   emergency-stop shortcut and *guaranteed* shutdown cleanup.

pub mod commands;
pub mod discovery;
pub mod error;
pub mod sink;
pub mod state;

use banden_core::{
    AppSettings, CoreRuntime, EventCategory, RecoveryManager, SessionManager,
    SimulatedControlBackend, SystemClock, TrafficSnapshot,
};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_global_shortcut::ShortcutState;
use tokio::sync::RwLock;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts(["Ctrl+Alt+X"])
                .expect("valid shortcut")
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        trigger_emergency_stop(app.clone());
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            commands::get_network_interfaces,
            commands::get_network_info,
            commands::discover_devices,
            commands::get_devices,
            commands::get_device,
            commands::set_device_kind,
            commands::ping_device,
            commands::get_device_connectivity,
            commands::get_device_events,
            commands::get_traffic_stats,
            commands::get_traffic_history,
            commands::get_device_traffic,
            commands::get_sessions,
            commands::get_session,
            commands::start_session,
            commands::stop_session,
            commands::emergency_stop,
            commands::get_activity,
            commands::get_system_status,
            commands::start_monitoring,
            commands::stop_monitoring,
            commands::get_settings,
            commands::update_settings,
            commands::clear_history,
            commands::purge_old_data,
            commands::get_data_dir,
        ])
        .setup(|app| {
            setup(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.app_handle().state::<Arc<state::AppState>>();
                let minimize = tauri::async_runtime::block_on(async {
                    state.settings().await.general.minimize_to_tray
                });
                if minimize {
                    tracing::info!("minimizing to tray instead of closing");
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build BanDen")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                tracing::info!("application exiting; running shutdown cleanup");
                shutdown(app);
            }
        });
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let data_dir = handle
        .path()
        .app_data_dir()
        .expect("app data dir resolvable");
    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join("banden.db");
    let heartbeat_path = data_dir.join("banden.heartbeat");

    tauri::async_runtime::block_on(async {
        let db = Arc::new(banden_db::Db::open(&db_path)?);

        // Settings: stored value or defaults. Lab mode was removed: the
        // app always uses the real ARP-isolation backend. The simulation
        // backends remain in banden-core for tests only.
        let settings: AppSettings = db
            .get_settings()
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        // Core wiring. Real ARP-isolation backend; if the platform cannot
        // provide one (no Npcap adapter, etc.) the app fails loudly in the
        // log and falls back to simulation rather than pretending.
        let backend: Arc<dyn banden_core::ControlBackend> = match banden_net::ArpCutBackend::new() {
            Ok(b) => {
                tracing::warn!("REAL control backend active (ARP isolation)");
                b
            }
            Err(e) => {
                tracing::error!(error = %e, "ARP backend unavailable; falling back to simulation");
                Arc::new(SimulatedControlBackend::new())
            }
        };
        let sink = Arc::new(sink::TauriEventSink::new(handle.clone(), db.clone()));
        let clock = Arc::new(SystemClock);
        let recovery = RecoveryManager::with_executor(
            backend.clone(),
            Arc::new(banden_net::NetRestorationExecutor),
            sink.clone(),
            db.clone(),
            clock.clone(),
        );
        let sessions = SessionManager::new(
            backend.clone(),
            recovery.clone(),
            sink.clone(),
            clock.clone(),
        );
        let runtime = CoreRuntime::new(
            backend,
            recovery,
            sessions,
            db.clone(),
            sink,
            db.clone(),
            clock,
            settings.clone(),
        );

        let app_state = Arc::new(state::AppState {
            app: handle.clone(),
            runtime: runtime.clone(),
            db,
            data_dir: data_dir.clone(),
            heartbeat_path: heartbeat_path.clone(),
            watchdog: Mutex::new(None),
            selected_interface: RwLock::new(None),
            monitor: RwLock::new(None),
            latest_snapshot: Arc::new(Mutex::new(TrafficSnapshot::default())),
            latency_ms: RwLock::new(None),
            settings: RwLock::new(settings.clone()),
            discovery_misses: Mutex::new(std::collections::HashMap::new()),
        });
        app.manage(app_state.clone());

        // Session-record reconciler: terminal transitions that happen inside
        // the runtime (duration cap, recovery failures) never pass through
        // the start/stop commands, so their DB rows would stay stale
        // (e.g. stuck "active"). Upsert the runtime's view periodically.
        {
            let rt = runtime.clone();
            let db2 = app_state.db.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    for view in rt.sessions.list().await {
                        let _ = db2.save_session(&view);
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
            });
        }

        // Recover anything a previous (crashed) run left behind, then close
        // phantom session rows (in-memory sessions do not survive restarts;
        // the journal above is the source of truth for restoration).
        let notes = runtime.startup_recover().await;
        match app_state.db.mark_stale_sessions_interrupted() {
            Ok(n) if n > 0 => {
                tracing::warn!(closed = n, "marked stale session rows as interrupted");
                runtime
                    .record_activity(
                        EventCategory::Recovery,
                        format!("{n} session(s) from a previous run were marked interrupted"),
                        None,
                    )
                    .await;
            }
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "failed to close stale sessions"),
        }
        for note in notes {
            runtime
                .record_activity(EventCategory::Error, note, None)
                .await;
        }

        // Fast startup path: pick the interface and start the counter-based
        // monitor immediately so the window shows live data right away. The
        // subnet sweep (which can take tens of seconds for offline hosts)
        // runs in the background; monitoring restarts once when it finishes
        // so lab-mode flows can be attributed to the discovered devices.
        {
            let interfaces = tokio::task::spawn_blocking(banden_net::list_interfaces)
                .await
                .map_err(|e| format!("join error: {e}"))??;
            let selected = banden_net::select_interface(
                &interfaces,
                settings.network.default_interface.as_deref(),
            );
            if let Some(sel) = selected {
                *app_state.selected_interface.write().await = Some(sel);
            }
        }
        state::start_monitoring(&app_state).await;
        discovery::spawn_periodic_tasks(app_state.clone());
        if settings.safety.recovery_watchdog {
            state::spawn_watchdog(&app_state);
        }
        tauri::async_runtime::spawn(async move {
            if let Err(e) = discovery::run_discovery(&app_state).await {
                tracing::warn!(error = %e, "initial discovery failed");
            } else {
                // Restart once so simulated per-device flows use the
                // discovered registry.
                state::start_monitoring(&app_state).await;
            }
        });

        runtime
            .record_activity(EventCategory::Info, "BanDen started".into(), None)
            .await;
        Ok::<(), Box<dyn std::error::Error>>(())
    })?;

    build_tray(app)?;
    Ok(())
}

fn build_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{MenuBuilder, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let show = MenuItem::with_id(app, "show", "Open BanDen", true, None::<&str>)?;
    let estop = MenuItem::with_id(app, "estop", "EMERGENCY STOP", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = MenuBuilder::new(app)
        .item(&show)
        .separator()
        .item(&estop)
        .separator()
        .item(&quit)
        .build()?;

    TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "estop" => trigger_emergency_stop(app.clone()),
            "quit" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show(); // ensure close-to-tray doesn't swallow exit
                }
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;
    Ok(())
}

/// Trigger the emergency-stop pipeline from outside the UI (hotkey/tray)
/// and tell the UI what is happening so it can show the staged progress.
fn trigger_emergency_stop(app: AppHandle) {
    tracing::warn!("emergency stop requested via global shortcut/tray");
    let _ = app.emit("emergency_stop_requested", serde_json::json!({ "v": 1 }));
    tauri::async_runtime::spawn(async move {
        let state = app.state::<Arc<state::AppState>>();
        let outcome = state.runtime.emergency_stop().await;
        let payload = match &outcome {
            Ok(o) => serde_json::json!({ "v": 1, "outcome": o }),
            Err(e) => serde_json::json!({ "v": 1, "error": e.to_string() }),
        };
        let _ = app.emit("emergency_stop_result", payload);
    });
}

/// Synchronous final cleanup on app exit. Runs the graceful session
/// shutdown so network state is restored even when the user quits
/// mid-session.
fn shutdown(app: &AppHandle) {
    let state = app.state::<Arc<state::AppState>>();
    tauri::async_runtime::block_on(async {
        if state.settings().await.safety.automatic_cleanup {
            let results = state
                .runtime
                .shutdown_all(banden_core::StopReason::UserRequested)
                .await;
            for r in results {
                if let Err(e) = r {
                    tracing::error!(error = %e, "session cleanup failed during exit");
                }
            }
        }
        state::stop_monitoring(&state).await;
        state::kill_watchdog(&state).await;
        // Remove the heartbeat so a lingering watchdog (if any) sees the
        // file stale only after journal work — by then the journal is clean.
        let _ = std::fs::remove_file(&state.heartbeat_path);
    });
}
