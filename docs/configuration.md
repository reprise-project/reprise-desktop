# Configuration

Reprise reads `~/.config/reprise/config.toml`. Every key is documented in
[`config.example.toml`](../config.example.toml), and a key you leave out
takes its default. An edit applies the moment you save the file.

Most keys are also in [Reprise Settings](programs.md#reprise-settings), which
writes back into the same file and leaves your comments, key order and the
sections it doesn't touch alone. Use whichever you like. They don't fight.

Some defaults depend on the interface; [interfaces.md](interfaces.md#defaults-by-interface)
lists them. Glass's `[glass]` section is documented there too.

## Sections

| Section | What it covers |
| --- | --- |
| `[appearance]` | `interface` and `color_scheme` |
| `[colors]`, `[metrics]` | single colours and sizes over the interface's own |
| `[fonts]`, `[icons]`, `[cursor]` | UI font, icon theme, pointer theme |
| `[toolkits]` | how GTK and Qt programs are themed |
| `[glass]` | Glass's colour, material, mode and blur |
| `[wallpaper]` | picture, slideshow, lock screen picture, water |
| `[screensaver]`, `[idle]` | savers and the idle steps |
| `[power]` | Turn Off commands, lid and buttons, battery alarms, external locker |
| `[taskbar]` | position, button modes, pinned programs, auto-hide |
| `[effects]` | animations and fades |
| `[switcher]`, `[overview]` | Alt+Tab, the 3D switcher, the window overview |
| `[workspaces]` | virtual desktops and their animation |
| `[window_snapping]`, `[windows]` | snapping, layouts, tabs, shake, window memory |
| `[[window_rules]]` | per-program size, place, desktop and state |
| `[session]` | session restore and leftover programs |
| `[keyboard]`, `[mouse]`, `[touchpad]`, `[touchscreen]`, `[gestures]` | input |
| `[[tablet_pads]]` | what pad buttons, rings and strips do |
| `[accessibility]` | text size, StickyKeys, SlowKeys, MouseKeys, magnifier, colour filters |
| `[night_light]` | schedule and warmth |
| `[launcher]`, `[clipboard]`, `[screenshot]`, `[previews]` | the launcher's file search, clipboard history, where screenshots go, live previews |
| `[privacy]` | recent files and location |
| `[thumbnail_cache]` | whether pictures' thumbnails are kept, and the cap |
| `[debug]` | the debug overlay |
| `[[keybinds]]` | shortcuts, layered over the defaults; `action = "none"` unbinds a chord |
| `[[startmenu]]`, `startmenu_style` | the Classic Start menu's rows, and which Start menu you get |
| `[[outputs]]` | per-monitor mode, scale, rotation and position |

## Wallpaper

| Key | Values | Default |
| --- | --- | --- |
| `image` | a picture file, or a built-in as `reprise:design/variant` (below); `""` shows `[colors] background` alone | Classic none, Blue `reprise:tarn/blue`, Glass `reprise:flow/glass` |
| `position` | `center`, `tile` or `stretch`; a built-in always fills the screen | `stretch` |
| `slideshow` | a folder whose pictures take turns, fading into each other | none |
| `interval` | minutes per picture, 1 to 1440 | 30 |
| `shuffle` | random order instead of by name | `false` |
| `lock_screen` | the picture behind the lock dialog too | off under Classic, on under Blue and Glass |
| `water` | `off`, `calm`, `lively` or `rain` | `off` |
| `water_rate` | `refresh` (each screen's refresh rate) or 30 to 240 times a second | `refresh` |

Reprise draws its built-in wallpapers in code at each screen's size, so they
are sharp at any resolution. `reprise:tarn` alone is its first variant.

| Design | Variants |
| --- | --- |
| `tarn`, `ridges` | `dawn`, `day`, `dusk`, `night`, `blue` |
| `terraces` | `spring`, `harvest`, `dusk`, `frost`, `blue` |
| `caustic`, `reeded`, `panes` | `sky`, `ruby`, `green`, `violet`, `dark` |
| `flow` | `night`, `ink`, `teal`, `ember` |
| `fold` | `teal`, `steel`, `sand`, `night` |
| `contour` | `teal`, `night`, `paper`, `dusk` |

`caustic`, `reeded`, `panes` and `flow` also take any hue: `hue-200` or
`hue-200-dark` (OKLCH degrees, 0 to 359), and `glass`, the hue of
`[glass] color` (dark in Glass's dark mode, where `flow/glass` is
`flow/night`). A glass-tinted wallpaper is drawn again when the glass colour
or mode changes. Without an `image` key, Blue shows `tarn/blue` and Glass
`flow/glass`; Classic keeps its desktop colour. Drawn pictures are cached in
`$XDG_CACHE_HOME/reprise/wallpapers`, so a login shows them at once; until
one is ready the desktop shows its colour. The lock screen, desktop water and
the Background page's monitor show a built-in like any picture.

Desktop water ripples the wallpaper: the pointer leaves a wake over bare
desktop, and clicks, dropped icons and windows opening, closing or moving make
waves. The lock screen's picture ripples too. It pauses under a fullscreen
window. The screen draws the water only in the frames it steps in, so a
`water_rate` below the refresh rate draws fewer frames. The desktop menu's Next
desktop background skips the slideshow ahead.

## Screen saver

`[screensaver] name` picks what `[idle] blank_after` starts: `blank` (the
default), `starfield`, `mystify`, `logo` or `photos`. It covers every screen
and hides the pointer until the next input, and `[idle] lock_after` locks the session underneath.
A fullscreen window holds the saver off and it counts again from when the
window leaves fullscreen; the lock, screen off and suspend steps keep their
timeouts unless a program inhibits idle.
`speed` (1 to 10), `stars`, `shapes`, `cycle_colors`, `folder` (for Photos;
Pictures when left out) and `photo_seconds` tune them.

## Taskbar

| Key | Values | Default |
| --- | --- | --- |
| `position` | `bottom`, `top`, `left` or `right`, the same edge on every screen | `bottom` |
| `side_width` | a side bar's width in px; 0 takes the interface's own | 0 |
| `buttons` | `combine`, `when_full`, `never` or `group` | by interface |
| `pinned` | Quick Launch programs lead the task row | by interface |
| `show_desktop_button` | a Show desktop strip at the end | by interface |
| `peek` | resting on a preview or the strip fades the other windows | by interface |
| `small_icons` | a 30 px bar (Glass only) | `false` |
| `tray_overflow` | quiet tray icons wait behind an arrow or a ^ | `true` |
| `auto_hide` | the bar slides away when nothing needs it | `false` |
| `reveal_on_edge` | the pointer resting on, clicking or dragging onto the 2 px line brings it back | `true` |
| `reveal_on_keys` | the Start key, Ctrl+Esc and Alt+Tab bring it back | `true` |
| `reveal_on_attention` | a window asking for attention or a new balloon brings it back | `true` |

The button modes:

- `combine`: one icon-only button per program.
- `when_full`: one labelled button per window until the row is full, then a
  program's windows share one.
- `never`: one labelled button per window, in the order they opened.
- `group`: as `never`, with a program's buttons kept side by side. Dragging
  one moves them all.

With the taskbar unlocked, dragging empty bar space to another edge writes
`position`, and dragging a side bar's inner edge writes `side_width`. A side
bar stacks its buttons one a row and puts the ones that don't fit behind a »
button.

The Start Menu page of Reprise Settings edits the Classic Start menu and the
Quick Launch buttons. The buttons and their order live in
`~/.config/reprise/quicklaunch/`.

## Effects

| Key | What | Default |
| --- | --- | --- |
| `window_animation` | windows fade and grow in, shrink out | by interface |
| `window_open_ms`, `window_close_ms` | how long | 250, 200 |
| `window_scale` | the size they grow from | 0.95 |
| `hover_fade` | caption buttons and Start fade their highlight | by interface |
| `hover_in_ms`, `hover_out_ms` | how long | 120, 250 |
| `taskbar_slide` | task buttons slide aside during a drag | by interface |
| `taskbar_hide_ms` | how long auto-hide takes, 0 for at once | 200 |

## Things to try

```toml
[appearance]
interface = "glass"

[glass]
color = "ruby"
material = "molten"
mode = "dark"

[taskbar]
position = "left"
auto_hide = true

[wallpaper]
water = "rain"

[session]
restore = true
```

Save, and watch it happen.
