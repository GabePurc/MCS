//! ESP32-C3 family device descriptions: `ESP32-C3` (QFN32, external flash assumed 4 MiB) and `ESP32-C3FH4`
//! (4 MiB flash in the package).
//!
//! Sources:
//! * Espressif's official `esp32c3.svd` (Apache-2.0): peripheral base addresses, register offsets, bit fields, reset
//!   values and interrupt matrix source numbers (via `gen_esp32c3.py` -> `esp32c3_gen.rs`).
//! * ESP32-C3 Technical Reference Manual (v1.x): chapter "System and Memory" (internal memory address mapping, table
//!   "Internal Memory Address Mapping" and the external memory cache windows IROM / DROM), chapter "Reset and Clock"
//!   (clock sources), chapter "Interrupt Matrix", chapter "IO MUX and GPIO Matrix".
//! * ESP32-C3 Series Datasheet v2.4: QFN32 (5 x 5 mm) pin list (table 2-1 "Pin Overview"), IO MUX functions (table 2-4),
//!   strapping pins GPIO2 / GPIO8 / GPIO9, operating conditions (3.0 - 3.6 V), 160 MHz maximum CPU clock.
//!
//! Assumptions (not checked against the TRM): the exact CPU interrupt numbers reserved for the core (0, 3, 4, 7 are used by the core's
//! own interrupt sources; the simulator does not enforce this). The external flash is assumed to be 4 MiB; the real
//! module may carry 2 - 16 MiB.

use crate::arm::device::{MemRegionSpec, MmioBitSpec, MmioRegisterSpec};
use crate::avr::device::{PeripheralGroupSpec, PinKind, PinSpec, RegisterAccess};

use super::super::device::*;
use super::esp32c3_gen as g;
use super::gen_types::RegDef;

/// IROM window (instruction bus view of the flash cache).
pub const IROM_BASE: u32 = 0x4200_0000;
/// DROM window (data bus view of the flash cache).
pub const DROM_BASE: u32 = 0x3c00_0000;
pub const DRAM_BASE: u32 = 0x3fc8_0000;
pub const IRAM_BASE: u32 = 0x4038_0000;
pub const SRAM0_BASE: u32 = 0x4037_c000;
pub const RTC_FAST_BASE: u32 = 0x5000_0000;
pub const SRAM1_SIZE: u32 = 0x6_0000;
pub const SRAM0_SIZE: u32 = 0x4000;

struct Variant {
    id: &'static str,
    name: &'static str,
    package: &'static str,
    flash_external: bool,
    datasheet: &'static str,
}

const C3: Variant = Variant { id: "esp32-c3", name: "ESP32-C3", package: "QFN32", flash_external: true, datasheet: "ESP32-C3 Series Datasheet with ESP32-C3 Technical Reference Manual" };
const C3FH4: Variant = Variant { id: "esp32-c3fh4", name: "ESP32-C3FH4", package: "QFN32", flash_external: false, datasheet: "ESP32-C3 Series Datasheet (ESP32-C3FH4: 4 MiB in-package flash) with ESP32-C3 Technical Reference Manual" };

pub fn devices() -> Vec<RiscvDeviceSpec> {
    vec![build(&C3), build(&C3FH4)]
}

fn base(name: &str) -> u32 {
    g::BASES.iter().find(|b| b.0 == name).unwrap_or_else(|| panic!("base {name}")).1
}

fn access(s: &str) -> RegisterAccess {
    match s {
        "r" => RegisterAccess::R,
        "w" => RegisterAccess::W,
        _ => RegisterAccess::Rw,
    }
}

/// Adds the registers of one peripheral instance from a generated template. Register names are
/// `<instance>_<register>`; `reset` may override the reset value of a register by name.
fn expand(out: &mut Vec<MmioRegisterSpec>, inst: &str, base: u32, defs: &[RegDef], reset: impl Fn(&str, u32) -> u32) {
    for d in defs {
        out.push(MmioRegisterSpec {
            name: format!("{inst}_{}", d.name),
            addr: base + d.off,
            size: 4,
            reset: reset(d.name, d.reset),
            group: inst.to_string(),
            desc: d.desc.to_string(),
            bits: d.bits.iter().map(|b| MmioBitSpec { name: b.name.to_string(), mask: b.mask, desc: b.desc.to_string() }).collect(),
            access: access(d.access),
        });
    }
}

/// `(QFN32 pin number, name, kind, GPIO, functions)`.
type PinRow = (u8, &'static str, PinKind, i8, &'static [&'static str]);

use PinKind::{Gnd, Io, Ref, Vcc};

/// ESP32-C3 Series Datasheet v2.4, table 2-1 "Pin Overview" and table 2-4 "IO MUX Pin Functions" (QFN32). Pins 18 - 24
/// are the SPI flash interface: VDD_SPI (GPIO11 only when VDD_SPI is reconfigured as a GPIO by eFuse; modelled as the
/// supply) and GPIO12 - GPIO17. Pin 33 is the exposed ground pad.
const PINS: &[PinRow] = &[
    (1, "LNA_IN", Ref, -1, &["RF input / output"]),
    (2, "VDD3P3", Vcc, -1, &["3.3 V supply (analog)"]),
    (3, "VDD3P3", Vcc, -1, &["3.3 V supply (analog)"]),
    (4, "XTAL_32K_P", Io, 0, &["GPIO0", "XTAL_32K_P", "ADC1_CH0"]),
    (5, "XTAL_32K_N", Io, 1, &["GPIO1", "XTAL_32K_N", "ADC1_CH1"]),
    (6, "GPIO2", Io, 2, &["GPIO2", "FSPIQ", "ADC1_CH2", "strapping"]),
    (7, "CHIP_EN", Ref, -1, &["Chip enable (reset, active high)"]),
    (8, "GPIO3", Io, 3, &["GPIO3", "ADC1_CH3"]),
    (9, "MTMS", Io, 4, &["GPIO4", "MTMS", "FSPIHD", "ADC1_CH4"]),
    (10, "MTDI", Io, 5, &["GPIO5", "MTDI", "FSPIWP", "ADC2_CH0"]),
    (11, "VDD3P3_RTC", Vcc, -1, &["3.3 V supply (RTC)"]),
    (12, "MTCK", Io, 6, &["GPIO6", "MTCK", "FSPICLK"]),
    (13, "MTDO", Io, 7, &["GPIO7", "MTDO", "FSPID"]),
    (14, "GPIO8", Io, 8, &["GPIO8", "strapping"]),
    (15, "GPIO9", Io, 9, &["GPIO9", "strapping (boot mode)"]),
    (16, "GPIO10", Io, 10, &["GPIO10", "FSPICS0"]),
    (17, "VDD3P3_CPU", Vcc, -1, &["3.3 V supply (CPU IO)"]),
    (18, "VDD_SPI", Vcc, -1, &["Flash supply (GPIO11 when VDD_SPI is configured as GPIO)"]),
    (19, "SPIHD", Io, 12, &["GPIO12", "SPIHD"]),
    (20, "SPIWP", Io, 13, &["GPIO13", "SPIWP"]),
    (21, "SPICS0", Io, 14, &["GPIO14", "SPICS0"]),
    (22, "SPICLK", Io, 15, &["GPIO15", "SPICLK"]),
    (23, "SPID", Io, 16, &["GPIO16", "SPID"]),
    (24, "SPIQ", Io, 17, &["GPIO17", "SPIQ"]),
    (25, "GPIO18", Io, 18, &["GPIO18", "USB_D-"]),
    (26, "GPIO19", Io, 19, &["GPIO19", "USB_D+"]),
    (27, "U0RXD", Io, 20, &["GPIO20", "U0RXD"]),
    (28, "U0TXD", Io, 21, &["GPIO21", "U0TXD"]),
    (29, "XTAL_N", Ref, -1, &["40 MHz crystal"]),
    (30, "XTAL_P", Ref, -1, &["40 MHz crystal"]),
    (31, "VDDA", Vcc, -1, &["3.3 V analog supply"]),
    (32, "VDDA", Vcc, -1, &["3.3 V analog supply"]),
    (33, "GND", Gnd, -1, &["Exposed pad (ground)"]),
];

fn pins(v: &Variant) -> Vec<PinSpec> {
    PINS.iter()
        .map(|&(number, name, kind, gpio, funcs)| {
            // The ESP32-C3FH4 connects pins 18 - 24 (VDD_SPI, GPIO12 - GPIO17) to the in-package flash.
            let internal = !v.flash_external && (18..=24).contains(&number);
            PinSpec {
                number,
                name: if internal { format!("{name} (in-package flash)") } else { name.to_string() },
                kind: if internal { Ref } else { kind },
                gpio: (gpio >= 0 && !internal).then_some(gpio as u8),
                functions: if internal { vec!["connected to the in-package flash".to_string()] } else { funcs.iter().map(|s| s.to_string()).collect() },
            }
        })
        .collect()
}

fn memory_map(flash_size: u32, external: bool) -> Vec<RiscvMemSpec> {
    let m = |name: &str, base: u32, size: u32, perm: &str, desc: &str| RiscvMemSpec { name: name.into(), base, size, perm: perm.into(), desc: desc.into() };
    let flash = if external { "external SPI flash" } else { "in-package SPI flash" };
    vec![
        m("Boot ROM (IBUS)", 0x4000_0000, 0x6_0000, "r-x", "384 KiB internal ROM 0; contents are not simulated (executing it stops the simulation)"),
        m("Boot ROM data (DBUS)", 0x3ff0_0000, 0x2_0000, "r--", "128 KiB internal ROM 1; reads as zero"),
        m("SRAM0 (IRAM)", SRAM0_BASE, SRAM0_SIZE, "rwx", "16 KiB internal SRAM 0, instruction bus only (the hardware can use it as instruction cache)"),
        m("SRAM1 (IRAM)", IRAM_BASE, SRAM1_SIZE, "rwx", "384 KiB internal SRAM 1, instruction bus view"),
        m("SRAM1 (DRAM)", DRAM_BASE, SRAM1_SIZE, "rw-", "384 KiB internal SRAM 1, data bus view (same bytes as the IRAM view)"),
        m("RTC FAST memory", RTC_FAST_BASE, 0x2000, "rwx", "8 KiB RTC fast memory (kept in deep sleep)"),
        m("Flash (IROM)", IROM_BASE, flash_size, "r-x", &format!("{flash} through the instruction cache; simplified linear mapping of the flash image")),
        m("Flash (DROM)", DROM_BASE, flash_size, "r--", &format!("{flash} through the data cache; simplified mapping (see docs/MULTI_ARCH.md)")),
        m("Peripherals", 0x6000_0000, 0x10_0000, "rw-", "APB peripheral registers"),
    ]
}

fn build(v: &Variant) -> RiscvDeviceSpec {
    let flash_size: u32 = 4 << 20;
    let uarts = (0..2u8)
        .map(|i| {
            let name = format!("UART{i}");
            RiscvUartInstance { base: base(&name), source: if i == 0 { 21 } else { 22 }, index: i, name }
        })
        .collect::<Vec<_>>();
    let timgs = (0..2u8)
        .map(|i| {
            let name = format!("TIMG{i}");
            RiscvTimgInstance { base: base(&name), t0_source: 32 + 2 * i, wdt_source: 33 + 2 * i, index: i, name }
        })
        .collect::<Vec<_>>();

    let mut regs = Vec::new();
    let plain = |_: &str, r: u32| r;
    expand(&mut regs, "SYSTEM", base("SYSTEM"), g::SYSTEM, plain);
    expand(&mut regs, "INTERRUPT_CORE0", base("INTERRUPT_CORE0"), g::INTERRUPT_CORE0, plain);
    expand(&mut regs, "SYSTIMER", base("SYSTIMER"), g::SYSTIMER, plain);
    for t in &timgs {
        expand(&mut regs, &t.name, t.base, g::TIMG, plain);
    }
    // Direct boot: the power-on reset cause is reported (RESET_CAUSE_PROCPU = 1, POR).
    expand(&mut regs, "RTC_CNTL", base("RTC_CNTL"), g::RTC_CNTL, |n, r| if n == "RESET_STATE" { r | 1 } else { r });
    expand(&mut regs, "GPIO", base("GPIO"), g::GPIO, plain);
    expand(&mut regs, "IO_MUX", base("IO_MUX"), g::IO_MUX, plain);
    for u in &uarts {
        expand(&mut regs, &u.name, u.base, g::UART, plain);
    }
    expand(&mut regs, "USB_DEVICE", base("USB_DEVICE"), g::USB_DEVICE, plain);
    regs.sort_by_key(|r| r.addr);

    let groups = [
        ("SYSTEM", "System registers: CPU clock selection, peripheral clock enable / reset"),
        ("INTERRUPT_CORE0", "Interrupt matrix and CPU interrupt controller"),
        ("SYSTIMER", "System timer: two 52-bit counters at 16 MHz, three comparators"),
        ("TIMG0", "Timer group 0: 54-bit general-purpose timer T0 and main system watchdog"),
        ("TIMG1", "Timer group 1: 54-bit general-purpose timer T0 and main system watchdog"),
        ("RTC_CNTL", "RTC control: RTC watchdog, super watchdog, clock and reset configuration"),
        ("GPIO", "GPIO matrix: output / enable / interrupt registers and peripheral signal routing"),
        ("IO_MUX", "IO MUX: pad function selection, pull-ups / pull-downs, input enable"),
        ("UART0", "UART 0"),
        ("UART1", "UART 1"),
        ("USB_DEVICE", "USB Serial/JTAG controller (CDC-ACM serial endpoint)"),
    ]
    .into_iter()
    .map(|(n, d)| PeripheralGroupSpec { name: n.into(), desc: d.into() })
    .collect();

    let interrupts = g::INTR_SOURCES.iter().map(|&(n, s)| RiscvIrqSpec { source: s, name: n.to_string(), desc: format!("Interrupt matrix source {s} (map register INTERRUPT_CORE0 + {:#x})", 4 * s as u32) }).collect();

    RiscvDeviceSpec {
        id: v.id.into(),
        name: v.name.into(),
        family: "ESP32-C3".into(),
        core_name: "ESP-RISC-V (RV32IMC)".into(),
        isa: "rv32imc_zicsr_zifencei".into(),
        flash_base: IROM_BASE,
        drom_base: DROM_BASE,
        flash_size,
        flash_external: v.flash_external,
        sram_base: DRAM_BASE,
        sram_size: SRAM1_SIZE,
        iram_base: IRAM_BASE,
        extra_ram: vec![
            MemRegionSpec { name: "SRAM0 (IRAM only)".into(), base: SRAM0_BASE, size: SRAM0_SIZE },
            MemRegionSpec { name: "RTC FAST memory".into(), base: RTC_FAST_BASE, size: 0x2000 },
        ],
        memory_map: memory_map(flash_size, v.flash_external),
        registers: regs,
        groups,
        interrupts,
        cpu_interrupts: 31,
        package: v.package.into(),
        pins: pins(v),
        gpio_count: 22,
        strapping: vec![2, 8, 9],
        clock: RiscvClockSpec { xtal_hz: 40e6, rc_fast_hz: 17.5e6, rc_slow_hz: 136e3, systimer_hz: 16e6, cpu_max_hz: 160e6 },
        vcc: 3.3,
        vcc_range: (3.0, 3.6),
        speed_grades: vec![(160e6, 3.0)],
        datasheet: v.datasheet.into(),
        die: None,
        peripheral_set: RiscvPeripheralSet {
            family: RiscvFamily::Esp32c3,
            system_base: base("SYSTEM"),
            intc_base: base("INTERRUPT_CORE0"),
            systimer_base: base("SYSTIMER"),
            rtc_base: base("RTC_CNTL"),
            gpio_base: base("GPIO"),
            io_mux_base: base("IO_MUX"),
            usb_base: base("USB_DEVICE"),
            uarts,
            timgs,
            uart_signal_base: 6,
        },
    }
}
