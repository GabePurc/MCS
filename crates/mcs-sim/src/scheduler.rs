//! Cycle-stamped event scheduler.
//!
//! Peripherals schedule events at the exact CPU cycle where something observable happens
//! (timer compare match, ADC conversion done, watchdog timeout...) instead of being ticked every
//! cycle. The CPU loop only compares `cycles >= next` per instruction. The number of
//! simultaneously active events is tiny, so a flat vector with a cached minimum beats a heap.

/// Identifies an event: the owning peripheral index and a peripheral-defined tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventKey {
    pub owner: u8,
    pub tag: u8,
}

#[derive(Default)]
pub struct Scheduler {
    events: Vec<(EventKey, u64)>,
    /// Cycle of the earliest event (u64::MAX when none).
    pub next: u64,
}

impl Scheduler {
    pub fn new() -> Self {
        Self { events: Vec::with_capacity(16), next: u64::MAX }
    }

    /// Schedules (or re-schedules) `key` at `cycle`.
    pub fn at(&mut self, key: EventKey, cycle: u64) {
        if let Some(e) = self.events.iter_mut().find(|e| e.0 == key) {
            e.1 = cycle;
            self.recompute();
        } else {
            self.events.push((key, cycle));
            self.next = self.next.min(cycle);
        }
    }

    pub fn cancel(&mut self, key: EventKey) {
        if let Some(i) = self.events.iter().position(|e| e.0 == key) {
            let c = self.events.swap_remove(i).1;
            if c <= self.next {
                self.recompute();
            }
        }
    }

    pub fn is_scheduled(&self, key: EventKey) -> Option<u64> {
        self.events.iter().find(|e| e.0 == key).map(|e| e.1)
    }

    /// Removes and returns the earliest event due at or before `now`.
    pub fn pop_due(&mut self, now: u64) -> Option<(EventKey, u64)> {
        if self.next > now {
            return None;
        }
        let (i, _) = self.events.iter().enumerate().min_by_key(|(_, e)| e.1)?;
        let e = self.events.swap_remove(i);
        self.recompute();
        Some(e)
    }

    pub fn clear(&mut self) {
        self.events.clear();
        self.next = u64::MAX;
    }

    fn recompute(&mut self) {
        self.next = self.events.iter().map(|e| e.1).min().unwrap_or(u64::MAX);
    }
}
