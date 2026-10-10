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

### User requests (GitHub issue #1)
- [x] Chip View: 3D package/lead frame/bond wires/die with shadows + ambient occlusion, and a flat die view; live flash heat map + PC, SRAM/stack, registers, decoded instruction, SREG, peripheral activity, pin levels
- [x] Fixed-rate speed mode (1 Hz ... 100 MHz cycles/s) besides real-time multiples and maximum; Custom speed dialog
- [x] Floating tool windows and pop-out OS windows for every panel (state mirrored from the main window)
- [x] Device Info panel (specs, speed grades, memory map, peripherals, pins, vectors, die floorplan, die photo link)
- [x] Instruction Set reference as a non-modal panel with descriptions, encodings and an asm <-> machine code converter
- [x] External clock fix: frequency changes apply immediately when CLKI is selected; Supply & Clock dialog selects source/prescaler (debugger CCP write) and warns above the speed grade
- [x] Pin signal generators (square wave / pulse bursts) and momentary push buttons
- [x] Machine-code source files (.mc) with live disassembly, line-level debugging, and program -> .mc conversion
- [x] In-app updates (Tauri updater): signed bundles + latest.json from the release workflow, Help > Check for Updates, start-up check, CHANGELOG-based release notes
- [x] ATmega48PA/88PA/168PA/328P and ATtiny25/45/85: specs, generalized timers (8/16-bit, shared flag registers, GTCCR), ATtiny85 PLL Timer1, USART, SPI, TWI (empty bus), USI, EEPROM, 10-bit ADC, comparator ACME/bandgap, INT0/INT1/PCINT groups, classic system control (CKSEL fuses, CLKPR, BOD, BOOTRST/IVSEL), multi-byte fuses, canonical sleep modes
- [x] Serial Monitor (UART decode/inject on any pin), EEPROM view/edit, fuse dialog with field menus and presets, DIP packages in the Chip View

### User requests (GitHub issue #3)
- [x] Custom microcontrollers (Device > Custom Microcontroller...): any flash (up to 8 MB), SRAM (up to the 64 KB data space), EEPROM, GPIO ports, INT pins, 8/16-bit timers, USARTs, SPI, TWI, ADC channels, comparator, multiplier, package, clock; only architectural limits (255 vectors, data space, 255 GPIOs); generated on the ATmega2560 register layout and wired from the spec
- [x] Core: non-power-of-two flash sizes, 22-bit PC (3-byte return addresses, CALL/RET/interrupt timing), EIJMP/EICALL via EIND, ELPM via RAMPZ
- [x] Build > Clear Output on Build / Run
- [x] Instruction help: editor hover on mnemonics (operation, flags, cycles, how to use it, example) for every instruction and assembler alias; usage + example in the Instruction Set panel, aliases listed
- [x] Chip View: zoom into FLASH / SRAM / EEPROM to read individual words and bytes (addresses, disassembly when large); clicking a memory block zooms there (from the 3D view too); costs scale with the visible area / executed code, not the memory size
- [x] Fix: Tab inserts spaces at the cursor (indents only with a selection)

## Next
- [ ] DWARF variable/type info (`.debug_info`) for a typed Watch window and locals
- [ ] Data breakpoints (watchpoints) and conditional / hit-count breakpoints
- [ ] More virtual components on the pin panel (LED, potentiometer, logic probe); serial (UART) stimulus
- [ ] Waveform: analog traces, protocol decoders (UART, SPI, I²C), export (VCD)
- [ ] Input synchronizer latency and input-capture noise canceler delay
- [ ] More devices (issue #1): ATtiny13A/2313/84, ATmega8/16/32/644/1284/2560, ATtiny20/40, then the AVR-0/1 series (new register map)
- [ ] Virtual I²C/SPI devices on the bus (EEPROM, sensors) so TWI/SPI transfers get answers
- [ ] SPM self-programming, debugWIRE, timer asynchronous (TOSC) mode, USART synchronous / MSPIM modes
- [ ] ARM Cortex-M (STM32) targets: needs the architecture abstraction below plus a Thumb-2 core, NVIC/SysTick and per-family peripherals
- [ ] Architecture abstraction for non-AVR targets (machine trait, register descriptions in specs)
- [ ] Project files (multi-file C builds, per-project device/clock/fuses)
- [ ] Signed release builds (Apple notarization, Windows code signing)
