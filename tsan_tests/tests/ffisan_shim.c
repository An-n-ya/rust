// Neutralize libffisan for TSan-linked test programs.
//
// The experimental stage1 `core`/`std` always pull in `libffisan.a` (see
// core/build.rs: `cargo:rustc-link-lib=static=ffisan`). That archive's
// `lib.c.o` defines strong `malloc`/`free`/`realloc`/`calloc`/
// `posix_memalign`/`aligned_alloc`/`memalign` interceptors (it is a
// drop-in libc allocator with red zones) plus the `__ffi_sanitizer_*`
// entry points the rlibs call.
//
// Those interceptors collide with the ThreadSanitizer runtime: during TSan
// bootstrap (preinit -> InitializeInterceptors -> dlsym -> glibc
// _dl_exception_create_format -> malloc) libffisan's malloc interceptor runs
// setup_internal() and calls pthread_mutex_init while TSan's REAL function
// pointer is still NULL, crashing with SIGSEGV before `main`.
//
// Because *std itself* has undefined references to the public names `free`
// and `malloc` (and libc is listed after the rlibs on the link line), simply
// providing the `__ffi_sanitizer_*` symbols is not enough: the linker would
// still extract `lib.c.o` from libcore.rlib to satisfy `free`/`malloc`
// (verified with `ld -Map`). So this shim defines the whole public
// malloc-family itself, forwarding to glibc's internal `__libc_*` entry
// points, *and* defines every `__ffi_sanitizer_*` symbol the rlibs reference.
// With all of `lib.c.o`'s externally visible symbols already defined, no
// archive member is ever extracted and libffisan's interceptors never enter
// the image.
//
// Consequence: heap allocation is served directly by glibc, so TSan's
// malloc interceptor is bypassed. That is fine for this harness -- race
// detection relies on the explicit `__tsan_read*/__tsan_write*` annotations,
// which operate on TSan shadow memory for any address regardless of how it
// was allocated. `__ffi_sanitizer_get_header` returns NULL so std's
// double-free / cross-language checks (a libffisan feature) are skipped.
#include <stddef.h>

// glibc internals, exported (GLIBC_2.2.5) but not declared in public headers.
extern void *__libc_malloc(size_t size);
extern void __libc_free(void *ptr);
extern void *__libc_realloc(void *ptr, size_t size);
extern void *__libc_calloc(size_t nitems, size_t size);
extern void *__libc_memalign(size_t alignment, size_t size);

// ---- public malloc-family: defined here so lib.c.o is never extracted ----
void *malloc(size_t size) { return __libc_malloc(size); }
void free(void *ptr) { __libc_free(ptr); }
void *realloc(void *ptr, size_t size) { return __libc_realloc(ptr, size); }
void *calloc(size_t nitems, size_t size) { return __libc_calloc(nitems, size); }

int posix_memalign(void **memptr, size_t alignment, size_t size) {
    // Mirror posix_memalign's contract on top of __libc_memalign.
    if (alignment == 0 || (alignment & (alignment - 1)) != 0 ||
        alignment % sizeof(void *) != 0) {
        return 22; // EINVAL
    }
    void *p = __libc_memalign(alignment, size);
    if (!p) {
        return 12; // ENOMEM
    }
    *memptr = p;
    return 0;
}

void *aligned_alloc(size_t alignment, size_t size) {
    return __libc_memalign(alignment, size);
}

void *memalign(size_t alignment, size_t size) {
    return __libc_memalign(alignment, size);
}

// ---- libffisan entry points referenced by core/std rlibs ------------------
void *__ffi_sanitizer_get_header(void *ptr) {
    (void)ptr;
    return NULL;
}

void *__ffi_sanitizer_rust_alloc(unsigned long size) {
    return __libc_malloc(size);
}

void __ffi_sanitizer_rust_free(void *ptr) {
    __libc_free(ptr);
}

void *__ffi_sanitizer_rust_realloc(void *ptr, unsigned long size) {
    return __libc_realloc(ptr, size);
}

void *__ffi_sanitizer_rust_calloc(unsigned long nitems, unsigned long size) {
    return __libc_calloc(nitems, size);
}

int __ffi_sanitizer_rust_posix_memalign(void **memptr, unsigned long alignment,
                                        unsigned long size) {
    return posix_memalign(memptr, alignment, size);
}

// Not referenced by the rlibs, but defined defensively so that even if a
// future std build calls them, `lib.c.o` still stays out of the link.
int __ffi_sanitizer_check_red_zone(void *ptr) {
    (void)ptr;
    return 0;
}

int __ffi_sanitizer_print_leak_summary(void) {
    return 0;
}

void __ffi_sanitizer_put_alloc_list(void *data, char *file, unsigned len,
                                    unsigned line) {
    (void)data;
    (void)file;
    (void)len;
    (void)line;
}
