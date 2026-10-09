// Out-of-bounds detection across the FFI boundary.
//
// C allocates a buffer and hands it to Rust as a raw pointer. Rust writes one
// byte past the end of the C allocation. When that pointer is later passed to
// an FFI call, the MIR pass inserts a post-condition check of the red zone,
// which panics with "FFI Probe: out-of-bound detected."
//
// (The C helper is compiled separately and linked via cc; see run.sh.)
#![feature(ffi_sanitizer)]
#![allow(improper_ctypes_definitions, dead_code)]

use std::os::raw::{c_int, c_void};

extern "C" {
    // C-side: allocates `n` bytes with malloc, returns a raw pointer.
    fn c_make_buf(n: usize) -> *mut c_void;
    // A trivial C FFI function Rust will call with the corrupted pointer.
    fn c_touch(_p: *mut c_void) -> c_int;
}

#[no_mangle]
pub extern "C" fn rust_oob() {
    let n = 8usize;
    let p = unsafe { c_make_buf(n) };
    assert!(!p.is_null());
    // Deliberate out-of-bounds write: index `n` is one past the end.
    unsafe {
        let bytes = p as *mut u8;
        *bytes.add(n) = 0x41; // OOB
    }
    // Passing the pointer to an FFI call triggers the red-zone post-condition.
    unsafe { c_touch(p) };
}

#[no_mangle]
pub extern "C" fn rust_entry() {
    rust_oob();
}

fn main() {
    rust_entry();
}
