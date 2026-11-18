//! Device identity, first-time pairing and the paired-device trust store.
//!
//! Trust model in one paragraph: every device owns a long-lived key pair and
//! a self-signed certificate. Devices recognise each other by the SHA-256
//! fingerprint of that certificate, and TLS proves possession of the key.
//! Pairing is the one-time step that lets each side learn the other's
//! fingerprint without trusting the network: both run SPAKE2 over a short
//! code shown on the host's screen, with both fingerprints and the TLS
//! channel binding mixed in. See `docs/security-threat-model.md`.

pub mod identity;
pub mod netpolicy;
pub mod pairing;
pub mod trust;
pub mod uri;

pub use identity::{Fingerprint, Identity};
pub use pairing::{AttemptTicket, PairingClient, PairingHost, PairingKeys, PairingWindow};
pub use trust::{PairedDevice, TrustStore};
pub use uri::PairingUri;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("could not generate identity: {0}")]
    Generate(String),
    #[error("stored identity is corrupt")]
    CorruptIdentity,
    #[error("stored trust data is corrupt")]
    CorruptTrustStore,
    #[error("pairing code must be {0} digits")]
    BadPairingCode(usize),
    #[error("pairing message is malformed")]
    BadPairingMessage,
    #[error("pairing key confirmation failed")]
    ConfirmationFailed,
    #[error("pairing link is not valid: {0}")]
    BadUri(&'static str),
    #[error("fingerprint is not valid")]
    BadFingerprint,
}
