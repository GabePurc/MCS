//! AVR CPU state: register file, status register, stack pointer, memories, pre-decoded program,
//! interrupt lines, sleep state and debugger bookkeeping. The executor lives in `machine.rs`
//! because instructions touch the data bus, which dispatches to peripherals.

use std::sync::Arc;

use mcs_core::avr::device::{fuse_field_value, AvrDeviceSpec};
use mcs_core::avr::isa::{self, feature, op, DecodeTable, OP_COUNT};
use serde::Serialize;

pub const SREG_C: u8 = 0x01;
pub const SREG_Z: u8 = 0x02;
pub const SREG_N: u8 = 0x04;
pub const SREG_V: u8 = 0x08;
pub const SREG_S: u8 = 0x10;
pub const SREG_H: u8 = 0x20;
pub const SREG_T: u8 = 0x40;
pub const SREG_I: u8 = 0x80;

/// `io_owner` values: plain storage, CPU registers mapped in I/O space, or a peripheral index.
pub const IO_PLAIN: u8 = 0xff;
pub const IO_SREG: u8 = 0xfe;
pub const IO_SPL: u8 = 0xfd;
pub const IO_SPH: u8 = 0xfc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    None,
    /// Cycle limit of the current `run` call reached.
    Limit,
    Breakpoint,
    BreakInsn,
    InvalidOpcode,
    /// ARM: the core locked up (fault inside a fault handler).
    Lockup,
    /// A step predicate or `request_stop` asked to stop.
    Requested,
    /// RISC-V: execution entered the boot ROM, which is not simulated.
    RomCall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallFrame {
    /// Word address execution returns to.
    pub return_pc: u32,
    /// Word address of the called function / vector.
    pub target_pc: u32,
    /// Interrupt vector number, or -1 for a regular call.
    pub vector: i16,
    /// SP after pushing the return address.
    pub sp: u32,
}

const MAX_SHADOW_STACK: usize = 512;

pub struct Cpu {
    pub spec: &'static AvrDeviceSpec,
    pub rc: bool,
    pub table: Arc<DecodeTable>,

    pub r: [u8; 32],
    /// Word address.
    pub pc: u32,
    pub sp: u16,
    pub sreg: u8,
    pub cycles: u64,
    pub instructions: u64,

    pub flash: Vec<u8>,
    pub flash_words: u32,
    pub pc_mask: u32,
    /// 22-bit PC (flash > 128 KB): calls, returns and interrupts use 3 stack bytes.
    pub pc3: bool,
    /// Data addresses of EIND / RAMPZ (None = absent, reads as 0).
    pub eind_addr: Option<u16>,
    pub rampz_addr: Option<u16>,
    /// Data space backing store (registers on classic cores + I/O + SRAM).
    pub data: Vec<u8>,
    pub sram_start: u16,
    pub data_end: u16,
    pub io_base: u16,
    /// Fuse bytes (see `AvrDeviceSpec::fuses`).
    pub fuses: Vec<u8>,
    pub lock_bits: u8,
    /// EEPROM contents (non-volatile: kept across resets and power cycles).
    pub eeprom: Vec<u8>,
    /// Bumped on every EEPROM change (the UI refreshes its copy).
    pub eeprom_version: u64,
    /// Word address of the reset vector (boot loader start when BOOTRST is programmed).
    pub reset_vector: u32,
    /// Word address of the interrupt vector table (moved to the boot section by IVSEL).
    pub vector_base: u32,

    // Pre-decoded program (parallel arrays indexed by word address).
    pub(crate) ops: Vec<u8>,
    pub(crate) oa: Vec<i32>,
    pub(crate) ob: Vec<i32>,
    pub(crate) cyc: [u8; OP_COUNT],
    pub(crate) len: [u8; OP_COUNT],

    /// Owner of each data address below SRAM (see IO_* constants).
    pub io_owner: Vec<u8>,
    /// Bits that read as 0 for SBI/CBI read-modify-write (write-1-to-clear flags, PINx).
    pub rmw_clear: Vec<u8>,

    pub vector_count: usize,
    /// Words per vector table entry (1 with RJMP tables, 2 with JMP).
    pub vector_words: u32,
    pub irq_pending: Vec<bool>,
    /// Peripheral index that acknowledges each vector.
    pub irq_owner: Vec<u8>,
    /// Set when a vector became pending; cleared when a scan finds nothing pending.
    pub irq_dirty: bool,
    /// Suppresses interrupt service for one instruction (after SEI / RETI).
    pub irq_inhibit: bool,

    pub sleeping: bool,
    pub sleep_mode: u8,
    /// Per sleep mode: vectors able to wake the core.
    pub wake_mask: Vec<Vec<bool>>,

    pub breakpoints: Vec<bool>,
    /// Per-word execution counters (empty = profiling off). Filled by `Machine::run`.
    pub exec_counts: Vec<u32>,
    /// Words whose counter went from 0 to 1 since the last take (capacity = all words, so
    /// pushing never allocates).
    pub exec_touched: Vec<u32>,
    pub shadow_stack: Vec<CallFrame>,
    pub stop_reason: StopReason,
    pub halt: bool,
}

impl Cpu {
    pub fn new(spec: &'static AvrDeviceSpec) -> Self {
        let rc = spec.features & feature::RC != 0;
        let table = isa::decode_table(spec.features);
        let flash_words = spec.flash_size / 2;
        let data_end = spec.sram_start + spec.sram_size;
        let mut cyc = [1u8; OP_COUNT];
        let mut len = [1u8; OP_COUNT];
        for d in isa::insns() {
            cyc[d.op as usize] = if rc { d.cycles_rc } else { d.cycles };
            len[d.op as usize] = d.words;
        }
        let pc3 = flash_words > 65536;
        if pc3 && !rc {
            // Microchip AVR Instruction Set Manual (DS40002198B): cycle counts with a 22-bit PC.
            for (o, c) in [(op::CALL, 5), (op::RCALL, 4), (op::ICALL, 4), (op::EICALL, 4), (op::RET, 5), (op::RETI, 5)] {
                cyc[o as usize] = c;
            }
        }
        let pc_mask = flash_words.next_power_of_two() - 1;
        let padded = pc_mask as usize + 1;
        let vector_count = spec.vector_count();
        let mut cpu = Self {
            spec,
            rc,
            table,
            r: [0; 32],
            pc: 0,
            sp: 0,
            sreg: 0,
            cycles: 0,
            instructions: 0,
            flash: vec![0xff; spec.flash_size as usize],
            flash_words,
            pc_mask,
            pc3,
            eind_addr: spec.register("EIND").map(|r| r.addr),
            rampz_addr: spec.register("RAMPZ").map(|r| r.addr),
            data: vec![0; data_end as usize],
            sram_start: spec.sram_start,
            data_end,
            io_base: spec.io_base,
            fuses: spec.fuse_defaults(),
            lock_bits: 0xff,
            eeprom: vec![0xff; spec.eeprom_size as usize],
            eeprom_version: 0,
            reset_vector: 0,
            vector_base: 0,
            ops: vec![0; padded],
            oa: vec![0; padded],
            ob: vec![0; padded],
            cyc,
            len,
            io_owner: vec![IO_PLAIN; spec.sram_start as usize],
            rmw_clear: vec![0; spec.sram_start as usize],
            vector_count,
            vector_words: if spec.features & feature::JMP != 0 { 2 } else { 1 },
            irq_pending: vec![false; vector_count],
            irq_owner: vec![IO_PLAIN; vector_count],
            irq_dirty: false,
            irq_inhibit: false,
            sleeping: false,
            sleep_mode: 0,
            wake_mask: Vec::new(),
            breakpoints: vec![false; padded],
            exec_counts: Vec::new(),
            exec_touched: Vec::new(),
            shadow_stack: Vec::with_capacity(64),
            stop_reason: StopReason::None,
            halt: false,
        };
        cpu.predecode_all();
        cpu
    }

    // ---------------------------------------------------------------------------------
    // Program memory
    // ---------------------------------------------------------------------------------

    pub fn load_flash(&mut self, image: &[u8]) {
        self.flash.fill(0xff);
        let n = image.len().min(self.flash.len());
        self.flash[..n].copy_from_slice(&image[..n]);
        self.predecode_all();
    }

    /// Debugger edit of program memory; re-decodes the affected words.
    pub fn write_flash_byte(&mut self, byte_addr: u32, value: u8) {
        if (byte_addr as usize) < self.flash.len() {
            self.flash[byte_addr as usize] = value;
            let w = byte_addr >> 1;
            self.predecode(w);
            if w > 0 {
                self.predecode(w - 1);
            }
        }
    }

    #[inline]
    pub fn flash_word(&self, word_addr: u32) -> u16 {
        let i = ((word_addr & self.pc_mask) << 1) as usize;
        match self.flash.get(i..i + 2) {
            Some(b) => u16::from_le_bytes([b[0], b[1]]),
            None => 0xffff, // padding beyond a non-power-of-two flash reads as erased
        }
    }

    fn predecode_all(&mut self) {
        for i in 0..=self.pc_mask {
            self.predecode(i);
        }
    }

    fn predecode(&mut self, i: u32) {
        let w1 = self.flash_word(i);
        let w2 = self.flash_word(i + 1);
        let d = isa::decode(&self.table, w1, w2);
        let (o, a, b) = match d.def {
            Some(def) => (def.op, d.values.first().copied().unwrap_or(0), d.values.get(1).copied().unwrap_or(0)),
            None => (0, w1 as i32, 0),
        };
        self.ops[i as usize] = o;
        self.oa[i as usize] = a;
        self.ob[i as usize] = b;
    }

    /// Op id at a word address (0 = invalid).
    pub fn op_at(&self, word_addr: u32) -> u8 {
        self.ops[(word_addr & self.pc_mask) as usize]
    }

    pub fn insn_words_at(&self, word_addr: u32) -> u32 {
        self.len[self.op_at(word_addr) as usize] as u32
    }

    /// Fuse bit (by name) is programmed (= 0).
    pub fn fuse_programmed(&self, name: &str) -> bool {
        self.spec.fuse_field(name).is_some_and(|(i, f)| self.fuses.get(i).is_some_and(|b| b & f.mask == 0))
    }

    /// Value of a (multi-bit) fuse field, e.g. CKSEL or BODLEVEL.
    pub fn fuse_value(&self, name: &str) -> Option<u8> {
        self.spec.fuse_field(name).map(|(i, f)| fuse_field_value(&self.fuses, i, f))
    }

    // ---------------------------------------------------------------------------------
    // Interrupts
    // ---------------------------------------------------------------------------------

    /// Peripherals call this whenever a source's (flag && enable) state changes.
    #[inline]
    pub fn set_irq(&mut self, vector: u8, pending: bool) {
        let v = vector as usize;
        if v >= self.irq_pending.len() {
            return;
        }
        if pending && !self.irq_pending[v] {
            self.irq_dirty = true;
        }
        self.irq_pending[v] = pending;
    }

    /// Highest-priority pending vector (lowest number), if any.
    pub fn pending_vector(&self) -> Option<u8> {
        (1..self.vector_count).find(|&v| self.irq_pending[v]).map(|v| v as u8)
    }

    pub fn wake_pending(&self) -> bool {
        let mask = self.wake_mask.get(self.sleep_mode as usize);
        (1..self.vector_count).any(|v| self.irq_pending[v] && mask.is_none_or(|m| m[v]))
    }

    pub(crate) fn push_frame(&mut self, return_pc: u32, target_pc: u32, vector: i16) {
        if self.shadow_stack.len() >= MAX_SHADOW_STACK {
            self.shadow_stack.remove(0);
        }
        self.shadow_stack.push(CallFrame { return_pc, target_pc, vector, sp: self.sp as u32 });
    }

    pub(crate) fn pop_frame(&mut self) {
        // Discard frames whose stack slot was unwound (handles manual stack manipulation).
        while self.shadow_stack.last().is_some_and(|f| (f.sp + 2 + self.pc3 as u32) < self.sp as u32) {
            self.shadow_stack.pop();
        }
        self.shadow_stack.pop();
    }

    /// CPU part of a reset. I/O register reset values are applied by the machine.
    pub fn reset(&mut self, power_on: bool) {
        self.pc = self.reset_vector;
        self.vector_base = 0;
        self.sreg = 0;
        self.sp = self.data_end - 1;
        self.irq_pending.fill(false);
        self.irq_dirty = false;
        self.irq_inhibit = false;
        self.sleeping = false;
        self.sleep_mode = 0;
        self.shadow_stack.clear();
        self.halt = false;
        if power_on {
            self.r = [0; 32];
            self.data.fill(0);
            self.cycles = 0;
            self.instructions = 0;
        }
    }
}
