# Protocols and portals

Reprise is built on Smithay and draws with OpenGL ES. It follows the
freedesktop conventions for the tray, notifications, screen saver
inhibition, autostart, the trash and `.desktop` files, and it speaks the
usual `wlr` and `ext` protocols, so third-party tools work.

Windows that ask for server-side decorations get Reprise's frames in the
current interface. Programs that draw their own title bars may look
different. They've brought their own furniture.

## Tools that work

| Tool | Through |
| --- | --- |
| Panels and docks | layer-shell, foreign-toplevel, ext-workspace |
| Lockers (swaylock, hyprlock, gtklock) | ext-session-lock; `[power] locker` runs one |
| Idle daemons (swayidle) | ext-idle-notify |
| Display configurators (wlr-randr, kanshi) | wlr-output-management |
| wlopm | wlr-output-power-management |
| Gamma tools (wlsunset, gammastep) | wlr-gamma-control; they take precedence over the night light |
| Clipboard managers | ext-data-control, wlr-data-control |
| Screenshot tools (grim) | wlr-screencopy, ext-image-copy-capture |
| Virtual input (wtype, wlrctl) | virtual-keyboard, wlr-virtual-pointer |
| Input methods (fcitx5, ibus) | input-method-v2, text-input-v3 |

Sandboxed programs, identified through `security-context-v1` or as Flatpaks,
don't see the privileged protocols: layer-shell, session lock,
foreign-toplevel, workspaces, output management, gamma, screen capture,
data control, virtual input and input methods. A Flatpak you trust can be
given the clipboard protocols with `[clipboard] trusted_apps`; see
[flatpak-clipboard.md](flatpak-clipboard.md). [security.md](security.md)
says what each privileged protocol lets a program do.

A virtual keyboard's keys go where the keyboard's would: they fire key
binds, open and drive menus and dialogs, and a screen saver Preview takes
them. At the lock screen and while an input capture holds the keyboard, the
compositor drops them.

A client hears pointer and drag motion at most once per refresh of the
screen the pointer is on, at the latest position, with `zwp_relative_pointer_v1` deltas
summed since the last one. A button, wheel, gesture, key or focus change
always reaches it after the motion it happened at. While a pointer lock or
confinement holds, every motion goes through as it comes.

## Wayland globals

- **Core and shell:** `wl_compositor`, `wl_subcompositor`, `wl_shm`,
  `wl_seat`, `wl_output`, `zxdg_output_manager_v1`, `wl_data_device_manager`,
  `zwp_linux_dmabuf_v1`, `xdg_wm_base`, `zxdg_decoration_manager_v1`,
  `org_kde_kwin_server_decoration_manager`, `xdg_activation_v1`,
  `xdg_wm_dialog_v1`, `xdg_toplevel_tag_manager_v1` (a toplevel tagged
  `file-dialog` opens at the size and place the last file dialog was left
  at), `xdg_toplevel_icon_manager_v1`, `zxdg_exporter_v1`/`zxdg_importer_v1`,
  `zxdg_exporter_v2`/`zxdg_importer_v2`, `xwayland_shell_v1`,
  `xdg_system_bell_v1` (a ring plays the `[sounds] theme`'s bell unless
  `[sounds] bell` is off, at most one per 100 ms, and flashes the task button of
  an unfocused window it names)
- **Selection:** `zwp_primary_selection_device_manager_v1`,
  `ext_data_control_manager_v1`, `zwlr_data_control_manager_v1`
- **Surfaces:** `wp_viewporter`, `wp_fractional_scale_manager_v1`,
  `wp_single_pixel_buffer_manager_v1`, `wp_content_type_manager_v1`,
  `wp_alpha_modifier_v1`, `wp_presentation`, `wp_linux_drm_syncobj_manager_v1`
  (when the GPU supports it), `wp_fifo_manager_v1` and
  `wp_commit_timing_manager_v1` (a window no display shows, minimised or on
  another desktop, has its fifo and timed commits applied at the refresh rate,
  so a vsynced Vulkan program keeps running), `ext_background_effect_manager_v1`
  (under Glass, a framed window's blur region gets the frame's glass, blurred
  and tinted, and a region across the window's top runs the frame down into it
  unbroken)
- **Input:** `zwp_pointer_constraints_v1`, `zwp_relative_pointer_manager_v1`,
  `zwp_pointer_gestures_v1`, `wp_cursor_shape_manager_v1`,
  `zwp_tablet_manager_v2`, `zwp_keyboard_shortcuts_inhibit_manager_v1`,
  `zwp_xwayland_keyboard_grab_manager_v1` (Xwayland only),
  `zwp_text_input_manager_v3`, `zwp_input_method_manager_v2`,
  `zwp_virtual_keyboard_manager_v1`, `zwlr_virtual_pointer_manager_v1`,
  `wp_pointer_warp_v1` (within the window under the pointer, while no grab
  but a button's own holds it)
- **Desktop tools:** `zwlr_layer_shell_v1`, `ext_session_lock_manager_v1`,
  `ext_foreign_toplevel_list_v1`, `zwlr_foreign_toplevel_manager_v1`,
  `ext_workspace_manager_v1`, `zwlr_output_manager_v1`,
  `zwlr_output_power_manager_v1`, `zwlr_gamma_control_manager_v1`
- **Idle:** `ext_idle_notifier_v1`, `zwp_idle_inhibit_manager_v1`
- **Capture:** `zwlr_screencopy_manager_v1`, `ext_image_copy_capture_manager_v1`,
  `ext_output_image_capture_source_manager_v1`,
  `ext_foreign_toplevel_image_capture_source_manager_v1`
- **Security:** `wp_security_context_manager_v1`

## Portals

`reprise-portal` is the xdg-desktop-portal backend:

- **ScreenCast:** a screen or a single window, zero-copy where the GPU
  allows, with an optional Remember
- **Screenshot:** every screen in one picture at its place in the layout, or
  for an interactive call the screen or window picked in the share dialog;
  the files are a hand-over in `$XDG_RUNTIME_DIR/reprise-portal/screenshots`,
  deleted after an hour. Also the colour picker
- **RemoteDesktop:** keyboard and pointer
- **InputCapture**, for Deskflow
- **GlobalShortcuts**
- **Access**, the generic permission prompt
- **FileChooser** and **AppChooser:** Reprise's own Open, Save and Open With
  dialogs. Their Tiles and Thumbnails views show pictures as Explorer does,
  from the same thumbnail cache, and they open in the view, sort, size and
  place the last Open or Save As dialog was left in
- **Notification**
- **Inhibit:** idle, suspend and log off
- **Background:** which apps run without a window and the prompt asking
  whether one may. xdg-desktop-portal writes the autostart entries Flatpak
  apps ask for itself
- **Wallpaper:** after a prompt showing the picture, a copy of it in
  `~/.local/share/reprise/wallpapers` becomes `[wallpaper] image`, and with
  `set-on` both also turns `lock_screen` on. The lock screen alone is
  refused, since it shows the desktop's picture or none

The compositor serves **Settings** (the light or dark look, contrast, fonts
and icons). `reprise-portals.conf` hands **Secret** to gnome-keyring,
**Print** to the GTK backend, and falls back to the wlr backend for screen
sharing. It names no **Lockdown** backend, so GNOME's lockdown keys (location,
camera, microphone, printing) don't apply and `[privacy] location` alone turns location off.

Reprise also hosts `org.freedesktop.Notifications`, the
StatusNotifierWatcher for the tray, `org.freedesktop.ScreenSaver` and the
GeoClue2 agent. It follows `com.canonical.Unity.LauncherEntry`'s `Update`
signal: `progress` and `progress-visible` draw on the task buttons of the
windows whose app id or desktop file matches the entry's
`application://<id>.desktop`, and Reprise's own `progress-paused` key shows the
progress as paused.

## The session

Reprise publishes its session to the systemd user manager and the D-Bus
activation environment, and takes it back out when it exits, so the next
desktop you log into starts clean. Once the variables are published it starts
`reprise-session.target`, which binds `graphical-session.target`, so user units
wanted by or `PartOf` that target run with the session; Log Off and exit stop
it. `loginctl lock-session` locks the screen.
XDG autostart honours `OnlyShowIn`, `NotShowIn`, `Hidden` and `TryExec`.
An entry marked `X-systemd-skip=true` or `X-GNOME-HiddenUnderSystemd=true`
whose program a user unit of the session runs (one `reprise-session.target`
pulls in, such as `spice-vdagent.service`) is left to that unit and not listed
on the Startup page; with no such unit it starts like any other entry.
