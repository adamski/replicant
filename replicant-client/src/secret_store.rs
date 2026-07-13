//! Encrypted-at-rest storage for the per-user sync credential.
//! See plan doc for the at-rest key limitation (co-located key = obfuscation,
//! not strong protection; DPAPI wrapping is a Windows follow-up).

use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use rand::RngCore;
use std::io::{self, Error, ErrorKind};
use std::path::Path;

const KEY_FILE: &str = "key.bin";
const CRED_FILE: &str = "credentials.enc";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Credentials {
    pub api_key: String,
    pub secret: String,
    pub user_id: uuid::Uuid,
}

/// Reads the at-rest key if present. Never creates one — callers that must
/// mint a key when absent use [`load_or_create_key`].
fn read_key(dir: &Path) -> io::Result<Option<[u8; 32]>> {
    let path = dir.join(KEY_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| Error::new(ErrorKind::InvalidData, "bad key length"))?;
    Ok(Some(arr))
}

fn load_or_create_key(dir: &Path) -> io::Result<[u8; 32]> {
    if let Some(key) = read_key(dir)? {
        return Ok(key);
    }
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    std::fs::create_dir_all(dir)?;
    write_private(&dir.join(KEY_FILE), &key)?;
    Ok(key)
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // TODO(hardening): wrap with DPAPI on Windows. For now rely on the
    // user-profile directory ACL.
    std::fs::write(path, bytes)
}

pub fn store(dir: &Path, creds: &Credentials) -> io::Result<()> {
    let key = load_or_create_key(dir)?;
    let cipher = ChaCha20Poly1305::new((&key).into());
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let plaintext = serde_json::to_vec(creds).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|_| Error::new(ErrorKind::Other, "encrypt failed"))?;

    let mut out = nonce_bytes.to_vec();
    out.extend_from_slice(&ciphertext);
    write_private(&dir.join(CRED_FILE), &out)
}

pub fn load(dir: &Path) -> io::Result<Option<Credentials>> {
    let path = dir.join(CRED_FILE);
    if !path.exists() {
        return Ok(None);
    }
    // Never mint a key on the read path: an existing credentials file
    // without a key is a broken/tampered state, not a "first run".
    let key = match read_key(dir)? {
        Some(key) => key,
        None => return Ok(None),
    };
    let cipher = ChaCha20Poly1305::new((&key).into());

    let bytes = std::fs::read(&path)?;
    if bytes.len() < 12 {
        return Err(Error::new(ErrorKind::InvalidData, "truncated"));
    }
    let (nonce_bytes, ciphertext) = bytes.split_at(12);
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| Error::new(ErrorKind::InvalidData, "decrypt failed"))?;

    let creds =
        serde_json::from_slice(&plaintext).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
    Ok(Some(creds))
}

pub fn clear(dir: &Path) -> io::Result<()> {
    let cred_path = dir.join(CRED_FILE);
    if cred_path.exists() {
        std::fs::remove_file(cred_path)?;
    }
    let key_path = dir.join(KEY_FILE);
    if key_path.exists() {
        std::fs::remove_file(key_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_credentials() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).unwrap().is_none());

        let user_id = uuid::Uuid::new_v4();
        let creds = Credentials {
            api_key: "rpa_x".into(),
            secret: "rps_y".into(),
            user_id,
        };
        store(dir.path(), &creds).unwrap();

        let loaded = load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.api_key, "rpa_x");
        assert_eq!(loaded.secret, "rps_y");
        assert_eq!(loaded.user_id, user_id);

        clear(dir.path()).unwrap();
        assert!(load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn clear_removes_both_key_and_credentials_files() {
        let dir = tempfile::tempdir().unwrap();
        store(
            dir.path(),
            &Credentials {
                api_key: "rpa_x".into(),
                secret: "rps_y".into(),
                user_id: uuid::Uuid::new_v4(),
            },
        )
        .unwrap();
        assert!(dir.path().join(KEY_FILE).exists());
        assert!(dir.path().join(CRED_FILE).exists());

        clear(dir.path()).unwrap();

        assert!(!dir.path().join(KEY_FILE).exists());
        assert!(!dir.path().join(CRED_FILE).exists());
    }

    #[test]
    fn load_on_empty_dir_returns_none_and_mints_no_key() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).unwrap().is_none());
        assert!(
            !dir.path().join(KEY_FILE).exists(),
            "load() must never mint a key"
        );
        assert!(!dir.path().join(CRED_FILE).exists());
    }

    #[test]
    fn tampered_ciphertext_fails_to_decrypt() {
        let dir = tempfile::tempdir().unwrap();
        store(
            dir.path(),
            &Credentials {
                api_key: "a".into(),
                secret: "b".into(),
                user_id: uuid::Uuid::new_v4(),
            },
        )
        .unwrap();
        let cred_path = dir.path().join("credentials.enc");
        let mut bytes = std::fs::read(&cred_path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        std::fs::write(&cred_path, bytes).unwrap();
        assert!(load(dir.path()).is_err());
    }
}
