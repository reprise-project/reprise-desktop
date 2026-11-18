//! Changing, adding and renaming a queue.

use super::*;

/// What Properties changes on a printer, field by field: `None` leaves
/// it. `Debug` drops the address's user name and password.
#[derive(Clone, Default, PartialEq)]
pub struct Change {
    pub info: Option<String>,
    pub location: Option<String>,
    pub device_uri: Option<String>,
    pub shared: Option<bool>,
    pub defaults: Vec<(String, Value)>,
}

impl std::fmt::Debug for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Change")
            .field("info", &self.info)
            .field("location", &self.location)
            .field(
                "device_uri",
                &self.device_uri.as_deref().map(without_userinfo),
            )
            .field("shared", &self.shared)
            .field("defaults", &self.defaults)
            .finish()
    }
}

/// A device URI whose `Debug` drops the user name and password.
#[derive(Clone, PartialEq)]
pub struct DeviceUri(pub String);

impl std::fmt::Debug for DeviceUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&without_userinfo(&self.0))
    }
}

/// Whether cupsd may hold a user name and password in an address it reports without them: an
/// SMB queue on a password-protected share has them in its address.
pub fn may_hide_password(device_uri: &str) -> bool {
    device_uri.starts_with("smb:")
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.info.is_none()
            && self.location.is_none()
            && self.device_uri.is_none()
            && self.shared.is_none()
            && self.defaults.is_empty()
    }

    /// The CUPS-Add-Modify-Printer request. With the queue's `ppd`, the
    /// defaults go into its Default lines and the PPD goes up with the
    /// request, as `lpadmin -o` sends it. The `stale` defaults are deleted.
    pub(super) fn into_request(self, name: &str, ppd: Option<&[u8]>, stale: &[String]) -> Request {
        let mut req = Request::new(CUPS_ADD_MODIFY_PRINTER)
            .op_attr("printer-uri", Value::uri(&printer_uri(name)));
        if let Some(s) = self.info {
            req = req.printer_attr("printer-info", Value::text(&s));
        }
        if let Some(s) = self.location {
            req = req.printer_attr("printer-location", Value::text(&s));
        }
        if let Some(s) = self.device_uri {
            req = req.printer_attr("device-uri", Value::uri(&s));
        }
        if let Some(b) = self.shared {
            req = req.printer_attr("printer-is-shared", Value::Bool(b));
        }
        let texts: Vec<(String, String)> = self
            .defaults
            .iter()
            .filter_map(|(k, v)| Some((option_of(k).to_string(), value_text(v)?)))
            .collect();
        let (data, applied) = match ppd {
            Some(p) if !texts.is_empty() => {
                let (out, applied) = ppd::with_defaults(p, &texts);
                ((out != p).then_some(out), applied)
            }
            _ => (None, Vec::new()),
        };
        for k in stale {
            if !self.defaults.iter().any(|(d, _)| d == k) {
                req = req.printer_attr(k, Value::Empty(ipp::DELETE_ATTR));
            }
        }
        for (k, v) in self.defaults {
            let option = option_of(&k);
            let carried = ppd::CARRIED.contains(&option) && applied.iter().any(|a| a == option);
            let v = if carried {
                Value::Empty(ipp::DELETE_ATTR)
            } else {
                v
            };
            req = req.printer_attr(&k, v);
        }
        if let Some(d) = data {
            req = req.with_data(d);
        }
        req
    }

    /// The same change through cups-pk-helper. On a queue with a PPD,
    /// PrinterAddOption sets the Default line as `lpadmin -o` does.
    pub(super) fn calls(&self, name: &str, has_ppd: bool, stale: &[String]) -> Vec<Call> {
        let n = || name.to_string();
        let mut out = Vec::new();
        if let Some(s) = &self.info {
            out.push(Call::PrinterSetInfo(n(), s.clone()));
        }
        if let Some(s) = &self.location {
            out.push(Call::PrinterSetLocation(n(), s.clone()));
        }
        if let Some(s) = &self.device_uri {
            out.push(Call::PrinterSetDevice(n(), s.clone()));
        }
        if let Some(b) = self.shared {
            out.push(Call::PrinterSetShared(n(), b));
        }
        for k in stale {
            if !self.defaults.iter().any(|(d, _)| d == k) {
                out.push(Call::PrinterAddOptionDefault(
                    n(),
                    option_of(k).into(),
                    Vec::new(),
                ));
            }
        }
        for (k, v) in &self.defaults {
            let Some(text) = value_text(v) else {
                continue;
            };
            let option = option_of(k).to_string();
            if !has_ppd {
                out.push(Call::PrinterAddOptionDefault(n(), option, vec![text]));
                continue;
            }
            out.push(Call::PrinterAddOption(
                n(),
                option.clone(),
                vec![text.clone()],
            ));
            let queue_value = if ppd::CARRIED.contains(&option.as_str()) {
                Vec::new()
            } else {
                vec![text]
            };
            out.push(Call::PrinterAddOptionDefault(n(), option, queue_value));
        }
        out
    }
}

/// "media-default" names the option "media".
fn option_of(attr: &str) -> &str {
    attr.strip_suffix("-default").unwrap_or(attr)
}

fn value_text(v: &Value) -> Option<String> {
    match v {
        Value::Int(i) | Value::Enum(i) => Some(i.to_string()),
        Value::Str(_, s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// The queue's PPD as cupsd serves it; `None` for a queue without one.
fn ppd_of(c: &Client, name: &str) -> Result<Option<Vec<u8>>> {
    match http::get(
        &c.server,
        &format!("/printers/{}.ppd", escape(name)),
        &c.user,
    ) {
        Ok(r) if !r.body.is_empty() => Ok(Some(r.body)),
        Err(e @ http::Error::Unreachable(_)) => Err(e.into()),
        Ok(_) | Err(_) => Ok(None),
    }
}

pub fn modify(c: &Client, name: &str, change: Change) -> Result<()> {
    if change.is_empty() {
        return Ok(());
    }
    let ppd = if change.defaults.is_empty() {
        None
    } else {
        ppd_of(c, name)?
    };
    let stale = stale_defaults(c, name).unwrap_or_else(|e| {
        log::warn!("reading {name}'s defaults: {e:?}");
        Vec::new()
    });
    let calls = change.calls(name, ppd.is_some(), &stale);
    c.admin(
        c.call("/admin/", change.into_request(name, ppd.as_deref(), &stale))
            .map(drop),
        || calls,
    )
}

/// The media and sides defaults cupsd reports twice for queue `name`: a queue
/// option an older Device Settings left beside the PPD's own default.
fn stale_defaults(c: &Client, name: &str) -> Result<Vec<String>> {
    let req = Request::new(GET_PRINTER_ATTRIBUTES)
        .op_attr("printer-uri", Value::uri(&printer_uri(name)))
        .requested(&["media-default", "sides-default"]);
    let r = c.call("/", req)?;
    Ok(r.first(PRINTER).map(stale_in).unwrap_or_default())
}

pub(super) fn stale_in(g: &ipp::Group) -> Vec<String> {
    ppd::CARRIED
        .iter()
        .map(|o| format!("{o}-default"))
        .filter(|n| g.attrs.iter().filter(|a| a.name == *n).count() > 1)
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Driver {
    Everywhere,
    Ppd(String),
    File(std::path::PathBuf),
}

/// A queue to add. `Debug` drops the address's user name and password.
#[derive(Clone, PartialEq, Eq)]
pub struct NewPrinter {
    pub name: String,
    pub device_uri: String,
    pub driver: Driver,
    pub info: String,
    pub location: String,
}

impl std::fmt::Debug for NewPrinter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewPrinter")
            .field("name", &self.name)
            .field("device_uri", &without_userinfo(&self.device_uri))
            .field("driver", &self.driver)
            .field("info", &self.info)
            .field("location", &self.location)
            .finish()
    }
}

pub fn add(c: &Client, p: &NewPrinter) -> Result<()> {
    let mut req = Request::new(CUPS_ADD_MODIFY_PRINTER)
        .op_attr("printer-uri", Value::uri(&printer_uri(&p.name)))
        .printer_attr("device-uri", Value::uri(&p.device_uri))
        .printer_attr("printer-info", Value::text(&p.info))
        .printer_attr("printer-location", Value::text(&p.location))
        .printer_attr("printer-is-accepting-jobs", Value::Bool(true))
        .printer_attr("printer-state", Value::Enum(3));
    match &p.driver {
        Driver::Everywhere => req = req.op_attr("ppd-name", Value::name("everywhere")),
        Driver::Ppd(name) => req = req.op_attr("ppd-name", Value::name(name)),
        Driver::File(path) => {
            let data = std::fs::read(path).map_err(|e| {
                Error::Refused(format!("{} could not be read: {e}.", path.display()))
            })?;
            req = req.with_data(data);
        }
    }
    c.admin(c.call("/admin/", req).map(drop), || add_calls(p))
}

/// Adding as cups-pk-helper does it: its add leaves the printer's state
/// to cupsd, so enabling and accepting follow.
pub(super) fn add_calls(p: &NewPrinter) -> Vec<Call> {
    let (name, uri, info, location) = (
        p.name.clone(),
        p.device_uri.clone(),
        p.info.clone(),
        p.location.clone(),
    );
    let add = match &p.driver {
        Driver::Everywhere => Call::PrinterAdd {
            name,
            uri,
            ppd: "everywhere".into(),
            info,
            location,
        },
        Driver::Ppd(ppd) => Call::PrinterAdd {
            name,
            uri,
            ppd: ppd.clone(),
            info,
            location,
        },
        Driver::File(path) => Call::PrinterAddWithPpdFile {
            name,
            uri,
            file: path.display().to_string(),
            info,
            location,
        },
    };
    vec![
        add,
        Call::PrinterSetEnabled(p.name.clone(), true),
        Call::PrinterSetAcceptJobs(p.name.clone(), true),
    ]
}

/// Rename a printer: CUPS has no rename, so a new queue takes the old
/// one's address, PPD, description, sharing, defaults, state, state message
/// and reasons, policies, allowed or denied users, banners and quotas, its jobs
/// move over (all or none), it becomes the default if the old one was, and
/// the old one goes.
/// cups-pk-helper's PrinterRename takes the same steps as root but leaves
/// the defaults the PPD does not hold, so they follow as calls; the state
/// message, reasons and quotas stay behind on that path. The
/// user's own default follows the name either way. cupsd reports the address without its user
/// name and password, so `device_uri`, when given, is the address the new queue takes instead.
pub fn rename(
    c: &Client,
    old: &str,
    new: &str,
    device_uri: Option<&DeviceUri>,
    s: &Snapshot,
) -> Result<()> {
    if let Some(why) = name_clash(old, new, s) {
        return Err(Error::Refused(why));
    }
    let ppd = ppd_of(c, old)?;
    let defaults = match options(c, old) {
        Ok(o) => carried_defaults(&o, ppd.is_some()),
        Err(e) => {
            log::warn!("reading {old}'s defaults: {e:?}");
            Vec::new()
        }
    };
    let peer = rename_by_peer(c, old, new, device_uri, s, ppd, &defaults);
    if matches!(peer, Err(Error::Denied))
        && c.helper.installed()
        && let Some(e) = helper_rename_refusal(old, s)
    {
        return Err(e);
    }
    c.admin(peer, || {
        helper_rename_calls(old, new, device_uri, &defaults)
    })?;
    if let Some(text) = renamed_default(&lpoptions_text(), &system_lpoptions_text(), old, new) {
        write_lpoptions(&text)?;
    }
    Ok(())
}

/// Why `old` cannot become `new`: cupsd finds a printer by its name in any case, so a rename onto
/// another case of a name, `old`'s own included, would modify that printer and then delete it.
pub fn name_clash(old: &str, new: &str, s: &Snapshot) -> Option<String> {
    let p = s
        .printers
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(new))?;
    Some(if p.name == old {
        format!("{new} differs from {old} only in case, which printer names ignore.")
    } else {
        format!("There is already a printer called {}.", p.name)
    })
}

/// The cups-pk-helper calls renaming `old` to `new`: PrinterRename copies the address without its
/// user name and password, so a given `device_uri` follows it.
pub(super) fn helper_rename_calls(
    old: &str,
    new: &str,
    device_uri: Option<&DeviceUri>,
    defaults: &[(String, Value)],
) -> Vec<Call> {
    let mut calls = vec![Call::PrinterRename(old.into(), new.into())];
    if let Some(u) = device_uri {
        calls.push(Call::PrinterSetDevice(new.into(), u.0.clone()));
    }
    calls.extend(defaults.iter().filter_map(|(k, v)| {
        let option = option_of(k).to_string();
        Some(Call::PrinterAddOptionDefault(
            new.into(),
            option,
            vec![value_text(v)?],
        ))
    }));
    calls
}

/// cups-pk-helper's PrinterRename moves only held jobs: it refuses a queue
/// with jobs waiting or printing and gives no reason, so the refusal is ours.
pub(super) fn helper_rename_refusal(old: &str, s: &Snapshot) -> Option<Error> {
    s.jobs_of(old)
        .any(|j| matches!(j.state, JobState::Pending | JobState::Printing))
        .then(|| {
            Error::Refused(format!(
                "{old} can be renamed once no documents are waiting or printing on it."
            ))
        })
}

/// The user's lpoptions `user` after printer `old` became `new`, when the default it or the
/// system's lpoptions `system` names was `old`: the system's file is not the user's to write, so
/// the user's own default takes the new name. None when the default was another printer.
pub(super) fn renamed_default(user: &str, system: &str, old: &str, new: &str) -> Option<String> {
    (own_default_in(user, system).as_deref() == Some(old)).then(|| with_user_default(user, new))
}

/// The `*-default` attributes a renamed queue keeps: color and quality,
/// and media and sides unless a PPD holds them.
pub(super) fn carried_defaults(o: &Options, has_ppd: bool) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let mut keep = |attr: &str, value: Option<Value>| {
        if let Some(v) = value {
            out.push((attr.to_string(), v));
        }
    };
    if !has_ppd {
        keep(
            "media-default",
            o.media.default.as_deref().map(Value::keyword),
        );
        keep(
            "sides-default",
            o.sides.default.as_deref().map(Value::keyword),
        );
    }
    keep(
        "print-color-mode-default",
        o.color.default.as_deref().map(Value::keyword),
    );
    keep(
        "print-quality-default",
        o.quality
            .default
            .as_deref()
            .and_then(|q| q.parse().ok())
            .map(Value::Enum),
    );
    out
}

/// The attributes a renamed queue copies from the old one's Get-Printer-Attributes as cupsd
/// reports them; cupsd's Add-Modify-Printer sets each.
const CARRIED: &[&str] = &[
    "printer-state-message",
    "printer-error-policy",
    "printer-op-policy",
    "job-sheets-default",
    "requesting-user-name-allowed",
    "requesting-user-name-denied",
    "job-quota-period",
    "job-k-limit",
    "job-page-limit",
];

/// The reasons cupsd derives itself or holds only while a job runs, and drops from printers.conf.
const PASSING_REASONS: &[&str] = &[
    "connecting-to-device",
    "cups-insecure-filter-warning",
    "cups-missing-filter-warning",
];

/// Queue `name`'s [`CARRIED`] attributes and its printer-state-reasons.
fn settings_of(c: &Client, name: &str) -> Result<ipp::Group> {
    let req = Request::new(GET_PRINTER_ATTRIBUTES)
        .op_attr("printer-uri", Value::uri(&printer_uri(name)))
        .requested(&[CARRIED, &["printer-state-reasons"]].concat());
    Ok(c.call("/", req)?
        .first(PRINTER)
        .cloned()
        .unwrap_or_default())
}

/// The CUPS-Add-Modify-Printer request creating `new` as a copy of `p`, in its state, at
/// `device_uri` when given, with the [`CARRIED`] attributes in `settings`.
pub(super) fn rename_request(
    new: &str,
    p: &Printer,
    device_uri: Option<&DeviceUri>,
    ppd: Option<Vec<u8>>,
    defaults: &[(String, Value)],
    settings: &ipp::Group,
) -> Request {
    let uri = device_uri.map_or(p.device_uri.as_str(), |u| u.0.as_str());
    let mut req = Request::new(CUPS_ADD_MODIFY_PRINTER)
        .op_attr("printer-uri", Value::uri(&printer_uri(new)))
        .printer_attr("device-uri", Value::uri(uri))
        .printer_attr("printer-info", Value::text(&p.info))
        .printer_attr("printer-location", Value::text(&p.location))
        .printer_attr("printer-is-shared", Value::Bool(p.shared))
        .printer_attr("printer-is-accepting-jobs", Value::Bool(p.accepting))
        .printer_attr(
            "printer-state",
            Value::Enum(if p.state == PrinterState::Stopped {
                5
            } else {
                3
            }),
        );
    for (k, v) in defaults {
        req = req.printer_attr(k, v.clone());
    }
    for a in settings
        .attrs
        .iter()
        .filter(|a| CARRIED.contains(&a.name.as_str()))
    {
        req = req.add(PRINTER, &a.name, a.values.clone());
    }
    if let Some(ppd) = ppd {
        req = req.with_data(ppd);
    }
    req
}

/// The CUPS-Add-Modify-Printer giving `new` the reasons in `settings` but the
/// [`PASSING_REASONS`]: a request of its own, since a PPD on the one making `new` clears them.
/// None when no reason is left.
fn reasons_request(new: &str, settings: &ipp::Group) -> Option<Request> {
    let reasons: Vec<Value> = settings
        .get("printer-state-reasons")
        .unwrap_or_default()
        .iter()
        .filter(|v| {
            v.as_str()
                .is_some_and(|r| r != "none" && !PASSING_REASONS.contains(&r))
        })
        .cloned()
        .collect();
    (!reasons.is_empty()).then(|| {
        Request::new(CUPS_ADD_MODIFY_PRINTER)
            .op_attr("printer-uri", Value::uri(&printer_uri(new)))
            .add(PRINTER, "printer-state-reasons", reasons)
    })
}

pub(super) fn rename_by_peer(
    c: &Client,
    old: &str,
    new: &str,
    device_uri: Option<&DeviceUri>,
    s: &Snapshot,
    ppd: Option<Vec<u8>>,
    defaults: &[(String, Value)],
) -> Result<()> {
    let Some(p) = s.printer(old) else {
        return Err(Error::Refused(format!("{old} is no longer there.")));
    };
    let settings = settings_of(c, old)?;
    // `new` is made stopped when `old` is, so no job starts on it as it moves over.
    c.call(
        "/admin/",
        rename_request(new, p, device_uri, ppd, defaults, &settings),
    )?;
    if let Some(req) = reasons_request(new, &settings)
        && let Err(e) = c.call("/admin/", req)
    {
        log::warn!("carrying {old}'s state reasons to {new}: {e:?}");
    }
    let mut moved = Vec::new();
    for j in s.jobs_of(old) {
        match move_job(c, j.id, new) {
            Ok(()) => moved.push(j.id),
            Err(_) if gone(c, j.id) => {}
            Err(e) => {
                unmake_rename(c, old, new, &moved);
                return Err(e);
            }
        }
    }
    let default = (s.default.as_deref() == Some(old)).then(|| set_default(c, new));
    finish_rename(old, new, default, || delete(c, old))
}

fn move_job(c: &Client, id: i32, to: &str) -> Result<()> {
    let req = Request::new(CUPS_MOVE_JOB)
        .op_attr("job-uri", Value::uri(&job_uri(id)))
        .add(JOB, "job-printer-uri", vec![Value::uri(&printer_uri(to))]);
    c.call("/jobs/", req).map(drop)
}

/// Whether job `id` finished or was purged since the snapshot: cupsd refuses to move such a
/// job, and it no longer holds up a rename.
fn gone(c: &Client, id: i32) -> bool {
    let req = Request::new(GET_JOB_ATTRIBUTES)
        .op_attr("job-uri", Value::uri(&job_uri(id)))
        .requested(&["job-state"]);
    match c.send("/", req) {
        Ok(r) if r.status == NOT_FOUND => true,
        Ok(r) if r.ok() => r
            .first(JOB)
            .is_some_and(|g| Job::from_group(g).state.done()),
        _ => false,
    }
}

/// Undo a rename whose jobs did not all move: the `moved` ones go back to `old`, and `new` goes
/// once it holds none.
fn unmake_rename(c: &Client, old: &str, new: &str, moved: &[i32]) {
    let mut emptied = true;
    for &id in moved {
        if let Err(e) = move_job(c, id, old) {
            log::warn!("moving job {id} back to {old}: {e:?}");
            emptied = false;
        }
    }
    if emptied && let Err(e) = delete(c, new) {
        log::warn!("deleting {new} after a refused move: {e:?}");
    }
}

/// The end of a rename once queue `new` holds `old`'s jobs: `old` goes even when making `new`
/// the default (`default`, when `old` was) failed, and that failure is the error, since `new`
/// stays either way.
pub(super) fn finish_rename(
    old: &str,
    new: &str,
    default: Option<Result<()>>,
    delete: impl FnOnce() -> Result<()>,
) -> Result<()> {
    delete()?;
    let Some(Err(e)) = default else {
        return Ok(());
    };
    let told =
        format!("{old} is now called {new}, but {new} could not be made the default printer.");
    Err(Error::Refused(match e.text() {
        why if why.is_empty() => told,
        why => format!("{told} {why}"),
    }))
}
