//! Port of the TypeScript assembler test suite (`tests/asm/assembler.test.ts`).

mod common;

use std::time::{Duration, Instant};

use common::*;
use mcs_asm::{assemble, def_include_name, generate_def_include, scan_includes, AssembleOptions};
use mcs_core::program::{LineEntry, ProgramFormat, ProgramSymbol, Severity, SymbolKind, SymbolSpace};

// ------------------------------------------------------------------------------------------------
// ATtiny10 encodings

const ENCODINGS: &[(&str, &[u16])] = &[
    ("nop", &[0x0000]),
    ("ldi r16, 0xFF", &[0xef0f]),
    ("LDI R16, $ff", &[0xef0f]),
    ("ser r17", &[0xef1f]),
    ("ldi r16, low(0x1234)", &[0xe304]),
    ("ldi r17, HIGH(0x1234)", &[0xe112]),
    ("ldi r18, 'A'", &[0xe421]),
    ("ldi r16, 0b1010", &[0xe00a]),
    ("ldi r16, 010", &[0xe008]),
    ("out 0x01, r16", &[0xb901]),
    ("in r16, 0x3F", &[0xb70f]),
    ("sbi 0x02, 0", &[0x9a10]),
    ("cbi 0x02, 3", &[0x9813]),
    ("sbic 0x00, 1", &[0x9901]),
    ("sbis 0x00, 7", &[0x9b07]),
    ("lds r16, 0x40", &[0xa100]),
    ("lds r31, 0xBF", &[0xa6ff]),
    ("sts 0x5F, r17", &[0xab1f]),
    ("ld r16, X", &[0x910c]),
    ("ld r16, X+", &[0x910d]),
    ("ld r17, -X", &[0x911e]),
    ("ld r16, Y", &[0x8108]),
    ("ld r16, Y+", &[0x9109]),
    ("ld r16, Z", &[0x8100]),
    ("ld r16, Z+", &[0x9101]),
    ("ld r16, -Z", &[0x9102]),
    ("st X, r16", &[0x930c]),
    ("st Y, r16", &[0x8308]),
    ("st -Y, r17", &[0x931a]),
    ("st Z, r16", &[0x8300]),
    ("st Z+, r20", &[0x9341]),
    ("push r16", &[0x930f]),
    ("pop r16", &[0x910f]),
    ("add r16, r17", &[0x0f01]),
    ("adc r16, r17", &[0x1f01]),
    ("sub r16, r17", &[0x1b01]),
    ("sbc r16, r17", &[0x0b01]),
    ("and r16, r17", &[0x2301]),
    ("or r16, r17", &[0x2b01]),
    ("eor r16, r17", &[0x2701]),
    ("mov r16, r17", &[0x2f01]),
    ("cp r16, r17", &[0x1701]),
    ("cpc r16, r17", &[0x0701]),
    ("cpse r16, r17", &[0x1301]),
    ("clr r16", &[0x2700]),
    ("lsl r16", &[0x0f00]),
    ("rol r16", &[0x1f00]),
    ("tst r16", &[0x2300]),
    ("cpi r16, 0x10", &[0x3100]),
    ("cpi r16, -1", &[0x3f0f]),
    ("subi r16, 1", &[0x5001]),
    ("subi r16, -5", &[0x5f0b]),
    ("sbci r16, 1", &[0x4001]),
    ("ori r16, 0x80", &[0x6800]),
    ("andi r16, 0x0F", &[0x700f]),
    ("sbr r16, 3", &[0x6003]),
    ("cbr r16, 0x0F", &[0x7f00]),
    ("com r16", &[0x9500]),
    ("neg r16", &[0x9501]),
    ("swap r16", &[0x9502]),
    ("inc r16", &[0x9503]),
    ("asr r16", &[0x9505]),
    ("lsr r16", &[0x9506]),
    ("ror r16", &[0x9507]),
    ("dec r16", &[0x950a]),
    ("bset 7", &[0x9478]),
    ("sei", &[0x9478]),
    ("cli", &[0x94f8]),
    ("sec", &[0x9408]),
    ("clc", &[0x9488]),
    ("set", &[0x9468]),
    ("clt", &[0x94e8]),
    ("bclr 0", &[0x9488]),
    ("ijmp", &[0x9409]),
    ("icall", &[0x9509]),
    ("ret", &[0x9508]),
    ("reti", &[0x9518]),
    ("sleep", &[0x9588]),
    ("break", &[0x9598]),
    ("wdr", &[0x95a8]),
    ("bld r16, 3", &[0xf903]),
    ("bst r16, 0", &[0xfb00]),
    ("sbrc r16, 1", &[0xfd01]),
    ("sbrs r17, 7", &[0xff17]),
    ("rjmp .", &[0xcfff]),
    ("rjmp PC", &[0xcfff]),
    ("rjmp .+2", &[0xc000]),
    ("rcall .", &[0xdfff]),
    ("brbs 1, .", &[0xf3f9]),
    ("breq .", &[0xf3f9]),
    ("brne .", &[0xf7f9]),
];

#[test]
fn attiny10_encodings() {
    let mut failures = Vec::new();
    for &(src, expected) in ENCODINGS {
        let r = asm(src);
        let errs = errors(&r);
        let got = flash_words(&r);
        if !errs.is_empty() || got != expected {
            failures.push(format!("{src}: expected {expected:04x?}, got {got:04x?} {errs:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ------------------------------------------------------------------------------------------------
// Labels, branches and data

#[test]
fn encodes_self_loops_and_backward_forward_branches() {
    let w = words(
        "
loop:   rjmp loop
        ldi r16, 5
again:  dec r16
        brne again      ; backwards: offset -2
        brne skip       ; forwards: offset +1
        nop
skip:   rjmp loop
",
    );
    assert_eq!(w, vec![0xcfff, 0xe005, 0x950a, 0xf7f1, 0xf409, 0x0000, 0xcff9]);
}

#[test]
fn packs_db_strings_and_bytes_with_zero_padding() {
    let r = asm(".db \"AB\", 0\n.db 1, 2\n");
    assert!(errors(&r).is_empty());
    assert_eq!(&r.program.flash[..6], &[0x41, 0x42, 0x00, 0x00, 0x01, 0x02]);
    assert!(r.diagnostics.iter().any(|d| d.severity == Severity::Warning && d.line == 1));
    assert_eq!(r.program.flash_used, 6);
}

#[test]
fn emits_dw_with_forward_refs_dd_and_gnu_byte_word() {
    let w = words(
        "
table:  .dw end, table, -1
        .dd 0x12345678
        .byte 1, 2
        .word 0xBEEF
end:    nop
",
    );
    assert_eq!(w, vec![7, 0, 0xffff, 0x5678, 0x1234, 0x0201, 0xbeef, 0x0000]);
}

#[test]
fn supports_org_pc_and_word_addresses() {
    let r = asm("rjmp main\n.org 0x10\nmain: rjmp PC-0x10\n");
    assert!(errors(&r).is_empty());
    assert_eq!(flash_words_range(&r, 0, 1), vec![0xc00f]);
    assert_eq!(flash_words_range(&r, 0x10, 1), vec![0xcfef]);
    let main = r.program.symbols.iter().find(|s| s.name == "main").unwrap();
    assert_eq!((main.address, main.space, main.kind), (0x20, SymbolSpace::Code, SymbolKind::Label));
}

#[test]
fn assigns_data_addresses_and_sizes_to_dseg_labels() {
    let r = asm(
        "
.dseg
buf:    .byte 4
count:
        .byte 1
.cseg
        lds r16, count
        sts buf+3, r16
",
    );
    assert!(errors(&r).is_empty(), "{:?}", errors(&r));
    let sym = |n: &str| r.program.symbols.iter().find(|s| s.name == n).unwrap().clone();
    let buf = sym("buf");
    assert_eq!((buf.address, buf.size, buf.space), (0x40, 4, SymbolSpace::Data));
    let count = sym("count");
    assert_eq!((count.address, count.size, count.space), (0x44, 1, SymbolSpace::Data));
    assert_eq!(flash_words(&r), vec![0xa104, 0xa903]);
}

#[test]
fn exports_equ_constants_and_labels_as_symbols() {
    let r = asm(".equ LED = 2\nstart: sbi 0x02, LED\n");
    assert_eq!(
        r.program.symbols,
        vec![
            ProgramSymbol { name: "LED".into(), address: 2, size: 0, kind: SymbolKind::Const, space: SymbolSpace::None, global: false },
            ProgramSymbol { name: "start".into(), address: 0, size: 0, kind: SymbolKind::Label, space: SymbolSpace::Code, global: true },
        ]
    );
}

// ------------------------------------------------------------------------------------------------
// Directives

#[test]
fn resolves_equ_forward_references() {
    let w = words(
        "
.equ A = B + 1
        ldi r16, A
        ldi r17, C
.equ B = target
.equ C = 0x20
target: nop
",
    );
    assert_eq!(w, vec![0xe003, 0xe210, 0x0000]);
}

#[test]
fn allows_set_redefinition_in_source_order() {
    let w = words(
        "
.set N = 1
        ldi r16, N
.set N = N + 1
        ldi r16, N
",
    );
    assert_eq!(w, vec![0xe001, 0xe002]);
}

#[test]
fn set_with_forward_reference_is_resolved_in_pass2() {
    let w = words(".set N = later\nldi r16, N\nlater: nop\n");
    assert_eq!(w, vec![0xe001, 0x0000]);
}

#[test]
fn supports_def_undef_register_aliases() {
    assert_eq!(words(".def temp = r20\nldi temp, 1\nmov r16, temp\n"), vec![0xe041, 0x2f04]);
    let r = asm(".def temp = r20\n.undef temp\nldi temp, 1\n");
    assert!(errors(&r)[0].contains("expected a register"), "{:?}", errors(&r));
    let r = asm(".undef nothing\n");
    assert!(r.ok);
    assert_eq!(r.diagnostics[0].message, "register alias 'nothing' is not defined");
    assert_eq!(first_msg(".def r5 = r20\n"), "'r5' is a register name");
    assert_eq!(first_msg(".def x = q\n"), ".def: expected a register (r0..r31)");
}

#[test]
fn evaluates_expressions_with_c_precedence_and_functions() {
    let w = words(
        "
        ldi r16, 1 + 2 * 3            ; 7
        ldi r17, (1 << 4) | 0x03      ; 0x13
        ldi r18, ~0x0F & 0xFF         ; 0xF0
        ldi r19, 10 > 3 ? 4 : 5       ; 4
        ldi r20, BYTE3(0x123456)      ; 0x12
        ldi r21, LOG2(64) + EXP2(2)   ; 10
        ldi r22, 17 % 5 - -1          ; 3
        ldi r23, !0 && 2 == 2         ; 1
        ldi r24, lo8(0xABCD)          ; 0xCD
        ldi r25, hi8(0xABCD)          ; 0xAB
        ldi r26, LWRD(0x12345678) >> 8 & 0xFF ; 0x56
        ldi r27, HWRD(0x12345678) & 0xFF      ; 0x34
        ldi r28, ABS(-3)
",
    );
    let imm: Vec<u16> = w.iter().map(|x| ((x >> 4) & 0xf0) | (x & 0x0f)).collect();
    assert_eq!(imm, vec![7, 0x13, 0xf0, 4, 0x12, 10, 3, 1, 0xcd, 0xab, 0x56, 0x34, 3]);
}

#[test]
fn handles_comments_of_all_styles() {
    let w = words(
        "
        ldi r16, 1 ; avrasm comment
        ldi r17, 2 // C++ comment
        /* block
           ldi r18, 3
        */ ldi r19, 4 /* inline */
",
    );
    assert_eq!(w, vec![0xe001, 0xe012, 0xe034]);
}

#[test]
fn expands_macros_with_parameters_and_local_labels() {
    let r = asm(
        "
.macro ldi16
        ldi @0, low(@2)
        ldi @1, high(@2)
.endm
.macro delay
        ldi @0, @1
wait:   dec @0
        brne wait
.endmacro
        ldi16 r24, r25, 0x1234
        delay r16, 3
        delay r17, 4
",
    );
    assert!(errors(&r).is_empty(), "{:?}", errors(&r));
    assert_eq!(flash_words(&r), vec![0xe384, 0xe192, 0xe003, 0x950a, 0xf7f1, 0xe014, 0x951a, 0xf7f1]);
    // Macro-expanded code maps to the invocation line.
    let rows: Vec<(u32, u32, bool)> = r.program.lines.iter().map(|l| (l.address, l.line, l.is_stmt)).collect();
    assert_eq!(&rows[..3], &[(0, 11, true), (2, 11, false), (4, 12, true)]);
    assert!(!r.program.symbols.iter().any(|s| s.name.contains("wait")));
    assert!(
        r.listing.lines().any(|l| l.split_whitespace().collect::<Vec<_>>() == ["C:000000", "e384", "+", "ldi", "r24,", "low(0x1234)"]),
        "{}",
        r.listing
    );
}

#[test]
fn expands_nested_macros_and_gnu_relative_targets() {
    let w = words(
        "
.macro inner
        subi @0, @1
.endm
.macro outer
        inner @0, 1
        inner @0, -1
.endm
        outer r20
        brne .-4
",
    );
    assert_eq!(w, vec![0x5041, 0x5f4f, 0xf7e9]);
}

#[test]
fn evaluates_conditionals() {
    let w = words(
        "
.equ MODE = 2
.if MODE == 1
        ldi r16, 1
.elif MODE == 2
        ldi r16, 2
  .ifdef UNDEFINED_THING
        ldi r16, 99
  .else
        ldi r17, 3
  .endif
.else
        ldi r16, 4
.endif
.ifndef MODE
        nop
.endif
.ifdef PORTB
        ldi r18, PORTB
.endif
",
    );
    assert_eq!(w, vec![0xe002, 0xe013, 0xe022]);
}

#[test]
fn elseif_and_defined_function() {
    let w = words(
        "
.equ V = 3
.if V == 1
        ldi r16, 1
.elseif defined(V) && V == 3
        ldi r16, 3
.elif 1
        ldi r16, 9
.endif
",
    );
    assert_eq!(w, vec![0xe003]);
}

#[test]
fn reports_error_warning_message_and_stops_at_exit() {
    let r = asm(".warning \"careful\"\n.message \"hello\"\nnop\n.exit\n.error \"not reached\"\n");
    assert!(r.ok);
    let got: Vec<(Severity, String)> = r.diagnostics.iter().map(|d| (d.severity, d.message.clone())).collect();
    assert_eq!(got, vec![(Severity::Warning, "careful".to_string()), (Severity::Info, "hello".to_string())]);
    let e = asm(".error \"boom\"\n");
    assert!(!e.ok);
    assert_eq!((e.diagnostics[0].message.as_str(), e.diagnostics[0].line), ("boom", 1));
    // `.exit <expr>` only exits when the expression is non-zero.
    assert_eq!(words(".exit 0\nnop\n.exit 1\nnop\n"), vec![0x0000]);
}

#[test]
fn accepts_and_ignores_listing_directives() {
    assert_eq!(words(".nolist\n.listmac\n.list\nnop\n.global x\n.globl y\n"), vec![0]);
    let r = asm(".nolist\nnop\n.list\nsleep\n");
    assert!(!r.listing.contains(" nop"));
    assert!(r.listing.contains(" sleep"));
}

// ------------------------------------------------------------------------------------------------
// Includes and devices

#[test]
fn provides_attiny10_definitions_via_tn10def_inc() {
    let r = asm(
        "
.include \"tn10def.inc\"
        sbi DDRB, DDB0
        out PORTB, r16
        ldi r16, RAMEND
        ldi r17, TIM0_OVFaddr
        ldi r18, OVF0addr
        ldi r19, (1 << CS00) | (1 << CS02)
        ldi ZL, 1
        sbrs r16, SREG_Z
",
    );
    assert!(errors(&r).is_empty(), "{:?}", errors(&r));
    assert_eq!(r.device_id, "attiny10");
    assert_eq!(flash_words(&r), vec![0x9a08, 0xb902, 0xe50f, 0xe014, 0xe024, 0xe035, 0xe0e1, 0xff01]);
}

#[test]
fn selects_the_device_from_include_or_device() {
    let opts = AssembleOptions::new("a.asm", "attiny10");
    let r = assemble(".include \"tn9def.inc\"\nnop\n", &opts);
    assert_eq!(r.device_id, "attiny9");
    assert_eq!(r.program.device.as_deref(), Some("attiny9"));
    let d = assemble(".device ATtiny4\nnop\n", &opts);
    assert_eq!(d.device_id, "attiny4");
    assert_eq!(d.program.flash.len(), 512);
    assert!(errors(&asm(".device ATmega9999\n"))[0].contains("unknown device"));
    assert!(errors(&asm("nop\n.device ATtiny4\n"))[0].contains("before any code"));
    assert!(errors(&asm(".device ATtiny4\n.device ATtiny5\n"))[0].contains("device already set to ATtiny4"));
    assert!(errors(&asm(".include \"m2561def.inc\"\n"))[0].contains("is for an unsupported device"));
    // Supported classic parts select themselves.
    assert_eq!(assemble(".include \"m328Pdef.inc\"\nnop\n", &opts).device_id, "atmega328p");
    // Re-selecting the default device is fine.
    assert!(asm(".device attiny10\n.include \"tn10def.inc\"\nnop\n").ok);
}

#[test]
fn processes_user_include_files_and_maps_lines() {
    let r = asm_inc(
        ".include \"macros.inc\"\n        setup\n",
        &[("macros.inc", ".equ VALUE = 7\n.macro setup\n ldi r16, VALUE\n.endm\nnop\n")],
    );
    assert!(errors(&r).is_empty(), "{:?}", errors(&r));
    assert_eq!(flash_words(&r), vec![0x0000, 0xe007]);
    assert_eq!(r.program.files, vec!["main.asm".to_string(), "macros.inc".to_string()]);
    assert_eq!(
        r.program.lines,
        vec![
            LineEntry { address: 0, file: 1, line: 5, is_stmt: true },
            LineEntry { address: 2, file: 0, line: 2, is_stmt: true },
        ]
    );
    assert!(errors(&asm(".include \"missing.inc\"\n"))[0].contains("not found"));
    // .exit only ends the include file; errors inside includes point at the include.
    let x = asm_inc(".include \"a.inc\"\nnop\n", &[("a.inc", "nop\n.exit\nbogus\n")]);
    assert_eq!(flash_words(&x), vec![0, 0]);
    let y = asm_inc("nop\n.include \"b.inc\"\n", &[("b.inc", "\n  ldi r16, 999\n")]);
    let d = &y.diagnostics[0];
    assert_eq!((d.file.as_str(), d.line, d.column), ("b.inc", 2, 12));
    let s = asm_inc(".include \"self.inc\"\n", &[("self.inc", ".include \"self.inc\"\n")]);
    assert!(errors(&s)[0].contains("recursive"));
}

#[test]
fn diagnostics_from_includes_sort_after_main_file() {
    let r = asm_inc("frob\n.include \"b.inc\"\nzap\n", &[("b.inc", "bad1\n")]);
    let got: Vec<(&str, u32)> = r.diagnostics.iter().map(|d| (d.file.as_str(), d.line)).collect();
    assert_eq!(got, vec![("main.asm", 1), ("main.asm", 3), ("b.inc", 1)]);
}

#[test]
fn scans_include_names_excluding_device_includes() {
    assert_eq!(
        scan_includes(".include \"tn10def.inc\"\n.include \"a.inc\" ; x\n/* .include \"b.inc\" */\nlbl: .include \"c.inc\"\n.include \"a.inc\"\n"),
        vec!["a.inc".to_string(), "c.inc".to_string()]
    );
}

fn equ_value(text: &str, name: &str) -> Option<i64> {
    let prefix = format!(".equ\t{name}\t= ");
    let line = text.lines().find(|l| l.starts_with(&prefix))?;
    let v = line[prefix.len()..].split_whitespace().next()?;
    match v.strip_prefix("0x") {
        Some(h) => i64::from_str_radix(h, 16).ok(),
        None => v.parse().ok(),
    }
}

#[test]
fn generates_a_tn10def_inc_equivalent() {
    let t10 = tiny10();
    assert_eq!(def_include_name(t10), "tn10def.inc");
    let mut m = t10.clone();
    m.name = "ATmega328P".into();
    assert_eq!(def_include_name(&m), "m328Pdef.inc");
    let text = generate_def_include(t10);
    let equ = |n: &str| equ_value(&text, n);
    assert!(text.lines().any(|l| l == ".device ATtiny10"));
    let expect = [
        ("PORTB", 2),
        ("SREG", 0x3f),
        ("DDB0", 0),
        ("PB2", 2),
        ("CS00", 0),
        ("CS02", 2),
        ("ISC01", 1),
        ("WDP2", 2),
        ("WDP3", 5),
        ("COM0A1", 7),
        ("SM2", 3),
        ("SREG_C", 0),
        ("SREG_I", 7),
        ("RAMEND", 0x5f),
        ("RAMSTART", 0x40),
        ("SRAM_START", 0x40),
        ("SRAM_SIZE", 32),
        ("FLASHEND", 0x1ff),
        ("INT0addr", 1),
        ("TIM0_OVFaddr", 4),
        ("INT_VECTORS_SIZE", 11),
        ("SIGNATURE_000", 0x1e),
        ("SIGNATURE_002", 0x03),
    ];
    for (name, v) in expect {
        assert_eq!(equ(name), Some(v), "{name}");
    }
    assert!(text.lines().any(|l| l == ".def\tZL\t= r30"));
    assert!(text.ends_with('\n'));
    // The generated text itself assembles cleanly (e.g. when saved under another name).
    let r = asm_inc(".include \"defs.inc\"\nldi r16, TIM0_COMPBaddr\n", &[("defs.inc", &text)]);
    assert!(errors(&r).is_empty(), "{:?}", errors(&r));
    assert_eq!(flash_words(&r), vec![0xe006]);
}

#[test]
fn uses_two_word_vectors_and_data_addresses_on_jmp_devices() {
    let text = generate_def_include(&test_mega());
    assert!(text.lines().any(|l| l.starts_with(".equ\tPORTB\t= 0x05")), "{text}");
    assert!(text.lines().any(|l| l.starts_with(".equ\tTCCR1A\t= 0x0080\t; MEMORY MAPPED")));
    assert!(text.lines().any(|l| l.starts_with(".equ\tTIM0_OVFaddr\t= 0x0020")));
    assert!(text.lines().any(|l| l.starts_with(".equ\tINT_VECTORS_SIZE\t= 34")));
    assert_eq!(equ_value(&text, "EEADRBITS"), Some(10));
    assert_eq!(equ_value(&text, "PB5"), Some(5));
    assert_eq!(equ_value(&text, "MAPPED_FLASH_START"), None);
}

// ------------------------------------------------------------------------------------------------
// Diagnostics

#[test]
fn reports_unknown_mnemonics_with_position() {
    let d = first("  nop\n  frob r16\n");
    assert_eq!(
        (d.message.as_str(), d.line, d.column, d.file.as_str()),
        ("unknown instruction or macro 'frob'", 2, 3, "main.asm")
    );
}

#[test]
fn rejects_instructions_not_supported_by_the_reduced_core() {
    for src in ["adiw r24, 1", "ldd r16, Y+1", "ld r16, Z+2", "std Z+1, r16", "lpm", "lpm r16, Z+", "mul r16, r17", "movw r16, r18", "jmp 0", "spm"] {
        let m = first_msg(src);
        assert!(m.contains("is not supported by ATtiny10 (AVRrc core)"), "{src}: {m}");
    }
    assert_eq!(first_msg("adiw r24, 1"), "instruction 'adiw' is not supported by ATtiny10 (AVRrc core)");
}

#[test]
fn rejects_r0_r15_on_the_reduced_core() {
    let d = first("mov r0, r16");
    assert_eq!((d.message.as_str(), d.column), ("register r0 is not available on ATtiny10 (only r16-r31)", 5));
    assert_eq!(first_msg("ldi r5, 1"), "register r5 is not available on ATtiny10 (only r16-r31)");
}

#[test]
fn checks_operand_ranges() {
    assert!(first_msg("ldi r16, 256").contains("constant 256 out of range"));
    assert!(first_msg("ldi r16, -129").contains("out of range"));
    assert_eq!(first_msg("out 64, r16"), "I/O address 0x40 out of range (0x00..0x3F)");
    assert!(first_msg("sbi 32, 0").contains("I/O address 0x20 out of range"));
    assert_eq!(first_msg("sbi 0, 8"), "bit number 8 out of range (0..7)");
    assert_eq!(first_msg("lds r16, 0x20"), "data address 0x20 out of range for 'lds' on the AVRrc core (0x40-0xBF)");
    assert!(first_msg("sts 0xC0, r16").contains("out of range"));
    assert_eq!(first_msg("brbs 8, ."), "SREG bit number 8 out of range (0..7)");
}

#[test]
fn reports_out_of_range_relative_branches() {
    let d = first("breq far\n.org 100\nfar: nop\n");
    assert_eq!(d.line, 1);
    assert_eq!(d.message, "relative branch out of range (offset 99 words, allowed -64..63)");
    // RJMP wraps around on small devices (3000 - 1 = 2999 = 439 mod 512 words).
    assert_eq!(words("rjmp 3000"), vec![0xc1b7]);
    assert!(first_msg("rjmp .+3").contains("not a word boundary"));
}

#[test]
fn reports_undefined_symbols_at_their_column() {
    let d = first("ldi r16, NOPE\n");
    assert_eq!((d.message.as_str(), d.line, d.column), ("undefined symbol 'NOPE'", 1, 10));
    let d = first(".equ X1 = MISSING + 1\n");
    assert_eq!((d.message.as_str(), d.line), ("undefined symbol 'MISSING'", 1));
    assert!(first_msg(".equ A = B\n.equ B = A\nldi r16, A\n").contains("circular"));
    assert!(first_msg(".org LATER\nLATER: nop\n").contains("forward references are not allowed"));
    assert_eq!(first_msg("ldi r16, r17 + 1"), "register 'r17' used where a constant is expected");
}

#[test]
fn reports_operand_shape_and_count_errors() {
    assert!(first_msg("ldi r16").contains("expects 2 operands"));
    assert_eq!(first_msg("ld r16, Q"), "invalid operand 2 for 'ld': expected X, X+, -X, Y, Y+, -Y, Z, Z+, -Z");
    assert!(first_msg("ldi 5, r16").contains("invalid operand 1 for 'ldi': expected a register"));
    assert!(first_msg("breq").contains("expects 1 operand"));
    assert!(first_msg("clr r16, r17").contains("expects 1 operand"));
    assert_eq!(first_msg("lpm r16"), "'lpm' expects 0 or 2 operands (got 1)");
    assert_eq!(first_msg("ldi r16,"), "missing operand");
}

#[test]
fn detects_overlapping_code_and_flash_overflow() {
    assert!(first_msg("nop\n.org 0\nnop\n").contains("overlaps"));
    assert_eq!(first_msg("nop\n.org 0\nnop\n"), "code overlaps previously assembled code at byte address 0x0000 (main.asm:1)");
    assert_eq!(first_msg(".org 511\nnop\nnop\n"), "code exceeds the ATtiny10 flash memory (1024 bytes) at byte address 0x0400");
    assert_eq!(first_msg(".dseg\n.byte 40\n"), "data segment exceeds SRAM (RAMEND = 0x005F, 32 bytes)");
}

#[test]
fn rejects_misplaced_statements_and_syntax_errors() {
    assert!(first_msg(".dseg\nnop\n").contains("not allowed in the data segment"));
    assert!(first_msg(".dseg\n.db 1\n").contains("not allowed in the data segment"));
    assert!(first_msg(".eseg\n.db 1\n").contains("EEPROM"));
    assert!(first_msg("ldi r16, (1 + 2\n").contains("expected ')'"));
    assert!(first_msg("ldi r16, 0x\n").contains("invalid number"));
    assert!(first_msg(".db \"abc\n").contains("unterminated string"));
    assert!(first_msg("a: nop\na: nop\n").contains("already defined"));
    assert!(first_msg(".if 1\nnop\n").contains("missing .endif"));
    assert!(first_msg(".endif\n").contains("without .if"));
    assert!(first_msg(".macro m\nnop\n").contains("missing .endm"));
    assert!(first_msg(".bogus\n").contains("unknown directive"));
    assert!(first_msg("ldi r16, 1/0\n").contains("division by zero"));
    assert_eq!(first_msg("x: .if 1\n"), "a label is not allowed before .if");
    assert_eq!(first_msg(".dw \"ab\"\n"), "strings are only allowed in .db");
    assert_eq!(first_msg(".else\n"), ".else without .if");
    assert_eq!(first_msg(".endm\n"), ".endm without .macro");
    assert_eq!(first_msg("r5: nop\n"), "'r5' is a reserved name");
    assert_eq!(first_msg(".org -1\n"), ".org address -1 is negative");
    assert_eq!(first_msg("ld r16, X+1\n"), "displacement addressing is only available with Y or Z");
    assert_eq!(first_msg("ldi r16, @0\n"), "macro parameter @0 has no value (missing macro argument?)");
}

#[test]
fn attributes_macro_errors_to_the_invocation_line() {
    let r = asm(".macro m\n ldi @0, 1\n.endm\n\n  m r5\n");
    let d = &r.diagnostics[0];
    assert_eq!((d.line, d.column), (5, 3));
    assert_eq!(d.message, "register r5 is not available on ATtiny10 (only r16-r31) (in macro 'm', main.asm:2)");
}

#[test]
fn collects_all_errors_and_keeps_going() {
    let r = asm("frob\nldi r16, 300\nadiw r24, 1\nnop\nldi r5, 1\n");
    assert!(!r.ok);
    let lines: Vec<u32> = r.diagnostics.iter().filter(|d| d.severity == Severity::Error).map(|d| d.line).collect();
    assert_eq!(lines, vec![1, 2, 3, 5]);
    // Bad instructions keep their slot (unknown mnemonics have no size), later code is still placed correctly.
    assert_eq!(flash_words_range(&r, 0, 4), vec![0xffff, 0xffff, 0x0000, 0xffff]);
}

#[test]
fn never_panics_on_garbage_input() {
    let mut seed: u64 = 12345;
    let mut rnd = move || {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fff_ffff;
        seed as f64 / 0x7fff_ffff as f64
    };
    let alphabet: Vec<char> =
        "abcdefgxyzXYZr0123456789 ,.:;+-*/()\"'@$#\n\t=<>!&|^~?ldistmovpushbrneqendifmacro".chars().collect();
    for _ in 0..200 {
        let len = (rnd() * 200.0) as usize;
        let s: String = (0..len).map(|_| alphabet[((rnd() * alphabet.len() as f64) as usize).min(alphabet.len() - 1)]).collect();
        let r = asm(&s);
        assert_eq!(r.program.flash.len(), 1024);
    }
    assert!(!asm(".macro r\nr\n.endm\nr\n").ok); // runaway recursion is caught
    let t0 = Instant::now();
    let bomb = asm(".macro b\nb\nb\n.endm\nb\n"); // exponential expansion is bounded
    assert!(t0.elapsed() < Duration::from_millis(2000), "{:?}", t0.elapsed());
    assert_eq!(errors(&bomb).iter().filter(|m| m.contains("too many macro expansions")).count(), 1);
    let r = assemble("nop", &AssembleOptions::new("x.asm", "nonexistent"));
    assert!(r.diagnostics[0].message.contains("unknown device"));
    assert_eq!(r.diagnostics[0].message, "unknown device 'nonexistent' (using ATtiny10)");
}

// ------------------------------------------------------------------------------------------------
// Output

#[test]
fn produces_line_table_files_flash_image_and_listing() {
    let r = asm("; header\nstart:\n    ldi r16, 0xFF\n    out 0x01, r16\n    rjmp start\n.db 1,2,3,4\n");
    assert!(r.ok);
    assert_eq!(r.program.format, ProgramFormat::Asm);
    assert_eq!(r.program.device.as_deref(), Some("attiny10"));
    assert_eq!(r.program.entry, 0);
    assert_eq!(r.program.flash.len(), 1024);
    assert_eq!(r.program.flash[10], 0xff);
    assert_eq!(r.program.flash_used, 10);
    assert_eq!(r.program.files, vec!["main.asm".to_string()]);
    assert_eq!(
        r.program.lines,
        vec![
            LineEntry { address: 0, file: 0, line: 3, is_stmt: true },
            LineEntry { address: 2, file: 0, line: 4, is_stmt: true },
            LineEntry { address: 4, file: 0, line: 5, is_stmt: true },
            LineEntry { address: 6, file: 0, line: 6, is_stmt: false },
        ]
    );
    let ws = |l: &str| l.split_whitespace().collect::<Vec<_>>().join(" ");
    let lines: Vec<String> = r.listing.lines().map(ws).collect();
    assert!(lines.iter().any(|l| l == "C:000000 ef0f ldi r16, 0xFF"), "{}", r.listing);
    assert!(lines.iter().any(|l| l == "C:000003 0201 0403 .db 1,2,3,4"), "{}", r.listing);
    assert!(r.listing.contains("0 errors"));
    assert!(r.listing.contains("; ATtiny10 memory use summary [bytes]:"));
    assert!(r.listing.contains(";   Code (flash):  10 of 1024 (1.0%)"), "{}", r.listing);
    assert!(r.listing.ends_with("; Assembly complete: 0 errors, 0 warnings"));
}

#[test]
fn result_serializes_to_camel_case_json() {
    let r = asm("nop\n");
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["deviceId"], "attiny10");
    assert_eq!(v["program"]["flashUsed"], 2);
    assert_eq!(v["program"]["format"], "asm");
    assert_eq!(v["program"]["lines"][0]["isStmt"], true);
    assert!(v["listing"].is_string());
    assert!(v["diagnostics"].is_array());
}

fn big_tiny_program() -> String {
    let mut lines: Vec<String> =
        [".include \"tn10def.inc\"", ".def tmp = r16", ".macro blink", " sbi PORTB, @0", " cbi PORTB, @0", ".endm"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    let mut i = 0;
    while lines.len() < 4000 {
        lines.push(format!("; block {i}"));
        lines.push(format!(".equ K{i} = {i} & 0xFF"));
        lines.push(format!(".if K{i} % 3 == 0"));
        lines.push(format!(" ldi tmp, low(K{i})"));
        lines.push(".else".into());
        lines.push(format!(" blink {}", i % 4));
        lines.push(".endif".into());
        lines.push(format!("lbl{i}:"));
        lines.push(format!(" rjmp lbl{i}"));
        i += 1;
    }
    lines.join("\n")
}

/// Best of a few runs (tests run in parallel, so single timings are noisy).
fn best_time(f: impl Fn()) -> Duration {
    f(); // warm-up
    (0..5)
        .map(|_| {
            let t0 = Instant::now();
            f();
            t0.elapsed()
        })
        .min()
        .unwrap()
}

#[test]
fn assembles_a_4k_line_program_quickly() {
    // Exercises includes, .def, macros and conditionals; the code is deliberately larger than 1 KB.
    let src = big_tiny_program();
    let opts = AssembleOptions::new("big.asm", "attiny10");
    let r = assemble(&src, &opts);
    assert!(r.diagnostics.iter().any(|d| d.message.contains("flash memory"))); // too big for 1 KB
    let dt = best_time(|| {
        assemble(&src, &opts);
    });
    println!("4000-line ATtiny10 program (macros, conditionals) assembled in {dt:?}");
    assert!(dt < Duration::from_millis(20), "{dt:?}");
}
