# Security Policy

## Supported versions

Security fixes target the latest `0.x` release line on `main`.

## Reporting a vulnerability

Please report vulnerabilities privately via GitHub Security Advisories
("Report a vulnerability" under the Security tab of the repository). Do not
open public issues for exploitable flaws. We aim to respond within 72 hours
and will credit reporters unless anonymity is requested.

## Threat model (summary)

- BanDen runs with normal user privileges. Everything in the default build
  (interface enumeration, ARP inspection, `SendARP` discovery, interface
  counters, ICMP latency) works without elevation.
- The SQLite database (`%APPDATA%/org.banden.app/`) contains network
  inventory, traffic history and the recovery journal. It is local-only;
  BanDen performs no network telemetry or remote access.
- The recovery journal is safety-critical: tampering with it could suppress
  restoration actions. Treat the app-data directory as trusted space
  (standard for local user-profile data).
- Control backends that modify real traffic handling require explicit
  opt-out of lab mode and (in roadmap implementations) an elevated helper;
  such backends must preserve the capture/journal/verify contract described
  in `docs/SAFETY.md`.

## Authorized-use boundary

Security research involving BanDen on networks you do not own requires the
network owner's written authorization. See `LICENSE` (additional term) and
`docs/SAFETY.md`.
