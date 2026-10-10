# Multi-architecture plan (issues #4, #5: STM32 G4/H7, ESP32)

MCS was AVR-only. Supporting ARM Cortex-M (STM32) and later ESP32 needs a seam between the
architecture-neutral layers (session, protocol, API, most of the UI) and per-architecture
cores, devices and peripherals. Work is staged so every stage ships with all tests green.

## Stage A — architecture seam (no new targets) — DONE
* `mcs_core::device::{Arch, DeviceRef}`: `DeviceRef::Avr(&'static AvrDeviceSpec)` (later
  `Arm(..)`); `devices::get_any(id)` / `list_any()` search every architecture. Specs serialize
  with an `arch` tag (`"avr"`) so the UI can branch.
* `mcs_sim::target::Target`: object-safe trait implemented by `avr::Machine`; `Session` owns a
  `Box<dyn Target>`. One dynamic call per time slice / command, never per instruction: the AVR
  executor hot path is untouched.
* `MachineState`: architecture-specific CPU state moves into `core: CoreState`
  (`Avr { sp, sreg, regs }`); `pc` stays in the architecture's native unit and the state adds
  `pc_bytes` (byte address) for neutral consumers. Memory travels as before for AVR.
* Implemented: `mcs_core::{device, devices}`, `mcs_sim::target::Target` (+ `avr/target.rs`),
  `CoreState`, the UI's `DeviceSpec` / `CoreState` types and `avrCore(state)` accessor. AVR-only
  state that is not part of `core` (`fuses`, `lock`, `eeprom`, `sleepMode`) stays at the top
  level of `MachineState` (empty / 0 on other architectures) until a second architecture needs
  a different shape.
* UI: `DeviceSpec = AvrDeviceSpec | ...` discriminated by `arch`; AVR-only panels (fuses,
  I/O view, Chip View, ISA, definitions) check `arch === 'avr'`.

## Stage B — ARMv7-M core (Cortex-M4/M7 integer, then DSP + FPU)
* `mcs_core::arm`: Thumb/Thumb-2 decoder + disassembler table (16/32-bit encodings), register
  descriptions.
* `mcs_sim::arm`: CPU (r0-r15, xPSR, MSP/PSP, CONTROL, PRIMASK/FAULTMASK/BASEPRI), memory bus
  (flash, SRAM, bit-band where present, MMIO dispatch to peripherals by address range),
  exception entry/return (NVIC priorities, tail-chaining kept simple), SysTick, SCB.
  Pre-decoded flash like the AVR core; event-driven peripherals on the same scheduler.
* Loading: ELF (`EM_ARM`) and Intel HEX at 0x0800_0000; vector table at the flash base.
* Tests: reference encodings from `clang --target=thumbv7em-none-eabi` and `llvm-objdump`
  (`xcrun --find llvm-objdump`), checked in as byte arrays.
* B1 (done): integer ISA, exceptions, NVIC, SysTick, SCB.
* B2 (done): DSP extension, FPv4-SP (M4F) and FPv5-D16 (M7).
  * Feature gating: `ArmFeatures::{DSP, FPV4_SP, FPV5_DP}` at decode time; `ArmConfig::{default
    (M4F), cortex_m7(), cortex_m3()}` select the core. Undefined encodings raise UNDEFINSTR.
  * FP state (`Cpu::fpr` S0-S31 with D0-D15 aliasing pairs, `Cpu::fpscr`) lives in the CPU struct;
    arithmetic is `mcs_sim::arm::fpu` (exact soft-float with FPSCR semantics, native f32/f64 fast
    paths for round-to-nearest without flush-to-zero). Fused ops are single-rounded.
  * CPACR gates every FP instruction (NOCP UsageFault); CP10 is consulted (CP11 must match).
  * Exception entry stacks the 26-word extended frame when CONTROL.FPCA is set; EXC_RETURN bit 4
    selects the frame on return. Lazy stacking (FPCCR.LSPEN) is performed eagerly; LSPACT never
    sets. On entry FPSCR takes FPDSCR and FPCA is cleared (handlers start with a clean FP state).
  * Cycle counts: DSP 1; Cortex-M4 TRM table 3-1 / FPU (ARM DDI 0439B) for FPv4-SP; Cortex-M7
    is dual issue and approximated with the same single-issue counts (double VDIV/VSQRT 29).
  * Simplifications: FPSID is approximate (implementer + subarchitecture only), instructions for
    the other coprocessors decode as UNDEFINSTR rather than NOCP, CPACR CP11 is not consulted
    separately from CP10.

## Stage C — STM32G4 (Cortex-M4F): STM32G431/G474
C1 (Rust side: devices, peripherals, `Target`, loaders, tests) and C2 (UI) are DONE; see `docs/ROADMAP.md`. Remaining: ADC.
RCC (HSI16/HSE/PLL, bus prescalers), FLASH ACR wait states, GPIO A-G (MODER/OTYPER/OSPEEDR/
PUPDR/IDR/ODR/BSRR/AFR), EXTI + SYSCFG, NVIC, SysTick, USART/LPUART, TIM2-TIM4/TIM6/TIM7,
ADC (later). UI (C2): Processor panel for Cortex-M registers incl. FPU, peripheral register view from the spec, memory / pins / waveform / serial adapted to ARM (quad LQFP drawing, only existing GPIOs). The UI's program counter is in the architecture's native unit everywhere (`pcToBytes` / `bytesToPc`; `BuildInfo.arch`); AVR-only features are disabled for ARM devices.

## Stage D — STM32H7 (Cortex-M7, double FPU): STM32H743 — DONE (Rust side and UI)
STM32H743IIT6 (LQFP176) and STM32H743ZIT6 (LQFP144, Nucleo-H743ZI); see `docs/ROADMAP.md`.
* Memory: `MemConfig` takes several RAM blocks and RAM aliases; `Bus` dispatches through a page table
  indexed by `addr >> 20` (regions own whole 1 MiB pages; construction panics on overlap). `ArmDeviceSpec`
  gained `flash_alias`, `extra_ram`, `ram_aliases`. The session `data` still covers RAM block 0 only (G4:
  SRAM+CCM, H7: DTCM); the other blocks are streamed through the `watchRam { index }` command (0 = none,
  k = `extra_ram[k - 1]`) into `MachineState.ramExtra { index, data }`, sent on the first state after the
  selection / a load and when the bytes changed (cached copy `Dbg.extra_sent`). The memory panel lists
  each block next to Flash / SRAM and edits them with byte `writeMem`; Device Info lists them in the map.
* Peripherals: `ArmPeripheralSet.family` (`Stm32G4` / `Stm32H7`) selects the RCC/PWR/FLASH/SYSCFG-EXTI models
  (`periph/h7.rs` for the H7, `SysExti` is parameterised by `ExtiLayout`); GPIO, USART/UART/LPUART and timers
  are the G4 models. `ClockTree` has four APB ratios and explicit timer ratios (cycles of the CPU clock per
  PCLKn / timer kernel tick); the cycle counter counts CPU clock cycles (sys_ck / D1CPRE).
* Simplifications: immediate oscillator/PLL lock and voltage scaling, no kernel-clock muxes (CCIPR), caches
  and MPU are register-level only, limits (VOS frequencies, wait states, PLL ranges) are advisory warnings.

## Stage E — ESP32
ESP32-C3 (RISC-V RV32IMC) first (simpler core, same seam), then the classic ESP32 (Xtensa LX6)
if wanted.

* **E1 (done): standalone RV32IMC core**, mirroring Stage B1; not wired into the session yet.
  * `mcs_core::riscv`: `decode` turns the first 16/32 bits into a 12-byte `Insn { imm, op, rd, rs1, rs2, len }`
    (RV32I + M + C + Zicsr + Zifencei + `mret`/`wfi`; compressed forms expand to their base `Op`, `len` stays
    2/4; everything else, incl. FP/atomics/RV64-only C forms/`sret`, is `Op::Illegal` with the raw bits in `imm`).
    `disassemble`/`format_insn` reproduce `llvm-objdump` (pseudo-instructions `li mv j ret nop neg not seqz ...`,
    `rdcycle`-style CSR aliases, hex immediates, `c.`-form for compressed HINTs, `lpad`/`prefetch.*`/`ntl.*` hints,
    CSR names generated into `csr_names.rs`). Reference vectors come from the rustc `llvm-objdump`
    (`tests/riscv_decode/gen_vectors.py`; `--full` + `RISCV_FULL=<file>` checks all 16-bit encodings and 180k words).
  * `mcs_sim::riscv`: `Cpu` (x0-x31, pc, cycle/instret counters, CSR file in `cpu.rs`), `Bus` (`bus.rs`: 4 KiB
    page table over aliasing windows with R/W/X permissions, `Mmio` trait with a `Cx` to raise/lower interrupt
    lines, lazily pre-decoded code pages cleared by every store/`Bus::load`/DMA write so self-modifying code and
    IRAM loads need no `fence.i`), `Machine` (`machine.rs`: allocation-free run loop, traps, interrupts, `wfi`).
    Interrupt lines 1-31 map to `mip` bits; priority MEI(11) > MSI(3) > MTI(7) > highest line number; vectored
    mode enters at `base + 4 * cause`, exceptions always at `base`.
  * Cycle model (approximation; the TRM publishes no instruction timing): ALU/CSR/store/fence 1, load 2, mul 1,
    div/rem 33, `jal` 2, taken branch and `jalr` 3, not-taken branch 1, trap entry and `mret` 3; no cache/flash wait
    states or load-use stalls. `mcycle` counts these cycles (also those added by `Machine::idle`).
  * Simplifications: M-mode only (no U-mode/PMP; MPP reads as 3), `time`/`mcountinhibit`/hpm counters absent or
    read-as-zero, misaligned loads/stores trap (like the ESP32-C3 core), `ebreak` can be a host breakpoint
    (`halt_on_ebreak`), `mtval` holds the instruction bits for illegal instructions and the pc for `ebreak`.
* **E2 (done, Rust side; UI in E3): ESP32-C3 device + `Target`.**
  * Devices: `ESP32-C3` (QFN32, external flash assumed 4 MiB) and `ESP32-C3FH4` (4 MiB in-package flash; pins 18-24 are
    connected to it; pinout from datasheet v2.4 tables 2-1 / 2-4) as `DeviceRef::Riscv` (`mcs_core::riscv::{device, devices}`, arch tag `"riscv"`). Peripheral base
    addresses, register offsets/fields/reset values (about 560 registers for the UI register view) and interrupt matrix
    source numbers come from Espressif's official `esp32c3.svd` (Apache-2.0) through `gen_esp32c3.py` -> `esp32c3_gen.rs`
    (the SVD is not checked in); memory map, clocks, pins and boot behaviour are from the TRM / datasheet and cited in
    the module docs, with the unverified items marked as assumptions (GPIO matrix constant-input encoding, `GPIO_STRAP_REG` bit layout).
  * Memory: ROM (384 KiB IBUS / 128 KiB DBUS) is mapped but empty and not executable; the machine stops with
    `StopReason::RomCall` (message with the ROM address and `ra`) when the pc enters it. SRAM0 16 KiB (IRAM only) +
    SRAM1 384 KiB (IRAM 0x4038_0000 and DRAM 0x3FC8_0000 alias one memory), RTC FAST 8 KiB at 0x5000_0000. Flash is a
    simplified cache/MMU: the flash image is mapped linearly at IROM 0x4200_0000 (offset = vaddr - base) and, because
    IDF-style images link IROM and DROM both at +0x20, DROM 0x3C00_0000 starts at a 64 KiB aligned flash offset behind the
    IROM data (`Bus::set_window_offset`, chosen at load time); no page-granular MMU, cache or wait states. The peripheral
    page 0x6000_0000-0x600D_0000 is filled with catch-all devices (reads 0, writes ignored, one warning per 4 KiB page).
  * Loading is "direct boot": `LoadedProgram.segments` (new; absolute-address chunks) hold the ELF `PT_LOAD` segments
    (`EM_RISCV` 243) or the segments of an ESP-IDF app image (`.bin`, magic 0xE9, `mcs_formats::espimage`); `Esp32c3::load`
    writes them to flash/RAM by address, re-applies the RAM ones at every power-on, and starts at the entry point with the
    peripherals in their reset state (CPU on XTAL/2 = 20 MHz as per the SYSTEM reset values, `mie` all ones, watchdogs
    disabled, no ROM or 2nd stage bootloader, no flash encryption/secure boot). `flash`/`flash_base` of the program carry the
    IROM part for the disassembly view. DWARF line info and symbols work as for ARM.
  * Peripherals, all event driven on the shared scheduler (`riscv::bus::Cx` carries scheduler, `Sys` and the interrupt
    controller; devices are dispatched through `Mmio` with `on_event`/`on_pin`/`on_clock_change`/`reset`; scheduler events due
    before a register access are dispatched first, so reads are exact): SYSTEM (CPU clock: XTAL/(PRE_DIV+1), PLL 80/160 MHz,
    RC_FAST/(PRE_DIV+1); APB 80 MHz under the PLL, else = CPU; clock-enable gating for UART/TIMG, `PERIP_RST_EN` resets the
    peripheral, `CPU_INTR_FROM_CPU_n` software interrupts); INTERRUPT_CORE0 (62 sources -> CPU interrupts 1-31 with enable,
    level/edge type, priority 0-15, threshold; the controller presents the single winner - highest priority, lowest number among
    equals, `priority >= threshold` - to the core as the matching `mip` bit); SYSTIMER (2 x 52-bit units at 16 MHz, 3 comparators,
    target and period modes); TIMG0/1 T0 (54-bit, divider, up/down, alarm with auto-reload, XTAL or APB source; MWDT registers
    stored, never reset); RTC_CNTL (stored; `SW_SYS_RST` resets the system; RTC/super watchdogs never fire); GPIO + IO MUX (22 pads,
    W1TS/W1TC, matrix `OUT_SEL`/`OEN_SEL`/`IN_SEL` for UART0/1, pull-ups, input enable, GPIO interrupts via source 16);
    UART0/1 (128-byte FIFOs, CLKDIV/FRAG baud from the selected source, bit-level TX/RX on the matrix signals, threshold/timeout/
    done/error interrupts, loopback); USB Serial/JTAG EP1 (bytes appear in the Serial Monitor at once). The Serial Monitor is
    bridged to UART0 (GPIO21 TX / GPIO20 RX, 115200 8N1) by default. Custom CSRs through `CsrHook`: PMP registers (stored),
    trigger module and performance counters (read 0, writes ignored), dedicated-GPIO CSRs (stored, unconnected).
  * `Esp32c3` (`mcs_sim::riscv::esp32c3`) owns the `Machine` and implements `Target`: load/reset/power cycle, breakpoints (the
    machine's `CHK` loop variant, a 64-bit bloom filter keeps checked runs near full speed), run-to, step into/over/out at
    instruction and source level (call depth from `jal(r)`/`ret`, trap nesting ignored), call stack from `ra` + a stack scan for
    return addresses with static targets recovered from `jal` / `auipc`+`jalr`, pins/serial/stimulus, debugger writes,
    snapshots with `CoreState::Riscv` and the register values (`io`). `mcs-api` disassembles RISC-V flash images; the AVR
    assemblers refuse RISC-V devices. Tests: `crates/mcs-sim/tests/esp32c3.rs` with programs built by
    `tests/esp32c3/gen_programs.py` (rustc `riscv32imc` + rust-lld, ELFs checked in under `tests/esp32c3/elf/`).
  * Simplifications / not modelled: the flash MMU/cache (above), instruction timing (E1 table, no flash wait states), the
    RTC slow clock/timer, deep/light sleep, EFUSE (reads 0, so no MAC address), RNG, SPI/I2C/LEDC/ADC/TWAI/RMT/DMA/crypto
    (catch-all devices), the USB host-to-device direction, GPIO sleep/hold features, UART flow control/RS485/IrDA/autobaud, the
    fractional `SCLK_DIV_A/B`, PMP enforcement, dedicated GPIO routing, JTAG. TIMG watchdogs and RTC watchdogs are stored only.
* **E3 (UI: done): ESP32-C3 in the UI**, mirroring Stage C2; more peripherals and the classic ESP32 remain (below).
  * RISC-V Processor panel (`RiscvProcessorPanel`, helpers in `services/riscvState.ts`): x0-x31 with ABI names, pc, cycle / instret,
    mstatus (MIE / MPIE, MPP), mie / mip bit boxes, mtvec (base + mode), mepc, mcause decoded, mtval, mscratch; edits go through
    `writeReg` (x1-x31) and `writeCpu` (`pc`, `mstatus`, `mie`, `mtvec`, `mepc`, `mcause`, `mtval`, `mscratch`).
  * Shared with ARM: the memory-mapped register view (`MmioIoView`, `writeMem`), the memory view (bus addresses, `watchRam` for
    SRAM0 / RTC FAST; the DRAM view notes the IRAM alias), the quad-package drawing (exposed pad = the pin numbered after the
    perimeter), pin lists (`existingGpios`), pc helpers (`pcToBytes` = bytes).
  * ESP32-C3 Device Info (`RiscvDeviceInfo`), pins labelled `GPIOn` (`pinLabel`), serial monitor on UART0 by default
    (GPIO21 / GPIO20) with the USB Serial/JTAG bytes merged in by the backend.
  * Examples: `examples/esp32c3_blink.elf` (GPIO2) and `examples/esp32c3_hello.elf` (UART0), generated from
    `tests/esp32c3/programs/ex_*.s` (self-contained, compressed instructions) and tested in `tests/examples.rs`.
* **Later: more peripherals** (SPI/I2C/LEDC/ADC) and the classic ESP32 (Xtensa LX6) if wanted.

## Toolchains
RISC-V: rustc's `riscv32imc-unknown-none-elf` target assembles test programs (`global_asm!`, `.insn` for raw
encodings) and its `llvm-tools` (`llvm-objdump`, `llvm-objcopy`, `llvm-nm`) give the reference output; no
riscv-gcc is needed.

No arm-none-eabi-gcc locally; Apple clang compiles/assembles for `thumbv7em-none-eabi`. Desktop
builds use arm-none-eabi-gcc when installed (toolchain detection like avr-gcc); an ARM/GNU
assembler in `mcs-asm` is a later item.
