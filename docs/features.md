# Features

Everything Reprise does, grouped by where you'd go looking for it. The
programs that come with it have [their own page](programs.md), and the looks
have [theirs](interfaces.md). Shortcuts below are the defaults; every one but
the Super tap, Super+Shift+Esc and the VT switch keys can be rebound.

## Taskbar

- **Any edge.** `[taskbar] position` puts the bar on any edge. Unlocked, drag
  empty bar space to another edge, or a side bar's inner edge to widen it.
- **Auto-hide.** The bar slides down to a 2 px line and comes back on the
  pointer, the Start key, Alt+Tab or an attention request.
- **Four button modes.** Icons only, labelled and grouped when the row is
  full, never grouped, or kept side by side per program.
- **Pinned programs.** Quick Launch can lead the task row: a pin starts its
  program and becomes its button once it runs. Drop a program on the row to pin it.
- **A bar per monitor.** Every monitor has its own taskbar. With
  `[taskbar] per_monitor_buttons` (a check box on Settings > Taskbar), each
  bar lists only the windows on its monitor (the one holding a window's
  centre), and a button follows its window across. Pins and the tray stay on every bar, and Alt+Tab lists
  every window.
- **Quick Launch overflow.** A Quick Launch band dragged shorter than its
  buttons puts the rest behind a » at its end.
- **Jump lists.** With `[taskbar] pinned` on (Glass's default), right-click a
  task button for the program's jump list: its recent files and its tasks
  (the actions its desktop entry offers, such as New Window) under Recent and
  Tasks headings, its windows, Pin or Unpin, Move to desktop and Close.
  Shift+right-click opens the window menu instead. With `pinned` off (Classic's
  and Blue's default), right-click opens the window menu, or a group's window
  list, with a Recent Files submenu when the program has recent files.
- **Drag to reorder.** Drag task buttons along the row. Middle-click starts
  another copy of the program.
- **Live previews.** Point at a task button for live pictures of its windows.
  Rest on one, or on Show desktop, and the other windows fade to their frames.
  A window whose program plays media (MPRIS) gets Previous, Play or Pause, and
  Next under its picture; Tab reaches them from the keyboard.
- **Attention.** A window that asks for attention flashes its task button
  for 30 s (`[taskbar] attention_flash`), then stays lit until you focus it.
- **Progress and badges.** A program that reports progress or an unread count
  (LauncherEntry) fills its task button or badges it (`[taskbar] badges`).
- **Spring-loaded buttons.** Hold a dragged file over a task button for half
  a second and its window comes up for the drop.
- **Cascade and tile.** The taskbar's menu cascades or tiles the windows,
  opens Task Manager (also Ctrl+Shift+Escape) or locks the bar.
- **Tray overflow.** Quiet tray icons wait behind an arrow, or a ^ flyout in
  Glass. Customize... sets each icon to show, hide or follow its status.
- **Calendar.** Click the clock for the month and, with
  evolution-data-server, today's events (`[taskbar] calendar_events`).
  Double-click it for Date and Time Properties.
- **From the keyboard.** Super+T lands on Start, Super+B on the tray. Super+1
  to Super+9 act on that task button; Super+D shows the desktop. A task button
  the arrows land on shows its live previews. Enter on a group opens them where
  a click would cycle its windows (`[taskbar] pinned`): the arrows pick one,
  Enter brings it up, Delete closes it, Escape goes back to the button.

## Start menu and launcher

- **Start.** Super, Ctrl+Esc or the Start button. Classic cascades, Blue and
  Glass have two columns; `startmenu_style` picks any of them under any look.
- **Your menu.** The Start Menu page of Reprise Settings edits the Classic
  menu's rows and Quick Launch, a folder of `.desktop` files in
  `~/.config/reprise/quicklaunch/`. It saves only your changes to the default
  menu, so the rows you leave alone follow each new Reprise's defaults. A menu
  saved whole as `[[startmenu]]` (by an older Reprise, or by hand) keeps
  winning and stays whole; Restore Defaults on the page switches it to
  changes, at the cost of its edits. The Blue and Glass menus don't show
  these rows: their left column holds the programs you pin with Pin to Start
  Menu and the ones you use most, their right column a fixed set of places.
  A right-click on the Start button opens your programs folder
  (`~/.local/share/applications`) or, under Properties, that page. In the
  Blue and Glass menus a right-click on your home folder, Documents, Pictures
  or Music offers Open, Explore and Properties, and on Computer Open and
  Properties (About Reprise).
- **Live program list.** A program shows up in the Start menu and the
  launcher the moment it's installed.
- **Recent documents** from `recently-used.xbel`, shared with GTK programs.
  Reprise's programs add the files they open and save.
  `[privacy] recent_files = false` hides them and records none.
- **Launcher.** Super+Space finds programs, settings ("repeat delay" opens
  its page on that control), files in your home folder and open windows.
  Glass Start's search finds the same; a settings row found by a search
  word greys that word after the page's name ("Advanced  low disk space").
- **Sums and units.** Type `2+3*4` or `5 km in mi` in the launcher and Enter
  copies the result.
- **File search** uses plocate's database, or `fd` without one, so nothing
  crawls your disk in the background. With neither, the launcher's Files
  section and Explorer's status bar say "File search needs plocate or fd."
- **Search by contents.** Settings > Search > "Find files by their contents"
  (`[launcher] contents`, off by default) adds localsearch's results.
- **Search providers.** Programs that answer the GNOME search provider
  interface add a section each, set on Settings > Search.
- **A program's menu** in any Start menu or the launcher also offers Open
  file location (Explorer on its `.desktop` file) and Add to desktop, a
  copy allowed to launch; the row greys while the desktop holds one.
- **Uninstall.** Uninstall... on a program's menu removes a Flatpak or,
  through PackageKit, a package, after a sheet says what goes.
- **Web apps.** A browser's "Install as app" asks first with the icon and
  name, then lands in All Programs.
- **Run...** for when you know exactly what you want. A command runs, a path
  opens, and Browse... picks an executable.

## Windows

- **Frames that work.** Minimize, maximize and close. A caption double-click
  does what `[windows] title_double_click` says. Super and a drag moves a window.
- **Window menu.** Alt+Space or a caption right-click, with Roll Up, Always
  on Top, Always Open Here, Mini View, Move to Desktop and the tab commands.
- **Always Open Here.** Ticked in the window menu, it writes a window rule
  that opens the program's windows on this screen, at this place and size.
- **Roll up.** Leaves only the title bar while the program keeps its size.
- **Mini view.** Mini View in the window menu (or a `mini-view` bind) opens a
  small live picture of the window that stays above every window. One on an
  unplugged screen moves to the screen under the pointer.
- **Window tabs.** Hold a window's caption over another's and they share one
  frame. Super+\` and Super+Page Down or Up step through the tabs.
- **Shake.** Shake a window by its caption and the other windows on that
  screen minimize; shake again to bring them back (`[windows] shake`).
- **Dim inactive windows.** Every window but the focused one darkens by
  `[windows] dim_inactive` percent (0 to 40, off by default).
- **Hidden frame rate.** `[windows] hidden_frame_rate` caps a vsynced game's
  frame rate while no screen shows its window.
- **Window memory.** A program opens where its last window closed, per set
  of connected screens (`[windows] remember`).
- **Window rules** match windows by app id and title and pin their size,
  place, screen, desktop, state or tab group. Settings > Window Rules lists
  them in order and adds, edits, moves and removes them; an Always Open Here
  rule edited there is yours, and the window menu leaves it alone. A browser's
  picture-in-picture video stays on top on every desktop unless a rule says
  otherwise.
- **Small conveniences.** Super+C centres a window, Super+Shift+B hides its
  frame, and Esc during a caption drag puts it back.
- **End Program.** Reprise pings windows, so closing one that stopped
  answering offers End Program at once.
- **Modal dialogs that stay modal.** Focus can't land behind a dialog a
  program marked modal, and no window of its family (its parent, the
  parent's other dialogs) comes up over it. A job's progress box stays live
  beside a box over its parent: Pause, Cancel and its X still work.
- **No focus stealing.** A new window opens behind the one you're typing in
  unless it has a reason to take the keyboard ([security.md](security.md#focus)).
- **Motion.** Windows fade and grow as they open, menus fade or slide
  (`[effects]`, on by default under Glass).
- **Menus past the window's edge.** Menus in Reprise's own programs open on
  popups of their own, so they show whole beside a small window.

The Window Behavior page of Reprise Settings sets the caption double-click,
shake, the dim, the hidden frame rate and window memory.

## Snapping

- **Edges and corners.** Drag a window to a side to fill it, to the top to
  maximize, into a corner for a quarter (`[window_snapping] distance`).
- **Snap layouts.** Rest on a maximize button, or press Super+Z, for six
  ways to split the screen. `[window_snapping] layouts` sets your own.
- **The snap bar.** Drag a window toward the top of the screen and the
  layouts drop down. Hold it over a zone and let go.
- **Suggestions.** The layouts start with your last few arrangements on that
  screen; picking one puts every window back in its zone.
- **Snap assist.** After a snap, the zones left over offer your other
  windows, live.
- **Snap groups.** Drag the edge between two snapped windows and both
  resize. Each desktop keeps its own layout.
- **From the keyboard.** Super and the arrows step a window through the
  zones and on to the next screen.
- **To another screen.** Super+Shift+Left or Right moves a window to the
  screen before or after its own, keeping its place and size in proportion;
  a maximized or snapped window stays maximized or snapped there.

![Snap assist offering Explorer for the free half after Notepad snapped left](screenshots/snap-assist.png)

## Switching

- **Alt+Tab.** A grid of icons with a live picture of the selected window,
  or live pictures of every window, in four sizes.
- **Alt+\`** (the key above Tab on any layout) switches between the focused
  program's windows alone.
- **3D switcher.** Glass only: Super+Tab tilts every window into a stack. See
  [interfaces](interfaces.md#glass-only).
- **Window overview.** Super+W, a hot corner (`[overview] hot_corner`) or
  three fingers up shows every window of the desktop side by side.
- **Virtual desktops.** Four by default, up to sixteen, with a pager on the
  taskbar. Super+Ctrl+Left or Right switches (`[workspaces]`).

## Desktop

- **Icons** for Documents, Computer, Home, Recycle Bin and the files in
  `~/Desktop`, to drag, rename and right-click (`desktop_extensions`).
  New selects what it makes and starts its rename. The desktop and every
  Explorer window share one undo history: Ctrl+Z in any of them undoes the
  last New, rename, copy, move or delete made in any of them. On the desktop,
  undoing anything but a desktop New or rename runs in Explorer, with its
  progress and Cancel.
- **Templates.** New on the desktop and in Explorer lists the files in your
  Templates folder (`XDG_TEMPLATES_DIR`, usually `~/Templates`); each row
  makes a copy named after its file.
- **Open in Terminal** on the desktop's and a folder's right-click menu
  starts your `terminal` there.
- **Widgets.** A clock, a calendar and a system meter, ticked under the
  desktop menu's Widgets or Settings > Background (`desktop_widgets`).
- **Trusted launchers.** A `.desktop` file runs only once you mark it
  trusted (Allow Launching), so a download can't pose as a program.
- **Programs and scripts.** A file with its Executable box ticked runs when
  opened, asking the first time (`confirm_scripts`, `script_open`).
- **File dialogs remember** their folder, view, sort and size per program,
  and the place you last dragged one to while it is on the program
  window's display; otherwise one opens over the program's window. They
  list changes to the folder as they happen.
- **File dialog keys and menu.** F5, Ctrl+H, Alt+arrows, Ctrl+Shift+N, F2
  and Delete, and a right-click menu with Pin to Places, Rename and Delete.
- **Pinned folders.** Pin to Places on a folder's menu adds it to every file
  dialog's places bar and Look in list, greyed while the folder is not there.
  They are GTK's bookmarks, so GTK programs share them, and Explorer's
  Favorites menu lists them.
- **Band select.** In an Open dialog that takes several files, drag from the
  background to select the entries a band touches.
- **Wallpaper** filled, fitted, stretched, tiled or centred, or a slideshow
  of a folder. Each virtual desktop and each monitor can have its own picture.
- **Desktop water.** The wallpaper ripples like a pond. Entirely
  unnecessary. Very relaxing. `ink`, `glow`, `sparks` and `neon` stir like fluid.
- **Screen savers.** Blank, a starfield, bouncing lines, a bouncing logo and
  your photos.

## Clipboard, emoji and screenshots

- **Clipboard history.** Super+V lists the last 25 copies, text and images,
  in memory only. Password managers' copies are left out.
- **The last copy stays** on the clipboard after the program that made it
  quits. Clipboard managers and middle-click paste work.
- **Emoji picker.** Super+. opens a grid of emoji at the caret. Search by
  name or shortcode; Enter types it. It wants a colour emoji font.
- **Colour emoji in text.** Emoji draw in colour from that font wherever
  Reprise draws text: titles, the taskbar, Notepad, Terminal and every field.
  Skin tones, families, flags and keycaps draw as one emoji. Greyed labels
  and menu items draw an emoji in their grey.
- **Path completion.** Typing a path in Run, Explorer's address bar or a file
  dialog completes it inline and lists the matches.
- **Pastes keep their place.** In Reprise's programs, keys and clicks after
  a paste wait until it is in, so Ctrl+V then Ctrl+O pastes, then opens, and
  keys typed after a key that opens a box, such as Ctrl+S, go to that box.
  They wait at most 5 seconds; a slower paste still lands when it arrives.
- **Edit menu.** A right-click, Shift+F10 or the Menu key in Reprise's text
  fields opens Undo, Redo, Cut, Copy, Paste, Delete and Select All.
- **Screenshots.** PrtSc for the screen, Alt+PrtSc for the window,
  Ctrl+Alt+PrtSc for its content, Shift+PrtSc or Super+Shift+S for a region
  or a window.
- **Screenshot delay.** `[screenshot] delay` (0 to 10 seconds, Settings >
  Screen Capture) waits before the picture, so a menu or a tooltip can be
  opened first; Shift+PrtSc waits after the pick. A screenshot key pressed
  while one waits cancels it, and so does locking the desktop. An app's
  interactive screenshot dialog has its own Delay box, which starts at the
  same value; an app's other screenshots never wait.
- **Screenshot files.** Each shot goes to the clipboard and, if you like, to
  Pictures/Screenshots as PNG, JPG or JPEG XL (`[screenshot] format`). The
  saved notice shows the picture and opens it on a click; its Edit opens it in
  Paint and Show in Folder selects it. The file joins the recent documents.
- **Screen recordings.** Ctrl+Shift+Alt+R records the screen under the
  pointer and stops it. V in the Shift+PrtSc picker records an area instead.
- **Recording files** are MP4s in Videos/Screencasts, in H.264, H.265 or AV1
  (`[recording]`, [configuration.md](configuration.md#recording)).
- **Settings > Screen Capture** sets the screenshot format and the
  recording's codecs, rates and sound.
- **Microphone in recordings.** With "Record what plays" and "Record the
  microphone" both on, a recording's sound has the default microphone mixed
  in, and the taskbar's microphone shows from its first frame until Stop or
  until the microphone goes away (`[screenshot] record_microphone`). A
  recording running when the desktop locks keeps hearing it, and a change to
  either box applies from the next recording. A microphone that goes away
  mid-recording, or isn't there at the start, leaves the recording running
  with what plays, and its saved notice says the microphone was disconnected,
  stopped working or wasn't found. A microphone that comes back later stays
  out of that recording.
- **Colour picker.** Super+Shift+C shows a loupe at the pointer; a click or
  Enter copies the colour as `#rrggbb`.

## Notifications

- **Balloons** in the corner by the tray, with the program's buttons. A chat
  program's Reply opens a text field in the balloon. A notification's picture
  (a photo, a screenshot) shows under its text, its shape kept.
- **History.** A bell in the tray keeps the session's notifications, grouped
  per program, each with its body in grey; a click on one runs its default
  action and closes the history. Super+Shift+N opens it from the keyboard, on the monitor with the
  pointer.
- **Bursts fold together.** A program that sends more than three balloons in
  ten seconds gets one "5 new from Mail" balloon instead.
- **Per program.** Don't Show Balloons and No Sounds, on a program's menu in
  the history or Settings > Notifications.
- **Do Not Disturb** holds back all but critical notifications, by hand,
  during `quiet_hours`, or over full-screen windows and screen shares.

We had notifications in 2001. They were allowed a small corner. They still
are.

## Lock, power and session

- **Lock screen** with a password or a fingerprint, whichever comes first.
  `[power] locker` runs another locker, with Reprise's lock as a fallback.
- **The lock is the login screen, locked.** Your tile says Locked and takes
  your password. Its power button has Stand By, and Restart and Turn Off
  under reprise-login.
- **Lock and unlock in motion.** On Blue and Glass the lock comes in and
  fades out (`[effects] logon_animation`).
- **Idle steps.** Screen saver, lock, screen off and suspend, each with its
  own timeout, and separate timeouts on battery.
- **Lock on sleep.** The screen locks before the machine suspends, whatever
  asked it to.
- **Lock when away.** Pick your phone on the Screen Saver & Lock page and the
  session locks once it's gone ([configuration.md](configuration.md#lock-when-away)).
- **Laptop things.** Lid and button actions, battery alarms, power profiles,
  and clamshell mode with a monitor plugged in.
- **Power saver on low battery.** At the low battery alarm Reprise switches
  to the power saver profile until you plug in (`[power] low_battery_saver`).
- **Battery charge limit.** Power Options' Power Meter tab turns UPower's
  charge limit on or off, where the battery driver supports one.
- **A Turn Off dialog** that dims the desktop behind it. Shutting down used
  to have a little gravity. Hold Shift and Stand By turns into Hibernate.
- **Restart into another system.** Where the boot loader lets logind pick the
  next boot (systemd-boot does), the Turn Off dialog's Restart Into... and
  Glass's Restart into list its boot entries, and Firmware Setup when the
  firmware has one. A pick restarts into it once, the same way Restart does.
  An entry shows the title from its file on the boot partition when your
  account may read it, else its id. A one-shot target logind still holds,
  whoever set it, is cleared before the next Turn Off, Restart or Hibernate,
  and a pick clears one of the other kind first; when it won't clear, a box
  asks whether to go ahead anyway. A Hibernate from the lid, idle or a power
  key clears it without asking and goes ahead. Stand By leaves it.
- **Save prompts.** Log Off, Turn Off and Restart ask every window to close
  and wait, then list who's holding things up, with End Now and Cancel. A
  program holding off log off through the Inhibit portal (a copy under way)
  keeps its windows until it lets go.
- **Install updates on the way out.** While PackageKit has an offline update
  prepared (by a software center, or `pkcon update --only-download`), the
  Turn Off dialog has an Install updates check box and Glass's Shut down
  options list Install updates and shut down and Install updates and restart.
  The box is checked when the dialog opens with it; one that shows up later,
  once PackageKit answers, starts unchecked. Turn Off or Restart with it
  checked sets the update to install just before the computer goes down, the
  Turn Off through `[power] poweroff`; it installs the next time the computer
  starts, before anything else, then the computer restarts into the desktop.
  Where polkit wants a password for that, it asks before anything closes.
  Cancel a save prompt and nothing is set. When the action fails, or Log Off
  or a critical battery takes over while it waits, the update is taken back,
  unless the computer is shutting down by then; when it stays, the message
  says it installs at the next start. An update
  gone by the time the computer goes down leaves a plain Turn Off or Restart.
  Restart Into... is greyed while the box is checked, and an update never
  installs into a boot logind's restart target sends elsewhere: a target that
  won't clear asks whether to go ahead without the update. An update another
  program set to install is taken back only when you untick the box, before
  Turn Off, Restart or Restart Into...; the power key, Glass's plain Shut down
  and Restart, and a box you never touched leave it set, and a checked box
  leaves what that program set the computer to do afterwards. When it won't
  be taken back, a box asks whether to go ahead anyway.
- **Session restore.** The programs you left open start again at the next
  login, on their desktops, with their tab and snap groups (`[session] restore`).
  Notepad comes back with each file it had open, Explorer with each window's
  tabs in their order, each at its folder and view (a network server's at the
  Network list), on the tab it showed, and Settings on its page. An Untitled Notepad comes back empty, as unsaved text is not kept; a
  file gone by then comes back as an empty document under its name, written
  only if you save it, and a folder gone by then opens Home. Other programs
  start once each, as their Start menu entry starts them, without the files
  they had open.
- **XDG autostart,** with a Startup page in Reprise Settings to turn entries
  on and off.
- **Language and formats per account.** Settings > Region & Language sets
  them for you or the system ([programs.md](programs.md#reprise-settings)).
- **Crash recovery.** When Reprise panics it starts again in place, locked if
  it was, and session restore brings the programs back.

## Login screen

Reprise brings its own login manager, `reprise-login`. It's installed
switched off; [install.md](install.md#login-manager) says how to switch to
it and back. `[login] interface` in `/etc/reprise/login.toml` picks the
design ([configuration.md](configuration.md#login-manager)), and without it
the screen takes the interface of the last session used at this computer.

- **Classic** is a Log On to Reprise dialog on the desktop colour.
- **Blue** is a welcome screen between two dark bands with a column of user
  tiles ("To begin, click your user name").
- **Glass** shows the last user's picture over the wallpaper with a password
  field, the other users one click away.
- **Switching users.** Switch User brings the login screen back while your
  session runs on, locked. A second login of a user resumes their session.
- **Shut Down and Restart** from the login screen, with an administrator's
  password while anyone is logged on.
- **PAM's messages.** A wrong name or password gets one fixed message; what
  PAM says otherwise (a locked account, `/run/nologin`) shows in its words.
- **Motion.** On Glass a picked tile's picture flies into place; Blue
  crossfades (`[login] animation`).
- **A screen reader.** Super+Alt+S starts Orca on the login screen and
  stops it; logging in stops it too. Without Orca the keys do nothing.
- **Caps Lock and layouts.** "Caps Lock is on." shows under the password,
  and a badge shows and switches the keyboard layout.
- **Autologin.** `[login] autologin_user` logs one user in when the computer
  starts.
- **Background and screens.** Settings > Login Manager picks the picture
  behind the login screen and can give it your monitors' layout.
- **Remote logins.** Logging in here as a user connected remotely brings the
  session to this screen ([Remote logins](remote-desktop.md#remote-logins)).
- **A safety net.** A session that ends as soon as it starts brings the
  login screen back with a note; a failing login screen gives way to tty2.

## Displays

- **Multiple monitors,** each with its own mode, scale, rotation and
  adaptive sync, arranged on the Display page. An unconfirmed change reverts.
- **A layout per set of monitors.** Docking or undocking puts back the layout
  last used with those monitors. Forget saved layouts on the Display page.
- **Duplicate.** Super+P opens the display switcher: Extend, Duplicate, Only
  1 and Only 2, also on the Display page. Duplicate runs the largest size
  every monitor lists that fits inside the mode each runs on its own. Extend
  puts each monitor back in the mode, scale and place it had before, or its
  `[[outputs]]` mode.
- **Fractional scaling** from 1 to 4 in quarter steps, per monitor. Programs
  that support it draw at the exact scale. The Display page's Scale list names
  the desktop size each step leaves, and warns below 800 x 600.
- **Variable refresh rate** per monitor: off, on, or only while the focused
  window is fullscreen there (`adaptive_sync = "fullscreen"`), so a panel
  that flickers at low refresh rates stays fixed on the desktop.
- **Colour profiles.** An ICC profile per monitor loads its calibration
  curves (`[[outputs]] icc_profile`).
- **Night light** from sunset to sunrise, or on a schedule. The displays
  icon's menu pauses it for an hour or until tomorrow.
- **A displays icon** in the tray: scroll to dim or brighten every display,
  click for a slider per display.
- **Auto-rotation** on convertibles and tablets with iio-sensor-proxy, with a
  rotation lock key (`[display] auto_rotate`).
- **Explicit sync** for the drivers that need it, NVIDIA's among them.

## Input

- **Keyboard layouts,** switched with Left Alt+Shift or from the tray, or
  kept per window (`[keyboard] layout_per_window`).
- **Shortcuts on any layout.** On a non-Latin layout, Ctrl+C copies and
  Super+L locks as on the first Latin one.
- **Shortcut sheet.** Hold Super on its own for a second for a sheet of every
  shortcut in force (`[shortcut_overlay]`, the Keyboard page).
- **Input methods.** fcitx5 and ibus work in programs and in Reprise's own
  fields. Start ibus with `ibus start --type wayland`.
- **Drop text into fields.** Text dragged from any program lands in
  Reprise's text fields; dropped files give their paths.
- **Touchpad gestures.** Three fingers (or four) switch desktops and open the
  overview; other pinches and swipes go to the program (`[gestures]`, the
  Touchpad page).
- **Touchpad and mouse.** Click method, scroll speed, middle click and
  left-handed buttons (`[mouse]`, `[touchpad]`, the Mouse and Touchpad pages).
- **Touchscreens.** A program that takes touch gets touch; anywhere else a
  finger clicks and drags. Each touchscreen can be mapped to its screen.
- **Drawing tablets** with pressure, tilt and pen buttons. The pad's buttons,
  rings and strips are set per pad in Settings.
- **Global shortcuts for programs.** A voice chat's push-to-talk works while
  another window has focus. Change its keys on the Keyboard page.
- **Media keys** for volume, brightness, playback, Wi-Fi, airplane mode, the
  touchpad and the rotation lock, with an on-screen panel.
- **Caps Lock and Num Lock** show "On" or "Off" in the same panel
  (`[osd]`, Settings > Keyboard).

## Accessibility

- **Screen readers.** Reprise's programs, taskbar, menus, dialogs, lock
  and login screens and window frames speak AT-SPI. They're tested with
  Orca.
- **Screen reader on and off without looking.** Super+Alt+S starts or stops
  Orca from anywhere, the lock and login screens included.
- **Magnifier** from 1x to 8x (Super+= and Super+-) that follows the pointer,
  the caret and the keyboard focus.
- **Colour filters:** grayscale, inverted, and corrections for three kinds of
  colour blindness. Super+Ctrl+C turns them off and on.
- **Larger text** at 125% to 200% without changing the scale. Reprise and
  your GTK and Qt programs grow with it.
- **Reduce motion** (`[effects] reduce_motion`) turns the desktop's
  animations off, and tells GTK programs to stop theirs.
- **StickyKeys and SlowKeys,** offered when you press Shift five times or
  hold the right Shift for eight seconds.
- **BounceKeys** ignores a key pressed again soon after you let it go
  (`[accessibility] bounce_keys`).
- **Find the pointer:** a tap of Ctrl on its own sends rings out from the
  pointer (`[accessibility] locate_pointer`).
- **Visual alerts:** a program's bell flashes its window or the whole screen
  (`[accessibility] visual_bell`).
- **MouseKeys:** the keypad moves the pointer, clicks and drags. Left
  Alt+Left Shift+Num Lock turns it on and off.
- **Access keys** underline once you press Alt, and from the start in a
  window opened from the keyboard.

## Hardware and network

- **Sound.** A speaker icon with a volume per playing program, the default
  devices and the playing track. Volume Control adds balance, bass and treble.
- **Event sounds.** Log on and off, error and warning boxes (the shell's
  and every program's), a USB device plugged in or out, emptying the Recycle Bin, a screenshot and the charger
  play their sound from the XDG sound theme; Settings > Sound turns them off
  (`[sounds] events`).
- **Network.** Wi-Fi, wired and VPN through NetworkManager, with WireGuard
  and OpenVPN editors, Create Hotspot and a speed meter option.
- **Airplane mode** from the network icon's menu, Settings > Network or the
  airplane-mode key.
- **Proxy.** A manual proxy or a PAC address in Settings > Network, for
  programs started afterwards.
- **Bluetooth.** Pairing, connections, device batteries and receiving files,
  from the tray.
- **Phone.** Through KDE Connect: battery in the tray, Ring phone, and Send
  files.
- **Drives** mount when you plug them in, ask for a passphrase if they're
  encrypted, and eject from the tray or from Computer.
- **Phones and cameras** plugged in by USB show in Computer under Portable
  Devices.
- **Thunderbolt and USB4.** A device waiting for authorization gets a
  balloon; Settings > Thunderbolt manages them.
- **Network shares.** Explorer's File > Connect to Server..., or an `smb://`
  or `\\server\share` address, mounts a share through gvfs.
- **Disk images.** Opening an `.iso` or `.img` offers to mount it read-only.
- **Disk Usage.** What takes a folder's or drive's space, largest first
  ([programs.md](programs.md#disk-usage)).
- **Printers.** Add, share, pause and manage printers and their queues.
  Driverless printers just work.
- **Software Updates** checks system packages, Flatpaks and firmware, and
  installs only what you tick.
- **Install from a file.** Double-click a `.flatpakref`, `.flatpak`,
  `.pkg.tar.zst` or `.deb` for a sheet that installs it on Install.
- **No driver discs.** Audio runs through PipeWire, networking through
  NetworkManager, Bluetooth through BlueZ and power through UPower and logind.

## Privacy and security

- **Indicators.** A red dot for screen sharing, recording or remote input; a
  microphone, camera or pin while a program uses one. A mixer's level meter
  lights the microphone too ([why](security.md#out-of-scope-and-known-gaps)).
- **Capture border.** A red border around what is being recorded or shared.
  Settings > Privacy turns it off.
- **Prompts.** Screen sharing, remote control, location, the camera and new
  launchers ask first. Cancel or Deny is the default button.
- **Sandboxed programs get less.** Flatpaks can't reach the privileged
  protocols ([security.md](security.md#sandboxed-programs)).
- **Clipboard access.** Settings > Privacy > Clipboard Access lets a ticked
  Flatpak read the clipboard in the background.
- **Admin prompts.** A polkit agent asks for the password, or a finger, when
  a program needs root.
- **Location.** Reprise is the geoclue agent: it asks before a sandboxed
  program gets your location, and one switch turns location off.
- **Automatic time zone.** Date and Time can set the time zone from your
  location ([programs.md](programs.md#date-and-time-properties)).
- **App permissions.** Settings > Privacy > App Permissions lists the
  remembered answers. Forget one and the program asks again.

## Sharing and remote access

- **Screen sharing** in browsers, video calls and OBS, of a whole screen or
  one window.
- **Remote control.** Tools can drive the keyboard and mouse once you allow
  them. One allowed with Remember shows a balloon as it starts.
- **One keyboard and mouse across computers** with Deskflow. The volume,
  screen brightness and playback keys act on the computer the pointer is
  on. Super+Shift+Esc always takes them back.
- **VM viewers and remote desktops get your shortcuts** while they have
  focus. Super+Shift+Esc takes them back.
- **Pointer lock and raw motion** for games, 3D tools and VM viewers.
- **Send by Email** from a Flatpak opens the message in the mail program
  Settings > Default Programs names.

## Remote Desktop

Reprise has its own RDP server. Another computer can use your session, and
the session moves between this screen and the client. Under reprise-login
several users can log in remotely at once.
[remote-desktop.md](remote-desktop.md) covers turning it on, connecting, the
encoders and the settings.

## Files

Explorer has thumbnails, five views, live folders, background copies, undo,
a shared freedesktop trash, Send To and drag and drop with every other
program. Sandboxed programs get Reprise's own Open, Save and Open With
dialogs. [More in programs.md](programs.md#explorer).

- **Dead mounts.** On a mount that stops answering, Explorer and the desktop
  keep answering; an operation on it gives up after 15 s.
- **Show in folder.** A browser's "Show in folder" opens Explorer with the
  file selected.

## Configuration

- **Welcome at first login.** A new user picks the interface, the colours
  and the text size ([programs.md](programs.md#welcome)).
- **Import from GNOME or Plasma.** Wallpaper, keyboard, mouse and more,
  only the rows you check ([programs.md](programs.md#import-from-another-desktop)).
- **Live.** Edits to `config.toml` apply when you save; a broken save changes
  nothing ([configuration.md](configuration.md#a-broken-save)).
- **Comments survive.** Reprise Settings writes back into `config.toml`
  without disturbing your comments or key order.
- **Rebindable shortcuts,** modifier-only chords included.
- **Third-party tools work.** Panels, lockers, idle daemons and display
  configurators work through the usual protocols ([protocols.md](protocols.md)).
- **A debug overlay** (`[debug] overlay`) shows frame times, damage, clients
  and memory on each screen. For when you want to know why.
- **A stuck main loop leaves a trace** in the journal, and a monitor whose
  driver hangs doesn't hold up input, programs or the lock.

## Not yet

- One GPU draws and lights monitors. `[display] all_gpus = true` tracks the
  other GPUs' monitors but keeps them dark. No DRM leases for VR headsets.
- No HDR or colour management.
- X11 programs draw at scale 1 and are scaled up on HiDPI screens.
- No help viewer. The ? buttons are there for decoration.
