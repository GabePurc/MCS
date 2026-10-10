//! ARMv7E-M DSP extension and floating-point tests (programs `dsp.s`, `fp.s`, `fpd.s`,
//! `fpirq.s`). Expected values are derived by hand from the ARM ARM (DDI 0403E.e) pseudocode and
//! IEEE 754; the comments in the programs show which `out` produces which value.

use super::*;
use mcs_core::arm::thumb::ArmFeatures;

fn run_to(m: &mut Machine, p: &Prog, label: &str) {
    assert_eq!(m.run(100_000_000), StopReason::Bkpt, "pc={:#x}", m.cpu.pc);
    assert_eq!(m.cpu.pc - 2, p.sym(label), "stopped at {:#x}", m.cpu.pc - 2);
}

fn boot_cfg(p: &Prog, cfg: ArmConfig, entry: &str) -> Machine {
    let mut m = Machine::new(cfg);
    m.load_image(p.code);
    m.cpu.pc = p.sym(entry);
    m
}

#[test]
fn dsp_parallel_saturating_packing_and_multiply_instructions() {
    let mut m = boot(&DSP);
    run_to_done(&mut m, &DSP);
    #[rustfmt::skip]
    let want = [
        0x0000_0507, 0xc,                                  // UADD8 + GE (lanes 2, 3 carry)
        0xffff_0002, 0x3,                                  // SSUB16
        0x0005_0003, 0xf,                                  // SASX
        0x8000_0002, 0xf,                                  // SADD16 wraps, GE from the signed sum
        0x7fff_8001, 0xf,                                  // QADD16 saturates, GE unchanged
        0x0010_1000,                                       // UQSUB8
        0x7fff_0003,                                       // SHADD16
        0xfeff_0001,                                       // UHSUB8
        0x0001_0008, 0xc,                                  // USAX
        0x1122_ccdd,                                       // SEL with GE = 0b1100
        8, 0x108,                                          // USAD8 / USADA8
        0x007f_ff80, 1, 0x00ff_0000, 1, 0x0005_0003, 0,    // SSAT16 / USAT16 and the Q flag
        0xbbbb_1111, 0xaaaa_2222, 0xaaaa_0000,             // PKHBT / PKHTB
        0xff80_ff80, 0x0034_0078, 0x0012_0056, 0xff81_ff81, 0x0100_0100, // SXTB16 UXTB16 SXTAB16 UXTAB16
        21, 15, 0xffff_fff2, 0xffff_fff6, 121, 0x8000_0014, 1, // SMULxy / SMLABB (Q on overflow)
        0xffff_8000, 3, 4,                                 // SMULWB / SMULWT / SMLAWT
        0x16, 1, 0xffff_fff7, 0xffff_ffff,                 // SMLALBB / SMLALTT
        23, 22, 0xffff_fff9, 0xffff_fffe, 123, 8, 0x8000_0000, 1, // SMUAD(X) SMUSD(X) SMLAD SMLSDX, Q
        0x18, 0, 0xffff_fffa, 0xffff_ffff,                 // SMLALD / SMLSLD
        0x1000_0000, 0, 1, 5, 4, 4,                        // SMMUL SMMULR SMMLA SMMLS SMMLSR
        0xffff_ffff, 0xffff_ffff,                          // UMAAL
        0x0005_0000, 0xf805_0000, 0x000a_0000,             // APSR GE / NZCVQ access
    ];
    assert_eq!(results(&mut m, want.len()), want);
    assert_eq!(m.cpu.r[12], 0x2000_0100 + 4 * want.len() as u32);
}

#[test]
fn dsp_instructions_are_undefined_on_a_cortex_m3() {
    let mut m = boot_cfg(&DSP, ArmConfig::cortex_m3(), "reset");
    // The first DSP instruction (UADD8) is an undefined instruction: forced HardFault -> `dflt`.
    run_to(&mut m, &DSP, "dflt");
    assert_ne!(m.scb.cfsr & (1 << 16), 0);
}

#[test]
fn single_precision_arithmetic_conversions_and_flags() {
    let mut m = boot(&FP);
    run_to_done(&mut m, &FP);
    #[rustfmt::skip]
    let want: [u32; 138] = [
        0, 4,                                                                     // FPCA before / after the first FP instruction
        0x4070_0000, 0, 0xbf40_0000, 0x4058_0000, 0xc058_0000,                    // + - * nmul
        0x3eaa_aaab, 0x10, 0x3fb5_04f3, 0x10, 0x4000_0000, 0, 0x4000_0000, 0xbfc0_0000, // / sqrt abs neg
        0, 0x10, 0x3380_0000, 0, 0xb380_0000, 0x3380_0000, 0xc000_1000, 0, 0,     // VMLA / fused family
        0x3eaa_aaab, 0x3eaa_aaab, 0x3eaa_aaaa, 0x3eaa_aaaa,                       // 1/3 in RN RP RM RZ
        0, 0x8000_0000, 0x7f80_0000, 0x14, 0x7f7f_ffff, 0x4090_0000,              // signed zero, overflow
        2, 0x10, 3, 2, 4, 3, 0xffff_fffe,                                         // float -> s32
        0x7fff_ffff, 1, 0xffff_ffff, 1, 0x8000_0000, 0, 0, 1, 0, 1, 0, 0x10,      // saturation, NaN, negative
        0xc0a0_0000, 0x4f80_0000, 0x10, 0x4b80_0000, 0x4b80_0001,                 // int -> float
        0x28, 0x280, 0x7fff, 1, 0xffff_8000, 0x3fc0_0000, 0xbd80_0000,            // fixed point
        0xdead_3e00, 0x3e00_3e00, 0x3fc0_0000, 0x3fc0_0000, 0x3e00_7c00, 0x14,    // half precision
        8, 2, 6, 2, 3, 0, 3, 1, 3, 1, 1, 1, 0x2000_0000,                          // compares
        0x7fc0_0000, 1, 0x7fc0_0000, 1, 0x7fc0_1234, 0, 0x7fc0_0001, 1, 0x7fc0_0000, // NaN handling
        2, 0, 0, 0x80,                                                            // denormals, FZ
        0x0040_0000, 0x18, 0x0040_0000, 0, 0, 8,                                  // underflow
        0x7f80_0000, 2, 0xff80_0000, 0x7fc0_0000, 1, 0x7fc0_0000, 1, 0x8000_0000, // divide by zero, sqrt
        0xffc0_0000, 0x7fc0_0000, 0,                                              // VNEG / VABS of a NaN
        0x3f80_0000, 0xbf00_0000, 0x41f8_0000, 0x3e00_0000, 0x1234_5678,          // VMOV
        0x1111_1111, 0x2222_2222, 0x1111_1111, 0x2222_2222, 0xf7c0_009f,
        0x1011_0021, 0x1100_0011,                                                 // MVFR0 / MVFR1 of the Cortex-M4F
        0xcafe_0001, 0xcafe_0001, 0x4049_0fdb, 0x2000_0410, 0x44, 0x11, 0x22,    // VSTR / VLDR / VSTM / VLDM
        64, 0xaaaa_0016, 0xaaaa_001f, 0, 0,                                       // VPUSH / VPOP, CONTROL
    ];
    assert_eq!(results(&mut m, want.len()), want, "(index of the first difference is the out number)");
}

#[test]
fn fp_instructions_fault_with_nocp_until_the_fpu_is_enabled() {
    let mut m = boot_at(&FP, "nocp");
    run_to(&mut m, &FP, "done_nocp");
    assert_eq!(results(&mut m, 4), [0, 1, 0x0008_0000, FP.sym("nocp_site")]);
    // CPACR = 0b01 (privileged only): privileged code runs FP instructions, unprivileged code faults.
    let mut m = boot(&FP);
    m.scb.cpacr = 0x0050_0000;
    let code: [u16; 3] = [0xee30, 0x0a81, 0xbe00]; // vadd.f32 s0, s1, s2; bkpt
    for (k, hw) in code.iter().enumerate() {
        assert!(m.mem_write(0x2000_1000 + 2 * k as u32, 2, *hw as u32));
    }
    m.cpu.pc = 0x2000_1000;
    m.cpu.fpr[1] = 1.5f32.to_bits();
    m.cpu.fpr[2] = 2.25f32.to_bits();
    assert_eq!(m.run(1000), StopReason::Bkpt);
    assert_eq!(m.cpu.fpr[0], 3.75f32.to_bits());
    let mut m = boot(&FP);
    m.scb.cpacr = 0x0050_0000;
    m.cpu.control |= 1; // CONTROL.nPRIV
    for (k, hw) in code.iter().enumerate() {
        assert!(m.mem_write(0x2000_1000 + 2 * k as u32, 2, *hw as u32));
    }
    m.cpu.pc = 0x2000_1000;
    m.run(1000);
    assert_ne!(m.scb.cfsr & (1 << 19), 0, "NOCP for unprivileged access");
    // Without an FPU the FP encodings are undefined instructions instead.
    let mut m = Machine::new(ArmConfig { features: ArmFeatures::DSP, ..ArmConfig::default() });
    m.load_image(FP.code);
    for (k, hw) in code.iter().enumerate() {
        assert!(m.mem_write(0x2000_1000 + 2 * k as u32, 2, *hw as u32));
    }
    m.cpu.pc = 0x2000_1000;
    m.run(1000);
    assert_ne!(m.scb.cfsr & (1 << 16), 0, "UNDEFINSTR without an FPU");
}

#[test]
fn double_precision_fpv5_additions_and_aliasing() {
    let mut m = boot_cfg(&FPD, ArmConfig::cortex_m7(), "reset");
    run_to_done(&mut m, &FPD);
    #[rustfmt::skip]
    let want: [u32; 129] = [
        0, 0x400e_0000, 0, 0xbfe8_0000, 0, 0x400b_0000, 0,                        // + - * (lo, hi pairs), flags
        0x5555_5555, 0x3fd5_5555, 0x10, 0x667f_3bcd, 0x3ff6_a09e, 0x10,           // 1/3 and sqrt(2)
        0, 0, 0, 0x3c90_0000,                                                     // non-fused 0 / fused 2^-54
        0x5555_5556, 0x3fd5_5555, 0x5555_5555, 0x3fd5_5555,                       // 1/3 toward +inf / -inf
        0x1111_1111, 0x2222_2222, 0x3333_3333, 0x1111_1111, 0x3333_3333, 0x2222_2222, 0x2222_2222, // D/S aliasing, VMOV.32
        0, 0x3ff8_0000, 0x8000_0000, 0x7ff8_0246,                                 // f32 -> f64 (NaN payload moves up)
        0x3eaa_aaab, 0x10, 2, 0x10, 0xffff_ffff, 1,                               // f64 -> f32, s32, u32 saturation
        0, 0xc014_0000, 0xffe0_0000, 0x41ef_ffff, 0,                              // s32/u32 -> f64
        384, 0, 0x3ff8_0000, 0, 0x3ff8_0000, 0x3e00_0000,                         // fixed point, half precision
        0, 0x3ff0_0000, 0, 0x3ff0_0000, 0, 0, 0, 0, 0x8000_0000, 0x4000_0000, 0x4040_0000, // VMAXNM / VMINNM
        0, 0xbff0_0000, 0, 0x4000_0000, 0, 0xc000_0000, 0, 0x4008_0000, 0, 0x4000_0000, // VRINTZ P M A N
        0, 0x4000_0000, 0, 0, 0x4000_0000, 0x10, 0x4000_0000,                     // VRINTR (no IXC), VRINTX (IXC), f32
        3, 2, 2, 0xffff_fffe, 1, 0xffff_fffe, 0x10,                               // VCVTA N P M
        0, 0x4000_0000, 0, 0x4000_0000, 0, 0x4000_0000, 0, 0x4000_0000,          // VSEL after "less"
        0, 0x3ff0_0000, 0, 0x4000_0000, 0, 0x4000_0000, 3, 0x3f80_0000,           // VSEL after "equal", unordered
        0, 0x7ff8_0000, 1, 0, 0x7ff0_0000, 2,                                     // inf - inf, 1/0
        0x1234, 0x7ff8_0000, 2, 0, 0, 0, 0, 0x80,                                 // payload, denormal add, FZ
        0, 0x7ff0_0000, 0x14,                                                     // overflow
        0x89ab_cdef, 0x0123_4567, 0x89ab_cdef, 0x0123_4567,                       // VSTR / VLDR
        64, 8, 8, 0xf, 0xf,                                                       // VPUSH / VPOP {d8-d15}
        0x1011_0221, 0x1200_0011, 0x40,                                           // MVFR0-2 of the Cortex-M7
    ];
    assert_eq!(results(&mut m, want.len()), want);
}

#[test]
fn fpv5_encodings_are_undefined_on_the_single_precision_fpu() {
    // vmaxnm.f32 s0, s1, s2 (0xfe80 0a81) is an FPv5 instruction: UNDEFINSTR on the Cortex-M4F.
    let mut m = boot(&FP);
    m.scb.cpacr = 0x00f0_0000;
    for (k, hw) in [0xfe80u16, 0x0a81, 0xbe00].iter().enumerate() {
        assert!(m.mem_write(0x2000_1000 + 2 * k as u32, 2, *hw as u32));
    }
    m.cpu.pc = 0x2000_1000;
    m.run(1000);
    assert_ne!(m.scb.cfsr & (1 << 16), 0);
}

#[test]
fn interrupt_during_fp_code_stacks_and_restores_the_extended_frame() {
    let mut m = boot(&FPIRQ);
    let handler = FPIRQ.sym("irq0_h");
    let mut n = 0;
    while m.cpu.pc != handler {
        assert_eq!(m.step(), StopReason::Limit);
        n += 1;
        assert!(n < 10_000);
    }
    // 26-word frame: R0-R3, R12, LR, PC, xPSR, S0-S15, FPSCR, reserved.
    assert_eq!(m.cpu.r[13], 0x2000_8000 - 0x68);
    assert_eq!(m.cpu.r[14], 0xffff_ffe9, "EXC_RETURN: thread, MSP, extended frame");
    assert_eq!(m.cpu.control & 4, 0, "FPCA is cleared on entry");
    assert_eq!(m.cpu.fpscr, 0, "FPSCR takes the FPDSCR default");
    let sp = m.cpu.r[13];
    let f = words(&mut m, sp, 26);
    assert_eq!(f[0], 0xe000_e200, "R0 at the time of the interrupt");
    assert_eq!(f[1], 1);
    assert_eq!(f[7] & 0x1ff, 0, "xPSR of the thread");
    let s: Vec<u32> = (0..16).map(|k| 0x3f80_0000 + k).collect();
    assert_eq!(&f[8..24], &s[..]);
    assert_eq!(f[24], 0x6040_0011, "stacked FPSCR");
    run_to_done(&mut m, &FPIRQ);
    assert_eq!(words(&mut m, 0x2000_0300, 6), [0, 0xffff_ffe9, 0x2000_8000 - 0x68, 0, 0, 4]);
    // After the return everything is back: S0-S31, FPSCR, FPCA, SP and the core registers.
    let d = words(&mut m, 0x2000_0500, 35);
    let want_s: Vec<u32> = (0..32).map(|k| 0x3f80_0000 + k).collect();
    assert_eq!(&d[..32], &want_s[..]);
    assert_eq!(d[32], 0x6040_0011);
    assert_eq!(d[33], 4);
    assert_eq!(d[34], 0x2000_8000);
    assert_eq!((m.cpu.r[0], m.cpu.r[1]), (0xe000_e200, 1), "core registers restored");
}

#[test]
fn interrupt_without_fp_context_uses_the_basic_frame() {
    let mut m = boot_at(&FPIRQ, "std_frame");
    run_to(&mut m, &FPIRQ, "done_std");
    assert_eq!(words(&mut m, 0x2000_0300, 3), [0, 0xffff_fff9, 0x2000_8000 - 0x20]);
    // Thread SP before / after the interrupt and CONTROL.FPCA afterwards (cleared by the return).
    assert_eq!(words(&mut m, 0x2000_0600, 3), [0x2000_8000, 0x2000_8000, 0]);
}

#[test]
fn nested_exceptions_each_stack_the_fp_context() {
    let mut m = boot_at(&FPIRQ, "nested");
    let mut lowest = 0x2000_8000u32;
    let mut guard = 0;
    loop {
        match m.step() {
            StopReason::Limit => {}
            StopReason::Bkpt => break,
            r => panic!("unexpected stop {r:?}"),
        }
        lowest = lowest.min(m.cpu.r[13]);
        guard += 1;
        assert!(guard < 100_000);
    }
    assert_eq!(lowest, 0x2000_8000 - 0x68 - 0x68, "two extended frames");
    // IRQ0 entry: FPCA clear, EXC_RETURN with the FP bit; S0 / FPSCR of the handler survive IRQ1.
    assert_eq!(words(&mut m, 0x2000_0300, 9), [0, 0xffff_ffe9, 0x2000_8000 - 0x68, 1, 0, 0, 0x4040_0000, 0x0040_0000, 4]);
    // IRQ1 entered from a handler that used FP: handler-mode extended frame.
    assert_eq!(words(&mut m, 0x2000_0340, 2), [0, 0xffff_ffe1]);
    let d = words(&mut m, 0x2000_0500, 35);
    let want_s: Vec<u32> = (0..32).map(|k| 0x3f80_0000 + k).collect();
    assert_eq!(&d[..32], &want_s[..]);
    assert_eq!(d[32], 0x2040_0001);
    assert_eq!(d[33], 4);
    assert_eq!(d[34], 0x2000_8000);
}

#[test]
fn fp_instruction_cycle_counts() {
    // vadd (1) + vdiv (14) + vsqrt (14) + vmla (3) + vldr (2) + vpush of 4 registers (1 + 4)
    let code: [u16; 14] = [
        0xee30, 0x0a81, // vadd.f32 s0, s1, s2
        0xee80, 0x0a81, // vdiv.f32 s0, s1, s2
        0xeeb1, 0x0ae0, // vsqrt.f32 s0, s1
        0xee00, 0x0a81, // vmla.f32 s0, s1, s2
        0xed90, 0x0a00, // vldr s0, [r0]
        0xed2d, 0x8a04, // vpush {s16-s19}
        0xbe00, 0xbf00,
    ];
    let mut m = boot(&FP);
    m.scb.cpacr = 0x00f0_0000;
    for (k, hw) in code.iter().enumerate() {
        assert!(m.mem_write(0x2000_1000 + 2 * k as u32, 2, *hw as u32));
    }
    m.cpu.pc = 0x2000_1000;
    m.cpu.r[0] = 0x2000_2000;
    m.cpu.fpr[1] = 1.5f32.to_bits();
    m.cpu.fpr[2] = 2.25f32.to_bits();
    let start = m.cpu.cycles;
    assert_eq!(m.run(10_000), StopReason::Bkpt);
    assert_eq!(m.cpu.cycles - start, 1 + 14 + 14 + 3 + 2 + 5 + 1);
}

#[test]
fn fpca_follows_aspen_and_can_be_set_by_software() {
    let code: [u16; 3] = [0xee30, 0x0a81, 0xbe00]; // vadd.f32 s0, s1, s2; bkpt
    let run = |aspen: bool| {
        let mut m = boot(&FP);
        m.scb.cpacr = 0x00f0_0000;
        if !aspen {
            m.scb.fpccr = 0;
        }
        for (k, hw) in code.iter().enumerate() {
            assert!(m.mem_write(0x2000_1000 + 2 * k as u32, 2, *hw as u32));
        }
        m.cpu.pc = 0x2000_1000;
        assert_eq!(m.run(1000), StopReason::Bkpt);
        m
    };
    assert_eq!(run(true).cpu.control & 4, 4, "FPCCR.ASPEN: an FP instruction sets CONTROL.FPCA");
    assert_eq!(run(false).cpu.control & 4, 0, "without ASPEN software manages FPCA");
    // The FPCCR / FPDSCR / MVFR registers are visible in the System Control Space.
    let mut m = boot(&FP);
    assert_eq!(m.mem_read(0xe000_ef34, 4), Some(0xc000_0000));
    assert!(m.mem_write(0xe000_ef3c, 4, 0xffff_ffff));
    assert_eq!(m.mem_read(0xe000_ef3c, 4), Some(0x07c0_0000));
    assert_eq!(m.mem_read(0xe000_ef40, 4), Some(0x1011_0021));
    // A Cortex-M3 has none of them.
    let mut m3 = Machine::new(ArmConfig::cortex_m3());
    assert_eq!(m3.mem_read(0xe000_ef34, 4), Some(0));
}
