//! Wall-clock source for real-time pacing (milliseconds, monotonic). Native builds use
//! `Instant`; WebAssembly builds import `performance.now()` from the host page.

#[cfg(not(target_arch = "wasm32"))]
pub fn now_ms() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

#[cfg(target_arch = "wasm32")]
pub fn now_ms() -> f64 {
    #[link(wasm_import_module = "env")]
    extern "C" {
        fn mcs_now_ms() -> f64;
    }
    // SAFETY: plain host function import without arguments.
    unsafe { mcs_now_ms() }
}
