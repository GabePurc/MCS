# MCS — Microcontroller Simulator

A cycle-accurate microcontroller simulator and source-level debugger with a Windows 7–style interface. It runs on Windows, macOS and Linux. The first supported family is the **ATtiny4/5/9/10**: the AVRrc reduced core in a SOT-23-6 package.

![MCS stopped at a breakpoint in blink.asm](docs/screenshot.jpg)

## Download

Get the latest installers from the **[Releases page](https://github.com/GabePurc/MCS/releases/latest)**:

- **Windows 10/11:** `MCS.Microcontroller.Simulator_<version>_x64-setup.exe`. Run it; no administrator rights are needed. The installer isn't signed yet, so if SmartScreen says *"Windows protected your PC"*, click **More info → Run anyway**.
- **macOS 10.15+ (Apple Silicon and Intel):** `MCS.Microcontroller.Simulator_<version>_universal.dmg`. Drag the app to Applications. The first time, right-click it and choose **Open** (it isn't notarized yet).

Assembly programs work out of the box. To build C programs, also install avr-gcc (see [C support](#c-support)).

## Features

- **Simulation engine in Rust** (`crates/mcs-sim`)
  - Pre-decoded flash and a jump-table executor give about 140 MIPS on an Apple M-series chip. That is roughly 180× real time for an ATtiny10 at 1 MHz.
  - Peripherals are event-driven, so they cost nothing while idle. In sleep, the simulator skips straight to the next event.
  - Peripheral models:
    - GPIO with pull-ups, the PINx toggle and the RESET pin
    - INT0 and pin-change interrupts
    - Timer0: all 16 waveform-generation modes, PWM output, input capture, external clock and the TEMP register
    - Analog comparator
    - 8-bit ADC with auto-trigger
    - Watchdog: interrupt and reset modes, WDRF/WDE lock, CCP-protected changes
    - Clock source and prescaler (CCP protected)
    - Sleep modes with the datasheet's wake-up sources
    - Power reduction, VCC level monitor, reset flags, NVM, fuse and signature mapping
  - Timing follows the ATtiny4/5/9/10 datasheet: instruction cycle counts, 4-cycle interrupt response, and the extra instruction after `SEI` and `RETI`.
- **Test bench**
  - Signal generators on any pin: square waves from 0.01 Hz to 20 MHz, or bursts of N pulses (active high or low). They are event driven and keep their frequency when the firmware changes the CPU clock.
  - Momentary push buttons, logic levels and analog voltages per pin
  - Execution profiling (per-instruction counts) for the Chip View heat map; it costs nothing while off.
- **Assembler** (`crates/mcs-asm`): compatible with Atmel avrasm2.
  - Supports macros, conditionals, includes, expressions, and device definitions (`.include "tn10def.inc"`) generated from the device model.
  - Errors carry exact line and column, and the assembler rejects instructions and registers the device doesn't have.
- **Machine code files** (`.mc`): write a program directly as 16-bit instruction words, in hex or binary. Live disassembly appears at the end of every line, and you build and step through them like assembly. **Build ▸ Open Program as Machine Code** turns any program into an editable `.mc` file.
- **C and GNU assembler** through your installed **avr-gcc**, which is detected automatically. Programs are debugged at the source line using DWARF line tables.
- **Imports** Intel HEX and ELF images built elsewhere, and exports Intel HEX.
- **Debugger**
  - Breakpoints in the editor margin or the disassembly
  - Step into, over and out by source line or by instruction
  - Run to cursor, set next statement, call stack and stop watch
- **Inspect everything**
  - Processor: PC, SP, X/Y/Z, SREG flags and registers, all editable
  - I/O view: peripheral, register and bit tree with live, editable values
  - Hex memory editor for data space and flash
  - Disassembly, plus a symbols and watch list
  - Pin stimulus: logic levels, analog voltages and VCC
  - Logic-analyzer waveform window with measurement cursors
  - **Chip View**: a 3D model of the chip (package, lead frame, gold bond wires, die) with soft shadows and ambient occlusion, or a flat die view. The die shows the running program live: a flash heat map with the PC, SRAM and stack bytes, the register file, the instruction being decoded, SREG, peripheral activity and pin levels.
  - **Device Info**: specifications, speed grades, memory map, peripherals, pins, vectors and the die floorplan, with a link to a real die photo
  - **Instruction Set** reference with descriptions, colour-coded encodings and an assembly ⇄ machine code converter
  - Changed values are highlighted in red, as in Visual Studio and Atmel Studio.
- **Windows 7 look on every OS**
  - Aero caption, glossy controls, Explorer-style selection, dockable tool windows with drag-and-drop tabs
  - Any tool window can float above the main window or open in its own OS window, for example on a second monitor. Drop a floating window on a tab strip to dock it again.
  - Bundled Selawik font (a metric-compatible Segoe UI clone) and Cascadia Mono
- **Web build**: the same Rust core compiles to WebAssembly (`crates/mcs-wasm`), so the UI also runs in a browser.

## Getting started

Prerequisites:

- Node ≥ 20.19 (24 recommended, see `.nvmrc`)
- A stable Rust toolchain
- The Tauri system dependencies for your OS: [tauri.app/start/prerequisites](https://tauri.app/start/prerequisites/). On Linux that means WebKitGTK 4.1.

```bash
npm install
npm run dev
```

`npm run dev` starts the desktop app in development mode.

Other commands:

| Command | What it does |
|---|---|
| `npm run build` | Builds release installers (`.msi`/`.exe`, `.dmg`/`.app`, `.deb`/`.AppImage`/`.rpm`) |
| `npm run web` | Builds the WebAssembly core and serves the UI in a browser (needs `rustup target add wasm32-unknown-unknown`) |
| `npm test` | Runs all Rust tests plus the front-end unit tests |
| `npm run typecheck` | Type-checks the UI |

Pushing a `v*` tag runs `.github/workflows/release.yml`, which builds installers for all three platforms.

### C support

Assembly needs nothing extra. For C and `.S` files, install avr-gcc with avr-libc:

- **macOS:** `brew tap osx-cross/avr && brew install avr-gcc@14`, or the Arduino IDE.
- **Windows:** Microchip's AVR 8-bit toolchain, or the Arduino IDE.
- **Linux:** `sudo apt install gcc-avr avr-libc` (or your distribution's equivalent).

MCS searches `PATH` and the usual install locations. You can also set the path under **Tools ▸ Toolchain Options**.

## Using it

1. Open an example from the Start Page, for example *Blink (assembly)*.
2. **F7** builds. **F5** runs or continues. **F10**, **F11** and **Shift+F11** step over, into and out. **F9** toggles a breakpoint.
3. Drive inputs in **Pins & Stimulus**: click a pin to cycle Z → 1 → 0, or choose `~` for an analog voltage. Watch outputs in **Waveform**: scroll to zoom, drag to pan, click and Shift+click to measure.
4. Double-click any value to edit it. Click SREG flags or I/O bit boxes to toggle them.
5. Choose the speed in the toolbar. **Speed ▸ Custom...** offers three modes: a fixed CPU clock from 1 Hz (one cycle per second) upward, a multiple of the chip's real speed, or maximum.
6. **Device ▸ Supply & Clock** sets VCC, the clock source (internal RC, 128 kHz, or external clock on CLKI), the external frequency and the prescaler. Changes apply while the program runs.
7. **View ▸ Chip View (3D)**, together with a slow speed, lets you watch the program execute inside the chip. Right-click any tool tab and choose **Float** or **Open in New Window** to detach it.

## Project layout

```
crates/mcs-core     ISA table (decoder/disassembler/assembler share it), device specs, program types
crates/mcs-sim      CPU executor, machine, peripherals, scheduler, debugger session
crates/mcs-asm      avrasm2-compatible assembler
crates/mcs-formats  Intel HEX, ELF and DWARF line-table loaders
crates/mcs-api      Front-end services shared by the desktop and web hosts
crates/mcs-wasm     WebAssembly adapter (JSON ABI)
src-tauri           Desktop app: Tauri commands, simulation thread, avr-gcc integration
src/ui              React UI (Win7 theme, docking, CodeMirror editor, panels)
examples            Example programs (also bundled into the app)
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how the pieces fit together and how to add another microcontroller. See [docs/ROADMAP.md](docs/ROADMAP.md) for status and next steps.

## License

MIT. The bundled fonts are under the SIL Open Font License 1.1.
