//! STM32H743 (Cortex-M7 r1p1, double-precision FPU, up to 480 MHz) device descriptions:
//! STM32H743IIT6 (LQFP176) and STM32H743ZIT6 (LQFP144, Nucleo-H743ZI).
//!
//! Sources:
//! * RM0433 Rev 8 (STM32H742/743/750/753 reference manual): memory map, RCC / PWR / FLASH / SYSCFG /
//!   EXTI behaviour, clock tree and bus domains (D1 AXI, D2 AHB/APB1/2, D3/D4 AHB4/APB4).
//! * DS12110 (STM32H743xI datasheet): memory sizes, packages, electrical limits (VDD 1.62-3.6 V,
//!   480 MHz on revision V at VOS0, 400 MHz on revision Y), HSE 4-48 MHz.
//! * `stm32h743xx.h` of ST's cmsis_device_h7: register offsets, bit masks and names, IRQ numbers and
//!   RCC clock-enable positions (via `gen_stm32h7.py` -> `stm32h7_gen.rs`).
//! * ST's SVD (modm-io/cmsis-svd-stm32 `STM32H743.svd`): register reset values.
//! * ST's STM32_open_pin_data (the CubeMX database): package pin numbers and alternate functions.
//! * ARM DDI 0489 / PM0253 (Cortex-M7 TRM, STM32F7/H7 programming manual): SysTick, SCB, NVIC, cache,
//!   MPU and TCM control registers.
//!
//! The core boots from `flash_base` (BOOT_ADD0 = 0x0800_0000), there is no flash alias at 0 and the
//! 64 KiB ITCM occupies address 0.

use crate::arm::device::*;
use crate::arm::thumb::ArmFeatures;
use crate::avr::device::PeripheralGroupSpec;

use super::common::{self, core_registers, expand};
use super::stm32h7_gen as g;

struct Variant {
    id: &'static str,
    name: &'static str,
    package: &'static str,
    pins: &'static [g::PinDef],
    af: &'static [(u8, u8, &'static str)],
}

const H743IIT6: Variant = Variant { id: "stm32h743iit6", name: "STM32H743IIT6", package: "LQFP176", pins: g::H743II_PINS, af: g::H743II_AF };
const H743ZIT6: Variant = Variant { id: "stm32h743zit6", name: "STM32H743ZIT6", package: "LQFP144", pins: g::H743ZI_PINS, af: g::H743ZI_AF };

pub fn devices() -> Vec<ArmDeviceSpec> {
    vec![build(&H743IIT6), build(&H743ZIT6)]
}

fn base(name: &str) -> u32 {
    common::base(g::BASES, name)
}

/// Clock-enable position of peripheral `name` (from the CMSIS header, see `gen_stm32h7.py`).
fn enable(name: &str) -> BusEnable {
    let e = g::ENABLE.iter().find(|e| e.0 == name).unwrap_or_else(|| panic!("enable {name}"));
    BusEnable { reg: e.1, bit: e.2 }
}

const CPUID_M7_R1P1: u32 = 0x411F_C271;
/// Ports A-K.
const PORTS: u8 = 11;

fn build(v: &Variant) -> ArmDeviceSpec {
    let nirq = g::VECTORS.iter().map(|x| x.0 as u32 + 1).max().unwrap_or(0);
    let irq = |n: &str| g::VECTORS.iter().find(|x| x.1 == n).unwrap_or_else(|| panic!("irq {n}")).0;

    // ---- memories
    let extra_ram = vec![
        MemRegionSpec { name: "ITCM".into(), base: 0x0000_0000, size: 64 * 1024 },
        MemRegionSpec { name: "AXI SRAM".into(), base: 0x2400_0000, size: 512 * 1024 },
        MemRegionSpec { name: "SRAM1-3".into(), base: 0x3000_0000, size: 288 * 1024 },
        MemRegionSpec { name: "SRAM4".into(), base: 0x3800_0000, size: 64 * 1024 },
        MemRegionSpec { name: "Backup SRAM".into(), base: 0x3880_0000, size: 4 * 1024 },
    ];
    // SRAM1-3 (extra block 3) are also reachable at 0x1000_0000 (D2 SRAM on the C-bus side).
    let ram_aliases = vec![MemAliasSpec { base: 0x1000_0000, size: 288 * 1024, region: 3, offset: 0 }];

    // ---- peripheral instances
    let gpio: Vec<GpioInstance> = (0..PORTS)
        .map(|p| {
            let name = format!("GPIO{}", (b'A' + p) as char);
            GpioInstance { base: base(&name), enable: enable(&name), name, port: p }
        })
        .collect();
    let uart = |name: &str, kind: UartKind, apb: u8| UartInstance { name: name.into(), kind, base: base(name), irq: irq(name), apb, enable: enable(name) };
    let uarts = vec![
        uart("USART1", UartKind::Usart, 2),
        uart("USART2", UartKind::Usart, 1),
        uart("USART3", UartKind::Usart, 1),
        uart("UART4", UartKind::Uart, 1),
        uart("UART5", UartKind::Uart, 1),
        uart("USART6", UartKind::Usart, 2),
        uart("UART7", UartKind::Uart, 1),
        uart("UART8", UartKind::Uart, 1),
        uart("LPUART1", UartKind::Lpuart, 4),
    ];
    let tim_irq = |n: &str| g::VECTORS.iter().find(|x| x.1 == n || x.1.starts_with(&format!("{n}_"))).unwrap_or_else(|| panic!("irq {n}")).0;
    let timer = |name: &str, width: u8, channels: u8| TimerInstance { name: name.into(), base: base(name), irq: tim_irq(name), width, channels, apb: 1, enable: enable(name) };
    let timers = vec![timer("TIM2", 32, 4), timer("TIM3", 16, 4), timer("TIM4", 16, 4), timer("TIM5", 32, 4), timer("TIM6", 16, 0), timer("TIM7", 16, 0)];
    let exti_irqs: Vec<u16> = (0..16u16)
        .map(|l| match l {
            0..=4 => irq(&format!("EXTI{l}")),
            5..=9 => irq("EXTI9_5"),
            _ => irq("EXTI15_10"),
        })
        .collect();

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
    core_registers(&mut regs, nirq, CPUID_M7_R1P1, true);
    regs.sort_by_key(|r| r.addr);

    // ---- groups
    let mut groups = vec![
        ("RCC", "Reset and clock control"),
        ("FLASH", "Embedded flash memory interface"),
        ("PWR", "Power control"),
        ("SYSCFG", "System configuration controller (EXTI line routing, overdrive)"),
        ("EXTI", "Extended interrupts and events controller"),
    ]
    .into_iter()
    .map(|(n, d)| PeripheralGroupSpec { name: n.into(), desc: d.into() })
    .collect::<Vec<_>>();
    groups.extend(gpio.iter().map(|p| PeripheralGroupSpec { name: p.name.clone(), desc: format!("General-purpose I/O port {}", (b'A' + p.port) as char) }));
    groups.extend(uarts.iter().map(|u| PeripheralGroupSpec { name: u.name.clone(), desc: match u.kind { UartKind::Usart => "Universal synchronous/asynchronous receiver transmitter", UartKind::Uart => "Universal asynchronous receiver transmitter", UartKind::Lpuart => "Low-power UART" }.into() }));
    groups.extend(timers.iter().map(|t| PeripheralGroupSpec { name: t.name.clone(), desc: if t.channels > 0 { format!("{}-bit general-purpose timer", t.width) } else { "Basic timer".into() } }));
    groups.extend([("SysTick", "System timer (Cortex-M7)"), ("SCB", "System control block (incl. cache maintenance, TCM control)"), ("MPU", "Memory protection unit (registers stored, not enforced)"), ("NVIC", "Nested vectored interrupt controller")].into_iter().map(|(n, d)| PeripheralGroupSpec { name: n.into(), desc: d.into() }));

    ArmDeviceSpec {
        id: v.id.into(),
        name: v.name.into(),
        family: "STM32H7".into(),
        core_name: "Cortex-M7".into(),
        features: ArmFeatures::CORTEX_M7.0,
        cpuid: CPUID_M7_R1P1,
        flash_base: 0x0800_0000,
        // Two 1 MiB banks at 0x0800_0000 and 0x0810_0000, contiguous.
        flash_size: 2 * 1024 * 1024,
        flash_alias: false,
        sram_base: 0x2000_0000,
        sram_size: 128 * 1024,
        ccm_sram: None,
        extra_ram,
        ram_aliases,
        registers: regs,
        groups,
        vectors: common::vectors(g::VECTORS),
        nirq,
        nvic_prio_bits: 4,
        package: v.package.into(),
        pins: common::pins(v.pins),
        gpio_count: PORTS * 16,
        clock: ArmClockSpec { hsi_hz: 64e6, lsi_hz: 32e3, hse_min_hz: 4e6, hse_max_hz: 48e6, hse_default_hz: 8e6, csi_hz: 4e6 },
        vcc: 3.3,
        vcc_range: (1.62, 3.6),
        // 480 MHz on revision V at VOS0 (400 MHz on revision Y); the simulator models revision V.
        speed_grades: vec![(480e6, 1.62)],
        datasheet: "DS12110 (STM32H743xI) with RM0433 Rev 8".into(),
        die: None,
        peripheral_set: ArmPeripheralSet {
            family: PeriphFamily::Stm32H7,
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
