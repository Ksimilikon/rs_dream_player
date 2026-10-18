//! library sync between devices: the transports (TCP by address, ADB over
//! USB, USB without adb (AOA), Bluetooth RFCOMM on Linux) and the worker thread that moves bytes
//! between a transport and the core's sync session (`sync_*`). The protocol
//! itself — pairing, comparing libraries, confirmations — lives in the core;
//! the worker only carries bytes and writes the received files.
//!
//! the side that listens is the host: it gives its library first, then takes
//! the other one's.

#[cfg(target_os = "linux")]
mod bt;
mod net;
mod usb;

use std::fs;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use api::sync::FFISyncEventKind;

use crate::ffi::{
    self,
    cdc::{Export, Flags},
    sync::Session,
};
use crate::ui::tabs::write_unique;

pub use net::DEFAULT_PORT;

/// how long a read waits before the worker checks for cancellation.
const READ_TIMEOUT: Duration = Duration::from_millis(200);

/// where and how to connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// wait for a connection on every interface (host).
    TcpListen(u16),
    /// connect to `host[:port]` (client).
    TcpConnect(String),
    /// `adb forward`, then connect to the phone that listens (client).
    AdbConnect(u16),
    /// `adb reverse`, then wait for the phone to connect (host).
    AdbListen(u16),
    /// wait for a Bluetooth connection (host).
    BtListen,
    /// connect to a Bluetooth device `AA:BB:CC:DD:EE:FF` (client).
    BtConnect(String),
    /// USB cable without adb: switch the phone to an accessory of ours
    /// (Android Open Accessory); the phone is the host. `VID:PID` picks the
    /// phone, otherwise it is found.
    Usb(usb::Filter),
}

impl Target {
    /// parses the arguments of `:sync` (see the HELP tab).
    pub fn parse(args: &str) -> Result<Self, String> {
        let words: Vec<&str> = args.split_whitespace().collect();
        let port = |w: Option<&&str>| match w {
            None => Ok(DEFAULT_PORT),
            Some(p) => p.parse().map_err(|_| format!("bad port: {p}")),
        };
        match words.as_slice() {
            ["listen", rest @ ..] if rest.len() <= 1 => Ok(Self::TcpListen(port(rest.first())?)),
            ["tcp", addr] => Ok(Self::TcpConnect(addr.to_string())),
            ["adb", "listen", rest @ ..] if rest.len() <= 1 => {
                Ok(Self::AdbListen(port(rest.first())?))
            }
            ["adb", rest @ ..] if rest.len() <= 1 => Ok(Self::AdbConnect(port(rest.first())?)),
            ["bt", "listen"] => Ok(Self::BtListen),
            ["usb"] => Ok(Self::Usb(None)),
            ["usb", id] => Ok(Self::Usb(Some(usb::parse_filter(id)?))),
            ["bt", addr] => Ok(Self::BtConnect(addr.to_string())),
            _ => Err(
                "usage: :sync listen [port] | tcp <ip[:port]> | usb [VID:PID] | adb [listen] [port] | bt listen | bt <MAC>"
                    .into(),
            ),
        }
    }
}

/// what to send one way — owned, it goes to the worker thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Library,
    Playlists(Vec<String>),
    Tracks(Vec<i64>),
}

/// what the session does once connected.
#[derive(Debug, Clone)]
pub enum Mode {
    /// two-way library sync (the other side syncs too).
    Sync,
    /// send a selection with the export flags (the other side receives).
    Send { what: Selection, flags: Flags },
    /// accept a send (the other side sends); `force` — as in import.
    Receive { force: bool },
}

/// what the worker reports to the interface.
pub enum Msg {
    Info(String),
    Ok(String),
    Warn(String),
    Error(String),
    /// received data was saved — the library has to be re-read.
    LibraryChanged,
    /// the worker is over; `complete` — the sync went through both rounds.
    Finished {
        complete: bool,
    },
}

/// a running sync; dropping it cancels the sync.
pub struct Worker {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Worker {
    pub fn start(target: Target, mode: Mode, music_dir: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let thread = thread::Builder::new()
            .name("sync".into())
            .spawn(move || run(target, &mode, &music_dir, &tx, &flag))
            .expect("spawning the sync thread");
        Self { rx, cancel, thread }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn running(&self) -> bool {
        !self.thread.is_finished()
    }

    /// messages since the last call.
    pub fn poll(&self) -> Vec<Msg> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // not joined: a connect in progress may take seconds; the thread
        // notices the flag and ends by itself.
        self.cancel();
    }
}

/// a connected byte stream.
trait Link: Read + Write + Send {
    fn set_timeout(&self, t: Option<Duration>) -> io::Result<()>;
}

impl Link for TcpStream {
    fn set_timeout(&self, t: Option<Duration>) -> io::Result<()> {
        self.set_read_timeout(t)
    }
}

#[cfg(unix)]
impl Link for std::os::unix::net::UnixStream {
    fn set_timeout(&self, t: Option<Duration>) -> io::Result<()> {
        self.set_read_timeout(t)
    }
}

/// keeps transport resources alive while the connection lives.
type Guard = Option<Box<dyn Send>>;

fn run(target: Target, mode: &Mode, music_dir: &Path, tx: &Sender<Msg>, cancel: &AtomicBool) {
    let send = |m| {
        let _ = tx.send(m);
    };
    if let Ok(key) = ffi::con::pub_key() {
        send(Msg::Info(format!(
            "this device: {}",
            ffi::con::fingerprint(&key)
        )));
    }
    let mut complete = false;
    match establish(&target, tx, cancel) {
        Ok((link, host, _guard)) => match ffi::con::open() {
            Ok(con) => {
                match drive(link, host, con, mode, music_dir, tx, cancel) {
                    Ok(done) => complete = done,
                    Err(e) => send(Msg::Error(format!("connection: {e}"))),
                }
                ffi::con::close(con);
            }
            Err(st) => send(Msg::Error(format!("sync: {}", ffi::con::describe(st)))),
        },
        Err(e) if e.kind() == io::ErrorKind::Interrupted => send(Msg::Warn("cancelled".into())),
        Err(e) => send(Msg::Error(e.to_string())),
    }
    send(Msg::Finished { complete });
}

/// brings the transport up; `true` — this side is the host.
fn establish(
    target: &Target,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> io::Result<(Box<dyn Link>, bool, Guard)> {
    let info = |s: String| {
        let _ = tx.send(Msg::Info(s));
    };
    let any = |port| SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
    let local = |port| SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    match target {
        Target::TcpListen(port) => {
            info(format!(
                "waiting for a connection on port {port} (c: cancel)"
            ));
            let (s, peer) = net::accept(any(*port), cancel)?;
            info(format!("connected: {peer}"));
            Ok((Box::new(s), true, None))
        }
        Target::TcpConnect(addr) => {
            let addr = net::parse_addr(addr)?;
            info(format!("connecting to {addr}"));
            Ok((Box::new(net::connect(addr)?), false, None))
        }
        Target::AdbConnect(port) => {
            let p = format!("tcp:{port}");
            net::adb(&["forward", &p, &p])?;
            info(format!("adb forward {p}; connecting to the phone"));
            Ok((Box::new(net::connect(local(*port))?), false, None))
        }
        Target::AdbListen(port) => {
            let p = format!("tcp:{port}");
            net::adb(&["reverse", &p, &p])?;
            info(format!(
                "adb reverse {p}; waiting for the phone (c: cancel)"
            ));
            let (s, _) = net::accept(local(*port), cancel)?;
            info("phone connected".into());
            Ok((Box::new(s), true, None))
        }
        Target::Usb(filter) => {
            let link = usb::connect(*filter, cancel, &info)?;
            info("connected over USB".into());
            Ok((Box::new(link), false, None))
        }
        #[cfg(target_os = "linux")]
        Target::BtListen => {
            info("waiting for a bluetooth connection (c: cancel)".into());
            let (s, peer, guard) = bt::accept(cancel)?;
            info(format!("connected: {peer}"));
            Ok((Box::new(s), true, Some(Box::new(guard))))
        }
        #[cfg(target_os = "linux")]
        Target::BtConnect(addr) => {
            info(format!("connecting to {addr} over bluetooth"));
            let (s, guard) = bt::connect(addr)?;
            Ok((Box::new(s), false, Some(Box::new(guard))))
        }
        #[cfg(not(target_os = "linux"))]
        Target::BtListen | Target::BtConnect(_) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "bluetooth sync is available on Linux only for now",
        )),
    }
}

/// carries bytes between the link and the session until it is over;
/// `true` — the session is complete.
fn drive(
    mut link: Box<dyn Link>,
    host: bool,
    con: u64,
    mode: &Mode,
    music_dir: &Path,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> io::Result<bool> {
    let send = |m| {
        let _ = tx.send(m);
    };
    let session = match mode {
        Mode::Sync => Session::new(con, host),
        Mode::Send { what, flags } => {
            let what = match what {
                Selection::Library => Export::Library,
                Selection::Playlists(names) => Export::Playlists(names),
                Selection::Tracks(ids) => Export::Tracks(ids),
            };
            Session::new_send(con, host, what, *flags)
        }
        Mode::Receive { force } => Session::new_receive(con, host, *force),
    };
    let mut s = session.map_err(|st| io::Error::other(ffi::sync::describe(st)))?;
    link.set_timeout(Some(READ_TIMEOUT))?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut changed = false;
    let mut complete = false;
    let res = loop {
        let out = s.take_output();
        if !out.is_empty() {
            link.write_all(&out)?;
            link.flush()?;
        }
        while let Some(ev) = s.poll_event() {
            match ev.kind {
                FFISyncEventKind::Import => import(&mut s, music_dir, tx),
                FFISyncEventKind::Received => changed = true,
                FFISyncEventKind::Done => complete = true,
                _ => {}
            }
            if let Some(msg) = describe(
                ev.kind,
                ev.status,
                ev.by_peer,
                ev.paired_id,
                ev.songs,
                ev.playlists,
            ) {
                send(msg);
            }
        }
        if s.finished() {
            break Ok(complete);
        }
        if cancel.load(Ordering::Relaxed) {
            s.cancel();
            send(Msg::Warn("cancelled".into()));
            break Ok(false);
        }
        match link.read(&mut buf) {
            Ok(0) => {
                break Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "closed by the peer",
                ));
            }
            Ok(n) => s.feed(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(e) => break Err(e),
        }
    };
    // the last answer (`Ack` / `Abort`) still has to reach the peer.
    let out = s.take_output();
    if !out.is_empty() {
        let _ = link.write_all(&out).and_then(|()| link.flush());
    }
    if changed {
        send(Msg::LibraryChanged);
    }
    res
}

/// writes the files of a received data portion: a song over the local file
/// with the same name if there is one, anything else next to the music.
fn import(s: &mut Session, music_dir: &Path, tx: &Sender<Msg>) {
    let files = s.import_files();
    let paths: Vec<Option<PathBuf>> = files
        .iter()
        .map(|f| {
            let res = match (&f.replace, f.cover) {
                (Some(path), false) => fs::write(path, f.data).map(|()| path.clone()),
                _ => write_unique(music_dir, &f.name, f.data),
            };
            res.map_err(|e| {
                let _ = tx.send(Msg::Warn(format!("failed to write {}: {e}", f.name)));
            })
            .ok()
        })
        .collect();
    drop(files);
    let refs: Vec<Option<&Path>> = paths.iter().map(|p| p.as_deref()).collect();
    if let Err(st) = s.import_done(&refs) {
        let _ = tx.send(Msg::Error(format!("import: {}", ffi::sync::describe(st))));
    }
}

/// a session event -> a log line (`None` — not worth one).
fn describe(
    kind: FFISyncEventKind,
    status: u32,
    by_peer: bool,
    paired_id: i64,
    songs: u32,
    playlists: u32,
) -> Option<Msg> {
    use FFISyncEventKind as K;
    let what = |verb: &str| match (songs, playlists) {
        (s, 0) => format!("{verb} {s} song(s)"),
        (0, p) => format!("{verb} {p} playlist(s)"),
        (s, p) => format!("{verb} {s} song(s), {p} playlist(s)"),
    };
    Some(match kind {
        K::None | K::Import => return None,
        K::Paired => {
            let peer = ffi::con::paired().into_iter().find(|p| p.id == paired_id);
            let name = peer.as_ref().map_or("?".into(), |p| {
                format!("{} [{}]", p.name, ffi::con::fingerprint(&p.key))
            });
            Msg::Ok(format!("paired with #{paired_id} {name}"))
        }
        K::Sent => Msg::Info(what("sent")),
        K::Received => Msg::Info(what("received")),
        K::PeerRejected => Msg::Warn(format!(
            "the peer didn't accept a portion: {}",
            ffi::sync::describe_wire(status)
        )),
        K::Done => Msg::Ok("complete".into()),
        K::Failed => Msg::Error(format!(
            "failed ({}): {}",
            if by_peer { "peer" } else { "here" },
            ffi::sync::describe_wire(status)
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_targets() {
        assert_eq!(Target::parse("listen"), Ok(Target::TcpListen(DEFAULT_PORT)));
        assert_eq!(Target::parse("listen 5000"), Ok(Target::TcpListen(5000)));
        assert_eq!(
            Target::parse("tcp 192.168.1.5"),
            Ok(Target::TcpConnect("192.168.1.5".into()))
        );
        assert_eq!(Target::parse("adb"), Ok(Target::AdbConnect(DEFAULT_PORT)));
        assert_eq!(
            Target::parse("adb listen 6000"),
            Ok(Target::AdbListen(6000))
        );
        assert_eq!(Target::parse("bt listen"), Ok(Target::BtListen));
        assert_eq!(Target::parse("usb"), Ok(Target::Usb(None)));
        assert_eq!(Target::parse("usb 0e8d:2008"), Ok(Target::Usb(Some((0x0e8d, 0x2008)))));
        assert_eq!(
            Target::parse("bt AA:BB:CC:DD:EE:FF"),
            Ok(Target::BtConnect("AA:BB:CC:DD:EE:FF".into()))
        );
        assert!(Target::parse("listen x").is_err());
        assert!(Target::parse("tcp").is_err());
        assert!(Target::parse("bogus").is_err());
    }

    #[test]
    fn address_gets_default_port() {
        assert_eq!(net::parse_addr("127.0.0.1").unwrap().port(), DEFAULT_PORT);
        assert_eq!(net::parse_addr("127.0.0.1:9").unwrap().port(), 9);
    }
}
