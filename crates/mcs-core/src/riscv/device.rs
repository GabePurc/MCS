//! Declarative description of a RISC-V microcontroller (the ESP32-C3 family): memories, memory-mapped
//! registers, interrupt matrix sources, package pins and the peripheral instances the simulator wires.
//! The simulator builds a machine from it and the UI builds its register view and pin diagram from the
//! serialized form. Register / group / pin types are shared with the ARM description.

use serde::Serialize;

use crate::arm::device::{MemRegionSpec, MmioRegisterSpec};
use crate::avr::device::{DieSpec, PeripheralGroupSpec, PinSpec};

/// One window of the CPU address map (for the Device Info panel).
#[derive(Clone, Debug, Serialize)]
pub struct RiscvMemSpec {
    pub name: String,
    pub base: u32,
    pub size: u32,
    /// Permissions as `rwx` letters (`-` where denied).
    pub perm: String,
    pub desc: String,
}

/// A peripheral source of the interrupt matrix.
#[derive(Clone, Debug, Serialize)]
pub struct RiscvIrqSpec {
    /// Source number (index of the INTERRUPT_CORE0 `*_MAP` register: offset `4 * source`).
    pub source: u8,
    pub name: String,
    pub desc: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiscvClockSpec {
    /// Main crystal (XTAL_CLK).
    pub xtal_hz: f64,
    /// Internal fast RC oscillator (RC_FAST_CLK).
    pub rc_fast_hz: f64,
    /// Internal slow RC oscillator (RC_SLOW_CLK).
    pub rc_slow_hz: f64,
    /// SYSTIMER tick rate (XTAL_CLK / 2.5).
    pub systimer_hz: f64,
    /// Highest CPU clock.
    pub cpu_max_hz: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RiscvFamily {
    Esp32c3,
}

#[derive(Clone, Debug, Serialize)]
pub struct RiscvUartInstance {
    pub name: String,
    pub base: u32,
    /// Interrupt matrix source number.
    pub source: u8,
    /// UART index (0, 1): selects the GPIO matrix signals and the SYSTEM clock-enable / reset bit.
    pub index: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct RiscvTimgInstance {
    pub name: String,
    pub base: u32,
    /// Sources of timer T0 and of the main system watchdog.
    pub t0_source: u8,
    pub wdt_source: u8,
    pub index: u8,
}

/// Peripheral wiring recipe: which instances the machine factory creates, and where.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiscvPeripheralSet {
    pub family: RiscvFamily,
    pub system_base: u32,
    pub intc_base: u32,
    pub systimer_base: u32,
    /// RTC_CNTL (the EFUSE block shares its 4 KiB page at +0x800).
    pub rtc_base: u32,
    pub gpio_base: u32,
    pub io_mux_base: u32,
    pub usb_base: u32,
    pub uarts: Vec<RiscvUartInstance>,
    pub timgs: Vec<RiscvTimgInstance>,
    /// First GPIO matrix signal number of UART n's TX (RX input); `uart_signal_base + 3 * n` (TXD/RXD).
    pub uart_signal_base: u8,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiscvDeviceSpec {
    pub id: String,
    pub name: String,
    pub family: String,
    pub core_name: String,
    /// ISA string of the hart.
    pub isa: String,
    /// IROM window: the flash image as seen by the instruction bus (also the program-memory base the UI uses).
    pub flash_base: u32,
    /// DROM window: the flash image as seen by the data bus.
    pub drom_base: u32,
    pub flash_size: u32,
    /// The flash is a separate chip on the SPI pins (false: in-package flash, e.g. ESP32-C3FH4).
    pub flash_external: bool,
    /// Data-bus base of the main SRAM (SRAM1); the session `data` covers this block.
    pub sram_base: u32,
    pub sram_size: u32,
    /// Instruction-bus base of SRAM1.
    pub iram_base: u32,
    /// Further RAM blocks the memory view can watch: SRAM0 (IRAM only), RTC FAST memory.
    pub extra_ram: Vec<MemRegionSpec>,
    /// The CPU address map.
    pub memory_map: Vec<RiscvMemSpec>,
    pub registers: Vec<MmioRegisterSpec>,
    pub groups: Vec<PeripheralGroupSpec>,
    /// Interrupt matrix sources.
    pub interrupts: Vec<RiscvIrqSpec>,
    /// Number of CPU interrupt lines (1..=31).
    pub cpu_interrupts: u8,
    pub package: String,
    pub pins: Vec<PinSpec>,
    /// Number of GPIOs (GPIO0 .. GPIO(n-1)); pin index = GPIO number.
    pub gpio_count: u8,
    /// GPIO numbers of the strapping pins.
    pub strapping: Vec<u8>,
    pub clock: RiscvClockSpec,
    pub vcc: f64,
    pub vcc_range: (f64, f64),
    pub speed_grades: Vec<(f64, f64)>,
    pub datasheet: String,
    pub die: Option<DieSpec>,
    pub peripheral_set: RiscvPeripheralSet,
}

impl RiscvDeviceSpec {
    pub fn register(&self, name: &str) -> Option<&MmioRegisterSpec> {
        self.registers.iter().find(|r| r.name.eq_ignore_ascii_case(name))
    }

    /// Address of a register (panics if missing: wiring bug).
    pub fn reg(&self, name: &str) -> u32 {
        self.register(name).unwrap_or_else(|| panic!("{}: register {name} not defined", self.name)).addr
    }

    /// GPIO names indexed by GPIO number ("GPIO0" ...).
    pub fn gpio_names(&self) -> Vec<String> {
        (0..self.gpio_count).map(|i| format!("GPIO{i}")).collect()
    }
}
