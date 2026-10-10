//! WebAssembly adapter: exposes the simulator session and the build/disassembly services to
//! JavaScript through a tiny JSON-over-linear-memory ABI (no wasm-bindgen needed):
//!
//! * `mcs_alloc(len) -> ptr` / `mcs_free(ptr, len)` manage buffers in WASM memory.
//! * `mcs_call(ptr, len) -> u64` takes a JSON request `{"method": ..., ...}` and returns
//!   `(out_ptr << 32) | out_len` of a JSON response the caller must free.
//! * The host provides `env.mcs_now_ms() -> f64` (performance.now) for real-time pacing.

use std::cell::RefCell;
use std::collections::HashMap;

use mcs_sim::protocol::Command;
use mcs_sim::session::Session;
use serde::Deserialize;
use serde_json::{json, Value};

thread_local! {
    static SESSION: RefCell<Session> = RefCell::new(Session::new());
}

#[derive(Deserialize)]
#[serde(tag = "method", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Request {
    /// Simulator command; returns the produced outputs.
    Sim { cmd: Command },
    /// Runs one time slice; returns outputs and the suggested delay before the next slice.
    Slice,
    ListDevices,
    BuildAsm { source: String, file_name: String, device_id: String, #[serde(default)] includes: HashMap<String, String> },
    BuildMachineCode { source: String, file_name: String, device_id: String },
    MachineCodeHints { source: String, device_id: String },
    /// Label keys are code byte addresses as strings (JSON object keys).
    ProgramToMachineCode { device_id: String, flash: Vec<u8>, used: usize, labels: HashMap<String, String>, title: String },
    ImportProgram { bytes: Vec<u8>, file_name: String, device_id: String },
    /// Label keys are code byte addresses as strings (JSON object keys).
    Disassemble { device_id: String, flash: Vec<u8>, labels: HashMap<String, String> },
    InstructionSet { device_id: String },
    DefInclude { device_id: String },
    ToIntelHex { flash: Vec<u8>, used: usize },
    RegisterCustomDevices { configs: Vec<mcs_api::CustomMcuConfig> },
    CustomDevicePreview { config: mcs_api::CustomMcuConfig },
    CustomDeviceDefaults,
}

fn dispatch(req: Request) -> Value {
    match req {
        Request::Sim { cmd } => SESSION.with(|s| json!({ "outputs": s.borrow_mut().handle(cmd), "idleMs": s.borrow().idle_ms() })),
        Request::Slice => SESSION.with(|s| {
            let mut s = s.borrow_mut();
            let outputs = s.slice();
            json!({ "outputs": outputs, "idleMs": s.idle_ms(), "running": s.is_running() })
        }),
        Request::ListDevices => json!(mcs_api::list_devices()),
        Request::BuildAsm { source, file_name, device_id, includes } => json!(mcs_api::build_asm(&source, &file_name, &device_id, &includes)),
        Request::BuildMachineCode { source, file_name, device_id } => json!(mcs_api::build_machine_code(&source, &file_name, &device_id)),
        Request::MachineCodeHints { source, device_id } => json!(mcs_api::machine_code_hints(&source, &device_id)),
        Request::ProgramToMachineCode { device_id, flash, used, labels, title } => {
            let labels: HashMap<u32, String> = labels.into_iter().filter_map(|(k, v)| k.parse().ok().map(|k| (k, v))).collect();
            json!(mcs_api::program_to_machine_code(&device_id, &flash, used, &labels, &title))
        }
        Request::ImportProgram { bytes, file_name, device_id } => json!(mcs_api::import_program(&bytes, &file_name, &device_id)),
        Request::Disassemble { device_id, flash, labels } => {
            let labels: HashMap<u32, String> = labels.into_iter().filter_map(|(k, v)| k.parse().ok().map(|k| (k, v))).collect();
            json!(mcs_api::disassemble(&device_id, &flash, &labels))
        }
        Request::InstructionSet { device_id } => json!(mcs_api::instruction_set(&device_id)),
        Request::DefInclude { device_id } => json!(mcs_api::def_include(&device_id)),
        Request::ToIntelHex { flash, used } => json!(mcs_api::to_intel_hex(&flash, used)),
        Request::RegisterCustomDevices { configs } => json!(mcs_api::register_custom_devices(&configs)),
        Request::CustomDevicePreview { config } => match mcs_api::custom_device_preview(&config) {
            Ok(p) => json!({ "ok": p }),
            Err(e) => json!({ "error": e }),
        },
        Request::CustomDeviceDefaults => json!(mcs_api::custom_device_defaults()),
    }
}

/// Handles one JSON request (also usable from native tests).
pub fn call_json(input: &str) -> String {
    let out = match serde_json::from_str::<Request>(input) {
        Ok(req) => json!({ "ok": true, "result": dispatch(req) }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    };
    out.to_string()
}

#[no_mangle]
pub extern "C" fn mcs_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr`/`len` must come from `mcs_alloc` or a `mcs_call` result and be freed once.
#[no_mangle]
pub unsafe extern "C" fn mcs_free(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
}

/// # Safety
/// `ptr` must point to `len` readable bytes of UTF-8 JSON (allocated with `mcs_alloc`).
#[no_mangle]
pub unsafe extern "C" fn mcs_call(ptr: *const u8, len: usize) -> u64 {
    let input = std::str::from_utf8(std::slice::from_raw_parts(ptr, len)).unwrap_or("");
    let mut out = call_json(input).into_bytes();
    out.shrink_to_fit();
    let (p, l) = (out.as_mut_ptr(), out.len());
    // Capacity equals length after shrink_to_fit, so mcs_free(p, l) reconstructs it exactly.
    std::mem::forget(out);
    ((p as u64) << 32) | l as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip() {
        let r: Value = serde_json::from_str(&call_json(r#"{"method":"listDevices"}"#)).unwrap();
        assert!(r["result"].as_array().unwrap().iter().any(|d| d["id"] == "attiny10"));
        let r: Value = serde_json::from_str(&call_json(r#"{"method":"buildAsm","source":"ldi r16, 1\nrjmp 0","fileName":"t.asm","deviceId":"attiny10"}"#)).unwrap();
        assert_eq!(r["result"]["ok"], true);
        let r: Value = serde_json::from_str(&call_json(r#"{"method":"sim","cmd":{"type":"init","deviceId":"attiny10"}}"#)).unwrap();
        assert!(r["result"]["outputs"].as_array().unwrap().len() >= 2);
        let r: Value = serde_json::from_str(&call_json(r#"{"method":"disassemble","deviceId":"attiny10","flash":[0,192],"labels":{"0":"start"}}"#)).unwrap();
        assert_eq!(r["result"][0]["mnemonic"], "rjmp");
        let r: Value = serde_json::from_str(&call_json(r#"{"method":"nope"}"#)).unwrap();
        assert_eq!(r["ok"], false);
    }
}
