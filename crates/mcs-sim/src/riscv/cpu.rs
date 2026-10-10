//! RV32 hart state: integer registers, program counter, performance counters and the
//! machine-mode CSR file.
//!
//! Implemented CSRs (RISC-V Privileged Architecture 20240411, ch. 2-3): `mstatus` (MIE, MPIE;
//! MPP hardwired to machine mode), `mstatush`, `misa` (RV32IMC, read-only), `mie`, `mip` (bits
//! are driven by [`Machine::set_irq`](super::Machine::set_irq); software writes are ignored),
//! `mtvec` (direct / vectored), `mscratch`, `mepc`, `mcause`, `mtval`, `mcounteren`,
//! `mcycle[h]`, `minstret[h]`, the read-only user aliases `cycle[h]` / `instret[h]`,
//! `mvendorid`, `marchid`, `mimpid`, `mhartid`, `mconfigptr`. The hardware performance monitor
//! CSRs (`mhpmcounter3-31[h]`, `hpmcounter3-31[h]`, `mhpmevent3-31`) and `mcountinhibit` read as
//! zero and ignore writes. `time[h]` and every other number are not implemented (illegal
//! instruction) unless a [`CsrHook`](super::CsrHook) provides them.

/// `mstatus` bit positions.
pub const MSTATUS_MIE: u32 = 1 << 3;
pub const MSTATUS_MPIE: u32 = 1 << 7;
/// `mstatus.MPP`: hardwired to machine mode (this hart has no other privilege levels).
pub const MSTATUS_MPP: u32 = 3 << 11;

/// `misa`: MXL = 32, extensions I, M, C.
pub const MISA: u32 = (1 << 30) | (1 << 8) | (1 << 12) | (1 << 2);

/// Interrupt lines 1-31 (bit n = cause n). Line 0 does not exist.
pub const IRQ_MASK: u32 = 0xffff_fffe;

/// Standard machine interrupt causes (software, timer, external).
pub const IRQ_MSI: u32 = 3;
pub const IRQ_MTI: u32 = 7;
pub const IRQ_MEI: u32 = 11;

/// Identification registers; the defaults are the values read from an ESP32-C3.
#[derive(Clone, Copy, Debug)]
pub struct HartIds {
    pub mvendorid: u32,
    pub marchid: u32,
    pub mimpid: u32,
    pub mhartid: u32,
}

impl Default for HartIds {
    /// Espressif JEDEC id (0x612), ESP32-C3 `marchid` / `mimpid`.
    fn default() -> Self {
        Self { mvendorid: 0x612, marchid: 0x8000_0001, mimpid: 1, mhartid: 0 }
    }
}

/// Machine-mode CSR state.
#[derive(Clone, Debug, Default)]
pub struct Csrs {
    /// Only MIE and MPIE are stored (MPP reads as machine mode).
    pub mstatus: u32,
    pub mie: u32,
    /// Interrupt-pending bits as driven by the interrupt sources.
    pub mip: u32,
    /// Bits 31:2 = vector base, bit 0 = vectored mode.
    pub mtvec: u32,
    pub mscratch: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,
    pub mcounteren: u32,
    /// `mcycle` / `minstret` = counter + offset (software writes adjust the offset).
    pub cycle_off: u64,
    pub instret_off: u64,
    pub ids: HartIds,
}

/// Register file, pc, counters and CSRs of the hart.
#[derive(Clone, Debug)]
pub struct Cpu {
    pub x: [u32; 32],
    pub pc: u32,
    /// CPU clock cycles executed (including idle time spent in `wfi`).
    pub cycles: u64,
    /// Retired instructions.
    pub instret: u64,
    pub csr: Csrs,
}

impl Cpu {
    pub fn new(ids: HartIds, pc: u32) -> Self {
        Self { x: [0; 32], pc, cycles: 0, instret: 0, csr: Csrs { ids, ..Csrs::default() } }
    }

    /// Reads a CSR; `None` if the number is not implemented here.
    pub fn csr_read(&self, csr: u16) -> Option<u32> {
        let c = &self.csr;
        let mcycle = self.cycles.wrapping_add(c.cycle_off);
        let minstret = self.instret.wrapping_add(c.instret_off);
        Some(match csr {
            0x300 => c.mstatus | MSTATUS_MPP,
            0x301 => MISA,
            0x304 => c.mie,
            0x305 => c.mtvec,
            0x306 => c.mcounteren,
            0x310 => 0,
            0x340 => c.mscratch,
            0x341 => c.mepc,
            0x342 => c.mcause,
            0x343 => c.mtval,
            0x344 => c.mip,
            0xb00 | 0xc00 => mcycle as u32,
            0xb80 | 0xc80 => (mcycle >> 32) as u32,
            0xb02 | 0xc02 => minstret as u32,
            0xb82 | 0xc82 => (minstret >> 32) as u32,
            0xf11 => c.ids.mvendorid,
            0xf12 => c.ids.marchid,
            0xf13 => c.ids.mimpid,
            0xf14 => c.ids.mhartid,
            0xf15 => 0,
            // mcountinhibit, mhpmevent3-31, mhpmcounter3-31[h], hpmcounter3-31[h]
            0x320..=0x33f | 0xb03..=0xb1f | 0xb83..=0xb9f | 0xc03..=0xc1f | 0xc83..=0xc9f => 0,
            _ => return None,
        })
    }

    /// Writes a CSR (the caller has already rejected writes to read-only numbers). Returns false if
    /// the number is not implemented here.
    pub fn csr_write(&mut self, csr: u16, v: u32) -> bool {
        match csr {
            0x300 => self.csr.mstatus = v & (MSTATUS_MIE | MSTATUS_MPIE),
            0x301 | 0x310 | 0x344 => {}
            0x304 => self.csr.mie = v & IRQ_MASK,
            0x305 => self.csr.mtvec = v & !2,
            0x306 => self.csr.mcounteren = v & 7,
            0x340 => self.csr.mscratch = v,
            0x341 => self.csr.mepc = v & !1,
            0x342 => self.csr.mcause = v,
            0x343 => self.csr.mtval = v,
            0xb00 => {
                let cur = self.cycles.wrapping_add(self.csr.cycle_off);
                self.csr.cycle_off = ((cur & !0xffff_ffff) | v as u64).wrapping_sub(self.cycles);
            }
            0xb80 => {
                let cur = self.cycles.wrapping_add(self.csr.cycle_off);
                self.csr.cycle_off = ((cur & 0xffff_ffff) | (v as u64) << 32).wrapping_sub(self.cycles);
            }
            0xb02 => {
                let cur = self.instret.wrapping_add(self.csr.instret_off);
                self.csr.instret_off = ((cur & !0xffff_ffff) | v as u64).wrapping_sub(self.instret);
            }
            0xb82 => {
                let cur = self.instret.wrapping_add(self.csr.instret_off);
                self.csr.instret_off = ((cur & 0xffff_ffff) | (v as u64) << 32).wrapping_sub(self.instret);
            }
            0x320..=0x33f | 0xb03..=0xb1f | 0xb83..=0xb9f => {}
            _ => return false,
        }
        true
    }
}
