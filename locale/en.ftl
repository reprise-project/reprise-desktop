# Reprise desktop: the compositor and its shell. An & marks the next letter as the access key.

## Ending the session
pow-updates-not-set = The updates could not be set to install. { $why }
# The quoted title is a boot entry's.
pow-boot-entry-gone = "{ $title }" is no longer a boot entry.
pow-updates-install-later = The updates will install another time.
# The action is a phrase such as "restart the computer".
pow-could-not = Reprise could not { $action }.
pow-do-you-want = Do you want to { $action } anyway?
pow-updates-set = Updates will install the next time the computer starts, and Reprise could not change that.
pow-updates-set-cleared = { $set } Restarting anyway does not start { $title }.
pow-updates-set-pending = { $set } The computer is also still set to start { $title }, so restarting anyway starts it.
pow-aimed = The computer is set to start { $title } the next time it starts, and Reprise could not change that.
pow-act-title-turn-off = Turn Off Computer
pow-act-title-restart = Restart Computer
pow-act-title-stand-by = Stand By
pow-act-title-hibernate = Hibernate
# $the is "the" or "this".
pow-act-on-turn-off =
    turn off { $the ->
        [this] this
       *[the] the
    } computer
pow-act-on-restart =
    restart { $the ->
        [this] this
       *[the] the
    } computer
pow-act-on-stand-by =
    put { $the ->
        [this] this
       *[the] the
    } computer on stand by
pow-act-on-hibernate =
    hibernate { $the ->
        [this] this
       *[the] the
    } computer
pow-act-anyway-turn-off = &Turn Off Anyway
pow-act-anyway-restart = &Restart Anyway
pow-act-anyway-stand-by = &Stand By Anyway
pow-act-anyway-hibernate = &Hibernate Anyway
pow-lock-failed = An error stopped it.
pow-not-allowed = You are not allowed to { $action }.
pow-unsupported = This computer does not support it.
pow-held-unnamed = A program is preventing it.
pow-held = These programs are preventing it:
pow-late = It had not happened after { $minutes } minutes.
pow-internal = An error in Reprise stopped it.
pow-updates-stay = Updates will install the next time the computer starts.
pow-command-not-started = "{ $cmd }" did not start: { $error }
pow-command-failed = "{ $cmd }" failed ({ $status }).
pow-app-unnamed = A program
pow-screen-recording = Screen recording
pow-screen-recording-reason = Finishing the screen recording

## Restart Into
pow-firmware-setup = Firmware Setup
# Two boot entries share a title; the id tells them apart.
pow-boot-entry-shared = { $title } ({ $id })

## After a crash
pow-crash-notice = Reprise restarted after a crash; the log has the details


## Desktop context menus
desk-menu-new = Ne&w
desk-menu-arrange-icons-by = Arrange &Icons By
desk-menu-widgets = Wi&dgets
desk-menu-refresh = R&efresh
desk-menu-paste = &Paste
desk-menu-paste-shortcut = Paste &Shortcut
desk-menu-reprise-settings = &Reprise Settings
desk-menu-about-reprise = &About Reprise
desk-menu-next-desktop-background = Next desktop &background
desk-menu-folder = &Folder
desk-menu-shortcut = &Shortcut...
desk-menu-text-document = &Text Document
desk-menu-compressed-folder = Compressed (&zipped) Folder
desk-menu-name = &Name
desk-menu-auto-arrange = &Auto Arrange
desk-menu-copy-here = &Copy Here
desk-menu-move-here = &Move Here
desk-menu-create-shortcuts-here = Create &Shortcuts Here
desk-menu-cancel = C&ancel
desk-menu-open = &Open
desk-menu-explore = E&xplore
desk-menu-sharing-and-security = S&haring and Security...
desk-menu-run-as = Run &as...
desk-menu-print = &Print
desk-menu-edit = &Edit
desk-menu-open-with = Open Wit&h
desk-menu-allow-launching = Allow &Launching
desk-menu-send-to = Se&nd To
desk-menu-cut = Cu&t
desk-menu-copy = &Copy
desk-menu-create-shortcut = Create &Shortcut
desk-menu-delete = &Delete
desk-menu-rename = Rena&me
desk-menu-properties = P&roperties
desk-menu-choose-program = &Choose Program...
desk-menu-empty-recycle-bin = Empt&y Recycle Bin
desk-menu-open-in-terminal = Open in Termina&l
desk-menu-pin-to-places = Pin to Pl&aces
desk-menu-unpin-from-places = Unpin from Pl&aces

## Desktop file actions
desk-send-to = Send To
desk-allow-launching = Allow Launching
desk-allow-launching-failed = Cannot allow { $name } to launch: { $error }
desk-udisks-not-running = UDisks2 is not running.
desk-mount-failed-title = Could not mount { $name }
desk-paste = Paste
desk-paste-picture-failed = Cannot save the pasted picture: { $error }
desk-new-folder = New Folder
desk-new-text-document = New Text Document
desk-new-compressed-folder = New Compressed (zipped) Folder
desk-create-failed = Cannot create { $what }: { $error }

## Desktop system icons
desk-place-documents = Documents
desk-place-computer = Computer
desk-recycle-bin = Recycle Bin
desk-tip-documents = Contains letters, reports, and other documents and files.
desk-tip-pictures = Contains digital photos, images, and graphic files.
desk-tip-music = Contains music and other audio files.
desk-tip-videos = Contains movies and other video files.
desk-tip-downloads = Contains files downloaded from the Internet.

## Displays
desk-display-built-in = Built-in display
# A display label two heads share, with each one's connector.
desk-display-label-connector = { $label } ({ $connector })
desk-display-brightness = { $label }: { $percent }%
desk-display-not-adjustable = { $label }: not adjustable
desk-displays-window = Displays
desk-display-switcher-window = Display switcher

## Desktop widgets
desk-widget-analog = &Analog
desk-widget-digital = &Digital
desk-widget-show-seconds = Show &Seconds
desk-widget-bars = &Bars
desk-widget-graph = &Graph
desk-widget-remove = &Remove
desk-widgets-pane = Desktop widgets

## Calendar
desk-calendar-window = Calendar

## Start menu places
start-recent-items = Recent Items
start-computer = Computer
start-control-panel = Control Panel
start-printers = Printers
start-run = Run...
start-help-support = Help and Support
start-tip-recent = Lists the files you opened most recently.
start-tip-run = Opens a program, folder, document or web site by its name.
start-tip-help = Opens the Reprise project page.
start-tip-on-desktop = The desktop already holds a copy of this program.
start-tip-no-packagekit = Uninstalling a program from the system's packages needs PackageKit.

## Place context menu
start-place-open = &Open
start-place-explore = E&xplore
start-place-properties = P&roperties

## Program context menu
start-open = &Open
start-open-location = Open f&ile location
start-unpin-taskbar = Unpin from Tas&kbar
start-pin-taskbar = Pin to Tas&kbar
start-unpin-start = Unpin from Start Men&u
start-pin-start = Pin to Start Men&u
start-add-to-desktop = Add to &desktop
start-remove-from-list = Remove from this &list
start-jump-pin = Pi&n to this list
start-jump-unpin = Unpi&n from this list
start-uninstall = U&ninstall...
start-properties = P&roperties

## Shut down options
start-install-shut-down = Install &updates and shut down
start-install-restart = Install updates and r&estart
start-switch-user = S&witch user
start-log-off = &Log off
start-lock = L&ock
start-shut-down = Shu&t down
start-restart = &Restart
start-restart-into = Restart &into
start-sleep = &Sleep
start-hibernate = &Hibernate

## Window menu
start-win-restore = &Restore
start-win-move = &Move
start-win-size = &Size
start-win-minimize = Mi&nimize
start-win-maximize = Ma&ximize
start-win-roll-down = Roll Do&wn
start-win-roll-up = Roll &Up
start-win-always-on-top = Always on &Top
start-win-open-here = Always &Open Here
start-win-mini-view = M&ini View
start-win-all-desktops = Show on &All Desktops
start-win-move-to-desktop = Move to &Desktop
start-win-move-to-screen = Move to Scr&een
start-win-add-to-tabs = Add to Ta&bs of
start-win-leave-tabs = Remove from Tab &Group
start-win-close = &Close
start-win-recent-files = Recent &Files
# The desktop's number; the keyed form marks it as the access key.
start-win-desktop-keyed = Desktop &{ $number }
start-win-desktop = Desktop { $number }
start-win-screen-keyed = &{ $number }. { $name }
start-win-screen = { $number }. { $name }

## Menus
start-menu-start = Start Menu
start-menu-window = Window Menu
start-menu-context = Context Menu
start-empty = (Empty)

## Start panel and launcher
start-panel-name = Start menu
start-launcher-name = Launcher
start-search-calculator = Calculator
# The counts are plain digits.
start-search-programs = Programs ({ $count })
start-search-settings = Settings ({ $count })
start-search-uninstall = Uninstall a program
start-search-uninstall-detail = Right-click it in All Programs
# A found section's heading and its row count.
start-search-found = { $heading } ({ $count })
start-search-files = Files
start-search-contents = Contents

## Taskbar menu
bar-menu-rename-desktop = Rena&me Desktop...
bar-menu-toolbars = &Toolbars
bar-menu-cascade = Casca&de Windows
bar-menu-tile-horizontally = Tile Windows &Horizontally
bar-menu-tile-vertically = Tile Windows &Vertically
bar-menu-show-desktop = &Show the Desktop
bar-menu-task-manager = Tas&k Manager
bar-menu-lock-taskbar = &Lock the Taskbar
bar-menu-properties = P&roperties
bar-menu-quick-launch = &Quick Launch

## Start button menu
bar-start-open = &Open
bar-start-explore = E&xplore
bar-start-properties = P&roperties

## Quick Launch button menu
bar-quick-open = &Open
bar-quick-delete = &Delete

## Jump lists
bar-jump-pinned = Pinned
bar-jump-recent = Recent
bar-jump-tasks = Tasks
bar-jump-pin = Pin this program to taskbar
bar-jump-unpin = Unpin this program from taskbar
bar-jump-close-window = Close window
bar-jump-close-all-windows = Close all windows
bar-move-to-desktop = Move to desktop

## Grouped task buttons
bar-group-minimize = Minimize Group
bar-group-close = Close Group
# The label of a group whose app id is empty.
bar-group-name-fallback = Window
# A group of several windows: the program's name and the window count.
bar-group-title = { $name } ({ $count })

## Tooltips
# The second line of the maximize button's tip; keys is the bound shortcut.
bar-tip-snap-layouts = Snap layouts: { $keys }
bar-tip-hide = Hide
bar-tip-show-hidden-icons = Show hidden icons
bar-tip-start = Click here to begin
bar-tip-show-desktop = Show desktop
# Ends a desktop's tip past the windows it lists.
bar-tip-pager-more = and { $count } more
bar-tip-more-windows = Show more windows
bar-tip-network-speed = Network speed

## Notification bell tip
bar-bell-none = Notifications
bar-bell-unseen =
    { $count ->
        [one] 1 new notification
       *[other] { $count } new notifications
    }
# time is the end of a timed Do Not Disturb.
bar-bell-until = { " " }until { $time }
bar-bell-fullscreen = { " " }while a full-screen window has focus
bar-bell-sharing = { " " }while the screen is shared
bar-bell-scheduled = { " " }during quiet hours
# why is one of the bar-bell- reasons above, with its leading space.
bar-bell-dnd = Do not disturb is on{ $why }

## Accessibility names
bar-taskbar-name = Taskbar
bar-switcher-name = Task Switcher
bar-previews-name = Window previews
bar-flyout-name = Hidden icons
bar-snap-layouts-name = Snap layouts
bar-overview-windows = Windows
bar-overview-snap-assist = Snap assist
bar-switcher-desktop = Desktop
bar-media-playing = Playing
bar-media-paused = Paused

## Run and Rename Desktop
dlg-run-title = Run
dlg-run-text = Type the name of a program, folder, document, or Internet resource, and Reprise will open it for you.
dlg-run-open = &Open:
dlg-run-browse = &Browse...
dlg-run-not-found = Reprise cannot find '{ $word }'. Make sure you typed the name correctly, and then try again.
# The path is a program whose file name is not valid UTF-8.
dlg-not-utf8 = Cannot use '{ $path }': its name is not valid UTF-8.
dlg-rename-desktop-title = Rename Desktop
# The number is the desktop's position, from 1.
dlg-rename-desktop-text = Type a name for desktop { $number }. With no name it is called Desktop { $number }.
dlg-rename-desktop-name = &Name:

## Confirm Delete on desktop icons
dlg-confirm-delete-title = Confirm Delete
dlg-confirm-file-delete-title = Confirm File Delete
dlg-hide-restore-folder = The contents of the folder will not be deleted. You can restore this icon to the desktop on the Background page of Reprise Settings.
dlg-hide-restore = To restore it later, use the Background page of Reprise Settings.
# The restore variable is one of the two sentences above.
dlg-hide-text =
    Are you sure you want to delete the { $label } icon from your desktop?

    { $restore }
dlg-icons-restore = To restore them later, use the Background page of Reprise Settings.
dlg-icons-restore-files =
    One or more of the items you are about to delete cannot be restored from the Recycle Bin.
    To restore them later, use the Background page of Reprise Settings.
dlg-icons-text =
    Are you sure you want to delete these icons from your desktop?

    { $restore }

## Accessibility key shortcut prompts
dlg-key-aid-sticky-title = StickyKeys
dlg-key-aid-sticky-text =
    Pressing the Shift key five times is the shortcut for StickyKeys. With StickyKeys, Shift, Ctrl, Alt and the Super key work one key at a time.

    Turn on StickyKeys?
    To leave it off, click No.
    To turn off this shortcut, click Settings...
dlg-key-aid-slow-title = SlowKeys
dlg-key-aid-slow-text =
    Holding down the right Shift key for eight seconds is the shortcut for SlowKeys. With SlowKeys, a key counts only once it has been held down for a moment, so brief keystrokes are ignored.

    Turn on SlowKeys?
    To leave it off, click No.
    To turn off this shortcut, click Settings...
dlg-key-aid-mouse-title = MouseKeys
dlg-key-aid-mouse-text =
    Pressing left Alt, left Shift and Num Lock together is the shortcut for MouseKeys. With MouseKeys, the keypad moves the pointer and clicks.

    Turn on MouseKeys?
    To leave it off, click No.
    To turn off this shortcut, click Settings...
dlg-key-aid-settings = &Settings...

## End Program
dlg-end-program-title = End Program - { $program }
dlg-end-program-text =
    This program has stopped responding.
    To go back to the desktop and give the program time to recover, click Cancel.
    Ending the program right away loses anything it has not saved. To end the program now, click End Now.
dlg-end-program-end = End Now

## Run's Browse dialog
dlg-browse-title = Browse
dlg-browse-open = &Open
dlg-browse-programs = Programs
dlg-browse-all-files = All Files

## File jobs
dlg-job-delete-failed = Error Deleting File or Folder
dlg-job-undo = Undo
dlg-job-new-shortcut = New Shortcut
dlg-job-explorer-failed = Cannot start Explorer: { $error }
dlg-menu-open-file-location = Open f&ile location

## The keyboard shortcut sheet
dlg-keys-run = Run { $command }
dlg-keys-start = Start { $app }
dlg-keys-start-menu = Start menu
dlg-keys-separator = {", "}
# A program's global shortcut: its name, then what the shortcut does.
dlg-keys-global = { $program }: { $what }

## Desktop file actions
dlg-edit-paste = Paste
dlg-edit-rename = Rename
dlg-edit-rename-failed = Cannot rename { $name }: { $error }
dlg-edit-undo = Undo
dlg-edit-undo-failed = Cannot undo: { $error }
dlg-edit-error = Error

## Clipboard history, emoji and colour pickers
dlg-clip-title = Clipboard History
# The size of an image in the clipboard history, in plain digits.
dlg-clip-image = Image, { $width }x{ $height }
dlg-emoji-title = Emoji
dlg-colour-title = Colour picker
dlg-colour-copied-title = Colour copied
dlg-colour-copied-text = { $hex } is on the clipboard.

## Screenshots
screenshot-saved = Screenshot saved
screenshot-not-saved = Screenshot not saved
# The error's reason on the first line.
screenshot-not-saved-body =
    { $error }
    The picture is on the clipboard.
screenshot-saving = Saving screenshots...
screenshot-saving-body = The session ends once they are saved.
screenshot-action-edit = Edit
screenshot-action-folder = Show in Folder
screenshot-note-busy = Earlier screenshots were still being encoded, so this one was saved as PNG.
screenshot-note-memory = Encoding it needed more than { $megabytes } MB, so it was saved as PNG.
screenshot-note-late = Encoding it took longer than { $minutes } minutes, so it was saved as PNG.
screenshot-note-outdated = The JPEG XL encoder could not start, so it was saved as PNG. Log out and back in after an update.
screenshot-note-no-library = Cannot encode it: the JPEG XL library did not load, so it was saved as PNG.
screenshot-note-killed = Cannot encode it: the encoder was killed, so it was saved as PNG.
screenshot-note-codec = Cannot encode it: { $error }, so it was saved as PNG.
screenshot-no-home = There is no home folder to save it in.
screenshot-cannot-make = Cannot make { $path }: { $error }

## Screen recording
recording-saved = Screen recording saved
recording-not-finished = Screen recording not finished
recording-may-not-play = It was stopped while it closed the file and may not play.
recording-failed = Screen recording failed
# The recorder's exit reason on the first line; { $reason } is its last stderr line.
recording-stopped-early = Stopped early: { $reason }
recording-sound-stopped = The sound stopped: { $reason }
recording-not-started = { $program } did not start: { $error }.
recording-wait-failed = Waiting for { $program } failed: { $error }.
recording-library-missing = A library the recorder needs did not load.
recording-status = The recorder stopped with status { $code }.
recording-killed = The recorder was killed.
recording-action-open = Open
recording-action-folder = Show in Folder
recording-no-home = There is no home folder to save it in.
recording-cannot-make = Cannot make { $path }: { $error }
recording-cannot-write = Cannot write in { $path }: { $error }
recording-tip-finishing =
    Finishing the recording...
    Click to stop it now; the file may not play.
# { $time } is minutes:seconds or hours:minutes:seconds.
recording-tip-running =
    Recording the screen, { $time }
    Click to stop recording.

## Region picker
region-title-record = Record an area
region-title-screenshot = Screenshot of an area

## On-screen display
osd-muted = Muted
osd-volume = Volume
osd-mic-muted = Microphone muted
osd-mic-on = Microphone on
osd-keyboard-backlight = Keyboard backlight
osd-play-pause = Play / Pause
osd-previous = Previous
osd-next = Next
# The media player's name, then what the key did.
osd-player = { $name }: { $verb }
osd-wireless-on = Wireless on
osd-wireless-held = Wireless held off by a hardware switch
osd-wireless-off = Wireless off
osd-touchpad-on = Touchpad on
osd-touchpad-off = Touchpad off
osd-airplane-on = Airplane mode on
osd-airplane-off = Airplane mode off
osd-rotation-on = Rotation lock on
osd-rotation-off = Rotation lock off
media-brightness = Brightness

## Screen reader
screen-reader-on = Screen reader on
screen-reader-off = Screen reader off
screen-reader-missing = No screen reader installed
screen-reader-failed = The screen reader did not start

## Night light
night-light-off = Off
night-light-panel-paused = Paused until { $time }
night-light-panel-until = until { $time }
night-light-panel-from = from { $time }
night-light-menu-settings = &Settings...
night-light-menu-resume = &Resume Night Light
night-light-menu-resumes-at = Resumes at { $time }
night-light-menu-disable-hour = Disable for 1 &hour
night-light-menu-disable-tomorrow = Disable until &tomorrow
night-light-tip-paused = Night light is paused until { $time }
night-light-tip-kelvin = Night light: { $kelvin } K
night-light-tip-starts = Night light starts at { $time }
night-light-tip = Night light

## Lock screen
lock-incorrect = The password is incorrect. Please retry.
lock-unavailable = The password could not be checked. Please retry.
lock-expired-summary = Your password has expired
lock-expired-body = Change it now. The next login to a new session will ask for a new password.
lock-finger-too-many = Too many fingerprint attempts. Type your password to unlock.
lock-backoff =
    Too many incorrect passwords. Wait { $seconds ->
        [one] { $seconds } second
       *[other] { $seconds } seconds
    } and retry.

## Notifications
notify-center-name = Notifications
notify-balloons-name = Notifications
notify-more = { $count } more notifications
notify-age-now = now
notify-age-minutes = { $minutes } min ago
notify-age-hours = { $hours } h ago
# { $count } balloons from the app { $app }.
notify-burst-title = { $count } new from { $app }
notify-program-no-balloons = Don't Show &Balloons
notify-program-no-sounds = No &Sounds
dnd-until = Do not disturb until { $time }
dnd-on = Do not disturb is on
dnd-menu-turn-off = &Turn off
dnd-menu-on = &Do not disturb
dnd-menu-hour = Do not disturb for 1 &hour
dnd-menu-tomorrow = Do not disturb until &tomorrow

## Privacy indicators
privacy-captured-screen-and-window = Your screen and a window were captured
privacy-captured-window = A window was captured
privacy-captured-screen = Your screen was captured
privacy-app-unnamed = An application
# The next messages list apps: { $count } is how many, { $one } the only one, { $rest } all but
# the last, comma separated, and { $last } the last.
privacy-controlling =
    { $count ->
        [0] An application is controlling this computer
        [1] { $one } is controlling this computer
       *[other] { $rest } and { $last } are controlling this computer
    }
privacy-using-camera =
    { $count ->
        [0] An application is using your camera
        [1] { $one } is using your camera
       *[other] { $rest } and { $last } are using your camera
    }
privacy-using-microphone =
    { $count ->
        [0] An application is using your microphone
        [1] { $one } is using your microphone
       *[other] { $rest } and { $last } are using your microphone
    }
privacy-shared-screen-and-window = Your screen and a window are being shared
privacy-shared-window = A window is being shared
privacy-shared-screen = Your screen is being shared
privacy-what-screen-and-window = your screen and a window
privacy-what-window = a window
privacy-what-screen = your screen
# { $what } is what is shared: your screen, a window, or both.
privacy-sharing =
    { $count ->
        [1] { $one } is sharing { $what }
       *[other] { $rest } and { $last } are sharing { $what }
    }

## Location
# { $count } is how many programs, { $one } the only one, { $rest } all but the last, comma
# separated, and { $last } the last.
location-using =
    { $count ->
        [0] A program is using your location
        [1] { $one } is using your location
       *[other] { $rest } and { $last } are using your location
    }

## Accessibility aids (StickyKeys, SlowKeys, MouseKeys)

misc-aid-sticky-shortcut = Pressing { $shift } five times
misc-aid-slow-shortcut = Holding the right { $shift } key for eight seconds
misc-aid-mouse-shortcut = Left { $alt }+left { $shift }+{ $num_lock }
misc-aid-sticky-what = { $shift }, { $ctrl }, { $alt } and the { $super_key } key work one key at a time.
misc-aid-slow-what = A key counts only once it has been held down for a moment.
misc-aid-mouse-what = The keypad moves the pointer. Hold { $ctrl } to go faster or { $shift } to creep.
# $what is the aid's description, $keys the shortcut that turns it off
misc-aid-on-body-shortcut = { $what } { $keys } turns it off.
misc-aid-off-body-shortcut = { $keys } turns it on again.
misc-aid-off-body-settings = Click here to turn it on again in Settings.
misc-aid-sticky-on-title = StickyKeys is on
misc-aid-sticky-off-title = StickyKeys is off
misc-aid-slow-on-title = SlowKeys is on
misc-aid-slow-off-title = SlowKeys is off
misc-aid-mouse-on-title = MouseKeys is on
misc-aid-mouse-off-title = MouseKeys is off
# $keys is a comma-separated list of modifier names
misc-sticky-tip-held = Held for the next key: { $keys }
misc-sticky-tip-locked = Locked: { $keys }
misc-mousekeys-button-left = left
misc-mousekeys-button-middle = middle
misc-mousekeys-button-right = right
misc-mousekeys-tip =
    MouseKeys
    The keys click the { $button } button
misc-mousekeys-tip-held = The { $button } button is held down

## Keyboard shortcuts taken

misc-shortcuts-taken-someone = A program
misc-shortcuts-taken-title = { $name } took the keyboard shortcuts
misc-shortcuts-taken-body = { $keys } gives them back.

## Debug overlay

misc-debug-renders = Renders/s
misc-debug-cpu-render = CPU/render
misc-debug-flip-interval = Flip interval
misc-debug-handoff-commit = Handoff + commit
misc-debug-damage = Damage
misc-debug-clients = Clients
misc-debug-windows = Windows
misc-debug-surfaces = Surfaces
misc-debug-memory = Memory

## Config file notices

misc-config-notice-title = Configuration
misc-config-keeps-settings = Reprise keeps the settings from before.
misc-config-started-defaults = Reprise started with the defaults.
misc-config-unreadable-at-start = Reprise could not read config.toml and started with the defaults. `journalctl --user -t reprise` names the error.
misc-config-error-at = { " " }line { $line }, column { $column }
# $at is misc-config-error-at or empty, $then says what Reprise runs with
misc-config-broken =
    { $name }{ $at }: { $message }.
    { $then } Click here to open the file.
# $reason is the I/O error kind
misc-config-unreadable =
    Reprise could not read { $file }: { $reason }.
    { $then }
misc-config-dropped-kept = kept their previous values
misc-config-dropped-reset = used the defaults there
misc-config-dropped-mixed = kept the previous values of { $kept }; { $reset } used the defaults
# $keys are old key names such as glass.mode
misc-config-moved = { $keys } in config.toml belong in [appearance] now. Reprise reads them for Glass where [appearance] leaves them out.
misc-config-dropped = Reprise ignored { $keys } in config.toml and { $what }. `journalctl --user -t reprise` names the errors.
misc-quicklaunch-browser = Launch Web Browser
misc-quicklaunch-show-desktop = Show Desktop
misc-quicklaunch-terminal = Terminal

## X11

misc-xwayland-down-title = X11 programs cannot run
misc-xwayland-down-body = Xwayland would not start. Programs that need X11 will not open until you log on again.
