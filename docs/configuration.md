# Configuration

Reprise reads `~/.config/reprise/config.toml`. Every key, its values and
its default are documented in [`config.example.toml`](../config.example.toml)
(installed as `/usr/share/doc/reprise/config.example.toml`), and a key you
leave out takes its default. An edit applies the moment you save the file.
This page indexes the sections and covers what doesn't fit beside a key.

`reprise --config FILE` reads another file. The programs Reprise starts read
that file too: they get its path as `REPRISE_CONFIG`.

Most keys are also in [Reprise Settings](programs.md#reprise-settings), which
writes back into the same file and leaves your comments, key order and the
sections it doesn't touch alone. Use whichever you like. They don't fight.

Some defaults depend on the interface; [interfaces.md](interfaces.md#defaults-by-interface)
lists them.

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
| `[power]` | Turn Off commands, lid and buttons, battery alarms, the low battery power saver, external locker |
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
| `[accessibility]` | text size, StickyKeys, SlowKeys, BounceKeys, MouseKeys, finding the pointer, visual alerts, magnifier, colour filters |
| `[night_light]` | schedule and warmth |
| `[launcher]`, `[clipboard]`, `[screenshot]`, `[previews]` | the launcher's file, window and content search (contents off by default) and the programs it asks, clipboard history (in memory only, password managers' copies left out; see [security.md](security.md#the-clipboard-history)), where screenshots go, the saved file's format (`format`: `png`, `jpg` or `jxl`, with `png_compression`, `jpg_quality`, `jxl_lossless`, `jxl_quality` and `jxl_effort`; the clipboard always takes a PNG), how many seconds a screenshot key waits before the picture (`delay`, 0 to 10, a longer one cut to 10) and whether screen recordings carry sound (`record_sound`, off) with the microphone mixed in (`record_microphone`, off), live previews |
| `[notifications]` | balloon sounds, quiet hours, Do Not Disturb during full screen and screen sharing, programs whose balloons or sounds are off |
| `[sounds]` | the sound theme balloons and the bell play from, whether the bell sounds, and `allow_boost`: playback volumes past 100 % up to 150 % |
| `[privacy]` | recent files, location and the red border around a shared or recorded screen |
| `[time_zone]` | `automatic`: the time zone follows the location, off by default (Date and Time > Time Zone) |
| `[recycle_bin]` | what Delete does, each drive's bin size, `older_than_days` to empty old items (Recycle Bin Properties) |
| `[thumbnail_cache]` | whether thumbnails are kept, the cap, and whether installed thumbnailers make them for PDFs, videos and fonts |
| `[storage]` | the low disk space warning |
| `[bluetooth]` | `battery_warning`: the percent a connected device's battery warns at, 15 by default, 0 off; `receive_files`: whether paired devices may offer files, off by default (Bluetooth Devices > Options) |
| `[recording]` | the screen recordings' video codec, encoder, rate control, frame rate, keyframe interval and sound codec ([below](#recording)) |
| `[remote_desktop]` | the Remote Desktop server's port, picture quality, frame rate, video encoder, sound, lock on leave, capture border and what this computer's screen shows while connected ([below](#remote-desktop)) |
| `[debug]` | the debug overlay |
| `[osd]` | `caps_lock`, `num_lock`: whether the key's on or off shows on screen when it changes, both on by default (Settings > Keyboard) |
| `[[keybinds]]` | shortcuts, layered over the defaults; `action = "spawn"` with `command` runs a program, `action = "none"` unbinds a chord. config.example.toml lists every action |
| `[[global_shortcuts]]`, `[portal] shortcuts_denied` | apps' shortcuts through the GlobalShortcuts portal and the apps refused them, written by the portal and Settings > Keyboard |
| `[[startmenu_changes]]`, `[[startmenu]]`, `startmenu_style` | the Classic Start menu's rows, as changes to the default menu (what Settings > Start Menu writes, so later default rows still reach you) or as the whole menu (`startmenu = []` for an empty one), which wins and stays whole until Restore Defaults on that page takes it out, and which Start menu you get |
| `[[outputs]]` | per-monitor mode, scale, rotation, position, adaptive sync and ICC profile; a layout saved for the connected set of monitors wins over all but the profile ([features.md](features.md#displays)) |
| `[display]` | `gpu`: which GPU draws and drives the monitors; `all_gpus`: also open the other GPUs. Both read when Reprise starts. `auto_rotate`: the built-in screen turns with the machine |
| `desktop_widgets` | widgets on the desktop: `clock`, `calendar`, `meter`; none by default ([features.md](features.md#desktop)) |
| `terminal`, `script_open`, `script_terminal`, `confirm_scripts` | the terminal, how opening a script runs it, and whether opening a script, a binary or a file its program runs asks first |
| `task_manager` | what the taskbar menu's Task Manager row and Ctrl+Shift+Escape start; Reprise's Task Manager by default |

## Wallpaper

`[wallpaper]` takes a picture file or one of the built-in wallpapers;
config.example.toml lists its keys and the built-ins' names. Reprise draws
the built-ins in code at each screen's size, so they are sharp at any
resolution. A glass-tinted one is drawn again when the glass colour or mode
changes. Drawn pictures are cached in `$XDG_CACHE_HOME/reprise/wallpapers`,
so a login shows them at once; until one is ready the desktop shows its
colour. Glass's lock screen, desktop water and the Background page's monitor
show a built-in like any picture; Classic's and Blue's lock screens draw the
welcome backdrop.

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

A monitor can have a picture of its own: `wallpaper` in its `[[outputs]]`
entry, matched by connector name, shows on every desktop in place of `image`,
the desktop's own picture and the slideshow, and `wallpaper_position` places it
(else `position` does). A monitor unplugged and plugged in again shows its own
picture again. Monitors placing the same file alike share one decode. In
Settings > Background, Use for lists each connected monitor while two or more
are connected: a picture or position picked with a monitor selected is that
monitor's own, and one picked for all monitors (the first row, Everywhere when
desktops are listed too) takes the connected monitors' own away; a monitor not
connected keeps its own. Under a slideshow, a desktop or monitor picked there
without a picture of its own has no picture row picked and its preview shows
the slideshow: the picture the desktop shows now while the folder picked is the
one applied, else the folder's first picture. The page's Restore Defaults takes
the connected monitors' own away too.

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

## Recording

`[recording]` sets how screen recordings are coded; its keys are in
config.example.toml. The recorder reads it as each recording starts. A file
it can't read or that isn't TOML gives the defaults.
`reprise-record --probe` prints what codes here
([programs.md](programs.md)). Settings > Screen Capture sets every key of
`[recording]` and `[screenshot]`'s `save`, `delay`, `record_sound`,
`record_microphone`, `format` and the format's compression.

## Remote Desktop

`[remote_desktop]` configures the server; whether it runs is the enabled
state of its user unit, which Settings > Remote Desktop turns on and off
([remote-desktop.md](remote-desktop.md)). The server reads the section
again on every save. A missing key takes its default. An invalid key keeps
the value it had before the save (its default at startup), and the config
notice names it. A save it can't read (its permissions, or bytes that
aren't UTF-8) or that isn't TOML keeps the settings from before, so the port
and the connected computers stay, and the journal says why. A removed file
gives the defaults; at startup so does an unreadable one.
Under reprise-login the server never listens itself: the login manager
takes the connections and hands them to it, and `port` goes unused ([Remote
logins](#remote-logins)).

Its keys are in config.example.toml, and
[remote-desktop.md](remote-desktop.md) says what each does to a connection.

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
A missing file gives the defaults. A key with a value it can't use takes
its default at start-up and keeps its last value on a reload, so a typo in
`enabled` doesn't turn remote logins off; `reprise-login-rdp` reads the
file afresh when it starts, so one started then stays off. An unknown key
is skipped. A file that isn't TOML or can't
be read gives every default at start-up, and a reload of it changes nothing:
the last settings stay. The journal names each (`journalctl -b -u
reprise-login`).

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

## Reduce motion

`[effects] reduce_motion = true` turns off window and notification animations, the
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
