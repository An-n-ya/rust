// Positive test for R3 (cross-thread handoff), concurrent case.
//
// A heap buffer's address is handed to two spawned closures that both call the
// C function `c_touch_write` on it. There is no ordering between the two
// workers, so the linked ThreadSanitizer runtime must report a data race on
// the FFI accesses inside the closures.
//
// NOTE: raw pointers are `!Send`, so a closure passed to `thread::spawn`
// cannot capture one directly; the tests smuggle the address through `usize`.
// This means the closure has no raw-pointer upvar and the R3 `acquire` half is
// not exercised here (see norace_spawn_handoff.rs and RESULTS.md), while the R3
// `release` half is still emitted in `main`, which holds the raw pointer local.
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
    println!("race_spawn_concurrent_done");
}
