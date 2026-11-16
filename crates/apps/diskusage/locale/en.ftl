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
refresh = &Refresh
explore = Open in &Explorer

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

## Message boxes
title-delete-failed = Error Deleting File or Folder
no-recycle-bin = there is no Recycle Bin
