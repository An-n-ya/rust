#!/usr/bin/env bash
# ============================================================================
# run_tsan.sh - minimal harness for the experimental ThreadSanitizer-based
# FFI race detector.
#
# Each test program is compiled with the locally-built stage1 rustc and linked
# against the *system clang 22* static ThreadSanitizer runtime. The programs are
# NOT compiler-instrumented (no -Zsanitizer, no #![feature(...)]); they issue
# hand-written __tsan_read*/__tsan_write* annotations instead. That is enough
# for the linked TSan runtime to detect and report races on FFI/shared memory.
#
# PASS/FAIL is decided by grepping the runtime's stderr for the string
# `ThreadSanitizer`, NOT by the process exit code. TSan reports a race on
# stderr and still exits non-zero (typically 66), so the exit code alone does
# not distinguish "race found" from "crashed for another reason".
# ============================================================================
set -u

RUST_ROOT="/home/annya/src/rust"
HOST="x86_64-unknown-linux-gnu"
RUSTC="$RUST_ROOT/build/$HOST/stage1/bin/rustc"
SYSROOT="$RUST_ROOT/build/$HOST/stage1"

# Static TSan runtime shipped with the system clang 22 toolchain.
TSAN_LIB="/usr/lib/clang/22/lib/linux/libclang_rt.tsan-x86_64.a"

HERE="$(cd "$(dirname "$0")" && pwd)"
TESTS_DIR="$HERE/tests"
BUILD_ROOT="$HERE/_build"
CC_BIN="${CC:-cc}"

# Test list. Format: "<name>:<expect>", where <expect> is one of:
#   race -> stderr MUST contain a ThreadSanitizer report
#   none -> stderr MUST NOT contain any ThreadSanitizer report
# The .rs source is tests/<name>.rs.
TESTS=(
    "sanity_link:race"
    "no_race_seq:none"
    "race_boundary:race"
    "norace_sequenced:none"
    "race_deref:race"
    "norace_deref_readread:none"
    "race_spawn_concurrent:race"
    "norace_spawn_handoff:none"
    "norace_locked_api:none"
)

# TSan runtime options:
#   halt_on_error=0 keeps the runtime alive after the first race, so a program
#   with several races still produces deterministic output and a normal exit
#   path instead of aborting immediately. It does not suppress the report.
# (The runner still keys off stderr, so this only affects run behaviour.)
export TSAN_OPTIONS="halt_on_error=0${TSAN_OPTIONS:+ $TSAN_OPTIONS}"

PASS=0
FAIL=0
RESULTS=()

for ENTRY in "${TESTS[@]}"; do
    NAME="${ENTRY%%:*}"
    EXPECT="${ENTRY##*:}"
    SRC="$TESTS_DIR/$NAME.rs"
    WORKDIR="$BUILD_ROOT/$NAME"
    BIN="$WORKDIR/$NAME"
    STDOUT_LOG="$WORKDIR/stdout.log"
    STDERR_LOG="$WORKDIR/stderr.log"

    mkdir -p "$WORKDIR"
    echo "=================================================================="
    echo "=== TEST: $NAME (expect: $EXPECT)"
    echo "=================================================================="

    # --- 1. C objects, built un-instrumented with cc (gcc) ------------------
    # tsan_helpers.c: the FFI helper functions under test.
    # ffisan_shim.c : neutralizes the experimental libffisan archive that the
    #   stage1 core/std auto-link, so its libc interceptors never enter the
    #   image and cannot collide with TSan (see the file's header comment).
    "$CC_BIN" -c -std=c11 -fPIC -O0 -g "$TESTS_DIR/tsan_helpers.c" \
        -o "$WORKDIR/tsan_helpers.o" 2>"$WORKDIR/cc_helpers.err"
    if [ $? -ne 0 ]; then
        echo "C helper compile FAILED:"; cat "$WORKDIR/cc_helpers.err"
        FAIL=$((FAIL + 1)); RESULTS+=("$NAME: FAIL (cc helpers)"); continue
    fi

    "$CC_BIN" -c -std=c11 -fPIC -O0 -g "$TESTS_DIR/ffisan_shim.c" \
        -o "$WORKDIR/ffisan_shim.o" 2>"$WORKDIR/cc_shim.err"
    if [ $? -ne 0 ]; then
        echo "shim compile FAILED:"; cat "$WORKDIR/cc_shim.err"
        FAIL=$((FAIL + 1)); RESULTS+=("$NAME: FAIL (cc shim)"); continue
    fi

    # --- 2. Compile + link with the stage1 rustc ---------------------------
    # Link flags that make this work (determined empirically):
    #   -Zpre-link-arg=<shim.o>      place the shim BEFORE all rlibs, so the
    #                                linker never extracts libffisan's lib.c.o
    #   -Wl,--whole-archive <tsan.a> -Wl,--no-whole-archive
    #                                pull in the ENTIRE static TSan runtime
    #                                (its .preinit_array initializer must be in
    #                                the image for TSan to start)
    #   -lpthread -ldl -lrt -lm -lstdc++
    #                                dependencies of the TSan runtime
    "$RUSTC" --sysroot "$SYSROOT" -Zunstable-options \
        -Zpre-link-arg="$WORKDIR/ffisan_shim.o" \
        --edition 2021 -C opt-level=0 -C panic=unwind \
        -C link-arg="$WORKDIR/tsan_helpers.o" \
        -C link-arg=-Wl,--whole-archive \
        -C link-arg="$TSAN_LIB" \
        -C link-arg=-Wl,--no-whole-archive \
        -C link-arg=-lpthread -C link-arg=-ldl -C link-arg=-lrt \
        -C link-arg=-lm -C link-arg=-lstdc++ \
        -o "$BIN" "$SRC" 2>"$WORKDIR/rustc.err"
    RC=$?
    if [ $RC -ne 0 ]; then
        echo "COMPILE/LINK FAILED (exit $RC):"; cat "$WORKDIR/rustc.err"
        FAIL=$((FAIL + 1)); RESULTS+=("$NAME: FAIL (link)"); continue
    fi

    # --- 3. Run, capturing stdout/stderr separately ------------------------
    "$BIN" >"$STDOUT_LOG" 2>"$STDERR_LOG"
    RUN_RC=$?
    echo "--- run exit code: $RUN_RC (informational only) ---"
    echo "--- stdout ---"; cat "$STDOUT_LOG"
    echo "--- stderr ---"; cat "$STDERR_LOG"

    # --- 4. Judge by the ThreadSanitizer marker in stderr ------------------
    if grep -q "ThreadSanitizer" "$STDERR_LOG"; then
        SAW_TSAN=1
    else
        SAW_TSAN=0
    fi

    if [ "$EXPECT" = "race" ] && [ "$SAW_TSAN" -eq 1 ]; then
        echo "RESULT: PASS ($NAME) - ThreadSanitizer report present, as expected"
        PASS=$((PASS + 1)); RESULTS+=("$NAME: PASS")
    elif [ "$EXPECT" = "none" ] && [ "$SAW_TSAN" -eq 0 ]; then
        echo "RESULT: PASS ($NAME) - no ThreadSanitizer report, as expected"
        PASS=$((PASS + 1)); RESULTS+=("$NAME: PASS")
    else
        echo "RESULT: FAIL ($NAME) - expected '$EXPECT', ThreadSanitizer marker present=$SAW_TSAN"
        FAIL=$((FAIL + 1)); RESULTS+=("$NAME: FAIL")
    fi
    echo
done

echo "=================================================================="
echo "=== SUMMARY ==="
echo "=================================================================="
for R in "${RESULTS[@]}"; do echo "  $R"; done
echo "------------------------------------------------------------------"
echo "  total=${#TESTS[@]}  pass=$PASS  fail=$FAIL"
echo "=================================================================="

# Non-zero overall exit status if any test failed.
[ "$FAIL" -eq 0 ]
