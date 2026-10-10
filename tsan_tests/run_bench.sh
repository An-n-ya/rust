#!/usr/bin/env bash
# ============================================================================
# run_bench.sh - wall-clock benchmark for the FFI-boundary instrumentation.
#
# Builds bench/bench_boundary.rs in three configurations and compares the
# measured time per FFI call:
#
#   plain      native build, no TSan runtime, feature off
#   tsan       TSan runtime linked, feature off
#   tsan-instr TSan runtime linked, feature on  -> ffi_race_* inserted at the
#              FFI boundary
#
# Two workloads are timed (see bench_boundary.rs):
#   boundary : tight loop of FFI calls           (worst case for the pass)
#   compute  : pure-Rust work between FFI calls  (dilutes the boundary cost)
#
# The three configurations are sampled INTERLEAVED within each repeat (so all
# of them see a similar CPU/cache state) and the minimum ns/iter over REPEATS
# is reported, to reduce scheduler and frequency-scaling noise. TSan leak
# detection is disabled for the runs so the one-shot leak scan does not pollute
# the numbers.
#
# Tunables (env): ITERS_BOUNDARY, ITERS_COMPUTE, REPEATS.
# ============================================================================
set -u

RUST_ROOT="/home/annya/src/rust"
HOST="x86_64-unknown-linux-gnu"
RUSTC="$RUST_ROOT/build/$HOST/stage1/bin/rustc"
SYSROOT="$RUST_ROOT/build/$HOST/stage1"
TSAN_LIB="/usr/lib/clang/22/lib/linux/libclang_rt.tsan-x86_64.a"

HERE="$(cd "$(dirname "$0")" && pwd)"
TESTS_DIR="$HERE/tests"
BENCH_SRC="$HERE/bench/bench_boundary.rs"
BUILD_ROOT="$HERE/bench/_build"
CC_BIN="${CC:-cc}"

ITERS_BOUNDARY="${ITERS_BOUNDARY:-100000000}"
ITERS_COMPUTE="${ITERS_COMPUTE:-10000000}"
REPEATS="${REPEATS:-7}"

export TSAN_OPTIONS="detect_leaks=0${TSAN_OPTIONS:+ $TSAN_OPTIONS}"

mkdir -p "$BUILD_ROOT"

# --- 1. C helpers, built un-instrumented with cc ----------------------------
"$CC_BIN" -c -std=c11 -fPIC -O2 -g "$TESTS_DIR/tsan_helpers.c" \
    -o "$BUILD_ROOT/tsan_helpers.o" || { echo "helpers compile FAILED"; exit 1; }
"$CC_BIN" -c -std=c11 -fPIC -O2 -g "$TESTS_DIR/ffisan_shim.c" \
    -o "$BUILD_ROOT/ffisan_shim.o" || { echo "shim compile FAILED"; exit 1; }

# --- 2. Build the three configurations --------------------------------------
COMMON=(
    --sysroot "$SYSROOT" -Zunstable-options
    --edition 2021 -C opt-level=2 -C panic=unwind
)
TSAN_ARGS=(
    -Zpre-link-arg="$BUILD_ROOT/ffisan_shim.o"
    -C link-arg="$BUILD_ROOT/tsan_helpers.o"
    -C link-arg=-Wl,--whole-archive
    -C link-arg="$TSAN_LIB"
    -C link-arg=-Wl,--no-whole-archive
    -C link-arg=-lpthread -C link-arg=-ldl -C link-arg=-lrt
    -C link-arg=-lm -C link-arg=-lstdc++
)

echo "=== BUILD ==="
if ! "$RUSTC" "${COMMON[@]}" \
        -C link-arg="$BUILD_ROOT/tsan_helpers.o" \
        -o "$BUILD_ROOT/bench_plain" "$BENCH_SRC" 2>"$BUILD_ROOT/build_plain.err"; then
    echo "plain build FAILED:"; cat "$BUILD_ROOT/build_plain.err"; exit 1
fi
echo "built plain"

if ! "$RUSTC" "${COMMON[@]}" "${TSAN_ARGS[@]}" \
        -o "$BUILD_ROOT/bench_tsan" "$BENCH_SRC" 2>"$BUILD_ROOT/build_tsan.err"; then
    echo "tsan build FAILED:"; cat "$BUILD_ROOT/build_tsan.err"; exit 1
fi
echo "built tsan"

if ! "$RUSTC" "${COMMON[@]}" --cfg bench_instrument "${TSAN_ARGS[@]}" \
        -o "$BUILD_ROOT/bench_tsan_instr" "$BENCH_SRC" 2>"$BUILD_ROOT/build_tsan_instr.err"; then
    echo "tsan-instr build FAILED:"; cat "$BUILD_ROOT/build_tsan_instr.err"; exit 1
fi
echo "built tsan-instr"

# Sanity: the instrumented build must actually reference the TSan runtime.
instr_syms="$(nm "$BUILD_ROOT/bench_tsan_instr" 2>/dev/null | grep -c "__tsan_")"
tsan_syms="$(nm "$BUILD_ROOT/bench_tsan" 2>/dev/null | grep -c "__tsan_")"
echo "tsan runtime symbols: tsan=$tsan_syms tsan-instr=$instr_syms"

CONFIGS=(plain tsan tsan-instr)
declare -A BINS=(
    [plain]="$BUILD_ROOT/bench_plain"
    [tsan]="$BUILD_ROOT/bench_tsan"
    [tsan-instr]="$BUILD_ROOT/bench_tsan_instr"
)

# --- 3. Interleaved measurement ---------------------------------------------
# Prints the minimum ns/iter per configuration for one workload.
measure_mode() {
    local mode="$1" iters="$2"
    declare -A samples=( [plain]="" [tsan]="" [tsan-instr]="" )
    local r c line v
    for ((r = 0; r < REPEATS; r++)); do
        for c in "${CONFIGS[@]}"; do
            line="$("${BINS[$c]}" "$mode" "$iters")"
            v="$(printf '%s' "$line" | sed -n 's/.*ns_per_iter=\([0-9.]*\).*/\1/p')"
            samples[$c]+="${v}"$'\n'
        done
    done
    for c in "${CONFIGS[@]}"; do
        printf '%s\t%s\n' "$c" "$(printf '%s' "${samples[$c]}" | sort -n | head -n1)"
    done
}

# --- 4. Report --------------------------------------------------------------
report_mode() {
    local mode="$1" iters="$2"
    echo
    echo "=================================================================="
    echo "=== WORKLOAD: $mode   (iters=$iters, repeats=$REPEATS) ==="
    echo "=================================================================="

    declare -A res=()
    local c v
    while IFS=$'\t' read -r c v; do res[$c]="$v"; done < <(measure_mode "$mode" "$iters")

    printf "  %-12s %12s ns/iter\n" "plain"      "${res[plain]}"
    printf "  %-12s %12s ns/iter\n" "tsan"       "${res[tsan]}"
    printf "  %-12s %12s ns/iter\n" "tsan-instr" "${res[tsan-instr]}"
    echo "  ------------------------------------------------------------"
    awk -v p="${res[plain]}" -v t="${res[tsan]}" -v i="${res[tsan-instr]}" 'BEGIN {
        if (t + 0 > 0) printf "  instrumentation overhead (tsan-instr vs tsan): %+8.2f%%\n", (i - t) / t * 100;
        if (p + 0 > 0) printf "  TSan runtime cost      (tsan vs plain):       %+8.2f%%\n", (t - p) / p * 100;
        if (p + 0 > 0) printf "  total vs native        (tsan-instr vs plain): %+8.2f%%\n", (i - p) / p * 100;
    }'
}

report_mode boundary "$ITERS_BOUNDARY"
report_mode compute  "$ITERS_COMPUTE"

echo
echo "=================================================================="
echo "=== DONE ==="
