//! Pausing, cancelling and defaulting a printer, the job operations and the test page.

use super::*;

const TEST_PAGES: [(&str, &str); 2] = [
    (
        "/usr/share/cups/data/default-testpage.pdf",
        "application/pdf",
    ),
    (
        "/usr/share/cups/data/testprint",
        "application/vnd.cups-banner",
    ),
];

fn printer_op(c: &Client, op: u16, name: &str) -> Result<()> {
    let req = Request::new(op).op_attr("printer-uri", Value::uri(&printer_uri(name)));
    c.call("/admin/", req).map(drop)
}

pub fn pause(c: &Client, name: &str) -> Result<()> {
    c.admin(printer_op(c, PAUSE_PRINTER, name), || {
        vec![Call::PrinterSetEnabled(name.into(), false)]
    })
}

pub fn resume(c: &Client, name: &str) -> Result<()> {
    c.admin(printer_op(c, RESUME_PRINTER, name), || {
        vec![Call::PrinterSetEnabled(name.into(), true)]
    })
}

/// Cancel every job on `name` and purge its kept aborted ones, which
/// Cancel-Jobs leaves. cups-pk-helper has no per-printer cancel, so it is
/// asked job by job. An account that may not cancel other people's jobs
/// and has no cups-pk-helper cancels and purges its own.
pub fn cancel_all(c: &Client, name: &str) -> Result<()> {
    let kept = aborted(c, Some(name)).unwrap_or_default();
    let peer = printer_op(c, CANCEL_JOBS, name);
    let by_peer = peer.is_ok();
    let all = c.admin(peer, || {
        let mut all = jobs(c, Some(name)).unwrap_or_default();
        all.extend(kept.iter().cloned());
        cancel_calls(&all)
    });
    let purge: Vec<Job> = match all {
        Ok(()) if by_peer => kept,
        Err(Error::NeedsHelper) => {
            let req =
                Request::new(CANCEL_MY_JOBS).op_attr("printer-uri", Value::uri(&printer_uri(name)));
            c.call("/", req)?;
            kept.into_iter().filter(|j| j.owner == c.user).collect()
        }
        r => return r,
    };
    for j in purge {
        c.call("/jobs/", job_request(j.id, JobOp::Remove))?;
    }
    Ok(())
}

pub(super) fn cancel_calls(jobs: &[Job]) -> Vec<Call> {
    jobs.iter()
        .map(|j| Call::JobCancelPurge(j.id, j.state.done()))
        .collect()
}

/// Make `name` the default: the server's, and the user's own when an
/// lpoptions names one (it would hide the server's otherwise). An
/// account that may not set the server's and has no cups-pk-helper sets
/// its own.
pub fn set_default(c: &Client, name: &str) -> Result<()> {
    let text = lpoptions_text();
    let server = c.admin(printer_op(c, CUPS_SET_DEFAULT, name), || {
        vec![Call::PrinterSetDefault(name.into())]
    });
    let has_own = own_default().is_some();
    if has_own || server == Err(Error::NeedsHelper) {
        write_lpoptions(&with_user_default(&text, name))?;
        return Ok(());
    }
    server
}

pub fn delete(c: &Client, name: &str) -> Result<()> {
    c.admin(printer_op(c, CUPS_DELETE_PRINTER, name), || {
        vec![Call::PrinterDelete(name.into())]
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobOp {
    Pause,
    Resume,
    Restart,
    Cancel,
    /// Cancel-Job for a finished job: purges it with its files, the only
    /// way cupsd lets it leave the history.
    Remove,
}

pub fn job_op(c: &Client, id: i32, op: JobOp) -> Result<()> {
    c.admin(c.call("/jobs/", job_request(id, op)).map(drop), || {
        vec![job_call(id, op)]
    })
}

fn job_request(id: i32, op: JobOp) -> Request {
    let code = match op {
        JobOp::Pause => HOLD_JOB,
        JobOp::Resume => RELEASE_JOB,
        JobOp::Restart => RESTART_JOB,
        JobOp::Cancel | JobOp::Remove => CANCEL_JOB,
    };
    let mut req = Request::new(code).op_attr("job-uri", Value::uri(&job_uri(id)));
    if op == JobOp::Remove {
        req = req.op_attr("purge-job", Value::Bool(true));
    }
    req
}

/// A job operation as cups-pk-helper takes it: another user's job needs
/// a password there, the account's own does not.
pub(super) fn job_call(id: i32, op: JobOp) -> Call {
    match op {
        JobOp::Pause => Call::JobSetHoldUntil(id, "indefinite".into()),
        JobOp::Resume => Call::JobSetHoldUntil(id, "no-hold".into()),
        JobOp::Restart => Call::JobRestart(id),
        JobOp::Cancel => Call::JobCancelPurge(id, false),
        JobOp::Remove => Call::JobCancelPurge(id, true),
    }
}

/// Print a test page on `name`: the PDF cups-filters ships when it is
/// there, which every queue takes, else cupsd's banner page, which needs
/// a banner filter. Printing is not an admin operation: this goes out as
/// the account and never through cups-pk-helper, which cannot print.
pub fn test_page(c: &Client, name: &str) -> Result<i32> {
    let (path, format) = TEST_PAGES
        .iter()
        .find(|(p, _)| std::path::Path::new(p).exists())
        .copied()
        .unwrap_or(TEST_PAGES[TEST_PAGES.len() - 1]);
    let data = std::fs::read(path)
        .map_err(|e| Error::Refused(format!("The test page could not be read ({path}: {e}).")))?;
    let req = Request::new(PRINT_JOB)
        .op_attr("printer-uri", Value::uri(&printer_uri(name)))
        .op_attr("job-name", Value::name("Test Page"))
        .op_attr("document-format", Value::mime(format))
        .with_data(data);
    let r = c.call(&format!("/printers/{}", escape(name)), req)?;
    Ok(r.first(JOB).and_then(|g| g.int("job-id")).unwrap_or(0))
}
