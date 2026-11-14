//! `--install FILE` (a package, `.flatpakref` or `.flatpak`) and `--uninstall DESKTOP-FILE`: the
//! install sheet asks first, then PackageKit or Flatpak does the work under polkit. A launcher an
//! app installed is deleted here.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};

use repriseapp::keys::form_input;
use repriseapp::{App, Caption, Ctx, Event, Reply, WinId};
use reprisefs::software::{Installation, Remover};
use reprisekit::access::Tree;
use reprisekit::controls::form::Form;
use reprisekit::controls::installsheet::{self, Choice, Sheet, Stage, Verb};
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::theme::TextUnits;
use zbus::Connection;

use crate::model::{Install, split_package_id};
use crate::{flatpak, guard, packagekit, pkgfile};

/// The desktop entry that opens package files, whose task button shows the progress.
const DESKTOP_ID: &str = "reprise-install";
const NO_PACKAGEKIT: &str = "Reprise installs and uninstalls the system's packages through \
                             PackageKit, which is not installed.";
const NO_FLATPAK: &str = "Installing a Flatpak needs Flatpak, which is not installed.";
const UNKNOWN: &str = "Unknown";
/// A `.flatpakref`'s size until Flatpak says how much it downloads.
const SIZE_LATER: &str = "Known once the download starts";
const LAUNCHER_NOTE: &str = "Only the launcher is removed. The program that installed it keeps \
                             its own files.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A package file, `.flatpakref` or `.flatpak` bundle.
    Install(PathBuf),
    /// The program with this desktop file.
    Uninstall(PathBuf),
}

/// Whether `args` (after the program's name) start with `--install` or `--uninstall`.
pub fn asks_for_sheet(args: &[std::ffi::OsString]) -> bool {
    args.first()
        .is_some_and(|a| a == "--install" || a == "--uninstall")
}

/// The target `args` name, if they ask for the sheet with one file. A relative path is made
/// absolute, so no tool it reaches can read it as an option.
pub fn target(args: &[std::ffi::OsString]) -> Option<Target> {
    let path = |p| std::path::absolute(PathBuf::from(p)).ok();
    match args {
        [flag, p] if flag == "--install" => Some(Target::Install(path(p)?)),
        [flag, p] if flag == "--uninstall" => Some(Target::Uninstall(path(p)?)),
        _ => None,
    }
}

/// What reading the target found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub icon: Option<String>,
    pub facts: Vec<(String, String)>,
    pub note: String,
    pub ready: bool,
    /// It may never go ahead: the note says why.
    pub refused: bool,
}

/// What Install or Uninstall runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Plan {
    #[default]
    Nothing,
    /// `flatpak` with these arguments, on the installation `at`.
    Flatpak { at: Installation, args: Vec<String> },
    /// Remove this package id and what needs it.
    Remove(String),
    /// Install the package in `file`, named `name`.
    InstallFile { file: String, name: String },
    /// Install this Arch package with `pkexec pacman -U`.
    Pacman(PathBuf),
    /// Delete the portal-installed launcher with this desktop file.
    Launcher(PathBuf),
}

#[derive(Debug)]
pub enum Msg {
    Found(Found),
    Progress {
        fraction: Option<f64>,
        doing: String,
    },
    Cancellable(bool),
    /// The most the job downloads, in bytes.
    Download(u64),
    Ended(Result<(), Failure>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// The password prompt was dismissed: the sheet asks again.
    Dismissed,
    Text(String),
}

/// What stops a running job: a PackageKit transaction's path or a `flatpak` process's pidfd.
#[derive(Debug, Default)]
struct Stopper {
    tid: Option<String>,
    pidfd: Option<flatpak::Pidfd>,
    cancelled: bool,
    /// The private copy's folder, removed by `emergency` when the process ends unwound. It
    /// outlives every run, so `Stopper::new_run` leaves it.
    copy: Option<PathBuf>,
}

impl Stopper {
    /// Forget the last run's job, keeping the private copy's folder.
    fn new_run(&mut self) {
        self.tid = None;
        self.pidfd = None;
        self.cancelled = false;
    }
}

type Poster = repriseapp::Poster<Msg>;

fn fact(label: &str, value: impl Into<String>) -> (String, String) {
    let v = value.into();
    let v = if v.is_empty() { UNKNOWN.into() } else { v };
    (label.into(), v)
}

fn size_text(bytes: Option<u64>) -> String {
    bytes.map(reprisefs::props::short_size).unwrap_or_default()
}

fn os_name() -> String {
    let text = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    reprisefs::sysinfo::os_release(&text).name
}

/// "Uninstalling it also removes 2 packages that need it: a, b." for the other `ids` a change
/// touches; empty when there are none.
fn also(ids: &[String], verb: Verb) -> String {
    const LISTED: usize = 6;
    if ids.is_empty() {
        return String::new();
    }
    let mut names: Vec<&str> = ids.iter().map(|id| split_package_id(id).0).collect();
    names.sort_unstable();
    names.dedup();
    let n = names.len();
    let mut list = names[..n.min(LISTED)].join(", ");
    if n > LISTED {
        list.push_str(&format!(" and {} more", n - LISTED));
    }
    let noun = if n == 1 { "package" } else { "packages" };
    match verb {
        Verb::Uninstall => {
            let needs = if n == 1 { "needs" } else { "need" };
            format!("Uninstalling it also removes {n} {noun} that {needs} it: {list}.")
        }
        Verb::Install => format!("Installing it also installs or updates {n} {noun}: {list}."),
    }
}

fn pk_text(e: packagekit::Error) -> String {
    match e {
        packagekit::Error::Unavailable => NO_PACKAGEKIT.into(),
        packagekit::Error::Dismissed => "The password prompt was dismissed.".into(),
        packagekit::Error::Failed(e) | packagekit::Error::SystemPackage(e) => e,
    }
}

fn install_of(i: Installation) -> Install {
    match i {
        Installation::System => Install::System,
        Installation::User => Install::User,
    }
}

/// The program with desktop file `file`: what removes it and what it is.
fn read_program(file: &Path, conn: Option<&Connection>) -> (Found, Plan) {
    let text = reprisefs::apps::read_desktop_file(file).unwrap_or_default();
    let id = file.file_name().map(|n| n.to_string_lossy().into_owned());
    let entry = id.and_then(|id| reprisefs::apps::parse_desktop_any(&text, &id));
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned());
    let mut found = Found {
        name: (entry.as_ref().map(|e| e.name.clone()))
            .or(stem)
            .unwrap_or_default(),
        icon: entry.and_then(|e| e.icon),
        ..Found::default()
    };
    let plan = match reprisefs::software::remover_of(file) {
        Remover::Flatpak(at) => {
            let app = reprisefs::software::flatpak_app(&text, file);
            let install = install_of(at);
            match flatpak::info(install, &app) {
                Ok(info) => {
                    let source = format!(
                        "{} (Flatpak, {} installation)",
                        info.origin,
                        install.label()
                    );
                    let size = pkgfile::size_bytes(&info.size);
                    found.facts = vec![
                        fact("App ID:", app.clone()),
                        fact("Version:", info.version),
                        fact("Size:", size.map_or(info.size, |b| size_text(Some(b)))),
                        fact("Source:", source),
                    ];
                    found.note = "Its settings and files in your home folder are kept.".into();
                    found.ready = true;
                    let r = if info.reference.is_empty() {
                        app
                    } else {
                        info.reference
                    };
                    let args = vec!["uninstall".into(), at.flag().into(), "-y".into(), r];
                    Plan::Flatpak { at, args }
                }
                Err(e) => {
                    found.note = e;
                    Plan::Nothing
                }
            }
        }
        Remover::Package => match conn {
            Some(c) => zbus::block_on(read_package(c, file, &mut found)),
            None => {
                found.note = NO_PACKAGEKIT.into();
                Plan::Nothing
            }
        },
        Remover::Launcher => {
            found.facts = launcher_facts(&text, |id| {
                let id = format!("{id}.desktop");
                let text = reprisefs::apps::read_desktop_file(&reprisefs::apps::find_file(&id)?);
                reprisefs::apps::parse_desktop_any(&text.ok()?, &id).map(|e| e.name)
            });
            found.note = LAUNCHER_NOTE.into();
            found.ready = true;
            Plan::Launcher(file.to_path_buf())
        }
        Remover::None => {
            found.note = "This program did not come from a package or a Flatpak, so Reprise \
                          cannot uninstall it."
                .into();
            Plan::Nothing
        }
    };
    (found, plan)
}

/// The sheet's rows for a launcher with desktop entry `text`: the Flatpak app that installed it,
/// named by `name_of` where it can.
fn launcher_facts(text: &str, name_of: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    let kv = reprisefs::apps::desktop_entry_kv(text);
    match kv
        .get("X-Flatpak")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        Some(id) => vec![fact(
            "Installed by:",
            name_of(id).unwrap_or_else(|| id.into()),
        )],
        None => Vec::new(),
    }
}

async fn read_package(c: &Connection, file: &Path, found: &mut Found) -> Plan {
    let id = match packagekit::owner(c, &file.to_string_lossy()).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            found.note = "No installed package holds this program.".into();
            return Plan::Nothing;
        }
        Err(e) => {
            found.note = pk_text(e);
            return Plan::Nothing;
        }
    };
    let (name, version) = split_package_id(&id);
    let size = packagekit::size(c, &id).await.ok().flatten();
    found.facts = vec![
        fact("Package:", name),
        fact("Version:", version),
        fact("Size:", size_text(size)),
        fact("Source:", format!("{} packages", os_name())),
    ];
    match packagekit::remove(c, &id, true, &mut |_, _| {}, &mut |_| {}).await {
        Ok(others) => {
            let set: Vec<&str> = std::iter::once(name)
                .chain(others.iter().map(|o| split_package_id(o).0))
                .collect();
            let blocks = guard::blocks(&set, &guard::essential(&set));
            if !blocks.is_empty() {
                found.note = guard::refusal(&blocks);
                found.refused = true;
                return Plan::Nothing;
            }
            found.note = also(&others, Verb::Uninstall);
            found.ready = true;
            Plan::Remove(id)
        }
        Err(packagekit::Error::SystemPackage(details)) => {
            found.note = guard::held(&details);
            found.refused = true;
            Plan::Nothing
        }
        Err(e) => {
            found.note = pk_text(e);
            Plan::Nothing
        }
    }
}

/// The host part of a repository address.
fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest)
}

/// A `.flatpakref`'s facts. `flatpak install --from` adds its repository, and the one `RuntimeRepo`
/// names, as system remotes that stay, so the note says where the runtime comes from.
fn ref_found(r: reprisefs::software::FlatpakRef, found: &mut Found) {
    let source = host(&r.url).to_string();
    let runtime = host(&r.runtime_repo);
    found.name = r.title;
    found.icon = Some(r.id.clone());
    found.facts = vec![
        fact("App ID:", r.id),
        fact("Branch:", r.branch),
        fact("Size:", SIZE_LATER),
        fact("Source:", format!("{source} (Flatpak)")),
    ];
    found.note = if runtime.is_empty() || runtime == source {
        "It is installed for every user of this computer, with the runtime it needs from the \
         same source, which stays added for later installs and updates."
            .into()
    } else {
        format!(
            "It is installed for every user of this computer, with the runtime it needs from \
             {runtime}. Both sources stay added for later installs and updates."
        )
    };
    found.ready = true;
}

/// Who built a package. `checked`: pacman will check a `.sig` beside the file as it installs it;
/// that proves a key pacman trusts signed it, not that the named packager did.
fn packager(name: String, checked: bool) -> String {
    match (name.is_empty(), checked) {
        (true, _) => String::new(),
        (false, true) => format!("{name} (signature checked when it installs)"),
        (false, false) => format!("(unverified) {name}"),
    }
}

/// Package file `path`, the sheet's private copy: what it is and what installs it.
fn read_file(path: &Path, signed: bool, conn: Option<&Connection>) -> (Found, Plan) {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut found = Found {
        name: file_name.clone(),
        icon: Some("package-x-generic".into()),
        ..Found::default()
    };
    let arg = path.to_string_lossy().into_owned();
    let flatpak_args = |how: &str| {
        let args = ["install", "--system", "-y", how, &arg];
        Plan::Flatpak {
            at: Installation::System,
            args: args.map(String::from).to_vec(),
        }
    };
    let plan = match pkgfile::kind_of(path) {
        None => {
            found.note = "Reprise cannot install this kind of file.".into();
            Plan::Nothing
        }
        Some(pkgfile::Kind::FlatpakRef | pkgfile::Kind::FlatpakBundle) if !flatpak::present() => {
            found.note = NO_FLATPAK.into();
            Plan::Nothing
        }
        Some(pkgfile::Kind::FlatpakRef) => {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            match reprisefs::software::flatpak_ref(&text) {
                Some(r) => {
                    ref_found(r, &mut found);
                    flatpak_args("--from")
                }
                None => {
                    found.note = "This file does not name a Flatpak app.".into();
                    Plan::Nothing
                }
            }
        }
        Some(pkgfile::Kind::FlatpakBundle) => {
            let size = std::fs::metadata(path).ok().map(|m| m.len());
            found.name = file_name.trim_end_matches(".flatpak").to_string();
            found.facts = vec![
                fact("File:", file_name),
                fact("Version:", ""),
                fact("Size:", size_text(size)),
                fact("Source:", "Flatpak bundle file"),
            ];
            found.note = "It is installed for every user of this computer.".into();
            found.ready = true;
            flatpak_args("--bundle")
        }
        Some(kind @ (pkgfile::Kind::Pacman | pkgfile::Kind::Deb)) => {
            match (pkgfile::read(path, kind), conn) {
                (Err(e), _) => {
                    found.note = e;
                    Plan::Nothing
                }
                (Ok(facts), conn) => {
                    found.name = facts.name.clone();
                    found.facts = vec![
                        fact("Package:", facts.name.clone()),
                        fact("Version:", facts.version),
                        fact("Size:", size_text(facts.size)),
                        fact(
                            "Packager:",
                            packager(
                                facts.packager,
                                signed
                                    && kind == pkgfile::Kind::Pacman
                                    && pkgfile::local_sigs_checked(),
                            ),
                        ),
                    ];
                    match conn {
                        _ if kind == pkgfile::Kind::Pacman => {
                            pacman_plan(path, &facts.name, &mut found)
                        }
                        Some(c) if zbus::block_on(packagekit_takes_debs(c)) => {
                            zbus::block_on(simulate_file(c, &arg, &facts.name, &mut found))
                        }
                        Some(_) if reprisefs::software::packagekit_present() => {
                            found.note = "This system's packages are of another kind.".into();
                            Plan::Nothing
                        }
                        _ => {
                            found.note = NO_PACKAGEKIT.into();
                            Plan::Nothing
                        }
                    }
                }
            }
        }
    };
    (found, plan)
}

/// Whether PackageKit's backend installs Debian packages. An Arch package goes through pacman
/// itself: PackageKit's pacman backend takes no zstd packages and wants every file signed.
async fn packagekit_takes_debs(c: &Connection) -> bool {
    let wanted = ["application/vnd.debian.binary-package", "application/x-deb"];
    packagekit::file_types(c)
        .await
        .is_ok_and(|t| t.iter().any(|t| wanted.contains(&t.as_str())))
}

/// An Arch package: pacman lists what it pulls in, and installs it.
fn pacman_plan(path: &Path, name: &str, found: &mut Found) -> Plan {
    match pkgfile::pacman_targets(path) {
        Ok(targets) => {
            let others: Vec<String> = targets.into_iter().filter(|t| t != name).collect();
            found.note = also(&others, Verb::Install);
            found.ready = true;
            Plan::Pacman(path.into())
        }
        Err(e) => {
            found.note = e;
            Plan::Nothing
        }
    }
}

async fn simulate_file(c: &Connection, file: &str, name: &str, found: &mut Found) -> Plan {
    match packagekit::install_file(c, file, name, true, &mut |_, _| {}, &mut |_| {}).await {
        Ok(others) => {
            found.note = also(&others, Verb::Install);
            found.ready = true;
            Plan::InstallFile {
                file: file.into(),
                name: name.into(),
            }
        }
        Err(e) => {
            found.note = pk_text(e);
            Plan::Nothing
        }
    }
}

fn lock(s: &Mutex<Stopper>) -> std::sync::MutexGuard<'_, Stopper> {
    s.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Run `plan`, posting its progress.
fn run(
    plan: &Plan,
    verb: Verb,
    conn: Option<&Connection>,
    poster: &Poster,
    stop: &Mutex<Stopper>,
) -> Result<(), Failure> {
    lock(stop).new_run();
    let mut doing = "Preparing".to_string();
    let mut report = |fraction: Option<f64>, line: Option<String>| {
        if let Some(l) = line {
            doing = l;
        }
        poster.post(Msg::Progress {
            fraction,
            doing: doing.clone(),
        });
    };
    report(None, None);
    let mut watch = |tx: packagekit::Tx| match tx {
        packagekit::Tx::Created(tid) => lock(stop).tid = Some(tid),
        packagekit::Tx::Cancellable(can) => poster.post(Msg::Cancellable(can)),
    };
    let pk = |r: Result<Vec<String>, packagekit::Error>| match r {
        Ok(_) => Ok(()),
        Err(packagekit::Error::Dismissed) => Err(Failure::Dismissed),
        Err(e) => Err(Failure::Text(pk_text(e))),
    };
    let done = match (plan, conn) {
        (Plan::Nothing, _) => Err(Failure::Text("There is nothing to do.".into())),
        (Plan::Pacman(path), _) => pkgfile::pacman_install(path, &mut |f, s| report(Some(f), s))
            .map_err(|e| match e {
                pkgfile::PacmanError::Dismissed => Failure::Dismissed,
                pkgfile::PacmanError::Failed(t) => Failure::Text(t),
            }),
        (Plan::Launcher(file), _) => reprisefs::software::remove_launcher(file)
            .map_err(|e| Failure::Text(format!("The launcher could not be removed: {e}."))),
        (Plan::Flatpak { at, args }, _) => {
            let at = install_of(*at);
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let what = match verb {
                Verb::Install => "Installing",
                Verb::Uninstall => "Uninstalling",
            };
            let before = flatpak::refs(at);
            flatpak::adopt_orphans();
            let ran = flatpak::follow(
                &args,
                &mut |id, f| {
                    let line = (!id.is_empty()).then(|| format!("{what} {id}"));
                    report(Some(f), line);
                },
                &mut |fd| {
                    lock(stop).pidfd = Some(fd);
                    poster.post(Msg::Cancellable(true));
                },
                &mut |bytes| poster.post(Msg::Download(bytes)),
            );
            lock(stop).pidfd = None;
            flatpak::sweep_orphans();
            match (ran, before) {
                (Err(e), Some(before)) if lock(stop).cancelled => {
                    poster.post(Msg::Cancellable(false));
                    settle_cancel(at, &before, verb, &mut report).map_err(|()| Failure::Text(e))
                }
                (ran, _) => ran.map_err(Failure::Text),
            }
        }
        (_, None) => Err(Failure::Text("There is no system bus.".into())),
        (Plan::Remove(id), Some(c)) => pk(zbus::block_on(packagekit::remove(
            c,
            id,
            false,
            &mut report,
            &mut watch,
        ))),
        (Plan::InstallFile { file, name }, Some(c)) => pk(zbus::block_on(
            packagekit::install_file(c, file, name, false, &mut report, &mut watch),
        )),
    };
    let cancelled = std::mem::take(&mut lock(stop).cancelled);
    match done {
        Err(Failure::Text(_)) if cancelled => Err(Failure::Text(match verb {
            Verb::Install => "The install was cancelled.".into(),
            Verb::Uninstall => "The uninstall was cancelled.".into(),
        })),
        done => done,
    }
}

/// After a cancelled Flatpak job, once the installation holds still: Ok when its app came or
/// went anyway, else Err(()) once the runtimes that came for it alone are uninstalled again.
fn settle_cancel(
    at: Install,
    before: &flatpak::Refs,
    verb: Verb,
    report: &mut dyn FnMut(Option<f64>, Option<String>),
) -> Result<(), ()> {
    report(None, Some("Waiting for Flatpak to stop".into()));
    let Some(after) = flatpak::settled_refs(at) else {
        return Err(());
    };
    match flatpak::settle(before, &after, verb == Verb::Install) {
        flatpak::Settled::Done => Ok(()),
        flatpak::Settled::Undo(refs) if refs.is_empty() => Err(()),
        flatpak::Settled::Undo(refs) => {
            report(
                None,
                Some("Removing what it installed before the cancel".into()),
            );
            let mut args = vec!["uninstall", flatpak::flag(at), "-y"];
            args.extend(refs.iter().map(String::as_str));
            let undone = flatpak::follow(
                &args,
                &mut |_, f| report(Some(f), None),
                &mut |_| {},
                &mut |_| {},
            );
            flatpak::sweep_orphans();
            if let Err(e) = undone {
                log::warn!("removing {refs:?} after a cancel: {e}");
            }
            Err(())
        }
    }
}

/// Where `file`'s private copy goes: `$XDG_RUNTIME_DIR` (else the temp folder) when it has the
/// room, else `/var/tmp` on disk, which root's installers read as well.
fn copy_parent(file: &Path) -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(std::env::temp_dir);
    let size = std::fs::metadata(file).map_or(0, |m| m.len());
    pkgfile::copy_parent(&runtime, Path::new("/var/tmp"), size, pkgfile::free_bytes)
}

/// Read `target`, post what it is, then run it each time `go` says so, until it ends other than
/// by a dismissed password prompt.
fn work(target: Target, go: Receiver<()>, poster: Poster, stop: Arc<Mutex<Stopper>>) {
    let conn = zbus::block_on(Connection::system()).ok();
    // `_copy` lives until the job ends: the plan installs it.
    let (found, plan, verb, _copy) = match &target {
        Target::Uninstall(file) => {
            let (f, p) = read_program(file, conn.as_ref());
            (f, p, Verb::Uninstall, None)
        }
        Target::Install(file) => match pkgfile::private_copy(file, &copy_parent(file)) {
            Ok(c) => {
                lock(&stop).copy = Some(c.dir().to_path_buf());
                let (f, p) = read_file(&c.file, c.signed, conn.as_ref());
                (f, p, Verb::Install, Some(c))
            }
            Err(e) => {
                let found = Found {
                    name: file
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    note: format!("Reprise could not read the file: {e}."),
                    ..Found::default()
                };
                (found, Plan::Nothing, Verb::Install, None)
            }
        },
    };
    poster.post(Msg::Found(found));
    while go.recv().is_ok() {
        let ended = run(&plan, verb, conn.as_ref(), &poster, &stop);
        let again = ended == Err(Failure::Dismissed);
        poster.post(Msg::Ended(ended));
        if !again {
            break;
        }
    }
}

/// Ask the running job to stop: SIGINT to `flatpak`, `Cancel` to the PackageKit transaction.
fn cancel(stop: &Arc<Mutex<Stopper>>) {
    let mut s = lock(stop);
    s.cancelled = true;
    if let Some(fd) = &s.pidfd {
        if let Err(e) = fd.interrupt() {
            log::warn!("cancelling flatpak: {e}");
        }
    } else if let Some(tid) = s.tid.clone() {
        std::thread::spawn(move || {
            let asked = zbus::block_on(async {
                let c = Connection::system().await?;
                packagekit::cancel(&c, &tid).await
            });
            if let Err(e) = asked {
                log::warn!("cancelling {tid}: {e}");
            }
        });
    }
}

struct Installer {
    sheet: Sheet,
    target: Option<Target>,
    form: Option<Form>,
    /// The sheet changed since the form was built.
    stale: bool,
    worker: Option<Receiver<Msg>>,
    go: Option<Sender<()>>,
    /// The worker, joined when the program ends so the file's private copy is deleted.
    thread: Option<std::thread::JoinHandle<()>>,
    stop: Arc<Mutex<Stopper>>,
    icon: Option<String>,
    icon_loaded: bool,
    units: TextUnits,
    asked: (i32, i32),
}

/// The rows a sheet shows while it reads: as many as every target ends with, so it keeps its size.
fn blank_facts() -> Vec<(String, String)> {
    vec![(String::new(), String::new()); 4]
}

impl Installer {
    fn new(target: Target) -> Installer {
        let (verb, path) = match &target {
            Target::Install(p) => (Verb::Install, p),
            Target::Uninstall(p) => (Verb::Uninstall, p),
        };
        let name = match verb {
            Verb::Install => path.file_name(),
            Verb::Uninstall => path.file_stem(),
        };
        let sheet = Sheet {
            verb,
            name: name
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            icon: None,
            facts: blank_facts(),
            note: String::new(),
            stage: Stage::Reading,
        };
        let asked = installsheet::client(&sheet);
        Installer {
            sheet,
            target: Some(target),
            form: None,
            stale: true,
            worker: None,
            go: None,
            thread: None,
            stop: Arc::default(),
            icon: None,
            icon_loaded: false,
            units: TextUnits::ONE,
            asked,
        }
    }

    fn set_stage(&mut self, stage: Stage) {
        self.sheet.stage = stage;
        self.stale = true;
    }

    fn form(&mut self, ctx: &Ctx) -> &mut Form {
        let forms = ctx.forms();
        let paint = forms.paint();
        if self.form.as_ref().is_some_and(|f| f.units() != paint.text) {
            self.stale = true;
        }
        if std::mem::take(&mut self.stale) || self.form.is_none() {
            let px = forms.fonts.size_px;
            let mut f = installsheet::form(&self.sheet, &ctx.text, px, paint.text);
            f.drawn_with(&paint);
            if let Some(old) = &self.form {
                f.carry_gate(old);
            }
            self.form = Some(f);
        }
        self.form.get_or_insert_with(|| Form::new(Vec::new()))
    }

    /// Take what the worker posted. A worker gone before it ended the job (it panicked) ends it
    /// as failed, so Close works again.
    fn drain(&mut self) {
        while let Some(rx) = &self.worker {
            match rx.try_recv() {
                Ok(m) => self.worker_msg(m),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.worker = None;
                    if matches!(
                        self.sheet.stage,
                        Stage::Reading | Stage::Asking { .. } | Stage::Running { .. }
                    ) {
                        let why = match self.sheet.verb {
                            Verb::Install => "The install stopped unexpectedly.",
                            Verb::Uninstall => "The uninstall stopped unexpectedly.",
                        };
                        self.worker_msg(Msg::Ended(Err(Failure::Text(why.into()))));
                    }
                }
            }
        }
    }

    fn worker_msg(&mut self, m: Msg) {
        match m {
            Msg::Found(f) => {
                self.sheet.name = f.name;
                if !f.facts.is_empty() {
                    self.sheet.facts = f.facts;
                }
                self.sheet.note = f.note;
                self.icon = f.icon;
                self.icon_loaded = false;
                self.set_stage(if f.refused {
                    Stage::Refused
                } else {
                    Stage::Asking { ready: f.ready }
                });
            }
            Msg::Progress { fraction, doing } => {
                let cancellable = match &self.sheet.stage {
                    Stage::Running { cancellable, .. } => *cancellable,
                    _ => false,
                };
                reprisebus::launcher::update(DESKTOP_ID, Some((fraction.unwrap_or(0.0), false)));
                self.set_stage(Stage::Running {
                    fraction,
                    doing,
                    cancellable,
                });
            }
            Msg::Download(bytes) => {
                for (label, value) in &mut self.sheet.facts {
                    if label == "Size:" && value == SIZE_LATER {
                        *value = format!("Up to {} to download", size_text(Some(bytes)));
                        self.stale = true;
                    }
                }
            }
            Msg::Cancellable(can) => {
                if let Stage::Running { cancellable, .. } = &mut self.sheet.stage {
                    *cancellable = can;
                    self.stale = true;
                }
            }
            Msg::Ended(ended) => {
                reprisebus::launcher::update(DESKTOP_ID, None);
                self.set_stage(match ended {
                    Ok(()) => Stage::Done,
                    Err(Failure::Dismissed) => Stage::Asking { ready: true },
                    Err(Failure::Text(why)) => {
                        log::warn!("{}: {why}", self.sheet.title());
                        Stage::Failed(why)
                    }
                });
            }
        }
    }

    fn load_icon(&mut self, ctx: &mut Ctx) {
        if std::mem::replace(&mut self.icon_loaded, true) {
            return;
        }
        let fallback = match self.sheet.verb {
            Verb::Install => "system-software-install",
            Verb::Uninstall => "application-x-executable",
        };
        let icon = (self.icon.as_deref())
            .and_then(|n| ctx.icons.get(n, 32))
            .or_else(|| ctx.icons.get(fallback, 32));
        if icon.is_some() {
            self.sheet.icon = icon;
            self.stale = true;
        }
    }

    fn choose(&mut self, choice: Choice) -> Reply {
        match choice {
            Choice::Go if self.sheet.stage == (Stage::Asking { ready: true }) => {
                self.set_stage(Stage::Running {
                    fraction: None,
                    doing: "Preparing".into(),
                    cancellable: false,
                });
                if let Some(go) = &self.go {
                    let _ = go.send(());
                }
                Reply::redraw()
            }
            Choice::Go => Reply::none(),
            Choice::Cancel => match &self.sheet.stage {
                Stage::Running { cancellable, .. } => {
                    if *cancellable {
                        cancel(&self.stop);
                    }
                    Reply::none()
                }
                _ => quit(),
            },
            Choice::Close => quit(),
        }
    }

    fn dispatch(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        self.load_icon(ctx);
        if matches!(ev, Event::Wake) {
            self.drain();
            self.load_icon(ctx);
            return Reply::redraw();
        }
        if matches!(ev, Event::CloseRequest) {
            let choice = self
                .sheet
                .choice(&reprisekit::controls::form::Event::Cancel);
            return choice.map_or_else(Reply::none, |c| self.choose(c));
        }
        let Some(fev) = form_input(self.form(ctx), &ev, ctx) else {
            return Reply::none();
        };
        match self.sheet.choice(&fev) {
            Some(c) => self.choose(c),
            None if fev == reprisekit::controls::form::Event::None => Reply::none(),
            None => Reply::redraw(),
        }
    }
}

fn quit() -> Reply {
    Reply {
        quit: true,
        ..Reply::none()
    }
}

impl App for Installer {
    fn title(&self) -> String {
        self.sheet.title().into()
    }

    fn app_id(&self) -> String {
        "reprise.install".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.units.size(self.asked)
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        let Some(target) = self.target.take() else {
            return repriseapp::Start::Continue;
        };
        let (poster, rx) = Poster::channel(waker);
        let (go, go_rx) = channel();
        let stop = self.stop.clone();
        let spawned = std::thread::Builder::new()
            .name("reprise-install".into())
            .spawn(move || work(target, go_rx, poster, stop));
        match spawned {
            Ok(h) => self.thread = Some(h),
            Err(e) => log::warn!("could not start the install thread: {e}"),
        }
        self.worker = Some(rx);
        self.go = Some(go);
        repriseapp::Start::Continue
    }

    fn event(&mut self, _win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.units = ctx.forms().text;
        let mut reply = self.dispatch(ev, ctx);
        let size = self.units.size(installsheet::client(&self.sheet));
        if size != self.units.size(self.asked) {
            self.asked = installsheet::client(&self.sheet);
            reply.size = Some(size);
            reply.redraw = true;
        }
        reply
    }

    fn draw(&mut self, _win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        let px = forms.fonts.size_px;
        let paint = Paint {
            s: 1,
            ..forms.paint()
        };
        self.form(ctx);
        if let Some(f) = &self.form {
            installsheet::draw(c, &ctx.text, &paint, f, px);
        }
    }
}

impl repriseapp::Extended for Installer {
    fn emergency(&mut self) {
        if let Some(dir) = lock(&self.stop).copy.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn text_sized(&self) -> bool {
        true
    }

    fn fixed(&self) -> bool {
        true
    }

    fn caption(&self, _win: WinId) -> Caption {
        Caption::Fixed
    }

    fn access(&mut self, _win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        let px = forms.fonts.size_px;
        self.form(ctx);
        if let Some(f) = &self.form
            && let Some(focus) = installsheet::access(f, tree, &ctx.text, px)
        {
            tree.set_focus(focus);
        }
    }
}

impl Drop for Installer {
    fn drop(&mut self) {
        // Without `go` the worker's wait ends, and it drops the copy as it returns.
        self.go = None;
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

/// Runs the sheet for `target`.
pub fn main(target: Target) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    repriseapp::run(Box::new(Installer::new(target))).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_names_the_target() {
        let args = |a: &[&str]| {
            a.iter()
                .map(Into::into)
                .collect::<Vec<std::ffi::OsString>>()
        };
        assert_eq!(
            target(&args(&["--install", "/d/foot.pkg.tar.zst"])),
            Some(Target::Install("/d/foot.pkg.tar.zst".into()))
        );
        assert_eq!(
            target(&args(&[
                "--uninstall",
                "/usr/share/applications/foot.desktop"
            ])),
            Some(Target::Uninstall(
                "/usr/share/applications/foot.desktop".into()
            ))
        );
        let Some(Target::Install(dashed)) = target(&args(&["--install", "-x.deb"])) else {
            panic!("a relative path is a target");
        };
        assert!(
            dashed.is_absolute() && dashed.ends_with("-x.deb"),
            "{dashed:?}"
        );
        assert_eq!(target(&args(&["--tray"])), None);
        assert!(!asks_for_sheet(&args(&["--tray"])));
        assert_eq!(target(&args(&["--install"])), None);
        assert!(
            asks_for_sheet(&args(&["--install"])),
            "a restored sheet with no file"
        );
    }

    #[test]
    fn the_note_names_the_other_packages_a_change_touches() {
        let ids = |names: &[&str]| -> Vec<String> {
            names
                .iter()
                .map(|n| format!("{n};1-1;x86_64;installed"))
                .collect()
        };
        assert_eq!(also(&[], Verb::Uninstall), "");
        assert_eq!(
            also(&ids(&["foot-terminfo"]), Verb::Uninstall),
            "Uninstalling it also removes 1 package that needs it: foot-terminfo."
        );
        assert_eq!(
            also(
                &ids(&["b", "a", "c", "d", "e", "f", "g", "h"]),
                Verb::Install
            ),
            "Installing it also installs or updates 8 packages: a, b, c, d, e, f and 2 more."
        );
    }

    #[test]
    fn a_launcher_names_the_flatpak_that_installed_it() {
        let named = |id: &str| (id == "org.gnome.Epiphany").then(|| "Web".to_string());
        let text = "[Desktop Entry]\nName=Maps\nX-Flatpak=org.gnome.Epiphany\n";
        assert_eq!(
            launcher_facts(text, named),
            vec![("Installed by:".to_string(), "Web".to_string())]
        );
        let other = "[Desktop Entry]\nName=Maps\nX-Flatpak=org.example.Store\n";
        assert_eq!(launcher_facts(other, named)[0].1, "org.example.Store");
        assert!(launcher_facts("[Desktop Entry]\nName=Maps\n", named).is_empty());
    }

    #[test]
    fn a_flatpakref_says_where_its_runtime_comes_from_and_that_the_sources_stay() {
        let at = |runtime: &str| {
            let text = format!(
                "[Flatpak Ref]\nName=org.example.Pinball\nUrl=https://dl.example.org/repo/\n\
                 RuntimeRepo={runtime}\n"
            );
            let mut found = Found::default();
            ref_found(reprisefs::software::flatpak_ref(&text).unwrap(), &mut found);
            found
        };
        let other = at("https://dl.flathub.org/repo/flathub.flatpakrepo");
        assert_eq!(
            other.note,
            "It is installed for every user of this computer, with the runtime it needs from \
             dl.flathub.org. Both sources stay added for later installs and updates."
        );
        assert_eq!(other.facts[3].1, "dl.example.org (Flatpak)");
        assert!(other.ready);
        for same in ["", "https://dl.example.org/repo/example.flatpakrepo"] {
            assert_eq!(
                at(same).note,
                "It is installed for every user of this computer, with the runtime it needs from \
                 the same source, which stays added for later installs and updates."
            );
        }
    }

    #[test]
    fn an_unsigned_packager_reads_as_unverified() {
        assert_eq!(
            packager("Arch Linux <a@b>".into(), false),
            "(unverified) Arch Linux <a@b>"
        );
        assert_eq!(
            packager("Arch Linux <a@b>".into(), true),
            "Arch Linux <a@b> (signature checked when it installs)"
        );
        for level in ["PackageOptional\nPackageTrustedOnly\n", "PackageRequired\n"] {
            assert!(pkgfile::sigs_checked(level), "{level:?} checks a .sig");
        }
        for level in ["PackageNever\n", "Never\n", ""] {
            assert!(!pkgfile::sigs_checked(level), "{level:?} ignores a .sig");
        }
        assert_eq!(fact("Packager:", packager(String::new(), false)).1, UNKNOWN);
    }

    #[test]
    fn a_repository_is_named_by_its_host() {
        assert_eq!(host("https://dl.flathub.org/repo/"), "dl.flathub.org");
        assert_eq!(host("dl.example.org"), "dl.example.org");
    }

    /// Asking needs a ready target; Cancel before the run ends the program, during it asks the job
    /// to stop, and a dismissed password prompt asks again.
    #[test]
    fn the_sheet_runs_only_from_a_ready_ask_and_returns_after_a_dismissal() {
        let mut app = Installer::new(Target::Install("/d/foot.pkg.tar.zst".into()));
        assert_eq!(app.sheet.name, "foot.pkg.tar.zst");
        assert_eq!(
            app.sheet.facts.len(),
            4,
            "the reading sheet keeps the final size"
        );
        let (go, go_rx) = channel();
        app.go = Some(go);
        assert!(
            !app.choose(Choice::Go).redraw,
            "nothing to run while reading"
        );
        app.worker_msg(Msg::Found(Found {
            name: "foot".into(),
            facts: blank_facts(),
            ready: true,
            ..Found::default()
        }));
        assert_eq!(app.sheet.stage, Stage::Asking { ready: true });
        app.choose(Choice::Go);
        assert!(go_rx.try_recv().is_ok(), "the worker was told to go");
        assert!(matches!(app.sheet.stage, Stage::Running { .. }));
        assert!(!app.choose(Choice::Cancel).quit, "a run is not left behind");
        app.worker_msg(Msg::Ended(Err(Failure::Dismissed)));
        assert_eq!(app.sheet.stage, Stage::Asking { ready: true });
        app.choose(Choice::Go);
        app.worker_msg(Msg::Ended(Ok(())));
        assert_eq!(app.sheet.stage, Stage::Done);
        assert!(app.choose(Choice::Close).quit);
    }

    #[test]
    fn a_worker_gone_mid_run_fails_the_sheet_and_close_quits() {
        let mut app = Installer::new(Target::Install("/d/foot.pkg.tar.zst".into()));
        let (tx, rx) = channel();
        app.worker = Some(rx);
        tx.send(Msg::Found(Found {
            ready: true,
            ..Found::default()
        }))
        .unwrap();
        app.drain();
        app.choose(Choice::Go);
        assert!(!app.choose(Choice::Cancel).quit, "Cancel is greyed mid-run");
        drop(tx);
        app.drain();
        assert_eq!(
            app.sheet.stage,
            Stage::Failed("The install stopped unexpectedly.".into())
        );
        assert!(app.choose(Choice::Cancel).quit, "Close ends the program");

        let mut app = Installer::new(Target::Install("/d/foot.pkg.tar.zst".into()));
        let (tx, rx) = channel();
        app.worker = Some(rx);
        tx.send(Msg::Ended(Ok(()))).unwrap();
        drop(tx);
        app.drain();
        assert_eq!(
            app.sheet.stage,
            Stage::Done,
            "a worker done and gone is no failure"
        );
    }

    #[test]
    fn a_flatpakref_size_fills_in_once_flatpak_tells_it() {
        let mut app = Installer::new(Target::Install("/d/x.flatpakref".into()));
        let facts = vec![fact("App ID:", "org.example.X"), fact("Size:", SIZE_LATER)];
        app.worker_msg(Msg::Found(Found {
            facts,
            ready: true,
            ..Found::default()
        }));
        app.choose(Choice::Go);
        app.worker_msg(Msg::Download(30_400_000));
        assert_eq!(app.sheet.facts[1].1, "Up to 28.9 MB to download");
        app.worker_msg(Msg::Download(1_000_000));
        assert_eq!(
            app.sheet.facts[1].1, "Up to 28.9 MB to download",
            "the first size stays"
        );

        let mut bundle = Installer::new(Target::Install("/d/x.flatpak".into()));
        bundle.worker_msg(Msg::Found(Found {
            facts: vec![fact("Size:", "12.0 MB")],
            ready: true,
            ..Found::default()
        }));
        bundle.worker_msg(Msg::Download(30_400_000));
        assert_eq!(bundle.sheet.facts[0].1, "12.0 MB", "a known size is kept");
    }

    #[test]
    fn the_private_copy_is_removed_by_emergency_after_a_run() {
        use repriseapp::Extended;
        let dir = std::env::temp_dir().join(format!("reprise-install-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("foot.pkg.tar.zst"), b"x").unwrap();
        let mut app = Installer::new(Target::Install("/d/foot.pkg.tar.zst".into()));
        lock(&app.stop).copy = Some(dir.clone());
        let (poster, _rx) = Poster::channel(repriseapp::Waker::noop());
        let ran = run(&Plan::Nothing, Verb::Install, None, &poster, &app.stop);
        assert!(ran.is_err());
        app.emergency();
        assert!(!dir.exists(), "emergency removes the copy a run kept");
    }

    #[test]
    fn a_refused_uninstall_offers_only_close() {
        let mut app = Installer::new(Target::Uninstall(
            "/usr/share/applications/x.desktop".into(),
        ));
        let (go, go_rx) = channel();
        app.go = Some(go);
        app.worker_msg(Msg::Found(Found {
            name: "X".into(),
            note: "Uninstalling it would remove glibc (needed by the system).".into(),
            refused: true,
            ..Found::default()
        }));
        assert_eq!(app.sheet.stage, Stage::Refused);
        assert!(!app.choose(Choice::Go).redraw);
        assert!(go_rx.try_recv().is_err(), "the worker never runs");
        let close = app
            .sheet
            .choice(&reprisekit::controls::form::Event::Default);
        assert_eq!(close, Some(Choice::Close));
    }
}
