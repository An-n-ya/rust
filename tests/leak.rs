// Leak detection: a Rust-allocated buffer is passed to a C FFI call (which
// registers it in FFIProbe's allocation list), and then the program exits
// without freeing it. At exit, `ffi_sanitizer_exit_pre_cond` reports the leak.
//
// Expected at runtime: "FFI Probe: memory leak detected".
#![feature(ffi_sanitizer)]
#![allow(dead_code, improper_ctypes_definitions)]

use std::os::raw::c_void;

extern "C" {
    fn c_touch(_p: *const c_void) -> i32;
}

#[no_mangle]
pub extern "C" fn rust_entry() {
    let layout = std::alloc::Layout::from_size_align(8, 8).unwrap();
    let p = unsafe { std::alloc::alloc(layout) };
    assert!(!p.is_null());
    // Passing the pointer to an FFI call registers it for leak tracking.
    unsafe { let _ = c_touch(p as *const c_void); }
    // Intentionally do NOT free `p` -> leak.
}

fn main() {
    rust_entry();
}
