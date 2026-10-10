//! [`Target`] implementation for the ESP32-C3: debugger session support (program loading, stepping,
//! breakpoints, writes, state snapshots) on top of [`Esp32c3`].
//!
//! Position values (`pc`, breakpoints, run-to) are byte addresses on this architecture.

use std::any::Any;

use mcs_core::device::DeviceRef;
use mcs_core::program::LoadedProgram;
use mcs_core::riscv::{decode, Insn};

use super::{Esp32c3, RunStop};
use crate::avr::peripherals::serial::SerialConfig;
use crate::avr::CallFrame;
use crate::pins::{ExtDrive, PinGenerator};
use crate::protocol::{CoreState, CpuField, MachineState, PeripheralInfo, PinState, RamExtra, StepKind};
use crate::riscv::bus::{STIM_OWNER, BRIDGE_OWNER};
use crate::riscv::debug::{is_call, StepCond, Stepper};
use crate::target::{Sent, StepPlan, StopReason, Target};

/// Words of stack scanned when reconstructing the call stack (from SP up, bounded by the stack's own
/// memory: reads outside RAM end the scan).
const UNWIND_WORDS: u32 = 512;
/// Frames reported to the UI.
const MAX_FRAMES: usize = 64;

impl Esp32c3 {
    /// True when the instruction(s) just before `ret` are a call that links into `ra` / `t0`, i.e. `ret` is
    /// a return address.
    fn is_return_address(&self, ret: u32) -> bool {
        if ret & 1 != 0 || ret < 4 {
            return false;
        }
        self.call_before(ret).is_some()
    }

    /// The call instruction ending at `ret` and its start address.
    fn call_before(&self, ret: u32) -> Option<(Insn, u32)> {
        let b = &self.machine.bus;
        b.peek(ret, 2)?; // the return address itself must be readable memory
        if let Some(w) = b.peek(ret - 4, 4) {
            if w & 3 == 3 {
                let i = decode(w);
                if is_call(&i) {
                    return Some((i, ret - 4));
                }
            }
        }
        let h = b.peek(ret - 2, 2)?;
        if h & 3 != 3 {
            let i = decode(h);
            if is_call(&i) && i.len == 2 {
                return Some((i, ret - 2));
            }
        }
        None
    }

    /// Static target of the call ending at `ret`: `jal`, or the `auipc` + `jalr` pair of a far call (0 when it
    /// cannot be determined, e.g. an indirect call).
    fn call_target(&self, ret: u32) -> u32 {
        use mcs_core::riscv::Op;
        let Some((i, at)) = self.call_before(ret) else { return 0 };
        match i.op {
            Op::Jal => at.wrapping_add(i.imm as u32),
            Op::Jalr if at >= 4 => match self.machine.bus.peek(at - 4, 4).map(decode) {
                Some(a) if a.op == Op::Auipc && a.rd == i.rs1 => (at - 4).wrapping_add(a.imm as u32).wrapping_add(i.imm as u32),
                _ => 0,
            },
            _ => 0,
        }
    }

    /// Call stack (innermost last), reconstructed from the live stack by looking for stacked values that are
    /// return addresses (they follow a call instruction), plus `ra` for a leaf function. A heuristic -- stale
    /// words below the live frames can show up as extra frames -- but it costs nothing while running and
    /// survives RTOS context switches.
    pub fn call_stack(&self) -> Vec<CallFrame> {
        let m = &self.machine;
        let sp = m.cpu.x[2];
        let end = sp.wrapping_add(4 * UNWIND_WORDS);
        let mut frames: Vec<CallFrame> = Vec::new();
        // A leaf function (or the first instructions of a call) still has the return address in `ra`: it is the
        // innermost call.
        let ra = m.cpu.x[1];
        let ra_frame = self.is_return_address(ra).then(|| CallFrame { return_pc: ra, target_pc: self.call_target(ra), vector: -1, sp });
        let mut a = sp & !3;
        let mut first_ret = None;
        while a < end && frames.len() < MAX_FRAMES {
            let Some(w) = m.bus.peek(a, 4) else { break };
            if self.is_return_address(w) {
                first_ret.get_or_insert(w);
                frames.push(CallFrame { return_pc: w, target_pc: self.call_target(w), vector: -1, sp: a + 4 });
            }
            a += 4;
        }
        if let Some(f) = ra_frame {
            if first_ret != Some(ra) {
                frames.insert(0, f);
            }
        }
        frames.reverse();
        frames
    }

    fn current_line_key(&self) -> i32 {
        self.machine.dbg.key_containing(self.machine.cpu.pc)
    }

    /// Debugger write through the CPU's bus (devices see it like a store).
    fn mem_write(&mut self, addr: u32, size: u32, value: u32) -> bool {
        let now = self.machine.cpu.cycles;
        let ok = self.machine.bus.write(addr, size, value, now).is_ok();
        self.after_io();
        ok
    }

    fn stop_kind(&self, r: RunStop) -> StopReason {
        match r {
            RunStop::Limit => StopReason::Limit,
            RunStop::Ebreak => StopReason::BreakInsn,
            RunStop::Breakpoint => StopReason::Breakpoint,
            RunStop::RomCall => StopReason::RomCall,
            RunStop::Requested => StopReason::Requested,
        }
    }
}

impl Target for Esp32c3 {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn device(&self) -> DeviceRef {
        DeviceRef::Riscv(self.spec)
    }

    fn load_program(&mut self, program: Option<&LoadedProgram>) {
        if let Some(p) = program {
            self.load(p);
        }
        self.reset_with(true);
    }

    fn debugger_reset(&mut self) {
        self.reset_with(false);
    }

    fn power_cycle(&mut self) {
        self.reset_with(true);
    }

    fn set_source_map(&mut self, program: Option<&LoadedProgram>) {
        let mut lines: Vec<(u32, i32)> = Vec::new();
        if let Some(p) = program {
            // Several statement rows can share an address; keep the file of the first one and the last row
            // from that file (the most specific statement in the user's file).
            for row in p.lines.iter().filter(|r| r.is_stmt) {
                let k = ((row.file as i32) << 20) | (row.line as i32 & 0xfffff);
                match lines.last_mut() {
                    Some(last) if last.0 == row.address => {
                        if last.1 >> 20 == k >> 20 {
                            last.1 = k;
                        }
                    }
                    _ => lines.push((row.address, k)),
                }
            }
            lines.sort_by_key(|e| e.0);
            lines.dedup_by_key(|e| e.0);
        }
        self.machine.dbg.lines = lines;
    }

    fn cycles(&self) -> u64 {
        self.machine.cpu.cycles
    }

    fn elapsed_seconds(&self) -> f64 {
        self.machine.bus.cx.sys.time_at(self.machine.cpu.cycles)
    }

    fn cycle_at(&self, seconds: f64) -> u64 {
        self.machine.bus.cx.sys.clock.cycle_at(seconds)
    }

    fn pc(&self) -> u32 {
        self.machine.cpu.pc
    }

    fn run_until(&mut self, limit: u64) -> StopReason {
        let r = self.run(limit);
        self.stop_kind(r)
    }

    fn step_one(&mut self) -> StopReason {
        // A sleeping core first runs on to the event that wakes it.
        for _ in 0..64 {
            let next = self.machine.bus.cx.sched.next;
            if !self.machine.is_sleeping() || next == u64::MAX {
                break;
            }
            self.run(next);
        }
        let now = self.machine.cpu.cycles;
        self.machine.bus.service(now);
        self.machine.sync_irq();
        self.machine.dbg.skip_pc = Some(self.machine.cpu.pc);
        match self.machine.step() {
            crate::riscv::StopReason::Ebreak => StopReason::BreakInsn,
            crate::riscv::StopReason::RomCall => {
                // Report through the normal run path so the message is posted.
                self.run(now + 1);
                StopReason::RomCall
            }
            _ => StopReason::Limit,
        }
    }

    fn run_to(&mut self, pc: u32) {
        self.machine.dbg.set_run_to(Some(pc & !1));
    }

    fn begin_step(&mut self, kind: StepKind, source: bool) -> StepPlan {
        let use_lines = source && !self.machine.dbg.lines.is_empty();
        let start = self.current_line_key();
        let trap_base = self.machine.dbg.trap_depth;
        let cond = match (kind, use_lines) {
            (StepKind::Into, false) => return StepPlan::Single,
            (StepKind::Over, false) => {
                let pc = self.machine.cpu.pc;
                let Some(insn) = self.machine.insn_at(pc) else { return StepPlan::Single };
                if !is_call(&insn) {
                    return StepPlan::Single;
                }
                StepCond::OverCall { ret: pc.wrapping_add(insn.len as u32) }
            }
            (StepKind::Out, _) => {
                if self.call_stack().is_empty() {
                    let c = self.machine.cpu.cycles;
                    self.machine.bus.cx.sys.warn_key(c, "stepout-empty", "Step Out: not inside a function call (call stack empty)");
                    return StepPlan::Refused;
                }
                StepCond::Out { lines: use_lines }
            }
            (StepKind::Into, true) => StepCond::IntoSrc { start },
            (StepKind::Over, true) => StepCond::OverSrc { start },
        };
        self.machine.dbg.step = Some(Stepper { cond, depth: 0, trap_base });
        self.machine.dbg.skip_pc = Some(self.machine.cpu.pc);
        StepPlan::Run
    }

    fn clear_stop_condition(&mut self) {
        self.machine.dbg.step = None;
        self.machine.dbg.set_run_to(None);
    }

    fn set_breakpoints(&mut self, pcs: &[u32]) {
        self.machine.dbg.set_breakpoints(pcs.iter().map(|&p| p & !1).collect());
    }

    fn set_pin_input(&mut self, pin: usize, ext: ExtDrive, volts: f64) {
        let now = self.machine.cpu.cycles;
        let sys = &mut self.machine.bus.cx.sys;
        if pin >= sys.pins.len() {
            return;
        }
        if sys.pins[pin].gen.is_some() {
            self.set_pin_generator(pin, None);
        }
        let sys = &mut self.machine.bus.cx.sys;
        sys.pins[pin].ext = ext;
        sys.pins[pin].ext_volts = volts;
        sys.update_pin(pin, now);
        self.after_io();
    }

    fn set_pin_generator(&mut self, pin: usize, gen: Option<PinGenerator>) {
        let now = self.machine.cpu.cycles;
        let b = &mut self.machine.bus;
        b.cx.owner = STIM_OWNER;
        b.cx.cycles = now;
        b.stim.set(pin, gen, &mut b.cx);
        self.after_io();
    }

    fn set_vcc(&mut self, volts: f64) {
        let now = self.machine.cpu.cycles;
        let sys = &mut self.machine.bus.cx.sys;
        sys.vcc = volts;
        for i in 0..sys.pins.len() {
            sys.update_pin(i, now);
        }
        self.after_io();
    }

    /// The crystal is fixed at 40 MHz on the ESP32-C3.
    fn set_external_clock(&mut self, _hz: f64) {}

    /// Per-instruction execution counting is not implemented for RISC-V (the chip view heat map is AVR-only).
    fn set_profiling(&mut self, _enabled: bool) {}

    fn watch_ram(&mut self, index: usize) -> Result<(), String> {
        if index > self.spec.extra_ram.len() {
            return Err(format!("RAM block {index} does not exist"));
        }
        self.extra_sel = index;
        self.extra_dirty = true;
        self.extra_sent = Vec::new();
        Ok(())
    }

    fn set_serial(&mut self, config: SerialConfig) {
        self.set_serial_config(config);
    }

    fn serial_send(&mut self, bytes: &[u8]) {
        let now = self.machine.cpu.cycles;
        let b = &mut self.machine.bus;
        b.cx.owner = BRIDGE_OWNER;
        b.cx.cycles = now;
        b.bridge.send(bytes, &mut b.cx);
        self.after_io();
    }

    fn write_data(&mut self, addr: u32, value: u8) -> Result<(), String> {
        if self.mem_write(addr, 1, value as u32) {
            Ok(())
        } else {
            Err(format!("Address 0x{addr:08X} is not writable"))
        }
    }

    fn write_mem(&mut self, addr: u32, size: u8, value: u32) -> Result<(), String> {
        if !matches!(size, 1 | 2 | 4) || addr & (size as u32 - 1) != 0 {
            return Err(format!("Unaligned or unsupported {size}-byte access at 0x{addr:08X}"));
        }
        if self.mem_write(addr, size as u32, value) {
            Ok(())
        } else {
            Err(format!("Address 0x{addr:08X} is not writable"))
        }
    }

    fn write_flash(&mut self, addr: u32, value: u8) -> Result<(), String> {
        let Some(off) = self.flash_offset(addr) else { return Err(format!("Address 0x{addr:08X} is outside the flash windows")) };
        self.machine.bus.mem_data_mut(self.flash)[off as usize] = value;
        self.machine.bus.flush_code();
        Ok(())
    }

    fn write_reg(&mut self, reg: usize, value: u32) -> Result<(), String> {
        match reg {
            0 => Err("x0 is hard-wired to zero".into()),
            1..=31 => {
                self.machine.cpu.x[reg] = value;
                Ok(())
            }
            _ => Err(format!("There is no register x{reg}")),
        }
    }

    fn write_cpu(&mut self, field: CpuField, value: u32) -> Result<(), String> {
        let m = &mut self.machine;
        match field {
            CpuField::Pc => m.set_pc(value),
            CpuField::Sp => m.cpu.x[2] = value,
            CpuField::Lr => m.cpu.x[1] = value,
            CpuField::Mstatus => m.cpu.csr_write(0x300, value).then_some(()).ok_or("mstatus is not writable")?,
            CpuField::Mie => {
                m.cpu.csr_write(0x304, value);
            }
            CpuField::Mtvec => {
                m.cpu.csr_write(0x305, value);
            }
            CpuField::Mepc => {
                m.cpu.csr_write(0x341, value);
            }
            CpuField::Mcause => {
                m.cpu.csr_write(0x342, value);
            }
            CpuField::Mtval => {
                m.cpu.csr_write(0x343, value);
            }
            CpuField::Mscratch => {
                m.cpu.csr_write(0x340, value);
            }
            _ => return Err("That register does not exist on RISC-V".into()),
        }
        m.refresh_irq_state();
        Ok(())
    }

    fn snapshot(&mut self, sent: &mut Sent, include_flash: bool, max_trace: usize) -> MachineState {
        let spec = self.spec;
        let first = sent.eeprom == u64::MAX;
        sent.eeprom = 0;
        let now = self.machine.cpu.cycles;
        // SRAM1 as seen on the data bus.
        let sram = &self.machine.bus.mem_data(self.sram)[0x4000..0x4000 + spec.sram_size as usize];
        let data = if first || self.ram_sent != sram {
            self.ram_sent.clear();
            self.ram_sent.extend_from_slice(sram);
            sram.to_vec()
        } else {
            Vec::new()
        };
        let ram_extra = match self.extra_sel {
            0 => None,
            k => {
                let cur: Vec<u8> = if k == 1 { self.machine.bus.mem_data(self.sram)[..0x4000].to_vec() } else { self.machine.bus.mem_data(self.rtc).to_vec() };
                if first || self.extra_dirty || self.extra_sent != cur {
                    self.extra_dirty = false;
                    self.extra_sent.clone_from(&cur);
                    Some(RamExtra { index: k, data: cur })
                } else {
                    None
                }
            }
        };
        let io: Vec<u32> = spec.registers.iter().map(|r| self.machine.bus.peek_register(r.addr, now).unwrap_or(0)).collect();
        let call_stack = self.call_stack();
        let peripherals = self.machine.bus.inspect_all(now).into_iter().map(|(name, values)| PeripheralInfo { name, values }).collect();
        let flash = include_flash.then(|| self.machine.bus.mem_data(self.flash).to_vec());
        let m = &mut self.machine;
        let sys = &mut m.bus.cx.sys;
        let (trace_from, trace_cycles, trace_levels) = sys.trace.read_since_wide(sent.trace, max_trace);
        let trace_words = sys.trace.words() as u32;
        sent.trace = sys.trace.seq;
        let serial = std::mem::take(&mut sys.serial_out);
        let pins = sys
            .pins
            .iter()
            .map(|p| PinState {
                level: p.level,
                dir: p.effective_dir(),
                out: p.out,
                pullup: p.pullup,
                pulldown: p.pulldown,
                ov_enable: p.ov_enable,
                ext: p.ext,
                ext_volts: p.ext_volts,
                volts: p.volts,
                reserved: p.reserved,
                reserved_by: p.reserved_by,
                gen: p.gen,
            })
            .collect();
        let (hz, vcc, messages) = (sys.clock.hz, sys.vcc, std::mem::take(&mut sys.messages));
        let time_sec = sys.time_at(now);
        let xtal = sys.clk.xtal.as_f64();
        let serial_config = m.bus.bridge.config();
        let c = &m.cpu;
        let core = CoreState::Riscv {
            x: c.x,
            pc: c.pc,
            mstatus: c.csr_read(0x300).unwrap_or(0),
            mie: c.csr.mie,
            mip: c.csr.mip,
            mtvec: c.csr.mtvec,
            mepc: c.csr.mepc,
            mcause: c.csr.mcause,
            mtval: c.csr.mtval,
            mscratch: c.csr.mscratch,
        };
        MachineState {
            running: false,
            pc: c.pc,
            pc_bytes: c.pc as u64,
            core,
            cycles: c.cycles,
            instructions: c.instret,
            time_sec,
            hz,
            ext_clock_hz: xtal,
            sleeping: m.is_sleeping(),
            sleep_mode: 0,
            reset_held: false,
            data,
            ram_extra,
            io,
            flash,
            flash_version: 0,
            fuses: Vec::new(),
            eeprom: None,
            serial,
            serial_config,
            lock: 0,
            pins,
            vcc,
            call_stack,
            peripherals,
            speed_hz: 0.0,
            trace_from,
            trace_cycles,
            trace_words,
            trace_levels,
            exec_heat: Vec::new(),
            messages,
            stop: None,
        }
    }
}
