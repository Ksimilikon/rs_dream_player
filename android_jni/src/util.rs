//! conversions between Rust / C ABI values and Java objects.

use std::ffi::{CStr, CString, c_char};
use std::fs::File;
use std::mem::ManuallyDrop;
use std::os::fd::FromRawFd;

use jni::{
    Env, JValue,
    errors::Result,
    jni_sig, jni_str,
    objects::{JObject, JString},
};

/// Java string -> Rust string; null -> `None`.
pub(crate) fn opt_string(env: &Env, s: &JString) -> Result<Option<String>> {
    match s.is_null() {
        true => Ok(None),
        false => s.try_to_string(env).map(Some),
    }
}

/// Rust string -> `CString` for the C ABI (an interior NUL truncates nothing,
/// the string becomes empty — the core then rejects it as invalid input).
pub(crate) fn cstring(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

/// optional `CString` -> pointer (null for `None`); the `CString` must outlive
/// the call.
pub(crate) fn opt_ptr(c: &Option<CString>) -> *const c_char {
    c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr())
}

/// C string owned by the core -> Rust string (null -> `None`). Copies, frees
/// nothing.
///
/// # Safety
/// `p` is null or a valid nul-terminated C string.
pub(crate) unsafe fn c_string(p: *const c_char) -> Option<String> {
    match p.is_null() {
        true => None,
        false => Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()),
    }
}

/// `Option<&str>` -> `java.lang.String` or null.
pub(crate) fn new_string<'l>(env: &mut Env<'l>, s: Option<&str>) -> Result<JObject<'l>> {
    match s {
        Some(s) => Ok(JString::from_str(env, s)?.into()),
        None => Ok(JObject::null()),
    }
}

/// `Option<i64>` -> `java.lang.Long` (Kotlin `Long?`) or null.
pub(crate) fn boxed_long<'l>(env: &mut Env<'l>, v: Option<i64>) -> Result<JObject<'l>> {
    let Some(v) = v else {
        return Ok(JObject::null());
    };
    env.call_static_method(
        jni_str!("java/lang/Long"),
        jni_str!("valueOf"),
        jni_sig!("(J)Ljava/lang/Long;"),
        &[JValue::Long(v)],
    )?
    .l()
}

/// objects -> `java.util.List` (Kotlin `List<T>`), via `Arrays.asList`.
pub(crate) fn list<'l>(env: &mut Env<'l>, items: Vec<JObject<'l>>) -> Result<JObject<'l>> {
    let array = env.new_object_array(
        items.len() as i32,
        jni_str!("java/lang/Object"),
        JObject::null(),
    )?;
    for (i, item) in items.iter().enumerate() {
        array.set_element(env, i, item)?;
    }
    let array: JObject = array.into();
    env.call_static_method(
        jni_str!("java/util/Arrays"),
        jni_str!("asList"),
        jni_sig!("([Ljava/lang/Object;)Ljava/util/List;"),
        &[JValue::Object(&array)],
    )?
    .l()
}

/// strings -> `java.util.List<String>`.
pub(crate) fn string_list<'l>(env: &mut Env<'l>, items: &[String]) -> Result<JObject<'l>> {
    let mut objs = Vec::with_capacity(items.len());
    for s in items {
        objs.push(new_string(env, Some(s))?);
    }
    list(env, objs)
}

/// bytes -> `byte[]` (Kotlin `ByteArray`).
pub(crate) fn byte_array<'l>(env: &mut Env<'l>, bytes: &[u8]) -> Result<JObject<'l>> {
    Ok(env.byte_array_from_slice(bytes)?.into())
}

/// a borrowed file descriptor from Kotlin as a `File` that is never closed
/// here (Kotlin owns and closes it).
///
/// # Safety
/// `fd` is an open descriptor that stays valid for the lifetime of the result.
pub(crate) unsafe fn borrowed_file(fd: i32) -> ManuallyDrop<File> {
    ManuallyDrop::new(unsafe { File::from_raw_fd(fd) })
}

/// runs the body of a native method: upgrades the env (catching panics) and
/// turns a JNI error into a Java `RuntimeException` + the default value.
pub(crate) fn run<'c, T: Default + 'c>(
    env: &mut jni::EnvUnowned<'c>,
    f: impl FnOnce(&mut Env<'c>) -> Result<T>,
) -> T {
    env.with_env(f)
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

unsafe extern "C" {
    /// liblog (always present on Android, already linked by the audio backend).
    fn __android_log_write(prio: i32, tag: *const c_char, text: *const c_char) -> i32;
}

/// installs (once) a panic hook that writes the panic message and location
/// to logcat (tag `dream_player`). stderr goes nowhere on Android, so without
/// it a panic in the core (which aborts across `extern "C"`) leaves no trace.
pub(crate) fn install_panic_logger() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            const ANDROID_LOG_FATAL: i32 = 7;
            let tag = cstring("dream_player");
            let text = cstring(&format!("panic: {info}"));
            unsafe { __android_log_write(ANDROID_LOG_FATAL, tag.as_ptr(), text.as_ptr()) };
            prev(info);
        }));
    });
}
