## Dates. long-date is a strftime pattern written in the LC_TIME locale's day and month names;
## $order is the order its short date puts day, month and year in (dmy, mdy, ymd).
long-date =
    { $order ->
        [dmy] %A %-d %B %Y
        [ymd] %A, %x
       *[mdy] %A, %B %-d, %Y
    }
## A file's date in Properties, in place of the weekday.
date-today = Today
date-yesterday = Yesterday

## Running a program or script from a file view
script-run = R&un
script-run-in-terminal = Run in termina&l
open-file-title = Open File
run-failed = Cannot run { $name }: { $why }
run-failed-not-found = The program that runs it was not found.
run-failed-denied = Access is denied.
run-failed-not-a-program = It is not a program this computer can run.

## Mounting a disk image
mount-image-title = Mount Disk Image
mount-image-mount = &Mount
mount-image-text =
    "{ $name }" is a disk image. Mounting it has this computer read the file system inside, which an image from someone you do not trust can use to harm it. It mounts read-only.

    Do you want to mount it?

## Compressing and extracting
compress-failed-title = Error Compressing File
compress-failed = Cannot compress { $name }: { $why }
compress-cancelled = the operation was cancelled.
extract-failed-title = Error Extracting File
extract-failed = Cannot extract { $name }: { $why }
extract-cancelled = the operation was cancelled.
extract-too-many = it holds { NUMBER($entries, useGrouping: "false") } entries, more than the { NUMBER($most, useGrouping: "false") } one extraction here takes.
extract-no-room = it expands to { $size }, more than the { $room } the free space here allows.
extract-climbs = it holds { $entry }, which would land outside the folder.
extract-link-out = it holds { $entry }, a link to outside the folder.
extract-not-a-folder = it holds the folder { $folder }, which is not a folder here.
# $why is why the merge stopped, a sentence of its own.
extract-kept = { $why } Some of its entries stay in place, and the rest in { $folder }.
extract-no-folder = it has no folder
extract-no-output = bsdtar gave no output
extract-no-bsdtar = bsdtar cannot start ({ $why }); is libarchive installed?
# $why is the first entry's reason; $others counts the entries that failed after it.
extract-and-others =
    { $others ->
        [one] { $why }, and 1 other item could not be extracted
       *[other] { $why }, and { NUMBER($others, useGrouping: "false") } other items could not be extracted
    }
compress-and-others =
    { $others ->
        [one] { $why }, and 1 other item could not be read
       *[other] { $why }, and { NUMBER($others, useGrouping: "false") } other items could not be read
    }
archive-unread = bsdtar could not read it.
archive-too-many = it holds more than { NUMBER($most, useGrouping: "false") } items.
archive-no-room = it takes { $size }, more than the free space here allows.
archive-more-than-listed = it holds more than its listing says.
archive-other-size = it holds less than its listing says.
archive-drag-too-large = the items picked take { $size }, more than a drag copies out of a compressed folder ({ $most }). Use Copy To Folder... or Extract All... instead.
archive-too-long = it took too long to read. Extract it to get at its files.
archive-missing = the file cannot be found or read.
archive-no-cache = there is no cache folder to copy it to.
background-no-data-dir = there is no data folder to keep the picture in.
background-no-name = the picture has no file name.
archive-locked = it is password-protected.
archive-not-held = it no longer holds { $name }.
archive-path-too-long = it holds a path too long to copy out.
archive-line-too-long = bsdtar listed a name too long to read.
archive-names-too-long = its names add up to more than { $most }.
archive-update-failed = Cannot update { $name }: { $why }
archive-update-not-zip = only a zip compressed folder can be written into.
archive-update-not-own = it is a link, or not a file of yours.
archive-update-too-large = it holds { $size }, more than the { $most } a save writes into.
archive-update-too-long = it took too long to rewrite.
archive-update-mismatch = the rewritten copy did not hold what it should, so it was left as it was.
archive-update-changed = it changed while it was being rewritten, so it was left as it was.
extract-list-too-long = it took too long to read.
extract-too-long = it took too long to extract.

## Copy, move and shortcut errors
move-failed-title = Error Moving File or Folder
copy-failed-title = Error Copying File or Folder
shortcut-failed-title = Error Creating Shortcut
move-failed = Cannot move { $name }: { $why }
copy-failed = Cannot copy { $name }: { $why }
shortcut-failed = Cannot create a shortcut to { $name }: { $why }
copy-cancelled = The copy was cancelled.
operation-cancelled = The operation was cancelled.
folder-too-deep = The folder is more than { NUMBER($depth, useGrouping: "false") } levels deep.

## Delete prompts
delete-file-title = Confirm File Delete
delete-folder-title = Confirm Folder Delete
delete-multiple-title = Confirm Multiple File Delete
delete-file-recycle = Are you sure you want to send '{ $name }' to the Recycle Bin?
delete-file-erase = Are you sure you want to delete '{ $name }'?
delete-folder-recycle = Are you sure you want to remove the folder '{ $name }' and move all its contents to the Recycle Bin?
delete-folder-erase = Are you sure you want to remove the folder '{ $name }' and all its contents?
delete-multiple-recycle = Are you sure you want to send these { NUMBER($count, useGrouping: "false") } items to the Recycle Bin?
delete-multiple-erase = Are you sure you want to delete these { NUMBER($count, useGrouping: "false") } items?
delete-no-bin-one = This drive has no Recycle Bin.
delete-no-bin-all = None of these items can go to the Recycle Bin.
delete-no-bin-some = Some of these items cannot go to the Recycle Bin.
# The lead sentence and the question, one after the other.
delete-prompt-lead = { $lead } { $question }
too-big-text = '{ $name }' is too big for the Recycle Bin. Do you want to delete it permanently?
undo-erase-many = The items Undo removes cannot go to the Recycle Bin. Do you want to delete them permanently?
undo-erase-one = '{ $name }' is on a drive that removes files immediately. Do you want to delete it permanently?
delete-failed = Cannot delete { $name }: { $why }

## File dialog and typed locations
folder-gone = The folder is no longer available.
not-in-recent = { $name } is not in { $place }. Type the full path to save or open it elsewhere.
folder-missing = The folder { $path } does not exist.
file-not-found = { $path } was not found. Check the file name and try again.
shortcut-no-program = Cannot find '{ $program }'. Check the spelling, then try again.
shortcut-no-location = Type the location of an item or a command.
device-no-folder = The device is mounted, but gvfs gives it no folder.

## A location that stops answering
not-responding = The location is not responding.
not-responding-unchanged = The location is not responding. Nothing was changed.
not-responding-may-change = The location is not responding. The change may still be made once it answers.

## Renaming
rename-bad-characters =
    A file name cannot contain any of the following characters:
    /
    A file name cannot contain the null character.
rename-invalid = Cannot rename { $old }: The file name you specified is not valid or too long.
rename-taken = Cannot rename { $old }: A file with the name you specified already exists. Specify a different file name.
rename-failed = Cannot rename { $old }: { $why }

## The question before running a script or program
ask-run = &Run
ask-open-in-notepad = Open in &Notepad
ask-script-text =
    "{ $name }" is a script. Running it can change or delete your files.

    Do you want to run it, or open it in Notepad to read it first?
ask-program-text =
    "{ $name }" is a program. Running it can change or delete your files.

    Do you want to run it?

## Other errors
exec-refused = Reprise cannot run '{ $name }' safely: its command line has a field code inside quotes.
link-no-program = Reprise cannot find the program '{ $name }'.
link-no-target = Reprise cannot find '{ $name }'.
no-executable-bit = This file system cannot mark files as executable.

## The infotips of the shell folders
place-tip-documents = Keeps your documents, pictures and other files together.
place-tip-computer = Shows the drives and devices attached to this computer.
place-tip-home = Holds your own files and settings.
place-tip-recycle-bin = Holds what you have deleted until you empty it.

## Ejecting a drive
eject-not-removable = This drive cannot be removed safely.
eject-holds-system = This drive holds part of the system, so it cannot be removed safely.
eject-held = Part of this drive is in use as swap space or by an LVM or RAID volume, so it cannot be removed safely.
eject-not-stopped = Its volumes were unmounted, but the drive could not be stopped and may still be in use.
eject-busy = '{ $volume }' is in use: a program is using a file on it. Close any program or window using it, and then try again.
eject-denied = You do not have permission to eject '{ $volume }'.
eject-failed = '{ $volume }' could not be ejected.
already-unmounted-one = '{ $name }' was already unmounted.
already-unmounted-many = { $rest } and '{ $last }' were already unmounted.

## File search
search-no-tool = File search needs plocate or fd.

## The proxy form
proxy-no-server = Type the address of at least one proxy server.
proxy-bad-ignore = The ignored hosts can't hold quotes, $ or \.
proxy-no-script = Type the address of the configuration script.
proxy-bad-script = The configuration script's address can't hold spaces, quotes, $ or \.
proxy-bad-address = The { $label } proxy's address can't hold spaces, quotes, $ or \.
proxy-bad-port = Type the { $label } proxy's port, a number from 1 to 65535.

## Properties sheet
# $type is the shared type's name.
props-all-of-type = All of type { $type }
props-multiple-types = Multiple Types
# $dir is the shared folder.
props-all-in = All in { $dir }
props-various-folders = Various Folders
# $ext is the upper-case file extension.
props-generic-type = { $ext } File
props-generic-file = File
# The Opens with line of a file whose type no program opens.
props-no-program = No program chosen
# $files and $folders are the grouped counts, $filesn and $foldersn the numbers that pick the plural.
props-contains =
    { $files } { $filesn ->
        [one] File
       *[other] Files
    }, { $folders } { $foldersn ->
        [one] Folder
       *[other] Folders
    }
