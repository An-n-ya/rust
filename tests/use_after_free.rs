// Use-after-free across FFI: C allocates, C frees, then the dangling pointer is
// passed to a Rust extern "C" function that uses it. The MIR pass inserts a
// pre-condition check on the FFI call, and the ffi_sanitizer_ffi_pre_cond shim
// panics with "FFI Probe: use-after-free ...".
#![feature(ffi_sanitizer)]
#![allow(improper_ctypes_definitions, dead_code)]

use std::os::raw::{c_void};

extern "C" {
    fn c_make(n: usize) -> *mut c_void;
    fn c_free(_p: *mut c_void);
}

#[no_mangle]
pub extern "C" fn rust_use_after_free(p: *mut c_void) -> u8 {
    // Reading through the pointer; the pre-condition on this FFI call fires.
    unsafe { *(p as *const u8) }
}

#[no_mangle]
pub extern "C" fn rust_entry() {
    let p = unsafe { c_make(8) };
    assert!(!p.is_null());
    unsafe { c_free(p) }; // freed
    // Use-after-free: pass dangling pointer into a Rust FFI export.
    let _ = rust_use_after_free(p);
}

fn main() {
    rust_entry();
}
