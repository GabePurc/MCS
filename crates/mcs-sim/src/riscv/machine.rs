//! RV32IMC machine: fetch / decode cache, executor, traps and interrupts.
//!
//! # Execution model
//!
//! [`Machine::run`] interprets pre-decoded [`Insn`]s (see [`Bus`] for the cache); the hot path
//! is allocation free and checks a single flag (`attn`) for pending interrupts / `wfi` sleep per
//! instruction. Peripherals are event driven: the owner of the machine advances time with
//! [`Machine::run`] / [`Machine::idle`] and calls [`Machine::set_irq`] when a peripheral's
//! interrupt line changes.
//!
//! # Traps
//!
//! Synchronous exceptions (cause numbers per the Privileged spec): illegal instruction (2; `mtval`
//! = instruction bits, also for unknown / read-only-violating CSR accesses), breakpoint (3; `mtval`
//! = pc), load / store address misaligned (4 / 6; `mtval` = address -- this hart does not emulate
//! misaligned accesses, like the ESP32-C3), load / store access fault (5 / 7: unmapped, no read /
//! write permission), instruction access fault (1: unmapped or not executable; for a 32-bit
//! instruction whose second halfword faults `mtval` is that halfword's address), ecall from
//! machine mode (11). Instruction address misaligned (0) cannot occur: with the C extension every
//! branch / jump target is 2-byte aligned, `mepc` / `jalr` targets clear bit 0, and bit 0 of a pc
//! set by the host is ignored.
//!
//! Interrupts: [`Machine::set_irq`] latches line `n` (1-31) into `mip` bit `n`; with `mstatus.MIE`
//! and the line enabled in `mie` the hart traps with `mcause = 0x8000_0000 | n` to `mtvec` (direct
//! mode) or `base + 4 * n` (vectored mode). When several lines are pending the priority is
//! MEI (11), MSI (3), MTI (7), then the highest line number.
//!
//! # Timing (approximation)
//!
//! One instruction retires at a time with the cycle counts below; flash/cache wait states, load-use
//! stalls and bus contention are not modelled. The ESP32-C3 core (a 4-stage in-order RV32IMC
//! pipeline, ESP32-C3 TRM chapter "ESP-RISC-V CPU") has no published per-instruction timing table,
//! so the values follow the pipeline structure: ALU / `lui` / `auipc` / CSR / `fence` / store 1,
//! load 2, `mul`/`mulh*` 1, `div`/`rem` 33 (32-iteration divider), `jal` 2, taken branch and `jalr`
//! 3, not-taken branch 1, trap entry and `mret` 3.

use mcs_core::riscv::{Insn, Op};

use super::bus::Bus;
use super::cpu::*;
use super::debug::{is_call, is_return, Dbg, StepCond};

pub const C_ALU: u64 = 1;
pub const C_LOAD: u64 = 2;
pub const C_STORE: u64 = 1;
pub const C_MUL: u64 = 1;
pub const C_DIV: u64 = 33;
pub const C_JAL: u64 = 2;
pub const C_JALR: u64 = 3;
pub const C_BRANCH_TAKEN: u64 = 3;
pub const C_TRAP: u64 = 3;
pub const C_MRET: u64 = 3;
pub const C_FENCE_I: u64 = 3;

/// Synchronous exception causes.
pub const CAUSE_FETCH_ACCESS: u32 = 1;
pub const CAUSE_ILLEGAL: u32 = 2;
pub const CAUSE_BREAKPOINT: u32 = 3;
pub const CAUSE_LOAD_MISALIGNED: u32 = 4;
pub const CAUSE_LOAD_ACCESS: u32 = 5;
pub const CAUSE_STORE_MISALIGNED: u32 = 6;
pub const CAUSE_STORE_ACCESS: u32 = 7;
pub const CAUSE_ECALL_M: u32 = 11;
/// `mcause` interrupt flag.
pub const CAUSE_IRQ: u32 = 1 << 31;
/// Host-internal pseudo cause: `ebreak` with [`Machine::halt_on_ebreak`].
const HOST_EBREAK: u32 = 0x100;

/// Why [`Machine::run`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// The cycle budget is used up.
    Cycles,
    /// `ebreak` executed with [`Machine::halt_on_ebreak`] set; pc is past the instruction.
    Ebreak,
    /// The hart executed `wfi` and sleeps until an enabled interrupt is pending. Time does not
    /// advance while sleeping; use [`Machine::idle`] and [`Machine::set_irq`].
    Wfi,
    /// A breakpoint address was reached (pc is at the breakpoint, the instruction has not executed).
    Breakpoint,
    /// A run-to address or step condition was met, or a device asked for a stop (system reset).
    Requested,
    /// The pc entered the boot ROM range ([`Machine::rom_range`]), whose contents are not simulated;
    /// pc is the ROM address, `ra` the caller's return address.
    RomCall,
}

/// A synchronous exception raised by an instruction.
#[derive(Clone, Copy, Debug)]
struct Trap {
    cause: u32,
    tval: u32,
}

/// Access to CSR numbers the core does not implement (ESP32-C3 custom CSRs, performance counters,
/// the debug trigger module...).
pub trait CsrHook: Send {
    /// Reads CSR `csr`; `None` raises an illegal-instruction exception.
    fn read(&mut self, csr: u16, cycles: u64) -> Option<u32>;
    /// Writes CSR `csr`; `false` raises an illegal-instruction exception.
    fn write(&mut self, csr: u16, value: u32) -> bool;
}

/// Static configuration of the hart.
#[derive(Clone, Copy, Debug)]
pub struct RvConfig {
    pub ids: HartIds,
    pub reset_pc: u32,
}

impl Default for RvConfig {
    /// ESP32-C3 identification; reset at the start of the IROM window.
    fn default() -> Self {
        Self { ids: HartIds::default(), reset_pc: 0x4200_0000 }
    }
}

pub struct Machine {
    pub cpu: Cpu,
    pub bus: Bus,
    pub cfg: RvConfig,
    /// Treat `ebreak` as a debugger breakpoint: stop with [`StopReason::Ebreak`] instead of trapping.
    pub halt_on_ebreak: bool,
    pub csr_hook: Option<Box<dyn CsrHook>>,
    /// Address range `[start, end)` whose execution stops the run loop with [`StopReason::RomCall`]
    /// (the pages must not be executable on the bus). Empty by default.
    pub rom_range: (u32, u32),
    /// Breakpoints, run-to address and step conditions (checked by the `CHK` variant of the loop).
    pub dbg: Dbg,
    /// A device asked the run loop to stop (see `Cx::request_stop`).
    halt_pending: bool,
    /// Interrupt taken at the next instruction boundary (`MIE` set and a pending enabled line).
    irq_gate: bool,
    sleeping: bool,
    /// `irq_gate || sleeping`: the only flag the run loop tests per instruction.
    attn: bool,
    /// Page tag of the last fetched page.
    ftag: u32,
    /// Start of that page in the code arena.
    fbase: usize,
    /// Cause of the last fetch fault.
    fault: Trap,
}

impl Machine {
    /// A machine with an empty address space; map memories into `bus` before running.
    pub fn new(cfg: RvConfig) -> Self {
        Self::with_bus(cfg, Bus::new())
    }

    pub fn with_bus(cfg: RvConfig, bus: Bus) -> Self {
        Self {
            cpu: Cpu::new(cfg.ids, cfg.reset_pc),
            bus,
            cfg,
            halt_on_ebreak: false,
            csr_hook: None,
            rom_range: (0, 0),
            dbg: Dbg::default(),
            halt_pending: false,
            irq_gate: false,
            sleeping: false,
            attn: false,
            ftag: u32::MAX,
            fbase: 0,
            fault: Trap { cause: 0, tval: 0 },
        }
    }

    /// Resets the hart (registers, CSRs, counters; `mip` follows the interrupt sources and is kept).
    pub fn reset(&mut self) {
        let mip = self.cpu.csr.mip;
        self.cpu = Cpu::new(self.cfg.ids, self.cfg.reset_pc);
        self.cpu.csr.mip = mip;
        self.sleeping = false;
        self.halt_pending = false;
        self.dbg.trap_depth = 0;
        self.dbg.step = None;
        self.dbg.skip_pc = None;
        self.dbg.set_run_to(None);
        self.recompute();
    }

    pub fn set_pc(&mut self, pc: u32) {
        self.cpu.pc = pc & !1;
    }

    /// Debugger view of plain memory (no side effects); `None` for peripherals / unmapped space.
    pub fn mem_read(&self, addr: u32, size: u32) -> Option<u32> {
        self.bus.peek(addr, size)
    }

    /// True while the hart sleeps in `wfi`.
    pub fn is_sleeping(&self) -> bool {
        self.sleeping
    }

    /// Advances time without executing (idle in `wfi`).
    pub fn idle(&mut self, cycles: u64) {
        self.cpu.cycles = self.cpu.cycles.wrapping_add(cycles);
    }

    // ---- interrupts -------------------------------------------------------------------------

    /// Latches interrupt line `line` (1-31) as pending.
    pub fn set_irq(&mut self, line: u32) {
        if (1..32).contains(&line) {
            self.cpu.csr.mip |= 1 << line;
            self.recompute();
        }
    }

    /// Clears the pending state of interrupt line `line`.
    pub fn clear_irq(&mut self, line: u32) {
        if (1..32).contains(&line) {
            self.cpu.csr.mip &= !(1 << line);
            self.recompute();
        }
    }

    /// Drives line `line` to `level` (level-sensitive sources).
    pub fn set_irq_line(&mut self, line: u32, level: bool) {
        if level {
            self.set_irq(line)
        } else {
            self.clear_irq(line)
        }
    }

    /// Replaces the whole pending vector (bit n = line n), as an interrupt controller would.
    pub fn set_irq_pending(&mut self, mask: u32) {
        self.cpu.csr.mip = mask & IRQ_MASK;
        self.recompute();
    }

    /// The current `mip` value.
    pub fn irq_pending(&self) -> u32 {
        self.cpu.csr.mip
    }

    /// Recomputes the interrupt gate / sleep state after `mip`, `mie` or `mstatus` changed.
    #[inline]
    fn recompute(&mut self) {
        let c = &self.cpu.csr;
        let pend = c.mip & c.mie != 0;
        self.irq_gate = pend && c.mstatus & MSTATUS_MIE != 0;
        if pend {
            self.sleeping = false;
        }
        self.attn = self.irq_gate || self.sleeping || self.halt_pending;
    }

    /// Re-evaluates the interrupt gate after the host changed `mstatus` / `mie` / `mip`.
    pub fn refresh_irq_state(&mut self) {
        self.recompute();
    }

    /// Applies interrupt-line changes and stop requests that devices queued outside a bus access (for
    /// example while the owner of the machine dispatched scheduler events).
    pub fn sync_irq(&mut self) {
        if self.bus.cx.irq_raise | self.bus.cx.irq_lower != 0 {
            self.apply_cx();
        }
    }

    /// Applies interrupt-line changes a peripheral requested during a bus access.
    #[cold]
    fn apply_cx(&mut self) {
        let cx = &mut self.bus.cx;
        let (set, clr) = (cx.irq_raise, cx.irq_lower);
        cx.irq_raise = 0;
        cx.irq_lower = 0;
        self.cpu.csr.mip = (self.cpu.csr.mip | set) & !clr & IRQ_MASK;
        if std::mem::take(&mut self.bus.cx.stop_req) {
            self.halt_pending = true;
        }
        self.recompute();
    }

    /// The pending enabled interrupt that wins arbitration.
    fn pick_irq(&self) -> u32 {
        let p = self.cpu.csr.mip & self.cpu.csr.mie;
        if p & (1 << IRQ_MEI) != 0 {
            IRQ_MEI
        } else if p & (1 << IRQ_MSI) != 0 {
            IRQ_MSI
        } else if p & (1 << IRQ_MTI) != 0 {
            IRQ_MTI
        } else {
            31 - p.leading_zeros()
        }
    }

    // ---- traps ------------------------------------------------------------------------------

    fn enter(&mut self, cause: u32, tval: u32, epc: u32, target: u32) {
        let c = &mut self.cpu.csr;
        c.mepc = epc & !1;
        c.mcause = cause;
        c.mtval = tval;
        let mie = c.mstatus & MSTATUS_MIE != 0;
        c.mstatus = if mie { MSTATUS_MPIE } else { 0 };
        self.cpu.pc = target;
        self.cpu.cycles += C_TRAP;
        self.recompute();
    }

    fn take_exception(&mut self, pc: u32, cause: u32, tval: u32) {
        let target = self.cpu.csr.mtvec & !3;
        self.enter(cause, tval, pc, target);
    }

    #[cold]
    fn take_interrupt(&mut self) {
        let n = self.pick_irq();
        let tv = self.cpu.csr.mtvec;
        let target = (tv & !3).wrapping_add(if tv & 1 != 0 { 4 * n } else { 0 });
        let pc = self.cpu.pc;
        self.enter(CAUSE_IRQ | n, 0, pc, target);
    }

    // ---- run loop ---------------------------------------------------------------------------

    /// Executes for at least `budget` cycles (the last instruction may overshoot) or until a stop
    /// condition. While breakpoints, a run-to address or a step condition are armed the checking
    /// variant of the loop runs; a breakpoint at the starting pc is skipped so execution can resume
    /// from it.
    pub fn run(&mut self, budget: u64) -> StopReason {
        if self.dbg.active() {
            self.run_loop::<true>(budget)
        } else {
            self.run_loop::<false>(budget)
        }
    }

    fn run_loop<const CHK: bool>(&mut self, budget: u64) -> StopReason {
        let limit = self.cpu.cycles.saturating_add(budget);
        // The program counter lives in a local so consecutive instructions do not wait on a
        // store-to-load round trip through memory; it is written back on every exit.
        let mut pc = self.cpu.pc & !1;
        // Resuming from the address a stop condition fired at must not fire it again.
        let mut skip = if CHK {
            self.dbg.hit_breakpoint = false;
            self.dbg.skip_pc.take() == Some(pc)
        } else {
            false
        };
        let reason = loop {
            if self.attn {
                self.cpu.pc = pc;
                if self.halt_pending {
                    self.halt_pending = false;
                    self.recompute();
                    break StopReason::Requested;
                }
                if self.irq_gate {
                    self.take_interrupt();
                    if CHK {
                        self.dbg.trap_depth += 1;
                    }
                    pc = self.cpu.pc;
                } else if self.sleeping {
                    break StopReason::Wfi;
                }
            }
            if self.cpu.cycles >= limit {
                break StopReason::Cycles;
            }
            if CHK {
                if skip {
                    skip = false;
                } else if self.dbg.wants(pc) {
                    if let Some(r) = self.check_stop(pc) {
                        self.dbg.skip_pc = Some(pc);
                        break r;
                    }
                }
            }
            let insn = self.fetch(pc);
            if insn.op == Op::Undecoded {
                if pc >= self.rom_range.0 && pc < self.rom_range.1 {
                    break StopReason::RomCall;
                }
                // fetch fault; `fetch_slow` left the cause in `self.fault`
                let t = self.fault;
                self.take_exception(pc, t.cause, t.tval);
                if CHK {
                    self.dbg.trap_depth += 1;
                }
                pc = self.cpu.pc;
                continue;
            }
            match self.exec(&insn, pc) {
                Ok(next) => {
                    pc = next;
                    self.cpu.instret += 1;
                    self.cpu.x[0] = 0;
                    if CHK && self.dbg.step.is_some() {
                        self.track(&insn);
                    }
                }
                Err(Trap { cause: HOST_EBREAK, .. }) => {
                    pc = pc.wrapping_add(insn.len as u32);
                    self.cpu.instret += 1;
                    break StopReason::Ebreak;
                }
                Err(t) => {
                    self.take_exception(pc, t.cause, t.tval);
                    if CHK {
                        self.dbg.trap_depth += 1;
                    }
                    pc = self.cpu.pc;
                }
            }
        };
        self.cpu.pc = pc;
        reason
    }

    /// Debugger conditions evaluated before the instruction at `pc` executes.
    #[cold]
    #[inline(never)]
    fn check_stop(&mut self, pc: u32) -> Option<StopReason> {
        let d = &mut self.dbg;
        if !d.bps.is_empty() && d.bps.binary_search(&pc).is_ok() {
            d.hit_breakpoint = true;
            return Some(StopReason::Breakpoint);
        }
        if d.run_to == Some(pc) {
            return Some(StopReason::Requested);
        }
        let st = d.step?;
        if d.trap_depth != st.trap_base {
            return None; // inside a handler entered during the step
        }
        let key = d.key_at(pc);
        let hit = match st.cond {
            StepCond::OverCall { ret } => pc == ret && st.depth <= 0,
            StepCond::Out { lines } => st.depth < 0 && (!lines || key != -1),
            StepCond::IntoSrc { start } => key != -1 && (key != start || st.depth != 0),
            StepCond::OverSrc { start } => {
                if key == -1 {
                    false
                } else if start == -1 {
                    true
                } else if st.depth != 0 {
                    st.depth < 0
                } else {
                    key >> 20 == start >> 20 && key != start
                }
            }
        };
        hit.then_some(StopReason::Requested)
    }

    /// Updates the step's call depth after `insn` executed.
    fn track(&mut self, insn: &Insn) {
        let d = &mut self.dbg;
        let Some(st) = d.step.as_mut() else { return };
        if insn.op == Op::Mret {
            let before = d.trap_depth;
            d.trap_depth = before.saturating_sub(1);
            if before <= st.trap_base {
                st.depth -= 1; // exception return out of the stepped context
            }
        } else if d.trap_depth != st.trap_base {
            // handler instructions do not change the depth
        } else if is_call(insn) {
            st.depth += 1;
        } else if is_return(insn) {
            st.depth -= 1;
        }
    }

    /// The decoded instruction at `pc` in plain memory (no side effects); `None` when unmapped or not
    /// readable.
    pub fn insn_at(&self, pc: u32) -> Option<Insn> {
        let lo = self.bus.peek(pc, 2)?;
        let word = if lo & 3 == 3 { lo | self.bus.peek(pc.wrapping_add(2), 2)? << 16 } else { lo };
        Some(mcs_core::riscv::decode(word))
    }

    /// Executes one instruction (or takes one pending interrupt, or reports `wfi` sleep).
    pub fn step(&mut self) -> StopReason {
        self.run(1)
    }

    /// The pre-decoded instruction at `pc`; `Op::Undecoded` means a fetch fault (see `fault`).
    #[inline(always)]
    fn fetch(&mut self, pc: u32) -> Insn {
        let mut idx = self.fbase + ((pc >> 1) & 0x7ff) as usize;
        if pc >> 12 != self.ftag || self.bus.code[idx].op == Op::Undecoded {
            // Slow path: fills the arena slot (the instruction is never returned by value so the
            // fast path keeps it in registers).
            if !self.fetch_slow(pc) {
                return Insn::UNDECODED;
            }
            idx = self.fbase + ((pc >> 1) & 0x7ff) as usize;
        }
        self.bus.code[idx]
    }

    /// Decodes the instruction at `pc` into the code arena and points the page cache at it.
    /// Returns false (with `fault` set) if the fetch faults.
    #[cold]
    #[inline(never)]
    fn fetch_slow(&mut self, pc: u32) -> bool {
        let fault = |tval| Trap { cause: CAUSE_FETCH_ACCESS, tval };
        let Ok(page) = self.bus.code_page(pc) else {
            self.fault = fault(pc);
            return false;
        };
        self.ftag = pc >> 12;
        self.fbase = page as usize * 0x800;
        if self.bus.code[self.fbase + ((pc >> 1) & 0x7ff) as usize].op != Op::Undecoded {
            return true;
        }
        match self.bus.decode_at(pc, page) {
            Ok(_) => true,
            Err(at) => {
                self.fault = fault(at);
                false
            }
        }
    }

    // ---- memory access ----------------------------------------------------------------------

    #[inline(always)]
    fn load(&mut self, addr: u32, size: u32) -> Result<u32, Trap> {
        if addr & (size - 1) != 0 {
            return Err(Trap { cause: CAUSE_LOAD_MISALIGNED, tval: addr });
        }
        match self.bus.read(addr, size, self.cpu.cycles) {
            Ok(v) => {
                if self.bus.cx.irq_raise | self.bus.cx.irq_lower != 0 {
                    self.apply_cx();
                }
                Ok(v)
            }
            Err(_) => Err(Trap { cause: CAUSE_LOAD_ACCESS, tval: addr }),
        }
    }

    #[inline(always)]
    fn store(&mut self, addr: u32, size: u32, value: u32) -> Result<(), Trap> {
        if addr & (size - 1) != 0 {
            return Err(Trap { cause: CAUSE_STORE_MISALIGNED, tval: addr });
        }
        match self.bus.write(addr, size, value, self.cpu.cycles) {
            Ok(()) => {
                if self.bus.cx.irq_raise | self.bus.cx.irq_lower != 0 {
                    self.apply_cx();
                }
                Ok(())
            }
            Err(_) => Err(Trap { cause: CAUSE_STORE_ACCESS, tval: addr }),
        }
    }

    // ---- CSRs -------------------------------------------------------------------------------

    #[cold]
    fn csr_instruction(&mut self, op: Op, rd: u8, rs1: u8, imm: i32, a: u32) -> Result<(), Trap> {
        let csr = (imm & 0xfff) as u16;
        let f3 = match op {
            Op::Csrrw => 1,
            Op::Csrrs => 2,
            Op::Csrrc => 3,
            Op::Csrrwi => 5,
            Op::Csrrsi => 6,
            _ => 7,
        };
        let illegal = || {
            let raw = (csr as u32) << 20 | (rs1 as u32) << 15 | f3 << 12 | (rd as u32) << 7 | 0x73;
            Trap { cause: CAUSE_ILLEGAL, tval: raw }
        };
        let (src, writes) = match op {
            Op::Csrrw => (a, true),
            Op::Csrrs | Op::Csrrc => (a, rs1 != 0),
            Op::Csrrwi => (rs1 as u32, true),
            _ => (rs1 as u32, rs1 != 0),
        };
        if writes && csr >> 10 == 3 {
            return Err(illegal());
        }
        let old = match self.cpu.csr_read(csr) {
            Some(v) => v,
            None => {
                let cycles = self.cpu.cycles;
                self.csr_hook.as_mut().and_then(|h| h.read(csr, cycles)).ok_or_else(illegal)?
            }
        };
        if writes {
            let new = match op {
                Op::Csrrw | Op::Csrrwi => src,
                Op::Csrrs | Op::Csrrsi => old | src,
                _ => old & !src,
            };
            let ok = self.cpu.csr_write(csr, new) || self.csr_hook.as_mut().is_some_and(|h| h.write(csr, new));
            if !ok {
                return Err(illegal());
            }
            self.recompute();
        }
        self.cpu.x[(rd & 31) as usize] = old;
        Ok(())
    }

    // ---- executor ---------------------------------------------------------------------------

    /// Executes `i` located at `pc`; returns the next pc and adds the cycle cost.
    #[inline(always)]
    fn exec(&mut self, i: &Insn, pc: u32) -> Result<u32, Trap> {
        let a = self.cpu.x[(i.rs1 & 31) as usize];
        let b = self.cpu.x[(i.rs2 & 31) as usize];
        let imm = i.imm as u32;
        let rd = (i.rd & 31) as usize;
        let mut next = pc.wrapping_add(i.len as u32);
        let mut cyc = C_ALU;
        macro_rules! wr {
            ($v:expr) => {{
                let v = $v;
                self.cpu.x[rd] = v;
            }};
        }
        macro_rules! branch {
            ($cond:expr) => {
                if $cond {
                    next = pc.wrapping_add(imm);
                    cyc = C_BRANCH_TAKEN;
                }
            };
        }
        match i.op {
            Op::Lui => wr!(imm),
            Op::Auipc => wr!(pc.wrapping_add(imm)),
            Op::Jal => {
                wr!(next);
                next = pc.wrapping_add(imm);
                cyc = C_JAL;
            }
            Op::Jalr => {
                let t = a.wrapping_add(imm) & !1;
                wr!(next);
                next = t;
                cyc = C_JALR;
            }
            Op::Beq => branch!(a == b),
            Op::Bne => branch!(a != b),
            Op::Blt => branch!((a as i32) < (b as i32)),
            Op::Bge => branch!((a as i32) >= (b as i32)),
            Op::Bltu => branch!(a < b),
            Op::Bgeu => branch!(a >= b),
            Op::Lb => {
                wr!(self.load(a.wrapping_add(imm), 1)? as i8 as i32 as u32);
                cyc = C_LOAD;
            }
            Op::Lh => {
                wr!(self.load(a.wrapping_add(imm), 2)? as i16 as i32 as u32);
                cyc = C_LOAD;
            }
            Op::Lw => {
                wr!(self.load(a.wrapping_add(imm), 4)?);
                cyc = C_LOAD;
            }
            Op::Lbu => {
                wr!(self.load(a.wrapping_add(imm), 1)?);
                cyc = C_LOAD;
            }
            Op::Lhu => {
                wr!(self.load(a.wrapping_add(imm), 2)?);
                cyc = C_LOAD;
            }
            Op::Sb => {
                self.store(a.wrapping_add(imm), 1, b)?;
                cyc = C_STORE;
            }
            Op::Sh => {
                self.store(a.wrapping_add(imm), 2, b)?;
                cyc = C_STORE;
            }
            Op::Sw => {
                self.store(a.wrapping_add(imm), 4, b)?;
                cyc = C_STORE;
            }
            Op::Addi => wr!(a.wrapping_add(imm)),
            Op::Slti => wr!(((a as i32) < (imm as i32)) as u32),
            Op::Sltiu => wr!((a < imm) as u32),
            Op::Xori => wr!(a ^ imm),
            Op::Ori => wr!(a | imm),
            Op::Andi => wr!(a & imm),
            Op::Slli => wr!(a << (imm & 31)),
            Op::Srli => wr!(a >> (imm & 31)),
            Op::Srai => wr!(((a as i32) >> (imm & 31)) as u32),
            Op::Add => wr!(a.wrapping_add(b)),
            Op::Sub => wr!(a.wrapping_sub(b)),
            Op::Sll => wr!(a << (b & 31)),
            Op::Slt => wr!(((a as i32) < (b as i32)) as u32),
            Op::Sltu => wr!((a < b) as u32),
            Op::Xor => wr!(a ^ b),
            Op::Srl => wr!(a >> (b & 31)),
            Op::Sra => wr!(((a as i32) >> (b & 31)) as u32),
            Op::Or => wr!(a | b),
            Op::And => wr!(a & b),
            Op::Mul => {
                wr!(a.wrapping_mul(b));
                cyc = C_MUL;
            }
            Op::Mulh => {
                wr!(((a as i32 as i64 * b as i32 as i64) >> 32) as u32);
                cyc = C_MUL;
            }
            Op::Mulhsu => {
                wr!(((a as i32 as i64 * b as i64) >> 32) as u32);
                cyc = C_MUL;
            }
            Op::Mulhu => {
                wr!(((a as u64 * b as u64) >> 32) as u32);
                cyc = C_MUL;
            }
            Op::Div => {
                wr!(if b == 0 { u32::MAX } else { (a as i32).wrapping_div(b as i32) as u32 });
                cyc = C_DIV;
            }
            Op::Divu => {
                wr!(a.checked_div(b).unwrap_or(u32::MAX));
                cyc = C_DIV;
            }
            Op::Rem => {
                wr!(if b == 0 { a } else { (a as i32).wrapping_rem(b as i32) as u32 });
                cyc = C_DIV;
            }
            Op::Remu => {
                wr!(a.checked_rem(b).unwrap_or(a));
                cyc = C_DIV;
            }
            Op::Fence => {}
            // Pre-decoded code is kept coherent by the bus (every write invalidates), so this only
            // models the pipeline flush.
            Op::FenceI => cyc = C_FENCE_I,
            Op::Csrrw | Op::Csrrs | Op::Csrrc | Op::Csrrwi | Op::Csrrsi | Op::Csrrci => self.csr_instruction(i.op, i.rd, i.rs1, i.imm, a)?,
            Op::Ecall => return Err(Trap { cause: CAUSE_ECALL_M, tval: 0 }),
            Op::Ebreak => {
                return Err(if self.halt_on_ebreak { Trap { cause: HOST_EBREAK, tval: 0 } } else { Trap { cause: CAUSE_BREAKPOINT, tval: pc } });
            }
            Op::Mret => {
                let c = &mut self.cpu.csr;
                let mpie = c.mstatus & MSTATUS_MPIE != 0;
                c.mstatus = MSTATUS_MPIE | if mpie { MSTATUS_MIE } else { 0 };
                next = c.mepc;
                cyc = C_MRET;
                self.recompute();
            }
            Op::Wfi => {
                let c = &self.cpu.csr;
                if c.mip & c.mie == 0 {
                    self.sleeping = true;
                    self.attn = true;
                }
            }
            Op::Illegal | Op::Undecoded => return Err(Trap { cause: CAUSE_ILLEGAL, tval: imm }),
        }
        self.cpu.cycles += cyc;
        Ok(next)
    }
}
