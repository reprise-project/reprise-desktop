//! Computer's removable drives through UDisks2: opening an unmounted volume
//! mounts it first (a locked one asks for its passphrase and unlocks it
//! before that), and Eject safely removes the row's drive. The calls can
//! wait on a polkit prompt, so they run as jobs and finish at a wake.

use crate::Explorer;
use crate::dialogs::Dialog;
use crate::place::{self, Place};
use repriseapp::unlock::{self, Outcome, Unlock};
use repriseapp::{Ctx, Event, Reply};
use reprisefs::udisks::{Error, ErrorKind, Snapshot, Volume};

impl Explorer {
    /// Take UDisks2's snapshots as it changes, so Computer gains and loses a
    /// drive that is plugged in or pulled out without a mount.
    pub(crate) fn follow_volumes(&mut self, wake: repriseapp::Waker) {
        let (tx, rx) = std::sync::mpsc::channel();
        reprisefs::udisks::follow(move |snap| {
            if tx.send(snap).is_ok() {
                wake.wake();
            }
        });
        self.volume_changes = Some(rx);
    }

    /// The latest snapshot `follow_volumes` posted, if any arrived.
    pub(crate) fn poll_volumes(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let snap = self.volume_changes.as_ref()?.try_iter().last()?;
        Some(self.volumes_changed(snap, ctx))
    }

    /// Computer re-lists when `snap` differs from what it shows, the Recycle Bin
    /// when a volume mounted or unmounted (for the drive's trash); elsewhere the
    /// drives are re-read if the mounts moved.
    pub(crate) fn volumes_changed(&mut self, snap: Snapshot, ctx: &mut Ctx) -> Reply {
        if snap == self.volumes {
            return Reply::none();
        }
        let mounts = |s: &Snapshot| -> Vec<std::path::PathBuf> {
            let mut m: Vec<_> = s.volumes.iter().flat_map(|v| v.mounts.clone()).collect();
            m.sort();
            m
        };
        let moved = mounts(&snap) != mounts(&self.volumes);
        self.volumes = snap;
        if self.place == Place::Computer || (moved && self.place == Place::RecycleBin) {
            return self.refresh(ctx);
        }
        self.sync_drives(ctx);
        Reply::none()
    }

    pub(crate) fn selected_unmounted(&self) -> Option<Volume> {
        if self.place != Place::Computer {
            return None;
        }
        let it = self.items.get(self.sv.selection.single()?)?;
        place::unmounted_volume(&self.volumes, it).cloned()
    }

    pub(crate) fn ejectable(&self, i: usize) -> bool {
        self.place == Place::Computer
            && self
                .items
                .get(i)
                .is_some_and(|it| place::eject_target(&self.volumes, it).is_some())
    }

    /// Mount `v`, then go into where it landed; a locked `v` raises Unlock Drive instead. A refusal raises a box titled after the volume; a dismissed prompt raises nothing.
    pub(crate) fn mount_and_open(&mut self, v: Volume, ctx: &mut Ctx) -> Reply {
        let Some(udisks) = self.udisks.clone() else {
            return Reply::none();
        };
        let name = v.name(self.volumes.disk_of(&v));
        if v.locked() {
            self.dialog = Some(Dialog::Unlock(Box::new(Unlock::open(v.path, &name, ctx))));
            return Reply::redraw_all();
        }
        self.spawn_job(ctx, move || {
            let mounted = udisks.mount(&v.path);
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| match mounted {
                Ok(at) => ex.go(Place::Folder(at), ctx),
                Err(e) => ex.udisks_failed(&format!("Could not open {name}"), &e, ctx),
            })
        });
        Reply::redraw()
    }

    /// Unlock Drive's window. OK unlocks the container on a job, then mounts
    /// its cleartext volume and goes into it; a refused passphrase keeps the
    /// box up to try again.
    pub(crate) fn unlock_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(Dialog::Unlock(u)) = self.dialog.as_mut() else {
            return Reply::none();
        };
        let clip = repriseapp::keys::form_clipboard(u.form.focused_edit(), &ev);
        let reply = match u.input(ev, ctx) {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Cancel => {
                self.dialog = None;
                self.show_waiting(ctx)
            }
            Outcome::Submit(pass) => {
                let volume = u.volume.clone();
                let Some(udisks) = self.udisks.clone() else {
                    return Reply::none();
                };
                let name = self.volumes.volume(&volume).map_or_else(
                    || "Removable Disk".into(),
                    |v| v.name(self.volumes.disk_of(v)),
                );
                self.spawn_job(ctx, move || {
                    let opened = udisks
                        .unlock(&volume, &pass)
                        .map(|clear| udisks.mount(&clear));
                    drop(pass);
                    Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                        ex.unlocked(&volume, &name, opened, ctx)
                    })
                });
                Reply::redraw()
            }
        };
        reply.with_clipboard(clip)
    }

    fn unlocked(
        &mut self,
        volume: &str,
        name: &str,
        opened: Result<Result<std::path::PathBuf, Error>, Error>,
        ctx: &mut Ctx,
    ) -> Reply {
        let open = matches!(&self.dialog, Some(Dialog::Unlock(u)) if u.volume == volume);
        match opened {
            Err(e) if e.kind != ErrorKind::Cancelled => {
                log::info!("unlocking {volume}: {e}");
                if let Some(Dialog::Unlock(u)) = self.dialog.as_mut().filter(|_| open) {
                    u.fail(unlock::failure(&e.message));
                }
                Reply::redraw_all()
            }
            done => {
                if open {
                    self.dialog = None;
                }
                let reply = match done {
                    Ok(Ok(at)) => self.go(Place::Folder(at), ctx),
                    Ok(Err(e)) => self.udisks_failed(&format!("Could not open {name}"), &e, ctx),
                    Err(_) => Reply::none(),
                };
                Reply {
                    redraw_all: true,
                    ..reply
                }
            }
        }
    }

    /// Eject: safely remove the drive under the single selected row, then re-read the drives.
    pub(crate) fn eject_selected(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(udisks) = self.udisks.clone() else {
            return Reply::none();
        };
        let Some(it) = self.sv.selection.single().and_then(|i| self.items.get(i)) else {
            return Reply::none();
        };
        let Some((v, disk)) = place::eject_target(&self.volumes, it) else {
            return Reply::none();
        };
        let title = format!("Problem Ejecting {}", disk.name());
        let volume = v.name(Some(disk));
        let disk = disk.path.clone();
        self.spawn_job(ctx, move || {
            // Read again on the job: what is mounted or unlocked now is what has to come off.
            let removed = udisks.safely_remove(&udisks.snapshot(), &disk);
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                let reply = ex.rescan(ctx);
                match removed {
                    Ok(()) => reply,
                    Err(e) if e.kind == ErrorKind::Busy => {
                        let text = format!(
                            "'{volume}' is in use. Close any program or window using it, \
                             and then try again."
                        );
                        ex.message_box(&title, text, ctx)
                    }
                    Err(e) => ex.udisks_failed(&title, &e, ctx),
                }
            })
        });
        Reply::redraw()
    }

    fn rescan(&mut self, ctx: &mut Ctx) -> Reply {
        if self.place == Place::Computer {
            return self.refresh(ctx);
        }
        self.mounts_seen.clear();
        self.sync_drives(ctx);
        Reply::redraw()
    }

    /// A box titled `title` over a failed UDisks2 call; nothing for a dismissed prompt.
    fn udisks_failed(&mut self, title: &str, e: &Error, ctx: &mut Ctx) -> Reply {
        if e.kind == ErrorKind::Cancelled {
            return Reply::redraw();
        }
        self.message_box(title, e.message.clone(), ctx)
    }
}
