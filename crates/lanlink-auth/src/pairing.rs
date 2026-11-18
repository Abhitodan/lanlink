//! First-time pairing.
//!
//! Protocol (runs inside the TLS 1.3 QUIC connection, on the control stream):
//!
//! ```text
//! client                                             host
//!   PairRequest  { spake_a }            ─────────────▶
//!                ◀─────────────  PairResponse { spake_b }
//!   PairConfirm  { mac_client }         ─────────────▶
//!                              (host user approves on screen)
//!                ◀─────────────  PairResult { accepted, mac_host }
//! ```
//!
//! * SPAKE2 (RFC 9382 construction, `spake2` crate, Ed25519 group) turns the
//!   short code into a strong shared key. An attacker who does not know the
//!   code gets one online guess per attempt and learns nothing offline.
//! * Both certificate fingerprints are the SPAKE2 identities, and the TLS
//!   exporter value is mixed into key confirmation. A machine-in-the-middle
//!   necessarily presents different certificates and sits on two different
//!   TLS sessions, so its confirmation cannot verify.
//! * The host limits attempts per pairing window and the window expires, so
//!   guessing the code online is bounded ([`PairingWindow`]).
//!
//! Nothing here is a new cryptographic primitive: SPAKE2 for the PAKE,
//! HMAC-SHA-256 for key confirmation.

use crate::identity::Fingerprint;
use crate::AuthError;
use ring::hmac;
use spake2::{Ed25519Group, Identity as SpakeId, Password, Spake2};
use std::time::{Duration, Instant};

/// Digits in a pairing code.
pub const CODE_DIGITS: usize = 8;
/// How long a pairing window stays open.
pub const WINDOW_TTL: Duration = Duration::from_secs(120);
/// Wrong codes tolerated before the window closes.
pub const MAX_ATTEMPTS: u8 = 3;
/// TLS exporter label used for channel binding.
pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-lanlink-pairing-v1";
/// Length of the channel-binding value.
pub const BINDING_LEN: usize = 32;

/// Values both sides derive after a successful SPAKE2 exchange.
pub struct PairingKeys {
    /// Sent by the client in `PairConfirm`.
    pub client_mac: [u8; 32],
    /// Sent by the host in `PairResult`.
    pub host_mac: [u8; 32],
    /// Six-digit confirmation code shown on both screens so the user can see
    /// that the phone and the PC agree on who they are talking to.
    pub confirmation_code: String,
    key: hmac::Key,
    transcript: [u8; 32],
}

impl PairingKeys {
    fn derive(
        shared: &[u8],
        client_fp: &Fingerprint,
        host_fp: &Fingerprint,
        binding: &[u8; BINDING_LEN],
    ) -> Self {
        let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
        ctx.update(b"lanlink-pairing-v1");
        ctx.update(&client_fp.0);
        ctx.update(&host_fp.0);
        ctx.update(binding);
        let mut transcript = [0u8; 32];
        transcript.copy_from_slice(ctx.finish().as_ref());

        let key = hmac::Key::new(hmac::HMAC_SHA256, shared);
        let tag = |label: &[u8]| -> [u8; 32] {
            let mut c = hmac::Context::with_key(&key);
            c.update(label);
            c.update(&transcript);
            let mut out = [0u8; 32];
            out.copy_from_slice(c.sign().as_ref());
            out
        };
        let sas = tag(b"confirmation-code");
        let n = u32::from_be_bytes([sas[0], sas[1], sas[2], sas[3]]) % 1_000_000;
        PairingKeys {
            client_mac: tag(b"client"),
            host_mac: tag(b"host"),
            confirmation_code: format!("{n:06}"),
            key,
            transcript,
        }
    }

    fn verify(&self, label: &[u8], mac: &[u8; 32]) -> Result<(), AuthError> {
        let mut data = Vec::with_capacity(label.len() + 32);
        data.extend_from_slice(label);
        data.extend_from_slice(&self.transcript);
        // `ring::hmac::verify` compares in constant time.
        hmac::verify(&self.key, &data, mac).map_err(|_| AuthError::ConfirmationFailed)
    }

    /// Host side: check the client's `PairConfirm`.
    pub fn verify_client(&self, mac: &[u8; 32]) -> Result<(), AuthError> {
        self.verify(b"client", mac)
    }

    /// Client side: check the host's `PairResult`.
    pub fn verify_host(&self, mac: &[u8; 32]) -> Result<(), AuthError> {
        self.verify(b"host", mac)
    }
}

/// Strip separators from a typed code and check its shape.
pub fn normalize_code(input: &str) -> Result<String, AuthError> {
    let digits: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    if digits.len() != CODE_DIGITS || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AuthError::BadPairingCode(CODE_DIGITS));
    }
    Ok(digits)
}

/// Client half of the exchange.
pub struct PairingClient {
    state: Spake2<Ed25519Group>,
    client_fp: Fingerprint,
    host_fp: Fingerprint,
}

impl PairingClient {
    /// Returns the state and the SPAKE2 message for `PairRequest`.
    pub fn start(
        code: &str,
        client_fp: Fingerprint,
        host_fp: Fingerprint,
    ) -> Result<(Self, Vec<u8>), AuthError> {
        let code = normalize_code(code)?;
        let (state, msg) = Spake2::<Ed25519Group>::start_a(
            &Password::new(code.as_bytes()),
            &SpakeId::new(&client_fp.0),
            &SpakeId::new(&host_fp.0),
        );
        Ok((
            PairingClient {
                state,
                client_fp,
                host_fp,
            },
            msg,
        ))
    }

    pub fn finish(
        self,
        host_msg: &[u8],
        binding: &[u8; BINDING_LEN],
    ) -> Result<PairingKeys, AuthError> {
        let shared = self
            .state
            .finish(host_msg)
            .map_err(|_| AuthError::BadPairingMessage)?;
        Ok(PairingKeys::derive(
            &shared,
            &self.client_fp,
            &self.host_fp,
            binding,
        ))
    }
}

/// Host half of the exchange.
pub struct PairingHost;

impl PairingHost {
    /// Process `PairRequest`. Returns the keys and the SPAKE2 message for
    /// `PairResponse`. The caller must still verify `PairConfirm` before
    /// trusting anything.
    pub fn respond(
        code: &str,
        client_fp: Fingerprint,
        host_fp: Fingerprint,
        client_msg: &[u8],
        binding: &[u8; BINDING_LEN],
    ) -> Result<(PairingKeys, Vec<u8>), AuthError> {
        let code = normalize_code(code)?;
        let (state, msg) = Spake2::<Ed25519Group>::start_b(
            &Password::new(code.as_bytes()),
            &SpakeId::new(&client_fp.0),
            &SpakeId::new(&host_fp.0),
        );
        let shared = state
            .finish(client_msg)
            .map_err(|_| AuthError::BadPairingMessage)?;
        Ok((
            PairingKeys::derive(&shared, &client_fp, &host_fp, binding),
            msg,
        ))
    }
}

/// A host-side pairing window: one short-lived code, a few attempts.
///
/// The window is the only time an unknown device can do anything at all, and
/// it is opened explicitly by the host user.
///
/// Attempts are counted when a device *starts* an exchange
/// ([`begin_attempt`](Self::begin_attempt)), not when its confirmation
/// fails: SPAKE2 gives one code guess per started exchange, so counting at
/// the end would let many concurrent connections each get a guess while the
/// window still looked open.
#[derive(Debug)]
pub struct PairingWindow {
    id: u64,
    code: String,
    expires: Instant,
    attempts_left: u8,
    closed: bool,
}

/// Proof that an attempt was counted against a particular window. The
/// confirmation step must present it, so an exchange started in a window
/// that was since closed, expired or replaced can never complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptTicket(u64);

fn next_window_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl PairingWindow {
    /// Open a window with a fresh random code.
    pub fn open(now: Instant) -> Result<Self, AuthError> {
        Ok(PairingWindow {
            id: next_window_id(),
            code: random_code()?,
            expires: now + WINDOW_TTL,
            attempts_left: MAX_ATTEMPTS,
            closed: false,
        })
    }

    /// Open a window with a caller-supplied code (tests and automation).
    pub fn with_code(code: &str, now: Instant) -> Result<Self, AuthError> {
        Ok(PairingWindow {
            id: next_window_id(),
            code: normalize_code(code)?,
            expires: now + WINDOW_TTL,
            attempts_left: MAX_ATTEMPTS,
            closed: false,
        })
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn expires(&self) -> Instant {
        self.expires
    }

    pub fn is_open(&self, now: Instant) -> bool {
        self.attempts_left > 0 && now < self.expires
    }

    pub fn attempts_left(&self) -> u8 {
        self.attempts_left
    }

    /// Start one pairing exchange: uses up an attempt and returns the code
    /// plus a ticket for [`ticket_valid`](Self::ticket_valid). `None` when
    /// the window is expired or out of attempts.
    pub fn begin_attempt(&mut self, now: Instant) -> Option<(AttemptTicket, String)> {
        if !self.is_open(now) {
            return None;
        }
        self.attempts_left -= 1;
        Some((AttemptTicket(self.id), self.code.clone()))
    }

    /// Whether an exchange started with `ticket` may still complete: same
    /// window, not expired, not closed by a successful pairing. (Using up the
    /// last attempt does not invalidate tickets already handed out.)
    pub fn ticket_valid(&self, ticket: AttemptTicket, now: Instant) -> bool {
        self.id == ticket.0 && now < self.expires && !self.closed
    }

    /// Give back the attempt of an exchange that failed before any code
    /// comparison was possible (a malformed SPAKE2 message reveals nothing),
    /// so junk requests cannot close the window on the user.
    pub fn refund(&mut self, ticket: AttemptTicket) {
        if self.id == ticket.0 && !self.closed && self.attempts_left < MAX_ATTEMPTS {
            self.attempts_left += 1;
        }
    }

    /// Whether `ticket` was handed out by this window (expired or not).
    pub fn issued(&self, ticket: AttemptTicket) -> bool {
        self.id == ticket.0
    }

    /// Every attempt has been started; nothing more can happen in this window
    /// except exchanges already in flight.
    pub fn exhausted(&self) -> bool {
        self.attempts_left == 0
    }

    /// A code is single-use: close the window once a device has paired.
    pub fn close(&mut self) {
        self.attempts_left = 0;
        self.closed = true;
    }
}

/// Uniform random decimal code from the OS CSPRNG (rejection sampling, so no
/// modulo bias).
fn random_code() -> Result<String, AuthError> {
    use ring::rand::SecureRandom;
    let rng = ring::rand::SystemRandom::new();
    let modulus: u32 = 10u32.pow(CODE_DIGITS as u32);
    let limit = u32::MAX - (u32::MAX % modulus);
    loop {
        let mut b = [0u8; 4];
        rng.fill(&mut b)
            .map_err(|_| AuthError::Generate("system random unavailable".into()))?;
        let v = u32::from_le_bytes(b);
        if v < limit {
            return Ok(format!("{:0width$}", v % modulus, width = CODE_DIGITS));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: Fingerprint = Fingerprint([1; 32]);
    const H: Fingerprint = Fingerprint([2; 32]);
    const BIND: [u8; 32] = [9; 32];

    fn run(
        client_code: &str,
        host_code: &str,
        client_view: (Fingerprint, Fingerprint, [u8; 32]),
        host_view: (Fingerprint, Fingerprint, [u8; 32]),
    ) -> (PairingKeys, PairingKeys) {
        let (client, msg_a) =
            PairingClient::start(client_code, client_view.0, client_view.1).unwrap();
        let (host_keys, msg_b) =
            PairingHost::respond(host_code, host_view.0, host_view.1, &msg_a, &host_view.2)
                .unwrap();
        let client_keys = client.finish(&msg_b, &client_view.2).unwrap();
        (client_keys, host_keys)
    }

    #[test]
    fn matching_code_confirms_both_ways() {
        let (c, h) = run("12345678", "1234-5678", (C, H, BIND), (C, H, BIND));
        assert!(h.verify_client(&c.client_mac).is_ok());
        assert!(c.verify_host(&h.host_mac).is_ok());
        assert_eq!(c.confirmation_code, h.confirmation_code);
        assert_eq!(c.confirmation_code.len(), 6);
        // The two directions are domain-separated.
        assert!(h.verify_client(&h.host_mac).is_err());
    }

    #[test]
    fn wrong_code_fails_confirmation() {
        let (c, h) = run("12345678", "12345679", (C, H, BIND), (C, H, BIND));
        assert!(h.verify_client(&c.client_mac).is_err());
        assert!(c.verify_host(&h.host_mac).is_err());
    }

    #[test]
    fn mitm_with_different_certificate_fails_even_with_right_code() {
        // The client believes it talks to H; the host sees certificate M
        // (the attacker's) instead of C.
        let m = Fingerprint([7; 32]);
        let (c, h) = run("12345678", "12345678", (C, H, BIND), (m, H, BIND));
        assert!(h.verify_client(&c.client_mac).is_err());
        assert!(c.verify_host(&h.host_mac).is_err());
    }

    #[test]
    fn different_tls_session_fails() {
        let (c, h) = run("12345678", "12345678", (C, H, BIND), (C, H, [8; 32]));
        assert!(h.verify_client(&c.client_mac).is_err());
        assert!(c.verify_host(&h.host_mac).is_err());
    }

    #[test]
    fn garbage_spake_message_is_an_error_not_a_panic() {
        let (client, _) = PairingClient::start("12345678", C, H).unwrap();
        assert!(client.finish(&[0u8; 5], &BIND).is_err());
        assert!(PairingHost::respond("12345678", C, H, &[0xff; 33], &BIND).is_err());
        assert!(PairingHost::respond("12345678", C, H, &[], &BIND).is_err());
    }

    #[test]
    fn code_shape_is_validated() {
        assert!(normalize_code("1234 5678").is_ok());
        assert!(normalize_code("1234567").is_err());
        assert!(normalize_code("12345678a").is_err());
        assert!(normalize_code("١٢٣٤٥٦٧٨").is_err());
    }

    #[test]
    fn window_expires_and_limits_attempts() {
        let t0 = Instant::now();
        let mut w = PairingWindow::open(t0).unwrap();
        assert_eq!(w.code().len(), CODE_DIGITS);
        assert!(w.code().bytes().all(|b| b.is_ascii_digit()));
        assert!(w.is_open(t0));
        assert!(!w.is_open(t0 + WINDOW_TTL));
        assert!(w.begin_attempt(t0 + WINDOW_TTL).is_none());
        for _ in 0..MAX_ATTEMPTS {
            let (_, code) = w.begin_attempt(t0).unwrap();
            assert_eq!(code, w.code());
        }
        assert!(w.exhausted());
        assert!(!w.is_open(t0));
        assert!(w.begin_attempt(t0).is_none());

        let mut w = PairingWindow::open(t0).unwrap();
        w.close();
        assert!(!w.is_open(t0));
    }

    /// Concurrent connections each start an exchange before any confirms:
    /// the window still hands out at most MAX_ATTEMPTS code guesses.
    #[test]
    fn concurrent_exchanges_are_bounded_by_the_attempt_limit() {
        let t0 = Instant::now();
        let mut w = PairingWindow::open(t0).unwrap();
        let started = (0..100).filter(|_| w.begin_attempt(t0).is_some()).count();
        assert_eq!(started, MAX_ATTEMPTS as usize);
    }

    #[test]
    fn tickets_are_bound_to_their_window() {
        let t0 = Instant::now();
        let mut a = PairingWindow::open(t0).unwrap();
        let (ticket, _) = a.begin_attempt(t0).unwrap();
        assert!(a.ticket_valid(ticket, t0));
        // Using up the remaining attempts does not void an exchange in flight.
        while a.begin_attempt(t0).is_some() {}
        assert!(a.ticket_valid(ticket, t0));
        // Expiry, closing, or a new window do.
        assert!(!a.ticket_valid(ticket, t0 + WINDOW_TTL));
        let b = PairingWindow::open(t0).unwrap();
        assert!(!b.ticket_valid(ticket, t0));
        a.close();
        assert!(!a.ticket_valid(ticket, t0));
    }

    #[test]
    fn malformed_exchanges_give_their_attempt_back() {
        let t0 = Instant::now();
        let mut w = PairingWindow::open(t0).unwrap();
        for _ in 0..10 {
            let (t, _) = w.begin_attempt(t0).unwrap();
            w.refund(t);
        }
        assert_eq!(w.attempts_left(), MAX_ATTEMPTS);
        // Another window's ticket refunds nothing here.
        let mut other = PairingWindow::open(t0).unwrap();
        let (foreign, _) = other.begin_attempt(t0).unwrap();
        w.begin_attempt(t0).unwrap();
        w.refund(foreign);
        assert_eq!(w.attempts_left(), MAX_ATTEMPTS - 1);
    }

    #[test]
    fn codes_are_not_constant() {
        let t0 = Instant::now();
        let codes: std::collections::HashSet<String> = (0..20)
            .map(|_| PairingWindow::open(t0).unwrap().code().to_owned())
            .collect();
        assert!(codes.len() > 15);
    }
}
