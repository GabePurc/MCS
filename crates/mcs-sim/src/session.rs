//! Debugger session: owns one simulation target (any architecture), drives it in time slices (real-time or max speed),
//! implements stepping and produces state snapshots. Transport agnostic — [`spawn`] runs it on
//! a dedicated thread; tests drive it directly.

#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;
use std::time::Duration;

use crate::clock::now_ms;

use mcs_core::devices;
use mcs_core::program::LoadedProgram;

use crate::avr::Machine;
use crate::protocol::*;
use crate::target::{new_target, Sent, StepPlan, StopReason, Target};

const SLICE_MS: f64 = 8.0;
const STATE_INTERVAL_MS: f64 = 33.0;
/// Max simulated seconds caught up in one real-time slice (prevents death spirals).
const MAX_CATCHUP_SEC: f64 = 0.25;
/// Max trace entries per state message (older ones are dropped while running fast).
const MAX_TRACE_PER_STATE: usize = 50_000;

pub struct Session {
    machine: Option<Box<dyn Target>>,
    running: bool,
    speed: SpeedMode,
    factor: f64,
    cycles_per_slice: u64,
    wall_start: f64,
    sim_start: f64,
    /// Cycle count at `wall_start` (fixed-rate speed mode).
    cycle_start: u64,
    /// Cycle count of the last published state (skip unchanged publishes at slow speeds).
    published_cycles: u64,
    last_publish: f64,
    /// Incremental data (pin trace, EEPROM) already delivered to the UI.
    sent: Sent,
    flash_version: u64,
    flash_sent: u64,
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
            running: false,
            speed: SpeedMode::Realtime,
            factor: 1.0,
            cycles_per_slice: 200_000,
            wall_start: now,
            sim_start: 0.0,
            cycle_start: 0,
            published_cycles: u64::MAX,
            last_publish: now,
            sent: Sent { trace: 0, eeprom: u64::MAX },
            flash_version: 0,
            flash_sent: u64::MAX,
            breakpoints: Vec::new(),
            pending_stop: None,
            run_to: None,
            speed_sample: (now, 0, 0.0),
            out: Vec::new(),
        }
    }

    /// The simulation target (any architecture).
    pub fn target(&mut self) -> Option<&mut dyn Target> {
        match self.machine.as_mut() {
            Some(m) => Some(&mut **m),
            None => None,
        }
    }

    /// The concrete AVR machine, when the session simulates an AVR device (tests, tooling).
    pub fn avr_machine(&mut self) -> Option<&mut Machine> {
        self.machine.as_mut()?.as_any_mut().downcast_mut::<Machine>()
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
                let pc = self.m().pc();
                self.stop(StopInfo { reason: StopKind::Pause, pc, message: None });
            }
            Command::Reset => {
                self.m().debugger_reset();
                let pc = self.m().pc();
                self.stop(StopInfo { reason: StopKind::Reset, pc, message: None });
            }
            Command::PowerCycle => {
                self.m().power_cycle();
                self.sent.trace = 0;
                let pc = self.m().pc();
                self.stop(StopInfo { reason: StopKind::Reset, pc, message: None });
            }
            Command::Step { kind, source } => self.step(kind, source),
            Command::RunTo { pc } => {
                self.run_to = Some(pc);
                self.m().run_to(pc);
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
            Command::SetExternalClock { hz } => {
                self.m().set_external_clock(hz);
                self.resync_clock();
                self.publish_if_idle();
            }
            Command::SetClockConfig { source, prescale_log2 } => {
                self.m().set_clock_config(source, prescale_log2)?;
                self.resync_clock();
                self.publish_if_idle();
            }
            Command::SetPinGenerator { pin, gen } => {
                self.m().set_pin_generator(pin, gen);
                self.publish_if_idle();
            }
            Command::SetProfiling { enabled } => self.m().set_profiling(enabled),
            Command::SetSerial { config } => {
                self.m().set_serial(config);
                self.publish_if_idle();
            }
            Command::SerialSend { bytes } => self.m().serial_send(&bytes),
            Command::WriteEeprom { addr, value } => {
                self.m().write_eeprom(addr, value)?;
                self.publish_if_idle();
            }
            Command::WriteData { addr, value } => {
                self.m().write_data(addr, value)?;
                self.publish_if_idle();
            }
            Command::WriteFlash { addr, value } => {
                self.m().write_flash(addr, value)?;
                self.flash_version += 1;
                self.publish_if_idle();
            }
            Command::WriteReg { reg, value } => {
                self.m().write_reg(reg, value)?;
                self.publish_if_idle();
            }
            Command::WriteCpu { field, value } => {
                self.m().write_cpu(field, value)?;
                self.publish_if_idle();
            }
            Command::WriteFuse { index, value } => {
                self.m().write_fuse(index, value)?;
                let pc = self.m().pc();
                self.stop(StopInfo { reason: StopKind::Reset, pc, message: None });
            }
            Command::RequestState => self.publish(),
            Command::Init { .. } | Command::Load { .. } | Command::Shutdown => unreachable!(),
        }
        Ok(())
    }

    fn m(&mut self) -> &mut dyn Target {
        &mut **self.machine.as_mut().expect("machine")
    }

    fn create_machine(&mut self, device_id: &str, program: Option<LoadedProgram>) -> Result<(), String> {
        let dev = devices::get_any(device_id).ok_or_else(|| format!("Unknown device '{device_id}'"))?;
        self.running = false;
        if self.machine.as_ref().map(|m| !m.device().same_as(&dev)).unwrap_or(true) {
            self.machine = Some(new_target(dev));
            self.out.push(Output::Device { spec: dev });
        }
        self.m().load_program(program.as_ref());
        self.m().set_source_map(program.as_ref());
        self.sent = Sent { trace: 0, eeprom: u64::MAX };
        self.flash_version += 1;
        self.apply_breakpoints();
        let pc = self.m().pc();
        self.stop(StopInfo { reason: StopKind::Load, pc, message: None });
        Ok(())
    }

    fn apply_breakpoints(&mut self) {
        let Self { machine, breakpoints, .. } = self;
        if let Some(m) = machine.as_deref_mut() {
            m.set_breakpoints(breakpoints);
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
        let cycles = self.m().cycles();
        self.speed_sample = (now_ms(), cycles, 0.0);
        self.publish();
    }

    fn stop(&mut self, info: StopInfo) {
        self.running = false;
        if let Some(m) = self.machine.as_mut() {
            m.clear_stop_condition();
        }
        self.run_to = None;
        self.pending_stop = Some(info);
        self.publish();
    }

    fn resync_clock(&mut self) {
        self.wall_start = now_ms();
        self.sim_start = self.machine.as_ref().map(|m| m.elapsed_seconds()).unwrap_or(0.0);
        self.cycle_start = self.machine.as_ref().map(|m| m.cycles()).unwrap_or(0);
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
        let mut cycle_start = self.cycle_start;
        let m = self.m();
        let now_cycles = m.cycles();
        let target = match speed {
            SpeedMode::Max => now_cycles + cps,
            SpeedMode::Clock => {
                let want = cycle_start as f64 + wall * factor;
                let cap = now_cycles as f64 + (MAX_CATCHUP_SEC * factor).max(1.0);
                if want > cap {
                    cycle_start = (cap - wall * factor).max(0.0) as u64;
                }
                want.min(cap) as u64
            }
            SpeedMode::Realtime => {
                let sim_target = sim_start + wall * factor;
                let max_target = m.elapsed_seconds() + MAX_CATCHUP_SEC * factor;
                if sim_target > max_target {
                    // Fell behind (slow host): drop the backlog instead of spiralling.
                    sim_start = max_target - wall * factor;
                }
                m.cycle_at(sim_target.min(max_target))
            }
        };
        let reason = if target > now_cycles { m.run_until(target) } else { StopReason::Limit };
        self.sim_start = sim_start;
        self.cycle_start = cycle_start;
        let elapsed = now_ms() - t0;
        if speed == SpeedMode::Max {
            let e = elapsed.max(0.05);
            let scaled = self.cycles_per_slice as f64 * (SLICE_MS / e);
            self.cycles_per_slice = scaled.clamp(1_000.0, 2e9) as u64;
        }
        if reason != StopReason::Limit {
            let info = self.stop_info(reason);
            self.stop(info);
        } else {
            // Slow fixed-rate modes advance a few cycles per slice: only publish real changes
            // (plus a periodic refresh for the speed readout).
            let since = now_ms() - self.last_publish;
            let changed = self.machine.as_ref().is_some_and(|m| m.cycles() != self.published_cycles);
            if since >= STATE_INTERVAL_MS && (changed || since >= 500.0) {
                self.publish();
            }
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
            // Below ~250 Hz a cycle is due less often than every slice: sleep until it is.
            (true, SpeedMode::Clock) => (1000.0 / self.factor).clamp(SLICE_MS / 2.0, STATE_INTERVAL_MS),
            _ => 3_600_000.0,
        }
    }

    fn stop_info(&mut self, reason: StopReason) -> StopInfo {
        let pc = self.m().pc();
        match reason {
            StopReason::Breakpoint => StopInfo { reason: StopKind::Breakpoint, pc, message: None },
            StopReason::BreakInsn => StopInfo { reason: StopKind::Break, pc, message: Some("BREAK instruction executed".into()) },
            StopReason::InvalidOpcode => StopInfo { reason: StopKind::Invalid, pc, message: Some("Invalid opcode".into()) },
            StopReason::Lockup => StopInfo { reason: StopKind::Invalid, pc, message: Some("CPU locked up (fault while handling a fault)".into()) },
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
        match self.m().begin_step(kind, source) {
            StepPlan::Single => {
                let r = self.m().step_one();
                let info = if r == StopReason::Limit {
                    let pc = self.m().pc();
                    StopInfo { reason: StopKind::Step, pc, message: None }
                } else {
                    self.stop_info(r)
                };
                self.stop(info);
            }
            // Long steps run through the normal slicing so they stay pausable.
            StepPlan::Run => self.start(),
            StepPlan::Refused => self.publish(),
        }
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
        let (flash_version, flash_sent) = (self.flash_version, self.flash_sent);
        let now = now_ms();
        let (sample_t, sample_c, sample_hz) = self.speed_sample;
        let Some(m) = self.machine.as_deref_mut() else { return };
        let mut state = m.snapshot(&mut self.sent, flash_sent != flash_version, MAX_TRACE_PER_STATE);
        let dt = (now - sample_t) / 1000.0;
        let speed = if dt >= 0.25 || !running {
            let hz = if running && dt > 0.0 { (state.cycles - sample_c) as f64 / dt } else { 0.0 };
            Some((now, state.cycles, hz))
        } else {
            None
        };
        state.running = running;
        state.flash_version = flash_version;
        state.speed_hz = speed.map(|s| s.2).unwrap_or(sample_hz);
        state.stop = self.pending_stop.take();
        if let Some(s) = speed {
            self.speed_sample = s;
        }
        self.published_cycles = state.cycles;
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
