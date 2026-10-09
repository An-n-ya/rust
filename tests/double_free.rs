// Double free detected by the Rust allocator shim.
//
// A single Box is created and then two independent `Box::from_raw` handles are
// made from the same pointer and both dropped. The std dealloc shim in the
// FFIProbe patch checks the header's f_free flag on the second dealloc and
// panics with "FFI Probe: double free detected".
//
// This does not require a cross-language call: the double-free guard lives in
// the patched System::dealloc, which is invoked whenever Rust frees memory.
#![feature(ffi_sanitizer)]
#![allow(dead_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::ptr;

#[no_mangle]
pub extern "C" fn rust_entry() {
    let layout = Layout::from_size_align(8, 8).unwrap();
    let p = unsafe { System.alloc(layout) };
    assert!(!p.is_null());

    // Wrap the same allocation in two Boxes; dropping both double-frees.
    let b1: Box<u64> = unsafe { Box::from_raw(p as *mut u64) };
    let b2: Box<u64> = unsafe { Box::from_raw(p as *mut u64) };
    drop(b2);
    drop(b1); // second free -> "FFI Probe: double free detected"
}

fn main() {
    rust_entry();
}
