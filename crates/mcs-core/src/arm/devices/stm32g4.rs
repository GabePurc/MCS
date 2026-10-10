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
use crate::avr::device::{PeripheralGroupSpec, PinKind, PinSpec, RegisterAccess};

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
    g::BASES.iter().find(|b| b.0 == name).unwrap_or_else(|| panic!("base {name}")).1
}

fn access(s: &str) -> RegisterAccess {
    match s {
        "r" => RegisterAccess::R,
        "w" => RegisterAccess::W,
        _ => RegisterAccess::Rw,
    }
}

fn expand(out: &mut Vec<MmioRegisterSpec>, inst: &str, base: u32, defs: &[g::RegDef], reset: impl Fn(&str, u32) -> u32) {
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

/// `id` is `<group>_<register>`, e.g. `SCB_VTOR`.
fn core_reg(out: &mut Vec<MmioRegisterSpec>, id: &str, addr: u32, reset: u32, acc: RegisterAccess, desc: &str, bits: &[(&str, u32, &str)]) {
    let group = id.split_once('_').map_or(id, |g| g.0);
    out.push(MmioRegisterSpec {
        name: id.to_string(),
        addr,
        size: 4,
        reset,
        group: group.to_string(),
        desc: desc.to_string(),
        bits: bits.iter().map(|&(n, m, d)| MmioBitSpec { name: n.to_string(), mask: m, desc: d.to_string() }).collect(),
        access: acc,
    });
}

/// SysTick, SCB and NVIC registers (ARM DUI 0553 chapter 4).
fn core_registers(out: &mut Vec<MmioRegisterSpec>, nirq: u32, cpuid: u32) {
    use RegisterAccess::*;
    core_reg(out, "SysTick_CSR", 0xE000_E010, 4, Rw, "SysTick control and status register", &[
        ("ENABLE", 1, "Counter enable"),
        ("TICKINT", 2, "SysTick exception request enable"),
        ("CLKSOURCE", 4, "Clock source: 0 = HCLK/8, 1 = HCLK"),
        ("COUNTFLAG", 1 << 16, "Counter reached 0 since the last read"),
    ]);
    core_reg(out, "SysTick_RVR", 0xE000_E014, 0, Rw, "SysTick reload value register", &[("RELOAD", 0x00ff_ffff, "Value loaded when the counter reaches 0")]);
    core_reg(out, "SysTick_CVR", 0xE000_E018, 0, Rw, "SysTick current value register", &[("CURRENT", 0x00ff_ffff, "Current counter value")]);
    core_reg(out, "SysTick_CALIB", 0xE000_E01C, 0, R, "SysTick calibration value register", &[]);
    core_reg(out, "SCB_CPUID", 0xE000_ED00, cpuid, R, "CPUID base register", &[
        ("REVISION", 0xf, "Patch release"),
        ("PARTNO", 0xfff0, "Part number (0xC24 = Cortex-M4)"),
        ("ARCHITECTURE", 0xf_0000, "Architecture (0xF = ARMv7-M)"),
        ("VARIANT", 0xf0_0000, "Major revision"),
        ("IMPLEMENTER", 0xff00_0000, "Implementer (0x41 = ARM)"),
    ]);
    core_reg(out, "SCB_ICSR", 0xE000_ED04, 0, Rw, "Interrupt control and state register", &[
        ("VECTACTIVE", 0x1ff, "Active exception number"),
        ("RETTOBASE", 1 << 11, "No other exception is active"),
        ("VECTPENDING", 0x1ff000, "Highest priority pending exception"),
        ("ISRPENDING", 1 << 22, "External interrupt pending"),
        ("PENDSTCLR", 1 << 25, "Clear SysTick pending"),
        ("PENDSTSET", 1 << 26, "Set SysTick pending"),
        ("PENDSVCLR", 1 << 27, "Clear PendSV pending"),
        ("PENDSVSET", 1 << 28, "Set PendSV pending"),
        ("NMIPENDSET", 1 << 31, "Set NMI pending"),
    ]);
    core_reg(out, "SCB_VTOR", 0xE000_ED08, 0, Rw, "Vector table offset register", &[("TBLOFF", 0xffff_ff80, "Vector table base address")]);
    core_reg(out, "SCB_AIRCR", 0xE000_ED0C, 0xfa05_0000, Rw, "Application interrupt and reset control register", &[
        ("VECTRESET", 1, "Reserved for debug"),
        ("VECTCLRACTIVE", 2, "Clear active vector"),
        ("SYSRESETREQ", 4, "System reset request"),
        ("PRIGROUP", 0x700, "Priority grouping"),
        ("ENDIANNESS", 1 << 15, "Data endianness (0 = little)"),
        ("VECTKEY", 0xffff_0000, "Write key 0x05FA"),
    ]);
    core_reg(out, "SCB_SCR", 0xE000_ED10, 0, Rw, "System control register", &[
        ("SLEEPONEXIT", 2, "Sleep on return to thread mode"),
        ("SLEEPDEEP", 4, "Deep sleep"),
        ("SEVONPEND", 0x10, "Wake on any pending interrupt"),
    ]);
    core_reg(out, "SCB_CCR", 0xE000_ED14, 0x200, Rw, "Configuration and control register", &[
        ("NONBASETHRDENA", 1, "Allow return to thread mode from any level"),
        ("USERSETMPEND", 2, "Unprivileged STIR access"),
        ("UNALIGN_TRP", 8, "Trap unaligned accesses"),
        ("DIV_0_TRP", 0x10, "Trap division by zero"),
        ("BFHFNMIGN", 0x100, "Ignore bus faults at priority -1/-2"),
        ("STKALIGN", 0x200, "8-byte stack alignment on exception entry"),
    ]);
    core_reg(out, "SCB_SHPR1", 0xE000_ED18, 0, Rw, "System handler priority register 1 (MemManage, BusFault, UsageFault)", &[]);
    core_reg(out, "SCB_SHPR2", 0xE000_ED1C, 0, Rw, "System handler priority register 2 (SVCall)", &[]);
    core_reg(out, "SCB_SHPR3", 0xE000_ED20, 0, Rw, "System handler priority register 3 (PendSV, SysTick)", &[]);
    core_reg(out, "SCB_SHCSR", 0xE000_ED24, 0, Rw, "System handler control and state register", &[
        ("MEMFAULTENA", 1 << 16, "MemManage fault enable"),
        ("BUSFAULTENA", 1 << 17, "BusFault enable"),
        ("USGFAULTENA", 1 << 18, "UsageFault enable"),
    ]);
    core_reg(out, "SCB_CFSR", 0xE000_ED28, 0, Rw, "Configurable fault status register (MMFSR, BFSR, UFSR)", &[]);
    core_reg(out, "SCB_HFSR", 0xE000_ED2C, 0, Rw, "HardFault status register", &[("VECTTBL", 2, "Vector table read fault"), ("FORCED", 1 << 30, "Forced HardFault"), ("DEBUGEVT", 1 << 31, "Debug event")]);
    core_reg(out, "SCB_MMFAR", 0xE000_ED34, 0, Rw, "MemManage fault address register", &[]);
    core_reg(out, "SCB_BFAR", 0xE000_ED38, 0, Rw, "BusFault address register", &[]);
    core_reg(out, "SCB_CPACR", 0xE000_ED88, 0, Rw, "Coprocessor access control register (FPU CP10/CP11)", &[("CP10", 3 << 20, "CP10 access"), ("CP11", 3 << 22, "CP11 access")]);
    let words = nirq.div_ceil(32);
    for (grp, base, desc) in [("ISER", 0xE000_E100u32, "Interrupt set-enable"), ("ICER", 0xE000_E180, "Interrupt clear-enable"), ("ISPR", 0xE000_E200, "Interrupt set-pending"), ("ICPR", 0xE000_E280, "Interrupt clear-pending"), ("IABR", 0xE000_E300, "Interrupt active bit")] {
        for w in 0..words {
            core_reg(out, &format!("NVIC_{grp}{w}"), base + 4 * w, 0, if grp == "IABR" { R } else { Rw }, &format!("{desc} register {w} (IRQ {}-{})", w * 32, w * 32 + 31), &[]);
        }
    }
    for w in 0..nirq.div_ceil(4) {
        core_reg(out, &format!("NVIC_IPR{w}"), 0xE000_E400 + 4 * w, 0, Rw, &format!("Interrupt priority register {w} (IRQ {}-{})", w * 4, w * 4 + 3), &[]);
    }
}

fn build(v: &Variant) -> ArmDeviceSpec {
    let sram_base = 0x2000_0000;
    let ccm = CcmSpec { base: 0x1000_0000, size: v.ccm_size, alias_base: sram_base + v.sram_size };
    let nirq = v.vectors.iter().map(|x| x.0 as u32 + 1).max().unwrap_or(0).max(102);

    // ---- peripheral instances
    let gpio: Vec<GpioInstance> = (0..7u8).map(|p| {
        let name = format!("GPIO{}", (b'A' + p) as char);
        GpioInstance { base: base(&name), name, port: p }
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
    core_registers(&mut regs, nirq, 0x410F_C241);
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
    let mut vectors: Vec<ArmVectorSpec> = [(1, "Reset", "Reset"), (2, "NMI", "Non-maskable interrupt"), (3, "HardFault", "All classes of fault"), (4, "MemManage", "Memory management fault"), (5, "BusFault", "Pre-fetch / memory access fault"), (6, "UsageFault", "Undefined instruction or illegal state"), (11, "SVCall", "System service call via SVC"), (12, "DebugMon", "Debug monitor"), (14, "PendSV", "Pendable request for system service"), (15, "SysTick", "System tick timer")]
        .into_iter()
        .map(|(i, n, d)| ArmVectorSpec { index: i, name: n.into(), desc: d.into() })
        .collect();
    vectors.extend(v.vectors.iter().map(|&(i, n, d)| ArmVectorSpec { index: 16 + i, name: n.into(), desc: d.into() }));

    // ---- pins
    let pins: Vec<PinSpec> = v
        .pins
        .iter()
        .map(|p| PinSpec {
            number: p.number,
            name: p.name.to_string(),
            kind: match p.kind {
                1 => PinKind::Vcc,
                2 => PinKind::Gnd,
                3 => PinKind::Ref,
                _ => PinKind::Io,
            },
            gpio: (p.gpio >= 0).then_some(p.gpio as u8),
            functions: p.functions.iter().map(|s| s.to_string()).collect(),
        })
        .collect();

    ArmDeviceSpec {
        id: v.id.into(),
        name: v.name.into(),
        family: "STM32G4".into(),
        core_name: "Cortex-M4F".into(),
        features: ArmFeatures::CORTEX_M4F.0,
        cpuid: 0x410F_C241,
        flash_base: 0x0800_0000,
        flash_size: v.flash_size,
        sram_base,
        sram_size: v.sram_size,
        ccm_sram: Some(ccm),
        registers: regs,
        groups,
        vectors,
        nirq,
        nvic_prio_bits: 4,
        package: v.package.into(),
        pins,
        gpio_count: 7 * 16,
        clock: ArmClockSpec { hsi_hz: 16e6, lsi_hz: 32e3, hse_min_hz: 4e6, hse_max_hz: 48e6, hse_default_hz: 8e6 },
        vcc: 3.3,
        vcc_range: (1.71, 3.6),
        speed_grades: vec![(170e6, 1.71)],
        datasheet: v.datasheet.into(),
        die: None,
        peripheral_set: ArmPeripheralSet {
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
