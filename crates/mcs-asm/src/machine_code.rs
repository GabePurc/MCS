//! Machine-code source files (`.mc`): programs written directly as instruction words, the way
//! the instruction set manual shows the opcodes. The result is a normal program image with a
//! line table, so machine-code files are debugged line by line like assembly.
//!
//! Syntax (one or more words per line, case-insensitive):
//!
//! ```text
//! ; comment            // comment too
//! @0x0010              ; continue at BYTE address 0x0010 (as shown in the Memory window)
//! .org 8               ; continue at WORD address 8 (avrasm2 semantics)
//! E00F                 ; one 16-bit word in hex (0x / $ prefixes optional)
//! 0xE00F 0xBB01        ; several words
//! 940C0010             ; 8 hex digits = a two-word instruction (first word first)
//! 0b1110_0000 0000 1111 ; binary: 0b, then digit groups until 16 bits are complete
//! 0020: E00F           ; listing style: byte address prefix
//! ```

use mcs_core::avr::device::AvrDeviceSpec;
use mcs_core::avr::devices;
use mcs_core::avr::isa::{self, DisasmContext};
use mcs_core::program::{Diagnostic, LineEntry, LoadedProgram, ProgramFormat, Severity};
use serde::Serialize;
use std::collections::HashMap;

/// Disassembly shown next to a source line.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McHint {
    /// 1-based source line.
    pub line: u32,
    /// Byte address of the line's first word.
    pub address: u32,
    pub text: String,
    /// All words on the line decode to instructions of the device.
    pub valid: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineCodeResult {
    pub ok: bool,
    pub program: LoadedProgram,
    pub device_id: String,
    pub listing: String,
    pub diagnostics: Vec<Diagnostic>,
    pub hints: Vec<McHint>,
}

struct Line {
    line: u32,
    /// First word address.
    start: u32,
    count: u32,
}

/// Builds a program from machine-code text for `device_id`.
pub fn assemble_machine_code(source: &str, file_name: &str, device_id: &str) -> MachineCodeResult {
    if let Some(name) = crate::non_avr_device(device_id) {
        let d = Diagnostic::error(crate::non_avr_message(&name), file_name, 0, 0);
        let mut program = LoadedProgram::empty(ProgramFormat::MachineCode, 0);
        program.files.push(file_name.to_string());
        program.diagnostics.push(d.clone());
        return MachineCodeResult { ok: false, program, device_id: device_id.to_string(), listing: String::new(), diagnostics: vec![d], hints: Vec::new() };
    }
    let spec = devices::get(device_id).or_else(|| devices::get("attiny10")).expect("device registry is never empty");
    let mut diags = Vec::new();
    let words_max = spec.flash_words();
    let mut flash = vec![0xffu8; spec.flash_size as usize];
    let mut owner: HashMap<u32, u32> = HashMap::new();
    let mut lines: Vec<Line> = Vec::new();
    let mut pc: u32 = 0;
    let mut used: u32 = 0;
    let err = |d: &mut Vec<Diagnostic>, sev: Severity, msg: String, line: u32, col: usize| d.push(Diagnostic::new(sev, msg, file_name, line, col as u32 + 1));

    for (idx, raw) in source.lines().enumerate() {
        let ln = idx as u32 + 1;
        let text = strip_comment(raw);
        let mut toks = tokens(text).into_iter().peekable();
        let mut words: Vec<u16> = Vec::new();
        let mut first_col = None;
        while let Some((col, tok)) = toks.next() {
            let lc = tok.to_ascii_lowercase();
            // Address directives.
            if let Some(a) = lc.strip_prefix('@') {
                match parse_int(a) {
                    Some(b) if b % 2 == 0 => pc = b / 2,
                    Some(_) => err(&mut diags, Severity::Error, format!("'{tok}': byte addresses of instructions must be even"), ln, col),
                    None => err(&mut diags, Severity::Error, format!("'{tok}': expected a byte address such as @0x0010"), ln, col),
                }
                continue;
            }
            if lc == ".org" {
                match toks.next().map(|(c, t)| (c, parse_int(&t.to_ascii_lowercase()))) {
                    Some((_, Some(w))) => pc = w,
                    Some((c, _)) => err(&mut diags, Severity::Error, ".org expects a word address".into(), ln, c),
                    None => err(&mut diags, Severity::Error, ".org expects a word address".into(), ln, col),
                }
                continue;
            }
            if let Some(a) = lc.strip_suffix(':') {
                // Listing-style "0020:" address prefix (byte address, hex).
                match u32::from_str_radix(a.trim_start_matches("0x"), 16) {
                    Ok(b) if b % 2 == 0 && words.is_empty() => pc = b / 2,
                    _ => err(&mut diags, Severity::Error, format!("'{tok}': expected an even hex byte address before the words"), ln, col),
                }
                continue;
            }
            first_col.get_or_insert(col);
            if let Some(bits) = lc.strip_prefix("0b") {
                // Binary word: 0b then digit groups until 16 bits are complete.
                let mut digits: String = bits.chars().filter(|&c| c != '_').collect();
                while digits.len() < 16 {
                    match toks.peek() {
                        Some((_, t)) if t.chars().all(|c| c == '0' || c == '1' || c == '_') => {
                            digits.extend(t.chars().filter(|&c| c != '_'));
                            toks.next();
                        }
                        _ => break,
                    }
                }
                if digits.len() != 16 || !digits.chars().all(|c| c == '0' || c == '1') {
                    err(&mut diags, Severity::Error, format!("binary word must have exactly 16 bits (got {} bits)", digits.len()), ln, col);
                    continue;
                }
                words.push(u16::from_str_radix(&digits, 2).unwrap_or(0));
                continue;
            }
            let h = lc.trim_start_matches("0x").trim_start_matches('$');
            if h.is_empty() || !h.chars().all(|c| c.is_ascii_hexdigit()) {
                err(&mut diags, Severity::Error, format!("'{tok}' is not a hex word (e.g. E00F) or a binary word (0b...)"), ln, col);
                continue;
            }
            match h.len() {
                1..=4 => words.push(u16::from_str_radix(h, 16).unwrap_or(0)),
                8 => {
                    words.push(u16::from_str_radix(&h[..4], 16).unwrap_or(0));
                    words.push(u16::from_str_radix(&h[4..], 16).unwrap_or(0));
                }
                _ => err(&mut diags, Severity::Error, format!("'{tok}': write words with up to 4 hex digits (8 for a two-word instruction)"), ln, col),
            }
        }
        if words.is_empty() {
            continue;
        }
        let start = pc;
        for w in &words {
            if pc >= words_max {
                err(&mut diags, Severity::Error, format!("address 0x{:04X} is beyond the {} bytes of program memory", pc * 2, spec.flash_size), ln, first_col.unwrap_or(0));
                break;
            }
            if let Some(prev) = owner.insert(pc, ln) {
                err(&mut diags, Severity::Warning, format!("word at 0x{:04X} overwrites the one from line {prev}", pc * 2), ln, first_col.unwrap_or(0));
            }
            flash[pc as usize * 2] = *w as u8;
            flash[pc as usize * 2 + 1] = (*w >> 8) as u8;
            pc += 1;
            used = used.max(pc * 2);
        }
        lines.push(Line { line: ln, start, count: pc - start });
    }

    let hints = annotate(spec, &flash, &lines, file_name, &mut diags);
    let mut line_table: Vec<LineEntry> = lines
        .iter()
        .flat_map(|l| (0..l.count).map(move |k| LineEntry { address: (l.start + k) * 2, file: 0, line: l.line, is_stmt: k == 0 }))
        .collect();
    line_table.sort_by_key(|e| e.address);
    line_table.dedup_by_key(|e| e.address);
    let listing = hints.iter().map(|h| format!("{:04X}  {}", h.address, h.text)).collect::<Vec<_>>().join("\n");
    diags.sort_by_key(|d| (d.line, d.column));
    let ok = !diags.iter().any(|d| d.severity == Severity::Error);
    let program = LoadedProgram {
        format: ProgramFormat::MachineCode,
        flash,
        flash_used: used,
        flash_base: 0,
        eeprom: None,
        fuses: None,
        lock: None,
        segments: Vec::new(),
        entry: 0,
        symbols: Vec::new(),
        files: vec![file_name.to_string()],
        lines: line_table,
        device: Some(spec.id.clone()),
        diagnostics: diags.clone(),
    };
    MachineCodeResult { ok, program, device_id: spec.id.clone(), listing, diagnostics: diags, hints }
}

/// Disassembles the instructions that start on each line; warns about invalid opcodes.
fn annotate(spec: &AvrDeviceSpec, flash: &[u8], lines: &[Line], file: &str, diags: &mut Vec<Diagnostic>) -> Vec<McHint> {
    let table = isa::decode_table(spec.features);
    let io_names: HashMap<u32, String> = spec.registers.iter().filter_map(|r| spec.data_to_io(r.addr).map(|io| (io as u32, r.name.clone()))).collect();
    let data_names: HashMap<u32, String> = spec.registers.iter().map(|r| (r.addr as u32, r.name.clone())).collect();
    let io_name = |a: u32| io_names.get(&a).cloned();
    let data_name = |a: u32| data_names.get(&a).cloned();
    let ctx = DisasmContext { code_label: None, io_name: Some(&io_name), data_name: Some(&data_name), flash_bytes: spec.flash_size };
    let words = spec.flash_words();
    let word = |i: u32| if i < words { u16::from_le_bytes([flash[i as usize * 2], flash[i as usize * 2 + 1]]) } else { 0xffff };
    let mut out = Vec::with_capacity(lines.len());
    for l in lines {
        let mut parts = Vec::new();
        let mut valid = true;
        let mut pc = l.start;
        while pc < l.start + l.count {
            let d = isa::disassemble(&table, pc, word(pc), word(pc + 1), &ctx);
            if d.valid {
                parts.push(if d.operands.is_empty() { d.mnemonic } else { format!("{} {}", d.mnemonic, d.operands) });
            } else {
                valid = false;
                parts.push(format!("??? (0x{:04X})", word(pc)));
                diags.push(Diagnostic::warning(format!("0x{:04X} at 0x{:04X} is not a valid {} instruction (the simulator stops there)", word(pc), pc * 2, spec.name), file, l.line, 1));
            }
            pc += d.words.max(1) as u32;
        }
        out.push(McHint { line: l.line, address: l.start * 2, text: parts.join("  |  "), valid });
    }
    out
}

fn strip_comment(s: &str) -> &str {
    let mut end = s.len();
    for pat in [";", "//", "#"] {
        if let Some(i) = s.find(pat) {
            end = end.min(i);
        }
    }
    &s[..end]
}

/// Whitespace/comma separated tokens with their byte column.
fn tokens(s: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in s.char_indices() {
        let sep = c.is_whitespace() || c == ',';
        match (sep, start) {
            (true, Some(b)) => {
                out.push((b, &s[b..i]));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(b) = start {
        out.push((b, &s[b..]));
    }
    out
}

fn parse_int(t: &str) -> Option<u32> {
    let t = t.trim();
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix('$')) {
        u32::from_str_radix(h, 16).ok()
    } else if let Some(b) = t.strip_prefix("0b") {
        u32::from_str_radix(b, 2).ok()
    } else {
        t.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_addresses_and_hints() {
        let src = "; blink\n@0\nE00F        ; ldi\n0b1011_1001 0000 0001\n@0x0010\n0020: CFFF\n940C0010\n";
        let r = assemble_machine_code(src, "t.mc", "attiny10");
        assert!(r.ok, "{:?}", r.diagnostics);
        assert_eq!(&r.program.flash[..4], &[0x0f, 0xe0, 0x01, 0xb9]);
        assert_eq!(&r.program.flash[0x20..0x22], &[0xff, 0xcf]);
        assert_eq!(r.hints[0], McHint { line: 3, address: 0, text: "ldi r16, 0x0F".into(), valid: true });
        assert_eq!(r.hints[1].text, "out DDRB, r16");
        assert_eq!(r.hints[2].address, 0x20);
        // JMP does not exist on the reduced core: warning, still loads.
        assert!(!r.hints[3].valid);
        assert_eq!(r.diagnostics.len(), 2);
        assert!(r.diagnostics.iter().all(|d| d.severity == Severity::Warning && d.line == 7));
        assert_eq!(r.program.lines.iter().filter(|l| l.is_stmt).map(|l| l.line).collect::<Vec<_>>(), [3, 4, 6, 7]);
        assert_eq!(r.program.flash_used, 0x26);
    }

    #[test]
    fn errors_have_positions() {
        let r = assemble_machine_code("E00F\nXYZ\n0b101\n@3\n.org 600\n0000\n", "t.mc", "attiny10");
        assert!(!r.ok);
        let at: Vec<_> = r.diagnostics.iter().map(|d| (d.line, d.column, d.severity)).collect();
        assert_eq!(at, [(2, 1, Severity::Error), (3, 1, Severity::Error), (4, 1, Severity::Error), (6, 1, Severity::Error)]);
        let r = assemble_machine_code("0000\n@0\n0000", "t.mc", "attiny10");
        assert!(r.ok);
        assert_eq!(r.diagnostics[0].severity, Severity::Warning);
    }
}
