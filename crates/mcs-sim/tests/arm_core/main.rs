//! ARMv7-M core tests: programs assembled by Apple clang (see `gen_programs.py`, `programs/*.s`)
//! run on `mcs_sim::arm::Machine`. Expected values are derived by hand from the instruction
//! semantics (ARM DDI 0403E.e) and the cycle model documented in `arm/exec.rs`.

mod programs;

use mcs_sim::arm::bus::{Cx, Mmio};
use mcs_sim::arm::{ArmConfig, Machine, StopReason};
use programs::*;

fn boot(p: &Prog) -> Machine {
    let mut m = Machine::new(ArmConfig::default());
    m.load_image(p.code);
    m
}

/// Boots, jumps to the named entry point and returns the machine.
fn boot_at(p: &Prog, entry: &str) -> Machine {
    let mut m = boot(p);
    m.cpu.pc = p.sym(entry);
    m
}

fn run_to_done(m: &mut Machine, p: &Prog) {
    assert_eq!(m.run(100_000_000), StopReason::Bkpt, "pc={:#x}", m.cpu.pc);
    assert_eq!(m.cpu.pc - 2, p.sym("done"), "stopped at {:#x}", m.cpu.pc - 2);
}

fn words(m: &mut Machine, addr: u32, n: usize) -> Vec<u32> {
    (0..n).map(|k| m.mem_read(addr + 4 * k as u32, 4).unwrap()).collect()
}

fn results(m: &mut Machine, n: usize) -> Vec<u32> {
    words(m, 0x2000_0100, n)
}

#[test]
fn alu_flags_shifts_multiply_bitfields() {
    let mut m = boot(&ALU);
    run_to_done(&mut m, &ALU);
    #[rustfmt::skip]
    let want = [
        0x8000_0000, 0x9000_0000,                    // 1. signed overflow
        0xffff_ffff, 0x8000_0000,                    // 2. borrow
        0, 0x6000_0000,                              // 3. carry + zero
        16, 3, 2,                                    // 4. ADC / SBC
        0x0200_0000, 0x2000_0000,                    // 5. LSLS #25 and its flags
        0xffff_ffff, 1, 0x7812_3456, 0x8000_0000,    //    ASRS, LSRS, RORS, RRX
        0x30, 0xff, 0x0f, 0xc0, 0xffff_fff0,         // 6. modified immediates
        42, 49, 7,                                   // 7. MUL MLA MLS
        0xffff_fffd, 2, 0xffff_fffd, 0xffff_ffff,    //    UMULL SMULL
        0xffff_fffa, 5, 0xffff_fffa, 0xffff_ffff,    //    UMLAL SMLAL
        142, 0xffff_ff72, 0, 0,                      //    UDIV SDIV, divide by zero
        0xbc, 0xffff_ffbc, 0xffff_fffb, 0xdff,       // 8. UBFX SBFX BFI
        0xdf0, 3, 0xb3d5_2c48, 0xcdab_3412, 0x3412_cdab, 0xffff_cdab,
        0xcd, 0xffff_ffcd, 0xabcd, 0xffff_abcd, 0xab, // 9. extend
        127, 0x0800_0000, 255, 0, 0xffff_fffb, 0x7fff_ffff, // 10. saturate
    ];
    assert_eq!(results(&mut m, want.len()), want);
    assert_eq!(m.cpu.r[12], 0x2000_0100 + 4 * want.len() as u32, "result pointer");
}

#[test]
fn memory_access_modes_stack_tables_exclusives() {
    let mut m = boot(&MEM);
    run_to_done(&mut m, &MEM);
    #[rustfmt::skip]
    let want = [
        1, 2, 3, 4, 0x2000_0400,                            // STM / LDMDB
        0x2000_7ff4, 0xaa, 0xbb, 0x2000_8000,               // PUSH / POP
        0x1111_2222, 0x8070_ff80,                           // STRD / LDRD
        0x1111, 0x22, 0xffff_ff80, 0xffff_ff80, 0x8011_1122, // sub-word + unaligned
        0x2000_0604, 0xdead_beef, 0x2000_060c, 0xdead_beef, 0, // pre/post index
        0xcafe_f00d, 0xcafe_f00d,                           // literal, ADR
        12, 21,                                             // TBB, TBH
        8, 0, 8, 1, 1,                                      // LDREX / STREX / CLREX
    ];
    assert_eq!(results(&mut m, want.len()), want);
}

#[test]
fn it_blocks() {
    let mut m = boot_at(&FLOW, "it_basic");
    assert_eq!(m.run(10_000), StopReason::Bkpt);
    // ITETE EQ; ITT HI (not taken); narrow ADD inside IT keeps flags (Z=1 C=1 -> 0x6); ITTEE NE.
    assert_eq!(results(&mut m, 8), [1, 3, 0, 0, 7, 6, 3, 4]);
}

#[test]
fn loop_cycle_counts_are_exact() {
    let mut m = boot_at(&FLOW, "loop_bne");
    assert_eq!(m.run(10_000), StopReason::Bkpt);
    assert_eq!(m.cpu.cycles, 400);
    assert_eq!(m.cpu.r[0], 0);
    let mut m = boot_at(&FLOW, "loop_cbz");
    assert_eq!(m.run(10_000), StopReason::Bkpt);
    assert_eq!(m.cpu.cycles, 55);
}

#[test]
fn calls_returns_and_condition_codes() {
    let mut m = boot_at(&FLOW, "call_ret");
    assert_eq!(m.run(10_000), StopReason::Bkpt);
    assert_eq!(results(&mut m, 2), [6, 24]);
    assert_eq!(m.cpu.r[13], 0x2000_8000, "stack balanced");
    let mut m = boot_at(&FLOW, "conds");
    assert_eq!(m.run(10_000), StopReason::Bkpt);
    assert_eq!(results(&mut m, 1), [7]);
}

#[test]
fn nvic_priority_preemption_and_tail_chaining() {
    let mut m = boot(&NVIC);
    // Stop when IRQ2's handler starts: it was tail-chained, so only the thread frame is stacked.
    let irq2 = NVIC.sym("irq2_h");
    let mut deepest = 0x2000_8000u32;
    while m.cpu.pc != irq2 {
        assert_eq!(m.step(), StopReason::Limit);
        deepest = deepest.min(m.cpu.r[13]);
    }
    assert_eq!(m.cpu.r[13], 0x2000_8000 - 0x20, "tail-chained: one frame");
    assert_eq!(m.cpu.ipsr, 16 + 2);
    assert_eq!(deepest, 0x2000_8000 - 0x40, "IRQ1 nested inside IRQ0: two frames");
    run_to_done(&mut m, &NVIC);
    assert_eq!(words(&mut m, 0x2000_0300, 6), [5, 1, 2, 3, 4, 5]);
    assert_eq!(m.cpu.ipsr, 0);
    assert_eq!(m.cpu.r[13], 0x2000_8000);
}

#[test]
fn nvic_priority_grouping_uses_sub_priority() {
    let mut m = boot_at(&NVIC, "grouped");
    assert_eq!(m.run(100_000), StopReason::Bkpt);
    // Nothing preempts (one group); IRQ1 has the lower sub-priority value so it goes first.
    assert_eq!(words(&mut m, 0x2000_0300, 6), [5, 1, 3, 4, 2, 5]);
}

#[test]
fn primask_defers_interrupts() {
    let mut m = boot_at(&NVIC, "masked");
    assert_eq!(m.run(100_000), StopReason::Bkpt);
    // The handler copied the marker (9) that was written while PRIMASK was set.
    assert_eq!(words(&mut m, 0x2000_0320, 3), [9, 9, 1]);
}

#[test]
fn basepri_masks_equal_and_lower_priorities() {
    let mut m = boot_at(&NVIC, "basepri");
    assert_eq!(m.run(100_000), StopReason::Bkpt);
    assert_eq!(words(&mut m, 0x2000_0300, 2), [1, 2], "only IRQ1 ran");
    assert_eq!(words(&mut m, 0x2000_0328, 1), [0], "IRQ3 still masked");
    assert_eq!(m.run(100_000), StopReason::Bkpt);
    assert_eq!(words(&mut m, 0x2000_0328, 1), [1], "IRQ3 ran after BASEPRI was cleared");
}

#[test]
fn svc_and_pendsv() {
    let mut m = boot(&SVC);
    run_to_done(&mut m, &SVC);
    // r0 = 10 + SVC number 5; SVC finished (1) before PendSV ran (stored 1 + 1).
    assert_eq!(results(&mut m, 3), [15, 1, 2]);
    assert_eq!(m.cpu.ipsr, 0);
}

#[test]
fn systick_fires_at_the_exact_cycle() {
    let mut m = boot(&SYSTICK);
    let enable = SYSTICK.sym("enable");
    while m.cpu.pc != enable {
        m.step();
    }
    let s = m.cpu.cycles; // the CSR write starts here
    // Event at s + 100 (RVR + 1 ticks); the spin loop's instruction boundaries fall on s + 2 + 3k,
    // so the first boundary at or after the event is s + 101; entry takes 12 cycles.
    assert_eq!(m.run(s + 101), StopReason::Limit);
    assert_eq!(m.cpu.ipsr, 0, "not yet");
    assert_eq!(m.run(s + 102), StopReason::Limit);
    assert_eq!(m.cpu.ipsr, 15);
    assert_eq!(m.cpu.pc, SYSTICK.sym("systick_h"));
    assert_eq!(m.cpu.cycles, s + 101 + 12);
    assert_eq!(m.cpu.r[14], 0xffff_fff9, "EXC_RETURN: thread mode, MSP");
    assert_eq!(m.mem_read(0x2000_8000 - 0x20 + 24, 4), Some(SYSTICK.sym("spin")), "stacked return address");
    // The interrupt returns to thread mode and the spin loop; ticks keep arriving every 100 cycles.
    assert_eq!(m.run(s + 250), StopReason::Limit);
    assert_eq!(m.cpu.ipsr, 0);
    assert_eq!(m.cpu.r[13], 0x2000_8000);
    assert_eq!(words(&mut m, 0x2000_0200, 1), [2]);
    // The third tick's handler stops at its BKPT.
    assert_eq!(m.run(s + 1000), StopReason::Bkpt);
    assert_eq!(m.cpu.ipsr, 15);
    assert_eq!(words(&mut m, 0x2000_0200, 1), [3]);
    assert!((s + 312..s + 340).contains(&m.cpu.cycles), "stopped at {}", m.cpu.cycles - s);
}

#[test]
fn wfi_sleeps_until_systick_and_fast_forwards() {
    let mut m = boot_at(&SYSTICK, "wfi_main");
    let enable = SYSTICK.sym("wfi_enable");
    while m.cpu.pc != enable {
        m.step();
    }
    let s = m.cpu.cycles;
    // Sleeping: the budget is consumed without executing instructions.
    assert_eq!(m.run(s + 500), StopReason::Limit);
    assert!(m.cpu.sleeping);
    assert_eq!(m.cpu.cycles, s + 500);
    assert_eq!(m.run(100_000), StopReason::Bkpt);
    assert_eq!(m.cpu.pc - 2, SYSTICK.sym("wfi_after"));
    assert_eq!(words(&mut m, 0x2000_0200, 1), [1]);
    // Event at s + 1000 wakes the core: entry 12 + handler + return 12 stays close to the event.
    assert!((s + 1000 + 24..s + 1000 + 60).contains(&m.cpu.cycles), "woke at {}", m.cpu.cycles - s);
    assert_eq!(m.cpu.ipsr, 0);
}

#[test]
fn systick_registers_read_back() {
    let mut m = boot_at(&SYSTICK, "read_back");
    assert_eq!(m.run(10_000), StopReason::Bkpt);
    // CVR 4 cycles after enabling (reload 49): 49 - 3 = 46. COUNTFLAG set after the first wrap,
    // cleared by reading CSR; CVR 72 cycles in: 49 - 21 = 28.
    assert_eq!(results(&mut m, 4), [46, 0x1_0005, 5, 28]);
}

fn fault_run(entry: &str) -> (Machine, Vec<u32>) {
    let mut m = boot_at(&FAULT, entry);
    assert_eq!(m.run(100_000), StopReason::Bkpt, "{entry}");
    let w = words(&mut m, 0x2000_0300, 5);
    (m, w)
}

#[test]
fn udf_escalates_to_hardfault_when_usagefault_disabled() {
    let (m, w) = fault_run("udf_hard");
    // HFSR.FORCED, CFSR.UNDEFINSTR, BFAR -, stacked pc = the UDF, handler id 3.
    assert_eq!(w, [1 << 30, 1 << 16, 0, FAULT.sym("udf_hard_site"), 3]);
    assert_eq!(m.cpu.ipsr, 3);
}

#[test]
fn udf_takes_usagefault_when_enabled() {
    let (m, w) = fault_run("udf_usage");
    assert_eq!(w, [0, 1 << 16, 0, FAULT.sym("udf_usage_site"), 6]);
    assert_eq!(m.cpu.ipsr, 6);
}

#[test]
fn unmapped_load_is_a_precise_bus_fault() {
    let (_, w) = fault_run("bus_hard");
    assert_eq!(w, [1 << 30, (1 << 9) | (1 << 15), 0x6000_0000, FAULT.sym("bus_hard_site"), 3]);
    let (m, w) = fault_run("bus_bus");
    assert_eq!(w, [0, (1 << 9) | (1 << 15), 0x6000_0010, FAULT.sym("bus_bus_site"), 5]);
    assert_eq!(m.cpu.ipsr, 5);
}

#[test]
fn divide_by_zero_trap() {
    let (_, w) = fault_run("div0");
    assert_eq!(w, [0, 1 << 25, 0, FAULT.sym("div0_site"), 6]);
}

#[test]
fn fault_in_hardfault_handler_locks_up() {
    let mut m = boot_at(&FAULT, "lockup");
    assert_eq!(m.run(100_000), StopReason::Lockup);
    assert_eq!(m.cpu.ipsr, 3);
}

#[test]
fn invalid_exc_return_faults() {
    let (m, w) = fault_run("bad_return");
    // INVPC (UFSR bit 18) escalated to HardFault because IRQ0 and UsageFault share priority 0.
    assert_eq!(w[0], 1 << 30);
    assert_eq!(w[1], 1 << 18);
    assert_eq!(w[3], FAULT.sym("bad_return_site"));
    assert_eq!(m.cpu.ipsr, 3);
}

#[test]
fn code_runs_from_ram_and_from_the_flash_alias() {
    let mut m = boot(&ALU);
    // movs r0, #42 ; bkpt #0 in SRAM.
    m.mem_write(0x2000_1000, 4, 0xbe00_202a);
    m.cpu.pc = 0x2000_1000;
    assert_eq!(m.run(100), StopReason::Bkpt);
    assert_eq!(m.cpu.r[0], 42);
    // The same flash code through the boot alias at address 0.
    let mut m = boot(&FLOW);
    m.cpu.pc = FLOW.sym("loop_bne") - 0x0800_0000;
    assert_eq!(m.run(10_000), StopReason::Bkpt);
    assert_eq!(m.cpu.cycles, 400);
}

/// Test peripheral: writing offset 0 arms a one-shot after that many cycles which raises IRQ0;
/// reading offset 4 returns the number of expirations and clears the line.
struct Timer {
    fired: u32,
}

impl Mmio for Timer {
    fn read(&mut self, offset: u32, _size: u8, cx: &mut Cx) -> u32 {
        if offset == 4 {
            cx.clear_irq(0);
            self.fired
        } else {
            0
        }
    }

    fn write(&mut self, offset: u32, _size: u8, value: u32, cx: &mut Cx) {
        if offset == 0 {
            cx.schedule(0, cx.cycles + value as u64);
        }
    }

    fn on_event(&mut self, _tag: u8, cx: &mut Cx) {
        self.fired += 1;
        cx.raise_irq(0);
    }
}

#[test]
fn mmio_peripheral_events_raise_interrupts() {
    let mut m = Machine::new(ArmConfig::default());
    m.add_peripheral(0x4000_0000, 0x400, Box::new(Timer { fired: 0 }));
    m.load_image(MMIO.code);
    let arm = MMIO.sym("arm_timer");
    while m.cpu.pc != arm {
        m.step();
    }
    let s = m.cpu.cycles;
    // Event at s + 500; boundaries at s + 2 + 3k include s + 500 exactly; entry + 12.
    assert_eq!(m.run(s + 501), StopReason::Limit);
    assert_eq!(m.cpu.ipsr, 16);
    assert_eq!(m.cpu.cycles, s + 500 + 12);
    run_to_done(&mut m, &MMIO);
    assert_eq!(words(&mut m, 0x2000_0200, 1), [1]);
    // An access to an unmapped peripheral address is a bus fault.
    assert_eq!(m.mem_read(0x4001_0000, 4), None);
}

#[test]
fn throughput() {
    for (name, p, entry, expect_cycles) in [("tight loop", &TPUT, "reset", Some(50_000_001u64)), ("mixed", &TPUT, "mixed", None)] {
        let mut m = boot_at(p, entry);
        let t = std::time::Instant::now();
        let stop = m.run(u64::MAX / 2);
        let secs = t.elapsed().as_secs_f64();
        assert_eq!(stop, StopReason::Bkpt);
        if let Some(c) = expect_cycles {
            assert_eq!(m.cpu.cycles, c);
        }
        let mhz = m.cpu.cycles as f64 / secs / 1e6;
        println!("{name}: {} cycles, {} instructions in {:.3}s = {:.0} simulated MHz", m.cpu.cycles, m.cpu.instructions, secs, mhz);
        if !cfg!(debug_assertions) {
            assert!(mhz > 100.0, "{name}: {mhz:.0} MHz is too slow");
        }
    }
}
