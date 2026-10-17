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
    // bind; one the pointer opened takes only Escape.
    if modes.snap_picker_keys && k.pressed {
        return with(if own(Action::SnapLayouts) {
            KeyRoute::Binds(vec![Action::SnapLayouts])
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
        let actions = bound(k, binds);
        return with(match (actions.is_empty(), k.syms.first()) {
            (false, _) => KeyRoute::Binds(actions),
            (true, Some(&sym)) => KeyRoute::Taskbar(sym),
            (true, None) => KeyRoute::Swallow,
        });
    }
    // The dialog with the keyboard takes every press (its field wants the
    // text), except the switchers' binds: Alt+Tab leaves a dialog as it
    // leaves a window, and so does Super+Tab. A windowed dialog lets Show
    // Desktop through as well.
    if modes.dialog_has_keyboard && k.pressed {
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
        return with(KeyRoute::Client);
    }
    if k.pressed {
        let mut actions = bound(k, binds);
        // With no window to close, the desktop's close bind opens Turn Off.
        if modes.desktop_has_keyboard {
            for a in &mut actions {
                if *a == Action::Close {
                    *a = Action::TurnOff;
                }
            }
        }
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
pub fn chord_allowed(modes: &Modes, a: &Action) -> bool {
    !modes.captured
        && !modes.shortcuts_inhibited
        && !modes.dragging
        && (!modes.locked || *a == Action::NextLayout)
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
mod tests {
    use super::*;

    #[test]
    fn every_media_key_is_reserved_for_the_desktop() {
        // The XF86 keysym block.
        for sym in 0x1008_ff00u32..=0x1008_ffff {
            if media_action_for(sym).is_some() {
                let name = repriseconf::keybind::key_name(sym).expect("an XF86 key has a name");
                assert!(repriseconf::keybind::reserved(&name), "{name} is reserved");
            }
        }
    }

    #[test]
    fn every_modifier_key_name_is_a_modifier_here() {
        for name in repriseconf::keybind::MODIFIER_KEYS {
            let (_, sym) = crate::config::parse_keybind(name).expect(name);
            assert!(modifier_key(sym).is_some(), "{name} is a modifier key");
        }
    }

    fn press(sym: u32, mods: u32) -> KeyPress {
        let lower = to_lower(sym);
        KeyPress {
            syms: vec![sym],
            lower: vec![lower],
            mods,
            pressed: true,
        }
    }

    fn release(sym: u32) -> KeyPress {
        KeyPress {
            pressed: false,
            ..press(sym, 0)
        }
    }

    fn binds() -> Vec<Keybind> {
        vec![
            Keybind {
                mods: MOD_ALT,
                keysym: KEY_Tab,
                action: Action::Cycle,
            },
            Keybind {
                mods: MOD_LOGO,
                keysym: KEY_Tab,
                action: Action::Flip,
            },
            Keybind {
                mods: MOD_LOGO,
                keysym: KEY_r,
                action: Action::Run,
            },
            Keybind {
                mods: MOD_LOGO,
                keysym: KEY_space,
                action: Action::Launcher,
            },
            Keybind {
                mods: MOD_LOGO,
                keysym: KEY_v,
                action: Action::ClipboardHistory,
            },
            Keybind {
                mods: MOD_LOGO,
                keysym: KEY_period,
                action: Action::EmojiPicker,
            },
        ]
    }

    fn r(modes: &Modes, k: &KeyPress) -> KeyRoute {
        route(modes, k, &binds(), &[]).route
    }

    #[test]
    fn the_taskbar_with_the_keyboard_takes_keys_but_not_binds() {
        let m = Modes {
            taskbar_keys: true,
            dialog_has_keyboard: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Right, 0)), KeyRoute::Taskbar(KEY_Right));
        assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::Taskbar(KEY_Escape));
        assert_eq!(
            r(&m, &press(KEY_Tab, MOD_ALT)),
            KeyRoute::Binds(vec![Action::Cycle]),
            "Alt+Tab still switches"
        );
        assert_eq!(r(&m, &release(KEY_Right)), KeyRoute::Client);
        let menu = Modes {
            menus_open: true,
            ..m.clone()
        };
        assert_eq!(
            r(&menu, &press(KEY_Escape, 0)),
            KeyRoute::Menu(KEY_Escape),
            "a menu the bar opened takes its keys first"
        );
        let calendar = Modes {
            calendar_open: true,
            ..m
        };
        assert_eq!(
            r(&calendar, &press(KEY_Escape, 0)),
            KeyRoute::CalendarClose,
            "Escape shuts the clock's popup before it leaves the bar"
        );
    }

    #[test]
    fn power_keys_route_before_everything() {
        let all = Modes {
            locked: true,
            captured: true,
            menus_open: true,
            dialog_has_keyboard: true,
            ..Modes::default()
        };
        assert_eq!(r(&all, &press(KEY_XF86PowerOff, 0)), KeyRoute::PowerKey);
        assert_eq!(r(&all, &press(KEY_XF86Sleep, 0)), KeyRoute::SleepKey);
        // A release is nobody's: the lock takes it, as it takes every key.
        assert_eq!(r(&all, &release(KEY_XF86PowerOff)), KeyRoute::Lock);
    }

    #[test]
    fn a_preview_saver_keeps_every_press_from_the_program() {
        let m = Modes {
            saver_preview: true,
            desktop_has_keyboard: false,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_a, 0)), KeyRoute::Swallow);
        assert_eq!(
            r(&m, &press(KEY_r, MOD_LOGO)),
            KeyRoute::Swallow,
            "binds wait too"
        );
        let super_down = route(&m, &press(KEY_Super_L, 0), &binds(), &[]);
        assert_eq!(super_down.route, KeyRoute::Swallow);
        assert_eq!(
            super_down.arm_super,
            Some(false),
            "its release opens no Start menu"
        );
        assert_eq!(
            r(&m, &press(KEY_XF86AudioMute, 0)),
            KeyRoute::Media(Action::Mute)
        );
        assert_eq!(
            r(&m, &release(KEY_a)),
            KeyRoute::Client,
            "held back by the taken list"
        );
    }

    #[test]
    fn precedence_runs_vt_lock_interaction_switcher_menu_dialog_rename_icons_binds_client() {
        let all = Modes {
            can_switch_vt: true,
            locked: true,
            keyboard_interaction: true,
            switcher_open: true,
            menus_open: true,
            dialog_has_keyboard: true,
            renaming: true,
            desktop_has_keyboard: true,
            has_icons: true,
            selected_icon: Some(0),
            ..Modes::default()
        };
        let enter = press(KEY_Return, 0);
        // VT switching wins over everything, the lock included.
        assert_eq!(
            r(&all, &press(KEY_XF86Switch_VT_3, 0)),
            KeyRoute::VtSwitch(3)
        );
        assert_eq!(r(&all, &release(KEY_XF86Switch_VT_3)), KeyRoute::Lock);
        let mut m = all.clone();
        assert_eq!(r(&m, &enter), KeyRoute::Lock);
        m.locked = false;
        assert_eq!(r(&m, &enter), KeyRoute::Interaction(KEY_Return));
        m.keyboard_interaction = false;
        assert_eq!(r(&m, &enter), KeyRoute::Swallow);
        m.switcher_open = false;
        assert_eq!(r(&m, &enter), KeyRoute::Menu(KEY_Return));
        m.menus_open = false;
        assert_eq!(r(&m, &enter), KeyRoute::Dialog);
        m.dialog_has_keyboard = false;
        assert_eq!(r(&m, &enter), KeyRoute::Rename);
        m.renaming = false;
        assert_eq!(r(&m, &enter), KeyRoute::IconOpen(0));
        m.selected_icon = None;
        assert_eq!(
            r(&m, &press(KEY_Down, 0)),
            KeyRoute::IconMove(IconMove::Down)
        );
        m.desktop_has_keyboard = false;
        assert_eq!(
            r(&m, &press(KEY_r, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::Run])
        );
        assert_eq!(r(&m, &enter), KeyRoute::Client);
    }

    #[test]
    fn the_switcher_bind_passes_a_dialog_and_show_desktop_a_windowed_one_but_not_a_welcome_panel() {
        let mut m = Modes {
            dialog_has_keyboard: true,
            ..Modes::default()
        };
        let alt_tab = press(KEY_Tab, MOD_ALT);
        let super_tab = press(KEY_Tab, MOD_LOGO);
        assert_eq!(r(&m, &alt_tab), KeyRoute::Binds(vec![Action::Cycle]));
        assert_eq!(r(&m, &super_tab), KeyRoute::Binds(vec![Action::Flip]));
        // Other binds still go to the dialog.
        assert_eq!(r(&m, &press(KEY_r, MOD_LOGO)), KeyRoute::Dialog);
        let mut with_sd = binds();
        with_sd.push(Keybind {
            mods: MOD_LOGO,
            keysym: KEY_d,
            action: Action::ShowDesktop,
        });
        let super_d = |m: &Modes| route(m, &press(KEY_d, MOD_LOGO), &with_sd, &[]).route;
        assert_eq!(super_d(&m), KeyRoute::Dialog);
        m.dialog_windowed = true;
        assert_eq!(
            super_d(&m),
            KeyRoute::Binds(vec![Action::ShowDesktop]),
            "a windowed dialog lets Show Desktop through"
        );
        assert_eq!(r(&m, &press(KEY_r, MOD_LOGO)), KeyRoute::Dialog);
        m.dialog_windowed = false;
        m.dialog_modal = true;
        assert_eq!(r(&m, &alt_tab), KeyRoute::Dialog);
        assert_eq!(r(&m, &super_tab), KeyRoute::Dialog);
    }

    #[test]
    fn the_switcher_binds_pass_an_open_start_menu_menu_or_popup() {
        let open = [
            Modes {
                menus_open: true,
                ..Modes::default()
            },
            Modes {
                start_panel_open: true,
                ..Modes::default()
            },
            Modes {
                start_panel_open: true,
                menus_open: true,
                ..Modes::default()
            },
            Modes {
                launcher_open: true,
                ..Modes::default()
            },
            Modes {
                clipview_open: true,
                ..Modes::default()
            },
            Modes {
                emojiview_open: true,
                ..Modes::default()
            },
            Modes {
                menus_open: true,
                dragging: true,
                ..Modes::default()
            },
        ];
        for m in open {
            assert_eq!(
                r(&m, &press(KEY_Tab, MOD_ALT)),
                KeyRoute::Binds(vec![Action::Cycle]),
                "{m:?}"
            );
            assert_eq!(
                r(&m, &press(KEY_Tab, MOD_LOGO)),
                KeyRoute::Binds(vec![Action::Flip]),
                "{m:?}"
            );
            assert_eq!(
                matches!(r(&m, &press(KEY_r, MOD_LOGO)), KeyRoute::Binds(_)),
                m.menus_open,
                "other binds pass a menu but stay with a popup: {m:?}"
            );
        }
        let welcome = Modes {
            menus_open: true,
            dialog_has_keyboard: true,
            dialog_modal: true,
            ..Modes::default()
        };
        assert_eq!(
            r(&welcome, &press(KEY_Tab, MOD_ALT)),
            KeyRoute::Menu(KEY_Tab)
        );
    }

    #[test]
    fn a_capture_takes_every_key_after_the_lock_and_before_everything_else() {
        let mut m = Modes {
            can_switch_vt: true,
            captured: true,
            keyboard_interaction: true,
            menus_open: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Return, 0)), KeyRoute::Captured);
        assert_eq!(r(&m, &release(KEY_Return)), KeyRoute::Captured);
        assert_eq!(r(&m, &press(KEY_r, MOD_LOGO)), KeyRoute::Captured);
        m.locked = true;
        assert_eq!(r(&m, &press(KEY_Return, 0)), KeyRoute::Lock);
    }

    #[test]
    fn super_shift_escape_and_vt_switches_are_never_captured() {
        let m = Modes {
            can_switch_vt: true,
            captured: true,
            ..Modes::default()
        };
        let chord = press(KEY_Escape, MOD_LOGO | MOD_SHIFT);
        let routed = route(&m, &chord, &binds(), &[]);
        assert_eq!(routed.route, KeyRoute::CaptureEscape);
        assert_eq!(routed.arm_super, Some(false));
        assert_eq!(r(&m, &press(KEY_XF86Switch_VT_2, 0)), KeyRoute::VtSwitch(2));
        // Escape alone, or with one of the two modifiers, is the receiver's.
        assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::Captured);
        assert_eq!(r(&m, &press(KEY_Escape, MOD_LOGO)), KeyRoute::Captured);
        assert_eq!(r(&Modes::default(), &chord), KeyRoute::Client);
        let locked = Modes { locked: true, ..m };
        assert_eq!(r(&locked, &chord), KeyRoute::Lock);
    }

    #[test]
    fn releases_pass_through_except_under_the_lock_and_the_switcher() {
        let mut m = Modes {
            keyboard_interaction: true,
            menus_open: true,
            dialog_has_keyboard: true,
            renaming: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &release(KEY_Return)), KeyRoute::Client);
        m.switcher_open = true;
        assert_eq!(r(&m, &release(KEY_Return)), KeyRoute::Swallow);
        m.locked = true;
        assert_eq!(r(&m, &release(KEY_Return)), KeyRoute::Lock);
    }

    #[test]
    fn switcher_keys() {
        let m = Modes {
            switcher_open: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::SwitcherCancel);
        assert_eq!(
            r(&m, &press(KEY_Tab, MOD_ALT)),
            KeyRoute::SwitcherStep { backwards: false }
        );
        assert_eq!(
            r(&m, &press(KEY_ISO_Left_Tab, MOD_ALT | MOD_SHIFT)),
            KeyRoute::SwitcherStep { backwards: true }
        );
        assert_eq!(r(&m, &press(KEY_a, 0)), KeyRoute::Swallow);
        assert_eq!(r(&m, &press(KEY_Right, MOD_ALT)), KeyRoute::Swallow);
    }

    #[test]
    fn the_3d_switcher_steps_with_its_bind_and_not_the_arrows() {
        let m = Modes {
            switcher_open: true,
            ..Modes::default()
        };
        assert_eq!(
            r(&m, &press(KEY_Escape, MOD_LOGO)),
            KeyRoute::SwitcherCancel
        );
        let step = |backwards| KeyRoute::SwitcherStep { backwards };
        assert_eq!(r(&m, &press(KEY_Tab, MOD_LOGO)), step(false));
        assert_eq!(
            r(&m, &press(KEY_ISO_Left_Tab, MOD_LOGO | MOD_SHIFT)),
            step(true)
        );
        for arrow in [KEY_Right, KEY_Down, KEY_Left, KEY_Up] {
            assert_eq!(r(&m, &press(arrow, MOD_LOGO)), KeyRoute::Swallow);
        }
        assert_eq!(r(&m, &press(KEY_a, MOD_LOGO)), KeyRoute::Swallow);
    }

    #[test]
    fn super_tap_arms_on_a_bare_press_and_fires_on_the_release() {
        let m = Modes::default();
        let bare = route(&m, &press(KEY_Super_L, MOD_LOGO), &binds(), &[]);
        assert_eq!(bare.arm_super, Some(true));
        assert!(!bare.fire_super);
        // Any other press disarms, and a Super+x chord never arms.
        assert_eq!(
            route(&m, &press(KEY_a, 0), &binds(), &[]).arm_super,
            Some(false)
        );
        assert_eq!(
            route(&m, &press(KEY_r, MOD_LOGO), &binds(), &[]).arm_super,
            Some(false)
        );
        let armed = Modes {
            super_tap_armed: true,
            ..Modes::default()
        };
        let up = route(&armed, &release(KEY_Super_L), &binds(), &[]);
        assert!(up.fire_super);
        assert_eq!(up.route, KeyRoute::Client);
        assert!(!route(&m, &release(KEY_Super_L), &binds(), &[]).fire_super);
        // The switcher swallows the press before the tap logic sees it.
        let sw = Modes {
            switcher_open: true,
            ..Modes::default()
        };
        assert_eq!(
            route(&sw, &press(KEY_Super_L, MOD_LOGO), &binds(), &[]).arm_super,
            None
        );
    }

    #[test]
    fn a_menu_skips_a_super_keysym_for_its_own_key() {
        let m = Modes {
            menus_open: true,
            ..Modes::default()
        };
        let k = KeyPress {
            syms: vec![KEY_Super_L, KEY_Down],
            lower: vec![KEY_Super_L, KEY_Down],
            mods: MOD_LOGO,
            pressed: true,
        };
        assert_eq!(r(&m, &k), KeyRoute::Menu(KEY_Down));
        assert_eq!(r(&m, &press(KEY_Super_L, MOD_LOGO)), KeyRoute::Swallow);
    }

    fn menu_binds() -> Vec<Keybind> {
        let bind = |mods, keysym, action| Keybind {
            mods,
            keysym,
            action,
        };
        vec![
            bind(MOD_LOGO, KEY_l, Action::Lock),
            bind(MOD_LOGO, KEY_d, Action::ShowDesktop),
            bind(MOD_LOGO, KEY_Left, Action::Snap(crate::snap::Dir::Left)),
            bind(MOD_CTRL, KEY_Escape, Action::StartMenu),
            bind(MOD_ALT, KEY_F4, Action::Close),
            bind(0, KEY_Print, Action::Screenshot),
        ]
    }

    #[test]
    fn the_close_bind_on_the_desktop_opens_turn_off() {
        let alt_f4 = press(KEY_F4, MOD_ALT);
        let desktop = Modes {
            desktop_has_keyboard: true,
            has_icons: true,
            selected_icon: Some(0),
            ..Modes::default()
        };
        let routed = route(&desktop, &alt_f4, &menu_binds(), &[]);
        assert_eq!(routed.route, KeyRoute::Binds(vec![Action::TurnOff]));
        assert!(routed.route.opens_surface());
        assert_eq!(
            route(&Modes::default(), &alt_f4, &menu_binds(), &[]).route,
            KeyRoute::Binds(vec![Action::Close]),
            "a focused window keeps the close bind"
        );
    }

    #[test]
    fn a_modified_bind_runs_over_an_open_menu() {
        let m = Modes {
            menus_open: true,
            ..Modes::default()
        };
        let rm = |k: &KeyPress| route(&m, k, &menu_binds(), &[]).route;
        for (sym, mods, a) in [
            (KEY_l, MOD_LOGO, Action::Lock),
            (KEY_d, MOD_LOGO, Action::ShowDesktop),
            (KEY_Left, MOD_LOGO, Action::Snap(crate::snap::Dir::Left)),
            (KEY_Escape, MOD_CTRL, Action::StartMenu),
            (KEY_F4, MOD_ALT, Action::Close),
        ] {
            assert_eq!(rm(&press(sym, mods)), KeyRoute::Binds(vec![a]));
        }
        assert_eq!(
            rm(&press(KEY_Print, 0)),
            KeyRoute::Menu(KEY_Print),
            "a bare bind stays the menu's"
        );
    }

    #[test]
    fn a_letter_under_ctrl_or_super_is_no_access_key() {
        let m = Modes {
            menus_open: true,
            ..Modes::default()
        };
        let rm = |m: &Modes, k: &KeyPress| route(m, k, &menu_binds(), &[]).route;
        assert_eq!(rm(&m, &press(KEY_r, MOD_CTRL)), KeyRoute::Swallow);
        assert_eq!(rm(&m, &press(KEY_r, MOD_LOGO)), KeyRoute::Swallow);
        assert_eq!(rm(&m, &press(KEY_1, MOD_CTRL)), KeyRoute::Swallow);
        assert_eq!(rm(&m, &press(KEY_r, 0)), KeyRoute::Menu(KEY_r));
        assert_eq!(rm(&m, &press(KEY_R, MOD_SHIFT)), KeyRoute::Menu(KEY_R));
        assert_eq!(rm(&m, &press(KEY_r, MOD_ALT)), KeyRoute::Menu(KEY_r));
        assert_eq!(rm(&m, &press(KEY_Down, MOD_CTRL)), KeyRoute::Menu(KEY_Down));
        let drag = Modes {
            dragging: true,
            ..m.clone()
        };
        assert_eq!(
            rm(&drag, &press(KEY_r, MOD_CTRL)),
            KeyRoute::Menu(KEY_r),
            "Ctrl held for a copy keeps the access keys"
        );
        assert_eq!(rm(&drag, &press(KEY_r, MOD_LOGO)), KeyRoute::Swallow);
    }

    #[test]
    fn during_a_drag_only_drag_binds_pass_a_menu() {
        let m = Modes {
            menus_open: true,
            dragging: true,
            ..Modes::default()
        };
        let rm = |k: &KeyPress| route(&m, k, &menu_binds(), &[]).route;
        assert_eq!(
            rm(&press(KEY_d, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::ShowDesktop])
        );
        assert_eq!(rm(&press(KEY_l, MOD_LOGO)), KeyRoute::Swallow);
        assert_eq!(rm(&press(KEY_F4, MOD_ALT)), KeyRoute::Menu(KEY_F4));
    }

    #[test]
    fn a_welcome_panel_keeps_the_binds_from_a_menu() {
        let m = Modes {
            menus_open: true,
            dialog_has_keyboard: true,
            dialog_modal: true,
            ..Modes::default()
        };
        let rm = |k: &KeyPress| route(&m, k, &menu_binds(), &[]).route;
        assert_eq!(rm(&press(KEY_l, MOD_LOGO)), KeyRoute::Swallow);
        assert_eq!(rm(&press(KEY_Left, MOD_LOGO)), KeyRoute::Menu(KEY_Left));
    }

    #[test]
    fn media_keys_route_before_everything_and_only_on_press() {
        let all = Modes {
            locked: true,
            captured: true,
            menus_open: true,
            dialog_has_keyboard: true,
            switcher_open: true,
            ..Modes::default()
        };
        assert_eq!(
            r(&all, &press(KEY_XF86AudioRaiseVolume, 0)),
            KeyRoute::Media(Action::VolumeUp)
        );
        assert_eq!(
            r(&all, &press(KEY_XF86MonBrightnessDown, 0)),
            KeyRoute::Media(Action::BrightnessDown)
        );
        assert_eq!(
            r(&all, &press(KEY_XF86AudioPause, 0)),
            KeyRoute::Media(Action::PlayPause)
        );
        assert_eq!(r(&all, &release(KEY_XF86AudioMute)), KeyRoute::Lock);
    }

    #[test]
    fn the_wifi_key_goes_to_the_lock_while_locked() {
        let locked = Modes {
            locked: true,
            ..Modes::default()
        };
        assert_eq!(r(&locked, &press(KEY_XF86WLAN, 0)), KeyRoute::Lock);
        assert_eq!(
            r(&Modes::default(), &press(KEY_XF86WLAN, 0)),
            KeyRoute::Media(Action::Wifi)
        );
    }

    #[test]
    fn a_config_bind_on_a_media_key_wins_over_the_default() {
        let binds = vec![Keybind {
            mods: 0,
            keysym: KEY_XF86AudioMute,
            action: Action::Lock,
        }];
        let m = Modes::default();
        assert_eq!(
            route(&m, &press(KEY_XF86AudioMute, 0), &binds, &[]).route,
            KeyRoute::Binds(vec![Action::Lock])
        );
        // With a modifier held the default no longer applies either: the
        // key is a chord now, and chords are the binds' business.
        assert_eq!(
            route(&m, &press(KEY_XF86AudioMute, MOD_LOGO), &binds, &[]).route,
            KeyRoute::Client
        );
    }

    #[test]
    fn an_open_start_panel_takes_every_press_but_a_menu_over_it_comes_first() {
        let m = Modes {
            start_panel_open: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_n, 0)), KeyRoute::StartPanel);
        assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::StartPanel);
        let m = Modes {
            start_panel_open: true,
            menus_open: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::Menu(KEY_Escape));
    }

    #[test]
    fn the_super_tap_still_fires_over_an_open_start_panel() {
        let m = Modes {
            start_panel_open: true,
            ..Modes::default()
        };
        let down = route(&m, &press(KEY_Super_L, MOD_LOGO), &binds(), &[]);
        assert_eq!(down.route, KeyRoute::StartPanel);
        assert_eq!(
            down.arm_super,
            Some(true),
            "a bare Super press arms the tap"
        );
        let armed = Modes {
            super_tap_armed: true,
            ..m
        };
        assert!(route(&armed, &release(KEY_Super_L), &binds(), &[]).fire_super);
    }

    #[test]
    fn an_open_launcher_takes_every_press_except_its_own_bind() {
        let m = Modes {
            launcher_open: true,
            dialog_has_keyboard: true,
            renaming: true,
            desktop_has_keyboard: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Return, 0)), KeyRoute::Launcher);
        assert_eq!(r(&m, &press(KEY_n, 0)), KeyRoute::Launcher);
        assert_eq!(
            r(&m, &press(KEY_space, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::Launcher])
        );
        let menus = Modes {
            menus_open: true,
            ..m.clone()
        };
        assert_eq!(r(&menus, &press(KEY_Return, 0)), KeyRoute::Menu(KEY_Return));
        assert_eq!(r(&m, &release(KEY_n)), KeyRoute::Client);
    }

    #[test]
    fn the_open_clipboard_history_takes_every_press_except_its_own_bind() {
        let m = Modes {
            clipview_open: true,
            dialog_has_keyboard: true,
            desktop_has_keyboard: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Return, 0)), KeyRoute::ClipView);
        assert_eq!(r(&m, &press(KEY_Delete, 0)), KeyRoute::ClipView);
        assert_eq!(r(&m, &press(KEY_space, MOD_LOGO)), KeyRoute::ClipView);
        assert_eq!(
            r(&m, &press(KEY_v, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::ClipboardHistory])
        );
        assert_eq!(r(&m, &release(KEY_v)), KeyRoute::Client);
        assert_eq!(
            r(&Modes::default(), &press(KEY_v, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::ClipboardHistory])
        );
    }

    #[test]
    fn binds_and_global_shortcuts_run_over_the_displays_panel() {
        let m = Modes {
            displays_open: true,
            ..Modes::default()
        };
        let rd = |k: &KeyPress| route(&m, k, &menu_binds(), &talk()).route;
        assert_eq!(
            rd(&press(KEY_l, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::Lock])
        );
        assert_eq!(
            rd(&press(KEY_Print, 0)),
            KeyRoute::Binds(vec![Action::Screenshot])
        );
        assert_eq!(rd(&press(KEY_Pause, 0)), hit());
        assert_eq!(rd(&press(KEY_Right, 0)), KeyRoute::Displays);
    }

    #[test]
    fn the_displays_panel_takes_the_keys_but_not_the_switcher() {
        let m = Modes {
            displays_open: true,
            ..Modes::default()
        };
        for sym in [KEY_Return, KEY_Tab, KEY_Escape, KEY_Left] {
            assert_eq!(r(&m, &press(sym, 0)), KeyRoute::Displays, "{sym:#x}");
        }
        assert_eq!(
            r(&m, &press(KEY_Tab, MOD_ALT)),
            KeyRoute::Binds(vec![Action::Cycle]),
            "Alt+Tab still switches"
        );
        assert_eq!(r(&m, &release(KEY_Left)), KeyRoute::Client);
    }

    #[test]
    fn the_open_emoji_picker_takes_every_press_except_its_own_bind() {
        let m = Modes {
            emojiview_open: true,
            desktop_has_keyboard: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Return, 0)), KeyRoute::EmojiView);
        assert_eq!(r(&m, &press(KEY_Left, 0)), KeyRoute::EmojiView);
        assert_eq!(r(&m, &press(KEY_v, MOD_LOGO)), KeyRoute::EmojiView);
        assert_eq!(
            r(&m, &press(KEY_period, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::EmojiPicker])
        );
        assert_eq!(r(&m, &release(KEY_period)), KeyRoute::Client);
        assert_eq!(
            r(&Modes::default(), &press(KEY_period, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::EmojiPicker])
        );
    }

    /// A balloon's reply field, once clicked, takes every press, binds
    /// and Escape included, except Alt+Tab; unclicked, it takes none.
    #[test]
    fn a_clicked_reply_field_takes_every_press_but_the_switcher() {
        let m = Modes {
            notify_reply_focused: true,
            desktop_has_keyboard: true,
            ..Modes::default()
        };
        for (sym, mods) in [
            (KEY_a, 0),
            (KEY_Return, 0),
            (KEY_Escape, 0),
            (KEY_r, MOD_LOGO),
        ] {
            assert_eq!(r(&m, &press(sym, mods)), KeyRoute::NotifyReply, "{sym:#x}");
        }
        assert_eq!(
            r(&m, &press(KEY_Tab, MOD_ALT)),
            KeyRoute::Binds(vec![Action::Cycle])
        );
        let unclicked = Modes {
            desktop_has_keyboard: true,
            ..Modes::default()
        };
        assert_ne!(r(&unclicked, &press(KEY_a, 0)), KeyRoute::NotifyReply);
    }

    #[test]
    fn the_notification_history_takes_every_press_but_a_bind() {
        let m = Modes {
            notify_center_open: true,
            desktop_has_keyboard: true,
            has_icons: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Down, 0)), KeyRoute::NotifyCenter(KEY_Down));
        assert_eq!(
            r(&m, &press(KEY_Escape, 0)),
            KeyRoute::NotifyCenter(KEY_Escape)
        );
        assert_eq!(
            r(&m, &press(KEY_Tab, MOD_ALT)),
            KeyRoute::Binds(vec![Action::Cycle])
        );
        let menus = Modes {
            menus_open: true,
            ..m.clone()
        };
        assert_eq!(r(&menus, &press(KEY_Escape, 0)), KeyRoute::Menu(KEY_Escape));
        assert_eq!(r(&m, &release(KEY_Down)), KeyRoute::Client);
    }

    #[test]
    fn the_region_overlay_takes_every_press_after_the_lock_and_a_capture() {
        let m = Modes {
            region_open: true,
            keyboard_interaction: true,
            menus_open: true,
            launcher_open: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::Region(KEY_Escape));
        assert_eq!(r(&m, &press(KEY_r, MOD_LOGO)), KeyRoute::Region(KEY_r));
        assert_eq!(r(&m, &release(KEY_Escape)), KeyRoute::Client);
        let locked = Modes {
            locked: true,
            ..m.clone()
        };
        assert_eq!(r(&locked, &press(KEY_Escape, 0)), KeyRoute::Lock);
        let captured = Modes {
            captured: true,
            ..m
        };
        assert_eq!(r(&captured, &press(KEY_Escape, 0)), KeyRoute::Captured);
    }

    #[test]
    fn escape_cancels_a_bar_drag_and_other_keys_pass() {
        let m = Modes {
            bar_drag: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::BarDragCancel);
        assert_eq!(r(&m, &press(KEY_n, 0)), KeyRoute::Client);
        assert_eq!(r(&m, &release(KEY_Escape)), KeyRoute::Client);
        // An open menu takes the Escape first.
        let menu = Modes {
            bar_drag: true,
            menus_open: true,
            ..Modes::default()
        };
        assert_ne!(r(&menu, &press(KEY_Escape, 0)), KeyRoute::BarDragCancel);
    }

    #[test]
    fn escape_cancels_a_pointer_move_and_other_keys_pass() {
        let m = Modes {
            pointer_move: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::MoveCancel);
        assert_eq!(r(&m, &press(KEY_n, 0)), KeyRoute::Client);
        assert_eq!(r(&m, &release(KEY_Escape)), KeyRoute::Client);
        let menu = Modes {
            pointer_move: true,
            menus_open: true,
            ..Modes::default()
        };
        assert_ne!(r(&menu, &press(KEY_Escape, 0)), KeyRoute::MoveCancel);
    }

    #[test]
    fn escape_closes_the_clock_popup_and_other_keys_pass() {
        let m = Modes {
            calendar_open: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::CalendarClose);
        assert_eq!(r(&m, &press(KEY_n, 0)), KeyRoute::Client);
        assert_eq!(r(&m, &release(KEY_Escape)), KeyRoute::Client);
    }

    #[test]
    fn escape_cancels_a_drag_and_no_client_hears_a_press() {
        let m = Modes {
            dragging: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::DragCancel);
        // Held with the copy modifier too, and ahead of Ctrl+Escape's
        // Start menu.
        let mut b = binds();
        b.push(Keybind {
            mods: MOD_CTRL,
            keysym: KEY_Escape,
            action: Action::StartMenu,
        });
        let ctrl_escape = press(KEY_Escape, MOD_CTRL);
        assert_eq!(route(&m, &ctrl_escape, &b, &[]).route, KeyRoute::DragCancel);
        assert_eq!(r(&m, &press(KEY_n, 0)), KeyRoute::Swallow);
        assert_eq!(r(&m, &release(KEY_n)), KeyRoute::Client);
        // Binds with nothing to drop on wait for the drop; the launcher's
        // and Run's fields take it.
        let routed = route(&m, &press(KEY_v, MOD_LOGO), &binds(), &[]);
        assert_eq!(routed.route, KeyRoute::Swallow);
        assert_eq!(routed.arm_super, Some(false));
        assert_eq!(
            r(&m, &press(KEY_space, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::Launcher])
        );
        assert_eq!(
            r(&m, &press(KEY_r, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::Run])
        );
    }

    #[test]
    fn alt_tab_and_the_super_tap_work_during_a_drag() {
        let m = Modes {
            dragging: true,
            ..Modes::default()
        };
        assert_eq!(
            r(&m, &press(KEY_Tab, MOD_ALT)),
            KeyRoute::Binds(vec![Action::Cycle])
        );
        let down = route(&m, &press(KEY_Super_L, 0), &binds(), &[]);
        assert_eq!(down.route, KeyRoute::Swallow);
        assert_eq!(down.arm_super, Some(true));
        let armed = Modes {
            super_tap_armed: true,
            ..m.clone()
        };
        assert!(route(&armed, &release(KEY_Super_L), &binds(), &[]).fire_super);
        // A client inhibiting shortcuts hears no key during a drag, so the
        // binds are Reprise's then too.
        let inhibited = Modes {
            shortcuts_inhibited: true,
            ..m
        };
        assert_eq!(
            r(&inhibited, &press(KEY_Tab, MOD_ALT)),
            KeyRoute::Binds(vec![Action::Cycle])
        );
        let down = route(&inhibited, &press(KEY_Super_L, 0), &binds(), &[]);
        assert_eq!(down.arm_super, Some(true));
        let armed = Modes {
            super_tap_armed: true,
            ..inhibited
        };
        assert!(route(&armed, &release(KEY_Super_L), &binds(), &[]).fire_super);
    }

    #[test]
    fn the_switcher_and_the_menus_keep_their_keys_during_a_drag() {
        let switcher = Modes {
            dragging: true,
            switcher_open: true,
            ..Modes::default()
        };
        assert_eq!(
            r(&switcher, &press(KEY_Tab, MOD_ALT)),
            KeyRoute::SwitcherStep { backwards: false }
        );
        assert_eq!(
            r(&switcher, &press(KEY_Escape, MOD_ALT)),
            KeyRoute::SwitcherCancel
        );
        assert_eq!(r(&switcher, &press(KEY_n, 0)), KeyRoute::Swallow);
        let menu = Modes {
            dragging: true,
            menus_open: true,
            ..Modes::default()
        };
        assert_eq!(r(&menu, &press(KEY_Escape, 0)), KeyRoute::Menu(KEY_Escape));
        assert_eq!(r(&menu, &press(KEY_Down, 0)), KeyRoute::Menu(KEY_Down));
    }

    /// Mid-drag with Start open, Shift pressed over a held Ctrl and the
    /// menu's arrows leave Ctrl picking the drop; a bind's own modifier
    /// and the switcher's are spent.
    #[test]
    fn a_menu_key_mid_drag_spends_no_held_modifier() {
        let menu = Modes {
            dragging: true,
            menus_open: true,
            ..Modes::default()
        };
        let spends = |m: &Modes, k: &KeyPress| drag_spends(&r(m, k), k.mods);
        let ctrl = MOD_CTRL;
        assert_eq!(spends(&menu, &press(KEY_Shift_L, ctrl)), 0);
        assert_eq!(spends(&menu, &press(KEY_Down, ctrl)), 0);
        assert_eq!(spends(&menu, &press(KEY_Return, ctrl)), 0);
        let switcher = Modes {
            dragging: true,
            switcher_open: true,
            ..Modes::default()
        };
        let alt = MOD_ALT;
        assert_eq!(spends(&switcher, &press(KEY_Tab, alt)), alt);
        assert_eq!(drag_spends(&KeyRoute::Binds(vec![Action::Cycle]), alt), alt);
        assert_eq!(drag_spends(&KeyRoute::Swallow, ctrl), 0);
    }

    #[test]
    fn only_the_binds_that_bring_a_target_within_reach_run_during_a_drag() {
        use crate::desktops::Step;
        use crate::magnifier::ZoomStep;
        use crate::snap::Dir;
        for a in [
            Action::Cycle,
            Action::StartMenu,
            Action::Launcher,
            Action::Run,
            Action::ShowDesktop,
            Action::Minimize,
            Action::Maximize,
            Action::Snap(Dir::Left),
            Action::Desktop(Step::Next),
            Action::Zoom(ZoomStep::In),
        ] {
            assert!(runs_during_drag(&a), "{a:?} runs during a drag");
        }
        for a in [
            Action::Close,
            Action::Exit,
            Action::Lock,
            Action::WindowMenu,
            Action::CarryToDesktop(Step::Next),
            Action::Screenshot,
            Action::ScreenshotRegion,
            Action::Terminal,
        ] {
            assert!(!runs_during_drag(&a), "{a:?} waits for the drop");
        }
    }

    #[test]
    fn a_bind_that_opens_a_surface_ends_a_held_gesture() {
        use crate::desktops::Step;
        use crate::magnifier::ZoomStep;
        use crate::snap::Dir;
        let menus = Modes {
            menus_open: true,
            ..Modes::default()
        };
        for m in [Modes::default(), menus] {
            for sym in [KEY_space, KEY_v, KEY_period, KEY_r] {
                assert!(
                    r(&m, &press(sym, MOD_LOGO)).opens_surface(),
                    "Super+{sym:#x} opens a surface"
                );
            }
        }
        assert!(!r(&Modes::default(), &press(KEY_a, 0)).opens_surface());
        for a in [
            Action::StartMenu,
            Action::WindowMenu,
            Action::FocusTray,
            Action::SnapLayouts,
        ] {
            assert!(opens_surface(&a), "{a:?} opens a surface");
        }
        for a in [
            Action::VolumeUp,
            Action::Zoom(ZoomStep::In),
            Action::Snap(Dir::Left),
            Action::Desktop(Step::Next),
            Action::Screenshot,
        ] {
            assert!(!opens_surface(&a), "{a:?} keeps the gesture");
        }
    }

    #[test]
    fn no_chord_fires_during_a_drag() {
        let m = Modes {
            dragging: true,
            ..Modes::default()
        };
        assert!(!chord_allowed(&m, &Action::NextLayout));
        assert!(chord_allowed(&Modes::default(), &Action::NextLayout));
    }

    #[test]
    fn modifiers_pressed_during_a_drag_fire_no_chord_after_it() {
        // Shift and Alt pressed while the drag runs, each press spoiling the
        // chord, then released once it has ended.
        let mut st = ChordState::default();
        let mut fired = feed(&mut st, &[(SHIFT, KEY_Shift_L, true)]);
        st.spoil();
        fired.extend(feed(&mut st, &[(ALT, KEY_Alt_L, true)]));
        st.spoil();
        fired.extend(feed(
            &mut st,
            &[(ALT, KEY_Alt_L, false), (SHIFT, KEY_Shift_L, false)],
        ));
        assert_eq!(fired, vec![], "no layout switch");
        let fired = feed(
            &mut st,
            &[
                (SHIFT, KEY_Shift_L, true),
                (ALT, KEY_Alt_L, true),
                (ALT, KEY_Alt_L, false),
                (SHIFT, KEY_Shift_L, false),
            ],
        );
        assert_eq!(fired, vec![NEXT]);
    }

    #[test]
    fn snap_assist_lets_the_snap_binds_through_and_the_overview_does_not() {
        use crate::snap::Dir;
        let bind = vec![Keybind {
            mods: MOD_LOGO,
            keysym: KEY_Left,
            action: Action::Snap(Dir::Left),
        }];
        let overview = Modes {
            overview_open: true,
            ..Modes::default()
        };
        let assist = Modes {
            snap_assist_open: true,
            ..overview.clone()
        };
        let snap = press(KEY_Left, MOD_LOGO);
        assert_eq!(
            route(&assist, &snap, &bind, &[]).route,
            KeyRoute::Binds(vec![Action::Snap(Dir::Left)])
        );
        assert_eq!(
            route(&assist, &press(KEY_Left, 0), &bind, &[]).route,
            KeyRoute::Overview,
            "a bare arrow walks the cards"
        );
        assert_eq!(
            route(&overview, &snap, &bind, &[]).route,
            KeyRoute::Overview
        );
    }

    #[test]
    fn the_snap_picker_takes_every_key_from_its_bind_and_only_escape_from_the_pointer() {
        let keys = Modes {
            snap_picker_open: true,
            snap_picker_keys: true,
            ..Modes::default()
        };
        assert_eq!(
            r(&keys, &press(KEY_Left, 0)),
            KeyRoute::SnapPicker(KEY_Left)
        );
        assert_eq!(r(&keys, &press(KEY_n, 0)), KeyRoute::SnapPicker(KEY_n));
        let bind = vec![Keybind {
            mods: MOD_LOGO,
            keysym: KEY_z,
            action: Action::SnapLayouts,
        }];
        let again = route(&keys, &press(KEY_z, MOD_LOGO), &bind, &[]).route;
        assert_eq!(again, KeyRoute::Binds(vec![Action::SnapLayouts]));
        let hover = Modes {
            snap_picker_open: true,
            ..Modes::default()
        };
        assert_eq!(
            r(&hover, &press(KEY_Escape, 0)),
            KeyRoute::SnapPicker(KEY_Escape)
        );
        assert_eq!(r(&hover, &press(KEY_Left, 0)), KeyRoute::Client);
    }

    #[test]
    fn escape_closes_the_hidden_icons_flyout_and_other_keys_pass() {
        let m = Modes {
            tray_flyout_open: true,
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::TrayFlyoutClose);
        assert_eq!(r(&m, &press(KEY_n, 0)), KeyRoute::Client);
    }

    #[test]
    fn an_inhibiting_client_gets_the_binds_and_super_but_nothing_ahead_of_them() {
        let m = Modes {
            can_switch_vt: true,
            shortcuts_inhibited: true,
            ..Modes::default()
        };
        let alt_tab = press(KEY_Tab, MOD_ALT);
        assert_eq!(r(&m, &alt_tab), KeyRoute::Client);
        assert_eq!(r(&m, &press(KEY_r, MOD_LOGO)), KeyRoute::Client);
        let sup = route(&m, &press(KEY_Super_L, MOD_LOGO), &binds(), &[]);
        assert_eq!(sup.route, KeyRoute::Client);
        assert_eq!(sup.arm_super, Some(false));
        let armed = Modes {
            super_tap_armed: true,
            ..m.clone()
        };
        assert!(!route(&armed, &release(KEY_Super_L), &binds(), &[]).fire_super);
        // VT switches, the machine's keys, the lock and an open menu still
        // come first.
        assert_eq!(r(&m, &press(KEY_XF86Switch_VT_2, 0)), KeyRoute::VtSwitch(2));
        assert_eq!(
            r(&m, &press(KEY_XF86AudioMute, 0)),
            KeyRoute::Media(Action::Mute)
        );
        let locked = Modes {
            locked: true,
            ..m.clone()
        };
        assert_eq!(r(&locked, &alt_tab), KeyRoute::Lock);
        let menus = Modes {
            menus_open: true,
            ..m.clone()
        };
        assert_eq!(r(&menus, &press(KEY_Return, 0)), KeyRoute::Menu(KEY_Return));
    }

    #[test]
    fn super_shift_escape_suspends_a_shortcuts_inhibitor() {
        let m = Modes {
            shortcuts_inhibited: true,
            ..Modes::default()
        };
        let chord = press(KEY_Escape, MOD_LOGO | MOD_SHIFT);
        let routed = route(&m, &chord, &binds(), &[]);
        assert_eq!(routed.route, KeyRoute::InhibitEscape);
        assert_eq!(routed.arm_super, Some(false));
        assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::Client);
        // A capture takes the chord first.
        let captured = Modes {
            captured: true,
            ..m
        };
        assert_eq!(r(&captured, &chord), KeyRoute::CaptureEscape);
    }

    /// Keycodes for the chord tests: the left and right Alt and Shift,
    /// Ctrl, Tab and a letter.
    const ALT: u32 = 64;
    const RALT: u32 = 108;
    const SHIFT: u32 = 50;
    const RSHIFT: u32 = 62;
    const CTRL: u32 = 37;
    const TAB: u32 = 23;
    const F: u32 = 41;

    fn chord_binds() -> Vec<Keybind> {
        let mut b = binds();
        b.push(Keybind {
            mods: MOD_SHIFT,
            keysym: KEY_Alt_L,
            action: Action::NextLayout,
        });
        b
    }

    /// Feed `(keycode, keysym, pressed)` events; the actions fired, in order.
    fn feed(st: &mut ChordState, events: &[(u32, u32, bool)]) -> Vec<Action> {
        let binds = chord_binds();
        events
            .iter()
            .filter_map(|&(code, sym, down)| {
                let k = if down { press(sym, 0) } else { release(sym) };
                st.key(code, &k, &binds)
            })
            .collect()
    }

    const NEXT: Action = Action::NextLayout;

    #[test]
    fn left_alt_and_shift_fire_on_the_first_release_in_either_order() {
        let mut st = ChordState::default();
        let fired = feed(
            &mut st,
            &[
                (ALT, KEY_Alt_L, true),
                (SHIFT, KEY_Shift_L, true),
                (SHIFT, KEY_Shift_L, false),
                (ALT, KEY_Alt_L, false),
            ],
        );
        assert_eq!(fired, vec![NEXT], "once, on the Shift release");
        // Shift first: the Alt key then gives Meta_L, and the Alt release fires.
        let fired = feed(
            &mut st,
            &[
                (RSHIFT, KEY_Shift_R, true),
                (ALT, KEY_Meta_L, true),
                (ALT, KEY_Meta_L, false),
                (RSHIFT, KEY_Shift_R, false),
            ],
        );
        assert_eq!(fired, vec![NEXT]);
    }

    #[test]
    fn each_shift_tap_under_a_held_alt_fires_again() {
        let mut st = ChordState::default();
        let fired = feed(
            &mut st,
            &[
                (ALT, KEY_Alt_L, true),
                (SHIFT, KEY_Shift_L, true),
                (SHIFT, KEY_Shift_L, false),
                (SHIFT, KEY_Shift_L, true),
                (SHIFT, KEY_Shift_L, false),
                (ALT, KEY_Alt_L, false),
            ],
        );
        assert_eq!(fired, vec![NEXT, NEXT]);
    }

    #[test]
    fn another_key_or_a_button_in_between_cancels_the_chord() {
        let mut st = ChordState::default();
        // Alt+Shift+Tab stays the switcher's.
        let fired = feed(
            &mut st,
            &[
                (ALT, KEY_Alt_L, true),
                (SHIFT, KEY_Shift_L, true),
                (TAB, KEY_ISO_Left_Tab, true),
                (TAB, KEY_ISO_Left_Tab, false),
                (SHIFT, KEY_Shift_L, false),
                (ALT, KEY_Alt_L, false),
            ],
        );
        assert!(fired.is_empty());
        feed(
            &mut st,
            &[(ALT, KEY_Alt_L, true), (SHIFT, KEY_Shift_L, true)],
        );
        st.spoil();
        let fired = feed(
            &mut st,
            &[(SHIFT, KEY_Shift_L, false), (ALT, KEY_Alt_L, false)],
        );
        assert!(fired.is_empty(), "a click in between");
        let fired = feed(
            &mut st,
            &[
                (F, KEY_f, true),
                (F, KEY_f, false),
                (ALT, KEY_Alt_L, true),
                (SHIFT, KEY_Shift_L, true),
                (SHIFT, KEY_Shift_L, false),
            ],
        );
        assert_eq!(fired, vec![NEXT]);
    }

    #[test]
    fn a_third_modifier_or_the_right_alt_is_not_the_chord() {
        let mut st = ChordState::default();
        let fired = feed(
            &mut st,
            &[
                (ALT, KEY_Alt_L, true),
                (SHIFT, KEY_Shift_L, true),
                (CTRL, KEY_Control_L, true),
                (CTRL, KEY_Control_L, false),
                (SHIFT, KEY_Shift_L, false),
                (ALT, KEY_Alt_L, false),
            ],
        );
        assert!(fired.is_empty(), "Ctrl joined an armed chord");
        let fired = feed(
            &mut st,
            &[
                (CTRL, KEY_Control_L, true),
                (ALT, KEY_Alt_L, true),
                (SHIFT, KEY_Shift_L, true),
                (SHIFT, KEY_Shift_L, false),
                (ALT, KEY_Alt_L, false),
                (CTRL, KEY_Control_L, false),
            ],
        );
        assert!(fired.is_empty(), "Ctrl+Alt+Shift");
        let fired = feed(
            &mut st,
            &[
                (RALT, KEY_Alt_R, true),
                (SHIFT, KEY_Shift_L, true),
                (SHIFT, KEY_Shift_L, false),
                (RALT, KEY_Alt_R, false),
            ],
        );
        assert!(fired.is_empty(), "the bind names the left Alt");
    }

    #[test]
    fn a_chord_bind_never_matches_as_a_press() {
        let m = Modes::default();
        let k = press(KEY_Alt_L, MOD_SHIFT);
        assert_eq!(route(&m, &k, &chord_binds(), &[]).route, KeyRoute::Client);
    }

    #[test]
    fn a_chord_runs_where_binds_do_and_the_layout_switch_at_the_lock() {
        let m = Modes::default();
        assert!(chord_allowed(&m, &NEXT));
        let locked = Modes {
            locked: true,
            ..Modes::default()
        };
        assert!(chord_allowed(&locked, &NEXT));
        assert!(!chord_allowed(&locked, &Action::Terminal));
        for m in [
            Modes {
                captured: true,
                ..Modes::default()
            },
            Modes {
                shortcuts_inhibited: true,
                ..Modes::default()
            },
        ] {
            assert!(!chord_allowed(&m, &NEXT));
        }
    }

    #[test]
    fn alt_enter_on_a_selected_icon_is_its_properties_and_enter_opens() {
        let m = Modes {
            desktop_has_keyboard: true,
            has_icons: true,
            selected_icon: Some(2),
            ..Modes::default()
        };
        for enter in [KEY_Return, KEY_KP_Enter] {
            assert_eq!(r(&m, &press(enter, 0)), KeyRoute::IconOpen(2));
            assert_eq!(
                r(&m, &press(enter, MOD_ALT)),
                KeyRoute::IconProperties(2),
                "Alt+Enter shows the icon's Properties and opens nothing"
            );
        }
    }

    #[test]
    fn icons_need_the_desktop_to_hold_the_keyboard() {
        let m = Modes {
            has_icons: true,
            selected_icon: Some(2),
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_F2, 0)), KeyRoute::Client);
        let m = Modes {
            desktop_has_keyboard: true,
            ..m
        };
        assert_eq!(r(&m, &press(KEY_F2, 0)), KeyRoute::IconRename(2));
        assert_eq!(r(&m, &press(KEY_Delete, 0)), KeyRoute::IconTrash(2));
        assert_eq!(
            r(&m, &press(KEY_Home, 0)),
            KeyRoute::IconMove(IconMove::Home)
        );
    }

    #[test]
    fn the_desktop_takes_its_menu_clipboard_and_refresh_keys() {
        let m = Modes {
            desktop_has_keyboard: true,
            has_icons: true,
            selected_icon: Some(2),
            ..Modes::default()
        };
        assert_eq!(r(&m, &press(KEY_Menu, 0)), KeyRoute::IconMenu(Some(2)));
        assert_eq!(
            r(&m, &press(KEY_F10, MOD_SHIFT)),
            KeyRoute::IconMenu(Some(2))
        );
        assert_eq!(r(&m, &press(KEY_F10, 0)), KeyRoute::Client);
        assert_eq!(r(&m, &press(KEY_a, MOD_CTRL)), KeyRoute::IconSelectAll);
        // Caps Lock gives the capital keysym; the route reads the lower.
        assert_eq!(r(&m, &press(KEY_C, MOD_CTRL)), KeyRoute::IconCopy(2));
        assert_eq!(r(&m, &press(KEY_x, MOD_CTRL)), KeyRoute::IconCut(2));
        assert_eq!(r(&m, &press(KEY_v, MOD_CTRL)), KeyRoute::IconPaste);
        assert_eq!(
            r(&m, &press(KEY_Delete, MOD_SHIFT)),
            KeyRoute::IconErase(2),
            "Shift+Delete deletes for good"
        );
        assert_eq!(r(&m, &press(KEY_v, 0)), KeyRoute::Client);
        // With nothing selected the menu is the desktop's, and Paste still
        // lands there.
        let none = Modes {
            selected_icon: None,
            ..m
        };
        assert_eq!(r(&none, &press(KEY_Menu, 0)), KeyRoute::IconMenu(None));
        assert_eq!(r(&none, &press(KEY_c, MOD_CTRL)), KeyRoute::Client);
        assert_eq!(r(&none, &press(KEY_v, MOD_CTRL)), KeyRoute::IconPaste);
        assert_eq!(r(&none, &press(KEY_F5, 0)), KeyRoute::IconRefresh);
    }

    #[test]
    fn a_bound_arrow_runs_its_bind_over_the_desktop_icons() {
        let m = Modes {
            desktop_has_keyboard: true,
            has_icons: true,
            ..Modes::default()
        };
        let mut binds = binds();
        binds.push(Keybind {
            mods: MOD_LOGO | MOD_CTRL,
            keysym: KEY_Right,
            action: Action::Desktop(crate::desktops::Step::Next),
        });
        let super_ctrl_right = press(KEY_Right, MOD_LOGO | MOD_CTRL);
        assert_eq!(
            route(&m, &super_ctrl_right, &binds, &[]).route,
            KeyRoute::Binds(vec![Action::Desktop(crate::desktops::Step::Next)])
        );
        assert_eq!(
            route(&m, &press(KEY_Right, 0), &binds, &[]).route,
            KeyRoute::IconMove(IconMove::Right)
        );
    }

    fn talk() -> Vec<GlobalBind> {
        vec![GlobalBind {
            mods: 0,
            keysym: KEY_Pause,
            app: "t".into(),
            id: "talk".into(),
        }]
    }
    fn hit() -> KeyRoute {
        KeyRoute::Global(vec![GlobalHit {
            app: "t".into(),
            id: "talk".into(),
        }])
    }

    #[test]
    fn a_live_global_chord_goes_to_its_program() {
        let m = Modes::default();
        assert_eq!(
            route(&m, &press(KEY_Pause, 0), &binds(), &talk()).route,
            hit()
        );
        assert_eq!(
            route(&m, &press(KEY_Pause, MOD_CTRL), &binds(), &talk()).route,
            KeyRoute::Client
        );
        assert_eq!(
            route(&m, &release(KEY_Pause), &binds(), &talk()).route,
            KeyRoute::Client,
            "releases are the held list's"
        );
    }

    #[test]
    fn a_keybind_beats_a_global_chord() {
        let g = vec![GlobalBind {
            mods: MOD_ALT,
            keysym: KEY_Tab,
            app: "t".into(),
            id: "x".into(),
        }];
        assert_eq!(
            route(&Modes::default(), &press(KEY_Tab, MOD_ALT), &binds(), &g).route,
            KeyRoute::Binds(vec![Action::Cycle])
        );
    }

    #[test]
    fn an_inhibitor_and_the_lock_come_before_a_global_chord() {
        let inhibited = Modes {
            shortcuts_inhibited: true,
            ..Modes::default()
        };
        assert_eq!(
            route(&inhibited, &press(KEY_Pause, 0), &binds(), &talk()).route,
            KeyRoute::Client
        );
        let locked = Modes {
            locked: true,
            ..Modes::default()
        };
        assert_eq!(
            route(&locked, &press(KEY_Pause, 0), &binds(), &talk()).route,
            KeyRoute::Lock
        );
    }

    #[test]
    fn a_global_chord_beats_the_icon_keys_and_the_notification_history() {
        let g = vec![GlobalBind {
            mods: MOD_CTRL,
            keysym: KEY_Delete,
            app: "t".into(),
            id: "x".into(),
        }];
        let x = KeyRoute::Global(vec![GlobalHit {
            app: "t".into(),
            id: "x".into(),
        }]);
        let ctrl_delete = press(KEY_Delete, MOD_CTRL);
        let desktop = Modes {
            desktop_has_keyboard: true,
            has_icons: true,
            selected_icon: Some(0),
            ..Modes::default()
        };
        assert_eq!(route(&desktop, &ctrl_delete, &binds(), &g).route, x);
        assert_eq!(
            route(&desktop, &ctrl_delete, &binds(), &[]).route,
            KeyRoute::IconTrash(0),
            "unbound, Ctrl+Delete trashes the icon"
        );
        let center = Modes {
            notify_center_open: true,
            ..Modes::default()
        };
        assert_eq!(route(&center, &ctrl_delete, &binds(), &g).route, x);
        assert_eq!(
            route(&center, &ctrl_delete, &binds(), &[]).route,
            KeyRoute::NotifyCenter(KEY_Delete)
        );
    }
}
