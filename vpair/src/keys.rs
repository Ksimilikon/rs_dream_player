//! X25519 key pair of the current device, kept in two files (like ssh keys):
//! `id_x25519` — the private key, `id_x25519.pub` — the public one. Both are
//! raw 32 bytes. The pair is generated on the first start.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
};

use rand_core::OsRng;
use x25519_dalek::{PublicKey, StaticSecret};

/// raw X25519 key length.
pub const KEY_LEN: usize = 32;

/// public X25519 key: of the current device or of a peer.
pub type PubKey = [u8; KEY_LEN];

pub const PRIVATE_KEY_FILE: &str = "id_x25519";
pub const PUBLIC_KEY_FILE: &str = "id_x25519.pub";

/// loads the key pair from `dir`, generating it if there is no private key
/// yet. The private key is the source of truth: a missing or stale public key
/// file is rewritten from it. An existing but broken private key is an error
/// — it is never replaced silently, otherwise the device would lose its
/// identity and every pairing with it.
pub(crate) fn load_or_create(dir: &Path) -> io::Result<(PubKey, StaticSecret)> {
    fs::create_dir_all(dir)?;
    let private_path = dir.join(PRIVATE_KEY_FILE);

    let secret = match fs::read(&private_path) {
        Ok(raw) => {
            let raw: [u8; KEY_LEN] = raw.try_into().map_err(|raw: Vec<u8>| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{PRIVATE_KEY_FILE}: {} bytes instead of {KEY_LEN}",
                        raw.len()
                    ),
                )
            })?;
            StaticSecret::from(raw)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let secret = StaticSecret::random_from_rng(OsRng);
            write_private(&private_path, &secret.to_bytes())?;
            secret
        }
        Err(e) => return Err(e),
    };

    let public = PublicKey::from(&secret).to_bytes();
    let public_path = dir.join(PUBLIC_KEY_FILE);
    if fs::read(&public_path).ok().as_deref() != Some(&public[..]) {
        fs::write(&public_path, public)?;
    }
    Ok((public, secret))
}

/// writes a new private key readable only by the owner (0600 on unix).
/// `create_new` — an existing file is never overwritten.
fn write_private(path: &Path, raw: &[u8; KEY_LEN]) -> io::Result<()> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    file.write_all(raw)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn creates_once_then_loads_the_same_pair() {
        let dir = tempdir().unwrap();
        let (public, _) = load_or_create(dir.path()).unwrap();
        assert_eq!(fs::read(dir.path().join(PUBLIC_KEY_FILE)).unwrap(), public);

        let (again, _) = load_or_create(dir.path()).unwrap();
        assert_eq!(public, again);
    }

    #[test]
    fn restores_public_key_from_private() {
        let dir = tempdir().unwrap();
        let (public, _) = load_or_create(dir.path()).unwrap();
        fs::remove_file(dir.path().join(PUBLIC_KEY_FILE)).unwrap();

        let (again, _) = load_or_create(dir.path()).unwrap();
        assert_eq!(public, again);
        assert_eq!(fs::read(dir.path().join(PUBLIC_KEY_FILE)).unwrap(), public);
    }

    #[test]
    fn broken_private_key_is_not_replaced() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(PRIVATE_KEY_FILE), b"short").unwrap();

        assert!(load_or_create(dir.path()).is_err());
        assert_eq!(
            fs::read(dir.path().join(PRIVATE_KEY_FILE)).unwrap(),
            b"short"
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_key_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        load_or_create(dir.path()).unwrap();
        let mode = fs::metadata(dir.path().join(PRIVATE_KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
