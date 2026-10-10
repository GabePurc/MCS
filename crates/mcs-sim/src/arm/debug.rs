//! Debugger state of the ARM machine: breakpoints, run-to and step conditions.
//!
//! None of this is consulted by the fast run loop; while breakpoints or a step condition are
//! armed the machine runs the monomorphized `CHK` variant of its loop, which evaluates the
//! conditions between instructions and tracks call depth (BL / BLX versus returns) so source- and
//! instruction-level step over / out work without a shadow stack in the executor.

use mcs_core::arm::thumb::{Insn, Op};

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
    /// Execution context the step started in (IPSR); instructions of interrupt handlers entered
    /// meanwhile do not change the depth.
    pub ipsr: u16,
}

#[derive(Default)]
pub struct Dbg {
    /// Breakpoint flags per halfword of flash (empty = none).
    pub bp: Vec<bool>,
    pub nbp: usize,
    pub run_to: Option<u32>,
    pub step: Option<Stepper>,
    /// Statement key per halfword of flash: `(file << 20) | line`, -1 where no statement starts.
    pub line_key: Vec<i32>,
    pub has_lines: bool,
    /// The last stop was a breakpoint (as opposed to a step / run-to condition).
    pub hit_breakpoint: bool,
    /// SRAM image last sent to the UI (the next state only carries it when it changed).
    pub ram_sent: Vec<u8>,
}

impl Dbg {
    #[inline]
    pub fn active(&self) -> bool {
        self.nbp > 0 || self.run_to.is_some() || self.step.is_some()
    }

    #[inline]
    pub fn key_at(&self, flash_off: u32) -> i32 {
        self.line_key.get((flash_off >> 1) as usize).copied().unwrap_or(-1)
    }
}

/// True for the instructions that call a function (the return address goes to LR).
#[inline]
pub fn is_call(i: &Insn) -> bool {
    matches!(i.op, Op::BL | Op::BLX_R)
}

/// True for the instructions that return from a function: `bx lr`, `pop {.., pc}`, `ldm {.., pc}`,
/// `ldr pc, ...`.
#[inline]
pub fn is_return(i: &Insn) -> bool {
    match i.op {
        Op::BX => i.rm == 14,
        Op::POP | Op::LDM | Op::LDMDB => i.imm & 0x8000 != 0,
        Op::LDR => i.rd == 15,
        _ => false,
    }
}
