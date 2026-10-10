// Negative test: a single thread performs repeated annotated writes on the
// same pointer with no concurrency. TSan must NOT report a data race.
// Validates the negative-detection path of the harness.
use std::ffi::c_void;

extern "C" {
    fn __tsan_write8(addr: *mut c_void);
}

fn main() {
    let shared: *mut u64 = Box::leak(Box::new(0u64));
    let p = shared as *mut c_void;

    for _ in 0..8 {
        unsafe { __tsan_write8(p) };
    }

    println!("no_race_seq_done");
}
