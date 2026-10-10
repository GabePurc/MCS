//! ARMv7-M core state: register file, program status, stack pointer banking, special registers.
//!
//! Reference: ARM DDI 0403E.e B1.4 (registers), B1.5 (exceptions).
//!
//! Layout notes (all chosen for a cheap executor):
//! * `r[13]` always holds the *active* stack pointer; the other one lives in `banked_sp`.
//! * `r[15]` is refreshed with `address + 4` before each instruction (the architectural PC read
//!   value); `pc` is the address of the *next* instruction to fetch.
//! * The APSR flags are separate booleans, materialized only for MRS / exception entry.
//! * The FP register file (`fpr`, S0-S31 with D0-D15 aliased onto pairs) and FPSCR live here too.

use serde::Serialize;

pub const PSR_N: u32 = 1 << 31;
pub const PSR_Z: u32 = 1 << 30;
pub const PSR_C: u32 = 1 << 29;
pub const PSR_V: u32 = 1 << 28;
pub const PSR_Q: u32 = 1 << 27;
pub const PSR_T: u32 = 1 << 24;

/// CONTROL register bits.
pub const CONTROL_NPRIV: u8 = 1;
pub const CONTROL_SPSEL: u8 = 2;
/// Floating-point context active: set when an FP instruction runs (FPCCR.ASPEN), selects the
/// extended exception frame.
pub const CONTROL_FPCA: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    None,
    /// Cycle limit of the current `run` call reached.
    Limit,
    /// BKPT executed (the PC points after it).
    Bkpt,
    /// Unrecoverable fault inside a fault handler (the core would be locked up).
    Lockup,
    /// `request_stop` was called.
    Requested,
}

pub struct Cpu {
    /// r0-r12, r13 = active SP, r14 = LR, r15 = current instruction address + 4.
    pub r: [u32; 16],
    /// Address of the next instruction to fetch.
    pub pc: u32,
    /// The stack pointer that is not active (MSP in thread mode on PSP, else PSP).
    pub banked_sp: u32,
    pub psp_active: bool,
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    pub q: bool,
    /// APSR.GE[3:0] (DSP extension).
    pub ge: u8,
    /// Current exception number (0 = thread mode).
    pub ipsr: u16,
    /// IT block state (EPSR ITSTATE[7:0]).
    pub itstate: u8,
    pub control: u8,
    pub primask: bool,
    pub faultmask: bool,
    pub basepri: u8,
    pub cycles: u64,
    pub instructions: u64,
    pub sleeping: bool,
    /// The sleep was entered by WFE (wakes on the event register or a preempting exception).
    pub sleep_wfe: bool,
    /// Event register (SEV / exception entry set it, WFE consumes it).
    pub event: bool,
    /// Local exclusive monitor.
    pub excl_valid: bool,
    pub excl_addr: u32,
    pub stop: StopReason,
    /// Floating-point registers S0-S31 (D0-D15 alias pairs: Dn = S(2n) low, S(2n+1) high).
    pub fpr: [u32; 32],
    pub fpscr: u32,
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            r: [0; 16],
            pc: 0,
            banked_sp: 0,
            psp_active: false,
            n: false,
            z: false,
            c: false,
            v: false,
            q: false,
            ge: 0,
            ipsr: 0,
            itstate: 0,
            control: 0,
            primask: false,
            faultmask: false,
            basepri: 0,
            cycles: 0,
            instructions: 0,
            sleeping: false,
            sleep_wfe: false,
            event: false,
            excl_valid: false,
            excl_addr: 0,
            stop: StopReason::None,
            fpr: [0; 32],
            fpscr: 0,
        }
    }

    /// Main stack pointer.
    #[inline]
    pub fn msp(&self) -> u32 {
        if self.psp_active {
            self.banked_sp
        } else {
            self.r[13]
        }
    }

    /// Process stack pointer.
    #[inline]
    pub fn psp(&self) -> u32 {
        if self.psp_active {
            self.r[13]
        } else {
            self.banked_sp
        }
    }

    pub fn set_msp(&mut self, v: u32) {
        if self.psp_active {
            self.banked_sp = v & !3;
        } else {
            self.r[13] = v & !3;
        }
    }

    pub fn set_psp(&mut self, v: u32) {
        if self.psp_active {
            self.r[13] = v & !3;
        } else {
            self.banked_sp = v & !3;
        }
    }

    /// Makes PSP (`true`) or MSP (`false`) the active stack pointer.
    #[inline]
    pub fn select_sp(&mut self, psp: bool) {
        if psp != self.psp_active {
            std::mem::swap(&mut self.r[13], &mut self.banked_sp);
            self.psp_active = psp;
        }
    }

    #[inline]
    pub fn handler_mode(&self) -> bool {
        self.ipsr != 0
    }

    /// Privileged execution (handler mode is always privileged).
    #[inline]
    pub fn privileged(&self) -> bool {
        self.ipsr != 0 || self.control & CONTROL_NPRIV == 0
    }

    /// APSR: flags N Z C V Q and GE[3:0].
    #[inline]
    pub fn apsr(&self) -> u32 {
        (self.n as u32) << 31 | (self.z as u32) << 30 | (self.c as u32) << 29 | (self.v as u32) << 28 | (self.q as u32) << 27 | (self.ge as u32) << 16
    }

    /// Sets N, Z, C, V and Q from bits 31:27 of `v`.
    #[inline]
    pub fn set_nzcvq(&mut self, v: u32) {
        self.n = v & PSR_N != 0;
        self.z = v & PSR_Z != 0;
        self.c = v & PSR_C != 0;
        self.v = v & PSR_V != 0;
        self.q = v & PSR_Q != 0;
    }

    /// Sets the whole APSR (flags and GE bits).
    pub fn set_apsr(&mut self, v: u32) {
        self.set_nzcvq(v);
        self.ge = ((v >> 16) & 0xf) as u8;
    }

    /// Full xPSR: flags, T bit, ITSTATE and exception number.
    pub fn xpsr(&self) -> u32 {
        let it = self.itstate as u32;
        self.apsr() | PSR_T | (it & 3) << 25 | (it >> 2) << 10 | self.ipsr as u32
    }

    /// Restores flags, ITSTATE and IPSR from a stacked xPSR (the T bit is ignored).
    pub fn set_xpsr(&mut self, v: u32) {
        self.set_apsr(v);
        self.itstate = (((v >> 10) & 0x3f) << 2 | ((v >> 25) & 3)) as u8;
        self.ipsr = (v & 0x1ff) as u16;
    }

    /// Evaluates a condition code against the current flags.
    #[inline]
    pub fn cond(&self, cc: u8) -> bool {
        let r = match cc >> 1 {
            0 => self.z,
            1 => self.c,
            2 => self.n,
            3 => self.v,
            4 => self.c && !self.z,
            5 => self.n == self.v,
            6 => self.n == self.v && !self.z,
            _ => return true,
        };
        r ^ (cc & 1 != 0 && cc != 15)
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}
