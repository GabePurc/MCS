//! EEPROM controller: EEAR, EEDR, EECR with the EEMPE/EEPE timed write sequence, erase / write /
//! atomic programming modes and their programming times, read stalls and the EE_READY interrupt.
//! The contents live in `Cpu::eeprom` (non-volatile across resets and power cycles).
//!
//! Sources: DS40002061B section 8.4 (EEPROM data memory, table 8-2 programming times);
//! Atmel-2586Q section 5.3.

use crate::avr::machine::{Cx, Peripheral};

pub struct EepromConfig {
    pub eecr: u16,
    pub eedr: u16,
    pub eearl: u16,
    pub eearh: Option<u16>,
    pub vector: u8,
}

const EERE: u8 = 0x01;
const EEPE: u8 = 0x02;
const EEMPE: u8 = 0x04;
const EERIE: u8 = 0x08;
const EEPM: u8 = 0x30;
const EV_DONE: u8 = 0;
/// Programming times per EEPM mode (s): atomic erase+write, erase only, write only.
const PROG_TIME: [f64; 3] = [3.4e-3, 1.8e-3, 1.8e-3];

pub struct Eeprom {
    c: EepromConfig,
    mempe_until: u64,
    /// Pending programming operation: (address, data, mode).
    pending: Option<(usize, u8, u8)>,
}

impl Eeprom {
    pub fn new(c: EepromConfig) -> Self {
        Self { c, mempe_until: 0, pending: None }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        let mut v = vec![(self.c.eecr, 0), (self.c.eedr, 0), (self.c.eearl, 0)];
        v.extend(self.c.eearh.map(|a| (a, 0)));
        v
    }

    fn addr(&self, cx: &Cx) -> usize {
        let lo = cx.cpu.data[self.c.eearl as usize] as usize;
        let hi = self.c.eearh.map(|a| cx.cpu.data[a as usize] as usize).unwrap_or(0);
        ((hi << 8) | lo) % cx.cpu.eeprom.len().max(1)
    }

    fn update_irq(&self, cx: &mut Cx) {
        let cr = cx.cpu.data[self.c.eecr as usize];
        cx.cpu.set_irq(self.c.vector, cr & EERIE != 0 && cr & EEPE == 0);
    }
}

impl Peripheral for Eeprom {
    fn name(&self) -> &str {
        "EEPROM"
    }

    fn read(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        self.peek(addr, cx)
    }

    fn peek(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        let v = cx.cpu.data[addr as usize];
        if addr == self.c.eecr && cx.now() <= self.mempe_until {
            v | EEMPE
        } else {
            v
        }
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        let a = addr as usize;
        let busy = self.pending.is_some();
        if addr != self.c.eecr {
            if busy && addr != self.c.eedr {
                cx.warn("eeprom-busy", "EEAR written while an EEPROM write is in progress (ignored)");
                return;
            }
            cx.cpu.data[a] = v;
            return;
        }
        let now = cx.now();
        let old = cx.cpu.data[a];
        // EEPM can only be changed while no write is in progress.
        let eepm = if busy { old & EEPM } else { v & EEPM };
        cx.cpu.data[a] = (old & EEPE) | eepm | (v & EERIE);
        if v & EEMPE != 0 && v & EEPE == 0 {
            self.mempe_until = now + 4;
        }
        if v & EEPE != 0 && !busy {
            if now <= self.mempe_until {
                let mode = (eepm >> 4).min(2);
                let ea = self.addr(cx);
                let data = cx.cpu.data[self.c.eedr as usize];
                self.pending = Some((ea, data, mode));
                cx.cpu.data[a] |= EEPE;
                self.mempe_until = 0;
                // The CPU is halted for two cycles when EEPE is set.
                cx.cpu.cycles += 2;
                let at = cx.sys.clock.cycle_at(cx.time_seconds() + PROG_TIME[mode as usize]).max(cx.now() + 1);
                cx.schedule(EV_DONE, at);
            } else {
                cx.warn("eeprom-eempe", "EECR.EEPE written without EEMPE in the preceding 4 cycles: EEPROM write ignored");
            }
        }
        if v & EERE != 0 {
            if busy {
                cx.warn("eeprom-read-busy", "EEPROM read while a write is in progress (ignored)");
            } else {
                let ea = self.addr(cx);
                cx.cpu.data[self.c.eedr as usize] = cx.cpu.eeprom[ea];
                // The CPU is halted for four cycles on a read.
                cx.cpu.cycles += 4;
            }
        }
        self.update_irq(cx);
    }

    fn on_event(&mut self, _tag: u8, _cycle: u64, cx: &mut Cx) {
        if let Some((ea, data, mode)) = self.pending.take() {
            let cell = &mut cx.cpu.eeprom[ea];
            *cell = match mode {
                0 => data,
                1 => 0xff,
                _ => *cell & data,
            };
            cx.cpu.eeprom_version += 1;
        }
        cx.cpu.data[self.c.eecr as usize] &= !EEPE;
        self.update_irq(cx);
    }

    fn reset(&mut self, cx: &mut Cx) {
        // A reset during programming aborts the write (the cell content is undefined; keep it).
        cx.cancel(EV_DONE);
        self.pending = None;
        self.mempe_until = 0;
        self.update_irq(cx);
    }

    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        let ea = self.addr(cx);
        vec![
            ("Size".into(), format!("{} bytes", cx.cpu.eeprom.len())),
            ("Address".into(), format!("0x{ea:03X}")),
            ("State".into(), if self.pending.is_some() { "Programming" } else { "Ready" }.into()),
        ]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
