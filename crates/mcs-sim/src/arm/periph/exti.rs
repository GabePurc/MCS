//! SYSCFG (EXTI line routing) and EXTI (RM0440 sections 10 and 16) as one device mapped over
//! `SYSCFG_BASE .. SYSCFG_BASE + 0x800` (EXTI sits at +0x400).
//!
//! EXTI: lines 0-15 follow the GPIO selected by SYSCFG_EXTICRx; rising / falling edge detection
//! sets the pending bit (PR1), the interrupt mask (IMR1) gates the NVIC line. EXTI0-4 have their
//! own interrupts, lines 5-9 and 10-15 share EXTI9_5 and EXTI15_10. SWIER sets pending bits in
//! software. Event mode (EMR1) is stored; events do not wake WFE. Lines 16+ (internal sources) and
//! the second register bank (IMR2...) are not modelled. SYSCFG registers need the SYSCFGEN clock
//! (APB2ENR bit 0); EXTI itself is always clocked.

use crate::arm::bus::{Cx, Mmio};

use super::{lane_read, lane_write};

const EXTI_OFF: u32 = 0x400;

pub struct SysExti {
    irqs: [u16; 16],
    // SYSCFG
    memrmp: u32,
    cfgr1: u32,
    exticr: [u32; 4],
    scsr: u32,
    cfgr2: u32,
    swpr: u32,
    // EXTI
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
    pub fn new(irqs: &[u16]) -> Self {
        let mut a = [0u16; 16];
        for (d, s) in a.iter_mut().zip(irqs) {
            *d = *s;
        }
        let mut x = Self { irqs: a, memrmp: 0, cfgr1: 0, exticr: [0; 4], scsr: 0, cfgr2: 0, swpr: 0, imr: 0, emr: 0, rtsr: 0, ftsr: 0, swier: 0, pr: 0, last: [0; 16] };
        x.set_reset_values();
        x
    }

    fn set_reset_values(&mut self) {
        self.memrmp = 0;
        self.cfgr1 = 0x7c00_0001;
        self.exticr = [0; 4];
        self.scsr = 0;
        self.cfgr2 = 0;
        self.swpr = 0;
        self.imr = 0xff82_0000;
        self.emr = 0;
        self.rtsr = 0;
        self.ftsr = 0;
        self.swier = 0;
        self.pr = 0;
        self.last = [0; 16];
    }

    /// GPIO index selected for `line`.
    #[inline]
    fn selected(&self, line: usize) -> usize {
        let port = (self.exticr[line >> 2] >> (4 * (line & 3))) & 0xf;
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

    fn syscfg_read(&self, off: u32) -> u32 {
        match off {
            0x00 => self.memrmp,
            0x04 => self.cfgr1,
            0x08 => self.exticr[0],
            0x0c => self.exticr[1],
            0x10 => self.exticr[2],
            0x14 => self.exticr[3],
            0x18 => self.scsr,
            0x1c => self.cfgr2,
            0x20 => self.swpr,
            _ => 0,
        }
    }

    fn exti_read(&self, off: u32) -> u32 {
        match off {
            0x00 => self.imr,
            0x04 => self.emr,
            0x08 => self.rtsr,
            0x0c => self.ftsr,
            0x10 => self.swier,
            0x14 => self.pr,
            _ => 0,
        }
    }
}

impl Mmio for SysExti {
    fn read(&mut self, offset: u32, size: u8, cx: &mut Cx) -> u32 {
        let v = if offset >= EXTI_OFF {
            self.exti_read((offset - EXTI_OFF) & !3)
        } else if cx.sys.clock_on(5, 0) {
            self.syscfg_read(offset & !3)
        } else {
            0
        };
        lane_read(v, offset, size)
    }

    fn write(&mut self, offset: u32, size: u8, value: u32, cx: &mut Cx) {
        if offset < EXTI_OFF {
            if !cx.sys.clock_on(5, 0) {
                return;
            }
            let off = offset & !3;
            let old = self.syscfg_read(off);
            let v = lane_write(old, offset, size, value);
            match off {
                0x00 => self.memrmp = v & 7,
                0x04 => self.cfgr1 = v,
                0x08..=0x14 => {
                    let k = ((off - 8) >> 2) as usize;
                    self.exticr[k] = v & 0x7777;
                    for line in 4 * k..4 * k + 4 {
                        self.baseline(line, cx);
                    }
                }
                0x18 => self.scsr = v,
                0x1c => self.cfgr2 = v,
                0x20 => self.swpr = v,
                _ => {}
            }
            return;
        }
        let off = (offset - EXTI_OFF) & !3;
        let v = lane_write(self.exti_read(off), offset, size, value);
        match off {
            0x00 => self.imr = v,
            0x04 => self.emr = v,
            0x08 => self.rtsr = v,
            0x0c => self.ftsr = v,
            0x10 => {
                let set = v & !self.swier;
                self.swier = v;
                self.pr |= set & 0xffff;
            }
            0x14 => {
                // Pending bits are cleared by writing 1; clearing also clears the software request.
                let clr = lane_write(0, offset, size, value);
                self.pr &= !clr;
                self.swier &= !clr;
            }
            _ => return,
        }
        self.update_irqs(cx);
    }

    fn peek(&mut self, offset: u32, _cx: &mut Cx) -> u32 {
        if offset >= EXTI_OFF { self.exti_read((offset - EXTI_OFF) & !3) } else { self.syscfg_read(offset & !3) }
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
