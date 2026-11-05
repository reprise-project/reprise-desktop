//! The binding between each control and its [`Settings`] field: writing, reading back and greying.

use super::appearance::{ANIM_CONTROLS, HOVER_CONTROLS, NL_CONTROLS, NL_TIMES};
use super::desktop::{
    DESKTOP_CONTROLS, DRAG_SNAP_CONTROLS, DURATION_CONTROLS, FPS_CONTROLS, REVEAL_CONTROLS,
};
use super::input::{MOUSE_KEYS_CONTROLS, SLOW_CONTROLS, STICKY_CONTROLS};
use super::system::{
    ALLOW_BOOST, BELL, NOTIFICATION_SOUND, QUALITY_VALUE, REMOTE_ENCODER, REMOTE_FRAME_RATE,
    REMOTE_PORT, REMOTE_QUALITY, REMOTE_SOUND, SOUND_CONTROLS,
};
use super::{
    CURSOR_FIXED_ROWS, DEBUG_IN_CAPTURES, DEBUG_OVERLAY, DELAY_MAX, DELAY_MIN, DOUBLE_CLICK,
    GTK_FIXED_ROWS, KB_ADD, KB_LAYOUTS, KB_REMOVE, KB_SWITCH_NOTE, KB_UP, KELVIN_MAX, KELVIN_MIN,
    KELVIN_STEP, MK_ACCEL, MK_SPEED, PRIVACY_LOCATION, SPEED_STEPS, Steps, THUMBNAILERS, THUMBS,
    THUMBS_MB, THUMBS_SIZE, TRAY_CUSTOMIZE, WATER, WATER_STYLE,
};
use crate::registry::Page;
use repriseconf::{Key, Lens, lens};
use reprisefs::xkb::{Layout, MAX_LAYOUTS};
use reprisekit::controls::combo;
use reprisekit::controls::form::{Control, Form};
use reprisekit::controls::listbox::ListBox;
use reprisekit::controls::propsheet::PropSheet;
use reprisekit::textedit::TextEdit;
use reprisekit::theme::Color;

use crate::model::{
    CURSOR_SIZES, Choices, DESKTOPS_DEFAULT, FONT_SMOOTHINGS, FRAME_RATES, HOT_CORNERS, INTERFACES,
    MENU_ANIMATIONS, PAGER_MARKS, POSITIONS, QT_PLATFORM_THEMES, REMOTE_ENCODERS, REMOTE_QUALITIES,
    STARTMENU_STYLES, SWITCH_ANIMATIONS, SWITCHER_SIZES, SWITCHER_STYLES, Settings, Shortcut,
    TASKBAR_BUTTONS, TASKBAR_POSITIONS, TEXT_SIZES, TITLE_DOUBLE_CLICKS, WATER_STYLES, keys_for,
    remote_quality_row,
};

pub(super) enum Bind {
    Text(&'static str, Lens<Settings, String>),
    Num(&'static str, Lens<Settings, i64>),
    Flag(&'static str, Lens<Settings, bool>),
    Choice(&'static str, Lens<Settings, String>),
    Named(&'static str, Lens<Settings, String>, &'static Choices),
    /// A [`Bind::Choice`] whose first rows are the labels of `Choices` standing for their values (`CURSOR_DEFAULT_ROW` for "").
    ChoiceOrDefault(&'static str, Lens<Settings, String>, &'static Choices),
    NamedNum(
        &'static str,
        Lens<Settings, i64>,
        &'static [(i64, &'static str)],
    ),
    /// The "Use virtual desktops" check box: ticked is more than one desktop.
    /// Unticking writes 1; ticking writes the count the up-down beside it shows.
    UseDesktops(&'static str, Lens<Settings, i64>),
    /// The desktop water check box: ticked is any style, unticked "off"; ticking takes the style the combo shows.
    Water(&'static str, Lens<Settings, String>),
    WaterStyle(&'static str, Lens<Settings, String>),
    /// The desktop count's up-down, 2 up. One desktop shows the default count, so ticking the check box starts from it.
    Desktops(&'static str, Lens<Settings, i64>),
    Track(&'static str, Lens<Settings, i64>),
    Kelvin(&'static str, Lens<Settings, i64>),
    Radio(&'static str, Lens<Settings, String>, &'static str),
    /// A check box standing for the field holding its first value when ticked, its second when not.
    Switch(
        &'static str,
        Lens<Settings, String>,
        &'static str,
        &'static str,
    ),
    Time(&'static str, Lens<Settings, String>),
    Delay(&'static str, Lens<Settings, i64>),
    Steps(&'static str, Lens<Settings, i64>, Steps),
    /// The Remote Desktop quality trackbar: its positions stand for [`REMOTE_QUALITIES`], in order.
    Quality(&'static str, Lens<Settings, String>),
    /// A scheme combo: the interface's own row (`own`) stands for the empty string, any other row for its name.
    Scheme(&'static str, Lens<Settings, String>, &'static str),
    /// The glass colour combo: a swatch row for its name, the Custom row for `Settings::glass_custom`.
    GlassColor(&'static str, Lens<Settings, String>),
    /// A trackbar whose position is the value less the first (glass intensity from 0, blur from 1).
    Level(&'static str, Lens<Settings, i64>, i64),
    /// A colour button against "#RRGGBB". Empty shows the desktop colour the look
    /// gives and stays empty while the button still shows it, so reading an untouched button back writes nothing.
    Hex(&'static str, Lens<Settings, String>),
    Icon(&'static str, Lens<Settings, Vec<String>>),
    /// An edit holding the list's entries apart by commas, an empty entry kept as one.
    Names(&'static str, Lens<Settings, Vec<String>>),
    /// A check box ticked while the list holds its name: a desktop widget.
    Member(&'static str, Lens<Settings, Vec<String>>, &'static str),
}

pub(super) fn binds() -> Vec<Bind> {
    vec![
        Bind::Text("task_manager", lens!(task_manager)),
        Bind::Text("startmenu_banner", lens!(startmenu_banner)),
        Bind::Flag("desktop_icons", lens!(desktop_icons)),
        Bind::Named(
            crate::backgroundpage::EXTENSIONS,
            lens!(desktop_extensions),
            crate::model::DESKTOP_EXTENSIONS,
        ),
        Bind::Flag("start_icon", lens!(start_icon)),
        Bind::Flag("adaptive_sync", lens!(adaptive_sync)),
        Bind::Flag("switcher_preview", lens!(switcher_preview)),
        Bind::Flag("switcher_desktop", lens!(switcher_desktop)),
        Bind::Flag("switcher_peek", lens!(switcher_peek)),
        Bind::Flag("switcher_flip", lens!(switcher_flip)),
        Bind::Flag("preview_taskbar", lens!(preview_taskbar)),
        Bind::Flag("osd_caps_lock", lens!(osd_caps_lock)),
        Bind::Flag("osd_num_lock", lens!(osd_num_lock)),
        Bind::Num("previews_fps", lens!(previews_fps)),
        Bind::Named("switcher_style", lens!(switcher_style), SWITCHER_STYLES),
        Bind::Named("switcher_size", lens!(switcher_size), SWITCHER_SIZES),
        Bind::Named(
            crate::defaultspage::DRIVE_INSERT,
            lens!(drive_insert),
            crate::model::DRIVE_INSERTS,
        ),
        Bind::Named(
            crate::defaultspage::SCRIPT_OPEN,
            lens!(script_open),
            crate::model::SCRIPT_OPENS,
        ),
        Bind::Flag(crate::defaultspage::CONFIRM_SCRIPTS, lens!(confirm_scripts)),
        Bind::Named(
            crate::defaultspage::SCRIPT_END,
            lens!(script_terminal),
            crate::model::SCRIPT_ENDS,
        ),
        Bind::Named("hot_corner", lens!(hot_corner), HOT_CORNERS),
        Bind::Flag("screenshot_save", lens!(screenshot_save)),
        Bind::Flag("clipboard_history", lens!(clipboard_history)),
        Bind::Named("interface", lens!(interface), INTERFACES),
        Bind::Scheme(
            crate::appearancepage::CLASSIC_SCHEME,
            lens!(color_scheme),
            crate::appearancepage::classic_default(),
        ),
        Bind::Scheme(
            crate::appearancepage::BLUE_SCHEME,
            lens!(color_scheme),
            reprisekit::blue::Variant::default().name(),
        ),
        Bind::GlassColor(crate::appearancepage::GLASS_COLOR, lens!(glass_color)),
        Bind::Named("glass_mode", lens!(glass_mode), crate::model::GLASS_MODES),
        Bind::Time("glass_dark_from", lens!(glass_dark_from)),
        Bind::Time("glass_dark_to", lens!(glass_dark_to)),
        Bind::Named(
            "glass_material",
            lens!(glass_material),
            crate::model::GLASS_MATERIALS,
        ),
        Bind::Named(
            "glass_frame",
            lens!(glass_frame),
            crate::model::GLASS_FRAMES,
        ),
        Bind::Named(
            "glass_notification_style",
            lens!(glass_notification_style),
            crate::model::GLASS_NOTIFY_STYLES,
        ),
        Bind::Level("glass_intensity", lens!(glass_intensity), 0),
        Bind::Flag("glass_transparency", lens!(glass_transparency)),
        Bind::Level("glass_blur", lens!(glass_blur), 1),
        Bind::Flag("glass_streaks", lens!(glass_streaks)),
        Bind::Flag("glass_fast_drag", lens!(glass_fast_drag)),
        Bind::Flag("glass_blur_frames", lens!(glass_blur_frames)),
        Bind::Level("glass_orb_glow", lens!(glass_orb_glow), 0),
        Bind::Named("wallpaper_position", lens!(wallpaper_position), POSITIONS),
        Bind::NamedNum(
            crate::backgroundpage::INTERVAL,
            lens!(slideshow_interval),
            crate::model::SLIDE_INTERVALS,
        ),
        Bind::Flag("slideshow_shuffle", lens!(slideshow_shuffle)),
        Bind::Hex("background", lens!(background)),
        Bind::Icon("documents", lens!(system_icons)),
        Bind::Icon("computer", lens!(system_icons)),
        Bind::Icon("home", lens!(system_icons)),
        Bind::Member("widget_clock", lens!(desktop_widgets), "clock"),
        Bind::Member("widget_calendar", lens!(desktop_widgets), "calendar"),
        Bind::Member("widget_meter", lens!(desktop_widgets), "meter"),
        Bind::UseDesktops("use_desktops", lens!(desktops)),
        Bind::Desktops("desktops", lens!(desktops)),
        Bind::Named(
            "desktop_animation",
            lens!(desktop_animation),
            SWITCH_ANIMATIONS,
        ),
        Bind::Num("desktop_animation_ms", lens!(desktop_animation_ms)),
        Bind::Named("pager_mark", lens!(pager_mark), PAGER_MARKS),
        Bind::Flag("wheel_on_desktop", lens!(wheel_on_desktop)),
        Bind::Names(super::desktop::DESKTOP_NAMES, lens!(desktop_names)),
        Bind::Flag("remember_windows", lens!(remember_windows)),
        Bind::Flag("window_tabs", lens!(window_tabs)),
        Bind::Flag("window_shake", lens!(window_shake)),
        Bind::Named(
            "title_double_click",
            lens!(title_double_click),
            TITLE_DOUBLE_CLICKS,
        ),
        Bind::Num("dim_inactive", lens!(dim_inactive)),
        Bind::Flag("session_restore", lens!(session_restore)),
        Bind::Flag("session_end_processes", lens!(session_end_processes)),
        Bind::Flag("restart_notice", lens!(restart_notice)),
        Bind::Named(
            "taskbar_position",
            lens!(taskbar_position),
            TASKBAR_POSITIONS,
        ),
        Bind::Named("taskbar_buttons", lens!(taskbar_buttons), TASKBAR_BUTTONS),
        Bind::Named("startmenu_style", lens!(startmenu_style), STARTMENU_STYLES),
        Bind::Flag("taskbar_pinned", lens!(taskbar_pinned)),
        Bind::Flag("taskbar_small_icons", lens!(taskbar_small_icons)),
        Bind::Flag("taskbar_show_desktop", lens!(taskbar_show_desktop)),
        Bind::Flag("taskbar_peek", lens!(taskbar_peek)),
        Bind::Flag("taskbar_tray_overflow", lens!(taskbar_tray_overflow)),
        Bind::Flag("taskbar_badges", lens!(taskbar_badges)),
        Bind::Flag("taskbar_network_meter", lens!(taskbar_network_meter)),
        Bind::Flag(NOTIFICATION_SOUND, lens!(notification_sound)),
        Bind::Choice(SOUND_CONTROLS[1], lens!(sound_theme)),
        Bind::Flag(BELL, lens!(bell)),
        Bind::Flag(ALLOW_BOOST, lens!(allow_boost)),
        Bind::Flag("taskbar_auto_hide", lens!(taskbar_auto_hide)),
        Bind::Flag("taskbar_reveal_on_edge", lens!(taskbar_reveal_on_edge)),
        Bind::Flag("taskbar_reveal_on_keys", lens!(taskbar_reveal_on_keys)),
        Bind::Flag(
            "taskbar_reveal_on_attention",
            lens!(taskbar_reveal_on_attention),
        ),
        Bind::Flag("recent_files", lens!(recent_files)),
        Bind::Flag(PRIVACY_LOCATION, lens!(location)),
        Bind::Flag("capture_border", lens!(capture_border)),
        Bind::Water(WATER, lens!(wallpaper_water)),
        Bind::WaterStyle(WATER_STYLE, lens!(wallpaper_water)),
        Bind::Flag(DEBUG_OVERLAY, lens!(debug_overlay)),
        Bind::Flag(DEBUG_IN_CAPTURES, lens!(debug_overlay_in_captures)),
        Bind::Flag("launcher_files", lens!(launcher_files)),
        Bind::Flag("launcher_windows", lens!(launcher_windows)),
        Bind::Flag(THUMBS, lens!(thumbnail_cache)),
        Bind::Num(THUMBS_MB, lens!(thumbnail_max_mb)),
        Bind::Flag(THUMBNAILERS, lens!(external_thumbnailers)),
        Bind::Text("font_family", lens!(font_family)),
        Bind::Num("font_size_px", lens!(font_size_px)),
        Bind::Flag("font_antialias", lens!(font_antialias)),
        Bind::Text("font_shell_dlg", lens!(font_shell_dlg)),
        Bind::Choice("icon_theme", lens!(icon_theme)),
        Bind::ChoiceOrDefault("cursor_theme", lens!(cursor_theme), CURSOR_FIXED_ROWS),
        Bind::ChoiceOrDefault("gtk_theme", lens!(gtk_theme), GTK_FIXED_ROWS),
        Bind::Named(
            "qt_platform_theme",
            lens!(qt_platform_theme),
            QT_PLATFORM_THEMES,
        ),
        Bind::Named("font_smoothing", lens!(font_smoothing), FONT_SMOOTHINGS),
        Bind::Flag("gtk4_colors", lens!(gtk4_colors)),
        Bind::NamedNum("cursor_size", lens!(cursor_size), CURSOR_SIZES),
        Bind::Num("frame_width", lens!(frame_width)),
        Bind::Num("caption_height", lens!(caption_height)),
        Bind::Num("taskbar_height", lens!(taskbar_height)),
        Bind::Num("menu_shadow", lens!(menu_shadow)),
        Bind::Num("menu_hover_ms", lens!(menu_hover_ms)),
        Bind::Num("menu_fade_ms", lens!(menu_fade_ms)),
        Bind::Named("menu_animation", lens!(menu_animation), MENU_ANIMATIONS),
        Bind::Flag("window_animation", lens!(window_animation)),
        Bind::Num("window_open_ms", lens!(window_open_ms)),
        Bind::Num("window_close_ms", lens!(window_close_ms)),
        Bind::Flag("hover_fade", lens!(hover_fade)),
        Bind::Num("hover_in_ms", lens!(hover_in_ms)),
        Bind::Num("hover_out_ms", lens!(hover_out_ms)),
        Bind::Flag("taskbar_slide", lens!(taskbar_slide)),
        Bind::Flag("session_fade", lens!(session_fade)),
        Bind::Steps("double_click_ms", lens!(double_click_ms), DOUBLE_CLICK),
        Bind::Text("kb_model", lens!(kb_model)),
        Bind::Text("kb_options", lens!(kb_options)),
        Bind::Num("repeat_rate", lens!(repeat_rate)),
        Bind::Num("repeat_delay", lens!(repeat_delay)),
        Bind::Flag("shortcut_overlay", lens!(shortcut_overlay)),
        Bind::Num("shortcut_overlay_ms", lens!(shortcut_overlay_ms)),
        Bind::Flag("layout_per_window", lens!(layout_per_window)),
        Bind::Text("standby", lens!(standby)),
        Bind::Text("poweroff", lens!(poweroff)),
        Bind::Text("reboot", lens!(reboot)),
        Bind::Text("hibernate", lens!(hibernate)),
        Bind::Flag("lock_on_sleep", lens!(lock_on_sleep)),
        Bind::Flag("lock_wallpaper", lens!(lock_wallpaper)),
        Bind::Num("lock_after", lens!(lock_after)),
        Bind::Num("screen_off_after", lens!(screen_off_after)),
        Bind::Num("blank_after", lens!(blank_after)),
        Bind::Num("lock_fade", lens!(lock_fade)),
        Bind::Num(crate::saverpage::AWAY_AFTER, lens!(away_after_s)),
        Bind::Num("low_space_percent", lens!(low_space_percent)),
        Bind::Num(REMOTE_PORT, lens!(remote_port)),
        Bind::Quality(REMOTE_QUALITY, lens!(remote_quality)),
        Bind::NamedNum(REMOTE_FRAME_RATE, lens!(remote_frame_rate), FRAME_RATES),
        Bind::Named(REMOTE_ENCODER, lens!(remote_encoder), REMOTE_ENCODERS),
        Bind::Flag(REMOTE_SOUND, lens!(remote_sound)),
        Bind::Track("mouse_speed", lens!(mouse_speed)),
        Bind::Flag("mouse_accel", lens!(mouse_accel)),
        Bind::Flag("mouse_natural", lens!(mouse_natural)),
        Bind::Track("touchpad_speed", lens!(touchpad_speed)),
        Bind::Flag("touchpad_accel", lens!(touchpad_accel)),
        Bind::Flag("touchpad_natural", lens!(touchpad_natural)),
        Bind::Flag("tap_to_click", lens!(tap_to_click)),
        Bind::Flag("two_finger_scroll", lens!(two_finger_scroll)),
        Bind::Flag(
            "two_finger_tap_right_click",
            lens!(two_finger_tap_right_click),
        ),
        Bind::Flag("disable_while_typing", lens!(disable_while_typing)),
        Bind::Flag("snap_enabled", lens!(snap_enabled)),
        Bind::Flag("snap_picker", lens!(snap_picker)),
        Bind::Flag("snap_bar", lens!(snap_bar)),
        Bind::Text("snap_layouts", lens!(snap_layouts)),
        Bind::Num("snap_distance", lens!(snap_distance)),
        Bind::Flag("nl_enabled", lens!(nl_enabled)),
        Bind::Kelvin("nl_temperature", lens!(nl_temperature)),
        Bind::Radio("nl_sun", lens!(nl_schedule), "sun"),
        Bind::Radio("nl_fixed", lens!(nl_schedule), "fixed"),
        Bind::Time("nl_from", lens!(nl_from)),
        Bind::Time("nl_to", lens!(nl_to)),
        Bind::Flag("sticky_keys", lens!(sticky_keys)),
        Bind::Flag("sticky_lock", lens!(sticky_lock)),
        Bind::Flag("sticky_two_keys_off", lens!(sticky_two_keys_off)),
        Bind::Flag("slow_keys", lens!(slow_keys)),
        Bind::Delay("slow_keys_delay", lens!(slow_keys_delay)),
        Bind::Flag("key_shortcuts", lens!(key_shortcuts)),
        Bind::Named(
            "color_filter",
            lens!(color_filter),
            crate::magnifierpage::FILTERS,
        ),
        Bind::Level("color_filter_intensity", lens!(color_filter_intensity), 0),
        Bind::Switch(
            "magnifier_follow",
            lens!(magnifier_follow),
            "pointer+focus",
            "pointer",
        ),
        Bind::Flag("mouse_keys", lens!(mouse_keys)),
        Bind::Flag("mouse_keys_num_lock", lens!(mouse_keys_num_lock)),
        Bind::Steps("mouse_keys_speed", lens!(mouse_keys_speed), MK_SPEED),
        Bind::Steps(
            "mouse_keys_acceleration",
            lens!(mouse_keys_acceleration),
            MK_ACCEL,
        ),
        Bind::NamedNum("text_size", lens!(text_size), TEXT_SIZES),
    ]
}

fn hhmm(s: &str) -> (u32, u32) {
    s.split_once(':')
        .and_then(|(h, m)| Some((h.trim().parse().ok()?, m.trim().parse().ok()?)))
        .filter(|&(h, m)| h < 24 && m < 60)
        .unwrap_or((0, 0))
}

fn kelvin_pos(k: i64) -> usize {
    ((k.clamp(KELVIN_MIN, KELVIN_MAX) - KELVIN_MIN + KELVIN_STEP / 2) / KELVIN_STEP) as usize
}

impl Bind {
    pub(super) fn id(&self) -> &'static str {
        match self {
            Bind::Text(id, _)
            | Bind::Num(id, _)
            | Bind::Flag(id, _)
            | Bind::Choice(id, _)
            | Bind::Named(id, _, _)
            | Bind::ChoiceOrDefault(id, ..)
            | Bind::NamedNum(id, _, _)
            | Bind::UseDesktops(id, _)
            | Bind::Water(id, _)
            | Bind::WaterStyle(id, _)
            | Bind::Desktops(id, _)
            | Bind::Track(id, _)
            | Bind::Kelvin(id, _)
            | Bind::Radio(id, _, _)
            | Bind::Switch(id, _, _, _)
            | Bind::Time(id, _)
            | Bind::Delay(id, _)
            | Bind::Steps(id, _, _)
            | Bind::Quality(id, _)
            | Bind::Scheme(id, _, _)
            | Bind::GlassColor(id, _)
            | Bind::Level(id, _, _)
            | Bind::Hex(id, _)
            | Bind::Icon(id, _)
            | Bind::Names(id, _)
            | Bind::Member(id, _, _) => id,
        }
    }

    /// Move the field this control writes off its value in `s`.
    fn nudge(&self, s: &mut Settings) {
        match self {
            Bind::Text(_, at)
            | Bind::Choice(_, at)
            | Bind::Named(_, at, _)
            | Bind::ChoiceOrDefault(_, at, _)
            | Bind::Water(_, at)
            | Bind::WaterStyle(_, at)
            | Bind::Radio(_, at, _)
            | Bind::Switch(_, at, _, _)
            | Bind::Time(_, at)
            | Bind::Quality(_, at)
            | Bind::Scheme(_, at, _)
            | Bind::GlassColor(_, at)
            | Bind::Hex(_, at) => at.get_mut(s).push('\u{1}'),
            Bind::Num(_, at)
            | Bind::NamedNum(_, at, _)
            | Bind::UseDesktops(_, at)
            | Bind::Desktops(_, at)
            | Bind::Track(_, at)
            | Bind::Kelvin(_, at)
            | Bind::Delay(_, at)
            | Bind::Steps(_, at, _)
            | Bind::Level(_, at, _) => *at.get_mut(s) += 1,
            Bind::Flag(_, at) => *at.get_mut(s) ^= true,
            Bind::Icon(_, at) => at.get_mut(s).push(String::new()),
            Bind::Names(_, at) => at.get_mut(s).push("\u{1}".into()),
            Bind::Member(_, at, name) => {
                let list = at.get_mut(s);
                match list.iter().position(|n| n == name) {
                    Some(i) => drop(list.remove(i)),
                    None => list.push(name.to_string()),
                }
            }
        }
    }
}

type Nudge = fn(&mut Settings);

/// The controls that write their keys outside [`binds`]: lists, pickers and the dialogs a page
/// opens, each with what it moves.
fn unbound() -> [(&'static str, Nudge); 8] {
    [
        (KB_LAYOUTS, |s| {
            s.kb_layout.push('\u{1}');
            s.kb_variant.push('\u{1}');
        }),
        (crate::backgroundpage::WALLPAPER, |s| {
            s.wallpaper.push('\u{1}');
            s.slideshow.push('\u{1}');
            s.wallpaper_desktops.push("\u{1}".into());
        }),
        (crate::saverpage::SAVER, |s| {
            s.saver.push('\u{1}');
            s.saver_folder.push('\u{1}');
            s.saver_cycle ^= true;
            for n in [
                &mut s.saver_speed,
                &mut s.saver_stars,
                &mut s.saver_shapes,
                &mut s.saver_photo_seconds,
            ] {
                *n += 1;
            }
        }),
        (crate::saverpage::WAIT, |s| s.blank_after += 1),
        (crate::saverpage::AWAY, |s| s.away_device.push('\u{1}')),
        (crate::defaultspage::TERMINAL, |s| s.terminal.push('\u{1}')),
        (crate::defaultspage::TERMINAL_ARGS, |s| {
            s.terminal.push('\u{1}')
        }),
        (crate::notifypage::PROGRAMS, |s| {
            s.notify_no_balloons.push("\u{1}".into());
            s.notify_no_sounds.push("\u{1}".into());
        }),
    ]
}

/// The config keys `form`'s controls write, in the field table's order: each control's field
/// nudged off the defaults and the keys that moved read back from the field table.
pub fn keys_of(form: &Form) -> Vec<Key> {
    let base = Settings::default();
    let mut moved = base.clone();
    for b in binds().iter().filter(|b| form.item(b.id()).is_some()) {
        b.nudge(&mut moved);
    }
    for (id, nudge) in unbound() {
        if form.item(id).is_some() {
            nudge(&mut moved);
        }
    }
    crate::model::keys_apart(&base, &moved)
}

/// Push `s` into every bound control and the Layouts list. Rebuilds the edits,
/// so call it on load or cancel, never on a keystroke: `TextEdit::new` puts the caret at the end.
pub fn write(sheet: &mut PropSheet, s: &Settings, reg: &[Layout], system: &str) {
    fill_layouts(sheet, s, reg, system, None);
    crate::menupage::fill(&mut sheet.pages[Page::StartMenu.index()], &s.startmenu);
    write_forms(sheet, s);
    follow(sheet, s);
}

pub fn write_forms(sheet: &mut PropSheet, s: &Settings) {
    for page in &mut sheet.pages {
        write_form(page, s);
    }
}

/// Push `s` into the bound controls `form` has; rebuilds its edits, so never on a keystroke.
pub fn write_form(form: &mut Form, s: &Settings) {
    for b in binds() {
        let Some(item) = form.item_mut(b.id()) else {
            continue;
        };
        match (&b, &mut item.control) {
            (Bind::Text(_, at), Control::Edit { edit, first, .. }) => {
                *edit = TextEdit::new(at.get(s).clone(), false);
                first.set(0);
            }
            (Bind::Names(_, at), Control::Edit { edit, first, .. }) => {
                *edit = TextEdit::new(at.get(s).join(", "), false);
                first.set(0);
            }
            (Bind::Num(_, at), Control::UpDown(u)) => {
                u.set(*at.get(s) as i32);
            }
            (Bind::Flag(_, at), Control::Check { checked, .. }) => *checked = *at.get(s),
            (Bind::Choice(_, at), Control::Combo(c)) => {
                // A value the list does not offer (a theme that went away) joins it rather than being dropped.
                c.select_text(at.get(s));
            }
            (Bind::ChoiceOrDefault(_, at, fixed), Control::Combo(c)) => {
                let v = at.get(s).as_str();
                let want = fixed
                    .iter()
                    .find(|(f, _)| *f == v)
                    .map_or(v, |(_, row)| *row);
                c.select_text(want);
            }
            (Bind::NamedNum(_, at, choices), Control::Combo(c)) => {
                let want = *at.get(s);
                c.items.truncate(choices.len());
                c.selected = match choices.iter().position(|(v, _)| *v == want) {
                    Some(i) => i,
                    None => {
                        c.items.push(combo::ListItem::new(want.to_string()));
                        choices.len()
                    }
                };
            }
            (Bind::Named(_, at, choices), Control::Combo(c)) => {
                let want = at.get(s);
                // A value the config does not know shows as itself after the known ones.
                c.items.truncate(choices.len());
                c.selected = match choices.iter().position(|(v, _)| v == want) {
                    Some(i) => i,
                    None => {
                        c.items.push(combo::ListItem::new(want.clone()));
                        choices.len()
                    }
                };
            }
            (Bind::UseDesktops(_, at), Control::Check { checked, .. }) => *checked = *at.get(s) > 1,
            (Bind::Water(_, at), Control::Check { checked, .. }) => *checked = at.get(s) != "off",
            (Bind::WaterStyle(_, at), Control::Combo(c)) => {
                if let Some(i) = WATER_STYLES.iter().position(|(v, _)| v == at.get(s)) {
                    c.selected = i;
                }
            }
            (Bind::Desktops(_, at), Control::UpDown(u)) => {
                let n = match *at.get(s) {
                    n if n > 1 => n,
                    _ => DESKTOPS_DEFAULT,
                };
                u.set(n as i32);
            }
            (Bind::Track(_, at), Control::TrackBar(t)) => {
                t.set((*at.get(s) + 10).clamp(0, SPEED_STEPS as i64 - 1) as usize);
            }
            (Bind::Kelvin(_, at), Control::TrackBar(t)) => {
                t.set(kelvin_pos(*at.get(s)));
            }
            (Bind::Radio(_, at, value), Control::Radio { checked, .. }) => {
                *checked = at.get(s) == value;
            }
            (Bind::Switch(_, at, on, _), Control::Check { checked, .. }) => {
                *checked = at.get(s) == on;
            }
            (Bind::Time(_, at), Control::Time(tf)) => {
                let (h, m) = hhmm(at.get(s));
                tf.set(h, m, 0);
            }
            (Bind::Delay(_, at), Control::TrackBar(t)) => {
                t.set((at.get(s).clamp(&DELAY_MIN, &DELAY_MAX) - DELAY_MIN) as usize);
            }
            (Bind::Steps(_, at, steps), Control::TrackBar(t)) => {
                t.set(steps.pos(*at.get(s)));
            }
            (Bind::Quality(_, at), Control::TrackBar(t)) => {
                t.set(remote_quality_row(at.get(s)));
            }
            (Bind::Scheme(_, at, own), Control::Combo(c)) => {
                let want = match at.get(s).as_str() {
                    "" => *own,
                    v => v,
                };
                // A name the list lacks (another interface's) shows as itself.
                c.select_text(want);
            }
            (Bind::GlassColor(_, at), Control::Combo(c)) => {
                c.selected = crate::appearancepage::glass_row(at.get(s), c.items.len());
            }
            (Bind::Level(_, at, first), Control::TrackBar(t)) => {
                t.set((*at.get(s) - first).clamp(0, 100) as usize);
            }
            (Bind::Hex(_, at), Control::Color(b)) => {
                b.color = Color::parse(at.get(s))
                    .unwrap_or_else(|| crate::appearancepage::theme(s).colors.background);
            }
            (Bind::Icon(name, at) | Bind::Member(_, at, name), Control::Check { checked, .. }) => {
                *checked = at.get(s).iter().any(|n| n == name);
            }
            _ => {}
        }
    }
}

/// Grey what depends on another control: the tap button map without
/// tap-to-click; the Layouts buttons by the list (Add up to four, Remove on a
/// row, Move Up below the first); the night light's controls by its check box
/// and times by the fixed schedule; the StickyKeys, SlowKeys and delay
/// options; the colour filter's intensity unless a correction is picked; on
/// Window Switcher, the preview check box while Alt+Tab shows thumbnails and
/// the 3D switcher outside Glass; on Virtual Desktops, the refresh rate while nothing
/// shows a live picture, and the count and animation without "Use virtual
/// desktops". The Appearance page is rebuilt for the pending interface and its
/// sample redrawn; the Background page greys the position without a wallpaper,
/// the file name extensions without the desktop's files, the water style
/// while water is off and its note without a fluid style, and shows the look's desktop
/// colour on its Color button; the Taskbar page
/// greys the auto-hide triggers while auto-hide is off, the Start Menu page the banner outside the
/// Classic style, the Sound page the sound theme while
/// neither notifications nor the bell play a sound, and the Advanced page
/// the debug overlay's captures check while the overlay is off and the thumbnail cache's size
/// while the cache is off; the Remote Desktop page names the quality picked. Runs from
/// `write` and after every read-back.
pub fn follow(sheet: &mut PropSheet, s: &Settings) {
    crate::appearancepage::follow(sheet, s);
    crate::backgroundpage::follow(&mut sheet.pages[Page::Background.index()], s);
    crate::saverpage::follow(&mut sheet.pages[Page::ScreenSaverLock.index()], s);
    if let Some(Control::UpDown(u)) = sheet.pages[Page::Power.index()]
        .item_mut("blank_after")
        .map(|i| &mut i.control)
    {
        u.set(s.blank_after.clamp(0, i32::MAX as i64) as i32);
    }
    let thumbnails = s.switcher_style == "thumbnails";
    let live = s.preview_taskbar || thumbnails || s.switcher_preview;
    let many = s.desktops > 1;
    let sp = &mut sheet.pages[Page::Switcher.index()];
    sp.set_enabled("switcher_preview", !thumbnails);
    sp.set_enabled("switcher_size_label", thumbnails);
    sp.set_enabled("switcher_size", thumbnails);
    sp.set_enabled("switcher_flip", s.interface == "glass");
    let wp = &mut sheet.pages[Page::WindowBehavior.index()];
    for id in DRAG_SNAP_CONTROLS {
        wp.set_enabled(id, s.snap_enabled);
    }
    let dp = &mut sheet.pages[Page::VirtualDesktops.index()];
    for id in FPS_CONTROLS {
        dp.set_enabled(id, live);
    }
    for id in DESKTOP_CONTROLS {
        dp.set_enabled(id, many);
    }
    for id in ["pager_mark_label", "pager_mark"] {
        dp.set_enabled(id, many && s.interface == "glass");
    }
    for id in DURATION_CONTROLS {
        dp.set_enabled(id, many && s.desktop_animation != "none");
    }
    for page in &mut sheet.pages {
        if let Some(Control::Check { enabled, .. }) = page
            .item_mut("two_finger_tap_right_click")
            .map(|i| &mut i.control)
        {
            *enabled = s.tap_to_click;
        }
    }
    let nl = &mut sheet.pages[Page::NightLight.index()];
    for id in NL_CONTROLS {
        nl.set_enabled(id, s.nl_enabled);
    }
    for id in NL_TIMES {
        nl.set_enabled(id, s.nl_enabled && s.nl_schedule == "fixed");
    }
    nl.set_text("nl_temp_value", &format!("{} K", s.nl_temperature));
    sheet.pages[Page::MouseTouchpad.index()]
        .set_text("double_click_value", &format!("{} ms", s.double_click_ms));
    let ax = &mut sheet.pages[Page::TextKeys.index()];
    for id in STICKY_CONTROLS {
        ax.set_enabled(id, s.sticky_keys);
    }
    for id in SLOW_CONTROLS {
        ax.set_enabled(id, s.slow_keys);
    }
    for id in MOUSE_KEYS_CONTROLS {
        ax.set_enabled(id, s.mouse_keys);
    }
    let tenths = s.slow_keys_delay;
    ax.set_text(
        "slow_delay_value",
        &format!("{}.{} seconds", tenths / 10, tenths % 10),
    );
    crate::magnifierpage::follow(&mut sheet.pages[Page::MagnifierFilters.index()], s);
    let fx = &mut sheet.pages[Page::Effects.index()];
    for id in ANIM_CONTROLS {
        fx.set_enabled(id, s.window_animation);
    }
    for id in HOVER_CONTROLS {
        fx.set_enabled(id, s.hover_fade);
    }
    let sp = &mut sheet.pages[Page::Sound.index()];
    for id in SOUND_CONTROLS {
        sp.set_enabled(id, s.notification_sound || s.bell);
    }
    let tb = &mut sheet.pages[Page::Taskbar.index()];
    for id in REVEAL_CONTROLS {
        tb.set_enabled(id, s.taskbar_auto_hide);
    }
    tb.set_enabled(TRAY_CUSTOMIZE, s.taskbar_tray_overflow);
    let sm = &mut sheet.pages[Page::StartMenu.index()];
    for id in ["banner_label", "startmenu_banner"] {
        sm.set_enabled(id, s.startmenu_style == "classic");
    }
    let quality = REMOTE_QUALITIES[remote_quality_row(&s.remote_quality)].1;
    sheet.pages[Page::RemoteDesktop.index()].set_text(QUALITY_VALUE, quality);
    let adv = &mut sheet.pages[Page::Advanced.index()];
    adv.set_enabled(DEBUG_IN_CAPTURES, s.debug_overlay);
    for id in THUMBS_SIZE {
        adv.set_enabled(id, s.thumbnail_cache);
    }
    show_keys(sheet, &s.shortcuts);
    let count = s.layouts().len();
    let picked = layout_selection(sheet);
    let page = &mut sheet.pages[Page::Keyboard.index()];
    let lit = [
        (KB_ADD, count < MAX_LAYOUTS),
        (KB_REMOVE, picked.is_some_and(|i| i < count)),
        (KB_UP, picked.is_some_and(|i| 0 < i && i < count)),
    ];
    for (id, on) in lit {
        page.set_enabled(id, on);
    }
    let focused = page.focus().and_then(|f| page.items.get(f)).map(|i| i.id);
    if lit.iter().any(|&(id, on)| !on && focused == Some(id)) {
        page.focus_id(KB_LAYOUTS);
    }
}

/// Fill every note that names a key from `binds`: the layout switch keys and the zoom keys.
pub fn show_keys(sheet: &mut PropSheet, binds: &[Shortcut]) {
    let switch = keys_for(binds, "next-layout").map_or_else(
        || "No key switches them.".to_string(),
        |k| format!("{k} switches them."),
    );
    sheet.pages[Page::Keyboard.index()].set_text(KB_SWITCH_NOTE, &switch);
    crate::magnifierpage::show_zoom_keys(&mut sheet.pages[Page::MagnifierFilters.index()], binds);
}

/// The layout xkbcommon compiles for an empty `[keyboard] layout`: the first of `env`
/// (`XKB_DEFAULT_LAYOUT`), else "us".
pub fn system_layout(env: Option<&str>) -> String {
    let first = env.and_then(|e| e.split(',').next()).map(str::trim);
    first.filter(|l| !l.is_empty()).unwrap_or("us").to_string()
}

/// The Layouts list from `s`; with no layouts, one greyed row naming `system`, the
/// [`system_layout`] in use.
pub fn fill_layouts(
    sheet: &mut PropSheet,
    s: &Settings,
    reg: &[Layout],
    system: &str,
    select: Option<usize>,
) {
    let layouts = s.layouts();
    let list = if layouts.is_empty() {
        let system = reprisefs::xkb::describe(reg, system, "");
        let mut list = ListBox::new(
            vec![format!("{} (system default)", system.description)],
            None,
        );
        list.greyed = vec![true];
        list
    } else {
        let keep = select.or(layout_selection(sheet));
        let rows: Vec<String> = (layouts.iter())
            .map(|(l, v)| reprisefs::xkb::describe(reg, l, v).description)
            .collect();
        let n = rows.len();
        ListBox::new(rows, keep.filter(|&i| i < n))
    };
    if let Some(Control::ListBox { list: l, .. }) = sheet.pages[Page::Keyboard.index()]
        .item_mut(KB_LAYOUTS)
        .map(|i| &mut i.control)
    {
        *l = list;
    }
    follow(sheet, s);
}

pub fn layout_selection(sheet: &PropSheet) -> Option<usize> {
    match sheet.pages[Page::Keyboard.index()]
        .item(KB_LAYOUTS)
        .map(|i| &i.control)
    {
        Some(Control::ListBox { list, .. }) => list.selected,
        _ => None,
    }
}

/// Pull the control `id`'s value back into `s`, if it is a bound one.
///
/// Per-control on purpose: an up-down clamps whatever the file held into its
/// range when built, so reading every control back would fold those clamps
/// into the pending state and the next Apply would rewrite a key the user
/// never touched. The app reads back only what it saw a `form::Event::Changed` for.
pub fn read_one(sheet: &PropSheet, id: &str, s: &mut Settings) {
    for b in binds() {
        if b.id() != id {
            continue;
        }
        for page in &sheet.pages {
            let Some(item) = page.item(b.id()) else {
                continue;
            };
            match (&b, &item.control) {
                (Bind::Text(_, at), Control::Edit { edit, .. }) => {
                    *at.get_mut(s) = edit.text.clone()
                }
                (Bind::Names(_, at), Control::Edit { edit, .. }) => {
                    *at.get_mut(s) = names(&edit.text)
                }
                (Bind::Num(_, at), Control::UpDown(u)) => *at.get_mut(s) = u.value() as i64,
                (Bind::Flag(_, at), Control::Check { checked, .. }) => *at.get_mut(s) = *checked,
                (Bind::Choice(_, at), Control::Combo(c)) => {
                    if let Some(v) = c.items.get(c.selected) {
                        *at.get_mut(s) = v.text.clone();
                    }
                }
                (Bind::ChoiceOrDefault(_, at, fixed), Control::Combo(c)) => {
                    if let Some(v) = c.items.get(c.selected) {
                        let t = v.text.as_str();
                        *at.get_mut(s) = fixed
                            .iter()
                            .find(|(_, row)| *row == t)
                            .map_or(t, |(value, _)| *value)
                            .to_string();
                    }
                }
                (Bind::NamedNum(_, at, choices), Control::Combo(c)) => {
                    let value = match choices.get(c.selected) {
                        Some((v, _)) => Some(*v),
                        None => c.items.get(c.selected).and_then(|i| i.text.parse().ok()),
                    };
                    if let Some(v) = value {
                        *at.get_mut(s) = v;
                    }
                }
                (Bind::Named(_, at, choices), Control::Combo(c)) => {
                    let value = match choices.get(c.selected) {
                        Some((v, _)) => Some(v.to_string()),
                        None => c.items.get(c.selected).map(|i| i.text.clone()),
                    };
                    if let Some(v) = value {
                        *at.get_mut(s) = v;
                    }
                }
                (Bind::UseDesktops(_, at), Control::Check { checked, .. }) => {
                    *at.get_mut(s) = match page.item("desktops").map(|i| &i.control) {
                        Some(Control::UpDown(u)) if *checked => u.value() as i64,
                        _ if *checked => DESKTOPS_DEFAULT,
                        _ => 1,
                    }
                }
                (Bind::Water(_, at), Control::Check { checked, .. }) => {
                    let style = page
                        .item(WATER_STYLE)
                        .and_then(|i| match &i.control {
                            Control::Combo(c) => WATER_STYLES.get(c.selected),
                            _ => None,
                        })
                        .map_or("calm", |(v, _)| *v);
                    if !*checked {
                        *at.get_mut(s) = "off".into();
                    } else if at.get(s) == "off" {
                        *at.get_mut(s) = style.into();
                    }
                }
                (Bind::WaterStyle(_, at), Control::Combo(c)) => {
                    if at.get(s) != "off"
                        && let Some((v, _)) = WATER_STYLES.get(c.selected)
                    {
                        *at.get_mut(s) = v.to_string();
                    }
                }
                (Bind::Desktops(_, at), Control::UpDown(u)) => *at.get_mut(s) = u.value() as i64,
                (Bind::Track(_, at), Control::TrackBar(t)) => *at.get_mut(s) = t.pos as i64 - 10,
                (Bind::Kelvin(_, at), Control::TrackBar(t)) => {
                    *at.get_mut(s) = KELVIN_MIN + t.pos as i64 * KELVIN_STEP
                }
                (Bind::Radio(_, at, value), Control::Radio { checked: true, .. }) => {
                    *at.get_mut(s) = value.to_string()
                }
                (Bind::Switch(_, at, on, off), Control::Check { checked, .. }) => {
                    *at.get_mut(s) = if *checked { on } else { off }.to_string()
                }
                (Bind::Time(_, at), Control::Time(tf)) => {
                    *at.get_mut(s) = format!("{:02}:{:02}", tf.h, tf.m)
                }
                (Bind::Delay(_, at), Control::TrackBar(t)) => {
                    *at.get_mut(s) = DELAY_MIN + t.pos as i64
                }
                (Bind::Steps(_, at, steps), Control::TrackBar(t)) => {
                    *at.get_mut(s) = steps.value(t.pos)
                }
                (Bind::Quality(_, at), Control::TrackBar(t)) => {
                    if let Some((v, _)) = REMOTE_QUALITIES.get(t.pos) {
                        *at.get_mut(s) = v.to_string();
                    }
                }
                (Bind::Scheme(_, at, own), Control::Combo(c)) => {
                    if let Some(v) = c.items.get(c.selected) {
                        *at.get_mut(s) = if v.text == *own {
                            String::new()
                        } else {
                            v.text.clone()
                        };
                    }
                }
                (Bind::GlassColor(_, at), Control::Combo(c)) => {
                    let v = match reprisekit::glass::swatch_names().nth(c.selected) {
                        Some(n) => n.to_string(),
                        None => s.glass_custom.clone(),
                    };
                    if !v.is_empty() {
                        *at.get_mut(s) = v;
                    }
                }
                (Bind::Level(_, at, first), Control::TrackBar(t)) => {
                    *at.get_mut(s) = t.pos as i64 + first
                }
                (Bind::Hex(_, at), Control::Color(b)) => {
                    let own = at.get(s).is_empty()
                        && b.color == crate::appearancepage::theme(s).colors.background;
                    if !own {
                        let c = b.color;
                        *at.get_mut(s) = format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b);
                    }
                }
                (Bind::Icon(name, at), Control::Check { checked, .. }) => {
                    let icons = crate::model::system_icons_with(at.get(s), name, *checked);
                    *at.get_mut(s) = icons;
                }
                (Bind::Member(_, at, name), Control::Check { checked, .. }) => {
                    let list = at.get_mut(s);
                    let has = list.iter().any(|n| n == name);
                    if *checked && !has {
                        list.push(name.to_string());
                    } else if !*checked {
                        list.retain(|n| n != name);
                    }
                }
                _ => {}
            }
        }
    }
}

/// `text`'s entries apart by commas, trimmed, an empty one kept in its place and the empty ones
/// at the end dropped.
pub(crate) fn names(text: &str) -> Vec<String> {
    let mut list: Vec<String> = text.split(',').map(|s| s.trim().to_string()).collect();
    while list.last().is_some_and(String::is_empty) {
        list.pop();
    }
    list
}
