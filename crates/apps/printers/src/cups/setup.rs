//! Changing, adding and renaming a queue.

use super::*;

/// What Properties changes on a printer, field by field: `None` leaves
/// it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Change {
    pub info: Option<String>,
    pub location: Option<String>,
    pub device_uri: Option<String>,
    pub shared: Option<bool>,
    pub defaults: Vec<(String, Value)>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewPrinter {
    pub name: String,
    pub device_uri: String,
    pub driver: Driver,
    pub info: String,
    pub location: String,
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
/// one's address, PPD, description, sharing and defaults, its jobs move
/// over, it becomes the default if the old one was, and the old one goes.
/// cups-pk-helper's PrinterRename takes the same steps as root but leaves
/// the defaults the PPD does not hold, so they follow as calls. The
/// user's own default follows the name either way.
pub fn rename(c: &Client, old: &str, new: &str, s: &Snapshot) -> Result<()> {
    let ppd = ppd_of(c, old)?;
    let defaults = match options(c, old) {
        Ok(o) => carried_defaults(&o, ppd.is_some()),
        Err(e) => {
            log::warn!("reading {old}'s defaults: {e:?}");
            Vec::new()
        }
    };
    let peer = rename_by_peer(c, old, new, s, ppd, &defaults);
    if matches!(peer, Err(Error::Denied))
        && c.helper.installed()
        && let Some(e) = helper_rename_refusal(old, s)
    {
        return Err(e);
    }
    c.admin(peer, || {
        let mut calls = vec![Call::PrinterRename(old.into(), new.into())];
        calls.extend(defaults.iter().filter_map(|(k, v)| {
            let option = option_of(k).to_string();
            Some(Call::PrinterAddOptionDefault(
                new.into(),
                option,
                vec![value_text(v)?],
            ))
        }));
        calls
    })?;
    if let Some(text) = renamed_default(&lpoptions_text(), &system_lpoptions_text(), old, new) {
        write_lpoptions(&text)?;
    }
    Ok(())
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

fn rename_by_peer(
    c: &Client,
    old: &str,
    new: &str,
    s: &Snapshot,
    ppd: Option<Vec<u8>>,
    defaults: &[(String, Value)],
) -> Result<()> {
    let Some(p) = s.printer(old) else {
        return Err(Error::Refused(format!("{old} is no longer there.")));
    };
    if s.printer(new).is_some() {
        return Err(Error::Refused(format!(
            "There is already a printer called {new}."
        )));
    }
    let mut req = Request::new(CUPS_ADD_MODIFY_PRINTER)
        .op_attr("printer-uri", Value::uri(&printer_uri(new)))
        .printer_attr("device-uri", Value::uri(&p.device_uri))
        .printer_attr("printer-info", Value::text(&p.info))
        .printer_attr("printer-location", Value::text(&p.location))
        .printer_attr("printer-is-shared", Value::Bool(p.shared))
        .printer_attr("printer-is-accepting-jobs", Value::Bool(true))
        .printer_attr("printer-state", Value::Enum(3));
    for (k, v) in defaults {
        req = req.printer_attr(k, v.clone());
    }
    if let Some(ppd) = ppd {
        req = req.with_data(ppd);
    }
    c.call("/admin/", req)?;
    for j in s.jobs_of(old) {
        let req = Request::new(CUPS_MOVE_JOB)
            .op_attr("job-uri", Value::uri(&job_uri(j.id)))
            .add(JOB, "job-printer-uri", vec![Value::uri(&printer_uri(new))]);
        if let Err(e) = c.call("/jobs/", req) {
            log::warn!("moving job {} to {new}: {e:?}", j.id);
        }
    }
    if s.default.as_deref() == Some(old) {
        set_default(c, new)?;
    }
    if p.paused() {
        pause(c, new)?;
    }
    delete(c, old)
}
