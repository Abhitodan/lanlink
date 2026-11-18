# Security policy

LANLink can see and control a Windows session, so security reports are taken seriously.

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's private reporting instead: the **Security** tab → **Report a vulnerability**. Include:

- what an attacker needs (same Wi-Fi? a paired phone? local access to the PC?);
- the steps or a proof of concept;
- the versions or commit you tested.

You should get an acknowledgement within a week. Fixes are released before the details are published.

## Scope

In scope:
- the PC app, the console host and the phone app;
- the protocol and pairing;
- anything that lets someone without an approved, paired phone see or control the PC, or lets a paired phone do more than intended.

Out of scope:
- denial of service by flooding the local network;
- attacks that need administrator rights or malware already running as the same Windows user;
- a stolen, unlocked phone that is still paired (revoke it on the PC).

These residual risks are listed in [docs/security-threat-model.md](docs/security-threat-model.md).

## Supported versions

No release has been made yet. Security fixes go to the default branch.
