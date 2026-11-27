# Software Updates: the update list, the tray tips and balloons, install and check messages.

## Update kinds, groups and origins
update-kind-security = Security update
update-kind-important = Important update
update-kind-bugfix = Bug fix
update-kind-normal = Update
update-kind-enhancement = Enhancement
update-kind-low = Optional update
update-group-packages = System packages
update-group-flatpak = Flatpak
update-group-firmware = Firmware
update-install-system = system
update-install-user = user
update-origin-app = Application ({ $install })
update-origin-runtime = Runtime ({ $install })
update-count =
    { $count ->
        [one] 1 update
       *[other] { NUMBER($count, useGrouping: "false") } updates
    }

## Window headline
headline-unchecked = Reprise has not checked for updates yet.
headline-checking = Checking for updates...
headline-up-to-date = Your computer is up to date.
headline-none-selected = 0 of { $updates } selected. Check the updates you want to install.
headline-some-selected = { NUMBER($picked, useGrouping: "false") } of { $updates } selected. Click Install to download and install { $picked ->
        [one] it
       *[other] them
    }.
headline-available = { $updates } { $count ->
        [one] is
       *[other] are
    } available for your computer. Click Install to download and install { $count ->
        [one] it
       *[other] them
    }.
headline-installing = Installing updates. You can close this window; the updates keep installing.
headline-restart-picked = Your computer needs a restart to finish updating. { $updates } selected: click Install to install { $picked ->
        [one] it
       *[other] them
    }, or Restart Now.
headline-installed-restart = The updates were installed. Restart your computer to finish installing them.
headline-installed-logoff = The updates were installed. Log off and back on to finish installing them.
headline-installed = The updates were installed.
headline-due-system = The system was updated. Some devices may not work until you restart your computer.
headline-due-reprise = Reprise was updated. Log off and back on, or restart your computer, to use the new version.
progress-line = { $doing } ({ $percent }%)

## Tray tips
tip-installing-percent = Installing updates ({ $percent }%)
tip-installing = Installing updates
tip-restart = Restart your computer to finish installing updates
tip-due-system = Restart to finish updating
tip-due-reprise = Log off or restart to finish updating Reprise
tip-logoff = Log off to finish installing updates
tip-up-to-date = Your computer is up to date
tip-available = { $updates } { $count ->
        [one] is
       *[other] are
    } available

## Balloons
balloon-news-security = Security updates are ready for your computer
balloon-news = Updates are ready for your computer
balloon-news-body = { $updates } { $count ->
        [one] is
       *[other] are
    } available. Click here to review and install { $count ->
        [one] it
       *[other] them
    }.
check-failed = Reprise could not check for updates. { $why }
install-failed = The updates could not be installed. { $why }
balloon-installed-title = Updates installed
balloon-installed-restart = Restart your computer to finish installing them.
balloon-installed-logoff = Log off and back on to finish installing them.
balloon-installed-none = Your computer is up to date.
balloon-due-system-title = Restart to finish updating
balloon-due-system = The system was updated. Some devices may not work until you restart.
balloon-due-reprise-title = Reprise was updated
balloon-due-reprise = Log off and back on, or restart, to use the new version.
balloon-request-now = Firmware update needs you
balloon-request-later = Finish the firmware update

## Window and tray
updates-title = Software Updates
column-name = Name
column-version = Version
column-type = Type
check-now = &Check Now
install = &Install
restart-now = &Restart Now
tray-review = Review Updates
tray-check = Check for Updates
balloon-restart-now = Restart Now
balloon-later = Later
install-preparing = Preparing to install
install-failed-title = Updates were not installed
install-updating = Updating { $name }
install-no-bus = There is no system bus.
install-no-packagekit = PackageKit is not installed.
install-no-fwupd = fwupd is not installed.

## Install guard
guard-more = { $list } and { NUMBER($count, useGrouping: "false") } more
guard-group = { $list } ({ $reason })
guard-reason-system = needed by the system
guard-reason-reprise = part of Reprise
guard-both = { $first } and { $second }
guard-refusal =
    { $verb ->
        [install] Installing it would remove { $parts }.
       *[uninstall] Uninstalling it would remove { $parts }.
    }

## Install sheet
installer-no-packagekit = Reprise installs and uninstalls the system's packages through PackageKit, which is not installed.
installer-no-flatpak = Installing a Flatpak needs Flatpak, which is not installed.
installer-bad-app-id = This program's Flatpak app ID is not valid, so Reprise cannot uninstall it.
installer-unknown = Unknown
installer-size-later = Known once the download starts
installer-size-up-to = Up to { $size } to download
installer-launcher-note = Only the launcher is removed. The program that installed it keeps its own files.
installer-more = { $list } and { NUMBER($count, useGrouping: "false") } more
installer-also-uninstall =
    Uninstalling it also removes { $count ->
        [one] 1 package that needs
       *[other] { NUMBER($count, useGrouping: "false") } packages that need
    } it: { $list }.
installer-also-install =
    Installing it also installs or updates { $count ->
        [one] 1 package
       *[other] { NUMBER($count, useGrouping: "false") } packages
    }: { $list }.
installer-note-removes =
    Installing it removes { $count ->
        [one] 1 package
       *[other] { NUMBER($count, useGrouping: "false") } packages
    }: { $list }.
installer-note-also-removes =
    { $added } It removes { $count ->
        [one] 1 package
       *[other] { NUMBER($count, useGrouping: "false") } packages
    }: { $list }.
installer-conflicts =
    It conflicts with { $count ->
        [one] 1 installed package
       *[other] { NUMBER($count, useGrouping: "false") } installed packages
    }: { $list }. Uninstall { $count ->
        [one] it
       *[other] them
    } first.
installer-password-dismissed = The password prompt was dismissed.
installer-fact-app-id = App ID:
installer-fact-version = Version:
installer-fact-size = Size:
installer-fact-source = Source:
installer-fact-installed-by = Installed by:
installer-fact-package = Package:
installer-fact-branch = Branch:
installer-fact-file = File:
installer-fact-packager = Packager:
installer-source-flatpak-install = { $origin } (Flatpak, { $install } installation)
installer-source-packages = { $os } packages
installer-source-flatpak = { $source } (Flatpak)
installer-source-bundle = Flatpak bundle file
installer-note-kept-files = Its settings and files in your home folder are kept.
installer-note-no-owner = No installed package holds this program.
installer-note-ref-same-source = It is installed for every user of this computer, with the runtime it needs from the same source, which stays added for later installs and updates.
installer-note-ref-runtime = It is installed for every user of this computer, with the runtime it needs from { $runtime }. Both sources stay added for later installs and updates.
installer-packager-checked = { $name } (signature checked when it installs)
installer-packager-unverified = (unverified) { $name }
installer-note-bad-kind = Reprise cannot install this kind of file.
installer-note-not-flatpak = This file does not name a Flatpak app.
installer-note-bundle = It is installed for every user of this computer.
installer-note-other-packages = This system's packages are of another kind.
installer-nothing-to-do = There is nothing to do.
installer-launcher-failed = The launcher could not be removed: { $why }.
installer-doing-install = Installing { $id }
installer-doing-uninstall = Uninstalling { $id }
installer-cancelled-install = The install was cancelled.
installer-cancelled-uninstall = The uninstall was cancelled.
installer-waiting-flatpak = Waiting for Flatpak to stop
installer-undoing = Removing what it installed before the cancel
installer-no-bus = There is no system bus.
installer-file-unreadable = Reprise could not read the file: { $why }.
installer-stopped-install = The install stopped unexpectedly.
installer-stopped-uninstall = The uninstall stopped unexpectedly.
installer-preparing = Preparing

## PackageKit, pacman and Flatpak
step-download = Downloading { $name }
step-install = Installing { $name }
step-remove = Removing { $name }
pk-timeout = PackageKit took too long and was stopped.
pk-stopped = PackageKit stopped.
pk-quiet = PackageKit went quiet.
pk-unfinished = PackageKit did not finish the request.
cancelled-uninstall = The uninstall was cancelled.
cancelled-install = The install was cancelled.
cancelled-update = The update was cancelled.
pkgfile-no-tool = { $program } is not installed, so Reprise cannot read this package.
pkgfile-unreadable = The package could not be read.
pkgfile-not-a-package = Not a package file.
pkgfile-no-name = The package does not name itself.
pkgfile-pkexec-failed = pkexec could not be started: { $why }.
pkgfile-pacman-failed = pacman did not install the package.
flatpak-start-failed = Flatpak could not be started: { $why }.
flatpak-failed = Flatpak failed.
flatpak-from = from { $origin }
flatpak-from-size = from { $origin }, { $size } to download

## Firmware
fwupd-unfinished = fwupd stopped answering before the firmware install finished.
fwupd-curl-failed = curl could not be started: { $why }.
fwupd-download-failed = Could not download { $url }: { $why }
fwupd-open-failed = Could not open { $path }: { $why }.
fwupd-step-decompress = Preparing the firmware for { $name }
fwupd-step-write = Writing the firmware to { $name }
fwupd-step-verify = Verifying { $name }
fwupd-step-restart = Restarting { $name }
fwupd-step-schedule = Scheduling the update for { $name }
fwupd-step-auth = Waiting for authentication
fwupd-step-busy = Waiting for { $name }
fwupd-step-user = Waiting for you
fwupd-request-replug = Unplug { $name } and plug it back in to continue the update.
fwupd-request-remove-cable = Unplug the USB cable of { $name } to continue the update.
fwupd-request-insert-cable = Plug the USB cable of { $name } back in to continue the update.
fwupd-request-replug-power = Unplug the power cable of { $name } and plug it back in to continue the update.
fwupd-request-unlock = Press unlock on { $name } to continue the update.
fwupd-request-keep-power = Do not turn off your computer or unplug its power until { $name } has finished updating.
fwupd-request-restart-daemon = Restart the fwupd service to finish updating { $name }.
fwupd-request-waiting = { $name } is waiting for you to continue the update.
fwupd-no-newer = { $name } has no newer firmware.
fwupd-no-source = The firmware for { $name } has nowhere to download from.
fwupd-downloading = Downloading the firmware for { $name }
