# Architecture

MCS is a Cargo workspace (simulation, tooling) plus a React UI hosted by Tauri. All program
logic is Rust; TypeScript only renders and routes user input.

```
                ┌───────────────────────────── src/ui (React + CodeMirror, Win7 theme) ─────────────────────────────┐
                │ panels ── stores (zustand) ── services (build, commands, files) ── backend/api.ts                  │
                └───────────────────────────────────────────────┬───────────────────────────────────────────────────┘
                                 Tauri invoke + Channel (desktop)│ JSON over WASM memory (browser)
                ┌────────────────────────── src-tauri ───────────┴──────┐   ┌──────────── crates/mcs-wasm ───────────┐
                │ commands, file IO, avr-gcc (toolchain.rs), sim thread │   │ same services, session in a Web Worker │
                └───────────────────┬───────────────────────────────────┘   └───────────────────┬────────────────────┘
                                    └──────────────── crates/mcs-api (host-independent services) ┘
                         ┌────────────────┬───────────────┼────────────────┐
                    mcs-sim           mcs-asm        mcs-formats       mcs-core
             (machine, session)   (assembler)   (HEX/ELF/DWARF)   (ISA, devices, program)
```

## mcs-core

* `avr/isa.rs` — one declarative table of every AVR instruction: bit pattern, operand kinds,
  feature requirements (reduced core, MUL, JMP...), cycles for AVRe and AVRrc. The decoder
  (a cached 64K first-word lookup per feature set), the disassembler and the assembler's
  encoder are all derived from it, so they can never disagree.
* `avr/device.rs` + `avr/devices/` — declarative device specs: memories, I/O registers with
  bit fields and reset values, vectors, pins/package, fuses, clock, and the name of the
  peripheral wiring recipe. The UI's I/O view, pin diagram and the assembler's generated
  `tnXXdef.inc` all come from the spec.
* `program.rs` — `LoadedProgram`: flash image + symbols + line table + diagnostics. Every
  front-end (assembler, ELF, HEX) produces it; the simulator and UI consume it.

## mcs-sim

* `avr/cpu.rs` — architectural state and the pre-decoded program (op id + two normalized
  operands per word, re-decoded on flash writes).
* `avr/machine.rs` — the executor (`exec`, a dense `match`), data bus, interrupt service, sleep,
  reset. I/O addresses without side effects are plain memory; owned addresses dispatch to the
  owning peripheral through `io_owner`.
* Peripherals implement the `Peripheral` trait and receive a `Cx` (CPU + machine services).
  They never call each other: cross-module effects (pin changes, ADC triggers, PRR, clock
  changes, resets) are queued as `Event`s and broadcast after the call returns.
* `scheduler.rs` — cycle-stamped events. Peripherals schedule the exact cycle where something
  observable happens (e.g. the timer's next compare match/TOP/BOTTOM tick) and advance lazily
  when software touches their registers, so idle peripherals cost nothing per instruction.
* `pins.rs` — electrical model (direction, latch, pull-up, peripheral override, external
  drive incl. analog voltage, Schmitt thresholds, contention detection), the logic-analyzer
  trace ring buffer and the piecewise clock model (cycles <-> seconds across clock changes).
* `session.rs` — debugger session: real-time pacing / max speed in time slices, breakpoints,
  run-to, source- or instruction-level stepping (via a per-instruction predicate), state
  snapshots (`protocol.rs`). `session::spawn` runs it on a thread for the desktop app.

## Adding a microcontroller

1. **Same family (another AVR):** add a spec in `crates/mcs-core/src/avr/devices/` and register
   it in `devices/mod.rs`. If its peripherals differ, add a `PeripheralSet` variant and a wiring
   function in `crates/mcs-sim/src/avr/peripherals/mod.rs`, reusing or adding peripheral models
   (port, timer16, analog, exint, system are parameterized by register addresses). Classic
   cores (ATmega) mainly need `regs_in_data_space = true`, `io_base = 0x20` and the feature
   flags; the executor already implements the full AVR instruction set.
2. **New architecture (e.g. ARM Cortex-M0, PIC):** add `mcs_core::<arch>` (ISA + device
   descriptions) and `mcs_sim::<arch>` (machine). The session/protocol layer is the seam: give
   the session a machine abstraction (trait) and keep `MachineState` architecture-neutral
   (registers already travel as a byte array; add a register-description list to the device
   spec so the Processor panel can render any register file).

## UI

* State lives in small zustand stores (`src/ui/state`); panels subscribe to just the slices they
  render, and the simulator publishes snapshots at ~30 Hz while running.
* Every user action is a command (`services/commands.ts`): menus, toolbar, context menus and
  shortcuts share enablement and behavior.
* `backend/api.ts` is the only module that knows about Tauri; in a plain browser it routes the
  same calls to the WebAssembly core (`backend/wasmHost.ts`, `backend/simWorker.ts`).
