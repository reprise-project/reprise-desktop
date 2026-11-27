# Configuration

Reprise reads `~/.config/reprise/config.toml` and reloads whenever you save. Every key, valid value and default is documented in [`config.example.toml`](../config.example.toml) (installed at `/usr/share/doc/reprise/config.example.toml`). Omitted keys use their defaults.

Run `reprise --config FILE` to use another file. Child programs receive that path in `REPRISE_CONFIG`.

[Reprise Settings](programs.md#reprise-settings) edits most keys in the same file while keeping your comments and key order. Some defaults depend on the active interface ([interfaces.md](interfaces.md#defaults-by-interface)).

## If the file has an error

If a value is invalid, Reprise keeps the previous value for that key and shows a notice naming it.

If the file isn't valid TOML, Reprise ignores the whole save and keeps your previous settings. The notice gives the line and column. Click it to open the file.

If the file is broken when you log in, Reprise starts with the defaults.

## Sections

| Section | What it configures |
| --- | --- |
| `[appearance]` | `interface` and `color_scheme` |
| `[colors]`, `[metrics]` | Colour and size overrides over the active interface |
| `[fonts]`, `[icons]`, `[cursor]` | UI font, icon theme and pointer theme |
| `[toolkits]` | GTK and Qt theming |
| `[glass]` | Glass colour, material, mode, schedule, blur and `notification_style` |
| `[wallpaper]` | Image, slideshow and water or fluid effects ([below](#wallpaper)) |
| `[screensaver]`, `[idle]` | Screen savers and idle timeouts ([below](#screen-saver)) |
| `[power]` | Turn Off commands, lid, buttons, battery alarms and external locker |
| `[lock]` | Bluetooth away lock (`away_device`, `away_after_s`) and lock screen `media` |
| `[taskbar]` | Edge position, button grouping, pinned apps, auto-hide and per-monitor buttons |
| `[effects]` | Window, hover and taskbar animations, plus `reduce_motion` |
| `[switcher]`, `[overview]` | Alt+Tab, 3D switcher and window overview |
| `[workspaces]` | Virtual desktops, names, animation and desktop wheel switching |
| `[window_snapping]`, `[windows]` | Snapping, layouts, tabs, shake, window memory and inactive dimming |
| `[[window_rules]]` | Per-program size, position, screen, desktop and state (`from_menu = true`) |
| `[session]` | Session restore and leftover programs |
| `[updates]` | `restart_notice` for kernel updates or `/run/reboot-required` (on by default) |
| `[keyboard]`, `[mouse]`, `[touchpad]`, `[touchscreen]`, `[gestures]` | Input device settings |
| `[shortcut_overlay]` | Shortcut sheet shown when holding Super and its hold delay |
| `[[tablets]]`, `[[tablet_pads]]` | Tablet output mapping, aspect ratio, pad buttons, rings and strips |
| `[accessibility]` | Text size, StickyKeys, SlowKeys, BounceKeys, MouseKeys, magnifier and filters |
| `[night_light]` | Night light schedule and colour warmth |
| `[launcher]` | File, window and content search (content off by default) |
| `[clipboard]` | In-memory clipboard history and `trusted_apps` ([security.md](security.md#the-clipboard-history)) |
| `[screenshot]` | Folder, `format` (`png`, `jpg`, `jxl`), quality, `delay` (0 to 10 s), `record_sound`, `record_microphone` |
| `[previews]` | Live taskbar previews |
| `[notifications]` | Balloon sounds, quiet hours, Do Not Disturb and per-app overrides |
| `[sounds]` | Sound theme, bell, `events`, `allow_boost` (up to 150%) and `on_connect` |
| `[privacy]` | Recent files, location and screen-sharing border |
| `[time_zone]` | `automatic` location-based time zone (off by default) |
| `[folders]` | Explorer window mode, click mode, hidden files, `hide_extensions` and infotips |
| `[recycle_bin]` | Delete behaviour, per-drive limits and `older_than_days` |
| `[thumbnail_cache]` | Cache toggle, size cap and external thumbnailers |
| `[storage]` | Low disk space warning |
| `[bluetooth]` | `battery_warning` (default 15%, 0 off) and `receive_files` (off by default) |
| `[recording]` | Video and audio codecs, encoder, rate control and FPS ([below](#recording)) |
| `[remote_desktop]` | Port, quality, FPS, encoder, sound and lock behaviour ([below](#remote-desktop)) |
| `[debug]` | Debug overlay |
| `[osd]` | `caps_lock` and `num_lock` on-screen indicators (both on by default) |
| `[[keybinds]]` | Custom shortcuts layered over defaults (`action = "spawn"` or `"none"`) |
| `[[global_shortcuts]]`, `[portal]` | Portal global shortcuts and `shortcuts_denied` |
| `[[startmenu_changes]]`, `[[startmenu]]`, `startmenu_style` | Classic Start menu edits or full replacement, and Start menu style |
| `startmenu_places`, `startmenu_power`, `startmenu_programs` | Blue and Glass Start menus: the places on the right, the power button's action, the number of frequent programs |
| `[[outputs]]` | Per-monitor mode, scale, rotation, position, VRR and ICC profile ([features.md](features.md#displays)) |
| `[display]` | Startup `gpu`, `all_gpus` and built-in screen `auto_rotate` |
| `desktop_extensions` | Desktop file extensions: `hide` (default), `hide_launchers` or `show` |
| `desktop_widgets` | Desktop widgets: `clock`, `calendar`, `meter` (none by default) |
| `terminal`, `script_open`, `script_terminal`, `confirm_scripts` | Default terminal and script execution prompts |
| `task_manager` | Program launched by Ctrl+Shift+Escape and taskbar menu |

## Wallpaper

`[wallpaper]` accepts an image file or a built-in wallpaper name (listed in `config.example.toml`). Built-ins render at screen resolution and cache in `$XDG_CACHE_HOME/reprise/wallpapers`. Glass-tinted built-ins redraw when the Glass colour or mode changes.

Set `desktops` for per-desktop wallpapers (fading over `[workspaces] animation_ms`, or 250 ms without animation). Set `wallpaper` and `wallpaper_position` in an `[[outputs]]` entry to assign a monitor its own wallpaper, which overrides `image`, `desktops` and slideshows. Use Settings > Background > Use for to configure per-monitor or per-desktop images, and desktop menu > Next desktop background to advance a slideshow.

Desktop water ripples the wallpaper on pointer movement, clicks, icon drops and window actions, and pauses under fullscreen windows. `water_rate` controls the simulation frame rate.

The `ink`, `glow`, `sparks` and `neon` styles replace ripples with an interactive fluid simulation (requiring OpenGL ES 3 with half-float render targets):

- Drag with the left mouse button or fingers on empty desktop to stir (replaces desktop drag selection).
- Tap for a burst, hold to pour, flick for a jet, twist two fingers for a whirlpool, or double-click/double-tap to clear.
- Fluid drifts for about a minute after the last input, then fades back to the wallpaper. The lock screen keeps Calm ripples.

## Screen saver

`[screensaver] name` sets what `[idle] blank_after` starts: `blank` (default), `starfield`, `mystify`, `logo` or `photos`. `[idle] lock_after` locks underneath it. Fullscreen windows defer the screen saver, though lock, display off and suspend timers continue unless an app inhibits idle.

Tune savers with `speed` (1 to 10), `stars`, `shapes`, `cycle_colors`, `folder` (defaults to Pictures) and `photo_seconds`. `photos` falls back to `blank` while locked.

## Recording

`[recording]` sets screen recording codecs, encoder, rate control, frame rate and keyframe interval (read at the start of each recording). Run `reprise-record --probe` to see supported encoders ([programs.md](programs.md)). Configure `[recording]` and `[screenshot]` together in Settings > Screen Capture.

## Remote Desktop

`[remote_desktop]` configures the RDP server (enabled or disabled via Settings > Remote Desktop, see [remote-desktop.md](remote-desktop.md)). The server reloads on save, keeping previous values if the save is invalid. Under `reprise-login`, the login manager handles listening, so your user `port` is ignored ([Remote logins](#remote-logins)).

## Login manager

`/etc/reprise/login.toml` configures `reprise-login` ([install.md](install.md#use-the-login-manager)) with a `[login]` table and a system `[remote_desktop]` table. Reload changes with `sudo systemctl reload reprise-login` or apply everything with `sudo systemctl restart reprise-login` (running sessions stay alive). Check errors with `journalctl -b -u reprise-login`.

Settings > Login Manager edits this file via `pkexec reprise-login-settings` (rewrites without comments and refuses to overwrite concurrent changes).

| Key | Purpose | Default |
| --- | --- | --- |
| `interface` | `"classic"`, `"blue"` or `"glass"` ([features.md](features.md#login-screen)) | Last session's interface (`"classic"` first) |
| `show_users` | Show user tiles on Blue and Glass (`false` prompts for username) | `true` |
| `autologin_user` | Log this user in automatically when no session is running | unset |
| `greeter_socket` | Expose a Wayland socket for testing (unsafe on real machines) | `false` |
| `animation` | Animate login and lock screen transitions | `true` |
| `background` | `"reprise:design/variant"` or `"/var/lib/reprise-login/background"` | Interface default |

`autologin_user` logs in only when `reprise-login` starts with no running session. Logging out returns to the login screen. `greeter_socket = true` lets any process owned by `reprise-greeter` capture or control the login screen.

You can add up to 16 `[[outputs]]` tables in `/etc/reprise/login.toml` using `name`, `mode`, `scale` (1 to 4 in steps of 0.25), `transform`, `position` (within 65536 of origin) and `primary`. Settings > Login Manager > Use My Display Settings copies your current layout.

Login accounts are users with a UID between `UID_MIN` and `UID_MAX` in `/etc/login.defs` (default 1000 to 60000) and a valid shell in `/etc/shells` (not `nologin` or `false`), up to 100 listed. `reprise-login` stores the last user and interface in `/var/lib/reprise-login/state.toml`.

### Remote logins

`[remote_desktop]` in `/etc/reprise/login.toml` enables system-wide RDP logins ([remote-desktop.md](remote-desktop.md#remote-logins), [security.md](security.md#remote-logins)). Each user's `config.toml` `[remote_desktop]` keys (except `port`) still apply.

| Key | Purpose | Default |
| --- | --- | --- |
| `enabled` | Listen via `reprise-login-rdp.socket` and start `reprise-login-rdp` | `false` |
| `port` | TCP port on all interfaces (1 to 65535) | `3389` |
| `group` | Group allowed to log in remotely (root is never allowed) | `"reprise-remote"` |
| `max_sessions` | Maximum total sessions including local ones (1 to 50) | `8` |
| `end_disconnected_after` | Minutes before idle background sessions end (0 to 10080, 0 = never) | unset |

Reloading applies `enabled`, `port` and `end_disconnected_after` immediately, and other keys on the next login. Turning `enabled` off leaves existing connections active. Active console sessions are never ended by `end_disconnected_after`.

## Lock when away

Set `[lock] away_device` to a paired Bluetooth MAC address (`"AA:BB:CC:DD:EE:FF"`) or BlueZ object path, or pick one in Settings > Screen Saver & Lock. Once BlueZ sees it connected, disconnecting for `away_after_s` seconds (default 30, 0 locks immediately) locks the session.

The device must hold an active connection while nearby (such as a phone connected for audio or KDE Connect). Pairing alone drops the link when idle. Turning Bluetooth off, removing the adapter, stopping BlueZ or unpairing also triggers the lock. Away lock never unlocks and ignores idle inhibitors.

## Reduce motion

`[effects] reduce_motion = true` disables desktop animations, fades, slides, wallpaper crossfades, 3D switcher flight and smooth magnifier panning. Direct manipulation (window drags, trackpad swipes), progress dialogs, screen savers, slideshows, water/fluid wallpapers and `[idle] lock_fade` stay active.

Reprise also sets `org.freedesktop.appearance` `reduced-motion` to `1` and `org.gnome.desktop.interface` `enable-animations` to `false` in the settings portal. The login screen uses `[login] animation` separately.

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

Save and the changes apply immediately.
