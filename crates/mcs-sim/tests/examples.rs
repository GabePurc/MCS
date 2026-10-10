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
    let c: Command = serde_json::from_str(r#"{"type":"setSpeed","mode":"clock","factor":1}"#).unwrap();
    assert!(matches!(c, Command::SetSpeed { mode: mcs_sim::protocol::SpeedMode::Clock, .. }));
    let c: Command = serde_json::from_str(r#"{"type":"setClockConfig","source":2,"prescaleLog2":0}"#).unwrap();
    assert!(matches!(c, Command::SetClockConfig { source: 2, prescale_log2: 0 }));
    let c: Command = serde_json::from_str(r#"{"type":"setPinGenerator","pin":2,"gen":{"hz":1000,"duty":0.5,"invert":false}}"#).unwrap();
    assert!(matches!(c, Command::SetPinGenerator { pin: 2, gen: Some(_) }));
    let c: Command = serde_json::from_str(r#"{"type":"setPinGenerator","pin":2,"gen":null}"#).unwrap();
    assert!(matches!(c, Command::SetPinGenerator { gen: None, .. }));
    let c: Command = serde_json::from_str(r#"{"type":"setProfiling","enabled":true}"#).unwrap();
    assert!(matches!(c, Command::SetProfiling { enabled: true }));
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
    let pc = s.avr_machine().unwrap().cpu.pc;
    assert!(pc < 16, "pc {pc}");
    let json = serde_json::to_string(&s.handle(Command::RequestState)).unwrap();
    assert!(json.contains("\"traceCycles\""));
}

#[test]
fn machine_code_blink_matches_assembly() {
    let path = format!("{}/../../examples/blink.mc", env!("CARGO_MANIFEST_DIR"));
    let src = std::fs::read_to_string(&path).unwrap();
    let r = mcs_asm::assemble_machine_code(&src, "blink.mc", "attiny10");
    assert!(r.ok && r.diagnostics.is_empty(), "{:#?}", r.diagnostics);
    let asm = load("blink.asm");
    assert_eq!(r.program.flash, asm.cpu.flash);
    let mut m = Machine::new(devices::get("attiny10").unwrap());
    m.load(&r.program);
    m.run(1_000_000);
    assert!(edges(&m, 0).len() >= 18);
}

fn load_on(name: &str) -> Machine {
    let path = format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"));
    let src = std::fs::read_to_string(&path).unwrap();
    // The device comes from the source's .include, whatever the default is.
    let r = assemble(&src, &AssembleOptions::new(name, "attiny10"));
    assert!(r.ok, "{name}: {:#?}", r.diagnostics);
    let mut m = Machine::new(devices::get(&r.device_id).unwrap());
    m.load(&r.program);
    m
}

#[test]
fn m328p_blink_toggles_every_half_second() {
    let mut m = load_on("m328p_blink.asm");
    assert_eq!(m.spec.id, "atmega328p");
    m.run(2_100_000);
    let e = edges(&m, 5);
    assert!(e.len() >= 4, "{e:?}");
    let period = e[2] - e[1];
    assert!((499_000..=501_000).contains(&period), "{period}");
    // The CPU sleeps (idle) between the timer interrupts.
    assert!(m.cpu.instructions < 1000, "{}", m.cpu.instructions);
}

#[test]
fn t85_pwm_follows_the_potentiometer() {
    let mut m = load_on("t85_pwm.asm");
    assert_eq!(m.spec.id, "attiny85");
    let duty = |m: &mut Machine, volts: f64| {
        m.set_pin_input(2, ExtDrive::Analog, volts);
        let c = m.cpu.cycles;
        m.run(c + 20_000);
        // 15.6 kHz PWM (64 cycles per period): measure the high fraction over the last 2048 cycles.
        let (_, cy, l) = m.sys.trace.read_since(0, usize::MAX);
        let end = m.cpu.cycles;
        let start = end - 2048;
        let mut high = 0u64;
        let mut level = 0;
        let mut t = start;
        for i in 0..cy.len() {
            if cy[i] <= start {
                level = (l[i] >> 1) & 1;
                continue;
            }
            if level == 1 {
                high += cy[i].min(end) - t;
            }
            t = cy[i].min(end);
            level = (l[i] >> 1) & 1;
        }
        if level == 1 {
            high += end - t;
        }
        high as f64 / 2048.0
    };
    let d1 = duty(&mut m, 1.25);
    let d2 = duty(&mut m, 3.75);
    assert!((0.2..0.3).contains(&d1), "{d1}");
    assert!((0.7..0.8).contains(&d2), "{d2}");
}

// ------------------------------------------------------------------ ESP32-C3 (prebuilt ELF examples)

mod esp32c3_examples {
    use mcs_core::riscv::devices;
    use mcs_sim::riscv::Esp32c3;
    use mcs_sim::target::{Sent, StopReason, Target};

    const IROM: u32 = 0x4200_0000;

    fn boot(name: &str) -> Esp32c3 {
        let bytes = std::fs::read(format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let spec = devices::get("esp32-c3").unwrap();
        let prog = mcs_formats::parse_elf_at(&bytes, spec.flash_size as usize, name, Some(IROM));
        assert!(!prog.has_errors(), "{name}: {:?}", prog.diagnostics);
        let mut m = Esp32c3::from_spec(spec);
        m.load_program(Some(&prog));
        m
    }

    #[test]
    fn blink_toggles_gpio2_every_250ms() {
        let mut m = boot("esp32c3_blink.elf");
        // 20 MHz reset clock: 1.25 s = 25M cycles = five 250 ms half periods.
        assert_eq!(m.run_until(26_000_000), StopReason::Limit);
        let s = m.snapshot(&mut Sent { trace: 0, eeprom: u64::MAX }, false, 1 << 20);
        let w = s.trace_words as usize;
        let mut edges = Vec::new();
        let mut last = None;
        for (i, &c) in s.trace_cycles.iter().enumerate() {
            let lv = s.trace_levels[i * w] >> 2 & 1;
            if last != Some(lv) {
                edges.push(c);
                last = Some(lv);
            }
        }
        // The first few cycles settle the pad (pull-up, output enable, first W1TS): look at the steady state.
        edges.retain(|&c| c > 100);
        assert!(edges.len() >= 4, "{edges:?}");
        for pair in edges.windows(2) {
            let d = pair[1] - pair[0];
            assert!((4_990_000..5_010_000).contains(&d), "half period {d} cycles: {edges:?}");
        }
    }

    #[test]
    fn hello_prints_the_greeting_and_echoes_input() {
        let mut m = boot("esp32c3_hello.elf");
        let mut sent = Sent { trace: 0, eeprom: u64::MAX };
        let mut serial = Vec::new();
        // The greeting is 64 characters at 115200 baud: about 5.6 ms = 112k cycles.
        m.run_until(200_000);
        serial.extend(m.snapshot(&mut sent, false, 1 << 20).serial);
        let text = String::from_utf8_lossy(&serial).into_owned();
        assert!(text.starts_with("Hello from the ESP32-C3!\r\n") && text.ends_with("echoed back.\r\n"), "{text:?}");
        m.serial_send(b"ok");
        m.run_until(400_000);
        serial.extend(m.snapshot(&mut sent, false, 1 << 20).serial);
        assert!(serial.ends_with(b"ok"), "{:?}", String::from_utf8_lossy(&serial));
    }
}
