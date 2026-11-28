//! The password box of an encrypted zip, raised when the window opens it as a folder and when
//! Extract reaches it. A password that opens the zip on view is kept for the window
//! ([`reprisefs::archive::Listings::keep`]); Extract's go to its job and no further.

use crate::Explorer;
use crate::dialogs::Dialog;
use crate::place::Place;
use repriseapp::unlock::{Outcome, Unlock, Words};
use repriseapp::{Ctx, Event, Reply};
use reprisefs::archive::{Key, Opened, Unkeyed};
use reprisefs::extract::Ask;
use reprisefs::progress::Asked;
use reprisekit::tr;
use std::path::{Path, PathBuf};

/// The theme icons the box wears: a compressed folder's.
const ICONS: &[&str] = &["package-x-generic"];

/// What an Unlock box's passphrase opens.
pub(crate) enum Unlocks {
    Drive,
    /// The encrypted zip on view, to list it.
    Open(PathBuf),
    /// The encrypted zip an extraction waits on; dropping the question ends the job.
    Extract(Asked),
}

impl Unlocks {
    pub(crate) fn holds_a_job(&self) -> bool {
        matches!(self, Unlocks::Extract(_))
    }
}

fn words(zip: &Path, extract: bool) -> Words {
    let name = zip.file_name().unwrap_or_default().to_string_lossy();
    let message = match extract {
        true => tr!("zipkey-extract", name = &*name),
        false => tr!("zipkey-open", name = &*name),
    };
    Words {
        title: tr!("zipkey-title"),
        message,
        label: tr!("zipkey-label"),
    }
}

fn wrong() -> String {
    tr!("zipkey-wrong")
}

/// The status line of a box asking again, `ask` saying why.
fn asked_again(ask: Ask) -> Option<String> {
    match ask {
        Ask::First => None,
        Ask::Wrong => Some(wrong()),
        Ask::Another => Some(tr!("zipkey-another")),
    }
}

/// What the box says of typed text that is no password bsdtar reads as typed.
fn unkeyed(why: Unkeyed) -> String {
    match why {
        Unkeyed::Empty | Unkeyed::Breaks => tr!("zipkey-breaks"),
        Unkeyed::TooLong => tr!("zipkey-too-long", most = Key::MOST_BYTES),
    }
}

impl Explorer {
    /// Asks for the password of the encrypted zip `zip` on view, unless its box is up already.
    /// Before the window first holds the keyboard the box waits ([`Explorer::keyboard_entered`]),
    /// so a password typed meanwhile cannot land in another program's window.
    pub(crate) fn ask_zip_key(&mut self, zip: PathBuf, ctx: &mut Ctx) -> Reply {
        if !self.entered {
            self.key_wait = Some(zip);
            return Reply::none();
        }
        if matches!(self.dialogs.up(), Some(Dialog::Unlock(_, Unlocks::Open(z))) if *z == zip) {
            return Reply::none();
        }
        let id = zip.to_string_lossy().into_owned();
        let u = Unlock::open_with(id, words(&zip, false), ICONS, ctx);
        self.raise(Dialog::Unlock(Box::new(u), Unlocks::Open(zip)))
    }

    /// The folder window took the keyboard: the first time, a password box waiting for it comes
    /// up while the zip is still on view.
    pub(crate) fn keyboard_entered(&mut self, ctx: &mut Ctx) -> Reply {
        if std::mem::replace(&mut self.entered, true) {
            return Reply::none();
        }
        match self.key_wait.take() {
            Some(zip) if matches!(&self.place, Place::Archive(f, _) if *f == zip) => {
                self.ask_zip_key(zip, ctx)
            }
            _ => Reply::none(),
        }
    }

    /// Asks for a password of `zip`, which the extraction holding `asked` waits on, `ask` saying
    /// why.
    pub(crate) fn ask_extract_key(
        &mut self,
        zip: &Path,
        ask: Ask,
        asked: Asked,
        ctx: &mut Ctx,
    ) -> Reply {
        if self.closing {
            return Reply::none();
        }
        let id = zip.to_string_lossy().into_owned();
        let mut u = Unlock::open_with(id, words(zip, true), ICONS, ctx);
        if let Some(why) = asked_again(ask) {
            u.fail(&why);
        }
        self.raise(Dialog::Unlock(Box::new(u), Unlocks::Extract(asked)))
    }

    /// The zip password box's window. Opening checks the password on a job, the box staying
    /// up to try again when it is wrong; Extract's job checks its own.
    pub(crate) fn zip_key_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(Dialog::Unlock(u, _)) = self.dialogs.up_mut() else {
            return Reply::none();
        };
        let clip = repriseapp::keys::form_clipboard(u.form.focused_edit(), &ev);
        let reply = match u.input(ev, ctx) {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Cancel => self.zip_key_cancelled(ctx),
            Outcome::Submit(typed) => match Key::new(typed) {
                Ok(key) => self.zip_key_given(key, ctx),
                Err(why) => {
                    u.fail(&unkeyed(why));
                    Reply::redraw()
                }
            },
        };
        reply.with_clipboard(clip)
    }

    fn zip_key_given(&mut self, key: Key, ctx: &mut Ctx) -> Reply {
        match self.dialogs.up() {
            Some(Dialog::Unlock(_, Unlocks::Open(zip))) => {
                let (zip, archives) = (zip.clone(), self.archives.clone());
                self.spawn_job(ctx, move || {
                    let opened = archives.check_key(&zip, key);
                    Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                        ex.zip_key_tried(&zip, opened, ctx)
                    })
                });
                Reply::redraw()
            }
            Some(Dialog::Unlock(_, Unlocks::Extract(_))) => self.answer_extract(Some(key)),
            _ => Reply::none(),
        }
    }

    /// Cancel: opening stops a check under way, whose answer is then let go, and says the zip is
    /// password-protected.
    fn zip_key_cancelled(&mut self, ctx: &mut Ctx) -> Reply {
        match self.dialogs.up() {
            Some(Dialog::Unlock(_, Unlocks::Open(zip))) => {
                let zip = zip.clone();
                self.take_up();
                self.archives.cancel();
                self.archive_open_failed(&zip, reprisefs::archive::refusal(None), ctx)
            }
            Some(Dialog::Unlock(_, Unlocks::Extract(_))) => self.answer_extract(None),
            _ => Reply::none(),
        }
    }

    /// Hands the password typed, or None for Cancel, to the extraction the box holds.
    fn answer_extract(&mut self, key: Option<Key>) -> Reply {
        if let Some(Dialog::Unlock(_, Unlocks::Extract(asked))) = self.take_up() {
            match &self.extracting {
                Some(ex) if asked.is_for(ex.job.meter()) => ex.job.unlock(key, asked),
                _ => drop(asked),
            }
            self.show_progress_dialogs();
        }
        Reply::redraw_all()
    }

    /// The check of a password for `zip` came back to the box that asked: wrong keeps it up to
    /// try again, and one that opens it is kept for the window and lists the zip again where the
    /// window still shows it. With the box gone the answer is let go.
    fn zip_key_tried(
        &mut self,
        zip: &Path,
        opened: Result<Option<Opened>, String>,
        ctx: &mut Ctx,
    ) -> Reply {
        let up = matches!(self.dialogs.up(), Some(Dialog::Unlock(_, Unlocks::Open(z))) if z == zip);
        if !up {
            return Reply::none();
        }
        if let (Ok(None), Some(Dialog::Unlock(u, _))) = (&opened, self.dialogs.up_mut()) {
            u.fail(&wrong());
            return Reply::redraw_all();
        }
        self.take_up();
        let reply = match opened {
            Ok(Some(opened)) => {
                self.archives.keep(opened);
                match matches!(&self.place, Place::Archive(f, _) if f == zip) {
                    true => self.refresh(ctx),
                    false => Reply::none(),
                }
            }
            Ok(None) => Reply::none(),
            Err(why) => self.archive_open_failed(zip, why, ctx),
        };
        Reply {
            redraw_all: true,
            ..reply
        }
    }

    /// A box saying the compressed folder `zip` did not open, and `why`.
    pub(crate) fn archive_open_failed(&mut self, zip: &Path, why: String, ctx: &mut Ctx) -> Reply {
        let name = zip.file_name().unwrap_or_default().to_string_lossy();
        let text = tr!("archive-open-failed", name = &*name, why = why);
        self.message_box(&tr!("archive-open-failed-title"), text, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The box fits its text, a long name wrapped and each status line shown, under every
    /// interface and text size.
    #[test]
    fn the_box_fits_its_text() {
        let over = reprisekit::interfaces::form_overflows(|_, forms, text| {
            let zip = Path::new("/d/Quarterly figures for the board, final.zip");
            let statuses = [
                None,
                Some(wrong()),
                Some(tr!("zipkey-another")),
                Some(unkeyed(Unkeyed::TooLong)),
                Some(unkeyed(Unkeyed::Breaks)),
            ];
            let mut forms_seen = Vec::new();
            for extract in [false, true] {
                for (i, status) in statuses.iter().enumerate() {
                    let words = words(zip, extract);
                    let lines = repriseapp::unlock::wrapped(&words.message, text, forms);
                    let mut u = Unlock::new(String::new(), words, &lines, None);
                    if let Some(why) = status {
                        u.fail(why);
                    }
                    forms_seen.push((format!("extract={extract} status {i}"), u.form.clone()));
                }
            }
            forms_seen
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }
}
