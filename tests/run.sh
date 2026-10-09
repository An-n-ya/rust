#!/usr/bin/env bash
# Build and run an FFIProbe test program using the locally-built FFIProbe compiler.
#
# Usage: ./run.sh <test.rs>
#
# It compiles the given single-file program with the stage2 rustc, linking the
# C helper shim (ffi_helpers.c) via the C compiler, then runs it and reports the
# exit status. FFIProbe diagnostics ("FFI Probe: ...") are printed by the
# runtime/panics and are captured in the output.
set -u

RUST_ROOT="/Users/annya/Documents/ustc/compiler/rust"
TESTS_DIR="$(cd "$(dirname "$0")" && pwd)"
STAGE2_RUSTC="$RUST_ROOT/build/aarch64-apple-darwin/stage2/bin/rustc"
SYSROOT="$RUST_ROOT/build/aarch64-apple-darwin/stage1"

if [[ ! -x "$STAGE2_RUSTC" ]]; then
  echo "ERROR: stage2 rustc not found at $STAGE2_RUSTC" >&2
  echo "Build it first with: python3 x.py build --stage 2" >&2
  exit 2
fi

SRC="${1:?usage: ./run.sh <test.rs>}"
NAME="$(basename "${SRC%.rs}")"
WORKDIR="$TESTS_DIR/_build/$NAME"
mkdir -p "$WORKDIR"

CC_BIN="${CC:-cc}"

echo "=== FFIProbe test: $NAME ==="
echo "rustc : $STAGE2_RUSTC"
echo "sysroot: $SYSROOT"
echo

# Compile the C helper into an object file.
"$CC_BIN" -c -std=c11 -fPIC -O0 -g "$TESTS_DIR/ffi_helpers.c" -o "$WORKDIR/ffi_helpers.o"
if [[ $? -ne 0 ]]; then
  echo "C helper compilation failed" >&2
  exit 3
fi

# Compile the Rust test with the FFIProbe compiler, linking the C helper object.
"$STAGE2_RUSTC" --sysroot "$SYSROOT" \
  -Zunstable-options \
  --edition 2021 \
  -C opt-level=0 \
  -C panic=unwind \
  -C link-arg="$WORKDIR/ffi_helpers.o" \
  -o "$WORKDIR/$NAME" \
  "$SRC"
RC=$?
if [[ $RC -ne 0 ]]; then
  echo
  echo "=== COMPILE FAILED (exit $RC) ==="
  exit $RC
fi

echo
echo "=== running $NAME ==="
"$WORKDIR/$NAME"
RUN_RC=$?
echo
echo "=== exit code: $RUN_RC ==="
if [[ $RUN_RC -ne 0 ]]; then
  echo "(non-zero exit indicates FFIProbe detected a memory-safety issue)"
fi
exit $RUN_RC
