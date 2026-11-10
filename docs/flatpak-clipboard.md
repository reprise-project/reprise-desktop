# Clipboard access for Flatpak apps

A Flatpak app copies and pastes while one of its windows has the focus,
like any other app. Reading the clipboard in the background, as a
clipboard manager or a clipboard sync tool does, takes the protocols
clipboard managers use, and Reprise hides those from sandboxed apps. For
the apps you trust, `[clipboard] trusted_apps` gives them back.

## Copy buttons

Ctrl+C in an app goes through the ordinary Wayland clipboard, which every
app has. Some apps copy another way. Proton Pass (`me.proton.Pass`) copies
through a native library (`arboard`) that tries the clipboard managers'
*data control* protocols (`ext_data_control_manager_v1` and
`zwlr_data_control_manager_v1`) first and, when they are hidden, falls back
to X11 through Xwayland.

Electron apps such as Proton Pass open an X11 window by default on Reprise,
and Reprise takes an X11 app's copy while an X11 window has the focus, so
their copy buttons work with no setting. Started with
`--ozone-platform=wayland`, the app's window is a Wayland one. For that case
Reprise also takes an X11 copy while a Wayland window of the same Flatpak app
has the focus: the click lands while its own window has the focus, and its
later clear of the clipboard is honoured whatever has the focus then. The rule
covers copying only: the app's X11 side reads the clipboard, and touches the
primary selection, only while an X11 window has the focus.

A copy made through X11 can be read by any X11 program at any time, until
it is cleared or replaced. The copying program holds it in the Xwayland
server, and Reprise doesn't stand between X11 programs. A Flatpak app that
uses X11 is not isolated from other X11 programs, sandboxed or not. For a
password manager that copies through X11 the choice is:

- No setting: every X11 program can read its copies until it clears them.
- Listed in `trusted_apps` (below): it copies through data control, so its
  copies stay on the Wayland clipboard, which X11 programs read only while an
  X11 window has the focus. It can then also read the clipboard in the
  background.

X11 doesn't say which program took the clipboard, only which window it named
as the owner, and a program may name any window. So while Proton Pass has the
focus, another X11 program that names one of Proton Pass's X11 windows can
replace or clear the clipboard too. While the clipboard holds an X11 copy,
Proton Pass's included, any X11 program that takes the clipboard replaces
what Wayland windows paste, whatever has the focus, while the paste still
offers the earlier copy's types and the clipboard history keeps showing the
earlier copy. See
[security.md](security.md#x11-programs).

## Background access

Data control can read the clipboard at any time, not only when the app has
the focus, so Reprise hides it from sandboxed apps, as sway and KWin do.
Flatpak marks its apps as sandboxed through `wp-security-context-v1`.

To give it to an app, tick the app in Settings > Privacy > Clipboard
Access. The list holds the installed Flatpak apps that can open a Wayland
window, and any id `trusted_apps` names that isn't installed, so it can be
unticked. Apply writes the ticks to `~/.config/reprise/config.toml`:

```toml
[clipboard]
trusted_apps = ["org.example.ClipboardManager"]
```

The value is the Flatpak app id, as `flatpak list --app` prints it. The
change applies to apps started after you save; restart the app if it is
already running. Reprise doesn't need a restart.

A listed app sees the two data control protocols and nothing else Reprise
hides from sandboxes: no screen capture, no input injection, no layer
shell. Unlisted apps see nothing new.

### What you are trusting it with

A listed app can read everything you copy, and the text you select, from
any app, whenever it runs, whether or not it has the focus. It can also
replace the clipboard at any time. Don't list an app you wouldn't trust
with that.

The app id comes from Flatpak, not the app, so an app can't pass itself off
as one on the list. Only Flatpak's sandboxes are matched: an app another
sandbox runs under the same id stays refused. Apps outside a sandbox
already see data control.

## Checking why an app is refused

Run Reprise with `RUST_LOG=reprise=debug`. Each time a sandboxed app is
refused data control, the log names the app:

    hiding ext_data_control_manager_v1 from sandboxed org.example.ClipboardManager; [clipboard] trusted_apps opens it
