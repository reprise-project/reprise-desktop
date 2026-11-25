# Programs

Reprise comes with its own programs, drawn with the same controls as the
desktop and dressed in whichever [interface](interfaces.md) you picked. GTK,
Qt and X11 programs run alongside them, of course. These are just the ones
that match the furniture.

## Explorer

The file manager, and the default for folders.

- **Five views:** Thumbnails, Tiles, Icons, List and Details, sorted by name,
  size, type or date. A folder tree or a task pane down the side, Back,
  Forward, Up and an address bar (Alt+D). It remembers your view.
- **Hidden items** (View > Hidden Items, Ctrl+H) shows dot names and the
  names a folder's `.hidden` lists, their icons faded, in every Explorer
  window and on the desktop.
- **Folder Options** (Tools menu) picks whether a folder opens in the same
  window or its own, single- or double-click opening (for the desktop too),
  hidden items, hidden extensions of known file types (shown by default, and
  only in Explorer: the desktop has `desktop_extensions`), item descriptions
  and the thumbnail cache. Apply reaches every open Explorer window and the
  desktop at once. Its File Types tab opens Default Programs.
- **Tabs:** Ctrl+T opens the folder in a new tab, Ctrl+W closes one, Ctrl+Tab
  goes round them, and a middle click on a folder opens it in a tab behind.
  Each tab keeps its own folder, history, view and selection.
- **Search** (Ctrl+F or F3) finds files by name anywhere under the folder
  shown, through plocate or fd, and lists them with an In Folder column.
  Escape in the field empties it, a second Escape (or Escape or Back over the
  results) returns to the folder.
- **Open in Terminal** on a folder's right-click menu starts your `terminal`
  there. **Pin to Places** adds the folder to the task pane and to every file
  dialog's places bar. **Disk Usage** opens [Disk Usage](#disk-usage) on it.
- **Favorites** are GTK's bookmarks (`$XDG_CONFIG_HOME/gtk-3.0/bookmarks`),
  the same list Pin to Places writes, so GTK programs' and the portal's file
  dialogs show them too. Add to Favorites... names the folder shown;
  Organize Favorites... renames, deletes and reorders them. The menu lists
  each one: a server address (`sftp://`, `smb://`, ...) connects as Connect to
  Server does, and an address of another kind or a folder that is not there
  is greyed. A change another program makes shows at once.
- **Places:** Desktop, Documents, Home, Computer, Network and the Recycle Bin.
  Computer lists your drives, locked ones included, the shares gvfs mounted
  and phones or cameras on USB; open one to mount it, Eject to remove it.
  Details shows each drive's Total Size and Free Space, Tiles a usage bar
  and "free of" line; a drive that does not answer stays blank. F2 or Rename
  on a drive sets its file system's label through UDisks2, which asks for a
  password through polkit when the drive is not yours; it is greyed for a file
  system UDisks2 cannot relabel and for the system's own drives (UDisks2's
  system hint, or mounted at `/`, `/boot`, `/efi`, `/boot/efi` or `/usr`),
  whose labels `fstab`, `crypttab` or the boot loader may name. A name may not
  start with `-`, and a drive swapped while the box is up keeps its label.
  Explorer does not format drives.
  The address bar writes a folder on a share, phone or camera under its name
  ("Test Phone/DCIM"), and takes that text typed back.
- **Disk images** (`.iso`, raw `.img`) open by mounting them read-only, after
  asking Mount / Cancel, and stay in Computer until Eject.
- **Connect to Server** (File menu, or the address bar) mounts an `smb://`,
  `sftp://`, `ftp://`, `ftps://`, `dav://`, `davs://`, `nfs://` or `afp://`
  address or a `\\server\share` path through gvfs, and can keep its password.
- **Network** lists the servers gvfs and Avahi find; open one to see an SMB
  server's shares or to connect to an SFTP server.
- **File operations** run in the background, with progress, Pause and Cancel:
  copy, move, delete, Move To, Copy To, Send To, F2 rename (a find-and-replace
  or numbering dialog for several items) and Copy as Path. Ctrl+Z undoes the
  last file operation of any Explorer window or the desktop, which Edit > Undo
  names. While an undo runs in one window, Undo is greyed in the others.
- **Archives:** Extract Here and Extract to for zip, 7z and tar, Compress to
  'name.zip' for the selection. An archive that would land outside its folder
  or fill the disk is refused ([limits](security.md#programs-and-files)).
- **Preview** (Space) shows the selected picture, text file, font or cached
  thumbnail over Explorer; the arrows step through the view and Enter opens it.
- **Drag and drop** between Explorer windows, the desktop and other
  programs, Flatpaks included. A right-drag asks whether to copy, move or
  make a shortcut.
- **Recycle Bin** on the freedesktop trash, shared with other programs, with
  Restore. Recycle Bin Properties (Properties in the Recycle Bin with nothing
  selected, or on the desktop's Recycle Bin) sets each drive's bin size and can
  empty items older than a number of days.
- **Properties** (Alt+Enter): size, dates, Read-only, Hidden and Executable, a
  Summary tab, a Checksums tab (SHA-256 and MD5, with Compare with) and a
  Shortcut tab for launchers. Several files share one sheet. The desktop's,
  the Start menu's and search results' Properties and Open With, and Recycle
  Bin Properties, are Explorer's too, each in a process of its own without a
  folder window.
- **The desktop's file work** runs in Explorer the same way: a drop or paste
  onto the desktop or one of its folders, Delete and Shift+Delete, Empty
  Recycle Bin and Send To > Compressed (zipped) Folder each start a process
  that asks, shows the progress dialog and ends with its job.
- **Log Off waits for file work.** While a copy, move, delete, Empty Recycle
  Bin, extraction or zip runs, in a folder window or a process of its own,
  Explorer holds off log off through the Inhibit portal, as other file
  managers do. Log Off, Turn Off and Restart leave its windows open and list
  it with what it's doing ("Copying files"), and once the job ends they go on.
- **Show in folder** from other programs: Explorer answers
  `org.freedesktop.FileManager1`, so a browser's "Show in folder" opens the
  file's folder with the file selected.
- **Thumbnails** go to the freedesktop cache, shared with other file managers.
  PDFs, videos and fonts get theirs from the installed thumbnailers, each in a
  bubblewrap jail. Folders update by themselves, no F5 needed.

## Notepad

The default for plain text. New, Open, Save, Find (F3 for the next one),
Replace, Go To, Time/Date (F5) and Word Wrap, with Ln and Col in the status
bar. A right-click on the text (or Shift+F10) opens the edit commands.
Ctrl+Z steps back through up to 1000 edits, Ctrl+Y redoes. It opens UTF-8
files, with or without a byte order mark, and UTF-16 LE and BE files with
one, and saves each back in its encoding and line endings; Save As has an
Encoding box. A save never leaves a file half written. Print is greyed out
for now. Notepad has always had its priorities.

When Notepad loses the desktop or crashes with unsaved text, it writes the
text to `~/.local/state/reprise/notepad/` and offers it at the next start,
one prompt per document naming it ("1 of 2"). No sends that text to the
Recycle Bin.

## Sticky Notes

Small coloured notes, each a window of its own with no frame. The strip along
a note's top has + for a new note (Ctrl+N too) and x to delete it; drag the
strip to move the note and its edges to resize it. Right-click the text for
the edit commands and the six colours.

Every change is saved a moment after you stop typing, to
`~/.local/share/reprise/notes/`. Closing a note's window closes all of them
and keeps them; they come back at the next login from the `reprise-notes
--restore` autostart entry, which the Startup page of Settings turns off.

## Image Viewer

The default for pictures, in any format Explorer's thumbnails read (PNG, JPEG,
GIF, BMP, WebP, TIFF, ICO, QOI, PNM, HDR, OpenEXR, JPEG XL and AVIF). It opens fitted
to the window, never past 100%. A picture past an eighth of the machine's
memory, or 512 MiB, says it is too large to show.

- **Walk the folder:** Left and Right (or Page Up and Page Down, or the
  wheel) go to the previous and next picture, sorted by name with numbers in
  order ("img9" before "img10"); Home and End go to the first and last.
- **Zoom:** + and - (or Ctrl and the wheel) step the zoom, Ctrl+0 fits the
  window and Ctrl+1 shows 100%. A larger picture pans by dragging.
- **Rotate:** Ctrl+R and Ctrl+L turn the view; the file is not changed.
- **Full Screen and Slide Show:** F11 or a double-click shows the picture
  over the whole screen; F5 starts a slide show, a picture every 3 seconds.
- **Moving pictures:** animated GIF, PNG and WebP play.
- **Delete** sends the picture to the Recycle Bin, asking first as Explorer
  does, and the next picture shows.
- **Set as Desktop Background** (File menu), greyed for formats the desktop
  does not draw (anything but PNG, JPEG, BMP, JPEG XL and AVIF).
- **Copy** (Ctrl+C) puts the file on the clipboard, and **Open** (Ctrl+O)
  picks another picture.

The status bar shows the name, the place in the folder, the size in pixels,
the file size and the zoom.

## Paint

A picture editor for quick drawings and touch-ups. The tool box holds select,
eraser, fill, pick color, magnifier, pencil, brush, airbrush, text, line,
curve and the shapes; the well under it sets the width, tip or fill. The left
button draws in the first color and the right in the second. Colors > Edit
Colors... opens the Color dialog, with 48 basic and 16 custom colors.

A selection moves with the pointer, and Cut, Copy and Paste carry pictures
as PNG. The Image menu flips,
rotates, resizes, skews, inverts and crops; View zooms from 1x to 8x with a
pixel grid. Ctrl+Z steps back through the last 50 changes. Paint opens the
formats Reprise reads and saves PNG, JPEG and BMP. A save never leaves the
file half written. Paint is offered for pictures in Open With without becoming
their default.

When Paint loses the desktop, crashes or is ended with an unsaved picture, it
writes the picture as a PNG to `~/.local/state/reprise/paint/` and offers it
at the next start.

## Calculator

Standard and Scientific, picked on the View menu. Standard works each
operator as you press it, so 2 + 3 × 4 = is 20; Scientific works the whole
expression with precedence and parentheses, so the same keys give 14.

- **Keys:** digits, `.`, `+ - * /`, Enter or `=`, Escape (C), Delete (CE),
  Backspace, and in Scientific a key for each function (`^`, `!`, `s`, `n`,
  `p`). Ctrl+L, Ctrl+R, Ctrl+M, Ctrl+P and Ctrl+Q are MC, MR, MS, M+ and M−.
- **Clipboard:** Ctrl+C copies the number shown; Ctrl+V pastes a number, or
  in Scientific works out a pasted expression.
- **History** (View > History, Ctrl+H): every answer, with what was asked.

Answers are shown to 12 significant digits.

## Character Map

Every character a font has, in a grid. Pick the font, narrow the grid to one
Unicode block, or search by name ("left arrow") or code (`U+20AC`). Click a
character to see it large with its name and code; double-click it to add it
to Characters to copy, and Copy puts that line on the clipboard.

## Terminal

A tabbed terminal, `reprise-terminal`. It opens in the directory it was
started from; `reprise-terminal -e program args...` runs a program instead of
your login shell. Closing a tab or the window while something other than the
shell runs there asks first, and the window closes when its last tab's
program exits. Programs see `TERM=xterm-256color` and `COLORTERM=truecolor`.

- **Tabs:** + opens your shell in the open tab's directory; the arrow beside
  it lists the shell, your profiles and the `Host` entries of `~/.ssh/config`.
  Drag a tab to reorder it, middle-click it to close it.
- **Tabs in the title bar:** Options > General > Tabs (new windows) set to
  In the title bar makes each tab a window of its own in one caption tab
  group. Without Reprise's tab groups the tabs stay below the toolbar.
- **Scrollback:** the bar down the right edge, the wheel and Shift+Page Up
  and Shift+Page Down scroll back through it; a key goes back to the newest
  output. The bar greys while there is nothing to scroll back to.
- **Find** on the toolbar searches the scrollback, with Match case.
- **Mouse:** a selection goes to the primary selection and a middle click
  pastes it. Hold Shift to select in a program that takes the mouse.
- **Paste:** control characters other than tab and line ends are dropped. A
  paste holding a line end asks first unless the program turned on bracketed
  paste.
- **From programs:** a title (OSC 0 and 2) names the tab and window, OSC 52
  copies to the clipboard (up to 1 MiB, never read back), and BEL rings the
  desktop's bell.
- **Drop:** dropped files type their paths and web links their address,
  quoted for the shell.
- **Options** (Settings): shell, cursor, scrollback (10000 lines by default),
  font, size, colour scheme, a see-through background under Glass, and named
  profiles for the new-tab menu.
- **Shortcuts:** Ctrl+Shift+T new tab, Ctrl+Shift+W close tab, Ctrl+Shift+N
  new window, Ctrl+Shift+C and Ctrl+Shift+V copy and paste, Ctrl+Shift+F
  find, Ctrl+Tab next tab, Ctrl+= and Ctrl+- zoom.
- **Files:** the options live in `$XDG_CONFIG_HOME/reprise/terminal.toml`,
  the window size in `$XDG_STATE_HOME/reprise/terminal.toml`.

## Reprise Settings

One window, 34 pages under seven headings, with link rows ending in "..." that
start the separate settings programs. Login Manager shows only where
`reprise-login` is installed. It opens on the page it was last left on.

- **Personalize:** Appearance, Background, Screen Saver & Lock, Fonts & Icons,
  Effects.
- **Taskbar & Windows:** Taskbar, Start Menu, Search, Window Switcher, Virtual
  Desktops, Window Behavior.
- **Display & Sound:** Display, Night Light, Sound, Screen Capture,
  Notifications.
- **Devices:** Keyboard, Mouse, Touchpad, Cursor, Tablet, Thunderbolt,
  Bluetooth..., Printers....
- **Accessibility:** Text & Keys, Pointer & Alerts, Magnifier & Filters.
- **Network:** Connections... (Network Connections), Network (airplane mode
  and proxy), Remote Desktop.
- **System:** Power Options..., Startup, Default Programs, Privacy, Date and
  Time..., Region & Language, User Accounts..., Login Manager, Software
  Updates..., Advanced, Backup.

The search field above the list (Ctrl+F) leaves only the pages and link rows
whose name, search words or a control's label hold every word typed: "repeat
delay" leaves Keyboard. Start's search and the launcher find the same pages.

Restore Defaults, on every page whose controls write `config.toml` keys,
takes the page's keys that are off their defaults out of `config.toml`. When
`config.toml` changes while Settings is open, the controls you haven't
touched show the new values. A display change asks whether to keep it and
reverts after 15 seconds without an answer. Reprise holds the old settings:
they come back after the 15 seconds even when Settings stops responding, and
at once when Settings is closed or killed during the question.

Login Manager sets the login screen and remote logins for the whole computer,
in `/etc/reprise/login.toml`. OK and Apply save through `pkexec`, which asks
for an administrator's password on each save.

Backup's Export... saves your settings and remembered places in one file
(`reprise-settings-YYYY-MM-DD.toml`), and Import... puts them back, on this
computer or another: `config.toml` with your shortcuts, Terminal's options,
the Quick Launch buttons, where windows open, display arrangements, the file
dialogs' folders and views, and Explorer's view settings. Import keeps the
files it replaces in `~/.config/reprise/reprise-settings-before-import.toml`,
which Import takes back.

`reprise-settings --page display` opens on a page: its name in lower case,
spaces as hyphens and `&` as `and` (`fonts-and-icons`); `--focus mouse_keys`
puts the keyboard on a control. `--export FILE` and `--import FILE` do the
Backup page's work without a window.

## Volume and sound

- **The speaker icon.** One click for a popup with the master volume, a
  slider and mute button for each program, the default devices and the track
  playing. With two outputs or more, a combo moves a program to another one.
  With no device, the device combos read "No audio device".
- **Sounds and Audio Devices.** Set the default devices, disable one you
  never use, and give each program its own volume and output. "When an audio
  device connects" picks what a headset, USB device or monitor plugged in
  later does: it becomes the default output (the default), the default output
  and input, or the defaults stay as they are (`[sounds] on_connect`). A
  Bluetooth headset that takes the input too switches to its headset profile,
  with lower sound quality, whenever a program records. One that only plays
  stays on its playback profile.
- **Volume Control.** A mixer with balance, volume and Mute for the speakers
  and each program, a Recording view, and Advanced Controls for bass, treble
  and whatever else the card offers.
- **Louder than 100 %.** With `[sounds] allow_boost = true` (Settings >
  Sound) the playback sliders and the volume keys go on to 150 %. Turning it
  off brings everything louder than 100 % back to it.

## Network Connections

The tray icon shows the link, a "limited" badge when the internet isn't
there, and a balloon to sign in when a café wants you to. The folder lists
your adapters and VPNs, each with Status and Repair. Pick a Wi-Fi network,
join a hidden one, create a hotspot, write a WireGuard tunnel with its own key
generator, or import a VPN file. Enterprise (802.1X) networks take PEAP, TTLS
or TLS logins, saved in the network's NetworkManager profile. With a CA
certificate the Domain row is required, so a stranger holding another
certificate from the same CA is refused. WEP networks still need
NetworkManager's own editor.

Reprise runs no secret agent, so a network or VPN whose profile leaves its key
to one can't be connected from here, and says so. A join that NetworkManager
gives up on says why in a balloon. The tray icon's Airplane Mode row turns
Wi-Fi, mobile broadband and Bluetooth off, and unticking it turns back on the
radios that were on. It is greyed while a hardware switch holds every radio
off.

## Bluetooth Devices

Pair with the Add Bluetooth Device Wizard, connect and disconnect, rename a
device, see its battery, and turn Bluetooth or discoverability on and off.
Pairing requests from a device show as a prompt naming it; one you didn't
start begins on "Do not allow it". With more than one adapter, the first one
that is on is used, and turning Bluetooth on or off switches every adapter.

The tray icon's menu and tooltip list the connected devices with their
battery, as BlueZ reports it. A balloon warns once when a device's battery
drops to `[bluetooth] battery_warning` (15 % by default, 0 turns it off).

With "Receive files from paired devices" ticked on the Options tab
(`[bluetooth] receive_files`, off by default), a paired device can send files
with Object Push. Each file asks Accept or Decline first and goes to
Downloads. The check box is greyed when obexd (`bluez-obex`) isn't installed.

## Power Options

Power profile, monitor and standby timeouts on mains and on battery, low and
critical battery alarms, a Power Meter for each battery, the battery charge
limit, and what the lid, power button and sleep button do. The battery icon's
menu switches profile. Keep Awake, a check on that menu and on the Power
Schemes tab, holds off the screen saver, the lock, screen off and idle standby
until you clear either check or Power Options exits; the two checks are one
setting. While it is on the icon shows, on a machine without a battery too,
and its tooltip says "Keep Awake is on". While the charge limit holds a plugged-in
battery, the icon's tooltip and the Power Meter say "Charging limited to 80%"
(or wherever the limit stops).

## Printers

The printer folder, a queue for each printer, and the Add Printer Wizard,
which finds printers on USB and the network or takes an address, and picks a
driver by maker and model. Properties covers sharing, ports, paper size,
two-sided and colour. A tray icon shows while something prints and says when
it's done or failed; a failed document stays in its queue until you restart
or cancel it.

## Task Manager

Ctrl+Shift+Escape, or Task Manager on the taskbar's menu, opens it
(`task_manager` picks another program).

- **Applications:** each program started from Start, the launcher or a
  desktop icon, while it shows a window; tray programs with no window open
  are left out. End Task closes its windows as their X button does and
  ends the program only when it shows none, so a tray program whose prompt
  it closed keeps running; End Now is offered for a program still showing a
  window 5 seconds later. Status reads Not Responding while a window doesn't
  answer.
- **Processes:** your processes, or every user's, with PID, CPU and memory;
  View > Select Columns... adds Threads, Disk, GPU and Command Line. End
  Process asks first.
- **Performance:** a list of devices on the left, each with a small history
  and its figure now; the one picked fills the right.
  - **CPU** and **Memory:** load and memory in use over the last hundred
    samples, swap drawn over memory. View > CPU History picks one graph or
    one per core. Temperature and fan show where the sensor chip gives them.
  - **Disk:** busy time, read and write rates, capacity and totals since
    startup, one per whole disk.
  - **Network:** what each adapter that is up receives and sends.
  - **GPU:** usage, video memory, temperature and fan per card. NVIDIA cards
    need the driver's NVML library; a card with no usage source shows "No
    usage data".
  - **Sensors:** every hardware sensor chip, with its temperatures, fans,
    voltages and power.
- **Updates** every second; View > Update Speed sets High, Normal, Low or
  Paused, and nothing is sampled while the window is minimized.
- **Options > Always On Top** keeps the window above the others.
- Always On Top, the update speed, the CPU graph and the Processes columns
  are kept in `~/.local/state/reprise/taskmgr.toml`.

## User Accounts

Change your name, picture and password. With admin rights, add and remove
accounts, choosing what happens to their files. An account that is logged on
can't be deleted, and the only administrator can't be made standard or
deleted. Add Account, and the Password tab of another
account, can ask for a new password at the next login
([security.md](security.md#programs-and-files)). While reprise-login is
installed, your account's Password tab also sets your Remote Desktop password
([remote-desktop.md](remote-desktop.md#remote-logins));
`reprise-accounts --remote-password` opens it there.

The Fingerprints tab adds and deletes your fingers on the fingerprint reader,
through fprintd, which asks for your password first. Without fprintd or a
reader the tab greys and says which is missing.

## Software Updates

Checks for system, Flatpak and firmware updates after login and every six
hours, and when a Flatpak installation changes or a firmware device is plugged
in. Lists them by source, all ticked; untick what you'd rather skip and click
Install. When a device asks for something mid-update (unplug it, press a
button), the window says what, or a balloon with the window closed. Nothing
installs on its own, and closing the window doesn't stop an install halfway.
While PackageKit has an offline update prepared, checks, Check Now included,
list updates without downloading new package lists, as new lists would
discard the prepared update.

It also notices a system updated from elsewhere, such as `pacman -Syu` in a
terminal, once the running kernel's modules are gone or a package leaves
`/run/reboot-required`, and says "Restart to finish updating" with Restart
Now and Later. When `/run/reboot-required.pkgs` names only Reprise (its
packages write it on an upgrade), it says logging off and back on is enough. Restart Now goes the way Start's Restart does: programs are
asked to close first. Settings > Advanced turns it
off (`[updates] restart_notice`).

### Install and uninstall

Opening a `.flatpakref`, a `.flatpak` bundle, an Arch package (`.pkg.tar.zst`
or `.pkg.tar.xz`) or a Debian package (`.deb`) opens Install Software, and
Uninstall... on a program's menu in Start or the launcher opens Uninstall
Software (`reprise-updates --install FILE`, `reprise-updates --uninstall
DESKTOP-FILE`). The sheet shows the name, version, size and who built it, and
lists any other package that comes along or goes too. A change that would
remove one of Reprise's own packages or one the system marks essential is
refused, and so is an Arch package that conflicts with an installed one
(pacman would have to remove it). Nothing changes until you click Install or Uninstall; polkit asks
for an administrator's password where the system's rules want one. A
packager reads "(unverified)" unless pacman will check a `.sig` beside the
package. Cancel stops a Flatpak run at any point, and a PackageKit run while
it is still downloading.

## Disk Usage

What takes a folder's or a drive's space: its folders and files, largest
first, each with its size, a bar for its share of the folder and how many
files it holds. Sizes fill in as the count runs in the background; they are
what the files take on disk, a hard-linked file counted once. Links are not
followed and other drives mounted inside are not entered. Double-click a
folder to go into it; Back, Up and Refresh do what they say, and an entry's
menu adds Open in Explorer and Move to Recycle Bin. Open it from Explorer,
from the low disk space balloon, or as `reprise-diskusage FOLDER`.

## Scheduled Tasks

The tasks your systemd user manager runs on a schedule: each user timer with
its schedule in words ("Every day at 3:00", "15 min after startup"), when it
runs next, when the timer last fired and its status (Ready, Running, Failed or
Disabled). Run Now
starts the task's service at once. Enable and Disable turn the timer on or off
for good, as `systemctl --user enable --now` and `disable --now` do; a timer
without an `[Install]` section is only started or stopped. Properties shows the
schedule as the unit file writes it, the command, how the last run ended and
the unit file. The list follows the manager, so a timer added, changed or
fired elsewhere shows at once. It's in Start > All Programs > System Tools.

New Task... is greyed: to add a task, put a `.timer` and a `.service` of the
same name in `~/.config/systemd/user` and run `systemctl --user daemon-reload`;
the list picks it up.

## Font Viewer

The default for font files (`.ttf`, `.otf`, `.ttc`, `.woff`, `.woff2`). It
shows the face's name, version and copyright, the alphabet, and a pangram at
sizes from 12 to 72 px. Install copies the file into your fonts folder
(`~/.local/share/fonts`) and runs `fc-cache` on it; a face already there says
Installed.

## Date and Time Properties

Double-click the clock. A month grid, an analog clock, the time zone and
network time.

"Set time zone automatically" on the Time Zone tab (`[time_zone] automatic`,
off by default) sets the zone of the nearest town of 15,000 people or more,
from where geoclue finds the machine. It looks when you turn it on and at
login, then after a resume or a new network connection, at most once an
hour. It is greyed with a note when geoclue is missing, when location is off
in Settings > Privacy, or when polkit wants a password to change the zone;
the package's polkit rule lets a local, active administrator (group `wheel`
or `sudo`) change it without one.

## Welcome

Shows at a new user's first login and steps through three pages: the
interface (Classic, Blue or Glass, with a sample of each), light or dark and
the text size, then the keys worth knowing with a button for Settings. Each
pick applies as you make it, and each lives in Settings too. Skip closes it.
Start > Programs > Accessories > Welcome runs it again. A user who already ran
Reprise never sees it on an upgrade. The last page names the launcher and
clipboard history keys as your `[[keybinds]]` have them. Blue comes in light
colours only, so Dark is greyed under it.

## Import from Another Desktop

Brings settings over from GNOME or Plasma used in the same account. It reads
GNOME's through `dconf dump /` (so only what you changed from GNOME's
defaults) and Plasma's from its files in `~/.config`. It never writes either.

The list shows each setting that would change, with its value now and the one
it would take, each with a check box. Nothing is written until you click
Import, and then only the checked rows, into `config.toml` with its comments
kept. When both desktops left settings, a box picks which one to import from.

| Found | Becomes |
| --- | --- |
| Wallpaper (the dark one under a dark style) | `[wallpaper] image` and `position`; JPEG, PNG, BMP, JPEG XL and AVIF only |
| Dark style | Glass `mode = "dark"`, Classic's One Dark scheme; greyed under Blue |
| Keyboard layouts, variants, model, options, layout per window | `[keyboard]` |
| Key repeat rate and delay | `[keyboard] repeat_rate`, `repeat_delay` |
| Mouse and touchpad speed, acceleration, natural scrolling, left-handed buttons | `[mouse]`, `[touchpad]` |
| Tap to click, two-finger scrolling and tap, off while typing, click method, off with a mouse | `[touchpad]` |
| Text scaling (GNOME), forced font DPI (Plasma) | `[accessibility] text_size`, the nearest step |
| Night light on, warmth, schedule | `[night_light]` |
| Pointer theme and size | `[cursor]` |
| Double-click time, fixed number of workspaces, hot corner | `double_click_ms`, `[workspaces] count`, `hot_corner` |
| Favourite apps (GNOME), task manager launchers (Plasma) | Quick Launch buttons, and `[taskbar] pinned` |

What has no Reprise setting is listed greyed at the end with why: the 12 or
24 hour clock (Reprise's follows the formats locale), input methods, the GTK
and icon themes and fonts (Reprise's look sets them), key repeat turned off
and title bar buttons. Start >
Programs > Accessories, the Backup page in Settings and Welcome's last page
all open it. Reading GNOME's settings needs the `dconf` command.

## Smaller things

- **Safely Remove Hardware.** Shows a tray icon while there's a drive, phone
  or camera to eject. What a plugged-in drive does is `[drives] on_insert`,
  set on Settings > Default Programs:
  - `"ask"` (the default): a "New drive" balloon with Mount and Open and Mount.
  - `"mount"` mounts it, `"open"` mounts it and opens a window on it.
  - `"nothing"` leaves it in Computer.
- **Phone.** With KDE Connect installed, a phone icon lists each paired phone
  in reach with its battery; its menu rings one, sends it files and opens KDE
  Connect, where pairing happens.
- **Low disk space.** A balloon says when your home folder's disk or `/` has
  less free than `[storage] low_space_percent` (5 % by default) or 1 GiB, with
  Open, Show Usage and Empty Recycle Bin.
- **Thunderbolt devices.** A Thunderbolt or USB4 device that connects
  unauthorized gets a balloon with Authorize Once, Always Authorize and
  Ignore. Settings > Thunderbolt lists and forgets enrolled devices.
- **Run As.** The admin password prompt, which takes a fingerprint too when
  polkit is set up for one. Cancelling it leaves things as they were.
- **Remote Desktop server.** `reprise-rdp`, the user unit
  `app-org.reprise.RemoteDesktop`, lets RDP clients use the session
  ([remote-desktop.md](remote-desktop.md), [security.md](security.md#remote-desktop)).
- **Screen recorder.** `reprise-record` records a screen or an area, with sound
  and the microphone if asked, into an MP4 under Videos/Screencasts as [`[recording]`](configuration.md#recording)
  says. It reads the screen through the portal ([security.md](security.md#sandboxed-programs)).
- **Remote logins.** Under `reprise-login` with remote logins on,
  `reprise-login-rdp` listens on the port instead and hands each login to the
  user's session ([security.md](security.md#remote-logins)).
- **About Reprise.** Version, operating system, kernel, model, processor,
  graphics, memory, disk, displays and uptime, with a Copy button for bug
  reports.
