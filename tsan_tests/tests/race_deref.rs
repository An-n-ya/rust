// Positive test for R2 (raw-pointer dereference read/write).
//
// `deref_write` is an FFI-exported function (`#[no_mangle] pub extern "C"`),
// so the MIR pass treats its body as an FFI boundary and instruments the raw
// pointer dereference `*p = v` with `ffi_race_write`. Two spawned threads call
// `deref_write` on the same address with no synchronization, so the linked
// ThreadSanitizer runtime must report a data race.
//
// No hand-written __tsan_* annotation is used: the FFI call boundary itself
// carries no raw-pointer argument, so detection must come from the R2
// dereference instrumentation inside the callee.
#![feature(ffi_race_sanitizer)]
#![allow(dead_code)]

use std::os::raw::c_int;
use std::thread;

#[no_mangle]
pub extern "C" fn deref_write(p: *mut c_int) {
    unsafe {
        *p = 1;
    }
}

fn main() {
    let ptr = Box::into_raw(Box::new(0 as c_int));
    // Raw pointers are not `Send`, so hand the address to the threads.
    let addr = ptr as usize;

    let a = thread::spawn(move || {
        let p = addr as *mut c_int;
        deref_write(p);
    });

    let b = thread::spawn(move || {
        let p = addr as *mut c_int;
        deref_write(p);
    });

    a.join().unwrap();
    b.join().unwrap();

    unsafe { drop(Box::from_raw(ptr)) };
    println!("race_deref_done");
}
