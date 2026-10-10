// Negative test for R3 (cross-thread handoff), sequenced case.
//
// The main thread performs an FFI write, then hands the buffer to a spawned
// worker that performs another FFI write, and only after `join` does the main
// thread write once more. Every access is ordered by the spawn/join
// happens-before edges (reinforced by the R3 `release` annotation emitted in
// `main`), so ThreadSanitizer must NOT report a data race.
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

    // Write before the handoff.
    unsafe { c_touch_write(shared, 0) };

    let addr = shared as usize;
    let worker = thread::spawn(move || {
        let p = addr as *mut c_void;
        unsafe { c_touch_write(p, 1) };
    });

    // Join before touching the buffer again: sequenced, not racy.
    worker.join().unwrap();
    unsafe { c_touch_write(shared, 2) };

    unsafe { c_free(shared) };
    println!("norace_spawn_handoff_done");
}
