# Install

Reprise targets a current desktop Linux: systemd and logind, PipeWire with
WirePlumber, NetworkManager. There's no period-correct driver CD to lose.

## Build and run

    cargo build --release

Once it's installed, pick "Reprise" in your login manager, or run `reprise`
from a TTY.

To try it without committing your afternoon, run it nested inside another
Wayland session:

    ./target/release/reprise --startup alacritty

### Logs

A session started from a login manager logs to the user journal under
`reprise`, with each log level as its journal priority:

    journalctl --user -b -t reprise
    journalctl --user -b -t reprise -p warning

Each program Reprise starts (the tray helpers, autostart entries, launches)
logs under its desktop entry id, such as `reprise-sound`, or under its
program name when it has no entry. Each runs in an `app-reprise-*.scope` of
the user manager, so journald's rate limit for a program that floods its log
applies to `user@<uid>.service` and leaves the compositor's session lines
alone. That budget (`RateLimitBurst=` lines per `RateLimitIntervalSec=` in
journald.conf, counted per priority) is shared by every app scope and user
service: a program flooding at one priority drops the other programs' lines
at that priority until the interval ends, and the system journal says
`Suppressed N messages from user@<uid>.service`. A unit's `LogRateLimit*=`
settings don't apply to user units. Reprise's own programs mark their lines
with the same priorities, so `-t reprise-sound -p warning` lists that
program's warnings and errors. The apps and tray helpers log their own info
lines and their libraries' warnings (`warn,reprise=info`) unless `RUST_LOG`
says otherwise.
Started from a terminal, Reprise logs to the terminal and its programs
inherit it; `REPRISE_LOG_STDERR=1` keeps the log on stderr anywhere.
`RUST_LOG` sets the level, as in `RUST_LOG=reprise=debug`.

A panic logs its backtrace at error priority, so
`journalctl --user -b -t reprise -p err` shows why Reprise restarted (see
Crash recovery in [features](features.md)); `-b -1` reads the boot before.

The log names windows and programs by app id, window id and pid. It does
not record window titles, notification text, clipboard contents, what is
typed or what is playing.

On Arch, `packaging/arch/PKGBUILD` packages the compositor, the programs, the
session file, the PAM files, the portal backend and the login manager
([below](#login-manager)). The programs are one
binary, `/usr/bin/reprise-apps`, and `reprise-notepad` and the other program
names are symlinks to it. The Remote Desktop server is a binary of its own,
`/usr/bin/reprise-rdp`, run by the user unit
`app-org.reprise.RemoteDesktop.service` with the desktop entry
`org.reprise.RemoteDesktop.desktop` beside it; the unit is installed but
not enabled. A build installed by hand needs all three: the portal skips
its prompt only for that unit running `/usr/bin/reprise-rdp` or
`/usr/local/bin/reprise-rdp`, and the server checks passwords with the
`reprise` in the same directory.

The PAM files are `reprise` (the lock screen's password),
`reprise-fingerprint` (its fingerprint) and `reprise-rdp` (Remote Desktop
logins and lock mode's remote Unlock dialog). `reprise-rdp` runs `pam_unix` alone, without `system-auth`, so wrong
Remote Desktop passwords never reach pam_faillock and never lock the account
locally. The Remote Desktop server throttles them instead: a growing back-off
per address after three failures, during which its connections are refused, and a 10 minute pause of every login after 10
failures within 10 minutes (see [security](security.md#remote-desktop)). A
build installed by hand needs `data/pam.d/reprise-rdp` in `/etc/pam.d/`;
without it PAM falls back to the `other` service, which on Arch refuses
every login and elsewhere may run the full system stack.

Every push to main also publishes a package as the
[Nightly release](https://github.com/reprise-project/reprise-desktop/releases/tag/nightly).
The package is unsigned, and pacman refuses an unsigned package given as a
URL, so download it first and install the local file:

    curl -LO https://github.com/reprise-project/reprise-desktop/releases/download/nightly/reprise-nightly-x86_64.pkg.tar.zst
    sudo pacman -U ./reprise-nightly-x86_64.pkg.tar.zst

## Login manager

The package also installs Reprise's own login manager, `reprise-login`,
disabled. SDDM, or whichever display manager you have, stays in charge
until you switch:

    sudo systemctl enable reprise-login --force

then reboot. `--force` moves the `display-manager.service` link over from
the old display manager. To go back:

    sudo systemctl enable sddm --force

(or another display manager), then reboot. If remote logins are on, turn
them off first, or run `sudo systemctl disable --now
reprise-login-rdp.socket`: reprise-login enables that socket, and nothing
else disables it, so it would hold the Remote Desktop port at every boot.
The same goes before removing the package.

reprise-login shows the login screen on VT 1 in Classic, Blue or Glass
([features](features.md#login-screen)) and starts each user's Reprise
session on a VT of its own, from VT 7 up, through the user's login shell.
`/etc/reprise/login.toml` picks the interface, whether the screen lists
users, a user to log in automatically
([configuration](configuration.md#login-manager)), and whether users may log
in from another computer ([Remote logins](#remote-logins)).

Ctrl+Alt+F1 brings the login screen back whenever none is showing, for
another user to log in. Ctrl+Alt+F2 reaches a text login on tty2 at any
time, and reprise-login
switches to it when the login screen fails three times in a row. systemd
restarts reprise-login itself when it exits, but starts it at most twice in
30 s, manual restarts included. Past that it leaves it stopped and starts a
text login on tty1. Once the cause is fixed, log in on the Ctrl+Alt+F2 text
login and run `sudo systemctl reset-failed reprise-login` and `sudo
systemctl start reprise-login` to bring the login screen back. Not on tty1:
starting reprise-login stops the text login there, and the shell in it.

The logs:

    journalctl -b -u reprise-login
    journalctl -b _UID=$(id -u reprise-greeter)

The first holds reprise-login and its PAM workers, the second the login
screen itself. In a session reprise-login starts, the compositor logs
under `reprise` as it does under any login manager (`journalctl --user -b
-t reprise`); output written before its log opens goes under
`reprise-session`.

A build installed by hand needs `/usr/bin/reprise-login` (the unit's
`ExecStart`), the unit in `/usr/lib/systemd/system/`, the six
`reprise-login*` PAM files from `data/pam.d/` in `/etc/pam.d/`
(`reprise-login-check` also checks the administrator password for Shut Down
and Restart on the login screen), the polkit action and rule from
`data/polkit-1/` under `/usr/share/polkit-1/` (the rule keeps every session
that isn't on screen, Switch User's included, from switching the console),
`/usr/lib/reprise/reprise-login-settings` (the path the polkit action names;
Settings > Login Manager saves through it), and the files from
`data/sysusers.d/` and `data/tmpfiles.d/` applied once with
`systemd-sysusers` and `systemd-tmpfiles --create`. A package's install runs
those two for you. Remote logins also need `/usr/bin/reprise-login-rdp`
with `reprise-login-rdp.service` and `reprise-login-rdp.socket` beside the
other unit (leave the socket disabled: reprise-login enables and disables
it), and `reprise-rdp-handover.socket` and `.service` in
`/usr/lib/systemd/user/`, with the socket enabled for every user (`sudo
systemctl --global enable reprise-rdp-handover.socket`).

After upgrading the package, reboot, or run `sudo systemctl restart
reprise-login` and have each remote user reconnect. The login manager, the
login screen, Settings, User Accounts and each session's `reprise-rdp
--hand-over` talk over sockets whose messages can change between versions: an
older program can't talk to a newer daemon, and a hand-over server that
started before the upgrade can't report its clients, so a background
session it serves counts as having none and its disconnect time starts.

### Remote logins

reprise-login can take RDP connections itself and log users in from another
computer, into the session they left running or a new one in the
background, several users at once ([features](features.md#remote-logins)).
It ships off. To turn it on:

1. Add each user who may log in remotely to the `reprise-remote` group:
   `sudo usermod -aG reprise-remote USER`.
2. Tick Allow remote logins to this computer on Settings > Login Manager,
   or set `[remote_desktop] enabled = true` in `/etc/reprise/login.toml`
   ([configuration](configuration.md#remote-logins)) and run `sudo
   systemctl reload reprise-login`.
3. Open the port, 3389/tcp unless `port` says otherwise, in your firewall:
   `sudo ufw allow 3389/tcp`, or `sudo firewall-cmd --permanent
   --add-port=3389/tcp` and `sudo firewall-cmd --reload`. Reprise opens no
   firewall port and sets up no forwarding.

A user logged in while the package installs has no
`reprise-rdp-handover.socket` until their systemd user manager restarts, and
their remote logins are refused (`uid N's hand-over socket: No such file or
directory` in reprise-login's log). Log that user out and back in, or run
`systemctl --user start reprise-rdp-handover.socket` as that user.

Users log in with their login password over TLS, or with NLA once they set
a Remote Desktop password in User Accounts
([security](security.md#remote-desktop-password)).

To turn it off, clear the check box, or set `enabled = false` and reload.
The front and its socket stop, the port is free, and every login not yet
in its session is refused. Taking
a user out of `reprise-remote` (`sudo gpasswd -d USER reprise-remote`)
stops that user's remote logins alone.

The logs:

    journalctl -b -u reprise-login -u reprise-login-rdp -u reprise-login-rdp.socket
    journalctl --user -b -u reprise-rdp-handover

The first holds the daemon's verdict on each login, with the reason for a
refusal, and the front's connections; the second, run as the user, that
user's server.

## Requirements

### To build

- Rust, clang, pkg-config and git
- cargo-nextest 0.9.149 or later, to run the tests (`cargo test` works
  without it)
- Reprise Terminal's emulation comes from the `alacritty_terminal` crate,
  built in; it needs no runtime library
- Python 3, for the tests' string id check (`scripts/i18n-check`)
- Translated strings come from the Fluent crates (`fluent-bundle`,
  `fluent-langneg`, `unic-langid`), built in; they need no runtime library
- Text fields find grapheme clusters with the `unicode-segmentation` crate,
  built in; it needs no runtime library
- Character Map names characters and groups them by block with the
  `unicode_names2` and `unicode-blocks` crates, built in; they need no runtime
  library
- A stuck main loop's backtrace is walked and named with the `backtrace`
  crate, built in; it needs no runtime library
- The Remote Desktop server (`reprise-rdp`) is built on the IronRDP crates,
  Tokio, rustls and rcgen, built in. rustls's aws-lc-rs compiles its C library
  with the C compiler. IronRDP turns on flate2's zlib backend, which every
  program shares, so zlib is a runtime requirement
- `reprise-rdp` encodes H.264 through FFmpeg 9 (libavcodec, libavfilter,
  libavutil, libswscale) with the `ffmpeg-the-third` crate, whose bindings are
  generated from FFmpeg's headers at build time (clang)
- `reprise-rdp` draws the full-colour (AVC444) views, and NVENC's input, on
  the GPU with OpenGL ES through the `khronos-egl` and `glow` crates, built
  in; it loads libEGL at run time, from the Mesa and libglvnd listed below,
  and on NVIDIA libcuda from `nvidia-utils` to copy the drawn pictures into
  CUDA

### To run

- systemd (logind and pam_systemd, the user manager, timedated, and
  sysusers and tmpfiles for the login manager) and libseat (from seatd; it
  talks to logind)
- For the login manager: the PAM files `reprise-login` (logins typed at the
  login screen), `reprise-login-greeter` (the login screen's own session),
  `reprise-login-autologin` (`[login] autologin_user`) and
  `reprise-login-check` (the login screen's administrator password for Shut
  Down and Restart, and Remote Desktop password changes), and the system
  user `reprise-greeter` with its home `/var/lib/reprise-greeter`, which the
  package's sysusers and tmpfiles files create
- For remote logins at the login manager: the PAM files
  `reprise-login-remote` (logins with the login password) and
  `reprise-login-remote-nla` (logins with a Remote Desktop password), the
  system user
  `reprise-login-rdp` and the group `reprise-remote` (sysusers), and Linux
  6.14 or later, whose kernel TLS follows TLS 1.3 key updates: on an older
  kernel a TLS 1.3 connection ends at its first key update. The oldest
  kernel remote logins have run on is 7.2.7
- For remote logins, the kernel's `tls` module (`CONFIG_TLS`): each
  connection reaches the user's `reprise-rdp` in kernel TLS. The front's unit
  loads it; without it a login passes its check and then fails to reach its
  session, and the journal says the module isn't loaded
- Optional, for remote logins: Landlock (`CONFIG_SECURITY_LANDLOCK`, with
  `landlock` in the kernel's active LSMs), which stops the front connecting
  out over TCP. Without it the front runs and logs a warning
- libudev, libinput 1.26 or later, libdrm, Mesa (GBM and the EGL and OpenGL ES
  drivers), libglvnd and libdisplay-info
- libxkbcommon and xkeyboard-config, whose registry names the keyboard
  layouts in the tray and in Reprise Settings
- freetype2, fontconfig, PAM, libxcrypt, dbus, PipeWire 1.4 or later, zlib
  and FFmpeg 9 (with libx264, which Remote Desktop encodes on when the GPU
  cannot, and libopus, which it sends sound to the client in)
- dbus-broker, or dbus-daemon 1.15 or later, whose peer credentials carry a
  process fd: Remote Desktop and the Inhibit portal check by it which process
  owns a name. On an older bus Settings reads Remote Desktop as off, its
  clipboard stays cut, and the portal's `screensaver-active` follows logind's
  `LockedHint`, which in lock mode stays true while the remote side is unlocked
- Xwayland, for X11 programs. Reprise starts it on the first X11 connection
  and stops it when X11 programs have gone quiet.
- A UI font: Wine's Tahoma is recommended (`ttf-tahoma` on Arch, from the
  AUR); without it, Reprise falls back to DejaVu Sans. See [Fonts](#fonts).
- An icon theme. A classic, early-2000s theme will look most at home; see
  [assets.md](assets.md#icons).
- A desktop folder on a file system with `user.` extended attributes, such
  as ext4, btrfs, xfs or tmpfs. A `.desktop` file runs from the desktop only
  when it's executable and carries Reprise's `user.reprise.trusted` marker,
  which Create Shortcut and Allow Launching set.
- Linux 6.16 or later for Deskflow and remote desktop input through libei,
  whose sockets use `SO_PASSRIGHTS` to refuse file descriptors.
- Linux 6.6 or later and a GPU driver with DRM syncobj timelines, for
  explicit sync, which NVIDIA's driver relies on.

### Optional

Install what you want working. Everything below lights up a part of the
desktop and stays out of the way when it's missing.

**Sound, power and hardware**

- WirePlumber, for the volume applet, Volume Control, the volume keys and
  sound on a Remote Desktop client
- pipewire-pulse, for desktop and microphone audio in OBS and browsers
- sound-theme-freedesktop, for the notification balloons' sound
- UPower, for the battery icon, power alarms and battery idle timeouts, and
  from 1.90 the battery charge limit
- power-profiles-daemon, for the power profile in Power Options and the
  battery menu
- BlueZ, for the Bluetooth icon, Bluetooth Devices and `[lock] away_device`
- KDE Connect (`kdeconnect`), for the phone icon (battery, Ring phone, Send
  files) and phones in Send To
- UDisks2, for mounting drives as they're plugged in, Safely Remove
  Hardware, unmounted drives and Eject in Computer, and mounting disk images
- ddcutil and the i2c-dev module, for external monitors' brightness over
  DDC/CI (the keys and the displays icon). Load i2c-dev at boot with
  `echo i2c-dev | sudo tee /etc/modules-load.d/i2c-dev.conf`; ddcutil's udev
  rule gives the logged-in user the monitors' I2C buses.
- hwdata, whose `pci.ids` names the graphics card in About Reprise and Task
  Manager
- tzdata, whose `zone1970.tab` places your time zone's city, for the night
  light's sunset and sunrise without a set location

**Network**

- NetworkManager, for the network icon and Network Connections
- dnsmasq, which NetworkManager runs to share a connection from a hotspot
- NetworkManager's VPN plugins (networkmanager-openvpn and the like), for
  importing and running those VPNs. WireGuard needs none.

**Printing, updates and accounts**

- CUPS with `cups.socket` enabled, for Printers, the queues and the printing
  tray icon. An account in CUPS's admin group (`SystemGroup` in
  `/etc/cups/cups-files.conf`, `wheel` on most systems) manages printers
  without a password, since Reprise talks to CUPS over its local socket.
  Reprise follows CUPS through its D-Bus events, so cupsd may exit when idle,
  even with Printers open; it does so only with `WebInterface No` in
  `/etc/cups/cupsd.conf`.
- cups-filters, for driverless IPP Everywhere and AirPrint printers
- cups-pk-helper, so accounts outside CUPS's admin group can manage printers
  after the admin password prompt
- avahi-daemon, so the Add Printer wizard lists network printers for accounts
  outside CUPS's admin group without the admin password prompt
- PackageKit, Flatpak and fwupd, for system, Flatpak and firmware updates in
  Software Updates
- AccountsService, for User Accounts and the user pictures on the login
  screen

**Security**

- polkit, for the admin password prompt, for saving Settings > Login Manager
  (pkexec), and for the login manager's rule that keeps every session not
  on screen, Switch User's and background ones alike, from switching the
  console
- gnome-keyring, for saved passwords and the Secret portal. The login
  manager's PAM file (SDDM's or `reprise-login`) unlocks it at login, and the
  lock screen's password unlocks it when it was left locked. Under
  autologin PAM has no password, so no login keyring
  is created and the Secret portal fails until one exists: create it once by
  running `gnome-keyring-daemon --unlock`, typing the new password and
  Ctrl+D, or by one password login. Each autologin session then asks for
  that password the first time a program reads a secret.
- fprintd, for a fingerprint at the lock screen (enroll with
  `fprintd-enroll`) through the `reprise-fingerprint` PAM service, which
  keeps an account pam_faillock locked locked. The admin prompt takes one
  when `/etc/pam.d/polkit-1` has `pam_fprintd`.
- geoclue, for programs that ask for your location. Reprise is its agent
  (the package's `/etc/geoclue/conf.d/90-reprise.conf` lets it); without an
  agent geoclue refuses everyone.

- A VA-API driver with an H.264 encoder, for hardware H.264 in Remote
  Desktop: `intel-media-driver` on Intel graphics, `libva-mesa-driver` on
  AMD (Arch's `mesa` package provides it). On NVIDIA, `nvidia-utils` (CUDA
  and NVENC), tried on a GeForce RTX 4080 SUPER with driver 615.78.
  Without a hardware encoder, Remote Desktop encodes H.264 on the CPU with
  x264 through FFmpeg's libx264, which Arch's `ffmpeg` links; a client
  without H.264 gets RemoteFX bitmaps.

**Portals and toolkits**

- xdg-desktop-portal 1.19 or newer, which routes screen sharing,
  screenshots, remote desktop, Deskflow, global shortcuts, file dialogs,
  permission prompts and the light or dark look to Reprise's portal backend,
  and lets files dragged between Explorer and Flatpak programs open on the
  other side. Older releases ask the backend to write the autostart entries
  Flatpak programs request, which Reprise leaves to the portal
- xdg-desktop-portal-wlr, the screen sharing and screenshot fallback
- xdg-desktop-portal-gtk, for printing from GTK3 programs
- gnome-themes-extra, for GTK3 programs' dark theme under a dark look
- qt6ct or qt5ct, so Qt programs Reprise starts take the look's colours
  (`[toolkits] qt_platform_theme = "reprise"`)

**Input, text and files**

- fcitx5 or ibus with its Wayland frontend, for input methods
- libx11's Compose files, for dead keys and the Compose key in the lock
  screen's password field
- at-spi2-core, for screen readers. Orca is the one Reprise is tested with.
- A CJK font such as Noto Sans CJK, for Chinese, Japanese and Korean text,
  and a bitmap one such as WenQuanYi Bitmap Song for crisp Chinese with
  `antialias = false`
- A colour emoji font such as Noto Color Emoji, for the emoji picker
- shared-mime-info, for file types and icons
- xdg-utils and libarchive, for opening files, zipping and extracting
  archives in Explorer and opening a network's sign-in page
- plocate with its database, or fd, for files in the launcher and Explorer's Search
- Freedesktop thumbnailers, for thumbnails of files Reprise does not decode
  itself (`[thumbnail_cache] external_thumbnailers`): papers or evince for
  PDF, PostScript, DjVu and comic books, ffmpegthumbnailer for videos,
  gnome-font-viewer for fonts, libgsf for OpenDocument and Office files.
  Any package that installs a `.thumbnailer` file works.

## Fonts

Reprise ships no fonts. The UI is set in Tahoma; Wine's free version
(LGPL) has the same metrics and is an optional dependency of the package. DejaVu Sans covers everything
else. The other period faces (Trebuchet MS, Franklin Gothic Medium, Segoe
UI and a wider dialog font) are picked up if you have a licensed copy. [assets.md](assets.md#fonts)
has the table.
