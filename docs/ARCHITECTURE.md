# BanDen Architecture

## Design principles

1. **Dangerous logic is pure and tested.** The session state machine,
   recovery manager and emergency-stop orchestration live in `banden-core`
   and depend only on ports (traits). They are unit-tested without hardware,
   including failure injection.
2. **The UI process is not part of the safety story.** Restoration data is
   journaled durably before any system state changes; an independent
   watchdog can finish the job if the app dies.
3. **Aggregate in the backend, summarize to the UI.** No raw packet stream
   crosses the IPC boundary; the frontend receives bounded snapshots.
4. **Boring, explicit state.** Sessions are explicit state machines; network
   state is one of `Normal | Modified | Restoring | Unknown`; `Unknown` is a
   real, surfaced state.

## Crate map

| Crate | Role | Depends on |
|---|---|---|
| `banden-core` | Domain models, session state machine, session manager, recovery manager + journal, emergency-stop orchestration, traffic aggregation, port definitions | — (only serde/tokio/chrono/uuid) |
| `banden-db` | SQLite persistence: migrations, repositories, `JournalStore` + `ActivityLog` implementations | `banden-core`, `rusqlite` |
| `banden-net` | Win32 networking: interfaces (`GetAdaptersAddresses`), ARP table + `SendARP`, discovery orchestration, interface counters (`GetIfEntry2`), ICMP latency, traffic monitor, simulated flow source, restoration executor | `banden-core`, `windows` |
| `banden-watchdog` | Independent recovery process: heartbeat/parent liveness, journal replay | `banden-core`, `banden-db`, `banden-net` |
| `banden-app` | Tauri shell: commands, events, tray, global shortcut, lifecycle, error mapping | all of the above, `tauri` |

## Session lifecycle

```
            prepare ok
 ┌────────┐ ───────────► ┌────────┐
 │Preparing│              │ Active │
 └────┬───┘ ◄──────────── └───┬────┘
      │ cancel (Stopping)    │ stop / fault
      ▼                      ▼
  ┌────────┐            ┌───────────┐    retries exhausted
  │Stopping│───────────►│ Restoring │ ───────────────────► ┌────────┐
  └────────┘            └─────┬─────┘                      │ Failed │
                              ▼                            └────────┘
                        ┌───────────┐
                        │ Verifying │
                        └─────┬─────┘
                 verify ok    │    verify failed
                              ▼                    ▼
                        ┌──────────┐       ┌─────────────────┐
                        │Completed │       │RecoveryRequired │──► Restoring (retry)
                        └──────────┘       └─────────────────┘
```

Rules encoded in `banden-core::session::machine` (single source of truth,
fully unit-tested):

- `Active` can never jump directly to `Completed` — restoration and
  verification are mandatory.
- Terminal states (`Completed`, `Failed`) are absorbing.
- Illegal transitions return an error; nothing is silently coerced.

### Ordering inside `SessionManager::start`

1. validate config; create session in `Preparing`;
2. `ControlBackend::prepare` — capture state, plan restoration actions;
3. `RecoveryManager::register_session` — journal persisted (durable)
   **before** any mutation;
4. `ControlBackend::apply`;
5. transition `Active`; arm duration timer.

`stop` (and the shared `shutdown_session` tail) always runs teardown →
restore → verify, even when earlier steps failed; a single automatic retry
cycle follows a failed verification before the session is marked `Failed`.

## Recovery

- **Journal** (`recovery_journal` table): one row per restoration action,
  states `pending → done | failed`, attempt counter, max 3 attempts.
  Actions are small, self-describing, versioned enums
  (`NoOp`, `ClearNeighborEntry`, `RestoreNeighborEntry`, `StopProcess`).
- **Network state** (`RecoveryManager`): `Normal | Modified | Restoring |
  Unknown`. Set to `Unknown` on restoration/verification failure — the UI
  banner and Dashboard reflect it.
- **Emergency stop** (`CoreRuntime::emergency_stop`): serialized by a lock;
  stages `cancelling → stopping_controllers → restoring_network_state →
  verifying_network_state → completed | recovery_required`, each stage
  emitted as a `recovery_progress` event and rendered step-by-step in the
  dialog.

## Watchdog

`banden-watchdog.exe` is spawned by the app with `--db`, `--heartbeat`,
`--parent-pid`. It polls:

- heartbeat file age (app touches it every 5 s; stale after 15 s),
- parent process liveness (`OpenProcess` + exit code).

When both signals indicate the app is gone, it opens the same SQLite
database, replays pending journal actions through `banden-net`'s restoration
executor, and records the outcome in the activity log. The decision logic is
a pure function covered by tests.

## Traffic pipeline

```
GetIfEntry2 (1 Hz)  ─► TrafficAggregator ─► bounded window (≤300 samples)
Simulation (lab)   ─►   (banden-core)        │
                                            ├─► traffic_update event (UI)
                                            └─► traffic_samples table (10 s)
```

Rate computation uses counter deltas with reset protection (adapter
re-enumeration yields zero rates, never garbage).

## Frontend

- **State**: TanStack Query owns backend state (devices, sessions, status,
  settings, activity); events invalidate queries. Zustand holds only UI
  state: palette, the bounded live-traffic ring, and the emergency-stop
  progress.
- **Navigation**: exactly six pages. Secondary information lives in sheets,
  dialogs and the command palette (`Ctrl+K`).
- **Emergency Stop** is reachable from: toolbar (every page), tray menu,
  global hotkey `Ctrl+Alt+X`, and the command palette. All entry points
  converge on the same staged progress dialog.

## Data flow contract (API)

Commands (section 21 of the spec) are exposed with stable snake_case names;
DTOs serialize snake_case and are mirrored in `src/types`. Events carry a
`v` version field. Technical errors are logged; users see mapped, friendly
messages plus a stable `code`.
