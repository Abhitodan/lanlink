//! The pairing link encoded in the QR code shown by the host.
//!
//! ```text
//! lanlink://pair?v=1&a=192.168.1.20,10.0.0.5&p=47990&f=<64 hex>&c=12345678&n=DESKTOP
//! ```
//!
//! Everything in it is either public (addresses, port, certificate
//! fingerprint, name) or short-lived (the pairing code, valid for one
//! pairing window). It never contains a key or a reusable credential.

use crate::identity::Fingerprint;
use crate::pairing::normalize_code;
use crate::AuthError;
use std::net::IpAddr;

const MAX_URI_LEN: usize = 600;
const MAX_ADDRS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingUri {
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    pub fingerprint: Fingerprint,
    pub code: String,
    pub name: String,
}

impl PairingUri {
    pub fn encode(&self) -> String {
        let addrs: Vec<String> = self
            .addresses
            .iter()
            .take(MAX_ADDRS)
            .map(|a| a.to_string())
            .collect();
        format!(
            "lanlink://pair?v=1&a={}&p={}&f={}&c={}&n={}",
            addrs.join(","),
            self.port,
            self.fingerprint.to_hex(),
            self.code,
            percent_encode(&self.name)
        )
    }

    pub fn parse(s: &str) -> Result<Self, AuthError> {
        let s = s.trim();
        if s.len() > MAX_URI_LEN {
            return Err(AuthError::BadUri("too long"));
        }
        let query = s
            .strip_prefix("lanlink://pair?")
            .ok_or(AuthError::BadUri("not a LANLink pairing code"))?;

        let mut version = None;
        let mut addresses = Vec::new();
        let mut port = None;
        let mut fingerprint = None;
        let mut code = None;
        let mut name = String::new();

        for pair in query.split('&') {
            let (k, v) = pair
                .split_once('=')
                .ok_or(AuthError::BadUri("malformed field"))?;
            match k {
                "v" => version = Some(v),
                "a" => {
                    for a in v.split(',').filter(|a| !a.is_empty()) {
                        if addresses.len() >= MAX_ADDRS {
                            return Err(AuthError::BadUri("too many addresses"));
                        }
                        let ip: IpAddr = a.parse().map_err(|_| AuthError::BadUri("address"))?;
                        addresses.push(ip);
                    }
                }
                "p" => port = Some(v.parse::<u16>().map_err(|_| AuthError::BadUri("port"))?),
                "f" => {
                    fingerprint = Some(
                        Fingerprint::from_hex(v).map_err(|_| AuthError::BadUri("fingerprint"))?,
                    )
                }
                "c" => code = Some(normalize_code(v).map_err(|_| AuthError::BadUri("code"))?),
                "n" => name = percent_decode(v).ok_or(AuthError::BadUri("name"))?,
                // Unknown fields are ignored so newer hosts can add hints.
                _ => {}
            }
        }

        if version != Some("1") {
            return Err(AuthError::BadUri("unsupported version"));
        }
        let port = port.ok_or(AuthError::BadUri("port"))?;
        if port == 0 {
            return Err(AuthError::BadUri("port"));
        }
        if addresses.is_empty() {
            return Err(AuthError::BadUri("address"));
        }
        Ok(PairingUri {
            addresses,
            port,
            fingerprint: fingerprint.ok_or(AuthError::BadUri("fingerprint"))?,
            code: code.ok_or(AuthError::BadUri("code"))?,
            name: name.chars().filter(|c| !c.is_control()).take(64).collect(),
        })
    }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PairingUri {
        PairingUri {
            addresses: vec!["192.168.1.50".parse().unwrap(), "fe80::1".parse().unwrap()],
            port: 47990,
            fingerprint: Fingerprint([0x5a; 32]),
            code: "01234567".into(),
            name: "Sam's Laptop & more".into(),
        }
    }

    #[test]
    fn roundtrip() {
        let u = sample();
        let enc = u.encode();
        assert!(enc.starts_with("lanlink://pair?v=1&a=192.168.1.50,fe80::1&p=47990&f=5a5a"));
        assert_eq!(PairingUri::parse(&enc).unwrap(), u);
    }

    #[test]
    fn rejects_malformed() {
        let good = sample().encode();
        assert!(PairingUri::parse("https://example.com").is_err());
        assert!(PairingUri::parse(&good.replace("v=1", "v=2")).is_err());
        assert!(PairingUri::parse(&good.replace("p=47990", "p=0")).is_err());
        assert!(PairingUri::parse(&good.replace("p=47990", "p=99999")).is_err());
        assert!(PairingUri::parse(&good.replace("c=01234567", "c=0123")).is_err());
        assert!(PairingUri::parse(&good.replace("192.168.1.50", "not-an-ip")).is_err());
        assert!(PairingUri::parse(&good.replace("&f=", "&x=")).is_err());
        assert!(PairingUri::parse(&format!("{good}{}", "&z=".repeat(400))).is_err());
        assert!(PairingUri::parse(&good.replace("n=Sam", "n=%ZZ")).is_err());
    }

    #[test]
    fn ignores_unknown_fields() {
        let u = format!("{}&future=1", sample().encode());
        assert_eq!(PairingUri::parse(&u).unwrap(), sample());
    }
}
