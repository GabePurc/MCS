//! Test-bench features used by the UI: external clock selection, debugger clock changes,
//! pin signal generators, execution profiling and the fixed-rate speed mode.

use mcs_core::avr::devices;
use mcs_core::avr::isa::{def_by_op, encode, op};
use mcs_core::program::{LoadedProgram, ProgramFormat};
use mcs_sim::avr::Machine;
use mcs_sim::pins::{ExtDrive, PinGenerator};
use mcs_sim::protocol::{Command, Output, SpeedMode};
use mcs_sim::session::Session;

fn tiny10(insns: &[(u8, &[i32])]) -> (Machine, LoadedProgram) {
    let spec = devices::get("attiny10").unwrap();
    let mut p = LoadedProgram::empty(ProgramFormat::Asm, spec.flash_size as usize);
    let mut w = 0;
    for (o, vals) in insns {
        for word in encode(def_by_op(*o).unwrap(), vals).unwrap() {
            p.flash[w * 2] = word as u8;
            p.flash[w * 2 + 1] = (word >> 8) as u8;
            w += 1;
        }
    }
    let mut m = Machine::new(spec);
    m.load(&p);
    (m, p)
}

fn edges(m: &Machine, bit: u32) -> Vec<u64> {
    let (_, c, l) = m.sys.trace.read_since(0, usize::MAX);
    (1..c.len()).filter(|&i| (l[i] ^ l[i - 1]) >> bit & 1 == 1).map(|i| c[i]).collect()
}

const LOOP: &[(u8, &[i32])] = &[(op::RJMP, &[-1])];

#[test]
fn external_clock_applies_when_selected_and_when_changed() {
    let (mut m, _) = tiny10(LOOP);
    assert_eq!(m.sys.clock.hz, 1e6); // 8 MHz RC / 8
    // Changing the external frequency while the RC oscillator is selected changes nothing.
    m.set_external_clock(4e6);
    assert_eq!(m.sys.clock.hz, 1e6);
    // Debugger selects the external clock, prescaler /1.
    m.debug_set_clock(2, 0);
    assert_eq!(m.sys.clock.hz, 4e6);
    // A new external frequency takes effect immediately while it is the clock source.
    m.set_external_clock(16e6);
    assert_eq!(m.sys.clock.hz, 16e6);
    m.debug_set_clock(0, 3);
    assert_eq!(m.sys.clock.hz, 1e6);
    m.debug_set_clock(1, 0);
    assert_eq!(m.sys.clock.hz, 128e3);
}

#[test]
fn square_wave_generator_keeps_its_frequency() {
    let (mut m, _) = tiny10(LOOP);
    m.set_pin_generator(2, Some(PinGenerator { hz: 1000.0, duty: 0.25, count: None, invert: false }));
    m.run(10_000); // 10 ms at 1 MHz
    let e = edges(&m, 2);
    assert!(e.len() >= 19, "{} edges", e.len());
    // Starts high at cycle 0: high for 250 us, low for 750 us.
    assert_eq!(e[0], 0);
    assert_eq!(e[1] - e[0], 250);
    assert_eq!(e[2] - e[1], 750);
    // Clock doubles: the generator stays at 1 kHz (2000 cycles per period).
    m.debug_set_clock(0, 2);
    let before = edges(&m, 2).len();
    m.run(m.cpu.cycles + 20_000);
    let e = edges(&m, 2);
    let n = e.len();
    assert!(n > before + 10);
    assert_eq!(e[n - 1] - e[n - 3], 2000);
    // Manual drive replaces the generator.
    m.set_pin_input(2, ExtDrive::Low, 0.0);
    assert!(m.sys.pins[2].gen.is_none());
    let n = edges(&m, 2).len();
    m.run(m.cpu.cycles + 10_000);
    assert_eq!(edges(&m, 2).len(), n);
}

#[test]
fn pulse_burst_stops_at_idle_level() {
    let (mut m, _) = tiny10(LOOP);
    m.set_pin_input(1, ExtDrive::High, 0.0);
    m.set_pin_generator(1, Some(PinGenerator { hz: 10_000.0, duty: 0.5, count: Some(3), invert: true }));
    m.run(5_000);
    // Rise from the manual drive, then 3 low pulses of 50 us.
    assert_eq!(edges(&m, 1), [0, 0, 50, 100, 150, 200, 250]);
    assert_eq!(m.sys.pins[1].level, 1);
    assert!(m.sys.pins[1].gen.is_none());
}

#[test]
fn generator_clocks_timer0_external_input() {
    // TCCR0B = 7: count rising edges on T0 (PB2).
    let (mut m, _) = tiny10(&[(op::LDI, &[16, 7]), (op::OUT, &[0x2d, 16]), (op::RJMP, &[-1])]);
    m.set_pin_generator(2, Some(PinGenerator { hz: 10_000.0, duty: 0.5, count: None, invert: false }));
    m.run(10_000);
    let tcnt = m.peek_data(0x28) as u32 | (m.peek_data(0x29) as u32) << 8;
    assert!((99..=101).contains(&tcnt), "TCNT0 = {tcnt}");
}

#[test]
fn profiling_counts_executed_words() {
    let (mut m, _) = tiny10(&[(op::NOP, &[]), (op::RJMP, &[-1])]);
    m.run(100);
    assert!(m.take_exec_counts().is_empty());
    m.set_profiling(true);
    m.run(m.cpu.cycles + 200);
    let c = m.take_exec_counts();
    assert_eq!(c.len(), 512);
    assert!(c[1] >= 99 && c[1] <= 101, "{}", c[1]);
    assert_eq!(c[2], 0);
    assert!(m.take_exec_counts().iter().all(|&n| n == 0));
}

#[test]
fn fixed_rate_speed_mode_runs_slowly() {
    let (_, p) = tiny10(LOOP);
    let mut s = Session::new();
    s.handle(Command::Load { device_id: "attiny10".into(), program: Box::new(p) });
    s.handle(Command::SetSpeed { mode: SpeedMode::Clock, factor: 50.0 });
    assert!(s.idle_ms() >= 3_600_000.0);
    s.handle(Command::Run);
    assert!((19.0..=21.0).contains(&s.idle_ms()));
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_millis() < 200 {
        s.slice();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let out = s.handle(Command::Pause);
    let Some(Output::State { state }) = out.into_iter().find(|o| matches!(o, Output::State { .. })) else { panic!("no state") };
    // ~10 cycles in 200 ms at 50 Hz (generous bounds for slow CI machines).
    assert!(state.cycles >= 4 && state.cycles <= 40, "{} cycles", state.cycles);
}
