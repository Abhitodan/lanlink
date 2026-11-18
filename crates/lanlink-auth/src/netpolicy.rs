//! Which peer addresses the host talks to at all.
//!
//! This is defence in depth, not the security boundary: a private address
//! says nothing about who is behind it (VPNs, guest networks, a compromised
//! device on the LAN). Authentication by certificate fingerprint is what
//! decides access. The filter only keeps the host from answering handshakes
//! from the public Internet if a port is ever forwarded to it by mistake.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub fn is_local_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_local_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local_v4(v4);
            }
            let seg0 = v6.segments()[0];
            v6 == Ipv6Addr::LOCALHOST
                || (seg0 & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (seg0 & 0xffc0) == 0xfe80 // link local fe80::/10
        }
    }
}

fn is_local_v4(v4: Ipv4Addr) -> bool {
    v4.is_loopback() || v4.is_private() || v4.is_link_local()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_addresses() {
        for a in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.50",
            "169.254.10.10",
            "::1",
            "fd12::1",
            "fe80::1234",
            "::ffff:192.168.0.2",
        ] {
            assert!(is_local_address(a.parse().unwrap()), "{a}");
        }
        for a in [
            "8.8.8.8",
            "172.32.0.1",
            "100.64.0.1",
            "2001:4860::8888",
            "::ffff:8.8.8.8",
            "0.0.0.0",
        ] {
            assert!(!is_local_address(a.parse().unwrap()), "{a}");
        }
    }
}
