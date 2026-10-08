# Roadmap & Progress

Update this file when work is finished (only list items that are done in the repo).

## Completed

### Core & simulation
- [x] Declarative AVR ISA table (full AVR instruction set; AVRrc timing) driving decoder, disassembler and assembler
- [x] ATtiny4/5/9/10 device specs (registers, bits, vectors, pins, fuses, signature)
- [x] CPU executor with pre-decoded flash, interrupts (priority, SEI/RETI one-instruction delay), sleep with fast-forward, shadow call stack
- [x] Peripherals: GPIO/PUE/PINx toggle/RESET pin, INT0 + PCINT, 16-bit Timer0 (16 WGM modes, OC outputs, input capture, T0 clock, TEMP), analog comparator, ADC (single/free-running/auto-trigger, noise-reduction sleep), watchdog (interrupt/reset, WDRF lock, CCP), CCP/CLKMSR/CLKPSR, SMCR, RSTFLR, PRR, VLM, NVM/fuse/signature mapping
- [x] Event scheduler + lazy timer sync; logic-analyzer trace; piecewise clock model
- [x] Debugger session: real-time / scaled / max speed, breakpoints, run-to, step into/over/out by line or instruction
- [x] ~140 MIPS throughput (release build, Apple Silicon)

### Tooling
- [x] avrasm2-compatible assembler with generated device includes, macros, conditionals, diagnostics, listing, line table
- [x] Intel HEX import/export, ELF + DWARF v2–v5 line-table loader
- [x] avr-gcc integration (auto-detection on Windows/macOS/Linux, GCC diagnostics parsing); C verified end-to-end with avr-gcc 14.3 (compile, DWARF line mapping incl. inlined headers, timing, interrupts, source stepping)

### Application
- [x] Tauri desktop app (Windows/macOS/Linux), frameless window with Aero caption
- [x] Windows 7 theme: menus, toolbar, docking tool windows (drag tabs between groups), dialogs, scrollbars, tooltips
- [x] CodeMirror editor: AVR asm + C highlighting, breakpoint margin, current-line marker, diagnostics, hover info, completion
- [x] Panels: Processor, I/O View, Memory (hex editor), Disassembly, Pins & Stimulus, Waveform, Output, Symbols, Call Stack, Breakpoints
- [x] Example programs (assembly + C), start page, recent files, session restore
- [x] WebAssembly build of the core; UI runs in a browser too
- [x] CI (tests on all three OSes) and tag-triggered releases: Windows NSIS installer (per-user, WebView2 bootstrap) + macOS universal .dmg

## Next
- [ ] DWARF variable/type info (`.debug_info`) for a typed Watch window and locals
- [ ] Data breakpoints (watchpoints) and conditional / hit-count breakpoints
- [ ] Virtual components on the pin panel (LED, button, potentiometer, logic probe)
- [ ] Waveform: analog traces, protocol decoders (UART, SPI, I²C), export (VCD)
- [ ] Input synchronizer latency and input-capture noise canceler delay
- [ ] ATtiny20/40 (TWI, SPI, USART-less), then classic ATtiny/ATmega (ATtiny85, ATmega328P)
- [ ] Architecture abstraction for non-AVR targets (machine trait, register descriptions in specs)
- [ ] Project files (multi-file C builds, per-project device/clock/fuses)
- [ ] Signed release builds (Apple notarization, Windows code signing)
