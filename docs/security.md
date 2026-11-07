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
- **Virtual keyboard** (`zwp_virtual_keyboard_manager_v1`): types keys that
  go where the keyboard's would, key binds included. They are dropped at
  the lock screen and while an input capture holds the keyboard.
- **Virtual pointer** (`zwlr_virtual_pointer_manager_v1`): moves the
  pointer and clicks, as a mouse would.
- **Foreign toplevel management** (`zwlr_foreign_toplevel_manager_v1`,
  `ext_foreign_toplevel_list_v1`): lists every window with its title and app
  id, and focuses, minimizes, maximizes, fullscreens or closes any of them.
- **Workspaces** (`ext_workspace_manager_v1`): lists the desktops and
  switches between them.
- **Layer shell** (`zwlr_layer_shell_v1`): puts a surface above or below
  every window, and can take the keyboard while it is shown.
- **Data control** (`ext_data_control_manager_v1`,
  `zwlr_data_control_manager_v1`): reads the clipboard and the primary
  selection at any time, and replaces them.
- **Screen capture** (`zwlr_screencopy_manager_v1`,
  `ext_image_copy_capture_manager_v1` and its capture sources): copies any
  screen or window without asking. A capture that keeps running draws the
  red capture border while it does (`[privacy] capture_border`); a single
  screenshot draws none. The one exception is the Remote Desktop server's
  capture, which goes without it unless `[remote_desktop] capture_border`
  is set: the portal marks that capture's connection through
  `reprise_input_v1`, which only the installed reprise-portal can bind, and
  only for the session it starts without a dialog for the server's enabled
  unit.
- **Session lock** (`ext_session_lock_manager_v1`): locks the screen and
  draws the lock screen. See [The lock screen](#the-lock-screen).
- **Output management, power and gamma** (`zwlr_output_manager_v1`,
  `zwlr_output_power_manager_v1`, `zwlr_gamma_control_manager_v1`):
  rearranges, turns off or recolours the screens.

These are the protocols panels, lockers, clipboard managers, screenshot
tools and input methods are built on; [protocols.md](protocols.md) lists the
tools. Reprise has no allow-list for them.

### Sandboxed programs

A client is sandboxed when it connects through a socket a sandbox opened
with `wp_security_context_manager_v1` (Flatpak does this), or when its
process sees `/.flatpak-info` (a Flatpak app that connected without one).
A sandboxed client sees none of the globals above. It also doesn't see
`wp_security_context_manager_v1` itself, so it can't open a socket of its
own under another app id. The full list is `PRIVILEGED_GLOBALS` in
[`src/sandbox.rs`](../src/sandbox.rs). A sandboxed program reaches screen
sharing, screenshots, remote control and input capture through the portal,
which asks you first.

`[clipboard] trusted_apps` gives the two data control globals back to the
Flatpak apps it names, and nothing else. The app id is the one Flatpak put
on the security context, not one the app chose, and only Flatpak's
contexts are matched. A listed app can read everything you copy. See
[flatpak-clipboard.md](flatpak-clipboard.md).

One more global, `reprise_input_v1`, carries remote control, input capture,
global shortcuts, the list of sandboxed apps with a window, the colour
picker and, from version 9, lock mode ([Lock mode](#lock-mode)) between the
compositor and the portal. Reprise offers it only to a client whose process
runs `reprise-portal` from `/usr/bin` or `/usr/local/bin`. That keeps other
clients from binding it directly; it is no boundary against code running as
you.

Reprise's remote desktop server starts its sessions without the permission
dialog, since nobody may be at the screen to answer it. Turning Remote
Desktop on in Settings is the consent. The portal skips the dialog only for
the app id `org.reprise.RemoteDesktop`, only while the user unit
`app-org.reprise.RemoteDesktop.service` is enabled and active, and only when
that unit's main process runs `reprise-rdp` from `/usr/bin` or
`/usr/local/bin` and is the process that asked. A sandboxed app's requests
reach the portal through its bus proxy, so they never pass. Such a session
gets the keyboard, the pointer and every screen whose top left lies at a
non-negative position in the layout, or every screen when none does, and
leaves no restore token behind. In lock mode it gets the screens made for
the client instead. As with `reprise_input_v1`, this is no boundary against
code running as you.

## X11 programs

All X11 programs share one Xwayland server, which Reprise starts on the
first X11 connection. A Flatpak app with X11 access joins the same server.
Inside it there is no isolation, as on any X11 desktop: an X11 program can
read the keys typed into other X11 windows, read their pixels, grab the
pointer and inject input into them through XTEST.

Wayland programs are not exposed to this. An X11 program sees only the X11
windows, hears keys only while an X11 window has the focus, and can't
capture or drive Wayland windows.

The clipboard bridge between X11 and Wayland opens only while an X11 window
has the focus: an X11 program can't read the Wayland clipboard, or replace
it, from the background.

An X11 window's process id comes from its own `_NET_WM_PID` property, so
the focus rules below take an X11 program's word for which program it is.

## The lock screen

The lock follows `ext-session-lock-v1`. While it holds, no window but an
external locker's gets the keyboard or the pointer and virtual keyboards are
dropped. Of the keys, only the power, sleep and media keys (but not the
Wi-Fi key), the VT switch keys and the layout switch act; the rest go to the
lock screen.

The built-in Unlock Computer dialog checks the password in a helper
process: the compositor starts itself again as `reprise
--password-conversation=USER` and writes the password to its stdin, never
to argv or the environment. The helper runs PAM's `reprise` service
(`/etc/pam.d/reprise`, or `login` when that file is missing) and reports
the verdict on stdout; only a success unlocks. A further prompt from the
stack (a one-time code, a security key PIN) relabels the field, and its
answer goes to the same helper's stdin. A helper that hangs is given up
after 90 s and the dialog asks again. Reprise zeroes the password copies
it holds.

When `/etc/pam.d/reprise-fingerprint` is installed, a second helper runs
the fingerprint conversation while the dialog shows. Either one succeeding
unlocks. A fingerprint conversation that ends without success (too many
fingers that did not match) starts again after 2 s, the wait doubling up to
30 s; one that finds no reader or no enrolled finger does not.

`[power] locker` runs an external locker (swaylock, hyprlock, gtklock)
instead of the dialog. The lock stays Reprise's: the screens are covered and
focus is refused as with the dialog, and the locker draws on top and does
its own PAM check. The fingerprint helper doesn't run under a locker, which
may claim the reader itself. If the locker crashes the session stays locked
and the dialog comes back. A locker may take over a lock that has none, the
dialog's or a crashed locker's, and unlock it. A locker whose lock has ended
can't unlock a lock taken after it.

When the lock can't check a password at all (no login name, a broken PAM
stack), switch to another VT, log in, and run `loginctl unlock-session`.
Any program that can bind the session lock global can already run that
command, which is why that global needs no extra gate.

## Login manager

`reprise-login` ([install.md](install.md#login-manager)) runs these
processes:

- **The daemon**, `reprise-login.service`, runs as root. It opens no
  network socket, only the two unix sockets below and the two of [Remote
  logins](#remote-logins). It reads
  `/etc/reprise/login.toml` and keeps the last user and interface in
  `/var/lib/reprise-login/state.toml` (0600, in a 0700 directory).
- **A PAM worker** per login, and one for the login screen: the daemon
  starts itself again as `reprise-login --worker`, as root. One worker holds
  one PAM handle from `pam_authenticate` to `pam_close_session`, so a module
  that takes the password at login (pam_gnome_keyring) sees it again when
  the session opens. It sets the user's groups and ids in the child it
  forks for the session, and waits as root to close the PAM session.
- **The login screen** is the compositor in greeter mode (`reprise
  --greeter`), run as the system user `reprise-greeter` in a greeter-class
  logind session on VT 1. The account has no password and no login shell;
  its home is `/var/lib/reprise-greeter` (0750). The worker sets
  `no_new_privs` before it starts the login screen, so a setuid program
  gains it nothing. It reads user pictures from AccountsService's
  directory and can read no more of a user's files than any other account.
  It listens on no Wayland socket, so the user services logind starts for
  `reprise-greeter` can't reach it; `[login] greeter_socket` opens one for
  testing.
- **Sessions** run as their user, through the user's login shell, with the
  environment PAM set.

The sockets live in `/run/reprise-login` (0755, root):

- `greeter.sock` is 0660, group `reprise-greeter`. The daemon accepts a
  connection only from the `reprise-greeter` uid (`SO_PEERCRED`) in the
  logind session the daemon started the login screen in, and only one at a
  time. A frame that doesn't decode closes the connection. Over it the
  login screen can start a login for a user name, answer PAM's prompts,
  start the session PAM let in, cancel, and ask to shut down or restart.
  Without the password PAM wants it can start no session. The
  `reprise-login` PAM service refuses root before it checks a password, so
  the login screen can't test root's password, and every failed login
  waits the same 2 s fail delay. A failed password gets one fixed message
  unless PAM sent notes of its own; PAM's refusal of an account comes in
  its words only after the password was right. An account that PAM resolves to uid 0 is
  still refused by the daemon with a wrong password's message, and a login
  with no request from the login screen for 5 minutes is cancelled.
- `session.sock` is 0666. A connection is accepted only when the peer's
  logind session is one the daemon started or found at start, and its uid
  is that session's user. A session can say which interface it runs and ask
  for Switch User; it can't start a login or reach another session. A frame
  that doesn't decode is skipped, and the connection stays open. A refused
  peer is told `refused` before the daemon hangs up; a session refused five
  times in a row stops calling.

Frames are length-prefixed JSON of at most 64 KiB. A password typed at the
login screen travels in zeroed buffers from the login screen through the
daemon to the worker, which hands it to PAM. It is never logged, every
`Debug` of a message that can hold one prints it redacted, and a frame that
fails to decode is logged by kind only.

The daemon's unit carries no systemd sandbox. Every session descends from
it (daemon, worker, session), and namespaces, a system call filter, a
capability bounding set or `NoNewPrivileges` would bind each user's session
as well: no setuid `sudo`, a read-only `/usr`, filtered system calls. The
login screen is confined by its own unprivileged user and `no_new_privs`
instead.

A worker moves into its logind session's scope when the session opens, out
of the unit's cgroup. Restarting reprise-login, or a crash, leaves running
sessions alone; a login still in its PAM conversation ends with the daemon.
A session whose worker dies is left closing by logind: its compositor ends
itself once logind reports it closing, and that user's next login first
stops whatever is left of the session's scope.

### Switching users

A session that loses the screen locks itself, whether through Switch User
or a VT switch key. Logging in again as the same user at the login screen
checks the password through the `reprise-login` PAM service, then unlocks
that session through logind and switches to it, so no second password is
asked. A user who already has a session never gets a second one. If the
switch fails, the session is locked again; a reprise-login that starts locks
every session that isn't on screen. A remote login's background session
instead locks its own screen when it links to reprise-login again, so its
connected client keeps the desktop.

The session unlocks on logind's `Unlock`, so it can't tell reprise-login's
resume from `loginctl unlock-session` run by root, by an admin polkit lets
lock and unlock sessions, or by any process of the session's own user,
another login of theirs included.

`reprise-login` runs pam_nologin: while `/run/nologin` exists (early boot,
and the last minutes before a scheduled shutdown), the login screen refuses
logins and resumes alike. Switching to a session's VT (Ctrl+Alt+F7 for the
first) still reaches its lock screen.

While any user session runs, Shut Down and Restart on the login screen ask
for an administrator's name and password, checked through the
`reprise-login-check` PAM service; the account must be in `wheel`. That
stack refuses root before it asks for a password. Every refusal, a wrong
password or an account outside `wheel`, comes 3 s after the last answer, so
the reply doesn't tell who is an administrator. Shutting down ends every
session, unsaved work included. A session that isn't on screen can't take
the screen from the user in front of it: polkit refuses it logind's activation and VT switch,
and it holds no terminal to switch VTs through ([Background
sessions](#background-sessions)).

## Remote Desktop

The Remote Desktop server, `reprise-rdp`, offers TLS security only and logs
in the session user alone; any other name is refused before PAM sees it.
NLA (CredSSP) is not offered: its NTLM exchange can only be checked against
a password the server already holds, and PAM never hands one out. Under TLS
the client sends the password itself, inside the encrypted connection. A
client set to require NLA is closed, and the journal says to choose
TLS. Under reprise-login this server never listens on the network; the
login manager takes every connection and offers NLA with a Remote Desktop
password ([Remote logins](#remote-logins)).

It checks the password as the Unlock Computer dialog does, in a
`reprise --rdp-password-conversation USER` helper with the password on its
stdin, but under its own PAM service, `reprise-rdp`: `pam_unix` for `auth`
and `account` and nothing else. That stack has no pam_faillock, so wrong
passwords sent over the network never lock the account at the lock screen.
Lock mode's remote Unlock dialog runs the same helper under the same
service.
A further PAM prompt fails the login, and so does a helper with no verdict
within 30 s, which is killed. A refusal by the account stack (an expired
account or password) is logged with PAM's reason. On each login `pam_unix`
logs `setuid failed` at debug priority: it tries to become root before it
runs `unix_chkpwd` for the account check, which the helper, running as you,
can't do. `unix_chkpwd` is setuid root, so the check still runs.

The TLS certificate is self-signed, made on the first start for the host
name, and kept in `$XDG_DATA_HOME/reprise/rdp/` (`cert.pem`, and `key.pem`
readable by you alone). Nothing vouches for it, so the client asks the first
time: compare the SHA-256 fingerprint it shows with the one on Settings >
Remote Desktop. New Certificate... replaces it for the next connection, and
clients that kept the old one warn.

`reprise-rdp` throttles failed logins itself, and logs each one with the
client's address:

- **Per address.** The first three failures are free. The fourth starts a
  2 s back-off, doubling with each further failure up to 60 s. During it the
  address's new connections are closed before TLS, a login already under way
  from it is rejected without a check, and the journal logs each refusal. A
  success clears the address, and so do 20 minutes without a failure from it.
- **Every address.** 10 failures within 10 minutes pause every login for 10
  minutes: no password is checked and the journal says so once. A success
  doesn't clear the failures before it.

A login counts as a failure from the moment its password check starts until
PAM accepts it, so a client that drops the connection mid-check still
spends an attempt. A failed login is answered at once. A connection has 10 s
to log in; a check in flight when that time runs out is finished first.

Up to `max_viewers` connections are served at once, counting those still
logging in. One more is closed at once, before TLS, and never touches the
session in use. With `second_client = "replace"` one more is let in, and once
it logs in every other connection is ended, so a password is still needed to
take a session over. So that one address cannot hold every place through the
login time, a third connection from an address that already has two still
logging in is closed the same way, and so is every connection from an address
in its back-off: a throttled address holds no place while it waits. TCP keepalive ends a connection whose
client has gone silent within about 30 s.

Nothing is captured while no client is logged in: the portal session that
shares the screen and takes input opens after the login and closes when the
client leaves. The portal starts it without asking, as described under [The
Wayland socket](#the-wayland-socket); turning Remote Desktop on is the
consent. While it runs, the screen-sharing dot shows. In show mode this
computer's screen keeps showing the session, and its own keyboard and mouse
keep working, so someone at the computer sees and can take part in what the
client does; in lock mode it shows the lock ([Lock mode](#lock-mode)). When
a logged-in client leaves, its held keys and buttons are let go; when it was
the last one shown the session, the session then locks through logind, so
the computer is not left open. When it left because the screen it showed
went away, a monitor switched on a KVM say, the lock waits `[remote_desktop]
lock_grace_s` seconds (5 unless set, at most 60) for a client to reconnect:
one shown the session by then keeps it unlocked, from whichever address,
since it logged in again. Until then the session stays unlocked, on a
monitor that just came back too; `lock_grace_s = 0` locks at once. Every
other leave, a client closing, Disconnect, a dropped network, locks without
waiting. A client that logged in but never got the picture and input (with
no screen to share, say) leaves the session as it was. Stopping the server,
as turning Remote Desktop off does, locks the session before a shown
client's connection closes, and at once while the lock waits for a
reconnect; each client is then told the server ended it, so it doesn't
reconnect. A server that dies with a client shown the session locks it when
it next starts. `[remote_desktop] lock_on_leave = false` turns these locks
off: a client shown the session with it off doesn't lock on a stop or the
next start, and the last to leave doesn't lock while it is off. None of
these locks follow a lock-mode connection, as the session stayed locked here
all through it.

The client shares the clipboard only while it is shown the session.
`reprise-rdp` opens its data control connection once the client's clipboard
channel is up and the client has the picture and input, and closes it when
the client leaves, clearing a selection
the client still holds. A copy on either side only lists its formats to the
other, and `reprise-rdp` fetches the bytes when a program on the other side
reads them. With `[clipboard] history` on, Reprise reads every new copy for
the history, so a client's text or picture crosses as soon as it is copied.
Text and pictures cross, files and the primary selection don't. Text over
16 MiB and pictures over 64 MiB are refused, measured on the wire.

### Lock mode

With `[remote_desktop] laptop_screen = "lock"`, the default, the first
client shown the session starts lock mode, and it lasts until the last one
goes. With `[power] locker` set, the server runs every connection in show
mode instead.

For each session it starts without a dialog for the server, the portal reads
the `ConsoleLayout` property of `org.reprise.RemoteDesktop`, only when that
name's owner is the process that started the session, and passes the
client's monitors to the compositor through `reprise_input_v1`'s
`lock_screen` (version 9). reprise-login's hand-over server claims that name
before it takes a connection and stops while another process holds it; a
session it started whose layout the portal can't read shares no screen,
where the user's own server's would share every screen. The compositor
makes one virtual screen per
monitor of the first object's layout, in a block to the right of the
computer's own screens. Any later layout resizes them, a later object's
first one included, until the last object goes. A layout with no monitor, more than 16, a side outside
200 to 8192 pixels or a corner more than 131072 pixels from the origin is
ignored. The portal casts these screens alone and marks their streams
virtual (`source_type` 4); every other stream, the standby screen's
included, stays a monitor. A change in the number of monitors closes the
session.

As lock mode starts, the compositor locks the session if it isn't locked.
This is the session lock as at any other time: logind's LockedHint is true
and every one of the computer's own screens is covered. The computer's
keyboard and pointer reach only the Unlock dialog on those screens, besides
the keys that act at any lock screen. Its keys are read through a keyboard
state of their own, so no key or modifier held at the computer reaches a
window. The virtual screens have no cover and take only the client's input.

The virtual screens lock on their own: after `lock_after` of remote
idleness, from Start > Lock or the lock key on the remote side, and with
logind's `Lock`, which locks both sides. Their Unlock dialog takes only
remote input, and its password unlocks only the virtual screens; the
computer's own screens stay locked. The remote side's lock goes when lock
mode ends, while the computer's stays.

The virtual screens' Unlock dialog checks its password as a Remote Desktop
login does, under the `reprise-rdp` service, never the lock screen's
`reprise`. Wrong passwords typed on the remote side never reach pam_faillock
or the fingerprint reader, so they can't lock the account at the computer.
The dialog backs off after failures like a login from one address: the first
three are free, the fourth starts a 2 s wait that doubles with each further
failure up to 60 s, and a success or 20 minutes without a failure clears
them. During the wait the dialog refuses to check and says how long is left.

Unlocking at the computer, by password, fingerprint or logind's `Unlock`,
ends lock mode: every libei sender, the server's and any other the portal
let in, is cut off at once (its libei connection is closed and its keys
object goes inert), each object hears `ended(console_unlocked)`, the portal
closes those sessions, and the windows come back to the computer's own
screens. The server sees LockedHint false within 500 ms of the screens
going and tells its client another connection took over, so a client set to
reconnect doesn't lock the computer again. A locker client that takes the
lock during lock mode ends it the same way, with `ended(replaced)`, and its
lock stays; that client is told nothing and may reconnect.

An object made or given its first layout while a locker client holds the
lock hears `ended(replaced)`, and one while `[power] locker` is set hears
`ended(unavailable)`. Neither makes a screen, and the portal shares the
computer's own screens as in show mode. Any other answer to the layout, or
none within 2 s, fails the session's Start, and the client is told its
display failed to start. Lock mode ending any other way leaves the session
locked.

The server never locks on leave after a lock-mode connection, since the
session stayed locked here all through it. A connection counts as one only
when the server ran it as lock mode, having read `laptop_screen = "lock"`
and no locker when its run's first client was shown the session, and the
portal marked every stream it granted virtual. One whose portal session fell
back to the computer's own screens (a compositor without version 9,
`ended(unavailable)` or `ended(replaced)`) locks on leave as in show mode.

## Remote logins

With `[remote_desktop] enabled = true` in `/etc/reprise/login.toml`
([configuration.md](configuration.md#remote-logins)), reprise-login takes
RDP connections for the members of the `reprise-remote` group, logs them in
whether or not they have a session, and hands each connection to that
user's own `reprise-rdp`. Two more processes join the ones under [Login
manager](#login-manager):

- **The front**, `reprise-login-rdp.service`, runs as the system user
  `reprise-login-rdp` and is the only process that listens on the port. It
  runs each connection up to its login: X.224, TLS and, under NLA, CredSSP.
  It holds the TLS key (`/var/lib/reprise-login-rdp/key.pem`, 0600), the NT
  hash of every Remote Desktop password, and the password of each client
  logging in. It decides nothing: it sends the name and password to the
  daemon and waits for its verdict. reprise-login starts and stops it.
- **The hand-over server**, `reprise-rdp --hand-over`, runs as the user in
  their session, as the user unit `reprise-rdp-handover.service`, started by
  `reprise-rdp-handover.socket` at `$XDG_RUNTIME_DIR/reprise-rdp/handover.sock`
  (0600, in a 0700 directory). It takes connections from reprise-login only
  and stops when the last client leaves.

Two more sockets in `/run/reprise-login`:

- `front.sock` takes one link at a time, from the `reprise-login-rdp` uid
  only. A `Ready` frame must carry exactly one socket; a socket on any other
  frame breaks the link.
- `account.sock` is 0666. Any local user asks it about their own Remote
  Desktop password, by the uid `SO_PEERCRED` gives: one request per
  connection, read within 5 s, and one change per user at a time.

The design assumes the front can be broken into. A front run by an
attacker holds every Remote Desktop password's NT hash and sees the
password of each user who logs in meanwhile, as any TLS endpoint does, but
can't log in with a hash: the daemon wants the password whose hash matches.
The hand-over goes to the uid the daemon looked up, never one the front
names. So the front carries a full sandbox: `ProtectSystem=strict`,
`ProtectHome`, `PrivateTmp`, `PrivateDevices`, `ProtectProc=invisible`,
`NoNewPrivileges`, a `@system-service` system call filter, AF_UNIX and
AF_INET(6) only, 512 MiB and 64 tasks. It binds the port with
`CAP_NET_BIND_SERVICE` and drops it once bound. The daemon's unit has none,
for the reason under [Login manager](#login-manager); the front starts no
session, so nothing of a user's descends from it.

### Checks

For each login the daemon checks, in order:

1. The throttle below, and at most 4 logins in their check at once.
2. The name, without any `DOMAIN\` or `@host`: at most 32 bytes of
   `[a-z_][a-z0-9_.-]*`.
3. The account: it must exist, not be root, and be in the group
   (`reprise-remote` unless `[remote_desktop] group` says otherwise).
4. The secret. Over TLS, the client's password goes to PAM's
   `reprise-login-remote` service: `pam_shells`, `pam_env` and `pam_unix
   nodelay` for `auth`, then system-login's `account` and `session`, with no
   pam_faillock, so remote failures never lock the console. The worker
   answers PAM's first password prompt with it; any other prompt fails the
   login. Under NLA, the daemon compares the NT hash of the password the
   client delegated with the stored one in constant time, then runs
   `reprise-login-remote-nla`, which lets only a uid of 1000 or more through
   and runs `pam_shells`, `pam_nologin` and system-login's `account` and
   `session`. Both set `PAM_RHOST` to the client's address for `auth` and
   `account`.
5. A session: the user's own, or a new background one while fewer than
   `max_sessions` run.

Every refusal waits until 3 s after the login reached the daemon, whatever
refused it, so an unknown name, a name outside the group, root and a wrong
password look the same to the client. The daemon's journal (`journalctl -b
-u reprise-login`) names the reason. A client has 60 s from its TCP accept
to send its credentials, time for its user to answer a certificate or
password prompt, and the login then has 8 s to reach its session. The
caps on connections not yet logged in bound what slow clients hold.

### Throttle

Two throttles count failed logins, with the numbers of the Remote Desktop
server's ([Remote Desktop](#remote-desktop)): the daemon's counts TLS login
passwords per client address, and the front's counts NLA Remote Desktop
passwords per IPv4 address or IPv6 /64, from the client's NTLM AUTHENTICATE.
Neither feeds the other, so each kind of secret has its own budget.

- **Per address.** Three failures are free; the fourth starts a 2 s
  back-off, doubling up to 60 s. Meanwhile the front refuses the address's
  new logins: before TLS after TLS failures, at the start of CredSSP after
  NLA failures. A success clears the address, and so do 20
  minutes without a failure from it.
- **Every address.** 10 failures within 10 minutes pause every login of that
  kind for 10 minutes, a right password included, and the journal says so
  once. Restarting reprise-login clears both throttles.

xfreerdp opens each TLS logon twice, so one mistyped password from it
counts two failures. Before a login the front also caps connections: 32 in
all, 2 per IPv4 address or IPv6 /64, and 256 KiB read from each.

### Hand-over

Once the daemon accepts a login, the front puts the TCP socket into kernel
TLS and sends it to the daemon over `SCM_RIGHTS`, with the connection's TLS
keys and a recording of the exchange with the password and auto-reconnect
cookie blanked. The daemon
opens the user's `handover.sock` without following symlinks or crossing
mounts (`openat2` with `RESOLVE_NO_SYMLINKS` and `RESOLVE_NO_XDEV`): the
runtime directory, `reprise-rdp` and the socket must belong to the user and
not be group or world writable, and `SO_PEERCRED` must name the user, so
only a listener the user made gets the connection. The hand-over server
takes links from root only and only a TCP socket, and replays the
recording. It takes only a recording whose login names
its own user with an empty password, and closes a fresh TCP connection. The
password and the NT hash never leave the front and the daemon.

TLS is 1.2 or 1.3 with AEAD suites only. After the hand-over, a TLS 1.3 key
update is followed, and any other handshake record, TLS 1.2 renegotiation
included, ends the connection.

The front makes its self-signed certificate for the host name on its first
start, in `/var/lib/reprise-login-rdp/`. Every user's connections use it,
and its fingerprint shows on Settings > Login Manager and on each user's
Remote Desktop page. Settings doesn't replace it: delete `cert.pem` and
`key.pem` there and restart the front (`sudo systemctl restart
reprise-login-rdp`). Under `ProtectHostname` the front reads the host name
once, so a new name, which the NLA name forms and the TLS server name use,
applies when the front restarts.

### Remote Desktop password

NTLM checks a client's proof against a password the server already holds,
which PAM never hands out. So NLA logs on with a Remote Desktop password
each user sets for themselves on Settings > Remote Desktop. It is kept as
its NT hash, unsalted MD4 as NTLM needs, which is fast to crack from a
copy. The file, `/var/lib/reprise-login/remote-passwords`, is 0600 root in a
0700 directory. Use a strong Remote Desktop password: it needs 8
characters or more and must differ from the login password.

- Setting or clearing it takes the login password, checked through
  `reprise-login-check` (system-auth). Clearing your own is always allowed,
  even outside the group.
- Locking the login password (`passwd -l`) leaves the Remote Desktop
  password working. To stop it, clear it on Settings > Remote Desktop, or
  take the user out of `reprise-remote`. An expired account
  (`chage -E`) is refused either way, by the account step.
- Setting one leaves TLS logins on: a client set to TLS still logs on with
  the login password.
- An entry whose account is gone, or whose name now has another uid, is
  dropped when reprise-login starts.

NLA takes the name as `user`, `.\user`, `HOST\user` (the host name's first
label), `user@host` or `user@`. TLS strips any `DOMAIN\` or `@host`. mstsc
is untested.

### Background sessions

A login for a user with no session starts one through the user's login
shell, on the first free VT from 7 up that isn't on screen. It is a logind
session on seat0 with `Remote=no`, so polkit treats it as a local session
that isn't active. It comes up locked: switching to its VT at the computer shows
its Unlock dialog, never its desktop, and its clients always run in lock mode
([Lock mode](#lock-mode)). Off the seat a session runs no lid action and no
idle suspend, and its power requests go through logind and polkit as an
inactive session's. Once someone switches to it at the computer, it counts
as a console session.

No session holds a controlling terminal: the PAM worker holds the session's
VT as its own, so no process of the session can switch VTs through the
kernel. The polkit rule `/usr/share/polkit-1/rules.d/50-reprise-login.rules`
refuses `org.freedesktop.login1.chvt` to every session that isn't active, so
`loginctl activate` and `chvt` from a background session, or from one left
behind by Switch User, fail. Only reprise-login, as root, switches.

While a background session runs, reprise-login holds an `idle` block
inhibitor; while a client is connected to any session, `sleep:idle`, so the
computer doesn't sleep under a client. Each session's compositor reads that
`sleep` inhibitor, held by root as "Reprise login manager", as a client
connected elsewhere: idle suspend waits, and a shut lid darkens the
internal panels instead of running `[power] lid_close`. The login screen
reads it the same way: while it is held, the screen takes logind's
`handle-lid-switch` and a shut lid darkens the panels; otherwise logind
acts on the lid.

A background session whose last client left ends `[remote_desktop]
end_disconnected_after` minutes later, unless a client comes back first;
logind ends its processes, unsaved work included. With it unset, and for a
console session, the session runs until its user logs off.

### Accepted risks

- **Names with a Remote Desktop password show.** The front picks NLA or TLS
  from the user name in the client's first, unauthenticated packet (its
  `mstshash` cookie), NLA when that name has a Remote Desktop password. So
  anyone who reaches the port can tell which names have one. Names leak,
  never passwords.
- **The chvt rule applies to the whole computer.** It sits in
  `/usr/share/polkit-1/rules.d` and applies whether or not reprise-login
  runs: under another display manager too, a session that isn't active
  can't activate itself. Display managers switch as root, so only scripts
  that activate their own inactive session lose out.
- **Hashes outlive group membership.** Taking a user out of `reprise-remote`
  stops their logins at once, as the group check comes before the hash
  compare, but their NT hash stays in the file and the front still gets it
  until they clear it.
- **Wrong passwords on account.sock count toward pam_faillock.**
  `reprise-login-check` includes system-auth, so a process running as a user
  can lock that user's console login for faillock's window by sending wrong
  login passwords, as `su` can.
- **A right administrator password shows through an account refusal.**
  When the password in the login screen's administrator form is right but
  PAM's account check refuses (an expired or locked account), the refusal
  comes at once in PAM's words instead of after 3 s, so whoever is at the
  console learns the password was right. The console login behaves the
  same.
- **Wrong passwords in the administrator form count toward pam_faillock.**
  `reprise-login-check` includes system-auth, so anyone at the login screen
  can lock an administrator's console login for faillock's window by typing
  wrong passwords for that name.
- **Only root's inhibitor counts.** The compositor counts a client
  elsewhere only for a `sleep` block inhibitor named "Reprise login manager"
  that uid 0 holds. Any user can still take inhibitors of their own.
- **Inactive polkit grants.** A background session gets polkit's
  `allow_inactive` answers, which for some actions are wider than a remote
  session's: NetworkManager's network control (a remote user can take the
  connection down), inhibiting shutdown and sleep, the power key and lid
  inhibitors, and realtime priority. All affect availability only.
- **Anyone can pause remote logins.** 10 wrong passwords every 10 minutes
  from any addresses keep one kind of login paused.
- **A late refusal tells the password was right.** A login refused after its
  password passed (the most sessions, no free VT, a failed start, the 8 s
  deadline) counts no failure, so its address doesn't back off. Under NLA
  the client learns it from the early result anyway, and a reconnect lands
  in the session the first one started.
- **The journal can fill.** The front and the daemon log one line per
  refused connection, so a storm of connections can push older lines out of
  a journal capped by size.
- **FUSE over the runtime directory.** A user who can mount FUSE over
  `/run/user/<uid>` with `user_allow_other` (off by default in
  `/etc/fuse.conf`) can stall the daemon at their own hand-over.
- **System accounts in the group.** The TLS stack refuses root only. A
  system account that an administrator adds to `reprise-remote`, with a
  shell in `/etc/shells`, can log in over TLS; the NLA stack refuses every
  uid below 1000.
- **The front can reach the system bus.** Its sandbox leaves the system
  bus socket visible, though it uses only `front.sock` and the journal.

## Keyboard shortcuts

A window can ask Reprise for every keyboard shortcut through
`zwp_keyboard_shortcuts_inhibit_manager_v1`, as VM viewers and remote
desktop clients do. While its surface has the keyboard, it gets the key
binds and the Super key. A balloon says so the first time a program takes
them.

These stay with Reprise under an inhibitor:

- the power, sleep and media keys, and the VT switch keys
- the binds that move no focus: lock, magnifier, colour filter, keyboard
  layout, Do Not Disturb, night light, the screenshots, volume, brightness,
  playback and Wi-Fi
- open menus

Super+Shift+Escape gives the binds back until that window next gains the
keyboard. The same chord ends an input capture (Deskflow, InputCapture
portal sessions). It can't be taken by an inhibitor or a capture.

## Focus

A new window takes the keyboard when it maps if any of these holds:

- it presents a trusted activation token, which Reprise mints when it
  starts a program or you click a tray icon or a notification, and which a
  program can mint only from an input event in its own focused window, or
  within 3 s of a drag dropped on one of its windows
- Reprise started its program (Start menu, launcher, desktop, Run, a key
  bind, a tray icon or a notification) within the last 30 s
- it shares a Wayland connection with the focused window
- its process is the focused window's, descends from it or sits in the
  session the focused process leads (a dialog, a second window, a program
  it started)
- the focused window owns it, as a portal's print dialog is made the child
  of its caller
- it is a prompt of `reprise-portal`, `reprise-polkit` or `reprise-drives`
- it was started from a terminal multiplexer pane whose client runs in the
  focused terminal
- no window has focus

Any other window opens behind the one you're typing in and flags its task
button. While the taskbar has the keyboard, it opens on top with its button
flagged and the keyboard stays on the taskbar. Under the lock no new window
takes the keyboard.

A program in the background can flag its task button (an untrusted
activation token, X11 urgency) but can't take the keyboard through
`xdg_activation_v1`. A program trusted with foreign toplevel management can,
since that protocol focuses any window it names.

## The portal

`reprise-portal` is the xdg-desktop-portal backend. It answers only calls
from the process that owns `org.freedesktop.portal.Desktop` on the session
bus; any other caller gets AccessDenied. App ids, parent windows and
restore data are taken on the frontend's word, and the frontend reads the
app id from the sandbox.

What a consent prompt remembers, and where:

- **ScreenCast and RemoteDesktop:** when the app asks to persist and you
  tick Remember, the portal returns a restore token, and xdg-desktop-portal
  keeps it in its permission store (tables `screencast` and
  `remote-desktop`). A token names the app it was issued to and restores
  only for that app. Without Remember nothing is kept.
- **Screenshot:** the prompt has no Remember. xdg-desktop-portal keeps its
  own `screenshot` permission, and once that says yes a non-interactive
  screenshot is taken without a prompt.
- **InputCapture:** asks every session, remembers nothing.
- **GlobalShortcuts:** an app with `[[global_shortcuts]]` entries in
  `config.toml` is allowed; a Deny adds it to `[portal] shortcuts_denied`
  and it isn't asked again.
- **Access** (camera, background): answers go to xdg-desktop-portal's
  permission store.
- **Location:** Reprise's geoclue agent keeps its own answers.

The share picker also keeps the last screen or window each app picked
(`$XDG_STATE_HOME/reprise-portal/lastpick.toml`), but only to preselect it:
you still confirm.

To revoke: `flatpak permissions` lists the permission store and `flatpak
permission-remove <table> <id>` removes an entry. The Keyboard page of
Reprise Settings forgets a program's global shortcuts, and Settings >
Privacy lists and forgets location answers. There is no page for screen
sharing and remote control grants yet.

## Programs and files

- **Launchers.** A `.desktop` file in a folder (the desktop, Explorer)
  runs only when it is executable and carries the `user.reprise.trusted`
  extended attribute, which Allow Launching sets. Extracting an archive
  restores the execute bit but not the attribute, so a downloaded launcher
  can't pose as a program.
- **Programs and scripts.** A file with the owner's execute bit that starts
  with `#!` or is an ELF binary runs when you open it, from the desktop,
  Explorer or a search. A script runs with no window (`script_open = "run"`,
  the default) or in your terminal (`script_open = "terminal"`). The
  execute bit is the only gate: an archive that restores it gives you a
  program that runs on a double-click.
- **App scopes.** A program started from a desktop entry runs in a systemd
  scope `app-reprise-<app id>-<n>.scope`, where xdg-desktop-portal reads a
  host app's id. The scope is for identification and cleanup, not
  confinement.
- **Ending the session.** With `[session] end_processes = true` (the
  default), Log Off stops the `app-reprise-*` scopes and, after a display
  manager login, terminates the login session, so windowless programs Reprise
  or the session started go too. A program started outside the session (a
  user service, an SSH login) keeps running. A nested Reprise stops neither.

## Out of scope and known gaps

- **No per-app Xwayland.** Every X11 program shares one server; see
  [X11 programs](#x11-programs). Run a program you don't trust as a
  Wayland client or in a Flatpak without X11 access.
- **Privileged globals have no allow-list.** Any unsandboxed program can
  capture the screen, read the clipboard, inject input or lock and unlock
  the screen through the globals above. Sandboxing is the only way to keep
  a program from them.
- **No clipboard portal.** A sandboxed program gets the clipboard only while
  its window has the focus, or through `[clipboard] trusted_apps`.
- **No permissions page.** Remembered screen sharing and remote control
  grants are revoked with `flatpak permission-remove`.
- **Background sessions get polkit's inactive grants** under
  reprise-login; see [Accepted risks](#accepted-risks).
- **Same-user code.** A program running as you can ptrace or preload the
  portal, the compositor or anything else of yours. Reprise doesn't defend
  against that, and doesn't treat it as a vulnerability.

## Reporting

Report a hole in any of these boundaries privately, as
[SECURITY.md](../SECURITY.md) describes.
