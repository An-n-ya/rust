# FFI-Boundary ThreadSanitizer — Test Results

Results for the experimental `ffi_race_sanitizer` feature: a MIR pass that
inserts ThreadSanitizer (TSan) runtime annotations **only at the Rust/C (FFI)
boundary**, instead of instrumenting every memory access as a full TSan build
does.

## Environment

| Item | Value |
| --- | --- |
| Stage1 compiler | `/home/annya/src/rust/build/x86_64-unknown-linux-gnu/stage1/bin/rustc` |
| Host triple | `x86_64-unknown-linux-gnu` |
| TSan runtime | `/usr/lib/clang/22/lib/linux/libclang_rt.tsan-x86_64.a` (clang 22) |
| C helper compiler | `cc` (gcc), un-instrumented (`-O0 -g`) |

## How to run

```sh
cd /home/annya/playground/FFIProbe/tsan_tests
bash run_tsan.sh
```

The harness compiles each `tests/<name>.rs` with the stage1 rustc, links the
whole static TSan runtime, runs it, and decides PASS/FAIL by grepping the
runtime's stderr for the string `ThreadSanitizer`. Exit codes are informational
only (TSan reports a race on stderr and still exits non-zero). `TSAN_OPTIONS`
is set to `halt_on_error=0` so multiple reports and a normal exit path are
possible.

Equivalent per-test invocation (shown for `race_boundary`):

```sh
# 1. build the un-instrumented C helpers with cc
cc -c -std=c11 -fPIC -O0 -g tests/tsan_helpers.c -o _build/race_boundary/tsan_helpers.o
cc -c -std=c11 -fPIC -O0 -g tests/ffisan_shim.c  -o _build/race_boundary/ffisan_shim.o

# 2. compile + link with the stage1 rustc
# (the shim is pre-linked so libffisan's libc interceptors never enter the image;
#  the TSan archive is pulled in whole so its preinit_array initializer runs)
build/x86_64-unknown-linux-gnu/stage1/bin/rustc \
    --sysroot build/x86_64-unknown-linux-gnu/stage1 -Zunstable-options \
    -Zpre-link-arg=_build/race_boundary/ffisan_shim.o \
    --edition 2021 -C opt-level=0 -C panic=unwind \
    -C link-arg=_build/race_boundary/tsan_helpers.o \
    -C link-arg=-Wl,--whole-archive \
    -C link-arg=/usr/lib/clang/22/lib/linux/libclang_rt.tsan-x86_64.a \
    -C link-arg=-Wl,--no-whole-archive \
    -C link-arg=-lpthread -C link-arg=-ldl -C link-arg=-lrt \
    -C link-arg=-lm -C link-arg=-lstdc++ \
    -o _build/race_boundary/race_boundary tests/race_boundary.rs

# 3. run and inspect stderr
TSAN_OPTIONS=halt_on_error=0 _build/race_boundary/race_boundary
```

## Instrumentation rules exercised

| Rule | What it inserts | Where |
| --- | --- | --- |
| **R1** | `ffi_race_read` / `ffi_race_write` | before an FFI call, for each raw-pointer argument (`*const` → read, `*mut` → write) |
| **R2** | `ffi_race_read` / `ffi_race_write` | before a raw-pointer dereference inside an FFI-related function, keyed on the `PlaceContext` |
| **R3** | `ffi_race_release` / `ffi_race_acquire` | before `thread::spawn` (release) and at the start of a closure that captures a raw pointer (acquire) |
| **R4** | `ffi_race_mutex_pre_lock`/`post_lock`/`pre_unlock`/`post_unlock` | around calls to a C API listed in `KNOWN_LOCKED_FFI_APIS` |

## Test matrix

| # | Test | Rule / purpose | Expect | Result | Key stderr |
| --- | --- | --- | --- | --- | --- |
| 1 | `sanity_link` | TSan runtime link sanity, hand-written `__tsan_*` only | race | **PASS** | `data race ... Read of size 8 ... sanity_link::main::{{closure}}` |
| 2 | `no_race_seq` | Negative control: single-thread, hand-written writes | none | **PASS** | *(empty)* |
| 3 | `race_boundary` | **R1** two threads call `c_touch_write` on the same `*mut c_void` | race | **PASS** | `data race ... Write of size 1 ... in core::ffi_race_sanitizer::tsan_write` |
| 4 | `norace_sequenced` | **R1** negative: worker then main, ordered by `join` | none | **PASS** | *(empty)* |
| 5 | `race_deref` | **R2** two threads call `deref_write(*p = 1)` with no sync | race | **PASS** | `data race ... Write of size 4 ... in core::ffi_race_sanitizer::tsan_write` |
| 6 | `norace_deref_readread` | **R2** negative: concurrent read/read on the same address | none | **PASS** | *(empty)* |
| 7 | `race_spawn_concurrent` | **R3** two spawned workers call `c_touch_write`, no ordering | race | **PASS** | `data race ... Write of size 1 ... in core::ffi_race_sanitizer::tsan_write` |
| 8 | `norace_spawn_handoff` | **R3** negative: spawn/join ordering + R3 release annotation | none | **PASS** | *(empty)* |
| 9 | `norace_locked_api` | **R4** two threads call the lock-protected `c_locked_touch` | none | **PASS** | *(empty)* |

Harness summary:

```
total=9  pass=9  fail=0
```

Tests 3, 5 and 7 carry **no hand-written `__tsan_*` annotation** — the runtime
report is produced purely by the compiler-inserted `ffi_race_*` wrappers.

## Task 7.1 — feature gate verification

A program that *would* be instrumented (an FFI call taking a `*mut u64` plus a
raw-pointer dereference) was compiled with and without
`#![feature(ffi_race_sanitizer)]`.

| Build | Result | `nm` on the binary |
| --- | --- | --- |
| **Feature off** | links cleanly | `__tsan_` symbol count = **0** |
| **Feature on** | links cleanly against TSan | `__tsan_` symbol count = **92**, `ffi_race_` wrapper refs = **2** |

With the feature off the pass returns immediately
(`if !tcx.features().ffi_race_sanitizer() { return; }`), so nothing ever calls
the wrappers and no `__tsan_*` symbol is referenced. No leftover debug
`println!`/`dbg!` exists in `ffi_race_sanitizer.rs`.

## R3 acquire path — MIR evidence

Raw pointers are `!Send`, so a closure passed to `thread::spawn` in a real
program cannot capture one directly (the tests smuggle the address through
`usize`). The acquire branch is therefore verified on a synthetic case where a
`move` closure captures a `*mut i32` directly:

```sh
rustc --sysroot <stage1> -Zunstable-options --crate-type=bin --edition 2021 \
      --emit=mir -o closure_acquire.mir closure_acquire.rs
grep ffi_race_ closure_acquire.mir
```

```
_33 = core::ffi_race_sanitizer::ffi_race_acquire::<i32>(move _32) -> [return: bb7, unwind continue];
```

The acquire wrapper is inserted at the start of the closure body, proving the
`is_closure_body` + `has_raw_ptr_upvar` path fires.

## R4 mutex-token design

TSan keys lock/unlock ordering on the **address** passed to `__tsan_mutex_*`.
An early version reused the offending data pointer as the mutex identity, which
made TSan's mutex shadow alias the shadow of the data itself, producing a
spurious race against the very access the annotation was meant to protect
(`norace_locked_api` reported a race between `__tsan_mutex_post_lock` and the
subsequent `tsan_write`).

Fix: the wrappers ignore the data pointer and use a dedicated process-wide
static, `MUTEX_TOKEN`, via `addr_of!`. That address is never accessed as
ordinary data, so the mutex shadow no longer collides with any data shadow.

Control experiment: removing `c_locked_touch` from `KNOWN_LOCKED_FFI_APIS` and
rebuilding makes `norace_locked_api` report a data race as expected, confirming
the pass (not the harness) is responsible for the ordering.

## Benchmark — instrumented vs un-instrumented

`bench/bench_boundary.rs` is a single source file compiled in three
configurations by `run_bench.sh`:

| Config | TSan runtime linked | `ffi_race_*` inserted | How the source is switched |
| --- | --- | --- | --- |
| `plain` | no | no | plain `rustc`, feature off |
| `tsan` | yes | no | `TSAN_ARGS`, feature off |
| `tsan-instr` | yes | yes | `TSAN_ARGS` + `--cfg bench_instrument` |

The source opts into the feature only when that cfg is set:

```rust
#![cfg_attr(bench_instrument, feature(ffi_race_sanitizer))]
```

so the *same* file gives a true before/after pair. Two workloads are timed
inside the program with `std::time::Instant`:

- **`boundary`** — a tight loop of `c_touch_write(p, i)` FFI calls (worst case:
  every iteration crosses the boundary and gets an `ffi_race_write`).
- **`compute`** — the same FFI call preceded by pure-Rust arithmetic that makes
  no FFI call and no raw-pointer dereference, so it is never instrumented; this
  dilutes the boundary cost.

Methodology: the three configurations are sampled **interleaved** within each
repeat (all three see the same CPU/cache state) and the **minimum** `ns/iter`
over `REPEATS` is taken, to suppress frequency-scaling and scheduler noise.
`TSAN_OPTIONS=detect_leaks=0` disables the one-shot leak scan at exit.

```sh
cd /home/annya/playground/FFIProbe/tsan_tests
bash run_bench.sh          # ITERS_BOUNDARY=100000000 ITERS_COMPUTE=10000000 REPEATS=7
```

### Results

Workload `boundary` (100,000,000 iters):

| Config | ns/iter |
| --- | --- |
| `plain` | 0.945 |
| `tsan` | 0.911 |
| `tsan-instr` | 2.279 |
| | |
| instrumentation overhead (`tsan-instr` vs `tsan`) | **+150.16 %** |
| TSan runtime cost, no instrumentation (`tsan` vs `plain`) | −3.60 % (noise) |
| total vs native (`tsan-instr` vs `plain`) | **+141.16 %** |

Workload `compute` (10,000,000 iters):

| Config | ns/iter |
| --- | --- |
| `plain` | 66.711 |
| `tsan` | 67.445 |
| `tsan-instr` | 69.505 |
| | |
| instrumentation overhead (`tsan-instr` vs `tsan`) | **+3.05 %** |
| TSan runtime cost, no instrumentation (`tsan` vs `plain`) | +1.10 % |
| total vs native (`tsan-instr` vs `plain`) | **+4.19 %** |

### Reading the numbers

- **Linking the TSan runtime alone is essentially free.** `tsan` vs `plain` is
  −3.6 % on `boundary` and +1.1 % on `compute` — both within run-to-run noise.
  The shadow-memory mappings are set up lazily; with no annotation ever called
  they cost nothing per iteration. All the cost is the *inserted* code.
- **The cost is concentrated at the boundary.** On `boundary`, every iteration
  makes exactly one FFI call and therefore pays one `ffi_race_write` (plus the
  TSan shadow lookup behind it), roughly tripling the time: +150 %.
- **Instrumenting only the boundary keeps the overhead local.** On `compute`,
  the ~66 ns of pure-Rust arithmetic per iteration is untouched by the pass, so
  the same absolute boundary cost becomes a **+3 %** end-to-end overhead. This
  is the intended behavior: the pass pays only where the FFI line is crossed,
  not for every memory access as a full TSan build would.
- The `boundary` figure is a deliberate worst case (100 % of the loop is FFI
  calls). Real code sits between the two rows, in proportion to how much of the
  hot path actually crosses the boundary.

## Known constraints

- **Raw pointers are `!Send`.** In real `thread::spawn` tests the address must
  travel as a `usize`, so the closure has no raw-pointer upvar and the R3
  `acquire` half does not fire there; it is verified on a synthetic closure
  instead (see above). The R3 `release` half still fires in `main`, which holds
  the raw-pointer local.
- **Single global `MUTEX_TOKEN`.** All lock-protected APIs share one token, so
  distinct C locks are over-synchronized with one another. Acceptable as an
  approximation for this prototype; a real implementation would assign one
  token per `(api, pointer)` pair.
- **Thin, sized pointees only.** R2 skips slices, trait objects and other
  unsized pointees in this MVP.
- **`ffisan_shim.c`** neutralizes the experimental `libffisan` archive that the
  stage1 `core`/`std` auto-link, so its libc interceptors cannot collide with
  the TSan bootstrap. Heap allocation is therefore served by glibc directly;
  detection relies on the explicit `__tsan_read*`/`__tsan_write*` annotations,
  which operate on TSan shadow memory regardless of how memory was allocated.
