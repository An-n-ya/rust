// Negative test for R1 (FFI call boundary read/write).
//
// Both the spawned thread and the main thread call `c_touch_write` on the same
// pointer, but they are ordered by `join`: the worker runs first and the main
// thread performs its FFI write only after `t.join()` returns. The MIR pass
// still instruments both FFI calls with `ffi_race_write`, but the join creates
// a happens-before edge, so ThreadSanitizer must NOT report a data race.
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

    let worker = thread::spawn(move || {
        let p = addr as *mut c_void;
        unsafe { c_touch_write(p, 1) };
    });

    // Join before touching the buffer from the main thread: sequenced, not racy.
    worker.join().unwrap();
    unsafe { c_touch_write(shared, 2) };

    unsafe { c_free(shared) };
    println!("norace_sequenced_done");
}
