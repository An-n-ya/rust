// C helper library for FFIProbe reproduction tests.
// Compiled and linked into each test binary via the `cc` crate (build.rs).
#include <stdlib.h>
#include <string.h>

// Allocate `n` bytes with plain malloc (marked F_ALLOC_C by libffisan when
// linked against it, but for these standalone tests we rely on the fact that
// the pointer flows through Rust FFI exports for detection).
void *c_make_buf(size_t n) {
    void *p = malloc(n);
    if (p) {
        memset(p, 0, n);
    }
    return p;
}

void *c_make(size_t n) {
    void *p = malloc(n);
    if (p) {
        memset(p, 0, n);
    }
    return p;
}

void c_free(void *p) {
    free(p);
}

int c_touch(const void *p) {
    // Read a byte; returns 0. Used to force a pointer through an FFI call so
    // that FFIProbe's pre/post conditions are evaluated.
    if (p) {
        return (int)(*(const unsigned char *)p);
    }
    return 0;
}
