# Programs

Reprise comes with its own programs, drawn with the same controls as the
desktop and dressed in whichever [interface](interfaces.md) you picked. GTK,
Qt and X11 programs run alongside them, of course. These are just the ones
that match the furniture.

## Explorer

The file manager, and the default for folders.

- **Five views:** Thumbnails, Tiles, Icons, List and Details, sorted by name,
  size, type or date. A folder tree or a task pane down the side, Back and
  Forward with their history, Up and an address bar (Alt+D). It remembers
  your view and window size.
- **Tabs:** Ctrl+T opens the folder shown in a new tab, Ctrl+W closes one
  (the last closes the window), Ctrl+Tab and Ctrl+Shift+Tab go round them.
  Middle-click a folder, or pick Open in New Tab on its menu, to open it in
  a tab behind; middle-click a tab to close it, drag one to move it. Each
  tab keeps its own folder, history, view, selection and scroll, and the
  title bar and taskbar show the open one's folder. The strip under the
  address bar shows from the second tab on.
- **Search** (Ctrl+F or F3): the box at the toolbar's right end finds files
  by name anywhere under the folder shown, through plocate or fd, and lists
  them in the view with an In Folder column. The usual right-click menu works
  on them. Escape, or going to another folder, brings the folder back.
- **Open in Terminal** on a folder's right-click menu, and on the
  background's in any folder but Computer and the Recycle Bin, starts your
  `terminal` in that folder.
- **Pinned folders:** Pin to Places on a folder's menu adds it to the
  task pane's Pinned group and to every file dialog's places bar (see
  Pinned folders in features.md).
- **Disk Usage** on a folder's or a drive's right-click menu opens
  [Disk Usage](#disk-usage) on it.
- **Places:** Desktop, Documents, Home, Computer and the Recycle Bin.
  Computer lists your drives, unmounted and locked removable ones included:
  open one to mount it (a locked one asks for its passphrase first), Eject to
  remove it safely. Network and FUSE mounts stay out of
  that list, so a slow server can't stall it. A disk image (`.iso`, raw
  `.img`) opens by mounting it read-only, Mount being its default row: the
  whole image's file system, or each partition's when it has none, then
  Explorer goes into the first. It stays in Computer until Eject unmounts
  it and detaches it; its menu's Open still hands it to its program.
- **File operations** run in the background, so a big copy doesn't freeze
  anything. A copy, move, delete, Restore or Empty Recycle Bin that takes
  longer than half a second shows its progress with Pause and Cancel, on the
  desktop too. Cut, Copy, Paste (with Yes to All), Paste Shortcut, New Folder,
  rename with F2, Move To, Copy To, Open With, and Send To a zip file, the
  desktop, a mail, a removable drive or a phone KDE Connect reaches. An archive (zip, 7z, tar plain or
  compressed) has Extract Here, which writes over nothing already there, and
  Extract to, into a new folder named after it; an archive with an entry
  that would land outside that folder is refused. Compress to 'name.zip'
  (or Send To > Compressed (zipped) Folder) zips the selection beside it,
  named after the one item or the folder of several, with progress and
  Cancel; a cancelled or failed zip leaves no file behind. F2 with several
  items selected opens a Rename dialog: find and replace (optionally ignoring
  case) or a base name numbered in view order ("Photo (1)", "Photo (2)"),
  with extensions kept by default and a preview of every new name; a name
  in use or not valid greys OK. Launchers are left out of it. Copy as Path
  (Ctrl+Shift+C) puts the selected items' full paths on the clipboard as
  text, one per line, in double quotes when a shell would split them.
  Ctrl+Z undoes the last one, a whole Rename dialog's worth at once;
  undoing a copy its Recycle Bin can't take (a drive set to remove files
  immediately, or too big) asks before deleting it permanently.
- **Preview** (Space): a window over Explorer shows the one selected file: a
  picture fitted to the screen, the first 256 KB of a text file, the
  thumbnail another program left in the shared cache (a PDF's, a video's), a
  font's sample, else its icon, type, size and date. The arrows step to the
  previous and next item in the view's order and move the selection with it;
  Enter opens the file, Space or Escape closes the preview. A file on a
  mount that stopped answering shows its card until the read comes back.
- **Drag and drop** between Explorer windows, the desktop and other
  programs, Flatpaks included. A right-drag asks whether to copy, move or
  make a shortcut. Drop a link from a browser and you get a link file.
- **Recycle Bin** on the freedesktop trash, shared with every other Linux
  program, with Restore and the original location of everything in it.
  Recycle Bin Properties can empty items older than a number of days, at
  login and then daily.
- **Properties** (Alt+Enter): size, dates, Read-only, Hidden and
  Executable, a Summary tab whose title, author and keywords live in the
  file's extended attributes, a Checksums tab, a Shortcut tab for launchers
  and a Change Icon for folders. The Checksums tab shows a file's SHA-256
  and MD5 with Copy buttons: a file up to 100 MB is hashed as the tab opens,
  a larger one at Calculate, with its progress, and closing the sheet stops
  it. Paste a hash into Compare with and it says "Matches SHA-256",
  "Matches MD5" or "No match".
  Select several files for one sheet that covers them all.
- **Thumbnails** decode in the background into the freedesktop thumbnail
  cache, shared with other file managers: its 128 px `normal` size at scale
  1, its 256 px `large` size at scale 2. Files Reprise does not decode
  itself (PDFs, videos, fonts) get theirs from the freedesktop thumbnailers
  installed in `share/thumbnailers`, two at a time at idle priority, each
  stopped after 10 seconds; a file one fails on is not tried again until
  it changes. The Browse... dialogs of Run and
  Open With read the same cache and leave the pictures it lacks to Explorer,
  so the desktop itself never decodes a picture. Folders update by
  themselves, no F5 needed.

## Notepad

The default for plain text. New, Open, Save, Find (F3 for the next one),
Replace, Go To and Time/Date (F5), with Ln and Col in the status bar. A
right-click on the text (or Shift+F10) opens Undo, Redo, Cut, Copy, Paste,
Delete and Select All, and Paste greys while the clipboard holds no text.
Ctrl+Z steps back through up to 1000 edits, and Ctrl+Y or Ctrl+Shift+Z
redoes. It opens UTF-8 files, with or without a byte order mark, and
UTF-16 LE and BE files with one, and saves each back in its encoding with
its LF, CRLF or CR line endings. Save As has an Encoding box (UTF-8, UTF-8
with BOM, UTF-16 LE, UTF-16 BE) that starts on the document's encoding; the
one picked is kept for later saves. Notepad never leaves a file half saved,
asks before opening a file that doesn't decode, and grows with Larger Text. Format > Word Wrap
folds long lines into the window, with no horizontal scrollbar; Ln and Col still count the file's
lines. Print is greyed out for now. Notepad has always had its priorities.

When Notepad loses the desktop or crashes with unsaved text, it writes the text to
`$XDG_STATE_HOME/reprise/notepad/` (`~/.local/state/reprise/notepad/` by default) and offers it
the next time it starts.

## Sticky Notes

Small coloured notes, each a window of its own. The strip along a note's
top has + for a new note (Ctrl+N too) and x to delete it, which asks first
when the note has text. Right-click the text (or Shift+F10) for Undo, Redo,
Cut, Copy, Paste, Delete and Select All, the six colours (yellow, green,
pink, purple, blue, white), New Note and Delete Note. Lines wrap at the
note's edge and rewrap when it is resized.

Every change is saved a moment after you stop typing, to
`$XDG_DATA_HOME/reprise/notes/` (`~/.local/share/reprise/notes/` by
default): `<id>.txt` holds a note's text as typed and `<id>.note` its
colour and size. Each note opens where it was left. Closing a note's window
closes all of them and keeps them; they come back at the next login, from
the `reprise-notes --restore` autostart entry, which you can turn off on
the Startup page of Reprise Settings. Starting Sticky Notes while it runs
opens a new note.

## Image Viewer

Opens a picture in any format Explorer's thumbnails read (PNG, JPEG, GIF, BMP,
WebP, TIFF, ICO, QOI, PNM, HDR and OpenEXR) and is the default for pictures.
It opens fitted to the window, never past 100%. The decode runs in the
background, and a large picture is shown from a copy scaled to the window.

- **Walk the folder:** Left and Right (or Page Up and Page Down, or the
  wheel) go to the previous and next picture in the folder, sorted by name
  with numbers in order ("img9" before "img10"); Home and End go to the first
  and last.
- **Zoom:** + and - (or Ctrl and the wheel, at the pointer) step the zoom,
  Ctrl+0 fits the window and Ctrl+1 shows the picture at 100%, one picture
  pixel per screen pixel. A picture larger than the window pans by dragging,
  with the scrollbars, or with Up and Down.
- **Rotate:** Ctrl+R turns the picture clockwise and Ctrl+L back. The turn is
  for viewing; the file is not changed.
- **Full Screen and Slide Show:** F11 or a double-click shows the picture on
  black over the whole screen; F5 starts a slide show there, a picture every
  3 seconds. Escape comes back.
- **Moving pictures:** animated GIF, PNG and WebP play.
- **Delete** sends the picture to the Recycle Bin, asking first as Explorer
  does (Shift+Delete deletes it for good), and the next picture shows.
- **Set as Desktop Background** (File menu) makes the picture the desktop's
  background and ends a background slide show. It is greyed for formats the
  desktop does not draw (anything but PNG, JPEG and BMP).
- **Copy** (Ctrl+C) puts the file on the clipboard to paste in Explorer, and
  **Open** (Ctrl+O) picks another picture.

The status bar shows the name, the place in the folder, the size in pixels,
the file size and the zoom.

## Paint

A picture editor for quick drawings and touch-ups. The tool box holds free-form and rectangle
select, eraser, fill, pick color, magnifier, pencil, brush, airbrush, text, line, curve,
rectangle, polygon, ellipse and rounded rectangle; the well under it sets the line width, brush
tip, eraser or spray size, outline and fill, or whether a selection covers what is under it. The
left button draws in the first color and the right in the second; a left or right click on the
color box picks them. Shift keeps lines level or at 45° and shapes square.

A selection moves with the pointer, Ctrl drags a copy, and Cut, Copy and Paste carry pictures on
the clipboard as PNG; a pasted picture larger than the canvas grows it. The text tool types into
a box with the font, size and bold set in the Fonts toolbar, and the text goes on the picture when
you click outside the box. The Image menu flips, rotates, resizes and skews, inverts the colors,
sets the canvas size (Attributes), clears the picture and crops it to the selection. View zooms
from 1x to 8x, with a pixel grid from 4x (Ctrl+G turns it off). Ctrl+Z steps back through the
last 50 changes, fewer when they are large.

Paint opens PNG, JPEG, BMP, GIF, WebP and the other formats Reprise reads, and saves PNG, JPEG
(the Quality box in Save As) and BMP; transparent pixels open on white. Set as Desktop Background
saves the picture and makes it the wallpaper. Paint is offered for pictures in Open With without
becoming their default.

## Calculator

Standard and Scientific, picked on the View menu. Standard works each
operator as you press it, so 2 + 3 × 4 = is 20; Scientific keeps the whole
expression and works it with precedence and parentheses, so the same keys
give 14, the answer the launcher gives for `2+3*4`. Both show the operation
in progress above the number and an M while the memory holds a value.

- **Keys:** digits, `.`, `+ - * /`, Enter or `=`, Escape (C), Delete (CE),
  Backspace, `%`, `@` (square root), `r` (1/x) and F9 (±). In Scientific
  also `^` or `y` (xʸ), `q` (x²), `!`, `s`, `o`, `t` (sin, cos, tan in the
  degrees, radians or grads picked beside the keys), `n` (ln), `l` (log),
  `p` (π), `e` and parentheses. Ctrl+L, Ctrl+R, Ctrl+M, Ctrl+P and Ctrl+Q are
  MC, MR, MS, M+ and M−.
- **Clipboard:** Ctrl+C copies the number shown; Ctrl+V pastes a number, or
  in Scientific works out a pasted expression.
- **History** (View > History, Ctrl+H): every answer, with what was asked.
  Click one to bring its answer back.

Answers are shown to 12 significant digits.

## Character Map

Every character a font has, in a grid. Pick the font (every family
installed), narrow the grid to one Unicode block, or search: a word or
several finds the characters whose names hold them all ("left arrow"), a
code finds its character (`U+20AC` or `20ac`), and a single character finds
itself. Click a character to see it large with its name and code; double-click
it, press Enter or Select to add it to Characters to copy, and Copy puts
that line on the clipboard. The arrows, Page Up, Page Down, Home and End move
in the grid, Ctrl+C copies the one picked, and Tab goes round the controls.

## Terminal

A tabbed terminal, `reprise-terminal`. It opens in the directory it was
started from; `reprise-terminal -e program args...` runs a program instead of
your login shell. The window closes when its last tab's program exits, and
closing a tab or the window while something other than the shell runs there
asks first. Programs see `TERM=xterm-256color` and `COLORTERM=truecolor`.

- **Tabs:** + opens your shell in the open tab's directory; the arrow beside
  it lists the shell, your profiles and the `Host` entries of
  `~/.ssh/config` (and the files it includes). Drag a tab to reorder it,
  middle-click it to close it.
- **Tabs in the title bar:** Options > General > Tabs (new windows) set to
  In the title bar makes each tab a window of its own in one caption tab
  group, with New Tab at the start of the toolbar. Open windows keep the
  mode they opened with. Without Reprise's tab groups (or with `[windows]
  tabs = false`) the tabs stay below the toolbar.
- **Toolbar:** Copy, Paste, Find and Settings. Find searches the scrollback
  up or down, with Match case, and highlights what it found.
- **Mouse:** a selection goes to the primary selection and a middle click
  pastes it. A program that takes the mouse gets the clicks; hold Shift to
  select anyway. The wheel scrolls back.
- **Paste:** control characters other than tab and line ends are dropped
  from a paste. A paste holding a line end into a program that has not
  turned on bracketed paste asks first, showing its first lines.
- **Drop:** files dropped on the window type their paths into the open tab,
  quoted for the shell where needed, and web links their address, each
  followed by a space; dropped text types as it came. A drop goes in as a
  paste and brings the window to the front.
- **Options** (Settings): the shell, cursor shape and blinking, the
  multi-line paste question, scrollback lines (10000 by default, at most
  100000) and where tabs go on General; the font, size in pixels,
  colour scheme (Reprise, Light or Dark) and, under Glass, a see-through
  background with its opacity on Appearance; on Profiles, named commands for
  the new-tab menu. Other terminal windows take a change when next focused.
- **Shortcuts:** Ctrl+Shift+T new tab, Ctrl+Shift+W close tab, Ctrl+Shift+N
  new window, Ctrl+Shift+C and Ctrl+Shift+V copy and paste, Ctrl+Shift+F
  find, Ctrl+Tab and Ctrl+Shift+Tab (or Ctrl+PgDn and Ctrl+PgUp) next and
  previous tab, Ctrl+= and Ctrl+- zoom and Ctrl+0 back, Shift+PgUp and
  Shift+PgDn page through the scrollback.
- **Files:** the options live in `$XDG_CONFIG_HOME/reprise/terminal.toml`,
  the window size in `$XDG_STATE_HOME/reprise/terminal.toml`.

## Reprise Settings

One window, 23 pages, four headings. Only one runs at a time.

- **Look**
  - **Appearance:** interface and colour scheme. Under Glass: mode,
    material, frame, transparency, blur, reflections and colour intensity.
  - **Background:** a picture or a slideshow, its position, the desktop
    colour, which icons sit on the desktop and whether their labels show
    file name extensions. Use for picks one virtual desktop to give a picture
    of its own; picking the picture for all desktops there takes it away.
  - **Screen Saver:** the saver, its settings and a live preview.
  - **Display:** arrange monitors by dragging them, or pick one with the
    arrows and move it with Ctrl+arrows to the next edge that way, Identify,
    the primary screen, and each screen's resolution, refresh rate, orientation, scale
    and adaptive sync. Forget saved layouts drops the layouts the desktop
    remembers per set of monitors.
  - **Night Light:** warmth, and sunset to sunrise or a schedule.
  - **Fonts & Icons:** the UI and dialog fonts, the icon theme, frame and
    caption sizes, and how GTK and Qt programs are themed.
  - **Effects:** window animations, highlight fades, menu shadows and how menus
    open (fade, slide or none) and taskbar slides.
- **Desktop**
  - **Taskbar:** position, button mode, pinned programs, Peek, Show desktop,
    tray overflow, auto-hide, and the Start button and menu style.
  - **Start Menu:** the Start menu tree and the Quick Launch buttons.
  - **Launcher:** whether it searches your files and open windows.
  - **Window Switcher:** Alt+Tab's icons or thumbnails, picture size, Peek,
    the 3D switcher and the overview's hot corner.
  - **Desktops:** how many, how they animate, their names, the taskbar's
    live previews, and the desktop widgets.
  - **Windows:** snapping, snap layouts, the snap bar, window tabs and window
    memory.
- **Input**
  - **Keyboard:** layouts, repeat, every shortcut, programs' global
    shortcuts included, whether Caps Lock and Num Lock show on screen, and
    the sheet holding Super shows.
  - **Mouse:** pointer speed, double-click speed, scrolling, and the
    touchpad's tapping, scrolling and disable-while-typing.
  - **Tablet:** what each pad button, pad key, ring, strip and dial does,
    and how far a ring or strip turns per step.
  - **Pointers:** cursor theme and size.
  - **Accessibility:** text size, StickyKeys, SlowKeys and MouseKeys.
  - **Vision:** colour filters and the magnifier.
- **System**
  - **Sounds:** the sound theme, and whether notifications and a program's
    bell play a sound.
  - **Power:** the Turn Off commands, idle timeouts, lock on sleep and the
    lock screen's wallpaper.
  - **Startup:** autostart entries and session restore.
  - **Default Programs:** browser, mail, music, video, photos, text editor,
    file manager, terminal and its arguments, what a plugged-in drive does,
    whether opening a script runs it on its own or in the terminal, whether
    a script's window stays open when it ends, and the program for
    every file type, which Browse... can pick by its file when it has no
    desktop entry.
  - **Advanced:** clipboard history, screenshots, recent files, location,
    desktop water, the debug overlay, the thumbnail cache (Clear removes
    only the thumbnails Reprise kept and says how much went), thumbnails from
    the installed thumbnailers, and an Edit File... button for when
    you'd rather just open `config.toml`.
  - **Backup:** Export... saves your settings and remembered places in one
    file (`reprise-settings-YYYY-MM-DD.toml`); Import... puts them back, on
    this computer or another. The page lists what the file holds:
    `config.toml` with your shortcuts, Terminal's options, the Quick Launch
    buttons, where windows open, display arrangements, the file dialogs'
    folders and views, and Explorer's view settings. The programs a session
    reopens, the Run and clipboard histories and location answers stay
    behind. Import shows what it replaces, keeps the current files in
    `~/.config/reprise/reprise-settings-before-import.toml` (a file Import
    takes back), writes the new ones, and the desktop picks them up at once.
    Display arrangements only apply to the same monitors.

Restore Defaults, at the left of the OK, Cancel and Apply row, is on every
page whose controls write `config.toml` keys. It lists the page's keys that
are off their defaults and, on OK, takes them out of `config.toml`, so the
defaults apply; the other pages' unapplied changes stay as they are. It is
greyed when nothing on the page is off its default. Display, Start Menu,
Tablet and Backup keep their settings elsewhere and have none.

When `config.toml` changes while Settings is open (an editor, or a key aid's
shortcut), the controls you haven't touched show the new values; yours stay
as you left them, and OK and Apply write only what you changed.

`reprise-settings --page display` opens on a page: its name in lower case,
spaces as hyphens and `&` as `and` (`fonts-and-icons`, `start-menu`).
`--focus mouse_keys` puts the keyboard on a control there.
`reprise-settings --export FILE` and `--import FILE` do the Backup page's
work without a window; `--import` asks nothing. A
display change applies first, then asks whether to keep it, and reverts after
15 seconds without an answer.

## Volume and sound

- **The speaker icon.** One click for a popup with the master volume, a
  slider for each program with its icon, its level in percent and a speaker
  button that mutes it, the default output and input, and the track that's
  playing with its buttons. Six programs fit side by side; more scroll while
  master stays put. The wheel over a column turns it. Double-click for the
  sheet.
- **Sounds and Audio Devices.** Set the default devices, disable one you
  never use, and on the Applications tab give each program its own volume.
  Each one has its icon, so you can find the program that started shouting.
- **Volume Control.** A mixer with a balance, a volume and Mute for the
  speakers and for each program, a Recording view that picks the input, and
  Advanced Controls for bass, treble and whatever else the card offers.
  Turning the volume up or down keeps the balance you set.
- **Louder than 100 %.** With `[sounds] allow_boost = true` (Settings >
  Sounds, "Allow volume above 100%") the popup's and Volume Control's
  playback sliders, the tray wheel and the volume keys go on to 150 %, the
  part past 100 % shaded and marked on each slider, and the on-screen panel
  writes the level out. Recording levels and the sheet's sliders stay at
  100 %. Turning it off brings every output and program louder than 100 %
  back to it; the speaker icon does that, so it needs to be running.

## Network Connections

The tray icon shows the link, a "limited" badge when the internet isn't
there, and a balloon to sign in when a café wants you to. The folder lists
your adapters and VPNs, each with Status (speed, bytes, address) and Repair.
Pick a Wi-Fi network from the list, create a hotspot (its Status counts the
devices that joined it), write a WireGuard
tunnel with its own key generator, or import a VPN file. When NetworkManager
won't show a saved WireGuard key, its field is empty and keeps the key as long
as no other key is changed.

An enterprise (802.1X) network, the kind campuses and offices run, asks for
its login when you pick it: PEAP, TTLS or TLS, an anonymous identity, the
CA certificate file or "Use system CA certificates" (or "No CA certificate
is required", which warns that a rogue network could pose as yours), the
inner method (MSCHAPv2, PAP or GTC), username and password, or for TLS your
certificate, private key and its password. With a CA, the Domain row is
required: the server's certificate must name that domain or a host under
it, so a stranger holding another certificate from the same CA is refused.
The login is saved in the network's NetworkManager profile, so
the next join needs nothing typed. WEP networks still need NetworkManager's
own editor.

The last row of the wireless list, "Connect to a hidden network...", joins
a network that doesn't broadcast its name: type the name, pick Open,
WPA-PSK or WPA3-SAE (and type the key) or Enterprise (which goes on to the
login above). Its profile is marked hidden so NetworkManager looks for it by name.

## Bluetooth Devices

Pair with the Add Bluetooth Device Wizard, connect and disconnect, rename a
device, see its battery, and turn Bluetooth or discoverability on and off.
Pairing requests from a device show up as a prompt.

The tray icon's menu lists the connected devices with their battery
("Headphones — 80%"), and a click on one opens its Properties; its tooltip
lists them the same way. The battery is what BlueZ reports as `Battery1`:
some headsets report it only with bluetoothd's experimental features on. When
a connected device's battery drops to `[bluetooth] battery_warning` (15 % by
default, set on the Options tab; 0 turns it off), a balloon with the device's
icon says so once; it warns again after the battery charges 5 points past
that. `REPRISE_BLUEZ_BUS=session` reads BlueZ off the session bus, for a
stand-in service in a test.

## Power Options

Power profile, monitor and standby timeouts on mains and on battery, low and
critical battery alarms, a Power Meter for each battery, and what the lid,
power button and sleep button do. The battery icon's menu switches profile.
While the battery charge limit holds a plugged-in battery, the icon's tooltip
and the Power Meter say "Charging limited to 80%" (or wherever the limit
stops).

## Printers

The printer folder, a queue for each printer, and the Add Printer Wizard,
which finds printers on USB and the network or takes an address, and picks a
driver by maker and model. Network printers come from avahi-daemon when it
runs, so an account outside CUPS's admin group meets the password prompt only
at Finish, or at Search Again, which looks for USB printers too. Properties covers sharing, ports, paper size,
two-sided and colour. A tray icon shows up while something prints and tells
you when it's done, or when it isn't. A document that failed stays in its
queue, counted in the folder and flagged on the tray icon, until you restart
or cancel it.

## Task Manager

Ctrl+Shift+Escape, or Task Manager on the taskbar's menu, opens it (`task_manager` picks
another program).

- **Applications:** each program started from the Start menu, the launcher or a
  desktop icon, with its icon and name. End Task asks the program and its
  helpers to close; one still running a few seconds later gets an End Now
  prompt. Switch To (or a double click) brings its window up.
- **Processes:** your processes with their PID, user, share of the CPU and
  resident memory; click a column title to sort by it, rest the pointer on a
  row for its command line. Show processes from all users lists everyone's,
  read only. End Process asks first. View > Select Columns... adds Threads,
  Disk (bytes read from and written to storage per second, blank for other
  users' processes) and Command Line (a process with no arguments, such as a
  kernel thread, shows its name in brackets). Drag a divider to widen a column.
- **Performance:** the CPU's load and the memory in use, now and over the
  last hundred samples, swap drawn over memory. View > CPU History draws a
  graph per core. Totals, memory and swap figures sit underneath.
- **Updates** every second; View > Update Speed sets High (half a second),
  Normal, Low (four seconds) or Paused, and F5 refreshes at once. Nothing is
  sampled while the window is minimized.
- The update speed, the CPU graph and the Processes columns and their widths
  are kept in `~/.local/state/reprise/taskmgr.toml`.

## User Accounts

Change your name, picture and password. With admin rights, add and remove
accounts, choosing what happens to their files. A new account can be told to
pick its own password at first login.

## Software Updates

Checks for system, Flatpak and firmware updates after login and every six
hours, and again when a Flatpak installation changes, a firmware device is
plugged in or out, or new firmware metadata arrives. Lists them by source, all
ticked; untick what you'd rather skip and click Install. Where fetching firmware
metadata takes a password, the checks on their own skip it and Check Now asks
for it. When a device asks for
something mid-update (unplug it, press a button), the window says what; with
the window closed, a balloon does. It says when a restart or a log off is due.
Closing the window doesn't stop an install halfway, and nothing installs on its
own.

It also notices a system updated from elsewhere, such as `pacman -Syu` in a
terminal: once the running kernel's modules folder under `/usr/lib/modules` is
gone, or a package leaves `/run/reboot-required`, a balloon says "Restart to
finish updating" with Restart Now and Later, once a boot, and the tray icon and
the window say it until the restart. Logging in after such an update and before
restarting notices it too. Restart Now goes the way Start's Restart does:
programs are asked to close first, so they can ask about unsaved work, and the
prompt listing what is left can still cancel it. Settings > Startup turns it
off (`[updates] restart_notice`).

## Disk Usage

What takes a folder's or a drive's space: its folders and files, largest
first, each with its size, a bar for its share of the folder and how many
files it holds. Sizes fill in as the count runs in the background; they are
what the files take on disk, a hard-linked file counted once. Links are not
followed and other drives mounted inside are not entered, so a drive that
has stopped answering holds nothing up. Double-click a folder to go into it;
Back, Up and Refresh do what they say, and Open in Explorer opens the folder
shown. An entry's right-click menu adds Open in Explorer and Move to Recycle
Bin, which asks first as Explorer's Delete does. Open it from Explorer, from
the low disk space balloon, or as `reprise-diskusage FOLDER`.

## Font Viewer

The default for font files (`.ttf`, `.otf`, `.ttc`, `.woff`, `.woff2`):
double-click one in Explorer or on the desktop. It shows the face's name,
version and copyright, the alphabet, digits and punctuation, and a pangram at
12, 18, 24, 36, 48 and 72 px, set from the file itself. A collection (`.ttc`)
picks its face from a list. Install copies the file into your fonts folder
(`$XDG_DATA_HOME/fonts`, usually `~/.local/share/fonts`) and runs `fc-cache`
on it, so other programs' font lists show it on their next start. A face
already there by its PostScript name says Installed; one under a system
fonts folder says Installed (system).

## Date and Time Properties

Double-click the clock. A month grid, an analog clock, the time zone and
network time.

## Smaller things

- **Safely Remove Hardware.** Handles drives as you plug them in, whatever
  their file system, unless UDisks2 marks them hidden or internal, and shows a
  tray icon while there's something to eject. What a plugged-in drive does is
  `[drives] on_insert`, set on Settings > Default Programs:
  - `"ask"` (the default): a "New drive" balloon with Mount and Open and
    Mount buttons (Unlock and Open and Unlock for an encrypted drive, which
    then asks for its passphrase). Nothing mounts until you pick one; a
    dismissed balloon leaves the drive in Computer to open from there.
  - `"mount"`: mounts it (asking an encrypted drive's passphrase first) and
    shows a balloon you can click to open it.
  - `"open"`: mounts it and opens a window on it.
  - `"nothing"`: leaves it in Computer.

  The key is read as each drive arrives, so a change applies to the next one.
- **Phone.** With KDE Connect installed, a phone icon shows while
  `kdeconnectd` runs. Its tooltip and menu list each paired phone in reach
  with its battery and whether it's charging; the menu rings one (Ring phone,
  loud even on silent), sends it files picked in an Open dialog (Send
  files...), and opens KDE Connect, where pairing happens (Open KDE Connect
  settings, also a left click). With no phone in reach the menu says "No
  devices". Pair a phone once in KDE Connect and Send To in Explorer and on
  the desktop lists it too. Low battery warnings come from KDE Connect itself.
- **Low disk space.** The same program reads how full the disks of your home
  folder and `/` are every two minutes. When one has less free than
  `[storage] low_space_percent` of its size (5 % by default, set on
  Settings > Power; 0 turns it off) or than 1 GiB (a tenth of a disk under
  10 GiB), a balloon says so once, with Open, which opens the disk, and
  Empty Recycle Bin while the bin holds something, which asks first as the
  bin's own menu does, or else Show Usage, which opens Disk Usage on it. It
  warns again after the disk climbs a point past the
  limit and falls back.
  Nothing happens on its own while the session is locked, and drives already
  plugged in when the session starts are left as they are.
- **Run As.** The admin password prompt. It takes a fingerprint too, when
  polkit is set up for one. Cancelling it in User Accounts, Network
  Connections, Printers, Software Updates, Date and Time, Explorer or Safely
  Remove Hardware leaves things as they were, with no error; a refused
  password says so. systemd's services wait 25 s for the password: in Date
  and Time a prompt left open longer closes, and the program says to try
  again.
- **About Reprise.** Version, operating system, kernel, model, processor,
  graphics, memory (in kilobytes, total and available), disk, displays and
  uptime, with a Copy button for bug reports. The programs' own Help > About
  boxes give your memory in kilobytes too.
