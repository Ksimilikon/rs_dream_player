//! Bluetooth RFCOMM transport (Linux, BlueZ). The sync service is registered
//! as a BlueZ profile with its own UUID, so the peer finds the channel via SDP
//! (an Android client connects by this UUID). The devices are paired at the
//! OS level beforehand (`bluetoothctl`), the sync keys are exchanged on top.
//!
//! BlueZ hands the connected socket over asynchronously; it is turned into a
//! plain blocking socket so the sync loop is the same as for TCP.

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bluer::rfcomm::{Profile, ProfileHandle, Role};
use bluer::{Address, Session, Uuid};
use futures::StreamExt;

/// UUID of the dream_player sync service.
pub const SERVICE_UUID: Uuid = Uuid::from_u128(0x6f3c1a52_8d0e_4b8e_9a4b_3d2f7c51a9e1);

const POLL: Duration = Duration::from_millis(200);

/// keeps the BlueZ session and profile registered while the connection
/// lives.
pub struct Guard {
    _rt: tokio::runtime::Runtime,
    _session: Session,
    _profile: ProfileHandle,
}

fn bt_err(e: bluer::Error) -> io::Error {
    let msg = e.to_string();
    match msg.contains("ServiceUnknown") {
        true => io::Error::other("bluetooth: BlueZ (bluetoothd) is not running"),
        false => io::Error::other(format!("bluetooth: {msg}")),
    }
}

fn runtime() -> io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

fn profile(role: Role) -> Profile {
    Profile {
        uuid: SERVICE_UUID,
        name: Some("dream_player sync".into()),
        role: Some(role),
        require_authentication: Some(false),
        require_authorization: Some(false),
        ..Default::default()
    }
}

/// the accepted async stream -> a blocking socket of its own (a duplicate of
/// the descriptor; the async one is dropped).
fn blocking(stream: bluer::rfcomm::Stream) -> io::Result<UnixStream> {
    let fd = unsafe { BorrowedFd::borrow_raw(stream.as_raw_fd()) }.try_clone_to_owned()?;
    drop(stream);
    // a `UnixStream` is just a socket descriptor wrapper here: read / write /
    // timeouts work for any socket.
    let sock = UnixStream::from(fd);
    sock.set_nonblocking(false)?;
    Ok(sock)
}

/// waits for one incoming connection to the sync service; `Interrupted`
/// once `cancel` is set. Returns the socket and the peer address.
pub fn accept(cancel: &AtomicBool) -> io::Result<(UnixStream, Address, Guard)> {
    let rt = runtime()?;
    let (sock, peer, session, handle) = rt.block_on(async {
        let session = Session::new().await.map_err(bt_err)?;
        let adapter = session.default_adapter().await.map_err(bt_err)?;
        adapter.set_powered(true).await.map_err(bt_err)?;
        let mut handle = session
            .register_profile(profile(Role::Server))
            .await
            .map_err(bt_err)?;
        loop {
            match tokio::time::timeout(POLL, handle.next()).await {
                Ok(Some(req)) => {
                    let peer = req.device();
                    let sock = blocking(req.accept().map_err(bt_err)?)?;
                    return Ok::<_, io::Error>((sock, peer, session, handle));
                }
                Ok(None) => return Err(io::Error::other("bluetooth: profile unregistered")),
                Err(_) if cancel.load(Ordering::Relaxed) => {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                Err(_) => {}
            }
        }
    })?;
    Ok((
        sock,
        peer,
        Guard {
            _rt: rt,
            _session: session,
            _profile: handle,
        },
    ))
}

/// connects to the sync service of the device `addr`
/// (`AA:BB:CC:DD:EE:FF`).
pub fn connect(addr: &str) -> io::Result<(UnixStream, Guard)> {
    let addr: Address = addr
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad bluetooth address"))?;
    let rt = runtime()?;
    let (sock, session, handle) = rt.block_on(async {
        let session = Session::new().await.map_err(bt_err)?;
        let adapter = session.default_adapter().await.map_err(bt_err)?;
        adapter.set_powered(true).await.map_err(bt_err)?;
        let mut handle = session
            .register_profile(profile(Role::Client))
            .await
            .map_err(bt_err)?;
        let device = adapter.device(addr).map_err(bt_err)?;
        // BlueZ answers `connect_profile` only after our profile accepted the
        // connection, so both are polled together.
        let (connected, req) = tokio::join!(device.connect_profile(&SERVICE_UUID), async {
            tokio::time::timeout(Duration::from_secs(20), handle.next()).await
        });
        connected.map_err(bt_err)?;
        let req = match req {
            Ok(Some(req)) => req,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "bluetooth: no connection",
                ));
            }
        };
        let sock = blocking(req.accept().map_err(bt_err)?)?;
        Ok::<_, io::Error>((sock, session, handle))
    })?;
    Ok((
        sock,
        Guard {
            _rt: rt,
            _session: session,
            _profile: handle,
        },
    ))
}
