//! ATmega328P and ATtiny85 (classic AVR families): peripherals exercised by small assembly
//! programs built with the MCS assembler and the generated m328Pdef.inc / tn85def.inc.

use mcs_asm::{assemble, AssembleOptions};
use mcs_core::avr::devices;
use mcs_sim::avr::{Machine, ResetSource, StopReason};
use mcs_sim::avr::peripherals::serial::SerialConfig;
use mcs_sim::pins::ExtDrive;

fn build(dev: &str, src: &str) -> Machine {
    let r = assemble(src, &AssembleOptions::new("t.asm", dev));
    assert!(r.ok, "{:#?}", r.diagnostics);
    let mut m = Machine::new(devices::get(&r.device_id).unwrap());
    m.load(&r.program);
    m
}

fn mega(body: &str) -> Machine {
    build("atmega328p", &format!(".include \"m328Pdef.inc\"\n.org 0\n    jmp reset\n{body}"))
}

fn tiny(body: &str) -> Machine {
    build("attiny85", &format!(".include \"tn85def.inc\"\n.org 0\n    rjmp reset\n{body}"))
}

/// Cycles at which GPIO `g` changed level.
fn edges(m: &Machine, g: u32) -> Vec<u64> {
    let (_, c, l) = m.sys.trace.read_since(0, usize::MAX);
    (1..c.len()).filter(|&i| (l[i] ^ l[i - 1]) >> g & 1 == 1).map(|i| c[i]).collect()
}

/// (rise, fall) cycle lists of GPIO `g`.
fn rises_falls(m: &Machine, g: u32) -> (Vec<u64>, Vec<u64>) {
    let (_, c, l) = m.sys.trace.read_since(0, usize::MAX);
    let r = (1..c.len()).filter(|&i| l[i] >> g & 1 == 1 && l[i - 1] >> g & 1 == 0).map(|i| c[i]).collect();
    let f = (1..c.len()).filter(|&i| l[i] >> g & 1 == 0 && l[i - 1] >> g & 1 == 1).map(|i| c[i]).collect();
    (r, f)
}

// GPIO numbers on the ATmega328P: PB0-7 = 0-7, PC0-6 = 8-14, PD0-7 = 15-22.
const PB5: usize = 5;
const PC0: usize = 8;
const PD0: usize = 15;
const PD1: usize = 16;
const PD2: usize = 17;
const PD6: usize = 21;

#[test]
fn mega_reset_defaults_and_gpio() {
    let mut m = mega("reset:\n sbi DDRB, DDB5\nloop:\n sbi PINB, PINB5\n rjmp loop\n");
    assert_eq!(m.sys.clock.hz, 1e6, "8 MHz RC with CKDIV8");
    assert_eq!(m.cpu.sp, 0x08ff);
    assert_eq!(m.peek_data(0x54) & 0x01, 0x01, "PORF");
    assert!(m.sys.pins[14].reserved, "PC6 is RESET");
    m.run(400);
    let e = edges(&m, PB5 as u32);
    assert!(e.len() > 50);
    // SBI (2 cycles) + RJMP (2 cycles) per toggle on the classic core.
    assert!(e.windows(2).all(|w| w[1] - w[0] == 4), "{:?}", &e[..5]);
}

#[test]
fn mega_clkpr_timed_sequence() {
    let mut m = mega(
        "reset:\n ldi r16, 0\n sts CLKPR, r16\n ldi r17, 1<<CLKPCE\n ldi r16, 0\n sts CLKPR, r17\n sts CLKPR, r16\n break\n",
    );
    assert_eq!(m.run(1000), StopReason::BreakInsn);
    assert_eq!(m.sys.clock.hz, 8e6);
    assert_eq!(m.peek_data(0x61), 0);
}

#[test]
fn mega_fuses_select_crystal_and_boot_reset() {
    let mut m = mega("reset:\n rjmp reset\n");
    m.cpu.fuses[0] = 0xff; // crystal, CKDIV8 unprogrammed (Arduino Uno)
    m.set_external_clock(16e6);
    m.power_on();
    assert_eq!(m.sys.clock.hz, 16e6);
    assert_eq!(m.sys.pins[6].reserved_by, "XTAL1");
    assert_eq!(m.sys.pins[7].reserved_by, "XTAL2");
    m.cpu.fuses[1] = 0xd8; // BOOTRST programmed, BOOTSZ = 00 (2048 words)
    m.power_on();
    assert_eq!(m.cpu.pc, 0x3800);
}

#[test]
fn mega_timer0_fast_pwm_on_oc0a() {
    let mut m = mega(
        "reset:\n sbi DDRD, DDD6\n ldi r16, 64\n out OCR0A, r16\n ldi r16, (1<<COM0A1)|(1<<WGM01)|(1<<WGM00)\n out TCCR0A, r16\n ldi r16, 1<<CS00\n out TCCR0B, r16\nl: rjmp l\n",
    );
    m.run(5000);
    let (r, f) = rises_falls(&m, PD6 as u32);
    assert!(r.len() > 10);
    assert_eq!(r[5] - r[4], 256);
    let fall = f.iter().find(|&&x| x > r[4]).unwrap();
    assert_eq!(fall - r[4], 65);
}

#[test]
fn mega_timer1_ctc_interrupt_with_jmp_vectors() {
    let mut m = mega(
        ".org OC1Aaddr\n jmp isr\n.org INT_VECTORS_SIZE\nreset:\n ldi r16, high(999)\n sts OCR1AH, r16\n ldi r16, low(999)\n sts OCR1AL, r16\n ldi r16, 1<<OCIE1A\n sts TIMSK1, r16\n ldi r16, (1<<WGM12)|(1<<CS10)\n sts TCCR1B, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n",
    );
    m.run(10_500);
    assert_eq!(m.cpu.r[20], 10);
}

#[test]
fn mega_timer2_overflow_and_power_save() {
    let mut m = mega(
        ".org OVF2addr\n jmp isr\n.org INT_VECTORS_SIZE\nreset:\n ldi r16, 1<<TOIE2\n sts TIMSK2, r16\n ldi r16, 1<<CS20\n sts TCCR2B, r16\n ldi r16, (3<<SM0)|(1<<SE)\n out SMCR, r16\n sei\nl: sleep\n rjmp l\nisr:\n inc r20\n reti\n",
    );
    m.run(256 * 10 + 300);
    // Timer2 keeps running (and wakes the CPU) in power-save: one overflow every 256 cycles.
    assert_eq!(m.cpu.r[20], 11);
}

#[test]
fn mega_usart_transmits_and_echoes_through_the_pins() {
    let mut m = mega(
        ".org INT_VECTORS_SIZE\nreset:\n ldi r16, 0\n sts UBRR0H, r16\n ldi r16, 12\n sts UBRR0L, r16\n ldi r16, 1<<U2X0\n sts UCSR0A, r16\n ldi r16, (1<<TXEN0)|(1<<RXEN0)\n sts UCSR0B, r16\n ldi r16, 3<<UCSZ00\n sts UCSR0C, r16\n ldi ZL, low(msg*2)\n ldi ZH, high(msg*2)\nsend:\n lpm r17, Z+\n tst r17\n breq echo\nw1: lds r16, UCSR0A\n sbrs r16, UDRE0\n rjmp w1\n sts UDR0, r17\n rjmp send\necho:\n lds r16, UCSR0A\n sbrs r16, RXC0\n rjmp echo\n lds r17, UDR0\nw2: lds r16, UCSR0A\n sbrs r16, UDRE0\n rjmp w2\n sts UDR0, r17\n rjmp echo\nmsg: .db \"Hi!\", 0\n",
    );
    m.set_serial(SerialConfig { monitor: Some(PD1), inject: Some(PD0), baud: 9600.0, data_bits: 8, parity: 0, stop_bits: 1 });
    m.run(50_000);
    assert_eq!(std::mem::take(&mut m.sys.serial_out), b"Hi!");
    m.serial_send(b"ok");
    let c = m.cpu.cycles;
    m.run(c + 50_000);
    assert_eq!(m.sys.serial_out, b"ok");
}

#[test]
fn mega_eeprom_write_read_and_persistence() {
    let mut m = mega(
        "reset:\n ldi r16, 0\n out EEARH, r16\n ldi r16, 0x10\n out EEARL, r16\n ldi r16, 0x5a\n out EEDR, r16\n sbi EECR, EEMPE\n sbi EECR, EEPE\nw: sbic EECR, EEPE\n rjmp w\n sbi EECR, EERE\n in r20, EEDR\n break\n",
    );
    assert_eq!(m.run(10_000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20], 0x5a);
    assert_eq!(m.cpu.eeprom[0x10], 0x5a);
    // About 3.4 ms of programming at 1 MHz.
    assert!((3300..4000).contains(&m.cpu.cycles), "{}", m.cpu.cycles);
    m.power_on();
    assert_eq!(m.cpu.eeprom[0x10], 0x5a, "EEPROM is non-volatile");
}

#[test]
fn mega_eeprom_needs_eempe() {
    let mut m = mega("reset:\n sbi EECR, EEPE\n nop\n nop\n break\n");
    m.run(1000);
    assert_eq!(m.cpu.eeprom[0], 0xff);
    assert_eq!(m.peek_data(0x3f) & 0x02, 0);
}

#[test]
fn mega_adc_10_bit_and_left_adjust() {
    let src = |admux: &str| format!("reset:\n ldi r16, {admux}\n sts ADMUX, r16\n ldi r16, (1<<ADEN)|(1<<ADSC)|(1<<ADPS1)|(1<<ADPS0)\n sts ADCSRA, r16\nw: lds r16, ADCSRA\n sbrc r16, ADSC\n rjmp w\n lds r20, ADCL\n lds r21, ADCH\n break\n");
    let mut m = mega(&src("1<<REFS0"));
    m.set_pin_input(PC0, ExtDrive::Analog, 2.5);
    assert_eq!(m.run(10_000), StopReason::BreakInsn);
    assert_eq!((m.cpu.r[21] as u16) << 8 | m.cpu.r[20] as u16, 512);
    let mut m = mega(&src("(1<<REFS0)|(1<<ADLAR)"));
    m.set_pin_input(PC0, ExtDrive::Analog, 2.5);
    m.run(10_000);
    assert_eq!(m.cpu.r[21], 0x80);
    // Internal 1.1 V bandgap measured against AVCC.
    let mut m = mega(&src("(1<<REFS0)|14"));
    m.run(10_000);
    let v = (m.cpu.r[21] as u16) << 8 | m.cpu.r[20] as u16;
    assert_eq!(v, (1.1 / 5.0 * 1024.0) as u16);
}

#[test]
fn mega_int0_falling_edge_and_pcint2() {
    let mut m = mega(
        ".org INT0addr\n jmp isr0\n.org PCI2addr\n jmp isr2\n.org INT_VECTORS_SIZE\nreset:\n ldi r16, 1<<ISC01\n sts EICRA, r16\n sbi EIMSK, INT0\n ldi r16, 1<<PCIE2\n sts PCICR, r16\n ldi r16, 1<<PCINT16\n sts PCMSK2, r16\n sbi PORTD, PORTD2\n sei\nl: rjmp l\nisr0:\n inc r20\n reti\nisr2:\n inc r21\n reti\n",
    );
    m.run(200);
    m.set_pin_input(PD2, ExtDrive::Low, 0.0);
    let c = m.cpu.cycles;
    m.run(c + 100);
    m.set_pin_input(PD2, ExtDrive::Float, 0.0); // pull-up: rising edge, ignored
    let c = m.cpu.cycles;
    m.run(c + 100);
    assert_eq!(m.cpu.r[20], 1);
    m.set_pin_input(PD0, ExtDrive::High, 0.0);
    m.set_pin_input(PD0, ExtDrive::Low, 0.0);
    let c = m.cpu.cycles;
    m.run(c + 100);
    assert!(m.cpu.r[21] >= 1);
}

#[test]
fn mega_pull_up_disable() {
    let mut m = mega("reset:\n sbi PORTB, PORTB0\n nop\n ldi r16, 1<<PUD\n out MCUCR, r16\n nop\n break\n");
    m.run(4);
    assert_eq!(m.sys.pins[0].pullup, 1);
    m.run(100);
    assert_eq!(m.sys.pins[0].pullup, 0);
}

#[test]
fn mega_spi_master_transfer() {
    let mut m = mega(
        "reset:\n ldi r16, (1<<DDB2)|(1<<DDB3)|(1<<DDB5)\n out DDRB, r16\n ldi r16, (1<<SPE)|(1<<MSTR)\n out SPCR, r16\n ldi r16, 0xa5\n out SPDR, r16\nw: in r16, SPSR\n sbrs r16, SPIF\n rjmp w\n in r20, SPDR\n break\n",
    );
    m.set_pin_input(4, ExtDrive::High, 0.0); // MISO
    assert_eq!(m.run(2000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20], 0xff);
    // fosc/4: 16 SCK edges, 2 cycles apart.
    let sck = edges(&m, 5);
    assert_eq!(sck.len(), 16);
    assert!(sck.windows(2).all(|w| w[1] - w[0] == 2));
    // MOSI carries 0xA5 MSB first: sample it at the rising (leading) edges.
    let (_, c, l) = m.sys.trace.read_since(0, usize::MAX);
    let level_at = |cy: u64, g: u32| (0..c.len()).rev().find(|&i| c[i] <= cy).map(|i| l[i] >> g & 1).unwrap();
    let bits: Vec<u32> = sck.iter().step_by(2).map(|&e| level_at(e, 3)).collect();
    assert_eq!(bits, [1, 0, 1, 0, 0, 1, 0, 1]);
}

#[test]
fn mega_twi_on_an_empty_bus_nacks() {
    let mut m = mega(
        "reset:\n ldi r16, 72\n sts TWBR, r16\n ldi r16, (1<<TWINT)|(1<<TWSTA)|(1<<TWEN)\n sts TWCR, r16\nw1: lds r16, TWCR\n sbrs r16, TWINT\n rjmp w1\n lds r20, TWSR\n ldi r16, 0xa0\n sts TWDR, r16\n ldi r16, (1<<TWINT)|(1<<TWEN)\n sts TWCR, r16\nw2: lds r16, TWCR\n sbrs r16, TWINT\n rjmp w2\n lds r21, TWSR\n ldi r16, (1<<TWINT)|(1<<TWSTO)|(1<<TWEN)\n sts TWCR, r16\n break\n",
    );
    assert_eq!(m.run(10_000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20] & 0xf8, 0x08, "START transmitted");
    assert_eq!(m.cpu.r[21] & 0xf8, 0x20, "SLA+W not acknowledged");
}

#[test]
fn mega_watchdog_timed_sequence_and_reset() {
    let mut m = mega(
        "reset:\n in r16, MCUSR\n sbrc r16, WDRF\n break\n wdr\n ldi r16, (1<<WDCE)|(1<<WDE)\n sts WDTCSR, r16\n ldi r16, (1<<WDE)|(1<<WDP0)\n sts WDTCSR, r16\nl: rjmp l\n",
    );
    // 32 ms timeout (WDP = 1) at 1 MHz.
    assert_eq!(m.run(60_000), StopReason::BreakInsn);
    assert_eq!(m.sys.last_reset, ResetSource::Watchdog);
    assert!((32_000..33_000).contains(&m.cpu.cycles), "{}", m.cpu.cycles);
}

#[test]
fn mega_brown_out_holds_the_mcu_in_reset() {
    let mut m = mega("reset:\n rjmp reset\n");
    m.cpu.fuses[2] = 0xfd; // BODLEVEL = 101: 2.7 V
    m.power_on();
    m.set_vcc(2.5);
    assert!(m.sys.reset_held);
    m.set_vcc(3.3);
    assert!(!m.sys.reset_held);
    assert_eq!(m.peek_data(0x54) & 0x04, 0x04, "BORF");
}

// ------------------------------------------------------------------------------- ATtiny85

#[test]
fn tiny85_defaults_and_pin_change_wake_from_power_down() {
    let mut m = tiny(
        ".org PCI0addr\n rjmp isr\nreset:\n sbi PORTB, PORTB3\n ldi r16, 1<<PCIE\n out GIMSK, r16\n sbi PCMSK, PCINT3\n ldi r16, (1<<SE)|(1<<SM1)\n out MCUCR, r16\n sei\n sleep\n inc r21\n break\nisr:\n inc r20\n reti\n",
    );
    assert_eq!(m.sys.clock.hz, 1e6);
    assert!(m.sys.pins[5].reserved, "PB5 is RESET");
    m.run(1000);
    assert!(m.cpu.sleeping);
    m.set_pin_input(3, ExtDrive::Low, 0.0);
    assert_eq!(m.run(m.cpu.cycles + 1000), StopReason::BreakInsn);
    assert_eq!((m.cpu.r[20], m.cpu.r[21]), (1, 1));
}

#[test]
fn tiny85_int0_sense_control_in_mcucr() {
    let mut m = tiny(".org INT0addr\n rjmp isr\nreset:\n ldi r16, 1<<ISC01\n out MCUCR, r16\n ldi r16, 1<<INT0\n out GIMSK, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n");
    m.set_pin_input(2, ExtDrive::High, 0.0);
    m.run(200);
    m.set_pin_input(2, ExtDrive::Low, 0.0);
    m.run(m.cpu.cycles + 100);
    m.set_pin_input(2, ExtDrive::High, 0.0);
    m.run(m.cpu.cycles + 100);
    assert_eq!(m.cpu.r[20], 1);
}

#[test]
fn tiny85_timer0_pwm_and_shared_flags() {
    let mut m = tiny(
        ".org OVF0addr\n rjmp isr\nreset:\n sbi DDRB, DDB0\n ldi r16, 100\n out OCR0A, r16\n ldi r16, (1<<COM0A1)|(1<<WGM01)|(1<<WGM00)\n out TCCR0A, r16\n ldi r16, 1<<TOIE0\n out TIMSK, r16\n ldi r16, 1<<CS00\n out TCCR0B, r16\n sei\nl: rjmp l\nisr:\n inc r20\n reti\n",
    );
    m.run(256 * 8 + 100);
    let (r, f) = rises_falls(&m, 0);
    assert_eq!(r[3] - r[2], 256);
    assert_eq!(f.iter().find(|&&x| x > r[2]).unwrap() - r[2], 101);
    assert!((7..=8).contains(&m.cpu.r[20]));
}

#[test]
fn tiny85_timer1_pwm_with_ocr1c_top_and_pll_clock() {
    let src = |cs: &str, pll: bool| {
        let pll = if pll { " ldi r16, 1<<PLLE\n out PLLCSR, r16\nwl: in r16, PLLCSR\n sbrs r16, PLOCK\n rjmp wl\n ldi r16, (1<<PLLE)|(1<<PCKE)\n out PLLCSR, r16\n" } else { "" };
        format!("reset:\n{pll} sbi DDRB, DDB1\n sbi DDRB, DDB0\n ldi r16, 99\n out OCR1C, r16\n ldi r16, 25\n out OCR1A, r16\n ldi r16, (1<<PWM1A)|(1<<COM1A0)|{cs}\n out TCCR1, r16\nl: rjmp l\n")
    };
    // CK/1: period OCR1C + 1 = 100 cycles, OC1A high for 26 of them, !OC1A complementary.
    let mut m = tiny(&src("(1<<CS10)", false));
    m.run(2000);
    let (r, f) = rises_falls(&m, 1);
    assert_eq!(r[5] - r[4], 100);
    assert_eq!(f.iter().find(|&&x| x > r[4]).unwrap() - r[4], 26);
    let (r0, _) = rises_falls(&m, 0);
    assert!(r0.iter().any(|&x| f.contains(&x)), "!OC1A rises when OC1A falls");
    // PCK = 64 MHz with /64: one tick per microsecond, same period as CK/1 at 1 MHz.
    let mut m = tiny(&src("(1<<CS12)|(1<<CS11)|(1<<CS10)", true));
    m.run(5000);
    let (r, _) = rises_falls(&m, 1);
    assert!(r.len() > 10);
    assert_eq!(r[6] - r[5], 100);
}

#[test]
fn tiny85_adc_internal_reference_and_bandgap() {
    let mut m = tiny(
        "reset:\n ldi r16, (1<<REFS1)|1\n out ADMUX, r16\n ldi r16, (1<<ADEN)|(1<<ADSC)|(1<<ADPS1)|(1<<ADPS0)\n out ADCSRA, r16\nw: sbic ADCSRA, ADSC\n rjmp w\n in r20, ADCL\n in r21, ADCH\n break\n",
    );
    m.set_pin_input(2, ExtDrive::Analog, 0.55); // ADC1 = PB2
    assert_eq!(m.run(10_000), StopReason::BreakInsn);
    assert_eq!((m.cpu.r[21] as u16) << 8 | m.cpu.r[20] as u16, 512);
}

#[test]
fn tiny85_usi_three_wire_software_strobes() {
    let mut m = tiny(
        "reset:\n sbi DDRB, DDB1\n ldi r16, 0xa5\n out USIDR, r16\n ldi r16, 1<<USIWM0\n out USICR, r16\n ldi r17, (1<<USIWM0)|(1<<USICLK)\n ldi r18, 8\nl: out USICR, r17\n dec r18\n brne l\n in r20, USISR\n break\n",
    );
    m.set_pin_input(0, ExtDrive::High, 0.0); // DI shifts ones in
    assert_eq!(m.run(1000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20] & 0x0f, 8);
    assert_eq!(m.peek_data(0x2f), 0xff);
    // DO showed the bits of 0xA5, MSB first.
    let (_, _, l) = m.sys.trace.read_since(0, usize::MAX);
    let mut seq: Vec<u32> = Vec::new();
    for v in &l {
        let b = v >> 1 & 1;
        if seq.last() != Some(&b) {
            seq.push(b);
        }
    }
    assert_eq!(seq, [0, 1, 0, 1, 0, 1, 0, 1], "1 0 1 0 0 1 0 1 collapses to alternating levels");
}

