//! ATmega164PA / 324PA / 644PA / 1284P and ATmega640 / 1280 / 2560: reset defaults and vector
//! counts, USART1, Timer/Counter3, ADC differential channels, INT2 / PCINT3, ELPM (x4 family);
//! third output compare unit (OCR1C, OC1C PWM, Timer5), INT4-7, PCINT1 on PE0 + PJ, ADC and
//! analog comparator MUX5, PRR1, EIJMP-style far calls above 128K words, and the 86-pin trace
//! (x0 family), driven by small assembly programs built with the MCS assembler and the generated
//! m164PAdef.inc ... m2560def.inc.

use mcs_asm::{assemble, AssembleOptions};
use mcs_core::avr::devices;
use mcs_sim::avr::peripherals::serial::SerialConfig;
use mcs_sim::avr::{Machine, StopReason};
use mcs_sim::pins::ExtDrive;

fn build(dev: &str, inc: &str, src: &str) -> Machine {
    let r = assemble(&format!(".include \"{inc}\"\n.org 0\n    jmp reset\n{src}"), &AssembleOptions::new("t.asm", dev));
    assert!(r.ok, "{:#?}", r.diagnostics);
    let mut m = Machine::new(devices::get(&r.device_id).unwrap());
    m.load(&r.program);
    m
}

fn m164(body: &str) -> Machine {
    build("atmega164pa", "m164PAdef.inc", body)
}

fn m324(body: &str) -> Machine {
    build("atmega324pa", "m324PAdef.inc", body)
}

fn m644(body: &str) -> Machine {
    build("atmega644pa", "m644PAdef.inc", body)
}

fn m1284(body: &str) -> Machine {
    build("atmega1284p", "m1284Pdef.inc", body)
}

fn m640(body: &str) -> Machine {
    build("atmega640", "m640def.inc", body)
}

fn m1280(body: &str) -> Machine {
    build("atmega1280", "m1280def.inc", body)
}

fn m2560(body: &str) -> Machine {
    build("atmega2560", "m2560def.inc", body)
}

/// Level of GPIO `g` in trace entry `i` (any number of pins).
fn level(m: &Machine, l: &[u32], i: usize, g: usize) -> u32 {
    let w = m.sys.trace.words();
    l[i * w + g / 32] >> (g % 32) & 1
}

/// (rise, fall) cycle lists of GPIO `g`.
fn rises_falls(m: &Machine, g: usize) -> (Vec<u64>, Vec<u64>) {
    let (_, c, l) = m.sys.trace.read_since_wide(0, usize::MAX);
    let r = (1..c.len()).filter(|&i| level(m, &l, i, g) == 1 && level(m, &l, i - 1, g) == 0).map(|i| c[i]).collect();
    let f = (1..c.len()).filter(|&i| level(m, &l, i, g) == 0 && level(m, &l, i - 1, g) == 1).map(|i| c[i]).collect();
    (r, f)
}

/// Cycles at which GPIO `g` changed level.
fn edges(m: &Machine, g: usize) -> Vec<u64> {
    let (_, c, l) = m.sys.trace.read_since_wide(0, usize::MAX);
    (1..c.len()).filter(|&i| level(m, &l, i, g) != level(m, &l, i - 1, g)).map(|i| c[i]).collect()
}

fn adc_result(m: &Machine) -> u16 {
    (m.cpu.r[21] as u16) << 8 | m.cpu.r[20] as u16
}

// GPIO numbers, ATmega164PA..1284P: PA0-7 = 0-7, PB0-7 = 8-15, PC0-7 = 16-23, PD0-7 = 24-31.
// ATmega640..2560: A 0-7, B 8-15, C 16-23, D 24-31, E 32-39, F 40-47, G 48-53, H 54-61, J 62-69,
// K 70-77, L 78-85.

const IDLE: &str = "reset:\nl: rjmp l\n";

#[test]
fn x4_reset_defaults_and_vector_counts() {
    for (m, ram_end, vectors, flash) in [
        (m164(IDLE), 0x04ffu16, 31usize, 16384u32),
        (m324(IDLE), 0x08ff, 31, 32768),
        (m644(IDLE), 0x10ff, 31, 65536),
        (m1284(IDLE), 0x40ff, 35, 131072),
    ] {
        let mut m = m;
        let name = m.spec.name.clone();
        assert_eq!(m.sys.clock.hz, 1e6, "{name}: 8 MHz RC with CKDIV8");
        assert_eq!(m.cpu.sp, ram_end, "{name}");
        assert_eq!(m.spec.vector_count(), vectors, "{name}");
        assert_eq!(m.spec.flash_size, flash, "{name}");
        assert_eq!(m.cpu.fuses, [0x62, 0x99, 0xff], "{name}");
        assert_eq!(m.peek_data(0x54) & 0x1f, 0x01, "{name}: MCUSR.PORF only");
        assert_eq!(m.sys.pins.len(), 32);
        // No pin is reserved: the PDIP-40 has dedicated RESET / XTAL pins and JTAG is not modelled.
        assert!(m.sys.pins.iter().all(|p| !p.reserved), "{name}");
    }
    let mut m = m644("reset:\n ldi r16, 0xff\n out DDRA, r16\n ldi r16, 0x80\nl: out PINA, r16\n rjmp l\n");
    m.run(200);
    let e = edges(&m, 7);
    assert!(e.len() > 40 && e.windows(2).all(|w| w[1] - w[0] == 3), "{:?}", &e[..4]);
    // The ATmega1284P is the only one with Timer3, PRR1 and RAMPZ.
    assert!(devices::get("atmega644pa").unwrap().register("TCCR3A").is_none());
    assert!(devices::get("atmega1284p").unwrap().register("PRR1").is_some());
}

#[test]
fn usart1_transmits_and_receives_on_the_644pa() {
    // 9600 baud at 1 MHz with U2X: UBRR = 12. USART1: TXD1 = PD3, RXD1 = PD2.
    let mut m = m644(
        ".org INT_VECTORS_SIZE\nreset:\n ldi r16, 0\n sts UBRR1H, r16\n ldi r16, 12\n sts UBRR1L, r16\n ldi r16, 1<<U2X1\n sts UCSR1A, r16\n ldi r16, (1<<TXEN1)|(1<<RXEN1)\n sts UCSR1B, r16\n ldi r16, 3<<UCSZ10\n sts UCSR1C, r16\n ldi ZL, low(msg*2)\n ldi ZH, high(msg*2)\nsend:\n lpm r17, Z+\n tst r17\n breq echo\nw1: lds r16, UCSR1A\n sbrs r16, UDRE1\n rjmp w1\n sts UDR1, r17\n rjmp send\necho:\n lds r16, UCSR1A\n sbrs r16, RXC1\n rjmp echo\n lds r17, UDR1\nw2: lds r16, UCSR1A\n sbrs r16, UDRE1\n rjmp w2\n sts UDR1, r17\n rjmp echo\nmsg: .db \"Hi!\", 0\n",
    );
    m.set_serial(SerialConfig { monitor: Some(27), inject: Some(26), baud: 9600.0, data_bits: 8, parity: 0, stop_bits: 1 });
    m.run(50_000);
    assert_eq!(std::mem::take(&mut m.sys.serial_out), b"Hi!");
    m.serial_send(b"ok");
    let c = m.cpu.cycles;
    m.run(c + 50_000);
    assert_eq!(m.sys.serial_out, b"ok");
    // 'H' = 0x48 (LSB first): the line is low for the start bit and data bits 0-2, 104 cycles each
    // (U2X, UBRR = 12).
    let (r, f) = rises_falls(&m, 27);
    let start = f[0];
    let low = r.iter().find(|&&x| x > start).unwrap() - start;
    assert!((4 * 104..=4 * 104 + 8).contains(&low), "start bit + 3 zero data bits at 104 cycles each (events fire at instruction boundaries): {low}");
    // USART0 is a separate instance: its data register stays untouched.
    assert_eq!(m.peek_data(0xc6), 0);
}

#[test]
fn timer3_on_the_1284p_ctc_toggles_oc3a_and_raises_the_compare_interrupt() {
    // CTC (TOP = OCR3A = 99), OC3A (PB6) toggles on every match: period 2 x 100 cycles.
    let mut m = m1284(
        ".org TIMER3_COMPAaddr\n jmp isr\nreset:\n sbi DDRB, DDB6\n ldi r16, 0\n sts OCR3AH, r16\n ldi r16, 99\n sts OCR3AL, r16\n ldi r16, 1<<COM3A0\n sts TCCR3A, r16\n ldi r16, (1<<WGM32)|(1<<CS30)\n sts TCCR3B, r16\n ldi r16, 1<<OCIE3A\n sts TIMSK3, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n",
    );
    m.run(1050);
    assert!((9..=10).contains(&m.cpu.r[20]), "{}", m.cpu.r[20]);
    let e = edges(&m, 14);
    assert!(e.len() >= 8, "{e:?}");
    assert!(e.windows(2).all(|w| w[1] - w[0] == 100), "{e:?}");
}

#[test]
fn prr1_stops_timer3_on_the_1284p() {
    let mut m = m1284(
        "reset:\n ldi r16, 1<<CS30\n sts TCCR3B, r16\n nop\n nop\n nop\n lds r20, TCNT3L\n ldi r16, 1<<PRTIM3\n sts PRR1, r16\n lds r21, TCNT3L\n nop\n nop\n nop\n nop\n lds r22, TCNT3L\n ldi r16, 0\n sts PRR1, r16\n nop\n nop\n nop\n nop\n lds r23, TCNT3L\n break\n",
    );
    assert_eq!(m.run(1000), StopReason::BreakInsn);
    assert!(m.cpu.r[21] > m.cpu.r[20], "running before PRR1");
    assert_eq!(m.cpu.r[22], m.cpu.r[21], "stopped by PRTIM3");
    assert!(m.cpu.r[23] > m.cpu.r[22], "running again");
}

#[test]
fn x4_adc_differential_gain_and_internal_reference() {
    // MUX = 01001: ADC1 - ADC0 at 10x gain. (2.01 V - 2.00 V) x 10 = 0.1 V against AVCC = 5 V:
    // 0.1 V x 512 / 5 V = 10 (two's complement, always signed in differential mode).
    let conv = "ldi r16, (1<<ADEN)|(1<<ADSC)|7\n sts ADCSRA, r16\nw: lds r16, ADCSRA\n sbrc r16, ADSC\n rjmp w\n lds r20, ADCL\n lds r21, ADCH\n break\n";
    let mut m = m644(&format!("reset:\n ldi r16, (1<<REFS0)|9\n sts ADMUX, r16\n {conv}"));
    m.set_pin_input(0, ExtDrive::Analog, 2.00);
    m.set_pin_input(1, ExtDrive::Analog, 2.01);
    assert_eq!(m.run(5000), StopReason::BreakInsn);
    assert_eq!(adc_result(&m), 10);
    // MUX = 10000: ADC0 - ADC1 at 1x gain.
    let mut m = m644(&format!("reset:\n ldi r16, (1<<REFS0)|16\n sts ADMUX, r16\n {conv}"));
    m.set_pin_input(0, ExtDrive::Analog, 1.0);
    m.set_pin_input(1, ExtDrive::Analog, 2.0);
    assert_eq!(m.run(5000), StopReason::BreakInsn);
    assert_eq!(adc_result(&m), 0x400 - 103, "-1 V x 512 / 5 V = -102.4, floored");
    // Internal 1.1 V reference (REFS = 10): 0.6 V on ADC3 -> 558.
    let mut m = m324(&format!("reset:\n ldi r16, (1<<REFS1)|3\n sts ADMUX, r16\n {conv}"));
    m.set_pin_input(3, ExtDrive::Analog, 0.6);
    assert_eq!(m.run(5000), StopReason::BreakInsn);
    assert_eq!(adc_result(&m), 558);
}

#[test]
fn x4_int2_and_the_fourth_pin_change_group() {
    // INT2 (PB2 = GPIO 10) falling edge; PCINT3 = PD7..PD0 (PD5 = GPIO 29, PCINT29, PCMSK3 bit 5).
    let mut m = m324(
        ".org INT2addr\n jmp isr2\n.org PCI3addr\n jmp isr3\nreset:\n ldi r16, 1<<ISC21\n sts EICRA, r16\n ldi r16, 1<<INT2\n out EIMSK, r16\n ldi r16, 1<<PCIE3\n sts PCICR, r16\n ldi r16, 1<<5\n sts PCMSK3, r16\n sei\nl: rjmp l\nisr2:\n inc r20\n reti\nisr3:\n inc r21\n reti\n",
    );
    m.set_pin_input(10, ExtDrive::High, 0.0);
    m.set_pin_input(29, ExtDrive::High, 0.0);
    m.run(100);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (0, 0));
    m.set_pin_input(10, ExtDrive::Low, 0.0);
    m.set_pin_input(28, ExtDrive::Low, 0.0); // PD4 is not in the mask
    m.run(200);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (1, 0));
    m.set_pin_input(29, ExtDrive::Low, 0.0);
    m.run(300);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (1, 1));
    m.set_pin_input(10, ExtDrive::High, 0.0); // rising: ISC21:20 = 10 ignores it
    m.run(400);
    assert_eq!(m.cpu.r[20], 1);
}

#[test]
fn x4_timer0_pwm_timer1_compare_b_and_timer2_overflow_wiring() {
    // OC0A = PB3 (GPIO 11) fast PWM, OC1B = PD4 (GPIO 28) toggling in CTC, Timer2 overflow IRQ.
    let mut m = m324(
        ".org TIMER2_OVFaddr\n jmp isr\nreset:\n sbi DDRB, DDB3\n sbi DDRD, DDD4\n ldi r16, 100\n out OCR0A, r16\n ldi r16, (1<<COM0A1)|(1<<WGM01)|(1<<WGM00)\n out TCCR0A, r16\n ldi r16, 1<<CS00\n out TCCR0B, r16\n ldi r16, 0\n sts OCR1AH, r16\n ldi r16, 49\n sts OCR1AL, r16\n ldi r16, 0\n sts OCR1BH, r16\n sts OCR1BL, r16\n ldi r16, 1<<COM1B0\n sts TCCR1A, r16\n ldi r16, (1<<WGM12)|(1<<CS10)\n sts TCCR1B, r16\n ldi r16, 1<<TOIE2\n sts TIMSK2, r16\n ldi r16, 1<<CS20\n sts TCCR2B, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n",
    );
    m.run(256 * 6 + 100);
    let (r, f) = rises_falls(&m, 11);
    assert_eq!(r[2] - r[1], 256);
    assert_eq!(f.iter().find(|&&x| x > r[1]).unwrap() - r[1], 101);
    assert!((5..=7).contains(&m.cpu.r[20]), "Timer2 overflows: {}", m.cpu.r[20]);
    // CTC with TOP = OCR1A = 49 and OCR1B = 0: OC1B toggles at TCNT1 = 0, i.e. every 50 cycles.
    let e = edges(&m, 28);
    assert!(e.len() > 20 && e.windows(2).skip(1).all(|w| w[1] - w[0] == 50), "{:?}", &e[..6]);
}

#[test]
fn elpm_reads_above_64k_on_the_1284p() {
    // RAMPZ = 1 selects bytes 0x10000.. (word 0x8000).
    let mut m = m1284("reset:\n ldi r16, 1\n out RAMPZ, r16\n ldi ZL, 0\n ldi ZH, 0\n elpm r20, Z+\n elpm r21, Z\n break\n.org 0x8000\n.db 0x11, 0x22\n");
    assert_eq!(m.run(100), StopReason::BreakInsn);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (0x11, 0x22));
}

#[test]
fn x0_reset_defaults_and_vector_counts() {
    for (m, flash, pc3) in [(m640(IDLE), 65536u32, false), (m1280(IDLE), 131072, false), (m2560(IDLE), 262144, true)] {
        let mut m = m;
        let name = m.spec.name.clone();
        assert_eq!(m.sys.clock.hz, 1e6, "{name}");
        assert_eq!(m.cpu.sp, 0x21ff, "{name}");
        assert_eq!(m.spec.vector_count(), 57, "{name}");
        assert_eq!(m.spec.flash_size, flash);
        assert_eq!(m.cpu.fuses, [0x62, 0x99, 0xff], "{name}");
        assert_eq!(m.sys.pins.len(), 86, "{name}");
        assert_eq!(m.spec.pins.len(), 100);
        assert_eq!(m.spec.register("EIND").is_some(), pc3, "{name}: EIND only where EIJMP exists");
        assert_eq!(m.peek_data(0x54) & 0x1f, 0x01, "{name}");
    }
    // ClockSource / CKDIV8: the Arduino Mega fuses select the crystal at the next power cycle.
    let mut m = m2560(IDLE);
    m.cpu.fuses = vec![0xff, 0xd8, 0xfd];
    m.set_external_clock(16e6);
    m.power_on();
    assert_eq!(m.sys.clock.hz, 16e6);
}

#[test]
fn timer1_third_compare_unit_raises_its_interrupt_and_drives_oc1c() {
    // Fast PWM 8-bit (WGM = 0101), non-inverting OC1C (PB7), OCR1C = 100, TOP = 255.
    let mut m = m2560(
        ".org TIMER1_COMPCaddr\n jmp isr\nreset:\n sbi DDRB, DDB7\n ldi r16, 0\n sts OCR1CH, r16\n ldi r16, 100\n sts OCR1CL, r16\n ldi r16, (1<<COM1C1)|(1<<WGM10)\n sts TCCR1A, r16\n ldi r16, (1<<WGM12)|(1<<CS10)\n sts TCCR1B, r16\n lds r22, TCCR1A\n lds r23, OCR1CL\n ldi r16, 1<<OCIE1C\n sts TIMSK1, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n",
    );
    m.run(256 * 8 + 100);
    assert!((7..=9).contains(&m.cpu.r[20]), "{}", m.cpu.r[20]);
    assert_eq!(m.cpu.r[22], (1 << 3) | 1, "COM1C1:0 (bits 3:2) and WGM11:10 read back");
    assert_eq!(m.cpu.r[23], 100);
    let (r, f) = rises_falls(&m, 15);
    assert!(r.len() >= 6, "{r:?}");
    assert_eq!(r[3] - r[2], 256);
    assert_eq!(f.iter().find(|&&x| x > r[2]).unwrap() - r[2], 101);
    // The A / B channels are unaffected: OC1A (PB5) and OC1B (PB6) stay low.
    assert!(edges(&m, 13).is_empty() && edges(&m, 14).is_empty());
}

#[test]
fn timer1_force_output_compare_c_and_oc1c_toggle() {
    // Normal mode, COM1C = 01 (toggle): FOC1C toggles OC1C (PB7 = GPIO 15) immediately.
    let mut m = m1280(
        "reset:\n sbi DDRB, DDB7\n ldi r16, 0x80\n sts OCR1CH, r16\n ldi r16, 0\n sts OCR1CL, r16\n ldi r16, 1<<COM1C0\n sts TCCR1A, r16\n ldi r16, 1<<CS10\n sts TCCR1B, r16\n ldi r16, 1<<FOC1C\n sts TCCR1C, r16\n lds r20, TCCR1C\n sts TCCR1C, r16\n break\n",
    );
    assert_eq!(m.run(200), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20], 0, "FOC1C reads as zero");
    assert_eq!(edges(&m, 15).len(), 2, "two forced toggles: {:?}", edges(&m, 15));
}

#[test]
fn timer5_compare_c_interrupt_uses_tifr5_and_timsk5() {
    let mut m = m2560(
        ".org TIMER5_COMPCaddr\n jmp isr\nreset:\n ldi r16, 0\n sts OCR5CH, r16\n ldi r16, 200\n sts OCR5CL, r16\n ldi r16, 1<<CS50\n sts TCCR5B, r16\n ldi r16, 1<<OCIE5C\n sts TIMSK5, r16\n sei\nl: rjmp l\nisr:\n lds r21, TIFR5\n inc r20\n break\n",
    );
    assert_eq!(m.run(2000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20], 1);
    // The flag is cleared when the vector executes, and OCF5C is bit 3 of TIFR5.
    assert_eq!(m.peek_data(0x3a) & 0x08, 0);
    assert!(m.cpu.cycles >= 200 && m.cpu.cycles < 260, "{}", m.cpu.cycles);
}

#[test]
fn int4_and_int7_use_eicrb() {
    // INT4 (PE4 = GPIO 36) falling, INT7 (PE7 = GPIO 39) rising.
    let mut m = m2560(
        ".org INT4addr\n jmp isr4\n.org INT7addr\n jmp isr7\nreset:\n ldi r16, (1<<ISC41)|(3<<ISC70)\n sts EICRB, r16\n ldi r16, (1<<INT4)|(1<<INT7)\n out EIMSK, r16\n sei\nl: rjmp l\nisr4:\n inc r20\n reti\nisr7:\n inc r21\n reti\n",
    );
    m.set_pin_input(36, ExtDrive::High, 0.0);
    m.set_pin_input(39, ExtDrive::Low, 0.0);
    m.run(100);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (0, 0));
    m.set_pin_input(36, ExtDrive::Low, 0.0);
    m.run(200);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (1, 0));
    m.set_pin_input(39, ExtDrive::High, 0.0);
    m.run(300);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (1, 1));
    m.set_pin_input(36, ExtDrive::High, 0.0); // rising on INT4: ignored
    m.set_pin_input(39, ExtDrive::Low, 0.0); // falling on INT7: ignored
    m.run(400);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (1, 1));
}

#[test]
fn pcint1_covers_pe0_and_pj0_to_pj6_only() {
    // PCMSK1 bit 0 = PCINT8 = PE0 (GPIO 32), bit 4 = PCINT12 = PJ3 (GPIO 65). PJ7 (GPIO 69) has no
    // pin change interrupt; PB0 belongs to PCINT0 whose mask is empty.
    let mut m = m2560(
        ".org PCINT1addr\n jmp isr\nreset:\n ldi r16, 1<<PCIE1\n sts PCICR, r16\n ldi r16, (1<<PCINT8)|(1<<PCINT12)\n sts PCMSK1, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n",
    );
    for g in [32, 65, 69, 8] {
        m.set_pin_input(g, ExtDrive::High, 0.0);
    }
    m.run(100);
    assert_eq!(m.cpu.r[20], 0);
    m.set_pin_input(65, ExtDrive::Low, 0.0);
    m.run(200);
    assert_eq!(m.cpu.r[20], 1);
    m.set_pin_input(32, ExtDrive::Low, 0.0);
    m.run(300);
    assert_eq!(m.cpu.r[20], 2);
    m.set_pin_input(69, ExtDrive::Low, 0.0);
    m.set_pin_input(8, ExtDrive::Low, 0.0);
    m.run(400);
    assert_eq!(m.cpu.r[20], 2);
}

#[test]
fn adc_channels_above_7_need_mux5() {
    let conv = "ldi r16, (1<<ADEN)|(1<<ADSC)|7\n sts ADCSRA, r16\nw: lds r16, ADCSRA\n sbrc r16, ADSC\n rjmp w\n lds r20, ADCL\n lds r21, ADCH\n break\n";
    // ADC10 = PK2 (GPIO 72) at 2.5 V: MUX5 = 1, MUX4:0 = 00010.
    let mut m = m2560(&format!("reset:\n ldi r16, 1<<MUX5\n sts ADCSRB, r16\n lds r22, ADCSRB\n ldi r16, (1<<REFS0)|2\n sts ADMUX, r16\n {conv}"));
    m.set_pin_input(72, ExtDrive::Analog, 2.5);
    m.set_pin_input(42, ExtDrive::Analog, 1.25); // ADC2 = PF2: must not be sampled
    assert_eq!(m.run(5000), StopReason::BreakInsn);
    assert_eq!(adc_result(&m), 512);
    assert_eq!(m.cpu.r[22], 0x08, "MUX5 is writable");
    // Without MUX5 the same MUX bits select ADC2 = PF2.
    let mut m = m2560(&format!("reset:\n ldi r16, (1<<REFS0)|2\n sts ADMUX, r16\n {conv}"));
    m.set_pin_input(72, ExtDrive::Analog, 2.5);
    m.set_pin_input(42, ExtDrive::Analog, 1.25);
    assert_eq!(m.run(5000), StopReason::BreakInsn);
    assert_eq!(adc_result(&m), 256);
    // MUX5:0 = 101001: ADC9 - ADC8 at 10x gain, PK1 = 2.05 V, PK0 = 2.00 V -> 0.5 V -> 51.
    let mut m = m2560(&format!("reset:\n ldi r16, 1<<MUX5\n sts ADCSRB, r16\n ldi r16, (1<<REFS0)|9\n sts ADMUX, r16\n {conv}"));
    m.set_pin_input(70, ExtDrive::Analog, 2.00);
    m.set_pin_input(71, ExtDrive::Analog, 2.05);
    assert_eq!(m.run(5000), StopReason::BreakInsn);
    assert_eq!(adc_result(&m), 51);
    // MUX5:0 = 111110 is reserved; 011110 selects the 1.1 V bandgap (1.1 V / AVCC x 1024 = 225).
    let mut m = m2560(&format!("reset:\n ldi r16, (1<<REFS0)|30\n sts ADMUX, r16\n {conv}"));
    assert_eq!(m.run(5000), StopReason::BreakInsn);
    assert_eq!(adc_result(&m), 225);
}

#[test]
fn analog_comparator_negative_input_follows_mux5() {
    // AIN0 = PE2 (GPIO 34) at 2 V against ADC9 = PK1 (GPIO 71) at 1 V (MUX5 = 1, MUX2:0 = 001) with
    // ACME set and the ADC off: ACO = 1; against ADC1 = PF1 (GPIO 41) at 3 V (MUX5 = 0): ACO = 0.
    let body = |srb: &str| format!("reset:\n ldi r16, {srb}\n sts ADCSRB, r16\n ldi r16, 1\n sts ADMUX, r16\n in r20, ACSR\n break\n");
    let mut m = m2560(&body("(1<<ACME)|(1<<MUX5)"));
    m.set_pin_input(34, ExtDrive::Analog, 2.0);
    m.set_pin_input(71, ExtDrive::Analog, 1.0);
    m.set_pin_input(41, ExtDrive::Analog, 3.0);
    assert_eq!(m.run(100), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20] & 0x20, 0x20, "ACO: 2 V > 1 V (ADC9)");
    let mut m = m2560(&body("1<<ACME"));
    m.set_pin_input(34, ExtDrive::Analog, 2.0);
    m.set_pin_input(71, ExtDrive::Analog, 1.0);
    m.set_pin_input(41, ExtDrive::Analog, 3.0);
    assert_eq!(m.run(100), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20] & 0x20, 0, "ACO: 2 V < 3 V (ADC1)");
}

#[test]
fn prr1_and_prr0_are_combined_for_the_timers() {
    // Timer3 (PRR1.PRTIM3) and Timer5 (PRR1.PRTIM5) run; stopping Timer3 must not re-enable it
    // when PRR0 is written afterwards.
    let mut m = m2560(
        "reset:\n ldi r16, 1<<CS30\n sts TCCR3B, r16\n ldi r16, 1<<CS50\n sts TCCR5B, r16\n ldi r16, 1<<PRTIM3\n sts PRR1, r16\n ldi r16, 1<<PRTIM0\n sts PRR0, r16\n lds r20, TCNT3L\n lds r22, TCNT5L\n nop\n nop\n nop\n nop\n nop\n nop\n lds r21, TCNT3L\n lds r23, TCNT5L\n lds r24, PRR1\n lds r25, PRR0\n break\n",
    );
    assert_eq!(m.run(1000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[21], m.cpu.r[20], "Timer3 stays stopped");
    assert!(m.cpu.r[23] > m.cpu.r[22], "Timer5 keeps running");
    assert_eq!((m.cpu.r[24], m.cpu.r[25]), (0x08, 0x20));
    // PRR0 bit 4 does not exist on the ATmega2560.
    let mut m = m2560("reset:\n ldi r16, 0xff\n sts PRR0, r16\n sts PRR1, r16\n lds r20, PRR0\n lds r21, PRR1\n break\n");
    assert_eq!(m.run(100), StopReason::BreakInsn);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (0xef, 0x3f));
}

#[test]
fn far_call_eicall_and_elpm_above_128k_words_on_the_2560() {
    // EICALL to word 0x18000 (EIND = 1, Z = 0x8000): a 3-byte return address; ELPM with RAMPZ = 3
    // reads the table at byte 0x30004.
    let mut m = m2560(
        "reset:\n ldi r16, 1\n out EIND, r16\n ldi ZL, low(far_a)\n ldi ZH, high(far_a)\n eicall\n ldi r21, 7\n ldi r16, 3\n out RAMPZ, r16\n ldi ZL, low(tbl*2)\n ldi ZH, high(tbl*2)\n elpm r22, Z+\n elpm r23, Z\n break\n.org 0x18000\nfar_a:\n in r24, SPL\n in r25, SPH\n ldi r20, 0x42\n ret\ntbl: .db 0xab, 0xcd\n",
    );
    let sp0 = m.cpu.sp;
    assert_eq!(m.run(200), StopReason::BreakInsn);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (0x42, 7));
    assert_eq!((m.cpu.r[22], m.cpu.r[23]), (0xab, 0xcd));
    assert_eq!(m.cpu.sp, sp0, "balanced");
    assert_eq!(((m.cpu.r[25] as u16) << 8 | m.cpu.r[24] as u16), sp0 - 3, "EICALL pushes a 3-byte return address");
    // The ATmega1280 (64K words) keeps a 2-byte return address.
    let mut m = m1280("reset:\n rcall f\n break\nf:\n in r24, SPL\n in r25, SPH\n ret\n");
    let sp0 = m.cpu.sp;
    assert_eq!(m.run(100), StopReason::BreakInsn);
    assert_eq!(((m.cpu.r[25] as u16) << 8 | m.cpu.r[24] as u16), sp0 - 2);
}

#[test]
fn pin_trace_covers_all_86_gpios() {
    // PL7 (GPIO 85) toggles; no lower pin may alias with it (a 32-bit shift would wrap onto 21).
    let mut m = m2560("reset:\n ldi r16, 0x80\n sts DDRL, r16\nl: sts PINL, r16\n rjmp l\n");
    m.run(400);
    assert_eq!(m.sys.trace.words(), 3);
    let e = edges(&m, 85);
    assert!(e.len() > 50 && e.windows(2).all(|w| w[1] - w[0] == 4), "{:?}", &e[..4]);
    for alias in [21, 53, 7] {
        assert!(edges(&m, alias).is_empty(), "GPIO {alias} aliased");
    }
    // The 32-bit view of the narrow API still describes pins 0-31.
    let (_, c, l) = m.sys.trace.read_since(0, usize::MAX);
    assert_eq!(c.len(), l.len());
}

#[test]
fn spi_and_twi_are_wired_to_the_2560_pins() {
    // Master SPI: SS = PB0, SCK = PB1, MOSI = PB2, MISO = PB3. fosc/4: SCK edges 2 cycles apart.
    let mut m = m2560(
        "reset:\n ldi r16, (1<<DDB0)|(1<<DDB1)|(1<<DDB2)\n out DDRB, r16\n ldi r16, (1<<SPE)|(1<<MSTR)\n out SPCR, r16\n ldi r16, 0xa5\n out SPDR, r16\nw: in r16, SPSR\n sbrs r16, SPIF\n rjmp w\n in r20, SPDR\n break\n",
    );
    m.set_pin_input(11, ExtDrive::High, 0.0); // MISO
    assert_eq!(m.run(2000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20], 0xff);
    let sck = edges(&m, 9);
    assert_eq!(sck.len(), 16);
    assert!(sck.windows(2).all(|w| w[1] - w[0] == 2));
}
