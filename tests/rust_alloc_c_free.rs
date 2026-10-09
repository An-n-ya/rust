// Cross-language free mismatch: Rust allocates through the global allocator
// (which FFIProbe reroutes to libffisan with F_ALLOC_R), then the memory is
// freed by the C allocator (F_FREE_C).
//
// Detection: the std dealloc shim in the FFIProbe patch panics with
//   "FFI Probe: undefind behavior: object alloc in C but dealloc in Rust"
// when a pointer whose header says f_alloc == F_ALLOC_C is deallocated by Rust,
// and the MIR pass catches the mirror case at the FFI boundary. Here we force
// the C side: Rust allocates, we hand the raw pointer to a C function that
// calls `free()` on it.
//
// Expected at runtime: FFI Probe post-condition / dealloc reports the mismatch.
#![feature(ffi_sanitizer)]
#![allow(dead_code, improper_ctypes_definitions)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::os::raw::c_void;
use std::ptr;

// A C `free` we can call to release a pointer from the "other language".
extern "C" {
    fn free(ptr: *mut c_void);
}

#[no_mangle]
pub extern "C" fn rust_make_and_c_free() {
    // Rust-side allocation via the patched System allocator -> header.f_alloc = R.
    let layout = Layout::from_size_align(8, 8).unwrap();
    let p = unsafe { System.alloc(layout) };
    assert!(!p.is_null());
    // Hand the Rust-owned pointer to C, which frees it (F_FREE_C).
    unsafe { free(p as *mut c_void) };
}

#[no_mangle]
pub extern "C" fn rust_entry() {
    rust_make_and_c_free();
}

fn main() {
    rust_entry();
}
