//! Runtime shims for the FFI-boundary data race sanitizer.
//!
//! Every `ffi_race_*` function is a thin wrapper over a ThreadSanitizer (TSan)
//! runtime entry point. The MIR pass `FFIRaceSanitizer` locates these wrappers
//! through their `#[rustc_diagnostic_item]` names and inserts calls to them at
//! FFI boundaries, so that only accesses that cross the Rust/C boundary are
//! reported to TSan.
//!
//! When the `ffi_race_sanitizer` feature is not enabled nothing ever calls into
//! this module, so no `__tsan_*` symbol is referenced by the produced binary.

use crate::ffi::{c_int, c_uint, c_ulong, c_void};
use crate::mem::size_of;

/// Process-wide identity used by the mutex annotations. TSan keys lock/unlock
/// ordering on the *address* passed to `__tsan_mutex_*`; it must therefore be a
/// stable address that is never also accessed as ordinary data.
static MUTEX_TOKEN: u8 = 0;

#[inline]
fn mutex_token() -> *mut c_void {
    crate::ptr::addr_of!(MUTEX_TOKEN) as *mut c_void
}

unsafe extern "C" {
    fn __tsan_read1(addr: *mut c_void);
    fn __tsan_read2(addr: *mut c_void);
    fn __tsan_read4(addr: *mut c_void);
    fn __tsan_read8(addr: *mut c_void);
    fn __tsan_read16(addr: *mut c_void);
    fn __tsan_write1(addr: *mut c_void);
    fn __tsan_write2(addr: *mut c_void);
    fn __tsan_write4(addr: *mut c_void);
    fn __tsan_write8(addr: *mut c_void);
    fn __tsan_write16(addr: *mut c_void);
    fn __tsan_read_range(addr: *mut c_void, size: c_ulong);
    fn __tsan_write_range(addr: *mut c_void, size: c_ulong);
    fn __tsan_acquire(addr: *mut c_void);
    fn __tsan_release(addr: *mut c_void);
    fn __tsan_mutex_pre_lock(addr: *mut c_void, flags: c_uint);
    fn __tsan_mutex_post_lock(addr: *mut c_void, flags: c_uint, recursion: c_int);
    fn __tsan_mutex_pre_unlock(addr: *mut c_void, flags: c_uint) -> c_int;
    fn __tsan_mutex_post_unlock(addr: *mut c_void, flags: c_uint);
}

#[inline]
unsafe fn tsan_read(addr: *const c_void, size: usize) {
    let addr = addr as *mut c_void;
    match size {
        1 => unsafe { __tsan_read1(addr) },
        2 => unsafe { __tsan_read2(addr) },
        4 => unsafe { __tsan_read4(addr) },
        8 => unsafe { __tsan_read8(addr) },
        16 => unsafe { __tsan_read16(addr) },
        n => unsafe { __tsan_read_range(addr, n as c_ulong) },
    }
}

#[inline]
unsafe fn tsan_write(addr: *const c_void, size: usize) {
    let addr = addr as *mut c_void;
    match size {
        1 => unsafe { __tsan_write1(addr) },
        2 => unsafe { __tsan_write2(addr) },
        4 => unsafe { __tsan_write4(addr) },
        8 => unsafe { __tsan_write8(addr) },
        16 => unsafe { __tsan_write16(addr) },
        n => unsafe { __tsan_write_range(addr, n as c_ulong) },
    }
}

/// Report a read of `*ptr` at an FFI boundary.
#[track_caller]
#[unstable(feature = "ffi_race_sanitizer", issue = "none")]
#[rustc_diagnostic_item = "ffi_race_read"]
pub unsafe fn ffi_race_read<T>(ptr: *const T) {
    unsafe { tsan_read(ptr as *const c_void, size_of::<T>()) }
}

/// Report a write of `*ptr` at an FFI boundary.
#[track_caller]
#[unstable(feature = "ffi_race_sanitizer", issue = "none")]
#[rustc_diagnostic_item = "ffi_race_write"]
pub unsafe fn ffi_race_write<T>(ptr: *mut T) {
    unsafe { tsan_write(ptr as *const c_void, size_of::<T>()) }
}

/// Establish the acquire side of a happens-before relation on `*ptr`.
#[track_caller]
#[unstable(feature = "ffi_race_sanitizer", issue = "none")]
#[rustc_diagnostic_item = "ffi_race_acquire"]
pub unsafe fn ffi_race_acquire<T>(ptr: *const T) {
    unsafe { __tsan_acquire(ptr as *const c_void as *mut c_void) }
}

/// Establish the release side of a happens-before relation on `*ptr`.
#[track_caller]
#[unstable(feature = "ffi_race_sanitizer", issue = "none")]
#[rustc_diagnostic_item = "ffi_race_release"]
pub unsafe fn ffi_race_release<T>(ptr: *const T) {
    unsafe { __tsan_release(ptr as *const c_void as *mut c_void) }
}

/// Annotate the start of a lock operation performed inside a C API.
///
/// The annotation is keyed on a dedicated process-wide token rather than on the
/// data pointer: reusing the data address as the mutex identity makes TSan's
/// mutex shadow collide with the shadow of the data itself, which the runtime
/// then reports as a spurious race against the very access the annotation is
/// meant to protect.
#[track_caller]
#[unstable(feature = "ffi_race_sanitizer", issue = "none")]
#[rustc_diagnostic_item = "ffi_race_mutex_pre_lock"]
pub unsafe fn ffi_race_mutex_pre_lock<T>(_ptr: *const T) {
    unsafe { __tsan_mutex_pre_lock(mutex_token(), 0) }
}

/// Annotate the end of a lock operation performed inside a C API.
#[track_caller]
#[unstable(feature = "ffi_race_sanitizer", issue = "none")]
#[rustc_diagnostic_item = "ffi_race_mutex_post_lock"]
pub unsafe fn ffi_race_mutex_post_lock<T>(_ptr: *const T) {
    unsafe { __tsan_mutex_post_lock(mutex_token(), 0, 0) }
}

/// Annotate the start of an unlock operation performed inside a C API.
#[track_caller]
#[unstable(feature = "ffi_race_sanitizer", issue = "none")]
#[rustc_diagnostic_item = "ffi_race_mutex_pre_unlock"]
pub unsafe fn ffi_race_mutex_pre_unlock<T>(_ptr: *const T) {
    unsafe {
        __tsan_mutex_pre_unlock(mutex_token(), 0);
    }
}

/// Annotate the end of an unlock operation performed inside a C API.
#[track_caller]
#[unstable(feature = "ffi_race_sanitizer", issue = "none")]
#[rustc_diagnostic_item = "ffi_race_mutex_post_unlock"]
pub unsafe fn ffi_race_mutex_post_unlock<T>(_ptr: *const T) {
    unsafe { __tsan_mutex_post_unlock(mutex_token(), 0) }
}
