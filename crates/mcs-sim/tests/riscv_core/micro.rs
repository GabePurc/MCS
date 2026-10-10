//! Bus, pre-decode cache, interrupt API and CSR-hook tests with hand-encoded instructions.

use mcs_sim::riscv::{Bus, Cx, Machine, Mmio, RvConfig, StopReason, PERM_RW, PERM_RX};

use super::{machine, shared_log, Log, IRQ_DEV};

const IRAM: u32 = 0x4037_c000;
const IRAM_END: u32 = 0x403e_0000;

fn i_type(imm: i32, rs1: u32, f3: u32, rd: u32, op: u32) -> u32 {
    ((imm as u32 & 0xfff) << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op
}
fn s_type(imm: i32, rs2: u32, rs1: u32, f3: u32) -> u32 {
    let imm = imm as u32 & 0xfff;
    ((imm >> 5) << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | ((imm & 31) << 7) | 0x23
}
fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    i_type(imm, rs1, 0, rd, 0x13)
}
fn slli(rd: u32, rs1: u32, sh: u32) -> u32 {
    i_type(sh as i32, rs1, 1, rd, 0x13)
}
fn lui(rd: u32, imm20: u32) -> u32 {
    (imm20 << 12) | (rd << 7) | 0x37
}
fn load(f3: u32, rd: u32, rs1: u32, imm: i32) -> u32 {
    i_type(imm, rs1, f3, rd, 0x03)
}
fn sw(rs2: u32, rs1: u32, imm: i32) -> u32 {
    s_type(imm, rs2, rs1, 2)
}
fn sh(rs2: u32, rs1: u32, imm: i32) -> u32 {
    s_type(imm, rs2, rs1, 1)
}
fn sb(rs2: u32, rs1: u32, imm: i32) -> u32 {
    s_type(imm, rs2, rs1, 0)
}
/// `jal rd, off` with `off` relative to the jal itself.
fn jal(rd: u32, off: i32) -> u32 {
    let o = off as u32;
    ((o >> 20 & 1) << 31) | ((o >> 1 & 0x3ff) << 21) | ((o >> 11 & 1) << 20) | ((o >> 12 & 0xff) << 12) | (rd << 7) | 0x6f
}
fn csrrw(rd: u32, csr: u32, rs1: u32) -> u32 {
    i_type(csr as i32, rs1, 1, rd, 0x73)
}
fn csrrs(rd: u32, csr: u32, rs1: u32) -> u32 {
    i_type(csr as i32, rs1, 2, rd, 0x73)
}
const EBREAK: u32 = 0x0010_0073;
const MRET: u32 = 0x3020_0073;
const WFI: u32 = 0x1050_0073;
const FENCE_I: u32 = 0x0000_100f;

fn put(m: &mut Machine, addr: u32, words: &[u32]) {
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    m.bus.load(addr, &bytes);
}

fn run_ebreak(m: &mut Machine) {
    assert_eq!(m.run(1_000_000), StopReason::Ebreak, "pc={:#x}", m.cpu.pc);
}

#[test]
fn executes_from_iram_and_counts() {
    let mut m = machine();
    put(&mut m, IRAM, &[addi(10, 0, 42), EBREAK]);
    m.set_pc(IRAM);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.x[10], 42);
    assert_eq!(m.cpu.pc, IRAM + 8);
    assert_eq!((m.cpu.cycles, m.cpu.instret), (1, 2));
}

#[test]
fn stores_into_code_invalidate_predecoded_instructions() {
    let mut m = machine();
    let t = 0x4038_0040; // IRAM window; the same bytes are 0x3FC8_0040 in DRAM
    put(&mut m, t, &[addi(10, 0, 1), EBREAK]);
    m.set_pc(t);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.x[10], 1);
    // 1. a word store through the IRAM window replaces `addi a0,x0,1` by `addi a0,x0,7`
    let main = 0x4038_0000;
    put(
        &mut m,
        main,
        &[lui(5, 0x40380), lui(6, 0x700), addi(6, 6, 0x513), sw(6, 5, 0x40), FENCE_I, jal(0, 0x40 - 20)],
    );
    m.cpu.x[10] = 0;
    m.set_pc(main);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.x[10], 7, "stale pre-decoded instruction executed");
    // 2. a store through the DRAM alias patches the executable window as well
    put(&mut m, main, &[lui(5, 0x3fc80), lui(6, 0x900), addi(6, 6, 0x513), sw(6, 5, 0x40), jal(0, 0x40 - 16)]);
    m.set_pc(main);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.x[10], 9, "DRAM-alias store did not invalidate the IRAM view");
    // 3. a halfword store into the second half of a 32-bit instruction invalidates it too
    let t2 = 0x4038_0080;
    put(&mut m, t2, &[i_type(0x111, 0, 0, 10, 0x13), EBREAK]);
    m.set_pc(t2);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.x[10], 0x111);
    put(&mut m, main, &[lui(5, 0x40380), addi(6, 0, 0x222), slli(6, 6, 4), sh(6, 5, 0x82), jal(0, 0x80 - 16)]);
    m.set_pc(main);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.x[10], 0x222);
    // 4. a byte store into the immediate field
    let t3 = 0x4038_0100;
    put(&mut m, t3, &[i_type(0x111, 0, 0, 10, 0x13), EBREAK]);
    m.set_pc(t3);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.x[10], 0x111);
    put(&mut m, main, &[lui(5, 0x40380), addi(6, 0, 0x22), sb(6, 5, 0x103), jal(0, 0x100 - 12)]);
    m.set_pc(main);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.x[10], 0x221);
}

#[test]
fn host_loads_invalidate_predecoded_instructions() {
    let mut m = machine();
    put(&mut m, IRAM, &[addi(10, 0, 3), EBREAK]);
    m.set_pc(IRAM);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.x[10], 3);
    put(&mut m, IRAM, &[addi(10, 0, 4), EBREAK]);
    m.set_pc(IRAM);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.x[10], 4);
}

#[test]
fn instruction_straddling_a_page_boundary() {
    let mut m = machine();
    let at = IRAM + 0xffe;
    let w = addi(10, 0, 99);
    m.bus.load(at, &w.to_le_bytes());
    put(&mut m, IRAM + 0x1002, &[EBREAK]);
    m.set_pc(at);
    run_ebreak(&mut m);
    assert_eq!((m.cpu.x[10], m.cpu.pc), (99, IRAM + 0x1006));
    // patch only the half that lives in the next page: the decoded entry in the previous page dies
    let w2 = addi(10, 0, 100);
    m.bus.load(IRAM + 0x1000, &w2.to_le_bytes()[2..]);
    m.set_pc(at);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.x[10], 100);
}

#[test]
fn second_halfword_fetch_fault_reports_its_address() {
    let mut m = machine();
    // `addi` whose second half would lie in the unmapped space right after IRAM
    m.bus.load(IRAM_END - 2, &addi(10, 0, 1).to_le_bytes()[..2]);
    put(&mut m, IRAM, &[EBREAK]);
    m.cpu.csr_write(0x305, IRAM);
    m.set_pc(IRAM_END - 2);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.csr_read(0x342), Some(1));
    assert_eq!(m.cpu.csr_read(0x341), Some(IRAM_END - 2));
    assert_eq!(m.cpu.csr_read(0x343), Some(IRAM_END));
}

#[test]
fn fetch_from_non_executable_window_faults_even_if_the_memory_is_cached() {
    let mut m = machine();
    put(&mut m, IRAM + 0x5000, &[addi(10, 0, 5), EBREAK]);
    m.set_pc(IRAM + 0x5000);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.x[10], 5);
    // the DRAM view of the same bytes (0x3FC8_1000) is not executable
    m.set_pc(0x3fc8_1000);
    m.cpu.csr_write(0x305, IRAM);
    put(&mut m, IRAM, &[EBREAK]);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.csr_read(0x342), Some(1));
    assert_eq!(m.cpu.csr_read(0x343), Some(0x3fc8_1000));
    assert_eq!(m.cpu.csr_read(0x341), Some(0x3fc8_1000));
}

struct Probe {
    log: Log,
}

impl Mmio for Probe {
    fn read(&mut self, offset: u32, size: u8, _cx: &mut Cx) -> u32 {
        self.log.lock().unwrap().push(('r', offset, size, 0));
        0x80a5_f08f
    }
    fn write(&mut self, offset: u32, size: u8, value: u32, _cx: &mut Cx) {
        self.log.lock().unwrap().push(('w', offset, size, value));
    }
}

#[test]
fn mmio_accesses_dispatch_with_size_and_extension() {
    let mut m = machine();
    let log = shared_log();
    m.bus.add_device(0x6000_1000, 0x1000, Box::new(Probe { log: log.clone() })).unwrap();
    put(
        &mut m,
        IRAM,
        &[
            lui(5, 0x60001),
            load(2, 10, 5, 0x10), // lw
            load(1, 11, 5, 0x12), // lh
            load(5, 12, 5, 0x12), // lhu
            load(0, 13, 5, 0x13), // lb
            load(4, 14, 5, 0x13), // lbu
            sw(10, 5, 0x20),
            sh(10, 5, 0x22),
            sb(10, 5, 0x23),
            EBREAK,
        ],
    );
    m.set_pc(IRAM);
    run_ebreak(&mut m);
    assert_eq!(&m.cpu.x[10..15], &[0x80a5_f08f, 0xffff_f08f, 0xf08f, 0xffff_ff8f, 0x8f]);
    assert_eq!(
        *log.lock().unwrap(),
        vec![
            ('r', 0x10, 4, 0),
            ('r', 0x12, 2, 0),
            ('r', 0x12, 2, 0),
            ('r', 0x13, 1, 0),
            ('r', 0x13, 1, 0),
            ('w', 0x20, 4, 0x80a5_f08f),
            ('w', 0x22, 2, 0xf08f),
            ('w', 0x23, 1, 0x8f),
        ]
    );
    // peripherals are invisible to the debugger view
    assert_eq!(m.mem_read(0x6000_1010, 4), None);
    assert_eq!(log.lock().unwrap().len(), 8);
}

#[test]
fn ebreak_traps_unless_it_is_a_host_breakpoint() {
    let mut m = machine();
    m.halt_on_ebreak = false;
    put(&mut m, IRAM, &[lui(5, 0x4037c), addi(5, 5, 0x40), csrrw(0, 0x305, 5), EBREAK]);
    put(&mut m, IRAM + 0x40, &[WFI]);
    m.set_pc(IRAM);
    assert_eq!(m.run(1000), StopReason::Wfi);
    assert_eq!(m.cpu.pc, IRAM + 0x44);
    assert_eq!(m.cpu.csr_read(0x342), Some(3));
    assert_eq!(m.cpu.csr_read(0x341), Some(IRAM + 12));
    assert_eq!(m.cpu.csr_read(0x343), Some(IRAM + 12));
    assert_eq!(m.cpu.csr_read(0x300), Some(0x1800), "MPIE copies MIE (0), MIE cleared");
}

#[test]
fn step_and_cycle_budget() {
    let mut m = machine();
    put(&mut m, IRAM, &[addi(10, 0, 1), addi(10, 10, 1), addi(10, 10, 1), addi(10, 10, 1), EBREAK]);
    m.set_pc(IRAM);
    assert_eq!(m.step(), StopReason::Cycles);
    assert_eq!((m.cpu.x[10], m.cpu.pc), (1, IRAM + 4));
    assert_eq!(m.run(2), StopReason::Cycles);
    assert_eq!((m.cpu.x[10], m.cpu.cycles), (3, 3));
    assert_eq!(m.run(100), StopReason::Ebreak);
    assert_eq!(m.cpu.x[10], 4);
}

#[test]
fn mepc_with_bit_one_resumes_at_a_two_byte_boundary() {
    let mut m = machine();
    put(&mut m, IRAM, &[lui(5, 0x4037c), addi(5, 5, 0x83), csrrw(0, 0x341, 5), MRET]);
    // code at +0x82: addi a0, x0, 5 ; ebreak (both 32-bit, 2-byte aligned)
    m.bus.load(IRAM + 0x82, &addi(10, 0, 5).to_le_bytes());
    m.bus.load(IRAM + 0x86, &EBREAK.to_le_bytes());
    m.set_pc(IRAM);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.x[10], 5);
    assert_eq!(m.cpu.csr_read(0x341), Some(IRAM + 0x82));
}

struct Hook {
    scratch: u32,
}

impl mcs_sim::riscv::CsrHook for Hook {
    fn read(&mut self, csr: u16, _cycles: u64) -> Option<u32> {
        match csr {
            0x7c0 => Some(self.scratch),
            0x800 => Some(0x55),
            _ => None,
        }
    }
    fn write(&mut self, csr: u16, value: u32) -> bool {
        if csr == 0x7c0 {
            self.scratch = value;
            true
        } else {
            false
        }
    }
}

#[test]
fn csr_hook_provides_custom_csrs() {
    let mut m = machine();
    m.csr_hook = Some(Box::new(Hook { scratch: 0 }));
    m.cpu.x[11] = 0x1234;
    put(
        &mut m,
        IRAM,
        &[
            lui(5, 0x4037c),
            addi(5, 5, 0x40),
            csrrw(0, 0x305, 5),
            csrrw(10, 0x7c0, 11),
            csrrs(12, 0x7c0, 0),
            csrrs(13, 0x800, 0),
            csrrw(0, 0x801, 11), // unknown to the hook: illegal instruction
        ],
    );
    put(&mut m, IRAM + 0x40, &[EBREAK]);
    m.set_pc(IRAM);
    run_ebreak(&mut m);
    assert_eq!((m.cpu.x[10], m.cpu.x[12], m.cpu.x[13]), (0, 0x1234, 0x55));
    assert_eq!(m.cpu.csr_read(0x342), Some(2));
    assert_eq!(m.cpu.csr_read(0x343), Some(csrrw(0, 0x801, 11)));
    assert_eq!(m.cpu.csr_read(0x341), Some(IRAM + 24));
}

#[test]
fn interrupt_line_api() {
    let mut m = machine();
    m.set_irq(0);
    m.set_irq(32);
    assert_eq!(m.irq_pending(), 0);
    m.set_irq(1);
    m.set_irq(31);
    assert_eq!(m.irq_pending(), 0x8000_0002);
    m.clear_irq(1);
    assert_eq!(m.irq_pending(), 0x8000_0000);
    m.set_irq_line(7, true);
    m.set_irq_line(31, false);
    assert_eq!(m.irq_pending(), 0x80);
    m.set_irq_pending(0xffff_ffff);
    assert_eq!(m.irq_pending(), 0xffff_fffe, "line 0 does not exist");
    m.set_irq_pending(0);
    assert_eq!(m.irq_pending(), 0);
}

#[test]
fn device_raised_interrupt_is_taken_at_the_next_boundary_and_reset_keeps_mip() {
    let mut m = machine();
    put(
        &mut m,
        IRAM,
        &[
            lui(5, 0x4037c),
            addi(5, 5, 0x40),
            csrrw(0, 0x305, 5),   // mtvec = handler
            addi(6, 0, 0x100),    // line 8
            csrrw(0, 0x304, 6),   // mie
            addi(7, 0, 8),
            csrrs(0, 0x300, 7),   // MIE = 1
            lui(8, IRQ_DEV >> 12),
            addi(6, 0, 0x100),
            sw(6, 8, 0),          // raise line 8
            addi(10, 0, 1),       // never executed before the handler
            EBREAK,
        ],
    );
    put(&mut m, IRAM + 0x40, &[EBREAK]);
    m.set_pc(IRAM);
    run_ebreak(&mut m);
    assert_eq!(m.cpu.pc, IRAM + 0x44);
    assert_eq!(m.cpu.x[10], 0);
    assert_eq!(m.cpu.csr_read(0x342), Some(0x8000_0008));
    assert_eq!(m.cpu.csr_read(0x341), Some(IRAM + 40));
    assert_eq!(m.irq_pending(), 0x100);
    m.reset();
    assert_eq!(m.cpu.pc, 0x4200_0000);
    assert_eq!((m.cpu.cycles, m.cpu.instret, m.cpu.x[5]), (0, 0, 0));
    assert_eq!(m.cpu.csr_read(0x300), Some(0x1800));
    assert_eq!(m.irq_pending(), 0x100, "interrupt sources outlive a hart reset");
}

#[test]
fn bus_mapping_rules() {
    let mut bus = Bus::new();
    let a = bus.add_mem(0x3000);
    assert_eq!(bus.mem_size(a), 0x3000);
    assert!(bus.map(0x1000_0000, 0x3000, a, 0, PERM_RW).is_ok());
    assert!(bus.map(0x1000_2000, 0x1000, a, 0, PERM_RW).is_err(), "overlap");
    assert!(bus.map(0x2000_0100, 0x1000, a, 0, PERM_RW).is_err(), "unaligned base");
    assert!(bus.map(0x2000_0000, 0x4000, a, 0, PERM_RW).is_err(), "larger than the memory");
    assert!(bus.map(0x2000_0000, 0x1000, a, 0x800, PERM_RW).is_err(), "unaligned memory offset");
    assert!(bus.map(0x2000_0000, 0, a, 0, PERM_RW).is_err(), "empty window");
    assert!(bus.map(0xffff_f000, 0x2000, a, 0, PERM_RX).is_err(), "wraps the address space");
    assert!(bus.add_device(0x1000_1000, 0x1000, Box::new(super::IrqCtl)).is_err(), "device over memory");
    assert!(bus.add_device(0x3000_0000, 0x100, Box::new(super::IrqCtl)).is_err(), "unaligned size");
    assert!(bus.add_device(0x3000_0000, 0x1000, Box::new(super::IrqCtl)).is_ok());
    // plain accesses
    assert_eq!(bus.write(0x1000_0002, 2, 0xbeef, 0), Ok(()));
    assert_eq!(bus.read(0x1000_0000, 4, 0), Ok(0xbeef_0000));
    assert!(bus.read(0x1000_3000, 4, 0).is_err());
    assert!(bus.read(0x0, 1, 0).is_err());
    assert_eq!(bus.peek(0x1000_0003, 1), Some(0xbe));
    assert_eq!(bus.peek(0x3000_0000, 4), None);
}

#[test]
fn unmapped_machine_traps_instead_of_panicking() {
    let mut m = Machine::new(RvConfig::default());
    for _ in 0..3 {
        m.step();
    }
    // fetch fault at the reset vector; mtvec = 0 keeps trapping at address 0
    assert_eq!(m.cpu.csr_read(0x342), Some(1));
    assert_eq!(m.cpu.csr_read(0x343), Some(0));
    assert!(m.cpu.cycles >= 3);
}

/// Throughput smoke test (not an assertion on speed): a tight loop of 3 instructions runs.
#[test]
fn tight_loop_runs_to_completion() {
    let mut m = machine();
    // a0 = 2_000_000; loop: addi a0,a0,-1 ; add a1,a1,a0 ; bne a0,x0,loop ; ebreak
    let off = -8i32 as u32;
    let bne = ((off >> 12 & 1) << 31) | ((off >> 5 & 0x3f) << 25) | (10 << 15) | (1 << 12) | ((off >> 1 & 0xf) << 8) | ((off >> 11 & 1) << 7) | 0x63;
    let add = (10 << 20) | (11 << 15) | (11 << 7) | 0x33;
    put(&mut m, IRAM, &[lui(10, 0x1e8), addi(10, 10, 0x480), addi(10, 10, -1), add, bne, EBREAK]);
    m.set_pc(IRAM);
    let t = std::time::Instant::now();
    run_ebreak_budget(&mut m);
    let dt = t.elapsed();
    eprintln!("{} instructions in {:?} = {:.0} MIPS", m.cpu.instret, dt, m.cpu.instret as f64 / dt.as_secs_f64() / 1e6);
    assert_eq!(m.cpu.x[10], 0);
}

fn run_ebreak_budget(m: &mut Machine) {
    assert_eq!(m.run(u64::MAX / 2), StopReason::Ebreak);
}

#[test]
fn bit_zero_of_the_pc_is_ignored() {
    let mut m = machine();
    put(&mut m, IRAM, &[addi(10, 0, 8), EBREAK]);
    m.set_pc(IRAM + 1);
    assert_eq!(m.cpu.pc, IRAM);
    m.cpu.pc = IRAM + 1; // set behind the API's back
    run_ebreak(&mut m);
    assert_eq!(m.cpu.x[10], 8);
}
