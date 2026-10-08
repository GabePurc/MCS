//! Debugger session: owns one machine, drives it in time slices (real-time or max speed),
//! implements stepping and produces state snapshots. Transport agnostic — [`spawn`] runs it on
//! a dedicated thread; tests drive it directly.

#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;
use std::time::Duration;

use crate::clock::now_ms;

use mcs_core::avr::devices;
use mcs_core::avr::isa::op;
use mcs_core::program::LoadedProgram;

use crate::avr::{Machine, ResetSource, StopReason};
use crate::protocol::*;

const SLICE_MS: f64 = 8.0;
const STATE_INTERVAL_MS: f64 = 33.0;
/// Max simulated seconds caught up in one real-time slice (prevents death spirals).
const MAX_CATCHUP_SEC: f64 = 0.25;
/// Max trace entries per state message (older ones are dropped while running fast).
const MAX_TRACE_PER_STATE: usize = 50_000;

pub struct Session {
    machine: Option<Machine>,
    program: Option<LoadedProgram>,
    running: bool,
    speed: SpeedMode,
    factor: f64,
    cycles_per_slice: u64,
    wall_start: f64,
    sim_start: f64,
    last_publish: f64,
    trace_sent: u64,
    flash_version: u64,
    flash_sent: u64,
    /// (file << 20 | line) per word address at statement starts, -1 elsewhere.
    line_key: Vec<i32>,
    breakpoints: Vec<u32>,
    pending_stop: Option<StopInfo>,
    run_to: Option<u32>,
    speed_sample: (f64, u64, f64),
    out: Vec<Output>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        let now = now_ms();
        Self {
            machine: None,
            program: None,
            running: false,
            speed: SpeedMode::Realtime,
            factor: 1.0,
            cycles_per_slice: 200_000,
            wall_start: now,
            sim_start: 0.0,
            last_publish: now,
            trace_sent: 0,
            flash_version: 0,
            flash_sent: u64::MAX,
            line_key: Vec::new(),
            breakpoints: Vec::new(),
            pending_stop: None,
            run_to: None,
            speed_sample: (now, 0, 0.0),
            out: Vec::new(),
        }
    }

    pub fn machine(&mut self) -> Option<&mut Machine> {
        self.machine.as_mut()
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Handles one command; returns the outputs to forward to the UI.
    pub fn handle(&mut self, cmd: Command) -> Vec<Output> {
        if let Err(e) = self.dispatch(cmd) {
            self.out.push(Output::Error { message: e });
        }
        std::mem::take(&mut self.out)
    }

    fn dispatch(&mut self, cmd: Command) -> Result<(), String> {
        match cmd {
            Command::Init { device_id } => return self.create_machine(&device_id, None),
            Command::Load { device_id, program } => return self.create_machine(&device_id, Some(*program)),
            Command::Shutdown => {
                self.running = false;
                return Ok(());
            }
            _ => {}
        }
        if self.machine.is_none() {
            return Ok(());
        }
        match cmd {
            Command::Run => self.start(),
            Command::Pause => {
                let pc = self.m().cpu.pc;
                self.stop(StopInfo { reason: StopKind::Pause, pc, message: None });
            }
            Command::Reset => {
                self.m().reset(ResetSource::Debugger);
                self.stop(StopInfo { reason: StopKind::Reset, pc: 0, message: None });
            }
            Command::PowerCycle => {
                self.m().power_on();
                self.trace_sent = 0;
                self.stop(StopInfo { reason: StopKind::Reset, pc: 0, message: None });
            }
            Command::Step { kind, source } => self.step(kind, source),
            Command::RunTo { pc } => {
                self.run_to = Some(pc);
                self.m().step_predicate = Some(Box::new(move |cpu| cpu.pc == pc));
                self.start();
            }
            Command::SetBreakpoints { pcs } => {
                self.breakpoints = pcs;
                self.apply_breakpoints();
            }
            Command::SetSpeed { mode, factor } => {
                self.speed = mode;
                self.factor = if factor > 0.0 { factor } else { 1.0 };
                self.resync_clock();
            }
            Command::SetPin { pin, ext, volts } => {
                self.m().set_pin_input(pin, ext, volts);
                self.publish_if_idle();
            }
            Command::SetVcc { volts } => {
                self.m().set_vcc(volts);
                self.publish_if_idle();
            }
            Command::SetExternalClock { hz } => self.m().sys.ext_clock_hz = hz,
            Command::WriteData { addr, value } => {
                self.m().poke_data(addr, value);
                self.publish_if_idle();
            }
            Command::WriteFlash { addr, value } => {
                self.m().cpu.write_flash_byte(addr, value);
                self.flash_version += 1;
                self.publish_if_idle();
            }
            Command::WriteReg { reg, value } => {
                self.m().cpu.r[reg & 31] = value;
                self.publish_if_idle();
            }
            Command::WriteCpu { field, value } => {
                let m = self.m();
                match field {
                    CpuField::Pc => m.cpu.pc = (value >> 1) & m.cpu.pc_mask,
                    CpuField::Sp => m.cpu.sp = value as u16,
                    CpuField::Sreg => m.cpu.sreg = value as u8,
                }
                self.publish_if_idle();
            }
            Command::WriteFuse { value } => {
                self.m().cpu.fuse = value;
                self.m().power_on();
                self.stop(StopInfo { reason: StopKind::Reset, pc: 0, message: None });
            }
            Command::RequestState => self.publish(),
            Command::Init { .. } | Command::Load { .. } | Command::Shutdown => unreachable!(),
        }
        Ok(())
    }

    fn m(&mut self) -> &mut Machine {
        self.machine.as_mut().expect("machine")
    }

    fn create_machine(&mut self, device_id: &str, program: Option<LoadedProgram>) -> Result<(), String> {
        let spec = devices::get(device_id).ok_or_else(|| format!("Unknown device '{device_id}'"))?;
        self.running = false;
        if self.machine.as_ref().map(|m| m.spec.id != spec.id).unwrap_or(true) {
            self.machine = Some(Machine::new(spec));
            self.out.push(Output::Device { spec: Box::new(spec.clone()) });
        }
        match &program {
            Some(p) => self.m().load(p),
            None => self.m().power_on(),
        }
        self.program = program;
        self.trace_sent = 0;
        self.flash_version += 1;
        self.build_line_map();
        self.apply_breakpoints();
        self.stop(StopInfo { reason: StopKind::Load, pc: 0, message: None });
        Ok(())
    }

    fn build_line_map(&mut self) {
        let words = self.m().cpu.flash_words as usize;
        let mut key = vec![-1i32; words];
        if let Some(p) = &self.program {
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
    }

    fn apply_breakpoints(&mut self) {
        let bps = self.breakpoints.clone();
        if let Some(m) = self.machine.as_mut() {
            m.cpu.breakpoints.fill(false);
            for pc in bps {
                if let Some(b) = m.cpu.breakpoints.get_mut(pc as usize) {
                    *b = true;
                }
            }
        }
    }

    // ---------------------------------------------------------------------------------
    // Run control
    // ---------------------------------------------------------------------------------

    fn start(&mut self) {
        if self.running {
            return;
        }
        self.running = true;
        self.resync_clock();
        let cycles = self.m().cpu.cycles;
        self.speed_sample = (now_ms(), cycles, 0.0);
        self.publish();
    }

    fn stop(&mut self, info: StopInfo) {
        self.running = false;
        if let Some(m) = self.machine.as_mut() {
            m.step_predicate = None;
        }
        self.run_to = None;
        self.pending_stop = Some(info);
        self.publish();
    }

    fn resync_clock(&mut self) {
        self.wall_start = now_ms();
        self.sim_start = self.machine.as_ref().map(|m| m.time_seconds()).unwrap_or(0.0);
    }

    /// Runs one time slice when running. Returns outputs (periodic state / stop events).
    pub fn slice(&mut self) -> Vec<Output> {
        if !self.running || self.machine.is_none() {
            return Vec::new();
        }
        let t0 = now_ms();
        let (speed, factor, cps) = (self.speed, self.factor, self.cycles_per_slice);
        let wall = (t0 - self.wall_start) / 1000.0;
        let mut sim_start = self.sim_start;
        let m = self.m();
        let target = match speed {
            SpeedMode::Max => m.cpu.cycles + cps,
            SpeedMode::Realtime => {
                let sim_target = sim_start + wall * factor;
                let max_target = m.time_seconds() + MAX_CATCHUP_SEC * factor;
                if sim_target > max_target {
                    // Fell behind (slow host): drop the backlog instead of spiralling.
                    sim_start = max_target - wall * factor;
                }
                m.sys.clock.cycle_at(sim_target.min(max_target))
            }
        };
        let reason = if target > m.cpu.cycles { m.run(target) } else { StopReason::Limit };
        self.sim_start = sim_start;
        let elapsed = now_ms() - t0;
        if speed == SpeedMode::Max {
            let e = elapsed.max(0.05);
            let scaled = self.cycles_per_slice as f64 * (SLICE_MS / e);
            self.cycles_per_slice = scaled.clamp(1_000.0, 2e9) as u64;
        }
        if reason != StopReason::Limit {
            let info = self.stop_info(reason);
            self.stop(info);
        } else if now_ms() - self.last_publish >= STATE_INTERVAL_MS {
            self.publish();
        }
        std::mem::take(&mut self.out)
    }

    /// How long the driver may wait before the next slice (real-time pacing).
    pub fn idle_time(&self) -> Duration {
        Duration::from_secs_f64(self.idle_ms() / 1000.0)
    }

    /// Milliseconds the driver may wait before the next slice (0 = immediately).
    pub fn idle_ms(&self) -> f64 {
        match (self.running, self.speed) {
            (true, SpeedMode::Max) => 0.0,
            (true, SpeedMode::Realtime) => SLICE_MS / 2.0,
            _ => 3_600_000.0,
        }
    }

    fn stop_info(&mut self, reason: StopReason) -> StopInfo {
        let pc = self.m().cpu.pc;
        match reason {
            StopReason::Breakpoint => StopInfo { reason: StopKind::Breakpoint, pc, message: None },
            StopReason::BreakInsn => StopInfo { reason: StopKind::Break, pc, message: Some("BREAK instruction executed".into()) },
            StopReason::InvalidOpcode => StopInfo { reason: StopKind::Invalid, pc, message: Some("Invalid opcode".into()) },
            _ => StopInfo { reason: if self.run_to.is_some() { StopKind::RunTo } else { StopKind::Step }, pc, message: None },
        }
    }

    // ---------------------------------------------------------------------------------
    // Stepping
    // ---------------------------------------------------------------------------------

    fn step(&mut self, kind: StepKind, source: bool) {
        if self.running {
            return;
        }
        let use_lines = source && self.program.as_ref().is_some_and(|p| !p.lines.is_empty());
        let start_key = self.current_line_key();
        let line_key = self.line_key.clone();
        let m = self.m();
        let depth0 = m.cpu.shadow_stack.len();

        let single = |s: &mut Self| {
            let r = s.m().step();
            let info = if r == StopReason::Limit {
                let pc = s.m().cpu.pc;
                StopInfo { reason: StopKind::Step, pc, message: None }
            } else {
                s.stop_info(r)
            };
            s.stop(info);
        };

        match (kind, use_lines) {
            (StepKind::Into, false) => return single(self),
            (StepKind::Over, false) => {
                let pc = m.cpu.pc;
                let o = m.cpu.op_at(pc);
                if !matches!(o, op::RCALL | op::ICALL | op::CALL | op::EICALL) {
                    return single(self);
                }
                let ret = (pc + m.cpu.insn_words_at(pc)) & m.cpu.pc_mask;
                m.step_predicate = Some(Box::new(move |cpu| cpu.pc == ret && cpu.shadow_stack.len() <= depth0));
            }
            (StepKind::Out, _) => {
                if depth0 == 0 {
                    let c = m.cpu.cycles;
                    m.sys.warn_key(c, "stepout-empty", "Step Out: not inside a function call (call stack empty)");
                    self.publish();
                    return;
                }
                m.step_predicate = Some(if use_lines {
                    Box::new(move |cpu| cpu.shadow_stack.len() < depth0 && line_key[cpu.pc as usize] != -1)
                } else {
                    Box::new(move |cpu| cpu.shadow_stack.len() < depth0)
                });
            }
            (StepKind::Into, true) => {
                m.step_predicate = Some(Box::new(move |cpu| {
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
                m.step_predicate = Some(Box::new(move |cpu| {
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
        // Long steps run through the normal slicing so they stay pausable.
        self.start();
    }

    fn current_line_key(&mut self) -> i32 {
        let pc = self.machine.as_ref().map(|m| m.cpu.pc as usize).unwrap_or(0);
        (0..=pc.min(self.line_key.len().saturating_sub(1))).rev().map(|w| self.line_key[w]).find(|&k| k != -1).unwrap_or(-1)
    }

    // ---------------------------------------------------------------------------------
    // State publishing
    // ---------------------------------------------------------------------------------

    fn publish_if_idle(&mut self) {
        if !self.running {
            self.publish();
        }
    }

    pub fn publish(&mut self) {
        let running = self.running;
        let (trace_sent, flash_version, flash_sent) = (self.trace_sent, self.flash_version, self.flash_sent);
        let now = now_ms();
        let (sample_t, sample_c, sample_hz) = self.speed_sample;
        let Some(m) = self.machine.as_mut() else { return };
        let data: Vec<u8> = (0..m.cpu.data_end).map(|a| m.peek_data(a)).collect();
        let (trace_from, trace_cycles, trace_levels) = m.sys.trace.read_since(trace_sent, MAX_TRACE_PER_STATE);
        let new_trace_sent = m.sys.trace.seq;
        let dt = (now - sample_t) / 1000.0;
        let speed = if dt >= 0.25 || !running {
            let hz = if running && dt > 0.0 { (m.cpu.cycles - sample_c) as f64 / dt } else { 0.0 };
            Some((now, m.cpu.cycles, hz))
        } else {
            None
        };
        let flash = (flash_sent != flash_version).then(|| m.cpu.flash.clone());
        let pins = m
            .sys
            .pins
            .iter()
            .map(|p| PinState { level: p.level, dir: p.dir, out: p.out, pullup: p.pullup, ov_enable: p.ov_enable, ext: p.ext, ext_volts: p.ext_volts, volts: p.volts, reserved: p.reserved })
            .collect();
        let peripherals = m.inspect_peripherals().into_iter().map(|(name, values)| PeripheralInfo { name, values }).collect();
        let stack = &m.cpu.shadow_stack;
        let state = MachineState {
            running,
            pc: m.cpu.pc,
            sp: m.cpu.sp,
            sreg: m.cpu.sreg,
            cycles: m.cpu.cycles,
            instructions: m.cpu.instructions,
            time_sec: m.time_seconds(),
            hz: m.sys.clock.hz,
            sleeping: m.cpu.sleeping,
            sleep_mode: m.cpu.sleep_mode,
            reset_held: m.sys.reset_held,
            regs: m.cpu.r.to_vec(),
            data,
            flash,
            flash_version,
            fuse: m.cpu.fuse,
            lock: m.cpu.lock_bits,
            pins,
            vcc: m.sys.vcc,
            call_stack: stack[stack.len().saturating_sub(64)..].to_vec(),
            peripherals,
            speed_hz: speed.map(|s| s.2).unwrap_or(sample_hz),
            trace_from,
            trace_cycles,
            trace_levels,
            messages: m.messages(),
            stop: self.pending_stop.take(),
        };
        if let Some(s) = speed {
            self.speed_sample = s;
        }
        self.trace_sent = new_trace_sent;
        self.flash_sent = flash_version;
        self.last_publish = now;
        self.out.push(Output::State { state: Box::new(state) });
    }
}

/// Handle to a session running on its own thread.
#[cfg(not(target_arch = "wasm32"))]
pub struct SessionThread {
    tx: Sender<Command>,
    handle: Option<JoinHandle<()>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl SessionThread {
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for SessionThread {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Spawns the simulation thread. `emit` receives every output (called on the sim thread).
pub fn spawn(emit: impl Fn(Output) + Send + 'static) -> SessionThread {
    let (tx, rx): (Sender<Command>, Receiver<Command>) = mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("mcs-sim".into())
        .spawn(move || {
            let mut s = Session::new();
            loop {
                // Wait for a command (bounded by the pacing interval while running).
                let first = match rx.recv_timeout(s.idle_time()) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                };
                for cmd in first.into_iter().chain(rx.try_iter()) {
                    if matches!(cmd, Command::Shutdown) {
                        return;
                    }
                    for o in s.handle(cmd) {
                        emit(o);
                    }
                }
                for o in s.slice() {
                    emit(o);
                }
            }
        })
        .expect("spawn simulation thread");
    SessionThread { tx, handle: Some(handle) }
}
