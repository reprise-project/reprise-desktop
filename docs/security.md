# Security model

This page says where Reprise draws its trust boundaries, so you can tell
what a program running in your session can and can't do, and what is a
bug. It is written for users and packagers. To report a problem, see
[Reporting](#reporting).

## The Wayland socket

Reprise's boundary is the Wayland socket in `$XDG_RUNTIME_DIR`. A program
that connects to it directly, without a sandbox's security context, is
trusted like you are. It runs as your user, so it could read your files and
your keyring anyway; Reprise doesn't pretend otherwise.

Such a program can bind every global Reprise offers, including these:

- **Input method** (`zwp_input_method_manager_v2`): reads the keys typed
  into the focused text field and types text into it.
- **Virtual keyboard and pointer** (`zwp_virtual_keyboard_manager_v1`,
  `zwlr_virtual_pointer_manager_v1`): types keys, key binds included, and
  moves and clicks the pointer. Keys are dropped at the lock screen and
  while an input capture holds the keyboard.
- **Foreign toplevel management and workspaces**
  (`zwlr_foreign_toplevel_manager_v1`, `ext_foreign_toplevel_list_v1`,
  `ext_workspace_manager_v1`): lists every window with its title and app id,
  acts on any of them, and switches desktops.
- **Layer shell** (`zwlr_layer_shell_v1`): puts a surface above or below
  every window, and can take the keyboard while it is shown.
- **Data control** (`ext_data_control_manager_v1`,
  `zwlr_data_control_manager_v1`): reads the clipboard and the primary
  selection at any time, and replaces them.
- **Screen capture** (`zwlr_screencopy_manager_v1`,
  `ext_image_copy_capture_manager_v1`): copies any screen or window without
  asking. A running capture draws the red capture border (`[privacy]
  capture_border`); a single screenshot shows nothing, and stills taken in a
  loop light the screen-sharing dot. The Remote Desktop server's capture,
  which only the installed portal can mark, goes without the border unless
  `[remote_desktop] capture_border` is set.
- **Session lock** (`ext_session_lock_manager_v1`): locks the screen and
  draws the lock screen. See [The lock screen](#the-lock-screen).
- **Output management, power and gamma** (`zwlr_output_manager_v1`,
  `zwlr_output_power_manager_v1`, `zwlr_gamma_control_manager_v1`):
  rearranges, turns off or recolours the screens.
- **On top** (`reprise_on_top_v1`): keeps one of the program's windows
  above the others, below a [consent prompt](#consent-prompts).

These are the protocols panels, lockers, clipboard managers, screenshot
tools and input methods are built on; [protocols.md](protocols.md) lists the
tools. Reprise has no allow-list for them.

### Sandboxed programs

A client is sandboxed when it connects through a socket a sandbox opened
with `wp_security_context_manager_v1` (Flatpak does this), when its process
sees `/.flatpak-info` or runs in an `app-flatpak-*.scope`, or when it runs
in a strict or devmode snap (a `snap.*` AppArmor profile or scope). A
classic snap counts as a host program. Reprise reads the connecting process
through a pidfd. One that has exited by then counts as sandboxed, and one
that hides its root (a non-dumpable one) counts as sandboxed by its cgroup,
and only inside a pid namespace of its own.

A sandboxed client sees none of the globals above, nor
`wp_security_context_manager_v1`, so it can't open a socket under another
app id. It does see `xdg_toplevel_drag_manager_v1`, which moves only its own
window during a drag it started. The full list is `PRIVILEGED_GLOBALS` in
[`src/sandbox.rs`](../src/sandbox.rs). A sandboxed program reaches screen
sharing, screenshots, remote control and input capture through the portal,
which asks you first.

Flatpak lets an app own and call the bus names under its app id, so an app
could take an `org.reprise.*` id. The compositor's `org.reprise.Tray`, each
Reprise program's `org.freedesktop.Application`, Explorer's
`org.freedesktop.FileManager1` and `org.reprise.RemoteDesktop` refuse every
call from a process that sees `/.flatpak-info`, or has exited, with
AccessDenied.

`[clipboard] trusted_apps` gives the two data control globals back to the
Flatpak apps it names, by the app id Flatpak put on the security context,
and nothing else. A listed app can read everything you copy. See
[flatpak-clipboard.md](flatpak-clipboard.md).

A sandboxed program's file list names a host file only under the document
portal (`$XDG_RUNTIME_DIR/doc`), so Reprise takes its drags and clipboard
file lists only through that portal, never as a cut. The compositor marks
its drags and selections `application/x-reprise-sandboxed` and strips the
type from any other program's, so Reprise's programs only ever copy its
files. An X11 drag counts as sandboxed unless a host X11 window has the
focus. An X11 selection goes by the focused window, and counts as sandboxed
when that window's X client has no process Xwayland can name.

`reprise_input_v1` carries remote control, input capture, global
shortcuts, the colour picker and lock mode between the compositor and the
portal. Reprise offers it only to a client whose process runs
`reprise-portal` from `/usr/bin` or `/usr/local/bin`. That keeps other
clients from binding it; it is no boundary against code running as you.

The remote desktop server's sessions skip the permission dialog, since
nobody may be at the screen; turning Remote Desktop on is the consent. The
portal skips it only for the main process of the enabled
`app-org.reprise.RemoteDesktop.service` (or, under reprise-login,
`reprise-rdp-handover.service`) running `reprise-rdp` from `/usr/bin` or
`/usr/local/bin`. Such a session's input lights no dot, types into the lock
screen, and is not kept from the power, sleep, VT switch, radio and
touchpad keys. The screen recorder skips the dialog only for a recording
you asked for: the compositor names the recorder's pid to the portal, which
lets one Start from that `reprise-record` through within 10 s, and opens
the recording's file itself, so nothing can swap the name under it. A
sandboxed app's requests come through its bus proxy and never pass either
way. Reprise's own portal clients take an answer only from the owner of
`org.freedesktop.portal.Desktop`.

## X11 programs

All X11 programs share one Xwayland server, which a Flatpak app with X11
access joins too. Inside it there is no isolation, as on any X11 desktop: an
X11 program can read the keys typed into other X11 windows and their pixels,
grab the pointer, inject input through XTEST, and read any copy made through
X11 until it is cleared or replaced.

The server admits only a client that presents the session's random
MIT-MAGIC-COOKIE-1, kept in `$XDG_RUNTIME_DIR/reprise/xauth-<n>` (0600) and
named in `XAUTHORITY`. Flatpak copies it into an app with X11 access; one
without X11 access has no cookie and is refused, network access or not. Code
running as you outside a sandbox can read the file, the same boundary as
the Wayland socket.

Wayland programs are not exposed to this. An X11 program sees only the X11
windows, hears keys only while an X11 window has the focus, and can't
capture or drive Wayland windows.

The clipboard bridge between X11 and Wayland opens while an X11 window has
the focus: only then may an X11 program read or replace the Wayland
clipboard and primary selection. One exception serves a Flatpak app P whose
window is a Wayland one and whose copy buttons fall back to X11: while P's
Wayland window has the focus, an X11 program may replace the clipboard (not
the primary selection) when the window it names as the owner was created by
a process right in P's `app-flatpak-<P>-<n>.scope`, in a pid namespace,
under no snap profile, with a readable `/.flatpak-info` naming P. Nothing is
read through the exception.

Each take is ruled from the focus and the owner at that moment. X11 names
only the window a client claims as the owner, so while P has the focus any
X11 program naming one of P's X11 windows, or reusing a pid of P's, replaces
the clipboard as P would. While the clipboard holds an X11 copy, any X11
program that takes the selection replaces what Wayland windows paste, with
no focus.

An X11 window's process is the one Xwayland recorded at its connection
(X-Resource), never its own `_NET_WM_PID`; the focus rules and End Now go by
it. A program forwarded over `ssh -X` is the local `ssh`, so End Now
disconnects its windows and sends no signal.

## The clipboard history

`[clipboard] history` keeps the last 25 copies in the compositor's memory,
never on disk, until the session ends. A copy whose
`x-kde-passwordManagerHint` reads `secret` is not kept and leaves the
clipboard when its program quits; password managers, `wl-copy --sensitive`
and browsers' private windows mark copies this way. An unmarked copy is
kept like any other. Nothing is recorded while the desktop is locked.

No program can list the history: an entry reaches a program only as the
clipboard, when you pick it in Super+V. The compositor's memory isn't
locked, so a kept copy can reach swap or a core dump.

## The lock screen

The lock follows `ext-session-lock-v1`. While it holds, no window but an
external locker's gets the keyboard or the pointer, and virtual keyboards
are dropped. Only the power, sleep and media keys (not the Wi-Fi key), the
VT switch keys, the layout switch and the screen reader switch act.

An app's RemoteDesktop session presses nothing while the desktop it lands
on is locked, and never the power, sleep, VT switch, radio, touchpad or
rotation lock keys. Reprise's own remote desktop server is not an app here:
its viewer types the password into the lock screen.

The built-in lock screen lists only the session's user, with the name
fixed, so nobody types another user's password into this session. Switch
User locks first; the login screen runs as `reprise-greeter` and gets no
picture of the session. The password goes on stdin to a helper (`reprise
--password-conversation=USER`) that runs PAM's `reprise` service
(`/etc/pam.d/reprise`, else `login`); only a success unlocks. It runs
`auth` alone, not `account`, so an expired password or `/run/nologin` never
keeps the right password out of the session behind the lock. Further
prompts (a one-time code, a key PIN) go to the same helper, and a helper
that hangs is given up after 90 s. With `/etc/pam.d/reprise-fingerprint`
installed, a second helper runs the fingerprint conversation. Reprise
zeroes the password copies it holds.

Wrong passwords typed at the computer are the PAM stack's to throttle, as
at the console login: one check runs at a time, `pam_unix` waits about 2 s
after a failure, and `pam_faillock`, where the distribution's stack has it
(Arch), locks the account after a few. Ubuntu's stack has no
`pam_faillock`, so there a guess costs those 2 s and no more. Passwords
from a Remote Desktop viewer back off on their own ([Lock mode](#lock-mode)).

The lock's power menu offers Restart and Turn Off only under reprise-login,
while no other session holds someone's work and no `autologin_user` is set,
so the lock never ends another user's work nor restarts into a desktop that
logs itself in. Only the computer's own keyboard and pointer reach it, and
logind is called without interactive authorization.

`[power] locker` runs an external locker (swaylock, hyprlock, gtklock)
instead. The lock stays Reprise's: the screens are covered, focus is
refused, and if the locker crashes the session stays locked and the screen
comes back.

While locked, the accessibility bus gets only the lock screen's tree, the
password as bullets, and the lock takes no reader actions. A screen cast
goes on showing the lock.

When the lock can't check a password at all (no login name, a broken PAM
stack), switch to another VT, log in, and run `loginctl unlock-session`.
Any program that can bind the session lock global can already run that
command, which is why that global needs no extra gate.

## Login manager

`reprise-login` ([install.md](install.md#login-manager)) runs these
processes:

- **The daemon**, `reprise-login.service`, runs as root. It opens no
  network socket, only the unix sockets below and those of [Remote
  logins](#remote-logins). It keeps its state in `/var/lib/reprise-login`, a
  0711 directory any account may pass through but not list.
- **A PAM worker** per login, as root. One worker holds one PAM handle from
  `pam_authenticate` to `pam_close_session` and sets the user's groups and
  ids in the session it forks.
- **The login screen** is the compositor in greeter mode, run as the system
  user `reprise-greeter` (no password, no login shell) with `no_new_privs`,
  on VT 1. It reads no more of a user's files than any other account and
  listens on no Wayland socket. It starts one program: Super+Alt+S runs
  `/usr/bin/orca`, when installed, with no arguments and only the login
  screen's own environment, and stops it on the next press and when the
  login screen exits. Orca gets one Wayland connection, handed over as
  `WAYLAND_SOCKET`, with a sandboxed program's globals. The login screen's
  tree publishes the password as bullets, echoes no key and takes no
  reader actions.
- **Sessions** run as their user, through the user's login shell, with the
  environment PAM set.
- **The settings helper**, `reprise-login-settings`, runs as root through
  `pkexec`, which asks for an administrator's password on each save. It
  takes the settings and the background picture's bytes on stdin, never a
  path, and writes the picture's pixels alone, without EXIF or text, since
  every account can read it.

The sockets live in `/run/reprise-login` (0755, root):

- `greeter.sock` (0660, group `reprise-greeter`) takes one connection, from
  the `reprise-greeter` uid (`SO_PEERCRED`) in the login screen's own
  logind session. Over it the login screen starts a login, answers PAM's
  prompts, starts the session PAM let in, and asks to shut down or restart.
  Without the password PAM wants it starts no session. Root is refused
  before a password is checked, and every failed login waits the same 2 s.
- `session.sock` (0666) takes a peer only from a logind session the daemon
  started or found, with that session's uid. A session can name its
  interface and ask for Switch User; it can't start a login or reach
  another session.

A password travels in zeroed buffers and is never logged, and every
`Debug` of a message that can hold one prints it redacted.

The daemon's unit carries no systemd sandbox: every session descends from
it, and namespaces, a system call filter or `NoNewPrivileges` would bind
each user's session too (no setuid `sudo`, a read-only `/usr`). The login
screen is confined by its own unprivileged user and `no_new_privs`. A
restart or crash of reprise-login leaves running sessions alone.

### Switching users

A session that loses the screen, through Switch User or a VT switch, locks
itself. Logging in again as the same user at the login screen checks the
password, then unlocks that session through logind and switches to it; a
user never gets a second session. A session off the seat locks when its link
to reprise-login drops.

The session unlocks on logind's `Unlock`, so it can't tell reprise-login's
resume from `loginctl unlock-session` run by root, by an admin polkit lets,
or by any process of the session's own user. Off the seat with no link to
reprise-login, it refuses `Unlock`. While `/run/nologin` exists the login
screen refuses logins and resumes alike.

While any session holding someone's work runs, Shut Down and Restart on the
login screen ask for an administrator's password (`reprise-login-check`,
`wheel` or `sudo`, never root). Every refusal comes 3 s after the last
answer, so the reply doesn't tell who is an administrator. A session that
isn't on screen can't take the screen from the user in front of it
([Background sessions](#background-sessions)).

## Remote Desktop

The Remote Desktop server, `reprise-rdp`, offers TLS security only and logs
in the session user alone; any other name is refused before PAM sees it.
NLA (CredSSP) is not offered: its NTLM exchange needs a password the server
already holds, and PAM never hands one out. Under TLS the client sends the
password inside the encrypted connection. Under reprise-login this server
never listens on the network; the login manager takes every connection
([Remote logins](#remote-logins)).

Without reprise-login the server listens on its port on every address.
Another local user whose program holds the port first takes every client's
connection; only a certificate fingerprint other than the one Settings
shows tells the client.

The password is checked in a helper, as at the lock screen, but under its
own PAM service `reprise-rdp`: `pam_unix` for `auth` and `account` and
nothing else. With no pam_faillock, wrong passwords sent over the network
never lock the account at the lock screen. Lock mode's remote lock screen
uses the same service. A further PAM prompt fails the login, as does a
helper with no verdict within 30 s.

The TLS certificate is self-signed and kept in `$XDG_DATA_HOME/reprise/rdp/`
(`key.pem` readable by you alone). Compare the SHA-256 fingerprint the
client shows with the one on Settings > Remote Desktop.

`reprise-rdp` throttles failed logins itself, and logs each one with the
client's address:

- **Per address.** The first three failures are free. The fourth starts a
  2 s back-off, doubling up to 60 s, during which the address's connections
  are closed before TLS. A success, or 20 minutes without a failure, clears
  it.
- **Every address.** 10 failures within 10 minutes pause every login for 10
  minutes.

A login counts as a failure from the start of its check, so a client that
drops mid-check still spends an attempt. A connection has 10 s to send its
X.224 request and TLS ClientHello, then 60 s to log in. Up to
`max_viewers` connections are served at once, counting those still logging
in, and an address (or IPv6 /64) with two still logging in, or backing off,
gets no more. With `second_client = "replace"` one more is let in, and it
ends the others only once it logs in.

Nothing is captured while no client is logged in: the portal session that
shares the screen and takes input opens after the login and closes when the
client leaves, and the screen-sharing dot shows while it runs. When the last
client shown the session leaves, the session locks unless
`[remote_desktop] lock_on_leave = false`
([remote-desktop.md](remote-desktop.md#disconnecting)).

Disconnect in the session, offered on the remote side of lock mode or off
the seat while a remote client drives it, ends every client and leaves the
session running, locked. It calls `Disconnect` on
`org.reprise.RemoteDesktop` on the user's session bus. Any process of the
user can call it and drop that user's clients; no other user reaches the
bus, and a Flatpak process is refused. The server takes `SessionEnding`
(Log Off) only from the compositor whose display it serves.

A Flatpak app with the id `org.reprise.RemoteDesktop` can own the server's
name. The server holds it without letting it be taken over and exits when
it finds it held. Settings, Disconnect and the standby screen talk to the
owner only when it runs outside a sandbox; any other process of yours is
trusted with the name, as it could reach the server's files anyway.

The client shares the clipboard only while it is shown the session, and
nothing crosses while the desktop is locked (as `org.freedesktop.ScreenSaver`
reports it from the process serving the session's display). Text and
pictures cross, files and the primary selection don't. Password-marked
copies stay out of the other side's clipboard history. Other data control
clients are not cut off during a lock.

### Lock mode

With `[remote_desktop] laptop_screen = "lock"`, the default, the first
client shown the session starts lock mode, until the last one goes. With
`[power] locker` set, every connection runs in show mode.

The portal reads the client's monitors from `org.reprise.RemoteDesktop`'s
`ConsoleLayout`, only when that name's owner started the session, and the
compositor makes one virtual screen per monitor, to the right of the
computer's own. The portal casts these screens alone.

As lock mode starts, the session locks as at any other time: LockedHint is
true and every one of the computer's own screens is covered. The computer's
keyboard and pointer reach only the lock screen, through a keyboard state
of their own. The virtual screens have no cover and take only the client's
input. They lock on their own (`lock_after` of remote idleness, Start >
Lock, logind's `Lock`), and their lock screen takes only remote input,
unlocks only them, and checks the password under `reprise-rdp`, so wrong
remote passwords never reach pam_faillock or the fingerprint reader. It
backs off as a login from one address does.

Unlocking at the computer ends lock mode: every input sender the portal let
in is cut off at once, the portal closes those sessions, the windows come
back, and the client is told another connection took over so it doesn't
reconnect. A locker client that takes the lock during lock mode ends it too,
and its lock stays. Without virtual screens (a locker, an old compositor)
the portal shares the computer's own screens as in show mode, and the
connection locks on leave as in show mode.

## Remote logins

With `[remote_desktop] enabled = true` in `/etc/reprise/login.toml`
([configuration.md](configuration.md#remote-logins)), reprise-login takes
RDP connections for the members of the `reprise-remote` group, logs them in
whether or not they have a session, and hands each connection to that
user's own `reprise-rdp`. Two more processes join those under [Login
manager](#login-manager):

- **The front**, `reprise-login-rdp.service`, runs as the system user
  `reprise-login-rdp` and is the only process that takes connections on the
  port. It runs each connection up to its login (X.224, TLS and, under NLA,
  CredSSP), and holds the TLS key, the NT hash of every Remote Desktop
  password and the password of each client logging in. It decides nothing:
  the daemon gives the verdict.
- **The hand-over server**, `reprise-rdp --hand-over`, runs as the user in
  their session, socket-activated at
  `$XDG_RUNTIME_DIR/reprise-rdp/handover.sock` (0600, in a 0700 directory).
  It takes connections from reprise-login only, and serves only the user's
  reprise-login session, never a text-login Reprise of the same user.

`reprise-login-rdp.socket` holds the port: systemd binds it in
`sockets.target`, before any user's service starts, so no local user can
take the port at boot or while the front restarts. While remote logins are
off the port is free, so a user who binds it then keeps it and turning them
on fails until that user lets go.

`/run/reprise-login` holds two more sockets: `front.sock`, one link from
the `reprise-login-rdp` uid only, and `account.sock` (0666), where any local
user asks about their own Remote Desktop password by their `SO_PEERCRED`
uid.

The design assumes the front can be broken into. A front run by an
attacker holds every Remote Desktop password's NT hash and sees the
password of each user who logs in meanwhile, as any TLS endpoint does, but
can't log in with a hash: the daemon wants the password whose hash matches.
The hand-over goes to the uid the daemon looked up, never one the front
names. So the front carries a full sandbox: `ProtectSystem=strict`,
`ProtectHome`, `PrivateTmp`, `PrivateDevices`, `ProtectProc=invisible`,
`NoNewPrivileges`, a `@system-service` system call filter, AF_UNIX only and
no bind, no capability, an empty network namespace and a `/run` holding only
its own directory and `/run/reprise-login`. It also refuses itself new IP
sockets, io_uring and `MSG_FASTOPEN` with seccomp (x86_64 and aarch64), and
TCP connect and bind with Landlock (Linux 6.7 or later), so it can't
reconnect a socket it accepted to another host.

A broken-into front still reaches the port and its connections, the three
sockets in `/run/reprise-login` any local user reaches, and AF_UNIX sockets
outside `/run`, `/tmp` and `/home` its uid may connect to. It reaches no TCP
or UDP service, no abstract or netlink socket and no D-Bus. Without
Landlock's TCP rules an accepted socket can still be reconnected to a TCP
service, and without seccomp also through TCP Fast Open on a kernel whose
Landlock predates the Fast Open fix (Linux 7.2 and its stable backports).

### Checks

For each login the daemon checks, in order:

1. The throttle below, and at most 4 logins in their check at once.
2. The name, without any `DOMAIN\` or `@host`: at most 32 bytes of
   `[a-z_][a-z0-9_.-]*`.
3. The account: it must exist, not be root, and be in the group
   (`reprise-remote` unless `[remote_desktop] group` says otherwise).
4. The secret. Over TLS, PAM's `reprise-login-remote` service checks the
   password with `pam_unix` and no pam_faillock, so remote failures never
   lock the console; any prompt but the first password fails. Under NLA, the
   daemon compares the delegated password's NT hash with the stored one in
   constant time, then `reprise-login-remote-nla` lets only a uid of 1000 or
   more through. Both set `PAM_RHOST`.
5. A session: the user's own, or a new background one while fewer than
   `max_sessions` run. A password that must be changed reaches only a
   running session.

Every refusal waits until 3 s after the login reached the daemon, so an
unknown name, a name outside the group, root and a wrong password look the
same to the client; the daemon's journal names the reason. Under NLA the
front refuses a wrong Remote Desktop password at once, with the same
answer for every other refusal. A client has 10 s to send its X.224 request
and ClientHello, then 60 s to send its credentials.

### Throttle

Throttles count failed logins: the daemon's TLS passwords per client
address, the front's NLA Remote Desktop passwords per IPv4 address or IPv6
/64, and a third the passwords NLA logins delegate, which bounds a front
that passes wrong ones on. No throttle feeds another.

- **Per address.** Three failures are free; the fourth starts a 2 s
  back-off, doubling up to 60 s, during which the front refuses the
  address's connections of that kind before TLS. A success, or 20 minutes
  without a failure, clears the address.
- **Every address.** 10 failures within 10 minutes pause every login of
  that kind for 10 minutes, a right password included. Restarting
  reprise-login clears every throttle.

An NTLM AUTHENTICATE counts as a failure until the client delegates the
password it proved. xfreerdp opens each TLS logon twice, so one mistyped
password counts two. Before a login the front caps connections: 32 in all,
2 per IPv4 address or IPv6 /64, 4 per IPv6 /48, and 256 KiB read from each.

### Hand-over

Once the daemon accepts a login, the front puts the TCP socket into kernel
TLS and sends it to the daemon with the TLS keys and a recording of the
exchange, the password blanked. The daemon opens the user's
`handover.sock` with `openat2`, without following symlinks or crossing
mounts; the runtime directory, `reprise-rdp` and the socket must belong to
the user and not be group or world writable, and `SO_PEERCRED` must name the
user, so only a listener the user made gets the connection. The hand-over
server takes links from root only and serves a connection once. The password
and the NT hash never leave the front and the daemon.

TLS is 1.2 or 1.3 with AEAD suites only. After the hand-over, any handshake
record but a TLS 1.3 key update ends the connection.

The front's self-signed certificate lives in `/var/lib/reprise-login-rdp/`,
and its fingerprint shows on Settings > Login Manager and each user's Remote
Desktop page. To replace it, delete `cert.pem` and `key.pem` there and run
`sudo systemctl restart reprise-login-rdp`.

### Remote Desktop password

NTLM checks a client's proof against a password the server already holds,
which PAM never hands out. So NLA logs on with a Remote Desktop password
each user sets in User Accounts, kept as its NT hash: unsalted MD4, fast to
crack from a copy. The file, `/var/lib/reprise-login/remote-passwords`, is
0600 root. Use a strong one: it needs 8 characters or more and must differ
from the login password.

- `account.sock` acts only on the caller's own uid, and a set or clear takes
  that user's login password, so an administrator can't change another
  user's there.
- Locking the login password (`passwd -l`) leaves the Remote Desktop
  password working: remove it in User Accounts, or take the user out of
  `reprise-remote`. An expired account is refused either way.
- Setting one leaves TLS logins with the login password on.
- An account deleted and made again with the same name and uid keeps the
  old entry, so remove the Remote Desktop password before deleting one.
- The front gets only the hashes of users in the group, never root's.
  Group changes in `/etc/group` apply at once; groups from systemd-homed,
  sssd, LDAP or nscd need `systemctl reload reprise-login`.

NLA takes the name as `user`, `.\user`, `HOST\user`, `user@host` or
`user@`. Only xfreerdp has been tried.

### Background sessions

A login for a user with no session starts one through the user's login
shell, on the first free VT from 7 up that isn't on screen. It is a logind
session on seat0 with `Remote=no`, so polkit treats it as a local session
that isn't active (see [Accepted risks](#accepted-risks)). It comes up
locked, even when someone switches to its VT before its compositor starts,
and its clients always run in [lock mode](#lock-mode). Off the seat it runs
no lid action and no idle suspend.

No session holds a controlling terminal, and the polkit rule
`/usr/share/polkit-1/rules.d/50-reprise-login.rules` refuses
`org.freedesktop.login1.chvt` to every caller outside the logind session on
screen, programs in app scopes and user services included. So no
background session can switch VTs, also while its user has an ssh login.
The compositor switches on Ctrl+Alt+F1..F12 from its own session, and
reprise-login as root.

While a client is connected to any session, reprise-login holds a `sleep`
inhibitor as root, so the computer doesn't sleep under a client: idle
suspend waits and a shut lid darkens the panels instead of running
`[power] lid_close`.

A background session whose last client left ends `[remote_desktop]
end_disconnected_after` minutes later unless a client comes back; logind
ends its processes, unsaved work included. A session the computer shows
counts as a console session and is kept until its user logs off.

### Accepted risks

- **Names with a Remote Desktop password show,** read from the client's
  first, unauthenticated packet. Names leak, never passwords.
- **The chvt rule applies to the whole computer,** under another display
  manager too.
- **Hashes outlive group membership** until the user clears theirs.
- **Wrong passwords on account.sock or the administrator form count toward
  pam_faillock,** so they can lock that account's console login, as `su`
  can.
- **A right administrator password shows through an account refusal,**
  which comes at once in PAM's words, as at the console login.
- **A user's own hand-over server can hold sleep off** by reporting a client
  count that never drops; polkit already lets their session do the same.
- **Inactive polkit grants.** A background session gets `allow_inactive`
  answers, some wider than a remote session's; all affect availability.
- **Active polkit grants while the user is active elsewhere** (an ssh login
  counts): a background session then gets `allow_active` answers, such as
  udisks and logind's power actions, without a password.
- **Any program of a local administrator sets the time zone** without a
  password ([Programs and files](#programs-and-files)).
- **Anyone can pause remote logins** of one kind with 10 wrong passwords
  every 10 minutes.
- **Many addresses can fill the logins in progress** for up to 70 s.
- **A late refusal tells the password was right,** and counts no failure.
- **The journal can fill** with one line per refused login.
- **FUSE over the runtime directory** lets a user stall their own hand-over
  and hold a daemon thread.
- **System accounts in the group** with a login shell can log in over TLS;
  NLA refuses every uid below 1000.

## Keyboard shortcuts

A window can ask Reprise for every keyboard shortcut through
`zwp_keyboard_shortcuts_inhibit_manager_v1`, as VM viewers and remote
desktop clients do. While its surface has the keyboard, it gets the key
binds and the Super key. A balloon says so the first time a program takes
them.

These stay with Reprise under an inhibitor:

- the power, sleep and media keys, and the VT switch keys
- the binds that move no focus: lock, magnifier, colour filter, screen
  reader, keyboard layout, Do Not Disturb, night light, the screenshots,
  screen recording, the colour picker, volume, brightness, playback, Wi-Fi,
  airplane mode, rotation lock and the touchpad
- open menus

Super+Shift+Escape gives the binds back until that window next gains the
keyboard. The same chord ends an input capture (Deskflow, InputCapture
portal sessions). It can't be taken by an inhibitor or a capture.

## Focus

A new window takes the keyboard when it maps if any of these holds:

- it presents a trusted activation token, which Reprise mints when it
  starts a program or you click a tray icon or a notification, and which a
  program can mint only from an input event in its own focused window, or
  within 3 s of a drop on one of its windows
- Reprise started its program within the last 30 s
- it shares a Wayland connection or a process with the focused window, its
  process descends from it or sits in the session it leads, or the focused
  window owns it (a portal's print dialog)
- it is a prompt of `reprise-portal`, `reprise-polkit`, `reprise-drives` or
  `reprise-bluetooth` (picked by title)
- it was started from a terminal multiplexer pane whose client runs in the
  focused terminal
- no window has focus

Any other window opens behind the one you're typing in and flags its task
button. Under the lock no new window takes the keyboard.

A program in the background can flag its task button but can't take the
keyboard through `xdg_activation_v1`. The exception is a program you just
asked to act (a search result you picked, a notification action's
`desktop-entry`): for 30 s its first activation raises it, once. That match
goes by app id, which a client picks itself, so a host program posing as
that one could take the raise; a sandboxed window takes it only under its
own Flatpak app id. A program trusted with foreign toplevel management can
focus any window anyway.

A notification's signals go only to the connection that sent it, so no
other program sees its activation token or the text typed into its reply.

## The portal

`reprise-portal` is the xdg-desktop-portal backend. It answers only calls
from the process that owns `org.freedesktop.portal.Desktop` and runs outside
any sandbox, read through its pidfd by the rules in [Sandboxed
programs](#sandboxed-programs); any other caller gets AccessDenied, so a
sandboxed app that holds the name while xdg-desktop-portal is away can't
speak for other apps. Its own `org.reprise.SessionEnd`, which lists the
apps holding logout off, answers only the compositor's process. App ids, parent windows and restore data are taken
on the frontend's word, and the frontend reads the app id from the sandbox.

What a consent prompt remembers, and where:

- **ScreenCast and RemoteDesktop:** with Remember ticked, the portal returns
  a restore token, which xdg-desktop-portal keeps in its permission store
  (`screencast`, `remote-desktop`) and restores only for the app it was
  issued to. A restored RemoteDesktop grant starts without a prompt but not
  unseen: while it holds an input device the screen-sharing dot shows,
  naming the app, and a critical "<app> is controlling this computer"
  balloon stays in the history, with Stop and forget, until dismissed.
- **Screenshot:** xdg-desktop-portal asks before an app's first
  non-interactive screenshot and keeps the answer (table `screenshot`). An
  interactive screenshot, or one of a window, opens the share picker.
- **Wallpaper:** an app with `yes` in table `wallpaper` sets a picture it
  can read itself without a prompt; one with `no` is refused. Otherwise
  Reprise's prompt shows the bytes it will install, and Remember keeps an
  Allow. A Deny is never kept.
- **InputCapture:** asks each session. Remember keeps an Allow in table
  `input-capture` with the devices granted. The grant goes by app id
  alone, so an unsandboxed program naming itself after a remembered app gets
  it too.
- **GlobalShortcuts:** an app with `[[global_shortcuts]]` entries is
  allowed; a Deny on its first prompt goes into `[portal]
  shortcuts_denied`. When an allowed app binds new ids, the prompt asks
  about those alone: Allow gives them their keys, Deny keeps them with no
  key. A shortcut gets the key an app suggests only when nothing else holds
  it and it isn't a key that types or edits text without Ctrl, Alt or
  Super, so an allowed app can't take keys from the window you type in.
- **Access** (camera, background): answers go to xdg-desktop-portal's
  permission store.
- **Location:** Reprise's geoclue agent keeps its own answers.

The Email portal opens the message unsent, with only the files the app
handed xdg-desktop-portal attached. The Account portal shares your login,
name and picture only after its prompt, every time. The DynamicLauncher
portal adds a launcher only after its prompt, and a launcher starts nothing
the app couldn't start itself.

### Consent prompts

The permission prompts, the share picker, the Access prompt and the
location prompt grant something, and a prompt that takes the keyboard while
you type must not be answered by your typing. Each of them:

- takes its access keys with Alt only: a bare A, R or S presses nothing.
- ignores keys and clicks for 500 ms after it gets the keyboard, or after
  the screen wakes; a key that went down then counts for nothing.
- grants only through a click or an Alt access key: Enter presses the
  refusing button wherever the focus is, and Space presses no other.
- stacks above every window kept on top and every mini view. Only a Wayland
  window of the installed portal, outside any sandbox, counts as a prompt;
  an X11 window never does.
- takes no input from an app's RemoteDesktop session, so an app granted the
  keyboard can't widen its own grant or answer another app's. Reprise's own
  remote desktop server still answers prompts from the viewer.

A sandboxed program with X11 access can still keep an override-redirect
window above other windows, though still under a consent prompt. The polkit
agent's password box and the Drives passphrase box are not consent prompts:
the secret they ask for keeps a stray key from answering them.

To revoke: `flatpak permissions` lists the permission store and `flatpak
permission-remove <table> <id>` removes an entry. Settings > Keyboard
forgets a program's global shortcuts, and Settings > Privacy > App
Permissions forgets the other remembered answers. Forget doesn't end a
running session, and a share still running stores its grant again when it
ends, so stop the share first.

## Programs and files

- **Terminal output.** Whatever a program in the terminal prints (a
  downloaded file, a remote host over ssh) is untrusted. Titles lose
  control, bidi and zero-width characters, and oversized escape sequences
  are dropped. An OSC 52 copy reaches the clipboard only from the open tab
  of a focused window, and an OSC 52 read is refused.
- **Bus data and colour profiles.** What programs send over the session bus
  (tray items, notifications, LauncherEntry, media players, calendars) and
  ICC profiles are read within caps, at the fields Reprise uses. Shown text
  loses control, bidi and zero-width characters, so it can't reorder or hide
  what it shows.
- **Content search.** Off by default (`[launcher] contents`): until it is
  on, no search calls localsearch, so the bus never starts the indexer for
  Reprise. Once on, the indexer keeps its index whether or not Reprise
  searches again. Typed words reach it as a bound argument.
- **Search providers.** Typed words go, from two characters on, to every
  installed search provider not turned off in Settings > Search; the bus may
  start it to answer. A provider counts only beside its own program's
  desktop entry, so one app can't speak for another. Reprise runs nothing a
  result names, and a result's clipboard text is copied only when picked.
- **Who a notification or tray icon is from.** The compositor reads the app
  behind a notification or tray icon from its process, by the rules under
  [Sandboxed programs](#sandboxed-programs). A sandboxed app's
  notification shows under its own desktop entry, whatever name and icon it
  sends, so it can't pass as Reprise, Settings or a password manager; its
  tray icon is kept under that id. A program outside a sandbox keeps the
  name it gives. Only a program outside a sandbox names a file to read with
  your rights: another sender's `image-path` or `sound-file` is dropped.
- **Tray icon files.** A file a Flatpak app names is opened inside its own
  sandbox root ([`crates/reprisebus/src/peer.rs`](../crates/reprisebus/src/peer.rs)
  lists the checks), a snap's only beneath its own folders, and an owner
  whose sandbox doesn't check out names no file. Icons are PNG or SVG within
  caps, an SVG priced against a time and memory budget before it is drawn,
  and a slow or flooding app holds back only its own icons.
- **Launchers.** A `.desktop` file in a folder runs only when it is
  executable and carries the `user.reprise.trusted` attribute, which Allow
  Launching sets and extraction never restores. The attribute is an HMAC
  under a per-user key of the file's filesystem id, inode and contents, so
  any edit drops it; a file over 128 MiB is never trusted. A `.desktop`
  dropped on Quick Launch runs only from an application folder or with the
  attribute; otherwise it asks the same way.
- **Drive Recycle Bins.** A drive's trash is used only as `.Trash-$uid`, a
  real folder of yours, or `$uid` in a real sticky `.Trash`, reached from
  the drive's top without following a link. Restore takes a `.trashinfo`
  path only on that drive, with no `..` step, so an item never lands off its
  drive.
- **Extracting archives.** Each archive comes out into a fresh folder of its
  own. An entry from the root or with a `..` step refuses the archive before
  anything is written, and so does one past the folder's limits: 90% of its
  free space, and 1,000,000 entries or its free inodes, whichever is fewer.
  A symbolic link that resolves outside the extraction, or a file
  hard-linked from outside it, refuses it afterwards and the folder is
  removed. Extract Here then moves the entries into place, writing over
  nothing and merging only into real folders. An archive with a hidden
  top-level entry or a link to nothing in the archive, and in the home
  folder one with a top-level `bin`, stays in its new folder, so Extract
  Here can't add autostart entries, settings or `~/bin` programs.
- **Copying from a drive.** A copy from a drive onto another device carries
  no extended attributes: no ACLs, `user.*` or `security.*` labels, and no
  launcher trust marker.
- **Programs and scripts.** A file with the owner's execute bit that starts
  with `#!` or is an ELF binary runs when you open it, from the desktop,
  Explorer, search, Documents, jump lists, Quick Launch or the Run box. The
  first open asks: Run / Open in Notepad / Cancel for a script, Run /
  Cancel for a binary, and Run / Cancel for a file whose type's program
  runs it (`.exe`, `.msi`, `.bat`, `.lnk`, `.jar`, `.appimage`). Run sets
  the trust marker, so the file opens unasked until it changes;
  `confirm_scripts = false` turns the question off. Application folders and
  root-owned files under `/usr` or `/opt` never ask.
- **Network shares.** Connect to Server hands the address to `gio mount` as
  one argument after `--` and every answer to its prompts on its standard
  input, so a password never shows in a process list. A password typed into
  the address is taken out before it is shown or logged. Remember stores it
  in the Secret Service, looked up only by the server's own attributes; the
  session is the spec's `plain` one, so it crosses your session bus
  unencrypted.
- **Disk images.** Opening an `.iso` or `.img` asks before it is mounted,
  since kernel file system drivers aren't hardened against hostile images,
  and mounts it read-only. Explicit menu choices (Mount, Open With) don't.
- **Exec lines.** A desktop entry's `Exec` is split with no shell, and a
  field code standing alone (`%f`, `%u`) becomes one argument per file. A
  field code inside a quoted argument, which the Desktop Entry spec forbids,
  refuses the entry.
- **Thumbnailers.** Thumbnails hand files Reprise doesn't decode itself to
  their installed `.thumbnailer` with no click, so each runs under
  bubblewrap: new namespaces, `/usr` and the input file read-only, a scratch
  folder its one writable place, no home, session bus or display socket, a
  memory cap and a deadline. JPEG XL thumbnails, Explorer's Space preview
  and the Account prompt's picture decode in the same jail, and the
  portal's other prompts never decode JPEG XL; Image Viewer is the one
  in-process JPEG XL decoder. Without bubblewrap none of these runs and the
  file keeps its icon. The other built-in decoders run in the
  process showing the folder, and one that panics marks its file failed.
- **App scopes.** A program started from a desktop entry runs in a systemd
  scope `app-reprise-<app id>-<n>.scope`, for identification and cleanup,
  not confinement. polkit judges a scoped program by the user's display
  session, and each Reprise session's agent fails a prompt for a program of
  another session, so a prompt never shows on the wrong screen.
- **Account passwords.** User Accounts never removes a password: Add
  Account needs one, and "Ask for a new password at the next login" expires
  the current one (`chage -d 0`). The change happens at the next login that
  starts a session; reaching a session that already runs takes the old
  password. Fingerprints are enrolled only for the user running it.
- **Network secrets.** A secret typed in Network Connections is stored in
  its NetworkManager profile (`/etc/NetworkManager/system-connections`). A
  profile that leaves a secret to an agent can't connect from Reprise.
- **Automatic time zone.** With `[time_zone] automatic` on, the compositor
  sets the zone from geoclue through timedated, with interactive
  authorization off. The package's rule
  `/usr/share/polkit-1/rules.d/50-reprise-timezone.rules` answers yes to
  `org.freedesktop.timedate1.set-timezone` alone, without a password, for a
  local, active user in `wheel` or `sudo`; others still need an
  administrator. Without the rule the setting greys.
- **Language and formats.** Region & Language writes only your own
  `locale.conf` and AccountsService language; Apply to the System goes
  through localed and polkit, with no rule shipped.
- **Bluetooth pairing.** The pairing agent answers only bluetoothd. A
  request Settings didn't start opens a box whose Enter refuses, keyed as
  the [consent prompts](#consent-prompts) are, and accepting pairs without
  trusting the device, so it connects no service until you connect it.
- **Thunderbolt devices.** A Thunderbolt or USB4 device can reach memory
  once authorized. `reprise-drives` never starts boltd and authorizes
  nothing until a button is pressed; boltd checks polkit for each call.
- **Receiving files over Bluetooth.** obexd lets any device in range push a
  file, so Reprise's OBEX agent is the gate. Receiving is off until "Receive
  files" is ticked; then the agent answers only the owner of
  `org.bluez.obex`, refuses an unpaired device, and asks before every file
  in a box gated like the pairing box. An accepted file moves into
  Downloads under a cleaned name that never replaces anything. obexd also
  serves File Transfer to trusted devices inside its own folder.
- **Ending the session.** With `[session] end_processes = true` (the
  default), Log Off stops the `app-reprise-*` scopes and, after a display
  manager login, the login session. A program started outside the session
  (a user service, an SSH login) keeps running.
- **Installing and uninstalling.** Any program of yours can open the
  install sheet on any file, so it goes ahead only on a click on Install or
  its Alt access key, gated as the consent prompts are. Reprise runs
  nothing as root: PackageKit, Flatpak or `pkexec pacman -U` does the work
  and polkit decides. The sheet installs a private copy, so a file swapped
  after it opened is not what installs. A downloaded package's install
  scripts run as root: install only packages you trust. A `.flatpakref`
  adds its repositories as system remotes that stay.

## Out of scope and known gaps

- **No per-app Xwayland** ([X11 programs](#x11-programs)). Only a Flatpak
  without X11 access is kept off the server; a program outside a sandbox is
  trusted like you either way.
- **A level meter lights the microphone.** A meter marks its own stream, and
  any recorder could set the same marks, so the indicator ignores them.
- **Privileged globals have no allow-list** ([The Wayland
  socket](#the-wayland-socket)). Sandboxing keeps a program from them.
- **No clipboard portal.** A sandboxed program gets the clipboard only while
  one of its windows has the focus, or through `[clipboard] trusted_apps`.
- **X11 programs can replace the clipboard** and read every X11 copy
  ([X11 programs](#x11-programs),
  [flatpak-clipboard.md](flatpak-clipboard.md#copy-buttons)).
- **Background sessions get polkit's inactive grants** ([Accepted
  risks](#accepted-risks)).
- **Idle inhibitors hold off the idle lock.** Any program, a sandboxed one
  included, holds `[idle] lock_after` off with an idle-inhibit-v1 inhibitor
  on a surface that shows, an `org.freedesktop.ScreenSaver` Inhibit cookie
  or the portal's Inhibit. None holds off a lock that is already up.
- **No NetworkManager secret agent.** `nmcli --ask connection up` connects
  a profile that asks for its password.
- **Same-user code.** A program running as you can ptrace or preload the
  portal, the compositor or anything else of yours, take a session's
  `reprise.polkit.session<id>` name before its agent does, or take
  `org.freedesktop.ScreenSaver` and hold its Remote Desktop clipboard.
  Reprise doesn't defend against that, and doesn't treat it as a
  vulnerability.

## Reporting

Report a hole in any of these boundaries privately, as
[SECURITY.md](../SECURITY.md) describes.
