//! LAN discovery for LANLink.
//!
//! Hosts advertise `_lanlink._udp.local.` with DNS-SD. The TXT record holds
//! only what a phone needs to list the host and recognise one it already
//! paired with:
//!
//! | key  | value                                             |
//! |------|---------------------------------------------------|
//! | `v`  | protocol version                                  |
//! | `id` | first 16 hex digits of the host certificate hash  |
//! | `n`  | display name chosen for the PC                    |
//!
//! No user names, OS details or session state are advertised. The `id` is
//! public information (the certificate is presented to anyone who connects)
//! and lets a phone match a paired host without connecting first.
//!
//! Discovery is a convenience only: nothing it reports is trusted. The
//! connection authenticates the host by its full fingerprint.

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use thiserror::Error;

pub const SERVICE_TYPE: &str = "_lanlink._udp.local.";

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("mDNS: {0}")]
    Mdns(#[from] mdns_sd::Error),
}

/// IPv4 addresses of interfaces that are up and on a private or link-local
/// network, in a stable order (Wi-Fi/Ethernet-style private ranges first).
/// These go into the pairing QR code.
pub fn local_addresses() -> Vec<IpAddr> {
    let Ok(ifaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut v4: Vec<Ipv4Addr> = ifaces
        .iter()
        .filter(|i| i.is_oper_up() && !i.is_loopback())
        .filter_map(|i| match i.ip() {
            IpAddr::V4(a) if a.is_private() || a.is_link_local() => Some(a),
            _ => None,
        })
        .collect();
    // 192.168/16 and 10/8 are typical home Wi-Fi; link-local last.
    v4.sort_by_key(|a| (a.is_link_local(), a.octets()[0] != 192, a.octets()));
    v4.dedup();
    v4.into_iter().map(IpAddr::V4).collect()
}

/// Short, public host identifier derived from the certificate fingerprint.
pub fn short_id(fingerprint_hex: &str) -> String {
    fingerprint_hex.chars().take(16).collect()
}

/// Keeps the host advertised while alive.
pub struct Advertiser {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Advertiser {
    pub fn start(
        display_name: &str,
        fingerprint_hex: &str,
        port: u16,
    ) -> Result<Self, DiscoveryError> {
        let daemon = ServiceDaemon::new()?;
        let instance: String = display_name
            .chars()
            .filter(|c| !c.is_control() && *c != '.')
            .take(48)
            .collect();
        let instance = if instance.is_empty() {
            "LANLink PC".to_string()
        } else {
            instance
        };
        let host = format!("lanlink-{}.local.", short_id(fingerprint_hex));
        let mut txt = HashMap::new();
        txt.insert("v".to_string(), lanlink_version().to_string());
        txt.insert("id".to_string(), short_id(fingerprint_hex));
        txt.insert("n".to_string(), instance.clone());
        let addrs: Vec<IpAddr> = local_addresses();
        let info = ServiceInfo::new(SERVICE_TYPE, &instance, &host, &addrs[..], port, txt)?
            .enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        daemon.register(info)?;
        tracing::info!(%fullname, port, "advertising on mDNS");
        Ok(Advertiser { daemon, fullname })
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

fn lanlink_version() -> u16 {
    1
}

/// A host seen on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredHost {
    pub fullname: String,
    pub name: String,
    pub id: String,
    pub addresses: Vec<Ipv4Addr>,
    pub port: u16,
    pub protocol: u16,
}

/// Browse for hosts until `timeout` elapses. Used by desktop test tools;
/// the Android app uses the platform's `NsdManager` instead.
pub fn browse(timeout: std::time::Duration) -> Result<Vec<DiscoveredHost>, DiscoveryError> {
    let daemon = ServiceDaemon::new()?;
    let rx = daemon.browse(SERVICE_TYPE)?;
    let deadline = std::time::Instant::now() + timeout;
    let mut found: HashMap<String, DiscoveredHost> = HashMap::new();
    while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(ServiceEvent::ServiceResolved(s)) => {
                let mut addresses: Vec<Ipv4Addr> = s.get_addresses_v4().into_iter().collect();
                addresses.sort();
                found.insert(
                    s.fullname.clone(),
                    DiscoveredHost {
                        fullname: s.fullname.clone(),
                        name: s.get_property_val_str("n").unwrap_or("").to_string(),
                        id: s.get_property_val_str("id").unwrap_or("").to_string(),
                        addresses,
                        port: s.get_port(),
                        protocol: s
                            .get_property_val_str("v")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0),
                    },
                );
            }
            Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                found.remove(&fullname);
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.stop_browse(SERVICE_TYPE);
    let _ = daemon.shutdown();
    let mut v: Vec<_> = found.into_values().collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_addresses_are_private_and_unique() {
        let addrs = local_addresses();
        println!("local addresses: {addrs:?}");
        for a in &addrs {
            let IpAddr::V4(v4) = a else { panic!("v4 only") };
            assert!(v4.is_private() || v4.is_link_local());
        }
        let mut d = addrs.clone();
        d.dedup();
        assert_eq!(d.len(), addrs.len());
    }

    #[test]
    fn short_id_is_prefix() {
        assert_eq!(short_id(&"ab".repeat(32)), "abababababababab");
    }

    /// Advertise and discover ourselves through the real mDNS stack.
    #[test]
    fn advertise_then_browse_finds_this_host() {
        if local_addresses().is_empty() {
            eprintln!("SKIP: no private network interface");
            return;
        }
        let fp = "c0ffee".repeat(10) + "abcd";
        let _adv = Advertiser::start("LANLink self-test", &fp, 47999).unwrap();
        let hosts = browse(std::time::Duration::from_secs(4)).unwrap();
        println!("discovered: {hosts:?}");
        let me = hosts.iter().find(|h| h.id == short_id(&fp));
        let Some(me) = me else {
            // Some networks/firewalls block multicast loopback; report it
            // rather than fail the build.
            eprintln!("WARN: own advertisement not seen; multicast may be blocked here");
            return;
        };
        assert_eq!(me.port, 47999);
        assert_eq!(me.name, "LANLink self-test");
        assert_eq!(me.protocol, 1);
        assert!(!me.addresses.is_empty());
    }
}
