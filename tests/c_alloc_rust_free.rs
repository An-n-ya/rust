// Cross-language free mismatch, C-alloc / Rust-free direction.
//
// C allocates a buffer (malloc -> header.f_alloc = F_ALLOC_C) and passes it to
// Rust, which then hands it to a Rust `extern "C"` export. That export frees the
// pointer through Rust's allocator. The std dealloc shim in the FFIProbe patch
// detects that f_alloc == F_ALLOC_C and panics:
//   "FFI Probe: undefind behavior: object alloc in C but dealloc in Rust"
//
// This is the mirror of rust_alloc_c_free.rs.
#![feature(ffi_sanitizer)]
#![allow(improper_ctypes_definitions, dead_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::os::raw::{c_void};
use std::ptr;

extern "C" {
    fn c_make(n: usize) -> *mut c_void;
}

// Rust-side export: receives a C-owned pointer and frees it through Rust.
#[no_mangle]
pub extern "C" fn rust_free_c_ptr(p: *mut c_void) {
    let layout = Layout::from_size_align(16, 8).unwrap();
    unsafe { System.dealloc(p as *mut u8, layout) };
}

#[no_mangle]
pub extern "C" fn rust_entry() {
    let p = unsafe { c_make(16) };
    assert!(!p.is_null());
    rust_free_c_ptr(p);
}

fn main() {
    rust_entry();
}
