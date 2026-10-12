# Assets

What Reprise draws itself, what it borrows from your system, and what it
leaves out on purpose.

Nearly everything on screen comes from code in this repository or from
software you already have installed. Anything loaded from your system keeps
its own license.

## Made in this repository

These share the project's license, GPL-3.0-or-later:

- **Controls and chrome drawn in code:** Reprise Blue and Reprise Glass hold
  no bitmaps. Every part is drawn at the output scale from shapes (rounded
  boxes, discs, gradients and stroked glyphs, in `crates/reprisekit/src/shape/`)
  and named colours: Blue's from its seven schemes in
  `crates/reprisekit/src/blue/scheme/`, Glass's from the `[glass]` settings.
  That covers caption buttons, check marks, radio buttons, arrows and other
  glyphs; Blue's frames, taskbar, Start button, tray and controls; Glass's
  frames, Start orb, taskbar, Start menu, controls, tooltip, preview popup,
  shadows and reflection streaks; the Background page's monitor, colour
  spectrum and desktop preview; and the GTK and Qt themes Reprise writes so
  programs follow the look.
- **Small icons:** the PNGs in `crates/reprisekit/src/art/icons/`, Reprise's
  own pixel artwork, loaded through `art::icons`: the tray's network, battery
  and Bluetooth icons, its built-in indicators (StickyKeys, MouseKeys, the
  notification bell, the camera and the location pin), the Bluetooth device
  icons, the Views button, the printer badges, the shortcut arrow and the
  delete badge. Signal bars, the battery's fill and the bell's count are
  drawn in code over them.
- **The Reprise logo:** the R mark, the app icons in `data/icons/hicolor/`,
  the masks in `crates/reprisekit/src/art/logo/` and the lockups in
  `docs/logo/`, all written by `scripts/logo/build.py`. The lockups carry
  Adwaita Sans glyphs as outlines, under the SIL Open Font License.
- **Screenshots** in `docs/screenshots/`.
- **`protocols/reprise-input-v1.xml`**, Reprise's own Wayland protocol.

## Fonts

Reprise asks fontconfig for the families in `[fonts] family`, in order. The
default is `["Tahoma", "DejaVu Sans"]`, and any other font works too.
Layouts are tuned for 11 px text, so a font with other metrics can put a
label a pixel or two off.

| Font | Used for | Without it | License |
| --- | --- | --- | --- |
| DejaVu Sans | the fallback for everything | | Bitstream Vera plus public domain; packaged everywhere (`ttf-dejavu` on Arch) |
| Tahoma | the UI | DejaVu Sans | Wine's version is LGPL-2.1-or-later (`ttf-tahoma` on Arch, AUR) |
| a second dialog font, named in `[fonts] shell_dlg` by family or `.ttf` path | the dialogs set in a wider face: End Program, Power Options, Sounds and Audio Devices, Volume Control, a connection's Status, Notepad's Find and Replace, folder Properties | `[fonts] family` | proprietary |
| Trebuchet MS | Reprise Blue's caption titles | DejaVu Sans | proprietary |
| Franklin Gothic Medium | Blue's Start label (italic), and the Log Off and Turn Off panels' titles in every interface (regular) | Trebuchet MS Bold (Italic), then DejaVu Sans | proprietary |
| Segoe UI | Reprise Glass's window titles | Noto Sans, then DejaVu Sans | proprietary |

Reprise doesn't ship, download or point you to the proprietary fonts. If you
hold a licence for one, put its `.ttf` files in `~/.local/share/fonts`.

## Icons

Beyond its logo and the small icons above, Reprise draws no icon artwork.
It loads icons by their freedesktop names from `[icons] theme`, and each
theme has its own license.

- The default, `hicolor`, has very few icons of its own. Set a full theme.
- For the intended look, pick a theme drawn in a classic, early-2000s style:
  32 px desktop icons, 16 px menu icons, bright colours and soft shading.
  Set `[icons] theme` to its directory name under `/usr/share/icons` or
  `~/.local/share/icons`, or pick it on the Fonts & Icons page.
- Adwaita, Breeze and Papirus look out of period but cover almost every name
  Reprise asks for.
- A missing icon falls back to a drawn generic glyph. A few spots have no
  fallback yet and show a magenta square with a black border, on purpose: it
  marks missing art, and the fix is a freedesktop icon name that fits.

## Colour schemes

The dark Classic schemes borrow their palettes from these editor themes. The
names belong to those projects; Reprise uses the colours, not their code or
artwork. Thanks to [Ayu](https://github.com/ayu-theme/ayu-colors),
[Catppuccin](https://catppuccin.com) (Mocha),
[Dracula](https://draculatheme.com),
[GitHub Dark](https://github.com/primer/github-vscode-theme),
[Gruvbox](https://github.com/morhetz/gruvbox), Monokai by Wimer Hazenberg,
[Night Owl](https://github.com/sdras/night-owl-vscode-theme),
[Nord](https://www.nordtheme.com),
[One Dark](https://github.com/atom/one-dark-syntax) and
[Tokyo Night](https://github.com/enkia/tokyo-night-vscode-theme).
`scripts/dark-schemes.py` maps each palette onto Reprise's system colours.

## Wallpapers and sounds

Reprise's built-in wallpapers (tarn, ridges, terraces, caustic, reeded,
panes, flow, fold and contour) are its own artwork, drawn in code from noise
and geometry at each screen's size in `crates/reprisekit/src/wallpapers/`.
They hold no image files and share the project's license. Beside them the
Background page lists pictures from `/usr/share/backgrounds`,
`/usr/share/wallpapers` and your Pictures folder, and Browse... adds any other
bmp, jpg, png or dib file.
Reprise ships no picture files and plays no event sounds.

## Contributing assets

A new asset needs a license that can be distributed with GPL-3.0 code, and a
line in this file saying where it came from. Artwork, sounds and fonts from
other desktops and operating systems can't be added, whether copied or
traced, even if you own a copy.
