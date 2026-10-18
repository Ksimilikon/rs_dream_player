//! TCP transports: a local network address typed by the user, and USB to an
//! Android device through ADB port forwarding (TCP over the cable).

use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// default sync port (TCP and ADB).
pub const DEFAULT_PORT: u16 = 47800;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(200);

/// `host[:port]` -> the first resolved address (the port defaults to
/// [`DEFAULT_PORT`]).
pub fn parse_addr(s: &str) -> io::Result<SocketAddr> {
    let with_port = match s.rsplit_once(':') {
        Some((_, port)) if port.parse::<u16>().is_ok() => s.to_owned(),
        _ => format!("{s}:{DEFAULT_PORT}"),
    };
    with_port
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "address doesn't resolve"))
}

pub fn connect(addr: SocketAddr) -> io::Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

/// waits for one incoming connection on `addr`; `Interrupted` once `cancel`
/// is set.
pub fn accept(addr: SocketAddr, cancel: &AtomicBool) -> io::Result<(TcpStream, SocketAddr)> {
    let listener = TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    loop {
        match listener.accept() {
            Ok((stream, peer)) => {
                stream.set_nonblocking(false)?;
                stream.set_nodelay(true)?;
                return Ok((stream, peer));
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if cancel.load(Ordering::Relaxed) {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                std::thread::sleep(POLL);
            }
            Err(e) => return Err(e),
        }
    }
}

/// runs `adb <args>`; an error carries adb's own message.
pub fn adb(args: &[&str]) -> io::Result<()> {
    let out = Command::new("adb")
        .args(args)
        .output()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => {
                io::Error::new(e.kind(), "adb is not installed (not in PATH)")
            }
            _ => e,
        })?;
    match out.status.success() {
        true => Ok(()),
        false => Err(io::Error::other(format!(
            "adb {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
    }
}
