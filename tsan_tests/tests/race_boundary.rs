// Positive test for R1 (FFI call boundary read/write).
//
// A heap buffer is shared by two spawned threads that both call the C function
// `c_touch_write` on the same `*mut c_void` with no synchronization. The MIR
// pass instruments each FFI call with `ffi_race_write` (the argument is a
// `*mut`), so the linked ThreadSanitizer runtime sees two concurrent,
// unsynchronized writes to the same address and reports a data race.
//
// No hand-written __tsan_* annotation is used: detection must come purely from
// the compiler-inserted instrumentation.
#![feature(ffi_race_sanitizer)]
#![allow(dead_code)]

use std::ffi::c_void;
use std::os::raw::c_int;
use std::thread;

extern "C" {
    fn c_make(n: usize) -> *mut c_void;
    fn c_free(p: *mut c_void);
    fn c_touch_write(p: *mut c_void, v: c_int);
}

fn main() {
    let shared = unsafe { c_make(8) };
    assert!(!shared.is_null());
    // Raw pointers are not `Send`, so hand the address to the threads.
    let addr = shared as usize;

    let a = thread::spawn(move || {
        let p = addr as *mut c_void;
        for i in 0..16 {
            unsafe { c_touch_write(p, i) };
        }
    });

    let b = thread::spawn(move || {
        let p = addr as *mut c_void;
        for i in 0..16 {
            unsafe { c_touch_write(p, i) };
        }
    });

    a.join().unwrap();
    b.join().unwrap();

    unsafe { c_free(shared) };
    println!("race_boundary_done");
}
