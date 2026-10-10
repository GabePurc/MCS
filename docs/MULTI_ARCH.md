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
C1 (Rust side: devices, peripherals, `Target`, loaders, tests) is DONE; see `docs/ROADMAP.md`. Remaining: ADC, UI (C2).
RCC (HSI16/HSE/PLL, bus prescalers), FLASH ACR wait states, GPIO A-G (MODER/OTYPER/OSPEEDR/
PUPDR/IDR/ODR/BSRR/AFR), EXTI + SYSCFG, NVIC, SysTick, USART/LPUART, TIM2-TIM4/TIM6/TIM7,
ADC (later). UI: Processor panel for Cortex-M registers, peripheral register view from the spec.

## Stage D — STM32H7 (Cortex-M7, double FPU): STM32H743
Large chips like STM32H743IIT6 (LQFP-176): RCC/PWR domains, GPIO A-K, USART, TIM, SysTick.

## Stage E — ESP32
ESP32-C3 (RISC-V RV32IMC) first (simpler core, same seam), then the classic ESP32 (Xtensa LX6)
if wanted.

## Toolchains
No arm-none-eabi-gcc locally; Apple clang compiles/assembles for `thumbv7em-none-eabi`. Desktop
builds use arm-none-eabi-gcc when installed (toolchain detection like avr-gcc); an ARM/GNU
assembler in `mcs-asm` is a later item.
