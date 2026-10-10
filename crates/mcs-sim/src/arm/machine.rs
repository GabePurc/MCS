//! The ARMv7-M machine: core + memory + NVIC/SysTick/SCB, run loop, exception entry/return.
//!
//! References: ARM DDI 0403E.e B1.5 (exception model: entry B1.5.6, return B1.5.8, priorities
//! B1.5.4, lockup B1.5.15), B3 (system level), Cortex-M4 TRM ARM DDI 0439B (cycle counts).
//!
//! Timing model: the cycle counter advances by the per-instruction counts of Cortex-M4 TRM
//! table 3-1 plus fixed exception latencies (12 cycles entry / 12 return, 6 for tail-chaining).
//! Pipeline overlap between adjacent loads/stores, flash wait states and bus contention are not
//! modelled. Interrupts are taken at instruction boundaries (no late-arrival preemption of an
//! entry sequence in progress).

use crate::scheduler::Scheduler;

use super::bus::{Bus, Cx, MemConfig, Mmio, T_PERIPH, T_PPB};
use super::cpu::{Cpu, StopReason, CONTROL_FPCA, CONTROL_SPSEL};
use super::nvic::*;
use super::scb::*;
use super::systick::{SysTick, SYSTICK_OWNER};
use mcs_core::arm::thumb::{self, ArmFeatures, Insn};

/// Device configuration of the core and its memories.
#[derive(Clone, Debug)]
pub struct ArmConfig {
    pub mem: MemConfig,
    pub features: ArmFeatures,
    /// Number of external interrupt lines (<= 240).
    pub nirq: u32,
    /// Implemented priority bits (STM32: 4).
    pub prio_bits: u8,
    pub cpuid: u32,
    /// CPU cycles per SysTick tick when CLKSOURCE = 0 (STM32: HCLK/8).
    pub systick_ext_div: u32,
    pub systick_calib: u32,
}

impl Default for ArmConfig {
    /// Cortex-M4F with 4 priority bits and 82 interrupts (STM32G4-like).
    fn default() -> Self {
        Self {
            mem: MemConfig::default(),
            features: ArmFeatures::CORTEX_M4F,
            nirq: 82,
            prio_bits: 4,
            cpuid: 0x410f_c241,
            systick_ext_div: 8,
            systick_calib: 0,
        }
    }
}

impl ArmConfig {
    /// Cortex-M7 with the double-precision FPU (FPv5-D16).
    pub fn cortex_m7() -> Self {
        Self { features: ArmFeatures::CORTEX_M7, cpuid: 0x411f_c270, ..Self::default() }
    }

    /// Cortex-M3 (no DSP extension, no FPU).
    pub fn cortex_m3() -> Self {
        Self { features: ArmFeatures::BASE, cpuid: 0x411f_c231, ..Self::default() }
    }
}

pub struct Machine {
    pub cpu: Cpu,
    pub bus: Bus,
    pub nvic: Nvic,
    pub scb: Scb,
    pub systick: SysTick,
    pub sched: Scheduler,
    pub cfg: ArmConfig,
    /// Pre-decoded flash, indexed by halfword offset from the flash start.
    pub(crate) prog: Vec<Insn>,
    /// Group priorities of the preempted contexts (one entry per active exception).
    pub(crate) nest: Vec<i16>,
    /// Group priority of the innermost active exception ([`IDLE_PRIO`] when none).
    pub(crate) act_prio: i16,
    /// The run loop ends when `cpu.cycles >= stop_limit` (set to 0 to stop immediately).
    pub(crate) stop_limit: u64,
    /// AIRCR.SYSRESETREQ was written.
    pub reset_requested: bool,
}

const SPSEL: u8 = CONTROL_SPSEL;

impl Machine {
    pub fn new(cfg: ArmConfig) -> Self {
        let bus = Bus::new(&cfg.mem);
        let prog = vec![Insn::UNDEF; (cfg.mem.flash_size / 2) as usize];
        let mut m = Self {
            cpu: Cpu::new(),
            bus,
            nvic: Nvic::new(cfg.nirq, cfg.prio_bits),
            scb: Scb::new(cfg.cpuid),
            systick: SysTick::new(cfg.systick_ext_div, cfg.systick_calib),
            sched: Scheduler::new(),
            prog,
            nest: Vec::with_capacity(16 + cfg.nirq as usize),
            act_prio: IDLE_PRIO,
            stop_limit: u64::MAX,
            reset_requested: false,
            cfg,
        };
        m.decode_range(0, m.prog.len());
        m
    }

    /// Maps a peripheral; returns its event-owner index.
    pub fn add_peripheral(&mut self, base: u32, size: u32, dev: Box<dyn Mmio>) -> u8 {
        self.bus.add_peripheral(base, size, dev)
    }

    // ---- flash / program ------------------------------------------------------------------

    /// Copies `data` into flash at byte `offset` and re-decodes the affected instructions.
    pub fn write_flash(&mut self, offset: u32, data: &[u8]) {
        let o = offset as usize;
        let end = (o + data.len()).min(self.bus.flash.len());
        if o >= end {
            return;
        }
        self.bus.flash[o..end].copy_from_slice(&data[..end - o]);
        // A 32-bit instruction starting one halfword earlier also depends on the first word.
        self.decode_range((o / 2).saturating_sub(1), end.div_ceil(2));
    }

    /// Loads a firmware image at the flash base and resets the core.
    pub fn load_image(&mut self, image: &[u8]) {
        self.write_flash(0, image);
        self.reset();
    }

    fn decode_range(&mut self, from: usize, to: usize) {
        let f = &self.bus.flash;
        let feat = self.cfg.features;
        let to = to.min(self.prog.len());
        for idx in from..to {
            let o = idx * 2;
            let hw1 = u16::from_le_bytes([f[o], f[o + 1]]);
            let hw2 = if o + 3 < f.len() { u16::from_le_bytes([f[o + 2], f[o + 3]]) } else { 0 };
            self.prog[idx] = thumb::decode(hw1, hw2, feat);
        }
    }

    // ---- reset ----------------------------------------------------------------------------

    /// Power-on / system reset: initial SP and PC come from the vector table at VTOR (0, or the
    /// flash base when there is no boot alias).
    pub fn reset(&mut self) {
        let vt = if self.bus.flash_alias { 0 } else { self.bus.flash_base };
        self.cpu = Cpu::new();
        self.scb = Scb::new(self.cfg.cpuid);
        self.scb.vtor = vt;
        self.nvic.reset();
        self.sched.clear();
        self.systick.reset(&mut self.sched);
        for d in self.bus.devs.iter_mut() {
            d.reset();
        }
        self.nest.clear();
        self.act_prio = IDLE_PRIO;
        self.stop_limit = u64::MAX;
        self.reset_requested = false;
        let sp = self.mem_read(vt, 4).unwrap_or(0);
        let pc = self.mem_read(vt + 4, 4).unwrap_or(0);
        self.cpu.r[13] = sp & !3;
        self.cpu.pc = pc & !1;
    }

    // ---- memory access --------------------------------------------------------------------

    #[inline]
    pub fn mem_read(&mut self, addr: u32, size: u32) -> Option<u32> {
        match self.bus.read_mem(addr, size) {
            Some(v) => Some(v),
            None => self.mem_read_slow(addr, size),
        }
    }

    #[inline(never)]
    fn mem_read_slow(&mut self, addr: u32, size: u32) -> Option<u32> {
        match self.bus.top[(addr >> 24) as usize] {
            T_PERIPH => {
                let (dev, off) = self.bus.find(addr)?;
                let mut cx = Cx { cycles: self.cpu.cycles, nvic: &mut self.nvic, sched: &mut self.sched, owner: dev as u8 };
                Some(self.bus.devs[dev].read(off, size as u8, &mut cx))
            }
            T_PPB => self.ppb_read(addr, size),
            _ => None,
        }
    }

    #[inline]
    pub fn mem_write(&mut self, addr: u32, size: u32, v: u32) -> bool {
        self.bus.write_ram(addr, size, v) || self.mem_write_slow(addr, size, v)
    }

    #[inline(never)]
    fn mem_write_slow(&mut self, addr: u32, size: u32, v: u32) -> bool {
        match self.bus.top[(addr >> 24) as usize] {
            T_PERIPH => {
                let Some((dev, off)) = self.bus.find(addr) else { return false };
                let mut cx = Cx { cycles: self.cpu.cycles, nvic: &mut self.nvic, sched: &mut self.sched, owner: dev as u8 };
                self.bus.devs[dev].write(off, size as u8, v, &mut cx);
                true
            }
            T_PPB => self.ppb_write(addr, size, v),
            _ => false,
        }
    }

    // ---- fetch ----------------------------------------------------------------------------

    #[inline]
    fn fetch(&mut self, pc: u32) -> Option<Insn> {
        let o = pc.wrapping_sub(self.bus.flash_base);
        if o < self.bus.flash_size {
            return self.prog.get((o >> 1) as usize).copied();
        }
        self.fetch_slow(pc)
    }

    #[inline(never)]
    fn fetch_slow(&self, pc: u32) -> Option<Insn> {
        if let Some(o) = self.bus.flash_offset(pc) {
            return self.prog.get((o >> 1) as usize).copied();
        }
        // Code executing from RAM is decoded on every fetch.
        let b = self.bus.ram_slice(pc, 2)?;
        let hw1 = u16::from_le_bytes([b[0], b[1]]);
        let hw2 = if thumb::is_32bit(hw1) {
            let b2 = self.bus.ram_slice(pc + 2, 2)?;
            u16::from_le_bytes([b2[0], b2[1]])
        } else {
            0
        };
        Some(thumb::decode(hw1, hw2, self.cfg.features))
    }

    // ---- run control ----------------------------------------------------------------------

    pub fn cycles(&self) -> u64 {
        self.cpu.cycles
    }

    /// Asks the run loop to stop after the current instruction.
    pub fn request_stop(&mut self) {
        self.cpu.stop = StopReason::Requested;
        self.stop_limit = 0;
    }

    /// Runs until the cycle counter reaches `limit`, a BKPT, a lockup or a stop request.
    /// Returns the reason; `StopReason::Limit` means the cycle budget was used up.
    pub fn run(&mut self, limit: u64) -> StopReason {
        self.cpu.stop = StopReason::None;
        self.stop_limit = limit;
        while self.cpu.cycles < self.stop_limit {
            if self.sched.next <= self.cpu.cycles || self.nvic.dirty {
                self.service();
                if self.cpu.cycles >= self.stop_limit {
                    break;
                }
            }
            if self.cpu.sleeping {
                // Fast-forward to the next scheduled event (or the end of the budget).
                let t = self.stop_limit.min(self.sched.next);
                if t > self.cpu.cycles {
                    self.cpu.cycles = t;
                }
                continue;
            }
            let pc = self.cpu.pc;
            match self.fetch(pc) {
                Some(insn) => {
                    self.cpu.r[15] = pc.wrapping_add(4);
                    self.cpu.pc = pc.wrapping_add(insn.len as u32);
                    self.cpu.instructions += 1;
                    self.exec(&insn, pc);
                }
                None => self.raise_fault(pc, EXC_BUSFAULT, BFSR_IBUSERR),
            }
        }
        if self.cpu.stop == StopReason::None {
            self.cpu.stop = StopReason::Limit;
        }
        self.cpu.stop
    }

    /// Executes one instruction (or takes one exception entry).
    pub fn step(&mut self) -> StopReason {
        let target = self.cpu.cycles + 1;
        self.run(target)
    }

    /// Dispatches due scheduler events, then takes the highest-priority preempting exception.
    fn service(&mut self) {
        while let Some((key, at)) = self.sched.pop_due(self.cpu.cycles) {
            if key.owner == SYSTICK_OWNER {
                self.systick.on_event(at, &mut self.sched, &mut self.nvic);
            } else {
                let mut cx = Cx { cycles: at, nvic: &mut self.nvic, sched: &mut self.sched, owner: key.owner };
                self.bus.devs[key.owner as usize].on_event(key.tag, &mut cx);
            }
        }
        if self.nvic.dirty {
            self.nvic.dirty = false;
            let exec = self.exec_prio();
            if let Some(exc) = self.nvic.best_pending(exec) {
                self.exception_entry(exc);
            } else if self.cpu.sleeping && !self.cpu.sleep_wfe && self.nvic.any_pending() {
                // WFI also wakes for an interrupt that is masked by PRIMASK / BASEPRI.
                self.cpu.sleeping = false;
            }
        }
        if self.cpu.sleeping && self.cpu.sleep_wfe && self.cpu.event {
            self.cpu.event = false;
            self.cpu.sleeping = false;
        }
        if self.reset_requested {
            self.reset();
        }
    }

    // ---- exceptions -----------------------------------------------------------------------

    /// Current execution priority (lowest value wins): innermost active exception, PRIMASK,
    /// FAULTMASK and BASEPRI.
    #[inline]
    pub fn exec_prio(&self) -> i16 {
        let mut p = self.act_prio;
        if self.cpu.faultmask {
            p = p.min(-1);
        } else if self.cpu.primask {
            p = p.min(0);
        }
        if self.cpu.basepri != 0 {
            p = p.min(self.nvic.group_of_raw(self.cpu.basepri));
        }
        p
    }

    fn lockup(&mut self) {
        self.cpu.stop = StopReason::Lockup;
        self.stop_limit = 0;
    }

    /// Raises a synchronous fault for the instruction at `pc` (which is re-stacked as the return
    /// address). Escalates to HardFault when the fault handler is disabled or cannot preempt, and
    /// locks up when HardFault itself cannot be taken.
    pub(crate) fn raise_fault(&mut self, pc: u32, exc: u16, bits: u32) {
        self.cpu.pc = pc;
        self.scb.cfsr |= bits;
        let exec = self.exec_prio();
        let enabled = match exc {
            EXC_MEMMANAGE => self.scb.shcsr & SHCSR_MEMFAULTENA != 0,
            EXC_BUSFAULT => self.scb.shcsr & SHCSR_BUSFAULTENA != 0,
            EXC_USAGEFAULT => self.scb.shcsr & SHCSR_USGFAULTENA != 0,
            _ => true,
        };
        if exc != EXC_HARDFAULT && enabled && self.nvic.group_prio(exc) < exec {
            self.nvic.set_sys_pending(exc);
            return;
        }
        if exec <= -1 {
            self.lockup();
            return;
        }
        if exc != EXC_HARDFAULT {
            self.scb.hfsr |= HFSR_FORCED;
        }
        self.nvic.set_sys_pending(EXC_HARDFAULT);
    }

    /// Precise data bus error at `addr` for the instruction at `pc`.
    #[cold]
    pub(crate) fn data_fault(&mut self, pc: u32, addr: u32) {
        self.scb.bfar = addr;
        self.raise_fault(pc, EXC_BUSFAULT, BFSR_PRECISERR | BFSR_BFARVALID);
    }

    /// Exception entry (ARM DDI 0403E.e B1.5.6): stack the 8-word frame (26 words with the
    /// floating-point context when CONTROL.FPCA is set), switch to handler mode.
    ///
    /// Lazy FP stacking (FPCCR.LSPEN) is performed eagerly: the 16 single registers and FPSCR are
    /// always written at entry, so FPCCR.LSPACT never becomes set and the handler sees the same
    /// register values either way. The extended frame is 0x68 bytes: R0-R3, R12, LR, ReturnAddress,
    /// xPSR, S0-S15, FPSCR and one reserved word.
    fn exception_entry(&mut self, exc: u16) {
        let ret_pc = self.cpu.pc;
        let thread = self.cpu.ipsr == 0;
        let ext = self.cpu.control & CONTROL_FPCA != 0 && self.cfg.features.has_fpu();
        let mut exc_return: u32 = if !thread {
            0xffff_fff1
        } else if !self.cpu.psp_active {
            0xffff_fff9
        } else {
            0xffff_fffd
        };
        if ext {
            exc_return &= !0x10;
        }
        let mut frame = self.cpu.r[13];
        let align = self.scb.ccr & CCR_STKALIGN != 0 && frame & 4 != 0;
        frame = frame.wrapping_sub(if ext { 0x68 } else { 0x20 });
        if align {
            frame &= !4;
        }
        let psr = self.cpu.xpsr() | (align as u32) << 9;
        let c = &self.cpu;
        let words = [c.r[0], c.r[1], c.r[2], c.r[3], c.r[12], c.r[14], ret_pc, psr];
        for (k, w) in words.iter().enumerate() {
            if !self.mem_write(frame.wrapping_add(4 * k as u32), 4, *w) {
                self.scb.cfsr |= BFSR_STKERR;
                self.lockup();
                return;
            }
        }
        if ext {
            for k in 0..18usize {
                let w = match k {
                    16 => self.cpu.fpscr,
                    17 => 0,
                    _ => self.cpu.fpr[k],
                };
                if !self.mem_write(frame.wrapping_add(0x20 + 4 * k as u32), 4, w) {
                    self.scb.cfsr |= BFSR_STKERR;
                    self.lockup();
                    return;
                }
            }
            // The handler starts with a clean FP state: FPSCR takes the FPDSCR defaults.
            self.cpu.control &= !CONTROL_FPCA;
            self.cpu.fpscr = self.scb.fpdscr;
            self.cpu.cycles += 18;
        }
        self.cpu.r[13] = frame;
        self.enter_handler(exc, exc_return);
        self.cpu.cycles += 12;
    }

    /// Common tail of entry and tail-chaining: handler mode, vector fetch, bookkeeping.
    fn enter_handler(&mut self, exc: u16, exc_return: u32) {
        let Some(vec) = self.mem_read(self.scb.vtor.wrapping_add(4 * exc as u32), 4) else {
            self.scb.hfsr |= HFSR_VECTTBL;
            self.lockup();
            return;
        };
        self.cpu.select_sp(false);
        self.cpu.control &= !SPSEL;
        self.cpu.ipsr = exc;
        self.cpu.itstate = 0;
        self.cpu.r[14] = exc_return;
        self.cpu.pc = vec & !1;
        self.nvic.take_pending(exc);
        self.nvic.set_active(exc, true);
        self.nest.push(self.act_prio);
        self.act_prio = self.nvic.group_prio(exc);
        self.cpu.sleeping = false;
        self.cpu.excl_valid = false;
        // A higher-priority exception may already be waiting.
        self.nvic.dirty = true;
    }

    /// Exception return via `EXC_RETURN` in `v` (ARM DDI 0403E.e B1.5.8), with tail-chaining.
    pub(crate) fn exception_return(&mut self, v: u32, pc: u32) {
        let exc = self.cpu.ipsr;
        let to_handler = v & 0xf == 1;
        let ext = v & 0x10 == 0;
        let ok = matches!(v & 0xf, 0x1 | 0x9 | 0xd) && v >> 5 == 0x07ff_ffff && (!ext || self.cfg.features.has_fpu());
        let remaining = self.nest.len().saturating_sub(1);
        if !ok || (to_handler && remaining == 0) || (!to_handler && remaining > 0 && self.scb.ccr & CCR_NONBASETHRDENA == 0) {
            self.raise_fault(pc, EXC_USAGEFAULT, UFSR_INVPC);
            return;
        }
        self.nvic.set_active(exc, false);
        self.act_prio = self.nest.pop().unwrap_or(IDLE_PRIO);
        if exc != EXC_NMI {
            self.cpu.faultmask = false;
        }
        // Tail-chain: a pending exception that beats the context we return to is entered without
        // unstacking (6 cycles) and keeps the same EXC_RETURN.
        let exec = self.exec_prio();
        if let Some(next) = self.nvic.best_pending(exec) {
            self.enter_handler(next, v);
            self.cpu.cycles += 6;
            return;
        }
        let use_psp = v & 4 != 0;
        let frame = if use_psp { self.cpu.psp() } else { self.cpu.msp() };
        let mut w = [0u32; 8];
        for (k, slot) in w.iter_mut().enumerate() {
            match self.mem_read(frame.wrapping_add(4 * k as u32), 4) {
                Some(x) => *slot = x,
                None => {
                    self.scb.cfsr |= BFSR_UNSTKERR;
                    self.lockup();
                    return;
                }
            }
        }
        if ext {
            // Extended frame: S0-S15, FPSCR (the FP context is restored before the core registers).
            let mut fp = [0u32; 17];
            for (k, slot) in fp.iter_mut().enumerate() {
                match self.mem_read(frame.wrapping_add(0x20 + 4 * k as u32), 4) {
                    Some(x) => *slot = x,
                    None => {
                        self.scb.cfsr |= BFSR_UNSTKERR;
                        self.lockup();
                        return;
                    }
                }
            }
            self.cpu.fpr[..16].copy_from_slice(&fp[..16]);
            self.cpu.fpscr = fp[16] & super::fpu::FPSCR_WMASK;
            self.cpu.cycles += 18;
        }
        let frame_size = if ext { 0x68 } else { 0x20 };
        let sp_after = frame.wrapping_add(frame_size).wrapping_add(if w[7] & (1 << 9) != 0 { 4 } else { 0 });
        let c = &mut self.cpu;
        c.r[0] = w[0];
        c.r[1] = w[1];
        c.r[2] = w[2];
        c.r[3] = w[3];
        c.r[12] = w[4];
        c.r[14] = w[5];
        c.pc = w[6] & !1;
        c.set_xpsr(w[7]);
        c.select_sp(use_psp && !to_handler);
        c.r[13] = sp_after;
        c.control = (c.control & !CONTROL_FPCA) | if ext { CONTROL_FPCA } else { 0 };
        if !to_handler {
            if use_psp {
                c.control |= SPSEL;
            } else {
                c.control &= !SPSEL;
            }
        }
        c.excl_valid = false;
        c.cycles += 12;
        self.nvic.dirty = true;
        if self.scb.scr & 2 != 0 && self.cpu.ipsr == 0 {
            // SLEEPONEXIT: sleep after returning to thread mode.
            self.cpu.sleeping = true;
            self.cpu.sleep_wfe = false;
        }
    }
}
