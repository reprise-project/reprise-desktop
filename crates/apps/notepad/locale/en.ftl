# Notepad. An & marks the next letter as the access key.

name = Notepad
untitled = Untitled
# The main window's title.
window-title = { $name } - Notepad
# The status bar; line and column are plain digits.
status-position = Ln { $line }, Col { $column }

## The menu bar
menu-file = &File
menu-new = &New
menu-open = &Open...
menu-save = &Save
menu-save-as = Save &As...
menu-page-setup = Page Set&up...
menu-print = &Print...
menu-exit = E&xit
menu-edit = &Edit
menu-undo = &Undo
menu-redo = &Redo
menu-cut = Cu&t
menu-copy = &Copy
menu-paste = &Paste
menu-delete = De&lete
menu-find = &Find...
menu-find-next = Find &Next
menu-replace = &Replace...
menu-go-to = &Go To...
menu-select-all = Select &All
menu-time-date = Time/&Date
menu-format = F&ormat
menu-word-wrap = &Word Wrap
menu-font = &Font...
menu-view = &View
menu-status-bar = &Status Bar
menu-help = &Help
menu-help-topics = &Help Topics
menu-about = &About Notepad

## Dialog titles and fields
title-go-to = Goto line
title-find = Find
title-replace = Replace
title-about = About Notepad
title-open = Open
title-save-as = Save As
go-to-line-number = &Line Number:
open-button = Open
save-button = Save
# A filter keeps its pattern in brackets: the file dialog matches names against it.
filter-text = Text Documents (*.txt)
filter-all = All Files (*.*)

## Message boxes
not-found = Cannot find "{ $query }"
changed-on-disk =
    { $name } has changed on disk since you opened it.
    Save anyway?
save-failed =
    Cannot create the { $path } file.
    Make sure that the path and file name are correct.
not-utf8 =
    { $path } is not a UTF-8 text file.
    Do you want to open a copy with the unreadable characters replaced?
recovered = A document was recovered. Open it?
save-changes =
    The text in the { $name } file has changed.
    Do you want to save the changes?
overwrite =
    { $path } already exists.
    Do you want to replace it?
open-not-found =
    { $path }
    File not found.
    Please verify the correct file name was given.
open-too-large =
    The file { $path } is too large for Notepad.
    Use another editor to edit the file.
open-not-a-file =
    Cannot open { $path }.
    It is not a file.
open-denied =
    Cannot open { $path }.
    Access is denied.
# error is the system's own message.
open-failed =
    Cannot open { $path }.
    { $error }
