//! Long-lived device identity: an ECDSA P-256 key and a self-signed
//! certificate. The private key never leaves this struct except through
//! [`Identity::to_bytes`], whose output callers must protect with the
//! platform store (DPAPI on Windows, Android Keystore on the phone).

use crate::AuthError;
use std::fmt;

/// SHA-256 of a device certificate's DER encoding.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    pub fn of_cert(cert_der: &[u8]) -> Self {
        let d = ring::digest::digest(&ring::digest::SHA256, cert_der);
        let mut out = [0u8; 32];
        out.copy_from_slice(d.as_ref());
        Fingerprint(out)
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    pub fn from_hex(s: &str) -> Result<Self, AuthError> {
        let s = s.trim();
        if s.len() != 64 || !s.is_ascii() {
            return Err(AuthError::BadFingerprint);
        }
        let mut out = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let h = std::str::from_utf8(chunk).map_err(|_| AuthError::BadFingerprint)?;
            out[i] = u8::from_str_radix(h, 16).map_err(|_| AuthError::BadFingerprint)?;
        }
        Ok(Fingerprint(out))
    }

    /// Short human-readable form for UI lists: `A1B2-C3D4-E5F6-0718`.
    pub fn short(&self) -> String {
        let h = self.to_hex().to_uppercase();
        format!("{}-{}-{}-{}", &h[0..4], &h[4..8], &h[8..12], &h[12..16])
    }

    /// Constant-time equality, for comparing against attacker-supplied
    /// certificates.
    pub fn ct_eq(&self, other: &Fingerprint) -> bool {
        let mut diff = 0u8;
        for (a, b) in self.0.iter().zip(other.0.iter()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({})", self.short())
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// A device's certificate and private key.
#[derive(Clone)]
pub struct Identity {
    cert_der: Vec<u8>,
    key_pkcs8_der: Vec<u8>,
}

const BLOB_MAGIC: &[u8; 4] = b"LLID";
const BLOB_VERSION: u8 = 1;
const MAX_DER: usize = 8 * 1024;

impl Identity {
    /// Create a fresh identity. `label` appears in the certificate subject
    /// for debugging only; it carries no authority.
    pub fn generate(label: &str) -> Result<Self, AuthError> {
        let key = rcgen::KeyPair::generate().map_err(|e| AuthError::Generate(e.to_string()))?;
        let mut params = rcgen::CertificateParams::new(vec!["lanlink.local".to_string()])
            .map_err(|e| AuthError::Generate(e.to_string()))?;
        let mut dn = rcgen::DistinguishedName::new();
        let cn: String = label.chars().filter(|c| !c.is_control()).take(48).collect();
        dn.push(rcgen::DnType::CommonName, format!("LANLink {cn}"));
        params.distinguished_name = dn;
        // Validity is not what LANLink trusts (the fingerprint is), but TLS
        // stacks expect a sane window.
        params.not_before = rcgen::date_time_ymd(2024, 1, 1);
        params.not_after = rcgen::date_time_ymd(2124, 1, 1);
        let cert = params
            .self_signed(&key)
            .map_err(|e| AuthError::Generate(e.to_string()))?;
        Ok(Identity {
            cert_der: cert.der().to_vec(),
            key_pkcs8_der: key.serialize_der(),
        })
    }

    pub fn cert_der(&self) -> &[u8] {
        &self.cert_der
    }

    pub fn key_pkcs8_der(&self) -> &[u8] {
        &self.key_pkcs8_der
    }

    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of_cert(&self.cert_der)
    }

    /// Serialize for storage. **Contains the private key.**
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(13 + self.cert_der.len() + self.key_pkcs8_der.len());
        out.extend_from_slice(BLOB_MAGIC);
        out.push(BLOB_VERSION);
        out.extend_from_slice(&(self.cert_der.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.cert_der);
        out.extend_from_slice(&(self.key_pkcs8_der.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.key_pkcs8_der);
        out
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, AuthError> {
        fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8], AuthError> {
            if b.len() < n {
                return Err(AuthError::CorruptIdentity);
            }
            let (head, tail) = b.split_at(n);
            *b = tail;
            Ok(head)
        }
        fn chunk<'a>(b: &mut &'a [u8]) -> Result<&'a [u8], AuthError> {
            let len = take(b, 4)?;
            let len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
            if len == 0 || len > MAX_DER {
                return Err(AuthError::CorruptIdentity);
            }
            take(b, len)
        }
        let mut b = b;
        if take(&mut b, 4)? != BLOB_MAGIC || take(&mut b, 1)? != [BLOB_VERSION] {
            return Err(AuthError::CorruptIdentity);
        }
        let cert_der = chunk(&mut b)?.to_vec();
        let key_pkcs8_der = chunk(&mut b)?.to_vec();
        if !b.is_empty() {
            return Err(AuthError::CorruptIdentity);
        }
        Ok(Identity {
            cert_der,
            key_pkcs8_der,
        })
    }
}

impl fmt::Debug for Identity {
    /// Never prints key material.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_roundtrip_and_uniqueness() {
        let a = Identity::generate("test-a").unwrap();
        let b = Identity::generate("test-b").unwrap();
        assert_ne!(a.fingerprint(), b.fingerprint());
        let restored = Identity::from_bytes(&a.to_bytes()).unwrap();
        assert_eq!(restored.fingerprint(), a.fingerprint());
        assert_eq!(restored.key_pkcs8_der(), a.key_pkcs8_der());
    }

    #[test]
    fn debug_output_hides_key() {
        let a = Identity::generate("x").unwrap();
        let dbg = format!("{a:?}");
        assert!(dbg.contains("Fingerprint("));
        assert!(dbg.len() < 80, "{dbg}");
    }

    #[test]
    fn rejects_corrupt_blobs() {
        let a = Identity::generate("x").unwrap();
        let good = a.to_bytes();
        assert!(Identity::from_bytes(&good[..good.len() - 1]).is_err());
        let mut extra = good.clone();
        extra.push(0);
        assert!(Identity::from_bytes(&extra).is_err());
        let mut bad_magic = good.clone();
        bad_magic[0] ^= 1;
        assert!(Identity::from_bytes(&bad_magic).is_err());
        assert!(Identity::from_bytes(&[]).is_err());
        // Length prefix far larger than the data must not allocate or panic.
        let mut huge = good[..5].to_vec();
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(Identity::from_bytes(&huge).is_err());
    }

    #[test]
    fn fingerprint_hex_roundtrip() {
        let fp = Fingerprint([0xab; 32]);
        assert_eq!(Fingerprint::from_hex(&fp.to_hex()).unwrap(), fp);
        assert_eq!(fp.short(), "ABAB-ABAB-ABAB-ABAB");
        assert!(Fingerprint::from_hex("zz").is_err());
        assert!(Fingerprint::from_hex(&"g".repeat(64)).is_err());
        assert!(fp.ct_eq(&fp));
        assert!(!fp.ct_eq(&Fingerprint([0xac; 32])));
    }
}
