//! STM32H743 device tests: programs assembled by Apple clang (Cortex-M7, FPv5-D16; see
//! `stm32h7/gen_programs.py`, `stm32h7/programs/*.s`) run on `mcs_sim::arm::Machine::from_spec`, plus
//! Session-level tests that load a tiny linked ELF (`mcs-formats/tests/data/stm32h743_blink.elf`,
//! built by `stm32h7/make_elf.py`). Expected values follow RM0433 (and the CMSIS header / SVD) and
//! the peripheral models' documented behaviour.

#[path = "stm32h7/programs.rs"]
mod programs;

use mcs_core::arm::device::ArmDeviceSpec;
use mcs_core::arm::devices;
use mcs_core::arm::thumb::ArmFeatures;
use mcs_core::device::DeviceRef;
use mcs_core::program::{LoadedProgram, ProgramFormat};
use mcs_sim::arm::{Machine, StopReason};
use mcs_sim::avr::peripherals::serial::SerialConfig;
use mcs_sim::pins::ExtDrive;
use mcs_sim::protocol::{Command, CoreState, Output, SpeedMode, StopKind};
use mcs_sim::session::Session;
use mcs_sim::target::Target;
use programs::*;

const ZI: &str = "stm32h743zit6";
const II: &str = "stm32h743iit6";
const RCC: u32 = 0x5802_4400;
const PWR: u32 = 0x5802_4800;
const FLASH: u32 = 0x5200_2000;
const SYSCFG: u32 = 0x5800_0400;
const EXTI_R: u32 = 0x5800_0000;
const GPIOA: u32 = 0x5802_0000;
const GPIOB: u32 = 0x5802_0400;
const GPIOC: u32 = 0x5802_0800;
const GPIOE: u32 = 0x5802_1000;
const SCB: u32 = 0xE000_ED00;
const PB0: usize = 16;
const PE1: usize = 4 * 16 + 1;
const PC13: usize = 2 * 16 + 13;
const PD8: usize = 3 * 16 + 8;
const PD9: usize = 3 * 16 + 9;

fn spec(id: &str) -> &'static ArmDeviceSpec {
    devices::get(id).unwrap()
}

fn machine(id: &str, p: &Prog) -> Machine {
    let s = spec(id);
    let mut m = Machine::from_spec(s);
    let mut prog = LoadedProgram::empty(ProgramFormat::Elf, s.flash_size as usize);
    prog.flash[..p.code.len()].copy_from_slice(p.code);
    prog.flash_used = p.code.len() as u32;
    m.load_program(Some(&prog));
    m
}

fn boot(p: &Prog) -> Machine {
    machine(ZI, p)
}

fn boot_at(p: &Prog, entry: &str) -> Machine {
    let mut m = boot(p);
    m.cpu.pc = p.sym(entry);
    m
}

/// Runs to the next BKPT and returns its address.
fn run_to_bkpt(m: &mut Machine) -> u32 {
    let limit = m.cpu.cycles + 1_000_000_000;
    assert_eq!(m.run(limit), StopReason::Bkpt, "pc={:#x} messages={:?}", m.cpu.pc, m.sys.messages);
    m.cpu.pc - 2
}

fn rd(m: &mut Machine, addr: u32) -> u32 {
    m.mem_read(addr, 4).unwrap_or_else(|| panic!("read {addr:#x}"))
}

fn wr(m: &mut Machine, addr: u32, v: u32) {
    assert!(m.mem_write(addr, 4, v), "write {addr:#x}");
}

fn set_bits(m: &mut Machine, addr: u32, bits: u32) {
    let v = rd(m, addr);
    wr(m, addr, v | bits);
}

fn rd64(m: &mut Machine, addr: u32) -> u64 {
    rd(m, addr) as u64 | (rd(m, addr + 4) as u64) << 32
}

/// (cycle, level) changes of GPIO `pin` recorded in the pin trace.
fn pin_edges(m: &Machine, pin: usize) -> Vec<(u64, u8)> {
    let words = m.sys.trace.words();
    let (_, cycles, levels) = m.sys.trace.read_since_wide(0, usize::MAX);
    let mut out: Vec<(u64, u8)> = Vec::new();
    for (i, &c) in cycles.iter().enumerate() {
        let lv = (levels[i * words + pin / 32] >> (pin % 32) & 1) as u8;
        if out.last().map(|e| e.1) != Some(lv) {
            out.push((c, lv));
        }
    }
    out
}

/// A machine in the reset state with HSE (8 MHz) on and a valid PLL1 set up but not enabled.
fn warn_texts(m: &Machine) -> Vec<String> {
    m.sys.messages.iter().map(|x| x.text.clone()).collect()
}

// ---------------------------------------------------------------------------------------------
// Devices, memory
// ---------------------------------------------------------------------------------------------

#[test]
fn device_descriptions() {
    for (id, pins, package) in [(II, 176, "LQFP176"), (ZI, 144, "LQFP144")] {
        let s = spec(id);
        assert_eq!(s.family, "STM32H7");
        assert_eq!(s.package, package);
        assert_eq!(s.pins.len(), pins, "{id}: package pins");
        assert_eq!(s.core_name, "Cortex-M7");
        assert_eq!(s.features, ArmFeatures::CORTEX_M7.0, "DSP + FPv5-D16");
        assert_eq!(s.cpuid, 0x411F_C271, "Cortex-M7 r1p1");
        assert_eq!((s.flash_base, s.flash_size, s.flash_alias), (0x0800_0000, 2 << 20, false));
        assert_eq!((s.nirq, s.nvic_prio_bits, s.gpio_count), (150, 4, 176));
        assert_eq!(s.speed_grades[0].0, 480e6);
        assert_eq!(s.clock.hsi_hz, 64e6);
        assert!(s.vcc_range.0 <= 1.62 && s.vcc_range.1 >= 3.6);
        // Registers are unique and sorted by address; ports A-K exist.
        assert!(s.registers.windows(2).all(|w| w[0].addr < w[1].addr), "{id}: register addresses");
        assert_eq!(s.reg("GPIOK_MODER"), 0x5802_2800);
        assert_eq!(s.reg("RCC_AHB4ENR"), RCC + 0xe0);
        assert_eq!(s.reg("EXTI_IMR1"), EXTI_R + 0x80);
        assert_eq!(s.reg("SYSCFG_EXTICR1"), SYSCFG + 8);
        assert_eq!(s.reg("MPU_RASR_A3"), 0xE000_EDB8);
        assert_eq!(s.register("GPIOA_MODER").unwrap().reset, 0xABFF_FFFF);
        assert_eq!(s.register("PWR_CR3").unwrap().reset, 6);
        assert_eq!(s.register("FLASH_ACR").unwrap().reset, 0x37);
        // IRQ names / numbers (RM0433 table 144).
        let irq = |n: &str| s.vectors.iter().find(|v| v.name == n).unwrap_or_else(|| panic!("{n}")).index - 16;
        assert_eq!((irq("TIM2"), irq("USART3"), irq("EXTI15_10"), irq("UART8"), irq("LPUART1"), irq("WAKEUP_PIN")), (28, 39, 40, 83, 142, 149));
        // Nucleo pins: PB0 / PE1 LEDs, PC13 button, PD8 / PD9 virtual COM port with AF7.
        let pin = |n: &str| s.pins.iter().find(|p| p.name.starts_with(n)).unwrap_or_else(|| panic!("{id} {n}"));
        assert_eq!(pin("PC13").gpio, Some(PC13 as u8));
        assert_eq!(pin("PB0").gpio, Some(PB0 as u8));
        assert!(pin("PD8").functions.iter().any(|f| f == "USART3_TX"));
        assert!(s.peripheral_set.alt_functions.contains(&(PD8 as u8, 7, "USART3_TX")));
        assert!(s.peripheral_set.alt_functions.contains(&(PD9 as u8, 7, "USART3_RX")));
        assert_eq!(s.peripheral_set.uarts.len(), 9);
        assert_eq!(s.peripheral_set.timers.len(), 6);
    }
    // The 144-pin package has fewer pins; PI0-PI11 only exist on the 176-pin one (PJ / PK need BGA240 or larger).
    assert!(spec(II).pins.iter().any(|p| p.name.starts_with("PI0")));
    assert!(!spec(ZI).pins.iter().any(|p| p.name.starts_with("PI0")));
    assert!(!spec(II).pins.iter().any(|p| p.name.starts_with("PK")));
}

#[test]
fn reset_takes_sp_and_pc_from_the_vector_table_in_flash() {
    let mut m = boot(&BOOT);
    assert_eq!(m.cpu.r[13], 0x2002_0000);
    assert_eq!(m.cpu.pc, BOOT.sym("reset"));
    assert_eq!(m.scb.vtor, 0x0800_0000, "BOOT_ADD0 = flash start; no alias at 0");
    assert_eq!(run_to_bkpt(&mut m), BOOT.sym("done"));
    assert_eq!(rd(&mut m, 0x2000_0100), 0xcafe_babe);
    assert!(matches!(m.device(), DeviceRef::Arm(s) if s.id == ZI));
    assert_eq!(rd(&mut m, SCB), 0x411F_C271, "CPUID");
    // HSI 64 MHz out of reset.
    assert_eq!(m.sys.clk.hclk_hz, 64e6);
    assert_eq!(m.sys.clock.hz, 64e6);
}

#[test]
fn every_ram_region_reads_and_writes_and_the_boundaries_hold() {
    let mut m = boot(&BOOT);
    // (base, size) of ITCM, DTCM, AXI SRAM, SRAM1-3, SRAM4, backup SRAM.
    for (base, size) in [(0x0000_0000u32, 64u32 << 10), (0x2000_0000, 128 << 10), (0x2400_0000, 512 << 10), (0x3000_0000, 288 << 10), (0x3800_0000, 64 << 10), (0x3880_0000, 4 << 10)] {
        for a in [base, base + 4, base + size - 4] {
            assert!(m.mem_write(a, 4, a ^ 0x5a5a_5a5a), "write {a:#x}");
            assert_eq!(m.mem_read(a, 4), Some(a ^ 0x5a5a_5a5a), "read {a:#x}");
        }
        assert!(m.mem_write(base + 1, 1, 0xab));
        assert_eq!(m.mem_read(base, 4).map(|v| v >> 8 & 0xff), Some(0xab));
        assert_eq!(m.mem_read(base + size, 4), None, "nothing beyond {base:#x}+{size:#x}");
        assert!(!m.mem_write(base + size, 4, 0));
    }
    // SRAM1-3 are also visible at 0x1000_0000 (same memory).
    assert!(m.mem_write(0x3000_0040, 4, 0x1234_5678));
    assert_eq!(m.mem_read(0x1000_0040, 4), Some(0x1234_5678));
    assert!(m.mem_write(0x1004_7ffc, 4, 0x9999_0000));
    assert_eq!(m.mem_read(0x3004_7ffc, 4), Some(0x9999_0000));
    // Flash is read-only from the bus; both banks are readable (2 MiB, second bank at 0x0810_0000).
    assert!(!m.mem_write(0x0800_0000, 4, 0));
    assert!(m.mem_read(0x0810_0000, 4).is_some() && m.mem_read(0x081f_fffc, 4).is_some());
    assert_eq!(m.mem_read(0x0820_0000, 4), None);
    // Gaps between the regions fault.
    for a in [0x0001_0000u32, 0x2002_0000, 0x2008_0000, 0x2408_0000, 0x3004_8000, 0x3801_0000] {
        assert_eq!(m.mem_read(a, 4), None, "{a:#x}");
    }
}

#[test]
fn ram_blocks_flash_and_itcm_code_via_a_program() {
    let mut m = boot(&MEM);
    assert_eq!(run_to_bkpt(&mut m), MEM.sym("done"));
    let got: Vec<u32> = (0..5).map(|k| rd(&mut m, 0x2000_0100 + 4 * k)).collect();
    assert_eq!(got, [0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444, 0x2222_2222], "AXI, SRAM1, SRAM4, backup, SRAM1 alias");
    assert_eq!(rd(&mut m, 0x2000_0114), 42, "function copied into the ITCM ran");
    assert!(m.sys.messages.is_empty(), "{:?}", m.sys.messages);
}

#[test]
fn unmapped_peripheral_space_reads_zero_with_a_warning() {
    let mut m = boot(&BOOT);
    assert_eq!(m.mem_read(0x4002_2000, 4), Some(0)); // ADC1 is not modelled
    assert!(m.mem_write(0x4002_2000, 4, 1));
    assert!(m.sys.messages.iter().any(|x| x.text.contains("0x40022000")), "{:?}", m.sys.messages);
}

// ---------------------------------------------------------------------------------------------
// Cortex-M7 system registers
// ---------------------------------------------------------------------------------------------

#[test]
fn cortex_m7_cache_maintenance_mpu_and_tcm_registers() {
    let mut m = boot(&BOOT);
    // CCR: IC / DC / BP are stored; STKALIGN stays.
    assert_eq!(rd(&mut m, SCB + 0x14), 0x200);
    wr(&mut m, SCB + 0x14, 7 << 16 | 0x200);
    assert_eq!(rd(&mut m, SCB + 0x14), 7 << 16 | 0x200);
    // Cache identification: separate I and D caches, 16 KiB, 4 ways, 128 sets.
    assert_eq!(rd(&mut m, SCB + 0x78), 0x0900_0003, "CLIDR");
    wr(&mut m, SCB + 0x84, 0);
    let ccsidr = rd(&mut m, SCB + 0x80);
    assert_eq!(((ccsidr >> 13) & 0x7fff, (ccsidr >> 3) & 0x3ff), (127, 3), "NumSets - 1, Associativity - 1");
    wr(&mut m, SCB + 0x84, 1);
    assert_eq!(rd(&mut m, SCB + 0x84), 1);
    assert_ne!(rd(&mut m, SCB + 0x80), ccsidr, "instruction cache has its own CCSIDR");
    // Maintenance operations are accepted and ignored.
    for off in [0xE000_EF50u32, 0xE000_EF58, 0xE000_EF5C, 0xE000_EF60, 0xE000_EF64, 0xE000_EF68, 0xE000_EF6C, 0xE000_EF70, 0xE000_EF74, 0xE000_EF78] {
        assert!(m.mem_write(off, 4, 0x2000_0000), "{off:#x}");
    }
    // The memory is unaffected by the "invalidate".
    wr(&mut m, 0x2400_0000, 0xfeed_f00d);
    wr(&mut m, 0xE000_EF5C, 0x2400_0000);
    assert_eq!(rd(&mut m, 0x2400_0000), 0xfeed_f00d);
    // TCM control.
    wr(&mut m, 0xE000_EF90, 1);
    wr(&mut m, 0xE000_EF94, 1);
    assert_eq!((rd(&mut m, 0xE000_EF90), rd(&mut m, 0xE000_EF94)), (1, 1));
    // MPU: 16 regions, registers stored and aliased (not enforced).
    assert_eq!(rd(&mut m, 0xE000_ED90), 16 << 8);
    wr(&mut m, 0xE000_ED9C, 0x2400_0000 | 0x10 | 5); // VALID, region 5
    assert_eq!(rd(&mut m, 0xE000_ED98), 5, "RNR follows RBAR.VALID");
    assert_eq!(rd(&mut m, 0xE000_ED9C), 0x2400_0005);
    wr(&mut m, 0xE000_EDA0, 0x1300_0000 | 0x1f << 1 | 1);
    assert_eq!(rd(&mut m, 0xE000_EDA0), 0x1300_003f);
    wr(&mut m, 0xE000_ED98, 4);
    wr(&mut m, 0xE000_EDA4, 0x3000_0000); // alias 1 = region 5
    assert_eq!(rd(&mut m, 0xE000_ED9C), 4, "region 4 is untouched by the alias write");
    wr(&mut m, 0xE000_ED98, 5);
    assert_eq!(rd(&mut m, 0xE000_ED9C) & !0x1f, 0x3000_0000, "alias 1 wrote region 5");
    wr(&mut m, 0xE000_ED94, 5);
    assert_eq!(rd(&mut m, 0xE000_ED94), 5);
    // ... and does not block anything: an enabled MPU without regions still lets code run.
    assert!(m.mem_write(0x2000_0000, 4, 1));
    // ACTLR is stored.
    wr(&mut m, 0xE000_E008, 0x1);
    assert_eq!(rd(&mut m, 0xE000_E008), 1);
}

// ---------------------------------------------------------------------------------------------
// RCC, FLASH, PWR, SysTick
// ---------------------------------------------------------------------------------------------

fn pll_run(entry: &str, reload: u32, mhz: f64) -> Machine {
    let mut m = boot_at(&PLL, entry);
    assert_eq!(run_to_bkpt(&mut m), PLL.sym("pll_done"));
    assert_eq!(rd(&mut m, 0x2000_0100), 0x1b, "SW = SWS = PLL1");
    assert_eq!(m.sys.clk.sysclk_hz, mhz * 1e6);
    assert_eq!(m.sys.clk.hclk_hz, mhz * 1e6, "CPU clock (D1CPRE /1)");
    assert_eq!(m.sys.clock.hz, mhz * 1e6);
    // HPRE /2, all APB prescalers /2: AXI / AHB = cpu / 2, PCLK = cpu / 4, timers at the AHB clock.
    let c = m.sys.clk;
    assert_eq!((c.ppre1, c.ppre2, c.ppre3, c.ppre4), (4, 4, 4, 4));
    assert_eq!((c.tim1, c.tim2), (2, 2));
    assert!(m.sys.messages.is_empty(), "correct bring-up sequence: {:?}", m.sys.messages);
    // The cycle counter now runs at the new CPU clock: 1 ms of SysTick = Hz / 1000 cycles = 1 ms.
    wr(&mut m, 0x2000_0104, reload);
    let mut stops = Vec::new();
    for _ in 0..3 {
        assert!(run_to_bkpt(&mut m) > PLL.sym("spin"), "BKPT of the SysTick handler");
        stops.push((m.cpu.cycles, m.sys.time_at(m.cpu.cycles)));
    }
    for w in stops.windows(2) {
        let (dc, dt) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
        let want = (mhz * 1e3) as u64;
        assert!((want - 3..=want + 3).contains(&dc), "SysTick period {dc} cycles");
        assert!((dt - 1e-3).abs() < 2e-8, "SysTick period {dt} s");
    }
    m
}

#[test]
fn pwr_vos1_and_pll1_to_400_mhz_cpu_frequency_changes_and_systick_follows() {
    let mut m = pll_run("reset400", 399_999, 400.0);
    assert_eq!(m.sys.vos_level(), 1);
    assert_eq!(rd(&mut m, FLASH) & 0xf, 2);
}

#[test]
fn pwr_vos0_overdrive_and_pll1_to_480_mhz() {
    let mut m = pll_run("reset480", 479_999, 480.0);
    assert_eq!(m.sys.vos_level(), 0, "VOS1 + SYSCFG_PWRCR.ODEN");
    assert_eq!(rd(&mut m, FLASH) & 0xf, 4);
    assert_eq!(rd(&mut m, PWR + 0x18) & 0xe000, 0xe000, "VOS1 (0b11) and VOSRDY");
    assert_eq!(rd(&mut m, PWR + 4) & 0xe000, 0xe000, "ACTVOS = 0b11, ACTVOSRDY");
    // The cycle counter advanced at 64 MHz first and 480 MHz afterwards: wall time stays continuous.
    let t = m.sys.time_at(m.cpu.cycles);
    assert!(t > 0.0 && t < 0.01, "{t}");
}

#[test]
fn rcc_warns_about_missing_voltage_scaling_and_wait_states() {
    let mut m = boot_at(&PLL, "reset_bad");
    run_to_bkpt(&mut m);
    let w = warn_texts(&m).join("\n");
    assert!(w.contains("CPU clock 480 MHz exceeds the 200 MHz allowed at VOS3"), "{w}");
    assert!(w.contains("AXI / AHB clock 240 MHz exceeds the 100 MHz"), "{w}");
    assert!(w.contains("APB1 clock 120 MHz exceeds the 50 MHz"), "{w}");
    // Reset leaves 7 wait states; fewer than needed is reported.
    let mut m = Machine::from_spec(spec(ZI));
    wr(&mut m, FLASH, 0);
    wr(&mut m, PWR + 0x18, 0xc000);
    set_bits(&mut m, RCC, 1 << 16);
    wr(&mut m, RCC + 0x28, 2 | 1 << 4);
    wr(&mut m, RCC + 0x2c, 3 << 2 | 1 << 16);
    wr(&mut m, RCC + 0x30, 99 | 1 << 9 | 1 << 16 | 1 << 24);
    set_bits(&mut m, RCC, 1 << 24);
    wr(&mut m, RCC + 0x18, 0x48); // HPRE /2: 400 MHz CPU, 200 MHz AXI; D1PPRE /2
    wr(&mut m, RCC + 0x1c, 0x440);
    wr(&mut m, RCC + 0x20, 0x40);
    wr(&mut m, RCC + 0x10, 3);
    let w = warn_texts(&m).join("\n");
    assert!(w.contains("needs FLASH_ACR.LATENCY >= 2"), "{w}");
    assert!(!w.contains("exceeds"), "VOS1 allows 400 / 200 MHz: {w}");
}

#[test]
fn pll_range_checks_and_fractional_divider() {
    let mut m = Machine::from_spec(spec(ZI));
    set_bits(&mut m, RCC, 1 << 16); // HSEON, 8 MHz
    wr(&mut m, RCC + 0x28, 2 | 2 << 4); // DIVM1 = 2: 4 MHz reference
    wr(&mut m, RCC + 0x2c, 1 << 16 | 1 << 0 | 2 << 2); // FRACEN, RGE = 4-8 MHz
    wr(&mut m, RCC + 0x30, 99 | 1 << 9 | 1 << 16 | 1 << 24); // N = 100
    wr(&mut m, RCC + 0x34, 4096 << 3); // FRACN = 0.5
    set_bits(&mut m, RCC, 1 << 24);
    assert_ne!(rd(&mut m, RCC) & 1 << 25, 0, "PLL1RDY");
    // VCO = 4 * (100 + 0.5) = 402 MHz in the wide range; P = /2 = 201 MHz.
    wr(&mut m, RCC + 0x10, 3);
    assert_eq!(m.sys.clk.sysclk_hz, 201e6);
    // DIVx are locked while the PLL runs.
    wr(&mut m, RCC + 0x30, 49 | 1 << 9 | 1 << 16 | 1 << 24);
    assert_eq!(rd(&mut m, RCC + 0x30) & 0x1ff, 99);
    // 4 MHz falls in the 4-8 MHz range (RGE = 2): no warning, and the VCO (402 MHz) is in range.
    assert!(warn_texts(&m).iter().all(|w| !w.contains("PLL1")), "{:?}", warn_texts(&m));
    // A PLL1RGE that does not match the reference is reported.
    let mut m = Machine::from_spec(spec(ZI));
    set_bits(&mut m, RCC, 1 << 16);
    wr(&mut m, RCC + 0x28, 2 | 1 << 4); // 8 MHz reference
    wr(&mut m, RCC + 0x2c, 1 << 16 | 1 << 2); // RGE = 1 (2-4 MHz)
    wr(&mut m, RCC + 0x30, 99 | 1 << 9 | 1 << 16 | 1 << 24);
    set_bits(&mut m, RCC, 1 << 24);
    assert!(warn_texts(&m).iter().any(|w| w.contains("PLL1RGE = 1 does not match")), "{:?}", warn_texts(&m));
    // A VCO outside the range selected by VCOSEL is reported (medium range is 150-420 MHz).
    let mut m = Machine::from_spec(spec(ZI));
    set_bits(&mut m, RCC, 1 << 16);
    wr(&mut m, RCC + 0x28, 2 | 1 << 4);
    wr(&mut m, RCC + 0x2c, 1 << 16 | 3 << 2 | 1 << 1); // VCOSEL = medium
    wr(&mut m, RCC + 0x30, 99 | 1 << 9 | 1 << 16 | 1 << 24); // 800 MHz
    set_bits(&mut m, RCC, 1 << 24);
    assert!(warn_texts(&m).iter().any(|w| w.contains("PLL1 VCO at 800 MHz is outside")), "{:?}", warn_texts(&m));
    // A reference outside 1-16 MHz is reported; DIVM = 0 disables the PLL.
    let mut m = Machine::from_spec(spec(ZI));
    wr(&mut m, RCC + 0x28, 0); // HSI 64 MHz, DIVM1 = 0
    set_bits(&mut m, RCC, 1 << 24);
    assert_eq!(rd(&mut m, RCC) & 1 << 25, 0, "no lock with DIVM = 0");
    let v = rd(&mut m, RCC) & !(1 << 24);
    wr(&mut m, RCC, v);
    wr(&mut m, RCC + 0x28, 1 << 4); // DIVM1 = 1: 64 MHz reference (DIVM is locked while PLL1 runs)
    set_bits(&mut m, RCC, 1 << 24);
    assert_ne!(rd(&mut m, RCC) & 1 << 25, 0);
    assert!(warn_texts(&m).iter().any(|w| w.contains("PLL1 input clock 64 MHz is outside 1-16 MHz")), "{:?}", warn_texts(&m));
}

#[test]
fn rcc_registers_prescalers_and_timer_clock_ratios() {
    let mut m = Machine::from_spec(spec(ZI));
    let cr = rd(&mut m, RCC);
    assert_eq!(cr & 0x83, 0x83, "HSION | HSIKERON | CSION");
    assert_ne!(cr & (1 << 2 | 1 << 5 | 1 << 8), 0, "HSIRDY | HSIDIVF | CSIRDY");
    assert_eq!(rd(&mut m, RCC + 0x10), 0, "SW = SWS = HSI");
    assert_eq!(rd(&mut m, RCC + 0x28), 0x0202_0200);
    assert_eq!(rd(&mut m, RCC + 0x2c), 0x01ff_0000);
    assert_eq!(rd(&mut m, RCC + 0x30), 0x0101_0280);
    // HSIDIV /2 -> 32 MHz.
    wr(&mut m, RCC, (cr & !0x18) | 1 << 3);
    assert_eq!(m.sys.clk.sysclk_hz, 32e6);
    wr(&mut m, RCC, cr);
    // HSE (8 MHz crystal) as SYSCLK, D1CPRE /2 -> CPU 4 MHz, HPRE /4, D2PPRE1 /4, D2PPRE2 /16, D3PPRE /8, D1PPRE /2.
    set_bits(&mut m, RCC, 1 << 16);
    wr(&mut m, RCC + 0x10, 2);
    assert_eq!(rd(&mut m, RCC + 0x10) & 0x38, 2 << 3, "SWS = HSE");
    wr(&mut m, RCC + 0x18, 0x8 << 8 | 0x4 << 4 | 0x9);
    wr(&mut m, RCC + 0x1c, 0x7 << 8 | 0x5 << 4);
    wr(&mut m, RCC + 0x20, 0x6 << 4);
    let c = m.sys.clk;
    assert_eq!((c.sysclk_hz, c.hclk_hz), (8e6, 4e6));
    assert_eq!((c.ppre1, c.ppre2, c.ppre3, c.ppre4), (16, 64, 8, 32), "CPU cycles per PCLK1-4");
    // TIMPRE = 0: timers run at 2 x PCLK when the prescaler is not 1; TIMPRE = 1: 4 x from /4 on.
    assert_eq!((c.tim1, c.tim2), (8, 32));
    set_bits(&mut m, RCC + 0x10, 1 << 15);
    let c = m.sys.clk;
    assert_eq!((c.tim1, c.tim2), (4, 16));
    // D2PPRE /1 and /2 give a timer clock equal to the AHB clock (HPRE /4 here).
    wr(&mut m, RCC + 0x1c, 0x4 << 4);
    assert_eq!(m.sys.clk.tim1, 4);
    wr(&mut m, RCC + 0x1c, 0);
    assert_eq!(m.sys.clk.tim1, 4);
    // SW = 5 is reserved and ignored; HSI cannot be switched off while it is not in use ... but it can now.
    wr(&mut m, RCC + 0x10, 5 | 1 << 15);
    assert_eq!(rd(&mut m, RCC + 0x10) & 7, 2);
    // HSE is in use: HSEON cannot be cleared.
    let v = rd(&mut m, RCC) & !(1 << 16);
    wr(&mut m, RCC, v);
    assert_ne!(rd(&mut m, RCC) & 1 << 16, 0);
    // Switching to HSI and dropping HSE works; with HSE gone while selected, SYSCLK falls back to HSI.
    wr(&mut m, RCC + 0x10, 0);
    let v = rd(&mut m, RCC) & !(1 << 16);
    wr(&mut m, RCC, v);
    assert_eq!(rd(&mut m, RCC) & 1 << 17, 0, "HSERDY cleared");
}

#[test]
fn csi_and_external_clock_input() {
    let mut m = Machine::from_spec(spec(ZI));
    wr(&mut m, RCC + 0x10, 1); // SW = CSI (4 MHz, on at reset)
    assert_eq!(m.sys.clk.sysclk_hz, 4e6);
    wr(&mut m, RCC + 0x10, 0);
    m.set_external_clock(25e6);
    set_bits(&mut m, RCC, 1 << 16);
    wr(&mut m, RCC + 0x10, 2);
    assert_eq!(m.sys.clk.sysclk_hz, 25e6, "HSE follows the external clock input");
}

#[test]
fn flash_pwr_and_syscfg_registers() {
    let mut m = Machine::from_spec(spec(ZI));
    // FLASH: reset 7 wait states, 3 = WRHIGHFREQ; both banks locked until their key sequence.
    assert_eq!(rd(&mut m, FLASH), 0x37);
    wr(&mut m, FLASH, 0x24);
    assert_eq!(rd(&mut m, FLASH), 0x24);
    assert_eq!(m.sys.flash_latency, 4);
    assert_eq!(rd(&mut m, FLASH + 0x0c) & 1, 1, "CR1.LOCK");
    wr(&mut m, FLASH + 0x04, 0x4567_0123);
    wr(&mut m, FLASH + 0x04, 0xCDEF_89AB);
    assert_eq!(rd(&mut m, FLASH + 0x0c) & 1, 0, "bank 1 unlocked");
    assert_eq!(rd(&mut m, FLASH + 0x10c) & 1, 1, "bank 2 still locked");
    wr(&mut m, FLASH + 0x104, 0x4567_0123);
    wr(&mut m, FLASH + 0x104, 0xCDEF_89AB);
    assert_eq!(rd(&mut m, FLASH + 0x10c) & 1, 0);
    let v = rd(&mut m, FLASH + 0x0c) | 1;
    wr(&mut m, FLASH + 0x0c, v);
    assert_eq!(rd(&mut m, FLASH + 0x0c) & 1, 1, "LOCK set again by software");
    // PWR: LDO supply, VOS3 at reset, voltage scaling is instantaneous.
    assert_eq!(rd(&mut m, PWR + 0x0c), 6, "LDOEN | SCUEN");
    assert_ne!(rd(&mut m, PWR + 4) & 1 << 13, 0, "ACTVOSRDY");
    assert_eq!(rd(&mut m, PWR + 0x18) & 0xe000, 0x6000, "VOS3 (0b01) | VOSRDY");
    assert_eq!(m.sys.vos_level(), 3);
    wr(&mut m, PWR + 0x18, 0x8000);
    assert_eq!(m.sys.vos_level(), 2);
    wr(&mut m, PWR + 0x18, 0xc000);
    assert_eq!(m.sys.vos_level(), 1);
    // ODEN needs the SYSCFG clock (APB4ENR bit 1).
    wr(&mut m, SYSCFG + 0x2c, 1);
    assert_eq!(rd(&mut m, SYSCFG + 0x2c), 0, "SYSCFG not clocked");
    assert_eq!(m.sys.vos_level(), 1);
    wr(&mut m, RCC + 0xf4, 2);
    wr(&mut m, SYSCFG + 0x2c, 1);
    assert_eq!(rd(&mut m, SYSCFG + 0x2c), 1);
    assert_eq!(m.sys.vos_level(), 0);
    // CR2.BREN -> BRRDY.
    wr(&mut m, PWR + 0x08, 1);
    assert_ne!(rd(&mut m, PWR + 0x08) & 1 << 16, 0);
}

// ---------------------------------------------------------------------------------------------
// GPIO, EXTI
// ---------------------------------------------------------------------------------------------

#[test]
fn gpio_leds_toggle_and_the_button_reads_back() {
    let mut m = Machine::from_spec(spec(ZI));
    assert_eq!(rd(&mut m, GPIOB), 0, "GPIOB is not clocked yet (AHB4ENR)");
    assert_eq!(rd(&mut m, GPIOA), 0);
    wr(&mut m, RCC + 0xe0, 1 << 1 | 1 << 2 | 1 << 4); // GPIOB, GPIOC, GPIOE
    assert_eq!(rd(&mut m, GPIOB), 0xFFFF_FEBF);
    assert_eq!(rd(&mut m, GPIOE), 0xFFFF_FFFF);
    wr(&mut m, GPIOB, 0xFFFF_FEBD); // PB0 output
    wr(&mut m, GPIOE, 0xFFFF_FFF7); // PE1 output
    wr(&mut m, GPIOC, 0xF3FF_FFFF); // PC13 input
    wr(&mut m, GPIOB + 0x18, 1);
    wr(&mut m, GPIOE + 0x18, 2);
    assert_eq!((m.sys.pins[PB0].level, m.sys.pins[PE1].level), (1, 1));
    wr(&mut m, GPIOB + 0x18, 1 << 16);
    assert_eq!((m.sys.pins[PB0].level, m.sys.pins[PE1].level), (0, 1));
    assert_eq!(rd(&mut m, GPIOE + 0x14), 2, "ODR");
    assert_eq!(pin_edges(&m, PB0).iter().map(|e| e.1).collect::<Vec<_>>(), [0, 1, 0]);
    // Button B1 (active high on the Nucleo-144).
    assert_eq!(rd(&mut m, GPIOC + 0x10) & 1 << 13, 0);
    m.set_pin_input(PC13, ExtDrive::High, 3.3);
    assert_ne!(rd(&mut m, GPIOC + 0x10) & 1 << 13, 0);
    // Pull-down and reset through AHB4RSTR.
    wr(&mut m, RCC + 0x88, 1 << 1);
    assert_eq!(rd(&mut m, GPIOB), 0xFFFF_FEBF, "GPIOB reset");
    wr(&mut m, RCC + 0x88, 0);
    // Port K (176-pin device) exists with its own clock bit.
    let mut m = Machine::from_spec(spec(II));
    assert_eq!(rd(&mut m, 0x5802_2800), 0);
    wr(&mut m, RCC + 0xe0, 1 << 10);
    assert_eq!(rd(&mut m, 0x5802_2800), 0xFFFF_FFFF);
    assert_eq!(m.sys.pins.len(), 176);
}

#[test]
fn exti15_10_interrupt_from_the_user_button() {
    let mut m = boot(&EXTI);
    assert_eq!(m.run(m.cpu.cycles + 20_000), StopReason::Limit);
    assert!(m.cpu.sleeping, "WFI loop");
    let count = |m: &mut Machine| rd(m, 0x2000_0200);
    assert_eq!(count(&mut m), 0);
    assert_eq!(rd(&mut m, EXTI_R + 0x80) & 1 << 13, 1 << 13, "IMR1 line 13");
    m.set_pin_input(PC13, ExtDrive::High, 3.3);
    m.run(m.cpu.cycles + 1_000);
    assert_eq!(count(&mut m), 1, "rising edge");
    assert_eq!(rd(&mut m, EXTI_R + 0x88), 0, "PR1 cleared by the handler");
    assert_eq!((m.sys.pins[PB0].level, m.sys.pins[PE1].level), (1, 1), "handler lit LD1 and LD2");
    m.set_pin_input(PC13, ExtDrive::Low, 0.0);
    m.run(m.cpu.cycles + 1_000);
    assert_eq!(count(&mut m), 1, "falling edge is not enabled");
    m.set_pin_input(PC13, ExtDrive::High, 3.3);
    m.run(m.cpu.cycles + 1_000);
    assert_eq!(count(&mut m), 2);
    // Software interrupt request (SWIER1 at +0x08).
    wr(&mut m, EXTI_R + 0x08, 1 << 13);
    m.run(m.cpu.cycles + 1_000);
    assert_eq!(count(&mut m), 3);
    // EXTI0-4 use their own IRQ, lines 10-15 share IRQ 40.
    let s = spec(ZI);
    assert_eq!(s.peripheral_set.exti_irqs[0], 6);
    assert_eq!(s.peripheral_set.exti_irqs[13], 40);
    assert_eq!(s.peripheral_set.exti_irqs[7], 23);
}

// ---------------------------------------------------------------------------------------------
// USART, timers, FPU
// ---------------------------------------------------------------------------------------------

#[test]
fn usart3_transmits_to_the_nucleo_virtual_com_port_and_the_serial_monitor() {
    let mut m = boot_at(&USART, "tx_main");
    m.set_serial(SerialConfig { monitor: Some(PD8), inject: None, baud: 115_200.0, data_bits: 8, parity: 0, stop_bits: 1 });
    assert_eq!(run_to_bkpt(&mut m), USART.sym("tx_sent"));
    assert!(m.sys.serial_out.is_empty(), "no frame is complete yet");
    assert_eq!(run_to_bkpt(&mut m), USART.sym("tx_done"));
    assert_eq!(m.sys.serial_out, b"Hi");
    // Two back-to-back 8N1 frames, BRR = 556 -> 556 cycles per bit (HSI 64 MHz, PCLK1 = 64 MHz).
    let mut bits = Vec::new();
    for b in *b"Hi" {
        bits.push(0u8);
        bits.extend((0..8).map(|k| b >> k & 1));
        bits.push(1);
    }
    let edges = pin_edges(&m, PD8);
    let t0 = edges.iter().find(|e| e.1 == 0 && e.0 > 0).expect("start bit").0;
    let mut want = vec![(t0, 0u8)];
    let mut level = 0;
    for (k, &b) in bits.iter().enumerate() {
        if b != level {
            want.push((t0 + 556 * k as u64, b));
            level = b;
        }
    }
    let got: Vec<(u64, u8)> = edges.into_iter().filter(|e| e.0 >= t0).collect();
    assert_eq!(got, want);
    assert!(m.cpu.cycles >= t0 + 556 * 20);
    assert_eq!(m.sys.pins[PD8].dir, 1);
}

#[test]
fn usart3_receives_bytes_from_the_serial_monitor() {
    let mut m = boot(&IDLE);
    wr(&mut m, RCC + 0xe0, 1 << 3);
    wr(&mut m, RCC + 0xe8, 1 << 18);
    wr(&mut m, 0x5802_0c00, 0xFFFA_FFFF);
    wr(&mut m, 0x5802_0c24, 0x77);
    let u = 0x4000_4800;
    wr(&mut m, u + 0x0c, 556);
    wr(&mut m, u, 5); // UE | RE
    m.set_serial(SerialConfig { monitor: None, inject: Some(PD9), baud: 115_200.0, data_bits: 8, parity: 0, stop_bits: 1 });
    m.serial_send(b"AB");
    m.run(m.cpu.cycles + 2 * 5560 + 1000);
    let isr = rd(&mut m, u + 0x1c);
    assert_ne!(isr & 1 << 5, 0, "RXNE");
    assert_ne!(isr & 1 << 3, 0, "ORE: 'B' arrived while 'A' was unread");
    assert_eq!(rd(&mut m, u + 0x24), b'A' as u32);
}

#[test]
fn every_uart_and_timer_instance_is_mapped_with_its_bus_clock() {
    let mut m = Machine::from_spec(spec(ZI));
    // (instance base, RCC enable register offset, bit)
    let uarts = [(0x4001_1000u32, 0xf0u32, 4u32), (0x4000_4400, 0xe8, 17), (0x4000_4800, 0xe8, 18), (0x4000_4c00, 0xe8, 19), (0x4000_5000, 0xe8, 20), (0x4001_1400, 0xf0, 5), (0x4000_7800, 0xe8, 30), (0x4000_7c00, 0xe8, 31), (0x5800_0c00, 0xf4, 3)];
    for (base, en, bit) in uarts {
        assert_eq!(rd(&mut m, base + 0x1c), 0, "{base:#x} not clocked");
        set_bits(&mut m, RCC + en, 1 << bit);
        assert_eq!(rd(&mut m, base + 0x1c), 0xc0, "{base:#x} ISR reset value");
    }
    // TIM2 / TIM5 are 32-bit, TIM3 / TIM4 16-bit, TIM6 / TIM7 basic.
    wr(&mut m, RCC + 0xe8, 0x3f | 1 << 17);
    assert_eq!(rd(&mut m, 0x4000_002c), 0xffff_ffff);
    assert_eq!(rd(&mut m, 0x4000_0c2c), 0xffff_ffff, "TIM5");
    assert_eq!(rd(&mut m, 0x4000_042c), 0xffff);
    assert_eq!(rd(&mut m, 0x4000_082c), 0xffff);
    wr(&mut m, 0x4000_1400 + 0x34, 5);
    assert_eq!(rd(&mut m, 0x4000_1400 + 0x34), 0, "TIM7 has no CCR1");
}

#[test]
fn tim2_update_interrupt_period() {
    let mut m = boot(&TIM);
    let mut stops = Vec::new();
    for _ in 0..4 {
        let at = run_to_bkpt(&mut m);
        assert!(at > TIM.sym("tim2_run"));
        stops.push(m.cpu.cycles);
    }
    // PSC = 63, ARR = 999 on the 64 MHz timer clock: exactly 64000 cycles (1 ms) between updates.
    // Individual stops jitter by a few cycles (the interrupt is taken at an instruction boundary); the
    // update events themselves are exactly periodic.
    for w in stops.windows(2) {
        assert!((w[1] - w[0]).abs_diff(64_000) <= 4, "{stops:?}");
    }
    assert_eq!(stops[3] - stops[0], 3 * 64_000, "{stops:?}");
    let frames = m.call_stack();
    assert!(frames.iter().any(|f| f.vector == 44), "IRQ 28 = exception 44: {frames:?}");
}

#[test]
fn tim2_follows_the_timer_kernel_clock_after_a_clock_change() {
    // HSE 8 MHz, HPRE /1, D2PPRE1 /2: timer clock = 2 x PCLK1 = 8 MHz = CPU clock -> 1 cycle per count.
    let mut m = boot(&IDLE);
    set_bits(&mut m, RCC, 1 << 16);
    wr(&mut m, RCC + 0x10, 2);
    wr(&mut m, RCC + 0x1c, 0x4 << 4);
    wr(&mut m, RCC + 0xe8, 1);
    wr(&mut m, 0x4000_0000 + 0x2c, 99);
    wr(&mut m, 0x4000_0000, 1);
    let c0 = m.cpu.cycles;
    m.run(c0 + 50);
    let cnt = rd(&mut m, 0x4000_0024) as u64;
    assert!(cnt.abs_diff(m.cpu.cycles - c0) <= 8, "one count per CPU cycle: cnt {cnt}, elapsed {}", m.cpu.cycles - c0);
}

fn f64_at(m: &mut Machine, off: u32) -> f64 {
    f64::from_bits(rd64(m, 0x2000_0100 + off))
}

#[test]
fn double_precision_fpu_program() {
    let mut m = boot(&FPU);
    assert_eq!(run_to_bkpt(&mut m), FPU.sym("done"));
    assert_eq!(f64_at(&mut m, 0), 1.5 * 2.25);
    assert_eq!(f64_at(&mut m, 8), 1.0 / 3.0);
    assert_eq!(f64_at(&mut m, 16), 2f64.sqrt());
    assert_eq!(f64_at(&mut m, 24), 0.1f64.mul_add(10.0, -1.0), "fused multiply-add rounds once");
    assert_ne!(f64_at(&mut m, 24), 0.0);
    assert_eq!(f64_at(&mut m, 32), 123.0);
    assert_eq!(f64_at(&mut m, 40), -6.5);
    assert_eq!(f64_at(&mut m, 48), 1.0, "1 + 2^-53 rounds to even");
    assert_eq!(rd(&mut m, 0x2000_0138) & 0xf000_0000, 0x2000_0000, "vcmp 3.0 > 2.0: C set");
    assert!(m.sys.messages.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Target / Session
// ---------------------------------------------------------------------------------------------

const BLINK_ELF: &[u8] = include_bytes!("../../mcs-formats/tests/data/stm32h743_blink.elf");

fn blink_program() -> LoadedProgram {
    mcs_formats::load_program_file_at(BLINK_ELF, "stm32h743_blink.elf", 2 << 20, 0x0800_0000)
}

fn states(outs: &[Output]) -> Vec<&mcs_sim::protocol::MachineState> {
    outs.iter().filter_map(|o| if let Output::State { state } = o { Some(&**state) } else { None }).collect()
}

fn sym(p: &LoadedProgram, name: &str) -> u32 {
    p.symbols.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("symbol {name}")).address
}

#[test]
fn session_runs_an_elf_on_the_stm32h743zit6() {
    let p = blink_program();
    assert!(!p.has_errors(), "{:?}", p.diagnostics);
    let mut s = Session::new();
    let outs = s.handle(Command::Init { device_id: ZI.into() });
    assert!(outs.iter().any(|o| matches!(o, Output::Device { spec: DeviceRef::Arm(d) } if d.id == ZI && d.pins.len() == 144)));
    let reset = sym(&p, "reset");
    let outs = s.handle(Command::Load { device_id: ZI.into(), program: Box::new(p.clone()) });
    let st = states(&outs).pop().unwrap();
    assert_eq!(st.pc, reset);
    assert!(matches!(st.core, CoreState::Arm { .. }));
    s.handle(Command::SetSpeed { mode: SpeedMode::Max, factor: 1.0 });
    let mut outs = s.handle(Command::Run);
    for _ in 0..5 {
        outs.extend(s.slice());
    }
    outs.extend(s.handle(Command::Pause));
    let edges: usize = states(&outs).iter().map(|st| st.trace_cycles.len()).sum();
    let outs = s.handle(Command::RequestState);
    let st = states(&outs).pop().unwrap();
    let CoreState::Arm { r, xpsr, msp, .. } = &st.core else { panic!("ARM core state") };
    assert_eq!(msp, &0x2002_0000, "initial stack pointer from the vector table");
    assert_ne!(xpsr & (1 << 24), 0, "Thumb bit");
    assert_eq!(r[15], st.pc);
    assert!(st.cycles > 1000 && st.instructions > 100);
    assert_eq!(st.hz, 64e6);
    assert_eq!(st.pins.len(), 176, "pin array covers ports A-K for both packages");
    assert_eq!(st.io.len(), spec(ZI).registers.len());
    assert!(edges >= 4, "PB0 toggles ({edges} trace entries)");
    // GPIOB is clocked and PB0 is an output.
    let idx = spec(ZI).registers.iter().position(|r| r.name == "GPIOB_MODER").unwrap();
    assert_eq!(st.io[idx] & 3, 1);
    // The debugger memory image covers the main RAM (DTCM, 128 KiB).
    assert!(st.data.len() <= 128 * 1024);
}

#[test]
fn session_on_the_176_pin_device_and_breakpoints() {
    let p = blink_program();
    let mut s = Session::new();
    let outs = s.handle(Command::Init { device_id: II.into() });
    assert!(outs.iter().any(|o| matches!(o, Output::Device { spec: DeviceRef::Arm(d) } if d.id == II && d.pins.len() == 176)));
    s.handle(Command::Load { device_id: II.into(), program: Box::new(p.clone()) });
    let delay = sym(&p, "delay");
    s.handle(Command::SetBreakpoints { pcs: vec![delay] });
    s.handle(Command::SetSpeed { mode: SpeedMode::Max, factor: 1.0 });
    s.handle(Command::Run);
    let mut stop = None;
    for _ in 0..20 {
        for o in s.slice() {
            if let Output::State { state } = o {
                if state.stop.is_some() {
                    stop = state.stop.clone();
                }
            }
        }
        if stop.is_some() {
            break;
        }
    }
    let stop = stop.expect("breakpoint hit");
    assert_eq!((stop.reason, stop.pc), (StopKind::Breakpoint, delay));
}

#[test]
fn power_cycle_restores_the_reset_clock_tree() {
    let mut m = pll_run("reset400", 399_999, 400.0);
    m.power_cycle();
    assert_eq!(m.sys.clk.hclk_hz, 64e6);
    assert_eq!(m.sys.vos_level(), 3);
    assert_eq!(rd(&mut m, RCC + 0x10), 0);
    assert_eq!(m.scb.vtor, 0x0800_0000);
    assert_eq!(m.cpu.pc, PLL.sym("reset400"));
}
