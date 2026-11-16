# Drives: the Safely Remove Hardware icon and its balloons.

# The tray icon's tooltip and the program's title.
tip = Safely Remove Hardware
# A volume or drive with no name of its own.
removable-disk = Removable Disk

## The Safely Remove Hardware menu
safely-remove = Safely remove { $device }
# $volumes are the drive's mounted volumes, comma separated.
safely-remove-drive = Safely remove { $drive } - { $volumes }

## A new drive
# A volume without a label: its size, such as "7.45 GB", and its drive's name.
arrival-name = { $size } { $drive }
new-drive = New drive: { $name }
locked-body = This drive is locked. Unlock it to use the files on it.
unlock-and-open = Unlock and Open
unlock = Unlock
mount-body = Mount it to use the files on it.
mount-and-open = Mount and Open
mount = Mount
mounted-body = Mounted at { $path }. Click here to open it.
mount-failed = Could not mount { $name }

## Removing a drive
safe-title = Safe To Remove Hardware
safe-body = The '{ $disk }' device can now be safely removed from the system.
eject-failed = Problem Ejecting { $disk }
busy-body = '{ $volumes }' is in use. Close any program or window using it, and then try again.

## Low disk space
low-space-title = Low Disk Space
# $free and $total are sizes such as "812 MB".
low-space-body = The disk of { $mount } has only { $free } free of { $total }.
low-space-open = Open
show-usage = Show Usage
empty-bin = Empty Recycle Bin

## Thunderbolt devices
authorize-title = Authorize { $device }?
authorize-body = This Thunderbolt device does nothing until you authorize it, and can then reach this computer's memory. Authorize it only if you trust it.
authorize-once = Authorize Once
authorize-always = Always Authorize
authorize-ignore = Ignore
authorize-failed = Could not authorize { $device }
change-not-started = Reprise could not start the change: { $error }
