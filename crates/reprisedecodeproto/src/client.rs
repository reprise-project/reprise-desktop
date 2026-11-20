//! The program's side of one decode: reprise-decode started confined by its caller's rules
//! ([`helper::run`]), the request and file written, the reply or the way it ended read as a
//! [`Picture`] or a [`NoPicture`]. An abort or a kill reads as [`NoPicture::TooLarge`]; a crash, or
//! an abort after a panic, as [`NoPicture::Broken`].

use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::time::Duration;

use repriseconfine::helper::{self, signal_name};

use crate::{BadReply, Limits, Picture, Reply, Request};

/// How long past its decode's deadline reprise-decode may take to start, confine itself and
/// reply before it is killed.
pub const SLACK: Duration = Duration::from_secs(5);

/// Why reprise-decode gave no picture, and what to log.
#[derive(Debug, PartialEq, Eq)]
pub enum NoPicture {
    /// The picture, or the memory its decode asked for, passes the limits.
    TooLarge(String),
    /// The file does not decode, or its decode ran out of time.
    Unreadable(String),
    /// reprise-decode could not be started or misbehaved: worth a warning.
    Broken(String),
}

impl NoPicture {
    pub fn why(&self) -> &str {
        match self {
            NoPicture::TooLarge(why) | NoPicture::Unreadable(why) | NoPicture::Broken(why) => why,
        }
    }
}

/// `request`'s `len` bytes of `file` decoded by the reprise-decode at `program` under `limits`,
/// killed `kill_after` from its start (the deadline and [`SLACK`] outside tests) or once `stop`
/// returns true.
pub fn decode(
    program: &Path,
    request: Request,
    file: impl Read,
    limits: Limits,
    kill_after: Duration,
    stop: &(dyn Fn() -> bool + Sync),
) -> Result<Picture, NoPicture> {
    let mut c = helper::command(program);
    c.args(limits.to_args());
    let cap = limits.reply_cap();
    let head = request.to_bytes();
    let input = head.as_slice().chain(file.take(request.len));
    let ran = helper::run_until(c, input, cap, kill_after, stop)
        .map_err(|e| NoPicture::Broken(format!("cannot start {}: {e}", program.display())))?;
    let ok = ran.status.as_ref().is_ok_and(|s| s.success());
    let said = ran.said.trim();
    let broken = |why: &str| NoPicture::Broken(format!("{why}; reprise-decode said: {said}"));
    match Reply::parse(ran.reply, limits) {
        _ if ran.stopped => Err(NoPicture::Unreadable(
            "reprise-decode was stopped: its picture is no longer wanted".into(),
        )),
        _ if ran.late => Err(NoPicture::Unreadable(
            "reprise-decode ran past its deadline and was killed".into(),
        )),
        _ if ran.past_cap => Err(broken(&format!(
            "reprise-decode replied past {} bytes",
            cap.max
        ))),
        Ok(Reply::Picture(p)) if ok => Ok(p),
        Ok(Reply::TooLarge) => Err(NoPicture::TooLarge(format!(
            "past {} bytes of pixels",
            limits.pixels
        ))),
        Ok(Reply::Unreadable(why)) => Err(NoPicture::Unreadable(why)),
        Ok(Reply::Failed(why)) => Err(broken(&why)),
        Err(BadReply::Tag(tag)) => Err(broken(&format!("reprise-decode replied {tag}"))),
        Err(BadReply::Size) => Err(broken("reprise-decode replied a picture of the wrong size")),
        _ => match ran.status {
            Ok(s) => match s.signal() {
                Some(libc::SIGSYS) => Err(broken(
                    "reprise-decode was killed by SIGSYS: a system call outside its seccomp filter",
                )),
                Some(signal @ (libc::SIGSEGV | libc::SIGBUS)) => Err(broken(&format!(
                    "the decoder crashed ({})",
                    signal_name(signal)
                ))),
                // rav1d's API is extern "C", so a panic inside it aborts, after the panic message.
                Some(libc::SIGABRT) if said.contains("panicked") => {
                    Err(broken("the decoder panicked"))
                }
                // The soft CPU limit, set at the deadline.
                Some(libc::SIGXCPU) => Err(NoPicture::Unreadable(
                    "reprise-decode ran past its CPU time and stopped".into(),
                )),
                // Any other abort is how a decoder meets an allocation it was refused, a SIGKILL
                // the OOM killer.
                Some(signal) => Err(NoPicture::TooLarge(format!(
                    "reprise-decode was killed by {}",
                    signal_name(signal)
                ))),
                None => Err(broken(&format!(
                    "reprise-decode stopped ({s}) without a reply"
                ))),
            },
            Err(e) => Err(broken(&format!("reprise-decode was lost: {e}"))),
        },
    }
}
