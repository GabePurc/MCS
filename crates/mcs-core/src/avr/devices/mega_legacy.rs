//! ATmega8 / ATmega16 / ATmega32 ("legacy mega": classic AVR core with the pre-ATmega48 peripheral
//! generation: single TCCRn registers on Timer0/2, UBRRH/UCSRC sharing one address, SFIOR, GICR,
//! MCUCSR, one-bit BODLEVEL + BODEN fuses, internal RC at 1/2/4/8 MHz, no CLKPR / PRR).
//!
//! Sources: Atmel-2486AA ATmega8/L (02/2013), Atmel-2466T ATmega16/L (07/2010), Atmel-2503Q
//! ATmega32/L (02/2011): register summaries, fuse tables, "System Clock and Clock Options",
//! "Power Management and Sleep Modes" (table 14), interrupt vector tables and pinouts; avr-libc
//! iom8.h / iom16.h / iom32.h for register addresses and bit names. Where avr-libc's fuse-default
//! macros disagree with the datasheet text, the datasheet wins (factory values E1 / D9 and E1 / 99).
//!
//! Deliberate simplifications:
//! - Timer2 asynchronous operation (ASSR.AS2, TOSC 32.768 kHz crystal) is not modelled: ASSR is a
//!   plain register, Timer2 runs from the system clock and stops in power-save, so it is not a
//!   wake-up source there.
//! - JTAG / On-chip debug (ATmega16/32): the JTAGEN fuse is programmed at the factory, which takes
//!   PC2-PC5 on real silicon; here those pins stay ordinary GPIO. OCDR (shares the OSCCAL address
//!   on the ATmega16) and the JTAG programming interface are not modelled.
//! - USART synchronous (UMSEL = 1) and master SPI modes are not simulated; SPM is not simulated.
//! - The dedicated RESET pin of the 40-pin parts has no GPIO slot (the pin trace is a 32-bit mask
//!   and the ATmega32 already has 32 GPIOs), so an external reset cannot be driven from the pin
//!   panel on the ATmega16/32; the ATmega8 uses PC6 as RESET (RSTDISBL fuse) like the ATmega328P.
//! - Stack pointer: the simulator starts every part with SP = RAMEND, whereas these three
//!   datasheets list 0x0000 as the initial value.

use crate::avr::device::*;
use crate::avr::isa::feature;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    M8,
    M16,
    M32,
}

struct Variant {
    id: &'static str,
    name: &'static str,
    kind: Kind,
    flash: u32,
    sram: u16,
    eeprom: u16,
    signature: [u8; 3],
    datasheet: &'static str,
}

const VARIANTS: &[Variant] = &[
    Variant { id: "atmega8", name: "ATmega8", kind: Kind::M8, flash: 8192, sram: 1024, eeprom: 512, signature: [0x1e, 0x93, 0x07], datasheet: "Atmel-2486AA ATmega8/L (02/2013)" },
    Variant { id: "atmega16", name: "ATmega16", kind: Kind::M16, flash: 16384, sram: 1024, eeprom: 512, signature: [0x1e, 0x94, 0x03], datasheet: "Atmel-2466T ATmega16/L (07/2010)" },
    Variant { id: "atmega32", name: "ATmega32", kind: Kind::M32, flash: 32768, sram: 2048, eeprom: 1024, signature: [0x1e, 0x95, 0x02], datasheet: "Atmel-2503Q ATmega32/L (02/2011)" },
];

impl Variant {
    /// ATmega16/32: PORTA, INT2, Timer0 compare unit, differential ADC, JTAG.
    fn big(&self) -> bool {
        self.kind != Kind::M8
    }
}

fn reg(name: &str, addr: u16, group: &str, desc: &str, bits: Vec<BitFieldSpec>) -> IoRegisterSpec {
    IoRegisterSpec { name: name.into(), addr, reset: 0, group: group.into(), desc: desc.into(), bits, access: RegisterAccess::Rw }
}

fn reset(mut r: IoRegisterSpec, v: u8) -> IoRegisterSpec {
    r.reset = v;
    r
}

fn ro(mut r: IoRegisterSpec) -> IoRegisterSpec {
    r.access = RegisterAccess::R;
    r
}

/// Single-bit fields named `prefix7..prefix0` for the bits set in `mask`.
fn nbits(prefix: &str, mask: u8) -> Vec<BitFieldSpec> {
    (0..8).rev().filter(|i| mask & (1 << i) != 0).map(|i| field(&format!("{prefix}{i}"), 1 << i, "")).collect()
}

fn b(name: &str, mask: u8, desc: &str) -> BitFieldSpec {
    field(name, mask, desc)
}

fn tccr_single(n: u8, desc: &str) -> Vec<BitFieldSpec> {
    vec![
        b(&format!("FOC{n}"), 0x80, "Force Output Compare"), b(&format!("WGM{n}0"), 0x40, "Waveform Generation Mode bit 0"),
        b(&format!("COM{n}1"), 0x20, "Compare Output Mode bit 1"), b(&format!("COM{n}0"), 0x10, "Compare Output Mode bit 0"),
        b(&format!("WGM{n}1"), 0x08, "Waveform Generation Mode bit 1"), b(&format!("CS{n}"), 0x07, desc),
    ]
}

/// Registers in data-space addresses (I/O address + 0x20).
fn registers(v: &Variant) -> Vec<IoRegisterSpec> {
    let big = v.big();
    let ramend = 0x60 + v.sram as u32 - 1;
    let mut r = vec![
        reg("TWBR", 0x20, "TWI", "TWI Bit Rate Register", vec![]),
        reset(reg("TWSR", 0x21, "TWI", "TWI Status Register", vec![b("TWS", 0xf8, "TWI Status"), b("TWPS", 0x03, "TWI Prescaler (1, 4, 16, 64)")]), 0xf8),
        reset(reg("TWAR", 0x22, "TWI", "TWI (Slave) Address Register", vec![b("TWA", 0xfe, "TWI Slave Address"), b("TWGCE", 0x01, "General Call Recognition Enable")]), 0xfe),
        reset(reg("TWDR", 0x23, "TWI", "TWI Data Register", vec![]), 0xff),
        ro(reg("ADCL", 0x24, "ADC", "ADC Data Register Low Byte (read first)", vec![])),
        ro(reg("ADCH", 0x25, "ADC", "ADC Data Register High Byte", vec![])),
        reg("ADCSRA", 0x26, "ADC", "ADC Control and Status Register A", vec![
            b("ADEN", 0x80, "ADC Enable"), b("ADSC", 0x40, "ADC Start Conversion"),
            if big { b("ADATE", 0x20, "ADC Auto Trigger Enable (source: SFIOR.ADTS)") } else { b("ADFR", 0x20, "ADC Free Running Select") },
            b("ADIF", 0x10, "ADC Interrupt Flag"), b("ADIE", 0x08, "ADC Interrupt Enable"), b("ADPS", 0x07, "ADC Prescaler Select"),
        ]),
        reg("ADMUX", 0x27, "ADC", "ADC Multiplexer Selection Register", vec![
            b("REFS", 0xc0, "Reference Selection (00 AREF, 01 AVCC, 11 internal 2.56 V)"), b("ADLAR", 0x20, "ADC Left Adjust Result"),
            if big { b("MUX", 0x1f, "Analog Channel and Gain Selection (0-7 ADCn, 8-29 differential, 30 1.22 V, 31 GND)") } else { b("MUX", 0x0f, "Analog Channel Selection (0-7 ADCn, 14 1.30 V, 15 GND)") },
        ]),
        reg("ACSR", 0x28, "AC", "Analog Comparator Control and Status Register", vec![
            b("ACD", 0x80, "Analog Comparator Disable"), b("ACBG", 0x40, "Bandgap Select (bandgap on the positive input)"), b("ACO", 0x20, "Analog Comparator Output"),
            b("ACI", 0x10, "Analog Comparator Interrupt Flag"), b("ACIE", 0x08, "Analog Comparator Interrupt Enable"), b("ACIC", 0x04, "Input Capture Enable (Timer1)"),
            b("ACIS", 0x03, "Interrupt Mode Select (00 toggle, 10 falling, 11 rising)"),
        ]),
        reg("UBRRL", 0x29, "USART", "USART Baud Rate Register Low Byte", vec![]),
        reg("UCSRB", 0x2a, "USART", "USART Control and Status Register B", vec![
            b("RXCIE", 0x80, "RX Complete Interrupt Enable"), b("TXCIE", 0x40, "TX Complete Interrupt Enable"), b("UDRIE", 0x20, "Data Register Empty Interrupt Enable"),
            b("RXEN", 0x10, "Receiver Enable"), b("TXEN", 0x08, "Transmitter Enable"), b("UCSZ2", 0x04, "Character Size bit 2"), b("RXB8", 0x02, "Receive Data Bit 8"), b("TXB8", 0x01, "Transmit Data Bit 8"),
        ]),
        reset(reg("UCSRA", 0x2b, "USART", "USART Control and Status Register A", vec![
            b("RXC", 0x80, "Receive Complete"), b("TXC", 0x40, "Transmit Complete"), b("UDRE", 0x20, "Data Register Empty"), b("FE", 0x10, "Frame Error"),
            b("DOR", 0x08, "Data OverRun"), b("PE", 0x04, "Parity Error"), b("U2X", 0x02, "Double Transmission Speed"), b("MPCM", 0x01, "Multi-processor Communication Mode"),
        ]), 0x20),
        reg("UDR", 0x2c, "USART", "USART I/O Data Register", vec![]),
        reg("SPCR", 0x2d, "SPI", "SPI Control Register", vec![
            b("SPIE", 0x80, "SPI Interrupt Enable"), b("SPE", 0x40, "SPI Enable"), b("DORD", 0x20, "Data Order (1 = LSB first)"), b("MSTR", 0x10, "Master/Slave Select"),
            b("CPOL", 0x08, "Clock Polarity"), b("CPHA", 0x04, "Clock Phase"), b("SPR", 0x03, "SPI Clock Rate Select (fosc/4, /16, /64, /128)"),
        ]),
        reg("SPSR", 0x2e, "SPI", "SPI Status Register", vec![b("SPIF", 0x80, "SPI Interrupt Flag"), b("WCOL", 0x40, "Write Collision Flag"), b("SPI2X", 0x01, "Double SPI Speed")]),
        reg("SPDR", 0x2f, "SPI", "SPI Data Register", vec![]),
        reg("PIND", 0x30, "PORTD", "Port D Input Pins (write 1 toggles PORTD bit)", nbits("PIND", 0xff)),
        reg("DDRD", 0x31, "PORTD", "Port D Data Direction Register", nbits("DDD", 0xff)),
        reg("PORTD", 0x32, "PORTD", "Port D Data Register", nbits("PORTD", 0xff)),
        reg("PINC", 0x33, "PORTC", "Port C Input Pins (write 1 toggles PORTC bit)", nbits("PINC", if big { 0xff } else { 0x7f })),
        reg("DDRC", 0x34, "PORTC", "Port C Data Direction Register", nbits("DDC", if big { 0xff } else { 0x7f })),
        reg("PORTC", 0x35, "PORTC", "Port C Data Register", nbits("PORTC", if big { 0xff } else { 0x7f })),
        reg("PINB", 0x36, "PORTB", "Port B Input Pins (write 1 toggles PORTB bit)", nbits("PINB", 0xff)),
        reg("DDRB", 0x37, "PORTB", "Port B Data Direction Register", nbits("DDB", 0xff)),
        reg("PORTB", 0x38, "PORTB", "Port B Data Register", nbits("PORTB", 0xff)),
        reg("EECR", 0x3c, "EEPROM", "EEPROM Control Register", vec![
            b("EERIE", 0x08, "EEPROM Ready Interrupt Enable"), b("EEMWE", 0x04, "EEPROM Master Write Enable"), b("EEWE", 0x02, "EEPROM Write Enable"), b("EERE", 0x01, "EEPROM Read Enable"),
        ]),
        reg("EEDR", 0x3d, "EEPROM", "EEPROM Data Register", vec![]),
        reg("EEARL", 0x3e, "EEPROM", "EEPROM Address Register Low Byte", vec![]),
        reg("EEARH", 0x3f, "EEPROM", "EEPROM Address Register High Byte", vec![]),
        // UBRRH and UCSRC share this address; URSEL (bit 7) selects the register on writes.
        reg("UBRRH", 0x40, "USART", "USART Baud Rate Register High Byte (URSEL = 0 on write)", vec![b("URSEL", 0x80, "Register Select (0 = UBRRH on write)"), b("UBRR", 0x0f, "Baud Rate bits 11:8")]),
        reset(reg("UCSRC", 0x40, "USART", "USART Control and Status Register C (URSEL = 1 on write)", vec![
            b("URSEL", 0x80, "Register Select (1 = UCSRC on write)"), b("UMSEL", 0x40, "USART Mode Select (0 asynchronous)"), b("UPM", 0x30, "Parity Mode (00 none, 10 even, 11 odd)"),
            b("USBS", 0x08, "Stop Bit Select (1 = 2 stop bits)"), b("UCSZ", 0x06, "Character Size bits 1:0 (11 = 8 bits)"), b("UCPOL", 0x01, "Clock Polarity"),
        ]), 0x86),
        reg("WDTCR", 0x41, "WDT", "Watchdog Timer Control Register", vec![
            if big { b("WDTOE", 0x10, "Watchdog Turn-off Enable") } else { b("WDCE", 0x10, "Watchdog Change Enable") },
            b("WDE", 0x08, "Watchdog Enable"), b("WDP", 0x07, "Watchdog Timer Prescaler bits 2:0"),
        ]),
        reg("ASSR", 0x42, "TC2", "Asynchronous Status Register (asynchronous operation is not simulated)", vec![
            b("AS2", 0x08, "Asynchronous Timer/Counter2"), b("TCN2UB", 0x04, "TCNT2 Update Busy"), b("OCR2UB", 0x02, "OCR2 Update Busy"), b("TCR2UB", 0x01, "TCCR2 Update Busy"),
        ]),
        reg("OCR2", 0x43, "TC2", "Timer/Counter2 Output Compare Register", vec![]),
        reg("TCNT2", 0x44, "TC2", "Timer/Counter2", vec![]),
        reg("TCCR2", 0x45, "TC2", "Timer/Counter2 Control Register", tccr_single(2, "Clock Select (0 stop, 1 /1, 2 /8, 3 /32, 4 /64, 5 /128, 6 /256, 7 /1024)")),
        reg("ICR1L", 0x46, "TC1", "Input Capture Register 1 Low Byte", vec![]),
        reg("ICR1H", 0x47, "TC1", "Input Capture Register 1 High Byte", vec![]),
        reg("OCR1BL", 0x48, "TC1", "Output Compare Register 1 B Low Byte", vec![]),
        reg("OCR1BH", 0x49, "TC1", "Output Compare Register 1 B High Byte", vec![]),
        reg("OCR1AL", 0x4a, "TC1", "Output Compare Register 1 A Low Byte", vec![]),
        reg("OCR1AH", 0x4b, "TC1", "Output Compare Register 1 A High Byte", vec![]),
        reg("TCNT1L", 0x4c, "TC1", "Timer/Counter1 Low Byte", vec![]),
        reg("TCNT1H", 0x4d, "TC1", "Timer/Counter1 High Byte", vec![]),
        reg("TCCR1B", 0x4e, "TC1", "Timer/Counter1 Control Register B", vec![
            b("ICNC1", 0x80, "Input Capture Noise Canceler"), b("ICES1", 0x40, "Input Capture Edge Select (1 = rising)"), b("WGM13", 0x10, "Waveform Generation Mode bit 3"),
            b("WGM12", 0x08, "Waveform Generation Mode bit 2"), b("CS1", 0x07, "Clock Select (0 stop, 1 /1, 2 /8, 3 /64, 4 /256, 5 /1024, 6 T1 falling, 7 T1 rising)"),
        ]),
        reg("TCCR1A", 0x4f, "TC1", "Timer/Counter1 Control Register A", vec![
            b("COM1A1", 0x80, "Compare Output Mode A bit 1"), b("COM1A0", 0x40, "Compare Output Mode A bit 0"), b("COM1B1", 0x20, "Compare Output Mode B bit 1"), b("COM1B0", 0x10, "Compare Output Mode B bit 0"),
            b("FOC1A", 0x08, "Force Output Compare A"), b("FOC1B", 0x04, "Force Output Compare B"), b("WGM11", 0x02, "Waveform Generation Mode bit 1"), b("WGM10", 0x01, "Waveform Generation Mode bit 0"),
        ]),
        reg("SFIOR", 0x50, "CPU", "Special Function IO Register", if big {
            vec![b("ADTS", 0xe0, "ADC Auto Trigger Source (0 free running, 1 AC, 2 INT0, 3 TC0 compare, 4 TC0 overflow, 5 TC1 compare B, 6 TC1 overflow, 7 TC1 capture)"),
                 b("ACME", 0x08, "Analog Comparator Multiplexer Enable"), b("PUD", 0x04, "Pull-up Disable"), b("PSR2", 0x02, "Prescaler Reset Timer/Counter2"), b("PSR10", 0x01, "Prescaler Reset Timer/Counter1 and Timer/Counter0")]
        } else {
            vec![b("ACME", 0x08, "Analog Comparator Multiplexer Enable"), b("PUD", 0x04, "Pull-up Disable"), b("PSR2", 0x02, "Prescaler Reset Timer/Counter2"), b("PSR10", 0x01, "Prescaler Reset Timer/Counter1 and Timer/Counter0")]
        }),
        reg("OSCCAL", 0x51, "CPU", "Oscillator Calibration Register", vec![]),
        reg("TCNT0", 0x52, "TC0", "Timer/Counter0", vec![]),
        reg("TCCR0", 0x53, "TC0", "Timer/Counter0 Control Register", if big {
            tccr_single(0, "Clock Select (0 stop, 1 /1, 2 /8, 3 /64, 4 /256, 5 /1024, 6 T0 falling, 7 T0 rising)")
        } else {
            vec![b("CS0", 0x07, "Clock Select (0 stop, 1 /1, 2 /8, 3 /64, 4 /256, 5 /1024, 6 T0 falling, 7 T0 rising)")]
        }),
        reg("MCUCSR", 0x54, "CPU", "MCU Control and Status Register (reset flags)", {
            let mut f = vec![b("WDRF", 0x08, "Watchdog Reset Flag"), b("BORF", 0x04, "Brown-out Reset Flag"), b("EXTRF", 0x02, "External Reset Flag"), b("PORF", 0x01, "Power-on Reset Flag")];
            if big {
                f.splice(0..0, [b("JTD", 0x80, "JTAG Interface Disable"), b("ISC2", 0x40, "Interrupt Sense Control 2 (0 falling, 1 rising)"), b("JTRF", 0x10, "JTAG Reset Flag")]);
            }
            f
        }),
        reg("MCUCR", 0x55, "CPU", "MCU Control Register", {
            let mut f = if v.kind == Kind::M16 {
                vec![b("SM2", 0x80, "Sleep Mode Select bit 2"), b("SE", 0x40, "Sleep Enable"), b("SM1", 0x20, "Sleep Mode Select bit 1"), b("SM0", 0x10, "Sleep Mode Select bit 0")]
            } else {
                vec![b("SE", 0x80, "Sleep Enable"), b("SM", 0x70, "Sleep Mode (000 idle, 001 ADC NR, 010 power-down, 011 power-save, 110 standby, 111 ext. standby)")]
            };
            f.extend([b("ISC1", 0x0c, "Interrupt Sense Control 1 (00 low, 01 any, 10 falling, 11 rising)"), b("ISC0", 0x03, "Interrupt Sense Control 0 (00 low, 01 any, 10 falling, 11 rising)")]);
            f
        }),
        reg("TWCR", 0x56, "TWI", "TWI Control Register", vec![
            b("TWINT", 0x80, "TWI Interrupt Flag"), b("TWEA", 0x40, "TWI Enable Acknowledge"), b("TWSTA", 0x20, "TWI START Condition"), b("TWSTO", 0x10, "TWI STOP Condition"),
            b("TWWC", 0x08, "TWI Write Collision"), b("TWEN", 0x04, "TWI Enable"), b("TWIE", 0x01, "TWI Interrupt Enable"),
        ]),
        reg("SPMCR", 0x57, "CPU", "Store Program Memory Control Register", vec![
            b("SPMIE", 0x80, "SPM Interrupt Enable"), b("RWWSB", 0x40, "Read-While-Write Section Busy"), b("RWWSRE", 0x10, "RWW Section Read Enable"),
            b("BLBSET", 0x08, "Boot Lock Bit Set"), b("PGWRT", 0x04, "Page Write"), b("PGERS", 0x02, "Page Erase"), b("SPMEN", 0x01, "Store Program Memory Enable"),
        ]),
        reg("TIFR", 0x58, "TC0", "Timer/Counter Interrupt Flag Register (all timers)", {
            let mut f = vec![b("OCF2", 0x80, "Timer2 Output Compare Match Flag"), b("TOV2", 0x40, "Timer2 Overflow Flag"), b("ICF1", 0x20, "Timer1 Input Capture Flag"), b("OCF1A", 0x10, "Timer1 Output Compare A Match Flag"),
                             b("OCF1B", 0x08, "Timer1 Output Compare B Match Flag"), b("TOV1", 0x04, "Timer1 Overflow Flag")];
            if big {
                f.push(b("OCF0", 0x02, "Timer0 Output Compare Match Flag"));
            }
            f.push(b("TOV0", 0x01, "Timer0 Overflow Flag"));
            f
        }),
        reg("TIMSK", 0x59, "TC0", "Timer/Counter Interrupt Mask Register (all timers)", {
            let mut f = vec![b("OCIE2", 0x80, "Timer2 Output Compare Match Interrupt Enable"), b("TOIE2", 0x40, "Timer2 Overflow Interrupt Enable"), b("TICIE1", 0x20, "Timer1 Input Capture Interrupt Enable"),
                             b("OCIE1A", 0x10, "Timer1 Output Compare A Match Interrupt Enable"), b("OCIE1B", 0x08, "Timer1 Output Compare B Match Interrupt Enable"), b("TOIE1", 0x04, "Timer1 Overflow Interrupt Enable")];
            if big {
                f.push(b("OCIE0", 0x02, "Timer0 Output Compare Match Interrupt Enable"));
            }
            f.push(b("TOIE0", 0x01, "Timer0 Overflow Interrupt Enable"));
            f
        }),
        reg("GIFR", 0x5a, "EXINT", "General Interrupt Flag Register", {
            let mut f = vec![b("INTF1", 0x80, "External Interrupt Flag 1"), b("INTF0", 0x40, "External Interrupt Flag 0")];
            if big {
                f.push(b("INTF2", 0x20, "External Interrupt Flag 2"));
            }
            f
        }),
        reg("GICR", 0x5b, "EXINT", "General Interrupt Control Register", {
            let mut f = vec![b("INT1", 0x80, "External Interrupt Request 1 Enable"), b("INT0", 0x40, "External Interrupt Request 0 Enable")];
            if big {
                f.push(b("INT2", 0x20, "External Interrupt Request 2 Enable"));
            }
            f.extend([b("IVSEL", 0x02, "Interrupt Vector Select (boot section)"), b("IVCE", 0x01, "Interrupt Vector Change Enable")]);
            f
        }),
        reset(reg("SPL", 0x5d, "CPU", "Stack Pointer Low Byte", vec![]), (ramend & 0xff) as u8),
        reset(reg("SPH", 0x5e, "CPU", "Stack Pointer High Byte", vec![]), (ramend >> 8) as u8),
        reg("SREG", 0x5f, "CPU", "Status Register", bits_msb_first(
            &[Some("I"), Some("T"), Some("H"), Some("S"), Some("V"), Some("N"), Some("Z"), Some("C")],
            &[("I", "Global Interrupt Enable"), ("T", "Bit Copy Storage"), ("H", "Half Carry Flag"), ("S", "Sign Bit (N xor V)"),
              ("V", "Two's Complement Overflow Flag"), ("N", "Negative Flag"), ("Z", "Zero Flag"), ("C", "Carry Flag")],
        )),
    ];
    if big {
        r.extend([
            reg("PINA", 0x39, "PORTA", "Port A Input Pins (write 1 toggles PORTA bit)", nbits("PINA", 0xff)),
            reg("DDRA", 0x3a, "PORTA", "Port A Data Direction Register", nbits("DDA", 0xff)),
            reg("PORTA", 0x3b, "PORTA", "Port A Data Register", nbits("PORTA", 0xff)),
            reg("OCR0", 0x5c, "TC0", "Timer/Counter0 Output Compare Register", vec![]),
        ]);
    }
    r.sort_by_key(|x| x.addr);
    r
}

fn vector_names(kind: Kind) -> Vec<(&'static str, &'static str)> {
    let common_head = [("RESET", "External Pin, Power-on Reset, Brown-out Reset and Watchdog Reset"), ("INT0", "External Interrupt Request 0"), ("INT1", "External Interrupt Request 1")];
    let int2 = ("INT2", "External Interrupt Request 2");
    let t0c = ("TIMER0_COMP", "Timer/Counter0 Compare Match");
    let mut v = common_head.to_vec();
    if kind == Kind::M32 {
        v.push(int2);
    }
    v.extend([
        ("TIMER2_COMP", "Timer/Counter2 Compare Match"),
        ("TIMER2_OVF", "Timer/Counter2 Overflow"),
        ("TIMER1_CAPT", "Timer/Counter1 Capture Event"),
        ("TIMER1_COMPA", "Timer/Counter1 Compare Match A"),
        ("TIMER1_COMPB", "Timer/Counter1 Compare Match B"),
        ("TIMER1_OVF", "Timer/Counter1 Overflow"),
    ]);
    if kind == Kind::M32 {
        v.push(t0c);
    }
    v.extend([
        ("TIMER0_OVF", "Timer/Counter0 Overflow"),
        ("SPI_STC", "SPI Serial Transfer Complete"),
        ("USART_RXC", "USART Rx Complete"),
        ("USART_UDRE", "USART Data Register Empty"),
        ("USART_TXC", "USART Tx Complete"),
        ("ADC", "ADC Conversion Complete"),
        ("EE_RDY", "EEPROM Ready"),
        ("ANA_COMP", "Analog Comparator"),
        ("TWI", "2-wire Serial Interface"),
    ]);
    if kind == Kind::M16 {
        v.extend([int2, t0c]);
    }
    v.push(("SPM_RDY", "Store Program Memory Ready"));
    v
}

fn fuses(v: &Variant) -> Vec<FuseByteSpec> {
    let f = |n: &str, m: u8, d: &str| FuseBitSpec { name: n.into(), mask: m, desc: d.into() };
    let low = FuseByteSpec {
        name: "Low".into(),
        default: 0xe1,
        bits: vec![
            f("BODLEVEL", 0x80, "Brown-out detector level (1 = 2.7 V, 0 = 4.0 V)"),
            f("BODEN", 0x40, "Brown-out detector enabled when programmed (0)"),
            f("SUT", 0x30, "Start-up time select"),
            f("CKSEL", 0x0f, "Clock source (0000 external clock, 0001-0100 internal RC 1/2/4/8 MHz, 0101-1000 external RC, 1001 32 kHz crystal, 1010-1111 crystal / resonator)"),
        ],
    };
    let tail = || vec![
        f("SPIEN", 0x20, "Serial programming enabled when programmed (0)"),
        f("CKOPT", 0x10, "Oscillator options (full-swing crystal amplifier) when programmed (0)"),
        f("EESAVE", 0x08, "EEPROM preserved through chip erase when programmed (0)"),
        f("BOOTSZ", 0x06, "Boot section size (see boot loader table)"),
        f("BOOTRST", 0x01, "Reset to the boot loader section when programmed (0)"),
    ];
    let mut hi = if v.kind == Kind::M8 {
        vec![f("RSTDISBL", 0x80, "External reset disabled (PC6 becomes I/O) when programmed (0)"), f("WDTON", 0x40, "Watchdog Timer always on when programmed (0)")]
    } else {
        vec![f("OCDEN", 0x80, "On-chip debug enabled when programmed (0)"), f("JTAGEN", 0x40, "JTAG interface enabled when programmed (0)")]
    };
    hi.extend(tail());
    // Datasheet factory values: ATmega8 high 0xD9 (SPIEN programmed), ATmega16/32 0x99 (SPIEN and
    // JTAGEN programmed).
    let default = if v.kind == Kind::M8 { 0xd9 } else { 0x99 };
    vec![low, FuseByteSpec { name: "High".into(), default, bits: hi }]
}

fn spec(v: &Variant) -> AvrDeviceSpec {
    let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let io = |number: u8, name: &str, gpio: u8, functions: Vec<String>| PinSpec { number, name: name.into(), kind: PinKind::Io, gpio: Some(gpio), functions };
    let power = |number: u8, name: &str, kind: PinKind| PinSpec { number, name: name.into(), kind, gpio: None, functions: vec![] };
    let big = v.big();
    let (pins, package, gpio_count) = if !big {
        // GPIO numbering: PB0-7 = 0-7, PC0-6 = 8-14, PD0-7 = 15-22.
        let pins = vec![
            io(1, "PC6", 14, s(&["RESET"])),
            io(2, "PD0", 15, s(&["RXD"])),
            io(3, "PD1", 16, s(&["TXD"])),
            io(4, "PD2", 17, s(&["INT0"])),
            io(5, "PD3", 18, s(&["INT1"])),
            io(6, "PD4", 19, s(&["XCK", "T0"])),
            power(7, "VCC", PinKind::Vcc),
            power(8, "GND", PinKind::Gnd),
            io(9, "PB6", 6, s(&["XTAL1", "TOSC1"])),
            io(10, "PB7", 7, s(&["XTAL2", "TOSC2"])),
            io(11, "PD5", 20, s(&["T1"])),
            io(12, "PD6", 21, s(&["AIN0"])),
            io(13, "PD7", 22, s(&["AIN1"])),
            io(14, "PB0", 0, s(&["ICP1"])),
            io(15, "PB1", 1, s(&["OC1A"])),
            io(16, "PB2", 2, s(&["SS", "OC1B"])),
            io(17, "PB3", 3, s(&["MOSI", "OC2"])),
            io(18, "PB4", 4, s(&["MISO"])),
            io(19, "PB5", 5, s(&["SCK"])),
            power(20, "AVCC", PinKind::Ref),
            power(21, "AREF", PinKind::Ref),
            power(22, "GND", PinKind::Gnd),
            io(23, "PC0", 8, s(&["ADC0"])),
            io(24, "PC1", 9, s(&["ADC1"])),
            io(25, "PC2", 10, s(&["ADC2"])),
            io(26, "PC3", 11, s(&["ADC3"])),
            io(27, "PC4", 12, s(&["ADC4", "SDA"])),
            io(28, "PC5", 13, s(&["ADC5", "SCL"])),
        ];
        (pins, "PDIP-28", 23)
    } else {
        // GPIO numbering: PA0-7 = 0-7, PB0-7 = 8-15, PC0-7 = 16-23, PD0-7 = 24-31.
        let pins = vec![
            io(1, "PB0", 8, s(&["XCK", "T0"])),
            io(2, "PB1", 9, s(&["T1"])),
            io(3, "PB2", 10, s(&["INT2", "AIN0"])),
            io(4, "PB3", 11, s(&["OC0", "AIN1"])),
            io(5, "PB4", 12, s(&["SS"])),
            io(6, "PB5", 13, s(&["MOSI"])),
            io(7, "PB6", 14, s(&["MISO"])),
            io(8, "PB7", 15, s(&["SCK"])),
            PinSpec { number: 9, name: "RESET".into(), kind: PinKind::Io, gpio: None, functions: s(&["RESET"]) },
            power(10, "VCC", PinKind::Vcc),
            power(11, "GND", PinKind::Gnd),
            PinSpec { number: 12, name: "XTAL2".into(), kind: PinKind::Io, gpio: None, functions: s(&["XTAL2"]) },
            PinSpec { number: 13, name: "XTAL1".into(), kind: PinKind::Io, gpio: None, functions: s(&["XTAL1"]) },
            io(14, "PD0", 24, s(&["RXD"])),
            io(15, "PD1", 25, s(&["TXD"])),
            io(16, "PD2", 26, s(&["INT0"])),
            io(17, "PD3", 27, s(&["INT1"])),
            io(18, "PD4", 28, s(&["OC1B"])),
            io(19, "PD5", 29, s(&["OC1A"])),
            io(20, "PD6", 30, s(&["ICP1"])),
            io(21, "PD7", 31, s(&["OC2"])),
            io(22, "PC0", 16, s(&["SCL"])),
            io(23, "PC1", 17, s(&["SDA"])),
            io(24, "PC2", 18, s(&["TCK"])),
            io(25, "PC3", 19, s(&["TMS"])),
            io(26, "PC4", 20, s(&["TDO"])),
            io(27, "PC5", 21, s(&["TDI"])),
            io(28, "PC6", 22, s(&["TOSC1"])),
            io(29, "PC7", 23, s(&["TOSC2"])),
            power(30, "AVCC", PinKind::Ref),
            power(31, "GND", PinKind::Gnd),
            power(32, "AREF", PinKind::Ref),
            io(33, "PA7", 7, s(&["ADC7"])),
            io(34, "PA6", 6, s(&["ADC6"])),
            io(35, "PA5", 5, s(&["ADC5"])),
            io(36, "PA4", 4, s(&["ADC4"])),
            io(37, "PA3", 3, s(&["ADC3"])),
            io(38, "PA2", 2, s(&["ADC2"])),
            io(39, "PA1", 1, s(&["ADC1"])),
            io(40, "PA0", 0, s(&["ADC0"])),
        ];
        (pins, "PDIP-40", 32)
    };
    let mut groups = vec![
        ("CPU", "CPU, Clock, Sleep, Reset & Power"),
        ("PORTA", "I/O Port A"),
        ("PORTB", "I/O Port B"),
        ("PORTC", "I/O Port C"),
        ("PORTD", "I/O Port D"),
        ("EXINT", "External Interrupts"),
        ("TC0", if big { "8-bit Timer/Counter0 with PWM" } else { "8-bit Timer/Counter0" }),
        ("TC1", "16-bit Timer/Counter1 with PWM"),
        ("TC2", "8-bit Timer/Counter2 with PWM (asynchronous mode not simulated)"),
        ("USART", "USART (serial port)"),
        ("SPI", "Serial Peripheral Interface"),
        ("TWI", "2-wire Serial Interface (I2C)"),
        ("AC", "Analog Comparator"),
        ("ADC", "10-bit Analog to Digital Converter"),
        ("EEPROM", "EEPROM"),
        ("WDT", "Watchdog Timer"),
    ];
    if !big {
        groups.retain(|g| g.0 != "PORTA");
    }
    let mut features = feature::MOVW | feature::MUL | feature::LPMX | feature::SPM;
    if big {
        features |= feature::JMP | feature::BREAK;
    }
    // Sleep: ATmega8 MCUCR SE bit 7, SM2:0 bits 6:4. ATmega16 SM2 = bit 7, SE = bit 6, SM1:0 = bits
    // 5:4 (non-contiguous: raw value = (reg & 0xb0) >> 4). ATmega32 like the ATmega8 plus
    // extended standby (Atmel-2486AA / 2466T / 2503Q "MCUCR").
    let sleep = match v.kind {
        Kind::M8 => SleepControl {
            register: "MCUCR".into(), se_mask: 0x80, sm_mask: 0x70,
            modes: vec![(0, SleepKind::Idle), (1, SleepKind::AdcNoiseReduction), (2, SleepKind::PowerDown), (3, SleepKind::PowerSave), (6, SleepKind::Standby)],
        },
        Kind::M16 => SleepControl {
            register: "MCUCR".into(), se_mask: 0x40, sm_mask: 0xb0,
            modes: vec![(0, SleepKind::Idle), (1, SleepKind::AdcNoiseReduction), (2, SleepKind::PowerDown), (3, SleepKind::PowerSave), (10, SleepKind::Standby), (11, SleepKind::ExtendedStandby)],
        },
        Kind::M32 => SleepControl {
            register: "MCUCR".into(), se_mask: 0x80, sm_mask: 0x70,
            modes: vec![(0, SleepKind::Idle), (1, SleepKind::AdcNoiseReduction), (2, SleepKind::PowerDown), (3, SleepKind::PowerSave), (6, SleepKind::Standby), (7, SleepKind::ExtendedStandby)],
        },
    };
    AvrDeviceSpec {
        id: v.id.into(),
        name: v.name.into(),
        family: "megaAVR (ATmega8/16/32)".into(),
        core_name: if big { "AVRe+ (AVR5)".into() } else { "AVRe+ (AVR4)".into() },
        features,
        flash_size: v.flash,
        sram_start: 0x60,
        sram_size: v.sram,
        eeprom_size: v.eeprom,
        io_base: 0x20,
        io_size: 64,
        regs_in_data_space: true,
        flash_map_base: None,
        nvm_map: None,
        signature: v.signature,
        calibration: 0x80,
        fuses: fuses(v),
        sleep,
        // BOOTSZ1:0 = 00..11: 1024/512/256/128 words (8K, 16K flash), 2048/1024/512/256 (32K).
        boot: Some(BootSpec { sizes_words: if v.kind == Kind::M32 { [2048, 1024, 512, 256] } else { [1024, 512, 256, 128] } }),
        vectors: vector_names(v.kind).iter().enumerate().map(|(i, (n, d))| VectorSpec { index: i as u8, name: (*n).into(), desc: (*d).into() }).collect(),
        registers: registers(v),
        groups: groups.iter().map(|(n, d)| PeripheralGroupSpec { name: (*n).into(), desc: (*d).into() }).collect(),
        package: package.into(),
        pins,
        gpio_count,
        has_adc: true,
        // Calibrated internal RC: 1 MHz at 5 V / 25 C by default (CKSEL = 0001); 32.768 kHz is the
        // Timer2 watch crystal.
        clock: ClockSpec { internal_hz: 1_000_000.0, slow_hz: 32_768.0, default_prescale_log2: 0 },
        vcc: 5.0,
        // "Speed grades": ATmega8L/16L/32L 0-8 MHz at 2.7-5.5 V, ATmega8/16/32 0-16 MHz at 4.5-5.5 V.
        vcc_range: (2.7, 5.5),
        speed_grades: vec![(8e6, 2.7), (16e6, 4.5)],
        datasheet: v.datasheet.into(),
        die: None,
        peripheral_set: PeripheralSet::MegaLegacy,
    }
}

pub fn devices() -> Vec<AvrDeviceSpec> {
    VARIANTS.iter().map(spec).collect()
}
