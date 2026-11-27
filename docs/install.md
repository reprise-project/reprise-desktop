# Install

Reprise targets current desktop Linux: systemd and logind, PipeWire with WirePlumber, and NetworkManager.

## Build and run

Build every component (compositor, programs, portal, and Remote Desktop server):

    cargo build --release --workspace

To try a build, log in on a free TTY (Ctrl+Alt+F3) and run:

    PATH="$PWD/target/release:$PATH" ./target/release/reprise --startup alacritty

Reprise opens display and input through logind, so it does not run nested inside another session. Keep `target/release` on `PATH` because Reprise starts its programs by name. Reprise's programs appear in the Start menu once you [install](#install-by-hand) their desktop entries under `applications` in `XDG_DATA_DIRS`.

### Read the logs

Under a login manager, Reprise logs to the user journal under `reprise` with matching journal priorities:

    journalctl --user -b -t reprise
    journalctl --user -b -t reprise -p warning

Launched programs log under their desktop entry id (such as `reprise-sound`) or program name. Reprise's programs use the same priorities and default to `warn,reprise=info`.

From a terminal, Reprise and its programs log to stderr. Set `REPRISE_LOG_STDERR=1` to log to stderr anywhere, and set `RUST_LOG` (such as `RUST_LOG=reprise=debug`) to change the level.

Panics log backtraces at error priority. Run `journalctl --user -b -t reprise -p err` to inspect a crash, or add `-b -1` for the previous boot. See [features](features.md).

Logs record app id, window id, and pid. They omit window titles, notification text, clipboard contents, typed text, and what is playing.

## Install a package

Once installed, pick "Reprise" in your login manager or run `reprise` from a TTY.

On Arch, `packaging/arch/PKGBUILD` packages the compositor, programs, session file, PAM files, portal backend, and [login manager](#use-the-login-manager). Every push to `main` publishes an unsigned Arch package to the [Nightly release](https://github.com/reprise-project/reprise-desktop/releases/tag/nightly). Download it first because `pacman` rejects unsigned URLs:

    curl -LO https://github.com/reprise-project/reprise-desktop/releases/download/nightly/reprise-nightly-x86_64.pkg.tar.zst
    sudo pacman -U ./reprise-nightly-x86_64.pkg.tar.zst

Programs share `/usr/bin/reprise-apps` via symlinks such as `reprise-notepad`. Remote Desktop runs `/usr/bin/reprise-rdp` via `app-org.reprise.RemoteDesktop.service` (disabled by default) and `org.reprise.RemoteDesktop.desktop`.

The package installs three session PAM files: `reprise` (lock password), `reprise-fingerprint` (fingerprint), and `reprise-rdp` (Remote Desktop). `reprise-rdp` uses `pam_unix` alone so failed remote logins never trigger `pam_faillock`. See [security](security.md#remote-desktop).

### Install on Ubuntu 26.04

Download and install the unsigned Nightly `.deb`:

    curl -LO https://github.com/reprise-project/reprise-desktop/releases/download/nightly/reprise-nightly-ubuntu26.04-amd64.deb
    sudo apt install ./reprise-nightly-ubuntu26.04-amd64.deb

To build the `.deb` from a checkout on Ubuntu 26.04:

    xargs -a packaging/deb/build-depends sudo apt-get install -y --no-install-recommends
    rustup default stable
    packaging/deb/build.sh

Use `rustup` because Ubuntu's `rustc` is older than the workspace's `rust-version`. The built package lands in `~/.cache/reprise-deb/`.

Ubuntu lacks `system-auth` and `system-login`, so the `.deb` adds `reprise-system-login` and `reprise-password-auth` (`pam_unix` and `pam_cap`). This stops `common-auth` from waiting for a fingerprint before asking for a password. Add extra auth modules such as `sss` or `krb5` to `reprise-password-auth`.

| | Reprise needs | Ubuntu 26.04 | Ubuntu 24.04 |
|---|---|---|---|
| libinput | 1.26 (tablet pad dials) | 1.31 | 1.25 |
| PipeWire | 1.4 | 1.6 | 1.0 |
| FFmpeg | 8 (Remote Desktop colour conversion) | 8.0 | 6.1 |
| libjxl | 0.11.1 (JPEG XL screenshots) | 0.11.1 | 0.7 |
| dbus-daemon | 1.15 (peer credentials with process fd) | 1.16 | 1.14 |
| Linux | 6.14 (remote logins), 6.16 (libei input) | 7.0 | 6.8 (7.0 with HWE) |
| Rust | 1.99 | 1.93 (use rustup) | 1.75 (use rustup) |

Ubuntu 24.04 has no package because those five libraries are too old. The 26.04 `.deb` also requires glibc 2.43 and libavcodec 62 (24.04 ships 2.39 and 60).

Debian 13 has no package yet: FFmpeg 7.1 blocks Remote Desktop, and Linux 6.12 blocks remote logins and libei input.

### Install by hand

On other distributions, build the workspace and copy the files from `package()` in `packaging/arch/local/PKGBUILD` out of `target/release/` and `data/` to the same paths. Keep the `/usr` prefix because the units, polkit action, portal, and `reprise-rdp` expect those paths.

Then run the post-install commands:

    sudo systemd-sysusers reprise-login.conf
    sudo systemd-tmpfiles --create reprise-login.conf
    sudo update-mime-database /usr/share/mime
    sudo systemctl daemon-reload

Install every PAM file, or PAM falls back silently to its `other` service. Leave `reprise-login-rdp.socket` disabled because `reprise-login` manages it.

## Use the login manager

The package installs `reprise-login` disabled. Switch to it and reboot:

    sudo systemctl enable reprise-login --force

To switch back (for example, to SDDM), run this and reboot:

    sudo systemctl enable sddm --force

Before switching away or removing the package, turn off remote logins or run `sudo systemctl disable --now reprise-login-rdp.socket` to free the Remote Desktop port.

`reprise-login` shows the login screen on VT 1 and starts sessions from VT 7 upward through the user's login shell. Configure it in `/etc/reprise/login.toml`. See [features](features.md#login-screen) and [configuration](configuration.md#login-manager). Settings > Login Manager > Use My Display Settings copies your display layout to `login.toml` and any chosen background picture to `/var/lib/reprise-login/background`.

Press Ctrl+Alt+F1 for the login screen, or Ctrl+Alt+F2 for a text login on tty2. After three login screen failures in a row, `reprise-login` switches to tty2. If systemd stops `reprise-login` after repeated restarts, log in on tty2 (not tty1) and run:

    sudo systemctl reset-failed reprise-login
    sudo systemctl start reprise-login

Read the daemon and login screen logs with:

    journalctl -b -u reprise-login
    journalctl -b _UID=$(id -u reprise-greeter)

Early startup output goes under `reprise-session`.

Update `reprise`, `reprise-portal`, and `reprise-apps` together. After upgrading, reboot, or run `sudo systemctl restart reprise-login` and reconnect remote users. Log out of each session once and reopen Settings.

### Turn on remote logins

`reprise-login` accepts multi-user RDP logins into existing or new sessions (off by default). See [remote desktop](remote-desktop.md#remote-logins).

1. Add each allowed user to `reprise-remote`: `sudo usermod -aG reprise-remote USER`.
2. Tick Allow remote logins to this computer in Settings > Login Manager. Or set `[remote_desktop] enabled = true` in `/etc/reprise/login.toml` and run `sudo systemctl reload reprise-login`. See [configuration](configuration.md#remote-logins).
3. Open port 3389/tcp (or your configured `port`) in your firewall: `sudo ufw allow 3389/tcp`, or `sudo firewall-cmd --permanent --add-port=3389/tcp` and `sudo firewall-cmd --reload`.

If a user was logged in during installation, log them out and back in or run `systemctl --user start reprise-rdp-handover.socket` as that user.

Users sign in with their login password over TLS, or NLA after setting a Remote Desktop password in User Accounts. See [security](security.md#remote-desktop-password).

To turn remote logins off, clear the check box or set `enabled = false` and reload. Remove one user with `sudo gpasswd -d USER reprise-remote`.

Read remote login logs with:

    journalctl -b -u reprise-login -u reprise-login-rdp -u reprise-login-rdp.socket
    journalctl --user -b -u reprise-rdp-handover

## Requirements

### To build

- Rust 1.99, clang (with a C compiler), pkg-config, git, and nasm 2.14 or later.
- FFmpeg 8 or later headers (libavcodec, libavfilter, libavutil, libswscale, libavformat), libjxl 0.11.1 or later headers (libjxl, libjxl_threads), and zlib.
- Python 3, for `scripts/i18n-check` and `packaging/third-party-notices`.
- For tests: cargo-nextest 0.9.149 or later (`cargo test` works without it), `bsdtar` (libarchive), `dbus-daemon`, shared-mime-info, DejaVu fonts, and curl for `scripts/test-fonts.sh`. `.github/ci-image/packages` lists the Arch set.

### To run

- **systemd and libseat:** systemd (logind, `pam_systemd`, user manager, timedated, localed, sysusers, tmpfiles) and libseat (from seatd).
- **Login manager:** PAM files `reprise-login`, `reprise-login-greeter`, `reprise-login-autologin`, and `reprise-login-check`, plus user `reprise-greeter` (`/var/lib/reprise-greeter`).
- **Remote logins:** PAM files `reprise-login-remote` and `reprise-login-remote-nla`, user `reprise-login-rdp`, group `reprise-remote`, Linux 6.14 or later, and the `tls` module (`CONFIG_TLS`). Landlock (`CONFIG_SECURITY_LANDLOCK`) is optional and blocks outbound TCP from the front.
- **Graphics and input:** libudev, libinput 1.26 or later, libdrm, Mesa (GBM, EGL, OpenGL ES), libglvnd, libdisplay-info, libxkbcommon, and xkeyboard-config.
- **Core libraries:** freetype2, fontconfig, PAM, libxcrypt, dbus, PipeWire 1.4 or later, zlib, and FFmpeg 8 or later (with libx264 and libopus). libx265 and libsvtav1 add CPU H.265 and AV1 recording.
- **libjxl:** Needed for JPEG XL screenshots via `/usr/lib/reprise/libreprise_encode_jxl.so`. Without it, they save as PNG.
- **D-Bus:** dbus-broker, or dbus-daemon 1.15 or later, for peer credentials with a process fd (Remote Desktop and Inhibit portal).
- **Xwayland:** Needed for X11 programs.
- **Fonts and icons:** A UI font (`ttf-tahoma` from Arch AUR recommended, fallback DejaVu Sans) and an icon theme. See [assets](assets.md#fonts) and [assets](assets.md#icons). Ubuntu's `.deb` recommends `adwaita-icon-theme-legacy`.
- **Desktop file system:** A file system with `user.` extended attributes (ext4, btrfs, xfs, or tmpfs) for desktop `.desktop` files marked `user.reprise.trusted`.
- **Kernel features:** Linux 6.6 or later with DRM syncobj timelines for explicit sync (NVIDIA). Linux 6.16 or later for Deskflow and libei input (`SO_PASSRIGHTS`).

### Optional

Missing packages disable only their feature.

**Sound, power and hardware**

- **WirePlumber:** Volume applet, Volume Control, volume keys, and Remote Desktop sound.
- **pipewire-pulse:** Desktop and microphone audio in OBS and browsers.
- **sound-theme-freedesktop:** Notification balloon and desktop event sounds.
- **UPower:** Battery icon, power alarms, battery idle timeouts, and charge limit (UPower 1.90 or later).
- **power-profiles-daemon:** Power profiles in Power Options and battery menu, plus low-battery power saver.
- **BlueZ:** Bluetooth icon, Bluetooth Devices, and `[lock] away_device`.
- **BlueZ OBEX:** `bluez-obex` on Arch or `bluez-obexd` on Ubuntu, for receiving Bluetooth files.
- **KDE Connect (`kdeconnect`):** Phone icon (battery, Ring phone, Send files) and phones in Send To.
- **UDisks2:** Auto-mounting drives, Safely Remove Hardware, Eject in Computer, and disk image mounting.
- **gvfs MTP and PTP:** `gvfs-mtp` and `gvfs-gphoto2` on Arch (`gvfs-backends` on Ubuntu) for USB devices in Computer > Portable Devices.
- **bolt:** Thunderbolt and USB4 authorization prompts and Settings > Thunderbolt.
- **iio-sensor-proxy:** Screen auto-rotation (`[display] auto_rotate`).
- **colord:** Extra ICC profiles in Display > Color profile while colord runs.
- **ddcutil and i2c-dev:** External monitor brightness over DDC/CI. Load `i2c-dev` at boot with `echo i2c-dev | sudo tee /etc/modules-load.d/i2c-dev.conf`.
- **hwdata:** Graphics card names (`pci.ids`) in About Reprise and Task Manager.
- **tzdata:** City coordinates (`zone1970.tab`) for night light times and `zone.tab` for automatic time zone.
- **Hardware video encoders:** VA-API (`intel-media-driver` on Intel, `libva-mesa-driver` or `mesa` on AMD) or `nvidia-utils` (CUDA and NVENC) for H.264, H.265, and AV1 encoding. Without one, FFmpeg encodes on CPU (`libx264`, `libx265`, `libsvtav1`), or Remote Desktop uses RemoteFX bitmaps.

**Network**

- NetworkManager, for the network icon and Network Connections.
- gvfs (with `gio` from GLib and `gvfsd-fuse`), for Connect to Server, Network Locations, and the Network place, plus `gvfs-smb` for SMB shares.
- dnsmasq, for NetworkManager hotspot sharing.
- curl, for naming a new Internet shortcut after its page's title (it reads the page's first 8 KB, waiting at most 3 seconds) and for firmware downloads with fwupd.
- NetworkManager VPN plugins (such as `networkmanager-openvpn`), for importing and running VPNs. WireGuard needs no plugin.

**Printing, updates and accounts**

- **CUPS:** Printers, queues, and tray icon (enable `cups.socket`). Members of `SystemGroup` (`/etc/cups/cups-files.conf`, usually `wheel`) manage printers without a password.
- **cups-filters:** Driverless IPP Everywhere and AirPrint printers.
- **cups-pk-helper:** Admin-prompted printer management outside CUPS's admin group.
- **avahi-daemon:** Passwordless printer discovery in Add Printer, plus SMB and SFTP discovery in the Network place.
- **PackageKit, Flatpak, and fwupd:** System, Flatpak, and firmware updates in Software Updates. PackageKit handles downloaded packages, offline updates, and Start menu uninstalls (Arch uses `pkexec` and `pacman`). Flatpak installs `.flatpakref` and `.flatpak` files, and Disk Usage's Clean Up removes unused runtimes through it.
- **systemd-coredump** (part of systemd on Arch, its own package on Ubuntu; systemd 251 or newer): the size of your crash dumps in Disk Usage's Clean Up.
- **AccountsService:** User Accounts, login screen pictures, Account portal pictures, and language sharing with other login managers.

**Security**

- **polkit:** Admin prompts, Settings > Login Manager (`pkexec`), Region & Language (`localed`), password expiry (`pkexec chage`), and background-session rules. `50-reprise-timezone.rules` lets local admins (`wheel`, `sudo`, or `admin`) set the time zone without a password.
- **gnome-keyring:** Saved passwords, Secret portal, and share passwords. Password logins and the lock screen unlock it. Under autologin, create the keyring once with `gnome-keyring-daemon --unlock` (type password, press Ctrl+D) or one password login.
- **kwallet-pam:** KWallet unlock at login via `pam_kwallet5` and `pam_kwallet_init`. Name the wallet `kdewallet`, use the login password, and use Blowfish encryption.
- **fprintd:** Lock screen fingerprints via `reprise-fingerprint` (User Accounts > Fingerprints). Add `pam_fprintd` to `/etc/pam.d/polkit-1` for admin prompts.
- **geoclue:** Location access and automatic time zone. `/etc/geoclue/conf.d/90-reprise.conf` registers Reprise as its agent.
- **dconf (`dconf-cli` on Ubuntu):** GNOME settings in Import from Another Desktop.

**Portals and toolkits**

- xdg-desktop-portal 1.19 or newer, for screen sharing, screenshots, remote desktop, Deskflow, global shortcuts, file dialogs, permissions, light or dark preference, and Flatpak drag-and-drop.
- xdg-desktop-portal-wlr, for screen sharing and screenshot fallback.
- xdg-desktop-portal-gtk, for GTK3 printing.
- gnome-themes-extra, for the GTK3 dark theme.
- qt6ct or qt5ct, for Qt colours (`[toolkits] qt_platform_theme = "reprise"`).

**Input, text and files**

- **fcitx5 or ibus:** Input methods (with their Wayland frontend).
- **libx11 Compose files:** Dead keys and the Compose key in the lock screen password field.
- **at-spi2-core and Orca:** Screen reader support. Super+Alt+S and Settings start and stop Orca (including `/usr/bin/orca` as `reprise-greeter` on the login screen).
- **CJK fonts:** Noto Sans CJK (or similar) for CJK text. Add WenQuanYi Bitmap Song for crisp Chinese with `antialias = false`.
- **Colour emoji font:** Noto Color Emoji (or similar) for colour emoji in text and the emoji picker.
- **shared-mime-info:** File types and icons.
- **xdg-utils and libarchive:** Web links, Explorer archives, network sign-in pages, and Send To > Mail Recipient (`xdg-email`). Also handles Send by Email attachments outside Thunderbird and Evolution.
- **plocate or fd:** File search in the launcher and Explorer. With both, `fd` adds files changed since the last `updatedb`.
- **localsearch:** Full-text search in the launcher and Start menu (enable in Settings > Search).
- **evolution-data-server:** Calendar events under the clock from GNOME Calendar, Evolution, or GNOME Online Accounts.
- **Freedesktop thumbnailers:** Extra thumbnails (`[thumbnail_cache] external_thumbnailers`): `papers` or `evince` (PDF, PostScript, DjVu, comics), `ffmpegthumbnailer` (videos), `gnome-font-viewer` (fonts), `libgsf` (OpenDocument and Office). Requires `bubblewrap`.
- **bubblewrap:** Sandboxes freedesktop thumbnailers, JPEG XL thumbnails, and Explorer Space previews for fonts and JPEG XL.
