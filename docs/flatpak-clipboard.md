# Clipboard access for Flatpak apps

Flatpak apps copy and paste normally while focused. Background clipboard access needs Wayland data control protocols, which Reprise hides from sandboxed apps. Add apps to `[clipboard] trusted_apps` to grant it.

## Copy buttons

`Ctrl+C` uses the regular Wayland clipboard. Some apps, such as Proton Pass (`me.proton.Pass`), use `arboard`, which tries `ext_data_control_manager_v1` and `zwlr_data_control_manager_v1` and falls back to X11 via Xwayland.

Electron apps open X11 windows by default on Reprise, so their copy buttons work out of the box. Under `--ozone-platform=wayland`, Reprise still accepts X11 copies while a Wayland window of the same Flatpak app has focus, and honours its later clear. Reading the clipboard or primary selection over X11 still requires X11 focus.

Any X11 app can read an X11 copy until cleared ([security.md](security.md#x11-programs)). For a password manager using X11 fallback:

- Unlisted: any X11 app can read its copies until cleared.
- Listed in `trusted_apps`: it copies over Wayland data control, which X11 apps read only while an X11 window has focus, and it gains background clipboard read access.

## Allow background access

Reprise identifies Flatpak sandboxes via `wp-security-context-v1`. Enable an app in Settings > Privacy > Clipboard Access, or in `~/.config/reprise/config.toml`:

```toml
[clipboard]
trusted_apps = ["org.example.ClipboardManager"]
```

Use the app ID from `flatpak list --app` and restart the app. Reprise reloads without restarting.

Listed apps receive only the two data control protocols, not screen capture, input injection or layer shell.

### What you trust a listed app with

A listed app can read everything you copy or select and replace the clipboard at any time, even without focus. Flatpak supplies the app ID, so apps cannot spoof it. Unsandboxed apps already have data control access.

## Check why an app is refused

Run Reprise with `RUST_LOG=reprise=debug` to log refusals:

    hiding ext_data_control_manager_v1 from sandboxed org.example.ClipboardManager; [clipboard] trusted_apps opens it
