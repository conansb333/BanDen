//! Watchdog process entry point.
//!
//! Spawned by the BanDen app as:
//! `banden-watchdog --db <path> --heartbeat <file> --parent-pid <pid>`
//!
//! Exits 0 after a successful (or empty) recovery; exits 2 when
//! restoration actions remain unhandled.

// Runs detached next to the GUI app; no console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(clippy::needless_raw_string_hashes)]

use banden_watchdog::{run_recovery, WatchdogConfig};
use std::time::Duration;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = match WatchdogConfig::parse_args(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("banden-watchdog: {e}");
            eprintln!(
                "usage: banden-watchdog --db <path> --heartbeat <file> --parent-pid <pid> [--interval-ms N] [--stale-secs N]"
            );
            std::process::exit(1);
        }
    };

    tracing::info!(
        db = %config.db_path.display(),
        heartbeat = %config.heartbeat_path.display(),
        pid = config.parent_pid,
        "watchdog started"
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let code = rt.block_on(async move {
        // Right after launch the heartbeat is missing (the app touches it
        // every 5 s) or stale (left over from a previous run). Allow a
        // grace period: while the parent is not proven dead, treat the
        // watchdog as healthy during it.
        const STARTUP_GRACE: Duration = Duration::from_secs(30);
        let started = std::time::Instant::now();
        loop {
            tokio::time::sleep(config.poll_interval).await;
            let alive = banden_watchdog::parent_alive(config.parent_pid);
            let age = banden_watchdog::heartbeat_age(&config.heartbeat_path);
            let evaluated = banden_watchdog::evaluate(alive, age, config.stale_after);
            let decision = if started.elapsed() < STARTUP_GRACE
                && evaluated != banden_watchdog::Decision::ParentDead
            {
                banden_watchdog::Decision::Healthy
            } else {
                evaluated
            };
            match decision {
                banden_watchdog::Decision::Healthy => continue,
                banden_watchdog::Decision::ParentDead => {
                    tracing::warn!("parent process is gone");
                    break;
                }
                banden_watchdog::Decision::HeartbeatStale => {
                    tracing::warn!("heartbeat is stale");
                    break;
                }
                banden_watchdog::Decision::Unknown => {
                    // Cannot prove the app is healthy; treat conservatively
                    // only if the journal actually holds pending work.
                    break;
                }
            }
        }

        let db = match banden_db::Db::open(&config.db_path) {
            Ok(db) => db,
            Err(e) => {
                tracing::error!(error = %e, "cannot open database; nothing to recover");
                return 0;
            }
        };
        let executor = banden_net::NetRestorationExecutor;
        let summary = run_recovery(&db, &db, &executor).await;
        if summary.failed > 0 {
            2
        } else {
            0
        }
    });

    // Give slow cleanup paths a moment, then exit deterministically.
    std::thread::sleep(Duration::from_millis(250));
    std::process::exit(code);
}
