//! MCS desktop application: Tauri commands that expose the Rust simulator, assembler, file
//! loaders and avr-gcc integration to the web UI. The simulation runs on its own thread and
//! streams state snapshots to the UI through a Tauri channel.

mod toolchain;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mcs_api::{BuildOutcome, DeviceSummary, DisasmLine, InsnInfo, McAnnotations};
use mcs_sim::protocol::{Command, Output};
use mcs_sim::session::{self, SessionThread};
use tauri::ipc::Channel;
use tauri::{Manager, State};

#[derive(Default)]
struct AppState {
    sim: Mutex<Option<SessionThread>>,
}

/// Starts (or restarts) the simulation thread; state is streamed through `channel`.
#[tauri::command]
fn sim_attach(state: State<AppState>, channel: Channel<Output>) {
    let mut sim = state.sim.lock().unwrap();
    *sim = None; // joins the previous thread
    *sim = Some(session::spawn(move |out| {
        let _ = channel.send(out);
    }));
}

#[tauri::command]
fn sim_command(state: State<AppState>, cmd: Command) {
    if let Some(s) = state.sim.lock().unwrap().as_ref() {
        s.send(cmd);
    }
}

#[tauri::command]
fn list_devices() -> Vec<DeviceSummary> {
    mcs_api::list_devices()
}

/// Collects `.include` files (recursively) relative to the source directory.
fn load_includes(source: &str, dir: Option<&Path>, out: &mut HashMap<String, String>, depth: u32) {
    if depth > 16 {
        return;
    }
    for name in mcs_api::user_includes(source) {
        if out.contains_key(&name) {
            continue;
        }
        let path = match dir {
            Some(d) => d.join(&name),
            None => PathBuf::from(&name),
        };
        if let Ok(text) = std::fs::read_to_string(&path) {
            out.insert(name, text.clone());
            load_includes(&text, path.parent(), out, depth + 1);
        }
    }
}

#[tauri::command]
fn build_asm(source: String, file_name: String, file_path: Option<String>, device_id: String) -> BuildOutcome {
    let dir = file_path.as_deref().and_then(|p| Path::new(p).parent());
    let mut includes = HashMap::new();
    load_includes(&source, dir, &mut includes, 0);
    let name = file_path.clone().unwrap_or(file_name);
    mcs_api::build_asm(&source, &name, &device_id, &includes)
}

#[tauri::command]
fn build_machine_code(source: String, file_name: String, file_path: Option<String>, device_id: String) -> BuildOutcome {
    mcs_api::build_machine_code(&source, file_path.as_deref().unwrap_or(&file_name), &device_id)
}

#[tauri::command]
fn machine_code_hints(source: String, device_id: String) -> McAnnotations {
    mcs_api::machine_code_hints(&source, &device_id)
}

#[tauri::command]
fn program_to_machine_code(device_id: String, flash: Vec<u8>, used: usize, labels: HashMap<u32, String>, title: String) -> String {
    mcs_api::program_to_machine_code(&device_id, &flash, used, &labels, &title)
}

#[tauri::command]
async fn build_c(source: String, file_name: String, file_path: Option<String>, device_id: String, optimize: String, extra_flags: Vec<String>, gcc_path: Option<String>) -> BuildOutcome {
    // The toolchain wrapper drives avr-gcc; ARM devices load an ELF / HEX built elsewhere for now.
    if mcs_core::devices::get_any(&device_id).is_some_and(|d| d.as_avr().is_none()) {
        let message = "C builds target AVR devices (avr-gcc). For an ARM Cortex-M device, build with arm-none-eabi-gcc or clang and load the ELF or Intel HEX file (File > Import HEX/ELF)".to_string();
        let diagnostics = vec![mcs_core::program::Diagnostic::new(mcs_core::program::Severity::Error, message, file_name, 0, 0)];
        return BuildOutcome { ok: false, program: None, diagnostics, output: String::new(), listing: None, device_id };
    }
    let dir = file_path.as_deref().and_then(|p| Path::new(p).parent()).map(Path::to_path_buf);
    let r = toolchain::compile(&toolchain::CompileRequest {
        source: &source,
        file_name: &file_name,
        dir,
        mcu: &device_id,
        optimize: &optimize,
        extra_flags: &extra_flags,
        gcc_path: gcc_path.as_deref(),
    });
    let output = if r.command.is_empty() { r.output } else { format!("{}\n{}", r.command, r.output) };
    match r.elf {
        Some(elf) => mcs_api::program_from_elf(&elf, &file_name, &device_id, r.diagnostics, output),
        None => BuildOutcome { ok: false, program: None, diagnostics: r.diagnostics, output, listing: None, device_id },
    }
}

#[tauri::command]
fn import_program(path: String, device_id: String) -> Result<BuildOutcome, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let name = Path::new(&path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(mcs_api::import_program(&bytes, &name, &device_id))
}

/// Parses an ELF / Intel HEX image that is already in memory (bundled examples).
#[tauri::command]
fn import_program_bytes(bytes: Vec<u8>, file_name: String, device_id: String) -> BuildOutcome {
    mcs_api::import_program(&bytes, &file_name, &device_id)
}

#[tauri::command]
fn detect_toolchain(gcc_path: Option<String>) -> Option<toolchain::ToolchainInfo> {
    toolchain::detect(gcc_path.as_deref())
}

#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))
}

#[tauri::command]
fn write_text_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| format!("{path}: {e}"))
}

#[tauri::command]
fn export_hex(path: String, flash: Vec<u8>, used: usize) -> Result<(), String> {
    let text = mcs_formats::to_intel_hex(&flash, 0, used.min(flash.len()));
    std::fs::write(&path, text).map_err(|e| format!("{path}: {e}"))
}

#[tauri::command]
fn disassemble(device_id: String, flash: Vec<u8>, labels: HashMap<u32, String>) -> Vec<DisasmLine> {
    mcs_api::disassemble(&device_id, &flash, &labels)
}

#[tauri::command]
fn instruction_set(device_id: String) -> Vec<InsnInfo> {
    mcs_api::instruction_set(&device_id)
}

#[tauri::command]
fn register_custom_devices(configs: Vec<mcs_api::CustomMcuConfig>) -> Vec<mcs_api::CustomRegistration> {
    mcs_api::register_custom_devices(&configs)
}

#[tauri::command]
fn custom_device_preview(config: mcs_api::CustomMcuConfig) -> Result<mcs_api::CustomPreview, String> {
    mcs_api::custom_device_preview(&config)
}

#[tauri::command]
fn custom_device_defaults() -> mcs_api::CustomMcuConfig {
    mcs_api::custom_device_defaults()
}

#[tauri::command]
fn def_include(device_id: String) -> Option<(String, String)> {
    mcs_api::def_include(&device_id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState::default())
        .setup(|app| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
            }
            Ok(())
        })
        // Pop-out tool windows belong to the main window: quit when it goes away.
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                window.app_handle().exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            sim_attach,
            sim_command,
            list_devices,
            build_asm,
            build_machine_code,
            machine_code_hints,
            program_to_machine_code,
            build_c,
            import_program,
            import_program_bytes,
            detect_toolchain,
            read_text_file,
            write_text_file,
            export_hex,
            disassemble,
            instruction_set,
            register_custom_devices,
            custom_device_preview,
            custom_device_defaults,
            def_include,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MCS");
}
