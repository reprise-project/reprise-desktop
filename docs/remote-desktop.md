# Remote Desktop

Reprise has its own RDP server, `reprise-rdp`, built on
[IronRDP](https://github.com/Devolutions/IronRDP). Another computer can use the
session you're logged in to here, and the session moves between this screen
and the client as you go. Under Reprise's login manager the users you allow
can log in from another computer, several at once. Nothing listens until you
turn it on.

- **Your session, from another computer.** A client logs in with your
  account's name and password and gets the session that runs here, windows
  and all. Up to `max_viewers` computers (4 unless set) can be connected at
  once, and every one sees the screens and drives the keyboard and mouse.
- **Hand-off.** By default, connecting locks this computer's screen and
  moves your windows to screens made for the client, one per client
  monitor or one the size of its window. Unlocking here ends the
  connection and brings the windows back where they were. Disconnecting
  leaves the session running, locked here, and the next connection lands
  back in it ([Lock mode](#lock-mode-and-show-mode)).
- **Several users at once.** Under `reprise-login`, every member of the
  `reprise-remote` group can log in from another computer, whether they're
  logged in here or not. Someone who isn't gets a session in the
  background, and this computer's screen stays on the login screen or on
  whoever is using it. Logging in at the login screen brings that session
  to the monitor, and Switch User sends it back. Up to `max_sessions`
  sessions (8 unless set) run at once, the ones at the screen included
  ([Remote logins](#remote-logins)).
- **Video on the GPU.** The picture goes as H.264, coded through VA-API on
  Intel and AMD, NVENC on NVIDIA, or Vulkan, and through x264 on the CPU
  when none of them works. At Balanced and Sharp, on VA-API, NVENC or
  x264, a client that offers it gets full colour (AVC444), so small
  coloured text stays sharp. The server codes the two halves of that
  picture on two encoders and merges them into one stream. At Balanced and
  1920x1080 on VA-API the stream is about a sixth of the bytes of one
  encoder coding both ([Encoders](#encoders-and-picture-quality)).
- **The usual extras.** Several client monitors, the clipboard both ways for text
  and pictures, this computer's sound on the client, its keyboard layout
  for the client's keys, and a standby screen that keeps the desktop up
  when no monitor is plugged in.

## Turning it on

> [!WARNING]
> Remote Desktop is a network service that takes logins, and it has had no
> formal security review. Use it with care. Keep it on your home network,
> or reach it from outside through a VPN such as Tailscale or WireGuard.
> Don't open the port to the internet.

Reprise opens no firewall port and sets up no port forwarding. If you run a
firewall, allow the port only from the networks you connect from (your LAN,
or the `tailscale0` interface).

- **In your session.** Settings > Remote Desktop > Allow remote connections to
  this computer enables and starts the user unit
  `app-org.reprise.RemoteDesktop.service`; clearing it stops and disables
  the unit. With it off nothing listens. The line under the check box says
  where it listens ("Listening on port 3389 at 192.168.1.20"), and the next
  one which video encoder it uses.
- **Under reprise-login.** The login manager takes the connections, for
  every user in `reprise-remote`, and the session's own server never
  listens. [install.md](install.md#remote-logins) has the steps and
  [Remote logins](#remote-logins) says how logging in works there.

## Connecting

- **Remmina.** Make an RDP profile with the address and
  port from that line, your login name and your password. Set Security
  protocol negotiation to TLS protocol security and leave Colour depth at
  Automatic, which lets the client take H.264. The first connection shows
  the server's certificate: compare its fingerprint with the one on the
  Settings page before you accept it. Other FreeRDP 3 clients work the same
  way, for example `xfreerdp3 /v:HOST /u:USER /sec:tls`.
- **xfreerdp3 options.** `/multimon` puts each screen on a client monitor
  of its own, `/dynamic-resolution` resizes the client's screen with its
  window in lock mode, and `/sound:latency:20` shortens the sound's delay.
- **Login.** Which password you type depends on the client's security
  setting and on who takes the connection ([Which
  password](#which-password)). After a fourth wrong password from an
  address, each further try waits longer
  ([security.md](security.md#remote-desktop)).
- **Clients.** xfreerdp3, Remmina and mstsc have been tried. mstsc
  logs in with NLA, so it needs reprise-login and a Remote Desktop password.

## Which password

RDP clients log in one of two ways. With TLS the client opens an encrypted
connection first and then sends your name and password over it, so Reprise
can check them like any login. With NLA (Network Level Authentication) the
client proves it knows the password without sending it, and the server has to
hold a copy of the password in a form NLA can check against. Your login
password is stored in a form NLA can't use, which is why NLA needs a second
password.

| Who takes the connection | Client set to TLS | Client set to NLA |
|---|---|---|
| Your session's own server (Settings > Remote Desktop) | your login password | refused |
| `reprise-login`, you have no Remote Desktop password | your login password | refused |
| `reprise-login`, you have a Remote Desktop password | your login password | your Remote Desktop password |

- **Most clients offer both.** xfreerdp3 and mstsc offer TLS and NLA, and
  Reprise picks: NLA when the user name the client sends ahead belongs to
  someone with a Remote Desktop password, TLS otherwise. So with no Remote
  Desktop password, your login password works from any client that still
  allows TLS.
- **Clients that only do NLA** need `reprise-login` and a Remote Desktop
  password. With xfreerdp3, `/sec:nla` forces NLA and `/sec:tls` forces TLS.
- **A refusal says why.** The journal names the fix: choose TLS, or set a
  Remote Desktop password. Under `reprise-login` it logs the first refusal
  each minute in full and counts the rest in one line a minute.
- **Setting one.** [Remote Desktop password](#remote-logins) below says
  where; [security.md](security.md#remote-desktop-password) says how it is
  stored.

## On the client

- **The picture.** In lock mode the client sees the screens made for it,
  each on its own monitor or one in its window (see Lock mode below); a
  client without H.264 shows only one of them after a resize. In show mode
  the client sees every screen. Screens at a negative
  position in the layout are left out, unless none is at a non-negative
  one. A client with several monitors (`xfreerdp3 /multimon`) gets each
  screen on its own monitor, fitted to it with its shape kept: the screen
  at the top left of the layout goes on the client's primary monitor and
  the others follow, top then left. A client monitor left over stays black,
  and a client with fewer monitors than there are screens shows them all
  side by side, scaled together to fit. Client monitors that overlap or
  span more than 8192 pixels get the side-by-side picture. A client in one
  window or on one monitor sees every screen side by side as they are laid
  out here, each at its own size; a smaller window scales the picture if
  the client is set to. A side-by-side picture wider or taller than 8192
  pixels is scaled to fit. A client without H.264 gets that side-by-side
  picture too, cut at 8192 pixels instead of scaled. Unplugging one of this
  computer's screens reconnects the client to the screens left; one plugged
  in shows from the next connection. The lock screen shows too, and the
  client's keyboard unlocks it. The client draws the pointer itself, so it
  moves without lag. Only what changed is sent, and a still screen sends
  nothing.
- **No monitor.** While Remote Desktop is on and no monitor is connected
  (switched to another computer on a KVM, or never plugged in since boot),
  a standby screen holds the desktop. It runs at the last monitor's size,
  scale and place, else the first `[[outputs]]` entry's (the primary one if
  set), else 1920x1080 at scale 1, and windows stay where they were. It is
  drawn only for the viewers, at `frame_rate`, and once a second while none
  watches. Settings > Displays and output management tools don't list it.
  A monitor coming back takes its place; viewers reconnect once then. The
  standby takes the last monitor's place even away from the layout's
  origin. In lock mode the client's own screens hold the desktop instead.
- **Keyboard and mouse.** Keys go by position and this computer's keyboard
  layout reads them. Characters the client sends as text are typed as they
  are. Caps Lock and Num Lock follow the client; with several connected,
  the lock keys of the first computer to send them set the session's until
  it leaves. Five mouse buttons, the wheel and the horizontal wheel work. Keys
  and buttons held when the client leaves are let go. The client's Power,
  Sleep and Wake keys do nothing here.
- **Copy and paste.** Text and pictures copied on either side paste on the
  other while a client is connected, up to 16 MiB of text as RDP carries it
  (UTF-16, so about 8 MiB of plain ASCII) and 64 MiB of picture. Pasting
  something copied on the client ends empty if the client doesn't send it
  within 5 s. Files stay where they are. A password manager's copy stays out
  of the other side's clipboard history when the client passes its mark on;
  FreeRDP on Linux doesn't. While the session is locked (on the
  client's side in lock mode) nothing crosses either way and neither side's
  clipboard changes. On unlock the client's newest copy from the lock lands
  here, or else a copy made here during the lock reaches the client.
- **Sound.** With Play sound on the remote computer on (`sound`), this
  computer's sound plays on every connected client and not here. A "Remote
  client" output becomes the default while a client is connected; the one
  before comes back when the last leaves, unless you picked another
  meanwhile, and also when the server stops or, after a crash, when it next
  starts. A change while connected applies at once. Sound goes as Opus when
  the client takes it, else as 16-bit PCM at 44.1 kHz. The client's own
  buffering sets most of the delay: with FreeRDP's default it is about 110
  ms, and `/sound:latency:20` brings it to about 65 ms. In lock mode the
  "Remote client" output is the default also with sound off or a client
  that doesn't take it, so the session is never heard here while locked.

## Encoders and picture quality

- **Encoders.** At start the server tries each H.264 encoder on a test
  frame: VA-API (Intel and AMD), NVENC (NVIDIA; tried on a GeForce RTX
  4080 SUPER with driver 615.78), Vulkan and x264 on the CPU. Automatic
  uses the first of them
  that works. A client without H.264 gets RemoteFX bitmaps. Video encoder
  on the Settings page (`encoder`) picks one; those that don't work here
  are greyed. A chosen encoder that doesn't work here falls back to
  Automatic and the Video line says so ("VA-API H.264 (NVENC
  unavailable)"); one that fails while coding hands over to the next that
  works, and after the last the picture goes as bitmaps. None (RemoteFX)
  sends bitmaps only. A change while connected applies at the next frame.
  The test runs in the background; a client that connects before it is
  done waits for it, at most about 5 s, and an encoder that takes longer
  is left out.
- **Quality.** Picture quality runs from Speed to Sharp and frame rate is
  30, 60, 120 or 144, never more than the screen's refresh rate; both apply to a connected client at once. With Balanced or
  Sharp, a client that offers it and VA-API, NVENC or x264 coding, the
  picture is sent in full colour (AVC444), so small coloured text stays
  sharp. Each of its two halves is coded on an encoder of its own and the
  two go out as one stream, in the AVC444v2 layout where the coded width
  is a whole number of 32-pixel columns. At Balanced that takes about a
  sixth of the bandwidth of one encoder coding both on VA-API, two fifths
  on x264 and under half on NVENC; in the AVC444 layout, under a third on
  VA-API, two thirds on x264 and four fifths on NVENC. A GeForce runs 8
  NVENC sessions at once, so past four screens a connection codes both
  halves of each on one NVENC encoder. If the two won't merge, one
  encoder codes both until the encoder is next set up
  ([`avc444_split_encoders`](configuration.md#remote-desktop) turns the
  two encoders off). One encoder sends the AVC444 layout, never AVC444v2.
  Speed and Vulkan send reduced colour, which takes less bandwidth.

The Video line on Settings > Remote Desktop names the encoder in use, and
the journal logs each change ([Troubleshooting](#troubleshooting)).

## Lock mode and show mode

- **Lock mode.** While connected, this computer's screen, on the Settings
  page (`laptop_screen`), picks what this computer shows. With Locks and
  fits the remote window, the default, the session locks here when the first
  client is shown it, and the client works on screens of its own: one per
  client monitor at its size and scale, or one at the size of its window, to
  the right of this computer's screens. The windows move over to them.
  Resizing a client's window (`xfreerdp3 /dynamic-resolution`) resizes its
  screen without a reconnect, and the windows keep their place on it. A
  change in the client's number of monitors ends the connection; a client
  set to reconnect comes back with a screen for each. With several clients,
  the first one's monitors set the screens until the last one leaves. With
  `[power] locker` set, lock mode is off, connections run as Stays on, and
  Settings greys the Locks choice.
- **Locks in lock mode.** This computer's keyboard and mouse reach only
  its lock screen, and its screens turn off after the idle screen-off
  time without input here, whatever the client does; idle suspend waits.
  The client's screens have a lock of their own: they lock after
  `[idle] lock_after` without remote input, from Start > Lock or with the
  lock key, and their lock screen checks the password as a login does
  ([security.md](security.md#lock-mode)). A password that must be changed
  still unlocks them, and the notification history says it has expired. Unlocking at this computer, by
  password, fingerprint or `loginctl unlock-session`, ends lock mode: every
  client is disconnected, told another connection took over so it doesn't
  reconnect, and the windows come back here. When the last client leaves,
  the session stays locked here.
- **While a client is connected,** with Stays on (`laptop_screen =
  "show"`), this screen keeps showing the session and its own keyboard and
  mouse keep working. The screen saver and the idle lock keep their
  timeouts, and remote input counts as activity; screen off and idle
  suspend wait. In either mode the red dot shows the screen is shared. The
  red capture border stays off unless Show a red border while a remote
  computer is connected, under the list of connected computers, is on
  (`capture_border`); a screen shared another way keeps its border. Closing
  the lid turns the laptop's panel off instead of running
  `[power] lid_close`, and the session goes on; opening it turns the panel
  back on. When the last client leaves with the lid still shut, the lid
  action runs 30 s later.
- **Leaving locks.** In show mode, when the last client shown the session
  leaves, whether it disconnects or Disconnect ends it, the session locks; a
  client shown it before the lock goes ahead keeps it unlocked. A client
  that logged in but never got the picture leaves the session as it was,
  unless it logged in to replace a client shown the session
  (`second_client = "replace"`): it then counts as shown from its login, so
  the client it replaced doesn't lock the session as it leaves.
  Clearing Lock this computer when the last one disconnects, under the list
  of connected computers (`lock_on_leave`), leaves it unlocked; it is read
  as each client is shown the session and as it leaves. When the last client
  left because the screen it showed went (a monitor unplugged or switched
  away on a KVM, or one coming back in place of the standby screen), the
  lock waits Wait before locking on a screen change (`lock_grace_s`, 5
  seconds) for it to reconnect: a client shown the session by then keeps it
  unlocked, and Don't wait locks at once. In lock mode the session stays
  locked here all through, so a leave locks nothing and these two settings
  don't apply.

## Disconnecting

- **From the Settings page.** Connected computers on the Settings page lists every
  client; Disconnect ends the selected one. Turning Remote Desktop off ends
  every client. Each is told it was disconnected on purpose, so a client set
  to reconnect after a network drop stays off.
- **Disconnect in the session.** On the remote side of lock mode, and in a
  session off this computer's screen while a client drives it, the session
  offers Disconnect: in Switch User's place on the Log Off dialog, in Stand
  By's place on the Turn Off dialog, where it holds the focus (Shift then
  gives no Hibernate), and first in Glass's Shut down options, which then
  list no Sleep or Hibernate. It ends every
  client and leaves the session running, locked here; the next connection
  lands back in it.
- **Logging off with a client connected.** Log Off, Turn Off and Restart
  tell every client the session logged off, so it doesn't reconnect. The
  session waits for that while the screen fades, 1 s at most. A Turn Off or
  Restart that logind refuses after that leaves the session running, its
  programs closed and no client connected; connecting again lands in it.
- **What a client is told.** A client this side ends hears why before the
  connection closes: 0x5, another connection took over, when a newer client
  replaces it (`second_client = "replace"`) and when lock mode ends at this
  computer, by an unlock or by a login at the login screen; 0xB, the user
  disconnected it, for Disconnect in the session or on the Settings page and
  for a port change; 0xC, the session logged off, for Log Off, Turn Off and
  Restart; 0x1, the server ended it, when Remote Desktop is turned off.
- **More clients.** A computer that connects while `max_viewers` computers
  are connected or logging in is turned away at once, as is a third from an
  address with two still logging in. With `second_client = "replace"` a
  computer that logs in ends every other connection instead; in lock mode
  the session stays locked here and its screens take the new viewer's
  monitors. A connection that dies without closing ends within about 30 s.

## Remote logins

Under `reprise-login`, the computer takes RDP connections for every user in
the `reprise-remote` group, whether they are logged in or not.
[install.md](install.md#remote-logins) says how to turn it on; Settings >
Login Manager holds its settings, and an administrator's password saves
them, asked on each save.

- **Connecting.** Point the client at this computer, port 3389 unless set
  otherwise, with your user name and the password [Which
  password](#which-password) names. NLA takes the name as `user`, `.\user`, `HOST\user`, `user@host` or `user@`
  ([security.md](security.md#remote-desktop-password)).
  Only xfreerdp has been tried with NLA.
- **Remote Desktop password.** You set it in User Accounts, on the
  Password tab of your own account's Properties: Create a Remote Desktop
  password... asks for your login password and the new one twice; once you
  have one, Change my Remote Desktop password... does the same and Remove my
  Remote Desktop password... asks for your login password. It needs 8
  characters or more and must differ from your login password. It is kept as
  a weak hash, so make it strong
  ([security.md](security.md#remote-desktop-password)). Locking your login
  password doesn't clear it. Create and Change are greyed, with a note
  saying why, while remote logins are off, you aren't in the remote group
  or you are root, as the login manager answers; Remove stays open. Another
  account's Password tab says each user sets their own. Without
  reprise-login installed the section isn't shown. Settings > Remote
  Desktop says "Remote logins are set on the Login Manager page." in place
  of its on switch, and its User Accounts... button opens your account's
  Password tab.
- **Into your session.** If you're logged in, here or in the background,
  the client joins that session, even while your password is marked to be
  changed at the next login, and your Remote Desktop settings (this
  computer's screen, `second_client`, `max_viewers`, picture and sound)
  apply as they do to your own server. Your Remote Desktop page shows no
  port and greys New Certificate...: the computer has one certificate for
  every user, its fingerprint on both pages.
- **A session in the background.** If you're not logged in, your session
  starts on a VT of its own and this computer's screen stays as it was, on
  the login screen or someone else's session. The session is locked here
  (switching to its VT shows its lock screen), and its clients always
  work on screens of their own, as in lock mode. Several users can each have one, up to
  `max_sessions` sessions in all, console ones included. A password that must be changed
  starts none: log in at the computer first. Logging in at the
  login screen brings it to the monitor, and its windows go to this
  computer's screens, each at its place relative to the screen it was on.
- **And back.** Switch User, or a VT switch key, sends a session on the
  monitor to the background, locked. A remote login of its user lands in it,
  with no new session; each window goes back to the client screen in the
  same place in the client's layout (counted primary first) as the one it
  left, or to the client's primary when there is none, at its relative
  place, and this computer's screen stays locked.
- **Leaving.** Disconnecting, from the client or with Disconnect in the
  session, leaves the session running, and the next connection lands back
  in it. End disconnected sessions after (`end_disconnected_after`) logs a
  background session off that long after its last client left, unsaved
  work included; Never keeps it until you log off. A session sent back to
  the background counts as one once a remote login reaches it. The session
  this computer shows is never ended this way.
- **Power.** While a client is connected to any session, the computer
  doesn't sleep, and a shut lid darkens the panel instead of
  running its action, at the login screen too. A session in the background never suspends the
  computer on idle.
- **Refusals.** A refused login is answered 3 s after the client sent its
  password, whatever refused it, and the journal of `reprise-login` says
  why. Failed logins back off per address,
  as with Remote Desktop ([security.md](security.md#throttle)).

## Settings

Settings > Remote Desktop holds the server's settings
([programs.md](programs.md#reprise-settings)), and `[remote_desktop]` in
`config.toml` the same keys ([configuration.md](configuration.md#remote-desktop)).
Under reprise-login the machine's settings are on Settings > Login Manager
and in `/etc/reprise/login.toml`
([configuration.md](configuration.md#remote-logins)). How the server checks
passwords and what it refuses is in
[security.md](security.md#remote-desktop), with
[lock mode](security.md#lock-mode) and [remote logins](security.md#remote-logins)
after it.

## Troubleshooting

The server in your session logs to the user journal:

    journalctl --user -b -u app-org.reprise.RemoteDesktop

- `listening on ADDRESS`, or `cannot listen on port N: ...` when another
  program holds the port. Settings > Remote Desktop shows the same.
- `ADDRESS: connected`, `ADDRESS: logged in as USER`, `ADDRESS: wrong
  password for USER`, and a `disconnected` or `closed` line with the reason
  a connection ended.
- `ADDRESS: refused: the client requires NLA or RDP security; choose TLS
  security in its settings` means the client asked for NLA or RDP
  security. Set it to TLS ([Login](#connecting)).
- `ADDRESS: closed; IP has N s of back-off left after failed logins` means
  that address typed wrong passwords. Wait out the back-off
  ([security.md](security.md#remote-desktop)).
- `encoder NAME is unavailable here; using LABEL` at start, when the
  chosen encoder failed its test frame; `LABEL failed, using LABEL instead`
  and `H.264 failed, sending bitmaps from now on` when one fails while
  coding. [install.md](install.md#optional) lists the drivers each encoder
  needs.
- `client offers RDP version ..., build ...` and `client offers graphics
  capabilities: ...` say what each client offers.

Under reprise-login, the login manager and its front log to the system
journal, and each user's hand-over server to theirs:

    journalctl -b -u reprise-login -u reprise-login-rdp -u reprise-login-rdp.socket
    journalctl --user -b -u reprise-rdp-handover

The first holds the verdict on each login and the reason for a refusal.
[install.md](install.md#remote-logins) has the rest.
