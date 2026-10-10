//! STM32 memory-mapped peripherals ([`Mmio`](super::bus::Mmio) implementations) and the machine
//! factory that wires them from an [`ArmDeviceSpec`](mcs_core::arm::device::ArmDeviceSpec).

/// Reads the `size`-byte lane at `offset` of a 32-bit register.
#[inline]
pub(crate) fn lane_read(reg: u32, offset: u32, size: u8) -> u32 {
    let sh = (offset & 3) * 8;
    match size {
        4 => reg,
        2 => (reg >> sh) & 0xffff,
        _ => (reg >> sh) & 0xff,
    }
}

/// Merges a `size`-byte write at `offset` into the 32-bit register value `old`.
#[inline]
pub(crate) fn lane_write(old: u32, offset: u32, size: u8, value: u32) -> u32 {
    if size == 4 {
        return value;
    }
    let sh = (offset & 3) * 8;
    let m = if size == 2 { 0xffffu32 } else { 0xff } << sh;
    (old & !m) | ((value << sh) & m)
}

pub mod gpio;
pub mod serial;
pub mod stimulus;
pub mod exti;
pub mod rcc;
pub mod tim;
pub mod uart;

use mcs_core::arm::device::ArmDeviceSpec;
use mcs_core::arm::thumb::ArmFeatures;

use super::bus::{MemConfig, RamAlias};
use super::machine::{ArmConfig, Machine};
use super::sys::{signal_id, ArmSys};

/// SysTick reference clock: HCLK / 8 on STM32.
const SYSTICK_EXT_DIV: u32 = 8;

impl Machine {
    /// Builds the complete microcontroller described by `spec`: memories (with the CCM SRAM
    /// alias), core peripherals and the STM32 peripherals of its [`ArmPeripheralSet`]
    /// (RCC, FLASH, PWR, SYSCFG/EXTI, GPIO ports, USART/UART/LPUART, timers), the pin array and the
    /// alternate-function routing. Power-on reset has been applied.
    pub fn from_spec(spec: &'static ArmDeviceSpec) -> Machine {
        let ps = &spec.peripheral_set;
        let ccm = spec.ccm_sram;
        let mem = MemConfig {
            flash_base: spec.flash_base,
            flash_size: spec.flash_size,
            flash_alias: true,
            ram: vec![(spec.sram_base, spec.ram_total())],
            ram_alias: ccm.map(|c| RamAlias { base: c.base, size: c.size, ram: 0, off: c.alias_base - spec.sram_base }),
        };
        let cfg = ArmConfig {
            mem,
            features: ArmFeatures(spec.features),
            nirq: spec.nirq,
            prio_bits: spec.nvic_prio_bits,
            cpuid: spec.cpuid,
            systick_ext_div: SYSTICK_EXT_DIV,
            systick_calib: 0,
            unmapped_peripherals_raz: true,
        };
        let mut sys = ArmSys::new(spec.gpio_count as usize, spec.clock.hsi_hz, spec.vcc, spec.clock.hse_default_hz);
        for &(gpio, af, name) in &ps.alt_functions {
            sys.af[gpio as usize][af as usize & 15] = signal_id(name);
        }
        let mut m = Machine::with_sys(cfg, sys, spec.clock.hsi_hz);
        m.spec = Some(spec);

        let add = |m: &mut Machine, name: &str, base: u32, size: u32, dev: Box<dyn super::bus::Mmio>, enable: Option<(u8, u8)>, listen: bool| {
            let idx = m.add_peripheral(base, size, dev);
            m.dev_names[idx as usize] = name.to_string();
            m.dev_enable[idx as usize] = enable;
            if listen {
                m.sys.listeners.push(idx);
            }
        };
        add(&mut m, "RCC", ps.rcc_base, 0x400, Box::new(rcc::Rcc::new(spec.clock.hsi_hz)), None, false);
        add(&mut m, "FLASH", ps.flash_base, 0x400, Box::<rcc::FlashIf>::default(), Some((0, 8)), false);
        add(&mut m, "PWR", ps.pwr_base, 0x400, Box::<rcc::Pwr>::default(), Some((3, 28)), false);
        assert_eq!(ps.exti_base, ps.syscfg_base + 0x400, "SYSCFG and EXTI share one device");
        add(&mut m, "EXTI", ps.syscfg_base, 0x800, Box::new(exti::SysExti::new(&ps.exti_irqs)), Some((5, 0)), true);
        for g in &ps.gpio {
            add(&mut m, &g.name, g.base, 0x400, Box::new(gpio::Gpio::new(g.port)), Some((1, g.port)), false);
        }
        for u in &ps.uarts {
            add(&mut m, &u.name, u.base, 0x400, Box::new(uart::Uart::new(u)), Some((u.enable.reg, u.enable.bit)), true);
        }
        for t in &ps.timers {
            add(&mut m, &t.name, t.base, 0x400, Box::new(tim::Timer::new(t)), Some((t.enable.reg, t.enable.bit)), false);
        }
        m.reset();
        m
    }
}
