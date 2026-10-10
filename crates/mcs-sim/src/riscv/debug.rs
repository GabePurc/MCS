//! Debugger state of the RISC-V machine: breakpoints, run-to and step conditions.
//!
//! None of this is consulted by the fast run loop; while breakpoints or a step condition are armed
//! the machine runs the monomorphized `CHK` variant of its loop, which evaluates the conditions
//! between instructions and tracks call depth (calls versus returns) so source- and
//! instruction-level step over / out work without a shadow stack in the executor.

use mcs_core::riscv::{Insn, Op};

/// What the armed step stops on (see `Target::begin_step`).
#[derive(Clone, Copy, Debug)]
pub enum StepCond {
    /// Step over a call: stop at `ret` with the call depth back at 0.
    OverCall { ret: u32 },
    /// Step out of the current function; with `lines`, only stop at a statement start.
    Out { lines: bool },
    /// Source step into: first statement start that differs from `start` (or a different frame).
    IntoSrc { start: i32 },
    /// Source step over: next statement of the same file and frame (called functions are skipped).
    OverSrc { start: i32 },
}

#[derive(Clone, Copy, Debug)]
pub struct Stepper {
    pub cond: StepCond,
    /// Call depth relative to the start of the step (calls +1, returns -1).
    pub depth: i32,
    /// Trap nesting (`Dbg::trap_depth`) the step started in; instructions of handlers entered
    /// meanwhile do not change the depth and are not checked against the step condition.
    pub trap_base: u32,
}

#[derive(Default)]
pub struct Dbg {
    /// Breakpoint addresses, sorted (set through [`Dbg::set_breakpoints`]).
    pub bps: Vec<u32>,
    /// Run-to address (set through [`Dbg::set_run_to`]).
    pub run_to: Option<u32>,
    pub step: Option<Stepper>,
    /// One bit per `(pc >> 1) & 63` of the breakpoint / run-to addresses: lets the run loop skip the
    /// full check for almost every instruction.
    bloom: u64,
    /// Statement table: `(address, (file << 20) | line)` sorted by address.
    pub lines: Vec<(u32, i32)>,
    /// Trap (interrupt / exception) nesting observed while debugger conditions were armed.
    pub trap_depth: u32,
    /// The last stop was a breakpoint (as opposed to a step / run-to condition).
    pub hit_breakpoint: bool,
    /// Address where the last debugger stop fired: the next run starting there executes that instruction
    /// instead of stopping again.
    pub skip_pc: Option<u32>,
}

impl Dbg {
    #[inline]
    pub fn active(&self) -> bool {
        !self.bps.is_empty() || self.run_to.is_some() || self.step.is_some()
    }

    /// True when the instruction at `pc` may need a full [`Machine::check_stop`](super::Machine) evaluation.
    #[inline(always)]
    pub fn wants(&self, pc: u32) -> bool {
        self.step.is_some() || self.bloom >> ((pc >> 1) & 63) & 1 != 0
    }

    pub fn set_breakpoints(&mut self, mut pcs: Vec<u32>) {
        pcs.sort_unstable();
        pcs.dedup();
        self.bps = pcs;
        self.rebuild_bloom();
    }

    pub fn set_run_to(&mut self, pc: Option<u32>) {
        self.run_to = pc;
        self.rebuild_bloom();
    }

    fn rebuild_bloom(&mut self) {
        self.bloom = self.bps.iter().copied().chain(self.run_to).fold(0, |m, a| m | 1 << ((a >> 1) & 63));
    }

    /// Statement key starting exactly at `pc`, or -1.
    #[inline]
    pub fn key_at(&self, pc: u32) -> i32 {
        match self.lines.binary_search_by_key(&pc, |e| e.0) {
            Ok(i) => self.lines[i].1,
            Err(_) => -1,
        }
    }

    /// Key of the statement containing `pc` (the closest statement start at or before it), or -1.
    pub fn key_containing(&self, pc: u32) -> i32 {
        let i = self.lines.partition_point(|e| e.0 <= pc);
        if i == 0 {
            -1
        } else {
            self.lines[i - 1].1
        }
    }
}

/// True for the instructions that call a function (they link into `ra` / `t0`).
#[inline]
pub fn is_call(i: &Insn) -> bool {
    matches!(i.op, Op::Jal | Op::Jalr) && (i.rd == 1 || i.rd == 5)
}

/// True for `ret` / `jr ra` style returns (`jalr x0, 0(ra|t0)`).
#[inline]
pub fn is_return(i: &Insn) -> bool {
    i.op == Op::Jalr && i.rd == 0 && (i.rs1 == 1 || i.rs1 == 5) && i.imm == 0
}
