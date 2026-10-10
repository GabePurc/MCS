//! [`Target`] implementation for the ARM machine: debugger session support (program loading,
//! stepping, breakpoints, writes, state snapshots) on top of [`Machine`].
//!
//! Position values (`pc`, breakpoints, run-to) are byte addresses on this architecture.

use std::any::Any;

use mcs_core::arm::thumb::{self, Op};
use mcs_core::device::DeviceRef;
use mcs_core::program::LoadedProgram;

use super::debug::{is_call, StepCond, Stepper};
use super::machine::{cx, Machine, BRIDGE_OWNER};
use super::{StopReason as ArmStop, systick};
use crate::avr::peripherals::serial::SerialConfig;
use crate::avr::CallFrame;
use crate::pins::{ExtDrive, PinGenerator};
use crate::protocol::{CoreState, CpuField, MachineState, PeripheralInfo, PinState, StepKind};
use crate::target::{Sent, StepPlan, StopReason, Target};

/// Words of stack scanned when reconstructing the call stack (from SP up, bounded by the stack's
/// own memory: reads outside RAM end the scan).
const UNWIND_WORDS: u32 = 512;
/// Frames reported to the UI.
const MAX_FRAMES: usize = 64;

impl Machine {
    fn spec_ref(&self) -> &'static mcs_core::arm::device::ArmDeviceSpec {
        self.spec.expect("ARM machine built from a device description")
    }

    /// Statement key at (or the closest statement start before) the current PC.
    fn current_line_key(&self) -> i32 {
        let idx = (self.cpu.pc.wrapping_sub(self.bus.flash_base) >> 1) as usize;
        let n = self.dbg.line_key.len();
        if n == 0 {
            return -1;
        }
        (0..=idx.min(n - 1)).rev().map(|i| self.dbg.line_key[i]).find(|&k| k != -1).unwrap_or(-1)
    }

    /// True when the halfword(s) before `ret` encode a BL / BLX, i.e. `ret` is a return address.
    fn is_return_address(&self, ret: u32) -> bool {
        if ret & 1 == 0 || ret < 4 {
            return false;
        }
        let ret = ret & !1;
        let flash_off = ret.wrapping_sub(self.bus.flash_base);
        if flash_off >= self.bus.flash_size || flash_off < 4 {
            return false;
        }
        let hw = |a: u32| self.bus.read_mem(a, 2).unwrap_or(0);
        let (h1, h2) = (hw(ret - 4), hw(ret - 2));
        // BL: 11110 S imm10 / 11 J1 1 J2 imm11;  BLX register: 010001111 Rm 000.
        (h1 & 0xf800 == 0xf000 && h2 & 0xd000 == 0xd000) || h2 & 0xff87 == 0x4780
    }

    /// Call target of the BL ending at `ret` (0 for BLX register, which has no static target).
    fn call_target(&self, ret: u32) -> u32 {
        let ret = ret & !1;
        let hw = |a: u32| self.bus.read_mem(a, 2).unwrap_or(0) as u16;
        let (h1, h2) = (hw(ret - 4), hw(ret - 2));
        if h1 & 0xf800 == 0xf000 && h2 & 0xd000 == 0xd000 {
            let i = thumb::decode(h1, h2, self.cfg.features);
            if i.op == Op::BL {
                return (ret - 4).wrapping_add(4).wrapping_add(i.imm);
            }
        }
        0
    }

    /// Call stack (innermost last): exception frames come from the exact record kept at exception
    /// entry; regular calls are reconstructed from the live stack by looking for stacked values that
    /// are return addresses (they follow a BL / BLX). The latter is a heuristic -- stale words below
    /// the live frames can show up as extra frames -- but costs nothing while running and survives
    /// RTOS context switches.
    pub fn call_stack(&mut self) -> Vec<CallFrame> {
        let sp = self.cpu.r[13];
        let end = sp.wrapping_add(4 * UNWIND_WORDS);
        let mut frames: Vec<CallFrame> = Vec::new();
        // Exception frames, innermost first.
        let mut exc: Vec<(u16, u32)> = self.exc_stack.iter().rev().copied().collect();
        exc.retain(|&(_, f)| f >= sp & !3);
        let mut a = sp & !3;
        let mut first_ret: Option<u32> = None;
        while a < end && frames.len() < MAX_FRAMES {
            if let Some(&(vector, frame)) = exc.first().filter(|&&(_, f)| f == a) {
                exc.remove(0);
                let pc = self.bus.read_mem(frame + 24, 4).unwrap_or(0);
                let handler = self.bus.read_mem(self.scb.vtor.wrapping_add(4 * vector as u32), 4).unwrap_or(0) & !1;
                frames.push(CallFrame { return_pc: pc & !1, target_pc: handler, vector: vector as i16, sp: frame });
                a += 32;
                continue;
            }
            let Some(w) = self.bus.read_mem(a, 4) else { break };
            if self.is_return_address(w) {
                if first_ret.is_none() {
                    first_ret = Some(w);
                }
                frames.push(CallFrame { return_pc: w & !1, target_pc: self.call_target(w), vector: -1, sp: a + 4 });
            }
            a += 4;
        }
        // A leaf function (or the first instructions of a call) still has the return address in LR.
        let lr = self.cpu.r[14];
        if self.cpu.ipsr == 0 && self.is_return_address(lr) && first_ret != Some(lr) && !frames.iter().take(2).any(|f| f.return_pc == lr & !1) {
            frames.push(CallFrame { return_pc: lr & !1, target_pc: self.call_target(lr), vector: -1, sp });
        }
        frames.reverse();
        frames
    }

    /// Value of the register at `addr` for the debugger views (no side effects).
    fn peek_register(&mut self, addr: u32) -> u32 {
        if addr & 0xfff0_0000 == 0xe000_0000 {
            if addr == 0xe000_e010 {
                let s = &self.systick;
                return s.enabled as u32 | (s.tickint as u32) << 1 | (s.clksource as u32) << 2 | (s.countflag as u32) << 16;
            }
            return self.ppb_read(addr, 4).unwrap_or(0);
        }
        let Some((dev, off)) = self.bus.find(addr) else { return 0 };
        let cycles = self.cpu.cycles;
        let mut cx = cx!(self, dev as u8, cycles);
        self.bus.devs[dev].peek(off, &mut cx)
    }
}

impl Target for Machine {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn device(&self) -> DeviceRef {
        DeviceRef::Arm(self.spec_ref())
    }

    fn load_program(&mut self, program: Option<&LoadedProgram>) {
        if let Some(p) = program {
            let used = (p.flash_used as usize).min(p.flash.len()).min(self.bus.flash.len());
            self.bus.flash.fill(0xff);
            self.bus.flash[..used].copy_from_slice(&p.flash[..used]);
            let halfwords = self.prog_len();
            self.decode_range(0, halfwords);
        }
        self.reset();
    }

    fn debugger_reset(&mut self) {
        self.reset_with(false);
    }

    fn power_cycle(&mut self) {
        self.reset();
    }

    fn set_source_map(&mut self, program: Option<&LoadedProgram>) {
        let halfwords = (self.bus.flash_size / 2) as usize;
        let mut key = vec![-1i32; halfwords];
        let base = self.bus.flash_base;
        if let Some(p) = program {
            // Several statement rows can share an address; use the file of the first one and the
            // last row from that file (the most specific statement in the user's file).
            for row in p.lines.iter().filter(|r| r.is_stmt) {
                let w = (row.address.wrapping_sub(base) >> 1) as usize;
                if w >= halfwords {
                    continue;
                }
                let k = ((row.file as i32) << 20) | (row.line as i32 & 0xfffff);
                if key[w] == -1 || key[w] >> 20 == k >> 20 {
                    key[w] = k;
                }
            }
        }
        self.dbg.line_key = key;
        self.dbg.has_lines = program.is_some_and(|p| !p.lines.is_empty());
    }

    fn cycles(&self) -> u64 {
        self.cpu.cycles
    }

    fn elapsed_seconds(&self) -> f64 {
        self.sys.time_at(self.cpu.cycles)
    }

    fn cycle_at(&self, seconds: f64) -> u64 {
        self.sys.clock.cycle_at(seconds)
    }

    fn pc(&self) -> u32 {
        self.cpu.pc
    }

    fn run_until(&mut self, limit: u64) -> StopReason {
        match self.run(limit) {
            ArmStop::Limit | ArmStop::None => StopReason::Limit,
            ArmStop::Bkpt => StopReason::BreakInsn,
            ArmStop::Lockup => StopReason::Lockup,
            ArmStop::Requested => {
                if self.dbg.hit_breakpoint {
                    StopReason::Breakpoint
                } else {
                    StopReason::Requested
                }
            }
        }
    }

    fn step_one(&mut self) -> StopReason {
        // A sleeping core first runs on to the event that wakes it.
        for _ in 0..64 {
            if !self.cpu.sleeping || self.sched.next == u64::MAX {
                break;
            }
            let t = self.sched.next;
            self.run(t);
        }
        match self.step() {
            ArmStop::Bkpt => StopReason::BreakInsn,
            ArmStop::Lockup => StopReason::Lockup,
            _ => StopReason::Limit,
        }
    }

    fn run_to(&mut self, pc: u32) {
        self.dbg.run_to = Some(pc & !1);
    }

    fn begin_step(&mut self, kind: StepKind, source: bool) -> StepPlan {
        let use_lines = source && self.dbg.has_lines;
        let start = self.current_line_key();
        let ipsr = self.cpu.ipsr;
        let cond = match (kind, use_lines) {
            (StepKind::Into, false) => return StepPlan::Single,
            (StepKind::Over, false) => {
                let pc = self.cpu.pc;
                let Some(insn) = self.peek_insn(pc) else { return StepPlan::Single };
                if !is_call(&insn) {
                    return StepPlan::Single;
                }
                StepCond::OverCall { ret: pc.wrapping_add(insn.len as u32) }
            }
            (StepKind::Out, _) => {
                if ipsr == 0 && self.call_stack().is_empty() {
                    let c = self.cpu.cycles;
                    self.sys.warn_key(c, "stepout-empty", "Step Out: not inside a function call (call stack empty)");
                    return StepPlan::Refused;
                }
                StepCond::Out { lines: use_lines }
            }
            (StepKind::Into, true) => StepCond::IntoSrc { start },
            (StepKind::Over, true) => StepCond::OverSrc { start },
        };
        self.dbg.step = Some(Stepper { cond, depth: 0, ipsr });
        StepPlan::Run
    }

    fn clear_stop_condition(&mut self) {
        self.dbg.step = None;
        self.dbg.run_to = None;
    }

    fn set_breakpoints(&mut self, pcs: &[u32]) {
        let halfwords = (self.bus.flash_size / 2) as usize;
        let mut bp = Vec::new();
        let mut n = 0;
        for &pc in pcs {
            let w = (pc.wrapping_sub(self.bus.flash_base) >> 1) as usize;
            if w < halfwords {
                if bp.is_empty() {
                    bp = vec![false; halfwords];
                }
                if !bp[w] {
                    bp[w] = true;
                    n += 1;
                }
            }
        }
        self.dbg.bp = bp;
        self.dbg.nbp = n;
    }

    fn set_pin_input(&mut self, pin: usize, ext: ExtDrive, volts: f64) {
        if pin >= self.sys.pins.len() {
            return;
        }
        let now = self.cpu.cycles;
        if self.sys.pins[pin].gen.is_some() {
            self.set_pin_generator(pin, None);
        }
        self.sys.pins[pin].ext = ext;
        self.sys.pins[pin].ext_volts = volts;
        self.sys.update_pin(pin, now);
        self.after_io();
    }

    fn set_pin_generator(&mut self, pin: usize, gen: Option<PinGenerator>) {
        let now = self.cpu.cycles;
        let mut cx = cx!(self, super::machine::STIM_OWNER, now);
        self.stim.set(pin, gen, &mut cx);
        self.after_io();
    }

    fn set_vcc(&mut self, volts: f64) {
        self.sys.vcc = volts;
        let now = self.cpu.cycles;
        for i in 0..self.sys.pins.len() {
            self.sys.update_pin(i, now);
        }
        self.after_io();
    }

    fn set_external_clock(&mut self, hz: f64) {
        if !(hz.is_finite() && hz > 0.0) {
            return;
        }
        self.sys.hse_hz = hz;
        self.sys.clock_dirty = true;
        self.after_io();
    }

    /// Per-instruction execution counting is not implemented for ARM (the chip view heat map is
    /// AVR-only for now).
    fn set_profiling(&mut self, _enabled: bool) {}

    fn set_serial(&mut self, config: SerialConfig) {
        let now = self.cpu.cycles;
        let mut cx = cx!(self, BRIDGE_OWNER, now);
        self.bridge.configure(config, &mut cx);
        self.after_io();
    }

    fn serial_send(&mut self, bytes: &[u8]) {
        let now = self.cpu.cycles;
        let mut cx = cx!(self, BRIDGE_OWNER, now);
        self.bridge.send(bytes, &mut cx);
        self.after_io();
    }

    fn write_data(&mut self, addr: u32, value: u8) -> Result<(), String> {
        if self.mem_write(addr, 1, value as u32) {
            Ok(())
        } else {
            Err(format!("Address 0x{addr:08X} is not writable"))
        }
    }

    fn write_flash(&mut self, addr: u32, value: u8) -> Result<(), String> {
        let off = if addr >= self.bus.flash_base { addr - self.bus.flash_base } else { addr };
        if off >= self.bus.flash_size {
            return Err(format!("Address 0x{addr:08X} is outside the flash"));
        }
        Machine::write_flash(self, off, &[value]);
        Ok(())
    }

    fn write_reg(&mut self, reg: usize, value: u32) -> Result<(), String> {
        match reg {
            0..=12 | 14 => self.cpu.r[reg] = value,
            13 => self.cpu.r[13] = value & !3,
            15 => self.cpu.pc = value & !1,
            _ => return Err(format!("There is no register r{reg}")),
        }
        Ok(())
    }

    fn write_cpu(&mut self, field: CpuField, value: u32) -> Result<(), String> {
        match field {
            CpuField::Pc => self.cpu.pc = value & !1,
            CpuField::Sp => self.cpu.r[13] = value & !3,
            CpuField::Xpsr => self.cpu.set_apsr(value),
            CpuField::Msp => self.cpu.set_msp(value),
            CpuField::Psp => self.cpu.set_psp(value),
            CpuField::Lr => self.cpu.r[14] = value,
            CpuField::Sreg => return Err("The ARM core has no SREG; use xPSR".into()),
        }
        Ok(())
    }

    fn snapshot(&mut self, sent: &mut Sent, include_flash: bool, max_trace: usize) -> MachineState {
        let spec = self.spec_ref();
        // SRAM image, only when it changed since the previous state (or after a load).
        let first = sent.eeprom == u64::MAX;
        sent.eeprom = 0;
        let ram = &self.bus.ram[0].data;
        let data = if first || self.dbg.ram_sent != *ram {
            self.dbg.ram_sent.clone_from(ram);
            ram.clone()
        } else {
            Vec::new()
        };
        let io: Vec<u32> = spec.registers.iter().map(|r| self.peek_register(r.addr)).collect();
        let (trace_from, trace_cycles, trace_levels) = self.sys.trace.read_since_wide(sent.trace, max_trace);
        let trace_words = self.sys.trace.words() as u32;
        sent.trace = self.sys.trace.seq;
        let flash = include_flash.then(|| self.bus.flash.clone());
        let serial = std::mem::take(&mut self.sys.serial_out);
        let serial_config = self.bridge.config();
        let pins = self
            .sys
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
        let call_stack = self.call_stack();
        let now = self.cpu.cycles;
        let mut peripherals = Vec::new();
        for d in 0..self.bus.devs.len() {
            let cx = cx!(self, d as u8, now);
            let values = self.bus.devs[d].inspect(&cx);
            if !values.is_empty() {
                peripherals.push(PeripheralInfo { name: self.dev_names[d].clone(), values });
            }
        }
        let c = &self.cpu;
        let mut r = c.r;
        r[15] = c.pc;
        let core = CoreState::Arm { r, xpsr: c.xpsr(), msp: c.msp(), psp: c.psp(), control: c.control, primask: c.primask, basepri: c.basepri, faultmask: c.faultmask };
        MachineState {
            running: false,
            pc: c.pc,
            pc_bytes: c.pc as u64,
            core,
            cycles: c.cycles,
            instructions: c.instructions,
            time_sec: self.sys.time_at(c.cycles),
            hz: self.sys.clock.hz,
            ext_clock_hz: self.sys.hse_hz,
            sleeping: c.sleeping,
            sleep_mode: 0,
            reset_held: false,
            data,
            io,
            flash,
            flash_version: 0,
            fuses: Vec::new(),
            eeprom: None,
            serial,
            serial_config,
            lock: 0,
            pins,
            vcc: self.sys.vcc,
            call_stack,
            peripherals,
            speed_hz: 0.0,
            trace_from,
            trace_cycles,
            trace_words,
            trace_levels,
            exec_heat: Vec::new(),
            messages: std::mem::take(&mut self.sys.messages),
            stop: None,
        }
    }
}

impl Machine {
    fn prog_len(&self) -> usize {
        self.bus.flash.len() / 2
    }

    /// Decoded instruction at `pc` (flash or RAM) without side effects.
    fn peek_insn(&self, pc: u32) -> Option<thumb::Insn> {
        let hw = |a: u32| self.bus.read_mem(a, 2).map(|v| v as u16);
        let h1 = hw(pc)?;
        let h2 = if thumb::is_32bit(h1) { hw(pc + 2)? } else { 0 };
        Some(thumb::decode(h1, h2, self.cfg.features))
    }
}

const _: () = {
    // SysTick keeps its COUNTFLAG semantics in `peek_register`.
    let _ = systick::CSR_COUNTFLAG;
};
