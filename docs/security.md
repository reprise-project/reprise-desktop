# Security model

Reprise's trust boundaries define what programs in your session can and cannot do. Report boundary breaks privately ([Reporting](#reporting)).

## The Wayland socket

The trust boundary is the Wayland socket in `$XDG_RUNTIME_DIR`. Unsandboxed clients run as your user and already have access to your files and keyring. Reprise grants them all Wayland globals without an allow-list ([protocols.md](protocols.md)):

- **Input method** (`zwp_input_method_manager_v2`): reads and injects text in focused fields.
- **Virtual input** (`zwp_virtual_keyboard_manager_v1`, `zwlr_virtual_pointer_manager_v1`): injects keys (including binds) and pointer events. Keys are dropped at the lock screen and during input capture.
- **Foreign toplevel and workspaces** (`zwlr_foreign_toplevel_manager_v1`, `ext_foreign_toplevel_list_v1`, `ext_workspace_manager_v1`): lists and controls windows and switches desktops.
- **Layer shell** (`zwlr_layer_shell_v1`): places surfaces above or below windows and can grab keyboard focus.
- **Data control** (`ext_data_control_manager_v1`, `zwlr_data_control_manager_v1`): reads and replaces the clipboard and primary selection at any time.
- **Screen capture** (`zwlr_screencopy_manager_v1`, `ext_image_copy_capture_manager_v1`): captures screens or windows without prompting. Active captures show the red border (`[privacy] capture_border`), and repeated stills light the screen-sharing dot. The Remote Desktop server hides the border unless `[remote_desktop] capture_border` is set.
- **Session lock** (`ext_session_lock_manager_v1`): locks the screen ([The lock screen](#the-lock-screen)).
- **Outputs and gamma** (`zwlr_output_manager_v1`, `zwlr_output_power_manager_v1`, `zwlr_gamma_control_manager_v1`, `reprise_output_trial_manager_v1`): changes display layout, power and gamma. Trials revert after at most 60 s and block saving display changes while active.
- **On top** (`reprise_on_top_v1`): keeps a window above others, below [consent prompts](#consent-prompts).
- **Reopen** (`reprise_reopen_v1`): registers up to 32 session-restore launch arguments per program in `$XDG_STATE_HOME/reprise/state.toml`.

### Sandboxed programs

Reprise classifies a client as sandboxed via pidfd if it connects through `wp_security_context_manager_v1`, sees `/.flatpak-info`, runs in an `app-flatpak-*.scope`, or runs in a strict or devmode snap (`snap.*` AppArmor profile or scope). Classic snaps count as host programs. Clients that exit before inspection count as sandboxed.

Sandboxed clients cannot see the privileged globals above or `wp_security_context_manager_v1` (see `PRIVILEGED_GLOBALS` in [`src/sandbox.rs`](../src/sandbox.rs)). They can use `xdg_toplevel_drag_manager_v1` for their own windows. Screen sharing, screenshots, remote control and input capture go through the portal with consent prompts.

`org.reprise.Tray`, Reprise apps' `org.freedesktop.Application`, Explorer's `org.freedesktop.FileManager1` and `org.reprise.RemoteDesktop` reject sandboxed callers with `AccessDenied`. Launching a Reprise app while a sandboxed app owns its D-Bus name ignores the name holder and starts standalone.

`[clipboard] trusted_apps` grants only the two data control globals to listed Flatpak app IDs ([flatpak-clipboard.md](flatpak-clipboard.md)).

Sandboxed file drags and clipboard file lists work only through the document portal (`$XDG_RUNTIME_DIR/doc`), never as cuts. Reprise marks them `application/x-reprise-sandboxed` so Reprise apps copy instead of move.

`reprise_input_v1` is exposed only to `reprise-portal` running from `/usr/bin` or `/usr/local/bin`. The portal skips consent prompts only for the enabled `app-org.reprise.RemoteDesktop.service` (or `reprise-rdp-handover.service`) running `reprise-rdp` from `/usr/bin` or `/usr/local/bin`, and for one user-initiated `reprise-record` start within 10 s. Reprise's portal clients accept replies only from the owner of `org.freedesktop.portal.Desktop`.

## X11 programs

All X11 programs, including Flatpaks with X11 access, share one Xwayland server with no isolation between them. Any X11 program can read keys and pixels of other X11 windows, grab the pointer, inject XTEST input and read X11 clipboard copies until cleared.

Xwayland requires the random `MIT-MAGIC-COOKIE-1` in `$XDG_RUNTIME_DIR/reprise/xauth-<n>` (`0600`, `XAUTHORITY`). Flatpaks without X11 access do not receive the cookie. Wayland programs are isolated from X11 apps.

The X11-Wayland clipboard bridge allows reads and writes only while an X11 window has focus. One exception allows a Wayland-mode Flatpak app to copy (never read or touch primary selection) through X11 while its Wayland window has focus, provided the claimed X11 owner belongs to the same `app-flatpak-<P>-<n>.scope`. Because X11 clients can spoof owner windows, another X11 app can still replace the clipboard during that window or while the clipboard holds an X11 copy.

Reprise identifies X11 window processes via Xwayland's `X-Resource` connection data, never `_NET_WM_PID`. Ending an `ssh -X` window disconnects it without signalling local `ssh`.

## The clipboard history

`[clipboard] history` stores the last 25 copies in memory only until session exit. Copies marked `x-kde-passwordManagerHint=secret` (password managers, `wl-copy --sensitive`, private browsing) are excluded and cleared when their app exits. Nothing is recorded while locked.

No program can read the history directly. Compositor memory is not locked against swap or core dumps.

## The lock screen

Locking uses `ext-session-lock-v1`. While locked, regular windows receive no input and virtual keyboards are dropped. Only power, sleep, media (not Wi-Fi), VT switch, layout switch and screen reader keys work. App RemoteDesktop sessions cannot type into the lock screen or press hardware/VT keys, though Reprise's own RDP server can unlock via the remote viewer.

The built-in lock screen locks to the session user and verifies passwords via `reprise --password-conversation=USER` using PAM's `reprise` service (`/etc/pam.d/reprise`, fallback `login`), running `auth` only (90 s timeout). With `/etc/pam.d/reprise-fingerprint`, a second helper handles fingerprints. Password buffers are zeroed.

Local password throttling relies on PAM (`pam_unix` 2 s delay, plus `pam_faillock` on distros like Arch, absent on Ubuntu). Remote viewers throttle separately ([Lock mode](#lock-mode)). The lock power menu offers Restart and Turn Off only under `reprise-login` when no other user session is active and `autologin_user` is unset.

With `[power] locker` set, Reprise keeps the session locked if the external locker crashes. While locked, AT-SPI exposes only the lock screen with bulleted passwords and rejects reader actions. If PAM breaks, switch VT and run `loginctl unlock-session`.

## Login manager

`reprise-login` ([install.md](install.md#use-the-login-manager)) separates privileges across five roles:

- **Daemon (`reprise-login.service`).** Runs as root, opens no network sockets (aside from [Remote logins](#remote-logins)) and keeps state in `/var/lib/reprise-login` (`0711`).
- **PAM workers.** One root worker per login manages `pam_authenticate` through `pam_close_session` and drops privileges for the session.
- **Login screen.** Runs as `reprise-greeter` (no password, no login shell) with `no_new_privs` on VT 1 and no Wayland socket. `Super+Alt+S` launches `/usr/bin/orca` over a sandboxed `WAYLAND_SOCKET`.
- **Sessions.** Run as each user through their login shell with PAM environment variables.
- **Settings helper (`reprise-login-settings`).** Runs as root via `pkexec` on each save, reading config and background image bytes on stdin and stripping image metadata.

Unix sockets in `/run/reprise-login` (`0755`, root):

- `greeter.sock` (`0660`, group `reprise-greeter`): accepts one connection from `reprise-greeter` (`SO_PEERCRED`) to run logins and power actions. Root logins are rejected before PAM, and failures wait 2 s.
- `session.sock` (`0666`): accepts only known logind sessions matching their UID to report interface changes and request Switch User.

Passwords use zeroed buffers and redact in debug output. The daemon unit cannot use systemd sandboxing because user sessions descend from it. Restarting `reprise-login` leaves running sessions alive.

### Switching users

Losing the seat (Switch User or VT switch) locks the session. Logging in again as the same user verifies the password and unlocks the existing session via logind. Sessions refuse logind `Unlock` while off-seat if disconnected from `reprise-login`. `/run/nologin` blocks both logins and resumes.

When another session is running, Shut Down and Restart on the login screen require an admin password (`reprise-login-check` for `wheel`, `sudo` or `admin`, never root), delaying refusals by 3 s.

## Remote Desktop

`reprise-rdp` requires TLS and accepts only the session user. NLA (CredSSP) is not offered for standalone sessions because PAM does not expose stored passwords. Under `reprise-login`, `reprise-rdp` does not listen on the network ([Remote logins](#remote-logins)). Without `reprise-login`, it listens on all interfaces on its configured port, so another local user binding that port first could intercept clients unless they verify the SHA-256 certificate fingerprint from Settings > Remote Desktop (keys live in `$XDG_DATA_HOME/reprise/rdp/`, `key.pem` user-only).

Passwords check against the `reprise-rdp` PAM service (`pam_unix` `auth` and `account` only, no `pam_faillock`, 30 s timeout) so network guesses cannot lock local login.

`reprise-rdp` logs failed logins with client addresses and throttles them:

- **Per address:** 3 free failures, then 2 s exponential backoff up to 60 s (connections close before TLS). Clears on success or after 20 minutes.
- **All addresses:** 10 failures in 10 minutes pause all logins for 10 minutes.

Clients have 10 s for X.224 and TLS ClientHello, then 60 s to authenticate. Up to `max_viewers` connections run concurrently (max 2 authenticating per IPv4 or IPv6 `/64`). With `second_client = "replace"`, one extra connection may authenticate before replacing existing viewers.

Screen capture starts only after authentication and stops on disconnect. Disconnecting the last viewer locks the session unless `[remote_desktop] lock_on_leave = false` ([remote-desktop.md](remote-desktop.md#disconnecting)). Calling `Disconnect` on `org.reprise.RemoteDesktop` drops clients and locks the session (sandboxed callers are refused). Clipboard sync shares text and images only while unlocked and excludes password-marked items.

### Lock mode

With `[remote_desktop] laptop_screen = "lock"` (default, disabled if `[power] locker` is set), the compositor locks local monitors (`LockedHint=true`) and creates virtual monitors matching the client layout. Local input reaches only the local lock screen, while remote input reaches only the virtual monitors.

Virtual monitors lock independently under `reprise-rdp` (with per-address backoff and no fingerprint or `pam_faillock`). Unlocking locally ends lock mode, cuts remote input, returns windows and disconnects the viewer.

## Remote logins

With `[remote_desktop] enabled = true` in `/etc/reprise/login.toml` ([configuration.md](configuration.md#remote-logins)), `reprise-login` adds two processes:

- **Front (`reprise-login-rdp.service`).** Runs as `reprise-login-rdp`, owns the TLS certificate and Remote Desktop NT hashes, handles X.224/TLS/CredSSP and asks the daemon for every decision.
- **Hand-over server (`reprise-rdp --hand-over`).** Socket-activated per user at `$XDG_RUNTIME_DIR/reprise-rdp/handover.sock` (`0600` in `0700`), accepting connections from root for `reprise-login` sessions only.

`reprise-login-rdp.socket` binds the port at boot in `sockets.target` so local users cannot squat on it while enabled. `/run/reprise-login/front.sock` accepts only `reprise-login-rdp`, and `account.sock` (`0666`) lets each user manage their own Remote Desktop password by `SO_PEERCRED` UID.

The front is treated as untrusted and runs heavily sandboxed (`ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`, `PrivateDevices`, `ProtectProc=invisible`, `NoNewPrivileges`, `@system-service`, empty network namespace, `AF_UNIX` only, seccomp blocking new IP sockets/`io_uring`/`MSG_FASTOPEN`, and Landlock TCP rules on Linux 6.7+). Even if compromised, it cannot log in using NT hashes alone because the daemon requires the plaintext password and hands sockets only to the verified UID.

### Checks

For each login, the daemon verifies in order:

1. Throttle state and max 4 concurrent login checks.
2. Username format (`[a-z_][a-z0-9_.-]*`, max 32 bytes, stripping `DOMAIN\` or `@host`).
3. Account exists, is not root and belongs to `[remote_desktop] group` (default `reprise-remote`).
4. Secret: TLS checks PAM `reprise-login-remote` (`pam_unix`, no `pam_faillock`). NLA checks the delegated password's NT hash in constant time and PAM `reprise-login-remote-nla` (UID >= 1000). Both set `PAM_RHOST`.
5. Session availability within `max_sessions`. Expired passwords reach only an already-running session.

TLS refusals delay 3 s to mask account status. NLA rejects invalid Remote Desktop passwords immediately with a uniform error. Handshake timeouts are 10 s for X.224/ClientHello and 60 s for login.

### Throttle

Separate throttles track daemon TLS failures (per client IP), front NLA failures (per IPv4 or IPv6 `/64`) and delegated NLA password failures. Each allows 3 free failures, then 2 s to 60 s backoff (clearing on success or after 20 minutes), and pauses all logins of that kind for 10 minutes after 10 failures in 10 minutes. Restarting `reprise-login` clears throttles.

Pre-auth connection caps on the front: 32 total, 2 per IPv4 or IPv6 `/64`, 4 per IPv6 `/48`, and 256 KiB read per connection.

### Hand-over

After approval, the front enables kernel TLS (TLS 1.2/1.3 with AEAD suites only) and passes the socket, keys and password-redacted handshake recording to the daemon. The daemon opens `handover.sock` via `openat2` (no symlinks or mount crossings), verifies ownership, permissions and `SO_PEERCRED`, and passes the socket to the user's hand-over server. Passwords and NT hashes never leave the front and daemon.

The front certificate lives in `/var/lib/reprise-login-rdp/` (fingerprint shown in Settings > Login Manager and Remote Desktop). Delete `cert.pem` and `key.pem` and run `sudo systemctl restart reprise-login-rdp` to regenerate it.

### Remote Desktop password

NLA requires a separate Remote Desktop password (at least 8 characters, distinct from your login password), set in User Accounts and stored as an unsalted MD4 NT hash in `/var/lib/reprise-login/remote-passwords` (`0600`, root).

- Setting or clearing via `account.sock` requires the caller's own UID and login password.
- Locking a login password (`passwd -l`) does not disable the Remote Desktop password. Clear it or remove the user from `reprise-remote`. Expired accounts are blocked either way.
- Setting a Remote Desktop password keeps TLS login-password auth enabled.
- Clear a user's Remote Desktop password before deleting and recreating an account with the same name and UID.
- `/etc/group` changes apply immediately. Group changes from `systemd-homed`, `sssd`, LDAP or `nscd` need `systemctl reload reprise-login`.

### Background sessions

A remote login without an existing session starts a locked session on the first free VT from 7 upward (`seat0`, `Remote=no`) running in [lock mode](#lock-mode) with no lid or idle-suspend actions.

`/usr/share/polkit-1/rules.d/50-reprise-login.rules` denies `org.freedesktop.login1.chvt` to any caller outside the active console session, preventing background sessions from switching VTs. While any remote client is connected, `reprise-login` holds a root `sleep` inhibitor. Disconnected background sessions are terminated by logind after `[remote_desktop] end_disconnected_after` minutes (console sessions are never terminated).

### Accepted risks

- **Username disclosure.** Unauthenticated NLA packets can reveal whether a username has a Remote Desktop password, never passwords.
- **Global chvt rule.** `50-reprise-login.rules` applies system-wide even under other display managers.
- **Residual hashes.** NT hashes remain on disk after group removal until cleared.
- **Faillock coupling.** Failed `account.sock` or admin-dialog passwords count toward `pam_faillock`.
- **Polkit grants.** Background sessions receive `allow_inactive` polkit permissions (or `allow_active` if the user is active elsewhere, such as over SSH).
- **Denial of service.** 10 failed logins pause remote logins for 10 minutes, multiple IPs can occupy login slots for up to 70 s, repeated failures grow the journal, a user's FUSE mount on `$XDG_RUNTIME_DIR` can stall a daemon thread, and a user's hand-over server can hold the sleep inhibitor.
- **Other gaps.** Late refusals can distinguish valid passwords, system accounts in the remote group with a login shell can use TLS (NLA blocks UID < 1000), and any program of a local admin can change the time zone without a password ([Programs and files](#programs-and-files)).

## Keyboard shortcuts

Windows can inhibit desktop shortcuts via `zwp_keyboard_shortcuts_inhibit_manager_v1` (a balloon notifies on first use). Reprise always keeps power, sleep, media, VT switch, open menus and non-focus-changing binds (lock, accessibility, layout, DND, night light, screenshots, recording, colour picker, hardware toggles).

Press `Super+Shift+Escape` at any time to release shortcut inhibition or end an InputCapture/Deskflow session.

## Focus

A new window receives focus on map only if:

- It presents a trusted `xdg_activation_v1` token (minted by Reprise on launch, tray/notification click, by a focused app after input, by the app a drag was just dropped on, by the app that heard the newest click or touch on that press's serial within 3 s (a panel that takes no keyboard), or for installed `reprise-explorer`, `reprise-settings` and `reprise-notes` called from the focused process tree/multiplexer).
- Reprise launched its program within the last 30 s.
- It shares a Wayland connection, process, session or owner with the focused window, or launched from a multiplexer pane inside the focused terminal.
- It is a prompt from `reprise-portal`, `reprise-polkit` or `reprise-drives`, or a `reprise-bluetooth` window tagged `device-prompt` (its pairing request and file offer). The tag from any other program is ignored.
- No window currently has focus.

Other windows open behind the focused window and flash their task button. No window takes focus while locked.

Background apps cannot steal focus via `xdg_activation_v1`, except for 30 s after you explicitly trigger a search result or notification action (matched strictly by Flatpak app ID for sandboxed apps). Notification signals go only to the originating bus connection.

## The portal

`reprise-portal` accepts calls only from the unsandboxed owner of `org.freedesktop.portal.Desktop` (checked via pidfd). `org.reprise.SessionEnd` accepts calls only from the compositor.

Remembered permissions by portal:

- **ScreenCast and RemoteDesktop.** Checking Remember saves a per-app restore token in xdg-desktop-portal (`screencast`, `remote-desktop`). Restored RemoteDesktop sessions still show the red dot and a persistent critical notification with Stop and forget.
- **Screenshot.** Non-interactive and Active Window requests store permission in table `screenshot`. Interactive picks and area drags are one-shot. Area and Active Window fail while locked.
- **Wallpaper.** Table `wallpaper` stores `yes` (Remember on Allow) or `no` (never remembered from Deny). Prompts preview sealed in-memory bytes and accept only BMP, JPEG, PNG, JPEG XL and AVIF within 16384 px and 512 MiB (max 1 open request per app, 4 total).
- **InputCapture.** Remember saves Allow in table `input-capture` by app ID.
- **GlobalShortcuts.** Allowed apps live in `[[global_shortcuts]]` and denied ones in `[portal] shortcuts_denied`. Reprise blocks unmodified typing/editing keys from being bound.
- **Access and Location.** Access (camera, background) uses xdg-desktop-portal's permission store. Location uses Reprise's geoclue agent store.

Email opens unsent messages with only passed files, Account prompts every time, and DynamicLauncher prompts before adding launchers that run with the app's own privileges.

### Consent prompts

Permission prompts, share pickers, Access prompts and location prompts prevent accidental approval while typing:

- Access keys require `Alt` (bare letters do nothing).
- Input is ignored for 500 ms after gaining focus or display wake.
- `Enter` triggers Deny/Cancel regardless of focus, and `Space` cannot trigger Allow.
- Prompts stack above always-on-top windows, mini views and X11 override-redirect windows, and accept only Wayland windows from the installed portal.
- App RemoteDesktop sessions cannot send input to consent prompts (Reprise's own RDP viewer can).

Revoke stored grants with `flatpak permissions`, `flatpak permission-remove <table> <id>`, Settings > Keyboard, or Settings > Privacy > App Permissions. Stop an active share before forgetting it so it does not re-save on close.

## Programs and files

- **Terminal output.** Control, bidi and zero-width characters are stripped from titles, oversized escapes are dropped, OSC 52 copies work only from the active tab of a focused window, and OSC 52 reads are refused.
- **Bus data and ICC profiles.** Inputs from tray, notifications, LauncherEntry, media players, calendars and ICC profiles are size-capped and stripped of control, bidi and zero-width characters.
- **Search and providers.** Content search (`[launcher] contents`) is off by default so `localsearch` is not started. Search providers receive queries of 2+ characters unless disabled in Settings > Search, match only their own desktop entry, and cannot run commands or touch the clipboard until selected.
- **Notifications and tray icons.** Sandboxed senders are labelled with their verified sandbox desktop entry and cannot pass `image-path` or `sound-file` host paths. Tray icon paths resolve only inside a Flatpak/snap sandbox, are capped (with an SVG time/memory budget) and time out after 2 s on hung mounts.
- **Launchers and scripts.** Folder `.desktop` files run only when executable and carrying a valid per-user HMAC `user.reprise.trusted` xattr (set by Allow Launching, max 128 MiB, invalidated on edit). Executable `#!` scripts, ELF binaries and runnable types (`.exe`, `.msi`, `.bat`, `.lnk`, `.jar`, `.appimage`) prompt on first open unless `confirm_scripts = false` or root-owned under `/usr` or `/opt`. Desktop `Exec` lines never invoke a shell and reject quoted field codes.
- **Drive Recycle Bins and copies.** Drive trash uses real `.Trash-$uid` or sticky `.Trash/$uid` directories without symlinks, and Restore rejects `..` or cross-drive paths. Cross-device copies drop extended attributes, ACLs and trust markers.
- **Extracting archives.** Archives extract into a fresh folder first. Absolute paths, `..`, >90% free space, >1,000,000 entries (or free inodes), escaping symlinks and external hard links abort extraction and delete the folder. Extract Here refuses to overwrite files or hoist hidden top-level entries, dangling links or a top-level `bin` in `$HOME`. A compressed folder is listed from the archive's headers (`bsdtar -tv`: a zip, 7z or plain tar is read without its member data, while a compressed tar is decompressed whole to reach them, as Extract's listing always was). A window keeps the last four archives' listings, failures included, until each changes. Links, devices and names from the root or with `..` are left out, and the listing stops past 1,000,000 entries or 128 MiB of names. Every bsdtar child runs with no core file, a 2 GiB memory limit and a CPU limit of 60 s plus 1 s per 2 MB of archive and of what it writes. It dies with the thread that started it, and leaving the archive stops its listing; one over its CPU limit gives a box saying the archive took too long to read and to extract it instead. It runs with no terminal and a stdin it cannot read, so an encrypted entry fails at once instead of asking for a passphrase; a zip whose central directory flags an encrypted entry, or an archive bsdtar says is encrypted, opens and extracts as a box saying it is password-protected. Opening a file copies only the first entry of that exact name (`bsdtar -xqO`, every byte of the name escaped) into a fresh private folder: no overwrites or links followed, refused unless it is the listed size and within 90% of free space. The copy is read-only. A copy that would run as a program (an executable or script by its first bytes, or a launcher, script or Windows program by its type or suffix, such as `.desktop`, `.sh`, `.py`, `.exe`, `.msi`, `.bat`, `.cmd`, `.vbs`, `.lnk`, `.jar` and AppImage) is deleted unopened, and a box says to extract the folder first. Copy, Copy To Folder... and a drag out of a compressed folder copy the picked files and folders the same way, one `bsdtar -xqO` per 128 files in archive order (a name listed twice in a run of its own), its output split by their listed sizes and refused unless they add up exactly, that Cancel and Pause reach mid-file, into a fresh private folder in the same cache (files 0644 with the copy's own dates, folders 0755, named only by the listing as the folder shows it: each name keeps its first listing), one Copy or Copy To Folder... at a time per window. One is refused past 90% of free space (twice its size when Copy To Folder... goes to the same file system), 1,000,000 entries or a path longer than the system takes; only then do the copies go into the folder picked, through the ordinary copy and its replace prompts. A private folder is held, by a lock on it, by each thing using its copies (the copy filling it, the clipboard or drag offering them, and each paste reading them, in any Explorer window or for the desktop), and is removed when the last of them lets go: Copy's when the window puts something else on the clipboard and no paste still reads it. One whose holders all died goes at the next copy or open a minute later. The locks hold within one machine: with a home shared over NFS by two machines logged in at once, a copy-out on one can sweep the other's held folders once they are a minute old. A drag offers only Copy and no portal key, and copies out at most 1 GiB (more gives a box naming Copy To Folder... and Extract All...). Its entries start coming out as it starts, without a progress dialog; each read of the drag, before or at the drop, waits until they are out (a Reprise target for up to 5 minutes), and a drag that ends without a drop stops the copy and lets go of it. The copies a drop took are handed over to its target, which may read them at any later time (a mail attachment when it is sent): nothing removes them before the sweep a day after the hand-over. A window that closes hands its clipboard's copies over the same way, as the clipboard history may paste them. Extract writes no file past the listing's total size, and stops when bsdtar has written 1 MiB more than that (its write count in `/proc`, which also counts what an outside decompressor writes into it, so a libarchive that hands a format to one reads as writing twice its listing). Where that count cannot be read, it measures each entry bsdtar reports as it lands (no link followed, each file once) and stops past the listing, starting from what has landed when the count goes mid-run, and any extraction whose files hold more than the listing once bsdtar ends is refused and removed. Extract runs under the same 2 GiB memory limit, so an archive packed with a larger dictionary lists but does not extract.
- **Network shares and disk images.** `gio mount` receives addresses after `--` and passwords on stdin (Secret Service stores remembered passwords over the `plain` session bus). Opening `.iso` or `.img` prompts and mounts read-only.
- **Thumbnailers and image codecs.** External `.thumbnailer` processes and JPEG XL/AVIF thumbnails run inside `bubblewrap` (isolated namespaces, read-only `/usr` and input, no home/bus/display, memory and time caps). All JPEG XL (`jxl-rs`) and AVIF (`rav1d`) decoding runs in `reprise-decode` and JPEG XL screenshot encoding in `reprise-encode`, both confined with `no_new_privs`, memory/CPU/timeout limits, Landlock and seccomp (encoding falls back to PNG on failure).
- **App scopes and session exit.** Launched apps run in `app-reprise-<app id>-<n>.scope` for tracking and cleanup, not confinement. `[session] end_processes = true` stops those scopes on Log Off (leaving user services and SSH logins running).
- **Services.** Task Manager starts, stops and restarts a system service only through the system manager's own polkit check (`org.freedesktop.systemd1.manage-units`), never `pkexec` or `sudo`, and never enables a system unit file. Its buttons grey when polkit refuses the action without a unit named, so a rule that allows only some units greys them for every system service. User services go to the user manager. A service's log is `journalctl --unit=NAME` (`--user-unit=` for a user service), the name checked against systemd's unit name characters so it is no option or pattern.
- **Accounts, network and time zone.** User Accounts always requires a password (expiring via `chage -d 0` when requested) and enrolls fingerprints only for the current user. Network secrets save in `/etc/NetworkManager/system-connections`. `/usr/share/polkit-1/rules.d/50-reprise-timezone.rules` allows passwordless `org.freedesktop.timedate1.set-timezone` for local active `wheel`/`sudo`/`admin` users.
- **Bluetooth and Thunderbolt.** Unsolicited pairing prompts, OBEX file receive prompts (off by default, paired devices only, safe filenames in Downloads), Thunderbolt authorizations (`boltd` + polkit) and package install sheets (`PackageKit`, Flatpak or `pkexec pacman -U` on a private file copy) all require explicit consent gated like consent prompts.

## Out of scope and known gaps

- **Shared Xwayland.** All X11 apps and X11-enabled Flatpaks share one server ([X11 programs](#x11-programs)).
- **Level meters light the microphone indicator.** Meter streams are indistinguishable from recorders.
- **No allow-list for privileged globals.** Unsandboxed apps are fully trusted ([The Wayland socket](#the-wayland-socket)).
- **No clipboard portal.** Sandboxed apps get the clipboard only on focus or via `[clipboard] trusted_apps`.
- **X11 clipboard exposure.** X11 apps can read X11 copies and replace the clipboard ([flatpak-clipboard.md](flatpak-clipboard.md#copy-buttons)).
- **Background session polkit grants.** Background sessions receive `allow_inactive` grants ([Accepted risks](#accepted-risks)).
- **Idle inhibitors block idle lock.** Any app (including sandboxed ones) can defer `[idle] lock_after` while unlocked.
- **No NetworkManager secret agent.** Use `nmcli --ask connection up` for agent-secret profiles.
- **Same-user code.** Unsandboxed code running as your user can ptrace, preload or claim D-Bus names. Defending against same-user code is out of scope.

## Reporting

Report vulnerabilities privately as described in [SECURITY.md](../SECURITY.md).
