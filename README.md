# FFIProbe (reproduction branch)

A rustc-based tool for detecting memory-safety bugs and, on this branch,
**data races** at the Rust/C FFI boundary.

This branch (`ffiprobe-repro`) contains the *reproduction* work built on top of
the FFIProbe paper. It bundles two related efforts:

1. **FFIProbe memory-bug reproduction** — port the paper's `ffi_sanitizer` MIR
   pass + runtime shim + intercepting C allocator to a current rustc, and
   reproduce all of the paper's reported bug classes.
2. **FFI-boundary data-race detection (new)** — an experimental
   `ffi_race_sanitizer` MIR pass that inserts ThreadSanitizer (TSan)
   annotations **only where the Rust/C boundary is crossed**, instead of
   instrumenting every memory access as a full TSan build does.

Both share the FFIProbe idea: instrument at the FFI boundary in MIR, then let a
runtime library do the checking.

> The compiler source changes for (2) live on the **`ffiprobe-tsan`** branch.
> This branch carries only the harness, reports and documentation for the
> data-race work — see [FFI-boundary data-race detection](#ffi-boundary-data-race-detection-tsan).

---

## 1. FFIProbe memory-bug reproduction

The paper — *FFIProbe: Detecting Unsafe FFI Pointer Interactions in Rust* —
detects memory-safety defects in Rust↔C FFI programs using three cooperating
mechanisms:

- **MIR instrumentation pass (`FFISanitizer`)** — inserts pre/post-condition
  checks at every FFI call, ownership conversion (`Box::from_raw`/`Box::leak`)
  and raw-pointer dereference.
- **Runtime shim (`core::ffi_sanitizer`)** — the checks called at those MIR
  sites, reading allocation headers and deciding violations.
- **Intercepting C allocator (`libffisan`)** — wraps `malloc`/`free`, placing a
  16-byte header before and a red-zone after each allocation to track who
  allocated/freed and detect OOB.

Bug classes reproduced: memory **leak**, **double free**, **cross-language
alloc/free mismatch** (both directions), **use-after-free** and **out-of-bounds**.

Full details, the API-drift port, and the test results are in
[REPRODUCTION_REPORT.md](REPRODUCTION_REPORT.md). Summary: **8/8** cases behave
exactly as the paper predicts, including 2 no-false-positive baselines.

### Files

| Path | Purpose |
| --- | --- |
| `changes.diff` | Diff of the changes made to the Rust source tree |
| `ffi_sanitizer_unwind_fix.patch` | MIR `UnwindAction` fix for the `FFISanitizer` pass |
| `scripts/` | Dataset download / build / evaluation scripts (`build_dataset.py`, `dl_crates.py`, `dl_ffi_bindings.py`, `run.py`) |
| `tests/` | FFIProbe memory-bug test cases + `run.sh` harness |
| `REPRODUCTION_REPORT.md` | Reproduction report and results |

### Usage

1. Build the patched compiler from the Rust source (see the
   [Rust Compiler Development Guide](https://rustc-dev-guide.rust-lang.org/building/how-to-build-and-run.html))
   after applying `changes.diff`.
2. Register it: `rustup toolchain link ffiprobe /path/to/build`.
3. Enable the feature in the crate under test: `#![feature(ffi_sanitizer)]`.
4. Run with `cargo +ffiprobe run` / `cargo +ffiprobe test`.

Detected issues are reported to the console during execution.

---

## 2. FFI-boundary data-race detection (TSan)

A full TSan build instruments every load/store and is therefore very slow. This
work applies the FFIProbe idea to data races: a new MIR pass
(`FFIRaceSanitizer`, feature `ffi_race_sanitizer`) inserts TSan annotations
**only at the FFI boundary**, aiming for a more precise, cheaper race checker.

The pass implements four rules:

| Rule | Inserts | Where |
| --- | --- | --- |
| **R1** | `ffi_race_read` / `ffi_race_write` | before an FFI call, per raw-pointer argument (`*const` → read, `*mut` → write) |
| **R2** | `ffi_race_read` / `ffi_race_write` | before a raw-pointer dereference in an FFI-related function (keyed on `PlaceContext`) |
| **R3** | `ffi_race_release` / `ffi_race_acquire` | before `thread::spawn` (release) and at the start of a closure capturing a raw pointer (acquire) |
| **R4** | `ffi_race_mutex_pre_lock` / `post_lock` / `pre_unlock` / `post_unlock` | around calls to a C API listed in `KNOWN_LOCKED_FFI_APIS` |

The wrappers are located by the pass via `#[rustc_diagnostic_item]` names and
expand to the `__tsan_*` runtime ABI. See the **`ffiprobe-tsan`** branch for the
pass (`compiler/rustc_mir_transform/src/ffi_race_sanitizer.rs`), the core shim
(`library/core/src/ffi_race_sanitizer.rs`) and the feature/hook registration.

### Results

The harness pairs each positive (race expected) case with a negative (clean)
control, and decides PASS/FAIL by grepping the TSan runtime's stderr:

```
total=9  pass=9  fail=0
```

Race cases R1/R2/R3 are reported purely by the compiler-inserted wrappers (no
hand-written `__tsan_*` in the test), and R4's ordering is confirmed by a
control experiment. Full matrix, feature-gate verification and MIR evidence:
[tsan_tests/RESULTS.md](tsan_tests/RESULTS.md).

### Benchmark

`tsan_tests/bench/bench_boundary.rs` is one source compiled in three
configurations (plain / TSan-linked / TSan-linked-and-instrumented), sampled
interleaved, min-of-7:

| Workload | plain | tsan | tsan-instr | instrumentation overhead |
| --- | --- | --- | --- | --- |
| boundary (100% FFI calls) | 0.945 | 0.911 | 2.279 ns/iter | **+150.2 %** |
| compute (FFI diluted by pure Rust) | 66.711 | 67.445 | 69.505 ns/iter | **+3.1 %** |

Key point: linking TSan with no instrumentation is essentially free; all the
cost is the inserted boundary code, so the overhead stays **local to the FFI
line**. Full methodology, raw runs and threats to validity:
[tsan_tests/BENCHMARK_REPORT.md](tsan_tests/BENCHMARK_REPORT.md).

### Files

| Path | Purpose |
| --- | --- |
| `tsan_tests/run_tsan.sh` | 9-case race harness (positive/negative pairs) |
| `tsan_tests/run_bench.sh` | Three-configuration benchmark driver |
| `tsan_tests/tests/` | Rust cases + `tsan_helpers.c` + `ffisan_shim.c` |
| `tsan_tests/bench/bench_boundary.rs` | Benchmark workload |
| `tsan_tests/RESULTS.md` | Test matrix, feature-gate check, MIR evidence |
| `tsan_tests/BENCHMARK_REPORT.md` | Benchmark report |

### How to run

```sh
cd tsan_tests
bash run_tsan.sh    # race harness
bash run_bench.sh   # benchmark (ITERS_BOUNDARY=100000000 ITERS_COMPUTE=10000000 REPEATS=7)
```

Both require a stage1 rustc built from the `ffiprobe-tsan` branch and the clang
TSan static runtime (`libclang_rt.tsan-x86_64.a`). Host: `x86_64-unknown-linux-gnu`.

---

## Trophy Case (FFIProbe, upstream)

| Package | Version | Bug Type | Issue |
| --- | --- | --- | --- |
| gotpl | 0.2.6 | UB | https://github.com/moyanj/gotpl/issues/1 |
| tcpp | 0.1.0 | LEAK | https://github.com/Phosphorus15/tcpp-rs/issues/1 |
| physfs-rs | 0.1.6 | LEAK | Not found repository |
| minimp4 | 0.1.2 | LEAK | https://github.com/darkskygit/minimp4.rs/issues/8 |
| wabt | 0.10.0 | LEAK | https://github.com/pepyakin/wabt-rs/issues/89 |
| husk | 0.1.3 | UB | https://github.com/hwittenborn/husk/issues/4 |
| regex-rs | 0.1.0 | LEAK | https://github.com/newsboat/newsboat/issues/3266 |
| wfc-rs | 0.6.1 | UB | https://github.com/nsmryan/wfc-rs/issues/2 |
| libmodbus-rs | 0.8.3 | OOB | https://github.com/zzeroo/libmodbus-rs/issues/23 |
| voxelizer | 1.0.0 | LEAK | Reported by others |
| rooster | 2.14.0 | LEAK | https://github.com/conradkleinespel/rooster/issues/90 |
| tidesdb-rs | 0.1.2 | UB | https://github.com/0x6flab/tidesdb-rs/issues/8 |
| spglib | 1.15.1 | UB | https://github.com/spglib/spglib-rs/issues/4 |
| libucl | 0.2.3 | LEAK | https://github.com/draft6/libucl-rs/issues/7 |
| vampire-prover | 0.2.0 | LEAK | https://github.com/Dragon-Hatcher/vampire-rs/issues/1 |
