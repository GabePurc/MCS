//! ATtiny13A, ATtiny24A/44A/84A and ATtiny2313A/4313: reset defaults, timers, ADC, USART, EEPROM
//! and pin-change wake-up, driven by small assembly programs built with the MCS assembler and
//! the generated tn13Adef.inc / tn84Adef.inc / tn2313Adef.inc.

use mcs_asm::{assemble, AssembleOptions};
use mcs_core::avr::devices;
use mcs_sim::avr::peripherals::serial::SerialConfig;
use mcs_sim::avr::{Machine, StopReason};
use mcs_sim::pins::ExtDrive;

fn build(dev: &str, inc: &str, src: &str) -> Machine {
    let r = assemble(&format!(".include \"{inc}\"\n.org 0\n    rjmp reset\n{src}"), &AssembleOptions::new("t.asm", dev));
    assert!(r.ok, "{:#?}", r.diagnostics);
    let mut m = Machine::new(devices::get(&r.device_id).unwrap());
    m.load(&r.program);
    m
}

fn t13(body: &str) -> Machine {
    build("attiny13a", "tn13Adef.inc", body)
}

fn t84(body: &str) -> Machine {
    build("attiny84a", "tn84Adef.inc", body)
}

fn t2313(body: &str) -> Machine {
    build("attiny2313a", "tn2313Adef.inc", body)
}

/// (rise, fall) cycle lists of GPIO `g`.
fn rises_falls(m: &Machine, g: u32) -> (Vec<u64>, Vec<u64>) {
    let (_, c, l) = m.sys.trace.read_since(0, usize::MAX);
    let r = (1..c.len()).filter(|&i| l[i] >> g & 1 == 1 && l[i - 1] >> g & 1 == 0).map(|i| c[i]).collect();
    let f = (1..c.len()).filter(|&i| l[i] >> g & 1 == 0 && l[i - 1] >> g & 1 == 1).map(|i| c[i]).collect();
    (r, f)
}

fn adc_result(m: &Machine) -> u16 {
    (m.cpu.r[21] as u16) << 8 | m.cpu.r[20] as u16
}

// ------------------------------------------------------------------------------ ATtiny13A

#[test]
fn tiny13a_reset_defaults_and_clock_fuses() {
    let mut m = t13("reset:\n sbi DDRB, DDB3\nl: sbi PINB, PINB3\n rjmp l\n");
    assert_eq!(m.sys.clock.hz, 1.2e6, "9.6 MHz RC with CKDIV8");
    assert_eq!(m.cpu.sp, 0x9f);
    assert_eq!(m.peek_data(0x54) & 0x01, 0x01, "PORF");
    assert!(m.sys.pins[5].reserved, "PB5 is RESET");
    m.run(200);
    let (r, f) = rises_falls(&m, 3);
    assert!(r.len() > 20 && f.len() > 20);
    // CKSEL = 01 (4.8 MHz), CKDIV8 unprogrammed; CKSEL = 11 (128 kHz) with CKDIV8.
    m.cpu.fuses[0] = 0x79;
    m.power_on();
    assert_eq!(m.sys.clock.hz, 4.8e6);
    m.cpu.fuses[0] = 0x6b;
    m.power_on();
    assert_eq!(m.sys.clock.hz, 128e3 / 8.0);
}

#[test]
fn tiny13a_timer0_fast_pwm_and_overflow_interrupt() {
    let mut m = t13(
        ".org OVF0addr\n rjmp isr\nreset:\n sbi DDRB, DDB0\n ldi r16, 100\n out OCR0A, r16\n ldi r16, (1<<COM0A1)|(1<<WGM01)|(1<<WGM00)\n out TCCR0A, r16\n ldi r16, 1<<TOIE0\n out TIMSK0, r16\n ldi r16, 1<<CS00\n out TCCR0B, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n",
    );
    m.run(256 * 8 + 100);
    let (r, f) = rises_falls(&m, 0);
    assert_eq!(r[3] - r[2], 256);
    assert_eq!(f.iter().find(|&&x| x > r[2]).unwrap() - r[2], 101);
    assert!((7..=8).contains(&m.cpu.r[20]));
}

#[test]
fn tiny13a_timer0_compare_flags_layout() {
    // TIFR0: TOV0 = bit 1, OCF0A = bit 2, OCF0B = bit 3.
    let mut m = t13("reset:\n ldi r16, 50\n out OCR0A, r16\n ldi r16, 1<<CS00\n out TCCR0B, r16\n ldi r17, 300>>1\nw: dec r17\n brne w\n in r20, TIFR0\n break\n");
    assert_eq!(m.run(2000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20] & 0x06, 0x06, "OCF0A and TOV0");
}

#[test]
fn tiny13a_adc_single_ended_reference_and_left_adjust() {
    let src = |admux: &str| format!("reset:\n ldi r16, {admux}\n out ADMUX, r16\n ldi r16, (1<<ADEN)|(1<<ADSC)|(1<<ADPS1)|(1<<ADPS0)\n out ADCSRA, r16\nw: sbic ADCSRA, ADSC\n rjmp w\n in r20, ADCL\n in r21, ADCH\n break\n");
    // ADC1 = PB2 against the internal 1.1 V reference (REFS0 = 1).
    let mut m = t13(&src("(1<<REFS0)|1"));
    m.set_pin_input(2, ExtDrive::Analog, 0.55);
    assert_eq!(m.run(10_000), StopReason::BreakInsn);
    assert_eq!(adc_result(&m), 512);
    // ADC3 = PB3 against VCC (5 V), left adjusted.
    let mut m = t13(&src("(1<<ADLAR)|3"));
    m.set_pin_input(3, ExtDrive::Analog, 2.5);
    m.run(10_000);
    assert_eq!(m.cpu.r[21], 0x80);
}

#[test]
fn tiny13a_pin_change_wakes_from_power_down() {
    let mut m = t13(
        ".org PCI0addr\n rjmp isr\nreset:\n sbi PORTB, PORTB3\n ldi r16, 1<<PCIE\n out GIMSK, r16\n sbi PCMSK, PCINT3\n ldi r16, (1<<SE)|(1<<SM1)\n out MCUCR, r16\n sei\n sleep\n inc r21\n break\nisr:\n inc r20\n reti\n",
    );
    m.run(1000);
    assert!(m.cpu.sleeping);
    m.set_pin_input(3, ExtDrive::Low, 0.0);
    assert_eq!(m.run(m.cpu.cycles + 1000), StopReason::BreakInsn);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (1, 1));
}

#[test]
fn tiny13a_eeprom_write_read() {
    let mut m = t13("reset:\n ldi r16, 0x10\n out EEARL, r16\n ldi r16, 0x5a\n out EEDR, r16\n sbi EECR, EEMPE\n sbi EECR, EEPE\nw: sbic EECR, EEPE\n rjmp w\n sbi EECR, EERE\n in r20, EEDR\n break\n");
    assert_eq!(m.run(20_000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20], 0x5a);
    assert_eq!(m.cpu.eeprom[0x10], 0x5a);
}

// --------------------------------------------------------------------------- ATtiny24A/44A/84A

#[test]
fn tiny84a_reset_defaults_ports_and_timer0_flags() {
    let mut m = t84(
        "reset:\n sbi DDRA, DDA2\n sbi PORTA, PORTA2\n sbi DDRB, DDB1\n ldi r16, 1<<CS00\n out TCCR0B, r16\n ldi r17, 200\nw: dec r17\n brne w\n in r20, TIFR0\n break\n",
    );
    assert_eq!(m.sys.clock.hz, 1e6);
    assert_eq!(m.cpu.sp, 0x025f);
    assert!(m.sys.pins[11].reserved, "PB3 is RESET");
    assert_eq!(m.run(2000), StopReason::BreakInsn);
    assert_eq!(m.sys.pins[2].level, 1, "PA2");
    assert_eq!(m.sys.pins[9].level, 0, "PB1");
    assert_eq!(m.cpu.r[20] & 0x01, 0x01, "TOV0 in TIFR0");
    // ATtiny24A: 128 B of SRAM.
    let t24 = Machine::new(devices::get("attiny24a").unwrap());
    assert_eq!(t24.cpu.sp, 0x00df);
}

#[test]
fn tiny84a_timer1_ctc_interrupt() {
    let mut m = t84(
        ".org OC1Aaddr\n rjmp isr\nreset:\n ldi r16, high(999)\n out OCR1AH, r16\n ldi r16, low(999)\n out OCR1AL, r16\n ldi r16, 1<<OCIE1A\n out TIMSK1, r16\n ldi r16, (1<<WGM12)|(1<<CS10)\n out TCCR1B, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n",
    );
    m.run(10_500);
    assert_eq!(m.cpu.r[20], 10);
}

#[test]
fn tiny84a_timer1_toggles_oc1a_and_captures_input() {
    // OC1A (PA6) toggles on compare match in CTC mode; ICP1 (PA7) captures TCNT1.
    let mut m = t84(
        "reset:\n sbi DDRA, DDA6\n ldi r16, high(99)\n out OCR1AH, r16\n ldi r16, low(99)\n out OCR1AL, r16\n ldi r16, 1<<COM1A0\n out TCCR1A, r16\n ldi r16, (1<<WGM12)|(1<<CS10)\n out TCCR1B, r16\nl: rjmp l\n",
    );
    m.run(1000);
    let (r, _) = rises_falls(&m, 6);
    assert!(r.len() >= 4);
    assert_eq!(r[3] - r[2], 200, "toggle every 100 cycles");
    let mut m = t84("reset:\n ldi r16, 1<<CS10\n out TCCR1B, r16\nl: rjmp l\n");
    m.set_pin_input(7, ExtDrive::High, 0.0);
    m.run(100);
    m.set_pin_input(7, ExtDrive::Low, 0.0); // falling edge (ICES1 = 0)
    m.run(m.cpu.cycles + 10);
    let icr = m.peek_data(0x44) as u16 | (m.peek_data(0x45) as u16) << 8;
    assert!((90..105).contains(&icr), "{icr}");
}

#[test]
fn tiny84a_adc_single_ended_adlar_in_adcsrb_and_differential() {
    let src = |admux: &str, srb: &str| format!("reset:\n ldi r16, {srb}\n out ADCSRB, r16\n ldi r16, {admux}\n out ADMUX, r16\n ldi r16, (1<<ADEN)|(1<<ADSC)|(1<<ADPS1)|(1<<ADPS0)\n out ADCSRA, r16\nw: sbic ADCSRA, ADSC\n rjmp w\n in r20, ADCL\n in r21, ADCH\n break\n");
    // ADC3 = PA3 against VCC.
    let mut m = t84(&src("3", "0"));
    m.set_pin_input(3, ExtDrive::Analog, 2.5);
    assert_eq!(m.run(10_000), StopReason::BreakInsn);
    assert_eq!(adc_result(&m), 512);
    let mut m = t84(&src("3", "1<<ADLAR"));
    m.set_pin_input(3, ExtDrive::Analog, 2.5);
    m.run(10_000);
    assert_eq!(m.cpu.r[21], 0x80, "ADLAR lives in ADCSRB");
    // ADC1 - ADC2 at 1x (MUX = 001100) against the internal 1.1 V reference.
    let mut m = t84(&src("(1<<REFS1)|0b001100", "0"));
    m.set_pin_input(1, ExtDrive::Analog, 1.0);
    m.set_pin_input(2, ExtDrive::Analog, 0.5);
    m.run(10_000);
    assert_eq!(adc_result(&m), (0.5f64 * 1024.0 / 1.1).floor() as u16);
    // Same pair at 20x (MUX0 = 1): 10 V saturates.
    let mut m = t84(&src("(1<<REFS1)|0b001101", "0"));
    m.set_pin_input(1, ExtDrive::Analog, 1.0);
    m.set_pin_input(2, ExtDrive::Analog, 0.5);
    m.run(10_000);
    assert_eq!(adc_result(&m), 1023);
    // Polarity reversal (MUX5): ADC2 - ADC1 is negative and clamps to 0 in unipolar mode.
    let mut m = t84(&src("(1<<REFS1)|0b101100", "0"));
    m.set_pin_input(1, ExtDrive::Analog, 1.0);
    m.set_pin_input(2, ExtDrive::Analog, 0.5);
    m.run(10_000);
    assert_eq!(adc_result(&m), 0);
}

#[test]
fn tiny84a_pin_change_group_1_wakes_from_power_down() {
    let mut m = t84(
        ".org PCI1addr\n rjmp isr\nreset:\n sbi PORTB, PORTB0\n ldi r16, 1<<PCIE1\n out GIMSK, r16\n ldi r16, 1<<PCINT8\n out PCMSK1, r16\n ldi r16, (1<<SE)|(1<<SM1)\n out MCUCR, r16\n sei\n sleep\n inc r21\n break\nisr:\n inc r20\n reti\n",
    );
    m.run(1000);
    assert!(m.cpu.sleeping);
    m.set_pin_input(8, ExtDrive::Low, 0.0); // PB0
    assert_eq!(m.run(m.cpu.cycles + 1000), StopReason::BreakInsn);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (1, 1));
}

#[test]
fn tiny84a_int0_on_pb2() {
    let mut m = t84(".org INT0addr\n rjmp isr\nreset:\n ldi r16, 1<<ISC01\n out MCUCR, r16\n ldi r16, 1<<INT0\n out GIMSK, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n");
    m.set_pin_input(10, ExtDrive::High, 0.0);
    m.run(200);
    m.set_pin_input(10, ExtDrive::Low, 0.0);
    m.run(m.cpu.cycles + 100);
    m.set_pin_input(10, ExtDrive::High, 0.0);
    m.run(m.cpu.cycles + 100);
    assert_eq!(m.cpu.r[20], 1);
}

// ---------------------------------------------------------------------------- ATtiny2313A/4313

#[test]
fn tiny2313a_reset_defaults() {
    let mut m = t2313("reset:\nl: rjmp l\n");
    assert_eq!(m.sys.clock.hz, 1e6, "8 MHz RC with CKDIV8");
    assert_eq!(m.cpu.sp, 0x00df);
    assert_eq!(m.peek_data(0x54) & 0x01, 0x01, "PORF");
    assert!(m.sys.pins[2].reserved, "PA2 is RESET");
    assert_eq!(devices::get("attiny4313").unwrap().ram_end(), 0x15f);
    assert_eq!(Machine::new(devices::get("attiny4313").unwrap()).cpu.sp, 0x015f);
    // CKSEL = 0010 (4 MHz), CKDIV8 unprogrammed.
    m.cpu.fuses[0] = 0xe2;
    m.power_on();
    assert_eq!(m.sys.clock.hz, 4e6);
}

#[test]
fn tiny2313a_timer0_and_timer1_share_tifr_and_timsk() {
    let mut m = t2313(
        ".org OC1Aaddr\n rjmp isr1\n.org OVF0addr\n rjmp isr0\nreset:\n ldi r16, high(99)\n out OCR1AH, r16\n ldi r16, low(99)\n out OCR1AL, r16\n ldi r16, (1<<WGM12)|(1<<CS10)\n out TCCR1B, r16\n ldi r16, 1<<CS00\n out TCCR0B, r16\n ldi r16, (1<<OCIE1A)|(1<<TOIE0)\n out TIMSK, r16\n sei\nl: rjmp l\nisr0:\n inc r20\n reti\nisr1:\n inc r21\n reti\n",
    );
    m.run(2600);
    assert!((9..=10).contains(&m.cpu.r[20]), "Timer0 overflows: {}", m.cpu.r[20]);
    assert!((25..=26).contains(&m.cpu.r[21]), "Timer1 compares: {}", m.cpu.r[21]);
}

#[test]
fn tiny2313a_tifr_flag_layout() {
    // TIFR: OCF1A = bit 6, TOV0 = bit 1, TOV1 = bit 7.
    let mut m = t2313(
        "reset:\n ldi r16, 0x10\n out OCR1BH, r16\n ldi r16, 0\n out OCR1BL, r16\n ldi r16, 0\n out OCR1AH, r16\n ldi r16, 99\n out OCR1AL, r16\n ldi r16, (1<<WGM12)|(1<<CS10)\n out TCCR1B, r16\n ldi r16, 1<<CS00\n out TCCR0B, r16\n ldi r17, 150\nw: dec r17\n brne w\n in r20, TIFR\n break\n",
    );
    assert_eq!(m.run(2000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20] & 0x42, 0x42);
    assert_eq!(m.cpu.r[20] & 0x80, 0, "TOV1 not set in CTC mode");
}

#[test]
fn tiny2313a_usart_transmits_and_echoes_through_the_pins() {
    let mut m = t2313(
        "reset:\n ldi r16, 0\n out UBRRH, r16\n ldi r16, 12\n out UBRRL, r16\n ldi r16, 1<<U2X\n out UCSRA, r16\n ldi r16, (1<<TXEN)|(1<<RXEN)\n out UCSRB, r16\n ldi r16, 3<<UCSZ0\n out UCSRC, r16\n ldi ZL, low(msg*2)\n ldi ZH, high(msg*2)\nsend:\n lpm r17, Z+\n tst r17\n breq echo\nw1: in r16, UCSRA\n sbrs r16, UDRE\n rjmp w1\n out UDR, r17\n rjmp send\necho:\n in r16, UCSRA\n sbrs r16, RXC\n rjmp echo\n in r17, UDR\nw2: in r16, UCSRA\n sbrs r16, UDRE\n rjmp w2\n out UDR, r17\n rjmp echo\nmsg: .db \"Hi!\", 0\n",
    );
    // PD1 (TXD) = GPIO 12, PD0 (RXD) = GPIO 11; 1 MHz / (8 * 13) = 9615 baud.
    m.set_serial(SerialConfig { monitor: Some(12), inject: Some(11), baud: 9600.0, data_bits: 8, parity: 0, stop_bits: 1 });
    m.run(50_000);
    assert_eq!(std::mem::take(&mut m.sys.serial_out), b"Hi!");
    m.serial_send(b"ok");
    let c = m.cpu.cycles;
    m.run(c + 50_000);
    assert_eq!(m.sys.serial_out, b"ok");
}

#[test]
fn tiny2313a_int1_and_pin_change_wake_from_power_down() {
    // SM1:0 = 11 is power-down (Table 7-2); INT1 = PD3 (GPIO 14, low level wakes it up),
    // PCINT0 group = PB0 (GPIO 3).
    let mut m = t2313(
        ".org INT1addr\n rjmp isr1\n.org PCI0addr\n rjmp isr0\nreset:\n sbi PORTB, PORTB0\n sbi PORTD, PORTD3\n ldi r16, (1<<PCIE0)|(1<<INT1)\n out GIMSK, r16\n ldi r16, 1<<PCINT0\n out PCMSK0, r16\n ldi r16, (1<<SE)|(1<<SM1)|(1<<SM0)\n out MCUCR, r16\n sei\n sleep\n inc r22\n sleep\n inc r23\n break\nisr0:\n inc r20\n reti\nisr1:\n inc r21\n in r18, GIMSK\n andi r18, 0xff^(1<<INT1)\n out GIMSK, r18\n reti\n",
    );
    m.run(1000);
    assert!(m.cpu.sleeping);
    m.set_pin_input(14, ExtDrive::Low, 0.0); // INT1 low level
    m.run(m.cpu.cycles + 200);
    assert_eq!((m.cpu.r[21], m.cpu.r[22]), (1, 1));
    assert!(m.cpu.sleeping);
    m.set_pin_input(3, ExtDrive::Low, 0.0); // PB0 pin change
    assert_eq!(m.run(m.cpu.cycles + 1000), StopReason::BreakInsn);
    assert_eq!((m.cpu.r[20], m.cpu.r[23]), (1, 1));
}

#[test]
fn tiny2313a_sleep_mode_field_is_non_contiguous() {
    // SM1 = 1, SM0 = 0 is standby, which keeps the oscillator running but is not power-down.
    let m = devices::get("attiny2313a").unwrap();
    let sc = &m.sleep;
    assert_eq!(sc.sm_mask, 0x50);
    let mode = |mcucr: u8| sc.modes.iter().find(|e| e.0 == (mcucr & sc.sm_mask) >> sc.sm_mask.trailing_zeros()).map(|e| e.1);
    use mcs_core::avr::device::SleepKind::*;
    assert_eq!([mode(0x00), mode(0x10), mode(0x40), mode(0x50)], [Some(Idle), Some(PowerDown), Some(Standby), Some(PowerDown)]);
}
