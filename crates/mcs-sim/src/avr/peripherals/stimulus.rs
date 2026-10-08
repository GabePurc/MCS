//! Test-bench signal generators (outside the MCU): square waves and pulse bursts driven onto
//! GPIO pins from the Pins & Stimulus window. Event driven like the on-chip peripherals: one
//! scheduled event per edge. Edges are timed in seconds, so a generator keeps its frequency when
//! the firmware changes the CPU clock.

use crate::avr::machine::{Cx, Peripheral, ResetSource};
use crate::pins::{ExtDrive, PinGenerator};

#[derive(Clone, Copy)]
struct Running {
    /// Simulated time (s) of the next edge.
    next_edge: f64,
    /// The next edge switches to the active level.
    next_active: bool,
    /// Active pulses still to start (None = continuous).
    remaining: Option<u32>,
}

pub struct Stimulus {
    run: Vec<Option<Running>>,
}

impl Stimulus {
    pub fn new(pins: usize) -> Self {
        Self { run: vec![None; pins] }
    }

    /// Starts (Some) or stops (None) the generator on GPIO `pin`.
    pub fn set(&mut self, pin: usize, gen: Option<PinGenerator>, cx: &mut Cx) {
        if pin >= self.run.len() {
            return;
        }
        let gen = gen.map(PinGenerator::sanitized);
        cx.sys.pins[pin].gen = gen;
        match gen {
            Some(g) => self.start(pin, g, cx),
            None => {
                self.run[pin] = None;
                cx.cancel(pin as u8);
            }
        }
    }

    /// The first period (active level) begins now.
    fn start(&mut self, pin: usize, g: PinGenerator, cx: &mut Cx) {
        let now = cx.time_seconds();
        self.run[pin] = Some(Running { next_edge: now, next_active: true, remaining: g.count.map(|n| n.max(1)) });
        let c = cx.now();
        self.fire(pin, c, cx);
    }

    /// Applies the pending edge of `pin` and schedules the following one.
    fn fire(&mut self, pin: usize, cycle: u64, cx: &mut Cx) {
        let (Some(mut r), Some(g)) = (self.run[pin], cx.sys.pins[pin].gen) else { return };
        drive(pin, r.next_active != g.invert, cx, cycle);
        if r.next_active {
            r.next_edge += g.active_s();
            r.next_active = false;
        } else {
            if let Some(n) = r.remaining.as_mut() {
                *n -= 1;
                if *n == 0 {
                    // Burst complete: the pin stays at the idle level.
                    self.run[pin] = None;
                    cx.sys.pins[pin].gen = None;
                    return;
                }
            }
            r.next_edge += g.idle_s();
            r.next_active = true;
        }
        self.run[pin] = Some(r);
        self.schedule(pin, cx);
    }

    fn schedule(&mut self, pin: usize, cx: &mut Cx) {
        if let Some(r) = self.run[pin] {
            // Edges closer than one CPU cycle collapse onto the next cycle.
            let at = cx.sys.clock.cycle_at(r.next_edge).max(cx.now() + 1);
            cx.schedule(pin as u8, at);
        }
    }
}

fn drive(pin: usize, high: bool, cx: &mut Cx, cycle: u64) {
    let p = &mut cx.sys.pins[pin];
    let ext = if high { ExtDrive::High } else { ExtDrive::Low };
    if p.ext != ext {
        p.ext = ext;
        cx.sys.update_pin(pin, cycle);
    }
}

impl Peripheral for Stimulus {
    fn name(&self) -> &str {
        "STIMULUS"
    }

    fn on_event(&mut self, tag: u8, cycle: u64, cx: &mut Cx) {
        self.fire(tag as usize, cycle, cx);
    }

    fn on_clock_change(&mut self, cx: &mut Cx) {
        for pin in 0..self.run.len() {
            self.schedule(pin, cx);
        }
    }

    fn reset(&mut self, cx: &mut Cx) {
        // External equipment keeps running across MCU resets; after a power cycle (time
        // restarts at 0) the generators restart from the beginning.
        let power_on = cx.sys.last_reset == ResetSource::PowerOn;
        for pin in 0..self.run.len() {
            match cx.sys.pins[pin].gen {
                Some(g) if power_on || self.run[pin].is_none() => self.start(pin, g, cx),
                Some(_) => self.schedule(pin, cx),
                None => self.run[pin] = None,
            }
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
