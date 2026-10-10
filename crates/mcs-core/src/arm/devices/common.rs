//! Helpers shared by the STM32 family descriptions: expansion of the generated register templates,
//! the Cortex-M core registers (SysTick, SCB, NVIC and, on the Cortex-M7, the cache / MPU / TCM
//! control registers), package pin conversion and the core exception vectors.

use crate::arm::device::*;
use crate::avr::device::{PinKind, PinSpec, RegisterAccess};

use super::gen_types::{PinDef, RegDef};

/// Base address of peripheral `name` in a generated `BASES` table.
pub(super) fn base(bases: &[(&str, u32)], name: &str) -> u32 {
    bases.iter().find(|b| b.0 == name).unwrap_or_else(|| panic!("base {name}")).1
}

fn access(s: &str) -> RegisterAccess {
    match s {
        "r" => RegisterAccess::R,
        "w" => RegisterAccess::W,
        _ => RegisterAccess::Rw,
    }
}

/// Adds the registers of one peripheral instance from a generated template; `reset` may override
/// the reset value of a register by name.
pub(super) fn expand(out: &mut Vec<MmioRegisterSpec>, inst: &str, base: u32, defs: &[RegDef], reset: impl Fn(&str, u32) -> u32) {
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

/// SysTick, SCB and NVIC registers (ARM DUI 0553 chapter 4); with `m7` also the Cortex-M7 cache,
/// MPU and TCM control registers (ARM DDI 0489 / PM0253).
pub(super) fn core_registers(out: &mut Vec<MmioRegisterSpec>, nirq: u32, cpuid: u32, m7: bool) {
    use RegisterAccess::*;
    core_reg(out, "SCB_ACTLR", 0xE000_E008, 0, Rw, "Auxiliary control register", &[]);
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
        ("PARTNO", 0xfff0, "Part number (0xC24 = Cortex-M4, 0xC27 = Cortex-M7)"),
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
    let mut ccr_bits = vec![
        ("NONBASETHRDENA", 1, "Allow return to thread mode from any level"),
        ("USERSETMPEND", 2, "Unprivileged STIR access"),
        ("UNALIGN_TRP", 8, "Trap unaligned accesses"),
        ("DIV_0_TRP", 0x10, "Trap division by zero"),
        ("BFHFNMIGN", 0x100, "Ignore bus faults at priority -1/-2"),
        ("STKALIGN", 0x200, "8-byte stack alignment on exception entry"),
    ];
    if m7 {
        ccr_bits.extend([("DC", 1 << 16, "Data cache enable (stored, caches are not modelled)"), ("IC", 1 << 17, "Instruction cache enable (stored)"), ("BP", 1 << 18, "Branch prediction enable (stored)")]);
    }
    core_reg(out, "SCB_CCR", 0xE000_ED14, 0x200, Rw, "Configuration and control register", &ccr_bits);
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
    if m7 {
        core_reg(out, "SCB_CLIDR", 0xE000_ED78, 0x0900_0003, R, "Cache level ID register (separate I and D caches, one level)", &[]);
        core_reg(out, "SCB_CTR", 0xE000_ED7C, 0x8f03_0003, R, "Cache type register", &[]);
        core_reg(out, "SCB_CCSIDR", 0xE000_ED80, 0xe00f_e019, R, "Cache size ID register of the cache selected by CSSELR (16 KiB, 4 ways, 32-byte lines)", &[]);
        core_reg(out, "SCB_CSSELR", 0xE000_ED84, 0, Rw, "Cache size selection register", &[("IND", 1, "0 = data cache, 1 = instruction cache"), ("LEVEL", 0xe, "Cache level (0 = L1)")]);
        core_reg(out, "MPU_TYPE", 0xE000_ED90, 0x1000, R, "MPU type register (16 regions)", &[("DREGION", 0xff00, "Number of data regions")]);
        core_reg(out, "MPU_CTRL", 0xE000_ED94, 0, Rw, "MPU control register (stored, not enforced)", &[("ENABLE", 1, "MPU enable"), ("HFNMIENA", 2, "MPU enabled during HardFault / NMI"), ("PRIVDEFENA", 4, "Privileged default memory map")]);
        core_reg(out, "MPU_RNR", 0xE000_ED98, 0, Rw, "MPU region number register", &[("REGION", 0xff, "Region selected by RBAR / RASR")]);
        core_reg(out, "MPU_RBAR", 0xE000_ED9C, 0, Rw, "MPU region base address register", &[("REGION", 0xf, "Region number"), ("VALID", 0x10, "Write: update RNR with REGION"), ("ADDR", 0xffff_ffe0, "Region base address")]);
        core_reg(out, "MPU_RASR", 0xE000_EDA0, 0, Rw, "MPU region attribute and size register", &[("ENABLE", 1, "Region enable"), ("SIZE", 0x3e, "Region size is 2^(SIZE+1) bytes"), ("SRD", 0xff00, "Sub-region disable"), ("AP", 0x0700_0000, "Access permission"), ("XN", 1 << 28, "Execute never")]);
        for k in 1..=3u32 {
            core_reg(out, &format!("MPU_RBAR_A{k}"), 0xE000_ED9C + 8 * k, 0, Rw, &format!("MPU region base address register alias {k}"), &[]);
            core_reg(out, &format!("MPU_RASR_A{k}"), 0xE000_EDA0 + 8 * k, 0, Rw, &format!("MPU region attribute and size register alias {k}"), &[]);
        }
        for (n, a, d) in [
            ("ICIALLU", 0xEF50u32, "Instruction cache invalidate all to PoU"),
            ("ICIMVAU", 0xEF58, "Instruction cache invalidate by address to PoU"),
            ("DCIMVAC", 0xEF5C, "Data cache invalidate by address to PoC"),
            ("DCISW", 0xEF60, "Data cache invalidate by set/way"),
            ("DCCMVAU", 0xEF64, "Data cache clean by address to PoU"),
            ("DCCMVAC", 0xEF68, "Data cache clean by address to PoC"),
            ("DCCSW", 0xEF6C, "Data cache clean by set/way"),
            ("DCCIMVAC", 0xEF70, "Data cache clean and invalidate by address to PoC"),
            ("DCCISW", 0xEF74, "Data cache clean and invalidate by set/way"),
            ("BPIALL", 0xEF78, "Branch predictor invalidate all"),
        ] {
            core_reg(out, &format!("SCB_{n}"), 0xE000_0000 + a, 0, W, &format!("{d} (accepted, caches are not modelled)"), &[]);
        }
        core_reg(out, "SCB_ITCMCR", 0xE000_EF90, 0, Rw, "Instruction tightly-coupled memory control register (stored)", &[("EN", 1, "ITCM enable"), ("RMW", 2, "Read-modify-write"), ("RETEN", 4, "Retry phase enable")]);
        core_reg(out, "SCB_DTCMCR", 0xE000_EF94, 0, Rw, "Data tightly-coupled memory control register (stored)", &[("EN", 1, "DTCM enable"), ("RMW", 2, "Read-modify-write"), ("RETEN", 4, "Retry phase enable")]);
        core_reg(out, "SCB_AHBPCR", 0xE000_EF98, 0, Rw, "AHBP control register (stored)", &[]);
        core_reg(out, "SCB_CACR", 0xE000_EF9C, 0, Rw, "L1 cache control register (stored)", &[]);
        core_reg(out, "SCB_AHBSCR", 0xE000_EFA0, 0, Rw, "AHB slave control register (stored)", &[]);
        core_reg(out, "SCB_ABFSR", 0xE000_EFA8, 0, Rw, "Auxiliary bus fault status register", &[]);
    }
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

/// Exception vectors: the fixed core ones followed by the device's external interrupts
/// `(IRQn, name, description)`.
pub(super) fn vectors(irqs: &[(u16, &str, &str)]) -> Vec<ArmVectorSpec> {
    let mut v: Vec<ArmVectorSpec> = [(1, "Reset", "Reset"), (2, "NMI", "Non-maskable interrupt"), (3, "HardFault", "All classes of fault"), (4, "MemManage", "Memory management fault"), (5, "BusFault", "Pre-fetch / memory access fault"), (6, "UsageFault", "Undefined instruction or illegal state"), (11, "SVCall", "System service call via SVC"), (12, "DebugMon", "Debug monitor"), (14, "PendSV", "Pendable request for system service"), (15, "SysTick", "System tick timer")]
        .into_iter()
        .map(|(i, n, d)| ArmVectorSpec { index: i, name: n.into(), desc: d.into() })
        .collect();
    v.extend(irqs.iter().map(|&(i, n, d)| ArmVectorSpec { index: 16 + i, name: n.into(), desc: d.into() }));
    v
}

/// Package pins from a generated pin table.
pub(super) fn pins(defs: &[PinDef]) -> Vec<PinSpec> {
    defs.iter()
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
        .collect()
}
