//! encryption for the moment of transfer: data of any structure (as a rule a
//! codec packet) is sealed for one paired device and opened by it; what the
//! data is, is learned from its own headers after opening. Nothing is stored
//! encrypted — opened data is plain.
//!
//! one-shot scheme without session state (the Noise `X` pattern: e, es, ss):
//! - the sender makes a fresh ephemeral X25519 pair for every packet;
//! - key = HKDF-SHA256(DH(eph, recipient) || DH(sender, recipient)), the
//!   context and all three public keys go into `info`. `es` makes the key
//!   unique per packet (so a fixed nonce is safe and no counters have to
//!   survive restarts), `ss` proves the sender: only the paired device can
//!   make a packet that opens;
//! - ChaCha20-Poly1305 over the data; the envelope header and the caller's
//!   `aad` (e.g. an open frame header of the transfer protocol) are
//!   authenticated too: they are not encrypted, but any change to them fails
//!   the opening.
//!
//! envelope: `[version: 1][ephemeral public key: 32][ciphertext: n][tag: 16]`
//! — [`OVERHEAD`] bytes on top of the data.
//!
//! not covered: replay (the same packet can be delivered again — importing a
//! codec packet twice is harmless), the data size is not hidden, and whoever
//! steals the recipient's private key can open packets recorded earlier.

use std::{error::Error, fmt};

use chacha20poly1305::{
    ChaCha20Poly1305, Key, KeyInit, Nonce, Tag,
    aead::{AeadInPlace, Error as AeadError},
};
use hkdf::Hkdf;
use rand_core::OsRng;
use sha2::Sha256;
use x25519_dalek::{EphemeralSecret, PublicKey, SharedSecret, StaticSecret};
use zeroize::Zeroizing;

use crate::keys::{KEY_LEN, PubKey};

/// envelope format version (the first byte).
pub const VERSION: u8 = 1;
const HEADER_LEN: usize = 1 + KEY_LEN;
const TAG_LEN: usize = 16;
/// bytes the envelope adds to the data.
pub const OVERHEAD: usize = HEADER_LEN + TAG_LEN;
/// binds the key to this protocol and envelope version.
const INFO: &[u8] = b"dream_player vpair packet v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptError {
    /// shorter than an empty envelope
    Malformed,
    /// envelope of an unknown version
    UnsupportedVersion(u8),
    /// a public key of low order: the shared secret would be predictable
    WeakKey,
    /// wrong keys (not from / not for this device), or the data is damaged
    /// or altered
    Decrypt,
    /// the data is beyond the AEAD limit (~256 GiB)
    TooLong,
}

impl fmt::Display for CryptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CryptError::Malformed => f.write_str("not an encrypted packet"),
            CryptError::UnsupportedVersion(v) => write!(f, "unsupported envelope version {v}"),
            CryptError::WeakKey => f.write_str("weak public key"),
            CryptError::Decrypt => f.write_str("decryption failed"),
            CryptError::TooLong => f.write_str("data too long"),
        }
    }
}

impl Error for CryptError {}

/// seals `data` from the owner of `own` for the owner of `peer`; `aad` is
/// authenticated but not encrypted and not included (the receiver passes the
/// same bytes to [`open`]).
pub(crate) fn seal(
    own: &StaticSecret,
    peer: &PubKey,
    aad: &[u8],
    data: &[u8],
) -> Result<Vec<u8>, CryptError> {
    let recipient = PublicKey::from(*peer);
    let eph = EphemeralSecret::random_from_rng(OsRng);
    let eph_pub = PublicKey::from(&eph);
    let cipher = cipher(
        &eph.diffie_hellman(&recipient),
        &own.diffie_hellman(&recipient),
        &eph_pub,
        &PublicKey::from(own),
        &recipient,
    )?;

    let mut out = Vec::with_capacity(OVERHEAD + data.len());
    out.push(VERSION);
    out.extend_from_slice(eph_pub.as_bytes());
    out.extend_from_slice(data);
    let (header, body) = out.split_at_mut(HEADER_LEN);
    let tag = cipher
        .encrypt_in_place_detached(&Nonce::default(), &[&header[..], aad].concat(), body)
        .map_err(|_: AeadError| CryptError::TooLong)?;
    out.extend_from_slice(&tag);
    Ok(out)
}

/// opens a packet sealed by the owner of `peer` for the owner of `own`; `aad`
/// must be the same bytes the sender passed to [`seal`].
pub(crate) fn open(
    own: &StaticSecret,
    peer: &PubKey,
    aad: &[u8],
    packet: &[u8],
) -> Result<Vec<u8>, CryptError> {
    if packet.len() < OVERHEAD {
        return Err(CryptError::Malformed);
    }
    if packet[0] != VERSION {
        return Err(CryptError::UnsupportedVersion(packet[0]));
    }
    let (header, rest) = packet.split_at(HEADER_LEN);
    let (body, tag) = rest.split_at(rest.len() - TAG_LEN);
    let mut eph = [0; KEY_LEN];
    eph.copy_from_slice(&header[1..]);
    let eph_pub = PublicKey::from(eph);
    let sender = PublicKey::from(*peer);
    let cipher = cipher(
        &own.diffie_hellman(&eph_pub),
        &own.diffie_hellman(&sender),
        &eph_pub,
        &sender,
        &PublicKey::from(own),
    )?;

    let mut data = body.to_vec();
    cipher
        .decrypt_in_place_detached(
            &Nonce::default(),
            &[header, aad].concat(),
            &mut data,
            Tag::from_slice(tag),
        )
        .map_err(|_: AeadError| CryptError::Decrypt)?;
    Ok(data)
}

/// the packet cipher from both shared secrets and the public keys.
fn cipher(
    es: &SharedSecret,
    ss: &SharedSecret,
    eph: &PublicKey,
    sender: &PublicKey,
    recipient: &PublicKey,
) -> Result<ChaCha20Poly1305, CryptError> {
    if !es.was_contributory() || !ss.was_contributory() {
        return Err(CryptError::WeakKey);
    }
    let mut ikm = Zeroizing::new([0; 2 * KEY_LEN]);
    ikm[..KEY_LEN].copy_from_slice(es.as_bytes());
    ikm[KEY_LEN..].copy_from_slice(ss.as_bytes());
    let info = [
        INFO,
        eph.as_bytes(),
        sender.as_bytes(),
        recipient.as_bytes(),
    ]
    .concat();

    let mut key = Zeroizing::new([0; KEY_LEN]);
    Hkdf::<Sha256>::new(None, &ikm[..])
        .expand(&info, &mut key[..])
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    Ok(ChaCha20Poly1305::new(Key::from_slice(&key[..])))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (StaticSecret, PubKey) {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret).to_bytes();
        (secret, public)
    }

    #[test]
    fn roundtrip_any_data() {
        let (a, a_pub) = pair();
        let (b, b_pub) = pair();
        for data in [&b""[..], b"x", &[0xAB; 100_000][..]] {
            let packet = seal(&a, &b_pub, b"", data).unwrap();
            assert_eq!(packet.len(), data.len() + OVERHEAD);
            assert_eq!(open(&b, &a_pub, b"", &packet).unwrap(), data);
        }
    }

    #[test]
    fn same_data_seals_differently() {
        let (a, _) = pair();
        let (_, b_pub) = pair();
        assert_ne!(
            seal(&a, &b_pub, b"", b"hello").unwrap(),
            seal(&a, &b_pub, b"", b"hello").unwrap()
        );
    }

    #[test]
    fn any_altered_byte_is_detected() {
        let (a, a_pub) = pair();
        let (b, b_pub) = pair();
        let packet = seal(&a, &b_pub, b"", b"some codec packet").unwrap();
        // the version byte gives its own error, every other byte fails the tag.
        for i in 1..packet.len() {
            let mut bad = packet.clone();
            bad[i] ^= 1;
            assert!(open(&b, &a_pub, b"", &bad).is_err(), "byte {i}");
        }
    }

    #[test]
    fn only_the_right_pair_opens() {
        let (a, a_pub) = pair();
        let (b, b_pub) = pair();
        let (c, c_pub) = pair();
        let packet = seal(&a, &b_pub, b"", b"secret").unwrap();
        // a stranger can't open it, and the recipient can't be fooled about the sender.
        assert_eq!(open(&c, &a_pub, b"", &packet), Err(CryptError::Decrypt));
        assert_eq!(open(&b, &c_pub, b"", &packet), Err(CryptError::Decrypt));
        // a packet made by a stranger with a fake sender doesn't open either.
        let forged = seal(&c, &b_pub, b"", b"secret").unwrap();
        assert_eq!(open(&b, &a_pub, b"", &forged), Err(CryptError::Decrypt));
    }

    #[test]
    fn bad_envelopes() {
        let (a, a_pub) = pair();
        assert_eq!(
            open(&a, &a_pub, b"", &[VERSION; OVERHEAD - 1]),
            Err(CryptError::Malformed)
        );
        let mut packet = seal(&a, &a_pub, b"", b"x").unwrap();
        packet[0] = 9;
        assert_eq!(
            open(&a, &a_pub, b"", &packet),
            Err(CryptError::UnsupportedVersion(9))
        );
    }

    #[test]
    fn low_order_key_is_refused() {
        let (a, _) = pair();
        assert_eq!(seal(&a, &[0; KEY_LEN], b"", b"x"), Err(CryptError::WeakKey));
    }

    #[test]
    fn aad_is_authenticated() {
        let (a, a_pub) = pair();
        let (b, b_pub) = pair();
        let packet = seal(&a, &b_pub, b"frame header", b"data").unwrap();
        // the aad is not inside the packet.
        assert_eq!(packet.len(), 4 + OVERHEAD);
        assert_eq!(open(&b, &a_pub, b"frame header", &packet).unwrap(), b"data");
        assert_eq!(
            open(&b, &a_pub, b"frame heaDer", &packet),
            Err(CryptError::Decrypt)
        );
        assert_eq!(open(&b, &a_pub, b"", &packet), Err(CryptError::Decrypt));
    }
}
