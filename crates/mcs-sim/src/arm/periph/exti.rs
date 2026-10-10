//! SYSCFG (EXTI line routing) and EXTI as one device mapped over a 0x800 byte window that holds both
//! blocks. Two register layouts share the logic:
//!
//! * STM32G4 (RM0440 sections 10 and 16): SYSCFG at +0x000, EXTI at +0x400 (IMR1, EMR1, RTSR1, FTSR1,
//!   SWIER1, PR1), SYSCFGEN = APB2ENR bit 0.
//! * STM32H7 (RM0433 sections 14 and 21): EXTI at +0x000 (RTSR1, FTSR1, SWIER1, D3PMR1, ..., then the
//!   Cortex-M7 masks CPUIMR1 / CPUEMR1 / CPUPR1 at +0x80), SYSCFG at +0x400 (PMCR, EXTICR1-4, CFGR,
//!   CCCSR, CCVR, CCCR, PWRCR, PKGR), SYSCFGEN = APB4ENR bit 1. SYSCFG_PWRCR.ODEN is published to
//!   [`ArmSys`](crate::arm::sys::ArmSys) for the PWR model (VOS0 overdrive).
//!
//! EXTI: lines 0-15 follow the GPIO selected by SYSCFG_EXTICRx; rising / falling edge detection
//! sets the pending bit (PR1), the interrupt mask (IMR1) gates the NVIC line. EXTI0-4 have their
//! own interrupts, lines 5-9 and 10-15 share EXTI9_5 and EXTI15_10. SWIER sets pending bits in
//! software. Event mode (EMR1) is stored; events do not wake WFE. Lines 16+ (internal sources), the
//! second and third register banks and the H7 D3 pending-clear registers are stored but have no
//! effect. EXTI itself is always clocked; SYSCFG registers need the SYSCFGEN clock.

use crate::arm::bus::{Cx, Mmio};

use super::{lane_read, lane_write};

/// Register layout of the SYSCFG / EXTI window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtiLayout {
    G4,
    H7,
}

#[derive(Clone, Copy)]
enum Reg {
    Imr,
    Emr,
    Rtsr,
    Ftsr,
    Swier,
    Pr,
}

impl ExtiLayout {
    /// The EXTI block is in the upper half of the window (G4) or the lower half (H7).
    fn exti_high(self) -> bool {
        self == ExtiLayout::G4
    }

    /// RCC enable register index and bit of the SYSCFG clock.
    fn syscfg_clock(self) -> (u8, u8) {
        match self {
            ExtiLayout::G4 => (5, 0),
            ExtiLayout::H7 => (8, 1),
        }
    }

    fn exti_reg(self, rel: u32) -> Option<Reg> {
        match (self, rel) {
            (ExtiLayout::G4, 0x00) | (ExtiLayout::H7, 0x80) => Some(Reg::Imr),
            (ExtiLayout::G4, 0x04) | (ExtiLayout::H7, 0x84) => Some(Reg::Emr),
            (ExtiLayout::G4, 0x08) | (ExtiLayout::H7, 0x00) => Some(Reg::Rtsr),
            (ExtiLayout::G4, 0x0c) | (ExtiLayout::H7, 0x04) => Some(Reg::Ftsr),
            (ExtiLayout::G4, 0x10) | (ExtiLayout::H7, 0x08) => Some(Reg::Swier),
            (ExtiLayout::G4, 0x14) | (ExtiLayout::H7, 0x88) => Some(Reg::Pr),
            _ => None,
        }
    }

    /// Writable bits of the SYSCFG register at `rel` (0 = read-only / absent).
    fn syscfg_mask(self, rel: u32) -> u32 {
        match (self, rel) {
            (_, 0x08..=0x14) => if self == ExtiLayout::G4 { 0x7777 } else { 0xffff },
            (ExtiLayout::G4, 0x00) => 7,
            (ExtiLayout::G4, 0x04 | 0x18 | 0x1c | 0x20) => u32::MAX,
            (ExtiLayout::H7, 0x04 | 0x18 | 0x20 | 0x28) => u32::MAX,
            (ExtiLayout::H7, 0x2c) => 1, // PWRCR.ODEN
            _ => 0,
        }
    }

    fn syscfg_reset(self, words: &mut [u32]) {
        words.iter_mut().for_each(|w| *w = 0);
        if self == ExtiLayout::G4 {
            words[1] = 0x7c00_0001; // CFGR1
        }
    }

    fn imr_reset(self) -> u32 {
        match self {
            ExtiLayout::G4 => 0xff02_0000,
            ExtiLayout::H7 => 0xffc0_0000,
        }
    }
}

const WORDS: usize = 256;
const ODEN_OFF: u32 = 0x2c;

pub struct SysExti {
    layout: ExtiLayout,
    irqs: [u16; 16],
    /// SYSCFG registers by word offset (EXTICR1-4 at words 2-5).
    sys: [u32; WORDS],
    /// Stored EXTI registers without behaviour (H7 D3PMR / D3PCR, line 2 and 3 banks).
    xr: [u32; WORDS],
    imr: u32,
    emr: u32,
    rtsr: u32,
    ftsr: u32,
    swier: u32,
    pr: u32,
    /// Last level seen on the selected pin of every line (edge detection reference).
    last: [u8; 16],
}

impl SysExti {
    pub fn new(layout: ExtiLayout, irqs: &[u16]) -> Self {
        let mut a = [0u16; 16];
        for (d, s) in a.iter_mut().zip(irqs) {
            *d = *s;
        }
        let mut x = Self { layout, irqs: a, sys: [0; WORDS], xr: [0; WORDS], imr: 0, emr: 0, rtsr: 0, ftsr: 0, swier: 0, pr: 0, last: [0; 16] };
        x.set_reset_values();
        x
    }

    fn set_reset_values(&mut self) {
        self.layout.syscfg_reset(&mut self.sys);
        self.xr = [0; WORDS];
        self.imr = self.layout.imr_reset();
        self.emr = 0;
        self.rtsr = 0;
        self.ftsr = 0;
        self.swier = 0;
        self.pr = 0;
        self.last = [0; 16];
    }

    /// True when window offset `offset` belongs to the EXTI block.
    #[inline]
    fn is_exti(&self, offset: u32) -> bool {
        (offset >= 0x400) == self.layout.exti_high()
    }

    /// GPIO index selected for `line`.
    #[inline]
    fn selected(&self, line: usize) -> usize {
        let port = (self.sys[2 + (line >> 2)] >> (4 * (line & 3))) & 0xf;
        port as usize * 16 + line
    }

    fn baseline(&mut self, line: usize, cx: &Cx) {
        let p = self.selected(line);
        self.last[line] = cx.sys.pins.get(p).map_or(0, |p| p.level);
    }

    /// Recomputes the NVIC line levels from PR & IMR.
    fn update_irqs(&self, cx: &mut Cx) {
        let active = self.pr & self.imr;
        // Lines sharing an IRQ are evaluated together.
        let mut done = 0u16;
        for line in 0..16 {
            let irq = self.irqs[line];
            if done >> line & 1 != 0 {
                continue;
            }
            let mut level = false;
            for l in line..16 {
                if self.irqs[l] == irq {
                    done |= 1 << l;
                    level |= active >> l & 1 != 0;
                }
            }
            cx.set_irq_line(irq as u32, level);
        }
    }

    fn syscfg_read(&self, rel: u32) -> u32 {
        self.sys.get((rel >> 2) as usize).copied().unwrap_or(0)
    }

    fn exti_read(&self, rel: u32) -> u32 {
        match self.layout.exti_reg(rel) {
            Some(Reg::Imr) => self.imr,
            Some(Reg::Emr) => self.emr,
            Some(Reg::Rtsr) => self.rtsr,
            Some(Reg::Ftsr) => self.ftsr,
            Some(Reg::Swier) => self.swier,
            Some(Reg::Pr) => self.pr,
            None if self.layout == ExtiLayout::H7 => self.xr.get((rel >> 2) as usize).copied().unwrap_or(0),
            None => 0,
        }
    }
}

impl Mmio for SysExti {
    fn read(&mut self, offset: u32, size: u8, cx: &mut Cx) -> u32 {
        let rel = offset & 0x3fc;
        let v = if self.is_exti(offset) {
            self.exti_read(rel)
        } else {
            let (reg, bit) = self.layout.syscfg_clock();
            if cx.sys.clock_on(reg, bit) { self.syscfg_read(rel) } else { 0 }
        };
        lane_read(v, offset, size)
    }

    fn write(&mut self, offset: u32, size: u8, value: u32, cx: &mut Cx) {
        let rel = offset & 0x3fc;
        if !self.is_exti(offset) {
            let (reg, bit) = self.layout.syscfg_clock();
            if !cx.sys.clock_on(reg, bit) {
                return;
            }
            let mask = self.layout.syscfg_mask(rel);
            if mask == 0 {
                return;
            }
            let w = (rel >> 2) as usize;
            let new = (self.sys[w] & !mask) | (lane_write(self.sys[w], offset, size, value) & mask);
            self.sys[w] = new;
            if (0x08..=0x14).contains(&rel) {
                let k = ((rel - 8) >> 2) as usize;
                for line in 4 * k..4 * k + 4 {
                    self.baseline(line, cx);
                }
            } else if self.layout == ExtiLayout::H7 && rel == ODEN_OFF {
                cx.sys.oden = new & 1 != 0;
            }
            return;
        }
        let v = lane_write(self.exti_read(rel), offset, size, value);
        match self.layout.exti_reg(rel) {
            Some(Reg::Imr) => self.imr = v,
            Some(Reg::Emr) => self.emr = v,
            Some(Reg::Rtsr) => self.rtsr = v,
            Some(Reg::Ftsr) => self.ftsr = v,
            Some(Reg::Swier) => {
                let set = v & !self.swier;
                self.swier = v;
                self.pr |= set & 0xffff;
            }
            Some(Reg::Pr) => {
                // Pending bits are cleared by writing 1; clearing also clears the software request.
                let clr = lane_write(0, offset, size, value);
                self.pr &= !clr;
                self.swier &= !clr;
            }
            None => {
                if self.layout == ExtiLayout::H7 {
                    if let Some(x) = self.xr.get_mut((rel >> 2) as usize) {
                        *x = v;
                    }
                }
                return;
            }
        }
        self.update_irqs(cx);
    }

    fn peek(&mut self, offset: u32, _cx: &mut Cx) -> u32 {
        let rel = offset & 0x3fc;
        if self.is_exti(offset) { self.exti_read(rel) } else { self.syscfg_read(rel) }
    }

    fn on_pin(&mut self, pin: usize, level: u8, _cycle: u64, cx: &mut Cx) {
        let line = pin & 15;
        if self.selected(line) != pin {
            return;
        }
        let prev = std::mem::replace(&mut self.last[line], level);
        let bit = 1u32 << line;
        let rising = prev == 0 && level == 1 && self.rtsr & bit != 0;
        let falling = prev == 1 && level == 0 && self.ftsr & bit != 0;
        if rising || falling {
            self.pr |= bit;
            self.update_irqs(cx);
        }
    }

    fn reset(&mut self, cx: &mut Cx) {
        self.set_reset_values();
        cx.sys.oden = false;
        for line in 0..16 {
            self.baseline(line, cx);
        }
        self.update_irqs(cx);
    }

    fn inspect(&self, _cx: &Cx) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for line in 0..16 {
            if (self.imr | self.rtsr | self.ftsr) >> line & 1 != 0 {
                let edges = match (self.rtsr >> line & 1, self.ftsr >> line & 1) {
                    (1, 1) => "both edges",
                    (1, 0) => "rising",
                    (0, 1) => "falling",
                    _ => "no edge",
                };
                let p = self.selected(line);
                out.push((format!("Line {line}"), format!("P{}{}, {edges}, mask {}, pending {}", (b'A' + (p / 16) as u8) as char, line, self.imr >> line & 1, self.pr >> line & 1)));
            }
        }
        out
    }
}
