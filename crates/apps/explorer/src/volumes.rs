//! Computer's removable drives through UDisks2: opening an unmounted volume
//! mounts it first (a locked one asks for its passphrase and unlocks it
//! before that), and Eject safely removes the row's drive. A disk image
//! opens by mounting it and shows in Computer until ejected. Phones and
//! cameras mount and unmount through gvfs. The calls can wait on a polkit
//! prompt or the device, so they run as jobs and finish at a wake.

use crate::Explorer;
use crate::dialogs::Dialog;
use crate::place::{self, Place};
use repriseapp::unlock::{self, Outcome, Unlock};
use repriseapp::{Ctx, Event, Reply};
use reprisefs::listing::Item;
use reprisefs::netshare::Unmount;
use reprisefs::portable::{self, Device};
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
        if self.mark_volumes() {
            self.refill_tree_node(&Place::Computer, ctx);
        }
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
            && self.items.get(i).is_some_and(|it| {
                place::eject_target(&self.volumes, it).is_some()
                    || self.row_device(it).is_some_and(|d| d.mount.is_some())
            })
    }

    /// Take the volume monitors' phones and cameras as they come and go.
    pub(crate) fn follow_portable(&mut self, wake: repriseapp::Waker) {
        let (tx, rx) = std::sync::mpsc::channel();
        reprisefs::portable::follow(move |devices| {
            if tx.send(devices).is_ok() {
                wake.wake();
            }
        });
        self.portable_changes = Some(rx);
    }

    /// The latest list `follow_portable` posted, if any arrived: Computer re-lists when it
    /// differs, and a mounted device's folder takes its name.
    pub(crate) fn poll_portable(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let devices = self.portable_changes.as_ref()?.try_iter().last()?;
        if devices == self.portable {
            return Some(Reply::none());
        }
        self.portable = devices;
        let mut shares = self.shares.clone();
        portable::dress(&mut shares.shares, &self.portable);
        if shares != self.shares {
            return Some(self.shares_changed(shares, ctx));
        }
        Some(match self.place == Place::Computer {
            true => self.refresh(ctx),
            false => Reply::none(),
        })
    }

    /// The phone or camera behind Computer's row `it`, with its mount.
    fn row_device(&self, it: &Item) -> Option<Device> {
        let joined = portable::with_mounts(&self.portable, &self.shares.shares);
        place::row_device(&joined, it).cloned()
    }

    pub(crate) fn selected_unmounted_device(&self) -> Option<Device> {
        if self.place != Place::Computer {
            return None;
        }
        let it = self.items.get(self.sv.selection.single()?)?;
        self.row_device(it).filter(|d| d.mount.is_none())
    }

    /// Mount `d` through gvfs on a job, then go into its folder; a refusal raises a box.
    pub(crate) fn mount_device_and_open(&mut self, d: Device, ctx: &mut Ctx) -> Reply {
        self.spawn_job(ctx, move || {
            let mounted = portable::mount(&d.uri);
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| match mounted {
                Ok(at) => ex.go(Place::Folder(at), ctx),
                Err(e) => ex.message_box(&format!("Could not open {}", d.name), e, ctx),
            })
        });
        Reply::redraw()
    }

    /// Eject on a mounted phone or camera: gvfs unmounts it, and Computer shows it unmounted when
    /// gvfs says so.
    fn eject_device(&mut self, d: Device, ctx: &mut Ctx) -> Reply {
        let Some(at) = d.mount else {
            return Reply::none();
        };
        self.spawn_job(ctx, move || {
            let done = reprisefs::netshare::unmount(&at, false);
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                let title = format!("Problem Ejecting {}", d.name);
                match done {
                    Ok(()) => Reply::none(),
                    Err(Unmount::Busy) => {
                        let text = format!(
                            "'{}' is in use. Close any program or window using it, and then try \
                             again.",
                            d.name
                        );
                        ex.message_box(&title, text, ctx)
                    }
                    Err(Unmount::Error(e)) => ex.message_box(&title, e, ctx),
                }
            })
        });
        Reply::redraw()
    }

    /// Mount `v`, then go into where it landed; a locked `v` raises Unlock Drive instead. A refusal raises a box titled after the volume; a dismissed prompt raises nothing.
    pub(crate) fn mount_and_open(&mut self, v: Volume, ctx: &mut Ctx) -> Reply {
        let Some(udisks) = self.udisks.clone() else {
            return Reply::none();
        };
        let name = v.name(self.volumes.disk_of(&v));
        if v.locked() {
            return self.raise(Dialog::Unlock(Box::new(Unlock::open(v.path, &name, ctx))));
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
        let Some(Dialog::Unlock(u)) = self.dialogs.up_mut() else {
            return Reply::none();
        };
        let clip = repriseapp::keys::form_clipboard(u.form.focused_edit(), &ev);
        let reply = match u.input(ev, ctx) {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Cancel => {
                self.take_up();
                Reply::redraw_all()
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
        let open = matches!(self.dialogs.up(), Some(Dialog::Unlock(u)) if u.volume == volume);
        match opened {
            Err(e) if e.kind != ErrorKind::Cancelled => {
                log::info!("unlocking {volume}: {e}");
                if let Some(Dialog::Unlock(u)) = self.dialogs.up_mut().filter(|_| open) {
                    u.fail(unlock::failure(&e.message));
                }
                Reply::redraw_all()
            }
            done => {
                if open {
                    self.take_up();
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

    /// Marks the mounted drives the last UDisks2 snapshot calls removable and adds its disk
    /// images: `/proc/mounts` alone takes an unlocked stick's cleartext volume for a fixed disk,
    /// and leaves out every loop device. Returns whether the drives' mounts changed.
    pub(crate) fn mark_volumes(&mut self) -> bool {
        for d in &mut self.drives {
            if self.volumes.removable_device(&d.device) {
                d.removable = true;
            }
        }
        let images = self.volumes.image_drives();
        let was = std::mem::take(&mut self.drive_mounts);
        self.drives.retain(|d| !d.device.starts_with("/dev/loop"));
        self.drives.extend(images);
        self.drive_mounts = place::drive_mounts(&self.drives, &self.shares.shares);
        self.drive_mounts != was
    }

    /// The single selected file, when it is a disk image.
    pub(crate) fn selected_disk_image(&self) -> Option<&Item> {
        if self.place == Place::RecycleBin {
            return None;
        }
        let it = self.items.get(self.sv.selection.single()?)?;
        let image = !it.is_folder()
            && it
                .mime
                .as_deref()
                .is_some_and(reprisefs::udisks::is_disk_image);
        image.then_some(it)
    }

    /// Opening the selected disk image: asks before [`Explorer::mount_image`] mounts it.
    pub(crate) fn ask_mount_image(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(it) = self.selected_disk_image() else {
            return Reply::none();
        };
        let path = it.path.clone();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        self.raise(Dialog::Confirm(crate::dialogs::Confirm {
            title: reprisefs::udisks::MOUNT_TITLE,
            text: reprisefs::udisks::mount_text(&name),
            min_text_h: None,
            template: reprisekit::controls::dlgtemplate::Template::MessageBox,
            icon: crate::icons::IconSlot::chrome(&mut ctx.icons, crate::dialogs::QUESTION_ICON, 32),
            paths: vec![path.clone()],
            erase: false,
            empty_bin: false,
            bin: None,
            focus: None,
            too_big: None,
            undo_erase: false,
            script: None,
            mount: true,
            job: None,
        }))
    }

    /// Mount: the selected disk image set up read-only and mounted, then its folder shown. A
    /// refusal or an image with nothing to mount raises a box.
    pub(crate) fn mount_image(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(it) = self.selected_disk_image() else {
            return Reply::none();
        };
        let path = it.path.clone();
        self.mount_image_at(path, ctx)
    }

    /// [`Explorer::mount_image`] for the disk image at `path`.
    pub(crate) fn mount_image_at(&mut self, path: std::path::PathBuf, ctx: &mut Ctx) -> Reply {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let title = format!("Could not mount {name}");
        let Some(udisks) = self.udisks.clone() else {
            return self.message_box(&title, "UDisks2 is not running.".into(), ctx);
        };
        self.spawn_job(ctx, move || {
            let mounted = udisks.mount_image(&path);
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| match mounted {
                Ok(at) => ex.go(Place::Folder(at), ctx),
                Err(e) => ex.udisks_failed(&title, &e, ctx),
            })
        });
        Reply::redraw()
    }

    /// The item menu's Open on a disk image: its program, past the Mount default.
    pub(crate) fn open_in_program(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(it) = self.selected_disk_image() else {
            return self.open_selected(ctx);
        };
        let (path, exec) = (it.path.clone(), it.exec);
        let how = reprisefs::program::Confirm::on_open(repriseconf::current().confirm_scripts);
        self.open_file(&path, exec, repriseconf::current().script_open, how, ctx)
    }

    /// Eject: safely remove the drive under the single selected row, or unmount and detach the
    /// disk image, then re-read the drives; or unmount the phone or camera.
    pub(crate) fn eject_selected(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(it) = self.sv.selection.single().and_then(|i| self.items.get(i)) else {
            return Reply::none();
        };
        if let Some(d) = self.row_device(it) {
            return self.eject_device(d, ctx);
        }
        let Some(udisks) = self.udisks.clone() else {
            return Reply::none();
        };
        let Some((v, disk)) = place::eject_target(&self.volumes, it) else {
            return Reply::none();
        };
        let title = format!(
            "Problem Ejecting {}",
            disk.map_or_else(|| v.name(None), |d| d.name())
        );
        let volume = v.name(disk);
        let image = v.image.as_ref().map(|i| i.loop_path.clone());
        let disk = disk.map(|d| d.path.clone());
        self.spawn_job(ctx, move || {
            // Read again on the job: what is mounted or unlocked now is what has to come off.
            let snap = udisks.snapshot();
            let removed = match (disk, image) {
                (Some(disk), _) => udisks.safely_remove(&snap, &disk),
                (None, Some(image)) => udisks.eject_image(&snap, &image),
                (None, None) => Ok(()),
            };
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
