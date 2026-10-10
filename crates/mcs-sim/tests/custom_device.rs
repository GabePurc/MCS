//! User-defined (custom) microcontrollers: every shape builds a machine, and a program on a
//! generated device drives a port pin and a second USART.

use mcs_asm::{assemble, AssembleOptions};
use mcs_core::avr::devices::{self, CustomMcuConfig};
use mcs_sim::avr::peripherals::serial::SerialConfig;
use mcs_sim::avr::Machine;

fn gpio(m: &Machine, func: &str) -> usize {
    m.spec.pins.iter().find(|p| p.functions.iter().any(|f| f == func)).and_then(|p| p.gpio).unwrap() as usize
}

#[test]
fn every_shape_builds_and_resets() {
    for cfg in [CustomMcuConfig::tiny(), CustomMcuConfig::default(), CustomMcuConfig::huge()] {
        let spec = devices::register_custom(&cfg).unwrap();
        let mut m = Machine::new(spec);
        m.run(100);
        assert_eq!(m.cpu.sp, spec.ram_end(), "{}: SP resets to RAMEND", spec.name);
        m.reset(mcs_sim::avr::ResetSource::External);
        assert!(m.sys.pins.len() == spec.gpio_count as usize);
    }
}

#[test]
fn port_toggle_and_second_usart_transmit() {
    let cfg = CustomMcuConfig { id: "custom-twouart".into(), usarts: 2, ..CustomMcuConfig::default() };
    let spec = devices::register_custom(&cfg).unwrap();
    let src = ".include \"custom-twouartdef.inc\"\n.org 0\n jmp reset\n.org INT_VECTORS_SIZE\nreset:\n\
         sbi DDRA, 0\n\
         ldi r16, 0\n sts UBRR1H, r16\n ldi r16, 12\n sts UBRR1L, r16\n ldi r16, 1<<U2X1\n sts UCSR1A, r16\n\
         ldi r16, 1<<TXEN1\n sts UCSR1B, r16\n ldi r16, 3<<UCSZ10\n sts UCSR1C, r16\n\
         ldi ZL, low(msg*2)\n ldi ZH, high(msg*2)\n\
         send:\n lpm r17, Z+\n tst r17\n breq done\n\
         w1: lds r16, UCSR1A\n sbrs r16, UDRE1\n rjmp w1\n sts UDR1, r17\n rjmp send\n\
         done:\n sbi PINA, 0\n rjmp done\n\
         msg: .db \"Hi2\", 0\n";
    let r = assemble(src, &AssembleOptions::new("t.asm", "custom-twouart"));
    assert!(r.ok, "{:#?}", r.diagnostics);
    let mut m = Machine::new(spec);
    m.load(&r.program);
    let tx = gpio(&m, "TXD1");
    m.set_serial(SerialConfig { monitor: Some(tx), inject: None, baud: 9600.0, data_bits: 8, parity: 0, stop_bits: 1 });
    m.run(100_000);
    assert_eq!(m.sys.serial_out, b"Hi2");
    let (_, c, l) = m.sys.trace.read_since(0, usize::MAX);
    let pa0 = gpio(&m, "ADC0");
    assert_eq!(pa0, 0, "ADC0 sits on PA0");
    assert!((1..c.len()).any(|i| (l[i] ^ l[i - 1]) & 1 == 1), "PA0 toggled");
}

/// Peripheral counts have no fixed cap: only the vector table and the data space limit them.
#[test]
fn many_instances_build_until_the_vector_table_is_full() {
    let many = CustomMcuConfig {
        id: "custom-many".into(),
        name: "Many".into(),
        ports: 12,
        ext_interrupts: 40,
        timers8: 12,
        timers16: 12,
        usarts: 12,
        spis: 10,
        twis: 10,
        ..CustomMcuConfig::default()
    };
    let spec = devices::register_custom(&many).unwrap();
    assert!(spec.register("UDR11").is_some() && spec.register("TWBR9").is_some());
    let mut m = Machine::new(spec);
    m.run(100);
    assert_eq!(m.cpu.sp, spec.ram_end());
    let too_many = CustomMcuConfig { id: "custom-too-many".into(), usarts: 90, ..many };
    let err = devices::register_custom(&too_many).unwrap_err();
    assert!(err.contains("255") || err.to_lowercase().contains("vector"), "{err}");
}
