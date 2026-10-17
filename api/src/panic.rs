//! panic safety net of the C ABI.
//!
//! - every exported function runs its body under [`guard`] (`catch_unwind`):
//!   a panic becomes the `Panic` status of its subsystem instead of unwinding
//!   into foreign code (which aborts the process). Out-parameters keep their
//!   empty values.
//! - a process-wide panic hook ([`install_hook`]) records EVERY panic — also
//!   the ones `guard` can't catch (core audio threads, callbacks): it bumps a
//!   counter, keeps the last message (`core_panic_count` /
//!   `core_last_panic`) and appends it to `<data>/core_panic.log` (the `data`
//!   directory of `init`), one line per panic:
//!   `<unix secs> [caught|UNCAUGHT] thread '<name>' panicked at <file:line:col>: <message>`.
//!   The file survives an abort, so the frontend can find out after a restart
//!   what happened (`UNCAUGHT` — the panic was not turned into a status: it
//!   killed a core thread or the whole process).
//! - a panic caught by `guard` is not passed on to the previous hook (the
//!   default one prints to stderr, a frontend's one may tear down its UI): it
//!   is reported by the status. Uncaught panics go on to the previous hook.
//!
//! after a caught panic the core may be left half-updated (the core lock is
//! taken over, see [`crate::core`]); the frontend decides whether to go on or
//! restart.

use std::{
    cell::Cell,
    ffi::c_char,
    fs::OpenOptions,
    io::Write,
    panic::{AssertUnwindSafe, PanicHookInfo, catch_unwind},
    path::PathBuf,
    ptr,
    sync::{
        Mutex, Once, OnceLock, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use crate::idx::to_c;

/// the safety net needs unwinding: with `panic = "abort"` `catch_unwind`
/// catches nothing and every panic kills the host process.
#[cfg(panic = "abort")]
compile_error!("the core C ABI requires `panic = \"unwind\"`");

/// log file name in the `data` directory.
pub const PANIC_LOG: &str = "core_panic.log";

/// the log is started over once it grows past this size.
const LOG_MAX_BYTES: u64 = 256 * 1024;

static HOOK: Once = Once::new();
static COUNT: AtomicU64 = AtomicU64::new(0);
static LAST: Mutex<Option<String>> = Mutex::new(None);
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

thread_local! {
    /// depth of [`guard`] calls on this thread (> 0 — a panic will be caught).
    static GUARDED: Cell<u32> = const { Cell::new(0) };
}

/// a status enum of a subsystem: the codes the safety net needs.
pub(crate) trait Status: Copy {
    const OK: Self;
    const NULL_POINTER: Self;
    const PANIC: Self;
}

/// installs the panic hook (once per process; every exported function calls
/// it, so it is in place before any core code runs). Rust frontends that set
/// their own hook should do it BEFORE the first core call: the core hook then
/// wraps theirs and keeps caught panics away from it.
pub fn install_hook() {
    HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let caught = GUARDED.with(Cell::get) > 0;
            record(info, caught);
            if !caught {
                prev(info);
            }
        }));
    });
}

/// from now on panics are also appended to `<data_dir>/core_panic.log`
/// (`init`; the first directory wins).
pub(crate) fn set_log_dir(data_dir: &std::path::Path) {
    let _ = LOG_PATH.set(data_dir.join(PANIC_LOG));
}

/// counts the panic, keeps its message and appends it to the log. Must not
/// panic itself (a panic inside the hook aborts).
fn record(info: &PanicHookInfo, caught: bool) {
    let thread = std::thread::current();
    // one line per panic: the log is read line by line.
    let msg = format!(
        "[{}] thread '{}' {info}",
        if caught { "caught" } else { "UNCAUGHT" },
        thread.name().unwrap_or("<unnamed>"),
    )
    .replace('\n', " ");
    COUNT.fetch_add(1, Ordering::SeqCst);
    if let Some(path) = LOG_PATH.get() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let too_big = std::fs::metadata(path).is_ok_and(|m| m.len() > LOG_MAX_BYTES);
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(!too_big)
            .truncate(too_big)
            .open(path);
        if let Ok(mut f) = file {
            let _ = writeln!(f, "{ts} {msg}");
        }
    }
    *LAST.lock().unwrap_or_else(PoisonError::into_inner) = Some(msg);
}

/// runs an exported function body: a panic -> `S::PANIC`.
pub(crate) fn guard<S: Status>(f: impl FnOnce() -> S) -> S {
    catch(f).unwrap_or(S::PANIC)
}

/// [`guard`] for functions without a result (`*_free_*`): the panic is only
/// recorded.
pub(crate) fn guard_unit(f: impl FnOnce()) {
    let _ = catch(f);
}

/// `catch_unwind` with the hook told that the panic is being caught (it is
/// recorded as `[caught]` and not passed on to the previous hook). `None` —
/// `f` panicked. Public for Rust bindings over the C ABI that run code of
/// their own on core threads (e.g. JNI callbacks).
pub fn catch<R>(f: impl FnOnce() -> R) -> Option<R> {
    install_hook();
    GUARDED.with(|g| g.set(g.get() + 1));
    let res = catch_unwind(AssertUnwindSafe(f));
    GUARDED.with(|g| g.set(g.get() - 1));
    res.ok()
}

/// number of panics in this process so far (caught and uncaught). Works
/// before `init` too.
#[unsafe(no_mangle)]
pub extern "C" fn core_panic_count() -> u64 {
    COUNT.load(Ordering::SeqCst)
}

/// message of the last panic (thread, location, text), or null if there was
/// none. Free with `idx_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn core_last_panic() -> *mut c_char {
    catch(|| {
        LAST.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_deref()
            .map_or(ptr::null_mut(), to_c)
    })
    .unwrap_or(ptr::null_mut())
}

#[cfg(test)]
mod tests {
    use std::ffi::CStr;

    use super::*;
    use crate::idx::FFIIdxStatus;
    use crate::player::FFIEngineStatus;

    /// the counter / last message are process-wide: panic tests run one at
    /// a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn last() -> String {
        let p = core_last_panic();
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        unsafe { crate::mem::idx_free_string(p) };
        s
    }

    #[test]
    fn panic_becomes_status_and_is_recorded() {
        let _serial = serial();
        let before = core_panic_count();
        let st: FFIIdxStatus = guard(|| panic!("boom-guard"));
        assert_eq!(st, FFIIdxStatus::Panic);
        assert!(core_panic_count() > before);
        let msg = last();
        assert!(msg.contains("boom-guard") && msg.contains("[caught]"), "{msg}");
        // the thread-local depth is restored after the catch.
        assert_eq!(GUARDED.with(Cell::get), 0);
    }

    #[test]
    fn with_out_writes_empty_on_panic() {
        let _serial = serial();
        let mut v = 7u64;
        let st: FFIEngineStatus =
            unsafe { crate::with_out(&mut v, 0, || -> Result<u64, _> { panic!("boom-out") }) };
        assert_eq!(st, FFIEngineStatus::Panic);
        assert_eq!(v, 0);
    }

    #[test]
    fn panic_is_appended_to_log() {
        let _serial = serial();
        // the log directory is process-wide: only this test sets it.
        let dir = std::env::temp_dir().join(format!("dp_panic_log_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        set_log_dir(&dir);
        let _: FFIIdxStatus = guard(|| panic!("boom-log"));
        let log = std::fs::read_to_string(dir.join(PANIC_LOG)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(log.contains("boom-log"), "{log}");
    }
}
