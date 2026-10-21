//! frames of the sync protocol over any byte stream (TCP, Bluetooth RFCOMM,
//! ADB, ...). The header is open (not encrypted); the payload of `Index` /
//! `Want` / `Data` is encrypted for the paired device, and the header is bound
//! to it (it is the AAD of the encryption, so a changed tag / status / length
//! fails the decryption).
//!
//! header, [`HEADER_LEN`] bytes, integers little-endian:
//! 1) magic: 4 bytes, [`MAGIC`] (`b"DPSY"`)
//! 2) version: `u8`, [`VERSION`]
//! 3) tag: `u8`, [`Tag`]
//! 4) status: `u8`, [`WireStatus`]; `Ok` in every frame except `Ack` / `Abort`
//! 5) payload length: `u32`, at most [`MAX_PAYLOAD`]
//!
//! then the payload. A frame with an unknown tag / status is still read whole
//! (its length is known) and answered with `Ack(InvalidPacket)`; a broken
//! magic / version or a huge length means the stream can't be trusted.
//!
//! `Tag`, `WireStatus` discriminants are part of the wire format: never
//! renumber, only append.

/// magic bytes at the start of every frame.
pub const MAGIC: [u8; 4] = *b"DPSY";
/// frame format version.
pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 11;
/// the largest payload accepted (one data portion is far smaller).
pub const MAX_PAYLOAD: u32 = 1 << 30;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    /// the sender's public key (32 bytes) and its session mode (1 byte,
    /// [`Mode`]), open — pairing.
    Hello = 1,
    /// encrypted codec packet: every playlist (names, dates, song hashes)
    /// and the hashes of the songs in playlists, without files.
    Index = 2,
    /// encrypted codec packet: what the receiver lacks — song hashes and
    /// playlist names.
    Want = 3,
    /// encrypted codec packet: a portion of the wanted songs (with files
    /// and covers), the last portion — the wanted playlists with covers.
    Data = 4,
    /// the sender has no more `Data` in this round (no payload).
    Done = 5,
    /// the result of the last received frame (no payload).
    Ack = 6,
    /// the session is broken off; the status says why (no payload).
    Abort = 7,
}

impl Tag {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::Hello,
            2 => Self::Index,
            3 => Self::Want,
            4 => Self::Data,
            5 => Self::Done,
            6 => Self::Ack,
            7 => Self::Abort,
            _ => return None,
        })
    }
}

/// result / reason carried by `Ack` and `Abort`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireStatus {
    Ok = 0,
    /// unknown tag / status, a bad header or an undecodable payload.
    InvalidPacket = 1,
    /// the frame is not expected at this point of the session.
    Unexpected = 2,
    /// the payload doesn't decrypt: not from / not for this pair of devices,
    /// or damaged.
    DecryptFailed = 3,
    /// pairing failed (e.g. the key differs from the saved one).
    PairFailed = 4,
    /// a data portion was not accepted (e.g. its playlists are older).
    Rejected = 5,
    /// the receiver failed to store the data.
    WriteFailed = 6,
    /// the device is busy with another transfer.
    Busy = 7,
    /// cancelled by the user.
    Cancelled = 8,
    /// an internal error of the sender (db, files).
    Internal = 9,
    /// the session modes of the two sides don't fit (e.g. sync against send).
    ModeMismatch = 10,
}

impl WireStatus {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Ok,
            1 => Self::InvalidPacket,
            2 => Self::Unexpected,
            3 => Self::DecryptFailed,
            4 => Self::PairFailed,
            5 => Self::Rejected,
            6 => Self::WriteFailed,
            7 => Self::Busy,
            8 => Self::Cancelled,
            9 => Self::Internal,
            10 => Self::ModeMismatch,
            _ => return None,
        })
    }
}

/// what a side wants from the session, announced in `Hello`. Fitting pairs:
/// `Sync`–`Sync`, `Send`–`Receive`. Discriminants are part of the wire
/// format.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// two-way library sync.
    Sync = 0,
    /// one-way: this side sends a chosen selection.
    Send = 1,
    /// one-way: this side accepts what the other one sends.
    Receive = 2,
}

impl Mode {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Sync,
            1 => Self::Send,
            2 => Self::Receive,
            _ => return None,
        })
    }

    /// the two sides can talk.
    pub fn fits(self, peer: Self) -> bool {
        matches!(
            (self, peer),
            (Self::Sync, Self::Sync) | (Self::Send, Self::Receive) | (Self::Receive, Self::Send)
        )
    }
}

/// a frame header for the given payload length.
pub fn header(tag: Tag, status: WireStatus, len: usize) -> [u8; HEADER_LEN] {
    let mut h = [0; HEADER_LEN];
    h[..4].copy_from_slice(&MAGIC);
    h[4] = VERSION;
    h[5] = tag as u8;
    h[6] = status as u8;
    h[7..].copy_from_slice(&(len as u32).to_le_bytes());
    h
}

/// one read frame.
#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    Known {
        tag: Tag,
        status: WireStatus,
        /// the raw header — the AAD of the payload encryption.
        header: [u8; HEADER_LEN],
        payload: Vec<u8>,
    },
    /// unknown tag or status; the payload is skipped.
    Unknown,
}

/// the stream can't be read further.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    BadMagic,
    BadVersion,
    TooLarge,
}

/// collects stream bytes and cuts them into frames.
#[derive(Default)]
pub struct Reader {
    buf: Vec<u8>,
}

impl Reader {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// the next whole frame; `None` — not enough bytes yet.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        if self.buf.len() < HEADER_LEN {
            return Ok(None);
        }
        let mut header = [0; HEADER_LEN];
        header.copy_from_slice(&self.buf[..HEADER_LEN]);
        if header[..4] != MAGIC {
            return Err(FrameError::BadMagic);
        }
        if header[4] != VERSION {
            return Err(FrameError::BadVersion);
        }
        let len = u32::from_le_bytes([header[7], header[8], header[9], header[10]]);
        if len > MAX_PAYLOAD {
            return Err(FrameError::TooLarge);
        }
        let end = HEADER_LEN + len as usize;
        if self.buf.len() < end {
            return Ok(None);
        }
        let payload = self.buf[HEADER_LEN..end].to_vec();
        self.buf.drain(..end);
        Ok(Some(
            match (Tag::from_u8(header[5]), WireStatus::from_u8(header[6])) {
                (Some(tag), Some(status)) => Frame::Known {
                    tag,
                    status,
                    header,
                    payload,
                },
                _ => Frame::Unknown,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(tag: Tag, status: WireStatus, payload: &[u8]) -> Vec<u8> {
        [&header(tag, status, payload.len())[..], payload].concat()
    }

    #[test]
    fn reads_frames_split_anywhere() {
        let bytes = [
            frame(Tag::Hello, WireStatus::Ok, &[7; 32]),
            frame(Tag::Ack, WireStatus::Rejected, &[]),
        ]
        .concat();
        let mut r = Reader::default();
        let mut got = Vec::new();
        for chunk in bytes.chunks(5) {
            r.push(chunk);
            while let Some(f) = r.next_frame().unwrap() {
                got.push(f);
            }
        }
        assert_eq!(got.len(), 2);
        assert!(
            matches!(&got[0], Frame::Known { tag: Tag::Hello, payload, .. } if payload == &[7; 32])
        );
        assert!(matches!(
            got[1],
            Frame::Known {
                tag: Tag::Ack,
                status: WireStatus::Rejected,
                ..
            }
        ));
    }

    #[test]
    fn unknown_tag_or_status_is_skipped_whole() {
        let mut bad_tag = frame(Tag::Data, WireStatus::Ok, b"xyz");
        bad_tag[5] = 99;
        let mut bad_status = frame(Tag::Ack, WireStatus::Ok, &[]);
        bad_status[6] = 200;
        let mut r = Reader::default();
        r.push(&[bad_tag, bad_status, frame(Tag::Done, WireStatus::Ok, &[])].concat());
        assert_eq!(r.next_frame().unwrap(), Some(Frame::Unknown));
        assert_eq!(r.next_frame().unwrap(), Some(Frame::Unknown));
        assert!(matches!(
            r.next_frame().unwrap(),
            Some(Frame::Known { tag: Tag::Done, .. })
        ));
    }

    #[test]
    fn broken_stream_is_reported() {
        let mut r = Reader::default();
        r.push(b"GET / HTTP/1.1\r\n");
        assert_eq!(r.next_frame(), Err(FrameError::BadMagic));

        let mut huge = header(Tag::Data, WireStatus::Ok, 0);
        huge[7..].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut r = Reader::default();
        r.push(&huge);
        assert_eq!(r.next_frame(), Err(FrameError::TooLarge));
    }
}
