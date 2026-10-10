//! Thumb decoder/disassembler checked against `llvm-objdump --triple=thumbv7em` output (Apple
//! clang) for every ARMv7-M base encoding; see `vectors.rs` (generated, checked in).

mod vectors;
mod vectors_ext;

use mcs_core::arm::disasm::Disassembler;
use mcs_core::arm::thumb::{decode, ArmFeatures, Insn, Op};
use vectors::{LONG_BRANCH_VECTORS, VECTORS};
use vectors_ext::{M4F_VECTORS, M7_VECTORS};

/// Normalizes objdump / our text: drops `<sym>` parts and comments, converts `0x..` literals to
/// decimal and collapses whitespace.
fn norm(s: &str) -> String {
    let mut s = s.split('@').next().unwrap().to_string();
    while let (Some(a), Some(b)) = (s.find('<'), s.find('>')) {
        if a < b {
            s.replace_range(a..=b, "");
        } else {
            break;
        }
    }
    let mut out = String::new();
    let b = s.as_bytes();
    let mut k = 0;
    while k < b.len() {
        if b[k] == b'0' && k + 1 < b.len() && b[k + 1] == b'x' {
            let mut e = k + 2;
            while e < b.len() && b[e].is_ascii_hexdigit() {
                e += 1;
            }
            let v = u64::from_str_radix(&s[k + 2..e], 16).unwrap();
            out.push_str(&v.to_string());
            k = e;
        } else {
            out.push(b[k] as char);
            k += 1;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn run(stream: &[(u32, &[u8], &str)], feat: ArmFeatures) -> Vec<String> {
    let mut d = Disassembler::new(feat);
    let mut bad = Vec::new();
    for (addr, bytes, want) in stream {
        let (len, got) = d.next(bytes, *addr).expect("bytes");
        let ok = len == bytes.len() && norm(&got) == norm(want);
        if !ok {
            bad.push(format!("{:#x} {:02x?}: got `{}` want `{}`", addr, bytes, norm(&got), norm(want)));
        }
    }
    bad
}

#[test]
fn base_isa_matches_llvm_objdump() {
    let bad = run(VECTORS, ArmFeatures::CORTEX_M4F);
    if !bad.is_empty() {
        let n = bad.len();
        let max: usize = std::env::var("ARM_MAX").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
        panic!("{} of {} mismatches:\n{}", n, VECTORS.len(), bad.iter().take(max).cloned().collect::<Vec<_>>().join("\n"));
    }
}

fn hws(bytes: &[u8]) -> (u16, u16) {
    let hw1 = u16::from_le_bytes([bytes[0], bytes[1]]);
    let hw2 = if bytes.len() > 2 { u16::from_le_bytes([bytes[2], bytes[3]]) } else { 0 };
    (hw1, hw2)
}

fn report(what: &str, total: usize, bad: Vec<String>) {
    if !bad.is_empty() {
        let max: usize = std::env::var("ARM_MAX").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
        panic!("{what}: {} of {total} mismatches:\n{}", bad.len(), bad.iter().take(max).cloned().collect::<Vec<_>>().join("\n"));
    }
}

#[test]
fn dsp_and_fpv4_sp_match_llvm_objdump() {
    report("M4F", M4F_VECTORS.len(), run(M4F_VECTORS, ArmFeatures::CORTEX_M4F));
}

#[test]
fn fpv5_d16_matches_llvm_objdump() {
    report("M7", M7_VECTORS.len(), run(M7_VECTORS, ArmFeatures::CORTEX_M7));
    // The M4F encodings are a subset of what the M7 decodes.
    report("M4F on M7", M4F_VECTORS.len(), run(M4F_VECTORS, ArmFeatures::CORTEX_M7));
}

#[test]
fn extension_encodings_are_undefined_without_the_extension() {
    // DSP / FP encodings must not decode to a base instruction (they decode to UNDEF).
    for (_, bytes, text) in M4F_VECTORS.iter().chain(M7_VECTORS) {
        let (hw1, hw2) = hws(bytes);
        let i = decode(hw1, hw2, ArmFeatures::BASE);
        if bytes.len() == 4 && i.op != Op::UNDEF {
            // Only instructions that also exist in the base ISA may decode (e.g. a plain `qadd`).
            panic!("{} decoded as {:?} without the extension", text, i.op);
        }
    }
}

#[test]
fn double_precision_and_fpv5_are_undefined_on_the_single_precision_fpu() {
    const V5: [&str; 8] = ["vsel", "vmaxnm", "vminnm", "vrint", "vcvta", "vcvtn", "vcvtp", "vcvtm"];
    for (_, bytes, text) in M7_VECTORS {
        let dp_only = text.contains("f64") || text.contains("mvfr2") || text.contains(".32\t") || V5.iter().any(|p| text.starts_with(p));
        let (hw1, hw2) = hws(bytes);
        let op = decode(hw1, hw2, ArmFeatures::CORTEX_M4F).op;
        let wide = text.starts_with("vmov\t") && text.contains("d");
        if dp_only || wide {
            assert_eq!(op, Op::UNDEF, "{text} must be undefined on FPv4-SP");
        }
    }
}

#[test]
fn parallel_add_subtract_covers_all_36_forms() {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    for (_, bytes, _) in M4F_VECTORS {
        let (hw1, hw2) = hws(bytes);
        let i = decode(hw1, hw2, ArmFeatures::CORTEX_M4F);
        if i.op == Op::PAR {
            seen.insert((i.aux, i.shift));
        }
    }
    assert_eq!(seen.len(), 36);
}

#[test]
fn insn_is_small() {
    assert!(std::mem::size_of::<Insn>() <= 16);
}

#[test]
fn dsp_extend_and_add_decodes_with_dsp_feature() {
    let i = decode(0xfa01, 0xf082, ArmFeatures::CORTEX_M4F); // sxtah r0, r1, r2
    assert_eq!(i.op, Op::SXTH);
    assert_eq!((i.rd, i.rn, i.rm), (0, 1, 2));
}

#[test]
fn long_branches_match() {
    // Far targets (outside the stream, some wrapping below zero) exercise the high offset bits.
    let bad = run(LONG_BRANCH_VECTORS, ArmFeatures::CORTEX_M4F);
    assert!(bad.is_empty(), "{}", bad.iter().take(10).cloned().collect::<Vec<_>>().join("\n"));
}

#[test]
fn every_operation_is_covered_by_the_vectors() {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    for (_, bytes, _) in VECTORS.iter().chain(M4F_VECTORS) {
        let (hw1, hw2) = hws(bytes);
        seen.insert(decode(hw1, hw2, ArmFeatures::CORTEX_M4F).op);
    }
    for (_, bytes, _) in M7_VECTORS {
        let (hw1, hw2) = hws(bytes);
        seen.insert(decode(hw1, hw2, ArmFeatures::CORTEX_M7).op);
    }
    let missing: Vec<usize> = (0..mcs_core::arm::thumb::OP_COUNT)
        .filter(|&n| {
            // SAFETY: `Op` is `repr(u8)` with dense discriminants 0..OP_COUNT.
            let op: Op = unsafe { std::mem::transmute(n as u8) };
            op != Op::UNDEF && !seen.contains(&op)
        })
        .collect();
    let names: Vec<&str> = missing.iter().map(|&n| mcs_core::arm::thumb::OP_NAMES[n]).collect();
    assert!(missing.is_empty(), "ops without a vector: {:?}", names);
}
