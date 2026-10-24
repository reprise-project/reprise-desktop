//! Background jobs and second-start requests, answered when they wake the loop.

use crate::registry::Page;
use crate::{
    PAGE_ACTION, SettingsApp, backgroundpage, locations::Located, pages, take, wallpapers,
};
use reprisebus::instance;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};

use repriseapp::{App, Reply, Waker};

impl SettingsApp {
    /// Run `job` on a thread; `Event::Wake` fires when its result is in or the thread ended without one.
    pub(crate) fn spawn<T: Send + 'static>(
        &self,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Receiver<T> {
        /// Wakes the loop on drop, so a panicking job wakes it too.
        struct WakeOnDrop(Option<Waker>);
        impl Drop for WakeOnDrop {
            fn drop(&mut self) {
                if let Some(w) = &self.0 {
                    w.wake();
                }
            }
        }
        let (tx, rx) = channel();
        let wake = WakeOnDrop(self.waker.clone());
        std::thread::spawn(move || {
            // Declared first, dropped last: the result is sent, or the
            // sender gone, before the loop looks.
            let _wake = wake;
            let tx = tx;
            let _ = tx.send(job());
        });
        rx
    }

    /// A thread is done: fill the list from the scan, keep a slideshow folder's listing, or put the decode on the monitor; a later start opens its page unless a dialog is up.
    pub(crate) fn wake(&mut self) -> Reply {
        let mut changed = false;
        let mut activate = None;
        if self.config_touched.swap(false, Ordering::AcqRel) {
            changed |= self.config_changed();
        }
        if self.slides_touched.load(Ordering::Acquire) {
            self.want_preview();
        }
        while let Some(req) = self.requests.as_ref().and_then(instance::Requests::next) {
            changed |= self.request(&req);
            activate = Some((repriseapp::MAIN, req.handoff));
        }
        if let Some(rx) = &self.scan {
            let found = match take(rx) {
                Ok(None) => None,
                Ok(Some(found)) => Some(found),
                Err(()) => {
                    log::warn!("the picture scan ended without a result");
                    Some(Vec::new())
                }
            };
            if let Some(found) = found {
                self.scan = None;
                self.wallpapers = wallpapers::add_missing(found, &self.wallpapers);
                self.fill_wallpapers();
                changed = true;
            }
        }
        if let Some((folder, rx)) = &self.listing {
            let found = match take(rx) {
                Ok(None) => None,
                Ok(Some(found)) => Some(found),
                Err(()) => Some(Vec::new()),
            };
            if let Some(found) = found {
                self.listed = Some((folder.clone(), found));
                self.listing = None;
                self.want_preview();
                self.show_monitor();
                changed = true;
            }
        }
        if let Some(rx) = &self.thumbing {
            let drawn = match take(rx) {
                Ok(None) => None,
                Ok(Some(t)) => Some(t),
                Err(()) => Some(Vec::new()),
            };
            if let Some(t) = drawn {
                self.thumbing = None;
                self.thumbs = t;
                if !backgroundpage::slideshow(&self.want) {
                    backgroundpage::set_thumbs(
                        &mut self.sheet.pages[Page::Background.index()],
                        &self.thumbs,
                    );
                }
                changed = true;
            }
        }
        if let Some(rx) = &self.locating {
            let found = match take(rx) {
                Ok(found) => found,
                Err(()) => {
                    log::warn!("the location list read ended without a result");
                    Some(Located::default())
                }
            };
            if let Some(found) = found {
                self.locating = None;
                self.show_locations(found);
                changed = true;
            }
        }
        let cleared = match &self.clearing {
            Some(rx) => match take(rx) {
                Ok(c) => c.map(pages::Clearing::Done),
                Err(()) => {
                    log::warn!("the thumbnail cache clear ended without a result");
                    Some(pages::Clearing::Failed)
                }
            },
            None => None,
        };
        if let Some(ended) = cleared {
            self.clearing = None;
            pages::show_clearing(&mut self.sheet.pages[Page::Advanced.index()], ended);
            changed = true;
        }
        let done = match &self.decoding {
            Some((path, rx)) => match take(rx) {
                Ok(None) => None,
                Ok(Some(d)) => Some((path.clone(), d)),
                Err(()) => Some((path.clone(), None)),
            },
            None => None,
        };
        if let Some((path, d)) = done {
            self.decoding = None;
            self.picture = Some(match d {
                Some(d) => backgroundpage::Picture {
                    path,
                    canvas: Some(d.canvas),
                    source_w: d.width,
                },
                None => {
                    log::warn!("{path}: not a picture this can decode");
                    backgroundpage::Picture {
                        path,
                        canvas: None,
                        source_w: 0,
                    }
                }
            });
            self.want_preview();
            self.show_monitor();
            changed = true;
        }
        let mut reply = if changed {
            Reply::redraw()
        } else {
            Reply::none()
        };
        reply.activate = activate;
        reply
    }

    pub(crate) fn request(&mut self, req: &instance::Request) -> bool {
        match &req.action {
            Some((action, params)) if action == PAGE_ACTION && self.children().is_empty() => {
                let (name, focus) = match params.as_slice() {
                    [name] => (name, None),
                    [name, focus] => (name, Some(focus.as_str())),
                    _ => return false,
                };
                Page::named(name).is_some_and(|p| self.open_page(p, focus))
            }
            _ => false,
        }
    }

    /// Open `page`, with the keyboard on control `focus` when one is named. True when anything moved.
    pub(crate) fn open_page(&mut self, page: Page, focus: Option<&str>) -> bool {
        if self.sheet.tabs.open == page.index() && focus.is_none() {
            return false;
        }
        self.blur_side_lists();
        self.sheet.tabs.open = page.index();
        self.sheet.reveal_open();
        match focus {
            Some(id) => self.sheet.focus_control(id),
            None => self.tab_opened(),
        }
        true
    }
}
