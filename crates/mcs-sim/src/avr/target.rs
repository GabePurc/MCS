//! [`Target`] implementation for the AVR machine: the architecture-specific parts of the
//! debugger session (step predicates, source-line map, debugger writes, state snapshots).

use std::any::Any;

use mcs_core::avr::isa::op;
use mcs_core::device::DeviceRef;
use mcs_core::program::LoadedProgram;

use super::peripherals::serial::SerialConfig;
use super::{Machine, ResetSource, StopReason};
use crate::pins::{ExtDrive, PinGenerator};
use crate::protocol::{CoreState, CpuField, MachineState, PeripheralInfo, PinState, StepKind};
use crate::target::{Sent, StepPlan, Target};

impl Machine {
    /// Line key at (or the closest statement start before) the current PC.
    fn current_line_key(&self) -> i32 {
        let pc = self.cpu.pc as usize;
        (0..=pc.min(self.line_key.len().saturating_sub(1))).rev().map(|w| self.line_key[w]).find(|&k| k != -1).unwrap_or(-1)
    }
}

impl Target for Machine {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn device(&self) -> DeviceRef {
        DeviceRef::Avr(self.spec)
    }

    fn load_program(&mut self, program: Option<&LoadedProgram>) {
        match program {
            Some(p) => self.load(p),
            None => self.power_on(),
        }
    }

    fn debugger_reset(&mut self) {
        self.reset(ResetSource::Debugger);
    }

    fn power_cycle(&mut self) {
        self.power_on();
    }

    fn set_source_map(&mut self, program: Option<&LoadedProgram>) {
        let words = self.cpu.pc_mask as usize + 1;
        let mut key = vec![-1i32; words];
        if let Some(p) = program {
            // Several statement rows can share an address (`for(;;)` + its first statement,
            // or a call site + inlined header code). Use the file of the first one and the
            // last row from that file: the most specific statement in the user's file.
            for row in p.lines.iter().filter(|r| r.is_stmt) {
                let w = (row.address >> 1) as usize;
                if w >= words {
                    continue;
                }
                let k = ((row.file as i32) << 20) | (row.line as i32 & 0xfffff);
                if key[w] == -1 || key[w] >> 20 == k >> 20 {
                    key[w] = k;
                }
            }
        }
        self.line_key = key;
        self.has_lines = program.is_some_and(|p| !p.lines.is_empty());
    }

    fn cycles(&self) -> u64 {
        self.cpu.cycles
    }

    fn elapsed_seconds(&self) -> f64 {
        self.time_seconds()
    }

    fn cycle_at(&self, seconds: f64) -> u64 {
        self.sys.clock.cycle_at(seconds)
    }

    fn pc(&self) -> u32 {
        self.cpu.pc
    }

    fn run_until(&mut self, limit: u64) -> StopReason {
        self.run(limit)
    }

    fn step_one(&mut self) -> StopReason {
        self.step()
    }

    fn run_to(&mut self, pc: u32) {
        self.step_predicate = Some(Box::new(move |cpu| cpu.pc == pc));
    }

    fn begin_step(&mut self, kind: StepKind, source: bool) -> StepPlan {
        let use_lines = source && self.has_lines;
        let start_key = self.current_line_key();
        let line_key = self.line_key.clone();
        let depth0 = self.cpu.shadow_stack.len();
        match (kind, use_lines) {
            (StepKind::Into, false) => return StepPlan::Single,
            (StepKind::Over, false) => {
                let pc = self.cpu.pc;
                let o = self.cpu.op_at(pc);
                if !matches!(o, op::RCALL | op::ICALL | op::CALL | op::EICALL) {
                    return StepPlan::Single;
                }
                let ret = (pc + self.cpu.insn_words_at(pc)) & self.cpu.pc_mask;
                self.step_predicate = Some(Box::new(move |cpu| cpu.pc == ret && cpu.shadow_stack.len() <= depth0));
            }
            (StepKind::Out, _) => {
                if depth0 == 0 {
                    let c = self.cpu.cycles;
                    self.sys.warn_key(c, "stepout-empty", "Step Out: not inside a function call (call stack empty)");
                    return StepPlan::Refused;
                }
                self.step_predicate = Some(if use_lines {
                    Box::new(move |cpu| cpu.shadow_stack.len() < depth0 && line_key[cpu.pc as usize] != -1)
                } else {
                    Box::new(move |cpu| cpu.shadow_stack.len() < depth0)
                });
            }
            (StepKind::Into, true) => {
                self.step_predicate = Some(Box::new(move |cpu| {
                    let k = line_key[cpu.pc as usize];
                    k != -1 && (k != start_key || cpu.shadow_stack.len() != depth0)
                }));
            }
            (StepKind::Over, true) => {
                // Source-level step over:
                // * without a source context (e.g. at the reset vector, before the C runtime
                //   calls main) stop at the first line with debug info, at any call depth;
                // * skip called functions (deeper frames) and stop when the current one returns;
                // * stay in the current file, so code inlined from headers (e.g. _delay_ms)
                //   is stepped over as part of its call-site line.
                self.step_predicate = Some(Box::new(move |cpu| {
                    let k = line_key[cpu.pc as usize];
                    if k == -1 {
                        return false;
                    }
                    if start_key == -1 {
                        return true;
                    }
                    let d = cpu.shadow_stack.len();
                    if d != depth0 {
                        return d < depth0;
                    }
                    (k >> 20) == (start_key >> 20) && k != start_key
                }));
            }
        }
        StepPlan::Run
    }

    fn clear_stop_condition(&mut self) {
        self.step_predicate = None;
    }

    fn set_breakpoints(&mut self, pcs: &[u32]) {
        self.cpu.breakpoints.fill(false);
        for &pc in pcs {
            if let Some(b) = self.cpu.breakpoints.get_mut(pc as usize) {
                *b = true;
            }
        }
    }

    fn set_pin_input(&mut self, pin: usize, ext: ExtDrive, volts: f64) {
        Machine::set_pin_input(self, pin, ext, volts);
    }

    fn set_pin_generator(&mut self, pin: usize, gen: Option<PinGenerator>) {
        Machine::set_pin_generator(self, pin, gen);
    }

    fn set_vcc(&mut self, volts: f64) {
        Machine::set_vcc(self, volts);
    }

    fn set_external_clock(&mut self, hz: f64) {
        Machine::set_external_clock(self, hz);
    }

    fn set_profiling(&mut self, enabled: bool) {
        Machine::set_profiling(self, enabled);
    }

    fn set_serial(&mut self, config: SerialConfig) {
        Machine::set_serial(self, config);
    }

    fn serial_send(&mut self, bytes: &[u8]) {
        Machine::serial_send(self, bytes);
    }

    fn write_data(&mut self, addr: u32, value: u8) -> Result<(), String> {
        self.poke_data(addr as u16, value);
        Ok(())
    }

    fn write_flash(&mut self, addr: u32, value: u8) -> Result<(), String> {
        self.cpu.write_flash_byte(addr, value);
        Ok(())
    }

    fn write_reg(&mut self, reg: usize, value: u32) -> Result<(), String> {
        self.cpu.r[reg & 31] = value as u8;
        Ok(())
    }

    fn write_cpu(&mut self, field: CpuField, value: u32) -> Result<(), String> {
        match field {
            CpuField::Pc => self.cpu.pc = (value >> 1) & self.cpu.pc_mask,
            CpuField::Sp => self.cpu.sp = value as u16,
            CpuField::Sreg => self.cpu.sreg = value as u8,
            CpuField::Xpsr | CpuField::Msp | CpuField::Psp | CpuField::Lr => return Err("This CPU field does not exist on AVR".into()),
        }
        Ok(())
    }

    fn write_eeprom(&mut self, addr: u32, value: u8) -> Result<(), String> {
        if let Some(c) = self.cpu.eeprom.get_mut(addr as usize) {
            *c = value;
            self.cpu.eeprom_version += 1;
        }
        Ok(())
    }

    fn write_fuse(&mut self, index: usize, value: u8) -> Result<(), String> {
        if let Some(f) = self.cpu.fuses.get_mut(index) {
            *f = value;
        }
        self.power_on();
        Ok(())
    }

    fn set_clock_config(&mut self, source: u8, prescale_log2: u8) -> Result<(), String> {
        self.debug_set_clock(source, prescale_log2);
        Ok(())
    }

    fn snapshot(&mut self, sent: &mut Sent, include_flash: bool, max_trace: usize) -> MachineState {
        let m = self;
        let data: Vec<u8> = (0..m.cpu.data_end).map(|a| m.peek_data(a)).collect();
        let (trace_from, trace_cycles, trace_levels) = m.sys.trace.read_since_wide(sent.trace, max_trace);
        let trace_words = m.sys.trace.words() as u32;
        sent.trace = m.sys.trace.seq;
        let flash = include_flash.then(|| m.cpu.flash.clone());
        let eeprom_version = m.cpu.eeprom_version;
        let eeprom = (sent.eeprom != eeprom_version).then(|| m.cpu.eeprom.clone());
        sent.eeprom = eeprom_version;
        let serial = std::mem::take(&mut m.sys.serial_out);
        let serial_config = m.serial_config();
        let pins = m
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
        let peripherals = m.inspect_peripherals().into_iter().map(|(name, values)| PeripheralInfo { name, values }).collect();
        let stack = &m.cpu.shadow_stack;
        MachineState {
            running: false,
            pc: m.cpu.pc,
            pc_bytes: m.cpu.pc as u64 * 2,
            core: CoreState::Avr { sp: m.cpu.sp, sreg: m.cpu.sreg, regs: m.cpu.r.to_vec() },
            cycles: m.cpu.cycles,
            instructions: m.cpu.instructions,
            time_sec: m.time_seconds(),
            hz: m.sys.clock.hz,
            ext_clock_hz: m.sys.ext_clock_hz,
            sleeping: m.cpu.sleeping,
            sleep_mode: m.cpu.sleep_mode,
            reset_held: m.sys.reset_held,
            data,
            io: Vec::new(),
            flash,
            flash_version: 0,
            fuses: m.cpu.fuses.clone(),
            eeprom,
            serial,
            serial_config,
            lock: m.cpu.lock_bits,
            pins,
            vcc: m.sys.vcc,
            call_stack: stack[stack.len().saturating_sub(64)..].to_vec(),
            peripherals,
            speed_hz: 0.0,
            trace_from,
            trace_cycles,
            trace_words,
            trace_levels,
            exec_heat: m.take_exec_counts(),
            messages: m.messages(),
            stop: None,
        }
    }
}
