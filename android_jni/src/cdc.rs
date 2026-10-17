//! packets (`cdc_*`): export, unpacking and the two-step import.
//!
//! packets and audio can be gigabytes, so they never go through the Java
//! heap: export writes the packet into a file descriptor, import reads the
//! packet from one and writes every file into a descriptor the frontend opened
//! (MediaStore, SAF, ...). Statuses are `api::cdc::FFICdcStatus` codes as
//! `Int`.

#![allow(non_snake_case)]

use std::ffi::{CString, c_char};
use std::io::{Read, Write};
use std::ptr;

use api::cdc::{self, FFICdcStatus, FFIImportFiles, FFIPacket, ImportSession};
use api::mem;
use jni::{
    Env, EnvUnowned, JValue,
    errors::Result,
    jni_sig, jni_str,
    objects::{JByteArray, JClass, JLongArray, JObject, JObjectArray, JString},
};

use crate::util::{borrowed_file, boxed_long, byte_array, c_string, cstring, list, new_string, opt_string, run};

/// status -> the `Int` returned to Kotlin.
fn code(st: FFICdcStatus) -> i32 {
    st as u32 as i32
}

/// runs a `cdc_pack_*` call and writes the packet into `fd`.
fn pack_to_fd(fd: i32, pack: impl FnOnce(*mut *mut u8, *mut usize) -> FFICdcStatus) -> i32 {
    let (mut p, mut l) = (ptr::null_mut(), 0usize);
    let st = pack(&mut p, &mut l);
    if st != FFICdcStatus::Ok {
        return code(st);
    }
    let bytes = match p.is_null() {
        true => &[][..],
        false => unsafe { std::slice::from_raw_parts(p, l) },
    };
    let res = unsafe { borrowed_file(fd) }.write_all(bytes);
    unsafe { mem::cdc_free_bytes(p, l) };
    match res {
        Ok(()) => code(FFICdcStatus::Ok),
        Err(_) => code(FFICdcStatus::WriteFailed),
    }
}

// ──────────────────────────────── export ────────────────────────────────

/// snapshot of the whole db into `fd`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_cdcPackLibrary<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    version: i32,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
    fd: i32,
) -> i32 {
    run(&mut env, |_| {
        Ok(pack_to_fd(fd, |p, l| unsafe {
            cdc::cdc_pack_library(version as u32, with_audio, with_names, with_covers, p, l)
        }))
    })
}

/// tracks by id (without playlists) into `fd`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_cdcPackTracks<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    version: i32,
    ids: JLongArray<'c>,
    with_audio: bool,
    with_names: bool,
    fd: i32,
) -> i32 {
    run(&mut env, |env| {
        let ids = match ids.is_null() {
            true => Vec::new(),
            false => {
                let mut buf = vec![0i64; ids.len(env)?];
                ids.get_region(env, 0, &mut buf)?;
                buf
            }
        };
        Ok(pack_to_fd(fd, |p, l| unsafe {
            cdc::cdc_pack_tracks(version as u32, ids.as_ptr(), ids.len(), with_audio, with_names, p, l)
        }))
    })
}

/// playlists by name (with all their songs) into `fd`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_cdcPackPlaylists<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    version: i32,
    names: JObjectArray<'c, JString<'c>>,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
    fd: i32,
) -> i32 {
    run(&mut env, |env| {
        let mut cs: Vec<CString> = Vec::new();
        if !names.is_null() {
            for i in 0..names.len(env)? {
                let s = names.get_element(env, i)?;
                cs.push(cstring(&opt_string(env, &s)?.unwrap_or_default()));
            }
        }
        let ptrs: Vec<*const c_char> = cs.iter().map(|c| c.as_ptr()).collect();
        Ok(pack_to_fd(fd, |p, l| unsafe {
            cdc::cdc_pack_playlists(
                version as u32,
                ptrs.as_ptr(),
                ptrs.len(),
                with_audio,
                with_names,
                with_covers,
                p,
                l,
            )
        }))
    })
}

// ──────────────────────────────── decode ────────────────────────────────

/// 16 hash bytes -> `ByteArray`.
fn hash<'l>(env: &mut Env<'l>, h: &[u8; 16]) -> Result<JObject<'l>> {
    byte_array(env, h)
}

/// `ptr, len` owned by the core -> `ByteArray?` (null pointer -> null).
fn opt_bytes<'l>(env: &mut Env<'l>, p: *const u8, len: usize) -> Result<JObject<'l>> {
    match p.is_null() {
        true => Ok(JObject::null()),
        false => byte_array(env, unsafe { std::slice::from_raw_parts(p, len) }),
    }
}

/// `FFIPacket` -> Kotlin `Packet`.
fn packet<'l>(env: &mut Env<'l>, pk: &FFIPacket) -> Result<JObject<'l>> {
    let songs = match pk.songs.is_null() {
        true => &[][..],
        false => unsafe { std::slice::from_raw_parts(pk.songs, pk.songs_len) },
    };
    let mut song_objs = Vec::with_capacity(songs.len());
    for s in songs {
        let h = hash(env, &s.hash)?;
        let name = new_string(env, unsafe { c_string(s.name) }.as_deref())?;
        let data = opt_bytes(env, s.data, s.data_len)?;
        song_objs.push(env.new_object(
            jni_str!("dev/dreamplayer/core/PacketSong"),
            jni_sig!("([BLjava/lang/String;[B)V"),
            &[JValue::Object(&h), JValue::Object(&name), JValue::Object(&data)],
        )?);
    }

    let pls = match pk.playlists.is_null() {
        true => &[][..],
        false => unsafe { std::slice::from_raw_parts(pk.playlists, pk.playlists_len) },
    };
    let mut pl_objs = Vec::with_capacity(pls.len());
    for p in pls {
        let name = new_string(env, unsafe { c_string(p.name) }.as_deref())?;
        let hashes = match p.tracks.is_null() {
            true => &[][..],
            false => unsafe { std::slice::from_raw_parts(p.tracks, p.tracks_len) },
        };
        let mut track_objs = Vec::with_capacity(hashes.len());
        for h in hashes {
            track_objs.push(hash(env, h)?);
        }
        let tracks = list(env, track_objs)?;
        let created = boxed_long(env, p.has_created_at.then_some(p.created_at as i64))?;
        let updated = boxed_long(env, p.has_updated_at.then_some(p.updated_at as i64))?;
        let cover_name = new_string(env, unsafe { c_string(p.cover_name) }.as_deref())?;
        let cover = opt_bytes(env, p.cover, p.cover_len)?;
        pl_objs.push(env.new_object(
            jni_str!("dev/dreamplayer/core/PacketPlaylist"),
            jni_sig!(
                "(Ljava/lang/String;Ljava/util/List;Ljava/lang/Long;Ljava/lang/Long;\
                 Ljava/lang/String;[B)V"
            ),
            &[
                JValue::Object(&name),
                JValue::Object(&tracks),
                JValue::Object(&created),
                JValue::Object(&updated),
                JValue::Object(&cover_name),
                JValue::Object(&cover),
            ],
        )?);
    }

    let songs = list(env, song_objs)?;
    let playlists = list(env, pl_objs)?;
    env.new_object(
        jni_str!("dev/dreamplayer/core/Packet"),
        jni_sig!("(ILjava/util/List;Ljava/util/List;)V"),
        &[
            JValue::Int(pk.version as i32),
            JValue::Object(&songs),
            JValue::Object(&playlists),
        ],
    )
}

/// unpacks a packet -> `DecodeResult(status, packet?)`. Meant for previews
/// (with `withAudio = false`): with audio everything lands in the Java heap.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_cdcDecode<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    data: JByteArray<'c>,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
) -> JObject<'c> {
    run(&mut env, |env| {
        let bytes = match data.is_null() {
            true => Vec::new(),
            false => env.convert_byte_array(&data)?,
        };
        let mut pk = FFIPacket {
            version: 0,
            songs: ptr::null_mut(),
            songs_len: 0,
            playlists: ptr::null_mut(),
            playlists_len: 0,
        };
        let st = unsafe {
            cdc::cdc_decode(bytes.as_ptr(), bytes.len(), with_audio, with_names, with_covers, &mut pk)
        };
        let obj = match st {
            FFICdcStatus::Ok => packet(env, &pk),
            _ => Ok(JObject::null()),
        };
        unsafe { mem::cdc_free_packet(pk) };
        let obj = obj?;
        env.new_object(
            jni_str!("dev/dreamplayer/core/DecodeResult"),
            jni_sig!("(ILdev/dreamplayer/core/Packet;)V"),
            &[JValue::Int(code(st)), JValue::Object(&obj)],
        )
    })
}

// ──────────────────────────────── import ────────────────────────────────

/// import session as seen by Kotlin: the core session + its file list. Passed
/// to Kotlin as an opaque `Long` handle.
struct Import {
    session: *mut ImportSession,
    files: FFIImportFiles,
}

impl Import {
    /// the core file entry `i`, if in range.
    fn file(&self, i: i32) -> Option<&cdc::FFIImportFile> {
        let i = usize::try_from(i).ok()?;
        (i < self.files.len).then(|| unsafe { &*self.files.ptr.add(i) })
    }
}

/// handle -> `Import` (`None` for 0).
///
/// # Safety
/// `handle` is 0 or a value returned by `cdcImportPrepare` and not yet passed
/// to `cdcImportCommit` / `cdcImportCancel`.
unsafe fn import_ref<'a>(handle: i64) -> Option<&'a Import> {
    (handle != 0).then(|| unsafe { &*(handle as *const Import) })
}

/// step 1: reads the packet from `fd`, checks it and the dates ->
/// `ImportPrepared(status, session, files)`. `files` holds only the names and
/// sizes; the bytes are written by `cdcImportWriteFile`. The db doesn't change.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_cdcImportPrepare<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    fd: i32,
    force: bool,
) -> JObject<'c> {
    run(&mut env, |env| {
        let mut bytes = Vec::new();
        let (st, handle, files) = match unsafe { borrowed_file(fd) }.read_to_end(&mut bytes) {
            Err(_) => (FFICdcStatus::ReadFailed, 0i64, Vec::new()),
            Ok(_) => {
                let mut session = ptr::null_mut();
                let mut files = FFIImportFiles {
                    ptr: ptr::null(),
                    len: 0,
                };
                let st = unsafe {
                    cdc::cdc_import_prepare(bytes.as_ptr(), bytes.len(), force, &mut session, &mut files)
                };
                drop(bytes);
                match st {
                    FFICdcStatus::Ok => {
                        let entries: Vec<(i32, String, i64)> = (0..files.len)
                            .map(|i| {
                                let f = unsafe { &*files.ptr.add(i) };
                                let name = unsafe { c_string(f.name) }.unwrap_or_default();
                                (f.kind as u32 as i32, name, f.data_len as i64)
                            })
                            .collect();
                        let handle = Box::into_raw(Box::new(Import { session, files })) as i64;
                        (st, handle, entries)
                    }
                    _ => (st, 0, Vec::new()),
                }
            }
        };

        let mut file_objs = Vec::with_capacity(files.len());
        for (kind, name, size) in &files {
            let name = new_string(env, Some(name))?;
            file_objs.push(env.new_object(
                jni_str!("dev/dreamplayer/core/ImportFile"),
                jni_sig!("(ILjava/lang/String;J)V"),
                &[JValue::Int(*kind), JValue::Object(&name), JValue::Long(*size)],
            )?);
        }
        let files = list(env, file_objs)?;
        env.new_object(
            jni_str!("dev/dreamplayer/core/ImportPrepared"),
            jni_sig!("(IJLjava/util/List;)V"),
            &[JValue::Int(code(st)), JValue::Long(handle), JValue::Object(&files)],
        )
    })
}

/// step 2: writes the bytes of file `index` into `fd` (opened by the frontend
/// wherever it can write). `Ok`, `WriteFailed` or `InvalidArgument` (bad
/// handle / index).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_cdcImportWriteFile<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    session: i64,
    index: i32,
    fd: i32,
) -> i32 {
    run(&mut env, |_| {
        let Some(file) = unsafe { import_ref(session) }.and_then(|imp| imp.file(index)) else {
            return Ok(code(FFICdcStatus::InvalidArgument));
        };
        let data = match file.data.is_null() {
            true => &[][..],
            false => unsafe { std::slice::from_raw_parts(file.data, file.data_len) },
        };
        Ok(match unsafe { borrowed_file(fd) }.write_all(data) {
            Ok(()) => code(FFICdcStatus::Ok),
            Err(_) => code(FFICdcStatus::WriteFailed),
        })
    })
}

/// step 3: saves the import. `paths[i]` is the final path of file `i` (null —
/// not written). The session is freed in any case; on a non-`Ok` status the
/// frontend deletes the written files itself.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_cdcImportCommit<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    session: i64,
    paths: JObjectArray<'c, JString<'c>>,
) -> i32 {
    run(&mut env, |env| {
        if session == 0 {
            return Ok(code(FFICdcStatus::NullPointer));
        }
        let imp = unsafe { Box::from_raw(session as *mut Import) };
        let mut cs: Vec<Option<CString>> = Vec::new();
        if !paths.is_null() {
            for i in 0..paths.len(env)? {
                let s = paths.get_element(env, i)?;
                cs.push(opt_string(env, &s)?.map(|p| cstring(&p)));
            }
        }
        let ptrs: Vec<*const c_char> = cs.iter().map(crate::util::opt_ptr).collect();
        Ok(code(unsafe { cdc::cdc_import_commit(imp.session, ptrs.as_ptr(), ptrs.len()) }))
    })
}

/// cancels the import and frees the session (0 is ignored).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_cdcImportCancel<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    session: i64,
) {
    run(&mut env, |_| {
        if session != 0 {
            let imp = unsafe { Box::from_raw(session as *mut Import) };
            unsafe { cdc::cdc_import_cancel(imp.session) };
        }
        Ok(())
    })
}
