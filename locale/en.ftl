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
# The determiner is "the" or "this".
pow-act-on-turn-off = turn off { $the } computer
pow-act-on-restart = restart { $the } computer
pow-act-on-stand-by = put { $the } computer on stand by
pow-act-on-hibernate = hibernate { $the } computer
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
start-uninstall = U&ninstall...
start-properties = P&roperties

## Shut down options
start-install-shut-down = Install &updates and shut down
start-install-restart = Install updates and r&estart
start-switch-user = S&witch user
start-log-off = &Log off
start-lock = L&ock
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
start-win-add-to-tabs = Add to Ta&bs of
start-win-leave-tabs = Remove from Tab &Group
start-win-close = &Close
start-win-recent-files = Recent &Files
# The desktop's number; the keyed form marks it as the access key.
start-win-desktop-keyed = Desktop &{ $number }
start-win-desktop = Desktop { $number }

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
