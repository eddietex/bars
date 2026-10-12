//! Running work on the main thread. Keyboard-layout lookups (used to type
//! and to find the key for Cmd+V) and the permission prompts abort the
//! process when called from any other thread.

use std::ffi::c_void;

unsafe extern "C" {
    // `dispatch_get_main_queue()` is a macro for the address of this.
    static _dispatch_main_q: c_void;
    fn dispatch_sync_f(queue: *const c_void, context: *mut c_void, work: unsafe extern "C" fn(*mut c_void));
    fn pthread_main_np() -> i32;
}

struct Job<F, R> {
    f: Option<F>,
    out: Option<std::thread::Result<R>>,
}

unsafe extern "C" fn trampoline<F: FnOnce() -> R, R>(context: *mut c_void) {
    let job = unsafe { &mut *(context as *mut Job<F, R>) };
    if let Some(f) = job.f.take() {
        job.out = Some(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
    }
}

/// Runs `f` on the main thread and waits for it. The app's event loop
/// services the main queue; before it starts, this waits until it does.
///
/// Values captured by `f` are only touched while the calling thread is
/// blocked here, so they don't need to be `Send`.
pub fn run<R, F: FnOnce() -> R>(f: F) -> R {
    if unsafe { pthread_main_np() } != 0 {
        return f();
    }
    let mut job = Job::<F, R> { f: Some(f), out: None };
    unsafe {
        dispatch_sync_f(
            &raw const _dispatch_main_q,
            &mut job as *mut Job<F, R> as *mut c_void,
            trampoline::<F, R>,
        );
    }
    match job.out.expect("the main queue ran the job") {
        Ok(r) => r,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
