# MCS — notes for coding agents

This file applies to the MCS repository and takes precedence over any parent-directory
AGENTS.md (the game-design docs referenced there do not apply to this project).

- Read `README.md`, `docs/ARCHITECTURE.md` and `docs/ROADMAP.md` before changing features.
- Keep code highly optimized: the executor hot path (`crates/mcs-sim/src/avr/machine.rs`)
  must stay allocation-free; peripherals must be event driven (no per-cycle ticking).
- All simulation/tooling logic belongs in Rust crates; the UI only renders and routes input.
  New backend features go into `crates/mcs-api` so both hosts (Tauri, WASM) get them.
- Device facts must come from datasheets; cite the source in the device/peripheral module docs.
- Run before committing: `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --release`, `npm run typecheck`, `npx vitest run`.
  (`cargo` builds of `src-tauri` need `dist/`; run `npm run build:web` once.)
- After finishing meaningful work, update `docs/ROADMAP.md` (only list work that is done).
