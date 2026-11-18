# 0008 Pairing: SPAKE2 bound to certificates and the TLS channel

**Problem.** Both sides have self-signed certificates and no shared
authority; each must learn the other's fingerprint without trusting the
network.

**Decision.** The PC user opens a pairing window (120 s, 3 attempts) showing a
QR code and an 8-digit code. Inside the TLS 1.3 QUIC connection both sides
run SPAKE2 (`spake2` crate) on the code, with both certificate fingerprints
as SPAKE2 identities and the TLS exporter value mixed into key confirmation
(HMAC-SHA-256). The PC user then approves after comparing a 6-digit
confirmation number shown on both screens.

**Properties (tested in `lanlink-auth` and `tests/integration`).**

* Wrong code: confirmation fails and consumes an attempt.
* A machine-in-the-middle presents a different certificate and TLS session,
  so confirmation fails even if it knows the code.
* The QR code holds only public data plus the short-lived code.
* With the QR code the phone also pins the PC fingerprint in TLS; without it
  SPAKE2 authenticates the certificate actually seen.
* No new primitives.

**Rejected.** Trust on first use (silent MITM window); long-lived secrets in
the QR code; inventing a key exchange.

**Residual risk.** `spake2` is a pre-1.0, unaudited crate. Online guessing is
bounded to 3 attempts per window against 10^8 codes.
