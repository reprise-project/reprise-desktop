#!/usr/bin/env python3
"""Generate the dark colour schemes for crates/reprisekit/src/schemes.rs and
splice them into the table in alphabetical order (run from the repo root).
Idempotent: an entry whose name is already in the table is replaced.

Each scheme maps a well-known editor palette onto the system colours:
  window        the editor background (fields recede)
  button_face   the theme's panel surface, one step lighter
  button_hilight / button_shadow / button_dk_shadow  the bevel, a clear
                step lighter and two steps darker so 3D edges still read
  active_title  the theme's accent, dark end at the left where the text is
  hilight       the same accent, tying the menu bar to the caption
  gray_text     the theme's comment colour
  background    the theme's darkest surface, for the desktop
  info_bk       a lifted surface for tooltips
  hot_light     the lighter accent, for links on the face
  menu_hilight / menu_bar  the accent and the surface, as a flat menu's lit
                row and bar would take them (Classic menus draw `hilight`
                and `button_face` and never read these)
"""
import re

FIELDS = [
    "active_title", "gradient_active_title", "inactive_title",
    "gradient_inactive_title", "title_text", "inactive_title_text",
    "button_face", "button_hilight", "button_light", "button_shadow",
    "button_dk_shadow", "button_text", "background", "window",
    "window_frame", "window_text", "hilight", "hilight_text", "gray_text",
    "menu", "menu_text", "menu_band", "menu_band_top", "gradient_menu_band",
    "menu_band_text", "desktop_text", "info_bk", "info_text", "hot_light",
    "menu_hilight", "menu_bar",
]


def scheme(name, *, fg, fg_dim, comment, base, surface, lift, shadow, dark,
           accent, accent2, accent_text, inactive, inactive2, info=None,
           frame=None):
    """fg: text. base: window. surface: face/menu. lift: bevel highlight.
    shadow/dark: bevel shadows (shadow doubles as the desktop, dark as the frame).
    accent -> accent2: caption gradient and Start band; accent_text on it."""
    return name, {
        "active_title": accent,
        "gradient_active_title": accent2,
        "inactive_title": inactive,
        "gradient_inactive_title": inactive2,
        "title_text": accent_text,
        "inactive_title_text": fg_dim,
        "button_face": surface,
        "button_hilight": lift,
        "button_light": surface,
        "button_shadow": shadow,
        "button_dk_shadow": dark,
        "button_text": fg,
        "background": shadow,
        "window": base,
        "window_frame": frame or dark,
        "window_text": fg,
        "hilight": accent,
        "hilight_text": accent_text,
        "gray_text": comment,
        "menu": surface,
        "menu_text": fg,
        "menu_band": accent,
        "menu_band_top": dark,
        "gradient_menu_band": accent2,
        "menu_band_text": accent_text,
        "desktop_text": fg,
        "info_bk": info or lift,
        "info_text": fg,
        "hot_light": accent2,
        "menu_hilight": accent,
        "menu_bar": surface,
    }


SCHEMES = [
    scheme("Tokyo Night",
           fg="#c0caf5", fg_dim="#a9b1d6", comment="#565f89",
           base="#1a1b26", surface="#24283b", lift="#414868",
           shadow="#16161e", dark="#0f0f14",
           accent="#3d59a1", accent2="#7aa2f7", accent_text="#ffffff",
           inactive="#414868", inactive2="#565f89", info="#292e42"),
    scheme("GitHub Dark",
           fg="#e6edf3", fg_dim="#8b949e", comment="#8b949e",
           base="#0d1117", surface="#21262d", lift="#484f58",
           shadow="#161b22", dark="#010409",
           accent="#1f6feb", accent2="#58a6ff", accent_text="#ffffff",
           inactive="#30363d", inactive2="#484f58", info="#30363d"),
    scheme("One Dark",
           fg="#abb2bf", fg_dim="#9da5b4", comment="#5c6370",
           base="#21252b", surface="#282c34", lift="#3e4451",
           shadow="#181a1f", dark="#101215",
           accent="#528bff", accent2="#61afef", accent_text="#ffffff",
           inactive="#3e4451", inactive2="#4b5263", info="#2c313a"),
    scheme("Dracula",
           fg="#f8f8f2", fg_dim="#f8f8f2", comment="#6272a4",
           base="#282a36", surface="#343746", lift="#6272a4",
           shadow="#21222c", dark="#191a21",
           accent="#bd93f9", accent2="#ff79c6", accent_text="#282a36",
           inactive="#44475a", inactive2="#6272a4", info="#44475a"),
    scheme("Ayu Dark",
           fg="#bfbdb6", fg_dim="#8a9199", comment="#6c7380",
           base="#0b0e14", surface="#11151c", lift="#2d3640",
           shadow="#070a0f", dark="#000000",
           accent="#e6b450", accent2="#ffb454", accent_text="#0b0e14",
           inactive="#1f2430", inactive2="#2d3640", info="#1f2430"),
    scheme("Night Owl",
           fg="#d6deeb", fg_dim="#5f7e97", comment="#637777",
           base="#011627", surface="#0b2942", lift="#1d3b53",
           shadow="#010e1a", dark="#000000",
           accent="#7e57c2", accent2="#c792ea", accent_text="#ffffff",
           inactive="#122d42", inactive2="#1d3b53", info="#0b253a"),
    scheme("Monokai",
           fg="#f8f8f2", fg_dim="#90908a", comment="#75715e",
           base="#1e1f1c", surface="#272822", lift="#49483e",
           shadow="#141511", dark="#000000",
           accent="#f92672", accent2="#fd971f", accent_text="#f8f8f2",
           inactive="#3e3d32", inactive2="#49483e", info="#3e3d32"),
    scheme("Catppuccin Mocha",
           fg="#cdd6f4", fg_dim="#a6adc8", comment="#6c7086",
           base="#1e1e2e", surface="#313244", lift="#585b70",
           shadow="#181825", dark="#11111b",
           accent="#cba6f7", accent2="#f5c2e7", accent_text="#1e1e2e",
           inactive="#45475a", inactive2="#585b70", info="#45475a"),
    scheme("Gruvbox Dark",
           fg="#ebdbb2", fg_dim="#a89984", comment="#928374",
           base="#282828", surface="#3c3836", lift="#665c54",
           shadow="#1d2021", dark="#000000",
           accent="#d65d0e", accent2="#fe8019", accent_text="#fbf1c7",
           inactive="#504945", inactive2="#665c54", info="#504945"),
    scheme("Nord",
           fg="#eceff4", fg_dim="#d8dee9", comment="#616e88",
           base="#2e3440", surface="#3b4252", lift="#4c566a",
           shadow="#242933", dark="#191c22",
           accent="#5e81ac", accent2="#88c0d0", accent_text="#eceff4",
           inactive="#434c5e", inactive2="#4c566a", info="#434c5e"),
]


def rgb(h):
    h = h.lstrip("#")
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))


def entry(name, cols):
    lines = [f'    (\n        "{name}",\n        ColorScheme {{']
    for f in FIELDS:
        r, g, b = rgb(cols[f])
        lines.append(f"            {f}: Color::rgb({r}, {g}, {b}),")
    lines.append("        },\n    ),")
    return "\n".join(lines)


def main():
    path = "crates/reprisekit/src/schemes.rs"
    src = open(path).read()
    head, rest = src.split("= &[\n", 1)
    body, tail = rest.split("\n];\n", 1)
    # Split the vec body into one string per entry.
    # The last entry is Reprise Standard on one line; the rest are blocks.
    body, standard = body.rsplit("\n", 1)
    parts = re.split(r'(?m)^    \(\n(?=        ")', body)
    entries = {"Reprise Standard": standard}
    for p in parts:
        if not p.strip():
            continue
        p = "    (\n" + p
        name = re.search(r'"([^"]+)"', p).group(1)
        entries[name] = p.rstrip("\n")
    for name, cols in SCHEMES:
        entries[name] = entry(name, cols)
    order = sorted(entries, key=str.casefold)
    # The two Reprise schemes stay last, the default Reprise Standard at
    # the very foot.
    for name in ("Reprise Classic", "Reprise Standard"):
        order.remove(name)
        order.append(name)
    new_body = "\n".join(entries[n] for n in order)
    open(path, "w").write(head + "= &[\n" + new_body + "\n];\n" + tail)
    print("\n".join(order))


if __name__ == "__main__":
    main()
