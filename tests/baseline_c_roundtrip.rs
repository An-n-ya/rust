// Negative test #2: C allocates, C frees, and the pointer is passed through a
// Rust extern "C" export that does NOT touch the memory in a way that triggers a
// check. This exercises the FFI boundary instrumentation without a real bug.
// Expected: no "FFI Probe:" diagnostic, exit 0.
#![feature(ffi_sanitizer)]
#![allow(dead_code, improper_ctypes_definitions)]

use std::os::raw::c_void;

extern "C" {
    fn c_make(n: usize) -> *mut c_void;
    fn c_free(_p: *mut c_void);
}

// Rust export that receives a C pointer and immediately hands it back to C free.
#[no_mangle]
pub extern "C" fn rust_relay_c_free(p: *mut c_void) {
    unsafe { c_free(p) };
}

#[no_mangle]
pub extern "C" fn rust_entry() {
    let p = unsafe { c_make(16) };
    assert!(!p.is_null());
    rust_relay_c_free(p);
}

fn main() {
    rust_entry();
    println!("baseline_c_roundtrip_ok");
}
