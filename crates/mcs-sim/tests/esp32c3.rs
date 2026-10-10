//! ESP32-C3 device tests: linked ELF programs (tests/esp32c3/programs, built by tests/esp32c3/gen_programs.py with rustc
//! for riscv32imc and checked in as tests/esp32c3/elf/*.elf) run through the `Target` API.

use mcs_core::program::LoadedProgram;
use mcs_core::riscv::devices;
use mcs_sim::pins::ExtDrive;
use mcs_sim::protocol::{CoreState, MachineState, StepKind};
use mcs_sim::riscv::Esp32c3;
use mcs_sim::target::{Sent, StepPlan, StopReason, Target};

const CALLS: &[u8] = include_bytes!("esp32c3/elf/calls.elf");
const CLOCK: &[u8] = include_bytes!("esp32c3/elf/clock.elf");
const CSR: &[u8] = include_bytes!("esp32c3/elf/csr.elf");
const DATA: &[u8] = include_bytes!("esp32c3/elf/data.elf");
const GPIO: &[u8] = include_bytes!("esp32c3/elf/gpio.elf");
const GPIO_IRQ: &[u8] = include_bytes!("esp32c3/elf/gpio_irq.elf");
const INTC: &[u8] = include_bytes!("esp32c3/elf/intc.elf");
const IRQ_SYSTIMER: &[u8] = include_bytes!("esp32c3/elf/irq_systimer.elf");
const ROM: &[u8] = include_bytes!("esp32c3/elf/rom.elf");
const TIMG: &[u8] = include_bytes!("esp32c3/elf/timg.elf");
const UART: &[u8] = include_bytes!("esp32c3/elf/uart.elf");
const UART1: &[u8] = include_bytes!("esp32c3/elf/uart1.elf");
const USB: &[u8] = include_bytes!("esp32c3/elf/usb.elf");

const RES: u32 = 0x3fc8_0100;
const IROM: u32 = 0x4200_0000;

struct Bench {
    m: Esp32c3,
    prog: LoadedProgram,
    sent: Sent,
    serial: Vec<u8>,
}

fn bench(elf: &[u8]) -> Bench {
    let spec = devices::get("esp32-c3").unwrap();
    let prog = mcs_formats::parse_elf_at(elf, spec.flash_size as usize, "t.elf", Some(IROM));
    assert!(!prog.has_errors(), "{:?}", prog.diagnostics);
    let mut m = Esp32c3::from_spec(spec);
    m.load_program(Some(&prog));
    m.set_source_map(Some(&prog));
    Bench { m, prog, sent: Sent { trace: 0, eeprom: u64::MAX }, serial: Vec::new() }
}

impl Bench {
    fn sym(&self, name: &str) -> u32 {
        self.prog.symbols.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("symbol {name}")).address
    }

    fn word(&self, addr: u32) -> u32 {
        self.m.machine.bus.peek(addr, 4).unwrap_or_else(|| panic!("peek {addr:#x}"))
    }

    fn results(&self, n: usize) -> Vec<u32> {
        (0..n as u32).map(|i| self.word(RES + 4 * i)).collect()
    }

    /// Runs for `cycles` more cycles (or until a stop).
    fn run(&mut self, cycles: u64) -> StopReason {
        let end = self.m.cycles() + cycles;
        self.m.run_until(end)
    }

    /// Runs until a stop other than the cycle limit (at most `cycles`).
    fn run_stop(&mut self, cycles: u64) -> StopReason {
        let r = self.run(cycles);
        assert_ne!(r, StopReason::Limit, "program did not stop within {cycles} cycles (pc {:#x})", self.m.pc());
        r
    }

    fn snap(&mut self) -> MachineState {
        let mut sent = Sent { trace: 0, eeprom: self.sent.eeprom };
        let s = self.m.snapshot(&mut sent, false, 1 << 20);
        self.sent.eeprom = sent.eeprom;
        self.serial.extend_from_slice(&s.serial);
        s
    }

    /// Cycles of the edges of GPIO `pin` in the pin trace: (cycle, level).
    fn edges(&mut self, pin: usize) -> Vec<(u64, u8)> {
        let s = self.snap();
        let w = s.trace_words as usize;
        let mut out = Vec::new();
        let mut last = None;
        for (i, &c) in s.trace_cycles.iter().enumerate() {
            let lv = (s.trace_levels[i * w + pin / 32] >> (pin % 32) & 1) as u8;
            if last != Some(lv) {
                out.push((c, lv));
                last = Some(lv);
            }
        }
        out
    }

    fn reg(&mut self, addr: u32) -> u32 {
        let now = self.m.cycles();
        self.m.machine.bus.peek_register(addr, now).unwrap_or_else(|| panic!("register {addr:#x}"))
    }
}

#[test]
fn elf_loads_into_the_flash_windows_and_runs() {
    let mut b = bench(CALLS);
    assert_eq!(b.prog.entry, IROM);
    assert_eq!(b.prog.flash_base, IROM);
    assert!(b.prog.segments.iter().any(|s| s.address == IROM));
    assert_eq!(b.m.pc(), IROM);
    // The program's code is visible through IROM; DROM is empty for this program.
    assert_eq!(b.word(IROM), 0x3fce_0137); // lui sp, 0x3fce0
    assert_eq!(b.run(20_000), StopReason::Limit);
    // v = middle(v) with middle(x) = (3x + 1) ^ (3(3x + 1) + 1)
    let mut v = 1u32;
    let mut seen = vec![0u32];
    for _ in 0..4000 {
        let a = v.wrapping_mul(3).wrapping_add(1);
        let c = a.wrapping_mul(3).wrapping_add(1);
        v = a ^ c;
        seen.push(v);
    }
    assert!(b.word(RES) != 0 && seen.contains(&b.word(RES)), "result {:#x}", b.word(RES));
    assert!(b.snap().instructions > 1000);
    let s = b.snap();
    match s.core {
        CoreState::Riscv { x, pc, .. } => {
            assert_eq!(x[0], 0);
            assert!((IROM..IROM + 0x100).contains(&pc));
            assert!((0x3fcd_0000..=0x3fce_0000).contains(&x[2]), "sp stays below the stack top: {:#x}", x[2]);
        }
        c => panic!("{c:?}"),
    }
}

#[test]
fn clock_switch_changes_timing() {
    let mut b = bench(CLOCK);
    assert_eq!(b.run_stop(2_000_000), StopReason::BreakInsn);
    let r = b.results(2);
    // 1000 iterations x 4 cycles: 200 us = 3200 ticks at 20 MHz, 25 us = 400 ticks at 160 MHz (+ a few for the reads).
    assert!((3200..3215).contains(&r[0]), "{r:?}");
    assert!((400..412).contains(&r[1]), "{r:?}");
    let s = b.snap();
    assert_eq!(s.hz, 160e6);
    let sys = &b.m.machine.bus.cx.sys;
    assert_eq!((sys.clk.cpu.as_f64(), sys.clk.apb.as_f64()), (160e6, 80e6));
    // The GPIO2 pulses around the two phases have the same width in cycles (same loop) ...
    let e = b.edges(2);
    let hi: Vec<(u64, u8)> = e[e.len() - 4..].to_vec();
    assert_eq!(hi.iter().map(|x| x.1).collect::<Vec<_>>(), [1, 0, 1, 0], "{hi:?}");
    let (w1, w2) = (hi[1].0 - hi[0].0, hi[3].0 - hi[2].0);
    assert!((4000..4100).contains(&w1) && (4000..4100).contains(&w2), "{hi:?}");
    // ... but the simulated time axis follows the clock change: ~4040 cycles at 20 MHz (202 us) + ~4040 at 160 MHz (25 us).
    let t = b.m.elapsed_seconds();
    assert!((225e-6..231e-6).contains(&t), "{t}");
}

#[test]
fn gpio_blink_input_readback_and_pull_up() {
    let mut b = bench(GPIO);
    assert_eq!(b.run_stop(200_000), StopReason::BreakInsn);
    let r = b.results(9);
    // Outputs GPIO2 / GPIO3 read back high through the pad (input enabled at reset), all other pads float high
    // through their pull-ups.
    assert!(r[..6].iter().all(|v| v & 0xc == 0xc), "{r:x?}");
    assert_eq!((r[6] & 0xc, r[6] & 0x20), (0, 0x20), "{r:x?}");
    assert_eq!((r[7], r[8]), (0, 0xc));
    // Six periods on GPIO2: 12 edges (after the initial pull-up/low settle), constant spacing.
    let e = b.edges(2);
    let rises: Vec<u64> = e.iter().filter(|x| x.1 == 1 && x.0 > 0).map(|x| x.0).collect();
    assert_eq!(rises.len(), 6, "{e:?}");
    let d: Vec<u64> = rises.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(d.iter().all(|&x| x == d[0]), "{d:?}");
    assert!((4000..4040).contains(&d[0]), "{d:?}");
    // Host-driven input: GPIO5 pulled low from outside reads 0, released it floats high again.
    assert_eq!(b.reg(0x6000_403c) >> 5 & 1, 1);
    b.m.set_pin_input(5, ExtDrive::Low, 0.0);
    assert_eq!(b.reg(0x6000_403c) >> 5 & 1, 0);
    b.m.set_pin_input(5, ExtDrive::Float, 0.0);
    assert_eq!(b.reg(0x6000_403c) >> 5 & 1, 1);
    // The pad state reaches the pin model the UI draws.
    let s = b.snap();
    assert_eq!((s.pins[2].dir, s.pins[2].level), (1, 0));
    assert_eq!(s.pins[5].dir, 0);
}

#[test]
fn systimer_alarm_interrupt_through_the_matrix_into_a_vectored_handler() {
    let mut b = bench(IRQ_SYSTIMER);
    assert_eq!(b.run_stop(2_000_000), StopReason::BreakInsn);
    // [cause0, tick0, start, cause1, tick1, cause2, tick2, cause3, tick3]
    let r = b.results(9);
    let cause = 0x8000_000a;
    assert_eq!((r[0], r[3], r[5], r[7]), (cause, cause, cause, cause), "{r:x?}");
    // One-shot: fires 800 ticks after the start (handler entry latency adds a few ticks).
    let d0 = r[1].wrapping_sub(r[2]);
    assert!((800..840).contains(&d0), "{d0}");
    // Periodic: 160 ticks apart.
    let (d1, d2) = (r[6] - r[4], r[8] - r[6]);
    assert!((159..=161).contains(&d1) && (159..=161).contains(&d2), "{r:?}");
    // The core ends in the wfi / interrupt state of the program: MIE restored by mret.
    match b.snap().core {
        CoreState::Riscv { mcause, mtvec, .. } => assert_eq!((mcause, mtvec & 1), (cause, 1)),
        c => panic!("{c:?}"),
    }
}

#[test]
fn timg_t0_alarm_clears_alarm_enable_and_sets_the_raw_status() {
    let mut b = bench(TIMG);
    assert_eq!(b.run_stop(2_000_000), StopReason::BreakInsn);
    let r = b.results(3);
    assert!((500..506).contains(&r[0]), "{r:?}");
    assert_eq!(r[1] >> 10 & 1, 0, "ALARM_EN is cleared by the alarm");
    assert_eq!(r[1] >> 31, 1);
    assert_eq!(r[2], 0);
    // 500 us at the reset clock (20 MHz): ~10000 cycles.
    assert!((10_000..10_400).contains(&b.m.cycles()), "{}", b.m.cycles());
}

#[test]
fn uart0_tx_on_gpio21_and_the_serial_monitor_in_both_directions() {
    let mut b = bench(UART);
    // 3 characters at 115200 baud, 10 bits each: ~261 us = 5220 cycles of 20 MHz.
    b.run(12_000);
    b.snap();
    assert_eq!(b.serial, b"Hi\n");
    // The frames are on the pin: GPIO21 idles high and has start-bit falling edges.
    let falls = b.edges(21).iter().filter(|e| e.1 == 0).count();
    assert!(falls >= 3, "{falls}");
    // Typed bytes reach UART0 RX on GPIO20; the program echoes byte + 1.
    b.m.serial_send(b"A");
    b.run(12_000);
    b.snap();
    assert_eq!(b.serial, b"Hi\nB");
    let r = b.results(2);
    assert_eq!(r[1], b'A' as u32, "{r:x?}");
    assert_eq!(r[0] & 0x03ff_03ff, 0, "FIFOs empty after TX_DONE: {r:x?}");
    // Framing is reported through the interrupt status of the real peripheral registers (no errors here).
    assert_eq!(b.reg(0x6000_0004) & 0x1c, 0);
}

#[test]
fn usb_serial_jtag_output_reads_flash_through_the_drom_window() {
    let mut b = bench(USB);
    assert_eq!(b.run_stop(2_000_000), StopReason::BreakInsn);
    b.snap();
    assert_eq!(b.serial, b"Hello C3\n");
    assert_eq!(b.results(1)[0], 2);
    // .rodata lives at 0x3C01_0000, behind the (small) code: the DROM window starts 64 KiB into the flash image.
    assert_eq!(b.word(0x3c01_0000), u32::from_le_bytes(*b"Hell"));
    assert_eq!(b.m.flash_offset(0x3c01_0000), Some(0x10000 + 0x10000));
}

#[test]
fn gpio_interrupt_edges_reach_a_direct_mode_handler() {
    let mut b = bench(GPIO_IRQ);
    assert_eq!(b.run(2_000), StopReason::Limit);
    assert_eq!(b.word(RES - 4), 0);
    b.m.set_pin_input(4, ExtDrive::Low, 0.0);
    b.run(2_000);
    b.m.set_pin_input(4, ExtDrive::Float, 0.0);
    b.run(2_000);
    // cause, status, cause, status; the count word is just below the results area.
    assert_eq!(b.word(RES - 4), 2);
    assert_eq!(b.results(4), vec![0x8000_000c, 1 << 4, 0x8000_000c, 1 << 4]);
    // A level that does not change the pad raises nothing more.
    b.run(2_000);
    assert_eq!(b.word(RES - 4), 2);
}

#[test]
fn interrupt_controller_priority_threshold_and_edge_latching() {
    let mut b = bench(INTC);
    assert_eq!(b.run_stop(2_000_000), StopReason::BreakInsn);
    let r = b.results(8);
    assert_eq!(r, vec![0x8000_0009, 0x8000_0005, 0x8000_0009, 0x20, 0x8000_0005, 0x800, 0x8000_000b, 0], "{r:x?}");
}

#[test]
fn calling_into_the_boot_rom_stops_with_a_message() {
    let mut b = bench(ROM);
    assert_eq!(b.run_stop(100_000), StopReason::RomCall);
    assert_eq!(b.m.pc(), 0x4000_0100);
    let s = b.snap();
    assert!(s.messages.iter().any(|m| m.text.contains("boot ROM") && m.text.contains("0x40000100")), "{:?}", s.messages);
    match s.core {
        CoreState::Riscv { x, .. } => assert!(x[1] > IROM && x[1] < IROM + 0x200, "ra {:#x}", x[1]),
        c => panic!("{c:?}"),
    }
    // Resetting clears the condition; the program runs into the ROM again.
    b.m.debugger_reset();
    assert_eq!(b.run_stop(100_000), StopReason::RomCall);
}

#[test]
fn breakpoints_call_stack_and_instruction_stepping() {
    let mut b = bench(CALLS);
    let (leaf, middle, main) = (b.sym("leaf"), b.sym("middle"), b.sym("main"));
    b.m.set_breakpoints(&[leaf]);
    assert_eq!(b.run_stop(100_000), StopReason::Breakpoint);
    assert_eq!(b.m.pc(), leaf);
    // _start -> main -> middle -> leaf: three frames (innermost last) with the call targets recovered from
    // auipc + jalr.
    let cs = b.snap().call_stack;
    let all: Vec<(u32, u32)> = cs.iter().map(|f| (f.return_pc, f.target_pc)).collect();
    assert_eq!(all.len(), 3, "{all:x?}");
    assert_eq!(all.iter().map(|f| f.1).collect::<Vec<_>>(), [main, middle, leaf]);
    assert!(all[0].0 < main && all[1].0 > main && all[1].0 < middle && all[2].0 > middle);
    let rets = &all[1..];
    // Resuming from the breakpoint address works and hits it again on the next call.
    assert_eq!(b.run_stop(100_000), StopReason::Breakpoint);
    assert_eq!(b.m.pc(), leaf);
    // Step out of leaf: stops right after the call in middle.
    b.m.set_breakpoints(&[]);
    let ret = rets[1].0;
    assert_eq!(b.m.begin_step(StepKind::Out, false), StepPlan::Run);
    assert_eq!(b.run_stop(100_000), StopReason::Requested);
    assert!(b.m.pc() == ret || b.m.pc() == rets[0].0 || (b.m.pc() > middle && b.m.pc() < middle + 0x30), "pc {:#x}", b.m.pc());
    b.m.clear_stop_condition();
    // Step over a call at instruction level: run to the call, step over.
    b.m.debugger_reset();
    let call_site = middle + 6; // the jalr of the first leaf() call (auipc at +6, jalr at +10)
    let _ = call_site;
    b.m.set_breakpoints(&[middle]);
    assert_eq!(b.run_stop(100_000), StopReason::Breakpoint);
    b.m.set_breakpoints(&[]);
    // middle: addi sp; sw ra; sw s0; auipc ra; jalr leaf
    for _ in 0..4 {
        assert_eq!(b.m.begin_step(StepKind::Into, false), StepPlan::Single);
        assert_eq!(b.m.step_one(), StopReason::Limit);
    }
    assert_eq!(b.m.pc(), middle + 6 + 4, "at the jalr");
    assert_eq!(b.m.begin_step(StepKind::Over, false), StepPlan::Run);
    assert_eq!(b.run_stop(100_000), StopReason::Requested);
    assert_eq!(b.m.pc(), middle + 6 + 8, "after the call, leaf was stepped over");
}

#[test]
fn source_level_stepping_uses_the_dwarf_line_table() {
    let mut b = bench(CALLS);
    assert!(!b.prog.lines.is_empty(), "calls.elf has DWARF line info");
    let (leaf, middle, main) = (b.sym("leaf"), b.sym("middle"), b.sym("main"));
    b.m.set_breakpoints(&[middle]);
    assert_eq!(b.run_stop(100_000), StopReason::Breakpoint);
    b.m.set_breakpoints(&[]);
    let line_of = |b: &Bench, pc: u32| b.prog.lines.iter().rev().find(|l| l.address <= pc).map(|l| l.line).unwrap();
    let start = line_of(&b, middle);
    // Source step over: stays in middle()'s lines, never stopping inside leaf().
    let mut lines = vec![start];
    for _ in 0..3 {
        assert_eq!(b.m.begin_step(StepKind::Over, true), StepPlan::Run);
        assert_eq!(b.run_stop(100_000), StopReason::Requested);
        let pc = b.m.pc();
        assert!(pc >= middle && pc < main + 0x40 && !(pc >= leaf && pc < main), "stopped at {pc:#x}");
        lines.push(line_of(&b, pc));
        b.m.clear_stop_condition();
    }
    assert!(lines.windows(2).all(|w| w[0] != w[1]), "{lines:?}");
    // Source step into at the call line enters leaf().
    b.m.debugger_reset();
    b.m.set_breakpoints(&[middle]);
    assert_eq!(b.run_stop(100_000), StopReason::Breakpoint);
    b.m.set_breakpoints(&[]);
    let mut entered = false;
    for _ in 0..4 {
        assert_eq!(b.m.begin_step(StepKind::Into, true), StepPlan::Run);
        assert_eq!(b.run_stop(100_000), StopReason::Requested);
        b.m.clear_stop_condition();
        if (leaf..main).contains(&b.m.pc()) {
            entered = true;
            break;
        }
    }
    assert!(entered, "stepping into the call line reaches leaf()");
}

#[test]
fn run_to_stops_at_the_address() {
    let mut b = bench(CALLS);
    let leaf = b.sym("leaf");
    b.m.run_to(leaf);
    assert_eq!(b.run_stop(100_000), StopReason::Requested);
    assert_eq!(b.m.pc(), leaf);
}

#[test]
fn debugger_writes_and_reset() {
    let mut b = bench(CALLS);
    b.m.write_mem(RES, 4, 0xdead_beef).unwrap();
    assert_eq!(b.word(RES), 0xdead_beef);
    b.m.write_data(RES + 4, 0x5a).unwrap();
    assert_eq!(b.word(RES + 4), 0x5a);
    // Flash is read-only for the CPU but the debugger can patch it.
    assert!(b.m.write_mem(IROM, 4, 0).is_err());
    b.m.write_reg(10, 7).unwrap();
    assert!(b.m.write_reg(0, 1).is_err());
    b.m.write_cpu(mcs_sim::protocol::CpuField::Mtvec, 0x4200_0100).unwrap();
    match b.snap().core {
        CoreState::Riscv { x, mtvec, .. } => assert_eq!((x[10], mtvec), (7, 0x4200_0100)),
        c => panic!("{c:?}"),
    }
    // A debugger reset keeps RAM and time; a power cycle clears RAM.
    b.run(5_000);
    let t = b.m.cycles();
    b.m.debugger_reset();
    assert!(b.m.cycles() >= t && b.m.pc() == IROM);
    b.m.power_cycle();
    assert_eq!((b.m.cycles(), b.word(RES)), (0, 0));
}

#[test]
fn register_view_values_follow_the_peripherals() {
    let mut b = bench(UART);
    b.run(2_000);
    let s = b.snap();
    let spec = devices::get("esp32-c3").unwrap();
    assert_eq!(s.io.len(), spec.registers.len());
    let at = |name: &str| s.io[spec.registers.iter().position(|r| r.name == name).unwrap()];
    assert_eq!(at("UART0_CLKDIV"), 347 | (3 << 20));
    assert_eq!(at("UART0_CLK_CONF") >> 20 & 3, 3, "XTAL clock source");
    assert_eq!(at("SYSTEM_SYSCLK_CONF"), 1);
    assert!(s.peripherals.iter().any(|p| p.name == "SYSTEM" && p.values.iter().any(|v| v.1.contains("20.000 MHz"))), "{:?}", s.peripherals);
}

#[test]
fn initialised_ram_segments_survive_power_cycles() {
    let mut b = bench(DATA);
    assert!(b.prog.segments.iter().any(|s| s.address == 0x3fc8_0000), "{:?}", b.prog.segments.iter().map(|s| s.address).collect::<Vec<_>>());
    assert_eq!(b.word(0x3fc8_0000), 0x1234_5678, "loaded before the first instruction");
    assert_eq!(b.run_stop(100_000), StopReason::BreakInsn);
    assert_eq!(b.results(2), vec![0x1234_5678, 0x1bad_b002]);
    // A debugger reset keeps RAM (the program changed it); a power cycle re-initialises it like a bootloader would.
    b.m.debugger_reset();
    assert_eq!(b.word(0x3fc8_0000), 0x1bad_b002);
    b.m.power_cycle();
    assert_eq!(b.word(0x3fc8_0000), 0x1234_5678);
    assert_eq!(b.word(RES), 0);
    assert_eq!(b.run_stop(100_000), StopReason::BreakInsn);
    assert_eq!(b.results(2), vec![0x1234_5678, 0x1bad_b002]);
}

#[test]
fn uart1_transmits_through_the_gpio_matrix() {
    use mcs_sim::avr::peripherals::serial::SerialConfig;
    let mut b = bench(UART1);
    b.m.set_serial(SerialConfig { monitor: Some(7), inject: None, baud: 115_200.0, data_bits: 8, parity: 0, stop_bits: 1 });
    assert_eq!(b.run_stop(200_000), StopReason::BreakInsn);
    b.snap();
    assert_eq!(b.serial, b"XY");
    assert_eq!(b.results(1)[0] >> 31, 1, "TXD idles high");
    // GPIO7 is driven by the peripheral (output enabled), GPIO21 (UART0 TX) is idle high without any frame.
    let s = b.snap();
    assert_eq!((s.pins[7].dir, s.pins[7].level), (1, 1));
    assert!(b.edges(21).iter().skip(1).all(|e| e.1 == 1));
}

#[test]
fn custom_csrs_pmp_performance_counters_and_illegal_csr_trap() {
    let mut b = bench(CSR);
    assert_eq!(b.run_stop(100_000), StopReason::BreakInsn);
    let r = b.results(8);
    assert_eq!(r[..6], [0x1234_5678, 0x1f, 0, 0, 0, 0x612], "{r:x?}");
    assert_eq!(r[6], 2, "illegal instruction");
    assert_eq!(r[7], 0x600d, "execution continued after the handler skipped the csrr");
}

#[test]
fn a_breakpoint_is_hit_when_a_run_starts_on_it_unless_it_just_fired() {
    let mut b = bench(CALLS);
    let middle = b.sym("middle");
    // Step instruction by instruction to the breakpoint address: it has not fired, so a run stops there at once.
    b.m.set_breakpoints(&[middle]);
    while b.m.pc() != middle {
        assert_eq!(b.m.step_one(), StopReason::Limit);
    }
    assert_eq!(b.run_stop(100), StopReason::Breakpoint);
    assert_eq!((b.m.pc(), b.snap().instructions < 20), (middle, true), "stopped before executing it");
    // After it fired, resuming executes the instruction and runs on to the next hit.
    assert_eq!(b.run_stop(100_000), StopReason::Breakpoint);
    assert_eq!(b.m.pc(), middle);
    assert!(b.snap().instructions > 8);
}
