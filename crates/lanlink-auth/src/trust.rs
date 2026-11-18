//! Paired-device records.
//!
//! The store is plain data with a stable text encoding. It holds no secrets
//! (fingerprints are public), but its *integrity* matters: whoever can add a
//! line can control the PC. The host therefore keeps it in the user profile,
//! protected by DPAPI together with the identity key.

use crate::identity::Fingerprint;
use crate::AuthError;

/// Upper bound on stored devices; keeps a corrupt or hostile file from
/// growing without limit.
pub const MAX_DEVICES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedDevice {
    pub fingerprint: Fingerprint,
    pub name: String,
    pub model: String,
    /// Seconds since the Unix epoch.
    pub paired_at: u64,
    /// Seconds since the Unix epoch; 0 if never connected after pairing.
    pub last_seen: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustStore {
    devices: Vec<PairedDevice>,
}

const HEADER: &str = "lanlink-trust v1";

impl TrustStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn devices(&self) -> &[PairedDevice] {
        &self.devices
    }

    pub fn get(&self, fp: &Fingerprint) -> Option<&PairedDevice> {
        // Constant-time comparison per entry: `fp` comes from the network.
        self.devices.iter().find(|d| d.fingerprint.ct_eq(fp))
    }

    pub fn contains(&self, fp: &Fingerprint) -> bool {
        self.get(fp).is_some()
    }

    /// Add or replace a device. The oldest entry is evicted when the store
    /// is full.
    pub fn insert(&mut self, device: PairedDevice) {
        self.devices
            .retain(|d| !d.fingerprint.ct_eq(&device.fingerprint));
        if self.devices.len() >= MAX_DEVICES {
            if let Some((idx, _)) = self
                .devices
                .iter()
                .enumerate()
                .min_by_key(|(_, d)| d.last_seen.max(d.paired_at))
            {
                self.devices.remove(idx);
            }
        }
        self.devices.push(device);
    }

    /// Remove a device. Returns `true` if it was present.
    pub fn revoke(&mut self, fp: &Fingerprint) -> bool {
        let before = self.devices.len();
        self.devices.retain(|d| !d.fingerprint.ct_eq(fp));
        self.devices.len() != before
    }

    pub fn clear(&mut self) {
        self.devices.clear();
    }

    pub fn touch(&mut self, fp: &Fingerprint, now_unix: u64) {
        if let Some(d) = self.devices.iter_mut().find(|d| d.fingerprint.ct_eq(fp)) {
            d.last_seen = now_unix;
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut s = String::from(HEADER);
        s.push('\n');
        for d in &self.devices {
            s.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\n",
                d.fingerprint.to_hex(),
                d.paired_at,
                d.last_seen,
                escape(&d.name),
                escape(&d.model)
            ));
        }
        s.into_bytes()
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, AuthError> {
        let s = std::str::from_utf8(b).map_err(|_| AuthError::CorruptTrustStore)?;
        let mut lines = s.lines();
        if lines.next() != Some(HEADER) {
            return Err(AuthError::CorruptTrustStore);
        }
        let mut store = TrustStore::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            if store.devices.len() >= MAX_DEVICES {
                return Err(AuthError::CorruptTrustStore);
            }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() != 5 {
                return Err(AuthError::CorruptTrustStore);
            }
            let fingerprint =
                Fingerprint::from_hex(parts[0]).map_err(|_| AuthError::CorruptTrustStore)?;
            let paired_at = parts[1].parse().map_err(|_| AuthError::CorruptTrustStore)?;
            let last_seen = parts[2].parse().map_err(|_| AuthError::CorruptTrustStore)?;
            store.devices.push(PairedDevice {
                fingerprint,
                name: unescape(parts[3])?,
                model: unescape(parts[4])?,
                paired_at,
                last_seen,
            });
        }
        Ok(store)
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().take(128) {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> Result<String, AuthError> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            _ => return Err(AuthError::CorruptTrustStore),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(n: u8, name: &str) -> PairedDevice {
        PairedDevice {
            fingerprint: Fingerprint([n; 32]),
            name: name.into(),
            model: "Pixel\t9".into(),
            paired_at: 1_700_000_000 + n as u64,
            last_seen: 0,
        }
    }

    #[test]
    fn roundtrip_with_awkward_names() {
        let mut s = TrustStore::new();
        s.insert(dev(1, "Sam's phone"));
        s.insert(dev(2, "tab\tnew\nline\\slash"));
        s.insert(dev(3, "日本語 📱"));
        let restored = TrustStore::from_bytes(&s.to_bytes()).unwrap();
        assert_eq!(restored, s);
    }

    #[test]
    fn revoke_removes_only_that_device() {
        let mut s = TrustStore::new();
        s.insert(dev(1, "a"));
        s.insert(dev(2, "b"));
        assert!(s.revoke(&Fingerprint([1; 32])));
        assert!(!s.revoke(&Fingerprint([1; 32])));
        assert!(!s.contains(&Fingerprint([1; 32])));
        assert!(s.contains(&Fingerprint([2; 32])));
    }

    #[test]
    fn insert_replaces_same_fingerprint() {
        let mut s = TrustStore::new();
        s.insert(dev(1, "old"));
        s.insert(dev(1, "new"));
        assert_eq!(s.devices().len(), 1);
        assert_eq!(s.devices()[0].name, "new");
    }

    #[test]
    fn store_is_bounded() {
        let mut s = TrustStore::new();
        for i in 0..=(MAX_DEVICES as u8 + 5) {
            s.insert(dev(i, "x"));
        }
        assert_eq!(s.devices().len(), MAX_DEVICES);
        // The oldest entries were evicted.
        assert!(!s.contains(&Fingerprint([0; 32])));
    }

    #[test]
    fn rejects_corrupt_input() {
        assert!(TrustStore::from_bytes(b"").is_err());
        assert!(TrustStore::from_bytes(b"something else\n").is_err());
        assert!(TrustStore::from_bytes(b"lanlink-trust v1\nnot-a-record\n").is_err());
        assert!(TrustStore::from_bytes(&[0xff, 0xfe]).is_err());
        let bad_escape = format!("{HEADER}\n{}\t1\t2\tbad\\q\tm\n", "ab".repeat(32));
        assert!(TrustStore::from_bytes(bad_escape.as_bytes()).is_err());
        assert!(TrustStore::from_bytes(format!("{HEADER}\n").as_bytes())
            .unwrap()
            .devices()
            .is_empty());
    }
}
