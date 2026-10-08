//! `Machine` = CPU core + clock + GPIO electrical model + peripherals for one AVR device, and
//! the instruction executor.
//!
//! Performance notes:
//! * Flash is pre-decoded into parallel arrays (op id + two normalized operands), so the
//!   executor never extracts bit fields; `match` over dense op ids compiles to a jump table.
//! * Peripherals are event driven (see `Scheduler`): the run loop only compares one integer per
//!   instruction for due events, checks one flag for interrupts and one bool for breakpoints.
//! * I/O registers without side effects are plain memory; only owned addresses dispatch to a
//!   peripheral (dynamic call), which is rare compared with ALU instructions.
//!
//! Peripherals never call each other directly. Cross-module effects (pin changes, ADC
//! triggers, power reduction, resets) are queued as [`Event`]s and broadcast after the current
//! peripheral call returns, which keeps borrowing simple and ordering deterministic.

use std::collections::{HashSet, VecDeque};

use mcs_core::avr::device::AvrDeviceSpec;
use mcs_core::avr::isa::op;
use mcs_core::program::LoadedProgram;
use serde::Serialize;

use super::cpu::*;
use super::peripherals;
use crate::pins::{ClockModel, ExtDrive, Pin, PinTrace};
use crate::scheduler::{EventKey, Scheduler};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResetSource {
    PowerOn,
    External,
    Watchdog,
    BrownOut,
    Debugger,
}

impl ResetSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::PowerOn => "power-on",
            Self::External => "external",
            Self::Watchdog => "watchdog",
            Self::BrownOut => "brown-out",
            Self::Debugger => "debugger",
        }
    }
}

/// Peripheral-to-peripheral trigger signals (ADC auto trigger, comparator capture...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Ac,
    Int0,
    Tc0CompA,
    Tc0Ovf,
    Tc0CompB,
    PcInt,
    Tc0Capt,
    /// Analog comparator output changed (value in the event).
    AcOutput,
}

/// Broadcast events, processed after the peripheral call that produced them.
#[derive(Clone, Copy, Debug)]
pub enum Event {
    Pin { pin: u8, level: u8, cycle: u64 },
    Analog { pin: u8 },
    Trigger { trigger: Trigger, value: u8, cycle: u64 },
    /// New PRR value.
    PowerReduction(u8),
    ClockChanged,
    VccChanged,
}

#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub cycle: u64,
    pub level: &'static str,
    pub text: String,
}

const MAX_MESSAGES: usize = 200;

/// Machine-wide services available to peripherals.
pub struct Sys {
    pub sched: Scheduler,
    pub pins: Vec<Pin>,
    pub clock: ClockModel,
    pub trace: PinTrace,
    pub vcc: f64,
    pub events: VecDeque<Event>,
    pub messages: Vec<Message>,
    warned: HashSet<String>,
    pub reset_request: Option<ResetSource>,
    /// GPIO acting as RESET (or None).
    pub reset_pin: Option<usize>,
    pub reset_held: bool,
    pub last_reset: ResetSource,
    /// Configuration Change Protection window end (inclusive cycle).
    pub ccp_until: u64,
    /// External clock frequency (CLKMSR = external).
    pub ext_clock_hz: f64,
}

impl Sys {
    pub fn warn(&mut self, cycle: u64, text: impl Into<String>) {
        let text = text.into();
        self.warn_key(cycle, text.clone(), text);
    }

    /// De-duplicated warning (tight loops would otherwise flood the log).
    pub fn warn_key(&mut self, cycle: u64, key: impl Into<String>, text: impl Into<String>) {
        let key = key.into();
        if self.warned.contains(&key) {
            return;
        }
        if self.warned.len() > 1000 {
            self.warned.clear();
        }
        self.warned.insert(key);
        self.message(cycle, "warning", text);
    }

    pub fn message(&mut self, cycle: u64, level: &'static str, text: impl Into<String>) {
        self.messages.push(Message { cycle, level, text: text.into() });
        if self.messages.len() > MAX_MESSAGES {
            let excess = self.messages.len() - MAX_MESSAGES;
            self.messages.drain(..excess);
        }
    }

    pub fn levels_mask(&self) -> u32 {
        self.pins.iter().enumerate().fold(0, |m, (i, p)| m | ((p.level as u32) << i))
    }

    /// Re-resolves one pin after its MCU-side or external configuration changed.
    pub fn update_pin(&mut self, i: usize, cycle: u64) {
        let vcc = self.vcc;
        let p = &mut self.pins[i];
        let prev_volts = p.volts;
        let changed = p.resolve(vcc);
        let level = p.level;
        let volts_changed = p.volts != prev_volts;
        if p.contention() {
            let msg = format!("Pin {}: output driven against an external {} source (short circuit!)", p.name, if p.ext == ExtDrive::Low { "LOW" } else { "HIGH" });
            self.warn_key(cycle, format!("contention-{i}"), msg);
        }
        if changed {
            let m = self.levels_mask();
            self.trace.record(cycle, m);
            self.events.push_back(Event::Pin { pin: i as u8, level, cycle });
        }
        if volts_changed {
            self.events.push_back(Event::Analog { pin: i as u8 });
        }
        if Some(i) == self.reset_pin {
            if level == 0 && !self.reset_held {
                self.reset_held = true;
                self.reset_request = Some(ResetSource::External);
            } else if level != 0 && self.reset_held {
                self.reset_held = false;
                self.message(cycle, "info", "External RESET released");
            }
        }
    }

    pub fn trigger(&mut self, trigger: Trigger, value: u8, cycle: u64) {
        self.events.push_back(Event::Trigger { trigger, value, cycle });
    }
}

/// Context handed to peripheral callbacks.
pub struct Cx<'a> {
    pub cpu: &'a mut Cpu,
    pub sys: &'a mut Sys,
    /// Index of the peripheral being called (for scheduling its own events).
    pub me: u8,
}

impl Cx<'_> {
    #[inline]
    pub fn now(&self) -> u64 {
        self.cpu.cycles
    }

    pub fn schedule(&mut self, tag: u8, cycle: u64) {
        self.sys.sched.at(EventKey { owner: self.me, tag }, cycle);
    }

    pub fn cancel(&mut self, tag: u8) {
        self.sys.sched.cancel(EventKey { owner: self.me, tag });
    }

    pub fn warn(&mut self, key: &str, text: impl Into<String>) {
        let c = self.cpu.cycles;
        self.sys.warn_key(c, key, text);
    }

    pub fn time_seconds(&self) -> f64 {
        self.sys.clock.time_at(self.cpu.cycles)
    }

    /// Fuse bit (by name) is programmed (= 0).
    pub fn fuse_programmed(&self, name: &str) -> bool {
        self.cpu.spec.fuse_bits.iter().find(|f| f.name == name).is_some_and(|f| self.cpu.fuse & f.mask == 0)
    }
}

/// A peripheral model. Register addresses are data-space addresses the peripheral claimed
/// via `Machine::claim_io`.
#[allow(unused_variables)]
pub trait Peripheral: Send {
    fn name(&self) -> &str;
    /// Called after I/O registers were set to their reset values.
    fn reset(&mut self, cx: &mut Cx);
    fn read(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        cx.cpu.data[addr as usize]
    }
    fn write(&mut self, addr: u16, value: u8, cx: &mut Cx) {
        cx.cpu.data[addr as usize] = value;
    }
    /// Side-effect free read for debugger views.
    fn peek(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        cx.cpu.data[addr as usize]
    }
    fn on_event(&mut self, tag: u8, cycle: u64, cx: &mut Cx) {}
    /// Interrupt vector `vector` is being executed (clear flags here).
    fn ack(&mut self, vector: u8, cx: &mut Cx) {}
    fn on_pin(&mut self, pin: u8, level: u8, cycle: u64, cx: &mut Cx) {}
    fn on_analog(&mut self, pin: u8, cx: &mut Cx) {}
    fn on_trigger(&mut self, trigger: Trigger, value: u8, cycle: u64, cx: &mut Cx) {}
    fn on_power_reduction(&mut self, prr: u8, cx: &mut Cx) {}
    fn on_clock_change(&mut self, cx: &mut Cx) {}
    fn on_vcc_change(&mut self, cx: &mut Cx) {}
    fn on_sleep(&mut self, mode: u8, cx: &mut Cx) {}
    fn on_wake(&mut self, cx: &mut Cx) {}
    fn on_wdr(&mut self, cx: &mut Cx) {}
    /// Internal (non register-mapped) state for the UI.
    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        Vec::new()
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// Per-instruction stop predicate used for stepping (true = stop before executing).
pub type StepPredicate = Box<dyn FnMut(&Cpu) -> bool + Send>;

pub struct Machine {
    pub spec: &'static AvrDeviceSpec,
    pub cpu: Cpu,
    pub sys: Sys,
    pub periphs: Vec<Box<dyn Peripheral>>,
    smcr: Option<u16>,
    /// Optional per-instruction predicate used for stepping; returns true to stop before executing.
    pub step_predicate: Option<StepPredicate>,
}

impl Machine {
    pub fn new(spec: &'static AvrDeviceSpec) -> Self {
        let hz = spec.clock.internal_hz / (1u32 << spec.clock.default_prescale_log2) as f64;
        let pins = (0..spec.gpio_count).map(|i| Pin::new(format!("P{}{}", spec.gpio_port_name, i))).collect();
        let mut m = Self {
            spec,
            cpu: Cpu::new(spec),
            sys: Sys {
                sched: Scheduler::new(),
                pins,
                clock: ClockModel::new(hz),
                trace: PinTrace::new(1 << 18),
                vcc: spec.vcc,
                events: VecDeque::new(),
                messages: Vec::new(),
                warned: HashSet::new(),
                reset_request: None,
                reset_pin: None,
                reset_held: false,
                last_reset: ResetSource::PowerOn,
                ccp_until: 0,
                ext_clock_hz: 8_000_000.0,
            },
            periphs: Vec::new(),
            smcr: spec.register("SMCR").map(|r| r.addr),
            step_predicate: None,
        };
        let sreg = spec.reg("SREG");
        m.cpu.io_owner[sreg as usize] = IO_SREG;
        m.cpu.io_owner[spec.reg("SPL") as usize] = IO_SPL;
        if let Some(sph) = spec.register("SPH") {
            m.cpu.io_owner[sph.addr as usize] = IO_SPH;
        }
        peripherals::wire(&mut m);
        m.power_on();
        m
    }

    /// Registers a peripheral; returns its index.
    pub fn add_peripheral(&mut self, p: Box<dyn Peripheral>) -> u8 {
        self.periphs.push(p);
        (self.periphs.len() - 1) as u8
    }

    /// Routes accesses to data address `addr` to peripheral `owner`. `rmw_clear` bits read as 0
    /// for SBI/CBI (write-one-to-clear flags).
    pub fn claim_io(&mut self, addr: u16, owner: u8, rmw_clear: u8) {
        self.cpu.io_owner[addr as usize] = owner;
        self.cpu.rmw_clear[addr as usize] = rmw_clear;
    }

    pub fn claim_irq(&mut self, vector: Option<u8>, owner: u8) {
        if let Some(v) = vector {
            self.cpu.irq_owner[v as usize] = owner;
        }
    }

    /// Typed access to a peripheral by name.
    pub fn peripheral_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.periphs.iter_mut().find_map(|p| p.as_any_mut().downcast_mut::<T>())
    }

    // ---------------------------------------------------------------------------------
    // Peripheral dispatch
    // ---------------------------------------------------------------------------------

    fn call<R>(&mut self, i: u8, f: impl FnOnce(&mut dyn Peripheral, &mut Cx) -> R) -> R {
        let Machine { cpu, sys, periphs, .. } = self;
        let r = f(periphs[i as usize].as_mut(), &mut Cx { cpu, sys, me: i });
        if !self.sys.events.is_empty() || self.sys.reset_request.is_some() {
            self.drain_events();
        }
        r
    }

    fn broadcast(&mut self, mut f: impl FnMut(&mut dyn Peripheral, &mut Cx)) {
        let Machine { cpu, sys, periphs, .. } = self;
        for (i, p) in periphs.iter_mut().enumerate() {
            f(p.as_mut(), &mut Cx { cpu, sys, me: i as u8 });
        }
        if !self.sys.events.is_empty() || self.sys.reset_request.is_some() {
            self.drain_events();
        }
    }

    /// Delivers queued events to all peripherals, then performs a requested reset.
    pub fn drain_events(&mut self) {
        while let Some(ev) = self.sys.events.pop_front() {
            let Machine { cpu, sys, periphs, .. } = self;
            for (i, p) in periphs.iter_mut().enumerate() {
                let cx = &mut Cx { cpu, sys, me: i as u8 };
                match ev {
                    Event::Pin { pin, level, cycle } => p.on_pin(pin, level, cycle, cx),
                    Event::Analog { pin } => p.on_analog(pin, cx),
                    Event::Trigger { trigger, value, cycle } => p.on_trigger(trigger, value, cycle, cx),
                    Event::PowerReduction(v) => p.on_power_reduction(v, cx),
                    Event::ClockChanged => p.on_clock_change(cx),
                    Event::VccChanged => p.on_vcc_change(cx),
                }
            }
        }
        if let Some(src) = self.sys.reset_request.take() {
            self.reset(src);
        }
    }

    fn dispatch_scheduled(&mut self) {
        while let Some((key, cycle)) = self.sys.sched.pop_due(self.cpu.cycles) {
            self.call(key.owner, |p, cx| p.on_event(key.tag, cycle, cx));
        }
    }

    // ---------------------------------------------------------------------------------
    // Data bus
    // ---------------------------------------------------------------------------------

    #[inline(always)]
    pub fn read_data(&mut self, addr: u16) -> u8 {
        let a = addr as usize;
        if a < self.cpu.sram_start as usize {
            if a < self.cpu.io_base as usize {
                return self.cpu.r[a];
            }
            let owner = self.cpu.io_owner[a];
            if owner == IO_PLAIN {
                return self.cpu.data[a];
            }
            return self.read_io(addr, owner);
        }
        if a < self.cpu.data_end as usize {
            return self.cpu.data[a];
        }
        self.read_mapped(addr, true)
    }

    #[inline(always)]
    pub fn write_data(&mut self, addr: u16, v: u8) {
        let a = addr as usize;
        if a < self.cpu.sram_start as usize {
            if a < self.cpu.io_base as usize {
                self.cpu.r[a] = v;
                return;
            }
            let owner = self.cpu.io_owner[a];
            if owner == IO_PLAIN {
                self.cpu.data[a] = v;
            } else {
                self.write_io(addr, owner, v);
            }
            return;
        }
        if a < self.cpu.data_end as usize {
            self.cpu.data[a] = v;
            return;
        }
        self.write_mapped(addr);
    }

    #[cold]
    fn read_io(&mut self, addr: u16, owner: u8) -> u8 {
        match owner {
            IO_SREG => self.cpu.sreg,
            IO_SPL => self.cpu.sp as u8,
            IO_SPH => (self.cpu.sp >> 8) as u8,
            i => self.call(i, |p, cx| p.read(addr, cx)),
        }
    }

    #[cold]
    fn write_io(&mut self, addr: u16, owner: u8, v: u8) {
        match owner {
            IO_SREG => {
                if v & SREG_I != 0 && self.cpu.sreg & SREG_I == 0 && self.cpu.pending_vector().is_some() {
                    self.cpu.irq_dirty = true;
                }
                self.cpu.sreg = v;
            }
            IO_SPL => self.cpu.sp = (self.cpu.sp & 0xff00) | v as u16,
            IO_SPH => self.cpu.sp = (self.cpu.sp & 0x00ff) | ((v as u16) << 8),
            i => self.call(i, |p, cx| p.write(addr, v, cx)),
        }
    }

    /// Side-effect free read of any data-space address (for debugger views).
    pub fn peek_data(&mut self, addr: u16) -> u8 {
        let a = addr as usize;
        if a < self.cpu.sram_start as usize {
            if a < self.cpu.io_base as usize {
                return self.cpu.r[a];
            }
            return match self.cpu.io_owner[a] {
                IO_PLAIN => self.cpu.data[a],
                IO_SREG => self.cpu.sreg,
                IO_SPL => self.cpu.sp as u8,
                IO_SPH => (self.cpu.sp >> 8) as u8,
                i => self.call(i, |p, cx| p.peek(addr, cx)),
            };
        }
        if a < self.cpu.data_end as usize {
            return self.cpu.data[a];
        }
        self.read_mapped(addr, false)
    }

    /// Debugger write (goes through peripherals so they observe it).
    pub fn poke_data(&mut self, addr: u16, v: u8) {
        self.write_data(addr, v);
    }

    /// Reads outside I/O+SRAM: memory-mapped flash and NVM areas (AVRrc).
    #[cold]
    fn read_mapped(&mut self, addr: u16, warn: bool) -> u8 {
        let s = self.spec;
        if let Some(base) = s.flash_map_base {
            if addr >= base && ((addr - base) as u32) < s.flash_size {
                return self.cpu.flash[(addr - base) as usize];
            }
        }
        if let Some(n) = s.nvm_map {
            if addr == n.lock {
                return self.cpu.lock_bits;
            }
            if addr == n.config {
                return self.cpu.fuse;
            }
            if addr == n.calibration {
                return s.calibration;
            }
            if addr >= n.signature && addr < n.signature + 3 {
                return s.signature[(addr - n.signature) as usize];
            }
            let within = |b: u16, len: u16| addr >= b && addr < b + len;
            if within(n.lock, 2) || within(n.config, 2) || within(n.calibration, 2) || within(n.signature, 4) {
                return 0xff;
            }
        }
        if warn {
            let (c, pc) = (self.cpu.cycles, self.cpu.pc * 2);
            self.sys.warn(c, format!("Read from unmapped data address 0x{addr:04X} at PC 0x{pc:04X}"));
        }
        0
    }

    #[cold]
    fn write_mapped(&mut self, addr: u16) {
        let s = self.spec;
        let (c, pc) = (self.cpu.cycles, self.cpu.pc * 2);
        if let Some(base) = s.flash_map_base {
            if addr >= base && ((addr - base) as u32) < s.flash_size {
                self.sys.warn(c, format!("Write to memory-mapped flash 0x{addr:04X} ignored (NVM is not self-programmable) at PC 0x{pc:04X}"));
                return;
            }
        }
        self.sys.warn(c, format!("Write to unmapped data address 0x{addr:04X} ignored at PC 0x{pc:04X}"));
    }

    // ---------------------------------------------------------------------------------
    // Pins / environment
    // ---------------------------------------------------------------------------------

    /// External stimulus from the UI / test bench.
    pub fn set_pin_input(&mut self, i: usize, ext: ExtDrive, volts: f64) {
        if i >= self.sys.pins.len() {
            return;
        }
        self.sys.pins[i].ext = ext;
        self.sys.pins[i].ext_volts = volts;
        let c = self.cpu.cycles;
        self.sys.update_pin(i, c);
        self.drain_events();
    }

    pub fn set_vcc(&mut self, v: f64) {
        self.sys.vcc = v;
        let c = self.cpu.cycles;
        for i in 0..self.sys.pins.len() {
            self.sys.update_pin(i, c);
        }
        self.sys.events.push_back(Event::VccChanged);
        self.drain_events();
    }

    pub fn time_seconds(&self) -> f64 {
        self.sys.clock.time_at(self.cpu.cycles)
    }

    // ---------------------------------------------------------------------------------
    // Program / reset
    // ---------------------------------------------------------------------------------

    pub fn load(&mut self, program: &LoadedProgram) {
        self.cpu.load_flash(&program.flash);
        if let Some(f) = program.fuses.as_ref().and_then(|f| f.first()) {
            self.cpu.fuse = *f;
        }
        if let Some(l) = program.lock.as_ref().and_then(|l| l.first()) {
            self.cpu.lock_bits = *l;
        }
        self.power_on();
    }

    pub fn power_on(&mut self) {
        let hz = self.spec.clock.internal_hz / (1u32 << self.spec.clock.default_prescale_log2) as f64;
        self.sys.clock.reset(hz);
        self.sys.trace.clear();
        self.sys.sched.clear();
        self.sys.reset_held = false;
        self.reset(ResetSource::PowerOn);
        let m = self.sys.levels_mask();
        self.sys.trace.record(0, m);
    }

    pub fn reset(&mut self, source: ResetSource) {
        let power_on = source == ResetSource::PowerOn;
        self.sys.last_reset = source;
        self.cpu.reset(power_on);
        for r in &self.spec.registers {
            self.cpu.data[r.addr as usize] = r.reset;
        }
        self.cpu.sp = self.cpu.data_end - 1;
        self.sys.reset_request = None;
        self.broadcast(|p, cx| p.reset(cx));
        // Peripheral resets may have set pin directions: resolve every pin.
        let c = self.cpu.cycles;
        for i in 0..self.sys.pins.len() {
            self.sys.update_pin(i, c);
        }
        self.sys.reset_request = None;
        self.drain_events();
        if !power_on {
            self.sys.message(c, "info", format!("MCU reset ({})", source.label()));
        }
    }

    pub fn messages(&mut self) -> Vec<Message> {
        std::mem::take(&mut self.sys.messages)
    }

    pub fn inspect_peripherals(&mut self) -> Vec<(String, Vec<(String, String)>)> {
        let Machine { cpu, sys, periphs, .. } = self;
        periphs
            .iter_mut()
            .enumerate()
            .map(|(i, p)| {
                let name = p.name().to_string();
                (name, p.inspect(&mut Cx { cpu, sys, me: i as u8 }))
            })
            .filter(|(_, v)| !v.is_empty())
            .collect()
    }

    // ---------------------------------------------------------------------------------
    // Execution
    // ---------------------------------------------------------------------------------

    pub fn request_stop(&mut self, reason: StopReason) {
        self.cpu.stop_reason = reason;
        self.cpu.halt = true;
    }

    /// Runs until `limit` cycles, a breakpoint, a BREAK instruction, an invalid opcode or a stop
    /// request. A breakpoint at the starting PC is ignored so execution can resume from it.
    /// While RESET is held only time advances.
    pub fn run(&mut self, limit: u64) -> StopReason {
        self.cpu.stop_reason = StopReason::None;
        self.cpu.halt = false;
        if self.sys.reset_held {
            let t = limit.min(self.sys.sched.next);
            if t > self.cpu.cycles {
                self.cpu.cycles = t;
            }
            self.dispatch_scheduled();
            return StopReason::Limit;
        }
        let mut skip_bp = true;
        let mut pred = self.step_predicate.take();
        while self.cpu.cycles < limit {
            if self.cpu.sleeping {
                if self.cpu.wake_pending() {
                    self.wake();
                    continue;
                }
                let t = limit.min(self.sys.sched.next);
                if t > self.cpu.cycles {
                    self.cpu.cycles = t;
                }
                self.dispatch_scheduled();
                if self.sys.reset_held {
                    break;
                }
                continue;
            }
            if self.cpu.irq_inhibit {
                self.cpu.irq_inhibit = false;
            } else if self.cpu.irq_dirty && self.cpu.sreg & SREG_I != 0 && self.service_irq() {
                if self.sys.sched.next <= self.cpu.cycles {
                    self.dispatch_scheduled();
                }
                continue;
            }
            let pc = self.cpu.pc as usize;
            if !skip_bp {
                if self.cpu.breakpoints[pc] {
                    self.cpu.stop_reason = StopReason::Breakpoint;
                    break;
                }
                if let Some(p) = pred.as_mut() {
                    if p(&self.cpu) {
                        self.cpu.stop_reason = StopReason::Requested;
                        break;
                    }
                }
            }
            skip_bp = false;
            self.exec();
            if self.sys.sched.next <= self.cpu.cycles {
                self.dispatch_scheduled();
            }
            if self.cpu.halt {
                break;
            }
        }
        self.step_predicate = pred;
        if self.cpu.stop_reason == StopReason::None {
            self.cpu.stop_reason = StopReason::Limit;
        }
        self.cpu.stop_reason
    }

    /// Executes exactly one instruction (servicing a pending interrupt first, like hardware).
    pub fn step(&mut self) -> StopReason {
        self.cpu.stop_reason = StopReason::None;
        self.cpu.halt = false;
        if self.sys.reset_held {
            return StopReason::Limit;
        }
        if self.cpu.sleeping {
            if !self.cpu.wake_pending() {
                let t = self.sys.sched.next;
                if t == u64::MAX {
                    return StopReason::Limit;
                }
                if t > self.cpu.cycles {
                    self.cpu.cycles = t;
                }
                self.dispatch_scheduled();
                if !self.cpu.wake_pending() {
                    return StopReason::Limit;
                }
            }
            self.wake();
        }
        if self.cpu.irq_inhibit {
            self.cpu.irq_inhibit = false;
        } else if self.cpu.irq_dirty && self.cpu.sreg & SREG_I != 0 && self.service_irq() {
            if self.sys.sched.next <= self.cpu.cycles {
                self.dispatch_scheduled();
            }
            return StopReason::Limit;
        }
        self.exec();
        if self.sys.sched.next <= self.cpu.cycles {
            self.dispatch_scheduled();
        }
        if self.cpu.halt { self.cpu.stop_reason } else { StopReason::Limit }
    }

    fn wake(&mut self) {
        if !self.cpu.sleeping {
            return;
        }
        self.cpu.sleeping = false;
        self.cpu.cycles += 4;
        self.broadcast(|p, cx| p.on_wake(cx));
    }

    fn service_irq(&mut self) -> bool {
        let Some(v) = self.cpu.pending_vector() else {
            self.cpu.irq_dirty = false;
            return false;
        };
        self.cpu.irq_pending[v as usize] = false;
        let owner = self.cpu.irq_owner[v as usize];
        if owner != IO_PLAIN {
            self.call(owner, |p, cx| p.ack(v, cx));
        }
        let ret = self.cpu.pc;
        self.push_pc(ret);
        self.cpu.sreg &= !SREG_I;
        self.cpu.pc = v as u32 * self.cpu.vector_words;
        self.cpu.cycles += 4;
        let target = self.cpu.pc;
        self.cpu.push_frame(ret, target, v as i16);
        true
    }

    #[inline]
    fn push_pc(&mut self, ret: u32) {
        // Low byte is pushed first, so the return address is stored big-endian in memory.
        let sp = self.cpu.sp;
        self.write_data(sp, ret as u8);
        self.write_data(sp.wrapping_sub(1), (ret >> 8) as u8);
        self.cpu.sp = sp.wrapping_sub(2);
        self.check_stack();
    }

    #[inline]
    fn pop_pc(&mut self) -> u32 {
        let sp = self.cpu.sp;
        let hi = self.read_data(sp.wrapping_add(1)) as u32;
        let lo = self.read_data(sp.wrapping_add(2)) as u32;
        self.cpu.sp = sp.wrapping_add(2);
        ((hi << 8) | lo) & self.cpu.pc_mask
    }

    #[inline]
    fn check_stack(&mut self) {
        if (self.cpu.sp as u32 + 1) < self.cpu.sram_start as u32 {
            let (c, sp, pc) = (self.cpu.cycles, self.cpu.sp, self.cpu.pc * 2);
            self.sys.warn_key(c, "stack-overflow", format!("Stack overflow: SP=0x{sp:04X} below SRAM start (PC 0x{pc:04X})"));
        }
    }

    #[inline]
    fn skip(&mut self) {
        let n = self.cpu.insn_words_at(self.cpu.pc + 1);
        self.cpu.pc = (self.cpu.pc + 1 + n) & self.cpu.pc_mask;
        self.cpu.cycles += n as u64;
    }

    #[inline(always)]
    fn ptr(&self, lo: usize) -> u16 {
        self.cpu.r[lo] as u16 | ((self.cpu.r[lo + 1] as u16) << 8)
    }

    #[inline(always)]
    fn set_ptr(&mut self, lo: usize, v: u16) {
        self.cpu.r[lo] = v as u8;
        self.cpu.r[lo + 1] = (v >> 8) as u8;
    }

    fn rmw_read(&mut self, addr: u16) -> u8 {
        let clear = self.cpu.rmw_clear.get(addr as usize).copied().unwrap_or(0);
        self.read_data(addr) & !clear
    }

    fn exec(&mut self) {
        let pc = self.cpu.pc;
        let i = pc as usize;
        let o = self.cpu.ops[i];
        let a = self.cpu.oa[i];
        let b = self.cpu.ob[i];
        self.cpu.cycles += self.cpu.cyc[o as usize] as u64;
        self.cpu.instructions += 1;
        let mask = self.cpu.pc_mask;
        let mut next = (pc + 1) & mask;
        let (au, bu) = (a as usize & 31, b as usize & 31);

        match o {
            op::NOP => {}
            op::MOVW => {
                self.cpu.r[au] = self.cpu.r[bu];
                self.cpu.r[au + 1] = self.cpu.r[bu + 1];
            }
            op::MULS | op::MULSU | op::FMUL | op::FMULS | op::FMULSU => {
                let (d, r) = (self.cpu.r[au], self.cpu.r[bu]);
                let sd = d as i8 as i32;
                let sr = r as i8 as i32;
                let (p, shift) = match o {
                    op::MULS => (sd * sr, false),
                    op::MULSU => (sd * r as i32, false),
                    op::FMUL => (d as i32 * r as i32, true),
                    op::FMULS => (sd * sr, true),
                    _ => (sd * r as i32, true),
                };
                let p = p as u32 & 0xffff;
                let res = if shift { (p << 1) & 0xffff } else { p };
                self.cpu.r[0] = res as u8;
                self.cpu.r[1] = (res >> 8) as u8;
                self.cpu.sreg = (self.cpu.sreg & !(SREG_C | SREG_Z)) | ((p >> 15) & 1) as u8 | if res == 0 { SREG_Z } else { 0 };
            }
            op::MUL => {
                let res = self.cpu.r[au] as u32 * self.cpu.r[bu] as u32;
                self.cpu.r[0] = res as u8;
                self.cpu.r[1] = (res >> 8) as u8;
                self.cpu.sreg = (self.cpu.sreg & !(SREG_C | SREG_Z)) | ((res >> 15) & 1) as u8 | if res == 0 { SREG_Z } else { 0 };
            }
            op::CPC => {
                let (d, r) = (self.cpu.r[au], self.cpu.r[bu]);
                let res = d.wrapping_sub(r).wrapping_sub(self.cpu.sreg & 1);
                self.cpu.sreg = sub_flags(self.cpu.sreg, d, r, res, true);
            }
            op::SBC => {
                let (d, r) = (self.cpu.r[au], self.cpu.r[bu]);
                let res = d.wrapping_sub(r).wrapping_sub(self.cpu.sreg & 1);
                self.cpu.r[au] = res;
                self.cpu.sreg = sub_flags(self.cpu.sreg, d, r, res, true);
            }
            op::ADD => {
                let (d, r) = (self.cpu.r[au], self.cpu.r[bu]);
                let res = d.wrapping_add(r);
                self.cpu.r[au] = res;
                self.cpu.sreg = add_flags(self.cpu.sreg, d, r, res);
            }
            op::ADC => {
                let (d, r) = (self.cpu.r[au], self.cpu.r[bu]);
                let res = d.wrapping_add(r).wrapping_add(self.cpu.sreg & 1);
                self.cpu.r[au] = res;
                self.cpu.sreg = add_flags(self.cpu.sreg, d, r, res);
            }
            op::CPSE => {
                if self.cpu.r[au] == self.cpu.r[bu] {
                    self.skip();
                    return;
                }
            }
            op::CP => {
                let (d, r) = (self.cpu.r[au], self.cpu.r[bu]);
                self.cpu.sreg = sub_flags(self.cpu.sreg, d, r, d.wrapping_sub(r), false);
            }
            op::SUB => {
                let (d, r) = (self.cpu.r[au], self.cpu.r[bu]);
                let res = d.wrapping_sub(r);
                self.cpu.r[au] = res;
                self.cpu.sreg = sub_flags(self.cpu.sreg, d, r, res, false);
            }
            op::AND | op::EOR | op::OR => {
                let (d, r) = (self.cpu.r[au], self.cpu.r[bu]);
                let res = match o {
                    op::AND => d & r,
                    op::EOR => d ^ r,
                    _ => d | r,
                };
                self.cpu.r[au] = res;
                self.cpu.sreg = logic_flags(self.cpu.sreg, res);
            }
            op::MOV => self.cpu.r[au] = self.cpu.r[bu],
            op::CPI => {
                let d = self.cpu.r[au];
                self.cpu.sreg = sub_flags(self.cpu.sreg, d, b as u8, d.wrapping_sub(b as u8), false);
            }
            op::SBCI => {
                let d = self.cpu.r[au];
                let res = d.wrapping_sub(b as u8).wrapping_sub(self.cpu.sreg & 1);
                self.cpu.r[au] = res;
                self.cpu.sreg = sub_flags(self.cpu.sreg, d, b as u8, res, true);
            }
            op::SUBI => {
                let d = self.cpu.r[au];
                let res = d.wrapping_sub(b as u8);
                self.cpu.r[au] = res;
                self.cpu.sreg = sub_flags(self.cpu.sreg, d, b as u8, res, false);
            }
            op::ORI | op::ANDI => {
                let res = if o == op::ORI { self.cpu.r[au] | b as u8 } else { self.cpu.r[au] & b as u8 };
                self.cpu.r[au] = res;
                self.cpu.sreg = logic_flags(self.cpu.sreg, res);
            }
            op::LD_Y => {
                let addr = self.ptr(28);
                self.cpu.r[au] = self.read_data(addr);
            }
            op::LD_Z => {
                let addr = self.ptr(30);
                self.cpu.r[au] = self.read_data(addr);
            }
            op::ST_Y => {
                let (addr, v) = (self.ptr(28), self.cpu.r[au]);
                self.write_data(addr, v);
            }
            op::ST_Z => {
                let (addr, v) = (self.ptr(30), self.cpu.r[au]);
                self.write_data(addr, v);
            }
            op::LDD_Y | op::LDD_Z => {
                let base = self.ptr(if o == op::LDD_Y { 28 } else { 30 });
                self.cpu.r[au] = self.read_data(base.wrapping_add(b as u16));
            }
            op::STD_Y | op::STD_Z => {
                let base = self.ptr(if o == op::STD_Y { 28 } else { 30 });
                let v = self.cpu.r[bu];
                self.write_data(base.wrapping_add(a as u16), v);
            }
            op::LDS_RC => self.cpu.r[au] = self.read_data(b as u16),
            op::STS_RC => {
                let v = self.cpu.r[bu];
                self.write_data(a as u16, v);
            }
            op::LDS => {
                self.cpu.r[au] = self.read_data(b as u16);
                next = (pc + 2) & mask;
            }
            op::STS => {
                let v = self.cpu.r[bu];
                self.write_data(a as u16, v);
                next = (pc + 2) & mask;
            }
            op::LD_ZP | op::LD_YP | op::LD_XP => {
                let lo = ptr_reg(o);
                let p = self.ptr(lo);
                self.cpu.r[au] = self.read_data(p);
                self.set_ptr(lo, p.wrapping_add(1));
            }
            op::LD_MZ | op::LD_MY | op::LD_MX => {
                let lo = ptr_reg(o);
                let p = self.ptr(lo).wrapping_sub(1);
                self.set_ptr(lo, p);
                self.cpu.r[au] = self.read_data(p);
            }
            op::LD_X => {
                let p = self.ptr(26);
                if self.cpu.rc && p >= self.cpu.data_end {
                    self.cpu.cycles += 1; // NVM / flash access wait state
                }
                self.cpu.r[au] = self.read_data(p);
            }
            op::ST_ZP | op::ST_YP | op::ST_XP => {
                let lo = ptr_reg(o);
                let p = self.ptr(lo);
                let v = self.cpu.r[au];
                self.write_data(p, v);
                self.set_ptr(lo, p.wrapping_add(1));
            }
            op::ST_MZ | op::ST_MY | op::ST_MX => {
                let lo = ptr_reg(o);
                let p = self.ptr(lo).wrapping_sub(1);
                self.set_ptr(lo, p);
                let v = self.cpu.r[au];
                self.write_data(p, v);
            }
            op::ST_X => {
                let (p, v) = (self.ptr(26), self.cpu.r[au]);
                self.write_data(p, v);
            }
            op::LPM_Z | op::LPM_ZP | op::ELPM_Z | op::ELPM_ZP | op::LPM | op::ELPM => {
                let z = self.ptr(30);
                let v = self.cpu.flash[z as usize % self.cpu.flash.len()];
                if o == op::LPM || o == op::ELPM {
                    self.cpu.r[0] = v;
                } else {
                    self.cpu.r[au] = v;
                }
                if o == op::LPM_ZP || o == op::ELPM_ZP {
                    self.set_ptr(30, z.wrapping_add(1));
                }
            }
            op::XCH | op::LAS | op::LAC | op::LAT => {
                let z = self.ptr(30);
                let m = self.read_data(z);
                let r = self.cpu.r[au];
                let w = match o {
                    op::XCH => r,
                    op::LAS => m | r,
                    op::LAC => m & !r,
                    _ => m ^ r,
                };
                self.write_data(z, w);
                self.cpu.r[au] = m;
            }
            op::POP => {
                self.cpu.sp = self.cpu.sp.wrapping_add(1);
                let sp = self.cpu.sp;
                self.cpu.r[au] = self.read_data(sp);
            }
            op::PUSH => {
                let (sp, v) = (self.cpu.sp, self.cpu.r[au]);
                self.write_data(sp, v);
                self.cpu.sp = sp.wrapping_sub(1);
                self.check_stack();
            }
            op::COM => {
                let res = !self.cpu.r[au];
                self.cpu.r[au] = res;
                let n = res >> 7;
                self.cpu.sreg = (self.cpu.sreg & 0xe0) | (n << 4) | (n << 2) | zf(res) | SREG_C;
            }
            op::NEG => {
                let d = self.cpu.r[au];
                let res = 0u8.wrapping_sub(d);
                self.cpu.r[au] = res;
                self.cpu.sreg = sub_flags(self.cpu.sreg, 0, d, res, false);
            }
            op::SWAP => self.cpu.r[au] = self.cpu.r[au].rotate_left(4),
            op::INC | op::DEC => {
                let res = if o == op::INC { self.cpu.r[au].wrapping_add(1) } else { self.cpu.r[au].wrapping_sub(1) };
                self.cpu.r[au] = res;
                let n = res >> 7;
                let v = (res == if o == op::INC { 0x80 } else { 0x7f }) as u8;
                self.cpu.sreg = (self.cpu.sreg & 0xe1) | ((n ^ v) << 4) | (v << 3) | (n << 2) | zf(res);
            }
            op::ASR | op::LSR | op::ROR => {
                let d = self.cpu.r[au];
                let res = match o {
                    op::ASR => (d >> 1) | (d & 0x80),
                    op::LSR => d >> 1,
                    _ => (d >> 1) | ((self.cpu.sreg & 1) << 7),
                };
                self.cpu.r[au] = res;
                let c = d & 1;
                let n = res >> 7;
                let v = n ^ c;
                self.cpu.sreg = (self.cpu.sreg & 0xe0) | ((n ^ v) << 4) | (v << 3) | (n << 2) | zf(res) | c;
            }
            op::BSET => {
                if a == 7 && self.cpu.sreg & SREG_I == 0 {
                    self.cpu.irq_inhibit = true;
                }
                self.cpu.sreg |= 1 << a;
            }
            op::BCLR => self.cpu.sreg &= !(1u8 << a),
            op::IJMP | op::EIJMP => next = self.ptr(30) as u32 & mask,
            op::DES => {
                let (c, pc2) = (self.cpu.cycles, pc * 2);
                self.sys.warn_key(c, "des", format!("DES instruction is not supported by the simulator (PC 0x{pc2:04X})"));
            }
            op::RET | op::RETI => {
                if o == op::RETI {
                    self.cpu.sreg |= SREG_I;
                    self.cpu.irq_inhibit = true;
                }
                self.cpu.pc = self.pop_pc();
                self.cpu.pop_frame();
                return;
            }
            op::SLEEP => {
                self.cpu.pc = next;
                let smcr = self.smcr.map(|a| self.cpu.data[a as usize]).unwrap_or(0);
                if smcr & 1 != 0 {
                    let mode = (smcr >> 1) & 7;
                    self.cpu.sleeping = true;
                    self.cpu.sleep_mode = mode;
                    self.broadcast(|p, cx| p.on_sleep(mode, cx));
                }
                return;
            }
            op::BREAK => {
                self.cpu.pc = next;
                self.request_stop(StopReason::BreakInsn);
                return;
            }
            op::WDR => self.broadcast(|p, cx| p.on_wdr(cx)),
            op::SPM | op::SPM_ZP => {
                let c = self.cpu.cycles;
                self.sys.warn_key(c, "spm", "SPM (self-programming) is not supported by the simulator yet");
            }
            op::ICALL | op::EICALL | op::RCALL | op::CALL => {
                let (ret, target) = match o {
                    op::RCALL => (next, (pc as i64 + 1 + a as i64) as u32 & mask),
                    op::CALL => ((pc + 2) & mask, a as u32 & mask),
                    _ => (next, self.ptr(30) as u32 & mask),
                };
                self.push_pc(ret);
                self.cpu.push_frame(ret, target, -1);
                next = target;
            }
            op::JMP => next = a as u32 & mask,
            op::RJMP => next = (pc as i64 + 1 + a as i64) as u32 & mask,
            op::ADIW | op::SBIW => {
                let (lo, hi) = (self.cpu.r[au], self.cpu.r[au + 1]);
                let val = lo as u16 | ((hi as u16) << 8);
                let res = if o == op::ADIW { val.wrapping_add(b as u16) } else { val.wrapping_sub(b as u16) };
                self.cpu.r[au] = res as u8;
                self.cpu.r[au + 1] = (res >> 8) as u8;
                let h7 = hi >> 7;
                let r15 = (res >> 15) as u8;
                let (v, c) = if o == op::ADIW { (!h7 & r15 & 1, !r15 & h7 & 1) } else { (h7 & !r15 & 1, r15 & !h7 & 1) };
                self.cpu.sreg = (self.cpu.sreg & 0xe0) | ((r15 ^ v) << 4) | (v << 3) | (r15 << 2) | if res == 0 { SREG_Z } else { 0 } | c;
            }
            op::CBI | op::SBI => {
                let addr = self.cpu.io_base + a as u16;
                let cur = self.rmw_read(addr);
                let v = if o == op::SBI { cur | (1 << b) } else { cur & !(1u8 << b) };
                self.write_data(addr, v);
            }
            op::SBIC | op::SBIS => {
                let addr = self.cpu.io_base + a as u16;
                let set = self.read_data(addr) & (1 << b) != 0;
                if set == (o == op::SBIS) {
                    self.skip();
                    return;
                }
            }
            op::IN => {
                let addr = self.cpu.io_base + b as u16;
                self.cpu.r[au] = self.read_data(addr);
            }
            op::OUT => {
                self.cpu.pc = next;
                let v = self.cpu.r[bu];
                self.write_data(self.cpu.io_base + a as u16, v);
                return;
            }
            op::LDI => self.cpu.r[au] = b as u8,
            op::BRBS | op::BRBC => {
                let set = self.cpu.sreg & (1 << a) != 0;
                if set == (o == op::BRBS) {
                    next = (pc as i64 + 1 + b as i64) as u32 & mask;
                    self.cpu.cycles += 1;
                }
            }
            op::BLD => {
                if self.cpu.sreg & SREG_T != 0 {
                    self.cpu.r[au] |= 1 << b;
                } else {
                    self.cpu.r[au] &= !(1u8 << b);
                }
            }
            op::BST => {
                if self.cpu.r[au] & (1 << b) != 0 {
                    self.cpu.sreg |= SREG_T;
                } else {
                    self.cpu.sreg &= !SREG_T;
                }
            }
            op::SBRC | op::SBRS => {
                let set = self.cpu.r[au] & (1 << b) != 0;
                if set == (o == op::SBRS) {
                    self.skip();
                    return;
                }
            }
            _ => {
                // INVALID (op 0): stop before the bad word.
                let erased = a == 0xffff;
                self.cpu.cycles -= 1;
                self.cpu.instructions -= 1;
                let c = self.cpu.cycles;
                let msg = format!(
                    "Invalid opcode 0x{:04X} at 0x{:04X}{}",
                    a,
                    pc * 2,
                    if erased { " (erased flash - did the program run off the end?)" } else { "" }
                );
                self.sys.message(c, "error", msg);
                self.request_stop(StopReason::InvalidOpcode);
                return;
            }
        }
        self.cpu.pc = next;
    }
}

#[inline(always)]
fn ptr_reg(o: u8) -> usize {
    match o {
        op::LD_XP | op::LD_MX | op::ST_XP | op::ST_MX => 26,
        op::LD_YP | op::LD_MY | op::ST_YP | op::ST_MY => 28,
        _ => 30,
    }
}

#[inline(always)]
fn zf(res: u8) -> u8 {
    if res == 0 { SREG_Z } else { 0 }
}

#[inline(always)]
fn add_flags(s: u8, d: u8, r: u8, res: u8) -> u8 {
    let c = (d & r) | (r & !res) | (!res & d);
    let v = (((d & r & !res) | (!d & !r & res)) >> 7) & 1;
    let n = res >> 7;
    (s & 0xc0) | if c & 0x08 != 0 { SREG_H } else { 0 } | ((n ^ v) << 4) | (v << 3) | (n << 2) | zf(res) | (c >> 7)
}

#[inline(always)]
fn sub_flags(s: u8, d: u8, r: u8, res: u8, keep_z: bool) -> u8 {
    let c = (!d & r) | (r & res) | (res & !d);
    let v = (((d & !r & !res) | (!d & r & res)) >> 7) & 1;
    let n = res >> 7;
    let z = if res == 0 { if keep_z { s & SREG_Z } else { SREG_Z } } else { 0 };
    (s & 0xc0) | if c & 0x08 != 0 { SREG_H } else { 0 } | ((n ^ v) << 4) | (v << 3) | (n << 2) | z | (c >> 7)
}

#[inline(always)]
fn logic_flags(s: u8, res: u8) -> u8 {
    let n = res >> 7;
    (s & 0xe1) | (n << 4) | (n << 2) | zf(res)
}
