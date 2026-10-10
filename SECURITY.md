# Security policy

## Reporting a vulnerability

Please report security problems privately, not in a public issue. Use
GitHub's private vulnerability reporting: open the
[Security tab](https://github.com/reprise-project/reprise-desktop/security)
of reprise-project/reprise-desktop and click "Report a vulnerability".

## Scope

Anything in this repository, in particular:

- the compositor (`reprise`): the Wayland protocols it serves, input
  handling, XWayland, the clipboard and screen capture
- the lock screen and its PAM authentication
- the xdg-desktop-portal backend (`reprise-portal`): screen sharing,
  screenshots, the access prompts Flatpak apps go through
- the polkit agent and the bundled programs (Explorer, Notepad, Settings,
  Network Connections, User Accounts, Software Updates and the rest)

Bugs in Smithay, PipeWire, NetworkManager, PAM or other dependencies belong
upstream. If Reprise uses one of them in a way that opens a hole, that's
in scope.

## What to include

- the commit or package version you tested
- your distribution and how you start Reprise (login manager or nested)
- steps to reproduce, and a proof of concept if you have one
- what an attacker gains: who they need to be to start with, and what they
  can do afterwards

## What to expect

I maintain Reprise on my own, so answers are best effort. I'll acknowledge
the report, tell you whether I consider it a vulnerability, and credit you
in the fix unless you'd rather I didn't.

## Supported versions

Only the latest release and the current `main` get fixes.
