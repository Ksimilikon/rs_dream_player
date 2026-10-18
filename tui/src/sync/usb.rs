//! USB transport without adb / debug mode: Android Open Accessory (AOA).
//! This side is the USB host: it finds the phone, asks it to restart as an
//! accessory of ours (the phone then offers to open Dream Player), and talks
//! to the accessory interface over two bulk endpoints. The phone is the
//! listening side (the host of the sync session).
//!
//! AOA v1: vendor control requests `51` (protocol version), `52` (identity
//! strings), `53` (start); the phone re-enumerates as Google `18d1:2d00`
//! (`2d01` — with adb). Linux needs access to the device nodes for a normal
//! user (udev rule, see the HELP tab); macOS works as is; Windows needs the
//! WinUSB driver for the accessory interface.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rusb::{Context, Device, DeviceHandle, Direction, TransferType, UsbContext};

use super::Link;

/// the identity the phone app's accessory filter matches.
const MANUFACTURER: &str = "DreamPlayer";
const MODEL: &str = "DreamPlayerSync";
const DESCRIPTION: &str = "Dream Player library sync";
const VERSION: &str = "1";
const URI: &str = "https://github.com/ksimilikon/rs_dream_player";
const SERIAL: &str = "";

const GOOGLE_VID: u16 = 0x18d1;
/// accessory; accessory + adb.
const ACCESSORY_PIDS: [u16; 2] = [0x2d00, 0x2d01];

const REQ_GET_PROTOCOL: u8 = 51;
const REQ_SEND_STRING: u8 = 52;
const REQ_START: u8 = 53;

const CONTROL_TIMEOUT: Duration = Duration::from_secs(1);
/// the phone app may still be waiting for the user to start the session.
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);
/// how long the phone takes to come back as an accessory.
const REENUMERATE: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(200);
/// one bulk write (the phone reads the accessory in 16 KiB requests).
const CHUNK: usize = 16 * 1024;

/// a phone given as `VID:PID` (hex), or found automatically.
pub type Filter = Option<(u16, u16)>;

/// `0e8d:2008` -> `(0x0e8d, 0x2008)`.
pub fn parse_filter(s: &str) -> Result<(u16, u16), String> {
    let (v, p) = s.split_once(':').ok_or("usb: expected VID:PID, e.g. 0e8d:2008")?;
    let hex = |x: &str| u16::from_str_radix(x, 16).map_err(|_| format!("usb: bad id {x}"));
    Ok((hex(v)?, hex(p)?))
}

/// the accessory interface of a phone, opened.
pub struct UsbLink {
    handle: DeviceHandle<Context>,
    ep_in: u8,
    ep_out: u8,
    max_packet: usize,
    /// read timeout in ms (0 — block).
    timeout_ms: AtomicU64,
    /// received bytes that didn't fit the caller's buffer.
    pending: Vec<u8>,
}

impl Read for UsbLink {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pending.is_empty() {
            let t = Duration::from_millis(self.timeout_ms.load(Ordering::Relaxed));
            // whole packets only: a smaller request could overflow.
            let mut tmp = vec![0u8; CHUNK];
            let n = self.handle.read_bulk(self.ep_in, &mut tmp, t).map_err(usb_err)?;
            tmp.truncate(n);
            self.pending = tmp;
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}

impl Write for UsbLink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let chunk = &buf[..buf.len().min(CHUNK)];
        let n = self
            .handle
            .write_bulk(self.ep_out, chunk, WRITE_TIMEOUT)
            .map_err(|e| match e {
                rusb::Error::Timeout => io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the phone doesn't take data: open Dream Player › Devices, choose USB cable and start",
                ),
                e => usb_err(e),
            })?;
        // a transfer of whole packets has no end mark: the phone would wait
        // for more. A zero-length packet ends it.
        if n > 0 && n % self.max_packet == 0 {
            let _ = self.handle.write_bulk(self.ep_out, &[], WRITE_TIMEOUT);
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Link for UsbLink {
    fn set_timeout(&self, t: Option<Duration>) -> io::Result<()> {
        let ms = t.map_or(0, |t| t.as_millis().max(1) as u64);
        self.timeout_ms.store(ms, Ordering::Relaxed);
        Ok(())
    }
}

fn usb_err(e: rusb::Error) -> io::Error {
    match e {
        rusb::Error::Timeout => io::Error::new(io::ErrorKind::TimedOut, "usb: timeout"),
        rusb::Error::NoDevice | rusb::Error::Pipe => {
            io::Error::new(io::ErrorKind::ConnectionAborted, "usb: the phone was disconnected")
        }
        rusb::Error::Access => io::Error::new(
            io::ErrorKind::PermissionDenied,
            "usb: no access to the device (Linux: add the udev rule from the HELP tab)",
        ),
        e => io::Error::other(format!("usb: {e}")),
    }
}

/// finds the phone, switches it to accessory mode if needed and opens the
/// accessory interface. `Interrupted` once `cancel` is set.
pub fn connect(filter: Filter, cancel: &AtomicBool, info: &dyn Fn(String)) -> io::Result<UsbLink> {
    let ctx = Context::new().map_err(usb_err)?;
    if let Some(dev) = find_accessory(&ctx)? {
        info("the phone is already an accessory".into());
        return open(dev);
    }
    let phone = find_phone(&ctx, filter)?;
    let (vid, pid) = ids(&phone);
    info(format!("phone {vid:04x}:{pid:04x}: switching to accessory mode"));
    start_accessory(&phone)?;
    info("confirm \"Open Dream Player\" on the phone".into());
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if let Some(dev) = find_accessory(&ctx)? {
            return open(dev);
        }
        if started.elapsed() > REENUMERATE {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "usb: the phone didn't come back as an accessory (unplug and retry)",
            ));
        }
        std::thread::sleep(POLL);
    }
}

fn ids(dev: &Device<Context>) -> (u16, u16) {
    dev.device_descriptor()
        .map(|d| (d.vendor_id(), d.product_id()))
        .unwrap_or((0, 0))
}

fn find_accessory(ctx: &Context) -> io::Result<Option<Device<Context>>> {
    Ok(ctx.devices().map_err(usb_err)?.iter().find(|d| {
        let (v, p) = ids(d);
        v == GOOGLE_VID && ACCESSORY_PIDS.contains(&p)
    }))
}

/// the phone to switch: the given `VID:PID`, otherwise the one device that
/// answers the AOA protocol request (only devices with a vendor-specific /
/// still-image (MTP) interface are asked — not keyboards, hubs...).
fn find_phone(ctx: &Context, filter: Filter) -> io::Result<Device<Context>> {
    let devices = ctx.devices().map_err(usb_err)?;
    if let Some((v, p)) = filter {
        return devices
            .iter()
            .find(|d| ids(d) == (v, p))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("usb: no device {v:04x}:{p:04x}")));
    }
    let mut denied = false;
    for dev in devices.iter().filter(maybe_phone) {
        match protocol(&dev) {
            Ok(v) if v >= 1 => return Ok(dev),
            Err(rusb::Error::Access) => denied = true,
            _ => {}
        }
    }
    Err(match denied {
        true => usb_err(rusb::Error::Access),
        false => io::Error::new(
            io::ErrorKind::NotFound,
            "usb: no Android phone found (is the cable a data cable? unlock the phone)",
        ),
    })
}

fn maybe_phone(dev: &Device<Context>) -> bool {
    let Ok(config) = dev.active_config_descriptor() else {
        return false;
    };
    config
        .interfaces()
        .flat_map(|i| i.descriptors())
        .any(|d| matches!(d.class_code(), 0x06 | 0xff))
}

/// the AOA protocol version of a device (0 — not supported).
fn protocol(dev: &Device<Context>) -> rusb::Result<u16> {
    let handle = dev.open()?;
    let mut buf = [0u8; 2];
    let rt = rusb::request_type(Direction::In, rusb::RequestType::Vendor, rusb::Recipient::Device);
    let n = handle.read_control(rt, REQ_GET_PROTOCOL, 0, 0, &mut buf, CONTROL_TIMEOUT)?;
    Ok(if n == 2 { u16::from_le_bytes(buf) } else { 0 })
}

fn start_accessory(dev: &Device<Context>) -> io::Result<()> {
    let handle = dev.open().map_err(usb_err)?;
    let rt = rusb::request_type(Direction::Out, rusb::RequestType::Vendor, rusb::Recipient::Device);
    for (index, s) in [MANUFACTURER, MODEL, DESCRIPTION, VERSION, URI, SERIAL].iter().enumerate() {
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0);
        handle
            .write_control(rt, REQ_SEND_STRING, 0, index as u16, &bytes, CONTROL_TIMEOUT)
            .map_err(usb_err)?;
    }
    handle
        .write_control(rt, REQ_START, 0, 0, &[], CONTROL_TIMEOUT)
        .map_err(usb_err)?;
    Ok(())
}

/// claims interface 0 of the accessory and finds its bulk endpoints.
fn open(dev: Device<Context>) -> io::Result<UsbLink> {
    let config = dev.active_config_descriptor().map_err(usb_err)?;
    let iface = config
        .interfaces()
        .next()
        .and_then(|i| i.descriptors().next())
        .ok_or_else(|| io::Error::other("usb: the accessory has no interface"))?;
    let (mut ep_in, mut ep_out, mut max_packet) = (None, None, 512usize);
    for ep in iface.endpoint_descriptors() {
        if ep.transfer_type() != TransferType::Bulk {
            continue;
        }
        max_packet = ep.max_packet_size().max(1) as usize;
        match ep.direction() {
            Direction::In => ep_in = Some(ep.address()),
            Direction::Out => ep_out = Some(ep.address()),
        }
    }
    let (Some(ep_in), Some(ep_out)) = (ep_in, ep_out) else {
        return Err(io::Error::other("usb: the accessory has no bulk endpoints"));
    };
    let handle = dev.open().map_err(usb_err)?;
    let _ = handle.set_auto_detach_kernel_driver(true);
    handle
        .claim_interface(iface.interface_number())
        .map_err(usb_err)?;
    Ok(UsbLink {
        handle,
        ep_in,
        ep_out,
        max_packet,
        timeout_ms: AtomicU64::new(0),
        pending: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vid_pid() {
        assert_eq!(parse_filter("0e8d:2008"), Ok((0x0e8d, 0x2008)));
        assert!(parse_filter("0e8d").is_err());
        assert!(parse_filter("zz:01").is_err());
    }
}
