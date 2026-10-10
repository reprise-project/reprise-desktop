# Clipboard access for Flatpak apps

Some Flatpak apps copy through the protocols clipboard managers use, and
Reprise hides those protocols from sandboxed apps. For the apps you trust,
`[clipboard] trusted_apps` gives them back.

## The symptom

A Flatpak app's copy button does nothing. Pasting gives you whatever was
on the clipboard before. Ctrl+C on selected text in the same app still
works.

Proton Pass (`me.proton.Pass`) is the known case: clicking a password or
username in it copies nothing.

## Why it happens

Ctrl+C in an app goes through the ordinary Wayland clipboard, which every
app has. Some apps copy another way instead. Proton Pass copies through a
native library (`arboard`) that uses the clipboard managers' *data control*
protocols (`ext_data_control_manager_v1` and
`zwlr_data_control_manager_v1`), the ones `wl-copy` and `cliphist` use.

Data control can read the clipboard at any time, not only when the app has
the focus, so Reprise hides it from sandboxed apps, as sway and KWin do.
Flatpak marks its apps as sandboxed through `wp-security-context-v1`.

Without data control, `arboard` falls back to X11 through Xwayland. Reprise
only takes an X11 app's clipboard while an X11 window has the focus, and
the app's own window is a Wayland one, so the copy is dropped.

KDE Plasma hides data control from Flatpak apps too, but KWin accepts an
X11 app's clipboard whichever window has the focus, so the fallback works
there.

## The fix

Name the app in `~/.config/reprise/config.toml`:

```toml
[clipboard]
trusted_apps = ["me.proton.Pass"]
```

The value is the Flatpak app id, as `flatpak list --app` prints it. The
change applies to apps started after you save; restart the app if it is
already running. Reprise doesn't need a restart.

A listed app sees the two data control protocols and nothing else Reprise
hides from sandboxes: no screen capture, no input injection, no layer
shell. Unlisted apps see nothing new.

### What you are trusting it with

Data control is not limited to copying. A listed app can read everything
you copy, and the text you select, from any app, whenever it runs, whether
or not it has the focus. It can also replace the clipboard at any time.
That is a reasonable thing to allow a password manager you already give
your passwords to. Don't list an app you wouldn't trust with that.

The app id comes from Flatpak, not the app, so an app can't pass itself off
as one on the list. Only Flatpak's sandboxes are matched: an app another
sandbox runs under the same id stays refused. Apps outside a sandbox
already see data control.

## Checking why an app is refused

Run Reprise with `RUST_LOG=reprise=debug`. Each time a sandboxed app is
refused data control, the log names the app:

    hiding ext_data_control_manager_v1 from sandboxed me.proton.Pass; [clipboard] trusted_apps opens it

## Without changing the config

Proton Pass copies fine when it runs under X11 altogether, since its window
then has the X11 focus:

    flatpak run me.proton.Pass --ozone-platform=x11

It can look blurry with fractional scaling. Don't remove its Wayland socket
(`flatpak override --nosocket=wayland`) to get the same effect: it then
fails to start.
