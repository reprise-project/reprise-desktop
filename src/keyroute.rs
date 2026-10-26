//! Who gets a key: the one ordered table of the compositor's modes. The
//! seat handler decodes a press once into a `KeyPress`, snapshots the modal
//! state into `Modes`, and `route` says what happens. Nothing here touches
//! Smithay, so the ordering rules (a lock beats a menu beats a dialog beats
//! a bind beats the client, but a modified bind passes a menu) are plain
//! data and tested as such. The power,
//! sleep and media keys are the machine's, ahead of the lock and a capture,
//! except the wifi key, which the lock takes.
//! VT switches come ahead of both, and Super+Shift+Escape stays local under
//! a capture, so the seat can always be taken back. A client inhibiting
//! shortcuts gets the binds and the Super tap, but nothing ahead of them,
//! and the same chord gives the binds back. Global shortcuts a program
//! bound through the portal match after the binds, and their release goes
//! to no one (`globalshortcuts`).
//!
//! A bind on a modifier key itself ("Shift+Alt_L") is a modifier-only
//! chord: `ChordState` watches every key beside the route and fires it on
//! a release, so the keys still reach whoever the route gives them to.

use crate::config::{Action, Keybind};
use crate::desktop::IconMove;
use crate::keys::*;

/// One key event, decoded: every keysym the key produces, the same
/// keysyms lowercased (binds and the switcher match on those), the
/// modifier mask with Caps and Num Lock masked out, and press or release.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyPress {
    pub syms: Vec<u32>,
    pub lower: Vec<u32>,
    pub mods: u32,
    pub pressed: bool,
}

impl KeyPress {
    fn has(&self, sym: u32) -> bool {
        self.syms.contains(&sym)
    }

    fn any_super(&self) -> bool {
        self.has(KEY_Super_L) || self.has(KEY_Super_R)
    }
}

/// The modal state routing depends on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    /// A session (a real seat, not a nested window) can switch VTs.
    pub can_switch_vt: bool,
    pub locked: bool,
    /// An InputCapture client has the keyboard.
    pub captured: bool,
    /// A Preview saver is up, or the key being handled ended one (`saverview::holds_keys`).
    pub saver_preview: bool,
    /// A keyboard move or size from the window menu is under way.
    pub keyboard_interaction: bool,
    /// An edge or width drag of the taskbar is under way (`bardrag`).
    pub bar_drag: bool,
    /// A window's frame is dragged with the pointer button held.
    pub pointer_move: bool,
    /// An Alt+Tab panel or the 3D switcher is open.
    pub switcher_open: bool,
    /// A Super key went down on its own and nothing else since.
    pub super_tap_armed: bool,
    pub menus_open: bool,
    /// The top dialog has the keyboard (see `dialogview::has_keyboard`).
    pub dialog_has_keyboard: bool,
    /// That dialog is a Welcome panel, which keeps even the switcher's
    /// bind.
    pub dialog_modal: bool,
    /// That dialog stands among the windows (`Dialog::windowed`), so Show
    /// Desktop reaches past it.
    pub dialog_windowed: bool,
    pub launcher_open: bool,
    /// The window overview is open, and whether it is snap assist, which
    /// lets the snap binds through.
    pub overview_open: bool,
    pub snap_assist_open: bool,
    /// The snap layout picker is open, and whether the bind opened it,
    /// which gives it every press.
    pub snap_picker_open: bool,
    pub snap_picker_keys: bool,
    /// The Glass Start menu is open.
    pub start_panel_open: bool,
    /// The clipboard history popup is open.
    pub clipview_open: bool,
    /// The emoji picker is open.
    pub emojiview_open: bool,
    /// The notification history popup is up.
    pub notify_center_open: bool,
    /// A notification balloon's reply field has the keyboard.
    pub notify_reply_focused: bool,
    /// Shift+PrtSc's region overlay is up.
    pub region_open: bool,
    /// The clock's month popup is open.
    pub calendar_open: bool,
    /// The displays cell's panel is open.
    pub displays_open: bool,
    /// The seat carries a drag between programs (`dnd`).
    pub dragging: bool,
    /// The tray's hidden icons flyout is open.
    pub tray_flyout_open: bool,
    /// The taskbar has the keyboard (Super+T, `taskbarview::BarKeys`).
    pub taskbar_keys: bool,
    pub renaming: bool,
    /// The focused client inhibits the compositor's shortcuts
    /// (`shortcutinhibit`).
    pub shortcuts_inhibited: bool,
    /// No window and no keyboard-interactive layer surface has the keyboard.
    pub desktop_has_keyboard: bool,
    pub has_icons: bool,
    pub selected_icon: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyRoute {
    /// The power button (`XF86PowerOff`): `[power] power_button`.
    PowerKey,
    /// The sleep button (`XF86Sleep`): `[power] sleep_button`.
    SleepKey,
    /// A media key with no modifier and no config bind of its own.
    Media(Action),
    /// `XF86Switch_VT_n`.
    VtSwitch(u32),
    /// Everything goes to the lock dialog.
    Lock,
    /// Super+Shift+Escape under a capture: end it and keep the key.
    CaptureEscape,
    /// Forwarded raw to the InputCapture receiver.
    Captured,
    /// Super+Shift+Escape under a shortcuts inhibitor: suspend it and keep
    /// the key.
    InhibitEscape,
    /// A keyboard move or size takes the press (its first keysym).
    Interaction(u32),
    /// Escape during a drag of the taskbar drops it.
    BarDragCancel,
    /// Escape during a pointer drag of a window's frame puts it back.
    MoveCancel,
    SwitcherCancel,
    /// Escape during a drag between programs cancels it.
    DragCancel,
    SwitcherStep {
        backwards: bool,
    },
    /// Consumed; neither a client nor a bind sees it.
    Swallow,
    /// The topmost menu takes the press (its first non-Super keysym).
    Menu(u32),
    Dialog,
    /// The launcher panel's field takes the press.
    Launcher,
    /// The window overview takes the press.
    Overview,
    /// The snap layout picker takes the press (its first keysym).
    SnapPicker(u32),
    /// The Glass Start menu takes the press.
    StartPanel,
    /// The clipboard history popup takes the press.
    ClipView,
    /// The emoji picker takes the press.
    EmojiView,
    /// The notification history popup takes the press (its first keysym).
    NotifyCenter(u32),
    /// The notification balloon's reply field takes the press.
    NotifyReply,
    /// The region overlay takes the press (its first keysym).
    Region(u32),
    /// Escape closes the clock's month popup.
    CalendarClose,
    /// The displays cell's panel takes the press.
    Displays,
    /// Escape closes the tray's hidden icons flyout.
    TrayFlyoutClose,
    /// The taskbar with the keyboard takes the press (its first keysym).
    Taskbar(u32),
    Rename,
    IconMove(IconMove),
    IconOpen(usize),
    /// Alt+Enter: the icon's Properties, as its menu's Properties row.
    IconProperties(usize),
    IconTrash(usize),
    /// Shift+Delete: the icon's file goes for good, once asked.
    IconErase(usize),
    IconRename(usize),
    /// The Menu key or Shift+F10: the selected icon's menu, or the
    /// desktop's with none.
    IconMenu(Option<usize>),
    /// Ctrl+A.
    IconSelectAll,
    /// Ctrl+X and Ctrl+C on the selected icon: its files go on the
    /// clipboard, cut or copied.
    IconCut(usize),
    IconCopy(usize),
    /// Ctrl+V: the clipboard's files onto the desktop.
    IconPaste,
    /// F5: the icons and the Recycle Bin listed again.
    IconRefresh,
    /// The keybinds the press matched, in keysym order.
    Binds(Vec<Action>),
    /// The global shortcuts the press matched; no keybind did.
    Global(Vec<GlobalHit>),
    /// Forwarded to the focused client.
    Client,
}

/// What the seat handler does with a key: the route, whether the Super
/// tap arms or disarms on this press, and whether it fires on this release
/// (the Super key on its own toggles the Start menu).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Routed {
    pub route: KeyRoute,
    pub arm_super: Option<bool>,
    pub fire_super: bool,
}

/// A `[[global_shortcuts]]` chord whose program holds a shortcuts object
/// (`globalshortcuts::live`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalBind {
    pub mods: u32,
    pub keysym: u32,
    pub app: String,
    pub id: String,
}

/// Which program's shortcut a press fired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalHit {
    pub app: String,
    pub id: String,
}

#[expect(non_upper_case_globals)] // xkb keysym names in patterns
/// Where `k` goes given `modes`. Releases mostly pass through so a client
/// never sees a key stuck down; the lock and the switcher take those too.
/// `globals` are matched after `binds`, in the same place and the same way.
pub fn route(modes: &Modes, k: &KeyPress, binds: &[Keybind], globals: &[GlobalBind]) -> Routed {
    let plain = |r| Routed {
        route: r,
        arm_super: None,
        fire_super: false,
    };
    // The power and sleep buttons are the machine's, not the session's:
    // they work at the lock screen and under a capture, as logind's own
    // handling would have.
    if k.pressed && k.has(KEY_XF86PowerOff) {
        return plain(KeyRoute::PowerKey);
    }
    if k.pressed && k.has(KEY_XF86Sleep) {
        return plain(KeyRoute::SleepKey);
    }
    // The media keys are the machine's too, with the same precedence, but
    // a config bind on the bare key replaces the built-in action. The wifi
    // key is the exception at the lock: nobody there may take the machine
    // off the network.
    if k.pressed
        && k.mods == 0
        && let Some(a) = k.syms.iter().find_map(|&s| media_action_for(s))
        && !(modes.locked && a == Action::Wifi)
    {
        let bound = k
            .lower
            .iter()
            .any(|&l| binds.iter().any(|b| b.mods == 0 && b.keysym == l));
        if !bound {
            return plain(KeyRoute::Media(a));
        }
    }
    // VT switching always works, under the lock and a capture too: another
    // VT asks for its own login, and it is the way out of a lock whose
    // password cannot be checked.
    if k.pressed
        && modes.can_switch_vt
        && let Some(sym) = k
            .syms
            .iter()
            .find(|s| (KEY_XF86Switch_VT_1..=KEY_XF86Switch_VT_12).contains(s))
    {
        return plain(KeyRoute::VtSwitch(sym - KEY_XF86Switch_VT_1 + 1));
    }
    if modes.locked {
        return plain(KeyRoute::Lock);
    }
    if modes.captured {
        // The receiver may hang or the portal never release: this chord is
        // the compositor's own way back. Super was forwarded as pressed, so
        // its release must not fire the Start menu here.
        if k.pressed && k.mods == MOD_LOGO | MOD_SHIFT && k.has(KEY_Escape) {
            return Routed {
                route: KeyRoute::CaptureEscape,
                arm_super: Some(false),
                fire_super: false,
            };
        }
        return plain(KeyRoute::Captured);
    }
    // A press under a Preview saver is no one's, the one that ends it too; a release goes where its press went.
    if modes.saver_preview && k.pressed {
        return Routed {
            route: KeyRoute::Swallow,
            arm_super: Some(false),
            fire_super: false,
        };
    }
    if modes.region_open && k.pressed {
        return plain(match k.syms.first() {
            Some(&s) => KeyRoute::Region(s),
            None => KeyRoute::Swallow,
        });
    }
    // During a drag between programs no client hears a key (the drag's
    // keyboard grab keeps them), but Reprise's own keys work and the drag
    // runs on: the switcher and the menus take theirs, Escape cancels the
    // drag, the binds `runs_during_drag` allows run and the Super tap opens
    // Start. Every other press is swallowed; the releases go through below.
    if modes.dragging && k.pressed {
        let route = if modes.switcher_open {
            switcher_key(k, binds)
        } else if modes.menus_open {
            menu_press(modes, k, binds)
        } else if k.has(KEY_Escape) {
            KeyRoute::DragCancel
        } else {
            let actions: Vec<Action> = bound(k, binds)
                .into_iter()
                .filter(runs_during_drag)
                .collect();
            if actions.is_empty() {
                KeyRoute::Swallow
            } else {
                KeyRoute::Binds(actions)
            }
        };
        return Routed {
            route,
            arm_super: Some(k.any_super() && k.mods & !MOD_LOGO == 0),
            fire_super: false,
        };
    }
    // A keyboard move or size owns every press.
    if modes.keyboard_interaction && k.pressed {
        return plain(match k.syms.first() {
            Some(&s) => KeyRoute::Interaction(s),
            None => KeyRoute::Swallow,
        });
    }
    // Escape drops a drag of the taskbar unless a menu is open, which
    // takes it first; every other key goes on.
    if modes.bar_drag && !modes.menus_open && k.pressed && k.has(KEY_Escape) {
        return plain(KeyRoute::BarDragCancel);
    }
    // So does a pointer drag of a window: it goes back where it started.
    if modes.pointer_move && !modes.menus_open && k.pressed && k.has(KEY_Escape) {
        return plain(KeyRoute::MoveCancel);
    }
    if modes.switcher_open {
        // The previously focused client must not see anything else.
        return plain(if k.pressed {
            switcher_key(k, binds)
        } else {
            KeyRoute::Swallow
        });
    }
    // The Super key on its own: armed by a bare Super press, disarmed by
    // any other press (a Super+x bind, say), fired on the release. A client
    // inhibiting shortcuts keeps the Super key, except during a drag, when
    // it hears no key and the tap armed then is Reprise's.
    let any_super = k.any_super();
    let (arm_super, fire_super) = if modes.shortcuts_inhibited && !modes.dragging {
        (k.pressed.then_some(false), false)
    } else if k.pressed {
        (Some(any_super && k.mods & !MOD_LOGO == 0), false)
    } else {
        (None, any_super && modes.super_tap_armed)
    };
    let with = |r| Routed {
        route: r,
        arm_super,
        fire_super,
    };
    let switch = || switch_bind(modes, k, binds);
    if modes.menus_open && k.pressed {
        return with(menu_press(modes, k, binds));
    }
    // The launcher, the Glass Start menu, the clipboard history popup and
    // the emoji picker each own every press while open, except their own
    // bind, which toggles them shut the way it opened them, and the
    // switchers' binds, as a menu.
    let own = |action: Action| {
        k.lower.iter().any(|&lower| {
            binds
                .iter()
                .any(|b| b.mods == k.mods && b.keysym == lower && b.action == action)
        })
    };
    if modes.overview_open && k.pressed {
        let snaps = bound(k, binds);
        return with(if own(Action::Overview) {
            KeyRoute::Binds(vec![Action::Overview])
        } else if let Some(r) = switch() {
            r
        } else if modes.snap_assist_open
            && !snaps.is_empty()
            && snaps.iter().all(|a| matches!(a, Action::Snap(_)))
        {
            KeyRoute::Binds(snaps)
        } else {
            KeyRoute::Overview
        });
    }
    // The snap layout picker the bind opened takes every press but its
    // bind and the switchers'; one the pointer opened takes only Escape.
    if modes.snap_picker_keys && k.pressed {
        return with(if own(Action::SnapLayouts) {
            KeyRoute::Binds(vec![Action::SnapLayouts])
        } else if let Some(r) = switch() {
            r
        } else {
            KeyRoute::SnapPicker(k.syms.first().copied().unwrap_or(0))
        });
    }
    if modes.snap_picker_open && k.pressed && k.has(KEY_Escape) {
        return with(KeyRoute::SnapPicker(KEY_Escape));
    }
    if modes.start_panel_open && k.pressed {
        return with(if own(Action::StartMenu) {
            KeyRoute::Binds(vec![Action::StartMenu])
        } else {
            switch().unwrap_or(KeyRoute::StartPanel)
        });
    }
    // The clock's popup takes only Escape, which closes it; everything
    // else goes where it would have.
    if modes.calendar_open && k.pressed && k.has(KEY_Escape) {
        return with(KeyRoute::CalendarClose);
    }
    // The hidden icons flyout likewise.
    if modes.tray_flyout_open && k.pressed && k.has(KEY_Escape) {
        return with(KeyRoute::TrayFlyoutClose);
    }
    // The displays panel takes every press that is not a bind or a global
    // shortcut, as the notification history: lock and screenshot work over it.
    if modes.displays_open && k.pressed {
        let actions = bound(k, binds);
        if !actions.is_empty() {
            return with(KeyRoute::Binds(actions));
        }
        let hits = bound_global(k, globals);
        return with(if hits.is_empty() {
            KeyRoute::Displays
        } else {
            KeyRoute::Global(hits)
        });
    }
    if modes.launcher_open && k.pressed {
        return with(if own(Action::Launcher) {
            KeyRoute::Binds(vec![Action::Launcher])
        } else {
            switch().unwrap_or(KeyRoute::Launcher)
        });
    }
    if modes.clipview_open && k.pressed {
        return with(if own(Action::ClipboardHistory) {
            KeyRoute::Binds(vec![Action::ClipboardHistory])
        } else {
            switch().unwrap_or(KeyRoute::ClipView)
        });
    }
    if modes.emojiview_open && k.pressed {
        return with(if own(Action::EmojiPicker) {
            KeyRoute::Binds(vec![Action::EmojiPicker])
        } else {
            switch().unwrap_or(KeyRoute::EmojiView)
        });
    }
    // A balloon's reply field, once clicked, takes every press but the
    // switcher's bind, as a dialog's field does.
    if modes.notify_reply_focused && k.pressed {
        let cycle = k.lower.iter().any(|&lower| {
            binds
                .iter()
                .any(|b| b.mods == k.mods && b.keysym == lower && b.action == Action::Cycle)
        });
        return with(if cycle {
            KeyRoute::Binds(vec![Action::Cycle])
        } else {
            KeyRoute::NotifyReply
        });
    }
    // The notification history takes every press that is not a bind or a
    // global shortcut: it has no field, so those keep working over it.
    if modes.notify_center_open && k.pressed {
        let actions = bound(k, binds);
        if !actions.is_empty() {
            return with(KeyRoute::Binds(actions));
        }
        let hits = bound_global(k, globals);
        return with(match (hits.is_empty(), k.syms.first()) {
            (false, _) => KeyRoute::Global(hits),
            (true, Some(&sym)) => KeyRoute::NotifyCenter(sym),
            (true, None) => KeyRoute::Swallow,
        });
    }
    // The taskbar with the keyboard takes every press that is not a bind:
    // the arrows walk it, Enter and the Menu key act on its item, Escape
    // gives the keyboard back.
    if modes.taskbar_keys && k.pressed {
        let actions = desktop_close(bound(k, binds), modes);
        return with(match (actions.is_empty(), k.syms.first()) {
            (false, _) => KeyRoute::Binds(actions),
            (true, Some(&sym)) => KeyRoute::Taskbar(sym),
            (true, None) => KeyRoute::Swallow,
        });
    }
    // The dialog with the keyboard takes every press (its field wants the
    // text), except binds that move no focus (`passes_dialog`) and the
    // switchers' binds: Alt+Tab leaves a dialog as it leaves a window, and
    // so does Super+Tab. A windowed dialog lets Show Desktop through as well.
    if modes.dialog_has_keyboard && k.pressed {
        let passing: Vec<Action> = bound(k, binds).into_iter().filter(passes_dialog).collect();
        if !passing.is_empty() {
            return with(KeyRoute::Binds(passing));
        }
        if modes.dialog_windowed && own(Action::ShowDesktop) {
            return with(KeyRoute::Binds(vec![Action::ShowDesktop]));
        }
        return with(switch().unwrap_or(KeyRoute::Dialog));
    }
    // An inline desktop rename likewise owns every press.
    if modes.renaming && k.pressed {
        return with(KeyRoute::Rename);
    }
    // The desktop icons when nothing else has the keyboard: arrows move
    // the selection, Enter opens, Alt+Enter shows Properties, Delete
    // trashes (Shift+Delete for good), F2 renames, F5 refreshes, the Menu
    // key opens the menu, and Ctrl+A, Ctrl+X, Ctrl+C and Ctrl+V select all,
    // cut, copy and paste. A bound key goes to its bind or global shortcut, so
    // Super+Ctrl+arrows switch desktops from an empty one.
    if k.pressed
        && modes.desktop_has_keyboard
        && bound(k, binds).is_empty()
        && bound_global(k, globals).is_empty()
    {
        if modes.has_icons
            && let Some(dir) = k.syms.iter().find_map(|&s| icon_move_for(s))
        {
            return with(KeyRoute::IconMove(dir));
        }
        for &lower in &k.lower {
            match (lower, k.mods) {
                (KEY_Menu, 0) | (KEY_F10, MOD_SHIFT) => {
                    return with(KeyRoute::IconMenu(modes.selected_icon));
                }
                (KEY_a, MOD_CTRL) if modes.has_icons => return with(KeyRoute::IconSelectAll),
                (KEY_v, MOD_CTRL) => return with(KeyRoute::IconPaste),
                (KEY_F5, 0) => return with(KeyRoute::IconRefresh),
                _ => {}
            }
        }
        if let Some(i) = modes.selected_icon {
            for &sym in &k.lower {
                match sym {
                    KEY_Return | KEY_KP_Enter if k.mods & MOD_ALT != 0 => {
                        return with(KeyRoute::IconProperties(i));
                    }
                    KEY_Return | KEY_KP_Enter => return with(KeyRoute::IconOpen(i)),
                    KEY_Delete | KEY_KP_Delete if k.mods == MOD_SHIFT => {
                        return with(KeyRoute::IconErase(i));
                    }
                    KEY_Delete | KEY_KP_Delete => return with(KeyRoute::IconTrash(i)),
                    KEY_F2 => return with(KeyRoute::IconRename(i)),
                    KEY_x if k.mods == MOD_CTRL => return with(KeyRoute::IconCut(i)),
                    KEY_c if k.mods == MOD_CTRL => return with(KeyRoute::IconCopy(i)),
                    _ => {}
                }
            }
        }
    }
    if modes.shortcuts_inhibited {
        // The chord ends a capture too, so it is the one way back to the
        // binds whatever took them.
        if k.pressed && k.mods == MOD_LOGO | MOD_SHIFT && k.has(KEY_Escape) {
            return with(KeyRoute::InhibitEscape);
        }
        if k.pressed {
            let passing: Vec<Action> = bound(k, binds).into_iter().filter(passes_dialog).collect();
            if !passing.is_empty() {
                return with(KeyRoute::Binds(passing));
            }
        }
        return with(KeyRoute::Client);
    }
    if k.pressed {
        let actions = desktop_close(bound(k, binds), modes);
        if !actions.is_empty() {
            return with(KeyRoute::Binds(actions));
        }
        let hits = bound_global(k, globals);
        if !hits.is_empty() {
            return with(KeyRoute::Global(hits));
        }
    }
    with(KeyRoute::Client)
}

/// A press in the open switcher: Escape cancels it, Tab or a switcher's
/// bind steps it; anything else (the arrows too) is swallowed.
fn switcher_key(k: &KeyPress, binds: &[Keybind]) -> KeyRoute {
    for &lower in &k.lower {
        if lower == KEY_Escape {
            return KeyRoute::SwitcherCancel;
        }
        if let Some(backwards) = switcher_step_for(lower, k.mods, binds) {
            return KeyRoute::SwitcherStep { backwards };
        }
    }
    KeyRoute::Swallow
}

/// Whether a bind's action opens a switcher: `cycle` the panel, `flip`
/// the 3D one.
fn opens_switcher(a: &Action) -> bool {
    matches!(a, Action::Cycle | Action::Flip)
}

/// The switcher bind `k` matches, as a route, unless a Welcome panel has
/// the keyboard (it keeps even those).
fn switch_bind(modes: &Modes, k: &KeyPress, binds: &[Keybind]) -> Option<KeyRoute> {
    if modes.dialog_has_keyboard && modes.dialog_modal {
        return None;
    }
    let a = bound(k, binds).into_iter().find(opens_switcher)?;
    Some(KeyRoute::Binds(vec![a]))
}

/// A press over the open menu stack. A bind with Ctrl, Alt or Super runs
/// (the menus close first, `keyboard`), unless a Welcome panel has the
/// keyboard; during a drag only those `runs_during_drag` allows. Any other
/// press drives the menu, but a letter or digit under Super, or under Ctrl
/// outside a drag (where Ctrl picks the drop action), is no access key.
fn menu_press(modes: &Modes, k: &KeyPress, binds: &[Keybind]) -> KeyRoute {
    let welcome = modes.dialog_has_keyboard && modes.dialog_modal;
    if !welcome && k.mods & !MOD_SHIFT != 0 {
        let actions: Vec<Action> = bound(k, binds)
            .into_iter()
            .filter(|a| !modes.dragging || runs_during_drag(a))
            .collect();
        if !actions.is_empty() {
            return KeyRoute::Binds(actions);
        }
    }
    let held = if modes.dragging {
        MOD_LOGO
    } else {
        MOD_LOGO | MOD_CTRL
    };
    match menu_key(k) {
        KeyRoute::Menu(sym)
            if k.mods & held != 0 && key_char(sym).is_some_and(char::is_alphanumeric) =>
        {
            KeyRoute::Swallow
        }
        r => r,
    }
}

/// A press in the open menu stack: its first keysym that is not Super.
fn menu_key(k: &KeyPress) -> KeyRoute {
    k.syms
        .iter()
        .copied()
        .find(|&s| s != KEY_Super_L && s != KEY_Super_R)
        .map_or(KeyRoute::Swallow, KeyRoute::Menu)
}

/// The held modifiers `mods` a press routed to `route` during a drag
/// spends, so they pick no drop action until released: a bind's own (the
/// Alt of Alt+Tab) and the switcher's. A menu's keys use none, so Ctrl
/// held from before Start opened still copies; a swallowed press and
/// the drag's cancel spend nothing either.
pub fn drag_spends(route: &KeyRoute, mods: u32) -> u32 {
    match route {
        KeyRoute::Swallow | KeyRoute::DragCancel | KeyRoute::Menu(_) => 0,
        _ => mods,
    }
}

/// Whether a bind's action runs during a drag between programs: those
/// that bring another window, the desktop or a field within reach of the
/// drop (the switcher, Start, the launcher, Run, Show Desktop, minimize,
/// maximize, snap, the desktop switch) and the magnifier. The rest (closing,
/// other dialogs, screenshots) wait for the drop.
pub fn runs_during_drag(a: &Action) -> bool {
    matches!(
        a,
        Action::Cycle
            | Action::Flip
            | Action::StartMenu
            | Action::Launcher
            | Action::Run
            | Action::ShowDesktop
            | Action::Minimize
            | Action::Maximize
            | Action::Snap(_)
            | Action::Desktop(_)
            | Action::Zoom(_)
    )
}

/// Whether a bind's action runs while a dialog has the keyboard, a Welcome
/// panel's included: the magnifier, lock, screenshots, the layout and the
/// machine's toggles leave the dialog its focus.
fn passes_dialog(a: &Action) -> bool {
    matches!(
        a,
        Action::Zoom(_)
            | Action::ColorFilter
            | Action::Lock
            | Action::NextLayout
            | Action::DoNotDisturb
            | Action::NightLight
            | Action::Screenshot
            | Action::ScreenshotWindow
            | Action::ScreenshotRegion
            | Action::ScreenshotContent
            | Action::VolumeUp
            | Action::VolumeDown
            | Action::Mute
            | Action::MicMute
            | Action::BrightnessUp
            | Action::BrightnessDown
            | Action::KbdBrightnessUp
            | Action::KbdBrightnessDown
            | Action::PlayPause
            | Action::Next
            | Action::Prev
            | Action::Stop
            | Action::Wifi
    )
}

/// Whether a bind's action opens or shuts a surface of Reprise's own that
/// takes the pointer: a pointer gesture still held ends before it runs
/// (`State::cancel_interaction`).
pub fn opens_surface(a: &Action) -> bool {
    matches!(
        a,
        Action::StartMenu
            | Action::WindowMenu
            | Action::Launcher
            | Action::ClipboardHistory
            | Action::EmojiPicker
            | Action::SnapLayouts
            | Action::Overview
            | Action::Cycle
            | Action::Flip
            | Action::ScreenshotRegion
            | Action::FocusTaskbar
            | Action::FocusTray
            | Action::Run
            | Action::LogOff
            | Action::TurnOff
            | Action::Exit
            | Action::Lock
    )
}

impl KeyRoute {
    /// Whether this route runs a bind that `opens_surface`.
    pub fn opens_surface(&self) -> bool {
        matches!(self, KeyRoute::Binds(a) if a.iter().any(opens_surface))
    }
}

/// The actions the binds give `k`, in keysym order.
/// With no window to close (the desktop or the taskbar has the keyboard), the close bind opens
/// Turn Off.
fn desktop_close(mut actions: Vec<Action>, modes: &Modes) -> Vec<Action> {
    if modes.desktop_has_keyboard {
        for a in &mut actions {
            if *a == Action::Close {
                *a = Action::TurnOff;
            }
        }
    }
    actions
}

fn bound(k: &KeyPress, binds: &[Keybind]) -> Vec<Action> {
    k.lower
        .iter()
        .filter_map(|&lower| {
            binds
                .iter()
                .find(|b| !b.is_chord() && b.mods == k.mods && b.keysym == lower)
                .map(|b| b.action.clone())
        })
        .collect()
}

/// The global shortcuts whose chord is `k`.
fn bound_global(k: &KeyPress, globals: &[GlobalBind]) -> Vec<GlobalHit> {
    (globals.iter())
        .filter(|g| g.mods == k.mods && k.lower.contains(&g.keysym))
        .map(|g| GlobalHit {
            app: g.app.clone(),
            id: g.id.clone(),
        })
        .collect()
}

/// A modifier key's keysym as a chord counts it, with its modifier bit;
/// None for any other key. Meta is Alt: xkb gives the left Alt key Meta_L
/// while Shift is held.
#[expect(non_upper_case_globals)] // xkb keysym names in patterns
pub fn modifier_key(sym: u32) -> Option<(u32, u32)> {
    Some(match sym {
        KEY_Shift_L | KEY_Shift_R => (sym, MOD_SHIFT),
        KEY_Control_L | KEY_Control_R => (sym, MOD_CTRL),
        KEY_Alt_L | KEY_Alt_R => (sym, MOD_ALT),
        KEY_Meta_L => (KEY_Alt_L, MOD_ALT),
        KEY_Meta_R => (KEY_Alt_R, MOD_ALT),
        KEY_Super_L | KEY_Super_R => (sym, MOD_LOGO),
        _ => return None,
    })
}

/// The modifier-only chords: which modifier keys are down, and whether
/// anything else went down since the first of them. A chord bind
/// (`Keybind::is_chord`) fires on the release of any of its keys when the
/// held modifier keys are exactly its own, include its key, and no other
/// key or button came in between. With the chord still held, pressing the
/// released key again arms it again, so each Shift tap under a held Alt
/// fires once.
#[derive(Debug, Default)]
pub struct ChordState {
    /// (keycode, keysym as `modifier_key` counts it, modifier bit).
    held: Vec<(u32, u32, u32)>,
    /// What the next release fires.
    armed: Option<Action>,
    /// Another key, a third modifier or a button went down since the first
    /// held key; cleared once every modifier key is up.
    spoiled: bool,
}

impl ChordState {
    /// Feed one key event: its keycode and decoded keysyms. Returns the
    /// action of the chord a release completes.
    pub fn key(&mut self, keycode: u32, k: &KeyPress, binds: &[Keybind]) -> Option<Action> {
        if !k.pressed {
            let before = self.held.len();
            self.held.retain(|h| h.0 != keycode);
            let fired = if self.held.len() < before && !self.spoiled {
                self.armed.take()
            } else {
                None
            };
            if self.held.is_empty() {
                self.spoiled = false;
                self.armed = None;
            }
            return fired;
        }
        let Some((sym, bit)) = k.syms.iter().find_map(|&s| modifier_key(s)) else {
            self.spoil();
            return None;
        };
        if self.held.is_empty() {
            self.spoiled = false;
        }
        if !self.held.iter().any(|h| h.0 == keycode) {
            self.held.push((keycode, sym, bit));
        }
        let hit = self.matching(binds);
        if self.armed.is_some() && hit.is_none() {
            self.spoiled = true;
        }
        self.armed = if self.spoiled { None } else { hit };
        None
    }

    /// A pointer button went down: nothing held now fires.
    pub fn spoil(&mut self) {
        if !self.held.is_empty() {
            self.spoiled = true;
        }
        self.armed = None;
    }

    /// The chord bind the held keys make exactly.
    fn matching(&self, binds: &[Keybind]) -> Option<Action> {
        let mask = self.held.iter().fold(0, |m, h| m | h.2);
        binds
            .iter()
            .find(|b| {
                modifier_key(b.keysym).is_some_and(|(sym, bit)| {
                    b.mods | bit == mask && self.held.iter().any(|h| h.1 == sym)
                })
            })
            .map(|b| b.action.clone())
    }
}

/// Whether a chord's action may run under `modes`: never under a capture
/// or a shortcuts inhibitor, whose client gets the keys, nor during a drag,
/// whose modifiers pick its action, and at the lock
/// only the layout switch, so the password can be typed in any layout.
/// A dialog with the keyboard passes what it passes a bind.
pub fn chord_allowed(modes: &Modes, a: &Action) -> bool {
    !modes.captured
        && !modes.shortcuts_inhibited
        && !modes.dragging
        && (!modes.locked || *a == Action::NextLayout)
        && (!modes.dialog_has_keyboard
            || passes_dialog(a)
            || (modes.dialog_windowed && *a == Action::ShowDesktop))
}

/// The built-in action of a media keysym; None for any other key.
#[expect(non_upper_case_globals)] // xkb keysym names in patterns
pub fn media_action_for(sym: u32) -> Option<Action> {
    Some(match sym {
        KEY_XF86AudioRaiseVolume => Action::VolumeUp,
        KEY_XF86AudioLowerVolume => Action::VolumeDown,
        KEY_XF86AudioMute => Action::Mute,
        KEY_XF86AudioMicMute => Action::MicMute,
        KEY_XF86MonBrightnessUp => Action::BrightnessUp,
        KEY_XF86MonBrightnessDown => Action::BrightnessDown,
        KEY_XF86KbdBrightnessUp => Action::KbdBrightnessUp,
        KEY_XF86KbdBrightnessDown => Action::KbdBrightnessDown,
        KEY_XF86AudioPlay | KEY_XF86AudioPause => Action::PlayPause,
        KEY_XF86AudioNext => Action::Next,
        KEY_XF86AudioPrev => Action::Prev,
        KEY_XF86AudioStop => Action::Stop,
        KEY_XF86WLAN => Action::Wifi,
        _ => return None,
    })
}

/// The step a key makes in the open switcher: Tab and the switchers'
/// binds go forward, either with Shift goes backwards; None for any other
/// key.
pub fn switcher_step_for(sym_lower: u32, mods: u32, binds: &[Keybind]) -> Option<bool> {
    let backwards = mods & MOD_SHIFT != 0;
    if sym_lower == KEY_Tab || sym_lower == KEY_ISO_Left_Tab {
        return Some(backwards);
    }
    let unshifted = mods & !MOD_SHIFT;
    binds
        .iter()
        .any(|b| b.mods == unshifted && b.keysym == sym_lower && opens_switcher(&b.action))
        .then_some(backwards)
}

/// The desktop-icon selection move an arrow, Home or End keysym makes.
#[expect(non_upper_case_globals)] // xkb keysym names in patterns
pub fn icon_move_for(sym: u32) -> Option<IconMove> {
    use IconMove::*;
    Some(match sym {
        KEY_Up | KEY_KP_Up => Up,
        KEY_Down | KEY_KP_Down => Down,
        KEY_Left | KEY_KP_Left => Left,
        KEY_Right | KEY_KP_Right => Right,
        KEY_Home | KEY_KP_Home => Home,
        KEY_End | KEY_KP_End => End,
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
