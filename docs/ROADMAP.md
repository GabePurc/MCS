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
- [x] ATtiny13A, ATtiny24A/44A/84A and ATtiny2313A/4313 (issue #5): specs from the data sheets, wiring recipes on the existing models (ADLAR-in-ADCSRB option and half-frequency RC clock source added; 64-entry differential/gain ADC table on the x4; non-contiguous SM1:0 sleep field on the 2313A), fuse presets, tests in `crates/mcs-sim/tests/tiny_more.rs`
- [x] ATmega8/16/32 (issue #5): classic register map (`PeripheralSet::MegaLegacy`): single-register TCCR0/TCCR2 timers with one compare unit, FOC1x in TCCR1A, shared UBRRH/UCSRC (URSEL), SFIOR (ADTS/ACME/PUD/PSR), GICR (INT enables + IVSEL), INT2 one-bit sense, legacy watchdog, CKSEL internal RC 1/2/4/8 MHz, BODEN/BODLEVEL, 8.5 ms EEPROM writes; tests in `crates/mcs-sim/tests/mega_legacy.rs`
- [x] ATmega164PA/324PA/644PA/1284P and ATmega640/1280/2560 (issue #5): two USARTs / four USARTs, TC3-5, third compare unit (OCnC) in the timer model, ADC/comparator MUX5, PRR1, EIND/RAMPZ on the 2560, multi-word pin trace for devices with more than 32 GPIOs (waveform view), Arduino Mega fuse preset; tests in `crates/mcs-sim/tests/mega_large.rs`
- [x] Multi-architecture seam, Stage A (issues #4, #5; `docs/MULTI_ARCH.md`): `mcs_core::device::DeviceRef` + `devices::get_any/list_any`, `mcs_sim::target::Target` implemented by `avr::Machine` (session drives a `Box<dyn Target>`), `MachineState.core: CoreState` + `pcBytes`, `arch` tag on every device spec, UI `DeviceSpec`/`CoreState` types; no behaviour change for AVR
- [x] Serial Monitor (UART decode/inject on any pin), EEPROM view/edit, fuse dialog with field menus and presets, DIP packages in the Chip View

### User requests (GitHub issue #3)
- [x] Custom microcontrollers (Device > Custom Microcontroller...): any flash (up to 8 MB), SRAM (up to the 64 KB data space), EEPROM, GPIO ports, INT pins, 8/16-bit timers, USARTs, SPI, TWI, ADC channels, comparator, multiplier, package, clock; only architectural limits (255 vectors, data space, 255 GPIOs); generated on the ATmega2560 register layout and wired from the spec
- [x] Core: non-power-of-two flash sizes, 22-bit PC (3-byte return addresses, CALL/RET/interrupt timing), EIJMP/EICALL via EIND, ELPM via RAMPZ
- [x] Build > Clear Output on Build / Run
- [x] Instruction help: editor hover on mnemonics (operation, flags, cycles, how to use it, example) for every instruction and assembler alias; usage + example in the Instruction Set panel, aliases listed
- [x] Chip View: zoom into FLASH / SRAM / EEPROM to read individual words and bytes (addresses, disassembly when large); clicking a memory block zooms there (from the 3D view too); costs scale with the visible area / executed code, not the memory size
- [x] Fix: Tab inserts spaces at the cursor (indents only with a selection)

### User requests (GitHub issue #2)
- [x] Symbol View (View > Symbol View, Ctrl+Shift+O): the editor shows one assembly label / C function at a time with a symbol sidebar in file order; one document (real line numbers, one undo history), edits and cursor stay in the shown symbol, jumps (go to line, breakpoints, debugger, search, undo) switch symbols; "+" adds a label / function after the shown one; incremental outline (changed lines only for assembly, top-level syntax tree for C)
- [x] Device Definitions (.inc) is a tool window (float, dock or pop out) with a line filter

### Multi-architecture (docs/MULTI_ARCH.md, Stage B1/B2)
- [x] Standalone ARMv7-M core, not yet wired into the session: `mcs_core::arm` Thumb/Thumb-2 decoder + UAL disassembler (all base-ISA encodings, DSP saturating/extend-and-add subset; checked against 1900+ `llvm-objdump` reference lines in `crates/mcs-core/tests/arm_thumb`), `mcs_sim::arm` machine (r0-r15/xPSR/MSP/PSP/CONTROL/PRIMASK/FAULTMASK/BASEPRI, pre-decoded flash, memory bus with `Mmio` peripheral trait, NVIC with priority grouping/preemption/tail-chaining, SysTick, SCB incl. fault escalation/lockup, WFI/WFE sleep with fast-forward, ~300 simulated MHz); tests in `crates/mcs-sim/tests/arm_core` (programs assembled by clang, `gen_programs.py`)
- [x] Stage C1, STM32G4 end to end on the Rust side: `STM32G431KB` (LQFP32) and `STM32G474RE` (LQFP64) as `DeviceRef::Arm` (`mcs_core::arm::{device, devices}`; register offsets, bit masks, IRQ names generated from ST's CMSIS header and pins/alternate functions from ST's CubeMX database by `gen_stm32g4.py`); `mcs_sim::arm::Machine::from_spec` wires memories (CCM SRAM alias), RCC (HSI16/HSE/PLL, prescalers, clock gating/reset, wait-state and boost warnings), FLASH ACR/key unlock, PWR, GPIO A-G (full electrical model, AF routing, lock), SYSCFG+EXTI, USART1-3/UART4-5/LPUART1 (bit-level TX/RX on the pins, serial monitor bridge), TIM2-4 (PWM/output compare) and TIM6/7 on the event scheduler; `Target` implementation (breakpoints, run-to, source/instruction step over/out, call stack, register `io` view); ELF `EM_ARM` + Intel HEX at 0x0800_0000 loading; ARM disassembly through `mcs_api::disassemble`; the AVR assembler refuses ARM devices; tests in `crates/mcs-sim/tests/stm32g4.rs`, `crates/mcs-formats/tests/arm.rs`, `crates/mcs-core/tests/arm_devices.rs` (UI support is a later stage)
- [x] Stage C2, ARM in the UI (`docs/MULTI_ARCH.md`): `ArmDeviceSpec` / `CoreState` Arm / `io` TypeScript mirrors with `isAvr` / `armCore` helpers and the pc-unit helpers `pcToBytes` / `bytesToPc` (AVR counts words, ARM bytes; `BuildInfo.arch` carries the unit through breakpoints, source mapping, disassembly, call stack and status bar). Rust: `CoreState::Arm` also carries S0-S31 and FPSCR on FPU devices, `WriteMem` (1/2/4-byte bus writes), `CpuField` Control / Primask / Basepri / Faultmask / Fpscr and `WriteReg` 16-47 for S0-S31. UI: ARM Processor panel (R0-R12, SP with MSP/PSP, xPSR flags + GE + IPSR exception name + EPSR, CONTROL / PRIMASK / FAULTMASK / BASEPRI, FPSCR flags and S/D register views), peripheral register view from `spec.registers` with 32-bit values and bit fields, Flash / SRAM (+CCM) memory view at bus addresses, LQFP quad pin diagram, waveform and serial pin lists limited to existing GPIOs, USART defaults (USART2), ARM Device Info; bundled `stm32g4_blink.elf` example on the STM32G474RE; Fuses, Instruction Set, `.inc` definitions, Custom MCU, Intel HEX export, machine-code view and Chip View are disabled or replaced by a hint on ARM devices; building `.asm` / `.c` for an ARM device reports that ELF / HEX must be loaded.
- [x] Stage D, STM32H743 (Cortex-M7 r1p1, DSP + FPv5-D16) end to end on the Rust side: `STM32H743IIT6` (LQFP176) and `STM32H743ZIT6` (LQFP144, Nucleo-H743ZI) as `DeviceRef::Arm` (`gen_stm32h7.py`: register layout/bits/IRQs from ST's CMSIS header, reset values from ST's SVD cross-checked against a second SVD, pins/AFs from CubeMX; shared helpers in `devices/common.rs`). Memory map generalised: the bus selects regions through a 4096-entry page table (`addr >> 20`, still one lookup on the hot path) with several RAM blocks and RAM aliases (ITCM 64 KiB at 0, DTCM 128 KiB, AXI SRAM 512 KiB, SRAM1-3 288 KiB also at 0x1000_0000, SRAM4, backup SRAM; 2 MiB flash without alias, boot vector at 0x0800_0000). Cortex-M7 system registers: CPUID 0x411FC271, CCR.IC/DC/BP, cache maintenance operations (no-ops) with plausible CLIDR/CTR/CCSIDR/CSSELR, 16-region MPU registers (stored and aliased, **not enforced**), ITCMCR/DTCMCR/AHBPCR/CACR/AHBSCR/ABFSR, ACTLR. STM32H7 RCC (HSI 64 MHz + HSIDIV, CSI, HSE, HSI48, PLL1-3 with DIVM/N/P/Q/R + FRACN, SW/SWS, D1CPRE/HPRE/D1PPRE/D2PPRE1-2/D3PPRE, TIMPRE timer clock ratios, AHB3/1/2/4 + APB3/1L/1H/2/4 ENR/RSTR incl. the C1 copies, range/VCO/VOS/wait-state warnings), PWR (CR3, D3CR VOS + VOSRDY, CSR1.ACTVOSRDY, SYSCFG ODEN = VOS0), FLASH interface (ACR, two-bank key sequences), SYSCFG+EXTI in the H7 layout (shared `SysExti` model parameterised by layout), GPIO A-K, USART1/2/3/6, UART4/5/7/8, LPUART1 (APB4), TIM2-7 reused from the G4 models (clock tree generalised to four APB buses); tests in `crates/mcs-sim/tests/stm32h7.rs` (clang-assembled programs under `tests/stm32h7/`, `mcs-formats/tests/data/stm32h743_blink.elf`). Not modelled: kernel clock selection (CCIPR), caches/MPU effects, flash wait-state timing, ADC/DMA/SPI/I2C/FDCAN/Ethernet/USB (UI support is a later stage)
- [x] Stage D UI, STM32H743 (issue #4): `watchRam` protocol command + `Target::watch_ram` (no-op default, ARM implementation) streaming one extra RAM block (ITCM, AXI SRAM, SRAM1-3, SRAM4, backup SRAM) as `MachineState.ramExtra`, only on selection / load / change; the Memory panel offers every `extraRam` block by name at its bus addresses with change highlighting, SP / stack marks and byte editing through `writeMem`; ARM Device Info lists the extra RAM in its memory map; bundled `stm32h7_blink.elf` example (PB0 / LD1 on the STM32H743ZIT6, identical to the tested `stm32h743_blink.elf`); tests in `crates/mcs-sim/tests/stm32h7.rs`.
- [x] Stage B2, complete Cortex-M4F / M7 instruction set: ARMv7E-M DSP extension (all parallel add/sub with GE flags, SEL, USAD8/USADA8, SSAT16/USAT16, PKH, SXTB16 family, every signed/dual/most-significant-word multiply, UMAAL, sticky Q, APSR.GE in MRS/MSR; gated on `ArmFeatures::DSP`), FPv4-SP (S0-S31, FPSCR with rounding modes/FZ/DN/AHP and exact exception flags, CPACR NOCP check, VLDR/VSTR/VLDM/VSTM/VPUSH/VPOP, all VMOV forms, VMRS/VMSR, arithmetic incl. fused VFMA family, VCMP, every VCVT incl. fixed point and half precision) and FPv5-D16 (D0-D15 aliasing the S file, double precision, VSEL, VMAXNM/VMINNM, VRINT*, VCVTA/N/P/M, `vmov.32` scalar), exception entry/return with the 26-word extended frame (CONTROL.FPCA, FPCCR.ASPEN, EXC_RETURN bit 4; lazy stacking is performed eagerly). The FP arithmetic is an exact integer soft-float (`mcs_sim::arm::fpu`) with native fast paths, cross-checked against Rust f32/f64 in the unit tests. Disassembler output checked against ~1500 more `llvm-objdump` lines (`gen_ext_vectors.py`); executor tests in `crates/mcs-sim/tests/arm_core/dsp_fp.rs`

## Next
- [ ] DWARF variable/type info (`.debug_info`) for a typed Watch window and locals
- [ ] Data breakpoints (watchpoints) and conditional / hit-count breakpoints
- [ ] More virtual components on the pin panel (LED, potentiometer, logic probe); serial (UART) stimulus
- [ ] Waveform: analog traces, protocol decoders (UART, SPI, I²C), export (VCD)
- [ ] Input synchronizer latency and input-capture noise canceler delay
- [ ] More devices (issues #1, #5): ATtiny20/40, then the AVR-0/1 series (new register map)
- [ ] Virtual I²C/SPI devices on the bus (EEPROM, sensors) so TWI/SPI transfers get answers
- [ ] SPM self-programming, debugWIRE, timer asynchronous (TOSC) mode, USART synchronous / MSPIM modes
- [ ] ARM Cortex-M (STM32) targets: core, STM32G4 devices and their peripherals are wired into the session (Stage B1/C1); still missing: ADC/DMA/SPI/I2C/CAN, an ARM assembler / arm-none-eabi-gcc driver, the STM32H7 family
- [ ] ARM Cortex-M (STM32) targets: core, STM32G4 devices and their peripherals are wired into the session (Stage B1/C1); still missing: UI support, FPU/DSP instructions, ADC/DMA/SPI/I2C/CAN (the STM32H743 is wired in Stage D)
- [ ] ARM Cortex-M (STM32) targets: Thumb-2 core with DSP and FPU, NVIC and SysTick exist (Stage B1/B2); still needs the architecture abstraction below, ELF/HEX loading at 0x0800_0000 and per-family peripherals
- [ ] Architecture abstraction for non-AVR targets (machine trait, register descriptions in specs)
- [ ] Project files (multi-file C builds, per-project device/clock/fuses)
- [ ] Signed release builds (Apple notarization, Windows code signing)
