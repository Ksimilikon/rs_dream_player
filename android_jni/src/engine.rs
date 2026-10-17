//! `init`, the end-of-track listener and playback (`engine_*`). Statuses are
//! `api::FFIInitStatus` / `api::player::FFIEngineStatus` codes as `Int`;
//! getters return `CoreResult(status, value)`.

#![allow(non_snake_case)]

use std::ptr;
use std::sync::{Arc, Mutex};

use api::idx::FFIStringList;
use api::mem;
use api::player::{self, FFIEngineStatus};
use jni::{
    EnvUnowned,
    errors::Result,
    jni_sig, jni_str,
    objects::{JClass, JObject, JString},
    refs::Global,
    vm::JavaVM,
};

use crate::util::{
    boxed_bool, boxed_float, boxed_long, c_string, core_result, cstring, new_string, opt_string,
    run, string_list,
};

/// status -> the `Int` returned to Kotlin.
fn code(st: FFIEngineStatus) -> i32 {
    st as u32 as i32
}

/// Kotlin `TrackEndListener` set by `setTrackEndListener` (`None` — no
/// notifications). `Arc` lets the audio thread take it without holding the
/// lock during the Java call.
static LISTENER: Mutex<Option<Arc<Global<JObject<'static>>>>> = Mutex::new(None);

/// end-of-track callback passed to the core; called from the core audio
/// thread, which is attached to the JVM for the call. A panic here would
/// abort the process (it crosses `extern "C"`), so it is caught and recorded
/// by the core.
extern "C" fn on_track_end() {
    let _ = api::panic::catch(notify_track_end);
}

fn notify_track_end() {
    let Some(listener) = LISTENER.lock().ok().and_then(|l| l.clone()) else {
        return;
    };
    let Ok(vm) = JavaVM::singleton() else {
        return;
    };
    let _ = vm.attach_current_thread(|env| -> Result<()> {
        let res = env.call_method(
            listener.as_ref(),
            jni_str!("onTrackEnd"),
            jni_sig!("()V"),
            &[],
        );
        // an exception thrown by the listener must not stay pending on the
        // audio thread.
        if res.is_err() && env.exception_check() {
            env.exception_describe();
            env.exception_clear();
        }
        Ok(())
    });
}

// ───────────────────────────────── init ─────────────────────────────────

/// hands the JavaVM and the application context to `ndk_context` (once per
/// process): the audio backend (cpal / AAudio) reads them from there and
/// panics without them. A JNI library has no ndk-glue to do it.
fn init_android_context(env: &mut jni::Env, context: &JObject) -> Result<()> {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if context.is_null() || DONE.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Ok(());
    }
    let app = env
        .call_method(context, jni_str!("getApplicationContext"), jni_sig!("()Landroid/content/Context;"), &[])?
        .l()?;
    let app = match app.is_null() {
        true => env.new_global_ref(context)?,
        false => env.new_global_ref(&app)?,
    };
    let vm = env.get_java_vm()?;
    // the global ref is leaked on purpose: the context must outlive the core.
    unsafe { ndk_context::initialize_android_context(vm.get_raw().cast(), app.into_raw().cast()) };
    Ok(())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_init<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    context: JObject<'c>,
    cache_dir: JString<'c>,
    config_dir: JString<'c>,
    data_dir: JString<'c>,
    music_dir: JString<'c>,
    need_db: bool,
) -> i32 {
    // the core hook first: the logcat logger then wraps it and logs every
    // panic, including the ones the core catches and returns as `PANIC`.
    api::panic::install_hook();
    crate::util::install_panic_logger();
    run(&mut env, |env| {
        init_android_context(env, &context)?;
        // a null directory stays a null pointer: the core reports it.
        let dir = |s: &JString| -> Result<_> { Ok(opt_string(env, s)?.map(|s| cstring(&s))) };
        let (cache, config, data, music) =
            (dir(&cache_dir)?, dir(&config_dir)?, dir(&data_dir)?, dir(&music_dir)?);
        let st = unsafe {
            api::init(
                crate::util::opt_ptr(&cache),
                crate::util::opt_ptr(&config),
                crate::util::opt_ptr(&data),
                crate::util::opt_ptr(&music),
                need_db,
            )
        };
        Ok(st as u32 as i32)
    })
}

/// sets (or clears with null) the end-of-track listener.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_setTrackEndListener<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    listener: JObject<'c>,
) {
    run(&mut env, |env| {
        let new = match listener.is_null() {
            true => None,
            false => Some(Arc::new(env.new_global_ref(&listener)?)),
        };
        if let Ok(mut slot) = LISTENER.lock() {
            *slot = new;
        }
        Ok(())
    })
}

// ───────────────────────────── diagnostics ──────────────────────────────

/// number of core panics in this process (caught and uncaught).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_corePanicCount<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
) -> i64 {
    run(&mut env, |_| Ok(api::panic::core_panic_count() as i64))
}

/// message of the last core panic -> `String?` (null — none).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_coreLastPanic<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
) -> JObject<'c> {
    run(&mut env, |env| {
        let p = api::panic::core_last_panic();
        let msg = unsafe { c_string(p) };
        unsafe { mem::idx_free_string(p) };
        new_string(env, msg.as_deref())
    })
}

// ─────────────────────────────── playback ───────────────────────────────

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_engineAddTrackId<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    id: i64,
) -> i32 {
    run(&mut env, |_| Ok(code(player::engine_add_track_id(id, Some(on_track_end)))))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_engineAddTrackPath<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    path: JString<'c>,
) -> i32 {
    run(&mut env, |env| {
        let path = opt_string(env, &path)?.map(|p| cstring(&p));
        Ok(code(unsafe {
            player::engine_add_track_path(crate::util::opt_ptr(&path), Some(on_track_end))
        }))
    })
}

/// plain no-argument / primitive engine calls -> status `Int`.
macro_rules! simple {
    ($($name:ident($($arg:ident: $ty:ty),*) => $call:expr;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name<'c>(
            mut env: EnvUnowned<'c>,
            _class: JClass<'c>,
            $($arg: $ty),*
        ) -> i32 {
            run(&mut env, |_| Ok(code($call)))
        }
    )*};
}

simple! {
    Java_dev_dreamplayer_core_Native_enginePlayPause() => player::engine_play_pause();
    Java_dev_dreamplayer_core_Native_enginePlay() => player::engine_play();
    Java_dev_dreamplayer_core_Native_enginePause() => player::engine_pause();
    Java_dev_dreamplayer_core_Native_engineStop() => player::engine_stop();
    Java_dev_dreamplayer_core_Native_engineSetSpeed(speed: f32) => player::engine_set_speed(speed);
    Java_dev_dreamplayer_core_Native_engineSetSeek(sec: i64) => player::engine_set_seek(sec.max(0) as u64);
    Java_dev_dreamplayer_core_Native_engineSetVolumeTrack(volume: f32) => player::engine_set_volume_track(volume);
    Java_dev_dreamplayer_core_Native_engineSetVolumeMaster(volume: f32) => player::engine_set_volume_master(volume);
}

/// engine getters -> `CoreResult<T>`: `$get` is an `engine_get_*(out)`
/// function, `$boxed` turns the value into the boxed Java object.
macro_rules! getter {
    ($($name:ident => $get:path, $init:expr, |$env:ident, $v:ident| $boxed:expr;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name<'c>(
            mut env: EnvUnowned<'c>,
            _class: JClass<'c>,
        ) -> JObject<'c> {
            run(&mut env, |$env| {
                let mut $v = $init;
                let st = unsafe { $get(&mut $v) };
                let value = match st {
                    FFIEngineStatus::Ok => $boxed?,
                    _ => JObject::null(),
                };
                core_result($env, code(st), &value)
            })
        }
    )*};
}

getter! {
    Java_dev_dreamplayer_core_Native_engineGetSeek => player::engine_get_seek, 0u64,
        |env, v| boxed_long(env, Some(v as i64));
    Java_dev_dreamplayer_core_Native_engineGetDuration => player::engine_get_duration, 0u64,
        |env, v| boxed_long(env, Some(v as i64));
    Java_dev_dreamplayer_core_Native_engineIsPause => player::engine_is_pause, false,
        |env, v| boxed_bool(env, v);
    Java_dev_dreamplayer_core_Native_engineIsEmpty => player::engine_is_empty, false,
        |env, v| boxed_bool(env, v);
    Java_dev_dreamplayer_core_Native_engineGetVolumeTrack => player::engine_get_volume_track, 0f32,
        |env, v| boxed_float(env, v);
    Java_dev_dreamplayer_core_Native_engineGetVolumeMaster => player::engine_get_volume_master, 0f32,
        |env, v| boxed_float(env, v);
}

// ─────────────────────────────── devices ────────────────────────────────

/// names of the output devices -> `CoreResult<List<String>>`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_engineGetOutputDevices<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
) -> JObject<'c> {
    run(&mut env, |env| {
        let mut list = FFIStringList {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let st = unsafe { player::engine_get_output_devices(&mut list) };
        if st != FFIEngineStatus::Ok {
            return core_result(env, code(st), &JObject::null());
        }
        let names: Vec<String> = match list.ptr.is_null() {
            true => Vec::new(),
            false => unsafe { std::slice::from_raw_parts(list.ptr, list.len) }
                .iter()
                .filter_map(|&p| unsafe { c_string(p) })
                .collect(),
        };
        unsafe { mem::engine_free_output_devices(list) };
        let names = string_list(env, &names)?;
        core_result(env, code(st), &names)
    })
}

/// name of the current output device -> `CoreResult<String>` (the value is
/// null on `OK` too when the device is unknown).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_engineGetOutputDevice<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
) -> JObject<'c> {
    run(&mut env, |env| {
        let mut p = ptr::null_mut();
        let st = unsafe { player::engine_get_output_device(&mut p) };
        let name = unsafe { c_string(p) };
        unsafe { mem::idx_free_string(p) };
        let name = new_string(env, name.as_deref())?;
        core_result(env, code(st), &name)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_engineSetOutputDevice<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    name: JString<'c>,
) -> i32 {
    run(&mut env, |env| {
        let name = opt_string(env, &name)?.map(|n| cstring(&n));
        Ok(code(unsafe { player::engine_set_output_device(crate::util::opt_ptr(&name)) }))
    })
}
