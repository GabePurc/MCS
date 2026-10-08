//! Interrupt flag / mask register pairs (TIFRn + TIMSKn, possibly shared by several timers as
//! on the ATtiny85). The pair is owned by this peripheral; the timers set flag bits directly in
//! data memory and call [`update_irqs`]. Flags are cleared by writing one or when the vector
//! executes.

use crate::avr::machine::{Cx, Peripheral};

pub struct IrqFlagsConfig {
    pub name: &'static str,
    pub flag_reg: u16,
    pub mask_reg: u16,
    /// (flag/enable bit mask, vector) for every source in the pair.
    pub map: Vec<(u8, u8)>,
}

/// Recomputes the interrupt request lines for `map` from the flag and mask registers.
pub fn update_irqs(cx: &mut Cx, flag_reg: u16, mask_reg: u16, map: &[(u8, u8)]) {
    let f = cx.cpu.data[flag_reg as usize] & cx.cpu.data[mask_reg as usize];
    for &(bit, v) in map {
        cx.cpu.set_irq(v, f & bit != 0);
    }
}

pub struct IrqFlags {
    c: IrqFlagsConfig,
    used: u8,
}

impl IrqFlags {
    pub fn new(c: IrqFlagsConfig) -> Self {
        let used = c.map.iter().fold(0, |m, e| m | e.0);
        Self { c, used }
    }

    /// Owned registers with their SBI/CBI read-clear masks.
    pub fn registers(&self) -> Vec<(u16, u8)> {
        vec![(self.c.flag_reg, 0xff), (self.c.mask_reg, 0)]
    }

    pub fn vectors(&self) -> Vec<Option<u8>> {
        self.c.map.iter().map(|e| Some(e.1)).collect()
    }

    fn update(&self, cx: &mut Cx) {
        update_irqs(cx, self.c.flag_reg, self.c.mask_reg, &self.c.map);
    }
}

impl Peripheral for IrqFlags {
    fn name(&self) -> &str {
        self.c.name
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        let a = addr as usize;
        if addr == self.c.flag_reg {
            cx.cpu.data[a] &= !v; // write one to clear
        } else {
            cx.cpu.data[a] = v & self.used;
        }
        self.update(cx);
    }

    fn ack(&mut self, vector: u8, cx: &mut Cx) {
        if let Some(&(bit, _)) = self.c.map.iter().find(|e| e.1 == vector) {
            cx.cpu.data[self.c.flag_reg as usize] &= !bit;
        }
        self.update(cx);
    }

    fn reset(&mut self, cx: &mut Cx) {
        self.update(cx);
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

pub struct GtccrConfig {
    pub addr: u16,
    /// TSM bit (timer synchronization mode).
    pub tsm: u8,
    /// Prescaler reset bits and the prescaler group each one resets.
    pub psr: Vec<(u8, u8)>,
    /// Bits that read as zero and are forwarded as `Trigger::GtccrStrobe` (FOC1A/FOC1B).
    pub strobes: u8,
    /// Bits stored as plain configuration (e.g. ATtiny85 PWM1B/COM1B), announced with RegWritten.
    pub config: u8,
}

/// General Timer/Counter Control Register: synchronization mode and prescaler resets shared by
/// the timers (they listen to `Trigger::TimerSync` / `Trigger::PrescalerReset`).
pub struct Gtccr {
    c: GtccrConfig,
}

impl Gtccr {
    pub fn new(c: GtccrConfig) -> Self {
        Self { c }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        vec![(self.c.addr, 0)]
    }
}

impl Peripheral for Gtccr {
    fn name(&self) -> &str {
        "GTCCR"
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        use crate::avr::machine::Trigger;
        let a = addr as usize;
        let now = cx.now();
        let old = cx.cpu.data[a];
        let tsm = v & self.c.tsm != 0;
        if (old & self.c.tsm != 0) != tsm {
            cx.sys.trigger(Trigger::TimerSync, tsm as u8, now);
        }
        let mut groups = 0;
        for &(bit, group) in &self.c.psr {
            if v & bit != 0 {
                groups |= group;
            }
        }
        if groups != 0 {
            cx.sys.trigger(Trigger::PrescalerReset, groups, now);
        }
        if v & self.c.strobes != 0 {
            cx.sys.trigger(Trigger::GtccrStrobe, v & self.c.strobes, now);
        }
        // PSR bits stay set while TSM is active (they are cleared by hardware otherwise).
        let psr_mask = self.c.psr.iter().fold(0, |m, e| m | e.0);
        let keep_psr = if tsm { v & psr_mask } else { 0 };
        cx.cpu.data[a] = (v & (self.c.tsm | self.c.config)) | keep_psr;
        if self.c.config != 0 && (old ^ v) & self.c.config != 0 {
            cx.sys.events.push_back(crate::avr::machine::Event::RegWritten(addr));
        }
    }

    fn reset(&mut self, _cx: &mut Cx) {}

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
