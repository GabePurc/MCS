//! Declarative description of an ARM Cortex-M microcontroller (memories, memory-mapped
//! registers, interrupt vectors, package pins, peripheral instances). The simulator wires a
//! machine from it ([`ArmPeripheralSet`]) and the UI builds its peripheral register view and pin
//! diagram from the serialized form. Field names mirror
//! [`AvrDeviceSpec`](crate::avr::device::AvrDeviceSpec) where the concept is the same.

use serde::Serialize;

use crate::avr::device::{DieSpec, PeripheralGroupSpec, PinSpec, RegisterAccess};

#[derive(Clone, Debug, Serialize)]
pub struct MmioBitSpec {
    pub name: String,
    /// Bit mask within the 32-bit register (multi-bit fields use contiguous masks).
    pub mask: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub desc: String,
}

/// One memory-mapped register (peripheral or core).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MmioRegisterSpec {
    /// `<instance>_<register>`, e.g. `GPIOA_MODER`.
    pub name: String,
    pub addr: u32,
    /// Register width in bytes.
    pub size: u8,
    pub reset: u32,
    /// Peripheral instance the register belongs to (for the register tree).
    pub group: String,
    pub desc: String,
    pub bits: Vec<MmioBitSpec>,
    pub access: RegisterAccess,
}

/// An exception / interrupt vector. `index` is the exception number: 1 Reset ... 15 SysTick,
/// 16 + n for external interrupt n.
#[derive(Clone, Debug, Serialize)]
pub struct ArmVectorSpec {
    pub index: u16,
    pub name: String,
    pub desc: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArmClockSpec {
    pub hsi_hz: f64,
    pub lsi_hz: f64,
    pub hse_min_hz: f64,
    pub hse_max_hz: f64,
    /// Crystal frequency assumed by the simulator until the user changes it.
    pub hse_default_hz: f64,
}

/// Core-coupled SRAM that is visible both at its own base and as the tail of the main SRAM.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CcmSpec {
    /// Dedicated (code-bus) address of the CCM SRAM.
    pub base: u32,
    pub size: u32,
    /// Address where the same memory appears contiguously after the main SRAM.
    pub alias_base: u32,
}

/// Position of a peripheral's clock-enable / reset bit: RCC register `reg` (0 AHB1, 1 AHB2,
/// 2 AHB3, 3 APB1 low, 4 APB1 high, 5 APB2), bit number `bit` (ENR and RSTR use the same bit).
#[derive(Clone, Copy, Debug, Serialize)]
pub struct BusEnable {
    pub reg: u8,
    pub bit: u8,
}

#[derive(Clone, Debug, Serialize)]
pub struct GpioInstance {
    pub name: String,
    /// Port number (0 = A). GPIO pin index = `port * 16 + bit`.
    pub port: u8,
    pub base: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UartKind {
    Usart,
    Uart,
    Lpuart,
}

#[derive(Clone, Debug, Serialize)]
pub struct UartInstance {
    pub name: String,
    pub kind: UartKind,
    pub base: u32,
    /// External interrupt number (IRQn).
    pub irq: u16,
    /// APB bus the register interface and (default) kernel clock come from: 1 or 2.
    pub apb: u8,
    pub enable: BusEnable,
}

#[derive(Clone, Debug, Serialize)]
pub struct TimerInstance {
    pub name: String,
    pub base: u32,
    pub irq: u16,
    /// Counter width in bits (16, or 32 for TIM2/TIM5).
    pub width: u8,
    /// Capture/compare channels (0 for the basic timers TIM6/TIM7).
    pub channels: u8,
    pub apb: u8,
    pub enable: BusEnable,
}

/// Peripheral wiring recipe: which instances the machine factory creates, and where.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArmPeripheralSet {
    pub rcc_base: u32,
    pub flash_base: u32,
    pub pwr_base: u32,
    pub syscfg_base: u32,
    pub exti_base: u32,
    pub gpio: Vec<GpioInstance>,
    pub uarts: Vec<UartInstance>,
    pub timers: Vec<TimerInstance>,
    /// IRQn of EXTI lines 0-15.
    pub exti_irqs: Vec<u16>,
    /// Package pin -> alternate function table: `(gpio index, AF number, signal)` for the
    /// signals the simulator routes (`USARTn_TX/RX`, `UARTn_*`, `LPUART1_*`, `TIMn_CHk`).
    #[serde(skip)]
    pub alt_functions: Vec<(u8, u8, &'static str)>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArmDeviceSpec {
    pub id: String,
    pub name: String,
    pub family: String,
    pub core_name: String,
    /// [`ArmFeatures`](crate::arm::thumb::ArmFeatures) bits.
    pub features: u32,
    /// CPUID register value of the core (revision / part number).
    pub cpuid: u32,
    pub flash_base: u32,
    pub flash_size: u32,
    pub sram_base: u32,
    /// Main SRAM (SRAM1 + SRAM2) in bytes; the CCM SRAM, when present, follows it in the
    /// address space and in the data the session sends.
    pub sram_size: u32,
    pub ccm_sram: Option<CcmSpec>,
    pub registers: Vec<MmioRegisterSpec>,
    pub groups: Vec<PeripheralGroupSpec>,
    pub vectors: Vec<ArmVectorSpec>,
    /// Number of external interrupt lines.
    pub nirq: u32,
    pub nvic_prio_bits: u8,
    pub package: String,
    pub pins: Vec<PinSpec>,
    /// Size of the GPIO pin array (`ports * 16`); pin index = `port * 16 + bit`.
    pub gpio_count: u8,
    pub clock: ArmClockSpec,
    pub vcc: f64,
    pub vcc_range: (f64, f64),
    pub speed_grades: Vec<(f64, f64)>,
    pub datasheet: String,
    pub die: Option<DieSpec>,
    pub peripheral_set: ArmPeripheralSet,
}

impl ArmDeviceSpec {
    /// Total RAM visible to the debugger (main SRAM + CCM SRAM), in bytes.
    pub fn ram_total(&self) -> u32 {
        self.sram_size + self.ccm_sram.map_or(0, |c| c.size)
    }

    pub fn register(&self, name: &str) -> Option<&MmioRegisterSpec> {
        self.registers.iter().find(|r| r.name.eq_ignore_ascii_case(name))
    }

    /// Address of a register (panics if missing: wiring bug).
    pub fn reg(&self, name: &str) -> u32 {
        self.register(name).unwrap_or_else(|| panic!("{}: register {name} not defined", self.name)).addr
    }

    /// GPIO names indexed by GPIO number ("PA0", "PB12"...).
    pub fn gpio_names(&self) -> Vec<String> {
        (0..self.gpio_count as usize).map(|i| format!("P{}{}", (b'A' + (i / 16) as u8) as char, i % 16)).collect()
    }
}
