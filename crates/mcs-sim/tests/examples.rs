//! Assembles every bundled example program and checks it behaves as described.

use mcs_asm::{assemble, AssembleOptions};
use mcs_core::avr::devices;
use mcs_sim::avr::Machine;
use mcs_sim::pins::ExtDrive;

fn load(name: &str) -> Machine {
    let path = format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"));
    let src = std::fs::read_to_string(&path).unwrap();
    let r = assemble(&src, &AssembleOptions::new(name, "attiny10"));
    assert!(r.ok, "{name}: {:#?}", r.diagnostics);
    let mut m = Machine::new(devices::get(&r.device_id).unwrap());
    m.load(&r.program);
    m
}

/// Times (cycles) at which PB`bit` changed level.
fn edges(m: &Machine, bit: u32) -> Vec<u64> {
    let (_, c, l) = m.sys.trace.read_since(0, usize::MAX);
    (1..c.len()).filter(|&i| (l[i] ^ l[i - 1]) >> bit & 1 == 1).map(|i| c[i]).collect()
}

#[test]
fn blink_toggles_every_50ms() {
    let mut m = load("blink.asm");
    m.run(1_000_000);
    let e = edges(&m, 0);
    assert!(e.len() >= 18, "{} edges", e.len());
    let period = e[5] - e[4];
    assert!((48_000..52_000).contains(&period), "period {period}");
}

#[test]
fn pwm_fade_drives_oc0a() {
    let mut m = load("pwm_fade.asm");
    m.run(300_000);
    assert!(edges(&m, 0).len() > 500);
}

#[test]
fn button_interrupt_toggles_led() {
    let mut m = load("button_interrupt.asm");
    m.run(10_000);
    assert_eq!(m.sys.pins[0].level, 0);
    m.set_pin_input(2, ExtDrive::Low, 0.0);
    m.run(20_000);
    assert_eq!(m.sys.pins[0].level, 1);
    m.set_pin_input(2, ExtDrive::High, 0.0);
    m.run(30_000);
    m.set_pin_input(2, ExtDrive::Low, 0.0);
    m.run(40_000);
    assert_eq!(m.sys.pins[0].level, 0);
    assert!(m.cpu.instructions < 200, "CPU should sleep between presses");
}

#[test]
fn adc_sets_pwm_duty() {
    let mut m = load("adc_to_pwm.asm");
    m.set_pin_input(2, ExtDrive::Analog, 1.25);
    m.run(200_000);
    let e = edges(&m, 0);
    let n = e.len();
    assert!(n > 20);
    // fast PWM 8-bit: high for OCR0A+1 of 256 ticks; 1.25 V of 5 V -> ADC 64 -> ~25 %
    let (rise, fall) = if m.sys.pins[0].level == 1 { (e[n - 3], e[n - 2]) } else { (e[n - 2], e[n - 1]) };
    let high = fall - rise;
    assert!((60..=70).contains(&high), "high time {high}");
}

#[test]
fn watchdog_wakes_from_power_down() {
    let mut m = load("watchdog_sleep.asm");
    m.run(2_100_000);
    let e = edges(&m, 0);
    assert_eq!(e.len(), 4, "{e:?}");
    let period = e[1] - e[0];
    assert!((510_000..515_000).contains(&period), "period {period}");
    assert!(m.cpu.instructions < 100);
}

#[test]
fn session_commands_deserialize_from_ui_json() {
    use mcs_sim::protocol::Command;
    let c: Command = serde_json::from_str(r#"{"type":"setPin","pin":2,"ext":"analog","volts":1.5}"#).unwrap();
    assert!(matches!(c, Command::SetPin { pin: 2, .. }));
    let c: Command = serde_json::from_str(r#"{"type":"step","kind":"over","source":true}"#).unwrap();
    assert!(matches!(c, Command::Step { .. }));
    let c: Command = serde_json::from_str(r#"{"type":"writeCpu","field":"pc","value":4}"#).unwrap();
    assert!(matches!(c, Command::WriteCpu { .. }));
}

#[test]
fn session_load_run_and_step() {
    use mcs_sim::protocol::{Command, Output, StepKind};
    use mcs_sim::session::Session;
    let src = std::fs::read_to_string(format!("{}/../../examples/blink.asm", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let r = assemble(&src, &AssembleOptions::new("blink.asm", "attiny10"));
    let mut s = Session::new();
    let out = s.handle(Command::Load { device_id: "attiny10".into(), program: Box::new(r.program) });
    assert!(out.iter().any(|o| matches!(o, Output::Device { .. })));
    assert!(out.iter().any(|o| matches!(o, Output::State { state } if state.stop.is_some())));
    // Source-level step over the rcall to delay must stop on the next line.
    for _ in 0..8 {
        s.handle(Command::Step { kind: StepKind::Over, source: true });
        for _ in 0..1000 {
            if !s.is_running() {
                break;
            }
            s.slice();
        }
    }
    let pc = s.machine().unwrap().cpu.pc;
    assert!(pc < 16, "pc {pc}");
    let json = serde_json::to_string(&s.handle(Command::RequestState)).unwrap();
    assert!(json.contains("\"traceCycles\""));
}
