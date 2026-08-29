# Integration tests

Workspace-level cross-crate integration tests live here. Currently the
per-crate suites (unit + integration) cover:

- `banden-core`: session state machine transitions, recovery manager
  (restore/verify/retry/failure), session manager lifecycle, emergency-stop
  orchestration, traffic aggregation.
- `banden-db`: migrations, repositories, journal store port.
- `banden-net`: parsing/normalization logic (subnet math, ARP diffing, OUI
  lookup, simulation bounds) plus machine-dependent smoke tests that are
  skipped gracefully on machines without usable network adapters.
- `banden-watchdog`: decision table, args parsing, journal replay.

End-to-end tests driving the packaged app (installer, watchdog kill-recovery
drill) are planned once the elevated helper roadmap item lands.
