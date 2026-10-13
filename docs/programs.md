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
- **Places:** Desktop, Documents, Home, Computer and the Recycle Bin.
  Computer lists your drives, unmounted and locked removable ones included:
  open one to mount it (a locked one asks for its passphrase first), Eject to
  remove it safely. Network and FUSE mounts stay out of
  that list, so a slow server can't stall it.
- **File operations** run in the background, so a big copy doesn't freeze
  anything. Cut, Copy, Paste (with Yes to All), Paste Shortcut, New Folder,
  rename with F2, Move To, Copy To, Open With, and Send To a zip file, the
  desktop, a mail or a removable drive. Ctrl+Z undoes the last one.
- **Drag and drop** between Explorer windows, the desktop and other
  programs, Flatpaks included. A right-drag asks whether to copy, move or
  make a shortcut. Drop a link from a browser and you get a link file.
- **Recycle Bin** on the freedesktop trash, shared with every other Linux
  program, with Restore and the original location of everything in it.
- **Properties** (Alt+Enter): size, dates, Read-only and Hidden, a Summary
  tab whose title, author and keywords live in the file's extended
  attributes, a Shortcut tab for launchers and a Change Icon for folders.
  Select several files for one sheet that covers them all.
- **Thumbnails** decode in the background into the freedesktop thumbnail
  cache, shared with other file managers. The Browse... dialogs of Run and
  Open With read the same cache and leave the pictures it lacks to Explorer,
  so the desktop itself never decodes a picture. Folders update by
  themselves, no F5 needed.

## Notepad

The default for plain text. New, Open, Save, Find (F3 for the next one),
Replace, Go To and Time/Date (F5), with Ln and Col in the status bar. It keeps
CRLF line endings, never leaves a file half saved, asks before opening a
file that isn't UTF-8, and grows with Larger Text. Word Wrap and Print are
greyed out for now. Notepad has always had its priorities.

## Reprise Settings

One window, 23 pages, four headings. Only one runs at a time.

- **Look**
  - **Appearance:** interface and colour scheme. Under Glass: mode,
    material, frame, transparency, blur, reflections and colour intensity.
  - **Background:** a picture or a slideshow, its position, the desktop
    colour and which icons sit on the desktop.
  - **Screen Saver:** the saver, its settings and a live preview.
  - **Display:** arrange monitors by dragging them, or pick one with the
    arrows and move it with Ctrl+arrows to the next edge that way, Identify,
    the primary screen, and each screen's resolution, refresh rate, orientation, scale
    and adaptive sync.
  - **Night Light:** warmth, and sunset to sunrise or a schedule.
  - **Fonts & Icons:** the UI and dialog fonts, the icon theme, frame and
    caption sizes, and how GTK and Qt programs are themed.
  - **Effects:** window animations, highlight fades, menu shadows and fades,
    and taskbar slides.
- **Desktop**
  - **Taskbar:** position, button mode, pinned programs, Peek, Show desktop,
    tray overflow, auto-hide, and the Start button and menu style.
  - **Start Menu:** the Start menu tree and the Quick Launch buttons.
  - **Launcher:** whether it searches your files.
  - **Window Switcher:** Alt+Tab's icons or thumbnails, picture size, Peek,
    the 3D switcher and the overview's hot corner.
  - **Desktops:** how many, how they animate, and the taskbar's live
    previews.
  - **Windows:** snapping, snap layouts, the snap bar, window tabs and window
    memory.
- **Input**
  - **Keyboard:** layouts, repeat, and every shortcut, programs' global
    shortcuts included.
  - **Mouse:** pointer speed, double-click speed, scrolling, and the
    touchpad's tapping, scrolling and disable-while-typing.
  - **Tablet:** what each pad button, pad key, ring, strip and dial does,
    and how far a ring or strip turns per step.
  - **Pointers:** cursor theme and size.
  - **Accessibility:** text size, StickyKeys, SlowKeys and MouseKeys.
  - **Vision:** colour filters and the magnifier.
- **System**
  - **Power:** the Turn Off commands, idle timeouts, lock on sleep and the
    lock screen's wallpaper.
  - **Startup:** autostart entries and session restore.
  - **Default Programs:** browser, mail, music, video, photos, text editor,
    file manager, terminal, what a plugged-in drive does, and the program
    for every file type.
  - **Advanced:** clipboard history, screenshots, recent files, location,
    desktop water, the debug overlay, and an Edit File... button for when
    you'd rather just open `config.toml`.

When `config.toml` changes while Settings is open (an editor, or a key aid's
shortcut), the controls you haven't touched show the new values; yours stay
as you left them, and OK and Apply write only what you changed.

`reprise-settings --page display` opens on a page: its name in lower case,
spaces as hyphens and `&` as `and` (`fonts-and-icons`, `start-menu`).
`--focus mouse_keys` puts the keyboard on a control there. A
display change applies first, then asks whether to keep it, and reverts after
15 seconds without an answer.

## Volume and sound

- **The speaker icon.** One click for a popup with the master volume, a
  slider for each program, the default output and input, and the track
  that's playing with its buttons. Double-click for the sheet.
- **Sounds and Audio Devices.** Set the default devices, disable one you
  never use, and on the Applications tab give each program its own volume.
  Each one has its icon, so you can find the program that started shouting.
- **Volume Control.** A mixer with a balance, a volume and Mute for the
  speakers and for each program, a Recording view that picks the input, and
  Advanced Controls for bass, treble and whatever else the card offers.
  Turning the volume up or down keeps the balance you set.

## Network Connections

The tray icon shows the link, a "limited" badge when the internet isn't
there, and a balloon to sign in when a café wants you to. The folder lists
your adapters and VPNs, each with Status (speed, bytes, address) and Repair.
Pick a Wi-Fi network from the list, create a hotspot (its Status counts the
devices that joined it), write a WireGuard
tunnel with its own key generator, or import a VPN file. When NetworkManager
won't show a saved WireGuard key, its field is empty and keeps the key as long
as no other key is changed.

## Bluetooth Devices

Pair with the Add Bluetooth Device Wizard, connect and disconnect, rename a
device, see its battery, and turn Bluetooth or discoverability on and off.
Pairing requests from a device show up as a prompt.

## Power Options

Power profile, monitor and standby timeouts on mains and on battery, low and
critical battery alarms, a Power Meter for each battery, and what the lid,
power button and sleep button do. The battery icon's menu switches profile.

## Printers

The printer folder, a queue for each printer, and the Add Printer Wizard,
which finds printers on USB and the network or takes an address, and picks a
driver by maker and model. Network printers come from avahi-daemon when it
runs, so an account outside CUPS's admin group meets the password prompt only
at Finish, or at Search Again, which looks for USB printers too. Properties covers sharing, ports, paper size,
two-sided and colour. A tray icon shows up while something prints and tells
you when it's done, or when it isn't.

## User Accounts

Change your name, picture and password. With admin rights, add and remove
accounts, choosing what happens to their files. A new account can be told to
pick its own password at first login.

## Software Updates

Checks for system, Flatpak and firmware updates after login and every six
hours, and again when a Flatpak installation changes, a firmware device is
plugged in or out, or new firmware metadata arrives. Lists them by source, all
ticked; untick what you'd rather skip and click Install. When a device asks for
something mid-update (unplug it, press a button), the window says what; with
the window closed, a balloon does. It says when a restart or a log off is due.
Closing the window doesn't stop an install halfway, and nothing installs on its
own.

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
