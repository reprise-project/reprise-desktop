//! Send To's reads, each on a listing thread since any can sit on a mount that never answers: the
//! mount points, the submenu's own folders and a stat per drive. A send to a drive waits on a
//! fresh stat of it and gives up after the action deadline.

use crate::Explorer;
use crate::acting::Deadline;
use repriseapp::listjob::Job;
use repriseapp::{Ctx, Reply};
use reprisefs::ops::DropOp;
use reprisefs::places::{self, MountedDrives};
use reprisefs::sendto::{Dirs, Drives, SendToRow};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(crate) struct SendReads {
    pub(crate) drives: Drives,
    dirs: Dirs,
    scan: Option<Out<(MountedDrives, Vec<PathBuf>)>>,
    dirs_read: Option<Out<Dirs>>,
    /// Each drive's stat still out.
    stats: Vec<(PathBuf, Out<bool>)>,
    /// Copies waiting on a fresh stat of their drive.
    pub(crate) sends: Vec<Sending>,
}

/// A read still out, and when it started.
struct Out<T> {
    job: Job<T>,
    at: Instant,
}

impl<T: Send + 'static> Out<T> {
    fn new(job: Job<T>) -> Out<T> {
        Out {
            job,
            at: Instant::now(),
        }
    }

    /// The answer, waited for until `until` if the read started under `fresh` ago: one already
    /// stuck on a mount costs a later wait nothing.
    fn take(&self, until: Option<Instant>, fresh: Duration) -> Option<T> {
        let now = Instant::now();
        let wait = match until {
            Some(until) if now.duration_since(self.at) < fresh => {
                until.saturating_duration_since(now)
            }
            _ => Duration::ZERO,
        };
        self.job.wait(wait)
    }
}

pub(crate) struct Sending {
    top: PathBuf,
    paths: Vec<PathBuf>,
    deadline: Deadline,
}

impl Default for SendReads {
    fn default() -> SendReads {
        SendReads {
            drives: Drives::new(MountedDrives::new()),
            dirs: Dirs::default(),
            scan: None,
            dirs_read: None,
            stats: Vec::new(),
            sends: Vec::new(),
        }
    }
}

impl Explorer {
    /// The Send To rows for a menu about to open: the mount points and folders read again, each
    /// drive stated again, and what answers within the listing wait shown. A drive whose stat is
    /// still out has a greyed row.
    pub(crate) fn send_rows(&mut self, ctx: &mut Ctx) -> Vec<SendToRow> {
        self.scan_send_drives(ctx);
        if self.send.dirs_read.is_none() {
            let desktop = places::user_dir("DESKTOP", "Desktop").unwrap_or_default();
            let read = self.spawn_read(ctx, move || Dirs::read(&desktop));
            self.send.dirs_read = Some(Out::new(read));
        }
        for top in self.send.drives.restat() {
            self.stat_send_drive(top, ctx);
        }
        let until = Instant::now() + self.list_wait();
        self.land_send_reads(Some(until), ctx);
        self.send.drives.send_to_rows(&self.send.dirs)
    }

    fn scan_send_drives(&mut self, ctx: &Ctx) {
        if let Some(mut scanner) = self.send.drives.start() {
            let scan = self.spawn_read(ctx, move || {
                let listed = scanner.listed().to_vec();
                (scanner, listed)
            });
            self.send.scan = Some(Out::new(scan));
        }
    }

    /// Stats drive `top`, its stat already counted as out.
    fn stat_send_drive(&mut self, top: PathBuf, ctx: &Ctx) {
        let at = top.clone();
        let job = self.spawn_read(ctx, move || places::usable_drive(&at));
        self.send.stats.push((top, Out::new(job)));
    }

    /// A send of `paths` to `dir`, one of the submenu's folders: a listed drive is stated first,
    /// and one whose stat is out sends nothing.
    pub(crate) fn send_to_folder(
        &mut self,
        paths: Vec<PathBuf>,
        dir: PathBuf,
        ctx: &mut Ctx,
    ) -> Reply {
        match self.send.drives.send_stat(&dir) {
            None => self.put_all(&paths, &dir, &|_| Some(DropOp::Copy), false, ctx),
            Some(false) => self.send_not_responding(&dir, ctx),
            Some(true) => {
                self.stat_send_drive(dir.clone(), ctx);
                let deadline = Deadline::start(self.action_deadline, ctx);
                let top = dir;
                self.send.sends.push(Sending {
                    top,
                    paths,
                    deadline,
                });
                let until = Instant::now() + self.list_wait();
                self.land_send_reads(Some(until), ctx).unwrap_or_default()
            }
        }
    }

    fn send_not_responding(&mut self, dir: &Path, ctx: &mut Ctx) -> Reply {
        log::warn!("send to: {} is not answering; nothing sent", dir.display());
        let text = reprisefs::deadline::not_responding(false, false);
        self.message_box("Send To", text, ctx)
    }

    /// Takes Send To's reads that answered since the last wake.
    pub(crate) fn take_send_reads(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        self.land_send_reads(None, ctx)
    }

    /// Lands the reads that have answered, waiting for them until `until` if given; a stat that
    /// lands sends what waited on it, and a send past its deadline gives up.
    fn land_send_reads(&mut self, until: Option<Instant>, ctx: &mut Ctx) -> Option<Reply> {
        let fresh = self.list_wait();
        if let Some((scanner, listed)) = self.send.scan.as_ref().and_then(|o| o.take(until, fresh))
        {
            self.send.scan = None;
            let (again, stat) = self.send.drives.land(scanner, listed);
            for top in stat {
                self.stat_send_drive(top, ctx);
            }
            if again {
                self.scan_send_drives(ctx);
            }
        }
        if let Some(dirs) = self
            .send
            .dirs_read
            .as_ref()
            .and_then(|o| o.take(until, fresh))
        {
            self.send.dirs = dirs;
            self.send.dirs_read = None;
        }
        let mut landed = Vec::new();
        self.send
            .stats
            .retain(|(top, out)| match out.take(until, fresh) {
                Some(usable) => {
                    landed.push((top.clone(), usable));
                    false
                }
                None => true,
            });
        let mut reply: Option<Reply> = None;
        let mut and = |r: Reply| {
            reply = Some(match reply.take() {
                Some(was) => was.and(r),
                None => r,
            })
        };
        for (top, usable) in landed {
            self.send.drives.land_stat(top.clone(), usable);
            let (go, wait) = std::mem::take(&mut self.send.sends)
                .into_iter()
                .partition(|s| s.top == top);
            self.send.sends = wait;
            for s in go.into_iter().filter(|_| usable) {
                and(self.put_all(&s.paths, &s.top, &|_| Some(DropOp::Copy), false, ctx));
            }
        }
        let now = Instant::now();
        let (late, wait) = std::mem::take(&mut self.send.sends)
            .into_iter()
            .partition(|s: &Sending| s.deadline.passed(now));
        self.send.sends = wait;
        for s in late {
            and(self.send_not_responding(&s.top, ctx));
        }
        reply
    }
}
