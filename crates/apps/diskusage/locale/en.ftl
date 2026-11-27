# Disk Usage. An & marks the next letter as the access key.

name = Disk Usage

## The column headers
column-name = Name
column-size = Size
column-share = Share
column-files = Files

## The buttons
back = < &Back
up = &Up
browse = Br&owse...
refresh = &Refresh
explore = Open in &Explorer
cleanup = &Clean Up...

## An entry's shortcut menu
menu-open = &Open
menu-explore = Open in &Explorer
menu-trash = Move to &Recycle Bin

## The list and the status line
# The Size cell of a folder that is another drive's mount point.
another-drive = Another drive
# $size is a file size such as "4.0 KB".
status-sized =
    { $count ->
        [one] { $size } in { $count } file
       *[other] { $size } in { $count } files
    }
status-counting = Counting... { $sized } so far
not-responding = The location is not responding.
unreadable = The folder cannot be read.

## Browse...'s folder dialog
title-browse = Browse for Folder
ok = OK

## Message boxes
title-delete-failed = Error Deleting File or Folder
no-recycle-bin = there is no Recycle Bin

## Clean Up
cleanup-title = Clean Up
cleanup-intro = Clean Up removes files you can do without. Pick what to remove.
cleanup-files = Files to remove
# The parts, each a check box. An & marks the access key.
cleanup-recycle-bin = &Recycle Bin
cleanup-thumbnails = &Thumbnails
cleanup-caches = Program &caches
cleanup-coredumps = Crash &dumps
cleanup-flatpak = Unused &Flatpak runtimes
# A part's size while it is still being counted.
cleanup-counting = Counting...
# $size is a file size such as "4.0 KB".
cleanup-total = Total to remove: { $size }
cleanup-description = Description
cleanup-about-recycle-bin = Files you deleted wait in the Recycle Bin. Emptying it removes them for good, on every drive.
cleanup-about-thumbnails = Small pictures of your files that folders show. Folders draw them again when they need them.
cleanup-about-caches = Files programs keep in your cache folder to start or load faster. Programs make them again, a little slower the first time.
cleanup-about-coredumps = Copies of programs' memory saved when they crashed. The system keeps them and removes them itself after a while.
cleanup-about-flatpak = Runtimes no installed Flatpak program uses. Flatpak decides which runtimes are unused and removes them.
cleanup-confirm = Are you sure you want to permanently remove these files?
# A line of the confirm box. $part is a part's name, $size its size.
cleanup-confirm-row = { $part }: { $size }
# A line of the confirm box for a part that empties a folder. $folder is the folder's path.
cleanup-confirm-folder-row = { $part } ({ $folder }): { $size }
# A line under the Flatpak part's line, one per runtime Flatpak would remove. $runtime is its
# id, arch and branch, such as "org.gnome.Platform/x86_64/47".
cleanup-confirm-runtime = - { $runtime } ({ $installation ->
        [system] system installation
       *[user] your installation
    })
cleanup-failed = Some files could not be removed.
# $reason is the system's error message.
cleanup-failed-row = { $part }: { $reason }
