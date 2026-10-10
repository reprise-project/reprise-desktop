# reprisekit

Reprise's controls come from `reprisekit`, a Rust toolkit you can use in your
own programs. Explorer, Notepad and Reprise Settings use it too, so your app
can look like it belongs on the same machine, in whichever interface the
user picked.

There are two parts:

- **`reprisekit`** draws the controls: buttons, check boxes, edit fields,
  menus, list and tree views, tabbed property sheets, file dialogs, message
  boxes, status bars and more, in Classic, Blue and Glass. Each control is a
  plain model with a layout, a `draw` into a pixel buffer, a mouse hit test
  and an accessibility node. There's no display server code in it, which
  makes it straightforward to test down to the pixel. A bevel is either
  right or it isn't.
- **`repriseapp`** runs a `reprisekit` program as a normal Wayland window.
  Implement the `App` trait (`title`, `event`, `draw`) and call
  `repriseapp::run`. It handles keyboard and mouse input, the clipboard,
  caret blinking, extra dialog windows, HiDPI and screen readers. It follows
  the desktop's interface and colour scheme, dark ones included.

To have a look:

    cargo run -p reprisekit --example gallery      # every control, drawn to a PNG in target/
    cargo run -p repriseapp --example controls     # a live window with a form
    cargo run -p repriseapp --example twowin       # a main window that opens a dialog

The two `repriseapp` examples need a running Wayland session. Any compositor
will do, though I am obviously going to recommend this one.
