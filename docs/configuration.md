# Configuration

Reprise reads `~/.config/reprise/config.toml`. Every key is documented in
[`config.example.toml`](../config.example.toml), and a key you leave out
takes its default. An edit applies the moment you save the file.

Most keys are also in [Reprise Settings](programs.md#reprise-settings), which
writes back into the same file and leaves your comments, key order and the
sections it doesn't touch alone. Use whichever you like. They don't fight.

Some defaults depend on the interface; [interfaces.md](interfaces.md#defaults-by-interface)
lists them. Glass's `[glass]` section is documented there too.

## A broken save

A key whose value Reprise can't use keeps its previous value (its default at
login) and a notice names it. A file that isn't TOML at all, such as an
unclosed quote or a table written twice, changes nothing: Reprise keeps the
settings from before, and a notice names the file, the line and column, and
what is wrong there. Clicking the notice opens the file in your text editor.
Saving the same error again doesn't repeat the notice, and the next save that
parses applies as usual. At login a broken file starts Reprise with the
defaults, with the same notice. A file that can't be read (its permissions, or
bytes that aren't UTF-8) gets a notice too; one that is gone for a moment
while an editor saves it doesn't.

## Sections

| Section | What it covers |
| --- | --- |
| `[appearance]` | `interface` and `color_scheme` |
| `[colors]`, `[metrics]` | single colours and sizes over the interface's own |
| `[fonts]`, `[icons]`, `[cursor]` | UI font, icon theme, pointer theme |
| `[toolkits]` | how GTK and Qt programs are themed |
| `[glass]` | Glass's colour, material, mode and its schedule, blur and notification_style |
| `[wallpaper]` | picture, slideshow, water |
| `[screensaver]`, `[idle]` | savers and the idle steps |
| `[power]` | Turn Off commands, lid and buttons, battery alarms, external locker |
| `[lock]` | `away_device`, `away_after_s`: lock when a paired Bluetooth device goes away ([features.md](features.md#lock-power-and-session)) |
| `[taskbar]` | position, button modes, pinned programs, auto-hide |
| `[effects]` | animations and fades |
| `[switcher]`, `[overview]` | Alt+Tab, the 3D switcher, the window overview |
| `[workspaces]` | virtual desktops, their names, their animation and the wheel over the desktop |
| `[window_snapping]`, `[windows]` | snapping, layouts, tabs, shake, window memory, title bar double click, dimming windows without the focus |
| `[[window_rules]]` | per-program size, place, screen (by connector or monitor id), desktop and state; `from_menu = true` marks the rule the window menu's Always Open Here wrote ([features.md](features.md#windows)) |
| `[session]` | session restore and leftover programs |
| `[updates]` | `restart_notice`: Software Updates says when a kernel update or `/run/reboot-required` calls for a restart, on by default (Settings > Advanced) |
| `[keyboard]`, `[mouse]`, `[touchpad]`, `[touchscreen]`, `[gestures]` | input |
| `[shortcut_overlay]` | the shortcut sheet holding Super shows, and how long Super is held first |
| `[[tablets]]` | which output a drawing tablet covers, and whether it keeps its shape |
| `[[tablet_pads]]` | what pad buttons, rings and strips do |
| `[accessibility]` | text size, StickyKeys, SlowKeys, MouseKeys, magnifier, colour filters |
| `[night_light]` | schedule and warmth |
| `[launcher]`, `[clipboard]`, `[screenshot]`, `[previews]` | the launcher's file and window search, clipboard history (in memory only, password managers' copies left out; see [security.md](security.md#the-clipboard-history)), where screenshots go, live previews |
| `[notifications]` | balloon sounds, quiet hours, Do Not Disturb during full screen and screen sharing, programs whose balloons or sounds are off |
| `[sounds]` | the sound theme balloons and the bell play from, whether the bell sounds, and `allow_boost`: playback volumes past 100 % up to 150 % |
| `[privacy]` | recent files and location |
| `[recycle_bin]` | what Delete does, each drive's bin size, `older_than_days` to empty old items (Recycle Bin Properties) |
| `[thumbnail_cache]` | whether thumbnails are kept, the cap, and whether installed thumbnailers make them for PDFs, videos and fonts |
| `[storage]` | the low disk space warning |
| `[bluetooth]` | `battery_warning`: the percent a connected device's battery warns at, 15 by default, 0 off (Bluetooth Devices > Options) |
| `[remote_desktop]` | the Remote Desktop server's port, picture quality, frame rate, video encoder, sound, lock on leave, capture border and what this computer's screen shows while connected ([below](#remote-desktop)) |
| `[debug]` | the debug overlay |
| `[osd]` | `caps_lock`, `num_lock`: whether the key's on or off shows on screen when it changes, both on by default (Settings > Keyboard) |
| `[[keybinds]]` | shortcuts, layered over the defaults; `action = "none"` unbinds a chord |
| `[[startmenu]]`, `startmenu_style` | the Classic Start menu's rows, and which Start menu you get |
| `[[outputs]]` | per-monitor mode, scale, rotation and position; a layout saved for the connected set of monitors wins over them ([features.md](features.md#displays)) |
| `[display]` | `gpu`: which GPU draws and drives the monitors; `all_gpus`: also open the other GPUs. Both read when Reprise starts |
| `desktop_widgets` | widgets on the desktop: `clock`, `calendar`, `meter`; none by default ([features.md](features.md#desktop)) |
| `terminal`, `script_open`, `script_terminal`, `confirm_scripts` | the terminal, how opening a script runs it, and whether opening a script, a binary or a file its program runs asks first |
| `task_manager` | what the taskbar menu's Task Manager row and Ctrl+Shift+Escape start; Reprise's Task Manager by default |

## Wallpaper

| Key | Values | Default |
| --- | --- | --- |
| `image` | a picture file, or a built-in as `reprise:design/variant` (below); `""` shows `[colors] background` alone | Classic none, Blue `reprise:tarn/blue`, Glass `reprise:flow/glass` |
| `position` | `center`, `tile` or `stretch`; a built-in always fills the screen | `stretch` |
| `slideshow` | a folder whose pictures take turns, fading into each other | none |
| `interval` | minutes per picture, 1 to 1440 | 30 |
| `shuffle` | random order instead of by name | `false` |
| `desktops` | each virtual desktop's own picture, spelled as `image` is: entry 1 is desktop 1; `""` shows the picture above | none |
| `water` | `off`, `calm`, `lively`, `rain`, `ink`, `glow`, `sparks` or `neon` | `off` |
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
one is ready the desktop shows its colour. Glass's lock screen, desktop water
and the Background page's monitor show a built-in like any picture; Classic's
and Blue's lock screens draw the welcome backdrop.

Desktop water ripples the wallpaper: the pointer leaves a wake over bare
desktop, and clicks, dropped icons and windows opening, closing or moving make
waves. Glass's lock screen picture ripples too. It pauses under a fullscreen
window. The screen draws the water only in the frames it steps in, so a
`water_rate` below the refresh rate draws fewer frames. Rain on its own draws
about 30 frames a second, each running the steps since the last, so its
ripples keep their speed; a wake, click or moving window brings back a frame
per step for two seconds. The desktop menu's Next desktop background skips the
slideshow ahead.

With `desktops`, a desktop with its own picture shows it in place of `image`
or the slideshow, and switching desktops fades from one picture to the next
over `[workspaces] animation_ms` (a quarter second with no animation). Only
the desktop on screen holds its picture: the next one is decoded or drawn when
the switch happens, so a desktop shows the picture before for that long.
`position` and `water` apply to every desktop.

`ink`, `glow`, `sparks` and `neon` swap the ripples for a fluid that the
pointer and fingers stir. Ink is soft coloured dye with a glow; it alternates
between a light flavour (deep colours on a pale backdrop) and a dark one (vivid
colours on a deep backdrop) each time the fluid starts fresh. Glow draws the
fluid as bright outlines with a shifting rainbow rim. Sparks is dim fluid
carrying glittering sparks. Neon is light streaks over a faint tile grid that
lights where the flow passes.

- Drag on empty desktop with the left mouse button or any number of fingers to
  stir; plain mouse movement leaves a faint wake. Each finger gets its own
  colour. The mouse's colour drifts along a stroke and each new press starts on
  the next colour. Icons work as usual, but dragging on empty desktop stirs
  instead of drawing a selection box.
- A tap makes a burst, a finger held still keeps pouring, a flick throws a long
  jet, two fingers twisting make a whirlpool, and a double tap (or a
  double-click on empty desktop) clears the fluid.
- While the fluid is live the wallpaper dims toward the style's backdrop. After
  the last touch the fluid keeps drifting for about a minute, then fades back to
  the wallpaper over about another minute and stops drawing.
- The lock screen keeps the Calm ripples. `water_rate` also sets the fluid's
  step rate.
- The fluid styles need OpenGL ES 3 with half-float render targets. Without
  them the desktop shows the plain wallpaper.

## Screen saver

`[screensaver] name` picks what `[idle] blank_after` starts: `blank` (the
default), `starfield`, `mystify`, `logo` or `photos`. It covers every screen
and hides the pointer until the next input, and `[idle] lock_after` locks the session underneath.
A fullscreen window holds the saver off and it counts again from when the
window leaves fullscreen; the lock, screen off and suspend steps keep their
timeouts unless a program inhibits idle.
`speed` (1 to 10), `stars`, `shapes`, `cycle_colors`, `folder` (for Photos;
Pictures when left out) and `photo_seconds` tune them. Photos runs as Blank
while the session is locked.

## Remote Desktop

`[remote_desktop]` configures the server; whether it runs is the enabled
state of its user unit, which Settings > Remote Desktop turns on and off
([remote-desktop.md](remote-desktop.md)). The server reads the section
again on every save. A missing key takes its default. An invalid key keeps
the value it had before the save (its default at startup), and the journal
names the invalid ones. A save it can't read (its permissions, or bytes that
aren't UTF-8) or that isn't TOML keeps the settings from before, so the port
and the connected computers stay, and the journal says why. A removed file
gives the defaults; at startup so does an unreadable one.
Under reprise-login the server never listens itself: the login manager
takes the connections and hands them to it, and `port` goes unused ([Remote
logins](#remote-logins)).

| Key | Values | Default |
| --- | --- | --- |
| `port` | 1 to 65535, the TCP port on every interface. A change moves the listener at once and ends every connected session; a port that can't be bound is tried again on the next save | `3389` |
| `quality` | `"speed"`, `"balanced"`, `"sharp"`: H.264 quantizer 30, 25 or 21. Balanced and Sharp send full colour (AVC444) to a client that offers it, Speed reduced colour (AVC420) | `"balanced"` |
| `frame_rate` | `30`, `60`, `120` or `144`, frames a second at most. The screen sends no more frames than its refresh rate, so the rate a client gets is the lower of the two | `60` |
| `encoder` | `"auto"`: the first of VA-API, NVENC, Vulkan and x264 that codes a test frame at start. `"vaapi"`, `"nvenc"`, `"vulkan"`, `"software"` (x264) pick one; one that doesn't work here falls back to `"auto"` with a journal line, and one that fails while coding hands over to the next that works, then to RemoteFX bitmaps. `"none"` sends RemoteFX bitmaps only | `"auto"` |
| `sound` | `true` plays this computer's sound on the client, not here; `false` plays nothing there | `true` |
| `max_viewers` | 1 to 16, how many computers may be connected at once, counting those still logging in; one more is closed before it logs in (with `second_client = "replace"`, one more is let in to replace them), as is a third from an address with two logging in and any from an address in its back-off after failed logins ([security.md](security.md#remote-desktop)). Read as each computer connects | `4` |
| `lock_on_leave` | `true` locks the session when the last logged-in computer leaves; `false` leaves it unlocked. Read as each computer logs in and leaves. In lock mode the session stays locked here when the last one leaves, whatever this says | `true` |
| `lock_grace_s` | 0 to 60: seconds the lock waits when the last computer left because the screen it showed went (a monitor switched away or back), so it can reconnect; one shown the session in that time keeps it unlocked. `0` locks at once. Any other leave locks at once. Read as each computer leaves; unused in lock mode | `5` |
| `capture_border` | `true` draws the red capture border around the screen the server shares while a computer is connected, if `[privacy] capture_border` is on; `false` leaves it off. The Sharing icon shows either way, and every other capture keeps its border. Read by the compositor on each save | `false` |
| `laptop_screen` | `"lock"`: while a computer is connected, this computer's screens show the session lock and only their lock screen takes this computer's keyboard and mouse; the viewer works on screens of its own, one per client monitor or one the size of its window. Unlocking here ends it and disconnects every computer. `"show"` keeps the session on this computer's screens as well. With `[power] locker` set, connections run as `"show"` and Settings greys the Locks choice. Read when a computer is shown the session while none is ([remote-desktop.md](remote-desktop.md#lock-mode-and-show-mode)) | `"lock"` |
| `avc444_split_encoders` | `true` codes the two full-colour (AVC444) views on two encoders of the same kind and merges them into one stream, a sixth to four fifths of the bytes of one encoder coding both depending on the encoder and layout ([remote-desktop.md](remote-desktop.md#encoders-and-picture-quality)), and in the AVC444v2 layout where the coded width is a whole number of 32-pixel columns; a merge that fails codes both views on one encoder until the encoder is next set up (a new frame rate or encoder choice), with a journal line. `false` codes both views on one encoder. One encoder always sends the AVC444 (v1) layout. Read as each computer is shown the session | `true` |
| `second_client` | `"join"`: a computer logging in while others are connected joins them. `"replace"`: it ends every other connection, each logged-in one told another connection took its place; in lock mode the screens stay locked here and take the new viewer's monitors. Read as each computer connects | `"join"` |

`quality`, `frame_rate` and `encoder` reach a connected client at the next
frame, and `sound` at once.

## Login manager

`/etc/reprise/login.toml` configures `reprise-login`, the login manager
([install.md](install.md#login-manager)), for the whole computer: the
`[login]` table and the machine's `[remote_desktop]` table below. It is read
when reprise-login starts and again on `sudo systemctl reload
reprise-login`. A reload applies `enabled`, `port` and
`end_disconnected_after` at once, the last counted from when each session's
last client left, or from the login that reached it while no client was
counted, the other `[remote_desktop]` keys from the next login,
`[login]` keys
the next time the login screen starts, and `autologin_user` the next time
reprise-login starts. `sudo systemctl restart reprise-login` applies
everything, and running sessions keep running. systemd starts it at most
twice in 30 s, so a third start within that time leaves a text login on
tty1 instead ([install.md](install.md#login-manager) says how to recover).
A missing file gives the defaults. A key with a value it can't use keeps
its default and an unknown key is skipped; a file that isn't TOML gives
every default. The journal names each (`journalctl -b -u reprise-login`).

Settings > Login Manager writes the file through `pkexec
reprise-login-settings`, which asks for an administrator's password on each save, refuses a file with an unknown key or a bad value, rewrites it from the
values it read (comments go) and reloads reprise-login. A file changed after
the page read it, by hand or from another session while the password prompt
was up, is not saved over: the page says so and shows the file as it is.

| Key | Values | Default |
| --- | --- | --- |
| `interface` | `"classic"`, `"blue"` or `"glass"`: the login screen's design ([features.md](features.md#login-screen)) | the `[appearance] interface` of the last session used at this computer under reprise-login; `"classic"` before any |
| `show_users` | `true` lists the accounts on the Blue and Glass screens, with each one's picture from AccountsService. `false` asks for a typed user name there, as Classic always does, and the screen gets no user list, Switch User's included | `true` |
| `autologin_user` | an account name. Each time reprise-login starts with no session running (one logind can't describe yet counts as running), it logs that user in without a password; a logout brings the login screen. A name that isn't a login account (below) shows the login screen, and the journal says why; `show_users` doesn't change which names it takes. `""` counts as unset | unset |
| `greeter_socket` | `true` lets the login screen listen on a Wayland socket in its runtime directory, so screenshot and input tools can reach it while testing. Any process of the `reprise-greeter` user can then reach the login screen: type into it, capture it, draw over it. Leave it `false` on a machine people use | `false` |
| `animation` | `false` shows each of the login screen's screens at once, with Glass's busy ring standing still, as the lock does under `[effects] logon_animation = false`. Settings > Login Manager > Animate the screen | `true` |
| `background` | what the login screen shows behind it in every interface: a built-in wallpaper's `"reprise:design/variant"` name (as `[wallpaper] image` takes), or `"/var/lib/reprise-login/background"`, the picture Settings > Login Manager > Browse... copied there. Any other path is refused, so the screen reads no file from a user's home. A picture missing or that doesn't decode leaves the default. A session's lock screen shows it too, read when the session starts and on each lock | Classic's desktop colour, Blue's welcome field, Glass's default wallpaper |

`[[outputs]]` tables in the same file set what the login screen runs on each
connector, with the `name`, `mode`, `scale`, `transform`, `position` and
`primary` keys of a session's `[[outputs]]`
([features.md](features.md#displays)). `scale` takes 1 to 4 in steps of 0.25, `position`
at most 65536 from the origin either way, and the file at most 16 tables;
past that every table is refused. A connector not listed, a turned-off
monitor included, runs its preferred mode at scale 1. Positions apply as
written whichever monitors are connected. The login screen goes on the
`primary` output while it is connected, else on the leftmost. Settings >
Login Manager > Use My Display Settings fills them from the screens as your
session runs them now, a saved monitor layout and its primary included, and
Reset removes them. Like `[login]`, they apply the next time the login
screen starts.

A login account has a uid between `UID_MIN` and `UID_MAX` of
`/etc/login.defs` (1000 and 60000 when unset) and a shell listed in
`/etc/shells` that isn't `nologin` or `false`. With `show_users` on, the
login screen lists them by name, at most 100.
Classic fills in the user who last logged in, and Glass with `show_users`
on opens on that user's picture. reprise-login keeps that user and the last interface in
`/var/lib/reprise-login/state.toml`.

### Remote logins

`[remote_desktop]` in `/etc/reprise/login.toml` lets reprise-login take RDP
connections and log their users in, into their running session or a new
one in the background ([remote-desktop.md](remote-desktop.md#remote-logins),
[security.md](security.md#remote-logins)). Each user's `[remote_desktop]`
keys in `config.toml`, `port` aside, still apply to their connections.

| Key | Values | Default |
| --- | --- | --- |
| `enabled` | `true` has systemd hold `port` from boot on (`reprise-login-rdp.socket`) and starts the front, `reprise-login-rdp`, on it; a port another program holds is named in reprise-login's journal, and each reload tries it again. `false` stops both, frees the port and refuses every login not yet handed to its session; clients already connected stay | `false` |
| `port` | 1 to 65535, the TCP port on every interface | `3389` |
| `group` | the group whose members may log in remotely. Root never can | `"reprise-remote"` |
| `max_sessions` | 1 to 50: the most sessions reprise-login runs, console ones included. A remote login that would start one more is refused | `8` |
| `end_disconnected_after` | 0 to 10080: minutes after its last client leaves, or after a login reaches it with no client counted, that a background session ends: one a remote login started, or one it reached while this computer showed another ([security.md](security.md#background-sessions)). The session on screen is never ended. `0` never ends it | unset (never) |

## Lock when away

`[lock] away_device` names a paired Bluetooth device, by address
(`"AA:BB:CC:DD:EE:FF"`) or BlueZ object path; empty (the default) turns it off.
Once BlueZ has seen it connected, it disconnecting for `away_after_s`
seconds (30 by default, 0 locks at once) locks the session. The Screen Saver
& Lock page of Reprise Settings lists the paired devices.

- The device must stay connected while it is near: a phone paired for calls
  or audio, or one that KDE Connect reaches over Bluetooth. A phone that is
  only paired drops the link when idle and locks you out each time. Signal
  strength is not used: BlueZ reports it only while scanning, and it jumps
  around.
- A device not connected when Reprise starts or the setting changes counts
  only once it connects.
- Bluetooth turned off, the adapter gone, BlueZ stopped and the device
  unpaired all count as gone.
- It never unlocks. Nothing holds it off: a playing video or another idle
  inhibitor holds off the idle lock only.

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
| `network_meter` | download and upload speed (`↓ 2.4 MB/s` over `↑ 120 KB/s`, in thousands of bytes) on two lines between the tray icons and the clock, read once a second while shown | `false` |
| `badges` | a program's unread count (LauncherEntry `count`) shows as a badge in its task buttons' top-right corner, `99+` above 99 | `true` |
| `auto_hide` | the bar slides away when nothing needs it | `false` |
| `reveal_on_edge` | the pointer resting on, clicking or dragging onto the 2 px line brings it back | `true` |
| `reveal_on_keys` | the Start key, Ctrl+Esc and Alt+Tab bring it back | `true` |
| `reveal_on_attention` | a window asking for attention or a new balloon brings it back | `true` |
| `attention_flash` | seconds a window's button flashes when it asks for attention before it stays lit until the window is focused; 0 lights it at once | 30 |

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

`startmenu_style` picks the Start menu: `classic`, `blue` or `glass`, by
default the interface's own. The Start Menu page of Reprise Settings edits the
Classic Start menu and the Quick Launch buttons. The buttons and their order live in
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
| `session_fade` | the screen fades in from black at log on and out to black when the session ends; the fade in waits a second after the display's mode is set, so a monitor syncing to it shows the whole fade | on |
| `session_fade_in_ms`, `session_fade_out_ms` | how long, 0 for at once | 600, 400 |
| `logon_animation` | the lock screen's transitions on Blue and Glass, its fade out after a right password or fingerprint, and Glass's busy ring turning; off, each screen shows at once, the ring stands still and an unlock takes the lock down at once. Settings > Effects > Animate the lock screen. The login screen follows `[login] animation` in `/etc/reprise/login.toml` | on |
| `reduce_motion` | the session's animations off or at once, whatever the keys above and `[workspaces] animation` say (see below). Settings > Effects > Reduce motion everywhere, also on Magnifier & Filters | off |

`reduce_motion = true` turns off window and notification animations, the
fade of the inactive-window dim, highlight fades, the taskbar's slides (task
buttons, auto-hide, the notification area's arrow), menu and Start menu
openings with the Start menu's crossfades, the screen's fade at log on and log
off, the lock screen's transitions and Glass's busy ring there (as
`logon_animation = false`), desktop switch animations (a lifted swipe lands at
once), the 3D switcher's flight and rolls, the snap bar's slide, wallpaper
crossfades, the grey fade behind Log Off and Turn Off and the magnifier's pan
to the caret, which jumps instead. Each cuts straight to its end. Settings
greys the keys it overrides. What moves because you move it stays: a swipe
follows your fingers and a dragged window your pointer. Reprise's progress
dialogs keep moving, and so do the screen savers and the slideshow's turns.
Desktop water, rain and the fluids keep running: they are a wallpaper you
picked, and `[wallpaper] water = "off"` stills them. `[idle] lock_fade` still
dims the screen before the lock, as the time to move the mouse. The settings
portal tells programs too: `org.freedesktop.appearance` `reduced-motion` is 1
and `org.gnome.desktop.interface` `enable-animations` is false. GTK programs
read `enable-animations`: GTK's own spinner stands still, while libadwaita
keeps its spinners turning. The login screen keeps its own `[login]
animation`.

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
```

Save, and watch it happen.
