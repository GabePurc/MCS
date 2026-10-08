//! AVR assembler (Atmel avrasm2 dialect plus common GNU conveniences) producing MCS program
//! images with debug info (symbols, line table, listing).
//!
//! Supported: directives (`.include` incl. generated device definition includes such as
//! `"tn10def.inc"`, `.device`, `.def/.undef`, `.equ`, `.set`, `.org`, `.cseg/.dseg/.eseg`,
//! `.byte`, `.db/.dw/.dd/.dq`, `.macro/.endm` with `@0..@9`, `.if/.elif/.else/.endif`,
//! `.ifdef/.ifndef`, `.error/.warning/.message`, `.list/.nolist/.listmac`, `.exit`), C-precedence
//! expressions with avrasm2 functions (`LOW`, `HIGH`, `BYTE1..4`, `LWRD`, `HWRD`, `PAGE`, `EXP2`,
//! `LOG2`, `ABS`) and GNU ones (`lo8`, `hi8`, `pm`), `PC`/`.`, forward references, instruction
//! aliases, register pairs and device feature checks.
//!
//! The assembler never panics on malformed input: every problem is reported as a [`Diagnostic`].

#![forbid(unsafe_code)]

mod assembler;
mod expr;
mod incgen;
mod lexer;
mod machine_code;
mod util;

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

use mcs_core::avr::device::AvrDeviceSpec;
use mcs_core::avr::devices;
use mcs_core::program::{Diagnostic, LoadedProgram, ProgramFormat};
use serde::Serialize;

pub use incgen::{def_include_name, generate_def_include};
pub use machine_code::{assemble_machine_code, MachineCodeResult, McHint};

use assembler::Assembler;
use lexer::TokKind;

/// Assembler inputs besides the source text.
#[derive(Clone, Copy, Debug)]
pub struct AssembleOptions<'a> {
    /// Main file name/path (used in diagnostics and the line table).
    pub file_name: &'a str,
    /// Default device id when the source does not select one (`.device` / `.include "xxdef.inc"`).
    pub device_id: &'a str,
    /// Contents of user include files keyed by the exact string used in the `.include` directive
    /// (resolved by the caller, see [`scan_includes`]).
    pub includes: &'a HashMap<String, String>,
}

impl<'a> AssembleOptions<'a> {
    /// Options without user include files.
    pub fn new(file_name: &'a str, device_id: &'a str) -> Self {
        static EMPTY: OnceLock<HashMap<String, String>> = OnceLock::new();
        Self { file_name, device_id, includes: EMPTY.get_or_init(HashMap::new) }
    }

    /// Same options with user include files.
    pub fn with_includes(self, includes: &'a HashMap<String, String>) -> Self {
        Self { includes, ..self }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssembleResult {
    /// No errors.
    pub ok: bool,
    /// Format `asm`, always present (possibly partial on error).
    pub program: LoadedProgram,
    /// Device actually used.
    pub device_id: String,
    /// Human readable listing: address, words hex, source line.
    pub listing: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// Assembles `source` for `opts.device_id` (unless the source selects another device).
/// Never panics on malformed input: all problems are reported in `diagnostics`.
pub fn assemble(source: &str, opts: &AssembleOptions) -> AssembleResult {
    let requested = devices::get(opts.device_id);
    let fallback = requested.or_else(|| devices::get("attiny10")).or_else(|| devices::all().first());
    match fallback {
        Some(dev) => run(source, opts, dev, requested.is_none(), None),
        None => no_device(opts),
    }
}

/// Assembles `source` for an explicit device description, e.g. a custom or test device that is
/// not part of the registry (`opts.device_id` is ignored). `.device <spec.id>` selects it too.
pub fn assemble_with_spec(source: &str, spec: &AvrDeviceSpec, opts: &AssembleOptions) -> AssembleResult {
    run(source, opts, spec, false, Some(spec))
}

fn run<'a>(
    source: &str,
    opts: &AssembleOptions<'a>,
    device: &'a AvrDeviceSpec,
    default_invalid: bool,
    custom: Option<&'a AvrDeviceSpec>,
) -> AssembleResult {
    let mut asm = Assembler::new(opts, device, default_invalid, custom);
    // Defensive only: the assembler is written not to panic. With `panic = "abort"` builds this
    // is a no-op.
    if let Err(e) = catch_unwind(AssertUnwindSafe(|| asm.run(source))) {
        let msg = e
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| e.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown failure".into());
        asm.internal_error(&msg);
    }
    asm.finish()
}

fn no_device(opts: &AssembleOptions) -> AssembleResult {
    let d = Diagnostic::error(format!("unknown device '{}' (no devices available)", opts.device_id), opts.file_name, 0, 0);
    let mut program = LoadedProgram::empty(ProgramFormat::Asm, 0);
    program.files.push(opts.file_name.to_string());
    program.diagnostics.push(d.clone());
    AssembleResult { ok: false, program, device_id: opts.device_id.to_string(), listing: String::new(), diagnostics: vec![d] }
}

/// Returns the user include file names referenced by `.include` directives in `source` (in
/// order, without duplicates) so the caller can pre-load them. Device definition files such as
/// "tn10def.inc" are generated internally and therefore not listed. Not recursive.
pub fn scan_includes(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for line in lexer::tokenize(source) {
        let t = &line.toks;
        let mut i = 0;
        while i + 1 < t.len() && t[i].k == TokKind::Id && t[i + 1].is_op(":") {
            i += 2;
        }
        if i + 1 < t.len() && t[i].k == TokKind::Dir && &*t[i].lc == "include" && t[i + 1].k == TokKind::Str {
            let name = &*t[i + 1].s;
            if seen.contains(name) || devices::id_from_include_name(util::basename(name)).is_some() {
                continue;
            }
            seen.insert(name.to_string());
            out.push(name.to_string());
        }
    }
    out
}
