# Contributing to LANLink

Thanks for helping. A few rules keep the project trustworthy.

## Ground rules

- **Evidence, not claims.** If a change says something works, it comes with a test or a recorded run. [docs/implementation-status.md](docs/implementation-status.md) lists only what has evidence. Keep it that way.
- **Security first.** Anything touching pairing, the protocol, input injection or storage gets a test for the attack it prevents. Read [docs/security-threat-model.md](docs/security-threat-model.md) first.
- **No hidden access.** Nothing may run in the background without the PC user knowing. Nothing may bypass UAC, the secure desktop or the lock screen. Nothing may weaken Windows or Android protections.
- **Licences.** Only permissively licensed dependencies (MIT, Apache-2.0, BSD, ISC). No GPL code. See [ADR 0010](docs/adr/0010-licensing.md).

## Before opening a pull request

```powershell
scripts\test-all.ps1
```

That runs:
- `cargo fmt --check` and `cargo clippy -D warnings`;
- every Rust test (the live capture, input and end-to-end tests need an unlocked desktop and skip, with a printed reason, without one);
- the Android unit tests and lint.

Update `docs/protocol.md` and the test vectors if you change the wire format, and bump `PROTOCOL_VERSION` when old peers would misread it.

## Reporting bugs

Use the bug-report template. Include:
- the Windows build and GPU;
- the phone model and Android version;
- what you expected, and what happened instead;
- the PC log, `%LOCALAPPDATA%\LANLink\lanlink-host.log`. Check it for anything private before you attach it.

Report security problems privately: see [SECURITY.md](SECURITY.md).
