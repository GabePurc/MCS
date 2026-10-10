//! STM32G4 (Cortex-M4F, up to 170 MHz) device descriptions: STM32G431KB and STM32G474RE.
//!
//! Sources:
//! * RM0440 Rev 8 (STM32G4 series reference manual): memory map, register reset values, clock tree.
//! * DS12589 (STM32G431xB) and DS12288 (STM32G474xB/C/E): memory sizes, packages, electrical limits.
//! * `stm32g431xx.h` / `stm32g474xx.h` of ST's cmsis-device-g4: register offsets, bit masks and
//!   names, IRQ numbers (via `gen_stm32g4.py` -> `stm32g4_gen.rs`).
//! * ST's STM32_open_pin_data (the CubeMX database): package pin numbers and alternate functions.
//! * ARM DDI 0439B (Cortex-M4 TRM) / DUI 0553: SysTick, SCB, NVIC registers.

use crate::arm::device::*;
use crate::arm::thumb::ArmFeatures;
use crate::avr::device::PeripheralGroupSpec;

use super::common::{self, core_registers, expand};
use super::stm32g4_gen as g;

struct Variant {
    id: &'static str,
    name: &'static str,
    flash_size: u32,
    /// SRAM1 + SRAM2.
    sram_size: u32,
    ccm_size: u32,
    package: &'static str,
    datasheet: &'static str,
    pins: &'static [g::PinDef],
    af: &'static [(u8, u8, &'static str)],
    vectors: &'static [(u16, &'static str, &'static str)],
    uart5: bool,
}

const G431KB: Variant = Variant {
    id: "stm32g431kb",
    name: "STM32G431KB",
    flash_size: 128 * 1024,
    sram_size: 22 * 1024,
    ccm_size: 10 * 1024,
    package: "LQFP32",
    datasheet: "DS12589 (STM32G431xB) with RM0440 Rev 8",
    pins: g::G431KB_PINS,
    af: g::G431KB_AF,
    vectors: g::G431KB_VECTORS,
    uart5: false,
};

const G474RE: Variant = Variant {
    id: "stm32g474re",
    name: "STM32G474RE",
    flash_size: 512 * 1024,
    sram_size: 96 * 1024,
    ccm_size: 32 * 1024,
    package: "LQFP64",
    datasheet: "DS12288 (STM32G474xB/C/E) with RM0440 Rev 8",
    pins: g::G474RE_PINS,
    af: g::G474RE_AF,
    vectors: g::G474RE_VECTORS,
    uart5: true,
};

pub fn devices() -> Vec<ArmDeviceSpec> {
    vec![build(&G431KB), build(&G474RE)]
}

fn base(name: &str) -> u32 {
    common::base(g::BASES, name)
}

fn build(v: &Variant) -> ArmDeviceSpec {
    let sram_base = 0x2000_0000;
    let ccm = CcmSpec { base: 0x1000_0000, size: v.ccm_size, alias_base: sram_base + v.sram_size };
    let nirq = v.vectors.iter().map(|x| x.0 as u32 + 1).max().unwrap_or(0).max(102);

    // ---- peripheral instances
    let gpio: Vec<GpioInstance> = (0..7u8).map(|p| {
        let name = format!("GPIO{}", (b'A' + p) as char);
        GpioInstance { base: base(&name), name, port: p, enable: BusEnable { reg: 1, bit: p } }
    }).collect();
    let irq = |n: &str| v.vectors.iter().find(|x| x.1 == n).unwrap_or_else(|| panic!("irq {n}")).0;
    let mut uarts = vec![
        UartInstance { name: "USART1".into(), kind: UartKind::Usart, base: base("USART1"), irq: irq("USART1"), apb: 2, enable: BusEnable { reg: 5, bit: 14 } },
        UartInstance { name: "USART2".into(), kind: UartKind::Usart, base: base("USART2"), irq: irq("USART2"), apb: 1, enable: BusEnable { reg: 3, bit: 17 } },
        UartInstance { name: "USART3".into(), kind: UartKind::Usart, base: base("USART3"), irq: irq("USART3"), apb: 1, enable: BusEnable { reg: 3, bit: 18 } },
        UartInstance { name: "UART4".into(), kind: UartKind::Uart, base: base("UART4"), irq: irq("UART4"), apb: 1, enable: BusEnable { reg: 3, bit: 19 } },
    ];
    if v.uart5 {
        uarts.push(UartInstance { name: "UART5".into(), kind: UartKind::Uart, base: base("UART5"), irq: irq("UART5"), apb: 1, enable: BusEnable { reg: 3, bit: 20 } });
    }
    uarts.push(UartInstance { name: "LPUART1".into(), kind: UartKind::Lpuart, base: base("LPUART1"), irq: irq("LPUART1"), apb: 1, enable: BusEnable { reg: 4, bit: 0 } });
    let tim_irq = |n: &str| v.vectors.iter().find(|x| x.1 == n || x.1.starts_with(&format!("{n}_"))).unwrap_or_else(|| panic!("irq {n}")).0;
    let timers = vec![
        TimerInstance { name: "TIM2".into(), base: base("TIM2"), irq: tim_irq("TIM2"), width: 32, channels: 4, apb: 1, enable: BusEnable { reg: 3, bit: 0 } },
        TimerInstance { name: "TIM3".into(), base: base("TIM3"), irq: tim_irq("TIM3"), width: 16, channels: 4, apb: 1, enable: BusEnable { reg: 3, bit: 1 } },
        TimerInstance { name: "TIM4".into(), base: base("TIM4"), irq: tim_irq("TIM4"), width: 16, channels: 4, apb: 1, enable: BusEnable { reg: 3, bit: 2 } },
        TimerInstance { name: "TIM6".into(), base: base("TIM6"), irq: tim_irq("TIM6"), width: 16, channels: 0, apb: 1, enable: BusEnable { reg: 3, bit: 4 } },
        TimerInstance { name: "TIM7".into(), base: base("TIM7"), irq: tim_irq("TIM7"), width: 16, channels: 0, apb: 1, enable: BusEnable { reg: 3, bit: 5 } },
    ];
    let exti_irqs: Vec<u16> = (0..16u16).map(|l| match l {
        0..=4 => irq(&format!("EXTI{l}")),
        5..=9 => irq("EXTI9_5"),
        _ => irq("EXTI15_10"),
    }).collect();

    // ---- registers
    let mut regs = Vec::new();
    let plain = |_: &str, r: u32| r;
    expand(&mut regs, "RCC", base("RCC"), g::RCC, plain);
    expand(&mut regs, "FLASH", base("FLASH"), g::FLASH, plain);
    expand(&mut regs, "PWR", base("PWR"), g::PWR, plain);
    expand(&mut regs, "SYSCFG", base("SYSCFG"), g::SYSCFG, plain);
    expand(&mut regs, "EXTI", base("EXTI"), g::EXTI, plain);
    for p in &gpio {
        let port = p.port;
        expand(&mut regs, &p.name, p.base, g::GPIO, |n, r| match (n, port) {
            ("MODER", 0) => 0xABFF_FFFF,
            ("MODER", 1) => 0xFFFF_FEBF,
            ("MODER", _) => 0xFFFF_FFFF,
            ("PUPDR", 0) => 0x6400_0000,
            ("PUPDR", 1) => 0x0000_0100,
            ("OSPEEDR", 0) => 0x0C00_0000,
            ("OSPEEDR", 1) => 0x0000_00C0,
            _ => r,
        });
    }
    for u in &uarts {
        expand(&mut regs, &u.name, u.base, g::USART, plain);
    }
    for t in &timers {
        let defs = if t.channels > 0 { g::TIM_GP } else { g::TIM_BASIC };
        let wide = t.width == 32;
        expand(&mut regs, &t.name, t.base, defs, |n, r| if n == "ARR" && wide { 0xFFFF_FFFF } else { r });
    }
    core_registers(&mut regs, nirq, 0x410F_C241, false);
    regs.sort_by_key(|r| r.addr);

    // ---- groups
    let mut groups = vec![
        ("RCC", "Reset and clock control"),
        ("FLASH", "Embedded flash memory interface"),
        ("PWR", "Power control"),
        ("SYSCFG", "System configuration controller (EXTI line routing)"),
        ("EXTI", "Extended interrupts and events controller"),
    ]
    .into_iter()
    .map(|(n, d)| PeripheralGroupSpec { name: n.into(), desc: d.into() })
    .collect::<Vec<_>>();
    groups.extend(gpio.iter().map(|p| PeripheralGroupSpec { name: p.name.clone(), desc: format!("General-purpose I/O port {}", (b'A' + p.port) as char) }));
    groups.extend(uarts.iter().map(|u| PeripheralGroupSpec { name: u.name.clone(), desc: match u.kind { UartKind::Usart => "Universal synchronous/asynchronous receiver transmitter", UartKind::Uart => "Universal asynchronous receiver transmitter", UartKind::Lpuart => "Low-power UART" }.into() }));
    groups.extend(timers.iter().map(|t| PeripheralGroupSpec { name: t.name.clone(), desc: if t.channels > 0 { format!("{}-bit general-purpose timer", t.width) } else { "Basic timer".into() } }));
    groups.extend([("SysTick", "System timer (Cortex-M4)"), ("SCB", "System control block"), ("NVIC", "Nested vectored interrupt controller")].into_iter().map(|(n, d)| PeripheralGroupSpec { name: n.into(), desc: d.into() }));

    // ---- vectors
    let vectors = common::vectors(v.vectors);

    // ---- pins
    let pins = common::pins(v.pins);

    ArmDeviceSpec {
        id: v.id.into(),
        name: v.name.into(),
        family: "STM32G4".into(),
        core_name: "Cortex-M4F".into(),
        features: ArmFeatures::CORTEX_M4F.0,
        cpuid: 0x410F_C241,
        flash_base: 0x0800_0000,
        flash_size: v.flash_size,
        flash_alias: true,
        sram_base,
        sram_size: v.sram_size,
        ccm_sram: Some(ccm),
        extra_ram: Vec::new(),
        ram_aliases: Vec::new(),
        registers: regs,
        groups,
        vectors,
        nirq,
        nvic_prio_bits: 4,
        package: v.package.into(),
        pins,
        gpio_count: 7 * 16,
        clock: ArmClockSpec { hsi_hz: 16e6, lsi_hz: 32e3, hse_min_hz: 4e6, hse_max_hz: 48e6, hse_default_hz: 8e6, csi_hz: 0.0 },
        vcc: 3.3,
        vcc_range: (1.71, 3.6),
        speed_grades: vec![(170e6, 1.71)],
        datasheet: v.datasheet.into(),
        die: None,
        peripheral_set: ArmPeripheralSet {
            family: PeriphFamily::Stm32G4,
            rcc_base: base("RCC"),
            flash_base: base("FLASH"),
            pwr_base: base("PWR"),
            syscfg_base: base("SYSCFG"),
            exti_base: base("EXTI"),
            gpio,
            uarts,
            timers,
            exti_irqs,
            alt_functions: v.af.to_vec(),
        },
    }
}
