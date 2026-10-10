//! RV32IMC core tests: programs assembled with the rustc `riscv32imc` toolchain (see
//! `gen_programs.py`, `programs/*.s`) run on `mcs_sim::riscv::Machine`. Expected values follow the
//! RISC-V specifications (Unprivileged / Privileged 20240411) and the cycle model documented in
//! `riscv/machine.rs`; `micro.rs` covers bus / pre-decode behaviour with hand-encoded instructions.

mod micro;
mod programs;

use std::sync::{Arc, Mutex};

use mcs_core::riscv::{decode, insn_len};
use mcs_sim::riscv::{Bus, Cx, Machine, Mmio, RvConfig, StopReason};
use programs::*;

pub const IROM: u32 = 0x4200_0000;
pub const RESULTS: u32 = 0x3fc8_0100;
pub const IRQ_DEV: u32 = 0x6000_0000;

/// Test device: offset 0 raises, offset 4 lowers the interrupt lines given by the written mask.
pub struct IrqCtl;

impl Mmio for IrqCtl {
    fn read(&mut self, _offset: u32, _size: u8, _cx: &mut Cx) -> u32 {
        0
    }
    fn write(&mut self, offset: u32, _size: u8, value: u32, cx: &mut Cx) {
        match offset {
            0 => cx.irq_raise |= value,
            4 => cx.irq_lower |= value,
            _ => {}
        }
    }
}

/// An ESP32-C3 memory map with the test interrupt device; `ebreak` stops the run.
pub fn machine() -> Machine {
    let (mut bus, _flash, _sram) = Bus::esp32c3(0x10000);
    bus.add_device(IRQ_DEV, 0x1000, Box::new(IrqCtl)).unwrap();
    let mut m = Machine::with_bus(RvConfig::default(), bus);
    m.halt_on_ebreak = true;
    m
}

pub fn boot(p: &Prog, entry: &str) -> Machine {
    let mut m = machine();
    m.bus.load(IROM, p.code);
    m.set_pc(p.sym(entry));
    m
}

/// Runs to the `ebreak` at symbol `done` (`len` = size of that instruction).
pub fn run_done(m: &mut Machine, p: &Prog, done: &str, len: u32) {
    assert_eq!(m.run(100_000_000), StopReason::Ebreak, "pc={:#x}", m.cpu.pc);
    assert_eq!(m.cpu.pc - len, p.sym(done), "stopped at {:#x}", m.cpu.pc - len);
}

pub fn results(m: &Machine, n: usize) -> Vec<u32> {
    (0..n).map(|k| m.mem_read(RESULTS + 4 * k as u32, 4).unwrap()).collect()
}

/// The result pointer (s11) tells how many words the program recorded.
pub fn recorded(m: &Machine) -> usize {
    ((m.cpu.x[27] - RESULTS) / 4) as usize
}

fn edges(p: &Prog, n: usize) -> Vec<u32> {
    let off = (p.sym("edges") - IROM) as usize;
    (0..n).map(|k| u32::from_le_bytes(p.code[off + 4 * k..off + 4 * k + 4].try_into().unwrap())).collect()
}

#[test]
fn alu_register_and_immediate_ops() {
    let mut m = boot(&ALU, "_start");
    run_done(&mut m, &ALU, "done", 4);
    let e = edges(&ALU, 15);
    let mut want = Vec::new();
    for &a in &e {
        for &b in &e {
            want.extend([
                a.wrapping_add(b),
                a.wrapping_sub(b),
                a << (b & 31),
                ((a as i32) < (b as i32)) as u32,
                (a < b) as u32,
                a ^ b,
                a >> (b & 31),
                ((a as i32) >> (b & 31)) as u32,
                a | b,
                a & b,
            ]);
        }
    }
    for &a in &e {
        for imm in [-2048i32, -1, 0, 1, 2047] {
            want.push(a.wrapping_add(imm as u32));
        }
        for imm in [-2048i32, -1, 0, 1, 2047] {
            want.push(((a as i32) < imm) as u32);
        }
        for imm in [-2048i32, -1, 0, 1, 2047] {
            want.push((a < imm as u32) as u32);
        }
        for imm in [-1i32, 0x7ff, -2048] {
            want.push(a ^ imm as u32);
        }
        for imm in [-1i32, 0x7ff, -2048] {
            want.push(a | imm as u32);
        }
        for imm in [-1i32, 0x7ff, -2048] {
            want.push(a & imm as u32);
        }
        for sh in [0u32, 1, 31] {
            want.push(a << sh);
        }
        for sh in [0u32, 1, 31] {
            want.push(a >> sh);
        }
        for sh in [0u32, 1, 31] {
            want.push(((a as i32) >> sh) as u32);
        }
    }
    // lui, auipc (offset from the label), x0
    want.extend([0xffff_f000, 0x1234_5678, 0x8000_0000, 0]);
    want.extend([0, 0x1000, 0xffff_f000]);
    want.extend([0; 7]);
    want.extend([7, 0xffff_fff9, 1, 0]);
    assert_eq!(recorded(&m), want.len());
    assert_eq!(results(&m, want.len()), want);
    assert_eq!(m.cpu.x[0], 0);
}

#[test]
fn multiply_and_divide_edge_cases() {
    let mut m = boot(&MULDIV, "_start");
    run_done(&mut m, &MULDIV, "done", 4);
    let e = edges(&MULDIV, 15);
    let mut want = Vec::new();
    for &a in &e {
        for &b in &e {
            let (sa, sb, ua, ub) = (a as i32 as i128, b as i32 as i128, a as i128, b as i128);
            want.push(a.wrapping_mul(b));
            want.push(((sa * sb) >> 32) as u32);
            want.push(((sa * ub) >> 32) as u32);
            want.push(((ua * ub) >> 32) as u32);
            // signed division rounds toward zero; x/0 = -1, rem x%0 = x; INT_MIN/-1 = INT_MIN, rem 0
            let (div, rem) = if b == 0 {
                (u32::MAX, a)
            } else if sa == i32::MIN as i128 && sb == -1 {
                (a, 0)
            } else {
                ((sa / sb) as u32, (sa % sb) as u32)
            };
            want.push(div);
            want.push(a.checked_div(b).unwrap_or(u32::MAX));
            want.push(rem);
            want.push(a.checked_rem(b).unwrap_or(a));
        }
    }
    want.extend([0x8000_0000, 0, 123_456_789u32.wrapping_mul(1000)]);
    assert_eq!(results(&m, want.len()), want);
    assert_eq!(recorded(&m), want.len());
    // spot checks written out by hand
    let at = |a: u32, b: u32, op: usize| {
        let (i, j) = (e.iter().position(|&x| x == a).unwrap(), e.iter().position(|&x| x == b).unwrap());
        want[(i * 15 + j) * 8 + op]
    };
    assert_eq!(at(0x8000_0000, 0xffff_ffff, 4), 0x8000_0000); // div overflow
    assert_eq!(at(0x8000_0000, 0xffff_ffff, 6), 0); // rem overflow
    assert_eq!(at(7, 0, 4), 0xffff_ffff); // div by zero
    assert_eq!(at(7, 0, 5), 0xffff_ffff); // divu by zero
    assert_eq!(at(7, 0, 6), 7); // rem by zero
    assert_eq!(at(0xffff_fff9, 2, 4), 0xffff_fffd); // -7 / 2 = -3
    assert_eq!(at(0xffff_fff9, 2, 6), 0xffff_ffff); // -7 % 2 = -1
    assert_eq!(at(0xffff_ffff, 0xffff_ffff, 1), 0); // mulh(-1,-1) = 0
    assert_eq!(at(0xffff_ffff, 0xffff_ffff, 3), 0xffff_fffe); // mulhu(max,max)
    assert_eq!(at(0xffff_ffff, 0xffff_ffff, 2), 0xffff_ffff); // mulhsu(-1,max) = -1
    assert_eq!(at(0x8000_0000, 0x8000_0000, 1), 0x4000_0000); // mulh(min,min)
}

#[test]
fn loads_stores_extension_and_alias_windows() {
    let mut m = boot(&MEM, "_start");
    run_done(&mut m, &MEM, "done", 4);
    #[rustfmt::skip]
    let want = [
        0x89ab_cdef, 0xffff_cdef, 0xffff_89ab, 0xcdef, 0x89ab,
        0xffff_ffef, 0xffff_ffcd, 0xffff_ffab, 0xffff_ff89, 0xef, 0xcd, 0xab, 0x89,
        0x89ab_efef, 0x5678_efef, 0x5678_5678, 0x7878_5678,
        0x0bad_f00d, 0x0bad_f00d, 0x0bad_f00d,
        0xa5a5_5a5a, 0x5a, 0x1122_3344, 0x1122_3344, 0x5566_7788,
        0xfeed_face, 0,
    ];
    assert_eq!(results(&m, want.len()), want);
    assert_eq!(recorded(&m), want.len());
}

#[test]
fn branches_jumps_and_links() {
    let mut m = boot(&BRANCH, "_start");
    run_done(&mut m, &BRANCH, "done", 4);
    let off = (BRANCH.sym("pairs") - IROM) as usize;
    let mut want = Vec::new();
    for k in 0..12 {
        let w = |i: usize| u32::from_le_bytes(BRANCH.code[off + 8 * k + 4 * i..off + 8 * k + 4 * i + 4].try_into().unwrap());
        let (a, b) = (w(0), w(1));
        let mut mask = 0;
        for (bit, cond) in [(1, a == b), (2, a != b), (4, (a as i32) < (b as i32)), (8, (a as i32) >= (b as i32)), (16, a < b), (32, a >= b)] {
            if cond {
                mask |= bit;
            }
        }
        want.push(mask);
    }
    want.extend([4, 55, 4, 4, 0x1234, 0x5678]);
    assert_eq!(results(&m, want.len()), want);
    assert_eq!(recorded(&m), want.len());
}

/// Names the compressed instruction form of a halfword.
fn cform(h: u16) -> &'static str {
    let (rd, rs2) = ((h >> 7) & 31, (h >> 2) & 31);
    match (h & 3, h >> 13) {
        (0, 0) => "c.addi4spn",
        (0, 2) => "c.lw",
        (0, 6) => "c.sw",
        (1, 0) if h == 1 => "c.nop",
        (1, 0) => "c.addi",
        (1, 1) => "c.jal",
        (1, 2) => "c.li",
        (1, 3) if rd == 2 => "c.addi16sp",
        (1, 3) => "c.lui",
        (1, 4) => match ((h >> 10) & 3, (h >> 5) & 3) {
            (0, _) => "c.srli",
            (1, _) => "c.srai",
            (2, _) => "c.andi",
            (_, 0) => "c.sub",
            (_, 1) => "c.xor",
            (_, 2) => "c.or",
            _ => "c.and",
        },
        (1, 5) => "c.j",
        (1, 6) => "c.beqz",
        (1, 7) => "c.bnez",
        (2, 0) => "c.slli",
        (2, 2) => "c.lwsp",
        (2, 4) if h & 0x1000 == 0 => {
            if rs2 == 0 {
                "c.jr"
            } else {
                "c.mv"
            }
        }
        (2, 4) => {
            if rs2 != 0 {
                "c.add"
            } else if rd == 0 {
                "c.ebreak"
            } else {
                "c.jalr"
            }
        }
        (2, 6) => "c.swsp",
        _ => "?",
    }
}

#[test]
fn compressed_instructions() {
    let mut m = boot(&RVC, "_start");
    run_done(&mut m, &RVC, "done", 2);
    #[rustfmt::skip]
    let want = [
        0xffff_fffe, 0xffff_ffff, 0xffff_fffd, 0xffff_ffff,    // c.li c.addi c.mv c.add
        0x0001_f000, 0xfffe_1000,                              // c.lui
        64, 432, 80, 1020, 4,                                  // c.addi16sp c.addi4spn
        0x0102_0304, 0x0102_0304, 0x0102_0304, 0x0102_0304, 0, // c.sw c.lw c.swsp c.lwsp
        0x0f0f_0f0f, 0xffff_ffff, 1, 0x1234_5670, 0,           // c.srli c.srai c.andi
        0xef10_ef10, 0xf0f0_f0f0, 0xfff0_fff0, 0x0f00_0f00,    // c.sub c.xor c.or c.and
        0x8000_0000, 0x00ab_cd00,                              // c.slli
        21, 0, 17,                                             // c.j c.jal c.jalr
        0, 2, 2, 9,                                            // c.beqz c.bnez
        2, 0,                                                  // 32-bit instructions on 2-byte boundaries
    ];
    assert_eq!(results(&m, want.len()), want);
    assert_eq!(recorded(&m), want.len());
    // every compressed form is used by the program image
    let start = (RVC.sym("rvc_start") - IROM) as usize;
    let mut forms = std::collections::BTreeSet::new();
    let mut p = start;
    while p + 2 <= RVC.code.len() {
        let h = u16::from_le_bytes([RVC.code[p], RVC.code[p + 1]]);
        let len = insn_len(h) as usize;
        if len == 2 {
            forms.insert(cform(h));
            assert_eq!(decode(h as u32).len, 2);
        }
        p += len;
    }
    let all = [
        "c.addi4spn", "c.lw", "c.sw", "c.nop", "c.addi", "c.jal", "c.li", "c.addi16sp", "c.lui", "c.srli", "c.srai", "c.andi", "c.sub", "c.xor",
        "c.or", "c.and", "c.j", "c.beqz", "c.bnez", "c.slli", "c.lwsp", "c.jr", "c.mv", "c.ebreak", "c.jalr", "c.add", "c.swsp",
    ];
    for f in all {
        assert!(forms.contains(f), "program does not exercise {f}: {forms:?}");
    }
}

#[test]
fn csr_instructions_and_registers() {
    let mut m = boot(&CSR, "_start");
    run_done(&mut m, &CSR, "done", 4);
    #[rustfmt::skip]
    let want = [
        // csrrw / csrrs / csrrc and immediate forms
        0x1234_5678, 0x1234_5678, 0xa5a5_a5a5, 0xa5a5_a5a5, 0xafaf_afaf, 0xafaf_afaf, 0x00af_00af,
        0x00af_00af, 21, 21, 31, 31, 26, 26, 26, 26, 26,
        // identification
        0x4000_1104, 0x612, 0x8000_0001, 1, 0, 0x612, 0x4000_1104,
        // mstatus
        0x1800, 0x1808, 0x1888, 0x1880,
        // mie mip mtvec mepc mcause mtval mcounteren
        0xffff_fffe, 0, 0x4200_0101, 0x4200_0100, 0x4200_0100, 0x4200_0002, 0x4200_0004, 0x8000_000b, 0xdead_beef, 7,
        // counters
        5, 4, 1, 1, 1, 1, 5, 1, 7, 7, 3, 3,
        // hpm
        0, 0, 0, 0, 0,
    ];
    assert_eq!(results(&m, want.len()), want);
    assert_eq!(recorded(&m), want.len());
}

fn trap_word(m: &Machine, n: usize) -> [u32; 4] {
    let r = results(m, 4 * (n + 1));
    [r[4 * n], r[4 * n + 1], r[4 * n + 2], r[4 * n + 3]]
}

#[test]
fn synchronous_traps_report_cause_epc_and_tval() {
    let mut m = boot(&TRAP, "_start");
    m.halt_on_ebreak = false; // `ebreak` is one of the traps; the program ends in `wfi`
    assert_eq!(m.run(1_000_000), StopReason::Wfi);
    assert_eq!(m.cpu.pc - 4, TRAP.sym("done"));
    let s = |n: &str| TRAP.sym(n);
    let dram = 0x3fc8_0400;
    let hartid_ro = (0xf14 << 20) | (1 << 15) | (6 << 12) | (12 << 7) | 0x73;
    #[rustfmt::skip]
    let want: Vec<(u32, u32, u32)> = vec![
        (2, s("site_unimp"), 0xc000_1073),
        (11, s("site_ecall"), 0),
        (3, s("site_ebreak"), s("site_ebreak")),
        (4, s("site_ldmis"), dram + 1),
        (4, s("site_ldmis2"), dram + 3),
        (6, s("site_stmis"), dram + 2),
        (6, s("site_stmis2"), dram + 1),
        (5, s("site_ldfault0"), 0),
        (5, s("site_ldfault1"), 0x7000_0010),
        (7, s("site_stfault_rom"), s("_start")),
        (7, s("site_stfault_rom2"), s("_start")),
        (7, s("site_stfault_drom"), 0x3c00_0000),
        (7, s("site_stfault_unmapped"), 0x7000_0000),
        (2, s("site_csr_ro"), 0xf115_1073),
        (2, s("site_csr_ro2"), hartid_ro),
        (2, s("site_csr_unknown"), 0x7c00_2673),
    ];
    // the 16 data traps, then one recorded word, then the fetch faults and the 16-bit illegal
    for (n, &(cause, epc, tval)) in want.iter().enumerate() {
        assert_eq!(trap_word(&m, n), [cause, epc, tval, 0x1880], "trap {n}");
    }
    let r = results(&m, 16 * 4 + 1 + 5 * 4 + 1);
    assert_eq!(r[64], 0x612, "legal CSR read does not trap");
    let fetch = [
        (s("site_fetch_dram"), 0x3fc8_0000),
        (s("site_fetch_unmapped"), 0x1000_0000),
        (s("site_fetch_mmio"), 0x6000_0000),
        (s("site_fetch_zero"), 0),
    ];
    for (k, &(_site, target)) in fetch.iter().enumerate() {
        let b = 65 + 4 * k;
        assert_eq!(&r[b..b + 4], &[1, target, target, 0x1880], "fetch fault {k}");
    }
    assert_eq!(&r[81..85], &[2, s("site_c_unimp"), 0, 0x1880]);
    assert_eq!(r[85], 0x1888, "mret restored MIE");
    assert_eq!(recorded(&m), 86);
}

fn rec3(m: &Machine, base: usize, n: usize) -> Vec<[u32; 3]> {
    let r = results(m, base + 3 * n);
    (0..n).map(|k| [r[base + 3 * k], r[base + 3 * k + 1], r[base + 3 * k + 2]]).collect()
}

#[test]
fn vectored_interrupts_arbitration_and_masking() {
    let mut m = boot(&IRQ, "t_vectored");
    run_done(&mut m, &IRQ, "done", 4);
    let t = IRQ.sym("vec_table");
    let ent = |n: u32| t + 4 * n + 4; // ra written by `jal ra, common_handler` in entry n
    let irq = |n: u32| 0x8000_0000 | n;
    let s = |n: &str| IRQ.sym(n);
    let mut want: Vec<u32> = Vec::new();
    // lines 5 and 20 raised together: 20 first, then 5; both resume at the instruction after the store
    want.extend([irq(20), ent(20), s("v1_after"), irq(5), ent(5), s("v1_after")]);
    // MEI > MSI > MTI > the rest
    for n in [11, 3, 7, 9] {
        want.extend([irq(n), ent(n), s("v2_after")]);
    }
    // exception in vectored mode enters at the base
    want.extend([11, t + 4, s("v_ecall")]);
    want.push(0x40); // pending but not enabled
    want.extend([irq(6), ent(6), s("v3_after")]);
    want.push(0x20); // pending with MIE = 0
    want.extend([irq(5), ent(5), s("v4_after")]);
    want.extend([0x1888, 0]);
    assert_eq!(results(&m, want.len()), want);
    assert_eq!(recorded(&m), want.len());
    assert_eq!(m.irq_pending(), 0);
}

#[test]
fn direct_mode_interrupts_enter_at_the_base() {
    let mut m = boot(&IRQ, "t_direct");
    assert_eq!(m.run(1_000_000), StopReason::Ebreak);
    assert_eq!(m.cpu.pc - 4, IRQ.sym("d_end"));
    let irq = |n: u32| 0x8000_0000 | n;
    let want = [
        [irq(20), 0x1111, IRQ.sym("d1_after")],
        [11, 0x1111, IRQ.sym("d_ecall")],
        [irq(13), 0x1111, IRQ.sym("d3_after")],
    ];
    assert_eq!(rec3(&m, 0, 3), want);
}

#[test]
fn wfi_sleeps_until_an_enabled_interrupt_is_pending() {
    let mut m = boot(&IRQ, "t_wfi");
    let s = |n: &str| IRQ.sym(n);
    // sleeps at the first wfi
    assert_eq!(m.run(10_000), StopReason::Wfi);
    assert!(m.is_sleeping());
    assert_eq!(m.cpu.pc, s("w1_after"));
    let (cycles, instret) = (m.cpu.cycles, m.cpu.instret);
    assert_eq!(m.run(10_000), StopReason::Wfi, "still asleep");
    assert_eq!((m.cpu.cycles, m.cpu.instret), (cycles, instret), "no time passes while the host does not idle");
    m.idle(500);
    assert_eq!(m.cpu.cycles, cycles + 500);
    // a line that is not enabled does not wake it
    m.set_irq(9);
    assert_eq!(m.run(10_000), StopReason::Wfi);
    m.clear_irq(9);
    m.set_irq(20);
    assert!(!m.is_sleeping());
    // handler runs, returns after the wfi, then MIE = 0 and the second wfi sleeps
    assert_eq!(m.run(10_000), StopReason::Wfi);
    assert_eq!(m.cpu.pc, s("w2_after"));
    m.set_irq(5); // wakes without trapping because MIE = 0
    assert!(!m.is_sleeping());
    assert_eq!(m.run(1_000_000), StopReason::Ebreak);
    assert_eq!(m.cpu.pc - 4, s("done_wfi"));
    let irq = |n: u32| 0x8000_0000 | n;
    let r = results(&m, 12);
    assert_eq!(&r[0..3], &[irq(20), 0, s("w1_after")]);
    assert_eq!(&r[3..6], &[1, 0x20, 2]);
    // after MIE is set the pending lines 6 and 5 are taken, highest first
    assert_eq!(&r[6..9], &[irq(6), 0, s("w4_after")]);
    assert_eq!(&r[9..12], &[irq(5), 0, s("w4_after")]);
    assert_eq!(recorded(&m), 12);
}

#[test]
fn cycle_model() {
    let mut m = boot(&TIMING, "_start");
    run_done(&mut m, &TIMING, "done", 4);
    #[rustfmt::skip]
    let want = [
        1, 2, 1, 2,             // add lw sw lb
        1, 1, 1, 1,             // mul mulh mulhsu mulhu
        33, 33, 33, 33, 33,     // div divu rem remu div-by-zero
        1, 1, 1, 1, 3,          // lui auipc csrr fence fence.i
        2, 3, 1, 3,             // jal, taken branch, not-taken branch, jalr
        3, 3,                   // trap entry, mret
        5,                      // instret
    ];
    assert_eq!(results(&m, want.len()), want);
}

/// Shared helper for the micro tests.
pub type Log = Arc<Mutex<Vec<(char, u32, u8, u32)>>>;

pub fn shared_log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}
