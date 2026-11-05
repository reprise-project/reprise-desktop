# Paint. An & marks the next letter as the access key.

name = Paint
untitled = Untitled
window-title = { $name } - Paint

## The menu bar
menu-file = &File
menu-new = &New
menu-open = &Open...
menu-save = &Save
menu-save-as = Save &As...
menu-background = Set as &Desktop Background
menu-exit = E&xit
menu-edit = &Edit
menu-undo = &Undo
menu-redo = &Redo
menu-cut = Cu&t
menu-copy = &Copy
menu-paste = &Paste
menu-clear-selection = C&lear Selection
menu-select-all = Select &All
menu-view = &View
menu-zoom-in = Zoom &In
menu-zoom-out = Zoom &Out
menu-actual-size = &Actual Size
menu-grid = Show &Grid
menu-text-toolbar = &Text Toolbar
menu-status-bar = &Status Bar
menu-image = &Image
menu-flip-rotate = &Flip/Rotate...
menu-stretch = &Resize/Skew...
menu-invert = &Invert Colors
menu-attributes = A&ttributes...
menu-clear-image = &Clear Image
menu-crop = C&rop to Selection
menu-colors = &Colors
menu-edit-colors = &Edit Colors...
menu-help = &Help
menu-about = &About Paint

## The tool box: each tool's name, and its hint in the status bar.
tool-free-select = Free-Form Select
tool-select = Select
tool-eraser = Eraser
tool-fill = Fill With Color
tool-picker = Pick Color
tool-magnifier = Magnifier
tool-pencil = Pencil
tool-brush = Brush
tool-airbrush = Airbrush
tool-text = Text
tool-line = Line
tool-curve = Curve
tool-rect = Rectangle
tool-polygon = Polygon
tool-ellipse = Ellipse
tool-round-rect = Rounded Rectangle
hint-free-select = Outline any shape in the picture to move or copy it.
hint-select = Drag a box around part of the picture to move or copy it.
hint-eraser = Paints the second color over the picture; the right button only replaces the first color.
hint-fill = Floods a single-colored area with the color of the button pressed.
hint-picker = Takes a color from the picture: left button for the first color, right for the second.
hint-magnifier = Click the picture to look closer; click again to go back.
hint-pencil = Draws one pixel wide wherever the pointer goes.
hint-brush = Paints with the brush tip picked below.
hint-airbrush = Sprays dots while the button is held.
hint-text = Drag a box, then type; click outside the box to set the text.
hint-line = Drag to draw a straight line; hold Shift for level or 45° lines.
hint-curve = Drag a line, then drag once or twice to bend it.
hint-rect = Drag to draw a rectangle; hold Shift for a square.
hint-polygon = Click each corner; double-click to close the shape.
hint-ellipse = Drag to draw an ellipse; hold Shift for a circle.
hint-round-rect = Drag to draw a box with rounded corners; hold Shift for a square.

## Options well entries
option-width = { $n } pixel line
option-tip-round = Round, { $n } pixels
option-tip-square = Square, { $n } pixels
option-tip-slash = Slanted right, { $n } pixels
option-tip-backslash = Slanted left, { $n } pixels
option-eraser = Eraser, { $n } pixels
option-spray = Spray, { $n } pixels
option-outline = Outline
option-both = Outline with fill
option-fill = Fill
option-opaque = Opaque background
option-transparent = Transparent background
option-zoom = { $n }x

## The status bar
status-position = { $x },{ $y }
status-size = { $w }x{ $h }

## Attributes
title-attributes = Attributes
attr-width = &Width:
attr-height = &Height:

## Resize and Skew
title-stretch = Resize and Skew
stretch-resize = Resize
stretch-skew = Skew
stretch-horizontal = &Horizontal:
stretch-vertical = &Vertical:
stretch-skew-horizontal = H&orizontal:
stretch-skew-vertical = V&ertical:
stretch-percent = %
stretch-degrees = Degrees

## Flip and Rotate
title-flip = Flip and Rotate
flip-group = Flip or rotate
flip-horizontal = &Flip horizontal
flip-vertical = Flip &vertical
rotate-90 = Rotate &90°
rotate-180 = Rotate &180°
rotate-270 = Rotate &270°

## The text toolbar
title-fonts = Fonts
fonts-bold = &Bold

## Files
title-open = Open
title-save-as = Save As
open-button = &Open
save-button = &Save
filter-pictures = All Picture Files (*.png;*.jpg;*.jpeg;*.bmp;*.gif;*.webp)
filter-png = PNG (*.png)
filter-jpeg = JPEG (*.jpg;*.jpeg)
filter-bmp = Bitmap (*.bmp)
filter-all = All Files (*.*)
quality-label = &Quality:
quality-best = Best (100)
quality-high = High (90)
quality-good = Good (75)
quality-small = Small (50)
save-changes = Save changes to { $name }?
overwrite = { $path } already exists.
    Do you want to replace it?
open-failed = Paint cannot read this file.
    { $path }
    { $error }
save-failed = Paint cannot save { $path }.
    { $error }
background-failed = Paint cannot set the desktop background.
    { $error }
paste-failed = The clipboard holds no picture Paint can read.
bad-size = Please enter a whole number from 1 to { $max }.
draw-failed = Paint could not draw this window. { $error }
