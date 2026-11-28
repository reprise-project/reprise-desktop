# Protocols and portals

Reprise is built on Smithay and renders with OpenGL ES. It follows freedesktop conventions for the tray, notifications, screen saver inhibition, autostart, trash and `.desktop` files, and serves standard `wlr` and `ext` Wayland protocols.

Minimized windows and hidden tabs receive the xdg-shell `suspended` state unless shown in a live preview. Windows requesting server-side decorations (or drawing to buffer edges with no shadow) receive Reprise frames. Client-decorated windows with their own shadows (such as GTK4 or libadwaita header bars) keep their own title bars.

## Third-party tools that work

| Tool | Protocol |
| --- | --- |
| Panels and docks | `layer-shell`, `foreign-toplevel`, `ext-workspace` |
| Lockers (swaylock, hyprlock, gtklock) | `ext-session-lock` (set `[power] locker`) |
| Idle daemons (swayidle) | `ext-idle-notify` |
| Display tools (wlr-randr, kanshi) | `wlr-output-management` |
| Output power (wlopm) | `wlr-output-power-management` |
| Gamma tools (wlsunset, gammastep) | `wlr-gamma-control` (overrides night light) |
| Clipboard managers | `ext-data-control`, `wlr-data-control` |
| Screenshot tools (grim) | `wlr-screencopy`, `ext-image-copy-capture` |
| Virtual input (wtype, wlrctl) | `virtual-keyboard`, `wlr-virtual-pointer` |
| Input methods (fcitx5, ibus) | `input-method-v2`, `text-input-v3` |

Sandboxed apps (via `security-context-v1` or Flatpak) do not see privileged protocols: layer-shell, session lock, foreign-toplevel, workspaces, output management, gamma, screen capture, data control, virtual input and input methods. To grant clipboard protocols to a Flatpak, see [flatpak-clipboard.md](flatpak-clipboard.md). See [security.md](security.md) for what each protocol allows.

Virtual keyboard input acts like hardware keys, including key binds and menus, except at the lock screen or during input capture.

## Wayland globals

- **Core and shell.** `wl_compositor`, `wl_subcompositor`, `wl_shm`, `wl_seat`, `wl_output`, `zxdg_output_manager_v1`, `wl_data_device_manager`, `zwp_linux_dmabuf_v1`, `xdg_wm_base`, `zxdg_decoration_manager_v1`, `org_kde_kwin_server_decoration_manager`, `xdg_activation_v1`, `xdg_wm_dialog_v1`, `xdg_toplevel_tag_manager_v1` (remembers `file-dialog` size and position per app; see [security.md](security.md#focus) for `device-prompt`), `xdg_toplevel_icon_manager_v1`, `zxdg_exporter_v1`/`zxdg_importer_v1`, `zxdg_exporter_v2`/`zxdg_importer_v2`, `xdg_toplevel_drag_manager_v1`, `xwayland_shell_v1`, `xdg_system_bell_v1` (plays `[sounds] theme` bell unless `[sounds] bell` is off and flashes unfocused task buttons).
- **Selection.** `zwp_primary_selection_device_manager_v1`, `ext_data_control_manager_v1`, `zwlr_data_control_manager_v1`.
- **Surfaces.** `wp_viewporter`, `wp_fractional_scale_manager_v1`, `wp_single_pixel_buffer_manager_v1`, `wp_content_type_manager_v1`, `wp_alpha_modifier_v1`, `wp_presentation`, `wp_linux_drm_syncobj_manager_v1` (if GPU supports it), `wp_fifo_manager_v1` and `wp_commit_timing_manager_v1` (hidden windows commit at refresh rate or `[windows] hidden_frame_rate`), `ext_background_effect_manager_v1` (extends Glass blur into framed windows).
- **Input.** `zwp_pointer_constraints_v1`, `zwp_relative_pointer_manager_v1`, `zwp_pointer_gestures_v1`, `wp_cursor_shape_manager_v1`, `zwp_tablet_manager_v2`, `zwp_keyboard_shortcuts_inhibit_manager_v1`, `zwp_xwayland_keyboard_grab_manager_v1` (Xwayland only), `zwp_text_input_manager_v3`, `zwp_input_method_manager_v2`, `zwp_virtual_keyboard_manager_v1`, `zwlr_virtual_pointer_manager_v1`, `wp_pointer_warp_v1`.
- **Desktop tools.** `zwlr_layer_shell_v1`, `ext_session_lock_manager_v1`, `ext_foreign_toplevel_list_v1`, `zwlr_foreign_toplevel_manager_v1`, `ext_workspace_manager_v1`, `zwlr_output_manager_v1`, `zwlr_output_power_manager_v1`, `zwlr_gamma_control_manager_v1`.
- **Idle.** `ext_idle_notifier_v1`, `zwp_idle_inhibit_manager_v1`.
- **Capture.** `zwlr_screencopy_manager_v1`, `ext_image_copy_capture_manager_v1`, `ext_output_image_capture_source_manager_v1`, `ext_foreign_toplevel_image_capture_source_manager_v1`.
- **Security.** `wp_security_context_manager_v1`.
- **Reprise protocols.** `reprise_input_v1` (installed `reprise-portal` only: libei, input capture, global shortcuts, RemoteDesktop input, colour picker, RDP capture, lock mode since v9, per-app sender/keys since v10, pointer keys since v12, Screenshot `pick_area`/`active_window` since v13), `reprise_tabs_v1` ([spec](../protocols/reprise-tabs-v1.xml)), `reprise_on_top_v1` (unsandboxed only, [spec](../protocols/reprise-on-top-v1.xml)), `reprise_reopen_v1` (unsandboxed only, [spec](../protocols/reprise-reopen-v1.xml)), `reprise_caption_v1` (unsandboxed only: a caption without the window icon and title, [spec](../protocols/reprise-caption-v1.xml)) and `reprise_output_trial_v1` (1 to 60 s display trial with automatic revert, [spec](../protocols/reprise-output-trial-v1.xml)).

## Portals and D-Bus services

`reprise-portal` implements these xdg-desktop-portal interfaces:

- **ScreenCast.** Screen or single window, zero-copy where supported, with optional Remember.
- **Screenshot.** Full layout, or interactive Screen, Window, Area (`Shift+PrtSc` style) and Active Window. Files save to `$XDG_RUNTIME_DIR/reprise-portal/screenshots` and expire after an hour. `PickColor` opens the colour picker.
- **RemoteDesktop.** Keyboard and pointer control.
- **InputCapture.** Barrier-based pointer and keyboard capture (for Deskflow).
- **GlobalShortcuts.** App-registered global shortcuts.
- **Access.** Generic permission prompt.
- **FileChooser and AppChooser.** Reprise Open, Save and Open With dialogs with thumbnail views and per-app folder, view, sort, size and position memory.
- **Notification.** Desktop notifications.
- **Inhibit.** Idle, suspend and log off inhibition.
- **Background.** Background app status and permission prompts (xdg-desktop-portal writes Flatpak autostart entries).
- **Wallpaper.** Prompts with a preview (unless previously allowed, see [security.md](security.md)) and saves to `~/.local/share/reprise/wallpapers` as `[wallpaper] image`. Lock-screen-only requests are refused.
- **Email.** Opens an unsent message in your Default Programs mail client (Thunderbird `-compose`, Evolution `mailto:` with `attach=`, or `xdg-email`).
- **Account.** Prompts to share your name, login and AccountsService picture (`file://` URI).
- **DynamicLauncher.** Prompts with icon, editable name and web app URL, then writes to `~/.local/share/xdg-desktop-portal/applications` and links into `~/.local/share/applications`. `RequestInstallToken` is restricted to Reprise programs.

The compositor serves **Settings** (light/dark, contrast, fonts, icons). `reprise-portals.conf` delegates **Secret** to `gnome-keyring`, **Print** to GTK, falls back to `wlr` for screen sharing, and sets no **Lockdown** backend (`[privacy] location` controls location).

Reprise also hosts `org.freedesktop.Notifications`, `StatusNotifierWatcher`, `org.freedesktop.ScreenSaver` and the GeoClue2 agent. Explorer's `reprise-file-manager.service` hosts `org.freedesktop.FileManager1` (`ShowFolders`, `ShowItems`, `ShowItemProperties`). Reprise supports `com.canonical.Unity.LauncherEntry` (`progress`, `progress-visible`, `progress-paused`, `count`, `count-visible`, `urgent`) on matching task buttons.

## The session

Reprise exports its environment to the systemd user manager and D-Bus activation environment, starts `reprise-session.target` (bound to `graphical-session.target`), and cleans up on exit. `loginctl lock-session` locks the screen.

A login-manager session owns the shared user manager, portals and D-Bus names. If you start a second Reprise session from a TTY while another Reprise session runs, the second session leaves those shared services with the primary session until the primary session exits.

XDG autostart honours `OnlyShowIn`, `NotShowIn`, `Hidden` and `TryExec`, and skips entries marked `X-systemd-skip=true` or `X-GNOME-HiddenUnderSystemd=true` when a systemd user unit under `reprise-session.target` runs the program. The Start menu and Open With honour `OnlyShowIn`, `NotShowIn` and `TryExec` against `XDG_CURRENT_DESKTOP`.
