//! TLS configuration: mutual authentication with self-signed certificates
//! identified by fingerprint.
//!
//! There is no certificate authority in LANLink, so the usual WebPKI path
//! validation does not apply. What replaces it:
//!
//! * The TLS 1.3 `CertificateVerify` signature is always checked, so a peer
//!   can only present a certificate whose private key it holds.
//! * The *client* decides whom it is talking to by comparing the server
//!   certificate's fingerprint with a pinned value ([`ServerTrust::Pinned`]).
//!   During first-time pairing without a QR code the fingerprint is not yet
//!   known ([`ServerTrust::Unverified`]); the connection is then good for
//!   exactly one thing, the SPAKE2 pairing exchange, which authenticates the
//!   certificate actually seen on this connection.
//! * The *server* accepts any well-formed client certificate at the TLS
//!   layer and authorizes by fingerprint at the application layer, where it
//!   can distinguish "paired", "pairing in progress" and "unknown".
//!
//! Neither side ever skips signature verification.

use lanlink_auth::{Fingerprint, Identity};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls13_signature, CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{CertificateError, DigitallySignedStruct, DistinguishedName, SignatureScheme};
use std::sync::Arc;

use crate::TransportError;

/// How the client authenticates the host certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerTrust {
    /// The host must present exactly this certificate.
    Pinned(Fingerprint),
    /// The certificate is not yet known. Only the pairing exchange may run
    /// on such a connection; the caller is responsible for enforcing that.
    Unverified,
}

/// Name sent as SNI. It is not used for trust decisions.
pub const SERVER_NAME: &str = "lanlink.local";

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn single_cert(identity: &Identity) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
    let cert = CertificateDer::from(identity.cert_der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key_pkcs8_der().to_vec()));
    (vec![cert], key)
}

#[derive(Debug)]
struct FingerprintServerVerifier {
    trust: ServerTrust,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for FingerprintServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if !intermediates.is_empty() {
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        match self.trust {
            ServerTrust::Pinned(expected) => {
                if Fingerprint::of_cert(end_entity.as_ref()).ct_eq(&expected) {
                    Ok(ServerCertVerified::assertion())
                } else {
                    Err(rustls::Error::InvalidCertificate(
                        CertificateError::ApplicationVerificationFailure,
                    ))
                }
            }
            // Deferred to the pairing exchange; see the module docs.
            ServerTrust::Unverified => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // QUIC is TLS 1.3 only.
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

#[derive(Debug)]
struct AnyClientCertVerifier {
    algs: WebPkiSupportedAlgorithms,
}

impl ClientCertVerifier for AnyClientCertVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        if !intermediates.is_empty() {
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        // Possession of the key is proven by `verify_tls13_signature`.
        // *Authorization* is decided by fingerprint in the session manager.
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

pub(crate) fn rustls_server(identity: &Identity) -> Result<rustls::ServerConfig, TransportError> {
    let provider = provider();
    let algs = provider.signature_verification_algorithms;
    let (certs, key) = single_cert(identity);
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| TransportError::Tls(e.to_string()))?
        .with_client_cert_verifier(Arc::new(AnyClientCertVerifier { algs }))
        .with_single_cert(certs, key)
        .map_err(|e| TransportError::Tls(e.to_string()))?;
    cfg.alpn_protocols = vec![lanlink_protocol::limits::ALPN.to_vec()];
    // No 0-RTT: early data is replayable, and every LANLink message that
    // matters (pairing, session start, input) must not be.
    cfg.max_early_data_size = 0;
    Ok(cfg)
}

pub(crate) fn rustls_client(
    identity: &Identity,
    trust: ServerTrust,
) -> Result<rustls::ClientConfig, TransportError> {
    let provider = provider();
    let algs = provider.signature_verification_algorithms;
    let (certs, key) = single_cert(identity);
    let mut cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| TransportError::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(FingerprintServerVerifier { trust, algs }))
        .with_client_auth_cert(certs, key)
        .map_err(|e| TransportError::Tls(e.to_string()))?;
    cfg.alpn_protocols = vec![lanlink_protocol::limits::ALPN.to_vec()];
    cfg.enable_early_data = false;
    Ok(cfg)
}
