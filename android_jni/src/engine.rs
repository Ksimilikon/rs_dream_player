//! `init`, the end-of-track listener and playback (`engine_*`).

#![allow(non_snake_case)]

use std::sync::{Arc, Mutex};

use api::{mem, player};
use jni::{
    EnvUnowned,
    errors::Result,
    jni_sig, jni_str,
    objects::{JClass, JObject, JString},
    refs::Global,
    vm::JavaVM,
};

use crate::util::{c_string, cstring, new_string, opt_string, run, string_list};

/// Kotlin `TrackEndListener` set by `setTrackEndListener` (`None` — no
/// notifications). `Arc` lets the audio thread take it without holding the
/// lock during the Java call.
static LISTENER: Mutex<Option<Arc<Global<JObject<'static>>>>> = Mutex::new(None);

/// end-of-track callback passed to the core; called from the core audio
/// thread, which is attached to the JVM for the call.
extern "C" fn on_track_end() {
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

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_init<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    cache_dir: JString<'c>,
    config_dir: JString<'c>,
    data_dir: JString<'c>,
    music_dir: JString<'c>,
    need_db: bool,
) {
    run(&mut env, |env| {
        let dir = |s: &JString| -> Result<_> {
            Ok(cstring(&opt_string(env, s)?.unwrap_or_default()))
        };
        let (cache, config, data, music) =
            (dir(&cache_dir)?, dir(&config_dir)?, dir(&data_dir)?, dir(&music_dir)?);
        api::init(cache.as_ptr(), config.as_ptr(), data.as_ptr(), music.as_ptr(), need_db);
        Ok(())
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

// ─────────────────────────────── playback ───────────────────────────────

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_engineAddTrackId<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    id: i64,
) -> bool {
    run(&mut env, |_| Ok(player::engine_add_track_id(id, Some(on_track_end))))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_engineAddTrackPath<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    path: JString<'c>,
) -> bool {
    run(&mut env, |env| {
        let Some(path) = opt_string(env, &path)? else {
            return Ok(false);
        };
        let c = cstring(&path);
        Ok(unsafe { player::engine_add_track_path(c.as_ptr(), Some(on_track_end)) })
    })
}

/// plain no-argument / primitive engine calls.
macro_rules! simple {
    ($($name:ident($($arg:ident: $ty:ty),*) $(-> $ret:ty)? => $call:expr;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name<'c>(
            mut env: EnvUnowned<'c>,
            _class: JClass<'c>,
            $($arg: $ty),*
        ) $(-> $ret)? {
            run(&mut env, |_| Ok($call))
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
    Java_dev_dreamplayer_core_Native_engineGetSeek() -> i64 => player::engine_get_seek() as i64;
    Java_dev_dreamplayer_core_Native_engineGetDuration() -> i64 => player::engine_get_duration() as i64;
    Java_dev_dreamplayer_core_Native_engineIsPause() -> bool => player::engine_is_pause();
    Java_dev_dreamplayer_core_Native_engineIsEmpty() -> bool => player::engine_is_empty();
    Java_dev_dreamplayer_core_Native_engineSetVolumeTrack(volume: f32) => player::engine_set_volume_track(volume);
    Java_dev_dreamplayer_core_Native_engineSetVolumeMaster(volume: f32) => player::engine_set_volume_master(volume);
    Java_dev_dreamplayer_core_Native_engineGetVolumeTrack() -> f32 => player::engine_get_volume_track();
    Java_dev_dreamplayer_core_Native_engineGetVolumeMaster() -> f32 => player::engine_get_volume_master();
}

// ─────────────────────────────── devices ────────────────────────────────

/// names of the output devices -> `List<String>`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_engineGetOutputDevices<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
) -> JObject<'c> {
    run(&mut env, |env| {
        let list = player::engine_get_output_devices();
        let names: Vec<String> = match list.ptr.is_null() {
            true => Vec::new(),
            false => unsafe { std::slice::from_raw_parts(list.ptr, list.len) }
                .iter()
                .filter_map(|&p| unsafe { c_string(p) })
                .collect(),
        };
        unsafe { mem::engine_free_output_devices(list) };
        string_list(env, &names)
    })
}

/// name of the current output device -> `String?`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_engineGetOutputDevice<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
) -> JObject<'c> {
    run(&mut env, |env| {
        let p = player::engine_get_output_device();
        let name = unsafe { c_string(p) };
        unsafe { mem::idx_free_string(p) };
        new_string(env, name.as_deref())
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_engineSetOutputDevice<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    name: JString<'c>,
) -> bool {
    run(&mut env, |env| {
        let Some(name) = opt_string(env, &name)? else {
            return Ok(false);
        };
        let c = cstring(&name);
        Ok(unsafe { player::engine_set_output_device(c.as_ptr()) })
    })
}
