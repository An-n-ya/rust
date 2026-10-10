// Positive sanity test: proves the system clang ThreadSanitizer runtime is
// linked and reports races, using ONLY hand-written __tsan_* annotations and
// NO compiler instrumentation (no #![feature(...)], no -Zsanitizer).
//
// Two threads race on a process-global heap u64 with no synchronization.
// Expected: TSan prints a "WARNING: ThreadSanitizer: data race" report.
use std::ffi::c_void;
use std::thread;

extern "C" {
    fn __tsan_read8(addr: *mut c_void);
    fn __tsan_write8(addr: *mut c_void);
    fn __tsan_read_range(addr: *mut c_void, size: u64);
    fn __tsan_write_range(addr: *mut c_void, size: u64);
}

fn main() {
    // Process-global heap allocation shared by both threads.
    let shared: *mut u64 = Box::leak(Box::new(0u64));
    let addr = shared as usize;

    let a = thread::spawn(move || {
        let p = addr as *mut u64 as *mut c_void;
        for _ in 0..8 {
            unsafe { __tsan_write8(p) };
            unsafe { __tsan_write_range(p, 8) };
        }
    });

    let b = thread::spawn(move || {
        let p = addr as *mut u64 as *mut c_void;
        for _ in 0..8 {
            unsafe { __tsan_read8(p) };
            unsafe { __tsan_read_range(p, 8) };
        }
    });

    a.join().unwrap();
    b.join().unwrap();

    println!("sanity_done");
}
