//! Fuzz-style robustness tests: the assembler must never panic (or overflow the stack) on
//! malformed input, and must always return a full-size flash image.

mod common;

use std::collections::HashMap;

use common::*;
use mcs_asm::{assemble, assemble_with_spec, scan_includes, AssembleOptions, AssembleResult};

/// Runs `f` on a thread with a deliberately small stack (recursion must stay bounded).
fn small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(1 << 20).spawn(f).unwrap().join().unwrap()
}

fn check(r: &AssembleResult, len: usize) {
    assert_eq!(r.program.flash.len(), len);
    assert!(!r.diagnostics.iter().any(|d| d.message.contains("internal assembler error")), "{:?}", r.diagnostics.first());
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const VOCAB: &[&str] = &[
    ".include", ".device", ".def", ".undef", ".equ", ".set", ".org", ".cseg", ".dseg", ".eseg", ".byte", ".db", ".dw",
    ".dd", ".dq", ".macro", ".endm", ".endmacro", ".if", ".elif", ".elseif", ".else", ".endif", ".ifdef", ".ifndef",
    ".error", ".warning", ".message", ".list", ".nolist", ".listmac", ".exit", ".global", "ldi", "ld", "ldd", "st",
    "std", "lds", "sts", "rjmp", "rcall", "brne", "breq", "adiw", "movw", "lsl", "clr", "ser", "cbr", "sbr", "sei",
    "out", "in", "sbi", "cbi", "push", "pop", "nop", "jmp", "call", "lpm", "r0", "r16", "r24", "r25", "r31", "X", "Y",
    "Z", "X+", "-Y", "Y+", "Z+", "Y+3", "PC", ".", "low", "high", "defined", "EXP2", "LOG2", "PORTB", "RAMEND", "ZL",
    "(", ")", ",", ":", "=", "+", "-", "*", "/", "%", "<<", ">>", "&", "|", "^", "~", "!", "?", "&&", "||", "==",
    "0", "1", "0x7fffffff", "4294967295", "$ff", "0b1", "07", "'a'", "\"str\"", "\"tn10def.inc\"", "\"inc.inc\"",
    "@0", "@1", "@9", "lbl", "lbl:", "m", "x1", "\n", "\n", "\n", ";", "/*", "*/", "//", "é", "\u{1F600}", "\t",
];

#[test]
fn random_token_soup_never_panics() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut includes = HashMap::new();
    includes.insert("inc.inc".to_string(), ".macro m\n ldi @0, @1\n.endm\nx1: .equ q = lbl\n".to_string());
    let opts = AssembleOptions::new("fuzz.asm", "attiny10").with_includes(&includes);
    for _ in 0..3000 {
        let n = rng.below(80);
        let mut s = String::new();
        for _ in 0..n {
            s.push_str(VOCAB[rng.below(VOCAB.len())]);
            if rng.below(3) != 0 {
                s.push(' ');
            }
        }
        let r = assemble(&s, &opts);
        check(&r, 1024);
        let _ = scan_includes(&s);
    }
}

#[test]
fn random_characters_never_panic() {
    let mut rng = Rng(42);
    let chars: Vec<char> = "abcxyzXYZr0123456789 ,.:;+-*/()\"'\\@$#\n\r\t=<>!&|^~?_é\u{00a0}\u{1F600}\u{0}".chars().collect();
    let spec = test_mega();
    let opts = AssembleOptions::new("fuzz.asm", "x");
    for i in 0..3000 {
        let n = rng.below(300);
        let s: String = (0..n).map(|_| chars[rng.below(chars.len())]).collect();
        if i % 2 == 0 {
            check(&asm(&s), 1024);
        } else {
            check(&assemble_with_spec(&s, &spec, &opts), 32768);
        }
    }
}

#[test]
fn mutated_valid_programs_never_panic() {
    let base = "
.include \"tn10def.inc\"
.def tmp = r16
.equ N = (1 << 3) | LATER
.macro blink
        sbi PORTB, @0
wait:   dec tmp
        brne wait
        cbi PORTB, @0
.endm
.dseg
buf:    .byte 4
.cseg
start:  ldi tmp, low(N)
        blink 1
.if N > 4
        lds r17, buf+1
.else
        sts buf, r17
.endif
        ld r18, Z+
        rjmp start
LATER:  .db \"hi\", 0, 1
        .dw start, LATER
";
    let mut rng = Rng(7);
    let bytes: Vec<char> = base.chars().collect();
    let pool: Vec<char> = "()@.,:;+-*/\"'\n0123456789abcxyzr$#éZ".chars().collect();
    for _ in 0..3000 {
        let mut s = bytes.clone();
        for _ in 0..1 + rng.below(6) {
            let pos = rng.below(s.len());
            match rng.below(3) {
                0 => s[pos] = pool[rng.below(pool.len())],
                1 => {
                    s.remove(pos);
                }
                _ => s.insert(pos, pool[rng.below(pool.len())]),
            }
        }
        let s: String = s.into_iter().collect();
        check(&asm(&s), 1024);
    }
}

#[test]
fn deep_nesting_is_bounded() {
    small_stack(|| {
        // Parentheses / unary operators / ternaries.
        let r = asm(&format!("ldi r16, {}1{}\n", "(".repeat(100_000), ")".repeat(100_000)));
        check(&r, 1024);
        assert!(errors(&r)[0].contains("nested too deeply"), "{:?}", errors(&r));
        check(&asm(&format!("ldi r16, {}1\n", "-~!".repeat(50_000))), 1024);
        check(&asm(&format!("ldi r16, {}1\n", "1 ? ".repeat(20_000))), 1024);
        check(&asm(&format!(".if {}1{}\nnop\n.endif\n", "(".repeat(10_000), ")".repeat(10_000))), 1024);

        // Long chains of forward-referencing .equ definitions.
        let mut src = String::from("ldi r16, E0\n");
        for i in 0..20_000 {
            src.push_str(&format!(".equ E{i} = E{} + 1\n", i + 1));
        }
        src.push_str(".equ E20000 = 0\n");
        let r = asm(&src);
        check(&r, 1024);
        assert!(!r.ok);
        // A short chain resolves fine.
        assert_eq!(words(".equ A = B + 1\n.equ B = C + 1\n.equ C = D + 1\n.equ D = 5\nldi r16, A\n"), vec![0xe008]);

        // Deeply nested macros and includes.
        let mut src = String::new();
        for i in 0..100 {
            src.push_str(&format!(".macro m{i}\n m{}\n.endm\n", i + 1));
        }
        src.push_str(".macro m100\n nop\n.endm\n m0\n");
        let r = asm(&src);
        check(&r, 1024);
        assert!(errors(&r)[0].contains("macro nesting too deep"));
        let mut includes = HashMap::new();
        for i in 0..50 {
            includes.insert(format!("i{i}.inc"), format!(".include \"i{}.inc\"\nnop\n", i + 1));
        }
        let opts = AssembleOptions::new("main.asm", "attiny10").with_includes(&includes);
        let r = assemble(".include \"i0.inc\"\n", &opts);
        check(&r, 1024);
        assert!(errors(&r).iter().any(|e| e.contains("includes nested too deeply")));
    });
}

#[test]
fn extreme_values_are_reported_not_panicking() {
    let cases = [
        ".org 0x7fffffff * 0x7fffffff * 4\nnop\n",
        ".org 0xffffffff\nnop\nlbl: .dw lbl\n",
        ".dseg\n.org 0xffffffff * 0xffff\n.byte 0xffffffff * 0xffffffff\nx: .byte 1\n",
        ".eseg\n.org 0xffffffff\n.db 1\n",
        ".dq 0xffffffff * 0xffffffff * 0xffffffff, -0x7fffffff * 0xffffffff\n",
        ".dd -0xffffffff * 0xffffffff, 1 << 63, 1 << 64, 1 >> -1\n",
        ".equ X = -0x7fffffff * 0xffffffff * 4\nldi r16, X\nrjmp X\nbrne X\n",
        "ldi r16, -(0x7fffffff * 0xffffffff * 4) / -1 % -1\n",
        "ldi r16, EXP2(53) + LOG2(0) + ABS(0x7fffffff * 0xffffffff * 4)\n",
        ".set S = 1\n.set S = S * 0xffffffff * 0xffffffff\nldi r16, S\n",
        "lbl: .equ lbl = 1\n.def lbl = r16\nlbl lbl, lbl\n",
        ".db \"\u{1F600}é\\x\\q\", 'é'\n",
        ".include\n.include 5\n.device\n.device 5\n.undef\n.macro\n.endm\n.error\n.exit 1/0\n",
        ".macro m\n.macro n\n.endm\n m\n m 1,2,3,4,5,6,7,8,9,10,11\n",
        ".if\n.elif\n.else 1\n.endif 1\n.ifdef\n.ifndef 5\n.endif\n",
        "jmp 0x7fffffffffff\ncall -1\nlds r16, -1\nsts 0x10000, r16\nin r16, -1\nsbi -1, -1\n",
    ];
    let spec = test_mega();
    let opts = AssembleOptions::new("main.asm", "testmega");
    for src in cases {
        check(&asm(src), 1024);
        check(&assemble_with_spec(src, &spec, &opts), 32768);
    }
    let r = asm(".org 0x7fffffff * 0x7fffff\nnop\n");
    assert!(errors(&r).iter().any(|e| e.contains("exceeds the ATtiny10 flash memory")), "{:?}", errors(&r));
    assert_eq!(errors(&asm(".org 0x7fffffff * 0x7fffffff * 4\nnop\n")), vec![".org address -17179869180 is negative"]);
}

#[test]
fn exponential_macros_and_huge_inputs_stay_fast() {
    let t0 = std::time::Instant::now();
    let r = asm(".macro b\nb\nb\nb\n.endm\nb\n");
    check(&r, 1024);
    assert!(t0.elapsed() < std::time::Duration::from_secs(2));
    let long_line = format!(".db {}\n", vec!["1"; 50_000].join(","));
    let r = asm(&long_line);
    check(&r, 1024);
    assert!(errors(&r).iter().any(|e| e.contains("flash memory")));
    let many_lines = "nop\n".repeat(100_000);
    check(&asm(&many_lines), 1024);
}
