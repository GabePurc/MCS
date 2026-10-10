//! STM32G4 device tests: programs assembled by Apple clang (see `stm32g4/gen_programs.py`,
//! `stm32g4/programs/*.s`) run on `mcs_sim::arm::Machine::from_spec`, plus Session-level tests that
//! load a tiny linked ELF (`mcs-formats/tests/data/stm32g4_blink.elf`, built by `stm32g4/make_elf.py`).
//! Expected values follow RM0440 and the peripheral models' documented behaviour.

#[path = "stm32g4/programs.rs"]
mod programs;

use mcs_core::arm::device::ArmDeviceSpec;
use mcs_core::arm::devices;
use mcs_core::device::DeviceRef;
use mcs_core::program::{LoadedProgram, ProgramFormat};
use mcs_sim::arm::{Machine, StopReason};
use mcs_sim::avr::peripherals::serial::SerialConfig;
use mcs_sim::pins::ExtDrive;
use mcs_sim::protocol::{Command, CoreState, Output, SpeedMode, StepKind, StopKind};
use mcs_sim::session::Session;
use mcs_sim::target::{Sent, Target};
use programs::*;

const G474: &str = "stm32g474re";
const RCC: u32 = 0x4002_1000;
const GPIOA: u32 = 0x4800_0000;

fn spec(id: &str) -> &'static ArmDeviceSpec {
    devices::get(id).unwrap()
}

fn machine(id: &str, p: &Prog) -> Machine {
    let s = spec(id);
    let mut m = Machine::from_spec(s);
    let mut prog = LoadedProgram::empty(ProgramFormat::Elf, s.flash_size as usize);
    prog.flash[..p.code.len()].copy_from_slice(p.code);
    prog.flash_used = p.code.len() as u32;
    m.load_program(Some(&prog));
    m
}

fn boot(p: &Prog) -> Machine {
    machine(G474, p)
}

fn boot_at(p: &Prog, entry: &str) -> Machine {
    let mut m = boot(p);
    m.cpu.pc = p.sym(entry);
    m
}

/// Runs to the next BKPT and returns its address.
fn run_to_bkpt(m: &mut Machine) -> u32 {
    let limit = m.cpu.cycles + 100_000_000;
    assert_eq!(m.run(limit), StopReason::Bkpt, "pc={:#x} messages={:?}", m.cpu.pc, m.sys.messages);
    m.cpu.pc - 2
}

fn rd(m: &mut Machine, addr: u32) -> u32 {
    m.mem_read(addr, 4).unwrap_or_else(|| panic!("read {addr:#x}"))
}

fn wr(m: &mut Machine, addr: u32, v: u32) {
    assert!(m.mem_write(addr, 4, v), "write {addr:#x}");
}

fn set_bits(m: &mut Machine, addr: u32, bits: u32) {
    let v = rd(m, addr);
    wr(m, addr, v | bits);
}

fn clear_bits(m: &mut Machine, addr: u32, bits: u32) {
    let v = rd(m, addr);
    wr(m, addr, v & !bits);
}

/// (cycle, level) changes of GPIO `pin` recorded in the pin trace.
fn pin_edges(m: &Machine, pin: usize) -> Vec<(u64, u8)> {
    let words = m.sys.trace.words();
    let (_, cycles, levels) = m.sys.trace.read_since_wide(0, usize::MAX);
    let mut out: Vec<(u64, u8)> = Vec::new();
    for (i, &c) in cycles.iter().enumerate() {
        let lv = (levels[i * words + pin / 32] >> (pin % 32) & 1) as u8;
        if out.last().map(|e| e.1) != Some(lv) {
            out.push((c, lv));
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Devices, memory
// ---------------------------------------------------------------------------------------------

#[test]
fn reset_takes_sp_and_pc_from_the_vector_table() {
    let mut m = boot(&BOOT);
    assert_eq!(m.cpu.r[13], 0x2002_0000);
    assert_eq!(m.cpu.pc, BOOT.sym("reset"));
    assert_eq!(run_to_bkpt(&mut m), BOOT.sym("done"));
    assert_eq!(rd(&mut m, 0x2000_0100), 0xcafe_babe);
    assert!(matches!(m.device(), DeviceRef::Arm(s) if s.id == G474));
    // RCC comes out of reset on HSI16.
    assert_eq!(m.sys.clk.hclk_hz, 16e6);
}

#[test]
fn ccm_sram_is_visible_at_both_addresses() {
    for (id, alias) in [("stm32g431kb", 0x2000_5800u32), (G474, 0x2001_8000)] {
        let mut m = Machine::from_spec(spec(id));
        assert!(m.mem_write(0x1000_0000, 4, 0x1234_5678), "{id}: CCM window");
        assert_eq!(m.mem_read(alias, 4), Some(0x1234_5678), "{id}: alias after SRAM1/2");
        assert!(m.mem_write(alias + 4, 2, 0xbeef));
        assert_eq!(m.mem_read(0x1000_0004, 2), Some(0xbeef));
        let end = spec(id).sram_base + spec(id).ram_total();
        assert_eq!(m.mem_read(end, 4), None, "{id}: nothing beyond the CCM");
    }
}

#[test]
fn unmapped_peripheral_space_reads_zero_with_a_warning() {
    let mut m = Machine::from_spec(spec(G474));
    assert_eq!(m.mem_read(0x5000_0000, 4), Some(0)); // ADC1 is not modelled
    assert!(m.mem_write(0x5000_0000, 4, 1));
    assert!(m.sys.messages.iter().any(|x| x.text.contains("0x50000000")), "{:?}", m.sys.messages);
}

// ---------------------------------------------------------------------------------------------
// RCC, FLASH, PWR, SysTick
// ---------------------------------------------------------------------------------------------

#[test]
fn rcc_switches_to_the_pll_at_170_mhz_and_systick_follows() {
    let mut m = boot(&PLL);
    assert_eq!(run_to_bkpt(&mut m), PLL.sym("pll_done"));
    assert_eq!(rd(&mut m, 0x2000_0100), 0xf, "SW = SWS = PLL");
    assert_eq!(m.sys.clk.sysclk_hz, 170e6);
    assert_eq!(m.sys.clk.hclk_hz, 170e6);
    assert_eq!(m.sys.clock.hz, 170e6);
    assert!(m.sys.messages.is_empty(), "correct bring-up sequence: {:?}", m.sys.messages);
    // The cycle counter now runs at 170 MHz: 1 ms of SysTick = 170000 cycles = 1 ms of time.
    let mut stops = Vec::new();
    for _ in 0..3 {
        assert!(run_to_bkpt(&mut m) > PLL.sym("spin"), "BKPT of the SysTick handler");
        stops.push((m.cpu.cycles, m.sys.time_at(m.cpu.cycles)));
    }
    for w in stops.windows(2) {
        let (dc, dt) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
        assert!((169_997..=170_003).contains(&dc), "SysTick period {dc} cycles");
        assert!((dt - 1e-3).abs() < 2e-8, "SysTick period {dt} s");
    }
}

#[test]
fn rcc_registers_hse_pll_and_prescalers() {
    let mut m = Machine::from_spec(spec(G474));
    assert_eq!(rd(&mut m, RCC), 0x500, "HSION | HSIRDY");
    assert_eq!(rd(&mut m, RCC + 8), 0x5, "SW = SWS = HSI16");
    // HSE (8 MHz crystal) as the PLL source: 8 / 2 * 85 / 2 = 170 MHz, AHB /2, APB1 /4, APB2 /2.
    wr(&mut m, 0x4002_2000, 8 | 0x600); // FLASH_ACR latency
    set_bits(&mut m, RCC, 1 << 16);
    assert_ne!(rd(&mut m, RCC) & 1 << 17, 0, "HSERDY");
    wr(&mut m, RCC + 0x0c, 3 | 1 << 4 | 85 << 8 | 1 << 24);
    set_bits(&mut m, RCC, 1 << 24);
    assert_ne!(rd(&mut m, RCC) & 1 << 25, 0, "PLLRDY");
    // PLLCFGR is locked while the PLL runs.
    wr(&mut m, RCC + 0x0c, 0);
    assert_eq!(rd(&mut m, RCC + 0x0c), 3 | 1 << 4 | 85 << 8 | 1 << 24);
    wr(&mut m, RCC + 8, 3 | 0b1000 << 4 | 0b101 << 8 | 0b100 << 11);
    assert_eq!(rd(&mut m, RCC + 8) >> 2 & 3, 3, "SWS = PLL");
    let c = m.sys.clk;
    assert_eq!((c.sysclk_hz, c.hclk_hz, c.ppre1, c.ppre2), (170e6, 85e6, 4, 2));
    // Timers on a prescaled APB bus run at 2 x PCLK.
    assert_eq!(c.timer_div(1), 2);
    assert_eq!(c.timer_div(2), 1);
    // The PLL source cannot be switched off while in use.
    clear_bits(&mut m, RCC, 1 << 16);
    assert_ne!(rd(&mut m, RCC) & 1 << 16, 0, "HSEON stays set");
    // Boost mode and wait states were set up for 85 MHz? Latency 8 is plenty, no warning.
    assert!(m.sys.messages.is_empty(), "{:?}", m.sys.messages);
}

#[test]
fn rcc_warns_about_missing_wait_states_and_boost_mode() {
    let mut m = Machine::from_spec(spec(G474));
    wr(&mut m, RCC + 0x0c, 2 | 3 << 4 | 85 << 8 | 1 << 24);
    set_bits(&mut m, RCC, 1 << 24);
    wr(&mut m, RCC + 8, 3);
    let text: Vec<_> = m.sys.messages.iter().map(|x| x.text.as_str()).collect();
    assert!(text.iter().any(|t| t.contains("LATENCY")), "{text:?}");
    assert!(text.iter().any(|t| t.contains("boost")), "{text:?}");
}

#[test]
fn peripheral_clock_gating_and_reset_through_rcc() {
    let mut m = Machine::from_spec(spec(G474));
    assert_eq!(rd(&mut m, GPIOA), 0, "GPIOA clock off: reads 0");
    wr(&mut m, GPIOA, 0);
    wr(&mut m, RCC + 0x4c, 1);
    assert_eq!(rd(&mut m, GPIOA), 0xABFF_FFFF, "MODER reset value (SWD pins in AF)");
    assert_eq!(rd(&mut m, GPIOA + 0x0c), 0x6400_0000, "PUPDR reset value");
    wr(&mut m, GPIOA, 0);
    assert_eq!(rd(&mut m, GPIOA), 0);
    wr(&mut m, RCC + 0x2c, 1); // AHB2RSTR: reset GPIOA
    assert_eq!(rd(&mut m, GPIOA), 0xABFF_FFFF);
}

#[test]
fn flash_latency_and_unlock_sequence() {
    let mut m = Machine::from_spec(spec(G474));
    let flash = 0x4002_2000;
    assert_eq!(rd(&mut m, flash) & 0xf, 1, "reset latency");
    wr(&mut m, flash, 0x0004_0608);
    assert_eq!(rd(&mut m, flash) & 0xf, 8);
    assert_ne!(rd(&mut m, flash + 0x14) & 1 << 31, 0, "locked after reset");
    wr(&mut m, flash + 8, 0x4567_0123);
    wr(&mut m, flash + 8, 0xCDEF_89AB);
    assert_eq!(rd(&mut m, flash + 0x14) & 1 << 31, 0, "unlocked");
}

// ---------------------------------------------------------------------------------------------
// GPIO, EXTI
// ---------------------------------------------------------------------------------------------

#[test]
fn gpio_output_drives_the_pin_and_trace() {
    let mut m = boot(&GPIO);
    assert_eq!(run_to_bkpt(&mut m), GPIO.sym("out_done"));
    // IDR reads the pad: PA5 plus the reset pull-ups of PA13 / PA15.
    assert_eq!(rd(&mut m, 0x2000_0100), 0xA020);
    assert_eq!(rd(&mut m, 0x2000_0104), 0xA000);
    assert_eq!(rd(&mut m, 0x2000_0108), 0, "ODR");
    let edges: Vec<u8> = pin_edges(&m, 5).iter().map(|e| e.1).collect();
    assert_eq!(edges, [0, 1, 0, 1, 0], "BSRR set/reset, ODR high/low");
    let t: Vec<u64> = pin_edges(&m, 5).iter().map(|e| e.0).collect();
    assert!(t.windows(2).all(|w| w[1] > w[0]));
    assert_eq!(m.sys.pins[5].dir, 1);
}

#[test]
fn gpio_input_with_pull_up_follows_the_outside_world() {
    let mut m = boot_at(&GPIO, "gpio_in");
    assert_eq!(run_to_bkpt(&mut m), GPIO.sym("in_a"));
    assert_eq!(rd(&mut m, 0x2000_0100) & 1, 1, "floating input with pull-up reads high");
    m.set_pin_input(0, ExtDrive::Low, 0.0);
    assert_eq!(run_to_bkpt(&mut m), GPIO.sym("in_b"));
    assert_eq!(rd(&mut m, 0x2000_0104) & 1, 0, "driven low from outside");
    m.set_pin_input(0, ExtDrive::High, 3.3);
    assert_eq!(run_to_bkpt(&mut m), GPIO.sym("in_c"));
    assert_eq!(rd(&mut m, 0x2000_0108) & 1, 1);
}

#[test]
fn gpio_pull_down_open_drain_and_lock() {
    let mut m = Machine::from_spec(spec(G474));
    wr(&mut m, RCC + 0x4c, 3); // GPIOA, GPIOB
    let b = GPIOA + 0x400;
    // PB8: input with pull-down reads low, pull-up reads high.
    wr(&mut m, b, !(3u32 << 16));
    wr(&mut m, b + 0x0c, 2 << 16);
    assert_eq!(rd(&mut m, b + 0x10) >> 8 & 1, 0);
    wr(&mut m, b + 0x0c, 1 << 16);
    assert_eq!(rd(&mut m, b + 0x10) >> 8 & 1, 1);
    // Open-drain output: ODR = 1 releases the pin (pull-up -> high), ODR = 0 pulls low.
    wr(&mut m, b, !(3u32 << 16) | 1 << 16);
    wr(&mut m, b + 4, 1 << 8);
    wr(&mut m, b + 0x14, 1 << 8);
    assert_eq!(rd(&mut m, b + 0x10) >> 8 & 1, 1, "released, pulled up");
    assert!(m.sys.pins[16 + 8].effective_dir() == 0);
    wr(&mut m, b + 0x14, 0);
    assert_eq!(rd(&mut m, b + 0x10) >> 8 & 1, 0, "driven low");
    // Lock PB8 with the key sequence; MODER can no longer change.
    wr(&mut m, b + 0x1c, 1 << 16 | 1 << 8);
    wr(&mut m, b + 0x1c, 1 << 8);
    wr(&mut m, b + 0x1c, 1 << 16 | 1 << 8);
    assert_ne!(rd(&mut m, b + 0x1c) & 1 << 16, 0, "LCKK");
    wr(&mut m, b, 0xFFFF_FFFF);
    assert_eq!(rd(&mut m, b) >> 16 & 3, 1, "PB8 mode unchanged");
    assert_eq!(rd(&mut m, b) >> 18 & 3, 3, "PB9 mode changed");
}

#[test]
fn exti_rising_edge_interrupt() {
    let mut m = boot(&EXTI);
    assert_eq!(m.run(m.cpu.cycles + 20_000), StopReason::Limit);
    assert!(m.cpu.sleeping, "WFI loop");
    let count = |m: &mut Machine| rd(m, 0x2000_0200);
    assert_eq!(count(&mut m), 0);
    m.set_pin_input(0, ExtDrive::High, 3.3);
    m.run(m.cpu.cycles + 1_000);
    assert_eq!(count(&mut m), 1, "rising edge");
    assert_eq!(rd(&mut m, 0x4001_0414), 0, "PR1 cleared by the handler");
    m.set_pin_input(0, ExtDrive::Low, 0.0);
    m.run(m.cpu.cycles + 1_000);
    assert_eq!(count(&mut m), 1, "falling edge is not enabled");
    m.set_pin_input(0, ExtDrive::High, 3.3);
    m.run(m.cpu.cycles + 1_000);
    assert_eq!(count(&mut m), 2);
    // Software interrupt request.
    wr(&mut m, 0x4001_0410, 1);
    m.run(m.cpu.cycles + 1_000);
    assert_eq!(count(&mut m), 3);
}

// ---------------------------------------------------------------------------------------------
// USART
// ---------------------------------------------------------------------------------------------

#[test]
fn usart1_transmits_on_the_pin_with_baud_timing_and_reaches_the_serial_monitor() {
    let mut m = boot_at(&USART, "tx_main");
    m.set_serial(SerialConfig { monitor: Some(9), inject: None, baud: 115_200.0, data_bits: 8, parity: 0, stop_bits: 1 });
    assert_eq!(run_to_bkpt(&mut m), USART.sym("tx_sent"));
    assert!(m.sys.serial_out.is_empty(), "no frame is complete yet");
    assert_eq!(run_to_bkpt(&mut m), USART.sym("tx_done"));
    assert_eq!(m.sys.serial_out, b"Hi");
    // Expected waveform: two back-to-back 8N1 frames, BRR = 139 -> 139 cycles per bit.
    let mut bits = Vec::new();
    for b in *b"Hi" {
        bits.push(0u8);
        bits.extend((0..8).map(|k| b >> k & 1));
        bits.push(1);
    }
    let edges = pin_edges(&m, 9);
    let t0 = edges.iter().find(|e| e.1 == 0 && e.0 > 0).expect("start bit").0;
    let mut want = vec![(t0, 0u8)];
    let mut level = 0;
    for (k, &b) in bits.iter().enumerate() {
        if b != level {
            want.push((t0 + 139 * k as u64, b));
            level = b;
        }
    }
    let got: Vec<(u64, u8)> = edges.into_iter().filter(|e| e.0 >= t0).collect();
    assert_eq!(got, want);
    // TC fired at the end of the last stop bit.
    assert!(m.cpu.cycles >= t0 + 139 * 20, "tx_done at {} (frame end {})", m.cpu.cycles, t0 + 139 * 20);
}

#[test]
fn usart1_receives_bytes_from_the_serial_monitor() {
    let mut m = boot_at(&USART, "rx_main");
    m.set_serial(SerialConfig { monitor: None, inject: Some(10), baud: 115_200.0, data_bits: 8, parity: 0, stop_bits: 1 });
    assert_eq!(run_to_bkpt(&mut m), USART.sym("rx_ready"));
    m.serial_send(b"AB");
    m.run(m.cpu.cycles + 2 * 1390 + 400);
    assert_eq!(m.mem_read(0x2000_0100, 1), Some(b'A' as u32));
    assert_eq!(m.mem_read(0x2000_0101, 1), Some(b'B' as u32));
    assert_eq!(m.mem_read(0x2000_0102, 1), Some(0));
}

#[test]
fn usart_overrun_and_flags() {
    let mut m = boot(&IDLE);
    wr(&mut m, RCC + 0x4c, 1);
    wr(&mut m, RCC + 0x60, 1 << 14);
    let u = 0x4001_3800;
    wr(&mut m, GPIOA, 0xABEF_FFFF); // PA10 AF
    wr(&mut m, GPIOA + 0x24, 7 << 8);
    wr(&mut m, u + 0x0c, 139);
    wr(&mut m, u, 5); // UE | RE
    m.set_serial(SerialConfig { monitor: None, inject: Some(10), baud: 115_200.0, data_bits: 8, parity: 0, stop_bits: 1 });
    m.serial_send(b"abc");
    m.run(m.cpu.cycles + 3 * 1390 + 400);
    let isr = rd(&mut m, u + 0x1c);
    assert_ne!(isr & 1 << 5, 0, "RXNE");
    assert_ne!(isr & 1 << 3, 0, "ORE: 'b' arrived while 'a' was unread");
    assert_eq!(rd(&mut m, u + 0x24), b'a' as u32);
    assert_eq!(rd(&mut m, u + 0x1c) & 1 << 5, 0, "RXNE cleared by reading RDR");
    wr(&mut m, u + 0x20, 1 << 3); // ICR.ORECF
    assert_eq!(rd(&mut m, u + 0x1c) & 1 << 3, 0);
}

// ---------------------------------------------------------------------------------------------
// Timers
// ---------------------------------------------------------------------------------------------

#[test]
fn tim2_update_interrupt_period() {
    let mut m = boot(&TIM);
    let mut stops = Vec::new();
    for _ in 0..4 {
        let at = run_to_bkpt(&mut m);
        assert!(at > TIM.sym("tim2_run"));
        stops.push(m.cpu.cycles);
    }
    // PSC = 15, ARR = 999 on a 16 MHz timer clock: exactly 16000 cycles between updates.
    for w in stops.windows(2) {
        assert_eq!(w[1] - w[0], 16_000);
    }
    // The handler runs in IRQ 28 = exception 44 and the call stack shows it.
    let frames = m.call_stack();
    assert!(frames.iter().any(|f| f.vector == 44), "{frames:?}");
}

#[test]
fn tim3_pwm_duty_on_the_pin() {
    let mut m = boot_at(&TIM, "tim3_pwm");
    m.run(m.cpu.cycles + 12_000);
    let edges = pin_edges(&m, 6);
    // The first pulse is long: OC1REF is already high when CC1E is set, before the counter starts.
    let first_high = edges.iter().position(|e| e.1 == 1).expect("PWM rising edge");
    let e = &edges[first_high + 2..];
    assert!(e.len() >= 20, "{} edges", e.len());
    for pair in e.windows(2) {
        let d = pair[1].0 - pair[0].0;
        assert_eq!(d, if pair[0].1 == 1 { 250 } else { 750 }, "duty cycle 25 %: {pair:?}");
    }
    // Changing CCR1 updates the compare (OC1PE: at the next update event).
    wr(&mut m, 0x4000_0434, 500);
    m.run(m.cpu.cycles + 3_000);
    let edges = pin_edges(&m, 6);
    let last = edges.len();
    let tail: Vec<_> = edges[last - 4..].windows(2).map(|w| w[1].0 - w[0].0).collect();
    assert!(tail.contains(&500), "{tail:?}");
}

#[test]
fn timer_registers_and_shadowing() {
    let mut m = Machine::from_spec(spec(G474));
    wr(&mut m, RCC + 0x58, 0b0111); // TIM2-4
    assert_eq!(rd(&mut m, 0x4000_002c), 0xffff_ffff, "TIM2 is 32-bit");
    assert_eq!(rd(&mut m, 0x4000_042c), 0xffff, "TIM3 is 16-bit");
    wr(&mut m, 0x4000_0400 + 0x2c, 99);
    wr(&mut m, 0x4000_0400 + 0x24, 0xffff_1234);
    assert_eq!(rd(&mut m, 0x4000_0424), 0x1234, "CNT is 16-bit");
    // TIM6 (basic timer) has no capture/compare registers.
    wr(&mut m, RCC + 0x58, 0b1_0000);
    wr(&mut m, 0x4000_1000 + 0x34, 5);
    assert_eq!(rd(&mut m, 0x4000_1000 + 0x34), 0);
    // Without the clock registers read 0.
    wr(&mut m, RCC + 0x58, 0);
    assert_eq!(rd(&mut m, 0x4000_042c), 0);
}

// ---------------------------------------------------------------------------------------------
// Target / Session
// ---------------------------------------------------------------------------------------------

const BLINK_ELF: &[u8] = include_bytes!("../../mcs-formats/tests/data/stm32g4_blink.elf");

fn blink_program() -> LoadedProgram {
    mcs_formats::load_program_file_at(BLINK_ELF, "stm32g4_blink.elf", 512 * 1024, 0x0800_0000)
}

fn states(outs: &[Output]) -> Vec<&mcs_sim::protocol::MachineState> {
    outs.iter().filter_map(|o| if let Output::State { state } = o { Some(&**state) } else { None }).collect()
}

fn sym(p: &LoadedProgram, name: &str) -> u32 {
    p.symbols.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("symbol {name}")).address
}

#[test]
fn session_runs_an_elf_on_the_stm32g474() {
    let p = blink_program();
    assert!(!p.has_errors(), "{:?}", p.diagnostics);
    let mut s = Session::new();
    let outs = s.handle(Command::Init { device_id: G474.into() });
    assert!(outs.iter().any(|o| matches!(o, Output::Device { spec: DeviceRef::Arm(d) } if d.id == G474)));
    let reset = sym(&p, "reset");
    let outs = s.handle(Command::Load { device_id: G474.into(), program: Box::new(p.clone()) });
    let st = states(&outs).pop().unwrap();
    assert_eq!(st.pc, reset);
    assert_eq!(st.pc_bytes, reset as u64);
    assert!(matches!(st.core, CoreState::Arm { .. }));
    s.handle(Command::SetSpeed { mode: SpeedMode::Max, factor: 1.0 });
    let mut edges = 0;
    let mut outs = s.handle(Command::Run);
    for _ in 0..5 {
        outs.extend(s.slice());
    }
    outs.extend(s.handle(Command::Pause));
    for st in states(&outs) {
        edges += st.trace_cycles.len();
    }
    let outs = s.handle(Command::RequestState);
    let st = states(&outs).pop().unwrap();
    let CoreState::Arm { r, xpsr, msp, .. } = &st.core else { panic!("ARM core state") };
    assert_eq!(msp, &0x2002_0000, "initial stack pointer from the vector table");
    assert_ne!(xpsr & (1 << 24), 0, "Thumb bit");
    assert_eq!(r[15], st.pc);
    assert!(st.cycles > 1000 && st.instructions > 100);
    assert_eq!(st.hz, 16e6);
    assert_eq!(st.pins.len(), 112);
    assert_eq!(st.io.len(), spec(G474).registers.len());
    assert!(edges >= 4, "PA5 toggles ({edges} trace entries)");
    // Memory travels once (128 KiB incl. CCM), then only when it changes.
    let outs = s.handle(Command::RequestState);
    assert!(states(&outs).pop().unwrap().data.len() <= 128 * 1024);
    // GPIOA is clocked and PA5 is an output.
    let idx = spec(G474).registers.iter().position(|r| r.name == "GPIOA_MODER").unwrap();
    assert_eq!(st.io[idx] & (3 << 10), 1 << 10);
}

#[test]
fn session_breakpoints_stepping_and_call_stack() {
    let p = blink_program();
    let (blink_loop, delay) = (sym(&p, "blink_loop"), sym(&p, "delay"));
    let mut s = Session::new();
    s.handle(Command::Load { device_id: G474.into(), program: Box::new(p.clone()) });
    s.handle(Command::SetSpeed { mode: SpeedMode::Max, factor: 1.0 });
    s.handle(Command::SetBreakpoints { pcs: vec![delay] });
    s.handle(Command::Run);
    let mut stop = None;
    for _ in 0..20 {
        for o in s.slice() {
            if let Output::State { state } = o {
                if state.stop.is_some() {
                    stop = state.stop.clone();
                }
            }
        }
        if stop.is_some() {
            break;
        }
    }
    let stop = stop.expect("breakpoint hit");
    assert_eq!((stop.reason, stop.pc), (StopKind::Breakpoint, delay));
    // The call stack shows the caller of `delay`.
    let outs = s.handle(Command::RequestState);
    let st = states(&outs).pop().unwrap();
    let f = st.call_stack.last().expect("one frame");
    assert_eq!(f.target_pc, delay);
    assert!(f.return_pc > blink_loop && f.return_pc < blink_loop + 0x20, "return into blink_loop: {:#x}", f.return_pc);
    assert_eq!(f.vector, -1);
    // Step out returns to the caller.
    s.handle(Command::SetBreakpoints { pcs: vec![] });
    let ret = f.return_pc;
    s.handle(Command::Step { kind: StepKind::Out, source: false });
    let mut stopped = None;
    for _ in 0..20 {
        for o in s.slice() {
            if let Output::State { state } = o {
                if state.stop.is_some() {
                    stopped = state.stop.clone();
                }
            }
        }
        if stopped.is_some() {
            break;
        }
    }
    assert_eq!(stopped.expect("step out").pc, ret);
}

#[test]
fn step_over_a_call_and_source_level_steps() {
    let p = blink_program();
    let mut m = Machine::from_spec(spec(G474));
    m.load_program(Some(&p));
    m.set_source_map(Some(&p));
    let blink_loop = sym(&p, "blink_loop");
    let delay = sym(&p, "delay");
    // Run to the loop start through a run-to.
    m.run_to(blink_loop);
    assert_eq!(m.run_until(m.cycles() + 1_000_000), mcs_sim::target::StopReason::Requested);
    assert_eq!(m.pc(), blink_loop);
    m.clear_stop_condition();
    // Instruction steps: movs, str, then the BL.
    for _ in 0..2 {
        assert_eq!(m.begin_step(StepKind::Into, false), mcs_sim::target::StepPlan::Single);
        m.step_one();
    }
    let bl_at = m.pc();
    assert_eq!(bl_at, blink_loop + 4);
    // Step over: stops after the call returns, the delay loop (100 x 2 instructions) has run.
    assert_eq!(m.begin_step(StepKind::Over, false), mcs_sim::target::StepPlan::Run);
    let c0 = m.cycles();
    assert_eq!(m.run_until(c0 + 1_000_000), mcs_sim::target::StopReason::Requested);
    assert_eq!(m.pc(), bl_at + 4, "after the BL");
    assert!(m.cycles() - c0 > 300, "the callee ran ({} cycles)", m.cycles() - c0);
    m.clear_stop_condition();
    // Not a call: Over is a single step.
    assert_eq!(m.begin_step(StepKind::Over, false), mcs_sim::target::StepPlan::Single);
    // Step Out of the callee: stops right after the BL.
    m.run_to(delay);
    m.run_until(m.cycles() + 10_000_000);
    m.clear_stop_condition();
    assert_eq!(m.pc(), delay);
    assert_eq!(m.begin_step(StepKind::Out, false), mcs_sim::target::StepPlan::Run);
    m.run_until(m.cycles() + 10_000_000);
    assert!(m.pc() > blink_loop && m.pc() < blink_loop + 0x30);
    // Source level: the line table maps the loop's statements.
    assert!(!p.lines.is_empty(), "DWARF line table loaded");
    m.clear_stop_condition();
    m.run_to(blink_loop);
    m.run_until(m.cycles() + 10_000_000);
    m.clear_stop_condition();
    let before = m.pc();
    assert_eq!(m.begin_step(StepKind::Over, true), mcs_sim::target::StepPlan::Run);
    m.run_until(m.cycles() + 10_000_000);
    assert!(m.pc() > before && m.pc() <= before + 8, "next source line: {:#x} -> {:#x}", before, m.pc());
}

#[test]
fn snapshot_reports_registers_and_pins() {
    let mut m = Machine::from_spec(spec(G474));
    let mut sent = Sent { trace: 0, eeprom: u64::MAX };
    let st = m.snapshot(&mut sent, true, 100);
    assert_eq!(st.data.len(), 128 * 1024);
    assert_eq!(st.flash.as_ref().map(|f| f.len()), Some(512 * 1024));
    let at = |name: &str| spec(G474).registers.iter().position(|r| r.name == name).unwrap_or_else(|| panic!("{name}"));
    assert_eq!(st.io[at("RCC_CR")], 0x500);
    assert_eq!(st.io[at("RCC_CFGR")], 5);
    assert_eq!(st.io[at("GPIOA_MODER")], 0xABFF_FFFF);
    assert_eq!(st.io[at("SCB_CPUID")], 0x410F_C241);
    assert_eq!(st.io[at("TIM2_ARR")], 0xffff_ffff);
    let st2 = m.snapshot(&mut sent, false, 100);
    assert!(st2.data.is_empty() && st2.flash.is_none(), "unchanged memory is not resent");
    m.mem_write(0x2000_0000, 4, 1);
    assert_eq!(m.snapshot(&mut sent, false, 100).data.len(), 128 * 1024);
    assert!(st.peripherals.iter().any(|p| p.name == "RCC"));
}

#[test]
fn g431kb_reset_and_memories() {
    let s = spec("stm32g431kb");
    assert_eq!((s.flash_size, s.sram_size, s.ccm_sram.unwrap().size, s.package.as_str()), (128 * 1024, 22 * 1024, 10 * 1024, "LQFP32"));
    let mut m = Machine::from_spec(s);
    assert_eq!(m.snapshot(&mut Sent { trace: 0, eeprom: u64::MAX }, false, 10).data.len(), 32 * 1024);
}

#[test]
fn power_cycle_and_debugger_reset() {
    let mut m = boot(&BOOT);
    run_to_bkpt(&mut m);
    let c = m.cpu.cycles;
    m.debugger_reset();
    assert_eq!(m.cpu.pc, BOOT.sym("reset"));
    assert!(m.cpu.cycles == c, "debugger reset keeps time");
    m.power_cycle();
    assert_eq!(m.cpu.cycles, 0);
    assert!(pin_edges(&m, 14).iter().all(|e| e.0 == 0) && pin_edges(&m, 15).iter().all(|e| e.0 == 0), "trace restarted at cycle 0");
}
