# Programs

Reprise bundles built-in programs that match your active [interface](interfaces.md). GTK, Qt and X11 apps run alongside them.

## Explorer

Explorer is the default file manager.

### Browse and search

Switch between five views (Thumbnails, Tiles, Icons, List, Details) and sort by name, size, type or date. Navigate with the folder tree, task pane, Back, Forward, Up and the address bar (`Alt+D` or `Ctrl+L`).

Press `Ctrl+H` to toggle dotfiles and `.hidden` entries across Explorer and the desktop. Open `Tools > Folder Options` to set click mode, window reuse, hidden items, known file extensions (shown by default in Explorer, while the desktop uses `desktop_extensions`), infotips, the thumbnail cache and Default Programs (`File Types` tab).

Use `Ctrl+T` for a new tab, `Ctrl+W` to close, `Ctrl+Tab` to switch, and middle-click to open folders in a background tab. Press `Ctrl+F` or `F3` to search by name using `plocate` or `fd` (`Escape` clears).

Right-click a folder for `Open in Terminal` (`terminal`), `Pin to Places` or [Disk Usage](#disk-usage). Favorites share `$XDG_CONFIG_HOME/gtk-3.0/bookmarks` with GTK apps and portal dialogs.

### Connect drives and network shares

Places lists Desktop, Documents, Home, Computer, Network and Recycle Bin. Computer shows drives, `gvfs` mounts and USB phones or cameras. Open a drive to mount it, press `F2` to relabel via `UDisks2`, or choose `Eject` (blocked for system, swap, LVM and RAID volumes). Opening `.iso` or `.img` files prompts to mount them read-only.

Use `File > Connect to Server` or the address bar for `smb://`, `sftp://`, `ftp://`, `ftps://`, `dav://`, `davs://`, `nfs://`, `afp://` or `\\server\share` via `gvfs`. Network lists SMB and SFTP servers discovered via `gvfs` and Avahi.

### Manage files and archives

Copy, move, delete, `F2` rename (including bulk find-and-replace or numbering), Move To, Copy To, Send To and Copy as Path run in the background with Pause, Cancel and `Ctrl+Z` undo (shared with the desktop). While file jobs run, Explorer inhibits log off and shutdown.

`New > Shortcut...` creates a launcher. `Ctrl+V` saves clipboard images in `[screenshot] format` (falling back to PNG). `Send To > Mail Recipient` attaches files via `xdg-email`. Extract `zip`, `7z` and `tar` archives (`Extract Here` or `Extract to`) within safety [limits](security.md#programs-and-files), or compress to `.zip`.

Press `Space` to preview pictures, text, fonts or thumbnails (arrows step, `Enter` opens). Drag-and-drop works with the desktop and Flatpaks (right-drag picks copy, move or shortcut). Recycle Bin uses freedesktop trash with per-drive size and age limits. `Alt+Enter` (`Properties`) shows permissions, Summary, `SHA-256`/`MD5` Checksums and Shortcut tabs.

Explorer serves `org.freedesktop.FileManager1`, updates folders live, uses the freedesktop thumbnail cache and runs external PDF, video and font thumbnailers inside `bubblewrap`.

## Notepad

Notepad edits plain text with Find (`F3`), Replace, Go To, Time/Date (`F5`), Word Wrap, status bar (`Ln`, `Col`), zoom from 10% to 500% (`Ctrl+Plus`, `Ctrl+Minus`, `Ctrl+0`), `Format > Font` and 1000-step `Ctrl+Z`/`Ctrl+Y` undo.

It opens UTF-8 (with or without BOM), UTF-16 LE/BE (with BOM) and Western (CP1252), preserving encoding and line endings unless changed in `Save As` (which appends `.txt` unless quoted). Open's Encoding box reads a file as the encoding picked; on Auto-Detect a BOM decides, else a file that is valid UTF-8 opens as UTF-8 and any other as CP1252. A CP1252 document holding a character CP1252 lacks is not saved until you agree to save it as UTF-8. Saves are atomic, and `Print` is disabled. Settings save in `$XDG_CONFIG_HOME/reprise/notepad.toml`, and crash recovery files save in `~/.local/state/reprise/notepad/`.

## Sticky Notes

Sticky Notes opens frameless coloured notes. Click `+` (`Ctrl+N`) to add, `x` to delete, drag the top strip to move, drag edges to resize and right-click for edit commands and six colours. A new note opens at the top right of the screen, clear of the desktop icons.

Notes auto-save in `~/.local/share/reprise/notes/` and reopen at login via `reprise-notes --restore` (toggle in Settings > Startup).

## Image Viewer

Image Viewer opens PNG, JPEG, GIF, BMP, WebP, TIFF, ICO, QOI, PNM, HDR, OpenEXR, JPEG XL and AVIF files (including animated GIF, PNG and WebP), refusing images over one-eighth of RAM or 512 MiB. The window opens large enough to show the picture at 100% where the screen has room.

Step through folders with `Left`/`Right`, `Page Up`/`Page Down`, scroll wheel, `Home` or `End`. Zoom with `+`, `-`, `Ctrl`+wheel, `Ctrl+0` (fit) or `Ctrl+1` (100%). Rotate view with `Ctrl+R`/`Ctrl+L`, toggle fullscreen with `F11` and run a 3-second slideshow with `F5`. Use `Ctrl+O`, `Ctrl+C`, `Delete`, `Edit` (Paint), `Show in Folder` and `Set as Desktop Background` (PNG, JPEG, BMP, JPEG XL, AVIF).

## Paint

Paint provides select, eraser, fill, colour picker, magnifier, pencil, brush, airbrush, text, line, curve and shape tools (left-click draws primary colour, right-click draws secondary). `Colors > Edit Colors...` offers 48 basic and 16 custom colours.

Move selections, Cut/Copy/Paste PNG data, flip, rotate, resize, skew, invert, crop, zoom 1x to 8x with pixel grid and undo up to 50 steps (`Ctrl+Z`). Paint reads all Reprise image formats, saves PNG, JPEG and BMP atomically, and stores crash recovery PNGs in `~/.local/state/reprise/paint/`.

## Calculator

Switch between Standard (immediate execution: `2 + 3 * 4 = 20`) and Scientific (operator precedence and parentheses: `14`) in `View`, displaying 12 significant digits.

Keys: digits, `.`, `+ - * /`, `Enter`/`=`, `Escape` (C), `Delete` (CE), `Backspace`, Scientific keys (`^`, `!`, `s`, `n`, `p`), memory (`Ctrl+L` MC, `Ctrl+R` MR, `Ctrl+M` MS, `Ctrl+P` M+, `Ctrl+Q` M-), `Ctrl+C`/`Ctrl+V` and `Ctrl+H` history.

## Character Map

Browse every glyph in any installed font, filter by Unicode block, search by name or code point (`U+20AC`), double-click to collect characters and click `Copy`.

## Terminal

`reprise-terminal` is a tabbed terminal (`TERM=xterm-256color`, `COLORTERM=truecolor`). Pass `-e program args...` to run a command directly.

Click `+` for a shell in the current directory, or its arrow for profiles and `~/.ssh/config` hosts. Drag tabs to reorder, middle-click to close, or set `Options > General > Tabs (new windows)` to `In the title bar` for Reprise window tab groups.

Scroll with the bar, wheel or `Shift+Page Up`/`Page Down`, and search scrollback with `Ctrl+Shift+F`. Selection copies to primary selection (hold `Shift` in mouse-aware apps). Paste strips control characters (prompting on multiline unless bracketed paste is on), and dropped files insert shell-quoted paths. Supports OSC 0/2 titles, write-only OSC 52 clipboard (up to 1 MiB), BEL alerts and OSC 8 links (`Ctrl+click` opens `http`, `https`, `mailto` and local `file://` via `xdg-open`). Right-click for Copy, Paste, Select All, Find and New Tab, with Open Link and Copy Link Address on top over a link.

Configure shell, cursor, scrollback (default 10000 lines), font (its size at 100% text; the cells grow with `[accessibility] text_size`), colours, Glass transparency and profiles in Settings (`$XDG_CONFIG_HOME/reprise/terminal.toml`, with window size in `$XDG_STATE_HOME/reprise/terminal.toml`).

Shortcuts: `Ctrl+Shift+T` (new tab), `Ctrl+Shift+W` (close tab), `Ctrl+Shift+N` (new window), `Ctrl+Shift+C`/`V` (copy/paste), `Ctrl+Shift+F` (find), `Ctrl+Tab` (next tab), `Ctrl+=`/`-` (zoom).

## Reprise Settings

Press `Super+I` to open Reprise Settings (34 pages across seven groups). Rows ending in `...` open standalone tools. `Login Manager` appears only when `reprise-login` is installed.

- **Personalize.** Appearance, Background, Screen Saver & Lock, Fonts & Icons, Effects.
- **Taskbar & Windows.** Taskbar, Start Menu, Search, Window Switcher, Virtual Desktops, Window Behavior, Window Rules.
- **Display & Sound.** Display, Night Light, Sound, Screen Capture, Notifications.
- **Devices.** Keyboard, Mouse, Touchpad, Cursor, Tablet, Thunderbolt, Bluetooth..., Printers....
- **Accessibility.** Text & Keys, Pointer & Alerts, Magnifier & Filters.
- **Network.** Connections..., Network (airplane mode and proxy), Remote Desktop.
- **System.** Power Options..., Startup, Default Programs, Privacy, Date and Time..., Region & Language, User Accounts..., Login Manager, Software Updates..., Advanced, Backup.

Press `Ctrl+F` to search pages and controls. `Restore Defaults` clears the current page's keys from `config.toml`. Display changes revert after 15 s unless confirmed. `Login Manager` saves `/etc/reprise/login.toml` via `pkexec`. `Backup` exports settings to `reprise-settings-YYYY-MM-DD.toml` and backs up prior state to `~/.config/reprise/reprise-settings-before-import.toml` on import.

CLI options: `reprise-settings --page <slug>` (such as `display` or `fonts-and-icons`), `--focus <control>`, `--export FILE` and `--import FILE`.

## Volume and sound

Click the tray speaker icon for master volume, per-app sliders and mutes, output switching, default devices and media controls. Sounds and Audio Devices configures defaults, disables unused devices and sets `[sounds] on_connect` (`playback`, `playback_and_recording` or `keep`). Volume Control adds balance, Recording view and hardware Advanced Controls. `[sounds] allow_boost = true` raises sliders and volume keys up to 150%.

## Network Connections

The tray icon shows link status, captive portal balloons and `Airplane Mode` (toggles Wi-Fi, mobile broadband and Bluetooth). Network Connections manages adapters, Wi-Fi (including hidden networks and hotspots), WireGuard (with key generation), OpenVPN imports and 802.1X Enterprise (`PEAP`, `TTLS`, `TLS`, requiring `Domain` with a CA certificate). WEP and secret-agent profiles are unsupported.

## Bluetooth Devices

Pair, connect, rename and monitor devices through BlueZ, with low-battery balloons at `[bluetooth] battery_warning` (default 15%, `0` off). Enable `[bluetooth] receive_files` (off by default, requires `bluez-obex`) on `Options` to accept Object Push transfers into `Downloads` after confirmation.

## Power Options

Configure power profiles, AC/battery timeouts, battery alarms, charge limits, lid/button actions and `Keep Awake` (blocks screen saver, lock, screen off and idle suspend while checked). The battery tray menu switches profiles and toggles `Keep Awake`.

## Printers

View printers and queues, add USB or network printers with the Add Printer Wizard, and configure sharing, ports, paper size, duplex and colour under `Properties`. A tray icon tracks active print jobs.

## Task Manager

Open with `Ctrl+Shift+Escape` or the taskbar menu (override with `task_manager`).

- `Applications` lists each open window by its title, flags `Not Responding` ones and offers `End Task` (`End Now` after 5 s), `Switch To` and `New Task...` (the Run box, also `File > New Task (Run...)`). `End Task` closes the one window; on a program's last window it ends the program.
- `Processes` lists PID, CPU, memory and optional `Threads`, `Disk`, `GPU` and `Command Line` columns, with confirmed `End Process`. Right-click a process for `Open File Location` and `Properties` of its program file.
- `Performance` graphs CPU (combined or per core), Memory/swap, Disk, Network, GPU (NVIDIA needs NVML) and Sensors.

Updates every second (configurable under `View > Update Speed`, paused while minimized) and saves preferences in `~/.local/state/reprise/taskmgr.toml`.

## User Accounts

Change your name, picture, password and `fprintd` fingerprints (`Fingerprints` tab). Admins can add or remove accounts and require a password change at next login ([security.md](security.md#programs-and-files)). Under `reprise-login`, the `Password` tab also sets your Remote Desktop password (`reprise-accounts --remote-password`, [remote-desktop.md](remote-desktop.md#remote-logins)).

## Software Updates

Checks system, Flatpak and firmware updates at login, every six hours and on device or Flatpak changes. Nothing installs automatically. Detects external kernel updates or `/run/reboot-required` to prompt for restart (or log off if `/run/reboot-required.pkgs` lists only Reprise), toggled via `[updates] restart_notice`.

Opening `.flatpakref`, `.flatpak`, `.pkg.tar.zst`, `.pkg.tar.xz` or `.deb` launches `reprise-updates --install FILE`. `Uninstall...` in Start or the launcher runs `reprise-updates --uninstall DESKTOP-FILE`. Essential packages and Reprise itself cannot be removed.

## Disk Usage

Open from Explorer, low-disk-space balloons or `reprise-diskusage FOLDER` to scan largest files and folders on the same mount (counting hard links once, skipping symlinks). Right-click for `Open in Explorer` or `Move to Recycle Bin`. `Browse...` scans another drive or folder.

## Scheduled Tasks

Open from `Start > All Programs > System Tools` to manage systemd user timers with live status (`Ready`, `Running`, `Failed`, `Stopped`, `Disabled`), `Run Now`, `Stop`, `Enable`, `Disable`, `Properties` and `View Output` (last 200 `journalctl --user-unit` lines).

`New Task...` writes `reprise-task-NAME.timer` and `.service` in `~/.config/systemd/user` (`$XDG_CONFIG_HOME/systemd/user`) for every 5, 10, 15 or 30 minutes, hourly, daily, weekly, monthly, one-time or at-log-on (30 s delay) schedules, passing arguments literally. Tasks run while logged in unless `loginctl enable-linger` is set, and skip missed runs. A run that fails raises a notification (the service's `ExecStopPost=` runs `reprise-schedtasks --failed NAME`). `Edit...` (the same box; unit names stay) and `Delete` act only on tasks created via `New Task...`, and Edit refuses one whose files were changed by hand.

## Font Viewer

Default viewer for `.ttf`, `.otf`, `.ttc`, `.woff` and `.woff2` files, showing metadata and 12 to 72 px samples. `Install` copies to `~/.local/share/fonts` and runs `fc-cache`.

## Date and Time Properties

Double-click the clock for calendar, clock, time zone and NTP settings. `Set time zone automatically` (`[time_zone] automatic`, off by default) updates via `geoclue` (at most hourly) when location is enabled and passwordless polkit permission is granted.

## Welcome

Runs on first login (`Start > Programs > Accessories > Welcome`) to pick interface, light/dark mode (no dark mode in Blue), text size and review active `[[keybinds]]`.

## Import from Another Desktop

Open from Accessories, Settings > Backup or Welcome to read GNOME (`dconf dump /`) or Plasma (`~/.config`) settings read-only and write checked entries into `config.toml`.

| Source setting | Reprise target |
| --- | --- |
| Wallpaper (with dark variant) | `[wallpaper] image`, `position` |
| Dark style | Glass `mode = "dark"`, Classic One Dark |
| Keyboard layouts, model, options, repeat | `[keyboard]` |
| Mouse and touchpad speed, gestures, buttons | `[mouse]`, `[touchpad]` |
| Text scaling / forced font DPI | `[accessibility] text_size` |
| Night light state, warmth, schedule | `[night_light]` |
| Pointer theme and size | `[cursor]` |
| Double-click time, workspaces, hot corner | `double_click_ms`, `[workspaces] count`, `hot_corner` |
| Favorites / pinned launchers | Quick Launch, `[taskbar] pinned` |

Settings without a Reprise equivalent (12/24-hour clock, input methods, GTK/icon themes, fonts, disabled key repeat, title bar buttons) appear greyed out.

## Other system tools

- **Safely Remove Hardware.** Ejects drives, phones and cameras. `[drives] on_insert` sets plug-in action (`"ask"`, `"mount"`, `"open"`, `"nothing"`).
- **Phone.** KDE Connect tray battery, ring and file send.
- **Low disk space.** Warns below `[storage] low_space_percent` (default 5%) or 1 GiB.
- **Thunderbolt devices.** Prompts to Authorize Once, Always Authorize or Ignore (manage in Settings > Thunderbolt).
- **Run As.** Polkit password and fingerprint prompt.
- **Remote Desktop server.** `reprise-rdp` (`app-org.reprise.RemoteDesktop`) serves RDP sessions ([remote-desktop.md](remote-desktop.md), [security.md](security.md#remote-desktop)).
- **Screen recorder.** `reprise-record` writes MP4s to `Videos/Screencasts` via [`[recording]`](configuration.md#recording) and the portal ([security.md](security.md#sandboxed-programs)).
- **Remote logins.** `reprise-login-rdp` handles incoming RDP logins under `reprise-login` ([security.md](security.md#remote-logins)).
- **About Reprise.** Hardware, display and uptime summary with `Copy`.
