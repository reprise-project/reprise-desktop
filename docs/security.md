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
  screenshot draws none. A program's second still within 3 s of its first
  shows the screen-sharing dot for 3 s, so screenshots taken in a loop show
  it and a single one does not. The one exception is the Remote Desktop
  server's capture, which goes without it unless `[remote_desktop] capture_border`
  is set: the portal marks that capture's connection through
  `reprise_input_v1`, which only the installed reprise-portal can bind, and
  only for a session it starts without a dialog for the server (its enabled
  unit, or reprise-login's hand-over server; see below).
- **Session lock** (`ext_session_lock_manager_v1`): locks the screen and
  draws the lock screen. See [The lock screen](#the-lock-screen).
- **Output management, power and gamma** (`zwlr_output_manager_v1`,
  `zwlr_output_power_manager_v1`, `zwlr_gamma_control_manager_v1`):
  rearranges, turns off or recolours the screens.
- **On top** (`reprise_on_top_v1`): keeps one of the program's windows
  above the others. A consent prompt still stacks above it
  ([Consent prompts](#consent-prompts)).

These are the protocols panels, lockers, clipboard managers, screenshot
tools and input methods are built on; [protocols.md](protocols.md) lists the
tools. Reprise has no allow-list for them.

### Sandboxed programs

A client is sandboxed when it connects through a socket a sandbox opened
with `wp_security_context_manager_v1` (Flatpak does this), or when its
process sees `/.flatpak-info` (a Flatpak app that connected without one) or
runs in Flatpak's `app-flatpak-*.scope`, or when it runs in a strict or
devmode snap: under a `snap.*` AppArmor profile, or in a `snap.*` scope
where AppArmor shows no label. A classic snap runs unconfined, so it counts
as a host program. Reprise looks at the process that connected through a
pidfd. One that has exited by then counts as sandboxed, so a Flatpak app
can't pass as a host program by connecting and exiting before the
connection is accepted. A process that hides its root (a non-dumpable one,
such as a password manager or a program with file capabilities) is judged
by its cgroup instead, and counts as sandboxed only when it also runs in a
pid namespace of its own, as bubblewrap's are. The client log marks a
process it could inspect neither way ", not inspected".
A sandboxed client sees none of the globals above. It also doesn't see
`wp_security_context_manager_v1` itself, so it can't open a socket of its
own under another app id. The full list is `PRIVILEGED_GLOBALS` in
[`src/sandbox.rs`](../src/sandbox.rs). A sandboxed program reaches screen
sharing, screenshots, remote control and input capture through the portal,
which asks you first.

`[clipboard] trusted_apps` gives the two data control globals back to the
Flatpak apps it names, and nothing else. The app id is the one Flatpak put
on the security context, not one the app chose, and only Flatpak's
contexts are matched. A listed app can read everything you copy. Settings >
Privacy > Clipboard Access ticks apps into the list. See
[flatpak-clipboard.md](flatpak-clipboard.md).

One more global, `reprise_input_v1`, carries remote control, input capture,
global shortcuts, the list of sandboxed apps with a window, the colour
picker, from version 9 lock mode ([Lock mode](#lock-mode)) and from version
10 the app each libei sender or keys object serves (`connect_app_sender`,
`create_app_keys`, so the screen-sharing dot can name it) between the
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
`/usr/local/bin` and is the process that asked. Under reprise-login it also
skips it, for any app id, when the process that asked is the main process of
the active `reprise-rdp-handover.service` running `reprise-rdp` from one of
those paths. Either way the session is the server's: its input lights no
dot, types into the lock screen, and is not kept from the power, sleep, VT
switch, radio and touchpad keys. A sandboxed app's requests
reach the portal through its bus proxy, so they never pass. Such a session
gets the keyboard, the pointer and every screen whose top left lies at a
non-negative position in the layout, or every screen when none does, and
leaves no restore token behind. In lock mode it gets the screens made for
the client instead. As with `reprise_input_v1`, this is no boundary against
code running as you.

The screen recorder, `reprise-record`, also starts its ScreenCast sessions
without the dialog, but only for a recording you asked for: the
compositor's recording key, the region picker's Record mode or the tray.
When it starts the recorder for one, the compositor sends the portal the
recorder's pid over `reprise_input_v1`. The portal skips the dialog only for
a Start from that process, running `reprise-record` from `/usr/bin` or
`/usr/local/bin`, within 10 s of the grant, and only once. A recorder started
any other way, from an autostart entry, the Run box, a launcher or a
terminal, gets the dialog like any other app. Such a session gets every
screen, with the pointer drawn in, and leaves no restore token behind; its
captures light the red border, around the area while an area is recorded.
The compositor makes the recording's file and hands it to the recorder
open; the recorder never opens a name under Videos, so a program that can
write there cannot point the recording at another file by swapping the
name. The recorder waits for its portal answers from the portal's own bus
name, and Stop signals it through a pidfd, which never reaches another
process that got its pid.

## X11 programs

All X11 programs share one Xwayland server, which Reprise starts on the
first X11 connection. A Flatpak app with X11 access joins the same server.
Inside it there is no isolation, as on any X11 desktop: an X11 program can
read the keys typed into other X11 windows, read their pixels, grab the
pointer and inject input into them through XTEST.

The server listens on `/tmp/.X11-unix/X<n>` and on the abstract socket of
the same name, and admits only a client that presents its
MIT-MAGIC-COOKIE-1. Reprise writes a random cookie for each session to
`$XDG_RUNTIME_DIR/reprise/xauth-<n>` (mode 0600) and hands programs its path
in `XAUTHORITY` beside `DISPLAY`; Flatpak copies it into an app with X11
access. A Flatpak app without X11 access but with network access shares the
host's network namespace and so reaches the abstract socket, but it has no
cookie to present and is refused. Code running as you outside a sandbox can
read the file, the same boundary as the Wayland socket.

Wayland programs are not exposed to this. An X11 program sees only the X11
windows, hears keys only while an X11 window has the focus, and can't
capture or drive Wayland windows.

A copy made through X11 stays with the program that made it, in the
Xwayland server, and any X11 program can read it there at any time, whatever
has the focus, until it is cleared or replaced. Reprise doesn't stand between
X11 programs. A Flatpak app that uses X11 is not isolated from other X11
programs: they can read its copies, as they can its keys and pixels.

The clipboard bridge between X11 and Wayland opens while an X11 window has
the focus: only then may an X11 program read the Wayland clipboard or primary
selection, or replace them. Electron apps, Proton Pass among them, open X11
windows by default on Reprise, so their copy buttons take this path.
One exception serves a Flatpak app whose window is a Wayland one and whose
copy buttons fall back to X11: while a Wayland window of Flatpak app P has the
focus, an X11 program may replace the clipboard (not the primary selection)
when the window it names as the owner was created by a process right in P's
`app-flatpak-<P>-<n>.scope` under the user manager's `app.slice`, in a pid
namespace, under no snap profile, with a readable `/.flatpak-info` naming P.
P comes from the security context Flatpak opened for that window, not from
the app. Nothing is read through the exception.

Each take is ruled when a TARGETS reply reaches Reprise, from the focus and
the selection's owner at that moment; nothing is remembered from an earlier
take. An X11 program letting go of the clipboard empties it, whatever has
the focus, when an X11 program's copy is on it. A refused take leaves the
clipboard as it was. While the clipboard holds an X11 copy, a Wayland paste
reads from whichever X11 program owns the selection now, so any X11 program
that takes it, naming any window as the owner (the root window included) and
with no focus, replaces what Wayland windows paste. The paste still offers
the earlier copy's types, and the clipboard history keeps showing the earlier
copy. That holds for a copy made in an X11 window and for one taken through
the exception alike.

X11 doesn't say which client took a selection, only which window it named as
the owner, and a client may name any window. So while P has the focus, any
X11 program that names one of P's X11 windows replaces the clipboard as P
would. The creator's pid is the one Xwayland recorded at its connection, so a
pid reused since then names whatever process holds it now: an X11 program
whose connecting process exited passes as P while one of P's processes holds
that pid, with no window of P's named.

An X11 window's process id comes from its own `_NET_WM_PID` property, so
the focus rules below take an X11 program's word for which program it is.

## The clipboard history

`[clipboard] history` keeps the last 25 copies in the compositor's memory
and never on disk; they go when the session ends. A copy whose
`x-kde-passwordManagerHint` type reads `secret` is not kept, and that type
is all Reprise reads of it, so the password never leaves the program that
copied it. KeePassXC, KWalletManager, the Bitwarden desktop app and
`wl-copy --sensitive` mark their copies this way, and Firefox marks its
saved passwords and whatever is copied in a private window.
A marked copy also leaves the clipboard when its program quits. An unmarked
copy is kept like any other, a password copied out of a text file
included. A Remote Desktop client's copy is marked when the client lists
`ExcludeClipboardContentFromMonitorProcessing` or
`CanIncludeInClipboardHistory` with it (see
[Remote Desktop](#remote-desktop)).

Nothing is recorded while the desktop is locked, and Super+V doesn't open.
What was recorded before the lock stays until the session ends or
`history` is turned off.

No program can list the history. An entry reaches a program only as the
clipboard: when you pick it in Super+V, or as the newest copy kept after
its program quit. The clipboard is read by data control clients and the
focused window. A sandboxed program sees no data control global unless
`[clipboard] trusted_apps` names it, and Reprise has no clipboard portal.

The compositor's memory isn't locked, so a kept copy can reach swap, and a
core dump of a crash holds the history.

## The lock screen

The lock follows `ext-session-lock-v1`. While it holds, no window but an
external locker's gets the keyboard or the pointer and virtual keyboards are
dropped. Of the keys, only the power, sleep and media keys (but not the
Wi-Fi key), the VT switch keys and the layout switch act; the rest go to the
lock screen.

An app's RemoteDesktop session (its libei sender and its NotifyKeyboard
keys) presses, types, moves and scrolls nothing while the desktop it lands
on is locked, by the lock screen or a locker. In lock mode that desktop is
the remote side: an app's input reaches the remote side's windows while that
side is unlocked and never the computer's own lock. The keys and buttons it
holds are released as the lock comes up, so none stays down at the lock
screen.
Locked or not, it never presses the power, sleep, VT switch, radio
(Wi-Fi, Bluetooth, the airplane-mode key), touchpad or rotation lock keys. Reprise's own remote desktop
server is not an app here: its viewer types the password into the lock
screen, in lock mode and at the computer's own lock, through libei or its
keys object.

The built-in lock screen is the login screen's locked state, drawn by the
session's own compositor: it lists only the session's user, with the name
fixed, and never another user's tile, so nobody types another user's
password into this session. Cancel and Escape only clear the field, and
Switch User locks before it asks reprise-login for the login screen, which
runs as `reprise-greeter` and gets no picture of the session. The screen
checks the password in a helper
process: the compositor starts itself again as `reprise
--password-conversation=USER` and writes the password to its stdin, never
to argv or the environment. The helper runs PAM's `reprise` service
(`/etc/pam.d/reprise`, or `login` when that file is missing) and reports
the verdict on stdout; only a success unlocks. It runs PAM's `auth` alone,
not `account`, so an expired password, `/run/nologin` or a time rule never
keeps the right password out of the session that runs behind the lock. A further prompt from the
stack (a one-time code, a security key PIN) relabels the field, and its
answer goes to the same helper's stdin. A helper that hangs is given up
after 90 s and the screen asks again. Reprise zeroes the password copies
it holds. The field stops at 512 bytes, the longest answer the helper
reads.

The lock's power button opens a menu. Stand By is in it whenever `[power]
standby` is set. Restart and Turn Off are in it only under reprise-login
(another display manager's autologin can't be read), while logind lists no
other session holding someone's work on any seat, remote ones included, and
`/etc/reprise/login.toml` sets no `autologin_user`; so the lock never ends
another user's work, nor restarts into a desktop that logs itself in
unlocked. Greeters, lock screens, service managers and closing (logged-off)
sessions hold no work; any other class does, one a later systemd adds
included. The login screen counts sessions the same way before it asks for
an administrator. The check runs when the menu opens and again when a row
is picked; failing then, the row ends nothing and the screen says to unlock
first. A session that starts in the second or two between that check and
logind's call isn't counted. Each row runs its `[power]` command, the
default ones on logind without overriding its block inhibitors. Restart
and Turn Off close the session's programs without their save prompts, as
none can show over the lock. While the session is locked logind is called
without interactive authorization, so a polkit rule that asks for a
password refuses on the lock instead of leaving the screen faded out under
a prompt nobody can reach. A refusal on the lock names no program and
quotes no command's error; the message box under the lock, seen after
unlocking, does. Only the computer's own keyboard and pointer reach the
menu, and its rows take no access keys, so a letter meant for the password
picks nothing. There is no power in lock mode on either side, nor while a
remote viewer drives the seat, and an open power menu closes when either
begins or a locker takes over. Without `[power] standby` outside
reprise-login the menu would be empty, and no power button shows.

Under reprise-login the lock draws the login screen's background, reading
`/etc/reprise/login.toml` and `/var/lib/reprise-login/background` (root's,
0644) off the loop; it writes neither.

When `/etc/pam.d/reprise-fingerprint` is installed, a second helper runs
the fingerprint conversation while the screen shows. Either one succeeding
unlocks at once; on Blue and Glass the covers then fade out over the desktop
for 200 ms, taking no input. A fingerprint conversation that ends without success (too many
fingers that did not match) starts again after 2 s, the wait doubling up to
30 s; one that finds no reader or no enrolled finger does not.

`[power] locker` runs an external locker (swaylock, hyprlock, gtklock)
instead of the screen. The lock stays Reprise's: the screens are covered and
focus is refused as with the screen, and the locker draws on top and does
its own PAM check. The fingerprint helper doesn't run under a locker, which
may claim the reader itself. If the locker crashes the session stays locked
and the screen comes back. A locker may take over a lock that has none, the
screen's or a crashed locker's, and unlock it. A locker whose lock has ended
can't unlock a lock taken after it.

The lock screen shows no sharing dot; a cast of a screen goes on showing
the lock.

The screen saver draws above the lock. Photos runs as Blank while the
desktop is locked, so no picture from `[screensaver] folder` shows to whoever
is at the locked computer.

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
  `/var/lib/reprise-login/state.toml` (0600, in a 0711 directory every
  account may pass through but not list, for the login screen's picture
  below; any account can still tell by name whether a file there exists and
  when it changed). Before it names the picture to the login screen it
  checks that it is a file of at most 160 MB whose PNG header gives at most
  7680x4320 pixels' worth and 16384 a side; it decodes none of it, and the
  login screen falls back to its default when the rest doesn't decode.
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
  directory and its background from `/var/lib/reprise-login/background`,
  and can read no more of a user's files than any other account.
  It listens on no Wayland socket, so the user services logind starts for
  `reprise-greeter` can't reach it; `[login] greeter_socket` opens one for
  testing.
- **Sessions** run as their user, through the user's login shell, with the
  environment PAM set.
- **The settings helper**, `reprise-login-settings`, runs as root through
  `pkexec`, which asks for an administrator's password on each save. It
  takes the settings and, for the background, a picture's bytes on stdin:
  Settings reads the picked file as the user, so the helper never opens a
  path a user named and no symlink can point it at another account's file.
  It refuses a picture over 32 MB, or one that isn't a PNG, JPEG or BMP of
  at most 16384 pixels a side and 64 million pixels that decodes whole
  within the decoder's memory limit, before it writes anything. It decodes
  with the same pure-Rust decoders and limits as the compositor, and
  Settings decodes the file the same way before it asks for the password,
  saying which of these it refuses. The copy holds only the pixels, scaled
  down to 7680x4320 pixels' worth in the picture's own shape, turned the
  way the photo's orientation says, as an 8-bit PNG: a photo's EXIF and
  text stay behind, since every account can read it. It goes to
  `/var/lib/reprise-login/background` (root's, 0644) through a temporary
  file opened `O_EXCL | O_NOFOLLOW`, renamed over whatever is there, a
  symlink included, without following it, once the settings are written.
  Settings that show no picture remove it.

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
A worker leads its session, so when it dies logind marks the session
closing and the session's compositor ends itself. Should logind still list
the session open, with no PAM handle left to close it, the daemon ends it
through logind, unsaved work included. It acts only when its own table has
that session id for the uid logind lists, and a failed read of logind's
sessions ends nothing. A worker
that exits after `pam_close_session` (a logout) leaves its session closing:
its compositor ends itself once logind reports it closing, and that user's
next login first stops whatever is left of the session's scope.

### Switching users

A session that loses the screen locks itself, whether through Switch User
or a VT switch key. Logging in again as the same user at the login screen
checks the password through the `reprise-login` PAM service, then unlocks
that session through logind and switches to it, so no second password is
asked. A user who already has a session never gets a second one; a session
logind can't describe after reprise-login restarts counts as running, so its
user's login waits until logind describes it. If the
switch fails, the session is locked again. A session off the seat locks its
own screen when its link to reprise-login drops, as when the daemon restarts
or dies, so a daemon that dies between unlocking a session for a resume and
switching to it leaves it locked. In lock mode only the computer's side
locks, and the connected client keeps its desktop. A reprise-login that
starts locks no session: a logind lock would lock a session's client out
too.

The login screen marks a session's tile Locked from its logind
`LockedHint`. reprise-login reads the hint from logind itself; a
`PropertiesChanged` signal on a session's path only prompts that read, so
another bus client sending one changes no tile.

The session unlocks on logind's `Unlock`, so it can't tell reprise-login's
resume from `loginctl unlock-session` run by root, by an admin polkit lets
lock and unlock sessions, or by any process of the session's own user,
another login of theirs included. Off the seat with its link to
reprise-login down, it refuses `Unlock` and stays locked: off the seat an
unlock comes from reprise-login resuming it, and with no link no resume is
under way.

`reprise-login` runs pam_nologin: while `/run/nologin` exists (early boot,
and the last minutes before a scheduled shutdown), the login screen refuses
logins and resumes alike. Switching to a session's VT (Ctrl+Alt+F7 for the
first) still reaches its lock screen.

While any session holding someone's work runs (counted as the lock counts
them), Shut Down and Restart on the login screen ask for an administrator's
name and password, checked through the `reprise-login-check` PAM service; the account must be in `wheel`. That
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

Without reprise-login the server listens on its port on every address, IPv6
and IPv4 alike, and gives up with `Address already in use`, shown on
Settings > Remote Desktop, when another socket holds the port on any of
them. Another local user whose program holds the whole port first still
takes every client's connection; only a certificate fingerprint other than
the one Settings shows tells the client. Under reprise-login the login
manager's socket holds the port from boot.

It checks the password as the lock screen does, in a
`reprise --rdp-password-conversation USER` helper with the password on its
stdin, but under its own PAM service, `reprise-rdp`: `pam_unix` for `auth`
and `account` and nothing else. That stack has no pam_faillock, so wrong
passwords sent over the network never lock the account at the lock screen.
Lock mode's remote lock screen runs the same helper under the same
service.
A further PAM prompt fails the login, and so does a helper with no verdict
within 30 s, which is killed. A refusal by the account stack (an expired
account, `chage -E`) is logged with PAM's reason. A password that must be
changed (`chage -d 0`) still logs in, and the server logs that it has
expired: the server serves the session that already runs, and the change is
asked at the next login that starts a new one. On each login `pam_unix`
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
spends an attempt. A wrong password is answered after PAM's failure delay,
about 2 s; a wrong name, a back-off and the pause are answered at once. A
connection has 10 s to log in; a check in flight when that time runs out is
finished first.

Up to `max_viewers` connections are served at once, counting those still
logging in. One more is closed at once, before TLS, and never touches the
session in use. With `second_client = "replace"` one more is let in, and once
it logs in every other connection is ended, so a password is still needed to
take a session over. So that one address cannot hold every place through the
login time, a third connection from an address, or for IPv6 from the same
/64 network, that already has two still logging in is closed the same way, and so is every
connection from an address in its back-off: a throttled address holds no
place while it waits. A few addresses can still hold every place: with the
default `max_viewers = 4`, two IPv4 addresses or two IPv6 /64s that open two
connections each, send nothing and reconnect every 10 s keep the owner's
client out without spending a login. Under reprise-login the front's own
limits apply instead ([Throttle](#throttle)). TCP keepalive ends a
connection whose client has gone silent within about 30 s.

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
no screen to share, say) leaves the session as it was. Under
`second_client = "replace"`, a login that ends a client shown the session
counts as shown from its login, so the replaced client's leave doesn't lock
the session under it; any other login counts from its first frame. Stopping the server,
as turning Remote Desktop off does, locks the session before a shown
client's connection closes, and at once while the lock waits for a
reconnect; each client is then told the server ended it, so it doesn't
reconnect. A server that dies with a client shown the session locks it when
it next starts. These locks go through logind: one that logind refuses or
doesn't answer within 5 s, after a leave or at a start, is tried again every
10 s until it takes or a client is shown the session again, and a hand-over
server with no client stays up until then; one at a stop is tried again at
the next start. `[remote_desktop] lock_on_leave = false` turns these locks
off: a client shown the session with it off doesn't lock on a stop or the
next start, and the last to leave doesn't lock while it is off. None of
these locks follow a lock-mode connection, as the session stayed locked here
all through it.

Disconnect in the session ends every client and leaves the session running.
It is offered only on the remote side of lock mode, or off the seat while a
remote client drives the session (the remote desktop server's libei sender
with a seat and devices counts; an app's sender lights the screen-sharing
dot instead), in the Log Off and Turn Off dialogs and Glass's Shut down options.
Nothing opens those from a locked screen, the login screen or a client that
hasn't logged in, and a press checks again that Disconnect is still offered,
so a dialog left open across an unlock at the computer does nothing. It
locks the computer's side first, then calls `Disconnect` on
`org.reprise.RemoteDesktop` on the user's session bus. A committed Log Off
with a client connected calls `SessionEnding` there, and a Turn Off or
Restart calls `PowerEnding`; both end every client as logged off. Any
process of the user can call `Disconnect` or `PowerEnding` and drop that
user's clients; the session keeps running, and no other user can reach the
bus. The server takes `SessionEnding` only from the compositor whose display
it serves, so another Reprise session of the same user logging off leaves
its clients connected. A process in a Flatpak sandbox is refused
these calls and the client list, and a change to the list is signalled
without the addresses.

Flatpak lets an app own the bus names under its app id, so an app with the
id `org.reprise.RemoteDesktop` can own the server's name. The server holds
the name without letting it be taken over, and a server that finds it held
exits with status 75, which neither of its units restarts: the TCP server
never serves without its name, and the hand-over server refuses that user's
remote logins until the app is gone. Settings, the session's Disconnect and
Log Off, and the standby screen talk to the name's owner only when it runs
outside a sandbox, judged by the process a pidfd from the bus names, so a
reused pid can't stand in for it. While a sandboxed app holds the name,
Remote Desktop reads as off there. Any other process of the user is trusted
with the name, as it could reach the server's files and the session
anyway.

Each client the server ends is told why before its connection closes (the
codes are in [remote-desktop.md](remote-desktop.md#disconnecting)). The reason
goes to every client, also one that didn't say it reads reasons. It carries
only the code, and only after the client logged in.

The client shares the clipboard only while it is shown the session.
`reprise-rdp` opens its data control connection once the client's clipboard
channel is up and the client has the picture and input, and closes it when
the client leaves, clearing a selection
the client still holds. A copy on either side only lists its formats to the
other, and `reprise-rdp` fetches the bytes when a program on the other side
reads them. With `[clipboard] history` on, Reprise reads every new copy for
the history, so a client's text or picture crosses as soon as it is copied.
Text and pictures cross, files and the primary selection don't.
A session copy offering `x-kde-passwordManagerHint` reaches the client
listing the registered formats `ExcludeClipboardContentFromMonitorProcessing`,
`CanIncludeInClipboardHistory` and `CanUploadToCloudClipboard`, answered
with no data, 0 and 0, which keep it out of the client's clipboard history
and cloud clipboard. The hint's value isn't read, so any value counts, and
the hint itself isn't listed. A client copy listing
`ExcludeClipboardContentFromMonitorProcessing` or
`CanIncludeInClipboardHistory` is offered in the session with the hint, read
as `secret` without asking the client, so the history skips it and its
text doesn't cross until a program pastes it.
`CanIncludeInClipboardHistory` counts whatever its value, as reading it would
ask the client for data before any paste. FreeRDP's X11 client lists only
formats it knows, so a mark set on such a client doesn't cross. Text over
16 MiB and pictures over 64 MiB are refused, measured on the wire, and a
clipboard message over 128 MiB ends the client's connection. The server's
units cap its memory at 1 GiB (`MemoryMax=`).

The clipboard follows the desktop's lock: this computer's lock in show mode,
the remote side's in lock mode. The compositor reports it as
`org.freedesktop.ScreenSaver`'s `GetActive` and `ActiveChanged` on the
session bus. `reprise-rdp` believes the name only while the bus reports its
owner to be the process serving the session's display, so the user's second
Reprise session holding the name holds the first session's clipboard until
it ends. While the name has no owner, another process owns it or the owner
doesn't answer, the clipboard is held as if locked, and the server logs why.
While locked, no format list goes either way, so the client machine's own
clipboard stays as it is. Each paste
the client asks for is answered with a failure, a read already under way
included, and a paste in the session of something copied on the client ends
empty. A copy on the client during the lock waits: on unlock the client's
newest copy reaches the session, or else the session's copy made during the
lock is offered to the client. `[clipboard] history` keeps nothing from the
lock. Other data control clients are not cut off during a lock: clipboard
managers, and programs that pass the clipboard on to another machine such as
KDE Connect's clipboard sync or Deskflow, still read and set it.

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
keyboard and pointer reach only the lock screen on those screens, besides
the keys that act at any lock screen. Its keys are read through a keyboard
state of their own, so no key or modifier held at the computer reaches a
window. The virtual screens have no cover and take only the client's input.

The virtual screens lock on their own: after `lock_after` of remote
idleness, from Start > Lock or the lock key on the remote side, and with
logind's `Lock`, which locks both sides. Their lock screen takes only
remote input, and its password unlocks only the virtual screens; the
computer's own screens stay locked. The remote side's lock goes when lock
mode ends, while the computer's stays.

The virtual screens' lock screen checks its password as a Remote Desktop
login does, under the `reprise-rdp` service, never the lock screen's
`reprise`. Wrong passwords typed on the remote side never reach pam_faillock
or the fingerprint reader, so they can't lock the account at the computer.
The screen backs off after failures like a login from one address: the first
three are free, the fourth starts a 2 s wait that doubles with each further
failure up to 60 s, and a success or 20 minutes without a failure clears
them. During the wait the screen refuses to check and says how long is left.
A right password that must be changed (`chage -d 0`) unlocks and clears the
failures like any success. A notice that the password has expired goes to the
notification history; its balloon waits, as every balloon does while the
screen is shared.

Unlocking at the computer, by password, fingerprint or logind's `Unlock`,
ends lock mode: every libei sender, the server's and any other the portal
let in, is cut off at once (its libei connection is closed and its keys
object goes inert), each object hears `ended(console_unlocked)`, the portal
closes those sessions, and the windows come back to the computer's own
screens. Until the portal destroys every object that heard `ended`, a
sender it asks for is refused, so a session that connects its input as the
console unlocks gets none. The server sees LockedHint false within 500 ms of the screens
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
  `reprise-login-rdp` and is the only process that takes connections on the
  port. It runs each connection up to its login: X.224, TLS and, under NLA,
  CredSSP.
  It holds the TLS key (`/var/lib/reprise-login-rdp/key.pem`, 0600), the NT
  hash of every Remote Desktop password, and the password of each client
  logging in. It decides nothing: it sends the name and password to the
  daemon and waits for its verdict. reprise-login starts and stops it.
- **The hand-over server**, `reprise-rdp --hand-over`, runs as the user in
  their session, as the user unit `reprise-rdp-handover.service`, started by
  `reprise-rdp-handover.socket` at `$XDG_RUNTIME_DIR/reprise-rdp/handover.sock`
  (0600, in a 0700 directory). It takes connections from reprise-login only
  and stops when its last client leaves, or once no client has been listed
  for 60 s since it started. It connects only to a display whose compositor
  runs in the user's reprise-login session, judged by the logind session of
  the process behind the socket: the one the user manager names, else any
  Wayland socket in `$XDG_RUNTIME_DIR`. It opens the portal session only once
  that session's start has restarted the portals and the portal backend's
  `WAYLAND_DISPLAY` is that display. So the remote client's picture, input,
  clipboard and lock check follow the reprise-login session, never a
  text-login Reprise of the same user.

The port is held by `reprise-login-rdp.socket`: systemd binds it in
`sockets.target`, before any user's service starts, and passes it to the
front, so no local user can take the port at boot or while the front
restarts and show clients a login of their own. This holds from the boot
after remote logins were turned on: the socket is enabled by reprise-login
when it applies the setting, so on the first boot after an upgrade that
brought the socket, the port is free until reprise-login starts. Turning
remote logins on writes the port to
`/etc/systemd/system.control/reprise-login-rdp.socket.d/port.conf`, enables
the socket and starts the front; turning them off stops both and disables
the socket. While they are off, reprise-login stops a socket started by hand
at its start and on each reload. While they are off the port is free, so a
user who binds it then keeps it, and turning them on fails until that user
lets go: reprise-login's journal names the socket, `journalctl -u
reprise-login-rdp.socket` gives the reason, and each reload while they are
on tries the port again. Until one succeeds the socket stays set to that
port, so a boot in between binds that port or none. Each start of
reprise-login and each reload while remote logins are on also clears a
failed socket or front and its start limit, and loads their unit files if
they changed on disk. A socket that is active while systemd holds nothing
on its port, in two reads of the kernel's table 100 ms apart, is restarted:
a daemon-reload that finds the socket's unit
changed while it ran leaves it so (installing a package without the socket
unit and then one with it does), and the front then fails until its start
limit. reprise-login checks for this at its start, on each of its reloads
and after each daemon-reload of systemd while remote logins are on, so the
daemon-reload a package upgrade ends with repairs a socket the upgrade left
so. A socket that holds its port is only started, so the port is never
free between boot and the front's start.

Two more sockets in `/run/reprise-login`:

- `front.sock` takes one link at a time, from the `reprise-login-rdp` uid
  only. A `Ready` frame must carry exactly one socket; a socket on any other
  frame breaks the link.
- `account.sock` is 0666. Any local user asks it about their own Remote
  Desktop password, by the uid `SO_PEERCRED` gives: one request per
  connection, read within 5 s, and one change per user at a time. A uid
  holds at most 4 connections, and 64 are open at most; only a uid that
  names an account waits for its request, among 16 at most, so a user's
  subordinate uids are answered and closed at once.

The design assumes the front can be broken into. A front run by an
attacker holds every Remote Desktop password's NT hash and sees the
password of each user who logs in meanwhile, as any TLS endpoint does, but
can't log in with a hash: the daemon wants the password whose hash matches.
The hand-over goes to the uid the daemon looked up, never one the front
names. So the front carries a full sandbox: `ProtectSystem=strict`,
`ProtectHome`, `PrivateTmp`, `PrivateDevices`, `ProtectProc=invisible`,
`NoNewPrivileges`, a `@system-service` system call filter, AF_UNIX only and
no bind (`SocketBindDeny=any`), 512 MiB and 64 tasks, no capability (systemd
passes it the bound port), an empty network namespace (`PrivateNetwork`) and
a `/run` holding only its own directory and `/run/reprise-login`. At its
start it refuses itself new IPv4 and IPv6 sockets, io_uring and sends with
`MSG_FASTOPEN` with seccomp (x86_64 and aarch64; elsewhere it logs a
warning), and every TCP connect and bind with Landlock (Linux 6.7 or later;
an older kernel logs a warning), so it can't reconnect a socket it accepted
to another host.

A front run by an attacker still reaches:

- the port and the connections it accepts on it;
- `front.sock`, and `account.sock` and `session.sock`, which any local user
  reaches;
- AF_UNIX sockets elsewhere in the file system, outside `/run`, `/tmp` and
  `/home`, that its uid may connect to.

It reaches no TCP or UDP service, on this host or another, no abstract
AF_UNIX socket, no netlink socket and no D-Bus. Without Landlock's TCP
rules, a socket it accepted can still be reconnected to a TCP service. Without
the seccomp filter, it can also be reconnected through TCP Fast Open on a
kernel whose Landlock predates the Fast Open fix (Linux 7.2 and its stable
backports).

The daemon's unit has no sandbox, for the reason under [Login
manager](#login-manager); the front starts no session, so nothing of a
user's descends from it.

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
   pam_faillock, so remote failures never lock the console. With pam_unix
   alone checking the password, a systemd-homed account can't log on over
   TLS. The worker
   answers PAM's first password prompt with it; any other prompt fails the
   login. Under NLA, the daemon compares the NT hash of the password the
   client delegated with the stored one in constant time, then runs
   `reprise-login-remote-nla`, which lets only a uid of 1000 or more through
   and runs `pam_shells`, `pam_nologin` and system-login's `account` and
   `session`. Both set `PAM_RHOST` to the client's address for `auth` and
   `account`.
5. A session: the user's own, or a new background one while fewer than
   `max_sessions` run. An account step that says the password must be
   changed (`chage -d 0`) asks for no change here: the login reaches the
   user's running session, here or in the background, and with none running
   it is refused like any account refusal, since a new session asks for the
   change at the login screen or a text console.

Every refusal reprise-login makes waits until 3 s after the login reached
the daemon, whatever refused it, so an unknown name, a name outside the
group, root and a wrong password look the same to the client. The daemon's
journal (`journalctl -b -u reprise-login`) names the reason. Under NLA the
front checks the Remote Desktop password itself: a wrong one is refused at
once with STATUS_LOGON_FAILURE and never reaches reprise-login, and an
unknown name, root, a name outside the group and one with no Remote Desktop
password get the same answer. The front's journal (`journalctl -b -u
reprise-login-rdp`) logs that refusal. Which names have a Remote Desktop
password shows anyway (see [Accepted risks](#accepted-risks)). A client has
10 s from its TCP accept to send its X.224 request and its TLS ClientHello,
then 60 s to send its credentials, time for its user to answer a
certificate or password prompt, and the login then has 8 s to reach its
session. The caps on connections not yet logged in bound what slow clients
hold.

### Throttle

Throttles count failed logins, with the numbers of the Remote Desktop
server's ([Remote Desktop](#remote-desktop)): the daemon's counts TLS login
passwords per client address, and the front's counts NLA Remote Desktop
passwords per IPv4 address or IPv6 /64, from the client's NTLM AUTHENTICATE.
The daemon counts the passwords NLA logins delegate in a throttle of their
own, which bounds a front that passes wrong ones on. No throttle feeds
another, so each kind of secret has its own budget.

- **Per address.** Three failures are free; the fourth starts a 2 s
  back-off, doubling up to 60 s. After failures the daemon counts, the front
  closes every new connection from the address (from its /64, for IPv6)
  before TLS. After NLA
  failures the front refuses at the negotiation, also before TLS, a
  connection that chose NLA: one for a user with a Remote Desktop password.
  TLS logons from the address go ahead. A success clears the address, and
  so do 20 minutes without a failure from it.
- **Every address.** 10 failures within 10 minutes pause every login of that
  kind for 10 minutes, a right password included, and the journal says so
  once. While NLA pauses, clients of users with a Remote Desktop password
  are refused at the negotiation, and TLS logins carry on: other users', and
  clients that offer TLS alone with the login password. Restarting
  reprise-login clears every throttle.

An NTLM AUTHENTICATE counts as a failed NLA password until the client
delegates the Remote Desktop password it proved. So a right proof that
delegates no password, or another user or password than the proof's, counts
as one, and so does a client that stalls inside its AUTHENTICATE.

xfreerdp opens each TLS logon twice, so one mistyped password from it
counts two failures. Before a login the front also caps connections: 32 in
all, 2 per IPv4 address or IPv6 /64, 4 per IPv6 /48, and 256 KiB read
from each.

The front logs every failed NLA password in full with the client's address,
and every verdict reprise-login sends it. Of its other refusals before a
login (the caps, a back-off, a failed handshake, no link to reprise-login)
the first each minute is logged in full and the rest are counted by kind in
one line a minute.

### Hand-over

Once the daemon accepts a login, the front puts the TCP socket into kernel
TLS and sends it to the daemon over `SCM_RIGHTS`, with the connection's TLS
keys and a recording of the exchange with the password and auto-reconnect
cookie blanked. The daemon
opens the user's `handover.sock` without following symlinks or crossing
mounts (`openat2` with `RESOLVE_NO_SYMLINKS` and `RESOLVE_NO_XDEV`): the
runtime directory and `reprise-rdp` must belong to the user and not be
group or world writable, the socket must belong to the user, and
`SO_PEERCRED` must name the user, so
only a listener the user made gets the connection. The hand-over server
takes links from root only and only a TCP socket, and replays the
recording. It takes only a recording whose login names its own user with an
empty password, and closes a fresh TCP connection. The daemon holds its copy
of the socket until the server reports taking the connection: one sent to a
server that is stopping goes to the next server the socket starts, and if no
server reports taking it within the login's 8 s, the daemon closes its copy.
A server takes a connection once only and closes a copy sent again, and
closes one whose kernel TLS has moved past the sequence numbers and nonces
the hand-over carries: a connection another server used is never served
again from its old keys. The password and the NT hash never leave the front and the daemon.

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
each user sets for themselves in User Accounts. It is kept as
its NT hash, unsalted MD4 as NTLM needs, which is fast to crack from a
copy. The file, `/var/lib/reprise-login/remote-passwords`, is 0600 root in a
0700 directory. Use a strong Remote Desktop password: it needs 8
characters or more and must differ from the login password.

- The account socket acts only on the caller's own uid, and setting or
  clearing it takes that user's login password, checked through
  `reprise-login-check` (system-auth), so an administrator can't set or
  clear another user's there. Clearing your own is always allowed, even
  outside the group: it reads no group. A set whose group look-up fails or
  times out is refused as failed, never as outside the group.
- Locking the login password (`passwd -l`) leaves the Remote Desktop
  password working. To stop it, the user removes it in User Accounts, or
  take the user out of `reprise-remote`. An expired account
  (`chage -E`) is refused either way by the account step. An expired
  password (`chage -d 0`) reaches only a session that already runs
  ([Checks](#checks)) until it is changed at the computer.
- Setting one leaves TLS logins on: a client set to TLS still logs on with
  the login password.
- An entry whose name now has another uid is dropped from the file when
  reprise-login starts and whenever it reads the group for the front's list
  while remote logins are on (below). An entry whose account is not found is
  left out of the front's list at once, and dropped only when a read at least
  10 minutes later, with no failed look-up, still finds no account: NSS
  answers "not found" while systemd-homed restarts or sssd is offline. An
  account deleted and made again with the same name and uid in between keeps
  the old entry, so remove the Remote Desktop password before deleting an
  account.
- The front gets only the hashes of users in the group, never root's.
  reprise-login reads the group for every stored user when the front links,
  each time `/etc/group` or `/etc/passwd` is written, on each reload
  (`systemctl reload reprise-login`), after a password is set in User
  Accounts, and when a login's or a session's group check disagrees with the
  front's list. A user put in or taken out of the group with `gpasswd` or
  `usermod` (members or the primary group) is on or off the list once that
  read answers, with no reload. Groups from systemd-homed records
  (`homectl update --member-of`), sssd or LDAP, and groups served from an
  nscd cache, follow at the other reads only, so a change there needs a
  reload. If reprise-login can't watch `/etc`, or the watch stops, its
  journal says so and the list follows at the other reads only.
  One read runs at a time; requests during it run one more after it. A
  reload that changes `group` takes every user off the list at once. A
  look-up that fails leaves its user off the list, and the read runs again
  after 30 s, doubling to 5 min, until it passes. The daemon's journal names
  each user put on or taken off the list.

NLA takes the name as `user`, `.\user`, `HOST\user` (the host name's first
label), `user@host` or `user@`. A client that runs bare NTLM instead of
SPNEGO may give any domain. TLS strips any `DOMAIN\` or `@host`. Only
xfreerdp has been tried.

### Background sessions

A login for a user with no session starts one through the user's login
shell, on the first free VT from 7 up that isn't on screen. It is a logind
session on seat0 with `Remote=no`, so polkit treats it as a local session
that isn't active, unless another session of the same user is active (see
[Accepted risks](#accepted-risks)). It comes up locked: switching to its VT
at the computer shows its lock screen, never its desktop, even when the
switch comes before its compositor starts (the worker marks the session's
environment with `REPRISE_LOGIN_BACKGROUND=1`, and a compositor started
with it locks whatever logind reports), and its clients always run in lock mode
([Lock mode](#lock-mode)). Off the seat a session runs no lid action and no
idle suspend, and its power requests go through logind and polkit as an
inactive session's. Once someone switches to it at the computer, it counts
as a console session. It keeps the mark, so it comes back locked after a
crash restart, and a resume while its compositor starts shows the lock;
both ask for the password once more. A console session that a remote login reaches while
the computer shows another VT, after Switch User say, counts as a
background session again. The daemon reads the VT on screen afresh for
that, and an unknown VT marks nothing.

No session holds a controlling terminal: the PAM worker holds the session's
VT as its own, so no process of the session can switch VTs through the
kernel. The polkit rule `/usr/share/polkit-1/rules.d/50-reprise-login.rules`
refuses `org.freedesktop.login1.chvt` to every caller that isn't in the
logind session on screen. polkit's own `subject.active` is true while any
session of the user is active, so the rule asks `loginctl show-session`
whether the caller's own session is. A process outside every session has no
`subject.session` and is refused before that: programs started from the
desktop (in app scopes) and user services can't switch VTs, the on-screen
session's included. So `loginctl activate` and `chvt` from a background
session, or from one left behind by Switch User, fail, also while the same
user has an ssh login. The compositor, which runs in its session, switches
VTs on Ctrl+Alt+F1..F12, and reprise-login switches as root.

While a background session runs, reprise-login holds an `idle` block
inhibitor; while a client is connected to any session, `sleep:idle`, so the
computer doesn't sleep under a client. Each session's compositor reads that
`sleep` inhibitor, held by root as "Reprise login manager", as a client
connected elsewhere: idle suspend waits, and a shut lid darkens the
internal panels instead of running `[power] lid_close`. The session still
locks unless `lid_close` is `nothing` or `screen_off`. The login screen
reads it the same way: while it is held, the screen takes logind's
`handle-lid-switch` and a shut lid darkens the panels; otherwise logind
acts on the lid.

A background session whose last client left ends `[remote_desktop]
end_disconnected_after` minutes later, unless a client comes back first;
logind ends its processes, unsaved work included. A login that reaches a
session with no client counted yet starts that time, so the session ends
even when the client that logged in never reaches it. When that time runs out
the daemon reads the VT on screen afresh, and a session the computer shows
counts as a console session and is kept; a login of its user still in its
check or on its way to it holds the end off another 8 s at a time, until
it reaches the session or is refused. With it unset, and for a console
session, the session runs until its user logs off. Disconnect in the
session ends its clients like any other leave and leaves it running,
locked. reprise-login keeps the list of background sessions in
`/run/reprise-login/background-sessions` (root, 0600), so a session off the
seat stays a background or console session across a restart of the daemon.
Without that list, as after the first restart following an upgrade, of the
sessions off the seat only those a remote login started count as background
ones, until a remote login reaches the others again. reprise-login links each background session
whatever `enabled` and the user's group say, since a client may still be
connected, and starts its time once the session's hand-over server reports
no client, or fails to report within the link's wait. It also keeps the
list of sessions whose hand-over server counted a client in
`/run/reprise-login/viewed-sessions` (root, 0600). A restart while remote
logins are off links those whatever the group says, so a client connected
to a console session keeps the computer awake; each one is linked that way
once, and leaves the list when its server first reports. With remote
logins on, a restart looks up the group for them first. A restart, or
turning remote logins on, links no other console session, so no hand-over
server starts where no client is; the user's next remote login links it.
A seat0 session logind can't describe at that restart holds back its
user's remote logins, which are refused, until logind describes it (asked
again every 5 seconds) or it ends.

### Accepted risks

- **Names with a Remote Desktop password show.** The front picks NLA or TLS
  from the user name in the client's first, unauthenticated packet (its
  `mstshash` cookie), NLA when that name is a group member's with a Remote
  Desktop password. So anyone who reaches the port can tell which names
  have one. Names leak, never passwords.
- **The chvt rule applies to the whole computer.** It sits in
  `/usr/share/polkit-1/rules.d` and applies whether or not reprise-login
  runs: under another display manager too, a session that isn't active
  can't activate itself. Display managers switch as root, so only scripts
  that activate their own inactive session lose out.
- **Hashes outlive group membership.** Taking a user out of `reprise-remote`
  stops their logins at once, as the group check comes before the hash
  compare, but their NT hash stays in the file until they clear it. The
  front keeps it until reprise-login next reads the group, right after a
  write of `/etc/group` or `/etc/passwd`, and until then a right guess of it
  is confirmed to the client.
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
- **A user's own hand-over server can hold sleep off.** reprise-login holds
  its `sleep` inhibitor while any session's hand-over server reports a
  client, and takes the count on trust: a server that kept its clients
  across a reprise-login restart reports clients the daemon never handed
  it. A user with a session can answer their own `handover.sock` with a
  count that never drops, so idle suspend waits and a shut lid darkens the
  panels instead of sleeping, in every session and at the login screen. The
  lid still locks each session whose `lid_close` isn't `nothing` or
  `screen_off`. That grants no more than the session has: polkit lets an
  active or inactive session
  take `sleep` and `handle-lid-switch` block inhibitors of its own, which
  keep the computer awake the same way. Only the name differs: the console
  sees the login manager's inhibitor instead of the user's. A count the
  server changes faster reaches the daemon at most every 250 ms, its latest
  value only, and the daemon takes one inhibitor at a time.
- **Inactive polkit grants.** A background session gets polkit's
  `allow_inactive` answers, which for some actions are wider than a remote
  session's: NetworkManager's network control (a remote user can take the
  connection down), inhibiting shutdown and sleep, the power key and lid
  inhibitors, and realtime priority. All affect availability only.
- **Active polkit grants while the user is active elsewhere.** polkit asks
  logind whether the user, not the session, is active. While the same user
  has an active session elsewhere, a text login on screen or any ssh login
  (logind counts a session without a seat as active), a background
  session gets the `allow_active` answers: mounting and powering off the
  computer's drives through udisks, NetworkManager changes, and logind's
  power actions, with no password. Only the VT switch is refused by the
  rule above; rules can't read an action's defaults, so mapping every
  other action needs a list.
- **Any program of a local administrator sets the time zone.** The time
  zone rule applies to the whole computer, so `timedatectl set-timezone`
  or any program an administrator in `wheel` or `sudo` runs in a local
  active session changes the zone without a password, whether or not the
  automatic time zone is on, and so does an administrator's background
  session while they are active elsewhere. A wrong zone shifts the times
  shown and the night light's hours; the clock itself is untouched.
- **Anyone can pause remote logins.** 10 wrong passwords every 10 minutes
  from any addresses keep one kind of login paused. For NLA that costs a TLS
  handshake and an NTLM AUTHENTICATE's first 12 bytes each, since the count
  starts before the proof is read. Meanwhile users with a Remote Desktop
  password can't log on, and TLS logins carry on.
- **Many addresses can fill the logins in progress.** Connections from 16
  IPv4 addresses or 8 IPv6 /48s that each send an X.224 request and a TLS
  ClientHello (fixed bytes, no handshake work) and then wait hold every place
  for up to 70 s at a time.
- **A late refusal tells the password was right.** A login refused after its
  password passed (the most sessions, no free VT, a failed start, the 8 s
  deadline) counts no failure, so its address doesn't back off. Under NLA
  the client learns it from the early result anyway, and a reconnect lands
  in the session the first one started.
- **The journal can fill.** The daemon logs one line per refused login it
  checked and the front one per failed NLA password, so a storm of logins
  can push older lines out of a journal capped by size. A refusal before any
  check (a backed-off address, a pause, remote logins off, too many checks)
  takes one line a minute per reason in the daemon's journal, counting those
  held back, and the front's other refusals a line or two a minute
  ([Throttle](#throttle)).
- **FUSE over the runtime directory.** A user who can mount FUSE over
  `/run/user/<uid>` with `user_allow_other` (off by default in
  `/etc/fuse.conf`) can hold their own hand-over until its link's open
  times out, and each such open leaves one of the daemon's threads stuck in
  the mount.
- **System accounts in the group.** The TLS stack refuses root only. A
  system account that an administrator adds to `reprise-remote`, with a
  shell in `/etc/shells`, can log in over TLS; the NLA stack refuses every
  uid below 1000.

## Keyboard shortcuts

A window can ask Reprise for every keyboard shortcut through
`zwp_keyboard_shortcuts_inhibit_manager_v1`, as VM viewers and remote
desktop clients do. While its surface has the keyboard, it gets the key
binds and the Super key. A balloon says so the first time a program takes
them.

These stay with Reprise under an inhibitor:

- the power, sleep and media keys, and the VT switch keys
- the binds that move no focus: lock, magnifier, colour filter, screen
  reader, keyboard layout, Do Not Disturb, night light, the screenshots, volume, brightness,
  playback, Wi-Fi and the touchpad
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

A notification's `ActivationToken`, `ActionInvoked`, `NotificationReplied`
and `NotificationClosed` signals go only to the connection that sent it, so
no other program sees the token or the text typed into its reply field.

## The portal

`reprise-portal` is the xdg-desktop-portal backend. It answers only calls
from the process that owns `org.freedesktop.portal.Desktop` on the session
bus and runs outside any sandbox, read through its pidfd by the rules in
[Sandboxed programs](#sandboxed-programs); any other caller gets
AccessDenied, so a sandboxed app that holds the name while
xdg-desktop-portal is away can't speak for other apps. App ids, parent windows and
restore data are taken on the frontend's word, and the frontend reads the
app id from the sandbox.

What a consent prompt remembers, and where:

- **ScreenCast and RemoteDesktop:** when the app asks to persist and you
  tick Remember, the portal returns a restore token, and xdg-desktop-portal
  keeps it in its permission store (tables `screencast` and
  `remote-desktop`). A token names the app it was issued to and restores
  only for that app. Without Remember nothing is kept. A restored
  RemoteDesktop grant starts without a prompt, but the app can't use it
  unseen: while an app's session holds the keyboard (a keys object for
  NotifyKeyboard) or a libei sender with a device, the screen-sharing dot
  shows, its tooltip naming the app by the app id the frontend gave
  ("An application" without one). The session the portal starts for
  Reprise's own server doesn't light it this way. A restored grant also shows a
  balloon, "<app> is controlling this computer", named and drawn from the
  app's desktop entry. It comes from Reprise at critical urgency, so the
  app's own balloon setting, Do Not Disturb, a screen share, a full-screen
  window and quiet hours don't hold it. It is up for 30 s, then stays open in
  the history until dismissed, and neither the history's caps nor a burst of
  notifications takes it out; one raised under the lock
  is still up after unlock. Each app has one: a later restored session
  replaces it, and it closes when the app's last session ends. Its Stop and
  forget, on the balloon or in the history, ends every session of the app
  and deletes the app's `remote-desktop` tokens from the
  permission store; a token the frontend holds only in memory (persist mode 1) restores nothing either
  until a prompt is allowed with Remember again. A restored ScreenCast
  shows no balloon, as the screen-sharing dot already covers it. A screen cast's app is
  named the same way, by the app the portal names on the cast's PipeWire
  node, which only the portal can set; a consumer's own name is used only
  when the node carries none.
- **Screenshot:** xdg-desktop-portal asks before an app's first
  non-interactive screenshot, through the Access prompt, and keeps the
  answer, yes or no, in its permission store (table and id `screenshot`).
  After a yes the app's screenshots are taken without a prompt; after a no
  they are refused without one. An interactive screenshot opens the share
  picker every time, which has no Remember.
- **Wallpaper:** an app with `yes` in the permission store (table and id
  `wallpaper`) sets a picture it reads itself without a prompt, and one with `no` is
  refused by xdg-desktop-portal. Without either, a call without
  `show-preview` first gets xdg-desktop-portal's Access prompt, which keeps
  its answer there itself, so Reprise doesn't ask again for a picture the app
  reads itself. Otherwise Reprise's prompt shows the picture; an app with an app id can tick
  Remember, and an Allow with it is kept as `yes`. A Deny there is never kept.
  A picture the app reads itself is one under `$XDG_RUNTIME_DIR/doc` (the
  document portal) or in its `~/.var/app/<app id>`, checked on the path the
  opened file resolves to. Any other picture, which the app may name without
  being able to read it, gets Reprise's prompt with the picture and no
  Remember, whatever the store holds.
- **InputCapture:** asks each session. An app with an app id can tick
  Remember, and an Allow with it goes to the permission store (table and id
  `input-capture`, the app id mapped to `yes` and the devices granted,
  `keyboard`, `pointer` or both); the app's later sessions asking for no more
  than those start without a prompt or a balloon. A session asking for a
  device the entry doesn't name, or an entry naming none, asks again. A
  session's receiver is offered only the devices it was granted, so a
  pointer-only session never gets keys. A Deny is never kept, and an app with
  no app id is asked every time. Settings > Privacy lists the remembered apps,
  and Forget drops one. The grant goes by app id alone, so an unsandboxed
  program that names itself after a remembered app gets it too.
- **GlobalShortcuts:** an app with `[[global_shortcuts]]` entries in
  `config.toml` is allowed; a Deny adds it to `[portal] shortcuts_denied`
  and it isn't asked again.
- **Access** (camera, background): answers go to xdg-desktop-portal's
  permission store.
- **Location:** Reprise's geoclue agent keeps its own answers.

The Email portal asks nothing: the message opens unsent in your mail
program, and you send it. Its attachments are only the files the app handed
xdg-desktop-portal as open files, which the frontend turns into paths; an
app can't name a path of its own. Each address must be a plain
`local@domain` and the subject one line, so neither reads as a header, a
`mailto:` query or a command-line option to the mail program; a call with
anything else is refused.

The Account portal shares your login, name and AccountsService picture only
after its prompt, which shows all three, and asks every time: nothing is
remembered. The reason under them is the app's own text.

The DynamicLauncher portal adds a launcher to the Start menu only after its
prompt, which shows the icon, the name and a web app's address. The address
shows only as one http or https URL with no spaces, control or bidi
characters, cut at 200 characters, so the app cannot add lines of its own.
It asks every time. xdg-desktop-portal writes the desktop file and checks it: one group,
an Exec line, and for a Flatpak an Exec that runs that same app (`flatpak
run`), so a launcher starts nothing the app couldn't start itself. An install
token, which skips the prompt, goes only to Reprise's own programs: an id
starting `reprise-` or `org.reprise.` with a desktop entry and no Flatpak app
of that id installed. A program outside a sandbox can write desktop files
itself anyway; the Flatpak check keeps out a sandboxed app named like a
Reprise program. xdg-desktop-portal itself gives one to a program outside a
sandbox that has no app id, without asking the backend. Uninstall... deletes the launcher, its link and the icon the
portal saved, and nothing else.

The share picker also keeps the last screen or window each app picked
(`$XDG_STATE_HOME/reprise-portal/lastpick.toml`), but only to preselect it:
you still confirm.

### Consent prompts

The permission prompts, the share picker, the Access prompt and the
location prompt grant something, and a prompt that takes the keyboard while
you type must not be answered by your typing. Each of them:

- takes its access keys with Alt only: a bare A, R or S presses nothing.
- ignores keys and clicks for 500 ms after it gets the keyboard: when it
  opens, each time it gets it back (the click that gives it the keyboard
  included) and when the screen wakes from the saver, blank or power-off. A
  key that went down in that time counts for nothing until it comes up, its
  repeats included.
- grants only through a click or an Alt access key. Enter presses the
  refusing button (Deny or Cancel) wherever the focus is, Space presses no
  other button, and the arrow keys don't move between buttons. Tab still
  moves the focus, to Remember or the list of sources.
- stacks above every window kept on top (X11 menus included) and every mini
  view. A sandboxed program isn't offered `reprise_on_top_v1` at all. Only a
  Wayland window from the installed portal, outside any sandbox, counts as a
  prompt: the compositor checks the process at the other end of the
  client's socket. An X11 window never counts, whatever `_NET_WM_PID` and
  `WM_CLASS` it sets.
- takes no input from an app's RemoteDesktop session: neither its libei
  sender's keys and clicks nor its NotifyKeyboard keys reach a prompt, so an
  app granted the keyboard can't widen its own grant or answer another
  app's. Reprise's own remote desktop server still answers prompts from the
  viewer. The compositor tells the two apart by the kind of sender the portal
  connects: `remote_session` for a session the portal started without a
  dialog for `reprise-rdp` (see [Sandboxed programs](#sandboxed-programs)),
  a plain sender for every other. Only the installed portal can bind
  `reprise_input_v1` to connect either. Someone driving this computer
  through Deskflow or another app answers a prompt at its own keyboard and
  mouse instead.

Xwayland's clients can't be told apart, so a sandboxed program with X11
access can still keep an override-redirect window (a menu or tooltip) above
the other windows and mini views. It stays under a consent prompt.

The polkit agent's password box and the Drives passphrase box are not
consent prompts: they stack like any window and take remote input, and the
secret they ask for keeps a stray key from answering them.

To revoke: `flatpak permissions` lists the permission store and `flatpak
permission-remove <table> <id>` removes an entry. The Keyboard page of
Reprise Settings forgets a program's global shortcuts, and Settings >
Privacy > App Permissions lists and forgets the answers in the `location`,
`background`, `screenshot`, `wallpaper` and `input-capture` tables, and the
screen sharing and remote control grants ticked Remember on. Those sit in
the `screencast` and `remote-desktop` tables, one entry per restore token,
and the list shows one row per program whatever the number of its tokens.
Forget removes the program from every token it holds in that table, and
deletes a token no other program holds, so its next session asks again. Forget doesn't end a session that is running: the
capture border shows a running share, the program ends it, and a restored
remote control session's balloon has Stop and forget. A screen share still
running when its grant is forgotten stores the grant again when it ends, so
stop the share first.

## Programs and files

- **Terminal output.** Whatever a program in the terminal prints (a `cat`
  of a downloaded file, a remote host over ssh) is untrusted. A title it
  sets shows its first 256 characters without control characters. Every
  Reprise program cuts the titles, app ids and tags it sends the compositor
  to 1024 bytes, since the compositor disconnects a client whose request
  outgrows its 4096-byte buffer, and that would close all of the program's
  windows. The terminal drops an OSC sequence longer than 64 KiB (an OSC 52
  one longer than a 1 MiB store) before it reaches the parser, and a title
  push (CSI 22 t) past 16 deep, so output can't make a tab hold memory
  without bound. An OSC 52 copy reaches the clipboard only from the open
  tab of a focused terminal window and only up to 1 MiB, so a background
  tab can't swap what you are about to paste; reading the clipboard with
  OSC 52 is refused. A title loses its bidi and zero-width characters, so it
  can't pass for another window's. A directory reported with OSC 7 picks
  where the next tab opens only while the tab's shell itself holds the
  foreground, the URL names this computer or no host, and the directory is
  the one the tab's process is in, so a host reached through ssh can't
  pick one. A dropped web link is typed
  shell-quoted like a path, and an ssh `Host` from `~/.ssh/config` is
  passed after `--`, one starting with `-` left out of the menu.
- **Colour profiles.** An ICC profile named by `[[outputs]] icc_profile`,
  or found for the Display page's list, may come from anywhere. Only its
  header, tag table, description and `vcgt` tag are read: a file over 64 MiB,
  more than 256 tags, a tag past the file's end, a `vcgt` larger than three
  channels of 65535 entries or a formula outside 0 to 1 is refused, and a
  FIFO in a profile folder fails at once. colord is asked only for its
  profiles' file names, and only while it runs.
- **Bus data.** What a program sends the compositor and `reprise-sound`
  over the session bus (tray items and their menus, LauncherEntry updates,
  media players' properties) is read only from a body of 4 MiB or less,
  and only at the types Reprise uses: a byte array or any other unused
  value is skipped unread, a dictionary keeps 64 entries, a string 4096
  bytes, a string list 16 strings, and each tray menu level, read as it
  opens, 256 rows. A notification keeps its first 64 hints and 64 action
  strings, and an `image-data` picture of 1024 px a side at most; a
  program lights at most 8 apps' task buttons through LauncherEntry.
  Notification, tray and player text loses its control, bidi and
  zero-width characters, so it can't reorder or hide what it shows. A
  player's title, artist and name show 256 characters at most. A
  notification's `image-path` picture is read only from a regular file of
  32 MiB or less, opened without blocking, and decoded on the tray icons'
  two threads below, 4 at a time per sender; one that takes over 2 s,
  counting its wait for a thread, is left out.
- **Content search.** Off by default (`[launcher] contents`): until it is
  turned on, the launcher and the Start menu's search never call
  localsearch, so the bus never starts the indexer for them. The Search
  page in Settings only asks the bus whether the name has an owner or can
  be started (`NameHasOwner`, `ListActivatableNames`), which starts
  nothing. Turned on, the first search starts localsearch, which then
  reads and indexes the files in the folders it is set to crawl and keeps
  that index under `~/.cache/tracker3` whether or not Reprise searches it
  again; turning the check off stops Reprise asking but leaves the indexer
  and its index as they are. A search provider whose program searches
  through localsearch itself, and so would start it (Files), is not asked
  while the check is off, whatever its tick under Program Results; its
  row there greys with a note. The launcher asks
  localsearch's endpoint over the
  session bus. The typed words reach it as a bound
  argument, never spliced into the query, and the answer is read off the
  compositor's loop: 1 MiB at most, within 3 s, only `file:///` URLs kept.
  A found file opens as one found by name does.
- **Search providers.** The typed words go, from two characters on, to every
  program whose `gnome-shell/search-providers/*.ini` is installed and not
  turned off in Settings > Search, at the bus name and path the file gives;
  the bus may start the program to answer. A file counts only when the
  program it names (`DesktopId`) has its desktop entry in the same data
  folder, and in a Flatpak export folder only when the file's name starts
  with that program's id, so one app's file can't speak for, or hide,
  another program's. Each answer is decoded within the
  session bus caps (4 MiB a reply; a result's strings cut at 4 KiB),
  on a thread off the compositor's loop, and dropped after 3 s. Only a
  result's id, name, description, icon name or file and clipboard text are
  read, and picking one hands its id back to the same program; Reprise runs
  nothing a result names. A result's clipboard text (Characters sends the
  character) goes on the clipboard when you pick that result, never before,
  so a provider can't fill the clipboard by answering a search. That
  program's next new window may take focus, as one Reprise started would.
- **Calendar events.** The clock's calendar reads evolution-data-server's
  source registry and opens each shown calendar read-only for one day's
  objects, off the compositor's loop, each call waiting 5 s at most. A reply
  is read only from a body of 4 MiB or less, the registry's at the UID and
  key file of each source alone. The query holds the day's times and the
  local zone's name, the name only when it is letters, digits and `/_+-`.
  A summary shows 256 characters at most, without control, bidi or
  zero-width characters.
- **Who a notification or tray icon is from.** The compositor reads which
  app a notification's sender or a tray icon's owner is from its process,
  through the pidfd the bus gives for it, by the rules the Wayland filter
  uses (see [Sandboxed programs](#sandboxed-programs)): a Flatpak app is
  the app id in its `/.flatpak-info` or its `app-flatpak-*.scope`, a
  strictly confined snap `<snap>_<app>` from its AppArmor profile or its
  scope. A sandboxed app's notification shows under that app's own desktop
  entry name and icon, else its app id, whatever `app_name`,
  `desktop-entry` and `app_icon` it sends, and Don't Show Balloons and No
  Sounds name it by that id; so it can't show a balloon as Reprise, Settings
  or a password manager. Its tray icon is kept in Customize Notifications
  under the same id and name, whatever its `Id` and `Title`, though it still
  draws its own picture and tooltip. A sender that has left by the time it
  is read, or shows too little of itself, gets no name or icon at all. A
  program outside a sandbox is trusted as you are and keeps the name it
  gives, and so does the notification portal, which forwards a sandboxed
  app's notification under the app id xdg-desktop-portal checked. Only a
  program outside a sandbox names a file for a notification to read with
  your rights: an `image-path` or `sound-file` path from any other sender is
  dropped (an icon name and a sound name stay), and the portal forwards
  only themed icon names without a `/`. A tray icon file a Flatpak app names
  (`IconName`, `AttentionIconName`, `OverlayIconName` as a path, or a name
  under `IconThemePath`) is opened inside the app's own sandbox. The bus
  peer is often the app's `xdg-dbus-proxy`, whose root shows the host's
  folders, so the root is taken from the sandbox Flatpak recorded for the
  instance the peer's `.flatpak-info` names
  (`$XDG_RUNTIME_DIR/.flatpak/<instance>/bwrapinfo.json`, which only
  programs outside the sandbox can write): its `child-pid`, only while that
  process shows the same app and instance in the mount namespace recorded,
  not the host's, and still runs once its root is open. That namespace must
  also be the peer's own, unless the peer's `.flatpak-info` says it is the
  proxy, which runs outside it. `openat2` keeps
  absolute links and `..` inside that root and refuses magic links such as
  `/proc/self/root`. A snap's opens only beneath its own folders
  (`/snap/<snap>`, `~/snap/<snap>`, `$XDG_RUNTIME_DIR/snap.<snap>`, a
  parallel instance's under either name), with no link or `..` out, since
  a snap sees the whole home. An owner that shows too little of itself, or
  whose sandbox doesn't check out, names no file; its pixmaps and theme
  names still show. Either way the file is a regular `.png` or `.svg`
  within the icon caps, its type checked before it is opened to read, an
  SVG whose entities hold markup or expand past 1 MiB is refused before it
  is parsed, an SVG from a sandbox embedding any picture is refused before a
  picture is decoded (any other SVG icon decodes only the pictures it draws,
  each once and at most 8, draws at most 4 PNG, JPEG or SVG pictures
  holding 65,536 PNG and JPEG pixels in all, at most 1024 a side, and
  leaves out GIF, WebP and any picture past these limits), an SVG is
  priced against one budget of about 78 ms of one core's CPU time for
  building its tree and drawing it together: one whose `<use>` copies and
  references would build past it is refused before usvg builds it, an
  embedded SVG picture builds within what is left or is left out, and one
  whose drawing at its size would cost past what the build left (every
  filter primitive priced by its kind and region, each layer at the size
  resvg allocates it: a rotated or skewed filtered group's layer stretched
  by the inverse of its rotation and skew, an SVG picture's layers bounded
  by five times the pixmap it lands in) or allocate more than 64 MiB of
  pixmaps in all is drawn as small as a quarter of that size and scaled
  up, or refused, and one read of
  an item's icons takes at most 16 MiB and 64 folders. Two threads shared
  by every item do the reads, from a shared queue of 64 in which one app
  holds at most 4. A read waits at most 10 seconds for a thread and then
  gets 2 seconds; past either, or past its app's share, the item keeps the
  icons it showed (unless their names changed) and is read once more 2
  seconds later. A hung read holds
  one thread and never starts another, and a removed item's read is
  dropped. An item sending more than 100 change signals in a second isn't
  listened to for 5 seconds, so a flood stops at the bus daemon instead of
  queueing there until the daemon disconnects the tray. If the tray's
  connection is lost anyway, it closes it and connects again, backing off
  up to 30 seconds while that fails, and asks for its names again. It then
  drops the inhibits, notifications and watches of every peer that left
  the bus meanwhile. Customize keeps a sandboxed app's desktop entry icon for it,
  never the path it sent, and drops a path from a choice saved without
  that guarantee.
- **Launchers.** A `.desktop` file in a folder (the desktop, Explorer)
  runs only when it is executable and carries the `user.reprise.trusted`
  extended attribute, which Allow Launching sets. Extracting an archive
  restores the execute bit but not the attribute, so a downloaded launcher
  can't pose as a program. The attribute is an HMAC under a per-user key of
  the file's filesystem id, inode and SHA-256 of its contents, so an edit
  that keeps the inode (Notepad's save, a rewrite on another machine) drops
  it, and the launcher or program asks again. Reprise's own rewrites of a
  launcher it trusts mark the new contents. The contents are hashed only
  when a marker is there, and a file over 128 MiB is never trusted, so a
  large AppImage or installer asks on every open.
- **Drive Recycle Bins.** A drive's trash is used only as `.Trash-$uid`, a
  real folder of yours, or as `$uid` in a real `.Trash` with the sticky bit,
  as the freedesktop trash spec asks. Listing it, Delete, Empty Recycle Bin,
  the `older_than_days` purge and the room a full bin makes all reach its
  `files` and `info` folders from the drive's top without following a link.
  A `files` or `info` that is a link is never entered: Empty Recycle Bin
  removes the link itself, and until then the drive counts as one without a
  Recycle Bin. A link in the bin goes as a link. The `directorysizes` cache
  is read only as a regular file of at most 1 MiB, and is written through a
  fresh file renamed into place, so a link there is replaced, never
  followed. Emptying a folder of any depth keeps a bounded number of folders
  open and stops at another file system.
- **Restore from a drive.** A drive's `.trashinfo` names a path relative to
  the drive's top. An absolute one counts only when it lies on that drive,
  and no path may climb out with `..`; the item stays in the bin and Restore
  says why. Restore walks the folders from the drive's top without following
  a link or entering another file system, so an item never lands off its
  drive, and a failed Restore removes the folders it made. The home trash
  keeps absolute paths, and its `files` and `info` may be links.
- **Extracting archives.** Each archive comes out into a fresh folder of its
  own. An entry from the root or with a `..` step refuses the archive before
  anything is written, and so does one past the folder's limits: 90% of its
  free space, and 1,000,000 entries or its free inodes, whichever is fewer.
  A symbolic link that resolves outside the extraction, or a file hard-linked
  from outside it, refuses it afterwards and the folder is removed. Extract
  Here then moves the entries into place, writing over nothing and merging
  only into real folders; if one can't move, the ones moved go back. Folders
  get their archived modes once everything is in place, so read-only ones
  extract too. An archive with a hidden top-level entry, or with a link that
  resolves to nothing in the archive, stays in its new folder, as with
  Extract to, so Extract Here can't add autostart entries or settings to a
  home folder, or a link that would name a file already there.
- **Copying from a drive.** A copy from a drive (a mount under `/run/media`
  or `/media`) onto another device carries no extended attributes: no POSIX
  ACLs, `user.*` attributes or `security.*` labels, which name another
  machine's users, and no launcher trust marker: a launcher copied off a
  drive runs only once you allow it again.
- **Programs and scripts.** A file with the owner's execute bit that starts
  with `#!` or is an ELF binary runs when you open it, from the desktop,
  Explorer or a search. The first open asks: Run / Open in Notepad / Cancel
  for a script, Run / Cancel for a binary, whatever its name. A file whose
  type's program runs it (`.exe`, `.msi`, `.bat`, `.lnk`, `.jar`,
  `.appimage`) asks Run / Cancel before that program opens it, execute bit
  or not, as does a file with no type by its name whose first bytes are
  `MZ`. The desktop, Explorer, search, the Start menu's Documents, jump
  lists and Quick Launch all ask the same way. Enter picks Run on a
  script's question and Cancel on the others. Search records a file in the
  recent files only once it opens. Run sets the trust marker above, so the
  file opens unasked until it changes; `confirm_scripts = false` turns the
  question off. A script
  runs with no window (`script_open = "run"`, the default) or in your
  terminal (`script_open = "terminal"`). Programs started from desktop
  entries in the application folders never ask, and neither does a program
  the system installed: a file whose real path is under `/usr` or `/opt`,
  where the file, its folders and the folders of the name it was opened by
  are all root's and writable by no one else.
- **Network shares.** Connect to Server hands the address to `gio mount`
  as one argument after `--`, and every answer to its prompts (user name,
  domain, password, a host-key choice) on its standard input, so a password
  never shows in a process list; a line break typed into a field is dropped
  so it can't answer the next prompt. A password typed into the address
  (`smb://user:password@server/share`) is taken out of it before the address
  reaches gio, the address bar, the Connect box or the log, and answers
  gio's first password prompt. The password field is wiped when the
  box closes and the copies Reprise holds are zeroed when dropped. Remember
  stores the password in the Secret Service's default collection under the
  network password schema (protocol, server, port, user, domain), and a
  stored password is looked up only by the server's own attributes. The
  Secret Service session is the spec's `plain` one: the password crosses
  the session bus, which only your own processes reach, unencrypted.
  Computer lists mounted shares from gvfs's mount tracker, asked only while
  gvfs runs, and never reads inside a share; opening, listing and
  unmounting run off the event loop, so a server that stops answering holds
  only that thread.
- **Disk images.** Opening an `.iso` or `.img` asks before it is mounted,
  since the kernel's file system driver then parses a file that may come
  from anywhere, and those drivers aren't hardened against hostile images.
  Explorer, the desktop, search, the Start menu's Documents and Quick Launch
  all ask, and Mount mounts it in Reprise, not in the type's program. The
  image is set up read-only on its loop device and each file system mounted
  with `ro`. Explorer's menu rows (Mount, and Open and Open With, which hand
  it to a program), Open With and a jump list's row, which opens it in that
  list's program, are explicit choices and don't ask.
- **Exec lines.** A desktop entry's `Exec` is split with no shell, and a
  field code standing as an argument of its own (`%f`, `%u`) becomes
  exactly one argument per file or URL, never shell text, so
  `sh -c "tool \"\$1\"" sh %f` is safe. A field code inside a quoted
  argument, which the Desktop Entry spec forbids, refuses the entry: Reprise
  shows that it cannot run it safely and runs nothing, since
  `sh -c "tool '%f'"` would read a file's name as shell commands.
- **Thumbnailers.** Thumbnails and Tiles hand each file Reprise doesn't
  decode itself to its installed `.thumbnailer` program as the folder
  shows, with no click. Each runs under bubblewrap in a systemd user scope:
  new user, pid, network, IPC and UTS namespaces, `/usr` and the input
  file read-only, a fresh scratch folder its one writable place, no home
  folder, no session bus or display socket, `MemoryMax=512M` and a 10
  second deadline, after which its process group is killed. Without
  bubblewrap, or where it can't start (no user namespaces, no systemd user
  manager, a file it can't bind), no thumbnailer runs and no file is marked
  failed; the built-in picture decoders (Rust, in Explorer or its
  `--thumbnailer` helper) still do. A font's sample in the
  Space preview is drawn the same way, by `reprise-explorer --font-sample`
  in the jail, so FreeType never parses a font file inside Explorer.
- **App scopes.** A program started from a desktop entry runs in a systemd
  scope `app-reprise-<app id>-<n>.scope`, where xdg-desktop-portal reads a
  host app's id. The scope is for identification and cleanup, not
  confinement. A scope is outside every logind session, so polkit judges
  its programs by the user's display session (logind's first graphical one)
  and sends their password prompts to that session's agent. A session that
  isn't its user's display session, such as a Reprise started from a text
  login while the user has a graphical session or an earlier ssh login,
  starts its programs unscoped, in its own session. Their prompts stay in
  it: each Reprise session runs its own polkit agent, which registers with
  polkit for its `XDG_SESSION_ID` and holds `reprise.polkit.session<id>` on
  the user bus. An agent that polkit would judge by another session, such as
  one started as a user unit, doesn't start, so it can't take the display
  session's prompts to another screen. When another session becomes the
  display session, programs scoped before are judged by it; its agent fails
  a prompt whose program runs in another session or in an app scope named
  with another session's `XDG_SESSION_ID`, so the prompt never shows on the
  wrong screen. The portal sees the session's programs
  with no app id, Task Manager's program list is empty, Log Off leaves
  their windowless ones running, and after a crash autostart starts its
  entries again beside copies that still run. This covers Reprise's
  launches only: a program that leaves its session on purpose
  (`systemd-run --user --scope`) is judged by the display session again
  (see Same-user code in the next section).
- **Account passwords.** User Accounts never removes a password. Add
  Account needs one, and "Ask for a new password at the next login" keeps
  the account's password and expires it (`pkexec chage -d 0`), so whoever
  logs in types that password before picking a new one. pkexec asks for an
  administrator's password for it, a prompt of its own beside
  AccountsService's for the rest of the change. An account whose
  AccountsService `PasswordMode` says it has no password (an empty hash, as
  `passwd -d` leaves), or may have none until its next login (one the old
  box emptied), is refused the box alone, so a password is typed for it. The
  change happens at the next login that starts a session, at the login
  screen or a text console. Reaching a session that already runs doesn't ask
  for it: the lock screen, lock mode's remote lock screen, logging in again
  at the login screen to a running session, a Remote Desktop login into one
  (TLS and NLA alike) and the user's own Remote Desktop server all take the
  old password. Until the change, a Remote Desktop login that would start a
  new session, a Remote Desktop password change and the login screen's
  administrator form (Shut Down and Restart while someone is logged on)
  refuse the account by the account step.
- **Fingerprints.** User Accounts adds and deletes fingers only for the
  user running it: it claims fprintd's reader with an empty user name,
  which fprintd reads as the caller's own, and asks no one about other
  accounts' fingers. fprintd's polkit action
  (`net.reactivated.fprint.device.enroll`) decides the rest; its stock
  rule asks the active local user for their own password, which Reprise's
  admin password prompt shows. While an add holds the reader, a
  fingerprint can't answer that prompt, so it takes the password.
- **Network secrets.** A Wi-Fi key, 802.1X password, VPN password or
  WireGuard key typed in Network Connections goes into its NetworkManager
  profile with flags 0, which NetworkManager stores in
  `/etc/NetworkManager/system-connections`. A saved profile whose flags leave
  a secret to a secret agent (agent-owned or not saved) keeps them: Reprise
  never writes that secret, greys its field, and refuses to connect the
  profile with a note, since it runs no agent to answer NetworkManager.
- **Automatic time zone.** With `[time_zone] automatic` on, the compositor
  sets the system time zone from what geoclue reports, through timedated
  with interactive authorization off, so polkit's rule for
  `org.freedesktop.timedate1.set-timezone` decides and nothing prompts.
  The package's rule `/usr/share/polkit-1/rules.d/50-reprise-timezone.rules`
  answers yes to that one action, without a password, for a subject that
  is local (a session on a seat), active and in group `wheel` or `sudo`.
  Such a user could set the zone with their password anyway, and the zone
  only changes how times are shown: the clock, NTP and every other action
  keep their policy, and remote and non-admin users still need an
  administrator's password. Without the rule (a build installed by hand, a
  site that deletes it) the setting stays greyed with a note. A zone only
  ever comes from tzdata's table, never from the location source.
- **Language and formats.** Settings > Region & Language writes only your
  own `~/.config/locale.conf` and AccountsService's language for your own
  account (`org.freedesktop.accounts.change-own-user-data`). Apply to the
  System calls localed's `SetLocale` with interactive authorization, so
  polkit's rule for `org.freedesktop.locale1.set-locale` (systemd's
  default: an administrator's password) decides; Reprise ships no rule.
  The compositor reads locale.conf at log on and passes on only `LANG`,
  `LANGUAGE` and `LC_*` assignments from it.
- **Bluetooth pairing.** Reprise's pairing agent answers only bluetoothd. A
  pairing request Settings didn't start (a nearby device pairing from its
  side, or a program calling `Device1.Pair`) opens a box whose Enter
  refuses, and accepting it pairs without trusting the device, so it can't
  connect a service (keyboard input included) until you connect it or tick
  "Connect to this device automatically". A second request waits for the
  open box to close, so a click meant for one device's box can't land on
  another's.
- **Thunderbolt devices.** A Thunderbolt or USB4 device can reach memory
  once authorized. `reprise-drives` reads devices only from the current
  owner of `org.freedesktop.bolt` and never starts boltd. A device that
  connects unauthorized gets one balloon per connection, and nothing is
  authorized until a button is pressed: Authorize Once calls
  `Device.Authorize`, Always Authorize enrolls it with the `auto` policy (or
  sets an enrolled device's policy to `auto`), Ignore does nothing. A
  button pressed from the history acts only while that device still waits.
  boltd checks polkit's `org.freedesktop.bolt.authorize`, `.enroll` and
  `.manage` for each call, with interactive authorization on; bolt's own
  rule lets an active local `wheel` session through without a password.
  Settings > Thunderbolt's Authorize enrolls and Forget calls
  `ForgetDevice` the same way. A device boltd already authorizes itself is
  never asked about; one enrolled under the `iommu` policy counts as that
  only while every domain reports its IOMMU on, since boltd leaves it
  waiting otherwise.
- **Receiving files over Bluetooth.** obexd lets any device in range push a
  file (BlueZ doesn't ask for Object Push), so Reprise's OBEX agent is the
  gate. It answers only the owner of `org.bluez.obex`, refuses a device BlueZ
  doesn't list as paired with the adapter it came in on, and asks before
  every file. obexd writes only inside its own folder (`~/.cache/obexd`), so
  an accepted file goes there under a hidden name and moves into Downloads
  once obexd reports it complete. The offered name loses path separators,
  control characters and leading dots, and the move never replaces
  anything: a taken name (a symlink included) is numbered. A transfer that
  ends early has its partial file removed. Every refusal, and an offer left
  unanswered for 50 seconds, replies `Rejected`: obexd takes a file into its
  own folder after any other error or after 60 seconds without an answer.
  Receiving is opt-in: "Receive files" is off until ticked on the Options
  tab, and while it is off the agent isn't registered, Reprise never starts
  obexd, and a running obexd refuses every push. Registering starts obexd,
  which also serves File Transfer to trusted devices inside that folder;
  Reprise leaves that as obexd sets it up.
- **Ending the session.** With `[session] end_processes = true` (the
  default), Log Off stops the `app-reprise-*` scopes and, after a display
  manager login, terminates the login session, so windowless programs Reprise
  or the session started go too. A program started outside the session (a
  user service, an SSH login) keeps running. A nested Reprise stops neither. While another Reprise session of the same user runs, Log Off stops
  only the scopes this session launched (each scope name carries its
  `XDG_SESSION_ID`) and leaves the user manager's variables, the session
  target and the portals to that session. A text-login Reprise that gave way
  to another Reprise session takes them back once that session's compositor
  exits and its display is gone, and takes back only the bus names nobody
  holds by then; beside a live desktop that is not Reprise it takes nothing
  ([The session](protocols.md#the-session)).
- **Installing and uninstalling.** Any program of yours can open the install
  sheet on any file (`reprise-updates --install FILE`), as a double-click
  does, so the sheet never acts on its own: it shows what the file is and goes
  ahead only on a click on Install (or Uninstall) or Alt with its access
  key, gated as the [consent prompts](#consent-prompts) are. Reprise runs
  nothing as root: PackageKit, Flatpak or `pkexec pacman -U` (for a
  `.pkg.tar.zst`, which PackageKit's pacman backend refuses) does the work
  and polkit decides whether it takes an administrator's password. The sheet
  first copies the file (and a `.sig` beside it) into a new folder in
  `$XDG_RUNTIME_DIR` only you can open, reads its facts from the copy and
  installs the copy, so a file someone swaps after it opened is not what
  installs. A downloaded package carries no repository signature, so
  PackageKit treats it as untrusted, the packager it names reads
  "(unverified)" unless pacman will check a `.sig`, and its install scripts
  run as root once it installs: install only packages you trust. A
  `.flatpakref` adds its repository, and the one its `RuntimeRepo` names, as
  system remotes that stay; the sheet names the second one. The uninstall sheet refuses a removal that would take Reprise's own
  packages or one the system marks essential; that guards against a slip,
  not against an administrator, who can still remove them with the package
  manager.

## Out of scope and known gaps

- **No per-app Xwayland.** Every X11 program shares one server; see
  [X11 programs](#x11-programs). Run a program you don't trust as a
  Wayland client or in a Flatpak without X11 access: it can't connect to
  the server, network access or not.
- **A level meter lights the microphone.** Every capture stream on a
  microphone lights it, pavucontrol's meter included. A meter marks its
  stream `stream.monitor` and `resample.peaks`, but a PipeWire program sets
  those on its own stream and applies them in its own process, and a peak
  stream at the source's rate still passes the full signal. Any recorder
  could set them, so the indicator doesn't read them.
- **Privileged globals have no allow-list.** Any unsandboxed program can
  capture the screen, read the clipboard, inject input or lock and unlock
  the screen through the globals above. Sandboxing is the only way to keep
  a program from them.
- **No clipboard portal.** A sandboxed program gets the clipboard only while
  one of its windows has the focus, or through `[clipboard] trusted_apps`.
- **X11 programs can replace the clipboard.** While the clipboard holds an
  X11 copy, any X11 program that takes the selection replaces what Wayland
  windows paste, with no focus. While a Wayland window of a Flatpak app has
  the focus, any X11 program can also replace a Wayland copy by naming that
  app's X11 window as its owner, or through a recycled pid
  ([X11 programs](#x11-programs)). It reads nothing that way.
- **X11 copies are readable by every X11 program.** A copy made through X11,
  a Flatpak app's included, can be read by any X11 program until it is
  cleared. Listing a password manager that copies through X11 in
  `[clipboard] trusted_apps` keeps its copies on the Wayland clipboard, and
  lets it read the clipboard in the background
  ([flatpak-clipboard.md](flatpak-clipboard.md#copy-buttons)).
- **Background sessions get polkit's inactive grants** under
  reprise-login, and its active ones while their user is active elsewhere;
  see [Accepted risks](#accepted-risks).
- **Idle inhibitors hold off the idle lock.** Any program, a sandboxed one
  included, holds `[idle] lock_after` off with an idle-inhibit-v1 inhibitor on
  a surface that shows (a window on the current desktop, a mapped layer
  surface, a popup on either) or through the portal's Inhibit, as on other
  desktops. An inhibitor on a surface that shows nowhere doesn't count, and
  none holds off a lock that is already up.
- **No NetworkManager secret agent.** A profile that asks for its password
  each time, or keeps it in another desktop's wallet, can't be connected
  from Reprise; `nmcli --ask connection up` can.
- **Same-user code.** A program running as you can ptrace or preload the
  portal, the compositor or anything else of yours, or take the
  `reprise.polkit.session<id>` name of a session before its agent does, so
  that session gets no password prompts, or take
  `org.freedesktop.ScreenSaver`, so its Remote Desktop clipboard stays held.
  Reprise doesn't defend against that, and doesn't treat it as a
  vulnerability.

## Reporting

Report a hole in any of these boundaries privately, as
[SECURITY.md](../SECURITY.md) describes.
