# Programs

Reprise bundles built-in programs that match your active [interface](interfaces.md). GTK, Qt and X11 apps run alongside them.

## Explorer

Explorer is the default file manager.

### Browse and search

Switch between five views (Thumbnails, Tiles, Icons, List, Details) and sort by name, size, type or date. Navigate with the folder tree, task pane, Back, Forward, Up (`Backspace` or `Alt+Up`; `Alt+Up` also works in the address bar and search field) and the address bar (`Alt+D` or `Ctrl+L`). In list and tile views, letters typed within a second select the first entry that starts with them; Task Manager's Performance list does the same.

Press `Ctrl+H` to toggle dotfiles and `.hidden` entries across Explorer and the desktop. Open `Tools > Folder Options` to set click mode, window reuse, hidden items, known file extensions (shown by default in Explorer, while the desktop uses `desktop_extensions`), infotips, the thumbnail cache and Default Programs (`File Types` tab).

Use `Ctrl+T` for a new tab, `Ctrl+W` to close, `Ctrl+Tab` to switch, and middle-click to open folders in a background tab. Press `Ctrl+F` or `F3` to search by name using `plocate` or `fd` (`Escape` clears).

Right-click a folder for `Open in Terminal` (`terminal`), `Pin to Places` or [Disk Usage](#disk-usage). Favorites share `$XDG_CONFIG_HOME/gtk-3.0/bookmarks` with GTK apps and portal dialogs.

### Connect drives and network shares

Places lists Desktop, Documents, Home, Computer, Network and Recycle Bin. Computer shows drives, `gvfs` mounts and USB phones or cameras. Open a drive to mount it, press `F2` to relabel via `UDisks2`, or choose `Eject` (blocked for system, swap, LVM and RAID volumes). Opening `.iso` or `.img` files prompts to mount them read-only.

Use `File > Connect to Server` or the address bar for `smb://`, `sftp://`, `ftp://`, `ftps://`, `dav://`, `davs://`, `nfs://`, `afp://` or `\\server\share` via `gvfs`. Network lists SMB and SFTP servers discovered via `gvfs` and Avahi.

### Manage files and archives

Copy, move, delete, `F2` rename (including bulk find-and-replace or numbering), Move To, Copy To, Send To and Copy as Path run in the background with Pause, Cancel and `Ctrl+Z` undo (shared with the desktop). While file jobs run, Explorer inhibits log off and shutdown.

`New > Shortcut...` creates a launcher. A web address is named after its host, then after the page's title when it loads within 3 seconds (with curl), and shows the default browser's icon. `Ctrl+V` saves clipboard images in `[screenshot] format` (falling back to PNG). `Send To > Mail Recipient` attaches files via `xdg-email`. Extract `zip`, `7z` and `tar` archives (`Extract Here`, `Extract to` or `Extract All...` into a folder you pick) within safety [limits](security.md#programs-and-files), or compress to `.zip`. `New > Compressed (zipped) Folder`, here and on the desktop, makes an empty zip, and files dropped or pasted onto a zip go into it, the replace prompt asking about each name it already holds. An archive opens as a read-only compressed folder unless you chose a default program for its type (`Open With` > `Always use`): browse it (an address below the archive, such as `~/pack.zip/docs`, typed in the address bar or Run opens that folder of it), open a file (a read-only copy under `~/.cache/reprise/archives`, removed on the first open a day later and left out of Recent files), copy entries out (`Copy` then `Paste`, `Copy To Folder...`, or a drag onto a folder or the desktop: the entries come out into a fresh folder under `~/.cache/reprise/archives` first, with a progress dialog, then copy in as any file does, asking before they replace one; a drag takes up to 1 GiB, comes out as it starts, and reaches only programs outside a sandbox), or `Extract all files` from the task pane. A program inside one (an executable, a script, a launcher or a Windows program, such as `.sh`, `.py`, `.exe`, `.bat`, `.jar` or an AppImage) does not open from there: extract the folder first.

Press `Space` to preview pictures, text, fonts or thumbnails (arrows step, `Enter` opens). Drag-and-drop works with the desktop and Flatpaks (right-drag picks copy, move or shortcut). Recycle Bin uses freedesktop trash with per-drive size and age limits. `Alt+Enter` (`Properties`) shows permissions, Summary, `SHA-256`/`MD5` Checksums and Shortcut tabs. A folder's Customize tab sets its icon and its picture, which Thumbnails view, `Space`, the file dialog's Thumbnails view and the desktop show on the folder. Both are kept in the folder's `.directory` (`Icon=`, `X-Reprise-Picture=`, a path relative to the folder when the picture is inside it), so they move with the folder; an icon or picture set through GNOME's metadata (`metadata::custom-icon`, `metadata::reprise-folder-picture`) shows where the `.directory` names none.

Explorer serves `org.freedesktop.FileManager1`, updates folders live, uses the freedesktop thumbnail cache and runs external PDF, video and font thumbnailers inside `bubblewrap`.

## Notepad

Notepad edits plain text with Find (`F3`), Replace, Go To, Time/Date (`F5`), Word Wrap, status bar (`Ln`, `Col`), zoom from 10% to 500% (`Ctrl+Plus`, `Ctrl+Minus`, `Ctrl+0`), `Format > Font` and 1000-step `Ctrl+Z`/`Ctrl+Y` undo.

It opens UTF-8 (with or without BOM), UTF-16 LE/BE (with BOM) and Western (CP1252), preserving encoding and line endings unless changed in `Save As` (which appends `.txt` unless quoted). Open's Encoding box reads a file as the encoding picked; on Auto-Detect a BOM decides, else a file that is valid UTF-8 opens as UTF-8 and any other as CP1252. A CP1252 document holding a character CP1252 lacks is not saved until you agree to save it as UTF-8. Saves are atomic. Saving over a read-only file says so and opens `Save As` on its name; for a file opened from a compressed folder, `Open` and `Save As` start in the folder holding the archive. Saving a file opened from a zip compressed folder asks whether to update the zip: `Yes` writes the text into it on a background job (with a progress dialog and Cancel), `No` opens `Save As`. Tar and 7z archives go through `Save As`. `File > Print...` (`Ctrl+P`) prints the text wrapped between the margins in the document's font; `File > Page Setup...` picks the paper, orientation, margins in millimetres, and a header and footer, printed in the top and bottom margins (none by default). In them `&f` is the file name, `&p` the page number, `&d` the date, `&t` the time and `&&` an ampersand; `&l`, `&c` and `&r` put what follows at the left, centre or right, and text before them is centred. Print lists the printers CUPS has queues for and the driverless (IPP Everywhere and AirPrint) printers avahi-daemon finds on the network; CUPS makes a temporary queue for one of those when it is printed on. It prints up to 500 pages at a time. `File > Print Preview...` shows the pages as they print, one at a time: `Page Down` and `Page Up` turn them, Zoom In shows a page at its printed size, Print... opens Print on the same pages and `Esc` closes it. Settings, the page setup included, save in `$XDG_CONFIG_HOME/reprise/notepad.toml`, and crash recovery files save in `~/.local/state/reprise/notepad/`. On the next start Notepad offers each one; Yes opens its text as an unsaved Untitled document, or in a new window when a file or unsaved text is already open.

## Sticky Notes

Sticky Notes opens frameless coloured notes. Click `+` (`Ctrl+N`) to add, `x` to delete, drag the top strip to move, drag edges to resize and right-click for edit commands and six colours. A new note opens at the top right of the screen, clear of the desktop icons.

Notes auto-save in `~/.local/share/reprise/notes/` and reopen at login via `reprise-notes --restore` (toggle in Settings > Startup).

## Image Viewer

Image Viewer opens PNG, JPEG, GIF, BMP, WebP, TIFF, ICO, QOI, PNM, HDR, OpenEXR, JPEG XL and AVIF files (including animated GIF, PNG and WebP), refusing images over one-eighth of RAM or 512 MiB. The window opens large enough to show the picture at 100% where the screen has room.

Step through folders with `Left`/`Right`, `Page Up`/`Page Down`, scroll wheel, `Home` or `End`; a picture opened from a compressed folder steps through the pictures beside it in the archive, each copied out as it shows. Zoom with `+`, `-`, `Ctrl`+wheel, `Ctrl+0` (fit) or `Ctrl+1` (100%). Rotate view with `Ctrl+R`/`Ctrl+L`, toggle fullscreen with `F11` and run a 3-second slideshow with `F5`. Use `Ctrl+O`, `Ctrl+C`, `Delete` (not for a picture opened from a compressed folder), `Edit` (Paint), `Show in Folder` and `Set as Desktop Background` (PNG, JPEG, BMP, JPEG XL, AVIF; a picture opened from a compressed folder is first copied to `~/.local/share/reprise/backgrounds`, since its read-only copy is removed a day later). `File > Print...` (`Ctrl+P`) prints the picture as it is turned, at the size the resolution its file states gives it (96 dpi when it states none), or as large as fits between the margins with Page Setup's `Fit to page`; a picture larger than the margins is always shrunk to fit. `File > Page Setup...` sets the paper, margins and scaling for the session. `File > Print Preview...` shows the page as it prints.

## Paint

Paint provides select, eraser, fill, colour picker, magnifier, pencil, brush, airbrush, text, line, curve and shape tools (left-click draws primary colour, right-click draws secondary). `Colors > Edit Colors...` offers 48 basic and 16 custom colours.

Move selections, Cut/Copy/Paste PNG data, flip, rotate, resize, skew, invert, crop, zoom 1x to 8x with pixel grid and undo up to 50 steps (`Ctrl+Z`). Paint reads all Reprise image formats, saves PNG, JPEG and BMP atomically (a read-only file through `Save As`; for a picture opened from a compressed folder, `Open` and `Save As` start in the folder holding the archive, saving one from a zip first asks whether to update the zip, and `Set as Desktop Background` first copies it to `~/.local/share/reprise/backgrounds`), and stores crash recovery PNGs in `~/.local/state/reprise/paint/`. `File > Print...` (`Ctrl+P`) prints the picture at 96 dpi, or as large as fits between the margins with Page Setup's `Fit to page`; a picture larger than the margins is always shrunk to fit. `File > Page Setup...` sets the paper, margins and scaling for the session. `File > Print Preview...` shows the page as it prints.

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

- `Applications` lists each open window by its title, flags `Not Responding` ones and offers `End Task` (`End Now` after 5 s), `Switch To` and `New Task...` (the Run box, also `File > New Task (Run...)`). `End Task` closes the one window; on a program's last window it ends the program. A window of a program Reprise did not start (from an ssh session, a service or D-Bus activation) is listed too; `End Now` ends it only when it is the user's own process and no service's.
- `Processes` lists PID, CPU, memory and optional `Threads`, `Disk`, `GPU` and `Command Line` columns, with confirmed `End Process`. Right-click a process for `Open File Location` and `Properties` of its program file.
- `Performance` graphs CPU (combined or per core), Memory/swap, Disk, Network, GPU (NVIDIA needs NVML) and Sensors.
- `Services` lists your systemd user services (and, with `Show system services`, the system's) with their status and startup. A `Filter:` field narrows the list by name or description and `Show only failed` keeps the failed ones. `Start`, `Stop` (confirmed) and `Restart` run on the selected one; a system service goes through polkit, which may ask for a password. Right-click for `Start at Login` on a user service and `Open Log` (also `Enter` or a double-click), its last 200 journal lines.

Updates every second (configurable under `View > Update Speed`, paused while minimized) and saves preferences in `~/.local/state/reprise/taskmgr.toml`.

## User Accounts

Change your name, picture, password and `fprintd` fingerprints (`Fingerprints` tab). Admins can add or remove accounts and require a password change at next login ([security.md](security.md#programs-and-files)). Under `reprise-login`, the `Password` tab also sets your Remote Desktop password (`reprise-accounts --remote-password`, [remote-desktop.md](remote-desktop.md#remote-logins)).

## Software Updates

Checks system, Flatpak and firmware updates at login, every six hours and on device or Flatpak changes. Nothing installs automatically. Detects external kernel updates or `/run/reboot-required` to prompt for restart (or log off if `/run/reboot-required.pkgs` lists only Reprise), toggled via `[updates] restart_notice`.

Opening `.flatpakref`, `.flatpak`, `.pkg.tar.zst`, `.pkg.tar.xz` or `.deb` launches `reprise-updates --install FILE`. `Uninstall...` in Start or the launcher runs `reprise-updates --uninstall DESKTOP-FILE`. Essential packages and Reprise itself cannot be removed.

## Disk Usage

Open from Explorer or `reprise-diskusage FOLDER` to scan largest files and folders on the same mount (counting hard links once, skipping symlinks). Right-click for `Open in Explorer` or `Move to Recycle Bin`. `Browse...` scans another drive or folder. A low disk space balloon's `Clean Up...` opens it on that disk with Clean Up over it (`reprise-diskusage --cleanup FOLDER`).

`Clean Up...` lists what can go, each with its size: the Recycle Bin (every drive's), the thumbnail cache (`$XDG_CACHE_HOME/thumbnails`), program caches (the rest of `$XDG_CACHE_HOME`), crash dumps (counted through `coredumpctl`, never removed: the system owns them) and the Flatpak runtimes `flatpak uninstall --unused` would remove. Only the Recycle Bin and thumbnails start checked. OK shows what goes, each folder by its path and each runtime by its ref, and asks again; nothing is removed before that Yes. At Yes, Flatpak is asked again and only the listed runtimes it still calls unused go; the system installation asks for a password. A box then says how much was freed and names each part that could not be removed, with the reason. A cache folder or thumbnail cache is left alone when it is a link, someone else's, a drive's root (other than `~/.cache` itself, which may be a subvolume or tmpfs), the home folder or above it, or holds the config, data or state folder or one of your folders (Documents, Pictures...) or where one of them links to, or when there is no home folder. Nothing under them is followed through a link or into another mounted drive.

## Scheduled Tasks

Open from `Start > All Programs > System Tools` to manage systemd user timers with live status (`Ready`, `Running`, `Failed`, `Stopped`, `Disabled`), `Run Now`, `Stop`, `Enable`, `Disable`, `Properties` and `View Output` (last 200 `journalctl --user-unit` lines).

`New Task...` writes `reprise-task-NAME.timer` and `.service` in `~/.config/systemd/user` (`$XDG_CONFIG_HOME/systemd/user`) for every 5, 10, 15 or 30 minutes, hourly, daily, weekly, monthly, one-time or at-log-on (30 s delay) schedules, passing arguments literally. `Once per startup, at log on` runs 30 s after the first log on of each boot only (a stamp in `~/.local/state/reprise/schedtasks`; later log ons read `Skipped, as it already ran since startup`) and needs `reprise-schedtasks` on PATH. Tasks run while logged in unless `loginctl enable-linger` is set, and skip missed runs. A run that fails raises a notification (the service's `ExecStopPost=` runs `reprise-schedtasks --failed NAME`). `Edit...` (the same box; unit names stay) and `Delete` act only on tasks created via `New Task...`, and Edit refuses one whose files were changed by hand.

## Font Viewer

Default viewer for `.ttf`, `.otf`, `.ttc`, `.woff` and `.woff2` files, showing metadata and 12 to 72 px samples. `Install` copies to `~/.local/share/fonts` and runs `fc-cache`.

## Date and Time Properties

Double-click the clock for calendar, clock, time zone and NTP settings. The calendar starts its week on the LC_TIME locale's first weekday, and the Date & Time tab names that locale with today's short date in it. `Set time zone automatically` (`[time_zone] automatic`, off by default) updates via `geoclue` (at most hourly) when location is enabled and passwordless polkit permission is granted.

## Welcome

Runs on first login (`Start > Programs > Accessories > Welcome`) to pick interface, light/dark mode, text size and review active `[[keybinds]]`.

## Import from Another Desktop

Open from Accessories, Settings > Backup or Welcome to read GNOME (`dconf dump /`) or Plasma (`~/.config`) settings read-only and write checked entries into `config.toml`.

| Source setting | Reprise target |
| --- | --- |
| Wallpaper (with dark variant) | `[wallpaper] image`, `position` |
| Dark style | `[appearance] mode = "dark"` |
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
- **Low disk space.** Warns below `[storage] low_space_percent` (default 5%) or 1 GiB, with Open, Clean Up... (Disk Usage's Clean Up on that disk) and, while the Recycle Bin holds something, Empty Recycle Bin.
- **Thunderbolt devices.** Prompts to Authorize Once, Always Authorize or Ignore (manage in Settings > Thunderbolt).
- **Run As.** Polkit password and fingerprint prompt.
- **Remote Desktop server.** `reprise-rdp` (`app-org.reprise.RemoteDesktop`) serves RDP sessions ([remote-desktop.md](remote-desktop.md), [security.md](security.md#remote-desktop)).
- **Screen recorder.** `reprise-record` writes MP4s to `Videos/Screencasts` via [`[recording]`](configuration.md#recording) and the portal ([security.md](security.md#sandboxed-programs)).
- **Remote logins.** `reprise-login-rdp` handles incoming RDP logins under `reprise-login` ([security.md](security.md#remote-logins)).
- **About Reprise.** Hardware, display and uptime summary with `Copy`.
