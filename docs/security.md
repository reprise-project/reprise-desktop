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
  screenshot draws none.
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

One more global, `reprise_input_v1`, carries remote control, input capture
and global shortcuts between the compositor and the portal. Reprise offers
it only to a client whose process runs `reprise-portal` from `/usr/bin` or
`/usr/local/bin`. That keeps other clients from binding it directly; it is
no boundary against code running as you.

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
the verdict on stdout; only a success unlocks. A helper that hangs is given
up after 90 s and the dialog asks again. Reprise zeroes the password copies
it holds.

When `/etc/pam.d/reprise-fingerprint` is installed, a second helper runs
the fingerprint conversation while the dialog shows. Either one succeeding
unlocks.

`[power] locker` runs an external locker (swaylock, hyprlock, gtklock)
instead of the dialog. The lock stays Reprise's: the screens are covered and
focus is refused as with the dialog, and the locker draws on top and does
its own PAM check. The fingerprint helper doesn't run under a locker, which
may claim the reader itself. If the locker crashes the session stays locked
and the dialog comes back. A locker may take over a lock that has none, the
dialog's or a crashed locker's, and unlock it.

When the lock can't check a password at all (no login name, a broken PAM
stack), switch to another VT, log in, and run `loginctl unlock-session`.
Any program that can bind the session lock global can already run that
command, which is why that global needs no extra gate.

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
  program can mint only from an input event in its own focused window
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
Advanced lists and forgets location answers. There is no page for screen
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
- **Same-user code.** A program running as you can ptrace or preload the
  portal, the compositor or anything else of yours. Reprise doesn't defend
  against that, and doesn't treat it as a vulnerability.

## Reporting

Report a hole in any of these boundaries privately, as
[SECURITY.md](../SECURITY.md) describes.
