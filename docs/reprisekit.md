# reprisekit

`reprisekit` is the Rust UI toolkit behind Reprise's bundled apps.

`reprisekit` draws controls (buttons, check boxes, inputs, menus, list and tree views, property sheets, dialogs and status bars) in Classic, Blue and Glass. Each control provides a model, layout, `draw` into a pixel buffer, hit test and accessibility node, with no display server dependency.

`repriseapp` runs a `reprisekit` program as a Wayland window. Implement `App` (`title`, `app_id`, `initial_size`, `event`, `draw`), add an empty `impl Extended` and call `repriseapp::run` (see `examples/hello.rs`). It handles input, clipboard, caret blinking, dialogs, HiDPI, screen readers and live theme changes.

## Run the examples

    cargo run -p reprisekit --example gallery      # draw controls to a PNG in target/
    cargo run -p repriseapp --example hello        # label and button window
    cargo run -p repriseapp --example controls     # live form window
    cargo run -p repriseapp --example twowin       # main window with dialog

The `repriseapp` examples run on any Wayland compositor.
