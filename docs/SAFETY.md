# BanDen Safety Design

This document is the contract behind BanDen's most important requirement:

> **BanDen must never rely on the UI process remaining alive in order to
> restore network state.**

## The eight-step control contract

Every control operation passes through:

1. **Initialization** — session record created in `Preparing`.
2. **State capture** — `ControlBackend::prepare` records everything needed
   to undo the operation (before anything is modified).
3. **Lifecycle registration** — restoration actions are written to the
   durable recovery journal (SQLite). This happens **before** `apply`.
4. **Active state** — the backend applies the control; the session moves to
   `Active`.
5. **Controlled shutdown** — teardown is idempotent and runs on every stop
   path: user stop, duration elapsed, fault, emergency stop, application
   exit.
6. **Restoration** — journal actions execute idempotently with bounded
   retries (max 3), regardless of teardown success.
7. **Verification** — the backend must *prove* the system matches the
   captured state. The journal must also be empty of pending work.
8. **Failure recovery** — verification/restore failure moves the session to
   `RecoveryRequired` (drives one automatic retry cycle), then `Failed` if
   still broken; global network state becomes `Unknown`, which the UI
   surfaces prominently. **Restoration success is never assumed.**

## Emergency Stop

One action, five visible stages, no fake spinner-then-success:

```
Cancelling active operations        ✓
Stopping control engines            ✓
Restoring network state             ✓
Verifying network state             …
─────────────────────────────────────
Completed / Recovery required
```

Entry points: toolbar button on every page (destructive styling +
confirmation dialog), system-tray menu, global hotkey `Ctrl+Alt+X`, command
palette. The pipeline is serialized (double-press cannot interleave restore
passes) and reports its exact outcome — including per-failure reasons — to
the UI and the activity log.

## Watchdog

The watchdog is a **separate process** (`banden-watchdog.exe`) spawned at
startup when `safety.recovery_watchdog` is enabled (default). It holds no
application state: it watches a heartbeat file (touched every 5 s, stale
after 15 s) and the parent process handle. On abnormal termination it
replays the pending recovery journal and appends a durable activity record.

The watchdog intentionally does *not* own the whole application — its sole
job is detecting abnormal termination and protecting network-state
restoration.

## Cleanup coverage

Restoration runs during:

- normal application shutdown (`RunEvent::Exit` → `shutdown_all`),
- session termination (user stop, duration elapsed),
- cancellation (stop during `Preparing`),
- recoverable exceptions (failure paths run restore + verify),
- unexpected application termination (watchdog journal replay),
- startup of the next run (`startup_recover` sweeps leftover journal
  entries — belt and braces if the watchdog was disabled).

## Lab mode (default on)

In lab mode the only registered control backend is the **simulation
backend**: it mutates no system state. This is deliberate:

- the entire dangerous machinery (state machine ordering, journal, restore,
  verify, watchdog, emergency stop) is exercised end-to-end and safely,
- demos and education work on any machine,
- opting into real control backends is an explicit, separate decision.

## Authorized use

BanDen is designed for networks you administer or are authorized to manage
(personal networks, home labs, controlled environments, education). The
license terms prohibit misuse. Control backends that modify third-party
traffic paths require both elevation and explicit authorization, and are not
enabled by any default.
