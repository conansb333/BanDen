# Contributing to BanDen

Thanks for helping build a serious network administration tool.

## Ground rules

1. **Authorized use only.** Contributions that add covert interception,
   detection evasion, or functionality aimed at networks the operator is not
   authorized to manage will be rejected.
2. **Safety first.** Anything touching the session lifecycle, recovery
   journal, watchdog or emergency stop must come with tests covering the
   failure paths, not just the happy path.
3. **Boring, reliable engineering.** No speculative rewrites, no clever
   abstractions where a plain function works.

## Workflow

1. Fork, create a feature branch from `main`.
2. Before opening a PR, run the full gate locally:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd apps/desktop && npm install && npm run typecheck && npm test && npm run build
```

3. Keep PRs small and vertical (one feature or fix). Reference the issue.
4. Describe the *safety impact* of your change explicitly when it touches
   sessions, recovery, the watchdog or any Win32 call that mutates state.

## Code conventions

- Rust: standard `rustfmt`; module-per-concern in the existing crates; no
  `unsafe` outside `banden-net` (and every `unsafe` block carries a
  `SAFETY:` comment).
- TypeScript/React: functional components, hooks from `@/hooks`, shadcn/ui
  primitives only, no business logic inside JSX, no direct `invoke` calls
  outside `src/lib/api.ts`.
- Migrations: never edit an applied migration; add a new numbered file in
  `migrations/` and bump `PRAGMA user_version` inside it.
- Events and commands: keep payloads versioned (`v` field) and mirror types
  in `apps/desktop/src/types`.

## Testing priorities

In order: session state machine transitions → recovery/journal → watchdog
decision logic → traffic aggregation → discovery parsing/normalization →
persistence → UI components.

## Licensing

By contributing you agree your contributions are licensed under the MIT
license (with the authorized-use term) as stated in `LICENSE`.
