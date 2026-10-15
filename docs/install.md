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

On Arch, `packaging/arch/PKGBUILD` packages the compositor, the programs, the
session file, the PAM files and the portal backend.

Every push to main also publishes a package as the
[Nightly release](https://github.com/reprise-project/reprise-desktop/releases/tag/nightly):

    sudo pacman -U https://github.com/reprise-project/reprise-desktop/releases/download/nightly/reprise-nightly-x86_64.pkg.tar.zst

## Requirements

### To build

- Rust, clang, pkg-config and git

### To run

- systemd (logind, the user manager and timedated) and libseat (from seatd;
  it talks to logind)
- libudev, libinput 1.26 or later, libdrm, Mesa (GBM and the EGL and OpenGL ES
  drivers), libglvnd and libdisplay-info
- libxkbcommon and xkeyboard-config, whose registry names the keyboard
  layouts in the tray and in Reprise Settings
- freetype2, fontconfig, PAM, libxcrypt, dbus and PipeWire
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

- WirePlumber, for the volume applet, Volume Control and the volume keys
- pipewire-pulse, for desktop and microphone audio in OBS and browsers
- UPower, for the battery icon, power alarms and battery idle timeouts
- power-profiles-daemon, for the power profile in Power Options and the
  battery menu
- BlueZ, for the Bluetooth icon and Bluetooth Devices
- UDisks2, for mounting drives as they're plugged in, Safely Remove
  Hardware, and unmounted drives and Eject in Computer
- ddcutil and the i2c-dev module, for the brightness keys on an external
  monitor
- hwdata, whose `pci.ids` names the graphics card in About Reprise
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
- AccountsService, for User Accounts

**Security**

- polkit, for the admin password prompt
- gnome-keyring, for saved passwords and the Secret portal. SDDM's PAM file
  unlocks it at login, and the lock screen's password unlocks it when it was
  left locked.
- fprintd, for a fingerprint at the lock screen (enroll with
  `fprintd-enroll`) through the `reprise-fingerprint` PAM service, which
  keeps an account pam_faillock locked locked. The admin prompt takes one
  when `/etc/pam.d/polkit-1` has `pam_fprintd`.
- geoclue, for programs that ask for your location. Reprise is its agent
  (the package's `/etc/geoclue/conf.d/90-reprise.conf` lets it); without an
  agent geoclue refuses everyone.

**Portals and toolkits**

- xdg-desktop-portal, which routes screen sharing, screenshots, remote
  desktop, Deskflow, global shortcuts, file dialogs, permission prompts and
  the light or dark look to Reprise's portal backend, and lets files dragged
  between Explorer and Flatpak programs open on the other side
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
- xdg-utils and libarchive, for opening files, zipping from Explorer and
  opening a network's sign-in page
- plocate with its database, or fd, for files in the launcher

## Fonts

Reprise ships no fonts. The UI is set in Tahoma; Wine's free version
(LGPL) has the same metrics and is an optional dependency of the package. DejaVu Sans covers everything
else. The other period faces (Trebuchet MS, Franklin Gothic Medium, Segoe
UI and a wider dialog font) are picked up if you have a licensed copy. [assets.md](assets.md#fonts)
has the table.
