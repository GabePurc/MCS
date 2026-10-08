//! External toolchain integration (avr-gcc) for C and GNU assembler (.S) sources.
//!
//! Detection searches PATH plus the usual install locations on Windows, macOS and Linux
//! (Microchip/Atmel toolchain, Arduino IDE bundled toolchain, Homebrew, distro packages).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use mcs_core::program::{Diagnostic, Severity};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct ToolchainInfo {
    pub gcc: String,
    pub version: String,
}

pub struct CompileRequest<'a> {
    pub source: &'a str,
    /// File name with extension (.c / .S / .cpp); selects the language and names diagnostics.
    pub file_name: &'a str,
    /// Directory of the saved file so relative #include works; None for untitled documents.
    pub dir: Option<PathBuf>,
    pub mcu: &'a str,
    pub optimize: &'a str,
    pub extra_flags: &'a [String],
    pub gcc_path: Option<&'a str>,
}

pub struct CompileResult {
    pub elf: Option<Vec<u8>>,
    pub output: String,
    pub command: String,
    pub diagnostics: Vec<Diagnostic>,
}

const EXE: &str = if cfg!(windows) { "avr-gcc.exe" } else { "avr-gcc" };

fn list_dirs(p: &Path) -> Vec<String> {
    std::fs::read_dir(p)
        .map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().into_string().ok()).collect())
        .unwrap_or_default()
}

fn arduino_tools(base: PathBuf, out: &mut Vec<PathBuf>) {
    let tools = base.join("packages/arduino/tools/avr-gcc");
    let mut versions = list_dirs(&tools);
    versions.sort();
    for v in versions.into_iter().rev() {
        out.push(tools.join(v).join("bin"));
    }
}

fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    if cfg!(target_os = "macos") {
        for base in ["/opt/homebrew", "/usr/local"] {
            dirs.push(PathBuf::from(base).join("bin"));
            let opt = PathBuf::from(base).join("opt");
            for v in list_dirs(&opt).into_iter().filter(|v| v.starts_with("avr-gcc")) {
                dirs.push(opt.join(v).join("bin"));
            }
        }
        arduino_tools(home.join("Library/Arduino15"), &mut dirs);
    } else if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            let Some(base) = std::env::var_os(var).map(PathBuf::from) else { continue };
            let studio = base.join("Atmel").join("Studio");
            for v in list_dirs(&studio) {
                dirs.push(studio.join(v).join("toolchain/avr8/avr8-gnu-toolchain/bin"));
            }
            let mchp = base.join("Microchip");
            for v in list_dirs(&mchp) {
                dirs.push(mchp.join(&v).join("bin"));
                dirs.push(mchp.join(&v).join("avr8-gnu-toolchain-win32_x86_64").join("bin"));
            }
            dirs.push(base.join("Arduino/hardware/tools/avr/bin"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            arduino_tools(PathBuf::from(local).join("Arduino15"), &mut dirs);
        }
        dirs.push(PathBuf::from("C:\\avr-gcc\\bin"));
        dirs.push(PathBuf::from("C:\\WinAVR\\bin"));
    } else {
        for d in ["/usr/bin", "/usr/local/bin", "/opt/avr-gcc/bin"] {
            dirs.push(PathBuf::from(d));
        }
        arduino_tools(home.join(".arduino15"), &mut dirs);
    }
    dirs
}

fn version_of(gcc: &Path) -> Option<String> {
    let out = Command::new(gcc).arg("--version").output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").trim().to_string())
}

pub fn detect(preferred: Option<&str>) -> Option<ToolchainInfo> {
    let mut seen = std::collections::HashSet::new();
    let candidates = preferred.filter(|p| !p.is_empty()).map(PathBuf::from).into_iter().chain(candidate_dirs().into_iter().map(|d| d.join(EXE)));
    for c in candidates {
        if !seen.insert(c.clone()) || !c.is_file() {
            continue;
        }
        if let Some(version) = version_of(&c) {
            return Some(ToolchainInfo { gcc: c.to_string_lossy().into_owned(), version });
        }
    }
    None
}

fn temp_dir() -> std::io::Result<PathBuf> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("mcs-build-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn compile(req: &CompileRequest) -> CompileResult {
    let Some(tc) = detect(req.gcc_path) else {
        let msg = "avr-gcc was not found. Install an AVR GCC toolchain (see Help > Toolchain Setup) or set its path in Tools > Toolchain Options.";
        return CompileResult { elf: None, output: msg.into(), command: String::new(), diagnostics: vec![Diagnostic::error(msg, "", 0, 0)] };
    };
    let tmp = match temp_dir() {
        Ok(t) => t,
        Err(e) => return CompileResult { elf: None, output: e.to_string(), command: String::new(), diagnostics: vec![] },
    };
    let base = Path::new(req.file_name).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "main.c".into());
    let src = tmp.join(&base);
    let out = tmp.join("program.elf");
    let result = (|| {
        std::fs::write(&src, req.source).map_err(|e| e.to_string())?;
        let ext = Path::new(&base).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        let mut args: Vec<String> = vec![format!("-mmcu={}", req.mcu), format!("-{}", req.optimize), "-g".into(), "-gdwarf-4".into(), "-Wall".into(), "-Wextra".into()];
        match ext.as_str() {
            "cpp" | "cc" => args.push("-std=gnu++17".into()),
            "s" | "S" | "sx" => {}
            _ => args.push("-std=gnu11".into()),
        }
        if let Some(d) = &req.dir {
            args.push(format!("-I{}", d.display()));
        }
        // Debug info should name the user's file, not the temporary copy we compile.
        let real_dir = req.dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| ".".into());
        args.push(format!("-ffile-prefix-map={}={}", tmp.display(), real_dir));
        args.extend(req.extra_flags.iter().filter(|f| !f.is_empty()).cloned());
        args.push("-o".into());
        args.push(out.to_string_lossy().into_owned());
        args.push(src.to_string_lossy().into_owned());
        let output = Command::new(&tc.gcc).args(&args).current_dir(req.dir.as_deref().unwrap_or(&tmp)).output().map_err(|e| e.to_string())?;
        // Report diagnostics against the user's file instead of the temp copy.
        let shown = req.dir.as_ref().map(|d| d.join(&base).to_string_lossy().into_owned()).unwrap_or(base.clone());
        let src_s = src.to_string_lossy().into_owned();
        let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)).replace(&src_s, &shown);
        let gcc_name = Path::new(&tc.gcc).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let command = format!(
            "{gcc_name} {}",
            args.iter().map(|a| if *a == src_s { shown.clone() } else if a.ends_with("program.elf") { "program.elf".into() } else { a.clone() }).collect::<Vec<_>>().join(" ")
        );
        let diagnostics = parse_gcc_diagnostics(&text);
        if !output.status.success() || !out.is_file() {
            return Ok(CompileResult { elf: None, output: text, command, diagnostics });
        }
        let elf = std::fs::read(&out).map_err(|e| e.to_string())?;
        let mut text = text;
        // avr-size normally sits next to avr-gcc; Homebrew's keg-only avr-gcc keeps binutils elsewhere.
        let size_exe = if cfg!(windows) { "avr-size.exe" } else { "avr-size" };
        let size_tool = std::iter::once(Path::new(&tc.gcc).with_file_name(size_exe))
            .chain(candidate_dirs().into_iter().map(|d| d.join(size_exe)))
            .find(|p| p.is_file());
        if let Some(size_tool) = size_tool {
            if let Ok(o) = Command::new(size_tool).arg(&out).output() {
                text.push_str(&String::from_utf8_lossy(&o.stdout).replace(&out.to_string_lossy().into_owned(), "program.elf"));
            }
        }
        Ok(CompileResult { elf: Some(elf), output: text, command, diagnostics })
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result.unwrap_or_else(|e: String| CompileResult { elf: None, output: e.clone(), command: String::new(), diagnostics: vec![Diagnostic::error(e, "", 0, 0)] })
}

/// Parses `file:line:col: error: message` lines from GCC output.
pub fn parse_gcc_diagnostics(text: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for l in text.lines() {
        for (marker, sev) in [(": error: ", Severity::Error), (": fatal error: ", Severity::Error), (": warning: ", Severity::Warning), (": note: ", Severity::Info)] {
            let Some(pos) = l.find(marker) else { continue };
            let head = &l[..pos];
            let msg = &l[pos + marker.len()..];
            // head = file:line[:col]   (file may contain ':' on Windows, so parse from the right)
            let mut parts = head.rsplitn(3, ':');
            let a = parts.next().unwrap_or("");
            let b = parts.next().unwrap_or("");
            let rest = parts.next();
            let (file, line, col) = match (rest, b.parse::<u32>(), a.parse::<u32>()) {
                (Some(f), Ok(line), Ok(col)) => (f.to_string(), line, col),
                _ => match a.parse::<u32>() {
                    Ok(line) => (format!("{}{}", rest.map(|r| format!("{r}:")).unwrap_or_default(), b), line, 0),
                    Err(_) => (head.to_string(), 0, 0),
                },
            };
            out.push(Diagnostic::new(sev, msg, file, line, col));
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gcc_lines() {
        let d = parse_gcc_diagnostics("main.c:12:5: error: 'x' undeclared\nC:\\p\\a.c:3:1: warning: unused\nfoo.c:7: note: here");
        assert_eq!(d.len(), 3);
        assert_eq!((d[0].file.as_str(), d[0].line, d[0].column), ("main.c", 12, 5));
        assert_eq!(d[1].file, "C:\\p\\a.c");
        assert_eq!(d[1].severity, Severity::Warning);
        assert_eq!((d[2].file.as_str(), d[2].line), ("foo.c", 7));
    }
}

/// End-to-end C tests (skipped when avr-gcc is not installed).
#[cfg(test)]
mod c_tests {
    use super::*;
    use mcs_core::avr::devices;
    use mcs_core::program::LoadedProgram;
    use mcs_sim::avr::Machine;
    use mcs_sim::protocol::{Command, StepKind};
    use mcs_sim::session::Session;

    fn build(example: &str) -> Option<LoadedProgram> {
        build_for(example, "attiny10")
    }

    fn build_for(example: &str, mcu: &str) -> Option<LoadedProgram> {
        detect(None)?;
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples");
        let source = std::fs::read_to_string(dir.join(example)).unwrap();
        let r = compile(&CompileRequest { source: &source, file_name: example, dir: Some(dir), mcu, optimize: "Os", extra_flags: &[], gcc_path: None });
        let elf = r.elf.unwrap_or_else(|| panic!("{example} failed to compile:\n{}", r.output));
        let out = mcs_api::program_from_elf(&elf, example, mcu, r.diagnostics, r.output);
        assert!(out.ok, "{example}: {:#?}", out.diagnostics);
        out.program
    }

    #[test]
    fn atmega328p_serial_c_talks_through_the_usart() {
        let Some(p) = build_for("m328p_serial.c", "atmega328p") else { return eprintln!("avr-gcc not found: skipping") };
        let mut m = Machine::new(devices::get("atmega328p").unwrap());
        m.load(&p);
        // TXD = PD1 (GPIO 16), RXD = PD0 (GPIO 15).
        m.set_serial(mcs_sim::avr::peripherals::serial::SerialConfig { monitor: Some(16), inject: Some(15), baud: 9600.0, data_bits: 8, parity: 0, stop_bits: 1 });
        m.run(100_000);
        let text = String::from_utf8_lossy(&m.sys.serial_out).into_owned();
        assert!(text.starts_with("Hello from the ATmega328P!"), "{text:?}");
        m.sys.serial_out.clear();
        m.serial_send(b"abc");
        let c = m.cpu.cycles;
        m.run(c + 50_000);
        assert_eq!(m.sys.serial_out, b"abc");
        assert_eq!(m.sys.pins[5].level, 1, "three key presses toggled the LED an odd number of times");
    }

    #[test]
    fn attiny85_blink_c_uses_timer0_overflow() {
        let Some(p) = build_for("t85_blink.c", "attiny85") else { return eprintln!("avr-gcc not found: skipping") };
        let mut m = Machine::new(devices::get("attiny85").unwrap());
        m.load(&p);
        m.run(2_000_000);
        let (_, c, l) = m.sys.trace.read_since(0, usize::MAX);
        let e: Vec<u64> = (1..c.len()).filter(|&i| (l[i] ^ l[i - 1]) >> 3 & 1 == 1).map(|i| c[i]).collect();
        assert!(e.len() >= 3, "{e:?}");
        // 2 overflows of 256 x 1024 cycles per toggle.
        assert_eq!(e[2] - e[1], 2 * 256 * 1024);
    }

    fn edges(m: &Machine) -> Vec<u64> {
        let (_, c, l) = m.sys.trace.read_since(0, usize::MAX);
        (1..c.len()).filter(|&i| (l[i] ^ l[i - 1]) & 1 == 1).map(|i| c[i]).collect()
    }

    #[test]
    fn blink_c_compiles_maps_lines_and_runs() {
        let Some(p) = build("blink.c") else { return eprintln!("avr-gcc not found: skipping") };
        assert_eq!(p.device.as_deref(), Some("attiny10"));
        assert!(p.files.iter().any(|f| f.ends_with("blink.c")), "{:?}", p.files);
        assert!(p.lines.iter().any(|l| l.line == 14), "no row for the PINB toggle line");
        assert!(p.symbols.iter().any(|s| s.name == "main"));
        let mut m = Machine::new(devices::get("attiny10").unwrap());
        m.load(&p);
        m.run(1_000_000);
        let e = edges(&m);
        assert!(e.len() >= 8, "{e:?}");
        let period = e[4] - e[3];
        assert!((99_000..=101_500).contains(&period), "toggle period {period} cycles");
    }

    #[test]
    fn pwm_fade_c_uses_the_timer_interrupt() {
        let Some(p) = build("pwm_fade.c") else { return eprintln!("avr-gcc not found: skipping") };
        assert!(p.symbols.iter().any(|s| s.name == "__vector_4"), "TIM0_OVF ISR missing");
        let mut m = Machine::new(devices::get("attiny10").unwrap());
        m.load(&p);
        m.run(2_000_000);
        assert!(edges(&m).len() > 1000);
        // The ISR keeps changing the duty cycle (OCR0A) while the PWM runs.
        let d1 = m.peek_data(0x26);
        m.run(2_050_000);
        assert_ne!(d1, m.peek_data(0x26));
    }

    #[test]
    fn c_source_level_stepping() {
        let Some(p) = build("blink.c") else { return eprintln!("avr-gcc not found: skipping") };
        let mut s = Session::new();
        s.handle(Command::Load { device_id: "attiny10".into(), program: Box::new(p.clone()) });
        s.handle(Command::SetSpeed { mode: mcs_sim::protocol::SpeedMode::Max, factor: 1.0 });
        let mut lines = Vec::new();
        for _ in 0..6 {
            s.handle(Command::Step { kind: StepKind::Over, source: true });
            for _ in 0..10_000 {
                if !s.is_running() {
                    break;
                }
                s.slice();
            }
            assert!(!s.is_running(), "step did not finish");
            let pc = s.machine().unwrap().cpu.pc * 2;
            // First row at the closest address <= pc (the call site wins over inlined rows).
            let best = p.lines.iter().filter(|r| r.address <= pc).map(|r| r.address).max().unwrap();
            let at: Vec<_> = p.lines.iter().filter(|r| r.address == best && r.is_stmt).collect();
            let row = at.iter().rev().find(|r| r.file == at[0].file).unwrap();
            assert!(p.files[row.file as usize].ends_with("blink.c"), "stopped in {}", p.files[row.file as usize]);
            lines.push(row.line);
        }
        // From reset: main's first statement (11), the toggle (14), the inlined delay (15), then
        // the for(;;) loop-back branch (13) and around again.
        assert_eq!(lines, vec![11, 14, 15, 13, 14, 15]);
        assert!(lines.iter().all(|&l| l <= 20), "stepped into a header: {lines:?}");
    }
}
