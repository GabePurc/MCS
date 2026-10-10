// Call stack / stepping test program (Rust, DWARF line info). main() calls middle() which calls leaf() twice and
// stores the running value; the loop never ends.
#![no_std]
#![no_main]

use core::ptr::write_volatile;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

core::arch::global_asm!(
    r#"
    .section .text.start,"ax"
    .globl _start
_start:
    lui sp, 0x3fce0
    call main
1:  j 1b
"#
);

const RESULT: *mut u32 = 0x3fc8_0100 as *mut u32;

#[no_mangle]
#[inline(never)]
pub extern "C" fn leaf(x: u32) -> u32 {
    x.wrapping_mul(3).wrapping_add(1)
}

#[no_mangle]
#[inline(never)]
pub extern "C" fn middle(x: u32) -> u32 {
    let a = leaf(x);
    let b = leaf(a);
    a ^ b
}

#[no_mangle]
pub extern "C" fn main() -> ! {
    let mut v = 1u32;
    loop {
        v = middle(v);
        unsafe { write_volatile(RESULT, v) };
    }
}
