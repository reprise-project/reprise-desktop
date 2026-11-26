# Image Viewer. An & marks the next letter as the access key.

name = Image Viewer
# The main window's title.
window-title = { $name } - Image Viewer

## The menu bar
menu-file = &File
menu-open = &Open...
menu-set-background = Set as Desktop &Background
menu-exit = E&xit
menu-edit = &Edit
menu-copy = &Copy
menu-delete = &Delete
menu-view = &View
menu-previous = &Previous Picture
menu-next = &Next Picture
menu-first = &First Picture
menu-last = &Last Picture
menu-zoom-in = Zoom &In
menu-zoom-out = Zoom &Out
menu-fit = Fit to &Window
menu-actual-size = &Actual Size
menu-full-screen = F&ull Screen
menu-slide-show = &Slide Show
menu-status-bar = Status &Bar
menu-image = &Image
menu-rotate-cw = Rotate &Clockwise
menu-rotate-ccw = Rotate C&ounterclockwise

## The Open dialog
title-open = Open
open-button = &Open
# A filter keeps its pattern in brackets: the file dialog matches names against it.
filter-pictures = All Pictures ({ $globs })
filter-all = All Files (*.*)

## The picture area and the status bar
no-picture = No picture is open.
cannot-open = This picture cannot be shown.
too-large = This picture is too large to show.
# The shown picture's place in its folder; both are plain digits.
position = { $at } of { $count }
# Width and height in pixels, plain digits.
dimensions = { $width } x { $height }
zoom = { $percent }%

## Message boxes
delete-failed = Cannot delete { $name }: { $error }
no-recycle-bin = Cannot find the Recycle Bin.
background-failed =
    The picture could not be set as the desktop background.

    { $error }
path-not-utf8 = The picture's path is not UTF-8.
