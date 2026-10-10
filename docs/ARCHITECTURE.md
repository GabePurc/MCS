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
* `device.rs` / `devices.rs` — architecture-neutral device handle: `Arch`, `DeviceRef`
  (`Avr(&'static AvrDeviceSpec)`, serialized as the spec plus an `"arch"` tag) and the
  registry `devices::get_any` / `list_any` across all architectures. Neutral code (session, API)
  uses these; AVR-only code (assembler, disassembler, definition files, ISA tables) takes the
  spec through `DeviceRef::as_avr()` or `avr::devices` directly.
* `arm/thumb.rs` + `arm/disasm.rs` — ARMv7-M Thumb/Thumb-2 decoder (encoding -> compact `Insn` with an `Op` id and normalized operands, branch/literal offsets relative to the instruction) and the UAL disassembler formatting the same `Insn`; the simulator's executor consumes the same decode result.
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
* `arm/` — ARMv7-M machine (STM32G4 devices via `Machine::from_spec`, behind the session through `target.rs`; peripherals in `periph/`, pins and clock tree in `sys.rs`, debugger state in `debug.rs`): `cpu.rs` registers/xPSR/SP banking, `bus.rs` memory map + `Mmio` peripheral trait (event-driven via `Cx::schedule`), `exec.rs` dense-`match` executor with Cortex-M4 cycle counts, `machine.rs` run loop + exception entry/return/tail-chaining/fault escalation, `nvic.rs`, `scb.rs` (System Control Space registers), `systick.rs`.
* `pins.rs` — electrical model (direction, latch, pull-up, peripheral override, external
  drive incl. analog voltage, Schmitt thresholds, contention detection), the logic-analyzer
  trace ring buffer (`ceil(GPIOs / 32)` words per entry, so the 86-pin ATmega2560 and custom devices trace every pin) and the piecewise clock model (cycles <-> seconds across clock changes).
* `avr/peripherals/stimulus.rs` — test-bench signal generators (square waves / pulse bursts on
  pins), a peripheral like any other: one scheduled event per edge, timed in seconds.
* Execution profiling: `Machine::run` is monomorphized over a `PROFILE` const so per-word
  execution counting (Chip View heat map) costs nothing while disabled.
* `target.rs` — the architecture seam: the object-safe `Target` trait (run to a cycle target,
  reset / power cycle, breakpoints, run-to and step plans, pin / VCC / clock / serial /
  stimulus commands, debugger writes, `snapshot`). `avr::Machine` implements it in
  `avr/target.rs`, which also owns the AVR-only parts (step predicates, source-line map, fuse /
  EEPROM / register writes, snapshot building). Debugger writes that only exist on some
  architectures have defaults that return an error.
* `session.rs` — debugger session over a `Box<dyn Target>` created from a `DeviceRef`:
  real-time / fixed-rate (cycles per second, down to 1 Hz) / max speed in time slices,
  breakpoints, run-to, source- or instruction-level stepping (the target arms the stop
  condition), state snapshots (`protocol.rs`). It makes at most one dynamic call per time
  slice, command or state publish, never per instruction: the executor loop stays inside the
  concrete machine. `Session::avr_machine()` downcasts for tests. `session::spawn` runs it on a
  thread for the desktop app.
* `protocol.rs` — `MachineState` is architecture-neutral (`pc` in the native unit plus
  `pcBytes`, memories, pins, trace) and carries the CPU registers in `core: CoreState`
  (`{arch: "avr", sp, sreg, regs}`). `Output::Device` sends the spec with its `arch` tag.

## Adding a microcontroller

1. **Same family (another AVR):** add a spec in `crates/mcs-core/src/avr/devices/` (see
   `tiny_rc.rs`, `tiny_x5.rs`, `mega_x8.rs`: registers with bits, vectors, pins with GPIO
   indices, fuse bytes, sleep control, boot sections) and register it in `devices/mod.rs`. Wire
   it with a `PeripheralSet` recipe in `crates/mcs-sim/src/avr/peripherals/mod.rs`; the models
   are parameterized by addresses, bit masks, pins and vectors: `Port`, `ExtInt` (any INTn /
   PCINT layout), `Timer` (8/16-bit) + `IrqFlags` (TIFR/TIMSK, shareable) + `Gtccr`, `Usart`,
   `Spi`, `Twi`, `Usi`, `Eeprom`, `Adc`/`AnalogComparator`, `Watchdog` (CCP or WDCE),
   `ClassicSystem` (CKSEL clock sources, CLKPR, BOD, MCUCR). Registers shared between modules
   have one owner that announces writes with `Event::RegWritten`; cross-module signals use
   `Trigger`s.
   **Custom (user-defined) devices:** `devices/custom.rs` turns a `CustomMcuConfig` into a full
   spec on the ATmega2560 register layout (extra instances are allocated in extended I/O);
   `devices::register_custom` leaks it into a runtime registry that `get`/`list` search, and
   `PeripheralSet::Custom` wires it by register/vector/pin-function names. The UI keeps the
   configurations in local storage and registers them with every backend instance at start-up
   (Tauri process; browser main thread + simulation worker).
2. **New architecture (e.g. ARM Cortex-M, ESP32):** the seam already exists (see
   `docs/MULTI_ARCH.md`). Add `mcs_core::<arch>` (ISA + device descriptions) and
   `mcs_sim::<arch>` (machine), then:
   * add a `DeviceRef::<Arch>(&'static ...Spec)` variant (`mcs_core::device`) and search the new
     registry in `mcs_core::devices::get_any` / `list_any`; the spec serializes with an `arch`
     tag, the UI's `DeviceSpec` union grows a matching member;
   * implement `mcs_sim::target::Target` for the machine and return it from
     `target::new_target`; the session, protocol and API layers need no changes;
   * add a `CoreState::<Arch>` variant (register file in the architecture's own shape) and the
     UI's matching `CoreState` member plus its processor/memory views.

## UI

* State lives in small zustand stores (`src/ui/state`); panels subscribe to just the slices they
  render, and the simulator publishes snapshots at ~30 Hz while running.
* Every user action is a command (`services/commands.ts`): menus, toolbar, context menus and
  shortcuts share enablement and behavior.
* `backend/api.ts` is the only module that knows about Tauri; in a plain browser it routes the
  same calls to the WebAssembly core (`backend/wasmHost.ts`, `backend/simWorker.ts`).
* Window layout (`state/layout.ts`): every tool panel is docked in one of four groups, floating
  (`dock/FloatingWindows.tsx`) or popped out into its own OS window. Pop-outs
  (`services/windows.ts`, `PopoutApp.tsx`) load the same page with `?popout=<panel>`; the main
  window stays the only owner of the simulator and documents and mirrors simulator outputs,
  workspace and settings to them over a bridge (Tauri events / BroadcastChannel). Pop-outs send
  simulator commands, UI commands and workspace edits back.
* Per architecture: `DeviceSpec` is `AvrDeviceSpec | ArmDeviceSpec` (`arch` tag) and `CoreState` has an `avr` and an `arm` member (`backend/types.ts`: `isAvr`, `avrCore`, `armCore`, `pcToBytes`). The Processor, I/O view, Memory and Device Info panels have an ARM variant (`ArmProcessorPanel`, `ArmIoView`, `ArmDeviceInfo`; Memory and Pins branch inside); the Chip View, fuses, ISA and `.inc` panels are AVR-only.
* Chip View (`src/ui/chip`): `floorplan.ts` derives a die floorplan from the device spec (memory
  arrays, CPU, one block per peripheral group, pads per package pin); `dieArt.ts` draws the
  silicon and the live block contents on canvases; `engine.ts` turns machine states into decaying
  highlights and redraws only blocks whose content fingerprint changed; `scene3d.ts` (three.js,
  lazily loaded) renders the package/lead frame/bond wires/die on demand with shadows and GTAO.
* Symbol View (`editor/symbolView.ts`): a CodeMirror extension keeps the document outline
  (labels / top-level C functions, updated incrementally) and a shown region; block
  decorations hide the rest, a change filter keeps typing inside it and a transaction filter
  clamps cursor moves or switches the region when something else moves the selection.
* Machine-code sources (`.mc`) are parsed by `mcs_asm::assemble_machine_code` into an ordinary
  `LoadedProgram` with a line table, so they debug like assembly.
