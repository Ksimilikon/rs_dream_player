//! virtual pairing of devices: an abstract state machine that coordinates the
//! pairing / transfer layer living outside the core (discovery, transport, the
//! physical devices themselves). The external layer reports what happened,
//! the core checks that the transition is allowed and keeps the state; on a
//! refused transition the external layer acts depending on the current state.
//!
//! - keys of the current device are kept in files, see [`keys`];
//! - data is encrypted only for the moment of transfer to / from a paired
//!   device, see [`crypt`];
//! - public keys of paired devices are kept in the db (`paired_devices`)
//!   together with a name the user gives them — for viewing only. The list
//!   lives only on the current device and is never handed to other devices.
//!
//! states of a device entry:
//! ```text
//! Unconnected --connect--> Connected --pair--> Paired
//! Connected / PairFailed --pair / fail_pairing--> PairFailed (retry by pair)
//! any --disconnect--> Unconnected
//! ```
//! work over the protocol (receiving / sending) is allowed only in `Paired`.
//! A device either receives or sends at a time; several devices work in
//! parallel.

pub mod crypt;
pub mod keys;

use std::{collections::HashMap, error::Error, fmt, io, path::Path};

use storage::Db;
use x25519_dalek::StaticSecret;

pub use crypt::CryptError;
pub use keys::{KEY_LEN, PubKey};
pub use storage::db::PairedDeviceRow;

/// name of a device until the user gives it one (matches the db default).
pub const DEFAULT_NAME: &str = "Unknown";

/// runtime id of a device entry in [`DeviceManager`]; not the db id.
pub type DeviceId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VPairState {
    /// Unconnected device
    Unconnected,
    /// success connect with a device, keys are not exchanged yet
    Connected,
    /// success pairing: the devices exchanged their public keys; ready for
    /// give/get data
    Paired,
    /// pairing failed; the external layer may retry `pair` or disconnect
    PairFailed(PairFailure),
}

/// why pairing failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairFailure {
    /// the peer or the user declined (reported by the external layer)
    Rejected,
    /// no answer in time (reported by the external layer)
    Timeout,
    /// transport / protocol error (reported by the external layer)
    Error,
    /// the key differs from the one saved for this device (set by the core)
    KeyMismatch,
    /// this device is already paired over another entry (set by the core)
    AlreadyPaired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VPairProcess {
    /// device ready for receiving or sending
    Free,
    Receiving,
    Sending,
}

pub struct Device {
    /// user-given name, [`DEFAULT_NAME`] until known
    name: String,
    state: VPairState,
    process: VPairProcess,
    /// public key of the peer, known after pairing
    pub_crypt: Option<PubKey>,
    /// db id of the paired device; the key is checked against it on pairing
    paired_id: Option<i64>,
}

impl Device {
    fn new(paired_id: Option<i64>, name: String) -> Self {
        Self {
            name,
            state: VPairState::Unconnected,
            process: VPairProcess::Free,
            pub_crypt: None,
            paired_id,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn state(&self) -> VPairState {
        self.state
    }
    pub fn process(&self) -> VPairProcess {
        self.process
    }
    pub fn pub_crypt(&self) -> Option<&PubKey> {
        self.pub_crypt.as_ref()
    }
    pub fn paired_id(&self) -> Option<i64> {
        self.paired_id
    }
}

#[derive(Debug)]
pub enum VPairError {
    /// no entry with this id
    UnknownDevice(DeviceId),
    /// no paired device with this db id
    UnknownPairedDevice(i64),
    /// the transition is not allowed in the current state
    WrongState(VPairState),
    /// the device is already receiving or sending
    Busy(VPairProcess),
    Db(Box<dyn Error>),
    Crypt(CryptError),
}

impl fmt::Display for VPairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VPairError::UnknownDevice(id) => write!(f, "no device entry {id}"),
            VPairError::UnknownPairedDevice(id) => write!(f, "no paired device {id}"),
            VPairError::WrongState(s) => write!(f, "not allowed in state {s:?}"),
            VPairError::Busy(p) => write!(f, "device is busy: {p:?}"),
            VPairError::Db(e) => write!(f, "db error: {e}"),
            VPairError::Crypt(e) => write!(f, "{e}"),
        }
    }
}

impl Error for VPairError {}

pub struct DeviceManager {
    devices: HashMap<DeviceId, Device>,
    next_id: DeviceId,
    /// public key of the current device, handed to peers on pairing
    pub_crypt: PubKey,
    private_crypt: StaticSecret,
}

impl DeviceManager {
    /// loads the key pair of the current device from `keys_dir`, creating it
    /// on the first start (see [`keys`]).
    pub fn new(keys_dir: &Path) -> io::Result<Self> {
        let (pub_crypt, private_crypt) = keys::load_or_create(keys_dir)?;
        Ok(Self {
            devices: HashMap::new(),
            next_id: 0,
            pub_crypt,
            private_crypt,
        })
    }

    pub fn pub_crypt(&self) -> &PubKey {
        &self.pub_crypt
    }

    pub fn device(&self, id: DeviceId) -> Option<&Device> {
        self.devices.get(&id)
    }

    pub fn devices(&self) -> impl Iterator<Item = (DeviceId, &Device)> {
        self.devices.iter().map(|(&id, d)| (id, d))
    }

    /// a new entry for an unknown device, in `Unconnected`.
    pub fn add(&mut self) -> DeviceId {
        self.insert(Device::new(None, DEFAULT_NAME.into()))
    }

    /// a new entry for an already paired device, in `Unconnected`; on pairing
    /// its key is checked against the saved one.
    pub fn add_paired(&mut self, db: &Db, paired_id: i64) -> Result<DeviceId, VPairError> {
        let row = db
            .get_paired_device(paired_id)
            .map_err(VPairError::Db)?
            .ok_or(VPairError::UnknownPairedDevice(paired_id))?;
        Ok(self.insert(Device::new(Some(row.id), row.name)))
    }

    /// drops the entry in any state.
    pub fn remove(&mut self, id: DeviceId) -> Option<Device> {
        self.devices.remove(&id)
    }

    /// Unconnected -> Connected.
    pub fn connect(&mut self, id: DeviceId) -> Result<(), VPairError> {
        let dev = self.get_mut(id)?;
        match dev.state {
            VPairState::Unconnected => {
                dev.state = VPairState::Connected;
                Ok(())
            }
            s => Err(VPairError::WrongState(s)),
        }
    }

    /// any -> Unconnected, a running transfer is dropped. The entry keeps the
    /// peer key and db id, so the next pairing is checked against them.
    pub fn disconnect(&mut self, id: DeviceId) -> Result<(), VPairError> {
        let dev = self.get_mut(id)?;
        dev.state = VPairState::Unconnected;
        dev.process = VPairProcess::Free;
        Ok(())
    }

    /// the devices exchanged public keys: Connected / PairFailed -> Paired, or
    /// PairFailed when the key differs from the saved one or the device is
    /// already paired over another entry. A new device is saved to the db.
    /// Returns the new state; on a db error the state is left as is.
    pub fn pair(&mut self, db: &Db, id: DeviceId, peer: &PubKey) -> Result<VPairState, VPairError> {
        let dev = self.devices.get(&id).ok_or(VPairError::UnknownDevice(id))?;
        if !matches!(dev.state, VPairState::Connected | VPairState::PairFailed(_)) {
            return Err(VPairError::WrongState(dev.state));
        }

        // a known device must present the same key (a forgotten one counts as new).
        if let Some(pid) = dev.paired_id
            && let Some(row) = db.get_paired_device(pid).map_err(VPairError::Db)?
            && row.pub_key != *peer
        {
            return Ok(self.fail(id, PairFailure::KeyMismatch));
        }

        let known = db.get_paired_device_id(peer).map_err(VPairError::Db)?;
        if let Some(pid) = known
            && self.devices.iter().any(|(&other, d)| {
                other != id && d.state == VPairState::Paired && d.paired_id == Some(pid)
            })
        {
            return Ok(self.fail(id, PairFailure::AlreadyPaired));
        }

        let pid = match known {
            Some(pid) => pid,
            None => db.save_paired_device(peer).map_err(VPairError::Db)?,
        };
        let row = db
            .get_paired_device(pid)
            .map_err(VPairError::Db)?
            .ok_or(VPairError::UnknownPairedDevice(pid))?;

        let dev = self.get_mut(id)?;
        dev.state = VPairState::Paired;
        dev.pub_crypt = Some(*peer);
        dev.paired_id = Some(pid);
        dev.name = row.name;
        Ok(dev.state)
    }

    /// the external layer reports a failed pairing: Connected / PairFailed ->
    /// PairFailed(reason).
    pub fn fail_pairing(&mut self, id: DeviceId, reason: PairFailure) -> Result<(), VPairError> {
        let dev = self.get_mut(id)?;
        match dev.state {
            VPairState::Connected | VPairState::PairFailed(_) => {
                dev.state = VPairState::PairFailed(reason);
                Ok(())
            }
            s => Err(VPairError::WrongState(s)),
        }
    }

    /// Free -> Receiving; only for a paired device.
    pub fn start_receiving(&mut self, id: DeviceId) -> Result<(), VPairError> {
        self.set_process(id, VPairProcess::Receiving)
    }

    /// Free -> Sending; only for a paired device.
    pub fn start_sending(&mut self, id: DeviceId) -> Result<(), VPairError> {
        self.set_process(id, VPairProcess::Sending)
    }

    /// Receiving / Sending -> Free.
    pub fn finish(&mut self, id: DeviceId) -> Result<(), VPairError> {
        self.set_process(id, VPairProcess::Free)
    }

    /// seals `data` of any structure for the device of a `Paired` entry, right
    /// before sending it (see [`crypt`]). `aad` — open bytes bound to the
    /// packet (e.g. a frame header); empty if there are none.
    pub fn encrypt(&self, id: DeviceId, aad: &[u8], data: &[u8]) -> Result<Vec<u8>, VPairError> {
        let peer = self.paired_peer(id)?;
        crypt::seal(&self.private_crypt, peer, aad, data).map_err(VPairError::Crypt)
    }

    /// opens a packet received from the device of a `Paired` entry with the
    /// same `aad` the sender used; the result is plain data.
    pub fn decrypt(&self, id: DeviceId, aad: &[u8], packet: &[u8]) -> Result<Vec<u8>, VPairError> {
        let peer = self.paired_peer(id)?;
        crypt::open(&self.private_crypt, peer, aad, packet).map_err(VPairError::Crypt)
    }

    /// every device this one has paired with — for the user to view.
    pub fn paired_devices(&self, db: &Db) -> Result<Vec<PairedDeviceRow>, VPairError> {
        db.get_paired_devices().map_err(VPairError::Db)
    }

    /// the user names a paired device; the name is for viewing only.
    pub fn rename(&mut self, db: &Db, paired_id: i64, name: &str) -> Result<(), VPairError> {
        db.set_paired_device_name(paired_id, name)
            .map_err(VPairError::Db)?;
        for dev in self.devices.values_mut() {
            if dev.paired_id == Some(paired_id) {
                dev.name = name.into();
            }
        }
        Ok(())
    }

    /// forgets a paired device: it is removed from the db, and the next
    /// pairing with it counts as a new one. Not allowed while it is paired
    /// over some entry — disconnect it first.
    pub fn forget(&mut self, db: &Db, paired_id: i64) -> Result<(), VPairError> {
        let entries = self
            .devices
            .values()
            .filter(|d| d.paired_id == Some(paired_id));
        for dev in entries {
            if dev.state == VPairState::Paired {
                return Err(VPairError::WrongState(dev.state));
            }
        }
        db.remove_paired_device(paired_id).map_err(VPairError::Db)?;
        for dev in self.devices.values_mut() {
            if dev.paired_id == Some(paired_id) {
                dev.paired_id = None;
                dev.name = DEFAULT_NAME.into();
            }
        }
        Ok(())
    }

    fn insert(&mut self, dev: Device) -> DeviceId {
        let id = self.next_id;
        self.next_id += 1;
        self.devices.insert(id, dev);
        id
    }

    fn get_mut(&mut self, id: DeviceId) -> Result<&mut Device, VPairError> {
        self.devices
            .get_mut(&id)
            .ok_or(VPairError::UnknownDevice(id))
    }

    fn fail(&mut self, id: DeviceId, reason: PairFailure) -> VPairState {
        let state = VPairState::PairFailed(reason);
        if let Some(dev) = self.devices.get_mut(&id) {
            dev.state = state;
        }
        state
    }

    /// public key of the peer of a `Paired` entry.
    fn paired_peer(&self, id: DeviceId) -> Result<&PubKey, VPairError> {
        let dev = self.devices.get(&id).ok_or(VPairError::UnknownDevice(id))?;
        match (dev.state, &dev.pub_crypt) {
            (VPairState::Paired, Some(key)) => Ok(key),
            (state, _) => Err(VPairError::WrongState(state)),
        }
    }

    fn set_process(&mut self, id: DeviceId, to: VPairProcess) -> Result<(), VPairError> {
        let dev = self.get_mut(id)?;
        if dev.state != VPairState::Paired {
            return Err(VPairError::WrongState(dev.state));
        }
        match (dev.process, to) {
            (VPairProcess::Free, _) | (_, VPairProcess::Free) => {
                dev.process = to;
                Ok(())
            }
            (busy, _) => Err(VPairError::Busy(busy)),
        }
    }
}

#[cfg(test)]
mod tests {
    use storage::DB_FILE_NAME;
    use tempfile::{TempDir, tempdir};

    use super::*;

    fn setup() -> (TempDir, Db, DeviceManager) {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        let mgr = DeviceManager::new(&dir.path().join("keys")).unwrap();
        (dir, db, mgr)
    }

    /// an entry brought to `Connected`.
    fn connected(mgr: &mut DeviceManager) -> DeviceId {
        let id = mgr.add();
        mgr.connect(id).unwrap();
        id
    }

    #[test]
    fn pairs_a_new_device_and_saves_its_key() {
        let (_dir, db, mut mgr) = setup();
        let id = connected(&mut mgr);

        assert_eq!(mgr.pair(&db, id, &[1; 32]).unwrap(), VPairState::Paired);
        let rows = mgr.paired_devices(&db).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pub_key, [1; 32]);
        assert_eq!(rows[0].name, DEFAULT_NAME);
        assert_eq!(mgr.device(id).unwrap().paired_id(), Some(rows[0].id));
    }

    #[test]
    fn pairing_needs_a_connection() {
        let (_dir, db, mut mgr) = setup();
        let id = mgr.add();
        assert!(matches!(
            mgr.pair(&db, id, &[1; 32]),
            Err(VPairError::WrongState(VPairState::Unconnected))
        ));
    }

    #[test]
    fn known_device_with_another_key_fails() {
        let (_dir, db, mut mgr) = setup();
        let id = connected(&mut mgr);
        mgr.pair(&db, id, &[1; 32]).unwrap();
        mgr.disconnect(id).unwrap();
        mgr.connect(id).unwrap();

        assert_eq!(
            mgr.pair(&db, id, &[2; 32]).unwrap(),
            VPairState::PairFailed(PairFailure::KeyMismatch)
        );
        // the saved key is untouched, a retry with the right one succeeds.
        assert_eq!(mgr.paired_devices(&db).unwrap().len(), 1);
        assert_eq!(mgr.pair(&db, id, &[1; 32]).unwrap(), VPairState::Paired);
    }

    #[test]
    fn known_device_keeps_its_name() {
        let (_dir, db, mut mgr) = setup();
        let id = connected(&mut mgr);
        mgr.pair(&db, id, &[1; 32]).unwrap();
        let pid = mgr.device(id).unwrap().paired_id().unwrap();
        mgr.rename(&db, pid, "phone").unwrap();
        mgr.remove(id);

        let id = mgr.add_paired(&db, pid).unwrap();
        assert_eq!(mgr.device(id).unwrap().name(), "phone");
        mgr.connect(id).unwrap();
        mgr.pair(&db, id, &[1; 32]).unwrap();
        assert_eq!(mgr.device(id).unwrap().name(), "phone");
    }

    #[test]
    fn same_device_cannot_be_paired_twice() {
        let (_dir, db, mut mgr) = setup();
        let a = connected(&mut mgr);
        let b = connected(&mut mgr);
        mgr.pair(&db, a, &[1; 32]).unwrap();

        assert_eq!(
            mgr.pair(&db, b, &[1; 32]).unwrap(),
            VPairState::PairFailed(PairFailure::AlreadyPaired)
        );
    }

    #[test]
    fn external_failure_and_retry() {
        let (_dir, db, mut mgr) = setup();
        let id = connected(&mut mgr);
        mgr.fail_pairing(id, PairFailure::Timeout).unwrap();
        assert_eq!(
            mgr.device(id).unwrap().state(),
            VPairState::PairFailed(PairFailure::Timeout)
        );
        assert_eq!(mgr.pair(&db, id, &[1; 32]).unwrap(), VPairState::Paired);
        assert!(mgr.fail_pairing(id, PairFailure::Error).is_err());
    }

    #[test]
    fn transfer_only_when_paired_and_one_at_a_time() {
        let (_dir, db, mut mgr) = setup();
        let a = connected(&mut mgr);
        assert!(matches!(
            mgr.start_sending(a),
            Err(VPairError::WrongState(VPairState::Connected))
        ));

        mgr.pair(&db, a, &[1; 32]).unwrap();
        mgr.start_sending(a).unwrap();
        assert!(matches!(
            mgr.start_receiving(a),
            Err(VPairError::Busy(VPairProcess::Sending))
        ));

        // another device works in parallel.
        let b = connected(&mut mgr);
        mgr.pair(&db, b, &[2; 32]).unwrap();
        mgr.start_receiving(b).unwrap();

        mgr.finish(a).unwrap();
        mgr.start_receiving(a).unwrap();

        mgr.disconnect(a).unwrap();
        assert_eq!(mgr.device(a).unwrap().process(), VPairProcess::Free);
    }

    #[test]
    fn forget_only_when_not_paired() {
        let (_dir, db, mut mgr) = setup();
        let id = connected(&mut mgr);
        mgr.pair(&db, id, &[1; 32]).unwrap();
        let pid = mgr.device(id).unwrap().paired_id().unwrap();
        assert!(mgr.forget(&db, pid).is_err());

        mgr.disconnect(id).unwrap();
        mgr.forget(&db, pid).unwrap();
        assert!(mgr.paired_devices(&db).unwrap().is_empty());

        // after forgetting, any key pairs as a new device with a fresh id.
        mgr.connect(id).unwrap();
        assert_eq!(mgr.pair(&db, id, &[2; 32]).unwrap(), VPairState::Paired);
        assert_ne!(mgr.device(id).unwrap().paired_id(), Some(pid));
    }

    #[test]
    fn encrypts_for_the_paired_peer_only() {
        let (_da, db_a, mut a) = setup();
        let (_db, db_b, mut b) = setup();
        let (a_pub, b_pub) = (*a.pub_crypt(), *b.pub_crypt());
        let a_to_b = connected(&mut a);
        let b_to_a = connected(&mut b);
        assert!(matches!(
            a.encrypt(a_to_b, b"", b"packet"),
            Err(VPairError::WrongState(VPairState::Connected))
        ));
        a.pair(&db_a, a_to_b, &b_pub).unwrap();
        b.pair(&db_b, b_to_a, &a_pub).unwrap();

        let packet = a.encrypt(a_to_b, b"", b"codec packet").unwrap();
        assert_eq!(b.decrypt(b_to_a, b"", &packet).unwrap(), b"codec packet");
        // the sender itself can't open what it sealed for the peer.
        assert!(matches!(
            a.decrypt(a_to_b, b"", &packet),
            Err(VPairError::Crypt(CryptError::Decrypt))
        ));

        b.disconnect(b_to_a).unwrap();
        assert!(matches!(
            b.decrypt(b_to_a, b"", &packet),
            Err(VPairError::WrongState(VPairState::Unconnected))
        ));
    }
}
