//! Computer's removable drives through UDisks2: opening an unmounted volume
//! mounts it first, and Eject safely removes the row's drive. Both calls can
//! wait on a polkit prompt, so they run as jobs and finish at a wake.

use crate::Explorer;
use crate::place::{self, Place};
use repriseapp::{Ctx, Reply};
use reprisefs::udisks::{Error, ErrorKind, Volume};

impl Explorer {
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

    /// Mount `v`, then go into where it landed. A refusal raises a box titled after the volume; a dismissed prompt raises nothing.
    pub(crate) fn mount_and_open(&mut self, v: Volume, ctx: &mut Ctx) -> Reply {
        let Some(udisks) = self.udisks.clone() else {
            return Reply::none();
        };
        let name = v.name(self.volumes.disk_of(&v));
        self.spawn_job(ctx, move || {
            let mounted = udisks.mount(&v.path);
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| match mounted {
                Ok(at) => ex.go(Place::Folder(at), ctx),
                Err(e) => ex.udisks_failed(&format!("Could not open {name}"), &e, ctx),
            })
        });
        Reply::redraw()
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
