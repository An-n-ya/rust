// Negative test for R4 (known lock-protected C API).
//
// Two spawned threads call `c_locked_touch` on the same `*mut c_void`. That C
// entry point is listed in the pass's `KNOWN_LOCKED_FFI_APIS`, so the MIR pass
// brackets the FFI call with `ffi_race_mutex_pre_lock`/`post_lock` before it and
// `ffi_race_mutex_pre_unlock`/`post_unlock` after it, using the data pointer as
// the mutex identity. Every thread passes the same address, so the linked
// ThreadSanitizer runtime treats the accesses as mutex-protected and does NOT
// report a race -- even though the writes really are concurrent at the CPU
// level, the annotations tell TSan they are ordered.
//
// (The matching control case removes `c_locked_touch` from the pass's API list;
// with no mutex annotations the same program is reported as a data race.)
#![feature(ffi_race_sanitizer)]
#![allow(dead_code)]

use std::ffi::c_void;
use std::os::raw::c_int;
use std::thread;

extern "C" {
    fn c_make(n: usize) -> *mut c_void;
    fn c_free(p: *mut c_void);
    fn c_locked_touch(p: *mut c_void, v: c_int);
}

fn main() {
    let shared = unsafe { c_make(8) };
    assert!(!shared.is_null());
    // Raw pointers are not `Send`, so hand the address to the threads.
    let addr = shared as usize;

    let a = thread::spawn(move || {
        let p = addr as *mut c_void;
        for i in 0..16 {
            unsafe { c_locked_touch(p, i) };
        }
    });

    let b = thread::spawn(move || {
        let p = addr as *mut c_void;
        for i in 0..16 {
            unsafe { c_locked_touch(p, i) };
        }
    });

    a.join().unwrap();
    b.join().unwrap();

    unsafe { c_free(shared) };
    println!("norace_locked_api_done");
}
