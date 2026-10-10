//! ATtiny2313A / ATtiny4313 (classic AVR core without MUL/JMP, PDIP-20).
//! Source: Atmel "ATtiny2313A/4313" datasheet Atmel-8246B (09/2011): register summary (section
//! 24), fuse tables (section 20.2), clock system (section 6), sleep modes (section 7); register
//! addresses and bit positions cross-checked against avr-libc iotn2313a.h / iotn4313.h.
//!
//! Differences from the ATtiny25/45/85: three GPIO ports (PA0-2, PB0-7, PD0-6), a full USART0
//! (separate UCSRC and UBRRH registers), a 16-bit Timer/Counter1 with input capture sharing
//! TIFR/TIMSK with the 8-bit Timer/Counter0, INT0/INT1 and three pin-change groups, no ADC, a
//! single 8-bit stack pointer register (SPL only) and a non-contiguous sleep mode field
//! (SM1 = MCUCR bit 6, SM0 = bit 4: 00 idle, 01/11 power-down, 10 standby).
//!
//! Clock: CKSEL 0010 = internal 4 MHz, 0100 = internal 8 MHz (Table 6-1); the part ships with
//! the 8 MHz RC oscillator and CKDIV8 programmed (section 6.2.1), i.e. low fuse 0x64. (avr-libc's
//! LFUSE_DEFAULT of 0x62 selects the 4 MHz oscillator and contradicts the data sheet.)
//!
//! Deliberate simplifications: USART SPI mode (UMSEL = 11) and synchronous mode, SPM self-
//! programming, the BOD-disable-in-sleep sequence (BODCR is plain storage) and the speed-versus-
//! VCC derating beyond the three speed grades are not modelled. The ATtiny4313's RAMEND (0x15F)
//! needs a ninth stack pointer bit that the data sheet does not describe; the model keeps the
//! stack pointer's high byte at its reset value (0x01) because there is no SPH register.

use super::tiny_x5::{b, nbits, reg};
use crate::avr::device::*;
use crate::avr::isa::feature;

struct Variant {
    id: &'static str,
    name: &'static str,
    flash: u32,
    sram: u16,
    eeprom: u16,
    signature: [u8; 3],
}

const VARIANTS: &[Variant] = &[
    Variant { id: "attiny2313a", name: "ATtiny2313A", flash: 2048, sram: 128, eeprom: 128, signature: [0x1e, 0x91, 0x0a] },
    Variant { id: "attiny4313", name: "ATtiny4313", flash: 4096, sram: 256, eeprom: 256, signature: [0x1e, 0x92, 0x0d] },
];

fn registers(v: &Variant) -> Vec<IoRegisterSpec> {
    let ramend = 0x60 + v.sram - 1;
    let mut spl = reg("SPL", 0x5d, "CPU", "Stack Pointer (8 bits)", vec![]);
    spl.reset = (ramend & 0xff) as u8;
    let r16 = |name: &str, addr: u16, group: &str, desc: &str| vec![reg(&format!("{name}L"), addr, group, &format!("{desc} Low Byte"), vec![]), reg(&format!("{name}H"), addr + 1, group, &format!("{desc} High Byte"), vec![])];
    let mut r = vec![
        reg("USIBR", 0x20, "USI", "USI Buffer Register", vec![]),
        reg("DIDR", 0x21, "AC", "Digital Input Disable Register", vec![b("AIN1D", 0x02, "AIN1 Digital Input Disable"), b("AIN0D", 0x01, "AIN0 Digital Input Disable")]),
        reg("UBRRH", 0x22, "USART", "USART Baud Rate Register High (UBRR11:8)", vec![]),
        reg("UCSRC", 0x23, "USART", "USART Control and Status Register C", vec![
            b("UMSEL", 0xc0, "USART Mode Select (00 asynchronous, 01 synchronous, 11 master SPI)"), b("UPM", 0x30, "Parity Mode (00 off, 10 even, 11 odd)"),
            b("USBS", 0x08, "Stop Bit Select"), b("UCSZ", 0x06, "Character Size bits 1:0"), b("UCPOL", 0x01, "Clock Polarity"),
        ]),
        reg("PCMSK1", 0x24, "EXINT", "Pin Change Mask Register 1 (PA2:0)", (0..3).rev().map(|i| b(&format!("PCINT{}", 8 + i), 1 << i, "")).collect()),
        reg("PCMSK2", 0x25, "EXINT", "Pin Change Mask Register 2 (PD6:0)", (0..7).rev().map(|i| b(&format!("PCINT{}", 11 + i), 1 << i, "")).collect()),
        reg("PRR", 0x26, "CPU", "Power Reduction Register", vec![b("PRTIM1", 0x08, "Power Reduction Timer/Counter1"), b("PRTIM0", 0x04, "Power Reduction Timer/Counter0"), b("PRUSI", 0x02, "Power Reduction USI"), b("PRUSART", 0x01, "Power Reduction USART")]),
        reg("BODCR", 0x27, "CPU", "Brown-Out Detector Control Register", vec![b("BPDS", 0x02, "BOD Power-down in Sleep"), b("BPDSE", 0x01, "BOD Power-down in Sleep Enable")]),
        reg("ACSR", 0x28, "AC", "Analog Comparator Control and Status Register", vec![
            b("ACD", 0x80, "Analog Comparator Disable"), b("ACBG", 0x40, "Bandgap Select"), b("ACO", 0x20, "Analog Comparator Output"),
            b("ACI", 0x10, "Analog Comparator Interrupt Flag"), b("ACIE", 0x08, "Analog Comparator Interrupt Enable"), b("ACIC", 0x04, "Analog Comparator Input Capture Enable"),
            b("ACIS", 0x03, "Interrupt Mode Select (00 toggle, 10 falling, 11 rising)"),
        ]),
        reg("UBRRL", 0x29, "USART", "USART Baud Rate Register Low (UBRR7:0)", vec![]),
        reg("UCSRB", 0x2a, "USART", "USART Control and Status Register B", vec![
            b("RXCIE", 0x80, "RX Complete Interrupt Enable"), b("TXCIE", 0x40, "TX Complete Interrupt Enable"), b("UDRIE", 0x20, "Data Register Empty Interrupt Enable"),
            b("RXEN", 0x10, "Receiver Enable"), b("TXEN", 0x08, "Transmitter Enable"), b("UCSZ2", 0x04, "Character Size bit 2"), b("RXB8", 0x02, "Receive Data Bit 8"), b("TXB8", 0x01, "Transmit Data Bit 8"),
        ]),
        reg("UCSRA", 0x2b, "USART", "USART Control and Status Register A", vec![
            b("RXC", 0x80, "USART Receive Complete"), b("TXC", 0x40, "USART Transmit Complete"), b("UDRE", 0x20, "USART Data Register Empty"),
            b("FE", 0x10, "Frame Error"), b("DOR", 0x08, "Data OverRun"), b("UPE", 0x04, "Parity Error"), b("U2X", 0x02, "Double the USART Transmission Speed"), b("MPCM", 0x01, "Multi-processor Communication Mode"),
        ]),
        reg("UDR", 0x2c, "USART", "USART I/O Data Register", vec![]),
        reg("USICR", 0x2d, "USI", "USI Control Register", vec![
            b("USISIE", 0x80, "Start Condition Interrupt Enable"), b("USIOIE", 0x40, "Counter Overflow Interrupt Enable"), b("USIWM", 0x30, "Wire Mode (01 three-wire, 10 two-wire)"),
            b("USICS", 0x0c, "Clock Source Select"), b("USICLK", 0x02, "Clock Strobe"), b("USITC", 0x01, "Toggle Clock Port Pin"),
        ]),
        reg("USISR", 0x2e, "USI", "USI Status Register", vec![b("USISIF", 0x80, "Start Condition Interrupt Flag"), b("USIOIF", 0x40, "Counter Overflow Interrupt Flag"), b("USIPF", 0x20, "Stop Condition Flag"), b("USIDC", 0x10, "Data Output Collision"), b("USICNT", 0x0f, "Counter Value")]),
        reg("USIDR", 0x2f, "USI", "USI Data Register", vec![]),
        reg("PIND", 0x30, "PORTD", "Port D Input Pins (write 1 toggles PORTD bit)", nbits("PIND", 0x7f)),
        reg("DDRD", 0x31, "PORTD", "Port D Data Direction Register", nbits("DDD", 0x7f)),
        reg("PORTD", 0x32, "PORTD", "Port D Data Register", nbits("PORTD", 0x7f)),
        reg("GPIOR0", 0x33, "CPU", "General Purpose I/O Register 0", vec![]),
        reg("GPIOR1", 0x34, "CPU", "General Purpose I/O Register 1", vec![]),
        reg("GPIOR2", 0x35, "CPU", "General Purpose I/O Register 2", vec![]),
        reg("PINB", 0x36, "PORTB", "Port B Input Pins (write 1 toggles PORTB bit)", nbits("PINB", 0xff)),
        reg("DDRB", 0x37, "PORTB", "Port B Data Direction Register", nbits("DDB", 0xff)),
        reg("PORTB", 0x38, "PORTB", "Port B Data Register", nbits("PORTB", 0xff)),
        reg("PINA", 0x39, "PORTA", "Port A Input Pins (write 1 toggles PORTA bit)", nbits("PINA", 0x07)),
        reg("DDRA", 0x3a, "PORTA", "Port A Data Direction Register", nbits("DDA", 0x07)),
        reg("PORTA", 0x3b, "PORTA", "Port A Data Register", nbits("PORTA", 0x07)),
        reg("EECR", 0x3c, "EEPROM", "EEPROM Control Register", vec![
            b("EEPM", 0x30, "EEPROM Programming Mode (00 erase+write, 01 erase, 10 write)"), b("EERIE", 0x08, "EEPROM Ready Interrupt Enable"),
            b("EEMPE", 0x04, "EEPROM Master Program Enable"), b("EEPE", 0x02, "EEPROM Program Enable"), b("EERE", 0x01, "EEPROM Read Enable"),
        ]),
        reg("EEDR", 0x3d, "EEPROM", "EEPROM Data Register", vec![]),
        reg("EEAR", 0x3e, "EEPROM", "EEPROM Address Register", vec![]),
        reg("PCMSK0", 0x40, "EXINT", "Pin Change Mask Register 0 (PB7:0)", nbits("PCINT", 0xff)),
        reg("WDTCSR", 0x41, "WDT", "Watchdog Timer Control Register", vec![
            b("WDIF", 0x80, "Watchdog Interrupt Flag"), b("WDIE", 0x40, "Watchdog Interrupt Enable"), b("WDP3", 0x20, "Watchdog Prescaler bit 3"),
            b("WDCE", 0x10, "Watchdog Change Enable"), b("WDE", 0x08, "Watchdog System Reset Enable"), b("WDP", 0x07, "Watchdog Prescaler bits 2:0"),
        ]),
        reg("TCCR1C", 0x42, "TC1", "Timer/Counter1 Control Register C", vec![b("FOC1A", 0x80, "Force Output Compare for Channel A"), b("FOC1B", 0x40, "Force Output Compare for Channel B")]),
        reg("GTCCR", 0x43, "TC1", "General Timer/Counter Control Register", vec![b("PSR10", 0x01, "Prescaler Reset Timer/Counter1 and Timer/Counter0")]),
        reg("CLKPR", 0x46, "CPU", "Clock Prescale Register", vec![b("CLKPCE", 0x80, "Clock Prescaler Change Enable"), b("CLKPS", 0x0f, "Clock Prescaler Select (division = 2^CLKPS)")]),
        reg("TCCR1B", 0x4e, "TC1", "Timer/Counter1 Control Register B", vec![
            b("ICNC1", 0x80, "Input Capture Noise Canceler"), b("ICES1", 0x40, "Input Capture Edge Select"), b("WGM13", 0x10, "Waveform Generation Mode bit 3"), b("WGM12", 0x08, "Waveform Generation Mode bit 2"),
            b("CS1", 0x07, "Clock Select (0 stop, 1 /1, 2 /8, 3 /64, 4 /256, 5 /1024, 6 T1 falling, 7 T1 rising)"),
        ]),
        reg("TCCR1A", 0x4f, "TC1", "Timer/Counter1 Control Register A", vec![b("COM1A", 0xc0, "Compare Output Mode A"), b("COM1B", 0x30, "Compare Output Mode B"), b("WGM11", 0x02, "Waveform Generation Mode bit 1"), b("WGM10", 0x01, "Waveform Generation Mode bit 0")]),
        reg("TCCR0A", 0x50, "TC0", "Timer/Counter0 Control Register A", vec![b("COM0A", 0xc0, "Compare Output Mode A"), b("COM0B", 0x30, "Compare Output Mode B"), b("WGM01", 0x02, "Waveform Generation Mode bit 1"), b("WGM00", 0x01, "Waveform Generation Mode bit 0")]),
        reg("OSCCAL", 0x51, "CPU", "Oscillator Calibration Register", vec![]),
        reg("TCNT0", 0x52, "TC0", "Timer/Counter0", vec![]),
        reg("TCCR0B", 0x53, "TC0", "Timer/Counter0 Control Register B", vec![b("FOC0A", 0x80, "Force Output Compare A"), b("FOC0B", 0x40, "Force Output Compare B"), b("WGM02", 0x08, "Waveform Generation Mode bit 2"), b("CS0", 0x07, "Clock Select (0 stop, 1 /1, 2 /8, 3 /64, 4 /256, 5 /1024, 6 T0 falling, 7 T0 rising)")]),
        reg("MCUSR", 0x54, "CPU", "MCU Status Register (reset flags)", vec![b("WDRF", 0x08, "Watchdog Reset Flag"), b("BORF", 0x04, "Brown-out Reset Flag"), b("EXTRF", 0x02, "External Reset Flag"), b("PORF", 0x01, "Power-on Reset Flag")]),
        reg("MCUCR", 0x55, "CPU", "MCU Control Register", vec![
            b("PUD", 0x80, "Pull-up Disable"), b("SM1", 0x40, "Sleep Mode bit 1"), b("SE", 0x20, "Sleep Enable"), b("SM0", 0x10, "Sleep Mode bit 0 (SM1:0 = 00 idle, 01 power-down, 10 standby, 11 power-down)"),
            b("ISC1", 0x0c, "Interrupt Sense Control 1 (00 low, 01 any, 10 falling, 11 rising)"), b("ISC0", 0x03, "Interrupt Sense Control 0 (00 low, 01 any, 10 falling, 11 rising)"),
        ]),
        reg("OCR0A", 0x56, "TC0", "Output Compare Register 0 A", vec![]),
        reg("SPMCSR", 0x57, "CPU", "Store Program Memory Control and Status Register", vec![b("RSIG", 0x20, "Read Device Signature Imprint Table"), b("CTPB", 0x10, "Clear Temporary Page Buffer"), b("RFLB", 0x08, "Read Fuse and Lock Bits"), b("PGWRT", 0x04, "Page Write"), b("PGERS", 0x02, "Page Erase"), b("SPMEN", 0x01, "Self Programming Enable")]),
        reg("TIFR", 0x58, "TC0", "Timer/Counter Interrupt Flag Register", vec![
            b("TOV1", 0x80, "Timer1 Overflow Flag"), b("OCF1A", 0x40, "Timer1 Output Compare A Flag"), b("OCF1B", 0x20, "Timer1 Output Compare B Flag"), b("ICF1", 0x08, "Timer1 Input Capture Flag"),
            b("OCF0B", 0x04, "Timer0 Output Compare B Flag"), b("TOV0", 0x02, "Timer0 Overflow Flag"), b("OCF0A", 0x01, "Timer0 Output Compare A Flag"),
        ]),
        reg("TIMSK", 0x59, "TC0", "Timer/Counter Interrupt Mask Register", vec![
            b("TOIE1", 0x80, "Timer1 Overflow Interrupt Enable"), b("OCIE1A", 0x40, "Timer1 Output Compare A Interrupt Enable"), b("OCIE1B", 0x20, "Timer1 Output Compare B Interrupt Enable"), b("ICIE1", 0x08, "Timer1 Input Capture Interrupt Enable"),
            b("OCIE0B", 0x04, "Timer0 Output Compare B Interrupt Enable"), b("TOIE0", 0x02, "Timer0 Overflow Interrupt Enable"), b("OCIE0A", 0x01, "Timer0 Output Compare A Interrupt Enable"),
        ]),
        reg("GIFR", 0x5a, "EXINT", "General Interrupt Flag Register", vec![
            b("INTF1", 0x80, "External Interrupt Flag 1"), b("INTF0", 0x40, "External Interrupt Flag 0"), b("PCIF0", 0x20, "Pin Change Interrupt Flag 0"),
            b("PCIF2", 0x10, "Pin Change Interrupt Flag 2"), b("PCIF1", 0x08, "Pin Change Interrupt Flag 1"),
        ]),
        reg("GIMSK", 0x5b, "EXINT", "General Interrupt Mask Register", vec![
            b("INT1", 0x80, "External Interrupt Request 1 Enable"), b("INT0", 0x40, "External Interrupt Request 0 Enable"), b("PCIE0", 0x20, "Pin Change Interrupt Enable 0"),
            b("PCIE2", 0x10, "Pin Change Interrupt Enable 2"), b("PCIE1", 0x08, "Pin Change Interrupt Enable 1"),
        ]),
        reg("OCR0B", 0x5c, "TC0", "Output Compare Register 0 B", vec![]),
        spl,
        reg("SREG", 0x5f, "CPU", "Status Register", bits_msb_first(
            &[Some("I"), Some("T"), Some("H"), Some("S"), Some("V"), Some("N"), Some("Z"), Some("C")],
            &[("I", "Global Interrupt Enable"), ("T", "Bit Copy Storage"), ("H", "Half Carry Flag"), ("S", "Sign Bit (N xor V)"),
              ("V", "Two's Complement Overflow Flag"), ("N", "Negative Flag"), ("Z", "Zero Flag"), ("C", "Carry Flag")],
        )),
    ];
    r.extend(r16("ICR1", 0x44, "TC1", "Timer/Counter1 Input Capture Register"));
    r.extend(r16("OCR1B", 0x48, "TC1", "Timer/Counter1 Output Compare Register B"));
    r.extend(r16("OCR1A", 0x4a, "TC1", "Timer/Counter1 Output Compare Register A"));
    r.extend(r16("TCNT1", 0x4c, "TC1", "Timer/Counter1"));
    r.sort_by_key(|x| x.addr);
    r
}

const VECTOR_NAMES: [(&str, &str); 21] = [
    ("RESET", "External Pin, Power-on Reset, Brown-out Reset, Watchdog Reset"),
    ("INT0", "External Interrupt Request 0"),
    ("INT1", "External Interrupt Request 1"),
    ("TIMER1_CAPT", "Timer/Counter1 Capture Event"),
    ("TIMER1_COMPA", "Timer/Counter1 Compare Match A"),
    ("TIMER1_OVF", "Timer/Counter1 Overflow"),
    ("TIMER0_OVF", "Timer/Counter0 Overflow"),
    ("USART_RX", "USART0, Rx Complete"),
    ("USART_UDRE", "USART0 Data Register Empty"),
    ("USART_TX", "USART0, Tx Complete"),
    ("ANALOG_COMP", "Analog Comparator"),
    ("PCINT0", "Pin Change Interrupt Request 0 (PB7:0)"),
    ("TIMER1_COMPB", "Timer/Counter1 Compare Match B"),
    ("TIMER0_COMPA", "Timer/Counter0 Compare Match A"),
    ("TIMER0_COMPB", "Timer/Counter0 Compare Match B"),
    ("USI_START", "USI Start Condition"),
    ("USI_OVF", "USI Overflow"),
    ("EE_READY", "EEPROM Ready"),
    ("WDT", "Watchdog Timer Overflow"),
    ("PCINT1", "Pin Change Interrupt Request 1 (PA2:0)"),
    ("PCINT2", "Pin Change Interrupt Request 2 (PD6:0)"),
];

fn fuses() -> Vec<FuseByteSpec> {
    let f = |n: &str, m: u8, d: &str| FuseBitSpec { name: n.into(), mask: m, desc: d.into() };
    vec![
        FuseByteSpec {
            name: "Low".into(),
            default: 0x64,
            bits: vec![
                f("CKDIV8", 0x80, "Divide clock by 8 at reset (CLKPR = /8) when programmed (0)"),
                f("CKOUT", 0x40, "Clock output on PD2 (CKOUT) when programmed (0)"),
                f("SUT", 0x30, "Start-up time select"),
                f("CKSEL", 0x0f, "Clock source (0000 external clock, 0010 internal 4 MHz, 0100 internal 8 MHz, 0110 internal 128 kHz, 1000-1111 crystal)"),
            ],
        },
        FuseByteSpec {
            name: "High".into(),
            default: 0xdf,
            bits: vec![
                f("DWEN", 0x80, "debugWIRE enabled when programmed (0)"),
                f("EESAVE", 0x40, "EEPROM preserved through chip erase when programmed (0)"),
                f("SPIEN", 0x20, "Serial programming enabled when programmed (0)"),
                f("WDTON", 0x10, "Watchdog Timer always on when programmed (0)"),
                f("BODLEVEL", 0x0e, "Brown-out detector level (111 disabled, 110 1.8 V, 101 2.7 V, 100 4.3 V)"),
                f("RSTDISBL", 0x01, "External reset disabled (PA2 becomes I/O) when programmed (0)"),
            ],
        },
        FuseByteSpec { name: "Extended".into(), default: 0xff, bits: vec![f("SELFPRGEN", 0x01, "Self-programming enabled when programmed (0)")] },
    ]
}

fn spec(v: &Variant) -> AvrDeviceSpec {
    let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let io = |number: u8, name: &str, gpio: u8, functions: Vec<String>| PinSpec { number, name: name.into(), kind: PinKind::Io, gpio: Some(gpio), functions };
    let power = |number: u8, name: &str, kind: PinKind| PinSpec { number, name: name.into(), kind, gpio: None, functions: vec![] };
    // GPIO numbering: PA0-2 = 0-2, PB0-7 = 3-10, PD0-6 = 11-17.
    let pins = vec![
        io(1, "PA2", 2, s(&["RESET", "dW", "PCINT10"])),
        io(2, "PD0", 11, s(&["RXD", "PCINT11"])),
        io(3, "PD1", 12, s(&["TXD", "PCINT12"])),
        io(4, "PA1", 1, s(&["XTAL2", "PCINT9"])),
        io(5, "PA0", 0, s(&["XTAL1", "CLKI", "PCINT8"])),
        io(6, "PD2", 13, s(&["XCK", "CKOUT", "INT0", "PCINT13"])),
        io(7, "PD3", 14, s(&["INT1", "PCINT14"])),
        io(8, "PD4", 15, s(&["T0", "PCINT15"])),
        io(9, "PD5", 16, s(&["T1", "OC0B", "PCINT16"])),
        power(10, "GND", PinKind::Gnd),
        io(11, "PD6", 17, s(&["ICP", "PCINT17"])),
        io(12, "PB0", 3, s(&["AIN0", "PCINT0"])),
        io(13, "PB1", 4, s(&["AIN1", "PCINT1"])),
        io(14, "PB2", 5, s(&["OC0A", "PCINT2"])),
        io(15, "PB3", 6, s(&["OC1A", "PCINT3"])),
        io(16, "PB4", 7, s(&["OC1B", "PCINT4"])),
        io(17, "PB5", 8, s(&["MOSI", "DI", "SDA", "PCINT5"])),
        io(18, "PB6", 9, s(&["MISO", "DO", "PCINT6"])),
        io(19, "PB7", 10, s(&["SCK", "USCK", "SCL", "PCINT7"])),
        power(20, "VCC", PinKind::Vcc),
    ];
    let groups = [
        ("CPU", "CPU, Clock, Sleep, Reset & Power"),
        ("PORTA", "I/O Port A"),
        ("PORTB", "I/O Port B"),
        ("PORTD", "I/O Port D"),
        ("EXINT", "External & Pin Change Interrupts"),
        ("TC0", "8-bit Timer/Counter0 with PWM"),
        ("TC1", "16-bit Timer/Counter1 with PWM and input capture"),
        ("USART", "USART0"),
        ("USI", "Universal Serial Interface"),
        ("AC", "Analog Comparator"),
        ("EEPROM", "EEPROM"),
        ("WDT", "Watchdog Timer"),
    ];
    AvrDeviceSpec {
        id: v.id.into(),
        name: v.name.into(),
        family: "tinyAVR (ATtiny2313A/4313)".into(),
        core_name: "AVRe (AVR25)".into(),
        features: feature::MOVW | feature::LPMX | feature::SPM | feature::BREAK,
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
        calibration: 0x80, // nominal: the real value is per chip
        fuses: fuses(),
        // MCUCR: SM1 (bit 6) and SM0 (bit 4) are not adjacent, so the raw mode value below is
        // (SM1 << 2) | SM0: 0 idle, 1 power-down, 4 standby, 5 power-down (Table 7-2).
        sleep: SleepControl {
            register: "MCUCR".into(),
            se_mask: 0x20,
            sm_mask: 0x50,
            modes: vec![(0, SleepKind::Idle), (1, SleepKind::PowerDown), (4, SleepKind::Standby), (5, SleepKind::PowerDown)],
        },
        boot: None,
        vectors: VECTOR_NAMES.iter().enumerate().map(|(i, (n, d))| VectorSpec { index: i as u8, name: (*n).into(), desc: (*d).into() }).collect(),
        registers: registers(v),
        groups: groups.iter().map(|(n, d)| PeripheralGroupSpec { name: (*n).into(), desc: (*d).into() }).collect(),
        package: "PDIP-20".into(),
        pins,
        gpio_count: 18,
        has_adc: false,
        clock: ClockSpec { internal_hz: 8_000_000.0, slow_hz: 128_000.0, default_prescale_log2: 3 },
        vcc: 5.0,
        // Speed grades: 4 MHz @ 1.8-5.5 V, 10 MHz @ 2.7-5.5 V, 20 MHz @ 4.5-5.5 V.
        vcc_range: (1.8, 5.5),
        speed_grades: vec![(4e6, 1.8), (10e6, 2.7), (20e6, 4.5)],
        datasheet: "ATtiny2313A/4313 datasheet Atmel-8246B (09/2011)".into(),
        die: None,
        peripheral_set: PeripheralSet::TinyX313,
    }
}

pub fn devices() -> Vec<AvrDeviceSpec> {
    VARIANTS.iter().map(spec).collect()
}
