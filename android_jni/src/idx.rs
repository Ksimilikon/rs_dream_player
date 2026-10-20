//! the index (`idx_*`): library reads and edits. Statuses are
//! `api::idx::FFIIdxStatus` codes as `Int`; reads return
//! `CoreResult(status, value)`.

#![allow(non_snake_case)]

use std::ffi::{CString, c_char};
use std::ptr;

use api::idx::{self, FFIIdxStatus};
use api::mem;
use jni::{
    Env, EnvUnowned, JValue,
    errors::Result,
    jni_sig, jni_str,
    objects::{JClass, JLongArray, JObject, JObjectArray, JString},
};

use crate::util::{
    boxed_long, c_string, core_result, cstring, list, new_string, opt_ptr, opt_string, run,
    string_list,
};

/// status -> the `Int` returned to Kotlin.
fn code(st: FFIIdxStatus) -> i32 {
    st as u32 as i32
}

/// array of C strings owned by the core -> `Vec<String>`.
///
/// # Safety
/// `ptr` is null or valid for `len` C strings.
unsafe fn c_strings(ptr: *mut *mut c_char, len: usize) -> Vec<String> {
    if ptr.is_null() {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(ptr, len) }
        .iter()
        .filter_map(|&p| unsafe { c_string(p) })
        .collect()
}

/// `FFITrackMetadata` -> Kotlin `TrackMeta`.
fn track_meta<'l>(env: &mut Env<'l>, m: &idx::FFITrackMetadata) -> Result<JObject<'l>> {
    let p = m.has_params.then_some(&m.params);
    let id = boxed_long(env, m.has_id.then_some(m.id))?;
    let path = new_string(env, unsafe { c_string(m.path) }.as_deref())?;
    let title = new_string(env, Some(&unsafe { c_string(m.title) }.unwrap_or_default()))?;
    let artists = string_list(env, &unsafe { c_strings(m.artists, m.artists_len) })?;
    let album = new_string(env, unsafe { c_string(m.album) }.as_deref())?;
    let genres = string_list(env, &unsafe { c_strings(m.genres, m.genres_len) })?;
    let cover = new_string(env, p.and_then(|p| unsafe { c_string(p.cover_art) }).as_deref())?;
    let color = new_string(env, unsafe { c_string(m.color) }.as_deref())?;
    let label = new_string(env, unsafe { c_string(m.user_label) }.as_deref())?;
    env.new_object(
        jni_str!("dev/dreamplayer/core/TrackMeta"),
        jni_sig!(
            "(Ljava/lang/Long;Ljava/lang/String;Ljava/lang/String;Ljava/util/List;\
             Ljava/lang/String;Ljava/util/List;JIIJLjava/lang/String;\
             FZLjava/lang/String;Ljava/lang/String;Z)V"
        ),
        &[
            JValue::Object(&id),
            JValue::Object(&path),
            JValue::Object(&title),
            JValue::Object(&artists),
            JValue::Object(&album),
            JValue::Object(&genres),
            JValue::Long(p.map_or(0, |p| p.duration_s as i64)),
            JValue::Int(p.map_or(0, |p| p.sample_rate as i32)),
            JValue::Int(p.map_or(0, |p| p.bitrate as i32)),
            JValue::Long(p.map_or(0, |p| p.listen_count)),
            JValue::Object(&cover),
            JValue::Float(m.volume),
            JValue::Bool(m.invalid),
            JValue::Object(&color),
            JValue::Object(&label),
            JValue::Bool(m.ignore_orphan),
        ],
    )
}

/// an `idx_get_*(out)` track list -> `CoreResult<List<TrackMeta>>`, freeing
/// the core buffer.
fn tracks_result<'l>(
    env: &mut Env<'l>,
    get: impl FnOnce(*mut idx::FFITracksList) -> FFIIdxStatus,
) -> Result<JObject<'l>> {
    let mut l = idx::FFITracksList {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let st = get(&mut l);
    if st != FFIIdxStatus::Ok {
        return core_result(env, code(st), &JObject::null());
    }
    let metas = match l.ptr.is_null() {
        true => &[][..],
        false => unsafe { std::slice::from_raw_parts(l.ptr, l.len) },
    };
    let mut items = Vec::with_capacity(metas.len());
    let res: Result<()> = metas.iter().try_for_each(|m| {
        items.push(track_meta(env, m)?);
        Ok(())
    });
    unsafe { mem::idx_free_tracks_list(l) };
    res?;
    let items = list(env, items)?;
    core_result(env, code(st), &items)
}

/// Java `String[]` -> `Vec<CString>` (null array -> empty).
pub(crate) fn c_string_array(env: &mut Env, arr: &JObjectArray<JString>) -> Result<Vec<CString>> {
    if arr.is_null() {
        return Ok(Vec::new());
    }
    let len = arr.len(env)?;
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let s = arr.get_element(env, i)?;
        out.push(cstring(&opt_string(env, &s)?.unwrap_or_default()));
    }
    Ok(out)
}

/// `LongArray` -> `Vec<i64>` (null -> empty).
pub(crate) fn long_array(env: &mut Env, arr: &JLongArray) -> Result<Vec<i64>> {
    if arr.is_null() {
        return Ok(Vec::new());
    }
    let mut buf = vec![0i64; arr.len(env)?];
    arr.get_region(env, 0, &mut buf)?;
    Ok(buf)
}

// ───────────────────────────────── reads ─────────────────────────────────

/// every playlist -> `CoreResult<List<PlaylistInfo>>`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxGetPlaylists<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
) -> JObject<'c> {
    run(&mut env, |env| {
        let mut l = idx::FFIPlaylistsList {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let st = unsafe { idx::idx_get_playlists(&mut l) };
        if st != FFIIdxStatus::Ok {
            return core_result(env, code(st), &JObject::null());
        }
        let pls = match l.ptr.is_null() {
            true => &[][..],
            false => unsafe { std::slice::from_raw_parts(l.ptr, l.len) },
        };
        let mut items = Vec::with_capacity(pls.len());
        let res: Result<()> = pls.iter().try_for_each(|p| {
            let name = new_string(env, unsafe { c_string(p.name) }.as_deref())?;
            let ids = match p.tracks.is_null() {
                true => &[][..],
                false => unsafe { std::slice::from_raw_parts(p.tracks, p.tracks_len) },
            };
            let track_ids = JLongArray::new(env, ids.len())?;
            track_ids.set_region(env, 0, ids)?;
            let track_ids: JObject = track_ids.into();
            let cover = new_string(env, unsafe { c_string(p.cover_art) }.as_deref())?;
            let created = boxed_long(env, p.has_created_at.then_some(p.created_at as i64))?;
            let updated = boxed_long(env, p.has_updated_at.then_some(p.updated_at as i64))?;
            items.push(env.new_object(
                jni_str!("dev/dreamplayer/core/PlaylistInfo"),
                jni_sig!(
                    "(Ljava/lang/String;[JLjava/lang/String;Ljava/lang/Long;Ljava/lang/Long;)V"
                ),
                &[
                    JValue::Object(&name),
                    JValue::Object(&track_ids),
                    JValue::Object(&cover),
                    JValue::Object(&created),
                    JValue::Object(&updated),
                ],
            )?);
            Ok(())
        });
        unsafe { mem::idx_free_playlists_list(l) };
        res?;
        let items = list(env, items)?;
        core_result(env, code(st), &items)
    })
}

/// the whole track pool -> `CoreResult<List<TrackMeta>>`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxGetTracks<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
) -> JObject<'c> {
    run(&mut env, |env| tracks_result(env, |out| unsafe { idx::idx_get_tracks(out) }))
}

/// songs that are in no playlist (`filter` — `api::idx::FFIOrphanFilter`)
/// -> `CoreResult<List<TrackMeta>>`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxGetOrphanTracks<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    filter: i32,
) -> JObject<'c> {
    run(&mut env, |env| {
        tracks_result(env, |out| unsafe { idx::idx_get_orphan_tracks(filter as u32, out) })
    })
}

/// the mark "ignore orphanhood" of a track.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxSetTrackIgnoreOrphan<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    id: i64,
    ignore: bool,
) -> i32 {
    run(&mut env, |_| Ok(code(idx::idx_set_track_ignore_orphan(id, ignore))))
}

/// one track by id or path (the path wins if not null) ->
/// `CoreResult<TrackMeta>` (`NOT_FOUND` — no such track).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxGetTrack<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    id: i64,
    path: JString<'c>,
) -> JObject<'c> {
    run(&mut env, |env| {
        let path = opt_string(env, &path)?.map(|p| cstring(&p));
        let mut m = std::mem::MaybeUninit::uninit();
        let st = unsafe { idx::idx_get_track(id, opt_ptr(&path), m.as_mut_ptr()) };
        // the core always fills a non-null out-parameter (empty on error).
        let m = unsafe { m.assume_init() };
        let res = match st {
            FFIIdxStatus::Ok => track_meta(env, &m),
            _ => Ok(JObject::null()),
        };
        unsafe { mem::idx_free_track_metadata(m) };
        core_result(env, code(st), &res?)
    })
}

// ───────────────────────────────── writes ────────────────────────────────

/// indexes a directory (null -> the music directory from `init`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxIndexDir<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    dir: JString<'c>,
) -> i32 {
    run(&mut env, |env| {
        let dir = opt_string(env, &dir)?.map(|d| cstring(&d));
        Ok(code(unsafe { idx::idx_index_dir(opt_ptr(&dir)) }))
    })
}

/// title and/or artists (null -> leave as is; artists are rewritten entirely).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxSetTrackMeta<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    id: i64,
    title: JString<'c>,
    artists: JObjectArray<'c, JString<'c>>,
) -> i32 {
    run(&mut env, |env| {
        let title = opt_string(env, &title)?.map(|t| cstring(&t));
        let artists = match artists.is_null() {
            true => None,
            false => Some(c_string_array(env, &artists)?),
        };
        let ptrs: Option<Vec<*const c_char>> =
            artists.as_ref().map(|a| a.iter().map(|c| c.as_ptr()).collect());
        let (aptr, alen) = match &ptrs {
            Some(p) => (p.as_ptr(), p.len()),
            None => (ptr::null(), 0),
        };
        Ok(code(unsafe { idx::idx_set_track_meta(id, opt_ptr(&title), aptr, alen) }))
    })
}

/// genres (rewritten entirely; empty / null -> cleared).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxSetTrackGenres<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    id: i64,
    genres: JObjectArray<'c, JString<'c>>,
) -> i32 {
    run(&mut env, |env| {
        let genres = c_string_array(env, &genres)?;
        let ptrs: Vec<*const c_char> = genres.iter().map(|c| c.as_ptr()).collect();
        Ok(code(unsafe { idx::idx_set_track_genres(id, ptrs.as_ptr(), ptrs.len()) }))
    })
}

/// `(id, String?) -> Int` setters (null clears the field).
macro_rules! string_setter {
    ($($name:ident => $call:path;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name<'c>(
            mut env: EnvUnowned<'c>,
            _class: JClass<'c>,
            id: i64,
            value: JString<'c>,
        ) -> i32 {
            run(&mut env, |env| {
                let value = opt_string(env, &value)?.map(|v| cstring(&v));
                Ok(code(unsafe { $call(id, opt_ptr(&value)) }))
            })
        }
    )*};
}

string_setter! {
    Java_dev_dreamplayer_core_Native_idxSetTrackAlbum => idx::idx_set_track_album;
    Java_dev_dreamplayer_core_Native_idxSetTrackCover => idx::idx_set_track_cover;
    Java_dev_dreamplayer_core_Native_idxSetTrackColor => idx::idx_set_track_color;
    Java_dev_dreamplayer_core_Native_idxSetTrackLabel => idx::idx_set_track_label;
}

/// new file path of a track (after a rename on disk).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxSetTrackPath<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    id: i64,
    path: JString<'c>,
) -> i32 {
    run(&mut env, |env| {
        let path = opt_string(env, &path)?.map(|p| cstring(&p));
        Ok(code(unsafe { idx::idx_set_track_path(id, opt_ptr(&path)) }))
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxSetTrackInvalid<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    id: i64,
    invalid: bool,
) -> i32 {
    run(&mut env, |_| Ok(code(idx::idx_set_track_invalid(id, invalid))))
}

/// the invalid mark of several tracks in one transaction.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxSetTracksInvalid<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    ids: JLongArray<'c>,
    invalid: bool,
) -> i32 {
    run(&mut env, |env| {
        let ids = long_array(env, &ids)?;
        Ok(code(unsafe { idx::idx_set_tracks_invalid(ids.as_ptr(), ids.len(), invalid) }))
    })
}

/// adds song files in one transaction -> `CoreResult<Long>` (the number written).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxAddTracks<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    paths: JObjectArray<'c, JString<'c>>,
) -> JObject<'c> {
    run(&mut env, |env| {
        let paths = c_string_array(env, &paths)?;
        let ptrs: Vec<*const c_char> = paths.iter().map(|c| c.as_ptr()).collect();
        let mut n = 0u64;
        let st = unsafe { idx::idx_add_tracks(ptrs.as_ptr(), ptrs.len(), &mut n) };
        let value = match st {
            FFIIdxStatus::Ok => boxed_long(env, Some(n as i64))?,
            _ => JObject::null(),
        };
        core_result(env, code(st), &value)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxSetTrackVolume<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    id: i64,
    volume: f32,
) -> i32 {
    run(&mut env, |_| Ok(code(idx::idx_set_track_volume(id, volume))))
}

/// dates a change of the track file made by the frontend (sync resends it).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxSetTrackFileChanged<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    id: i64,
) -> i32 {
    run(&mut env, |_| Ok(code(idx::idx_set_track_file_changed(id))))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxRemoveTrack<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    id: i64,
) -> i32 {
    run(&mut env, |_| Ok(code(idx::idx_remove_track(id))))
}

/// removes every invalid track -> `CoreResult<Long>` (the number removed).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxRemoveInvalidTracks<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
) -> JObject<'c> {
    run(&mut env, |env| {
        let mut n = 0u64;
        let st = unsafe { idx::idx_remove_invalid_tracks(&mut n) };
        let value = match st {
            FFIIdxStatus::Ok => boxed_long(env, Some(n as i64))?,
            _ => JObject::null(),
        };
        core_result(env, code(st), &value)
    })
}

/// creates / updates playlist `name` from track ids (in this order).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxSavePlaylist<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    name: JString<'c>,
    track_ids: JLongArray<'c>,
    cover: JString<'c>,
) -> i32 {
    run(&mut env, |env| {
        let name = opt_string(env, &name)?.map(|n| cstring(&n));
        let ids = long_array(env, &track_ids)?;
        let cover = opt_string(env, &cover)?.map(|c| cstring(&c));
        Ok(code(unsafe {
            idx::idx_save_playlist(opt_ptr(&name), ids.as_ptr(), ids.len(), opt_ptr(&cover))
        }))
    })
}

/// cover of playlist `name` without touching its tracks (null clears it).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxSetPlaylistCover<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    name: JString<'c>,
    cover: JString<'c>,
) -> i32 {
    run(&mut env, |env| {
        let name = opt_string(env, &name)?.map(|n| cstring(&n));
        let cover = opt_string(env, &cover)?.map(|c| cstring(&c));
        Ok(code(unsafe { idx::idx_set_playlist_cover(opt_ptr(&name), opt_ptr(&cover)) }))
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_idxRemovePlaylist<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    name: JString<'c>,
) -> i32 {
    run(&mut env, |env| {
        let name = opt_string(env, &name)?.map(|n| cstring(&n));
        Ok(code(unsafe { idx::idx_remove_playlist(opt_ptr(&name)) }))
    })
}
