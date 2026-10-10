// C helper library for the TSan FFI probe tests.
// Compiled un-instrumented with `cc` (gcc). The TSan runtime, linked
// separately, still observes these accesses through the interceptors it
// installs for malloc/pthread, but the plain stores below are deliberately
// NOT annotated by hand.
#include <stdlib.h>
#include <pthread.h>

// Allocate a heap buffer of `n` bytes.
void *c_make(unsigned long n) {
    return malloc(n);
}

// Free a buffer previously returned by c_make.
void c_free(void *p) {
    free(p);
}

// Single un-synchronized 32-bit store.
void c_touch_write(void *p, int v) {
    *(int *)p = v;
}

// Single un-synchronized 32-bit load.
int c_touch_read(const void *p) {
    return *(const int *)p;
}

// Internal lock protecting every access made through this entry point.
// Models a C API that is internally lock-protected.
static pthread_mutex_t g_lock = PTHREAD_MUTEX_INITIALIZER;

void c_locked_touch(void *p, int v) {
    pthread_mutex_lock(&g_lock);
    *(int *)p = v;
    pthread_mutex_unlock(&g_lock);
}
