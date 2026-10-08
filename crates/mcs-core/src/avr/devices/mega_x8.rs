//! ATmega48PA / ATmega88PA / ATmega168PA / ATmega328P (classic AVR core, PDIP-28).
//! Source: Atmel/Microchip "ATmega48A/PA/88A/PA/168A/PA/328/P" datasheet DS40002061B, the
//! register summary (section 30), fuse tables (section 28.2) and avr-libc iom328p.h / iomx8.h.

use crate::avr::device::*;
use crate::avr::isa::feature;

struct Variant {
    id: &'static str,
    name: &'static str,
    flash: u32,
    sram: u16,
    eeprom: u16,
    signature: [u8; 3],
    /// AVR5 (JMP/CALL, 2-word vectors) vs AVR4.
    jmp: bool,
    /// Boot section sizes in words per BOOTSZ value (None: no boot loader support).
    boot: Option<[u32; 4]>,
    /// Where BOOTSZ/BOOTRST and BODLEVEL live (differs between the sizes).
    layout: FuseLayout,
}

#[derive(Clone, Copy)]
enum FuseLayout {
    /// ATmega48: BODLEVEL in high fuse, SELFPRGEN in extended.
    M48,
    /// ATmega88/168: BODLEVEL in high fuse, BOOTSZ/BOOTRST in extended.
    M88,
    /// ATmega328: BOOTSZ/BOOTRST in high fuse, BODLEVEL in extended.
    M328,
}

const VARIANTS: &[Variant] = &[
    Variant { id: "atmega48pa", name: "ATmega48PA", flash: 4096, sram: 512, eeprom: 256, signature: [0x1e, 0x92, 0x0a], jmp: false, boot: None, layout: FuseLayout::M48 },
    Variant { id: "atmega88pa", name: "ATmega88PA", flash: 8192, sram: 1024, eeprom: 512, signature: [0x1e, 0x93, 0x0f], jmp: false, boot: Some([1024, 512, 256, 128]), layout: FuseLayout::M88 },
    Variant { id: "atmega168pa", name: "ATmega168PA", flash: 16384, sram: 1024, eeprom: 512, signature: [0x1e, 0x94, 0x0b], jmp: true, boot: Some([1024, 512, 256, 128]), layout: FuseLayout::M88 },
    Variant { id: "atmega328p", name: "ATmega328P", flash: 32768, sram: 2048, eeprom: 1024, signature: [0x1e, 0x95, 0x0f], jmp: true, boot: Some([2048, 1024, 512, 256]), layout: FuseLayout::M328 },
];

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

fn registers(v: &Variant) -> Vec<IoRegisterSpec> {
    let mut r = vec![
        reg("PINB", 0x23, "PORTB", "Port B Input Pins (write 1 toggles PORTB bit)", nbits("PINB", 0xff)),
        reg("DDRB", 0x24, "PORTB", "Port B Data Direction Register", nbits("DDB", 0xff)),
        reg("PORTB", 0x25, "PORTB", "Port B Data Register", nbits("PORTB", 0xff)),
        reg("PINC", 0x26, "PORTC", "Port C Input Pins (write 1 toggles PORTC bit)", nbits("PINC", 0x7f)),
        reg("DDRC", 0x27, "PORTC", "Port C Data Direction Register", nbits("DDC", 0x7f)),
        reg("PORTC", 0x28, "PORTC", "Port C Data Register", nbits("PORTC", 0x7f)),
        reg("PIND", 0x29, "PORTD", "Port D Input Pins (write 1 toggles PORTD bit)", nbits("PIND", 0xff)),
        reg("DDRD", 0x2a, "PORTD", "Port D Data Direction Register", nbits("DDD", 0xff)),
        reg("PORTD", 0x2b, "PORTD", "Port D Data Register", nbits("PORTD", 0xff)),
        reg("TIFR0", 0x35, "TC0", "Timer/Counter0 Interrupt Flag Register", vec![b("OCF0B", 0x04, "Output Compare B Match Flag"), b("OCF0A", 0x02, "Output Compare A Match Flag"), b("TOV0", 0x01, "Overflow Flag")]),
        reg("TIFR1", 0x36, "TC1", "Timer/Counter1 Interrupt Flag Register", vec![b("ICF1", 0x20, "Input Capture Flag"), b("OCF1B", 0x04, "Output Compare B Match Flag"), b("OCF1A", 0x02, "Output Compare A Match Flag"), b("TOV1", 0x01, "Overflow Flag")]),
        reg("TIFR2", 0x37, "TC2", "Timer/Counter2 Interrupt Flag Register", vec![b("OCF2B", 0x04, "Output Compare B Match Flag"), b("OCF2A", 0x02, "Output Compare A Match Flag"), b("TOV2", 0x01, "Overflow Flag")]),
        reg("PCIFR", 0x3b, "EXINT", "Pin Change Interrupt Flag Register", vec![b("PCIF2", 0x04, "Pin Change Interrupt Flag 2 (PCINT23..16)"), b("PCIF1", 0x02, "Pin Change Interrupt Flag 1 (PCINT14..8)"), b("PCIF0", 0x01, "Pin Change Interrupt Flag 0 (PCINT7..0)")]),
        reg("EIFR", 0x3c, "EXINT", "External Interrupt Flag Register", vec![b("INTF1", 0x02, "External Interrupt Flag 1"), b("INTF0", 0x01, "External Interrupt Flag 0")]),
        reg("EIMSK", 0x3d, "EXINT", "External Interrupt Mask Register", vec![b("INT1", 0x02, "External Interrupt Request 1 Enable"), b("INT0", 0x01, "External Interrupt Request 0 Enable")]),
        reg("GPIOR0", 0x3e, "CPU", "General Purpose I/O Register 0", vec![]),
        reg("EECR", 0x3f, "EEPROM", "EEPROM Control Register", vec![
            b("EEPM", 0x30, "EEPROM Programming Mode (00 erase+write, 01 erase, 10 write)"), b("EERIE", 0x08, "EEPROM Ready Interrupt Enable"),
            b("EEMPE", 0x04, "EEPROM Master Write Enable"), b("EEPE", 0x02, "EEPROM Write Enable"), b("EERE", 0x01, "EEPROM Read Enable"),
        ]),
        reg("EEDR", 0x40, "EEPROM", "EEPROM Data Register", vec![]),
        reg("EEARL", 0x41, "EEPROM", "EEPROM Address Register Low Byte", vec![]),
        reg("EEARH", 0x42, "EEPROM", "EEPROM Address Register High Byte", vec![]),
        reg("GTCCR", 0x43, "TC0", "General Timer/Counter Control Register", vec![b("TSM", 0x80, "Timer/Counter Synchronization Mode"), b("PSRASY", 0x02, "Prescaler Reset Timer/Counter2"), b("PSRSYNC", 0x01, "Prescaler Reset Timer/Counter1 and Timer/Counter0")]),
        reg("TCCR0A", 0x44, "TC0", "Timer/Counter0 Control Register A", vec![b("COM0A", 0xc0, "Compare Output Mode A"), b("COM0B", 0x30, "Compare Output Mode B"), b("WGM01", 0x02, "Waveform Generation Mode bit 1"), b("WGM00", 0x01, "Waveform Generation Mode bit 0")]),
        reg("TCCR0B", 0x45, "TC0", "Timer/Counter0 Control Register B", vec![b("FOC0A", 0x80, "Force Output Compare A"), b("FOC0B", 0x40, "Force Output Compare B"), b("WGM02", 0x08, "Waveform Generation Mode bit 2"), b("CS0", 0x07, "Clock Select (0 stop, 1 /1, 2 /8, 3 /64, 4 /256, 5 /1024, 6 T0 falling, 7 T0 rising)")]),
        reg("TCNT0", 0x46, "TC0", "Timer/Counter0", vec![]),
        reg("OCR0A", 0x47, "TC0", "Output Compare Register 0 A", vec![]),
        reg("OCR0B", 0x48, "TC0", "Output Compare Register 0 B", vec![]),
        reg("GPIOR1", 0x4a, "CPU", "General Purpose I/O Register 1", vec![]),
        reg("GPIOR2", 0x4b, "CPU", "General Purpose I/O Register 2", vec![]),
        reg("SPCR", 0x4c, "SPI", "SPI Control Register", vec![
            b("SPIE", 0x80, "SPI Interrupt Enable"), b("SPE", 0x40, "SPI Enable"), b("DORD", 0x20, "Data Order (1 = LSB first)"), b("MSTR", 0x10, "Master/Slave Select"),
            b("CPOL", 0x08, "Clock Polarity"), b("CPHA", 0x04, "Clock Phase"), b("SPR", 0x03, "SPI Clock Rate Select (fosc/4, /16, /64, /128)"),
        ]),
        reg("SPSR", 0x4d, "SPI", "SPI Status Register", vec![b("SPIF", 0x80, "SPI Interrupt Flag"), b("WCOL", 0x40, "Write Collision Flag"), b("SPI2X", 0x01, "Double SPI Speed")]),
        reg("SPDR", 0x4e, "SPI", "SPI Data Register", vec![]),
        reg("ACSR", 0x50, "AC", "Analog Comparator Control and Status Register", vec![
            b("ACD", 0x80, "Analog Comparator Disable"), b("ACBG", 0x40, "Bandgap Select (1.1 V on the positive input)"), b("ACO", 0x20, "Analog Comparator Output"),
            b("ACI", 0x10, "Analog Comparator Interrupt Flag"), b("ACIE", 0x08, "Analog Comparator Interrupt Enable"), b("ACIC", 0x04, "Input Capture Enable (Timer1)"),
            b("ACIS", 0x03, "Interrupt Mode Select (00 toggle, 10 falling, 11 rising)"),
        ]),
        reg("SMCR", 0x53, "CPU", "Sleep Mode Control Register", vec![b("SM", 0x0e, "Sleep Mode (000 idle, 001 ADC NR, 010 power-down, 011 power-save, 110 standby, 111 ext. standby)"), b("SE", 0x01, "Sleep Enable")]),
        reg("MCUSR", 0x54, "CPU", "MCU Status Register (reset flags)", vec![b("WDRF", 0x08, "Watchdog Reset Flag"), b("BORF", 0x04, "Brown-out Reset Flag"), b("EXTRF", 0x02, "External Reset Flag"), b("PORF", 0x01, "Power-on Reset Flag")]),
        reg("MCUCR", 0x55, "CPU", "MCU Control Register", vec![
            b("BODS", 0x40, "BOD Sleep"), b("BODSE", 0x20, "BOD Sleep Enable"), b("PUD", 0x10, "Pull-up Disable"), b("IVSEL", 0x02, "Interrupt Vector Select (boot section)"), b("IVCE", 0x01, "Interrupt Vector Change Enable"),
        ]),
        reg("SPMCSR", 0x57, "CPU", "Store Program Memory Control and Status Register", vec![
            b("SPMIE", 0x80, "SPM Interrupt Enable"), b("RWWSB", 0x40, "Read-While-Write Section Busy"), b("SIGRD", 0x20, "Signature Row Read"), b("RWWSRE", 0x10, "RWW Section Read Enable"),
            b("BLBSET", 0x08, "Boot Lock Bit Set"), b("PGWRT", 0x04, "Page Write"), b("PGERS", 0x02, "Page Erase"), b("SPMEN", 0x01, "Store Program Memory Enable"),
        ]),
        reset(reg("SPL", 0x5d, "CPU", "Stack Pointer Low Byte", vec![]), ((v.sram + 0x100 - 1) & 0xff) as u8),
        reset(reg("SPH", 0x5e, "CPU", "Stack Pointer High Byte", vec![]), ((v.sram + 0x100 - 1) >> 8) as u8),
        reg("SREG", 0x5f, "CPU", "Status Register", bits_msb_first(
            &[Some("I"), Some("T"), Some("H"), Some("S"), Some("V"), Some("N"), Some("Z"), Some("C")],
            &[("I", "Global Interrupt Enable"), ("T", "Bit Copy Storage"), ("H", "Half Carry Flag"), ("S", "Sign Bit (N xor V)"),
              ("V", "Two's Complement Overflow Flag"), ("N", "Negative Flag"), ("Z", "Zero Flag"), ("C", "Carry Flag")],
        )),
        reg("WDTCSR", 0x60, "WDT", "Watchdog Timer Control Register", vec![
            b("WDIF", 0x80, "Watchdog Interrupt Flag"), b("WDIE", 0x40, "Watchdog Interrupt Enable"), b("WDP3", 0x20, "Watchdog Prescaler bit 3"),
            b("WDCE", 0x10, "Watchdog Change Enable"), b("WDE", 0x08, "Watchdog System Reset Enable"), b("WDP", 0x07, "Watchdog Prescaler bits 2:0"),
        ]),
        reg("CLKPR", 0x61, "CPU", "Clock Prescale Register", vec![b("CLKPCE", 0x80, "Clock Prescaler Change Enable"), b("CLKPS", 0x0f, "Clock Prescaler Select (division = 2^CLKPS)")]),
        reg("PRR", 0x64, "CPU", "Power Reduction Register", vec![
            b("PRTWI", 0x80, "Power Reduction TWI"), b("PRTIM2", 0x40, "Power Reduction Timer/Counter2"), b("PRTIM0", 0x20, "Power Reduction Timer/Counter0"),
            b("PRTIM1", 0x08, "Power Reduction Timer/Counter1"), b("PRSPI", 0x04, "Power Reduction SPI"), b("PRUSART0", 0x02, "Power Reduction USART0"), b("PRADC", 0x01, "Power Reduction ADC"),
        ]),
        reg("OSCCAL", 0x66, "CPU", "Oscillator Calibration Register", vec![]),
        reg("PCICR", 0x68, "EXINT", "Pin Change Interrupt Control Register", vec![b("PCIE2", 0x04, "Pin Change Interrupt Enable 2"), b("PCIE1", 0x02, "Pin Change Interrupt Enable 1"), b("PCIE0", 0x01, "Pin Change Interrupt Enable 0")]),
        reg("EICRA", 0x69, "EXINT", "External Interrupt Control Register A", vec![b("ISC1", 0x0c, "Interrupt Sense Control 1 (00 low, 01 any, 10 falling, 11 rising)"), b("ISC0", 0x03, "Interrupt Sense Control 0 (00 low, 01 any, 10 falling, 11 rising)")]),
        reg("PCMSK0", 0x6b, "EXINT", "Pin Change Mask Register 0 (PB7..PB0)", nbits("PCINT", 0xff)),
        reg("PCMSK1", 0x6c, "EXINT", "Pin Change Mask Register 1 (PC6..PC0)", (8..15).rev().map(|i| field(&format!("PCINT{i}"), 1 << (i - 8), "")).collect()),
        reg("PCMSK2", 0x6d, "EXINT", "Pin Change Mask Register 2 (PD7..PD0)", (16..24).rev().map(|i| field(&format!("PCINT{i}"), 1 << (i - 16), "")).collect()),
        reg("TIMSK0", 0x6e, "TC0", "Timer/Counter0 Interrupt Mask Register", vec![b("OCIE0B", 0x04, "Output Compare B Match Interrupt Enable"), b("OCIE0A", 0x02, "Output Compare A Match Interrupt Enable"), b("TOIE0", 0x01, "Overflow Interrupt Enable")]),
        reg("TIMSK1", 0x6f, "TC1", "Timer/Counter1 Interrupt Mask Register", vec![b("ICIE1", 0x20, "Input Capture Interrupt Enable"), b("OCIE1B", 0x04, "Output Compare B Match Interrupt Enable"), b("OCIE1A", 0x02, "Output Compare A Match Interrupt Enable"), b("TOIE1", 0x01, "Overflow Interrupt Enable")]),
        reg("TIMSK2", 0x70, "TC2", "Timer/Counter2 Interrupt Mask Register", vec![b("OCIE2B", 0x04, "Output Compare B Match Interrupt Enable"), b("OCIE2A", 0x02, "Output Compare A Match Interrupt Enable"), b("TOIE2", 0x01, "Overflow Interrupt Enable")]),
        ro(reg("ADCL", 0x78, "ADC", "ADC Data Register Low Byte (read first)", vec![])),
        ro(reg("ADCH", 0x79, "ADC", "ADC Data Register High Byte", vec![])),
        reg("ADCSRA", 0x7a, "ADC", "ADC Control and Status Register A", vec![
            b("ADEN", 0x80, "ADC Enable"), b("ADSC", 0x40, "ADC Start Conversion"), b("ADATE", 0x20, "ADC Auto Trigger Enable"),
            b("ADIF", 0x10, "ADC Interrupt Flag"), b("ADIE", 0x08, "ADC Interrupt Enable"), b("ADPS", 0x07, "ADC Prescaler Select"),
        ]),
        reg("ADCSRB", 0x7b, "ADC", "ADC Control and Status Register B", vec![b("ACME", 0x40, "Analog Comparator Multiplexer Enable"), b("ADTS", 0x07, "ADC Auto Trigger Source")]),
        reg("ADMUX", 0x7c, "ADC", "ADC Multiplexer Selection Register", vec![
            b("REFS", 0xc0, "Reference Selection (00 AREF, 01 AVCC, 11 internal 1.1 V)"), b("ADLAR", 0x20, "ADC Left Adjust Result"),
            b("MUX", 0x0f, "Analog Channel Selection (0-7 ADCn, 8 temperature, 14 1.1 V, 15 GND)"),
        ]),
        reg("DIDR0", 0x7e, "ADC", "Digital Input Disable Register 0", (0..6).rev().map(|i| field(&format!("ADC{i}D"), 1 << i, "")).collect()),
        reg("DIDR1", 0x7f, "AC", "Digital Input Disable Register 1", vec![b("AIN1D", 0x02, "AIN1 Digital Input Disable"), b("AIN0D", 0x01, "AIN0 Digital Input Disable")]),
        reg("TCCR1A", 0x80, "TC1", "Timer/Counter1 Control Register A", vec![b("COM1A", 0xc0, "Compare Output Mode A"), b("COM1B", 0x30, "Compare Output Mode B"), b("WGM11", 0x02, "Waveform Generation Mode bit 1"), b("WGM10", 0x01, "Waveform Generation Mode bit 0")]),
        reg("TCCR1B", 0x81, "TC1", "Timer/Counter1 Control Register B", vec![
            b("ICNC1", 0x80, "Input Capture Noise Canceler"), b("ICES1", 0x40, "Input Capture Edge Select (1 = rising)"), b("WGM13", 0x10, "Waveform Generation Mode bit 3"),
            b("WGM12", 0x08, "Waveform Generation Mode bit 2"), b("CS1", 0x07, "Clock Select (0 stop, 1 /1, 2 /8, 3 /64, 4 /256, 5 /1024, 6 T1 falling, 7 T1 rising)"),
        ]),
        reg("TCCR1C", 0x82, "TC1", "Timer/Counter1 Control Register C", vec![b("FOC1A", 0x80, "Force Output Compare A"), b("FOC1B", 0x40, "Force Output Compare B")]),
        reg("TCNT1L", 0x84, "TC1", "Timer/Counter1 Low Byte", vec![]),
        reg("TCNT1H", 0x85, "TC1", "Timer/Counter1 High Byte", vec![]),
        reg("ICR1L", 0x86, "TC1", "Input Capture Register 1 Low Byte", vec![]),
        reg("ICR1H", 0x87, "TC1", "Input Capture Register 1 High Byte", vec![]),
        reg("OCR1AL", 0x88, "TC1", "Output Compare Register 1 A Low Byte", vec![]),
        reg("OCR1AH", 0x89, "TC1", "Output Compare Register 1 A High Byte", vec![]),
        reg("OCR1BL", 0x8a, "TC1", "Output Compare Register 1 B Low Byte", vec![]),
        reg("OCR1BH", 0x8b, "TC1", "Output Compare Register 1 B High Byte", vec![]),
        reg("TCCR2A", 0xb0, "TC2", "Timer/Counter2 Control Register A", vec![b("COM2A", 0xc0, "Compare Output Mode A"), b("COM2B", 0x30, "Compare Output Mode B"), b("WGM21", 0x02, "Waveform Generation Mode bit 1"), b("WGM20", 0x01, "Waveform Generation Mode bit 0")]),
        reg("TCCR2B", 0xb1, "TC2", "Timer/Counter2 Control Register B", vec![b("FOC2A", 0x80, "Force Output Compare A"), b("FOC2B", 0x40, "Force Output Compare B"), b("WGM22", 0x08, "Waveform Generation Mode bit 2"), b("CS2", 0x07, "Clock Select (0 stop, 1 /1, 2 /8, 3 /32, 4 /64, 5 /128, 6 /256, 7 /1024)")]),
        reg("TCNT2", 0xb2, "TC2", "Timer/Counter2", vec![]),
        reg("OCR2A", 0xb3, "TC2", "Output Compare Register 2 A", vec![]),
        reg("OCR2B", 0xb4, "TC2", "Output Compare Register 2 B", vec![]),
        reg("ASSR", 0xb6, "TC2", "Asynchronous Status Register", vec![
            b("EXCLK", 0x40, "Enable External Clock Input"), b("AS2", 0x20, "Asynchronous Timer/Counter2"), b("TCN2UB", 0x10, "TCNT2 Update Busy"),
            b("OCR2AUB", 0x08, "OCR2A Update Busy"), b("OCR2BUB", 0x04, "OCR2B Update Busy"), b("TCR2AUB", 0x02, "TCCR2A Update Busy"), b("TCR2BUB", 0x01, "TCCR2B Update Busy"),
        ]),
        reg("TWBR", 0xb8, "TWI", "TWI Bit Rate Register", vec![]),
        reset(reg("TWSR", 0xb9, "TWI", "TWI Status Register", vec![b("TWS", 0xf8, "TWI Status"), b("TWPS", 0x03, "TWI Prescaler (1, 4, 16, 64)")]), 0xf8),
        reset(reg("TWAR", 0xba, "TWI", "TWI (Slave) Address Register", vec![b("TWA", 0xfe, "TWI Slave Address"), b("TWGCE", 0x01, "General Call Recognition Enable")]), 0xfe),
        reset(reg("TWDR", 0xbb, "TWI", "TWI Data Register", vec![]), 0xff),
        reg("TWCR", 0xbc, "TWI", "TWI Control Register", vec![
            b("TWINT", 0x80, "TWI Interrupt Flag"), b("TWEA", 0x40, "TWI Enable Acknowledge"), b("TWSTA", 0x20, "TWI START Condition"), b("TWSTO", 0x10, "TWI STOP Condition"),
            b("TWWC", 0x08, "TWI Write Collision"), b("TWEN", 0x04, "TWI Enable"), b("TWIE", 0x01, "TWI Interrupt Enable"),
        ]),
        reg("TWAMR", 0xbd, "TWI", "TWI (Slave) Address Mask Register", vec![]),
        reset(reg("UCSR0A", 0xc0, "USART0", "USART Control and Status Register A", vec![
            b("RXC0", 0x80, "Receive Complete"), b("TXC0", 0x40, "Transmit Complete"), b("UDRE0", 0x20, "Data Register Empty"), b("FE0", 0x10, "Frame Error"),
            b("DOR0", 0x08, "Data OverRun"), b("UPE0", 0x04, "Parity Error"), b("U2X0", 0x02, "Double Transmission Speed"), b("MPCM0", 0x01, "Multi-processor Communication Mode"),
        ]), 0x20),
        reg("UCSR0B", 0xc1, "USART0", "USART Control and Status Register B", vec![
            b("RXCIE0", 0x80, "RX Complete Interrupt Enable"), b("TXCIE0", 0x40, "TX Complete Interrupt Enable"), b("UDRIE0", 0x20, "Data Register Empty Interrupt Enable"),
            b("RXEN0", 0x10, "Receiver Enable"), b("TXEN0", 0x08, "Transmitter Enable"), b("UCSZ02", 0x04, "Character Size bit 2"), b("RXB80", 0x02, "Receive Data Bit 8"), b("TXB80", 0x01, "Transmit Data Bit 8"),
        ]),
        reset(reg("UCSR0C", 0xc2, "USART0", "USART Control and Status Register C", vec![
            b("UMSEL0", 0xc0, "USART Mode Select (00 asynchronous)"), b("UPM0", 0x30, "Parity Mode (00 none, 10 even, 11 odd)"), b("USBS0", 0x08, "Stop Bit Select (1 = 2 stop bits)"),
            b("UCSZ0", 0x06, "Character Size bits 1:0 (11 = 8 bits)"), b("UCPOL0", 0x01, "Clock Polarity"),
        ]), 0x06),
        reg("UBRR0L", 0xc4, "USART0", "USART Baud Rate Register Low Byte", vec![]),
        reg("UBRR0H", 0xc5, "USART0", "USART Baud Rate Register High Byte", vec![]),
        reg("UDR0", 0xc6, "USART0", "USART I/O Data Register", vec![]),
    ];
    r.sort_by_key(|x| x.addr);
    r
}

const VECTOR_NAMES: [(&str, &str); 26] = [
    ("RESET", "External Pin, Power-on Reset, Brown-out Reset and Watchdog System Reset"),
    ("INT0", "External Interrupt Request 0"),
    ("INT1", "External Interrupt Request 1"),
    ("PCINT0", "Pin Change Interrupt Request 0"),
    ("PCINT1", "Pin Change Interrupt Request 1"),
    ("PCINT2", "Pin Change Interrupt Request 2"),
    ("WDT", "Watchdog Time-out Interrupt"),
    ("TIMER2_COMPA", "Timer/Counter2 Compare Match A"),
    ("TIMER2_COMPB", "Timer/Counter2 Compare Match B"),
    ("TIMER2_OVF", "Timer/Counter2 Overflow"),
    ("TIMER1_CAPT", "Timer/Counter1 Capture Event"),
    ("TIMER1_COMPA", "Timer/Counter1 Compare Match A"),
    ("TIMER1_COMPB", "Timer/Counter1 Compare Match B"),
    ("TIMER1_OVF", "Timer/Counter1 Overflow"),
    ("TIMER0_COMPA", "Timer/Counter0 Compare Match A"),
    ("TIMER0_COMPB", "Timer/Counter0 Compare Match B"),
    ("TIMER0_OVF", "Timer/Counter0 Overflow"),
    ("SPI_STC", "SPI Serial Transfer Complete"),
    ("USART_RX", "USART Rx Complete"),
    ("USART_UDRE", "USART Data Register Empty"),
    ("USART_TX", "USART Tx Complete"),
    ("ADC", "ADC Conversion Complete"),
    ("EE_READY", "EEPROM Ready"),
    ("ANALOG_COMP", "Analog Comparator"),
    ("TWI", "2-wire Serial Interface"),
    ("SPM_READY", "Store Program Memory Ready"),
];

fn fuses(v: &Variant) -> Vec<FuseByteSpec> {
    let f = |n: &str, m: u8, d: &str| FuseBitSpec { name: n.into(), mask: m, desc: d.into() };
    let low = FuseByteSpec {
        name: "Low".into(),
        default: 0x62,
        bits: vec![
            f("CKDIV8", 0x80, "Divide clock by 8 at reset (CLKPR = /8) when programmed (0)"),
            f("CKOUT", 0x40, "Clock output on PB0 (CLKO) when programmed (0)"),
            f("SUT", 0x30, "Start-up time select"),
            f("CKSEL", 0x0f, "Clock source (0000 external clock, 0010 internal 8 MHz RC, 0011 internal 128 kHz, 0100-0101 32 kHz crystal, 0110-0111 full-swing crystal, 1000-1111 crystal)"),
        ],
    };
    let common = || vec![
        f("RSTDISBL", 0x80, "External reset disabled (PC6 becomes I/O) when programmed (0)"),
        f("DWEN", 0x40, "debugWIRE enabled when programmed (0)"),
        f("SPIEN", 0x20, "Serial programming enabled when programmed (0)"),
        f("WDTON", 0x10, "Watchdog Timer always on when programmed (0)"),
        f("EESAVE", 0x08, "EEPROM preserved through chip erase when programmed (0)"),
    ];
    let bod = |mask: u8| f("BODLEVEL", mask, "Brown-out detector level (111 disabled, 110 1.8 V, 101 2.7 V, 100 4.3 V)");
    let boot = || vec![f("BOOTSZ", 0x06, "Boot section size (see boot loader table)"), f("BOOTRST", 0x01, "Reset to the boot loader section when programmed (0)")];
    match v.layout {
        FuseLayout::M48 => {
            let mut hi = common();
            hi.push(bod(0x07));
            vec![low, FuseByteSpec { name: "High".into(), default: 0xdf, bits: hi }, FuseByteSpec { name: "Extended".into(), default: 0xff, bits: vec![f("SELFPRGEN", 0x01, "Self-programming enabled when programmed (0)")] }]
        }
        FuseLayout::M88 => {
            let mut hi = common();
            hi.push(bod(0x07));
            vec![low, FuseByteSpec { name: "High".into(), default: 0xdf, bits: hi }, FuseByteSpec { name: "Extended".into(), default: 0xf9, bits: boot() }]
        }
        FuseLayout::M328 => {
            let mut hi = common();
            hi.extend(boot());
            vec![low, FuseByteSpec { name: "High".into(), default: 0xd9, bits: hi }, FuseByteSpec { name: "Extended".into(), default: 0xff, bits: vec![bod(0x07)] }]
        }
    }
}

fn spec(v: &Variant) -> AvrDeviceSpec {
    let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let io = |number: u8, name: &str, gpio: u8, functions: Vec<String>| PinSpec { number, name: name.into(), kind: PinKind::Io, gpio: Some(gpio), functions };
    let power = |number: u8, name: &str, kind: PinKind| PinSpec { number, name: name.into(), kind, gpio: None, functions: vec![] };
    // GPIO numbering: PB0-7 = 0-7, PC0-6 = 8-14, PD0-7 = 15-22.
    let pins = vec![
        io(1, "PC6", 14, s(&["RESET", "PCINT14"])),
        io(2, "PD0", 15, s(&["RXD", "PCINT16"])),
        io(3, "PD1", 16, s(&["TXD", "PCINT17"])),
        io(4, "PD2", 17, s(&["INT0", "PCINT18"])),
        io(5, "PD3", 18, s(&["INT1", "OC2B", "PCINT19"])),
        io(6, "PD4", 19, s(&["T0", "XCK", "PCINT20"])),
        power(7, "VCC", PinKind::Vcc),
        power(8, "GND", PinKind::Gnd),
        io(9, "PB6", 6, s(&["XTAL1", "TOSC1", "PCINT6"])),
        io(10, "PB7", 7, s(&["XTAL2", "TOSC2", "PCINT7"])),
        io(11, "PD5", 20, s(&["T1", "OC0B", "PCINT21"])),
        io(12, "PD6", 21, s(&["AIN0", "OC0A", "PCINT22"])),
        io(13, "PD7", 22, s(&["AIN1", "PCINT23"])),
        io(14, "PB0", 0, s(&["ICP1", "CLKO", "PCINT0"])),
        io(15, "PB1", 1, s(&["OC1A", "PCINT1"])),
        io(16, "PB2", 2, s(&["SS", "OC1B", "PCINT2"])),
        io(17, "PB3", 3, s(&["MOSI", "OC2A", "PCINT3"])),
        io(18, "PB4", 4, s(&["MISO", "PCINT4"])),
        io(19, "PB5", 5, s(&["SCK", "PCINT5"])),
        power(20, "AVCC", PinKind::Ref),
        power(21, "AREF", PinKind::Ref),
        power(22, "GND", PinKind::Gnd),
        io(23, "PC0", 8, s(&["ADC0", "PCINT8"])),
        io(24, "PC1", 9, s(&["ADC1", "PCINT9"])),
        io(25, "PC2", 10, s(&["ADC2", "PCINT10"])),
        io(26, "PC3", 11, s(&["ADC3", "PCINT11"])),
        io(27, "PC4", 12, s(&["ADC4", "SDA", "PCINT12"])),
        io(28, "PC5", 13, s(&["ADC5", "SCL", "PCINT13"])),
    ];
    let groups = [
        ("CPU", "CPU, Clock, Sleep, Reset & Power"),
        ("PORTB", "I/O Port B"),
        ("PORTC", "I/O Port C"),
        ("PORTD", "I/O Port D"),
        ("EXINT", "External & Pin Change Interrupts"),
        ("TC0", "8-bit Timer/Counter0 with PWM"),
        ("TC1", "16-bit Timer/Counter1 with PWM"),
        ("TC2", "8-bit Timer/Counter2 with PWM (asynchronous)"),
        ("USART0", "USART0 (serial port)"),
        ("SPI", "Serial Peripheral Interface"),
        ("TWI", "2-wire Serial Interface (I2C)"),
        ("AC", "Analog Comparator"),
        ("ADC", "10-bit Analog to Digital Converter"),
        ("EEPROM", "EEPROM"),
        ("WDT", "Watchdog Timer"),
    ];
    let mut features = feature::MOVW | feature::MUL | feature::LPMX | feature::SPM | feature::BREAK;
    if v.jmp {
        features |= feature::JMP;
    }
    AvrDeviceSpec {
        id: v.id.into(),
        name: v.name.into(),
        family: "megaAVR (ATmega48/88/168/328)".into(),
        core_name: if v.jmp { "AVRe+ (AVR5)".into() } else { "AVRe+ (AVR4)".into() },
        features,
        flash_size: v.flash,
        sram_start: 0x100,
        sram_size: v.sram,
        eeprom_size: v.eeprom,
        io_base: 0x20,
        io_size: 64,
        regs_in_data_space: true,
        flash_map_base: None,
        nvm_map: None,
        signature: v.signature,
        calibration: 0x9a,
        fuses: fuses(v),
        // Section 10.11.1 SMCR.
        sleep: SleepControl {
            register: "SMCR".into(),
            se_mask: 0x01,
            sm_mask: 0x0e,
            modes: vec![
                (0, SleepKind::Idle), (1, SleepKind::AdcNoiseReduction), (2, SleepKind::PowerDown), (3, SleepKind::PowerSave),
                (6, SleepKind::Standby), (7, SleepKind::ExtendedStandby),
            ],
        },
        boot: v.boot.map(|sizes_words| BootSpec { sizes_words }),
        vectors: VECTOR_NAMES.iter().enumerate().map(|(i, (n, d))| VectorSpec { index: i as u8, name: (*n).into(), desc: (*d).into() }).collect(),
        registers: registers(v),
        groups: groups.iter().map(|(n, d)| PeripheralGroupSpec { name: (*n).into(), desc: (*d).into() }).collect(),
        package: "PDIP-28".into(),
        pins,
        gpio_count: 23,
        has_adc: true,
        clock: ClockSpec { internal_hz: 8_000_000.0, slow_hz: 128_000.0, default_prescale_log2: 3 },
        vcc: 5.0,
        // Section 29.3 "Speed grades": 4 MHz @ 1.8 V, 10 MHz @ 2.7 V, 20 MHz @ 4.5 V.
        vcc_range: (1.8, 5.5),
        speed_grades: vec![(4e6, 1.8), (10e6, 2.7), (20e6, 4.5)],
        datasheet: "ATmega48A/PA/88A/PA/168A/PA/328/P datasheet DS40002061B".into(),
        die: None,
        peripheral_set: PeripheralSet::MegaX8,
    }
}

pub fn devices() -> Vec<AvrDeviceSpec> {
    VARIANTS.iter().map(spec).collect()
}
