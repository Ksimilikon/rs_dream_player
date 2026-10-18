//! FFI over device pairing (`vpair`). Prefix `con_` = connections.
//!
//! the core is an abstract state machine: the frontend (discovery, transport,
//! the physical devices) reports what happened, the core checks that the
//! transition is allowed and keeps the state. A refused transition returns
//! `WrongState` / `Busy`; the current state is read with `con_get_device`.
//!
//! - a device entry (`id: u64`) lives only while the core runs; a paired
//!   device also has a db id (`paired_id: i64`) that survives restarts;
//! - public keys are raw 32 bytes (X25519). The key pair of the current device
//!   is kept in `<data>/keys` and created on the first `con_*` call;
//! - the paired device list is for the user to view only and is never handed
//!   to other devices;
//! - data is encrypted only for the moment of transfer (`con_encrypt` /
//!   `con_decrypt`): any bytes, as a rule a `cdc_*` packet. What the data is,
//!   is learned from its own headers after decryption; nothing is stored
//!   encrypted;
//! - returned structs / buffers are freed by the paired `con_free_*` from
//!   [`crate::mem`].
//!
//! entry states: `Unconnected -con_connect-> Connected -con_pair-> Paired`;
//! a failed pairing -> `PairFailed` (retry with `con_pair`); `con_disconnect`
//! -> `Unconnected` from any state. Receiving / sending only in `Paired`, one
//! at a time per device; several devices work in parallel.

use std::{ffi::c_char, ptr};

use storage::Db;
use vpair::{
    CryptError, DeviceManager, KEY_LEN, PairFailure, PairedDeviceRow, PubKey, VPairError,
    VPairProcess, VPairState,
};

use crate::cdc::raw_slice;
use crate::idx::{c_to_string, to_c, vec_to_raw};
use crate::panic::Status;
use crate::{CStrError, with_out};

/// result of any `con_*` function (except `con_free_*`). `repr(u32)`: never
/// renumber, only append.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFIConStatus {
    Ok = 0,
    /// `init` hasn't succeeded yet.
    NotInitialized = 1,
    /// a required pointer (argument or out-parameter) is null.
    NullPointer = 2,
    /// a string argument is not valid UTF-8.
    InvalidUtf8 = 3,
    /// an argument is out of range (e.g. an unknown failure reason).
    InvalidArgument = 4,
    /// the core runs without a db (`need_db == false`).
    NoDatabase = 5,
    /// the key pair of the current device can't be loaded / created.
    KeysFailed = 6,
    /// no device entry with this id.
    UnknownDevice = 7,
    /// no paired device with this db id.
    UnknownPairedDevice = 8,
    /// the transition is not allowed in the current state.
    WrongState = 9,
    /// the device is already receiving or sending.
    Busy = 10,
    /// a db query / write failed.
    DbFailed = 11,
    /// the core panicked (see `core_last_panic`).
    Panic = 12,
    /// not an encrypted packet (too short) or an unknown envelope version.
    InvalidPacket = 13,
    /// a public key of low order: encryption would be predictable.
    WeakKey = 14,
    /// the packet is not from / not for this pair of devices, or it is
    /// damaged or altered.
    DecryptFailed = 15,
    /// the data is beyond the encryption limit (~256 GiB).
    TooLong = 16,
}

impl Status for FFIConStatus {
    const OK: Self = Self::Ok;
    const NULL_POINTER: Self = Self::NullPointer;
    const PANIC: Self = Self::Panic;
}

impl From<CStrError> for FFIConStatus {
    fn from(e: CStrError) -> Self {
        match e {
            CStrError::Null => Self::NullPointer,
            CStrError::InvalidUtf8 => Self::InvalidUtf8,
        }
    }
}

impl From<VPairError> for FFIConStatus {
    fn from(e: VPairError) -> Self {
        match e {
            VPairError::UnknownDevice(_) => Self::UnknownDevice,
            VPairError::UnknownPairedDevice(_) => Self::UnknownPairedDevice,
            VPairError::WrongState(_) => Self::WrongState,
            VPairError::Busy(_) => Self::Busy,
            VPairError::Db(_) => Self::DbFailed,
            VPairError::Crypt(e) => match e {
                CryptError::Malformed | CryptError::UnsupportedVersion(_) => Self::InvalidPacket,
                CryptError::WeakKey => Self::WeakKey,
                CryptError::Decrypt => Self::DecryptFailed,
                CryptError::TooLong => Self::TooLong,
            },
        }
    }
}

/// state of a device entry. `repr(u32)`: never renumber, only append.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFIConState {
    Unconnected = 0,
    Connected = 1,
    /// the devices exchanged public keys; ready to receive / send.
    Paired = 2,
    /// see [`FFIConLink::failure`].
    PairFailed = 3,
}

/// why pairing failed. `repr(u32)`: never renumber, only append.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFIPairFailure {
    /// the state is not `PairFailed`.
    None = 0,
    /// the peer or the user declined (reported by the frontend).
    Rejected = 1,
    /// no answer in time (reported by the frontend).
    Timeout = 2,
    /// transport / protocol error (reported by the frontend).
    Error = 3,
    /// the key differs from the one saved for this device (set by the core).
    KeyMismatch = 4,
    /// this device is already paired over another entry (set by the core).
    AlreadyPaired = 5,
}

/// what a paired device is busy with. `repr(u32)`: never renumber, only
/// append.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFIConProcess {
    Free = 0,
    Receiving = 1,
    Sending = 2,
}

/// state of an entry with the failure reason (`None` unless `PairFailed`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FFIConLink {
    pub state: FFIConState,
    pub failure: FFIPairFailure,
}

/// a device entry (owns `name`).
#[repr(C)]
pub struct FFIConDevice {
    pub id: u64,
    pub link: FFIConLink,
    pub process: FFIConProcess,
    pub has_paired_id: bool, // Option<i64>
    pub paired_id: i64,
    pub has_pub_key: bool, // Option<[u8; 32]>, known after pairing
    pub pub_key: [u8; KEY_LEN],
    /// user-given name (`Unknown` until known).
    pub name: *mut c_char,
}

/// list of device entries (owns the buffer).
#[repr(C)]
pub struct FFIConDevicesList {
    pub ptr: *mut FFIConDevice,
    pub len: usize,
}

/// a paired device from the db (owns `name`).
#[repr(C)]
pub struct FFIConPaired {
    pub id: i64,
    pub pub_key: [u8; KEY_LEN],
    /// user-given name, for viewing only.
    pub name: *mut c_char,
}

/// list of paired devices (owns the buffer).
#[repr(C)]
pub struct FFIConPairedList {
    pub ptr: *mut FFIConPaired,
    pub len: usize,
}

// ───────────────────────── helpers (crate-private) ─────────────────────────

/// runs `f` on the pairing state machine under the core lock.
fn with_con<R>(
    f: impl FnOnce(&mut DeviceManager, Option<&Db>) -> Result<R, FFIConStatus>,
) -> Result<R, FFIConStatus> {
    let mut core = crate::core().ok_or(FFIConStatus::NotInitialized)?;
    let (mgr, db) = core.get_vpair().map_err(|_| FFIConStatus::KeysFailed)?;
    f(mgr, db)
}

fn need_db(db: Option<&Db>) -> Result<&Db, FFIConStatus> {
    db.ok_or(FFIConStatus::NoDatabase)
}

fn status<R>(res: Result<R, FFIConStatus>) -> FFIConStatus {
    res.err().unwrap_or(FFIConStatus::Ok)
}

/// a transition that needs no db -> status.
fn run(f: impl FnOnce(&mut DeviceManager) -> Result<(), VPairError>) -> FFIConStatus {
    status(with_con(|mgr, _| Ok(f(mgr)?)))
}

/// a byte buffer into `out_ptr`/`out_len` (null/0 on error).
///
/// # Safety
/// `out_ptr`/`out_len` are null or valid pointers.
unsafe fn out_bytes(
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
    f: impl FnOnce() -> Result<Vec<u8>, FFIConStatus>,
) -> FFIConStatus {
    if out_len.is_null() {
        return FFIConStatus::NullPointer;
    }
    let mut len = 0;
    let st = unsafe {
        with_out(out_ptr, ptr::null_mut(), || {
            let (p, l) = vec_to_raw(f()?);
            len = l;
            Ok(p)
        })
    };
    unsafe { out_len.write(len) };
    st
}

fn link_to_ffi(state: VPairState) -> FFIConLink {
    let (state, failure) = match state {
        VPairState::Unconnected => (FFIConState::Unconnected, FFIPairFailure::None),
        VPairState::Connected => (FFIConState::Connected, FFIPairFailure::None),
        VPairState::Paired => (FFIConState::Paired, FFIPairFailure::None),
        VPairState::PairFailed(f) => (
            FFIConState::PairFailed,
            match f {
                PairFailure::Rejected => FFIPairFailure::Rejected,
                PairFailure::Timeout => FFIPairFailure::Timeout,
                PairFailure::Error => FFIPairFailure::Error,
                PairFailure::KeyMismatch => FFIPairFailure::KeyMismatch,
                PairFailure::AlreadyPaired => FFIPairFailure::AlreadyPaired,
            },
        ),
    };
    FFIConLink { state, failure }
}

const EMPTY_LINK: FFIConLink = FFIConLink {
    state: FFIConState::Unconnected,
    failure: FFIPairFailure::None,
};

fn empty_device() -> FFIConDevice {
    FFIConDevice {
        id: 0,
        link: EMPTY_LINK,
        process: FFIConProcess::Free,
        has_paired_id: false,
        paired_id: 0,
        has_pub_key: false,
        pub_key: [0; KEY_LEN],
        name: ptr::null_mut(),
    }
}

fn device_to_ffi(id: u64, dev: &vpair::Device) -> FFIConDevice {
    let (has_paired_id, paired_id) = dev.paired_id().map_or((false, 0), |i| (true, i));
    let (has_pub_key, pub_key) = dev
        .pub_crypt()
        .map_or((false, [0; KEY_LEN]), |k| (true, *k));
    FFIConDevice {
        id,
        link: link_to_ffi(dev.state()),
        process: match dev.process() {
            VPairProcess::Free => FFIConProcess::Free,
            VPairProcess::Receiving => FFIConProcess::Receiving,
            VPairProcess::Sending => FFIConProcess::Sending,
        },
        has_paired_id,
        paired_id,
        has_pub_key,
        pub_key,
        name: to_c(dev.name()),
    }
}

fn paired_to_ffi(row: &PairedDeviceRow) -> FFIConPaired {
    FFIConPaired {
        id: row.id,
        pub_key: row.pub_key,
        name: to_c(&row.name),
    }
}

// ───────────────────────────────── API ─────────────────────────────────────

/// public key of the current device into `out` (32 bytes) — handed to a peer
/// on pairing.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn con_get_pub_key(out: *mut [u8; KEY_LEN]) -> FFIConStatus {
    crate::panic::guard(|| unsafe {
        with_out(out, [0; KEY_LEN], || {
            with_con(|mgr, _| Ok(*mgr.pub_crypt()))
        })
    })
}

/// a new entry for an unknown device (`Unconnected`); its id into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn con_add(out: *mut u64) -> FFIConStatus {
    crate::panic::guard(|| unsafe { with_out(out, 0, || with_con(|mgr, _| Ok(mgr.add()))) })
}

/// a new entry for an already paired device (`Unconnected`); its id into
/// `out`. On `con_pair` the peer key is checked against the saved one.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn con_add_paired(paired_id: i64, out: *mut u64) -> FFIConStatus {
    crate::panic::guard(|| unsafe {
        with_out(out, 0, || {
            with_con(|mgr, db| Ok(mgr.add_paired(need_db(db)?, paired_id)?))
        })
    })
}

/// drops an entry in any state.
#[unsafe(no_mangle)]
pub extern "C" fn con_remove(id: u64) -> FFIConStatus {
    crate::panic::guard(|| {
        run(|mgr| {
            mgr.remove(id)
                .map(drop)
                .ok_or(VPairError::UnknownDevice(id))
        })
    })
}

/// the transport is up: `Unconnected` -> `Connected`.
#[unsafe(no_mangle)]
pub extern "C" fn con_connect(id: u64) -> FFIConStatus {
    crate::panic::guard(|| run(|mgr| mgr.connect(id)))
}

/// the transport is down: any -> `Unconnected`, a running transfer is
/// dropped. The entry keeps the peer key for the next pairing.
#[unsafe(no_mangle)]
pub extern "C" fn con_disconnect(id: u64) -> FFIConStatus {
    crate::panic::guard(|| run(|mgr| mgr.disconnect(id)))
}

/// the devices exchanged public keys (`peer_key` — 32 bytes of the peer):
/// `Connected` / `PairFailed` -> `Paired`, or `PairFailed` with
/// `KeyMismatch` / `AlreadyPaired`. A new device is saved to the db. The new
/// state into `out`.
///
/// # Safety
/// `peer_key` points to 32 readable bytes; `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn con_pair(
    id: u64,
    peer_key: *const [u8; KEY_LEN],
    out: *mut FFIConLink,
) -> FFIConStatus {
    crate::panic::guard(|| unsafe {
        with_out(out, EMPTY_LINK, || {
            if peer_key.is_null() {
                return Err(FFIConStatus::NullPointer);
            }
            let peer: PubKey = *peer_key;
            with_con(|mgr, db| Ok(link_to_ffi(mgr.pair(need_db(db)?, id, &peer)?)))
        })
    })
}

/// the frontend reports a failed pairing: `Connected` / `PairFailed` ->
/// `PairFailed(reason)`. `reason` — `FFIPairFailure::Rejected` / `Timeout` /
/// `Error`; the rest are set by the core only (`InvalidArgument`).
#[unsafe(no_mangle)]
pub extern "C" fn con_fail_pairing(id: u64, reason: u32) -> FFIConStatus {
    crate::panic::guard(|| {
        let reason = match reason {
            r if r == FFIPairFailure::Rejected as u32 => PairFailure::Rejected,
            r if r == FFIPairFailure::Timeout as u32 => PairFailure::Timeout,
            r if r == FFIPairFailure::Error as u32 => PairFailure::Error,
            _ => return FFIConStatus::InvalidArgument,
        };
        run(|mgr| mgr.fail_pairing(id, reason))
    })
}

/// `Free` -> `Receiving`; only for a `Paired` entry.
#[unsafe(no_mangle)]
pub extern "C" fn con_start_receiving(id: u64) -> FFIConStatus {
    crate::panic::guard(|| run(|mgr| mgr.start_receiving(id)))
}

/// `Free` -> `Sending`; only for a `Paired` entry.
#[unsafe(no_mangle)]
pub extern "C" fn con_start_sending(id: u64) -> FFIConStatus {
    crate::panic::guard(|| run(|mgr| mgr.start_sending(id)))
}

/// `Receiving` / `Sending` -> `Free`.
#[unsafe(no_mangle)]
pub extern "C" fn con_finish(id: u64) -> FFIConStatus {
    crate::panic::guard(|| run(|mgr| mgr.finish(id)))
}

/// one entry into `out`. Free with `con_free_device`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn con_get_device(id: u64, out: *mut FFIConDevice) -> FFIConStatus {
    crate::panic::guard(|| unsafe {
        with_out(out, empty_device(), || {
            with_con(|mgr, _| {
                mgr.device(id)
                    .map(|dev| device_to_ffi(id, dev))
                    .ok_or(FFIConStatus::UnknownDevice)
            })
        })
    })
}

/// every entry into `out`, ordered by id. Free with
/// `con_free_devices_list`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn con_get_devices(out: *mut FFIConDevicesList) -> FFIConStatus {
    crate::panic::guard(|| {
        let empty = FFIConDevicesList {
            ptr: ptr::null_mut(),
            len: 0,
        };
        unsafe {
            with_out(out, empty, || {
                with_con(|mgr, _| {
                    let mut items: Vec<FFIConDevice> = mgr
                        .devices()
                        .map(|(id, dev)| device_to_ffi(id, dev))
                        .collect();
                    items.sort_by_key(|d| d.id);
                    let (ptr, len) = vec_to_raw(items);
                    Ok(FFIConDevicesList { ptr, len })
                })
            })
        }
    })
}

/// encrypts `data` (any bytes, as a rule a `cdc_*` packet) for the device of
/// a `Paired` entry, right before sending it. The packet into
/// `out_ptr`/`out_len` (`data_len + 49` bytes); free it with
/// `con_free_bytes`. A null `data` is allowed only with `data_len == 0`.
///
/// # Safety
/// `data` points to `data_len` readable bytes; `out_ptr`/`out_len` are valid
/// pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn con_encrypt(
    id: u64,
    data: *const u8,
    data_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> FFIConStatus {
    crate::panic::guard(|| unsafe {
        out_bytes(out_ptr, out_len, || {
            let data = raw_slice(data, data_len).ok_or(FFIConStatus::NullPointer)?;
            with_con(|mgr, _| Ok(mgr.encrypt(id, &[], data)?))
        })
    })
}

/// decrypts a packet received from the device of a `Paired` entry. The plain
/// data into `out_ptr`/`out_len`; free it with `con_free_bytes`.
/// `DecryptFailed` — the packet is not from this device / not for this one,
/// or it is damaged.
///
/// # Safety
/// `packet` points to `packet_len` readable bytes; `out_ptr`/`out_len` are
/// valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn con_decrypt(
    id: u64,
    packet: *const u8,
    packet_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> FFIConStatus {
    crate::panic::guard(|| unsafe {
        out_bytes(out_ptr, out_len, || {
            let packet = raw_slice(packet, packet_len).ok_or(FFIConStatus::NullPointer)?;
            with_con(|mgr, _| Ok(mgr.decrypt(id, &[], packet)?))
        })
    })
}

/// every device this one has paired with (from the db) into `out` — for the
/// user to view. Free with `con_free_paired_list`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn con_get_paired(out: *mut FFIConPairedList) -> FFIConStatus {
    crate::panic::guard(|| {
        let empty = FFIConPairedList {
            ptr: ptr::null_mut(),
            len: 0,
        };
        unsafe {
            with_out(out, empty, || {
                with_con(|mgr, db| {
                    let rows = mgr.paired_devices(need_db(db)?)?;
                    let (ptr, len) = vec_to_raw(rows.iter().map(paired_to_ffi).collect());
                    Ok(FFIConPairedList { ptr, len })
                })
            })
        }
    })
}

/// the user names a paired device; the name is for viewing only.
///
/// # Safety
/// `name` is a valid nul-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn con_rename(paired_id: i64, name: *const c_char) -> FFIConStatus {
    crate::panic::guard(|| {
        let name = match unsafe { c_to_string(name) } {
            Ok(name) => name,
            Err(e) => return e.into(),
        };
        status(with_con(|mgr, db| {
            Ok(mgr.rename(need_db(db)?, paired_id, &name)?)
        }))
    })
}

/// forgets a paired device (removes it from the db); the next pairing with
/// it counts as a new one. `WrongState` while it is `Paired` over some entry.
#[unsafe(no_mangle)]
pub extern "C" fn con_forget(paired_id: i64) -> FFIConStatus {
    crate::panic::guard(|| status(with_con(|mgr, db| Ok(mgr.forget(need_db(db)?, paired_id)?))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_before_init_return_not_initialized() {
        let mut id = 7u64;
        assert_eq!(unsafe { con_add(&mut id) }, FFIConStatus::NotInitialized);
        assert_eq!(id, 0, "the out-parameter is reset on error");

        let mut key = [1u8; KEY_LEN];
        assert_eq!(
            unsafe { con_get_pub_key(&mut key) },
            FFIConStatus::NotInitialized
        );
        assert_eq!(key, [0; KEY_LEN]);

        assert_eq!(con_connect(0), FFIConStatus::NotInitialized);
        assert_eq!(con_forget(1), FFIConStatus::NotInitialized);

        let mut link = FFIConLink {
            state: FFIConState::Paired,
            failure: FFIPairFailure::KeyMismatch,
        };
        assert_eq!(
            unsafe { con_pair(0, &[2; KEY_LEN], &mut link) },
            FFIConStatus::NotInitialized
        );
        assert_eq!(link, EMPTY_LINK);
    }

    #[test]
    fn crypt_before_init_resets_outputs() {
        let (mut p, mut l) = (ptr::dangling_mut(), 5);
        assert_eq!(
            unsafe { con_encrypt(0, b"x".as_ptr(), 1, &mut p, &mut l) },
            FFIConStatus::NotInitialized
        );
        assert!(p.is_null() && l == 0, "the out-parameters are reset");

        let (mut p, mut l) = (ptr::dangling_mut(), 5);
        assert_eq!(
            unsafe { con_decrypt(0, ptr::null(), 3, &mut p, &mut l) },
            FFIConStatus::NullPointer
        );
        assert!(p.is_null() && l == 0);
        assert_eq!(
            unsafe { con_encrypt(0, ptr::null(), 0, ptr::null_mut(), &mut l) },
            FFIConStatus::NullPointer
        );
    }

    #[test]
    fn crypt_errors_map_to_ffi() {
        let st = |e| FFIConStatus::from(VPairError::Crypt(e));
        assert_eq!(st(CryptError::Malformed), FFIConStatus::InvalidPacket);
        assert_eq!(
            st(CryptError::UnsupportedVersion(2)),
            FFIConStatus::InvalidPacket
        );
        assert_eq!(st(CryptError::Decrypt), FFIConStatus::DecryptFailed);
    }

    #[test]
    fn bad_arguments_are_reported() {
        assert_eq!(
            unsafe { con_add(ptr::null_mut()) },
            FFIConStatus::NullPointer
        );
        let mut link = EMPTY_LINK;
        assert_eq!(
            unsafe { con_pair(0, ptr::null(), &mut link) },
            FFIConStatus::NullPointer
        );
        assert_eq!(
            unsafe { con_rename(1, ptr::null()) },
            FFIConStatus::NullPointer
        );
        // core-only reasons and unknown numbers are refused.
        assert_eq!(
            con_fail_pairing(0, FFIPairFailure::KeyMismatch as u32),
            FFIConStatus::InvalidArgument
        );
        assert_eq!(con_fail_pairing(0, 99), FFIConStatus::InvalidArgument);
    }

    #[test]
    fn failure_reasons_map_to_ffi() {
        assert_eq!(
            link_to_ffi(VPairState::Paired),
            FFIConLink {
                state: FFIConState::Paired,
                failure: FFIPairFailure::None,
            }
        );
        assert_eq!(
            link_to_ffi(VPairState::PairFailed(PairFailure::AlreadyPaired)),
            FFIConLink {
                state: FFIConState::PairFailed,
                failure: FFIPairFailure::AlreadyPaired,
            }
        );
    }
}
