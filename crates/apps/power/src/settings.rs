use repriseconf::{Key, Lens, get, key as k, lens, remove, set};
use toml_edit::{DocumentMut, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Seconds; 0 disables.
    pub screen_off_after: i64,
    pub suspend_after: i64,
    /// `[idle.battery]` overrides; None inherits the key above.
    pub battery_screen_off_after: Option<i64>,
    pub battery_suspend_after: Option<i64>,
    /// Percent; 0 is an alarm that is off.
    pub battery_low: i64,
    pub battery_critical: i64,
    pub critical_action: String,
    pub lid_close: String,
    pub power_button: String,
    pub sleep_button: String,
    pub always_show_icon: bool,
}

// These defaults mirror the compositor's `IdleConfig` and `PowerConfig`
// (src/config.rs); the two must change together.
impl Default for Settings {
    fn default() -> Self {
        Settings {
            screen_off_after: 900,
            suspend_after: 0,
            battery_screen_off_after: None,
            battery_suspend_after: None,
            battery_low: 10,
            battery_critical: 3,
            critical_action: "hibernate".into(),
            lid_close: "standby".into(),
            power_button: "ask".into(),
            sleep_button: "standby".into(),
            always_show_icon: false,
        }
    }
}

impl Settings {
    pub fn battery_screen_off(&self) -> i64 {
        self.battery_screen_off_after
            .unwrap_or(self.screen_off_after)
    }

    pub fn battery_suspend(&self) -> i64 {
        self.battery_suspend_after.unwrap_or(self.suspend_after)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commands {
    pub standby: String,
    pub hibernate: String,
    pub poweroff: String,
}

impl Default for Commands {
    fn default() -> Self {
        Commands {
            standby: "systemctl suspend".into(),
            hibernate: "systemctl hibernate".into(),
            poweroff: "systemctl poweroff".into(),
        }
    }
}

impl Commands {
    pub fn for_action(&self, action: &str) -> Option<String> {
        match action {
            "standby" => Some(self.standby.clone()),
            "hibernate" => Some(self.hibernate.clone()),
            "turnoff" => Some(self.poweroff.clone()),
            _ => None,
        }
    }
}

/// A sleep logind reports it cannot do ("no", "na") becomes a shutdown, so
/// the machine does not run flat. `can` is None when logind did not answer.
pub fn critical_action(action: &str, can: impl FnOnce(&str) -> Option<String>) -> &str {
    let method = match action {
        "standby" => "CanSuspend",
        "hibernate" => "CanHibernate",
        _ => return action,
    };
    match can(method).as_deref() {
        Some("no" | "na") => "turnoff",
        _ => action,
    }
}

pub fn commands(text: &str) -> Commands {
    let mut c = Commands::default();
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return c;
    };
    let read = |name, into: &mut String| {
        if let Some(v) = get(&doc, &k("power", name)).and_then(|i| i.as_str()) {
            *into = v.to_string();
        }
    };
    read("standby", &mut c.standby);
    read("hibernate", &mut c.hibernate);
    read("poweroff", &mut c.poweroff);
    c
}

enum Field {
    Int(Key, Lens<Settings, i64>),
    OptInt(Key, Lens<Settings, Option<i64>>),
    Text(Key, Lens<Settings, String>),
    Flag(Key, Lens<Settings, bool>),
}

/// The single place a key name appears, so `load` and `save` agree.
fn fields() -> Vec<Field> {
    vec![
        Field::Int(k("idle", "screen_off_after"), lens!(screen_off_after)),
        Field::Int(k("idle", "suspend_after"), lens!(suspend_after)),
        Field::OptInt(
            k("idle.battery", "screen_off_after"),
            lens!(battery_screen_off_after),
        ),
        Field::OptInt(
            k("idle.battery", "suspend_after"),
            lens!(battery_suspend_after),
        ),
        Field::Int(k("power", "battery_low"), lens!(battery_low)),
        Field::Int(k("power", "battery_critical"), lens!(battery_critical)),
        Field::Text(k("power", "critical_action"), lens!(critical_action)),
        Field::Text(k("power", "lid_close"), lens!(lid_close)),
        Field::Text(k("power", "power_button"), lens!(power_button)),
        Field::Text(k("power", "sleep_button"), lens!(sleep_button)),
        Field::Flag(k("power", "always_show_icon"), lens!(always_show_icon)),
    ]
}

impl Field {
    fn key(&self) -> &Key {
        match self {
            Field::Int(k, _) | Field::OptInt(k, _) | Field::Text(k, _) | Field::Flag(k, _) => k,
        }
    }

    fn value(&self, s: &Settings) -> Option<Value> {
        match self {
            Field::Int(_, at) => Some(Value::from(*at.get(s))),
            Field::OptInt(_, at) => at.get(s).map(Value::from),
            Field::Text(_, at) => Some(Value::from(at.get(s).as_str())),
            Field::Flag(_, at) => Some(Value::from(*at.get(s))),
        }
    }

    fn same(&self, a: &Settings, b: &Settings) -> bool {
        match self {
            Field::Int(_, at) => at.get(a) == at.get(b),
            Field::OptInt(_, at) => at.get(a) == at.get(b),
            Field::Text(_, at) => at.get(a) == at.get(b),
            Field::Flag(_, at) => at.get(a) == at.get(b),
        }
    }
}

pub fn load(text: &str) -> Settings {
    let mut s = Settings::default();
    let Ok(doc) = text.parse::<DocumentMut>() else {
        log::warn!("config.toml does not parse; showing defaults");
        return s;
    };
    for f in fields() {
        let Some(item) = get(&doc, f.key()) else {
            continue;
        };
        match &f {
            Field::Int(_, at) => {
                if let Some(v) = item.as_integer() {
                    *at.get_mut(&mut s) = v;
                }
            }
            Field::OptInt(_, at) => {
                if let Some(v) = item.as_integer() {
                    *at.get_mut(&mut s) = Some(v);
                }
            }
            Field::Text(_, at) => {
                if let Some(v) = item.as_str() {
                    *at.get_mut(&mut s) = v.to_string();
                }
            }
            Field::Flag(_, at) => {
                if let Some(v) = item.as_bool() {
                    *at.get_mut(&mut s) = v;
                }
            }
        }
    }
    s
}

/// A key `want` leaves at `base` stays as the file has it: another writer may
/// have changed it. `Err` when `text` does not parse, rather than replace a
/// file the user is editing.
pub fn save(text: &str, want: &Settings, base: &Settings) -> Result<String, String> {
    let mut doc = text.parse::<DocumentMut>().map_err(|e| e.to_string())?;
    let have = load(text);
    let def = Settings::default();
    for f in fields() {
        if f.same(want, base) || f.same(want, &have) {
            continue;
        }
        match f.value(want) {
            Some(v) if !f.same(want, &def) => {
                set(&mut doc, f.key(), v).map_err(|e| e.to_string())?
            }
            _ => remove(&mut doc, f.key()),
        }
    }
    Ok(doc.to_string())
}

#[cfg(test)]
fn has(text: &str, key: &Key) -> bool {
    text.parse::<DocumentMut>()
        .ok()
        .and_then(|d| get(&d, key).map(toml_edit::Item::is_value))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_gives_defaults_for_an_empty_file() {
        assert_eq!(load(""), Settings::default());
        assert_eq!(load("not = = toml"), Settings::default());
    }

    #[test]
    fn load_reads_every_key() {
        let s = load(
            "[idle]\nscreen_off_after = 60\nsuspend_after = 120\n\
             [idle.battery]\nscreen_off_after = 30\n\
             [power]\nbattery_low = 20\ncritical_action = \"standby\"\nalways_show_icon = true\n",
        );
        assert_eq!(s.screen_off_after, 60);
        assert_eq!(s.suspend_after, 120);
        assert_eq!(s.battery_screen_off_after, Some(30));
        assert_eq!(s.battery_suspend_after, None);
        assert_eq!(s.battery_screen_off(), 30);
        assert_eq!(s.battery_suspend(), 120);
        assert_eq!(s.battery_low, 20);
        assert_eq!(s.critical_action, "standby");
        assert!(s.always_show_icon);
    }

    #[test]
    fn save_writes_only_what_changed_and_drops_defaults() {
        let text = "# mine\nterminal = \"foot\"\n\n[power]\nbattery_low = 20\n";
        let mut want = load(text);
        want.battery_low = 10;
        want.lid_close = "lock".into();
        want.battery_suspend_after = Some(600);
        let out = save(text, &want, &load(text)).unwrap();
        assert!(out.starts_with("# mine\nterminal = \"foot\"\n"), "{out}");
        assert!(!has(&out, &k("power", "battery_low")), "{out}");
        assert!(out.contains("lid_close = \"lock\""), "{out}");
        assert!(out.contains("[idle.battery]\nsuspend_after = 600"), "{out}");
        assert_eq!(load(&out), want);
    }

    #[test]
    fn save_removes_a_battery_override_set_back_to_inherit() {
        let text = "[idle.battery]\nscreen_off_after = 30\n";
        let mut want = load(text);
        want.battery_screen_off_after = None;
        let out = save(text, &want, &load(text)).unwrap();
        assert!(!has(&out, &k("idle.battery", "screen_off_after")), "{out}");
    }

    #[test]
    fn commands_come_from_power_with_defaults() {
        let c = commands("[power]\nhibernate = \"zzz\"\n");
        assert_eq!(c.hibernate, "zzz");
        assert_eq!(c.standby, "systemctl suspend");
        assert_eq!(c.for_action("hibernate").as_deref(), Some("zzz"));
        assert_eq!(c.for_action("nothing"), None);
    }

    #[test]
    fn a_critical_sleep_logind_cannot_do_becomes_a_shutdown() {
        let answer = |a: &'static str| move |_: &str| Some(a.to_string());
        assert_eq!(critical_action("hibernate", answer("na")), "turnoff");
        assert_eq!(critical_action("standby", answer("no")), "turnoff");
        assert_eq!(critical_action("hibernate", answer("yes")), "hibernate");
        assert_eq!(critical_action("standby", answer("challenge")), "standby");
        assert_eq!(critical_action("hibernate", |_| None), "hibernate");
        let asked = std::cell::RefCell::new(Vec::new());
        let ask = |m: &str| {
            asked.borrow_mut().push(m.to_string());
            Some("na".to_string())
        };
        assert_eq!(critical_action("standby", ask), "turnoff");
        assert_eq!(critical_action("hibernate", ask), "turnoff");
        assert_eq!(critical_action("turnoff", ask), "turnoff");
        assert_eq!(critical_action("nothing", ask), "nothing");
        assert_eq!(*asked.borrow(), ["CanSuspend", "CanHibernate"]);
    }

    #[test]
    fn save_refuses_a_broken_file() {
        let def = Settings::default();
        assert!(save("[power\n", &def, &def).is_err());
    }

    #[test]
    fn save_leaves_an_unchanged_file_alone() {
        let text = "[power]\nbattery_low = 20   # keep\n";
        assert_eq!(save(text, &load(text), &load(text)).unwrap(), text);
    }

    #[test]
    fn save_leaves_a_key_another_writer_changed_meanwhile() {
        let opened = "[idle]\nscreen_off_after = 900\n[power]\nbattery_low = 10\n";
        let base = load(opened);
        let mut want = base.clone();
        want.lid_close = "hibernate".into();
        let edited = "[idle]\nscreen_off_after = 600\n[power]\nbattery_low = 20\n";
        let out = save(edited, &want, &base).unwrap();
        let got = load(&out);
        assert_eq!(got.screen_off_after, 600, "{out}");
        assert_eq!(got.battery_low, 20, "{out}");
        assert_eq!(got.lid_close, "hibernate", "{out}");
    }
}
