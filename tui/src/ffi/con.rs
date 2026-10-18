//! safe wrapper over device pairing (`con_*`): connection entries and the
//! paired device list. Core buffers are copied into owning values and freed
//! right away with the paired `con_free_*`.

use std::ffi::{CStr, CString};
use std::ptr;

use api::con::{self, FFIConPairedList, FFIConStatus};
use api::mem;

/// a device this one has paired with (from the db).
pub struct Paired {
    pub id: i64,
    pub key: [u8; 32],
    /// user-given name, for viewing only.
    pub name: String,
}

/// human-readable description of a core status.
pub fn describe(st: FFIConStatus) -> &'static str {
    use FFIConStatus as S;
    match st {
        S::Ok => "ok",
        S::NotInitialized => "core is not initialized",
        S::NullPointer => "internal error: null pointer",
        S::InvalidUtf8 => "name is not valid UTF-8",
        S::InvalidArgument => "internal error: invalid argument",
        S::NoDatabase => "core runs without a database",
        S::KeysFailed => "failed to load / create the device keys",
        S::UnknownDevice => "no such connection",
        S::UnknownPairedDevice => "no such paired device",
        S::WrongState => "not allowed in the current state (disconnect first?)",
        S::Busy => "device is busy with a transfer",
        S::DbFailed => "database error",
        S::Panic => "core panicked (see core_panic.log in the data dir)",
        S::InvalidPacket => "not an encrypted packet",
        S::WeakKey => "weak peer key",
        S::DecryptFailed => "decryption failed",
        S::TooLong => "data too long",
    }
}

fn check(st: FFIConStatus) -> Result<(), FFIConStatus> {
    match st {
        FFIConStatus::Ok => Ok(()),
        e => Err(e),
    }
}

/// short fingerprint of a public key for the user: `ab12 cd34 ef56 7890`.
pub fn fingerprint(key: &[u8; 32]) -> String {
    key[..8]
        .chunks(2)
        .map(|c| format!("{:02x}{:02x}", c[0], c[1]))
        .collect::<Vec<_>>()
        .join(" ")
}

/// public key of this device.
pub fn pub_key() -> Result<[u8; 32], FFIConStatus> {
    let mut key = [0; 32];
    check(unsafe { con::con_get_pub_key(&mut key) })?;
    Ok(key)
}

/// a new connection entry, brought to `Connected` (the transport is up).
pub fn open() -> Result<u64, FFIConStatus> {
    let mut id = 0;
    check(unsafe { con::con_add(&mut id) })?;
    if let Err(e) = check(con::con_connect(id)) {
        con::con_remove(id);
        return Err(e);
    }
    Ok(id)
}

/// drops a connection entry (the transport is down).
pub fn close(id: u64) {
    con::con_disconnect(id);
    con::con_remove(id);
}

/// every paired device.
pub fn paired() -> Vec<Paired> {
    let mut list = FFIConPairedList {
        ptr: ptr::null_mut(),
        len: 0,
    };
    if unsafe { con::con_get_paired(&mut list) } != FFIConStatus::Ok || list.ptr.is_null() {
        return Vec::new();
    }
    let out = unsafe { std::slice::from_raw_parts(list.ptr, list.len) }
        .iter()
        .map(|p| Paired {
            id: p.id,
            key: p.pub_key,
            name: match p.name.is_null() {
                true => String::new(),
                false => unsafe { CStr::from_ptr(p.name) }
                    .to_string_lossy()
                    .into_owned(),
            },
        })
        .collect();
    unsafe { mem::con_free_paired_list(list) };
    out
}

pub fn rename(paired_id: i64, name: &str) -> Result<(), FFIConStatus> {
    let name = CString::new(name).map_err(|_| FFIConStatus::InvalidUtf8)?;
    check(unsafe { con::con_rename(paired_id, name.as_ptr()) })
}

pub fn forget(paired_id: i64) -> Result<(), FFIConStatus> {
    check(con::con_forget(paired_id))
}
