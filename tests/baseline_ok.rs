// Baseline / negative test: correct FFI usage must NOT trigger any FFIProbe
// diagnostic. Rust allocates with the global allocator, passes the pointer to a
// C function that reads/writes in-bounds, then the pointer is freed by Rust.
// Expected: the program runs to completion with no "FFI Probe:" output.
#![feature(ffi_sanitizer)]
#![allow(improper_ctypes_definitions, dead_code)]

use std::os::raw::{c_int, c_void};

extern "C" {
    fn c_make_buf(n: usize) -> *mut c_void;
    fn c_touch(_p: *const c_void) -> c_int;
    fn c_free(_p: *mut c_void);
}

#[no_mangle]
pub extern "C" fn rust_ok() {
    let n = 8usize;
    let p = unsafe { c_make_buf(n) };
    assert!(!p.is_null());
    // In-bounds write.
    unsafe {
        let bytes = p as *mut u8;
        *bytes.add(n - 1) = 0x41; // in-bounds
    }
    // Pass to FFI (triggers pre/post conditions, all clean here).
    let _ = unsafe { c_touch(p) };
    // Same language frees: C alloc, C free -> consistent, no panic.
    unsafe { c_free(p) };
}

#[no_mangle]
pub extern "C" fn rust_entry() {
    rust_ok();
}

fn main() {
    rust_entry();
    println!("baseline_ok: no FFIProbe diagnostics expected");
}
