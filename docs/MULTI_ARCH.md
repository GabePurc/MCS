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

## Stage D — STM32H7 (Cortex-M7, double FPU): STM32H743 — DONE (Rust side)
STM32H743IIT6 (LQFP176) and STM32H743ZIT6 (LQFP144, Nucleo-H743ZI); see `docs/ROADMAP.md`.
* Memory: `MemConfig` takes several RAM blocks and RAM aliases; `Bus` dispatches through a page table
  indexed by `addr >> 20` (regions own whole 1 MiB pages; construction panics on overlap). `ArmDeviceSpec`
  gained `flash_alias`, `extra_ram`, `ram_aliases`; the debugger memory image / session `data` still covers
  RAM block 0 only (G4: SRAM+CCM, H7: DTCM) -- a protocol/UI follow-up for the other blocks.
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
* **E2 (planned): ESP32-C3 device + Target.** `Esp32c3` device description (memory map from the TRM, IO MUX/GPIO
  matrix pins, `DeviceRef::Riscv`), interrupt matrix + the ESP32-C3 INTC (CPU interrupts 1-31 with priorities,
  thresholds and edge/level types driving `Machine::set_irq_pending`), SYSTEM/clock, GPIO, UART0/1, TIMG0/1,
  SYSTIMER; `riscv::target` implementing `Target` (registers, breakpoints, step over/out, disassembly through
  `mcs_api`), ELF `EM_RISCV` + flash image loading (`esptool`-style app images), custom CSR hook for the C3
  GPIO/performance-counter CSRs.
* **E3 (planned): UI + more peripherals.** RISC-V Processor panel (x0-x31 with ABI names, pc, mstatus/mie/mip/
  mtvec/mcause), ESP32-C3 pin diagram, serial/waveform adapted; then SPI/I2C/LEDC/ADC and the classic ESP32
  (Xtensa LX6) if wanted.

## Toolchains
RISC-V: rustc's `riscv32imc-unknown-none-elf` target assembles test programs (`global_asm!`, `.insn` for raw
encodings) and its `llvm-tools` (`llvm-objdump`, `llvm-objcopy`, `llvm-nm`) give the reference output; no
riscv-gcc is needed.

No arm-none-eabi-gcc locally; Apple clang compiles/assembles for `thumbv7em-none-eabi`. Desktop
builds use arm-none-eabi-gcc when installed (toolchain detection like avr-gcc); an ARM/GNU
assembler in `mcs-asm` is a later item.
