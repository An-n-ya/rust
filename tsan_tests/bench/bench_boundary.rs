// Benchmark for the FFI-boundary data race sanitizer.
//
// `run_bench.sh` builds this file three ways:
//   * plain      : native build, no TSan runtime, feature off
//   * tsan       : TSan runtime linked, feature off
//   * tsan-instr : TSan runtime linked, feature on (ffi_race_* inserted)
//
// Two workloads (selected by argv[1]):
//   * boundary : a tight loop of FFI calls with little else. This is the
//                worst case for the boundary instrumentation, because every
//                iteration crosses the FFI line.
//   * compute  : each FFI call is preceded by pure-Rust arithmetic that makes
//                no FFI call and no raw-pointer dereference. Such code is
//                never instrumented, so it dilutes the boundary cost -- the
//                whole point of instrumenting only the FFI boundary.
//
// Timing is measured inside the program with `std::time::Instant` and printed
// as milliseconds plus nanoseconds per iteration, so the runner only has to
// parse one line.
#![cfg_attr(bench_instrument, feature(ffi_race_sanitizer))]
#![allow(dead_code)]

use std::ffi::c_void;
use std::hint::black_box;
use std::os::raw::c_int;
use std::time::Instant;

extern "C" {
    fn c_make(n: usize) -> *mut c_void;
    fn c_free(p: *mut c_void);
    fn c_touch_write(p: *mut c_void, v: c_int);
    fn c_touch_read(p: *const c_void) -> c_int;
}

/// Pure-Rust work with no FFI call and no raw-pointer dereference, so the
/// FFI-boundary pass leaves it completely untouched.
#[inline(never)]
fn compute(seed: u64) -> u64 {
    let mut x = seed;
    for _ in 0..64 {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51afd7ed558ccd);
    }
    x
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "boundary".to_owned());
    let iters: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20_000_000);

    let p = unsafe { c_make(8) };
    assert!(!p.is_null(), "c_make failed");

    // Untimed warm-up so that page faults and first-call costs do not skew the
    // measured interval.
    let warm = iters / 10 + 1;
    for i in 0..warm {
        unsafe { c_touch_write(p, i as c_int) };
    }

    let mut acc: u64 = 0;
    let start = Instant::now();
    if mode == "compute" {
        for i in 0..iters {
            acc = acc.wrapping_add(compute(i as u64));
            unsafe { c_touch_write(p, acc as c_int) };
        }
    } else {
        for i in 0..iters {
            unsafe { c_touch_write(p, i as c_int) };
        }
    }
    let elapsed = start.elapsed();

    // Consume the results so nothing can be optimised away.
    black_box(acc);
    black_box(unsafe { c_touch_read(p) });
    unsafe { c_free(p) };

    let secs = elapsed.as_secs_f64();
    println!(
        "mode={mode} iters={iters} elapsed_ms={:.3} ns_per_iter={:.3}",
        secs * 1e3,
        secs * 1e9 / iters as f64
    );
}
