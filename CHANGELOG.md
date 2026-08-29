# Changelog

All notable changes to BanDen are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
uses semantic versioning.

## [0.1.0] — 2026-08-25

Initial public development snapshot.

### Added

- **Allowlist mode**: cut everything except the apps you pick -
  default-deny enforcement that also stops apps dodging DNS/SNI blocking
  via hardcoded IPs or encrypted DNS
- **Network Map** page: inferred star topology (internet -> gateway ->
  devices) with online/offline state
- **Device drawer**: Overview / Connectivity / History tabs, per-device
  ping + availability + latency history, device-kind picker with
  automatic classification
- **Connection-reset window** and 500 ms ARP poison cadence so blocks
  hold against fast-ARP phones
- **UI**: colored dashboard stat cards, kind icons, activity severity
  filters, allowlist/block/rate-limit session scopes with duration
  presets and rate sliders

- **Core** (`banden-core`): session lifecycle state machine with explicit,
  tested transition table; session manager with safety-critical ordering
  (journal before mutation); recovery manager with durable journal,
  idempotent restoration actions, bounded retries and verification; network
  state classification (`Normal/Modified/Restoring/Unknown`); staged
  emergency-stop orchestration; bounded traffic aggregation (rates, windows,
  per-device and protocol stats); simulation and fault-injection control
  backends.
- **Persistence** (`banden-db`): SQLite schema via embedded migrations;
  repositories for devices, sessions, traffic samples, activity, settings
  and the recovery journal; implements the core `JournalStore` and
  `ActivityLog` ports.
- **Network layer** (`banden-net`): adapter enumeration, subnet derivation,
  IPv4 ARP/neighbor table access, `SendARP` probes with bounded parallel
  sweep, reverse DNS, curated MAC OUI lookup, interface traffic counters,
  ICMP latency probe, traffic monitor with lab-mode simulated flows,
  Win32 restoration executor.
- **Watchdog** (`banden-watchdog`): independent process monitoring heartbeat
  and parent liveness; replays the recovery journal after abnormal
  termination; pure decision logic fully unit-tested.
- **Desktop app** (`banden-app`): Tauri 2 shell with the full command API
  (network, devices, traffic, sessions, emergency stop, activity, system,
  settings, data), versioned events, system tray, global `Ctrl+Alt+X`
  emergency-stop hotkey, close-to-tray, autostart, startup recovery and
  guaranteed exit cleanup.
- **Frontend**: six pages (Dashboard, Devices, Traffic, Controls, Activity,
  Settings), command palette (`Ctrl+K`), staged emergency-stop dialog,
  device detail sheet, session start/stop flows with confirmation, live
  traffic chart (Recharts), dark/light theming.
- **Infrastructure**: CI workflow (fmt, clippy, tests, frontend build),
  deterministic icon generation, documentation set (architecture, safety,
  security, contributing).
