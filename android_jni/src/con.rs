//! device pairing (`con_*`): connection entries and the paired device list.
//! Statuses are `api::con::FFIConStatus` codes as `Int`; reads return
//! `CoreResult(status, value)`. Pairing itself and the encryption run inside
//! a sync session (`sync_*`), so only what the frontend drives is bound here.

#![allow(non_snake_case)]

use std::ptr;

use api::con::{self, FFIConDevice, FFIConDevicesList, FFIConPairedList, FFIConStatus};
use api::mem;
use jni::{
    Env, EnvUnowned, JValue,
    errors::Result,
    jni_sig, jni_str,
    objects::{JClass, JObject, JString},
};

use crate::util::{boxed_long, byte_array, c_string, core_result, cstring, list, new_string, opt_ptr, opt_string, run};

/// status -> the `Int` returned to Kotlin.
fn code(st: FFIConStatus) -> i32 {
    st as u32 as i32
}

/// `FFIConDevice` -> Kotlin `ConDevice`.
fn device<'l>(env: &mut Env<'l>, d: &FFIConDevice) -> Result<JObject<'l>> {
    let paired_id = boxed_long(env, d.has_paired_id.then_some(d.paired_id))?;
    let key = match d.has_pub_key {
        true => byte_array(env, &d.pub_key)?,
        false => JObject::null(),
    };
    let name = new_string(env, Some(&unsafe { c_string(d.name) }.unwrap_or_default()))?;
    env.new_object(
        jni_str!("dev/dreamplayer/core/ConDevice"),
        jni_sig!("(JIIILjava/lang/Long;[BLjava/lang/String;)V"),
        &[
            JValue::Long(d.id as i64),
            JValue::Int(d.link.state as u32 as i32),
            JValue::Int(d.link.failure as u32 as i32),
            JValue::Int(d.process as u32 as i32),
            JValue::Object(&paired_id),
            JValue::Object(&key),
            JValue::Object(&name),
        ],
    )
}

/// public key of this device -> `CoreResult<ByteArray>` (32 bytes).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_conGetPubKey<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
) -> JObject<'c> {
    run(&mut env, |env| {
        let mut key = [0u8; 32];
        let st = unsafe { con::con_get_pub_key(&mut key) };
        let value = match st {
            FFIConStatus::Ok => byte_array(env, &key)?,
            _ => JObject::null(),
        };
        core_result(env, code(st), &value)
    })
}

/// a new entry for an unknown device -> `CoreResult<Long>` (its id).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_conAdd<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
) -> JObject<'c> {
    run(&mut env, |env| {
        let mut id = 0u64;
        let st = unsafe { con::con_add(&mut id) };
        let value = match st {
            FFIConStatus::Ok => boxed_long(env, Some(id as i64))?,
            _ => JObject::null(),
        };
        core_result(env, code(st), &value)
    })
}

/// `(id) -> Int` transitions of an entry.
macro_rules! transition {
    ($($name:ident => $call:path;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name<'c>(mut env: EnvUnowned<'c>, _class: JClass<'c>, id: i64) -> i32 {
            run(&mut env, |_| Ok(code($call(id as u64))))
        }
    )*};
}

transition! {
    Java_dev_dreamplayer_core_Native_conConnect => con::con_connect;
    Java_dev_dreamplayer_core_Native_conDisconnect => con::con_disconnect;
    Java_dev_dreamplayer_core_Native_conRemove => con::con_remove;
}

/// every entry, ordered by id -> `CoreResult<List<ConDevice>>`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_conGetDevices<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
) -> JObject<'c> {
    run(&mut env, |env| {
        let mut raw = FFIConDevicesList {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let st = unsafe { con::con_get_devices(&mut raw) };
        let built = (|| {
            if st != FFIConStatus::Ok || raw.ptr.is_null() {
                return Ok(JObject::null());
            }
            let mut items = Vec::with_capacity(raw.len);
            for d in unsafe { std::slice::from_raw_parts(raw.ptr, raw.len) } {
                items.push(device(env, d)?);
            }
            list(env, items)
        })();
        // freed even if building the Kotlin objects failed.
        unsafe { mem::con_free_devices_list(raw) };
        let value = match (st, built?) {
            (FFIConStatus::Ok, v) if v.is_null() => list(env, Vec::new())?,
            (_, v) => v,
        };
        core_result(env, code(st), &value)
    })
}

/// every device this one has paired with -> `CoreResult<List<PairedDevice>>`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_conGetPaired<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
) -> JObject<'c> {
    run(&mut env, |env| {
        let mut raw = FFIConPairedList {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let st = unsafe { con::con_get_paired(&mut raw) };
        let built = (|| {
            if st != FFIConStatus::Ok {
                return Ok(JObject::null());
            }
            let rows = match raw.ptr.is_null() {
                true => &[][..],
                false => unsafe { std::slice::from_raw_parts(raw.ptr, raw.len) },
            };
            let mut items = Vec::with_capacity(rows.len());
            for p in rows {
                let key = byte_array(env, &p.pub_key)?;
                let name = new_string(env, Some(&unsafe { c_string(p.name) }.unwrap_or_default()))?;
                items.push(env.new_object(
                    jni_str!("dev/dreamplayer/core/PairedDevice"),
                    jni_sig!("(J[BLjava/lang/String;)V"),
                    &[JValue::Long(p.id), JValue::Object(&key), JValue::Object(&name)],
                )?);
            }
            list(env, items)
        })();
        unsafe { mem::con_free_paired_list(raw) };
        let value = built?;
        core_result(env, code(st), &value)
    })
}

/// the user names a paired device (for viewing only).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_conRename<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    paired_id: i64,
    name: JString<'c>,
) -> i32 {
    run(&mut env, |env| {
        let name = opt_string(env, &name)?.map(|n| cstring(&n));
        Ok(code(unsafe { con::con_rename(paired_id, opt_ptr(&name)) }))
    })
}

/// forgets a paired device.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dreamplayer_core_Native_conForget<'c>(
    mut env: EnvUnowned<'c>,
    _class: JClass<'c>,
    paired_id: i64,
) -> i32 {
    run(&mut env, |_| Ok(code(con::con_forget(paired_id))))
}
