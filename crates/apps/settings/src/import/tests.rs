use super::*;
use std::collections::HashMap;

/// A `dconf dump /` of a GNOME account whose owner changed a few things, with the noise programs
/// leave beside them.
const DUMP: &str = r#"[org/gnome/desktop/background]
color-shading-type='solid'
picture-options='zoom'
picture-uri='file:///home/sam/Pictures/Fjord%20Morning.jpg'
picture-uri-dark='file:///usr/share/backgrounds/gnome/blobs-d.svg'
primary-color='#3071AE'

[org/gnome/desktop/input-sources]
mru-sources=[('xkb', 'de+nodeadkeys'), ('xkb', 'us')]
sources=[('xkb', 'us'), ('xkb', 'de+nodeadkeys'), ('ibus', 'anthy')]
xkb-options=['caps:escape', 'compose:ralt']

[org/gnome/desktop/interface]
clock-format='12h'
color-scheme='prefer-dark'
cursor-size=32
gtk-theme='Adwaita-dark'
text-scaling-factor=1.25

[org/gnome/desktop/peripherals/keyboard]
delay=uint32 300
repeat-interval=uint32 30

[org/gnome/desktop/peripherals/mouse]
accel-profile='flat'
speed=-0.34

[org/gnome/desktop/peripherals/touchpad]
click-method='fingers'
natural-scroll=true
send-events='disabled-on-external-mouse'
tap-to-click=false
two-finger-scrolling-enabled=true

[org/gnome/mutter]
dynamic-workspaces=false

[org/gnome/desktop/wm/preferences]
num-workspaces=6

[org/gnome/settings-daemon/plugins/color]
night-light-enabled=true
night-light-schedule-automatic=false
night-light-schedule-from=20.5
night-light-schedule-to=6.0
night-light-temperature=uint32 3700

[org/gnome/shell]
favorite-apps=['firefox.desktop', 'org.gnome.Nautilus.desktop', 'org.gnome.Settings.desktop']
welcome-dialog-last-shown-version='46.2'

[org/gnome/nautilus/window-state]
initial-size=(890, 550)
"#;

struct Fake {
    apps: HashMap<&'static str, &'static str>,
    files: Vec<&'static str>,
    quick: Vec<String>,
}

fn entry(id: &str, name: &str) -> AppEntry {
    reprisefs::apps::parse_desktop(
        &format!("[Desktop Entry]\nType=Application\nName={name}\nExec={name}\n"),
        id,
    )
    .unwrap()
}

impl System for Fake {
    fn app(&self, id: &str) -> Option<(AppEntry, PathBuf)> {
        let file = PathBuf::from("/usr/share/applications").join(id);
        self.apps.get(id).map(|n| (entry(id, n), file))
    }
    fn default_app(&self, mime: &str) -> Option<(AppEntry, PathBuf)> {
        (mime == "x-scheme-handler/http").then(|| self.app("firefox.desktop"))?
    }
    fn is_file(&self, path: &Path) -> bool {
        self.files.iter().any(|f| Path::new(f) == path)
    }
    fn quick_launch(&self) -> Vec<String> {
        self.quick.clone()
    }
}

fn fake() -> Fake {
    Fake {
        apps: HashMap::from([
            ("firefox.desktop", "Firefox"),
            ("org.gnome.Nautilus.desktop", "Files"),
        ]),
        files: vec!["/home/sam/Pictures/Fjord Morning.jpg"],
        quick: Vec::new(),
    }
}

fn row<'a>(p: &'a Plan, what: &str) -> &'a Line {
    (p.lines.iter())
        .find(|l| l.what == what)
        .unwrap_or_else(|| panic!("no {what} row in {:#?}", p.lines))
}

fn cells(l: &Line) -> (&str, &str, bool) {
    (&l.now, &l.new, l.greyed)
}

#[test]
fn gvariant_text_reads_strings_numbers_lists_and_tuples() {
    use gnome::Value::*;
    assert_eq!(
        gnome::parse_value("[('xkb', 'us'), ('ibus', \"it's\")]"),
        Some(List(vec![
            List(vec![Str("xkb".into()), Str("us".into())]),
            List(vec![Str("ibus".into()), Str("it's".into())]),
        ]))
    );
    assert_eq!(gnome::parse_value("uint32 30"), Some(Num(30.0)));
    assert_eq!(gnome::parse_value("-0.34"), Some(Num(-0.34)));
    assert_eq!(gnome::parse_value("@as []"), Some(List(Vec::new())));
    assert_eq!(gnome::parse_value("'caf\\u00e9'"), Some(Str("café".into())));
    assert_eq!(gnome::parse_value("true"), Some(Bool(true)));
    assert_eq!(gnome::parse_value("['a',"), None);
}

#[test]
fn a_gnome_dump_becomes_reprise_changes() {
    let p = plan("GNOME", &gnome::settings(DUMP), "", &fake());
    assert_eq!(
        cells(row(&p, "Wallpaper")),
        (
            "None",
            "/home/sam/Pictures/Fjord Morning.jpg (Stretch)",
            false
        ),
        "the dark style's own picture is an SVG, so the light one, zoom read as Stretch"
    );
    assert_eq!(cells(row(&p, "Dark style")), ("Off", "On", false));
    assert_eq!(
        cells(row(&p, "Keyboard layouts")),
        ("System default", "us, de (nodeadkeys)", false)
    );
    assert_eq!(
        cells(row(&p, "Keyboard options")),
        ("None", "caps:escape,compose:ralt", false)
    );
    assert_eq!(
        cells(row(&p, "Key repeat rate")),
        ("25 a second", "33 a second", false)
    );
    assert_eq!(
        cells(row(&p, "Key repeat delay")),
        ("600 ms", "300 ms", false)
    );
    assert_eq!(cells(row(&p, "Mouse speed")), ("0.0", "-0.3", false));
    assert_eq!(cells(row(&p, "Mouse acceleration")), ("On", "Off", false));
    assert_eq!(
        cells(row(&p, "Touchpad natural scrolling")),
        ("Off", "On", false)
    );
    assert_eq!(cells(row(&p, "Tap to click")), ("On", "Off", false));
    assert_eq!(
        cells(row(&p, "Touchpad click method")),
        ("The pad's own", "Finger count", false)
    );
    assert_eq!(
        cells(row(&p, "Touchpad off with a mouse")),
        ("Off", "On", false)
    );
    assert_eq!(cells(row(&p, "Text size")), ("100%", "125%", false));
    assert_eq!(cells(row(&p, "Night light")), ("Off", "On", false));
    assert_eq!(
        cells(row(&p, "Night light warmth")),
        ("4000 K", "3700 K", false)
    );
    assert_eq!(
        cells(row(&p, "Night light hours")),
        ("Sunset to sunrise", "20:30 to 06:00", false)
    );
    assert_eq!(
        cells(row(&p, "Pointer size")),
        ("The session's", "32 px", false)
    );
    assert_eq!(cells(row(&p, "Virtual desktops")), ("4", "6", false));
    let quick: Vec<&str> = (p.lines.iter())
        .filter(|l| l.what == "Quick Launch" && !l.greyed)
        .map(|l| l.new.as_str())
        .collect();
    assert_eq!(quick, ["Firefox", "Files"]);
    assert!(
        p.lines.iter().all(|l| l.what != "Touchpad"),
        "the pad is already on"
    );
}

#[test]
fn what_reprise_cant_take_comes_last_greyed_with_why() {
    let p = plan("GNOME", &gnome::settings(DUMP), "", &fake());
    let first_grey = p.lines.iter().position(|l| l.greyed).unwrap();
    assert!(p.lines[first_grey..].iter().all(|l| l.greyed));
    assert_eq!(p.writes.len(), first_grey, "a write per change");
    assert_eq!(
        cells(row(&p, "Clock")),
        ("the clock follows the formats locale", "12-hour", true)
    );
    assert_eq!(
        cells(row(&p, "Input method")),
        ("set up in the input method", "ibus: anthy", true)
    );
    assert!(cells(row(&p, "GTK theme")).2);
    let missing = p
        .lines
        .iter()
        .find(|l| l.what == "Quick Launch" && l.greyed);
    assert_eq!(
        missing.map(cells),
        Some(("not installed", "org.gnome.Settings", true))
    );
}

#[test]
fn a_setting_already_in_place_or_a_program_already_in_quick_launch_is_left_out() {
    let config = "[keyboard]\nrepeat_rate = 33\n";
    let mut sys = fake();
    sys.quick = vec!["firefox.desktop".into()];
    let p = plan("GNOME", &gnome::settings(DUMP), config, &sys);
    assert!(p.lines.iter().all(|l| l.what != "Key repeat rate"));
    let quick: Vec<&str> = (p.lines.iter())
        .filter(|l| l.what == "Quick Launch" && !l.greyed)
        .map(|l| l.new.as_str())
        .collect();
    assert_eq!(quick, ["Files"]);
}

#[test]
fn a_picture_reprise_cant_show_or_find_is_greyed() {
    let found = [Setting::Wallpaper {
        path: "/usr/share/backgrounds/gnome/blobs-d.svg".into(),
        position: "stretch",
    }];
    let p = plan("GNOME", &found, "", &fake());
    assert_eq!(
        cells(&p.lines[0]),
        (
            "Reprise shows JPEG, PNG and BMP pictures",
            "/usr/share/backgrounds/gnome/blobs-d.svg",
            true
        )
    );
    let gone = [Setting::Wallpaper {
        path: "/home/sam/gone.png".into(),
        position: "stretch",
    }];
    let p = plan("GNOME", &gone, "", &fake());
    assert_eq!(cells(&p.lines[0]).0, "the file is missing");
}

#[test]
fn dark_is_greyed_under_blue() {
    let config = "[appearance]\ninterface = \"blue\"\n";
    let p = plan("GNOME", &[Setting::Dark(true)], config, &fake());
    assert_eq!(
        cells(&p.lines[0]),
        ("Blue comes in light colours only", "On", true)
    );
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("reprise-import-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn checked_rows_land_in_the_config_file_and_quick_launch_and_the_rest_stays() {
    let dir = scratch("write");
    let config = dir.join("config.toml");
    std::fs::write(&config, "# mine\n[keyboard]\nrepeat_delay = 450\n").unwrap();
    let quick = dir.join("quicklaunch");
    std::fs::create_dir_all(&quick).unwrap();
    let launcher = "[Desktop Entry]\nType=Application\nName=Ed\nExec=ed\n";
    std::fs::write(quick.join("ed.desktop"), launcher).unwrap();
    let apps = dir.join("applications");
    std::fs::create_dir_all(&apps).unwrap();
    let firefox = "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox %u\n";
    std::fs::write(apps.join("firefox.desktop"), firefox).unwrap();

    let picked = [
        Write::Config(Setting::Natural(Device::Touchpad, true)),
        Write::Config(Setting::Dark(true)),
        Write::Config(Setting::NightSchedule(Some((
            "20:30".into(),
            "06:00".into(),
        )))),
        Write::QuickLaunch(
            entry("firefox.desktop", "Firefox"),
            apps.join("firefox.desktop"),
        ),
    ];
    let refs: Vec<&Write> = picked.iter().collect();
    write(&config, Some(&quick), &refs).unwrap();

    let text = std::fs::read_to_string(&config).unwrap();
    assert!(
        text.starts_with("# mine\n[keyboard]\nrepeat_delay = 450\n"),
        "{text}"
    );
    let s = model::load(&text);
    assert!(s.touchpad_natural);
    assert_eq!(s.color_scheme, welcome::DARK_SCHEME);
    assert_eq!(
        (s.nl_schedule.as_str(), s.nl_from.as_str()),
        ("fixed", "20:30")
    );
    assert_eq!(s.repeat_delay, 450, "an unpicked key keeps its value");
    assert_eq!(
        ql::read_order(&quick).unwrap(),
        ["ed.desktop", "firefox.desktop"],
        "imported programs come after the folder's own"
    );
    assert_eq!(
        std::fs::read_to_string(quick.join("firefox.desktop")).unwrap(),
        firefox
    );
}

#[test]
fn kconfig_reads_nested_groups_escapes_and_expanded_values() {
    let k = plasma::KConfig::parse(
        "[General]\nName=Breeze\nName[de]=Brise\n\n[A][B][C]\nKey[$e]=$HOME/x\\sy\n",
        "/home/sam",
    );
    assert_eq!(k.get(&["General"], "Name"), Some("Breeze"));
    assert_eq!(k.get(&["A", "B", "C"], "Key"), Some("/home/sam/x y"));
    assert_eq!(k.get(&["A", "B"], "Key"), None);
}

/// A Plasma 6 account's files, as Plasma writes them after a few changes.
fn plasma_config(dir: &Path) {
    let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
    write(
        "kdeglobals",
        "[General]\nColorScheme=BreezeDark\nfont=Inter,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n\n\
         [Icons]\nTheme=breeze-dark\n\n[KDE]\nLookAndFeelPackage=org.kde.breezedark.desktop\n\
         DoubleClickInterval=550\n",
    );
    write(
        "kxkbrc",
        "[Layout]\nDisplayNames=,\nLayoutList=us,hu\nModel=pc105\nOptions=grp:alt_shift_toggle\n\
         ResetOldOptions=true\nSwitchMode=Window\nUse=true\nVariantList=,qwerty\n",
    );
    write(
        "kcminputrc",
        "[Keyboard]\nNumLock=0\nRepeatDelay=250\nRepeatRate=40\n\n\
         [Libinput][1133][16511][Logitech G502]\nPointerAcceleration=0.400\n\
         PointerAccelerationProfile=1\n\n\
         [Libinput][1739][52619][SYNA8004:00 06CB:CD8B Touchpad]\nClickMethod=2\n\
         DisableWhileTyping=false\nNaturalScroll=true\nTapToClick=true\n\n\
         [Mouse]\ncursorSize=36\ncursorTheme=Breeze_Light\n",
    );
    write(
        "kwinrc",
        "[Desktops]\nId_1=a\nNumber=3\nRows=1\n\n[NightColor]\nActive=true\n\
         EveningBeginFixed=2130\nMode=Times\nMorningBeginFixed=0615\nNightTemperature=3300\n",
    );
    write("kcmfonts", "[General]\nforceFontDPIWayland=144\n");
    write(
        "plasma-org.kde.plasma.desktop-appletsrc",
        "[Containments][1]\nactivityId=6d1d\nformfactor=0\nlastScreen=0\n\
         plugin=org.kde.plasma.folder\nwallpaperplugin=org.kde.image\n\n\
         [Containments][1][Wallpaper][org.kde.image][General]\nFillMode=6\n\
         Image=file:///home/sam/Pictures/Lake.png\n\n\
         [Containments][2]\nformfactor=2\nlastScreen=0\nplugin=org.kde.panel\n\n\
         [Containments][2][Applets][5]\nplugin=org.kde.plasma.icontasks\n\n\
         [Containments][2][Applets][5][Configuration][General]\n\
         launchers=applications:org.kde.dolphin.desktop,preferred://browser,applications:org.kde.konsole.desktop\n\n\
         [Containments][2][Applets][7]\nplugin=org.kde.plasma.digitalclock\n\n\
         [Containments][2][Applets][7][Configuration][Appearance]\nuse24hFormat=2\n",
    );
}

#[test]
fn a_plasma_account_becomes_reprise_changes() {
    let dir = scratch("plasma");
    plasma_config(&dir);
    let mut sys = fake();
    sys.files = vec!["/home/sam/Pictures/Lake.png"];
    sys.apps = HashMap::from([
        ("org.kde.dolphin.desktop", "Dolphin"),
        ("firefox.desktop", "Firefox"),
    ]);
    let p = plan("Plasma", &plasma::settings(&dir, "/home/sam"), "", &sys);
    assert_eq!(
        cells(row(&p, "Wallpaper")),
        ("None", "/home/sam/Pictures/Lake.png (Center)", false)
    );
    assert_eq!(cells(row(&p, "Dark style")), ("Off", "On", false));
    assert_eq!(
        cells(row(&p, "Keyboard layouts")),
        ("System default", "us, hu (qwerty)", false)
    );
    assert_eq!(
        cells(row(&p, "Keyboard model")),
        ("System default", "pc105", false)
    );
    assert_eq!(cells(row(&p, "Layout per window")), ("Off", "On", false));
    assert_eq!(
        cells(row(&p, "Key repeat rate")),
        ("25 a second", "40 a second", false)
    );
    assert_eq!(cells(row(&p, "Mouse speed")), ("0.0", "0.4", false));
    assert_eq!(cells(row(&p, "Mouse acceleration")), ("On", "Off", false));
    assert_eq!(
        cells(row(&p, "Touchpad natural scrolling")),
        ("Off", "On", false)
    );
    assert_eq!(
        cells(row(&p, "Touchpad off while typing")),
        ("On", "Off", false)
    );
    assert_eq!(
        cells(row(&p, "Touchpad click method")),
        ("The pad's own", "Finger count", false)
    );
    assert_eq!(cells(row(&p, "Text size")), ("100%", "150%", false));
    assert_eq!(
        cells(row(&p, "Double-click time")),
        ("500 ms", "550 ms", false)
    );
    assert_eq!(
        cells(row(&p, "Pointer theme")),
        ("The session's", "Breeze_Light", false)
    );
    assert_eq!(
        cells(row(&p, "Night light warmth")),
        ("4000 K", "3300 K", false)
    );
    assert_eq!(
        cells(row(&p, "Night light hours")),
        ("Sunset to sunrise", "21:30 to 06:15", false)
    );
    assert_eq!(cells(row(&p, "Virtual desktops")), ("4", "3", false));
    assert_eq!(cells(row(&p, "Clock")).1, "24-hour");
    let quick: Vec<(&str, bool)> = (p.lines.iter())
        .filter(|l| l.what == "Quick Launch")
        .map(|l| (l.new.as_str(), l.greyed))
        .collect();
    assert_eq!(
        quick,
        [
            ("Dolphin", false),
            ("Firefox", false),
            ("org.kde.konsole", true)
        ],
        "preferred://browser is the default web browser"
    );
}

#[test]
fn a_wallpaper_package_shows_its_largest_picture() {
    let dir = scratch("package");
    let images = dir.join("Next/contents/images");
    std::fs::create_dir_all(&images).unwrap();
    for f in ["1280x1024.png", "5120x2880.png", "1920x1080.png"] {
        std::fs::write(images.join(f), "").unwrap();
    }
    let applets = format!(
        "[Containments][3][Wallpaper][org.kde.image][General]\nImage={}/Next/\n",
        dir.display()
    );
    std::fs::write(dir.join("plasma-org.kde.plasma.desktop-appletsrc"), applets).unwrap();
    let found = plasma::settings(&dir, "/home/sam");
    assert_eq!(
        found.first(),
        Some(&Setting::Wallpaper {
            path: images.join("5120x2880.png").display().to_string(),
            position: "stretch",
        })
    );
}

#[test]
fn an_account_without_either_desktop_finds_nothing() {
    let dir = scratch("empty");
    assert!(plasma::settings(&dir, "/home/sam").is_empty());
    assert!(gnome::settings("[org/gnome/nautilus/preferences]\nshow-hidden=true\n").is_empty());
}

#[test]
fn a_built_in_wallpaper_reads_without_a_position() {
    let found = [Setting::Wallpaper {
        path: "/home/sam/Pictures/Fjord Morning.jpg".into(),
        position: "center",
    }];
    let config = "[wallpaper]\nimage = \"reprise:flow/glass\"\n";
    let p = plan("GNOME", &found, config, &fake());
    assert_eq!(
        cells(&p.lines[0]),
        (
            "reprise:flow/glass",
            "/home/sam/Pictures/Fjord Morning.jpg (Center)",
            false
        )
    );
}

#[test]
fn numbers_past_reprises_ranges_are_written_at_the_nearest_end() {
    let found = [
        Setting::CursorSize(300),
        Setting::DoubleClick(20_000),
        Setting::RepeatDelay(-1),
        Setting::RepeatRate(-5),
    ];
    let p = plan("Plasma", &found, "", &fake());
    assert_eq!(cells(row(&p, "Pointer size")).1, "256 px");
    let dir = scratch("clamp");
    let config = dir.join("config.toml");
    let refs: Vec<&Write> = p.writes.iter().collect();
    write(&config, None, &refs).unwrap();
    let text = std::fs::read_to_string(&config).unwrap();
    let c = repriseconf::config::Config::parse(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
    assert_eq!((c.cursor.size, c.keyboard.repeat_delay), (256, 0));
    assert_eq!(c.keyboard.repeat_rate, 0);
    assert!(text.contains("double_click_ms = 10000"), "{text}");
}

#[test]
fn a_program_that_cannot_be_read_writes_nothing() {
    let dir = scratch("missing");
    let config = dir.join("config.toml");
    std::fs::write(&config, "# mine\n").unwrap();
    let quick = dir.join("quicklaunch");
    let picked = [
        Write::Config(Setting::Dark(true)),
        Write::QuickLaunch(entry("gone.desktop", "Gone"), dir.join("gone.desktop")),
    ];
    let refs: Vec<&Write> = picked.iter().collect();
    assert!(write(&config, Some(&quick), &refs).is_err());
    assert_eq!(std::fs::read_to_string(&config).unwrap(), "# mine\n");
    assert!(!quick.exists());
}

#[test]
fn a_plasma_file_that_is_a_fifo_reads_as_none() {
    let dir = scratch("fifo");
    let made = std::process::Command::new("mkfifo")
        .arg(dir.join("kdeglobals"))
        .status()
        .unwrap();
    assert!(made.success());
    assert!(plasma::settings(&dir, "/home/sam").is_empty());
}
