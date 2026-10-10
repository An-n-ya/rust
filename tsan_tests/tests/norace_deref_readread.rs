// Negative test for R2 (raw-pointer dereference read/write).
//
// `deref_read` is an FFI-exported function whose body performs only a raw
// pointer read (`*p`). The MIR pass instruments it with `ffi_race_read`. Two
// spawned threads read the same address concurrently. Read/read accesses do
// not conflict, so ThreadSanitizer must NOT report a data race.
#![feature(ffi_race_sanitizer)]
#![allow(dead_code)]

use std::os::raw::c_int;
use std::thread;

#[no_mangle]
pub extern "C" fn deref_read(p: *const c_int) -> c_int {
    unsafe { *p }
}

fn main() {
    let ptr = Box::into_raw(Box::new(42 as c_int));
    let addr = ptr as usize;

    let a = thread::spawn(move || {
        let p = addr as *const c_int;
        let _ = deref_read(p);
    });

    let b = thread::spawn(move || {
        let p = addr as *const c_int;
        let _ = deref_read(p);
    });

    a.join().unwrap();
    b.join().unwrap();

    unsafe { drop(Box::from_raw(ptr)) };
    println!("norace_deref_readread_done");
}
