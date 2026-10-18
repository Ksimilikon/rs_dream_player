//! library sync between paired devices (`sync_*`). Statuses are
//! `api::sync::FFISyncStatus` codes as `Int`; a session is an opaque `Long`
//! handle (the core pointer). Kotlin carries the bytes over its transport:
//! received bytes go in with `syncFeed`, the bytes to send are written
//! straight into the transport's descriptor (`syncWriteOutput`) and the
//! received files into the target files' descriptors
//! (`syncImportWriteFile`), so neither passes through the Java heap.
//!
//! [`IO_FAILED`] is JNI-only: the descriptor couldn't be written.

#![allow(non_snake_case)]

use std::ffi::{CString, c_char};
use std::io::Write;
use std::ptr;

use api::mem;
use api::sync::{self, FFISyncEvent, FFISyncEventKind, FFISyncFiles, FFISyncSession, FFISyncStatus};
use jni::{
    Env, EnvUnowned, JValue,
    errors::Result,
    jni_sig, jni_str,
    objects::{JByteArray, JClass, JLongArray, JObject, JObjectArray, JString},
};

use crate::idx::{c_string_array, long_array};
use crate::util::{borrowed_file, boxed_long, c_string, core_result, cstring, list, new_string, opt_string, run};

/// the descriptor passed from Kotlin couldn't be written (not an
/// `FFISyncStatus`; `SyncStatus.IO_FAILED` in Kotlin).
const IO_FAILED: i32 = -1;

/// status -> the `Int` returned to Kotlin.
fn code(st: FFISyncStatus) -> i32 {
    st as u32 as i32
}

/// handle -> the session pointer (null for 0).
fn session(handle: i64) -> *mut FFISyncSession {
    handle as *mut FFISyncSession
}

/// a `sync_new*` call -> `CoreResult<Long>` (the session handle).
fn opened<'l>(
    env: &mut Env<'l>,
    open: impl FnOnce(*mut *mut FFISyncSession) -> FFISyncStatus,
) -> Result<JObject<'l>> {
    let mut s = ptr::null_mut();
    let st = open(&mut s);
    let value = match st {
        FFISyncStatus::Ok => boxed_long(env, Some(s as i64))?,
        _ => JObject::null(),
    };
    core_result(env, code(st), &value)
}

/// a two-way sync session over the `Connected` entry `con` -> the handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_syncNew<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    con: i64,
    host: bool,
) -> JObject<'c> {
    run(&mut env, |env| opened(env, |out| unsafe { sync::sync_new(con as u64, host, out) }))
}

/// a one-way send of a selection (`what`: 0 — library, 1 — playlists by
/// name, 2 — tracks by id) with the export flags -> the handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_syncNewSend<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    con: i64,
    host: bool,
    what: i32,
    names: JObjectArray<'c, JString<'c>>,
    ids: JLongArray<'c>,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
) -> JObject<'c> {
    run(&mut env, |env| {
        let names = c_string_array(env, &names)?;
        let name_ptrs: Vec<*const c_char> = names.iter().map(|c| c.as_ptr()).collect();
        let ids = long_array(env, &ids)?;
        opened(env, |out| unsafe {
            sync::sync_new_send(
                con as u64,
                host,
                what as u32,
                name_ptrs.as_ptr(),
                name_ptrs.len(),
                ids.as_ptr(),
                ids.len(),
                with_audio,
                with_names,
                with_covers,
                out,
            )
        })
    })
}

/// a one-way receive session -> the handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_syncNewReceive<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    con: i64,
    host: bool,
    force: bool,
) -> JObject<'c> {
    run(&mut env, |env| {
        opened(env, |out| unsafe { sync::sync_new_receive(con as u64, host, force, out) })
    })
}

/// frees the session (0 is ignored).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_syncFree<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    handle: i64,
) {
    run(&mut env, |_| {
        unsafe { sync::sync_free(session(handle)) };
        Ok(())
    })
}

/// bytes received from the peer: `data[0 until len]`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_syncFeed<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    handle: i64,
    data: JByteArray<'c>,
    len: i32,
) -> i32 {
    run(&mut env, |env| {
        let bytes = match data.is_null() {
            true => Vec::new(),
            false => env.convert_byte_array(&data)?,
        };
        let len = usize::try_from(len).unwrap_or(0).min(bytes.len());
        Ok(code(unsafe { sync::sync_feed(session(handle), bytes.as_ptr(), len) }))
    })
}

/// writes everything the session has to send into `fd` (the transport) ->
/// `SyncStatus` or [`IO_FAILED`].
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_syncWriteOutput<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    handle: i64,
    fd: i32,
) -> i32 {
    run(&mut env, |_| {
        let (mut p, mut l) = (ptr::null_mut(), 0usize);
        let st = unsafe { sync::sync_take_output(session(handle), &mut p, &mut l) };
        if st != FFISyncStatus::Ok || p.is_null() {
            return Ok(code(st));
        }
        let mut out = unsafe { borrowed_file(fd) };
        let written = out
            .write_all(unsafe { std::slice::from_raw_parts(p, l) })
            .and_then(|()| out.flush());
        unsafe { mem::sync_free_bytes(p, l) };
        Ok(match written {
            Ok(()) => code(FFISyncStatus::Ok),
            Err(_) => IO_FAILED,
        })
    })
}

/// the next event -> `SyncEvent`, or null when there is none now.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_syncPollEvent<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    handle: i64,
) -> JObject<'c> {
    run(&mut env, |env| {
        let mut ev = FFISyncEvent {
            kind: FFISyncEventKind::None,
            status: 0,
            by_peer: false,
            paired_id: 0,
            songs: 0,
            playlists: 0,
        };
        let st = unsafe { sync::sync_poll_event(session(handle), &mut ev) };
        if st != FFISyncStatus::Ok || ev.kind == FFISyncEventKind::None {
            return Ok(JObject::null());
        }
        env.new_object(
            jni_str!("dev/dreamplayer/core/SyncEvent"),
            jni_sig!("(IIZJII)V"),
            &[
                JValue::Int(ev.kind as u32 as i32),
                JValue::Int(ev.status as i32),
                JValue::Bool(ev.by_peer),
                JValue::Long(ev.paired_id),
                JValue::Int(ev.songs as i32),
                JValue::Int(ev.playlists as i32),
            ],
        )
    })
}

/// `true` — the session is over (`Done` / `Failed`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_syncFinished<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    handle: i64,
) -> bool {
    run(&mut env, |_| Ok(unsafe { sync::sync_finished(session(handle)) }))
}

/// the core's view of the waiting import (`WrongState` — none).
///
/// # Safety
/// `handle` is a live session; the files stay valid until the next
/// `sync_import_*` / `sync_cancel` / `sync_free` call.
unsafe fn import_files(handle: i64) -> std::result::Result<&'static [sync::FFISyncFile], FFISyncStatus> {
    let mut files = FFISyncFiles {
        ptr: ptr::null(),
        len: 0,
    };
    let st = unsafe { sync::sync_import_files(session(handle), &mut files) };
    match (st, files.ptr.is_null()) {
        (FFISyncStatus::Ok, true) => Ok(&[]),
        (FFISyncStatus::Ok, false) => Ok(unsafe { std::slice::from_raw_parts(files.ptr, files.len) }),
        (e, _) => Err(e),
    }
}

/// the files of the waiting import (after an `Import` event) ->
/// `CoreResult<List<SyncFile>>`: kind, suggested name, size and the local file
/// to write over (null — write anywhere). The bytes go with
/// `syncImportWriteFile`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_syncImportFiles<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    handle: i64,
) -> JObject<'c> {
    run(&mut env, |env| {
        let files = match unsafe { import_files(handle) } {
            Ok(f) => f,
            Err(st) => return core_result(env, code(st), &JObject::null()),
        };
        let mut items = Vec::with_capacity(files.len());
        for f in files {
            let name = new_string(env, Some(&unsafe { c_string(f.name) }.unwrap_or_default()))?;
            let replace = new_string(env, unsafe { c_string(f.replace) }.as_deref())?;
            items.push(env.new_object(
                jni_str!("dev/dreamplayer/core/SyncFile"),
                jni_sig!("(ILjava/lang/String;JLjava/lang/String;)V"),
                &[
                    JValue::Int(f.kind as u32 as i32),
                    JValue::Object(&name),
                    JValue::Long(f.data_len as i64),
                    JValue::Object(&replace),
                ],
            )?);
        }
        let value = list(env, items)?;
        core_result(env, code(FFISyncStatus::Ok), &value)
    })
}

/// writes the bytes of import file `index` into `fd` -> `SyncStatus`
/// (`InvalidArgument` — no such file) or [`IO_FAILED`].
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_syncImportWriteFile<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    handle: i64,
    index: i32,
    fd: i32,
) -> i32 {
    run(&mut env, |_| {
        let files = match unsafe { import_files(handle) } {
            Ok(f) => f,
            Err(st) => return Ok(code(st)),
        };
        let Some(f) = usize::try_from(index).ok().and_then(|i| files.get(i)) else {
            return Ok(code(FFISyncStatus::InvalidArgument));
        };
        let data = match f.data.is_null() {
            true => &[][..],
            false => unsafe { std::slice::from_raw_parts(f.data, f.data_len) },
        };
        let mut out = unsafe { borrowed_file(fd) };
        Ok(match out.write_all(data).and_then(|()| out.flush()) {
            Ok(()) => code(FFISyncStatus::Ok),
            Err(_) => IO_FAILED,
        })
    })
}

/// the import files are written: `paths[i]` — where file `i` went (null — not
/// written). The portion is saved and the peer answered.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_syncImportDone<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    handle: i64,
    paths: JObjectArray<'c, JString<'c>>,
) -> i32 {
    run(&mut env, |env| {
        let mut owned: Vec<Option<CString>> = Vec::new();
        if !paths.is_null() {
            for i in 0..paths.len(env)? {
                let p = paths.get_element(env, i)?;
                owned.push(opt_string(env, &p)?.map(|s| cstring(&s)));
            }
        }
        let ptrs: Vec<*const c_char> = owned
            .iter()
            .map(|c| c.as_ref().map_or(ptr::null(), |c| c.as_ptr()))
            .collect();
        Ok(code(unsafe { sync::sync_import_done(session(handle), ptrs.as_ptr(), ptrs.len()) }))
    })
}

/// breaks the session off; the `Abort` for the peer goes out with the next
/// `syncWriteOutput`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_syncCancel<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    handle: i64,
) -> i32 {
    run(&mut env, |_| Ok(code(unsafe { sync::sync_cancel(session(handle)) })))
}
