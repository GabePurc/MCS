//! Architecture-neutral description of a program image ready to be loaded into a simulated MCU,
//! plus the debug information (symbols + source line mapping) produced by the assembler, the
//! ELF loader or the Intel HEX loader. Serialized to the UI as camelCase JSON.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProgramFormat {
    Asm,
    Hex,
    Elf,
    /// Hand-written instruction words (`.mc` files).
    #[serde(rename = "mc")]
    MachineCode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    /// Source file path/name (as given to the tool). Empty when not tied to a file.
    pub file: String,
    /// 1-based line, 0 when unknown.
    pub line: u32,
    /// 1-based column, 0 when unknown.
    pub column: u32,
}

impl Diagnostic {
    pub fn new(severity: Severity, message: impl Into<String>, file: impl Into<String>, line: u32, column: u32) -> Self {
        Self { severity, message: message.into(), file: file.into(), line, column }
    }
    pub fn error(message: impl Into<String>, file: impl Into<String>, line: u32, column: u32) -> Self {
        Self::new(Severity::Error, message, file, line, column)
    }
    pub fn warning(message: impl Into<String>, file: impl Into<String>, line: u32, column: u32) -> Self {
        Self::new(Severity::Warning, message, file, line, column)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    Func,
    Object,
    Label,
    Const,
    Section,
    Other,
}

/// Address space a symbol lives in. `Code` addresses are BYTE addresses in program memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolSpace {
    Code,
    Data,
    Eeprom,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramSymbol {
    pub name: String,
    /// Byte address within `space` (data-space address for Data, byte address for Code).
    pub address: u32,
    /// Size in bytes, 0 when unknown.
    pub size: u32,
    pub kind: SymbolKind,
    pub space: SymbolSpace,
    pub global: bool,
}

/// One row of the address -> source mapping. Sorted by address in `LoadedProgram::lines`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineEntry {
    /// Byte address in program memory.
    pub address: u32,
    /// Index into `LoadedProgram::files`.
    pub file: u32,
    /// 1-based line number.
    pub line: u32,
    /// True when the address is a recommended breakpoint/statement start.
    pub is_stmt: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedProgram {
    pub format: ProgramFormat,
    /// Program memory image (bytes, little-endian words for AVR). Unprogrammed bytes are 0xFF.
    pub flash: Vec<u8>,
    /// Number of meaningful bytes from the start of `flash` (highest programmed address + 1).
    pub flash_used: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eeprom: Option<Vec<u8>>,
    /// Fuse bytes (index = fuse number).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuses: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock: Option<Vec<u8>>,
    /// Entry point (byte address).
    pub entry: u32,
    pub symbols: Vec<ProgramSymbol>,
    /// Source file paths referenced by `lines`.
    pub files: Vec<String>,
    pub lines: Vec<LineEntry>,
    /// Device the program was built for, when known (e.g. "attiny10").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

impl LoadedProgram {
    pub fn empty(format: ProgramFormat, flash_size: usize) -> Self {
        Self {
            format,
            flash: vec![0xff; flash_size],
            flash_used: 0,
            eeprom: None,
            fuses: None,
            lock: None,
            entry: 0,
            symbols: Vec::new(),
            files: Vec::new(),
            lines: Vec::new(),
            device: None,
            diagnostics: Vec::new(),
        }
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.severity == Severity::Error)
    }
}
