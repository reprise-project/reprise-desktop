//! Restart Into: the boot loader entries logind lists (`BootLoaderEntries`) and firmware setup,
//! offered by the Turn off computer panel's Restart Into... and the Shut down options' Restart into
//! when logind can set them. A pick restarts through [`crate::sessionend::begin_into`], which sets
//! the one-shot target just before the reboot and clears it when the reboot does not happen.
//!
//! logind lists entry ids only. A type #1 entry's title is read from its file on the boot
//! partition when the user may read it; anything else shows its id tidied up ([`pretty`]).

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::compositor::State;
use crate::dialog::Dialog;
use crate::menuview::{MenuFrom, MenuKind};
use crate::startmenu::MenuItem;

const LOGIND: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER: &str = "org.freedesktop.login1.Manager";
const CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// Where the boot partition's `loader/entries` may be mounted, per the Boot Loader Specification.
const ENTRY_ROOTS: [&str; 3] = ["/efi", "/boot", "/boot/efi"];

/// systemd-boot's own entries that a restart has no use for: firmware setup has its own row.
const HIDDEN: [&str; 3] = [
    "auto-reboot-to-firmware-setup",
    "auto-poweroff",
    "auto-reboot",
];

/// Somewhere to restart into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootTarget {
    /// The boot loader entry id; None for firmware setup.
    pub entry: Option<String>,
    pub title: String,
    /// polkit asks for a password before logind sets it.
    pub password: bool,
}

impl BootTarget {
    fn firmware(password: bool) -> BootTarget {
        BootTarget {
            entry: None,
            title: "Firmware Setup".into(),
            password,
        }
    }
}

/// A `Can*` answer that offers the action, and whether it asks for a password.
fn offered(answer: &str) -> Option<bool> {
    match answer {
        "yes" => Some(false),
        "challenge" => Some(true),
        _ => None,
    }
}

/// What logind on `conn` offers to restart into: its entries, then firmware setup. A call that
/// fails offers nothing of its kind.
pub async fn offer(conn: &zbus::Connection) -> Vec<BootTarget> {
    let proxy = match manager(conn).await {
        Ok(p) => p,
        Err(e) => {
            log::info!("restart into: logind: {e}");
            return Vec::new();
        }
    };
    let can = async |method: &str| match proxy.call::<_, _, String>(method, &()).await {
        Ok(answer) => offered(&answer),
        Err(e) => {
            log::info!("restart into: {method}: {e}");
            None
        }
    };
    let mut out = Vec::new();
    if let Some(password) = can("CanRebootToBootLoaderEntry").await {
        match proxy.get_property::<Vec<String>>("BootLoaderEntries").await {
            Ok(ids) => out = entries(&ids, password, &ENTRY_ROOTS.map(PathBuf::from)),
            Err(e) => log::info!("restart into: BootLoaderEntries: {e}"),
        }
    }
    if let Some(password) = can("CanRebootToFirmwareSetup").await {
        out.push(BootTarget::firmware(password));
    }
    out
}

pub(crate) async fn manager(conn: &zbus::Connection) -> zbus::Result<zbus::Proxy<'static>> {
    zbus::proxy::Builder::new(conn)
        .destination(LOGIND)
        .path(LOGIND_PATH)
        .interface(LOGIND_MANAGER)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
}

/// Set logind's one-shot `target` for the next reboot, or with `on` false clear it again.
pub async fn aim(
    conn: &zbus::Connection,
    target: &BootTarget,
    on: bool,
    interactive: bool,
) -> zbus::Result<()> {
    let proxy = manager(conn).await?;
    let auth = match interactive {
        true => zbus::proxy::MethodFlags::ALLOW_INTERACTIVE_AUTH,
        false => zbus::proxy::MethodFlags::empty(),
    };
    match &target.entry {
        Some(id) => {
            let id = if on { id.as_str() } else { "" };
            (proxy.call_with_flags::<_, _, ()>("SetRebootToBootLoaderEntry", auth, &(id,))).await?;
        }
        None => {
            (proxy.call_with_flags::<_, _, ()>("SetRebootToFirmwareSetup", auth, &(on,))).await?;
        }
    }
    Ok(())
}

/// `ids` as targets, titled from their files under `roots`; titles two entries share carry the id.
fn entries(ids: &[String], password: bool, roots: &[PathBuf]) -> Vec<BootTarget> {
    let shown: Vec<&String> = ids
        .iter()
        .filter(|id| !HIDDEN.contains(&id.as_str()))
        .collect();
    let titles: Vec<String> = (shown.iter())
        .map(|id| title_under(roots, id).unwrap_or_else(|| pretty(id)))
        .collect();
    (shown.iter().zip(&titles))
        .map(|(id, title)| {
            let shared = titles.iter().filter(|t| *t == title).count() > 1;
            BootTarget {
                entry: Some(id.to_string()),
                title: match shared {
                    true => format!("{title} ({id})"),
                    false => title.clone(),
                },
                password,
            }
        })
        .collect()
}

/// The `title` of type #1 entry `id` from the first of `roots` whose `loader/entries` holds it.
fn title_under(roots: &[PathBuf], id: &str) -> Option<String> {
    if !id.ends_with(".conf") || id.contains('/') {
        return None;
    }
    roots
        .iter()
        .map(|r| r.join("loader/entries").join(id))
        .find_map(|p| read_title(&p))
}

fn read_title(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|l| {
        let (key, value) = l.trim().split_once(char::is_whitespace)?;
        let value = value.trim();
        (key == "title" && !value.is_empty()).then(|| value.to_string())
    })
}

/// Entry id `id` as a title: without `auto-` and its file extension, words apart, the first
/// capitalised.
fn pretty(id: &str) -> String {
    let stem = id.strip_prefix("auto-").unwrap_or(id);
    let stem = (stem.strip_suffix(".conf"))
        .or_else(|| stem.strip_suffix(".efi"))
        .unwrap_or(stem);
    let words = stem.replace(['-', '_'], " ");
    let mut chars = words.trim().chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => id.to_string(),
    }
}

/// Ask logind what there is to restart into, off the loop; an open Turn off computer panel shows
/// or drops Restart Into... with the answer.
pub fn refresh(state: &mut State) {
    state.spawn_job(|| {
        let targets = zbus::block_on(async {
            let conn = zbus::connection::Builder::system()
                .method_timeout(CALL_TIMEOUT)
                .build()
                .await;
            match conn {
                Ok(c) => offer(&c).await,
                Err(e) => {
                    log::info!("restart into: system bus: {e}");
                    Vec::new()
                }
            }
        });
        Box::new(move |state: &mut State| answered(state, targets))
    });
}

fn answered(state: &mut State, targets: Vec<BootTarget>) {
    if state.boot_targets == targets {
        return;
    }
    log::info!("restart into: {} targets", targets.len());
    state.boot_targets = targets;
    let on = !state.boot_targets.is_empty();
    for i in 0..state.dialogs.views.len() {
        if state.dialogs.views[i].dialog.offer_restart_into(on) {
            state.refit_dialog(i);
            state.redraw_dialog(i);
        }
    }
}

impl State {
    /// The Turn off computer panel for the `[power]` commands, Restart Into... in it when there is
    /// somewhere to restart into. Asks logind again for the next one.
    pub fn turn_off_dialog(&mut self) -> Dialog {
        let disconnect = crate::console::disconnect_offered(self);
        let mut d = Dialog::turn_off(&self.config.power, disconnect);
        d.offer_restart_into(!self.boot_targets.is_empty());
        refresh(self);
        d
    }
}

/// The rows that list `targets`: the entries, then firmware setup under a separator.
pub fn menu_items(targets: &[BootTarget]) -> Vec<MenuItem> {
    let mut items = Vec::new();
    for t in targets {
        if t.entry.is_none() && !items.is_empty() {
            items.push(MenuItem::Separator);
        }
        items.push(MenuItem::entry(t.title.clone()));
    }
    items
}

/// The menu's row `idx`, counting the separator [`menu_items`] puts in.
pub fn target_at(targets: &[BootTarget], idx: usize) -> Option<BootTarget> {
    let mut row = 0;
    for (i, t) in targets.iter().enumerate() {
        if t.entry.is_none() && i > 0 {
            row += 1;
        }
        if row == idx {
            return Some(t.clone());
        }
        row += 1;
    }
    None
}

/// Restart Into... on the top dialog was pressed: list the targets under it, the first row lit
/// when the keyboard pressed it.
pub fn open_menu(state: &mut State, keys: bool) {
    let Some(v) = state.dialogs.views.last() else {
        return;
    };
    let Some(i) = (v.dialog.buttons.iter())
        .position(|b| b.action == crate::dialog::DialogAction::RestartInto)
    else {
        return;
    };
    let Some(r) = v.layout.buttons.get(i) else {
        return;
    };
    if state.boot_targets.is_empty() {
        return;
    }
    let at = (v.x + r.x, v.y + r.bottom());
    let targets = state.boot_targets.clone();
    state.close_menu();
    let items = menu_items(&targets);
    state.push_context_menu(
        at,
        items,
        MenuKind::RestartInto(targets),
        MenuFrom::Elsewhere,
    );
    if keys && !state.menus.is_empty() {
        state.enter_menu_keys();
    }
}

/// A target was picked: the panel it came from closes and the computer restarts into it.
pub fn pick(state: &mut State, target: BootTarget) {
    let top_offers = (state.dialogs.views.last()).is_some_and(|v| {
        (v.dialog.buttons.iter()).any(|b| b.action == crate::dialog::DialogAction::RestartInto)
    });
    if top_offers {
        state.apply_dialog(crate::dialog::Outcome::Close);
    }
    log::info!("restart into: {:?}", target.entry);
    let cmd = state.config.power.reboot.clone();
    crate::sessionend::begin_into(state, cmd, target);
}

#[cfg(test)]
pub(crate) mod tests;
