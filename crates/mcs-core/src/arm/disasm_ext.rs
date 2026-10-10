//! UAL text of the DSP-extension and floating-point operations (used by [`super::disasm`]).
//!
//! The conventions follow `llvm-objdump --triple=thumbv7em-none-eabihf`: the data type suffix
//! (`.f32`) comes after the IT condition, `vmov.f32 s0, #1.000000e+00` prints the immediate in C
//! `%e` style, and `vpush`/`vpop` list every register.

use super::disasm::reg_name;
use super::thumb::*;
use super::vfp::*;

fn r(n: u8) -> &'static str {
    reg_name(n)
}

const PFX: [&str; 6] = ["s", "q", "sh", "u", "uq", "uh"];
const KIND: [&str; 6] = ["add16", "asx", "sax", "sub16", "add8", "sub8"];

/// Half selector letter: `t` (top) or `b` (bottom).
fn tb(top: bool) -> char {
    if top {
        't'
    } else {
        'b'
    }
}

/// (mnemonic, operands) of a DSP-extension operation.
pub(super) fn dsp_text(i: &Insn) -> (String, String) {
    let rrr = || format!("{}, {}, {}", r(i.rd), r(i.rn), r(i.rm));
    let rrrr = || format!("{}, {}, {}, {}", r(i.rd), r(i.rn), r(i.rm), r(i.ra));
    let long = || format!("{}, {}, {}, {}", r(i.rd), r(i.ra), r(i.rn), r(i.rm));
    let xy = |base: &str| format!("{}{}{}", base, tb(i.aux & 1 != 0), tb(i.aux & 2 != 0));
    let sfx = |on: bool, c: &'static str| if on { c } else { "" };
    let rot = if i.amt != 0 { format!(", ror #{}", i.amt) } else { String::new() };
    match i.op {
        Op::PAR => (format!("{}{}", PFX[i.aux as usize % 6], KIND[i.shift as usize % 6]), rrr()),
        Op::SEL => ("sel".into(), rrr()),
        Op::USAD8 => ("usad8".into(), rrr()),
        Op::USADA8 => ("usada8".into(), rrrr()),
        Op::SSAT16 => ("ssat16".into(), format!("{}, #{}, {}", r(i.rd), i.imm, r(i.rn))),
        Op::USAT16 => ("usat16".into(), format!("{}, #{}, {}", r(i.rd), i.imm, r(i.rn))),
        Op::PKH => {
            let shift = if i.shift == SH_ASR { format!(", asr #{}", i.amt) } else if i.amt != 0 { format!(", lsl #{}", i.amt) } else { String::new() };
            (if i.shift == SH_ASR { "pkhtb" } else { "pkhbt" }.into(), format!("{}{}", rrr(), shift))
        }
        Op::SXTB16 | Op::UXTB16 => {
            let base = if i.op == Op::SXTB16 { "sxt" } else { "uxt" };
            if i.rn == 15 {
                (format!("{}b16", base), format!("{}, {}{}", r(i.rd), r(i.rm), rot))
            } else {
                (format!("{}ab16", base), format!("{}, {}, {}{}", r(i.rd), r(i.rn), r(i.rm), rot))
            }
        }
        Op::SMUL_XY => (xy("smul"), rrr()),
        Op::SMLA_XY => (xy("smla"), rrrr()),
        Op::SMULW => (format!("smulw{}", tb(i.aux & 2 != 0)), rrr()),
        Op::SMLAW => (format!("smlaw{}", tb(i.aux & 2 != 0)), rrrr()),
        Op::SMLAL_XY => (xy("smlal"), long()),
        Op::SMUAD => (format!("smuad{}", sfx(i.aux & 1 != 0, "x")), rrr()),
        Op::SMUSD => (format!("smusd{}", sfx(i.aux & 1 != 0, "x")), rrr()),
        Op::SMLAD => (format!("smlad{}", sfx(i.aux & 1 != 0, "x")), rrrr()),
        Op::SMLSD => (format!("smlsd{}", sfx(i.aux & 1 != 0, "x")), rrrr()),
        Op::SMLALD => (format!("smlald{}", sfx(i.aux & 1 != 0, "x")), long()),
        Op::SMLSLD => (format!("smlsld{}", sfx(i.aux & 1 != 0, "x")), long()),
        Op::SMMUL => (format!("smmul{}", sfx(i.aux & 1 != 0, "r")), rrr()),
        Op::SMMLA => (format!("smmla{}", sfx(i.aux & 1 != 0, "r")), rrrr()),
        Op::SMMLS => (format!("smmls{}", sfx(i.aux & 1 != 0, "r")), rrrr()),
        _ => ("umaal".into(), long()),
    }
}

/// S or D register name.
pub fn fp_reg(n: u8, dp: bool) -> String {
    if dp {
        format!("d{}", n)
    } else {
        format!("s{}", n)
    }
}

/// `{s0, s1, ...}` / `{d0, d1, ...}` list of `count` registers from `first`.
fn fp_list(first: u8, count: u32, dp: bool) -> String {
    let mut s = String::from("{");
    for k in 0..count as u8 {
        if k != 0 {
            s.push_str(", ");
        }
        s.push_str(&fp_reg(first + k, dp));
    }
    s.push('}');
    s
}

/// C `%e` formatting (six fraction digits, at least two exponent digits).
pub fn fmt_e(v: f64) -> String {
    let s = format!("{:.6e}", v);
    match s.split_once('e') {
        Some((m, e)) => {
            let (sign, digits) = match e.strip_prefix('-') {
                Some(d) => ('-', d),
                None => ('+', e),
            };
            format!("{}e{}{:0>2}", m, sign, digits)
        }
        None => s,
    }
}

fn fsuffix(dp: bool) -> &'static str {
    if dp {
        ".f64"
    } else {
        ".f32"
    }
}

fn int_type(signed: bool, bits: u32) -> String {
    format!("{}{}", if signed { 's' } else { 'u' }, bits)
}

/// (mnemonic, data type suffix, operands) of a floating-point operation.
pub(super) fn fp_text(i: &Insn) -> (String, String, String) {
    let dp = i.aux & 1 != 0;
    let f = fsuffix(dp);
    let reg = |n: u8| fp_reg(n, dp);
    let sx = |n: u8| fp_reg(n, false);
    let three = || format!("{}, {}, {}", reg(i.rd), reg(i.rn), reg(i.rm));
    let two = || format!("{}, {}", reg(i.rd), reg(i.rm));
    let mem = |ops: &mut String| {
        let off = i.imm as i32;
        *ops = if off == 0 {
            format!("{}, [{}]", reg(i.rd), r(i.rn))
        } else {
            format!("{}, [{}, #{}]", reg(i.rd), r(i.rn), off)
        };
    };
    let name = OP_NAMES[i.op as usize];
    let mut ops = String::new();
    let mut mn = name.to_string();
    let mut dt = String::new();
    match i.op {
        Op::VLDR | Op::VSTR => mem(&mut ops),
        Op::VLDM | Op::VSTM => {
            mn = format!("{}{}", name, if i.aux & 4 != 0 { "db" } else { "ia" });
            ops = format!("{}{}, {}", r(i.rn), if i.aux & 2 != 0 { "!" } else { "" }, fp_list(i.rd, i.imm, dp));
        }
        Op::VPUSH | Op::VPOP => ops = fp_list(i.rd, i.imm, dp),
        Op::VMOV_I => {
            dt = f.into();
            let v = if dp { f64::from_bits((i.imm as u64) << 32) } else { f32::from_bits(i.imm) as f64 };
            ops = format!("{}, #{}", reg(i.rd), fmt_e(v));
        }
        Op::VMOV_F => {
            dt = f.into();
            ops = two();
        }
        Op::VMOV_RS => ops = format!("{}, {}", r(i.rd), sx(i.rm)),
        Op::VMOV_SR => ops = format!("{}, {}", sx(i.rm), r(i.rd)),
        Op::VMOV_2S => {
            ops = if i.aux & 1 != 0 {
                format!("{}, {}, {}, {}", r(i.rd), r(i.rn), sx(i.rm), sx(i.rm + 1))
            } else {
                format!("{}, {}, {}, {}", sx(i.rm), sx(i.rm + 1), r(i.rd), r(i.rn))
            };
        }
        Op::VMOV_D2 => {
            ops = if i.aux & 1 != 0 {
                format!("{}, {}, d{}", r(i.rd), r(i.rn), i.rm)
            } else {
                format!("d{}, {}, {}", i.rm, r(i.rd), r(i.rn))
            };
        }
        Op::VMOV_SC => {
            dt = ".32".into();
            ops = if i.aux & 1 != 0 { format!("{}, d{}[{}]", r(i.rd), i.rm, i.amt) } else { format!("d{}[{}], {}", i.rm, i.amt, r(i.rd)) };
        }
        Op::VMRS => {
            let reg_name = match i.imm {
                FPREG_FPSID => "fpsid",
                FPREG_FPSCR => "fpscr",
                FPREG_MVFR2 => "mvfr2",
                FPREG_MVFR1 => "mvfr1",
                _ => "mvfr0",
            };
            ops = format!("{}, {}", if i.rd == 15 { "APSR_nzcv" } else { r(i.rd) }, reg_name);
        }
        Op::VMSR => ops = format!("fpscr, {}", r(i.rd)),
        Op::VADD | Op::VSUB | Op::VMUL | Op::VNMUL | Op::VDIV | Op::VMLA | Op::VMLS | Op::VNMLA | Op::VNMLS | Op::VFMA | Op::VFMS | Op::VFNMA | Op::VFNMS | Op::VMAXNM | Op::VMINNM => {
            dt = f.into();
            ops = three();
        }
        Op::VABS | Op::VNEG | Op::VSQRT => {
            dt = f.into();
            ops = two();
        }
        Op::VCMP | Op::VCMPE => {
            dt = f.into();
            ops = if i.s == 1 { format!("{}, #0", reg(i.rn)) } else { format!("{}, {}", reg(i.rn), reg(i.rm)) };
        }
        Op::VSEL => {
            mn = format!("vsel{}", ["eq", "vs", "ge", "gt"][(i.s & 3) as usize]);
            dt = f.into();
            ops = three();
        }
        Op::VRINT => {
            mn = format!("vrint{}", ["r", "z", "x", "a", "n", "p", "m"][(i.s as usize).min(6)]);
            dt = f.into();
            ops = two();
        }
        Op::VCVT_FI => {
            mn = format!("vcvt{}", ["", "r", "a", "n", "p", "m"][(i.s as usize).min(5)]);
            dt = format!(".{}{}", int_type(i.ra != 0, 32), fsuffix(dp));
            ops = format!("{}, {}", sx(i.rd), reg(i.rm));
        }
        Op::VCVT_IF => {
            dt = format!("{}.{}", fsuffix(dp), int_type(i.ra != 0, 32));
            ops = format!("{}, {}", reg(i.rd), sx(i.rm));
        }
        Op::VCVT_FX => {
            let size = if i.aux & 4 != 0 { 32 } else { 16 };
            let fixed = int_type(i.ra != 0, size);
            dt = if i.aux & 2 != 0 { format!(".{}{}", fixed, fsuffix(dp)) } else { format!("{}.{}", fsuffix(dp), fixed) };
            ops = format!("{}, {}, #{}", reg(i.rd), reg(i.rd), i.imm);
        }
        Op::VCVT_DS => {
            if dp {
                dt = ".f64.f32".into();
                ops = format!("d{}, s{}", i.rd, i.rm);
            } else {
                dt = ".f32.f64".into();
                ops = format!("s{}, d{}", i.rd, i.rm);
            }
        }
        Op::VCVTB | Op::VCVTT => {
            let wide = if i.aux & 2 != 0 { ".f64" } else { ".f32" };
            if i.aux & 1 != 0 {
                dt = format!(".f16{}", wide);
                ops = format!("s{}, {}", i.rd, fp_reg(i.rm, i.aux & 2 != 0));
            } else {
                dt = format!("{}.f16", wide);
                ops = format!("{}, s{}", fp_reg(i.rd, i.aux & 2 != 0), i.rm);
            }
        }
        _ => {}
    }
    (mn, dt, ops)
}
