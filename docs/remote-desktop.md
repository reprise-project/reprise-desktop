# Remote Desktop

Reprise includes an RDP server, `reprise-rdp`, built on [IronRDP](https://github.com/Devolutions/IronRDP). Another computer can use your session, and windows move between this screen and the client. Under `reprise-login`, users in `reprise-remote` can log in remotely at once. Nothing listens until you turn it on.

## Turn on Remote Desktop

> [!WARNING]
> Remote Desktop takes network logins and has had no formal security review. Keep it on your home network, or reach it over a VPN such as Tailscale or WireGuard. Do not open the port to the internet.

Reprise opens no firewall port and sets up no port forwarding. If you run a firewall, allow the port only from trusted networks such as your LAN or `tailscale0`.

To share your session, open Settings > Remote Desktop and turn on Allow remote connections to this computer. That enables and starts `app-org.reprise.RemoteDesktop.service`. Clearing it stops and disables the unit.

The lines below the check box show the listening address, port (3389 by default), and video encoder. If the server fails to start, Settings shows why ([Check the logs](#check-the-logs)).

Under `reprise-login`, the login manager takes connections for `reprise-remote` users, and your session server stays off ([install.md](install.md#turn-on-remote-logins), [Remote logins](#remote-logins)).

## Connect from a client

`xfreerdp3`, Remmina, and `mstsc` work with Reprise.

In Remmina, add an RDP profile with the address and port from Settings > Remote Desktop, your login name, and your password. Set Security protocol negotiation to TLS protocol security and leave Colour depth at Automatic so the client uses H.264.

With FreeRDP 3, connect from a terminal:

    xfreerdp3 /v:HOST /u:USER /sec:tls

Useful `xfreerdp3` options:

- `/multimon` puts each screen on its own client monitor.
- `/dynamic-resolution` resizes the client's screen with its window in lock mode.
- `/sound:latency:20` lowers sound delay.

On your first connection, compare the certificate fingerprint with Settings > Remote Desktop before accepting it.

`mstsc` uses NLA, so it needs `reprise-login` and a Remote Desktop password. Compare its SHA-1 thumbprint with Settings > Login Manager, or pick SHA-1 thumbprint on Settings > Remote Desktop.

After a fourth wrong password from an address, further tries wait longer ([security.md](security.md#remote-desktop)).

FreeRDP versions before 3.32.2 on x86 can show dotted fringes on coloured edges. Upgrade to FreeRDP 3.32.2 or later, use a window width divisible by 32, or set picture quality to Speed.

## Which password to use

RDP clients log in with TLS or NLA (Network Level Authentication). With TLS, the client sends your user name and password over an encrypted connection, so Reprise checks your login password. With NLA, the client proves it knows the password without sending it. Because NLA cannot check your login password, it needs a separate Remote Desktop password.

| Server | Client set to TLS | Client set to NLA |
|---|---|---|
| Your session (Settings > Remote Desktop) | Your login password | Refused |
| `reprise-login`, no Remote Desktop password | Your login password | Refused |
| `reprise-login`, with Remote Desktop password | Your login password | Your Remote Desktop password |

Most clients offer both TLS and NLA. Reprise picks NLA if you have a Remote Desktop password, and TLS otherwise. NLA-only clients need `reprise-login` and a Remote Desktop password. In `xfreerdp3`, `/sec:nla` forces NLA and `/sec:tls` forces TLS.

See [Remote Desktop password](#remote-desktop-password) to set one and [security.md](security.md#remote-desktop-password) for how Reprise stores it.

## What works on the client

### Screens and display

With several monitors (`xfreerdp3 /multimon`), each screen appears on its own monitor, fitted to its shape. Extra monitors stay black. A single window, or fewer client monitors than screens, shows all screens side by side.

Unplugging a local monitor reconnects the client to the remaining screens. A newly plugged monitor shows up on the next connection. The lock screen appears on the client, and the client's keyboard unlocks it.

### When no monitor is connected

With Remote Desktop on and no monitor connected (such as on a KVM or headless boot), a standby screen keeps your desktop running. It uses the last monitor's size, scale, and position, else the first `[[outputs]]` entry (primary if set), else 1920x1080 at scale 1.

Settings > Displays hides the standby screen. When a monitor connects again, viewers reconnect once. In lock mode, client screens hold the desktop instead.

### Keyboard and mouse

Keys go by position and use this computer's keyboard layout. Caps Lock and Num Lock follow the client. Five mouse buttons, vertical scroll, and horizontal scroll work. Held keys and buttons release when a client disconnects.

### Copy and paste

Text and pictures copy both ways while connected. Limits are 16 MiB of text (UTF-16 over RDP, about 8 MiB of ASCII) and 64 MiB for pictures. Files do not copy across.

Nothing crosses the clipboard while the session is locked (or while the remote side is locked in lock mode).

### Sound

With Play sound on the remote computer (`sound`) on, audio plays on connected clients instead of here. Changes apply right away.

A "Remote client" output becomes default while connected and captures every program's audio. When the last client leaves, prior outputs return unless you changed them.

Audio uses Opus when supported, or 16-bit PCM at 44.1 kHz. Delay is about 110 ms in FreeRDP, or about 65 ms with `/sound:latency:20`.

In lock mode, "Remote client" captures all audio even with `sound` off or a silent client, keeping sound off locally while locked.

## Choose a video encoder and quality

At start, the server tests H.264 encoders in order: VA-API (Intel and AMD), NVENC (NVIDIA), Vulkan, and x264 (CPU). Automatic picks the first working encoder.

Pick one under Video encoder (`encoder`) on Settings > Remote Desktop. Unavailable encoders are greyed out. If your choice fails at start, the server falls back to Automatic and notes why on the Video line.

If an encoder fails while running, the server moves to the next working one, then to RemoteFX bitmaps. None (RemoteFX) sends bitmaps only, and clients without H.264 always get bitmaps. Changes apply on the next frame.

Picture quality runs from Speed to Sharp. Frame rate (`frame_rate`) is 30, 60, 120, or 144, capped at the screen's refresh rate. Both apply right away.

At Balanced and Sharp, VA-API, NVENC, and x264 send full colour (AVC444) so small coloured text stays sharp. Two encoders split the work by default ([`avc444_split_encoders`](configuration.md#remote-desktop) disables this). Speed and Vulkan send reduced colour to save bandwidth.

## What this screen shows while connected

While connected, this computer's screen (`laptop_screen`) on Settings > Remote Desktop controls what this computer shows during a connection.

### Lock mode

Locks and fits the remote window is the default. Connecting locks this computer and moves your windows to client screens: one per client monitor at its size and scale, or one matching its window.

Resizing a client window (`xfreerdp3 /dynamic-resolution`) resizes its screen without reconnecting. Changing the client's monitor count disconnects it so it can reconnect with the new layout. With several clients, the first client sets the screens until the last leaves.

Local input reaches only the local lock screen. Local monitors turn off when idle, while idle suspend waits for remote clients.

The client's screens lock after `[idle] lock_after` of remote idle time, from Start > Lock, or with the lock key ([security.md](security.md#lock-mode)).

Unlocking locally (by password, fingerprint, or `loginctl unlock-session`) ends lock mode. Every client disconnects without auto-reconnecting, and your windows return here. When the last client leaves, this computer stays locked.

If `[power] locker` is set, lock mode is off, connections use Stays on, and Settings greys out the Locks choice.

### Show mode

With Stays on (`laptop_screen = "show"`), this computer's screen, keyboard, and mouse stay active. Remote input resets the screen saver and idle lock, while screen off and idle suspend wait.

When the last client leaves, the session locks. Clear Lock this computer when the last one disconnects (`lock_on_leave`) under Connected computers to leave it unlocked.

If a monitor change drops the last client (unplugged, KVM switch, or standby replacement), locking waits for `lock_grace_s`. That setting (Wait before locking on a screen change) defaults to 5 seconds. Pick Don't wait to lock right away. Both settings apply only in show mode.

In both modes, a red dot marks screen sharing. The red capture border stays off unless you enable Show a red border while a remote computer is connected (`capture_border`). Closing a laptop lid turns off the panel instead of running `[power] lid_close`. If the last client leaves with the lid still shut, the lid action runs 30 s later.

## Disconnecting

Connected computers on Settings > Remote Desktop lists active clients. Click Disconnect to end one, or turn Remote Desktop off to end all. Both block auto-reconnect. Turning Remote Desktop off also locks the session unless `lock_on_leave` is off.

On the remote side of lock mode and in background sessions, Disconnect replaces Switch User on Log Off and Stand By on Turn Off. In Glass, it appears first under Shut down. Disconnect ends every client and leaves the session running and locked here.

Log Off, Turn Off, and Restart tell clients the session ended so they do not reconnect.

Up to `max_viewers` computers (4 by default) can connect or log in at once. Extra clients are turned away. With `second_client = "replace"`, a new login disconnects existing clients, and lock mode switches to the new client's monitors. A dead connection times out in about 30 s.

## Remote logins

Under `reprise-login`, any user in `reprise-remote` can log in over RDP whether logged in locally or not ([install.md](install.md#turn-on-remote-logins)). Machine settings live on Settings > Login Manager and require an administrator's password to save.

Connect on port 3389 (unless changed) with your user name and the password from [Which password to use](#which-password-to-use). NLA accepts `user`, `.\user`, `HOST\user`, `user@host`, and `user@` ([security.md](security.md#remote-desktop-password)).

### Remote Desktop password

Open User Accounts, open your account's Properties, and use the Password tab to create, change, or remove your Remote Desktop password. The User Accounts... button on Settings > Remote Desktop opens this tab.

It needs at least 8 characters and must differ from your login password. Reprise stores it as a weak hash, so pick a strong one ([security.md](security.md#remote-desktop-password)). Locking your login password does not clear it.

Each user sets their own. Create and Change are greyed out while remote logins are off, if you are not in `reprise-remote`, or as root. Remove stays open.

### Background and local sessions

If you are logged in locally or in the background, the client joins that session with your settings (`laptop_screen`, `second_client`, `max_viewers`, picture, and sound). Settings > Remote Desktop hides the port and greys out New Certificate... because one certificate serves every user. Both pages show its fingerprint and thumbprint.

If you are not logged in, your session starts in the background on its own VT. This computer's screen stays on the login screen or whoever is using it. The background session stays locked locally and uses client screens like lock mode.

Up to `max_sessions` sessions (8 by default) run at once, including local sessions. An account with an expired password cannot start a background session. Log in locally first.

Logging in locally brings your background session to the monitor. Switch User or a VT switch sends a local session back to the background, locked, for a remote login to rejoin.

### Leaving and power

Disconnecting leaves a background session running for your next connection. End disconnected sessions after (`end_disconnected_after`) logs it off after that idle time, unsaved work included. Pick Never to keep it until log off. The local session never ends this way.

While any client is connected, the computer does not sleep. A shut laptop lid darkens the panel instead of running its lid action, even at the login screen. Background sessions never suspend on idle.

Refused logins wait 3 s and back off per address ([security.md](security.md#throttle)). Check the `reprise-login` journal for the reason.

## Where settings live

Session settings live on Settings > Remote Desktop ([programs.md](programs.md#reprise-settings)) and under `[remote_desktop]` in `config.toml` ([configuration.md](configuration.md#remote-desktop)).

Under `reprise-login`, machine settings live on Settings > Login Manager and in `/etc/reprise/login.toml` ([configuration.md](configuration.md#remote-logins)).

Password checks and refusal rules are in [security.md](security.md#remote-desktop), along with [lock mode](security.md#lock-mode) and [remote logins](security.md#remote-logins).

## Check the logs

Read your session server's log from the user journal:

    journalctl --user -b -u app-org.reprise.RemoteDesktop

Common entries:

- `listening on ADDRESS`, or `cannot listen on port N: ...` if another program holds the port.
- `ADDRESS: logged in as USER`, `ADDRESS: wrong password for USER`, and `disconnected` or `closed` lines with the reason.
- `ADDRESS: refused: the client requires NLA or RDP security; choose TLS security in its settings`. Switch the client to TLS ([Connect from a client](#connect-from-a-client)).
- `ADDRESS: closed; IP has N s of back-off left after failed logins` means that address hit the wrong-password limit ([security.md](security.md#remote-desktop)).
- `encoder NAME is unavailable here; using LABEL` logs at start. `LABEL failed, using LABEL instead` and `H.264 failed, sending bitmaps from now on` log mid-session ([install.md](install.md#optional)).

Under `reprise-login`, check the system journal for login verdicts and refusals, and the user journal for hand-over logs:

    journalctl -b -u reprise-login -u reprise-login-rdp -u reprise-login-rdp.socket
    journalctl --user -b -u reprise-rdp-handover

See [install.md](install.md#turn-on-remote-logins) for setup details.
