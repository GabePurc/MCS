//! Classic-core forms on a fake ATmega-like test device, plus the exhaustive
//! disassemble -> reassemble round trips.

mod common;

use std::time::{Duration, Instant};

use common::*;
use mcs_asm::{assemble_with_spec, AssembleOptions, AssembleResult};
use mcs_core::avr::device::AvrDeviceSpec;
use mcs_core::avr::isa::{decode, decode_table, disassemble, op, DisasmContext};
use mcs_core::program::SymbolSpace;

fn mega(src: &str) -> Vec<u16> {
    let spec = test_mega();
    let r = mega_asm(src, &spec);
    assert_eq!(errors(&r), Vec::<String>::new(), "source: {src}");
    flash_words(&r)
}

#[test]
fn encodes_classic_only_forms() {
    let cases: &[(&str, &[u16])] = &[
        ("ldd r16, Y+5", &[0x810d]),
        ("ld r16, Y+5", &[0x810d]),
        ("ldd r16, Z", &[0x8100]),
        ("std Z+63, r31", &[0xaff7]),
        ("adiw r24, 1", &[0x9601]),
        ("adiw r25:r24, 1", &[0x9601]),
        ("sbiw r30, 63", &[0x97ff]),
        ("movw r16, r18", &[0x0189]),
        ("movw r17:r16, r1:r0", &[0x0180]),
        ("mul r0, r1", &[0x9c01]),
        ("lpm", &[0x95c8]),
        ("lpm r16, Z+", &[0x9105]),
        ("lds r0, 0x0100", &[0x9000, 0x0100]),
        ("sts 0x0100, r1", &[0x9210, 0x0100]),
        ("jmp target\nnop\ntarget: call target", &[0x940c, 0x0003, 0x0000, 0x940e, 0x0003]),
        ("ldi r16, low(RAMEND)\nout PORTB, r16", &[0xef0f, 0xb905]),
        ("ldi XL, 1\nldi YH, 2", &[0xe0a1, 0xe0d2]),
    ];
    for &(src, expected) in cases {
        assert_eq!(mega(src), expected, "{src}");
    }
}

#[test]
fn device_directive_selects_the_custom_spec() {
    let spec = test_mega();
    let r = mega_asm(".device TestMega\nmul r0, r1\n", &spec);
    assert!(r.ok, "{:?}", errors(&r));
    assert_eq!(r.device_id, "testmega");
    // Switching to a registry device before any code is allowed.
    let r = mega_asm(".device ATtiny10\nnop\n", &spec);
    assert_eq!(r.device_id, "attiny10");
}

#[test]
fn writes_eseg_data_to_the_eeprom_image() {
    let spec = test_mega();
    let r = mega_asm(".eseg\nee: .db 1, 2\n.dw 0x1234\n.dq -2\n.cseg\nldi r16, ee+1\n", &spec);
    assert_eq!(errors(&r), Vec::<String>::new());
    let ee = r.program.eeprom.as_ref().unwrap();
    assert_eq!(&ee[..13], &[1, 2, 0x34, 0x12, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    let sym = r.program.symbols.iter().find(|s| s.name == "ee").unwrap();
    assert_eq!((sym.address, sym.space), (0, SymbolSpace::Eeprom));
    assert_eq!(flash_words(&r), vec![0xe001]);
    assert!(r.listing.lines().any(|l| l.starts_with("E:000000 01 02")), "{}", r.listing);
    assert!(r.listing.contains(";   EEPROM:        5 of 1024"), "{}", r.listing);
    // .byte reservations in the EEPROM segment are bounded by the EEPROM size.
    let e = mega_asm(".eseg\n.byte 1025\n", &spec);
    assert_eq!(errors(&e)[0], "EEPROM segment exceeds the TestMega EEPROM size (1024 bytes)");
}

#[test]
fn reports_classic_core_register_constraints() {
    let spec = test_mega();
    let e = |src: &str| errors(&mega_asm(src, &spec)).into_iter().next().unwrap_or_default();
    assert_eq!(e("ldi r5, 1"), "'ldi' requires a register in r16-r31 (got r5)");
    assert!(e("adiw r23, 1").contains("r24, r26, r28 or r30"));
    assert!(e("movw r1, r2").contains("even register"));
    assert!(e("mulsu r24, r16").contains("r16-r23"));
    assert!(e("adiw r24:r25, 1").contains("invalid register pair"));
    assert!(e("ldd r16, Y+64").contains("displacement 64 out of range"));
    assert!(e("ld r16, X+1").contains("only available with Y or Z"));
    assert_eq!(e("call 0x400000"), "address 0x400000 out of range (0x00..0x3FFFFF)");
}

/// Disassembles every valid first instruction word for `spec`, assembles the text back and
/// checks the encoding is identical (covers every operand form and alias of the ISA table).
fn round_trip(spec: &AvrDeviceSpec, batch: usize) -> (usize, Vec<String>) {
    let table = decode_table(spec.features);
    const W2: u16 = 0x0123;
    let ctx = DisasmContext::default();
    let mut mismatches = Vec::new();
    let mut checked = 0;
    let mut lines: Vec<String> = Vec::new();
    let mut expected: Vec<u16> = Vec::new();
    let assemble = |src: &str| -> AssembleResult { assemble_with_spec(src, spec, &AssembleOptions::new("rt.asm", &spec.id)) };
    let flush = |lines: &mut Vec<String>, expected: &mut Vec<u16>, mismatches: &mut Vec<String>| {
        if lines.is_empty() {
            return;
        }
        let r = assemble(&lines.join("\n"));
        for e in r.diagnostics.iter().filter(|d| d.severity == mcs_core::program::Severity::Error).take(5) {
            let line = lines.get((e.line as usize).wrapping_sub(1)).cloned().unwrap_or_default();
            mismatches.push(format!("{line}: {}", e.message));
        }
        let got = flash_words_range(&r, 0, expected.len());
        for (i, (&g, &x)) in got.iter().zip(expected.iter()).enumerate() {
            if mismatches.len() >= 20 {
                break;
            }
            if g != x {
                mismatches.push(format!("word {i}: expected {x:04x} got {g:04x}"));
            }
        }
        lines.clear();
        expected.clear();
    };
    for w in 0..=0xffffu32 {
        let w = w as u16;
        let o = table.ops[w as usize];
        if o == 0 {
            continue;
        }
        let d = decode(&table, w, W2);
        let def = d.def.unwrap();
        let pc = expected.len() as u32;
        let dis = disassemble(&table, pc, w, W2, &ctx);
        let text = match o {
            op::BRBS | op::BRBC => format!("{} PC+({})", dis.mnemonic, d.values[1] + 1),
            op::RJMP | op::RCALL => format!("{} PC+({})", dis.mnemonic, d.values[0] + 1),
            op::JMP | op::CALL => format!("{} {}", dis.mnemonic, d.values[0]),
            _ => format!("{} {}", dis.mnemonic, dis.operands),
        };
        lines.push(text);
        expected.push(w);
        if def.words == 2 {
            expected.push(W2);
        }
        checked += 1;
        if expected.len() >= batch {
            flush(&mut lines, &mut expected, &mut mismatches);
        }
    }
    flush(&mut lines, &mut expected, &mut mismatches);
    (checked, mismatches)
}

#[test]
fn round_trips_every_attiny10_instruction_word() {
    let (checked, mismatches) = round_trip(tiny10(), 500);
    assert_eq!(mismatches, Vec::<String>::new());
    assert!(checked > 30000, "{checked}");
}

#[test]
fn round_trips_every_classic_core_instruction_word() {
    let (checked, mismatches) = round_trip(&test_mega(), 4000);
    assert_eq!(mismatches, Vec::<String>::new());
    assert!(checked > 50000, "{checked}");
}

#[test]
fn assembles_4k_classic_lines_quickly() {
    let mut lines = Vec::with_capacity(4000);
    for i in 0..4000 {
        lines.push(match i % 8 {
            0 => format!("L{i}: ldi r{}, {} ; comment", 16 + (i % 16), i & 0xff),
            1 => format!("    ldd r{}, Y+{}", i % 32, i % 64),
            2 => format!("    brne L{}", i - 2),
            3 => format!("    sts 0x{:x}, r{}", 0x100 + i, i % 32),
            4 => format!("    rcall L{}", i - 4),
            5 => format!("    out 0x05, r{}", i % 32),
            6 => format!("    adiw r24, {}", i % 64),
            _ => format!("    .dw L{} + {i}", i - 7),
        });
    }
    let src = lines.join("\n");
    let spec = test_mega();
    let opts = AssembleOptions::new("big.asm", "testmega");
    let r = assemble_with_spec(&src, &spec, &opts);
    assert_eq!(errors(&r), Vec::<String>::new());
    assert_eq!(r.program.lines.len(), 4000);
    assemble_with_spec(&src, &spec, &opts); // warm-up
    let dt = (0..5)
        .map(|_| {
            let t0 = Instant::now();
            assemble_with_spec(&src, &spec, &opts);
            t0.elapsed()
        })
        .min()
        .unwrap();
    println!("4000-line classic-core program assembled in {dt:?}");
    assert!(dt < Duration::from_millis(20), "{dt:?}");
}
