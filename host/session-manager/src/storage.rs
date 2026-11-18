//! Host persistence: the host identity (private key) and the paired-device
//! trust store, both encrypted with DPAPI for the current Windows user.
//!
//! Files live in `%LOCALAPPDATA%\LANLink\`. DPAPI ties them to the user's
//! Windows credentials: another user on the machine, or someone who copies
//! the files elsewhere, cannot decrypt them. Writes go to a temporary file
//! and are then renamed, so a crash never leaves a half-written store.

use lanlink_auth::{Identity, TrustStore};
use std::path::{Path, PathBuf};
use thiserror::Error;
use windows::core::w;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("DPAPI {0} failed: {1}")]
    Dpapi(&'static str, windows::core::Error),
    #[error("{0}")]
    Corrupt(#[from] lanlink_auth::AuthError),
}

/// Purpose label mixed into DPAPI as entropy so LANLink blobs cannot be
/// confused with other applications' DPAPI blobs.
const ENTROPY: &[u8] = b"LANLink host storage v1";

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    }
}

/// Encrypt for the current user.
pub fn protect(plain: &[u8]) -> Result<Vec<u8>, StorageError> {
    let input = blob(plain);
    let entropy = blob(ENTROPY);
    let mut out = CRYPT_INTEGER_BLOB::default();
    // SAFETY: input blobs point at live slices; DPAPI allocates `out` with
    // LocalAlloc, which we copy and free exactly once.
    unsafe {
        CryptProtectData(
            &input,
            w!("LANLink"),
            Some(&entropy),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
        .map_err(|e| StorageError::Dpapi("protect", e))?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(Some(HLOCAL(out.pbData.cast())));
        Ok(v)
    }
}

/// Decrypt a blob produced by [`protect`] for the current user.
pub fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, StorageError> {
    let input = blob(sealed);
    let entropy = blob(ENTROPY);
    let mut out = CRYPT_INTEGER_BLOB::default();
    // SAFETY: as in `protect`. The plaintext copy is zeroed before freeing.
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            Some(&entropy),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
        .map_err(|e| StorageError::Dpapi("unprotect", e))?;
        let slice = std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize);
        let v = slice.to_vec();
        slice.fill(0);
        LocalFree(Some(HLOCAL(out.pbData.cast())));
        Ok(v)
    }
}

pub struct HostStore {
    dir: PathBuf,
}

impl HostStore {
    /// `%LOCALAPPDATA%\LANLink`.
    pub fn default_dir() -> PathBuf {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("LANLink")
    }

    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|source| StorageError::Io {
            path: dir.clone(),
            source,
        })?;
        Ok(HostStore { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn read_sealed(&self, name: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let path = self.dir.join(name);
        match std::fs::read(&path) {
            Ok(b) => Ok(Some(unprotect(&b)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(StorageError::Io { path, source }),
        }
    }

    fn write_sealed(&self, name: &str, plain: &[u8]) -> Result<(), StorageError> {
        let sealed = protect(plain)?;
        let path = self.dir.join(name);
        let tmp = self.dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, &sealed).map_err(|source| StorageError::Io {
            path: tmp.clone(),
            source,
        })?;
        std::fs::rename(&tmp, &path).map_err(|source| StorageError::Io { path, source })
    }

    /// Load the host identity, creating and saving one on first run.
    pub fn load_or_create_identity(&self, label: &str) -> Result<Identity, StorageError> {
        if let Some(b) = self.read_sealed("identity.bin")? {
            return Ok(Identity::from_bytes(&b)?);
        }
        let id = Identity::generate(label)?;
        self.write_sealed("identity.bin", &id.to_bytes())?;
        tracing::info!(fingerprint = %id.fingerprint().short(), "created host identity");
        Ok(id)
    }

    pub fn load_trust(&self) -> Result<TrustStore, StorageError> {
        match self.read_sealed("trust.bin")? {
            Some(b) => Ok(TrustStore::from_bytes(&b)?),
            None => Ok(TrustStore::new()),
        }
    }

    pub fn save_trust(&self, trust: &TrustStore) -> Result<(), StorageError> {
        self.write_sealed("trust.bin", &trust.to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lanlink_auth::{Fingerprint, PairedDevice};

    #[test]
    fn dpapi_roundtrip_and_tamper_detection() {
        let sealed = protect(b"secret key material").unwrap();
        assert!(!sealed.windows(6).any(|w| w == b"secret"));
        assert_eq!(unprotect(&sealed).unwrap(), b"secret key material");
        let mut bad = sealed.clone();
        let n = bad.len();
        bad[n - 5] ^= 0xff;
        assert!(unprotect(&bad).is_err());
        assert!(unprotect(b"not a dpapi blob").is_err());
    }

    #[test]
    fn identity_and_trust_persist() {
        let dir = std::env::temp_dir().join(format!("lanlink-store-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = HostStore::open(&dir).unwrap();
        let a = store.load_or_create_identity("test").unwrap();
        let b = store.load_or_create_identity("test").unwrap();
        assert_eq!(
            a.fingerprint(),
            b.fingerprint(),
            "identity is stable across loads"
        );

        let mut trust = store.load_trust().unwrap();
        assert!(trust.devices().is_empty());
        trust.insert(PairedDevice {
            fingerprint: Fingerprint([3; 32]),
            name: "Phone".into(),
            model: "Pixel".into(),
            paired_at: 1,
            last_seen: 2,
        });
        store.save_trust(&trust).unwrap();
        assert_eq!(store.load_trust().unwrap(), trust);
        let raw = std::fs::read(dir.join("trust.bin")).unwrap();
        assert!(
            !raw.windows(5).any(|w| w == b"Phone"),
            "trust store is encrypted at rest"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
