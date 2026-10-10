//! ATmega8 / ATmega16 / ATmega32: reset defaults, timers (single TCCRn layout), USART URSEL,
//! external interrupts (INT2), ADC (free running, SFIOR trigger select, differential gain),
//! SFIOR.PUD, the legacy watchdog, sleep with the non-contiguous ATmega16 SM field and IVSEL in
//! GICR, driven by small assembly programs built with the MCS assembler and the generated
//! m8def.inc / m16def.inc / m32def.inc.

use mcs_asm::{assemble, AssembleOptions};
use mcs_core::avr::device::SleepKind;
use mcs_core::avr::devices;
use mcs_sim::avr::peripherals::serial::SerialConfig;
use mcs_sim::avr::{Machine, ResetSource, StopReason};
use mcs_sim::pins::ExtDrive;

fn build(dev: &str, inc: &str, jump: &str, src: &str) -> Machine {
    let r = assemble(&format!(".include \"{inc}\"\n.org 0\n    {jump} reset\n{src}"), &AssembleOptions::new("t.asm", dev));
    assert!(r.ok, "{:#?}", r.diagnostics);
    let mut m = Machine::new(devices::get(&r.device_id).unwrap());
    m.load(&r.program);
    m
}

fn m8(body: &str) -> Machine {
    build("atmega8", "m8def.inc", "rjmp", body)
}

fn m16(body: &str) -> Machine {
    build("atmega16", "m16def.inc", "jmp", body)
}

fn m32(body: &str) -> Machine {
    build("atmega32", "m32def.inc", "jmp", body)
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

// GPIO numbers: ATmega8 PB0-7 = 0-7, PC0-6 = 8-14, PD0-7 = 15-22.
// ATmega16/32 PA0-7 = 0-7, PB0-7 = 8-15, PC0-7 = 16-23, PD0-7 = 24-31.

#[test]
fn reset_defaults_and_clock_fuses() {
    let mut m = m8("reset:\n sbi DDRB, DDB0\nl: sbi PINB, PINB0\n rjmp l\n");
    assert_eq!(m.sys.clock.hz, 1e6, "internal RC, CKSEL = 0001");
    assert_eq!(m.cpu.sp, 0x045f);
    assert_eq!(m.peek_data(0x54) & 0x01, 0x01, "MCUCSR.PORF");
    assert!(m.sys.pins[14].reserved, "PC6 is RESET");
    m.run(200);
    let (r, f) = rises_falls(&m, 0);
    assert!(r.len() > 20 && f.len() > 20);
    // CKSEL = 0100 internal 8 MHz, 0010 2 MHz.
    m.cpu.fuses[0] = 0xe4;
    m.power_on();
    assert_eq!(m.sys.clock.hz, 8e6);
    m.cpu.fuses[0] = 0xe2;
    m.power_on();
    assert_eq!(m.sys.clock.hz, 2e6);
    // RSTDISBL frees PC6.
    m.cpu.fuses[1] = 0x59;
    m.power_on();
    assert!(!m.sys.pins[14].reserved);

    let m = m16("reset:\nl: rjmp l\n");
    assert_eq!((m.sys.clock.hz, m.cpu.sp), (1e6, 0x045f));
    assert_eq!(m.cpu.fuses, [0xe1, 0x99]);
    let m = m32("reset:\nl: rjmp l\n");
    assert_eq!((m.sys.clock.hz, m.cpu.sp), (1e6, 0x085f));
}

#[test]
fn brown_out_uses_boden_and_one_bit_bodlevel() {
    let mut m = m16("reset:\nl: rjmp l\n");
    m.set_vcc(3.0);
    assert!(!m.sys.brown_out, "BODEN unprogrammed");
    m.set_vcc(5.0);
    m.cpu.fuses[0] = 0xa1; // BODEN programmed, BODLEVEL = 1 (2.7 V)
    m.power_on();
    m.set_vcc(3.0);
    assert!(!m.sys.brown_out);
    m.set_vcc(2.5);
    assert!(m.sys.brown_out);
    m.set_vcc(5.0);
    m.cpu.fuses[0] = 0x21; // BODLEVEL = 0 (4.0 V)
    m.power_on();
    m.set_vcc(3.0);
    assert!(m.sys.brown_out);
}

#[test]
fn atmega8_timer0_is_a_plain_counter_with_overflow_interrupt() {
    let mut m = m8(
        ".org OVF0addr\n rjmp isr\nreset:\n ldi r16, 1<<TOIE0\n out TIMSK, r16\n ldi r16, (1<<CS00)\n out TCCR0, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n",
    );
    m.run(256 * 8 + 100);
    assert!((7..=8).contains(&m.cpu.r[20]), "{}", m.cpu.r[20]);
    // Only CS02:0 exist in TCCR0; there is no compare unit.
    // (The ATmega8 has no BREAK instruction: programs idle in a loop.)
    let mut m = m8("reset:\n ldi r16, 0xff\n out TCCR0, r16\n in r20, TCCR0\nl: rjmp l\n");
    m.run(500);
    assert_eq!(m.cpu.r[20], 0x07);
}

#[test]
fn atmega16_timer0_fast_pwm_on_oc0() {
    // OC0 = PB3 (GPIO 11): non-inverting fast PWM, OCR0 = 100.
    let mut m = m16(
        ".org OVF0addr\n jmp isr\nreset:\n sbi DDRB, DDB3\n ldi r16, 100\n out OCR0, r16\n ldi r16, (1<<WGM00)|(1<<WGM01)|(1<<COM01)|(1<<CS00)\n out TCCR0, r16\n in r21, TCCR0\n ldi r16, 1<<TOIE0\n out TIMSK, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n",
    );
    m.run(256 * 8 + 100);
    let (r, f) = rises_falls(&m, 11);
    assert_eq!(r[3] - r[2], 256);
    assert_eq!(f.iter().find(|&&x| x > r[2]).unwrap() - r[2], 101);
    assert!((7..=8).contains(&m.cpu.r[20]));
    assert_eq!(m.cpu.r[21], 0x69, "TCCR0 reads back WGM00, COM01, WGM01 and CS00 (FOC0 reads 0)");
}

#[test]
fn timer2_ctc_with_ocr2_and_interrupt() {
    // ATmega32: TIMER2_COMP is vector 4 (OC2addr), the compare flag is TIFR bit 7.
    let mut m = m32(
        ".org OC2addr\n jmp isr\nreset:\n ldi r16, 99\n out OCR2, r16\n ldi r16, 1<<OCIE2\n out TIMSK, r16\n ldi r16, (1<<WGM21)|(1<<CS20)\n out TCCR2, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n",
    );
    m.run(1_050);
    assert!((9..=10).contains(&m.cpu.r[20]), "{}", m.cpu.r[20]);
    // ATmega8: same code, one-word vectors.
    let mut m = m8(
        ".org OC2addr\n rjmp isr\nreset:\n ldi r16, 99\n out OCR2, r16\n ldi r16, 1<<OCIE2\n out TIMSK, r16\n ldi r16, (1<<WGM21)|(1<<CS20)\n out TCCR2, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n",
    );
    m.run(1_050);
    assert!((9..=10).contains(&m.cpu.r[20]), "{}", m.cpu.r[20]);
}

#[test]
fn timer1_foc1a_in_tccr1a_toggles_oc1a() {
    // ATmega8: OC1A = PB1 (GPIO 1). COM1A0 = 1 toggles on compare; FOC1A strobes it with the
    // timer stopped and reads back as 0.
    let mut m = m8("reset:\n sbi DDRB, DDB1\n ldi r16, (1<<COM1A0)|(1<<FOC1A)\n out TCCR1A, r16\n in r20, TCCR1A\nl: rjmp l\n");
    m.run(1000);
    assert_eq!(m.sys.pins[1].level, 1);
    assert_eq!(m.cpu.r[20], 0x40);
    // ATmega16: OC1B = PD4 (GPIO 28), FOC1B strobe.
    let mut m = m16("reset:\n sbi DDRD, DDD4\n ldi r16, (1<<COM1B0)|(1<<FOC1B)\n out TCCR1A, r16\n break\n");
    assert_eq!(m.run(1000), StopReason::BreakInsn);
    assert_eq!(m.sys.pins[28].level, 1);
}

#[test]
fn usart_ubrrh_ucsrc_share_an_address_and_transmit() {
    let mut m = m8(
        "reset:\n ldi r16, 0x01\n out UBRRH, r16\n in r23, UBRRH\n in r24, UCSRC\n ldi r16, (1<<URSEL)|(3<<UCSZ0)\n out UCSRC, r16\n in r20, UBRRH\n in r21, UCSRC\n nop\n in r22, UCSRC\n \
         ldi r16, 0\n out UBRRH, r16\n ldi r16, 12\n out UBRRL, r16\n ldi r16, 1<<U2X\n out UCSRA, r16\n ldi r16, 1<<TXEN\n out UCSRB, r16\n ldi r17, 'O'\n out UDR, r17\nw: sbis UCSRA, TXC\n rjmp w\nl: rjmp l\n",
    );
    // PD1 (TXD) = GPIO 16; UBRR = 12 with U2X at 1 MHz is 9615 baud.
    m.set_serial(SerialConfig { monitor: Some(16), inject: Some(15), baud: 9600.0, data_bits: 8, parity: 0, stop_bits: 1 });
    m.run(50_000);
    assert_eq!(m.cpu.r[23], 0x01, "first read: UBRRH (URSEL = 0 write)");
    assert_eq!(m.cpu.r[24], 0x86, "back-to-back read: UCSRC reset value, URSEL reads 1");
    assert_eq!(m.cpu.r[20], 0x01, "UBRRH unchanged by the UCSRC write");
    assert_eq!(m.cpu.r[21], 0x86, "UCSRC = URSEL | UCSZ1:0 = 8 bits (UCSZ = 11, reset value kept)");
    assert_eq!(m.cpu.r[22], 0x01, "a read after a gap returns UBRRH again");
    assert_eq!(std::mem::take(&mut m.sys.serial_out), b"O");
}

#[test]
fn atmega32_int2_is_edge_only_with_isc2_in_mcucsr() {
    // INT2 = PB2 (GPIO 10). ISC2 = 1: rising edge.
    let src = |isc2: &str| format!(".org INT2addr\n jmp isr\nreset:\n ldi r16, {isc2}\n out MCUCSR, r16\n ldi r16, 1<<INT2\n out GICR, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n");
    let mut m = m32(&src("1<<ISC2"));
    m.set_pin_input(10, ExtDrive::Low, 0.0);
    m.run(200);
    m.set_pin_input(10, ExtDrive::High, 0.0);
    m.run(m.cpu.cycles + 100);
    assert_eq!(m.cpu.r[20], 1);
    m.set_pin_input(10, ExtDrive::Low, 0.0);
    m.run(m.cpu.cycles + 100);
    assert_eq!(m.cpu.r[20], 1, "no interrupt on the falling edge");
    assert_eq!(m.peek_data(0x54) & 0x40, 0x40, "ISC2 is a plain MCUCSR bit");
    // ISC2 = 0: falling edge; INT2 is vector 3 on the ATmega32.
    let mut m = m32(&src("0"));
    m.set_pin_input(10, ExtDrive::Low, 0.0);
    m.run(200);
    m.set_pin_input(10, ExtDrive::High, 0.0);
    m.run(m.cpu.cycles + 100);
    assert_eq!(m.cpu.r[20], 0);
    m.set_pin_input(10, ExtDrive::Low, 0.0);
    m.run(m.cpu.cycles + 100);
    assert_eq!(m.cpu.r[20], 1);
}

#[test]
fn atmega16_adc_single_ended_and_differential_gain() {
    let src = |admux: &str| format!("reset:\n ldi r16, {admux}\n out ADMUX, r16\n ldi r16, (1<<ADEN)|(1<<ADSC)|(1<<ADPS1)|(1<<ADPS0)\n out ADCSRA, r16\nw: sbic ADCSRA, ADSC\n rjmp w\n in r20, ADCL\n in r21, ADCH\n break\n");
    // ADC1 = PA1 against AVCC.
    let mut m = m16(&src("(1<<REFS0)|1"));
    m.set_pin_input(1, ExtDrive::Analog, 2.5);
    assert_eq!(m.run(10_000), StopReason::BreakInsn);
    assert_eq!(adc_result(&m), 512);
    // MUX4:0 = 01001: ADC1 - ADC0 at 10x against 2.56 V, two's complement 10-bit result.
    let mut m = m16(&src("(3<<REFS0)|9"));
    m.set_pin_input(1, ExtDrive::Analog, 0.31);
    m.set_pin_input(0, ExtDrive::Analog, 0.20);
    m.run(10_000);
    let expect = (((0.31f64 - 0.20) * 10.0 * 512.0) / 2.56).floor() as i32;
    assert_eq!(adc_result(&m), expect as u16);
    let mut m = m16(&src("(3<<REFS0)|9"));
    m.set_pin_input(1, ExtDrive::Analog, 0.20);
    m.set_pin_input(0, ExtDrive::Analog, 0.31);
    m.run(10_000);
    let expect = (((0.20f64 - 0.31) * 10.0 * 512.0) / 2.56).floor() as i32;
    assert!(expect < 0);
    assert_eq!(adc_result(&m), (expect as u16) & 0x3ff);
    // MUX4:0 = 10110: ADC6 - ADC1 at 1x; 11110 = 1.22 V bandgap against AVCC.
    let mut m = m16(&src("(1<<REFS0)|0b10110"));
    m.set_pin_input(6, ExtDrive::Analog, 3.0);
    m.set_pin_input(1, ExtDrive::Analog, 1.0);
    m.run(10_000);
    assert_eq!(adc_result(&m), ((2.0f64 * 512.0) / 5.0).floor() as u16);
    let mut m = m16(&src("(1<<REFS0)|0b11110"));
    m.run(10_000);
    assert_eq!(adc_result(&m), ((1.22f64 * 1024.0) / 5.0).floor() as u16);
}

#[test]
fn adc_trigger_select_in_sfior_and_atmega8_free_running() {
    // ATmega16: ADATE with ADTS = 100 (Timer0 overflow) starts a conversion on the overflow; the
    // analog comparator trigger (ADTS = 001) never fires.
    let prog = |adts: u8| format!(
        "reset:\n ldi r16, {adts}<<ADTS0\n out SFIOR, r16\n ldi r16, (1<<ADEN)|(1<<ADATE)|(1<<ADPS1)|(1<<ADPS0)\n out ADCSRA, r16\n in r18, ADCSRA\n ldi r16, 1<<CS00\n out TCCR0, r16\n ldi r17, 255\nw: dec r17\n brne w\n ldi r17, 255\nw2: dec r17\n brne w2\n in r19, ADCSRA\n break\n"
    );
    let mut m = m16(&prog(4));
    assert_eq!(m.run(3000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[18] & 0x50, 0, "no conversion before the overflow");
    assert_ne!(m.cpu.r[19] & 0x50, 0, "conversion started by the Timer0 overflow (ADSC or ADIF)");
    let mut m = m16(&prog(1));
    assert_eq!(m.run(3000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[19] & 0x50, 0);
    // ATmega8: ADCSRA bit 5 is ADFR; three conversions complete back to back without ADSC rewrites.
    let mut m = m8(
        "reset:\n ldi r16, (1<<ADEN)|(1<<ADSC)|(1<<ADFR)|(1<<ADPS1)|(1<<ADPS0)\n out ADCSRA, r16\n ldi r20, 0\nw: sbis ADCSRA, ADIF\n rjmp w\n sbi ADCSRA, ADIF\n inc r20\n cpi r20, 3\n brne w\nl: rjmp l\n",
    );
    m.set_pin_input(8, ExtDrive::Analog, 1.0);
    m.run(200);
    assert_eq!(m.cpu.r[20], 0, "the first conversion takes 25 ADC clocks");
    m.run(800);
    assert_eq!(m.cpu.r[20], 3, "free running: back-to-back conversions without ADSC rewrites");
}

#[test]
fn sfior_pud_disables_pull_ups() {
    let mut m = m16("reset:\n sbi PORTA, PORTA0\n break\n ldi r16, 1<<PUD\n out SFIOR, r16\n break\n ldi r16, 0\n out SFIOR, r16\n break\n");
    assert_eq!(m.run(1000), StopReason::BreakInsn);
    assert_eq!(m.sys.pins[0].pullup, 1, "pull-up on PA0");
    assert_eq!(m.run(2000), StopReason::BreakInsn);
    assert_eq!(m.sys.pins[0].pullup, 0, "PUD set");
    assert_eq!(m.peek_data(0x50) & 0x04, 0x04);
    assert_eq!(m.run(3000), StopReason::BreakInsn);
    assert_eq!(m.sys.pins[0].pullup, 1, "PUD cleared");
}

#[test]
fn legacy_watchdog_sequence_and_timeout() {
    // WDE can be set and WDP changed at any time; clearing WDE needs WDTOE|WDE, then WDE = 0.
    let mut m = m16(
        "reset:\n ldi r16, 1<<WDE\n out WDTCR, r16\n ldi r16, (1<<WDE)|7\n out WDTCR, r16\n in r20, WDTCR\n ldi r16, 0\n out WDTCR, r16\n in r21, WDTCR\n \
         ldi r16, (1<<WDTOE)|(1<<WDE)\n out WDTCR, r16\n ldi r16, 0\n out WDTCR, r16\n in r22, WDTCR\n break\n",
    );
    assert_eq!(m.run(1000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20], 0x0f, "WDP changed while enabled");
    assert_eq!(m.cpu.r[21] & 0x08, 0x08, "WDE cannot be cleared without the sequence");
    assert_eq!(m.cpu.r[22], 0x00, "disabled by the timed sequence");
    // Time-out: 16K cycles of the 1 MHz watchdog oscillator (about 16 ms) without WDR.
    let mut m = m8("reset:\n ldi r16, 1<<WDE\n out WDTCR, r16\nl: rjmp l\n");
    m.run(20_000);
    assert_eq!(m.sys.last_reset, ResetSource::Watchdog);
    assert_eq!(m.peek_data(0x54) & 0x08, 0x08, "MCUCSR.WDRF");
    // WDR keeps the watchdog from timing out.
    let mut m = m8("reset:\n ldi r16, 1<<WDE\n out WDTCR, r16\nl: wdr\n rjmp l\n");
    m.run(100_000);
    assert_eq!(m.sys.last_reset, ResetSource::PowerOn);
}

#[test]
fn atmega16_power_down_wakes_on_int0_with_non_contiguous_sm_bits() {
    // MCUCR: SM2 = bit 7, SE = bit 6, SM1:0 = bits 5:4 -> SM1 + SE is power-down.
    let mut m = m16(
        ".org INT0addr\n jmp isr\nreset:\n sbi PORTD, PORTD2\n ldi r16, 1<<INT0\n out GICR, r16\n ldi r16, (1<<SE)|(1<<SM1)\n out MCUCR, r16\n sei\n sleep\n inc r21\n break\n\
         isr:\n inc r20\n in r18, GICR\n andi r18, 0xff^(1<<INT0)\n out GICR, r18\n reti\n",
    );
    m.run(1000);
    assert!(m.cpu.sleeping);
    assert_eq!(m.cpu.sleep_mode, SleepKind::PowerDown as u8);
    m.set_pin_input(26, ExtDrive::Low, 0.0); // PD2, low level
    assert_eq!(m.run(m.cpu.cycles + 1000), StopReason::BreakInsn);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (1, 1));
    // SM2 + SM1 (110) is standby, SM2 + SM1 + SM0 extended standby.
    let sc = &devices::get("atmega16").unwrap().sleep;
    let mode = |mcucr: u8| sc.modes.iter().find(|e| e.0 == (mcucr & sc.sm_mask) >> sc.sm_mask.trailing_zeros()).map(|e| e.1);
    assert_eq!([mode(0x00), mode(0x10), mode(0x20), mode(0x30), mode(0xa0), mode(0xb0), mode(0x80)], [Some(SleepKind::Idle), Some(SleepKind::AdcNoiseReduction), Some(SleepKind::PowerDown), Some(SleepKind::PowerSave), Some(SleepKind::Standby), Some(SleepKind::ExtendedStandby), None]);
    // ATmega32 / ATmega8: SM2:0 are bits 6:4, SE is bit 7.
    let sc = &devices::get("atmega32").unwrap().sleep;
    assert_eq!((sc.se_mask, sc.sm_mask), (0x80, 0x70));
}

#[test]
fn gicr_ivsel_moves_the_vectors_with_the_timed_sequence() {
    // ATmega8: boot section = last 1024 words (BOOTSZ = 00), starts at word 3072.
    let body = |seq: &str| format!(
        ".org OVF0addr\n rjmp app\n.org 3081\n rjmp boot\nreset:\n {seq}\n ldi r16, 1<<TOIE0\n out TIMSK, r16\n ldi r16, 1<<CS00\n out TCCR0, r16\n sei\nl: rjmp l\napp:\n inc r20\n reti\nboot:\n inc r21\n reti\n"
    );
    let mut m = m8(&body("ldi r16, 1<<IVCE\n out GICR, r16\n ldi r16, 1<<IVSEL\n out GICR, r16"));
    m.run(256 * 4 + 100);
    assert_eq!(m.cpu.r[20], 0, "application vector not used");
    assert!(m.cpu.r[21] >= 3, "boot vector table: {}", m.cpu.r[21]);
    assert_eq!(m.cpu.vector_base, 3072);
    // Without IVCE the IVSEL write is ignored.
    let mut m = m8(&body("ldi r16, 1<<IVSEL\n out GICR, r16"));
    m.run(256 * 4 + 100);
    assert_eq!((m.cpu.r[21], m.cpu.vector_base), (0, 0));
    assert!(m.cpu.r[20] >= 3);
}

#[test]
fn eeprom_write_takes_8_5_ms_and_has_no_eepm_bits() {
    let mut m = m32("reset:\n ldi r16, 1\n out EEARH, r16\n ldi r16, 0xff\n out EEARL, r16\n ldi r16, 0xa5\n out EEDR, r16\n ldi r16, 0x30\n out EECR, r16\n in r22, EECR\n sbi EECR, EEMWE\n sbi EECR, EEWE\nw: sbic EECR, EEWE\n rjmp w\n ldi r16, 0\n out EEDR, r16\n sbi EECR, EERE\n in r20, EEDR\n break\n");
    assert_eq!(m.run(50_000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20], 0xa5);
    assert_eq!(m.cpu.eeprom[0x1ff], 0xa5);
    assert_eq!(m.cpu.r[22] & 0x30, 0, "EECR bits 5:4 do not exist");
    assert!((8_400..9_000).contains(&m.cpu.cycles), "8.5 ms at 1 MHz: {}", m.cpu.cycles);
}

#[test]
fn assembler_include_has_legacy_names_and_both_ubrrh_and_ucsrc() {
    // Compiles all of these names (OC0addr is the ATmega16 Timer0 compare vector).
    let m = m16(".org OC0addr\n jmp isr\n.org URXCaddr\n jmp isr\n.org SPMRaddr\n jmp isr\nreset:\n in r16, UBRRH\n in r17, UCSRC\n in r18, OSCCAL\n in r19, MCUCSR\nisr: reti\n");
    assert_eq!(m.spec.vector("TIMER0_COMP"), Some(19));
}
