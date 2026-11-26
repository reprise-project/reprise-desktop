# Security policy

## How to report a vulnerability

Report security issues privately via the [Security tab](https://github.com/reprise-project/reprise-desktop/security) on `reprise-project/reprise-desktop` using "Report a vulnerability". Do not open a public issue.

## What is in scope

Everything in this repository is in scope: `reprise` (protocols, input, XWayland, clipboard, capture), the lock screen and PAM, `reprise-login`, `reprise-rdp`, `reprise-login-rdp`, `reprise-portal`, the polkit agent and bundled programs.

Report bugs in Smithay, PipeWire, NetworkManager, PAM or other dependencies upstream unless Reprise's use creates the flaw.

## What counts as a vulnerability

Crossing any boundary in [docs/security.md](docs/security.md) is a vulnerability. Items listed there as trusted or known gaps are not, though issues proposing fixes are welcome.

## What to include

- Commit or package version tested
- Distribution and startup method (login manager or nested)
- Steps to reproduce and any proof of concept
- Attacker starting position and impact

## What happens next

Reprise has one maintainer, so responses are best effort. I acknowledge reports, state whether the issue is a vulnerability and credit you unless asked not to.

## Supported versions

Only the latest release and `main` receive fixes.
