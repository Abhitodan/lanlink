//! Pairing-link parsing and address/fingerprint argument parsing.

use crate::events::Json;
use lanlink_client::lanlink_auth::{Fingerprint, PairingUri};
use lanlink_client::lanlink_protocol::wire;
use std::net::{IpAddr, SocketAddr};

/// Parse a scanned QR link. Returns JSON or a readable reason.
pub fn parse_pairing_uri(uri: &str) -> Result<String, String> {
    let u = PairingUri::parse(uri).map_err(|e| e.to_string())?;
    // A pairing code can arrive from a web link, not only the PC's
    // screen. Never let one send this phone off the local network.
    let addresses: Vec<IpAddr> = u
        .addresses
        .iter()
        .copied()
        .filter(|a| lanlink_client::lanlink_auth::netpolicy::is_local_address(*a))
        .collect();
    if addresses.is_empty() {
        return Err(
            "This pairing code points outside your local network, so LANLink will not use it."
                .into(),
        );
    }
    let mut j = Json::object();
    j.str("name", &wire::display_name(&u.name, 48))
        .str("fingerprint", &u.fingerprint.to_hex())
        .str("code", &u.code)
        .int("port", u.port as i64)
        .str_array("addresses", addresses.iter().map(|a| a.to_string()));
    Ok(j.finish())
}

pub(crate) fn parse_addr(host: &str, port: i32) -> Result<SocketAddr, String> {
    let clean = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split('%')
        .next()
        .unwrap_or("");
    let ip: IpAddr = clean
        .parse()
        .map_err(|_| format!("\"{host}\" is not an IP address"))?;
    let port = u16::try_from(port).map_err(|_| "invalid port".to_string())?;
    if port == 0 {
        return Err("invalid port".into());
    }
    Ok(SocketAddr::new(ip, port))
}

pub(crate) fn parse_fp(hex: &str) -> Result<Option<Fingerprint>, String> {
    if hex.trim().is_empty() {
        return Ok(None);
    }
    Fingerprint::from_hex(hex)
        .map(Some)
        .map_err(|_| "invalid host identity".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ipv4_and_scoped_or_bracketed_ipv6() {
        assert!(parse_addr("192.168.1.20", 47990).is_ok());
        assert!(parse_addr("fe80::1%wlan0", 47990).is_ok());
        assert!(parse_addr("[fe80::1%wlan0]", 47990).is_ok());
        assert!(parse_addr("192.168.1.20", 0).is_err());
    }
}
