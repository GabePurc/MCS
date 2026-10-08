//! CPU, peripheral and timing tests for the ATtiny10 model. Programs are encoded directly from
//! the instruction table so these tests do not depend on the assembler crate.

use mcs_core::avr::devices;
use mcs_core::avr::isa::{def_by_op, encode, op};
use mcs_core::program::{LoadedProgram, ProgramFormat};
use mcs_sim::avr::cpu::*;
use mcs_sim::avr::{Machine, ResetSource, StopReason};
use mcs_sim::pins::ExtDrive;

type Insn<'a> = (u8, &'a [i32]);

fn image(insns: &[Insn], size: usize) -> Vec<u8> {
    let mut out = vec![0xff; size];
    let mut w = 0;
    for (o, vals) in insns {
        let def = def_by_op(*o).unwrap();
        for word in encode(def, vals).unwrap_or_else(|e| panic!("{}: {e}", def.name)) {
            out[w * 2] = word as u8;
            out[w * 2 + 1] = (word >> 8) as u8;
            w += 1;
        }
    }
    out
}

fn tiny10(insns: &[Insn]) -> Machine {
    let spec = devices::get("attiny10").unwrap();
    let mut m = Machine::new(spec);
    let mut p = LoadedProgram::empty(ProgramFormat::Asm, spec.flash_size as usize);
    p.flash = image(insns, spec.flash_size as usize);
    m.load(&p);
    m
}

// I/O addresses (ATtiny10)
const PINB: i32 = 0x00;
const DDRB: i32 = 0x01;
const PUEB: i32 = 0x03;
const PCMSK: i32 = 0x10;
const PCICR: i32 = 0x12;
const ADMUX: i32 = 0x1b;
const ADCSRA: i32 = 0x1d;
const ADCL: i32 = 0x19;
const ICR0L: i32 = 0x22;
const ICR0H: i32 = 0x23;
const OCR0AL: i32 = 0x26;
const OCR0AH: i32 = 0x27;
const TIMSK0: i32 = 0x2b;
const TCCR0B: i32 = 0x2d;
const TCCR0A: i32 = 0x2e;
const WDTCSR: i32 = 0x31;
const CLKPSR: i32 = 0x36;
const SMCR: i32 = 0x3a;
const CCP: i32 = 0x3c;

fn run_alu(a: i32, b: i32, o: u8) -> (u8, u8) {
    let mut m = tiny10(&[(op::LDI, &[16, a]), (op::LDI, &[17, b]), (op::BCLR, &[0]), (o, &[16, 17]), (op::BREAK, &[])]);
    m.run(1000);
    (m.cpu.r[16], m.cpu.sreg)
}

#[test]
fn add_sets_carry_overflow_zero() {
    let (r, s) = run_alu(0x80, 0x80, op::ADD);
    assert_eq!(r, 0);
    assert_eq!(s & (SREG_C | SREG_V | SREG_Z | SREG_S), SREG_C | SREG_V | SREG_Z | SREG_S);
}

#[test]
fn add_half_carry() {
    let (r, s) = run_alu(0x0f, 0x01, op::ADD);
    assert_eq!(r, 0x10);
    assert_eq!(s & SREG_H, SREG_H);
    assert_eq!(s & SREG_C, 0);
}

#[test]
fn sub_borrow_and_overflow() {
    let (r, s) = run_alu(0x01, 0x02, op::SUB);
    assert_eq!(r, 0xff);
    assert_eq!(s & (SREG_C | SREG_N | SREG_S | SREG_H), SREG_C | SREG_N | SREG_S | SREG_H);
    let (r, s) = run_alu(0x80, 0x01, op::SUB);
    assert_eq!(r, 0x7f);
    assert_eq!(s & (SREG_V | SREG_S | SREG_N), SREG_V | SREG_S);
}

#[test]
fn cpc_keeps_zero_sticky() {
    let mut m = tiny10(&[(op::LDI, &[16, 5]), (op::LDI, &[17, 5]), (op::LDI, &[18, 6]), (op::LDI, &[19, 7]), (op::CP, &[16, 17]), (op::CPC, &[18, 19]), (op::BREAK, &[])]);
    m.run(100);
    assert_eq!(m.cpu.sreg & SREG_Z, 0);
    assert_eq!(m.cpu.sreg & SREG_C, SREG_C);
}

#[test]
fn unary_ops() {
    let mut m = tiny10(&[
        (op::LDI, &[16, 0x80]), (op::NEG, &[16]),
        (op::LDI, &[17, 0x55]), (op::COM, &[17]),
        (op::LDI, &[18, 0x7f]), (op::INC, &[18]),
        (op::LDI, &[19, 0x81]), (op::ASR, &[19]),
        (op::LDI, &[20, 0x81]), (op::LSR, &[20]),
        (op::LDI, &[21, 0x02]), (op::BSET, &[0]), (op::ROR, &[21]),
        (op::LDI, &[22, 0x12]), (op::SWAP, &[22]),
        (op::BREAK, &[]),
    ]);
    assert_eq!(m.run(1000), StopReason::BreakInsn);
    assert_eq!(&m.cpu.r[16..23], &[0x80, 0xaa, 0x80, 0xc0, 0x40, 0x81, 0x21]);
}

#[test]
fn loop_cycle_count_is_exact() {
    let mut m = tiny10(&[(op::LDI, &[16, 10]), (op::DEC, &[16]), (op::BRBC, &[1, -2]), (op::BREAK, &[])]);
    m.run(10_000);
    assert_eq!(m.cpu.cycles, 1 + 10 + 9 * 2 + 1 + 1);
}

#[test]
fn rcall_ret_stack_and_timing() {
    let mut m = tiny10(&[(op::RCALL, &[1]), (op::BREAK, &[]), (op::LDI, &[16, 42]), (op::RET, &[])]);
    m.run(1000);
    assert_eq!(m.cpu.r[16], 42);
    assert_eq!(m.cpu.sp, 0x5f);
    assert_eq!(m.cpu.cycles, 3 + 1 + 4 + 1);
    // Return address stored big-endian: [SP+1]=high, [SP+2]=low
    assert_eq!(m.cpu.data[0x5f], 1);
    assert_eq!(m.cpu.data[0x5e], 0);
}

#[test]
fn pointer_loads_stores_and_flash_mapping() {
    let mut m = tiny10(&[
        (op::LDI, &[26, 0x40]), (op::LDI, &[27, 0]), (op::LDI, &[16, 0xab]), (op::ST_XP, &[16]), (op::ST_XP, &[16]),
        (op::LDS_RC, &[17, 0x41]), (op::STS_RC, &[0x50, 17]),
        (op::LDI, &[30, 0x00]), (op::LDI, &[31, 0x40]), (op::LD_ZP, &[18]), (op::LD_Z, &[19]),
        (op::BREAK, &[]),
    ]);
    m.run(1000);
    assert_eq!(m.cpu.data[0x40], 0xab);
    assert_eq!(m.cpu.data[0x41], 0xab);
    assert_eq!(m.cpu.data[0x50], 0xab);
    assert_eq!(m.cpu.r[26], 0x42);
    assert_eq!(m.cpu.r[18], 0xa0); // LDI r26,0x40 = 0xE4A0
    assert_eq!(m.cpu.r[19], 0xe4);
}

#[test]
fn signature_via_nvm_mapping_and_skip() {
    let mut m = tiny10(&[
        (op::LDI, &[30, 0xc0]), (op::LDI, &[31, 0x3f]), (op::LD_ZP, &[16]), (op::LD_ZP, &[17]), (op::LD_ZP, &[18]),
        (op::SBRS, &[16, 0]), (op::LDI, &[20, 1]), (op::BREAK, &[]),
    ]);
    m.run(1000);
    assert_eq!(&m.cpu.r[16..19], &[0x1e, 0x90, 0x03]);
    assert_eq!(m.cpu.r[20], 1);
}

#[test]
fn erased_flash_stops_with_invalid_opcode() {
    let mut m = tiny10(&[(op::NOP, &[])]);
    assert_eq!(m.run(1000), StopReason::InvalidOpcode);
    assert_eq!(m.cpu.pc, 1);
    assert!(m.messages().iter().any(|x| x.text.contains("erased flash")));
}

#[test]
fn breakpoints_stop_before_execution_and_resume() {
    let mut m = tiny10(&[(op::LDI, &[16, 1]), (op::LDI, &[16, 2]), (op::LDI, &[16, 3]), (op::RJMP, &[-1])]);
    m.cpu.breakpoints[2] = true;
    assert_eq!(m.run(1000), StopReason::Breakpoint);
    assert_eq!(m.cpu.pc, 2);
    assert_eq!(m.cpu.r[16], 2);
    assert_eq!(m.run(1000), StopReason::Limit);
    assert_eq!(m.cpu.r[16], 3);
}

#[test]
fn gpio_toggle_trace() {
    let mut m = tiny10(&[(op::LDI, &[16, 1]), (op::OUT, &[DDRB, 16]), (op::SBI, &[PINB, 0]), (op::RJMP, &[-2])]);
    m.run(100);
    let (_, c, _) = m.sys.trace.read_since(0, usize::MAX);
    assert!(c.len() > 10);
    assert_eq!(c[3] - c[2], 3); // sbi 1 + rjmp 2
}

#[test]
fn external_input_and_pullups() {
    let mut m = tiny10(&[(op::LDI, &[16, 0x02]), (op::OUT, &[PUEB, 16]), (op::IN, &[17, PINB]), (op::BREAK, &[])]);
    m.set_pin_input(0, ExtDrive::High, 0.0);
    m.run(100);
    assert_eq!(m.cpu.r[17] & 3, 3);
}

#[test]
fn pin_change_wakes_from_power_down() {
    let mut m = tiny10(&[
        (op::RJMP, &[10]), (op::RETI, &[]), (op::RJMP, &[16]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]),
        (op::LDI, &[16, 1]), (op::OUT, &[PCMSK, 16]), (op::OUT, &[PCICR, 16]),
        (op::LDI, &[16, 5]), (op::OUT, &[SMCR, 16]),
        (op::BSET, &[7]), (op::SLEEP, &[]), (op::BREAK, &[]),
        (op::LDI, &[20, 1]), (op::RETI, &[]),
    ]);
    m.run(10_000);
    assert!(m.cpu.sleeping);
    assert_eq!(m.cpu.cycles, 10_000);
    m.set_pin_input(0, ExtDrive::High, 0.0);
    assert_eq!(m.run(20_000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20], 1);
}

#[test]
fn timer_overflow_interrupt_every_65536_cycles() {
    let mut m = tiny10(&[
        (op::RJMP, &[10]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]), (op::RJMP, &[11]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]),
        (op::LDI, &[16, 1]), (op::OUT, &[TIMSK0, 16]), (op::OUT, &[TCCR0B, 16]), (op::BSET, &[7]), (op::RJMP, &[-1]),
        (op::INC, &[20]), (op::RETI, &[]),
    ]);
    m.run(65536 * 3 + 200);
    assert_eq!(m.cpu.r[20], 3);
}

#[test]
fn ctc_toggles_oc0a_every_ocr_plus_one() {
    let mut m = tiny10(&[
        (op::LDI, &[16, 1]), (op::OUT, &[DDRB, 16]),
        (op::LDI, &[16, 0]), (op::OUT, &[OCR0AH, 16]), (op::LDI, &[16, 99]), (op::OUT, &[OCR0AL, 16]),
        (op::LDI, &[16, 0x40]), (op::OUT, &[TCCR0A, 16]),
        (op::LDI, &[16, 0x09]), (op::OUT, &[TCCR0B, 16]),
        (op::RJMP, &[-1]),
    ]);
    m.run(5000);
    let (_, c, _) = m.sys.trace.read_since(0, usize::MAX);
    let edges = &c[2..];
    assert!(edges.len() > 40);
    for w in edges.windows(2) {
        assert_eq!(w[1] - w[0], 100);
    }
}

#[test]
fn fast_pwm_icr_top_duty_cycle() {
    let mut m = tiny10(&[
        (op::LDI, &[16, 1]), (op::OUT, &[DDRB, 16]),
        (op::LDI, &[16, 0x03]), (op::OUT, &[ICR0H, 16]), (op::LDI, &[16, 0xe7]), (op::OUT, &[ICR0L, 16]),
        (op::LDI, &[16, 0x00]), (op::OUT, &[OCR0AH, 16]), (op::LDI, &[16, 250]), (op::OUT, &[OCR0AL, 16]),
        (op::LDI, &[16, 0x82]), (op::OUT, &[TCCR0A, 16]),
        (op::LDI, &[16, 0x19]), (op::OUT, &[TCCR0B, 16]),
        (op::RJMP, &[-1]),
    ]);
    m.run(20_000);
    let (_, c, l) = m.sys.trace.read_since(0, usize::MAX);
    let rises: Vec<u64> = (1..c.len()).filter(|&i| l[i] & 1 == 1 && l[i - 1] & 1 == 0).map(|i| c[i]).collect();
    let falls: Vec<u64> = (1..c.len()).filter(|&i| l[i] & 1 == 0 && l[i - 1] & 1 == 1).map(|i| c[i]).collect();
    assert!(rises.len() > 10);
    assert_eq!(rises[5] - rises[4], 1000);
    let fall = falls.iter().find(|&&f| f > rises[4]).unwrap();
    assert_eq!(fall - rises[4], 251);
}

#[test]
fn input_capture_latches_tcnt() {
    let mut m = tiny10(&[(op::LDI, &[16, 0x41]), (op::OUT, &[TCCR0B, 16]), (op::RJMP, &[-1])]);
    m.run(1000);
    m.set_pin_input(1, ExtDrive::High, 0.0);
    let tcnt = m.peek_data(0x28) as u32 | ((m.peek_data(0x29) as u32) << 8);
    assert_eq!(m.peek_data(0x2a) & 0x20, 0x20);
    assert_eq!(m.peek_data(0x22) as u32 | ((m.peek_data(0x23) as u32) << 8), tcnt);
}

#[test]
fn watchdog_reset_sets_wdrf_and_forces_wde() {
    let mut m = tiny10(&[(op::LDI, &[16, 0x08]), (op::OUT, &[WDTCSR, 16]), (op::RJMP, &[-1])]);
    m.run(20_000); // 20 ms at 1 MHz (WDT 16 ms)
    assert_eq!(m.sys.last_reset, ResetSource::Watchdog);
    assert_eq!(m.peek_data(0x3b) & 0x08, 0x08);
    assert_eq!(m.peek_data(0x31) & 0x08, 0x08);
}

#[test]
fn ccp_protected_clock_prescaler() {
    let mut m = tiny10(&[
        (op::LDI, &[16, 0]), (op::OUT, &[CLKPSR, 16]),
        (op::LDI, &[17, 0xd8]), (op::OUT, &[CCP, 17]), (op::OUT, &[CLKPSR, 16]), (op::BREAK, &[]),
    ]);
    m.run(100);
    assert_eq!(m.sys.clock.hz, 8_000_000.0);
    assert!(m.messages().iter().any(|x| x.text.contains("CLKPSR write ignored")));
}

#[test]
fn adc_converts_pin_voltage() {
    let mut m = tiny10(&[
        (op::LDI, &[16, 2]), (op::OUT, &[ADMUX, 16]),
        (op::LDI, &[16, 0xc0]), (op::OUT, &[ADCSRA, 16]),
        (op::SBIC, &[ADCSRA, 6]), (op::RJMP, &[-2]),
        (op::IN, &[20, ADCL]), (op::BREAK, &[]),
    ]);
    m.set_pin_input(2, ExtDrive::Analog, 2.5);
    assert_eq!(m.run(10_000), StopReason::BreakInsn);
    assert_eq!(m.cpu.r[20], 128);
}

#[test]
fn sleep_idle_wakes_on_timer_and_sei_delays_one_instruction() {
    let mut m = tiny10(&[
        (op::RJMP, &[10]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]), (op::RJMP, &[14]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]), (op::RETI, &[]),
        (op::LDI, &[16, 1]), (op::OUT, &[TIMSK0, 16]), (op::OUT, &[TCCR0B, 16]), (op::LDI, &[16, 1]), (op::OUT, &[SMCR, 16]),
        (op::BSET, &[7]), (op::SLEEP, &[]), (op::RJMP, &[-2]),
        (op::INC, &[20]), (op::RETI, &[]),
    ]);
    m.run(65536 * 2 + 300);
    assert_eq!(m.cpu.r[20], 2);
    assert!(m.cpu.instructions < 100);
    assert_eq!(m.cpu.sreg & SREG_I, SREG_I);
}

#[test]
fn throughput() {
    let mut m = tiny10(&[
        (op::LDI, &[16, 3]), (op::OUT, &[DDRB, 16]),
        (op::LDI, &[16, 0x81]), (op::OUT, &[TCCR0A, 16]), (op::LDI, &[16, 0x09]), (op::OUT, &[TCCR0B, 16]),
        (op::LDI, &[16, 0x80]), (op::OUT, &[OCR0AL, 16]),
        (op::LDI, &[17, 0]), (op::INC, &[17]), (op::CPI, &[17, 200]), (op::BRBC, &[1, -3]), (op::SBI, &[PINB, 1]), (op::RJMP, &[-6]),
    ]);
    let cycles = 100_000_000u64;
    let t0 = std::time::Instant::now();
    m.run(cycles);
    let secs = t0.elapsed().as_secs_f64();
    let mips = m.cpu.instructions as f64 / secs / 1e6;
    println!("{:.1} MHz simulated, {:.1} MIPS", cycles as f64 / secs / 1e6, mips);
    assert!(mips > 20.0, "too slow: {mips:.1} MIPS");
}
